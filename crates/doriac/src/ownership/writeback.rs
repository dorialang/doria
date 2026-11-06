//! Replacement effects preserve a borrowed place's physical storage type.
//! Lifetime roots may widen to a subtree; a storage type must not widen with them.

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Home {
    Input(BorrowSource),
    Capture(BindingId),
    Context(CallbackCapturePlace),
    Typed(ResolvedType),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Effect {
    pub home: Home,
    pub value: Home,
    pub site: Span,
    // Named generic Copy parameters retain local value semantics.
    pub move_condition: Option<ResolvedType>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Summary {
    pub returns: Vec<Home>,
    pub writes: Vec<Effect>,
    pub invocations: Vec<Invocation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Input {
    pub homes: Vec<Home>,
    pub values: Vec<Home>,
    pub callables: Vec<Callable>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Invocation {
    pub callee: Callable,
    pub arguments: Vec<Input>,
    pub function: crate::types::SemanticFunctionType<ResolvedType>,
    pub site: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Callable {
    Input(usize),
    Capture(BindingId),
    Context(CallbackCapturePlace),
    Known(CallbackCaptureTarget),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Constraint {
    pub home: ResolvedType,
    pub value: ResolvedType,
    pub site: Span,
    pub write_site: Span,
}

pub(super) struct Analysis {
    pub constraints: Vec<Constraint>,
    parameters: HashMap<Span, Vec<String>>,
    properties: HashMap<(crate::types::ClassType<ResolvedType>, String), ResolvedType>,
}

impl Analysis {
    pub fn new(
        program: &ast::Program,
        classes: &[crate::semantics::ClassSemanticInfo],
        composition: &crate::trait_composition::CompositionPlan,
    ) -> Self {
        let mut parameters = HashMap::new();
        let mut index = |function: &ast::FunctionDecl| {
            parameters.insert(
                function.span,
                function
                    .type_params
                    .iter()
                    .map(|p| p.name.clone())
                    .collect(),
            );
        };
        for (function, _) in program_callables(program) {
            index(&function);
        }
        for (members, context) in composition
            .classes
            .values()
            .map(|class| {
                (
                    &class.members,
                    crate::property_hooks::PropertyHookContext::Class,
                )
            })
            .chain(
                composition
                    .source_trait_members
                    .values()
                    .map(|members| (members, crate::property_hooks::PropertyHookContext::Trait)),
            )
        {
            for member in members {
                for function in crate::property_hooks::member_callables(member, context) {
                    index(&function);
                }
            }
        }
        Self {
            constraints: Vec::new(),
            parameters,
            properties: classes
                .iter()
                .flat_map(|class| {
                    class.properties.iter().map(|property| {
                        (
                            (
                                crate::types::ClassType::new(
                                    class.declaration_name.clone(),
                                    class.arguments.clone(),
                                ),
                                property.name.clone(),
                            ),
                            property.ty.clone(),
                        )
                    })
                })
                .collect(),
        }
    }
}

pub(super) fn append<T: Clone + PartialEq>(destination: &mut Vec<T>, incoming: &[T]) {
    for value in incoming {
        if !destination.contains(value) {
            destination.push(value.clone());
        }
    }
}

impl Checker<'_> {
    fn writeback_callables(&self, expression: &Expr, scopes: &Scopes) -> Vec<Callable> {
        if self
            .resolved_type(expression)
            .and_then(non_null_function_type)
            .is_none()
        {
            return Vec::new();
        }
        if let Expr::Variable { name, .. } = ungroup_expr(expression) {
            if let Some(id) = scopes.get(name).and_then(|binding| binding.canonical_id) {
                if let Some(index) = self.retained.current.as_ref().and_then(|current| {
                    current
                        .parameters
                        .iter()
                        .position(|(parameter, _)| *parameter == Some(id))
                }) {
                    return vec![Callable::Input(index)];
                }
            }
        }
        let symbolic = self
            .writeback_homes(expression, scopes)
            .into_iter()
            .filter_map(|home| match home {
                Home::Input(BorrowSource::Parameter(index)) => Some(Callable::Input(index)),
                Home::Capture(id) => Some(Callable::Capture(id)),
                Home::Context(place) => Some(Callable::Context(place)),
                _ => None,
            })
            .collect::<Vec<_>>();
        if !symbolic.is_empty() {
            return symbolic;
        }
        if let Some(targets) = self
            .callback_value_targets
            .get(&expression.span())
            .filter(|targets| !targets.is_empty())
        {
            return targets.iter().cloned().map(Callable::Known).collect();
        }
        self.retained_value_from_expr(expression, scopes)
            .and_then(|value| value.closure_id)
            .map(|closure| {
                vec![Callable::Known(CallbackCaptureTarget {
                    declaration: Span {
                        source: closure.source,
                        expansion: closure.expansion,
                        start: closure.start,
                        end: closure.end,
                    },
                    captures: Vec::new(),
                })]
            })
            .unwrap_or_else(|| vec![Callable::Unknown])
    }

    pub(super) fn apply_callable_writeback(
        &mut self,
        site: Span,
        callee: &Expr,
        args: &[Argument],
        function: &crate::types::SemanticFunctionType<ResolvedType>,
        scopes: &Scopes,
    ) {
        let arguments = args
            .iter()
            .map(|arg| Input {
                homes: self.writeback_homes(&arg.value, scopes),
                values: self.writeback_values(&arg.value, scopes),
                callables: self.writeback_callables(&arg.value, scopes),
            })
            .collect::<Vec<_>>();
        for callee in self.writeback_callables(callee, scopes) {
            self.apply_writeback_invocation(
                site,
                Invocation {
                    callee,
                    arguments: arguments.clone(),
                    function: function.clone(),
                    site,
                },
                scopes,
            );
        }
    }

    fn writeback_binding_homes(&self, binding: &Binding) -> Vec<Home> {
        if !binding.writeback_homes.is_empty() {
            return binding.writeback_homes.clone();
        }
        let Some(id) = binding.canonical_id else {
            return Vec::new();
        };
        if binding.borrowed_place {
            if let Some(current) = &self.retained.current {
                if let Some(index) = current
                    .parameters
                    .iter()
                    .position(|(parameter, take)| *parameter == Some(id) && !take)
                {
                    return vec![Home::Input(BorrowSource::Parameter(index))];
                }
            }
            if self
                .binding_resolution
                .declarations_by_id
                .get(&id)
                .is_some_and(|d| d.kind == BindingKind::ClosureCapture)
            {
                return vec![Home::Capture(id)];
            }
        }
        self.binding_resolution
            .declarations_by_id
            .get(&id)
            .and_then(|d| d.source_type.clone())
            .map(Home::Typed)
            .into_iter()
            .collect()
    }

    pub(super) fn writeback_homes(&self, expression: &Expr, scopes: &Scopes) -> Vec<Home> {
        if self.creates_owned_interface_temporary(expression) {
            return vec![Home::Typed(
                self.interface_conversion_types[&expression.span()].clone(),
            )];
        }
        let expression = ungroup_expr(expression);
        match expression {
            Expr::Null { .. } => return Vec::new(),
            Expr::Variable { name, .. } => {
                if let Some(binding) = scopes.get(name) {
                    return self.writeback_binding_homes(binding);
                }
            }
            Expr::This { .. } => return vec![Home::Input(BorrowSource::Receiver)],
            Expr::Binary {
                left,
                op: BinaryOp::Coalesce,
                right,
                ..
            } => {
                let mut homes = Vec::new();
                for arm in self.coalesce_source_arms(left, right) {
                    append(&mut homes, &self.writeback_homes(arm, scopes));
                }
                return homes;
            }
            Expr::PropertyAccess {
                object, property, ..
            } if self.getter_signature(expression).is_none() => {
                if let Some(ResolvedType::Class(class)) =
                    self.resolved_type(object).map(projected_payload_type)
                {
                    if let Some(ty) = self
                        .writeback
                        .properties
                        .get(&(class.clone(), property.clone()))
                    {
                        return vec![Home::Typed(ty.clone())];
                    }
                }
            }
            Expr::Index { collection, .. } => {
                if let Some(ty) = self
                    .resolved_type(collection)
                    .map(projected_payload_type)
                    .and_then(element_type)
                {
                    return vec![Home::Typed(ty.clone())];
                }
            }
            _ => {}
        }
        if let Some((signature, receiver, args)) = self.retained_call(expression, scopes) {
            if signature
                .return_borrow
                .is_some_and(|borrow| borrow.kind == ReturnBorrowKind::Value)
            {
                if let Some(declaration) = signature
                    .declaration
                    .filter(|d| self.retained.analyzed_callables.contains(d))
                {
                    let homes = self
                        .retained
                        .callables
                        .get(&declaration)
                        .map(|s| s.writeback.returns.as_slice())
                        .unwrap_or_default();
                    return homes
                        .iter()
                        .flat_map(|home| {
                            self.instantiate_writeback_home(
                                home,
                                expression.span(),
                                receiver,
                                args,
                                &signature,
                                None,
                                scopes,
                                false,
                            )
                        })
                        .collect();
                }
            }
        }
        // Intrinsic getters expose storage of their checked result type. Their
        // layout and home transport are independently validated in MIR.
        self.resolved_type(expression)
            .cloned()
            .map(Home::Typed)
            .into_iter()
            .collect()
    }

    fn writeback_values(&self, expression: &Expr, scopes: &Scopes) -> Vec<Home> {
        if self.creates_owned_interface_temporary(expression) {
            return vec![Home::Typed(
                self.interface_conversion_types[&expression.span()].clone(),
            )];
        }
        if let Some(Fact::Constructed { origins, .. }) = self.flow_fact(expression) {
            let types = origins
                .iter()
                .filter_map(|origin| self.resolved_types.get(origin).cloned())
                .map(Home::Typed)
                .collect::<Vec<_>>();
            if !types.is_empty() {
                return types;
            }
        }
        if let Expr::Variable { name, .. } = ungroup_expr(expression) {
            if let Some(id) = scopes.get(name).and_then(|b| b.canonical_id) {
                if let Some(index) = self.retained.current.as_ref().and_then(|current| {
                    current
                        .parameters
                        .iter()
                        .position(|(parameter, _)| *parameter == Some(id))
                }) {
                    let declared = self
                        .binding_resolution
                        .declarations_by_id
                        .get(&id)
                        .and_then(|d| d.source_type.as_ref());
                    if declared == self.resolved_type(expression) {
                        return vec![Home::Input(BorrowSource::Parameter(index))];
                    }
                }
            }
        }
        self.resolved_type(expression)
            .cloned()
            .map(Home::Typed)
            .into_iter()
            .collect()
    }

    pub(super) fn record_writeback_return(&mut self, expression: &Expr, scopes: &Scopes) {
        if self.current_return_borrow.is_none() {
            return;
        }
        let homes = self.writeback_homes(expression, scopes);
        if let Some(current) = &self.retained.current {
            append(
                &mut self
                    .retained
                    .callables
                    .entry(current.declaration)
                    .or_default()
                    .writeback
                    .returns,
                &homes,
            );
        }
    }

    pub(super) fn record_writeback_assignment(
        &mut self,
        target: &Expr,
        value: &Expr,
        scopes: &Scopes,
        site: Span,
    ) {
        let Expr::Variable { name, .. } = ungroup_expr(target) else {
            return;
        };
        let Some(binding) = scopes.get(name) else {
            return;
        };
        let writes_original = binding.borrowed_place
            && binding.writable
            && self.retained.current.as_ref().is_some_and(|current| {
                binding.canonical_id.is_some_and(|id| {
                    current
                        .parameters
                        .iter()
                        .any(|(parameter, take)| *parameter == Some(id) && !take)
                        || self
                            .binding_resolution
                            .declarations_by_id
                            .get(&id)
                            .is_some_and(|d| d.kind == BindingKind::ClosureCapture)
                })
            });
        if !writes_original {
            return;
        }
        let homes = self.writeback_binding_homes(binding);
        let values = self.writeback_values(value, scopes);
        let move_condition = self
            .retained
            .current
            .as_ref()
            .filter(|current| !current.closure)
            .and(binding.canonical_id)
            .and_then(|id| self.binding_resolution.declarations_by_id.get(&id))
            .and_then(|declaration| declaration.source_type.clone());
        for home in homes {
            for value in &values {
                self.record_writeback_effect(
                    home.clone(),
                    value.clone(),
                    site,
                    site,
                    move_condition.clone(),
                );
            }
        }
    }

    fn record_writeback_effect(
        &mut self,
        home: Home,
        value: Home,
        site: Span,
        write_site: Span,
        mut move_condition: Option<ResolvedType>,
    ) {
        if let Some(ty) = &move_condition {
            if !resolved_type_requires_conservative_move(ty) {
                if !resolved_type_is_move_type(ty, &self.move_enum_names) {
                    return;
                }
                move_condition = None;
            }
        }
        if let (Home::Typed(home), Home::Typed(value), None) = (&home, &value, &move_condition) {
            append(
                &mut self.writeback.constraints,
                &[Constraint {
                    home: home.clone(),
                    value: value.clone(),
                    site,
                    write_site,
                }],
            );
        } else if let Some(current) = &self.retained.current {
            append(
                &mut self
                    .retained
                    .callables
                    .entry(current.declaration)
                    .or_default()
                    .writeback
                    .writes,
                &[Effect {
                    home,
                    value,
                    site: write_site,
                    move_condition,
                }],
            );
        }
    }

    pub(super) fn apply_writeback_effects(
        &mut self,
        site: Span,
        receiver: Option<&Expr>,
        args: &[Argument],
        signature: &Signature,
        scopes: &Scopes,
    ) {
        let targets = self
            .callback_capture_targets
            .get(&site)
            .cloned()
            .unwrap_or_default();
        let declarations = if targets.is_empty() {
            signature
                .declaration
                .map(|d| vec![(d, None)])
                .unwrap_or_default()
        } else {
            targets
                .iter()
                .map(|target| (target.declaration, Some(target)))
                .collect()
        };
        for (declaration, callback) in declarations {
            let summary = self
                .retained
                .callables
                .get(&declaration)
                .map(|s| s.writeback.clone())
                .unwrap_or_default();
            for effect in summary.writes {
                let move_condition = effect
                    .move_condition
                    .as_ref()
                    .map(|ty| self.substitute_writeback_type(ty, site, signature));
                let homes = self.instantiate_writeback_home(
                    &effect.home,
                    site,
                    receiver,
                    args,
                    signature,
                    callback,
                    scopes,
                    false,
                );
                let values = self.instantiate_writeback_home(
                    &effect.value,
                    site,
                    receiver,
                    args,
                    signature,
                    callback,
                    scopes,
                    true,
                );
                for home in homes {
                    for value in &values {
                        self.record_writeback_effect(
                            home.clone(),
                            value.clone(),
                            site,
                            effect.site,
                            move_condition.clone(),
                        );
                    }
                }
            }
            for invocation in summary.invocations {
                let mut arguments = Vec::new();
                for input in &invocation.arguments {
                    arguments.push(Input {
                        homes: input
                            .homes
                            .iter()
                            .flat_map(|home| {
                                self.instantiate_writeback_home(
                                    home, site, receiver, args, signature, callback, scopes, false,
                                )
                            })
                            .collect(),
                        values: input
                            .values
                            .iter()
                            .flat_map(|value| {
                                self.instantiate_writeback_home(
                                    value, site, receiver, args, signature, callback, scopes, true,
                                )
                            })
                            .collect(),
                        callables: input
                            .callables
                            .iter()
                            .flat_map(|callee| {
                                self.instantiate_writeback_callable(
                                    callee, site, args, signature, callback, scopes,
                                )
                            })
                            .collect(),
                    });
                }
                let function = self
                    .instantiate_writeback_home(
                        &Home::Typed(ResolvedType::Function(Box::new(invocation.function))),
                        site,
                        receiver,
                        args,
                        signature,
                        callback,
                        scopes,
                        true,
                    )
                    .into_iter()
                    .find_map(|home| match home {
                        Home::Typed(ResolvedType::Function(function)) => Some(*function),
                        _ => None,
                    })
                    .expect("specialization preserves function type");
                for callee in self.instantiate_writeback_callable(
                    &invocation.callee,
                    site,
                    args,
                    signature,
                    callback,
                    scopes,
                ) {
                    self.apply_writeback_invocation(
                        site,
                        Invocation {
                            callee,
                            arguments: arguments.clone(),
                            function: function.clone(),
                            site: invocation.site,
                        },
                        scopes,
                    );
                }
            }
        }
    }

    fn apply_writeback_invocation(&mut self, site: Span, invocation: Invocation, scopes: &Scopes) {
        let mut pending = vec![invocation];
        let mut visited = Vec::new();
        while let Some(invocation) = pending.pop() {
            if visited.contains(&invocation) {
                continue;
            }
            visited.push(invocation.clone());
            match &invocation.callee {
                Callable::Input(_) | Callable::Capture(_) | Callable::Context(_) => {
                    if let Some(current) = &self.retained.current {
                        append(
                            &mut self
                                .retained
                                .callables
                                .entry(current.declaration)
                                .or_default()
                                .writeback
                                .invocations,
                            &[invocation],
                        );
                    }
                }
                Callable::Known(target) => {
                    let summary = self
                        .retained
                        .callables
                        .get(&target.declaration)
                        .map(|s| s.writeback.clone())
                        .unwrap_or_default();
                    let signature = Signature::for_function_value(
                        &invocation.function,
                        Some(target.declaration),
                        &self.move_enum_names,
                    );
                    let resolve_home = |home: &Home, value: bool| match home {
                        Home::Input(BorrowSource::Parameter(index)) => invocation
                            .arguments
                            .get(*index)
                            .map(|input| {
                                if value {
                                    input.values.clone()
                                } else {
                                    input.homes.clone()
                                }
                            })
                            .unwrap_or_default(),
                        _ => self.instantiate_writeback_home(
                            home,
                            site,
                            None,
                            &[],
                            &signature,
                            Some(target),
                            scopes,
                            value,
                        ),
                    };
                    let resolve_callable = |callee: &Callable| match callee {
                        Callable::Input(index) => invocation
                            .arguments
                            .get(*index)
                            .map(|input| input.callables.clone())
                            .unwrap_or_default(),
                        _ => self.instantiate_writeback_callable(
                            callee,
                            site,
                            &[],
                            &signature,
                            Some(target),
                            scopes,
                        ),
                    };
                    let mut effects = Vec::new();
                    for effect in summary.writes {
                        let move_condition = effect
                            .move_condition
                            .as_ref()
                            .map(|ty| self.substitute_writeback_type(ty, site, &signature));
                        for home in resolve_home(&effect.home, false) {
                            for value in resolve_home(&effect.value, true) {
                                effects.push((
                                    home.clone(),
                                    value,
                                    effect.site,
                                    move_condition.clone(),
                                ));
                            }
                        }
                    }
                    for nested in summary.invocations {
                        let arguments = nested
                            .arguments
                            .iter()
                            .map(|input| Input {
                                homes: input
                                    .homes
                                    .iter()
                                    .flat_map(|home| resolve_home(home, false))
                                    .collect(),
                                values: input
                                    .values
                                    .iter()
                                    .flat_map(|value| resolve_home(value, true))
                                    .collect(),
                                callables: input
                                    .callables
                                    .iter()
                                    .flat_map(resolve_callable)
                                    .collect(),
                            })
                            .collect::<Vec<_>>();
                        for callee in resolve_callable(&nested.callee) {
                            pending.push(Invocation {
                                callee,
                                arguments: arguments.clone(),
                                function: nested.function.clone(),
                                site: nested.site,
                            });
                        }
                    }
                    for (home, value, write_site, move_condition) in effects {
                        self.record_writeback_effect(home, value, site, write_site, move_condition);
                    }
                }
                Callable::Unknown => {
                    // Without a proved body, the full written replacement domain
                    // is the only guarantee supplied by a writable parameter.
                    for (input, parameter) in invocation
                        .arguments
                        .into_iter()
                        .zip(invocation.function.parameters)
                    {
                        if parameter.ownership_mode
                            == crate::types::FunctionTypeParameterMode::Writable
                        {
                            for home in input.homes {
                                self.record_writeback_effect(
                                    home,
                                    Home::Typed(parameter.ty.clone()),
                                    site,
                                    invocation.site,
                                    None,
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    fn instantiate_writeback_callable(
        &self,
        callee: &Callable,
        site: Span,
        args: &[Argument],
        signature: &Signature,
        target: Option<&CallbackCaptureTarget>,
        scopes: &Scopes,
    ) -> Vec<Callable> {
        match callee {
            Callable::Input(index) => signature
                .bind_arguments(args)
                .param_to_arg
                .get(*index)
                .copied()
                .flatten()
                .and_then(|i| args.get(i))
                .map(|arg| self.writeback_callables(&arg.value, scopes))
                .unwrap_or_default(),
            Callable::Capture(id) => {
                let Some(target) = target else {
                    return vec![callee.clone()];
                };
                if let Some((_, places)) = target
                    .captures
                    .iter()
                    .find(|(capture, _)| capture == id)
                    .filter(|(_, places)| !places.is_empty())
                {
                    return places
                        .iter()
                        .flat_map(|place| self.writeback_callable_context(place, site, scopes))
                        .collect();
                }
                self.closures
                    .get(&ClosureId::from_span(target.declaration))
                    .and_then(|info| {
                        info.captures
                            .iter()
                            .find(|capture| capture.environment_binding_id == *id)
                    })
                    .map(|capture| {
                        self.writeback_callable_context(
                            &CallbackCapturePlace {
                                source: CallbackCaptureSource::Binding(capture.source_binding_id),
                                callers: Vec::new(),
                            },
                            site,
                            scopes,
                        )
                    })
                    .unwrap_or_else(|| vec![callee.clone()])
            }
            Callable::Context(place) => self.writeback_callable_context(place, site, scopes),
            _ => vec![callee.clone()],
        }
    }

    fn writeback_callable_context(
        &self,
        place: &CallbackCapturePlace,
        site: Span,
        scopes: &Scopes,
    ) -> Vec<Callable> {
        if let Some((next, remaining)) = place.callers.split_first() {
            if *next != site {
                return vec![Callable::Context(place.clone())];
            }
            return self.writeback_callable_context(
                &CallbackCapturePlace {
                    source: place.source,
                    callers: remaining.to_vec(),
                },
                site,
                scopes,
            );
        }
        match place.source {
            CallbackCaptureSource::Expression(span) => self
                .retained
                .pending_expressions
                .get(&span)
                .map(|expr| self.writeback_callables(expr, scopes))
                .unwrap_or_else(|| vec![Callable::Context(place.clone())]),
            CallbackCaptureSource::Binding(id) => {
                let Some((name, binding)) = scopes.get_by_canonical(id) else {
                    return vec![Callable::Context(place.clone())];
                };
                if let Some(index) = self.retained.current.as_ref().and_then(|current| {
                    current
                        .parameters
                        .iter()
                        .position(|(parameter, _)| *parameter == Some(id))
                }) {
                    return vec![Callable::Input(index)];
                }
                if let Some(closure) = binding
                    .retained_value
                    .as_ref()
                    .and_then(|value| value.closure_id)
                {
                    return vec![Callable::Known(CallbackCaptureTarget {
                        declaration: Span {
                            source: closure.source,
                            expansion: closure.expansion,
                            start: closure.start,
                            end: closure.end,
                        },
                        captures: Vec::new(),
                    })];
                }
                let _ = name;
                vec![Callable::Unknown]
            }
        }
    }

    fn substitute_writeback_type(
        &self,
        ty: &ResolvedType,
        site: Span,
        signature: &Signature,
    ) -> ResolvedType {
        crate::types::substitute_resolved_type(ty, &self.call_type_substitutions(site, signature))
    }

    pub(super) fn call_type_substitutions(
        &self,
        site: Span,
        signature: &Signature,
    ) -> HashMap<String, ResolvedType> {
        let mut substitutions = HashMap::new();
        if let Some(crate::semantics::CallableTarget::Method { class_type, .. }) =
            self.call_targets.get(&site).or_else(|| {
                self.property_accessor_calls
                    .get(&site)
                    .and_then(|calls| calls.getter.as_ref())
                    .map(|getter| &getter.target)
            })
        {
            if let Some(parameters) = self.receiver_type_params.get(&class_type.name) {
                substitutions.extend(
                    parameters
                        .iter()
                        .zip(&class_type.arguments)
                        .map(|(parameter, ty)| (parameter.name.clone(), ty.clone())),
                );
            }
        }
        if let Some((parameters, specialization)) = signature
            .declaration
            .and_then(|d| self.writeback.parameters.get(&d))
            .zip(self.generic_call_specializations.get(&site))
        {
            substitutions.extend(parameters.iter().zip(&specialization.arguments).map(
                |(name, crate::semantics::GenericArgument::Type(ty))| (name.clone(), ty.clone()),
            ));
        }
        substitutions
    }

    #[allow(clippy::too_many_arguments)]
    fn instantiate_writeback_home(
        &self,
        home: &Home,
        site: Span,
        receiver: Option<&Expr>,
        args: &[Argument],
        signature: &Signature,
        callback: Option<&CallbackCaptureTarget>,
        scopes: &Scopes,
        value: bool,
    ) -> Vec<Home> {
        match home {
            Home::Input(source) => self
                .call_borrow_source_expr(
                    ReturnBorrow {
                        kind: ReturnBorrowKind::Value,
                        source: *source,
                        writable: true,
                        lifetime: RetainedLifetime::LastUse,
                    },
                    receiver,
                    signature,
                    args,
                )
                .map(|input| {
                    if value {
                        self.writeback_values(input, scopes)
                    } else {
                        self.writeback_homes(input, scopes)
                    }
                })
                .unwrap_or_default(),
            Home::Typed(ty) => vec![Home::Typed(
                self.substitute_writeback_type(ty, site, signature),
            )],
            Home::Capture(id) => {
                let Some(target) = callback else {
                    return vec![home.clone()];
                };
                let capture = self
                    .closures
                    .get(&ClosureId::from_span(target.declaration))
                    .and_then(|info| {
                        info.captures
                            .iter()
                            .find(|capture| capture.environment_binding_id == *id)
                    });
                let Some(capture) = capture else {
                    return vec![home.clone()];
                };
                if capture.mode == ast::ClosureCaptureMode::Take {
                    return vec![Home::Typed(capture.source_type.clone())];
                }
                if let Some((_, places)) = target
                    .captures
                    .iter()
                    .find(|(captured, _)| captured == id)
                    .filter(|(_, places)| !places.is_empty())
                {
                    return places
                        .iter()
                        .flat_map(|place| self.writeback_context(place, site, scopes, value))
                        .collect();
                }
                self.writeback_context(
                    &CallbackCapturePlace {
                        source: CallbackCaptureSource::Binding(capture.source_binding_id),
                        callers: Vec::new(),
                    },
                    site,
                    scopes,
                    value,
                )
            }
            Home::Context(place) => self.writeback_context(place, site, scopes, value),
        }
    }

    fn writeback_context(
        &self,
        place: &CallbackCapturePlace,
        site: Span,
        scopes: &Scopes,
        value: bool,
    ) -> Vec<Home> {
        if let Some((next, remaining)) = place.callers.split_first() {
            if *next != site {
                return vec![Home::Context(place.clone())];
            }
            return self.writeback_context(
                &CallbackCapturePlace {
                    source: place.source,
                    callers: remaining.to_vec(),
                },
                site,
                scopes,
                value,
            );
        }
        match place.source {
            CallbackCaptureSource::Expression(span) => self
                .retained
                .pending_expressions
                .get(&span)
                .map(|expression| {
                    if value {
                        self.writeback_values(expression, scopes)
                    } else {
                        self.writeback_homes(expression, scopes)
                    }
                })
                .unwrap_or_else(|| vec![Home::Context(place.clone())]),
            CallbackCaptureSource::Binding(id) => scopes
                .get_by_canonical(id)
                .map(|(_, binding)| self.writeback_binding_homes(binding))
                .unwrap_or_else(|| vec![Home::Context(place.clone())]),
        }
    }
}

fn nonnullable(ty: &ResolvedType) -> &ResolvedType {
    match ty {
        ResolvedType::Nullable(inner) => inner,
        _ => ty,
    }
}

fn projected_payload_type(ty: &ResolvedType) -> &ResolvedType {
    match nonnullable(ty) {
        ResolvedType::SharedHandle(kind, payload) if kind.forwards_payload() => {
            projected_payload_type(payload)
        }
        ty => ty,
    }
}

fn element_type(ty: &ResolvedType) -> Option<&ResolvedType> {
    match ty {
        ResolvedType::TypedArray(element)
        | ResolvedType::List(element)
        | ResolvedType::Deque(element) => Some(element),
        ResolvedType::Dictionary(_, value) | ResolvedType::SortedDictionary(_, value) => {
            Some(value)
        }
        _ => None,
    }
}
