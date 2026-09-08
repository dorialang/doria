//! Preserve the owner or access lease behind borrowed nominal views, including
//! views materialized in separate locals for interface dispatch and narrowing.
use super::*;

#[derive(Clone, PartialEq, Eq)]
struct State {
    roots: Vec<HashSet<mir::LocalId>>,
    ended: HashSet<mir::LocalId>,
}

impl State {
    fn new(function: &mir::Function) -> Self {
        Self {
            roots: function
                .locals
                .iter()
                .map(|local| HashSet::from([local.id]))
                .collect(),
            ended: HashSet::new(),
        }
    }

    fn end(&mut self, local: mir::LocalId) {
        self.ended.insert(local);
        for (index, roots) in self.roots.iter().enumerate() {
            if roots.contains(&local) {
                self.ended.insert(mir::LocalId(index));
            }
        }
    }

    fn assign(&mut self, local: mir::LocalId, roots: HashSet<mir::LocalId>) {
        self.end(local);
        self.roots[local.0] = if roots.is_empty() {
            HashSet::from([local])
        } else {
            roots
        };
        self.ended.remove(&local);
    }

    fn sources(
        &self,
        program: &mir::Program,
        value: &mir::Rvalue,
    ) -> Result<HashSet<mir::LocalId>, BackendError> {
        Ok(escaping_class_local_borrows(program, value)?
            .into_iter()
            .flat_map(|local| self.roots[local.0].iter().copied())
            .collect())
    }

    fn accesses(
        &mut self,
        program: &mir::Program,
        accesses: &ClassLocalAccesses<'_>,
        checking: bool,
    ) -> Result<(), BackendError> {
        for access in accesses.iter() {
            match access {
                ClassLocalAccess::Borrow(local)
                | ClassLocalAccess::PropertyBorrow(local, _)
                | ClassLocalAccess::Transfer(local) => {
                    if checking && self.ended.contains(&local) {
                        return Err(malformed_mir(format!(
                            "view local{} is used after its source ownership ended",
                            local.0
                        )));
                    }
                    if matches!(access, ClassLocalAccess::Transfer(_)) {
                        self.end(local);
                    }
                }
                ClassLocalAccess::Call(callee, args) if checking => {
                    let mut borrows = HashMap::new();
                    for (local, mode) in borrowed_call_locals(program, callee, args)? {
                        if self.ended.contains(&local) {
                            return Err(malformed_mir(format!(
                                "view local{} is used after its source ownership ended",
                                local.0
                            )));
                        }
                        for root in &self.roots[local.0] {
                            if borrows
                                .get(root)
                                .is_some_and(|previous| mode.conflicts_with(*previous))
                            {
                                return Err(class_access_error(
                                    "call through borrowed views",
                                    "takes overlapping writable borrows of",
                                    *root,
                                ));
                            }
                            borrows.insert(*root, mode);
                        }
                    }
                }
                ClassLocalAccess::BeginCall | ClassLocalAccess::Call(_, _) => {}
            }
        }
        Ok(())
    }

    fn statement(
        &mut self,
        program: &mir::Program,
        function: &mir::Function,
        statement: &mir::Statement,
        checking: bool,
    ) -> Result<(), BackendError> {
        if checking {
            if let mir::Statement::AssignLocal { value, .. } = statement {
                for local in escaping_class_local_borrows(program, value)? {
                    if self.ended.contains(&local) {
                        return Err(malformed_mir(format!(
                            "view local{} is used after its source ownership ended",
                            local.0
                        )));
                    }
                }
            }
        }
        let assignment = match statement {
            mir::Statement::AssignLocal { target, value }
                if !local_in(function, *target)?.owned =>
            {
                Some((*target, self.sources(program, value)?))
            }
            mir::Statement::AssignLocal { target, value } => {
                let mut roots = if value.mixed_ownership() == mir::MixedOwnership::ShellOnly {
                    self.sources(program, value)?
                } else if let Some(source) = value.direct_place_local() {
                    // Moving an owned shell changes its home, not the lifetime
                    // of any payload it borrows from another owner or lease.
                    self.roots[source.0]
                        .iter()
                        .copied()
                        .filter(|root| *root != source)
                        .collect()
                } else {
                    HashSet::new()
                };
                roots.insert(*target);
                Some((*target, roots))
            }
            _ => None,
        };
        self.accesses(
            program,
            &collect_statement_class_local_accesses(statement),
            checking,
        )?;
        if let Some((target, roots)) = assignment {
            self.assign(target, roots);
        }
        match statement {
            mir::Statement::AssignLocalGroup { targets, .. } => {
                for target in targets {
                    self.assign(*target, HashSet::new());
                }
            }
            mir::Statement::ExtractErrorObject { target, .. } => {
                self.assign(*target, HashSet::new())
            }
            mir::Statement::DropClass { local, .. }
            | mir::Statement::DropError { local }
            | mir::Statement::DropMixed { local }
            | mir::Statement::DropCollection { local, .. }
            | mir::Statement::DropSharedReference { local, .. }
            | mir::Statement::DropWeakReference { local, .. }
            | mir::Statement::DropWritableSharedReference { local, .. }
            | mir::Statement::DropWritableWeakReference { local, .. }
            | mir::Statement::DropSharedReferenceAccess { local, .. } => self.end(*local),
            _ => {}
        }
        Ok(())
    }

    fn edge(
        &self,
        program: &mir::Program,
        function: &mir::Function,
        terminator: &mir::Terminator,
        target: mir::BlockId,
    ) -> Result<Self, BackendError> {
        let mut state = self.clone();
        let (result, borrow) = match terminator {
            mir::Terminator::CheckedCall {
                function: callee,
                args,
                result,
                success,
                ..
            } if *success == target => {
                let borrow = function_in(program, *callee)?
                    .return_borrow
                    .map(|borrow| borrowed_call_rvalue_source(program, *callee, args, borrow))
                    .transpose()?;
                (*result, borrow)
            }
            mir::Terminator::IndirectCall {
                function_type,
                args,
                result,
                continuation: success,
                ..
            }
            | mir::Terminator::CheckedIndirectCall {
                function_type,
                args,
                result,
                success,
                ..
            } if *success == target => {
                let borrow = match function_type_in(program, *function_type)?.return_borrow {
                    Some(mir::ReturnBorrow {
                        source: mir::BorrowSource::Parameter(index),
                        ..
                    }) => args.get(index),
                    _ => None,
                };
                (*result, borrow)
            }
            mir::Terminator::CheckedCall { error, failure, .. }
            | mir::Terminator::CheckedIndirectCall { error, failure, .. }
            | mir::Terminator::CheckedConstruct { error, failure, .. }
            | mir::Terminator::CheckedIo { error, failure, .. }
                if *failure == target =>
            {
                (Some(*error), None)
            }
            mir::Terminator::CheckedConstruct {
                result, success, ..
            } if *success == target => (Some(*result), None),
            mir::Terminator::CheckedIo {
                result, success, ..
            } if *success == target => (*result, None),
            _ => (None, None),
        };
        if let Some(result) = result {
            let roots = if !local_in(function, result)?.owned {
                borrow
                    .map(|value| state.sources(program, value))
                    .transpose()?
                    .unwrap_or_default()
            } else {
                HashSet::new()
            };
            state.assign(result, roots);
        }
        Ok(state)
    }
}

pub(super) fn validate(
    program: &mir::Program,
    function: &mir::Function,
) -> Result<(), BackendError> {
    let mut entries = vec![None::<State>; function.blocks.len()];
    entries[function.entry_block.0] = Some(State::new(function));
    let mut pending = VecDeque::from([function.entry_block]);
    while let Some(id) = pending.pop_front() {
        let mut state = entries[id.0].clone().expect("reachable view state");
        let block = block_in(function, id)?;
        for statement in &block.statements {
            state.statement(program, function, statement, false)?;
        }
        state.accesses(
            program,
            &collect_terminator_class_local_accesses(&block.terminator),
            false,
        )?;
        for target in analysis_terminator_targets(&block.terminator, true) {
            let outgoing = state.edge(program, function, &block.terminator, target)?;
            let changed = match &mut entries[target.0] {
                None => {
                    entries[target.0] = Some(outgoing);
                    true
                }
                Some(existing) => {
                    let before = existing.clone();
                    for (roots, incoming) in existing.roots.iter_mut().zip(outgoing.roots) {
                        roots.extend(incoming);
                    }
                    existing.ended.extend(outgoing.ended);
                    *existing != before
                }
            };
            if changed {
                pending.push_back(target);
            }
        }
    }
    for (block, entry) in function.blocks.iter().zip(entries) {
        let Some(mut state) = entry else {
            continue;
        };
        for statement in &block.statements {
            state.statement(program, function, statement, true)?;
        }
        state.accesses(
            program,
            &collect_terminator_class_local_accesses(&block.terminator),
            true,
        )?;
    }
    Ok(())
}
