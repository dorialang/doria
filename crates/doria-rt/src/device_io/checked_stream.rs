//! Checked primitive transfers over the canonical process-owned devices.
//! Source declarations and inline view layouts are compiler-owned separately.

use crate::checked_io::{fail_standard_io, IoFailureOutputs, IoTransferOutputs, OP_READ, OP_WRITE};
use crate::io_status::DrIoStatusV1;
use crate::{panic_catalogued, DrStackFrameV2, DrStringV1};

use super::stream::{dr_v1_standard_stream_read, dr_v1_standard_stream_write};
use super::StandardStream;

const READ: u8 = 1;
const WRITE: u8 = 2;

/// `dr_v1_checked_standard_stream_io` for a PHP host, which names the buffer by range.
/// # Safety
/// As `dr_v1_checked_standard_stream_io`, for the range `byte_region::host_range` forms.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_standard_stream_io(
    frame: *const DrStackFrameV2,
    stream: u8,
    operation: u8,
    base: *mut u8,
    capacity: usize,
    offset: usize,
    length: usize,
    result_tag: *mut u64,
    result_value: *mut usize,
    message: *mut *mut DrStringV1,
    error_path: *mut *mut DrStringV1,
    system_code: *mut i64,
    transferred: *mut usize,
    meta: *mut u64,
) -> u8 {
    dr_v1_checked_standard_stream_io(
        frame,
        stream,
        operation,
        crate::byte_region::host_range(base, capacity, offset, length),
        length,
        result_tag,
        result_value,
        message,
        error_path,
        system_code,
        transferred,
        meta,
    )
}

/// Borrows a standard view and caller storage for one primitive transfer.
/// Stream tags: 0 input, 1 output, 2 error. Operation tags: 1 read, 2 write.
/// Returns 0 success or 1 existing IoError. Read outcomes are 0 Data,
/// 1 WouldBlock, 2 EndOfStream; write outcomes are 0 Accepted, 1 WouldBlock.
/// Data/Accepted carry byte counts. Empty requests do no device I/O or admission.
/// A checked failure owns one message and has no fabricated file path. Optional
/// system code and byte progress preserve the shared checked_io encoding.
/// Ordinary output broken pipes retain the existing status-0 termination rule.
/// No handle, view owner, buffer or control block is allocated on success.
///
/// # Safety
/// Tags are from the private sets above. Outputs are valid, disjoint writable
/// empty slots, disjoint from inputs. Frame is null or a live frame chain.
/// Data is writable/readable for length on read/write respectively, or null for
/// length zero. Its loan ends at return; it cannot alias a pending transfer's
/// borrowed storage. Device admission uses the shared process-device claim.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_checked_standard_stream_io(
    frame: *const DrStackFrameV2,
    stream: u8,
    operation: u8,
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
    let outputs = IoTransferOutputs {
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
    let device = match stream {
        0 => StandardStream::Stdin,
        1 => StandardStream::Stdout,
        2 => StandardStream::Stderr,
        _ => panic_catalogued(frame, b"P1001"),
    };
    let mut status = DrIoStatusV1::default();
    let (io_operation, result) = match operation {
        READ => (
            OP_READ,
            dr_v1_standard_stream_read(stream, data, length, &mut status),
        ),
        WRITE => (
            OP_WRITE,
            dr_v1_standard_stream_write(stream, data, length, &mut status),
        ),
        _ => panic_catalogued(frame, b"P1001"),
    };
    outputs.finish(frame, io_operation, result, &status, |failure, outputs| {
        fail_standard_io(frame, io_operation, device, failure, outputs)
    })
}

#[cfg(test)]
#[path = "checked_stream_tests.rs"]
mod tests;
