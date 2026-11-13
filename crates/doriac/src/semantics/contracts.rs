//! Declaration contracts, independent of runtime carriers and backend layout.

use super::*;
use crate::types::InterfaceType;

mod availability;
mod conformance;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContractFacts {
    pub interfaces: Vec<InterfaceFacts>,
    pub interface_specializations: Vec<InterfaceSpecializationFacts>,
    /// Scope-resolved member contracts for each constrained receiver expression.
    pub constrained_member_surfaces: HashMap<Span, ConstrainedMemberSurface>,
    /// Resolved authored clauses keyed by conformance or member declaration.
    /// These are predicates, not evidence that a member or witness is active.
    pub conditions: HashMap<Span, NominalConditionFacts>,
    /// Method/accessor declaration to its authored condition declaration.
    /// Accessors of one computed property share the property's condition.
    pub callable_conditions: HashMap<Span, Span>,
    pub traits: Vec<TraitFacts>,
    pub conformances: Vec<ConformanceFacts>,
    pub member_references: Vec<ContractMemberReference>,
    pub compositions: Vec<CompositionFacts>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NominalTypeParameter {
    pub name: String,
    pub declaration: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NominalObligation {
    /// Authored parameter identity survives composition and specialization.
    pub subject: NominalTypeParameter,
    pub subject_type: ResolvedType,
    pub subject_span: Span,
    pub interface: InterfaceType<ResolvedType>,
    pub interface_span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NominalConditionFacts {
    pub clause: Span,
    /// Lexical identities for every symbolic type in the obligations, including
    /// interface arguments. Method parameters cannot shadow owner parameters.
    pub parameters: Vec<NominalTypeParameter>,
    pub obligations: Vec<NominalObligation>,
}

impl NominalConditionFacts {
    pub(crate) fn substitute(
        &self,
        bindings: &HashMap<String, ResolvedType>,
        receiver: &ResolvedType,
        parameters: Vec<NominalTypeParameter>,
    ) -> Self {
        let mut condition = self.clone();
        condition.parameters = parameters;
        for obligation in &mut condition.obligations {
            obligation.subject_type =
                crate::types::substitute_trait_type(&obligation.subject_type, bindings, receiver);
            for argument in &mut obligation.interface.arguments {
                *argument = crate::types::substitute_trait_type(argument, bindings, receiver);
            }
        }
        condition
    }
}

impl ContractFacts {
    pub(crate) fn method_implementations<'a>(
        &'a self,
        interface: &'a InterfaceType<ResolvedType>,
        requirement: Span,
    ) -> impl Iterator<Item = (&'a ResolvedType, Option<Span>)> + 'a {
        self.conformances
            .iter()
            .filter(move |fact| {
                fact.interface == *interface
                    && fact.status == ConformanceStatus::Checked
                    && !crate::types::resolved_type_is_symbolic(&fact.implementing_type)
            })
            .map(move |fact| {
                let implementation = fact
                    .implementations
                    .iter()
                    .find(|implementation| {
                        implementation
                            .requirement_origins
                            .iter()
                            .any(|origin| origin.declaration == requirement)
                    })
                    .and_then(|implementation| implementation.implementation);
                (&fact.implementing_type, implementation)
            })
    }
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

/// The checked, substituted requirement graph used by ownership, retained-source
/// analysis and lowering. It does not establish conformance or executable admission.
/// Keeping it separate from declaration syntax prevents backend conformance lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceSpecializationFacts {
    pub specialization: InterfaceType<ResolvedType>,
    pub ancestors: Vec<InterfaceType<ResolvedType>>,
    pub requirements: Vec<RequirementFacts>,
    pub valid: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstrainedMemberSurface {
    pub requirements: Vec<RequirementFacts>,
    pub has_error_message: bool,
}

#[derive(Debug, Clone)]
pub(super) struct ConstrainedReceiver {
    parameter: String,
    interfaces: Vec<InterfaceType<TypeId>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementOrigin {
    pub interface: InterfaceType<ResolvedType>,
    pub declaration: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequirementFacts {
    pub name: String,
    /// Accessors share callable compatibility, but not method-name identity.
    pub accessor: Option<crate::ast::PropertyHookKind>,
    pub origins: Vec<RequirementOrigin>,
    pub signature: CallableSignatureSemanticInfo,
    pub generic_parameters: Vec<TypeParamDecl>,
    pub receiver_mode: ReceiverMode,
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

#[derive(Debug, Clone)]
pub(super) struct InterfaceDefinition {
    declaration: InterfaceDecl,
    parents: Vec<(InterfaceType<TypeId>, Span)>,
    local_requirements: Vec<(String, Option<crate::ast::PropertyHookKind>, MethodInfo)>,
    valid: bool,
}

#[derive(Debug, Clone)]
pub(super) struct CanonicalRequirement {
    name: String,
    accessor: Option<crate::ast::PropertyHookKind>,
    pub(super) origins: Vec<(InterfaceType<TypeId>, Span)>,
    pub(super) method: MethodInfo,
}

#[derive(Debug, Clone, Default)]
pub(super) struct InterfaceRequirements {
    ancestors: Vec<InterfaceType<TypeId>>,
    requirements: Vec<CanonicalRequirement>,
    valid: bool,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ClassConformances {
    pub(super) entries: Vec<(InterfaceType<TypeId>, Span)>,
    pub(super) active_arguments: Vec<TypeId>,
    pub(super) valid: bool,
}

#[derive(Debug, Clone)]
struct ClassConformancePath {
    interface: InterfaceType<TypeId>,
    origin: Span,
    declaring_class: String,
    direct: bool,
    conditional: bool,
    obligations: Result<Vec<super::nominal_proof::Step>, super::nominal_proof::Failure>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct DeclaredClassConformances {
    paths: Vec<ClassConformancePath>,
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
    Availability,
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
            Self::Availability => "availability condition",
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
    pub(super) fn record_constrained_receiver(&mut self, receiver: TypeId, span: Span) {
        let Some(receiver) = self.property_contract_receiver(receiver) else {
            return;
        };
        let TypeKind::TypeParameter(parameter) = self.types.kind(receiver).clone() else {
            return;
        };
        let interfaces = self.constrained_interfaces(&parameter, span);
        self.constrained_receivers.insert(
            span,
            ConstrainedReceiver {
                parameter,
                interfaces,
            },
        );
    }

    pub(super) fn publish_constrained_member_surfaces(&mut self) {
        // Resolve bounds while their lexical scope is active, then publish the
        // final inferred contracts rather than an early copy of their effects.
        for (span, receiver) in std::mem::take(&mut self.constrained_receivers) {
            let mut names = Vec::new();
            let mut has_error_message = false;
            for interface in &receiver.interfaces {
                let graph = self.canonical_interface_requirements(interface, &mut Vec::new());
                if !graph.valid {
                    continue;
                }
                has_error_message |= interface.name == "Error"
                    || graph
                        .ancestors
                        .iter()
                        .any(|ancestor| ancestor.name == "Error");
                names.extend(
                    graph
                        .requirements
                        .iter()
                        .map(|r| (r.name.clone(), r.accessor)),
                );
            }
            names.sort_by_key(|(name, accessor)| (name.clone(), accessor.map(|kind| kind as u8)));
            names.dedup();
            let requirements = names
                .into_iter()
                .filter_map(|(name, accessor)| {
                    self.constrained_requirement_from_interfaces(
                        &receiver.parameter,
                        &receiver.interfaces,
                        &name,
                        accessor,
                    )
                    .ok()
                    .flatten()
                    .map(|requirement| self.requirement_facts(&requirement))
                })
                .collect();
            self.contracts.constrained_member_surfaces.insert(
                span,
                ConstrainedMemberSurface {
                    requirements,
                    has_error_message,
                },
            );
        }
    }

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
        self.interface_requirement(interface, name, None)
            .map(|requirement| requirement.method)
    }

    pub(super) fn interface_requirement(
        &mut self,
        interface: &InterfaceType<TypeId>,
        name: &str,
        accessor: Option<crate::ast::PropertyHookKind>,
    ) -> Option<CanonicalRequirement> {
        let mut requirement = self
            .canonical_interface_requirements(interface, &mut Vec::new())
            .requirements
            .into_iter()
            .find(|requirement| requirement.name == name && requirement.accessor == accessor)?;
        self.resolve_interface_self_result(&mut requirement.method, interface);
        Some(requirement)
    }

    /// The receiver mode an interface requirement declares.
    pub(super) fn interface_requirement_receiver(&self, requirement: Span) -> Option<ReceiverMode> {
        self.interface_definitions
            .values()
            .flat_map(|definition| &definition.local_requirements)
            .find(|(_, _, method)| method.declaration == requirement)?
            .2
            .receiver_mode
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
        self.constrained_member_requirement(parameter, name, None, span)
    }

    pub(super) fn constrained_member_requirement(
        &mut self,
        parameter: &str,
        name: &str,
        accessor: Option<crate::ast::PropertyHookKind>,
        span: Span,
    ) -> Result<Option<CanonicalRequirement>, Vec<Span>> {
        let interfaces = self.constrained_interfaces(parameter, span);
        self.constrained_requirement_from_interfaces(parameter, &interfaces, name, accessor)
    }

    pub(super) fn constrained_interfaces(
        &mut self,
        parameter: &str,
        span: Span,
    ) -> Vec<InterfaceType<TypeId>> {
        let constraints = self
            .type_parameter_scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(parameter))
            .cloned()
            .unwrap_or_default();
        let mut interfaces = Vec::new();
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
            if let Some(interface) = self.resolve_interface_edge(&constraint, span) {
                interfaces.push(interface);
            }
        }
        let subject = self
            .types
            .intern(TypeKind::TypeParameter(parameter.to_string()));
        for step in self.nominal_condition_scopes.iter().flatten() {
            if step.query.subject == subject && !interfaces.contains(&step.query.interface) {
                interfaces.push(step.query.interface.clone());
            }
        }
        interfaces
    }

    fn constrained_requirement_from_interfaces(
        &mut self,
        parameter: &str,
        interfaces: &[InterfaceType<TypeId>],
        name: &str,
        accessor: Option<crate::ast::PropertyHookKind>,
    ) -> Result<Option<CanonicalRequirement>, Vec<Span>> {
        let mut candidates = Vec::new();
        for interface in interfaces {
            if let Some(requirement) = self
                .canonical_interface_requirements(interface, &mut Vec::new())
                .requirements
                .into_iter()
                .find(|requirement| requirement.name == name && requirement.accessor == accessor)
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
            .find(|requirement| requirement.name == method && requirement.accessor.is_none())
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
        if declared.receiver_mode == Some(ReceiverMode::Consuming) {
            self.record_capture_requirement_for_expr(object, scopes, CaptureRequirement::Take);
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
        // The selected target owns the effect rule. Erased and constrained
        // calls (type-parameter and trait-self receivers) pick their
        // implementation after this body is checked, so they carry the complete
        // automatic set, exactly as selected core operations do.
        match self.call_targets.get(&span).cloned() {
            Some(target) => {
                self.record_selected_contract_effects(&target, &method, span);
            }
            None => self.record_checked_effects(method.checked_effects, span),
        }
    }

    pub(super) fn record_trait_method_target(
        &mut self,
        owner: &str,
        name: &str,
        declaration: Span,
        span: Span,
    ) {
        self.call_targets
            .entry(span)
            .or_insert_with(|| CallableTarget::ConstrainedMethod {
                receiver: ResolvedType::TraitSelf(owner.into()),
                method_name: name.into(),
                requirement: declaration,
                implementations: Vec::new(),
            });
    }

    pub(super) fn specialize_constrained_method(
        &mut self,
        span: Span,
        body: &super::callable_contracts::BodyInstance,
    ) -> Option<MethodInfo> {
        let CallableTarget::ConstrainedMethod {
            receiver,
            method_name,
            requirement,
            ..
        } = self.call_targets.get(&span)?.clone()
        else {
            return None;
        };
        let receiver = self.types.intern_resolved(&body.substitute(&receiver));
        if requirement.source == crate::compiler_known_contracts::SOURCE_ID
            && matches!(
                self.types.kind(receiver),
                TypeKind::Integer(_) | TypeKind::Float(_) | TypeKind::Bool | TypeKind::String
            )
        {
            return None;
        }
        let TypeKind::Class(class_type) = self.types.kind(receiver).clone() else {
            return None;
        };
        if self.type_is_symbolic(receiver)
            || self.class_requires_trait_composition(&class_type.name)
            || !self.class_composition_is_valid(&class_type.name)
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
        if !self.primitive_interface_erasure_is_forbidden(value) {
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

    pub(super) fn primitive_interface_erasure_is_forbidden(&self, value: TypeId) -> bool {
        matches!(
            self.types.kind(value),
            TypeKind::Integer(_) | TypeKind::Float(_) | TypeKind::Bool | TypeKind::String
        )
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
        let arguments = if matches!(name, "Comparable" | "Equatable") {
            vec![ty]
        } else {
            Vec::new()
        };
        self.type_has_declared_interface(ty, &InterfaceType::new(name, arguments))
    }

    pub(super) fn public_iterable_element(&mut self, ty: TypeId) -> Option<TypeId> {
        self.core_iteration_element(ty, "Iterable")
    }

    fn core_iteration_element(&mut self, ty: TypeId, contract: &str) -> Option<TypeId> {
        if !self.is_core_interface(contract) {
            return None;
        }
        let interfaces = match self.types.kind(ty).clone() {
            TypeKind::Class(class) => {
                let conformances = self.class_interface_closure(&class, &mut HashSet::new());
                if !conformances.valid {
                    return None;
                }
                conformances
                    .entries
                    .into_iter()
                    .map(|(interface, _)| interface)
                    .collect::<Vec<_>>()
            }
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
                .find(|interface| interface.name == contract)
            {
                return iterable.arguments.first().copied();
            }
        }
        None
    }

    pub(super) fn select_core_operation(
        &mut self,
        ty: TypeId,
        operation: crate::compiler_known_contracts::CoreValueOperation,
        span: Span,
    ) -> bool {
        use crate::compiler_known_contracts::CoreValueOperation;

        // Only equality and duplication define absence handling for nullable values.
        let ty = match self.types.kind(ty) {
            TypeKind::Nullable(inner)
                if matches!(
                    operation,
                    CoreValueOperation::Equal | CoreValueOperation::Clone
                ) =>
            {
                *inner
            }
            _ => ty,
        };
        if matches!(self.types.kind(ty), TypeKind::Class(class) if !self.class_composition_is_valid(&class.name))
            || !self.has_core_contract(ty, operation.contract())
        {
            return false;
        }
        let Some((target, method)) = self.select_contract_method(ty, operation.method(), span)
        else {
            return false;
        };
        self.record_selected_effects(
            &target,
            &method,
            span,
            super::executable_effects::SelectedUse::Implicit,
        );
        let call = CoreValueCallInfo {
            operation,
            receiver_type: self.types.resolved(ty),
            target,
        };
        let calls = self.core_operation_calls.entry(span).or_default();
        if !calls.contains(&call) {
            calls.push(call);
        }
        true
    }

    fn select_contract_method(
        &mut self,
        ty: TypeId,
        method_name: &str,
        span: Span,
    ) -> Option<(CallableTarget, MethodInfo)> {
        let method_name = method_name.to_string();
        Some(match self.types.kind(ty).clone() {
            TypeKind::Class(class) => {
                let (declaring, method) = self.lookup_instance_method(&class, &method_name)?;
                let declaring = self.types.intern(TypeKind::Class(declaring));
                let ResolvedType::Class(class_type) = self.types.resolved(declaring) else {
                    unreachable!("resolved core implementation class")
                };
                let target = CallableTarget::Method {
                    class_type,
                    method_name,
                    direct_parent: false,
                };
                let direct = self
                    .classes
                    .get(&class.name)
                    .is_some_and(|class| !class.is_open);
                if direct {
                    self.exact_method_calls.insert((span, method.declaration));
                }
                self.record_method_effect_dependency(&method, direct);
                (target, method)
            }
            TypeKind::TypeParameter(parameter) => {
                let Ok(Some(required)) =
                    self.constrained_requirement(&parameter, &method_name, span)
                else {
                    return None;
                };
                let target = CallableTarget::ConstrainedMethod {
                    receiver: self.types.resolved(ty),
                    method_name,
                    requirement: required.method.declaration,
                    implementations: Vec::new(),
                };
                (target, required.method)
            }
            TypeKind::Interface(interface) => {
                let required = self
                    .canonical_interface_requirements(&interface, &mut Vec::new())
                    .requirements
                    .into_iter()
                    .find(|required| required.name == method_name && required.accessor.is_none())?;
                let interface = self.resolved_interface(&interface);
                let target = CallableTarget::InterfaceMethod {
                    interface,
                    method_name,
                    requirement: required.method.declaration,
                };
                (target, required.method)
            }
            _ => return None,
        })
    }

    /// Record a contract-selected call and return its transport.
    pub(super) fn record_selected_contract_effects(
        &mut self,
        target: &CallableTarget,
        method: &MethodInfo,
        span: Span,
    ) -> Vec<TypeId> {
        self.record_selected_effects(
            target,
            method,
            span,
            super::executable_effects::SelectedUse::Call,
        )
    }

    /// Record a contract-selected member use and return its transport. `selected`
    /// says how each specialization finds the member again.
    pub(super) fn record_selected_effects(
        &mut self,
        target: &CallableTarget,
        method: &MethodInfo,
        span: Span,
        selected: super::executable_effects::SelectedUse,
    ) -> Vec<TypeId> {
        if self.selects_implementation_after_checking(target) {
            let effects = self.complete_function_value_effects(&method.checked_effects, span);
            match super::executable_effects::call_source(target, method, false, selected) {
                Some(callee) => {
                    self.record_call_effect_profiles(effects.clone(), effects.clone(), span, callee)
                }
                None => self.record_checked_effects(effects.iter().copied(), span),
            }
            return effects;
        }
        let virtual_call = method.virtual_root.is_some()
            && !self
                .exact_method_calls
                .contains(&(span, method.declaration))
            && !matches!(
                target,
                CallableTarget::Method {
                    direct_parent: true,
                    ..
                }
            );
        let mut effects = method.checked_effects.clone();
        let dependency = if virtual_call {
            let ambient = self
                .ambient_effect_seed
                .virtual_calls
                .get(&method.declaration)
                .into_iter()
                .flatten()
                .cloned()
                .collect::<Vec<_>>();
            for effect in ambient {
                let effect = self.types.intern_resolved(&effect);
                if !effects.contains(&effect) {
                    effects.push(effect);
                }
            }
            super::callable_contracts::EffectDependency::Virtual(method.declaration)
        } else {
            super::callable_contracts::EffectDependency::Exact(method.declaration)
        };
        let source = super::executable_effects::call_source(target, method, virtual_call, selected);
        self.record_named_call_effects(dependency, effects.clone(), span, source);
        effects
    }

    /// Whether a call's implementation is selected after the calling body is
    /// checked. One check then stands for every implementation the call may
    /// select, so the call carries the complete ambient profile: erased
    /// interface calls, type-parameter receivers, and trait-self calls that the
    /// composer satisfies. A trait member with a body is the trait's own
    /// implementation, so its exact profile applies; each composed copy is
    /// revalidated against the composer's members (Decision 0134).
    fn selects_implementation_after_checking(&self, target: &CallableTarget) -> bool {
        match target {
            CallableTarget::InterfaceMethod { .. } => true,
            CallableTarget::ConstrainedMethod {
                receiver: ResolvedType::TraitSelf(owner),
                method_name,
                ..
            } => !matches!(
                self.trait_source_member(owner, method_name),
                Some(ClassMember::Method(method))
                    if !matches!(method.body, FunctionBody::Requirement { .. })
            ),
            CallableTarget::ConstrainedMethod { .. } => true,
            _ => false,
        }
    }

    pub(super) fn select_public_iteration(&mut self, ty: TypeId, span: Span) {
        let Some((target, method)) = self.select_contract_method(ty, "iterator", span) else {
            return;
        };
        self.record_selected_effects(
            &target,
            &method,
            span,
            super::executable_effects::SelectedUse::Implicit,
        );
        self.call_targets.entry(span).or_insert(target);
        let iterable = self.types.resolved(ty);
        for name in ["hasCurrent", "getCurrent", "advance"] {
            if let Some((target, operation)) =
                self.select_contract_method(method.return_ty, name, span)
            {
                self.record_selected_effects(
                    &target,
                    &operation,
                    span,
                    super::executable_effects::SelectedUse::IterationStep {
                        iterable: iterable.clone(),
                        member: name,
                    },
                );
            }
        }
    }

    pub(super) fn check_specialized_core_operations(
        &mut self,
        body: &super::callable_contracts::BodyInstance,
    ) {
        for (span, foreach) in self.foreach_loops.clone() {
            if foreach.iterable_family == ForeachIterableFamily::PublicIterable
                && body.declaration.contains(span)
            {
                self.specialize_constrained_method(span, body);
                let ty = self
                    .types
                    .intern_resolved(&body.substitute(&foreach.iterable_type));
                if !self.type_is_symbolic(ty) {
                    self.select_public_iteration(ty, span);
                }
            }
        }
        for (span, calls) in self.core_operation_calls.clone() {
            if body.declaration.contains(span) {
                for call in calls {
                    let template = self.types.intern_resolved(&call.receiver_type);
                    let ty = body.substitute_id(template, &mut self.types);
                    if ty != template && !self.type_is_symbolic(ty) {
                        self.select_core_operation(ty, call.operation, span);
                    }
                }
            }
        }
        for (span, left, right) in self.pending_relational_comparisons.clone() {
            if body.declaration.contains(span) {
                let left = body.substitute_id(left, &mut self.types);
                let right = body.substitute_id(right, &mut self.types);
                if !self.type_is_symbolic(left) && !self.type_is_symbolic(right) {
                    self.check_relational_operand_types(left, right, span);
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
            for property in &declaration.properties {
                self.collect_property_hook_signatures(
                    property,
                    &declaration.name,
                    crate::property_hooks::PropertyHookContext::Interface,
                );
                if let Some(previous) = names.insert(property.name.clone(), property.name_span) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0753",
                            format!("requirement `{}` is declared more than once", property.name),
                            property.name_span,
                        )
                        .with_title("Duplicate Interface Requirement")
                        .with_related(previous, "the previous requirement is here"),
                    );
                }
                let invalid = property.access != MemberAccess::External
                    || property.is_static
                    || property.open_span.is_some()
                    || property.override_span.is_some()
                    || property.initializer.is_some()
                    || property
                        .hooks
                        .iter()
                        .any(|hook| hook.body.as_block().is_some());
                if invalid {
                    self.diagnostics.push(
                        Diagnostic::new("E0749", "an interface property requires external instance accessor signatures without implementations or an initializer", property.span)
                            .with_title("Invalid Interface Requirement"),
                    );
                }
                for hook in &property.hooks {
                    let signature = &self.function_signatures[&hook.span];
                    requirements.push((
                        property.name.clone(),
                        Some(hook.kind),
                        MethodInfo {
                            declaration: hook.span,
                            is_open: false,
                            is_override: false,
                            virtual_root: None,
                            access: property.access,
                            receiver_mode: Some(
                                if hook.kind == crate::ast::PropertyHookKind::Set
                                    || hook.writable_span.is_some()
                                {
                                    ReceiverMode::Writable
                                } else {
                                    ReceiverMode::Readonly
                                },
                            ),
                            return_borrow: signature.return_borrow,
                            is_static: false,
                            enclosing_type_bindings: HashMap::new(),
                            type_params: Vec::new(),
                            params: signature.params.clone(),
                            return_ty: signature.return_ty,
                            checked_effects: signature.checked_effects.clone(),
                        },
                    ));
                }
            }
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
                    None,
                    MethodInfo {
                        declaration: requirement.span,
                        is_open: false,
                        is_override: false,
                        virtual_root: None,
                        access: MemberAccess::External,
                        receiver_mode: Some(ReceiverMode::from_declaration(requirement)),
                        return_borrow: signature.return_borrow,
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
        self.resolve_interface_edge_in_owner(ty, span, None)
    }

    pub(super) fn resolve_interface_edge_in_owner(
        &mut self,
        ty: &TypeRef,
        span: Span,
        owner: Option<&str>,
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
        let resolved = self.resolve_contract_type_in_owner(ty, span, owner);
        match self.types.kind(resolved) {
            TypeKind::Interface(interface) => Some(interface.clone()),
            _ => None,
        }
    }

    pub(super) fn resolve_contract_type(&mut self, ty: &TypeRef, span: Span) -> TypeId {
        self.resolve_contract_type_in_owner(ty, span, None)
    }

    fn resolve_contract_type_in_owner(
        &mut self,
        ty: &TypeRef,
        span: Span,
        owner: Option<&str>,
    ) -> TypeId {
        self.contract_type_depth += 1;
        let resolved = self.resolve_type_ref_in_position(ty, span, TypePosition::Value, owner);
        self.contract_type_depth -= 1;
        resolved
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
            .map(|(name, accessor, method)| CanonicalRequirement {
                name: name.clone(),
                accessor: *accessor,
                origins: vec![(interface.clone(), method.declaration)],
                method: self.substitute_requirement_method(method, &substitutions),
            })
            .collect::<Vec<_>>();
        let mut member_kinds = HashMap::new();
        let mut conflicts = HashSet::new();
        for requirement in inherited.iter().chain(&locals) {
            if let Some(previous) = member_kinds.insert(&requirement.name, requirement) {
                if previous.accessor.is_some() != requirement.accessor.is_some()
                    && conflicts.insert(&requirement.name)
                {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "E0753",
                            format!(
                                "`{}` is required as both a method and a property",
                                requirement.name
                            ),
                            definition.declaration.name_span,
                        )
                        .with_title("Conflicting Interface Requirements")
                        .with_related(
                            previous.method.declaration,
                            "one member contract is declared here",
                        )
                        .with_related(
                            requirement.method.declaration,
                            "the conflicting member contract is declared here",
                        ),
                    );
                    result.valid = false;
                }
            }
        }
        let mut names = HashSet::new();
        for requirement in &inherited {
            if !names.insert((requirement.name.clone(), requirement.accessor)) {
                continue;
            }
            let same_name = inherited
                .iter()
                .filter(|other| {
                    other.name == requirement.name && other.accessor == requirement.accessor
                })
                .collect::<Vec<_>>();
            if let Some(local) = locals.iter_mut().find(|local| {
                local.name == requirement.name && local.accessor == requirement.accessor
            }) {
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

    /// Substitution can turn a requirement's `T` into a region, so a region
    /// contract is decided from the substituted return type.
    fn substitute_requirement_method(
        &mut self,
        method: &MethodInfo,
        substitutions: &HashMap<String, TypeId>,
    ) -> MethodInfo {
        let mut method = self.substitute_contract_method(method, substitutions);
        let region =
            crate::types::ByteRegionKind::from_resolved(&self.types.resolved(method.return_ty));
        if region.is_some() {
            method.return_borrow =
                crate::ownership::requirement_return_borrow(region, method.receiver_mode);
        }
        method
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
        for parameter in &method.type_params {
            method.enclosing_type_bindings.remove(&parameter.name);
        }
        if method.type_params.is_empty() {
            for parameter in &mut method.params {
                parameter.ty = self.substitute_type_id(parameter.ty, substitutions);
            }
            method.return_ty = self.substitute_type_id(method.return_ty, substitutions);
            method.checked_effects = method
                .checked_effects
                .iter()
                .map(|effect| self.substitute_type_id(*effect, substitutions))
                .collect();
        } else {
            let signature = self.bound_method_signature(&method, &method.enclosing_type_bindings);
            method.params = signature.params;
            method.return_ty = signature.return_ty;
            method.checked_effects = signature.checked_effects;
        }
        if let Some(borrow) =
            self.return_borrow_for_instance(method.declaration, &method.enclosing_type_bindings)
        {
            method.return_borrow = borrow;
        }
        method.return_borrow =
            self.specialize_return_borrow(method.return_borrow, &method.params, method.return_ty);
        // A generic ordinary getter can become callable only after substitution.
        // Its interface contract must retain the same receiver bound as an
        // explicitly spelled callable getter, without borrowing the new carrier.
        if method.return_borrow.is_none()
            && self.non_null_function_type(method.return_ty).is_some()
            && self.interface_definitions.values().any(|definition| {
                definition
                    .local_requirements
                    .iter()
                    .any(|(_, accessor, requirement)| {
                        *accessor == Some(crate::ast::PropertyHookKind::Get)
                            && requirement.declaration == method.declaration
                    })
            })
        {
            method.return_borrow = Some(ReturnBorrow {
                source: BorrowSource::Receiver,
                writable: method.receiver_mode == Some(ReceiverMode::Writable),
                kind: ReturnBorrowKind::Retained,
                lifetime: RetainedLifetime::Cleanup,
            });
        }
        if method.declaration.source == crate::compiler_known_contracts::SOURCE_ID
            && crate::compiler_known_contracts::interfaces().any(|interface| {
                interface.name == "Iterator"
                    && interface.requirements.iter().any(|requirement| {
                        requirement.name == "getCurrent" && requirement.span == method.declaration
                    })
            })
        {
            method.return_borrow =
                self.type_is_move_type(method.return_ty)
                    .then_some(ReturnBorrow {
                        source: BorrowSource::Receiver,
                        writable: false,
                        kind: ReturnBorrowKind::Value,
                        lifetime: RetainedLifetime::LastUse,
                    });
        }
        method
    }

    pub(super) fn resolved_interface(
        &self,
        interface: &InterfaceType<TypeId>,
    ) -> InterfaceType<ResolvedType> {
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
            accessor: requirement.accessor,
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
                        borrow: false,
                        has_default: false,
                        default: None,
                        declaration: None,
                    })
                    .collect(),
                return_type: self.types.resolved(method.return_ty),
            },
            receiver_mode: method
                .receiver_mode
                .expect("interface requirements have a receiver"),
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
        let condition = if failures == [ContractMismatch::Availability] {
            self.callable_condition(method).ok().flatten()
        } else {
            None
        };
        let mut diagnostic = Diagnostic::new(
            "E0755",
            format!(
                "{} `{name}` does not preserve the required {detail}",
                match required.accessor {
                    Some(crate::ast::PropertyHookKind::Get) => "getter",
                    Some(crate::ast::PropertyHookKind::Set) => "setter",
                    None => "method",
                }
            ),
            condition
                .as_ref()
                .map_or(method.declaration, |condition| condition.clause),
        )
        .with_title("Interface Contract Does Not Match");
        if condition.is_some() {
            diagnostic =
                diagnostic.with_related(method.declaration, "the implementation is declared here");
        }
        for (_, origin) in &required.origins {
            diagnostic = diagnostic.with_related(*origin, "the required contract is declared here");
        }
        if failures.contains(&ContractMismatch::Availability) {
            if let Ok(Some(condition)) = self.callable_condition(method) {
                diagnostic = diagnostic.with_related(
                    condition.clause,
                    "this implementation's condition must follow from the required contract",
                );
            }
        }
        self.diagnostics.push(diagnostic);
    }

    pub(super) fn check_nominal_conformances(&mut self) {
        self.check_composed_method_obligations();
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
            self.type_parameter_scopes
                .push(type_parameter_scope(&declaration.type_params));
            let facts = self.checked_class_conformance_facts(&declaration, &class);
            self.type_parameter_scopes.pop();
            self.contracts.conformances.extend(facts);
        }
    }

    /// `Doria\Std\Io\Watchable` is sealed to the standard library (Decision
    /// 0140): the runtime backs every watchable source, so a conformance
    /// declared in authored source is rejected, whether it names the interface
    /// or one that extends it. Returns whether `origin` is such a conformance.
    fn report_sealed_conformance(
        &mut self,
        class: &str,
        interface: &InterfaceType<TypeId>,
        origin: Span,
    ) -> bool {
        let watchable = crate::compiler_known_stream::WATCHABLE;
        if interface.name != watchable
            || origin.source.is_compiler_owned()
            || self
                .interface_declaration(watchable)
                .is_none_or(|declaration| {
                    declaration.span.source != crate::compiler_known_stream::SOURCE_ID
                })
        {
            return false;
        }
        // One class declaration is checked once per concrete instance.
        if self
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0781" && diagnostic.span == origin)
        {
            return true;
        }
        let mut diagnostic = Diagnostic::new(
            "E0781",
            format!("class `{class}` cannot implement `{watchable}`"),
            origin,
        )
        .with_title("Standard Interface Cannot Be Implemented")
        .with_primary_label("Conformance Declared Here")
        .with_explanation(format!(
            "A watchable source is backed by the runtime, so only standard library types implement `{watchable}`."
        ))
        .with_help(
            "watch a `Doria\\Std\\Term\\RawMode` with `TransferPlan::watch` or `andWatch`; a class that owns one passes it instead of implementing the interface",
        );
        if let Some(Some(declared)) = self.declared_conformance_types.get(&origin) {
            if declared.name != watchable {
                diagnostic = diagnostic.with_note(format!(
                    "`{}` extends `{watchable}`, so implementing it implements `{watchable}` too.",
                    declared.name
                ));
            }
        }
        self.diagnostics.push(diagnostic);
        true
    }

    fn checked_class_conformance_facts(
        &mut self,
        declaration: &ClassDecl,
        class: &ClassType<TypeId>,
    ) -> Vec<ConformanceFacts> {
        let mut facts = Vec::new();
        let implementing_type = self.types.intern(TypeKind::Class(class.clone()));
        let conformances = self.class_interface_closure(class, &mut HashSet::new());
        let composition_valid = !self.class_requires_trait_composition(&class.name)
            && self.class_composition_is_valid(&class.name);
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
            let requirements = self.canonical_interface_requirements(&interface, &mut Vec::new());
            let sealed = self.report_sealed_conformance(&class.name, &interface, origin);
            let mut fact = ConformanceFacts {
                implementing_type: self.types.resolved(implementing_type),
                interface: self.resolved_interface(&interface),
                origin,
                status: if sealed
                    || !conformances.valid
                    || !requirements.valid
                    || !composition_valid
                {
                    ConformanceStatus::Invalid
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
                let method = if let Some(kind) = requirement.accessor {
                    self.lookup_instance_property(class, &requirement.name)
                        .and_then(|(owner, property)| {
                            self.property_accessor_method(&property, &owner, kind)
                        })
                } else {
                    self.find_concrete_contract_method(class, &requirement.name)
                };
                let mut implementation = RequirementImplementation {
                    requirement_origins: self.requirement_origins(&requirement),
                    implementation: None,
                    failures: Vec::new(),
                    exact_dynamic_return: None,
                };
                if conformances.valid && composition_valid && requirements.valid {
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
                                "class `{}` does not implement required {} `{}`",
                                class.name,
                                match requirement.accessor {
                                    Some(crate::ast::PropertyHookKind::Get) => "getter",
                                    Some(crate::ast::PropertyHookKind::Set) => "setter",
                                    None => "method",
                                },
                                requirement.name
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
            facts.push(fact);
        }
        facts
    }

    pub(super) fn specialize_conformance_facts(&mut self, classes: &[ClassSemanticInfo]) {
        for class in classes.iter().filter(|class| !class.arguments.is_empty()) {
            let Some(declaration) = self.program.items.iter().find_map(|item| match item {
                Item::Class(declaration) if declaration.name == class.declaration_name => {
                    Some(declaration.clone())
                }
                _ => None,
            }) else {
                continue;
            };
            let instance = ClassType::new(
                &class.declaration_name,
                class
                    .arguments
                    .iter()
                    .map(|argument| self.types.intern_resolved(argument))
                    .collect(),
            );
            for fact in self.checked_class_conformance_facts(&declaration, &instance) {
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

    pub(super) fn find_concrete_contract_method(
        &mut self,
        class: &ClassType<TypeId>,
        name: &str,
    ) -> Option<MethodInfo> {
        let root = class.name.clone();
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
                if class.name == root || method.access == MemberAccess::External {
                    return Some(self.specialize_method_for_class(&method, &class));
                }
            }
            current = self.specialized_parent_type(&class);
        }
        None
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

    pub(super) fn class_composition_is_valid(&self, class: &str) -> bool {
        let mut current = Some(class);
        let mut visited = HashSet::new();
        while let Some(name) = current {
            if self.composition.invalid_classes.contains(name) {
                return false;
            }
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

    // Later body, hierarchy, initialization, and ownership checks must revoke
    // provisional composition facts before any consumer can treat them as final.
    pub(super) fn invalidate_erroneous_compositions(&mut self) {
        if self.composition.classes.is_empty() {
            return;
        }
        for diagnostic in &self.diagnostics {
            if diagnostic.severity != crate::diagnostics::DiagnosticSeverity::Error {
                continue;
            }
            if let Some(origin) = self.composition.origin(diagnostic.span) {
                self.composition
                    .invalid_classes
                    .insert(origin.composing_class.clone());
            } else {
                for item in &self.program.items {
                    if let Item::Class(class) = item {
                        if class.span.contains(diagnostic.span) {
                            self.composition.invalid_classes.insert(class.name.clone());
                        }
                    }
                }
            }
        }
        for origin in &self.composition.origins {
            if self
                .contracts
                .traits
                .iter()
                .any(|facts| facts.name == origin.trait_type.name && !facts.valid)
            {
                self.composition
                    .invalid_classes
                    .insert(origin.composing_class.clone());
            }
        }
        let invalid = self
            .classes
            .keys()
            .filter(|name| !self.class_composition_is_valid(name))
            .cloned()
            .collect::<HashSet<_>>();
        for fact in &mut self.contracts.conformances {
            if matches!(&fact.implementing_type, ResolvedType::Class(class) if invalid.contains(&class.name))
            {
                fact.status = ConformanceStatus::Invalid;
                for implementation in &mut fact.implementations {
                    implementation.implementation = None;
                }
            }
        }
        self.core_operation_calls.retain(|_, calls| {
            calls.retain(|call| !matches!(&call.receiver_type, ResolvedType::Class(class) if invalid.contains(&class.name)));
            !calls.is_empty()
        });
        for target in self.call_targets.values_mut() {
            if let CallableTarget::ConstrainedMethod {
                implementations, ..
            } = target
            {
                implementations.retain(|implementation| !matches!(&implementation.receiver, ResolvedType::Class(class) if invalid.contains(&class.name)));
            }
        }
    }

    fn check_composed_method_obligations(&mut self) {
        for (index, obligation) in self.composition.obligations.clone().into_iter().enumerate() {
            let parameters = self
                .program
                .items
                .iter()
                .find_map(|item| match item {
                    Item::Class(class) if class.name == obligation.class => {
                        Some(class.type_params.clone())
                    }
                    _ => None,
                })
                .unwrap_or_default();
            self.type_parameter_scopes
                .push(type_parameter_scope(&parameters));
            let required = &obligation.requirement;
            let signature = self.resolve_function_signature(required, Some(&obligation.class));
            let contract = MethodInfo {
                declaration: required.span,
                access: required.access,
                is_static: required.is_static,
                receiver_mode: (!required.is_static)
                    .then_some(ReceiverMode::from_declaration(required)),
                return_borrow: signature.return_borrow,
                is_open: false,
                is_override: false,
                virtual_root: None,
                enclosing_type_bindings: HashMap::new(),
                type_params: signature.type_params,
                params: signature.params,
                return_ty: signature.return_ty,
                checked_effects: signature.checked_effects,
            };
            let class_type = self.symbolic_class_type(&obligation.class);
            let method = self
                .class_type(class_type)
                .and_then(|class| self.find_concrete_contract_method(&class, &required.name));
            let failure = if let Some(method) = method {
                let failures = self.method_contract_failures(&method, &contract);
                self.composition.obligations[index].implementation = Some(method.declaration);
                self.composition.obligations[index].failures = failures.clone();
                (!failures.is_empty()).then(|| {
                    Diagnostic::new(
                        "E0757",
                        format!(
                            "`{}::{}` does not satisfy its trait contract: {}",
                            obligation.class,
                            required.name,
                            failures
                                .iter()
                                .map(|failure| failure.description())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                        method.declaration,
                    )
                    .with_title("Trait Method Contract Mismatch")
                    .with_related(
                        obligation.origin.authored_declaration,
                        "the trait contract is declared here",
                    )
                })
            } else {
                Some(
                    Diagnostic::new(
                        "E0757",
                        format!(
                            "class `{}` does not supply required trait method `{}`",
                            obligation.class, required.name
                        ),
                        obligation
                            .origin
                            .paths
                            .first()
                            .and_then(|path| path.first())
                            .copied()
                            .unwrap_or(required.span),
                    )
                    .with_title("Trait Requirement Is Not Implemented")
                    .with_related(
                        obligation.origin.authored_declaration,
                        "the required method is declared here",
                    ),
                )
            };
            if let Some(failure) = failure {
                self.composition.invalid_classes.insert(obligation.class);
                self.diagnostics.push(failure);
            }
            self.type_parameter_scopes.pop();
        }
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
                        Some((&method.name, method.name_span))
                    }
                    ClassMember::Property(property) => {
                        self.with_declaration_condition(property.span, |checker| {
                            checker.check_trait_type(&property.ty, property.span, &declaration.name)
                        });
                        self.collect_property_hook_signatures(
                            property,
                            &declaration.name,
                            crate::property_hooks::PropertyHookContext::Trait,
                        );
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
                for callable in crate::property_hooks::member_callables(
                    member,
                    crate::property_hooks::PropertyHookContext::Trait,
                ) {
                    for parameter in &callable.params {
                        if let Some(default) = &parameter.default {
                            self.check_trait_expression(default);
                        }
                    }
                    if let Some(body) = callable.body.as_block() {
                        let mut forbidden = Vec::new();
                        crate::ast::visit::block(body, &mut |expression| {
                            collect_trait_parent_uses(expression, &mut forbidden)
                        });
                        self.report_trait_parent_uses(forbidden);
                    }
                }
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
                }
            }
            self.type_parameter_scopes.pop();
        }
    }

    fn check_trait_type(&mut self, ty: &TypeRef, span: Span, owner: &str) {
        let ty = self.resolve_type_ref_in_position(ty, span, TypePosition::Value, Some(owner));
        if crate::compiler_known_stream::contains_call_scoped_request(&self.types.resolved(ty)) {
            self.report_call_scoped_request(span, "be stored in a trait member");
        }
    }

    pub(super) fn check_trait_bodies(&mut self, declaration: &crate::ast::TraitDecl) {
        let before = self.diagnostics.len();
        self.type_parameter_scopes
            .push(type_parameter_scope(&declaration.type_params));
        self.contract_type_depth += 1;
        for member in &declaration.members {
            if let ClassMember::Property(property) = member {
                self.check_property_initializer(&declaration.name, property);
                self.check_property_hook_bodies(
                    property,
                    &declaration.name,
                    crate::property_hooks::PropertyHookContext::Trait,
                );
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
        let ClassMember::Property(property) = self.trait_source_member(owner, name)? else {
            return None;
        };
        if property.is_static {
            return None;
        }
        let property = property.clone();
        let ty = self.resolve_property_type(&property, owner, property.span);
        Some(PropertyInfo {
            access: property.access,
            writable: property.writable,
            borrowed_source: None,
            ty,
            init_state: if property.initializer.is_some() {
                PropertyInitState::HasInitializer
            } else {
                PropertyInitState::Uninitialized
            },
            declaration_span: property.span,
            hooks: crate::property_hooks::declaration_facts(
                &property,
                crate::property_hooks::PropertyHookContext::Trait,
            )
            .map(|facts| facts.symbols()),
        })
    }

    pub(super) fn trait_method(&self, owner: &str, name: &str) -> Option<MethodInfo> {
        let ClassMember::Method(method) = self.trait_source_member(owner, name)? else {
            return None;
        };
        let signature = self.function_signatures.get(&method.span)?;
        Some(MethodInfo {
            declaration: method.span,
            access: method.access,
            is_static: method.is_static,
            receiver_mode: (!method.is_static).then_some(ReceiverMode::from_declaration(method)),
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

    fn trait_source_member(&self, owner: &str, name: &str) -> Option<&ClassMember> {
        let authored = &self.trait_declaration(owner)?.members;
        let imported: &[ClassMember] = match self.callable_body_program {
            CallableBodyProgram::Expanded(_) => &[],
            CallableBodyProgram::Authored(_) => self.composition.source_trait_members.get(owner)?,
        };
        let mut matches = authored
            .iter()
            .chain(imported)
            .filter(|member| crate::trait_composition::member_name(member) == name);
        let member = matches.next()?;
        // Ambiguous contributions still need the shared coherence decision.
        if matches.next().is_some() {
            return None;
        }
        Some(member)
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
            if !self.check_nominal_type_constraints(
                &declaration.name,
                &parameters,
                &arguments,
                *span,
            ) {
                continue;
            }
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
        if !self.callable_availability_preserved(method, required) {
            failures.push(ContractMismatch::Availability);
        }
        let generic_valid = method.type_params.len() == required.type_params.len()
            && self.normalized_method_constraints(method)
                == self.normalized_method_constraints(required);
        if !generic_valid {
            failures.push(ContractMismatch::GenericParameters);
        }
        let bindings = self.normalized_method_bindings(method);
        let actual_signature = self.bound_method_signature(method, &bindings);
        let bindings = self.normalized_method_bindings(required);
        let required_signature = self.bound_method_signature(required, &bindings);
        if method.params.len() != required.params.len() {
            failures.push(ContractMismatch::ParameterCount);
        }
        for (actual, expected) in actual_signature
            .params
            .iter()
            .zip(&required_signature.params)
        {
            if actual.name != expected.name {
                failures.push(ContractMismatch::ParameterNames);
            }
            if actual.ty != expected.ty {
                failures.push(ContractMismatch::ParameterTypes);
            }
            if actual.take != expected.take || actual.writable != expected.writable {
                failures.push(ContractMismatch::ParameterOwnership);
            }
        }
        let receiver_matches = match (method.receiver_mode, required.receiver_mode) {
            (Some(actual), Some(required)) => actual.can_override(required),
            (actual, required) => actual == required,
        };
        if !receiver_matches {
            failures.push(ContractMismatch::Receiver);
        }
        if !self
            .override_return_is_compatible(required_signature.return_ty, actual_signature.return_ty)
        {
            failures.push(ContractMismatch::ReturnType);
        }
        if !match (required.return_borrow, method.return_borrow) {
            (None, None) => true,
            (Some(expected), None) => expected.kind == ReturnBorrowKind::Retained,
            (Some(expected), Some(actual)) => {
                expected.kind == actual.kind
                    && expected.source == actual.source
                    && expected.lifetime.accepts(actual.lifetime)
                    && expected
                        .kind
                        .accepts_writable(expected.writable, actual.writable)
            }
            _ => false,
        } {
            failures.push(ContractMismatch::ReturnProvenance);
        }
        if !actual_signature.checked_effects.iter().all(|actual| {
            crate::checked_effects::is_automatic_effect(&self.types.resolved(*actual))
                || self.checked_error_type_covers(&required_signature.checked_effects, *actual)
        }) {
            failures.push(ContractMismatch::CheckedEffects);
        }
        if required.access == MemberAccess::External && method.access != MemberAccess::External {
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

    pub(super) fn normalized_method_constraints(
        &mut self,
        method: &MethodInfo,
    ) -> Vec<HashSet<(String, Vec<TypeId>)>> {
        let bindings = self.normalized_method_bindings(method);
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

    pub(super) fn normalized_method_bindings(
        &mut self,
        method: &MethodInfo,
    ) -> HashMap<String, TypeId> {
        let mut bindings = method.enclosing_type_bindings.clone();
        for (index, parameter) in method.type_params.iter().enumerate() {
            bindings.insert(
                parameter.name.clone(),
                self.types
                    .intern(TypeKind::TypeParameter(format!("#method{index}"))),
            );
        }
        bindings
    }

    pub(super) fn method_signature_template(&self, method: &MethodInfo) -> FunctionInfo {
        if !method.type_params.is_empty() {
            if let Some(signature) = self.function_signatures.get(&method.declaration) {
                return signature.clone();
            }
        }
        // Compiler-known methods carry their signature directly.
        FunctionInfo {
            declaration: method.declaration,
            type_params: method.type_params.clone(),
            params: method.params.clone(),
            return_ty: method.return_ty,
            return_borrow: method.return_borrow,
            checked_effects: method.checked_effects.clone(),
        }
    }

    fn bound_method_signature(
        &mut self,
        method: &MethodInfo,
        bindings: &HashMap<String, TypeId>,
    ) -> FunctionInfo {
        // Bind the authored template once: an owner replacement can have the
        // same spelling as a method parameter without becoming that parameter.
        let mut signature = self.method_signature_template(method);
        if method.type_params.is_empty() {
            return signature;
        }
        for parameter in &mut signature.params {
            parameter.ty = self.substitute_type_id(parameter.ty, bindings);
        }
        signature.return_ty = self.substitute_type_id(signature.return_ty, bindings);
        signature.checked_effects = signature
            .checked_effects
            .iter()
            .map(|effect| self.substitute_type_id(*effect, bindings))
            .collect();
        signature
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
