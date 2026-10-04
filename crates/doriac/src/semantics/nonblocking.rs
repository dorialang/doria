//! Execution effects are independent of checked-error escape. In particular,
//! catching an I/O error does not make the operation nonblocking.

use super::*;
use crate::control_flow::{build_function_cfg_with_checked_effects, NodeAction};
use crate::types::CollectionFamily;

mod call_contexts;
mod cleanup_effects;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Value {
    Expression(Span),
    Binding(BindingId),
    Returned(Span),
    CallbackReturned(Span),
    Projection(crate::ownership::cleanup::Source),
    Property(Span),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct PayloadField {
    case: EnumCaseId,
    index: usize,
}

#[derive(Default)]
struct Body {
    parameters: Vec<String>,
    calls: HashSet<Span>,
    blocking: Option<Span>,
}

#[derive(Clone)]
struct Call {
    owner: Span,
    site: Span,
    targets: Vec<Span>,
    callee: Option<Value>,
    arguments: Vec<(Option<String>, Value)>,
    bindings: HashMap<String, ResolvedType>,
}

#[derive(Default)]
struct Graph {
    bodies: HashMap<Span, Body>,
    flows: HashSet<(Value, Value)>,
    origins: HashMap<Value, HashSet<Span>>,
    owners: HashMap<Value, Span>,
    calls: Vec<Call>,
    closures: HashMap<Span, ClosureExpression>,
    visited_when: HashSet<Span>,
    nonblocking_contracts: HashSet<Span>,
    cleanups: Vec<(Span, Span, Value)>,
    generic_cleanups: Vec<(Span, Span, ResolvedType, Value)>,
    closure_cleanups: HashMap<Span, cleanup_effects::Effects>,
    packed_fields: HashSet<(Value, Value, PayloadField)>,
    unpacked_fields: HashSet<(Value, Value, PayloadField)>,
}

pub(super) struct CallProvenance {
    graph: Graph,
    hooks: Vec<Span>,
}

impl Graph {
    fn flow(&mut self, source: Value, target: Value) {
        self.flows.insert((source, target));
    }

    fn alias(&mut self, left: Value, right: Value) {
        self.flow(left, right);
        self.flow(right, left);
    }

    fn owner(&self, enclosing: Span, expression: Span) -> Span {
        self.closures
            .keys()
            .filter(|span| {
                span.source == expression.source
                    && span.expansion == expression.expansion
                    && span.start < expression.start
                    && span.end >= expression.end
            })
            .min_by_key(|span| span.end - span.start)
            .copied()
            .unwrap_or(enclosing)
    }

    fn connect_call(
        &mut self,
        _checker: &Checker<'_>,
        owner: Span,
        site: Span,
        targets: &[Span],
        arguments: &[(Option<String>, Value)],
    ) {
        if !targets.is_empty() {
            self.calls.push(Call {
                owner,
                site,
                targets: targets.to_vec(),
                callee: None,
                arguments: arguments.to_vec(),
                bindings: HashMap::new(),
            });
        }
    }
}

impl Checker<'_> {
    pub(super) fn prepare_call_provenance(
        &self,
        backing_fields: &HashMap<Span, crate::property_hooks::PropertyBackingField>,
    ) -> CallProvenance {
        let mut hooks = Vec::new();
        let mut functions = Vec::new();
        let mut initializers = Vec::new();
        for item in &self.program.items {
            match item {
                Item::Function(function) => functions.push(function.clone()),
                Item::Class(class) => {
                    for member in &class.members {
                        if let ClassMember::Property(property) = member {
                            hooks.extend(property.hooks.iter().map(|hook| hook.span));
                            if let Some(initializer) = &property.initializer {
                                initializers.push((class.span, property.span, initializer.clone()));
                            }
                        }
                        functions.extend(
                            crate::property_hooks::member_callables(
                                member,
                                crate::property_hooks::PropertyHookContext::Class,
                            )
                            .map(|function| function.into_owned()),
                        );
                    }
                }
                Item::Trait(declaration) => {
                    for member in &declaration.members {
                        if let ClassMember::Property(property) = member {
                            hooks.extend(property.hooks.iter().map(|hook| hook.span));
                        }
                        functions.extend(
                            crate::property_hooks::member_callables(
                                member,
                                crate::property_hooks::PropertyHookContext::Trait,
                            )
                            .map(|function| function.into_owned()),
                        );
                    }
                }
                _ => {}
            }
        }
        if hooks.is_empty()
            && self.callable_value_calls.is_empty()
            && self.list_algorithm_calls.is_empty()
        {
            return CallProvenance {
                graph: Graph::default(),
                hooks,
            };
        }
        let mut graph = Graph::default();
        for item in &self.program.items {
            if let Item::Interface(interface) = item {
                graph.nonblocking_contracts.extend(
                    interface
                        .properties
                        .iter()
                        .flat_map(|property| property.hooks.iter().map(|hook| hook.span)),
                );
            }
        }
        for (declaration, field) in backing_fields {
            graph.alias(
                Value::Property(*declaration),
                Value::Property(field.declaration),
            );
        }
        for class in self.classes.values() {
            if let Some(parent) = class
                .parent
                .as_ref()
                .and_then(|parent| self.classes.get(&parent.name))
            {
                graph
                    .bodies
                    .entry(class.declaration)
                    .or_default()
                    .calls
                    .insert(parent.declaration);
            }
            if let Some(constructor) = class.methods.get("__construct") {
                for parameter in &constructor.params {
                    let Some(property) = class.properties.get(&parameter.name) else {
                        continue;
                    };
                    if property.init_state != PropertyInitState::PromotedParameter {
                        continue;
                    }
                    if let Some(binding) =
                        self.binding_resolution
                            .declarations_by_id
                            .values()
                            .find(|binding| {
                                binding.owner == LexicalOwner::Callable(constructor.declaration)
                                    && binding.name == parameter.name
                            })
                    {
                        graph.flow(
                            Value::Binding(binding.id),
                            Value::Property(property.declaration_span),
                        );
                    }
                }
            }
        }
        for function in &functions {
            if function.body.as_block().is_none() {
                continue;
            }
            graph.bodies.entry(function.span).or_default().parameters = function
                .params
                .iter()
                .map(|parameter| parameter.name.clone())
                .collect();
            if let Some(body) = function.body.as_block() {
                crate::ast::visit::block(body, &mut |expr| {
                    if let Expr::Closure(closure) = expr {
                        graph
                            .closures
                            .insert(closure.span, closure.as_ref().clone());
                    }
                });
            }
        }
        for (_, _, initializer) in &initializers {
            crate::ast::visit::expr(initializer, &mut |expr| {
                if let Expr::Closure(closure) = expr {
                    graph
                        .closures
                        .insert(closure.span, closure.as_ref().clone());
                }
            });
        }
        let closures = graph.closures.values().cloned().collect::<Vec<_>>();
        for closure in &closures {
            graph.bodies.entry(closure.span).or_default().parameters = closure
                .parameters
                .iter()
                .map(|parameter| parameter.name.clone())
                .collect();
        }
        for function in &functions {
            if let Some(body) = function.body.as_block() {
                self.blocking_body(&mut graph, function.span, body);
            }
        }
        for closure in &closures {
            match &closure.body {
                ClosureBody::Block(body) => self.blocking_body(&mut graph, closure.span, body),
                ClosureBody::Expression { expression, .. } => {
                    graph.flow(
                        Value::Expression(expression.span()),
                        Value::Returned(closure.span),
                    );
                    crate::ast::visit::expr(expression, &mut |expr| {
                        self.blocking_expression(&mut graph, closure.span, expr)
                    });
                }
            }
        }
        for (class, property, initializer) in initializers {
            graph.bodies.entry(property).or_default();
            graph
                .bodies
                .entry(class)
                .or_default()
                .calls
                .insert(property);
            graph.flow(
                Value::Expression(initializer.span()),
                Value::Property(property),
            );
            crate::ast::visit::expr(&initializer, &mut |expr| {
                self.blocking_expression(&mut graph, property, expr)
            });
        }
        for iteration in self.foreach_loops.values() {
            for binding in &iteration.binding_ids {
                graph.flow(
                    Value::Expression(iteration.iterable_span),
                    Value::Binding(*binding),
                );
            }
        }
        for binding in self.binding_resolution.declarations_by_id.values() {
            let owner = match binding.owner {
                LexicalOwner::Callable(owner) => Some(owner),
                LexicalOwner::Closure(id) => graph
                    .closures
                    .keys()
                    .find(|span| ClosureId::from_span(**span) == id)
                    .copied(),
                LexicalOwner::TopLevel => None,
            };
            if let Some(owner) = owner {
                graph.owners.insert(Value::Binding(binding.id), owner);
            }
        }
        CallProvenance { graph, hooks }
    }

    pub(super) fn known_callback_targets(
        &self,
        provenance: &CallProvenance,
    ) -> HashMap<Span, Vec<Span>> {
        if !provenance
            .graph
            .calls
            .iter()
            .any(|call| call.callee.is_some())
        {
            return HashMap::new();
        }
        call_contexts::resolve(self, &provenance.graph).known_callback_targets
    }

    pub(super) fn check_nonblocking_property_hooks(
        &mut self,
        provenance: &mut CallProvenance,
        cleanup: &crate::ownership::cleanup::Analysis,
        partial_cleanups: &[crate::constructor_init::ConstructorPartialCleanup],
    ) {
        if provenance.hooks.is_empty() {
            return;
        }
        self.blocking_cleanups(&mut provenance.graph, cleanup, partial_cleanups);
        let blockers = call_contexts::resolve(self, &provenance.graph).blockers;
        for hook in provenance.hooks.iter().copied() {
            if let Some(site) = blockers.get(&hook).copied() {
                self.diagnostics.push(
                    Diagnostic::new("E0770", "property hooks must not perform potentially blocking work", hook)
                        .with_title("Property Hook May Block")
                        .with_related(site, "Potentially Blocking Operation")
                        .with_explanation("the nonblocking rule applies through helpers and callback invocation; catching an I/O error does not make the operation nonblocking")
                        .with_help("perform input/output outside the hook, and use callbacks whose nonblocking body can be established"),
                );
            }
        }
    }

    fn blocking_call_targets(&self, declaration: Span, direct: bool) -> Vec<Span> {
        let mut targets = vec![declaration];
        if !direct {
            let root = self
                .override_roots
                .get(&declaration)
                .copied()
                .unwrap_or(declaration);
            targets.extend(
                self.override_roots
                    .iter()
                    .filter_map(|(member, family)| (*family == root).then_some(*member)),
            );
            for conformance in &self.contracts.conformances {
                for implementation in &conformance.implementations {
                    if implementation
                        .requirement_origins
                        .iter()
                        .any(|origin| origin.declaration == declaration)
                    {
                        targets.extend(implementation.implementation);
                    }
                }
            }
        }
        targets.sort();
        targets.dedup();
        targets
    }

    fn blocking_receiver_targets(
        &self,
        declaration: Span,
        direct: bool,
        receiver: Option<&ResolvedType>,
    ) -> Vec<Span> {
        let mut receiver = receiver;
        while let Some(ResolvedType::Nullable(inner) | ResolvedType::SharedHandle(_, inner)) =
            receiver
        {
            receiver = Some(inner);
        }
        let mut targets = self.blocking_call_targets(declaration, direct);
        let Some(ResolvedType::Class(receiver)) = receiver else {
            return targets;
        };
        targets.retain(|target| {
            *target == declaration
                || self.classes.iter().any(|(name, class)| {
                    let declares_target = class
                        .methods
                        .values()
                        .any(|method| method.declaration == *target)
                        || class
                            .properties
                            .values()
                            .filter_map(|property| property.hooks.as_ref())
                            .any(|hooks| {
                                hooks
                                    .getter
                                    .iter()
                                    .chain(hooks.setter.iter())
                                    .any(|accessor| accessor.declaration == *target)
                            });
                    if !declares_target {
                        return false;
                    }
                    let mut current = Some(name.as_str());
                    let mut visited = HashSet::new();
                    while let Some(name) = current {
                        if name == receiver.name {
                            return true;
                        }
                        if !visited.insert(name) {
                            break;
                        }
                        current = self
                            .classes
                            .get(name)
                            .and_then(|class| class.parent.as_ref())
                            .map(|parent| parent.name.as_str());
                    }
                    false
                })
        });
        targets
    }

    fn expression_call_targets(&self, expression: &Expr) -> Vec<Span> {
        let span = expression.span();
        if let Some(method) = self.method_call_targets.get(&span) {
            let receiver = match expression {
                Expr::MethodCall { object, .. } => self.expression_types.get(&object.span()),
                _ => None,
            };
            return self.blocking_receiver_targets(
                method.declaration,
                method.direct_parent || method.virtual_root.is_none(),
                receiver,
            );
        }
        self.call_targets
            .get(&span)
            .map(|target| self.blocking_target(target))
            .unwrap_or_default()
    }

    fn blocking_target(&self, target: &CallableTarget) -> Vec<Span> {
        match target {
            CallableTarget::Function { name } => self
                .functions
                .get(name)
                .map(|function| vec![function.declaration])
                .unwrap_or_default(),
            CallableTarget::InterfaceMethod { requirement, .. } => {
                self.blocking_call_targets(*requirement, false)
            }
            CallableTarget::ConstrainedMethod {
                requirement,
                implementations,
                ..
            } => {
                let mut targets = self.blocking_call_targets(*requirement, false);
                targets.extend(implementations.iter().flat_map(|implementation| {
                    self.blocking_call_targets(implementation.declaration, false)
                }));
                targets
            }
            CallableTarget::Method {
                class_type,
                method_name,
                direct_parent,
            } => self
                .classes
                .get(&class_type.name)
                .and_then(|class| class.methods.get(method_name))
                .map(|method| self.blocking_call_targets(method.declaration, *direct_parent))
                .unwrap_or_default(),
        }
    }

    fn blocking_property_identity(&self, object: &Expr, name: &str) -> Option<Span> {
        let mut ty = self.expression_types.get(&object.span())?;
        while let ResolvedType::Nullable(inner) | ResolvedType::SharedHandle(_, inner) = ty {
            ty = inner;
        }
        let ResolvedType::Class(class) = ty else {
            return None;
        };
        let mut current = self.classes.get(&class.name)?;
        let mut visited = HashSet::new();
        while visited.insert(current.declaration) {
            if let Some(property) = current.properties.get(name) {
                return Some(property.declaration_span);
            }
            current = self.classes.get(&current.parent.as_ref()?.name)?;
        }
        None
    }

    fn blocking_expression(&self, graph: &mut Graph, enclosing: Span, expression: &Expr) {
        let span = expression.span();
        let owner = graph.owner(enclosing, span);
        let value = Value::Expression(span);
        graph.owners.insert(value, owner);
        if let Some(binding) = self.binding_resolution.uses_by_span.get(&span) {
            graph.alias(Value::Binding(*binding), value);
        }
        match expression {
            Expr::Closure(closure) => {
                graph.origins.entry(value).or_default().insert(closure.span);
            }
            Expr::Grouped { expr, .. } => graph.alias(Value::Expression(expr.span()), value),
            Expr::PropertyAccess {
                object, property, ..
            } => {
                if matches!(property.as_str(), "keys" | "values")
                    && self
                        .expression_types
                        .get(&object.span())
                        .and_then(CollectionFamily::from_resolved)
                        == Some(CollectionFamily::Dictionary)
                {
                    graph.flow(Value::Expression(object.span()), value);
                }
                if !self
                    .property_accessor_calls
                    .get(&span)
                    .is_some_and(|calls| calls.getter.is_some() || calls.setter.is_some())
                {
                    if let Some(property) = self.blocking_property_identity(object, property) {
                        graph.alias(Value::Property(property), value);
                    }
                }
            }
            Expr::Index { collection, .. } => {
                graph.alias(Value::Expression(collection.span()), value)
            }
            Expr::Array { elements, .. } => {
                for element in elements {
                    graph.flow(Value::Expression(element.value.span()), value);
                }
            }
            Expr::ArrayRepeat { value: element, .. } => {
                graph.flow(Value::Expression(element.span()), value)
            }
            Expr::MethodCall {
                object,
                method,
                args,
                ..
            } => {
                if let Some(family) = self
                    .expression_types
                    .get(&object.span())
                    .and_then(CollectionFamily::from_resolved)
                {
                    for index in family.ingested_arguments(method) {
                        if let Some(argument) = args.get(*index) {
                            graph.flow(
                                Value::Expression(argument.value.span()),
                                Value::Expression(object.span()),
                            );
                        }
                    }
                }
            }
            Expr::Binary {
                op: BinaryOp::Coalesce,
                left,
                right,
                ..
            } => {
                graph.flow(Value::Expression(left.span()), value);
                graph.flow(Value::Expression(right.span()), value);
            }
            Expr::Match {
                scrutinee, arms, ..
            } => {
                for (index, arm) in arms.iter().enumerate() {
                    graph.flow(Value::Expression(arm.value.span()), value);
                    if let (
                        MatchPattern::EnumCase {
                            bindings: Some(bindings),
                            ..
                        },
                        Some(MatchArmSemanticInfo {
                            pattern: ResolvedMatchPattern::EnumCase { case_id, .. },
                            ..
                        }),
                    ) = (
                        &arm.pattern,
                        self.matches
                            .get(&span)
                            .and_then(|info| info.arms.get(index)),
                    ) {
                        for (field, binding) in bindings.iter().enumerate() {
                            if let Some(id) = self
                                .binding_resolution
                                .declaration_by_span
                                .get(&binding.span)
                            {
                                graph.unpacked_fields.insert((
                                    Value::Expression(scrutinee.span()),
                                    Value::Binding(*id),
                                    PayloadField {
                                        case: *case_id,
                                        index: field,
                                    },
                                ));
                            }
                        }
                    }
                    let bindings = match &arm.pattern {
                        MatchPattern::TypeBinding { binding, .. } => std::slice::from_ref(binding),
                        _ => &[],
                    };
                    for binding in bindings {
                        if let Some(id) = self
                            .binding_resolution
                            .declaration_by_span
                            .get(&binding.span)
                        {
                            graph.flow(Value::Expression(scrutinee.span()), Value::Binding(*id));
                        }
                    }
                }
            }
            Expr::When(when) if graph.visited_when.insert(span) => {
                if let Some(given) = &when.given {
                    self.blocking_block(graph, owner, &given.block, value);
                }
                for branch in &when.branches {
                    self.blocking_block(graph, owner, &branch.block, value);
                }
                if let Some(finalizer) = &when.finally {
                    self.blocking_block(graph, owner, &finalizer.block, value);
                }
            }
            Expr::FunctionCall { name, .. }
                if Builtin::from_name(name).is_some_and(Builtin::may_block) =>
            {
                graph
                    .bodies
                    .entry(owner)
                    .or_default()
                    .blocking
                    .get_or_insert(span);
            }
            _ => {}
        }
        let arguments = match expression {
            Expr::FunctionCall { args, .. }
            | Expr::MethodCall { args, .. }
            | Expr::StaticCall { args, .. }
            | Expr::CallableCall { args, .. }
            | Expr::New { args, .. } => args.as_slice(),
            _ => &[],
        }
        .iter()
        .map(|argument| {
            (
                argument.name.as_ref().map(|name| name.text.clone()),
                Value::Expression(argument.value.span()),
            )
        })
        .collect::<Vec<_>>();
        graph.connect_call(
            self,
            owner,
            span,
            &self.expression_call_targets(expression),
            &arguments,
        );
        if let Some(case_id) = self.enum_case_constructions.get(&span) {
            if let Some(case) = self
                .enums
                .values()
                .find(|definition| definition.id == case_id.enum_id)
                .and_then(|definition| definition.cases.get(case_id.index))
            {
                let parameters = case
                    .payload
                    .iter()
                    .map(|field| field.name.as_str())
                    .collect::<Vec<_>>();
                let bound = crate::arg_binding::bind_arguments(
                    &parameters,
                    &vec![false; parameters.len()],
                    &arguments
                        .iter()
                        .map(|(name, _)| name.as_deref())
                        .collect::<Vec<_>>(),
                );
                for ((_, argument), field) in arguments.iter().zip(bound.arg_to_param) {
                    if let Some(field) = field {
                        graph.packed_fields.insert((
                            *argument,
                            value,
                            PayloadField {
                                case: *case_id,
                                index: field,
                            },
                        ));
                    }
                }
            }
        }
        if let Some(calls) = self.core_operation_calls.get(&span) {
            for call in calls {
                graph.connect_call(self, owner, span, &self.blocking_target(&call.target), &[]);
            }
        }
        if matches!(expression, Expr::New { .. }) {
            if let Some(ResolvedType::Class(class)) = self.expression_types.get(&span) {
                if let Some(class) = self.classes.get(&class.name) {
                    graph
                        .bodies
                        .entry(owner)
                        .or_default()
                        .calls
                        .insert(class.declaration);
                }
            }
        }
        if let Some(calls) = self.property_accessor_calls.get(&span) {
            let receiver = match expression {
                Expr::PropertyAccess { object, .. } | Expr::MethodCall { object, .. } => {
                    self.expression_types.get(&object.span())
                }
                _ => None,
            };
            if let Some(accessor) = &calls.getter {
                graph.connect_call(
                    self,
                    owner,
                    span,
                    &self.blocking_receiver_targets(
                        accessor.declaration,
                        accessor.virtual_root.is_none(),
                        receiver,
                    ),
                    &[],
                );
            }
        }
        if let Some(call) = self.callable_value_calls.get(&span) {
            if call.target_kind == CallableValueTargetKind::Property {
                if let Expr::MethodCall { object, method, .. } = expression {
                    if let Some(getter) = self
                        .property_accessor_calls
                        .get(&call.callee_span)
                        .and_then(|calls| calls.getter.as_ref())
                    {
                        graph.connect_call(
                            self,
                            owner,
                            call.callee_span,
                            &self.blocking_call_targets(getter.declaration, false),
                            &[],
                        );
                    } else if let Some(property) = self.blocking_property_identity(object, method) {
                        graph.alias(
                            Value::Property(property),
                            Value::Expression(call.callee_span),
                        );
                    }
                }
            }
            graph.calls.push(Call {
                owner,
                site: span,
                targets: Vec::new(),
                callee: Some(Value::Expression(call.callee_span)),
                arguments: arguments.clone(),
                bindings: HashMap::new(),
            });
        }
        if let Some(call) = self.list_algorithm_calls.get(&span) {
            let receiver = Value::Expression(call.receiver_span);
            let callback_arguments = if call.kind == ListAlgorithmKind::Reduce {
                let accumulator = arguments.first().map(|(_, value)| *value).unwrap_or(value);
                graph.flow(value, accumulator);
                vec![(None, accumulator), (None, receiver)]
            } else {
                if call.kind == ListAlgorithmKind::Filter {
                    graph.flow(receiver, value);
                }
                vec![(None, receiver)]
            };
            graph.calls.push(Call {
                owner,
                site: span,
                targets: Vec::new(),
                callee: Some(Value::Expression(call.callback_span)),
                arguments: callback_arguments,
                bindings: HashMap::new(),
            });
        }
    }

    fn blocking_body(&self, graph: &mut Graph, owner: Span, block: &Block) {
        self.blocking_block(graph, owner, block, Value::Returned(owner));
    }

    fn blocking_block(&self, graph: &mut Graph, owner: Span, block: &Block, returned: Value) {
        let cfg = build_function_cfg_with_checked_effects(
            block,
            owner,
            &self.given_preludes,
            &self.checked_effect_sites,
            &self.catch_error_types,
            &self.catch_coverage,
            &self.terminal_assertion_spans(),
        );
        for node in cfg.nodes {
            let statement = match &node.action {
                NodeAction::Statement(statement) => Some(statement),
                NodeAction::ForInitializer(ForInitializer::VarDecl(declaration)) => {
                    self.blocking_local(graph, declaration);
                    crate::ast::visit::expr(&declaration.initializer, &mut |expr| {
                        self.blocking_expression(graph, owner, expr)
                    });
                    None
                }
                _ => None,
            };
            if let Some(statement) = statement {
                match statement {
                    Stmt::Echo { span, .. } => {
                        graph
                            .bodies
                            .entry(owner)
                            .or_default()
                            .blocking
                            .get_or_insert(*span);
                    }
                    Stmt::VarDecl(declaration) => self.blocking_local(graph, declaration),
                    Stmt::Assignment(assignment) => {
                        self.blocking_assignment(graph, owner, assignment)
                    }
                    Stmt::Increment(increment) => self.blocking_increment(graph, owner, increment),
                    Stmt::Return {
                        expr: Some(expr), ..
                    } => graph.flow(Value::Expression(expr.span()), returned),
                    _ => {}
                }
                crate::ast::visit::stmt(statement, &mut |expr| {
                    self.blocking_expression(graph, owner, expr)
                });
            }
            match &node.action {
                NodeAction::Expression(expr)
                | NodeAction::Assume {
                    condition: expr, ..
                } => {
                    crate::ast::visit::expr(expr, &mut |expr| {
                        self.blocking_expression(graph, owner, expr)
                    });
                }
                NodeAction::ForIncrement(ForIncrement::Increment(increment)) => {
                    self.blocking_increment(graph, owner, increment);
                    crate::ast::visit::expr(&increment.target, &mut |expr| {
                        self.blocking_expression(graph, owner, expr)
                    });
                }
                _ => {}
            }
            let assignment = match &node.action {
                NodeAction::ForInitializer(ForInitializer::Assignment(assignment)) => {
                    Some(assignment)
                }
                NodeAction::ForIncrement(ForIncrement::Assignment(assignment)) => {
                    Some(assignment.as_ref())
                }
                _ => None,
            };
            if let Some(assignment) = assignment {
                self.blocking_assignment(graph, owner, assignment);
                crate::ast::visit::expr(&assignment.target, &mut |expr| {
                    self.blocking_expression(graph, owner, expr)
                });
                crate::ast::visit::expr(&assignment.value, &mut |expr| {
                    self.blocking_expression(graph, owner, expr)
                });
            }
        }
    }

    fn blocking_assignment(&self, graph: &mut Graph, owner: Span, assignment: &Assignment) {
        let source = Value::Expression(assignment.value.span());
        graph.flow(source, Value::Expression(assignment.target.span()));
        if let Some(setter) = self
            .property_accessor_calls
            .get(&assignment.target.span())
            .and_then(|calls| calls.setter.as_ref())
        {
            graph.connect_call(
                self,
                owner,
                assignment.target.span(),
                &self.blocking_call_targets(setter.declaration, false),
                &[(None, source)],
            );
        }
    }

    fn blocking_increment(&self, graph: &mut Graph, owner: Span, increment: &IncrementStmt) {
        if let Some(setter) = self
            .property_accessor_calls
            .get(&increment.target.span())
            .and_then(|calls| calls.setter.as_ref())
        {
            graph.connect_call(
                self,
                owner,
                increment.target.span(),
                &self.blocking_call_targets(setter.declaration, false),
                &[],
            );
        }
    }

    fn blocking_local(&self, graph: &mut Graph, declaration: &VarDecl) {
        for binding in &declaration.bindings {
            if let Some(id) = self
                .binding_resolution
                .declaration_by_span
                .get(&binding.span)
            {
                graph.flow(
                    Value::Expression(declaration.initializer.span()),
                    Value::Binding(*id),
                );
            }
        }
    }
}
