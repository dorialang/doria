//! Checked scalar transport over the complete registered owner's shared group.

use super::*;
use crate::transfer_group::adapter::AdapterTerminal;
use crate::transfer_group::SetupFailure;
use doria_diagnostic_catalogue::IoMessageOperation;

/// Fetches through the registered complete adapters, with one call-owned wait.
/// Private wait tags are 0 immediate, 1 indefinite, 2 signed relative nanoseconds,
/// and 3 an exact two-word Deadline (low word first). Success returns tag 0 with
/// result 0 Completed, 1 TimedOut, or 2 Exhausted. A completion has its original
/// directional key and outcome 0 absent, 1 Data/Accepted, 2 WouldBlock, 3 EOF,
/// or 4 TimedOut. Returns 1 with ordinary IoError fields, or 2 with the original
/// owned adapter Error. No public enum layout is imposed by this scalar transport.
/// Control retains the complete raw stop/cancellation facts for the caller;
/// projecting a transfer result does not erase those independent facts.
/// # Safety
/// Owner is null for checked rejection or live exclusive adapter-group storage.
/// Error is a writable EMPTY interface carrier (a nonempty carrier is rejected
/// unchanged). All outputs are disjoint writable slots, disjoint from the group
/// and its retained storage. Key has OperationKey size/alignment. No frame, wait
/// or output location is retained. Control has DrCompletionStatusV1 size/alignment.
/// No region loan overlaps this mutation.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_transfer_group_observe(
    frame: *const DrStackFrameV2,
    owner: *mut c_void,
    wait_kind: u8,
    low: u64,
    high: i64,
    result: *mut u8,
    reading: *mut u8,
    key: *mut c_void,
    outcome: *mut u8,
    byte_count: *mut usize,
    error: *mut DrInterfaceCarrierV1,
    control: *mut c_void,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    *result = 0;
    *reading = 0;
    *key.cast::<super::super::plan::OperationKey>() = super::super::plan::OperationKey {
        owner: 0,
        position: 0,
    };
    *outcome = 0;
    *byte_count = 0;
    let control = &mut *control.cast::<DrCompletionStatusV1>();
    *control = DrCompletionStatusV1::default();
    let outputs = IoFailureOutputs {
        message,
        path,
        system_code,
        transferred_byte_count,
        meta,
    };
    checked_group_result(
        frame,
        0,
        &DrIoStatusV1::default(),
        IoMessageOperation::Wait,
        &outputs,
    );
    if owner.is_null() || !(*error).object.is_null() || !(*error).vtable.is_null() {
        return checked_observation_failure(frame, Failure::invalid_input(), &outputs);
    }
    adapter::clear_observed(owner);
    let budget = match DrWaitBudgetV1::from_wait(wait_kind, low, high) {
        Ok(budget) => budget,
        Err(failure) => return checked_observation_failure(frame, failure, &outputs),
    };
    let (mut registration, mut position) = (0, 0);
    let mut transfer = crate::transfer_group::adapter::DrAdapterOutcomeV1::default();
    let tag = adapter::dr_v1_transfer_adapter_group_observe(
        frame,
        owner,
        &budget,
        &mut registration,
        &mut position,
        &mut transfer,
        error,
        ptr::from_mut(control).cast(),
    );
    match tag {
        5 | 6 => {
            *result = if tag == 6 { 1 } else { 2 };
            0
        }
        7 => checked_observation_failure(
            frame,
            control
                .wait_failure
                .failure()
                .unwrap_or_else(|| crate::runtime_invariant_panic()),
            &outputs,
        ),
        0 | 1 | 3 | 4 => {
            *reading = if adapter::dr_v1_transfer_adapter_group_observed(owner, 1, key) == 0 {
                1
            } else if adapter::dr_v1_transfer_adapter_group_observed(owner, 0, key) == 0 {
                0
            } else {
                crate::runtime_invariant_panic()
            };
            if tag == 1 {
                return 2;
            }
            if tag == 3 {
                // The complete adapter alone owns transfer errors. A platform
                // terminal here is rejected preparation/configuration, not I/O.
                return checked_group_result(
                    frame,
                    3,
                    &control.transfer,
                    IoMessageOperation::Configure,
                    &outputs,
                );
            }
            if tag == 0 {
                if transfer.tag > if *reading == 1 { 3 } else { 1 }
                    || (transfer.tag == 0 && transfer.has_count != 1)
                    || i64::try_from(transfer.count).is_err()
                {
                    crate::runtime_invariant_panic();
                }
                *outcome = transfer.tag + 1;
                *byte_count = transfer.count;
            }
            0
        }
        _ => crate::runtime_invariant_panic(),
    }
}

unsafe fn checked_observation_failure(
    frame: *const DrStackFrameV2,
    failure: Failure,
    outputs: &IoFailureOutputs,
) -> u8 {
    let mut status = DrIoStatusV1::default();
    status.fail(failure);
    crate::checked_io::dr_v1_checked_readiness_failure(
        frame,
        IoMessageOperation::Wait as u8,
        &status,
        outputs.message,
        outputs.path,
        outputs.system_code,
        outputs.transferred_byte_count,
        outputs.meta,
    )
}

/// Consumes the plan and begins transfers through the complete pending adapters.
/// Returns 0 with an owned group, 1 with Configure/TransferGroup IoError fields,
/// or 2 with the original owned checked preparation error. Failed setup submits
/// no work, releases the plan's loans and leaves no active group.
/// # Safety
/// Set is null for checked rejection or live exclusive pending-set storage.
/// Plan is null or owned plan storage, consumed on success and failure; its
/// retained buffer loans outlive successful group execution. Frame is current
/// or null. Outputs are disjoint writable EMPTY slots; a nonempty error carrier
/// is permitted only for unchanged checked rejection. No output is retained.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_transfer_group_begin(
    frame: *const DrStackFrameV2,
    set: *mut c_void,
    plan: *mut c_void,
    owner: *mut *mut c_void,
    error: *mut DrInterfaceCarrierV1,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    checked_group_begin_result(
        frame,
        owner,
        error,
        IoFailureOutputs {
            message,
            path,
            system_code,
            transferred_byte_count,
            meta,
        },
        || {
            adapter::begin_adapter_plan(
                frame,
                set,
                plan,
                &DrWaitBudgetV1::indefinite(),
                error,
                true,
            )
        },
    )
}

/// Begins with explicitly group-local controls. Null omits that control;
/// no set-scoped cancellation policy is selected implicitly. The exact deadline
/// is copied and cancellation is retained without retaining either input address.
/// # Safety
/// Frame, set, plan and outputs obey the v1 checked begin contract. A deadline
/// is null or points to a live exact DrDeadlineV1 for this call; cancellation is
/// null or carries a live state claim for acquisition during this call.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_checked_transfer_group_begin(
    frame: *const DrStackFrameV2,
    set: *mut c_void,
    plan: *mut c_void,
    deadline: *const crate::deadline::DrDeadlineV1,
    cancellation: *mut crate::cancellation::DrCancellationV1,
    owner: *mut *mut c_void,
    error: *mut DrInterfaceCarrierV1,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    let budget = deadline
        .as_ref()
        .map_or_else(DrWaitBudgetV1::indefinite, |deadline| {
            DrWaitBudgetV1::from_deadline(*deadline)
        });
    checked_group_begin_result(
        frame,
        owner,
        error,
        IoFailureOutputs {
            message,
            path,
            system_code,
            transferred_byte_count,
            meta,
        },
        || {
            adapter::begin_adapter_plan_controlled(
                frame,
                set,
                plan,
                &budget,
                cancellation,
                error,
                true,
            )
        },
    )
}

unsafe fn checked_group_begin_result(
    frame: *const DrStackFrameV2,
    owner: *mut *mut c_void,
    error: *mut DrInterfaceCarrierV1,
    outputs: IoFailureOutputs,
    begin: impl FnOnce() -> Result<*mut c_void, SetupFailure<AdapterTerminal>>,
) -> u8 {
    *owner = ptr::null_mut();
    let mut status = DrIoStatusV1::default();
    checked_group_result(frame, 0, &status, IoMessageOperation::Configure, &outputs);
    let failure = match begin() {
        Ok(group) => {
            *owner = group;
            return 0;
        }
        Err(SetupFailure::Platform(failure))
        | Err(SetupFailure::Preparation(AdapterTerminal::PlatformFailure(failure))) => failure,
        Err(SetupFailure::Preparation(AdapterTerminal::CheckedError { error: owned, .. })) => {
            *error = owned.into_carrier();
            return 2;
        }
        _ => crate::runtime_invariant_panic(),
    };
    let tag = status.fail(failure);
    checked_group_result(frame, tag, &status, IoMessageOperation::Configure, &outputs)
}

/// Returns 0 with an optional readonly completed prefix, or 1 with IoError
/// fields. Unavailability is successful absence, distinct from an empty prefix.
/// # Safety
/// Owner/key are null for checked rejection or live shared group/key storage.
/// Outputs are disjoint writable EMPTY slots, disjoint from all inputs and the
/// retained buffer. A present region expires before group mutation/destruction.
/// Frame is null or live for this call; no output address or frame is retained.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_transfer_group_read_region(
    frame: *const DrStackFrameV2,
    owner: *const c_void,
    key: *const c_void,
    data: *mut *const u8,
    length: *mut usize,
    present: *mut u8,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    *data = ptr::null();
    *length = 0;
    *present = 0;
    let mut status = DrIoStatusV1::default();
    let tag = if owner.is_null() || key.is_null() {
        status.fail(Failure::invalid_input())
    } else {
        adapter::dr_v1_transfer_adapter_group_read_region(owner, key, data, length, &mut status)
    };
    let tag = match tag {
        0 => {
            *present = 1;
            0
        }
        1 => 0,
        3 => 3,
        _ => crate::runtime_invariant_panic(),
    };
    checked_group_result(
        frame,
        tag,
        &status,
        IoMessageOperation::Access,
        &IoFailureOutputs {
            message,
            path,
            system_code,
            transferred_byte_count,
            meta,
        },
    )
}

/// Reuses one original operation after checked signed range validation.
/// Returns 0 on successful resubmission or 1 with Configure/TransferGroup IoError
/// fields. Rejection never submits, invokes an adapter, or consumes its Error.
/// # Safety
/// Owner/key are null for checked rejection or live exclusive group/key storage.
/// No completion/region loan may overlap group mutation. Reading is 0 or 1;
/// other values are rejected. Outputs are disjoint writable EMPTY error slots.
/// Frame is null or live for this call and is not retained.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_transfer_group_resubmit(
    frame: *const DrStackFrameV2,
    owner: *mut c_void,
    key: *const c_void,
    reading: u8,
    offset: i64,
    length: i64,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    let mut status = DrIoStatusV1::default();
    let tag = match (usize::try_from(offset), usize::try_from(length)) {
        (Ok(offset), Ok(length)) if !owner.is_null() && !key.is_null() => {
            adapter::dr_v1_transfer_adapter_group_submit_key(
                frame,
                owner,
                key,
                reading,
                offset,
                length,
                &mut status,
            )
        }
        _ => status.fail(Failure::invalid_input()),
    };
    checked_group_result(
        frame,
        tag,
        &status,
        IoMessageOperation::Configure,
        &IoFailureOutputs {
            message,
            path,
            system_code,
            transferred_byte_count,
            meta,
        },
    )
}

unsafe fn checked_group_result(
    frame: *const DrStackFrameV2,
    tag: u8,
    status: &DrIoStatusV1,
    operation: IoMessageOperation,
    outputs: &IoFailureOutputs,
) -> u8 {
    match tag {
        0 => {
            *outputs.message = ptr::null_mut();
            *outputs.path = ptr::null_mut();
            *outputs.system_code = 0;
            *outputs.transferred_byte_count = 0;
            *outputs.meta = 0;
            0
        }
        3 => crate::checked_io::dr_v1_checked_transfer_group_failure(
            frame,
            operation as u8,
            status,
            outputs.message,
            outputs.path,
            outputs.system_code,
            outputs.transferred_byte_count,
            outputs.meta,
        ),
        _ => crate::runtime_invariant_panic(),
    }
}
