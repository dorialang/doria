//! Checked declaration and owner bindings for concrete interface witnesses.

use super::*;
use crate::types::InterfaceType;
use contracts::{ConformanceFacts, ConformanceStatus, RequirementOrigin};

type Key = (ClassType<ResolvedType>, InterfaceType<ResolvedType>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Witness {
    pub origins: Vec<RequirementOrigin>,
    pub declaration: Span,
    pub parameters: Vec<String>,
    pub exact_dynamic_return: Option<ResolvedType>,
    owner_bindings: HashMap<String, ResolvedType>,
}

impl Witness {
    pub(crate) fn instance(
        &self,
        arguments: &[GenericArgument],
        site: Span,
    ) -> DiagnosticResult<CallableInstanceKey> {
        if self.parameters.len() != arguments.len() {
            return Err(failure(
                site,
                "checked interface witness has inconsistent generic arity",
            ));
        }
        let mut bindings = self.owner_bindings.clone();
        for (parameter, argument) in self.parameters.iter().zip(arguments) {
            let GenericArgument::Type(ty) = argument;
            if bindings.insert(parameter.clone(), ty.clone()).is_some() {
                return Err(failure(
                    site,
                    "checked interface witness has conflicting type bindings",
                ));
            }
        }
        Ok(CallableInstanceKey::new(self.declaration, &bindings))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum Plan {
    #[default]
    Unprepared,
    Checked(HashMap<Key, Vec<Witness>>),
}

impl Plan {
    pub(crate) fn conformance(&self, fact: &ConformanceFacts) -> DiagnosticResult<&[Witness]> {
        let Self::Checked(plans) = self else {
            return Err(failure(
                fact.origin,
                "checked interface-witness plan is unavailable",
            ));
        };
        let ResolvedType::Class(class) = &fact.implementing_type else {
            return Err(failure(
                fact.origin,
                "checked class witness has another implementing type",
            ));
        };
        let witnesses = plans
            .get(&(class.clone(), fact.interface.clone()))
            .ok_or_else(|| {
                failure(
                    fact.origin,
                    "checked conformance has no canonical witness plan",
                )
            })?;
        if fact.status != ConformanceStatus::Checked
            || witnesses.len() != fact.implementations.len()
            || witnesses
                .iter()
                .zip(&fact.implementations)
                .any(|(witness, implementation)| {
                    implementation.requirement_origins != witness.origins
                        || implementation.implementation != Some(witness.declaration)
                        || !implementation.failures.is_empty()
                        || implementation.exact_dynamic_return != witness.exact_dynamic_return
                })
        {
            return Err(failure(
                fact.origin,
                "checked conformance differs from its canonical witnesses",
            ));
        }
        Ok(witnesses)
    }
}

fn failure(site: Span, message: &str) -> Vec<Diagnostic> {
    vec![Diagnostic::new("I2401", message, site)]
}

pub(super) fn collect(
    declarations: &callable_contracts::MemberDeclarations<'_>,
    classes: &[ClassSemanticInfo],
    contracts: &contracts::ContractFacts,
) -> DiagnosticResult<Plan> {
    let classes = classes
        .iter()
        .map(|class| {
            (
                ClassType::new(class.declaration_name.clone(), class.arguments.clone()),
                class,
            )
        })
        .collect::<HashMap<_, _>>();
    let mut plans = HashMap::new();
    for conformance in &contracts.conformances {
        if conformance.status != ConformanceStatus::Checked {
            continue;
        }
        let ResolvedType::Class(receiver) = &conformance.implementing_type else {
            continue;
        };
        let Some(class) = classes.get(receiver) else {
            continue;
        };
        let mut witnesses = Vec::with_capacity(conformance.implementations.len());
        for implementation in &conformance.implementations {
            let declaration = implementation.implementation.ok_or_else(|| {
                failure(
                    conformance.origin,
                    "checked interface witness has no implementation declaration",
                )
            })?;
            let (function, owner) = declarations.get(&declaration).ok_or_else(|| {
                failure(
                    declaration,
                    "checked interface witness has no callable declaration",
                )
            })?;
            let owner_type = std::iter::once(receiver)
                .chain(&class.ancestors)
                .find(|ty| ty.name == owner.name)
                .ok_or_else(|| {
                    failure(
                        declaration,
                        "checked interface witness is outside its owner hierarchy",
                    )
                })?;
            if owner.type_params.len() != owner_type.arguments.len() {
                return Err(failure(
                    declaration,
                    "checked interface witness has incomplete owner bindings",
                ));
            }
            witnesses.push(Witness {
                origins: implementation.requirement_origins.clone(),
                declaration,
                parameters: function
                    .type_params
                    .iter()
                    .map(|parameter| parameter.name.clone())
                    .collect(),
                exact_dynamic_return: implementation.exact_dynamic_return.clone(),
                owner_bindings: owner
                    .type_params
                    .iter()
                    .zip(&owner_type.arguments)
                    .map(|(parameter, ty)| (parameter.name.clone(), ty.clone()))
                    .collect(),
            });
        }
        let key = (receiver.clone(), conformance.interface.clone());
        if plans.insert(key, witnesses).is_some() {
            return Err(failure(
                conformance.origin,
                "checked conformance has duplicate witness plans",
            ));
        }
    }
    Ok(Plan::Checked(plans))
}
