//! Configuration retains descriptors, never copies bytes or performs I/O.
use super::*;
use crate::compiler_known_io::{IoErrorReason, IoOperation, IoTarget};

pub(super) struct PlanOwner {
    pub(super) owner: u64,
    pub(super) identity: u64,
    pub(super) entries: Vec<(u64, ByteRegionValue, bool)>,
}

impl Interpreter<'_> {
    pub(super) fn transfer_plan_key(
        &mut self,
        plan: mir::LocalId,
        position: mir::LocalId,
        key: crate::inline_value::InlineValueType,
    ) -> Result<CheckedIoResult, InterpreterError> {
        let LocalValue::Class { object, .. } = read_local(&self.current_frame()?.locals, plan)?
        else {
            return Err(InterpreterError::new("transfer key lookup has no plan"));
        };
        let LocalValue::Scalar(mir::ScalarValue::Integer(position)) =
            read_local(&self.current_frame()?.locals, position)?
        else {
            return Err(InterpreterError::new(
                "transfer key lookup has no int position",
            ));
        };
        let plan = self
            .transfer_plans
            .get(object)
            .ok_or_else(|| InterpreterError::new("transfer key lookup plan was released"))?;
        let position = usize::try_from(position.signed_value()).ok();
        let entry = position.and_then(|position| plan.entries.get(position));
        let reading = matches!(
            key,
            crate::inline_value::InlineValueType::ReadTransferKey(_)
        );
        if !entry.is_some_and(|(_, _, direction)| *direction == reading) {
            return Ok(CheckedIoResult::Error(self.allocate_io_error(
                IoOperation::Access,
                IoTarget::TransferPlan,
                IoErrorReason::InvalidInput,
                None,
                None,
            )?));
        }
        let bits =
            u128::from(plan.identity) | ((position.expect("validated position") as u128) << 64);
        Ok(CheckedIoResult::Success(Some(LocalValue::Scalar(
            mir::ScalarValue::Inline(crate::inline_value::InlineValue { ty: key, bits }),
        ))))
    }

    pub(super) fn configure_transfer_plan(
        &mut self,
        class: crate::class_layout::ClassId,
        previous: Option<mir::LocalId>,
        key: mir::LocalId,
        region: mir::LocalId,
        reading: bool,
    ) -> Result<CheckedIoResult, InterpreterError> {
        let previous = previous
            .map(|previous| -> Result<_, InterpreterError> {
                let LocalValue::Class { object, .. } = self.read_or_take_local(previous, true)?
                else {
                    return Err(InterpreterError::new("transfer append has no owned plan"));
                };
                self.heap
                    .remove(&object)
                    .ok_or_else(|| InterpreterError::new("transfer append owner was released"))?;
                self.transfer_plans
                    .remove(&object)
                    .ok_or_else(|| InterpreterError::new("transfer append has no configuration"))
            })
            .transpose()?;
        let LocalValue::Scalar(mir::ScalarValue::Inline(key)) =
            read_local(&self.current_frame()?.locals, key)?
        else {
            return Err(InterpreterError::new("transfer setup has no inline key"));
        };
        let owner = key.bits as u64;
        let key = (key.bits >> 64) as u64;
        let region = self.region_local(region)?;
        let mut conflicting = false;
        if let Some(plan) = &previous {
            for (_, prior, prior_reading) in &plan.entries {
                conflicting |=
                    self.transfer_regions_conflict(region, reading, *prior, *prior_reading)?;
            }
        }
        if owner == 0 || previous.as_ref().is_some_and(|plan| plan.owner != owner) || conflicting {
            return Ok(CheckedIoResult::Error(self.allocate_io_error(
                IoOperation::Configure,
                IoTarget::TransferPlan,
                IoErrorReason::InvalidInput,
                None,
                None,
            )?));
        }
        let mut plan = previous.unwrap_or(PlanOwner {
            owner,
            identity: self.next_object as u64,
            entries: Vec::new(),
        });
        plan.entries.push((key, region, reading));
        let object = self.next_object;
        self.next_object += 1;
        self.heap
            .insert(object, ObjectValue::new(class, Vec::new()));
        self.transfer_plans.insert(object, plan);
        Ok(CheckedIoResult::Success(Some(LocalValue::Class {
            object,
            class,
        })))
    }

    pub(super) fn transfer_regions_conflict(
        &self,
        region: ByteRegionValue,
        reading: bool,
        prior: ByteRegionValue,
        prior_reading: bool,
    ) -> Result<bool, InterpreterError> {
        let source = region
            .source
            .ok_or_else(|| InterpreterError::new("transfer region has no storage"))?;
        let prior_source = prior
            .source
            .ok_or_else(|| InterpreterError::new("retained transfer region has no storage"))?;
        let storage = self
            .borrowed_collections
            .get(&source.collection)
            .ok_or_else(|| InterpreterError::new("transfer region loan was released"))?;
        let prior_storage = self
            .borrowed_collections
            .get(&prior_source.collection)
            .ok_or_else(|| InterpreterError::new("retained transfer region loan was released"))?;
        let end = source
            .index
            .checked_add(region.length)
            .ok_or_else(|| InterpreterError::new("transfer region range overflow"))?;
        let prior_end = prior_source
            .index
            .checked_add(prior.length)
            .ok_or_else(|| InterpreterError::new("retained transfer region range overflow"))?;
        Ok(storage.entries.ptr_eq(&prior_storage.entries)
            && (reading || prior_reading)
            && region.length != 0
            && prior.length != 0
            && source.index < prior_end
            && prior_source.index < end)
    }
}
