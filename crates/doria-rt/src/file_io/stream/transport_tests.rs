use super::*;
use crate::platform_io::Reason;
use core::mem::MaybeUninit;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture {
    directory: PathBuf,
    path: PathBuf,
    path_bytes: Vec<u8>,
}

impl Fixture {
    fn new(name: &str) -> Self {
        static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "doria-file-transport-{}-{}-{}",
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

fn assert_invalid_input(status: &DrFileStatusV1) {
    assert_eq!(status.value, 0);
    assert_eq!(status.reason, Reason::InvalidInput as u8);
    assert_eq!(status.system_code, 0);
    assert_eq!(status.has_system_code, 0);
    assert_eq!(status.transferred_byte_count, 0);
    assert_eq!(status.has_transferred_byte_count, 0);
}

fn mark_storage(file: &mut MaybeUninit<OwnedFile>) {
    // These bytes remain uninitialized as an OwnedFile. Never form a typed
    // owner reference or invoke close/drop unless native open succeeds.
    unsafe {
        ptr::write_bytes(
            file.as_mut_ptr().cast::<u8>(),
            0xa5,
            mem::size_of::<OwnedFile>(),
        )
    }
}

fn assert_storage_unchanged(file: &MaybeUninit<OwnedFile>) {
    let bytes = unsafe {
        core::slice::from_raw_parts(file.as_ptr().cast::<u8>(), mem::size_of::<OwnedFile>())
    };
    assert!(bytes.iter().all(|byte| *byte == 0xa5));
}

#[test]
fn reported_storage_layout_matches_local_uninitialized_owner_storage() {
    let mut size = usize::MAX;
    let mut alignment = usize::MAX;
    unsafe { dr_v1_file_layout(&mut size, &mut alignment) };
    assert_eq!(size, mem::size_of::<MaybeUninit<OwnedFile>>());
    assert_eq!(alignment, mem::align_of::<MaybeUninit<OwnedFile>>());
    let mut file = MaybeUninit::<OwnedFile>::uninit();
    assert_eq!(file.as_mut_ptr() as usize % alignment, 0);
}

#[test]
fn raw_readiness_source_is_a_pure_loan_of_the_original_complete_owner() {
    let fixture = Fixture::new("source");
    fs::write(&fixture.path, b"unchanged").unwrap();
    for access in [0, 1, 2] {
        let mut file = MaybeUninit::<OwnedFile>::uninit();
        let mut status = stale_status();
        unsafe {
            assert_eq!(
                dr_v1_file_open(
                    fixture.path_bytes.as_ptr(),
                    fixture.path_bytes.len(),
                    access,
                    0,
                    0,
                    file.as_mut_ptr().cast(),
                    &mut status,
                ),
                0
            );
            let pointer = file.as_ptr().cast::<c_void>();
            let ((source, again, home, home_again), allocations) =
                crate::probe_runtime_allocations(|| {
                    (
                        dr_v1_file_readiness_source(pointer),
                        dr_v1_file_readiness_source(pointer),
                        dr_v1_file_readiness_source_home(pointer),
                        dr_v1_file_readiness_source_home(pointer),
                    )
                });
            assert_eq!(source, (*file.as_ptr()).readiness_source().value());
            assert_eq!(
                home,
                ((*file.as_ptr()).readiness_source()
                    as *const crate::native_handle::ReadinessSource)
                    .cast()
            );
            assert_eq!(*home.cast::<*const c_void>(), source);
            assert_eq!(home_again, home);
            assert_eq!(again, source);
            assert_eq!(allocations, 0);
            assert_eq!(
                dr_v1_file_control(file.as_mut_ptr().cast(), 4, 0, 0, &mut status),
                0
            );
            assert_eq!(
                status.value, 0,
                "forming a source must not change the cursor"
            );
            let (result, closes) = crate::native_handle::with_close_probe(|| {
                dr_v1_file_close(file.as_mut_ptr().cast(), &mut status)
            });
            assert_eq!(result, 0);
            assert_eq!(closes, 1, "the source adds no close obligation");
        }
    }
    assert_eq!(fs::read(&fixture.path).unwrap(), b"unchanged");
}

#[test]
fn readiness_source_has_one_stable_word_and_survives_unborrowed_owner_moves() {
    assert_eq!(
        mem::size_of::<crate::native_handle::ReadinessSource>(),
        mem::size_of::<usize>()
    );
    assert_eq!(
        mem::align_of::<crate::native_handle::ReadinessSource>(),
        mem::align_of::<usize>()
    );
    let fixture = Fixture::new("source-move");
    fs::write(&fixture.path, b"unchanged").unwrap();
    let owner = OwnedFile::open(&fixture.path_bytes, super::super::FileOpenRequest::READ).unwrap();
    let expected = owner.readiness_source().value();
    let owner = Box::new(owner);
    unsafe {
        let pointer = (&*owner as *const OwnedFile).cast();
        let home = dr_v1_file_readiness_source_home(pointer);
        assert_eq!(*home.cast::<*const c_void>(), expected);
        assert_eq!(
            crate::native_handle::ReadinessSource::handle(expected).unwrap(),
            owner.handle.get()
        );
    }
    drop(owner);
    #[cfg(unix)]
    {
        let zero = crate::native_handle::ReadinessSource::new(0);
        assert!(!zero.value().is_null());
        assert_eq!(
            crate::native_handle::ReadinessSource::handle(zero.value()).unwrap(),
            0
        );
    }
}

#[test]
fn failed_open_leaves_uninitialized_storage_without_a_close_obligation() {
    let fixture = Fixture::new("missing");
    let mut file = MaybeUninit::<OwnedFile>::uninit();
    mark_storage(&mut file);
    let mut status = stale_status();
    let result = unsafe {
        dr_v1_file_open(
            fixture.path_bytes.as_ptr(),
            fixture.path_bytes.len(),
            0,
            0,
            0,
            file.as_mut_ptr().cast(),
            &mut status,
        )
    };
    assert_eq!(result, 3);
    assert_eq!(status.reason, Reason::NotFound as u8);
    assert_eq!(status.has_system_code, 1);
    assert_ne!(status.system_code, 0);
    assert_eq!(status.value, 0);
    assert_eq!(status.has_transferred_byte_count, 0);
    assert_eq!(status.transferred_byte_count, 0);
    assert_storage_unchanged(&file);
    assert!(!fixture.path.exists());

    status = stale_status();
    let invalid_path = b"bad\0path";
    assert_eq!(
        unsafe {
            dr_v1_file_open(
                invalid_path.as_ptr(),
                invalid_path.len(),
                0,
                0,
                0,
                file.as_mut_ptr().cast(),
                &mut status,
            )
        },
        3
    );
    assert_invalid_input(&status);
    assert_storage_unchanged(&file);
    // Neither failed call initialized an owner. MaybeUninit drops no owner.
}

#[test]
fn invalid_open_tags_and_requests_do_not_create_or_truncate_files() {
    let existing = Fixture::new("invalid-existing");
    let missing = Fixture::new("invalid-missing");
    fs::write(&existing.path, b"preserved\0\xff").unwrap();
    for (access, creation, append) in [(3, 4, 0), (2, 5, 0), (2, 4, 2), (0, 4, 0), (0, 0, 1)] {
        for fixture in [&existing, &missing] {
            let mut file = MaybeUninit::<OwnedFile>::uninit();
            mark_storage(&mut file);
            let mut status = stale_status();
            assert_eq!(
                unsafe {
                    dr_v1_file_open(
                        fixture.path_bytes.as_ptr(),
                        fixture.path_bytes.len(),
                        access,
                        creation,
                        append,
                        file.as_mut_ptr().cast(),
                        &mut status,
                    )
                },
                3
            );
            assert_invalid_input(&status);
            assert_storage_unchanged(&file);
        }
        assert_eq!(fs::read(&existing.path).unwrap(), b"preserved\0\xff");
        assert!(!missing.path.exists());
    }
}

#[test]
fn empty_null_regions_return_zero_progress_instead_of_eof() {
    let fixture = Fixture::new("empty");
    let mut file = MaybeUninit::<OwnedFile>::uninit();
    let mut status = stale_status();
    unsafe {
        assert_eq!(
            dr_v1_file_open(
                fixture.path_bytes.as_ptr(),
                fixture.path_bytes.len(),
                2,
                2,
                0,
                file.as_mut_ptr().cast(),
                &mut status,
            ),
            0
        );
        assert_clear_failure_facts(&status);
        status = stale_status();
        assert_eq!(
            dr_v1_file_read(file.as_mut_ptr().cast(), ptr::null_mut(), 0, &mut status),
            0
        );
        assert_eq!(status.value, 0);
        assert_clear_failure_facts(&status);
        status = stale_status();
        assert_eq!(
            dr_v1_file_write(file.as_mut_ptr().cast(), ptr::null(), 0, &mut status),
            0
        );
        assert_eq!(status.value, 0);
        assert_clear_failure_facts(&status);
        let mut destination = [0x77];
        status = stale_status();
        assert_eq!(
            dr_v1_file_read(
                file.as_mut_ptr().cast(),
                destination.as_mut_ptr(),
                1,
                &mut status
            ),
            2
        );
        assert_eq!(destination, [0x77]);
        assert_eq!(status.value, 0);
        assert_clear_failure_facts(&status);
        status = stale_status();
        assert_eq!(dr_v1_file_close(file.as_mut_ptr().cast(), &mut status), 0);
        assert_eq!(status.value, 0);
        assert_clear_failure_facts(&status);
        // Consuming close made this storage uninitialized; do not drop/reuse it.
    }
    assert!(fs::read(&fixture.path).unwrap().is_empty());
}

#[test]
fn readonly_raw_inspection_preserves_shared_owners_cursor_and_failure_fields() {
    let fixture = Fixture::new("readonly-inspection");
    fs::write(&fixture.path, b"abcdef").unwrap();
    let mut file = MaybeUninit::<OwnedFile>::uninit();
    let mut status = stale_status();
    unsafe {
        assert_eq!(
            dr_v1_file_open(
                fixture.path_bytes.as_ptr(),
                fixture.path_bytes.len(),
                2,
                0,
                0,
                file.as_mut_ptr().cast(),
                &mut status
            ),
            0
        );
        assert_eq!(
            dr_v1_file_control(file.as_mut_ptr().cast(), 3, 2, 0, &mut status),
            0
        );
        {
            let first = &*file.as_ptr();
            let second = &*file.as_ptr();
            let ((), allocations) = crate::probe_runtime_allocations(|| {
                for _ in 0..32 {
                    for (operation, expected) in [(4, 2), (5, 6)] {
                        status = stale_status();
                        assert_eq!(
                            dr_v1_file_inspect(file.as_ptr().cast(), operation, &mut status),
                            0
                        );
                        assert_eq!(status.value, expected);
                        assert_clear_failure_facts(&status);
                    }
                    for operation in [0, 1, 2, 3, 6, 255] {
                        status = stale_status();
                        assert_eq!(
                            dr_v1_file_inspect(file.as_ptr().cast(), operation, &mut status),
                            3
                        );
                        assert_invalid_input(&status);
                    }
                    assert_eq!(first.position().unwrap(), 2);
                    assert_eq!(second.length().unwrap(), 6);
                }
            });
            assert_eq!(allocations, 0);
        }
        assert_eq!(dr_v1_file_close(file.as_mut_ptr().cast(), &mut status), 0);
    }
    assert_eq!(fs::read(&fixture.path).unwrap(), b"abcdef");
}

#[test]
fn invalid_control_tags_preserve_cursor_and_file_contents() {
    let fixture = Fixture::new("control");
    fs::write(&fixture.path, b"abcdef").unwrap();
    let mut file = MaybeUninit::<OwnedFile>::uninit();
    let mut status = stale_status();
    unsafe {
        assert_eq!(
            dr_v1_file_open(
                fixture.path_bytes.as_ptr(),
                fixture.path_bytes.len(),
                2,
                0,
                0,
                file.as_mut_ptr().cast(),
                &mut status,
            ),
            0
        );
        assert_eq!(
            dr_v1_file_control(file.as_mut_ptr().cast(), 3, 2, 0, &mut status),
            0
        );
        assert_eq!(status.value, 2);
        for (operation, origin) in [(6, 0), (11, 0), (255, 0), (3, 3), (3, 255), (7, 2), (8, 2)] {
            status = stale_status();
            assert_eq!(
                dr_v1_file_control(file.as_mut_ptr().cast(), operation, -1, origin, &mut status),
                3
            );
            assert_invalid_input(&status);
            assert_eq!(
                dr_v1_file_control(file.as_mut_ptr().cast(), 4, 0, 0, &mut status),
                0
            );
            assert_eq!(status.value, 2);
            assert_clear_failure_facts(&status);
            assert_eq!(
                dr_v1_file_control(file.as_mut_ptr().cast(), 5, 0, 0, &mut status),
                0
            );
            assert_eq!(status.value, 6);
        }
        assert_eq!(dr_v1_file_close(file.as_mut_ptr().cast(), &mut status), 0);
    }
    assert_eq!(fs::read(&fixture.path).unwrap(), b"abcdef");
}

#[test]
fn finite_binary_roundtrip_borrows_regions_controls_and_consumes_close() {
    let fixture = Fixture::new("roundtrip");
    let mut file = MaybeUninit::<OwnedFile>::uninit();
    let mut status = stale_status();
    let mut source = [0x55, 0x55, 0, 0xff, 0x80, b'A', 0, b'B', 0x55];
    let expected = [0, 0xff, 0x80, b'A', 0, b'B'];
    unsafe {
        assert_eq!(
            dr_v1_file_open(
                fixture.path_bytes.as_ptr(),
                fixture.path_bytes.len(),
                2,
                2,
                0,
                file.as_mut_ptr().cast(),
                &mut status,
            ),
            0
        );
        let mut written = 0;
        for _ in 0..expected.len() {
            if written == expected.len() {
                break;
            }
            status = stale_status();
            assert_eq!(
                dr_v1_file_write(
                    file.as_mut_ptr().cast(),
                    source.as_ptr().add(2 + written),
                    expected.len() - written,
                    &mut status,
                ),
                0
            );
            let count = status.value as usize;
            assert!(count > 0 && count <= expected.len() - written);
            written += count;
            assert_clear_failure_facts(&status);
            assert_eq!(&source[2..8], &expected);
        }
        assert_eq!(written, expected.len());
        // The synchronous write retains no caller region after it returns.
        source.fill(0xcc);
        for operation in [0, 1, 2] {
            status = stale_status();
            assert_eq!(
                dr_v1_file_control(file.as_mut_ptr().cast(), operation, 0, 0, &mut status),
                0
            );
            assert_eq!(status.value, 0);
            assert_clear_failure_facts(&status);
        }
        for operation in [4, 5] {
            assert_eq!(
                dr_v1_file_control(file.as_mut_ptr().cast(), operation, 0, 0, &mut status),
                0
            );
            assert_eq!(status.value, expected.len() as u64);
            assert_clear_failure_facts(&status);
        }
        assert_eq!(
            dr_v1_file_control(file.as_mut_ptr().cast(), 3, -2, 1, &mut status),
            0
        );
        assert_eq!(status.value, 4);
        assert_eq!(
            dr_v1_file_control(file.as_mut_ptr().cast(), 3, -1, 2, &mut status),
            0
        );
        assert_eq!(status.value, 5);
        assert_eq!(
            dr_v1_file_control(file.as_mut_ptr().cast(), 3, 0, 0, &mut status),
            0
        );
        let mut destination = [0x66; 12];
        let mut initialized = 0;
        let mut eof = false;
        for _ in 0..=expected.len() {
            status = stale_status();
            let result = dr_v1_file_read(
                file.as_mut_ptr().cast(),
                destination.as_mut_ptr().add(2 + initialized),
                8 - initialized,
                &mut status,
            );
            assert_clear_failure_facts(&status);
            if result == 2 {
                assert_eq!(status.value, 0);
                eof = true;
                break;
            }
            assert_eq!(result, 0);
            let count = status.value as usize;
            assert!(count > 0 && count <= expected.len() - initialized);
            initialized += count;
            assert!(destination[2 + initialized..]
                .iter()
                .all(|byte| *byte == 0x66));
        }
        assert!(eof, "finite binary transport did not reach EOF");
        assert_eq!(initialized, expected.len());
        assert_eq!(&destination[..2], &[0x66; 2]);
        assert_eq!(&destination[2..8], &expected);
        assert_eq!(&destination[8..], &[0x66; 4]);
        status = stale_status();
        assert_eq!(dr_v1_file_close(file.as_mut_ptr().cast(), &mut status), 0);
        assert_eq!(status.value, 0);
        assert_clear_failure_facts(&status);
    }
    // Visibility and successful sync calls are not power-loss durability proof.
    assert_eq!(fs::read(&fixture.path).unwrap(), expected);
}

#[test]
fn failure_status_preserves_optional_zero_and_positive_facts_after_caller_reset() {
    for system_code in [None, Some(0), Some(-123)] {
        for transferred_byte_count in [None, Some(0), Some(17)] {
            let failure = Failure {
                reason: Reason::PermissionDenied,
                system_code,
                transferred_byte_count,
            };
            let mut status = stale_status();
            assert_eq!(status.has_system_code, 1);
            assert_eq!(status.has_transferred_byte_count, 1);
            // Each transport entry resets before fail; fail is not a separate
            // status-lifecycle owner and must preserve the exact supplied facts.
            status = DrFileStatusV1::default();
            assert_eq!(status.fail(failure), 3);
            assert_eq!(status.value, 0);
            assert_eq!(status.reason, Reason::PermissionDenied as u8);
            assert_eq!(status.has_system_code, u8::from(system_code.is_some()));
            assert_eq!(status.system_code, system_code.unwrap_or(0));
            assert_eq!(
                status.has_transferred_byte_count,
                u8::from(transferred_byte_count.is_some())
            );
            assert_eq!(
                status.transferred_byte_count,
                transferred_byte_count.unwrap_or(0)
            );
        }
    }
}
