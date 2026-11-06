use super::*;
use crate::transfer_group::adapter::DrAdapterContextV1;
use core::{mem, ptr};
use std::cell::{Cell, RefCell};
use std::io::{Read, Write};
use std::os::fd::IntoRawFd;
use std::os::unix::net::UnixStream;
use std::rc::Rc;

use crate::io_status::DrIoStatusV1;
use crate::native_handle::OwnedHandle;
use crate::transfer_group::adapter::{DrAdapterOutcomeV1, OwnedIoError};
use crate::transfer_group::pending::{
    dr_v1_pending_request_complete, dr_v1_pending_request_configure_native, pending_callback_facts,
    DrPendingRegionV1,
};
use crate::transfer_group::Buffer;
use crate::DrInterfaceCarrierV1;

#[path = "file_tests.rs"]
mod file_tests;

#[path = "standard_tests.rs"]
mod standard_tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Call {
    Empty,
    Prepare {
        index: usize,
        frame: usize,
        caller: usize,
        length: usize,
    },
    Complete {
        index: usize,
        frame: usize,
        caller: usize,
        length: usize,
        tag: u8,
        submitted: u8,
        count: usize,
        progress: Option<usize>,
        selected: (usize, usize, usize),
        immediate: Option<(u8, usize)>,
    },
}

struct Log {
    calls: [Call; 32],
    used: usize,
}

impl Log {
    fn new() -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            calls: [Call::Empty; 32],
            used: 0,
        }))
    }

    fn push(&mut self, call: Call) {
        self.calls[self.used] = call;
        self.used += 1;
    }

    fn calls(&self) -> &[Call] {
        &self.calls[..self.used]
    }
}

struct Context {
    index: usize,
    endpoint: *const c_void,
    log: Rc<RefCell<Log>>,
    storage: [u8; 8],
    selected_length: Option<usize>,
    transform: bool,
    immediate: Option<DrAdapterOutcomeV1>,
    outer: Option<DrAdapterOutcomeV1>,
    prepare_error: Option<OwnedIoError>,
    failure_error: Option<OwnedIoError>,
    cancel_on_prepare: *mut DrCancellationV1,
}

impl Context {
    fn new(index: usize, endpoint: *const c_void, log: &Rc<RefCell<Log>>) -> Box<Self> {
        Box::new(Self {
            index,
            endpoint,
            log: Rc::clone(log),
            storage: [0; 8],
            selected_length: None,
            transform: false,
            immediate: None,
            outer: None,
            prepare_error: None,
            failure_error: None,
            cancel_on_prepare: ptr::null_mut(),
        })
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
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    let context = &mut *(*context).object.cast::<Context>();
    context.log.borrow_mut().push(Call::Prepare {
        index: context.index,
        frame: frame as usize,
        caller: caller as usize,
        length,
    });
    if !context.cancel_on_prepare.is_null() {
        request_cancellation(context.cancel_on_prepare);
    }
    if let Some(owned) = context.prepare_error.take() {
        error.write(owned.into_carrier());
        return 1;
    }
    if let Some(immediate) = context.immediate {
        outcome.write(immediate);
        return 0;
    }
    *region = DrPendingRegionV1 {
        endpoint: context.endpoint,
        data: if context.selected_length.is_some() {
            context.storage.as_mut_ptr()
        } else {
            caller
        },
        length: context.selected_length.unwrap_or(length),
    };
    2
}

unsafe extern "C" fn prepare_read(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    caller: *mut u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    prepare_view(frame, context, view, caller, length, outcome, error)
}

unsafe fn prepare_view(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    caller: *mut u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    let mut region = DrPendingRegionV1::default();
    match prepare(frame, context, caller, length, &mut region, outcome, error) {
        2 => {
            dr_v1_pending_request_configure_native(
                view,
                region.endpoint,
                region.data,
                region.length,
            );
            0
        }
        0 => {
            dr_v1_pending_request_complete(view, (*outcome).tag, (*outcome).count);
            0
        }
        status => status,
    }
}

unsafe extern "C" fn prepare_write(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    caller: *const u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    prepare_view(
        frame,
        context,
        view,
        caller.cast_mut(),
        length,
        outcome,
        error,
    )
}

struct CompleteCall {
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    caller: *mut u8,
    length: usize,
    tag: u8,
    submitted: u8,
    native: *const DrIoStatusV1,
    selected: *const DrPendingRegionV1,
    immediate: *const DrAdapterOutcomeV1,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
}

unsafe fn complete(call: CompleteCall, reading: bool) -> u8 {
    let context = &mut *(*call.context).object.cast::<Context>();
    let native = &*call.native;
    let selected = &*call.selected;
    let immediate = call.immediate.as_ref();
    context.log.borrow_mut().push(Call::Complete {
        index: context.index,
        frame: call.frame as usize,
        caller: call.caller as usize,
        length: call.length,
        tag: call.tag,
        submitted: call.submitted,
        count: native.value as usize,
        progress: (native.has_transferred_byte_count != 0).then_some(native.transferred_byte_count),
        selected: (
            selected.endpoint as usize,
            selected.data as usize,
            selected.length,
        ),
        immediate: immediate.map(|outcome| (outcome.tag, outcome.count)),
    });
    if call.tag == 3 || (call.tag == 4 && context.failure_error.is_some()) {
        let error = context
            .failure_error
            .take()
            .expect("configured exact adapter error");
        call.error.write(error.into_carrier());
        return 1;
    }
    if call.tag == 4 {
        // Cleanup succeeds without manufacturing a transfer outcome.
        return 0;
    }
    if reading && context.transform && call.tag == 0 && call.submitted != 0 {
        assert!(native.value as usize <= call.length);
        for index in 0..native.value as usize {
            call.caller
                .add(index)
                .write(selected.data.add(index).read() ^ 0xff);
        }
    }
    call.outcome
        .write(
            context
                .outer
                .or_else(|| immediate.copied())
                .unwrap_or(DrAdapterOutcomeV1 {
                    count: native.value as usize,
                    tag: if call.tag == 1 {
                        1
                    } else if call.tag == 2
                        || (reading && selected.length != 0 && native.value == 0)
                    {
                        2
                    } else {
                        0
                    },
                    has_count: 0,
                }),
        );
    0
}

unsafe extern "C" fn complete_read(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    caller: *mut u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    let (tag, submitted, native, selected, immediate) = pending_callback_facts(view);
    complete(
        CompleteCall {
            frame,
            context,
            caller,
            length,
            tag,
            submitted,
            native,
            selected,
            immediate,
            outcome,
            error,
        },
        true,
    )
}

unsafe extern "C" fn complete_write(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    caller: *const u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    let (tag, submitted, native, selected, immediate) = pending_callback_facts(view);
    complete(
        CompleteCall {
            frame,
            context,
            caller: caller.cast_mut(),
            length,
            tag,
            submitted,
            native,
            selected,
            immediate,
            outcome,
            error,
        },
        false,
    )
}

fn source() -> (Box<OwnedHandle>, UnixStream) {
    let (stream, peer) = UnixStream::pair().unwrap();
    stream.set_nonblocking(true).unwrap();
    (
        Box::new(unsafe { OwnedHandle::from_raw(stream.into_raw_fd()) }),
        peer,
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

fn budget(kind: u8) -> DrWaitBudgetV1 {
    let mut value = mem::MaybeUninit::uninit();
    let (mut reason, mut code, mut has_code) = (0, 0, 0);
    assert_eq!(
        unsafe {
            crate::wait_budget::dr_v1_wait_budget_init(
                kind,
                0,
                value.as_mut_ptr(),
                &mut reason,
                &mut code,
                &mut has_code,
            )
        },
        0
    );
    unsafe { value.assume_init() }
}

fn selection(indices: &[usize]) -> FixedSlice<usize> {
    let mut selected = FixedSlice::new(indices.len()).unwrap();
    for index in indices {
        selected.push(*index).unwrap();
    }
    selected
}

unsafe fn request_cancellation(cancellation: *mut DrCancellationV1) {
    let (mut reason, mut code, mut has_code) = (0, 0, 0);
    assert_eq!(
        crate::cancellation::dr_v1_cancellation_request(
            cancellation,
            &mut reason,
            &mut code,
            &mut has_code
        ),
        0
    );
}

struct ExactError {
    drops: Rc<Cell<usize>>,
}

unsafe extern "C" fn drop_error(_: *const DrStackFrameV2, object: *mut u8) {
    let error = Box::from_raw(object.cast::<ExactError>());
    error.drops.set(error.drops.get() + 1);
}

fn descriptor() -> crate::DrInterfaceVtableV1 {
    crate::DrInterfaceVtableV1 {
        type_name: crate::IO_ERROR_TYPE.as_ptr(),
        type_name_length: crate::IO_ERROR_TYPE.len(),
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
    }
}

fn exact_error(
    descriptor: &crate::DrInterfaceVtableV1,
    drops: &Rc<Cell<usize>>,
) -> (OwnedIoError, *mut u8) {
    let object = Box::into_raw(Box::new(ExactError {
        drops: Rc::clone(drops),
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

#[test]
fn same_registration_duplex_prepares_all_distinct_requests_and_completes_each_once() {
    let (owner, mut peer) = source();
    let log = Log::new();
    let mut context = Context::new(0, owner.native_handle_home(), &log);
    let mut set = unsafe { PendingSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add(19, owner.native_handle_home(), context.adapter())
            .unwrap();
    }
    let wait = budget(1);
    let mut input = [0xa5; 4];
    let input_pointer = input.as_ptr();
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut input), Buffer::Write(b"ping")].into_iter())
            .unwrap();
    let mut group = unsafe {
        set.begin_selected(ptr::null(), buffers, &wait, selection(&[0, 0]))
            .unwrap()
    };
    assert!(matches!(
        log.borrow().calls()[..2],
        [Call::Prepare { .. }, Call::Prepare { .. }]
    ));
    let mut output = [0; 4];
    peer.read_exact(&mut output).unwrap();
    assert_eq!(&output, b"ping");
    peer.write_all(b"pong").unwrap();
    let mut seen = [false; 2];
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        while let Some(completion) = group.next(ptr::null()) {
            assert_eq!(completion.key, 19);
            assert!(!seen[completion.index]);
            seen[completion.index] = true;
            assert!(matches!(
                completion.outcome.transfer,
                Some(AdapterTerminal::Data(4) | AdapterTerminal::Accepted(4))
            ));
        }
    });
    assert_eq!(allocations, 0);
    assert_eq!(seen, [true, true]);
    assert_eq!(group.completed(0).unwrap(), b"pong");
    assert_eq!(group.read_storage_mut(0).unwrap().as_ptr(), input_pointer);
    drop(group);
    assert_ne!(
        set.native.as_mut().unwrap().cached_operation(0, 0),
        set.native.as_mut().unwrap().cached_operation(0, 1)
    );
    assert_eq!(
        log.borrow()
            .calls()
            .iter()
            .filter(|call| matches!(call, Call::Complete { .. }))
            .count(),
        2
    );
    set.remove(19).unwrap();
    assert_eq!(set.records.occupied_len(), 0);
    assert_eq!(set.registrations.occupied_len(), 0);
    peer.write_all(b"live").unwrap();
}

#[test]
fn competing_pending_reads_prepare_together_and_receive_plan_ordered_bytes() {
    let (owner, mut peer) = source();
    let log = Log::new();
    let mut context = Context::new(0, owner.native_handle_home(), &log);
    let mut set = unsafe { PendingSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add(19, owner.native_handle_home(), context.adapter())
            .unwrap();
    }
    let (mut a, mut b) = ([0xa5; 3], [0xb5; 3]);
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut a), Buffer::Read(&mut b)].into_iter()).unwrap();
    let wait = budget(1);
    let mut group = unsafe {
        set.begin_selected(ptr::null(), buffers, &wait, selection(&[0; 2]))
            .unwrap()
    };
    assert!(matches!(
        log.borrow().calls(),
        [Call::Prepare { .. }, Call::Prepare { .. }]
    ));
    peer.write_all(b"abcdef").unwrap();
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
    assert_eq!(
        log.borrow()
            .calls()
            .iter()
            .filter(|call| matches!(call, Call::Complete { .. }))
            .count(),
        2
    );
    set.remove(19).unwrap();
}

#[test]
fn repeated_registration_cannot_share_writable_selected_storage_and_cleans_all_preparations() {
    let (owner, _peer) = source();
    let log = Log::new();
    let mut context = Context::new(0, owner.native_handle_home(), &log);
    context.selected_length = Some(2);
    let mut set = unsafe { PendingSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add(19, owner.native_handle_home(), context.adapter())
            .unwrap();
    }
    let wait = budget(1);
    let (mut first, mut second) = ([0xa5; 4], [0x5a; 4]);
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut first), Buffer::Read(&mut second)].into_iter())
            .unwrap();
    let failure = unsafe { set.begin_selected(ptr::null(), buffers, &wait, selection(&[0, 0])) }
        .err()
        .unwrap();
    assert!(
        matches!(failure, SetupFailure::Preparation(AdapterTerminal::PlatformFailure(failure))
        if failure == Failure::invalid_input())
    );
    assert_eq!(first, [0xa5; 4]);
    assert_eq!(second, [0x5a; 4]);
    let calls = log.borrow();
    assert_eq!(
        calls
            .calls()
            .iter()
            .filter(|call| matches!(call, Call::Prepare { .. }))
            .count(),
        2
    );
    assert_eq!(
        calls
            .calls()
            .iter()
            .filter(|call| matches!(
                call,
                Call::Complete {
                    tag: 4,
                    submitted: 0,
                    ..
                }
            ))
            .count(),
        2
    );
    assert!(calls
        .calls()
        .iter()
        .all(|call| !matches!(call, Call::Complete { submitted: 1, .. })));
}

#[test]
fn reversed_subset_uses_same_native_slots_and_transforms_selected_storage_without_hot_allocations()
{
    let (a, mut a_peer) = source();
    let (unused, mut unused_peer) = source();
    let (writer, mut write_peer) = source();
    let log = Log::new();
    let mut a_context = Context::new(0, a.native_handle_home(), &log);
    a_context.selected_length = Some(3);
    a_context.transform = true;
    let mut unused_context = Context::new(1, unused.native_handle_home(), &log);
    let mut writer_context = Context::new(2, writer.native_handle_home(), &log);
    writer_context.selected_length = Some(3);
    writer_context.storage[..3].copy_from_slice(b"one");
    let mut set = unsafe { PendingSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add(10, a.native_handle_home(), a_context.adapter())
            .unwrap();
        set.add(20, unused.native_handle_home(), unused_context.adapter())
            .unwrap();
        set.add(30, writer.native_handle_home(), writer_context.adapter())
            .unwrap();
    }
    let a_snapshot = set.native.as_mut().unwrap().pending_snapshot(0).unwrap();
    let write_snapshot = set.native.as_mut().unwrap().pending_snapshot(2).unwrap();
    let selected_address = a_context.storage.as_ptr() as usize;
    let begin_frame = frame();
    let next_frame = frame();
    let submit_frame = frame();
    let poll = budget(0);
    let mut bytes = [0x55; 8];
    let original = bytes.as_mut_ptr() as usize;
    let write_bytes = b"original";
    let write_caller = write_bytes.as_ptr() as usize;
    unused_peer.write_all(b"untouched").unwrap();
    let buffers =
        BufferList::from_iter([Buffer::Write(write_bytes), Buffer::Read(&mut bytes)].into_iter())
            .unwrap();
    let mut group = unsafe {
        set.begin_selected(&begin_frame, buffers, &poll, selection(&[2, 0]))
            .unwrap()
    };
    assert_eq!(group.request_info(1), Some((true, 8)));
    assert!(group.completed(1).is_none());
    assert!(group.read_storage_mut(1).is_none());
    a_peer.write_all(b"abc").unwrap();
    let ((written, read), allocations) = crate::probe_runtime_allocations(|| unsafe {
        (
            group.next(&next_frame).unwrap(),
            group.next(&next_frame).unwrap(),
        )
    });
    assert_eq!(allocations, 0);
    assert_eq!((written.key, written.index), (30, 0));
    assert!(matches!(
        written.outcome.transfer,
        Some(AdapterTerminal::Accepted(3))
    ));
    assert_eq!((read.key, read.index), (10, 1));
    assert!(matches!(
        read.outcome.transfer,
        Some(AdapterTerminal::Data(3))
    ));
    assert_eq!(
        group.completed(1).unwrap(),
        &[b'a' ^ 0xff, b'b' ^ 0xff, b'c' ^ 0xff]
    );
    assert_eq!(
        group.read_storage_mut(1).unwrap().as_ptr() as usize,
        original
    );
    assert_eq!(group.stop(), None);
    let mut actual = [0; 3];
    write_peer.read_exact(&mut actual).unwrap();
    assert_eq!(&actual, b"one");
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        group.submit(&submit_frame, 0, 3, 3).unwrap();
        let again = group.next(&next_frame).unwrap();
        assert!(matches!(
            again.outcome.transfer,
            Some(AdapterTerminal::Accepted(3))
        ));
    });
    assert_eq!(allocations, 0);
    write_peer.read_exact(&mut actual).unwrap();
    assert_eq!(&actual, b"one");
    drop(group);
    assert_eq!(
        set.native.as_mut().unwrap().pending_snapshot(0).unwrap(),
        a_snapshot
    );
    assert_eq!(
        set.native.as_mut().unwrap().pending_snapshot(2).unwrap(),
        write_snapshot
    );
    let calls = log.borrow();
    assert!(calls.calls().iter().all(|call| !matches!(
        call,
        Call::Prepare { index: 1, .. } | Call::Complete { index: 1, .. }
    )));
    assert!(calls
        .calls()
        .iter()
        .any(|call| matches!(call, Call::Complete {
        index: 0, frame, caller, length: 8, tag: 0, submitted: 1,
        selected: (_, data, 3), ..
    } if *frame == ptr::from_ref(&next_frame) as usize && *caller == original
        && *data == selected_address)));
    assert!(calls
        .calls()
        .iter()
        .any(|call| matches!(call, Call::Prepare {
        index: 2, frame, caller, length: 3,
    } if *frame == ptr::from_ref(&submit_frame) as usize && *caller == write_caller + 3)));
    drop(calls);
    let mut untouched = [0; 9];
    assert_eq!(
        unsafe { libc::read(unused.get(), untouched.as_mut_ptr().cast(), 9) },
        9
    );
    assert_eq!(&untouched, b"untouched");
    assert_eq!(&bytes[3..], &[0x55; 5]);
}

#[test]
fn immediate_prepare_still_completes_outer_adapter_and_empty_eof_and_zero_data_are_distinct() {
    let (owner, mut peer) = source();
    let log = Log::new();
    let mut context = Context::new(0, owner.native_handle_home(), &log);
    context.immediate = Some(DrAdapterOutcomeV1 {
        count: 0,
        tag: 0,
        has_count: 0,
    });
    context.outer = Some(DrAdapterOutcomeV1 {
        count: 0,
        tag: 0,
        has_count: 0,
    });
    let mut set = unsafe { PendingSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add(1, owner.native_handle_home(), context.adapter())
            .unwrap();
    }
    let poll = budget(0);
    let current = frame();
    let mut bytes = [0x33; 4];
    peer.write_all(b"stay").unwrap();
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(&current, buffers, &poll, selection(&[0]))
            .unwrap()
    };
    assert!(matches!(
        unsafe { group.next(&current).unwrap() }.outcome.transfer,
        Some(AdapterTerminal::Data(0))
    ));
    unsafe {
        group.submit(&current, 0, 0, 0).unwrap();
    }
    assert!(matches!(
        unsafe { group.next(&current).unwrap() }.outcome.transfer,
        Some(AdapterTerminal::Data(0))
    ));
    drop(group);
    assert_eq!(
        log.borrow()
            .calls()
            .iter()
            .filter(|call| matches!(
                call,
                Call::Complete {
                    tag: 0,
                    submitted: 0,
                    immediate: Some((0, 0)),
                    ..
                }
            ))
            .count(),
        2
    );
    context.immediate = Some(DrAdapterOutcomeV1 {
        count: 0,
        tag: 2,
        has_count: 0,
    });
    context.outer = None;
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(&current, buffers, &poll, selection(&[0]))
            .unwrap()
    };
    assert!(matches!(
        unsafe { group.next(&current).unwrap() }.outcome.transfer,
        Some(AdapterTerminal::EndOfStream)
    ));
    drop(group);
    assert_eq!(bytes, [0x33; 4]);
    let mut payload = [0; 4];
    assert_eq!(
        unsafe { libc::read(owner.get(), payload.as_mut_ptr().cast(), 4) },
        4
    );
    assert_eq!(&payload, b"stay");
    assert_eq!(
        set.native.as_mut().unwrap().pending_snapshot(0).unwrap().2,
        0
    );
}

#[test]
fn foreign_handle_home_rejects_before_io_and_preserves_original_checked_error_identity() {
    let descriptor = descriptor();
    let drops = Rc::new(Cell::new(0));
    let (owner, mut peer) = source();
    let copied_handle = Box::new(owner.get());
    let log = Log::new();
    let mut context = Context::new(0, ptr::from_ref(copied_handle.as_ref()).cast(), &log);
    let (error, original) = exact_error(&descriptor, &drops);
    context.failure_error = Some(error);
    let mut set = unsafe { PendingSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add(1, owner.native_handle_home(), context.adapter())
            .unwrap();
    }
    peer.write_all(b"live").unwrap();
    let poll = budget(0);
    let current = frame();
    let mut bytes = [0x44; 4];
    let caller = bytes.as_mut_ptr() as usize;
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let error = match unsafe { set.begin_selected(&current, buffers, &poll, selection(&[0])) } {
        Err(SetupFailure::Preparation(AdapterTerminal::CheckedError {
            error,
            progress: None,
        })) => error,
        _ => panic!("expected the exact owned complete error as a typed setup failure"),
    };
    let carrier = error.into_carrier();
    assert_eq!(carrier.object, original);
    assert_eq!(carrier.vtable, ptr::from_ref(&descriptor));
    assert_eq!(drops.get(), 0);
    assert_eq!(log.borrow().calls().len(), 2);
    assert!(matches!(log.borrow().calls()[1], Call::Complete {
        tag: 4, submitted: 0, progress: None, selected: (endpoint, data, 4), ..
    } if endpoint == context.endpoint as usize && data == caller));
    assert_eq!(
        set.native.as_mut().unwrap().pending_snapshot(0).unwrap().2,
        0
    );
    assert_eq!(bytes, [0x44; 4]);
    let mut actual = [0; 4];
    assert_eq!(
        unsafe { libc::read(owner.get(), actual.as_mut_ptr().cast(), 4) },
        4
    );
    assert_eq!(&actual, b"live");
    drop(set);
    assert_eq!(drops.get(), 0);
    unsafe {
        ((*carrier.vtable).drop)(ptr::null(), carrier.object);
    }
    assert_eq!(drops.get(), 1);
}

#[test]
fn later_preparation_error_submits_nothing_and_cleans_earlier_selected_storage_once() {
    let descriptor = descriptor();
    let drops = Rc::new(Cell::new(0));
    let (first, mut first_peer) = source();
    let (second, mut second_peer) = source();
    let log = Log::new();
    let mut first_context = Context::new(0, first.native_handle_home(), &log);
    first_context.selected_length = Some(3);
    let selected = first_context.storage.as_ptr() as usize;
    let mut second_context = Context::new(1, second.native_handle_home(), &log);
    let (error, original) = exact_error(&descriptor, &drops);
    second_context.prepare_error = Some(error);
    let mut set = unsafe { PendingSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add(1, first.native_handle_home(), first_context.adapter())
            .unwrap();
        set.add(2, second.native_handle_home(), second_context.adapter())
            .unwrap();
    }
    first_peer.write_all(b"abc").unwrap();
    second_peer.write_all(b"def").unwrap();
    let current = frame();
    let poll = budget(0);
    let (mut a, mut b) = ([0x11; 4], [0x22; 4]);
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut a), Buffer::Read(&mut b)].into_iter()).unwrap();
    let error = match unsafe { set.begin_selected(&current, buffers, &poll, selection(&[0, 1])) } {
        Err(SetupFailure::Preparation(AdapterTerminal::CheckedError {
            error,
            progress: None,
        })) => error,
        _ => panic!(
            "preparation must return the original checked carrier before any native transfer"
        ),
    };
    let carrier = error.into_carrier();
    assert_eq!(carrier.object, original);
    assert_eq!(carrier.vtable, ptr::from_ref(&descriptor));
    let calls = log.borrow();
    assert_eq!(
        calls
            .calls()
            .iter()
            .filter(|call| matches!(call,
        Call::Complete { index: 0, tag: 4, submitted: 0, progress: None,
            selected: (_, data, 3), .. } if *data == selected))
            .count(),
        1
    );
    assert!(!calls.calls().iter().any(|call| matches!(
        call,
        Call::Complete { index: 1, .. } | Call::Complete { submitted: 1, .. }
    )));
    drop(calls);
    assert_eq!((a, b), ([0x11; 4], [0x22; 4]));
    assert_eq!(
        set.native.as_mut().unwrap().pending_snapshot(0).unwrap().2,
        0
    );
    let mut actual = [0; 3];
    for (owner, expected) in [(&first, b"abc"), (&second, b"def")] {
        assert_eq!(
            unsafe { libc::read(owner.get(), actual.as_mut_ptr().cast(), 3) },
            3
        );
        assert_eq!(&actual, expected);
    }
    unsafe {
        ((*carrier.vtable).drop)(ptr::null(), carrier.object);
    }
    assert_eq!(drops.get(), 1);
}

#[test]
fn cancellation_before_submission_cleans_prepared_loan_without_io_or_invented_progress() {
    let cancellation = crate::cancellation::dr_v1_cancellation_new();
    assert!(!cancellation.is_null());
    let (owner, mut peer) = source();
    let log = Log::new();
    let mut context = Context::new(0, owner.native_handle_home(), &log);
    context.selected_length = Some(3);
    context.cancel_on_prepare = cancellation;
    let selected = context.storage.as_ptr() as usize;
    let mut set = unsafe { PendingSet::new(cancellation).unwrap() };
    unsafe {
        set.add(1, owner.native_handle_home(), context.adapter())
            .unwrap();
    }
    let current = frame();
    let indefinite = budget(1);
    let mut bytes = [0x88; 4];
    peer.write_all(b"stay").unwrap();
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(&current, buffers, &indefinite, selection(&[0]))
            .unwrap()
    };
    let stopped = unsafe { group.next(&current).unwrap() };
    assert!(stopped.outcome.transfer.is_none());
    assert_eq!(stopped.outcome.stop, Some(Stop::Cancellation));
    drop(group);
    assert!(matches!(log.borrow().calls()[1], Call::Complete {
        tag: 4, submitted: 0, count: 0, progress: None, selected: (_, data, 3), ..
    } if data == selected));
    assert_eq!(log.borrow().calls().len(), 2);
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
        log.borrow().calls().len(),
        2,
        "persistent cancellation was reset"
    );
    assert_eq!(bytes, [0x88; 4]);
    let mut actual = [0; 4];
    assert_eq!(
        unsafe { libc::read(owner.get(), actual.as_mut_ptr().cast(), 4) },
        4
    );
    assert_eq!(&actual, b"stay");
    let ((), closes) = crate::native_handle::with_close_probe(|| drop(set));
    assert_eq!(closes, 1, "only the existing watcher is owned by the set");
    unsafe {
        crate::cancellation::dr_v1_cancellation_release(cancellation);
    }
    assert!(unsafe { libc::fcntl(owner.get(), libc::F_GETFL) } >= 0);
}

#[test]
fn native_cancellation_stops_read_retries_but_preserves_won_write_and_drop_cleanup() {
    let descriptor = descriptor();
    let drops = Rc::new(Cell::new(0));
    let cancellation = crate::cancellation::dr_v1_cancellation_new();
    assert!(!cancellation.is_null());
    let (reader, mut read_peer) = source();
    let (writer, mut write_peer) = source();
    let log = Log::new();
    let mut read_context = Context::new(0, reader.native_handle_home(), &log);
    let (error, original) = exact_error(&descriptor, &drops);
    read_context.failure_error = Some(error);
    let mut write_context = Context::new(1, writer.native_handle_home(), &log);
    let mut set = unsafe { PendingSet::new(cancellation).unwrap() };
    unsafe {
        set.add(1, reader.native_handle_home(), read_context.adapter())
            .unwrap();
        set.add(2, writer.native_handle_home(), write_context.adapter())
            .unwrap();
    }
    let before = set.native.as_mut().unwrap().pending_snapshot(0).unwrap();
    let current = frame();
    let indefinite = budget(1);
    let mut bytes = [0x66; 4];
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut bytes), Buffer::Write(b"won")].into_iter())
            .unwrap();
    let mut group = unsafe {
        set.begin_selected(&current, buffers, &indefinite, selection(&[0, 1]))
            .unwrap()
    };
    // The read became a readiness retry at begin, with no retired native
    // outcome; the write already completed. Cancellation must not manufacture
    // a transfer Error or zero progress. The complete adapter may still throw
    // its original checked cleanup Error, which must move through unchanged.
    // Queue read data before cancellation to detect a forbidden retry syscall.
    assert!(group.read_storage_mut(0).is_none());
    read_peer.write_all(b"stay").unwrap();
    unsafe {
        request_cancellation(cancellation);
    }
    let ((read, write), allocations) = crate::probe_runtime_allocations(|| unsafe {
        (group.next(&current).unwrap(), group.next(&current).unwrap())
    });
    assert_eq!(allocations, 0);
    assert_eq!(read.outcome.stop, Some(Stop::Cancellation));
    assert_eq!(read.outcome.cancellation_failure, None);
    assert_eq!(group.completed(0), None);
    let error = match read.outcome.transfer {
        Some(AdapterTerminal::CheckedError {
            error,
            progress: None,
        }) => error,
        _ => panic!("native cancellation must finalize through the exact complete adapter"),
    };
    assert!(matches!(
        write.outcome.transfer,
        Some(AdapterTerminal::Accepted(3))
    ));
    assert_eq!(write.outcome.stop, Some(Stop::Cancellation));
    assert_eq!(write.outcome.cancellation_failure, None);
    assert_eq!(group.completed(1), Some(b"won".as_slice()));
    assert_eq!(group.stop(), Some(Stop::Cancellation));
    drop(group);
    assert_eq!(
        set.native.as_mut().unwrap().pending_snapshot(0).unwrap(),
        before
    );
    assert_eq!(bytes, [0x66; 4]);
    assert_eq!(log.borrow().calls().len(), 4);
    assert_eq!(
        log.borrow()
            .calls()
            .iter()
            .filter(|call| matches!(
                call,
                Call::Complete {
                    index: 0,
                    tag: 4,
                    submitted: 0,
                    count: 0,
                    progress: None,
                    immediate: None,
                    ..
                }
            ))
            .count(),
        1
    );
    assert_eq!(
        log.borrow()
            .calls()
            .iter()
            .filter(|call| matches!(
                call,
                Call::Complete {
                    index: 1,
                    tag: 0,
                    submitted: 1,
                    count: 3,
                    progress: None,
                    immediate: None,
                    ..
                }
            ))
            .count(),
        1
    );
    let mut actual = [0; 4];
    assert_eq!(
        unsafe { libc::read(reader.get(), actual.as_mut_ptr().cast(), 4) },
        4
    );
    assert_eq!(&actual, b"stay");
    write_peer.read_exact(&mut actual[..3]).unwrap();
    assert_eq!(&actual[..3], b"won");
    let carrier = error.into_carrier();
    assert_eq!(carrier.object, original);
    assert_eq!(carrier.vtable, ptr::from_ref(&descriptor));
    drop(set);
    assert_eq!(drops.get(), 0);
    unsafe {
        ((*carrier.vtable).drop)(ptr::null(), carrier.object);
        crate::cancellation::dr_v1_cancellation_release(cancellation);
    }
    assert_eq!(drops.get(), 1);

    let log = Log::new();
    let mut context = Context::new(0, reader.native_handle_home(), &log);
    let (error, _) = exact_error(&descriptor, &drops);
    context.failure_error = Some(error);
    let mut set = unsafe { PendingSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add(1, reader.native_handle_home(), context.adapter())
            .unwrap();
    }
    let before = set.native.as_mut().unwrap().pending_snapshot(0).unwrap();
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let group = unsafe {
        set.begin_selected(&current, buffers, &indefinite, selection(&[0]))
            .unwrap()
    };
    let ((), allocations) = crate::probe_runtime_allocations(|| drop(group));
    assert_eq!(allocations, 0);
    assert_eq!(
        drops.get(),
        2,
        "drop releases the one exact completion carrier"
    );
    assert_eq!(
        set.native.as_mut().unwrap().pending_snapshot(0).unwrap(),
        before
    );
    assert_eq!(log.borrow().calls().len(), 2);
    assert!(matches!(
        log.borrow().calls()[1],
        Call::Complete {
            frame: 0,
            tag: 4,
            submitted: 0,
            count: 0,
            progress: None,
            immediate: None,
            ..
        }
    ));
    read_peer.write_all(b"reuse").unwrap();
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let mut group = unsafe {
        set.begin_selected(&current, buffers, &indefinite, selection(&[0]))
            .unwrap()
    };
    assert!(matches!(
        unsafe { group.next(&current).unwrap() }.outcome.transfer,
        Some(AdapterTerminal::Data(4))
    ));
    drop(group);
    assert_eq!(bytes, *b"reus");
    assert_eq!(
        set.native.as_mut().unwrap().pending_snapshot(0).unwrap(),
        before
    );
}

#[test]
fn selected_capabilities_and_rejected_membership_never_prepare_transfer_or_claim_endpoints() {
    let (a, mut a_peer) = source();
    let (b, mut b_peer) = source();
    let log = Log::new();
    let mut a_context = Context::new(0, a.native_handle_home(), &log);
    let mut b_context = Context::new(1, b.native_handle_home(), &log);
    let mut read_only = b_context.adapter();
    read_only.prepare_write = None;
    read_only.complete_write = None;
    let mut set = unsafe { PendingSet::new(ptr::null_mut()).unwrap() };
    unsafe {
        set.add(1, a.native_handle_home(), a_context.adapter())
            .unwrap();
        set.add(2, b.native_handle_home(), read_only).unwrap();
    }
    let a_snapshot = set.native.as_mut().unwrap().pending_snapshot(0).unwrap();
    let poll = budget(0);
    let current = frame();
    let mut bytes = [0x77; 4];
    a_peer.write_all(b"stay").unwrap();
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut bytes), Buffer::Write(b"never")].into_iter())
            .unwrap();
    let indices = selection(&[0, 1]);
    let (result, allocations) = crate::probe_runtime_allocations(|| unsafe {
        set.begin_selected(&current, buffers, &poll, indices)
    });
    assert_eq!(allocations, 0);
    assert!(matches!(result, Err(SetupFailure::Platform(_))));
    drop(result);
    assert!(log.borrow().calls().is_empty());
    let copied_handle = Box::new(a.get());
    for endpoint in [
        a.native_handle_home(),
        ptr::from_ref(copied_handle.as_ref()).cast(),
        ptr::null(),
    ] {
        let ((result, closes), allocations) = crate::probe_runtime_allocations(|| {
            crate::native_handle::with_close_probe(|| unsafe {
                set.add(3, endpoint, a_context.adapter())
            })
        });
        assert!(result.is_err());
        assert_eq!((allocations, closes), (0, 0));
    }
    set.remove(2).unwrap();
    assert_eq!(set.index_of(2), None);
    assert_eq!(set.key(1), None);
    assert_eq!(set.key(0), Some(1));
    assert!(unsafe { set.add(2, b.native_handle_home(), b_context.adapter()) }.is_err());
    assert_eq!(
        unsafe { set.add(3, b.native_handle_home(), b_context.adapter()) }.unwrap(),
        1
    );
    assert_eq!(
        set.native.as_mut().unwrap().pending_snapshot(0).unwrap(),
        a_snapshot
    );
    assert_eq!(set.index_of(3), Some(1));
    let ((), closes) = crate::native_handle::with_close_probe(|| drop(set));
    assert_eq!(closes, 1, "borrowed endpoints acquired no close owner");
    assert_eq!(bytes, [0x77; 4]);
    let mut actual = [0; 4];
    assert_eq!(
        unsafe { libc::read(a.get(), actual.as_mut_ptr().cast(), 4) },
        4
    );
    assert_eq!(&actual, b"stay");
    b_peer.write_all(b"live").unwrap();
    assert_eq!(
        unsafe { libc::read(b.get(), actual.as_mut_ptr().cast(), 4) },
        4
    );
    assert_eq!(&actual, b"live");
}
