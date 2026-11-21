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
                        return Err(vec![unsupported(
                            *span,
                            "a pointer offset has one argument",
                        )]);
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
                // An element's alignment is its stride: the typed array
                // stores each element at its own width.
                let align = byte_region::built_element(region_type, *span, context)?
                    .map_or(1, |(_, stride)| u64::from(stride));
                let region = byte_region::local(object, false, context)?;
                let target = context.declare_return_temp(POINTER, false);
                context.push_statement(mir::Statement::Pointer {
                    target: Some(target),
                    operation: mir::PointerOperation::RegionData { region, align },
                    span: *span,
                });
                return Ok(Some(local_rvalue(target, POINTER, false)));
            }
            let Some((access, _)) = pointer_type(object, context) else {
                return Ok(None);
            };
            if pointer_intrinsics::instance_member(access, property).map(|member| member.operation)
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
                    let ty = context.expression_type(unparenthesized_place(expr))?;
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
                    // A field of an aggregate a property holds lives in the
                    // object's storage, as the property does.
                    let property_pointer = || {
                        vec![crate::unsafe_code::unbuilt_form(
                            *span,
                            "Pointers To Properties Cannot Be Built Yet",
                            format!(
                                "`{}::{method}` of a property is part of Doria; this compiler does not build it yet",
                                access.short_name()
                            ),
                        )]
                    };
                    let (local, offset) = match unparenthesized_place(argument) {
                        hir::Expr::Variable { name, span: place } => {
                            (context.lookup_local(name, *place)?, 0)
                        }
                        place => match field_place(place, context)? {
                            Some(FieldPlace {
                                aggregate: mir::ForeignAggregatePlace::Local(local),
                                offset,
                                ..
                            }) => (local, offset),
                            Some(_) | None => return Err(property_pointer()),
                        },
                    };
                    let target = context.declare_return_temp(POINTER, false);
                    context.push_statement(mir::Statement::Pointer {
                        target: Some(target),
                        operation: mir::PointerOperation::AddressOf { local, offset },
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
    aggregate: mir::ForeignAggregatePlace,
    offset: u64,
    field: FieldKind,
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
    context: &mut LoweringContext,
) -> DiagnosticResult<Option<FieldPlace>> {
    let hir::Expr::PropertyAccess {
        object,
        property,
        null_safe,
        span,
        ..
    } = unparenthesized_place(expr)
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
            "a nullable foreign struct or union is not built yet",
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
    if let Some(outer) = field_place(object, context)? {
        return Ok(Some(FieldPlace {
            aggregate: outer.aggregate,
            offset: outer.offset + field_offset,
            field,
        }));
    }
    let aggregate_type = aggregate_type(&object_type, object.span(), context)?;
    let aggregate = match unparenthesized_place(object) {
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            if context.local_type(local) != mir::Type::ForeignAggregate(aggregate_type) {
                return Err(vec![unsupported(
                    *span,
                    "a field access names a local that holds no foreign struct or union",
                )]);
            }
            mir::ForeignAggregatePlace::Local(local)
        }
        place @ hir::Expr::PropertyAccess { .. } if stored_property_place(place, context) => {
            let (object, property, ty) = lower_property_place(place, context)?;
            if ty != mir::Type::ForeignAggregate(aggregate_type) {
                return Err(vec![unsupported(
                    place.span(),
                    "a field access names a property that holds no foreign struct or union",
                )]);
            }
            mir::ForeignAggregatePlace::Property { object, property }
        }
        hir::Expr::StaticMember {
            class_name,
            member,
            span,
        } if context.constant_value(object).is_none() => mir::ForeignAggregatePlace::Static(
            static_aggregate(class_name, member, *span, aggregate_type, context)?,
        ),
        _ => {
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
            mir::ForeignAggregatePlace::Local(local)
        }
    };
    Ok(Some(FieldPlace {
        aggregate,
        offset: field_offset,
        field,
    }))
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
    let Some(place) = field_place(expr, context)? else {
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
    let Some(place) = field_place(target, context)? else {
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
    if let Some(value) = context.constant_value(unparenthesized_place(expr)).cloned() {
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
    match unparenthesized_place(expr) {
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
            id: static_aggregate(class_name, member, *span, ty, context)?,
        }),
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            if context.local_type(local) != mir::Type::ForeignAggregate(ty) {
                return Err(vec![unsupported(
                    *span,
                    "a foreign struct or union local has another type",
                )]);
            }
            Ok(mir::ForeignAggregateExpression::Local { ty, local })
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
        hir::Expr::FunctionCall { name, args, span } => {
            let signature = context.lookup_function(name, *span)?;
            returns_aggregate(signature.return_type, ty, *span)?;
            Ok(mir::ForeignAggregateExpression::Call {
                ty,
                function: signature.id,
                args: lower_call_args(name, args, signature, *span, context)?,
                span: *span,
            })
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
            returns_aggregate(signature.return_type, ty, *span)?;
            Ok(mir::ForeignAggregateExpression::Call {
                ty,
                function: signature.id,
                args,
                span: *span,
            })
        }
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } => {
            let (signature, args) =
                lower_static_method_call(class_name, method, args, *span, context)?;
            returns_aggregate(signature.return_type, ty, *span)?;
            Ok(mir::ForeignAggregateExpression::Call {
                ty,
                function: signature.id,
                args,
                span: *span,
            })
        }
        other => Err(vec![unsupported(
            other.span(),
            "this foreign struct or union value is not built yet",
        )]),
    }
}

/// The static property `class_name::member`, which holds a foreign struct
/// or union of type `ty`.
fn static_aggregate(
    class_name: &str,
    member: &str,
    span: Span,
    ty: mir::ForeignAggregateType,
    context: &LoweringContext,
) -> DiagnosticResult<mir::StaticId> {
    let (id, actual) = context.static_property(class_name, member, span)?;
    if actual != mir::Type::ForeignAggregate(ty) {
        return Err(vec![unsupported(
            span,
            "a static property holds another type than the foreign struct or union expected",
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

fn returns_aggregate(
    return_type: mir::ReturnType,
    ty: mir::ForeignAggregateType,
    span: Span,
) -> DiagnosticResult<()> {
    if return_type == mir::ReturnType::Value(mir::Type::ForeignAggregate(ty)) {
        Ok(())
    } else {
        Err(vec![unsupported(
            span,
            "a call returns another type than the foreign struct or union expected",
        )])
    }
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
