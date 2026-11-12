//! Keyed configuration shares the ordinary group owner and native cleanup.

use super::*;
use crate::transfer_group::plan::{EntryKind, OperationKey};
use crate::transfer_group::WatchSource;

/// Readonly metadata lookup; a copied operation key owns and borrows nothing.
/// Kind is 0 for a write, 1 for a read and 2 for a watch; the entry at the
/// position must be of that kind.
/// # Safety
/// Plan is null or a live shared plan. Result is aligned writable OperationKey
/// storage; all outputs are disjoint empty slots, disjoint from the plan.
/// Frame is null or live for checked-error allocation during this call only.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_transfer_plan_key(
    frame: *const DrStackFrameV2,
    plan: *const c_void,
    position: i64,
    kind: u8,
    result: *mut c_void,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    result.cast::<OperationKey>().write(OperationKey {
        owner: 0,
        position: 0,
    });
    let lookup = if plan.is_null() {
        Err(Failure::invalid_input())
    } else {
        EntryKind::from_byte(kind)
            .and_then(|kind| (*plan.cast::<TransferPlan<'static>>()).operation_key(position, kind))
    };
    match lookup {
        Ok(key) => {
            result.cast::<OperationKey>().write(key);
            *message = ptr::null_mut();
            *path = ptr::null_mut();
            *system_code = 0;
            *transferred_byte_count = 0;
            *meta = 0;
            0
        }
        Err(failure) => {
            let mut status = DrIoStatusV1::default();
            status.fail(failure);
            crate::checked_io::fail_nontransfer_io(
                frame,
                crate::checked_io::OP_ACCESS,
                crate::checked_io::TARGET_TRANSFER_PLAN,
                &status,
                IoFailureOutputs {
                    message,
                    path,
                    system_code,
                    transferred_byte_count,
                    meta,
                },
            )
        }
    }
}

/// # Safety
/// Outputs are disjoint writable usize storage. Keys are inline Copy values;
/// their owner stamp is never a pointer or an independent stream borrower.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_key_layout(size: *mut usize, align: *mut usize) {
    *size = mem::size_of::<PlanKey>();
    *align = mem::align_of::<PlanKey>();
}

/// # Safety
/// Set is null or live. Key is aligned writable queried-layout storage disjoint
/// from status/set. Active sets are permitted only for checked rejection before
/// their group-borrowed native registration storage is inspected.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_key(
    set: *mut c_void,
    index: usize,
    key: *mut c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    key.cast::<PlanKey>().write(PlanKey { owner: 0, value: 0 });
    let set = set.cast::<SetOwner>();
    if set.is_null() || (*set).active {
        (*status).fail(Failure::invalid_input());
        return 3;
    }
    (*status).complete(
        (*set)
            .native
            .key(index)
            .ok_or_else(Failure::invalid_input)
            .map(|value| {
                key.cast::<PlanKey>().write(PlanKey {
                    owner: (*set).identity,
                    value,
                });
                0
            }),
    )
}

unsafe fn allocate_plan(frame: *const DrStackFrameV2, plan: TransferPlan<'static>) -> *mut c_void {
    let owner = allocate_owner::<TransferPlan<'static>>(frame);
    owner.write(plan);
    owner.cast()
}

pub(super) unsafe fn take_plan(owner: *mut c_void) -> Option<TransferPlan<'static>> {
    if owner.is_null() {
        return None;
    }
    let plan = owner.cast::<TransferPlan<'static>>().read();
    crate::dr_v1_class_free(owner.cast());
    Some(plan)
}

/// # Safety
/// Key/descriptor are null or readable records. Invalid metadata is permitted
/// for checked rejection before any byte slice. Valid nonempty data addresses
/// name live storage with the descriptor's access mode, disjoint from outputs.
/// The compiler retains that loan until plan drop or consuming begin/append;
/// moving the opaque owner does not move or copy bytes. Frame is null or live
/// for owner-allocation failure reporting during this call only.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_plan_new(
    frame: *const DrStackFrameV2,
    key: *const c_void,
    buffer: *const DrTransferBufferV1,
    status: *mut DrIoStatusV1,
) -> *mut c_void {
    *status = DrIoStatusV1::default();
    if key.is_null()
        || buffer.is_null()
        || (*key.cast::<PlanKey>()).owner == 0
        || !(*buffer).valid()
    {
        (*status).fail(Failure::invalid_input());
        return ptr::null_mut();
    }
    match TransferPlan::new(*key.cast::<PlanKey>(), (*buffer).buffer()) {
        Ok(plan) => allocate_plan(frame, plan),
        Err(failure) => {
            (*status).fail(failure);
            ptr::null_mut()
        }
    }
}

/// Consumes the input plan on success AND failure. Byte/stream owners remain
/// with their callers; failed setup releases descriptor loans without I/O.
/// # Safety
/// Owner is null or an exclusively owned live plan. Key/buffer satisfy new's
/// record/storage requirements; malformed/conflicting incoming metadata and
/// foreign keys are allowed only for rejection before slice creation. Repeated
/// registration keys are valid when the retained regions do not conflict.
/// Successful append retains all old and incoming loans in the returned plan.
/// Frame is null or live for owner-allocation failure reporting during this call.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_plan_append(
    frame: *const DrStackFrameV2,
    owner: *mut c_void,
    key: *const c_void,
    buffer: *const DrTransferBufferV1,
    status: *mut DrIoStatusV1,
) -> *mut c_void {
    *status = DrIoStatusV1::default();
    let Some(plan) = take_plan(owner) else {
        (*status).fail(Failure::invalid_input());
        return ptr::null_mut();
    };
    if key.is_null()
        || buffer.is_null()
        || !plan.accepts_key(*key.cast::<PlanKey>())
        || !(*buffer).valid()
        || plan.conflicts_with((*buffer).region())
    {
        (*status).fail(Failure::invalid_input());
        return ptr::null_mut();
    }
    match plan.append(*key.cast::<PlanKey>(), (*buffer).buffer()) {
        Ok(plan) => allocate_plan(frame, plan),
        Err((failure, _, _)) => {
            (*status).fail(failure);
            ptr::null_mut()
        }
    }
}

/// Creates a read or write plan through the shared checked-I/O scalar transport.
/// Descriptor and status layouts stay runtime-owned; no transfer starts here.
/// # Safety
/// Key and region satisfy the raw plan-new contract. The compiler retains the
/// region's exclusive read destination or shared write source loan until drop.
/// Frame is null or live for this call. All outputs are disjoint writable EMPTY
/// slots, disjoint from inputs; neither frame nor output addresses are retained.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_transfer_plan_new(
    frame: *const DrStackFrameV2,
    key: *const c_void,
    data: *mut u8,
    length: usize,
    reading: u8,
    result: *mut *mut c_void,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    let buffer = DrTransferBufferV1 {
        data,
        length,
        reading,
    };
    let mut status = DrIoStatusV1::default();
    let owner = dr_v1_transfer_plan_new(frame, key, &buffer, &mut status);
    checked_plan_result(
        frame,
        owner,
        &status,
        result,
        IoFailureOutputs {
            message,
            path,
            system_code,
            transferred_byte_count,
            meta,
        },
    )
}

/// Consumes the old plan on success AND checked failure, preserving callers'
/// byte owners. Successful append forwards every old and incoming buffer loan.
/// # Safety
/// Plan/key/region satisfy the raw consuming-append contract, including
/// compiler-enforced shared/exclusive retained loans. Invalid/conflicting incoming
/// metadata is permitted for checked rejection before byte-slice creation.
/// Frame is null or live for this call. All outputs are disjoint writable EMPTY
/// slots, disjoint from inputs; neither frame nor output addresses are retained.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_transfer_plan_append(
    frame: *const DrStackFrameV2,
    plan: *mut c_void,
    key: *const c_void,
    data: *mut u8,
    length: usize,
    reading: u8,
    result: *mut *mut c_void,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    let buffer = DrTransferBufferV1 {
        data,
        length,
        reading,
    };
    let mut status = DrIoStatusV1::default();
    let owner = dr_v1_transfer_plan_append(frame, plan, key, &buffer, &mut status);
    checked_plan_result(
        frame,
        owner,
        &status,
        result,
        IoFailureOutputs {
            message,
            path,
            system_code,
            transferred_byte_count,
            meta,
        },
    )
}

/// Creates a plan whose first entry watches a runtime-backed source (Decision
/// 0140). Source is the `WatchSource` word: the source's kind above bit 32 and
/// that kind's token below. The plan belongs to no readiness set until a keyed
/// entry is appended. Nothing is waited for before a group begins.
/// # Safety
/// Frame is null or live for this call. All outputs are disjoint writable EMPTY
/// slots; neither frame nor output addresses are retained. The compiler keeps
/// the watched source in shared use until the plan, or the group begun from
/// it, is dropped.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_transfer_plan_watch_new(
    frame: *const DrStackFrameV2,
    source: u64,
    result: *mut *mut c_void,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    let mut status = DrIoStatusV1::default();
    let owner = match WatchSource::from_bits(source).and_then(TransferPlan::watching) {
        Ok(plan) => allocate_plan(frame, plan),
        Err(failure) => {
            status.fail(failure);
            ptr::null_mut()
        }
    };
    checked_plan_result(
        frame,
        owner,
        &status,
        result,
        IoFailureOutputs {
            message,
            path,
            system_code,
            transferred_byte_count,
            meta,
        },
    )
}

/// Appends a watch. Consumes the old plan on success and checked failure,
/// forwarding every earlier loan only into a successful replacement.
/// # Safety
/// Plan is null for checked rejection or an exclusively owned live plan. The
/// frame, source and outputs obey `dr_v1_checked_transfer_plan_watch_new`.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_transfer_plan_watch_append(
    frame: *const DrStackFrameV2,
    plan: *mut c_void,
    source: u64,
    result: *mut *mut c_void,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    let mut status = DrIoStatusV1::default();
    let appended = match take_plan(plan) {
        Some(plan) => WatchSource::from_bits(source)
            .and_then(|source| plan.append_watch(source).map_err(|(failure, _)| failure)),
        None => Err(Failure::invalid_input()),
    };
    let owner = match appended {
        Ok(plan) => allocate_plan(frame, plan),
        Err(failure) => {
            status.fail(failure);
            ptr::null_mut()
        }
    };
    checked_plan_result(
        frame,
        owner,
        &status,
        result,
        IoFailureOutputs {
            message,
            path,
            system_code,
            transferred_byte_count,
            meta,
        },
    )
}

/// `dr_v1_checked_transfer_plan_new` for a PHP host, which names the buffer by range.
/// # Safety
/// As `dr_v1_checked_transfer_plan_new`, for the range `byte_region::host_range` forms.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_transfer_plan_new(
    frame: *const DrStackFrameV2,
    key: *const c_void,
    base: *mut u8,
    capacity: usize,
    offset: usize,
    length: usize,
    reading: u8,
    result: *mut *mut c_void,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    dr_v1_checked_transfer_plan_new(
        frame,
        key,
        crate::byte_region::host_range(base, capacity, offset, length),
        length,
        reading,
        result,
        message,
        path,
        system_code,
        transferred_byte_count,
        meta,
    )
}

/// `dr_v1_checked_transfer_plan_append` for a PHP host, which names the buffer by range.
/// # Safety
/// As `dr_v1_checked_transfer_plan_append`, for the range `byte_region::host_range` forms.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_transfer_plan_append(
    frame: *const DrStackFrameV2,
    plan: *mut c_void,
    key: *const c_void,
    base: *mut u8,
    capacity: usize,
    offset: usize,
    length: usize,
    reading: u8,
    result: *mut *mut c_void,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    dr_v1_checked_transfer_plan_append(
        frame,
        plan,
        key,
        crate::byte_region::host_range(base, capacity, offset, length),
        length,
        reading,
        result,
        message,
        path,
        system_code,
        transferred_byte_count,
        meta,
    )
}

unsafe fn checked_plan_result(
    frame: *const DrStackFrameV2,
    owner: *mut c_void,
    status: &DrIoStatusV1,
    result: *mut *mut c_void,
    outputs: IoFailureOutputs,
) -> u8 {
    *result = owner;
    if owner.is_null() {
        return crate::checked_io::dr_v1_checked_transfer_plan_failure(
            frame,
            status,
            outputs.message,
            outputs.path,
            outputs.system_code,
            outputs.transferred_byte_count,
            outputs.meta,
        );
    }
    *outputs.message = ptr::null_mut();
    *outputs.path = ptr::null_mut();
    *outputs.system_code = 0;
    *outputs.transferred_byte_count = 0;
    *outputs.meta = 0;
    0
}

/// # Safety
/// Plan is null or a live exclusively owned plan, dropped once. Descriptor
/// loans end here; no native stream or original byte owner is dropped.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_plan_drop(plan: *mut c_void) {
    drop(take_plan(plan));
}

/// Consumes plan on success AND failure. The resulting group uses plan order
/// while native request slots stay registered, stable and set-owned.
/// # Safety
/// Set/plan are null or live owners, disjoint from writable status. Plan is
/// exclusively owned. Active/wrong/retired membership is allowed for rejection
/// before native registration/budget access. Otherwise budget is live for this
/// call and the compiler exclusively borrows the whole set and retains every
/// original plan loan until group drop. The frozen budget is copied inline.
/// Drop the group before its set/storage.
/// Frame is null or live for owner-allocation failure reporting during this call.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_plan_begin(
    frame: *const DrStackFrameV2,
    set: *mut c_void,
    plan: *mut c_void,
    budget: *const DrWaitBudgetV1,
    status: *mut DrIoStatusV1,
) -> *mut c_void {
    *status = DrIoStatusV1::default();
    let Some(plan) = take_plan(plan) else {
        (*status).fail(Failure::invalid_input());
        return ptr::null_mut();
    };
    let set = set.cast::<SetOwner>();
    if set.is_null() || (*set).active {
        (*status).fail(Failure::invalid_input());
        return ptr::null_mut();
    }
    let prepared = match plan.prepare((*set).identity, |key| (*set).native.index_of(key)) {
        Ok(prepared) => prepared,
        Err((failure, _)) => {
            (*status).fail(failure);
            return ptr::null_mut();
        }
    };
    // A watch needs the complete registered owner's group; this bare native
    // owner has no place in its wait for one.
    if budget.is_null() || prepared.watches.len() != 0 {
        (*status).fail(Failure::invalid_input());
        return ptr::null_mut();
    }
    begin_group(
        frame,
        set,
        prepared.buffers,
        Some(prepared.selection),
        prepared.identity,
        budget,
        status,
    )
}

#[cfg(test)]
mod tests;
