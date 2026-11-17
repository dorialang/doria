//! Raw pointer operations (Decision 0148 section 4).
//!
//! A pointer is the one-word inline value `InlineValueType::Pointer`; its
//! pointee type and access are checking facts that MIR does not carry. Each
//! operation that reads or changes memory or an address is one
//! `Statement::Pointer` over a pointer local, so both native backends lower
//! one checked operation per statement and the interpreter can find every
//! one before it runs a program. Conversions that keep the address (`cast`,
//! `Ptr::from`, `toFunction`) are the value itself.

use super::*;
use crate::foreign::{ForeignType, PointerAccess};
use crate::pointer_intrinsics::{self, PointerOperation};

const POINTER: mir::Type = mir::Type::Scalar(mir::ScalarType::Inline(
    crate::inline_value::InlineValueType::Pointer,
));

/// The checked pointer type of `expr`: its access and pointee.
fn pointer_type(
    expr: &hir::Expr,
    context: &LoweringContext,
) -> Option<(PointerAccess, ResolvedType)> {
    match context.resolved_expression_type(expr).ok()? {
        ResolvedType::Foreign(foreign) => match *foreign {
            ForeignType::Pointer { access, pointee } => Some((access, pointee)),
            _ => None,
        },
        _ => None,
    }
}

/// The value a pointer member call, property or companion call produces, or
/// `None` when `expr` is not one.
pub(super) fn expression(
    expr: &hir::Expr,
    expected: mir::Type,
    context: &mut LoweringContext,
) -> DiagnosticResult<Option<mir::Rvalue>> {
    match unparenthesized_place(expr) {
        hir::Expr::MethodCall {
            object,
            method,
            args,
            null_safe: false,
            span,
        } => {
            let Some((access, pointee)) = pointer_type(object, context) else {
                return Ok(None);
            };
            let Some(member) = pointer_intrinsics::instance_member(access, method) else {
                return Ok(None);
            };
            let pointer = materialize_pointer(object, context)?;
            match member.operation {
                PointerOperation::Read { aligned } => {
                    let ty = pointee_type(&pointee, *span, context)?;
                    let target = context.declare_return_temp(ty, false);
                    context.push_statement(mir::Statement::Pointer {
                        target: Some(target),
                        operation: mir::PointerOperation::Read {
                            pointer,
                            ty,
                            aligned,
                        },
                        span: *span,
                    });
                    Ok(Some(local_rvalue(target, ty, false)))
                }
                PointerOperation::Offset | PointerOperation::ByteOffset => {
                    let scale = if member.operation == PointerOperation::Offset {
                        pointee_size(&pointee, *span, context)?
                    } else {
                        1
                    };
                    let [count] = argument_values(args)[..] else {
                        return Err(vec![unsupported(*span, "a pointer offset has one argument")]);
                    };
                    let count = lower_integer_expression(count, context)?;
                    let target = context.declare_return_temp(POINTER, false);
                    context.push_statement(mir::Statement::Pointer {
                        target: Some(target),
                        operation: mir::PointerOperation::Offset {
                            pointer,
                            count,
                            scale,
                        },
                        span: *span,
                    });
                    Ok(Some(local_rvalue(target, POINTER, false)))
                }
                _ => Err(vec![unsupported(
                    *span,
                    format!("pointer member `{method}` has no value"),
                )]),
            }
        }
        hir::Expr::PropertyAccess {
            object,
            property,
            null_safe: false,
            span,
            ..
        } => {
            let Some((access, _)) = pointer_type(object, context) else {
                return Ok(None);
            };
            if pointer_intrinsics::instance_member(access, property)
                .map(|member| member.operation)
                != Some(PointerOperation::Address)
            {
                return Ok(None);
            }
            let pointer = materialize_pointer(object, context)?;
            let address = mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::UInt64));
            let target = context.declare_return_temp(address, false);
            context.push_statement(mir::Statement::Pointer {
                target: Some(target),
                operation: mir::PointerOperation::Address { pointer },
                span: *span,
            });
            Ok(Some(local_rvalue(target, address, false)))
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } => {
            let Some(access) = PointerAccess::from_source_name(class_name) else {
                return Ok(None);
            };
            let Some(member) = pointer_intrinsics::static_member(access, method) else {
                return Ok(None);
            };
            let [argument] = argument_values(args)[..] else {
                return Err(vec![unsupported(
                    *span,
                    format!("`{}::{method}` has one argument", access.short_name()),
                )]);
            };
            match member.operation {
                // The address is the value: only checking distinguishes
                // the pointee, access or C function type.
                PointerOperation::Cast
                | PointerOperation::DropWriteAccess
                | PointerOperation::ToFunction => {
                    lower_rvalue_as_expected(argument, expected, context).map(Some)
                }
                PointerOperation::AddressOf => {
                    let hir::Expr::Variable { name, span: place } = unparenthesized_place(argument)
                    else {
                        return Err(vec![unsupported(
                            *span,
                            "a pointer to a field or property is not built yet",
                        )]);
                    };
                    let local = context.lookup_local(name, *place)?;
                    let target = context.declare_return_temp(POINTER, false);
                    context.push_statement(mir::Statement::Pointer {
                        target: Some(target),
                        operation: mir::PointerOperation::AddressOf { local },
                        span: *span,
                    });
                    Ok(Some(local_rvalue(target, POINTER, false)))
                }
                PointerOperation::FromAddress => {
                    let address = lower_integer_expression(argument, context)?;
                    let nullable = mir::Type::NullableScalar(mir::ScalarType::Inline(
                        crate::inline_value::InlineValueType::Pointer,
                    ));
                    let target = context.declare_return_temp(nullable, false);
                    context.push_statement(mir::Statement::Pointer {
                        target: Some(target),
                        operation: mir::PointerOperation::FromAddress { address },
                        span: *span,
                    });
                    Ok(Some(local_rvalue(target, nullable, false)))
                }
                _ => Err(vec![unsupported(
                    *span,
                    format!(
                        "`{}::{method}` is not built yet",
                        access.short_name()
                    ),
                )]),
            }
        }
        _ => Ok(None),
    }
}

/// `$mutable->write($value)` and `writeUnaligned` as a statement; `false`
/// when `expr` is not one.
pub(super) fn statement(expr: &hir::Expr, context: &mut LoweringContext) -> DiagnosticResult<bool> {
    let hir::Expr::MethodCall {
        object,
        method,
        args,
        null_safe: false,
        span,
    } = unparenthesized_place(expr)
    else {
        return Ok(false);
    };
    let Some((access, pointee)) = pointer_type(object, context) else {
        return Ok(false);
    };
    let Some(PointerOperation::Write { aligned }) =
        pointer_intrinsics::instance_member(access, method).map(|member| member.operation)
    else {
        // A read or offset whose value the statement discards still runs.
        let expected = context.expression_type(expr)?;
        return match expression(expr, expected, context)? {
            Some(value) => {
                lower_discarded_rvalue(value, context);
                Ok(true)
            }
            None => Ok(false),
        };
    };
    let [value] = argument_values(args)[..] else {
        return Err(vec![unsupported(*span, "a pointer write has one argument")]);
    };
    let pointer = materialize_pointer(object, context)?;
    let ty = pointee_type(&pointee, *span, context)?;
    let value = lower_rvalue_as_expected(value, ty, context)?;
    context.push_statement(mir::Statement::Pointer {
        target: None,
        operation: mir::PointerOperation::Write {
            pointer,
            value,
            aligned,
        },
        span: *span,
    });
    Ok(true)
}

/// The pointer `expr` evaluates to, in a local the operation reads.
fn materialize_pointer(
    expr: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::LocalId> {
    let value = lower_rvalue_as_expected(expr, POINTER, context)?;
    if let mir::Rvalue::Value(mir::ValueExpression::Inline(mir::InlineValueExpression::Use {
        operand: mir::Operand::Local(local),
        ..
    })) = value
    {
        return Ok(local);
    }
    let local = context.declare_return_temp(POINTER, false);
    context.push_statement(mir::Statement::AssignLocal {
        target: local,
        value,
    });
    Ok(local)
}

/// The MIR type a pointer reads and writes: a scalar with the same C layout.
fn pointee_type(
    pointee: &ResolvedType,
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<mir::Type> {
    match context.mir_resolved_type(pointee) {
        Some(ty @ mir::Type::Scalar(scalar)) if crate::native_abi::pointee_scalar_size(scalar).is_some() => Ok(ty),
        _ => Err(vec![unsupported(
            span,
            "reading and writing foreign structs and unions through a pointer is not built yet",
        )]),
    }
}

/// The size of `T` in `offset`'s scaling.
fn pointee_size(pointee: &ResolvedType, span: Span, context: &LoweringContext) -> DiagnosticResult<u64> {
    let mir::Type::Scalar(scalar) = pointee_type(pointee, span, context)? else {
        unreachable!("a pointee type is a scalar");
    };
    Ok(crate::native_abi::pointee_scalar_size(scalar).expect("a pointee scalar has a C size"))
}
