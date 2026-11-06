use std::collections::HashMap;

use crate::diagnostics::Diagnostic;
use crate::source::Span;
use crate::types::{TypeId, TypeKind, TypeRegistry};

#[allow(clippy::too_many_arguments)]
pub(crate) fn infer_type_parameter_bindings(
    types: &mut TypeRegistry,
    diagnostics: &mut Vec<Diagnostic>,
    callee: &str,
    pattern: TypeId,
    actual: TypeId,
    span: Span,
    bindings: &mut HashMap<String, TypeId>,
    from_context: bool,
) {
    match (types.kind(pattern).clone(), types.kind(actual).clone()) {
        (TypeKind::TypeParameter(name), _) if from_context && bindings.contains_key(&name) => {}
        (TypeKind::TypeParameter(name), TypeKind::Null) => {
            let message = format!(
                "cannot infer type parameter `{name}` of {callee} from `null`; `null` requires a nullable expected type"
            );
            if !diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "E0538"
                    && diagnostic.span == span
                    && diagnostic.message == message
            }) {
                diagnostics.push(Diagnostic::new("E0538", message, span).with_help(
                    "pass a value with a concrete type or use a nullable parameter shape",
                ));
            }
            let unknown = types.unknown();
            bindings.entry(name).or_insert(unknown);
        }
        (TypeKind::TypeParameter(name), TypeKind::Void) => {
            let message = format!(
                "cannot infer type parameter `{name}` of {callee} from a `void` expression"
            );
            if !diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "E0538"
                    && diagnostic.span == span
                    && diagnostic.message == message
            }) {
                diagnostics.push(
                    Diagnostic::new("E0538", message, span)
                        .with_help("pass an expression that produces a value"),
                );
            }
            let unknown = types.unknown();
            bindings.entry(name).or_insert(unknown);
        }
        (TypeKind::TypeParameter(name), TypeKind::Unknown | TypeKind::EmptyCollection) => {
            let _ = name;
        }
        (TypeKind::TypeParameter(name), _) => {
            if let Some(previous) = bindings.get(&name).copied() {
                if previous != actual {
                    diagnostics.push(
                        Diagnostic::new(
                            "E0532",
                            format!(
                                "type parameter `{name}` of {callee} was inferred as both `{}` and `{}`",
                                types.display(previous),
                                types.display(actual)
                            ),
                            span,
                        )
                        .with_help(
                            "pass arguments whose corresponding generic types are identical",
                        ),
                    );
                }
            } else {
                bindings.insert(name, actual);
            }
        }
        (TypeKind::Nullable(pattern), TypeKind::Nullable(actual)) => {
            infer_type_parameter_bindings(
                types,
                diagnostics,
                callee,
                pattern,
                actual,
                span,
                bindings,
                from_context,
            );
        }
        (TypeKind::Nullable(pattern), TypeKind::Null) => {
            let _ = pattern;
        }
        (TypeKind::Nullable(pattern), _) => {
            infer_type_parameter_bindings(
                types,
                diagnostics,
                callee,
                pattern,
                actual,
                span,
                bindings,
                from_context,
            );
        }
        (TypeKind::TypedArray(pattern), TypeKind::TypedArray(actual))
        | (TypeKind::List(pattern), TypeKind::List(actual))
        | (TypeKind::Set(pattern), TypeKind::Set(actual))
        | (TypeKind::SortedSet(pattern), TypeKind::SortedSet(actual))
        | (TypeKind::PriorityQueue(pattern), TypeKind::PriorityQueue(actual))
        | (TypeKind::Deque(pattern), TypeKind::Deque(actual)) => {
            infer_type_parameter_bindings(
                types,
                diagnostics,
                callee,
                pattern,
                actual,
                span,
                bindings,
                from_context,
            );
        }
        // A shared handle unifies with the same handle kind, through its payload:
        // `SharedReference<T>` against `SharedReference<Node>` binds `T = Node`.
        // The kinds must match, so unification never crosses the family boundary.
        (
            TypeKind::SharedHandle(pattern_kind, pattern),
            TypeKind::SharedHandle(actual_kind, actual),
        ) if pattern_kind == actual_kind => {
            infer_type_parameter_bindings(
                types,
                diagnostics,
                callee,
                pattern,
                actual,
                span,
                bindings,
                from_context,
            );
        }
        (
            TypeKind::Dictionary(pattern_key, pattern_value),
            TypeKind::Dictionary(actual_key, actual_value),
        )
        | (
            TypeKind::SortedDictionary(pattern_key, pattern_value),
            TypeKind::SortedDictionary(actual_key, actual_value),
        ) => {
            infer_type_parameter_bindings(
                types,
                diagnostics,
                callee,
                pattern_key,
                actual_key,
                span,
                bindings,
                from_context,
            );
            infer_type_parameter_bindings(
                types,
                diagnostics,
                callee,
                pattern_value,
                actual_value,
                span,
                bindings,
                from_context,
            );
        }
        (TypeKind::Class(pattern), TypeKind::Class(actual))
        | (TypeKind::Interface(pattern), TypeKind::Interface(actual))
            if pattern.name == actual.name && pattern.arguments.len() == actual.arguments.len() =>
        {
            for (pattern, actual) in pattern.arguments.into_iter().zip(actual.arguments) {
                infer_type_parameter_bindings(
                    types,
                    diagnostics,
                    callee,
                    pattern,
                    actual,
                    span,
                    bindings,
                    from_context,
                );
            }
        }
        (TypeKind::Enum(pattern), TypeKind::Enum(actual))
            if pattern.id == actual.id
                && pattern.name == actual.name
                && pattern.arguments.len() == actual.arguments.len() =>
        {
            for (pattern, actual) in pattern.arguments.into_iter().zip(actual.arguments) {
                infer_type_parameter_bindings(
                    types,
                    diagnostics,
                    callee,
                    pattern,
                    actual,
                    span,
                    bindings,
                    from_context,
                );
            }
        }
        (TypeKind::Function(pattern), TypeKind::Function(actual))
            if pattern.invocation_mode == actual.invocation_mode
                && pattern.parameters.len() == actual.parameters.len()
                && pattern
                    .parameters
                    .iter()
                    .zip(&actual.parameters)
                    .all(|(pattern, actual)| pattern.ownership_mode == actual.ownership_mode)
                && pattern.checked_effects.len() == actual.checked_effects.len() =>
        {
            for (pattern, actual) in pattern.parameters.into_iter().zip(actual.parameters) {
                infer_type_parameter_bindings(
                    types,
                    diagnostics,
                    callee,
                    pattern.ty,
                    actual.ty,
                    span,
                    bindings,
                    from_context,
                );
            }
            infer_type_parameter_bindings(
                types,
                diagnostics,
                callee,
                pattern.return_type,
                actual.return_type,
                span,
                bindings,
                from_context,
            );
            for (pattern, actual) in pattern
                .checked_effects
                .into_iter()
                .zip(actual.checked_effects)
            {
                infer_type_parameter_bindings(
                    types,
                    diagnostics,
                    callee,
                    pattern,
                    actual,
                    span,
                    bindings,
                    from_context,
                );
            }
        }
        _ => {}
    }
}
