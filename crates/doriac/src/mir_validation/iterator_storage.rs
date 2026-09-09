//! A bounded, shared storage proof for directly acquired collection cursors.
//!
//! Only one-shot acquisitions whose carrier stays in protocol calls qualify.
//! Returned cursors, owned forwarding, captures, and repeated acquisitions keep
//! their ordinary heap representation. This is not general escape analysis.

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackCollectionIterator {
    pub source: mir::LocalId,
    pub cursor: mir::LocalId,
}

/// Derive storage only from validated MIR; no source annotation can waive a
/// lifetime, carrier-ownership, or protocol-selection invariant.
pub fn stack_collection_iterators(
    program: &mir::Program,
    function: &mir::Function,
) -> Vec<StackCollectionIterator> {
    let mut result = Vec::new();
    for block in &function.blocks {
        for acquisition in &block.statements {
            let mir::Statement::AssignLocal {
                target: cursor,
                value:
                    mir::Rvalue::Interface(mir::InterfaceExpression {
                        value: mir::InterfaceValue::NewCollectionIterator { source, .. },
                        ..
                    }),
            } = acquisition
            else {
                continue;
            };
            if !function.locals[cursor.0].owned || repeated_block(function, block.id) {
                continue;
            }
            let mut aliases = HashSet::from([*cursor]);
            loop {
                let before = aliases.len();
                for statement in function.blocks.iter().flat_map(|block| &block.statements) {
                    if let Some((target, source)) = borrowed_alias(statement) {
                        if aliases.contains(&source) && !function.locals[target.0].owned {
                            aliases.insert(target);
                        }
                    }
                }
                if aliases.len() == before {
                    break;
                }
            }
            let mut definitions = HashSet::new();
            let valid = function.blocks.iter().all(|body| {
                body.statements.iter().all(|statement| {
                    let alias = borrowed_alias(statement).is_some_and(|(target, source)| {
                        aliases.contains(&target) && aliases.contains(&source)
                    });
                    if let mir::Statement::AssignLocal { target, .. } = statement {
                        if aliases.contains(target) {
                            if !definitions.insert(*target) || (!alias && *target != *cursor) {
                                return false;
                            }
                            if *target == *cursor && !std::ptr::eq(statement, acquisition) {
                                return false;
                            }
                        }
                    }
                    let writes = match statement {
                        mir::Statement::AssignLocalGroup { targets, .. }
                        | mir::Statement::BindPayloadEnumFields { targets, .. } => targets.clone(),
                        mir::Statement::CoreCollection { operation, .. } => operation.outputs(),
                        mir::Statement::BindClosureEnvironment { bindings, .. } => {
                            bindings.iter().map(|(_, local)| *local).collect()
                        }
                        mir::Statement::ExtractErrorObject { target, .. } => vec![*target],
                        _ => Vec::new(),
                    };
                    if writes.iter().any(|local| aliases.contains(local)) {
                        return false;
                    }
                    alias || !touches(&collect_statement_class_local_accesses(statement), &aliases)
                }) && protocol_only(program, &body.terminator, &aliases)
            });
            if valid {
                result.push(StackCollectionIterator {
                    source: *source,
                    cursor: *cursor,
                });
            }
        }
    }
    // Sources identify the allocation site for expression lowering.
    result.retain(|plan| {
        function
            .blocks
            .iter()
            .flat_map(|block| {
                block
                    .statements
                    .iter()
                    .map(collect_statement_class_local_accesses)
                    .chain(std::iter::once(collect_terminator_class_local_accesses(
                        &block.terminator,
                    )))
            })
            .flat_map(|accesses| accesses.iterator_acquisitions)
            .filter(|source| *source == plan.source)
            .count()
            == 1
    });
    result
}

fn borrowed_alias(statement: &mir::Statement) -> Option<(mir::LocalId, mir::LocalId)> {
    match statement {
        mir::Statement::AssignLocal {
            target,
            value:
                mir::Rvalue::Interface(mir::InterfaceExpression {
                    value:
                        mir::InterfaceValue::Local {
                            local,
                            transfer: false,
                        },
                    ..
                }),
        } => Some((*target, *local)),
        _ => None,
    }
}

fn touches(accesses: &ClassLocalAccesses<'_>, aliases: &HashSet<mir::LocalId>) -> bool {
    accesses
        .borrowed()
        .chain(accesses.transferred())
        .chain(accesses.resource_reads.iter().copied())
        .chain(accesses.resource_transfers.iter().copied())
        .any(|local| aliases.contains(&local))
}

fn protocol_only(
    program: &mir::Program,
    terminator: &mir::Terminator,
    aliases: &HashSet<mir::LocalId>,
) -> bool {
    let (result, error) = match terminator {
        mir::Terminator::CheckedCall { result, error, .. }
        | mir::Terminator::CheckedIndirectCall { result, error, .. }
        | mir::Terminator::CheckedIo { result, error, .. } => (*result, Some(*error)),
        mir::Terminator::IndirectCall { result, .. } => (*result, None),
        mir::Terminator::CheckedConstruct { result, error, .. } => (Some(*result), Some(*error)),
        _ => (None, None),
    };
    if result
        .into_iter()
        .chain(error)
        .any(|local| aliases.contains(&local))
    {
        return false;
    }
    let accesses = collect_terminator_class_local_accesses(terminator);
    if !touches(&accesses, aliases) {
        return true;
    }
    let (mir::Terminator::IndirectCall {
        callee:
            mir::IndirectCallee::InterfaceMethod {
                receiver,
                interface,
                slot,
            },
        args,
        ..
    }
    | mir::Terminator::CheckedIndirectCall {
        callee:
            mir::IndirectCallee::InterfaceMethod {
                receiver,
                interface,
                slot,
            },
        args,
        ..
    }) = terminator
    else {
        return false;
    };
    aliases.contains(receiver)
        && matches!(args.as_slice(),
        [mir::Rvalue::Interface(mir::InterfaceExpression { value: mir::InterfaceValue::Local { local, transfer: false }, .. })]
        if aliases.contains(local))
        && matches!(
            retained::iteration_operation(program, *interface, *slot),
            Some(
                crate::compiler_known_contracts::IterationOperation::HasCurrent
                    | crate::compiler_known_contracts::IterationOperation::GetCurrent
                    | crate::compiler_known_contracts::IterationOperation::Advance
            )
        )
}

fn repeated_block(function: &mir::Function, start: mir::BlockId) -> bool {
    let mut pending = terminator_targets(&function.blocks[start.0].terminator);
    let mut seen = HashSet::new();
    while let Some(block) = pending.pop() {
        if block == start {
            return true;
        }
        if seen.insert(block) {
            pending.extend(terminator_targets(&function.blocks[block.0].terminator));
        }
    }
    false
}
