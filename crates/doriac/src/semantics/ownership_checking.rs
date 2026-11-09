//! Build one ownership context from checked callable and nominal facts.

use super::*;

impl Checker<'_> {
    pub(super) fn check_constructor_initialization(
        &mut self,
        backing_fields: &HashMap<Span, crate::property_hooks::PropertyBackingField>,
    ) -> Vec<crate::constructor_init::ConstructorPartialCleanup> {
        let contexts = self
            .classes
            .values()
            .map(|class| {
                let parent = class
                    .parent
                    .as_ref()
                    .and_then(|parent| self.classes.get(&parent.name));
                let mut inherited = HashSet::new();
                let mut ancestor = parent;
                let mut visited = HashSet::new();
                while let Some(info) = ancestor {
                    if !visited.insert(info.declaration) {
                        break;
                    }
                    for property in info.properties.values() {
                        if property.hooks.as_ref().is_some_and(|hooks| {
                            hooks.storage
                                == Some(crate::property_hooks::PropertyHookStorage::Computed)
                        }) {
                            continue;
                        }
                        inherited.insert(
                            backing_fields
                                .get(&property.declaration_span)
                                .map_or(property.declaration_span, |field| field.declaration),
                        );
                    }
                    ancestor = info
                        .parent
                        .as_ref()
                        .and_then(|parent| self.classes.get(&parent.name));
                }
                let mut inherited_stored_properties = inherited.into_iter().collect::<Vec<_>>();
                inherited_stored_properties.sort();
                (
                    class.declaration,
                    crate::constructor_init::ConstructorCleanupContext {
                        parent_class: parent.map(|parent| parent.declaration),
                        parent_constructor: parent
                            .and_then(|parent| parent.methods.get("__construct"))
                            .map(|method| method.declaration),
                        inherited_stored_properties,
                    },
                )
            })
            .collect();
        let analysis = crate::constructor_init::check_program_with_cleanup_context(
            self.program,
            backing_fields,
            &self.given_preludes,
            &self.checked_effect_sites,
            &self.catch_error_types,
            &self.catch_coverage,
            &contexts,
        );
        for (span, kind) in analysis.property_writes {
            if let Some(write) = self.property_writes.get_mut(&span) {
                write.kind = kind;
            }
        }
        self.diagnostics.extend(analysis.diagnostics);
        analysis.partial_cleanups
    }

    pub(super) fn check_post_ownership_contracts(
        &mut self,
        analysis: &mut crate::ownership::OwnershipAnalysis,
        provenance: &mut nonblocking::CallProvenance,
        partial_cleanups: &[crate::constructor_init::ConstructorPartialCleanup],
    ) {
        // Return-loan inference publishes the same carrier facts to all consumers.
        for (binding, ownership) in &analysis.binding_ownership {
            if let Some(declaration) = self.binding_resolution.declarations_by_id.get_mut(binding) {
                declaration.ownership = *ownership;
            }
        }
        let mut accessors = std::mem::take(&mut self.property_accessor_calls);
        for calls in accessors.values_mut() {
            if let Some(getter) = &mut calls.getter {
                let result = self.types.intern_resolved(&getter.return_type);
                if !self.type_is_move_type(result) {
                    getter.return_borrow = None;
                } else if !matches!(
                    getter.declaring_type,
                    ResolvedType::Interface(_) | ResolvedType::TypeParameter(_)
                ) {
                    getter.return_borrow =
                        analysis.return_borrows.get(&getter.declaration).copied();
                }
            }
        }
        self.property_accessor_calls = accessors;
        self.check_nonblocking_contracts(provenance, &analysis.cleanup, partial_cleanups);
        self.check_call_scoped_requests(&analysis.closures, &analysis.return_borrows);
        self.diagnostics.append(&mut analysis.diagnostics);
        self.check_property_hook_return_contracts(&analysis.return_borrows);
        self.invalidate_erroneous_compositions();
    }

    pub(super) fn interface_conversion_types(&self) -> HashMap<Span, ResolvedType> {
        self.contextual_expression_types
            .iter()
            .filter_map(|(span, ty)| {
                let resolved = self.types.resolved(*ty);
                let inner = if let ResolvedType::Nullable(inner) = &resolved {
                    inner.as_ref()
                } else {
                    &resolved
                };
                matches!(inner, ResolvedType::Interface(_)).then_some((*span, resolved))
            })
            .collect()
    }

    pub(super) fn analyze_ownership(
        &mut self,
        program: &Program,
        classes: &[ClassSemanticInfo],
        enums: &[EnumSemanticInfo],
        interface_conversion_types: &HashMap<Span, ResolvedType>,
        provenance: &nonblocking::CallProvenance,
    ) -> (
        specialized_ownership::FactsByInstance,
        crate::ownership::OwnershipAnalysis,
    ) {
        let inferred_move_returns = self
            .function_signatures
            .iter()
            .filter_map(|(span, signature)| {
                self.type_is_move_type(signature.return_ty).then_some(*span)
            })
            .collect();
        let return_borrows = self
            .function_signatures
            .iter()
            .map(|(span, signature)| (*span, signature.return_borrow))
            .collect();
        let move_enum_names = crate::ownership::MoveEnums::new(
            self.enums
                .values()
                .filter(|definition| !definition.capabilities.copy)
                .map(|definition| definition.name.clone()),
            self.enum_instances.values().map(|definition| {
                (
                    definition
                        .source_type
                        .map_arguments(|argument| self.types.resolved(*argument)),
                    !definition.capabilities.copy,
                )
            }),
        );
        let (callback_capture_targets, callback_value_targets) =
            self.callback_capture_targets(provenance);
        let source_types = match self.callable_body_program {
            CallableBodyProgram::Expanded(_) => {
                crate::ownership::SourceTypes::from_semantic_classes(
                    classes,
                    &super::collect_ordered_enum_semantics(self),
                    &self.contracts,
                )
            }
            CallableBodyProgram::Authored(_) => self.retained_source_types(),
        };
        let context = crate::ownership::OwnershipAnalysisContext {
            inferred_move_returns: &inferred_move_returns,
            return_borrows: &return_borrows,
            specialized_return_borrows: &self.specialized_return_borrow_seed,
            resolved_types: &self.expression_types,
            interface_conversion_types,
            flow_facts: &self.flow_facts,
            move_enum_names: &move_enum_names,
            given_preludes: &self.given_preludes,
            checked_effect_sites: &self.checked_effect_sites,
            catch_error_types: &self.catch_error_types,
            catch_coverage: &self.catch_coverage,
            binding_resolution: &self.binding_resolution,
            closures: &self.closures,
            callable_value_calls: &self.callable_value_calls,
            callback_capture_targets: &callback_capture_targets,
            callback_value_targets: &callback_value_targets,
            assertion_callable_invocations: &self.assertion_callable_invocations,
            list_algorithm_calls: &self.list_algorithm_calls,
            call_targets: &self.call_targets,
            property_accessor_calls: &self.property_accessor_calls,
            property_writes: &self.property_writes,
            matches: &self.matches,
            enum_semantics: enums,
            enum_case_constructions: &self.enum_case_constructions,
            contracts: &self.contracts,
            classes,
            generic_call_specializations: &self.generic_call_specializations,
            composition: &self.composition,
            source_types: &source_types,
            retained_callable_seed: &self.retained_callable_seed,
        };
        let mut retained = self.retained_callable_seed.clone();
        // Checked composing receivers refine storage homes before caller diagnostics.
        // Authored and composed summaries share the recursive dependency graph.
        let mut analysis = loop {
            let context = crate::ownership::OwnershipAnalysisContext {
                retained_callable_seed: &retained,
                ..context
            };
            let analysis = match self.callable_body_program {
                CallableBodyProgram::Expanded(_) => {
                    crate::ownership::check_program_with_inferred_move_returns(program, &context)
                }
                CallableBodyProgram::Authored(_) => {
                    crate::ownership::check_authored_program_with_inferred_move_returns(
                        program, &context,
                    )
                }
            };
            if matches!(self.callable_body_program, CallableBodyProgram::Expanded(_))
                || self.composition.callable_templates.is_empty()
            {
                break analysis;
            }
            let mut refined = analysis.retained_callables.clone();
            let composed = specialized_ownership::composed_retained_summaries(
                program,
                self.callable_body_program,
                &crate::ownership::OwnershipAnalysisContext {
                    retained_callable_seed: &refined,
                    ..context
                },
                &self.types,
            );
            refined.extend(composed);
            if refined == retained {
                break analysis;
            }
            retained = refined;
        };
        let context = crate::ownership::OwnershipAnalysisContext {
            retained_callable_seed: &retained,
            ..context
        };
        let (specialized, specialized_analysis) = specialized_ownership::analyze(
            program,
            self.callable_body_program,
            &context,
            &self.callable_instances,
            &self.types,
        );
        self.diagnostics.extend(specialized_analysis.diagnostics);
        analysis
            .writeback_constraints
            .extend(specialized_analysis.writeback_constraints);
        for constraint in std::mem::take(&mut analysis.writeback_constraints) {
            let target = self.types.intern_resolved(&constraint.home);
            let value = self.types.intern_resolved(&constraint.value);
            if !self.is_assignable(target, value) {
                let home = self.types.display(target);
                let value = self.types.display(value);
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0403",
                        format!("writable borrow can replace `{home}` storage with `{value}`"),
                        constraint.site,
                    )
                    .with_title("Type Mismatch")
                    .with_explanation("A writable view keeps its original storage type. Replacing its value must remain valid for that storage.")
                    .with_related(constraint.write_site, "This Operation Can Replace the Stored Value")
                    .with_help("Use a compatible replacement or mutate the existing value in place."),
                );
            }
        }
        (specialized, analysis)
    }
}
