use super::{FileAccess, FileCreation, FileOpenRequest, FileSharing, OwnedFile, SeekOrigin};
use crate::byte_io::{self, ReadProgress, WriteProgress};
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
            "doria-owned-file-{}-{}-{}",
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

    fn open(&self, request: FileOpenRequest) -> OwnedFile {
        OwnedFile::open(&self.path_bytes, request).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.directory);
    }
}

fn request(access: FileAccess, creation: FileCreation, append: bool) -> FileOpenRequest {
    FileOpenRequest {
        access,
        creation,
        append,
        sharing: FileSharing::ReadWrite,
    }
}

fn write_all(file: &mut OwnedFile, source: &[u8]) {
    byte_io::write_all(source.len(), |offset, length| {
        file.write(&source[offset..offset + length])
    })
    .unwrap();
}

#[test]
fn typed_creation_modes_preserve_or_replace_exact_file_contents() {
    let fixture = Fixture::new("creation");
    for access in [FileAccess::Read, FileAccess::Write, FileAccess::ReadWrite] {
        assert!(OwnedFile::open(
            &fixture.path_bytes,
            request(access, FileCreation::OpenExisting, false),
        )
        .is_err());
        assert!(!fixture.path.exists());
    }
    assert!(OwnedFile::open(
        &fixture.path_bytes,
        request(FileAccess::Write, FileCreation::TruncateExisting, false),
    )
    .is_err());
    assert!(!fixture.path.exists());

    let mut file = fixture.open(request(
        FileAccess::Write,
        FileCreation::OpenOrCreate,
        false,
    ));
    write_all(&mut file, b"preserved\0\xff");
    file.close().unwrap();
    fixture
        .open(request(
            FileAccess::ReadWrite,
            FileCreation::OpenOrCreate,
            false,
        ))
        .close()
        .unwrap();
    assert_eq!(fs::read(&fixture.path).unwrap(), b"preserved\0\xff");
    assert!(OwnedFile::open(
        &fixture.path_bytes,
        request(FileAccess::Write, FileCreation::CreateNew, false),
    )
    .is_err());
    assert_eq!(fs::read(&fixture.path).unwrap(), b"preserved\0\xff");

    let file = fixture.open(request(
        FileAccess::Write,
        FileCreation::TruncateExisting,
        false,
    ));
    assert_eq!(file.length().unwrap(), 0);
    file.close().unwrap();
    assert!(fs::read(&fixture.path).unwrap().is_empty());

    fs::write(&fixture.path, b"replace me").unwrap();
    let mut file = fixture.open(request(
        FileAccess::ReadWrite,
        FileCreation::CreateOrTruncate,
        false,
    ));
    assert_eq!(file.length().unwrap(), 0);
    write_all(&mut file, b"replacement");
    file.close().unwrap();
    assert_eq!(fs::read(&fixture.path).unwrap(), b"replacement");

    for creation in [FileCreation::CreateNew, FileCreation::CreateOrTruncate] {
        let fresh = Fixture::new("fresh");
        fresh
            .open(request(FileAccess::Write, creation, false))
            .close()
            .unwrap();
        assert!(fresh.path.is_file());
        assert!(fs::read(&fresh.path).unwrap().is_empty());
    }
}

#[test]
#[cfg(any(unix, windows))]
fn every_accepted_file_open_request_creates_a_noninheritable_owner() {
    #[cfg(windows)]
    unsafe extern "system" {
        fn GetHandleInformation(handle: crate::native_handle::NativeHandle, flags: *mut u32)
            -> i32;
    }
    #[cfg(windows)]
    const HANDLE_FLAG_INHERIT: u32 = 0x0000_0001;

    let mut request_classes = 0;
    let mut opened_handles = 0;
    for access in [
        FileAccess::Read,
        FileAccess::Write,
        FileAccess::ReadWrite,
        FileAccess::AppendOnly,
    ] {
        for creation in [
            FileCreation::OpenExisting,
            FileCreation::OpenOrCreate,
            FileCreation::CreateNew,
            FileCreation::TruncateExisting,
            FileCreation::CreateOrTruncate,
        ] {
            for append in [false, true] {
                if (access == FileAccess::Read
                    && (append || creation != FileCreation::OpenExisting))
                    || (access == FileAccess::AppendOnly
                        && (!append || creation != FileCreation::OpenOrCreate))
                {
                    continue;
                }
                request_classes += 1;
                let request = request(access, creation, append);
                for existing in [false, true] {
                    if (!existing
                        && matches!(
                            creation,
                            FileCreation::OpenExisting | FileCreation::TruncateExisting
                        ))
                        || (existing && creation == FileCreation::CreateNew)
                    {
                        continue;
                    }
                    let fixture = Fixture::new("inheritance");
                    if existing {
                        fs::write(&fixture.path, b"existing fixture").unwrap();
                    }
                    let file =
                        OwnedFile::open(&fixture.path_bytes, request).unwrap_or_else(|error| {
                            panic!("{request:?}, existing={existing}: {error:?}")
                        });
                    #[cfg(unix)]
                    {
                        let flags = unsafe { libc::fcntl(file.handle.get(), libc::F_GETFD) };
                        assert!(
                            flags >= 0,
                            "F_GETFD failed for {request:?}, existing={existing}"
                        );
                        assert_ne!(
                            flags & libc::FD_CLOEXEC,
                            0,
                            "inheritable descriptor for {request:?}, existing={existing}"
                        );
                    }
                    #[cfg(windows)]
                    {
                        let mut flags = u32::MAX;
                        assert_ne!(
                            unsafe { GetHandleInformation(file.handle.get(), &mut flags) },
                            0,
                            "GetHandleInformation failed for {request:?}, existing={existing}"
                        );
                        assert_eq!(
                            flags & HANDLE_FLAG_INHERIT,
                            0,
                            "inheritable handle for {request:?}, existing={existing}"
                        );
                    }
                    file.close().unwrap();
                    opened_handles += 1;
                }
            }
        }
    }
    assert_eq!(request_classes, 22);
    assert_eq!(opened_handles, 31);
}

#[test]
fn invalid_open_requests_neither_create_nor_mutate_files() {
    let existing = Fixture::new("invalid-existing");
    let missing = Fixture::new("invalid-missing");
    fs::write(&existing.path, b"must survive\0\xff").unwrap();
    let invalid = [
        request(FileAccess::Read, FileCreation::OpenOrCreate, false),
        request(FileAccess::Read, FileCreation::CreateNew, false),
        request(FileAccess::Read, FileCreation::TruncateExisting, false),
        request(FileAccess::Read, FileCreation::CreateOrTruncate, false),
        request(FileAccess::Read, FileCreation::OpenExisting, true),
        request(FileAccess::AppendOnly, FileCreation::OpenExisting, false),
        request(FileAccess::AppendOnly, FileCreation::OpenExisting, true),
        request(FileAccess::AppendOnly, FileCreation::OpenOrCreate, false),
        request(FileAccess::AppendOnly, FileCreation::CreateNew, false),
        request(FileAccess::AppendOnly, FileCreation::CreateNew, true),
        request(
            FileAccess::AppendOnly,
            FileCreation::TruncateExisting,
            false,
        ),
        request(FileAccess::AppendOnly, FileCreation::TruncateExisting, true),
        request(
            FileAccess::AppendOnly,
            FileCreation::CreateOrTruncate,
            false,
        ),
        request(FileAccess::AppendOnly, FileCreation::CreateOrTruncate, true),
    ];
    for invalid in invalid {
        assert!(OwnedFile::open(&existing.path_bytes, invalid).is_err());
        assert_eq!(fs::read(&existing.path).unwrap(), b"must survive\0\xff");
        assert!(OwnedFile::open(&missing.path_bytes, invalid).is_err());
        assert!(!missing.path.exists());
    }
}

#[test]
fn access_modes_reject_forbidden_transfers_without_mutating_the_file() {
    let fixture = Fixture::new("access");
    fs::write(&fixture.path, b"unchanged\0\xff").unwrap();
    let mut reader = fixture.open(FileOpenRequest::READ);
    assert!(reader.write(b"must not replace").is_err());
    assert_eq!(reader.position().unwrap(), 0);
    reader.close().unwrap();
    let mut writer = fixture.open(request(
        FileAccess::Write,
        FileCreation::OpenExisting,
        false,
    ));
    let mut destination = [0x77; 3];
    assert!(writer.read(&mut destination).is_err());
    assert_eq!(destination, [0x77; 3]);
    assert_eq!(writer.position().unwrap(), 0);
    writer.close().unwrap();
    assert_eq!(fs::read(&fixture.path).unwrap(), b"unchanged\0\xff");
}

#[test]
fn binary_regions_remain_borrowed_and_empty_reads_are_not_eof() {
    let fixture = Fixture::new("regions");
    let mut file = fixture.open(request(
        FileAccess::ReadWrite,
        FileCreation::CreateNew,
        false,
    ));
    let source = [0x55, 0x55, 0, 0xff, 0x80, b'A', 0, b'B', 0x55];
    let original = source;
    assert_eq!(file.write(&[]).unwrap(), WriteProgress::Accepted(0));
    assert_eq!(file.position().unwrap(), 0);
    write_all(&mut file, &source[2..8]);
    assert_eq!(source, original);
    assert_eq!(file.seek(0, SeekOrigin::Start).unwrap(), 0);
    assert_eq!(file.read(&mut []).unwrap(), ReadProgress::Data(0));
    assert_eq!(file.position().unwrap(), 0);

    let mut destination = [0x66; 12];
    let mut initialized = 0;
    let mut eof = false;
    for _ in 0..=source[2..8].len() {
        match file.read(&mut destination[2 + initialized..10]).unwrap() {
            ReadProgress::Data(count) => {
                assert!(count > 0 && count <= 8 - initialized);
                initialized += count;
                assert!(destination[2 + initialized..]
                    .iter()
                    .all(|byte| *byte == 0x66));
            }
            ReadProgress::EndOfStream => {
                eof = true;
                break;
            }
            ReadProgress::WouldBlock(failure) => panic!("regular file would block: {failure:?}"),
        }
    }
    assert!(eof, "finite binary fixture did not reach EOF");
    assert_eq!(initialized, 6);
    assert_eq!(&destination[..2], &[0x66; 2]);
    assert_eq!(&destination[2..8], &source[2..8]);
    assert_eq!(&destination[8..], &[0x66; 4]);
    assert_eq!(file.read(&mut []).unwrap(), ReadProgress::Data(0));
    assert_eq!(file.write(&[]).unwrap(), WriteProgress::Accepted(0));
    assert_eq!(file.position().unwrap(), 6);
    assert_eq!(file.length().unwrap(), 6);
    assert_eq!(source, original);
    file.close().unwrap();
    assert_eq!(fs::read(&fixture.path).unwrap(), source[2..8]);
}

#[test]
fn seek_position_and_length_preserve_state_after_failed_operations() {
    let fixture = Fixture::new("seek");
    fs::write(&fixture.path, b"abcdef").unwrap();
    let mut file = fixture.open(request(
        FileAccess::ReadWrite,
        FileCreation::OpenExisting,
        false,
    ));
    assert_eq!(file.position().unwrap(), 0);
    assert_eq!(file.length().unwrap(), 6);
    assert_eq!(file.position().unwrap(), 0);
    assert_eq!(file.seek(2, SeekOrigin::Start).unwrap(), 2);
    let mut bytes = [0; 2];
    let count = match file.read(&mut bytes).unwrap() {
        ReadProgress::Data(count) => count,
        outcome => panic!("expected file data: {outcome:?}"),
    };
    assert!(count > 0 && count <= bytes.len());
    assert_eq!(&bytes[..count], &b"cd"[..count]);
    assert_eq!(file.position().unwrap(), 2 + count as u64);
    assert_eq!(
        file.seek(-1, SeekOrigin::Current).unwrap(),
        1 + count as u64
    );
    assert_eq!(file.seek(-2, SeekOrigin::End).unwrap(), 4);
    write_all(&mut file, b"Z");
    assert_eq!(file.position().unwrap(), 5);
    assert_eq!(file.length().unwrap(), 6);
    for (offset, origin) in [
        (-1, SeekOrigin::Start),
        (-100, SeekOrigin::Current),
        (i64::MAX, SeekOrigin::Current),
    ] {
        assert!(file.seek(offset, origin).is_err());
        assert_eq!(file.position().unwrap(), 5);
        assert_eq!(file.length().unwrap(), 6);
    }
    assert_eq!(file.seek(9, SeekOrigin::Start).unwrap(), 9);
    assert_eq!(file.read(&mut bytes).unwrap(), ReadProgress::EndOfStream);
    assert_eq!(file.position().unwrap(), 9);
    write_all(&mut file, b"!");
    assert_eq!(file.position().unwrap(), 10);
    assert_eq!(file.length().unwrap(), 10);
    file.close().unwrap();
    assert_eq!(fs::read(&fixture.path).unwrap(), b"abcdZf\0\0\0!");
}

#[test]
fn append_writes_remain_at_end_after_explicit_seeks() {
    for access in [FileAccess::Write, FileAccess::ReadWrite] {
        let fixture = Fixture::new("append");
        fs::write(&fixture.path, b"abc").unwrap();
        let mut file = fixture.open(request(access, FileCreation::OpenExisting, true));
        assert_eq!(file.seek(0, SeekOrigin::Start).unwrap(), 0);
        write_all(&mut file, b"\0");
        assert_eq!(file.position().unwrap(), 4);
        assert_eq!(file.seek(1, SeekOrigin::Start).unwrap(), 1);
        write_all(&mut file, b"\xff");
        assert_eq!(file.position().unwrap(), 5);
        assert_eq!(file.length().unwrap(), 5);
        file.close().unwrap();
        assert_eq!(fs::read(&fixture.path).unwrap(), b"abc\0\xff");
    }
    let fresh = Fixture::new("append-create");
    let mut file = fresh.open(request(FileAccess::Write, FileCreation::OpenOrCreate, true));
    write_all(&mut file, b"created");
    file.close().unwrap();
    assert_eq!(fs::read(&fresh.path).unwrap(), b"created");
}

#[test]
fn independent_public_file_owners_share_read_and_write_access() {
    let fixture = Fixture::new("sharing");
    fs::write(&fixture.path, b"abc").unwrap();
    for first_access in [FileAccess::Read, FileAccess::Write, FileAccess::ReadWrite] {
        for second_access in [FileAccess::Read, FileAccess::Write, FileAccess::ReadWrite] {
            let first = fixture.open(request(first_access, FileCreation::OpenExisting, false));
            let mut second =
                fixture.open(request(second_access, FileCreation::OpenExisting, false));
            assert_eq!(first.position().unwrap(), 0);
            assert_eq!(second.seek(1, SeekOrigin::Start).unwrap(), 1);
            assert_eq!(first.position().unwrap(), 0);
            second.close().unwrap();
            first.close().unwrap();
        }
    }
}

#[test]
#[cfg(windows)]
fn legacy_file_helpers_keep_their_existing_windows_sharing_contracts() {
    let fixture = Fixture::new("legacy-sharing");
    fs::write(&fixture.path, b"abc").unwrap();
    let reader = fixture.open(FileOpenRequest {
        sharing: FileSharing::ReadOnly,
        ..FileOpenRequest::READ
    });
    fixture.open(FileOpenRequest::READ).close().unwrap();
    assert!(OwnedFile::open(
        &fixture.path_bytes,
        request(FileAccess::Write, FileCreation::OpenExisting, false),
    )
    .is_err());
    reader.close().unwrap();
    for access in [FileAccess::Write, FileAccess::AppendOnly] {
        let writer = fixture.open(FileOpenRequest {
            sharing: FileSharing::Exclusive,
            ..request(
                access,
                FileCreation::OpenOrCreate,
                access == FileAccess::AppendOnly,
            )
        });
        assert!(OwnedFile::open(&fixture.path_bytes, FileOpenRequest::READ).is_err());
        writer.close().unwrap();
    }
}

#[test]
fn duplex_append_advances_to_current_end_but_empty_write_preserves_cursor() {
    let fixture = Fixture::new("duplex-append");
    fs::write(&fixture.path, b"abc").unwrap();
    let mut first = fixture.open(request(
        FileAccess::ReadWrite,
        FileCreation::OpenExisting,
        true,
    ));
    let mut second = fixture.open(request(
        FileAccess::ReadWrite,
        FileCreation::OpenExisting,
        true,
    ));
    let mut bytes = [0; 1];
    assert_eq!(first.seek(1, SeekOrigin::Start).unwrap(), 1);
    assert_eq!(first.write(&[]).unwrap(), WriteProgress::Accepted(0));
    assert_eq!(first.position().unwrap(), 1);
    assert_eq!(first.length().unwrap(), 3);
    assert_eq!(first.read(&mut bytes).unwrap(), ReadProgress::Data(1));
    assert_eq!(bytes, *b"b");
    write_all(&mut second, b"X");
    assert_eq!(second.position().unwrap(), 4);
    assert_eq!(first.position().unwrap(), 2);
    assert_eq!(second.read(&mut bytes).unwrap(), ReadProgress::EndOfStream);
    write_all(&mut first, b"Y");
    assert_eq!(first.position().unwrap(), 5);
    assert_eq!(first.read(&mut bytes).unwrap(), ReadProgress::EndOfStream);
    first.close().unwrap();
    second.close().unwrap();
    assert_eq!(fs::read(&fixture.path).unwrap(), b"abcXY");
}

#[test]
fn flush_and_explicit_sync_operations_preserve_position_and_visible_bytes() {
    let fixture = Fixture::new("sync");
    let mut file = fixture.open(request(
        FileAccess::ReadWrite,
        FileCreation::CreateNew,
        false,
    ));
    write_all(&mut file, b"visible\0\xff");
    let position = file.position().unwrap();
    file.flush().unwrap();
    assert_eq!(file.position().unwrap(), position);
    file.sync_data().unwrap();
    assert_eq!(file.position().unwrap(), position);
    file.sync_metadata().unwrap();
    assert_eq!(file.position().unwrap(), position);
    assert_eq!(file.length().unwrap(), position);
    file.close().unwrap();
    // Readback proves bytes and successful calls, not power-loss durability or
    // that an unbuffered flush issued no synchronization syscall.
    assert_eq!(fs::read(&fixture.path).unwrap(), b"visible\0\xff");
}

#[test]
fn finite_binary_copy_reuses_one_stack_buffer_and_handles_the_short_tail() {
    for length in [0, 8_195] {
        let input = Fixture::new("copy-input");
        let output = Fixture::new("copy-output");
        let expected: Vec<u8> = (0..length).map(|index| (index % 251) as u8).collect();
        fs::write(&input.path, &expected).unwrap();
        let ((), costs) = crate::io_cost_probe::measure(|| {
            let mut reader = input.open(FileOpenRequest::READ);
            let mut writer =
                output.open(request(FileAccess::Write, FileCreation::CreateNew, false));
            let mut buffer = [0; 31];
            let address = buffer.as_ptr();
            let mut transferred = 0;
            let mut eof = false;
            let (_, allocations) = crate::probe_runtime_allocations(|| {
                for _ in 0..=expected.len() {
                    assert_eq!(buffer.as_ptr(), address);
                    match reader.read(&mut buffer).unwrap() {
                        ReadProgress::Data(count) => {
                            assert!(count > 0 && count <= buffer.len());
                            assert!(transferred + count <= expected.len());
                            assert_eq!(
                                &buffer[..count],
                                &expected[transferred..transferred + count]
                            );
                            write_all(&mut writer, &buffer[..count]);
                            transferred += count;
                        }
                        ReadProgress::EndOfStream => {
                            eof = true;
                            break;
                        }
                        ReadProgress::WouldBlock(failure) => {
                            panic!("regular file would block: {failure:?}")
                        }
                    }
                }
                assert!(eof, "finite streaming copy did not reach EOF");
                assert_eq!(transferred, expected.len());
                assert_eq!(reader.position().unwrap(), expected.len() as u64);
                assert_eq!(writer.length().unwrap(), expected.len() as u64);
                writer.flush().unwrap();
            });
            assert_eq!(
                allocations, 0,
                "steady-state file copy allocated runtime storage"
            );
            reader.close().unwrap();
            writer.close().unwrap();
        });
        assert_eq!(
            costs, [0; 6],
            "synchronous open/transfer/inspection/flush/close must not access timing, readiness, helpers or cancellation"
        );
        assert_eq!(fs::read(&output.path).unwrap(), expected);
    }
}

#[test]
fn dropping_an_owned_file_releases_it_for_reopening() {
    let fixture = Fixture::new("drop");
    {
        let mut file = fixture.open(request(FileAccess::Write, FileCreation::CreateNew, false));
        write_all(&mut file, b"drop closes");
    }
    let mut reopened = fixture.open(request(
        FileAccess::Write,
        FileCreation::TruncateExisting,
        false,
    ));
    write_all(&mut reopened, b"reopened");
    reopened.close().unwrap();
    assert_eq!(fs::read(&fixture.path).unwrap(), b"reopened");
}

#[test]
fn allocation_probe_observes_the_actual_runtime_owner_and_open_setup() {
    let (_, allocations) = crate::probe_runtime_allocations(|| unsafe {
        let memory = crate::allocate(7);
        assert!(!memory.is_null());
        crate::deallocate(memory);
    });
    assert_eq!(allocations, 1);
    let fixture = Fixture::new("allocation-open");
    let (file, allocations) = crate::probe_runtime_allocations(|| {
        fixture.open(request(FileAccess::Write, FileCreation::CreateNew, false))
    });
    assert_eq!(
        allocations, 1,
        "opening allocates only native path conversion storage"
    );
    let (_, allocations) = crate::probe_runtime_allocations(|| file.close().unwrap());
    assert_eq!(allocations, 0);
}
