//! Shared private group lifetime, buffer validation and transport for native adapters.

use core::{
    ffi::c_void,
    mem::{self, ManuallyDrop},
    ptr, slice,
    sync::atomic::{AtomicUsize, Ordering},
};

use super::{Buffer, BufferList, DrCompletionStatusV1, FixedSlice, PlanKey, Stop, TransferPlan};
use crate::checked_io::IoFailureOutputs;
use crate::io_status::DrIoStatusV1;
use crate::platform_io::{Failure, Reason};
use crate::transfer_group::pending::DrPendingTransferAdapterV1;
use crate::wait_budget::DrWaitBudgetV1;
use crate::{DrInterfaceCarrierV1, DrStackFrameV2, DrStringV1};

mod adapter;
mod checked_group;
pub(crate) mod plan;
pub(crate) mod registration;

/// Constructs the same opaque owner used by membership and group transport.
/// Native resources remain lazy; actual creation failures use the existing
/// Register/ReadinessSet checked-I/O projection, not backend-owned status layouts.
/// Owner heap-allocation failure instead follows fatal source-identified P1601.
/// Success transfers one owner pointer, whose readiness-set drop frees it once.
/// # Safety
/// Frame is null or live for this call. Outputs are disjoint writable EMPTY
/// slots, including the owner and any owned string fields.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_readiness_set_new(
    frame: *const DrStackFrameV2,
    owner: *mut *mut c_void,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    *owner = ptr::null_mut();
    *message = ptr::null_mut();
    *path = ptr::null_mut();
    *system_code = 0;
    *transferred_byte_count = 0;
    *meta = 0;
    let mut status = DrIoStatusV1::default();
    let created = adapter::dr_v1_readiness_set_new(frame, &mut status);
    if !created.is_null() {
        *owner = created;
        return 0;
    }
    crate::checked_io::dr_v1_checked_readiness_failure(
        frame,
        doria_diagnostic_catalogue::IoMessageOperation::Register as u8,
        &status,
        message,
        path,
        system_code,
        transferred_byte_count,
        meta,
    )
}

/// Checked admission of the original complete carrier through the existing
/// owning membership core. Source-only members need no pending descriptor.
/// Failure consumes the ordinary taken value; no callbacks run during admission.
/// Returns 0 on success, 1 with existing Register/ReadinessSet IoError fields.
/// # Safety
/// Frame is null or live for this call. Set/source/adapter/value/key obey the
/// raw owned-admission contract. Error fields are disjoint writable EMPTY slots,
/// disjoint from all inputs and outputs. Frame and output addresses are not retained.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_readiness_set_add_owned(
    frame: *const DrStackFrameV2,
    set: *mut c_void,
    source: *const c_void,
    resolve: Option<crate::ResolveSourceCallback>,
    pending_adapter: *const DrPendingTransferAdapterV1,
    value: *mut DrInterfaceCarrierV1,
    key: *mut c_void,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    let mut status = DrIoStatusV1::default();
    let tag = adapter::dr_v1_readiness_set_add_owned(
        frame,
        set,
        source,
        resolve,
        pending_adapter,
        value,
        key,
        &mut status,
    );
    checked_readiness_membership_result(
        frame,
        tag,
        &status,
        doria_diagnostic_catalogue::IoMessageOperation::Register,
        IoFailureOutputs {
            message,
            path,
            system_code,
            transferred_byte_count,
            meta,
        },
    )
}

/// Checked removal returns the unchanged original object and descriptor only
/// after native membership retires. Failure preserves membership and ownership.
/// Returns 0 on success, 1 with existing Unregister/ReadinessSet IoError fields.
/// # Safety
/// Frame is null or live for this call. Set/key/value obey raw owned removal;
/// nonempty value storage is permitted only for unchanged checked rejection.
/// Error fields are disjoint writable EMPTY slots, disjoint from every input
/// and output. Frame and output addresses are not retained.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_readiness_set_remove_owned(
    frame: *const DrStackFrameV2,
    set: *mut c_void,
    key: *const c_void,
    value: *mut DrInterfaceCarrierV1,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    let mut status = DrIoStatusV1::default();
    let tag = adapter::dr_v1_readiness_set_remove_owned(set, key, value, &mut status);
    checked_readiness_membership_result(
        frame,
        tag,
        &status,
        doria_diagnostic_catalogue::IoMessageOperation::Unregister,
        IoFailureOutputs {
            message,
            path,
            system_code,
            transferred_byte_count,
            meta,
        },
    )
}

/// Checked access lends the original complete carrier without transferring it.
/// Returns 0 on success, 1 with existing Access/ReadinessSet IoError fields.
/// # Safety
/// Frame is null or live for this call. Set/key/value obey raw owned-member
/// lookup, including NONOWNING output and compiler-enforced access/lifetime.
/// Value home follows raw lookup and returns the pinned original carrier's
/// address on success, null on rejection; it does not extend its lifetime.
/// Error fields are disjoint writable EMPTY slots, disjoint from every input
/// and output. Frame and output addresses are not retained.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_readiness_set_get_owned(
    frame: *const DrStackFrameV2,
    set: *mut c_void,
    key: *const c_void,
    value: *mut DrInterfaceCarrierV1,
    value_home: *mut *mut DrInterfaceCarrierV1,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    let mut status = DrIoStatusV1::default();
    let tag = adapter::dr_v1_readiness_set_get_owned(set, key, value, value_home, &mut status);
    checked_readiness_membership_result(
        frame,
        tag,
        &status,
        doria_diagnostic_catalogue::IoMessageOperation::Access,
        IoFailureOutputs {
            message,
            path,
            system_code,
            transferred_byte_count,
            meta,
        },
    )
}

unsafe fn checked_readiness_membership_result(
    frame: *const DrStackFrameV2,
    tag: u8,
    status: &DrIoStatusV1,
    operation: doria_diagnostic_catalogue::IoMessageOperation,
    outputs: IoFailureOutputs,
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
        3 => crate::checked_io::dr_v1_checked_readiness_failure(
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

#[cfg(windows)]
use crate::byte_io::{NativeTransferGroup as NativeGroup, NativeTransferSet};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::unix_readiness::{NativeTransferGroup as NativeGroup, NativeTransferSet};

#[repr(C)]
pub struct DrTransferBufferV1 {
    pub(crate) data: *mut u8,
    pub(crate) length: usize,
    pub(crate) reading: u8,
}

impl DrTransferBufferV1 {
    fn valid(&self) -> bool {
        self.reading <= 1
            && self.length <= isize::MAX as usize
            && (self.length == 0 || !self.data.is_null())
            && (self.data as usize).checked_add(self.length).is_some()
    }

    fn region(&self) -> (usize, usize, bool) {
        (self.data as usize, self.length, self.reading != 0)
    }

    /// All metadata and conflicting loans have already been checked. The
    /// compiler retains the original storage until this descriptor is dropped.
    unsafe fn buffer<'b>(&self) -> Buffer<'b> {
        let data = if self.length == 0 {
            ptr::NonNull::<u8>::dangling().as_ptr()
        } else {
            self.data
        };
        if self.reading != 0 {
            Buffer::Read(slice::from_raw_parts_mut(data, self.length))
        } else {
            Buffer::Write(slice::from_raw_parts(data, self.length))
        }
    }
}

fn valid_descriptors(descriptors: &[DrTransferBufferV1]) -> bool {
    descriptors
        .iter()
        .enumerate()
        .all(|(position, descriptor)| {
            descriptor.valid()
                && descriptors[..position]
                    .iter()
                    .all(|prior| !super::regions_conflict(descriptor.region(), prior.region()))
        })
}

/// # Safety
/// Outputs are disjoint writable usize storage; returned status storage is
/// aligned to the reported alignment. This is a private runtime record layout.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_outcome_layout(size: *mut usize, align: *mut usize) {
    *size = mem::size_of::<DrCompletionStatusV1>();
    *align = mem::align_of::<DrCompletionStatusV1>();
}

struct SetOwner {
    native: NativeTransferSet,
    active: bool,
    // This stamp survives dynamic membership edits, never set destruction.
    identity: usize,
    // Dynamic registration values never repeat, including failed admissions.
    next_registration: u64,
}

static NEXT_SET_IDENTITY: AtomicUsize = AtomicUsize::new(0);

fn next_identity(counter: &AtomicUsize) -> Result<usize, Failure> {
    counter
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
            last.checked_add(1)
        })
        .map(|last| last + 1)
        .map_err(|_| exhausted())
}

/// Allocate only a headerless runtime class owner. Buffer, request-table and
/// native-resource allocation keep their existing independent failure contracts.
/// # Safety
/// Frame is null or live for this call. T is a runtime-owned class payload; its
/// storage is initialized before use and freed through the matching class free.
unsafe fn allocate_owner<T>(frame: *const DrStackFrameV2) -> *mut T {
    crate::dr_v2_class_allocate(frame, mem::size_of::<T>(), mem::align_of::<T>()).cast()
}

/// # Safety
/// Frame is null or live for allocation failure reporting during this call.
pub(crate) unsafe fn allocate_set(
    frame: *const DrStackFrameV2,
    native: NativeTransferSet,
) -> Result<*mut c_void, Failure> {
    let identity = next_identity(&NEXT_SET_IDENTITY)?;
    let owner = allocate_owner::<SetOwner>(frame);
    owner.write(SetOwner {
        native,
        active: false,
        identity,
        next_registration: 0,
    });
    Ok(owner.cast())
}

/// # Safety
/// Owner is null or live. An active set may be supplied only for checked
/// rejection; no native state or retained loan is accessed on that path. An
/// inactive fixed owning set must be uniquely owned. Borrowed sets are rejected
/// before consumption. Failure leaves ownership with the
/// caller; success consumes the allocation and returns its live native streams.
pub(crate) unsafe fn take_set(owner: *mut c_void) -> Result<NativeTransferSet, Failure> {
    if owner.is_null() {
        return Err(Failure::invalid_input());
    }
    let owner = owner.cast::<SetOwner>();
    if (*owner).active || (*owner).native.is_borrowed() {
        return Err(Failure::invalid_input());
    }
    let SetOwner { native, .. } = owner.read();
    crate::dr_v1_class_free(owner.cast());
    Ok(native)
}

fn exhausted() -> Failure {
    Failure {
        reason: Reason::ResourceExhausted,
        system_code: None,
        transferred_byte_count: None,
    }
}

struct GroupOwner {
    group: ManuallyDrop<NativeGroup<'static, 'static>>,
    set: *mut SetOwner,
    identity: super::plan::GroupIdentity,
}

impl Drop for GroupOwner {
    fn drop(&mut self) {
        unsafe {
            ManuallyDrop::drop(&mut self.group);
            (*self.set).active = false;
        }
    }
}

/// # Safety
/// Set is live; status is writable storage disjoint from the set and all inputs.
/// An already-active/borrowed set or mismatched count is permitted for rejection;
/// descriptors, budget and retained loans are not accessed on those paths.
/// Otherwise descriptors is readable storage for count entries. Invalid tags,
/// oversized lengths and null nonempty regions are permitted for checked
/// rejection before any byte slice is formed. Conflicting region overlaps and
/// address-range overflow are also rejected. If those checks pass, budget is
/// live for this call and each nonempty data pointer is valid for length bytes.
/// Writable regions and all inputs/outputs are disjoint. The inactive set is exclusively
/// borrowed; on success all regions stay live and unavailable until group drop.
/// The frozen budget is copied inline. The owner cannot move its set or be used
/// concurrently; it is dropped once before the set. Frame is null or live for this call only.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_group_begin(
    frame: *const DrStackFrameV2,
    set: *mut c_void,
    buffers: *const DrTransferBufferV1,
    count: usize,
    budget: *const DrWaitBudgetV1,
    status: *mut DrIoStatusV1,
) -> *mut c_void {
    *status = DrIoStatusV1::default();
    let set = set.cast::<SetOwner>();
    if set.is_null() || (*set).active || (*set).native.is_borrowed() {
        (*status).fail(Failure::invalid_input());
        return ptr::null_mut();
    }
    if count != (*set).native.len() {
        (*status).fail(Failure::invalid_input());
        return ptr::null_mut();
    }
    if buffers.is_null() || budget.is_null() {
        (*status).fail(Failure::invalid_input());
        return ptr::null_mut();
    }
    let descriptors = slice::from_raw_parts(buffers, count);
    if !valid_descriptors(descriptors) {
        (*status).fail(Failure::invalid_input());
        return ptr::null_mut();
    }
    let buffers = match BufferList::from_iter(descriptors.iter().map(|b| b.buffer())) {
        Ok(buffers) => buffers,
        Err(failure) => {
            (*status).fail(failure);
            return ptr::null_mut();
        }
    };
    let identity = match super::plan::next_plan_identity() {
        Ok(identity) => identity,
        Err(failure) => {
            (*status).fail(failure);
            return ptr::null_mut();
        }
    };
    begin_group(frame, set, buffers, None, identity, budget, status)
}

/// Both entry paths finish validation and allocation before this single native
/// group lifetime gate. Full-set callers need no selection allocation.
unsafe fn begin_group(
    frame: *const DrStackFrameV2,
    set: *mut SetOwner,
    buffers: BufferList<'static>,
    selection: Option<FixedSlice<usize>>,
    identity: u64,
    budget: *const DrWaitBudgetV1,
    status: *mut DrIoStatusV1,
) -> *mut c_void {
    // Finish every allocation before any transfer can consume caller bytes.
    let owner = allocate_owner::<GroupOwner>(frame);
    (*set).active = true;
    let result = match selection {
        Some(selection) => (*set).native.begin_selected(buffers, &*budget, selection),
        None => (*set).native.begin(buffers, &*budget),
    };
    let group = match result {
        Ok(group) => group,
        Err(failure) => {
            (*set).active = false;
            crate::dr_v1_class_free(owner.cast());
            (*status).fail(failure);
            return ptr::null_mut();
        }
    };
    let group = GroupOwner {
        group: ManuallyDrop::new(group),
        set,
        identity: super::plan::GroupIdentity::new(identity),
    };
    owner.write(group);
    owner.cast()
}

#[cfg(test)]
mod tests;

/// Returns 5 when no transfers remain, 7 for a wait failure before taking a
/// queued completion, or existing private transfer tags. Tag 7 has no identity.
/// # Safety
/// Owner is live exclusively borrowed; key/index/status are disjoint writable
/// storage, with status having the layout from dr_v1_transfer_outcome_layout.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_group_next(
    owner: *mut c_void,
    key: *mut u64,
    index: *mut usize,
    status: *mut c_void,
) -> u8 {
    observe_group(owner, None, key, index, status)
}

/// Observes with a call-owned budget. Tag 5 means exhausted; tag 6 means only
/// this fetch timed out (including an immediate poll with no ready completion).
/// Tag 7 reports a wait failure once, preserving queued transfers for later fetches.
/// # Safety
/// Owner and outputs obey group_next's requirements. Budget is initialized,
/// live for this call, and disjoint from outputs; its value is never retained.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_group_observe(
    owner: *mut c_void,
    budget: *const DrWaitBudgetV1,
    key: *mut u64,
    index: *mut usize,
    status: *mut c_void,
) -> u8 {
    observe_group(owner, Some(*budget), key, index, status)
}

/// Shared projection for both indefinite and budgeted observation.
/// # Safety
/// The same live owner and disjoint output requirements as group_next apply.
pub(crate) unsafe fn observe_group(
    owner: *mut c_void,
    budget: Option<DrWaitBudgetV1>,
    key: *mut u64,
    index: *mut usize,
    status: *mut c_void,
) -> u8 {
    let owner = &mut *owner.cast::<GroupOwner>();
    owner.identity.clear();
    let group = &mut owner.group;
    let status = &mut *status.cast::<DrCompletionStatusV1>();
    *status = DrCompletionStatusV1::default();
    *key = 0;
    *index = 0;
    let observation = match budget {
        Some(mut budget) => group.observe(&mut budget),
        None => group.observe(&mut super::observation::Indefinite),
    };
    let completion = match observation {
        super::Observation::Completion(completion) => completion,
        super::Observation::NoCompletion => return 5,
        super::Observation::TimedOut => return 6,
        super::Observation::WaitFailed(failure) => return status.project_wait_failure(failure),
    };
    *key = completion.key;
    *index = completion.index;
    let (reading, length) = group.request_info(completion.index).unwrap();
    owner.identity.observe(completion.index, reading);
    NativeTransferSet::project_completion(
        status,
        completion.outcome.after_wait_failure_observation(),
        reading,
        length,
    )
}

/// Returns 0 with the last fetched key for the requested direction, or 1 when
/// absent. Metadata lookup never retires work, transfers ownership or allocates.
/// # Safety
/// Owner is live shared storage. Key is disjoint aligned writable OperationKey
/// storage; reading is 0 for writes or 1 for reads.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_group_observed(
    owner: *const c_void,
    reading: u8,
    key: *mut c_void,
) -> u8 {
    project_observed_operation(&(*owner.cast::<GroupOwner>()).identity, reading, key)
}

unsafe fn project_observed_operation(
    identity: &super::plan::GroupIdentity,
    reading: u8,
    key: *mut c_void,
) -> u8 {
    let key = &mut *key.cast::<super::plan::OperationKey>();
    *key = super::plan::OperationKey {
        owner: 0,
        position: 0,
    };
    let Some(observed) = identity.observed(reading != 0) else {
        return 1;
    };
    *key = observed;
    0
}

unsafe fn project_completed_region(
    region: Result<Option<&[u8]>, Failure>,
    data: *mut *const u8,
    length: *mut usize,
    status: *mut DrIoStatusV1,
) -> u8 {
    *data = ptr::null();
    *length = 0;
    *status = DrIoStatusV1::default();
    match region {
        Ok(Some(bytes)) => {
            *data = bytes.as_ptr();
            *length = bytes.len();
            0
        }
        Ok(None) => 1,
        Err(failure) => (*status).fail(failure),
    }
}

/// Returns 0 with a completed read prefix (including a present empty prefix),
/// 1 while pending or without known progress, or 3 for an invalid operation key.
/// Lookup performs no I/O, allocation, observation or loan release.
/// # Safety
/// Owner is live shared storage; key is aligned live OperationKey storage.
/// Outputs are disjoint writable storage, disjoint from inputs and the region.
/// The returned readonly region expires before the next group mutation/drop.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_group_read_region(
    owner: *const c_void,
    key: *const c_void,
    data: *mut *const u8,
    length: *mut usize,
    status: *mut DrIoStatusV1,
) -> u8 {
    let owner = &*owner.cast::<GroupOwner>();
    let key = *key.cast::<super::plan::OperationKey>();
    let region = owner
        .identity
        .operation_position(key, true, |index| owner.group.request_info(index))
        .map(|index| owner.group.completed(index));
    project_completed_region(region, data, length, status)
}

/// Resubmits the original directional operation without changing its identity.
/// Invalid/foreign keys are rejected before mutation or I/O.
/// # Safety
/// Owner is live exclusive storage with no borrowed completion/region view.
/// Key is aligned live OperationKey storage, disjoint from owner and status.
/// Reading is 0 for writes or 1 for reads; status is disjoint writable storage.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_group_submit_key(
    owner: *mut c_void,
    key: *const c_void,
    reading: u8,
    offset: usize,
    length: usize,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    if reading > 1 {
        return (*status).fail(Failure::invalid_input());
    }
    let owner = &mut *owner.cast::<GroupOwner>();
    let key = *key.cast::<super::plan::OperationKey>();
    let result = owner
        .identity
        .operation_position(key, reading != 0, |index| owner.group.request_info(index))
        .and_then(|index| owner.group.submit(index, offset, length));
    (*status).complete(result.map(|()| 0))
}

/// # Safety
/// Owner is live exclusively borrowed, and status is disjoint writable storage.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_group_submit(
    owner: *mut c_void,
    index: usize,
    offset: usize,
    length: usize,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    (*status).complete(
        (*owner.cast::<GroupOwner>())
            .group
            .submit(index, offset, length)
            .map(|()| 0),
    )
}

/// Returns a group-borrowed completed prefix, or writable idle read storage.
/// # Safety
/// Owner is live and exclusively borrowed. Outputs are disjoint writable
/// storage. The returned region is not retained past the next mutation/drop;
/// writable access cannot overlap a completion view or any active request.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_group_region(
    owner: *mut c_void,
    index: usize,
    writable: u8,
    data: *mut *mut u8,
    length: *mut usize,
) -> u8 {
    *data = ptr::null_mut();
    *length = 0;
    let group = &mut (*owner.cast::<GroupOwner>()).group;
    if writable == 1 {
        if let Some(bytes) = group.read_storage_mut(index) {
            *data = bytes.as_mut_ptr();
            *length = bytes.len();
            return 0;
        }
    } else if writable == 0 {
        if let Some(bytes) = group.completed(index) {
            *data = bytes.as_ptr().cast_mut();
            *length = bytes.len();
            return 0;
        }
    }
    1
}

/// # Safety
/// Owner is live exclusively borrowed; no returned region is being accessed.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_group_stop(owner: *mut c_void) -> u8 {
    match (*owner.cast::<GroupOwner>()).group.stop() {
        None => 0,
        Some(Stop::Cancellation) => 1,
        Some(Stop::BudgetExhausted) => 2,
        Some(Stop::Failure(_, _)) => 3,
    }
}

/// # Safety
/// Owner is null or a live owned group, dropped once after all region accesses.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_group_drop(owner: *mut c_void) {
    if !owner.is_null() {
        ptr::drop_in_place(owner.cast::<GroupOwner>());
        crate::dr_v1_class_free(owner.cast());
    }
}

/// # Safety
/// Owner is null or a live owned set with no active group, dropped once. This
/// closes streams only in the legacy fixed owning mode. Borrowed mode destroys
/// registration metadata without closing projected streams. The compiler drops
/// this native set before its complete retained stream owners/associations.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_transfer_set_drop(owner: *mut c_void) {
    if !owner.is_null() {
        if (*owner.cast::<SetOwner>()).active {
            crate::runtime_invariant_panic();
        }
        ptr::drop_in_place(owner.cast::<SetOwner>());
        crate::dr_v1_class_free(owner.cast());
    }
}
