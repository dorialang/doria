use super::*;
use crate::compiler_known_contracts::CoreValueOperation;
use mir::CoreCollectionOperation as Op;

pub(super) fn operation(
    collection: mir::LocalId,
    operation: Op,
    context: &mut LoweringContext<'_>,
) {
    context.push_statement(mir::Statement::CoreCollection {
        collection,
        operation,
    });
}

pub(super) fn value_at(
    collection: mir::LocalId,
    position: mir::LocalId,
    ty: mir::Type,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::Rvalue> {
    let mir::Type::Collection(id) = context.local_type(collection) else {
        unreachable!()
    };
    if !context.collection_type(id).uses_core_operations() {
        let offset = collection_offset_rvalue(position);
        return collection_value_rvalue(collection, offset.clone(), offset, ty, true);
    }
    let target = context.declare_borrowed_temp(ty, false);
    operation(collection, Op::ValueAt { position, target }, context);
    Ok(local_rvalue(target, ty, false))
}

pub(super) fn key_at(
    collection: mir::LocalId,
    position: mir::LocalId,
    ty: mir::Type,
    context: &mut LoweringContext<'_>,
) -> mir::Rvalue {
    let target = context.declare_borrowed_temp(ty, false);
    operation(collection, Op::KeyAt { position, target }, context);
    local_rvalue(target, ty, false)
}

fn int_slot(value: i128, context: &mut LoweringContext<'_>) -> mir::LocalId {
    let local = context.declare_temp(true, IntegerType::Int64);
    context.push_statement(mir::Statement::AssignLocal {
        target: local,
        value: integer_constant_rvalue(value),
    });
    local
}

fn nonnegative(local: mir::LocalId) -> mir::BoolExpression {
    mir::BoolExpression::Compare {
        op: mir::CompareOp::GreaterEqual,
        left: Box::new(mir::ValueExpression::Integer(local_integer_expression(
            local,
            IntegerType::Int64,
        ))),
        right: Box::new(mir::ValueExpression::Integer(
            mir::IntegerExpression::constant(
                IntegerValue::from_i128(IntegerType::Int64, 0).unwrap(),
            ),
        )),
    }
}

pub(super) fn literal(
    id: mir::CollectionTypeId,
    elements: &[hir::ArrayElement],
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::CollectionExpression> {
    let definition = context.collection_type(id).clone();
    let output = context.declare_owned_temp(mir::Type::Collection(id));
    context.locals[output.0].writable = true;
    context.push_statement(mir::Statement::AssignLocal {
        target: output,
        value: mir::Rvalue::Collection(mir::CollectionExpression::Literal {
            collection: id,
            entries: Vec::new(),
        }),
    });
    for element in elements {
        let key = element.key.as_ref().ok_or_else(|| {
            vec![unsupported(
                element.value.span(),
                "dictionary literal requires a key",
            )]
        })?;
        let lowered = lower_rvalue_as_expected(
            key,
            definition.key.expect("core dictionary literal"),
            context,
        )?;
        let lowered = core_value::materialize_value(lowered, context);
        let value = lower_rvalue_as_expected(&element.value, definition.value, context)?;
        set(
            output,
            &definition,
            local_rvalue(lowered.0, lowered.1, true),
            value,
            key.span(),
            context,
        )?;
    }
    Ok(mir::CollectionExpression::Local {
        collection: id,
        local: output,
        transfer: true,
        assume_non_null: false,
    })
}

pub(super) fn walk(
    source: mir::LocalId,
    span: Span,
    context: &mut LoweringContext<'_>,
    mut body: impl FnMut(mir::LocalId, &mut LoweringContext<'_>) -> DiagnosticResult<()>,
) -> DiagnosticResult<()> {
    let index = int_slot(0, context);
    let count = core_value::materialize_value(
        mir::Rvalue::Value(value_expression_from_operand(
            mir::ScalarType::Integer(IntegerType::Int64),
            mir::Operand::CollectionLength(source),
        )),
        context,
    )
    .0;
    let header = context.create_block();
    let visit = context.create_block();
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
                count,
                IntegerType::Int64,
            ))),
        },
        then_block: visit,
        else_block: exit,
    });
    context.current_block = Some(visit);
    context.begin_statement_temporaries();
    let result = body(index, context);
    context.finish_statement_temporaries(result.is_ok());
    result?;
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
    Ok(())
}

fn append_duplicate(
    output: mir::LocalId,
    value: mir::Rvalue,
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<()> {
    let duplicate = core_value::duplicate(value, span, context)?;
    let duplicate = core_value::materialize_value(duplicate, context);
    let mir::Type::Collection(id) = context.local_type(output) else {
        unreachable!()
    };
    let hash = if context.collection_type(id).comparator == Some(mir::CollectionComparator::Core) {
        None
    } else {
        let hash = core_value::invoke(
            CoreValueOperation::Hash,
            local_rvalue(duplicate.0, duplicate.1, false),
            Vec::new(),
            span,
            false,
            context,
        )?;
        Some(core_value::materialize_value(hash, context).0)
    };
    let count = core_value::materialize_value(
        mir::Rvalue::Value(value_expression_from_operand(
            mir::ScalarType::Integer(IntegerType::Int64),
            mir::Operand::CollectionLength(output),
        )),
        context,
    )
    .0;
    operation(
        output,
        Op::Insert {
            position: count,
            key: None,
            value: duplicate.0,
            hash,
        },
        context,
    );
    Ok(())
}

pub(super) fn set_algebra(
    left: mir::LocalId,
    right: mir::LocalId,
    id: mir::CollectionTypeId,
    algebra: mir::SetAlgebraOp,
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::CollectionExpression> {
    let definition = context.collection_type(id).clone();
    let output = context.declare_owned_temp(mir::Type::Collection(id));
    context.locals[output.0].writable = true;
    context.push_statement(mir::Statement::AssignLocal {
        target: output,
        value: mir::Rvalue::Collection(mir::CollectionExpression::Literal {
            collection: id,
            entries: Vec::new(),
        }),
    });
    let sources = if algebra == mir::SetAlgebraOp::Union {
        vec![left, right]
    } else {
        vec![left]
    };
    for (pass, source) in sources.into_iter().enumerate() {
        walk(source, span, context, |index, context| {
            let value = value_at(source, index, definition.value, context)?;
            let value = core_value::materialize_value(value, context);
            let exit = context.create_block();
            if algebra != mir::SetAlgebraOp::Union || pass == 1 {
                let other = if pass == 0 { right } else { left };
                let (position, _, _) = lookup(other, &definition, value, span, context)?;
                let present = nonnegative(position);
                let selected = if algebra == mir::SetAlgebraOp::Intersect {
                    present
                } else {
                    mir::BoolExpression::Not(Box::new(present))
                };
                let append = context.create_block();
                context.terminate_current(mir::Terminator::Branch {
                    condition: selected,
                    then_block: append,
                    else_block: exit,
                });
                context.current_block = Some(append);
            }
            append_duplicate(output, local_rvalue(value.0, value.1, false), span, context)?;
            context.terminate_current(mir::Terminator::Jump(exit));
            context.current_block = Some(exit);
            Ok(())
        })?;
    }
    if definition.comparator == Some(mir::CollectionComparator::Core) {
        core_ordered::sort(output, &definition, span, context)?;
    }
    Ok(mir::CollectionExpression::Local {
        collection: id,
        local: output,
        transfer: true,
        assume_non_null: false,
    })
}

pub(super) fn preserving_set(
    source: &hir::Expr,
    id: mir::CollectionTypeId,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::CollectionExpression> {
    let definition = context.collection_type(id).clone();
    let (input, _) = core_value::materialize_operand(source, context)?;
    let output = context.declare_owned_temp(mir::Type::Collection(id));
    context.locals[output.0].writable = true;
    context.push_statement(mir::Statement::AssignLocal {
        target: output,
        value: mir::Rvalue::Collection(mir::CollectionExpression::Literal {
            collection: id,
            entries: Vec::new(),
        }),
    });
    walk(input, source.span(), context, |index, context| {
        let value = value_at(input, index, definition.value, context)?;
        let value = core_value::materialize_value(value, context);
        let (position, _) = hash_lookup(output, &definition, value, source.span(), context)?;
        let append = context.create_block();
        let exit = context.create_block();
        context.terminate_current(mir::Terminator::Branch {
            condition: nonnegative(position),
            then_block: exit,
            else_block: append,
        });
        context.current_block = Some(append);
        append_duplicate(
            output,
            local_rvalue(value.0, value.1, false),
            source.span(),
            context,
        )?;
        context.terminate_current(mir::Terminator::Jump(exit));
        context.current_block = Some(exit);
        Ok(())
    })?;
    Ok(mir::CollectionExpression::Local {
        collection: id,
        local: output,
        transfer: true,
        assume_non_null: false,
    })
}

pub(super) fn expression(
    expr: &hir::Expr,
    expected: mir::Type,
    consume: bool,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<Option<mir::Rvalue>> {
    if let hir::Expr::MethodCall {
        object,
        method,
        args,
        null_safe: false,
        span,
    } = expr
    {
        if method == "pop" && args.is_empty() {
            if let Ok(mir::Type::Collection(id)) = context.expression_type(object) {
                let definition = context.collection_type(id).clone();
                if definition.kind == mir::CollectionKind::PriorityQueue
                    && definition.uses_core_operations()
                {
                    let (collection, _) = lower_collection_local(object, context)?;
                    return core_ordered::pop(
                        collection,
                        &definition,
                        expected,
                        *span,
                        consume,
                        context,
                    )
                    .map(Some);
                }
            }
        }
    }
    let (object, key, remove, required) = match expr {
        hir::Expr::Index {
            collection, index, ..
        } => (collection.as_ref(), index.as_ref(), false, true),
        hir::Expr::MethodCall {
            object,
            method,
            args,
            null_safe: false,
            ..
        } if matches!(method.as_str(), "get" | "remove") && args.len() == 1 => {
            (object.as_ref(), &args[0].value, method == "remove", false)
        }
        _ => return Ok(None),
    };
    let Ok(mir::Type::Collection(id)) = context.expression_type(object) else {
        return Ok(None);
    };
    let definition = context.collection_type(id).clone();
    let Some(key_ty) = definition.key.filter(|_| definition.uses_core_operations()) else {
        return Ok(None);
    };
    let (collection, _) = lower_collection_local(object, context)?;
    let key = lower_rvalue_as_borrowed(key, key_ty, context)?;
    let key = core_value::materialize_value(key, context);
    let (position, _, _) = lookup(collection, &definition, key, expr.span(), context)?;
    if required {
        operation(collection, Op::RequireKey { position }, context);
        return value_at(collection, position, definition.value, context).map(Some);
    }
    let owned = user_local_type_owns_value(expected) && (remove || !expected.has_move_ownership());
    let result = context.declare_checked_call_slot(expected, owned);
    let present = context.create_block();
    let absent = context.create_block();
    let exit = context.create_block();
    context.terminate_current(mir::Terminator::Branch {
        condition: nonnegative(position),
        then_block: present,
        else_block: absent,
    });
    context.current_block = Some(present);
    let value = if remove {
        remove_at(collection, &definition, position, context)
    } else {
        value_at(collection, position, definition.value, context)?
    };
    let value = core_value::materialize_value(value, context);
    let value = convert_call_result(value.0, value.1, expected, remove, expr.span(), context)?;
    context.push_statement(mir::Statement::AssignLocal {
        target: result,
        value,
    });
    context.terminate_current(mir::Terminator::Jump(exit));
    context.current_block = Some(absent);
    context.push_statement(mir::Statement::AssignLocal {
        target: result,
        value: null_rvalue_for_type(expected, expr.span())?,
    });
    context.terminate_current(mir::Terminator::Jump(exit));
    context.current_block = Some(exit);
    if owned && !consume {
        context.track_statement_owned_local(result, expected);
    }
    Ok(Some(local_rvalue(result, expected, owned && consume)))
}

pub(super) fn set(
    collection: mir::LocalId,
    definition: &mir::CollectionType,
    key: mir::Rvalue,
    value: mir::Rvalue,
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<()> {
    let key = core_value::materialize_value(key, context);
    let value = core_value::materialize_value(value, context);
    let (position, insertion, hash) = lookup(collection, definition, key, span, context)?;
    let replace = context.create_block();
    let append = context.create_block();
    let exit = context.create_block();
    context.terminate_current(mir::Terminator::Branch {
        condition: nonnegative(position),
        then_block: replace,
        else_block: append,
    });
    context.current_block = Some(replace);
    let previous = context.declare_checked_call_slot(
        definition.value,
        user_local_type_owns_value(definition.value),
    );
    operation(
        collection,
        Op::Exchange {
            position,
            value: value.0,
            previous,
        },
        context,
    );
    lower_discarded_rvalue(local_rvalue(previous, definition.value, true), context);
    context.terminate_current(mir::Terminator::Jump(exit));
    context.current_block = Some(append);
    operation(
        collection,
        Op::Insert {
            position: insertion,
            key: Some(key.0),
            value: value.0,
            hash,
        },
        context,
    );
    context.terminate_current(mir::Terminator::Jump(exit));
    context.current_block = Some(exit);
    Ok(())
}

pub(super) fn discard_removed(
    collection: mir::LocalId,
    definition: &mir::CollectionType,
    key: mir::Rvalue,
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<()> {
    let key = core_value::materialize_value(key, context);
    let (position, _, _) = lookup(collection, definition, key, span, context)?;
    let remove = context.create_block();
    let exit = context.create_block();
    context.terminate_current(mir::Terminator::Branch {
        condition: nonnegative(position),
        then_block: remove,
        else_block: exit,
    });
    context.current_block = Some(remove);
    let value = remove_at(collection, definition, position, context);
    lower_discarded_rvalue(value, context);
    context.terminate_current(mir::Terminator::Jump(exit));
    context.current_block = Some(exit);
    Ok(())
}

pub(super) fn hash_lookup(
    collection: mir::LocalId,
    definition: &mir::CollectionType,
    expected: (mir::LocalId, mir::Type),
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<(mir::LocalId, mir::LocalId)> {
    let hash = core_value::invoke(
        CoreValueOperation::Hash,
        local_rvalue(expected.0, expected.1, false),
        Vec::new(),
        span,
        false,
        context,
    )?;
    let (hash, _) = core_value::materialize_value(hash, context);
    let result = int_slot(-1, context);
    let previous = int_slot(-1, context);
    let slot = context.declare_temp(false, IntegerType::Int64);
    let position = context.declare_temp(false, IntegerType::Int64);
    let probe = context.create_block();
    let candidate = context.create_block();
    let found = context.create_block();
    let next = context.create_block();
    let exit = context.create_block();
    context.terminate_current(mir::Terminator::Jump(probe));
    context.current_block = Some(probe);
    operation(
        collection,
        Op::HashNext {
            hash,
            previous,
            target: slot,
        },
        context,
    );
    context.terminate_current(mir::Terminator::Branch {
        condition: nonnegative(slot),
        then_block: candidate,
        else_block: exit,
    });
    context.current_block = Some(candidate);
    operation(
        collection,
        Op::HashPosition {
            slot,
            target: position,
        },
        context,
    );
    let key = if let Some(ty) = definition.key {
        key_at(collection, position, ty, context)
    } else {
        value_at(collection, position, definition.value, context)?
    };
    let key = core_value::materialize_value(key, context);
    let condition = core_value::lower_local_equality(key, expected, false, span, context)?;
    context.terminate_current(mir::Terminator::Branch {
        condition,
        then_block: found,
        else_block: next,
    });
    context.current_block = Some(found);
    context.push_statement(mir::Statement::AssignLocal {
        target: result,
        value: collection_offset_rvalue(position),
    });
    context.terminate_current(mir::Terminator::Jump(exit));
    context.current_block = Some(next);
    context.push_statement(mir::Statement::AssignLocal {
        target: previous,
        value: collection_offset_rvalue(slot),
    });
    context.terminate_current(mir::Terminator::Jump(probe));
    context.current_block = Some(exit);
    Ok((result, hash))
}

fn lookup(
    collection: mir::LocalId,
    definition: &mir::CollectionType,
    expected: (mir::LocalId, mir::Type),
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<(mir::LocalId, mir::LocalId, Option<mir::LocalId>)> {
    if definition.uses_core_hash() {
        let (position, hash) = hash_lookup(collection, definition, expected, span, context)?;
        let insertion = core_value::materialize_value(
            mir::Rvalue::Value(value_expression_from_operand(
                mir::ScalarType::Integer(IntegerType::Int64),
                mir::Operand::CollectionLength(collection),
            )),
            context,
        )
        .0;
        Ok((position, insertion, Some(hash)))
    } else {
        let (position, insertion) =
            core_ordered::lookup(collection, definition, expected, span, context)?;
        Ok((position, insertion, None))
    }
}

pub(super) fn remove_at(
    collection: mir::LocalId,
    definition: &mir::CollectionType,
    position: mir::LocalId,
    context: &mut LoweringContext<'_>,
) -> mir::Rvalue {
    let key = definition
        .key
        .map(|ty| context.declare_checked_call_slot(ty, user_local_type_owns_value(ty)));
    let value = context.declare_checked_call_slot(
        definition.value,
        user_local_type_owns_value(definition.value),
    );
    operation(
        collection,
        Op::Remove {
            position,
            key,
            value,
        },
        context,
    );
    if let (Some(key), Some(ty)) = (key, definition.key) {
        lower_discarded_rvalue(local_rvalue(key, ty, true), context);
    }
    local_rvalue(value, definition.value, true)
}

pub(super) fn membership(
    collection: mir::LocalId,
    definition: &mir::CollectionType,
    value: mir::Rvalue,
    op: mir::CollectionMembershipOp,
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::BoolExpression> {
    let value = core_value::materialize_value(value, context);
    let (position, insertion, hash) = lookup(collection, definition, value, span, context)?;
    let present = nonnegative(position);
    if op == mir::CollectionMembershipOp::Contains {
        return Ok(present);
    }
    let change = context.create_block();
    let exit = context.create_block();
    let condition = if op == mir::CollectionMembershipOp::Add {
        mir::BoolExpression::Not(Box::new(present.clone()))
    } else {
        present.clone()
    };
    context.terminate_current(mir::Terminator::Branch {
        condition: condition.clone(),
        then_block: change,
        else_block: exit,
    });
    context.current_block = Some(change);
    if op == mir::CollectionMembershipOp::Add {
        operation(
            collection,
            Op::Insert {
                position: insertion,
                key: None,
                value: value.0,
                hash,
            },
            context,
        );
    } else {
        let removed = remove_at(collection, definition, position, context);
        lower_discarded_rvalue(removed, context);
    }
    context.terminate_current(mir::Terminator::Jump(exit));
    context.current_block = Some(exit);
    Ok(condition)
}
