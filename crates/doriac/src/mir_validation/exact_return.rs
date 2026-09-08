//! Prove interface `self` results from executable MIR, not their static type alone.

use super::*;

pub(super) fn validate(program: &mir::Program) -> Result<(), BackendError> {
    let mut proofs = Proofs {
        program,
        results: HashMap::new(),
    };
    for table in &program.interface_vtables {
        let mir::ImplementingType::Class(class) = table.implementing_type else {
            continue;
        };
        let contract = interface_in(program, table.interface)?;
        for (entry, method) in table.methods.iter().zip(&contract.methods) {
            if method.exact_dynamic_return && !proofs.function(*entry, class)? {
                return Err(malformed_mir(format!(
                    "interface self entry function{} does not prove exact dynamic class#{} on every return",
                    entry.0, class.0
                )));
            }
        }
    }
    Ok(())
}

struct Proofs<'a> {
    program: &'a mir::Program,
    results: HashMap<(mir::FunctionId, ClassId), bool>,
}

impl Proofs<'_> {
    fn function(&mut self, id: mir::FunctionId, class: ClassId) -> Result<bool, BackendError> {
        if let Some(result) = self.results.get(&(id, class)) {
            return Ok(*result);
        }
        // A recursive call alone is not evidence of an exact constructed result.
        self.results.insert((id, class), false);
        let function = function_in(self.program, id)?;
        if function.return_borrow.is_some() {
            return Ok(false);
        }
        if function.return_type == mir::ReturnType::Value(mir::Type::Class(class))
            && !class_in(self.program, class)?.is_open
        {
            self.results.insert((id, class), true);
            return Ok(true);
        }
        let mut entries = vec![None; function.blocks.len()];
        entries[function.entry_block.0] = Some(HashSet::new());
        let mut pending = VecDeque::from([function.entry_block]);
        while let Some(id) = pending.pop_front() {
            let block = block_in(function, id)?;
            let mut facts = entries[id.0].clone().expect("reachable block");
            for statement in &block.statements {
                self.statement(statement, class, &mut facts)?;
            }
            let accesses = collect_terminator_class_local_accesses(&block.terminator);
            self.invalidate(&accesses, &mut facts)?;
            let mut edge = |target: mir::BlockId, facts: &HashSet<(mir::LocalId, mir::Type)>| {
                if merge_definite_class_refinements(&mut entries[target.0], facts) {
                    pending.push_back(target);
                }
            };
            match &block.terminator {
                mir::Terminator::Jump(target) => edge(*target, &facts),
                mir::Terminator::Branch {
                    condition,
                    then_block,
                    else_block,
                } => {
                    for (target, value) in [(*then_block, true), (*else_block, false)] {
                        if constant_bool_expression(condition)
                            .is_none_or(|constant| constant == value)
                        {
                            edge(target, &facts);
                        }
                    }
                }
                mir::Terminator::IndirectCall {
                    result,
                    continuation,
                    ..
                } => {
                    Self::set_result(&mut facts, *result, class, false);
                    edge(*continuation, &facts);
                }
                mir::Terminator::CheckedCall {
                    function,
                    result,
                    error,
                    success,
                    failure,
                    ..
                } => {
                    let exact = function_in(self.program, *function)?.virtual_slot.is_none()
                        && self.function(*function, class)?;
                    Self::set_result(&mut facts, Some(*error), class, false);
                    Self::set_result(&mut facts, *result, class, false);
                    edge(*failure, &facts);
                    Self::set_result(&mut facts, *result, class, exact);
                    edge(*success, &facts);
                }
                mir::Terminator::CheckedConstruct {
                    class: constructed,
                    result,
                    error,
                    success,
                    failure,
                    ..
                } => {
                    Self::set_result(&mut facts, Some(*error), class, false);
                    Self::set_result(&mut facts, Some(*result), class, false);
                    edge(*failure, &facts);
                    Self::set_result(&mut facts, Some(*result), class, *constructed == class);
                    edge(*success, &facts);
                }
                mir::Terminator::CheckedIndirectCall {
                    result,
                    error,
                    success,
                    failure,
                    ..
                }
                | mir::Terminator::CheckedIo {
                    result,
                    error,
                    success,
                    failure,
                    ..
                } => {
                    Self::set_result(&mut facts, Some(*error), class, false);
                    Self::set_result(&mut facts, *result, class, false);
                    edge(*success, &facts);
                    edge(*failure, &facts);
                }
                mir::Terminator::ErrorSwitch {
                    cases,
                    catch_all,
                    fallback,
                    ..
                } => {
                    for target in cases
                        .iter()
                        .map(|(_, target)| target)
                        .chain(catch_all)
                        .chain([fallback])
                    {
                        edge(*target, &facts);
                    }
                }
                mir::Terminator::Return(_)
                | mir::Terminator::ReturnVoid
                | mir::Terminator::Panic { .. }
                | mir::Terminator::Unreachable
                | mir::Terminator::PropagateError { .. } => {}
            }
        }
        let mut result = true;
        for block in &function.blocks {
            let Some(mut facts) = entries[block.id.0].clone() else {
                continue;
            };
            for statement in &block.statements {
                self.statement(statement, class, &mut facts)?;
            }
            if let mir::Terminator::Return(value) = &block.terminator {
                result &= self.value(value, class, &facts)?;
            }
        }
        self.results.insert((id, class), result);
        Ok(result)
    }

    fn invalidate(
        &self,
        accesses: &ClassLocalAccesses<'_>,
        facts: &mut HashSet<(mir::LocalId, mir::Type)>,
    ) -> Result<(), BackendError> {
        invalidate_call_proofs(self.program, accesses, |local| {
            facts.retain(|(root, _)| *root != local)
        })?;
        for access in &accesses.accesses {
            if let ClassLocalAccess::Transfer(local) = access {
                facts.retain(|(root, _)| root != local);
            }
        }
        Ok(())
    }

    fn set_result(
        facts: &mut HashSet<(mir::LocalId, mir::Type)>,
        local: Option<mir::LocalId>,
        class: ClassId,
        exact: bool,
    ) {
        if let Some(local) = local {
            facts.retain(|(root, _)| *root != local);
            if exact {
                facts.insert((local, mir::Type::Class(class)));
            }
        }
    }

    fn statement(
        &mut self,
        statement: &mir::Statement,
        class: ClassId,
        facts: &mut HashSet<(mir::LocalId, mir::Type)>,
    ) -> Result<(), BackendError> {
        let exact = match statement {
            mir::Statement::AssignLocal { value, .. }
            | mir::Statement::AssignLocalGroup { value, .. } => self.value(value, class, facts)?,
            _ => false,
        };
        self.invalidate(&collect_statement_class_local_accesses(statement), facts)?;
        match statement {
            mir::Statement::AssignLocal { target, .. } => {
                Self::set_result(facts, Some(*target), class, exact)
            }
            mir::Statement::AssignLocalGroup { targets, .. } => {
                for target in targets {
                    Self::set_result(facts, Some(*target), class, exact);
                }
            }
            mir::Statement::DropClass { local, .. }
            | mir::Statement::DropError { local }
            | mir::Statement::ExtractErrorObject { target: local, .. } => {
                Self::set_result(facts, Some(*local), class, false)
            }
            _ => {}
        }
        Ok(())
    }

    fn value(
        &mut self,
        value: &mir::Rvalue,
        class: ClassId,
        facts: &HashSet<(mir::LocalId, mir::Type)>,
    ) -> Result<bool, BackendError> {
        match value {
            mir::Rvalue::Class(value) => self.class(value, class, facts),
            mir::Rvalue::Interface(value) => self.interface(&value.value, class, facts),
            _ => Ok(false),
        }
    }

    fn class(
        &mut self,
        value: &mir::ClassExpression,
        class: ClassId,
        facts: &HashSet<(mir::LocalId, mir::Type)>,
    ) -> Result<bool, BackendError> {
        if value.class() == class && !class_in(self.program, class)?.is_open {
            return Ok(true);
        }
        Ok(match value {
            mir::ClassExpression::New { concrete_class, .. } => *concrete_class == class,
            mir::ClassExpression::Local { local, .. }
            | mir::ClassExpression::NullableLocalAssumeNonNull { local, .. }
            | mir::ClassExpression::InterfacePayload { local, .. } => {
                facts.contains(&(*local, mir::Type::Class(class)))
            }
            mir::ClassExpression::InterfaceReceiver { vtable, .. } => {
                interface_vtable_in(self.program, *vtable)?.implementing_type
                    == mir::ImplementingType::Class(class)
            }
            mir::ClassExpression::Call { function, .. } => {
                function_in(self.program, *function)?.virtual_slot.is_none()
                    && self.function(*function, class)?
            }
            _ => false,
        })
    }

    fn interface(
        &mut self,
        value: &mir::InterfaceValue,
        class: ClassId,
        facts: &HashSet<(mir::LocalId, mir::Type)>,
    ) -> Result<bool, BackendError> {
        Ok(match value {
            mir::InterfaceValue::FromClass { object, .. } => self.class(object, class, facts)?,
            mir::InterfaceValue::Upcast { source, .. } => {
                self.interface(&source.value, class, facts)?
            }
            mir::InterfaceValue::Local { local, .. }
            | mir::InterfaceValue::NullableLocalAssumeNonNull { local, .. }
            | mir::InterfaceValue::NarrowedLocal { local, .. } => {
                facts.contains(&(*local, mir::Type::Class(class)))
            }
            mir::InterfaceValue::Call { function, .. } => {
                function_in(self.program, *function)?.virtual_slot.is_none()
                    && self.function(*function, class)?
            }
            _ => false,
        })
    }
}
