//! Configuration retains descriptors, never copies bytes or performs I/O.
use super::*;
use crate::compiler_known_io::{IoErrorReason, IoOperation, IoTarget};

/// One entry of a plan. Its position in the plan identifies it in keys.
#[derive(Debug, Clone, Copy)]
pub(super) enum PlanEntry {
    Transfer {
        registration: u64,
        region: ByteRegionValue,
        reading: bool,
    },
    /// A watched source's identity: its kind above bit 32, its token below.
    Watch(u64),
}

/// The `WatchSource` kind of a terminal raw mode; its token is the generation.
pub(super) const RAW_MODE_WATCH: u64 = 1;

pub(super) struct PlanOwner {
    /// The set whose registrations the plan's transfers name. A plan of
    /// watches alone names none and can begin on any set.
    pub(super) owner: Option<u64>,
    pub(super) identity: u64,
    pub(super) entries: Vec<PlanEntry>,
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
        let kind = entry.map(|entry| match entry {
            PlanEntry::Transfer { reading, .. } => u8::from(*reading),
            PlanEntry::Watch(_) => 2,
        });
        if kind.is_none() || kind != key.transfer_entry_kind() {
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

    /// An append consumes its plan whether or not the new entry is accepted.
    fn take_appended_plan(
        &mut self,
        previous: Option<mir::LocalId>,
    ) -> Result<Option<PlanOwner>, InterpreterError> {
        previous
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
            .transpose()
    }

    /// The new owner of a plan that gained `entry`.
    fn store_configured_plan(
        &mut self,
        class: crate::class_layout::ClassId,
        previous: Option<PlanOwner>,
        owner: Option<u64>,
        entry: PlanEntry,
    ) -> CheckedIoResult {
        let mut plan = previous.unwrap_or(PlanOwner {
            owner: None,
            identity: self.next_object as u64,
            entries: Vec::new(),
        });
        plan.owner = plan.owner.or(owner);
        plan.entries.push(entry);
        let object = self.next_object;
        self.next_object += 1;
        self.heap
            .insert(object, ObjectValue::new(class, Vec::new()));
        self.transfer_plans.insert(object, plan);
        CheckedIoResult::Success(Some(LocalValue::Class { object, class }))
    }

    /// Adds a watch. The plan keeps only the source's identity; the loan of
    /// the source itself is the compiler's (Decision 0140).
    pub(super) fn watch_transfer_plan(
        &mut self,
        class: crate::class_layout::ClassId,
        previous: Option<mir::LocalId>,
        identity: mir::LocalId,
    ) -> Result<CheckedIoResult, InterpreterError> {
        let previous = self.take_appended_plan(previous)?;
        let LocalValue::Scalar(mir::ScalarValue::Inline(identity)) =
            read_local(&self.current_frame()?.locals, identity)?
        else {
            return Err(InterpreterError::new("watch setup has no source identity"));
        };
        let identity = identity.bits as u64;
        if identity >> 32 != RAW_MODE_WATCH {
            return Ok(CheckedIoResult::Error(self.allocate_io_error(
                IoOperation::Configure,
                IoTarget::TransferPlan,
                IoErrorReason::InvalidInput,
                None,
                None,
            )?));
        }
        Ok(self.store_configured_plan(class, previous, None, PlanEntry::Watch(identity)))
    }

    pub(super) fn configure_transfer_plan(
        &mut self,
        class: crate::class_layout::ClassId,
        previous: Option<mir::LocalId>,
        key: mir::LocalId,
        region: mir::LocalId,
        reading: bool,
    ) -> Result<CheckedIoResult, InterpreterError> {
        let previous = self.take_appended_plan(previous)?;
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
            for entry in &plan.entries {
                if let PlanEntry::Transfer {
                    region: prior,
                    reading: prior_reading,
                    ..
                } = entry
                {
                    conflicting |=
                        self.transfer_regions_conflict(region, reading, *prior, *prior_reading)?;
                }
            }
        }
        let foreign = previous
            .as_ref()
            .and_then(|plan| plan.owner)
            .is_some_and(|bound| bound != owner);
        if owner == 0 || foreign || conflicting {
            return Ok(CheckedIoResult::Error(self.allocate_io_error(
                IoOperation::Configure,
                IoTarget::TransferPlan,
                IoErrorReason::InvalidInput,
                None,
                None,
            )?));
        }
        Ok(self.store_configured_plan(
            class,
            previous,
            Some(owner),
            PlanEntry::Transfer {
                registration: key,
                region,
                reading,
            },
        ))
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
