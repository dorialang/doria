use super::*;
use crate::compiler_known_contracts::CoreValueOperation;

type Value = (mir::LocalId, mir::Type);

fn int(value: i128) -> mir::IntegerExpression {
    mir::IntegerExpression::constant(IntegerValue::from_i128(IntegerType::Int64, value).unwrap())
}

fn local(value: mir::LocalId) -> mir::IntegerExpression {
    local_integer_expression(value, IntegerType::Int64)
}

fn binary(
    op: mir::IntegerBinaryOp,
    left: mir::IntegerExpression,
    right: mir::IntegerExpression,
    span: Span,
) -> mir::IntegerExpression {
    mir::IntegerExpression::Binary {
        ty: IntegerType::Int64,
        op,
        left: Box::new(left),
        right: Box::new(right),
        span,
        right_span: span,
    }
}

fn assign(target: mir::LocalId, value: mir::IntegerExpression, context: &mut LoweringContext<'_>) {
    context.push_statement(mir::Statement::AssignLocal {
        target,
        value: mir::Rvalue::Value(mir::ValueExpression::Integer(value)),
    });
}

fn slot(value: mir::IntegerExpression, context: &mut LoweringContext<'_>) -> mir::LocalId {
    let target = context.declare_temp(true, IntegerType::Int64);
    assign(target, value, context);
    target
}

fn test(
    op: mir::CompareOp,
    left: mir::IntegerExpression,
    right: mir::IntegerExpression,
) -> mir::BoolExpression {
    mir::BoolExpression::Compare {
        op,
        left: Box::new(mir::ValueExpression::Integer(left)),
        right: Box::new(mir::ValueExpression::Integer(right)),
    }
}

fn length(collection: mir::LocalId) -> mir::IntegerExpression {
    mir::IntegerExpression::use_operand(
        IntegerType::Int64,
        mir::Operand::CollectionLength(collection),
    )
}

fn element(
    collection: mir::LocalId,
    definition: &mir::CollectionType,
    position: mir::LocalId,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<Value> {
    let value = if let Some(key) = definition.key {
        core_collection::key_at(collection, position, key, context)
    } else {
        core_collection::value_at(collection, position, definition.value, context)?
    };
    Ok(core_value::materialize_value(value, context))
}

fn ordering(
    left: Value,
    right: Value,
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::ValueExpression> {
    let result = core_value::invoke(
        CoreValueOperation::Compare,
        local_rvalue(left.0, left.1, false),
        vec![local_rvalue(right.0, right.1, false)],
        span,
        false,
        context,
    )?;
    let mir::Rvalue::Value(value @ mir::ValueExpression::Enum(_)) = result else {
        return Err(vec![unsupported(
            span,
            "comparison must return the canonical Ordering enum",
        )]);
    };
    Ok(value)
}

fn is_ordering(
    value: mir::ValueExpression,
    case: &str,
    context: &LoweringContext<'_>,
) -> mir::BoolExpression {
    mir::BoolExpression::Compare {
        op: mir::CompareOp::Equal,
        left: Box::new(value),
        right: Box::new(mir::ValueExpression::Enum(mir::EnumExpression::Case(
            context
                .enum_case_value("Ordering", case)
                .expect("checked Ordering case"),
        ))),
    }
}

pub(super) fn lookup(
    collection: mir::LocalId,
    definition: &mir::CollectionType,
    needle: Value,
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<(mir::LocalId, mir::LocalId)> {
    let low = slot(int(0), context);
    let high = slot(length(collection), context);
    let found = slot(int(-1), context);
    let header = context.create_block();
    let probe = context.create_block();
    let upper = context.create_block();
    let lower = context.create_block();
    let bound = context.create_block();
    let candidate = context.create_block();
    let present = context.create_block();
    let exit = context.create_block();
    context.terminate_current(mir::Terminator::Jump(header));
    context.current_block = Some(header);
    context.terminate_current(mir::Terminator::Branch {
        condition: test(mir::CompareOp::Less, local(low), local(high)),
        then_block: probe,
        else_block: bound,
    });
    context.current_block = Some(probe);
    let middle = slot(
        binary(
            mir::IntegerBinaryOp::Add,
            local(low),
            binary(
                mir::IntegerBinaryOp::Divide,
                binary(
                    mir::IntegerBinaryOp::Subtract,
                    local(high),
                    local(low),
                    span,
                ),
                int(2),
                span,
            ),
            span,
        ),
        context,
    );
    let value = element(collection, definition, middle, context)?;
    let compared = ordering(value, needle, span, context)?;
    context.terminate_current(mir::Terminator::Branch {
        condition: is_ordering(compared, "Less", context),
        then_block: upper,
        else_block: lower,
    });
    context.current_block = Some(upper);
    assign(
        low,
        binary(mir::IntegerBinaryOp::Add, local(middle), int(1), span),
        context,
    );
    context.terminate_current(mir::Terminator::Jump(header));
    context.current_block = Some(lower);
    assign(high, local(middle), context);
    context.terminate_current(mir::Terminator::Jump(header));
    context.current_block = Some(bound);
    context.terminate_current(mir::Terminator::Branch {
        condition: test(mir::CompareOp::Less, local(low), length(collection)),
        then_block: candidate,
        else_block: exit,
    });
    context.current_block = Some(candidate);
    let value = element(collection, definition, low, context)?;
    let compared = ordering(value, needle, span, context)?;
    context.terminate_current(mir::Terminator::Branch {
        condition: is_ordering(compared, "Equal", context),
        then_block: present,
        else_block: exit,
    });
    context.current_block = Some(present);
    assign(found, local(low), context);
    context.terminate_current(mir::Terminator::Jump(exit));
    context.current_block = Some(exit);
    Ok((found, low))
}

type ComparePositions<'a> = dyn FnMut(
        mir::LocalId,
        mir::LocalId,
        &mut LoweringContext<'_>,
    ) -> DiagnosticResult<mir::ValueExpression>
    + 'a;
type SwapPositions<'a> =
    dyn FnMut(mir::LocalId, mir::LocalId, &mut LoweringContext<'_>) -> DiagnosticResult<()> + 'a;

// Select the complete sift path before mutating an observable heap. A checked
// comparison failure therefore leaves its original ordering and owners intact.
fn sift_destination(
    root: mir::LocalId,
    replacement: mir::LocalId,
    count: mir::LocalId,
    maximum: bool,
    span: Span,
    compare: &mut ComparePositions<'_>,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::LocalId> {
    let parent = slot(local(root), context);
    let header = context.create_block();
    let choose = context.create_block();
    let compare_right = context.create_block();
    let use_right = context.create_block();
    let compare_parent = context.create_block();
    let descend = context.create_block();
    let exit = context.create_block();
    context.terminate_current(mir::Terminator::Jump(header));
    context.current_block = Some(header);
    let child = slot(
        binary(
            mir::IntegerBinaryOp::Add,
            binary(mir::IntegerBinaryOp::Multiply, local(parent), int(2), span),
            int(1),
            span,
        ),
        context,
    );
    context.terminate_current(mir::Terminator::Branch {
        condition: test(mir::CompareOp::Less, local(child), local(count)),
        then_block: choose,
        else_block: exit,
    });
    context.current_block = Some(choose);
    let right = slot(
        binary(mir::IntegerBinaryOp::Add, local(child), int(1), span),
        context,
    );
    context.terminate_current(mir::Terminator::Branch {
        condition: test(mir::CompareOp::Less, local(right), local(count)),
        then_block: compare_right,
        else_block: compare_parent,
    });
    context.current_block = Some(compare_right);
    let compared = compare(right, child, context)?;
    context.terminate_current(mir::Terminator::Branch {
        condition: is_ordering(compared, if maximum { "Greater" } else { "Less" }, context),
        then_block: use_right,
        else_block: compare_parent,
    });
    context.current_block = Some(use_right);
    assign(child, local(right), context);
    context.terminate_current(mir::Terminator::Jump(compare_parent));
    context.current_block = Some(compare_parent);
    let compared = compare(replacement, child, context)?;
    context.terminate_current(mir::Terminator::Branch {
        condition: is_ordering(compared, if maximum { "Less" } else { "Greater" }, context),
        then_block: descend,
        else_block: exit,
    });
    context.current_block = Some(descend);
    assign(parent, local(child), context);
    context.terminate_current(mir::Terminator::Jump(header));
    context.current_block = Some(exit);
    Ok(parent)
}

fn rotate_down(
    root: mir::LocalId,
    destination: mir::LocalId,
    span: Span,
    swap: &mut SwapPositions<'_>,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<()> {
    let cursor = slot(local(destination), context);
    let header = context.create_block();
    let body = context.create_block();
    let exit = context.create_block();
    context.terminate_current(mir::Terminator::Jump(header));
    context.current_block = Some(header);
    context.terminate_current(mir::Terminator::Branch {
        condition: test(mir::CompareOp::NotEqual, local(cursor), local(root)),
        then_block: body,
        else_block: exit,
    });
    context.current_block = Some(body);
    swap(root, cursor, context)?;
    assign(
        cursor,
        binary(
            mir::IntegerBinaryOp::Divide,
            binary(mir::IntegerBinaryOp::Subtract, local(cursor), int(1), span),
            int(2),
            span,
        ),
        context,
    );
    context.terminate_current(mir::Terminator::Jump(header));
    context.current_block = Some(exit);
    Ok(())
}

fn heapify(
    count: mir::LocalId,
    maximum: bool,
    span: Span,
    compare: &mut ComparePositions<'_>,
    swap: &mut SwapPositions<'_>,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<()> {
    let root = slot(
        binary(
            mir::IntegerBinaryOp::Subtract,
            binary(mir::IntegerBinaryOp::Divide, local(count), int(2), span),
            int(1),
            span,
        ),
        context,
    );
    let header = context.create_block();
    let body = context.create_block();
    let exit = context.create_block();
    context.terminate_current(mir::Terminator::Jump(header));
    context.current_block = Some(header);
    context.terminate_current(mir::Terminator::Branch {
        condition: test(mir::CompareOp::GreaterEqual, local(root), int(0)),
        then_block: body,
        else_block: exit,
    });
    context.current_block = Some(body);
    let destination = sift_destination(root, root, count, maximum, span, compare, context)?;
    rotate_down(root, destination, span, swap, context)?;
    assign(
        root,
        binary(mir::IntegerBinaryOp::Subtract, local(root), int(1), span),
        context,
    );
    context.terminate_current(mir::Terminator::Jump(header));
    context.current_block = Some(exit);
    Ok(())
}

fn raw_swap(
    collection: mir::LocalId,
    left: mir::LocalId,
    right: mir::LocalId,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<()> {
    core_collection::operation(
        collection,
        mir::CoreCollectionOperation::Swap { left, right },
        context,
    );
    Ok(())
}

pub(super) fn push(
    collection: mir::LocalId,
    definition: &mir::CollectionType,
    value: mir::Rvalue,
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<()> {
    let value = core_value::materialize_value(value, context);
    let count = slot(length(collection), context);
    let destination = slot(local(count), context);
    let header = context.create_block();
    let compare = context.create_block();
    let ascend = context.create_block();
    let insert = context.create_block();
    context.terminate_current(mir::Terminator::Jump(header));
    context.current_block = Some(header);
    context.terminate_current(mir::Terminator::Branch {
        condition: test(mir::CompareOp::Greater, local(destination), int(0)),
        then_block: compare,
        else_block: insert,
    });
    context.current_block = Some(compare);
    let parent = slot(
        binary(
            mir::IntegerBinaryOp::Divide,
            binary(
                mir::IntegerBinaryOp::Subtract,
                local(destination),
                int(1),
                span,
            ),
            int(2),
            span,
        ),
        context,
    );
    let stored = element(collection, definition, parent, context)?;
    let compared = ordering(value, stored, span, context)?;
    context.terminate_current(mir::Terminator::Branch {
        condition: is_ordering(compared, "Less", context),
        then_block: ascend,
        else_block: insert,
    });
    context.current_block = Some(ascend);
    assign(destination, local(parent), context);
    context.terminate_current(mir::Terminator::Jump(header));
    context.current_block = Some(insert);
    core_collection::operation(
        collection,
        mir::CoreCollectionOperation::Insert {
            position: count,
            key: None,
            value: value.0,
            hash: None,
        },
        context,
    );
    let cursor = slot(local(count), context);
    let shift = context.create_block();
    let swap = context.create_block();
    let exit = context.create_block();
    context.terminate_current(mir::Terminator::Jump(shift));
    context.current_block = Some(shift);
    context.terminate_current(mir::Terminator::Branch {
        condition: test(mir::CompareOp::NotEqual, local(cursor), local(destination)),
        then_block: swap,
        else_block: exit,
    });
    context.current_block = Some(swap);
    let parent = slot(
        binary(
            mir::IntegerBinaryOp::Divide,
            binary(mir::IntegerBinaryOp::Subtract, local(cursor), int(1), span),
            int(2),
            span,
        ),
        context,
    );
    raw_swap(collection, cursor, parent, context)?;
    assign(cursor, local(parent), context);
    context.terminate_current(mir::Terminator::Jump(shift));
    context.current_block = Some(exit);
    Ok(())
}

pub(super) fn pop(
    collection: mir::LocalId,
    definition: &mir::CollectionType,
    expected: mir::Type,
    span: Span,
    consume: bool,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::Rvalue> {
    let result = context.declare_checked_call_slot(expected, true);
    let present = context.create_block();
    let absent = context.create_block();
    let exit = context.create_block();
    let count = slot(length(collection), context);
    context.terminate_current(mir::Terminator::Branch {
        condition: test(mir::CompareOp::Greater, local(count), int(0)),
        then_block: present,
        else_block: absent,
    });
    context.current_block = Some(present);
    let root = slot(int(0), context);
    let last = slot(
        binary(mir::IntegerBinaryOp::Subtract, local(count), int(1), span),
        context,
    );
    let destination = sift_destination(
        root,
        last,
        last,
        false,
        span,
        &mut |left, right, context| {
            let left = element(collection, definition, left, context)?;
            let right = element(collection, definition, right, context)?;
            ordering(left, right, span, context)
        },
        context,
    )?;
    raw_swap(collection, root, last, context)?;
    let value = core_collection::remove_at(collection, definition, last, context);
    let value = core_value::materialize_value(value, context);
    rotate_down(
        root,
        destination,
        span,
        &mut |left, right, context| raw_swap(collection, left, right, context),
        context,
    )?;
    let value = convert_call_result(value.0, value.1, expected, true, span, context)?;
    context.push_statement(mir::Statement::AssignLocal {
        target: result,
        value,
    });
    context.terminate_current(mir::Terminator::Jump(exit));
    context.current_block = Some(absent);
    context.push_statement(mir::Statement::AssignLocal {
        target: result,
        value: null_rvalue_for_type(expected, span)?,
    });
    context.terminate_current(mir::Terminator::Jump(exit));
    context.current_block = Some(exit);
    if !consume {
        context.track_statement_owned_local(result, expected);
    }
    Ok(local_rvalue(result, expected, consume))
}

fn empty(id: mir::CollectionTypeId, context: &mut LoweringContext<'_>) -> mir::LocalId {
    let output = context.declare_owned_temp(mir::Type::Collection(id));
    context.locals[output.0].writable = true;
    context.push_statement(mir::Statement::AssignLocal {
        target: output,
        value: mir::Rvalue::Collection(mir::CollectionExpression::Literal {
            collection: id,
            entries: Vec::new(),
        }),
    });
    output
}

fn append_duplicate(
    output: mir::LocalId,
    source: mir::LocalId,
    index: mir::LocalId,
    definition: &mir::CollectionType,
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<()> {
    let key = if let Some(ty) = definition.key {
        let key = collection_key_at_rvalue(source, collection_offset_rvalue(index), ty, context)?;
        let key = core_value::duplicate(key, span, context)?;
        Some(core_value::materialize_value(key, context).0)
    } else {
        None
    };
    let value = core_collection::value_at(source, index, definition.value, context)?;
    let value = core_value::duplicate(value, span, context)?;
    let value = core_value::materialize_value(value, context).0;
    let position = slot(length(output), context);
    core_collection::operation(
        output,
        mir::CoreCollectionOperation::Insert {
            position,
            key,
            value,
            hash: None,
        },
        context,
    );
    Ok(())
}

pub(super) fn preserving_from(
    source: &hir::Expr,
    id: mir::CollectionTypeId,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<mir::CollectionExpression> {
    let definition = context.collection_type(id).clone();
    let span = source.span();
    let (source, _) = core_value::materialize_operand(source, context)?;
    let output = empty(id, context);
    let count = slot(length(source), context);
    if definition.kind == mir::CollectionKind::PriorityQueue {
        core_collection::walk(source, span, context, |index, context| {
            append_duplicate(output, source, index, &definition, span, context)
        })?;
        heapify(
            count,
            false,
            span,
            &mut |left, right, context| {
                let left = element(output, &definition, left, context)?;
                let right = element(output, &definition, right, context)?;
                ordering(left, right, span, context)
            },
            &mut |left, right, context| raw_swap(output, left, right, context),
            context,
        )?;
    } else {
        let int_ty = mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64));
        let scratch_id =
            context.collection_registry.ids[&(mir::CollectionKind::List, None, int_ty)];
        let indices = empty(scratch_id, context);
        core_collection::walk(source, span, context, |index, context| {
            context.push_statement(mir::Statement::CollectionAdd {
                collection: indices,
                value: collection_offset_rvalue(index),
                index: None,
                op: mir::CollectionMutationOp::Add,
            });
            Ok(())
        })?;
        let read_index =
            |index, context: &mut LoweringContext<'_>| -> DiagnosticResult<mir::LocalId> {
                let value = core_collection::value_at(indices, index, int_ty, context)?;
                Ok(core_value::materialize_value(value, context).0)
            };
        let mut compare = |left, right, context: &mut LoweringContext<'_>| {
            let left = read_index(left, context)?;
            let right = read_index(right, context)?;
            let left = element(source, &definition, left, context)?;
            let right = element(source, &definition, right, context)?;
            ordering(left, right, span, context)
        };
        let mut swap = |left, right, context: &mut LoweringContext<'_>| {
            let a = read_index(left, context)?;
            let b = read_index(right, context)?;
            context.push_statement(mir::Statement::AssignCollectionIndex {
                positional: true,
                collection: indices,
                index: collection_offset_rvalue(left),
                value: collection_offset_rvalue(b),
            });
            context.push_statement(mir::Statement::AssignCollectionIndex {
                positional: true,
                collection: indices,
                index: collection_offset_rvalue(right),
                value: collection_offset_rvalue(a),
            });
            Ok(())
        };
        heapify(count, true, span, &mut compare, &mut swap, context)?;
        let root = slot(int(0), context);
        let end = slot(
            binary(mir::IntegerBinaryOp::Subtract, local(count), int(1), span),
            context,
        );
        let header = context.create_block();
        let body = context.create_block();
        let sorted = context.create_block();
        context.terminate_current(mir::Terminator::Jump(header));
        context.current_block = Some(header);
        context.terminate_current(mir::Terminator::Branch {
            condition: test(mir::CompareOp::Greater, local(end), int(0)),
            then_block: body,
            else_block: sorted,
        });
        context.current_block = Some(body);
        swap(root, end, context)?;
        let destination = sift_destination(root, root, end, true, span, &mut compare, context)?;
        rotate_down(root, destination, span, &mut swap, context)?;
        assign(
            end,
            binary(mir::IntegerBinaryOp::Subtract, local(end), int(1), span),
            context,
        );
        context.terminate_current(mir::Terminator::Jump(header));
        context.current_block = Some(sorted);
        let previous = slot(int(-1), context);
        core_collection::walk(indices, span, context, |position, context| {
            let index = read_index(position, context)?;
            let append = context.create_block();
            let exit = context.create_block();
            if definition.kind == mir::CollectionKind::SortedSet {
                let compare = context.create_block();
                context.terminate_current(mir::Terminator::Branch {
                    condition: test(mir::CompareOp::GreaterEqual, local(previous), int(0)),
                    then_block: compare,
                    else_block: append,
                });
                context.current_block = Some(compare);
                let left = element(source, &definition, previous, context)?;
                let right = element(source, &definition, index, context)?;
                let compared = ordering(left, right, span, context)?;
                context.terminate_current(mir::Terminator::Branch {
                    condition: is_ordering(compared, "Equal", context),
                    then_block: exit,
                    else_block: append,
                });
            } else {
                context.terminate_current(mir::Terminator::Jump(append));
            }
            context.current_block = Some(append);
            append_duplicate(output, source, index, &definition, span, context)?;
            assign(previous, local(index), context);
            context.terminate_current(mir::Terminator::Jump(exit));
            context.current_block = Some(exit);
            Ok(())
        })?;
    }
    Ok(mir::CollectionExpression::Local {
        collection: id,
        local: output,
        transfer: true,
        assume_non_null: false,
    })
}

pub(super) fn sort(
    collection: mir::LocalId,
    definition: &mir::CollectionType,
    span: Span,
    context: &mut LoweringContext<'_>,
) -> DiagnosticResult<()> {
    let count = slot(length(collection), context);
    let mut compare = |left, right, context: &mut LoweringContext<'_>| {
        let left = element(collection, definition, left, context)?;
        let right = element(collection, definition, right, context)?;
        ordering(left, right, span, context)
    };
    let mut swap =
        |left, right, context: &mut LoweringContext<'_>| raw_swap(collection, left, right, context);
    heapify(count, true, span, &mut compare, &mut swap, context)?;
    let root = slot(int(0), context);
    let end = slot(
        binary(mir::IntegerBinaryOp::Subtract, local(count), int(1), span),
        context,
    );
    let header = context.create_block();
    let body = context.create_block();
    let exit = context.create_block();
    context.terminate_current(mir::Terminator::Jump(header));
    context.current_block = Some(header);
    context.terminate_current(mir::Terminator::Branch {
        condition: test(mir::CompareOp::Greater, local(end), int(0)),
        then_block: body,
        else_block: exit,
    });
    context.current_block = Some(body);
    swap(root, end, context)?;
    let destination = sift_destination(root, root, end, true, span, &mut compare, context)?;
    rotate_down(root, destination, span, &mut swap, context)?;
    assign(
        end,
        binary(mir::IntegerBinaryOp::Subtract, local(end), int(1), span),
        context,
    );
    context.terminate_current(mir::Terminator::Jump(header));
    context.current_block = Some(exit);
    Ok(())
}
