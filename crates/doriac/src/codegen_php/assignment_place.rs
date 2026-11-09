//! Statement-owned locations preserve receiver identity and lexical field access.

use super::*;
use crate::ast::PropertyHookKind;

pub(super) fn stored_property(
    target: &Expr,
    output: &mut String,
    indent: usize,
    scopes: &mut PhpNameScopes,
) -> Option<String> {
    let Expr::PropertyAccess {
        object,
        property,
        null_safe: false,
        span,
        ..
    } = expression_region::ungrouped(target)
    else {
        return None;
    };
    if [PropertyHookKind::Get, PropertyHookKind::Set]
        .into_iter()
        .any(|kind| {
            scopes
                .specialization
                .accessor(*span, kind, &scopes.substitutions)
                .is_some()
        })
    {
        return None;
    }
    let receiver = scopes.fresh_temp("__doria_assignment_receiver");
    expression_region::emit(object, output, indent, scopes, |output, indent, scopes| {
        let value = property_hooks::receiver(object, false, scopes);
        expression_region::consume(output, indent, scopes);
        writeln(output, indent, &format!("${receiver} = {value};"));
        scopes.track_expression_temporary(PhpExpressionTemporary::Alias(receiver.clone()));
    });
    Some(format!("${receiver}->{property}"))
}
