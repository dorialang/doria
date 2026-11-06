//! Nominal type formation uses caller premises and fully substituted arguments.
//! Resolving a proof rule's identity must not validate it in its caller's scope.

use super::nominal_proof::{Context, Query};
use super::*;
use crate::types::InterfaceType;

pub(super) struct ConditionTypeCheck {
    declaration: Span,
    condition: contracts::NominalConditionFacts,
    caller: Context,
}

pub(super) struct ClassConformanceCheck {
    class: ClassType<TypeId>,
    caller: Context,
}

impl Checker<'_> {
    pub(super) fn with_nominal_identity<T>(&mut self, resolve: impl FnOnce(&mut Self) -> T) -> T {
        self.nominal_identity_depth += 1;
        let resolved = resolve(self);
        self.nominal_identity_depth -= 1;
        resolved
    }

    pub(super) fn defer_condition_type_check(
        &mut self,
        declaration: Span,
        condition: contracts::NominalConditionFacts,
    ) {
        let mut caller = self.nominal_proof_context();
        let bindings = condition
            .parameters
            .iter()
            .map(|parameter| {
                (
                    parameter.name.clone(),
                    self.types
                        .intern(TypeKind::TypeParameter(parameter.name.clone())),
                )
            })
            .collect();
        if let Ok(steps) = self.condition_queries(&condition, &bindings) {
            for step in steps {
                caller.assume(step);
            }
        }
        self.pending_condition_type_checks.push(ConditionTypeCheck {
            declaration,
            condition,
            caller,
        });
    }

    pub(super) fn check_condition_type_formations(&mut self) -> Vec<ConditionTypeCheck> {
        let mut checked = Vec::new();
        for mut check in std::mem::take(&mut self.pending_condition_type_checks) {
            let mut valid = true;
            let mut visited = HashSet::new();
            for obligation in &check.condition.obligations {
                let interface = InterfaceType::new(
                    &obligation.interface.name,
                    obligation
                        .interface
                        .arguments
                        .iter()
                        .map(|ty| self.types.intern_resolved(ty))
                        .collect(),
                );
                let ty = self.types.intern(TypeKind::Interface(interface));
                valid &= self.nominal_type_well_formed(
                    ty,
                    obligation.interface_span,
                    &mut check.caller,
                    &mut visited,
                );
            }
            if !valid {
                self.contracts.conditions.remove(&check.declaration);
            } else {
                checked.push(check);
            }
        }
        checked
    }

    pub(super) fn check_condition_conformances(&mut self, checks: Vec<ConditionTypeCheck>) {
        let mut invalid = Vec::new();
        for check in checks {
            let mut caller = check.caller.frozen_premises();
            let mut valid = true;
            let mut visited = HashSet::new();
            for obligation in &check.condition.obligations {
                for argument in &obligation.interface.arguments {
                    let ty = self.types.intern_resolved(argument);
                    valid &= self.formed_type_conformances_coherent(ty, &mut caller, &mut visited);
                }
            }
            if !valid {
                invalid.push(check.declaration);
            }
        }
        if !invalid.is_empty() {
            for declaration in invalid {
                self.contracts.conditions.remove(&declaration);
            }
            self.declared_class_conformance_cache.clear();
        }
    }

    pub(super) fn defer_class_conformance_check(
        &mut self,
        class: ClassType<TypeId>,
        caller: &Context,
    ) {
        self.defer_class_conformance_check_in(class, caller, self.current_callable);
    }

    fn defer_class_conformance_check_in(
        &mut self,
        class: ClassType<TypeId>,
        caller: &Context,
        callable: Option<Span>,
    ) {
        if self.nominal_identity_depth == 0 {
            if let Some(callable) = callable {
                self.callable_coherence_obligations
                    .entry(callable)
                    .or_default()
                    .insert(class.clone());
            }
            self.pending_class_conformance_checks
                .push(ClassConformanceCheck {
                    class,
                    caller: caller.frozen_premises(),
                });
        }
    }

    pub(super) fn check_pending_class_conformances(&mut self) {
        for mut check in std::mem::take(&mut self.pending_class_conformance_checks) {
            let ty = self.types.intern(TypeKind::Class(check.class));
            self.formed_type_conformances_coherent(ty, &mut check.caller, &mut HashSet::new());
        }
    }

    pub(super) fn check_specialized_callable_coherence(
        &mut self,
        body: &callable_contracts::BodyInstance,
    ) {
        let Some(obligations) = self
            .callable_coherence_obligations
            .get(&body.declaration)
            .cloned()
        else {
            return;
        };
        let mut classes = obligations
            .into_iter()
            .map(|class| {
                ClassType::new(
                    class.name,
                    class
                        .arguments
                        .into_iter()
                        .map(|ty| body.substitute_id(ty, &mut self.types))
                        .collect(),
                )
            })
            .collect::<Vec<_>>();
        classes.sort_by_key(|class| {
            (
                class.name.clone(),
                class
                    .arguments
                    .iter()
                    .map(|ty| self.types.resolved(*ty))
                    .collect::<Vec<_>>(),
            )
        });
        for class in classes {
            if class.arguments.iter().any(|ty| self.type_is_symbolic(*ty)) {
                continue;
            }
            // Concrete calls must establish guards from nominal declarations;
            // symbolic declaration premises cannot manufacture a capability.
            self.defer_class_conformance_check(class, &Context::default());
        }
    }

    pub(super) fn defer_formed_type_conformances(&mut self, ty: TypeId) {
        if self.nominal_identity_depth > 0 {
            return;
        }
        let mut classes = Vec::new();
        self.formed_class_types(ty, &mut HashSet::new(), &mut classes);
        if !classes.is_empty() {
            let caller = self.nominal_proof_context();
            for class in classes {
                self.defer_class_conformance_check(class, &caller);
            }
        }
    }

    pub(super) fn defer_formed_type_conformances_in(
        &mut self,
        ty: TypeId,
        caller: &Context,
        callable: Option<Span>,
    ) {
        if self.nominal_identity_depth > 0 {
            return;
        }
        let mut classes = Vec::new();
        self.formed_class_types(ty, &mut HashSet::new(), &mut classes);
        for class in classes {
            self.defer_class_conformance_check_in(class, caller, callable);
        }
    }

    fn formed_type_conformances_coherent(
        &mut self,
        ty: TypeId,
        caller: &mut Context,
        visited: &mut HashSet<TypeId>,
    ) -> bool {
        let mut classes = Vec::new();
        self.formed_class_types(ty, visited, &mut classes);
        let mut parents = HashMap::new();
        let mut valid = true;
        let mut cursor = 0;
        while cursor < classes.len() {
            let class = classes[cursor].clone();
            cursor += 1;
            let conformances = self.class_interface_closure_in(&class, &mut HashSet::new(), caller);
            valid &= conformances.valid;
            // Only proved edges contribute targets. Unproved guards remain in
            // the root formation's obligation for concrete substitution.
            let mut dependencies = conformances.active_arguments;
            dependencies.extend(self.specialized_stored_types(&class));
            for dependency in dependencies {
                let mut nested = Vec::new();
                self.formed_class_types(dependency, visited, &mut nested);
                for child in nested {
                    if let Some(cycle) = super::instantiation::expanding_specialization_cycle(
                        &parents,
                        &class,
                        &child,
                        &self.types,
                    ) {
                        if !child
                            .arguments
                            .iter()
                            .any(|argument| self.type_is_symbolic(*argument))
                        {
                            self.report_expanding_nominal_specialization(&cycle);
                            valid = false;
                        }
                        continue;
                    }
                    parents.insert(child.clone(), class.clone());
                    classes.push(child);
                }
            }
        }
        valid
    }

    pub(super) fn specialized_stored_types(&mut self, class: &ClassType<TypeId>) -> Vec<TypeId> {
        let Some(info) = self.classes.get(&class.name) else {
            return Vec::new();
        };
        let mut stored = info
            .properties
            .iter()
            .filter(|(_, property)| {
                !property.hooks.as_ref().is_some_and(|hooks| {
                    hooks.storage == Some(crate::property_hooks::PropertyHookStorage::Computed)
                })
            })
            .map(|(name, property)| (name.clone(), property.ty))
            .chain(
                info.static_properties
                    .iter()
                    .filter(|(_, property)| {
                        !property.hooks.as_ref().is_some_and(|hooks| {
                            hooks.storage
                                == Some(crate::property_hooks::PropertyHookStorage::Computed)
                        })
                    })
                    .map(|(name, property)| (name.clone(), property.ty)),
            )
            .collect::<Vec<_>>();
        stored.sort_by(|left, right| left.0.cmp(&right.0));
        let bindings = self.class_type_substitutions(class);
        let mut types = stored
            .into_iter()
            .map(|(_, ty)| self.substitute_type_id(ty, &bindings))
            .collect::<Vec<_>>();
        if let Some(parent) = self.specialized_parent_type(class) {
            types.push(self.types.intern(TypeKind::Class(parent)));
        }
        types
    }

    pub(super) fn report_expanding_nominal_specialization(&mut self, cycle: &[String]) {
        let Some((name, span)) = cycle
            .iter()
            .filter_map(|name| {
                self.classes
                    .get(name)
                    .map(|info| (name, info.declaration))
                    .or_else(|| self.enums.get(name).map(|info| (name, info.span)))
            })
            .min_by_key(|(_, span)| *span)
        else {
            return;
        };
        let message = format!(
            "generic specialization of {} `{}` recursively expands its type arguments and has no finite monomorphization",
            if self.classes.contains_key(name) { "class" } else { "enum" }, name
        );
        if !self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0539" && diagnostic.message == message)
        {
            self.diagnostics.push(
                Diagnostic::new("E0539", message, span).with_help(
                    "keep recursive type references at the same concrete type, or move the type-changing step outside the recursion",
                ),
            );
        }
    }

    fn formed_class_types(
        &self,
        ty: TypeId,
        visited: &mut HashSet<TypeId>,
        classes: &mut Vec<ClassType<TypeId>>,
    ) {
        let mut instances = Vec::new();
        self.formed_nominal_types(ty, visited, &mut instances);
        classes.extend(instances.into_iter().filter_map(|instance| match instance {
            instantiation::NominalInstance::Class(class) => Some(class),
            instantiation::NominalInstance::Enum(_) => None,
        }));
    }

    pub(super) fn formed_nominal_types(
        &self,
        ty: TypeId,
        visited: &mut HashSet<TypeId>,
        instances: &mut Vec<instantiation::NominalInstance>,
    ) {
        if matches!(
            self.types.kind(ty),
            TypeKind::Integer(_)
                | TypeKind::Float(_)
                | TypeKind::Bool
                | TypeKind::String
                | TypeKind::Void
                | TypeKind::Null
                | TypeKind::Unknown
                | TypeKind::TypeParameter(_)
        ) {
            return;
        }
        if !visited.insert(ty) {
            return;
        }
        let children = match self.types.kind(ty).clone() {
            TypeKind::Class(class) => {
                instances.push(instantiation::NominalInstance::Class(class.clone()));
                class.arguments
            }
            TypeKind::Interface(interface) => interface.arguments,
            TypeKind::Enum(enum_type) => {
                instances.push(instantiation::NominalInstance::Enum(enum_type.clone()));
                enum_type.arguments
            }
            TypeKind::Nullable(inner)
            | TypeKind::TypedArray(inner)
            | TypeKind::List(inner)
            | TypeKind::Set(inner)
            | TypeKind::SortedSet(inner)
            | TypeKind::PriorityQueue(inner)
            | TypeKind::Deque(inner)
            | TypeKind::SharedHandle(_, inner) => vec![inner],
            TypeKind::Dictionary(key, value) | TypeKind::SortedDictionary(key, value) => {
                vec![key, value]
            }
            TypeKind::Function(function) => function
                .parameters
                .into_iter()
                .map(|parameter| parameter.ty)
                .chain([function.return_type])
                .chain(function.checked_effects)
                .collect(),
            _ => Vec::new(),
        };
        for child in children {
            self.formed_nominal_types(child, visited, instances);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn nominal_arguments_well_formed(
        &mut self,
        name: &str,
        parameters: &[TypeParamInfo],
        arguments: &[TypeId],
        span: Span,
        caller: &mut Context,
        visited: &mut HashSet<TypeId>,
    ) -> bool {
        let mut valid = true;
        for argument in arguments {
            valid &= self.nominal_type_well_formed(*argument, span, caller, visited);
        }
        let bindings = parameters
            .iter()
            .zip(arguments)
            .map(|(parameter, argument)| (parameter.name.clone(), *argument))
            .collect();
        for (parameter, argument) in parameters.iter().zip(arguments) {
            if matches!(self.types.kind(*argument), TypeKind::Unknown) {
                valid = false;
                continue;
            }
            for constraint in &parameter.constraints {
                if self.argument_bound_holds(
                    constraint,
                    *argument,
                    &bindings,
                    Some(name),
                    span,
                    caller,
                    visited,
                ) {
                    continue;
                }
                valid = false;
                self.report_type_argument_bound(name, *argument, constraint, span);
            }
        }
        valid
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn argument_bound_holds(
        &mut self,
        constraint: &TypeRef,
        argument: TypeId,
        bindings: &HashMap<String, TypeId>,
        owner: Option<&str>,
        span: Span,
        caller: &mut Context,
        visited: &mut HashSet<TypeId>,
    ) -> bool {
        if matches!(self.types.kind(argument), TypeKind::Unknown)
            && self.primitive_satisfies_compiler_constraint(argument, &constraint.name)
        {
            return true;
        }
        let target = self.with_nominal_identity(|checker| {
            checker.type_parameter_scopes.push(
                bindings
                    .keys()
                    .map(|name| (name.clone(), Vec::new()))
                    .collect(),
            );
            let target = if checker.is_core_interface(&constraint.name)
                && matches!(constraint.name.as_str(), "Comparable" | "Equatable")
                && constraint.arguments.is_empty()
            {
                Some(InterfaceType::new(&constraint.name, vec![argument]))
            } else {
                checker.resolve_interface_edge_in_owner(constraint, span, owner)
            };
            checker.type_parameter_scopes.pop();
            target.map(|target| {
                InterfaceType::new(
                    target.name,
                    target
                        .arguments
                        .iter()
                        .map(|ty| checker.substitute_type_id(*ty, bindings))
                        .collect(),
                )
            })
        });
        let Some(target) = target else {
            return false;
        };
        let target_ty = self.types.intern(TypeKind::Interface(target.clone()));
        self.nominal_type_well_formed(target_ty, span, caller, visited)
            && self
                .prove_nominal(
                    Query {
                        subject: argument,
                        interface: target,
                    },
                    caller,
                    Some(span),
                    &[],
                )
                .is_ok()
    }

    fn nominal_type_well_formed(
        &mut self,
        ty: TypeId,
        span: Span,
        caller: &mut Context,
        visited: &mut HashSet<TypeId>,
    ) -> bool {
        if !visited.insert(ty) {
            return true;
        }
        match self.types.kind(ty).clone() {
            TypeKind::Enum(enum_type) => {
                let Some(parameters) = self
                    .enums
                    .get(&enum_type.name)
                    .map(|definition| definition.type_params.clone())
                else {
                    return false;
                };
                self.nominal_arguments_well_formed(
                    &enum_type.name,
                    &parameters,
                    &enum_type.arguments,
                    span,
                    caller,
                    visited,
                )
            }
            TypeKind::Class(class) => {
                let Some(parameters) = self
                    .classes
                    .get(&class.name)
                    .map(|info| info.type_params.clone())
                else {
                    return false;
                };
                let valid = self.nominal_arguments_well_formed(
                    &class.name,
                    &parameters,
                    &class.arguments,
                    span,
                    caller,
                    visited,
                );
                if valid {
                    self.defer_class_conformance_check(class, caller);
                }
                valid
            }
            TypeKind::Interface(interface) => {
                let Some(declaration) = self.interface_declaration(&interface.name).cloned() else {
                    return false;
                };
                let parameters = declaration
                    .type_params
                    .iter()
                    .map(|parameter| TypeParamInfo {
                        name: parameter.name.clone(),
                        constraints: parameter.constraints.clone(),
                    })
                    .collect::<Vec<_>>();
                self.nominal_arguments_well_formed(
                    &interface.name,
                    &parameters,
                    &interface.arguments,
                    span,
                    caller,
                    visited,
                )
            }
            TypeKind::Nullable(inner)
            | TypeKind::TypedArray(inner)
            | TypeKind::List(inner)
            | TypeKind::Deque(inner)
            | TypeKind::SharedHandle(_, inner) => {
                self.nominal_type_well_formed(inner, span, caller, visited)
            }
            TypeKind::Set(inner) | TypeKind::SortedSet(inner) | TypeKind::PriorityQueue(inner) => {
                let (name, bound) = match self.types.kind(ty) {
                    TypeKind::Set(_) => ("Set", "Hashable"),
                    TypeKind::SortedSet(_) => ("SortedSet", "Comparable"),
                    _ => ("PriorityQueue", "Comparable"),
                };
                let parameter = TypeParamInfo {
                    name: "T".into(),
                    constraints: vec![TypeRef::named(bound)],
                };
                self.nominal_arguments_well_formed(
                    name,
                    &[parameter],
                    &[inner],
                    span,
                    caller,
                    visited,
                )
            }
            TypeKind::Dictionary(key, value) | TypeKind::SortedDictionary(key, value) => {
                let (name, bound) = if matches!(self.types.kind(ty), TypeKind::Dictionary(_, _)) {
                    ("Dictionary", "Hashable")
                } else {
                    ("SortedDictionary", "Comparable")
                };
                let parameters = [
                    TypeParamInfo {
                        name: "K".into(),
                        constraints: vec![TypeRef::named(bound)],
                    },
                    TypeParamInfo {
                        name: "V".into(),
                        constraints: vec![],
                    },
                ];
                self.nominal_arguments_well_formed(
                    name,
                    &parameters,
                    &[key, value],
                    span,
                    caller,
                    visited,
                )
            }
            TypeKind::Function(function) => {
                let mut valid = true;
                for child in function
                    .parameters
                    .iter()
                    .map(|parameter| parameter.ty)
                    .chain([function.return_type])
                    .chain(function.checked_effects)
                {
                    valid &= self.nominal_type_well_formed(child, span, caller, visited);
                }
                valid
            }
            _ => true,
        }
    }

    fn report_type_argument_bound(
        &mut self,
        name: &str,
        argument: TypeId,
        constraint: &TypeRef,
        span: Span,
    ) {
        let message = format!("type argument `{}` for `{name}<...>` does not satisfy required constraint `{constraint}`", self.types.display(argument));
        if !self.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "E0535" && diagnostic.span == span && diagnostic.message == message
        }) {
            self.diagnostics
                .push(Diagnostic::new("E0535", message, span));
        }
    }
}
