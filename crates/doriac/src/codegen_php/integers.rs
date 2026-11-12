//! Integer operators and stable-place updates share one width-aware operation.
use super::*;

pub(super) const RUNTIME: &str = include_str!("integers_runtime.php.inc");

pub(super) fn ty(resolved: Option<&ResolvedType>) -> Option<IntegerType> {
    match resolved {
        Some(ResolvedType::Integer(integer)) => Some(*integer),
        _ => None,
    }
}

pub(super) fn operation(
    op: &str,
    left: &str,
    right: &str,
    integer: IntegerType,
    span: Span,
    scopes: &PhpNameScopes,
) -> String {
    operation_at(op, left, right, integer, span, None, scopes)
}

/// `right_span` is the right operand's own span, for the operators whose
/// panic names that operand: a zero divisor or a shift count past the width.
pub(super) fn operation_at(
    op: &str,
    left: &str,
    right: &str,
    integer: IntegerType,
    span: Span,
    right_span: Option<Span>,
    scopes: &PhpNameScopes,
) -> String {
    let right_operand = right_span
        .filter(|_| matches!(op, "/" | "%" | "<<" | ">>"))
        .map(|right| {
            format!(
                ", {}, {}",
                scopes.source_location(right, right.start),
                scopes.source_location(right, right.end)
            )
        })
        .unwrap_or_default();
    format!(
        "__doria_integer_operation({op:?}, {left}, {right}, {}, {}, {}, {}, {}{right_operand})",
        integer.bit_width(),
        integer.is_signed(),
        scopes.source_location(span, span.start),
        scopes.source_location(span, span.end),
        scopes.callable_identity(),
    )
}

pub(super) fn binary(
    left: &Expr,
    op: &str,
    right: &Expr,
    integer: IntegerType,
    span: Span,
    scopes: &PhpNameScopes,
) -> String {
    // Positional value arguments are eagerly evaluated left to right in PHP.
    // Operand owners remain in the enclosing statement's temporary registry.
    operation_at(
        op,
        &emit_expr(left, scopes),
        &emit_expr(right, scopes),
        integer,
        span,
        Some(right.span()),
        scopes,
    )
}

pub(super) fn assignment_op(op: &AssignOp) -> Option<&'static str> {
    match op {
        AssignOp::AddAssign => Some("+"),
        AssignOp::SubAssign => Some("-"),
        AssignOp::MulAssign => Some("*"),
        AssignOp::DivAssign => Some("/"),
        AssignOp::ModAssign => Some("%"),
        AssignOp::ShiftLeftAssign => Some("<<"),
        AssignOp::ShiftRightAssign => Some(">>"),
        AssignOp::BitwiseAndAssign => Some("&"),
        AssignOp::BitwiseOrAssign => Some("|"),
        AssignOp::BitwiseXorAssign => Some("^"),
        AssignOp::Assign => None,
    }
}

pub(super) fn assignment(assignment: &Assignment, scopes: &PhpNameScopes) -> Option<String> {
    let op = assignment_op(&assignment.op)?;
    let integer = ty(scopes.expression_types.get(&assignment.value.span()))?;
    Some(update(
        &assignment.target,
        Some(&assignment.value),
        op,
        integer,
        assignment.span,
        scopes,
    ))
}

pub(super) fn update(
    target: &Expr,
    right: Option<&Expr>,
    op: &str,
    integer: IntegerType,
    span: Span,
    scopes: &PhpNameScopes,
) -> String {
    numeric_place::update(target, span, scopes, |previous, scopes| {
        let right_span = right.map(Expr::span);
        let right = right.map_or_else(|| "1".to_string(), |right| emit_expr(right, scopes));
        operation_at(op, previous, &right, integer, span, right_span, scopes)
    })
}

/// `Target::from(value)`. A widening conversion cannot fail and PHP integers
/// hold every Doria integer up to 64 bits, so it is the value itself; any
/// other is checked against the target's range.
pub(super) fn convert(
    value: &Expr,
    source: IntegerType,
    target: IntegerType,
    span: Span,
    scopes: &PhpNameScopes,
) -> String {
    let emitted = emit_expr(value, scopes);
    if target.min_value() <= source.min_value() && source.max_value() <= target.max_value() {
        return format!("({emitted})");
    }
    let operand = value.span();
    format!(
        "__doria_integer_convert({emitted}, {}, {}, {}, {}, {}, {}, {}, {}, {})",
        source.bit_width(),
        source.is_signed(),
        target.bit_width(),
        target.is_signed(),
        scopes.source_location(span, span.start),
        scopes.source_location(span, span.end),
        scopes.source_location(operand, operand.start),
        scopes.source_location(operand, operand.end),
        scopes.callable_identity(),
    )
}
