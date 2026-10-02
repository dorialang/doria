//! A stack environment must not leave the activation which owns its storage.
//! Track may-contain-stack values through the CFG, including local moves and joins.

use super::*;

pub(super) fn validate(program: &mir::Program) -> Result<(), BackendError> {
    if !program.closure_descriptors.iter().any(|descriptor| {
        descriptor.environment_placement == mir::ClosureEnvironmentPlacement::Stack
    }) {
        return Ok(());
    }
    for function in &program.functions {
        let mut incoming = vec![None; function.blocks.len()];
        incoming[function.entry_block.0] = Some(HashSet::new());
        let mut pending = VecDeque::from([function.entry_block]);
        while let Some(id) = pending.pop_front() {
            let mut state = incoming[id.0].clone().unwrap();
            let block = &function.blocks[id.0];
            for statement in &block.statements {
                let accesses = collect_statement_class_local_accesses(statement);
                validate_calls(program, &accesses, &state)?;
                match statement {
                    mir::Statement::AssignLocal { target, value } => {
                        let carries = carries(program, value, &state);
                        if carries && function.params.contains(target) {
                            return Err(escape());
                        }
                        assign(&mut state, *target, carries);
                    }
                    mir::Statement::AssignLocalGroup { targets, value } => {
                        let carries = carries(program, value, &state);
                        for target in targets {
                            assign(&mut state, *target, carries);
                        }
                    }
                    mir::Statement::DropFunction { local, .. } => {
                        state.remove(local);
                    }
                    mir::Statement::BindPayloadEnumFields {
                        source, targets, ..
                    } => {
                        let carries = state.contains(source);
                        for target in targets {
                            assign(&mut state, *target, carries);
                        }
                    }
                    mir::Statement::ExtractErrorObject { target, error, .. } => {
                        let carries = state.contains(error);
                        assign(&mut state, *target, carries);
                    }
                    mir::Statement::AssignProperty { .. }
                    | mir::Statement::AssignStatic { .. }
                    | mir::Statement::CollectionAdd { .. }
                    | mir::Statement::CollectionSet { .. }
                    | mir::Statement::AssignCollectionIndex { .. }
                        if contains(program, &accesses, &state) =>
                    {
                        return Err(escape())
                    }
                    _ if !accesses.collection_mutations.is_empty()
                        && contains(program, &accesses, &state) =>
                    {
                        return Err(escape())
                    }
                    _ => {}
                }
            }
            let accesses = collect_terminator_class_local_accesses(&block.terminator);
            validate_calls(program, &accesses, &state)?;
            match &block.terminator {
                mir::Terminator::Return(value) if carries(program, value, &state) => {
                    return Err(escape())
                }
                mir::Terminator::PropagateError { error } if state.contains(error) => {
                    return Err(escape())
                }
                mir::Terminator::CheckedCall {
                    function: callee,
                    result: Some(result),
                    ..
                } => {
                    let callee = &program.functions[callee.0];
                    let retains = callee.return_borrow.is_some()
                        || retained::contract(callee).is_some_and(|plan| !plan.returns.is_empty());
                    let contains = retains && contains(program, &accesses, &state);
                    assign(&mut state, *result, contains);
                }
                mir::Terminator::IndirectCall {
                    function_type,
                    result: Some(result),
                    ..
                }
                | mir::Terminator::CheckedIndirectCall {
                    function_type,
                    result: Some(result),
                    ..
                } => {
                    let contains = program.function_types[function_type.0]
                        .return_borrow
                        .is_some()
                        && contains(program, &accesses, &state);
                    assign(&mut state, *result, contains);
                }
                mir::Terminator::CheckedConstruct { result, .. } => {
                    let contains = contains(program, &accesses, &state);
                    assign(&mut state, *result, contains);
                }
                _ => {}
            }
            for successor in terminator_targets(&block.terminator) {
                match &mut incoming[successor.0] {
                    None => {
                        incoming[successor.0] = Some(state.clone());
                        pending.push_back(successor);
                    }
                    Some(existing) => {
                        let length = existing.len();
                        existing.extend(state.iter().copied());
                        if length != existing.len() {
                            pending.push_back(successor);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

fn assign(state: &mut HashSet<mir::LocalId>, local: mir::LocalId, carries: bool) {
    if carries {
        state.insert(local);
    } else {
        state.remove(&local);
    }
}

fn contains(
    program: &mir::Program,
    accesses: &ClassLocalAccesses<'_>,
    state: &HashSet<mir::LocalId>,
) -> bool {
    accesses.closure_constructions.iter().any(|descriptor| {
        program.closure_descriptors[descriptor.0].environment_placement
            == mir::ClosureEnvironmentPlacement::Stack
    }) || accesses
        .borrowed()
        .chain(accesses.transferred())
        .chain(accesses.resource_reads.iter().copied())
        .any(|local| state.contains(&local))
}

fn carries(program: &mir::Program, value: &mir::Rvalue, state: &HashSet<mir::LocalId>) -> bool {
    if !value.ty().has_move_ownership() {
        return false;
    }
    let mut accesses = ClassLocalAccesses::default();
    collect_rvalue_class_local_accesses(value, &mut accesses);
    contains(program, &accesses, state)
}

fn validate_calls(
    program: &mir::Program,
    accesses: &ClassLocalAccesses<'_>,
    state: &HashSet<mir::LocalId>,
) -> Result<(), BackendError> {
    for access in &accesses.accesses {
        let ClassLocalAccess::Call(target, args) = access else {
            continue;
        };
        for (index, argument) in args.iter().enumerate() {
            let mode = match target {
                CallTarget::Direct {
                    function,
                    parameter_offset,
                } => program.functions[function.0].parameter_modes[index + parameter_offset],
                CallTarget::Indirect(function_type) => {
                    program.function_types[function_type.0].parameters[index].mode
                }
            };
            if mode == mir::FunctionParameterMode::Take && carries(program, argument, state) {
                return Err(escape());
            }
        }
    }
    Ok(())
}

fn escape() -> BackendError {
    malformed_mir("stack closure environment escapes its owning activation")
}
