//! Key and element storage of the hashed and ordered collections.
//!
//! A Copy enum with payload cases conforms to `Hashable` and `Comparable` when
//! its payloads do (Decision 0096), so it is a valid `Set` element,
//! `Dictionary` key, `SortedSet` element, `SortedDictionary` key and
//! `PriorityQueue` element. This compiler stores those keys in one machine
//! word and does not implement wider keys yet; E0783 marks that boundary. A
//! key whose type is a type parameter is decided per specialization, so the
//! checker records it and rechecks every concrete instance.
use super::*;

const TITLE: &str = "Enums With Payloads Cannot Be Collection Keys Yet";

#[derive(Debug, Clone)]
pub(super) struct SymbolicKey {
    span: Span,
    ty: TypeId,
    role: String,
    contract: &'static str,
}

impl Checker<'_> {
    /// Checks the storage of a key or element type that has met, or is about
    /// to be checked against, `contract` (`Hashable` or `Comparable`).
    pub(super) fn check_collection_key_storage(
        &mut self,
        ty: TypeId,
        span: Span,
        role: &str,
        contract: &'static str,
    ) {
        if self.type_is_symbolic(ty) {
            if !self
                .symbolic_collection_keys
                .iter()
                .any(|key| key.span == span && key.ty == ty)
            {
                self.symbolic_collection_keys.push(SymbolicKey {
                    span,
                    ty,
                    role: role.to_string(),
                    contract,
                });
            }
            return;
        }
        let has_payload_cases = match self.types.kind(ty) {
            TypeKind::Enum(enum_type) => {
                self.enum_definition(enum_type).is_some_and(|definition| {
                    definition.cases.iter().any(|case| !case.payload.is_empty())
                })
            }
            _ => false,
        };
        if !has_payload_cases
            || !self.primitive_satisfies_compiler_constraint(ty, contract)
            || self
                .diagnostics
                .iter()
                .any(|existing| existing.code == "E0783" && existing.span == span)
        {
            return;
        }
        let display = self.types.display(ty);
        self.diagnostics.push(
            Diagnostic::new(
                "E0783",
                format!("{role} type `{display}` is an enum with payload cases"),
                span,
            )
            .with_title(TITLE)
            .with_explanation(format!(
                "`{display}` is `{contract}`, and using an enum with payload cases as a collection key or element is part of Doria; this compiler does not implement it yet."
            ))
            .with_help(
                "key the collection by a string, an int, or a class built from the case and its payload, and keep the enum values in a `List` or as `Dictionary` values",
            ),
        );
    }

    /// Rechecks the recorded type-parameter keys inside `body` at its
    /// concrete type arguments.
    pub(super) fn check_specialized_collection_keys(
        &mut self,
        body: &super::callable_contracts::BodyInstance,
    ) {
        for key in self.symbolic_collection_keys.clone() {
            if !body.declaration.contains(key.span) {
                continue;
            }
            let ty = body.substitute_id(key.ty, &mut self.types);
            if ty != key.ty && !self.type_is_symbolic(ty) {
                self.check_collection_key_storage(ty, key.span, &key.role, key.contract);
            }
        }
    }
}
