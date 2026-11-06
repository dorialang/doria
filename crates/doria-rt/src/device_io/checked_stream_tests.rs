use super::*;
use crate::platform_io::{Failure, Reason};
use core::ptr;

struct Slots {
    tag: u64,
    value: usize,
    message: *mut DrStringV1,
    path: *mut DrStringV1,
    system_code: i64,
    transferred: usize,
    meta: u64,
}

impl Slots {
    fn new() -> Self {
        Self {
            tag: u64::MAX,
            value: usize::MAX,
            message: ptr::null_mut(),
            path: ptr::null_mut(),
            system_code: -99,
            transferred: usize::MAX,
            meta: u64::MAX,
        }
    }

    unsafe fn call(&mut self, stream: u8, operation: u8, data: *mut u8, length: usize) -> u8 {
        assert!(self.message.is_null() && self.path.is_null());
        dr_v1_checked_standard_stream_io(
            ptr::null(),
            stream,
            operation,
            data,
            length,
            &mut self.tag,
            &mut self.value,
            &mut self.message,
            &mut self.path,
            &mut self.system_code,
            &mut self.transferred,
            &mut self.meta,
        )
    }

    unsafe fn intrinsic_without_value(
        &mut self,
        operation: u8,
        data: *const u8,
        length: usize,
    ) -> u8 {
        assert!(self.message.is_null() && self.path.is_null());
        let mut result = ptr::dangling_mut();
        let mut valid = usize::MAX;
        let mut invalid = usize::MAX;
        let code = crate::checked_io::dr_v4_checked_io(
            ptr::null(),
            operation,
            ptr::null_mut(),
            data,
            length,
            &mut result,
            &mut self.message,
            &mut self.path,
            &mut self.system_code,
            &mut valid,
            &mut invalid,
            &mut self.transferred,
            &mut self.meta,
        );
        assert!(result.is_null());
        assert_eq!((valid, invalid), (0, 0));
        code
    }

    fn outputs(&mut self) -> IoTransferOutputs {
        IoTransferOutputs {
            tag: &mut self.tag,
            value: &mut self.value,
            failure: IoFailureOutputs {
                message: &mut self.message,
                path: &mut self.path,
                system_code: &mut self.system_code,
                transferred_byte_count: &mut self.transferred,
                meta: &mut self.meta,
            },
        }
    }

    unsafe fn project(
        &mut self,
        device: StandardStream,
        operation: u8,
        result: u8,
        status: &DrIoStatusV1,
    ) -> u8 {
        let outputs = self.outputs();
        outputs.clear();
        outputs.finish(
            ptr::null(),
            operation,
            result,
            status,
            |failure, outputs| fail_standard_io(ptr::null(), operation, device, failure, outputs),
        )
    }

    fn assert_success(&self, tag: u64, count: usize) {
        assert_eq!((self.tag, self.value), (tag, count));
        assert!(self.message.is_null() && self.path.is_null());
        assert_eq!((self.system_code, self.transferred, self.meta), (0, 0, 0));
    }

    fn message(&self) -> &[u8] {
        assert!(!self.message.is_null());
        unsafe {
            core::slice::from_raw_parts(
                crate::string_bytes(self.message),
                (*self.message).byte_length,
            )
        }
    }
}

impl Drop for Slots {
    fn drop(&mut self) {
        unsafe {
            crate::dr_v1_string_release(self.message);
            crate::dr_v1_string_release(self.path);
        }
    }
}

fn device(stream: u8) -> StandardStream {
    match stream {
        0 => StandardStream::Stdin,
        1 => StandardStream::Stdout,
        2 => StandardStream::Stderr,
        _ => unreachable!(),
    }
}

#[test]
fn binary_reads_drain_the_original_queue_without_utf8_conversion_or_allocation() {
    let _serial = super::super::test_lock();
    unsafe {
        let _input = super::super::TestInput::new(b"ab\xff\0cd", true);
        let queue = super::super::stdin_input();
        let mut data = [b'~'; 5];
        let mut slots = Slots::new();
        let ((), allocations) = crate::probe_runtime_allocations(|| {
            assert_eq!(slots.call(0, READ, data.as_mut_ptr(), 3), 0);
            slots.assert_success(0, 3);
            assert_eq!(&data, b"ab\xff~~");
            assert_eq!(slots.call(0, READ, data.as_mut_ptr(), 3), 0);
            slots.assert_success(0, 3);
            assert_eq!(&data, b"\0cd~~");
            assert_eq!(slots.call(0, READ, data.as_mut_ptr(), data.len()), 0);
            slots.assert_success(2, 0);
            assert_eq!(slots.call(0, READ, ptr::null_mut(), 0), 0);
            slots.assert_success(0, 0);
            assert_eq!(super::super::stdin_input(), queue);
        });
        assert_eq!(allocations, 0);
        assert!(super::super::standard_device(StandardStream::Stdin)
            .try_borrow()
            .is_some());
    }
}

#[test]
fn empty_checked_transfers_do_not_acquire_or_release_an_active_device_claim() {
    let _serial = super::super::test_lock();
    for stream in 0..3 {
        let device = super::super::standard_device(device(stream));
        let loan = unsafe { device.borrow() };
        let mut slots = Slots::new();
        let (result, allocations) = crate::probe_runtime_allocations(|| unsafe {
            slots.call(
                stream,
                if stream == 0 { READ } else { WRITE },
                ptr::null_mut(),
                0,
            )
        });
        assert_eq!((result, allocations), (0, 0));
        slots.assert_success(0, 0);
        assert!(device.try_borrow().is_none());
        drop(loan);
        assert!(device.try_borrow().is_some());
    }
}

#[test]
fn copied_device_admission_preserves_the_existing_claim_bytes_and_checked_target() {
    let _serial = super::super::test_lock();
    unsafe {
        let _input = super::super::TestInput::new(b"original", true);
        for stream in 0..3 {
            let device = super::super::standard_device(device(stream));
            let loan = device.borrow();
            let mut bytes = [b'X'; 3];
            let mut slots = Slots::new();
            let operation = if stream == 0 { READ } else { WRITE };
            let (result, allocations) = crate::probe_runtime_allocations(|| {
                slots.call(stream, operation, bytes.as_mut_ptr(), bytes.len())
            });
            assert_eq!((result, allocations), (1, 1));
            assert_eq!(bytes, [b'X'; 3]);
            assert_eq!((slots.tag, slots.value), (0, 0));
            assert!(slots.path.is_null());
            assert_eq!((slots.system_code, slots.transferred), (0, 0));
            let operation = if stream == 0 { OP_READ } else { OP_WRITE };
            assert_eq!(
                slots.meta,
                u64::from(operation) | (u64::from(stream + 1) << 8) | (2 << 16)
            );
            assert_eq!(
                slots.message(),
                match stream {
                    0 => b"failed to read standard input: invalid input".as_slice(),
                    1 => b"failed to write standard output: invalid input".as_slice(),
                    2 => b"failed to write standard error: invalid input".as_slice(),
                    _ => unreachable!(),
                }
            );
            assert!(device.try_borrow().is_none());
            drop(loan);
            assert!(device.try_borrow().is_some());
        }
        let mut bytes = [0; 8];
        let mut slots = Slots::new();
        assert_eq!(slots.call(0, READ, bytes.as_mut_ptr(), bytes.len()), 0);
        slots.assert_success(0, 8);
        assert_eq!(&bytes, b"original");
    }
}

#[test]
fn legacy_intrinsics_use_the_same_fallible_admission_without_draining_loaned_input() {
    let _serial = super::super::test_lock();
    unsafe {
        let _input = super::super::TestInput::new(b"original\n", true);
        for (stream, intrinsic, operation) in [
            (0, crate::checked_io::READ_LINE, OP_READ),
            (0, crate::checked_io::READ_STDIN_BYTES, OP_READ),
            (1, crate::checked_io::WRITE_STDOUT, OP_WRITE),
            (2, crate::checked_io::WRITE_STDERR, OP_WRITE),
        ] {
            let device = super::super::standard_device(device(stream));
            let loan = device.borrow();
            let mut slots = Slots::new();
            let (result, allocations) = crate::probe_runtime_allocations(|| {
                slots.intrinsic_without_value(intrinsic, b"X".as_ptr(), 1)
            });
            assert_eq!((result, allocations), (1, 1));
            assert!(slots.path.is_null());
            assert_eq!((slots.system_code, slots.transferred), (0, 0));
            assert_eq!(
                slots.meta,
                u64::from(operation) | (u64::from(stream + 1) << 8) | (2 << 16)
            );
            assert!(device.try_borrow().is_none());
            drop(loan);
            assert!(device.try_borrow().is_some());
        }
        let line = crate::line_io::read_line().unwrap();
        let (bytes, length) = line.bytes.unwrap();
        assert_eq!(core::slice::from_raw_parts(bytes, length), b"original");
    }
}

#[test]
fn empty_legacy_outputs_and_noop_flush_preserve_an_existing_device_claim() {
    let _serial = super::super::test_lock();
    unsafe {
        for stream in [1, 2] {
            let channel = device(stream);
            let device = super::super::standard_device(channel);
            let loan = device.borrow();
            let mut slots = Slots::new();
            let ((), allocations) = crate::probe_runtime_allocations(|| {
                assert_eq!(
                    super::super::write(channel, ptr::null(), 0),
                    super::super::WriteOutcome::Success
                );
                assert_eq!(
                    super::super::write_bytes(channel, ptr::null(), 0),
                    super::super::WriteOutcome::Success
                );
                assert_eq!(
                    super::super::flush(channel),
                    super::super::WriteOutcome::Success
                );
                assert_eq!(
                    slots.intrinsic_without_value(
                        if stream == 1 {
                            crate::checked_io::WRITE_STDOUT
                        } else {
                            crate::checked_io::WRITE_STDERR
                        },
                        ptr::null(),
                        0,
                    ),
                    0
                );
                assert!(device.try_borrow().is_none());
            });
            assert_eq!(allocations, 0);
            assert!(slots.message.is_null() && slots.path.is_null());
            assert_eq!(
                (slots.system_code, slots.transferred, slots.meta),
                (0, 0, 0)
            );
            drop(loan);
            assert!(device.try_borrow().is_some());
        }
    }
}

#[test]
fn interactivity_queries_never_reenter_or_release_an_input_output_or_error_claim() {
    let _serial = super::super::test_lock();
    unsafe {
        for stream in 0..3 {
            let channel = device(stream);
            let device = super::super::standard_device(channel);
            let loan = device.borrow();
            let expected = super::super::is_interactive_unclaimed(channel);
            let (observed, allocations) =
                crate::probe_runtime_allocations(|| super::super::is_interactive(channel));
            assert_eq!(observed, expected);
            assert_eq!(allocations, 0);
            assert!(device.try_borrow().is_none());
            drop(loan);
            assert!(device.try_borrow().is_some());
        }
    }
}

#[test]
fn invalid_standard_capabilities_keep_the_actual_target_without_accessing_storage() {
    let _serial = super::super::test_lock();
    for (stream, operation, io_operation) in
        [(0, WRITE, OP_WRITE), (1, READ, OP_READ), (2, READ, OP_READ)]
    {
        let mut data = [b'X'; 2];
        let mut slots = Slots::new();
        assert_eq!(
            unsafe { slots.call(stream, operation, data.as_mut_ptr(), data.len()) },
            1
        );
        assert_eq!(data, [b'X'; 2]);
        assert!(slots.path.is_null());
        assert_eq!(
            slots.meta,
            u64::from(io_operation) | (u64::from(stream + 1) << 8) | (2 << 16)
        );
    }
}

#[test]
fn ordinary_outcomes_share_file_transport_without_allocating_or_fabricating_failure_facts() {
    for stream in 0..3 {
        let operation = if stream == 0 { OP_READ } else { OP_WRITE };
        for (result, value, tag, expected) in
            [(0, 0, 0, 0), (0, 3, 0, 3), (1, 99, 1, 0), (2, 99, 2, 0)]
        {
            if stream != 0 && result == 2 {
                continue;
            }
            let status = DrIoStatusV1 {
                value,
                system_code: -99,
                transferred_byte_count: usize::MAX,
                reason: 255,
                has_system_code: 1,
                has_transferred_byte_count: 1,
            };
            let mut slots = Slots::new();
            let (result, allocations) = crate::probe_runtime_allocations(|| unsafe {
                slots.project(device(stream), operation, result, &status)
            });
            assert_eq!((result, allocations), (0, 0));
            slots.assert_success(tag, expected);
        }
    }
}

#[test]
fn checked_device_failures_preserve_all_reasons_codes_and_unknown_zero_positive_progress() {
    for stream in 0..3 {
        let operation = if stream == 0 { OP_READ } else { OP_WRITE };
        for reason in [
            Reason::NotFound,
            Reason::PermissionDenied,
            Reason::InvalidInput,
            Reason::Interrupted,
            Reason::ResourceExhausted,
            Reason::Unsupported,
            Reason::Closed,
            Reason::Other,
        ] {
            for system_code in [None, Some(0), Some(13), Some(-7)] {
                for progress in [None, Some(0), Some(3), Some(usize::MAX)] {
                    let mut status = DrIoStatusV1::default();
                    assert_eq!(
                        status.fail(Failure {
                            reason,
                            system_code,
                            transferred_byte_count: progress
                        }),
                        3
                    );
                    let mut slots = Slots::new();
                    let (result, allocations) = crate::probe_runtime_allocations(|| unsafe {
                        slots.project(device(stream), operation, 3, &status)
                    });
                    assert_eq!((result, allocations), (1, 1));
                    let progress = progress.filter(|count| i64::try_from(*count).is_ok());
                    assert_eq!((slots.tag, slots.value), (0, 0));
                    assert!(slots.path.is_null());
                    assert_eq!(slots.system_code, system_code.unwrap_or(0));
                    assert_eq!(slots.transferred, progress.unwrap_or(0));
                    assert_eq!(
                        slots.meta,
                        u64::from(operation)
                            | (u64::from(stream + 1) << 8)
                            | ((reason as u64) << 16)
                            | (u64::from(system_code.is_some()) << 32)
                            | (u64::from(progress.is_some()) << 34)
                    );
                    assert!(!slots.message().is_empty());
                }
            }
        }
    }
}
