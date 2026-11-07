use super::*;
use crate::transfer_group::adapter::test_object::TestObject;
use crate::transfer_group::adapter::DrAdapterContextV1;
use crate::transfer_group::adapter::DrAdapterOutcomeV1;
use crate::{DrInterfaceCarrierV1, DrInterfaceVtableV1};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Clone, Copy)]
enum Action {
    Pending,
    Data(&'static [u8]),
    Accepted(usize),
    EndOfStream,
    TimedOut,
    Error,
}

#[derive(Clone, Copy, Default)]
struct Call {
    frame: usize,
    data: usize,
    length: usize,
    reading: bool,
}

struct Script {
    actions: &'static [Action],
    calls: [Call; 16],
    count: usize,
    output: Option<i32>,
    error: *mut u8,
    error_descriptor: *const DrInterfaceVtableV1,
    progress: Option<usize>,
}

impl Script {
    fn new(actions: &'static [Action]) -> TestObject<Self> {
        TestObject::new(Self {
            actions,
            calls: [Call::default(); 16],
            count: 0,
            output: None,
            error: ptr::null_mut(),
            error_descriptor: &ERROR_DESCRIPTOR,
            progress: None,
        })
    }
}

impl TestObject<Script> {
    fn descriptor(&self) -> DrTransferAdapterV1 {
        DrTransferAdapterV1 {
            context: self.context(),
            read: Some(read),
            write: Some(write),
        }
    }
}

impl Script {
    unsafe fn attempt(
        &mut self,
        frame: *const DrStackFrameV2,
        data: *mut u8,
        length: usize,
        reading: bool,
        outcome: *mut DrAdapterOutcomeV1,
        error: *mut DrInterfaceCarrierV1,
    ) -> u8 {
        let action = self
            .actions
            .get(self.count)
            .copied()
            .unwrap_or_else(|| *self.actions.last().unwrap());
        self.calls[self.count] = Call {
            frame: frame as usize,
            data: data as usize,
            length,
            reading,
        };
        self.count += 1;
        match action {
            Action::Pending => (*outcome).tag = 1,
            Action::Data(bytes) => {
                assert!(reading && bytes.len() <= length);
                if !bytes.is_empty() {
                    ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
                }
                (*outcome).count = bytes.len();
                (*outcome).has_count = 1;
            }
            Action::Accepted(count) => {
                assert!(!reading && count <= length);
                if let Some(output) = self.output {
                    assert_eq!(libc::write(output, data.cast(), count), count as isize);
                }
                (*outcome).count = count;
                (*outcome).has_count = 1;
            }
            Action::EndOfStream => {
                assert!(reading);
                (*outcome).tag = 2;
            }
            Action::TimedOut => {
                assert!(reading);
                (*outcome).tag = 3;
            }
            Action::Error => {
                assert!(!self.error.is_null());
                (*error).object = mem::replace(&mut self.error, ptr::null_mut());
                (*error).vtable = self.error_descriptor;
                if let Some(count) = self.progress {
                    assert!(count <= length);
                    if reading && count != 0 {
                        ptr::write_bytes(data, 0xEE, count);
                    }
                    (*outcome).count = count;
                    (*outcome).has_count = 1;
                }
                return 1;
            }
        }
        0
    }
}

unsafe extern "C" fn read(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    data: *mut u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    TestObject::<Script>::in_callback((*context).object)
        .attempt(frame, data, length, true, outcome, error)
}

unsafe extern "C" fn write(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    data: *const u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    TestObject::<Script>::in_callback((*context).object).attempt(
        frame,
        data.cast_mut(),
        length,
        false,
        outcome,
        error,
    )
}

fn frame() -> DrStackFrameV2 {
    DrStackFrameV2 {
        parent: ptr::null(),
        function_name: ptr::null(),
        function_name_length: 0,
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
fn same_registration_dispatches_distinct_duplex_operations_through_complete_adapter() {
    let (source, _peer) = source();
    let script = Script::new(&[Action::Data(b"abc"), Action::Accepted(3)]);
    let mut set = unsafe { AdapterSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add(9, source.as_raw_fd(), script.descriptor()).unwrap();
    }
    let wait = budget(1);
    let mut input = [0xa5; 3];
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut input), Buffer::Write(b"out")].into_iter())
            .unwrap();
    let mut selected = FixedSlice::new(2).unwrap();
    selected.push(0).unwrap();
    selected.push(0).unwrap();
    let mut group = unsafe {
        set.begin_selected(ptr::null(), buffers, &wait, selected)
            .unwrap()
    };
    let mut seen = [false; 2];
    while let Some(completion) = unsafe { group.next(ptr::null()) } {
        assert_eq!(completion.key, 9);
        assert!(!seen[completion.index]);
        seen[completion.index] = true;
        assert!(matches!(
            completion.outcome.transfer,
            Some(AdapterTerminal::Data(3) | AdapterTerminal::Accepted(3))
        ));
    }
    assert_eq!(seen, [true; 2]);
    drop(group);
    assert_eq!(&input, b"abc");
    assert_eq!(unsafe { (*script.as_ptr()).count }, 2);
    assert!(
        unsafe { (*script.as_ptr()).calls[0].reading }
            && !unsafe { (*script.as_ptr()).calls[1].reading }
    );
    assert_ne!(unsafe { (*script.as_ptr()).calls[0].data }, unsafe {
        (*script.as_ptr()).calls[1].data
    });
    set.remove(9).unwrap();
}

#[test]
fn competing_complete_adapter_reads_do_not_dispatch_the_queued_callback_early() {
    let (source, _peer) = source();
    let script = Script::new(&[Action::Pending, Action::Data(b"abc"), Action::Data(b"def")]);
    let mut set = unsafe { AdapterSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add(9, source.as_raw_fd(), script.descriptor()).unwrap();
    }
    let wait = budget(1);
    let (mut a, mut b) = ([0xa5; 3], [0xb5; 3]);
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut a), Buffer::Read(&mut b)].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(ptr::null(), buffers, &wait, selection(&[0, 0]))
            .unwrap()
    };
    assert_eq!(unsafe { (*script.as_ptr()).count }, 1);
    assert!(group.read_storage_mut(1).is_none());
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        assert_eq!(group.next(ptr::null()).unwrap().index, 0);
        assert_eq!(group.next(ptr::null()).unwrap().index, 1);
        assert_eq!(group.completed(0).unwrap(), b"abc");
        assert_eq!(group.completed(1).unwrap(), b"def");
        assert!(group.next(ptr::null()).is_none());
        drop(group);
    });
    assert_eq!(allocations, 0);
    assert_eq!(unsafe { (*script.as_ptr()).count }, 3);
    set.remove(9).unwrap();
}

fn budget(kind: u8) -> DrWaitBudgetV1 {
    let mut budget = mem::MaybeUninit::uninit();
    let (mut reason, mut code, mut has_code) = (0, 0, 0);
    assert_eq!(
        unsafe {
            crate::wait_budget::dr_v1_wait_budget_init(
                kind,
                0,
                budget.as_mut_ptr(),
                &mut reason,
                &mut code,
                &mut has_code,
            )
        },
        0
    );
    unsafe { budget.assume_init() }
}

fn selection(indices: &[usize]) -> FixedSlice<usize> {
    let mut selected = FixedSlice::new(indices.len()).unwrap();
    for index in indices {
        selected.push(*index).unwrap();
    }
    selected
}

fn source() -> (UnixStream, UnixStream) {
    let (source, peer) = UnixStream::pair().unwrap();
    source.set_nonblocking(true).unwrap();
    (source, peer)
}

#[test]
fn reversed_subset_calls_complete_adapters_without_consuming_source_payloads_or_copying_buffers() {
    let (mut first, mut first_peer) = source();
    let (mut unused, mut unused_peer) = source();
    let (mut last, mut last_peer) = source();
    first_peer.write_all(b"watch-A").unwrap();
    unused_peer.write_all(b"unused").unwrap();
    last_peer.write_all(b"watch-C").unwrap();
    let a = Script::new(&[Action::Pending, Action::Pending, Action::Data(b"adapter-A")]);
    let b = Script::new(&[Action::Data(b"unused adapter")]);
    let c = Script::new(&[Action::Pending, Action::Pending, Action::Data(b"C")]);
    let mut set = unsafe { AdapterSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add(10, first.as_raw_fd(), a.descriptor()).unwrap();
        set.add(20, unused.as_raw_fd(), b.descriptor()).unwrap();
        set.add(30, last.as_raw_fd(), c.descriptor()).unwrap();
    }
    let requests = [set.slots.raw(0), set.slots.raw(2)];
    let watcher = set.watcher.handle().unwrap();
    let (begin_frame, next_frame) = (frame(), frame());
    let mut last_bytes = [0xCA; 4];
    let mut first_bytes = [0xCB; 12];
    let pointers = [last_bytes.as_mut_ptr(), first_bytes.as_mut_ptr()];
    let buffers = BufferList::from_iter(
        [
            Buffer::Read(&mut last_bytes),
            Buffer::Read(&mut first_bytes),
        ]
        .into_iter(),
    )
    .unwrap();
    let poll = budget(0);
    let mut group = unsafe {
        set.begin_selected(&begin_frame, buffers, &poll, selection(&[2, 0]))
            .unwrap()
    };
    let (first_completion, allocations) =
        crate::probe_runtime_allocations(|| unsafe { group.next(&next_frame).unwrap() });
    assert_eq!(allocations, 0);
    assert_eq!((first_completion.index, first_completion.key), (0, 30));
    assert!(matches!(
        first_completion.outcome.transfer,
        Some(AdapterTerminal::Data(1))
    ));
    assert_eq!(group.completed(0).unwrap(), b"C");
    assert_eq!(group.completed(0).unwrap().as_ptr(), pointers[0]);
    let second = unsafe { group.next(&next_frame).unwrap() };
    assert_eq!((second.index, second.key), (1, 10));
    assert!(matches!(
        second.outcome.transfer,
        Some(AdapterTerminal::Data(9))
    ));
    assert_eq!(group.completed(1).unwrap(), b"adapter-A");
    assert_eq!(group.completed(1).unwrap().as_ptr(), pointers[1]);
    assert!(unsafe { group.next(&next_frame) }.is_none());
    drop(group);
    assert_eq!(unsafe { (*b.as_ptr()).count }, 0);
    for script in [&a, &c] {
        assert_eq!(unsafe { (*script.as_ptr()).count }, 3);
        assert!(unsafe {
            (&(*script.as_ptr()).calls)[..3]
                .iter()
                .all(|call| call.reading)
        });
        assert_eq!(
            unsafe { (*script.as_ptr()).calls[0].frame },
            &begin_frame as *const _ as usize
        );
        assert!(unsafe {
            (&(*script.as_ptr()).calls)[1..3]
                .iter()
                .all(|call| call.frame == &next_frame as *const _ as usize)
        });
    }
    assert_eq!(unsafe { (*a.as_ptr()).calls[0].data }, pointers[1] as usize);
    assert_eq!(unsafe { (*c.as_ptr()).calls[0].data }, pointers[0] as usize);
    assert_eq!([set.slots.raw(0), set.slots.raw(2)], requests);
    assert_eq!(set.watcher.handle().unwrap(), watcher);
    assert!(set.watcher.slots().iter().all(|slot| slot.interests == 0));
    for (stream, expected) in [
        (&mut first, b"watch-A".as_slice()),
        (&mut unused, b"unused"),
        (&mut last, b"watch-C"),
    ] {
        let mut payload = [0; 7];
        stream.read_exact(&mut payload[..expected.len()]).unwrap();
        assert_eq!(&payload[..expected.len()], expected);
    }
}

#[test]
fn partial_writes_reuse_the_original_suffix_and_request_without_hot_allocations() {
    let (writer, mut peer) = source();
    let adapter = Script::new(&[Action::Accepted(2), Action::Accepted(3)]);
    unsafe { (*adapter.as_ptr()).output = Some(writer.as_raw_fd()) };
    let mut set = unsafe { AdapterSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add(7, writer.as_raw_fd(), adapter.descriptor())
            .unwrap()
    };
    let request = set.slots.raw(0);
    let (initial_frame, later_frame) = (frame(), frame());
    let bytes = *b"hello";
    let buffers = BufferList::from_iter([Buffer::Write(&bytes)].into_iter()).unwrap();
    let poll = budget(0);
    let mut group = unsafe {
        set.begin_selected(&initial_frame, buffers, &poll, selection(&[0]))
            .unwrap()
    };
    assert_eq!(group.request_info(0), Some((false, 5)));
    assert!(group.read_storage_mut(0).is_none());
    let (_, allocations) = crate::probe_runtime_allocations(|| unsafe {
        let first = group.next(&later_frame).unwrap();
        assert!(matches!(
            first.outcome.transfer,
            Some(AdapterTerminal::Accepted(2))
        ));
        assert_eq!(group.completed(0).unwrap(), b"he");
        group.submit(&later_frame, 0, 2, 3).unwrap();
        let rest = group.next(&later_frame).unwrap();
        assert!(matches!(
            rest.outcome.transfer,
            Some(AdapterTerminal::Accepted(3))
        ));
        assert_eq!(group.completed(0).unwrap(), b"llo");
    });
    assert_eq!(allocations, 0);
    drop(group);
    assert_eq!(set.slots.raw(0), request);
    assert_eq!(unsafe { (*adapter.as_ptr()).count }, 2);
    assert_eq!(
        unsafe { (*adapter.as_ptr()).calls[0].data },
        bytes.as_ptr() as usize
    );
    assert_eq!(
        unsafe { (*adapter.as_ptr()).calls[1].data },
        unsafe { bytes.as_ptr().add(2) } as usize
    );
    assert_eq!(
        unsafe {
            (
                (*adapter.as_ptr()).calls[0].length,
                (*adapter.as_ptr()).calls[1].length,
            )
        },
        (5, 3)
    );
    assert_eq!(
        unsafe { (*adapter.as_ptr()).calls[0].frame },
        &initial_frame as *const _ as usize
    );
    assert_eq!(
        unsafe { (*adapter.as_ptr()).calls[1].frame },
        &later_frame as *const _ as usize
    );
    let mut received = [0; 5];
    peer.read_exact(&mut received).unwrap();
    assert_eq!(&received, b"hello");
}

#[test]
fn zero_data_empty_dispatch_eof_and_timed_out_remain_distinct_adapter_outcomes() {
    let (source, _peer) = source();
    let adapter = Script::new(&[
        Action::Data(b""),
        Action::Data(b""),
        Action::EndOfStream,
        Action::TimedOut,
    ]);
    let mut set = unsafe { AdapterSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add(1, source.as_raw_fd(), adapter.descriptor())
            .unwrap()
    };
    let mut bytes = [0xA5; 4];
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let poll = budget(0);
    let current = frame();
    let mut group = unsafe {
        set.begin_selected(&current, buffers, &poll, selection(&[0]))
            .unwrap()
    };
    unsafe {
        assert!(matches!(
            group.next(&current).unwrap().outcome.transfer,
            Some(AdapterTerminal::Data(0))
        ));
        group.submit(&current, 0, 0, 0).unwrap();
        assert!(matches!(
            group.next(&current).unwrap().outcome.transfer,
            Some(AdapterTerminal::Data(0))
        ));
        group.submit(&current, 0, 0, 1).unwrap();
        assert!(matches!(
            group.next(&current).unwrap().outcome.transfer,
            Some(AdapterTerminal::EndOfStream)
        ));
        group.submit(&current, 0, 0, 1).unwrap();
        assert!(matches!(
            group.next(&current).unwrap().outcome.transfer,
            Some(AdapterTerminal::TimedOut)
        ));
    }
    assert!(group.completed(0).is_none());
    assert_eq!(group.stop(), None);
    assert_eq!(group.read_storage_mut(0).unwrap(), &[0xA5; 4]);
    drop(group);
    assert_eq!(unsafe { (*adapter.as_ptr()).count }, 4);
    assert_eq!(unsafe { (*adapter.as_ptr()).calls[1].length }, 0);
}

struct ExactError {
    identity: usize,
    drops: *const AtomicUsize,
}

unsafe extern "C" fn drop_error(frame: *const DrStackFrameV2, object: *mut u8) {
    assert!(
        frame.is_null(),
        "owned terminal retained a stale call frame"
    );
    let error = Box::from_raw(object.cast::<ExactError>());
    assert_eq!(error.identity, 0x1234);
    (*error.drops).fetch_add(1, Ordering::SeqCst);
}

const ERROR_DESCRIPTOR: DrInterfaceVtableV1 = DrInterfaceVtableV1 {
    type_name: b"Fixture\\DistinctIoError".as_ptr(),
    type_name_length: b"Fixture\\DistinctIoError".len(),
    message_offset: 0,
    drop: drop_error,
    class_size: mem::size_of::<ExactError>(),
    origin_offset: 0,
    assertion_offsets: 0,
    assertion_magic: 0,
    class_descriptor: ptr::null(),
    interface_id: 0,
    io_transferred_byte_count_offset: 0,
    io_progress_magic: 0,
};

fn error(drops: &AtomicUsize) -> *mut u8 {
    Box::into_raw(Box::new(ExactError {
        identity: 0x1234,
        drops,
    }))
    .cast()
}

#[test]
fn same_registration_failure_keeps_operation_identity_original_error_and_pending_sibling() {
    let (source, _peer) = source();
    let drops = AtomicUsize::new(0);
    let script = Script::new(&[
        Action::Pending,
        Action::Error,
        Action::Pending,
        Action::Data(b"abc"),
    ]);
    let original = error(&drops);
    unsafe { (*script.as_ptr()).error = original };
    let descriptor = unsafe { (*script.as_ptr()).error_descriptor };
    let mut set = unsafe { AdapterSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add(23, source.as_raw_fd(), script.descriptor())
            .unwrap();
    }
    let mut input = [0xa5; 3];
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut input), Buffer::Write(b"out")].into_iter())
            .unwrap();
    let wait = budget(1);
    let mut group = unsafe {
        set.begin_selected(ptr::null(), buffers, &wait, selection(&[0, 0]))
            .unwrap()
    };
    let completion = unsafe { group.next(ptr::null()).unwrap() };
    assert_eq!((completion.key, completion.index), (23, 1));
    let error = match completion.outcome.transfer.unwrap() {
        AdapterTerminal::CheckedError {
            error,
            progress: None,
        } => error,
        _ => panic!("the original checked error and unknown progress must survive"),
    };
    let carrier = error.into_carrier();
    assert_eq!((carrier.object, carrier.vtable), (original, descriptor));
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert_eq!(group.stop(), None);
    assert!(group.completed(0).is_none());
    assert!(group.read_storage_mut(0).is_none());
    let sibling = unsafe { group.next(ptr::null()).unwrap() };
    assert_eq!((sibling.key, sibling.index), (23, 0));
    assert!(matches!(
        sibling.outcome.transfer,
        Some(AdapterTerminal::Data(3))
    ));
    assert_eq!(group.completed(0).unwrap(), b"abc");
    drop(group);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    unsafe {
        ((*carrier.vtable).drop)(ptr::null(), carrier.object);
    }
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn checked_error_identity_progress_and_once_only_drop_survive_terminal_cache_and_move() {
    let (source, _peer) = source();
    let drops = AtomicUsize::new(0);
    let adapter = Script::new(&[Action::Error, Action::Error]);
    let descriptor = unsafe { (*adapter.as_ptr()).error_descriptor };
    unsafe { (*adapter.as_ptr()).error = error(&drops) };
    unsafe { (*adapter.as_ptr()).progress = Some(2) };
    let mut set = unsafe { AdapterSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add(1, source.as_raw_fd(), adapter.descriptor())
            .unwrap()
    };
    let current = frame();
    let poll = budget(0);
    let mut bytes = [0xAA; 4];
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let group = unsafe {
        set.begin_selected(&current, buffers, &poll, selection(&[0]))
            .unwrap()
    };
    drop(group); // An unconsumed cached checked error is destroyed once.
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert_eq!(&bytes[..2], &[0xEE; 2]);
    set.remove(1).unwrap();
    let object = error(&drops);
    unsafe { (*adapter.as_ptr()).error = object };
    unsafe { (*adapter.as_ptr()).progress = None };
    assert_eq!(unsafe { (*adapter.as_ptr()).error }, object);
    assert_eq!(unsafe { (*adapter.as_ptr()).progress }, None);
    unsafe {
        set.add(2, source.as_raw_fd(), adapter.descriptor())
            .unwrap()
    };
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(&current, buffers, &poll, selection(&[0]))
            .unwrap()
    };
    let terminal = unsafe { group.next(&current).unwrap() }
        .outcome
        .transfer
        .unwrap();
    let AdapterTerminal::CheckedError { error, progress } = terminal else {
        panic!("checked error identity was flattened")
    };
    assert_eq!(progress, None);
    assert!(group.completed(0).is_none());
    let carrier = error.into_carrier();
    assert_eq!(carrier.object, object);
    assert_eq!(carrier.vtable, descriptor);
    drop(group);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    unsafe { ((*carrier.vtable).drop)(ptr::null(), carrier.object) };
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}

#[test]
fn cancellation_and_drop_retire_watches_without_callback_and_keep_external_sources_live() {
    let (mut source, mut peer) = source();
    let cancellation = crate::cancellation::dr_v1_cancellation_new();
    assert!(!cancellation.is_null());
    let adapter = Script::new(&[Action::Pending]);
    let mut set = unsafe { AdapterSet::new(cancellation).unwrap() };
    unsafe {
        set.add(1, source.as_raw_fd(), adapter.descriptor())
            .unwrap()
    };
    let request = set.slots.raw(0);
    let current = frame();
    let indefinite = budget(1);
    let mut bytes = [0xAB; 5];
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(&current, buffers, &indefinite, selection(&[0]))
            .unwrap()
    };
    let (mut reason, mut code, mut has_code) = (0, 0, 0);
    assert_eq!(
        unsafe {
            crate::cancellation::dr_v1_cancellation_request(
                cancellation,
                &mut reason,
                &mut code,
                &mut has_code,
            )
        },
        0
    );
    let stopped = unsafe { group.next(&current).unwrap() };
    assert!(stopped.outcome.transfer.is_none());
    assert_eq!(stopped.outcome.stop, Some(Stop::Cancellation));
    drop(group);
    assert_eq!(unsafe { (*adapter.as_ptr()).count }, 1);
    assert_eq!(set.slots.raw(0), request);
    assert!(set.watcher.slots().iter().all(|slot| slot.interests == 0));
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(&current, buffers, &indefinite, selection(&[0]))
            .unwrap()
    };
    assert!(unsafe { group.next(&current).unwrap() }
        .outcome
        .transfer
        .is_none());
    drop(group);
    assert_eq!(
        unsafe { (*adapter.as_ptr()).count },
        1,
        "persistent cancellation was reset"
    );
    drop(set);
    unsafe { crate::cancellation::dr_v1_cancellation_release(cancellation) };
    peer.write_all(b"live").unwrap();
    source.read_exact(&mut [0; 4]).unwrap();

    let adapter = Script::new(&[Action::Pending, Action::Data(b"reuse")]);
    let mut set = unsafe { AdapterSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add(7, source.as_raw_fd(), adapter.descriptor())
            .unwrap()
    };
    let request = set.slots.raw(0);
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let group = unsafe {
        set.begin_selected(&current, buffers, &indefinite, selection(&[0]))
            .unwrap()
    };
    let (_, closes) = crate::native_handle::with_close_probe(|| drop(group));
    assert_eq!(closes, 0);
    assert_eq!(unsafe { (*adapter.as_ptr()).count }, 1);
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(&current, buffers, &indefinite, selection(&[0]))
            .unwrap()
    };
    assert!(matches!(
        unsafe { group.next(&current).unwrap() }.outcome.transfer,
        Some(AdapterTerminal::Data(5))
    ));
    assert_eq!(group.completed(0).unwrap(), b"reuse");
    drop(group);
    assert_eq!(set.slots.raw(0), request);
    let (_, closes) = crate::native_handle::with_close_probe(|| drop(set));
    assert_eq!(closes, 1, "only the watcher is owned by this set");
    peer.write_all(b"live").unwrap();
    source.read_exact(&mut [0; 4]).unwrap();
}

#[test]
fn permanent_regular_file_readiness_spends_one_immediate_poll_without_raw_transfer() {
    static NEXT_FILE: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "doria-adapter-regular-{}-{}",
        std::process::id(),
        NEXT_FILE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    file.write_all(b"unconsumed").unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    let adapter = Script::new(&[Action::Pending]);
    let mut set = unsafe { AdapterSet::new(ptr::null_mut()).unwrap() };
    unsafe { set.add(1, file.as_raw_fd(), adapter.descriptor()).unwrap() };
    let mut bytes = [0xCC; 4];
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let poll = budget(0);
    let current = frame();
    let mut group = unsafe {
        set.begin_selected(&current, buffers, &poll, selection(&[0]))
            .unwrap()
    };
    let (stopped, allocations) =
        crate::probe_runtime_allocations(|| unsafe { group.next(&current).unwrap() });
    assert_eq!(allocations, 0);
    assert!(stopped.outcome.transfer.is_none());
    assert_eq!(stopped.outcome.stop, Some(Stop::BudgetExhausted));
    drop(group);
    assert_eq!(
        unsafe { (*adapter.as_ptr()).count },
        3,
        "immediate advisory readiness busy-polled"
    );
    assert_eq!(bytes, [0xCC; 4]);
    assert_eq!(set.watcher.immediate_count, 0);
    assert_eq!(file.stream_position().unwrap(), 0);
    let mut payload = String::new();
    file.read_to_string(&mut payload).unwrap();
    assert_eq!(payload, "unconsumed");
    drop(set);
    drop(file);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn rejected_admission_and_later_callback_capability_never_transfer_or_close_sources() {
    let (mut first, mut first_peer) = source();
    let (mut second, mut second_peer) = source();
    let a = Script::new(&[Action::Data(b"A")]);
    let b = Script::new(&[Action::Data(b"B")]);
    let mut set = unsafe { AdapterSet::new(ptr::null_mut()).unwrap() };
    unsafe { set.add(10, first.as_raw_fd(), a.descriptor()).unwrap() };
    let mut read_only = b.descriptor();
    read_only.write = None;
    unsafe { set.add(20, second.as_raw_fd(), read_only).unwrap() };
    let request = set.slots.raw(0);
    let (_, closes) = crate::native_handle::with_close_probe(|| {
        assert_eq!(
            unsafe { set.add(30, first.as_raw_fd(), a.descriptor()) },
            Err(Failure::invalid_input())
        );
        assert!(unsafe { set.add(30, -1, a.descriptor()) }.is_err());
    });
    assert_eq!(closes, 0);
    assert_eq!(
        (set.len(), set.key(0), set.key(1), set.key(2)),
        (2, Some(10), Some(20), None)
    );
    assert_eq!(set.index_of(20), Some(1));
    let mut bytes = [0; 1];
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut bytes), Buffer::Write(b"x")].into_iter()).unwrap();
    let poll = budget(0);
    let current = frame();
    let indices = selection(&[0, 1]);
    let ((failure, allocations), closes) = crate::native_handle::with_close_probe(|| {
        crate::probe_runtime_allocations(|| unsafe {
            set.begin_selected(&current, buffers, &poll, indices)
                .err()
                .unwrap()
        })
    });
    assert!(
        matches!(failure, SetupFailure::Platform(failure) if failure == Failure::invalid_input())
    );
    assert_eq!(
        (
            allocations,
            closes,
            unsafe { (*a.as_ptr()).count },
            unsafe { (*b.as_ptr()).count }
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(set.slots.raw(0), request);
    assert!(set.watcher.slots().iter().all(|slot| slot.interests == 0));
    set.remove(20).unwrap();
    assert_eq!(set.key(1), None);
    assert_eq!(
        unsafe { set.add(20, second.as_raw_fd(), read_only) },
        Err(Failure::invalid_input())
    );
    assert_eq!(
        unsafe { set.add(30, second.as_raw_fd(), read_only).unwrap() },
        1
    );
    assert_eq!(set.slots.raw(0), request);
    let (_, closes) = crate::native_handle::with_close_probe(|| drop(set));
    assert_eq!(closes, 1);
    for (source, peer) in [
        (&mut first, &mut first_peer),
        (&mut second, &mut second_peer),
    ] {
        peer.write_all(b"live").unwrap();
        source.read_exact(&mut [0; 4]).unwrap();
    }
}
