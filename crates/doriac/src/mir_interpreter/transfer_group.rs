//! Ordinary MIR callbacks surround virtual transfers over retained source loans.
use super::*;
use crate::compiler_known_io::{IoErrorReason, IoOperation, IoTarget};
use crate::compiler_known_stream::{self as stream, StreamOperation};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CheckedCall {
    pub(super) result: Option<mir::LocalId>,
    pub(super) error: mir::LocalId,
    pub(super) success: mir::BlockId,
    pub(super) failure: mir::BlockId,
    pub(super) span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    SetupSource,
    Source,
    Prepare,
    CompleteSubmitted,
    Complete { next: usize },
    ControlDrain { next: usize },
    CompleteResubmission,
    Observe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Callback {
    group: usize,
    position: usize,
    phase: Phase,
}

struct Entry {
    /// The entry's position in the plan, which counts watches too. Keys carry
    /// it; everything else addresses a transfer by its index in `entries`.
    position: usize,
    registration: u64,
    buffer: ByteRegionValue,
    region: ByteRegionValue,
    request: usize,
    reading: bool,
    prepared: bool,
    delivered: bool,
    ready: Option<FunctionOutcome>,
    completed_progress: Option<usize>,
}

impl Entry {
    fn resubmission_region(&self, offset: usize, length: usize) -> Option<ByteRegionValue> {
        if !self.delivered
            || self.prepared
            || self.ready.is_some()
            || offset.checked_add(length)? > self.buffer.length
        {
            return None;
        }
        let mut region = self.buffer;
        let source = region.source.as_mut()?;
        source.index = source.index.checked_add(offset)?;
        region.length = length;
        Some(region)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WatchState {
    /// Waiting for its source; reports `Ready` once the source is ready.
    Armed,
    /// The group stopped while the watch was armed: `Ready` is owed once.
    Retired,
    /// Its `Ready` was fetched; only a resubmission arms it again.
    Idle,
}

/// A watch of the plan (Decision 0140). It has no stream, request or buffer.
struct Watch {
    position: usize,
    /// The `WatchSource` word: its kind above bit 32, its token below.
    source: u64,
    state: WatchState,
}

/// The fetch a group is serving.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fetch {
    call: CheckedCall,
    result_type: mir::PayloadEnumType,
    /// Only an `Indefinite` wait keeps checking armed watches.
    indefinite: bool,
}

/// The kind of plan entry a key identifies, as the runtime numbers it.
const WRITE_ENTRY: u8 = 0;
const READ_ENTRY: u8 = 1;
const WATCH_ENTRY: u8 = 2;

pub(super) struct GroupOwner {
    pub(super) standard_claims: [bool; 3],
    set: usize,
    class: crate::class_layout::ClassId,
    identity: u64,
    /// The plan position and kind of the completion the last fetch returned.
    observed: Option<(usize, u8)>,
    entries: Vec<Entry>,
    watches: Vec<Watch>,
    setup: Option<CheckedCall>,
    setup_error: Option<InterfaceCarrier>,
    setup_reason: Option<IoErrorReason>,
    resubmission: Option<CheckedCall>,
    observation: Option<Fetch>,
    span: Span,
    deadline: Option<i128>,
    cancellation: Option<u128>,
    stopped: bool,
    wait_failure: Option<(IoErrorReason, Option<i64>)>,
    wait_failure_reported: bool,
}

impl Interpreter<'_> {
    fn group_control_stopped(&mut self, group: usize) -> Result<bool, InterpreterError> {
        let owner = &self.transfer_groups[&group];
        if owner.stopped {
            return Ok(true);
        }
        let requested = owner
            .cancellation
            .map(|identity| self.cancellation_state_requested(identity))
            .transpose()?
            .unwrap_or(false);
        let deadline = owner.deadline;
        let mut failure = None;
        let stopped = if requested {
            true
        } else if let Some(deadline) = deadline {
            match (self.monotonic_clock)() {
                Ok(sample) => i128::from(sample) >= deadline,
                Err(error) => {
                    failure = Some(match error {
                        doria_time::ClockFailure::System { system_code } => {
                            (IoErrorReason::Other, Some(system_code))
                        }
                        doria_time::ClockFailure::Unsupported => (IoErrorReason::Unsupported, None),
                        doria_time::ClockFailure::InvalidReading => (IoErrorReason::Other, None),
                    });
                    true
                }
            }
        } else {
            false
        };
        if stopped {
            self.mark_group_stopped(group, failure);
        }
        Ok(stopped)
    }

    /// A stopped group waits on nothing. Like a pending transfer, a watch
    /// that was armed still reports its completion once.
    fn mark_group_stopped(&mut self, group: usize, failure: Option<(IoErrorReason, Option<i64>)>) {
        let owner = self.transfer_groups.get_mut(&group).unwrap();
        owner.stopped = true;
        owner.wait_failure = failure;
        for watch in &mut owner.watches {
            if watch.state == WatchState::Armed {
                watch.state = WatchState::Retired;
            }
        }
    }

    /// Whether an active transfer was submitted to a source that had nothing
    /// for it: it has neither an outcome nor a failure yet. Every fetch tries
    /// it again, so it completes once its source need not wait.
    fn transfer_awaits_source(&self, group: usize, position: usize) -> bool {
        let entry = &self.transfer_groups[&group].entries[position];
        let view = &self.pending_requests[&entry.request];
        entry.prepared
            && view.outcome.is_none()
            && view.failure.is_none()
            && matches!(
                view.selection,
                Some(file_stream::RequestSelection::Native {
                    prepared: false,
                    ..
                })
            )
    }

    /// What the first transfer still waiting on its source waits for.
    fn awaited_source(&self, group: usize) -> Result<Option<&'static str>, InterpreterError> {
        let owner = &self.transfer_groups[&group];
        for (position, entry) in owner.entries.iter().enumerate() {
            if entry.delivered || !self.transfer_awaits_source(group, position) {
                continue;
            }
            if let Some(file_stream::RequestSelection::Native { source, region, .. }) =
                self.pending_requests[&entry.request].selection
            {
                if let Some(wait) = self.readiness_source_read_wait(source, region.length)? {
                    return Ok(Some(wait));
                }
            }
        }
        Ok(None)
    }

    fn absent_transfer_outcome(&self, reading: bool) -> Result<FunctionOutcome, InterpreterError> {
        let name = if reading {
            "Doria\\Std\\Io\\ReadOutcome"
        } else {
            "Doria\\Std\\Io\\WriteOutcome"
        };
        let ty = self
            .program
            .enums
            .iter()
            .find(|definition| definition.name == name)
            .and_then(|definition| definition.payload_type())
            .ok_or_else(|| InterpreterError::new("stopped transfer has no canonical outcome"))?;
        Ok(FunctionOutcome::Value(LocalValue::NullablePayloadEnum {
            ty,
            value: None,
        }))
    }

    fn stop_virtual_group(
        &mut self,
        group: usize,
        first: usize,
    ) -> Result<StepOutcome, InterpreterError> {
        let owner = &self.transfer_groups[&group];
        if let Some(position) =
            (first..owner.entries.len()).find(|position| owner.entries[*position].prepared)
        {
            self.push_transfer_phase(Callback {
                group,
                position,
                phase: Phase::ControlDrain { next: position + 1 },
            })?;
            return Ok(StepOutcome::Continue);
        }
        let count = self.transfer_groups[&group].entries.len();
        for position in 0..count {
            let entry = &self.transfer_groups[&group].entries[position];
            if !entry.delivered && entry.ready.is_none() {
                let absent = self.absent_transfer_outcome(entry.reading)?;
                self.transfer_groups.get_mut(&group).unwrap().entries[position].ready =
                    Some(absent);
            }
        }
        if self.transfer_groups[&group].setup.is_some() {
            return self.finish_virtual_group_setup(group);
        }
        if self.transfer_groups[&group].resubmission.is_some() {
            return self.return_transfer_resubmission(group);
        }
        let observation = self
            .transfer_groups
            .get_mut(&group)
            .unwrap()
            .observation
            .take()
            .ok_or_else(|| InterpreterError::new("control drainage has no waiting caller"))?;
        self.fetch_virtual_completion(group, observation)
    }

    pub(super) fn observe_transfer_group(
        &mut self,
        group: mir::LocalId,
        wait: mir::LocalId,
        result_type: mir::PayloadEnumType,
        call: CheckedCall,
    ) -> Result<StepOutcome, InterpreterError> {
        let LocalValue::Class { object: group, .. } =
            read_local(&self.current_frame()?.locals, group)?
        else {
            return Err(InterpreterError::new("completion observation has no group"));
        };
        let group = *group;
        let LocalValue::PayloadEnum(wait) = read_local(&self.current_frame()?.locals, wait)? else {
            return Err(InterpreterError::new("completion observation has no Wait"));
        };
        // Most virtual transfers complete as they are submitted, but relative
        // waits still freeze their clock reading before observing any
        // completed operation.
        let indefinite = wait.case.index == 3;
        let relative_wait = if wait.case.index == 1 {
            let Some(LocalValue::Scalar(mir::ScalarValue::Inline(duration))) = wait.fields.first()
            else {
                return Err(InterpreterError::new("relative wait has no Duration"));
            };
            duration.bits as i64 >= 0
        } else {
            false
        };
        if relative_wait {
            if let Err(failure) = (self.monotonic_clock)() {
                self.transfer_groups.get_mut(&group).unwrap().observed = None;
                let (reason, code) = match failure {
                    doria_time::ClockFailure::System { system_code } => {
                        (IoErrorReason::Other, Some(system_code))
                    }
                    doria_time::ClockFailure::Unsupported => (IoErrorReason::Unsupported, None),
                    doria_time::ClockFailure::InvalidReading => (IoErrorReason::Other, None),
                };
                let error = self.allocate_io_error(
                    IoOperation::Wait,
                    IoTarget::ReadinessSet,
                    reason,
                    code,
                    None,
                )?;
                return self.finish_checked_io_result(
                    CheckedIoResult::Error(error),
                    call.result,
                    call.error,
                    call.success,
                    call.failure,
                );
            }
        }
        let observation = Fetch {
            call,
            result_type,
            indefinite,
        };
        let owner = &self.transfer_groups[&group];
        let active = owner.entries.iter().any(|entry| entry.prepared)
            || owner
                .watches
                .iter()
                .any(|watch| watch.state == WatchState::Armed);
        if active && self.group_control_stopped(group)? {
            let owner = self.transfer_groups.get_mut(&group).unwrap();
            owner.observed = None;
            owner.observation = Some(observation);
            owner.span = call.span;
            return self.stop_virtual_group(group, 0);
        }
        self.fetch_virtual_completion(group, observation)
    }

    /// Whether the source of an armed watch is ready now. A terminal raw mode
    /// is ready when its next event wait would not time out.
    fn watch_source_ready(&mut self, source: u64) -> Result<bool, InterpreterError> {
        if source >> 32 != transfer_plan::RAW_MODE_WATCH {
            return Err(InterpreterError::new("watch has an unknown source kind"));
        }
        Ok(self.terminal_watch_ready(source as u32))
    }

    /// The first watch, in plan order, whose completion can be reported: one
    /// the group retired while armed, or an armed one whose source is ready.
    fn reportable_watch(&mut self, group: usize) -> Result<Option<usize>, InterpreterError> {
        let count = self.transfer_groups[&group].watches.len();
        for index in 0..count {
            let watch = &self.transfer_groups[&group].watches[index];
            let (source, state) = (watch.source, watch.state);
            let reportable = match state {
                WatchState::Retired => true,
                WatchState::Armed => self.watch_source_ready(source)?,
                WatchState::Idle => false,
            };
            if reportable {
                return Ok(Some(index));
            }
        }
        Ok(None)
    }

    /// Reports `Completed(Ready($key))` and leaves the watch idle.
    fn finish_watch_observation(
        &mut self,
        group: usize,
        index: usize,
        observation: Fetch,
    ) -> Result<StepOutcome, InterpreterError> {
        let Fetch {
            call, result_type, ..
        } = observation;
        let types = stream::TransferObservationTypes::for_result(self.program, result_type)
            .ok_or_else(|| InterpreterError::new("completion has no canonical schema"))?;
        let owner = self.transfer_groups.get_mut(&group).unwrap();
        let watch = &mut owner.watches[index];
        watch.state = WatchState::Idle;
        let position = watch.position;
        owner.observed = Some((position, WATCH_ENTRY));
        let key = LocalValue::Scalar(mir::ScalarValue::Inline(crate::inline_value::InlineValue {
            ty: types.watch_key,
            bits: u128::from(owner.identity) | ((position as u128) << 64),
        }));
        let completion = Self::transfer_enum(types.completion, 2, vec![key]);
        self.finish_checked_io_result(
            CheckedIoResult::Success(Some(Self::transfer_enum(result_type, 0, vec![completion]))),
            call.result,
            call.error,
            call.success,
            call.failure,
        )
    }

    fn fetch_virtual_completion(
        &mut self,
        group: usize,
        observation: Fetch,
    ) -> Result<StepOutcome, InterpreterError> {
        let Fetch {
            call,
            result_type,
            indefinite,
        } = observation;
        let owner = self.transfer_groups.get_mut(&group).unwrap();
        owner.observed = None;
        if let Some((reason, code)) = owner.wait_failure.filter(|_| !owner.wait_failure_reported) {
            owner.wait_failure_reported = true;
            let error = self.allocate_io_error(
                IoOperation::Wait,
                IoTarget::ReadinessSet,
                reason,
                code,
                None,
            )?;
            return self.finish_checked_io_result(
                CheckedIoResult::Error(error),
                call.result,
                call.error,
                call.success,
                call.failure,
            );
        }
        // Watches report before queued transfers. The check happens here, as
        // the completion is reported, so events the program has already
        // drained are never announced.
        if let Some(index) = self.reportable_watch(group)? {
            return self.finish_watch_observation(group, index, observation);
        }
        // The first transfer, in plan order, with a completion to report. One
        // whose source had nothing for it is tried again, and passed over
        // while its source still has nothing.
        let mut position = None;
        for candidate in 0..self.transfer_groups[&group].entries.len() {
            if self.transfer_groups[&group].entries[candidate].delivered {
                continue;
            }
            if self.transfer_awaits_source(group, candidate) {
                if self.submit_virtual_transfer(group, candidate)? {
                    return Ok(StepOutcome::CleanExit);
                }
                if self.transfer_awaits_source(group, candidate) {
                    continue;
                }
            }
            position = Some(candidate);
            break;
        }
        let Some(position) = position else {
            let owner = &self.transfer_groups[&group];
            let waiting = owner
                .watches
                .iter()
                .any(|watch| watch.state == WatchState::Armed);
            let pending = owner.entries.iter().any(|entry| !entry.delivered);
            if waiting && indefinite {
                // Each check of a source that is not ready uses up one moment
                // in which it had nothing, so this ends when the source does.
                // No transfer can complete meanwhile: the program is here.
                loop {
                    if let Some(index) = self.reportable_watch(group)? {
                        return self.finish_watch_observation(group, index, observation);
                    }
                }
            }
            if pending && indefinite {
                // Only the group's deadline can end this wait. Waits here take
                // no time, so the deadline is reached now; without one the
                // wait never ends, and the run stops with the reason.
                if owner.deadline.is_none() {
                    let wait = self.awaited_source(group)?.ok_or_else(|| {
                        InterpreterError::new("a pending transfer waits on no source")
                    })?;
                    return Err(InterpreterError::new(wait));
                }
                self.mark_group_stopped(group, None);
                let owner = self.transfer_groups.get_mut(&group).unwrap();
                owner.observation = Some(observation);
                owner.span = call.span;
                return self.stop_virtual_group(group, 0);
            }
            // Nothing completed within the wait: `TimedOut` while a transfer
            // is pending or a watch is armed, `Exhausted` once nothing is.
            let waited = waiting || pending;
            let value = Self::transfer_enum(result_type, if waited { 1 } else { 2 }, Vec::new());
            return self.finish_checked_io_result(
                CheckedIoResult::Success(Some(value)),
                call.result,
                call.error,
                call.success,
                call.failure,
            );
        };
        let owner = self.transfer_groups.get_mut(&group).unwrap();
        owner.span = call.span;
        owner.observation = Some(observation);
        if let Some(outcome) = owner.entries[position].ready.take() {
            return self.finish_transfer_observation(group, position, outcome);
        }
        self.push_transfer_phase(Callback {
            group,
            position,
            phase: Phase::Observe,
        })?;
        Ok(StepOutcome::Continue)
    }

    fn transfer_enum(
        ty: mir::PayloadEnumType,
        index: usize,
        fields: Vec<LocalValue>,
    ) -> LocalValue {
        LocalValue::PayloadEnum(PayloadEnumValue {
            ty,
            case: crate::enums::EnumCaseId {
                enum_id: ty.id,
                index,
            },
            moved_fields: vec![false; fields.len()],
            fields,
        })
    }

    fn finish_transfer_observation(
        &mut self,
        group: usize,
        position: usize,
        outcome: FunctionOutcome,
    ) -> Result<StepOutcome, InterpreterError> {
        self.retire_completed_transfer(group, position, &outcome)?;
        let owner = self.transfer_groups.get_mut(&group).unwrap();
        let Fetch {
            call, result_type, ..
        } = owner
            .observation
            .take()
            .ok_or_else(|| InterpreterError::new("completion observation has no caller"))?;
        let entry = &mut owner.entries[position];
        entry.delivered = true;
        let reading = entry.reading;
        let position = entry.position;
        owner.observed = Some((position, if reading { READ_ENTRY } else { WRITE_ENTRY }));
        let direction = usize::from(!reading);
        let identity = u128::from(owner.identity) | ((position as u128) << 64);
        let outcome = match outcome {
            FunctionOutcome::CheckedError(error) => CheckedIoResult::Error(error),
            FunctionOutcome::Value(LocalValue::NullablePayloadEnum { ty, value }) => {
                let types = stream::TransferObservationTypes::for_result(self.program, result_type)
                    .ok_or_else(|| InterpreterError::new("completion has no canonical schema"))?;
                if ty != types.outcomes[direction] {
                    return Err(InterpreterError::new(
                        "completion callback returned another outcome type",
                    ));
                }
                let key = LocalValue::Scalar(mir::ScalarValue::Inline(
                    crate::inline_value::InlineValue {
                        ty: types.keys[direction],
                        bits: identity,
                    },
                ));
                let completion = Self::transfer_enum(
                    types.completion,
                    direction,
                    vec![key, LocalValue::NullablePayloadEnum { ty, value }],
                );
                CheckedIoResult::Success(Some(Self::transfer_enum(
                    result_type,
                    0,
                    vec![completion],
                )))
            }
            _ => {
                return Err(InterpreterError::new(
                    "completion callback returned another value shape",
                ));
            }
        };
        self.finish_checked_io_result(outcome, call.result, call.error, call.success, call.failure)
    }

    fn retire_completed_transfer(
        &mut self,
        group: usize,
        position: usize,
        outcome: &FunctionOutcome,
    ) -> Result<(), InterpreterError> {
        let entry = &self.transfer_groups[&group].entries[position];
        let request = entry.request;
        let bound = entry.region.length;
        let progress = match outcome {
            FunctionOutcome::CheckedError(error) => self.io_error_progress(*error, bound)?,
            FunctionOutcome::Value(LocalValue::NullablePayloadEnum { value, .. }) => {
                match value.as_ref() {
                    Some(outcome) if outcome.case.index == 0 => {
                        let [LocalValue::Scalar(mir::ScalarValue::Integer(count))] =
                            outcome.fields.as_slice()
                        else {
                            return Err(InterpreterError::new(
                                "completed transfer has no byte count",
                            ));
                        };
                        Some(
                            usize::try_from(count.signed_value())
                                .ok()
                                .filter(|count| *count <= bound)
                                .ok_or_else(|| {
                                    InterpreterError::new(
                                        "completed transfer exceeds its retained buffer",
                                    )
                                })?,
                        )
                    }
                    Some(outcome) if entry.reading && outcome.case.index == 2 => Some(0),
                    _ => None,
                }
            }
            _ => {
                return Err(InterpreterError::new(
                    "completion callback returned another value shape",
                ));
            }
        };
        let entry = &mut self.transfer_groups.get_mut(&group).unwrap().entries[position];
        entry.prepared = false;
        entry.completed_progress = progress;
        self.release_transfer_request_pin(request)
    }

    pub(super) fn resubmit_transfer(
        &mut self,
        group: mir::LocalId,
        key: mir::LocalId,
        reading: bool,
        range: [mir::LocalId; 2],
        call: CheckedCall,
    ) -> Result<StepOutcome, InterpreterError> {
        let LocalValue::Class { object: group, .. } =
            read_local(&self.current_frame()?.locals, group)?
        else {
            return Err(InterpreterError::new("resubmission has no group"));
        };
        let group = *group;
        let LocalValue::Scalar(mir::ScalarValue::Inline(key)) =
            read_local(&self.current_frame()?.locals, key)?
        else {
            return Err(InterpreterError::new("resubmission has no directional key"));
        };
        let mut bounds = [0usize; 2];
        let mut valid = true;
        for (bound, local) in bounds.iter_mut().zip(range) {
            let LocalValue::Scalar(mir::ScalarValue::Integer(value)) =
                read_local(&self.current_frame()?.locals, local)?
            else {
                return Err(InterpreterError::new("resubmission has no int range"));
            };
            match usize::try_from(value.signed_value()) {
                Ok(value) => *bound = value,
                Err(_) => valid = false,
            }
        }
        let [offset, length] = bounds;
        let owner = self
            .transfer_groups
            .get(&group)
            .ok_or_else(|| InterpreterError::new("resubmission group was released"))?;
        let position = usize::try_from(key.bits >> 64).ok().and_then(|position| {
            owner
                .entries
                .iter()
                .position(|entry| entry.position == position)
        });
        let entry = position.map(|position| &owner.entries[position]);
        let region = entry
            .filter(|entry| entry.reading == reading)
            .and_then(|entry| entry.resubmission_region(offset, length));
        valid &= key.bits as u64 == owner.identity
            && !owner.stopped
            && owner.setup.is_none()
            && owner.resubmission.is_none()
            && region.is_some();
        if !valid {
            let error = self.allocate_io_error(
                IoOperation::Configure,
                IoTarget::TransferGroup,
                IoErrorReason::InvalidInput,
                None,
                None,
            )?;
            return self.finish_checked_io_result(
                CheckedIoResult::Error(error),
                call.result,
                call.error,
                call.success,
                call.failure,
            );
        }
        let position = position.expect("validated operation key");
        let owner = self.transfer_groups.get_mut(&group).unwrap();
        let entry = &mut owner.entries[position];
        entry.region = region.expect("validated retained range");
        entry.delivered = false;
        entry.completed_progress = None;
        let request = entry.request;
        owner.resubmission = Some(call);
        owner.span = call.span;
        let view = self.pending_requests.get_mut(&request).unwrap();
        view.selection = None;
        view.invalid = false;
        view.observing = false;
        view.outcome = None;
        view.failure = None;
        view.bound = length;
        if length != 0 && self.group_control_stopped(group)? {
            return self.stop_virtual_group(group, 0);
        }
        self.push_transfer_source_getter(
            group,
            position,
            CheckedContinuation::TransferGroup(Callback {
                group,
                position,
                phase: Phase::Source,
            }),
            call.span,
        )?;
        Ok(StepOutcome::Continue)
    }

    fn finish_resubmission(
        &mut self,
        group: usize,
        position: usize,
        error: Option<InterfaceCarrier>,
        progress: Option<usize>,
    ) -> Result<StepOutcome, InterpreterError> {
        let owner = self.transfer_groups.get_mut(&group).unwrap();
        owner.entries[position].ready = error.map(FunctionOutcome::CheckedError);
        owner.entries[position].completed_progress = progress;
        if error.is_some() {
            let request = owner.entries[position].request;
            let view = self.pending_requests.get_mut(&request).unwrap();
            view.selection = None;
            view.outcome = None;
            view.failure = None;
            view.observing = true;
        }
        self.return_transfer_resubmission(group)
    }

    fn return_transfer_resubmission(
        &mut self,
        group: usize,
    ) -> Result<StepOutcome, InterpreterError> {
        let call = self
            .transfer_groups
            .get_mut(&group)
            .unwrap()
            .resubmission
            .take()
            .ok_or_else(|| InterpreterError::new("resubmission has no caller"))?;
        // Preparation failures are transfer results, not failed configuration.
        // The group owns the original Error until observation or destruction.
        self.finish_checked_io_result(
            CheckedIoResult::Success(None),
            call.result,
            call.error,
            call.success,
            call.failure,
        )
    }

    pub(super) fn transfer_group_read_region(
        &mut self,
        group: mir::LocalId,
        key: mir::LocalId,
    ) -> Result<CheckedIoResult, InterpreterError> {
        let LocalValue::Class { object, .. } = read_local(&self.current_frame()?.locals, group)?
        else {
            return Err(InterpreterError::new("completed read access has no group"));
        };
        let LocalValue::Scalar(mir::ScalarValue::Inline(key)) =
            read_local(&self.current_frame()?.locals, key)?
        else {
            return Err(InterpreterError::new(
                "completed read access has no read key",
            ));
        };
        let owner = self
            .transfer_groups
            .get(object)
            .ok_or_else(|| InterpreterError::new("completed read group was released"))?;
        let entry = usize::try_from(key.bits >> 64).ok().and_then(|position| {
            owner
                .entries
                .iter()
                .find(|entry| entry.position == position)
        });
        if key.bits as u64 != owner.identity || !entry.is_some_and(|entry| entry.reading) {
            return Ok(CheckedIoResult::Error(self.allocate_io_error(
                IoOperation::Access,
                IoTarget::TransferGroup,
                IoErrorReason::InvalidInput,
                None,
                None,
            )?));
        }
        let entry = entry.expect("validated read key");
        // Eager virtual I/O is not retirement: preparation still protects the
        // buffer until ordinary completion dispatch has finished.
        let progress = if entry.prepared {
            None
        } else {
            entry.completed_progress
        };
        let region = match progress {
            Some(length) if length <= entry.region.length => ByteRegionValue {
                kind: mir::ByteRegionKind::Readonly,
                nullable: true,
                source: entry.region.source,
                length,
            },
            Some(_) => {
                return Err(InterpreterError::new(
                    "completed read exceeds its retained buffer",
                ));
            }
            None => ByteRegionValue::absent(mir::ByteRegionKind::Readonly),
        };
        Ok(CheckedIoResult::Success(Some(LocalValue::ByteRegion(
            region,
        ))))
    }

    pub(super) fn observed_transfer_key(
        &self,
        group: mir::LocalId,
        key: crate::inline_value::InlineValueType,
    ) -> Result<Option<mir::ScalarValue>, InterpreterError> {
        let LocalValue::Class { object, .. } = read_local(&self.current_frame()?.locals, group)?
        else {
            return Err(InterpreterError::new("observed key has no group"));
        };
        let owner = self
            .transfer_groups
            .get(object)
            .ok_or_else(|| InterpreterError::new("observed key group was released"))?;
        let kind = key.transfer_entry_kind();
        Ok(owner
            .observed
            .filter(|(_, observed)| Some(*observed) == kind)
            .map(|(position, _)| {
                mir::ScalarValue::Inline(crate::inline_value::InlineValue {
                    ty: key,
                    bits: u128::from(owner.identity) | ((position as u128) << 64),
                })
            }))
    }

    pub(super) fn begin_transfer_group(
        &mut self,
        class: crate::class_layout::ClassId,
        set: mir::LocalId,
        plan: mir::LocalId,
        controls: [mir::LocalId; 2],
        call: CheckedCall,
    ) -> Result<StepOutcome, InterpreterError> {
        let set = self.readiness_set_id(set)?;
        let LocalValue::Class { object: plan, .. } = self.read_or_take_local(plan, true)? else {
            return Err(InterpreterError::new("group setup has no taken plan"));
        };
        self.heap
            .remove(&plan)
            .ok_or_else(|| InterpreterError::new("group setup plan owner was released"))?;
        let plan = self
            .transfer_plans
            .remove(&plan)
            .ok_or_else(|| InterpreterError::new("group setup plan configuration was released"))?;
        let owner = &self.readiness_sets[&set];
        // A plan of watches alone names no set, so any set can begin it.
        let valid = !owner.active
            && plan.owner.is_none_or(|bound| bound == set as u64)
            && !plan.entries.is_empty()
            && plan.entries.iter().all(|entry| {
                let transfer_plan::PlanEntry::Transfer {
                    registration,
                    reading,
                    ..
                } = entry
                else {
                    return true;
                };
                owner
                    .members
                    .get(registration)
                    .and_then(|(_, value)| value.as_ref())
                    .is_some_and(|value| {
                        let ty = local_value_type(self.program, value);
                        let operation = if *reading {
                            StreamOperation::PrepareRead
                        } else {
                            StreamOperation::PrepareWrite
                        };
                        self.program.stream_callback(ty, operation).is_some()
                    })
            });
        if !valid {
            let error = self.allocate_io_error(
                IoOperation::Configure,
                IoTarget::TransferGroup,
                IoErrorReason::InvalidInput,
                None,
                None,
            )?;
            return self.finish_checked_io_result(
                CheckedIoResult::Error(error),
                call.result,
                call.error,
                call.success,
                call.failure,
            );
        }
        let deadline = match read_local(&self.current_frame()?.locals, controls[0])? {
            LocalValue::NullableScalar {
                value: Some(mir::ScalarValue::Inline(value)),
                ..
            } => Some(value.bits as i128),
            LocalValue::NullableScalar { value: None, .. } => None,
            _ => {
                return Err(InterpreterError::new(
                    "group deadline has no nullable inline value",
                ))
            }
        };
        let cancellation = match read_local(&self.current_frame()?.locals, controls[1])? {
            LocalValue::NullableScalar {
                value: Some(mir::ScalarValue::Inline(value)),
                ..
            } => Some(value.bits),
            LocalValue::NullableScalar { value: None, .. } => None,
            _ => {
                return Err(InterpreterError::new(
                    "group cancellation has no nullable inline value",
                ))
            }
        };
        if let Some(identity) = cancellation {
            if !inline_value::retain_source_claim(&mut self.cancellation_states, identity)? {
                let error = self.allocate_io_error(
                    IoOperation::Configure,
                    IoTarget::TransferGroup,
                    IoErrorReason::ResourceExhausted,
                    None,
                    None,
                )?;
                return self.finish_checked_io_result(
                    CheckedIoResult::Error(error),
                    call.result,
                    call.error,
                    call.success,
                    call.failure,
                );
            }
        }
        let group = self.next_object;
        self.next_object += 1;
        let mut entries = Vec::new();
        // Every watch of the plan is armed as the group begins.
        let mut watches = Vec::new();
        for (position, entry) in plan.entries.into_iter().enumerate() {
            let (registration, region, reading) = match entry {
                transfer_plan::PlanEntry::Watch(source) => {
                    watches.push(Watch {
                        position,
                        source,
                        state: WatchState::Armed,
                    });
                    continue;
                }
                transfer_plan::PlanEntry::Transfer {
                    registration,
                    region,
                    reading,
                } => (registration, region, reading),
            };
            let request = self.next_object;
            self.next_object += 1;
            self.pending_requests.insert(
                request,
                file_stream::RequestView {
                    selection: None,
                    invalid: false,
                    observing: false,
                    outcome: None,
                    failure: None,
                    target: IoTarget::TransferGroup,
                    reading,
                    source: None,
                    bound: region.length,
                    owner: Some((group, entries.len())),
                    pin: None,
                },
            );
            entries.push(Entry {
                position,
                registration,
                buffer: region,
                region,
                request,
                reading,
                prepared: false,
                delivered: false,
                ready: None,
                completed_progress: None,
            });
        }
        self.readiness_sets.get_mut(&set).unwrap().active = true;
        self.transfer_groups.insert(
            group,
            GroupOwner {
                standard_claims: [false; 3],
                set,
                class,
                identity: plan.identity,
                observed: None,
                entries,
                watches,
                setup: Some(call),
                setup_error: None,
                setup_reason: None,
                resubmission: None,
                observation: None,
                span: call.span,
                deadline,
                cancellation,
                stopped: false,
                wait_failure: None,
                wait_failure_reported: false,
            },
        );
        if self.transfer_groups[&group]
            .entries
            .iter()
            .any(|entry| entry.region.length != 0)
            && self.group_control_stopped(group)?
        {
            return self.stop_virtual_group(group, 0);
        }
        // Watches need no stream callback, so a plan of watches alone is set up.
        if self.transfer_groups[&group].entries.is_empty() {
            return self.finish_virtual_group_setup(group);
        }
        self.push_transfer_source_getter(
            group,
            0,
            CheckedContinuation::TransferGroup(Callback {
                group,
                position: 0,
                phase: Phase::SetupSource,
            }),
            call.span,
        )?;
        Ok(StepOutcome::Continue)
    }

    /// Arms a watch again. Only a watch of this group whose `Ready` was
    /// fetched can be armed, and never once the group has stopped: the same
    /// conditions a transfer is resubmitted under.
    pub(super) fn resubmit_watch(
        &mut self,
        group: mir::LocalId,
        key: mir::LocalId,
    ) -> Result<CheckedIoResult, InterpreterError> {
        let LocalValue::Class { object: group, .. } =
            read_local(&self.current_frame()?.locals, group)?
        else {
            return Err(InterpreterError::new("watch resubmission has no group"));
        };
        let group = *group;
        let LocalValue::Scalar(mir::ScalarValue::Inline(key)) =
            read_local(&self.current_frame()?.locals, key)?
        else {
            return Err(InterpreterError::new("watch resubmission has no watch key"));
        };
        let key = *key;
        let owner = self
            .transfer_groups
            .get(&group)
            .ok_or_else(|| InterpreterError::new("watch resubmission group was released"))?;
        let idle = usize::try_from(key.bits >> 64)
            .ok()
            .and_then(|position| {
                owner
                    .watches
                    .iter()
                    .position(|watch| watch.position == position)
            })
            .filter(|index| {
                key.bits as u64 == owner.identity
                    && !owner.stopped
                    && owner.setup.is_none()
                    && owner.resubmission.is_none()
                    && owner.watches[*index].state == WatchState::Idle
            });
        let Some(index) = idle else {
            return Ok(CheckedIoResult::Error(self.allocate_io_error(
                IoOperation::Configure,
                IoTarget::TransferGroup,
                IoErrorReason::InvalidInput,
                None,
                None,
            )?));
        };
        self.transfer_groups.get_mut(&group).unwrap().watches[index].state = WatchState::Armed;
        Ok(CheckedIoResult::Success(None))
    }

    fn transfer_receiver(
        &self,
        group: usize,
        position: usize,
    ) -> Result<(mir::Type, &LocalValue), InterpreterError> {
        let group = self
            .transfer_groups
            .get(&group)
            .ok_or_else(|| InterpreterError::new("transfer callback group was released"))?;
        let value = self
            .readiness_sets
            .get(&group.set)
            .and_then(|set| set.members.get(&group.entries[position].registration))
            .and_then(|(_, value)| value.as_ref())
            .ok_or_else(|| {
                InterpreterError::new("transfer callback complete owner was released")
            })?;
        Ok((local_value_type(self.program, value), value))
    }

    pub(super) fn push_transfer_source_getter(
        &mut self,
        group: usize,
        position: usize,
        continuation: CheckedContinuation,
        span: Span,
    ) -> Result<(), InterpreterError> {
        let (ty, value) = self.transfer_receiver(group, position)?;
        let (carrier, entry) =
            self.stream_callback_entry(ty, value, StreamOperation::ReadinessSource)?;
        self.push_checked_frame(
            entry,
            &[LocalValue::Error(carrier)],
            &[None],
            continuation,
            span,
        )
    }

    fn push_transfer_phase(&mut self, callback: Callback) -> Result<(), InterpreterError> {
        let group = &self.transfer_groups[&callback.group];
        let operation = &group.entries[callback.position];
        let request = operation.request;
        let region = operation.region;
        let reading = operation.reading;
        let span = group.span;
        let name = if reading {
            stream::READ_REQUEST
        } else {
            stream::WRITE_REQUEST
        };
        let class = self
            .program
            .classes
            .iter()
            .find(|class| {
                class.declaration_name == name && class.source_span.source == stream::SOURCE_ID
            })
            .ok_or_else(|| {
                InterpreterError::new("transfer callback has no canonical request declaration")
            })?
            .id;
        let operation = match (callback.phase, reading) {
            (Phase::Prepare, true) => StreamOperation::PrepareRead,
            (Phase::Prepare, false) => StreamOperation::PrepareWrite,
            (
                Phase::Complete { .. }
                | Phase::ControlDrain { .. }
                | Phase::CompleteResubmission
                | Phase::CompleteSubmitted
                | Phase::Observe,
                true,
            ) => StreamOperation::CompleteRead,
            (
                Phase::Complete { .. }
                | Phase::ControlDrain { .. }
                | Phase::CompleteResubmission
                | Phase::CompleteSubmitted
                | Phase::Observe,
                false,
            ) => StreamOperation::CompleteWrite,
            (Phase::SetupSource | Phase::Source, _) => {
                return Err(InterpreterError::new(
                    "source getter has no request argument",
                ));
            }
        };
        let (ty, value) = self.transfer_receiver(callback.group, callback.position)?;
        let (carrier, entry) = self.stream_callback_entry(ty, value, operation)?;
        if matches!(
            operation,
            StreamOperation::CompleteRead | StreamOperation::CompleteWrite
        ) {
            self.pending_requests.get_mut(&request).unwrap().observing = true;
        }
        self.push_checked_frame(
            entry,
            &[
                LocalValue::Error(carrier),
                LocalValue::Class {
                    object: request,
                    class,
                },
                LocalValue::ByteRegion(region),
            ],
            &[None, None, None],
            CheckedContinuation::TransferGroup(callback),
            span,
        )
    }

    pub(super) fn finish_transfer_callback(
        &mut self,
        callback: Callback,
        mut outcome: FunctionOutcome,
    ) -> Result<StepOutcome, InterpreterError> {
        let group = callback.group;
        let position = callback.position;
        let request = self.transfer_groups[&group].entries[position].request;
        if matches!(
            callback.phase,
            Phase::Observe
                | Phase::Complete { .. }
                | Phase::ControlDrain { .. }
                | Phase::CompleteResubmission
                | Phase::CompleteSubmitted
        ) && matches!(
            &outcome,
            FunctionOutcome::Value(LocalValue::NullablePayloadEnum { value: None, .. })
        ) {
            let view = &self.pending_requests[&request];
            if view.outcome.is_some()
                || view.failure.is_some()
                || matches!(
                    view.selection,
                    Some(file_stream::RequestSelection::Immediate(_))
                )
            {
                // Null denotes absence, never permission to erase an actual
                // native or immediate result (Decision 0110).
                outcome = FunctionOutcome::CheckedError(self.allocate_io_error(
                    IoOperation::Configure,
                    IoTarget::TransferGroup,
                    IoErrorReason::InvalidInput,
                    None,
                    None,
                )?);
            }
        }
        match callback.phase {
            Phase::ControlDrain { next } => {
                self.retire_completed_transfer(group, position, &outcome)?;
                self.transfer_groups.get_mut(&group).unwrap().entries[position].ready =
                    Some(outcome);
                return self.stop_virtual_group(group, next);
            }
            Phase::Observe => return self.finish_transfer_observation(group, position, outcome),
            Phase::CompleteSubmitted => {
                self.retire_completed_transfer(group, position, &outcome)?;
                self.transfer_groups.get_mut(&group).unwrap().entries[position].ready =
                    Some(outcome);
                return if self.transfer_groups[&group].resubmission.is_some() {
                    self.return_transfer_resubmission(group)
                } else {
                    self.submit_virtual_group(group, position + 1)
                };
            }
            Phase::SetupSource | Phase::Source => {
                match outcome {
                    FunctionOutcome::CheckedError(error) => {
                        if self.transfer_groups[&group].resubmission.is_some() {
                            self.release_transfer_request_pin(request)?;
                            return self.finish_resubmission(group, position, Some(error), None);
                        }
                        self.transfer_groups.get_mut(&group).unwrap().setup_error = Some(error);
                        return self.retire_transfer_group(group, 0);
                    }
                    FunctionOutcome::Value(value) => {
                        let source = self.readiness_source_value(&value)?;
                        if let Some(reason) = self.replace_request_source(request, source)? {
                            if self.transfer_groups[&group].resubmission.is_some() {
                                let error = self.allocate_io_error(
                                    if self.transfer_groups[&group].entries[position].reading {
                                        IoOperation::Read
                                    } else {
                                        IoOperation::Write
                                    },
                                    self.readiness_source_target(source)?,
                                    reason,
                                    None,
                                    None,
                                )?;
                                self.release_transfer_request_pin(request)?;
                                return self.finish_resubmission(
                                    group,
                                    position,
                                    Some(error),
                                    None,
                                );
                            }
                            self.transfer_groups.get_mut(&group).unwrap().setup_reason =
                                Some(reason);
                            return self.retire_transfer_group(group, 0);
                        }
                    }
                    _ => {
                        return Err(InterpreterError::new(
                            "transfer source getter returned another value shape",
                        ));
                    }
                }
                if self.transfer_groups[&group].entries[position].region.length != 0
                    && self.group_control_stopped(group)?
                {
                    return self.stop_virtual_group(group, 0);
                }
                if callback.phase == Phase::SetupSource {
                    let (position, phase) =
                        if position + 1 == self.transfer_groups[&group].entries.len() {
                            (0, Phase::Source)
                        } else {
                            (position + 1, Phase::SetupSource)
                        };
                    self.push_transfer_source_getter(
                        group,
                        position,
                        CheckedContinuation::TransferGroup(Callback {
                            group,
                            position,
                            phase,
                        }),
                        self.transfer_groups[&group].span,
                    )?;
                    return Ok(StepOutcome::Continue);
                }
                self.push_transfer_phase(Callback {
                    phase: Phase::Prepare,
                    ..callback
                })?;
            }
            Phase::Prepare => {
                match outcome {
                    FunctionOutcome::CheckedError(error) => {
                        if self.transfer_groups[&group].resubmission.is_some() {
                            let progress = self.io_error_progress(
                                error,
                                self.transfer_groups[&group].entries[position].region.length,
                            )?;
                            self.release_transfer_request_pin(request)?;
                            return self.finish_resubmission(
                                group,
                                position,
                                Some(error),
                                progress,
                            );
                        }
                        self.transfer_groups.get_mut(&group).unwrap().setup_error = Some(error);
                        return self.retire_transfer_group(group, 0);
                    }
                    FunctionOutcome::Void => {
                        self.transfer_groups.get_mut(&group).unwrap().entries[position].prepared =
                            true;
                    }
                    _ => {
                        return Err(InterpreterError::new(
                            "transfer preparation returned a value",
                        ));
                    }
                }
                if self.pending_requests[&request].invalid
                    || self.pending_requests[&request].selection.is_none()
                    || self.transfer_selected_regions_conflict(group, position)?
                {
                    let resubmitting = self.transfer_groups[&group].resubmission.is_some();
                    if !resubmitting {
                        self.transfer_groups.get_mut(&group).unwrap().setup_reason =
                            Some(IoErrorReason::InvalidInput);
                    }
                    self.push_transfer_phase(Callback {
                        phase: if resubmitting {
                            Phase::CompleteResubmission
                        } else {
                            Phase::Complete { next: 0 }
                        },
                        ..callback
                    })?;
                    return Ok(StepOutcome::Continue);
                }
                self.prepare_standard_region(request)?;
                if self.transfer_groups[&group].entries[position].region.length != 0
                    && self.group_control_stopped(group)?
                {
                    return self.stop_virtual_group(group, 0);
                }
                if self.transfer_groups[&group].resubmission.is_some() {
                    if self.submit_virtual_transfer(group, position)? {
                        return Ok(StepOutcome::CleanExit);
                    }
                    if matches!(
                        self.pending_requests[&request].selection,
                        Some(
                            file_stream::RequestSelection::Immediate(_)
                                | file_stream::RequestSelection::Native { prepared: true, .. }
                        )
                    ) {
                        self.push_transfer_phase(Callback {
                            phase: Phase::CompleteSubmitted,
                            ..callback
                        })?;
                        return Ok(StepOutcome::Continue);
                    }
                    return self.finish_resubmission(group, position, None, None);
                }
                if position + 1 == self.transfer_groups[&group].entries.len() {
                    return self.submit_virtual_group(group, 0);
                }
                let position = position + 1;
                self.push_transfer_source_getter(
                    group,
                    position,
                    CheckedContinuation::TransferGroup(Callback {
                        group,
                        position,
                        phase: Phase::Source,
                    }),
                    self.transfer_groups[&group].span,
                )?;
            }
            Phase::Complete { next } => {
                if let FunctionOutcome::CheckedError(error) = outcome {
                    let owner = self.transfer_groups.get_mut(&group).unwrap();
                    if owner.setup.is_some() && owner.setup_error.is_none() {
                        owner.setup_error = Some(error);
                    } else {
                        self.push_owned_drop_task(OwnedDrop::Error(error))?;
                    }
                }
                self.transfer_groups.get_mut(&group).unwrap().entries[position].prepared = false;
                self.release_transfer_request_pin(request)?;
                return self.retire_transfer_group(group, next);
            }
            Phase::CompleteResubmission => {
                let error = match outcome {
                    FunctionOutcome::CheckedError(error) => error,
                    _ => self.allocate_io_error(
                        if self.transfer_groups[&group].entries[position].reading {
                            IoOperation::Read
                        } else {
                            IoOperation::Write
                        },
                        self.pending_requests[&request].target.clone(),
                        IoErrorReason::InvalidInput,
                        None,
                        None,
                    )?,
                };
                self.transfer_groups.get_mut(&group).unwrap().entries[position].prepared = false;
                self.release_transfer_request_pin(request)?;
                let progress = self.io_error_progress(
                    error,
                    self.transfer_groups[&group].entries[position].region.length,
                )?;
                return self.finish_resubmission(group, position, Some(error), progress);
            }
        }
        Ok(StepOutcome::Continue)
    }

    fn transfer_selected_regions_conflict(
        &self,
        group: usize,
        position: usize,
    ) -> Result<bool, InterpreterError> {
        let entries = &self.transfer_groups[&group].entries;
        let Some(file_stream::RequestSelection::Native { region, .. }) = self.pending_requests
            [&entries[position].request]
            .selection
            .as_ref()
        else {
            return Ok(false);
        };
        for (other_position, prior) in entries.iter().enumerate() {
            if other_position == position || !prior.prepared {
                continue;
            }
            if let Some(file_stream::RequestSelection::Native { region: other, .. }) =
                self.pending_requests[&prior.request].selection.as_ref()
            {
                if self.transfer_regions_conflict(
                    *region,
                    entries[position].reading,
                    *other,
                    prior.reading,
                )? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn submit_virtual_group(
        &mut self,
        group: usize,
        first: usize,
    ) -> Result<StepOutcome, InterpreterError> {
        let count = self.transfer_groups[&group].entries.len();
        for position in first..count {
            if self.transfer_groups[&group].entries[position].region.length != 0
                && self.group_control_stopped(group)?
            {
                return self.stop_virtual_group(group, 0);
            }
            if self.submit_virtual_transfer(group, position)? {
                return Ok(StepOutcome::CleanExit);
            }
            let request = self.transfer_groups[&group].entries[position].request;
            if matches!(
                self.pending_requests[&request].selection,
                Some(
                    file_stream::RequestSelection::Immediate(_)
                        | file_stream::RequestSelection::Native { prepared: true, .. }
                )
            ) {
                self.push_transfer_phase(Callback {
                    group,
                    position,
                    phase: Phase::CompleteSubmitted,
                })?;
                return Ok(StepOutcome::Continue);
            }
        }
        self.finish_virtual_group_setup(group)
    }

    fn finish_virtual_group_setup(
        &mut self,
        group: usize,
    ) -> Result<StepOutcome, InterpreterError> {
        let owner = self.transfer_groups.get_mut(&group).unwrap();
        let call = owner.setup.take().unwrap();
        let class = owner.class;
        self.heap.insert(group, ObjectValue::new(class, Vec::new()));
        self.finish_checked_io_result(
            CheckedIoResult::Success(Some(LocalValue::Class {
                object: group,
                class,
            })),
            call.result,
            call.error,
            call.success,
            call.failure,
        )
    }

    fn submit_virtual_transfer(
        &mut self,
        group: usize,
        position: usize,
    ) -> Result<bool, InterpreterError> {
        let entry = &self.transfer_groups[&group].entries[position];
        let request = entry.request;
        let reading = entry.reading;
        let selected = match self.pending_requests[&request].selection.as_ref() {
            Some(file_stream::RequestSelection::Native { prepared: true, .. }) => return Ok(false),
            Some(file_stream::RequestSelection::Native { source, region, .. }) => {
                Some((*source, *region))
            }
            Some(file_stream::RequestSelection::Immediate(_)) => None,
            None => {
                return Err(InterpreterError::new(
                    "virtual submission has no prepared selection",
                ));
            }
        };
        if let Some((source, region)) = selected {
            // A read whose source has nothing yet stays active with no
            // outcome; a later fetch performs it.
            if reading
                && self
                    .readiness_source_read_wait(source, region.length)?
                    .is_some()
            {
                return Ok(false);
            }
            let name = if reading {
                "Doria\\Std\\Io\\ReadOutcome"
            } else {
                "Doria\\Std\\Io\\WriteOutcome"
            };
            let ty = self
                .program
                .enums
                .iter()
                .find(|definition| definition.name == name)
                .and_then(|definition| definition.payload_type())
                .ok_or_else(|| InterpreterError::new("virtual submission has no outcome type"))?;
            let result = match source {
                readiness_set::Source::File(object) if reading => {
                    self.read_file_region(object, region, ty)?
                }
                readiness_set::Source::File(object) => {
                    self.write_file_region(object, region, ty)?
                }
                readiness_set::Source::Standard(stream) => {
                    match self.transfer_standard_region(stream, region, ty)? {
                        standard_stream::Transfer::Outcome(result) => result,
                        standard_stream::Transfer::CleanExit => return Ok(true),
                    }
                }
            };
            let view = self.pending_requests.get_mut(&request).unwrap();
            match result {
                Ok(LocalValue::PayloadEnum(value)) => view.outcome = Some(value),
                Err(error) => {
                    view.target = error.target;
                    view.failure = Some((error.reason, error.system_code, error.count));
                }
                _ => {
                    return Err(InterpreterError::new(
                        "virtual transfer produced another shape",
                    ));
                }
            }
        }
        Ok(false)
    }

    pub(super) fn drop_transfer_group(&mut self, group: usize) -> Result<(), InterpreterError> {
        self.retire_transfer_group(group, 0)?;
        Ok(())
    }

    fn retire_transfer_group(
        &mut self,
        group: usize,
        first: usize,
    ) -> Result<StepOutcome, InterpreterError> {
        let owner = &self.transfer_groups[&group];
        if let Some(position) =
            (first..owner.entries.len()).find(|position| owner.entries[*position].prepared)
        {
            self.push_transfer_phase(Callback {
                group,
                position,
                phase: Phase::Complete { next: position + 1 },
            })?;
            return Ok(StepOutcome::Continue);
        }
        let owner = self.transfer_groups.remove(&group).unwrap();
        if let Some(identity) = owner.cancellation {
            inline_value::release_source_claim(&mut self.cancellation_states, identity)?;
        }
        for entry in owner.entries {
            if let Some(FunctionOutcome::CheckedError(error)) = entry.ready {
                self.push_owned_drop_task(OwnedDrop::Error(error))?;
            }
            self.release_transfer_request_pin(entry.request)?;
            self.pending_requests.remove(&entry.request);
        }
        for (index, claimed) in owner.standard_claims.into_iter().enumerate() {
            if claimed {
                if self.standard_claims[index] != Some(StandardClaim::Group(group)) {
                    return Err(InterpreterError::new(
                        "standard group claim has another owner",
                    ));
                }
                self.standard_claims[index] = None;
            }
        }
        self.readiness_sets.get_mut(&owner.set).unwrap().active = false;
        self.heap.remove(&group);
        if let Some(call) = owner.setup {
            let error = match owner.setup_error {
                Some(error) => error,
                None => self.allocate_io_error(
                    IoOperation::Configure,
                    IoTarget::TransferGroup,
                    owner.setup_reason.unwrap_or(IoErrorReason::InvalidInput),
                    None,
                    None,
                )?,
            };
            return self.finish_checked_io_result(
                CheckedIoResult::Error(error),
                call.result,
                call.error,
                call.success,
                call.failure,
            );
        }
        Ok(StepOutcome::Continue)
    }

    fn release_transfer_request_pin(&mut self, request: usize) -> Result<(), InterpreterError> {
        if let Some(source) = self.pending_requests.get_mut(&request).unwrap().pin.take() {
            self.release_request_source(source)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_clock_failure_is_reported_first_and_keeps_unsubmitted_completions() {
        thread_local! { static READS: core::cell::Cell<usize> = const { core::cell::Cell::new(0) }; }
        thread_local! { static LATE: core::cell::Cell<bool> = const { core::cell::Cell::new(false) }; }
        thread_local! { static FAILED: core::cell::Cell<bool> = const { core::cell::Cell::new(false) }; }
        fn clock() -> Result<u64, doria_time::ClockFailure> {
            READS.with(|reads| {
                let value = reads.get();
                reads.set(value + 1);
                if !FAILED.get() && (value == 0 || LATE.get()) {
                    Ok(100)
                } else {
                    Err(doria_time::ClockFailure::System { system_code: 0 })
                }
            })
        }
        let source = r#"
use Doria\Std\Io\{File, ReadinessSet, TransferPlan, TransferWaitResult, TransferCompletion,
    Wait, ReadOutcome, IoError, IoOperation, IoTarget};
use Doria\Std\Time\{Deadline, Duration, ClockError};
use Doria\Std\Concurrency\CancellationSource;
function main(): void throws IoError, ClockError {
    write_file("input", "a");
    let writable $set = new ReadinessSet();
    let $registration = $set->register(File::openRead("input"));
    let $source = new CancellationSource();
    writable Bytes $buffer = Bytes::allocate(1);
    $buffer[0] = 99;
    let $deadline = Deadline::fromNow(Duration::fromNanoseconds(100));
    {
        let $plan = TransferPlan::read($registration, $buffer->getWritableRegion(0, 1));
        let $key = $plan->getReadKey(0);
        let writable $group = $set->begin($plan, deadline: $deadline, cancellation: $source->token);
        try { $group->getNextCompletion(Wait::Indefinite); panic("group clock failure lost"); }
        catch (IoError $error) {
            if ($error->operation != IoOperation::Wait || $error->target != IoTarget::ReadinessSet
                || ($error->systemCode ?? -1) != 0 || $error->transferredByteCount != null
                || $group->observedReadKey != null) { panic("wrong group-clock failure"); }
        }
        if ($group->getNextCompletion(Wait::Indefinite) != TransferWaitResult::Completed(TransferCompletion::Read($key, null))
            || $group->getNextCompletion(Wait::Immediate) != TransferWaitResult::Exhausted) { panic("stopped completion was lost"); }
    }
    if ($buffer[0] != 99) { panic("failed clock submitted bytes"); }
    echo "clock failure first\n";
}
"#;
        for late in [false, true] {
            READS.set(0);
            LATE.set(late);
            FAILED.set(false);
            let source = if late {
                source
                    .replace(
                        "TransferCompletion::Read($key, null)",
                        "TransferCompletion::Read($key, ReadOutcome::Data(1))",
                    )
                    .replace("$buffer[0] != 99", "$buffer[0] != 97")
            } else {
                source.to_owned()
            };
            let program = crate::lower_source_to_mir("group-clock-failure.doria", &source).unwrap();
            let mut interpreter = Interpreter::new(
                &program,
                InterpreterLimits::default(),
                MirIo::default(),
                clock,
            )
            .unwrap();
            interpreter
                .push_frame(program.entry, &[], &[], None, None)
                .unwrap();
            loop {
                match interpreter.step().unwrap() {
                    StepOutcome::Continue => {
                        if late
                            && interpreter
                                .transfer_groups
                                .values()
                                .any(|owner| owner.setup.is_none())
                        {
                            FAILED.set(true);
                        }
                    }
                    StepOutcome::EntryReturned(FunctionOutcome::Void) => break,
                    _ => panic!("clock failure was not caught"),
                }
            }
            assert_eq!(interpreter.stdout, b"clock failure first\n");
            assert!(interpreter.stderr.is_empty());
            assert!(interpreter.heap.is_empty());
            assert!(interpreter.cancellation_states.is_empty());
            assert!(interpreter.transfer_groups.is_empty());
            assert!(interpreter.pending_requests.is_empty());
            if late {
                assert!(READS.get() > 2);
            } else {
                assert_eq!(READS.get(), 2);
            }
        }
    }

    #[test]
    fn relative_wait_clock_failure_leaves_the_original_completion_queued() {
        let source = r#"
use Doria\Std\Io\{File, ReadinessSet, TransferPlan, TransferWaitResult, TransferCompletion,
    ReadOutcome, Wait, IoError, IoOperation, IoTarget};
use Doria\Std\Time\Duration;
function main(): void {
    write_file("input", "a");
    writable ReadinessSet $set = new ReadinessSet();
    let $registration = $set->register(File::openRead("input"));
    writable Bytes $buffer = Bytes::allocate(1);
    {
        let $plan = TransferPlan::read($registration, $buffer->getWritableRegion(0, 1));
        let $key = $plan->getReadKey(0);
        let writable $group = $set->begin($plan);
        try { $group->getNextCompletion(Wait::For(Duration::fromNanoseconds(1))); panic("clock failure lost"); }
        catch (IoError $error) {
            if ($error->operation != IoOperation::Wait || $error->target != IoTarget::ReadinessSet
                || $error->transferredByteCount != null || $group->observedReadKey != null
                || $group->observedWriteKey != null) { panic("wait failure took a transfer"); }
            echo ($error->systemCode ?? -1) . "\n";
        }
        if ($group->getNextCompletion(Wait::Indefinite) != TransferWaitResult::Completed(TransferCompletion::Read($key, ReadOutcome::Data(1)))
            || $group->getNextCompletion(Wait::Immediate) != TransferWaitResult::Exhausted
            || $group->getNextCompletion(Wait::For(Duration::fromNanoseconds(-1))) != TransferWaitResult::Exhausted) { panic("queued completion lost or repeated"); }
    }
}
"#;
        let program = crate::lower_source_to_mir("wait-clock-failure.doria", source).unwrap();
        for (clock, expected) in [
            (
                (|| Err(doria_time::ClockFailure::System { system_code: 0 })) as fn() -> _,
                "0\n",
            ),
            (
                (|| Err(doria_time::ClockFailure::System { system_code: 17 })) as fn() -> _,
                "17\n",
            ),
            (
                (|| Err(doria_time::ClockFailure::Unsupported)) as fn() -> _,
                "-1\n",
            ),
            (
                (|| Err(doria_time::ClockFailure::InvalidReading)) as fn() -> _,
                "-1\n",
            ),
        ] {
            let (execution, _) = interpret_internal_with_clock(
                &program,
                InterpreterLimits::default(),
                MirIo::default(),
                clock,
            )
            .unwrap();
            assert_eq!(execution.output.stdout, expected.as_bytes());
            assert!(execution.output.stderr.is_empty());
            assert_eq!(execution.output.exit_status, 0);
        }
    }

    #[test]
    fn reuse_ranges_use_original_storage_and_preserve_it_on_rejection() {
        for reading in [true, false] {
            let buffer = ByteRegionValue {
                kind: if reading {
                    mir::ByteRegionKind::Writable
                } else {
                    mir::ByteRegionKind::Readonly
                },
                nullable: false,
                source: Some(CollectionElementPlace {
                    collection: 4,
                    generation: 7,
                    index: 11,
                    ty: mir::Type::Scalar(mir::ScalarType::Integer(IntegerType::UInt8)),
                }),
                length: 8,
            };
            let mut entry = Entry {
                position: 0,
                registration: 2,
                buffer,
                region: buffer,
                request: 3,
                reading,
                prepared: false,
                delivered: true,
                ready: None,
                completed_progress: None,
            };
            for (offset, length) in [(0, 8), (2, 3), (8, 0)] {
                let selected = entry.resubmission_region(offset, length).unwrap();
                assert_eq!(selected.source.unwrap().index, 11 + offset);
                assert_eq!(selected.length, length);
                assert_eq!(selected.kind, buffer.kind);
                assert_eq!(entry.buffer, buffer);
            }
            for (offset, length) in [(9, 0), (8, 1), (usize::MAX, 1), (1, usize::MAX)] {
                assert!(entry.resubmission_region(offset, length).is_none());
                assert_eq!(entry.region, buffer);
            }
            entry.region.length = 1;
            assert_eq!(entry.resubmission_region(0, 8).unwrap().length, 8);
            entry.prepared = true;
            assert!(entry.resubmission_region(0, 0).is_none());
            entry.prepared = false;
            entry.delivered = false;
            assert!(entry.resubmission_region(0, 0).is_none());
        }
    }
}
