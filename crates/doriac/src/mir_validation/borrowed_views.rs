//! Preserve the owner or access lease behind borrowed nominal views, including
//! views materialized in separate locals for interface dispatch and narrowing.
use super::*;

#[derive(Clone, PartialEq, Eq)]
struct State {
    roots: Vec<HashSet<mir::LocalId>>,
    loans: Vec<HashSet<mir::RetainedSource>>,
    ended: HashSet<mir::LocalId>,
}

impl State {
    fn new(program: &mir::Program, function: &mir::Function) -> Self {
        Self {
            roots: function
                .locals
                .iter()
                .map(|local| HashSet::from([local.id]))
                .collect(),
            loans: function
                .locals
                .iter()
                .map(|local| {
                    if function.params.contains(&local.id)
                        && retained::carries_source(program, local.ty)
                    {
                        HashSet::from([mir::RetainedSource {
                            local: local.id,
                            inherited: true,
                        }])
                    } else {
                        HashSet::new()
                    }
                })
                .collect(),
            ended: HashSet::new(),
        }
    }

    fn end(&mut self, local: mir::LocalId) {
        self.ended.insert(local);
        for (index, roots) in self.roots.iter().enumerate() {
            if roots.contains(&local)
                || self.loans[index]
                    .iter()
                    .any(|loan| !loan.inherited && loan.local == local)
            {
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
        let mut sources = escaping_class_local_borrows(program, value)?;
        if value.borrows_move_value() {
            sources.extend(value.direct_place_local());
        }
        Ok(sources
            .into_iter()
            .flat_map(|local| self.roots[local.0].iter().copied())
            .collect())
    }

    fn conflicts(&self, local: mir::LocalId) -> Result<(), BackendError> {
        if self.loans.iter().enumerate().any(|(index, loans)| {
            !self.ended.contains(&mir::LocalId(index))
                && loans
                    .iter()
                    .any(|loan| !loan.inherited && self.roots[local.0].contains(&loan.local))
        }) {
            return Err(malformed_mir(format!(
                "source local{} is mutated or ended while an iterator retains it",
                local.0
            )));
        }
        Ok(())
    }

    fn accesses(
        &mut self,
        program: &mir::Program,
        function: &mir::Function,
        accesses: &ClassLocalAccesses<'_>,
        checking: bool,
    ) -> Result<(), BackendError> {
        if checking {
            for value in &accesses.independent_values {
                retained::validate_independent(
                    function,
                    &retained::value_sources(program, value, &self.roots, &self.loans)?,
                )?;
            }
            for local in &accesses.resource_reads {
                if self.ended.contains(local) {
                    return Err(malformed_mir(
                        "resource view is used after its source ownership ended",
                    ));
                }
            }
            for local in accesses
                .collection_mutations
                .iter()
                .chain(&accesses.resource_transfers)
            {
                self.conflicts(*local)?;
            }
        }
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
                        if checking {
                            self.conflicts(local)?;
                        }
                        self.end(local);
                    }
                }
                ClassLocalAccess::Call(callee, args) => {
                    if checking {
                        retained::validate_call_inputs(
                            program,
                            function,
                            callee,
                            args,
                            &self.roots,
                            &self.loans,
                        )?;
                    }
                    let mut borrows = HashMap::new();
                    let mut mutated = HashSet::new();
                    for (local, mode) in borrowed_call_locals(program, callee, args)? {
                        if matches!(mode, ClassBorrowMode::Writable) {
                            if checking {
                                self.conflicts(local)?;
                            }
                            mutated.extend(self.roots[local.0].iter().copied());
                        }
                        if checking && self.ended.contains(&local) {
                            return Err(malformed_mir(format!(
                                "view local{} is used after its source ownership ended",
                                local.0
                            )));
                        }
                        for root in &self.roots[local.0] {
                            if checking
                                && borrows
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
                    for local in &function.locals {
                        if !local.owned
                            && !function.params.contains(&local.id)
                            && !mutated.contains(&local.id)
                            && self.roots[local.id.0]
                                .iter()
                                .any(|root| mutated.contains(root))
                        {
                            self.ended.insert(local.id);
                        }
                    }
                }
                ClassLocalAccess::BeginCall => {}
            }
        }
        for local in &accesses.resource_transfers {
            self.end(*local);
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
            mir::Statement::CoreCollection {
                collection,
                operation:
                    mir::CoreCollectionOperation::KeyAt { target, .. }
                    | mir::CoreCollectionOperation::ValueAt { target, .. },
            } => Some((*target, self.roots[collection.0].clone())),
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
        let assigned_loans = if let mir::Statement::AssignLocal { value, .. } = statement {
            retained::value_sources(program, value, &self.roots, &self.loans)?
        } else {
            HashSet::new()
        };
        if checking {
            if let mir::Statement::CoreCollection {
                collection,
                operation,
            } = statement
            {
                if operation.mutates() {
                    self.conflicts(*collection)?;
                }
                for input in operation.transfers() {
                    retained::validate_independent(function, &self.loans[input.0])?;
                }
            }
            for source in &assigned_loans {
                if !source.inherited && self.ended.contains(&source.local) {
                    return Err(malformed_mir("iterator retains an ended source owner"));
                }
            }
            let stored = match statement {
                mir::Statement::AssignProperty {
                    property, value, ..
                } if !property_in(program, property.class, *property)?.borrowed_source => {
                    Some(value)
                }
                mir::Statement::CollectionAdd { value, op, .. }
                    if *op != mir::CollectionMutationOp::Remove =>
                {
                    Some(value)
                }
                mir::Statement::CollectionSet { value, .. }
                | mir::Statement::AssignCollectionIndex { value, .. }
                | mir::Statement::AssignStatic { value, .. } => Some(value),
                _ => None,
            };
            if let Some(value) = stored {
                retained::validate_independent(
                    function,
                    &retained::value_sources(program, value, &self.roots, &self.loans)?,
                )?;
            }
            if let mir::Statement::CollectionSet { key, .. } = statement {
                retained::validate_independent(
                    function,
                    &retained::value_sources(program, key, &self.roots, &self.loans)?,
                )?;
            }
            match statement {
                mir::Statement::CollectionAdd { collection, .. }
                | mir::Statement::CollectionSet { collection, .. }
                | mir::Statement::AssignCollectionIndex { collection, .. }
                | mir::Statement::CollectionClear { collection, .. } => {
                    self.conflicts(*collection)?
                }
                mir::Statement::AssignProperty { object, .. } => self.conflicts(*object)?,
                _ => {}
            }
        }
        self.accesses(
            program,
            function,
            &collect_statement_class_local_accesses(statement),
            checking,
        )?;
        if let Some((target, roots)) = assignment {
            if checking {
                self.conflicts(target)?;
            }
            self.assign(target, roots);
            self.loans[target.0] = assigned_loans;
        }
        match statement {
            mir::Statement::CoreCollection { operation, .. } => {
                if !matches!(
                    operation,
                    mir::CoreCollectionOperation::KeyAt { .. }
                        | mir::CoreCollectionOperation::ValueAt { .. }
                ) {
                    for target in operation.outputs() {
                        if checking {
                            self.conflicts(target)?;
                        }
                        let roots = if local_in(function, target)?.owned {
                            HashSet::from([target])
                        } else {
                            HashSet::new()
                        };
                        self.assign(target, roots);
                        self.loans[target.0].clear();
                    }
                }
            }
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
            | mir::Statement::DropFunction { local, .. }
            | mir::Statement::DropCollection { local, .. }
            | mir::Statement::DropSharedReference { local, .. }
            | mir::Statement::DropWeakReference { local, .. }
            | mir::Statement::DropWritableSharedReference { local, .. }
            | mir::Statement::DropWritableWeakReference { local, .. }
            | mir::Statement::DropSharedReferenceAccess { local, .. } => {
                if checking {
                    self.conflicts(*local)?;
                }
                self.end(*local);
            }
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
            let loans = match terminator {
                mir::Terminator::CheckedConstruct {
                    constructor,
                    args,
                    success,
                    ..
                } if *success == target => retained::call_sources(
                    program,
                    *constructor,
                    args,
                    true,
                    &state.roots,
                    &state.loans,
                )?,
                mir::Terminator::CheckedCall {
                    function: callee,
                    args,
                    success,
                    ..
                } if *success == target => retained::call_sources(
                    program,
                    *callee,
                    args,
                    false,
                    &state.roots,
                    &state.loans,
                )?,
                mir::Terminator::IndirectCall {
                    callee,
                    args,
                    continuation: success,
                    ..
                }
                | mir::Terminator::CheckedIndirectCall {
                    callee,
                    args,
                    success,
                    ..
                } if *success == target => {
                    retained::indirect_sources(program, callee, args, &state.roots, &state.loans)?
                }
                _ => HashSet::new(),
            };
            state.assign(result, roots);
            state.loans[result.0] = loans;
        }
        Ok(state)
    }
}

pub(super) fn validate(
    program: &mir::Program,
    function: &mir::Function,
) -> Result<(), BackendError> {
    let mut entries = vec![None::<State>; function.blocks.len()];
    entries[function.entry_block.0] = Some(State::new(program, function));
    let mut pending = VecDeque::from([function.entry_block]);
    while let Some(id) = pending.pop_front() {
        let mut state = entries[id.0].clone().expect("reachable view state");
        let block = block_in(function, id)?;
        for statement in &block.statements {
            state.statement(program, function, statement, false)?;
        }
        state.accesses(
            program,
            function,
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
                    for (loans, incoming) in existing.loans.iter_mut().zip(outgoing.loans) {
                        loans.extend(incoming);
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
        if let mir::Terminator::Return(value) = &block.terminator {
            retained::validate_return(
                function,
                &retained::value_sources(program, value, &state.roots, &state.loans)?,
            )?;
        }
        retained::validate_indirect_inputs(
            program,
            function,
            &block.terminator,
            &state.roots,
            &state.loans,
        )?;
        state.accesses(
            program,
            function,
            &collect_terminator_class_local_accesses(&block.terminator),
            true,
        )?;
    }
    Ok(())
}
