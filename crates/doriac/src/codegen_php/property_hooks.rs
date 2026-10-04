//! Emit checked accessors as ordinary calls, with the normal argument and
//! full-expression ownership machinery. Backing accesses have no call fact.

use super::*;
use crate::ast::PropertyHookKind;

/// An override contributes a new initialization phase, not another physical
/// field. This identity is checked by semantics and preserved in HIR.
fn inherited_backing_field(
    property: &PropertyDecl,
) -> Option<&crate::property_hooks::PropertyBackingField> {
    property
        .hooks
        .as_ref()
        .and_then(|hooks| hooks.backing_field.as_ref())
        .filter(|field| field.declaration != property.span)
}

pub(super) fn has_runtime_initializer(
    property: &PropertyDecl,
    semantic_info: &SemanticInfo,
) -> bool {
    !property.is_static
        && property.initializer.as_ref().is_some_and(|value| {
            inherited_backing_field(property).is_some()
                || is_payload_enum_expression(value, semantic_info)
                || requires_php_runtime_property_initializer(value, semantic_info).is_some()
        })
}

pub(super) fn emit_initializer(
    property: &PropertyDecl,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) {
    let initializer = property
        .initializer
        .as_ref()
        .expect("scheduled property initializer");
    let initializer_scopes = scopes.specialization.property_scope(scopes, &property.name);
    let value = emit_owned_expr(initializer, &initializer_scopes);
    if let Some(field) = inherited_backing_field(property) {
        let replacement = scopes.fresh_temp("__doria_property_replacement");
        let previous = scopes.fresh_temp("__doria_property_previous");
        // Evaluate completely before touching the parent's initialized value.
        // If acquisition throws, the completed parent phase still owns it.
        writeln(output, indent, &format!("${replacement} = {value};"));
        writeln(
            output,
            indent,
            &format!("${previous} = $this->{} ?? null;", field.property_name),
        );
        writeln(
            output,
            indent,
            &format!("$this->{} = ${replacement};", field.property_name),
        );
        writeln(output, indent, &format!("unset(${replacement});"));
        writeln(output, indent, &format!("__doria_drop_value(${previous});"));
    } else {
        writeln(
            output,
            indent,
            &format!("$this->{} = {value};", property.name),
        );
    }
}

fn receiver(object: &Expr, null_safe: bool, scopes: &PhpNameScopes) -> String {
    let emitted = emit_member_receiver(object, scopes);
    if scopes
        .expression_types
        .get(&object.span())
        .and_then(shared::kind)
        .is_some()
    {
        format!("{emitted}{}payload()", if null_safe { "?->" } else { "->" })
    } else {
        emitted
    }
}

pub(super) fn getter(
    object: &Expr,
    null_safe: bool,
    span: Span,
    scopes: &PhpNameScopes,
) -> Option<String> {
    let (symbol, _) =
        scopes
            .specialization
            .accessor(span, PropertyHookKind::Get, &scopes.substitutions)?;
    Some(format!(
        "{}{}{symbol}()",
        receiver(object, null_safe, scopes),
        if null_safe { "?->" } else { "->" }
    ))
}

pub(super) fn assignment(assignment: &Assignment, scopes: &PhpNameScopes) -> Option<String> {
    let mut target = &assignment.target;
    while let Expr::Grouped { expr, .. } = target {
        target = expr;
    }
    let Expr::PropertyAccess { object, span, .. } = target else {
        return None;
    };
    let (setter, plan) =
        scopes
            .specialization
            .accessor(*span, PropertyHookKind::Set, &scopes.substitutions)?;
    let mut nested = scopes.clone();
    let temporaries = Rc::new(RefCell::new(Vec::new()));
    nested.expression_temporaries = Some(Rc::clone(&temporaries));
    let object = receiver(object, false, &nested);
    let receiver_name = scopes.expression_temp("__doria_hook_receiver_", *span);
    let mut body = format!("${receiver_name} = {object}; ");
    let argument = if assignment.op == AssignOp::Assign {
        emit_call_argument(&assignment.value, plan.parameters.first(), &nested)
    } else {
        let (getter, _) = scopes
            .specialization
            .accessor(*span, PropertyHookKind::Get, &scopes.substitutions)
            .expect("checked property update has a getter");
        let value_name = scopes.expression_temp("__doria_hook_value_", *span);
        body.push_str(&format!("${value_name} = ${receiver_name}->{getter}(); "));
        let rhs = emit_expr(&assignment.value, &nested);
        // PHP's shared validator rejects integer compounds; the accepted float
        // subset uses the same operators as ordinary assignments.
        match assignment.op {
            AssignOp::AddAssign => format!("(${value_name} + {rhs})"),
            AssignOp::SubAssign => format!("(${value_name} - {rhs})"),
            AssignOp::MulAssign => format!("(${value_name} * {rhs})"),
            AssignOp::DivAssign => format!("fdiv(${value_name}, {rhs})"),
            _ => unreachable!("PHP compound assignment was validated"),
        }
    };
    body.push_str(&format!("${receiver_name}->{setter}({argument});"));
    let temporaries = temporaries.borrow();
    Some(emit_expression_body(scopes, &body, &temporaries))
}
