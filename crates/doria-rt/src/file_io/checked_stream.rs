//! Checked native file operations over the single opaque runtime file owner.
//! Success borrows caller storage without allocating; failure construction uses
//! the same catalogue, strings, metadata, and progress rules as checked_io.
use core::{ffi::c_void, ptr};

use crate::checked_io::{
    fail_named_io, IoFailureOutputs, IoTransferOutputs as Outputs, OP_ACCESS, OP_CLOSE, OP_LOCK,
    OP_OPEN, OP_READ, OP_RESIZE, OP_SEEK, OP_SYNCHRONIZE_DATA, OP_SYNCHRONIZE_METADATA, OP_UNLOCK,
    OP_WRITE, TARGET_FILE,
};
use crate::{dr_v1_class_free, dr_v2_class_allocate, panic_catalogued, DrStackFrameV2, DrStringV1};

use super::stream::transport::object::{
    dr_v1_file_object_close, dr_v1_file_object_control_int, dr_v1_file_object_layout,
    dr_v1_file_object_open, dr_v1_file_object_path, dr_v1_file_object_read,
    dr_v1_file_object_write, file_object_target,
};
use super::stream::transport::DrFileStatusV1;

const OPEN: u8 = 0;
const READ: u8 = 1;
const WRITE: u8 = 2;
const CLOSE: u8 = 3;

/// Shared checked file transport for both native profiles.
///
/// Operation tags: 0 open, 1 read, 2 write, 3 consuming close. Returns 0 for
/// success or 1 for IoError. Open returns the owned payload address in value;
/// read tags are 0 Data, 1 WouldBlock, 2 EndOfStream; write tags are 0 Accepted,
/// 1 WouldBlock. Data/Accepted return the byte count in value. Empty requests
/// succeed with zero progress. Allocation failure remains a fatal panic.
/// A returned IoError carries one owned message reference and one retained
/// opening-path reference; the caller releases or transfers both. Metadata
/// and optional system-code/progress facts use checked_io's existing encoding.
///
/// Close clears the native obligation but leaves the payload/path initialized.
/// Generated consuming-method cleanup must subsequently run file-object drop
/// and common class free once, including after constructing a close IoError.
/// File failures never invoke the standard-output status-0 broken-pipe carve-out.
///
/// # Safety
/// The operation must be one of the private tags above. All outputs must be
/// writable, mutually disjoint, and disjoint from borrowed inputs, and contain
/// no unreleased owned references before the call. Frame must be null or a
/// valid generated frame chain. Open borrows a live path; owner
/// is ignored and returns a new owner only on success. Other operations borrow
/// an initialized open opaque file owner exclusively; path/options are ignored.
/// For read/write, data must be writable/readable for length bytes respectively
/// (null is allowed for zero length), disjoint from owner, and borrowed only
/// until return. Close requires independent ownership transferred into its
/// generated callee; neither another operation nor close may follow it.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_file_io(
    frame: *const DrStackFrameV2,
    operation: u8,
    path: *mut DrStringV1,
    owner: *mut c_void,
    access: u8,
    creation: u8,
    append: u8,
    data: *mut u8,
    length: usize,
    result_tag: *mut u64,
    result_value: *mut usize,
    message: *mut *mut DrStringV1,
    error_path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred: *mut usize,
    meta: *mut u64,
) -> u8 {
    let outputs = Outputs {
        tag: result_tag,
        value: result_value,
        failure: IoFailureOutputs {
            message,
            path: error_path,
            system_code,
            transferred_byte_count: transferred,
            meta,
        },
    };
    outputs.clear();
    if operation == OPEN {
        return open(frame, path, access, creation, append, outputs);
    }
    let io_operation = match operation {
        READ => OP_READ,
        WRITE => OP_WRITE,
        CLOSE => OP_CLOSE,
        _ => panic_catalogued(frame, b"P1001"),
    };
    let path = dr_v1_file_object_path(owner);
    let mut status = DrFileStatusV1::default();
    let result = match operation {
        READ => dr_v1_file_object_read(owner, data, length, &mut status),
        WRITE => dr_v1_file_object_write(owner, data, length, &mut status),
        CLOSE => dr_v1_file_object_close(owner, &mut status),
        _ => panic_catalogued(frame, b"P1001"),
    };
    let target = file_object_target(owner);
    outputs.finish(frame, io_operation, result, &status, |failure, outputs| {
        fail_named_io(frame, io_operation, target, failure, path, outputs)
    })
}

/// Checked synchronization, seek and readonly inspection.
/// Tags: 1 data sync, 2 full sync, 3 seek (origin 0 Start, 1 Current, 2 End),
/// 4 position, 5 length, 6 set length, 7 lock and 8 try lock (origin is the
/// mode: 0 Shared, 1 Exclusive; try lock's value is 1 acquired, 0 contended),
/// 9 unlock, 10 best-effort unlock. Synchronization/resizing have no public result.
/// Returns 0 success or 1 IoError with the retained opening path and no progress.
/// # Safety
/// The file owner is live and initialized. Seek/sync borrow exclusively;
/// inspection borrows shared. Inputs and disjoint writable outputs remain live for
/// the call; outputs contain no unreleased references. Frame is null or valid.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_file_control(
    frame: *const DrStackFrameV2,
    owner: *mut c_void,
    operation: u8,
    offset: i64,
    origin: u8,
    value: *mut i64,
    message: *mut *mut DrStringV1,
    error_path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred: *mut usize,
    meta: *mut u64,
) -> u8 {
    *value = 0;
    *message = ptr::null_mut();
    *error_path = ptr::null_mut();
    *system_code = 0;
    *transferred = 0;
    *meta = 0;
    let io_operation = match operation {
        1 => OP_SYNCHRONIZE_DATA,
        2 => OP_SYNCHRONIZE_METADATA,
        3 => OP_SEEK,
        4 | 5 => OP_ACCESS,
        6 => OP_RESIZE,
        7 | 8 => OP_LOCK,
        9 | 10 => OP_UNLOCK,
        11 => OP_CLOSE,
        _ => panic_catalogued(frame, b"P1001"),
    };
    let mut status = DrFileStatusV1::default();
    match dr_v1_file_object_control_int(owner, operation, offset, origin, &mut status) {
        0 => {
            *value = status.value as i64;
            0
        }
        3 => fail_status(
            frame,
            io_operation,
            file_object_target(owner),
            dr_v1_file_object_path(owner),
            &status,
            IoFailureOutputs {
                message,
                path: error_path,
                system_code,
                transferred_byte_count: transferred,
                meta,
            },
        ),
        _ => panic_catalogued(frame, b"P1001"),
    }
}

unsafe fn open(
    frame: *const DrStackFrameV2,
    path: *mut DrStringV1,
    access: u8,
    creation: u8,
    append: u8,
    outputs: Outputs,
) -> u8 {
    let mut size = 0;
    let mut alignment = 0;
    dr_v1_file_object_layout(&mut size, &mut alignment);
    let owner = dr_v2_class_allocate(frame, size, alignment).cast();
    let mut status = DrFileStatusV1::default();
    match dr_v1_file_object_open(path, access, creation, append, owner, &mut status) {
        0 => {
            *outputs.value = owner as usize;
            0
        }
        result => {
            // Failed open never initialized fields or retained the path. Free
            // only the payload; file-object drop would invent an obligation.
            dr_v1_class_free(owner.cast());
            match result {
                3 => fail_status(frame, OP_OPEN, TARGET_FILE, path, &status, outputs.failure),
                // OS path encoding allocation follows existing checked-file
                // intrinsics; class allocation already uses its own P1601.
                4 => panic_catalogued(frame, b"P1206"),
                _ => panic_catalogued(frame, b"P1001"),
            }
        }
    }
}

unsafe fn fail_status(
    frame: *const DrStackFrameV2,
    operation: u8,
    target: u8,
    path: *mut DrStringV1,
    status: &DrFileStatusV1,
    outputs: IoFailureOutputs,
) -> u8 {
    let failure = status
        .failure()
        .unwrap_or_else(|| panic_catalogued(frame, b"P1001"));
    fail_named_io(frame, operation, target, failure, path, outputs)
}

#[cfg(test)]
#[path = "checked_stream_tests.rs"]
mod tests;
