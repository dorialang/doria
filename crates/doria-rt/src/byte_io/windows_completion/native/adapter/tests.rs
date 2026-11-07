use super::*;
use crate::byte_io::{ReadProgress, WriteProgress};
use crate::file_io::stream::{
    FileAccess, FileCreation, FileOpenRequest, FileSharing, OwnedFile, SeekOrigin,
};
use crate::native_handle::{with_close_probe, OwnedHandle};
use crate::transfer_group::adapter::test_object::TestObject;
use crate::transfer_group::adapter::DrAdapterContextV1;
use crate::transfer_group::adapter::DrAdapterOutcomeV1;
use crate::DrInterfaceCarrierV1;
use core::{ffi::c_void, mem::MaybeUninit};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

struct Fixture {
    directory: PathBuf,
    path: PathBuf,
}

fn readiness_handle(file: &OwnedFile) -> NativeHandle {
    unsafe {
        let owner = (file as *const OwnedFile).cast();
        let home = crate::file_io::stream::transport::dr_v1_file_readiness_source_home(owner);
        crate::native_handle::ReadinessSource::handle(*home.cast::<*const c_void>()).unwrap()
    }
}

#[test]
fn same_registration_adapter_operations_do_not_share_terminal_facts_or_close_ownership() {
    let fixture = Fixture::new(b"abcd");
    let context = Metered::new(fixture.open(false));
    let source = readiness_handle(unsafe { &(*context.as_ptr()).file });
    let mut set = unsafe { AdapterSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add(7, source, context.descriptor()).unwrap();
    }
    let mut empty = [];
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut empty), Buffer::Write(b"XY")].into_iter())
            .unwrap();
    let wait = immediate();
    let mut group = unsafe {
        set.begin_selected(ptr::null(), buffers, &wait, selection(&[0, 0]))
            .unwrap()
    };
    let mut seen = [false; 2];
    while let Some(completion) = unsafe { group.next(ptr::null()) } {
        assert_eq!(completion.key, 7);
        assert!(!seen[completion.index]);
        seen[completion.index] = true;
        match (completion.index, completion.outcome.transfer) {
            (0, Some(AdapterTerminal::Data(0))) | (1, Some(AdapterTerminal::Accepted(2))) => {}
            _ => panic!("each operation retains its own typed outcome"),
        }
    }
    assert_eq!(seen, [true; 2]);
    let ((), closes) = with_close_probe(|| drop(group));
    assert_eq!(closes, 0);
    assert_eq!(
        unsafe { ((*context.as_ptr()).reads, (*context.as_ptr()).writes) },
        (1, 1)
    );
    assert_eq!(unsafe { (*context.as_ptr()).file.position().unwrap() }, 2);
    let ((), closes) = with_close_probe(|| set.remove(7).unwrap());
    assert_eq!(closes, 0);
}

#[test]
fn shared_cursor_complete_adapter_orders_read_write_read_without_hot_allocations() {
    let fixture = Fixture::new(b"abcdefghi");
    let context = Metered::new(fixture.open(false));
    let source = readiness_handle(unsafe { &(*context.as_ptr()).file });
    let mut set = unsafe { AdapterSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add(7, source, context.descriptor()).unwrap();
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
    let wait = immediate();
    let mut group = unsafe {
        set.begin_selected(ptr::null(), buffers, &wait, selection(&[0; 3]))
            .unwrap()
    };
    assert_eq!(
        unsafe { ((*context.as_ptr()).reads, (*context.as_ptr()).writes) },
        (1, 0)
    );
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        for index in 0..3 {
            assert_eq!(group.next(ptr::null()).unwrap().index, index);
        }
        assert_eq!(group.completed(0).unwrap(), b"abc");
        assert_eq!(group.completed(2).unwrap(), b"fg");
        assert!(group.next(ptr::null()).is_none());
        drop(group);
    });
    assert_eq!(allocations, 0);
    assert_eq!(
        unsafe { ((*context.as_ptr()).reads, (*context.as_ptr()).writes) },
        (2, 1)
    );
    assert_eq!(unsafe { (*context.as_ptr()).file.position().unwrap() }, 7);
    assert_eq!(fs::read(&fixture.path).unwrap(), b"abcXYfghi");
    set.remove(7).unwrap();
}

impl Fixture {
    fn new(bytes: &[u8]) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "doria-windows-adapter-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("Dória-漢字.bin");
        fs::write(&path, bytes).unwrap();
        Self { directory, path }
    }

    fn open(&self, append: bool) -> OwnedFile {
        OwnedFile::open(
            self.path.to_str().unwrap().as_bytes(),
            FileOpenRequest {
                access: FileAccess::ReadWrite,
                creation: FileCreation::OpenExisting,
                append,
                sharing: FileSharing::ReadWrite,
            },
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.directory);
    }
}

struct Metered {
    file: OwnedFile,
    reads: usize,
    writes: usize,
    pointer: usize,
    frame: *const DrStackFrameV2,
    native_failure: Option<Failure>,
    cancel: *mut DrCancellationV1,
    cancel_failure: Option<u8>,
}

impl Metered {
    fn new(file: OwnedFile) -> TestObject<Self> {
        TestObject::new(Self {
            file,
            reads: 0,
            writes: 0,
            pointer: 0,
            frame: ptr::null(),
            native_failure: None,
            cancel: ptr::null_mut(),
            cancel_failure: None,
        })
    }
}

impl TestObject<Metered> {
    fn descriptor(&self) -> DrTransferAdapterV1 {
        DrTransferAdapterV1 {
            context: self.context(),
            read: Some(read),
            write: Some(write),
        }
    }
}

unsafe extern "C" fn read(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    data: *mut u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    _: *mut DrInterfaceCarrierV1,
) -> u8 {
    crate::test_callback::guard(|| {
        let context = TestObject::<Metered>::in_callback((*context).object);
        context.reads += 1;
        context.pointer = data as usize;
        context.frame = frame;
        *outcome = DrAdapterOutcomeV1::default();
        match context
            .file
            .read(core::slice::from_raw_parts_mut(data, length))
        {
            Ok(ReadProgress::Data(count)) => {
                (*outcome).count = count;
                (*outcome).has_count = 1;
                for byte in core::slice::from_raw_parts_mut(data, count) {
                    *byte = byte.to_ascii_uppercase();
                }
            }
            Ok(ReadProgress::EndOfStream) => (*outcome).tag = 2,
            Ok(ReadProgress::WouldBlock(_)) => (*outcome).tag = 1,
            Err(failure) => {
                // Fixture failures remain observable outside C, without unwinding
                // through its callback ABI or turning an unexpected fault into pass.
                context.native_failure = Some(failure);
                (*outcome).tag = 3;
            }
        }
        if !context.cancel.is_null() {
            let (mut reason, mut code, mut has_code) = (0, 0, 0);
            let status = cancellation::dr_v1_cancellation_request(
                context.cancel,
                &mut reason,
                &mut code,
                &mut has_code,
            );
            context.cancel_failure = Some(status);
        }
        0
    })
}

unsafe extern "C" fn write(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    data: *const u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    _: *mut DrInterfaceCarrierV1,
) -> u8 {
    crate::test_callback::guard(|| {
        let context = TestObject::<Metered>::in_callback((*context).object);
        context.writes += 1;
        context.pointer = data as usize;
        context.frame = frame;
        *outcome = DrAdapterOutcomeV1::default();
        match context
            .file
            .write(core::slice::from_raw_parts(data, length))
        {
            Ok(WriteProgress::Accepted(count)) => {
                (*outcome).count = count;
                (*outcome).has_count = 1;
            }
            Ok(WriteProgress::WouldBlock(_)) => (*outcome).tag = 1,
            Err(failure) => {
                context.native_failure = Some(failure);
                (*outcome).tag = 1;
            }
        }
        0
    })
}

fn selection(indices: &[usize]) -> FixedSlice<usize> {
    let mut selected = FixedSlice::new(indices.len()).unwrap();
    for index in indices {
        selected.push(*index).unwrap();
    }
    selected
}

fn immediate() -> DrWaitBudgetV1 {
    let mut budget = MaybeUninit::uninit();
    let (mut reason, mut code, mut has_code) = (0, 0, 0);
    unsafe {
        assert_eq!(
            crate::wait_budget::dr_v1_wait_budget_init(
                0,
                0,
                budget.as_mut_ptr(),
                &mut reason,
                &mut code,
                &mut has_code,
            ),
            0
        );
        budget.assume_init()
    }
}

fn frame(name: &'static [u8]) -> DrStackFrameV2 {
    DrStackFrameV2 {
        parent: ptr::null(),
        function_name: name.as_ptr(),
        function_name_length: name.len(),
        source_path: ptr::null(),
        source_path_length: 0,
        source_text: ptr::null(),
        source_text_length: 0,
        function_span_start: 0,
        function_span_end: 0,
        active_span_start: 0,
        active_span_end: 0,
    }
}

#[test]
fn regular_source_dispatches_adapter_subset_without_copy_or_close_claim() {
    let fixture_a = Fixture::new(b"abc");
    let fixture_b = Fixture::new(b"zz");
    let fixture_c = Fixture::new(b"uv");
    let a = Metered::new(fixture_a.open(false));
    let b = Metered::new(fixture_b.open(false));
    let c = Metered::new(fixture_c.open(false));
    let mut set = unsafe { AdapterSet::new(ptr::null_mut()) }.unwrap();
    assert_eq!(
        unsafe { set.add(10, readiness_handle(&(*a.as_ptr()).file), a.descriptor()) }.unwrap(),
        0
    );
    let stable_a = set.slots.raw(0);
    assert_eq!(
        unsafe { set.add(20, readiness_handle(&(*b.as_ptr()).file), b.descriptor()) }.unwrap(),
        1
    );
    assert_eq!(
        unsafe { set.add(30, readiness_handle(&(*c.as_ptr()).file), c.descriptor()) }.unwrap(),
        2
    );
    assert_eq!(set.slots.raw(0), stable_a);
    let mut bytes_a = [0x55; 3];
    let mut bytes_c = [0x55; 2];
    let pointer_a = bytes_a.as_mut_ptr() as usize;
    let pointer_c = bytes_c.as_mut_ptr() as usize;
    let buffers = BufferList::<AdapterTerminal>::from_iter(
        [Buffer::Read(&mut bytes_c), Buffer::Read(&mut bytes_a[..2])].into_iter(),
    )
    .unwrap();
    let budget = immediate();
    let begin_frame = frame(b"begin");
    let next_frame = frame(b"next");
    let submit_frame = frame(b"submit");
    let selected = selection(&[2, 0]);
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        let mut group = set
            .begin_selected(&begin_frame, buffers, &budget, selected)
            .unwrap();
        let first = group.next(&next_frame).unwrap();
        assert_eq!((first.key, first.index), (30, 0));
        assert!(matches!(
            first.outcome.transfer,
            Some(AdapterTerminal::Data(2))
        ));
        assert_eq!(group.completed(0).unwrap(), b"UV");
        let second = group.next(&next_frame).unwrap();
        assert_eq!((second.key, second.index), (10, 1));
        assert!(matches!(
            second.outcome.transfer,
            Some(AdapterTerminal::Data(2))
        ));
        assert_eq!(group.completed(1).unwrap(), b"AB");
        assert_eq!(group.request_info(1), Some((true, 2)));
        assert_eq!(
            group.read_storage_mut(1).unwrap().as_mut_ptr() as usize,
            pointer_a
        );
        group.submit(&submit_frame, 1, 0, 1).unwrap();
        let tail = group.next(&next_frame).unwrap();
        assert!(matches!(
            tail.outcome.transfer,
            Some(AdapterTerminal::Data(1))
        ));
        assert_eq!(group.completed(1).unwrap(), b"C");
        group.submit(&next_frame, 1, 0, 1).unwrap();
        let eof = group.next(&next_frame).unwrap();
        assert!(matches!(
            eof.outcome.transfer,
            Some(AdapterTerminal::EndOfStream)
        ));
        assert_eq!(group.completed(1).unwrap(), b"");
        assert!(group.next(&next_frame).is_none());
        assert_eq!(group.stop(), None);
    });
    assert_eq!(allocations, 0);
    assert_eq!(
        (
            unsafe { (*a.as_ptr()).reads },
            unsafe { (*b.as_ptr()).reads },
            unsafe { (*c.as_ptr()).reads }
        ),
        (3, 0, 1)
    );
    assert_eq!(
        unsafe { ((*a.as_ptr()).pointer, (*c.as_ptr()).pointer) },
        (pointer_a, pointer_c)
    );
    assert_eq!(
        unsafe { (*a.as_ptr()).frame },
        &next_frame as *const DrStackFrameV2
    );
    assert_eq!(
        unsafe { (*c.as_ptr()).frame },
        &begin_frame as *const DrStackFrameV2
    );
    assert!(
        unsafe { (*a.as_ptr()).native_failure.is_none() }
            && unsafe { (*c.as_ptr()).native_failure.is_none() }
    );
    let ((), closes) = with_close_probe(|| {
        set.remove(20).unwrap();
        assert_eq!(set.key(1), None);
        assert_eq!(set.key(0), Some(10));
        assert_eq!(set.index_of(30), Some(2));
        assert_eq!(set.len(), 2);
        assert_eq!(set.slots.raw(0), stable_a);
        drop(set);
    });
    assert_eq!(closes, 0);
    let mut untouched = [0; 2];
    assert_eq!(
        unsafe { (*b.as_ptr()).file.read(&mut untouched).unwrap() },
        ReadProgress::Data(2)
    );
    assert_eq!(&untouched, b"zz");
    assert_eq!(unsafe { (*a.as_ptr()).file.position().unwrap() }, 3);
    assert_eq!(bytes_a[2], 0x55);
}

#[test]
fn registered_append_owner_keeps_cursor_and_dispatches_empty_adapter_call() {
    let fixture = Fixture::new(b"abc");
    let first = Metered::new(fixture.open(true));
    let mut second = fixture.open(true);
    unsafe { (*first.as_ptr()).file.seek(1, SeekOrigin::Start).unwrap() };
    let mut set = unsafe { AdapterSet::new(ptr::null_mut()) }.unwrap();
    let (source, allocations) =
        crate::probe_runtime_allocations(|| readiness_handle(unsafe { &(*first.as_ptr()).file }));
    assert_eq!(allocations, 0);
    unsafe { set.add(10, source, first.descriptor()) }.unwrap();
    assert_eq!(unsafe { (*first.as_ptr()).file.position().unwrap() }, 1);
    assert_eq!(second.write(b"X").unwrap(), WriteProgress::Accepted(1));
    assert_eq!(second.position().unwrap(), 4);
    assert_eq!(unsafe { (*first.as_ptr()).file.position().unwrap() }, 1);
    let input = *b"Y";
    let buffers =
        BufferList::<AdapterTerminal>::from_iter([Buffer::Write(&input)].into_iter()).unwrap();
    let budget = immediate();
    unsafe {
        let mut group = set
            .begin_selected(ptr::null(), buffers, &budget, selection(&[0]))
            .unwrap();
        assert!(matches!(
            group.next(ptr::null()).unwrap().outcome.transfer,
            Some(AdapterTerminal::Accepted(1))
        ));
        group.submit(ptr::null(), 0, 0, 0).unwrap();
        assert!(matches!(
            group.next(ptr::null()).unwrap().outcome.transfer,
            Some(AdapterTerminal::Accepted(0))
        ));
    }
    assert_eq!(unsafe { (*first.as_ptr()).writes }, 2);
    assert!(unsafe { (*first.as_ptr()).native_failure.is_none() });
    assert_eq!(unsafe { (*first.as_ptr()).file.position().unwrap() }, 5);
    assert_eq!(input, *b"Y");
    drop(set);
    assert_eq!(
        unsafe { (*first.as_ptr()).file.write(b"Z").unwrap() },
        WriteProgress::Accepted(1)
    );
    assert_eq!(unsafe { (*first.as_ptr()).file.position().unwrap() }, 6);
    assert_eq!(second.position().unwrap(), 4);
    assert_eq!(fs::read(&fixture.path).unwrap(), b"abcXYZ");
}

#[test]
fn non_disk_and_wrong_direction_reject_before_io_and_callback_cancellation_stops_later_slot() {
    use crate::win32::CreatePipe;
    let fixture_a = Fixture::new(b"ab");
    let fixture_b = Fixture::new(b"cd");
    let a = Metered::new(fixture_a.open(false));
    let b = Metered::new(fixture_b.open(false));
    let state = cancellation::dr_v1_cancellation_new();
    assert!(!state.is_null());
    let _source_claim = CancellationClaim(state);
    let mut set = unsafe { AdapterSet::new(state) }.unwrap();
    let (mut pipe_read, mut pipe_write) = (ptr::null_mut(), ptr::null_mut());
    assert_ne!(
        unsafe { CreatePipe(&mut pipe_read, &mut pipe_write, ptr::null_mut(), 0) },
        0
    );
    let pipe_read = unsafe { OwnedHandle::from_raw(pipe_read) };
    let _pipe_write = unsafe { OwnedHandle::from_raw(pipe_write) };
    let (rejected, allocations) =
        crate::probe_runtime_allocations(|| unsafe { set.add(1, pipe_read.get(), a.descriptor()) });
    assert_eq!(rejected.err().unwrap().reason, Reason::Unsupported);
    assert_eq!(allocations, 0);
    assert_eq!(unsafe { (*a.as_ptr()).reads }, 0);
    let (rejected, allocations) = crate::probe_runtime_allocations(|| unsafe {
        set.add(2, -1_isize as NativeHandle, a.descriptor())
    });
    assert_eq!(rejected.err().unwrap().system_code, Some(6));
    assert_eq!(allocations, 0);
    unsafe { set.add(10, readiness_handle(&(*a.as_ptr()).file), a.descriptor()) }.unwrap();
    let mut descriptor = b.descriptor();
    descriptor.read = None;
    unsafe { set.add(20, readiness_handle(&(*b.as_ptr()).file), descriptor) }.unwrap();
    let mut bytes_a = [0x55; 2];
    let mut bytes_b = [0x55; 2];
    let buffers = BufferList::<AdapterTerminal>::from_iter(
        [Buffer::Read(&mut bytes_a), Buffer::Read(&mut bytes_b)].into_iter(),
    )
    .unwrap();
    let budget = immediate();
    let indices = selection(&[0, 1]);
    let (rejected, allocations) = crate::probe_runtime_allocations(|| unsafe {
        match set.begin_selected(ptr::null(), buffers, &budget, indices) {
            Err(failure) => Some(failure),
            Ok(group) => {
                drop(group);
                None
            }
        }
    });
    assert!(
        matches!(rejected.unwrap(), SetupFailure::Platform(failure) if failure.reason == Reason::InvalidInput)
    );
    assert_eq!(allocations, 0);
    assert_eq!(
        unsafe { ((*a.as_ptr()).reads, (*b.as_ptr()).reads) },
        (0, 0)
    );
    assert_eq!(bytes_a, [0x55; 2]);
    assert_eq!(bytes_b, [0x55; 2]);
    set.remove(20).unwrap();
    unsafe { set.add(30, readiness_handle(&(*b.as_ptr()).file), b.descriptor()) }.unwrap();
    unsafe { (*a.as_ptr()).cancel = state };
    let buffers = BufferList::<AdapterTerminal>::from_iter(
        [Buffer::Read(&mut bytes_a), Buffer::Read(&mut bytes_b)].into_iter(),
    )
    .unwrap();
    unsafe {
        let mut group = set
            .begin_selected(ptr::null(), buffers, &budget, selection(&[0, 1]))
            .unwrap();
        let won = group.next(ptr::null()).unwrap();
        assert_eq!(won.key, 10);
        assert!(matches!(
            won.outcome.transfer,
            Some(AdapterTerminal::Data(2))
        ));
        let stopped = group.next(ptr::null()).unwrap();
        assert_eq!(stopped.key, 30);
        assert!(stopped.outcome.transfer.is_none());
        assert_eq!(stopped.outcome.stop, Some(Stop::Cancellation));
        assert!(group.next(ptr::null()).is_none());
        assert_eq!(group.stop(), Some(Stop::Cancellation));
    }
    assert_eq!(
        unsafe { ((*a.as_ptr()).reads, (*b.as_ptr()).reads) },
        (1, 0)
    );
    assert_eq!(unsafe { (*a.as_ptr()).cancel_failure }, Some(0));
    assert!(unsafe { (*a.as_ptr()).native_failure.is_none() });
    assert_eq!(&bytes_a, b"AB");
    assert_eq!(bytes_b, [0x55; 2]);
    assert_eq!(unsafe { (*b.as_ptr()).file.position().unwrap() }, 0);
}
