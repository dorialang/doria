//! One pinned hosted owner drives the existing setup, observation, reuse and
//! teardown state machines. No host callback or destructor runs through C.

use super::*;
use crate::transfer_group::observation::ObservationState;
use crate::transfer_group::resubmission::ResubmissionState;
use crate::transfer_group::Observation;
use core::task::Poll;

#[cfg(windows)]
use crate::byte_io::NativePendingSetup as PendingSetup;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::unix_readiness::pending::PendingSetup;

#[cfg(windows)]
type Timeout = u32;
#[cfg(any(target_os = "linux", target_os = "macos"))]
type Timeout = DrWaitBudgetV1;

enum Call {
    Startup,
    Idle,
    Observation {
        state: ObservationState<Timeout>,
        budget: DrWaitBudgetV1,
    },
    Resubmission(ResubmissionState),
    Cleanup,
    StartupCleanup(SetupFailure<AdapterTerminal>),
    Closed,
    Polling,
}

struct HostGroupOwner {
    setup: ManuallyDrop<PendingSetup<'static, 'static>>,
    call: Call,
    set: *mut AdapterSetOwner,
    identity: crate::transfer_group::plan::GroupIdentity,
}

enum Event {
    Started,
    Observed(Observation<AdapterTerminal>),
    Resubmitted,
    Closed,
    Failed(SetupFailure<AdapterTerminal>),
}

impl HostGroupOwner {
    unsafe fn poll(&mut self, frame: *const DrStackFrameV2) -> Poll<Event> {
        match mem::replace(&mut self.call, Call::Polling) {
            Call::Startup => match self.setup.poll(frame) {
                Poll::Pending => {
                    self.call = Call::Startup;
                    Poll::Pending
                }
                Poll::Ready(Ok(())) => {
                    self.call = Call::Idle;
                    Poll::Ready(Event::Started)
                }
                Poll::Ready(Err(failure)) => {
                    self.call = Call::StartupCleanup(failure);
                    self.poll(frame)
                }
            },
            Call::Observation {
                mut state,
                mut budget,
            } => match state.poll(&mut self.setup.group, &mut budget, frame) {
                Poll::Pending => {
                    self.call = Call::Observation { state, budget };
                    Poll::Pending
                }
                Poll::Ready(observation) => {
                    self.call = Call::Idle;
                    Poll::Ready(Event::Observed(observation))
                }
            },
            Call::Resubmission(mut state) => match state.poll(&mut self.setup.group, frame) {
                Poll::Pending => {
                    self.call = Call::Resubmission(state);
                    Poll::Pending
                }
                Poll::Ready(()) => {
                    self.call = Call::Idle;
                    Poll::Ready(Event::Resubmitted)
                }
            },
            Call::Cleanup => match self.setup.poll_cleanup(frame) {
                Poll::Pending => {
                    self.call = Call::Cleanup;
                    Poll::Pending
                }
                Poll::Ready(()) => {
                    self.call = Call::Closed;
                    Poll::Ready(Event::Closed)
                }
            },
            Call::StartupCleanup(failure) => match self.setup.poll_cleanup(frame) {
                Poll::Pending => {
                    self.call = Call::StartupCleanup(failure);
                    Poll::Pending
                }
                Poll::Ready(()) => {
                    self.call = Call::Closed;
                    Poll::Ready(Event::Failed(failure))
                }
            },
            _ => crate::runtime_invariant_panic(),
        }
    }

    fn host_call(&self) -> Option<(usize, *mut c_void)> {
        let group = &self.setup.group;
        (0..group.len()).find_map(|index| {
            let (operation, _) = group.request(index);
            group
                .driver
                .awaiting_host_call(operation)
                .map(|record| (index, record))
        })
    }
}

/// Consumes the plan, copies its budget and installs the sole whole-group
/// cleanup guard before any host call. Startup is completed through poll.
/// # Safety
/// Set is exclusive external readiness membership; selected complete values,
/// typed sources and buffer loans outlive this owner. Plan is owned and consumed
/// on all paths. Frame/budget/status are current, disjoint call-only storage.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_transfer_plan_configure(
    frame: *const DrStackFrameV2,
    set: *mut c_void,
    plan: *mut c_void,
    budget: *const DrWaitBudgetV1,
    status: *mut DrIoStatusV1,
) -> *mut c_void {
    if budget.is_null() {
        *status = DrIoStatusV1::default();
        drop(plan::take_plan(plan));
        (*status).fail(Failure::invalid_input());
        return ptr::null_mut();
    }
    configure_host_plan(frame, set, plan, status, |native, buffers, selection| {
        native.configure_selected(frame, buffers, &*budget, selection)
    })
}

/// Copies optional group controls and retains cancellation before host startup.
/// # Safety
/// Inputs obey the v1 configure contract. Deadline is null or live call-only
/// DrDeadlineV1 storage; cancellation is null or a live claim for acquisition.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_host_transfer_plan_configure(
    frame: *const DrStackFrameV2,
    set: *mut c_void,
    plan: *mut c_void,
    deadline: *const crate::deadline::DrDeadlineV1,
    cancellation: *mut DrCancellationV1,
    status: *mut DrIoStatusV1,
) -> *mut c_void {
    let budget = deadline
        .as_ref()
        .map_or_else(DrWaitBudgetV1::indefinite, |deadline| {
            DrWaitBudgetV1::from_deadline(*deadline)
        });
    configure_host_plan(frame, set, plan, status, |native, buffers, selection| {
        native.configure_selected_controlled(frame, buffers, &budget, selection, cancellation)
    })
}

unsafe fn configure_host_plan(
    frame: *const DrStackFrameV2,
    set: *mut c_void,
    plan: *mut c_void,
    status: *mut DrIoStatusV1,
    configure: impl FnOnce(
        &'static mut PendingSet,
        BufferList<'static, AdapterTerminal>,
        FixedSlice<usize>,
    ) -> Result<PendingSetup<'static, 'static>, SetupFailure<AdapterTerminal>>,
) -> *mut c_void {
    *status = DrIoStatusV1::default();
    let Some(plan) = plan::take_plan(plan) else {
        (*status).fail(Failure::invalid_input());
        return ptr::null_mut();
    };
    let set = set.cast::<AdapterSetOwner>();
    if set.is_null() || (*set).active || !(*set).readiness || !(*set).pending {
        (*status).fail(Failure::invalid_input());
        return ptr::null_mut();
    }
    let prepared = match plan.prepare_as::<AdapterTerminal>((*set).identity, |key| {
        AdapterSetOwner::native(set).index_of(key)
    }) {
        Ok(prepared) => prepared,
        Err((failure, _)) => {
            (*status).fail(failure);
            return ptr::null_mut();
        }
    };
    let AdapterSetMode::Pending(native) = AdapterSetOwner::native(set) else {
        crate::runtime_invariant_panic();
    };
    let owner = allocate_owner::<HostGroupOwner>(frame);
    (*set).active = true;
    match configure(native, prepared.buffers, prepared.selection) {
        Ok(setup) => {
            owner.write(HostGroupOwner {
                setup: ManuallyDrop::new(setup),
                call: Call::Startup,
                set,
                identity: crate::transfer_group::plan::GroupIdentity::new(prepared.identity),
            });
            owner.cast()
        }
        Err(SetupFailure::Platform(failure)) => {
            (*set).active = false;
            crate::dr_v1_class_free(owner.cast());
            (*status).fail(failure);
            ptr::null_mut()
        }
        Err(_) => crate::runtime_invariant_panic(),
    }
}

/// # Safety
/// Owner is exclusive and idle, with no region/result loan. Budget/status are
/// disjoint current storage. The copied budget cannot change during this fetch.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_transfer_group_observe(
    owner: *mut c_void,
    budget: *const DrWaitBudgetV1,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    let owner = &mut *owner.cast::<HostGroupOwner>();
    if !matches!(owner.call, Call::Idle) || budget.is_null() {
        return (*status).fail(Failure::invalid_input());
    }
    owner.identity.clear();
    owner.call = Call::Observation {
        state: ObservationState::new(),
        budget: *budget,
    };
    0
}

/// # Safety
/// Owner is exclusive and idle with no region/result loan. Status is disjoint
/// current writable storage. Wait words are the canonical scalar transport.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_transfer_group_observe_wait(
    owner: *mut c_void,
    kind: u8,
    low: u64,
    high: i64,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    let owner = &mut *owner.cast::<HostGroupOwner>();
    if !matches!(owner.call, Call::Idle) {
        return (*status).fail(Failure::invalid_input());
    }
    // A fetch clears the previous identity even if freezing its wait fails.
    owner.identity.clear();
    let budget = match DrWaitBudgetV1::from_wait(kind, low, high) {
        Ok(budget) => budget,
        Err(failure) => return (*status).fail(failure),
    };
    owner.call = Call::Observation {
        state: ObservationState::new(),
        budget,
    };
    0
}

/// # Safety
/// Owner is exclusive and idle; key/status are disjoint readable/writable homes.
/// No buffer/result loan overlaps reuse. Direction is zero/one write/read.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_transfer_group_resubmit(
    owner: *mut c_void,
    key: *const c_void,
    reading: u8,
    offset: usize,
    length: usize,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    let owner = &mut *owner.cast::<HostGroupOwner>();
    if !matches!(owner.call, Call::Idle) || reading > 1 || key.is_null() {
        return (*status).fail(Failure::invalid_input());
    }
    let position = owner
        .identity
        .operation_position(*key.cast(), reading != 0, |index| {
            owner.setup.group.request_info(index)
        });
    let state = position
        .and_then(|index| ResubmissionState::new(&mut owner.setup.group, index, offset, length));
    match state {
        Ok(state) => {
            owner.call = Call::Resubmission(state);
            0
        }
        Err(failure) => (*status).fail(failure),
    }
}

/// Cleanup starts cancel-all before drain-all. An outstanding preparation reply
/// is accepted before its once-owed completion, never thrown away by teardown.
/// # Safety
/// Owner is exclusive live storage, not currently inside a poll/host call.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_transfer_group_close(owner: *mut c_void) {
    let owner = &mut *owner.cast::<HostGroupOwner>();
    if matches!(owner.call, Call::Closed | Call::Polling) {
        crate::runtime_invariant_panic();
    }
    owner.call = Call::Cleanup;
}

/// 8 means an ordinary host handoff. Other tags match the existing completion
/// projection: 0 outcome, 1 original hosted Error, 3 platform failure, 4 absent
/// transfer, 5 exhausted, 6 timeout, 7 wait failure. Kind is 0 startup,
/// 1 observation, 2 resubmission or 3 close. No output home is retained.
/// # Safety
/// Owner is exclusive live storage. Outputs are disjoint writable EMPTY homes;
/// terminal has its queried opaque layout. Frame is current for this poll only.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_transfer_group_poll(
    frame: *const DrStackFrameV2,
    owner: *mut c_void,
    kind: *mut u8,
    index: *mut usize,
    outcome: *mut DrAdapterOutcomeV1,
    terminal: *mut c_void,
    control: *mut c_void,
) -> u8 {
    *kind = 0;
    *index = 0;
    *outcome = DrAdapterOutcomeV1::default();
    let control = &mut *control.cast::<DrCompletionStatusV1>();
    *control = DrCompletionStatusV1::default();
    let owner = &mut *owner.cast::<HostGroupOwner>();
    let Poll::Ready(event) = owner.poll(frame) else {
        return 8;
    };
    match event {
        Event::Started => 0,
        Event::Resubmitted => {
            *kind = 2;
            0
        }
        Event::Closed => {
            *kind = 3;
            0
        }
        Event::Failed(SetupFailure::Platform(failure)) => control.transfer.fail(failure),
        Event::Failed(SetupFailure::Preparation(error)) => {
            project_terminal(error, outcome, terminal, &mut control.transfer)
        }
        Event::Observed(observation) => {
            *kind = 1;
            let completion = match observation {
                Observation::Completion(completion) => completion,
                Observation::NoCompletion => return 5,
                Observation::TimedOut => return 6,
                Observation::WaitFailed(failure) => return control.project_wait_failure(failure),
            };
            *index = completion.index;
            let (reading, _) = owner.setup.group.request_info(completion.index).unwrap();
            owner.identity.observe(completion.index, reading);
            let crate::transfer_group::Awaited {
                transfer,
                stop,
                cancellation_failure,
            } = completion.outcome.after_wait_failure_observation();
            control.project(
                crate::transfer_group::Awaited {
                    transfer: None,
                    stop,
                    cancellation_failure,
                },
                reading,
            );
            match transfer {
                None => 4,
                Some(error) => project_terminal(error, outcome, terminal, &mut control.transfer),
            }
        }
    }
}

unsafe fn project_terminal(
    error: AdapterTerminal,
    outcome: *mut DrAdapterOutcomeV1,
    terminal: *mut c_void,
    status: &mut DrIoStatusV1,
) -> u8 {
    match error {
        error @ AdapterTerminal::HostError { .. } => {
            terminal.cast::<AdapterTerminal>().write(error);
            1
        }
        AdapterTerminal::CheckedError { .. } => crate::runtime_invariant_panic(),
        error => error.project(&mut *outcome, &mut crate::empty_error_carrier(), status),
    }
}

/// # Safety
/// Owner is a shared loan during an outstanding handoff, never overlapping a
/// poll. Outputs are disjoint writable homes. Record/view are borrowed until
/// the single matching reply; caller bytes remain retained by the group.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_transfer_group_action(
    owner: *const c_void,
    index: *mut usize,
    record: *mut *mut c_void,
    view: *mut c_void,
    caller: *mut *mut u8,
    length: *mut usize,
    reading: *mut u8,
) -> u8 {
    *index = 0;
    *record = ptr::null_mut();
    *caller = ptr::null_mut();
    *length = 0;
    *reading = 0;
    let owner = &*owner.cast::<HostGroupOwner>();
    let Some((position, pending)) = owner.host_call() else {
        return 0;
    };
    *index = position;
    *record = pending;
    crate::transfer_group::pending::host::dr_v1_host_pending_action(
        pending, view, caller, length, reading,
    )
}

/// Lends the native selector only for this checked configuration call. PHP
/// obtains a fresh typed source first; no driver address survives that dispatch.
/// # Safety
/// Owner/view describe the outstanding preparation. File, source, the range
/// `byte_region::host_range` forms and outputs obey
/// checked_pending_request_configure_file's retained-loan contract.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_pending_request_configure_file(
    frame: *const DrStackFrameV2,
    owner: *mut c_void,
    view: *mut c_void,
    file: *mut c_void,
    fresh_source: *const c_void,
    base: *mut u8,
    capacity: usize,
    offset: usize,
    length: usize,
    error: *mut DrInterfaceCarrierV1,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    with_host_configurator(frame, owner, view, || {
        crate::transfer_group::pending::dr_v1_checked_pending_request_configure_file(
            frame,
            view,
            file,
            fresh_source,
            crate::byte_region::host_range(base, capacity, offset, length),
            length,
            error,
            message,
            path,
            system_code,
            transferred_byte_count,
            meta,
        )
    })
}

/// # Safety
/// Owner/view describe the outstanding preparation. Stream, source, the range
/// `byte_region::host_range` forms and outputs obey
/// checked_pending_request_configure_standard's retained-loan contract.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_pending_request_configure_standard(
    frame: *const DrStackFrameV2,
    owner: *mut c_void,
    view: *mut c_void,
    stream: u8,
    fresh_source: *const c_void,
    base: *mut u8,
    capacity: usize,
    offset: usize,
    length: usize,
    error: *mut DrInterfaceCarrierV1,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    with_host_configurator(frame, owner, view, || {
        crate::transfer_group::pending::dr_v1_checked_pending_request_configure_standard(
            frame,
            view,
            stream,
            fresh_source,
            crate::byte_region::host_range(base, capacity, offset, length),
            length,
            error,
            message,
            path,
            system_code,
            transferred_byte_count,
            meta,
        )
    })
}

unsafe fn with_host_configurator(
    frame: *const DrStackFrameV2,
    owner: *mut c_void,
    view: *mut c_void,
    configure: impl FnOnce() -> u8,
) -> u8 {
    let owner = &mut *owner.cast::<HostGroupOwner>();
    let Some((index, record)) = owner.host_call() else {
        crate::runtime_invariant_panic();
    };
    if !crate::transfer_group::pending::host::matches_view(record, view) {
        crate::runtime_invariant_panic();
    }
    owner.setup.group.with_call_frame(frame, |group| {
        let (operation, request) = group.request(index);
        group
            .driver
            .with_host_configurator(operation, request, configure)
    })
}

/// # Safety
/// Owner is shared idle storage; returned region expires before mutation/drop.
/// Keys and outputs follow the existing exact operation-key region contract.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_transfer_group_read_region(
    owner: *const c_void,
    key: *const c_void,
    data: *mut *const u8,
    length: *mut usize,
    status: *mut DrIoStatusV1,
) -> u8 {
    let owner = &*owner.cast::<HostGroupOwner>();
    let region = if matches!(owner.call, Call::Idle) && !key.is_null() {
        owner
            .identity
            .operation_position(*key.cast(), true, |index| {
                owner.setup.group.request_info(index)
            })
            .map(|index| owner.setup.group.completed(index))
    } else {
        Err(Failure::invalid_input())
    };
    project_completed_region(region, data, length, status)
}

/// # Safety
/// Owner is shared idle storage. Key is a disjoint writable operation-key home.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_transfer_group_observed(
    owner: *const c_void,
    reading: u8,
    key: *mut c_void,
) -> u8 {
    project_observed_operation(&(*owner.cast::<HostGroupOwner>()).identity, reading, key)
}

/// # Safety
/// Owner is owned exclusive storage after close/failed-startup cleanup completed.
/// All host views/results are released and Error claims have been consumed.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_transfer_group_drop(
    frame: *const DrStackFrameV2,
    owner: *mut c_void,
) {
    if owner.is_null() {
        return;
    }
    let owner = owner.cast::<HostGroupOwner>();
    if !matches!((*owner).call, Call::Closed) {
        crate::runtime_invariant_panic();
    }
    ManuallyDrop::take(&mut (*owner).setup).drop_in_frame(frame);
    (*(*owner).set).active = false;
    crate::dr_v1_class_free(owner.cast());
}

#[cfg(test)]
mod tests;
