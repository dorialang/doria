//! Numeric updates retain one checked storage home through evaluation and commit.

use super::*;
use crate::ast::PropertyHookKind;

pub(super) const RUNTIME: &str = r#"
function __doria_numeric_assign(int|float &$place, int|float $value): void
{
    $place = $value;
}

function __doria_numeric_assign_cell(__DoriaCell $place, int|float $value): void
{
    $place->value = $value;
}

function __doria_float_operation(string $op, float $left, float $right): float
{
    return match ($op) {
        "+" => $left + $right,
        "-" => $left - $right,
        "*" => $left * $right,
        "/" => fdiv($left, $right),
        default => throw new LogicException("compiler float operation is unknown"),
    };
}

// A Doria `float32` is carried in a PHP float that always holds a binary32
// value. Rounding a binary64 number to binary32 is one IEEE conversion.
function __doria_float32(float $value): float
{
    return unpack('G', pack('G', $value))[1];
}

// One binary64 operation on two binary32 values, rounded once to binary32, is
// the binary32 operation: binary64 carries 53 bits, at least 2 * 24 + 2.
function __doria_float32_operation(string $op, float $left, float $right): float
{
    return __doria_float32(__doria_float_operation($op, $left, $right));
}

// `Float::toInt(value)` truncates toward zero. NaN, an infinity and a value
// outside the canonical `int` have no result; the panic points at the value,
// inside the conversion.
function __doria_float_to_int(float $value, int $start, int $end,
    int $valueStart, int $valueEnd, string $callable): int
{
    if (!($value >= -9223372036854775808.0 && $value < 9223372036854775808.0)) {
        __doria_panic("P1110", $valueStart, $valueEnd, null, $callable,
            [], __doria_diagnostic_site($start, $end));
    }
    return (int) $value;
}
"#;

/// The width of the float expression at `span`, when it is one.
pub(super) fn float_type(span: Span, scopes: &PhpNameScopes) -> Option<FloatType> {
    match scopes.expression_types.get(&span) {
        Some(ResolvedType::Float(ty)) => Some(*ty),
        _ => None,
    }
}

pub(super) fn float_assignment_op(op: &AssignOp) -> Option<&'static str> {
    match op {
        AssignOp::AddAssign => Some("+"),
        AssignOp::SubAssign => Some("-"),
        AssignOp::MulAssign => Some("*"),
        AssignOp::DivAssign => Some("/"),
        _ => None,
    }
}

#[derive(Clone, Copy)]
enum Numeric {
    Integer(IntegerType),
    Float(FloatType),
}

pub(super) fn update(
    target: &Expr,
    span: Span,
    scopes: &PhpNameScopes,
    compute: impl FnOnce(&str, &PhpNameScopes) -> String,
) -> String {
    let mut target = target;
    while let Expr::Grouped { expr, .. } = target {
        target = expr;
    }
    let direct = direct_place(target, scopes);
    let (assign, place, previous) = if let Some(place) = direct {
        ("__doria_numeric_assign", place.clone(), place)
    } else {
        let home = scopes.expression_temp("__doria_numeric_home_", span);
        let cell = borrowed_argument_cell(target, scopes)
            .expect("checked numeric update has a stable storage home");
        (
            "__doria_numeric_assign_cell",
            format!("(${home} = {cell})"),
            format!("${home}->value"),
        )
    };
    // Register target owners before RHS owners; cleanup reverses that order.
    let result = compute(&previous, scopes);
    format!("{assign}({place}, {result})")
}

fn direct_place(target: &Expr, scopes: &PhpNameScopes) -> Option<String> {
    match target {
        Expr::Variable { span, .. } | Expr::This { span } => {
            scopes.place_for_use(*span).and_then(|place| match place {
                PhpBindingPlace::Direct(name) => Some(format!("${name}")),
                _ => None,
            })
        }
        Expr::StaticMember {
            class_name,
            member,
            span,
        } if scopes.is_static_property(class_name, member)
            && scopes
                .specialization
                .accessor(*span, PropertyHookKind::Get, &scopes.substitutions)
                .is_none() =>
        {
            Some(format!(
                "{}::${member}",
                scopes.static_member_class_symbol(class_name, *span)
            ))
        }
        _ => None,
    }
}

fn prepare_direct_place(
    target: &Expr,
    output: &mut String,
    scopes: &mut PhpNameScopes,
) -> Option<String> {
    if let Some(place) = direct_place(target, scopes) {
        return Some(place);
    }
    assignment_place::stored_property(target, output, 0, scopes)
}

pub(super) fn emit_assignment_statement(
    assignment: &Assignment,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> bool {
    let (op, numeric) = if let Some(integer) =
        integers::ty(scopes.expression_types.get(&assignment.value.span()))
    {
        let Some(op) = integers::assignment_op(&assignment.op) else {
            return false;
        };
        (op, Numeric::Integer(integer))
    } else if let Some(float) = float_type(assignment.value.span(), scopes) {
        let Some(op) = float_assignment_op(&assignment.op) else {
            return false;
        };
        (op, Numeric::Float(float))
    } else {
        return false;
    };
    emit_update_statement(
        Update {
            target: &assignment.target,
            right: Some(&assignment.value),
            op,
            numeric,
            span: assignment.span,
        },
        output,
        indent,
        scopes,
    );
    true
}

pub(super) fn emit_increment_statement(
    increment: &IncrementStmt,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> bool {
    let numeric = if let Some(integer) =
        integers::ty(scopes.expression_types.get(&increment.target.span()))
    {
        Numeric::Integer(integer)
    } else if let Some(float) = float_type(increment.target.span(), scopes) {
        Numeric::Float(float)
    } else {
        return false;
    };
    let op = match increment.op {
        IncrementOp::Increment => "+",
        IncrementOp::Decrement => "-",
    };
    emit_update_statement(
        Update {
            target: &increment.target,
            right: None,
            op,
            numeric,
            span: increment.span,
        },
        output,
        indent,
        scopes,
    );
    true
}

struct Update<'a> {
    target: &'a Expr,
    right: Option<&'a Expr>,
    op: &'static str,
    numeric: Numeric,
    span: Span,
}

fn emit_update_statement(
    update: Update<'_>,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    let Update {
        target,
        right,
        op,
        numeric,
        span,
    } = update;
    let target = expression_region::ungrouped(target);
    let mut body = String::new();
    let previous = scopes.fresh_temp("__doria_numeric_previous");
    let setter = if let Expr::PropertyAccess { span, .. } = target {
        scopes
            .specialization
            .accessor(*span, PropertyHookKind::Set, &scopes.substitutions)
            .map(|accessor| (accessor.symbol.to_string(), accessor.callable.clone()))
    } else {
        None
    };
    let (commit, hooked) = if let Some((setter, setter_plan)) = setter {
        let Expr::PropertyAccess { object, span, .. } = target else {
            unreachable!();
        };
        let accessor = scopes
            .specialization
            .accessor(*span, PropertyHookKind::Get, &scopes.substitutions)
            .expect("checked numeric property update has a getter");
        let getter = accessor.symbol.to_string();
        let getter_home = accessor.callable.caller_receiver_home();
        let setter_home = setter_plan.caller_receiver_home();
        let receiver = scopes.fresh_temp("__doria_numeric_receiver");
        let home = if getter_home || setter_home {
            let home = scopes.fresh_temp("__doria_numeric_home");
            let source = emit_receiver_borrow_home(object, false, scopes);
            scopes.track_expression_temporary(PhpExpressionTemporary::Alias(home.clone()));
            writeln(&mut body, 0, &format!("${home} = {source};"));
            writeln(&mut body, 0, &format!("${receiver} = ${home}->value;"));
            Some(home)
        } else {
            let source = property_hooks::receiver(object, false, scopes);
            writeln(&mut body, 0, &format!("${receiver} = {source};"));
            None
        };
        scopes.track_expression_temporary(PhpExpressionTemporary::Alias(receiver.clone()));
        let arguments = if getter_home {
            format!("${}", home.as_ref().unwrap())
        } else {
            String::new()
        };
        writeln(
            &mut body,
            0,
            &format!("${previous} = ${receiver}->{getter}({arguments});"),
        );
        let arguments = if setter_home {
            format!("${}, ", home.as_ref().unwrap())
        } else {
            String::new()
        };
        (format!("${receiver}->{setter}({arguments}"), true)
    } else if let Some(place) = prepare_direct_place(target, &mut body, scopes) {
        writeln(&mut body, 0, &format!("${previous} = {place};"));
        (format!("__doria_numeric_assign({place}, "), false)
    } else {
        let home = scopes.fresh_temp("__doria_numeric_home");
        let cell = borrowed_argument_cell(target, scopes)
            .expect("checked numeric update has a stable storage home");
        scopes.track_expression_temporary(PhpExpressionTemporary::Alias(home.clone()));
        writeln(&mut body, 0, &format!("${home} = {cell};"));
        writeln(&mut body, 0, &format!("${previous} = ${home}->value;"));
        (format!("__doria_numeric_assign_cell(${home}, "), false)
    };
    // Capture both the home and its previous value before the RHS can mutate
    // that storage. The enclosing statement retains target and operand owners.
    let right_span = right.map(Expr::span);
    // `++` and `--` step by one of the target's own type.
    let right = right.map_or_else(
        || match numeric {
            Numeric::Integer(_) => "1".to_string(),
            Numeric::Float(_) => "1.0".to_string(),
        },
        |right| value_result::stage_value(right, false, &mut body, 0, scopes),
    );
    let result = match numeric {
        Numeric::Integer(integer) => integers::operation_at(
            op,
            &format!("${previous}"),
            &right,
            integer,
            span,
            right_span,
            scopes,
        ),
        Numeric::Float(float) => float_operation(op, float, &format!("${previous}"), &right),
    };
    writeln(&mut body, 0, &format!("{commit}{result});"));
    let token = (hooked && !scopes.compiler_owned_callable)
        .then(|| scopes.fresh_temp("__doria_numeric_operation"));
    if let Some(token) = &token {
        writeln(
            output,
            indent,
            &format!(
                "${token} = __doria_operation_enter({});",
                scopes.checked_origin(target.span()),
            ),
        );
        writeln(output, indent, "try {");
    }
    for line in body.lines() {
        writeln(output, indent + usize::from(token.is_some()), line);
    }
    if let Some(token) = token {
        writeln(output, indent, "} finally {");
        writeln(
            output,
            indent + 1,
            &format!("__doria_operation_leave(${token});"),
        );
        writeln(output, indent, "}");
    }
}

/// One float operation at its own width. Every float `+ - * /` in PHP output
/// is emitted here, so a `float32` result is always rounded to binary32.
pub(super) fn float_operation(op: &str, ty: FloatType, left: &str, right: &str) -> String {
    let function = match ty {
        FloatType::Float32 => "__doria_float32_operation",
        FloatType::Float64 => "__doria_float_operation",
    };
    format!("{function}({op:?}, {left}, {right})")
}

pub(super) fn float_binary(left: &Expr, op: &str, right: &Expr, scopes: &PhpNameScopes) -> String {
    let ty = float_type(left.span(), scopes).expect("float operation has a float left operand");
    float_operation(op, ty, &emit_expr(left, scopes), &emit_expr(right, scopes))
}

pub(super) fn float_assignment(assignment: &Assignment, scopes: &PhpNameScopes) -> Option<String> {
    let ty = float_type(assignment.value.span(), scopes)?;
    let op = float_assignment_op(&assignment.op)?;
    Some(update(
        &assignment.target,
        assignment.span,
        scopes,
        |previous, scopes| float_operation(op, ty, previous, &emit_expr(&assignment.value, scopes)),
    ))
}

/// `Float::toInt(value)`, checked against the canonical `int` range.
pub(super) fn float_to_int(value: &Expr, span: Span, scopes: &PhpNameScopes) -> String {
    let operand = value.span();
    format!(
        "__doria_float_to_int({}, {}, {}, {}, {}, {})",
        emit_expr(value, scopes),
        scopes.source_location(span, span.start),
        scopes.source_location(span, span.end),
        scopes.source_location(operand, operand.start),
        scopes.source_location(operand, operand.end),
        scopes.callable_identity(),
    )
}
