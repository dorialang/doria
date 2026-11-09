use super::*;
use crate::semantics::nominal_proof::{Query, Step};

impl Checker<'_> {
    pub(in crate::semantics) fn callable_condition(
        &self,
        method: &MethodInfo,
    ) -> Result<Option<NominalConditionFacts>, ()> {
        let Some(owner) = self.contracts.callable_conditions.get(&method.declaration) else {
            return Ok(None);
        };
        self.contracts
            .conditions
            .get(owner)
            .cloned()
            .map(Some)
            .ok_or(())
    }

    fn condition_method_bindings(
        &mut self,
        method: &MethodInfo,
        condition: &NominalConditionFacts,
    ) -> HashMap<String, TypeId> {
        let mut bindings = self.normalized_method_bindings(method);
        for parameter in &condition.parameters {
            bindings.entry(parameter.name.clone()).or_insert_with(|| {
                self.types
                    .intern(TypeKind::TypeParameter(parameter.name.clone()))
            });
        }
        bindings
    }

    pub(super) fn callable_availability_preserved(
        &mut self,
        actual: &MethodInfo,
        required: &MethodInfo,
    ) -> bool {
        let conclusion = match self.callable_condition(actual) {
            Ok(None) => return true,
            Ok(Some(condition)) => condition,
            Err(()) => return false,
        };
        let mut context = self.nominal_proof_context();
        // The interface/base method's generic bounds are caller premises.
        // Its implementation's additional guard is never an assumption.
        for (index, bounds) in self
            .normalized_method_constraints(required)
            .into_iter()
            .enumerate()
        {
            let subject = self
                .types
                .intern(TypeKind::TypeParameter(format!("#method{index}")));
            for (name, arguments) in bounds {
                context.assume(Step {
                    query: Query {
                        subject,
                        interface: InterfaceType::new(name, arguments),
                    },
                    origin: Some(required.declaration),
                });
            }
        }
        match self.callable_condition(required) {
            Err(()) => return false,
            Ok(None) => {}
            Ok(Some(premise)) => {
                let bindings = self.condition_method_bindings(required, &premise);
                let Ok(steps) = self.condition_queries(&premise, &bindings) else {
                    return false;
                };
                for step in steps {
                    context.assume(step);
                }
            }
        }
        let bindings = self.condition_method_bindings(actual, &conclusion);
        let Ok(steps) = self.condition_queries(&conclusion, &bindings) else {
            return false;
        };
        steps.into_iter().all(|step| {
            self.prove_nominal(step.query, &mut context, step.origin, &[])
                .is_ok()
        })
    }

    pub(in crate::semantics) fn check_conditional_availability(&mut self) {
        let declarations = self
            .program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Class(class) => Some(class.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut reported = HashSet::new();
        for declaration in declarations {
            let ty = self.symbolic_class_type(&declaration.name);
            let Some(class) = self.class_type(ty) else {
                continue;
            };
            let ancestors = self.specialized_ancestor_types(&class);
            if ancestors.iter().any(|ancestor| ancestor.name == class.name) {
                continue;
            }
            self.type_parameter_scopes
                .push(type_parameter_scope(&declaration.type_params));
            self.check_property_override_availability(&class);
            for owner in std::iter::once(class.clone()).chain(ancestors) {
                let Some(authored) = self.program.items.iter().find_map(|item| match item {
                    Item::Class(authored) if authored.name == owner.name => Some(authored.clone()),
                    _ => None,
                }) else {
                    continue;
                };
                let bindings = self.class_type_substitutions(&owner);
                for (entry, origin) in authored
                    .implements
                    .iter()
                    .zip(&authored.syntax.inheritance_type_spans)
                {
                    self.type_parameter_scopes
                        .push(type_parameter_scope(&authored.type_params));
                    let interface = self
                        .with_nominal_identity(|checker| {
                            checker.resolve_interface_edge_in_owner(
                                &entry.ty,
                                *origin,
                                Some(&owner.name),
                            )
                        })
                        .map(|mut interface| {
                            interface.arguments = interface
                                .arguments
                                .iter()
                                .map(|ty| self.substitute_type_id(*ty, &bindings))
                                .collect();
                            interface
                        });
                    self.type_parameter_scopes.pop();
                    let Some(interface) = interface else {
                        continue;
                    };
                    let premise = if entry.condition.is_some() {
                        let Some(condition) = self.contracts.conditions.get(origin).cloned() else {
                            continue;
                        };
                        let Ok(steps) = self.condition_queries(&condition, &bindings) else {
                            continue;
                        };
                        steps
                    } else {
                        Vec::new()
                    };
                    self.nominal_condition_scopes.push(premise);
                    let requirements =
                        self.canonical_interface_requirements(&interface, &mut Vec::new());
                    if requirements.valid {
                        for requirement in requirements.requirements {
                            let method = if let Some(kind) = requirement.accessor {
                                self.lookup_instance_property(&class, &requirement.name)
                                    .and_then(|(owner, property)| {
                                        self.property_accessor_method(&property, &owner, kind)
                                    })
                            } else {
                                self.find_concrete_contract_method(&class, &requirement.name)
                            };
                            let Some(method) = method else {
                                continue;
                            };
                            if !self.callable_availability_preserved(&method, &requirement.method)
                                && reported.insert((
                                    self.contracts.callable_conditions[&method.declaration],
                                    interface.name.clone(),
                                    requirement.name.clone(),
                                ))
                            {
                                self.report_requirement_mismatch(
                                    &requirement.name,
                                    &method,
                                    &requirement,
                                    &[ContractMismatch::Availability],
                                );
                            }
                        }
                    }
                    self.nominal_condition_scopes.pop();
                }
            }
            self.type_parameter_scopes.pop();
        }
    }

    fn check_property_override_availability(&mut self, owner: &ClassType<TypeId>) {
        let class = self.classes[&owner.name].clone();
        let Some(parent) = &class.parent else {
            return;
        };
        let mut properties = class.properties.iter().collect::<Vec<_>>();
        properties.sort_by_key(|(_, property)| property.declaration_span);
        for (name, property) in properties {
            if !property
                .hooks
                .as_ref()
                .is_some_and(|hooks| hooks.is_override)
            {
                continue;
            }
            let Some((inherited_owner, _)) = self.lookup_inherited_member(parent, name) else {
                continue;
            };
            let Some(inherited) = self.classes[&inherited_owner.name]
                .properties
                .get(name)
                .cloned()
            else {
                continue;
            };
            for kind in [
                crate::ast::PropertyHookKind::Get,
                crate::ast::PropertyHookKind::Set,
            ] {
                let Some(actual) = self.property_accessor_method(property, owner, kind) else {
                    continue;
                };
                let Some(required) =
                    self.property_accessor_method(&inherited, &inherited_owner, kind)
                else {
                    continue;
                };
                if !self.callable_availability_preserved(&actual, &required) {
                    let condition = self.callable_condition(&actual).ok().flatten();
                    let mut diagnostic = Diagnostic::new("E0729", format!("property `{}::{name}` strengthens its inherited availability condition", owner.name), condition.as_ref().map_or(actual.declaration, |condition| condition.clause))
                        .with_title("Override Contract Does Not Match")
                        .with_related(required.declaration, "the inherited accessor contract is declared here");
                    if let Ok(Some(condition)) = self.callable_condition(&actual) {
                        diagnostic = diagnostic.with_related(
                            condition.clause,
                            "this property's condition must follow from its inherited contract",
                        );
                    }
                    self.diagnostics.push(diagnostic);
                    break;
                }
            }
        }
    }
}
