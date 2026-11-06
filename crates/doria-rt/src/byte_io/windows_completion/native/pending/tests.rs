//! Genuine Windows completion fixtures. Cross-compilation does not execute
//! their kernel, packet-retirement, cancellation or ownership assertions.

use crate::transfer_group::adapter::DrAdapterContextV1;
use core::{mem, ptr};
use std::os::windows::io::{IntoRawHandle, OwnedHandle as StdHandle};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::super::broker::transport::{pending_native_dequeues, pending_test_lock};
use super::super::broker::AttachedStream;
use super::super::tests::{budget, connected_pair, TestClaim};
use super::super::{Overlapped, Pipe, WaitControl, WindowsDriver, STATUS_PENDING};
use super::*;
use crate::byte_io::{ReadProgress, WriteProgress};
use crate::file_io::stream::{
    FileAccess, FileCreation, FileOpenRequest, FileSharing, OwnedFile, SeekOrigin,
};
use crate::native_handle::{with_close_probe, OwnedHandle};
use crate::transfer_group::adapter::{DrAdapterOutcomeV1, OwnedIoError};
use crate::transfer_group::pending::{
    dr_v1_pending_request_configure_native, pending_callback_facts,
};
use crate::transfer_group::pending::{DrPendingRegionV1, FileTerminal};
use crate::transfer_group::{Awaited, Buffer, TerminalOutcome};
use crate::{DrInterfaceCarrierV1, DrInterfaceVtableV1};

const TEST_NANOSECONDS: u64 = 5_000_000_000;

#[test]
fn same_attachment_pending_duplex_uses_distinct_request_and_completion_records() {
    let _serial = pending_test_lock();
    let (server, client) = connected_pair();
    let owner = Context::new(server, Selection::Caller);
    let mut context = DuplexContext {
        endpoint: owner.endpoint(),
        prepares: 0,
        completes: 0,
    };
    let mut peer = peer(client);
    let mut set = unsafe { PendingSet::new(ptr::null_mut()).unwrap() };
    let adapter = DrPendingTransferAdapterV1 {
        context: DrAdapterContextV1 {
            object: ptr::from_mut(&mut context).cast(),
            descriptor: core::ptr::null(),
        },
        prepare_read: Some(duplex_prepare_read),
        prepare_write: Some(duplex_prepare_write),
        complete_read: Some(duplex_complete_read),
        complete_write: Some(duplex_complete_write),
    };
    unsafe {
        set.add(91, owner.endpoint(), adapter).unwrap();
    }
    let wait = budget(2, TEST_NANOSECONDS);
    let mut input = [0xa5; 4];
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut input), Buffer::Write(b"ping")].into_iter())
            .unwrap();
    let mut group = unsafe {
        set.begin_selected(ptr::null(), buffers, &wait, selection([0, 0]))
            .unwrap()
    };
    assert_eq!(context.prepares, 2);
    let mut output = [0; 4];
    assert_eq!(
        transfer(&mut peer, Buffer::Read(&mut output)),
        Awaited::complete(Ok(4))
    );
    assert_eq!(&output, b"ping");
    assert_eq!(
        transfer(&mut peer, Buffer::Write(b"pong")),
        Awaited::complete(Ok(4))
    );
    let mut seen = [false; 2];
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        while let Some(completion) = group.next(ptr::null()) {
            assert_eq!(completion.key, 91);
            assert!(!seen[completion.index]);
            seen[completion.index] = true;
            assert!(matches!(
                completion.outcome.transfer,
                Some(AdapterTerminal::Data(4) | AdapterTerminal::Accepted(4))
            ));
        }
    });
    assert_eq!(allocations, 0);
    assert_eq!(seen, [true; 2]);
    assert_eq!(context.completes, 2);
    assert_eq!(group.completed(0).unwrap(), b"pong");
    drop(group);
    set.native.as_ref().unwrap().pending_assert_retired();
    assert_ne!(
        set.native.as_ref().unwrap().cached_operation(0, 0),
        set.native.as_ref().unwrap().cached_operation(0, 1)
    );
    set.remove(91).unwrap();
    assert_eq!(set.records.occupied_len(), 0);
}

// Unlike the single-request fixtures, this adapter observes the operation's
// own callback view, never a registration's default OVERLAPPED address.
struct DuplexContext {
    endpoint: *const c_void,
    prepares: usize,
    completes: usize,
}

#[test]
fn competing_pending_reads_prepare_all_and_assign_bytes_in_plan_order() {
    let _serial = pending_test_lock();
    let (server, client) = connected_pair();
    let owner = Context::new(server, Selection::Caller);
    let mut context = DuplexContext {
        endpoint: owner.endpoint(),
        prepares: 0,
        completes: 0,
    };
    let adapter = DrPendingTransferAdapterV1 {
        context: DrAdapterContextV1 {
            object: ptr::from_mut(&mut context).cast(),
            descriptor: core::ptr::null(),
        },
        prepare_read: Some(duplex_prepare_read),
        prepare_write: Some(duplex_prepare_write),
        complete_read: Some(duplex_complete_read),
        complete_write: Some(duplex_complete_write),
    };
    let mut peer = peer(client);
    let mut set = unsafe { PendingSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add(91, owner.endpoint(), adapter).unwrap();
    }
    let (mut a, mut b) = ([0xa5; 3], [0xb5; 3]);
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut a), Buffer::Read(&mut b)].into_iter()).unwrap();
    let wait = budget(2, TEST_NANOSECONDS);
    let mut group = unsafe {
        set.begin_selected(ptr::null(), buffers, &wait, selection([0; 2]))
            .unwrap()
    };
    assert_eq!(context.prepares, 2);
    assert_eq!(context.completes, 0);
    assert_eq!(
        transfer(&mut peer, Buffer::Write(b"abcdef")),
        Awaited::complete(Ok(6))
    );
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        for index in 0..2 {
            let completion = group.next(ptr::null()).unwrap();
            assert_eq!(completion.index, index);
            assert!(matches!(
                completion.outcome.transfer,
                Some(AdapterTerminal::Data(3))
            ));
        }
        assert_eq!(group.completed(0).unwrap(), b"abc");
        assert_eq!(group.completed(1).unwrap(), b"def");
        assert!(group.next(ptr::null()).is_none());
        drop(group);
    });
    assert_eq!(allocations, 0);
    assert_eq!(context.completes, 2);
    set.native.as_ref().unwrap().pending_assert_retired();
    set.remove(91).unwrap();
}

unsafe extern "C" fn duplex_prepare_read(
    _: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    data: *mut u8,
    length: usize,
    _: *mut DrAdapterOutcomeV1,
    _: *mut DrInterfaceCarrierV1,
) -> u8 {
    let context = &mut *(*context).object.cast::<DuplexContext>();
    context.prepares += 1;
    dr_v1_pending_request_configure_native(view, context.endpoint, data, length);
    0
}

unsafe extern "C" fn duplex_prepare_write(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    data: *const u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    duplex_prepare_read(
        frame,
        context,
        view,
        data.cast_mut(),
        length,
        outcome,
        error,
    )
}

unsafe extern "C" fn duplex_complete_read(
    _: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    data: *mut u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    _: *mut DrInterfaceCarrierV1,
) -> u8 {
    let context = &mut *(*context).object.cast::<DuplexContext>();
    context.completes += 1;
    let (tag, submitted, status, selected, _) = pending_callback_facts(view);
    assert_eq!((*selected).endpoint, context.endpoint);
    assert_eq!((*selected).data, data);
    assert_eq!((*selected).length, length);
    assert_eq!((tag, submitted), (0, 1));
    outcome.write(DrAdapterOutcomeV1 {
        tag,
        count: (*status).value as usize,
        has_count: 1,
    });
    0
}

unsafe extern "C" fn duplex_complete_write(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    data: *const u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    duplex_complete_read(
        frame,
        context,
        view,
        data.cast_mut(),
        length,
        outcome,
        error,
    )
}

#[test]
fn repeated_attachment_selected_scratch_conflict_cleans_each_request_without_submission() {
    let _serial = pending_test_lock();
    let (server, _client) = connected_pair();
    let mut context = Context::new(server, Selection::Scratch);
    let mut set = unsafe { PendingSet::new(ptr::null_mut()).unwrap() };
    register(&mut set, 92, &mut context);
    let wait = budget(2, TEST_NANOSECONDS);
    let (mut first, mut second) = ([0xa5; 4], [0x5a; 4]);
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut first), Buffer::Read(&mut second)].into_iter())
            .unwrap();
    let failure = unsafe { set.begin_selected(ptr::null(), buffers, &wait, selection([0, 0])) }
        .err()
        .unwrap();
    assert!(
        matches!(failure, SetupFailure::Preparation(AdapterTerminal::PlatformFailure(failure))
        if failure == Failure::invalid_input())
    );
    assert_eq!((context.prepares, context.completes), (2, 2));
    assert_eq!((context.last.tag, context.last.submitted), (4, 0));
    assert_eq!(context.metered, 0);
    assert_eq!(first, [0xa5; 4]);
    assert_eq!(second, [0x5a; 4]);
    set.native.as_ref().unwrap().pending_assert_retired();
}

#[derive(Clone, Copy)]
enum Selection {
    Caller,
    Scratch,
    Foreign(*const c_void),
    NullData,
    PreparationError,
}

#[derive(Default)]
struct Completed {
    frame: usize,
    caller: usize,
    selected: usize,
    submitted: u8,
    tag: u8,
    reason: u8,
    code: Option<i64>,
    progress: Option<usize>,
    dequeue: usize,
}

struct Context {
    owner: EndpointOwner,
    selection: Selection,
    scratch: [u8; 8],
    prepares: usize,
    completes: usize,
    metered: usize,
    last_prepare_frame: usize,
    last_prepare_caller: usize,
    last: Completed,
    request: usize,
    invalid: bool,
    error: Option<OwnedIoError>,
    cancellation: *mut DrCancellationV1,
}

enum EndpointOwner {
    Attached(AttachedStream),
    File(OwnedFile),
}

impl Context {
    fn new(handle: StdHandle, selection: Selection) -> Box<Self> {
        // The real fixture factory supplies idle overlapped byte pipes;
        // adoption establishes this sole owner's persistent association.
        let owner = EndpointOwner::Attached(unsafe {
            AttachedStream::adopt(OwnedHandle::from_raw(handle.into_raw_handle())).unwrap()
        });
        Self::with_owner(owner, selection)
    }

    fn file(owner: OwnedFile, selection: Selection) -> Box<Self> {
        Self::with_owner(EndpointOwner::File(owner), selection)
    }

    fn with_owner(owner: EndpointOwner, selection: Selection) -> Box<Self> {
        Box::new(Self {
            owner,
            selection,
            scratch: [0; 8],
            prepares: 0,
            completes: 0,
            metered: 0,
            last_prepare_frame: 0,
            last_prepare_caller: 0,
            last: Completed::default(),
            request: 0,
            invalid: false,
            error: None,
            cancellation: ptr::null_mut(),
        })
    }

    fn endpoint(&self) -> *const c_void {
        match &self.owner {
            EndpointOwner::Attached(owner) => ptr::from_ref(owner).cast(),
            EndpointOwner::File(owner) => ptr::from_ref(owner).cast(),
        }
    }

    fn owned_file(&mut self) -> &mut OwnedFile {
        match &mut self.owner {
            EndpointOwner::File(owner) => owner,
            EndpointOwner::Attached(_) => panic!("typed File leaf required"),
        }
    }

    fn adapter(&mut self) -> DrPendingTransferAdapterV1 {
        DrPendingTransferAdapterV1 {
            context: DrAdapterContextV1 {
                object: (self as *mut Self).cast(),
                descriptor: core::ptr::null(),
            },
            prepare_read: Some(prepare_read),
            prepare_write: Some(prepare_write),
            complete_read: Some(complete_read),
            complete_write: Some(complete_write),
        }
    }
}

unsafe fn prepare(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    caller: *mut u8,
    length: usize,
    region: *mut DrPendingRegionV1,
    error: *mut DrInterfaceCarrierV1,
    reading: bool,
) -> u8 {
    let context = &mut *(*context).object.cast::<Context>();
    context.prepares += 1;
    context.last_prepare_frame = frame as usize;
    context.last_prepare_caller = caller as usize;
    if matches!(context.selection, Selection::PreparationError) {
        if let Some(owned) = context.error.take() {
            error.write(owned.into_carrier());
            return 1;
        }
        context.invalid = true;
    }
    let (endpoint, data, length) = match context.selection {
        Selection::Scratch => {
            let length = length.min(context.scratch.len());
            if !reading {
                // Explicit adapter transformation, not a runtime hidden copy.
                for (index, byte) in core::slice::from_raw_parts(caller, length)
                    .iter()
                    .enumerate()
                {
                    context.scratch[index] = byte.to_ascii_uppercase();
                }
            }
            (context.endpoint(), context.scratch.as_mut_ptr(), length)
        }
        Selection::Foreign(endpoint) => (endpoint, caller, length),
        Selection::NullData => (context.endpoint(), ptr::null_mut(), length),
        _ => (context.endpoint(), caller, length),
    };
    region.write(DrPendingRegionV1 {
        endpoint,
        data,
        length,
    });
    if !context.cancellation.is_null() {
        let (mut reason, mut code, mut has_code) = (0, 0, 0);
        if crate::cancellation::dr_v1_cancellation_request(
            context.cancellation,
            &mut reason,
            &mut code,
            &mut has_code,
        ) != 0
        {
            context.invalid = true;
        }
    }
    2
}

unsafe extern "C" fn prepare_read(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    caller: *mut u8,
    length: usize,
    _: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    prepare_view(frame, context, view, caller, length, error, true)
}

unsafe fn prepare_view(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    caller: *mut u8,
    length: usize,
    error: *mut DrInterfaceCarrierV1,
    reading: bool,
) -> u8 {
    let mut region = DrPendingRegionV1::default();
    let status = prepare(frame, context, caller, length, &mut region, error, reading);
    if status == 2 {
        dr_v1_pending_request_configure_native(view, region.endpoint, region.data, region.length);
        0
    } else {
        status
    }
}

unsafe extern "C" fn prepare_write(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    caller: *const u8,
    length: usize,
    _: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    prepare_view(
        frame,
        context,
        view,
        caller.cast_mut(),
        length,
        error,
        false,
    )
}

struct CompleteCall {
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    caller: *mut u8,
    bound: usize,
    tag: u8,
    submitted: u8,
    native: *const crate::io_status::DrIoStatusV1,
    selected: *const DrPendingRegionV1,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
    reading: bool,
}

unsafe fn complete(call: CompleteCall) -> u8 {
    let CompleteCall {
        frame,
        context,
        caller,
        bound,
        tag,
        submitted,
        native,
        selected,
        outcome,
        error,
        reading,
    } = call;
    let context = &mut *(*context).object.cast::<Context>();
    context.completes += 1;
    context.last = Completed {
        frame: frame as usize,
        caller: caller as usize,
        selected: (*selected).data as usize,
        submitted,
        tag,
        reason: (*native).reason,
        code: ((*native).has_system_code != 0).then_some((*native).system_code),
        progress: ((*native).has_transferred_byte_count != 0)
            .then_some((*native).transferred_byte_count),
        dequeue: pending_native_dequeues(),
    };
    if submitted != 0
        && (context.request == 0
            || ptr::read_volatile((*((context.request) as *const Overlapped)).internal.get())
                == STATUS_PENDING)
    {
        context.invalid = true;
    }
    if let Some(owned) = context.error.take() {
        error.write(owned.into_carrier());
        return 1;
    }
    if tag == 4 {
        // Successful staging cleanup has no underlying transfer result. The
        // shared protocol normalizes this callback success to NoTransfer.
        return 0;
    }
    if tag == 3 {
        // This test adapter explicitly maps a retired platform fault. Original
        // native facts remain recorded above; no callback starts another I/O.
        (*outcome).tag = if reading { 3 } else { 1 };
        return 0;
    }
    if tag == 2 && reading {
        (*outcome).tag = 2;
        return 0;
    }
    if tag == 1 {
        (*outcome).tag = 1;
        return 0;
    }
    let count = (*native).value as usize;
    if count > bound || count > (*selected).length {
        context.invalid = true;
        (*outcome).tag = if reading { 3 } else { 1 };
        return 0;
    }
    if reading && matches!(context.selection, Selection::Scratch) {
        let source = core::slice::from_raw_parts((*selected).data, count);
        let destination = core::slice::from_raw_parts_mut(caller, count);
        for (destination, source) in destination.iter_mut().zip(source) {
            *destination = source.to_ascii_uppercase();
        }
    }
    context.metered += count;
    (*outcome).count = count;
    0
}

unsafe extern "C" fn complete_read(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    caller: *mut u8,
    bound: usize,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    let (tag, submitted, native, selected, _) = pending_callback_facts(view);
    complete(CompleteCall {
        frame,
        context,
        caller,
        bound,
        tag,
        submitted,
        native,
        selected,
        outcome,
        error,
        reading: true,
    })
}

unsafe extern "C" fn complete_write(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    caller: *const u8,
    bound: usize,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    let (tag, submitted, native, selected, _) = pending_callback_facts(view);
    complete(CompleteCall {
        frame,
        context,
        caller: caller.cast_mut(),
        bound,
        tag,
        submitted,
        native,
        selected,
        outcome,
        error,
        reading: false,
    })
}

fn register(set: &mut PendingSet, key: u64, context: &mut Context) -> usize {
    let endpoint = context.endpoint();
    let adapter = context.adapter();
    let index = unsafe { set.add(key, endpoint, adapter) }.unwrap();
    context.request = set.native.as_ref().unwrap().pending_snapshot(index).2;
    index
}

fn register_file(set: &mut PendingSet, key: u64, context: &mut Context) -> usize {
    let file = match &mut context.owner {
        EndpointOwner::File(owner) => owner as *mut OwnedFile,
        EndpointOwner::Attached(_) => panic!("typed File registration required"),
    };
    let adapter = context.adapter();
    let index = unsafe { set.add_file(key, file, adapter) }.unwrap();
    let snapshot = set.native.as_ref().unwrap().pending_snapshot(index);
    assert_eq!(snapshot.1, 0, "File leaf must have no IOCP association");
    context.request = snapshot.2;
    index
}

fn selection<const N: usize>(indices: [usize; N]) -> FixedSlice<usize> {
    let mut selected = FixedSlice::new(N).unwrap();
    for index in indices {
        selected.push(index).unwrap();
    }
    selected
}

fn peer(handle: StdHandle) -> Pipe<WindowsDriver> {
    Pipe {
        driver: unsafe { WindowsDriver::new(handle.into_raw_handle()).unwrap() },
    }
}

fn transfer(peer: &mut Pipe<WindowsDriver>, buffer: Buffer<'_>) -> Awaited {
    let wait = budget(2, TEST_NANOSECONDS);
    let mut control = unsafe { WaitControl::new(Some(&wait), ptr::null_mut()).unwrap() };
    peer.transfer(buffer, &mut control)
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
fn attached_subset_reuses_requests_and_finishes_through_complete_adapter() {
    let _serial = pending_test_lock();
    let (server_a, client_a) = connected_pair();
    let (server_b, client_b) = connected_pair();
    let (server_c, client_c) = connected_pair();
    let mut a = Context::new(server_a, Selection::Scratch);
    let mut b = Context::new(server_b, Selection::Caller);
    let mut c = Context::new(server_c, Selection::Scratch);
    let mut peer_a = peer(client_a);
    let mut peer_b = peer(client_b);
    let mut peer_c = peer(client_c);
    let mut set = unsafe { PendingSet::new(ptr::null_mut()) }.unwrap();
    assert_eq!(register(&mut set, 10, &mut a), 0);
    let stable_a = set.native.as_ref().unwrap().pending_snapshot(0);
    let state_a = set.records.raw(0).unwrap();
    assert_eq!(register(&mut set, 20, &mut b), 1);
    assert_eq!(register(&mut set, 30, &mut c), 2);
    assert_eq!(set.native.as_ref().unwrap().pending_snapshot(0), stable_a);
    assert_eq!(set.records.raw(0), Some(state_a));
    assert_eq!(
        transfer(&mut peer_a, Buffer::Write(b"ab")),
        Awaited::complete(Ok(2))
    );
    assert_eq!(
        transfer(&mut peer_b, Buffer::Write(b"untouched")),
        Awaited::complete(Ok(9))
    );
    assert_eq!(
        transfer(&mut peer_c, Buffer::Write(b"cd")),
        Awaited::complete(Ok(2))
    );
    let wait = budget(2, TEST_NANOSECONDS);
    let begin_frame = frame(b"pending begin");
    let next_frame = frame(b"pending next");
    let submit_frame = frame(b"pending submit");
    let (mut bytes_a, mut bytes_c) = ([0x55; 4], [0x55; 4]);
    let pointer_a = bytes_a.as_mut_ptr() as usize;
    let pointer_c = bytes_c.as_mut_ptr() as usize;
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut bytes_c), Buffer::Read(&mut bytes_a)].into_iter())
            .unwrap();
    let indices = selection([2, 0]);
    let before = pending_native_dequeues();
    let (mut group, allocations) = crate::probe_runtime_allocations(|| unsafe {
        set.begin_selected(&begin_frame, buffers, &wait, indices)
            .unwrap()
    });
    assert_eq!(allocations, 0);
    assert_eq!(group.request_info(0), Some((true, 4)));
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        for _ in 0..2 {
            let completion = group.next(&next_frame).unwrap();
            assert!(matches!(
                completion.outcome.transfer,
                Some(AdapterTerminal::Data(2))
            ));
            match completion.key {
                30 => {
                    assert_eq!(completion.index, 0);
                    assert_eq!(group.completed(0), Some(b"CD".as_slice()));
                }
                10 => {
                    assert_eq!(completion.index, 1);
                    assert_eq!(group.completed(1), Some(b"AB".as_slice()));
                }
                _ => panic!("unexpected selected registration"),
            }
        }
        assert!(group.next(&next_frame).is_none());
        group.read_storage_mut(1).unwrap()[3] = 0x77;
        assert_eq!(
            transfer(&mut peer_a, Buffer::Write(b"xy")),
            Awaited::complete(Ok(2))
        );
        group.submit(&submit_frame, 1, 1, 2).unwrap();
        let completion = group.next(&next_frame).unwrap();
        assert_eq!((completion.key, completion.index), (10, 1));
        assert!(matches!(
            completion.outcome.transfer,
            Some(AdapterTerminal::Data(2))
        ));
        assert_eq!(group.completed(1), Some(b"XY".as_slice()));
        assert!(group.next(&next_frame).is_none());
        assert_eq!(group.stop(), None);
    });
    assert_eq!(allocations, 0);
    drop(group);
    set.native.as_ref().unwrap().pending_assert_retired();
    assert_eq!(pending_native_dequeues() - before, 3);
    assert_eq!((a.prepares, a.completes, a.metered), (2, 2, 4));
    assert_eq!((c.prepares, c.completes, c.metered), (1, 1, 2));
    assert_eq!((b.prepares, b.completes), (0, 0));
    assert_eq!(
        a.last_prepare_frame,
        &submit_frame as *const DrStackFrameV2 as usize
    );
    assert_eq!(
        c.last_prepare_frame,
        &begin_frame as *const DrStackFrameV2 as usize
    );
    assert_eq!(a.last_prepare_caller, pointer_a + 1);
    assert_eq!(c.last_prepare_caller, pointer_c);
    assert_eq!(a.last.caller, pointer_a + 1);
    assert_eq!(c.last.caller, pointer_c);
    assert_eq!(a.last.selected, a.scratch.as_ptr() as usize);
    assert_eq!(c.last.selected, c.scratch.as_ptr() as usize);
    assert_eq!((a.last.submitted, c.last.submitted), (1, 1));
    assert!(a.last.dequeue > before && c.last.dequeue > before);
    assert!(!a.invalid && !b.invalid && !c.invalid);
    assert_eq!(bytes_a[3], 0x77);
    let ((), closes) = with_close_probe(|| {
        set.remove(20).unwrap();
        assert_eq!(set.key(1), None);
        assert_eq!(set.native.as_ref().unwrap().pending_snapshot(0), stable_a);
        assert_eq!(set.records.raw(0), Some(state_a));
    });
    assert_eq!(closes, 0);
    assert_eq!(register(&mut set, 40, &mut b), 1);
    let mut untouched = [0; 9];
    let buffers = BufferList::from_iter([Buffer::Read(&mut untouched)].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(&begin_frame, buffers, &wait, selection([1]))
            .unwrap()
    };
    let completion = unsafe { group.next(&next_frame) }.unwrap();
    assert_eq!((completion.key, completion.index), (40, 0));
    assert!(matches!(
        completion.outcome.transfer,
        Some(AdapterTerminal::Data(9))
    ));
    assert_eq!(group.completed(0), Some(b"untouched".as_slice()));
    drop(group);
    // The same native slot can select adapter-owned transformed write storage.
    // The original write loan remains untouched and the peer observes the outer
    // adapter's bytes, not a bypass of its preparation/completion callbacks.
    let input = *b"pq";
    let buffers = BufferList::from_iter([Buffer::Write(&input)].into_iter()).unwrap();
    let indices = selection([0]);
    let before = pending_native_dequeues();
    let (mut group, allocations) = crate::probe_runtime_allocations(|| unsafe {
        set.begin_selected(&begin_frame, buffers, &wait, indices)
            .unwrap()
    });
    assert_eq!(allocations, 0);
    let mut received = [0; 2];
    assert_eq!(
        transfer(&mut peer_a, Buffer::Read(&mut received)),
        Awaited::complete(Ok(2))
    );
    assert_eq!(&received, b"PQ");
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        let completion = group.next(&next_frame).unwrap();
        assert_eq!((completion.key, completion.index), (10, 0));
        assert!(matches!(
            completion.outcome.transfer,
            Some(AdapterTerminal::Accepted(2))
        ));
        assert_eq!(group.completed(0), Some(input.as_slice()));
        assert!(group.next(&next_frame).is_none());
    });
    assert_eq!(allocations, 0);
    drop(group);
    assert_eq!(pending_native_dequeues() - before, 1);
    assert_eq!((a.prepares, a.completes, a.metered), (3, 3, 6));
    assert_eq!(set.native.as_ref().unwrap().pending_snapshot(0), stable_a);
    assert_eq!(&input, b"pq");
    let ((), closes) = with_close_probe(|| drop(set));
    assert_eq!(closes, 0);
    assert_eq!((b.prepares, b.completes), (1, 1));
}

#[test]
fn cancellation_and_drop_drain_each_native_packet_before_exactly_one_finish() {
    let _serial = pending_test_lock();
    let (server_a, client_a) = connected_pair();
    let (server_b, client_b) = connected_pair();
    let mut a = Context::new(server_a, Selection::Caller);
    let mut b = Context::new(server_b, Selection::Caller);
    let _peers = (peer(client_a), peer(client_b));
    let cancellation = TestClaim::new();
    let mut set = unsafe { PendingSet::new(cancellation.state()) }.unwrap();
    register(&mut set, 10, &mut a);
    register(&mut set, 20, &mut b);
    let identities = [
        set.native.as_ref().unwrap().pending_snapshot(0),
        set.native.as_ref().unwrap().pending_snapshot(1),
    ];
    let wait = budget(2, TEST_NANOSECONDS);
    let current_frame = frame(b"cancel completion");
    let (mut bytes_a, mut bytes_b) = ([0x77; 4], [0x88; 4]);
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut bytes_a), Buffer::Read(&mut bytes_b)].into_iter())
            .unwrap();
    let before = pending_native_dequeues();
    let mut group = unsafe {
        set.begin_selected(&current_frame, buffers, &wait, selection([0, 1]))
            .unwrap()
    };
    cancellation.request();
    for _ in 0..2 {
        let completion = unsafe { group.next(&current_frame) }.unwrap();
        assert_eq!(completion.outcome.stop, Some(Stop::Cancellation));
        assert!(matches!(
            completion.outcome.transfer,
            Some(AdapterTerminal::TimedOut)
        ));
    }
    assert!(unsafe { group.next(&current_frame) }.is_none());
    drop(group);
    set.native.as_ref().unwrap().pending_assert_retired();
    assert_eq!(pending_native_dequeues() - before, 2);
    for context in [&a, &b] {
        assert_eq!((context.prepares, context.completes), (1, 1));
        assert_eq!(
            (context.last.tag, context.last.submitted, context.last.code),
            (3, 1, Some(995))
        );
        assert_eq!(
            context.last.frame,
            &current_frame as *const DrStackFrameV2 as usize
        );
        assert!(!context.invalid);
    }
    let ((), closes) = with_close_probe(|| drop(set));
    assert_eq!(closes, 0);

    // Fresh set/cancellation context, same complete owners and native affinity.
    // Direct destruction must complete both operations with a null frame.
    let mut set = unsafe { PendingSet::new(ptr::null_mut()) }.unwrap();
    register(&mut set, 30, &mut a);
    register(&mut set, 40, &mut b);
    for (index, identity) in identities.iter().enumerate() {
        assert_eq!(
            &set.native.as_ref().unwrap().pending_snapshot(index).0,
            &identity.0
        );
        assert_eq!(
            &set.native.as_ref().unwrap().pending_snapshot(index).1,
            &identity.1
        );
    }
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut bytes_b), Buffer::Read(&mut bytes_a)].into_iter())
            .unwrap();
    let before = pending_native_dequeues();
    let group = unsafe {
        set.begin_selected(&current_frame, buffers, &wait, selection([1, 0]))
            .unwrap()
    };
    let ((), closes) = with_close_probe(|| drop(group));
    assert_eq!(closes, 0);
    set.native.as_ref().unwrap().pending_assert_retired();
    assert_eq!(pending_native_dequeues() - before, 2);
    for context in [&a, &b] {
        assert_eq!((context.prepares, context.completes), (2, 2));
        assert_eq!((context.last.frame, context.last.submitted), (0, 1));
        assert!(!context.invalid);
    }
    assert_eq!(bytes_a, [0x77; 4]);
    assert_eq!(bytes_b, [0x88; 4]);
}

struct ErrorPayload {
    drops: Arc<AtomicUsize>,
    message: &'static str,
    target: &'static str,
    code: i64,
    progress: i64,
}

unsafe extern "C" fn drop_error(_: *const DrStackFrameV2, object: *mut u8) {
    let payload = Box::from_raw(object.cast::<ErrorPayload>());
    payload.drops.fetch_add(1, Ordering::Relaxed);
}

fn descriptor() -> DrInterfaceVtableV1 {
    DrInterfaceVtableV1 {
        type_name: crate::IO_ERROR_TYPE.as_ptr(),
        type_name_length: crate::IO_ERROR_TYPE.len(),
        message_offset: 0,
        drop: drop_error,
        class_size: mem::size_of::<ErrorPayload>(),
        origin_offset: 0,
        assertion_offsets: 0,
        assertion_magic: 0,
        class_descriptor: ptr::null(),
        interface_id: 0,
        io_transferred_byte_count_offset: 0,
        io_progress_magic: 0,
    }
}

fn error(descriptor: &DrInterfaceVtableV1, drops: &Arc<AtomicUsize>) -> (OwnedIoError, *mut u8) {
    let object = Box::into_raw(Box::new(ErrorPayload {
        drops: Arc::clone(drops),
        message: "original complete adapter error",
        target: "unaltered/target",
        code: 91,
        progress: -7,
    }))
    .cast();
    (
        unsafe {
            OwnedIoError::from_carrier(DrInterfaceCarrierV1 {
                object,
                vtable: descriptor,
            })
        },
        object,
    )
}

fn assert_original(
    terminal: Option<AdapterTerminal>,
    object: *mut u8,
    descriptor: &DrInterfaceVtableV1,
) {
    let Some(AdapterTerminal::CheckedError { error, progress }) = terminal else {
        panic!("original checked carrier was not returned");
    };
    assert_eq!(progress, None);
    let carrier = error.into_carrier();
    assert_eq!(carrier.object, object);
    assert_eq!(carrier.vtable, descriptor as *const DrInterfaceVtableV1);
    unsafe {
        let payload = &*carrier.object.cast::<ErrorPayload>();
        assert_eq!(
            (
                payload.message,
                payload.target,
                payload.code,
                payload.progress
            ),
            (
                "original complete adapter error",
                "unaltered/target",
                91,
                -7
            )
        );
        drop(OwnedIoError::from_carrier(carrier));
    }
}

fn setup_error(
    result: Result<PendingGroup<'_, '_>, SetupFailure<AdapterTerminal>>,
) -> SetupFailure<AdapterTerminal> {
    match result {
        Err(error) => error,
        Ok(group) => {
            drop(group);
            panic!("invalid preparation unexpectedly began a native group");
        }
    }
}

#[test]
fn foreign_endpoint_malformed_region_and_preparation_error_reject_before_native_io() {
    let _serial = pending_test_lock();
    // Descriptor storage precedes every potential carrier owner, including
    // failure/unwind paths. The test payload owns its separate drop counter.
    let descriptor = descriptor();
    let drops = Arc::new(AtomicUsize::new(0));
    let (server, client) = connected_pair();
    let (foreign_server, foreign_client) = connected_pair();
    let mut foreign = Context::new(foreign_server, Selection::Caller);
    let mut context = Context::new(server, Selection::Foreign(foreign.endpoint()));
    let _peers = (peer(client), peer(foreign_client));
    let (owned, object) = error(&descriptor, &drops);
    context.error = Some(owned);
    let mut set = unsafe { PendingSet::new(ptr::null_mut()) }.unwrap();
    register(&mut set, 10, &mut context);
    let wait = budget(2, TEST_NANOSECONDS);
    let current_frame = frame(b"rejected endpoint");
    let mut bytes = [0x77; 4];
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let indices = selection([0]);
    let before = pending_native_dequeues();
    let (result, allocations) = crate::probe_runtime_allocations(|| unsafe {
        set.begin_selected(&current_frame, buffers, &wait, indices)
    });
    assert_eq!(allocations, 0);
    match setup_error(result) {
        SetupFailure::Preparation(terminal) => assert_original(Some(terminal), object, &descriptor),
        _ => panic!("checked staging cleanup error did not fail begin"),
    }
    assert_eq!(pending_native_dequeues(), before);
    assert_eq!((context.prepares, context.completes), (1, 1));
    assert_eq!(
        (
            context.last.tag,
            context.last.submitted,
            context.last.reason
        ),
        (4, 0, 0)
    );
    assert_eq!(context.last.code, None);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
    set.native.as_ref().unwrap().pending_assert_retired();
    set.remove(10).unwrap();

    context.selection = Selection::NullData;
    register(&mut set, 20, &mut context);
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let indices = selection([0]);
    let (result, allocations) = crate::probe_runtime_allocations(|| unsafe {
        set.begin_selected(&current_frame, buffers, &wait, indices)
    });
    assert_eq!(allocations, 0);
    assert!(matches!(setup_error(result),
        SetupFailure::Preparation(AdapterTerminal::PlatformFailure(failure))
            if failure == Failure::invalid_input()));
    assert_eq!((context.prepares, context.completes), (2, 2));
    assert_eq!(context.last.submitted, 0);
    assert_eq!(pending_native_dequeues(), before);
    set.remove(20).unwrap();

    context.selection = Selection::PreparationError;
    let (owned, object) = error(&descriptor, &drops);
    context.error = Some(owned);
    register(&mut set, 30, &mut context);
    register(&mut set, 40, &mut foreign);
    let mut foreign_bytes = [0x55; 4];
    let buffers = BufferList::from_iter(
        [Buffer::Read(&mut foreign_bytes), Buffer::Read(&mut bytes)].into_iter(),
    )
    .unwrap();
    let indices = selection([1, 0]);
    let (result, allocations) = crate::probe_runtime_allocations(|| unsafe {
        set.begin_selected(&current_frame, buffers, &wait, indices)
    });
    assert_eq!(allocations, 0);
    match setup_error(result) {
        SetupFailure::Preparation(terminal) => assert_original(Some(terminal), object, &descriptor),
        _ => panic!("preparation throw did not fail begin"),
    }
    assert_eq!((context.prepares, context.completes), (3, 2));
    assert_eq!(pending_native_dequeues(), before);
    assert_eq!(drops.load(Ordering::Relaxed), 2);
    assert_eq!((foreign.prepares, foreign.completes), (1, 1));
    assert_eq!(
        (
            foreign.last.tag,
            foreign.last.submitted,
            foreign.last.reason
        ),
        (4, 0, 0)
    );
    assert_eq!(foreign.last.code, None);
    assert_eq!(foreign_bytes, [0x55; 4]);
    assert!(!context.invalid);
    assert_eq!(bytes, [0x77; 4]);
    let ((), closes) = with_close_probe(|| drop(set));
    assert_eq!(closes, 0);
}

#[test]
fn attached_endpoint_identity_rejects_other_owner_and_readiness_value_before_dereference() {
    let _serial = pending_test_lock();
    let descriptor = descriptor();
    let drops = Arc::new(AtomicUsize::new(0));
    let (server, client) = connected_pair();
    let (other_server, other_client) = connected_pair();
    let other = Context::new(other_server, Selection::Caller);
    let mut context = Context::new(server, Selection::Caller);
    let _peers = (peer(client), peer(other_client));
    let mut set = unsafe { PendingSet::new(ptr::null_mut()) }.unwrap();
    register(&mut set, 10, &mut context);
    let readiness = crate::native_handle::ReadinessSource::new(
        set.native.as_ref().unwrap().pending_snapshot(0).0 as crate::native_handle::NativeHandle,
    );
    let wait = budget(2, TEST_NANOSECONDS);
    let current_frame = frame(b"attached identity before typed cast");
    let mut bytes = [0x77; 3];
    let before = pending_native_dequeues();
    for (position, endpoint) in [other.endpoint(), readiness.value()]
        .into_iter()
        .enumerate()
    {
        context.selection = Selection::Foreign(endpoint);
        let (owned, object) = error(&descriptor, &drops);
        context.error = Some(owned);
        let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
        let indices = selection([0]);
        let (result, allocations) = crate::probe_runtime_allocations(|| unsafe {
            set.begin_selected(&current_frame, buffers, &wait, indices)
        });
        assert_eq!(allocations, 0);
        match setup_error(result) {
            SetupFailure::Preparation(terminal) => {
                assert_original(Some(terminal), object, &descriptor)
            }
            _ => panic!("rejected attached endpoint did not preserve checked cleanup"),
        }
        assert_eq!(
            (context.prepares, context.completes),
            (position + 1, position + 1)
        );
        assert_eq!(
            (
                context.last.tag,
                context.last.submitted,
                context.last.reason
            ),
            (4, 0, 0)
        );
        assert_eq!((context.last.code, context.last.progress), (None, None));
        assert_eq!(context.last.selected, bytes.as_ptr() as usize);
        assert_eq!(bytes, [0x77; 3]);
        assert_eq!(pending_native_dequeues(), before);
        assert_eq!(drops.load(Ordering::Relaxed), position + 1);
        set.native.as_ref().unwrap().pending_assert_retired();
    }
    assert_eq!((other.prepares, other.completes), (0, 0));
    assert!(!context.invalid && !other.invalid);
    let ((), closes) = with_close_probe(|| drop(set));
    assert_eq!(closes, 0);
}

struct FileFixture {
    directory: std::path::PathBuf,
    path: std::path::PathBuf,
}

impl FileFixture {
    fn new(bytes: &[u8]) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "doria-windows-pending-file-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir(&directory).unwrap();
        let path = directory.join("Dória-漢字.bin");
        std::fs::write(&path, bytes).unwrap();
        Self { directory, path }
    }

    fn open(&self, append: bool) -> OwnedFile {
        OwnedFile::open(
            self.path.to_str().unwrap().as_bytes(),
            FileOpenRequest {
                access: FileAccess::ReadWrite,
                creation: FileCreation::OpenExisting,
                sharing: FileSharing::ReadWrite,
                append,
            },
        )
        .unwrap()
    }
}

impl Drop for FileFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_dir(&self.directory);
    }
}

#[test]
fn one_pending_file_cursor_orders_read_write_read_without_packets_or_hot_allocations() {
    let _serial = pending_test_lock();
    let fixture = FileFixture::new(b"abcdefghi");
    let mut file = fixture.open(false);
    let endpoint = ptr::from_mut(&mut file).cast_const().cast();
    let mut context = DuplexContext {
        endpoint,
        prepares: 0,
        completes: 0,
    };
    let adapter = DrPendingTransferAdapterV1 {
        context: DrAdapterContextV1 {
            object: ptr::from_mut(&mut context).cast(),
            descriptor: core::ptr::null(),
        },
        prepare_read: Some(duplex_prepare_read),
        prepare_write: Some(duplex_prepare_write),
        complete_read: Some(duplex_complete_read),
        complete_write: Some(duplex_complete_write),
    };
    let mut set = unsafe { PendingSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add_file(91, ptr::from_mut(&mut file), adapter).unwrap();
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
    let wait = budget(2, TEST_NANOSECONDS);
    let before = pending_native_dequeues();
    let mut group = unsafe {
        set.begin_selected(ptr::null(), buffers, &wait, selection([0; 3]))
            .unwrap()
    };
    assert_eq!(context.prepares, 3);
    assert_eq!(context.completes, 0);
    assert_eq!(std::fs::read(&fixture.path).unwrap(), b"abcdefghi");
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
    assert_eq!(context.completes, 3);
    assert_eq!(pending_native_dequeues(), before);
    assert_eq!(file.position().unwrap(), 7);
    assert_eq!(std::fs::read(&fixture.path).unwrap(), b"abcXYfghi");
    set.native.as_ref().unwrap().pending_assert_retired();
    set.remove(91).unwrap();
}

#[test]
fn regular_file_leaves_preserve_cursors_append_empty_eof_and_sole_close() {
    let _serial = pending_test_lock();
    let fixture = FileFixture::new(b"abcd");
    let mut a = Context::file(fixture.open(false), Selection::Scratch);
    let mut b = Context::file(fixture.open(false), Selection::Caller);
    let mut append = Context::file(fixture.open(true), Selection::Scratch);
    append.owned_file().seek(0, SeekOrigin::Start).unwrap();
    let mut set = unsafe { PendingSet::new(ptr::null_mut()) }.unwrap();
    assert_eq!(register_file(&mut set, 10, &mut a), 0);
    let identity = set.native.as_ref().unwrap().pending_snapshot(0);
    let state = set.records.raw(0).unwrap();
    assert_eq!(register_file(&mut set, 20, &mut b), 1);
    assert_eq!(register_file(&mut set, 30, &mut append), 2);
    assert_eq!(set.native.as_ref().unwrap().pending_snapshot(0), identity);
    assert_eq!(set.records.raw(0), Some(state));
    let wait = budget(2, TEST_NANOSECONDS);
    let current_frame = frame(b"pending regular File");
    let (mut bytes_a, mut bytes_b) = ([0x77; 4], [0x88; 2]);
    let caller_a = bytes_a.as_mut_ptr() as usize;
    let buffers = BufferList::from_iter(
        [Buffer::Read(&mut bytes_b), Buffer::Read(&mut bytes_a[..2])].into_iter(),
    )
    .unwrap();
    let indices = selection([1, 0]);
    let before = pending_native_dequeues();
    let (mut group, allocations) = crate::probe_runtime_allocations(|| unsafe {
        set.begin_selected(&current_frame, buffers, &wait, indices)
            .unwrap()
    });
    assert_eq!(allocations, 0);
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        for _ in 0..2 {
            let completion = group.next(&current_frame).unwrap();
            assert!(matches!(
                completion.outcome.transfer,
                Some(AdapterTerminal::Data(2))
            ));
            match completion.key {
                10 => assert_eq!(group.completed(completion.index), Some(b"AB".as_slice())),
                20 => assert_eq!(group.completed(completion.index), Some(b"ab".as_slice())),
                _ => panic!("unexpected File completion"),
            }
        }
        group.submit(&current_frame, 1, 0, 2).unwrap();
        let completion = group.next(&current_frame).unwrap();
        assert_eq!((completion.index, completion.key), (1, 10));
        assert!(matches!(
            completion.outcome.transfer,
            Some(AdapterTerminal::Data(2))
        ));
        assert_eq!(group.completed(1), Some(b"CD".as_slice()));
        assert!(group.next(&current_frame).is_none());
    });
    assert_eq!(allocations, 0);
    drop(group);
    assert_eq!(pending_native_dequeues(), before);
    set.native.as_ref().unwrap().pending_assert_retired();
    assert_eq!(a.owned_file().position().unwrap(), 4);
    assert_eq!(b.owned_file().position().unwrap(), 2);
    assert_eq!(a.last.caller, caller_a);
    assert_eq!(a.last.selected, a.scratch.as_ptr() as usize);
    assert_eq!((a.prepares, a.completes, a.metered), (2, 2, 4));
    assert_eq!((append.prepares, append.completes), (0, 0));
    let ((), closes) = with_close_probe(|| set.remove(10).unwrap());
    assert_eq!(closes, 0);
    assert_eq!(register_file(&mut set, 40, &mut a), 0);

    // An empty read is Data(0), including at EOF. A following nonempty read
    // preserves the actual File EndOfStream fact instead of inventing an errno.
    let mut eof_storage = [0x55; 2];
    let buffers = BufferList::from_iter([Buffer::Read(&mut eof_storage)].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(&current_frame, buffers, &wait, selection([0]))
            .unwrap()
    };
    assert!(matches!(
        unsafe { group.next(&current_frame) }
            .unwrap()
            .outcome
            .transfer,
        Some(AdapterTerminal::EndOfStream)
    ));
    unsafe { group.submit(&current_frame, 0, 0, 0) }.unwrap();
    assert!(matches!(
        unsafe { group.next(&current_frame) }
            .unwrap()
            .outcome
            .transfer,
        Some(AdapterTerminal::Data(0))
    ));
    drop(group);
    assert_eq!((a.last.tag, a.last.submitted), (0, 1));
    assert_eq!(eof_storage, [0x55; 2]);
    assert_eq!(a.owned_file().position().unwrap(), 4);

    let input = *b"xy";
    let buffers = BufferList::from_iter([Buffer::Write(&input)].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(&current_frame, buffers, &wait, selection([2]))
            .unwrap()
    };
    assert!(matches!(
        unsafe { group.next(&current_frame) }
            .unwrap()
            .outcome
            .transfer,
        Some(AdapterTerminal::Accepted(2))
    ));
    drop(group);
    assert_eq!(&input, b"xy");
    assert_eq!(std::fs::read(&fixture.path).unwrap(), b"abcdXY");
    assert_eq!(append.owned_file().position().unwrap(), 6);
    assert_eq!(append.last.selected, append.scratch.as_ptr() as usize);
    assert_eq!(pending_native_dequeues(), before);
    assert!(!a.invalid && !b.invalid && !append.invalid);
    let ((), closes) = with_close_probe(|| drop(set));
    assert_eq!(closes, 0);
    let ((), closes) = with_close_probe(|| {
        drop(a);
        drop(b);
        drop(append);
    });
    assert_eq!(closes, 3);
}

#[test]
fn file_preparation_cancellation_does_no_io_and_completion_keeps_original_error() {
    let _serial = pending_test_lock();
    let descriptor = descriptor();
    let drops = Arc::new(AtomicUsize::new(0));
    let fixture = FileFixture::new(b"abc");
    let mut context = Context::file(fixture.open(false), Selection::Caller);
    let cancellation = TestClaim::new();
    context.cancellation = cancellation.state();
    let mut set = unsafe { PendingSet::new(cancellation.state()) }.unwrap();
    register_file(&mut set, 10, &mut context);
    let before = pending_native_dequeues();
    let wait = budget(2, TEST_NANOSECONDS);
    let current_frame = frame(b"cancel staged File");
    let mut bytes = [0x77; 3];
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(&current_frame, buffers, &wait, selection([0]))
            .unwrap()
    };
    let completion = unsafe { group.next(&current_frame) }.unwrap();
    assert_eq!(completion.outcome.stop, Some(Stop::Cancellation));
    assert!(completion.outcome.transfer.is_none());
    drop(group);
    assert_eq!((context.prepares, context.completes), (1, 1));
    assert_eq!((context.last.tag, context.last.submitted), (4, 0));
    assert_eq!((context.last.reason, context.last.code), (0, None));
    assert_eq!(context.owned_file().position().unwrap(), 0);
    assert_eq!(bytes, [0x77; 3]);
    let ((), closes) = with_close_probe(|| drop(set));
    assert_eq!(closes, 0);

    context.cancellation = ptr::null_mut();
    // The opaque readiness VALUE is not the typed complete File endpoint.
    // Reject it by identity before any cast/dereference or file submission.
    context.selection = Selection::Foreign(context.owned_file().readiness_source().value());
    let mut rejected = unsafe { PendingSet::new(ptr::null_mut()) }.unwrap();
    register_file(&mut rejected, 20, &mut context);
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let result = unsafe { rejected.begin_selected(&current_frame, buffers, &wait, selection([0])) };
    assert!(matches!(setup_error(result),
        SetupFailure::Preparation(AdapterTerminal::PlatformFailure(failure))
            if failure == Failure::invalid_input()));
    assert_eq!((context.prepares, context.completes), (2, 2));
    assert_eq!((context.last.tag, context.last.submitted), (4, 0));
    assert_eq!(context.owned_file().position().unwrap(), 0);
    assert_eq!(bytes, [0x77; 3]);
    drop(rejected);

    context.selection = Selection::Caller;
    let (owned, object) = error(&descriptor, &drops);
    context.error = Some(owned);
    let mut set = unsafe { PendingSet::new(ptr::null_mut()) }.unwrap();
    register_file(&mut set, 20, &mut context);
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(&current_frame, buffers, &wait, selection([0]))
            .unwrap()
    };
    let completion = unsafe { group.next(&current_frame) }.unwrap();
    assert_original(completion.outcome.transfer, object, &descriptor);
    assert!(unsafe { group.next(&current_frame) }.is_none());
    drop(group);
    assert_eq!(context.owned_file().position().unwrap(), 3);
    assert_eq!(bytes, *b"abc");
    assert_eq!((context.prepares, context.completes), (3, 3));
    assert_eq!((context.last.tag, context.last.submitted), (0, 1));
    assert_eq!(drops.load(Ordering::Relaxed), 1);
    assert_eq!(pending_native_dequeues(), before);
    assert!(!context.invalid);
    let ((), closes) = with_close_probe(|| drop(set));
    assert_eq!(closes, 0);
    let ((), closes) = with_close_probe(|| drop(context));
    assert_eq!(closes, 1);
}

#[test]
fn mixed_file_pipe_subset_preserves_idle_file_and_drains_only_real_pipe_packets() {
    let _serial = pending_test_lock();
    let fixture = FileFixture::new(b"abcd");
    let mut file = Context::file(fixture.open(false), Selection::Caller);
    let mut idle = Context::file(fixture.open(false), Selection::Caller);
    let (server, client) = connected_pair();
    let mut pipe = Context::new(server, Selection::Caller);
    let mut peer = peer(client);
    let mut set = unsafe { PendingSet::new(ptr::null_mut()) }.unwrap();
    register_file(&mut set, 10, &mut file);
    register(&mut set, 20, &mut pipe);
    let identities = [
        set.native.as_ref().unwrap().pending_snapshot(0),
        set.native.as_ref().unwrap().pending_snapshot(1),
    ];
    register_file(&mut set, 30, &mut idle);
    assert_eq!(
        set.native.as_ref().unwrap().pending_snapshot(0),
        identities[0]
    );
    assert_eq!(
        set.native.as_ref().unwrap().pending_snapshot(1),
        identities[1]
    );
    assert_eq!(
        transfer(&mut peer, Buffer::Write(b"q")),
        Awaited::complete(Ok(1))
    );
    let wait = budget(2, TEST_NANOSECONDS);
    let current_frame = frame(b"mixed pending leaves");
    let (mut pipe_bytes, mut file_bytes) = ([0x66; 1], [0x77; 2]);
    let buffers = BufferList::from_iter(
        [Buffer::Read(&mut pipe_bytes), Buffer::Read(&mut file_bytes)].into_iter(),
    )
    .unwrap();
    let before = pending_native_dequeues();
    let mut group = unsafe {
        set.begin_selected(&current_frame, buffers, &wait, selection([1, 0]))
            .unwrap()
    };
    for _ in 0..2 {
        let completion = unsafe { group.next(&current_frame) }.unwrap();
        match completion.key {
            10 => assert!(matches!(
                completion.outcome.transfer,
                Some(AdapterTerminal::Data(2))
            )),
            20 => assert!(matches!(
                completion.outcome.transfer,
                Some(AdapterTerminal::Data(1))
            )),
            _ => panic!("unexpected mixed completion"),
        }
    }
    drop(group);
    set.native.as_ref().unwrap().pending_assert_retired();
    assert_eq!(pending_native_dequeues() - before, 1);
    assert_eq!(file_bytes, *b"ab");
    assert_eq!(pipe_bytes, *b"q");
    assert_eq!((idle.prepares, idle.completes), (0, 0));
    assert_eq!(idle.owned_file().position().unwrap(), 0);
    set.remove(30).unwrap();
    assert_eq!(register_file(&mut set, 40, &mut idle), 2);
    assert_eq!(
        set.native.as_ref().unwrap().pending_snapshot(0),
        identities[0]
    );
    assert_eq!(
        set.native.as_ref().unwrap().pending_snapshot(1),
        identities[1]
    );

    // File submit wins before return; pipe read is genuinely still pending.
    // Direct Drop preserves the File result and cancels/drains only the pipe.
    let buffers = BufferList::from_iter(
        [Buffer::Read(&mut file_bytes), Buffer::Read(&mut pipe_bytes)].into_iter(),
    )
    .unwrap();
    let before = pending_native_dequeues();
    let group = unsafe {
        set.begin_selected(&current_frame, buffers, &wait, selection([0, 1]))
            .unwrap()
    };
    drop(group);
    assert_eq!(pending_native_dequeues() - before, 1);
    assert_eq!(file_bytes, *b"cd");
    assert_eq!((file.prepares, file.completes, file.metered), (2, 2, 4));
    assert_eq!(
        (file.last.frame, file.last.tag, file.last.submitted),
        (0, 0, 1)
    );
    assert_eq!((pipe.prepares, pipe.completes), (2, 2));
    assert_eq!(
        (pipe.last.frame, pipe.last.tag, pipe.last.code),
        (0, 3, Some(995))
    );
    assert_eq!((idle.prepares, idle.completes), (0, 0));
    assert_eq!(idle.owned_file().position().unwrap(), 0);
    assert!(!file.invalid && !pipe.invalid && !idle.invalid);
    set.native.as_ref().unwrap().pending_assert_retired();
    let ((), closes) = with_close_probe(|| drop(set));
    assert_eq!(closes, 0);
}

#[test]
fn actual_nonregular_file_is_rejected_before_callbacks_or_native_transfer() {
    let _serial = pending_test_lock();
    // NUL is an actual synchronous Windows device owner, not fabricated File
    // storage or a cast of an AttachedStream. It is intentionally not DISK.
    let owner = OwnedFile::open(
        b"NUL",
        FileOpenRequest {
            access: FileAccess::ReadWrite,
            creation: FileCreation::OpenExisting,
            append: false,
            sharing: FileSharing::ReadWrite,
        },
    )
    .unwrap();
    let mut context = Context::file(owner, Selection::Caller);
    let mut set = unsafe { PendingSet::new(ptr::null_mut()) }.unwrap();
    let file = context.owned_file() as *mut OwnedFile;
    let adapter = context.adapter();
    let before = pending_native_dequeues();
    let ((result, allocations), closes) = with_close_probe(|| {
        crate::probe_runtime_allocations(|| unsafe { set.add_file(10, file, adapter) })
    });
    assert_eq!(result, Err(Failure::unsupported()));
    assert_eq!((allocations, closes), (0, 0));
    assert_eq!((context.prepares, context.completes), (0, 0));
    assert_eq!(set.key(0), None);
    assert_eq!(pending_native_dequeues(), before);

    // An admitted real File syscall failure is still a submitted, retired
    // operation. Keep the exact checked platform facts (including unknown
    // progress) rather than converting it to an unsubmitted setup rejection.
    let fixture = FileFixture::new(b"abc");
    let owner = OwnedFile::open(
        fixture.path.to_str().unwrap().as_bytes(),
        FileOpenRequest::READ,
    )
    .unwrap();
    let mut readonly = Context::file(owner, Selection::Caller);
    let index = register_file(&mut set, 20, &mut readonly);
    let wait = budget(2, TEST_NANOSECONDS);
    let current_frame = frame(b"failed submitted File write");
    let input = *b"x";
    let buffers = BufferList::from_iter([Buffer::Write(&input)].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(&current_frame, buffers, &wait, selection([index]))
            .unwrap()
    };
    assert!(matches!(
        unsafe { group.next(&current_frame) }
            .unwrap()
            .outcome
            .transfer,
        Some(AdapterTerminal::WouldBlock)
    ));
    drop(group);
    assert_eq!(
        (
            readonly.last.tag,
            readonly.last.submitted,
            readonly.last.code
        ),
        (3, 1, Some(5))
    );
    assert_eq!(readonly.last.progress, None);
    assert_eq!(readonly.owned_file().position().unwrap(), 0);
    assert_eq!(std::fs::read(&fixture.path).unwrap(), b"abc");
    assert_eq!(pending_native_dequeues(), before);
    assert!(!readonly.invalid);
    let ((), closes) = with_close_probe(|| drop(set));
    assert_eq!(closes, 0);
    let ((), closes) = with_close_probe(|| drop(context));
    assert_eq!(closes, 1);
    let ((), closes) = with_close_probe(|| drop(readonly));
    assert_eq!(closes, 1);
}

#[test]
fn file_terminal_projection_retains_typed_tags_without_restoring_rejected_progress() {
    // Deterministic private fact projection only. In particular, these injected
    // WouldBlock facts do not claim an actual regular file produced WouldBlock.
    let failure = Failure::other();
    let facts = [
        (FileTerminal::Read(Ok(ReadProgress::Data(0))), 0),
        (FileTerminal::Read(Ok(ReadProgress::EndOfStream)), 2),
        (FileTerminal::Read(Ok(ReadProgress::WouldBlock(failure))), 1),
        (FileTerminal::Write(Ok(WriteProgress::Accepted(0))), 0),
        (
            FileTerminal::Write(Ok(WriteProgress::WouldBlock(failure))),
            1,
        ),
    ];
    for (actual, expected_tag) in facts {
        let ((tag, status), allocations) = crate::probe_runtime_allocations(|| {
            actual.bounded_status(actual.result().bounded(3), 3)
        });
        assert_eq!(allocations, 0);
        assert_eq!(tag, expected_tag);
        assert_eq!(status.value, 0);
        assert_eq!(status.has_transferred_byte_count, 0);
        assert_eq!(status.has_system_code, 0);
    }

    for actual in [
        FileTerminal::Read(Ok(ReadProgress::Data(4))),
        FileTerminal::Write(Ok(WriteProgress::Accepted(4))),
    ] {
        let bounded = actual.result().bounded(3);
        assert_eq!(bounded, Err(Failure::other()));
        let (tag, status) = actual.bounded_status(bounded, 3);
        assert_eq!(tag, 3);
        assert_eq!(status.reason, Failure::other().reason as u8);
        assert_eq!(status.value, 0);
        assert_eq!(status.has_transferred_byte_count, 0);
    }

    let failure = Failure::from_system_code(91).with_transferred_byte_count(Some(4));
    for actual in [
        FileTerminal::Read(Err(failure)),
        FileTerminal::Write(Err(failure)),
    ] {
        let bounded = actual.result().bounded(3);
        assert_eq!(bounded, Err(failure.with_transferred_byte_count(None)));
        let (tag, status) = actual.bounded_status(bounded, 3);
        assert_eq!(tag, 3);
        assert_eq!(status.reason, failure.reason as u8);
        assert_eq!((status.has_system_code, status.system_code), (1, 91));
        assert_eq!(status.has_transferred_byte_count, 0);
        assert_eq!(status.value, 0);
    }
}
