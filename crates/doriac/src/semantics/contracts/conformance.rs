//! Cache declaration paths; availability belongs to the current caller's proof.

use super::*;
use crate::semantics::nominal_proof::{Context, EdgeKind};

impl Checker<'_> {
    /// Source obligations include conditional edges without publishing a witness.
    pub(in crate::semantics) fn declared_requirement_implementations(
        &mut self,
        origins: &HashSet<Span>,
    ) -> HashSet<Span> {
        let names = self.classes.keys().cloned().collect::<Vec<_>>();
        let mut implementations = HashSet::new();
        for name in names {
            let ty = self.symbolic_class_type(&name);
            let Some(class) = self.class_type(ty) else {
                continue;
            };
            let declared = self.declared_class_conformances(&class, &mut HashSet::new());
            for path in declared.paths {
                if path.obligations.is_err() {
                    continue;
                }
                let requirements = self.with_nominal_identity(|checker| {
                    checker.canonical_interface_requirements(&path.interface, &mut Vec::new())
                });
                if !requirements.valid {
                    continue;
                }
                for requirement in requirements.requirements {
                    if !requirement
                        .origins
                        .iter()
                        .any(|(_, declaration)| origins.contains(declaration))
                    {
                        continue;
                    }
                    let method = if let Some(kind) = requirement.accessor {
                        self.lookup_instance_property(&class, &requirement.name)
                            .and_then(|(owner, property)| {
                                self.property_accessor_method(&property, &owner, kind)
                            })
                    } else {
                        self.find_concrete_contract_method(&class, &requirement.name)
                    };
                    if let Some(method) = method {
                        implementations.insert(method.declaration);
                    }
                }
            }
        }
        implementations
    }

    pub(in crate::semantics) fn check_class_conformance_types(&mut self) {
        debug_assert!(self.type_parameter_scopes.is_empty());
        debug_assert!(self.nominal_condition_scopes.is_empty());
        for item in &self.program.items {
            let Item::Class(class) = item else {
                continue;
            };
            self.type_parameter_scopes
                .push(type_parameter_scope(&class.type_params));
            let mut direct = HashMap::new();
            for (entry, origin) in class
                .implements
                .iter()
                .zip(&class.syntax.inheritance_type_spans)
            {
                let mut interface = self.with_declaration_condition(*origin, |checker| {
                    checker.resolve_interface_edge_in_owner(&entry.ty, *origin, Some(&class.name))
                });
                // Duplicate identity is authored; concrete substitution can
                // make distinct source paths equal without duplicating source.
                if let Some(resolved) = &interface {
                    if let Some(previous) = direct.get(resolved) {
                        self.report_class_conformance_overlap(resolved, *origin, *previous, true);
                        interface = None;
                    } else {
                        direct.insert(resolved.clone(), *origin);
                    }
                }
                self.declared_conformance_types.insert(*origin, interface);
            }
            self.type_parameter_scopes.pop();
        }
        self.declared_class_conformance_cache.clear();
    }

    pub(in crate::semantics) fn class_interface_closure(
        &mut self,
        class: &ClassType<TypeId>,
        visiting: &mut HashSet<String>,
    ) -> ClassConformances {
        let mut caller = self.nominal_proof_context();
        self.class_interface_closure_in(class, visiting, &mut caller)
    }

    pub(in crate::semantics) fn class_interface_closure_in(
        &mut self,
        class: &ClassType<TypeId>,
        visiting: &mut HashSet<String>,
        caller: &mut Context,
    ) -> ClassConformances {
        let declared = self.declared_class_conformances(class, visiting);
        let active = declared
            .paths
            .iter()
            .filter(|path| {
                path.obligations.as_ref().is_ok_and(|obligations| {
                    obligations.iter().all(|step| {
                        self.prove_nominal(step.query.clone(), caller, step.origin, &[])
                            .is_ok()
                    })
                })
            })
            .collect::<Vec<_>>();
        let mut result = ClassConformances {
            entries: Vec::new(),
            active_arguments: active
                .iter()
                .flat_map(|path| path.interface.arguments.iter().copied())
                .collect(),
            valid: declared.valid,
        };
        let mut canonical = Vec::<&ClassConformancePath>::new();
        for (index, path) in active.iter().enumerate() {
            if !canonical
                .iter()
                .any(|previous| previous.interface.name == path.interface.name)
            {
                canonical.push(path);
                result.entries.push((path.interface.clone(), path.origin));
            }
            for previous in active[..index]
                .iter()
                .filter(|previous| previous.interface.name == path.interface.name)
            {
                if previous
                    .interface
                    .arguments
                    .iter()
                    .zip(&path.interface.arguments)
                    .any(|(left, right)| {
                        left != right
                            && !self.type_is_symbolic(*left)
                            && !self.type_is_symbolic(*right)
                    })
                {
                    result.valid = false;
                    self.report_class_conformance_overlap(
                        &path.interface,
                        path.origin,
                        previous.origin,
                        false,
                    );
                }
            }
            // Authored repeats remain errors. Conditional inherited paths may
            // extend availability, with their active overlap coalesced above.
            if path.direct && !path.conditional {
                if let Some(previous) = active[..index].iter().find(|previous| {
                    previous.declaring_class != path.declaring_class
                        && !previous.conditional
                        && previous.interface == path.interface
                }) {
                    result.valid = false;
                    self.report_class_conformance_overlap(
                        &path.interface,
                        path.origin,
                        previous.origin,
                        true,
                    );
                }
            }
        }
        result
    }

    fn declared_class_conformances(
        &mut self,
        class: &ClassType<TypeId>,
        visiting: &mut HashSet<String>,
    ) -> DeclaredClassConformances {
        if let Some(cached) = self.declared_class_conformance_cache.get(class) {
            return cached.clone();
        }
        if !visiting.insert(class.name.clone()) {
            return DeclaredClassConformances::default();
        }
        let subject = self.types.intern(TypeKind::Class(class.clone()));
        let edges = self.nominal_edges(subject);
        let mut result = DeclaredClassConformances {
            paths: Vec::new(),
            valid: edges.valid,
        };
        for edge in &edges.entries {
            if edge.kind == EdgeKind::ClassParent {
                if let TypeKind::Class(parent) = self.types.kind(edge.ty).clone() {
                    let inherited = self.declared_class_conformances(&parent, visiting);
                    result.valid &= inherited.valid;
                    result.paths.extend(inherited.paths);
                }
            }
        }
        for edge in edges.entries {
            if edge.kind != EdgeKind::Conformance {
                continue;
            }
            let Some(Some(declared)) = self.declared_conformance_types.get(&edge.origin).cloned()
            else {
                result.valid = false;
                continue;
            };
            let bindings = self.class_type_substitutions(class);
            let interface = InterfaceType::new(
                declared.name,
                declared
                    .arguments
                    .iter()
                    .map(|argument| self.substitute_type_id(*argument, &bindings))
                    .collect(),
            );
            let requirements = self.with_nominal_identity(|checker| {
                checker.canonical_interface_requirements(&interface, &mut Vec::new())
            });
            result.valid &= requirements.valid;
            let path = ClassConformancePath {
                interface,
                origin: edge.origin,
                declaring_class: class.name.clone(),
                direct: true,
                conditional: edge.conditional,
                obligations: edge.obligations,
            };
            result.paths.push(path.clone());
            result
                .paths
                .extend(
                    requirements
                        .ancestors
                        .into_iter()
                        .map(|interface| ClassConformancePath {
                            interface,
                            direct: false,
                            ..path.clone()
                        }),
                );
        }
        visiting.remove(&class.name);
        self.declared_class_conformance_cache
            .insert(class.clone(), result.clone());
        result
    }

    fn report_class_conformance_overlap(
        &mut self,
        interface: &InterfaceType<TypeId>,
        origin: Span,
        previous: Span,
        repeated: bool,
    ) {
        let (message, title) = if repeated {
            let ty = self.types.intern(TypeKind::Interface(interface.clone()));
            (
                format!(
                    "`{}` repeats an existing conformance",
                    self.types.display(ty)
                ),
                "Redundant Or Conflicting Conformance",
            )
        } else {
            (
                format!(
                    "different specializations of `{}` cannot be implemented together",
                    interface.name
                ),
                "Conflicting Interface Specializations",
            )
        };
        if self.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "E0752" && diagnostic.span == origin && diagnostic.message == message
        }) {
            return;
        }
        let mut diagnostic = Diagnostic::new("E0752", message, origin)
            .with_title(title)
            .with_related(previous, "the existing conformance originates here");
        let mut related = HashSet::from([origin, previous]);
        for obligation in [origin, previous]
            .into_iter()
            .filter_map(|origin| self.contracts.conditions.get(&origin))
            .flat_map(|condition| &condition.obligations)
        {
            if related.insert(obligation.interface_span) {
                diagnostic = diagnostic.with_related(
                    obligation.interface_span,
                    "this capability condition contributes to the conformance",
                );
            }
        }
        self.diagnostics.push(diagnostic);
    }
}
