use super::*;
use crate::native_handle::with_close_probe;
use crate::platform_io::Reason;
use crate::{
    dr_v1_class_free, dr_v1_string_from_utf8, dr_v1_string_release, dr_v1_string_retain,
    dr_v2_class_allocate, probe_runtime_allocations,
};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(any(unix, windows))]
#[path = "object/locking_tests.rs"]
mod locking_tests;

struct Fixture {
    directory: PathBuf,
    path: PathBuf,
    path_bytes: Vec<u8>,
}

impl Fixture {
    fn new(name: &str) -> Self {
        static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "doria-file-object-{}-{}-{}",
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

#[test]
fn object_pending_endpoint_projects_the_actual_contained_owner_without_layout_alias_or_io() {
    let fixture = Fixture::new("pending-endpoint");
    fs::write(&fixture.path, b"unchanged").unwrap();
    let path = RuntimeString::new(&fixture.path_bytes);
    for access in [0, 1, 2] {
        let mut file = ObjectStorage::new();
        let mut status = stale_status();
        assert_eq!(file.open(&path, access, 0, 0, &mut status), 0);
        unsafe {
            let expected =
                ptr::from_mut((*file.pointer.cast::<FileObject>()).owner()).cast::<c_void>();
            let scalar = dr_v1_file_object_readiness_source(file.pointer);
            let source_home = dr_v1_file_object_readiness_source_home(file.pointer);
            let (((endpoint, again), allocations), closes) = with_close_probe(|| {
                probe_runtime_allocations(|| {
                    (
                        dr_v1_file_object_pending_endpoint(file.pointer),
                        dr_v1_file_object_pending_endpoint(file.pointer),
                    )
                })
            });
            assert_eq!(endpoint, expected);
            assert_eq!(again, expected);
            assert_ne!(endpoint, scalar.cast_mut());
            // The runtime may place a field at offset zero. Coincident homes
            // do not grant layout reinterpretation; each typed projection is
            // formed by the owner that knows its actual private representation.
            assert_eq!(*source_home.cast::<*const c_void>(), scalar);
            assert_eq!((allocations, closes), (0, 0));
            assert_eq!(dr_v1_file_object_path(file.pointer), path.0);
            assert_eq!(
                dr_v1_file_object_control(file.pointer, 4, 0, 0, &mut status),
                0
            );
            assert_eq!(status.value, 0);
            let (result, closes) =
                with_close_probe(|| dr_v1_file_object_close(file.pointer, &mut status));
            assert_eq!((result, closes), (0, 1));
            let ((), closes) = with_close_probe(|| file.destroy());
            assert_eq!(closes, 0);
        }
    }
    assert_eq!(fs::read(&fixture.path).unwrap(), b"unchanged");
}

#[test]
fn object_readiness_source_loans_the_contained_owner_without_changing_state() {
    let fixture = Fixture::new("source");
    fs::write(&fixture.path, b"unchanged").unwrap();
    let path = RuntimeString::new(&fixture.path_bytes);
    for access in [0, 1, 2] {
        let mut file = ObjectStorage::new();
        let mut status = stale_status();
        assert_eq!(file.open(&path, access, 0, 0, &mut status), 0);
        unsafe {
            let expected_owner = (*file.pointer.cast::<FileObject>()).owner();
            let expected_handle = expected_owner.native_handle();
            let expected_owner = ptr::from_mut(expected_owner);
            let projection = pending_source(file.pointer).unwrap();
            let expected = projection.value();
            let expected_home = projection.home();
            let references = (*path.0).references;
            let ((source, again, home, home_again), allocations) =
                probe_runtime_allocations(|| {
                    (
                        dr_v1_file_object_readiness_source(file.pointer),
                        dr_v1_file_object_readiness_source(file.pointer),
                        dr_v1_file_object_readiness_source_home(file.pointer),
                        dr_v1_file_object_readiness_source_home(file.pointer),
                    )
                });
            assert_eq!(source, expected);
            assert_eq!(home, expected_home);
            assert_eq!(home_again, home);
            assert_eq!(*home.cast::<*const c_void>(), source);
            assert_eq!(again, source);
            assert_eq!(allocations, 0);
            assert_eq!(
                (*path.0).references,
                references,
                "formation retains nothing"
            );
            let forwarded = *home.cast::<*const c_void>();
            let physical = ReadinessProjection::borrow(forwarded).unwrap();
            assert_eq!(physical.origin.handle().unwrap(), expected_handle);
            let (owner, origin) = physical.origin.file().unwrap();
            assert_eq!(owner, expected_owner);
            assert_eq!(origin.payload, file.pointer);
            assert_eq!(origin.path, path.0);
            assert_eq!(dr_v1_file_object_path(file.pointer), path.0);
            assert_eq!(
                dr_v1_file_object_control(file.pointer, 4, 0, 0, &mut status),
                0
            );
            assert_eq!(status.value, 0);
            let (result, closes) =
                with_close_probe(|| dr_v1_file_object_close(file.pointer, &mut status));
            assert_eq!(result, 0);
            assert_eq!(closes, 1);
            let (_, closes) = with_close_probe(|| file.destroy());
            assert_eq!(closes, 0, "class cleanup must not retry consuming close");
        }
    }
    assert_eq!(fs::read(&fixture.path).unwrap(), b"unchanged");
}

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

struct ObjectStorage {
    pointer: *mut c_void,
    size: usize,
    initialized: bool,
}

impl ObjectStorage {
    fn new() -> Self {
        let mut size = 0;
        let mut alignment = 0;
        unsafe { dr_v1_file_object_layout(&mut size, &mut alignment) };
        let pointer = unsafe { dr_v2_class_allocate(ptr::null(), size, alignment).cast() };
        Self {
            pointer,
            size,
            initialized: false,
        }
    }

    fn open(
        &mut self,
        path: &RuntimeString,
        access: u8,
        creation: u8,
        append: u8,
        status: &mut DrFileStatusV1,
    ) -> u8 {
        assert!(!self.initialized);
        let result = unsafe {
            dr_v1_file_object_open(path.0, access, creation, append, self.pointer, status)
        };
        self.initialized = result == 0;
        result
    }

    fn destroy(&mut self) {
        assert!(self.initialized);
        self.initialized = false;
        unsafe { dr_v1_file_object_drop(ptr::null(), self.pointer) };
    }
}

impl Drop for ObjectStorage {
    fn drop(&mut self) {
        if self.initialized {
            self.destroy();
        }
        // File-object drop destroys fields only. The original class allocator
        // owns the payload, including storage never initialized by failed open.
        unsafe { dr_v1_class_free(self.pointer.cast()) };
    }
}

fn stale_status() -> DrFileStatusV1 {
    DrFileStatusV1 {
        value: 91,
        system_code: 5,
        transferred_byte_count: 7,
        reason: Reason::Other as u8,
        has_system_code: 1,
        has_transferred_byte_count: 1,
    }
}

fn assert_clear_failure_facts(status: &DrFileStatusV1) {
    assert_eq!(status.reason, 0);
    assert_eq!(status.system_code, 0);
    assert_eq!(status.has_system_code, 0);
    assert_eq!(status.transferred_byte_count, 0);
    assert_eq!(status.has_transferred_byte_count, 0);
}

unsafe fn assert_path(pointer: *mut DrStringV1, expected: &[u8]) {
    assert_eq!(
        core::slice::from_raw_parts(string_bytes(pointer), (*pointer).byte_length),
        expected,
    );
}

#[test]
fn queried_headerless_object_layout_uses_one_common_class_allocation() {
    let ((size, alignment), allocations) = probe_runtime_allocations(|| {
        let mut size = usize::MAX;
        let mut alignment = usize::MAX;
        unsafe { dr_v1_file_object_layout(&mut size, &mut alignment) };
        (size, alignment)
    });
    assert_eq!(allocations, 0);
    assert_eq!(size, mem::size_of::<MaybeUninit<FileObject>>());
    assert_eq!(alignment, mem::align_of::<MaybeUninit<FileObject>>());
    let (storage, allocations) = probe_runtime_allocations(ObjectStorage::new);
    assert_eq!(allocations, 1);
    assert_eq!(storage.size, size);
    assert_eq!(storage.pointer as usize % alignment, 0);
    let ((), allocations) = probe_runtime_allocations(|| drop(storage));
    assert_eq!(allocations, 0);
}

#[test]
fn failed_object_open_does_not_touch_payload_retain_path_or_create_close_obligation() {
    let existing = Fixture::new("existing");
    let missing = Fixture::new("missing");
    fs::write(&existing.path, b"preserved\0\xff").unwrap();
    let cases = [
        (&missing.path_bytes[..], 0, 0, 0, Reason::NotFound),
        (&existing.path_bytes[..], 3, 4, 0, Reason::InvalidInput),
        (&existing.path_bytes[..], 2, 5, 0, Reason::InvalidInput),
        (&existing.path_bytes[..], 2, 4, 2, Reason::InvalidInput),
        (&existing.path_bytes[..], 0, 4, 0, Reason::InvalidInput),
        (&existing.path_bytes[..], 0, 0, 1, Reason::InvalidInput),
        (&b"bad\0path"[..], 0, 0, 0, Reason::InvalidInput),
    ];
    for (bytes, access, creation, append, reason) in cases {
        let path = RuntimeString::new(bytes);
        let mut storage = ObjectStorage::new();
        // Raw bytes only: failed construction must never form or drop a typed
        // FileObject from this deliberately invalid object representation.
        unsafe { ptr::write_bytes(storage.pointer.cast::<u8>(), 0xa5, storage.size) };
        let mut status = stale_status();
        let (result, closes) =
            with_close_probe(|| storage.open(&path, access, creation, append, &mut status));
        assert_eq!(result, 3);
        assert_eq!(closes, 0);
        assert_eq!(status.reason, reason as u8);
        assert_eq!(status.value, 0);
        assert_eq!(status.has_transferred_byte_count, 0);
        assert_eq!(status.transferred_byte_count, 0);
        if reason == Reason::InvalidInput {
            assert_eq!(status.has_system_code, 0);
            assert_eq!(status.system_code, 0);
        } else {
            assert_eq!(status.has_system_code, 1);
            assert_ne!(status.system_code, 0);
        }
        unsafe {
            assert_eq!((*path.0).references, 1);
            assert!(
                core::slice::from_raw_parts(storage.pointer.cast::<u8>(), storage.size)
                    .iter()
                    .all(|byte| *byte == 0xa5)
            );
        }
        let ((), closes) = with_close_probe(|| drop(storage));
        assert_eq!(closes, 0);
    }
    assert_eq!(fs::read(&existing.path).unwrap(), b"preserved\0\xff");
    assert!(!missing.path.exists());
}

#[test]
fn retained_opening_path_survives_caller_release_and_consuming_close() {
    let fixture = Fixture::new("path-lifetime");
    let path = RuntimeString::new(&fixture.path_bytes);
    let original = path.0;
    let mut storage = ObjectStorage::new();
    let mut status = stale_status();
    assert_eq!(storage.open(&path, 2, 2, 0, &mut status), 0);
    assert_clear_failure_facts(&status);
    assert_eq!(unsafe { (*original).references }, 2);
    path.release();
    assert_eq!(unsafe { (*original).references }, 1);

    let ((), allocations) = probe_runtime_allocations(|| {
        for _ in 0..64 {
            let borrowed = unsafe { dr_v1_file_object_path(storage.pointer) };
            assert_eq!(borrowed, original);
            unsafe { assert_path(borrowed, &fixture.path_bytes) };
        }
    });
    assert_eq!(allocations, 0);
    assert_eq!(unsafe { (*original).references }, 1);
    status = stale_status();
    let ((result, allocations), closes) = with_close_probe(|| {
        probe_runtime_allocations(|| unsafe {
            dr_v1_file_object_close(storage.pointer, &mut status)
        })
    });
    assert_eq!(result, 0);
    assert_eq!(allocations, 0);
    assert_eq!(closes, 1);
    assert_eq!(status.value, 0);
    assert_clear_failure_facts(&status);
    let borrowed = unsafe { dr_v1_file_object_path(storage.pointer) };
    assert_eq!(borrowed, original);
    unsafe { assert_path(borrowed, &fixture.path_bytes) };

    // Canonical error construction can retain this borrowed target after
    // close. Destroying the wrapper releases only its own reference.
    let target = RuntimeString(unsafe { dr_v1_string_retain(borrowed) });
    let (((), allocations), closes) =
        with_close_probe(|| probe_runtime_allocations(|| storage.destroy()));
    assert_eq!(allocations, 0);
    assert_eq!(closes, 0);
    assert_eq!(unsafe { (*target.0).references }, 1);
    unsafe { assert_path(target.0, &fixture.path_bytes) };
    drop(storage);
}

#[test]
fn borrowed_binary_transfers_empty_requests_and_eof_allocate_no_runtime_storage() {
    let fixture = Fixture::new("binary");
    let path = RuntimeString::new(&fixture.path_bytes);
    let mut storage = ObjectStorage::new();
    let mut status = stale_status();
    assert_eq!(storage.open(&path, 2, 2, 0, &mut status), 0);
    let expected = [0x00, 0xff, 0x80, b'\n', b'A'];
    let mut source = [0xa1, 0x00, 0xff, 0x80, b'\n', b'A', 0xb2];
    let mut destination = [0xc3; 7];
    let ((), allocations) = probe_runtime_allocations(|| {
        status = stale_status();
        assert_eq!(
            unsafe { dr_v1_file_object_read(storage.pointer, ptr::null_mut(), 0, &mut status) },
            0,
        );
        assert_eq!(status.value, 0);
        assert_clear_failure_facts(&status);
        status = stale_status();
        assert_eq!(
            unsafe { dr_v1_file_object_write(storage.pointer, ptr::null(), 0, &mut status) },
            0,
        );
        assert_eq!(status.value, 0);
        assert_clear_failure_facts(&status);

        let mut written = 0;
        for _ in 0..expected.len() {
            if written == expected.len() {
                break;
            }
            let requested = (expected.len() - written).min(2);
            status = stale_status();
            assert_eq!(
                unsafe {
                    dr_v1_file_object_write(
                        storage.pointer,
                        source.as_ptr().add(1 + written),
                        requested,
                        &mut status,
                    )
                },
                0,
            );
            assert_clear_failure_facts(&status);
            let count = usize::try_from(status.value).unwrap();
            assert!(count > 0 && count <= requested);
            written += count;
        }
        assert_eq!(written, expected.len());
        assert_eq!(source[0], 0xa1);
        assert_eq!(source[6], 0xb2);
        // A completed synchronous write no longer borrows this caller region.
        source[1..6].fill(0x77);
        status = stale_status();
        assert_eq!(
            unsafe { dr_v1_file_object_control(storage.pointer, 3, 0, 0, &mut status) },
            0,
        );
        assert_eq!(status.value, 0);
        assert_clear_failure_facts(&status);
        let mut read = 0;
        for _ in 0..expected.len() {
            if read == expected.len() {
                break;
            }
            let requested = (expected.len() - read).min(2);
            status = stale_status();
            assert_eq!(
                unsafe {
                    dr_v1_file_object_read(
                        storage.pointer,
                        destination.as_mut_ptr().add(1 + read),
                        requested,
                        &mut status,
                    )
                },
                0,
            );
            assert_clear_failure_facts(&status);
            let count = usize::try_from(status.value).unwrap();
            assert!(count > 0 && count <= requested);
            read += count;
        }
        assert_eq!(read, expected.len());
        assert_eq!(&destination[1..6], &expected);
        assert_eq!(destination[0], 0xc3);
        assert_eq!(destination[6], 0xc3);
        let before_eof = destination;
        status = stale_status();
        assert_eq!(
            unsafe {
                dr_v1_file_object_read(
                    storage.pointer,
                    destination.as_mut_ptr().add(1),
                    5,
                    &mut status,
                )
            },
            2,
        );
        assert_eq!(status.value, 0);
        assert_clear_failure_facts(&status);
        assert_eq!(destination, before_eof);
        status = stale_status();
        assert_eq!(
            unsafe { dr_v1_file_object_read(storage.pointer, ptr::null_mut(), 0, &mut status) },
            0,
        );
        assert_eq!(status.value, 0);
        assert_clear_failure_facts(&status);
        status = stale_status();
        assert_eq!(
            unsafe { dr_v1_file_object_control(storage.pointer, 0, 0, 0, &mut status) },
            0
        );
        assert_eq!(status.value, 0);
        assert_clear_failure_facts(&status);
    });
    assert_eq!(allocations, 0);
    let (((), allocations), closes) = with_close_probe(|| {
        probe_runtime_allocations(|| {
            status = stale_status();
            assert_eq!(
                unsafe { dr_v1_file_object_close(storage.pointer, &mut status) },
                0
            );
            assert_clear_failure_facts(&status);
            storage.destroy();
        })
    });
    assert_eq!(allocations, 0);
    assert_eq!(closes, 1);
    assert_eq!(fs::read(&fixture.path).unwrap(), expected);
}

#[test]
fn readonly_object_inspection_keeps_shared_access_and_rejects_closed_resources() {
    let fixture = Fixture::new("readonly-inspection");
    fs::write(&fixture.path, b"abcdef").unwrap();
    let path = RuntimeString::new(&fixture.path_bytes);
    let mut storage = ObjectStorage::new();
    let mut status = stale_status();
    assert_eq!(storage.open(&path, 2, 0, 0, &mut status), 0);
    unsafe {
        assert_eq!(
            dr_v1_file_object_control(storage.pointer, 3, 2, 0, &mut status),
            0
        );
        {
            let object = &*storage.pointer.cast::<FileObject>();
            let resource = &*object.resource;
            let first = &*resource.shared_file().unwrap();
            let second = &*resource.shared_file().unwrap();
            let (((), allocations), closes) = with_close_probe(|| {
                probe_runtime_allocations(|| {
                    for _ in 0..32 {
                        for (operation, expected) in [(4, 2), (5, 6)] {
                            status = stale_status();
                            assert_eq!(
                                dr_v1_file_object_inspect(
                                    storage.pointer.cast_const(),
                                    operation,
                                    &mut status
                                ),
                                0
                            );
                            assert_eq!(status.value, expected);
                            assert_clear_failure_facts(&status);
                        }
                        for operation in [0, 1, 2, 3, 6, 255] {
                            status = stale_status();
                            assert_eq!(
                                dr_v1_file_object_inspect(
                                    storage.pointer.cast_const(),
                                    operation,
                                    &mut status
                                ),
                                3
                            );
                            assert_eq!(status.reason, Reason::InvalidInput as u8);
                            assert_eq!(
                                (
                                    status.value,
                                    status.has_system_code,
                                    status.has_transferred_byte_count
                                ),
                                (0, 0, 0)
                            );
                        }
                        assert_eq!(first.position().unwrap(), 2);
                        assert_eq!(second.length().unwrap(), 6);
                        assert_eq!(
                            dr_v1_file_object_readiness_source(storage.pointer),
                            resource.source().value()
                        );
                        assert_eq!(dr_v1_file_object_path(storage.pointer), path.0);
                        assert_eq!((*path.0).references, 2);
                    }
                })
            });
            assert_eq!((allocations, closes), (0, 0));
        }
        assert_eq!(dr_v1_file_object_close(storage.pointer, &mut status), 0);
        for operation in [4, 5] {
            status = stale_status();
            assert_eq!(
                dr_v1_file_object_inspect(storage.pointer.cast_const(), operation, &mut status),
                3
            );
            assert_eq!(status.reason, Reason::Closed as u8);
            assert_eq!(
                (
                    status.value,
                    status.has_system_code,
                    status.has_transferred_byte_count
                ),
                (0, 0, 0)
            );
            assert_eq!(dr_v1_file_object_path(storage.pointer), path.0);
        }
    }
    let ((), closes) = with_close_probe(|| drop(storage));
    assert_eq!(closes, 0);
    assert_eq!(unsafe { (*path.0).references }, 1);
    assert_eq!(fs::read(&fixture.path).unwrap(), b"abcdef");
}

#[test]
fn opaque_file_controls_preserve_cursor_path_and_ownership_without_allocating() {
    let fixture = Fixture::new("controls");
    fs::write(&fixture.path, b"abcdef").unwrap();
    let path = RuntimeString::new(&fixture.path_bytes);
    let mut storage = ObjectStorage::new();
    let mut status = stale_status();
    assert_eq!(storage.open(&path, 2, 0, 0, &mut status), 0);
    let (((), allocations), closes) = with_close_probe(|| {
        probe_runtime_allocations(|| {
            for (operation, offset, origin, expected) in [
                (4, 0, 0, 0),
                (5, 0, 0, 6),
                (3, 2, 0, 2),
                (3, 1, 1, 3),
                (3, -1, 2, 5),
                (5, 0, 0, 6),
                (4, 0, 0, 5),
            ] {
                status = stale_status();
                assert_eq!(
                    unsafe {
                        dr_v1_file_object_control(
                            storage.pointer,
                            operation,
                            offset,
                            origin,
                            &mut status,
                        )
                    },
                    0,
                );
                assert_eq!(status.value, expected);
                assert_clear_failure_facts(&status);
            }
            for operation in [0, 1, 2] {
                status = stale_status();
                assert_eq!(
                    unsafe {
                        dr_v1_file_object_control(storage.pointer, operation, 0, 0, &mut status)
                    },
                    0,
                );
                assert_eq!(status.value, 0);
                assert_clear_failure_facts(&status);
                assert_eq!(
                    unsafe { dr_v1_file_object_control(storage.pointer, 4, 0, 0, &mut status) },
                    0,
                );
                assert_eq!(status.value, 5);
            }
            for (operation, origin) in [(6, 0), (11, 0), (255, 0), (3, 3), (3, 255), (3, 0), (7, 2)]
            {
                status = stale_status();
                assert_eq!(
                    unsafe {
                        dr_v1_file_object_control(
                            storage.pointer,
                            operation,
                            -1,
                            origin,
                            &mut status,
                        )
                    },
                    3,
                );
                assert_eq!(status.value, 0);
                assert_eq!(status.reason, Reason::InvalidInput as u8);
                assert_eq!(status.has_system_code, 0);
                assert_eq!(status.system_code, 0);
                assert_eq!(status.has_transferred_byte_count, 0);
                assert_eq!(status.transferred_byte_count, 0);
                assert_eq!(
                    unsafe { dr_v1_file_object_control(storage.pointer, 4, 0, 0, &mut status) },
                    0,
                );
                assert_eq!(status.value, 5);
                assert_clear_failure_facts(&status);
                let borrowed = unsafe { dr_v1_file_object_path(storage.pointer) };
                assert_eq!(borrowed, path.0);
                unsafe { assert_path(borrowed, &fixture.path_bytes) };
                assert_eq!(unsafe { (*path.0).references }, 2);
            }
        })
    });
    assert_eq!(allocations, 0);
    assert_eq!(closes, 0);
    let ((), closes) = with_close_probe(|| drop(storage));
    assert_eq!(closes, 1);
    assert_eq!(unsafe { (*path.0).references }, 1);
    assert_eq!(fs::read(&fixture.path).unwrap(), b"abcdef");
    // Successful sync and readback do not establish power-loss durability.
}

#[test]
fn explicit_close_and_implicit_drop_each_close_once_and_leave_payload_free_to_caller() {
    for explicit in [false, true] {
        let fixture = Fixture::new("cleanup");
        fs::write(&fixture.path, b"data").unwrap();
        let path = RuntimeString::new(&fixture.path_bytes);
        let mut storage = ObjectStorage::new();
        let mut status = stale_status();
        assert_eq!(storage.open(&path, 0, 0, 0, &mut status), 0);
        assert_eq!(unsafe { (*path.0).references }, 2);
        let (((), allocations), closes) = with_close_probe(|| {
            probe_runtime_allocations(|| {
                if explicit {
                    status = stale_status();
                    assert_eq!(
                        unsafe { dr_v1_file_object_close(storage.pointer, &mut status) },
                        0
                    );
                    assert_eq!(status.value, 0);
                    assert_clear_failure_facts(&status);
                }
                storage.destroy();
            })
        });
        assert_eq!(closes, 1);
        assert_eq!(allocations, 0);
        assert_eq!(unsafe { (*path.0).references }, 1);
        // Storage is still allocated but contains no live FileObject. This
        // common class free is the sole payload free, not another field drop.
        let (((), allocations), closes) =
            with_close_probe(|| probe_runtime_allocations(|| drop(storage)));
        assert_eq!(closes, 0);
        assert_eq!(allocations, 0);
        assert_eq!(fs::read(&fixture.path).unwrap(), b"data");
    }
}

#[test]
fn reported_close_failure_preserves_facts_and_path_without_retry_on_drop() {
    for progress in [None, Some(0), Some(3)] {
        let fixture = Fixture::new("reported-close-failure");
        fs::write(&fixture.path, b"data").unwrap();
        let path = RuntimeString::new(&fixture.path_bytes);
        let mut storage = ObjectStorage::new();
        let mut status = stale_status();
        assert_eq!(storage.open(&path, 0, 0, 0, &mut status), 0);
        let failure = Failure {
            reason: Reason::Interrupted,
            system_code: Some(123),
            transferred_byte_count: progress,
        };
        let ((result, allocations), closes) = with_close_probe(|| {
            probe_runtime_allocations(|| {
                // Close the real owner before reporting the injected failure.
                // Never invalidate an externally held descriptor: another test
                // could reuse it before a subsequent close attempt.
                let result = unsafe {
                    (*storage.pointer.cast::<FileObject>()).close_with(|owner| {
                        owner.close()?;
                        Err(failure)
                    })
                };
                // Identical reset/packing mechanism to the production export.
                status = DrFileStatusV1::default();
                status.complete(result.map(|()| 0))
            })
        });
        assert_eq!(result, 3);
        assert_eq!(allocations, 0);
        assert_eq!(closes, 1);
        assert_eq!(status.value, 0);
        assert_eq!(status.reason, failure.reason as u8);
        assert_eq!(status.has_system_code, 1);
        assert_eq!(status.system_code, 123);
        assert_eq!(
            status.has_transferred_byte_count,
            u8::from(progress.is_some())
        );
        assert_eq!(status.transferred_byte_count, progress.unwrap_or(0));
        assert!(unsafe {
            (*(*storage.pointer.cast::<FileObject>()).resource)
                .file()
                .is_err()
        });
        let borrowed = unsafe { dr_v1_file_object_path(storage.pointer) };
        assert_eq!(borrowed, path.0);
        unsafe { assert_path(borrowed, &fixture.path_bytes) };
        assert_eq!(unsafe { (*path.0).references }, 2);
        let (((), allocations), closes) = with_close_probe(|| {
            probe_runtime_allocations(|| {
                storage.destroy();
                drop(storage);
            })
        });
        assert_eq!(allocations, 0);
        assert_eq!(closes, 0);
        assert_eq!(unsafe { (*path.0).references }, 1);
    }
}

#[test]
fn idle_metadata_pin_retains_error_path_but_does_not_keep_an_unused_handle_open() {
    let fixture = Fixture::new("idle-resource-pin");
    fs::write(&fixture.path, b"original").unwrap();
    let path = RuntimeString::new(&fixture.path_bytes);
    let mut storage = ObjectStorage::new();
    let mut status = stale_status();
    assert_eq!(storage.open(&path, 0, 0, 0, &mut status), 0);
    unsafe {
        let origin = pending_source(storage.pointer).unwrap().origin;
        let ((pin, allocations), closes) =
            with_close_probe(|| probe_runtime_allocations(|| origin.metadata_pin().unwrap()));
        assert_eq!((allocations, closes), (0, 0));
        let ((), closes) = with_close_probe(|| {
            storage.destroy();
            drop(storage);
        });
        assert_eq!(closes, 1, "idle metadata must not defer implicit close");
        assert_eq!(origin.handle().unwrap_err().reason, Reason::Closed);
        assert_eq!(origin.selected_pin().err().unwrap().reason, Reason::Closed);
        assert_path(origin.file().unwrap().1.path, &fixture.path_bytes);
        assert_eq!((*path.0).references, 2);
        let ((), closes) = with_close_probe(|| drop(pin));
        assert_eq!(closes, 0, "metadata retirement must not retry close");
        assert_eq!((*path.0).references, 1);
    }
}

#[test]
fn selected_resource_outlives_wrapper_and_closes_once_after_last_actual_use() {
    let fixture = Fixture::new("selected-resource-pin");
    fs::write(&fixture.path, b"original").unwrap();
    let path = RuntimeString::new(&fixture.path_bytes);
    let mut storage = ObjectStorage::new();
    let mut status = stale_status();
    assert_eq!(storage.open(&path, 0, 0, 0, &mut status), 0);
    unsafe {
        let origin = pending_source(storage.pointer).unwrap().origin;
        let handle = origin.handle().unwrap();
        let metadata = origin.metadata_pin().unwrap();
        let (([first, second], allocations), closes) = with_close_probe(|| {
            probe_runtime_allocations(|| {
                [
                    origin.selected_pin().unwrap(),
                    origin.selected_pin().unwrap(),
                ]
            })
        });
        assert_eq!((allocations, closes), (0, 0));
        let ((), closes) = with_close_probe(|| {
            storage.destroy();
            drop(storage);
        });
        assert_eq!(closes, 0, "active work retains the one real obligation");
        assert_eq!(origin.handle().unwrap(), handle);
        assert!(
            origin.selected_pin().is_err(),
            "an old source cannot select new work"
        );
        let ((), closes) = with_close_probe(|| drop(first));
        assert_eq!(closes, 0);
        let (file, facts) = origin.file().unwrap();
        let mut bytes = [0; 8];
        assert!(matches!(
            (*file).read(&mut bytes),
            Ok(crate::byte_io::ReadProgress::Data(8))
        ));
        assert_eq!(&bytes, b"original");
        assert_path(facts.path, &fixture.path_bytes);
        let ((), closes) = with_close_probe(|| drop(second));
        assert_eq!(
            closes, 1,
            "last active retirement performs exactly one close"
        );
        assert_eq!(origin.handle().unwrap_err().reason, Reason::Closed);
        assert_path(facts.path, &fixture.path_bytes);
        let ((), closes) = with_close_probe(|| drop(metadata));
        assert_eq!(closes, 0);
        assert_eq!((*path.0).references, 1);
    }
}
