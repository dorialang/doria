//! Declaration contracts, independent of runtime carriers and backend layout.

use super::*;
use crate::types::InterfaceType;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContractFacts {
    pub interfaces: Vec<InterfaceFacts>,
    pub interface_specializations: Vec<InterfaceSpecializationFacts>,
    pub traits: Vec<TraitFacts>,
    pub conformances: Vec<ConformanceFacts>,
    pub boundaries: Vec<SupportBoundary>,
    pub member_references: Vec<ContractMemberReference>,
    pub compositions: Vec<CompositionFacts>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositionFacts {
    pub class: String,
    pub span: Span,
    pub uses: Vec<ContractEdge>,
    pub valid: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractMemberReference {
    pub span: Span,
    pub origins: Vec<Span>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractEdge {
    pub authored_type: TypeRef,
    pub specialization: crate::types::NominalType<ResolvedType>,
    pub span: Span,
    pub declaration: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceFacts {
    pub name: String,
    pub declaration: Span,
    pub name_span: Span,
    pub type_parameters: Vec<TypeParamDecl>,
    pub parents: Vec<ContractEdge>,
    pub requirements: Vec<RequirementFacts>,
    pub valid: bool,
}

/// The checked, substituted requirement graph used by runtime lowering. Keeping
/// this separate from declaration syntax prevents backend conformance lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceSpecializationFacts {
    pub specialization: InterfaceType<ResolvedType>,
    pub ancestors: Vec<InterfaceType<ResolvedType>>,
    pub requirements: Vec<RequirementFacts>,
    pub valid: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementOrigin {
    pub interface: InterfaceType<ResolvedType>,
    pub declaration: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementFacts {
    pub name: String,
    pub origins: Vec<RequirementOrigin>,
    pub signature: CallableSignatureSemanticInfo,
    pub generic_parameters: Vec<TypeParamDecl>,
    pub writable_receiver: bool,
    pub checked_effects: Vec<ResolvedType>,
    pub return_borrow: Option<ReturnBorrow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraitFacts {
    pub name: String,
    pub declaration: Span,
    pub name_span: Span,
    pub type_parameters: Vec<TypeParamDecl>,
    pub uses: Vec<ContractEdge>,
    pub valid: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConformanceStatus {
    Checked,
    Invalid,
    DeferredComposition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementImplementation {
    pub requirement_origins: Vec<RequirementOrigin>,
    pub implementation: Option<Span>,
    pub failures: Vec<ContractMismatch>,
    pub exact_dynamic_return: Option<ResolvedType>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConformanceFacts {
    pub implementing_type: ResolvedType,
    pub interface: InterfaceType<ResolvedType>,
    pub origin: Span,
    pub status: ConformanceStatus,
    pub implementations: Vec<RequirementImplementation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingContractOperation {
    CoreValueOperation,
    TraitComposition,
}

impl PendingContractOperation {
    pub fn slice(self) -> u8 {
        match self {
            Self::CoreValueOperation => 3,
            Self::TraitComposition => 4,
        }
    }

    pub fn diagnostic(self, span: Span) -> Diagnostic {
        let (code, description) = match self {
            Self::CoreValueOperation => ("E0759", "core value-contract execution"),
            Self::TraitComposition => ("E0493", "trait composition"),
        };
        Diagnostic::unsupported_stage(
            code,
            format!(
                "{description} is not yet supported; it requires Stage 35 Slice {}",
                self.slice()
            ),
            span,
        )
        .with_title(match self {
            Self::CoreValueOperation => "Core Contract Execution Is Not Yet Supported",
            Self::TraitComposition => "Trait Composition Is Not Yet Supported",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportBoundary {
    pub operation: PendingContractOperation,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub(super) struct InterfaceDefinition {
    declaration: InterfaceDecl,
    parents: Vec<(InterfaceType<TypeId>, Span)>,
    local_requirements: Vec<(String, MethodInfo)>,
    valid: bool,
}

#[derive(Debug, Clone)]
pub(super) struct CanonicalRequirement {
    name: String,
    origins: Vec<(InterfaceType<TypeId>, Span)>,
    method: MethodInfo,
}

#[derive(Debug, Clone, Default)]
pub(super) struct InterfaceRequirements {
    ancestors: Vec<InterfaceType<TypeId>>,
    requirements: Vec<CanonicalRequirement>,
    valid: bool,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ClassConformances {
    entries: Vec<(InterfaceType<TypeId>, Span)>,
    valid: bool,
}

fn type_ref_contains_self(ty: &TypeRef) -> bool {
    ty.name == "self"
        || ty.type_arguments().any(type_ref_contains_self)
        || ty
            .grouped
            .as_ref()
            .is_some_and(|grouped| type_ref_contains_self(&grouped.inner))
        || ty.function.as_ref().is_some_and(|function| {
            function
                .parameters
                .iter()
                .any(|parameter| type_ref_contains_self(&parameter.ty))
                || type_ref_contains_self(&function.return_type)
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractMismatch {
    GenericParameters,
    ParameterCount,
    ParameterNames,
    ParameterTypes,
    ParameterOwnership,
    Receiver,
    ReturnType,
    ReturnProvenance,
    ExactDynamicReturn,
    CheckedEffects,
    Accessibility,
    StaticMethod,
}

impl ContractMismatch {
    pub fn description(self) -> &'static str {
        match self {
            Self::GenericParameters => "generic arity or constraints",
            Self::ParameterCount => "parameter count",
            Self::ParameterNames => "parameter names",
            Self::ParameterTypes => "parameter types",
            Self::ParameterOwnership => "parameter ownership modes",
            Self::Receiver => "receiver access",
            Self::ReturnType => "return type",
            Self::ReturnProvenance => "return ownership or provenance",
            Self::ExactDynamicReturn => "owned exact dynamic implementing-class result",
            Self::CheckedEffects => "checked effects",
            Self::Accessibility => "external accessibility",
            Self::StaticMethod => "instance method identity",
        }
    }
}

impl Checker<'_> {
    pub(super) fn publish_interface_specializations(&mut self) {
        let conformances = self
            .contracts
            .conformances
            .iter()
            .map(|fact| fact.interface.clone())
            .collect::<Vec<_>>();
        for interface in conformances {
            let interface = InterfaceType::new(
                interface.name,
                interface
                    .arguments
                    .iter()
                    .map(|argument| self.types.intern_resolved(argument))
                    .collect(),
            );
            self.types.intern(TypeKind::Interface(interface));
        }
        // A type used only in a test or storage declaration still needs its
        // canonical graph. Building a graph can intern further referenced types.
        let mut next = 0;
        while let Some(kind) = self.types.kinds().get(next).cloned() {
            next += 1;
            if let TypeKind::Interface(interface) = kind {
                self.canonical_interface_requirements(&interface, &mut Vec::new());
            }
        }
        let mut specializations = self.interface_requirements.iter().collect::<Vec<_>>();
        specializations.sort_by_key(|(interface, _)| {
            (
                &interface.name,
                interface
                    .arguments
                    .iter()
                    .map(|ty| self.types.display(*ty))
                    .collect::<Vec<_>>(),
            )
        });
        self.contracts.interface_specializations = specializations
            .into_iter()
            .map(|(interface, graph)| InterfaceSpecializationFacts {
                specialization: self.resolved_interface(interface),
                ancestors: graph
                    .ancestors
                    .iter()
                    .map(|ancestor| self.resolved_interface(ancestor))
                    .collect(),
                requirements: graph
                    .requirements
                    .iter()
                    .map(|requirement| self.requirement_facts(requirement))
                    .collect(),
                valid: graph.valid,
            })
            .collect();
    }

    pub(super) fn interface_receiver(&self, ty: TypeId) -> Option<InterfaceType<TypeId>> {
        match self.types.kind(ty) {
            TypeKind::Interface(interface) => Some(interface.clone()),
            TypeKind::Nullable(inner) => self.interface_receiver(*inner),
            TypeKind::SharedHandle(kind, inner) if Self::shared_handle_forwards(*kind) => {
                self.interface_receiver(*inner)
            }
            _ => None,
        }
    }

    pub(super) fn interface_method(
        &mut self,
        interface: &InterfaceType<TypeId>,
        name: &str,
    ) -> Option<MethodInfo> {
        let mut method = self
            .canonical_interface_requirements(interface, &mut Vec::new())
            .requirements
            .into_iter()
            .find(|requirement| requirement.name == name)
            .map(|requirement| requirement.method)?;
        self.resolve_interface_self_result(&mut method, interface);
        Some(method)
    }

    fn resolve_interface_self_result(
        &mut self,
        method: &mut MethodInfo,
        interface: &InterfaceType<TypeId>,
    ) {
        if matches!(
            self.types.kind(method.return_ty),
            TypeKind::InterfaceSelf(_)
        ) {
            method.return_ty = self.types.intern(TypeKind::Interface(interface.clone()));
        }
    }

    pub(super) fn constrained_requirement(
        &mut self,
        parameter: &str,
        name: &str,
        span: Span,
    ) -> Result<Option<CanonicalRequirement>, Vec<Span>> {
        let constraints = self
            .type_parameter_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(parameter))
            .cloned()
            .unwrap_or_default();
        let mut candidates = Vec::new();
        for mut constraint in constraints {
            if self.is_core_interface(&constraint.name)
                && matches!(constraint.name.as_str(), "Comparable" | "Equatable")
                && constraint.arguments.is_empty()
            {
                constraint
                    .arguments
                    .push(crate::types::TypeArgumentRef::Type(TypeRef::named(
                        parameter,
                    )));
            }
            if self.interface_declaration(&constraint.name).is_none() {
                continue;
            }
            let Some(interface) = self.resolve_interface_edge(&constraint, span) else {
                continue;
            };
            if let Some(requirement) = self
                .canonical_interface_requirements(&interface, &mut Vec::new())
                .requirements
                .into_iter()
                .find(|requirement| requirement.name == name)
            {
                candidates.push(requirement);
            }
        }
        if candidates.is_empty() {
            return Ok(None);
        }
        // Intersections do not select a winner by constraint order. A callable
        // must satisfy every contract that guarantees this method.
        let mut selected = if let Some(candidate) = candidates.iter().find(|candidate| {
            candidates.iter().all(|other| {
                self.method_contract_failures(&candidate.method, &other.method)
                    .is_empty()
            })
        }) {
            candidate.clone()
        } else {
            return Err(candidates
                .iter()
                .flat_map(|requirement| requirement.origins.iter().map(|(_, span)| *span))
                .collect());
        };
        selected.origins.clear();
        for candidate in candidates {
            for origin in candidate.origins {
                if !selected.origins.contains(&origin) {
                    selected.origins.push(origin);
                }
            }
        }
        if matches!(
            self.types.kind(selected.method.return_ty),
            TypeKind::InterfaceSelf(_)
        ) {
            selected.method.return_ty = self
                .types
                .intern(TypeKind::TypeParameter(parameter.to_string()));
        }
        Ok(Some(selected))
    }

    pub(super) fn constrained_method_return(
        &mut self,
        parameter: &str,
        name: &str,
        span: Span,
    ) -> TypeId {
        self.constrained_requirement(parameter, name, span)
            .ok()
            .flatten()
            .map(|requirement| requirement.method.return_ty)
            .unwrap_or_else(|| self.types.unknown())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn check_interface_call(
        &mut self,
        interface: &InterfaceType<TypeId>,
        object: &Expr,
        method: &str,
        member_span: Span,
        args: &[Argument],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let Some(mut required) = self
            .canonical_interface_requirements(interface, &mut Vec::new())
            .requirements
            .into_iter()
            .find(|requirement| requirement.name == method)
        else {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0304",
                    format!(
                        "interface `{}` has no requirement `{method}`",
                        interface.name
                    ),
                    span,
                )
                .with_title("Unknown Interface Method"),
            );
            return;
        };
        self.resolve_interface_self_result(&mut required.method, interface);
        self.check_contract_call(
            required,
            object,
            method,
            member_span,
            args,
            span,
            scopes,
            method_context,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn check_contract_call(
        &mut self,
        required: CanonicalRequirement,
        object: &Expr,
        method: &str,
        member_span: Span,
        args: &[Argument],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        let receiver = self.infer_expr_type(object, scopes, method_context);
        if matches!(self.types.kind(receiver), TypeKind::TypeParameter(_)) {
            self.call_targets
                .entry(span)
                .or_insert_with(|| CallableTarget::ConstrainedMethod {
                    receiver: self.types.resolved(receiver),
                    method_name: method.to_string(),
                    requirement: required.method.declaration,
                    implementations: Vec::new(),
                });
        } else if let Some(interface) = self.interface_receiver(receiver) {
            let interface = self.resolved_interface(&interface);
            self.call_targets
                .entry(span)
                .or_insert_with(|| CallableTarget::InterfaceMethod {
                    interface,
                    method_name: method.to_string(),
                    requirement: required.method.declaration,
                });
        }
        if self.contract_type_depth == 0
            && required.origins.iter().any(|(interface, span)| {
                span.source == crate::compiler_known_contracts::SOURCE_ID
                    && crate::compiler_known_contracts::requires_core_execution(&interface.name)
            })
        {
            self.report_contract_boundary(
                PendingContractOperation::CoreValueOperation,
                member_span,
            );
        }
        self.record_contract_member_reference(
            member_span,
            required.origins.iter().map(|(_, span)| *span).collect(),
        );
        self.check_declared_contract_method(
            required.method,
            object,
            method,
            args,
            span,
            scopes,
            method_context,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn check_declared_contract_method(
        &mut self,
        declared: MethodInfo,
        object: &Expr,
        method: &str,
        args: &[Argument],
        span: Span,
        scopes: &ScopeStack,
        method_context: Option<&MethodContext>,
    ) {
        if declared.is_static {
            self.diagnostics.push(Diagnostic::new(
                "E0487",
                format!("static method `{method}` must be called with `::`"),
                span,
            ));
            return;
        }
        if declared.receiver_mode == Some(ReceiverMode::Writable)
            && !self.is_writable_object_path(object, scopes, method_context)
        {
            self.diagnostics.push(Diagnostic::new(
                "E0203",
                format!("cannot call writable method `{method}` through a readonly receiver"),
                span,
            ));
        }
        let method = self.instantiate_generic_method_call(
            &format!("declared method `{method}`"),
            &declared,
            args,
            span,
            scopes,
            method_context,
        );
        self.check_call_arguments(
            "declared method",
            &method.params,
            args,
            span,
            scopes,
            method_context,
        );
        let effects = if matches!(
            self.call_targets.get(&span),
            Some(CallableTarget::InterfaceMethod { .. })
        ) {
            self.complete_function_value_effects(&method.checked_effects, span)
        } else {
            method.checked_effects
        };
        self.record_checked_effects(effects, span);
    }

    pub(super) fn specialize_constrained_method(
        &mut self,
        span: Span,
        substitutions: &HashMap<String, TypeId>,
    ) -> Option<MethodInfo> {
        let CallableTarget::ConstrainedMethod {
            receiver,
            method_name,
            ..
        } = self.call_targets.get(&span)?.clone()
        else {
            return None;
        };
        let receiver = self.types.intern_resolved(&receiver);
        let receiver = self.substitute_type_id(receiver, substitutions);
        let TypeKind::Class(class_type) = self.types.kind(receiver).clone() else {
            return None;
        };
        if self.type_is_symbolic(receiver)
            || self.class_requires_trait_composition(&class_type.name)
        {
            return None;
        }
        let (declaring_class, method) = self.lookup_instance_method(&class_type, &method_name)?;
        let receiver = self.types.resolved(receiver);
        let declaring_type = self.types.intern(TypeKind::Class(declaring_class));
        let ResolvedType::Class(declaring_class) = self.types.resolved(declaring_type) else {
            unreachable!()
        };
        let implementation = ConstrainedMethodImplementation {
            receiver,
            declaring_class,
            declaration: method.declaration,
        };
        if let Some(CallableTarget::ConstrainedMethod {
            implementations, ..
        }) = self.call_targets.get_mut(&span)
        {
            if !implementations.contains(&implementation) {
                implementations.push(implementation);
            }
        }
        Some(method)
    }

    pub(super) fn reject_primitive_interface_erasure(
        &mut self,
        target: TypeId,
        value: TypeId,
        span: Span,
    ) -> bool {
        let target = match self.types.kind(target) {
            TypeKind::Nullable(inner) => *inner,
            _ => target,
        };
        if !matches!(self.types.kind(target), TypeKind::Interface(_)) {
            return false;
        }
        let value = match self.types.kind(value) {
            TypeKind::Nullable(inner) => *inner,
            _ => value,
        };
        if !matches!(
            self.types.kind(value),
            TypeKind::Integer(_) | TypeKind::Float(_) | TypeKind::Bool | TypeKind::String
        ) {
            return false;
        }
        self.diagnostics.push(
            Diagnostic::new(
                "E0760",
                format!(
                    "primitive `{}` cannot inhabit an interface-typed slot",
                    self.types.display(value)
                ),
                span,
            )
            .with_title("Primitive Interface Erasure Is Not Allowed")
            .with_help("use a generic constraint for an unboxed primitive value"),
        );
        true
    }

    pub(super) fn interface_declaration(&self, name: &str) -> Option<&InterfaceDecl> {
        self.authored_interface_declaration(name).or_else(|| {
            if self.program.items.iter().any(|item| match item {
                Item::Class(declaration) => declaration.name == name,
                Item::Enum(declaration) => declaration.name == name,
                Item::Trait(declaration) => declaration.name == name,
                _ => false,
            }) {
                return None;
            }
            crate::compiler_known_contracts::interfaces()
                .find(|declaration| declaration.name == name)
        })
    }

    pub(super) fn authored_interface_declaration(&self, name: &str) -> Option<&InterfaceDecl> {
        self.program.items.iter().find_map(|item| match item {
            Item::Interface(declaration) if declaration.name == name => Some(declaration),
            _ => None,
        })
    }

    pub(super) fn is_core_interface(&self, name: &str) -> bool {
        self.interface_declaration(name).is_some_and(|declaration| {
            declaration.span.source == crate::compiler_known_contracts::SOURCE_ID
        })
    }

    pub(super) fn has_core_contract(&mut self, ty: TypeId, name: &str) -> bool {
        if !self.is_core_interface(name) {
            return false;
        }
        let ty = match self.types.kind(ty) {
            TypeKind::Nullable(inner) => *inner,
            _ => ty,
        };
        let arguments = if matches!(name, "Comparable" | "Equatable") {
            vec![ty]
        } else {
            Vec::new()
        };
        self.type_has_declared_interface(
            ty,
            &InterfaceType::new(name, arguments),
            &mut HashSet::new(),
        )
    }

    pub(super) fn public_iterable_element(&mut self, ty: TypeId) -> Option<TypeId> {
        if !self.is_core_interface("Iterable") {
            return None;
        }
        let interfaces = match self.types.kind(ty).clone() {
            TypeKind::Class(class) => self
                .class_interface_closure(&class, &mut HashSet::new())
                .entries
                .into_iter()
                .map(|(interface, _)| interface)
                .collect::<Vec<_>>(),
            TypeKind::Interface(interface) => vec![interface],
            TypeKind::TypeParameter(parameter) => {
                let constraints = self
                    .type_parameter_scopes
                    .iter()
                    .rev()
                    .find_map(|scope| scope.get(&parameter))
                    .cloned()
                    .unwrap_or_default();
                constraints
                    .iter()
                    .filter_map(|constraint| {
                        self.resolve_interface_edge(constraint, Span::new(0, 0))
                    })
                    .collect()
            }
            _ => return None,
        };
        for interface in interfaces {
            let mut contracts = self
                .canonical_interface_requirements(&interface, &mut Vec::new())
                .ancestors;
            contracts.push(interface);
            if let Some(iterable) = contracts
                .iter()
                .find(|interface| interface.name == "Iterable")
            {
                return iterable.arguments.first().copied();
            }
        }
        None
    }

    pub(super) fn gate_core_operation(
        &mut self,
        ty: TypeId,
        contract: &'static str,
        span: Span,
    ) -> bool {
        if self.type_is_symbolic(ty) {
            let template = (span, ty, contract);
            if !self.pending_core_operations.contains(&template) {
                self.pending_core_operations.push(template);
            }
            return false;
        }
        if self.has_core_contract(ty, contract) {
            self.report_contract_boundary(PendingContractOperation::CoreValueOperation, span);
            true
        } else {
            false
        }
    }

    pub(super) fn check_specialized_core_operations(
        &mut self,
        declaration: Span,
        substitutions: &HashMap<String, TypeId>,
    ) {
        for (span, ty, contract) in self.pending_core_operations.clone() {
            if span.source == declaration.source
                && span.start >= declaration.start
                && span.end <= declaration.end
            {
                let ty = self.substitute_type_id(ty, substitutions);
                if !self.type_is_symbolic(ty) {
                    self.gate_core_operation(ty, contract, span);
                }
            }
        }
    }

    pub(super) fn trait_declaration(&self, name: &str) -> Option<&crate::ast::TraitDecl> {
        self.program.items.iter().find_map(|item| match item {
            Item::Trait(declaration) if declaration.name == name => Some(declaration),
            _ => None,
        })
    }

    pub(super) fn record_contract_member_reference(&mut self, span: Span, origins: Vec<Span>) {
        if !self
            .contracts
            .member_references
            .iter()
            .any(|reference| reference.span == span && reference.origins == origins)
        {
            self.contracts
                .member_references
                .push(ContractMemberReference { span, origins });
        }
    }

    fn with_duplicate_type_removal(
        &self,
        diagnostic: Diagnostic,
        span: Span,
        entries: &[Span],
    ) -> Diagnostic {
        let Some(index) = entries
            .iter()
            .position(|entry| *entry == span)
            .and_then(|index| index.checked_sub(1))
        else {
            return diagnostic;
        };
        let separator = span.at(entries[index].end, span.start);
        if self.source_slice(separator).is_some_and(|source| {
            source
                .chars()
                .filter(|character| !character.is_ascii_whitespace())
                .eq([','])
        }) {
            diagnostic.with_fix(span.at(entries[index].end, span.end), "")
        } else {
            diagnostic
        }
    }

    pub(super) fn interface_declares_ancestor(
        &self,
        name: &str,
        ancestor: &str,
        visiting: &mut HashSet<String>,
    ) -> bool {
        if name == ancestor {
            return true;
        }
        if !visiting.insert(name.to_string()) {
            return false;
        }
        let found = self.interface_declaration(name).is_some_and(|declaration| {
            declaration
                .parents
                .iter()
                .any(|parent| self.interface_declares_ancestor(&parent.name, ancestor, visiting))
        });
        visiting.remove(name);
        found
    }

    pub(super) fn report_contract_boundary(
        &mut self,
        operation: PendingContractOperation,
        span: Span,
    ) {
        if !self
            .contracts
            .boundaries
            .iter()
            .any(|boundary| boundary.operation == operation && boundary.span == span)
        {
            self.contracts
                .boundaries
                .push(SupportBoundary { operation, span });
        }
        // Pending execution is not a semantic error. Publish its diagnostic only
        // after checking, so it cannot suppress effect or ownership facts.
    }

    pub(super) fn collect_interface_declarations(&mut self) {
        let declarations = crate::compiler_known_contracts::interfaces()
            .filter(|declaration| {
                self.interface_declaration(&declaration.name)
                    .is_some_and(|selected| selected.span == declaration.span)
            })
            .cloned()
            .chain(self.program.items.iter().filter_map(|item| match item {
                Item::Interface(declaration) => Some(declaration.clone()),
                _ => None,
            }))
            .collect::<Vec<_>>();
        for declaration in declarations {
            let before = self.diagnostics.len();
            if matches!(declaration.name.as_str(), "Displayable" | "Error") && self.program.items.iter().any(|item| matches!(item, Item::Interface(authored) if authored.span == declaration.span)) {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0309",
                        format!(
                            "`{}` is a compiler-known interface and cannot be redeclared",
                            declaration.name
                        ),
                        declaration.name_span,
                    )
                    .with_title("Compiler-Known Interface Cannot Be Redeclared"),
                );
            }
            self.check_type_parameter_declarations(
                &declaration.type_params,
                &format!("interface `{}`", declaration.name),
            );
            self.type_parameter_scopes
                .push(type_parameter_scope(&declaration.type_params));
            let mut parents = Vec::new();
            for (parent, span) in declaration
                .parents
                .iter()
                .zip(&declaration.syntax.inheritance_type_spans)
            {
                if let Some(parent) = self.resolve_interface_edge(parent, *span) {
                    parents.push((parent, *span));
                }
            }
            let mut requirements = Vec::new();
            let mut names = HashMap::new();
            for requirement in &declaration.requirements {
                if let Some(previous) =
                    names.insert(requirement.name.clone(), requirement.name_span)
                {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0753",
                            format!(
                                "requirement `{}` is declared more than once",
                                requirement.name
                            ),
                            requirement.name_span,
                        )
                        .with_title("Duplicate Interface Requirement")
                        .with_related(previous, "the previous requirement is here"),
                    );
                }
                let invalid = requirement.access != MemberAccess::External
                    || requirement.is_static
                    || requirement.is_open
                    || requirement.is_override
                    || LifecycleMethod::from_method_name(&requirement.name).is_some()
                    || requirement.body.as_block().is_some()
                    || requirement.return_type.is_none()
                    || requirement
                        .params
                        .iter()
                        .any(|parameter| parameter.default.is_some());
                if invalid {
                    self.diagnostics.push(Diagnostic::new("E0749", "an interface requirement must be an external instance method signature with an explicit return type, no defaults, and a terminating semicolon", requirement.span)
                        .with_title("Invalid Interface Requirement"));
                }
                if requirement
                    .params
                    .iter()
                    .any(|parameter| type_ref_contains_self(&parameter.ty))
                    || requirement.return_type.as_ref().is_some_and(|ty| {
                        type_ref_contains_self(ty)
                            && (ty.name != "self" || ty.nullable || !ty.arguments.is_empty())
                    })
                {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0749",
                            "interface `self` is permitted only as the exact owned return type",
                            requirement.span,
                        )
                        .with_title("Invalid Interface Self Type"),
                    );
                }
                let signature =
                    self.resolve_function_signature(requirement, Some(&declaration.name));
                self.function_signatures
                    .insert(requirement.span, signature.clone());
                requirements.push((
                    requirement.name.clone(),
                    MethodInfo {
                        declaration: requirement.span,
                        is_open: false,
                        is_override: false,
                        virtual_root: None,
                        access: MemberAccess::External,
                        receiver_mode: Some(if requirement.writable_this {
                            ReceiverMode::Writable
                        } else {
                            ReceiverMode::Readonly
                        }),
                        return_borrow: None,
                        is_static: false,
                        enclosing_type_bindings: HashMap::new(),
                        type_params: signature.type_params,
                        params: signature.params,
                        return_ty: signature.return_ty,
                        checked_effects: signature.checked_effects,
                    },
                ));
            }
            self.type_parameter_scopes.pop();
            let valid = self.diagnostics.len() == before;
            self.interface_definitions.insert(
                declaration.name.clone(),
                InterfaceDefinition {
                    declaration,
                    parents,
                    local_requirements: requirements,
                    valid,
                },
            );
        }
        let mut names = self
            .interface_definitions
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        names.sort_by_key(|name| self.interface_definitions[name].declaration.span);
        for name in names {
            let definition = self.interface_definitions[&name].clone();
            let arguments = definition
                .declaration
                .type_params
                .iter()
                .map(|parameter| {
                    self.types
                        .intern(TypeKind::TypeParameter(parameter.name.clone()))
                })
                .collect();
            let interface = InterfaceType::new(&name, arguments);
            let requirements = self.canonical_interface_requirements(&interface, &mut Vec::new());
            let parents = definition
                .declaration
                .parents
                .iter()
                .zip(&definition.declaration.syntax.inheritance_type_spans)
                .filter_map(|(ty, span)| {
                    let (specialization, _) = definition
                        .parents
                        .iter()
                        .find(|(_, parent_span)| parent_span == span)?;
                    self.interface_declaration(&ty.name)
                        .map(|parent| ContractEdge {
                            authored_type: ty.clone(),
                            specialization: self.resolved_interface(specialization),
                            span: *span,
                            declaration: parent.span,
                        })
                })
                .collect();
            self.contracts.interfaces.push(InterfaceFacts {
                name,
                declaration: definition.declaration.span,
                name_span: definition.declaration.name_span,
                type_parameters: definition.declaration.type_params,
                parents,
                requirements: requirements
                    .requirements
                    .iter()
                    .map(|requirement| self.requirement_facts(requirement))
                    .collect(),
                valid: requirements.valid,
            });
        }
    }

    pub(super) fn resolve_interface_edge(
        &mut self,
        ty: &TypeRef,
        span: Span,
    ) -> Option<InterfaceType<TypeId>> {
        if ty.nullable || ty.function.is_some() || self.interface_declaration(&ty.name).is_none() {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0750",
                    format!("`{ty}` must name an interface specialization"),
                    span,
                )
                .with_title("Interface Type Is Required"),
            );
            return None;
        }
        if matches!(ty.name.as_str(), "Displayable" | "Error")
            && self.authored_interface_declaration(&ty.name).is_none()
        {
            return self
                .expect_type_arg_count(ty, 0, span)
                .then(|| InterfaceType::new(&ty.name, Vec::new()));
        }
        let resolved = self.resolve_contract_type(ty, span);
        match self.types.kind(resolved) {
            TypeKind::Interface(interface) => Some(interface.clone()),
            _ => None,
        }
    }

    pub(super) fn resolve_contract_type(&mut self, ty: &TypeRef, span: Span) -> TypeId {
        self.contract_type_depth += 1;
        let resolved = self.resolve_type_ref_in_position(ty, span, TypePosition::Value, None);
        self.contract_type_depth -= 1;
        resolved
    }

    /// Nominality depends on authored edges, not member discovery order. Final
    /// method conformance is checked separately once every signature is known.
    pub(super) fn type_has_declared_interface(
        &mut self,
        ty: TypeId,
        target: &InterfaceType<TypeId>,
        visiting: &mut HashSet<String>,
    ) -> bool {
        if self.is_core_interface("Iterable") && target.name == "Iterable" {
            let element = match self.types.kind(ty) {
                TypeKind::TypedArray(element)
                | TypeKind::List(element)
                | TypeKind::Set(element)
                | TypeKind::SortedSet(element)
                | TypeKind::Deque(element)
                | TypeKind::PriorityQueue(element) => Some(*element),
                TypeKind::Dictionary(_, value) | TypeKind::SortedDictionary(_, value) => {
                    Some(*value)
                }
                _ => None,
            };
            if let Some(element) = element {
                return target.arguments.as_slice() == [element];
            }
        }
        let (name, arguments, parameters, edges) = match self.types.kind(ty).clone() {
            TypeKind::TypeParameter(parameter) => {
                let key = format!("parameter:{parameter}");
                if !visiting.insert(key.clone()) {
                    return false;
                }
                let constraints = self
                    .type_parameter_scopes
                    .iter()
                    .rev()
                    .find_map(|scope| scope.get(&parameter))
                    .cloned()
                    .unwrap_or_default();
                let found = constraints.iter().any(|constraint| {
                    let interface = if self.is_core_interface(&constraint.name)
                        && matches!(constraint.name.as_str(), "Comparable" | "Equatable")
                        && constraint.arguments.is_empty()
                    {
                        Some(InterfaceType::new(&constraint.name, vec![ty]))
                    } else {
                        self.resolve_interface_edge(
                            constraint,
                            self.interface_declaration(&constraint.name)
                                .map_or(Span::new(0, 0), |declaration| declaration.span),
                        )
                    };
                    interface.is_some_and(|interface| {
                        let resolved = self.types.intern(TypeKind::Interface(interface));
                        self.type_has_declared_interface(resolved, target, visiting)
                    })
                });
                visiting.remove(&key);
                return found;
            }
            TypeKind::Error => return target.name == "Error" && target.arguments.is_empty(),
            TypeKind::Interface(interface) => {
                if &interface == target {
                    return true;
                }
                let Some(declaration) = self.interface_declaration(&interface.name).cloned() else {
                    return false;
                };
                let edges = declaration
                    .parents
                    .into_iter()
                    .zip(declaration.syntax.inheritance_type_spans)
                    .collect::<Vec<_>>();
                (
                    interface.name,
                    interface.arguments,
                    declaration.type_params,
                    edges,
                )
            }
            TypeKind::Class(class) => {
                let Some(declaration) = self.program.items.iter().find_map(|item| match item {
                    Item::Class(declaration) if declaration.name == class.name => {
                        Some(declaration.clone())
                    }
                    _ => None,
                }) else {
                    return false;
                };
                let mut edges = declaration
                    .implements
                    .into_iter()
                    .zip(declaration.syntax.inheritance_type_spans)
                    .collect::<Vec<_>>();
                if let Some(parent) = declaration.parent {
                    edges.push((parent, declaration.parent_span.unwrap_or(declaration.span)));
                }
                (class.name, class.arguments, declaration.type_params, edges)
            }
            _ => return false,
        };
        if !visiting.insert(name.clone()) {
            return false;
        }
        let bindings = parameters
            .iter()
            .zip(arguments)
            .map(|(parameter, ty)| (parameter.name.clone(), ty))
            .collect::<HashMap<_, _>>();
        self.type_parameter_scopes
            .push(type_parameter_scope(&parameters));
        let found = edges.iter().any(|(edge, span)| {
            if self.interface_declaration(&edge.name).is_none()
                && !self.classes.contains_key(&edge.name)
            {
                return false;
            }
            let resolved = if self.interface_declaration(&edge.name).is_some() {
                let Some(interface) = self.resolve_interface_edge(edge, *span) else {
                    return false;
                };
                self.types.intern(TypeKind::Interface(interface))
            } else {
                self.resolve_contract_type(edge, *span)
            };
            let resolved = self.substitute_type_id(resolved, &bindings);
            self.type_has_declared_interface(resolved, target, visiting)
        });
        self.type_parameter_scopes.pop();
        visiting.remove(&name);
        found
    }

    fn canonical_interface_requirements(
        &mut self,
        interface: &InterfaceType<TypeId>,
        visiting: &mut Vec<(String, Span)>,
    ) -> InterfaceRequirements {
        if let Some(cached) = self.interface_requirements.get(interface) {
            return cached.clone();
        }
        let Some(definition) = self.interface_definitions.get(&interface.name).cloned() else {
            return InterfaceRequirements::default();
        };
        if let Some(cycle_start) = visiting
            .iter()
            .position(|(name, _)| name == &interface.name)
        {
            let mut diagnostic = Diagnostic::new(
                "E0751",
                format!("interface inheritance cycle reaches `{}`", interface.name),
                visiting
                    .last()
                    .map_or(definition.declaration.span, |(_, span)| *span),
            )
            .with_title("Interface Inheritance Cycle");
            for (_, span) in &visiting[cycle_start..] {
                diagnostic = diagnostic
                    .with_related(*span, "this inheritance edge participates in the cycle");
            }
            self.diagnostics.push(diagnostic);
            return InterfaceRequirements::default();
        }
        let substitutions = definition
            .declaration
            .type_params
            .iter()
            .zip(&interface.arguments)
            .map(|(parameter, ty)| (parameter.name.clone(), *ty))
            .collect::<HashMap<_, _>>();
        let mut result = InterfaceRequirements {
            valid: definition.valid,
            ..Default::default()
        };
        let mut direct = HashMap::new();
        let mut inherited = Vec::<CanonicalRequirement>::new();
        for (parent, span) in &definition.parents {
            let parent = InterfaceType::new(
                &parent.name,
                parent
                    .arguments
                    .iter()
                    .map(|ty| self.substitute_type_id(*ty, &substitutions))
                    .collect(),
            );
            if let Some(previous) = direct.insert(parent.clone(), *span) {
                let diagnostic = Diagnostic::new(
                    "E0752",
                    format!(
                        "interface `{}` repeats parent `{}`",
                        interface.name, parent.name
                    ),
                    *span,
                )
                .with_title("Repeated Interface Parent")
                .with_related(previous, "the first parent occurs here");
                self.diagnostics.push(self.with_duplicate_type_removal(
                    diagnostic,
                    *span,
                    &definition.declaration.syntax.inheritance_type_spans,
                ));
                result.valid = false;
                continue;
            }
            visiting.push((interface.name.clone(), *span));
            let requirements = self.canonical_interface_requirements(&parent, visiting);
            visiting.pop();
            result.valid &= requirements.valid;
            for ancestor in std::iter::once(parent).chain(requirements.ancestors) {
                if let Some(previous) = result
                    .ancestors
                    .iter()
                    .find(|previous| previous.name == ancestor.name)
                {
                    if previous != &ancestor {
                        self.diagnostics.push(Diagnostic::new("E0752", format!("different specializations of interface `{}` are inherited together", ancestor.name), *span).with_title("Conflicting Interface Specializations"));
                        result.valid = false;
                    }
                } else {
                    result.ancestors.push(ancestor);
                }
            }
            for requirement in requirements.requirements {
                if !inherited
                    .iter()
                    .any(|previous| previous.origins == requirement.origins)
                {
                    inherited.push(requirement);
                }
            }
        }
        let mut locals = definition
            .local_requirements
            .iter()
            .map(|(name, method)| CanonicalRequirement {
                name: name.clone(),
                origins: vec![(interface.clone(), method.declaration)],
                method: self.substitute_contract_method(method, &substitutions),
            })
            .collect::<Vec<_>>();
        let mut names = HashSet::new();
        for requirement in &inherited {
            if !names.insert(requirement.name.clone()) {
                continue;
            }
            let same_name = inherited
                .iter()
                .filter(|other| other.name == requirement.name)
                .collect::<Vec<_>>();
            if let Some(local) = locals
                .iter_mut()
                .find(|local| local.name == requirement.name)
            {
                for parent in &same_name {
                    let failures = self.method_contract_failures(&local.method, &parent.method);
                    if !failures.is_empty() {
                        self.report_requirement_mismatch(
                            &local.name,
                            &local.method,
                            parent,
                            &failures,
                        );
                        result.valid = false;
                    }
                    for origin in &parent.origins {
                        if !local.origins.contains(origin) {
                            local.origins.push(origin.clone());
                        }
                    }
                }
                continue;
            }
            let mut merged = requirement.clone();
            for other in same_name.into_iter().skip(1) {
                if !self
                    .method_contract_failures(&merged.method, &other.method)
                    .is_empty()
                    || !self
                        .method_contract_failures(&other.method, &merged.method)
                        .is_empty()
                {
                    self.diagnostics.push(Diagnostic::new("E0753", format!("inherited contracts for `{}` differ; redeclare a contract substitutable for every parent", requirement.name), definition.declaration.name_span)
                        .with_title("Conflicting Interface Requirements").with_related(merged.method.declaration, "one inherited requirement is here").with_related(other.method.declaration, "another inherited requirement is here"));
                    result.valid = false;
                }
                for origin in &other.origins {
                    if !merged.origins.contains(origin) {
                        merged.origins.push(origin.clone());
                    }
                }
            }
            result.requirements.push(merged);
        }
        result.requirements.extend(locals);
        self.interface_requirements
            .insert(interface.clone(), result.clone());
        result
    }

    pub(super) fn substitute_contract_method(
        &mut self,
        method: &MethodInfo,
        substitutions: &HashMap<String, TypeId>,
    ) -> MethodInfo {
        let mut method = method.clone();
        for binding in method.enclosing_type_bindings.values_mut() {
            *binding = self.substitute_type_id(*binding, substitutions);
        }
        method.enclosing_type_bindings.extend(substitutions.clone());
        for parameter in &mut method.params {
            parameter.ty = self.substitute_type_id(parameter.ty, substitutions);
        }
        method.return_ty = self.substitute_type_id(method.return_ty, substitutions);
        if method.declaration.source == crate::compiler_known_contracts::SOURCE_ID
            && crate::compiler_known_contracts::interfaces().any(|interface| {
                interface.name == "Iterator"
                    && interface.requirements.iter().any(|requirement| {
                        requirement.name == "current" && requirement.span == method.declaration
                    })
            })
        {
            method.return_borrow =
                self.type_is_move_type(method.return_ty)
                    .then_some(ReturnBorrow {
                        source: BorrowSource::Receiver,
                        writable: false,
                    });
        }
        method.checked_effects = method
            .checked_effects
            .iter()
            .map(|effect| self.substitute_type_id(*effect, substitutions))
            .collect();
        method
    }

    fn resolved_interface(&self, interface: &InterfaceType<TypeId>) -> InterfaceType<ResolvedType> {
        InterfaceType::new(
            &interface.name,
            interface
                .arguments
                .iter()
                .map(|ty| self.types.resolved(*ty))
                .collect(),
        )
    }

    fn requirement_origins(&self, requirement: &CanonicalRequirement) -> Vec<RequirementOrigin> {
        requirement
            .origins
            .iter()
            .map(|(interface, declaration)| RequirementOrigin {
                interface: self.resolved_interface(interface),
                declaration: *declaration,
            })
            .collect()
    }

    fn requirement_facts(&self, requirement: &CanonicalRequirement) -> RequirementFacts {
        let method = &requirement.method;
        let generic_parameters = self
            .interface_declaration(&requirement.origins[0].0.name)
            .and_then(|declaration| {
                declaration
                    .requirements
                    .iter()
                    .find(|method| method.span == requirement.origins[0].1)
            })
            .map(|method| method.type_params.clone())
            .unwrap_or_default();
        RequirementFacts {
            name: requirement.name.clone(),
            origins: self.requirement_origins(requirement),
            generic_parameters,
            signature: CallableSignatureSemanticInfo {
                generic_parameter_count: method.type_params.len(),
                parameters: method
                    .params
                    .iter()
                    .map(|parameter| CallableParameterSemanticInfo {
                        name: parameter.name.clone(),
                        r#type: self.types.resolved(parameter.ty),
                        take: parameter.take,
                        writable: parameter.writable,
                        has_default: false,
                    })
                    .collect(),
                return_type: self.types.resolved(method.return_ty),
            },
            writable_receiver: method.receiver_mode == Some(ReceiverMode::Writable),
            checked_effects: method
                .checked_effects
                .iter()
                .map(|ty| self.types.resolved(*ty))
                .collect(),
            return_borrow: method.return_borrow,
        }
    }

    fn report_requirement_mismatch(
        &mut self,
        name: &str,
        method: &MethodInfo,
        required: &CanonicalRequirement,
        failures: &[ContractMismatch],
    ) {
        let detail = failures
            .iter()
            .map(|failure| failure.description())
            .collect::<Vec<_>>()
            .join(", ");
        let mut diagnostic = Diagnostic::new(
            "E0755",
            format!("method `{name}` does not preserve the required {detail}"),
            method.declaration,
        )
        .with_title("Interface Contract Does Not Match");
        for (_, origin) in &required.origins {
            diagnostic = diagnostic.with_related(*origin, "the required contract is declared here");
        }
        self.diagnostics.push(diagnostic);
    }

    pub(super) fn check_nominal_conformances(&mut self) {
        loop {
            let invalid = self
                .contracts
                .traits
                .iter()
                .filter(|declaration| !declaration.valid)
                .map(|declaration| declaration.declaration)
                .collect::<HashSet<_>>();
            let mut changed = false;
            for declaration in &mut self.contracts.traits {
                if declaration.valid
                    && declaration
                        .uses
                        .iter()
                        .any(|edge| invalid.contains(&edge.declaration))
                {
                    declaration.valid = false;
                    changed = true;
                }
            }
            if !changed {
                for composition in &mut self.contracts.compositions {
                    composition.valid &= composition
                        .uses
                        .iter()
                        .all(|edge| !invalid.contains(&edge.declaration));
                }
                break;
            }
        }
        let declarations = self
            .program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Class(declaration) => Some(declaration.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        for declaration in declarations {
            let implementing_type = self.symbolic_class_type(&declaration.name);
            let Some(class) = self.class_type(implementing_type) else {
                continue;
            };
            let conformances = self.class_interface_closure(&class, &mut HashSet::new());
            let deferred = self.class_requires_trait_composition(&class.name);
            for (interface, origin) in conformances.entries {
                let legacy_diagnostic = matches!(interface.name.as_str(), "Displayable" | "Error")
                    && self.diagnostics.iter().any(|diagnostic| {
                        matches!(
                            diagnostic.code,
                            "E0463" | "E0613" | "E0614" | "E0615" | "E0616"
                        ) && diagnostic.span.source == declaration.span.source
                            && diagnostic.span.start >= declaration.span.start
                            && diagnostic.span.end <= declaration.span.end
                    });
                let requirements =
                    self.canonical_interface_requirements(&interface, &mut Vec::new());
                let mut fact = ConformanceFacts {
                    implementing_type: self.types.resolved(implementing_type),
                    interface: self.resolved_interface(&interface),
                    origin,
                    status: if !conformances.valid
                        || !requirements.valid
                        || !self.class_composition_is_valid(&class.name)
                    {
                        ConformanceStatus::Invalid
                    } else if deferred {
                        ConformanceStatus::DeferredComposition
                    } else if requirements.valid && !legacy_diagnostic {
                        ConformanceStatus::Checked
                    } else {
                        ConformanceStatus::Invalid
                    },
                    implementations: Vec::new(),
                };
                for mut requirement in requirements.requirements {
                    // Dependent interface `self` is instantiated by the concrete implementer,
                    // never by the lexical interface or the method's declaring ancestor.
                    let requires_exact_return = matches!(
                        self.types.kind(requirement.method.return_ty),
                        TypeKind::InterfaceSelf(_)
                    );
                    if requires_exact_return {
                        requirement.method.return_ty = implementing_type;
                    }
                    let method = self.find_concrete_contract_method(&class, &requirement.name);
                    let mut implementation = RequirementImplementation {
                        requirement_origins: self.requirement_origins(&requirement),
                        implementation: None,
                        failures: Vec::new(),
                        exact_dynamic_return: None,
                    };
                    if !deferred && requirements.valid {
                        if let Some(method) = method {
                            implementation.implementation = Some(method.declaration);
                            implementation.failures =
                                self.method_contract_failures(&method, &requirement.method);
                            if requires_exact_return {
                                let expected = self.types.resolved(implementing_type);
                                if method.return_borrow.is_none()
                                    && self.prove_exact_return(
                                        method.declaration,
                                        &expected,
                                        &method.enclosing_type_bindings,
                                        &mut HashSet::new(),
                                    )
                                {
                                    implementation.exact_dynamic_return = Some(expected);
                                } else {
                                    implementation
                                        .failures
                                        .push(ContractMismatch::ExactDynamicReturn);
                                }
                            }
                            if !implementation.failures.is_empty() {
                                fact.status = ConformanceStatus::Invalid;
                                if !legacy_diagnostic {
                                    self.report_requirement_mismatch(
                                        &requirement.name,
                                        &method,
                                        &requirement,
                                        &implementation.failures,
                                    );
                                }
                            }
                        } else {
                            fact.status = ConformanceStatus::Invalid;
                            let mut diagnostic = Diagnostic::new(
                                "E0754",
                                format!(
                                    "class `{}` does not implement required method `{}`",
                                    class.name, requirement.name
                                ),
                                declaration.name_span,
                            )
                            .with_title("Interface Implementation Is Missing");
                            for (_, origin) in &requirement.origins {
                                diagnostic = diagnostic
                                    .with_related(*origin, "this requirement must be implemented");
                            }
                            if !legacy_diagnostic {
                                self.diagnostics.push(diagnostic);
                            }
                        }
                    }
                    fact.implementations.push(implementation);
                }
                self.contracts.conformances.push(fact);
            }
        }
    }

    pub(super) fn specialize_conformance_facts(&mut self, classes: &[ClassSemanticInfo]) {
        let templates = self.contracts.conformances.clone();
        for class in classes.iter().filter(|class| !class.arguments.is_empty()) {
            let Some(declaration) = self.program.items.iter().find_map(|item| match item {
                Item::Class(declaration) if declaration.name == class.declaration_name => {
                    Some(declaration)
                }
                _ => None,
            }) else {
                continue;
            };
            let substitutions = declaration
                .type_params
                .iter()
                .zip(&class.arguments)
                .map(|(parameter, argument)| {
                    (parameter.name.clone(), self.types.intern_resolved(argument))
                })
                .collect::<HashMap<_, _>>();
            for template in &templates {
                if !matches!(&template.implementing_type, ResolvedType::Class(implementer) if implementer.name == class.declaration_name)
                {
                    continue;
                }
                let mut fact = template.clone();
                let ty = self.types.intern_resolved(&fact.implementing_type);
                let ty = self.substitute_type_id(ty, &substitutions);
                fact.implementing_type = self.types.resolved(ty);
                for implementation in &mut fact.implementations {
                    if let Some(exact) = &mut implementation.exact_dynamic_return {
                        let ty = self.types.intern_resolved(exact);
                        let ty = self.substitute_type_id(ty, &substitutions);
                        *exact = self.types.resolved(ty);
                    }
                }
                for interface in std::iter::once(&mut fact.interface).chain(
                    fact.implementations.iter_mut().flat_map(|implementation| {
                        implementation
                            .requirement_origins
                            .iter_mut()
                            .map(|origin| &mut origin.interface)
                    }),
                ) {
                    for argument in &mut interface.arguments {
                        let ty = self.types.intern_resolved(argument);
                        let ty = self.substitute_type_id(ty, &substitutions);
                        *argument = self.types.resolved(ty);
                    }
                }
                if !self.contracts.conformances.contains(&fact) {
                    self.contracts.conformances.push(fact);
                }
            }
        }
    }

    fn prove_exact_return(
        &mut self,
        declaration: Span,
        expected: &ResolvedType,
        bindings: &HashMap<String, TypeId>,
        visiting: &mut HashSet<Span>,
    ) -> bool {
        if !visiting.insert(declaration) {
            return false;
        }
        let function = self.program.items.iter().find_map(|item| match item {
            Item::Function(function) if function.span == declaration => Some(function),
            Item::Class(class) => class.members.iter().find_map(|member| match member {
                ClassMember::Method(method) if method.span == declaration => Some(method),
                _ => None,
            }),
            _ => None,
        });
        let analysis = function.and_then(|function| {
            crate::return_analysis::analyze_with_given(function, &self.given_preludes)
        });
        let proven = analysis.is_some_and(|analysis| {
            !analysis.fallthrough_reachable
                && analysis
                    .value_returns()
                    .all(|value| self.prove_exact_result(value, expected, bindings, visiting))
        });
        visiting.remove(&declaration);
        proven
    }

    fn exact_result_type(
        &mut self,
        span: Span,
        bindings: &HashMap<String, TypeId>,
    ) -> Option<ResolvedType> {
        let ty = self.expression_types.get(&span)?.clone();
        let ty = self.types.intern_resolved(&ty);
        let ty = self.substitute_type_id(ty, bindings);
        Some(self.types.resolved(ty))
    }

    fn prove_exact_result(
        &mut self,
        expr: &Expr,
        expected: &ResolvedType,
        bindings: &HashMap<String, TypeId>,
        visiting: &mut HashSet<Span>,
    ) -> bool {
        // A closed result type is exact regardless of how the value was produced.
        if self.exact_result_type(expr.span(), bindings).as_ref() == Some(expected)
            && matches!(expected, ResolvedType::Class(class) if self.classes.get(&class.name).is_some_and(|class| !class.is_open))
        {
            return true;
        }
        match expr {
            Expr::Grouped { expr, .. } => {
                self.prove_exact_result(expr, expected, bindings, visiting)
            }
            Expr::New { span, .. } => {
                self.exact_result_type(*span, bindings).as_ref() == Some(expected)
            }
            Expr::Variable { span, .. } => match self.flow_facts.get(span).cloned() {
                Some(crate::narrowing::Fact::Constructed { origins, .. }) => {
                    !origins.is_empty()
                        && origins.iter().all(|origin| {
                            self.exact_result_type(*origin, bindings).as_ref() == Some(expected)
                        })
                }
                _ => false,
            },
            Expr::Match { arms, .. } => {
                !arms.is_empty()
                    && arms.iter().all(|arm| {
                        self.prove_exact_result(&arm.value, expected, bindings, visiting)
                    })
            }
            Expr::When(expression) => expression.branches.iter().all(|branch| {
                let analysis = crate::return_analysis::analyze_block_with_given(
                    &branch.block,
                    branch.span,
                    &self.given_preludes,
                );
                !analysis.fallthrough_reachable
                    && analysis
                        .value_returns()
                        .all(|value| self.prove_exact_result(value, expected, bindings, visiting))
            }),
            Expr::FunctionCall { span, .. }
            | Expr::StaticCall { span, .. }
            | Expr::MethodCall { span, .. } => match self.call_targets.get(span).cloned() {
                Some(CallableTarget::Function { name }) => {
                    let Some(function) = self.functions.get(&name) else {
                        return false;
                    };
                    let declaration = function.declaration;
                    function.return_borrow.is_none()
                        && self.prove_exact_return(declaration, expected, bindings, visiting)
                }
                Some(CallableTarget::Method {
                    class_type,
                    method_name,
                    direct_parent,
                }) => {
                    let class = ClassType {
                        name: class_type.name,
                        arguments: class_type
                            .arguments
                            .iter()
                            .map(|ty| self.types.intern_resolved(ty))
                            .collect(),
                    };
                    let Some(method) = self.find_concrete_contract_method(&class, &method_name)
                    else {
                        return false;
                    };
                    let exact_dispatch = direct_parent
                        || method.virtual_root.is_none()
                        || self
                            .classes
                            .get(&class.name)
                            .is_some_and(|class| !class.is_open);
                    exact_dispatch
                        && method.return_borrow.is_none()
                        && self.prove_exact_return(
                            method.declaration,
                            expected,
                            &method.enclosing_type_bindings,
                            visiting,
                        )
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn find_concrete_contract_method(
        &mut self,
        class: &ClassType<TypeId>,
        name: &str,
    ) -> Option<MethodInfo> {
        let mut current = Some(class.clone());
        let mut visited = HashSet::new();
        while let Some(class) = current {
            if !visited.insert(class.name.clone()) {
                return None;
            }
            if let Some(method) = self
                .classes
                .get(&class.name)
                .and_then(|class| class.methods.get(name))
                .cloned()
            {
                return Some(self.specialize_method_for_class(&method, &class));
            }
            current = self.specialized_parent_type(&class);
        }
        None
    }

    fn class_interface_closure(
        &mut self,
        class: &ClassType<TypeId>,
        visiting: &mut HashSet<String>,
    ) -> ClassConformances {
        if let Some(cached) = self.class_conformance_cache.get(class) {
            return cached.clone();
        }
        if !visiting.insert(class.name.clone()) {
            return ClassConformances::default();
        }
        let mut result = self
            .specialized_parent_type(class)
            .map(|parent| self.class_interface_closure(&parent, visiting))
            .unwrap_or(ClassConformances {
                entries: Vec::new(),
                valid: true,
            });
        let declaration = self.program.items.iter().find_map(|item| match item {
            Item::Class(declaration) if declaration.name == class.name => Some(declaration.clone()),
            _ => None,
        });
        if let Some(declaration) = declaration {
            let substitutions = self.class_type_substitutions(class);
            self.type_parameter_scopes
                .push(type_parameter_scope(&declaration.type_params));
            for (index, ty) in declaration.implements.iter().enumerate() {
                let span = declaration
                    .syntax
                    .inheritance_type_spans
                    .get(index)
                    .copied()
                    .unwrap_or(declaration.span);
                let Some(mut interface) = self.resolve_interface_edge(ty, span) else {
                    result.valid = false;
                    continue;
                };
                interface.arguments = interface
                    .arguments
                    .iter()
                    .map(|ty| self.substitute_type_id(*ty, &substitutions))
                    .collect();
                let requirements =
                    self.canonical_interface_requirements(&interface, &mut Vec::new());
                result.valid &= requirements.valid;
                let ancestors = requirements.ancestors;
                if let Some((previous, previous_span)) = result
                    .entries
                    .iter()
                    .find(|(previous, _)| previous.name == interface.name)
                {
                    let description = if previous == &interface {
                        "repeats an existing conformance"
                    } else {
                        "conflicts with an existing specialization"
                    };
                    result.valid = false;
                    if !self
                        .diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.code == "E0752" && diagnostic.span == span)
                    {
                        self.diagnostics.push(
                            Diagnostic::new("E0752", format!("`{ty}` {description}"), span)
                                .with_title("Redundant Or Conflicting Conformance")
                                .with_related(
                                    *previous_span,
                                    "the existing conformance originates here",
                                ),
                        );
                    }
                    continue;
                }
                result.entries.push((interface, span));
                for ancestor in ancestors {
                    if let Some((previous, previous_span)) = result
                        .entries
                        .iter()
                        .find(|(previous, _)| previous.name == ancestor.name)
                    {
                        if previous != &ancestor {
                            result.valid = false;
                            self.diagnostics.push(Diagnostic::new("E0752", format!("different specializations of `{}` cannot be implemented together", ancestor.name), span).with_title("Conflicting Interface Specializations").with_related(*previous_span, "the other specialization originates here"));
                        }
                    } else {
                        result.entries.push((ancestor, span));
                    }
                }
            }
            self.type_parameter_scopes.pop();
        }
        visiting.remove(&class.name);
        self.class_conformance_cache
            .insert(class.clone(), result.clone());
        result
    }

    pub(super) fn class_requires_trait_composition(&self, class: &str) -> bool {
        let mut current = Some(class);
        let mut visited = HashSet::new();
        while let Some(name) = current {
            if !visited.insert(name) {
                break;
            }
            if self.program.items.iter().any(|item| matches!(item, Item::Class(declaration) if declaration.name == name && declaration.members.iter().any(|member| matches!(member, ClassMember::Uses(_))))) { return true; }
            current = self
                .classes
                .get(name)
                .and_then(|class| class.parent.as_ref())
                .map(|parent| parent.name.as_str());
        }
        false
    }

    fn class_composition_is_valid(&self, class: &str) -> bool {
        let mut current = Some(class);
        let mut visited = HashSet::new();
        while let Some(name) = current {
            if !visited.insert(name) {
                return false;
            }
            if self
                .contracts
                .compositions
                .iter()
                .any(|composition| composition.class == name && !composition.valid)
            {
                return false;
            }
            current = self
                .classes
                .get(name)
                .and_then(|class| class.parent.as_ref())
                .map(|parent| parent.name.as_str());
        }
        true
    }

    pub(super) fn collect_trait_declarations(&mut self) {
        let declarations = self
            .program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Trait(declaration) => Some(declaration.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut graph = HashMap::new();
        for declaration in declarations {
            let before = self.diagnostics.len();
            self.check_adaptation_namespace(&declaration.members);
            self.check_type_parameter_declarations(
                &declaration.type_params,
                &format!("trait `{}`", declaration.name),
            );
            self.type_parameter_scopes
                .push(type_parameter_scope(&declaration.type_params));
            let mut edges = Vec::new();
            let mut members = HashMap::new();
            for member in &declaration.members {
                let identity = match member {
                    ClassMember::Uses(composition) => {
                        edges.extend(self.check_trait_use(composition));
                        None
                    }
                    ClassMember::Method(method) => {
                        if method.is_open
                            || method.is_override
                            || LifecycleMethod::from_method_name(&method.name).is_some()
                        {
                            self.diagnostics.push(Diagnostic::new("E0756", "traits cannot declare lifecycle methods, `open`, or `override`", method.span).with_title("Trait Hierarchy Member Is Not Allowed"));
                        }
                        if method.body.as_block().is_none()
                            && (method.return_type.is_none()
                                || method
                                    .params
                                    .iter()
                                    .any(|parameter| parameter.default.is_some()))
                        {
                            self.diagnostics.push(Diagnostic::new("E0749", "trait requirements need an explicit return type and cannot declare defaults", method.span).with_title("Invalid Trait Requirement"));
                        }
                        let signature =
                            self.resolve_function_signature(method, Some(&declaration.name));
                        self.function_signatures.insert(method.span, signature);
                        for parameter in &method.params {
                            if let Some(default) = &parameter.default {
                                self.check_trait_expression(default);
                            }
                        }
                        if let Some(body) = method.body.as_block() {
                            let mut forbidden = Vec::new();
                            crate::ast::visit::block(body, &mut |expression| {
                                collect_trait_parent_uses(expression, &mut forbidden)
                            });
                            self.report_trait_parent_uses(forbidden);
                        }
                        Some((&method.name, method.name_span))
                    }
                    ClassMember::Property(property) => {
                        self.check_trait_type(&property.ty, property.span, &declaration.name);
                        if let Some(initializer) = &property.initializer {
                            self.check_trait_expression(initializer);
                        }
                        Some((&property.name, property.span))
                    }
                    ClassMember::Constant(constant) => {
                        if let Some(ty) = &constant.ty {
                            self.check_trait_type(ty, constant.span, &declaration.name);
                        }
                        self.check_trait_expression(&constant.initializer);
                        Some((&constant.name, constant.span))
                    }
                };
                if let Some((name, span)) = identity {
                    if let Some(previous) = members.insert(name.clone(), span) {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0309",
                                format!("trait member `{name}` is declared more than once"),
                                span,
                            )
                            .with_title("Trait Member Name Is Already Declared")
                            .with_related(previous, "the previous member is here"),
                        );
                    }
                }
            }
            self.type_parameter_scopes.pop();
            graph.insert(declaration.name.clone(), edges.clone());
            self.contracts.traits.push(TraitFacts {
                name: declaration.name,
                declaration: declaration.span,
                name_span: declaration.name_span,
                type_parameters: declaration.type_params,
                uses: edges,
                valid: self.diagnostics.len() == before,
            });
        }
        let mut completed = HashSet::new();
        let mut cyclic = self
            .contracts
            .traits
            .iter()
            .filter(|declaration| !declaration.valid)
            .map(|declaration| declaration.name.clone())
            .collect::<HashSet<_>>();
        for declaration in &self.contracts.traits {
            check_trait_cycles(
                &declaration.name,
                &graph,
                &mut Vec::new(),
                &mut completed,
                &mut cyclic,
                &mut self.diagnostics,
            );
        }
        for declaration in &mut self.contracts.traits {
            declaration.valid &= !cyclic.contains(&declaration.name);
        }
        let classes = self
            .program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Class(declaration) => Some(declaration.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        for class in classes {
            let namespace_valid = self.check_adaptation_namespace(&class.members);
            self.type_parameter_scopes
                .push(type_parameter_scope(&class.type_params));
            for member in &class.members {
                if let ClassMember::Uses(composition) = member {
                    let before = self.diagnostics.len();
                    let edges = self.check_trait_use(composition);
                    let valid_dependencies = edges.iter().all(|edge| {
                        self.contracts.traits.iter().any(|declaration| {
                            declaration.declaration == edge.declaration && declaration.valid
                        })
                    });
                    let valid =
                        namespace_valid && self.diagnostics.len() == before && valid_dependencies;
                    self.contracts.compositions.push(CompositionFacts {
                        class: class.name.clone(),
                        span: composition.span,
                        uses: edges,
                        valid,
                    });
                    if valid {
                        self.report_contract_boundary(
                            PendingContractOperation::TraitComposition,
                            composition.span,
                        );
                    }
                }
            }
            self.type_parameter_scopes.pop();
        }
    }

    fn check_trait_type(&mut self, ty: &TypeRef, span: Span, owner: &str) {
        self.resolve_type_ref_in_position(ty, span, TypePosition::Value, Some(owner));
    }

    pub(super) fn check_trait_bodies(&mut self, declaration: &crate::ast::TraitDecl) {
        let before = self.diagnostics.len();
        self.type_parameter_scopes
            .push(type_parameter_scope(&declaration.type_params));
        self.contract_type_depth += 1;
        for member in &declaration.members {
            if let ClassMember::Property(property) = member {
                self.check_property_initializer(&declaration.name, property);
            }
            if let ClassMember::Method(method) = member {
                self.check_function(
                    method,
                    Some(MethodContext {
                        class_name: declaration.name.clone(),
                        receiver_access: if method.is_static {
                            ReceiverAccess::Unavailable
                        } else if method.writable_this {
                            ReceiverAccess::Writable
                        } else {
                            ReceiverAccess::Readonly
                        },
                    }),
                );
            }
        }
        self.contract_type_depth -= 1;
        self.type_parameter_scopes.pop();
        if self.diagnostics.len() != before {
            if let Some(facts) = self
                .contracts
                .traits
                .iter_mut()
                .find(|facts| facts.declaration == declaration.span)
            {
                facts.valid = false;
            }
        }
    }

    pub(super) fn trait_property(&mut self, owner: &str, name: &str) -> Option<PropertyInfo> {
        let property =
            self.trait_declaration(owner)?
                .members
                .iter()
                .find_map(|member| match member {
                    ClassMember::Property(property)
                        if property.name == name && !property.is_static =>
                    {
                        Some(property.clone())
                    }
                    _ => None,
                })?;
        let ty = self.resolve_type_ref_with_class(&property.ty, property.span, Some(owner));
        Some(PropertyInfo {
            access: property.access,
            writable: property.writable,
            ty,
            init_state: if property.initializer.is_some() {
                PropertyInitState::HasInitializer
            } else {
                PropertyInitState::Uninitialized
            },
            declaration_span: property.span,
        })
    }

    pub(super) fn trait_method(&self, owner: &str, name: &str) -> Option<MethodInfo> {
        let method =
            self.trait_declaration(owner)?
                .members
                .iter()
                .find_map(|member| match member {
                    ClassMember::Method(method) if method.name == name => Some(method),
                    _ => None,
                })?;
        let signature = self.function_signatures.get(&method.span)?;
        Some(MethodInfo {
            declaration: method.span,
            access: method.access,
            is_static: method.is_static,
            receiver_mode: (!method.is_static).then_some(if method.writable_this {
                ReceiverMode::Writable
            } else {
                ReceiverMode::Readonly
            }),
            return_borrow: signature.return_borrow,
            is_open: false,
            is_override: false,
            virtual_root: None,
            enclosing_type_bindings: HashMap::new(),
            type_params: signature.type_params.clone(),
            params: signature.params.clone(),
            return_ty: signature.return_ty,
            checked_effects: signature.checked_effects.clone(),
        })
    }

    fn check_adaptation_namespace(&mut self, members: &[ClassMember]) -> bool {
        let before = self.diagnostics.len();
        let mut names = members
            .iter()
            .filter_map(|member| match member {
                ClassMember::Method(method) => Some((method.name.as_str(), method.name_span)),
                ClassMember::Property(property) => Some((property.name.as_str(), property.span)),
                ClassMember::Constant(constant) => Some((constant.name.as_str(), constant.span)),
                ClassMember::Uses(_) => None,
            })
            .collect::<HashMap<_, _>>();
        for member in members {
            let ClassMember::Uses(composition) = member else {
                continue;
            };
            for adaptation in &composition.adaptations {
                let TraitAdaptationKind::Alias {
                    alias: Some(alias), ..
                } = &adaptation.kind
                else {
                    continue;
                };
                if let Some(previous) = names.insert(&alias.text, alias.span) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0757",
                            format!("trait alias `{}` collides with another member", alias.text),
                            alias.span,
                        )
                        .with_title("Trait Alias Name Is Already Declared")
                        .with_related(previous, "the other member is here"),
                    );
                }
            }
        }
        self.diagnostics.len() == before
    }

    fn check_trait_expression(&mut self, expression: &Expr) {
        let mut forbidden = Vec::new();
        crate::ast::visit::expr(expression, &mut |expression| {
            collect_trait_parent_uses(expression, &mut forbidden)
        });
        self.report_trait_parent_uses(forbidden);
    }

    fn report_trait_parent_uses(&mut self, spans: Vec<Span>) {
        for span in spans {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0756",
                    "traits cannot use `parent::`; their composer determines the class hierarchy",
                    span,
                )
                .with_title("Parent Access Is Not Allowed In Traits"),
            );
        }
    }

    fn check_trait_use(&mut self, composition: &TraitUse) -> Vec<ContractEdge> {
        let mut edges = Vec::new();
        for (ty, span) in composition.traits.iter().zip(&composition.type_spans) {
            let declaration = self.program.items.iter().find_map(|item| match item {
                Item::Trait(declaration) if declaration.name == ty.name => {
                    Some(declaration.clone())
                }
                _ => None,
            });
            let Some(declaration) = declaration.filter(|_| !ty.nullable && ty.function.is_none())
            else {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0750",
                        format!("`{ty}` must name a trait specialization"),
                        *span,
                    )
                    .with_title("Trait Type Is Required"),
                );
                continue;
            };
            if !self.expect_type_arg_count(ty, declaration.type_params.len(), *span) {
                continue;
            }
            let arguments = ty
                .type_arguments()
                .map(|argument| self.resolve_contract_type(argument, *span))
                .collect::<Vec<_>>();
            let parameters = declaration
                .type_params
                .iter()
                .map(|parameter| TypeParamInfo {
                    name: parameter.name.clone(),
                    constraints: parameter.constraints.clone(),
                })
                .collect::<Vec<_>>();
            self.check_class_type_constraints(&declaration.name, &parameters, &arguments, *span);
            edges.push(ContractEdge {
                authored_type: ty.clone(),
                specialization: crate::types::NominalType::new(
                    &ty.name,
                    arguments
                        .iter()
                        .map(|argument| self.types.resolved(*argument))
                        .collect(),
                ),
                span: *span,
                declaration: declaration.span,
            });
        }
        let mut aliases = HashMap::new();
        for adaptation in &composition.adaptations {
            let selected =
                self.resolve_trait_specialization(&adaptation.origin, adaptation.origin_span);
            if let TraitAdaptationKind::Alias {
                alias: Some(alias), ..
            } = &adaptation.kind
            {
                if let Some(previous) = aliases.insert(alias.text.clone(), alias.span) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0757",
                            format!("trait alias `{}` is declared more than once", alias.text),
                            alias.span,
                        )
                        .with_title("Duplicate Trait Alias")
                        .with_related(previous, "the previous alias is here"),
                    );
                }
                if alias.text == adaptation.method.text {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0757",
                            "an alias cannot duplicate the method's original name",
                            alias.span,
                        )
                        .with_title("Trait Alias Duplicates Its Original"),
                    );
                }
            }
            let mut origins = vec![(&adaptation.origin, adaptation.origin_span)];
            if let TraitAdaptationKind::InsteadOf {
                excluded,
                type_spans,
                ..
            } = &adaptation.kind
            {
                origins.extend(
                    excluded
                        .iter()
                        .zip(type_spans)
                        .map(|(ty, span)| (ty, *span)),
                );
            }
            for (index, (ty, span)) in origins.into_iter().enumerate() {
                let specialization = if index == 0 {
                    selected.clone()
                } else {
                    self.resolve_trait_specialization(ty, span)
                };
                if index > 0 && specialization == selected {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0757",
                            "a selected trait origin cannot exclude itself",
                            span,
                        )
                        .with_title("Trait Selection Excludes Itself"),
                    );
                }
                if !edges
                    .iter()
                    .any(|edge| edge.specialization == specialization)
                {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0757",
                            format!("adapted origin `{ty}` is not named by this `uses` entry"),
                            span,
                        )
                        .with_title("Trait Adaptation Origin Is Unavailable"),
                    );
                }
                let declaration = self.program.items.iter().find_map(|item| match item {
                    Item::Trait(declaration) if declaration.name == ty.name => Some(declaration),
                    _ => None,
                });
                if let Some(declaration) = declaration {
                    let member = declaration.members.iter().find(|member| match member {
                        ClassMember::Method(method) => method.name == adaptation.method.text,
                        ClassMember::Property(property) => property.name == adaptation.method.text,
                        ClassMember::Constant(constant) => constant.name == adaptation.method.text,
                        ClassMember::Uses(_) => false,
                    });
                    if index == 0 {
                        if let Some(ClassMember::Method(method)) = member {
                            self.record_contract_member_reference(
                                adaptation.method.span,
                                vec![method.span],
                            );
                        }
                    }
                    if member.is_some_and(|member| !matches!(member, ClassMember::Method(_)))
                        || (member.is_none()
                            && !declaration
                                .members
                                .iter()
                                .any(|member| matches!(member, ClassMember::Uses(_))))
                    {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0757",
                                format!(
                                    "`{ty}::{}` does not name an available trait method",
                                    adaptation.method.text
                                ),
                                adaptation.method.span,
                            )
                            .with_title("Trait Method Is Required"),
                        );
                    }
                }
            }
        }
        edges
    }

    fn resolve_trait_specialization(
        &mut self,
        ty: &TypeRef,
        span: Span,
    ) -> crate::types::NominalType<ResolvedType> {
        let arguments = ty
            .type_arguments()
            .map(|argument| {
                let argument = self.resolve_contract_type(argument, span);
                self.types.resolved(argument)
            })
            .collect();
        crate::types::NominalType::new(&ty.name, arguments)
    }

    pub(super) fn method_contract_failures(
        &mut self,
        method: &MethodInfo,
        required: &MethodInfo,
    ) -> Vec<ContractMismatch> {
        let mut failures = Vec::new();
        let generic_valid = method.type_params.len() == required.type_params.len()
            && self.normalized_method_constraints(method)
                == self.normalized_method_constraints(required);
        if !generic_valid {
            failures.push(ContractMismatch::GenericParameters);
        }
        let substitutions = method
            .type_params
            .iter()
            .zip(&required.type_params)
            .map(|(actual, required)| {
                (
                    actual.name.clone(),
                    self.types
                        .intern(TypeKind::TypeParameter(required.name.clone())),
                )
            })
            .collect::<HashMap<_, _>>();
        if method.params.len() != required.params.len() {
            failures.push(ContractMismatch::ParameterCount);
        }
        for (actual, expected) in method.params.iter().zip(&required.params) {
            if actual.name != expected.name {
                failures.push(ContractMismatch::ParameterNames);
            }
            if self.substitute_type_id(actual.ty, &substitutions) != expected.ty {
                failures.push(ContractMismatch::ParameterTypes);
            }
            if actual.take != expected.take || actual.writable != expected.writable {
                failures.push(ContractMismatch::ParameterOwnership);
            }
        }
        if matches!(
            (required.receiver_mode, method.receiver_mode),
            (Some(ReceiverMode::Readonly), Some(ReceiverMode::Writable))
        ) {
            failures.push(ContractMismatch::Receiver);
        }
        let actual_return = self.substitute_type_id(method.return_ty, &substitutions);
        if !self.override_return_is_compatible(required.return_ty, actual_return) {
            failures.push(ContractMismatch::ReturnType);
        }
        if !match (required.return_borrow, method.return_borrow) {
            (None, None) => true,
            (Some(expected), Some(actual)) => {
                expected.source == actual.source && (!expected.writable || actual.writable)
            }
            _ => false,
        } {
            failures.push(ContractMismatch::ReturnProvenance);
        }
        let actual_effects = method
            .checked_effects
            .iter()
            .map(|effect| self.substitute_type_id(*effect, &substitutions))
            .collect::<Vec<_>>();
        if !actual_effects.iter().all(|actual| {
            crate::checked_effects::is_automatic_effect(&self.types.resolved(*actual))
                || self.checked_error_type_covers(&required.checked_effects, *actual)
        }) {
            failures.push(ContractMismatch::CheckedEffects);
        }
        if method.access != MemberAccess::External {
            failures.push(ContractMismatch::Accessibility);
        }
        if method.is_static != required.is_static {
            failures.push(ContractMismatch::StaticMethod);
        }
        let mut seen = Vec::new();
        failures.retain(|failure| {
            if seen.contains(failure) {
                false
            } else {
                seen.push(*failure);
                true
            }
        });
        failures
    }

    fn normalized_method_constraints(
        &mut self,
        method: &MethodInfo,
    ) -> Vec<HashSet<(String, Vec<TypeId>)>> {
        let mut bindings = method.enclosing_type_bindings.clone();
        for (index, parameter) in method.type_params.iter().enumerate() {
            bindings.insert(
                parameter.name.clone(),
                self.types
                    .intern(TypeKind::TypeParameter(format!("#method{index}"))),
            );
        }
        self.type_parameter_scopes.push(
            bindings
                .keys()
                .map(|name| (name.clone(), Vec::new()))
                .collect(),
        );
        let result = method
            .type_params
            .iter()
            .map(|parameter| {
                parameter
                    .constraints
                    .iter()
                    .map(|constraint| {
                        let mut arguments = constraint
                            .type_arguments()
                            .map(|argument| {
                                let ty = self.resolve_contract_type(argument, method.declaration);
                                self.substitute_type_id(ty, &bindings)
                            })
                            .collect::<Vec<_>>();
                        if arguments.is_empty()
                            && self
                                .authored_interface_declaration(&constraint.name)
                                .is_none()
                            && matches!(constraint.name.as_str(), "Comparable" | "Equatable")
                        {
                            arguments.push(bindings[&parameter.name]);
                        }
                        (constraint.name.clone(), arguments)
                    })
                    .collect()
            })
            .collect();
        self.type_parameter_scopes.pop();
        result
    }
}

fn collect_trait_parent_uses(expression: &Expr, spans: &mut Vec<Span>) {
    if let Expr::StaticCall {
        qualifier: StaticQualifier::Parent,
        qualifier_span,
        ..
    }
    | Expr::StaticMember {
        qualifier: StaticQualifier::Parent,
        qualifier_span,
        ..
    } = expression
    {
        spans.push(*qualifier_span);
    }
}

fn check_trait_cycles(
    name: &str,
    graph: &HashMap<String, Vec<ContractEdge>>,
    visiting: &mut Vec<(String, Span)>,
    completed: &mut HashSet<String>,
    cyclic: &mut HashSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) -> bool {
    if let Some(start) = visiting.iter().position(|(ancestor, _)| ancestor == name) {
        let mut diagnostic = Diagnostic::new(
            "E0751",
            format!("trait composition cycle reaches `{name}`"),
            visiting.last().expect("cycle has an edge").1,
        )
        .with_title("Trait Composition Cycle");
        for (name, span) in &visiting[start..] {
            cyclic.insert(name.clone());
            diagnostic =
                diagnostic.with_related(*span, "this composition edge participates in the cycle");
        }
        diagnostics.push(diagnostic);
        return false;
    }
    if !completed.insert(name.to_string()) {
        return !cyclic.contains(name);
    }
    for edge in graph.get(name).into_iter().flatten() {
        visiting.push((name.to_string(), edge.span));
        if !check_trait_cycles(
            &edge.authored_type.name,
            graph,
            visiting,
            completed,
            cyclic,
            diagnostics,
        ) {
            cyclic.insert(name.to_string());
        }
        visiting.pop();
    }
    !cyclic.contains(name)
}

#[cfg(test)]
mod runtime_effect_tests {
    #[test]
    fn erased_requirement_call_keeps_automatic_io_transport() {
        let (_, analysis) = crate::analyze_source_for_ide(
            "effects.doria",
            r#"
interface Reader { function read(): int; }
function read(Reader $reader): int { return $reader->read(); }
"#,
        )
        .unwrap();
        let effects = analysis
            .info
            .checked_effect_sites
            .values()
            .flatten()
            .collect::<Vec<_>>();
        for name in [
            crate::compiler_known_io::IO_ERROR,
            crate::compiler_known_io::INVALID_UTF8_ERROR,
        ] {
            assert!(effects.iter().any(|effect| matches!(effect, crate::types::ResolvedType::Class(class) if class.name == name)), "{effects:?}");
        }
    }
}
