//! Raw pointer operations and extern declaration bodies (Decision 0148
//! sections 4, 14 and 15).
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
                    let (local, offset) = match unparenthesized_place(argument) {
                        hir::Expr::Variable { name, span: place } => {
                            (context.lookup_local(name, *place)?, 0)
                        }
                        place => match field_place(place, context)? {
                            Some(field) => (field.aggregate, field.offset),
                            None => {
                                return Err(vec![crate::unsafe_code::unbuilt_form(
                                    *span,
                                    "Pointers To Properties Cannot Be Built Yet",
                                    format!(
                                        "`{}::{method}` of a property is part of Doria; this compiler does not build it yet",
                                        access.short_name()
                                    ),
                                )]);
                            }
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
        symbol: binding.symbol.clone(),
        library: binding.library.clone(),
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

/// A field of a foreign struct or union as a place: the local holding the
/// outermost aggregate, the field's byte offset in it, and the field's type.
struct FieldPlace {
    aggregate: mir::LocalId,
    offset: u64,
    ty: mir::Type,
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
/// An aggregate that is not a local's whole value, such as a call result,
/// is first copied to a temporary; writes reach only locals, which checking
/// proves writable.
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
    let ty = context.mir_resolved_type(&field_type).ok_or_else(|| {
        vec![unsupported(
            *span,
            "a fixed-length array field is not built yet",
        )]
    })?;
    if let Some(outer) = field_place(object, context)? {
        return Ok(Some(FieldPlace {
            aggregate: outer.aggregate,
            offset: outer.offset + field_offset,
            ty,
        }));
    }
    let aggregate = match unparenthesized_place(object) {
        hir::Expr::Variable { name, span } => context.lookup_local(name, *span)?,
        _ => {
            let aggregate = aggregate_type(&object_type, object.span(), context)?;
            let value =
                lower_rvalue_as_expected(object, mir::Type::ForeignAggregate(aggregate), context)?;
            let local = context.declare_return_temp(mir::Type::ForeignAggregate(aggregate), false);
            context.push_statement(mir::Statement::AssignLocal {
                target: local,
                value,
            });
            local
        }
    };
    if !matches!(
        context.local_type(aggregate),
        mir::Type::ForeignAggregate(_)
    ) {
        return Err(vec![unsupported(
            *span,
            "a field access names a local that holds no foreign struct or union",
        )]);
    }
    Ok(Some(FieldPlace {
        aggregate,
        offset: field_offset,
        ty,
    }))
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
    let target = context.declare_return_temp(place.ty, false);
    context.push_statement(mir::Statement::ForeignField {
        aggregate: place.aggregate,
        offset: place.offset,
        access: mir::ForeignFieldAccess::Read { target },
        span: expr.span(),
    });
    convert_call_result(target, place.ty, expected, false, expr.span(), context).map(Some)
}

/// `$aggregate->field = $value`, a compound assignment to a field, and
/// `$aggregate->field++`; `false` when `target` is not a field of a foreign
/// struct or union. A compound form reads the field before it evaluates
/// the right-hand side, as every read-modify-write place does.
pub(super) fn field_write(
    target: &hir::Expr,
    update: FieldUpdate<'_>,
    context: &mut LoweringContext,
) -> DiagnosticResult<bool> {
    let Some(place) = field_place(target, context)? else {
        return Ok(false);
    };
    let value = match update {
        FieldUpdate::Assign(value) => lower_rvalue_as_expected(value, place.ty, context)?,
        FieldUpdate::Compound { .. } | FieldUpdate::Increment { .. } => {
            let mir::Type::Scalar(scalar) = place.ty else {
                return Err(vec![unsupported(
                    target.span(),
                    "a read-modify-write of a foreign field needs a scalar field",
                )]);
            };
            let current = context.declare_return_temp(place.ty, false);
            context.push_statement(mir::Statement::ForeignField {
                aggregate: place.aggregate,
                offset: place.offset,
                access: mir::ForeignFieldAccess::Read { target: current },
                span: target.span(),
            });
            let operand = mir::Operand::Local(current);
            mir::Rvalue::Value(match update {
                FieldUpdate::Compound { op, value, span } => {
                    lower_compound_value(operand, scalar, op, value, span, context)?
                }
                FieldUpdate::Increment { op, span } => {
                    lower_increment_value(operand, scalar, op, span)?
                }
                FieldUpdate::Assign(_) => unreachable!("handled above"),
            })
        }
    };
    context.push_statement(mir::Statement::ForeignField {
        aggregate: place.aggregate,
        offset: place.offset,
        access: mir::ForeignFieldAccess::Write { value },
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
    match unparenthesized_place(expr) {
        hir::Expr::New {
            class_type,
            args,
            span,
            ..
        } => construct(class_type, args, ty, *span, context),
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
        let field_type = mir_field_type(&facts.fields[index], argument.span, context)?;
        let value = lower_rvalue_as_expected(&argument.value, field_type, context)?;
        fields.push(mir::ForeignFieldValue {
            offset: layout.fields[index].offset,
            value,
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
            &mut |ty| context.mir_resolved_type(ty),
        )?;
        fields.push(mir::ForeignFieldValue {
            offset: layout.fields[index].offset,
            value,
        });
    }
    Ok(mir::ForeignAggregateExpression::Construct { ty, fields, span })
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
