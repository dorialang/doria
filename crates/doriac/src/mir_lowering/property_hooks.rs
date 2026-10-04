//! Property operations reuse ordinary callable dispatch, result ownership, and
//! checked-error cleanup. Direct backing accesses have no accessor call fact.

use super::*;

enum AccessorCallPlan {
    Direct(FunctionSignature),
    Interface(mir::InterfaceTypeId, interface::CallPlan),
}

impl AccessorCallPlan {
    fn resolve(
        call: &crate::semantics::PropertyAccessorCallSemanticInfo,
        property: &str,
        kind: crate::ast::PropertyHookKind,
        span: Span,
        context: &LoweringContext<'_>,
    ) -> DiagnosticResult<Self> {
        let target = call
            .target
            .specialize(|ty| substitute_resolved_type(ty, &context.type_substitutions))
            .ok_or_else(|| {
                vec![Diagnostic::new(
                    "I2401",
                    "checked accessor has no concrete callable target",
                    span,
                )]
            })?;
        if let Some((interface, plan)) =
            interface::call_plan_target(span, Some(target.clone()), context)?
        {
            return Ok(Self::Interface(interface, plan));
        }
        let CallableTarget::Method { class_type, .. } = target else {
            return Err(vec![Diagnostic::new(
                "I2401",
                "checked accessor did not specialize to an instance callable",
                span,
            )]);
        };
        let class = context.class_id_for_type(&class_type).ok_or_else(|| {
            vec![Diagnostic::new(
                "I2401",
                "checked accessor owner has no native class",
                span,
            )]
        })?;
        let name = crate::property_hooks::accessor_name(property, kind);
        Ok(Self::Direct(context.lookup_method(class, &name, span)?))
    }

    fn signature(&self) -> &FunctionSignature {
        match self {
            Self::Direct(signature) => signature,
            Self::Interface(_, plan) => &plan.signature,
        }
    }

    fn call(
        self,
        receiver: (mir::LocalId, mir::Type),
        args: Vec<mir::Rvalue>,
        span: Span,
        context: &mut LoweringContext<'_>,
    ) -> DiagnosticResult<Option<(mir::LocalId, mir::Type, bool)>> {
        match self {
            Self::Direct(signature) => {
                let expected = mir::Type::Class(signature.method_class.expect("instance accessor"));
                let mut lowered = vec![convert_call_result(
                    receiver.0, receiver.1, expected, false, span, context,
                )?];
                lowered.extend(args);
                materialize_signature_call(signature, lowered, span, false, context)
            }
            Self::Interface(interface, plan) => {
                let receiver = convert_call_result(
                    receiver.0,
                    receiver.1,
                    mir::Type::Interface(interface),
                    false,
                    span,
                    context,
                )?;
                interface::materialize_lowered_call(
                    receiver,
                    args,
                    (interface, plan),
                    span,
                    false,
                    context,
                )
            }
        }
    }
}

pub(super) fn materialize_getter(
    expression: &hir::Expr,
    consume_result: bool,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<Option<(mir::LocalId, mir::Type, bool)>> {
    let hir::Expr::PropertyAccess {
        object,
        property,
        null_safe,
        span,
        ..
    } = expression
    else {
        return Ok(None);
    };
    let call = context.semantic_info.property_accessor_calls[span]
        .getter
        .as_ref()
        .expect("checked getter call");
    let plan = AccessorCallPlan::resolve(
        call,
        property,
        crate::ast::PropertyHookKind::Get,
        *span,
        context,
    )?;
    let signature = match plan {
        AccessorCallPlan::Direct(signature) => signature,
        AccessorCallPlan::Interface(interface, plan) => {
            return interface::materialize_call(
                expression,
                (interface, plan),
                consume_result,
                context,
            );
        }
    };
    let method = crate::property_hooks::accessor_name(property, crate::ast::PropertyHookKind::Get);
    if *null_safe {
        let ty = context.expression_type(expression)?;
        return materialize_null_safe_signature_call(
            object,
            &method,
            signature,
            &[],
            *span,
            Some((ty, consume_result)),
            context,
        );
    }
    let (signature, arguments) =
        lower_instance_call_with_signature(object, &method, &[], *span, signature, context)?;
    materialize_signature_call(signature, arguments, *span, consume_result, context)
}

struct SetterPlace {
    receiver: (mir::LocalId, mir::Type),
    getter: Option<AccessorCallPlan>,
    setter: AccessorCallPlan,
    span: Span,
}

impl SetterPlace {
    fn lower(
        target: &hir::Expr,
        context: &mut LoweringContext<'_>,
    ) -> DiagnosticResult<Option<Self>> {
        let target = unparenthesized_place(target);
        let Some(calls) = context
            .semantic_info
            .property_accessor_calls
            .get(&target.span())
        else {
            return Ok(None);
        };
        let Some(setter) = &calls.setter else {
            return Ok(None);
        };
        let hir::Expr::PropertyAccess {
            object,
            property,
            span,
            null_safe: false,
            ..
        } = target
        else {
            return Err(vec![Diagnostic::new(
                "I2401",
                "checked setter requires a non-null property receiver",
                target.span(),
            )]);
        };
        let mut setter = AccessorCallPlan::resolve(
            setter,
            property,
            crate::ast::PropertyHookKind::Set,
            *span,
            context,
        )?;
        let mut getter = calls
            .getter
            .as_ref()
            .map(|call| {
                AccessorCallPlan::resolve(
                    call,
                    property,
                    crate::ast::PropertyHookKind::Get,
                    *span,
                    context,
                )
            })
            .transpose()?;
        // Retain the actual receiver once, before either accessor or the RHS.
        // Getter and setter may be declared at different inheritance depths.
        materialize_nested_collection_places(object, true, context)?;
        let receiver = match &setter {
            AccessorCallPlan::Direct(_) => {
                let class = inferred_class_type(object, context).ok_or_else(|| {
                    vec![Diagnostic::new(
                        "I2401",
                        "checked setter receiver has no native class",
                        object.span(),
                    )]
                })?;
                mir::Rvalue::Class(lower_class_call_receiver(object, class, context)?)
            }
            AccessorCallPlan::Interface(interface, _) => mir::Rvalue::interface(
                *interface,
                lower_interface_expression(object, *interface, false, context)?,
            ),
        };
        let receiver_type = receiver.ty();
        let local = materialize_call_receiver(receiver, true, context);
        if context.lifecycle_phase
            && matches!(unparenthesized_place(object), hir::Expr::This { .. })
        {
            for plan in std::iter::once(&mut setter).chain(getter.iter_mut()) {
                if let AccessorCallPlan::Direct(signature) = plan {
                    if let Some(direct) = signature.direct_id {
                        signature.id = direct;
                    }
                }
            }
        }
        Ok(Some(Self {
            receiver: (local, receiver_type),
            getter,
            setter,
            span: *span,
        }))
    }

    fn read(
        &mut self,
        context: &mut LoweringContext<'_>,
    ) -> DiagnosticResult<(mir::Operand, mir::ScalarType)> {
        let getter = self
            .getter
            .take()
            .expect("checked read-modify-write has a getter");
        let Some((local, mir::Type::Scalar(ty), _)) =
            getter.call(self.receiver, Vec::new(), self.span, context)?
        else {
            return Err(vec![Diagnostic::new(
                "I2401",
                "checked property update requires a scalar getter",
                self.span,
            )]);
        };
        Ok((mir::Operand::Local(local), ty))
    }

    fn write(self, value: mir::Rvalue, context: &mut LoweringContext<'_>) -> DiagnosticResult<()> {
        self.setter
            .call(self.receiver, vec![value], self.span, context)?;
        Ok(())
    }
}

pub(super) fn lower_assignment(
    assignment: &hir::Assignment,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<bool> {
    let Some(mut place) = SetterPlace::lower(&assignment.target, context)? else {
        return Ok(false);
    };
    let value = if assignment.op == hir::AssignOp::Assign {
        let signature = place.setter.signature();
        materialize_nested_collection_places(&assignment.value, false, context)?;
        lower_call_argument(
            &assignment.value,
            signature.parameter_types[0],
            signature.parameter_modes[0],
            context,
        )?
    } else {
        let (operand, ty) = place.read(context)?;
        materialize_nested_collection_places(&assignment.value, false, context)?;
        mir::Rvalue::Value(lower_compound_value(
            operand,
            ty,
            &assignment.op,
            &assignment.value,
            assignment.span,
            context,
        )?)
    };
    place.write(value, context)?;
    Ok(true)
}

pub(super) fn lower_increment(
    increment: &hir::IncrementStmt,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<bool> {
    let Some(mut place) = SetterPlace::lower(&increment.target, context)? else {
        return Ok(false);
    };
    let (operand, ty) = place.read(context)?;
    let value = lower_increment_value(operand, ty, &increment.op, increment.span)?;
    place.write(mir::Rvalue::Value(value), context)?;
    Ok(true)
}
