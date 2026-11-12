//! Statement regions keep operation scopes around evaluation, before conversion.

use super::*;

#[derive(Debug)]
pub(super) struct Root {
    expression: *const Expr,
    name: String,
    captured: Vec<Captured>,
    emitted: bool,
}

#[derive(Debug)]
struct Captured {
    value: String,
    span: Span,
    checked: bool,
    diagnostic: bool,
}

impl Root {
    pub(super) fn captured_name(&self) -> Option<&str> {
        (!self.captured.is_empty() || self.emitted).then_some(self.name.as_str())
    }
}

pub(super) fn ungrouped(mut expr: &Expr) -> &Expr {
    while let Expr::Grouped { expr: inner, .. } = expr {
        expr = inner;
    }
    expr
}

pub(super) fn is_root(expr: &Expr, scopes: &PhpNameScopes) -> bool {
    scopes
        .expression_region
        .as_ref()
        .is_some_and(|root| std::ptr::eq(root.borrow().expression, ungrouped(expr)))
}

pub(super) fn guarded(
    expr: &Expr,
    value: String,
    diagnostic: bool,
    scopes: &PhpNameScopes,
) -> String {
    let checked = scopes.has_checked_effects(expr.span());
    let diagnostic = diagnostic && !scopes.compiler_owned_callable;
    if checked || diagnostic {
        if let Some(root) = &scopes.expression_region {
            let mut root = root.borrow_mut();
            if std::ptr::eq(root.expression, ungrouped(expr)) {
                assert!(
                    !root.emitted
                        && (!std::ptr::eq(root.expression, expr) || root.captured.is_empty()),
                    "statement root must be emitted once"
                );
                root.captured.push(Captured {
                    value,
                    span: expr.span(),
                    checked,
                    diagnostic,
                });
                return format!("${}", root.name);
            }
        }
    }
    let value = if checked {
        let caught = scopes.expression_temp("__doria_call_error_", expr.span());
        emit_expression_body(
            scopes,
            &format!(
                "try {{ return {value}; }} catch (__DoriaCheckedError ${caught}) {{ ${caught}->ensureOrigin({}, {}); __doria_rethrow_checked_error(${caught}); }}",
                scopes.checked_origin(expr.span()),
                scopes.callable_identity(),
            ),
            &[],
        )
    } else {
        value
    };
    if diagnostic {
        diagnostic_sites::operation(value, expr.span(), scopes)
    } else {
        value
    }
}

pub(super) fn emit(
    expr: &Expr,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
    render: impl FnOnce(&mut String, usize, &mut PhpNameScopes),
) {
    region(expr, output, indent, scopes, |output, indent, scopes| {
        let previous_call = scopes.prepared_call.take();
        scopes.prepared_call =
            value_argument::prepare(ungrouped(expr), output, indent, scopes).map(Rc::new);
        let previous_operands = scopes.scalar_operands.clone();
        prepare_scalar_operands(ungrouped(expr), output, indent, scopes);
        let previous_receiver = scopes.prepared_receiver.take();
        scopes.prepared_receiver =
            value_argument::Receiver::prepare(ungrouped(expr), output, indent, scopes).map(Rc::new);
        render(output, indent, scopes);
        scopes.prepared_receiver = previous_receiver;
        scopes.scalar_operands = previous_operands;
        scopes.prepared_call = previous_call;
    });
}

pub(super) fn region(
    expr: &Expr,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
    render: impl FnOnce(&mut String, usize, &mut PhpNameScopes),
) {
    let name = scopes.fresh_temp("__doria_operation_value");
    // Grouping preserves its producer; shared provenance alone does not.
    let root = Rc::new(RefCell::new(Root {
        expression: ungrouped(expr),
        name,
        captured: Vec::new(),
        emitted: false,
    }));
    let previous_root = scopes.expression_region.replace(Rc::clone(&root));
    let previous_temporaries = scopes.expression_temporaries.clone();
    let temporaries = previous_temporaries.clone().unwrap_or_default();
    scopes.expression_temporaries = Some(Rc::clone(&temporaries));
    let mut body = String::new();
    render(&mut body, 0, scopes);
    scopes.expression_region = previous_root;
    scopes.expression_temporaries = previous_temporaries.clone();

    let root = root.borrow();
    assert!(
        root.captured.is_empty(),
        "statement evaluation must precede its consumer"
    );
    if root.emitted {
        let mut evaluated = String::new();
        writeln(&mut evaluated, 0, "try {");
        append(&body, &mut evaluated, 1);
        writeln(&mut evaluated, 0, "} finally {");
        writeln(&mut evaluated, 1, &format!("unset(${});", root.name));
        writeln(&mut evaluated, 0, "}");
        body = evaluated;
    }
    let temporaries = temporaries.borrow();
    if previous_temporaries.is_none() && !temporaries.is_empty() {
        writeln(output, indent, "try {");
        append(&body, output, indent + 1);
        writeln(output, indent, "} finally {");
        for temporary in temporaries.iter().rev() {
            writeln(output, indent + 1, &temporary.cleanup());
        }
        writeln(output, indent, "}");
    } else {
        append(&body, output, indent);
    }
}

fn snapshot_type(ty: &ResolvedType) -> bool {
    match ty {
        ResolvedType::Bool
        | ResolvedType::Integer(_)
        | ResolvedType::Float(_)
        | ResolvedType::Null => true,
        ResolvedType::Nullable(inner) => snapshot_type(inner),
        ResolvedType::Class(class) => {
            crate::inline_value::InlineValueKind::for_nominal(&class.name, class.arguments.len())
                .is_some_and(|kind| !kind.requires_owned_cleanup())
        }
        _ => false,
    }
}

fn snapshot_operand(expr: &Expr, scopes: &PhpNameScopes) -> bool {
    scopes.expression_types.get(&expr.span()).is_some_and(|ty| {
        snapshot_type(&crate::types::substitute_resolved_type(
            ty,
            &scopes.substitutions,
        ))
    })
}

// Operands with no evaluation effects. A plain string literal is an immutable
// Copy value; interpolation evaluates expressions and stays staged.
pub(super) fn simple_scalar(expr: &Expr) -> bool {
    matches!(
        ungrouped(expr),
        Expr::Variable { .. }
            | Expr::Int { .. }
            | Expr::Float { .. }
            | Expr::Bool { .. }
            | Expr::Null { .. }
            | Expr::String { .. }
    )
}

fn prepare_scalar_operands(
    expr: &Expr,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    if let Some(value) = bytes::prepare_scalar(expr, output, indent, scopes) {
        scopes
            .scalar_operands
            .insert(ungrouped(expr) as *const Expr, value);
        return;
    }
    if let Some(value) = value_result::prepare_comparison(expr, output, indent, scopes) {
        let value = guarded(expr, value, true, scopes);
        scopes
            .scalar_operands
            .insert(ungrouped(expr) as *const Expr, value);
        return;
    }
    // Snapshots share statement ownership; lazy right-side setup stays gated.
    match expr {
        Expr::Binary {
            left, op, right, ..
        } if snapshot_operand(left, scopes)
            && snapshot_operand(right, scopes)
            && !(simple_scalar(left) && simple_scalar(right)) =>
        {
            let left_value = value(left, output, indent, scopes);
            scopes
                .scalar_operands
                .insert(ungrouped(left) as *const Expr, left_value.clone());
            let gate = match op {
                BinaryOp::And => Some(left_value.clone()),
                BinaryOp::Or => Some(format!("!({left_value})")),
                BinaryOp::Coalesce => Some(format!("{left_value} === null")),
                _ => None,
            };
            if let Some(gate) = &gate {
                writeln(output, indent, &format!("if ({gate}) {{"));
            }
            let right_value = value(right, output, indent + usize::from(gate.is_some()), scopes);
            if gate.is_some() {
                writeln(output, indent, "}");
            }
            scopes
                .scalar_operands
                .insert(ungrouped(right) as *const Expr, right_value);
        }
        Expr::Unary { expr, .. } if snapshot_operand(expr, scopes) && !simple_scalar(expr) => {
            let operand = value(expr, output, indent, scopes);
            scopes
                .scalar_operands
                .insert(ungrouped(expr) as *const Expr, operand);
        }
        _ => {}
    }
}

// Consumers choose this boundary after setup and before using the raw value.
pub(super) fn consume(output: &mut String, indent: usize, scopes: &PhpNameScopes) {
    let Some(root) = &scopes.expression_region else {
        return;
    };
    let mut root = root.borrow_mut();
    if root.captured.is_empty() {
        return;
    }
    root.emitted = true;
    for captured in std::mem::take(&mut root.captured) {
        let assignment = format!("${} = {};", root.name, captured.value);
        let assignment = if captured.checked {
            let span = captured.span;
            let caught = scopes.expression_temp("__doria_call_error_", span);
            format!(
                "try {{ {assignment} }} catch (__DoriaCheckedError ${caught}) {{ ${caught}->ensureOrigin({}, {}); __doria_rethrow_checked_error(${caught}); }}",
                scopes.checked_origin(span),
                scopes.callable_identity(),
            )
        } else {
            assignment
        };
        if captured.diagnostic {
            let token = scopes.expression_temp("__doria_operation_", captured.span);
            writeln(
                output,
                indent,
                &format!(
                    "${token} = __doria_operation_enter({}); try {{ {assignment} }} finally {{ __doria_operation_leave(${token}); }}",
                    scopes.checked_origin(captured.span),
                ),
            );
        } else {
            writeln(output, indent, &assignment);
        }
    }
}

pub(super) fn operation(expr: &Expr, value: String, scopes: &PhpNameScopes) -> String {
    if !scopes.compiler_owned_callable {
        if let Some(root) = &scopes.expression_region {
            let mut root = root.borrow_mut();
            if std::ptr::eq(root.expression, ungrouped(expr)) && !root.emitted {
                // Conversion consumes the staged value after its producer's guard.
                root.captured.push(Captured {
                    value,
                    span: expr.span(),
                    checked: false,
                    diagnostic: true,
                });
                return format!("${}", root.name);
            }
        }
    }
    diagnostic_sites::operation(value, expr.span(), scopes)
}

fn append(body: &str, output: &mut String, indent: usize) {
    for line in body.lines() {
        writeln(output, indent, line);
    }
}

pub(super) struct Value {
    body: String,
    pub(super) value: String,
}

impl Value {
    pub(super) fn emit(&self, output: &mut String, indent: usize) {
        append(&self.body, output, indent);
    }
}

pub(super) fn prepare(expr: &Expr, scopes: &mut PhpNameScopes) -> Value {
    let mut body = String::new();
    if let Some(value) = value_result::statement_value(expr, false, &mut body, 0, scopes) {
        return Value { body, value };
    }
    prepare_with(expr, scopes, emit_expr)
}

fn prepare_with(
    expr: &Expr,
    scopes: &mut PhpNameScopes,
    render: fn(&Expr, &PhpNameScopes) -> String,
) -> Value {
    let result = scopes.fresh_temp("__doria_expression_result");
    let mut body = String::new();
    emit(expr, &mut body, 0, scopes, |output, indent, scopes| {
        let value = render(expr, scopes);
        consume(output, indent, scopes);
        writeln(output, indent, &format!("${result} = {value};"));
    });
    Value {
        body,
        value: format!("${result}"),
    }
}

pub(super) fn value(
    expr: &Expr,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> String {
    let value = prepare(expr, scopes);
    value.emit(output, indent);
    value.value
}

pub(super) fn expression(expr: &Expr, scopes: &PhpNameScopes) -> String {
    let mut inner = scopes.clone();
    // The enclosing reader owns borrow projection and value conversions.
    let value = prepare_with(expr, &mut inner, emit_expr_unboxed_impl);
    let mut body = String::new();
    value.emit(&mut body, 0);
    writeln(&mut body, 0, &format!("return {};", value.value));
    emit_expression_body(&inner, &body, &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_include_only_trivial_inline_values_and_preserve_nominal_arity() {
        use crate::types::ClassType;
        for (name, arguments, expected) in [
            (crate::compiler_known_time::DURATION, 0, true),
            (crate::compiler_known_time::DEADLINE, 0, true),
            (crate::compiler_known_stream::REGISTRATION_KEY, 1, true),
            (crate::compiler_known_stream::READ_TRANSFER_KEY, 0, true),
            (crate::compiler_known_stream::WRITE_TRANSFER_KEY, 0, true),
            (crate::compiler_known_stream::WATCH_KEY, 0, true),
            (crate::compiler_known_stream::WATCH_SOURCE, 0, true),
            (crate::compiler_known_stream::STANDARD_INPUT, 0, true),
            (crate::compiler_known_stream::STANDARD_OUTPUT, 0, true),
            (crate::compiler_known_stream::STANDARD_ERROR, 0, true),
            (
                crate::compiler_known_concurrency::CANCELLATION_TOKEN,
                0,
                false,
            ),
            ("Duration", 0, false),
        ] {
            let ty = ResolvedType::Class(ClassType::new(name, vec![ResolvedType::Bool; arguments]));
            assert_eq!(snapshot_type(&ty), expected, "{name}");
            assert_eq!(
                snapshot_type(&ResolvedType::Nullable(Box::new(ty))),
                expected,
                "?{name}"
            );
            let wrong = ResolvedType::Class(ClassType::new(
                name,
                vec![ResolvedType::Bool; arguments + 1],
            ));
            assert!(!snapshot_type(&wrong), "wrong nominal arity: {name}");
        }
    }
}
