use core::ptr;

use doria_diagnostic_catalogue::{
    visit_invalid_utf8_message_parts, visit_io_error_message_parts, IoMessageOperation,
    IoMessageReason, IoMessageTarget, Utf8MessageSource,
};

use crate::device_io::{self, StandardStream, WriteOutcome};
use crate::io_status::DrIoStatusV1;
use crate::platform_io::{Failure, Reason};
use crate::{
    allocate, allocate_string_with_frame, bytes, deallocate, exit_process, file_io, line_io,
    panic_catalogued, string_bytes, string_bytes_mut, DrStackFrameV2, DrStringV1,
};

pub const READ_LINE: u8 = 0;
pub const READ_FILE_TEXT: u8 = 1;
pub const READ_FILE_BYTES: u8 = 2;
pub const READ_STDIN_BYTES: u8 = 3;
pub const WRITE_FILE: u8 = 4;
pub const APPEND_FILE: u8 = 5;
pub const WRITE_STDOUT: u8 = 6;
pub const WRITE_STDERR: u8 = 7;
pub const READINESS_SET_NEW: u8 = 8;

pub(crate) const OP_OPEN: u8 = IoMessageOperation::Open as u8;
pub(crate) const OP_READ: u8 = IoMessageOperation::Read as u8;
pub(crate) const OP_WRITE: u8 = IoMessageOperation::Write as u8;
const OP_APPEND: u8 = IoMessageOperation::Append as u8;
const OP_FLUSH: u8 = IoMessageOperation::Flush as u8;
pub(crate) const OP_CLOSE: u8 = IoMessageOperation::Close as u8;
const OP_REGISTER: u8 = IoMessageOperation::Register as u8;
const OP_UNREGISTER: u8 = IoMessageOperation::Unregister as u8;
pub(crate) const OP_WAIT: u8 = IoMessageOperation::Wait as u8;
const OP_CONFIGURE: u8 = IoMessageOperation::Configure as u8;
pub(crate) const OP_ACCESS: u8 = IoMessageOperation::Access as u8;
pub(crate) const OP_SEEK: u8 = IoMessageOperation::Seek as u8;
pub(crate) const OP_SYNCHRONIZE_DATA: u8 = IoMessageOperation::SynchronizeData as u8;
pub(crate) const OP_SYNCHRONIZE_METADATA: u8 = IoMessageOperation::SynchronizeMetadata as u8;
pub(crate) const OP_RESIZE: u8 = IoMessageOperation::Resize as u8;
pub(crate) const OP_LOCK: u8 = IoMessageOperation::Lock as u8;
pub(crate) const OP_UNLOCK: u8 = IoMessageOperation::Unlock as u8;
pub(crate) const OP_SPAWN: u8 = IoMessageOperation::Spawn as u8;
pub(crate) const OP_TERMINATE: u8 = IoMessageOperation::Terminate as u8;

pub(crate) const TARGET_FILE: u8 = 0;
const TARGET_STDIN: u8 = 1;
const TARGET_STDOUT: u8 = 2;
const TARGET_STDERR: u8 = 3;
const TARGET_READINESS_SET: u8 = 4;
pub(crate) const TARGET_TRANSFER_PLAN: u8 = 5;
pub(crate) const TARGET_TRANSFER_GROUP: u8 = 6;
pub(crate) const TARGET_PROCESS: u8 = 7;
pub(crate) const TARGET_PROCESS_INPUT: u8 = 8;
pub(crate) const TARGET_PROCESS_OUTPUT: u8 = 9;
pub(crate) const TARGET_PROCESS_ERRORS: u8 = 10;

const ERROR_IO: u8 = 0;
const ERROR_INVALID_UTF8: u8 = 1;

const META_OPERATION_SHIFT: u32 = 0;
const META_TARGET_SHIFT: u32 = 8;
const META_REASON_SHIFT: u32 = 16;
const META_KIND_SHIFT: u32 = 24;
const META_HAS_SYSTEM_CODE_SHIFT: u32 = 32;
const META_HAS_INVALID_COUNT_SHIFT: u32 = 33;
const META_HAS_TRANSFERRED_COUNT_SHIFT: u32 = 34;

struct FailureOutputs {
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    valid_byte_count: *mut usize,
    invalid_byte_count: *mut usize,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
}

/// The existing IoError fields, shared by checked intrinsic and file transport.
/// These are private ABI output locations, not another diagnostic/error model.
#[derive(Clone, Copy)]
pub(crate) struct IoFailureOutputs {
    pub(crate) message: *mut *mut DrStringV1,
    pub(crate) path: *mut *mut DrStringV1,
    pub(crate) system_code: *mut i64,
    pub(crate) transferred_byte_count: *mut usize,
    pub(crate) meta: *mut u64,
}

impl IoFailureOutputs {
    pub(crate) unsafe fn clear(&self) {
        *self.message = ptr::null_mut();
        *self.path = ptr::null_mut();
        *self.system_code = 0;
        *self.transferred_byte_count = 0;
        *self.meta = 0;
    }
}

pub(crate) struct IoTransferOutputs {
    pub(crate) tag: *mut u64,
    pub(crate) value: *mut usize,
    pub(crate) failure: IoFailureOutputs,
}

impl IoTransferOutputs {
    pub(crate) unsafe fn clear(&self) {
        *self.tag = 0;
        *self.value = 0;
        self.failure.clear();
    }

    /// File and standard-device primitives share one outcome/progress decoder.
    /// Only an actual failure invokes the target-specific error constructor.
    pub(crate) unsafe fn finish(
        self,
        frame: *const DrStackFrameV2,
        operation: u8,
        result: u8,
        status: &DrIoStatusV1,
        fail: impl FnOnce(Failure, IoFailureOutputs) -> u8,
    ) -> u8 {
        match result {
            0 => {
                *self.value = usize::try_from(status.value)
                    .unwrap_or_else(|_| panic_catalogued(frame, b"P1001"));
                0
            }
            1 if operation != OP_CLOSE => {
                *self.tag = 1;
                0
            }
            2 if operation == OP_READ => {
                *self.tag = 2;
                0
            }
            3 => fail(
                status
                    .failure()
                    .unwrap_or_else(|| panic_catalogued(frame, b"P1001")),
                self.failure,
            ),
            _ => panic_catalogued(frame, b"P1001"),
        }
    }
}

/// Projects an actual readiness infrastructure failure through the existing
/// IoError field transport on every platform. Creation/admission uses Register,
/// removal uses Unregister, borrowing uses Access, and clock/wakeup/wait
/// failures use Wait.
///
/// An adapter's owned checked Error never crosses this boundary: it moves
/// through the existing error carrier unchanged. Infrastructure failures have
/// no byte-transfer progress; a supplied progress fact violates the private ABI
/// rather than being silently discarded or reconstructed as another error.
///
/// Returns the ordinary checked-failure tag 1. Allocation failure remains fatal.
/// # Safety
/// Status is a live actual failed record, not a zeroed success/outcome record.
/// Operation is Register, Unregister, Access, or Wait. Frame is null or live for this
/// call. All outputs are disjoint writable empty slots, disjoint from status.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_readiness_failure(
    frame: *const DrStackFrameV2,
    operation: u8,
    status: *const DrIoStatusV1,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    if !matches!(operation, OP_REGISTER | OP_UNREGISTER | OP_WAIT | OP_ACCESS) {
        crate::runtime_invariant_panic();
    }
    fail_nontransfer_io(
        frame,
        operation,
        TARGET_READINESS_SET,
        status,
        IoFailureOutputs {
            message,
            path,
            system_code,
            transferred_byte_count,
            meta,
        },
    )
}

/// Projects an actual transfer-plan configuration failure through the existing
/// IoError fields. No I/O has begun: the target has no path or byte progress.
/// System-code presence is retained independently from its numeric value.
/// Returns ordinary checked-failure tag 1; message allocation remains fatal.
/// # Safety
/// Status is a live actual failed record. Frame is null or live for this call.
/// All outputs are disjoint writable empty slots, disjoint from status.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_transfer_plan_failure(
    frame: *const DrStackFrameV2,
    status: *const DrIoStatusV1,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    fail_nontransfer_io(
        frame,
        OP_CONFIGURE,
        TARGET_TRANSFER_PLAN,
        status,
        IoFailureOutputs {
            message,
            path,
            system_code,
            transferred_byte_count,
            meta,
        },
    )
}

// Non-transfer infrastructure has one projection, including the invariant
// checks and absence bits. Never discard a supplied progress fact or fabricate
// a path; preserve the same IoError transport used by actual device failures.
pub(crate) unsafe fn fail_nontransfer_io(
    frame: *const DrStackFrameV2,
    operation: u8,
    target: u8,
    status: *const DrIoStatusV1,
    outputs: IoFailureOutputs,
) -> u8 {
    let failure = (*status)
        .failure()
        .unwrap_or_else(|| crate::runtime_invariant_panic());
    if failure.transferred_byte_count.is_some() {
        crate::runtime_invariant_panic();
    }
    *outputs.message = ptr::null_mut();
    *outputs.path = ptr::null_mut();
    *outputs.system_code = 0;
    *outputs.transferred_byte_count = 0;
    *outputs.meta = 0;
    fail_io_fields(frame, operation, target, failure, ptr::null_mut(), outputs)
}

/// Shared checked-I/O transport used by both native code generators.
///
/// Returns 0 for success and 1 for an ordinary checked failure. Allocation
/// failures remain fatal runtime panics. Closed ordinary output pipes retain
/// Doria's permanent status-0 process exit and therefore do not return.
///
/// # Safety
/// All pointers must satisfy the operation-specific contract. Every out pointer
/// must be writable. `path_or_prompt` and `contents` are borrowed for this call.
#[no_mangle]
pub unsafe extern "C" fn dr_v4_checked_io(
    current_frame: *const DrStackFrameV2,
    operation: u8,
    path_or_prompt: *mut DrStringV1,
    contents: *const u8,
    contents_length: usize,
    result: *mut *mut core::ffi::c_void,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    valid_byte_count: *mut usize,
    invalid_byte_count: *mut usize,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    *result = ptr::null_mut();
    *message = ptr::null_mut();
    *path = ptr::null_mut();
    *system_code = 0;
    *valid_byte_count = 0;
    *invalid_byte_count = 0;
    *transferred_byte_count = 0;
    *meta = 0;
    let outputs = FailureOutputs {
        message,
        path,
        system_code,
        valid_byte_count,
        invalid_byte_count,
        transferred_byte_count,
        meta,
    };
    match operation {
        READ_LINE => read_line(current_frame, path_or_prompt, result, outputs),
        READ_FILE_TEXT | READ_FILE_BYTES => {
            read_file(current_frame, operation, path_or_prompt, result, outputs)
        }
        READ_STDIN_BYTES => read_stdin_bytes(current_frame, result, outputs),
        WRITE_FILE | APPEND_FILE => write_file(
            current_frame,
            operation,
            path_or_prompt,
            contents,
            contents_length,
            outputs,
        ),
        WRITE_STDOUT | WRITE_STDERR => {
            write_stream(current_frame, operation, contents, contents_length, outputs)
        }
        READINESS_SET_NEW => {
            #[cfg(any(target_os = "linux", target_os = "macos", windows))]
            {
                crate::transfer_group::transport::dr_v1_checked_readiness_set_new(
                    current_frame,
                    result,
                    message,
                    path,
                    system_code,
                    transferred_byte_count,
                    meta,
                )
            }
            #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
            {
                fail_io(
                    current_frame,
                    OP_REGISTER,
                    TARGET_READINESS_SET,
                    Failure::unsupported(),
                    ptr::null_mut(),
                    outputs,
                )
            }
        }
        _ => fail_io(
            current_frame,
            OP_READ,
            TARGET_STDIN,
            Failure::invalid_input(),
            ptr::null_mut(),
            outputs,
        ),
    }
}

unsafe fn read_line(
    frame: *const DrStackFrameV2,
    prompt: *mut DrStringV1,
    result: *mut *mut core::ffi::c_void,
    outputs: FailureOutputs,
) -> u8 {
    let length = if prompt.is_null() {
        0
    } else {
        (*prompt).byte_length
    };
    if length != 0 {
        match device_io::write(StandardStream::Stdout, string_bytes(prompt), length) {
            WriteOutcome::Success => {}
            WriteOutcome::BrokenPipe => exit_process(0),
            WriteOutcome::OtherFailure(failure) => {
                return fail_io(
                    frame,
                    OP_WRITE,
                    TARGET_STDOUT,
                    failure,
                    ptr::null_mut(),
                    outputs,
                )
            }
        }
    }
    match device_io::flush(StandardStream::Stdout) {
        WriteOutcome::Success => {}
        WriteOutcome::BrokenPipe => exit_process(0),
        WriteOutcome::OtherFailure(failure) => {
            return fail_io(
                frame,
                OP_FLUSH,
                TARGET_STDOUT,
                failure,
                ptr::null_mut(),
                outputs,
            )
        }
    }
    match line_io::read_line() {
        Ok(line) => {
            *result = match line.bytes {
                Some((data, length)) => crate::dr_v1_string_from_utf8(data, length).cast(),
                None => ptr::null_mut(),
            };
            0
        }
        Err(line_io::ReadLineError::Read(failure)) => fail_io(
            frame,
            OP_READ,
            TARGET_STDIN,
            failure,
            ptr::null_mut(),
            outputs,
        ),
        Err(line_io::ReadLineError::InvalidUtf8 {
            valid_byte_count,
            invalid_byte_count,
        }) => fail_utf8(
            frame,
            TARGET_STDIN,
            ptr::null_mut(),
            valid_byte_count,
            invalid_byte_count,
            outputs,
        ),
        Err(line_io::ReadLineError::Allocation) => panic_catalogued(frame, b"P1206"),
    }
}

unsafe fn read_file(
    frame: *const DrStackFrameV2,
    operation: u8,
    path: *mut DrStringV1,
    result: *mut *mut core::ffi::c_void,
    outputs: FailureOutputs,
) -> u8 {
    let path_bytes = core::slice::from_raw_parts(string_bytes(path), (*path).byte_length);
    match file_io::read_file(path_bytes) {
        Ok(contents) if operation == READ_FILE_BYTES => {
            let (data, length) = contents.into_raw_parts();
            *result = bytes::from_owned(data, length).cast();
            0
        }
        Ok(contents) => {
            let data = core::slice::from_raw_parts(contents.bytes, contents.length);
            match core::str::from_utf8(data) {
                Ok(_) => {
                    *result = crate::dr_v1_string_from_utf8(data.as_ptr(), data.len()).cast();
                    0
                }
                Err(error) => fail_utf8(
                    frame,
                    TARGET_FILE,
                    path,
                    error.valid_up_to(),
                    error.error_len(),
                    outputs,
                ),
            }
        }
        Err(file_io::FileError::Allocation) => panic_catalogued(
            frame,
            if operation == READ_FILE_BYTES {
                b"P1302"
            } else {
                b"P1206"
            },
        ),
        Err(file_io::FileError::PathNul) => fail_io(
            frame,
            OP_OPEN,
            TARGET_FILE,
            Failure::invalid_input(),
            path,
            outputs,
        ),
        Err(file_io::FileError::Open(failure)) => {
            fail_io(frame, OP_OPEN, TARGET_FILE, failure, path, outputs)
        }
        Err(file_io::FileError::Read(failure)) => {
            fail_io(frame, OP_READ, TARGET_FILE, failure, path, outputs)
        }
        Err(file_io::FileError::Write(failure)) => {
            fail_io(frame, OP_READ, TARGET_FILE, failure, path, outputs)
        }
    }
}

unsafe fn read_stdin_bytes(
    frame: *const DrStackFrameV2,
    result: *mut *mut core::ffi::c_void,
    outputs: FailureOutputs,
) -> u8 {
    let (loan, buffered) = match line_io::take_buffered_input() {
        Ok(input) => input,
        Err(failure) => {
            return fail_io(
                frame,
                OP_READ,
                TARGET_STDIN,
                failure,
                ptr::null_mut(),
                outputs,
            )
        }
    };
    let mut capacity = 4096_usize;
    while capacity < buffered.length {
        capacity = capacity
            .checked_mul(2)
            .unwrap_or_else(|| panic_catalogued(frame, b"P1302"));
    }
    let mut data = allocate(capacity);
    if data.is_null() {
        panic_catalogued(frame, b"P1302");
    }
    if buffered.length != 0 {
        ptr::copy_nonoverlapping(buffered.bytes, data, buffered.length);
    }
    let mut length = buffered.length;
    if buffered.eof {
        *result = bytes::from_owned(data, length).cast();
        return 0;
    }
    loop {
        if length == capacity {
            let next = capacity
                .checked_mul(2)
                .unwrap_or_else(|| panic_catalogued(frame, b"P1302"));
            let replacement = allocate(next);
            if replacement.is_null() {
                deallocate(data);
                panic_catalogued(frame, b"P1302");
            }
            ptr::copy_nonoverlapping(data, replacement, length);
            deallocate(data);
            data = replacement;
            capacity = next;
        }
        match loan.read_bytes(data.add(length), capacity - length) {
            Ok(0) => {
                *result = bytes::from_owned(data, length).cast();
                return 0;
            }
            Ok(read) => length += read,
            Err(failure) => {
                deallocate(data);
                return fail_io(
                    frame,
                    OP_READ,
                    TARGET_STDIN,
                    failure.after_transferred(length),
                    ptr::null_mut(),
                    outputs,
                );
            }
        }
    }
}

unsafe fn write_file(
    frame: *const DrStackFrameV2,
    operation: u8,
    path: *mut DrStringV1,
    contents: *const u8,
    contents_length: usize,
    outputs: FailureOutputs,
) -> u8 {
    let path_bytes = core::slice::from_raw_parts(string_bytes(path), (*path).byte_length);
    let contents = core::slice::from_raw_parts(contents, contents_length);
    let result = if operation == APPEND_FILE {
        file_io::append_file(path_bytes, contents)
    } else {
        file_io::write_file(path_bytes, contents)
    };
    match result {
        Ok(()) => 0,
        Err(file_io::FileError::Allocation) => panic_catalogued(frame, b"P1206"),
        Err(file_io::FileError::PathNul) => fail_io(
            frame,
            if operation == APPEND_FILE {
                OP_APPEND
            } else {
                OP_WRITE
            },
            TARGET_FILE,
            Failure::invalid_input(),
            path,
            outputs,
        ),
        Err(file_io::FileError::Open(failure)) => {
            fail_io(frame, OP_OPEN, TARGET_FILE, failure, path, outputs)
        }
        Err(file_io::FileError::Write(failure)) => fail_io(
            frame,
            if operation == APPEND_FILE {
                OP_APPEND
            } else {
                OP_WRITE
            },
            TARGET_FILE,
            failure,
            path,
            outputs,
        ),
        Err(file_io::FileError::Read(failure)) => fail_io(
            frame,
            if operation == APPEND_FILE {
                OP_APPEND
            } else {
                OP_WRITE
            },
            TARGET_FILE,
            failure,
            path,
            outputs,
        ),
    }
}

unsafe fn write_stream(
    frame: *const DrStackFrameV2,
    operation: u8,
    contents: *const u8,
    contents_length: usize,
    outputs: FailureOutputs,
) -> u8 {
    let stream = if operation == WRITE_STDERR {
        StandardStream::Stderr
    } else {
        StandardStream::Stdout
    };
    match device_io::write_bytes(stream, contents, contents_length) {
        WriteOutcome::Success => 0,
        WriteOutcome::BrokenPipe => exit_process(0),
        WriteOutcome::OtherFailure(failure) => fail_io(
            frame,
            OP_WRITE,
            if operation == WRITE_STDERR {
                TARGET_STDERR
            } else {
                TARGET_STDOUT
            },
            failure,
            ptr::null_mut(),
            outputs,
        ),
    }
}

unsafe fn fail_io(
    frame: *const DrStackFrameV2,
    operation: u8,
    target: u8,
    failure: Failure,
    path: *mut DrStringV1,
    outputs: FailureOutputs,
) -> u8 {
    fail_io_fields(
        frame,
        operation,
        target,
        failure,
        path,
        IoFailureOutputs {
            message: outputs.message,
            path: outputs.path,
            system_code: outputs.system_code,
            transferred_byte_count: outputs.transferred_byte_count,
            meta: outputs.meta,
        },
    )
}

/// Constructs the canonical checked file failure, retaining its borrowed path.
/// # Safety
/// Outputs must be writable and disjoint; path must be a live borrowed string.
pub(crate) unsafe fn fail_file_io(
    frame: *const DrStackFrameV2,
    operation: u8,
    failure: Failure,
    path: *mut DrStringV1,
    outputs: IoFailureOutputs,
) -> u8 {
    fail_io_fields(frame, operation, TARGET_FILE, failure, path, outputs)
}

/// A failure of a string-named target other than a file: a child process or
/// one of its pipes, named by the program. Retains the borrowed name.
/// # Safety
/// Outputs must be writable and disjoint; name must be a live borrowed string.
pub(crate) unsafe fn fail_named_io(
    frame: *const DrStackFrameV2,
    operation: u8,
    target: u8,
    failure: Failure,
    name: *mut DrStringV1,
    outputs: IoFailureOutputs,
) -> u8 {
    if !target_names_string(target) {
        crate::runtime_invariant_panic();
    }
    fail_io_fields(frame, operation, target, failure, name, outputs)
}

/// Standard-device failures carry their actual target, never a fabricated path.
pub(crate) unsafe fn fail_standard_io(
    frame: *const DrStackFrameV2,
    operation: u8,
    stream: StandardStream,
    failure: Failure,
    outputs: IoFailureOutputs,
) -> u8 {
    let target = match stream {
        StandardStream::Stdin => TARGET_STDIN,
        StandardStream::Stdout => TARGET_STDOUT,
        StandardStream::Stderr => TARGET_STDERR,
    };
    fail_io_fields(frame, operation, target, failure, ptr::null_mut(), outputs)
}

unsafe fn fail_io_fields(
    frame: *const DrStackFrameV2,
    operation: u8,
    target: u8,
    failure: Failure,
    path: *mut DrStringV1,
    outputs: IoFailureOutputs,
) -> u8 {
    *outputs.system_code = failure.system_code.unwrap_or(0);
    let transferred_byte_count = failure
        .transferred_byte_count
        .filter(|count| i64::try_from(*count).is_ok());
    *outputs.transferred_byte_count = transferred_byte_count.unwrap_or(0);
    *outputs.message = io_message(frame, operation, target, failure.reason as u8, path);
    if !path.is_null() {
        *outputs.path = crate::dr_v1_string_retain(path);
    }
    *outputs.meta = metadata(
        operation,
        target,
        failure.reason as u8,
        ERROR_IO,
        failure.system_code.is_some(),
        false,
        transferred_byte_count.is_some(),
    );
    1
}

unsafe fn fail_utf8(
    frame: *const DrStackFrameV2,
    source: u8,
    path: *mut DrStringV1,
    valid: usize,
    invalid: Option<usize>,
    outputs: FailureOutputs,
) -> u8 {
    *outputs.system_code = 0;
    *outputs.transferred_byte_count = 0;
    *outputs.message = utf8_message(frame, source, path);
    if !path.is_null() {
        *outputs.path = crate::dr_v1_string_retain(path);
    }
    *outputs.valid_byte_count = valid;
    *outputs.invalid_byte_count = invalid.unwrap_or(0);
    *outputs.meta = metadata(
        OP_READ,
        source,
        Reason::InvalidInput as u8,
        ERROR_INVALID_UTF8,
        false,
        invalid.is_some(),
        false,
    );
    1
}

const fn metadata(
    operation: u8,
    target: u8,
    reason: u8,
    kind: u8,
    has_system_code: bool,
    has_invalid_count: bool,
    has_transferred_count: bool,
) -> u64 {
    ((operation as u64) << META_OPERATION_SHIFT)
        | ((target as u64) << META_TARGET_SHIFT)
        | ((reason as u64) << META_REASON_SHIFT)
        | ((kind as u64) << META_KIND_SHIFT)
        | ((has_system_code as u64) << META_HAS_SYSTEM_CODE_SHIFT)
        | ((has_invalid_count as u64) << META_HAS_INVALID_COUNT_SHIFT)
        | ((has_transferred_count as u64) << META_HAS_TRANSFERRED_COUNT_SHIFT)
}

/// Projects failed group access/configuration through the existing IoError transport.
/// # Safety
/// Status is a live failed record without byte progress. Frame is null or live.
/// All outputs are disjoint writable empty slots, disjoint from status.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_transfer_group_failure(
    frame: *const DrStackFrameV2,
    operation: u8,
    status: *const DrIoStatusV1,
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred_byte_count: *mut usize,
    meta: *mut u64,
) -> u8 {
    if !matches!(operation, OP_ACCESS | OP_CONFIGURE) {
        crate::runtime_invariant_panic();
    }
    fail_nontransfer_io(
        frame,
        operation,
        TARGET_TRANSFER_GROUP,
        status,
        IoFailureOutputs {
            message,
            path,
            system_code,
            transferred_byte_count,
            meta,
        },
    )
}

/// Targets whose IoTarget case carries the reported string: a file path or
/// a process's program.
pub(crate) const fn target_names_string(target: u8) -> bool {
    matches!(
        target,
        TARGET_FILE
            | TARGET_PROCESS
            | TARGET_PROCESS_INPUT
            | TARGET_PROCESS_OUTPUT
            | TARGET_PROCESS_ERRORS
    )
}

unsafe fn io_message(
    frame: *const DrStackFrameV2,
    operation: u8,
    target: u8,
    reason: u8,
    path: *const DrStringV1,
) -> *mut DrStringV1 {
    let operation = match operation {
        OP_OPEN => IoMessageOperation::Open,
        OP_READ => IoMessageOperation::Read,
        OP_WRITE => IoMessageOperation::Write,
        OP_APPEND => IoMessageOperation::Append,
        OP_FLUSH => IoMessageOperation::Flush,
        OP_CLOSE => IoMessageOperation::Close,
        OP_REGISTER => IoMessageOperation::Register,
        OP_UNREGISTER => IoMessageOperation::Unregister,
        OP_WAIT => IoMessageOperation::Wait,
        OP_CONFIGURE => IoMessageOperation::Configure,
        OP_ACCESS => IoMessageOperation::Access,
        OP_SEEK => IoMessageOperation::Seek,
        OP_SYNCHRONIZE_DATA => IoMessageOperation::SynchronizeData,
        OP_SYNCHRONIZE_METADATA => IoMessageOperation::SynchronizeMetadata,
        OP_RESIZE => IoMessageOperation::Resize,
        OP_LOCK => IoMessageOperation::Lock,
        OP_UNLOCK => IoMessageOperation::Unlock,
        OP_SPAWN => IoMessageOperation::Spawn,
        OP_TERMINATE => IoMessageOperation::Terminate,
        _ => IoMessageOperation::Read,
    };
    let reason = match reason {
        value if value == Reason::NotFound as u8 => IoMessageReason::NotFound,
        value if value == Reason::PermissionDenied as u8 => IoMessageReason::PermissionDenied,
        value if value == Reason::InvalidInput as u8 => IoMessageReason::InvalidInput,
        value if value == Reason::Interrupted as u8 => IoMessageReason::Interrupted,
        value if value == Reason::ResourceExhausted as u8 => IoMessageReason::ResourceExhausted,
        value if value == Reason::Unsupported as u8 => IoMessageReason::Unsupported,
        value if value == Reason::Closed as u8 => IoMessageReason::Closed,
        _ => IoMessageReason::Other,
    };
    let path_bytes = if target_names_string(target) && !path.is_null() {
        core::slice::from_raw_parts(string_bytes(path), (*path).byte_length)
    } else {
        &[]
    };
    let target = match target {
        TARGET_FILE => IoMessageTarget::File(path_bytes),
        TARGET_PROCESS => IoMessageTarget::Process(path_bytes),
        TARGET_PROCESS_INPUT => IoMessageTarget::ProcessInput(path_bytes),
        TARGET_PROCESS_OUTPUT => IoMessageTarget::ProcessOutput(path_bytes),
        TARGET_PROCESS_ERRORS => IoMessageTarget::ProcessErrors(path_bytes),
        TARGET_STDIN => IoMessageTarget::StandardInput,
        TARGET_STDOUT => IoMessageTarget::StandardOutput,
        TARGET_STDERR => IoMessageTarget::StandardError,
        TARGET_READINESS_SET => IoMessageTarget::ReadinessSet,
        TARGET_TRANSFER_PLAN => IoMessageTarget::TransferPlan,
        TARGET_TRANSFER_GROUP => IoMessageTarget::TransferGroup,
        _ => IoMessageTarget::StandardInput,
    };
    build_message(frame, |write| {
        visit_io_error_message_parts(operation, target, reason, write)
    })
}

unsafe fn utf8_message(
    frame: *const DrStackFrameV2,
    source: u8,
    path: *const DrStringV1,
) -> *mut DrStringV1 {
    let path_bytes = if source == TARGET_FILE && !path.is_null() {
        core::slice::from_raw_parts(string_bytes(path), (*path).byte_length)
    } else {
        &[]
    };
    let source = if source == TARGET_FILE {
        Utf8MessageSource::File(path_bytes)
    } else {
        Utf8MessageSource::StandardInput
    };
    build_message(frame, |write| {
        visit_invalid_utf8_message_parts(source, write)
    })
}

unsafe fn build_message(
    frame: *const DrStackFrameV2,
    mut visit: impl FnMut(&mut dyn FnMut(&[u8])),
) -> *mut DrStringV1 {
    let mut length = 0_usize;
    visit(&mut |part| {
        length = length
            .checked_add(part.len())
            .unwrap_or_else(|| panic_catalogued(frame, b"P1205"));
    });
    let result = allocate_string_with_frame(frame, length);
    let mut offset = 0;
    visit(&mut |part| {
        ptr::copy_nonoverlapping(
            part.as_ptr(),
            string_bytes_mut(result).add(offset),
            part.len(),
        );
        offset += part.len();
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    mod readiness;
    mod transfer_plan;

    #[test]
    fn failure_transport_distinguishes_unavailable_zero_and_partial_progress() {
        unsafe {
            for count in [None, Some(0), Some(2048), Some(usize::MAX)] {
                let mut message = ptr::null_mut();
                let mut path = ptr::null_mut();
                let mut system_code = -1;
                let mut valid_byte_count = 0;
                let mut invalid_byte_count = 0;
                let mut transferred_byte_count = usize::MAX;
                let mut meta = 0;
                let outputs = FailureOutputs {
                    message: &mut message,
                    path: &mut path,
                    system_code: &mut system_code,
                    valid_byte_count: &mut valid_byte_count,
                    invalid_byte_count: &mut invalid_byte_count,
                    transferred_byte_count: &mut transferred_byte_count,
                    meta: &mut meta,
                };
                let failure = Failure::from_system_code(28).with_transferred_byte_count(count);
                assert_eq!(
                    fail_io(
                        ptr::null(),
                        OP_WRITE,
                        TARGET_STDOUT,
                        failure,
                        ptr::null_mut(),
                        outputs
                    ),
                    1
                );
                let representable = count.filter(|count| i64::try_from(*count).is_ok());
                assert_eq!(
                    (meta >> META_HAS_TRANSFERRED_COUNT_SHIFT) & 1,
                    u64::from(representable.is_some())
                );
                assert_eq!(transferred_byte_count, representable.unwrap_or(0));
                assert_eq!(system_code, 28);
                assert_eq!((meta >> META_HAS_SYSTEM_CODE_SHIFT) & 1, 1);
                assert!(!message.is_null());
                assert!(path.is_null());
                crate::dr_v1_string_release(message);
            }
        }
    }

    #[test]
    fn successful_checked_io_clears_stale_failure_progress() {
        unsafe {
            let mut result = ptr::dangling_mut();
            let mut message = ptr::dangling_mut();
            let mut path = ptr::dangling_mut();
            let mut system_code = -1;
            let mut valid_byte_count = usize::MAX;
            let mut invalid_byte_count = usize::MAX;
            let mut transferred_byte_count = usize::MAX;
            let mut meta = u64::MAX;
            assert_eq!(
                dr_v4_checked_io(
                    ptr::null(),
                    WRITE_STDOUT,
                    ptr::null_mut(),
                    b"".as_ptr(),
                    0,
                    &mut result,
                    &mut message,
                    &mut path,
                    &mut system_code,
                    &mut valid_byte_count,
                    &mut invalid_byte_count,
                    &mut transferred_byte_count,
                    &mut meta,
                ),
                0
            );
            assert!(result.is_null() && message.is_null() && path.is_null());
            assert_eq!(
                (
                    system_code,
                    valid_byte_count,
                    invalid_byte_count,
                    transferred_byte_count,
                    meta
                ),
                (0, 0, 0, 0, 0)
            );
        }
    }
}
