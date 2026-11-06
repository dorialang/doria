use super::*;

pub(super) const fn is_type(ty: mir::Type) -> bool {
    matches!(
        ty,
        mir::Type::ByteRegion(_) | mir::Type::NullableByteRegion(_)
    )
}

pub(super) fn expression(
    expr: &hir::Expr,
    kind: mir::ByteRegionKind,
    transfer: bool,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::ByteRegionExpression> {
    let transfer = transfer && kind.is_writable();
    if let Some(mir::Rvalue::ByteRegion(value)) =
        materialize_checked_rvalue(expr, mir::Type::ByteRegion(kind), transfer, context)?
    {
        return Ok(value);
    }
    if is_branching_expr(expr) {
        let mir::Rvalue::ByteRegion(value) =
            lower_branching_rvalue(expr, mir::Type::ByteRegion(kind), transfer, context)?
        else {
            unreachable!("region branch has the checked region type")
        };
        return Ok(value);
    }
    if let Some((collection, index, ty)) = lower_list_remove_at(expr, context)? {
        if ty != mir::Type::ByteRegion(kind) {
            return Err(vec![unsupported(
                expr.span(),
                "region removal changes its checked type",
            )]);
        }
        return Ok(mir::ByteRegionExpression::CollectionIndex {
            kind,
            collection,
            index: Box::new(index),
            index_span: expr.span(),
            positional: false,
            remove: true,
        });
    }
    match expr {
        hir::Expr::Grouped { expr, .. } => expression(expr, kind, transfer, context),
        hir::Expr::Index {
            collection, index, ..
        } => {
            let (collection, index) = lower_collection_index_operand(
                collection,
                index,
                mir::Type::ByteRegion(kind),
                context,
            )?;
            Ok(mir::ByteRegionExpression::CollectionIndex {
                kind,
                collection,
                index: Box::new(index),
                index_span: expr.span(),
                positional: false,
                remove: false,
            })
        }
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::ByteRegion(actual) if actual == kind => {
                    Ok(mir::ByteRegionExpression::Local {
                        kind,
                        local,
                        transfer,
                    })
                }
                mir::Type::NullableByteRegion(actual) if actual == kind => {
                    Ok(mir::ByteRegionExpression::AssumePresent {
                        kind,
                        value: Box::new(mir::NullableByteRegionExpression::Local {
                            kind,
                            local,
                            transfer,
                        }),
                    })
                }
                _ => Err(vec![unsupported(
                    *span,
                    "byte region local has another checked type",
                )]),
            }
        }
        // `$text->byteRegion` (decision log entry 49): the whole string as a
        // readonly region. The receiver stays a borrowed place, so the view
        // retains it exactly as a `Bytes` region retains its storage.
        hir::Expr::PropertyAccess {
            object,
            property,
            null_safe: false,
            take: false,
            span,
        } if property == "byteRegion" && context.expression_type(object)? == mir::Type::String => {
            if kind.is_writable() {
                return Err(vec![unsupported(
                    *span,
                    "a string's byte region is readonly",
                )]);
            }
            let source = lower_call_argument(
                object,
                mir::Type::String,
                mir::FunctionParameterMode::Readonly,
                context,
            )?;
            let int = IntegerType::Int64;
            let length = mir::Rvalue::Value(mir::ValueExpression::Integer(
                mir::IntegerExpression::use_operand(
                    int,
                    mir::Operand::StringIntrinsic(Box::new(mir::StringIntrinsicCall {
                        kind: mir::StringIntrinsicKind::ByteLength,
                        args: vec![source.clone()],
                        result: mir::Type::Scalar(mir::ScalarType::Integer(int)),
                        span: *span,
                        argument_spans: vec![object.span()],
                    })),
                ),
            ));
            let offset = mir::Rvalue::Value(mir::ValueExpression::Integer(
                mir::IntegerExpression::constant(crate::numeric::IntegerValue::from_bits(int, 0)),
            ));
            Ok(mir::ByteRegionExpression::Region {
                kind,
                source: Box::new(source),
                offset: Box::new(offset),
                length: Box::new(length),
                span: *span,
            })
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe: false,
        } if matches!(method.as_str(), "getRegion" | "getWritableRegion")
            && matches!(
                context.expression_type(object)?,
                mir::Type::ByteRegion(_) | mir::Type::Collection(_)
            ) =>
        {
            let source_type = context.expression_type(object)?;
            // Use the common argument place/temporary machinery so receiver
            // evaluation precedes arguments without losing its access mode.
            let source = lower_call_argument(
                object,
                source_type,
                if kind.is_writable() {
                    mir::FunctionParameterMode::Writable
                } else {
                    mir::FunctionParameterMode::Readonly
                },
                context,
            )?;
            form(kind, source, args, *span, context)
        }
        _ => {
            let (signature, args) = call(expr, context)?;
            Ok(mir::ByteRegionExpression::Call {
                span: expr.span(),
                kind,
                function: signature.id,
                args,
                return_borrow: signature.return_borrow,
            })
        }
    }
}

pub(super) fn nullable_expression(
    expr: &hir::Expr,
    kind: mir::ByteRegionKind,
    transfer: bool,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::NullableByteRegionExpression> {
    let transfer = transfer && kind.is_writable();
    if let Some(mir::Rvalue::NullableByteRegion(value)) =
        materialize_checked_rvalue(expr, mir::Type::NullableByteRegion(kind), transfer, context)?
    {
        return Ok(value);
    }
    if is_branching_expr(expr) {
        let mir::Rvalue::NullableByteRegion(value) =
            lower_branching_rvalue(expr, mir::Type::NullableByteRegion(kind), transfer, context)?
        else {
            unreachable!("nullable region branch has the checked region type")
        };
        return Ok(value);
    }
    if let Some((collection, key, stored, access)) =
        lower_collection_nullable_property(expr, context)?
    {
        if !nullable_collection_value_matches(stored, mir::Type::ByteRegion(kind)) {
            return Err(vec![unsupported(
                expr.span(),
                "nullable region access changes its checked type",
            )]);
        }
        return Ok(mir::NullableByteRegionExpression::CollectionGet {
            kind,
            collection,
            key: Box::new(key),
            index_span: expr.span(),
            access,
        });
    }
    if let hir::Expr::MethodCall {
        object,
        method,
        args,
        null_safe: false,
        ..
    } = unparenthesized_place(expr)
    {
        if let Some((collection, key, stored, access)) =
            lower_dictionary_get(object, method, args, context)?
        {
            if !nullable_collection_value_matches(stored, mir::Type::ByteRegion(kind)) {
                return Err(vec![unsupported(
                    expr.span(),
                    "nullable region access changes its checked type",
                )]);
            }
            return Ok(mir::NullableByteRegionExpression::CollectionGet {
                kind,
                collection,
                key: Box::new(key),
                index_span: expr.span(),
                access,
            });
        }
    }
    if let Some((collection, index, ty)) = lower_list_remove_at(expr, context)? {
        let value = collection_remove_at_rvalue(collection, index, ty, expr.span())?;
        if let mir::Rvalue::NullableByteRegion(value) = value {
            return Ok(value);
        }
        if let mir::Rvalue::ByteRegion(value) = value {
            return Ok(mir::NullableByteRegionExpression::Present(value));
        }
        return Err(vec![unsupported(
            expr.span(),
            "nullable region removal changes its checked type",
        )]);
    }
    match expr {
        hir::Expr::Null { .. } => Ok(mir::NullableByteRegionExpression::Null { kind }),
        hir::Expr::Grouped { expr, .. } => nullable_expression(expr, kind, transfer, context),
        hir::Expr::Index {
            collection, index, ..
        } if context.expression_type(expr)? == mir::Type::NullableByteRegion(kind) => {
            let (collection, index) = lower_collection_index_operand(
                collection,
                index,
                mir::Type::NullableByteRegion(kind),
                context,
            )?;
            Ok(mir::NullableByteRegionExpression::CollectionIndex {
                kind,
                collection,
                index: Box::new(index),
                index_span: expr.span(),
                positional: false,
                remove: false,
            })
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe: true,
        } if matches!(method.as_str(), "getRegion" | "getWritableRegion")
            && matches!(
                context.expression_type(object)?,
                mir::Type::NullableByteRegion(_) | mir::Type::NullableCollection(_)
            ) =>
        {
            let source_type = context.expression_type(object)?;
            let present_type = non_null_match_type(source_type).0;
            let source = lower_rvalue_as_borrowed(object, source_type, context)?;
            let selection = context.coalesce_selection(object);
            let writable = context
                .semantic_info
                .writable_object_paths
                .contains(&object.span());
            let result = materialize_null_safe_call(
                source,
                selection,
                writable,
                None,
                *span,
                Some((mir::Type::NullableByteRegion(kind), transfer)),
                |source, context| {
                    let source = narrowed_match_local_rvalue(
                        source,
                        source_type,
                        present_type,
                        false,
                        *span,
                        context,
                    )?;
                    let value = mir::Rvalue::ByteRegion(form(kind, source, args, *span, context)?);
                    let (local, ty) = core_value::materialize_value(value, context);
                    Ok(Some((local, ty, kind.is_writable())))
                },
                context,
            )?
            .expect("null-safe region operation has a value");
            let mir::Rvalue::NullableByteRegion(value) = local_rvalue(result.0, result.1, result.2)
            else {
                unreachable!()
            };
            Ok(value)
        }
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::NullableByteRegion(actual) if actual == kind => {
                    Ok(mir::NullableByteRegionExpression::Local {
                        kind,
                        local,
                        transfer,
                    })
                }
                mir::Type::ByteRegion(actual) if actual == kind => Ok(
                    mir::NullableByteRegionExpression::Present(mir::ByteRegionExpression::Local {
                        kind,
                        local,
                        transfer,
                    }),
                ),
                _ => Err(vec![unsupported(
                    *span,
                    "nullable byte region local has another checked type",
                )]),
            }
        }
        _ if context.expression_type(expr)? == mir::Type::ByteRegion(kind) => {
            expression(expr, kind, transfer, context)
                .map(mir::NullableByteRegionExpression::Present)
        }
        _ => {
            let (signature, args) = call(expr, context)?;
            Ok(mir::NullableByteRegionExpression::Call {
                span: expr.span(),
                kind,
                function: signature.id,
                args,
                return_borrow: signature.return_borrow,
            })
        }
    }
}

/// `WritableByteRegion::copyFrom`. The receiver is evaluated first and
/// borrowed writable for the copy only; the source is a readonly region
/// argument. Neither region outlives the expression.
pub(super) fn copy_from(
    expr: &hir::Expr,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<Option<mir::IntegerExpression>> {
    let hir::Expr::MethodCall {
        object,
        method,
        args,
        span,
        null_safe: false,
    } = expr
    else {
        return Ok(None);
    };
    let writable = mir::Type::ByteRegion(mir::ByteRegionKind::Writable);
    if method != "copyFrom" || context.expression_type(object)? != writable {
        return Ok(None);
    }
    let [source] = argument_values(args)[..] else {
        return Err(vec![unsupported(
            *span,
            "copyFrom expects one source region",
        )]);
    };
    let destination = lower_call_argument(
        object,
        writable,
        mir::FunctionParameterMode::Writable,
        context,
    )?;
    let source = lower_call_argument(
        source,
        mir::Type::ByteRegion(mir::ByteRegionKind::Readonly),
        mir::FunctionParameterMode::Readonly,
        context,
    )?;
    Ok(Some(mir::IntegerExpression::CopyByteRegion {
        destination: Box::new(destination),
        source: Box::new(source),
        span: *span,
    }))
}

/// `WritableByteRegion::copyWithin(offset, length, destination)`: one
/// statement over the writable receiver local, so validation sees a single
/// write; the backend checks both ranges and moves the bytes with memmove.
pub(super) fn copy_within(
    expr: &hir::Expr,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<bool> {
    let hir::Expr::MethodCall {
        object,
        method,
        args,
        span,
        null_safe: false,
    } = expr
    else {
        return Ok(false);
    };
    if method != "copyWithin"
        || context.expression_type(object)? != mir::Type::ByteRegion(mir::ByteRegionKind::Writable)
    {
        return Ok(false);
    }
    let region = local(object, true, context)?;
    let names = args
        .iter()
        .map(|arg| arg.name.as_ref().map(|name| name.text.as_str()))
        .collect::<Vec<_>>();
    let bound = crate::arg_binding::bind_arguments(
        &["offset", "length", "destination"],
        &[false, false, false],
        &names,
    );
    let mut operands = [None, None, None];
    let int_type = mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64));
    for (position, argument) in args.iter().enumerate() {
        let Some(parameter) = bound.arg_to_param[position] else {
            return Err(vec![unsupported(
                *span,
                "copyWithin argument has no checked parameter",
            )]);
        };
        let value = lower_rvalue_as_expected(&argument.value, int_type, context)?;
        let (local, ty) = core_value::materialize_value(value, context);
        operands[parameter] = Some(local_rvalue(local, ty, false));
    }
    let [Some(offset), Some(length), Some(destination)] = operands else {
        return Err(vec![unsupported(
            *span,
            "copyWithin requires offset, length and destination arguments",
        )]);
    };
    context.push_statement(mir::Statement::CopyWithinByteRegion {
        region,
        offset,
        length,
        destination,
        span: *span,
    });
    Ok(true)
}

fn form(
    kind: mir::ByteRegionKind,
    source: mir::Rvalue,
    args: &[hir::Argument],
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::ByteRegionExpression> {
    let names = args
        .iter()
        .map(|arg| arg.name.as_ref().map(|name| name.text.as_str()))
        .collect::<Vec<_>>();
    let bound = crate::arg_binding::bind_arguments(&["offset", "length"], &[false, false], &names);
    let mut operands = [None, None];
    let int_type = mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64));
    for (position, argument) in args.iter().enumerate() {
        let Some(parameter) = bound.arg_to_param[position] else {
            return Err(vec![unsupported(
                span,
                "byte region argument has no checked parameter",
            )]);
        };
        let value = lower_rvalue_as_expected(&argument.value, int_type, context)?;
        let (local, ty) = core_value::materialize_value(value, context);
        operands[parameter] = Some(local_rvalue(local, ty, false));
    }
    let [Some(offset), Some(length)] = operands else {
        return Err(vec![unsupported(
            span,
            "byte region requires checked offset and length arguments",
        )]);
    };
    Ok(mir::ByteRegionExpression::Region {
        kind,
        source: Box::new(source),
        offset: Box::new(offset),
        length: Box::new(length),
        span,
    })
}

pub(super) fn nullable_length(
    expr: &hir::Expr,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<Option<mir::NullableScalarExpression>> {
    let hir::Expr::PropertyAccess {
        object,
        property,
        null_safe: true,
        span,
        ..
    } = unparenthesized_place(expr)
    else {
        return Ok(None);
    };
    let mir::Type::NullableByteRegion(kind) = context.expression_type(object)? else {
        return Ok(None);
    };
    if property != "length" {
        return Ok(None);
    }
    let ty = mir::ScalarType::Integer(IntegerType::Int64);
    let source =
        mir::Rvalue::NullableByteRegion(nullable_expression(object, kind, false, context)?);
    let selection = context.coalesce_selection(object);
    let result = materialize_null_safe_call(
        source,
        selection,
        false,
        None,
        *span,
        Some((mir::Type::NullableScalar(ty), false)),
        |source, context| {
            let present = mir::Rvalue::ByteRegion(mir::ByteRegionExpression::AssumePresent {
                kind,
                value: Box::new(mir::NullableByteRegionExpression::Local {
                    kind,
                    local: source,
                    transfer: false,
                }),
            });
            let (source, _) = core_value::materialize_value(present, context);
            let value = mir::Rvalue::Value(value_expression_from_operand(
                ty,
                mir::Operand::ByteRegionLength(source),
            ));
            let (local, ty) = core_value::materialize_value(value, context);
            Ok(Some((local, ty, false)))
        },
        context,
    )?
    .expect("nullable region length has a value");
    Ok(Some(mir::NullableScalarExpression::Local {
        ty,
        local: result.0,
    }))
}

fn call(
    expr: &hir::Expr,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<(FunctionSignature, Vec<mir::Rvalue>)> {
    match expr {
        hir::Expr::FunctionCall { name, args, span } => {
            let signature = context.lookup_function(name, *span)?;
            let args =
                lower_call_args_with_ownership(name, args, signature.clone(), *span, context)?;
            Ok((signature, args))
        }
        hir::Expr::MethodCall {
            object,
            method,
            args,
            span,
            null_safe: false,
        } => lower_instance_method_call(object, method, args, *span, context),
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
            ..
        } => lower_static_method_call(class_name, method, args, *span, context),
        _ => Err(vec![unsupported(
            expr.span(),
            "checked byte region expression has no MIR lowering",
        )]),
    }
}

pub(super) fn local(
    expr: &hir::Expr,
    writable: bool,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::LocalId> {
    let mir::Type::ByteRegion(kind) = context.expression_type(expr)? else {
        return Err(vec![unsupported(
            expr.span(),
            "byte region access requires a present region",
        )]);
    };
    if let hir::Expr::Variable { name, span } = unparenthesized_place(expr) {
        let local = context.lookup_local(name, *span)?;
        if context.local_type(local) == mir::Type::ByteRegion(kind) {
            return Ok(local);
        }
    }
    let value = mir::Rvalue::ByteRegion(expression(expr, kind, false, context)?);
    let (local, _) = core_value::materialize_value(value, context);
    context.locals[local.0].writable = writable;
    Ok(local)
}
