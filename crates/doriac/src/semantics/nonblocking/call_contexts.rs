//! Instantiate callback provenance at call sites. Formal parameters and returned
//! closures must not join unrelated callers; stored properties remain shared facts.

use super::*;
use crate::semantics::callable_contracts::BodyInstance;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ScopedValue(Option<usize>, Value);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Origin {
    body: Span,
    environment: usize,
}

struct Context {
    body: Span,
    effective: Span,
    parent: Option<usize>,
    entry: Option<Span>,
    environment: Option<usize>,
    blocking: Option<Span>,
    instance: BodyInstance,
}

#[derive(PartialEq, Eq, Hash)]
struct Instance {
    caller: usize,
    site: Span,
    body: Span,
    environment: Option<usize>,
    bindings: Vec<(String, ResolvedType)>,
}

struct Solver<'a, 'program> {
    checker: &'a Checker<'program>,
    graph: &'a Graph,
    contexts: Vec<Context>,
    instances: HashMap<Instance, usize>,
    parameters: HashMap<Span, Vec<Option<BindingId>>>,
    flows: HashSet<(ScopedValue, ScopedValue)>,
    origins: HashMap<ScopedValue, HashSet<Origin>>,
    unknown: HashSet<ScopedValue>,
    /// Caller context, callee context, and the call site in the caller.
    edges: HashSet<(usize, usize, Span)>,
    cleanups: HashSet<(usize, usize, Span, Value)>,
    packed_fields: HashSet<(ScopedValue, ScopedValue, StoredField)>,
    unpacked_fields: HashSet<(ScopedValue, ScopedValue, StoredField)>,
    roots: Roots,
}

#[derive(Clone, Copy)]
enum Roots {
    Executable,
    AuthoredContract,
}

pub(super) struct Resolution {
    pub blockers: HashMap<Span, Span>,
    pub known_callback_targets: HashMap<Span, Vec<Span>>,
    pub callback_captures: HashMap<Span, Vec<crate::ownership::CallbackCaptureTarget>>,
    pub callback_values: HashMap<Span, Vec<crate::ownership::CallbackCaptureTarget>>,
}

pub(super) fn resolve(checker: &Checker<'_>, graph: &Graph) -> Resolution {
    resolve_with_roots(checker, graph, Roots::Executable)
}

pub(super) fn resolve_contracts(checker: &Checker<'_>, graph: &Graph) -> Resolution {
    resolve_with_roots(checker, graph, Roots::AuthoredContract)
}

fn resolve_with_roots(checker: &Checker<'_>, graph: &Graph, roots: Roots) -> Resolution {
    let mut solver = Solver {
        checker,
        graph,
        contexts: Vec::new(),
        instances: HashMap::new(),
        parameters: HashMap::new(),
        flows: HashSet::new(),
        origins: HashMap::new(),
        unknown: HashSet::new(),
        edges: HashSet::new(),
        cleanups: HashSet::new(),
        packed_fields: HashSet::new(),
        unpacked_fields: HashSet::new(),
        roots,
    };
    for (owner, body) in &graph.bodies {
        solver.parameters.insert(
            *owner,
            body.parameters
                .iter()
                .map(|name| {
                    checker
                        .binding_resolution
                        .declarations_by_id
                        .values()
                        .find(|binding| {
                            binding.name == *name
                                && graph.owners.get(&Value::Binding(binding.id)) == Some(owner)
                                && matches!(
                                    binding.kind,
                                    BindingKind::FunctionParameter
                                        | BindingKind::MethodParameter
                                        | BindingKind::ClosureParameter
                                )
                        })
                        .map(|binding| binding.id)
                })
                .collect(),
        );
    }
    let incoming = graph
        .calls
        .iter()
        .flat_map(|call| call.targets.iter())
        .copied()
        .chain(
            graph
                .bodies
                .values()
                .flat_map(|body| body.calls.iter())
                .copied(),
        )
        .map(|declaration| {
            BodyInstance::new(
                checker.callable_body_program,
                declaration,
                HashMap::new(),
                &checker.composition,
            )
            .declaration
        })
        .collect::<HashSet<_>>();
    let mut bodies = graph.bodies.keys().copied().collect::<Vec<_>>();
    bodies.sort();
    for body in &bodies {
        if !incoming.contains(body) && !graph.closures.contains_key(body) {
            let _ = solver.root(*body);
        }
    }
    solver.propagate();
    // An otherwise unreachable recursive component still has to obey the hook
    // contract. Seed only components not already instantiated by a real call.
    for body in bodies {
        // A seed that adds no context leaves the fixpoint unchanged.
        if !graph.closures.contains_key(&body) && solver.root(body) {
            solver.propagate();
        }
    }
    let mut known_callback_targets = HashMap::<Span, HashSet<Span>>::new();
    let mut callback_captures =
        HashMap::<Span, Vec<crate::ownership::CallbackCaptureTarget>>::new();
    let mut unknown_calls = HashSet::new();
    for context in 0..solver.contexts.len() {
        let owner = solver.contexts[context].body;
        for call in graph.calls.iter().filter(|call| call.owner == owner) {
            if let Some(callee) = call.callee {
                let callee = solver.scoped(context, callee);
                if solver.unknown.contains(&callee)
                    || solver.origins.get(&callee).is_none_or(HashSet::is_empty)
                {
                    unknown_calls.insert(call.site);
                    solver.contexts[context].blocking.get_or_insert(call.site);
                } else if let Some(origins) = solver.origins.get(&callee) {
                    known_callback_targets
                        .entry(call.site)
                        .or_default()
                        .extend(origins.iter().map(|origin| origin.body));
                    for origin in origins {
                        let target = solver.callback_target(context, *origin);
                        let targets = callback_captures.entry(call.site).or_default();
                        if !targets.contains(&target) {
                            targets.push(target);
                        }
                    }
                }
            }
        }
    }
    for (caller, environment, site, value) in &solver.cleanups {
        let value = solver.scoped(*environment, *value);
        if solver.unknown.contains(&value)
            || solver.origins.get(&value).is_none_or(HashSet::is_empty)
        {
            solver.contexts[*caller].blocking.get_or_insert(*site);
        }
    }
    // A context reports its own blocking site, else the earliest site it
    // reaches through a callee. Compiler-owned sources cannot be shown, so a
    // site inside one is reported at the call that entered it.
    let direct = solver
        .contexts
        .iter()
        .map(|context| context.blocking.is_some())
        .collect::<Vec<_>>();
    loop {
        let mut changed = false;
        for &(caller, callee, call_site) in &solver.edges {
            if direct[caller] {
                continue;
            }
            let Some(site) = solver.contexts[callee].blocking else {
                continue;
            };
            let site = if site.source.is_compiler_owned() {
                call_site
            } else {
                site
            };
            if solver.contexts[caller]
                .blocking
                .is_none_or(|current| site < current)
            {
                solver.contexts[caller].blocking = Some(site);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut blockers = HashMap::new();
    let mut callback_values = HashMap::<Span, Vec<crate::ownership::CallbackCaptureTarget>>::new();
    let mut unknown_values = HashSet::new();
    // Expose the same solved origins for function-valued arguments, not only
    // their eventual invocation sites. Formal inputs stay symbolic in ownership.
    for (value, owner) in &graph.owners {
        let Value::Expression(span) = value else {
            continue;
        };
        if !checker.expression_types.get(span).is_some_and(|ty| matches!(ty, ResolvedType::Function(_)) || matches!(ty, ResolvedType::Nullable(inner) if matches!(inner.as_ref(), ResolvedType::Function(_)))) { continue; }
        for (context, _) in solver
            .contexts
            .iter()
            .enumerate()
            .filter(|(_, definition)| definition.body == *owner)
        {
            let value = solver.scoped(context, *value);
            if solver.unknown.contains(&value)
                || solver.origins.get(&value).is_none_or(HashSet::is_empty)
            {
                unknown_values.insert(*span);
            } else if let Some(origins) = solver.origins.get(&value) {
                for origin in origins {
                    let target = solver.callback_target(context, *origin);
                    let targets = callback_values.entry(*span).or_default();
                    if !targets.contains(&target) {
                        targets.push(target);
                    }
                }
            }
        }
    }
    for context in solver.contexts {
        if let Some(site) = context.blocking {
            blockers
                .entry(context.body)
                .and_modify(|old: &mut Span| *old = (*old).min(site))
                .or_insert(site);
        }
    }
    Resolution {
        blockers,
        callback_values: callback_values
            .into_iter()
            .filter(|(span, _)| !unknown_values.contains(span))
            .collect(),
        callback_captures: callback_captures
            .into_iter()
            .filter(|(site, _)| !unknown_calls.contains(site))
            .collect(),
        known_callback_targets: known_callback_targets
            .into_iter()
            .filter(|(site, _)| !unknown_calls.contains(site))
            .map(|(site, targets)| {
                let mut targets = targets.into_iter().collect::<Vec<_>>();
                targets.sort();
                (site, targets)
            })
            .collect(),
    }
}

impl Solver<'_, '_> {
    fn callback_target(
        &self,
        context: usize,
        origin: Origin,
    ) -> crate::ownership::CallbackCaptureTarget {
        crate::ownership::CallbackCaptureTarget {
            declaration: origin.body,
            captures: self
                .checker
                .closures
                .get(&ClosureId::from_span(origin.body))
                .into_iter()
                .flat_map(|closure| &closure.captures)
                .map(|capture| {
                    (
                        capture.environment_binding_id,
                        self.capture_sources(
                            context,
                            self.scoped(
                                origin.environment,
                                Value::Binding(capture.source_binding_id),
                            ),
                        ),
                    )
                })
                .collect(),
        }
    }
}

impl Solver<'_, '_> {
    fn capture_sources(
        &self,
        caller: usize,
        captured: ScopedValue,
    ) -> Vec<crate::ownership::CallbackCapturePlace> {
        use crate::ownership::{CallbackCapturePlace, CallbackCaptureSource};
        let mut pending = vec![captured];
        let mut visited = HashSet::new();
        while let Some(value) = pending.pop() {
            if !visited.insert(value) {
                continue;
            }
            pending.extend(
                self.flows
                    .iter()
                    .filter_map(|(source, target)| (*target == value).then_some(*source)),
            );
        }
        let mut context = Some(caller);
        let mut callers = Vec::new();
        while let Some(index) = context {
            let mut sources = visited
                .iter()
                .filter_map(|value| {
                    if value.0 != Some(index) {
                        return None;
                    }
                    match value.1 {
                        Value::Binding(binding) => Some(CallbackCaptureSource::Binding(binding)),
                        Value::Expression(span) => Some(CallbackCaptureSource::Expression(span)),
                        _ => None,
                    }
                })
                .collect::<Vec<_>>();
            if sources
                .iter()
                .any(|source| matches!(source, CallbackCaptureSource::Binding(_)))
            {
                sources.retain(|source| matches!(source, CallbackCaptureSource::Binding(_)));
            }
            if !sources.is_empty() {
                sources.sort();
                sources.dedup();
                return sources
                    .into_iter()
                    .map(|source| CallbackCapturePlace {
                        source,
                        callers: callers.clone(),
                    })
                    .collect();
            }
            let current = &self.contexts[index];
            if let Some(entry) = current.entry {
                callers.push(entry);
            }
            context = current.parent;
        }
        Vec::new()
    }

    /// Returns whether the body gained a root context.
    fn root(&mut self, body: Span) -> bool {
        // Authored signatures must be inferred even without a concrete caller.
        // Execution cleanup checks still use only registered specializations.
        let mut declarations = if matches!(
            self.checker.callable_body_program,
            CallableBodyProgram::Authored(_)
        ) {
            self.checker
                .composition
                .callable_templates
                .iter()
                .filter_map(|(effective, template)| {
                    (template.declaration == body).then_some(*effective)
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        if declarations.is_empty() {
            declarations.push(body);
        }
        declarations.sort();
        let before = self.contexts.len();
        for effective in declarations {
            if self
                .contexts
                .iter()
                .any(|context| context.effective == effective)
            {
                continue;
            }
            let specializations = match self.roots {
                Roots::Executable if effective != body => self
                    .checker
                    .callable_instances
                    .iter()
                    .filter(|instance| instance.declaration == effective)
                    .map(|instance| {
                        instance
                            .bindings
                            .iter()
                            .map(|(name, ty)| (name.clone(), self.checker.types.resolved(*ty)))
                            .collect()
                    })
                    .collect(),
                Roots::Executable => self.checker.blocking_specializations(effective),
                Roots::AuthoredContract => vec![HashMap::new()],
            };
            for bindings in specializations {
                let instance = BodyInstance::new(
                    self.checker.callable_body_program,
                    effective,
                    bindings,
                    &self.checker.composition,
                );
                let context = self.instantiate(effective, instance, None, None, None);
                for parameter in self.parameters.get(&body).into_iter().flatten().flatten() {
                    self.unknown
                        .insert(ScopedValue(Some(context), Value::Binding(*parameter)));
                }
            }
        }
        self.contexts.len() != before
    }

    fn owner(&self, value: Value) -> Option<Span> {
        match value {
            Value::Property(_) => None,
            Value::Returned(owner) => Some(owner),
            Value::Binding(_) => self.graph.owners.get(&value).copied(),
            Value::CallbackReturned(span) => self.owner(Value::Expression(span)),
            Value::Projection(source) => {
                use crate::ownership::cleanup::Source;
                match source {
                    Source::EnumPayload { scrutinee, .. } => {
                        self.owner(Value::Expression(scrutinee))
                    }
                    Source::CollectionElement { collection, .. } => {
                        self.owner(Value::Expression(collection))
                    }
                    _ => self.owner(source.into()),
                }
            }
            Value::Expression(span) => self.graph.owners.get(&value).copied().or_else(|| {
                self.graph
                    .bodies
                    .keys()
                    .filter(|owner| {
                        owner.source == span.source
                            && owner.expansion == span.expansion
                            && owner.start <= span.start
                            && owner.end >= span.end
                    })
                    .min_by_key(|owner| owner.end - owner.start)
                    .copied()
            }),
        }
    }

    fn scoped(&self, context: usize, value: Value) -> ScopedValue {
        let Some(owner) = self.owner(value) else {
            return ScopedValue(None, value);
        };
        let mut current = Some(context);
        while let Some(index) = current {
            let candidate = &self.contexts[index];
            if candidate.body == owner {
                return ScopedValue(Some(index), value);
            }
            current = candidate.environment.or(candidate.parent);
        }
        ScopedValue(Some(context), value)
    }

    fn instantiate(
        &mut self,
        effective: Span,
        instance: BodyInstance,
        parent: Option<usize>,
        entry: Option<Span>,
        environment: Option<usize>,
    ) -> usize {
        let body = instance.declaration;
        let index = self.contexts.len();
        self.contexts.push(Context {
            body,
            effective,
            parent,
            entry,
            environment,
            instance,
            blocking: self.graph.bodies.get(&body).and_then(|body| body.blocking),
        });
        for (source, target) in &self.graph.flows {
            if self.owner(*source).or_else(|| self.owner(*target)) == Some(body) {
                self.flows
                    .insert((self.scoped(index, *source), self.scoped(index, *target)));
            } else if self.owner(*source).is_none() && self.owner(*target).is_none() {
                self.flows
                    .insert((ScopedValue(None, *source), ScopedValue(None, *target)));
            }
        }
        for (source, target, field) in &self.graph.packed_fields {
            if self.owner(*source).or_else(|| self.owner(*target)) == Some(body) {
                self.packed_fields.insert((
                    self.scoped(index, *source),
                    self.scoped(index, *target),
                    field.clone(),
                ));
            }
        }
        for (source, target, field) in &self.graph.unpacked_fields {
            if self.owner(*source).or_else(|| self.owner(*target)) == Some(body) {
                self.unpacked_fields.insert((
                    self.scoped(index, *source),
                    self.scoped(index, *target),
                    field.clone(),
                ));
            }
        }
        for (value, origins) in &self.graph.origins {
            if self.owner(*value) == Some(body) {
                self.origins
                    .entry(self.scoped(index, *value))
                    .or_default()
                    .extend(origins.iter().map(|body| Origin {
                        body: *body,
                        environment: index,
                    }));
            }
        }
        if let Some(environment) = environment {
            if let Some(closure) = self.checker.closures.get(&ClosureId::from_span(body)) {
                for capture in &closure.captures {
                    let source =
                        self.scoped(environment, Value::Binding(capture.source_binding_id));
                    let captured =
                        self.scoped(index, Value::Binding(capture.environment_binding_id));
                    self.flows.insert((source, captured));
                    if capture.mode == ClosureCaptureMode::Writable {
                        self.flows.insert((captured, source));
                    }
                }
            }
        }
        for (owner, site, value) in &self.graph.cleanups {
            if *owner == body {
                self.cleanups.insert((index, index, *site, *value));
            }
        }
        for (owner, site, ty, value) in &self.graph.generic_cleanups {
            if *owner == body {
                let ty = self.contexts[index].instance.substitute(ty);
                let mut effects = cleanup_effects::Effects::default();
                self.checker
                    .drop_effects(&ty, *value, &mut HashSet::new(), &mut effects);
                self.cleanup_effects(index, index, *site, &effects);
            }
        }
        index
    }

    fn call(&mut self, caller: usize, call: &Call, body: Span, environment: Option<usize>) {
        let effective = body;
        let instance = if let Some(environment) = environment {
            let mut instance = self.contexts[environment].instance.clone();
            instance.declaration = body;
            instance
        } else {
            BodyInstance::new(
                self.checker.callable_body_program,
                effective,
                self.call_bindings(caller, call, environment),
                &self.checker.composition,
            )
        };
        let body = instance.declaration;
        if !self.graph.bodies.contains_key(&body) {
            return;
        }
        let mut identity_bindings = instance
            .substitutions
            .iter()
            .map(|(name, ty)| (name.clone(), ty.clone()))
            .collect::<Vec<_>>();
        identity_bindings.sort_by(|left, right| left.0.cmp(&right.0));
        let key = Instance {
            caller,
            site: call.site,
            body: effective,
            environment,
            bindings: identity_bindings,
        };
        let callee = if let Some(existing) = self.instances.get(&key) {
            *existing
        } else {
            let mut ancestor = Some(caller);
            let mut recursive = None;
            while let Some(index) = ancestor {
                if self.contexts[index].effective == effective
                    && self.contexts[index].instance == instance
                {
                    recursive = Some(index);
                    break;
                }
                ancestor = self.contexts[index].parent;
            }
            let callee = recursive.unwrap_or_else(|| {
                self.instantiate(
                    effective,
                    instance,
                    Some(caller),
                    Some(call.site),
                    environment,
                )
            });
            self.instances.insert(key, callee);
            callee
        };
        self.edges.insert((caller, callee, call.site));
        self.flows.insert((
            self.scoped(callee, Value::Returned(body)),
            self.scoped(caller, Value::Expression(call.site)),
        ));
        if let Some(Value::Expression(callback)) = call.callee {
            self.flows.insert((
                self.scoped(callee, Value::Returned(body)),
                self.scoped(caller, Value::CallbackReturned(callback)),
            ));
        }
        let parameters = &self.graph.bodies[&body].parameters;
        let bound = crate::arg_binding::bind_arguments(
            &parameters.iter().map(String::as_str).collect::<Vec<_>>(),
            &vec![false; parameters.len()],
            &call
                .arguments
                .iter()
                .map(|(name, _)| name.as_deref())
                .collect::<Vec<_>>(),
        );
        for ((_, argument), parameter) in call.arguments.iter().zip(bound.arg_to_param) {
            if let Some(binding) = parameter.and_then(|index| self.parameters[&body][index]) {
                let argument = self.scoped(caller, *argument);
                let parameter = self.scoped(callee, Value::Binding(binding));
                self.flows.insert((argument, parameter));
                if self.checker.binding_resolution.declarations_by_id[&binding].ownership
                    == BindingOwnership::WritableBorrow
                {
                    self.flows.insert((parameter, argument));
                }
            }
        }
    }

    fn call_bindings(
        &self,
        caller: usize,
        call: &Call,
        environment: Option<usize>,
    ) -> HashMap<String, ResolvedType> {
        let outer = &self.contexts[environment.unwrap_or(caller)].instance;
        if environment.is_some() {
            return outer.substitutions.clone();
        }
        let mut bindings = HashMap::new();
        let target = self.checker.call_targets.get(&call.site).or_else(|| {
            self.checker
                .property_accessor_calls
                .get(&call.site)
                .and_then(|calls| calls.getter.as_ref().or(calls.setter.as_ref()))
                .map(|call| &call.target)
        });
        if let Some(CallableTarget::Method { class_type, .. }) = target {
            if let Some(class) = self.checker.classes.get(&class_type.name) {
                bindings.extend(
                    class
                        .type_params
                        .iter()
                        .zip(&class_type.arguments)
                        .map(|(parameter, ty)| (parameter.name.clone(), outer.substitute(ty))),
                );
            }
        }
        if let Some(pending) = self.checker.pending_generic_calls.get(&call.site) {
            bindings.extend(pending.inference.bindings.iter().map(|(name, ty)| {
                (
                    name.clone(),
                    outer.substitute(&self.checker.types.resolved(*ty)),
                )
            }));
        }
        bindings.extend(
            call.bindings
                .iter()
                .map(|(name, ty)| (name.clone(), outer.substitute(ty))),
        );
        bindings
    }

    fn cleanup_effects(
        &mut self,
        caller: usize,
        environment: usize,
        site: Span,
        effects: &cleanup_effects::Effects,
    ) {
        // Symbolic release obligations are checked again for executable instances.
        if effects.unknown
            || (matches!(self.roots, Roots::Executable) && !effects.generic.is_empty())
        {
            self.contexts[caller].blocking.get_or_insert(site);
        }
        for callback in &effects.callbacks {
            self.cleanups.insert((caller, environment, site, *callback));
        }
        for (target, class) in &effects.targets {
            self.call(
                caller,
                &Call {
                    owner: self.contexts[caller].body,
                    site,
                    targets: vec![*target],
                    callee: None,
                    arguments: Vec::new(),
                    bindings: self.checker.drop_target_bindings(class),
                },
                *target,
                None,
            );
        }
    }

    fn propagate(&mut self) {
        // Each pass visits only the calls owned by a context's body.
        let graph = self.graph;
        let mut calls_by_owner = HashMap::<Span, Vec<&Call>>::new();
        for call in &graph.calls {
            calls_by_owner.entry(call.owner).or_default().push(call);
        }
        loop {
            let before = (
                self.contexts.len(),
                self.flows.len(),
                self.origins.values().map(HashSet::len).sum::<usize>(),
                self.unknown.len(),
                self.cleanups.len(),
            );
            for (source, target) in &self.flows {
                let incoming = self.origins.get(source).cloned().unwrap_or_default();
                self.origins.entry(*target).or_default().extend(incoming);
                if self.unknown.contains(source) {
                    self.unknown.insert(*target);
                }
            }
            self.propagate_fields();
            for caller in 0..self.contexts.len() {
                let owner = self.contexts[caller].body;
                for call in calls_by_owner.get(&owner).into_iter().flatten().copied() {
                    if let Some(callee) = call.callee {
                        let origins = self
                            .origins
                            .get(&self.scoped(caller, callee))
                            .cloned()
                            .unwrap_or_default();
                        for origin in origins {
                            self.call(caller, call, origin.body, Some(origin.environment));
                        }
                    } else {
                        if !call.targets.iter().any(|target| {
                            self.graph.bodies.contains_key(
                                &BodyInstance::new(
                                    self.checker.callable_body_program,
                                    *target,
                                    HashMap::new(),
                                    &self.checker.composition,
                                )
                                .declaration,
                            ) || self.graph.nonblocking_contracts.contains(target)
                        }) {
                            self.contexts[caller].blocking.get_or_insert(call.site);
                        }
                        for target in &call.targets {
                            self.call(caller, call, *target, None);
                        }
                    }
                }
                if let Some(body) = self.graph.bodies.get(&owner) {
                    for target in &body.calls {
                        self.call(
                            caller,
                            &Call {
                                owner,
                                site: *target,
                                targets: vec![*target],
                                callee: None,
                                arguments: Vec::new(),
                                bindings: self.contexts[caller].instance.substitutions.clone(),
                            },
                            *target,
                            None,
                        );
                    }
                }
            }
            for (caller, environment, site, value) in self.cleanups.clone() {
                let origins = self
                    .origins
                    .get(&self.scoped(environment, value))
                    .cloned()
                    .unwrap_or_default();
                for origin in origins {
                    if let Some(mut effects) =
                        self.graph.closure_cleanups.get(&origin.body).cloned()
                    {
                        for (ty, value) in std::mem::take(&mut effects.generic) {
                            let ty = self.contexts[origin.environment].instance.substitute(&ty);
                            self.checker.drop_effects(
                                &ty,
                                value,
                                &mut HashSet::new(),
                                &mut effects,
                            );
                        }
                        self.cleanup_effects(caller, origin.environment, site, &effects);
                    }
                }
            }
            let after = (
                self.contexts.len(),
                self.flows.len(),
                self.origins.values().map(HashSet::len).sum::<usize>(),
                self.unknown.len(),
                self.cleanups.len(),
            );
            if before == after {
                break;
            }
        }
    }

    fn propagate_fields(&mut self) {
        // Match construction and extraction through ordinary alias/call flows.
        // Saturating finite value edges also handles nested/recursive aggregates
        // without inventing an unbounded stack of symbolic field paths.
        let mut successors = HashMap::<ScopedValue, Vec<ScopedValue>>::new();
        for (source, target) in &self.flows {
            successors.entry(*source).or_default().push(*target);
        }
        let mut extractions = HashMap::<ScopedValue, Vec<(ScopedValue, StoredField)>>::new();
        for (source, target, field) in &self.unpacked_fields {
            extractions
                .entry(*source)
                .or_default()
                .push((*target, field.clone()));
            if self.unknown.contains(source) {
                self.unknown.insert(*target);
            }
        }
        for (value, aggregate, field) in &self.packed_fields {
            let mut pending = vec![*aggregate];
            let mut seen = HashSet::new();
            while let Some(current) = pending.pop() {
                if !seen.insert(current) {
                    continue;
                }
                for (target, selected) in extractions.get(&current).into_iter().flatten() {
                    if selected.overlaps(field) {
                        self.flows.insert((*value, *target));
                    }
                }
                pending.extend(successors.get(&current).into_iter().flatten());
            }
        }
    }
}
