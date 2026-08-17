use core::ptr;

use doria_diagnostic_catalogue::{
    visit_invalid_utf8_message_parts, visit_io_error_message_parts, IoMessageOperation,
    IoMessageReason, IoMessageTarget, Utf8MessageSource,
};

use crate::device_io::{self, StandardStream, WriteOutcome};
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

const OP_OPEN: u8 = 0;
const OP_READ: u8 = 1;
const OP_WRITE: u8 = 2;
const OP_APPEND: u8 = 3;
const OP_FLUSH: u8 = 4;

const TARGET_FILE: u8 = 0;
const TARGET_STDIN: u8 = 1;
const TARGET_STDOUT: u8 = 2;
const TARGET_STDERR: u8 = 3;

const ERROR_IO: u8 = 0;
const ERROR_INVALID_UTF8: u8 = 1;

const META_OPERATION_SHIFT: u32 = 0;
const META_TARGET_SHIFT: u32 = 8;
const META_REASON_SHIFT: u32 = 16;
const META_KIND_SHIFT: u32 = 24;
const META_HAS_SYSTEM_CODE_SHIFT: u32 = 32;
const META_HAS_INVALID_COUNT_SHIFT: u32 = 33;

struct FailureOutputs {
    message: *mut *mut DrStringV1,
    path: *mut *mut DrStringV1,
    system_code: *mut i64,
    valid_byte_count: *mut usize,
    invalid_byte_count: *mut usize,
    meta: *mut u64,
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
pub unsafe extern "C" fn dr_v3_checked_io(
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
    meta: *mut u64,
) -> u8 {
    *result = ptr::null_mut();
    *message = ptr::null_mut();
    *path = ptr::null_mut();
    *system_code = 0;
    *valid_byte_count = 0;
    *invalid_byte_count = 0;
    *meta = 0;
    let outputs = FailureOutputs {
        message,
        path,
        system_code,
        valid_byte_count,
        invalid_byte_count,
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
        Ok(Some((data, length))) => {
            *result = crate::dr_v1_string_from_utf8(data, length).cast();
            0
        }
        Ok(None) => 0,
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
    let buffered = line_io::take_buffered_input();
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
        match device_io::read_bytes(StandardStream::Stdin, data.add(length), capacity - length) {
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
                    failure,
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
    *outputs.system_code = failure.system_code.unwrap_or(0);
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
) -> u64 {
    ((operation as u64) << META_OPERATION_SHIFT)
        | ((target as u64) << META_TARGET_SHIFT)
        | ((reason as u64) << META_REASON_SHIFT)
        | ((kind as u64) << META_KIND_SHIFT)
        | ((has_system_code as u64) << META_HAS_SYSTEM_CODE_SHIFT)
        | ((has_invalid_count as u64) << META_HAS_INVALID_COUNT_SHIFT)
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
    let path_bytes = if target == TARGET_FILE && !path.is_null() {
        core::slice::from_raw_parts(string_bytes(path), (*path).byte_length)
    } else {
        &[]
    };
    let target = match target {
        TARGET_FILE => IoMessageTarget::File(path_bytes),
        TARGET_STDIN => IoMessageTarget::StandardInput,
        TARGET_STDOUT => IoMessageTarget::StandardOutput,
        TARGET_STDERR => IoMessageTarget::StandardError,
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
