//! PHP output for the core value operations of Copy enums (Decision 0096).
//!
//! Equality is exact: a unit enum is a native PHP enum compared by identity,
//! and an enum with payloads compares its case and payloads through
//! `__doriaEquals`. The order and the hash are part of Doria and not generated
//! yet, so the backend refuses them with B4601 before it writes any PHP.
use super::*;
use crate::compiler_known_contracts::CoreValueOperation;
use crate::semantics::CallableTarget;

const CODE: &str = "B4601";
const TITLE: &str = "PHP Target Cannot Order Or Hash Enum Values Yet";

fn refused(span: Span, operation: impl Into<String>) -> BackendError {
    BackendError::from_diagnostics(vec![Diagnostic::new(
        CODE,
        format!(
            "PHP compatibility backend does not generate {} yet; use the `native` or `debug` target for this valid Doria program",
            operation.into()
        ),
        span,
    )
    .with_title(TITLE)])
}

/// Refuses `hash` and `compare` selected for an enum receiver.
pub(super) fn validate_call(
    target: Option<CallableTarget>,
    span: Span,
) -> Result<(), BackendError> {
    let Some(CallableTarget::ConstrainedMethod {
        receiver: receiver @ ResolvedType::Enum(_),
        requirement,
        ..
    }) = target
    else {
        return Ok(());
    };
    match CoreValueOperation::from_requirement(requirement) {
        Some(operation @ (CoreValueOperation::Hash | CoreValueOperation::Compare)) => Err(refused(
            span,
            format!(
                "`{}` for the enum `{}`",
                operation.method(),
                crate::attributes::metadata_type_name(&receiver)
            ),
        )),
        _ => Ok(()),
    }
}

/// Refuses a hashed or ordered collection keyed by an enum.
pub(super) fn validate_collection(ty: &ResolvedType, span: Span) -> Result<(), BackendError> {
    let (collection, key) = match ty {
        ResolvedType::Set(key) => ("Set", key),
        ResolvedType::SortedSet(key) => ("SortedSet", key),
        ResolvedType::PriorityQueue(key) => ("PriorityQueue", key),
        ResolvedType::Dictionary(key, _) => ("Dictionary", key),
        ResolvedType::SortedDictionary(key, _) => ("SortedDictionary", key),
        _ => return Ok(()),
    };
    if matches!(**key, ResolvedType::Enum(_)) {
        return Err(refused(
            span,
            format!(
                "a `{collection}` keyed by the enum `{}`",
                crate::attributes::metadata_type_name(key)
            ),
        ));
    }
    Ok(())
}

/// The PHP expression for `$left->equals($right)` on enum values.
pub(super) fn equal(left: &str, right: &str) -> String {
    format!("__doria_equal({left}, {right})")
}
