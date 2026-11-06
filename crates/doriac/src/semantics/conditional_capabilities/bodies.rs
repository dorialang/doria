use super::super::contracts::NominalConditionFacts;
use super::super::nominal_proof::{Context, Failure};
use super::super::*;

#[derive(Debug, Clone)]
pub(in crate::semantics) struct AvailabilityCall {
    declaration: Span,
    method_parameters: Vec<String>,
    condition: NominalConditionFacts,
    bindings: HashMap<String, TypeId>,
    caller: Context,
    caller_declaration: Option<Span>,
}

impl Checker<'_> {
    pub(in crate::semantics) fn assume_callable_condition(&mut self, declaration: Span) {
        if let Some(owner) = self
            .contracts
            .callable_conditions
            .get(&declaration)
            .copied()
        {
            self.assume_declaration_condition(owner);
        }
    }

    pub(in crate::semantics) fn with_declaration_condition<T>(
        &mut self,
        declaration: Span,
        check: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let depth = self.nominal_condition_scopes.len();
        let previous_condition = self.instantiation_condition;
        if self.contracts.conditions.contains_key(&declaration) {
            self.instantiation_condition = Some(declaration);
        }
        self.assume_declaration_condition(declaration);
        let result = check(self);
        self.nominal_condition_scopes.truncate(depth);
        self.instantiation_condition = previous_condition;
        result
    }

    fn assume_declaration_condition(&mut self, declaration: Span) {
        let Some(condition) = self.contracts.conditions.get(&declaration).cloned() else {
            return;
        };
        let bindings = condition
            .parameters
            .iter()
            .map(|parameter| {
                (
                    parameter.name.clone(),
                    self.types
                        .intern(TypeKind::TypeParameter(parameter.name.clone())),
                )
            })
            .collect();
        if let Ok(steps) = self.condition_queries(&condition, &bindings) {
            self.nominal_condition_scopes.push(steps);
        }
    }

    pub(in crate::semantics) fn defer_callable_availability(
        &mut self,
        method: &MethodInfo,
        bindings: &HashMap<String, TypeId>,
        span: Span,
    ) {
        let Ok(Some(condition)) = self.callable_condition(method) else {
            return;
        };
        let method_parameters = method
            .type_params
            .iter()
            .map(|parameter| parameter.name.clone())
            .collect::<Vec<_>>();
        let mut bindings = bindings.clone();
        for parameter in &condition.parameters {
            if !method_parameters.contains(&parameter.name) {
                bindings.entry(parameter.name.clone()).or_insert_with(|| {
                    self.types
                        .intern(TypeKind::TypeParameter(parameter.name.clone()))
                });
            }
        }
        let caller = self.nominal_proof_context();
        self.pending_availability_calls.insert(
            (span, condition.clause),
            AvailabilityCall {
                declaration: method.declaration,
                method_parameters,
                condition,
                bindings,
                caller,
                caller_declaration: self.current_callable,
            },
        );
    }

    pub(in crate::semantics) fn check_pending_availability_calls(&mut self) {
        let mut calls = std::mem::take(&mut self.pending_availability_calls)
            .into_iter()
            .collect::<Vec<_>>();
        calls.sort_by_key(|(key, _)| *key);
        for ((span, _), mut call) in calls {
            if let Some(pending) = self.pending_generic_calls.get(&span) {
                if pending.declaration == call.declaration {
                    call.bindings.extend(pending.inference.bindings.clone());
                }
            }
            // Missing generic arguments receive the ordinary inference finding.
            // Result context may have completed them after the initial call.
            if call
                .method_parameters
                .iter()
                .any(|parameter| !call.bindings.contains_key(parameter))
            {
                continue;
            }
            let result = self
                .condition_queries(&call.condition, &call.bindings)
                .and_then(|steps| {
                    for step in &steps {
                        self.defer_formed_type_conformances_in(
                            step.query.subject,
                            &call.caller,
                            call.caller_declaration,
                        );
                        let interface = self
                            .types
                            .intern(TypeKind::Interface(step.query.interface.clone()));
                        self.defer_formed_type_conformances_in(
                            interface,
                            &call.caller,
                            call.caller_declaration,
                        );
                    }
                    for step in steps {
                        self.prove_nominal(step.query, &mut call.caller, step.origin, &[])?;
                    }
                    Ok(())
                });
            let Err(failure) = result else {
                continue;
            };
            let mut diagnostic = Diagnostic::new(
                "E0779",
                "this member's availability condition is not proved by the caller",
                span,
            )
            .with_title("Member Availability Condition Is Not Satisfied")
            .with_related(
                call.condition.clause,
                "the required capability condition is declared here",
            )
            .with_help("provide nominal bounds that prove the member's condition in this caller");
            match failure {
                Failure::Unsatisfied(query) => {
                    let subject = self.types.display(query.subject);
                    let interface = self.types.intern(TypeKind::Interface(query.interface));
                    diagnostic = diagnostic.with_explanation(format!(
                        "The caller does not establish that `{subject}` implements `{}`.",
                        self.types.display(interface),
                    ));
                }
                Failure::Cycle(chain) => {
                    diagnostic = diagnostic.with_explanation(
                        "A circular nominal proof cannot establish its own capability premise.",
                    );
                    for step in chain {
                        if let Some(origin) = step.origin {
                            diagnostic = diagnostic.with_related(
                                origin,
                                "this bound participates in the circular proof",
                            );
                        }
                    }
                }
                Failure::UnresolvedCondition(origin) => {
                    diagnostic = diagnostic.with_related(
                        origin,
                        "this condition has no valid resolved declaration facts",
                    );
                }
                Failure::MissingSubstitution(parameter) => {
                    diagnostic = diagnostic.with_related(
                        parameter.declaration,
                        "this condition parameter has no checked substitution",
                    );
                }
            }
            self.diagnostics.push(diagnostic);
        }
    }
}
