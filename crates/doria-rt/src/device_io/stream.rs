//! Non-owning standard-device transport: tags identify existing devices, not
//! allocated owners. Public Doria names and compiler dispatch remain separate.
use crate::io_status::DrIoStatusV1;
use crate::platform_io::Failure;

use super::{read_progress, write_progress, StandardStream};

/// Pure borrowed projection of the canonical process device. No handle query,
/// native association, allocation, buffer drain or ownership claim occurs.
#[no_mangle]
pub extern "C" fn dr_v1_standard_stream_readiness_source(stream: u8) -> *const core::ffi::c_void {
    let stream = match stream {
        0 => StandardStream::Stdin,
        1 => StandardStream::Stdout,
        2 => StandardStream::Stderr,
        _ => return core::ptr::null(),
    };
    super::standard_device(stream).source().value()
}

#[no_mangle]
pub extern "C" fn dr_v1_standard_stream_readiness_source_home(
    stream: u8,
) -> *const core::ffi::c_void {
    let stream = match stream {
        0 => StandardStream::Stdin,
        1 => StandardStream::Stdout,
        2 => StandardStream::Stderr,
        _ => return core::ptr::null(),
    };
    super::standard_device(stream).source().home()
}

/// Borrows the canonical stdin device and caller-owned writable byte storage.
/// Returns 0 data, 1 would-block, 2 EOF, 3 checked failure; status carries count.
/// No handle, buffer, or control block is constructed by accessing the device.
/// # Safety
/// Stdin consumers must be serialized, and any earlier returned line slice must
/// no longer be borrowed. Destination must be writable for capacity bytes (null
/// permitted for zero capacity), disjoint from the writable status.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_standard_stream_read(
    stream: u8,
    destination: *mut u8,
    capacity: usize,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    if stream != StandardStream::Stdin as u8 {
        return (*status).fail(Failure::invalid_input());
    }
    (*status).read(read_progress(StandardStream::Stdin, destination, capacity))
}

/// Borrows stdout or stderr and readonly byte storage for one primitive write.
/// Returns 0 accepted, 1 would-block, 3 checked failure; status carries count.
/// Ordinary standard-output broken pipes retain the existing status-0 exit.
/// # Safety
/// Source must be readable for length bytes (null permitted for zero length)
/// and disjoint from the writable status. Caller serializes device mutations.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_standard_stream_write(
    stream: u8,
    source: *const u8,
    length: usize,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    let stream = match stream {
        1 => StandardStream::Stdout,
        2 => StandardStream::Stderr,
        _ => return (*status).fail(Failure::invalid_input()),
    };
    let result = write_progress(stream, source, length);
    if let Err(failure) = result {
        #[cfg(unix)]
        let outcome = super::classify_unix_write_error(Some(failure));
        #[cfg(windows)]
        let outcome = super::classify_windows_write_error(Some(failure));
        #[cfg(not(any(unix, windows)))]
        let outcome = super::WriteOutcome::OtherFailure(failure);
        if outcome == super::WriteOutcome::BrokenPipe {
            crate::exit_process(0);
        }
    }
    (*status).write(result)
}

#[cfg(test)]
mod tests {
    use core::ptr;

    use super::*;

    #[test]
    fn empty_transfers_need_no_device_or_buffer_and_clear_stale_status() {
        let _serial = super::super::device::test_lock();
        unsafe {
            let mut status = DrIoStatusV1 {
                value: 99,
                system_code: 99,
                transferred_byte_count: 99,
                reason: 99,
                has_system_code: 1,
                has_transferred_byte_count: 1,
            };
            assert_eq!(
                dr_v1_standard_stream_read(0, ptr::null_mut(), 0, &mut status),
                0
            );
            for stream in [1, 2] {
                assert_eq!(
                    dr_v1_standard_stream_write(stream, ptr::null(), 0, &mut status),
                    0
                );
            }
            assert_eq!(status.value, 0);
            assert_eq!(status.system_code, 0);
            assert_eq!(status.transferred_byte_count, 0);
            assert_eq!(status.reason, 0);
            assert_eq!(status.has_system_code, 0);
            assert_eq!(status.has_transferred_byte_count, 0);
        }
    }

    #[test]
    fn invalid_capabilities_fail_without_touching_byte_storage() {
        unsafe {
            let mut status = DrIoStatusV1::default();
            for stream in [1, 2, 255] {
                assert_eq!(
                    dr_v1_standard_stream_read(stream, ptr::null_mut(), 1, &mut status),
                    3
                );
                assert_eq!(
                    status.reason,
                    crate::platform_io::Reason::InvalidInput as u8
                );
            }
            for stream in [0, 255] {
                assert_eq!(
                    dr_v1_standard_stream_write(stream, ptr::null(), 1, &mut status),
                    3
                );
                assert_eq!(
                    status.reason,
                    crate::platform_io::Reason::InvalidInput as u8
                );
            }
        }
    }
}
