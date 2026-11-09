use crate::transfer_group::adapter::test_object::TestObject;
use crate::transfer_group::adapter::DrAdapterContextV1;
use core::{mem, ptr};
use std::cell::Cell;
use std::panic::AssertUnwindSafe;
use std::rc::Rc;

use super::*;
use crate::transfer_group::{
    Awaited, Buffer, BufferList, Control, GroupDriver, Query, RequestSlots, Stop, Submission,
    TransferGroup, Wait, Wake,
};

#[test]
fn callback_context_accepts_zero_inline_bits_with_present_metadata() {
    let marker = 0_u8;
    let mut context = DrAdapterContextV1 {
        object: ptr::null_mut(),
        descriptor: ptr::null(),
    };
    assert!(!context.valid());
    context.descriptor = (&marker as *const u8).cast();
    assert!(context.valid());
}
struct ErrorPayload {
    drops: Rc<Cell<usize>>,
    message: &'static str,
    target: &'static str,
    system_code: i64,
    progress: Option<i64>,
}

unsafe extern "C" fn drop_error(_: *const DrStackFrameV2, object: *mut u8) {
    crate::test_callback::guard(|| {
        let object = Box::from_raw(object.cast::<ErrorPayload>());
        object.drops.set(object.drops.get() + 1);
    })
}

fn descriptor() -> crate::DrInterfaceVtableV1 {
    crate::DrInterfaceVtableV1 {
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

fn owned_error(
    descriptor: &crate::DrInterfaceVtableV1,
    drops: &Rc<Cell<usize>>,
    progress: Option<i64>,
) -> OwnedIoError {
    let object = Box::into_raw(Box::new(ErrorPayload {
        drops: Rc::clone(drops),
        message: "adapter-specific message",
        target: "original/wrapped/path",
        system_code: 91,
        progress,
    }));
    unsafe {
        OwnedIoError::from_carrier(DrInterfaceCarrierV1 {
            object: object.cast(),
            vtable: descriptor,
        })
    }
}

struct Context {
    calls: usize,
    tag: u8,
    count: usize,
    progress: Option<usize>,
    error: Option<OwnedIoError>,
    cancel: Option<Rc<Cell<bool>>>,
    frame_address: usize,
}

impl Context {
    fn new(tag: u8, count: usize) -> TestObject<Self> {
        TestObject::new(Self {
            calls: 0,
            tag,
            count,
            progress: None,
            error: None,
            cancel: None,
            frame_address: 0,
        })
    }
}

impl TestObject<Context> {
    fn adapter(&self) -> DrTransferAdapterV1 {
        DrTransferAdapterV1 {
            context: self.context(),
            read: Some(read_callback),
            write: Some(write_callback),
        }
    }
}

unsafe fn callback(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    let context = TestObject::<Context>::in_callback((*context).object);
    context.calls += 1;
    context.frame_address = frame as usize; // observation, not a retained frame loan
    if let Some(cancel) = &context.cancel {
        cancel.set(true);
    }
    if let Some(owned) = context.error.take() {
        (*outcome).count = context.progress.unwrap_or(0);
        (*outcome).has_count = u8::from(context.progress.is_some());
        error.write(owned.into_carrier());
        1
    } else {
        (*outcome).tag = context.tag;
        (*outcome).count = context.count;
        0
    }
}

unsafe extern "C" fn read_callback(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    _: *mut u8,
    _: usize,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    crate::test_callback::guard(|| callback(frame, context, outcome, error))
}

unsafe extern "C" fn write_callback(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    _: *const u8,
    _: usize,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    crate::test_callback::guard(|| callback(frame, context, outcome, error))
}

#[test]
fn four_word_callback_layout_and_explicit_outcomes_preserve_zero_data_eof_and_timeout() {
    assert_eq!(
        mem::size_of::<DrAdapterOutcomeV1>(),
        2 * mem::size_of::<usize>()
    );
    assert_eq!(
        mem::align_of::<DrAdapterOutcomeV1>(),
        mem::align_of::<usize>()
    );
    assert_eq!(
        mem::size_of::<DrTransferAdapterV1>(),
        4 * mem::size_of::<usize>()
    );
    assert_eq!(
        mem::size_of::<DrAdapterContextV1>(),
        2 * mem::size_of::<usize>()
    );
    assert_eq!(
        mem::align_of::<DrAdapterContextV1>(),
        mem::align_of::<usize>()
    );
    assert_eq!(mem::offset_of!(DrAdapterContextV1, object), 0);
    assert_eq!(
        mem::offset_of!(DrAdapterContextV1, descriptor),
        mem::size_of::<usize>()
    );
    assert_eq!(mem::offset_of!(DrTransferAdapterV1, context), 0);
    assert_eq!(
        mem::offset_of!(DrTransferAdapterV1, read),
        2 * mem::size_of::<usize>()
    );
    assert_eq!(
        mem::offset_of!(DrTransferAdapterV1, write),
        3 * mem::size_of::<usize>()
    );
    let outcome = DrAdapterOutcomeV1::default();
    assert_eq!(
        ptr::addr_of!(outcome.tag) as usize - ptr::addr_of!(outcome) as usize,
        mem::size_of::<usize>()
    );
    assert_eq!(
        ptr::addr_of!(outcome.has_count) as usize - ptr::addr_of!(outcome) as usize,
        mem::size_of::<usize>() + 1
    );
    let context = Context::new(0, 0);
    let adapter = context.adapter();
    let mut bytes = [0_u8; 8];
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        assert!(matches!(
            adapter.read(ptr::null(), bytes.as_mut_ptr(), 8),
            AdapterAttempt::Terminal(AdapterTerminal::Data(0))
        ));
        (*context.as_ptr()).tag = 2;
        assert!(matches!(
            adapter.read(ptr::null(), bytes.as_mut_ptr(), 8),
            AdapterAttempt::Terminal(AdapterTerminal::EndOfStream)
        ));
        (*context.as_ptr()).tag = 3;
        assert!(matches!(
            adapter.read(ptr::null(), bytes.as_mut_ptr(), 8),
            AdapterAttempt::Terminal(AdapterTerminal::TimedOut)
        ));
        (*context.as_ptr()).tag = 1;
        assert!(matches!(
            adapter.read(ptr::null(), bytes.as_mut_ptr(), 8),
            AdapterAttempt::Pending
        ));
        assert!(matches!(
            adapter.write(ptr::null(), bytes.as_ptr(), 8),
            AdapterAttempt::Pending
        ));
        (*context.as_ptr()).tag = 0;
        (*context.as_ptr()).count = 3;
        assert!(matches!(
            adapter.write(ptr::null(), bytes.as_ptr(), 8),
            AdapterAttempt::Terminal(AdapterTerminal::Accepted(3))
        ));
    });
    assert_eq!(allocations, 0);
    assert_eq!(unsafe { (*context.as_ptr()).calls }, 6);
}

#[test]
fn readiness_control_facts_never_rebuild_or_drop_the_original_transfer_error() {
    use doria_diagnostic_catalogue::IoMessageOperation;
    let descriptor = descriptor();
    for operation in [
        IoMessageOperation::Register,
        IoMessageOperation::Unregister,
        IoMessageOperation::Wait,
    ] {
        for original_progress in [None, Some(0), Some(2), Some(-3)] {
            let drops = Rc::new(Cell::new(0));
            let owned = owned_error(&descriptor, &drops, original_progress);
            let original = owned.carrier().object;
            let progress = original_progress.and_then(|count| usize::try_from(count).ok());
            let failure = Failure::from_system_code(13);
            let mut control = crate::transfer_group::DrCompletionStatusV1::default();
            assert_eq!(
                control.project(
                    Awaited {
                        transfer: None,
                        stop: Some(Stop::Failure(operation, failure)),
                        cancellation_failure: Some((operation, failure)),
                    },
                    true
                ),
                4
            );
            assert_eq!(
                (control.wait_operation, control.cancellation_operation),
                (operation as u8, operation as u8)
            );
            assert_eq!(control.wait_failure.failure(), Some(failure));
            assert_eq!(control.cancellation_failure.failure(), Some(failure));
            let terminal = AdapterTerminal::CheckedError {
                error: owned,
                progress,
            };
            let mut outcome = DrAdapterOutcomeV1::default();
            let mut error = crate::empty_error_carrier();
            unsafe {
                assert_eq!(
                    terminal.project(&mut outcome, &mut error, &mut control.transfer),
                    1
                );
                assert_eq!(error.object, original);
                assert_eq!(error.vtable, &descriptor as *const _);
                let payload = &*error.object.cast::<ErrorPayload>();
                assert_eq!(payload.progress, original_progress);
                assert_eq!(payload.message, "adapter-specific message");
                assert_eq!(payload.target, "original/wrapped/path");
                assert_eq!(payload.system_code, 91);
                assert_eq!(drops.get(), 0);
                assert_eq!(
                    (outcome.has_count, outcome.count),
                    (u8::from(progress.is_some()), progress.unwrap_or(0))
                );
                assert_eq!((control.stop, control.wait_operation), (3, operation as u8));
                assert_eq!(control.wait_failure.has_transferred_byte_count, 0);
                drop(OwnedIoError::from_carrier(error));
            }
            assert_eq!(drops.get(), 1);
        }
    }
}

#[test]
fn checked_callback_error_moves_original_carrier_and_optional_progress_exactly_once() {
    let descriptor = descriptor();
    for progress in [None, Some(0), Some(2)] {
        let drops = Rc::new(Cell::new(0));
        let context = Context::new(0, 0);
        unsafe {
            (*context.as_ptr()).error = Some(owned_error(
                &descriptor,
                &drops,
                progress.map(|count| count as i64),
            ))
        };
        let original = unsafe { (*context.as_ptr()).error.as_ref().unwrap().carrier().object };
        unsafe { (*context.as_ptr()).progress = progress };
        let adapter = context.adapter();
        let mut bytes = [0_u8; 8];
        let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
            let AdapterAttempt::Terminal(terminal) =
                adapter.read(ptr::null(), bytes.as_mut_ptr(), bytes.len())
            else {
                panic!("expected original checked error")
            };
            assert_eq!(terminal.progress(), progress);
            let mut output = DrAdapterOutcomeV1::default();
            let mut platform = DrIoStatusV1::default();
            let mut error = crate::empty_error_carrier();
            assert_eq!(terminal.project(&mut output, &mut error, &mut platform), 1);
            assert_eq!(error.object, original);
            assert_eq!(error.vtable, &descriptor as *const _);
            assert_eq!(output.has_count, u8::from(progress.is_some()));
            assert_eq!(output.count, progress.unwrap_or(0));
            let payload = &*error.object.cast::<ErrorPayload>();
            assert_eq!(
                (
                    payload.message,
                    payload.target,
                    payload.system_code,
                    payload.progress
                ),
                (
                    "adapter-specific message",
                    "original/wrapped/path",
                    91,
                    progress.map(|count| count as i64)
                )
            );
            assert_eq!(drops.get(), 0);
            drop(OwnedIoError::from_carrier(error));
        });
        assert_eq!(allocations, 0);
        assert_eq!(drops.get(), 1);
        assert!(unsafe { (*context.as_ptr()).error.is_none() });
    }
}

#[test]
fn unusable_checked_progress_preserves_signed_error_and_withholds_prefix() {
    let descriptor = descriptor();
    for reading in [true, false] {
        for (original_progress, projected) in [
            (9, Some(9)),
            (-3, None),
            (-1, Some(usize::MAX)),
            (i64::MAX, usize::try_from(i64::MAX).ok()),
        ] {
            let drops = Rc::new(Cell::new(0));
            let context = Context::new(0, 0);
            unsafe {
                (*context.as_ptr()).error =
                    Some(owned_error(&descriptor, &drops, Some(original_progress)))
            };
            unsafe { (*context.as_ptr()).progress = projected };
            let original = unsafe { (*context.as_ptr()).error.as_ref().unwrap().carrier().object };
            let adapter = context.adapter();
            let mut bytes = [0_u8; 8];
            let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
                let attempt = if reading {
                    adapter.read(ptr::null(), bytes.as_mut_ptr(), bytes.len())
                } else {
                    adapter.write(ptr::null(), bytes.as_ptr(), bytes.len())
                };
                let AdapterAttempt::Terminal(terminal) = attempt else {
                    panic!("checked error must stay terminal")
                };
                assert_eq!(terminal.progress(), None);
                let mut output = DrAdapterOutcomeV1::default();
                let mut platform = DrIoStatusV1::default();
                let mut error = crate::empty_error_carrier();
                assert_eq!(terminal.project(&mut output, &mut error, &mut platform), 1);
                assert_eq!((output.has_count, output.count), (0, 0));
                assert_eq!(error.object, original);
                assert_eq!(error.vtable, &descriptor as *const _);
                let payload = &*error.object.cast::<ErrorPayload>();
                assert_eq!(payload.progress, Some(original_progress));
                assert_eq!(payload.message, "adapter-specific message");
                assert_eq!(payload.target, "original/wrapped/path");
                assert_eq!(payload.system_code, 91);
                assert_eq!(drops.get(), 0);
                drop(OwnedIoError::from_carrier(error));
            });
            assert_eq!(allocations, 0);
            assert_eq!(drops.get(), 1);
            assert!(unsafe { (*context.as_ptr()).error.is_none() });
        }
    }
}

#[test]
fn checked_terminal_bounding_preserves_original_error_ownership() {
    let descriptor = descriptor();
    for (original_progress, projected) in [(9, 9), (-1, usize::MAX)] {
        let drops = Rc::new(Cell::new(0));
        let error = owned_error(&descriptor, &drops, Some(original_progress));
        let original = error.carrier().object;
        let terminal = AdapterTerminal::CheckedError {
            error,
            progress: Some(projected),
        };
        let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
            let bounded = terminal.bounded(8);
            assert_eq!(bounded.progress(), None);
            let mut output = DrAdapterOutcomeV1::default();
            let mut platform = DrIoStatusV1::default();
            let mut error = crate::empty_error_carrier();
            assert_eq!(bounded.project(&mut output, &mut error, &mut platform), 1);
            assert_eq!((output.has_count, output.count), (0, 0));
            assert_eq!(error.object, original);
            assert_eq!(error.vtable, &descriptor as *const _);
            assert_eq!(
                (*error.object.cast::<ErrorPayload>()).progress,
                Some(original_progress)
            );
            assert_eq!(drops.get(), 0);
            drop(OwnedIoError::from_carrier(error));
        });
        assert_eq!(allocations, 0);
        assert_eq!(drops.get(), 1);
    }
}

#[test]
fn adapter_projection_and_untaken_error_cleanup_never_reconstruct_a_transfer() {
    let descriptor = descriptor();
    let drops = Rc::new(Cell::new(0));
    let terminal = AdapterTerminal::CheckedError {
        error: owned_error(&descriptor, &drops, Some(0)),
        progress: Some(0),
    };
    drop(Awaited::complete(terminal));
    assert_eq!(drops.get(), 1);
    for (terminal, tag, count, present) in [
        (AdapterTerminal::Data(0), 0, 0, 1),
        (AdapterTerminal::Accepted(2), 0, 2, 1),
        (AdapterTerminal::EndOfStream, 2, 0, 0),
        (AdapterTerminal::TimedOut, 3, 0, 0),
    ] {
        let mut output = DrAdapterOutcomeV1::default();
        let mut platform = DrIoStatusV1::default();
        let mut error = crate::empty_error_carrier();
        assert_eq!(
            unsafe { terminal.project(&mut output, &mut error, &mut platform) },
            0
        );
        assert_eq!(
            (output.tag, output.count, output.has_count),
            (tag, count, present)
        );
        assert!(error.object.is_null() && error.vtable.is_null());
    }
    assert!(AdapterTerminal::NoTransfer.into_transfer().is_none());
}

struct Request {
    data: *mut u8,
    length: usize,
    reading: bool,
    terminal: Option<AdapterTerminal>,
}

impl Request {
    fn new() -> Self {
        Self {
            data: ptr::null_mut(),
            length: 0,
            reading: false,
            terminal: None,
        }
    }
}

struct Driver {
    adapters: [DrTransferAdapterV1; 2],
    frame: *const DrStackFrameV2,
    observed_frame: Rc<Cell<usize>>,
    waits: Rc<Cell<usize>>,
}

impl Driver {
    unsafe fn attempt(&mut self, index: usize, request: &mut Request) -> bool {
        let result = if request.reading {
            self.adapters[index].read(self.frame, request.data, request.length)
        } else {
            self.adapters[index].write(self.frame, request.data.cast_const(), request.length)
        };
        match result {
            AdapterAttempt::Pending => false,
            AdapterAttempt::Terminal(terminal) => {
                request.terminal = Some(terminal);
                request.data = ptr::null_mut();
                true
            }
        }
    }
}

// The fixture only retains a group's byte loan for pending synchronous attempts;
// stop never invokes a callback, and no kernel/worker retains those bytes.
unsafe impl GroupDriver<AdapterTerminal> for Driver {
    type Request = Request;
    type Timeout = u32;
    const MAXIMUM_TRANSFER_BYTES: usize = isize::MAX as usize;
    const QUERY_CAN_TRANSFER: bool = true;
    const EMPTY_MUST_DISPATCH: bool = true;
    unsafe fn resource(
        &self,
        index: usize,
        _: *mut Request,
    ) -> Result<crate::transfer_group::TransferResource, Failure> {
        Ok(crate::transfer_group::TransferResource {
            identity: index,
            shared_cursor: false,
        })
    }

    fn set_call_frame(&mut self, frame: *const DrStackFrameV2) {
        self.frame = frame;
        self.observed_frame.set(frame as usize);
    }

    fn call_frame(&self) -> *const DrStackFrameV2 {
        self.frame
    }
    unsafe fn reset_request(&mut self, _: usize, request: *mut Request) {
        request.write(Request::new());
    }
    unsafe fn submit(
        &mut self,
        index: usize,
        request: *mut Request,
        buffer: &mut Buffer<'_>,
    ) -> Submission {
        let request = &mut *request;
        match buffer {
            Buffer::Read(bytes) => {
                request.data = bytes.as_mut_ptr();
                request.length = bytes.len();
                request.reading = true;
            }
            Buffer::Write(bytes) => {
                request.data = bytes.as_ptr().cast_mut();
                request.length = bytes.len();
            }
        }
        if self.attempt(index, request) {
            Submission::Complete
        } else {
            Submission::Pending
        }
    }
    unsafe fn query(&mut self, index: usize, request: *mut Request) -> Query<AdapterTerminal> {
        let request = &mut *request;
        if request.terminal.is_none() {
            self.attempt(index, request);
        }
        request
            .terminal
            .take()
            .map_or(Query::Pending, Query::Terminal)
    }
    fn wait_any(&mut self, _: u32, _: Option<usize>) -> Result<Wake, Failure> {
        self.waits.set(self.waits.get() + 1);
        Ok(Wake::Completion)
    }
    unsafe fn cancel(
        &mut self,
        _: usize,
        request: *mut Request,
    ) -> Result<(), (doria_diagnostic_catalogue::IoMessageOperation, Failure)> {
        if (*request).terminal.is_none() {
            (*request).terminal = Some(AdapterTerminal::NoTransfer);
        }
        (*request).data = ptr::null_mut();
        Ok(())
    }
    unsafe fn drain(&mut self, _: usize, request: *mut Request) -> Option<AdapterTerminal> {
        Some((*request).terminal.take().expect("cancel before drain"))
    }
}

struct Budget {
    cancelled: Rc<Cell<bool>>,
    immediate: bool,
}
impl Control for Budget {
    fn next_wait(&mut self) -> Result<Result<Wait<u32>, Stop>, Failure> {
        if self.cancelled.get() {
            Ok(Err(Stop::Cancellation))
        } else {
            Ok(Ok(Wait {
                timeout: if self.immediate { 0 } else { 100 },
                immediate: self.immediate,
            }))
        }
    }
    fn cancellation_signal(&self) -> Option<usize> {
        None
    }
}

fn driver(
    a: &TestObject<Context>,
    b: &TestObject<Context>,
) -> (Driver, Rc<Cell<usize>>, Rc<Cell<usize>>) {
    let frame = Rc::new(Cell::new(0));
    let waits = Rc::new(Cell::new(0));
    (
        Driver {
            adapters: [a.adapter(), b.adapter()],
            frame: ptr::null(),
            observed_frame: Rc::clone(&frame),
            waits: Rc::clone(&waits),
        },
        frame,
        waits,
    )
}

#[test]
fn shared_group_dispatches_empty_adapters_and_clears_frames_even_on_host_unwind() {
    let a = Context::new(0, 0);
    let b = Context::new(0, 0);
    let (driver, observed_frame, _) = driver(&a, &b);
    let mut slots = RequestSlots::new(&[10], |_| Request::new()).unwrap();
    let buffers = BufferList::from_iter([Buffer::Read(&mut [])].into_iter()).unwrap();
    let mut group = TransferGroup::begin(
        &mut slots,
        buffers,
        driver,
        Budget {
            cancelled: Rc::new(Cell::new(false)),
            immediate: false,
        },
    )
    .unwrap();
    assert_eq!(unsafe { (*a.as_ptr()).calls }, 1);
    assert_eq!(observed_frame.get(), 0);
    let frame: DrStackFrameV2 = unsafe { mem::zeroed() };
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        let first = group.with_call_frame(&frame, |group| group.next()).unwrap();
        assert!(matches!(
            first.outcome.transfer,
            Some(AdapterTerminal::Data(0))
        ));
        group
            .with_call_frame(&frame, |group| group.submit(0, 0, 0))
            .unwrap();
        let second = group.with_call_frame(&frame, |group| group.next()).unwrap();
        assert!(matches!(
            second.outcome.transfer,
            Some(AdapterTerminal::Data(0))
        ));
    });
    assert_eq!(allocations, 0);
    assert_eq!(unsafe { (*a.as_ptr()).calls }, 2);
    assert_eq!(
        unsafe { (*a.as_ptr()).frame_address },
        &frame as *const _ as usize
    );
    assert_eq!(observed_frame.get(), 0);
    assert!(
        crate::test_report::expect_panic(AssertUnwindSafe(|| unsafe {
            group.with_call_frame(&frame, |_| panic!("test-only host unwind"));
        }))
        .is_err()
    );
    assert_eq!(observed_frame.get(), 0);
}

#[test]
fn earlier_adapter_cancellation_prevents_later_initial_dispatch() {
    let cancelled = Rc::new(Cell::new(false));
    let a = Context::new(0, 1);
    unsafe { (*a.as_ptr()).cancel = Some(Rc::clone(&cancelled)) };
    let b = Context::new(0, 1);
    let (driver, _, _) = driver(&a, &b);
    let mut slots = RequestSlots::new(&[10, 20], |_| Request::new()).unwrap();
    let buffers =
        BufferList::from_iter([Buffer::Write(b"a"), Buffer::Write(b"b")].into_iter()).unwrap();
    let mut group = TransferGroup::begin(
        &mut slots,
        buffers,
        driver,
        Budget {
            cancelled,
            immediate: false,
        },
    )
    .unwrap();
    assert_eq!(
        unsafe { ((*a.as_ptr()).calls, (*b.as_ptr()).calls) },
        (1, 0)
    );
    let first = group.next().unwrap();
    assert!(matches!(
        first.outcome.transfer,
        Some(AdapterTerminal::Accepted(1))
    ));
    let second = group.next().unwrap();
    assert!(second.outcome.transfer.is_none());
    assert_eq!(second.outcome.stop, Some(Stop::Cancellation));
}

#[test]
fn adapter_query_rechecks_stop_between_selected_callbacks() {
    let cancelled = Rc::new(Cell::new(false));
    let a = Context::new(1, 0);
    let b = Context::new(1, 0);
    let (driver, _, _) = driver(&a, &b);
    let mut slots = RequestSlots::new(&[10, 20], |_| Request::new()).unwrap();
    let buffers =
        BufferList::from_iter([Buffer::Write(b"a"), Buffer::Write(b"b")].into_iter()).unwrap();
    let mut group = TransferGroup::begin(
        &mut slots,
        buffers,
        driver,
        Budget {
            cancelled: Rc::clone(&cancelled),
            immediate: false,
        },
    )
    .unwrap();
    assert_eq!(
        unsafe { ((*a.as_ptr()).calls, (*b.as_ptr()).calls) },
        (1, 1)
    );
    unsafe { (*a.as_ptr()).tag = 0 };
    unsafe { (*a.as_ptr()).count = 1 };
    unsafe { (*a.as_ptr()).cancel = Some(cancelled) };
    unsafe { (*b.as_ptr()).tag = 0 };
    unsafe { (*b.as_ptr()).count = 1 };
    let first = group.next().unwrap();
    assert!(matches!(
        first.outcome.transfer,
        Some(AdapterTerminal::Accepted(1))
    ));
    assert_eq!(
        unsafe { ((*a.as_ptr()).calls, (*b.as_ptr()).calls) },
        (2, 1)
    );
    let second = group.next().unwrap();
    assert!(second.outcome.transfer.is_none());
    assert_eq!(second.outcome.stop, Some(Stop::Cancellation));
}

#[test]
fn empty_adapter_callback_preserves_its_original_checked_error() {
    let descriptor = descriptor();
    let drops = Rc::new(Cell::new(0));
    let a = Context::new(0, 0);
    unsafe { (*a.as_ptr()).error = Some(owned_error(&descriptor, &drops, Some(0))) };
    unsafe { (*a.as_ptr()).progress = Some(0) };
    let original = unsafe { (*a.as_ptr()).error.as_ref().unwrap().carrier().object };
    let b = Context::new(0, 0);
    let (driver, _, _) = driver(&a, &b);
    let mut slots = RequestSlots::new(&[10], |_| Request::new()).unwrap();
    let buffers = BufferList::from_iter([Buffer::Write(b"")].into_iter()).unwrap();
    let mut group = TransferGroup::begin(
        &mut slots,
        buffers,
        driver,
        Budget {
            cancelled: Rc::new(Cell::new(false)),
            immediate: false,
        },
    )
    .unwrap();
    assert_eq!(unsafe { (*a.as_ptr()).calls }, 1);
    let completion = group.next().unwrap();
    let Some(AdapterTerminal::CheckedError { error, progress }) = completion.outcome.transfer
    else {
        panic!("empty adapter error was replaced with synthetic success");
    };
    assert_eq!(progress, Some(0));
    assert_eq!(error.carrier().object, original);
    assert_eq!(drops.get(), 0);
    drop(error);
    assert_eq!(drops.get(), 1);
}

#[test]
fn immediate_adapter_poll_is_bounded_despite_permanently_ready_would_block() {
    let a = Context::new(1, 0);
    let b = Context::new(0, 0);
    let (driver, _, waits) = driver(&a, &b);
    let mut slots = RequestSlots::new(&[10], |_| Request::new()).unwrap();
    let buffers = BufferList::from_iter([Buffer::Write(b"a")].into_iter()).unwrap();
    let mut group = TransferGroup::begin(
        &mut slots,
        buffers,
        driver,
        Budget {
            cancelled: Rc::new(Cell::new(false)),
            immediate: true,
        },
    )
    .unwrap();
    let (completion, allocations) = crate::probe_runtime_allocations(|| group.next().unwrap());
    assert!(completion.outcome.transfer.is_none());
    assert_eq!(completion.outcome.stop, Some(Stop::BudgetExhausted));
    assert_eq!(waits.get(), 1);
    assert_eq!(unsafe { (*a.as_ptr()).calls }, 3); // initial attempt, pre-poll query, post-poll query
    assert_eq!(allocations, 0);
    assert!(group.next().is_none());
}

#[test]
fn shared_group_drop_releases_untaken_checked_errors_after_retiring_requests() {
    let descriptor = descriptor();
    let drops = Rc::new(Cell::new(0));
    let a = Context::new(0, 0);
    unsafe { (*a.as_ptr()).error = Some(owned_error(&descriptor, &drops, None)) };
    let b = Context::new(1, 0);
    let (driver, observed_frame, _) = driver(&a, &b);
    let mut slots = RequestSlots::new(&[10, 20], |_| Request::new()).unwrap();
    let buffers =
        BufferList::from_iter([Buffer::Write(b"a"), Buffer::Write(b"b")].into_iter()).unwrap();
    let group = TransferGroup::begin(
        &mut slots,
        buffers,
        driver,
        Budget {
            cancelled: Rc::new(Cell::new(false)),
            immediate: false,
        },
    )
    .unwrap();
    assert_eq!(drops.get(), 0);
    let cleanup = unsafe { mem::zeroed::<DrStackFrameV2>() };
    unsafe { group.drop_in_frame(&cleanup) };
    assert_eq!(observed_frame.get(), 0);
    assert_eq!(drops.get(), 1);
    assert_eq!(
        unsafe { ((*a.as_ptr()).calls, (*b.as_ptr()).calls) },
        (1, 1)
    );
}

#[test]
fn one_plan_preparation_engine_preserves_adapter_loans_and_allocation_count() {
    let key = crate::transfer_group::PlanKey {
        owner: 19,
        value: 7,
    };
    let plan = crate::transfer_group::TransferPlan::new(key, Buffer::Write(b"borrowed")).unwrap();
    let (prepared, allocations) = crate::probe_runtime_allocations(|| {
        plan.prepare_as::<AdapterTerminal>(19, |value| (value == 7).then_some(3))
            .ok()
            .unwrap()
    });
    assert_eq!(allocations, 2);
    assert_eq!(prepared.selection.as_slice(), &[3]);
    assert_eq!(prepared.buffers.request_info(0), Some((false, 8)));
}
