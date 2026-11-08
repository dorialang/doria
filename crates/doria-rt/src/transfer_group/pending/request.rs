//! Call-scoped views of group-owned pending records. No native request fields,
//! close claims, callback frames, or owned Error carriers live in these views.

use super::{
    decode_terminal, Configuration, DrAdapterOutcomeV1, DrIoStatusV1, DrPendingRegionV1,
    PendingState, Phase, Prepared,
};
use crate::checked_io::{fail_named_io, fail_standard_io, IoFailureOutputs, OP_READ, OP_WRITE};
use crate::native_handle::PhysicalOrigin;
use crate::platform_io::Failure;
use crate::transfer_group::adapter::AdapterTerminal;
use crate::{DrInterfaceCarrierV1, DrStackFrameV2, DrStringV1};
use core::{ffi::c_void, ptr};

const PREPARATION: u8 = 0;
const COMPLETION: u8 = 1;

/// Runtime-created and borrowed only for the current callback. Backends pass
/// its opaque address and never construct, copy, destroy, or inspect this layout.
#[repr(C)]
pub(super) struct DrPendingRequestViewV1 {
    pub(super) record: *mut PendingState,
    // Borrowed facts in the stable request, never callback-stack output slots.
    native: *const DrIoStatusV1,
    immediate: *const DrAdapterOutcomeV1,
    phase: u8,
    pub(super) native_tag: u8,
    pub(super) submitted: u8,
}

impl DrPendingRequestViewV1 {
    pub(super) fn preparation(record: *mut PendingState) -> Self {
        Self {
            record,
            native: ptr::null(),
            immediate: ptr::null(),
            phase: PREPARATION,
            native_tag: 4,
            submitted: 0,
        }
    }

    /// # Safety
    /// Record is stable and exclusively borrowed through this completion call.
    /// Its Finalizing phase retains the actual retired/immediate facts.
    pub(super) unsafe fn completion(record: *mut PendingState) -> Self {
        let Phase::Finalizing(facts) = &(*record).phase else {
            crate::runtime_invariant_panic();
        };
        Self {
            record,
            native: ptr::from_ref(&facts.native.1),
            immediate: facts.immediate.as_ref().map_or(ptr::null(), ptr::from_ref),
            phase: COMPLETION,
            native_tag: facts.native.0,
            submitted: u8::from(facts.submitted),
        }
    }

    /// Decode an ordinary preparation reply independently of how the adapter
    /// was invoked. Configuration and the original owned Error remain in the
    /// shared request protocol; a host does not reconstruct either contract.
    /// # Safety
    /// The view and exclusively borrowed record are live for this preparation.
    /// Error is empty or the reply's owned carrier, transferred on failure.
    pub(super) unsafe fn prepared_reply(
        &self,
        status: u8,
        outcome: DrAdapterOutcomeV1,
        error: DrInterfaceCarrierV1,
    ) -> Prepared {
        let record = &*self.record;
        if self.phase != PREPARATION || !matches!(record.phase, Phase::Preparing) {
            crate::runtime_invariant_panic();
        }
        if status != 0 {
            return Prepared::Failed(decode_terminal(
                status,
                outcome,
                error,
                record.reading,
                record.bound,
            ));
        }
        if !error.object.is_null() || !error.vtable.is_null() {
            crate::runtime_invariant_panic();
        }
        if record.invalid_configuration {
            return Prepared::Invalid;
        }
        match record.configuration {
            Some(Configuration::Native(region)) => Prepared::Native(region),
            Some(Configuration::Immediate(outcome)) => Prepared::Immediate(outcome),
            None => Prepared::Invalid,
        }
    }

    /// # Safety
    /// The view and exclusively borrowed record are live for this completion.
    /// Error is empty or the reply's owned carrier, transferred on failure.
    pub(super) unsafe fn completed_reply(
        &self,
        status: u8,
        outcome: DrAdapterOutcomeV1,
        error: DrInterfaceCarrierV1,
    ) -> AdapterTerminal {
        let record = &*self.record;
        if self.phase != COMPLETION || !matches!(record.phase, Phase::Finalizing(_)) {
            crate::runtime_invariant_panic();
        }
        let has_outcome = self.native_tag != 4 || !self.immediate.is_null();
        if status == 4 || (!has_outcome && status == 0) {
            if !error.object.is_null() || !error.vtable.is_null() {
                crate::runtime_invariant_panic();
            }
            return if has_outcome {
                AdapterTerminal::PlatformFailure(Failure::invalid_input())
            } else {
                // Cleanup has no outcome to decode. It neither requires nor
                // reads an invented success count from the reply's output.
                AdapterTerminal::NoTransfer
            };
        }
        decode_terminal(status, outcome, error, record.reading, record.bound)
    }

    unsafe fn can_configure(&self) -> bool {
        let record = &mut *self.record;
        if self.phase != PREPARATION
            || !matches!(record.phase, Phase::Preparing)
            || record.configuration.is_some()
            || record.invalid_configuration
        {
            // Keep the FIRST selection for exact-once cleanup. A void public
            // complete call cannot manufacture a checked exception or panic.
            record.invalid_configuration = true;
            return false;
        }
        true
    }

    unsafe fn refresh_source(
        &self,
        frame: *const DrStackFrameV2,
        fresh_source: *const c_void,
    ) -> Result<(), AdapterTerminal> {
        if !self.can_configure() {
            return Err(AdapterTerminal::PlatformFailure(Failure::invalid_input()));
        }
        let selector = (*self.record)
            .configurator
            .ok_or_else(|| AdapterTerminal::PlatformFailure(Failure::invalid_input()))?;
        let source = (selector.resolve)(
            selector.driver,
            selector.index,
            selector.request,
            frame,
            (!fresh_source.is_null()).then_some(fresh_source),
        )?
        .ok_or_else(|| AdapterTerminal::PlatformFailure(Failure::invalid_input()))?;
        // Completion retains the adapter dispatch used to prepare. Only the
        // source selection changes at this nested configuration boundary.
        (*self.record).origin = Some(source.origin);
        Ok(())
    }
}

/// Checked source-following configuration. Native bindings rerun their getter;
/// External bindings require a fresh typed source. 0 succeeds, 1 moves the
/// original getter Error, and 3 carries canonical Configure/File IoError fields.
/// The void File helper below is only the fixed-public-owner contract; raw
/// owner fixtures select their retained native endpoint through configure_native.
/// # Safety
/// View is this call's preparation view. File is null for rejection or a live
/// compiler File payload; all outputs are disjoint writable EMPTY storage.
/// Data is a retained group/complete-owner loan. Frame is call-only.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_pending_request_configure_file(
    frame: *const DrStackFrameV2,
    view: *mut c_void,
    file: *mut c_void,
    fresh_source: *const c_void,
    data: *mut u8,
    length: usize,
    error: *mut DrInterfaceCarrierV1,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    initialize_configuration_outputs(
        error,
        message,
        path,
        system_code,
        transferred_byte_count,
        meta,
    );
    let outputs = IoFailureOutputs {
        message,
        path,
        system_code,
        transferred_byte_count,
        meta,
    };
    let file_path = if file.is_null() {
        ptr::null_mut()
    } else {
        crate::file_io::stream::transport::object::dr_v1_file_object_path(file)
    };
    let result = (|| {
        if view.is_null() || file.is_null() {
            return Err(AdapterTerminal::PlatformFailure(Failure::invalid_input()));
        }
        let request = &*view.cast::<DrPendingRequestViewV1>();
        request.refresh_source(frame, fresh_source)?;
        dr_v1_pending_request_configure_file(view, file, data, length);
        if (*request.record).invalid_configuration {
            return Err(AdapterTerminal::PlatformFailure(Failure::invalid_input()));
        }
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(AdapterTerminal::CheckedError { error: owned, .. }) => {
            *error = owned.into_carrier();
            1
        }
        Err(AdapterTerminal::PlatformFailure(failure)) => {
            let operation = doria_diagnostic_catalogue::IoMessageOperation::Configure as u8;
            // A child pipe's failure names the pipe, as its transfers do.
            let target = if file.is_null() {
                crate::checked_io::TARGET_FILE
            } else {
                crate::file_io::stream::transport::object::file_object_target(file)
            };
            fail_named_io(frame, operation, target, failure, file_path, outputs);
            3
        }
        Err(_) => crate::runtime_invariant_panic(),
    }
}

/// Same source-following checked transport, with canonical standard-device
/// target facts rather than a File/path reconstruction.
/// # Safety
/// Same call-only view, retained region and empty output contracts as File.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_pending_request_configure_standard(
    frame: *const DrStackFrameV2,
    view: *mut c_void,
    stream: u8,
    fresh_source: *const c_void,
    data: *mut u8,
    length: usize,
    error: *mut DrInterfaceCarrierV1,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    initialize_configuration_outputs(
        error,
        message,
        path,
        system_code,
        transferred_byte_count,
        meta,
    );
    let outputs = IoFailureOutputs {
        message,
        path,
        system_code,
        transferred_byte_count,
        meta,
    };
    let standard = match stream {
        0 => crate::device_io::StandardStream::Stdin,
        1 => crate::device_io::StandardStream::Stdout,
        2 => crate::device_io::StandardStream::Stderr,
        _ => crate::runtime_invariant_panic(),
    };
    let result = (|| {
        if view.is_null() {
            return Err(AdapterTerminal::PlatformFailure(Failure::invalid_input()));
        }
        let request = &*view.cast::<DrPendingRequestViewV1>();
        request.refresh_source(frame, fresh_source)?;
        dr_v1_pending_request_configure_standard(view, stream, data, length);
        if (*request.record).invalid_configuration {
            return Err(AdapterTerminal::PlatformFailure(Failure::invalid_input()));
        }
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(AdapterTerminal::CheckedError { error: owned, .. }) => {
            *error = owned.into_carrier();
            1
        }
        Err(AdapterTerminal::PlatformFailure(failure)) => {
            fail_standard_io(
                frame,
                doria_diagnostic_catalogue::IoMessageOperation::Configure as u8,
                standard,
                failure,
                outputs,
            );
            3
        }
        Err(_) => crate::runtime_invariant_panic(),
    }
}

unsafe fn initialize_configuration_outputs(
    error: *mut DrInterfaceCarrierV1,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) {
    if error.is_null() || !(*error).object.is_null() || !(*error).vtable.is_null() {
        crate::runtime_invariant_panic();
    }
    *message = ptr::null_mut();
    *path = ptr::null_mut();
    *system_code = 0;
    *transferred_byte_count = 0;
    *meta = 0;
}

/// Preserve the existing event-ledger fixtures without exposing view layout
/// to production compiler code. Returned facts are borrowed for this call only.
#[cfg(test)]
pub(crate) unsafe fn pending_callback_facts(
    view: *mut c_void,
) -> (
    u8,
    u8,
    *const DrIoStatusV1,
    *const DrPendingRegionV1,
    *const DrAdapterOutcomeV1,
) {
    let view = &*view.cast::<DrPendingRequestViewV1>();
    (
        view.native_tag,
        view.submitted,
        view.native,
        ptr::addr_of!((*view.record).native_region),
        view.immediate,
    )
}

/// # Safety
/// View is the live exclusive preparation callback's opaque request view.
/// Endpoint/storage are compiler-retained loans through completion. This only
/// records selection; the existing driver validates leaf identity before I/O.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_pending_request_configure_native(
    view: *mut c_void,
    endpoint: *const c_void,
    data: *mut u8,
    length: usize,
) {
    let view = &*view.cast::<DrPendingRequestViewV1>();
    if !view.can_configure() {
        return;
    }
    let record = &mut *view.record;
    record.configuration = Some(Configuration::Native(DrPendingRegionV1 {
        endpoint,
        data,
        length,
    }));
    if endpoint.is_null()
        || (length != 0 && data.is_null())
        || length > isize::MAX as usize
        || (data as usize).checked_add(length).is_none()
    {
        record.invalid_configuration = true;
    }
}

/// # Safety
/// View is live for the current preparation call. File is its registered live
/// complete file payload, exclusively retained through retirement. Region is
/// a group/complete-owner loan; local temporary storage may not be selected.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_pending_request_configure_file(
    view: *mut c_void,
    file: *mut c_void,
    data: *mut u8,
    length: usize,
) {
    let request = &*view.cast::<DrPendingRequestViewV1>();
    if !request.can_configure() {
        return;
    }
    let record = &mut *request.record;
    if record
        .origin
        .and_then(PhysicalOrigin::file)
        .is_none_or(|(_, origin)| origin.resource.is_null() || origin.payload != file)
    {
        // This private fixed-owner helper uses its captured physical origin.
        // Bound callers refresh/select through the checked helper first. A raw
        // owner can share its first field's address, but that equality proves
        // no FileObject layout. Require resource-backed File provenance and
        // reject raw fixture records or foreign payloads before interpretation.
        record.invalid_configuration = true;
        return;
    }
    let Ok((owner, _)) = crate::file_io::stream::transport::object::pending_projection(file) else {
        record.invalid_configuration = true;
        return;
    };
    dr_v1_pending_request_configure_native(view, owner.cast(), data, length);
}

/// Select the registered canonical standard device, never a copied handle or
/// substitute File. Its group already holds the device loan before preparation.
/// Queued input/observed EOF becomes an immediate outcome through the existing
/// complete adapter protocol; a probe never consumes the queue.
/// # Safety
/// View is this preparation callback's request, and the selected region is a
/// retained group/adapter loan. No other device consumer may run in this loan.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_pending_request_configure_standard(
    view: *mut c_void,
    stream: u8,
    data: *mut u8,
    length: usize,
) {
    let request = &*view.cast::<DrPendingRequestViewV1>();
    if !request.can_configure() {
        return;
    }
    let record = &mut *request.record;
    let Some(PhysicalOrigin::Standard(registered)) = record.origin else {
        record.invalid_configuration = true;
        return;
    };
    if registered as u8 != stream || (record.reading != (stream == 0)) {
        record.invalid_configuration = true;
        return;
    }
    dr_v1_pending_request_configure_native(
        view,
        core::ptr::from_ref(crate::device_io::standard_device(registered)).cast(),
        data,
        length,
    );
}

/// # Safety
/// View is a live request view; tag/count are the canonical compiler-projected
/// actual outcome, never null. Configuration is preparation-only. Invalid or
/// repeated calls latch checked setup rejection, without overwriting staging.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_pending_request_complete(view: *mut c_void, tag: u8, count: usize) {
    let view = &*view.cast::<DrPendingRequestViewV1>();
    if !view.can_configure() {
        return;
    }
    let record = &mut *view.record;
    let valid = match tag {
        0 => count <= record.bound,
        1 => count == 0,
        2 | 3 => record.reading && count == 0,
        _ => false,
    };
    if !valid {
        record.invalid_configuration = true;
        return;
    }
    record.configuration = Some(Configuration::Immediate(DrAdapterOutcomeV1 {
        count,
        tag,
        has_count: u8::from(tag == 0),
    }));
}

/// Same checked-I/O output convention as checked_file_io. Returns 0 PRESENT,
/// 4 ABSENT, or 1 canonical IoError fields. No transfer, wait or retry occurs.
/// Immediate presence wins over submitted=0; tag 4 is absence, not Data(0).
/// Original adapter Errors never enter this projection: callback transport
/// moves their existing owned interface carrier unchanged.
/// # Safety
/// View is live for this callback. All outputs are disjoint writable EMPTY
/// locations, disjoint from the view and retained roots. Frame is current/null.
/// Checked observation requires typed-source/file-object registration with a real retained origin;
/// raw private native fixtures are not a public checked-I/O admission path.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_pending_request_outcome(
    frame: *const DrStackFrameV2,
    view: *const c_void,
    tag: *mut u64,
    count: *mut usize,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred: *mut usize,
    meta: *mut u64,
) -> u8 {
    *tag = 0;
    *count = 0;
    *message = ptr::null_mut();
    *path = ptr::null_mut();
    *system_code = 0;
    *transferred = 0;
    *meta = 0;
    let view = &*view.cast::<DrPendingRequestViewV1>();
    let record = &*view.record;
    let failure = if view.phase != COMPLETION || !matches!(record.phase, Phase::Finalizing(_)) {
        Failure::invalid_input()
    } else if let Some(immediate) = view.immediate.as_ref() {
        *tag = u64::from(immediate.tag);
        *count = immediate.count;
        return 0;
    } else {
        match view.native_tag {
            4 => return 4,
            0 => {
                *count = usize::try_from((*view.native).value)
                    .unwrap_or_else(|_| crate::runtime_invariant_panic());
                return 0;
            }
            1 => {
                *tag = 1;
                return 0;
            }
            2 if record.reading => {
                *tag = 2;
                return 0;
            }
            3 => (*view.native)
                .failure()
                .unwrap_or_else(|| crate::runtime_invariant_panic()),
            _ => crate::runtime_invariant_panic(),
        }
    };
    // Missing origin is a violated PRIVATE bridge precondition, never a
    // substituted public target/path. Typed-source admission seeds it before
    // preparation, so wrong-phase user access produces checked InvalidInput.
    let origin = record
        .origin
        .unwrap_or_else(|| crate::runtime_invariant_panic());
    let operation = if record.reading { OP_READ } else { OP_WRITE };
    let outputs = IoFailureOutputs {
        message,
        path,
        system_code,
        transferred_byte_count: transferred,
        meta,
    };
    match origin {
        PhysicalOrigin::File { origin, .. } => fail_named_io(
            frame,
            operation,
            origin.error_target(),
            failure,
            origin.path,
            outputs,
        ),
        PhysicalOrigin::Standard(stream) => {
            fail_standard_io(frame, operation, stream, failure, outputs)
        }
    }
}
