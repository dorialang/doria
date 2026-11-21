use super::*;

pub(super) const fn is_type(ty: mir::Type) -> bool {
    matches!(ty, mir::Type::Region(_) | mir::Type::NullableRegion(_))
}

pub(super) fn expression(
    expr: &hir::Expr,
    kind: mir::RegionType,
    transfer: bool,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::RegionExpression> {
    let transfer = transfer && kind.is_writable();
    if let Some(mir::Rvalue::Region(value)) =
        materialize_checked_rvalue(expr, mir::Type::Region(kind), transfer, context)?
    {
        return Ok(value);
    }
    if is_branching_expr(expr) {
        let mir::Rvalue::Region(value) =
            lower_branching_rvalue(expr, mir::Type::Region(kind), transfer, context)?
        else {
            unreachable!("region branch has the checked region type")
        };
        return Ok(value);
    }
    if let Some((collection, index, ty)) = lower_list_remove_at(expr, context)? {
        if ty != mir::Type::Region(kind) {
            return Err(vec![unsupported(
                expr.span(),
                "region removal changes its checked type",
            )]);
        }
        return Ok(mir::RegionExpression::CollectionIndex {
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
                mir::Type::Region(kind),
                context,
            )?;
            Ok(mir::RegionExpression::CollectionIndex {
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
                mir::Type::Region(actual) if actual == kind => Ok(mir::RegionExpression::Local {
                    kind,
                    local,
                    transfer,
                }),
                mir::Type::NullableRegion(actual) if actual == kind => {
                    Ok(mir::RegionExpression::AssumePresent {
                        kind,
                        value: Box::new(mir::NullableRegionExpression::Local {
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
            Ok(mir::RegionExpression::Region {
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
                mir::Type::Region(_) | mir::Type::Collection(_)
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
        hir::Expr::StaticCall {
            class_name,
            method,
            args,
            span,
        } if method == crate::byte_intrinsics::FROM_POINTER
            && crate::byte_intrinsics::RegionCompanion::from_class_name(class_name).is_some() =>
        {
            from_pointer(kind, args, *span, context)
        }
        _ => {
            let (signature, args) = call(expr, context)?;
            Ok(mir::RegionExpression::Call {
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
    kind: mir::RegionType,
    transfer: bool,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::NullableRegionExpression> {
    let transfer = transfer && kind.is_writable();
    if let Some(mir::Rvalue::NullableRegion(value)) =
        materialize_checked_rvalue(expr, mir::Type::NullableRegion(kind), transfer, context)?
    {
        return Ok(value);
    }
    if is_branching_expr(expr) {
        let mir::Rvalue::NullableRegion(value) =
            lower_branching_rvalue(expr, mir::Type::NullableRegion(kind), transfer, context)?
        else {
            unreachable!("nullable region branch has the checked region type")
        };
        return Ok(value);
    }
    if let Some((collection, key, stored, access)) =
        lower_collection_nullable_property(expr, context)?
    {
        if !nullable_collection_value_matches(stored, mir::Type::Region(kind)) {
            return Err(vec![unsupported(
                expr.span(),
                "nullable region access changes its checked type",
            )]);
        }
        return Ok(mir::NullableRegionExpression::CollectionGet {
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
            if !nullable_collection_value_matches(stored, mir::Type::Region(kind)) {
                return Err(vec![unsupported(
                    expr.span(),
                    "nullable region access changes its checked type",
                )]);
            }
            return Ok(mir::NullableRegionExpression::CollectionGet {
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
        if let mir::Rvalue::NullableRegion(value) = value {
            return Ok(value);
        }
        if let mir::Rvalue::Region(value) = value {
            return Ok(mir::NullableRegionExpression::Present(value));
        }
        return Err(vec![unsupported(
            expr.span(),
            "nullable region removal changes its checked type",
        )]);
    }
    match expr {
        hir::Expr::Null { .. } => Ok(mir::NullableRegionExpression::Null { kind }),
        hir::Expr::Grouped { expr, .. } => nullable_expression(expr, kind, transfer, context),
        hir::Expr::Index {
            collection, index, ..
        } if context.expression_type(expr)? == mir::Type::NullableRegion(kind) => {
            let (collection, index) = lower_collection_index_operand(
                collection,
                index,
                mir::Type::NullableRegion(kind),
                context,
            )?;
            Ok(mir::NullableRegionExpression::CollectionIndex {
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
                mir::Type::NullableRegion(_) | mir::Type::NullableCollection(_)
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
                Some((mir::Type::NullableRegion(kind), transfer)),
                |source, context| {
                    let source = narrowed_match_local_rvalue(
                        source,
                        source_type,
                        present_type,
                        None,
                        false,
                        *span,
                        context,
                    )?;
                    let value = mir::Rvalue::Region(form(kind, source, args, *span, context)?);
                    let (local, ty) = core_value::materialize_value(value, context);
                    Ok(Some((local, ty, kind.is_writable())))
                },
                context,
            )?
            .expect("null-safe region operation has a value");
            let mir::Rvalue::NullableRegion(value) = local_rvalue(result.0, result.1, result.2)
            else {
                unreachable!()
            };
            Ok(value)
        }
        hir::Expr::Variable { name, span } => {
            let local = context.lookup_local(name, *span)?;
            match context.local_type(local) {
                mir::Type::NullableRegion(actual) if actual == kind => {
                    Ok(mir::NullableRegionExpression::Local {
                        kind,
                        local,
                        transfer,
                    })
                }
                mir::Type::Region(actual) if actual == kind => Ok(
                    mir::NullableRegionExpression::Present(mir::RegionExpression::Local {
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
        _ if context.expression_type(expr)? == mir::Type::Region(kind) => {
            expression(expr, kind, transfer, context).map(mir::NullableRegionExpression::Present)
        }
        _ => {
            let (signature, args) = call(expr, context)?;
            // A constrained call can select an implementation that narrows the
            // requirement's nullable region to a present one.
            if signature.return_type == mir::ReturnType::Value(mir::Type::Region(kind)) {
                return Ok(mir::NullableRegionExpression::Present(
                    mir::RegionExpression::Call {
                        span: expr.span(),
                        kind,
                        function: signature.id,
                        args,
                        return_borrow: signature.return_borrow,
                    },
                ));
            }
            Ok(mir::NullableRegionExpression::Call {
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
    // A byte region or a typed array region; the source views the same
    // elements with readonly access.
    let destination_type = match context.expression_type(object)? {
        mir::Type::Region(region) if method == "copyFrom" && region.is_writable() => region,
        _ => return Ok(None),
    };
    built_element(destination_type, *span, context)?;
    let writable = mir::Type::Region(destination_type);
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
        mir::Type::Region(destination_type.with_access(mir::ByteRegionKind::Readonly)),
        mir::FunctionParameterMode::Readonly,
        context,
    )?;
    Ok(Some(mir::IntegerExpression::CopyRegion {
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
        || context.expression_type(object)?
            != mir::Type::Region(mir::RegionType {
                access: mir::ByteRegionKind::Writable,
                array: None,
            })
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
    kind: mir::RegionType,
    source: mir::Rvalue,
    args: &[hir::Argument],
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::RegionExpression> {
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
    built_element(kind, span, context)?;
    Ok(mir::RegionExpression::Region {
        kind,
        source: Box::new(source),
        offset: Box::new(offset),
        length: Box::new(length),
        span,
    })
}

/// `ByteRegion::fromPointer($owner, $pointer, $length)` and its siblings
/// (Decision 0148 section 10): a pointer operation checks the range, and
/// the region over it borrows the owner, a checked place, exactly as a
/// region borrows its source.
fn from_pointer(
    kind: mir::RegionType,
    args: &[hir::Argument],
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::RegionExpression> {
    let names = args
        .iter()
        .map(|arg| arg.name.as_ref().map(|name| name.text.as_str()))
        .collect::<Vec<_>>();
    let bound = crate::byte_intrinsics::from_pointer_binding(&names);
    let [Some(owner), Some(pointer), Some(length)] = bound.param_to_arg[..] else {
        return Err(vec![unsupported(
            span,
            "a region over foreign memory has checked owner, pointer and length arguments",
        )]);
    };
    let owner_value = &args[owner].value;
    let owner = lower_call_argument(
        owner_value,
        context.expression_type(owner_value)?,
        if kind.is_writable() {
            mir::FunctionParameterMode::Writable
        } else {
            mir::FunctionParameterMode::Readonly
        },
        context,
    )?;
    // Every element a region over foreign memory views is FFI-safe: a
    // scalar is stored at its own width and aligned to it, and a foreign
    // struct or union at its C size and alignment.
    let (stride, align) = match built_element(kind, span, context)? {
        None => (1, 1),
        Some((element, stride)) => (
            stride as u64,
            crate::native_abi::array_region_alignment(element).unwrap_or(stride) as u64,
        ),
    };
    let pointer = super::foreign::materialize_pointer(&args[pointer].value, context)?;
    let int = IntegerType::Int64;
    let length = lower_integer_expression(&args[length].value, context)?;
    let length_local = context.declare_temp(false, int);
    context.push_statement(mir::Statement::AssignLocal {
        target: length_local,
        value: mir::Rvalue::Value(mir::ValueExpression::Integer(length)),
    });
    let data = context.declare_return_temp(super::foreign::POINTER, false);
    context.push_statement(mir::Statement::Pointer {
        target: Some(data),
        operation: mir::PointerOperation::ForeignRange {
            pointer,
            length: mir::IntegerExpression::use_operand(int, mir::Operand::Local(length_local)),
            stride,
            align,
        },
        span,
    });
    Ok(mir::RegionExpression::Foreign {
        kind,
        owner: Box::new(owner),
        data,
        length: length_local,
    })
}

/// The scalar type of the elements of `region`: `uint8` for a byte region,
/// and the built element type of a typed array region (E0784 otherwise).
pub(super) fn element_scalar(
    region: mir::RegionType,
    span: Span,
    context: &LoweringContext<'_>,
) -> DiagnosticResult<mir::ScalarType> {
    match built_element(region, span, context)? {
        None => Ok(mir::ScalarType::Integer(IntegerType::UInt8)),
        Some((mir::Type::Scalar(scalar), _)) => Ok(scalar),
        Some(_) => Err(vec![unsupported(
            span,
            "a built region element is a scalar",
        )]),
    }
}

/// The type of the elements of `region`: `uint8` for a byte region, and
/// the built element type of a typed array region (E0784 otherwise),
/// which is a scalar or a nullable pointer.
pub(super) fn element_type(
    region: mir::RegionType,
    span: Span,
    context: &LoweringContext<'_>,
) -> DiagnosticResult<mir::Type> {
    Ok(match built_element(region, span, context)? {
        None => mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::UInt8)),
        Some((element, _)) => element,
    })
}

/// `$region[$index]` read as `?expected`, for a typed array region whose
/// elements are nullable pointers (Decision 0148 sections 3 and 8).
pub(super) fn nullable_element_read(
    expr: &hir::Expr,
    region: &hir::Expr,
    index: &hir::Expr,
    region_type: mir::RegionType,
    expected: mir::ScalarType,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::NullableScalarExpression> {
    if element_type(region_type, expr.span(), context)? != mir::Type::NullableScalar(expected) {
        return Err(vec![unsupported(
            expr.span(),
            "a region element read has the element's type",
        )]);
    }
    let region = local(region, false, context)?;
    let index = lower_rvalue_as_expected(
        index,
        mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64)),
        context,
    )?;
    Ok(mir::NullableScalarExpression::RegionIndex {
        ty: expected,
        region,
        index: Box::new(index),
        index_span: expr.span(),
    })
}

/// The element type and stride of a typed array region the execution
/// targets build, or E0784 at `span` for an element type none builds yet.
/// `None` for a byte region.
pub(super) fn built_element(
    region: mir::RegionType,
    span: Span,
    context: &LoweringContext<'_>,
) -> DiagnosticResult<Option<(mir::Type, usize)>> {
    let Some(array) = region.array else {
        return Ok(None);
    };
    let element = context.collection_type(array).value;
    match crate::native_abi::array_region_stride(element) {
        Some(stride) => Ok(Some((element, stride))),
        None => Err(vec![crate::unsafe_code::unbuilt_region_element(
            span,
            region.access.array_source_name(),
            &element.to_string(),
        )]),
    }
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
    let mir::Type::NullableRegion(kind) = context.expression_type(object)? else {
        return Ok(None);
    };
    if property != "length" {
        return Ok(None);
    }
    let ty = mir::ScalarType::Integer(IntegerType::Int64);
    let source = mir::Rvalue::NullableRegion(nullable_expression(object, kind, false, context)?);
    let selection = context.coalesce_selection(object);
    let result = materialize_null_safe_call(
        source,
        selection,
        false,
        None,
        *span,
        Some((mir::Type::NullableScalar(ty), false)),
        |source, context| {
            let present = mir::Rvalue::Region(mir::RegionExpression::AssumePresent {
                kind,
                value: Box::new(mir::NullableRegionExpression::Local {
                    kind,
                    local: source,
                    transfer: false,
                }),
            });
            let (source, _) = core_value::materialize_value(present, context);
            let value = mir::Rvalue::Value(value_expression_from_operand(
                ty,
                mir::Operand::RegionLength(source),
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
    let mir::Type::Region(kind) = context.expression_type(expr)? else {
        return Err(vec![unsupported(
            expr.span(),
            "byte region access requires a present region",
        )]);
    };
    if let hir::Expr::Variable { name, span } = unparenthesized_place(expr) {
        let local = context.lookup_local(name, *span)?;
        if context.local_type(local) == mir::Type::Region(kind) {
            return Ok(local);
        }
    }
    let value = mir::Rvalue::Region(expression(expr, kind, false, context)?);
    let (local, _) = core_value::materialize_value(value, context);
    context.locals[local.0].writable = writable;
    Ok(local)
}
