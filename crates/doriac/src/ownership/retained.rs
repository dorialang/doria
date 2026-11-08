//! Source loans carried by iterators and byte regions, independently of carrier ownership.

use super::*;

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct SourceTypes {
    classes: HashSet<crate::types::ClassType<ResolvedType>>,
    interfaces: HashSet<crate::types::InterfaceType<ResolvedType>>,
    enums: HashSet<crate::enums::EnumType>,
}

impl SourceTypes {
    pub(crate) fn from_semantic_classes(
        classes: &[crate::semantics::ClassSemanticInfo],
        enums: &[crate::semantics::EnumSemanticInfo],
        contracts: &crate::semantics::contracts::ContractFacts,
    ) -> Self {
        Self::new(
            &classes
                .iter()
                .map(|class| {
                    (
                        crate::types::ClassType::new(
                            class.declaration_name.clone(),
                            class.arguments.clone(),
                        ),
                        class
                            .properties
                            .iter()
                            .any(|property| property.borrowed_source.is_some()),
                        class
                            .properties
                            .iter()
                            .map(|property| property.ty.clone())
                            .collect(),
                    )
                })
                .collect::<Vec<_>>(),
            &enums
                .iter()
                .map(|definition| {
                    (
                        definition.source_type.clone(),
                        definition
                            .cases
                            .iter()
                            .flat_map(|case| &case.payload)
                            .map(|field| field.ty.clone())
                            .collect(),
                    )
                })
                .collect::<Vec<_>>(),
            contracts,
        )
    }

    pub(crate) fn new(
        classes: &[(
            crate::types::ClassType<ResolvedType>,
            bool,
            Vec<ResolvedType>,
        )],
        enums: &[(crate::enums::EnumType, Vec<ResolvedType>)],
        contracts: &crate::semantics::contracts::ContractFacts,
    ) -> Self {
        let mut sources = Self {
            enums: HashSet::new(),
            classes: classes
                .iter()
                .filter(|(name, borrowed, _)| {
                    *borrowed
                        || matches!(
                            name.name.as_str(),
                            crate::compiler_known_stream::READINESS_SET
                                | crate::compiler_known_stream::TRANSFER_PLAN
                                | crate::compiler_known_stream::TRANSFER_GROUP
                        )
                })
                .map(|(name, _, _)| name.clone())
                .collect(),
            interfaces: contracts
                .interface_specializations
                .iter()
                .filter(|interface| interface.valid)
                .filter(|interface| {
                    interface.requirements.iter().any(|requirement| {
                        requirement.origins.iter().any(|origin| {
                            origin.interface.name == "Iterator"
                                && origin.declaration.source
                                    == crate::compiler_known_contracts::SOURCE_ID
                        })
                    })
                })
                .map(|interface| interface.specialization.clone())
                .collect(),
        };
        loop {
            let before = sources.classes.len() + sources.interfaces.len() + sources.enums.len();
            for (ty, fields) in enums {
                if fields.iter().any(|field| sources.carries(field)) {
                    sources.enums.insert(ty.clone());
                }
            }
            for (name, _, properties) in classes {
                if properties.iter().any(|ty| sources.carries(ty)) {
                    sources.classes.insert(name.clone());
                }
            }
            for conformance in &contracts.conformances {
                if conformance.status == crate::semantics::contracts::ConformanceStatus::Checked
                    && sources.carries(&conformance.implementing_type)
                {
                    sources.interfaces.insert(conformance.interface.clone());
                }
            }
            if before == sources.classes.len() + sources.interfaces.len() + sources.enums.len() {
                break;
            }
        }
        sources
    }

    pub(crate) fn carries(&self, ty: &ResolvedType) -> bool {
        match ty {
            ResolvedType::ByteRegion(_) | ResolvedType::Function(_) => true,
            ResolvedType::Nullable(inner)
            | ResolvedType::TypedArray(inner)
            | ResolvedType::List(inner)
            | ResolvedType::Set(inner)
            | ResolvedType::SortedSet(inner)
            | ResolvedType::PriorityQueue(inner)
            | ResolvedType::Deque(inner) => self.carries(inner),
            ResolvedType::Dictionary(key, value) | ResolvedType::SortedDictionary(key, value) => {
                self.carries(key) || self.carries(value)
            }
            ResolvedType::Class(class) => self.classes.contains(class),
            ResolvedType::Interface(interface) => self.interfaces.contains(interface),
            ResolvedType::Enum(ty) => self.enums.contains(ty),
            _ => false,
        }
    }

    pub(crate) fn lifetime(&self, ty: &ResolvedType) -> Option<RetainedLifetime> {
        if crate::types::ByteRegionKind::from_resolved(ty).is_some() {
            Some(RetainedLifetime::LastUse)
        } else if self.carries(ty) {
            Some(RetainedLifetime::Cleanup)
        } else {
            None
        }
    }
}

#[derive(Default)]
pub(super) struct Analysis {
    pub callables: HashMap<Span, RetainedCallableInfo>,
    pub current: Option<FunctionContext>,
    pub expression_values: HashMap<Span, RetainedValueState>,
    pub yielded_values: Vec<Option<RetainedValueState>>,
    source_types: SourceTypes,
    requirements: HashMap<Span, Signature>,
    pub(super) implementations: HashMap<Span, Vec<Span>>,
    pub(super) analyzed_callables: HashSet<Span>,
    /// Bodyless trait members. An interface requirement is in `requirements`.
    trait_requirements: HashSet<Span>,
    /// Requirements returning one of their own generic parameters. That result
    /// comes from the call's arguments, so a region there is not the contract's.
    argument_typed_requirements: HashSet<Span>,
    recursive_calls: HashSet<(Span, Span)>,
    live_bindings: HashMap<Span, HashMap<Span, HashSet<BindingId>>>,
    ancestors: HashMap<String, Vec<String>>,
    pub(super) pending_boundaries: HashSet<Span>,
    pub(super) properties: HashMap<(String, String), (crate::class_layout::PropertyId, bool)>,
    pub(super) pending_expressions: HashMap<Span, Expr>,
    pub(super) failed_storage_calls: HashMap<Span, Scopes>,
}

#[cfg(test)]
mod enum_source_tests {
    use super::*;
    use crate::enums::{EnumId, EnumType};

    #[test]
    fn retained_enum_payloads_follow_exact_applications_and_nested_storage() {
        let application = |argument| EnumType::with_arguments(EnumId(0), "Packet", vec![argument]);
        let region = ResolvedType::ByteRegion(crate::types::ByteRegionKind::Readonly);
        let carrying = application(region.clone());
        let plain = application(ResolvedType::Bool);
        let outer = EnumType::new(EnumId(1), "Outer");
        let class = crate::types::ClassType::new("Holder", vec![]);
        let sources = SourceTypes::new(
            &[(
                class.clone(),
                false,
                vec![ResolvedType::Enum(carrying.clone())],
            )],
            &[
                (outer.clone(), vec![ResolvedType::Class(class)]),
                (plain.clone(), vec![ResolvedType::Bool]),
                (carrying.clone(), vec![region]),
            ],
            &Default::default(),
        );
        assert_eq!(
            sources.lifetime(&ResolvedType::Enum(outer)),
            Some(RetainedLifetime::Cleanup)
        );
        assert!(sources.carries(&ResolvedType::Enum(carrying)));
        assert!(!sources.carries(&ResolvedType::Enum(plain)));
    }
}

pub(super) struct FunctionContext {
    pub(super) declaration: Span,
    pub(super) parameters: Vec<(Option<BindingId>, bool)>,
    constructor: bool,
    pub(super) closure: bool,
    pub(super) owns_receiver: bool,
    pub(super) returned_selectors: Vec<owned_storage::Selector>,
    pub(super) returned_storage: Option<owned_storage::ReturnedStorage>,
    return_borrow: Option<crate::types::FunctionReturnBorrow>,
}

impl Analysis {
    pub(super) fn seed_callables(
        &mut self,
        summaries: &HashMap<Span, RetainedCallableInfo>,
        composition: &crate::trait_composition::CompositionPlan,
    ) {
        self.callables.extend(summaries.clone());
        // Opaque contract summaries do not establish that a body was analyzed.
        self.analyzed_callables.extend(
            composition
                .callable_templates
                .keys()
                .filter(|declaration| summaries.contains_key(declaration))
                .copied(),
        );
    }

    /// A loan reached through a retained `borrow` field depends on the
    /// receiver's caller source; only the stored owning prefix names a path.
    pub(super) fn owning_prefix(
        &self,
        fields: &[crate::class_layout::PropertyId],
    ) -> Vec<crate::class_layout::PropertyId> {
        fields
            .iter()
            .take_while(|field| {
                !crate::compiler_known_stream::is_child_pipe_place(**field)
                    && !self
                        .properties
                        .values()
                        .any(|(id, borrowed)| id == *field && *borrowed)
            })
            .copied()
            .collect()
    }

    /// A generic template with no concrete instance is still checked; without
    /// field identities every field of `$this` would overlap every other.
    /// Declaration-scoped identities keep its stored paths disjoint. They name
    /// places only, never storage, and no instance shares their class id.
    pub(super) fn add_template_fields(
        &mut self,
        declarations: &HashMap<(String, String), super::PropertyInfo>,
    ) {
        let mut missing = declarations
            .iter()
            .filter(|(key, _)| !self.properties.contains_key(*key))
            .collect::<Vec<_>>();
        missing.sort_by(|left, right| left.0.cmp(right.0));
        let mut classes = HashMap::<&str, crate::class_layout::ClassId>::new();
        let mut indices = HashMap::<&str, usize>::new();
        for ((class, property), info) in missing {
            let next = usize::MAX - classes.len();
            let id = *classes
                .entry(class.as_str())
                .or_insert(crate::class_layout::ClassId(next));
            let index = indices.entry(class.as_str()).or_insert(0);
            self.properties.insert(
                (class.clone(), property.clone()),
                (
                    crate::class_layout::PropertyId {
                        class: id,
                        index: *index,
                    },
                    info.borrowed_source.is_some(),
                ),
            );
            *index += 1;
        }
    }

    pub fn new(
        classes: &[crate::semantics::ClassSemanticInfo],
        contracts: &crate::semantics::contracts::ContractFacts,
        move_enums: &super::MoveEnums,
        source_types: &SourceTypes,
    ) -> Self {
        let mut analysis = Self {
            source_types: source_types.clone(),
            ancestors: classes
                .iter()
                .map(|class| {
                    (
                        class.declaration_name.clone(),
                        class
                            .ancestors
                            .iter()
                            .map(|ancestor| ancestor.name.clone())
                            .collect(),
                    )
                })
                .collect(),
            properties: classes
                .iter()
                .flat_map(|class| {
                    class.properties.iter().map(|property| {
                        (
                            (class.declaration_name.clone(), property.name.clone()),
                            (property.id, property.borrowed_source.is_some()),
                        )
                    })
                })
                .collect(),
            ..Self::default()
        };
        for interface in contracts
            .interface_specializations
            .iter()
            .filter(|interface| interface.valid)
        {
            for requirement in &interface.requirements {
                for origin in &requirement.origins {
                    if origin.declaration.source == crate::compiler_known_stream::SOURCE_ID
                        && crate::compiler_known_stream::is_pending_phase(
                            &origin.interface.name,
                            &requirement.name,
                        )
                        && matches!(requirement.name.as_str(), "prepareRead" | "prepareWrite")
                    {
                        analysis.pending_boundaries.insert(origin.declaration);
                    }
                    let signature = Signature {
                        declaration: Some(origin.declaration),
                        params: requirement
                            .signature
                            .parameters
                            .iter()
                            .map(|parameter| Parameter {
                                name: parameter.name.clone(),
                                move_type: resolved_type_is_move_type(
                                    &parameter.r#type,
                                    move_enums,
                                ) || resolved_type_requires_conservative_move(
                                    &parameter.r#type,
                                ),
                                class_type: matches!(parameter.r#type, ResolvedType::Class(_)),
                                generic: resolved_type_requires_conservative_move(
                                    &parameter.r#type,
                                ),
                                take: parameter.take,
                                writable: parameter.writable,
                                borrow: false,
                            })
                            .collect(),
                        returns_move_type: resolved_type_is_move_type(
                            &requirement.signature.return_type,
                            move_enums,
                        ) && requirement
                            .return_borrow
                            .is_none_or(|borrow| borrow.kind == ReturnBorrowKind::Retained),
                        // A region contract follows the type of each call:
                        // see `specialize_requirement_return`.
                        return_borrow: requirement.return_borrow.filter(|_| {
                            crate::types::ByteRegionKind::from_resolved(
                                &requirement.signature.return_type,
                            )
                            .is_none()
                        }),
                        receiver: Some(if requirement.receiver_mode.is_consuming() {
                            UseMode::Give
                        } else if requirement.receiver_mode.is_writable() {
                            UseMode::Write
                        } else {
                            UseMode::Read
                        }),
                        ..Signature::default()
                    };
                    analysis
                        .requirements
                        .entry(origin.declaration)
                        .or_insert(signature);
                    if requirement.signature.generic_parameter_count > 0
                        && crate::types::resolved_type_is_symbolic(
                            &requirement.signature.return_type,
                        )
                    {
                        analysis
                            .argument_typed_requirements
                            .insert(origin.declaration);
                    }
                    if requirement.name == "iterator"
                        && requirement.origins.iter().any(|origin| {
                            origin.declaration.source == crate::compiler_known_contracts::SOURCE_ID
                                && origin.interface.name == "Iterable"
                        })
                    {
                        analysis.callables.insert(
                            origin.declaration,
                            RetainedCallableInfo {
                                returns: vec![RetainedSource {
                                    source: BorrowSource::Receiver,
                                    inherited: false,
                                    writable: false,
                                    lifetime: RetainedLifetime::Cleanup,
                                    owned_path: Vec::new(),
                                }],
                                ..RetainedCallableInfo::default()
                            },
                        );
                    }
                }
            }
        }
        for conformance in &contracts.conformances {
            if conformance.status != crate::semantics::contracts::ConformanceStatus::Checked {
                continue;
            }
            for implementation in &conformance.implementations {
                let Some(declaration) = implementation.implementation else {
                    continue;
                };
                for origin in &implementation.requirement_origins {
                    if analysis.pending_boundaries.contains(&origin.declaration) {
                        analysis.pending_boundaries.insert(declaration);
                    }
                    let implementations = analysis
                        .implementations
                        .entry(origin.declaration)
                        .or_default();
                    if !implementations.contains(&declaration) {
                        implementations.push(declaration);
                    }
                }
            }
        }
        analysis
    }

    pub fn enter_function(
        &mut self,
        function: &ast::FunctionDecl,
        bindings: &BindingResolution,
    ) -> Option<FunctionContext> {
        if function.name == "__construct" {
            for (index, parameter) in function.params.iter().enumerate().filter(|(_, parameter)| {
                parameter.constructor_role.is_promoted() && parameter.borrow_span.is_some()
            }) {
                let source = RetainedSource {
                    source: BorrowSource::Parameter(index),
                    inherited: false,
                    writable: parameter.writable,
                    lifetime: RetainedLifetime::Cleanup,
                    owned_path: Vec::new(),
                };
                let info = self.callables.entry(function.span).or_default();
                append_sources(&mut info.constructs, [source]);
            }
        }
        self.current.replace(FunctionContext {
            declaration: function.span,
            parameters: function
                .params
                .iter()
                .map(|parameter| {
                    (
                        bindings.declaration_by_span.get(&parameter.span).copied(),
                        parameter.take,
                    )
                })
                .collect(),
            constructor: function.name == "__construct",
            closure: false,
            owns_receiver: function.consuming_this,
            returned_selectors: Vec::new(),
            returned_storage: None,
            return_borrow: None,
        })
    }

    pub fn enter_closure(
        &mut self,
        closure: &ast::ClosureExpression,
        bindings: &BindingResolution,
        return_borrow: Option<crate::types::FunctionReturnBorrow>,
    ) -> Option<FunctionContext> {
        self.current.replace(FunctionContext {
            declaration: closure.span,
            parameters: closure
                .parameters
                .iter()
                .map(|parameter| {
                    (
                        bindings
                            .declaration_by_span
                            .get(&parameter.name_span)
                            .copied(),
                        parameter.take,
                    )
                })
                .collect(),
            constructor: false,
            closure: true,
            owns_receiver: false,
            returned_selectors: Vec::new(),
            returned_storage: None,
            return_borrow,
        })
    }

    pub fn leave_callable(&mut self, previous: Option<FunctionContext>) {
        if let Some(current) = std::mem::replace(&mut self.current, previous) {
            // Reanalysis replaces early unknowns without changing recursive call inputs mid-body.
            let info = self.callables.entry(current.declaration).or_default();
            info.returned_selectors = current.returned_selectors;
            info.returned_storage = current.returned_storage;
        }
    }

    fn carries_source(&self, ty: &ResolvedType) -> bool {
        self.source_types.carries(ty)
    }

    pub(super) fn source_lifetime(&self, ty: &ResolvedType) -> Option<RetainedLifetime> {
        self.source_types.lifetime(ty)
    }
}

impl Checker<'_> {
    pub(super) fn prepare_retained_liveness(&mut self, body: &ast::Block, declaration: Span) {
        if self.retained.live_bindings.contains_key(&declaration) {
            return;
        }
        let graph = crate::control_flow::build_function_cfg_with_checked_effects(
            body,
            declaration,
            self.given_preludes,
            self.checked_effect_sites,
            self.catch_error_types,
            self.catch_coverage,
            &HashSet::new(),
        );
        let actions = graph
            .nodes
            .iter()
            .map(|node| {
                use crate::control_flow::NodeAction;
                let mut uses = HashSet::new();
                let mut definitions = HashSet::new();
                let mut excluded = Vec::<Span>::new();
                let assignment = match &node.action {
                    NodeAction::Statement(Stmt::Assignment(assignment))
                    | NodeAction::ForInitializer(ast::ForInitializer::Assignment(assignment)) => {
                        Some(assignment)
                    }
                    NodeAction::ForIncrement(ast::ForIncrement::Assignment(assignment)) => {
                        Some(assignment.as_ref())
                    }
                    _ => None,
                };
                if let Some(assignment) =
                    assignment.filter(|assignment| assignment.op == AssignOp::Assign)
                {
                    if let Expr::Variable { span, .. } = ungroup_expr(&assignment.target) {
                        let binding = self.binding_resolution.uses_by_span.get(span).copied();
                        definitions.extend(binding);
                        // Replacement acquires its RHS before releasing the
                        // previous value. Its existing loans therefore remain
                        // live at entry even though the committed assignment
                        // kills that value's identity for successor liveness.
                        uses.extend(binding);
                        excluded.push(*span);
                    }
                }
                if let NodeAction::Statement(Stmt::VarDecl(declaration))
                | NodeAction::ForInitializer(ast::ForInitializer::VarDecl(declaration)) =
                    &node.action
                {
                    definitions.extend(declaration.bindings.iter().filter_map(|binding| {
                        self.binding_resolution
                            .declaration_by_span
                            .get(&binding.span)
                            .copied()
                    }));
                }
                let mut visit = |expr: &Expr| {
                    if excluded.iter().any(|span| span.contains(expr.span())) {
                        return;
                    }
                    match expr {
                        Expr::Closure(closure) => {
                            if let Some(info) =
                                self.closures.get(&ClosureId::from_span(closure.span))
                            {
                                uses.extend(
                                    info.captures
                                        .iter()
                                        .map(|capture| capture.source_binding_id),
                                );
                            }
                            // Only acquisition runs here. The body has its own CFG.
                            excluded.push(closure.span);
                        }
                        Expr::When(when) => {
                            // The shared CFG already projects its branches and
                            // finalizers before the enclosing action commits.
                            excluded.push(when.span);
                        }
                        Expr::Variable { span, .. } | Expr::This { span } => {
                            uses.extend(self.binding_resolution.uses_by_span.get(span).copied());
                        }
                        _ => {}
                    }
                };
                match &node.action {
                    NodeAction::Statement(statement) => ast::visit::stmt(statement, &mut visit),
                    NodeAction::Expression(expr) => ast::visit::expr(expr, &mut visit),
                    NodeAction::ForInitializer(ast::ForInitializer::VarDecl(declaration)) => {
                        ast::visit::expr(&declaration.initializer, &mut visit);
                    }
                    NodeAction::ForInitializer(ast::ForInitializer::Assignment(assignment)) => {
                        ast::visit::expr(&assignment.target, &mut visit);
                        ast::visit::expr(&assignment.value, &mut visit);
                    }
                    NodeAction::ForIncrement(ast::ForIncrement::Assignment(assignment)) => {
                        ast::visit::expr(&assignment.target, &mut visit);
                        ast::visit::expr(&assignment.value, &mut visit);
                    }
                    NodeAction::ForIncrement(ast::ForIncrement::Increment(increment)) => {
                        ast::visit::expr(&increment.target, &mut visit);
                    }
                    // An assumption refines an already evaluated value.
                    NodeAction::None | NodeAction::Assume { .. } => {}
                }
                (uses, definitions)
            })
            .collect::<Vec<_>>();
        let mut live = actions
            .iter()
            .map(|(uses, _)| uses.clone())
            .collect::<Vec<_>>();
        loop {
            let mut changed = false;
            for node in graph.nodes.iter().rev() {
                let (uses, definitions) = &actions[node.id.0];
                let mut incoming = uses.clone();
                for successor in &node.successors {
                    incoming.extend(
                        live[successor.0]
                            .iter()
                            .filter(|id| !definitions.contains(id))
                            .copied(),
                    );
                }
                if incoming != live[node.id.0] {
                    live[node.id.0] = incoming;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let mut sites = HashMap::<Span, HashSet<BindingId>>::new();
        for node in &graph.nodes {
            // Finalizers can appear along several edges. Releasing a loan at
            // their shared source site requires it to be dead on every edge.
            let site = sites.entry(node.span).or_default();
            for entry in &node.evaluation_entries {
                site.extend(live[entry.0].iter().copied());
            }
        }
        self.retained.live_bindings.insert(declaration, sites);
    }

    pub(super) fn retained_live_bindings(&self, site: Span) -> Option<HashSet<BindingId>> {
        let current = self.retained.current.as_ref()?;
        let mut live = self
            .retained
            .live_bindings
            .get(&current.declaration)?
            .get(&site)?
            .clone();
        // A borrowed input still belongs to the caller after this body returns.
        // Local last use cannot erase effects stored in that external owner.
        live.extend(
            current
                .parameters
                .iter()
                .filter(|(_, take)| !take)
                .filter_map(|(id, _)| *id),
        );
        Some(live)
    }

    pub(super) fn infer_retained_callables(&mut self, program: &ast::Program) {
        self.index_pending_expressions(program);
        let callables = program_callables(program);
        for (function, _) in &callables {
            self.seed_pending_file_selection(function);
            if matches!(function.body, ast::FunctionBody::Requirement { .. }) {
                self.retained.trait_requirements.insert(function.span);
            }
            if let ast::FunctionBody::Intrinsic { implementation, .. } = function.body {
                append_sources(
                    &mut self
                        .retained
                        .callables
                        .entry(function.span)
                        .or_default()
                        .returns,
                    implementation.retained_sources(),
                );
            }
        }
        for item in &program.items {
            if let Item::Interface(interface) = item {
                for requirement in &interface.requirements {
                    self.seed_pending_requirement_selection(requirement);
                }
            }
        }
        // Known bodies begin at the bottom of the source-summary lattice.
        // An opaque declaration's lifetime bound is not evidence that one of
        // those bodies actually creates such a dependency. Nor is a seeded
        // summary: it was computed by another pass under that pass's class
        // identities (the source pass numbers one template per generic class,
        // execution numbers each concrete instance), so joining it into a body
        // analyzed here would retain paths through unrelated classes.
        for (function, _) in &callables {
            if function.body.as_block().is_some() {
                self.retained.callables.remove(&function.span);
                self.retained.analyzed_callables.insert(function.span);
            }
        }
        for item in &program.items {
            let Item::Class(class) = item else { continue };
            for member in &class.members {
                let ClassMember::Method(method) = member else {
                    continue;
                };
                if !method.is_override {
                    continue;
                }
                for ancestor in self
                    .retained
                    .ancestors
                    .get(&class.name)
                    .into_iter()
                    .flatten()
                {
                    if let Some(base) = self
                        .methods
                        .get(&(ancestor.clone(), method.name.clone()))
                        .and_then(|signature| signature.declaration)
                    {
                        let implementations =
                            self.retained.implementations.entry(base).or_default();
                        if !implementations.contains(&method.span) {
                            implementations.push(method.span);
                        }
                    }
                }
            }
        }
        for closure in self.closures.values() {
            let span = Span {
                source: closure.closure_id.source,
                expansion: closure.closure_id.expansion,
                start: closure.closure_id.start,
                end: closure.closure_id.end,
            };
            self.retained.callables.remove(&span);
            self.retained.analyzed_callables.insert(span);
        }
        // Known callback dispatch uses the same joined body summaries as
        // virtual method dispatch, including definite removals and returned storage.
        for (&site, targets) in self.callback_capture_targets {
            let implementations = self.retained.implementations.entry(site).or_default();
            for target in targets {
                if !implementations.contains(&target.declaration) {
                    implementations.push(target.declaration);
                }
            }
        }
        let body_declarations = self.retained.analyzed_callables.clone();
        for (requirement, implementations) in &self.retained.implementations {
            if !implementations.is_empty()
                && implementations
                    .iter()
                    .all(|implementation| self.retained.analyzed_callables.contains(implementation))
            {
                self.retained.analyzed_callables.insert(*requirement);
            }
        }
        self.index_recursive_retained_calls(program);
        // Reuse the ownership pass's control-flow joins and canonical bindings.
        // Cyclic projected calls widen to their input subtree; finite paths stay exact.
        loop {
            let before = self.retained.callables.clone();
            for (function, receiver) in &callables {
                self.check_function(function, *receiver);
            }
            // A call through a checked requirement can select any of its
            // implementations. Keep their source dependencies at that boundary,
            // in the requirement's dispatch join. The requirement's own entry
            // stays its body: an open method's body and MIR plan retain only
            // what that body returns, never an override's properties.
            for (requirement, implementations) in &self.retained.implementations {
                let summaries = implementations
                    .iter()
                    .chain(
                        body_declarations
                            .contains(requirement)
                            .then_some(requirement),
                    )
                    .filter_map(|implementation| {
                        self.retained.callables.get(implementation).cloned()
                    })
                    .collect::<Vec<_>>();
                let mut summary = RetainedCallableInfo {
                    dispatch: None,
                    ..self
                        .retained
                        .callables
                        .get(requirement)
                        .cloned()
                        .unwrap_or_default()
                };
                summary.returned_storage = None;
                summary.removals = summaries
                    .first()
                    .map(|s| s.removals.clone())
                    .unwrap_or_default();
                summary.exceptional_removals = summaries
                    .first()
                    .map(|s| s.exceptional_removals.clone())
                    .unwrap_or_default();
                for candidate in summaries {
                    writeback::append(&mut summary.writeback.returns, &candidate.writeback.returns);
                    writeback::append(&mut summary.writeback.writes, &candidate.writeback.writes);
                    writeback::append(
                        &mut summary.writeback.invocations,
                        &candidate.writeback.invocations,
                    );
                    summary
                        .removals
                        .retain(|selection| candidate.removals.contains(selection));
                    summary
                        .exceptional_removals
                        .retain(|selection| candidate.exceptional_removals.contains(selection));
                    let mut storage = candidate.returned_storage.unwrap_or_else(|| {
                        owned_storage::ReturnedStorage {
                            direct: candidate.returns.clone(),
                            ..Default::default()
                        }
                    });
                    let mut returns = candidate.returns;
                    // An interface receiver is opaque, and a requirement's
                    // implementations can include classes this receiver is
                    // not, so a path into one of them names nothing here.
                    if self.retained.requirements.contains_key(requirement) {
                        whole_receiver_sources(&mut returns);
                        storage.retain_whole_receivers();
                    }
                    summary
                        .returned_storage
                        .get_or_insert_with(Default::default)
                        .merge(storage);
                    append_sources(&mut summary.returns, returns);
                    owned_storage::append_stores(
                        &mut summary.stored_sources,
                        candidate.stored_sources,
                    );
                    owned_storage::append_stores(
                        &mut summary.exceptional_stored_sources,
                        candidate.exceptional_stored_sources,
                    );
                    pending::append_selections(
                        &mut summary.pending_selections,
                        candidate.pending_selections,
                    );
                    for source in candidate.requires_independent {
                        if !summary.requires_independent.contains(&source) {
                            summary.requires_independent.push(source);
                        }
                    }
                    for source in candidate.payload_inputs {
                        if !summary.payload_inputs.contains(&source) {
                            summary.payload_inputs.push(source);
                        }
                    }
                }
                self.retained
                    .callables
                    .entry(*requirement)
                    .or_default()
                    .dispatch = Some(Box::new(summary));
            }
            self.diagnostics.clear();
            self.writeback.constraints.clear();
            self.analyzed_closures.clear();
            self.prepared_closure_evaluations.clear();
            self.closure_values.clear();
            self.closure_ownership.clear();
            self.retained.expression_values.clear();
            self.retained.failed_storage_calls.clear();
            if self.retained.callables == before {
                break;
            }
        }
    }

    fn index_recursive_retained_calls(&mut self, program: &ast::Program) {
        let mut graph = HashMap::<Span, HashSet<Span>>::new();
        let scopes = Scopes::default();
        let mut index = |function: &ast::FunctionDecl| {
            let Some(body) = function.body.as_block() else {
                return;
            };
            let edges = graph.entry(function.span).or_default();
            ast::visit::block(body, &mut |expr| {
                if let Some(declaration) = self
                    .retained_call(expr, &scopes)
                    .and_then(|(signature, _, _)| signature.declaration)
                {
                    edges.insert(declaration);
                }
            });
        };
        for (function, _) in program_callables(program) {
            index(&function);
        }
        for (effective, template) in &self.composition.callable_templates {
            graph
                .entry(*effective)
                .or_default()
                .insert(template.declaration);
        }
        for (requirement, implementations) in &self.retained.implementations {
            graph
                .entry(*requirement)
                .or_default()
                .extend(implementations);
        }
        for (&caller, callees) in &graph {
            for &callee in callees {
                let mut pending = vec![callee];
                let mut visited = HashSet::new();
                while let Some(next) = pending.pop() {
                    if next == caller {
                        self.retained.recursive_calls.insert((caller, callee));
                        break;
                    }
                    if visited.insert(next) {
                        pending.extend(graph.get(&next).into_iter().flatten().copied());
                    }
                }
            }
        }
    }

    pub(super) fn extend_retained_place(
        &self,
        place: &mut BorrowPlace,
        fields: &[crate::class_layout::PropertyId],
        signature: &Signature,
    ) {
        let recursive = self.retained.current.as_ref().is_some_and(|current| {
            signature.declaration.is_some_and(|callee| {
                self.retained
                    .recursive_calls
                    .contains(&(current.declaration, callee))
            })
        });
        if recursive && !place.fields.is_empty() && !fields.is_empty() {
            place.projectable = false;
        } else {
            place.extend(fields);
        }
    }

    pub(super) fn parameter_retained_value(
        &self,
        id: Option<BindingId>,
    ) -> Option<RetainedValueState> {
        let selector = self.parameter_storage_selector(id);
        let id = id?;
        let declaration = self.binding_resolution.declarations_by_id.get(&id)?;
        let ty = declaration.source_type.as_ref()?;
        // Symbolic input contents may carry loans, independently of whether
        // the callable returns ownership or borrows the input's own storage.
        let symbolic_owner = crate::types::resolved_type_is_symbolic(ty)
            && self.retained.current.as_ref().is_some_and(|current| {
                current
                    .parameters
                    .iter()
                    .any(|(parameter, take)| *parameter == Some(id) && *take)
            });
        let carries_source = self.retained.carries_source(ty)
            || symbolic_owner
            || matches!(ty, ResolvedType::TypeParameter(name)
            if self.current_type_params.iter().any(|parameter| parameter.name == *name && parameter.constraints.iter()
                .any(|constraint| self.retained.source_types.interfaces.iter().any(|interface| interface.name == constraint.name))));
        let value = carries_source.then(|| {
            let mut value = iterator_value(vec![RetainedLoan {
                root: BorrowRoot::Binding(id),
                root_key: format!("retained-parameter:{}", id.0).into(),
                access: if crate::types::ByteRegionKind::from_resolved(ty)
                    .is_some_and(crate::types::ByteRegionKind::is_writable)
                {
                    BorrowAccess::Writable
                } else {
                    BorrowAccess::Readonly
                },
                capture_span: declaration.span.unwrap_or_default(),
                source_depth: 0,
                inherited: true,
                lifetime: if crate::types::ByteRegionKind::from_resolved(ty).is_some() {
                    RetainedLifetime::LastUse
                } else {
                    RetainedLifetime::Cleanup
                },
                reborrows: Vec::new(),
                pending_capture: None,
            }]);
            if let Some(current) = self.retained.current.as_ref().filter(|_| {
                non_null_function_type(ty).is_none()
                    && crate::types::ByteRegionKind::from_resolved(ty).is_none()
                    && (resolved_type_is_move_type(ty, &self.move_enum_names)
                        || resolved_type_requires_conservative_move(ty))
            }) {
                if let Some(index) = current
                    .parameters
                    .iter()
                    .position(|(parameter, take)| *parameter == Some(id) && *take)
                {
                    value.storage.input(BorrowSource::Parameter(index));
                }
            }
            value
        });
        join_retained_value(value.as_ref(), selector.as_ref())
    }

    pub(super) fn retained_call<'a>(
        &self,
        expr: &'a Expr,
        scopes: &Scopes,
    ) -> Option<(Signature, Option<&'a Expr>, &'a [Argument])> {
        let (receiver, args) = match ungroup_expr(expr) {
            Expr::PropertyAccess { object, .. } => {
                let mut signature = self.getter_signature(expr)?;
                self.specialize_carrier_return(expr, &mut signature);
                self.specialize_requirement_return(expr, &mut signature);
                return Some((signature, Some(object), &[]));
            }
            Expr::New {
                class_type, args, ..
            } => {
                return Some((self.constructors.get(&class_type.name)?.clone(), None, args));
            }
            Expr::CallableCall { args, span, .. } => {
                let call = self.callable_value_calls.get(span)?;
                let function = non_null_function_type(&call.function_type)?;
                let mut signature = Signature::for_function_value(
                    function,
                    self.retained
                        .implementations
                        .contains_key(span)
                        .then_some(*span),
                    &self.move_enum_names,
                );
                self.specialize_copy_parameter_return(expr, args, &mut signature);
                return Some((signature, None, args));
            }
            Expr::FunctionCall { args, .. } | Expr::StaticCall { args, .. } => (None, args),
            Expr::MethodCall {
                object,
                method,
                args,
                ..
            } => {
                if let Some(intrinsic) = self
                    .resolved_type(object)
                    .and_then(|ty| {
                        let ty = match ty {
                            ResolvedType::Nullable(inner) => inner,
                            _ => ty,
                        };
                        crate::byte_intrinsics::member(ty, method)
                    })
                    .filter(|member| member.kind == crate::byte_intrinsics::ByteMemberKind::Method)
                {
                    return Some((
                        Signature {
                            params: intrinsic
                                .parameters
                                .into_iter()
                                .map(|parameter| Parameter {
                                    name: parameter.name.to_string(),
                                    move_type: false,
                                    class_type: false,
                                    generic: false,
                                    take: false,
                                    writable: false,
                                    borrow: false,
                                })
                                .collect(),
                            receiver: Some(
                                if intrinsic.receiver_mode
                                    == Some(crate::symbols::ReceiverMode::Writable)
                                {
                                    UseMode::Write
                                } else {
                                    UseMode::Read
                                },
                            ),
                            returns_move_type: resolved_type_is_move_type(
                                &intrinsic.return_type,
                                &self.move_enum_names,
                            ),
                            ..Signature::default()
                        },
                        Some(object),
                        args,
                    ));
                }
                (Some(object.as_ref()), args)
            }
            _ => return None,
        };
        let signature = match self.call_targets.get(&expr.span()) {
            Some(crate::semantics::CallableTarget::Function { name }) => self.signatures.get(name),
            Some(crate::semantics::CallableTarget::Method {
                class_type,
                method_name,
                ..
            }) => self.retained_method(&class_type.name, method_name),
            Some(crate::semantics::CallableTarget::ConstrainedMethod {
                receiver: ResolvedType::TraitSelf(owner),
                method_name,
                ..
            }) => self.retained_method(owner, method_name),
            Some(
                crate::semantics::CallableTarget::InterfaceMethod { requirement, .. }
                | crate::semantics::CallableTarget::ConstrainedMethod { requirement, .. },
            ) => self.retained.requirements.get(requirement),
            _ => match ungroup_expr(expr) {
                Expr::FunctionCall { name, .. } => self.signatures.get(name),
                Expr::MethodCall { object, method, .. } => self
                    .expr_class(object, scopes)
                    .and_then(|class| self.retained_method(&class, method)),
                Expr::StaticCall {
                    qualifier, method, ..
                } => self
                    .qualifier_class(qualifier)
                    .and_then(|class| self.methods.get(&(class, method.clone()))),
                _ => None,
            },
        }?;
        let mut signature = signature.clone();
        self.specialize_carrier_return(expr, &mut signature);
        self.specialize_copy_parameter_return(expr, args, &mut signature);
        self.specialize_requirement_return(expr, &mut signature);
        Some((signature, receiver, args))
    }

    /// Whether this call reaches a bodyless requirement whose contract decides
    /// its result. An interface requirement never has a body; a trait member may.
    fn calls_requirement(&self, expr: &Expr) -> bool {
        let target = self.call_targets.get(&expr.span()).or_else(|| {
            self.property_accessor_calls
                .get(&ungroup_expr(expr).span())
                .and_then(|calls| calls.getter.as_ref())
                .map(|getter| &getter.target)
        });
        let Some(
            crate::semantics::CallableTarget::InterfaceMethod { requirement, .. }
            | crate::semantics::CallableTarget::ConstrainedMethod { requirement, .. },
        ) = target
        else {
            return false;
        };
        (self.retained.requirements.contains_key(requirement)
            || self.retained.trait_requirements.contains(requirement))
            && !self
                .retained
                .argument_typed_requirements
                .contains(requirement)
    }

    /// A call through a requirement carries the loan that requirement elides
    /// (Decision 0134). The call's own type decides it, because a generic
    /// requirement returns a region only where its `T` is one.
    fn specialize_requirement_return(&self, expr: &Expr, signature: &mut Signature) {
        if !self.calls_requirement(expr) {
            return;
        }
        let region = self
            .resolved_type(expr)
            .and_then(crate::types::ByteRegionKind::from_resolved);
        if region.is_some() {
            signature.return_borrow = super::requirement_return_borrow(
                region,
                signature.receiver.map(|mode| match mode {
                    UseMode::Read => crate::symbols::ReceiverMode::Readonly,
                    UseMode::Write => crate::symbols::ReceiverMode::Writable,
                    UseMode::Give => crate::symbols::ReceiverMode::Consuming,
                }),
            );
        }
    }

    /// The source a call through a region-returning requirement retains: the
    /// whole receiver, whichever implementation the call selects.
    fn requirement_region_source(
        &self,
        expr: &Expr,
        signature: &Signature,
    ) -> Option<RetainedSource> {
        if !self.calls_requirement(expr) {
            return None;
        }
        self.resolved_type(expr)
            .and_then(crate::types::ByteRegionKind::from_resolved)?;
        let borrow = signature.return_borrow?;
        Some(RetainedSource {
            source: borrow.source,
            inherited: false,
            writable: borrow.writable,
            lifetime: borrow.lifetime,
            owned_path: Vec::new(),
        })
    }

    fn specialize_carrier_return(&self, expr: &Expr, signature: &mut Signature) {
        let Some(declaration) = signature.declaration else {
            return;
        };
        let substitutions = self.call_type_substitutions(expr.span(), signature);
        if let Some(borrow) = crate::semantics::specialized_return_borrow(
            self.specialized_return_borrows,
            declaration,
            &substitutions,
        ) {
            signature.return_borrow = borrow;
            if let Some(ty) = self
                .resolved_type(expr)
                .filter(|ty| !resolved_type_requires_conservative_move(ty))
            {
                signature.returns_move_type = resolved_type_is_move_type(ty, &self.move_enum_names);
            }
        }
    }

    fn specialize_copy_parameter_return(
        &self,
        expr: &Expr,
        args: &[Argument],
        signature: &mut Signature,
    ) {
        let Some(borrow) = signature
            .return_borrow
            .filter(|borrow| borrow.kind == ReturnBorrowKind::Value)
        else {
            return;
        };
        let BorrowSource::Parameter(index) = borrow.source else {
            return;
        };
        let Some(parameter) = signature.params.get(index) else {
            return;
        };
        if parameter.move_type {
            return;
        }
        let copy = if parameter.generic {
            self.call_borrow_source_expr(borrow, None, signature, args)
                .and_then(|source| self.resolved_type(source))
                .is_some_and(|ty| {
                    !resolved_type_is_move_type(ty, &self.move_enum_names)
                        && !resolved_type_requires_conservative_move(ty)
                })
        } else {
            true
        };
        if copy {
            signature.return_borrow = None;
            signature.returns_move_type = self
                .resolved_type(expr)
                .is_some_and(|ty| resolved_type_is_move_type(ty, &self.move_enum_names));
        }
    }

    fn retained_method(&self, class: &str, method: &str) -> Option<&Signature> {
        std::iter::once(class)
            .chain(
                self.retained
                    .ancestors
                    .get(class)
                    .into_iter()
                    .flatten()
                    .map(String::as_str),
            )
            .find_map(|class| self.methods.get(&(class.to_owned(), method.to_owned())))
    }

    fn constructor_sources(&self, signature: &Signature) -> Vec<RetainedSource> {
        let mut sources = signature
            .declaration
            .and_then(|declaration| self.retained.callables.get(&declaration))
            .map(|info| info.for_callers().constructs.clone())
            .unwrap_or_default();
        for (index, parameter) in signature
            .params
            .iter()
            .enumerate()
            .filter(|(_, parameter)| parameter.borrow)
        {
            let source = RetainedSource {
                source: BorrowSource::Parameter(index),
                inherited: false,
                writable: parameter.writable,
                lifetime: RetainedLifetime::Cleanup,
                owned_path: Vec::new(),
            };
            append_sources(&mut sources, [source]);
        }
        sources
    }

    pub(super) fn retain_parent_constructor_sources(
        &mut self,
        site: Span,
        signature: &Signature,
        args: &[Argument],
        scopes: &Scopes,
    ) {
        if !self
            .retained
            .current
            .as_ref()
            .is_some_and(|current| current.constructor)
        {
            return;
        }
        let loans = self
            .retained_call_loans(
                &self.constructor_sources(signature),
                site,
                None,
                args,
                signature,
                scopes,
            )
            .unwrap_or_default();
        let mut sources = Vec::new();
        for loan in loans {
            if let Some(source) = self.parameter_source(&loan) {
                append_sources(&mut sources, [source]);
            } else {
                self.diagnostics.push(iterator_escape(
                    site,
                    loan.capture_span,
                    "constructed owner would outlive its constructor-local source",
                ));
            }
        }
        let current = self.retained.current.as_ref().expect("constructor context");
        let info = self
            .retained
            .callables
            .entry(current.declaration)
            .or_default();
        append_sources(&mut info.constructs, sources);
    }

    pub(super) fn iterator_value_from_expr(
        &self,
        expr: &Expr,
        scopes: &Scopes,
    ) -> Option<RetainedValueState> {
        if let Some(value) = self.retained.expression_values.get(&expr.span()) {
            return Some(value.clone());
        }
        if matches!(ungroup_expr(expr), Expr::This { .. }) {
            return self
                .receiver_class
                .as_ref()
                .filter(|class| {
                    self.retained
                        .source_types
                        .classes
                        .iter()
                        .any(|source| source.name == **class)
                })
                .map(|_| {
                    if let Some(value) = scopes
                        .get("this")
                        .and_then(|binding| binding.retained_value.clone())
                    {
                        return value;
                    }
                    let mut value = iterator_value(vec![RetainedLoan {
                        root: BorrowRoot::Receiver,
                        root_key: "retained-receiver".into(),
                        access: BorrowAccess::Readonly,
                        capture_span: expr.span(),
                        source_depth: 0,
                        inherited: true,
                        lifetime: RetainedLifetime::Cleanup,
                        reborrows: Vec::new(),
                        pending_capture: None,
                    }]);
                    if self
                        .retained
                        .current
                        .as_ref()
                        .is_some_and(|current| current.owns_receiver)
                    {
                        value.storage.input(BorrowSource::Receiver);
                    }
                    value
                });
        }
        if let Expr::Binary {
            left,
            op: BinaryOp::Coalesce,
            right,
            ..
        } = ungroup_expr(expr)
        {
            match self.flow_fact(left) {
                Some(Fact::NonNull | Fact::Exact(_) | Fact::Constructed { .. }) => {
                    return self.retained_value_from_expr(left, scopes);
                }
                Some(Fact::Null) => return self.retained_value_from_expr(right, scopes),
                None if matches!(ungroup_expr(left), Expr::Null { .. }) => {
                    return self.retained_value_from_expr(right, scopes);
                }
                None => {}
            }
            return join_retained_value(
                self.retained_value_from_expr(left, scopes).as_ref(),
                self.retained_value_from_expr(right, scopes).as_ref(),
            );
        }
        let viewed = match ungroup_expr(expr) {
            Expr::MethodCall { object, method, .. } => self
                .resolved_type(object)
                .and_then(|ty| crate::types::ByteRegionKind::method_result(ty, method))
                .map(|kind| (object, kind)),
            Expr::PropertyAccess {
                object, property, ..
            } => self
                .resolved_type(object)
                .and_then(|ty| crate::types::ByteRegionKind::property_result(ty, property))
                .map(|kind| (object, kind)),
            _ => None,
        };
        if let Some((object, kind)) = viewed {
            let access = if kind.is_writable() {
                BorrowAccess::Writable
            } else {
                BorrowAccess::Readonly
            };
            let loans = self.reborrow_source_loans(object, scopes, access);
            return Some(iterator_value(loans));
        }
        if self
            .resolved_type(expr)
            .is_some_and(|ty| self.retained.carries_source(ty))
        {
            let collection = match ungroup_expr(expr) {
                Expr::Index { collection, .. } => Some(collection.as_ref()),
                Expr::MethodCall { object, method, .. }
                    if self
                        .expr_collection_info(object, scopes)
                        .is_some_and(|info| {
                            info.family.borrows_element(method)
                                || info.family.removes_element(method)
                        }) =>
                {
                    Some(object.as_ref())
                }
                Expr::PropertyAccess {
                    object, property, ..
                } if self
                    .expr_collection_info(object, scopes)
                    .is_some_and(|info| info.family.borrows_element(property)) =>
                {
                    Some(object.as_ref())
                }
                _ => None,
            };
            if let Some(collection) = collection {
                // Opaque selection retains candidate partitions. Discharging a
                // collection member requires a separate, proved removal.
                return self.retained_value_from_expr(collection, scopes);
            }
        }
        if let Expr::CallableCall { args, span, .. } = ungroup_expr(expr) {
            if !self.retained.implementations.contains_key(span) {
                let call = self.callable_value_calls.get(span)?;
                let function = non_null_function_type(&call.function_type)?;
                let returned = function.return_borrow?;
                let crate::types::FunctionBorrowSource::Parameter(index) = returned.source;
                let input = &args.get(index)?.value;
                if returned.kind == ReturnBorrowKind::Value {
                    // A returned carrier preserves its existing loans; a borrowed
                    // leaf from that carrier depends only on ordinary borrow roots.
                    if !self.retained.carries_source(&function.return_type)
                        && non_null_function_type(&function.return_type).is_none()
                        && !crate::types::resolved_type_is_symbolic(&function.return_type)
                    {
                        return None;
                    }
                    return self.retained_value_from_expr(input, scopes);
                }
                if !self.retained.carries_source(&function.return_type)
                    && !crate::types::resolved_type_is_symbolic(&function.return_type)
                {
                    return None;
                }
                let access = if returned.writable {
                    BorrowAccess::Writable
                } else {
                    BorrowAccess::Readonly
                };
                let consumes_source = function.parameters.get(index)?.ownership_mode
                    == crate::types::FunctionTypeParameterMode::Take;
                if consumes_source {
                    return self.retained_value_from_expr(input, scopes);
                }
                let mut loans = self.reborrow_source_loans(input, scopes, access);
                if !consumes_source {
                    for loan in &mut loans {
                        loan.lifetime = returned.lifetime;
                    }
                }
                return Some(iterator_value(loans));
            }
        }
        let (signature, receiver, args) = self.retained_call(expr, scopes)?;
        // One join covers every specialization of a generic requirement. A
        // call whose own result cannot carry a loan keeps none of them.
        if self.calls_requirement(expr)
            && self.resolved_type(expr).is_some_and(|ty| {
                !self.retained.carries_source(ty) && !crate::types::resolved_type_is_symbolic(ty)
            })
        {
            return None;
        }
        // The contract is all a caller can rely on. Paths from the dispatch join
        // would name properties of implementers this receiver may not be.
        if let Some(declared) = self.requirement_region_source(expr, &signature) {
            return self
                .retained_call_loans(&[declared], expr.span(), receiver, args, &signature, scopes)
                .map(iterator_value);
        }
        if !matches!(ungroup_expr(expr), Expr::New { .. }) {
            let info = signature
                .declaration
                .and_then(|declaration| self.retained.callables.get(&declaration))
                .map(RetainedCallableInfo::for_callers);
            let mut storage = info.and_then(|info| info.returned_storage.clone());
            for candidate in signature
                .declaration
                .and_then(|declaration| self.retained.implementations.get(&declaration))
                .into_iter()
                .flatten()
                .filter_map(|candidate| self.retained.callables.get(candidate))
            {
                if storage.is_none() && candidate.returned_storage.is_some() {
                    storage = Some(owned_storage::ReturnedStorage {
                        direct: info.map(|info| info.returns.clone()).unwrap_or_default(),
                        ..Default::default()
                    });
                }
                if let Some(storage) = &mut storage {
                    storage.merge(candidate.returned_storage.clone().unwrap_or_else(|| {
                        owned_storage::ReturnedStorage {
                            direct: candidate.returns.clone(),
                            ..Default::default()
                        }
                    }));
                }
            }
            if let Some(storage) = &storage {
                return self.returned_storage_from_call(
                    storage,
                    expr.span(),
                    receiver,
                    args,
                    &signature,
                    scopes,
                );
            }
        }
        let sources = if matches!(ungroup_expr(expr), Expr::New { .. }) {
            self.constructor_sources(&signature)
        } else {
            signature
                .declaration
                .and_then(|declaration| self.retained.callables.get(&declaration))
                .map(|info| info.for_callers().returns.clone())
                .unwrap_or_default()
        };
        if sources.is_empty() {
            if signature
                .return_borrow
                .is_some_and(|borrow| borrow.kind == ReturnBorrowKind::Retained)
                && signature.declaration.is_some_and(|declaration| {
                    self.retained.analyzed_callables.contains(&declaration)
                })
            {
                return Some(iterator_value(Vec::new()));
            }
            return None;
        }
        self.retained_call_loans(&sources, expr.span(), receiver, args, &signature, scopes)
            .map(iterator_value)
    }

    pub(super) fn retained_call_loans(
        &self,
        sources: &[RetainedSource],
        site: Span,
        receiver: Option<&Expr>,
        args: &[Argument],
        signature: &Signature,
        scopes: &Scopes,
    ) -> Option<Vec<RetainedLoan>> {
        let mut loans = Vec::new();
        for source in sources {
            let input = self.call_borrow_source_expr(
                ReturnBorrow {
                    kind: ReturnBorrowKind::Retained,
                    source: source.source,
                    writable: source.writable,
                    lifetime: source.lifetime,
                },
                receiver,
                signature,
                args,
            )?;
            if source.inherited {
                let region = self
                    .resolved_types
                    .get(&site)
                    .and_then(crate::types::ByteRegionKind::from_resolved);
                let consumes_source = match source.source {
                    BorrowSource::Receiver => signature.receiver == Some(UseMode::Give),
                    BorrowSource::Parameter(index) => {
                        signature.params.get(index).is_some_and(|param| param.take)
                    }
                };
                if region.is_some() && !consumes_source {
                    let access = if source.writable {
                        BorrowAccess::Writable
                    } else {
                        BorrowAccess::Readonly
                    };
                    let mut inherited = self.reborrow_source_loans(input, scopes, access);
                    for loan in &mut inherited {
                        loan.lifetime = source.lifetime;
                        self.extend_retained_place(
                            &mut loan.root_key,
                            &source.owned_path,
                            signature,
                        );
                    }
                    append_loans(&mut loans, &inherited);
                } else if let Some(value) = self.retained_value_from_expr(input, scopes) {
                    let mut inherited = value.leases;
                    if !consumes_source {
                        for loan in &mut inherited {
                            loan.lifetime = source.lifetime;
                        }
                    }
                    for loan in &mut inherited {
                        self.extend_retained_place(
                            &mut loan.root_key,
                            &source.owned_path,
                            signature,
                        );
                    }
                    append_loans(&mut loans, &inherited);
                }
            } else {
                let access = if source.writable {
                    BorrowAccess::Writable
                } else {
                    BorrowAccess::Readonly
                };
                for mut loan in self.retained_source_loans(input, scopes, access) {
                    loan.lifetime = source.lifetime;
                    self.extend_retained_place(&mut loan.root_key, &source.owned_path, signature);
                    append_loans(&mut loans, &[loan]);
                }
            }
        }
        Some(loans)
    }

    pub(super) fn reborrow_source_loans(
        &self,
        input: &Expr,
        scopes: &Scopes,
        access: BorrowAccess,
    ) -> Vec<RetainedLoan> {
        if let Some(value) = self.retained_value_from_expr(input, scopes) {
            let mut loans = value.leases;
            if self
                .resolved_type(input)
                .and_then(crate::types::ByteRegionKind::from_resolved)
                .is_some_and(|kind| !kind.is_writable())
            {
                return loans;
            }
            for key in self.borrow_root_keys(input, scopes) {
                for loan in &mut loans {
                    let reborrow = (key.clone(), access);
                    if !loan.reborrows.contains(&reborrow) {
                        loan.reborrows.push(reborrow);
                    }
                }
            }
            loans
        } else {
            self.retained_source_loans(input, scopes, access)
        }
    }

    pub(super) fn retained_source_loans(
        &self,
        expr: &Expr,
        scopes: &Scopes,
        access: BorrowAccess,
    ) -> Vec<RetainedLoan> {
        self.retained_loans_at_source(
            self.borrow_source_root(expr, scopes),
            expr.span(),
            scopes,
            access,
        )
    }

    pub(super) fn retained_loans_at_source(
        &self,
        source: Option<BorrowSourceRoot>,
        span: Span,
        scopes: &Scopes,
        access: BorrowAccess,
    ) -> Vec<RetainedLoan> {
        if access == BorrowAccess::Readonly && source == Some(BorrowSourceRoot::ImmutableNull) {
            return Vec::new();
        }
        if let Some(source) = source.filter(|source| !source.places().is_empty()) {
            return source
                .places()
                .iter()
                .map(|key| self.retained_loan_for_place(Some(key.clone()), span, scopes, access))
                .collect();
        }
        vec![self.retained_loan_for_place(None, span, scopes, access)]
    }

    fn retained_loan_for_place(
        &self,
        key: Option<BorrowPlace>,
        span: Span,
        scopes: &Scopes,
        access: BorrowAccess,
    ) -> RetainedLoan {
        let binding = key.as_ref().and_then(|key| {
            scopes
                .0
                .iter()
                .rev()
                .flat_map(|scope| scope.iter())
                .find_map(|(name, binding)| {
                    let binding = binding.as_ref()?;
                    let captured_place = binding
                        .canonical_id
                        .and_then(|id| self.binding_resolution.declarations_by_id.get(&id))
                        .is_some_and(|declaration| declaration.kind == BindingKind::ClosureCapture)
                        && binding
                            .borrow_root
                            .as_ref()
                            .is_some_and(|source| source.places().contains(key));
                    (binding_identity_key(binding.id, name) == key.owner || captured_place)
                        .then_some(binding)
                })
        });
        let (root, source_depth) = if key.as_ref().is_some_and(|place| place.owner == "$this") {
            (BorrowRoot::Receiver, 0)
        } else if let Some(binding) = binding {
            (
                binding
                    .canonical_id
                    .map(BorrowRoot::Binding)
                    .unwrap_or(BorrowRoot::Temporary),
                binding.scope_depth,
            )
        } else {
            (BorrowRoot::Temporary, scopes.lexical_depth() + 1)
        };
        RetainedLoan {
            root,
            root_key: key.unwrap_or_else(|| format!("temporary:{}", span.start).into()),
            access,
            capture_span: span,
            source_depth,
            inherited: false,
            lifetime: RetainedLifetime::LastUse,
            reborrows: Vec::new(),
            pending_capture: None,
        }
    }

    pub(super) fn parameter_source(&self, loan: &RetainedLoan) -> Option<RetainedSource> {
        let current = self.retained.current.as_ref()?;
        let source = match loan.root {
            BorrowRoot::Receiver
                if !current.closure && (!current.owns_receiver || loan.inherited) =>
            {
                BorrowSource::Receiver
            }
            BorrowRoot::Binding(id) => {
                let (index, (_, take)) = current
                    .parameters
                    .iter()
                    .enumerate()
                    .find(|(_, (parameter, _))| *parameter == Some(id))?;
                if *take && !loan.inherited {
                    return None;
                }
                BorrowSource::Parameter(index)
            }
            BorrowRoot::Receiver | BorrowRoot::Temporary | BorrowRoot::EnclosingEnvironment(_) => {
                return None;
            }
        };
        Some(RetainedSource {
            source,
            inherited: loan.inherited,
            writable: loan.access == BorrowAccess::Writable,
            lifetime: loan.lifetime,
            owned_path: self.retained.owning_prefix(&loan.root_key.fields),
        })
    }

    pub(super) fn record_returned_sources(&mut self, value: &RetainedValueState) {
        let sources = value
            .leases
            .iter()
            .filter_map(|loan| self.parameter_source(loan))
            .collect::<Vec<_>>();
        let Some(current) = &self.retained.current else {
            return;
        };
        let info = self
            .retained
            .callables
            .entry(current.declaration)
            .or_default();
        append_sources(&mut info.returns, sources);
    }

    pub(super) fn validate_returned_iterator(
        &mut self,
        value: &RetainedValueState,
        span: Span,
    ) -> bool {
        let mut sources = Vec::new();
        for loan in &value.leases {
            if self
                .retained
                .current
                .as_ref()
                .is_some_and(|current| current.closure)
                && self.loan_is_from_closure_environment(loan)
            {
                self.diagnostics.push(Diagnostic::new(
                    "E0653",
                    "returning a borrow from a captured environment is not yet supported (Decision 0121)",
                    span,
                ).with_title("Captured Environment Borrow Cannot Be Returned")
                 .with_help("pass the source as a typed argument, or return an independently owned value"));
                return false;
            }
            let Some(mut source) = self.parameter_source(loan) else {
                self.diagnostics.push(iterator_escape(
                    span,
                    loan.capture_span,
                    "returned borrow-bound value would outlive its source",
                ));
                return false;
            };
            if let Some(kind) = self
                .resolved_types
                .get(&span)
                .and_then(crate::types::ByteRegionKind::from_resolved)
            {
                source.writable = kind.is_writable();
            }
            append_sources(&mut sources, [source]);
        }
        if let Some(current) = self
            .retained
            .current
            .as_ref()
            .filter(|current| current.closure)
        {
            if sources.iter().any(|source| {
                current.return_borrow.is_none_or(|borrow| {
                    let crate::types::FunctionBorrowSource::Parameter(index) = borrow.source;
                    source.source != BorrowSource::Parameter(index)
                })
            }) {
                self.diagnostics.push(Diagnostic::new(
                    "E0659",
                    "returned borrow-bound value requires one proven argument source in the closure type",
                    span,
                ).with_title("Closure Has Multiple Incompatible Return Roots")
                 .with_help("return a view tied to one borrowed argument; general callable lifetime contracts remain deferred by Decision 0121"));
                return false;
            }
        }
        if let Some(current) = &self.retained.current {
            let info = self
                .retained
                .callables
                .entry(current.declaration)
                .or_default();
            if !current.closure
                && self
                    .resolved_types
                    .get(&span)
                    .and_then(crate::types::ByteRegionKind::from_resolved)
                    .is_some()
            {
                let mut roots = info.returns.iter().chain(&sources).map(|loan| loan.source);
                if roots
                    .next()
                    .is_some_and(|root| roots.any(|other| other != root))
                {
                    self.diagnostics.push(Diagnostic::new(
                        "E0478",
                        "returned byte region has multiple incompatible source roots",
                        span,
                    ).with_title("Returned Borrow Has Ambiguous Source")
                     .with_help("return a region tied to `$this` or to one borrowed parameter, as required by Decision 0089"));
                    return false;
                }
            }
            append_sources(&mut info.returns, sources);
        }
        true
    }

    fn loan_is_from_closure_environment(&self, loan: &RetainedLoan) -> bool {
        match loan.root {
            BorrowRoot::EnclosingEnvironment(_) | BorrowRoot::Receiver => true,
            BorrowRoot::Binding(id) => self
                .binding_resolution
                .declarations_by_id
                .get(&id)
                .is_some_and(|declaration| {
                    matches!(declaration.owner, crate::symbols::LexicalOwner::Closure(_))
                        && matches!(
                            declaration.kind,
                            BindingKind::ClosureCapture | BindingKind::MethodReceiver
                        )
                }),
            BorrowRoot::Temporary => false,
        }
    }

    pub(super) fn validate_iterator_storage(
        &mut self,
        value: &RetainedValueState,
        span: Span,
        destination: &str,
    ) -> bool {
        for loan in &value.leases {
            if loan.inherited {
                if let Some(source) = self.parameter_source(loan) {
                    if let Some(current) = &self.retained.current {
                        let info = self
                            .retained
                            .callables
                            .entry(current.declaration)
                            .or_default();
                        if !info.requires_independent.contains(&source.source) {
                            info.requires_independent.push(source.source);
                        }
                        continue;
                    }
                }
            }
            self.diagnostics.push(iterator_escape(
                span,
                loan.capture_span,
                &format!("borrow-bound value cannot be retained in {destination}"),
            ));
            return false;
        }
        true
    }

    /// An opaque input's incoming dependencies reached an enum payload; the
    /// caller decides whether its actual argument can travel there.
    pub(super) fn record_payload_inputs(&mut self, value: &RetainedValueState) {
        let Some(declaration) = self
            .retained
            .current
            .as_ref()
            .map(|current| current.declaration)
        else {
            return;
        };
        let sources = value
            .leases
            .iter()
            .filter(|loan| loan.inherited)
            .filter_map(|loan| self.parameter_source(loan))
            .map(|source| source.source)
            .collect::<Vec<_>>();
        let info = self.retained.callables.entry(declaration).or_default();
        for source in sources {
            if !info.payload_inputs.contains(&source) {
                info.payload_inputs.push(source);
            }
        }
    }

    pub(super) fn validate_constructed_property_storage(
        &mut self,
        value: &RetainedValueState,
        span: Span,
        destination: &str,
        on_receiver: bool,
    ) -> bool {
        if on_receiver
            && value.kind == RetainedValueKind::SourceCarrier
            && self
                .retained
                .current
                .as_ref()
                .is_some_and(|current| current.constructor)
        {
            let mut sources = Vec::new();
            for loan in &value.leases {
                if let Some(source) = loan
                    .inherited
                    .then(|| self.parameter_source(loan))
                    .flatten()
                {
                    append_sources(&mut sources, [source]);
                } else {
                    return self.validate_owned_function_storage(value, span, destination);
                }
            }
            // Construction transfers incoming dependencies to the new owner;
            // it does not store them in an independently surviving receiver.
            let declaration = self.retained.current.as_ref().unwrap().declaration;
            let info = self.retained.callables.entry(declaration).or_default();
            append_sources(&mut info.constructs, sources);
            return true;
        }
        self.validate_owned_function_storage(value, span, destination)
    }

    pub(super) fn validate_retained_call_inputs(
        &mut self,
        receiver: Option<&Expr>,
        args: &[Argument],
        signature: &Signature,
        scopes: &Scopes,
    ) {
        let (required, payload_inputs) = signature
            .declaration
            .and_then(|declaration| self.retained.callables.get(&declaration))
            .map(RetainedCallableInfo::for_callers)
            .map(|info| {
                (
                    info.requires_independent.clone(),
                    info.payload_inputs.clone(),
                )
            })
            .unwrap_or_default();
        for source in payload_inputs {
            let Some(input) = self.call_borrow_source_expr(
                ReturnBorrow {
                    kind: ReturnBorrowKind::Retained,
                    source,
                    writable: false,
                    lifetime: RetainedLifetime::Cleanup,
                },
                receiver,
                signature,
                args,
            ) else {
                continue;
            };
            if let Some(value) = self
                .retained_value_from_expr(input, scopes)
                .filter(|value| value.kind == RetainedValueKind::Closure)
            {
                self.validate_owned_function_storage(&value, input.span(), "an enum payload");
            }
        }
        for source in required {
            let Some(input) = self.call_borrow_source_expr(
                ReturnBorrow {
                    kind: ReturnBorrowKind::Retained,
                    source,
                    writable: false,
                    lifetime: RetainedLifetime::Cleanup,
                },
                receiver,
                signature,
                args,
            ) else {
                continue;
            };
            if let Some(value) = self.retained_value_from_expr(input, scopes) {
                self.validate_owned_function_storage(
                    &value,
                    input.span(),
                    "storage retained by the callee",
                );
            }
        }
    }
}

pub(super) fn loan_provenance(leases: &[RetainedLoan]) -> ValueProvenance {
    let mut roots = leases
        .iter()
        .map(|loan| loan.root.clone())
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();
    if roots.is_empty() {
        ValueProvenance::Owned
    } else {
        ValueProvenance::BorrowBound(roots)
    }
}

pub(super) fn iterator_value(leases: Vec<RetainedLoan>) -> RetainedValueState {
    RetainedValueState {
        kind: RetainedValueKind::SourceCarrier,
        closure_id: None,
        provenance: loan_provenance(&leases),
        leases,
        nonescaping_parameter: false,
        take_parameter_insertion: None,
        storage: owned_storage::Contents::default(),
        enum_payloads: None,
    }
}

pub(super) fn append_loans(destination: &mut Vec<RetainedLoan>, incoming: &[RetainedLoan]) {
    for loan in incoming {
        if let Some(found) = destination.iter_mut().find(|found| found.same_source(loan)) {
            found.lifetime = found.lifetime.join(loan.lifetime);
        } else {
            destination.push(loan.clone());
        }
    }
}

pub(super) fn whole_receiver_sources(sources: &mut [RetainedSource]) {
    for source in sources {
        if source.source == BorrowSource::Receiver {
            source.owned_path.clear();
        }
    }
}

pub(super) fn append_sources(
    destination: &mut Vec<RetainedSource>,
    incoming: impl IntoIterator<Item = RetainedSource>,
) {
    for source in incoming {
        if let Some(found) = destination.iter_mut().find(|found| {
            found.source == source.source
                && found.inherited == source.inherited
                && found.owned_path == source.owned_path
        }) {
            found.writable |= source.writable;
            found.lifetime = found.lifetime.join(source.lifetime);
        } else {
            destination.push(source);
        }
    }
}

pub(super) fn iterator_escape(span: Span, source: Span, message: &str) -> Diagnostic {
    Diagnostic::new("E0762", message, span)
        .with_title("Value Cannot Outlive Borrowed Source")
        .with_related(source, "Source Loan Starts Here")
        .with_help("keep the borrowed value within its source's lifetime, or build an independently owned value")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_seeds_preserve_opaque_declaration_knowledge() {
        let opaque = Span {
            start: 1,
            end: 2,
            ..Span::default()
        };
        let authored = Span {
            start: 3,
            end: 4,
            ..Span::default()
        };
        let effective = authored.in_expansion(crate::source::ExpansionId(1));
        let missing = authored.in_expansion(crate::source::ExpansionId(2));
        let mut composition = crate::trait_composition::CompositionPlan::default();
        for declaration in [effective, missing] {
            composition.callable_templates.insert(
                declaration,
                crate::trait_composition::CallableTemplate {
                    declaration: authored,
                    bindings: HashMap::new(),
                    receiver: ResolvedType::TraitSelf("Owner".into()),
                },
            );
        }
        let summaries = HashMap::from([
            (opaque, RetainedCallableInfo::default()),
            (effective, RetainedCallableInfo::default()),
        ]);
        let mut analysis = Analysis::default();
        analysis.seed_callables(&summaries, &composition);
        assert_eq!(analysis.callables, summaries);
        assert_eq!(analysis.analyzed_callables, HashSet::from([effective]));
    }
}
