//! Concrete carrier-return contracts share the ordinary source CFG inference.

use super::*;
use crate::ownership::ReturnBorrowLookup;

pub(crate) type Facts = HashMap<specialized_ownership::Instance, Option<ReturnBorrow>>;

pub(crate) fn lookup(
    facts: &Facts,
    declaration: Span,
    substitutions: &HashMap<String, ResolvedType>,
) -> Option<Option<ReturnBorrow>> {
    facts
        .get(&specialized_ownership::Instance::new(
            declaration,
            substitutions,
        ))
        .copied()
}

impl Checker<'_> {
    pub(super) fn authored_call_return_borrow(
        &mut self,
        call: &Expr,
    ) -> Option<ReturnBorrowLookup> {
        let CallableTarget::ConstrainedMethod {
            receiver: ResolvedType::TraitSelf(_),
            requirement,
            ..
        } = self.call_targets.get(&call.span())?
        else {
            return None;
        };
        let signature = self.function_signatures.get(requirement)?.clone();
        let bindings = self
            .pending_generic_calls
            .get(&call.span())
            .filter(|pending| pending.declaration == signature.declaration)
            .map(|pending| pending.inference.bindings.clone())
            .unwrap_or_default();
        // Read the live authored contract without rechecking arguments or
        // admitting dependencies during source-only inference.
        let mut params = signature.params;
        for parameter in &mut params {
            parameter.ty = self.substitute_type_id(parameter.ty, &bindings);
        }
        let result = self.substitute_type_id(signature.return_ty, &bindings);
        let borrow = self
            .return_borrow_for_instance(signature.declaration, &bindings)
            .unwrap_or(signature.return_borrow);
        let borrow = self.specialize_return_borrow(borrow, &params, result);
        Some(Self::bind_call_return_borrow(call, borrow, &params))
    }

    pub(super) fn return_borrow_for_instance(
        &self,
        declaration: Span,
        substitutions: &HashMap<String, TypeId>,
    ) -> Option<Option<ReturnBorrow>> {
        let substitutions = substitutions
            .iter()
            .map(|(name, ty)| (name.clone(), self.types.resolved(*ty)))
            .collect();
        lookup(
            &self.specialized_return_borrow_seed,
            declaration,
            &substitutions,
        )
    }

    pub(super) fn owned_copy_interface_erasure(
        &mut self,
        span: Span,
        substitute: impl Fn(&ResolvedType) -> ResolvedType,
    ) -> bool {
        let Some(destination) = self.contextual_expression_types.get(&span) else {
            return false;
        };
        let destination = substitute(&self.types.resolved(*destination));
        let inner = match &destination {
            ResolvedType::Nullable(inner) => inner.as_ref(),
            ty => ty,
        };
        if !matches!(inner, ResolvedType::Interface(_)) {
            return false;
        }
        let Some(source) = self.expression_types.get(&span) else {
            return false;
        };
        let source = substitute(source);
        if matches!(source, ResolvedType::Null | ResolvedType::Unsupported)
            || crate::ownership::resolved_type_requires_conservative_move(&source)
        {
            return false;
        }
        let source = self.types.intern_resolved(&source);
        !self.type_is_move_type(source)
    }

    /// Read checked selection and reuse the declaring class's existing substitution.
    /// This query never rechecks an argument or overwrites a call-site plan.
    pub(super) fn return_call_instance(
        &mut self,
        call: &Expr,
        substitute: impl Fn(&ResolvedType) -> ResolvedType,
    ) -> Option<(specialized_ownership::Instance, Span)> {
        let getter = self
            .property_accessor_calls
            .get(&call.span())
            .and_then(|calls| calls.getter.as_ref())
            .cloned();
        let target = getter
            .as_ref()
            .map(|getter| &getter.target)
            .or_else(|| self.call_targets.get(&call.span()))?
            .specialize(&substitute)?;
        let (declaration, mut bindings) = match target {
            CallableTarget::Function { name } => {
                (self.functions.get(&name)?.declaration, HashMap::new())
            }
            CallableTarget::Method {
                class_type,
                method_name,
                ..
            } => {
                let class = ClassType::new(
                    class_type.name,
                    class_type
                        .arguments
                        .iter()
                        .map(|ty| self.types.intern_resolved(ty))
                        .collect(),
                );
                if let Some(getter) = getter {
                    let method = if getter.receiver_mode.is_none() {
                        let (owner, StaticMemberInfo::Property(property)) =
                            self.lookup_static_member(&class, &method_name)?
                        else {
                            return None;
                        };
                        self.static_property_accessor_method(
                            &property,
                            &owner,
                            crate::ast::PropertyHookKind::Get,
                        )?
                    } else {
                        let (owner, property) =
                            self.lookup_instance_property(&class, &method_name)?;
                        self.property_accessor_method(
                            &property,
                            &owner,
                            crate::ast::PropertyHookKind::Get,
                        )?
                    };
                    (method.declaration, method.enclosing_type_bindings)
                } else {
                    let (_, method) = self.lookup_instance_method(&class, &method_name)?;
                    (method.declaration, method.enclosing_type_bindings)
                }
            }
            CallableTarget::InterfaceMethod { .. } | CallableTarget::ConstrainedMethod { .. } => {
                return None
            }
        };
        let signature = self.function_signatures.get(&declaration)?;
        if !signature.type_params.is_empty() {
            let arguments = &self
                .generic_call_specializations
                .get(&call.span())?
                .arguments;
            if arguments.len() != signature.type_params.len() {
                return None;
            }
            for (parameter, GenericArgument::Type(ty)) in
                signature.type_params.iter().zip(arguments)
            {
                let ty = substitute(ty);
                bindings.insert(parameter.name.clone(), self.types.intern_resolved(&ty));
            }
        }
        let bindings = bindings
            .into_iter()
            .map(|(name, ty)| (name, self.types.resolved(ty)))
            .collect();
        Some((
            specialized_ownership::Instance::new(declaration, &bindings),
            declaration,
        ))
    }

    /// A requirement returning `T` promises a region loan only in the
    /// instances where `T` is a region, so an instance reads that contract from
    /// the call's substituted type.
    fn requirement_return_for_instance(
        &self,
        call: &Expr,
        substitute: impl Fn(&ResolvedType) -> ResolvedType,
    ) -> Option<ReturnBorrow> {
        let target = self
            .property_accessor_calls
            .get(&call.span())
            .and_then(|calls| calls.getter.as_ref())
            .map(|getter| &getter.target)
            .or_else(|| self.call_targets.get(&call.span()))?;
        let (CallableTarget::InterfaceMethod { requirement, .. }
        | CallableTarget::ConstrainedMethod { requirement, .. }) = target
        else {
            return None;
        };
        // A generic method's own `T` comes from its arguments, not the contract.
        let signature = self.function_signatures.get(requirement)?;
        if !signature.type_params.is_empty() && self.type_is_symbolic(signature.return_ty) {
            return None;
        }
        let receiver = self.interface_requirement_receiver(*requirement)?;
        let result = substitute(self.expression_types.get(&call.span())?);
        crate::ownership::requirement_return_borrow(
            crate::types::ByteRegionKind::from_resolved(&result),
            Some(receiver),
        )
    }

    pub(super) fn infer_specialized_return_borrows(&mut self) -> Facts {
        let program = self.program;
        let backing_fields = self.property_backing_fields();
        collect_ordered_class_semantics(program, self, &backing_fields);
        let mut functions = HashMap::new();
        for item in &self.callable_body_program().items {
            match item {
                Item::Function(function) => {
                    functions.insert(function.span, (std::borrow::Cow::Borrowed(function), None));
                }
                Item::Class(class) => {
                    for member in &class.members {
                        for function in crate::property_hooks::member_callables(
                            member,
                            crate::property_hooks::PropertyHookContext::Class,
                        ) {
                            functions.insert(function.span, (function, Some(class.name.clone())));
                        }
                    }
                }
                Item::Trait(declaration) => {
                    for member in &declaration.members {
                        for function in crate::property_hooks::member_callables(
                            member,
                            crate::property_hooks::PropertyHookContext::Trait,
                        ) {
                            functions
                                .insert(function.span, (function, Some(declaration.name.clone())));
                        }
                    }
                }
                _ => {}
            }
        }
        let mut plans = Vec::new();
        let mut facts = HashMap::new();
        for instance in self.callable_instances.clone() {
            let substitutions = instance
                .bindings
                .iter()
                .map(|(name, ty)| (name.clone(), self.types.resolved(*ty)))
                .collect::<HashMap<_, _>>();
            let body = callable_contracts::BodyInstance::new(
                self.callable_body_program,
                instance.declaration,
                substitutions.clone(),
                &self.composition,
            );
            if substitutions.is_empty() && body.declaration == instance.declaration {
                continue;
            }
            let Some(signature) = self.function_signatures.get(&body.declaration).cloned() else {
                continue;
            };
            let Some((function, class)) = functions.get(&body.declaration) else {
                continue;
            };
            let Some(graph) = self.return_borrow_graph(function) else {
                continue;
            };
            let concrete_parameters = signature
                .params
                .iter()
                .map(|parameter| {
                    let mut parameter = parameter.clone();
                    let ty = body.substitute(&self.types.resolved(parameter.ty));
                    parameter.ty = self.types.intern_resolved(&ty);
                    parameter
                })
                .collect::<Vec<_>>();
            let result = body.substitute(&self.types.resolved(signature.return_ty));
            let region_result = crate::types::ByteRegionKind::from_resolved(&result).is_some();
            let result = self.types.intern_resolved(&result);
            if signature.return_borrow.is_none() && !region_result {
                continue;
            }
            if signature
                .return_borrow
                .is_some_and(|borrow| borrow.kind == ReturnBorrowKind::Retained)
            {
                let borrow = self.specialize_return_borrow(
                    signature.return_borrow,
                    &concrete_parameters,
                    result,
                );
                facts.insert(
                    specialized_ownership::Instance::new(instance.declaration, &substitutions),
                    borrow,
                );
                continue;
            }
            let mut copy = false;
            for ty in body.substitutions.values() {
                let id = self.types.intern_resolved(ty);
                copy |= !self.type_is_move_type(id) && !self.type_is_symbolic(id);
            }
            // A concrete region carries storage provenance even when the
            // symbolic taking signature had no result-carrier loan.
            if !copy && !region_result {
                continue;
            }
            let mut scopes = ScopeStack::new();
            for parameter in &signature.params {
                let _ = scopes.declare(
                    parameter.name.clone(),
                    Binding::unresolved(parameter.writable, parameter.ty, parameter.ty, None, None),
                );
            }
            let context = class.as_ref().map(|class_name| MethodContext {
                class_name: class_name.clone(),
                receiver_access: if function.is_static {
                    ReceiverAccess::Unavailable
                } else if function.has_writable_receiver() {
                    ReceiverAccess::Writable
                } else {
                    ReceiverAccess::Readonly
                },
            });
            let base_calls = self.checked_return_borrow_calls(function, &scopes, context.as_ref());
            let parameters = concrete_parameters
                .iter()
                .map(|parameter| {
                    (
                        self.parameter_can_supply_return_borrow(parameter.ty, parameter.take),
                        self.type_is_symbolic(parameter.ty),
                    )
                })
                .collect::<Vec<_>>();
            let borrowed_parameters =
                crate::ownership::return_borrow_parameters(parameters.into_iter());
            plans.push((
                instance.declaration,
                function.clone(),
                substitutions,
                body,
                graph,
                base_calls,
                borrowed_parameters,
                self.type_is_move_type(result) || region_result,
                concrete_parameters,
                result,
            ));
        }
        for _ in 0..=plans.len() {
            let before = facts.clone();
            for (
                declaration,
                function,
                substitutions,
                body,
                graph,
                base_calls,
                parameters,
                move_result,
                concrete_parameters,
                result,
            ) in &plans
            {
                let mut calls = base_calls.clone();
                if let Some(block) = function.body.as_block() {
                    crate::ast::visit::block(block, &mut |expression| {
                        if self.owned_copy_interface_erasure(expression.span(), |ty| {
                            body.substitute(ty)
                        }) {
                            calls.insert(expression.span(), ReturnBorrowLookup::OwnedValue);
                        } else if let Some(borrow) = self
                            .requirement_return_for_instance(expression, |ty| body.substitute(ty))
                        {
                            calls.insert(
                                expression.span(),
                                ReturnBorrowLookup::Resolved(Some(borrow)),
                            );
                        } else if let Some((instance, callee)) =
                            self.return_call_instance(expression, |ty| body.substitute(ty))
                        {
                            let borrow = facts.get(&instance).copied().or_else(|| {
                                self.specialized_return_borrow_seed.get(&instance).copied()
                            });
                            if let Some(borrow) = borrow {
                                if let Some(signature) = self.function_signatures.get(&callee) {
                                    calls.insert(
                                        expression.span(),
                                        Self::bind_call_return_borrow(
                                            expression,
                                            borrow,
                                            &signature.params,
                                        ),
                                    );
                                }
                            }
                        }
                    });
                }
                let borrow = if *move_result {
                    crate::ownership::function_return_borrow_with_bindings(
                        function,
                        parameters.clone(),
                        &mut |expression| {
                            calls
                                .get(&expression.span())
                                .copied()
                                .unwrap_or(ReturnBorrowLookup::Unresolved)
                        },
                        &self.binding_resolution,
                        graph,
                    )
                } else {
                    None
                };
                let borrow = self.specialize_return_borrow(borrow, concrete_parameters, *result);
                facts.insert(
                    specialized_ownership::Instance::new(*declaration, substitutions),
                    borrow,
                );
            }
            if facts == before {
                break;
            }
        }
        facts
    }
}
