//! Type arguments stated at a call (Decision 0105, Amendment: Type Arguments
//! At The Call).
//!
//! Name resolution has already chosen the reading of every `callee<A>(...)`,
//! so a call here either states a list or does not. One rule covers every
//! generic callee: the list names each type parameter in order, each
//! argument meets its constraints, and the stated arguments are fixed before
//! the call's arguments are inferred.

use super::*;

pub(super) const NOT_GENERIC_CODE: &str = "E0811";
pub(super) const TYPE_ARGUMENT_COUNT_CODE: &str = "E0812";

/// A list stated at one call, with the class whose body states it.
#[derive(Debug, Clone)]
pub(super) struct StatedTypeArguments {
    list: CallTypeArguments,
    declaring_class: Option<String>,
    consumed: bool,
}

impl StatedTypeArguments {
    /// From `<` to `>`.
    pub(super) fn span(&self) -> Span {
        self.list.span
    }
}

impl<'program> Checker<'program> {
    /// Records the list `expr` states, if it is a call that states one.
    pub(super) fn register_stated_type_arguments(
        &mut self,
        expr: &Expr,
        method_context: Option<&MethodContext>,
    ) {
        let (Expr::FunctionCall {
            type_arguments: Some(list),
            span,
            ..
        }
        | Expr::MethodCall {
            type_arguments: Some(list),
            span,
            ..
        }
        | Expr::StaticCall {
            type_arguments: Some(list),
            span,
            ..
        }) = expr
        else {
            return;
        };
        self.stated_type_arguments
            .entry(*span)
            .or_insert_with(|| StatedTypeArguments {
                list: list.clone(),
                declaring_class: method_context.map(|context| context.class_name.clone()),
                consumed: false,
            });
    }

    /// The bindings the call at `span` states for `type_params`: empty when
    /// it states none, and after reporting when its list does not fit.
    pub(super) fn stated_type_argument_bindings(
        &mut self,
        callee: &str,
        type_params: &[TypeParamInfo],
        span: Span,
    ) -> HashMap<String, TypeId> {
        let Some(arguments) = self.stated_type_argument_types(callee, type_params.len(), span)
        else {
            return HashMap::new();
        };
        type_params
            .iter()
            .map(|param| param.name.clone())
            .zip(arguments)
            .collect()
    }

    /// The types the call at `span` states, when it states exactly
    /// `expected` of them. Reports a list on a callee with no type
    /// parameters, or of the wrong length.
    pub(super) fn stated_type_argument_types(
        &mut self,
        callee: &str,
        expected: usize,
        span: Span,
    ) -> Option<Vec<TypeId>> {
        self.stated_type_argument_types_in_position(callee, expected, span, TypePosition::Value)
    }

    /// [`Self::stated_type_argument_types`] for a callee whose type
    /// parameters each occupy `position`, as the pointee parameter of
    /// `Pointer::cast` occupies [`TypePosition::Pointee`].
    pub(super) fn stated_type_argument_types_in_position(
        &mut self,
        callee: &str,
        expected: usize,
        span: Span,
        position: TypePosition,
    ) -> Option<Vec<TypeId>> {
        let stated = self.stated_type_arguments.get_mut(&span)?;
        stated.consumed = true;
        let list = stated.list.clone();
        let declaring_class = stated.declaring_class.clone();
        let found = list.arguments.len();
        if found != expected {
            self.report_type_argument_count(callee, expected, &list);
            return None;
        }
        Some(
            list.arguments
                .iter()
                .map(|argument| {
                    self.resolve_type_ref_in_position(
                        argument,
                        list.span,
                        position,
                        declaring_class.as_deref(),
                    )
                })
                .collect(),
        )
    }

    /// `List::map<U>` and `List::reduce<A>` declare one type parameter and
    /// `List::filter` none (Stage 30g).
    pub(super) fn stated_list_algorithm_type_argument(
        &mut self,
        method: &str,
        span: Span,
    ) -> Option<TypeId> {
        let expected = usize::from(matches!(method, "map" | "reduce"));
        self.stated_type_argument_types(&format!("`List::{method}`"), expected, span)
            .and_then(|types| types.first().copied())
    }

    /// Reports once per list. Inference may read a call more than once and
    /// discard what a tentative read reported, so the check is against the
    /// diagnostics kept, never a flag.
    fn report_type_argument_count(
        &mut self,
        callee: &str,
        expected: usize,
        list: &CallTypeArguments,
    ) {
        if self.diagnostics.iter().any(|diagnostic| {
            matches!(diagnostic.code, NOT_GENERIC_CODE | TYPE_ARGUMENT_COUNT_CODE)
                && diagnostic.span == list.span
        }) {
            return;
        }
        let found = list.arguments.len();
        if expected == 0 {
            self.diagnostics.push(
                Diagnostic::new(
                    NOT_GENERIC_CODE,
                    format!("{callee} has no type parameters, and this call states {found} type argument{}", plural(found)),
                    list.span,
                )
                .with_title("Callee Takes No Type Arguments")
                .with_help("remove the type arguments")
                .with_fix(list.span, ""),
            );
            return;
        }
        self.diagnostics.push(
            Diagnostic::new(
                TYPE_ARGUMENT_COUNT_CODE,
                format!(
                    "{callee} has {expected} type parameter{}, and this call states {found}",
                    plural(expected)
                ),
                list.span,
            )
            .with_title("Wrong Number Of Type Arguments")
            .with_help(format!(
                "state one type argument for each of the {expected} type parameter{} in order, or none and let the arguments determine them",
                plural(expected)
            )),
        );
    }

    /// Every stated list that no generic callee read belongs to a call whose
    /// callee has no type parameters. A call that already failed reports
    /// nothing more.
    pub(super) fn report_unconsumed_type_arguments(&mut self) {
        let mut unconsumed = self
            .stated_type_arguments
            .iter()
            .filter(|(_, stated)| !stated.consumed)
            .map(|(span, stated)| (*span, stated.list.clone()))
            .collect::<Vec<_>>();
        unconsumed.sort_by_key(|(span, _)| *span);
        for (call, list) in unconsumed {
            let call_failed = self.diagnostics.iter().any(|diagnostic| {
                diagnostic.severity == crate::diagnostics::DiagnosticSeverity::Error
                    && diagnostic.span.source == call.source
                    && diagnostic.span.start >= call.start
                    && diagnostic.span.end <= call.end
            });
            if !call_failed {
                self.report_type_argument_count("this callee", 0, &list);
            }
        }
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}
