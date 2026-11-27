//! Pointer operations and extern declaration bodies (Decision 0148
//! sections 4, 14 and 15).
//!
//! A pointer is the one-word inline value `InlineValueType::Pointer`; its
//! pointee type and access are checking facts that MIR does not carry. Each
//! operation that reads or changes memory or an address is one
//! `Statement::Pointer` over a pointer local, so both native backends lower
//! one checked operation per statement and the interpreter can find every
//! one before it runs a program. Conversions that keep the address (`cast`,
//! `Pointer::from`, `toFunction`) are the value itself.

use super::*;
use crate::foreign::{ForeignType, PointerAccess};
use crate::pointer_intrinsics::{self, PointerOperation};

pub(super) const POINTER: mir::Type = mir::Type::Scalar(mir::ScalarType::Inline(
    crate::inline_value::InlineValueType::Pointer,
));

/// The checked pointer type of `expr`: its access and pointee.
fn pointer_type(
    expr: &hir::Expr,
    context: &LoweringContext,
) -> Option<(PointerAccess, ResolvedType)> {
    pointer_parts(context.resolved_expression_type(expr).ok()?)
}

/// The checked `?Pointer<T>` or `?WritablePointer<T>` type of `expr`: its
/// access and pointee.
fn nullable_pointer_type(
    expr: &hir::Expr,
    context: &LoweringContext,
) -> Option<(PointerAccess, ResolvedType)> {
    match context.resolved_expression_type(expr).ok()? {
        ResolvedType::Nullable(inner) => pointer_parts(*inner),
        _ => None,
    }
}

fn pointer_parts(ty: ResolvedType) -> Option<(PointerAccess, ResolvedType)> {
    match ty {
        ResolvedType::Foreign(foreign) => match *foreign {
            ForeignType::Pointer { access, pointee } => Some((access, pointee)),
            _ => None,
        },
        _ => None,
    }
}

/// The data or C function pointer type `ty` names in this instance, with
/// its type arguments substituted: the canonical checked type that pointer
/// type tests compare and an exact `mixed` tag interns.
pub(super) fn exact_pointer_type(
    ty: &ResolvedType,
    context: &LoweringContext,
) -> Option<ForeignType<ResolvedType>> {
    match substitute_resolved_type(ty, &context.type_substitutions) {
        ResolvedType::Foreign(foreign)
            if matches!(
                *foreign,
                ForeignType::Pointer { .. } | ForeignType::Function(_)
            ) =>
        {
            Some(*foreign)
        }
        _ => None,
    }
}

/// The foreign Copy type `ty` names in this instance whose identity its MIR
/// type does not carry: a data or C function pointer, or a C boundary type,
/// which is the target integer of its width.
fn exact_foreign_scalar(
    ty: &ResolvedType,
    context: &LoweringContext,
) -> Option<ForeignType<ResolvedType>> {
    exact_pointer_type(ty, context).or_else(|| {
        match substitute_resolved_type(ty, &context.type_substitutions) {
            ResolvedType::Foreign(foreign) if matches!(*foreign, ForeignType::Boundary(_)) => {
                Some(*foreign)
            }
            _ => None,
        }
    })
}

/// Whether `$value is T` can hold for a scalar `value` when `T` or the
/// value's type is a pointer, a C function pointer or a C boundary type, or
/// `None` otherwise. Their MIR types do not carry their identity (a
/// `CLong` is the target's integer), so the test compares the checked
/// types: it holds for a value of exactly the tested type, which for a
/// nullable value means one that is not null (Decision 0148 sections 3, 5
/// and 12).
pub(super) fn foreign_type_test_matches(
    value: &hir::Expr,
    value_type: mir::Type,
    tested: &ResolvedType,
    context: &LoweringContext,
) -> Option<bool> {
    if !matches!(
        value_type,
        mir::Type::Scalar(_) | mir::Type::NullableScalar(_)
    ) {
        return None;
    }
    let value = match context.resolved_expression_type(value).ok()? {
        ResolvedType::Nullable(inner) => *inner,
        value => value,
    };
    let tested = exact_foreign_scalar(tested, context);
    let value = exact_foreign_scalar(&value, context);
    (tested.is_some() || value.is_some()).then(|| tested == value)
}

/// The value a pointer member call, property or companion call produces, or
/// `None` when `expr` is not one.
pub(super) fn expression(
    expr: &hir::Expr,
    expected: mir::Type,
    context: &mut LoweringContext,
) -> DiagnosticResult<Option<mir::Rvalue>> {
    if let Some(value) = null_safe_member(expr, expected, context)? {
        return Ok(Some(value));
    }
    match unparenthesized_place(expr, context) {
        // The name of a C-callable Doria function is the address of its C entry
        // (section 12).
        hir::Expr::Identifier { name, span }
            if context
                .semantic_info
                .c_abi_function_values
                .contains_key(span) =>
        {
            let function = context.lookup_function(name, *span)?.id;
            let target = context.declare_return_temp(POINTER, false);
            context.push_statement(mir::Statement::Pointer {
                target: Some(target),
                operation: mir::PointerOperation::CEntryAddress { function },
                span: *span,
            });
            let value = local_rvalue(target, POINTER, false);
            Ok(Some(if expected == POINTER {
                value
            } else {
                nullable_present_rvalue(value)
            }))
        }
        // `$reference->toHandle()` consumes the reference (section 11).
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe: false,
        } if method == crate::foreign_handles::TO_HANDLE
            && args.is_empty()
            && matches!(
                context.expression_type(object)?,
                mir::Type::SharedReference(_) | mir::Type::WritableSharedReference(_)
            ) =>
        {
            let ty = context.expression_type(object)?;
            create_handle(object, ty, *span, expected, context).map(Some)
        }
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
            let (target, ty) = member_value(pointer, &pointee, member, args, *span, context)?;
            Ok(Some(local_rvalue(target, ty, false)))
        }
        hir::Expr::PropertyAccess {
            object,
            property,
            null_safe: false,
            span,
            ..
        } => {
            if property == "pointer"
                && crate::foreign_strings::encoding_of(&context.resolved_expression_type(object)?)
                    .is_some()
            {
                let string = materialize_foreign_string(object, context)?;
                let target = context.declare_return_temp(POINTER, false);
                context.push_statement(mir::Statement::Pointer {
                    target: Some(target),
                    operation: mir::PointerOperation::ForeignStringData { string },
                    span: *span,
                });
                return Ok(Some(local_rvalue(target, POINTER, false)));
            }
            if let (true, mir::Type::Region(region_type)) = (
                matches!(property.as_str(), "pointer" | "writablePointer"),
                context.expression_type(object)?,
            ) {
                let region = byte_region::local(object, false, context)?;
                let target = byte_region::pointer(region, region_type, *span, context)?;
                return Ok(Some(local_rvalue(target, POINTER, false)));
            }
            let Some((access, pointee)) = pointer_type(object, context) else {
                return Ok(None);
            };
            let Some(member) = pointer_intrinsics::instance_member(access, property)
                .filter(|member| member.operation == PointerOperation::Address)
            else {
                return Ok(None);
            };
            let pointer = materialize_pointer(object, context)?;
            let (target, ty) = member_value(pointer, &pointee, member, &[], *span, context)?;
            Ok(Some(local_rvalue(target, ty, false)))
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } if crate::foreign::ForeignStringEncoding::from_source_name(class_name).is_some() => {
            owned_string_construction(method, args, *span, expected, context)
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } if crate::foreign_handles::static_operation(class_name, method).is_some() => {
            let (_, operation) = crate::foreign_handles::static_operation(class_name, method)
                .expect("the handle operation was matched");
            let [argument] = argument_values(args)[..] else {
                return Err(vec![unsupported(
                    *span,
                    format!("`{class_name}::{method}` has one argument"),
                )]);
            };
            match operation {
                crate::foreign_handles::HandleOperation::Create => {
                    let ty = context.expression_type(argument)?;
                    create_handle(argument, ty, *span, expected, context).map(Some)
                }
                crate::foreign_handles::HandleOperation::Reclaim
                | crate::foreign_handles::HandleOperation::Borrow => {
                    let ty = context.expression_type(unparenthesized_place(expr, context))?;
                    let reclaim = operation == crate::foreign_handles::HandleOperation::Reclaim;
                    let handle = materialize_pointer(argument, context)?;
                    handle_value_type(ty, *span)?;
                    let target = if reclaim {
                        context.declare_owned_temp(ty)
                    } else {
                        context.declare_borrowed_temp(ty, false)
                    };
                    context.push_statement(mir::Statement::Pointer {
                        target: Some(target),
                        operation: mir::PointerOperation::FromHandle {
                            handle,
                            ty,
                            reclaim,
                        },
                        span: *span,
                    });
                    Ok(Some(local_rvalue(target, ty, reclaim)))
                }
            }
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } if class_name == "String"
            && crate::foreign::ForeignStringEncoding::from_string_reader(method).is_some() =>
        {
            let encoding = crate::foreign::ForeignStringEncoding::from_string_reader(method)
                .expect("the reader was matched");
            let [argument] = argument_values(args)[..] else {
                return Err(vec![unsupported(
                    *span,
                    format!("`String::{method}` has one argument"),
                )]);
            };
            let pointer = materialize_pointer(argument, context)?;
            let target = context.declare_return_temp(mir::Type::NullableString, false);
            context.push_statement(mir::Statement::Pointer {
                target: Some(target),
                operation: mir::PointerOperation::ReadForeignString { pointer, encoding },
                span: *span,
            });
            // As for a call's string result, the string's own cleanup
            // transport releases the temporary.
            Ok(Some(local_rvalue(target, mir::Type::NullableString, false)))
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
                    let (place, offset) =
                        pointer_place(argument, access == PointerAccess::Write, context)?;
                    let target = context.declare_return_temp(POINTER, false);
                    context.push_statement(mir::Statement::Pointer {
                        target: Some(target),
                        operation: mir::PointerOperation::AddressOf { place, offset },
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
                    format!("`{}::{method}` is not built yet", access.short_name()),
                )]),
            }
        }
        _ => Ok(None),
    }
}

/// `CString::from($text)`, `CString::tryFrom($text)` and the `Utf16String`
/// pair (section 9): a new owned buffer, absent from `tryFrom` for text
/// with NUL.
fn owned_string_construction(
    method: &str,
    args: &[hir::Argument],
    span: Span,
    expected: mir::Type,
    context: &mut LoweringContext,
) -> DiagnosticResult<Option<mir::Rvalue>> {
    let construction = crate::foreign_strings::OwnedStringConstruction::from_name(method);
    let [argument] = argument_values(args)[..] else {
        return Err(vec![unsupported(
            span,
            format!("`{method}` has one argument"),
        )]);
    };
    let text = Box::new(lower_string_expression(argument, context)?);
    match (construction, expected) {
        (
            Some(crate::foreign_strings::OwnedStringConstruction::From),
            mir::Type::Collection(collection),
        ) => Ok(Some(mir::Rvalue::Collection(
            mir::CollectionExpression::ForeignString {
                collection,
                text,
                span,
            },
        ))),
        (
            Some(crate::foreign_strings::OwnedStringConstruction::TryFrom),
            mir::Type::NullableCollection(collection),
        ) => Ok(Some(mir::Rvalue::NullableCollection(
            mir::NullableCollectionExpression::TryForeignString { collection, text },
        ))),
        _ => Err(vec![unsupported(
            span,
            format!("`{method}` does not produce the expected foreign string type"),
        )]),
    }
}

/// The local holding the `CString` or `Utf16String` that `expr` names,
/// borrowed for its `pointer`.
fn materialize_foreign_string(
    expr: &hir::Expr,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::LocalId> {
    let (local, _) = lower_collection_local(expr, context)?;
    Ok(local)
}

/// `$writablePointer->write($value)` and `writeUnaligned` as a statement; `false`
/// when `expr` is not one.
/// `$writablePointer?->write($value)` writes only when the pointer is not null.
pub(super) fn statement(expr: &hir::Expr, context: &mut LoweringContext) -> DiagnosticResult<bool> {
    let hir::Expr::MethodCall {
        object,
        method,
        args,
        null_safe,
        span,
    } = unparenthesized_place(expr, context)
    else {
        return Ok(false);
    };
    let Some((access, pointee)) = (if *null_safe {
        nullable_pointer_type(object, context)
    } else {
        pointer_type(object, context)
    }) else {
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
    if *null_safe {
        null_safe_pointer(object, *span, None, context, |pointer, context| {
            write(pointer, &pointee, aligned, args, *span, context)?;
            Ok(None)
        })?;
    } else {
        let pointer = materialize_pointer(object, context)?;
        write(pointer, &pointee, aligned, args, *span, context)?;
    }
    Ok(true)
}

/// The value of the pointer member `member` of the pointer in `pointer`: a
/// read, an element or byte offset, or the address, in a new local.
fn member_value(
    pointer: mir::LocalId,
    pointee: &ResolvedType,
    member: pointer_intrinsics::PointerMember,
    args: &[hir::Argument],
    span: Span,
    context: &mut LoweringContext,
) -> DiagnosticResult<(mir::LocalId, mir::Type)> {
    let (ty, operation) = match member.operation {
        PointerOperation::Read { aligned } => {
            let ty = pointee_type(pointee, span, context)?;
            (
                ty,
                mir::PointerOperation::Read {
                    pointer,
                    ty,
                    aligned,
                },
            )
        }
        PointerOperation::Offset | PointerOperation::ByteOffset => {
            let scale = if member.operation == PointerOperation::Offset {
                pointee_size(pointee, span, context)?
            } else {
                1
            };
            let [count] = argument_values(args)[..] else {
                return Err(vec![unsupported(span, "a pointer offset has one argument")]);
            };
            let count = lower_integer_expression(count, context)?;
            (
                POINTER,
                mir::PointerOperation::Offset {
                    pointer,
                    count,
                    scale,
                },
            )
        }
        PointerOperation::Address => (
            mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::UInt64)),
            mir::PointerOperation::Address { pointer },
        ),
        _ => {
            return Err(vec![unsupported(
                span,
                format!("pointer member `{}` has no value", member.name),
            )])
        }
    };
    let target = context.declare_return_temp(ty, false);
    context.push_statement(mir::Statement::Pointer {
        target: Some(target),
        operation,
        span,
    });
    Ok((target, ty))
}

/// `write` or `writeUnaligned` of the one argument in `args` through the
/// pointer in `pointer`.
fn write(
    pointer: mir::LocalId,
    pointee: &ResolvedType,
    aligned: bool,
    args: &[hir::Argument],
    span: Span,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let [value] = argument_values(args)[..] else {
        return Err(vec![unsupported(span, "a pointer write has one argument")]);
    };
    let ty = pointee_type(pointee, span, context)?;
    let value = lower_rvalue_as_expected(value, ty, context)?;
    context.push_statement(mir::Statement::Pointer {
        target: None,
        operation: mir::PointerOperation::Write {
            pointer,
            value,
            aligned,
        },
        span,
    });
    Ok(())
}

/// `$maybe?->read()`, `?->readUnaligned()`, `?->offset($count)`,
/// `?->byteOffset($bytes)` and `?->address` of a `?Pointer<T>` or
/// `?WritablePointer<T>` (Decision 0148 section 3): the member's value when
/// the pointer is not null, and null otherwise, converted to `expected`;
/// `None` when `expr` is not one.
fn null_safe_member(
    expr: &hir::Expr,
    expected: mir::Type,
    context: &mut LoweringContext,
) -> DiagnosticResult<Option<mir::Rvalue>> {
    let (object, name, args, span) = match unparenthesized_place(expr, context) {
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe: true,
        } => (object, method, args.as_slice(), *span),
        hir::Expr::PropertyAccess {
            object,
            property,
            null_safe: true,
            take: false,
            span,
            ..
        } => (object, property, &[][..], *span),
        _ => return Ok(None),
    };
    let Some((access, pointee)) = nullable_pointer_type(object, context) else {
        return Ok(None);
    };
    let Some(member) = pointer_intrinsics::instance_member(access, name) else {
        return Ok(None);
    };
    let result = context.expression_type(expr)?;
    let value = null_safe_pointer(
        object,
        span,
        Some((result, false)),
        context,
        |pointer, context| {
            let (local, ty) = member_value(pointer, &pointee, member, args, span, context)?;
            Ok(Some((local, ty, false)))
        },
    )?;
    let (local, ty, transfer) = value.expect("a null-safe pointer member has a value");
    convert_call_result(local, ty, expected, transfer, span, context).map(Some)
}

/// The branch of `?->` on the nullable pointer `object`, evaluated once:
/// `operation` runs on the pointer in a local when it is not null, and the
/// result, if any, is null otherwise.
fn null_safe_pointer(
    object: &hir::Expr,
    span: Span,
    result: Option<(mir::Type, bool)>,
    context: &mut LoweringContext,
    operation: impl FnOnce(
        mir::LocalId,
        &mut LoweringContext,
    ) -> DiagnosticResult<Option<(mir::LocalId, mir::Type, bool)>>,
) -> DiagnosticResult<Option<(mir::LocalId, mir::Type, bool)>> {
    let mir::Type::Scalar(scalar) = POINTER else {
        unreachable!("a pointer is a scalar")
    };
    let selection = context.coalesce_selection(object);
    let receiver =
        mir::Rvalue::NullableScalar(lower_nullable_scalar_expression(object, scalar, context)?);
    materialize_null_safe_call(
        receiver,
        selection,
        false,
        None,
        span,
        result,
        |receiver, context| {
            let pointer = context.declare_return_temp(POINTER, false);
            context.push_statement(mir::Statement::AssignLocal {
                target: pointer,
                value: mir::Rvalue::Value(value_expression_from_operand(
                    scalar,
                    mir::Operand::NullablePayload(receiver),
                )),
            });
            operation(pointer, context)
        },
        context,
    )
}

/// The handle that holds `value` of type `ty`, whose ownership moves into
/// it (Decision 0148 section 11).
fn create_handle(
    value: &hir::Expr,
    ty: mir::Type,
    span: Span,
    expected: mir::Type,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::Rvalue> {
    handle_value_type(ty, span)?;
    let value = lower_call_argument(value, ty, mir::FunctionParameterMode::Take, context)?;
    let target = context.declare_return_temp(POINTER, false);
    context.push_statement(mir::Statement::Pointer {
        target: Some(target),
        operation: mir::PointerOperation::CreateHandle { value },
        span,
    });
    let handle = local_rvalue(target, POINTER, false);
    Ok(if expected == POINTER {
        handle
    } else {
        present_as_expected(handle, expected)
    })
}

/// A handle holds a class value or a shared reference
/// ([`mir::Type::is_handle_value`]).
fn handle_value_type(ty: mir::Type, span: Span) -> DiagnosticResult<()> {
    if ty.is_handle_value() {
        Ok(())
    } else {
        Err(vec![unsupported(
            span,
            "a handle holds a class value or a shared reference",
        )])
    }
}

/// The pointer `expr` evaluates to, in a local the operation reads.
pub(super) fn materialize_pointer(
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

/// The MIR type a pointer reads and writes: a scalar or a foreign struct or
/// union with the same C layout.
fn pointee_type(
    pointee: &ResolvedType,
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<mir::Type> {
    match context.mir_resolved_type(pointee) {
        Some(ty) if crate::native_abi::pointee_layout(ty).is_some() => Ok(ty),
        _ => Err(vec![unsupported(
            span,
            "checking admitted a pointee without a C layout",
        )]),
    }
}

/// The size of `T` in `offset`'s scaling: its C size, which for a foreign
/// struct or union already includes the padding to its alignment.
fn pointee_size(
    pointee: &ResolvedType,
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<u64> {
    let ty = pointee_type(pointee, span, context)?;
    Ok(crate::native_abi::pointee_layout(ty)
        .expect("a pointee type has a C layout")
        .0)
}

/// The body of an extern declaration's MIR function (Decision 0148 sections
/// 14 and 15): one foreign call of its C function with the parameters, whose
/// result, if any, the function returns. Every Doria call of the declaration
/// is an ordinary call of this function.
pub(super) fn lower_extern_body(
    function: &hir::FunctionDecl,
    params: &[mir::LocalId],
    parameter_types: &[mir::Type],
    return_type: mir::ReturnType,
    context: &mut LoweringContext,
) -> DiagnosticResult<()> {
    let Some(binding) = context.semantic_info.foreign_bindings.get(&function.span) else {
        return Err(vec![Diagnostic::internal_compiler_error(
            "extern declaration has no binding",
            format!(
                "checking recorded no foreign binding for the extern declaration `{}`",
                function.name
            ),
            function.span,
        )]);
    };
    let result = match return_type {
        mir::ReturnType::Value(ty) => Some(ty),
        mir::ReturnType::Void => None,
    };
    let foreign = mir::ForeignFunction {
        callee: mir::ForeignCallee::Symbol {
            symbol: binding.symbol.clone(),
            library: binding.library.clone(),
        },
        system_code: binding.system_code,
        parameters: parameter_types.to_vec(),
        return_type: result,
        fixed_parameters: binding.fixed_parameters,
    };
    let target = result.map(|ty| context.declare_return_temp(ty, false));
    context.push_statement(mir::Statement::ForeignCall {
        function: Box::new(foreign),
        args: params.to_vec(),
        target,
        span: function.span,
    });
    context.cleanup_scopes_from(0);
    context.terminate_current(match (target, result) {
        (Some(local), Some(ty)) => mir::Terminator::Return(local_rvalue(local, ty, false)),
        _ => mir::Terminator::ReturnVoid,
    });
    Ok(())
}

/// A call of a C function pointer value (Decision 0148 section 12), or
/// `None` when `call` calls a Doria function value. The callee is
/// evaluated first and then each argument from left to right, each copied
/// into a local of its C parameter's type; one foreign call of the pointer
/// follows. Its result, which a C function returns by value, is the call's
/// unowned result local.
pub(super) fn function_pointer_call(
    call: &hir::CallableCall,
    function_type: &ResolvedType,
    context: &mut LoweringContext,
) -> DiagnosticResult<Option<Option<(mir::LocalId, mir::Type, bool)>>> {
    let ResolvedType::Foreign(foreign) = function_type else {
        return Ok(None);
    };
    let ForeignType::Function(signature) = foreign.as_ref() else {
        return Ok(None);
    };
    let callee = lower_rvalue_as_expected(&call.callee, POINTER, context)?;
    let (callee, _) = core_value::materialize_value(callee, context);
    if call.args.len() != signature.parameters.len() {
        return Err(vec![unsupported(
            call.span,
            "checking admitted a C function pointer call with another argument count",
        )]);
    }
    let mut parameters = Vec::with_capacity(signature.parameters.len());
    let mut args = Vec::with_capacity(call.args.len());
    for (argument, parameter) in call.args.iter().zip(&signature.parameters) {
        let ty = context.mir_resolved_type(parameter).ok_or_else(|| {
            vec![unsupported(
                argument.span,
                "a C function pointer parameter has no MIR type",
            )]
        })?;
        let value = lower_rvalue_as_expected(&argument.value, ty, context)?;
        let (local, _) = core_value::materialize_value(value, context);
        parameters.push(ty);
        args.push(local);
    }
    let return_type = match &signature.return_type {
        ResolvedType::Void => None,
        ty => Some(context.mir_resolved_type(ty).ok_or_else(|| {
            vec![unsupported(
                call.span,
                "a C function pointer result has no MIR type",
            )]
        })?),
    };
    let target = return_type.map(|ty| context.declare_return_temp(ty, false));
    context.push_statement(mir::Statement::ForeignCall {
        function: Box::new(mir::ForeignFunction {
            callee: mir::ForeignCallee::Pointer(callee),
            system_code: None,
            parameters,
            return_type,
            fixed_parameters: None,
        }),
        args,
        target,
        span: call.span,
    });
    Ok(Some(
        target
            .zip(return_type)
            .map(|(local, ty)| (local, ty, false)),
    ))
}

/// A field of a foreign struct or union as a place: where the outermost
/// aggregate is, the field's byte offset in it, and the field's type.
struct FieldPlace {
    aggregate: mir::InlinePlace,
    offset: u64,
    field: FieldKind,
    /// The aggregate is in lasting storage. A computed value, or a copy
    /// read from a `mixed` box, lives in a temporary instead, which a
    /// field read may use and `Pointer::to` may not address.
    stored: bool,
}

/// What a foreign field holds.
enum FieldKind {
    /// A scalar, pointer or nested aggregate of this MIR type.
    Value(mir::Type),
    /// A fixed-length array field `element[length]`, read as a typed array
    /// region and written by copying one (section 6).
    Array { element: ResolvedType, length: u64 },
}

/// The checked facts of the foreign struct or union `ty` is, or `None` for
/// another type.
fn aggregate_facts<'c>(
    ty: &ResolvedType,
    context: &'c LoweringContext,
) -> Option<&'c crate::foreign::ForeignAggregateFacts> {
    let ResolvedType::Foreign(foreign) = ty else {
        return None;
    };
    let ForeignType::Aggregate(aggregate) = foreign.as_ref() else {
        return None;
    };
    context
        .semantic_info
        .foreign_aggregates
        .get(&aggregate.name)
}

/// The MIR type of `ty` as a foreign aggregate.
fn aggregate_type(
    ty: &ResolvedType,
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<mir::ForeignAggregateType> {
    match context.mir_resolved_type(ty) {
        Some(mir::Type::ForeignAggregate(aggregate)) => Ok(aggregate),
        _ => Err(vec![unsupported(
            span,
            "a foreign struct or union reached lowering without a C layout",
        )]),
    }
}

/// `$aggregate->field` and `$aggregate->field->nested` as a place, or
/// `None` when `expr` is not a field access of a foreign struct or union.
/// The outermost aggregate is a local or a stored class property, each
/// addressed in place, so a write reaches it; checking proves the place
/// writable. Any other aggregate, such as a call result or a property a
/// getter hook computes, is first copied to a temporary, which only reads
/// reach.
fn field_place(
    expr: &hir::Expr,
    writable: bool,
    context: &mut LoweringContext,
) -> DiagnosticResult<Option<FieldPlace>> {
    let hir::Expr::PropertyAccess {
        object,
        property,
        null_safe,
        span,
        ..
    } = unparenthesized_place(expr, context)
    else {
        return Ok(None);
    };
    let Ok(object_type) = context.resolved_expression_type(object) else {
        return Ok(None);
    };
    let Some(facts) = aggregate_facts(&object_type, context) else {
        return Ok(None);
    };
    if *null_safe {
        return Err(vec![unsupported(
            *span,
            "a null-safe foreign field access reached the in-place field path",
        )]);
    }
    let Some(layout) = facts.layout.clone() else {
        return Err(vec![unsupported(
            *span,
            "a foreign struct or union without a C layout reached lowering",
        )]);
    };
    let Some((index, field)) = facts.field(property) else {
        return Err(vec![unsupported(
            *span,
            format!("checking admitted the unknown foreign field `{property}`"),
        )]);
    };
    let field_type = field.ty.clone();
    let field_offset = layout.fields[index].offset;
    let field = match fixed_array(&field_type) {
        Some((element, length)) => FieldKind::Array {
            element: element.clone(),
            length,
        },
        None => FieldKind::Value(
            context
                .mir_resolved_type(&field_type)
                .ok_or_else(|| vec![unsupported(*span, "a foreign field type has no MIR type")])?,
        ),
    };
    if let Some(outer) = field_place(object, writable, context)? {
        return Ok(Some(FieldPlace {
            aggregate: outer.aggregate,
            offset: outer.offset + field_offset,
            field,
            stored: outer.stored,
        }));
    }
    let aggregate_type = aggregate_type(&object_type, object.span(), context)?;
    let mut stored = true;
    let aggregate = match unparenthesized_place(object, context) {
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::ForeignAggregate(ty) if ty == aggregate_type => {
                    mir::InlinePlace::Local(local)
                }
                // Checking narrowed the `?Name` local to present, so the
                // field is in its storage at the payload offset.
                mir::Type::NullableForeignAggregate(ty) if ty == aggregate_type => {
                    mir::InlinePlace::NullableLocalAssumeNonNull(local)
                }
                // An exact `is` narrowed the `mixed` local: a read copies the
                // boxed value, which other copies of the box share.
                mir::Type::Mixed | mir::Type::NullableMixed if !writable => {
                    stored = false;
                    let value = aggregate_expression(object, aggregate_type, context)?;
                    let local = context
                        .declare_return_temp(mir::Type::ForeignAggregate(aggregate_type), false);
                    context.push_statement(mir::Statement::AssignLocal {
                        target: local,
                        value: mir::Rvalue::ForeignAggregate(value),
                    });
                    mir::InlinePlace::Local(local)
                }
                mir::Type::Mixed | mir::Type::NullableMixed => {
                    return Err(vec![unsupported(
                        *span,
                        "a field of a `mixed` value narrowed to a foreign struct or union cannot be written in place yet",
                    )])
                }
                _ => {
                    return Err(vec![unsupported(
                        *span,
                        "a field access names a local that holds no foreign struct or union",
                    )])
                }
            }
        }
        place @ hir::Expr::PropertyAccess { .. } if stored_property_place(place, context) => {
            let (object, property, ty) = lower_property_place(place, context)?;
            if ty != mir::Type::ForeignAggregate(aggregate_type) {
                return Err(vec![unsupported(
                    place.span(),
                    "a field access names a property that holds no foreign struct or union",
                )]);
            }
            mir::InlinePlace::Property { object, property }
        }
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
        } if context.constant_value(object).is_none() => mir::InlinePlace::Static(static_place(
            class_name,
            member,
            *span,
            mir::Type::ForeignAggregate(aggregate_type),
            context,
        )?),
        hir::Expr::Index {
            collection, index, ..
        } => element_place(
            collection,
            index,
            mir::Type::ForeignAggregate(aggregate_type),
            object.span(),
            writable,
            context,
        )?,
        _ => {
            stored = false;
            let value = lower_rvalue_as_expected(
                object,
                mir::Type::ForeignAggregate(aggregate_type),
                context,
            )?;
            let local =
                context.declare_return_temp(mir::Type::ForeignAggregate(aggregate_type), false);
            context.push_statement(mir::Statement::AssignLocal {
                target: local,
                value,
            });
            mir::InlinePlace::Local(local)
        }
    };
    Ok(Some(FieldPlace {
        aggregate,
        offset: field_offset,
        field,
        stored,
    }))
}

/// Element `index` of the collection or typed array region `collection`,
/// which holds foreign structs or unions of type `ty`, as a place in the
/// element's storage. The position or key is evaluated once into a local.
/// `writable` gives the collection or region writable access, which a
/// write through the element needs.
fn element_place(
    collection: &hir::Expr,
    index: &hir::Expr,
    element: mir::Type,
    index_span: Span,
    writable: bool,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::InlinePlace> {
    let position = mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64));
    if let mir::Type::Region(region) = context.expression_type(collection)? {
        if super::byte_region::element_type(region, index_span, context)? != element {
            return Err(vec![unsupported(
                index_span,
                "a region element place names a region of another element type",
            )]);
        }
        let region = super::byte_region::local(collection, writable, context)?;
        let index = lower_rvalue_as_expected(index, position, context)?;
        let (index, _) = core_value::materialize_value(index, context);
        return Ok(mir::InlinePlace::RegionElement {
            region,
            index,
            index_span,
        });
    }
    let Some((collection, collection_type)) =
        lower_collection_receiver(collection, writable, context)?
    else {
        return Err(vec![unsupported(
            index_span,
            "an element place names a value that is not a collection",
        )]);
    };
    let info = context.collection_type(collection_type).clone();
    if info.value != element {
        return Err(vec![unsupported(
            index_span,
            "an element place names a collection of another element type",
        )]);
    }
    let index = lower_rvalue_as_expected(index, info.key.unwrap_or(position), context)?;
    let (index, _) = core_value::materialize_value(index, context);
    Ok(mir::InlinePlace::CollectionElement {
        collection,
        index,
        positional: false,
        index_span,
    })
}

/// The storage `Pointer::to($place)` and `WritablePointer::to($place)`
/// address in place (Decision 0148 section 7, amended 2026-11-21): a local,
/// a narrowed nullable local, a foreign struct or union field, a stored
/// property, a static property, or an element of `T[]`, `List<T>` or a
/// typed array region. Checking admits only these and pins each one.
fn pointer_place(
    expr: &hir::Expr,
    writable: bool,
    context: &mut LoweringContext,
) -> DiagnosticResult<(mir::InlinePlace, u64)> {
    let place = unparenthesized_place(expr, context);
    if let Some(field) = field_place(place, writable, context)? {
        if !field.stored {
            return Err(vec![unsupported(
                place.span(),
                "checking admitted a pointer to a field of a value without lasting storage",
            )]);
        }
        return Ok((field.aggregate, field.offset));
    }
    let ty = context.expression_type(place)?;
    let addressed = match place {
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            let stored = context.local_type(local);
            // Checking narrowed a `?T` local to present: the pointee is its
            // payload, in place.
            if stored != ty
                && matches!(
                    stored,
                    mir::Type::NullableScalar(_) | mir::Type::NullableForeignAggregate(_)
                )
            {
                mir::InlinePlace::NullableLocalAssumeNonNull(local)
            } else {
                mir::InlinePlace::Local(local)
            }
        }
        hir::Expr::PropertyAccess { .. } if stored_property_place(place, context) => {
            let (object, property, _) = lower_property_place(place, context)?;
            mir::InlinePlace::Property { object, property }
        }
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
        } => mir::InlinePlace::Static(static_place(class_name, member, *span, ty, context)?),
        hir::Expr::Index {
            collection, index, ..
        } => element_place(collection, index, ty, place.span(), writable, context)?,
        _ => {
            return Err(vec![unsupported(
                place.span(),
                "checking admitted a pointer to a value that is not a place",
            )])
        }
    };
    Ok((addressed, 0))
}

/// `$values[$index]` and `$values->removeAt($index)` of a collection or
/// typed array region whose elements are `element`, a foreign struct or
/// union or its `?Name` form, as a copy of the element; `None` for another
/// expression.
fn element_rvalue(
    expr: &hir::Expr,
    element: mir::Type,
    context: &mut LoweringContext,
) -> DiagnosticResult<Option<mir::Rvalue>> {
    if let Some((collection, index, value_type)) = lower_list_remove_at(expr, context)? {
        if value_type != element {
            return Err(vec![unsupported(
                expr.span(),
                "List::removeAt of another element type reached foreign aggregate lowering",
            )]);
        }
        return collection_remove_at_rvalue(collection, index, element, expr.span()).map(Some);
    }
    let hir::Expr::Index {
        collection, index, ..
    } = unparenthesized_place(expr, context)
    else {
        return Ok(None);
    };
    let index_span = expr.span();
    if let mir::Type::Region(region) = context.expression_type(collection)? {
        let mir::Type::ForeignAggregate(ty) = element else {
            return Err(vec![unsupported(
                index_span,
                "a region of `?Name` elements reached foreign aggregate lowering",
            )]);
        };
        if super::byte_region::element_type(region, index_span, context)? != element {
            return Err(vec![unsupported(
                index_span,
                "a region element read names a region of another element type",
            )]);
        }
        let region = super::byte_region::local(collection, false, context)?;
        let index = lower_rvalue_as_expected(
            index,
            mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64)),
            context,
        )?;
        return Ok(Some(mir::Rvalue::ForeignAggregate(
            mir::ForeignAggregateExpression::RegionIndex {
                ty,
                region,
                index: Box::new(index),
                index_span,
            },
        )));
    }
    let (collection, index) = lower_collection_index_operand(collection, index, element, context)?;
    collection_value_rvalue(collection, index.clone(), index, element, false, index_span).map(Some)
}

/// Whether `expr` reads a stored class property in place: a property
/// access that is not null-safe, takes nothing and has no getter hook
/// (Decision 0135), whose value lives in the object's storage.
fn stored_property_place(expr: &hir::Expr, context: &LoweringContext) -> bool {
    matches!(
        expr,
        hir::Expr::PropertyAccess {
            null_safe: false,
            take: false,
            ..
        }
    ) && !context
        .semantic_info
        .property_accessor_calls
        .get(&expr.span())
        .is_some_and(|calls| calls.getter.is_some())
}

/// The element type and count of a fixed-length array field type.
fn fixed_array(ty: &ResolvedType) -> Option<(&ResolvedType, u64)> {
    let ResolvedType::Foreign(foreign) = ty else {
        return None;
    };
    match foreign.as_ref() {
        ForeignType::FixedArray { element, length } => Some((element, *length)),
        _ => None,
    }
}

/// The value of a foreign struct or union field read, converted to
/// `expected`, or `None` when `expr` is not one.
pub(super) fn field_read(
    expr: &hir::Expr,
    expected: mir::Type,
    context: &mut LoweringContext,
) -> DiagnosticResult<Option<mir::Rvalue>> {
    if let Some(value) = null_safe_field_read(expr, expected, context)? {
        return Ok(Some(value));
    }
    // A writable region of a fixed-length array field needs writable
    // access to the place that holds the aggregate.
    let writable = matches!(
        expected,
        mir::Type::Region(region) if region.access == crate::types::ByteRegionKind::Writable
    );
    let Some(place) = field_place(expr, writable, context)? else {
        return Ok(None);
    };
    let (ty, target, access) = match place.field {
        FieldKind::Value(ty) => {
            let target = context.declare_return_temp(ty, false);
            (ty, target, mir::ForeignFieldAccess::Read { target })
        }
        // A fixed-length array field reads as the region checking typed:
        // its access follows the containing place (section 6).
        FieldKind::Array { length, .. } => {
            let ty = context.expression_type(expr)?;
            let mir::Type::Region(region) = ty else {
                return Err(vec![unsupported(
                    expr.span(),
                    "a fixed-length array field reads as a region",
                )]);
            };
            byte_region::built_element(region, expr.span(), context)?;
            let target = context.declare_return_temp(ty, false);
            // A writable field region is the writable place its elements
            // are written through.
            context.locals[target.0].writable = region.is_writable();
            (
                ty,
                target,
                mir::ForeignFieldAccess::Region { target, length },
            )
        }
    };
    context.push_statement(mir::Statement::ForeignField {
        aggregate: place.aggregate,
        offset: place.offset,
        access,
        span: expr.span(),
    });
    convert_call_result(target, ty, expected, false, expr.span(), context).map(Some)
}

/// `$maybe?->field` of a `?Name` value (Decision 0148 section 6): the
/// field when the value is present, and null otherwise, converted to
/// `expected`; `None` when `expr` is not one. The receiver is evaluated
/// once. A local, stored property, static property or collection element
/// is read in place in the present branch, so a region over a
/// fixed-length array field views the stored value; any other receiver is
/// read from a local it is copied into.
fn null_safe_field_read(
    expr: &hir::Expr,
    expected: mir::Type,
    context: &mut LoweringContext,
) -> DiagnosticResult<Option<mir::Rvalue>> {
    let hir::Expr::PropertyAccess {
        object,
        property,
        null_safe: true,
        take: false,
        span,
        ..
    } = unparenthesized_place(expr, context)
    else {
        return Ok(None);
    };
    let Ok(mir::Type::NullableForeignAggregate(aggregate)) = context.expression_type(object) else {
        return Ok(None);
    };
    let result = context.expression_type(expr)?;
    let (present, nullable) = non_null_match_type(result);
    if !nullable {
        return Err(vec![unsupported(
            *span,
            "a null-safe foreign field read has no nullable result type",
        )]);
    }
    let nullable_type = mir::Type::NullableForeignAggregate(aggregate);
    let selection = context.coalesce_selection(object);
    let writable = present.is_writable_region();
    // A local is tested and read in place. A stored receiver is tested
    // through a copy and read in place.
    let local = match unparenthesized_place(object, context) {
        hir::Expr::Variable { name, span } => Some(context.lookup_local(name, *span)?)
            .filter(|local| context.local_type(*local) == nullable_type),
        _ => None,
    };
    let receiver = match local {
        Some(_) => None,
        None => Some(match stored_nullable_receiver(object, aggregate, writable, context)? {
            Some((copy, place)) => (copy, Some(place)),
            // Checking gives a region writable access only through a
            // writable place, and a call result of an inline Copy type is
            // never one (Decision 0089), so a temporary is only read.
            None if writable => {
                return Err(vec![unsupported(
                    *span,
                    "checking gave writable access to a field of a `?Name` value without lasting storage",
                )])
            }
            None => (nullable_aggregate_expression(object, aggregate, context)?, None),
        }),
    };
    let stored = receiver.as_ref().and_then(|(_, place)| *place);
    let read = move |receiver, context: &mut LoweringContext| {
        let place = stored.unwrap_or(mir::InlinePlace::NullableLocalAssumeNonNull(receiver));
        let field = present_field(place, aggregate, property, *span, present, context)?;
        // A writable region is a move carrier the result takes over, as
        // every null-safe region result is.
        Ok(Some((field, present, writable)))
    };
    let value = match local {
        Some(local) => null_safe_call_on(
            local,
            nullable_type,
            selection,
            None,
            *span,
            Some((result, false)),
            read,
            context,
        )?,
        None => {
            let (receiver, _) = receiver.expect("a receiver that is not a local is lowered");
            materialize_null_safe_call(
                mir::Rvalue::NullableForeignAggregate(receiver),
                selection,
                false,
                None,
                *span,
                Some((result, false)),
                read,
                context,
            )?
        }
    };
    let (local, ty, transfer) = value.expect("a null-safe field read has a value");
    convert_call_result(local, ty, expected, transfer, *span, context).map(Some)
}

/// A `?Name` receiver of type `aggregate` in lasting storage: a stored
/// property, a static property or a collection element, as a copy for the
/// presence test and the narrowed place a present field is in. `writable`
/// gives a collection writable access. `None` for any other receiver.
fn stored_nullable_receiver(
    object: &hir::Expr,
    aggregate: mir::ForeignAggregateType,
    writable: bool,
    context: &mut LoweringContext,
) -> DiagnosticResult<Option<(mir::NullableForeignAggregateExpression, mir::InlinePlace)>> {
    let nullable = mir::Type::NullableForeignAggregate(aggregate);
    let object = unparenthesized_place(object, context);
    if context.constant_value(object).is_some() {
        return Ok(None);
    }
    let mismatch = || {
        vec![unsupported(
            object.span(),
            "a null-safe field read names stored storage without a `?Name` value",
        )]
    };
    Ok(Some(match object {
        place @ hir::Expr::PropertyAccess { .. } if stored_property_place(place, context) => {
            let (object, property, actual) = lower_property_place(place, context)?;
            if actual != nullable {
                return Err(mismatch());
            }
            (
                mir::NullableForeignAggregateExpression::Property {
                    ty: aggregate,
                    object,
                    property,
                },
                mir::InlinePlace::NullablePropertyAssumeNonNull { object, property },
            )
        }
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
        } => {
            let (id, actual) = context.static_property(class_name, member, *span)?;
            if actual != nullable {
                return Err(mismatch());
            }
            (
                mir::NullableForeignAggregateExpression::Static { ty: aggregate, id },
                mir::InlinePlace::NullableStaticAssumeNonNull(id),
            )
        }
        hir::Expr::Index {
            collection, index, ..
        } => {
            let mir::InlinePlace::CollectionElement {
                collection,
                index,
                positional,
                index_span,
            } = element_place(
                collection,
                index,
                nullable,
                object.span(),
                writable,
                context,
            )?
            else {
                return Err(mismatch());
            };
            let position = local_rvalue(index, context.local_type(index), false);
            (
                mir::NullableForeignAggregateExpression::CollectionIndex {
                    ty: aggregate,
                    collection,
                    index: Box::new(position),
                    index_span,
                    positional,
                    remove: false,
                },
                mir::InlinePlace::NullableCollectionElementAssumeNonNull {
                    collection,
                    index,
                    positional,
                    index_span,
                },
            )
        }
        _ => return Ok(None),
    }))
}

/// The field `property` of the present `?Name` value in `place`, read into
/// a local of type `ty`.
fn present_field(
    place: mir::InlinePlace,
    aggregate: mir::ForeignAggregateType,
    property: &str,
    span: Span,
    ty: mir::Type,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::LocalId> {
    let mismatch = || {
        vec![unsupported(
            span,
            "a null-safe foreign field read names no checked field",
        )]
    };
    let definition = context
        .collection_registry
        .foreign_aggregates
        .get(aggregate.id.0)
        .cloned()
        .ok_or_else(mismatch)?;
    let facts = context
        .semantic_info
        .foreign_aggregates
        .get(&definition.name)
        .ok_or_else(mismatch)?;
    let (index, field) = facts.field(property).ok_or_else(mismatch)?;
    let offset = definition.layout.fields[index].offset;
    let array = fixed_array(&field.ty).map(|(_, length)| length);
    // A writable region the read forms is a fresh move carrier, which the
    // statement owns until the null-safe result takes it over.
    let owned = ty.is_writable_region();
    let target = context.declare_checked_call_slot(ty, owned);
    let access = match array {
        Some(length) => {
            let mir::Type::Region(region) = ty else {
                return Err(mismatch());
            };
            byte_region::built_element(region, span, context)?;
            context.locals[target.0].writable = region.is_writable();
            mir::ForeignFieldAccess::Region { target, length }
        }
        None => mir::ForeignFieldAccess::Read { target },
    };
    context.push_statement(mir::Statement::ForeignField {
        aggregate: place,
        offset,
        access,
        span,
    });
    if owned {
        context.track_statement_owned_local(target, ty);
    }
    Ok(target)
}

/// `$aggregate->field = $value`, a compound assignment to a field, and
/// `$aggregate->field++`; `false` when `target` is not a field of a foreign
/// struct or union. A compound form reads the field before it evaluates
/// the right-hand side, as every read-modify-write place does. A whole
/// fixed-length array field is copied from a readonly region of its count
/// (section 6, amended 2026-11-17).
pub(super) fn field_write(
    target: &hir::Expr,
    update: FieldUpdate<'_>,
    context: &mut LoweringContext,
) -> DiagnosticResult<bool> {
    let Some(place) = field_place(target, true, context)? else {
        return Ok(false);
    };
    let write = match (place.field, update) {
        (FieldKind::Array { element, length }, FieldUpdate::Assign(value)) => {
            let region = copied_region(&element, value.span(), context)?;
            mir::ForeignFieldValue {
                offset: place.offset,
                value: lower_rvalue_as_expected(value, region, context)?,
                copied_elements: Some(length),
            }
        }
        (FieldKind::Array { .. }, _) => {
            return Err(vec![unsupported(
                target.span(),
                "checking admitted a read-modify-write of a fixed-length array field",
            )])
        }
        (FieldKind::Value(ty), FieldUpdate::Assign(value)) => mir::ForeignFieldValue {
            offset: place.offset,
            value: lower_rvalue_as_expected(value, ty, context)?,
            copied_elements: None,
        },
        (FieldKind::Value(ty), update) => {
            let mir::Type::Scalar(scalar) = ty else {
                return Err(vec![unsupported(
                    target.span(),
                    "a read-modify-write of a foreign field needs a scalar field",
                )]);
            };
            let current = context.declare_return_temp(ty, false);
            context.push_statement(mir::Statement::ForeignField {
                aggregate: place.aggregate,
                offset: place.offset,
                access: mir::ForeignFieldAccess::Read { target: current },
                span: target.span(),
            });
            let operand = mir::Operand::Local(current);
            let value = mir::Rvalue::Value(match update {
                FieldUpdate::Compound { op, value, span } => {
                    lower_compound_value(operand, scalar, op, value, span, context)?
                }
                FieldUpdate::Increment { op, span } => {
                    lower_increment_value(operand, scalar, op, span)?
                }
                FieldUpdate::Assign(_) => unreachable!("handled above"),
            });
            mir::ForeignFieldValue {
                offset: place.offset,
                value,
                copied_elements: None,
            }
        }
    };
    context.push_statement(mir::Statement::ForeignField {
        aggregate: place.aggregate,
        offset: write.offset,
        access: mir::ForeignFieldAccess::Write {
            value: write.value,
            copied_elements: write.copied_elements,
        },
        span: target.span(),
    });
    Ok(true)
}

/// How a field write computes the value it stores.
#[derive(Clone, Copy)]
pub(super) enum FieldUpdate<'a> {
    Assign(&'a hir::Expr),
    Compound {
        op: &'a hir::AssignOp,
        value: &'a hir::Expr,
        span: Span,
    },
    Increment {
        op: &'a hir::IncrementOp,
        span: Span,
    },
}

/// A foreign struct or union value of type `ty`: a construction, a copy of
/// a local, or a value the shared paths (calls, pointer reads, field reads
/// and branches) have already placed in a local.
pub(super) fn aggregate_expression(
    expr: &hir::Expr,
    ty: mir::ForeignAggregateType,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::ForeignAggregateExpression> {
    if let Some(value) = context
        .constant_value(unparenthesized_place(expr, context))
        .cloned()
    {
        let lowered = lower_const_rvalue(
            &value,
            mir::Type::ForeignAggregate(ty),
            expr.span(),
            &context.semantic_info.enums,
            &context.semantic_info.foreign_aggregates,
            &mut |ty| context.mir_resolved_type(ty),
        )?;
        let mir::Rvalue::ForeignAggregate(value) = lowered else {
            unreachable!("a foreign aggregate constant lowers to a foreign aggregate value");
        };
        return Ok(value);
    }
    // `??`, `when` and `match` merge their branches through the shared
    // branching lowering, as every other value family's does.
    if is_branching_expr(unparenthesized_place(expr, context)) {
        let value = lower_branching_rvalue(
            unparenthesized_place(expr, context),
            mir::Type::ForeignAggregate(ty),
            false,
            context,
        )?;
        let mir::Rvalue::ForeignAggregate(value) = value else {
            return Err(vec![unsupported(
                expr.span(),
                "a branching foreign struct or union value merged to another type",
            )]);
        };
        return Ok(value);
    }
    if let Some(element) = element_rvalue(expr, mir::Type::ForeignAggregate(ty), context)? {
        let mir::Rvalue::ForeignAggregate(element) = element else {
            unreachable!("an element of `Name` values is a foreign aggregate value");
        };
        return Ok(element);
    }
    match unparenthesized_place(expr, context) {
        hir::Expr::New {
            class_type,
            args,
            span,
            ..
        } => construct(class_type, args, ty, *span, context),
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
        } => Ok(mir::ForeignAggregateExpression::Static {
            ty,
            id: static_place(
                class_name,
                member,
                *span,
                mir::Type::ForeignAggregate(ty),
                context,
            )?,
        }),
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::ForeignAggregate(actual) if actual == ty => {
                    Ok(mir::ForeignAggregateExpression::Local { ty, local })
                }
                // Checking narrowed the `?Name` local to present.
                mir::Type::NullableForeignAggregate(actual) if actual == ty => {
                    Ok(mir::ForeignAggregateExpression::NullableLocalAssumeNonNull { ty, local })
                }
                // An exact `is` narrowed the `mixed` local to `Name`.
                mir::Type::Mixed | mir::Type::NullableMixed
                    if context.exact_mixed_tag(expr)
                        == Some(mir::MixedTag::ForeignAggregate(ty)) =>
                {
                    Ok(mir::ForeignAggregateExpression::MixedPayload { ty, mixed: local })
                }
                _ => Err(vec![unsupported(
                    *span,
                    "a foreign struct or union local has another type",
                )]),
            }
        }
        // Getter hooks and interface properties are calls the shared
        // checked-call path lowers first, so a property access that
        // reaches here reads a stored property.
        place @ hir::Expr::PropertyAccess { .. } if stored_property_place(place, context) => {
            let (object, property, actual) = lower_property_place(place, context)?;
            if actual != mir::Type::ForeignAggregate(ty) {
                return Err(vec![unsupported(
                    place.span(),
                    "a property holds another type than the foreign struct or union expected",
                )]);
            }
            Ok(mir::ForeignAggregateExpression::Property {
                ty,
                object,
                property,
            })
        }
        call => match aggregate_call(call, mir::Type::ForeignAggregate(ty), context)? {
            Some((function, args, span)) => Ok(mir::ForeignAggregateExpression::Call {
                ty,
                function,
                args,
                span,
            }),
            None => Err(vec![unsupported(
                call.span(),
                "this foreign struct or union value is not built yet",
            )]),
        },
    }
}

/// The function a call of a Doria function, method or static method
/// returning `result`, a foreign struct or union or its `?Name` form,
/// calls, with its lowered arguments and span; `None` when `expr` is not
/// such a call.
fn aggregate_call(
    expr: &hir::Expr,
    result: mir::Type,
    context: &mut LoweringContext,
) -> DiagnosticResult<Option<(mir::FunctionId, Vec<mir::Rvalue>, Span)>> {
    let (signature, args, span) = match expr {
        hir::Expr::FunctionCall { name, args, span } => {
            let signature = context.lookup_function(name, *span)?;
            let args = lower_call_args(name, args, signature.clone(), *span, context)?;
            (signature, args, *span)
        }
        // A collection's members are its own operations, which the shared
        // collection paths lower; they are no Doria method.
        hir::Expr::MethodCall { object, .. }
            if matches!(
                context.expression_type(object)?,
                mir::Type::Collection(_)
                    | mir::Type::ReadonlySharedReferenceAccess(mir::SharedPayload::Collection(_))
                    | mir::Type::WritableSharedReferenceAccess(mir::SharedPayload::Collection(_))
            ) =>
        {
            return Ok(None)
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe: false,
        } => {
            let (signature, args) =
                lower_instance_method_call(object, method, args, *span, context)?;
            (signature, args, *span)
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } => {
            let (signature, args) =
                lower_static_method_call(class_name, method, args, *span, context)?;
            (signature, args, *span)
        }
        _ => return Ok(None),
    };
    if signature.return_type != mir::ReturnType::Value(result) {
        return Err(vec![unsupported(
            span,
            "a call returns another type than the foreign struct or union expected",
        )]);
    }
    Ok(Some((signature.id, args, span)))
}

/// A `?Name` value of type `ty`: `null`, a value of type `Name` made
/// present, or a copy of a `?Name` local, stored property, static property
/// or call result. Calls, branches and null-safe reads go through the shared
/// paths, which place their value in a local first.
pub(super) fn nullable_aggregate_expression(
    expr: &hir::Expr,
    ty: mir::ForeignAggregateType,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::NullableForeignAggregateExpression> {
    let nullable = mir::Type::NullableForeignAggregate(ty);
    if context.expression_is_null(expr)
        || matches!(
            context.constant_value(unparenthesized_place(expr, context)),
            Some(crate::const_eval::ConstValue::Null)
        )
    {
        return Ok(mir::NullableForeignAggregateExpression::Null(ty));
    }
    let present = |value| Ok(mir::NullableForeignAggregateExpression::Present(value));
    if context
        .constant_value(unparenthesized_place(expr, context))
        .is_some()
    {
        return present(aggregate_expression(expr, ty, context)?);
    }
    match unparenthesized_place(expr, context) {
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            if context.local_type(local) == nullable {
                return Ok(mir::NullableForeignAggregateExpression::Local { ty, local });
            }
        }
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
        } => {
            let (id, actual) = context.static_property(class_name, member, *span)?;
            if actual == nullable {
                return Ok(mir::NullableForeignAggregateExpression::Static { ty, id });
            }
        }
        place @ hir::Expr::PropertyAccess { .. } if stored_property_place(place, context) => {
            let (object, property, actual) = lower_property_place(place, context)?;
            if actual == nullable {
                return Ok(mir::NullableForeignAggregateExpression::Property {
                    ty,
                    object,
                    property,
                });
            }
            if actual == mir::Type::ForeignAggregate(ty) {
                return present(mir::ForeignAggregateExpression::Property {
                    ty,
                    object,
                    property,
                });
            }
        }
        _ => {}
    }
    if context.expression_type(expr)? == mir::Type::ForeignAggregate(ty) {
        return present(aggregate_expression(expr, ty, context)?);
    }
    if let Some(element) = element_rvalue(expr, nullable, context)? {
        let mir::Rvalue::NullableForeignAggregate(element) = element else {
            unreachable!("an element of `?Name` values is a nullable foreign aggregate value");
        };
        return Ok(element);
    }
    if let Some((function, args, span)) =
        aggregate_call(unparenthesized_place(expr, context), nullable, context)?
    {
        return Ok(mir::NullableForeignAggregateExpression::Call {
            ty,
            function,
            args,
            span,
        });
    }
    let value = if is_branching_expr(unparenthesized_place(expr, context)) {
        Some(lower_branching_rvalue(
            unparenthesized_place(expr, context),
            nullable,
            false,
            context,
        )?)
    } else {
        materialize_checked_rvalue(expr, nullable, false, context)?
    };
    match value {
        Some(mir::Rvalue::NullableForeignAggregate(value)) => Ok(value),
        _ => Err(vec![unsupported(
            expr.span(),
            "this nullable foreign struct or union value is not built yet",
        )]),
    }
}

/// The static property `class_name::member`, which holds a foreign struct
/// or union of type `ty`.
/// The static property `class_name::member`, which holds `expected` in
/// place.
fn static_place(
    class_name: &str,
    member: &str,
    span: Span,
    expected: mir::Type,
    context: &LoweringContext,
) -> DiagnosticResult<mir::StaticId> {
    let (id, actual) = context.static_property(class_name, member, span)?;
    if actual != expected {
        return Err(vec![unsupported(
            span,
            "a static property holds another type than its place expects",
        )]);
    }
    Ok(id)
}

/// The supplied fields of the constant foreign struct or union `value` of
/// type `expected` (Decision 0148 section 6): each one's offset in the
/// target's C layout, MIR type and constant value, in declaration order.
pub(super) fn constant_fields<'v>(
    value: &'v crate::const_eval::ConstForeignAggregateValue,
    expected: mir::ForeignAggregateType,
    aggregates: &HashMap<String, crate::foreign::ForeignAggregateFacts>,
    span: Span,
    resolve_type: &mut impl FnMut(&ResolvedType) -> Option<mir::Type>,
) -> DiagnosticResult<Vec<(u64, mir::Type, &'v crate::const_eval::ConstValue)>> {
    let mismatch = || {
        vec![unsupported(
            span,
            "a constant foreign struct or union does not match its checked layout",
        )]
    };
    let facts = aggregates.get(&value.aggregate.name).ok_or_else(mismatch)?;
    let layout = facts.layout.as_ref().ok_or_else(mismatch)?;
    let aggregate =
        ResolvedType::Foreign(Box::new(ForeignType::Aggregate(value.aggregate.clone())));
    if resolve_type(&aggregate) != Some(mir::Type::ForeignAggregate(expected))
        || value.fields.len() != facts.fields.len()
        || layout.fields.len() != facts.fields.len()
    {
        return Err(mismatch());
    }
    let mut fields = Vec::with_capacity(value.fields.len());
    for ((field, declared), placed) in value.fields.iter().zip(&facts.fields).zip(&layout.fields) {
        let Some(field) = field else {
            continue;
        };
        let ty = resolve_type(&declared.ty).ok_or_else(mismatch)?;
        fields.push((placed.offset, ty, field));
    }
    Ok(fields)
}

/// `new Name(...)` of a foreign struct or union (Decision 0148 section 6,
/// with the amendment of 2026-11-16): every byte zero, then the supplied
/// fields in source order, then each omitted struct field that declares a
/// default. An omitted field without one is a fixed-length array, which
/// stays zero-filled.
fn construct(
    class_type: &crate::types::TypeRef,
    args: &[hir::Argument],
    ty: mir::ForeignAggregateType,
    span: Span,
    context: &mut LoweringContext,
) -> DiagnosticResult<mir::ForeignAggregateExpression> {
    let Some(facts) = context
        .semantic_info
        .foreign_aggregates
        .get(&class_type.name)
        .cloned()
    else {
        return Err(vec![unsupported(
            span,
            "a construction of a foreign struct or union names no checked aggregate",
        )]);
    };
    let Some(layout) = facts.layout.clone() else {
        return Err(vec![unsupported(
            span,
            "a foreign struct or union without a C layout reached lowering",
        )]);
    };
    let mut supplied = vec![false; facts.fields.len()];
    let mut fields = Vec::with_capacity(facts.fields.len());
    for (position, argument) in args.iter().enumerate() {
        let index = match &argument.name {
            Some(name) => facts.field(&name.text).map(|(index, _)| index),
            None => Some(position),
        }
        .filter(|index| *index < facts.fields.len())
        .ok_or_else(|| {
            vec![unsupported(
                argument.span,
                "checking admitted an argument that names no foreign field",
            )]
        })?;
        supplied[index] = true;
        // A fixed-length array field is copied from a readonly region of
        // its element count (section 6, amended 2026-11-16).
        let (field_type, copied_elements) = match fixed_array(&facts.fields[index].ty) {
            Some((element, length)) => (
                copied_region(element, argument.span, context)?,
                Some(length),
            ),
            None => (
                mir_field_type(&facts.fields[index], argument.span, context)?,
                None,
            ),
        };
        let value = lower_rvalue_as_expected(&argument.value, field_type, context)?;
        fields.push(mir::ForeignFieldValue {
            offset: layout.fields[index].offset,
            value,
            copied_elements,
        });
    }
    for (index, field) in facts.fields.iter().enumerate() {
        let Some(default) = field.default.as_ref().filter(|_| !supplied[index]) else {
            continue;
        };
        let field_type = mir_field_type(field, span, context)?;
        let value = lower_const_rvalue(
            default,
            field_type,
            span,
            &context.semantic_info.enums,
            &context.semantic_info.foreign_aggregates,
            &mut |ty| context.mir_resolved_type(ty),
        )?;
        fields.push(mir::ForeignFieldValue {
            offset: layout.fields[index].offset,
            value,
            copied_elements: None,
        });
    }
    Ok(mir::ForeignAggregateExpression::Construct { ty, fields, span })
}

/// The readonly region type `ArrayRegion<element>` a fixed-length array
/// field is copied from, at construction or by assignment (section 6).
fn copied_region(
    element: &ResolvedType,
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<mir::Type> {
    let region = ResolvedType::ArrayRegion(
        crate::types::ByteRegionKind::Readonly,
        Box::new(element.clone()),
    );
    let Some(mir::Type::Region(region)) = context.mir_resolved_type(&region) else {
        return Err(vec![unsupported(
            span,
            "a fixed-length array field's region type has no MIR type",
        )]);
    };
    byte_region::built_element(region, span, context)?;
    Ok(mir::Type::Region(region))
}

fn mir_field_type(
    field: &crate::foreign::ForeignFieldFacts,
    span: Span,
    context: &LoweringContext,
) -> DiagnosticResult<mir::Type> {
    context.mir_resolved_type(&field.ty).ok_or_else(|| {
        vec![unsupported(
            span,
            "a fixed-length array field is not built yet",
        )]
    })
}
