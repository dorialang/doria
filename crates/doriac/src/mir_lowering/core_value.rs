use super::*;
use crate::compiler_known_contracts::CoreValueOperation;

pub(super) fn primitive_call(
    expr: &hir::Expr,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<Option<mir::Rvalue>> {
    let hir::Expr::MethodCall {
        object, args, span, ..
    } = expr
    else {
        return Ok(None);
    };
    let Some(crate::semantics::CallableTarget::ConstrainedMethod {
        receiver,
        requirement,
        ..
    }) = context
        .semantic_info
        .call_targets
        .get(span)
        .and_then(|target| {
            target.specialize(|ty| substitute_resolved_type(ty, &context.type_substitutions))
        })
    else {
        return Ok(None);
    };
    if !matches!(
        receiver,
        ResolvedType::Integer(_)
            | ResolvedType::Float(_)
            | ResolvedType::Bool
            | ResolvedType::String
    ) {
        return Ok(None);
    }
    let operation = CoreValueOperation::from_requirement(requirement).ok_or_else(|| {
        vec![unsupported(
            *span,
            "primitive call has no canonical core requirement",
        )]
    })?;
    let receiver = materialize_operand(object, context)?;
    let other = args
        .first()
        .map(|argument| materialize_operand(&argument.value, context))
        .transpose()?;
    let result = match operation {
        CoreValueOperation::Equal => {
            mir::Rvalue::Value(mir::ValueExpression::Bool(primitive_compare(
                receiver,
                other.expect("checked equals argument"),
                mir::CompareOp::Equal,
            )?))
        }
        CoreValueOperation::Compare => {
            let other = other.expect("checked compare argument");
            let case = |name| {
                context
                    .enum_case_value("Ordering", name)
                    .map(|value| {
                        mir::Rvalue::Value(mir::ValueExpression::Enum(mir::EnumExpression::Case(
                            value,
                        )))
                    })
                    .ok_or_else(|| vec![unsupported(*span, "canonical Ordering case is missing")])
            };
            let less = case("Less")?;
            let equal = case("Equal")?;
            let greater = case("Greater")?;
            let unequal = select(
                primitive_compare(receiver, other, mir::CompareOp::Less)?,
                less,
                greater,
                context,
            );
            select(
                primitive_compare(receiver, other, mir::CompareOp::Equal)?,
                equal,
                unequal,
                context,
            )
        }
        CoreValueOperation::Hash => primitive_hash(receiver, *span, context)?,
        CoreValueOperation::Clone => {
            return Err(vec![unsupported(
                *span,
                "Copy primitives do not select an owned class clone",
            )])
        }
    };
    Ok(Some(result))
}

fn primitive_compare(
    left: (mir::LocalId, mir::Type),
    right: (mir::LocalId, mir::Type),
    op: mir::CompareOp,
) -> DiagnosticResult<mir::BoolExpression> {
    match (
        local_rvalue(left.0, left.1, false),
        local_rvalue(right.0, right.1, false),
    ) {
        (mir::Rvalue::String(left), mir::Rvalue::String(right)) => {
            Ok(mir::BoolExpression::StringCompare {
                op,
                left: Box::new(left),
                right: Box::new(right),
            })
        }
        (
            mir::Rvalue::Value(mir::ValueExpression::Bool(left)),
            mir::Rvalue::Value(mir::ValueExpression::Bool(right)),
        ) if op == mir::CompareOp::Less => Ok(mir::BoolExpression::Binary {
            op: mir::BoolBinaryOp::And,
            left: Box::new(mir::BoolExpression::Not(Box::new(left))),
            right: Box::new(right),
        }),
        (mir::Rvalue::Value(left), mir::Rvalue::Value(right)) => Ok(mir::BoolExpression::Compare {
            op,
            left: Box::new(left),
            right: Box::new(right),
        }),
        _ => Err(vec![unsupported(
            Span::default(),
            "primitive comparison has incompatible operands",
        )]),
    }
}

fn select(
    condition: mir::BoolExpression,
    yes: mir::Rvalue,
    no: mir::Rvalue,
    context: &mut LoweringContext<'_>,
) -> mir::Rvalue {
    let ty = yes.ty();
    let result = context.declare_checked_call_slot(ty, false);
    let then_block = context.create_block();
    let else_block = context.create_block();
    let exit = context.create_block();
    context.terminate_current(mir::Terminator::Branch {
        condition,
        then_block,
        else_block,
    });
    for (block, value) in [(then_block, yes), (else_block, no)] {
        context.current_block = Some(block);
        context.push_statement(mir::Statement::AssignLocal {
            target: result,
            value,
        });
        context.terminate_current(mir::Terminator::Jump(exit));
    }
    context.current_block = Some(exit);
    local_rvalue(result, ty, false)
}

fn primitive_hash(
    receiver: (mir::LocalId, mir::Type),
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::Rvalue> {
    let uint = IntegerType::UInt64;
    let scalar = |value| mir::Rvalue::Value(mir::ValueExpression::Integer(value));
    let convert = |value| mir::IntegerExpression::Convert {
        ty: uint,
        value: Box::new(value),
        span,
        value_span: span,
    };
    match local_rvalue(receiver.0, receiver.1, false) {
        mir::Rvalue::String(value) => Ok(scalar(mir::IntegerExpression::use_operand(
            uint,
            mir::Operand::StringIntrinsic(Box::new(mir::StringIntrinsicCall {
                kind: mir::StringIntrinsicKind::Hash,
                args: vec![mir::Rvalue::String(value)],
                result: mir::Type::Scalar(mir::ScalarType::Integer(uint)),
                span,
                argument_spans: vec![span],
            })),
        ))),
        mir::Rvalue::Value(mir::ValueExpression::Bool(condition)) => Ok(select(
            condition,
            scalar(mir::IntegerExpression::constant(
                IntegerValue::from_i128(uint, 1).unwrap(),
            )),
            scalar(mir::IntegerExpression::constant(
                IntegerValue::from_i128(uint, 0).unwrap(),
            )),
            context,
        )),
        mir::Rvalue::Value(mir::ValueExpression::Integer(value)) if value.ty() == uint => {
            Ok(scalar(value))
        }
        mir::Rvalue::Value(mir::ValueExpression::Integer(value)) if !value.ty().is_signed() => {
            Ok(scalar(convert(value)))
        }
        mir::Rvalue::Value(mir::ValueExpression::Integer(value)) => {
            let negative = mir::BoolExpression::Compare {
                op: mir::CompareOp::Less,
                left: Box::new(mir::ValueExpression::Integer(value.clone())),
                right: Box::new(mir::ValueExpression::Integer(
                    mir::IntegerExpression::constant(
                        IntegerValue::from_i128(value.ty(), 0).unwrap(),
                    ),
                )),
            };
            let inverted = mir::IntegerExpression::Unary {
                ty: value.ty(),
                op: mir::IntegerUnaryOp::BitwiseNot,
                operand: Box::new(value.clone()),
                span,
            };
            let negative_hash = mir::IntegerExpression::Unary {
                ty: uint,
                op: mir::IntegerUnaryOp::BitwiseNot,
                operand: Box::new(convert(inverted)),
                span,
            };
            Ok(select(
                negative,
                scalar(negative_hash),
                scalar(convert(value)),
                context,
            ))
        }
        _ => Err(vec![unsupported(
            span,
            "primitive has no Hashable implementation",
        )]),
    }
}

pub(super) fn materialize_operand(
    expr: &hir::Expr,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<(mir::LocalId, mir::Type)> {
    materialize_nested_collection_places(expr, false, context)?;
    let ty = context.expression_type(expr)?;
    let value = lower_rvalue_as_borrowed(expr, ty, context)?;
    Ok(materialize_value(value, context))
}

pub(super) fn materialize_value(
    value: mir::Rvalue,
    context: &mut LoweringContext<'_>,
) -> (mir::LocalId, mir::Type) {
    let ty = value.ty();
    let owned = user_local_type_owns_value(ty) && !value.borrows_move_value();
    let local = context.declare_checked_call_slot(ty, owned);
    context.push_statement(mir::Statement::AssignLocal {
        target: local,
        value,
    });
    if owned {
        context.track_statement_owned_local(local, ty);
    }
    (local, ty)
}

pub(super) fn index_of(
    collection: mir::LocalId,
    value: mir::Rvalue,
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::NullableScalarExpression> {
    let ty = value.ty();
    if selected_target(
        CoreValueOperation::Equal,
        non_null_match_type(ty).0,
        span,
        context,
    )
    .is_none()
    {
        if matches!(
            non_null_match_type(ty).0,
            mir::Type::Class(_) | mir::Type::Interface(_)
        ) {
            return Err(vec![unsupported(
                span,
                "collection equality has no checked contract selection",
            )]);
        }
        return Ok(mir::NullableScalarExpression::CollectionIndexOf {
            collection,
            value: Box::new(value),
        });
    }
    // The source stays borrowed throughout the scan. User equality executes in
    // ordinary MIR, so checked exits use the enclosing frame's cleanup route.
    let expected = materialize_value(value, context);
    let index_ty = IntegerType::Int64;
    let scalar = mir::ScalarType::Integer(index_ty);
    let result = context.declare_checked_call_slot(mir::Type::NullableScalar(scalar), false);
    let index = context.declare_temp(true, index_ty);
    let count = context.declare_temp(false, index_ty);
    context.push_statement(mir::Statement::AssignLocal {
        target: index,
        value: integer_constant_rvalue(0),
    });
    context.push_statement(mir::Statement::AssignLocal {
        target: count,
        value: mir::Rvalue::Value(value_expression_from_operand(
            scalar,
            mir::Operand::CollectionLength(collection),
        )),
    });
    context.push_statement(mir::Statement::AssignLocal {
        target: result,
        value: mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Null(scalar)),
    });
    let header = context.create_block();
    let body = context.create_block();
    let found = context.create_block();
    let advance = context.create_block();
    let exit = context.create_block();
    context.terminate_current(mir::Terminator::Jump(header));
    context.current_block = Some(header);
    context.terminate_current(mir::Terminator::Branch {
        condition: mir::BoolExpression::Compare {
            op: mir::CompareOp::Less,
            left: Box::new(mir::ValueExpression::Integer(local_integer_expression(
                index, index_ty,
            ))),
            right: Box::new(mir::ValueExpression::Integer(local_integer_expression(
                count, index_ty,
            ))),
        },
        then_block: body,
        else_block: exit,
    });
    context.current_block = Some(body);
    let element = core_collection::value_at(collection, index, ty, context)?;
    let element = materialize_value(element, context);
    let condition = lower_local_equality(element, expected, false, span, context)?;
    context.terminate_current(mir::Terminator::Branch {
        condition,
        then_block: found,
        else_block: advance,
    });
    context.current_block = Some(found);
    context.push_statement(mir::Statement::AssignLocal {
        target: result,
        value: mir::Rvalue::NullableScalar(mir::NullableScalarExpression::Value(
            mir::ValueExpression::Integer(local_integer_expression(index, index_ty)),
        )),
    });
    context.terminate_current(mir::Terminator::Jump(exit));
    context.current_block = Some(advance);
    context.push_statement(mir::Statement::AssignLocal {
        target: index,
        value: mir::Rvalue::Value(mir::ValueExpression::Integer(
            mir::IntegerExpression::Binary {
                ty: index_ty,
                op: mir::IntegerBinaryOp::Add,
                left: Box::new(local_integer_expression(index, index_ty)),
                right: Box::new(mir::IntegerExpression::constant(
                    IntegerValue::from_i128(index_ty, 1).unwrap(),
                )),
                span,
                right_span: span,
            },
        )),
    });
    context.terminate_current(mir::Terminator::Jump(header));
    context.current_block = Some(exit);
    Ok(mir::NullableScalarExpression::Local {
        ty: scalar,
        local: result,
    })
}

pub(super) fn membership(
    collection: mir::LocalId,
    definition: &mir::CollectionType,
    value: mir::Rvalue,
    op: mir::CollectionMembershipOp,
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::BoolExpression> {
    if definition.uses_core_operations()
        && op != mir::CollectionMembershipOp::ContainsValue
        && definition.kind != mir::CollectionKind::PriorityQueue
    {
        return super::core_collection::membership(
            collection, definition, value, op, span, context,
        );
    }
    let value_ty = value.ty();
    let sequential = definition.key.is_none()
        && !matches!(
            definition.kind,
            mir::CollectionKind::Set | mir::CollectionKind::SortedSet
        );
    let supported = (sequential
        && matches!(
            op,
            mir::CollectionMembershipOp::Contains | mir::CollectionMembershipOp::Remove
        ))
        || op == mir::CollectionMembershipOp::ContainsValue;
    if !supported
        || selected_target(
            CoreValueOperation::Equal,
            non_null_match_type(value_ty).0,
            span,
            context,
        )
        .is_none()
    {
        if matches!(
            non_null_match_type(value_ty).0,
            mir::Type::Class(_) | mir::Type::Interface(_)
        ) {
            return Err(vec![unsupported(
                span,
                "collection membership has no executable checked contract selection",
            )]);
        }
        return Ok(mir::BoolExpression::CollectionHas {
            collection,
            value: Box::new(value),
            op,
        });
    }
    let index = index_of(collection, value, span, context)?;
    let present = mir::BoolExpression::NullableScalarIsPresent(Box::new(index.clone()));
    if op != mir::CollectionMembershipOp::Remove {
        return Ok(present);
    }
    let remove = context.create_block();
    let exit = context.create_block();
    context.terminate_current(mir::Terminator::Branch {
        condition: present.clone(),
        then_block: remove,
        else_block: exit,
    });
    context.current_block = Some(remove);
    let mir::NullableScalarExpression::Local { local, ty } = index else {
        unreachable!("user-defined equality scan materializes its index")
    };
    let index = narrowed_match_local_rvalue(
        local,
        mir::Type::NullableScalar(ty),
        mir::Type::Scalar(ty),
        false,
        span,
        context,
    )?;
    let removed = collection_remove_at_rvalue(collection, index, definition.value)?;
    lower_discarded_rvalue(removed, context);
    context.terminate_current(mir::Terminator::Jump(exit));
    context.current_block = Some(exit);
    Ok(present)
}

pub(super) fn lower_equality(
    left: &hir::Expr,
    right: &hir::Expr,
    negate: bool,
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::BoolExpression> {
    let left = materialize_operand(left, context)?;
    let right = materialize_operand(right, context)?;
    lower_local_equality(left, right, negate, span, context)
}

pub(super) fn fill(
    value: &hir::Expr,
    count: &hir::Expr,
    collection: mir::CollectionTypeId,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<Option<mir::CollectionExpression>> {
    let ty = context.collection_type(collection).value;
    if selected_target(
        CoreValueOperation::Clone,
        non_null_match_type(ty).0,
        value.span(),
        context,
    )
    .is_none()
    {
        return Ok(None);
    }
    let source = lower_rvalue_as_borrowed(value, ty, context)?;
    let (source, _) = materialize_value(source, context);
    let count_value = lower_integer_expression(count, context)?;
    let count_local = context.declare_temp(false, IntegerType::Int64);
    context.push_statement(mir::Statement::AssignLocal {
        target: count_local,
        value: mir::Rvalue::Value(mir::ValueExpression::Integer(count_value)),
    });
    build(
        collection,
        source,
        count_local,
        count.span(),
        value.span(),
        mir::CollectionBuildKind::Fill,
        context,
    )
    .map(Some)
}

pub(super) fn preserving_from(
    source: &hir::Expr,
    collection: mir::CollectionTypeId,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<Option<mir::CollectionExpression>> {
    if matches!(source, hir::Expr::Array { elements, .. } if elements.is_empty()) {
        return Ok(None);
    }
    let definition = context.collection_type(collection);
    if definition.comparator == Some(mir::CollectionComparator::Core) {
        return core_ordered::preserving_from(source, collection, context).map(Some);
    }
    if definition.kind == mir::CollectionKind::Set && definition.uses_core_hash() {
        return super::core_collection::preserving_set(source, collection, context).map(Some);
    }
    if !definition.value.has_move_ownership()
        && !definition.key.is_some_and(mir::Type::has_move_ownership)
    {
        return Ok(None);
    }
    let (source_local, _) = materialize_operand(source, context)?;
    let count = context.declare_temp(false, IntegerType::Int64);
    context.push_statement(mir::Statement::AssignLocal {
        target: count,
        value: mir::Rvalue::Value(value_expression_from_operand(
            mir::ScalarType::Integer(IntegerType::Int64),
            mir::Operand::CollectionLength(source_local),
        )),
    });
    build(
        collection,
        source_local,
        count,
        source.span(),
        source.span(),
        mir::CollectionBuildKind::PreservingFrom,
        context,
    )
    .map(Some)
}

fn build(
    collection: mir::CollectionTypeId,
    source: mir::LocalId,
    count_local: mir::LocalId,
    count_span: Span,
    span: Span,
    kind: mir::CollectionBuildKind,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::CollectionExpression> {
    let definition = context.collection_type(collection).clone();
    let setup = context
        .current_block
        .expect("construction has a live setup block");
    let output = context.declare_owned_temp(mir::Type::Collection(collection));
    context.locals[output.0].writable = true;
    context.push_statement(mir::Statement::AssignLocal {
        target: output,
        value: mir::Rvalue::Collection(mir::CollectionExpression::ConstructionCapacity {
            collection,
            count: Box::new(local_integer_expression(count_local, IntegerType::Int64)),
            count_span,
        }),
    });
    let index = context.declare_temp(true, IntegerType::Int64);
    context.push_statement(mir::Statement::AssignLocal {
        target: index,
        value: integer_constant_rvalue(0),
    });
    let header = context.create_block();
    let body = context.create_block();
    let exit = context.create_block();
    context.terminate_current(mir::Terminator::Jump(header));
    context.current_block = Some(header);
    context.terminate_current(mir::Terminator::Branch {
        condition: mir::BoolExpression::Compare {
            op: mir::CompareOp::Less,
            left: Box::new(mir::ValueExpression::Integer(local_integer_expression(
                index,
                IntegerType::Int64,
            ))),
            right: Box::new(mir::ValueExpression::Integer(local_integer_expression(
                count_local,
                IntegerType::Int64,
            ))),
        },
        then_block: body,
        else_block: exit,
    });
    context.current_block = Some(body);
    let (key, value) = match kind {
        mir::CollectionBuildKind::Fill => (None, local_rvalue(source, definition.value, false)),
        mir::CollectionBuildKind::PreservingFrom => {
            let key = if let Some(ty) = definition.key {
                let value =
                    collection_key_at_rvalue(source, collection_offset_rvalue(index), ty, context)?;
                let value = duplicate(value, span, context)?;
                let (local, ty) = materialize_value(value, context);
                Some(local_rvalue(local, ty, true))
            } else {
                None
            };
            let offset = collection_offset_rvalue(index);
            (
                key,
                collection_value_rvalue(source, offset.clone(), offset, definition.value, true)?,
            )
        }
    };
    let duplicate = duplicate(value, span, context)?;
    let append = context
        .current_block
        .expect("duplication has a success continuation");
    context.push_statement(mir::Statement::CollectionAdd {
        collection: output,
        value: duplicate,
        index: key,
        op: mir::CollectionMutationOp::Initialize,
    });
    context.push_statement(mir::Statement::AssignLocal {
        target: index,
        value: mir::Rvalue::Value(mir::ValueExpression::Integer(
            mir::IntegerExpression::Binary {
                ty: IntegerType::Int64,
                op: mir::IntegerBinaryOp::Add,
                left: Box::new(local_integer_expression(index, IntegerType::Int64)),
                right: Box::new(mir::IntegerExpression::constant(
                    IntegerValue::from_i128(IntegerType::Int64, 1).unwrap(),
                )),
                span,
                right_span: span,
            },
        )),
    });
    context.terminate_current(mir::Terminator::Jump(header));
    context.current_block = Some(exit);
    context.blocks[setup.0]
        .statements
        .push(mir::Statement::ControlFlowPlan(
            mir::ControlFlowPlan::CollectionBuild(mir::CollectionBuildPlan {
                kind,
                output,
                source,
                count: count_local,
                index,
                setup,
                header,
                body,
                append,
                exit,
            }),
        ));
    let finished = context.declare_owned_temp(mir::Type::Collection(collection));
    context.push_statement(mir::Statement::AssignLocal {
        target: finished,
        value: mir::Rvalue::Collection(mir::CollectionExpression::FinishConstruction {
            collection,
            source: output,
        }),
    });
    Ok(mir::CollectionExpression::Local {
        collection,
        local: finished,
        transfer: true,
        assume_non_null: false,
    })
}

pub(super) fn duplicate(
    value: mir::Rvalue,
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::Rvalue> {
    if !value.ty().has_move_ownership() {
        return Ok(value);
    }
    let ty = value.ty();
    let (inner, nullable) = non_null_match_type(ty);
    let source = context.declare_borrowed_temp(ty, false);
    context.push_statement(mir::Statement::AssignLocal {
        target: source,
        value,
    });
    let entry = context
        .current_block
        .expect("duplication has a live source");
    let (call, absent, exit, result) = if nullable {
        let call = context.create_block();
        let absent = context.create_block();
        let exit = context.create_block();
        let result = context.declare_checked_call_slot(ty, true);
        context.terminate_current(mir::Terminator::Branch {
            condition: match_presence_condition(source, ty)?,
            then_block: call,
            else_block: absent,
        });
        context.current_block = Some(absent);
        let null = match ty {
            mir::Type::NullableClass(class) => {
                mir::Rvalue::NullableClass(mir::NullableClassExpression::Null(class))
            }
            mir::Type::NullableInterface(interface) => {
                mir::Rvalue::nullable_interface(interface, mir::NullableInterfaceValue::Null)
            }
            _ => {
                return Err(vec![unsupported(
                    span,
                    "nullable duplication requires a Cloneable class or interface",
                )])
            }
        };
        context.push_statement(mir::Statement::AssignLocal {
            target: result,
            value: null,
        });
        context.terminate_current(mir::Terminator::Jump(exit));
        context.current_block = Some(call);
        let receiver = narrowed_match_local_rvalue(source, ty, inner, false, span, context)?;
        let value = invoke(
            CoreValueOperation::Clone,
            receiver,
            Vec::new(),
            span,
            true,
            context,
        )?;
        let value = match value {
            mir::Rvalue::Class(value) => {
                mir::Rvalue::NullableClass(mir::NullableClassExpression::Class(value))
            }
            mir::Rvalue::Interface(value) => mir::Rvalue::nullable_interface(
                value.interface,
                mir::NullableInterfaceValue::Present(value.value),
            ),
            _ => {
                return Err(vec![unsupported(
                    span,
                    "Cloneable duplication returned another representation",
                )])
            }
        };
        context.push_statement(mir::Statement::AssignLocal {
            target: result,
            value,
        });
        context.terminate_current(mir::Terminator::Jump(exit));
        context.current_block = Some(exit);
        (call, Some(absent), exit, result)
    } else {
        let result = invoke(
            CoreValueOperation::Clone,
            local_rvalue(source, ty, false),
            Vec::new(),
            span,
            true,
            context,
        )?;
        (
            entry,
            None,
            context.current_block.unwrap(),
            result
                .direct_place_local()
                .expect("clone result is materialized"),
        )
    };
    context.blocks[entry.0]
        .statements
        .push(mir::Statement::ControlFlowPlan(
            mir::ControlFlowPlan::ElementDuplication(mir::ElementDuplicationPlan {
                source,
                result,
                entry,
                call,
                absent,
                exit,
            }),
        ));
    Ok(local_rvalue(result, ty, true))
}

pub(super) fn lower_local_equality(
    left: (mir::LocalId, mir::Type),
    right: (mir::LocalId, mir::Type),
    negate: bool,
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::BoolExpression> {
    let nullable = non_null_match_type(left.1).1 || non_null_match_type(right.1).1;
    let equality = if nullable {
        let present = |(local, ty)| {
            if non_null_match_type(ty).1 {
                match_presence_condition(local, ty)
            } else {
                Ok(mir::BoolExpression::Use {
                    operand: mir::Operand::Scalar(mir::ScalarValue::Bool(true)),
                })
            }
        };
        let left_present = present(left)?;
        let right_present = present(right)?;
        let both = context.create_block();
        let absent = context.create_block();
        let join = context.create_block();
        let result =
            context.declare_checked_call_slot(mir::Type::Scalar(mir::ScalarType::Bool), false);
        context.terminate_current(mir::Terminator::Branch {
            condition: mir::BoolExpression::Binary {
                op: mir::BoolBinaryOp::And,
                left: Box::new(left_present.clone()),
                right: Box::new(right_present.clone()),
            },
            then_block: both,
            else_block: absent,
        });
        context.current_block = Some(absent);
        context.push_statement(mir::Statement::AssignLocal {
            target: result,
            value: mir::Rvalue::Value(mir::ValueExpression::Bool(mir::BoolExpression::Not(
                Box::new(mir::BoolExpression::Binary {
                    op: mir::BoolBinaryOp::Xor,
                    left: Box::new(left_present),
                    right: Box::new(right_present),
                }),
            ))),
        });
        context.terminate_current(mir::Terminator::Jump(join));
        context.current_block = Some(both);
        let value = invoke_equality(left, right, span, context)?;
        context.push_statement(mir::Statement::AssignLocal {
            target: result,
            value: mir::Rvalue::Value(mir::ValueExpression::Bool(value)),
        });
        context.terminate_current(mir::Terminator::Jump(join));
        context.current_block = Some(join);
        mir::BoolExpression::Use {
            operand: mir::Operand::Local(result),
        }
    } else {
        invoke_equality(left, right, span, context)?
    };
    Ok(if negate {
        mir::BoolExpression::Not(Box::new(equality))
    } else {
        equality
    })
}

fn invoke_equality(
    left: (mir::LocalId, mir::Type),
    right: (mir::LocalId, mir::Type),
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::BoolExpression> {
    let present = |(local, ty), context: &LoweringContext<'_>| {
        let (inner, nullable) = non_null_match_type(ty);
        if nullable {
            narrowed_match_local_rvalue(local, ty, inner, false, span, context)
        } else {
            Ok(local_rvalue(local, ty, false))
        }
    };
    let receiver = present(left, context)?;
    let argument = present(right, context)?;
    if selected_target(
        CoreValueOperation::Equal,
        non_null_match_type(left.1).0,
        span,
        context,
    )
    .is_none()
    {
        let mir::Type::Class(class) = non_null_match_type(left.1).0 else {
            return Err(vec![unsupported(
                span,
                "identity equality requires a class",
            )]);
        };
        let left = context.declare_borrowed_temp(mir::Type::Class(class), false);
        context.push_statement(mir::Statement::AssignLocal {
            target: left,
            value: receiver,
        });
        let right = context.declare_borrowed_temp(mir::Type::Class(class), false);
        context.push_statement(mir::Statement::AssignLocal {
            target: right,
            value: argument,
        });
        return Ok(mir::BoolExpression::ClassIdentityCompare {
            op: mir::CompareOp::Equal,
            class,
            left,
            right,
        });
    }
    let value = invoke(
        CoreValueOperation::Equal,
        receiver,
        vec![argument],
        span,
        false,
        context,
    )?;
    let mir::Rvalue::Value(mir::ValueExpression::Bool(value)) = value else {
        return Err(vec![unsupported(
            span,
            "checked equality implementation must return bool",
        )]);
    };
    Ok(value)
}

pub(super) fn invoke(
    operation: CoreValueOperation,
    receiver: mir::Rvalue,
    args: Vec<mir::Rvalue>,
    span: Span,
    consume_result: bool,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::Rvalue> {
    let target = selected_target(operation, receiver.ty(), span, context).ok_or_else(|| {
        vec![unsupported(
            span,
            "core value operation has no checked implementation",
        )]
    })?;
    let receiver_type = receiver.ty();
    let receiver_local = context.declare_borrowed_temp(receiver_type, false);
    context.push_statement(mir::Statement::AssignLocal {
        target: receiver_local,
        value: receiver,
    });
    let receiver = local_rvalue(receiver_local, receiver_type, false);
    let call_block = context
        .current_block
        .expect("core operation has a live call block");
    let (interface, slot, selected, result) = if let Some(plan) =
        interface::call_plan_target(span, Some(target.clone()), context)?
    {
        let interface = plan.0;
        let slot = plan.1.slot;
        let (local, ty, transfer) = interface::materialize_lowered_call(
            receiver,
            args,
            plan,
            span,
            consume_result,
            context,
        )?
        .expect("core value operations return values");
        (
            interface,
            slot,
            mir::CoreValueCallTarget::Erased,
            (local, ty, transfer),
        )
    } else {
        let class = match target {
            CallableTarget::Method { class_type, .. } => context.class_id_for_type(&class_type),
            _ => None,
        }
        .ok_or_else(|| {
            vec![unsupported(
                span,
                "core value operation has no checked concrete implementation",
            )]
        })?;
        let signature = context.lookup_method(class, operation.method(), span)?;
        let mir::ReturnType::Value(ty) = signature.return_type else {
            return Err(vec![unsupported(span, "core value operation returns void")]);
        };
        let mir::Type::Class(receiver_class) = receiver_type else {
            return Err(vec![unsupported(
                span,
                "concrete core receiver is not a class",
            )]);
        };
        let (vtable, interface, slot) = context
            .collection_registry
            .interface_vtables
            .iter()
            .find_map(|table| {
                if table.implementing_type != mir::ImplementingType::Class(receiver_class) {
                    return None;
                }
                context.collection_registry.interface_types[table.interface.0]
                    .methods
                    .iter()
                    .enumerate()
                    .find(|(_, method)| {
                        method.requirement.source == crate::compiler_known_contracts::SOURCE_ID
                            && method.name == operation.method()
                    })
                    .map(|(slot, _)| (table.id, table.interface, slot))
            })
            .ok_or_else(|| {
                vec![unsupported(
                    span,
                    "core operation has no nominal conformance proof",
                )]
            })?;
        let receiver = if class == receiver_class {
            receiver
        } else {
            mir::Rvalue::Class(mir::ClassExpression::Local {
                class,
                local: receiver_local,
                transfer: false,
            })
        };
        let function = signature.id;
        let mut lowered = vec![receiver];
        lowered.extend(args);
        let result = if signature.checked_effects.is_empty() {
            let value =
                interface::direct_call_result(ty, signature.id, lowered, signature.return_borrow);
            let owned = user_local_type_owns_value(ty) && !value.borrows_move_value();
            let local = context.declare_checked_call_slot(ty, owned);
            context.push_statement(mir::Statement::AssignLocal {
                target: local,
                value,
            });
            if owned && !consume_result {
                context.track_statement_owned_local(local, ty);
            }
            (local, ty, owned && consume_result)
        } else {
            materialize_checked_signature_call(signature, lowered, span, consume_result, context)?
                .expect("core value operations return values")
        };
        (
            interface,
            slot,
            mir::CoreValueCallTarget::Direct { function, vtable },
            result,
        )
    };
    let success = context
        .current_block
        .expect("core operation has a success block");
    context.blocks[call_block.0]
        .statements
        .push(mir::Statement::ControlFlowPlan(
            mir::ControlFlowPlan::CoreValue(mir::CoreValueCallPlan {
                operation,
                receiver: receiver_local,
                interface,
                slot,
                target: selected,
                result: result.0,
                call: call_block,
                success,
                source_span: span,
            }),
        ));
    Ok(local_rvalue(result.0, result.1, result.2))
}

fn selected_target(
    operation: CoreValueOperation,
    receiver: mir::Type,
    span: Span,
    context: &LoweringContext<'_>,
) -> Option<CallableTarget> {
    context
        .semantic_info
        .core_operation_calls
        .get(&span)?
        .iter()
        .find_map(|call| {
            if call.operation != operation
                || context.mir_resolved_type(&call.receiver_type) != Some(receiver)
            {
                return None;
            }
            call.target
                .specialize(|ty| substitute_resolved_type(ty, &context.type_substitutions))
        })
}
