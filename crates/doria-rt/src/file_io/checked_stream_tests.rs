use super::*;
use crate::native_handle::with_close_probe;
use crate::platform_io::{Failure, Reason};
use crate::{
    dr_v1_string_from_utf8, dr_v1_string_release, probe_runtime_allocations, string_bytes,
};
use doria_diagnostic_catalogue::{
    visit_io_error_message_parts, IoMessageOperation, IoMessageReason, IoMessageTarget,
};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::super::stream::transport::object::dr_v1_file_object_drop;

struct Fixture {
    directory: PathBuf,
    path: PathBuf,
    path_bytes: Vec<u8>,
}

impl Fixture {
    fn new(name: &str) -> Self {
        static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "doria-checked-file-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join(format!("{name}-Dória-漢字.bin"));
        let path_bytes = path
            .to_str()
            .expect("UTF-8 fixture path")
            .as_bytes()
            .to_vec();
        Self {
            directory,
            path,
            path_bytes,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.directory);
    }
}

struct RuntimeString(*mut DrStringV1);

impl RuntimeString {
    fn new(bytes: &[u8]) -> Self {
        core::str::from_utf8(bytes).unwrap();
        Self(unsafe { dr_v1_string_from_utf8(bytes.as_ptr(), bytes.len()) })
    }

    fn release(mut self) {
        unsafe { dr_v1_string_release(self.0) };
        self.0 = ptr::null_mut();
    }
}

impl Drop for RuntimeString {
    fn drop(&mut self) {
        unsafe { dr_v1_string_release(self.0) };
    }
}

struct Owner(*mut c_void);

impl Drop for Owner {
    fn drop(&mut self) {
        unsafe {
            // Close consumes the native obligation, not this common class
            // payload. Drop fields once, then free the original allocation.
            dr_v1_file_object_drop(ptr::null(), self.0);
            dr_v1_class_free(self.0.cast());
        }
    }
}

struct Request {
    operation: u8,
    path: *mut DrStringV1,
    owner: *mut c_void,
    access: u8,
    creation: u8,
    append: u8,
    data: *mut u8,
    length: usize,
}

impl Request {
    fn open(path: *mut DrStringV1, access: u8, creation: u8, append: u8) -> Self {
        Self {
            operation: OPEN,
            path,
            owner: ptr::null_mut(),
            access,
            creation,
            append,
            data: ptr::null_mut(),
            length: 0,
        }
    }

    fn transfer(operation: u8, owner: &Owner, data: *mut u8, length: usize) -> Self {
        Self {
            operation,
            path: ptr::null_mut(),
            owner: owner.0,
            access: 0,
            creation: 0,
            append: 0,
            data,
            length,
        }
    }

    fn close(owner: &Owner) -> Self {
        Self::transfer(CLOSE, owner, ptr::null_mut(), 0)
    }
}

struct Slots {
    tag: u64,
    value: usize,
    message: *mut DrStringV1,
    error_path: *mut DrStringV1,
    system_code: i64,
    transferred: usize,
    meta: u64,
    owns_error: bool,
}

impl Slots {
    fn new() -> Self {
        Self {
            tag: 0,
            value: 0,
            message: ptr::null_mut(),
            error_path: ptr::null_mut(),
            system_code: 0,
            transferred: 0,
            meta: 0,
            owns_error: false,
        }
    }

    fn release_error(&mut self) {
        if self.owns_error {
            unsafe {
                dr_v1_string_release(self.message);
                dr_v1_string_release(self.error_path);
            }
        }
        self.message = ptr::null_mut();
        self.error_path = ptr::null_mut();
        self.owns_error = false;
    }

    fn prime_stale(&mut self) {
        self.release_error();
        self.tag = u64::MAX;
        self.value = usize::MAX;
        // Output sentinels are never dereferenced or released. The checked ABI
        // must replace them before returning success or an owned error target.
        self.message = ptr::dangling_mut();
        self.error_path = ptr::dangling_mut();
        self.system_code = -1;
        self.transferred = usize::MAX;
        self.meta = u64::MAX;
    }

    fn call(&mut self, request: Request) -> u8 {
        self.prime_stale();
        let result = unsafe {
            dr_v1_checked_file_io(
                ptr::null(),
                request.operation,
                request.path,
                request.owner,
                request.access,
                request.creation,
                request.append,
                request.data,
                request.length,
                &mut self.tag,
                &mut self.value,
                &mut self.message,
                &mut self.error_path,
                &mut self.system_code,
                &mut self.transferred,
                &mut self.meta,
            )
        };
        self.owns_error = true;
        result
    }

    fn fail_status(&mut self, operation: u8, path: *mut DrStringV1, status: &DrFileStatusV1) -> u8 {
        self.prime_stale();
        // Production resets result slots before calling this failure-only seam.
        self.tag = 0;
        self.value = 0;
        let outputs = Outputs {
            tag: &mut self.tag,
            value: &mut self.value,
            failure: IoFailureOutputs {
                message: &mut self.message,
                path: &mut self.error_path,
                system_code: &mut self.system_code,
                transferred_byte_count: &mut self.transferred,
                meta: &mut self.meta,
            },
        };
        let result = unsafe {
            fail_status(
                ptr::null(),
                operation,
                TARGET_FILE,
                path,
                status,
                outputs.failure,
            )
        };
        self.owns_error = true;
        result
    }

    fn control(&mut self, owner: &Owner, operation: u8, offset: i64, origin: u8) -> u8 {
        self.prime_stale();
        self.tag = 0;
        let mut value = -1_i64;
        let result = unsafe {
            dr_v1_checked_file_control(
                ptr::null(),
                owner.0,
                operation,
                offset,
                origin,
                &mut value,
                &mut self.message,
                &mut self.error_path,
                &mut self.system_code,
                &mut self.transferred,
                &mut self.meta,
            )
        };
        self.value = usize::try_from(value).unwrap();
        self.owns_error = true;
        result
    }

    fn assert_success(&self, tag: u64, value: usize) {
        assert_eq!(self.tag, tag);
        assert_eq!(self.value, value);
        assert!(self.message.is_null());
        assert!(self.error_path.is_null());
        assert_eq!(self.system_code, 0);
        assert_eq!(self.transferred, 0);
        assert_eq!(self.meta, 0);
    }

    fn assert_failure(
        &self,
        operation: u8,
        reason: Reason,
        code: Option<i64>,
        progress: Option<usize>,
    ) {
        let representable = progress.filter(|count| i64::try_from(*count).is_ok());
        assert_eq!(self.tag, 0);
        assert_eq!(self.value, 0);
        assert!(!self.message.is_null());
        assert!(!self.error_path.is_null());
        assert_eq!(self.system_code, code.unwrap_or(0));
        assert_eq!(self.transferred, representable.unwrap_or(0));
        // Exact private IoError metadata: file target/kind and invalid-UTF8
        // count are zero; absent optional progress is not a known-zero count.
        let expected = u64::from(operation)
            | (u64::from(reason as u8) << 16)
            | (u64::from(code.is_some()) << 32)
            | (u64::from(representable.is_some()) << 34);
        assert_eq!(self.meta, expected);
    }
}

impl Drop for Slots {
    fn drop(&mut self) {
        self.release_error();
    }
}

unsafe fn assert_string(string: *mut DrStringV1, expected: &[u8]) {
    assert_eq!(
        core::slice::from_raw_parts(string_bytes(string), (*string).byte_length),
        expected,
    );
}

fn catalogue_reason(reason: Reason) -> IoMessageReason {
    match reason {
        Reason::NotFound => IoMessageReason::NotFound,
        Reason::PermissionDenied => IoMessageReason::PermissionDenied,
        Reason::InvalidInput => IoMessageReason::InvalidInput,
        Reason::Interrupted => IoMessageReason::Interrupted,
        Reason::ResourceExhausted => IoMessageReason::ResourceExhausted,
        Reason::Unsupported => IoMessageReason::Unsupported,
        Reason::Closed => IoMessageReason::Closed,
        Reason::Other => IoMessageReason::Other,
    }
}

fn expected_message(operation: IoMessageOperation, reason: Reason, path: &[u8]) -> Vec<u8> {
    let mut expected = Vec::new();
    // Collect the authoritative catalogue visitor, not a second message
    // template, escaping rule, or runtime string-builder implementation.
    visit_io_error_message_parts(
        operation,
        IoMessageTarget::File(path),
        catalogue_reason(reason),
        |part| expected.extend_from_slice(part),
    );
    expected
}

#[test]
fn synthetic_transfer_outcomes_keep_empty_data_would_block_and_eof_distinct() {
    for operation in [OP_READ, OP_WRITE] {
        for (raw_tag, count, result_tag, result_count) in
            [(0, 0, 0, 0), (0, 17, 0, 17), (1, 0, 1, 0), (2, 0, 2, 0)]
        {
            if operation == OP_WRITE && raw_tag == 2 {
                continue; // EOF is a read outcome, not a write outcome.
            }
            let mut slots = Slots::new();
            let status = DrFileStatusV1 {
                value: count,
                ..DrFileStatusV1::default()
            };
            let outputs = Outputs {
                tag: &mut slots.tag,
                value: &mut slots.value,
                failure: IoFailureOutputs {
                    message: &mut slots.message,
                    path: &mut slots.error_path,
                    system_code: &mut slots.system_code,
                    transferred_byte_count: &mut slots.transferred,
                    meta: &mut slots.meta,
                },
            };
            let (result, allocations) = probe_runtime_allocations(|| unsafe {
                outputs.finish(
                    ptr::null(),
                    operation,
                    raw_tag,
                    &status,
                    |failure, outputs| {
                        crate::checked_io::fail_file_io(
                            ptr::null(),
                            operation,
                            failure,
                            ptr::null_mut(),
                            outputs,
                        )
                    },
                )
            });
            assert_eq!(result, 0);
            assert_eq!(allocations, 0);
            slots.assert_success(result_tag, result_count);
        }
    }
}

#[test]
fn checked_seek_and_shared_inspection_preserve_path_and_allocate_only_errors() {
    let fixture = Fixture::new("seek-inspect");
    fs::write(&fixture.path, b"abcdef").unwrap();
    let path = RuntimeString::new(&fixture.path_bytes);
    let mut slots = Slots::new();
    assert_eq!(slots.call(Request::open(path.0, 2, 0, 0)), 0);
    let owner = Owner(slots.value as *mut c_void);
    path.release();
    let (((), allocations), closes) = with_close_probe(|| {
        probe_runtime_allocations(|| {
            for _ in 0..32 {
                for (operation, offset, origin, value) in [
                    (3, 2, 0, 2),
                    (4, 0, 0, 2),
                    (5, 0, 0, 6),
                    (3, -1, 1, 1),
                    (3, -2, 2, 4),
                    (4, 0, 0, 4),
                ] {
                    assert_eq!(slots.control(&owner, operation, offset, origin), 0);
                    slots.assert_success(0, value);
                }
            }
        })
    });
    assert_eq!((allocations, closes), (0, 0));
    assert_eq!(slots.control(&owner, 3, -1, 0), 1);
    assert_eq!(slots.meta & 0xff, u64::from(OP_SEEK));
    assert_eq!(slots.meta & (1 << 34), 0);
    assert_eq!(slots.transferred, 0);
    unsafe {
        assert_string(slots.error_path, &fixture.path_bytes);
    }
    assert_eq!(slots.control(&owner, 4, 0, 0), 0);
    slots.assert_success(0, 4);
    assert_eq!(slots.call(Request::close(&owner)), 0);
    // Storage/path remain initialized after close for error construction.
    for operation in [3, 4, 5] {
        assert_eq!(slots.control(&owner, operation, 0, 0), 1);
        slots.assert_failure(
            if operation == 3 { OP_SEEK } else { OP_ACCESS },
            Reason::Closed,
            None,
            None,
        );
        unsafe {
            assert_string(slots.error_path, &fixture.path_bytes);
        }
    }
    let ((), closes) = with_close_probe(|| drop(owner));
    assert_eq!(closes, 0);
    assert_eq!(fs::read(&fixture.path).unwrap(), b"abcdef");
}

#[test]
fn checked_binary_transfers_reset_outputs_and_allocate_no_successful_transfer_storage() {
    let fixture = Fixture::new("binary");
    let path = RuntimeString::new(&fixture.path_bytes);
    let mut slots = Slots::new();
    assert_eq!(slots.call(Request::open(path.0, 1, 2, 0)), 0);
    let writer = Owner(slots.value as *mut c_void);
    assert!(!writer.0.is_null());
    slots.assert_success(0, writer.0 as usize);
    assert_eq!(unsafe { (*path.0).references }, 2);
    let expected = [0x00, 0xff, 0x80, b'\n', b'A'];
    let mut source = [0xa1, 0x00, 0xff, 0x80, b'\n', b'A', 0xb2];
    let (((), allocations), closes) = with_close_probe(|| {
        probe_runtime_allocations(|| {
            assert_eq!(
                slots.call(Request::transfer(WRITE, &writer, ptr::null_mut(), 0)),
                0
            );
            slots.assert_success(0, 0);
            let mut offset = 0;
            for _ in 0..expected.len() {
                if offset == expected.len() {
                    break;
                }
                let requested = (expected.len() - offset).min(2);
                assert_eq!(
                    slots.call(Request::transfer(
                        WRITE,
                        &writer,
                        unsafe { source.as_mut_ptr().add(1 + offset) },
                        requested,
                    )),
                    0,
                );
                let accepted = slots.value;
                assert!(accepted > 0 && accepted <= requested);
                slots.assert_success(0, accepted);
                offset += accepted;
            }
            assert_eq!(offset, expected.len());
            assert_eq!(source[0], 0xa1);
            assert_eq!(source[6], 0xb2);
            source[1..6].fill(0x77);
            assert_eq!(slots.call(Request::close(&writer)), 0);
            slots.assert_success(0, 0);
            drop(writer);
        })
    });
    assert_eq!(allocations, 0);
    assert_eq!(closes, 1);
    assert_eq!(unsafe { (*path.0).references }, 1);

    assert_eq!(slots.call(Request::open(path.0, 0, 0, 0)), 0);
    let reader = Owner(slots.value as *mut c_void);
    slots.assert_success(0, reader.0 as usize);
    let mut destination = [0xc3; 7];
    let (((), allocations), closes) = with_close_probe(|| {
        probe_runtime_allocations(|| {
            assert_eq!(
                slots.call(Request::transfer(READ, &reader, ptr::null_mut(), 0)),
                0
            );
            slots.assert_success(0, 0);
            let mut offset = 0;
            for _ in 0..expected.len() {
                if offset == expected.len() {
                    break;
                }
                let requested = (expected.len() - offset).min(2);
                assert_eq!(
                    slots.call(Request::transfer(
                        READ,
                        &reader,
                        unsafe { destination.as_mut_ptr().add(1 + offset) },
                        requested,
                    )),
                    0,
                );
                let count = slots.value;
                assert!(count > 0 && count <= requested);
                slots.assert_success(0, count);
                offset += count;
            }
            assert_eq!(offset, expected.len());
            assert_eq!(&destination[1..6], &expected);
            assert_eq!(destination[0], 0xc3);
            assert_eq!(destination[6], 0xc3);
            let before_eof = destination;
            assert_eq!(
                slots.call(Request::transfer(
                    READ,
                    &reader,
                    unsafe { destination.as_mut_ptr().add(1) },
                    5,
                )),
                0,
            );
            slots.assert_success(2, 0);
            assert_eq!(destination, before_eof);
            assert_eq!(
                slots.call(Request::transfer(READ, &reader, ptr::null_mut(), 0)),
                0
            );
            slots.assert_success(0, 0);
            assert_eq!(slots.call(Request::close(&reader)), 0);
            slots.assert_success(0, 0);
            drop(reader);
        })
    });
    assert_eq!(allocations, 0);
    assert_eq!(closes, 1);
    assert_eq!(unsafe { (*path.0).references }, 1);
    assert_eq!(fs::read(&fixture.path).unwrap(), expected);
}

#[test]
fn checked_file_resize_preserves_cursor_zero_growth_and_retained_error_path() {
    let fixture = Fixture::new("resize");
    fs::write(&fixture.path, b"abcdef").unwrap();
    let path = RuntimeString::new(&fixture.path_bytes);
    let mut slots = Slots::new();
    assert_eq!(slots.call(Request::open(path.0, 2, 0, 0)), 0);
    let owner = Owner(slots.value as *mut c_void);
    path.release();
    assert_eq!(slots.control(&owner, 3, 5, 0), 0);
    let (((), allocations), closes) = with_close_probe(|| {
        probe_runtime_allocations(|| {
            for length in [2, 6] {
                assert_eq!(slots.control(&owner, 6, length, 0), 0);
                slots.assert_success(0, 0);
                assert_eq!(slots.control(&owner, 4, 0, 0), 0);
                slots.assert_success(0, 5);
                assert_eq!(slots.control(&owner, 5, 0, 0), 0);
                slots.assert_success(0, length as usize);
            }
        })
    });
    assert_eq!((allocations, closes), (0, 0));
    assert_eq!(fs::read(&fixture.path).unwrap(), b"ab\0\0\0\0");
    assert_eq!(slots.control(&owner, 6, -1, 0), 1);
    slots.assert_failure(OP_RESIZE, Reason::InvalidInput, None, None);
    unsafe {
        assert_string(slots.error_path, &fixture.path_bytes);
    }
    assert_eq!(slots.control(&owner, 4, 0, 0), 0);
    slots.assert_success(0, 5);
    assert_eq!(slots.control(&owner, 5, 0, 0), 0);
    slots.assert_success(0, 6);
    assert_eq!(slots.control(&owner, 6, 0, 0), 0);
    assert!(fs::read(&fixture.path).unwrap().is_empty());
    assert_eq!(slots.control(&owner, 4, 0, 0), 0);
    slots.assert_success(0, 5);
    assert_eq!(slots.call(Request::close(&owner)), 0);
    assert_eq!(slots.control(&owner, 6, 1, 0), 1);
    slots.assert_failure(OP_RESIZE, Reason::Closed, None, None);
    unsafe {
        assert_string(slots.error_path, &fixture.path_bytes);
    }
    let ((), closes) = with_close_probe(|| drop(owner));
    assert_eq!(closes, 0);
}

#[test]
fn checked_synchronization_preserves_file_state_and_retained_path_without_success_allocations() {
    let fixture = Fixture::new("synchronize");
    fs::write(&fixture.path, b"visible\0\xff").unwrap();
    let path = RuntimeString::new(&fixture.path_bytes);
    let mut slots = Slots::new();
    assert_eq!(slots.call(Request::open(path.0, 2, 0, 0)), 0);
    let owner = Owner(slots.value as *mut c_void);
    path.release();
    assert_eq!(slots.control(&owner, 3, 3, 0), 0);
    let (((), allocations), closes) = with_close_probe(|| {
        probe_runtime_allocations(|| {
            for operation in [1, 2] {
                assert_eq!(slots.control(&owner, operation, 0, 0), 0);
                slots.assert_success(0, 0);
                assert_eq!(slots.control(&owner, 4, 0, 0), 0);
                slots.assert_success(0, 3);
                assert_eq!(slots.control(&owner, 5, 0, 0), 0);
                slots.assert_success(0, 9);
            }
        })
    });
    assert_eq!((allocations, closes), (0, 0));
    assert_eq!(slots.call(Request::close(&owner)), 0);
    for (operation, label) in [(1, OP_SYNCHRONIZE_DATA), (2, OP_SYNCHRONIZE_METADATA)] {
        assert_eq!(slots.control(&owner, operation, 0, 0), 1);
        slots.assert_failure(label, Reason::Closed, None, None);
        unsafe {
            assert_string(slots.error_path, &fixture.path_bytes);
        }
    }
    let ((), closes) = with_close_probe(|| drop(owner));
    assert_eq!(closes, 0);
    assert_eq!(fs::read(&fixture.path).unwrap(), b"visible\0\xff");
}

#[test]
fn checked_duplex_owner_shares_one_cursor_between_reads_and_writes() {
    let fixture = Fixture::new("duplex");
    fs::write(&fixture.path, [0x00, 0xff, 0x80, b'\n', b'A']).unwrap();
    let path = RuntimeString::new(&fixture.path_bytes);
    let mut slots = Slots::new();
    assert_eq!(slots.call(Request::open(path.0, 2, 0, 0)), 0);
    let owner = Owner(slots.value as *mut c_void);
    slots.assert_success(0, owner.0 as usize);
    let mut destination = [0xa1, 0xc3, 0xc3, 0xb2];
    let mut source = [0xd4, 0x55, 0x00, 0xe5];
    let (((), allocations), closes) = with_close_probe(|| {
        probe_runtime_allocations(|| {
            let mut read = 0;
            for _ in 0..2 {
                if read == 2 {
                    break;
                }
                assert_eq!(
                    slots.call(Request::transfer(
                        READ,
                        &owner,
                        unsafe { destination.as_mut_ptr().add(1 + read) },
                        2 - read,
                    )),
                    0,
                );
                let count = slots.value;
                assert!(count > 0 && count <= 2 - read);
                slots.assert_success(0, count);
                read += count;
            }
            assert_eq!(read, 2);
            assert_eq!(destination, [0xa1, 0x00, 0xff, 0xb2]);
            let mut written = 0;
            for _ in 0..2 {
                if written == 2 {
                    break;
                }
                assert_eq!(
                    slots.call(Request::transfer(
                        WRITE,
                        &owner,
                        unsafe { source.as_mut_ptr().add(1 + written) },
                        2 - written,
                    )),
                    0,
                );
                let count = slots.value;
                assert!(count > 0 && count <= 2 - written);
                slots.assert_success(0, count);
                written += count;
            }
            assert_eq!(written, 2);
            assert_eq!(source, [0xd4, 0x55, 0x00, 0xe5]);
            assert_eq!(slots.call(Request::close(&owner)), 0);
            slots.assert_success(0, 0);
            drop(owner);
        })
    });
    assert_eq!(allocations, 0);
    assert_eq!(closes, 1);
    assert_eq!(
        fs::read(&fixture.path).unwrap(),
        [0x00, 0xff, 0x55, 0x00, b'A']
    );
}

#[test]
fn checked_failed_open_retains_only_the_error_target_and_creates_no_close_obligation() {
    let missing = Fixture::new("missing");
    let existing = Fixture::new("invalid-request");
    fs::write(&existing.path, b"preserved\0\xff").unwrap();
    for (bytes, access, creation, reason, has_code) in [
        (&missing.path_bytes[..], 0, 0, Reason::NotFound, true),
        (&existing.path_bytes[..], 0, 4, Reason::InvalidInput, false),
        (&b"bad\0path"[..], 0, 0, Reason::InvalidInput, false),
    ] {
        let path = RuntimeString::new(bytes);
        let original = path.0;
        let mut slots = Slots::new();
        let (result, closes) =
            with_close_probe(|| slots.call(Request::open(path.0, access, creation, 0)));
        assert_eq!(result, 1);
        assert_eq!(closes, 0);
        let code = has_code.then_some(slots.system_code);
        if has_code {
            assert_ne!(slots.system_code, 0);
        }
        slots.assert_failure(OP_OPEN, reason, code, None);
        assert_eq!(slots.error_path, original);
        assert_eq!(unsafe { (*original).references }, 2);
        unsafe {
            assert_string(original, bytes);
            assert_string(
                slots.message,
                &expected_message(IoMessageOperation::Open, reason, bytes),
            );
        }
        path.release();
        assert_eq!(unsafe { (*slots.error_path).references }, 1);
        unsafe { assert_string(slots.error_path, bytes) };
        let ((), closes) = with_close_probe(|| drop(slots));
        assert_eq!(closes, 0);
    }
    assert!(!missing.path.exists());
    assert_eq!(fs::read(&existing.path).unwrap(), b"preserved\0\xff");
}

#[test]
fn checked_wrong_access_errors_keep_the_original_opening_path_after_caller_release() {
    for (access, operation, io_operation, catalogue_operation) in [
        (0, WRITE, OP_WRITE, IoMessageOperation::Write),
        (1, READ, OP_READ, IoMessageOperation::Read),
    ] {
        let fixture = Fixture::new("wrong-access");
        fs::write(&fixture.path, b"preserved\0\xff").unwrap();
        let path = RuntimeString::new(&fixture.path_bytes);
        let original = path.0;
        let mut slots = Slots::new();
        assert_eq!(slots.call(Request::open(path.0, access, 0, 0)), 0);
        let owner = Owner(slots.value as *mut c_void);
        slots.assert_success(0, owner.0 as usize);
        path.release();
        assert_eq!(unsafe { (*original).references }, 1);
        let mut region = [0xa1, 0x99, 0xb2];
        assert_eq!(
            slots.call(Request::transfer(
                operation,
                &owner,
                unsafe { region.as_mut_ptr().add(1) },
                1,
            )),
            1,
        );
        assert_ne!(slots.system_code, 0);
        let reason = Failure::from_system_code(slots.system_code).reason;
        slots.assert_failure(io_operation, reason, Some(slots.system_code), None);
        assert_eq!(slots.error_path, original);
        assert_eq!(unsafe { (*original).references }, 2);
        assert_eq!(region, [0xa1, 0x99, 0xb2]);
        unsafe {
            assert_string(slots.error_path, &fixture.path_bytes);
            assert_string(
                slots.message,
                &expected_message(catalogue_operation, reason, &fixture.path_bytes),
            );
        }
        let target = RuntimeString(core::mem::replace(&mut slots.error_path, ptr::null_mut()));
        let ((result, allocations), closes) = with_close_probe(|| {
            probe_runtime_allocations(|| {
                let result = slots.call(Request::close(&owner));
                drop(owner);
                result
            })
        });
        assert_eq!(result, 0);
        assert_eq!(allocations, 0);
        assert_eq!(closes, 1);
        slots.assert_success(0, 0);
        assert_eq!(unsafe { (*target.0).references }, 1);
        unsafe { assert_string(target.0, &fixture.path_bytes) };
        assert_eq!(fs::read(&fixture.path).unwrap(), b"preserved\0\xff");
    }
}

#[test]
fn consuming_checked_close_keeps_path_for_error_construction_until_field_cleanup() {
    let fixture = Fixture::new("close-path");
    fs::write(&fixture.path, b"data").unwrap();
    let path = RuntimeString::new(&fixture.path_bytes);
    let original = path.0;
    let mut slots = Slots::new();
    assert_eq!(slots.call(Request::open(path.0, 0, 0, 0)), 0);
    let owner = Owner(slots.value as *mut c_void);
    path.release();
    let ((result, allocations), closes) =
        with_close_probe(|| probe_runtime_allocations(|| slots.call(Request::close(&owner))));
    assert_eq!(result, 0);
    assert_eq!(allocations, 0);
    assert_eq!(closes, 1);
    slots.assert_success(0, 0);
    let borrowed = unsafe { dr_v1_file_object_path(owner.0) };
    assert_eq!(borrowed, original);
    assert_eq!(unsafe { (*borrowed).references }, 1);
    unsafe { assert_string(borrowed, &fixture.path_bytes) };

    // The actual kernel close above succeeded. This is synthetic close-error
    // packing against its retained target, not a forced C-ABI close failure.
    let status = DrFileStatusV1 {
        reason: Reason::Interrupted as u8,
        ..DrFileStatusV1::default()
    };
    assert_eq!(slots.fail_status(OP_CLOSE, borrowed, &status), 1);
    slots.assert_failure(OP_CLOSE, Reason::Interrupted, None, None);
    assert_eq!(slots.error_path, original);
    assert_eq!(unsafe { (*original).references }, 2);
    let (((), allocations), closes) =
        with_close_probe(|| probe_runtime_allocations(|| drop(owner)));
    assert_eq!(closes, 0);
    assert_eq!(allocations, 0);
    assert_eq!(unsafe { (*slots.error_path).references }, 1);
    unsafe {
        assert_string(slots.error_path, &fixture.path_bytes);
        assert_string(
            slots.message,
            &expected_message(
                IoMessageOperation::Close,
                Reason::Interrupted,
                &fixture.path_bytes,
            ),
        );
    }
}

#[test]
fn synthetic_file_failure_status_uses_canonical_messages_and_exact_optional_metadata() {
    let bytes = "target-Dória-漢字`\n.bin".as_bytes();
    let path = RuntimeString::new(bytes);
    for (operation, catalogue_operation) in [
        (OP_OPEN, IoMessageOperation::Open),
        (OP_READ, IoMessageOperation::Read),
        (OP_WRITE, IoMessageOperation::Write),
        (OP_CLOSE, IoMessageOperation::Close),
        (OP_SYNCHRONIZE_DATA, IoMessageOperation::SynchronizeData),
        (
            OP_SYNCHRONIZE_METADATA,
            IoMessageOperation::SynchronizeMetadata,
        ),
        (OP_RESIZE, IoMessageOperation::Resize),
    ] {
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
            let expected = expected_message(catalogue_operation, reason, bytes);
            for code in [None, Some(0), Some(123)] {
                for progress in [None, Some(0), Some(2048), Some(usize::MAX)] {
                    if matches!(
                        operation,
                        OP_SYNCHRONIZE_DATA | OP_SYNCHRONIZE_METADATA | OP_RESIZE
                    ) && progress.is_some()
                    {
                        continue;
                    }
                    let status = DrFileStatusV1 {
                        value: u64::MAX,
                        reason: reason as u8,
                        // Nonzero stale backing values with presence=false
                        // must remain unavailable, not accidentally present.
                        system_code: code.unwrap_or(-1),
                        transferred_byte_count: progress.unwrap_or(17),
                        has_system_code: u8::from(code.is_some()),
                        has_transferred_byte_count: u8::from(progress.is_some()),
                    };
                    let mut slots = Slots::new();
                    let (result, allocations) =
                        probe_runtime_allocations(|| slots.fail_status(operation, path.0, &status));
                    assert_eq!(result, 1);
                    // One canonical message allocation; the error path is a
                    // retained reference, never a cloned opening-path string.
                    assert_eq!(allocations, 1);
                    slots.assert_failure(operation, reason, code, progress);
                    assert_eq!(slots.error_path, path.0);
                    assert_eq!(unsafe { (*path.0).references }, 2);
                    unsafe {
                        assert_string(slots.error_path, bytes);
                        assert_string(slots.message, &expected);
                    }
                    drop(slots);
                    assert_eq!(unsafe { (*path.0).references }, 1);
                }
            }
        }
    }
}
