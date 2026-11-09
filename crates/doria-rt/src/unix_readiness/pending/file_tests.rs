//! Real regular-file leaves through the same native request/group lifecycle.
//! These fixtures do not assert public File pending-capability conformance.

use super::*;
use crate::file_io::stream::{
    FileAccess, FileCreation, FileOpenRequest, FileSharing, OwnedFile, SeekOrigin,
};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

struct FileFixture {
    directory: PathBuf,
    path: PathBuf,
}

impl FileFixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "doria-pending-file-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("file-Dória.bin");
        Self { directory, path }
    }

    fn open(&self, access: FileAccess, append: bool) -> Box<OwnedFile> {
        Box::new(
            OwnedFile::open(
                self.path.to_str().unwrap().as_bytes(),
                FileOpenRequest {
                    access,
                    creation: FileCreation::OpenExisting,
                    append,
                    sharing: FileSharing::ReadWrite,
                },
            )
            .unwrap(),
        )
    }
}

impl Drop for FileFixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.directory);
    }
}

fn endpoint(file: &mut OwnedFile) -> *const c_void {
    ptr::from_mut(file).cast_const().cast()
}

#[test]
fn one_file_shared_cursor_orders_read_write_read_and_reuse_through_complete_adapter() {
    let fixture = FileFixture::new();
    fs::write(&fixture.path, b"abcdefghi").unwrap();
    let mut file = fixture.open(FileAccess::ReadWrite, false);
    let log = Log::new();
    let context = Context::new(0, endpoint(&mut file), &log);
    let mut set = unsafe { PendingSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add_file(1, ptr::from_mut(file.as_mut()), context.adapter())
            .unwrap();
    }
    let (mut a, mut c) = ([0xa5; 3], [0xc5; 2]);
    let buffers = BufferList::from_iter(
        [
            Buffer::Read(&mut a),
            Buffer::Write(b"XY"),
            Buffer::Read(&mut c),
        ]
        .into_iter(),
    )
    .unwrap();
    let wait = budget(1);
    let mut group = unsafe {
        set.begin_selected(ptr::null(), buffers, &wait, selection(&[0; 3]))
            .unwrap()
    };
    assert!(matches!(
        log.borrow().calls(),
        [
            Call::Prepare { .. },
            Call::Prepare { .. },
            Call::Prepare { .. }
        ]
    ));
    assert_eq!(
        fs::read(&fixture.path).unwrap(),
        b"abcdefghi",
        "queued write has not changed the shared cursor or file"
    );
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        assert_eq!(group.next(ptr::null()).unwrap().index, 0);
        assert_eq!(group.next(ptr::null()).unwrap().index, 1);
        assert_eq!(group.next(ptr::null()).unwrap().index, 2);
        assert_eq!(group.completed(0).unwrap(), b"abc");
        assert_eq!(group.completed(2).unwrap(), b"fg");
        group.submit(ptr::null(), 0, 0, 2).unwrap();
        assert_eq!(group.next(ptr::null()).unwrap().index, 0);
        assert_eq!(group.completed(0).unwrap(), b"hi");
        assert!(group.next(ptr::null()).is_none());
        drop(group);
    });
    assert_eq!(allocations, 0);
    assert_eq!(file.position().unwrap(), 9);
    assert_eq!(fs::read(&fixture.path).unwrap(), b"abcXYfghi");
    assert_eq!(
        log.borrow()
            .calls()
            .iter()
            .filter(|call| matches!(call, Call::Complete { .. }))
            .count(),
        4
    );
    set.remove(1).unwrap();
}

#[test]
fn owned_file_cursors_stay_independent_and_reusable_without_hot_allocations_or_extra_closes() {
    let fixture = FileFixture::new();
    fs::write(&fixture.path, b"abcdefgh").unwrap();
    let mut first = fixture.open(FileAccess::Read, false);
    let mut second = fixture.open(FileAccess::Read, false);
    second.seek(4, SeekOrigin::Start).unwrap();
    let handles = [first.native_handle(), second.native_handle()];
    let log = Log::new();
    let first_context = Context::new(0, endpoint(&mut first), &log);
    unsafe { (*first_context.as_ptr()).selected_length = Some(2) };
    unsafe { (*first_context.as_ptr()).transform = true };
    let second_context = Context::new(1, endpoint(&mut second), &log);
    let mut set = unsafe { PendingSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add_file(1, ptr::from_mut(first.as_mut()), first_context.adapter())
            .unwrap();
        set.add_file(2, ptr::from_mut(second.as_mut()), second_context.adapter())
            .unwrap();
    }
    let original = [
        set.native.as_mut().unwrap().pending_snapshot(0).unwrap(),
        set.native.as_mut().unwrap().pending_snapshot(1).unwrap(),
    ];
    let current = frame();
    let poll = budget(0);
    let (mut a, mut b) = ([0x11; 4], [0x22; 4]);
    let caller_b = b.as_mut_ptr() as usize;
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut a), Buffer::Read(&mut b)].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(&current, buffers, &poll, selection(&[1, 0]))
            .unwrap()
    };
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        let first_ready = group.next(&current).unwrap();
        let second_ready = group.next(&current).unwrap();
        assert_eq!((first_ready.key, second_ready.key), (2, 1));
        assert!(matches!(
            first_ready.outcome.transfer,
            Some(AdapterTerminal::Data(4))
        ));
        assert!(matches!(
            second_ready.outcome.transfer,
            Some(AdapterTerminal::Data(2))
        ));
        assert_eq!(group.completed(0).unwrap(), b"efgh");
        assert_eq!(group.completed(1).unwrap(), &[b'a' ^ 0xff, b'b' ^ 0xff]);
        assert!(group.next(&current).is_none());
        group.submit(&current, 1, 2, 2).unwrap();
        let reused = group.next(&current).unwrap();
        assert_eq!((reused.index, reused.key), (1, 1));
        assert!(matches!(
            reused.outcome.transfer,
            Some(AdapterTerminal::Data(2))
        ));
        assert_eq!(group.completed(1).unwrap(), &[b'c' ^ 0xff, b'd' ^ 0xff]);
        assert!(group.next(&current).is_none());
    });
    assert_eq!(
        allocations, 0,
        "counts runtime allocations, not kernel resources"
    );
    drop(group);
    assert_eq!(a, *b"efgh");
    assert_eq!(b, [b'a' ^ 0xff, b'b' ^ 0xff, b'c' ^ 0xff, b'd' ^ 0xff]);
    assert!(log.borrow().calls().iter().any(|call| matches!(call,
        Call::Prepare { index: 0, caller, length: 2, .. } if *caller == caller_b + 2)));
    for (index, before) in original.into_iter().enumerate() {
        let after = set
            .native
            .as_mut()
            .unwrap()
            .pending_snapshot(index)
            .unwrap();
        assert_eq!((after.0, after.1), (before.0, before.1));
        assert_eq!((after.2, after.3), (0, true));
    }
    let mut bytes = [0; 2];
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(&current, buffers, &poll, selection(&[0]))
            .unwrap()
    };
    assert!(matches!(
        unsafe { group.next(&current).unwrap() }.outcome.transfer,
        Some(AdapterTerminal::Data(2))
    ));
    drop(group);
    assert_eq!(bytes, [b'e' ^ 0xff, b'f' ^ 0xff]);
    let ((), closes) = crate::native_handle::with_close_probe(|| drop(set));
    assert_eq!(
        closes, 1,
        "the set owns only its watcher, never file handles"
    );
    assert_eq!(first.position().unwrap(), 6);
    assert_eq!(second.position().unwrap(), 8);
    for handle in handles {
        assert!(unsafe { libc::fcntl(handle, libc::F_GETFD) } >= 0);
    }
    let ((), closes) = crate::native_handle::with_close_probe(|| {
        drop(second);
        drop(first);
    });
    assert_eq!(closes, 2, "each complete file owner closes exactly once");
}

#[test]
fn owned_file_submission_preserves_append_and_empty_write_without_query_repeating_io() {
    let fixture = FileFixture::new();
    fs::write(&fixture.path, b"prefix").unwrap();
    let mut file = fixture.open(FileAccess::ReadWrite, true);
    file.seek(0, SeekOrigin::Start).unwrap();
    let log = Log::new();
    let context = Context::new(0, endpoint(&mut file), &log);
    let mut set = unsafe { PendingSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add_file(1, ptr::from_mut(file.as_mut()), context.adapter())
            .unwrap();
    }
    let current = frame();
    let poll = budget(0);
    let buffers = BufferList::from_iter([Buffer::Write(b"AB")].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(&current, buffers, &poll, selection(&[0]))
            .unwrap()
    };
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        assert!(matches!(
            group.next(&current).unwrap().outcome.transfer,
            Some(AdapterTerminal::Accepted(2))
        ));
        assert!(group.next(&current).is_none());
        group.submit(&current, 0, 1, 1).unwrap();
        assert!(matches!(
            group.next(&current).unwrap().outcome.transfer,
            Some(AdapterTerminal::Accepted(1))
        ));
        group.submit(&current, 0, 0, 0).unwrap();
        assert!(matches!(
            group.next(&current).unwrap().outcome.transfer,
            Some(AdapterTerminal::Accepted(0))
        ));
        assert!(group.next(&current).is_none());
    });
    assert_eq!(allocations, 0);
    drop(group);
    assert_eq!(fs::read(&fixture.path).unwrap(), b"prefixABB");
    assert_eq!(
        set.native.as_mut().unwrap().pending_snapshot(0).unwrap().2,
        0
    );
    assert_eq!(
        log.borrow()
            .calls()
            .iter()
            .filter(|call| matches!(
                call,
                Call::Complete {
                    tag: 0,
                    submitted: 1,
                    ..
                }
            ))
            .count(),
        3
    );
    drop(set);
    assert_eq!(file.position().unwrap(), 9);
}

#[test]
fn owned_file_empty_read_and_eof_are_distinct_and_cancellation_preserves_won_progress() {
    let fixture = FileFixture::new();
    fs::write(&fixture.path, b"x").unwrap();
    let mut file = fixture.open(FileAccess::Read, false);
    let cancellation = crate::cancellation::dr_v1_cancellation_new();
    assert!(!cancellation.is_null());
    let log = Log::new();
    let context = Context::new(0, endpoint(&mut file), &log);
    let mut set = unsafe { PendingSet::new(cancellation).unwrap() };
    unsafe {
        set.add_file(1, ptr::from_mut(file.as_mut()), context.adapter())
            .unwrap();
    }
    let current = frame();
    let indefinite = budget(1);
    let mut bytes = [0x77; 1];
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(&current, buffers, &indefinite, selection(&[0]))
            .unwrap()
    };
    assert!(matches!(
        unsafe { group.next(&current).unwrap() }.outcome.transfer,
        Some(AdapterTerminal::Data(1))
    ));
    unsafe { group.submit(&current, 0, 0, 0).unwrap() };
    assert!(matches!(
        unsafe { group.next(&current).unwrap() }.outcome.transfer,
        Some(AdapterTerminal::Data(0))
    ));
    unsafe { group.submit(&current, 0, 0, 1).unwrap() };
    assert!(matches!(
        unsafe { group.next(&current).unwrap() }.outcome.transfer,
        Some(AdapterTerminal::EndOfStream)
    ));
    drop(group);
    assert_eq!(bytes, [b'x']);
    assert!(log.borrow().calls().iter().any(|call| matches!(
        call,
        Call::Complete {
            tag: 2,
            submitted: 1,
            count: 0,
            selected: (_, _, 1),
            ..
        }
    )));
    assert!(log.borrow().calls().iter().any(|call| matches!(
        call,
        Call::Complete {
            tag: 0,
            submitted: 1,
            count: 0,
            selected: (_, _, 0),
            ..
        }
    )));
    drop(set);
    assert_eq!(file.position().unwrap(), 1);
    unsafe { crate::cancellation::dr_v1_cancellation_release(cancellation) };

    // A second group observes a syscall that won before cancellation. Draining
    // must preserve it rather than retry, replace, or invent a zero-byte error.
    let mut file = fixture.open(FileAccess::Read, false);
    let cancellation = crate::cancellation::dr_v1_cancellation_new();
    assert!(!cancellation.is_null());
    let log = Log::new();
    let context = Context::new(0, endpoint(&mut file), &log);
    let mut set = unsafe { PendingSet::new(cancellation).unwrap() };
    unsafe {
        set.add_file(1, ptr::from_mut(file.as_mut()), context.adapter())
            .unwrap();
    }
    let mut bytes = [0x88; 1];
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(&current, buffers, &indefinite, selection(&[0]))
            .unwrap()
    };
    unsafe { request_cancellation(cancellation) };
    let completion = unsafe { group.next(&current).unwrap() };
    assert!(matches!(
        completion.outcome.transfer,
        Some(AdapterTerminal::Data(1))
    ));
    assert_eq!(completion.outcome.stop, Some(Stop::Cancellation));
    drop(group);
    assert_eq!(bytes, [b'x']);
    assert!(matches!(
        log.borrow().calls()[1],
        Call::Complete {
            tag: 0,
            submitted: 1,
            count: 1,
            progress: None,
            ..
        }
    ));
    drop(set);
    assert_eq!(file.position().unwrap(), 1);
    unsafe { crate::cancellation::dr_v1_cancellation_release(cancellation) };
}

#[test]
fn owned_file_zero_selected_region_is_data_not_eof_for_a_nonempty_outer_request() {
    let fixture = FileFixture::new();
    fs::write(&fixture.path, b"stay").unwrap();
    let mut file = fixture.open(FileAccess::Read, false);
    let log = Log::new();
    let context = Context::new(0, endpoint(&mut file), &log);
    unsafe { (*context.as_ptr()).selected_length = Some(0) };
    let mut set = unsafe { PendingSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add_file(1, ptr::from_mut(file.as_mut()), context.adapter())
            .unwrap();
    }
    let current = frame();
    let poll = budget(0);
    let mut bytes = [0x44; 4];
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(&current, buffers, &poll, selection(&[0]))
            .unwrap()
    };
    assert!(matches!(
        unsafe { group.next(&current).unwrap() }.outcome.transfer,
        Some(AdapterTerminal::Data(0))
    ));
    drop(group);
    assert_eq!(bytes, [0x44; 4]);
    assert!(matches!(
        log.borrow().calls()[1],
        Call::Complete {
            tag: 0,
            submitted: 1,
            count: 0,
            selected: (_, _, 0),
            ..
        }
    ));
    drop(set);
    assert_eq!(file.position().unwrap(), 0);
    assert_eq!(fs::read(&fixture.path).unwrap(), b"stay");
}

#[test]
fn owned_file_failure_keeps_the_original_checked_carrier_and_actual_submission_fact() {
    let fixture = FileFixture::new();
    fs::write(&fixture.path, b"unchanged").unwrap();
    let mut file = fixture.open(FileAccess::Read, false);
    let actual_failure = file.write(b"no").unwrap_err();
    let descriptor = descriptor();
    let drops = Rc::new(Cell::new(0));
    let (error, original) = exact_error(&descriptor, &drops);
    let log = Log::new();
    let context = Context::new(0, endpoint(&mut file), &log);
    unsafe { (*context.as_ptr()).failure_error = Some(error) };
    let mut set = unsafe { PendingSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add_file(1, ptr::from_mut(file.as_mut()), context.adapter())
            .unwrap();
    }
    let current = frame();
    let poll = budget(0);
    let buffers = BufferList::from_iter([Buffer::Write(b"no")].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(&current, buffers, &poll, selection(&[0]))
            .unwrap()
    };
    let completion = unsafe { group.next(&current).unwrap() };
    let Some(AdapterTerminal::CheckedError {
        error,
        progress: None,
    }) = completion.outcome.transfer
    else {
        panic!("original owned checked failure required")
    };
    let carrier = error.into_carrier();
    assert_eq!(carrier.object, original);
    assert_eq!(carrier.vtable, ptr::from_ref(&descriptor));
    assert!(unsafe { group.next(&current) }.is_none());
    drop(group);
    assert!(matches!(log.borrow().calls()[1], Call::Complete {
        tag: 3, submitted: 1, progress, ..
    } if progress == actual_failure.transferred_byte_count));
    drop(set);
    assert_eq!(file.position().unwrap(), 0);
    assert_eq!(fs::read(&fixture.path).unwrap(), b"unchanged");
    assert_eq!(drops.get(), 0);
    unsafe { ((*carrier.vtable).drop)(ptr::null(), carrier.object) };
    assert_eq!(drops.get(), 1);
}

#[test]
fn owned_file_before_submit_cancellation_completes_cleanup_without_touching_the_file() {
    let fixture = FileFixture::new();
    fs::write(&fixture.path, b"stay").unwrap();
    let mut file = fixture.open(FileAccess::Read, false);
    let cancellation = crate::cancellation::dr_v1_cancellation_new();
    assert!(!cancellation.is_null());
    let log = Log::new();
    let context = Context::new(0, endpoint(&mut file), &log);
    unsafe { (*context.as_ptr()).cancel_on_prepare = cancellation };
    unsafe { (*context.as_ptr()).selected_length = Some(3) };
    let selected = unsafe { (*context.as_ptr()).storage.as_ptr() } as usize;
    let mut set = unsafe { PendingSet::new(cancellation).unwrap() };
    unsafe {
        set.add_file(1, ptr::from_mut(file.as_mut()), context.adapter())
            .unwrap();
    }
    let current = frame();
    let indefinite = budget(1);
    let mut bytes = [0x55; 4];
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(&current, buffers, &indefinite, selection(&[0]))
            .unwrap()
    };
    let stopped = unsafe { group.next(&current).unwrap() };
    assert!(stopped.outcome.transfer.is_none());
    assert_eq!(stopped.outcome.stop, Some(Stop::Cancellation));
    drop(group);
    assert_eq!(bytes, [0x55; 4]);
    assert_eq!(log.borrow().calls().len(), 2);
    assert!(matches!(log.borrow().calls()[1], Call::Complete {
        tag: 4, submitted: 0, progress: None, selected: (_, data, 3), ..
    } if data == selected));
    assert_eq!(
        set.native.as_mut().unwrap().pending_snapshot(0).unwrap().2,
        0
    );
    drop(set);
    assert_eq!(file.position().unwrap(), 0);
    assert_eq!(fs::read(&fixture.path).unwrap(), b"stay");
    unsafe { crate::cancellation::dr_v1_cancellation_release(cancellation) };
}

#[test]
fn owned_file_and_nonblocking_stream_share_selected_requests_without_touching_unused_file() {
    let fixture = FileFixture::new();
    fs::write(&fixture.path, b"file").unwrap();
    let mut file = fixture.open(FileAccess::Read, false);
    let mut unused_file = fixture.open(FileAccess::Read, false);
    let (stream, mut peer) = source();
    let log = Log::new();
    let file_context = Context::new(0, endpoint(&mut file), &log);
    let stream_context = Context::new(1, stream.native_handle_home(), &log);
    let unused_context = Context::new(2, endpoint(&mut unused_file), &log);
    let mut set = unsafe { PendingSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add_file(1, ptr::from_mut(file.as_mut()), file_context.adapter())
            .unwrap();
        set.add(2, stream.native_handle_home(), stream_context.adapter())
            .unwrap();
        set.add_file(
            3,
            ptr::from_mut(unused_file.as_mut()),
            unused_context.adapter(),
        )
        .unwrap();
    }
    let original = set.native.as_mut().unwrap().pending_snapshot(2).unwrap();
    let current = frame();
    let indefinite = budget(1);
    let (mut pipe_bytes, mut file_bytes) = ([0x33; 2], [0x44; 2]);
    let buffers = BufferList::from_iter(
        [Buffer::Read(&mut pipe_bytes), Buffer::Read(&mut file_bytes)].into_iter(),
    )
    .unwrap();
    let mut group = unsafe {
        set.begin_selected(&current, buffers, &indefinite, selection(&[1, 0]))
            .unwrap()
    };
    let file_completion = unsafe { group.next(&current).unwrap() };
    assert_eq!((file_completion.index, file_completion.key), (1, 1));
    assert!(matches!(
        file_completion.outcome.transfer,
        Some(AdapterTerminal::Data(2))
    ));
    peer.write_all(b"io").unwrap();
    let stream_completion = unsafe { group.next(&current).unwrap() };
    assert_eq!((stream_completion.index, stream_completion.key), (0, 2));
    assert!(matches!(
        stream_completion.outcome.transfer,
        Some(AdapterTerminal::Data(2))
    ));
    drop(group);
    assert_eq!(pipe_bytes, *b"io");
    assert_eq!(file_bytes, *b"fi");
    assert_eq!(
        set.native.as_mut().unwrap().pending_snapshot(2).unwrap(),
        original
    );
    assert!(!log.borrow().calls().iter().any(|call| matches!(
        call,
        Call::Prepare { index: 2, .. } | Call::Complete { index: 2, .. }
    )));
    assert_eq!(
        set.native.as_mut().unwrap().pending_snapshot(0).unwrap().2,
        0
    );
    drop(set);
    assert_eq!(file.position().unwrap(), 2);
    assert_eq!(unused_file.position().unwrap(), 0);
    assert!(unsafe { libc::fcntl(stream.get(), libc::F_GETFD) } >= 0);
}

#[test]
fn owned_file_rejects_scalar_readiness_as_prepared_endpoint_without_dereferencing_or_io() {
    let fixture = FileFixture::new();
    fs::write(&fixture.path, b"stay").unwrap();
    let mut file = fixture.open(FileAccess::Read, false);
    // This word is genuine wait metadata but not an OwnedFile pointer. Exact
    // configured identity must reject it before any owner cast/dereference.
    let scalar = file.readiness_source().value();
    let log = Log::new();
    let context = Context::new(0, scalar, &log);
    let mut set = unsafe { PendingSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add_file(1, ptr::from_mut(file.as_mut()), context.adapter())
            .unwrap();
    }
    let current = frame();
    let poll = budget(0);
    let mut bytes = [0x66; 4];
    let caller = bytes.as_mut_ptr() as usize;
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let result = unsafe { set.begin_selected(&current, buffers, &poll, selection(&[0])) };
    assert!(
        matches!(&result, Err(SetupFailure::Preparation(AdapterTerminal::PlatformFailure(failure)))
        if failure.reason == crate::platform_io::Reason::InvalidInput)
    );
    drop(result);
    assert_eq!(bytes, [0x66; 4]);
    assert_eq!(log.borrow().calls().len(), 2);
    assert!(matches!(log.borrow().calls()[1], Call::Complete {
        tag: 4, submitted: 0, progress: None, selected: (selected, data, 4), ..
    } if selected == scalar as usize && data == caller));
    assert_eq!(
        set.native.as_mut().unwrap().pending_snapshot(0).unwrap().2,
        0
    );
    drop(set);
    assert_eq!(file.position().unwrap(), 0);
    assert_eq!(fs::read(&fixture.path).unwrap(), b"stay");
}

#[test]
fn nonregular_owned_file_devices_and_pipes_are_rejected_before_transfer_without_close_claims() {
    let mut device = Box::new(OwnedFile::open(b"/dev/null", FileOpenRequest::READ).unwrap());
    let fifo_fixture = FileFixture::new();
    let path = std::ffi::CString::new(fifo_fixture.path.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    // Opening both directions keeps this real FIFO fixture deterministic: no
    // auxiliary thread, nonblocking conversion, worker, or waiting open needed.
    let mut pipe = fifo_fixture.open(FileAccess::ReadWrite, false);
    let log = Log::new();
    let device_context = Context::new(0, endpoint(&mut device), &log);
    let pipe_context = Context::new(1, endpoint(&mut pipe), &log);
    let mut set = unsafe { PendingSet::new(ptr::null_mut()).unwrap() };
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        let ((), closes) = crate::native_handle::with_close_probe(|| unsafe {
            for (key, file, context) in [
                (1, ptr::from_mut(device.as_mut()), &device_context),
                (2, ptr::from_mut(pipe.as_mut()), &pipe_context),
            ] {
                let failure = set.add_file(key, file, context.adapter()).unwrap_err();
                assert_eq!(failure.reason, crate::platform_io::Reason::Unsupported);
                assert_eq!(failure.system_code, None);
                assert_eq!(failure.transferred_byte_count, None);
                assert!(set.index_of(key).is_none());
            }
        });
        assert_eq!(closes, 0);
    });
    assert_eq!(allocations, 0);
    assert!(log.borrow().calls().is_empty());
    drop(set);
    let mut bytes = [0x99; 1];
    assert_eq!(
        device.read(&mut bytes).unwrap(),
        crate::byte_io::ReadProgress::EndOfStream
    );
    assert_eq!(bytes, [0x99]);
    assert_eq!(
        pipe.write(b"p").unwrap(),
        crate::byte_io::WriteProgress::Accepted(1)
    );
    assert_eq!(
        pipe.read(&mut bytes).unwrap(),
        crate::byte_io::ReadProgress::Data(1)
    );
    assert_eq!(bytes, [b'p']);
}
