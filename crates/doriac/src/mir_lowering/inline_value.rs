use super::*;

pub(super) fn lower_concurrency_intrinsic(
    intrinsic: crate::compiler_known_concurrency::ConcurrencyFunction,
    span: Span,
    params: &[mir::LocalId],
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) {
    use crate::compiler_known_concurrency::ConcurrencyFunction;
    match intrinsic {
        ConcurrencyFunction::SourceConstructor => {
            context.cleanup_scopes_from(0);
            context.terminate_current(mir::Terminator::ReturnVoid);
        }
        ConcurrencyFunction::SourceCancel => {
            let error = context.declare_checked_call_slot(mir::Type::ERROR, true);
            let success = context.create_block();
            let failure = context.create_block();
            context.terminate_current(mir::Terminator::CheckedCancellationRequest {
                source: params[0],
                result: None,
                error,
                success,
                failure,
                span,
            });
            context.current_block = Some(failure);
            context.route_checked_error(error);
            context.current_block = Some(success);
            context.cleanup_scopes_from(0);
            context.terminate_current(mir::Terminator::ReturnVoid);
        }
        ConcurrencyFunction::SourceToken | ConcurrencyFunction::SourceRequested => {
            let value = if intrinsic == ConcurrencyFunction::SourceToken {
                let mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Inline(ty))) =
                    return_type
                else {
                    unreachable!("source token getter returns a checked inline token");
                };
                mir::ValueExpression::Inline(mir::InlineValueExpression::CancellationToken {
                    ty,
                    source: params[0],
                    span,
                })
            } else {
                mir::ValueExpression::Bool(mir::BoolExpression::CancellationRequested {
                    source: params[0],
                })
            };
            context.cleanup_scopes_from(0);
            context.terminate_current(mir::Terminator::Return(mir::Rvalue::Value(value)));
        }
    }
}

pub(super) fn lower_intrinsic(
    intrinsic: crate::compiler_known_time::TimeFunction,
    span: Span,
    params: &[mir::LocalId],
    context: &mut LoweringContext,
) {
    use crate::compiler_known_time::TimeFunction;
    let value = match intrinsic {
        TimeFunction::DeadlineFromNow => {
            let ty = mir::Type::Scalar(mir::ScalarType::Inline(
                crate::inline_value::InlineValueType::Deadline,
            ));
            let result = context.declare_checked_call_slot(ty, false);
            let error = context.declare_checked_call_slot(mir::Type::ERROR, true);
            let success = context.create_block();
            let failure = context.create_block();
            context.terminate_current(mir::Terminator::CheckedDeadlineFromNow {
                duration: params[0],
                result: Some(result),
                error,
                success,
                failure,
                span,
            });
            context.current_block = Some(failure);
            context.route_checked_error(error);
            context.current_block = Some(success);
            context.cleanup_scopes_from(0);
            context.terminate_current(mir::Terminator::Return(local_rvalue(result, ty, false)));
            return;
        }
        TimeFunction::DurationFromNanoseconds => {
            mir::ValueExpression::Inline(mir::InlineValueExpression::DurationFromNanoseconds(
                Box::new(mir::IntegerExpression::use_operand(
                    IntegerType::Int64,
                    mir::Operand::Local(params[0]),
                )),
            ))
        }
        TimeFunction::DurationNanoseconds => {
            mir::ValueExpression::Integer(mir::IntegerExpression::DurationNanoseconds(Box::new(
                mir::InlineValueExpression::Use {
                    ty: crate::inline_value::InlineValueType::Duration,
                    operand: mir::Operand::Local(params[0]),
                    mode: mir::InlineValueUseMode::Borrow,
                },
            )))
        }
    };
    context.cleanup_scopes_from(0);
    context.terminate_current(mir::Terminator::Return(mir::Rvalue::Value(value)));
}

pub(super) fn lower_expression(
    expr: &hir::Expr,
    ty: crate::inline_value::InlineValueType,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::InlineValueExpression> {
    lower_expression_with_ownership(expr, ty, false, context)
}

pub(super) fn lower_expression_with_ownership(
    expr: &hir::Expr,
    ty: crate::inline_value::InlineValueType,
    owned: bool,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::InlineValueExpression> {
    let scalar = mir::ScalarType::Inline(ty);
    let expected = mir::Type::Scalar(scalar);
    if let Some((local, actual)) = materialize_callable_scalar_result(expr, context)? {
        if actual != scalar {
            return Err(vec![unsupported(
                expr.span(),
                "callable returns another inline value type",
            )]);
        }
        return Ok(mir::InlineValueExpression::Use {
            ty,
            operand: mir::Operand::Local(local),
            mode: use_mode(ty, owned),
        });
    }
    if let Some(mir::Rvalue::Value(mir::ValueExpression::Inline(value))) =
        materialize_checked_rvalue(expr, expected, owned, context)?
    {
        return Ok(value);
    }
    if let Some((collection, index, actual)) = lower_list_remove_at(expr, context)? {
        if actual != expected {
            return Err(vec![unsupported(
                expr.span(),
                "List::removeAt result has another inline value type",
            )]);
        }
        return Ok(mir::InlineValueExpression::Use {
            ty,
            operand: mir::Operand::CollectionIndex {
                index_span: expr.span(),
                collection,
                index: Box::new(index),
                positional: false,
                remove: true,
            },
            mode: if ty.requires_owned_cleanup() {
                mir::InlineValueUseMode::TransferOwnedCopy
            } else {
                mir::InlineValueUseMode::Borrow
            },
        });
    }
    let operand = match expr {
        hir::Expr::Grouped { expr, .. } => {
            return lower_expression_with_ownership(expr, ty, owned, context)
        }
        hir::Expr::Binary {
            left,
            op: hir::BinaryOp::Coalesce,
            right,
            ..
        } => {
            return match context.coalesce_selection(left) {
                CoalesceSelection::Left => {
                    lower_expression_with_ownership(left, ty, owned, context)
                }
                CoalesceSelection::Right => {
                    lower_expression_with_ownership(right, ty, owned, context)
                }
                CoalesceSelection::Dynamic => Ok(mir::InlineValueExpression::Coalesce {
                    ty,
                    left: Box::new(lower_nullable_inline_expression(left, ty, owned, context)?),
                    right: Box::new(lower_expression_with_ownership(right, ty, owned, context)?),
                }),
            };
        }
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                actual if actual == expected => mir::Operand::Local(local),
                mir::Type::NullableScalar(actual) if actual == scalar => {
                    mir::Operand::NullablePayload(local)
                }
                mir::Type::Mixed | mir::Type::NullableMixed
                    if context
                        .exact_mixed_local(expr)
                        .is_some_and(|(_, actual)| actual == expected) =>
                {
                    mir::Operand::MixedPayload {
                        mixed: local,
                        tag: mir::MixedTag::Inline(ty),
                    }
                }
                _ => {
                    return Err(vec![unsupported(
                        *span,
                        "inline value local has another type",
                    )]);
                }
            }
        }
        hir::Expr::PropertyAccess { .. } => {
            let (object, property) = lower_property_operand(expr, expected, context)?;
            mir::Operand::Property { object, property }
        }
        hir::Expr::Index {
            collection, index, ..
        } => {
            let (collection, index) =
                lower_collection_index_operand(collection, index, expected, context)?;
            mir::Operand::CollectionIndex {
                index_span: expr.span(),
                collection,
                index: Box::new(index),
                positional: false,
                remove: false,
            }
        }
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
            ..
        } => {
            let (id, actual) = context.static_property(class_name, member, *span)?;
            if actual != expected {
                return Err(vec![unsupported(
                    *span,
                    "static property has another inline value type",
                )]);
            }
            mir::Operand::Static(id)
        }
        hir::Expr::FunctionCall { name, args, span } => {
            let signature = context.lookup_function(name, *span)?;
            if signature.return_type != mir::ReturnType::Value(expected) {
                return Err(vec![unsupported(
                    *span,
                    "call returns another inline value type",
                )]);
            }
            return Ok(mir::InlineValueExpression::Call {
                ty,
                function: signature.id,
                args: lower_call_args(name, args, signature, *span, context)?,
                span: *span,
            });
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            ..
        } => {
            let (signature, args) =
                lower_instance_method_call(object, method, args, *span, context)?;
            return call(&signature, args, ty, *span);
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
            ..
        } => {
            let (signature, args) =
                lower_static_method_call(class_name, method, args, *span, context)?;
            return call(&signature, args, ty, *span);
        }
        _ => {
            return Err(vec![unsupported(
                expr.span(),
                "this inline value expression is not supported by native compilation",
            )]);
        }
    };
    Ok(mir::InlineValueExpression::Use {
        ty,
        operand,
        mode: use_mode(ty, owned),
    })
}

fn use_mode(ty: crate::inline_value::InlineValueType, owned: bool) -> mir::InlineValueUseMode {
    if owned && ty.requires_owned_cleanup() {
        mir::InlineValueUseMode::Copy
    } else {
        mir::InlineValueUseMode::Borrow
    }
}

fn call(
    signature: &FunctionSignature,
    args: Vec<mir::Rvalue>,
    ty: crate::inline_value::InlineValueType,
    span: Span,
) -> DiagnosticResult<mir::InlineValueExpression> {
    if signature.return_type
        != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Inline(ty)))
    {
        return Err(vec![unsupported(
            span,
            "call returns another inline value type",
        )]);
    }
    Ok(mir::InlineValueExpression::Call {
        ty,
        function: signature.id,
        args,
        span,
    })
}
