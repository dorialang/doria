use super::*;

pub(super) fn builtin_receiver(
    program: &mir::Program,
    function: &mir::Function,
    receiver: mir::LocalId,
    vtable: mir::InterfaceVtableId,
    cursor: bool,
) -> Result<mir::CollectionTypeId, BackendError> {
    let table = interface_vtable_in(program, vtable)?;
    let collection = match table.implementing_type {
        mir::ImplementingType::Collection(collection) if !cursor => collection,
        mir::ImplementingType::CollectionIterator(collection) if cursor => collection,
        _ => {
            return Err(malformed_mir(
                "collection adapter has another implementing type",
            ))
        }
    };
    if function.params.first() != Some(&receiver)
        || local_in(function, receiver)?.ty != mir::Type::Interface(table.interface)
        || local_in(function, receiver)?.owned
        || !table.methods.contains(&function.id)
        || program
            .interface_vtables
            .iter()
            .any(|other| other.id != vtable && other.methods.contains(&function.id))
    {
        return Err(malformed_mir(
            "collection adapter receiver lacks its exact vtable proof",
        ));
    }
    Ok(collection)
}

pub(super) fn builtin_vtable(
    program: &mir::Program,
    table: &mir::InterfaceVtable,
) -> Result<(), BackendError> {
    use crate::compiler_known_contracts::IterationOperation as Op;
    let (collection, operations) = match table.implementing_type {
        mir::ImplementingType::Collection(collection) => (collection, vec![Op::Acquire]),
        mir::ImplementingType::CollectionIterator(collection) => (
            collection,
            vec![Op::HasCurrent, Op::GetCurrent, Op::Advance],
        ),
        _ => return Err(malformed_mir("collection adapter has class identity")),
    };
    let collection = collection_in(program, collection)?;
    let interface = interface_in(program, table.interface)?;
    if matches!(
        collection.kind,
        mir::CollectionKind::Bytes | mir::CollectionKind::PriorityQueue
    ) || table.error_descriptor.is_some()
        || !interface.ancestors.is_empty()
        || interface.methods.len() != operations.len()
    {
        return Err(malformed_mir(
            "collection adapter exposes an unsupported public contract",
        ));
    }
    for (slot, operation) in operations.into_iter().enumerate() {
        let method = &interface.methods[slot];
        let signature = function_type_in(program, method.signature)?;
        if Op::from_requirement(method.requirement) != Some(operation)
            || signature.parameters.len() != 1
            || signature.parameters[0].ty != mir::Type::Interface(table.interface)
            || signature.parameters[0].mode
                != if operation == Op::Advance {
                    mir::FunctionParameterMode::Writable
                } else {
                    mir::FunctionParameterMode::Readonly
                }
            || !signature.checked_effects.is_empty()
        {
            return Err(malformed_mir(
                "collection adapter does not implement a canonical iteration requirement",
            ));
        }
        match operation {
            Op::HasCurrent
                if signature.return_type
                    != mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Bool)) =>
            {
                return Err(malformed_mir("collection cursor presence must return bool"))
            }
            Op::Advance if signature.return_type != mir::ReturnType::Void => {
                return Err(malformed_mir("collection cursor advance must return void"))
            }
            Op::GetCurrent if signature.return_type != mir::ReturnType::Value(collection.value) => {
                return Err(malformed_mir(
                    "collection iterator yields another element type",
                ))
            }
            Op::Acquire => {
                let mir::ReturnType::Value(mir::Type::Interface(cursor)) = signature.return_type
                else {
                    return Err(malformed_mir(
                        "collection acquisition does not return an Iterator",
                    ));
                };
                if program
                    .interface_vtable(
                        mir::ImplementingType::CollectionIterator(collection.id),
                        cursor,
                    )
                    .is_none()
                {
                    return Err(malformed_mir(
                        "collection acquisition has no matching cursor implementation",
                    ));
                }
            }
            _ => {}
        }
        let expected_borrow = (operation == Op::GetCurrent
            && collection.value.has_move_ownership())
        .then_some(mir::ReturnBorrow {
            source: mir::BorrowSource::Parameter(0),
            writable: false,
        });
        if signature.return_borrow != expected_borrow {
            return Err(malformed_mir(
                "collection iterator has another element ownership contract",
            ));
        }
    }
    Ok(())
}
use crate::compiler_known_contracts::IterationOperation;

pub(super) fn validate(
    program: &mir::Program,
    function: &mir::Function,
    plan: &mir::PublicForeachPlan,
) -> Result<(), BackendError> {
    let source = local_in(function, plan.source)?;
    let cursor = local_in(function, plan.cursor)?;
    let element = local_in(function, plan.value_binding)?;
    if source.writable
        || !source.synthetic
        || !cursor.owned
        || !cursor.writable
        || !cursor.synthetic
        || element.writable
        || (element.ty.has_move_ownership() && element.owned)
    {
        return Err(malformed_mir(
            "public foreach has invalid source, cursor, or element ownership",
        ));
    }
    for (call, operation) in plan.calls.iter().zip([
        IterationOperation::Acquire,
        IterationOperation::HasCurrent,
        IterationOperation::GetCurrent,
        IterationOperation::Advance,
    ]) {
        if call.operation != operation
            || call.receiver
                != if operation == IterationOperation::Acquire {
                    plan.source
                } else {
                    plan.cursor
                }
        {
            return Err(malformed_mir(
                "public foreach must use its one acquired cursor for every operation",
            ));
        }
        validate_call(program, function, call)?;
    }
    let [acquire, has, current, advance] = &plan.calls;
    let acquire_result = acquire
        .result
        .ok_or_else(|| malformed_mir("iterator acquisition has no result"))?;
    let current_result = current
        .result
        .ok_or_else(|| malformed_mir("current element has no result"))?;
    if !assigned_from(function, acquire.success, plan.cursor, acquire_result)?
        || !assigned_from(
            function,
            current.success,
            plan.value_binding,
            current_result,
        )?
        || local_in(function, current_result)?.ty != element.ty
    {
        return Err(malformed_mir(
            "public foreach lost its cursor or current element result",
        ));
    }
    if has.call != plan.header
        || current.call != plan.body
        || advance.call != plan.update
        || !matches!(&block_in(function, has.success)?.terminator,
            mir::Terminator::Branch { condition: mir::BoolExpression::Use { operand: mir::Operand::Local(local) }, then_block, else_block }
                if Some(*local) == has.result && *then_block == plan.body && *else_block == plan.exit)
        || !matches!(&block_in(function, advance.success)?.terminator, mir::Terminator::Jump(target) if *target == plan.header)
        || !matches!(&block_in(function, acquire.success)?.terminator, mir::Terminator::Jump(target) if *target == plan.header)
    {
        return Err(malformed_mir(
            "public foreach does not preserve acquire/test/current/advance control flow",
        ));
    }
    for (proof, use_block) in [
        (acquire.success, plan.header),
        (has.success, plan.body),
        (plan.body, current.success),
    ] {
        if !dominates(function, proof, use_block)? {
            return Err(malformed_mir(
                "public foreach operation is reachable without its prerequisite",
            ));
        }
    }
    if cfg_reaches_without(function, plan.header, acquire.call, &[plan.exit])? {
        return Err(malformed_mir(
            "public foreach reacquires its cursor during the loop",
        ));
    }
    for block in &plan.continue_sources {
        if !cfg_reaches(function, *block, plan.update)? {
            return Err(malformed_mir(
                "public foreach continue does not advance the same cursor",
            ));
        }
    }
    Ok(())
}

fn assigned_from(
    function: &mir::Function,
    block: mir::BlockId,
    target: mir::LocalId,
    source: mir::LocalId,
) -> Result<bool, BackendError> {
    Ok(block_in(function, block)?.statements.iter().any(|statement| matches!(statement,
        mir::Statement::AssignLocal { target: assigned, value } if *assigned == target && value.direct_place_local() == Some(source))))
}

fn dominates(
    function: &mir::Function,
    proof: mir::BlockId,
    use_block: mir::BlockId,
) -> Result<bool, BackendError> {
    Ok(proof == use_block
        || !cfg_reaches_without(function, function.entry_block, use_block, &[proof])?)
}

fn cfg_reaches_without(
    function: &mir::Function,
    start: mir::BlockId,
    target: mir::BlockId,
    stops: &[mir::BlockId],
) -> Result<bool, BackendError> {
    let mut pending = vec![start];
    let mut visited = HashSet::new();
    while let Some(block) = pending.pop() {
        if stops.contains(&block) || !visited.insert(block) {
            continue;
        }
        if block == target {
            return Ok(true);
        }
        pending.extend(terminator_targets(&block_in(function, block)?.terminator));
    }
    Ok(false)
}

fn validate_call(
    program: &mir::Program,
    function: &mir::Function,
    plan: &mir::IterationCallPlan,
) -> Result<(), BackendError> {
    if retained::iteration_operation(program, plan.interface, plan.slot) != Some(plan.operation) {
        return Err(malformed_mir(
            "public iteration selected a noncanonical requirement",
        ));
    }
    let method = &program.interface_types[plan.interface.0].methods[plan.slot];
    let signature = function_type_in(program, method.signature)?;
    let block = block_in(function, plan.call)?;
    block_in(function, plan.success)?;
    let receiver = local_in(function, plan.receiver)?;
    let writable = plan.operation == IterationOperation::Advance;
    if method.writable_receiver != writable || (writable && !receiver.writable) {
        return Err(malformed_mir(
            "iteration receiver mode disagrees with its requirement",
        ));
    }
    let (args, return_type, return_borrow) = match plan.target {
        mir::CoreValueCallTarget::Direct {
            function: target,
            vtable,
        } => {
            let table = program
                .interface_vtables
                .get(vtable.0)
                .ok_or_else(|| malformed_mir("iteration has no conformance table"))?;
            let selected = function_in(program, target)?;
            let entry = table
                .methods
                .get(plan.slot)
                .and_then(|id| program.functions.get(id.0))
                .ok_or_else(|| malformed_mir("iteration conformance table has no method"))?;
            if table.interface != plan.interface
                || !matches!(receiver.ty, mir::Type::Class(class) if table.implementing_type == mir::ImplementingType::Class(class))
                || selected.source_span != entry.source_span
            {
                return Err(malformed_mir(
                    "iteration call does not select its checked implementation",
                ));
            }
            let args = if selected.checked_effects.is_empty() {
                if plan.success != plan.call {
                    return Err(malformed_mir(
                        "ordinary iteration call has an invalid continuation",
                    ));
                }
                block
                    .statements
                    .iter()
                    .find_map(|statement| {
                        let valid = match statement {
                            mir::Statement::AssignLocal { target, .. } => {
                                Some(*target) == plan.result
                            }
                            mir::Statement::CallVoid { .. } => plan.result.is_none(),
                            _ => false,
                        };
                        if !valid {
                            return None;
                        }
                        collect_statement_class_local_accesses(statement)
                            .accesses
                            .into_iter()
                            .find_map(|access| match access {
                                ClassLocalAccess::Call(
                                    CallTarget::Direct { function, .. },
                                    args,
                                ) if function == target => Some(args),
                                _ => None,
                            })
                    })
                    .ok_or_else(|| {
                        malformed_mir("iteration proof does not describe its ordinary call")
                    })?
            } else {
                match &block.terminator {
                    mir::Terminator::CheckedCall {
                        function,
                        args,
                        result,
                        success,
                        ..
                    } if *function == target
                        && *result == plan.result
                        && *success == plan.success =>
                    {
                        args
                    }
                    _ => {
                        return Err(malformed_mir(
                            "iteration proof does not describe its checked call",
                        ))
                    }
                }
            };
            (args, selected.return_type, selected.return_borrow)
        }
        mir::CoreValueCallTarget::Erased => {
            let (callee, ty, args, result, success) = match &block.terminator {
                mir::Terminator::IndirectCall {
                    callee,
                    function_type,
                    args,
                    result,
                    continuation,
                    ..
                } => (callee, function_type, args, result, continuation),
                mir::Terminator::CheckedIndirectCall {
                    callee,
                    function_type,
                    args,
                    result,
                    success,
                    ..
                } => (callee, function_type, args, result, success),
                _ => return Err(malformed_mir("erased iteration has no indirect call")),
            };
            if receiver.ty != mir::Type::Interface(plan.interface)
                || *ty != method.signature
                || *result != plan.result
                || *success != plan.success
                || !matches!(callee, mir::IndirectCallee::InterfaceMethod { interface, slot, receiver }
                    if *interface == plan.interface && *slot == plan.slot && *receiver == plan.receiver)
            {
                return Err(malformed_mir(
                    "erased iteration changed its requirement or receiver",
                ));
            }
            (
                args.as_slice(),
                signature.return_type,
                signature.return_borrow,
            )
        }
    };
    if args.len() != 1
        || args[0].direct_place_local() != Some(plan.receiver)
        || !args[0].borrows_move_value()
    {
        return Err(malformed_mir("iteration must borrow its selected receiver"));
    }
    if let Some(result) = plan.result {
        let local = local_in(function, result)?;
        if return_type != mir::ReturnType::Value(local.ty) || !local.synthetic {
            return Err(malformed_mir("iteration has an invalid result type"));
        }
        if plan.operation == IterationOperation::GetCurrent
            && local.ty.has_move_ownership()
            && (local.owned
                || return_borrow.is_none_or(|borrow| {
                    borrow.writable
                        || borrow.source
                            != if plan.target == mir::CoreValueCallTarget::Erased {
                                mir::BorrowSource::Parameter(0)
                            } else {
                                mir::BorrowSource::Receiver
                            }
                }))
        {
            return Err(malformed_mir("current Move element must borrow the cursor"));
        }
        if plan.operation == IterationOperation::Acquire
            && (!local.owned || return_borrow.is_some())
        {
            return Err(malformed_mir(
                "iterator acquisition must own its cursor state",
            ));
        }
    } else if return_type != mir::ReturnType::Void || plan.operation != IterationOperation::Advance
    {
        return Err(malformed_mir("iteration unexpectedly has no result"));
    }
    Ok(())
}
