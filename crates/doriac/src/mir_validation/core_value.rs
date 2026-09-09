use super::*;
use crate::compiler_known_contracts::{CoreValueOperation, SOURCE_ID};

pub(super) fn collection_operation(
    program: &mir::Program,
    function: &mir::Function,
    collection: mir::LocalId,
    operation: &mir::CoreCollectionOperation,
) -> Result<(), BackendError> {
    use mir::CoreCollectionOperation as Op;
    let local = local_in(function, collection)?;
    let mir::Type::Collection(id) = local.ty else {
        return Err(malformed_mir(
            "core collection operation has a non-collection receiver",
        ));
    };
    let definition = collection_in(program, id)?;
    if !definition.uses_core_operations() || (operation.mutates() && !local.writable) {
        return Err(malformed_mir(
            "core collection operation lacks its concrete writable storage contract",
        ));
    }
    let integer = mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64));
    let hash_type = mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::UInt64));
    let check = |id, ty, owned: Option<bool>| -> Result<(), BackendError> {
        let value = local_in(function, id)?;
        if value.ty != ty
            || owned.is_some_and(|owned| value.owned != owned && ty.has_move_ownership())
        {
            return Err(malformed_mir(
                "core collection operand has the wrong type or ownership",
            ));
        }
        Ok(())
    };
    let keys = |key: Option<mir::LocalId>, owned| -> Result<(), BackendError> {
        match (key, definition.key) {
            (Some(key), Some(ty)) => check(key, ty, Some(owned)),
            (None, None) => Ok(()),
            _ => Err(malformed_mir(
                "core collection key does not match its layout",
            )),
        }
    };
    let inputs = operation.inputs();
    let outputs = operation.outputs();
    if outputs
        .iter()
        .any(|output| *output == collection || inputs.contains(output))
        || outputs.iter().collect::<HashSet<_>>().len() != outputs.len()
        || operation.transfers().iter().collect::<HashSet<_>>().len() != operation.transfers().len()
    {
        return Err(malformed_mir(
            "core collection operation overlaps input and output ownership",
        ));
    }
    for output in &outputs {
        if !local_in(function, *output)?.synthetic {
            return Err(malformed_mir(
                "core collection output is not a compiler temporary",
            ));
        }
    }
    match operation {
        Op::RequireKey { position } => {
            if definition.key.is_none() {
                return Err(malformed_mir("key assertion targets a sequence"));
            }
            check(*position, integer, None)?;
        }
        Op::Insert {
            position,
            key,
            value,
            hash,
        } => {
            check(*position, integer, None)?;
            keys(*key, true)?;
            check(*value, definition.value, Some(true))?;
            if hash.is_some() != definition.uses_core_hash() {
                return Err(malformed_mir(
                    "core collection insertion loses its raw hash",
                ));
            }
            if let Some(hash) = hash {
                check(*hash, hash_type, None)?;
            }
        }
        Op::Remove {
            position,
            key,
            value,
        } => {
            check(*position, integer, None)?;
            keys(*key, true)?;
            check(*value, definition.value, Some(true))?;
        }
        Op::Swap { left, right } => {
            check(*left, integer, None)?;
            check(*right, integer, None)?;
        }
        Op::KeyAt { position, target } => {
            check(*position, integer, None)?;
            let Some(key) = definition.key else {
                return Err(malformed_mir("core collection key read has no key"));
            };
            check(*target, key, Some(false))?;
            if local_in(function, *target)?.writable {
                return Err(malformed_mir("collection key is exposed writable"));
            }
        }
        Op::ValueAt { position, target } => {
            check(*position, integer, None)?;
            check(*target, definition.value, Some(false))?;
            if local_in(function, *target)?.writable {
                return Err(malformed_mir("core collection read is exposed writable"));
            }
        }
        Op::Exchange {
            position,
            value,
            previous,
        } => {
            check(*position, integer, None)?;
            check(*value, definition.value, Some(true))?;
            check(*previous, definition.value, Some(true))?;
        }
        Op::HashNext {
            hash,
            previous,
            target,
        } => {
            if !definition.uses_core_hash() {
                return Err(malformed_mir("hash probe targets an ordered collection"));
            }
            check(*hash, hash_type, None)?;
            check(*previous, integer, None)?;
            check(*target, integer, None)?;
        }
        Op::HashPosition { slot, target } => {
            if !definition.uses_core_hash() {
                return Err(malformed_mir("hash position targets an ordered collection"));
            }
            check(*slot, integer, None)?;
            check(*target, integer, None)?;
        }
    }
    Ok(())
}

pub(super) fn validate(
    program: &mir::Program,
    function: &mir::Function,
    plan: &mir::CoreValueCallPlan,
) -> Result<(), BackendError> {
    let receiver = local_in(function, plan.receiver)?;
    let result = local_in(function, plan.result)?;
    let interface = program
        .interface_types
        .get(plan.interface.0)
        .ok_or_else(|| malformed_mir("core operation selects an unknown interface"))?;
    let method = interface
        .methods
        .get(plan.slot)
        .ok_or_else(|| malformed_mir("core operation selects an unknown requirement"))?;
    let contract = function_type_in(program, method.signature)?;
    let requirement = crate::compiler_known_contracts::interfaces()
        .find(|interface| interface.name == plan.operation.contract())
        .and_then(|interface| {
            interface
                .requirements
                .iter()
                .find(|method| method.name == plan.operation.method())
        })
        .expect("core operation has a compiler-owned requirement");
    if method.requirement != requirement.span
        || method.name != plan.operation.method()
        || method.writable_receiver
        || receiver.owned
        || receiver.writable
        || !receiver.synthetic
        || !result.synthetic
        || contract.return_borrow.is_some()
    {
        return Err(malformed_mir(
            "core operation lacks a readonly nominal contract selection",
        ));
    }
    let valid_result = match plan.operation {
        CoreValueOperation::Equal => result.ty == mir::Type::Scalar(mir::ScalarType::Bool),
        CoreValueOperation::Hash => {
            result.ty == mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::UInt64))
        }
        CoreValueOperation::Compare => {
            contract.return_type == mir::ReturnType::Value(result.ty)
                && matches!(result.ty, mir::Type::Scalar(mir::ScalarType::Enum(id))
                if program.enums.get(id.0).is_some_and(|definition|
                    definition.source_span.source == SOURCE_ID && definition.name == "Ordering"))
        }
        CoreValueOperation::Clone => {
            result.ty == receiver.ty && result.owned && method.exact_dynamic_return
        }
    };
    if !valid_result {
        return Err(malformed_mir(
            "core operation has an invalid result type or ownership",
        ));
    }
    let block = block_in(function, plan.call)?;
    block_in(function, plan.success)?;
    let args = match plan.target {
        mir::CoreValueCallTarget::Direct {
            function: target,
            vtable,
        } => {
            let table = program
                .interface_vtables
                .get(vtable.0)
                .ok_or_else(|| malformed_mir("core operation lacks a checked vtable"))?;
            let selected = function_in(program, target)?;
            let entry = table
                .methods
                .get(plan.slot)
                .and_then(|id| program.functions.get(id.0))
                .ok_or_else(|| malformed_mir("core operation vtable has no selected entry"))?;
            if table.interface != plan.interface
                || !matches!(receiver.ty, mir::Type::Class(class) if table.implementing_type == mir::ImplementingType::Class(class))
                || selected.source_span != entry.source_span
                || selected.return_type != mir::ReturnType::Value(result.ty)
                || selected.return_borrow.is_some()
            {
                return Err(malformed_mir(
                    "core operation direct target is not its checked implementation",
                ));
            }
            if selected.checked_effects.is_empty() {
                if plan.success != plan.call {
                    return Err(malformed_mir(
                        "ordinary core call has an invalid success block",
                    ));
                }
                block
                    .statements
                    .iter()
                    .find_map(|statement| match statement {
                        mir::Statement::AssignLocal {
                            target: local,
                            value,
                        } if *local == plan.result => direct_call(value)
                            .filter(|(id, _)| *id == target)
                            .map(|(_, args)| args),
                        _ => None,
                    })
                    .ok_or_else(|| {
                        malformed_mir("core operation proof does not describe its ordinary call")
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
                        && *result == Some(plan.result)
                        && *success == plan.success =>
                    {
                        args
                    }
                    _ => {
                        return Err(malformed_mir(
                            "core operation proof does not describe its checked call",
                        ))
                    }
                }
            }
        }
        mir::CoreValueCallTarget::Erased => {
            if receiver.ty != mir::Type::Interface(plan.interface) {
                return Err(malformed_mir(
                    "erased core operation has a different receiver contract",
                ));
            }
            let (callee, signature, args, result, success) = match &block.terminator {
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
                _ => return Err(malformed_mir("erased core operation has no indirect call")),
            };
            if *signature != method.signature
                || *result != Some(plan.result)
                || *success != plan.success
                || !matches!(callee, mir::IndirectCallee::InterfaceMethod { interface, slot, .. }
                    if *interface == plan.interface && *slot == plan.slot)
            {
                return Err(malformed_mir(
                    "erased core operation selected a different requirement",
                ));
            }
            args
        }
    };
    if args.is_empty()
        || args
            .iter()
            .any(|value| value.ty().has_move_ownership() && !value.borrows_move_value())
    {
        return Err(malformed_mir(
            "core operation must borrow its receiver and Move arguments",
        ));
    }
    let receiver_argument = args[0].direct_place_local();
    if receiver_argument != Some(plan.receiver)
        && !(plan.target == mir::CoreValueCallTarget::Erased
            && block.statements.iter().any(|statement| {
                matches!(statement,
            mir::Statement::AssignLocal { target, value } if Some(*target) == receiver_argument
                && value.direct_place_local() == Some(plan.receiver) && value.borrows_move_value())
            }))
    {
        return Err(malformed_mir(
            "core operation invokes a different receiver from its selection proof",
        ));
    }
    Ok(())
}

fn direct_call(value: &mir::Rvalue) -> Option<(mir::FunctionId, &Vec<mir::Rvalue>)> {
    use mir::*;
    match value {
        Rvalue::Value(ValueExpression::Bool(BoolExpression::Call { function, args }))
        | Rvalue::Value(ValueExpression::Integer(IntegerExpression::Call {
            function, args, ..
        }))
        | Rvalue::Value(ValueExpression::Enum(EnumExpression::Call { function, args, .. }))
        | Rvalue::Class(ClassExpression::Call { function, args, .. })
        | Rvalue::Interface(InterfaceExpression {
            value: InterfaceValue::Call { function, args, .. },
            ..
        }) => Some((*function, args)),
        _ => None,
    }
}

pub(super) fn validate_duplication(
    program: &mir::Program,
    function: &mir::Function,
    plan: &mir::ElementDuplicationPlan,
) -> Result<(), BackendError> {
    let source = local_in(function, plan.source)?;
    let result = local_in(function, plan.result)?;
    if source.owned
        || source.writable
        || !source.synthetic
        || !result.owned
        || !result.synthetic
        || source.ty != result.ty
    {
        return Err(malformed_mir(
            "element duplication must borrow its source and own its result",
        ));
    }
    let call = block_in(function, plan.call)?;
    let candidates = call
        .statements
        .iter()
        .filter_map(|statement| match statement {
            mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::CoreValue(core))
                if core.operation == CoreValueOperation::Clone
                    && call.statements.iter().any(|statement| matches!(statement,
                        mir::Statement::AssignLocal { target, value } if *target == core.receiver
                            && value.direct_place_local() == Some(plan.source) && value.borrows_move_value())) =>
            {
                Some(core)
            }
            _ => None,
        }).collect::<Vec<_>>();
    let [core] = candidates.as_slice() else {
        return Err(malformed_mir(
            "element duplication has no unique selected Cloneable call for its source",
        ));
    };
    validate(program, function, core)?;
    if !call.statements.iter().any(|statement| {
        matches!(statement,
        mir::Statement::AssignLocal { target, value } if *target == core.receiver
            && value.direct_place_local() == Some(plan.source) && value.borrows_move_value())
    }) {
        return Err(malformed_mir(
            "element duplication clones a different source",
        ));
    }
    let Some(absent) = plan.absent else {
        if plan.call != plan.entry || plan.exit != core.success || plan.result != core.result {
            return Err(malformed_mir(
                "element duplication has a different clone result or continuation",
            ));
        }
        return Ok(());
    };
    let condition = match source.ty {
        mir::Type::NullableClass(class) => mir::BoolExpression::NullableClassIsPresent(Box::new(
            mir::NullableClassExpression::Local {
                class,
                local: plan.source,
                transfer: false,
            },
        )),
        mir::Type::NullableInterface(interface) => mir::BoolExpression::NullableErrorIsPresent(
            Box::new(mir::NullableInterfaceExpression {
                interface,
                value: mir::NullableInterfaceValue::Local {
                    local: plan.source,
                    transfer: false,
                },
            }),
        ),
        _ => {
            return Err(malformed_mir(
                "nullable duplication has a non-nullable source",
            ))
        }
    };
    if !matches!(&block_in(function, plan.entry)?.terminator, mir::Terminator::Branch {
        condition: actual, then_block, else_block,
    } if *actual == condition && *then_block == plan.call && *else_block == absent)
    {
        return Err(malformed_mir(
            "nullable duplication must test presence before cloning",
        ));
    }
    let absent = block_in(function, absent)?;
    if !absent.statements.iter().any(|statement| match statement {
        mir::Statement::AssignLocal { target, value } if *target == plan.result => matches!(
            value,
            mir::Rvalue::NullableClass(mir::NullableClassExpression::Null(_))
                | mir::Rvalue::NullableInterface(mir::NullableInterfaceExpression {
                    value: mir::NullableInterfaceValue::Null,
                    ..
                })
        ),
        _ => false,
    }) || !matches!(absent.terminator, mir::Terminator::Jump(target) if target == plan.exit)
    {
        return Err(malformed_mir(
            "nullable duplication must preserve absence without cloning",
        ));
    }
    let success = block_in(function, core.success)?;
    if !success.statements.iter().any(|statement| match statement {
        mir::Statement::AssignLocal { target, value }
            if *target == plan.result && !value.borrows_move_value() =>
        {
            match value {
                mir::Rvalue::NullableClass(mir::NullableClassExpression::Class(
                    mir::ClassExpression::Local {
                        local,
                        transfer: true,
                        ..
                    },
                )) => *local == core.result,
                mir::Rvalue::NullableInterface(mir::NullableInterfaceExpression {
                    value:
                        mir::NullableInterfaceValue::Present(mir::InterfaceValue::Local {
                            local,
                            transfer: true,
                        }),
                    ..
                }) => *local == core.result,
                _ => false,
            }
        }
        _ => false,
    }) || !matches!(success.terminator, mir::Terminator::Jump(target) if target == plan.exit)
    {
        return Err(malformed_mir(
            "nullable duplication must wrap its owned clone result",
        ));
    }
    Ok(())
}

pub(super) fn validate_fill(
    program: &mir::Program,
    function: &mir::Function,
    plan: &mir::CollectionBuildPlan,
) -> Result<(), BackendError> {
    let output = local_in(function, plan.output)?;
    let mir::Type::Collection(collection) = output.ty else {
        return Err(malformed_mir("sequence fill has no collection output"));
    };
    let source = local_in(function, plan.source)?;
    let doria_int = mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::Int64));
    let definition = collection_in(program, collection)?;
    let source_matches = match plan.kind {
        mir::CollectionBuildKind::Fill => source.ty == definition.value && definition.key.is_none(),
        mir::CollectionBuildKind::PreservingFrom => {
            let mir::Type::Collection(source_type) = source.ty else {
                return Err(malformed_mir(
                    "preserving construction source is not a collection",
                ));
            };
            let source_type = collection_in(program, source_type)?;
            source_type.value == definition.value
                && source_type.key == definition.key
                && match definition.kind {
                    mir::CollectionKind::SortedDictionary => {
                        source_type.kind == mir::CollectionKind::Dictionary
                    }
                    mir::CollectionKind::Set
                    | mir::CollectionKind::SortedSet
                    | mir::CollectionKind::PriorityQueue
                    | mir::CollectionKind::Deque => matches!(
                        source_type.kind,
                        mir::CollectionKind::List | mir::CollectionKind::TypedArray
                    ),
                    _ => false,
                }
        }
    };
    if !output.owned
        || !output.synthetic
        || !output.writable
        || !source_matches
        || local_in(function, plan.count)?.ty != doria_int
        || local_in(function, plan.index)?.ty != doria_int
    {
        return Err(malformed_mir(
            "sequence fill has invalid source, capacity, or ownership",
        ));
    }
    let setup = block_in(function, plan.setup)?;
    if plan.kind == mir::CollectionBuildKind::PreservingFrom
        && !setup.statements.iter().any(|statement| matches!(statement,
            mir::Statement::AssignLocal { target, value: mir::Rvalue::Value(mir::ValueExpression::Integer(
                mir::IntegerExpression::Use { operand: mir::Operand::CollectionLength(source), .. })) }
                if *target == plan.count && *source == plan.source)) {
        return Err(malformed_mir("preserving construction must visit the source length"));
    }
    if !setup.statements.iter().any(|statement| matches!(statement,
        mir::Statement::AssignLocal { target, value: mir::Rvalue::Collection(
            mir::CollectionExpression::ConstructionCapacity { count, collection: ty, .. }) }
                if *target == plan.output && *ty == collection && integer_expression_reads_local(count, plan.count)))
        || !setup.statements.iter().any(|statement| matches!(statement,
            mir::Statement::AssignLocal { target, value: mir::Rvalue::Value(mir::ValueExpression::Integer(value)) }
                if *target == plan.index && integer_expression_is_constant(value, 0)))
        || !matches!(setup.terminator, mir::Terminator::Jump(target) if target == plan.header) {
        return Err(malformed_mir("sequence fill must allocate its empty live prefix and start at zero"));
    }
    if !matches!(&block_in(function, plan.header)?.terminator,
        mir::Terminator::Branch { condition: mir::BoolExpression::Compare { op: mir::CompareOp::Less, left, right }, then_block, else_block }
            if value_expression_reads_integer_local(left, plan.index) && value_expression_reads_integer_local(right, plan.count)
                && *then_block == plan.body && *else_block == plan.exit)
    {
        return Err(malformed_mir(
            "sequence fill must bound every initialization by its capacity",
        ));
    }
    let body = block_in(function, plan.body)?;
    let duplication = body
        .statements
        .iter()
        .find_map(|statement| match statement {
            mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::ElementDuplication(
                duplicate,
            )) if duplicate.exit == plan.append => Some(duplicate),
            _ => None,
        })
        .ok_or_else(|| malformed_mir("sequence fill must duplicate once per initialized slot"))?;
    validate_duplication(program, function, duplication)?;
    let core = block_in(function, duplication.call)?
        .statements
        .iter()
        .find_map(|statement| match statement {
            mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::CoreValue(core))
                if core.operation == CoreValueOperation::Clone =>
            {
                Some(core)
            }
            _ => None,
        })
        .ok_or_else(|| malformed_mir("collection construction has no clone call"))?;
    let mut region = HashSet::from([plan.body, plan.append, duplication.call, core.success]);
    region.extend(duplication.absent);
    for block in &function.blocks {
        for target in terminator_targets(&block.terminator) {
            if (target == plan.header && block.id != plan.setup && block.id != plan.append)
                || (target == plan.body && block.id != plan.header)
                || (target == plan.exit && block.id != plan.header)
                || (region.contains(&target) && target != plan.body && !region.contains(&block.id))
            {
                return Err(malformed_mir(
                    "collection construction has an unproved entry into its initialization region",
                ));
            }
        }
        if region.contains(&block.id) && block.statements.iter().any(|statement| matches!(statement,
            mir::Statement::AssignLocal { target, .. } if *target == plan.source || *target == plan.count)) {
            return Err(malformed_mir("collection construction overwrites its source or bound"));
        }
        if (region.contains(&block.id) || block.id == plan.setup)
            && block
                .statements
                .iter()
                .any(|statement| statement_drops_local(statement, plan.output))
        {
            return Err(malformed_mir(
                "collection construction drops its output before finishing",
            ));
        }
    }
    if !body.statements.iter().any(|statement| {
        matches!(statement,
        mir::Statement::AssignLocal { target, value } if *target == duplication.source
            && value.borrows_move_value() && match plan.kind {
                mir::CollectionBuildKind::Fill => value.direct_place_local() == Some(plan.source),
                mir::CollectionBuildKind::PreservingFrom => rvalue_reads_collection_value_at(value, plan.source, plan.index),
            })
    }) {
        return Err(malformed_mir("sequence fill duplicates a different source"));
    }
    let append = block_in(function, plan.append)?;
    if !append.statements.iter().any(|statement| matches!(statement,
        mir::Statement::CollectionAdd { collection, value, index: key, op: mir::CollectionMutationOp::Initialize }
            if *collection == plan.output && value.direct_place_local() == Some(duplication.result) && !value.borrows_move_value()
                && match (definition.key, key) {
                    (None, None) => true,
                    (Some(ty), Some(value)) if collection_type_is_copy(ty) => value.direct_place_local().is_some_and(|local|
                        body.statements.iter().any(|statement| matches!(statement,
                            mir::Statement::AssignLocal { target, value } if *target == local && rvalue_reads_collection_key_at(value, plan.source, plan.index)))),
                    _ => false,
                }))
        || !append.statements.iter().any(|statement| matches!(statement,
            mir::Statement::AssignLocal { target, value: mir::Rvalue::Value(mir::ValueExpression::Integer(
                mir::IntegerExpression::Binary { op: mir::IntegerBinaryOp::Add, left, right, .. })) }
                    if *target == plan.index && integer_expression_reads_local(left, plan.index) && integer_expression_is_constant(right, 1)))
        || !matches!(append.terminator, mir::Terminator::Jump(target) if target == plan.header) {
        return Err(malformed_mir("sequence fill must initialize one owned element before advancing"));
    }
    let mut initializations = 0;
    let mut index_writes = 0;
    let mut output_writes = 0;
    for block in &function.blocks {
        for statement in &block.statements {
            match statement {
                mir::Statement::CollectionAdd {
                    collection,
                    op: mir::CollectionMutationOp::Initialize,
                    ..
                } if *collection == plan.output => initializations += 1,
                mir::Statement::AssignLocal { target, .. } if *target == plan.index => {
                    index_writes += 1
                }
                mir::Statement::AssignLocal { target, .. } if *target == plan.output => {
                    output_writes += 1
                }
                _ => {}
            }
        }
    }
    if initializations != 1 || index_writes != 2 || output_writes != 1 {
        return Err(malformed_mir(
            "sequence fill initialization state is overwritten or duplicated",
        ));
    }
    if let mir::Terminator::CheckedCall { failure, .. }
    | mir::Terminator::CheckedIndirectCall { failure, .. } =
        block_in(function, duplication.call)?.terminator
    {
        if block_in(function, failure)?
            .statements
            .iter()
            .filter(|statement| statement_drops_local(statement, plan.output))
            .count()
            != 1
        {
            return Err(malformed_mir(
                "sequence fill checked failure must drop its initialized prefix once",
            ));
        }
    }
    Ok(())
}

/// Capacity-only allocations are private to one proven construction region.
/// Finish moves the completed allocation into a distinct ordinary owner; all
/// other escapes and observations of the partial output are rejected.
pub(super) fn validate_constructions(function: &mir::Function) -> Result<(), BackendError> {
    let mut plans = HashMap::new();
    for block in &function.blocks {
        for statement in &block.statements {
            if let mir::Statement::ControlFlowPlan(mir::ControlFlowPlan::CollectionBuild(plan)) =
                statement
            {
                if block.id != plan.setup || plans.insert(plan.output, plan).is_some() {
                    return Err(malformed_mir(
                        "collection construction has duplicate or misplaced ownership proofs",
                    ));
                }
            }
        }
    }
    let mut finished = HashSet::new();
    let mut allocated = HashSet::new();
    for block in &function.blocks {
        for (position, statement) in block.statements.iter().enumerate() {
            let accesses = collect_statement_class_local_accesses(statement);
            for capacity in &accesses.construction_capacities {
                let mir::Statement::AssignLocal {
                    target,
                    value: mir::Rvalue::Collection(value),
                } = statement
                else {
                    return Err(malformed_mir(
                        "collection capacity allocation must initialize its proven owner directly",
                    ));
                };
                if value != *capacity
                    || !plans.get(target).is_some_and(|plan| plan.setup == block.id)
                    || !allocated.insert(*target)
                {
                    return Err(malformed_mir(
                        "collection capacity allocation has no unique construction proof",
                    ));
                }
            }
            for local in &accesses.resource_reads {
                let Some(plan) = plans.get(local) else {
                    continue;
                };
                if accesses
                    .resource_reads
                    .iter()
                    .filter(|found| *found == local)
                    .count()
                    != 1
                {
                    return Err(malformed_mir(
                        "collection construction duplicates its unfinished output",
                    ));
                }
                match statement {
                    mir::Statement::CollectionAdd {
                        collection,
                        op: mir::CollectionMutationOp::Initialize,
                        ..
                    } if collection == local && block.id == plan.append => {}
                    mir::Statement::AssignLocal {
                        target,
                        value:
                            mir::Rvalue::Collection(mir::CollectionExpression::FinishConstruction {
                                source,
                                ..
                            }),
                    } if source == local
                        && target != local
                        && block.id == plan.exit
                        && position == 0 =>
                    {
                        if !local_in(function, *target)?.owned || !finished.insert(*source) {
                            return Err(malformed_mir(
                                "collection construction must transfer into one distinct owner",
                            ));
                        }
                    }
                    _ => {
                        return Err(malformed_mir(
                            "unfinished collection is observed or escapes its construction region",
                        ))
                    }
                }
            }
        }
        let accesses = collect_terminator_class_local_accesses(&block.terminator);
        if !accesses.construction_capacities.is_empty()
            || accesses
                .resource_reads
                .iter()
                .any(|local| plans.contains_key(local))
        {
            return Err(malformed_mir(
                "unfinished collection escapes through control flow",
            ));
        }
    }
    if plans
        .keys()
        .any(|local| !allocated.contains(local) || !finished.contains(local))
    {
        return Err(malformed_mir(
            "collection construction must allocate and finish exactly once",
        ));
    }
    Ok(())
}
