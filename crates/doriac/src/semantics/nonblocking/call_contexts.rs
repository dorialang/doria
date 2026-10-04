//! Instantiate callback provenance at call sites. Formal parameters and returned
//! closures must not join unrelated callers; stored properties remain shared facts.

use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct ScopedValue(Option<usize>, Value);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Origin {
    body: Span,
    environment: usize,
}

struct Context {
    body: Span,
    parent: Option<usize>,
    environment: Option<usize>,
    blocking: Option<Span>,
    bindings: HashMap<String, ResolvedType>,
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
    edges: HashSet<(usize, usize)>,
    cleanups: HashSet<(usize, usize, Span, Value)>,
    packed_fields: HashSet<(ScopedValue, ScopedValue, PayloadField)>,
    unpacked_fields: HashSet<(ScopedValue, ScopedValue, PayloadField)>,
}

pub(super) struct Resolution {
    pub blockers: HashMap<Span, Span>,
    pub known_callback_targets: HashMap<Span, Vec<Span>>,
}

pub(super) fn resolve(checker: &Checker<'_>, graph: &Graph) -> Resolution {
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
        .collect::<HashSet<_>>();
    let mut bodies = graph.bodies.keys().copied().collect::<Vec<_>>();
    bodies.sort();
    for body in &bodies {
        if !incoming.contains(body) && !graph.closures.contains_key(body) {
            solver.root(*body);
        }
    }
    solver.propagate();
    // An otherwise unreachable recursive component still has to obey the hook
    // contract. Seed only components not already instantiated by a real call.
    for body in bodies {
        if !graph.closures.contains_key(&body)
            && !solver.contexts.iter().any(|context| context.body == body)
        {
            solver.root(body);
            solver.propagate();
        }
    }
    let mut known_callback_targets = HashMap::<Span, HashSet<Span>>::new();
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
    loop {
        let mut changed = false;
        for (caller, callee) in &solver.edges {
            if solver.contexts[*caller].blocking.is_none() {
                if let Some(site) = solver.contexts[*callee].blocking {
                    solver.contexts[*caller].blocking = Some(site);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut blockers = HashMap::new();
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
    fn root(&mut self, body: Span) {
        for bindings in self.checker.blocking_specializations(body) {
            let context = self.instantiate(body, None, None, bindings);
            for parameter in self.parameters.get(&body).into_iter().flatten().flatten() {
                self.unknown
                    .insert(ScopedValue(Some(context), Value::Binding(*parameter)));
            }
        }
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
        body: Span,
        parent: Option<usize>,
        environment: Option<usize>,
        bindings: HashMap<String, ResolvedType>,
    ) -> usize {
        let index = self.contexts.len();
        self.contexts.push(Context {
            body,
            parent,
            environment,
            bindings,
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
                    *field,
                ));
            }
        }
        for (source, target, field) in &self.graph.unpacked_fields {
            if self.owner(*source).or_else(|| self.owner(*target)) == Some(body) {
                self.unpacked_fields.insert((
                    self.scoped(index, *source),
                    self.scoped(index, *target),
                    *field,
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
                let ty = crate::types::substitute_resolved_type(ty, &self.contexts[index].bindings);
                let mut effects = cleanup_effects::Effects::default();
                self.checker
                    .drop_effects(&ty, *value, &mut HashSet::new(), &mut effects);
                self.cleanup_effects(index, index, *site, &effects);
            }
        }
        index
    }

    fn call(&mut self, caller: usize, call: &Call, body: Span, environment: Option<usize>) {
        if !self.graph.bodies.contains_key(&body) {
            return;
        }
        let bindings = self.call_bindings(caller, call, environment);
        let mut identity_bindings = bindings
            .iter()
            .map(|(name, ty)| (name.clone(), ty.clone()))
            .collect::<Vec<_>>();
        identity_bindings.sort_by(|left, right| left.0.cmp(&right.0));
        let key = Instance {
            caller,
            site: call.site,
            body,
            environment,
            bindings: identity_bindings,
        };
        let callee = if let Some(existing) = self.instances.get(&key) {
            *existing
        } else {
            let mut ancestor = Some(caller);
            let mut recursive = None;
            while let Some(index) = ancestor {
                if self.contexts[index].body == body && self.contexts[index].bindings == bindings {
                    recursive = Some(index);
                    break;
                }
                ancestor = self.contexts[index].parent;
            }
            let callee = recursive
                .unwrap_or_else(|| self.instantiate(body, Some(caller), environment, bindings));
            self.instances.insert(key, callee);
            callee
        };
        self.edges.insert((caller, callee));
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
        let outer = &self.contexts[environment.unwrap_or(caller)].bindings;
        if environment.is_some() {
            return outer.clone();
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
                bindings.extend(class.type_params.iter().zip(&class_type.arguments).map(
                    |(parameter, ty)| {
                        (
                            parameter.name.clone(),
                            crate::types::substitute_resolved_type(ty, outer),
                        )
                    },
                ));
            }
        }
        if let Some(pending) = self.checker.pending_generic_calls.get(&call.site) {
            bindings.extend(pending.bindings.iter().map(|(name, ty)| {
                (
                    name.clone(),
                    crate::types::substitute_resolved_type(
                        &self.checker.types.resolved(*ty),
                        outer,
                    ),
                )
            }));
        }
        bindings.extend(call.bindings.iter().map(|(name, ty)| {
            (
                name.clone(),
                crate::types::substitute_resolved_type(ty, outer),
            )
        }));
        bindings
    }

    fn cleanup_effects(
        &mut self,
        caller: usize,
        environment: usize,
        site: Span,
        effects: &cleanup_effects::Effects,
    ) {
        if effects.unknown || !effects.generic.is_empty() {
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
                for call in self.graph.calls.iter().filter(|call| call.owner == owner) {
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
                            self.graph.bodies.contains_key(target)
                                || self.graph.nonblocking_contracts.contains(target)
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
                                bindings: self.contexts[caller].bindings.clone(),
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
                            let ty = crate::types::substitute_resolved_type(
                                &ty,
                                &self.contexts[origin.environment].bindings,
                            );
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
        let mut extractions = HashMap::<ScopedValue, Vec<(ScopedValue, PayloadField)>>::new();
        for (source, target, field) in &self.unpacked_fields {
            extractions
                .entry(*source)
                .or_default()
                .push((*target, *field));
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
                    if selected == field {
                        self.flows.insert((*value, *target));
                    }
                }
                pending.extend(successors.get(&current).into_iter().flatten());
            }
        }
    }
}
