//! Caller-scoped editor availability consumes the same nominal proofs as calls.

use super::composition::{ClassMemberAvailability, ClassMemberSurface, MemberAvailability};
use super::nominal_proof::{Context, Failure, Query, Step};
use super::*;

pub(super) struct ClassMemberLookup {
    pub(super) receiver: ClassType<TypeId>,
    caller: Context,
}

impl Checker<'_> {
    pub(super) fn class_member_receiver(&self, receiver: TypeId) -> Option<ClassType<TypeId>> {
        match self.types.kind(receiver) {
            TypeKind::Class(class) => Some(class.clone()),
            TypeKind::Nullable(inner) => self.class_member_receiver(*inner),
            TypeKind::SharedHandle(kind, inner) if Self::shared_handle_forwards(*kind) => {
                self.class_member_receiver(*inner)
            }
            _ => None,
        }
    }

    pub(super) fn record_class_member_lookup(&mut self, receiver: ClassType<TypeId>, span: Span) {
        if self.contracts.callable_conditions.is_empty() {
            return;
        }
        let caller = self.nominal_proof_context().frozen_premises();
        self.class_member_lookups
            .insert(span, ClassMemberLookup { receiver, caller });
    }

    pub(super) fn collect_class_member_availability(
        &mut self,
        surfaces: &[ClassMemberSurface],
        templates: &HashMap<(ClassType<ResolvedType>, String), Vec<MethodInfo>>,
    ) -> HashMap<Span, ClassMemberAvailability> {
        let mut lookups = std::mem::take(&mut self.class_member_lookups)
            .into_iter()
            .collect::<Vec<_>>();
        lookups.sort_by_key(|(span, _)| *span);
        lookups
            .into_iter()
            .map(|(span, lookup)| {
                let receiver = ClassType::new(
                    lookup.receiver.name,
                    lookup
                        .receiver
                        .arguments
                        .iter()
                        .map(|argument| self.types.resolved(*argument))
                        .collect(),
                );
                let mut members = HashMap::new();
                if let Some(surface) = surfaces
                    .iter()
                    .find(|surface| surface.receiver == receiver && surface.valid)
                {
                    for member in &surface.members {
                        let availability = templates
                            .get(&(receiver.clone(), member.name.clone()))
                            .map_or(MemberAvailability::Proved, |methods| {
                                self.member_methods_availability(methods, &lookup.caller)
                            });
                        members.insert(member.name.clone(), availability);
                    }
                }
                (span, ClassMemberAvailability { receiver, members })
            })
            .collect()
    }

    pub(super) fn member_methods_availability(
        &mut self,
        methods: &[MethodInfo],
        caller: &Context,
    ) -> MemberAvailability {
        let mut result = MemberAvailability::Proved;
        for method in methods {
            match self
                .with_nominal_identity(|checker| checker.member_method_availability(method, caller))
            {
                MemberAvailability::Unproved => return MemberAvailability::Unproved,
                MemberAvailability::RequiresTypeArguments => {
                    result = MemberAvailability::RequiresTypeArguments
                }
                MemberAvailability::Proved => {}
            }
        }
        result
    }

    fn member_method_availability(
        &mut self,
        method: &MethodInfo,
        caller: &Context,
    ) -> MemberAvailability {
        let condition = match self.callable_condition(method) {
            Ok(None) => return MemberAvailability::Proved,
            Ok(Some(condition)) => condition,
            Err(()) => return MemberAvailability::Unproved,
        };
        let mut context = caller.frozen_premises();
        let mut bindings = self.normalized_method_bindings(method);
        for parameter in &condition.parameters {
            bindings.entry(parameter.name.clone()).or_insert_with(|| {
                self.types
                    .intern(TypeKind::TypeParameter(parameter.name.clone()))
            });
        }
        let mut unresolved = HashMap::new();
        for (index, bounds) in self
            .normalized_method_constraints(method)
            .into_iter()
            .enumerate()
        {
            let name = format!("#method{index}");
            let subject = self.types.intern(TypeKind::TypeParameter(name.clone()));
            unresolved.insert(name, self.types.intern(TypeKind::Unknown));
            for (name, arguments) in bounds {
                context.assume(Step {
                    query: Query {
                        subject,
                        interface: crate::types::InterfaceType::new(name, arguments),
                    },
                    origin: Some(method.declaration),
                });
            }
        }
        let Ok(steps) = self.condition_queries(&condition, &bindings) else {
            return MemberAvailability::Unproved;
        };
        let mut result = MemberAvailability::Proved;
        for step in steps {
            let needs_arguments =
                self.substitute_type_id(step.query.subject, &unresolved) != step.query.subject
                    || step.query.interface.arguments.iter().any(|argument| {
                        self.substitute_type_id(*argument, &unresolved) != *argument
                    });
            match self.prove_nominal(step.query, &mut context, step.origin, &[]) {
                Ok(()) => {}
                Err(Failure::Unsatisfied(_)) if needs_arguments => {
                    result = MemberAvailability::RequiresTypeArguments
                }
                Err(_) => return MemberAvailability::Unproved,
            }
        }
        result
    }
}
