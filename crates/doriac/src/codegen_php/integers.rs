//! Checked arithmetic and stable-place updates share one width-aware operation.
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
    format!(
        "__doria_integer_operation({op:?}, {left}, {right}, {}, {}, {}, {}, {})",
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
    operation(
        op,
        &emit_expr(left, scopes),
        &emit_expr(right, scopes),
        integer,
        span,
        scopes,
    )
}

pub(super) fn assignment_op(op: &AssignOp) -> Option<&'static str> {
    match op {
        AssignOp::AddAssign => Some("+"),
        AssignOp::SubAssign => Some("-"),
        AssignOp::MulAssign => Some("*"),
        _ => None,
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
        let right = right.map_or_else(|| "1".to_string(), |right| emit_expr(right, scopes));
        operation(op, previous, &right, integer, span, scopes)
    })
}
