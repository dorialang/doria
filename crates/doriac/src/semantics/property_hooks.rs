use super::*;
use crate::ast::PropertyHookKind;
use crate::property_hooks::{declaration_facts, PropertyHookContext};

impl Checker<'_> {
    pub(super) fn property_backing_fields(
        &mut self,
    ) -> HashMap<Span, crate::property_hooks::PropertyBackingField> {
        let mut classes = self.classes.keys().cloned().collect::<Vec<_>>();
        classes.sort();
        let mut ordered = Vec::new();
        for name in classes {
            let ty = self.symbolic_class_type(&name);
            let TypeKind::Class(owner) = self.types.kind(ty).clone() else {
                continue;
            };
            let ancestors = self.specialized_ancestor_types(&owner);
            if !ancestors.iter().any(|ancestor| ancestor.name == name) {
                ordered.push((ancestors, owner));
            }
        }
        ordered.sort_by_key(|(ancestors, _)| ancestors.len());
        let mut fields = HashMap::<Span, crate::property_hooks::PropertyBackingField>::new();
        for (ancestors, owner) in ordered {
            let properties = self.classes[&owner.name].properties.clone();
            let mut properties = properties.iter().collect::<Vec<_>>();
            properties.sort_by_key(|(_, property)| property.declaration_span);
            for (name, property) in properties {
                let Some(hooks) = &property.hooks else {
                    continue;
                };
                let inherited = ancestors.iter().find_map(|ancestor| {
                    let property = self.classes.get(&ancestor.name)?.properties.get(name)?;
                    fields.get(&property.declaration_span).cloned()
                });
                if hooks.storage == Some(crate::property_hooks::PropertyHookStorage::Backed) {
                    if let Some(field) = &inherited {
                        let ancestor = ancestors
                            .iter()
                            .find(|ancestor| ancestor.name == field.declaring_class)
                            .expect("inherited backing field belongs to an ancestor");
                        let root = self.classes[&ancestor.name].properties[name].clone();
                        let root = self.specialize_property_for_class(&root, ancestor);
                        if property.ty != root.ty {
                            self.diagnostics.push(
                                Diagnostic::new(
                                    "E0729",
                                    format!(
                                        "backed override `{}::{name}` must preserve the inherited field type `{}`",
                                        owner.name,
                                        self.types.display(root.ty),
                                    ),
                                    property.declaration_span,
                                )
                                .with_title("Override Contract Does Not Match")
                                .with_related(field.declaration, "Inherited Backing Field Declared Here")
                                .with_explanation("backed overrides share one physical field; a narrower getter return cannot change that field's stored type"),
                            );
                        }
                    }
                }
                let field = inherited.or_else(|| {
                    (hooks.storage == Some(crate::property_hooks::PropertyHookStorage::Backed))
                        .then(|| crate::property_hooks::PropertyBackingField {
                            declaring_class: owner.name.clone(),
                            property_name: name.clone(),
                            declaration: property.declaration_span,
                        })
                });
                if let Some(field) = field {
                    fields.insert(property.declaration_span, field);
                }
            }
        }
        fields
    }

    pub(super) fn validate_property_hook_hierarchies(&mut self) {
        let mut classes = self.classes.keys().cloned().collect::<Vec<_>>();
        classes.sort();
        let mut ordered = Vec::new();
        for name in classes {
            let ty = self.symbolic_class_type(&name);
            let TypeKind::Class(owner) = self.types.kind(ty).clone() else {
                continue;
            };
            let ancestors = self.specialized_ancestor_types(&owner);
            if ancestors.iter().any(|ancestor| ancestor.name == name) {
                continue;
            }
            ordered.push((ancestors.len(), owner));
        }
        // An override inherits the original accessor root, independent of source order.
        ordered.sort_by_key(|(depth, _)| *depth);
        for (_, owner) in ordered {
            let class = self.classes[&owner.name].clone();
            let mut properties = class.properties.iter().collect::<Vec<_>>();
            properties.sort_by_key(|(_, property)| property.declaration_span);
            for (name, property) in properties {
                let Some(hooks) = &property.hooks else {
                    continue;
                };
                if hooks.is_open && (!class.is_open || property.access == MemberAccess::Internal) {
                    self.diagnostics.push(
                        Diagnostic::new("E0725", format!("property `{}::{name}` cannot be open", owner.name), property.declaration_span)
                            .with_title("Property Cannot Be Open")
                            .with_help("only external instance properties on open classes may be declared open"),
                    );
                }
                let inherited = class
                    .parent
                    .as_ref()
                    .and_then(|parent| self.lookup_inherited_member(parent, name));
                let Some((parent, member)) = inherited else {
                    if hooks.is_override {
                        self.diagnostics.push(
                            Diagnostic::new("E0730", format!("property `{}::{name}` uses `override`, but no inherited open property matches", owner.name), property.declaration_span)
                                .with_title("Override Has No Target"),
                        );
                    }
                    continue;
                };
                let Some(inherited) = self.classes[&parent.name].properties.get(name).cloned()
                else {
                    continue; // The common member-namespace check reports this collision.
                };
                if inherited.hooks.is_none() {
                    continue;
                }
                let inherited = self.specialize_property_for_class(&inherited, &parent);
                let required = [PropertyHookKind::Get, PropertyHookKind::Set]
                    .into_iter()
                    .filter_map(|kind| {
                        self.property_accessor_method(&inherited, &parent, kind)
                            .map(|method| (kind, method))
                    })
                    .collect::<Vec<_>>();
                if !required
                    .iter()
                    .any(|(_, method)| method.virtual_root.is_some())
                {
                    self.report_inherited_member_collision(
                        &owner.name,
                        name,
                        property.declaration_span,
                        &parent,
                        member.span,
                    );
                    continue;
                }
                if !hooks.is_override {
                    self.diagnostics.push(
                        Diagnostic::new("E0726", format!("property `{}::{name}` replaces an inherited open property and must use `override`", owner.name), property.declaration_span)
                            .with_title("Override Modifier Is Required")
                            .with_related(member.span, "The Inherited Open Property Is Declared Here")
                            .with_help("add `override` before the property declaration"),
                    );
                    continue;
                }
                let mut overrides = Vec::new();
                let mut compatible = true;
                for (kind, required) in required {
                    let actual = self.property_accessor_method(property, &owner, kind);
                    if actual.as_ref().is_some_and(|method| {
                        self.method_contract_failures(method, &required).is_empty()
                    }) {
                        let method = actual.unwrap();
                        overrides.push((
                            method.declaration,
                            required.virtual_root.unwrap_or(required.declaration),
                            required.declaration,
                        ));
                    } else {
                        compatible = false;
                        let accessor = match kind {
                            PropertyHookKind::Get => "getter",
                            PropertyHookKind::Set => "setter",
                        };
                        self.diagnostics.push(
                            Diagnostic::new("E0729", format!("property `{}::{name}` does not preserve its inherited {accessor} contract", owner.name), actual.map_or(property.declaration_span, |method| method.declaration))
                                .with_title("Override Contract Does Not Match")
                                .with_related(required.declaration, "The Inherited Accessor Contract Is Declared Here")
                                .with_help("preserve every inherited accessor's type, parameter name, ownership, receiver access, return provenance, and checked effects"),
                        );
                    }
                }
                if compatible {
                    // The property remains virtual, including any accessor first
                    // introduced by this override.
                    for accessor in [hooks.getter, hooks.setter].into_iter().flatten() {
                        self.override_roots
                            .entry(accessor.declaration)
                            .or_insert(accessor.declaration);
                    }
                    for (declaration, root, overridden) in overrides {
                        self.override_roots.insert(declaration, root);
                        self.overridden_declarations.insert(declaration, overridden);
                    }
                }
            }
        }
        // Calls are checked before the post-body contract pass establishes
        // accessor families. Publish the final roots for both reads and writes.
        for calls in self.property_accessor_calls.values_mut() {
            for call in calls.getter.iter_mut().chain(calls.setter.iter_mut()) {
                if let Some(root) = self.override_roots.get(&call.declaration) {
                    call.virtual_root = Some(*root);
                }
            }
        }
    }

    pub(super) fn check_setter_argument(
        &mut self,
        target: &Expr,
        value: &Expr,
        scopes: &ScopeStack,
        context: Option<&MethodContext>,
    ) {
        let target = match target {
            Expr::Grouped { expr, .. } => {
                return self.check_setter_argument(expr, value, scopes, context)
            }
            _ => target,
        };
        let Expr::PropertyAccess {
            object, property, ..
        } = target
        else {
            return;
        };
        let Some(method) = self.property_accessor(target, PropertyHookKind::Set, scopes, context)
        else {
            return;
        };
        if let Some(parameter) = method.params.first() {
            let receiver = self.infer_expr_type(object, scopes, context);
            self.check_bound_argument_type(
                &format!("setter `{}::{property}`", self.types.display(receiver)),
                parameter,
                value,
                0,
                scopes,
                context,
            );
        }
    }

    pub(super) fn property_getter(
        &mut self,
        expr: &Expr,
        scopes: &ScopeStack,
        context: Option<&MethodContext>,
    ) -> Option<MethodInfo> {
        self.property_accessor(expr, PropertyHookKind::Get, scopes, context)
    }

    fn property_accessor(
        &mut self,
        expr: &Expr,
        kind: PropertyHookKind,
        scopes: &ScopeStack,
        context: Option<&MethodContext>,
    ) -> Option<MethodInfo> {
        let Expr::PropertyAccess {
            object, property, ..
        } = expr
        else {
            return None;
        };
        let receiver = self.infer_expr_type(object, scopes, context);
        if let Some(receiver) = self.property_contract_receiver(receiver) {
            return self
                .property_contract_requirement(receiver, property, kind, expr.span())
                .ok()
                .flatten()
                .map(|required| required.method);
        }
        let (owner, info) = self.property_for_return_borrow(object, property, scopes, context)?;
        if self.accesses_hook_backing(object, &info) {
            return None;
        }
        self.property_accessor_method(&info, &owner, kind)
    }

    pub(super) fn property_contract_receiver(&self, ty: TypeId) -> Option<TypeId> {
        match self.types.kind(ty) {
            TypeKind::Interface(_) | TypeKind::TypeParameter(_) => Some(ty),
            TypeKind::Nullable(inner) => self.property_contract_receiver(*inner),
            TypeKind::SharedHandle(kind, inner) if Self::shared_handle_forwards(*kind) => {
                self.property_contract_receiver(*inner)
            }
            _ => None,
        }
    }

    pub(super) fn property_contract_requirement(
        &mut self,
        receiver: TypeId,
        property: &str,
        kind: PropertyHookKind,
        span: Span,
    ) -> Result<Option<contracts::CanonicalRequirement>, Vec<Span>> {
        match self.types.kind(receiver).clone() {
            TypeKind::Interface(interface) => {
                Ok(self.interface_requirement(&interface, property, Some(kind)))
            }
            TypeKind::TypeParameter(parameter) => {
                self.constrained_member_requirement(&parameter, property, Some(kind), span)
            }
            _ => unreachable!("a property contract has an interface or constrained receiver"),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn check_contract_property_accessor(
        &mut self,
        receiver: TypeId,
        object: &Expr,
        property: &str,
        kind: PropertyHookKind,
        member_span: Span,
        span: Span,
        scopes: &ScopeStack,
        context: Option<&MethodContext>,
    ) -> Option<MethodInfo> {
        let required = match self.property_contract_requirement(receiver, property, kind, span) {
            Ok(Some(required)) => required,
            Ok(None) => {
                let other = match kind {
                    PropertyHookKind::Get => PropertyHookKind::Set,
                    PropertyHookKind::Set => PropertyHookKind::Get,
                };
                if let Ok(Some(other)) =
                    self.property_contract_requirement(receiver, property, other, span)
                {
                    self.report_missing_property_accessor(
                        receiver,
                        property,
                        kind,
                        span,
                        other.method.declaration,
                    );
                } else {
                    let constrained =
                        matches!(self.types.kind(receiver), TypeKind::TypeParameter(_));
                    self.diagnostics.push(
                        Diagnostic::new(
                            if constrained { "E0537" } else { "E0303" },
                            format!(
                                "property `{property}` is not guaranteed by `{}`",
                                self.types.display(receiver)
                            ),
                            member_span,
                        )
                        .with_title("Unknown Contract Property")
                        .with_help(
                            "declare the property accessor in the receiver's interface contract",
                        ),
                    );
                }
                return None;
            }
            Err(origins) => {
                let mut diagnostic = Diagnostic::new(
                    "E0753",
                    format!(
                        "constraints on `{}` provide conflicting contracts for `{property}`",
                        self.types.display(receiver)
                    ),
                    member_span,
                )
                .with_title("Conflicting Constrained Properties");
                for origin in origins {
                    diagnostic = diagnostic
                        .with_related(origin, "this constraint declares a different contract");
                }
                self.diagnostics.push(diagnostic);
                return None;
            }
        };
        self.record_contract_member_reference(
            member_span,
            required.origins.iter().map(|(_, span)| *span).collect(),
        );
        self.record_property_accessor_call(
            object,
            property,
            receiver,
            &required.method,
            kind,
            span,
            scopes,
            context,
        );
        Some(required.method)
    }

    pub(super) fn property_for_return_borrow(
        &mut self,
        object: &Expr,
        property: &str,
        scopes: &ScopeStack,
        context: Option<&MethodContext>,
    ) -> Option<(ClassType<TypeId>, PropertyInfo)> {
        let object_ty = self.infer_expr_type(object, scopes, context);
        if let TypeKind::TraitSelf(owner) = self.types.kind(object_ty).clone() {
            return self
                .trait_property(&owner, property)
                .map(|info| (ClassType::new(owner, Vec::new()), info));
        }
        let class = self.expr_class_type(object, scopes, context)?;
        self.lookup_instance_property(&class, property)
    }

    pub(super) fn check_property_read(
        &mut self,
        object: &Expr,
        property: &str,
        member_span: Span,
        span: Span,
        scopes: &ScopeStack,
        context: Option<&MethodContext>,
    ) {
        let receiver = self.infer_expr_type(object, scopes, context);
        if let Some(receiver) = self.property_contract_receiver(receiver) {
            self.check_contract_property_accessor(
                receiver,
                object,
                property,
                PropertyHookKind::Get,
                member_span,
                span,
                scopes,
                context,
            );
            return;
        }
        if let Some((_, owner, info)) =
            self.lookup_property(object, property, span, scopes, context)
        {
            self.check_property_accessor_call(
                object,
                property,
                &info,
                &owner,
                PropertyHookKind::Get,
                span,
                scopes,
                context,
            );
        }
    }

    /// The lexical accessor, including a closure nested in its body, may use
    /// its own storage. All other reads and writes invoke the public accessor.
    pub(super) fn accesses_hook_backing(&self, object: &Expr, property: &PropertyInfo) -> bool {
        let Some(hooks) = &property.hooks else {
            return false;
        };
        if !Self::is_direct_this(object) {
            return false;
        }
        let mut owner = self.current_lexical_owner;
        loop {
            if let LexicalOwner::Callable(span) = owner {
                return [hooks.getter, hooks.setter]
                    .into_iter()
                    .flatten()
                    .any(|accessor| accessor.declaration == span);
            }
            let Some(parent) = self.binding_resolution.lexical_parents.get(&owner) else {
                return false;
            };
            owner = *parent;
        }
    }

    pub(super) fn property_accessor_method(
        &mut self,
        property: &PropertyInfo,
        owner: &ClassType<TypeId>,
        kind: PropertyHookKind,
    ) -> Option<MethodInfo> {
        let hooks = property.hooks.as_ref()?;
        let accessor = match kind {
            PropertyHookKind::Get => hooks.getter?,
            PropertyHookKind::Set => hooks.setter?,
        };
        let signature = self.function_signatures.get(&accessor.declaration)?;
        let method = MethodInfo {
            declaration: accessor.declaration,
            is_open: hooks.is_open,
            is_override: hooks.is_override,
            virtual_root: self
                .override_roots
                .get(&accessor.declaration)
                .copied()
                .or_else(|| hooks.is_open.then_some(accessor.declaration)),
            access: property.access,
            receiver_mode: Some(accessor.receiver_mode),
            return_borrow: signature.return_borrow,
            is_static: false,
            enclosing_type_bindings: HashMap::new(),
            type_params: signature.type_params.clone(),
            params: signature.params.clone(),
            return_ty: signature.return_ty,
            checked_effects: signature.checked_effects.clone(),
        };
        Some(self.specialize_method_for_class(&method, owner))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn check_property_accessor_call(
        &mut self,
        object: &Expr,
        property_name: &str,
        property: &PropertyInfo,
        owner: &ClassType<TypeId>,
        kind: PropertyHookKind,
        span: Span,
        scopes: &ScopeStack,
        context: Option<&MethodContext>,
    ) {
        if property.hooks.is_none() || self.accesses_hook_backing(object, property) {
            return;
        }
        let declaring_type = self.symbolic_property_owner(owner);
        let Some(method) = self.property_accessor_method(property, owner, kind) else {
            self.report_missing_property_accessor(
                declaring_type,
                property_name,
                kind,
                span,
                property.declaration_span,
            );
            return;
        };
        self.record_property_accessor_call(
            object,
            property_name,
            declaring_type,
            &method,
            kind,
            span,
            scopes,
            context,
        );
    }

    fn symbolic_property_owner(&mut self, owner: &ClassType<TypeId>) -> TypeId {
        if self.trait_declaration(&owner.name).is_some() {
            self.symbolic_class_type(&owner.name)
        } else {
            self.types.intern(TypeKind::Class(owner.clone()))
        }
    }

    fn report_missing_property_accessor(
        &mut self,
        owner: TypeId,
        property: &str,
        kind: PropertyHookKind,
        span: Span,
        declaration: Span,
    ) {
        let (code, accessor, action) = match kind {
            PropertyHookKind::Get => ("E0767", "getter", "read"),
            PropertyHookKind::Set => ("E0768", "setter", "assigned"),
        };
        self.diagnostics.push(
            Diagnostic::new(
                code,
                format!(
                    "property `{}::{property}` has no {accessor} and cannot be {action}",
                    self.types.display(owner)
                ),
                span,
            )
            .with_title("Property Accessor Is Missing")
            .with_related(declaration, "Property Declared Here"),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn record_property_accessor_call(
        &mut self,
        object: &Expr,
        property_name: &str,
        declaring_type: TypeId,
        method: &MethodInfo,
        kind: PropertyHookKind,
        span: Span,
        scopes: &ScopeStack,
        context: Option<&MethodContext>,
    ) {
        let receiver_mode = method
            .receiver_mode
            .expect("instance accessor has a receiver");
        if receiver_mode.is_writable() {
            self.record_capture_requirement_for_expr(object, scopes, CaptureRequirement::Writable);
            if !self.is_writable_object_path(object, scopes, context) {
                self.diagnostics.push(
                    Diagnostic::new("E0203", format!("cannot invoke writable accessor for `{}::{property_name}` through readonly value", self.types.display(declaring_type)), span)
                        .with_title("Property Accessor Requires Writable Access"),
                );
            }
        }
        self.record_callable_dependency(method.declaration);
        let declaring_type = self.types.resolved(declaring_type);
        let target = match &declaring_type {
            ResolvedType::Class(class_type) => CallableTarget::Method {
                class_type: class_type.clone(),
                method_name: property_name.into(),
                direct_parent: false,
            },
            ResolvedType::Interface(interface) => CallableTarget::InterfaceMethod {
                interface: interface.clone(),
                method_name: property_name.into(),
                requirement: method.declaration,
            },
            _ => CallableTarget::ConstrainedMethod {
                receiver: declaring_type.clone(),
                method_name: property_name.into(),
                requirement: method.declaration,
                implementations: Vec::new(),
            },
        };
        let effects = self.selected_contract_effects(&target, method, span);
        self.record_checked_effects(effects.iter().copied(), span);
        let call = PropertyAccessorCallSemanticInfo {
            target,
            declaring_type,
            declaration: method.declaration,
            virtual_root: method.virtual_root,
            receiver_mode,
            return_type: self.types.resolved(method.return_ty),
            return_borrow: method.return_borrow,
            checked_effects: effects
                .iter()
                .map(|effect| self.types.resolved(*effect))
                .collect(),
            parameter: method
                .params
                .first()
                .map(|parameter| CallableParameterSemanticInfo {
                    name: parameter.name.clone(),
                    r#type: self.types.resolved(parameter.ty),
                    take: parameter.take,
                    writable: parameter.writable,
                    borrow: false,
                    has_default: false,
                }),
        };
        let calls = self.property_accessor_calls.entry(span).or_default();
        match kind {
            PropertyHookKind::Get => calls.getter = Some(call),
            PropertyHookKind::Set => calls.setter = Some(call),
        }
    }

    pub(super) fn specialize_constrained_property_accessors(
        &mut self,
        callable: Span,
        substitutions: &HashMap<String, TypeId>,
    ) {
        let mut calls = std::mem::take(&mut self.property_accessor_calls);
        for (span, accessors) in &mut calls {
            if !callable.contains(*span) {
                continue;
            }
            for (kind, call) in [
                (PropertyHookKind::Get, accessors.getter.as_mut()),
                (PropertyHookKind::Set, accessors.setter.as_mut()),
            ] {
                let Some(call) = call else { continue };
                let CallableTarget::ConstrainedMethod {
                    receiver,
                    method_name,
                    implementations,
                    ..
                } = &mut call.target
                else {
                    continue;
                };
                let receiver = self.types.intern_resolved(receiver);
                let receiver = self.substitute_type_id(receiver, substitutions);
                if self.type_is_symbolic(receiver) {
                    continue;
                }
                let TypeKind::Class(class) = self.types.kind(receiver).clone() else {
                    continue;
                };
                if self.class_requires_trait_composition(&class.name)
                    || !self.class_composition_is_valid(&class.name)
                {
                    continue;
                }
                let Some((declaring_class, property)) =
                    self.lookup_instance_property(&class, method_name)
                else {
                    continue;
                };
                let Some(method) = self.property_accessor_method(&property, &declaring_class, kind)
                else {
                    continue;
                };
                let declaring_type = self.types.intern(TypeKind::Class(declaring_class));
                let ResolvedType::Class(declaring_class) = self.types.resolved(declaring_type)
                else {
                    unreachable!()
                };
                let implementation = ConstrainedMethodImplementation {
                    receiver: self.types.resolved(receiver),
                    declaring_class,
                    declaration: method.declaration,
                };
                if !implementations.contains(&implementation) {
                    implementations.push(implementation);
                }
            }
        }
        self.property_accessor_calls = calls;
    }

    pub(super) fn collect_property_hook_signatures(
        &mut self,
        property: &PropertyDecl,
        owner: &str,
        context: PropertyHookContext,
    ) {
        let Some(facts) = declaration_facts(property, context) else {
            return;
        };
        let property_type = self.resolve_type_ref_in_position(
            &property.ty,
            property.name_span,
            TypePosition::Value,
            Some(owner),
        );
        for (accessor, callable) in facts.accessors().zip(facts.callables()) {
            let mut signature = self.resolve_function_signature(&callable, Some(owner));
            if accessor.hook.kind == PropertyHookKind::Set {
                if let Some(parameter) = signature.params.first() {
                    if parameter.ty != property_type
                        && !matches!(self.types.kind(parameter.ty), TypeKind::Unknown)
                        && !matches!(self.types.kind(property_type), TypeKind::Unknown)
                    {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0769",
                                format!(
                                    "setter for `${}` accepts `{}`, but the property has type `{}`",
                                    property.name,
                                    self.types.display(parameter.ty),
                                    self.types.display(property_type),
                                ),
                                accessor.hook.parameter.as_ref().unwrap().span,
                            )
                            .with_title("Setter Input Type Must Match The Property")
                            .with_related(property.name_span, "Property Declared Here")
                            .with_help("use the property's type for the setter parameter; put conversions in an explicitly called method"),
                        );
                    }
                }
            }
            if context == PropertyHookContext::Interface
                && self.type_is_move_type(signature.return_ty)
            {
                let kind = if accessor.hook.borrowed_span.is_some() {
                    Some(crate::types::ReturnBorrowKind::Value)
                } else if self.non_null_function_type(signature.return_ty).is_some() {
                    Some(crate::types::ReturnBorrowKind::Retained)
                } else {
                    None
                };
                signature.return_borrow = kind.map(|kind| ReturnBorrow {
                    source: BorrowSource::Receiver,
                    writable: false,
                    kind,
                });
            }
            self.function_signatures.insert(callable.span, signature);
        }
    }

    pub(super) fn check_property_hook_bodies(
        &mut self,
        property: &PropertyDecl,
        owner: &str,
        context: PropertyHookContext,
    ) {
        let Some(facts) = declaration_facts(property, context) else {
            return;
        };
        for callable in facts.callables() {
            self.check_function(
                &callable,
                Some(MethodContext {
                    class_name: owner.to_string(),
                    receiver_access: if callable.is_static {
                        ReceiverAccess::Unavailable
                    } else if callable.writable_this {
                        ReceiverAccess::Writable
                    } else {
                        ReceiverAccess::Readonly
                    },
                }),
            );
        }
    }

    pub(super) fn check_property_hook_return_contracts(
        &mut self,
        return_borrows: &HashMap<Span, ReturnBorrow>,
    ) {
        for item in &self.program.items {
            let members = match item {
                Item::Class(class) => &class.members,
                Item::Trait(declaration) => &declaration.members,
                _ => continue,
            };
            for member in members {
                let ClassMember::Property(property) = member else {
                    continue;
                };
                for hook in &property.hooks {
                    let Some(span) = hook.borrowed_span else {
                        continue;
                    };
                    let Some(signature) = self.function_signatures.get(&hook.span) else {
                        continue;
                    };
                    if hook.body.as_block().is_none()
                        || !self.type_is_move_type(signature.return_ty)
                        || return_borrows.get(&hook.span).is_some_and(|borrow| {
                            borrow.source == BorrowSource::Receiver
                                && borrow.kind == crate::types::ReturnBorrowKind::Value
                        })
                    {
                        continue;
                    }
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0766",
                            format!("getter for `${}` does not return a borrowed value from its receiver", property.name),
                            span,
                        )
                        .with_title("Getter Does Not Satisfy Its Borrowed Result Contract")
                        .with_related(property.name_span, "Property Declared Here")
                        .with_explanation("`borrowed get` lends an existing value from the receiver; it does not give the caller an independently owned value")
                        .with_help("return a value borrowed from the receiver, or remove `borrowed` if the getter is intended to return ownership"),
                    );
                }
            }
        }
    }
}
