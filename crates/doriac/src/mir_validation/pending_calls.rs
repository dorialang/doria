//! A once-call carrier remains owned during operand evaluation. Its local is
//! retired by a transfer or guarded drop on every non-aborting exit path.

use super::*;

pub(super) fn validate(program: &mir::Program) -> Result<(), BackendError> {
    for function in &program.functions {
        let pending = function
            .blocks
            .iter()
            .filter_map(|block| match &block.terminator {
                mir::Terminator::IndirectCall {
                    callee,
                    invocation_mode: mir::FunctionInvocationMode::Once,
                    ..
                }
                | mir::Terminator::CheckedIndirectCall {
                    callee,
                    invocation_mode: mir::FunctionInvocationMode::Once,
                    ..
                } => match callee {
                    mir::IndirectCallee::Closure(mir::FunctionExpression::Local {
                        local,
                        transfer: true,
                        ..
                    }) if function.locals[local.0].owned => Some(*local),
                    _ => None,
                },
                _ => None,
            })
            .collect::<HashSet<_>>();
        if pending.is_empty() {
            continue;
        }
        validate_function(function, &pending)?;
    }
    Ok(())
}

fn validate_function(
    function: &mir::Function,
    pending: &HashSet<mir::LocalId>,
) -> Result<(), BackendError> {
    let mut incoming = vec![None; function.blocks.len()];
    incoming[function.entry_block.0] = Some(
        function
            .params
            .iter()
            .copied()
            .filter(|local| pending.contains(local))
            .collect::<HashSet<_>>(),
    );
    let mut work = VecDeque::from([function.entry_block]);
    while let Some(id) = work.pop_front() {
        let mut live = incoming[id.0].clone().unwrap();
        let block = &function.blocks[id.0];
        for statement in &block.statements {
            retire_transfers(
                &collect_statement_class_local_accesses(statement),
                &mut live,
            );
            match statement {
                mir::Statement::DropFunction { local, .. } => {
                    live.remove(local);
                }
                mir::Statement::AssignLocal { target, .. } if pending.contains(target) => {
                    acquire(*target, &mut live)?
                }
                mir::Statement::AssignLocalGroup { targets, .. } => {
                    for target in targets.iter().filter(|target| pending.contains(target)) {
                        acquire(*target, &mut live)?;
                    }
                }
                _ => {}
            }
        }
        retire_transfers(
            &collect_terminator_class_local_accesses(&block.terminator),
            &mut live,
        );
        if matches!(
            block.terminator,
            mir::Terminator::Return(_)
                | mir::Terminator::ReturnVoid
                | mir::Terminator::PropagateError { .. }
        ) && !live.is_empty()
        {
            return Err(malformed_mir(format!(
                "pending once-call carrier is not consumed or released before exit from {}",
                function.name
            )));
        }
        let result = match block.terminator {
            mir::Terminator::CheckedCall {
                result: Some(result),
                success,
                ..
            }
            | mir::Terminator::CheckedIndirectCall {
                result: Some(result),
                success,
                ..
            } => Some((result, success)),
            mir::Terminator::IndirectCall {
                result: Some(result),
                continuation,
                ..
            } => Some((result, continuation)),
            _ => None,
        };
        for successor in terminator_targets(&block.terminator) {
            let mut edge = live.clone();
            if let Some((result, success)) = result {
                if successor == success && pending.contains(&result) {
                    acquire(result, &mut edge)?;
                }
            }
            match &mut incoming[successor.0] {
                None => {
                    incoming[successor.0] = Some(edge);
                    work.push_back(successor);
                }
                Some(existing) => {
                    let size = existing.len();
                    existing.extend(edge);
                    if existing.len() != size {
                        work.push_back(successor);
                    }
                }
            }
        }
    }
    Ok(())
}

fn acquire(local: mir::LocalId, live: &mut HashSet<mir::LocalId>) -> Result<(), BackendError> {
    if !live.insert(local) {
        return Err(malformed_mir(
            "pending once-call carrier is overwritten before its ownership handoff",
        ));
    }
    Ok(())
}

fn retire_transfers(accesses: &ClassLocalAccesses<'_>, live: &mut HashSet<mir::LocalId>) {
    let mut conditional = 0;
    let mut storage = accesses.storage_accesses.iter().peekable();
    for position in 0..=accesses.accesses.len() {
        while storage.peek().is_some_and(|(at, _)| *at == position) {
            let (_, access) = storage.next().unwrap();
            if conditional == 0 {
                if let StorageAccess::Transfer(local) = access {
                    live.remove(local);
                }
            }
        }
        match accesses.accesses.get(position) {
            Some(ClassLocalAccess::BeginConditional) => conditional += 1,
            Some(ClassLocalAccess::EndConditional) => conditional -= 1,
            Some(ClassLocalAccess::Transfer(local)) if conditional == 0 => {
                live.remove(local);
            }
            _ => {}
        }
    }
}
