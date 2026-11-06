use super::*;

#[derive(Debug)]
pub(super) enum Input {
    Existing(String),
    Value { owned: bool, cell: bool },
}

impl Input {
    pub(super) fn borrowed_cell(value: &Expr, scopes: &PhpNameScopes) -> Self {
        Self::for_value(value, false, true, scopes)
    }

    pub(super) fn named(
        value: &Expr,
        parameter: Option<&crate::php_closure::PhpCallableParameter>,
        scopes: &PhpNameScopes,
    ) -> Self {
        let owned = parameter.is_some_and(|parameter| parameter.take);
        let cell = parameter.is_some_and(|parameter| parameter.cell);
        Self::for_value(value, owned, cell, scopes)
    }

    pub(super) fn callback(
        value: &Expr,
        index: usize,
        function: &mir::FunctionType,
        scopes: &PhpNameScopes,
    ) -> Self {
        let mode = function.parameters[index].mode;
        let cell = scopes.closure_plan.parameter_uses_cell(function.id, index);
        if mode == mir::FunctionParameterMode::Writable {
            return borrowed_argument_cell(value, scopes)
                .map(Self::Existing)
                .unwrap_or(Self::Value {
                    owned: false,
                    cell: false,
                });
        }
        let owned = mode == mir::FunctionParameterMode::Take;
        Self::for_value(value, owned, cell, scopes)
    }

    fn for_value(value: &Expr, owned: bool, cell: bool, scopes: &PhpNameScopes) -> Self {
        if cell {
            let existing = if owned && expression_has_move_ownership(value, scopes) {
                assignment_target_cell(value, scopes)
            } else if owned {
                None
            } else {
                borrowed_argument_cell(value, scopes)
            };
            if let Some(existing) = existing {
                return Self::Existing(existing);
            }
        }
        Self::Value { owned, cell }
    }

    pub(super) fn expression(self, expr: &Expr, scopes: &PhpNameScopes) -> String {
        match self {
            Self::Existing(value) => value,
            Self::Value { owned, cell } => {
                if cell && !owned {
                    if let Some(value) = value_result::borrowed_cell_expression(expr, scopes) {
                        return value;
                    }
                }
                let value = if owned {
                    emit_owned_expr(expr, scopes)
                } else {
                    emit_expr(expr, scopes)
                };
                Self::finish(value, expr.span(), owned, cell, scopes)
            }
        }
    }

    fn stage(
        self,
        expr: &Expr,
        output: &mut String,
        indent: usize,
        scopes: &mut PhpNameScopes,
    ) -> String {
        match self {
            Self::Existing(value) => value,
            Self::Value { owned, cell } => {
                if cell && !owned {
                    if let Some(value) =
                        value_result::statement_borrowed_cell(expr, output, indent, scopes)
                    {
                        return value;
                    }
                }
                let value = value_result::statement_value(expr, owned, output, indent, scopes)
                    .unwrap_or_else(|| {
                        if owned {
                            emit_owned_expr(expr, scopes)
                        } else {
                            emit_expr(expr, scopes)
                        }
                    });
                Self::finish(value, expr.span(), owned, cell, scopes)
            }
        }
    }

    fn finish(
        value: String,
        span: Span,
        owned: bool,
        cell: bool,
        scopes: &PhpNameScopes,
    ) -> String {
        if !cell {
            return value;
        }
        let value = format!("new __DoriaCell({value})");
        if owned {
            stage_owned_argument_cell(span, value, scopes)
        } else {
            value
        }
    }
}

#[derive(Debug)]
pub(super) struct Receiver {
    expression: *const Expr,
    producer: *const Expr,
    value: String,
    cell: String,
}

impl Receiver {
    pub(super) fn prepare(
        expr: &Expr,
        output: &mut String,
        indent: usize,
        scopes: &mut PhpNameScopes,
    ) -> Option<Self> {
        let Expr::PropertyAccess { object, .. } = expr else {
            return None;
        };
        // Keep stored places and specialized field channels out of value snapshots.
        if expression_region::simple_scalar(object)
            || matches!(expression_region::ungrouped(object), Expr::This { .. })
            || matches!(expression_region::ungrouped(object), Expr::Index { .. })
            || matches!(expression_region::ungrouped(object), Expr::PropertyAccess { span, .. }
                if scopes.specialization.accessor(*span, crate::ast::PropertyHookKind::Get,
                    &scopes.substitutions).is_none())
            || matches!(expression_region::ungrouped(object), Expr::StaticMember { class_name, member, .. }
                if scopes.is_static_property(class_name, member))
            || value_result::has_field_value(object, scopes)
            || scopes.scalar_operands.contains_key(&(expr as *const Expr))
        {
            return None;
        }
        let returned_cell =
            expression_returns_borrow_cell(expression_region::ungrouped(object), scopes);
        let owned = !returned_cell
            && expression_creates_owner(expression_region::ungrouped(object), scopes);
        let name = scopes.fresh_temp(if returned_cell {
            "__doria_receiver_cell"
        } else {
            "__doria_property_receiver"
        });
        scopes.track_expression_temporary(if owned {
            PhpExpressionTemporary::Owned(name.clone())
        } else {
            PhpExpressionTemporary::Alias(name.clone())
        });
        expression_region::emit(object, output, indent, scopes, |output, indent, scopes| {
            if returned_cell {
                let producer = expression_region::ungrouped(object);
                let value = expression_region::guarded(
                    producer,
                    emit_expr_unboxed_impl(producer, scopes),
                    true,
                    scopes,
                );
                expression_region::consume(output, indent, scopes);
                writeln(output, indent, &format!("${name} = {value};"));
                return;
            }
            value_result::emit_value_into(object, &name, owned, output, indent, scopes);
        });
        Some(Self {
            expression: object.as_ref(),
            producer: expression_region::ungrouped(object),
            value: if returned_cell {
                Self::cell_value(object, &format!("${name}"), scopes)
            } else {
                format!("${name}")
            },
            cell: if returned_cell {
                format!("${name}")
            } else {
                format!("__DoriaCell::borrowed(${name})")
            },
        })
    }

    pub(super) fn value(&self, expr: &Expr) -> Option<String> {
        std::ptr::eq(self.expression, expr).then(|| self.value.clone())
    }

    pub(super) fn cell(&self, expr: &Expr) -> Option<String> {
        std::ptr::eq(self.producer, expression_region::ungrouped(expr)).then(|| self.cell.clone())
    }

    fn cell_value(expr: &Expr, cell: &str, scopes: &PhpNameScopes) -> String {
        let value = if let Expr::Grouped { expr: inner, .. } = expr {
            format!("({})", Self::cell_value(inner, cell, scopes))
        } else {
            format!("({cell})?->value")
        };
        emit_expr_value(expr, value, scopes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Transport {
    Boxed,
    FieldResult,
    FieldArguments,
    FieldArgumentsAndResult,
}

impl Transport {
    pub(super) fn for_call(arguments: bool, result: bool) -> Self {
        match (arguments, result) {
            (false, false) => Self::Boxed,
            (false, true) => Self::FieldResult,
            (true, false) => Self::FieldArguments,
            (true, true) => Self::FieldArgumentsAndResult,
        }
    }

    pub(super) fn callback_entry(self) -> &'static str {
        match self {
            Self::Boxed => "__invoke",
            Self::FieldResult => "__doriaFieldResult",
            Self::FieldArguments => "__doriaFieldArguments",
            Self::FieldArgumentsAndResult => "__doriaFieldArgumentsResult",
        }
    }

    pub(super) fn arguments(self) -> bool {
        matches!(self, Self::FieldArguments | Self::FieldArgumentsAndResult)
    }

    pub(super) fn result(self) -> bool {
        matches!(self, Self::FieldResult | Self::FieldArgumentsAndResult)
    }
}

pub(super) fn expression(expr: &Expr, scopes: &PhpNameScopes) -> Option<String> {
    if let Some(prepared) = scopes
        .prepared_call
        .as_ref()
        .filter(|prepared| std::ptr::eq(prepared.expression, expr))
    {
        return Some(prepared.render(scopes));
    }
    let (span, args) = match expr {
        Expr::FunctionCall { span, args, .. }
        | Expr::MethodCall { span, args, .. }
        | Expr::StaticCall { span, args, .. } => (*span, args),
        Expr::CallableCall(_) => return Some(expression_region::expression(expr, scopes)),
        _ => return None,
    };
    if scopes
        .field_arguments
        .as_ref()
        .is_some_and(|(active, _)| *active == span)
    {
        return None;
    }
    let signature = Signature::named(span, scopes)?;
    let CallableSignature::Named(callable) = &signature.callable else {
        unreachable!();
    };
    if callable.consumes_receiver {
        return None;
    }
    let parameters = &signature.fields;
    if !parameters.iter().any(Option::is_some) {
        return None;
    }
    // Validate every scalar input before rendering operands or acquiring claims.
    let bound = signature.bind(args);
    let prepared = args
        .iter()
        .enumerate()
        .map(|(written, argument)| {
            let index = bound.arg_to_param[written]?;
            let values = match parameters.get(index)? {
                Some(ty) => Some(value_result::argument_values(&argument.value, ty, scopes)?),
                None => None,
            };
            Some((index, values))
        })
        .collect::<Option<Vec<_>>>()?;
    let mut values = Vec::new();
    for (argument, (index, fields)) in args.iter().zip(prepared) {
        if let Some(fields) = fields {
            let names =
                value_result::parameter_names(parameters[index].as_ref().unwrap(), index, scopes);
            values.extend(fields.into_iter().zip(names).map(|(value, name)| {
                if argument.name.is_some() {
                    format!("{name}: {value}")
                } else {
                    value
                }
            }));
        } else {
            let value = emit_call_argument(&argument.value, callable.parameters.get(index), scopes);
            values.push(
                argument
                    .name
                    .as_ref()
                    .map_or(value.clone(), |name| format!("{}: {value}", name.text)),
            );
        }
    }
    signature.append_defaults(&bound, &mut values, scopes);
    let mut call_scopes = scopes.clone();
    call_scopes.field_arguments = Some((span, values));
    Some(emit_expr_unboxed_impl(expr, &call_scopes))
}

#[derive(Debug)]
pub(super) struct Prepared {
    expression: *const Expr,
    span: Span,
    invocation: Invocation,
    arguments: Vec<String>,
    field_arguments: bool,
}

#[derive(Debug)]
enum Invocation {
    Function,
    Static {
        class: String,
        home: Option<String>,
    },
    Method {
        receiver: String,
        home: Option<String>,
        null_safe: bool,
    },
    Setter {
        receiver: String,
        home: Option<String>,
        symbol: String,
    },
    Callback {
        callee: String,
    },
}

impl Prepared {
    pub(super) fn render(&self, scopes: &PhpNameScopes) -> String {
        let mut scopes = scopes.clone();
        if self.field_arguments {
            scopes.field_arguments = Some((self.span, self.arguments.clone()));
        }
        let mut arguments = self.arguments.clone();
        match &self.invocation {
            Invocation::Callback { callee } => {
                let entry =
                    Transport::for_call(self.field_arguments, scopes.field_call == Some(self.span))
                        .callback_entry();
                format!("(${callee})->{entry}({})", arguments.join(", "))
            }
            invocation => {
                let symbol = if let Invocation::Setter { symbol, .. } = invocation {
                    if self.field_arguments {
                        scopes.specialization.argument_symbol(symbol, false)
                    } else {
                        symbol
                    }
                } else {
                    scopes.call_symbol(self.span).unwrap_or_else(|| {
                        panic!(
                            "checked field argument entry for {invocation:?} at {:?}",
                            self.span
                        )
                    })
                };
                let (callee, home) = match invocation {
                    Invocation::Function => (symbol.to_string(), None),
                    Invocation::Static { class, home } => {
                        (format!("{class}::{symbol}"), home.as_ref())
                    }
                    Invocation::Method {
                        receiver,
                        home,
                        null_safe,
                    } => (
                        format!(
                            "{receiver}{}{symbol}",
                            if *null_safe { "?->" } else { "->" }
                        ),
                        home.as_ref(),
                    ),
                    Invocation::Setter { receiver, home, .. } => {
                        (format!("{receiver}->{symbol}"), home.as_ref())
                    }
                    Invocation::Callback { .. } => unreachable!(),
                };
                if let Some(home) = home {
                    arguments.insert(0, home.clone());
                }
                format!("{callee}({})", arguments.join(", "))
            }
        }
    }
}

struct Signature {
    callable: CallableSignature,
    fields: Vec<Option<ResolvedType>>,
    defaults: Vec<Option<ConstValue>>,
}

enum CallableSignature {
    Named(crate::php_closure::PhpCallablePlan),
    Callback(mir::FunctionType),
}

impl Signature {
    fn named(span: Span, scopes: &PhpNameScopes) -> Option<Self> {
        Some(Self {
            callable: CallableSignature::Named(scopes.closure_plan.callable_at(span)?.clone()),
            fields: scopes
                .specialization
                .argument_fields(span, &scopes.substitutions)?
                .to_vec(),
            defaults: scopes
                .specialization
                .argument_defaults(span, &scopes.substitutions)?,
        })
    }

    fn bind(&self, arguments: &[Argument]) -> crate::arg_binding::BoundArguments {
        let names = match &self.callable {
            CallableSignature::Named(callable) => callable
                .parameters
                .iter()
                .map(|parameter| parameter.name.as_str())
                .collect(),
            CallableSignature::Callback(function) => vec![""; function.parameters.len()],
        };
        let defaults = self
            .defaults
            .iter()
            .map(Option::is_some)
            .collect::<Vec<_>>();
        let arguments = arguments
            .iter()
            .map(|argument| match &self.callable {
                CallableSignature::Named(_) => {
                    argument.name.as_ref().map(|name| name.text.as_str())
                }
                CallableSignature::Callback(_) => None,
            })
            .collect::<Vec<_>>();
        let bound = crate::arg_binding::bind_arguments(&names, &defaults, &arguments);
        assert!(
            bound.unknown.is_empty()
                && bound.duplicate.is_empty()
                && bound.missing.is_empty()
                && bound.overflow == 0,
            "checked call arguments have a complete binding"
        );
        bound
    }

    fn append_defaults(
        &self,
        bound: &crate::arg_binding::BoundArguments,
        values: &mut Vec<String>,
        scopes: &PhpNameScopes,
    ) {
        for (index, argument) in bound.param_to_arg.iter().enumerate() {
            if argument.is_some() {
                continue;
            }
            let value = self.defaults[index]
                .as_ref()
                .expect("checked omitted parameter has a default");
            let CallableSignature::Named(callable) = &self.callable else {
                unreachable!("callback types have full arity");
            };
            let (names, defaults) = if let Some(ty) = &self.fields[index] {
                (
                    value_result::parameter_names(ty, index, scopes),
                    value_result::constant_arguments(value, ty, scopes),
                )
            } else {
                let parameter = &callable.parameters[index];
                let value = emit_const_value(value, &scopes.specialization);
                (
                    vec![parameter.name.clone()],
                    vec![if parameter.cell {
                        format!("new __DoriaCell({value})")
                    } else {
                        value
                    }],
                )
            };
            values.extend(
                names
                    .into_iter()
                    .zip(defaults)
                    .map(|(name, value)| format!("{name}: {value}")),
            );
        }
    }

    fn takes(&self, index: usize) -> bool {
        match &self.callable {
            CallableSignature::Named(callable) => callable.parameters[index].take,
            CallableSignature::Callback(function) => {
                function.parameters[index].mode == mir::FunctionParameterMode::Take
            }
        }
    }

    fn argument(&self, argument: &Expr, index: usize, scopes: &PhpNameScopes) -> Input {
        match &self.callable {
            CallableSignature::Named(callable) => {
                Input::named(argument, callable.parameters.get(index), scopes)
            }
            CallableSignature::Callback(function) => {
                Input::callback(argument, index, function, scopes)
            }
        }
    }

    fn stage_argument(
        &self,
        value: &Expr,
        index: usize,
        started: &str,
        output: &mut String,
        indent: usize,
        scopes: &mut PhpNameScopes,
    ) -> Vec<String> {
        if let Some(ty) = &self.fields[index] {
            value_result::Place::argument(value, ty, output, indent, scopes)
                .names()
                .map(|name| format!("${name}"))
                .collect()
        } else {
            let name = stage_expression(
                value,
                self.takes(index),
                started,
                output,
                indent,
                scopes,
                |output, indent, scopes| {
                    self.argument(value, index, scopes)
                        .stage(value, output, indent, scopes)
                },
            );
            vec![format!("${name}")]
        }
    }
}

pub(super) fn prepare(
    expr: &Expr,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> Option<Prepared> {
    let (span, arguments, signature) = match expr {
        Expr::FunctionCall { span, args, .. }
        | Expr::StaticCall { span, args, .. }
        | Expr::MethodCall { span, args, .. } => (*span, args, Signature::named(*span, scopes)?),
        Expr::CallableCall(call) => {
            let function = scopes
                .closure_plan
                .callable_value_type(scopes.closure_owner, call.span)
                .clone();
            let fields = scopes.specialization.function_argument_fields(&function);
            (
                call.span,
                &call.args,
                Signature {
                    defaults: vec![None; function.parameters.len()],
                    callable: CallableSignature::Callback(function),
                    fields,
                },
            )
        }
        _ => return None,
    };
    let field_arguments = signature.fields.iter().any(Option::is_some);
    if !matches!(&signature.callable, CallableSignature::Callback(_))
        && !field_arguments
        && !value_result::field_call(expr, scopes)
        && arguments
            .iter()
            .all(|argument| expression_region::simple_scalar(&argument.value))
        && !matches!(expr, Expr::MethodCall { object, .. }
            if !expression_region::simple_scalar(object)
                && !matches!(expression_region::ungrouped(object), Expr::This { .. }))
    {
        return None;
    }
    let started = scopes.fresh_temp("__doria_call_started");
    scopes.track_expression_temporary(PhpExpressionTemporary::Alias(started.clone()));
    writeln(output, indent, &format!("${started} = false;"));
    let (invocation, presence) = match expr {
        Expr::FunctionCall { .. } => (Invocation::Function, None),
        Expr::StaticCall { class_name, .. } => {
            let parent = scopes.direct_parent_calls.contains(&span);
            let CallableSignature::Named(callable) = &signature.callable else {
                unreachable!()
            };
            if parent && callable.consumes_receiver {
                let receiver = scopes
                    .lookup("this")
                    .map(|cell| format!("__doria_take_cell(${cell})"))
                    .expect("consuming parent receiver has an owned home");
                stage_value(receiver, true, &started, output, indent, scopes);
            }
            let home = if parent && callable.caller_receiver_home() {
                let Some(mir::ClosureOwner::Callable(instance)) = scopes.closure_owner else {
                    unreachable!()
                };
                let binding = scopes
                    .closure_plan
                    .binding_resolution
                    .receiver_for_callable(scopes.specialization.callable(instance).declaration)
                    .and_then(|binding| scopes.place(binding))
                    .and_then(PhpBindingPlace::cell)
                    .expect("parent receiver has a checked storage home");
                Some(format!(
                    "${}",
                    stage_value(binding, false, &started, output, indent, scopes)
                ))
            } else {
                None
            };
            let class = if parent {
                "parent".to_string()
            } else {
                scopes
                    .specialization
                    .class_symbol_for_call(span, &scopes.substitutions)
                    .map(str::to_string)
                    .unwrap_or_else(|| php_type_symbol(class_name))
            };
            (Invocation::Static { class, home }, None)
        }
        Expr::MethodCall {
            object, null_safe, ..
        } => {
            let CallableSignature::Named(callable) = &signature.callable else {
                unreachable!()
            };
            let home = callable.caller_receiver_home();
            let receiver = stage_expression(
                object,
                callable.consumes_receiver,
                &started,
                output,
                indent,
                scopes,
                |output, indent, scopes| {
                    if callable.consumes_receiver {
                        if let Some(value) =
                            value_result::statement_value(object, true, output, indent, scopes)
                        {
                            return value;
                        }
                        emit_owned_expr(object, scopes)
                    } else if home {
                        emit_receiver_borrow_home(object, *null_safe, scopes)
                    } else {
                        if let Some(value) =
                            value_result::statement_value(object, false, output, indent, scopes)
                        {
                            return property_hooks::receiver_value(
                                object, value, *null_safe, scopes,
                            );
                        }
                        property_hooks::receiver(object, *null_safe, scopes)
                    }
                },
            );
            let physical_home = home.then(|| format!("${receiver}"));
            let receiver = if home {
                format!("${receiver}{}value", if *null_safe { "?->" } else { "->" })
            } else {
                format!("${receiver}")
            };
            let presence = null_safe.then(|| format!("{receiver} !== null"));
            (
                Invocation::Method {
                    receiver,
                    home: physical_home,
                    null_safe: *null_safe,
                },
                presence,
            )
        }
        Expr::CallableCall(call) => {
            let CallableSignature::Callback(function) = &signature.callable else {
                unreachable!()
            };
            let once = function.invocation_mode == mir::FunctionInvocationMode::Once;
            let callee = stage_expression(
                &call.callee,
                once,
                &started,
                output,
                indent,
                scopes,
                |output, indent, scopes| {
                    if let Some(value) =
                        value_result::statement_value(&call.callee, once, output, indent, scopes)
                    {
                        return value;
                    }
                    if once {
                        assignment_target_cell(&call.callee, scopes)
                            .map(|cell| format!("__doria_take_cell({cell})"))
                            .unwrap_or_else(|| emit_expr(&call.callee, scopes))
                    } else {
                        emit_expr(&call.callee, scopes)
                    }
                },
            );
            (Invocation::Callback { callee }, None)
        }
        _ => unreachable!(),
    };
    if let Some(presence) = &presence {
        writeln(output, indent, &format!("if ({presence}) {{"));
    }
    let setup_indent = indent + usize::from(presence.is_some());
    let mut values = Vec::new();
    let bound = signature.bind(arguments);
    for (written, argument) in arguments.iter().enumerate() {
        let index = bound.arg_to_param[written].expect("checked argument binding");
        let staged = signature.stage_argument(
            &argument.value,
            index,
            &started,
            output,
            setup_indent,
            scopes,
        );
        if let Some(label) = &argument.name {
            let names = signature.fields[index].as_ref().map_or_else(
                || vec![label.text.clone()],
                |ty| value_result::parameter_names(ty, index, scopes),
            );
            values.extend(
                staged
                    .into_iter()
                    .zip(names)
                    .map(|(value, name)| format!("{name}: {value}")),
            );
        } else {
            values.extend(staged);
        }
    }
    signature.append_defaults(&bound, &mut values, scopes);
    writeln(output, setup_indent, &format!("${started} = true;"));
    if presence.is_some() {
        writeln(output, indent, "}");
    }
    Some(Prepared {
        expression: expr,
        span,
        invocation,
        arguments: values,
        field_arguments,
    })
}

pub(super) fn prepare_setter(
    assignment: &Assignment,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> Option<Prepared> {
    let target = expression_region::ungrouped(&assignment.target);
    let Expr::PropertyAccess { object, span, .. } = target else {
        return None;
    };
    let accessor = scopes.specialization.accessor(
        *span,
        crate::ast::PropertyHookKind::Set,
        &scopes.substitutions,
    )?;
    let symbol = accessor.symbol.to_string();
    let home = accessor.callable.caller_receiver_home();
    let signature = Signature {
        defaults: vec![None; accessor.callable.parameters.len()],
        callable: CallableSignature::Named(accessor.callable.clone()),
        fields: accessor.field_parameters.to_vec(),
    };
    let field_arguments = signature.fields.iter().any(Option::is_some);
    let started = scopes.fresh_temp("__doria_call_started");
    scopes.track_expression_temporary(PhpExpressionTemporary::Alias(started.clone()));
    writeln(output, indent, &format!("${started} = false;"));
    let receiver = stage_expression(
        object,
        false,
        &started,
        output,
        indent,
        scopes,
        |output, indent, scopes| {
            if home {
                emit_receiver_borrow_home(object, false, scopes)
            } else {
                if let Some(value) =
                    value_result::statement_value(object, false, output, indent, scopes)
                {
                    return property_hooks::receiver_value(object, value, false, scopes);
                }
                property_hooks::receiver(object, false, scopes)
            }
        },
    );
    let physical_home = home.then(|| format!("${receiver}"));
    let receiver = if home {
        format!("${receiver}->value")
    } else {
        format!("${receiver}")
    };
    let arguments =
        signature.stage_argument(&assignment.value, 0, &started, output, indent, scopes);
    writeln(output, indent, &format!("${started} = true;"));
    Some(Prepared {
        expression: target,
        span: *span,
        invocation: Invocation::Setter {
            receiver,
            home: physical_home,
            symbol,
        },
        arguments,
        field_arguments,
    })
}

fn stage_expression(
    expr: &Expr,
    pending: bool,
    started: &str,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
    render: impl FnOnce(&mut String, usize, &mut PhpNameScopes) -> String,
) -> String {
    let name = scopes.fresh_temp("__doria_call_operand");
    track_operand(&name, pending, started, scopes);
    expression_region::emit(expr, output, indent, scopes, |output, indent, scopes| {
        let value = render(output, indent, scopes);
        expression_region::consume(output, indent, scopes);
        writeln(output, indent, &format!("${name} = {value};"));
    });
    name
}

fn stage_value(
    value: String,
    pending: bool,
    started: &str,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> String {
    let name = scopes.fresh_temp("__doria_call_operand");
    track_operand(&name, pending, started, scopes);
    writeln(output, indent, &format!("${name} = {value};"));
    name
}

fn track_operand(name: &str, pending: bool, started: &str, scopes: &PhpNameScopes) {
    scopes.track_expression_temporary(if pending {
        PhpExpressionTemporary::Pending {
            name: name.to_string(),
            started: started.to_string(),
        }
    } else {
        PhpExpressionTemporary::Alias(name.to_string())
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn named_scalar_arguments_keep_authored_order_through_statement_staging() {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let source = r#"
function mark(int $value): int { echo $value; return $value; }
function add(int $left, int $right): int { return $left + $right; }
function main(): void { echo add(right: mark(2), left: mark(1)); }
"#;
        let php = crate::compile_source_to_php("scalar-call-operands.doria", source).unwrap();
        assert!(php.contains("__doria_call_started"));
        let mut child = match Command::new("php")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("PHP unavailable; scalar argument execution skipped");
                return;
            }
            Err(error) => panic!("PHP could not start: {error}"),
        };
        child
            .stdin
            .take()
            .unwrap()
            .write_all(format!("{php}\n__DoriaFunction_6d61696e();").as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(output.stdout, b"213");
    }

    #[test]
    fn branch_call_temporaries_keep_pending_and_borrowed_ownership_distinct() {
        let source = r#"
enum Packet { case Number(int $value); }
function make(): Packet { return Packet::Number(1); }
function number(Packet $value): int { return match ($value) { Packet::Number($number) => $number, }; }
class Consumer { consuming function read(Packet $value): int { return number($value); } }
class Reader { function read(Packet $value): int { return number($value); } }
function choose(bool $consume): int {
    return match ($consume) {
        true => (new Consumer())->read(make()),
        false => (new Reader())->read(make()),
    };
}
function main(): void { choose(true); choose(false); }
"#;
        let php = crate::compile_source_to_php("branch-call-temporaries.doria", source).unwrap();
        assert!(php.contains("__doria_call_started"));
    }
}
