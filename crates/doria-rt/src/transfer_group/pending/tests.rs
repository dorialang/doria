use crate::transfer_group::adapter::DrAdapterContextV1;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::*;
use crate::transfer_group::adapter::OwnedIoError;
use crate::transfer_group::{
    BufferList, Control, RequestSlots, SetupFailure, Stop, TransferGroup, Wait,
};
use doria_diagnostic_catalogue::IoMessageOperation;

mod hosted;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Event {
    Reset(usize, usize),
    Prepare(usize, usize, usize, usize),
    Submit(usize, usize, usize, usize),
    Endpoint(usize, usize, usize),
    Query(usize),
    Wait,
    Cancel(usize),
    Retire(usize),
    ReleaseSource(usize),
    Complete {
        index: usize,
        frame: usize,
        caller: usize,
        length: usize,
        tag: u8,
        submitted: u8,
        immediate: Option<(u8, usize)>,
        code: Option<i64>,
        progress: Option<usize>,
        value: u64,
        reason: u8,
        selected: (usize, usize, usize),
    },
    Frame(usize),
}

type Ledger = Rc<RefCell<Vec<Event>>>;

fn ledger() -> Ledger {
    // Configuration-only storage; bounded fixtures stay below this capacity.
    Rc::new(RefCell::new(Vec::with_capacity(256)))
}

fn record(ledger: &Ledger, event: Event) {
    ledger.borrow_mut().push(event);
}

struct ErrorPayload {
    drops: Rc<Cell<usize>>,
    message: &'static str,
    target: &'static str,
    system_code: i64,
    progress: Option<i64>,
}

unsafe extern "C" fn drop_error(_: *const DrStackFrameV2, object: *mut u8) {
    let payload = Box::from_raw(object.cast::<ErrorPayload>());
    payload.drops.set(payload.drops.get() + 1);
}

fn error_descriptor() -> crate::DrInterfaceVtableV1 {
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
) -> (OwnedIoError, *mut u8) {
    let object = Box::into_raw(Box::new(ErrorPayload {
        drops: Rc::clone(drops),
        message: "complete outer adapter error",
        target: "original/transformed/target",
        system_code: 91,
        progress,
    }))
    .cast::<u8>();
    let error = unsafe {
        OwnedIoError::from_carrier(DrInterfaceCarrierV1 {
            object,
            vtable: descriptor,
        })
    };
    (error, object)
}

#[derive(Clone, Copy)]
enum Selection {
    Caller,
    Storage,
    Invalid,
    Empty,
}

struct Context {
    index: usize,
    ledger: Ledger,
    prepares: usize,
    completes: usize,
    selection: Selection,
    endpoint: *const c_void,
    storage: [u8; 8],
    immediate: Option<(u8, usize)>,
    outer: Option<(u8, usize)>,
    preparation_error: Option<OwnedIoError>,
    completion_error: Option<OwnedIoError>,
    error_prefix: Option<usize>,
    cancel_on_prepare: Option<Rc<Cell<bool>>>,
    transform: bool,
    configuration_test: u8,
    null_completion: bool,
}

impl Context {
    fn new(index: usize, ledger: &Ledger) -> Box<Self> {
        Box::new(Self {
            index,
            ledger: Rc::clone(ledger),
            prepares: 0,
            completes: 0,
            selection: Selection::Caller,
            endpoint: endpoint(index),
            storage: [0; 8],
            immediate: None,
            outer: None,
            preparation_error: None,
            completion_error: None,
            error_prefix: None,
            cancel_on_prepare: None,
            transform: false,
            configuration_test: 0,
            null_completion: false,
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
    context.prepares += 1;
    record(
        &context.ledger,
        Event::Prepare(context.index, frame as usize, caller as usize, length),
    );
    if let Some(cancelled) = &context.cancel_on_prepare {
        cancelled.set(true);
    }
    if let Some(owned) = context.preparation_error.take() {
        (*outcome).count = context.error_prefix.unwrap_or(0);
        (*outcome).has_count = u8::from(context.error_prefix.is_some());
        error.write(owned.into_carrier());
        return 1;
    }
    if let Some((tag, count)) = context.immediate {
        (*outcome).tag = tag;
        (*outcome).count = count;
        return 0;
    }
    *region = match context.selection {
        Selection::Caller => DrPendingRegionV1 {
            endpoint: context.endpoint,
            data: caller,
            length,
        },
        Selection::Storage => DrPendingRegionV1 {
            endpoint: context.endpoint,
            data: context.storage.as_mut_ptr(),
            length: 4,
        },
        Selection::Invalid => DrPendingRegionV1 {
            endpoint: context.endpoint,
            data: ptr::null_mut(),
            length: 1,
        },
        Selection::Empty => DrPendingRegionV1 {
            endpoint: context.endpoint,
            data: ptr::null_mut(),
            length: 0,
        },
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
    let mode = (*(*context).object.cast::<Context>()).configuration_test;
    if mode == 1 {
        let _ = prepare(frame, context, caller, length, &mut region, outcome, error);
        return 0;
    }
    match prepare(frame, context, caller, length, &mut region, outcome, error) {
        2 => {
            dr_v1_pending_request_configure_native(
                view,
                region.endpoint,
                region.data,
                region.length,
            );
            if mode == 2 {
                dr_v1_pending_request_configure_native(
                    view,
                    region.endpoint,
                    region.data,
                    region.length,
                );
            } else if mode == 3 {
                dr_v1_pending_request_complete(view, 0, 0);
            }
            0
        }
        0 => {
            dr_v1_pending_request_complete(view, (*outcome).tag, (*outcome).count);
            if mode == 4 {
                dr_v1_pending_request_configure_native(view, endpoint(0), caller, length);
            }
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

unsafe fn complete(call: CompleteCall) -> u8 {
    let CompleteCall {
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
    } = call;
    let context = &mut *(*context).object.cast::<Context>();
    let native = &*native;
    let selected = &*selected;
    let immediate = immediate
        .as_ref()
        .map(|outcome| (outcome.tag, outcome.count));
    context.completes += 1;
    record(
        &context.ledger,
        Event::Complete {
            index: context.index,
            frame: frame as usize,
            caller: caller as usize,
            length,
            tag,
            submitted,
            immediate,
            code: (native.has_system_code != 0).then_some(native.system_code),
            progress: (native.has_transferred_byte_count != 0)
                .then_some(native.transferred_byte_count),
            value: native.value,
            reason: native.reason,
            selected: (
                selected.endpoint as usize,
                selected.data as usize,
                selected.length,
            ),
        },
    );
    if let Some(owned) = context.completion_error.take() {
        (*outcome).count = context.error_prefix.unwrap_or(0);
        (*outcome).has_count = u8::from(context.error_prefix.is_some());
        error.write(owned.into_carrier());
        return 1;
    }
    if context.null_completion {
        return 4;
    }
    if tag == 4 {
        // There is no native outcome for successful unsubmitted cleanup. This
        // deliberately invalid success shape proves the bridge never decodes
        // or requires a placeholder WouldBlock/Data(0)/EOF from that slot.
        outcome.write(DrAdapterOutcomeV1 {
            count: usize::MAX,
            tag: u8::MAX,
            has_count: u8::MAX,
        });
        return 0;
    }
    if context.transform && tag == 0 && submitted == 1 {
        for index in 0..(native.value as usize)
            .min(length)
            .min(context.storage.len())
        {
            caller.add(index).write(context.storage[index] ^ 0xff);
        }
    }
    let (outer_tag, count) = context
        .outer
        .or(immediate)
        .unwrap_or((0, if tag == 0 { native.value as usize } else { 0 }));
    (*outcome).tag = outer_tag;
    (*outcome).count = count;
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
    complete(CompleteCall {
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
    })
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
    complete(CompleteCall {
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
    })
}

#[derive(Clone, Copy)]
enum NativeSubmission {
    Complete,
    Pending,
    Rejected(Failure),
}

#[derive(Clone, Copy)]
struct Spec {
    submission: NativeSubmission,
    result: Result<usize, Failure>,
    query_failure: Option<Failure>,
    cancel_failure: Option<Failure>,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            submission: NativeSubmission::Pending,
            result: Ok(2),
            query_failure: None,
            cancel_failure: None,
        }
    }
}

#[derive(Default)]
struct Behavior {
    competing: bool,
    specs: [Spec; 3],
    ready: [bool; 3],
    release_on_wait: bool,
    wait_failure: Option<Failure>,
    resource_failure: Option<usize>,
}

struct NativeRequest {
    endpoint: *const c_void,
    data: *mut u8,
    live: bool,
    terminal: Option<Result<usize, Failure>>,
}

impl NativeRequest {
    fn new() -> Self {
        Self {
            endpoint: ptr::null(),
            data: ptr::null_mut(),
            live: false,
            terminal: None,
        }
    }
}

struct Native {
    ledger: Ledger,
    behavior: Rc<RefCell<Behavior>>,
    frame: Rc<Cell<usize>>,
}

static ENDPOINTS: [u8; 3] = [1, 2, 3];

fn endpoint(index: usize) -> *const c_void {
    ptr::addr_of!(ENDPOINTS[index]).cast()
}

// These model leaf tokens are configured independently from all readiness
// metadata and compared by identity; validation never dereferences a token.
unsafe impl PendingNativeDriver for Native {
    unsafe fn release_source(&mut self, index: usize, _: *mut NativeRequest) {
        record(&self.ledger, Event::ReleaseSource(index));
    }

    unsafe fn validate_endpoint(
        &self,
        index: usize,
        request: *mut NativeRequest,
        selected: *const c_void,
    ) -> Result<(), Failure> {
        record(
            &self.ledger,
            Event::Endpoint(index, request as usize, selected as usize),
        );
        let registered = if request.is_null() {
            ptr::null()
        } else {
            // Configuration metadata only; preparation precedes native I/O.
            unsafe { ptr::addr_of!((*request).endpoint).read() }
        };
        if selected == endpoint(index) && selected == registered {
            Ok(())
        } else {
            Err(Failure::invalid_input())
        }
    }
}

impl Native {
    fn new(ledger: &Ledger, behavior: &Rc<RefCell<Behavior>>) -> Self {
        Self {
            ledger: Rc::clone(ledger),
            behavior: Rc::clone(behavior),
            frame: Rc::new(Cell::new(0)),
        }
    }

    unsafe fn retire(&self, index: usize, request: *mut NativeRequest) {
        (*request).live = false;
        (*request).data = ptr::null_mut();
        (*request).terminal = None;
        record(&self.ledger, Event::Retire(index));
    }
}

// Only this synchronous model owns native byte access. A terminal query/drain
// clears it before the pending wrapper invokes complete; cancel never retires.
unsafe impl GroupDriver for Native {
    type Request = NativeRequest;
    type Timeout = u32;
    const MAXIMUM_TRANSFER_BYTES: usize = isize::MAX as usize;
    unsafe fn resource(
        &self,
        index: usize,
        _: *mut NativeRequest,
    ) -> Result<crate::transfer_group::TransferResource, Failure> {
        if self.behavior.borrow().resource_failure == Some(index) {
            return Err(Failure::unsupported());
        }
        Ok(crate::transfer_group::TransferResource {
            identity: if self.behavior.borrow().competing {
                0
            } else {
                index
            },
            shared_cursor: false,
        })
    }

    fn set_call_frame(&mut self, frame: *const DrStackFrameV2) {
        self.frame.set(frame as usize);
        record(&self.ledger, Event::Frame(frame as usize));
    }

    unsafe fn reset_request(&mut self, index: usize, request: *mut NativeRequest) {
        request.write(NativeRequest::new());
        (*request).endpoint = endpoint(index);
        record(&self.ledger, Event::Reset(index, request as usize));
    }

    unsafe fn submit(
        &mut self,
        index: usize,
        request: *mut NativeRequest,
        buffer: &mut Buffer<'_>,
    ) -> Submission {
        let data = match buffer {
            Buffer::Read(bytes) => bytes.as_mut_ptr(),
            Buffer::Write(bytes) => bytes.as_ptr().cast_mut(),
        };
        record(
            &self.ledger,
            Event::Submit(index, request as usize, data as usize, buffer.len()),
        );
        let spec = self.behavior.borrow().specs[index];
        if let NativeSubmission::Rejected(failure) = spec.submission {
            return Submission::Failed(failure);
        }
        if let Buffer::Read(bytes) = buffer {
            for (destination, value) in bytes.iter_mut().zip([11, 22, 33, 44]) {
                *destination = value;
            }
        }
        (*request).data = data;
        (*request).live = true;
        match spec.submission {
            NativeSubmission::Complete => {
                (*request).terminal = Some(spec.result);
                Submission::Complete
            }
            NativeSubmission::Pending => Submission::Pending,
            NativeSubmission::Rejected(_) => unreachable!(),
        }
    }

    unsafe fn query(&mut self, index: usize, request: *mut NativeRequest) -> Query {
        record(&self.ledger, Event::Query(index));
        let behavior = self.behavior.borrow();
        let spec = behavior.specs[index];
        if let Some(failure) = spec.query_failure {
            return Query::PendingFailure(IoMessageOperation::Wait, failure);
        }
        let terminal = (*request)
            .terminal
            .or_else(|| behavior.ready[index].then_some(spec.result));
        drop(behavior);
        if let Some(result) = terminal {
            self.retire(index, request);
            Query::Terminal(result)
        } else {
            Query::Pending
        }
    }

    fn wait_any(&mut self, _: u32, _: Option<usize>) -> Result<Wake, Failure> {
        record(&self.ledger, Event::Wait);
        let mut behavior = self.behavior.borrow_mut();
        if let Some(failure) = behavior.wait_failure {
            return Err(failure);
        }
        if behavior.release_on_wait {
            behavior.ready.fill(true);
        }
        Ok(Wake::Completion)
    }

    unsafe fn cancel(
        &mut self,
        index: usize,
        _: *mut NativeRequest,
    ) -> Result<(), (IoMessageOperation, Failure)> {
        record(&self.ledger, Event::Cancel(index));
        self.behavior.borrow().specs[index]
            .cancel_failure
            .map_or(Ok(()), |failure| Err((IoMessageOperation::Wait, failure)))
    }

    unsafe fn drain(
        &mut self,
        index: usize,
        request: *mut NativeRequest,
    ) -> Option<Result<usize, Failure>> {
        let result = self.behavior.borrow().specs[index].result;
        self.retire(index, request);
        Some(result)
    }
}

struct Budget {
    cancelled: Rc<Cell<bool>>,
    calls: Rc<Cell<usize>>,
}

impl Budget {
    fn new(cancelled: &Rc<Cell<bool>>) -> Self {
        Self {
            cancelled: Rc::clone(cancelled),
            calls: Rc::new(Cell::new(0)),
        }
    }
}

impl Control for Budget {
    fn next_wait(&mut self) -> Result<Result<Wait<u32>, Stop>, Failure> {
        self.calls.set(self.calls.get() + 1);
        if self.cancelled.get() {
            Ok(Err(Stop::Cancellation))
        } else {
            Ok(Ok(Wait {
                timeout: 10,
                immediate: false,
            }))
        }
    }
    fn cancellation_signal(&self) -> Option<usize> {
        None
    }
}

fn records(contexts: &mut [&mut Context]) -> FixedSlice<PendingState> {
    let mut records = FixedSlice::new(contexts.len()).unwrap();
    for context in contexts {
        records
            .push(PendingState::new(context.adapter()).unwrap())
            .unwrap();
    }
    records
}

fn position(events: &[Event], predicate: impl Fn(&Event) -> bool) -> usize {
    events
        .iter()
        .position(predicate)
        .expect("expected lifecycle event")
}

fn assert_retirement_order(events: &[Event], indices: &[usize]) {
    for index in indices {
        let retired = position(events, |event| *event == Event::Retire(*index));
        let completed = position(
            events,
            |event| matches!(event, Event::Complete { index: actual, submitted: 1, .. } if actual == index),
        );
        assert!(retired < completed);
        for cancelled in indices {
            let cancel = position(events, |event| *event == Event::Cancel(*cancelled));
            assert!(cancel < retired, "cancel every request before any drain");
        }
    }
}

#[test]
fn six_word_pending_descriptor_and_four_word_sync_descriptor_preserve_direction_pairs() {
    assert_eq!(
        mem::size_of::<DrPendingTransferAdapterV1>(),
        6 * mem::size_of::<usize>()
    );
    assert_eq!(
        mem::size_of::<DrPendingRegionV1>(),
        3 * mem::size_of::<usize>()
    );
    assert_eq!(
        mem::size_of::<DrTransferAdapterV1>(),
        4 * mem::size_of::<usize>()
    );
    assert_eq!(mem::offset_of!(DrPendingTransferAdapterV1, context), 0);
    assert_eq!(
        mem::offset_of!(DrPendingTransferAdapterV1, prepare_read),
        2 * mem::size_of::<usize>()
    );
    assert_eq!(
        mem::offset_of!(DrPendingTransferAdapterV1, prepare_write),
        3 * mem::size_of::<usize>()
    );
    assert_eq!(
        mem::offset_of!(DrPendingTransferAdapterV1, complete_read),
        4 * mem::size_of::<usize>()
    );
    assert_eq!(
        mem::offset_of!(DrPendingTransferAdapterV1, complete_write),
        5 * mem::size_of::<usize>()
    );
    let events = ledger();
    let mut context = Context::new(0, &events);
    let mut adapter = context.adapter();
    assert!(adapter.validate(true).is_ok() && adapter.validate(false).is_ok());
    adapter.complete_read = None;
    assert!(!adapter.valid());
    assert!(PendingState::new(adapter).is_err());
    adapter.prepare_read = None;
    assert!(adapter.valid());
    assert!(adapter.validate(true).is_err());
    assert!(adapter.validate(false).is_ok());
}

#[test]
fn queued_preparations_cancel_exactly_once_preserving_original_error_and_immediate_fact() {
    let descriptor = error_descriptor();
    let events = ledger();
    let behavior = Rc::new(RefCell::new(Behavior::default()));
    behavior.borrow_mut().competing = true;
    let cancelled = Rc::new(Cell::new(false));
    let drops = Rc::new(Cell::new(0));
    let (error, original) = owned_error(&descriptor, &drops, None);
    let mut a = Context::new(0, &events);
    let mut b = Context::new(1, &events);
    let mut c = Context::new(2, &events);
    b.completion_error = Some(error);
    c.immediate = Some((0, 1));
    let mut states = records(&mut [&mut a, &mut b, &mut c]);
    let mut slots = RequestSlots::new(&[10, 20, 30], |_| NativeRequest::new()).unwrap();
    let buffers = BufferList::from_iter(
        [
            Buffer::Write(b"one"),
            Buffer::Write(b"two"),
            Buffer::Write(b"three"),
        ]
        .into_iter(),
    )
    .unwrap();
    let driver =
        unsafe { PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null()) };
    let mut group =
        TransferGroup::begin_prepared(&mut slots, buffers, driver, Budget::new(&cancelled))
            .unwrap();
    assert_eq!((a.prepares, b.prepares, c.prepares), (1, 1, 1));
    let snapshot = events.borrow();
    assert_eq!(
        snapshot
            .iter()
            .filter(|event| matches!(event, Event::Submit(..)))
            .count(),
        1
    );
    assert!(
        position(&snapshot, |event| matches!(event, Event::Prepare(2, ..)))
            < position(&snapshot, |event| matches!(event, Event::Submit(0, ..)))
    );
    drop(snapshot);
    cancelled.set(true);
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        let first = group.next().unwrap();
        assert!(matches!(
            first.outcome.transfer,
            Some(AdapterTerminal::Accepted(2))
        ));
        assert_eq!(first.outcome.stop, Some(Stop::Cancellation));
        let second = group.next().unwrap();
        assert_eq!(second.index, 1);
        assert_eq!(second.outcome.stop, Some(Stop::Cancellation));
        let Some(AdapterTerminal::CheckedError { error, progress }) = second.outcome.transfer
        else {
            panic!("queued completion error was lost")
        };
        assert_eq!(progress, None);
        let carrier = error.into_carrier();
        assert_eq!(carrier.object, original);
        assert_eq!(carrier.vtable, &descriptor as *const _);
        drop(unsafe { OwnedIoError::from_carrier(carrier) });
        let third = group.next().unwrap();
        assert!(matches!(
            third.outcome.transfer,
            Some(AdapterTerminal::Accepted(1))
        ));
        assert_eq!(third.outcome.stop, Some(Stop::Cancellation));
        assert!(group.next().is_none());
        drop(group);
    });
    assert_eq!(allocations, 0);
    assert_eq!(drops.get(), 1);
    assert_eq!((a.completes, b.completes, c.completes), (1, 1, 1));
    let snapshot = events.borrow();
    assert_eq!(
        snapshot
            .iter()
            .filter(|event| matches!(event, Event::Cancel(_)))
            .count(),
        1
    );
    assert_eq!(
        snapshot
            .iter()
            .filter(|event| matches!(event, Event::Retire(_)))
            .count(),
        1
    );
    assert!(snapshot.iter().any(|event| matches!(
        event,
        Event::Complete {
            index: 1,
            submitted: 0,
            tag: 4,
            progress: None,
            ..
        }
    )));
    assert!(snapshot.iter().any(|event| matches!(
        event,
        Event::Complete {
            index: 2,
            submitted: 0,
            immediate: Some((0, 1)),
            ..
        }
    )));
    drop(snapshot);
    cancelled.set(false);
    behavior.borrow_mut().ready.fill(true);
    let buffers = BufferList::from_iter(
        [
            Buffer::Write(b"one"),
            Buffer::Write(b"two"),
            Buffer::Write(b"three"),
        ]
        .into_iter(),
    )
    .unwrap();
    let driver =
        unsafe { PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null()) };
    let reuse_budget = Budget::new(&cancelled);
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        let mut reused =
            TransferGroup::begin_prepared(&mut slots, buffers, driver, reuse_budget).unwrap();
        for index in 0..3 {
            assert_eq!(reused.next().unwrap().index, index);
        }
        assert!(reused.next().is_none());
    });
    assert_eq!(allocations, 0);
    assert_eq!((a.completes, b.completes, c.completes), (2, 2, 2));
}

#[test]
fn immediate_completed_prefix_is_available_before_fetch_and_retirement_runs_once() {
    for fetch in [false, true] {
        let events = ledger();
        let behavior = Rc::new(RefCell::new(Behavior::default()));
        let mut context = Context::new(0, &events);
        context.immediate = Some((0, 0));
        context.outer = Some((0, 2));
        let mut states = records(&mut [&mut context]);
        let mut slots = RequestSlots::new(&[10], |_| NativeRequest::new()).unwrap();
        let mut bytes = [11, 12, 13];
        let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
        let driver = unsafe {
            PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null())
        };
        let mut group = TransferGroup::begin_prepared(
            &mut slots,
            buffers,
            driver,
            Budget::new(&Rc::new(Cell::new(false))),
        )
        .unwrap();
        assert_eq!(context.completes, 1);
        assert_eq!(group.completed(0), Some(&[11, 12][..]));
        if fetch {
            let completion = group.next().unwrap();
            assert!(matches!(
                completion.outcome.transfer,
                Some(AdapterTerminal::Data(2))
            ));
            assert!(group.next().is_none());
        }
        drop(group);
        assert_eq!(context.completes, 1);
        assert!(!events.borrow().iter().any(|event| matches!(
            event,
            Event::Submit(..)
                | Event::Query(..)
                | Event::Wait
                | Event::Cancel(..)
                | Event::Retire(..)
        )));
    }
}

#[test]
fn immediate_completion_error_keeps_its_exact_carrier_until_fetch_or_destruction() {
    let descriptor = error_descriptor();
    for fetch in [false, true] {
        let events = ledger();
        let behavior = Rc::new(RefCell::new(Behavior::default()));
        let drops = Rc::new(Cell::new(0));
        let (error, original) = owned_error(&descriptor, &drops, None);
        let mut context = Context::new(0, &events);
        context.immediate = Some((0, 0));
        context.completion_error = Some(error);
        let mut states = records(&mut [&mut context]);
        let mut slots = RequestSlots::new(&[10], |_| NativeRequest::new()).unwrap();
        let mut bytes = [0];
        let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
        let driver = unsafe {
            PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null())
        };
        let mut group = TransferGroup::begin_prepared(
            &mut slots,
            buffers,
            driver,
            Budget::new(&Rc::new(Cell::new(false))),
        )
        .unwrap();
        assert_eq!(context.completes, 1);
        assert_eq!(drops.get(), 0);
        assert_eq!(group.completed(0), None);
        if fetch {
            let completion = group.next().unwrap();
            let Some(AdapterTerminal::CheckedError { error, progress }) =
                completion.outcome.transfer
            else {
                panic!("queued immediate error was lost")
            };
            assert_eq!(progress, None);
            let carrier = error.into_carrier();
            assert_eq!(carrier.object, original);
            assert_eq!(carrier.vtable, &descriptor as *const _);
            assert_eq!(drops.get(), 0);
            drop(unsafe { OwnedIoError::from_carrier(carrier) });
            assert_eq!(drops.get(), 1);
            assert!(group.next().is_none());
        }
        drop(group);
        assert_eq!(context.completes, 1);
        assert_eq!(drops.get(), 1);
        assert!(!events.borrow().iter().any(|event| matches!(
            event,
            Event::Submit(..)
                | Event::Query(..)
                | Event::Wait
                | Event::Cancel(..)
                | Event::Retire(..)
        )));
    }
}

#[test]
fn batch_preparation_precedes_submission_and_immediate_outer_metering() {
    let events = ledger();
    let behavior = Rc::new(RefCell::new(Behavior::default()));
    behavior.borrow_mut().specs[0].submission = NativeSubmission::Complete;
    let mut a = Context::new(0, &events);
    let mut b = Context::new(1, &events);
    b.immediate = Some((0, 0));
    b.outer = Some((0, 3));
    let mut states = records(&mut [&mut a, &mut b]);
    let mut slots = RequestSlots::new(&[10, 20], |_| NativeRequest::new()).unwrap();
    let buffers =
        BufferList::from_iter([Buffer::Write(b"abcd"), Buffer::Write(b"meter")].into_iter())
            .unwrap();
    let native = Native::new(&events, &behavior);
    let frame_observer = Rc::clone(&native.frame);
    let frame: DrStackFrameV2 = unsafe { mem::zeroed() };
    let driver = unsafe { PendingDriver::new(native, &mut states, &frame) };
    let mut group = TransferGroup::begin_prepared(
        &mut slots,
        buffers,
        driver,
        Budget::new(&Rc::new(Cell::new(false))),
    )
    .unwrap();
    assert_eq!(frame_observer.get(), 0);
    let first = unsafe { group.with_call_frame(&frame, |group| group.next()) }.unwrap();
    let second = unsafe { group.with_call_frame(&frame, |group| group.next()) }.unwrap();
    assert_ne!(first.index, second.index);
    for completion in [first, second] {
        assert!(matches!(
            (completion.index, completion.outcome.transfer),
            (0, Some(AdapterTerminal::Accepted(2))) | (1, Some(AdapterTerminal::Accepted(3)))
        ));
    }
    assert_eq!(
        (a.prepares, b.prepares, a.completes, b.completes),
        (1, 1, 1, 1)
    );
    let snapshot = events.borrow();
    let last_prepare = snapshot
        .iter()
        .rposition(|event| matches!(event, Event::Prepare(..)))
        .unwrap();
    let first_dispatch = position(&snapshot, |event| {
        matches!(event, Event::Submit(..) | Event::Complete { .. })
    });
    assert!(last_prepare < first_dispatch);
    assert!(!snapshot
        .iter()
        .any(|event| matches!(event, Event::Submit(1, ..))));
    assert!(snapshot.iter().any(|event| matches!(
        event,
        Event::Complete {
            index: 1,
            submitted: 0,
            immediate: Some((0, 0)),
            ..
        }
    )));
    assert_eq!(frame_observer.get(), 0);
}

#[test]
fn preparation_checked_error_moves_exact_carrier_and_untaken_cleanup_runs_once() {
    let descriptor = error_descriptor();
    for take in [false, true] {
        let events = ledger();
        let behavior = Rc::new(RefCell::new(Behavior::default()));
        let drops = Rc::new(Cell::new(0));
        let (error, original) = owned_error(&descriptor, &drops, Some(-3));
        let mut context = Context::new(0, &events);
        context.preparation_error = Some(error);
        let mut states = records(&mut [&mut context]);
        let mut slots = RequestSlots::new(&[10], |_| NativeRequest::new()).unwrap();
        let buffers = BufferList::from_iter([Buffer::Write(b"loan")].into_iter()).unwrap();
        let driver = unsafe {
            PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null())
        };
        let setup = TransferGroup::begin_prepared(
            &mut slots,
            buffers,
            driver,
            Budget::new(&Rc::new(Cell::new(false))),
        );
        let failure = match setup {
            Err(failure) => failure,
            Ok(group) => {
                drop(group);
                panic!("preparation error must fail begin before submission")
            }
        };
        if take {
            let SetupFailure::Preparation(AdapterTerminal::CheckedError { error, progress }) =
                failure
            else {
                panic!("preparation error must not be reconstructed")
            };
            assert_eq!(progress, None);
            let carrier = error.into_carrier();
            assert_eq!(carrier.object, original);
            assert_eq!(carrier.vtable, &descriptor as *const _);
            let payload = unsafe { &*carrier.object.cast::<ErrorPayload>() };
            assert_eq!(
                (
                    payload.message,
                    payload.target,
                    payload.system_code,
                    payload.progress
                ),
                (
                    "complete outer adapter error",
                    "original/transformed/target",
                    91,
                    Some(-3)
                )
            );
            assert_eq!(drops.get(), 0);
            drop(unsafe { OwnedIoError::from_carrier(carrier) });
        } else {
            drop(failure);
        }
        assert_eq!(drops.get(), 1);
        assert_eq!((context.prepares, context.completes), (1, 0));
        assert!(!events.borrow().iter().any(|event| matches!(
            event,
            Event::Submit(..) | Event::Retire(..) | Event::Complete { .. }
        )));
    }
}

#[test]
fn native_rejection_and_invalid_endpoint_finish_once_without_submission_claim() {
    for rejection in 0..3 {
        let events = ledger();
        let behavior = Rc::new(RefCell::new(Behavior::default()));
        let mut context = Context::new(0, &events);
        match rejection {
            0 => context.selection = Selection::Invalid,
            1 => context.endpoint = endpoint(1),
            _ => {
                behavior.borrow_mut().specs[0].submission = NativeSubmission::Rejected(
                    Failure::from_system_code(93).with_transferred_byte_count(Some(9)),
                );
            }
        }
        let mut states = records(&mut [&mut context]);
        let mut slots = RequestSlots::new(&[10], |_| NativeRequest::new()).unwrap();
        let buffers = BufferList::from_iter([Buffer::Write(b"loan")].into_iter()).unwrap();
        let driver = unsafe {
            PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null())
        };
        let setup = TransferGroup::begin_prepared(
            &mut slots,
            buffers,
            driver,
            Budget::new(&Rc::new(Cell::new(false))),
        );
        if rejection == 2 {
            let mut group = setup.unwrap();
            let result = group.next().unwrap();
            assert!(matches!(
                result.outcome.transfer,
                Some(AdapterTerminal::Accepted(0))
            ));
            assert!(group.next().is_none());
        } else {
            assert!(
                matches!(setup, Err(SetupFailure::Preparation(AdapterTerminal::PlatformFailure(failure))) if failure == Failure::invalid_input())
            );
        }
        assert_eq!(context.completes, 1);
        let snapshot = events.borrow();
        let completed = snapshot
            .iter()
            .find(|event| matches!(event, Event::Complete { .. }))
            .unwrap();
        assert!(matches!(
            completed,
            Event::Complete {
                submitted: 0,
                immediate: None,
                progress: None,
                ..
            }
        ));
        if rejection == 2 {
            assert!(matches!(
                completed,
                Event::Complete {
                    code: Some(93),
                    tag: 3,
                    selected: (_, _, 4),
                    ..
                }
            ));
            assert_eq!(
                snapshot
                    .iter()
                    .filter(|event| matches!(event, Event::Submit(..)))
                    .count(),
                1
            );
        } else {
            assert!(!snapshot
                .iter()
                .any(|event| matches!(event, Event::Submit(..))));
            assert!(matches!(
                completed,
                Event::Complete {
                    tag: 4,
                    code: None,
                    value: 0,
                    reason: 0,
                    ..
                }
            ));
            if rejection == 0 {
                assert!(
                    matches!(completed, Event::Complete { selected: (leaf, 0, 1), .. } if *leaf == endpoint(0) as usize)
                );
            } else {
                assert!(
                    matches!(completed, Event::Complete { selected: (leaf, _, 4), .. } if *leaf == endpoint(1) as usize)
                );
            }
        }
        assert!(!snapshot
            .iter()
            .any(|event| matches!(event, Event::Retire(..) | Event::Cancel(..))));
    }
}

#[test]
fn pending_query_retires_before_completion_and_reuses_stable_request_without_allocations() {
    let events = ledger();
    let behavior = Rc::new(RefCell::new(Behavior::default()));
    behavior.borrow_mut().release_on_wait = true;
    let mut context = Context::new(0, &events);
    let mut states = records(&mut [&mut context]);
    let mut slots = RequestSlots::new(&[10], |_| NativeRequest::new()).unwrap();
    let original = slots.raw(0) as usize;
    let buffers = BufferList::from_iter([Buffer::Write(b"abcdef")].into_iter()).unwrap();
    let native = Native::new(&events, &behavior);
    let observed_frame = Rc::clone(&native.frame);
    let driver = unsafe { PendingDriver::new(native, &mut states, ptr::null()) };
    let mut group = TransferGroup::begin_prepared(
        &mut slots,
        buffers,
        driver,
        Budget::new(&Rc::new(Cell::new(false))),
    )
    .unwrap();
    assert_eq!(context.completes, 0);
    let frame: DrStackFrameV2 = unsafe { mem::zeroed() };
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        let first = group.with_call_frame(&frame, |group| group.next()).unwrap();
        assert!(matches!(
            first.outcome.transfer,
            Some(AdapterTerminal::Accepted(2))
        ));
        assert_eq!(group.completed(0), Some(&b"ab"[..]));
        group
            .with_call_frame(&frame, |group| group.submit(0, 2, 3))
            .unwrap();
        let second = group.with_call_frame(&frame, |group| group.next()).unwrap();
        assert!(matches!(
            second.outcome.transfer,
            Some(AdapterTerminal::Accepted(2))
        ));
        assert_eq!(group.completed(0), Some(&b"cd"[..]));
    });
    assert_eq!(allocations, 0);
    assert_eq!((context.prepares, context.completes), (2, 2));
    assert_eq!(observed_frame.get(), 0);
    let snapshot = events.borrow();
    let waited = position(&snapshot, |event| *event == Event::Wait);
    let retired = position(&snapshot, |event| *event == Event::Retire(0));
    let completed = position(&snapshot, |event| {
        matches!(event, Event::Complete { index: 0, .. })
    });
    assert!(waited < retired && retired < completed);
    assert!(snapshot
        .iter()
        .filter_map(|event| match event {
            Event::Reset(0, address) | Event::Submit(0, address, ..) => Some(*address),
            _ => None,
        })
        .all(|address| address == original));
    assert!(snapshot
        .iter()
        .filter_map(|event| match event {
            Event::Complete { frame, .. } => Some(*frame),
            _ => None,
        })
        .all(|address| address == &frame as *const _ as usize));
    drop(snapshot);
    drop(group);
    assert_eq!(slots.raw(0) as usize, original);
}

#[test]
fn native_completion_winning_cancellation_is_not_suppressed_by_preflight() {
    let events = ledger();
    let behavior = Rc::new(RefCell::new(Behavior::default()));
    let cancelled = Rc::new(Cell::new(false));
    let mut context = Context::new(0, &events);
    let mut states = records(&mut [&mut context]);
    let mut slots = RequestSlots::new(&[10], |_| NativeRequest::new()).unwrap();
    let buffers = BufferList::from_iter([Buffer::Write(b"loan")].into_iter()).unwrap();
    let driver =
        unsafe { PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null()) };
    let mut group =
        TransferGroup::begin_prepared(&mut slots, buffers, driver, Budget::new(&cancelled))
            .unwrap();
    cancelled.set(true);
    behavior.borrow_mut().ready[0] = true;
    let completed = group.next().unwrap();
    assert!(matches!(
        completed.outcome.transfer,
        Some(AdapterTerminal::Accepted(2))
    ));
    assert_eq!(completed.outcome.stop, None);
    assert_eq!((context.prepares, context.completes), (1, 1));
    let snapshot = events.borrow();
    assert!(!snapshot
        .iter()
        .any(|event| matches!(event, Event::Cancel(..) | Event::Wait)));
    assert!(
        position(&snapshot, |event| *event == Event::Retire(0))
            < position(&snapshot, |event| matches!(
                event,
                Event::Complete { submitted: 1, .. }
            ))
    );
}

#[test]
fn cancellation_failure_fans_out_before_drain_and_preserves_outer_error() {
    let descriptor = error_descriptor();
    let events = ledger();
    let behavior = Rc::new(RefCell::new(Behavior::default()));
    behavior.borrow_mut().specs[0].cancel_failure = Some(Failure::from_system_code(99));
    behavior.borrow_mut().specs[0].result = Ok(3);
    behavior.borrow_mut().specs[1].result =
        Err(Failure::from_system_code(777).with_transferred_byte_count(Some(1)));
    let drops = Rc::new(Cell::new(0));
    let (error, original) = owned_error(&descriptor, &drops, Some(1));
    let cancelled = Rc::new(Cell::new(false));
    let mut a = Context::new(0, &events);
    let mut b = Context::new(1, &events);
    b.completion_error = Some(error);
    b.error_prefix = Some(1);
    let mut states = records(&mut [&mut a, &mut b]);
    let mut slots = RequestSlots::new(&[10, 20], |_| NativeRequest::new()).unwrap();
    let buffers =
        BufferList::from_iter([Buffer::Write(b"loan"), Buffer::Write(b"other")].into_iter())
            .unwrap();
    let driver =
        unsafe { PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null()) };
    let mut group =
        TransferGroup::begin_prepared(&mut slots, buffers, driver, Budget::new(&cancelled))
            .unwrap();
    cancelled.set(true);
    let first = group.next().unwrap();
    assert!(matches!(
        first.outcome.transfer,
        Some(AdapterTerminal::Accepted(3))
    ));
    assert_eq!(first.outcome.stop, Some(Stop::Cancellation));
    assert_eq!(
        first.outcome.cancellation_failure,
        Some((IoMessageOperation::Wait, Failure::from_system_code(99)))
    );
    let second = group.next().unwrap();
    assert_eq!(second.outcome.stop, Some(Stop::Cancellation));
    let Some(AdapterTerminal::CheckedError { error, progress }) = second.outcome.transfer else {
        panic!("outer checked error was lost after native cancellation")
    };
    assert_eq!(progress, Some(1));
    let carrier = error.into_carrier();
    assert_eq!(carrier.object, original);
    assert_eq!(carrier.vtable, &descriptor as *const _);
    assert_eq!(
        unsafe { (*carrier.object.cast::<ErrorPayload>()).system_code },
        91
    );
    drop(unsafe { OwnedIoError::from_carrier(carrier) });
    assert_eq!(drops.get(), 1);
    assert_eq!((a.completes, b.completes), (1, 1));
    let snapshot = events.borrow();
    assert_retirement_order(&snapshot, &[0, 1]);
    assert!(snapshot.iter().any(|event| matches!(
        event,
        Event::Complete {
            index: 1,
            submitted: 1,
            tag: 3,
            code: Some(777),
            progress: Some(1),
            ..
        }
    )));
}

#[test]
fn query_and_wait_failures_still_cancel_and_retire_every_native_request() {
    for query_failure in [false, true] {
        let events = ledger();
        let behavior = Rc::new(RefCell::new(Behavior::default()));
        let failure = Failure::from_system_code(321);
        if query_failure {
            behavior.borrow_mut().specs[0].query_failure = Some(failure);
        } else {
            behavior.borrow_mut().wait_failure = Some(failure);
        }
        let mut a = Context::new(0, &events);
        let mut b = Context::new(1, &events);
        let mut states = records(&mut [&mut a, &mut b]);
        let mut slots = RequestSlots::new(&[10, 20], |_| NativeRequest::new()).unwrap();
        let buffers =
            BufferList::from_iter([Buffer::Write(b"loan"), Buffer::Write(b"more")].into_iter())
                .unwrap();
        let native = Native::new(&events, &behavior);
        let observed_frame = Rc::clone(&native.frame);
        let driver = unsafe { PendingDriver::new(native, &mut states, ptr::null()) };
        let mut group = TransferGroup::begin_prepared(
            &mut slots,
            buffers,
            driver,
            Budget::new(&Rc::new(Cell::new(false))),
        )
        .unwrap();
        let frame: DrStackFrameV2 = unsafe { mem::zeroed() };
        for _ in 0..2 {
            let completion =
                unsafe { group.with_call_frame(&frame, |group| group.next()) }.unwrap();
            assert_eq!(
                completion.outcome.stop,
                Some(Stop::Failure(IoMessageOperation::Wait, failure))
            );
            assert!(matches!(
                completion.outcome.transfer,
                Some(AdapterTerminal::Accepted(2))
            ));
        }
        assert_eq!(observed_frame.get(), 0);
        assert_eq!((a.completes, b.completes), (1, 1));
        assert_retirement_order(&events.borrow(), &[0, 1]);
    }
}

#[test]
fn transformed_storage_retains_selected_provenance_and_original_completion_region() {
    let events = ledger();
    let behavior = Rc::new(RefCell::new(Behavior::default()));
    behavior.borrow_mut().specs[0].submission = NativeSubmission::Complete;
    behavior.borrow_mut().specs[0].result = Ok(4);
    let mut context = Context::new(0, &events);
    context.selection = Selection::Storage;
    context.transform = true;
    let selected = context.storage.as_ptr() as usize;
    let mut bytes = [0_u8; 6];
    let original = bytes.as_mut_ptr() as usize;
    let mut states = records(&mut [&mut context]);
    let mut slots = RequestSlots::new(&[10], |_| NativeRequest::new()).unwrap();
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let driver =
        unsafe { PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null()) };
    let mut group = TransferGroup::begin_prepared(
        &mut slots,
        buffers,
        driver,
        Budget::new(&Rc::new(Cell::new(false))),
    )
    .unwrap();
    let completion = group.next().unwrap();
    assert!(matches!(
        completion.outcome.transfer,
        Some(AdapterTerminal::Data(4))
    ));
    assert_eq!(group.completed(0), Some(&[244, 233, 222, 211][..]));
    let snapshot = events.borrow();
    assert!(snapshot
        .iter()
        .any(|event| matches!(event, Event::Submit(0, _, data, 4) if *data == selected)));
    assert!(snapshot
        .iter()
        .any(|event| matches!(event, Event::Complete {
        caller, length: 6, submitted: 1, selected: (leaf, data, 4), ..
    } if *caller == original && *data == selected && *leaf == endpoint(0) as usize)));
    assert_ne!(selected, original);
    assert!(
        position(&snapshot, |event| *event == Event::Retire(0))
            < position(&snapshot, |event| matches!(event, Event::Complete { .. }))
    );
    drop(snapshot);
    drop(group);
    assert_eq!(bytes, [244, 233, 222, 211, 0, 0]);
    assert_eq!((context.prepares, context.completes), (1, 1));
}

#[test]
fn stopping_prepared_native_has_no_transfer_but_staged_immediate_completes_once() {
    for immediate in [false, true] {
        let events = ledger();
        let behavior = Rc::new(RefCell::new(Behavior::default()));
        let cancelled = Rc::new(Cell::new(false));
        let mut context = Context::new(0, &events);
        context.cancel_on_prepare = Some(Rc::clone(&cancelled));
        context.null_completion = !immediate;
        if immediate {
            context.immediate = Some((1, 0));
        }
        let mut states = records(&mut [&mut context]);
        let mut slots = RequestSlots::new(&[10], |_| NativeRequest::new()).unwrap();
        let buffers = BufferList::from_iter([Buffer::Write(b"loan")].into_iter()).unwrap();
        let driver = unsafe {
            PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null())
        };
        let mut group =
            TransferGroup::begin_prepared(&mut slots, buffers, driver, Budget::new(&cancelled))
                .unwrap();
        let completion = group.next().unwrap();
        assert_eq!(completion.outcome.stop, Some(Stop::Cancellation));
        if immediate {
            assert!(matches!(
                completion.outcome.transfer,
                Some(AdapterTerminal::WouldBlock)
            ));
            assert_eq!(context.completes, 1);
            assert!(events.borrow().iter().any(|event| matches!(
                event,
                Event::Complete {
                    submitted: 0,
                    immediate: Some((1, 0)),
                    selected: (0, 0, 0),
                    ..
                }
            )));
        } else {
            assert!(completion.outcome.transfer.is_none());
            assert_eq!(context.completes, 1);
            assert!(events.borrow().iter().any(|event| matches!(
                event,
                Event::Complete {
                    tag: 4,
                    submitted: 0,
                    immediate: None,
                    code: None,
                    progress: None,
                    value: 0,
                    reason: 0,
                    selected: (_, _, 4),
                    ..
                }
            )));
        }
        assert!(group.completed(0).is_none());
        assert!(group.next().is_none());
        drop(group);
        assert_eq!(context.completes, 1);
        let snapshot = events.borrow();
        assert!(!snapshot.iter().any(|event| matches!(
            event,
            Event::Submit(..) | Event::Retire(..) | Event::Cancel(..)
        )));
        assert!(matches!(unsafe { &(*states.raw(0)).phase }, Phase::Taken));
        assert!(unsafe { (*states.raw(0)).caller_data.is_null() });
        assert!(unsafe { (*states.raw(0)).native_region.data.is_null() });
    }
}

#[test]
fn empty_requests_dispatch_once_and_would_block_is_a_terminal_fact() {
    for immediate in [false, true] {
        let events = ledger();
        let behavior = Rc::new(RefCell::new(Behavior::default()));
        behavior.borrow_mut().specs[0].submission = NativeSubmission::Complete;
        behavior.borrow_mut().specs[0].result = Ok(0);
        let mut context = Context::new(0, &events);
        if immediate {
            context.immediate = Some((1, 0));
        } else {
            context.selection = Selection::Empty;
        }
        let mut states = records(&mut [&mut context]);
        let mut slots = RequestSlots::new(&[10], |_| NativeRequest::new()).unwrap();
        let buffers = BufferList::from_iter([Buffer::Write(b"")].into_iter()).unwrap();
        let budget = Budget::new(&Rc::new(Cell::new(false)));
        let clock_calls = Rc::clone(&budget.calls);
        let driver = unsafe {
            PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null())
        };
        let mut group = TransferGroup::begin_prepared(&mut slots, buffers, driver, budget).unwrap();
        let completion = group.next().unwrap();
        if immediate {
            assert!(matches!(
                completion.outcome.transfer,
                Some(AdapterTerminal::WouldBlock)
            ));
            assert_eq!(group.completed(0), None);
        } else {
            assert!(matches!(
                completion.outcome.transfer,
                Some(AdapterTerminal::Accepted(0))
            ));
            assert_eq!(group.completed(0), Some(&b""[..]));
        }
        assert_eq!(completion.outcome.stop, None);
        assert_eq!(clock_calls.get(), 0);
        assert_eq!((context.prepares, context.completes), (1, 1));
        assert!(group.next().is_none());
        assert!(!events
            .borrow()
            .iter()
            .any(|event| matches!(event, Event::Wait)));
    }
}

#[test]
fn drop_drains_pending_with_null_frame_and_leaves_request_reusable() {
    let events = ledger();
    let behavior = Rc::new(RefCell::new(Behavior::default()));
    let mut context = Context::new(0, &events);
    let mut states = records(&mut [&mut context]);
    let mut slots = RequestSlots::new(&[10], |_| NativeRequest::new()).unwrap();
    let original = slots.raw(0) as usize;
    let frame: DrStackFrameV2 = unsafe { mem::zeroed() };
    let native = Native::new(&events, &behavior);
    let observed_frame = Rc::clone(&native.frame);
    let driver = unsafe { PendingDriver::new(native, &mut states, &frame) };
    let buffers = BufferList::from_iter([Buffer::Write(b"loan")].into_iter()).unwrap();
    let group = TransferGroup::begin_prepared(
        &mut slots,
        buffers,
        driver,
        Budget::new(&Rc::new(Cell::new(false))),
    )
    .unwrap();
    assert_eq!(observed_frame.get(), 0);
    let ((), allocations) = crate::probe_runtime_allocations(|| drop(group));
    assert_eq!(allocations, 0);
    assert_eq!(context.completes, 1);
    assert_retirement_order(&events.borrow(), &[0]);
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        Event::Complete {
            frame: 0,
            submitted: 1,
            ..
        }
    )));
    assert_eq!(slots.raw(0) as usize, original);
    assert!(!unsafe { (*slots.raw(0)).live });
    assert!(unsafe { (*slots.raw(0)).data.is_null() });
    behavior.borrow_mut().ready[0] = true;
    let driver =
        unsafe { PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null()) };
    let buffers = BufferList::from_iter([Buffer::Write(b"reuse")].into_iter()).unwrap();
    let mut reused = TransferGroup::begin_prepared(
        &mut slots,
        buffers,
        driver,
        Budget::new(&Rc::new(Cell::new(false))),
    )
    .unwrap();
    assert!(matches!(
        reused.next().unwrap().outcome.transfer,
        Some(AdapterTerminal::Accepted(2))
    ));
    assert_eq!((context.prepares, context.completes), (2, 2));
    drop(reused);
    assert_eq!(slots.raw(0) as usize, original);
}

#[test]
fn sparse_selection_rejects_holes_before_preparing_then_preserves_plan_order() {
    let events = ledger();
    let behavior = Rc::new(RefCell::new(Behavior::default()));
    behavior.borrow_mut().ready.fill(true);
    let mut a = Context::new(0, &events);
    let mut b = Context::new(1, &events);
    let mut c = Context::new(2, &events);
    let mut states = StableSlots::new().unwrap();
    for context in [&mut *a, &mut *b, &mut *c] {
        states
            .insert(PendingState::new(context.adapter()).unwrap())
            .unwrap();
    }
    drop(states.remove(1));
    let mut slots = RequestSlots::dynamic().unwrap();
    for key in [10, 20, 30] {
        slots.add(key, NativeRequest::new()).unwrap();
    }
    slots.remove(1).unwrap();
    let original = slots.raw(2) as usize;
    let mut selection = FixedSlice::new(2).unwrap();
    selection.push(2).unwrap();
    selection.push(1).unwrap();
    let buffers =
        BufferList::from_iter([Buffer::Write(b"third"), Buffer::Write(b"hole")].into_iter())
            .unwrap();
    let driver =
        unsafe { PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null()) };
    let rejected = TransferGroup::begin_selected_prepared(
        &mut slots,
        buffers,
        driver,
        Budget::new(&Rc::new(Cell::new(false))),
        selection,
    );
    assert!(rejected.is_err());
    drop(rejected);
    assert!(events.borrow().is_empty());
    assert_eq!((a.prepares, b.prepares, c.prepares), (0, 0, 0));
    let mut selection = FixedSlice::new(2).unwrap();
    selection.push(2).unwrap();
    selection.push(0).unwrap();
    let buffers =
        BufferList::from_iter([Buffer::Write(b"third"), Buffer::Write(b"first")].into_iter())
            .unwrap();
    let driver =
        unsafe { PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null()) };
    let mut group = TransferGroup::begin_selected_prepared(
        &mut slots,
        buffers,
        driver,
        Budget::new(&Rc::new(Cell::new(false))),
        selection,
    )
    .unwrap();
    let first = group.next().unwrap();
    let second = group.next().unwrap();
    assert_eq!(
        (first.index, first.key, second.index, second.key),
        (0, 30, 1, 10)
    );
    assert!(matches!(
        first.outcome.transfer,
        Some(AdapterTerminal::Accepted(2))
    ));
    assert!(matches!(
        second.outcome.transfer,
        Some(AdapterTerminal::Accepted(2))
    ));
    assert_eq!((a.prepares, b.prepares, c.prepares), (1, 0, 1));
    assert!(events
        .borrow()
        .iter()
        .any(|event| matches!(event, Event::Submit(2, address, ..) if *address == original)));
    drop(group);
    assert_eq!(slots.raw(2) as usize, original);
}

#[test]
fn later_preparation_error_aborts_before_submission_cleans_earlier_success_and_allows_reuse() {
    let descriptor = error_descriptor();
    let events = ledger();
    let behavior = Rc::new(RefCell::new(Behavior::default()));
    behavior.borrow_mut().ready.fill(true);
    let drops = Rc::new(Cell::new(0));
    let (error, original_error) = owned_error(&descriptor, &drops, Some(-3));
    let mut a = Context::new(0, &events);
    let mut b = Context::new(1, &events);
    let mut c = Context::new(2, &events);
    b.preparation_error = Some(error);
    let mut states = records(&mut [&mut a, &mut b, &mut c]);
    let mut slots = RequestSlots::new(&[10, 20, 30], |_| NativeRequest::new()).unwrap();
    let addresses = [
        slots.raw(0) as usize,
        slots.raw(1) as usize,
        slots.raw(2) as usize,
    ];
    let buffers = BufferList::from_iter(
        [
            Buffer::Write(b"first"),
            Buffer::Write(b"failed"),
            Buffer::Write(b"untouched"),
        ]
        .into_iter(),
    )
    .unwrap();
    let driver =
        unsafe { PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null()) };
    let failure = TransferGroup::begin_prepared(
        &mut slots,
        buffers,
        driver,
        Budget::new(&Rc::new(Cell::new(false))),
    )
    .err()
    .expect("later checked preparation must fail begin");
    let SetupFailure::Preparation(AdapterTerminal::CheckedError { error, progress }) = failure
    else {
        panic!("begin must own the original later preparation error")
    };
    assert_eq!(progress, None);
    let carrier = error.into_carrier();
    assert_eq!(carrier.object, original_error);
    assert_eq!(carrier.vtable, &descriptor as *const _);
    assert_eq!(
        unsafe { (*carrier.object.cast::<ErrorPayload>()).progress },
        Some(-3)
    );
    assert_eq!(drops.get(), 0);
    assert_eq!((a.prepares, b.prepares, c.prepares), (1, 1, 0));
    assert_eq!((a.completes, b.completes, c.completes), (1, 0, 0));
    let snapshot = events.borrow();
    assert!(!snapshot.iter().any(|event| matches!(
        event,
        Event::Submit(..) | Event::Cancel(..) | Event::Retire(..)
    )));
    assert!(snapshot.iter().any(|event| matches!(event, Event::Complete {
        index: 0, tag: 4, submitted: 0, immediate: None, value: 0, reason: 0, code: None, progress: None,
        selected: (leaf, _, 5), ..
    } if *leaf == endpoint(0) as usize)));
    assert!(!snapshot
        .iter()
        .any(|event| matches!(event, Event::Complete { index: 1 | 2, .. })));
    assert!(
        position(&snapshot, |event| matches!(event, Event::Prepare(1, ..)))
            < position(&snapshot, |event| matches!(
                event,
                Event::Complete { index: 0, .. }
            ))
    );
    assert!(snapshot
        .iter()
        .any(|event| matches!(event, Event::Endpoint(0, request, leaf)
        if *request == addresses[0] && *leaf == endpoint(0) as usize)));
    drop(snapshot);
    drop(unsafe { OwnedIoError::from_carrier(carrier) });
    assert_eq!(drops.get(), 1);
    assert!(matches!(unsafe { &(*states.raw(0)).phase }, Phase::Taken));
    assert!(matches!(unsafe { &(*states.raw(1)).phase }, Phase::Taken));
    assert!(matches!(unsafe { &(*states.raw(2)).phase }, Phase::Idle));
    let buffers = BufferList::from_iter(
        [
            Buffer::Write(b"reuse-a"),
            Buffer::Write(b"reuse-b"),
            Buffer::Write(b"reuse-c"),
        ]
        .into_iter(),
    )
    .unwrap();
    let driver =
        unsafe { PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null()) };
    let mut reused = TransferGroup::begin_prepared(
        &mut slots,
        buffers,
        driver,
        Budget::new(&Rc::new(Cell::new(false))),
    )
    .unwrap();
    for key in [10, 20, 30] {
        let completion = reused.next().unwrap();
        assert_eq!(completion.key, key);
        assert!(matches!(
            completion.outcome.transfer,
            Some(AdapterTerminal::Accepted(2))
        ));
    }
    drop(reused);
    assert_eq!(
        [
            slots.raw(0) as usize,
            slots.raw(1) as usize,
            slots.raw(2) as usize
        ],
        addresses
    );
    assert_eq!((a.completes, b.completes, c.completes), (2, 1, 1));
    assert_eq!(drops.get(), 1);
}

#[test]
fn cancellation_between_preparations_cleans_each_success_without_native_submission() {
    let events = ledger();
    let behavior = Rc::new(RefCell::new(Behavior::default()));
    let cancelled = Rc::new(Cell::new(false));
    let mut a = Context::new(0, &events);
    let mut b = Context::new(1, &events);
    let mut c = Context::new(2, &events);
    b.cancel_on_prepare = Some(Rc::clone(&cancelled));
    let mut states = records(&mut [&mut a, &mut b, &mut c]);
    let mut slots = RequestSlots::new(&[10, 20, 30], |_| NativeRequest::new()).unwrap();
    let addresses = [
        slots.raw(0) as usize,
        slots.raw(1) as usize,
        slots.raw(2) as usize,
    ];
    let buffers = BufferList::from_iter(
        [
            Buffer::Write(b"first"),
            Buffer::Write(b"second"),
            Buffer::Write(b"later"),
        ]
        .into_iter(),
    )
    .unwrap();
    let driver =
        unsafe { PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null()) };
    let mut group =
        TransferGroup::begin_prepared(&mut slots, buffers, driver, Budget::new(&cancelled))
            .unwrap();
    for _ in 0..3 {
        let completion = group.next().unwrap();
        assert!(completion.outcome.transfer.is_none());
        assert_eq!(completion.outcome.stop, Some(Stop::Cancellation));
        assert_eq!(completion.outcome.cancellation_failure, None);
    }
    assert!(group.next().is_none());
    assert_eq!((a.prepares, b.prepares, c.prepares), (1, 1, 0));
    assert_eq!((a.completes, b.completes, c.completes), (1, 1, 0));
    let snapshot = events.borrow();
    assert!(!snapshot.iter().any(|event| matches!(
        event,
        Event::Submit(..) | Event::Cancel(..) | Event::Retire(..)
    )));
    for (index, address) in addresses.iter().enumerate().take(2) {
        assert_eq!(snapshot.iter().filter(|event| matches!(event, Event::Complete { index: actual, .. } if *actual == index)).count(), 1);
        assert!(snapshot.iter().any(|event| matches!(event, Event::Complete {
            index: actual, tag: 4, submitted: 0, immediate: None, value: 0, reason: 0, code: None, progress: None,
            selected: (leaf, data, length), ..
        } if *actual == index && *leaf == endpoint(index) as usize && *data != 0 && *length == 5 + index)));
        assert!(snapshot.iter().any(
            |event| matches!(event, Event::Endpoint(actual, request, leaf)
            if *actual == index && *request == *address && *leaf == endpoint(index) as usize)
        ));
    }
    assert!(!snapshot.iter().any(|event| matches!(
        event,
        Event::Prepare(2, ..) | Event::Complete { index: 2, .. }
    )));
    drop(snapshot);
    drop(group);
    for index in 0..2 {
        assert!(unsafe { (*states.raw(index)).caller_data.is_null() });
        assert!(unsafe { (*states.raw(index)).native_region.data.is_null() });
        assert!(unsafe { (*states.raw(index)).native_region.endpoint.is_null() });
    }
}

#[test]
fn unsubmitted_cleanup_checked_error_is_preserved_instead_of_no_transfer() {
    let descriptor = error_descriptor();
    let events = ledger();
    let behavior = Rc::new(RefCell::new(Behavior::default()));
    let drops = Rc::new(Cell::new(0));
    let (error, original) = owned_error(&descriptor, &drops, Some(-3));
    let cancelled = Rc::new(Cell::new(false));
    let mut context = Context::new(0, &events);
    context.cancel_on_prepare = Some(Rc::clone(&cancelled));
    context.completion_error = Some(error);
    let mut states = records(&mut [&mut context]);
    let mut slots = RequestSlots::new(&[10], |_| NativeRequest::new()).unwrap();
    let buffers = BufferList::from_iter([Buffer::Write(b"loan")].into_iter()).unwrap();
    let driver =
        unsafe { PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null()) };
    let mut group =
        TransferGroup::begin_prepared(&mut slots, buffers, driver, Budget::new(&cancelled))
            .unwrap();
    let completion = group.next().unwrap();
    assert_eq!(completion.outcome.stop, Some(Stop::Cancellation));
    let Some(AdapterTerminal::CheckedError { error, progress }) = completion.outcome.transfer
    else {
        panic!("successful preparation's cleanup error must not be erased")
    };
    assert_eq!(progress, None);
    let carrier = error.into_carrier();
    assert_eq!(carrier.object, original);
    assert_eq!(carrier.vtable, &descriptor as *const _);
    assert_eq!(
        unsafe { (*carrier.object.cast::<ErrorPayload>()).progress },
        Some(-3)
    );
    assert_eq!(context.completes, 1);
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        Event::Complete {
            tag: 4,
            submitted: 0,
            immediate: None,
            value: 0,
            reason: 0,
            code: None,
            progress: None,
            ..
        }
    )));
    assert!(!events
        .borrow()
        .iter()
        .any(|event| matches!(event, Event::Submit(..) | Event::Retire(..))));
    drop(unsafe { OwnedIoError::from_carrier(carrier) });
    drop(group);
    assert_eq!(drops.get(), 1);
}

#[test]
fn post_prepare_resource_failure_preserves_original_checked_cleanup_error() {
    let descriptor = error_descriptor();
    let events = ledger();
    let behavior = Rc::new(RefCell::new(Behavior::default()));
    behavior.borrow_mut().resource_failure = Some(0);
    let drops = Rc::new(Cell::new(0));
    let (error, original) = owned_error(&descriptor, &drops, Some(-3));
    let cancelled = Rc::new(Cell::new(false));
    let mut context = Context::new(0, &events);
    context.completion_error = Some(error);
    let mut states = records(&mut [&mut context]);
    let mut slots = RequestSlots::new(&[10], |_| NativeRequest::new()).unwrap();
    let buffers = BufferList::from_iter([Buffer::Write(b"loan")].into_iter()).unwrap();
    let driver =
        unsafe { PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null()) };
    let error =
        match TransferGroup::begin_prepared(&mut slots, buffers, driver, Budget::new(&cancelled)) {
            Err(SetupFailure::Preparation(AdapterTerminal::CheckedError { error, progress })) => {
                assert_eq!(progress, None);
                error
            }
            _ => panic!("resource rejection erased successful preparation's checked cleanup error"),
        };
    assert_eq!(error.carrier().object, original);
    assert_eq!(error.carrier().vtable, &descriptor as *const _);
    assert_eq!(
        unsafe { (*error.carrier().object.cast::<ErrorPayload>()).progress },
        Some(-3)
    );
    assert_eq!((context.prepares, context.completes), (1, 1));
    assert_eq!(drops.get(), 0);
    assert!(events.borrow().iter().any(|event| matches!(
        event,
        Event::Complete {
            tag: 4,
            submitted: 0,
            immediate: None,
            ..
        }
    )));
    assert!(!events.borrow().iter().any(|event| matches!(
        event,
        Event::Submit(..) | Event::Cancel(..) | Event::Retire(..)
    )));
    assert!(unsafe { (*states.raw(0)).caller_data.is_null() });
    assert!(unsafe { (*states.raw(0)).native_region.endpoint.is_null() });
    drop(error);
    assert_eq!(drops.get(), 1);
}

#[test]
fn request_missing_repeated_and_mixed_configuration_reject_before_io_and_clean_once() {
    for mode in 1..=4 {
        let events = ledger();
        let behavior = Rc::new(RefCell::new(Behavior::default()));
        let mut context = Context::new(0, &events);
        context.configuration_test = mode;
        if mode == 4 {
            context.immediate = Some((0, 0));
        }
        let mut states = records(&mut [&mut context]);
        let mut slots = RequestSlots::new(&[10], |_| NativeRequest::new()).unwrap();
        let buffers = BufferList::from_iter([Buffer::Write(b"loan")].into_iter()).unwrap();
        let driver = unsafe {
            PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null())
        };
        let result = TransferGroup::begin_prepared(
            &mut slots,
            buffers,
            driver,
            Budget::new(&Rc::new(Cell::new(false))),
        );
        assert!(
            matches!(result, Err(SetupFailure::Preparation(AdapterTerminal::PlatformFailure(failure)))
            if failure == Failure::invalid_input())
        );
        assert_eq!((context.prepares, context.completes), (1, 1));
        let snapshot = events.borrow();
        assert!(!snapshot.iter().any(|event| matches!(
            event,
            Event::Submit(..) | Event::Cancel(..) | Event::Retire(..)
        )));
        let completed = snapshot
            .iter()
            .find(|event| matches!(event, Event::Complete { .. }))
            .unwrap();
        if mode == 4 {
            assert!(matches!(
                completed,
                Event::Complete {
                    submitted: 0,
                    immediate: Some((0, 0)),
                    ..
                }
            ));
        } else if mode != 1 {
            assert!(
                matches!(completed, Event::Complete { tag: 4, selected: (leaf, _, 4), .. }
                if *leaf == endpoint(0) as usize)
            );
        }
    }
}

#[test]
fn nullable_completion_cannot_silently_discard_an_actual_native_or_immediate_result() {
    for immediate in [false, true] {
        let events = ledger();
        let behavior = Rc::new(RefCell::new(Behavior::default()));
        behavior.borrow_mut().specs[0].submission = NativeSubmission::Complete;
        let mut context = Context::new(0, &events);
        context.null_completion = true;
        if immediate {
            context.immediate = Some((0, 0));
        }
        let mut states = records(&mut [&mut context]);
        let mut slots = RequestSlots::new(&[10], |_| NativeRequest::new()).unwrap();
        let buffers = BufferList::from_iter([Buffer::Write(b"loan")].into_iter()).unwrap();
        let driver = unsafe {
            PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null())
        };
        let mut group = TransferGroup::begin_prepared(
            &mut slots,
            buffers,
            driver,
            Budget::new(&Rc::new(Cell::new(false))),
        )
        .unwrap();
        assert!(matches!(group.next().unwrap().outcome.transfer,
            Some(AdapterTerminal::PlatformFailure(failure)) if failure == Failure::invalid_input()));
        assert!(group.next().is_none());
        drop(group);
        assert_eq!((context.prepares, context.completes), (1, 1));
    }
}

struct RequestObservation {
    result: u8,
    tag: u64,
    count: usize,
    message: *mut crate::DrStringV1,
    path: *mut crate::DrStringV1,
    code: i64,
    transferred: usize,
    meta: u64,
}

impl Drop for RequestObservation {
    fn drop(&mut self) {
        unsafe {
            crate::dr_v1_string_release(self.message);
            crate::dr_v1_string_release(self.path);
        }
    }
}

fn completion_view(
    record: &mut PendingState,
    tag: u8,
    submitted: bool,
    native: &DrIoStatusV1,
    immediate: Option<&DrAdapterOutcomeV1>,
) -> DrPendingRequestViewV1 {
    record.phase = Phase::Finalizing(CompletionFacts::new(
        submitted,
        Some((tag, *native)),
        immediate,
    ));
    unsafe { DrPendingRequestViewV1::completion(record) }
}

unsafe fn observe_request(view: &DrPendingRequestViewV1) -> RequestObservation {
    let mut observed = RequestObservation {
        result: u8::MAX,
        tag: u64::MAX,
        count: usize::MAX,
        message: ptr::null_mut(),
        path: ptr::null_mut(),
        code: i64::MIN,
        transferred: usize::MAX,
        meta: u64::MAX,
    };
    observed.result = dr_v1_checked_pending_request_outcome(
        ptr::null(),
        ptr::from_ref(view).cast(),
        &mut observed.tag,
        &mut observed.count,
        &mut observed.message,
        &mut observed.path,
        &mut observed.code,
        &mut observed.transferred,
        &mut observed.meta,
    );
    observed
}

#[test]
fn request_observation_keeps_absence_zero_eof_would_block_and_immediate_distinct_without_allocating(
) {
    let mut context = Context::new(0, &ledger());
    let mut record = PendingState::new(context.adapter()).unwrap();
    record.reading = true;
    for tag in [0, 1, 2, 4] {
        let status = DrIoStatusV1::default();
        let view = completion_view(&mut record, tag, tag != 4, &status, None);
        let (observed, allocations) =
            crate::probe_runtime_allocations(|| unsafe { observe_request(&view) });
        assert_eq!(allocations, 0);
        assert_eq!(observed.result, if tag == 4 { 4 } else { 0 });
        assert_eq!(observed.tag, if tag == 4 { 0 } else { u64::from(tag) });
        assert_eq!(observed.count, 0);
        assert!(observed.message.is_null() && observed.path.is_null());
    }
    for tag in [0, 1, 2, 3] {
        let status = DrIoStatusV1::default();
        let immediate = DrAdapterOutcomeV1 {
            tag,
            count: 0,
            has_count: u8::from(tag == 0),
        };
        let view = completion_view(&mut record, 0, false, &status, Some(&immediate));
        let (observed, allocations) =
            crate::probe_runtime_allocations(|| unsafe { observe_request(&view) });
        assert_eq!(allocations, 0);
        assert_eq!(
            (observed.result, observed.tag, observed.count),
            (0, u64::from(tag), 0)
        );
    }
}

#[test]
fn request_view_snapshots_completion_facts_without_retaining_native_output_slots() {
    let mut context = Context::new(0, &ledger());
    let mut record = PendingState::new(context.adapter()).unwrap();
    record.reading = true;
    record.bound = 8;
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        let view = {
            let mut native = DrIoStatusV1 {
                value: 3,
                ..DrIoStatusV1::default()
            };
            let view = completion_view(&mut record, 0, true, &native, None);
            native.value = 99;
            core::hint::black_box(native);
            view
        };
        let observed = observe_request(&core::hint::black_box(view));
        assert_eq!((observed.result, observed.tag, observed.count), (0, 0, 3));

        let view = {
            let native = DrIoStatusV1::default();
            let mut immediate = DrAdapterOutcomeV1 {
                tag: 0,
                has_count: 1,
                count: 0,
            };
            let view = completion_view(&mut record, 4, false, &native, Some(&immediate));
            immediate.tag = 1;
            core::hint::black_box(immediate);
            view
        };
        let observed = observe_request(&core::hint::black_box(view));
        assert_eq!((observed.result, observed.tag, observed.count), (0, 0, 0));

        let failure = Failure::from_system_code(0).with_transferred_byte_count(Some(3));
        let mut view = {
            let mut native = DrIoStatusV1::default();
            native.fail(failure);
            let view = completion_view(&mut record, 3, true, &native, None);
            native.fail(Failure::from_system_code(99));
            core::hint::black_box(native);
            view
        };
        let (_, _, native, _, _) = pending_callback_facts(ptr::from_mut(&mut view).cast());
        let Phase::Finalizing(facts) = &record.phase else {
            panic!("lost record-owned completion facts")
        };
        assert_eq!(native, ptr::from_ref(&facts.native.1));
        assert_eq!((*native).failure(), Some(failure));
    });
    assert_eq!(allocations, 0);
    assert_eq!((context.prepares, context.completes), (0, 0));
}

#[test]
fn preparation_reply_uses_shared_configuration_without_invoking_a_native_callback() {
    let mut context = Context::new(0, &ledger());
    let mut record = PendingState::new(context.adapter()).unwrap();
    let mut bytes = [0; 4];
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        for reading in [true, false] {
            record.reading = reading;
            record.bound = bytes.len();
            record.phase = Phase::Preparing;
            for tag in 0..=if reading { 3 } else { 1 } {
                record.configuration = None;
                let mut view = DrPendingRequestViewV1::preparation(&mut record);
                dr_v1_pending_request_complete(
                    ptr::from_mut(&mut view).cast(),
                    tag,
                    if tag == 0 { 3 } else { 0 },
                );
                let Prepared::Immediate(outcome) = view.prepared_reply(
                    0,
                    DrAdapterOutcomeV1::default(),
                    crate::empty_error_carrier(),
                ) else {
                    panic!("lost immediate preparation")
                };
                assert_eq!(
                    (outcome.tag, outcome.count),
                    (tag, if tag == 0 { 3 } else { 0 })
                );
            }
            record.configuration = None;
            let mut view = DrPendingRequestViewV1::preparation(&mut record);
            assert!(matches!(
                view.prepared_reply(
                    0,
                    DrAdapterOutcomeV1::default(),
                    crate::empty_error_carrier()
                ),
                Prepared::Invalid
            ));
            dr_v1_pending_request_configure_native(
                ptr::from_mut(&mut view).cast(),
                endpoint(0),
                bytes.as_mut_ptr(),
                bytes.len(),
            );
            let Prepared::Native(region) = view.prepared_reply(
                0,
                DrAdapterOutcomeV1::default(),
                crate::empty_error_carrier(),
            ) else {
                panic!("lost selected native storage")
            };
            assert_eq!(region.endpoint, endpoint(0));
            assert_eq!(region.data, bytes.as_mut_ptr());
            assert_eq!(region.length, bytes.len());
        }
    });
    assert_eq!(allocations, 0);
    assert_eq!((context.prepares, context.completes), (0, 0));
}

#[test]
fn ordinary_preparation_reply_uses_native_validation_without_callback_dispatch() {
    for reading in [true, false] {
        let events = ledger();
        let behavior = Rc::new(RefCell::new(Behavior::default()));
        let mut context = Context::new(0, &events);
        let mut states = records(&mut [&mut context]);
        let mut request = NativeRequest::new();
        let mut bytes = [0; 4];
        let caller = bytes.as_mut_ptr();
        let mut buffer = if reading {
            Buffer::Read(&mut bytes)
        } else {
            Buffer::Write(&bytes)
        };
        let mut driver = unsafe {
            PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null())
        };
        let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
            driver.reset_request(0, &mut request);
            assert!(driver
                .begin_preparation(0, &mut request, &mut buffer)
                .is_ok());
            let prepared = {
                let mut view = DrPendingRequestViewV1::preparation(driver.record(0));
                dr_v1_pending_request_configure_native(
                    ptr::from_mut(&mut view).cast(),
                    endpoint(0),
                    caller,
                    4,
                );
                view.prepared_reply(
                    0,
                    DrAdapterOutcomeV1::default(),
                    crate::empty_error_carrier(),
                )
            };
            assert!(matches!(
                driver.accept_preparation(0, &mut request, prepared),
                Preparation::Ready
            ));
            assert!(!request.live);
            let record = &*driver.record(0);
            assert_eq!(
                (record.caller_data, record.native_region.data),
                (caller, caller)
            );
            assert_eq!((record.bound, record.native_bound), (4, 4));
            assert!(record.configurator.is_none());
            assert!(record.validate_idle_source_update().is_err());
            driver.retire_drain(0, &mut request);
            assert!(!events
                .borrow()
                .iter()
                .any(|event| matches!(event, Event::ReleaseSource(_))));
            let terminal = {
                let view = DrPendingRequestViewV1::completion(driver.record(0));
                assert_eq!(view.native_tag, 4);
                view.completed_reply(
                    4,
                    DrAdapterOutcomeV1::default(),
                    crate::empty_error_carrier(),
                )
            };
            driver.accept_completion(0, &mut request, terminal);
            assert!(matches!(
                driver.drain(0, &mut request),
                Some(AdapterTerminal::NoTransfer)
            ));
            assert!((*driver.record(0)).validate_idle_source_update().is_ok());
        });
        assert_eq!(allocations, 0);
        assert_eq!((context.prepares, context.completes), (0, 0));
        let events = events.borrow();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::Endpoint(..)))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::ReleaseSource(0)))
                .count(),
            1
        );
        assert!(!events.iter().any(|event| matches!(
            event,
            Event::Submit(..) | Event::Query(_) | Event::Retire(_)
        )));
    }
}

#[test]
fn rejected_ordinary_preparation_retains_first_selection_until_original_cleanup_error() {
    for mode in 0..4 {
        let events = ledger();
        let behavior = Rc::new(RefCell::new(Behavior::default()));
        let mut context = Context::new(0, &events);
        let mut states = records(&mut [&mut context]);
        let mut request = NativeRequest::new();
        let mut bytes = [0; 4];
        let caller = bytes.as_mut_ptr();
        let mut buffer = Buffer::Read(&mut bytes);
        let descriptor = error_descriptor();
        let drops = Rc::new(Cell::new(0));
        let (error, original) = owned_error(&descriptor, &drops, Some(3));
        let carrier = error.into_carrier();
        let mut driver = unsafe {
            PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null())
        };
        let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
            driver.reset_request(0, &mut request);
            assert!(driver
                .begin_preparation(0, &mut request, &mut buffer)
                .is_ok());
            let prepared = {
                let mut view = DrPendingRequestViewV1::preparation(driver.record(0));
                match mode {
                    0 => {} // Missing configuration still owns one cleanup.
                    1 | 2 => dr_v1_pending_request_configure_native(
                        ptr::from_mut(&mut view).cast(),
                        if mode == 1 { endpoint(1) } else { endpoint(0) },
                        if mode == 2 { ptr::null_mut() } else { caller },
                        4,
                    ),
                    3 => {
                        dr_v1_pending_request_complete(ptr::from_mut(&mut view).cast(), 0, 0);
                        dr_v1_pending_request_complete(ptr::from_mut(&mut view).cast(), 0, 3);
                    }
                    _ => unreachable!(),
                }
                view.prepared_reply(
                    0,
                    DrAdapterOutcomeV1::default(),
                    crate::empty_error_carrier(),
                )
            };
            let Preparation::Completion(failure) =
                driver.accept_preparation(0, &mut request, prepared)
            else {
                panic!("normal preparation return lost its cleanup")
            };
            assert_eq!(failure, Failure::invalid_input());
            assert!(!request.live);
            driver.set_call_frame(ptr::null());
            for _ in 0..3 {
                driver.retire_drain(0, &mut request);
                let record = &*driver.record(0);
                assert_eq!(record.caller_data, caller);
                assert!(record.configurator.is_none());
                assert!(record.validate_idle_source_update().is_err());
                let Phase::Finalizing(facts) = &record.phase else {
                    panic!("validation failure discarded staging before cleanup")
                };
                assert!(!facts.submitted);
                assert_eq!(
                    facts.immediate.map(|value| (value.tag, value.count)),
                    (mode == 3).then_some((0, 0))
                );
            }
            assert!(!events
                .borrow()
                .iter()
                .any(|event| matches!(event, Event::ReleaseSource(_))));
            let terminal = {
                let view = DrPendingRequestViewV1::completion(driver.record(0));
                view.completed_reply(
                    1,
                    DrAdapterOutcomeV1 {
                        count: 3,
                        has_count: 1,
                        tag: 0,
                    },
                    carrier,
                )
            };
            driver.accept_completion(0, &mut request, terminal);
            let AdapterTerminal::CheckedError { error, progress } =
                driver.preparation_failure(0, failure)
            else {
                panic!("validation failure replaced the original cleanup Error")
            };
            assert_eq!(progress, Some(3));
            assert_eq!(error.carrier().object, original);
            assert_eq!(error.carrier().vtable, &descriptor as *const _);
            assert_eq!((*original.cast::<ErrorPayload>()).system_code, 91);
            assert_eq!(drops.get(), 0);
            assert!((*driver.record(0)).validate_idle_source_update().is_ok());
            drop(error);
        });
        assert_eq!(allocations, 0);
        assert_eq!(drops.get(), 1);
        assert_eq!((context.prepares, context.completes), (0, 0));
        let events = events.borrow();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::ReleaseSource(0)))
                .count(),
            1
        );
        assert!(!events.iter().any(|event| matches!(
            event,
            Event::Submit(..) | Event::Query(_) | Event::Retire(_)
        )));
    }
}

#[test]
fn original_host_cleanup_error_wins_preparation_validation_without_native_carrier() {
    use crate::transfer_group::adapter::host_error::{
        dr_v1_host_adapter_error_acknowledge, dr_v1_host_adapter_error_claim,
        dr_v1_host_adapter_error_disposition, dr_v1_host_adapter_error_initialize, HostErrorSlot,
    };
    use core::mem::MaybeUninit;

    let events = ledger();
    let behavior = Rc::new(RefCell::new(Behavior::default()));
    let mut context = Context::new(0, &events);
    let mut states = records(&mut [&mut context]);
    let mut request = NativeRequest::new();
    let mut bytes = [0; 4];
    let mut buffer = Buffer::Read(&mut bytes);
    let mut slot = MaybeUninit::<HostErrorSlot>::uninit();
    let mut terminal = MaybeUninit::<AdapterTerminal>::uninit();
    let mut driver =
        unsafe { PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null()) };
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        let slot_pointer = slot.as_mut_ptr().cast();
        dr_v1_host_adapter_error_initialize(slot_pointer, 11);
        driver.reset_request(0, &mut request);
        assert!(driver
            .begin_preparation(0, &mut request, &mut buffer)
            .is_ok());
        let Preparation::Completion(failure) =
            driver.accept_preparation(0, &mut request, Prepared::Invalid)
        else {
            panic!("normal preparation return must own one cleanup")
        };
        assert_eq!(failure, Failure::invalid_input());
        assert!(!request.live);
        assert_eq!(
            dr_v1_host_adapter_error_claim(slot_pointer, terminal.as_mut_ptr().cast(), 1, 9, 4),
            0
        );
        driver.accept_completion(0, &mut request, terminal.assume_init_read());
        let failure = driver.preparation_failure(0, failure);
        assert!(failure.is_checked_error());
        assert_eq!(failure.progress(), None);
        let AdapterTerminal::HostError { error, .. } = failure else {
            panic!("validation replaced the original host Error claim")
        };
        assert_eq!(dr_v1_host_adapter_error_disposition(slot_pointer), 1);
        assert_eq!(error.into_identity(), 11);
        assert_eq!(dr_v1_host_adapter_error_disposition(slot_pointer), 3);
        assert_eq!(dr_v1_host_adapter_error_acknowledge(slot_pointer), 0);
        assert!((*driver.record(0)).validate_idle_source_update().is_ok());
    });
    assert_eq!(allocations, 0);
    assert_eq!((context.prepares, context.completes), (0, 0));
    assert_eq!(
        events
            .borrow()
            .iter()
            .filter(|event| matches!(event, Event::ReleaseSource(0)))
            .count(),
        1
    );
    assert!(!events.borrow().iter().any(|event| matches!(
        event,
        Event::Submit(..) | Event::Query(_) | Event::Retire(_)
    )));
}

#[test]
fn throwing_ordinary_preparation_preserves_original_error_and_never_completes() {
    let events = ledger();
    let behavior = Rc::new(RefCell::new(Behavior::default()));
    let mut context = Context::new(0, &events);
    let mut states = records(&mut [&mut context]);
    let mut request = NativeRequest::new();
    let mut bytes = [0; 4];
    let mut buffer = Buffer::Read(&mut bytes);
    let descriptor = error_descriptor();
    let drops = Rc::new(Cell::new(0));
    let (error, original) = owned_error(&descriptor, &drops, Some(9));
    let carrier = error.into_carrier();
    let mut driver =
        unsafe { PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null()) };
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        driver.reset_request(0, &mut request);
        assert!(driver
            .begin_preparation(0, &mut request, &mut buffer)
            .is_ok());
        let prepared = {
            let view = DrPendingRequestViewV1::preparation(driver.record(0));
            view.prepared_reply(
                1,
                DrAdapterOutcomeV1 {
                    count: 9,
                    has_count: 1,
                    tag: 0,
                },
                carrier,
            )
        };
        let Preparation::Failed(AdapterTerminal::CheckedError { error, progress }) =
            driver.accept_preparation(0, &mut request, prepared)
        else {
            panic!("throwing preparation lost its original Error")
        };
        assert_eq!(progress, None);
        assert_eq!(error.carrier().object, original);
        assert_eq!((*original.cast::<ErrorPayload>()).progress, Some(9));
        assert_eq!(drops.get(), 0);
        let record = &*driver.record(0);
        assert!(matches!(record.phase, Phase::Taken));
        assert!(record.caller_data.is_null());
        assert!(record.configuration.is_none());
        assert!(record.validate_idle_source_update().is_ok());
        drop(error);
    });
    assert_eq!(allocations, 0);
    assert_eq!(drops.get(), 1);
    assert_eq!((context.prepares, context.completes), (0, 0));
    let events = events.borrow();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, Event::ReleaseSource(0)))
            .count(),
        1
    );
    assert!(!events.iter().any(|event| matches!(
        event,
        Event::Endpoint(..)
            | Event::Submit(..)
            | Event::Query(_)
            | Event::Retire(_)
            | Event::Complete { .. }
    )));
}

#[test]
fn retired_query_retains_loans_until_host_reply_without_requerying_or_allocating() {
    let events = ledger();
    let behavior = Rc::new(RefCell::new(Behavior::default()));
    let mut context = Context::new(0, &events);
    context.selection = Selection::Storage;
    let mut states = records(&mut [&mut context]);
    let mut request = NativeRequest::new();
    let mut bytes = [0; 6];
    let caller = bytes.as_mut_ptr();
    let selected = context.storage.as_mut_ptr();
    let mut buffer = Buffer::Read(&mut bytes);
    let mut driver =
        unsafe { PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null()) };
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        driver.reset_request(0, &mut request);
        assert!(driver.prepare_request(0, &mut request, &mut buffer).is_ok());
        assert!(matches!(
            driver.submit(0, &mut request, &mut buffer),
            Submission::Pending
        ));
        assert!(matches!(
            driver.retire_query(0, &mut request),
            Query::Pending
        ));
        assert!(request.live);
        behavior.borrow_mut().ready[0] = true;
        assert!(matches!(
            driver.retire_query(0, &mut request),
            Query::Terminal(())
        ));
        assert!(!request.live);

        // Resume in a different call without keeping a callback frame or view.
        driver.set_call_frame(ptr::null());
        for _ in 0..3 {
            assert!(matches!(
                driver.retire_query(0, &mut request),
                Query::Terminal(())
            ));
            driver.retire_drain(0, &mut request);
            assert!(driver.cancel(0, &mut request).is_ok());
            let record = &mut *driver.record(0);
            assert_eq!(record.caller_data, caller);
            assert_eq!(record.native_region.data, selected);
            assert_eq!((record.bound, record.native_bound), (6, 4));
            assert!(record.validate_idle_source_update().is_err());
            let view = DrPendingRequestViewV1::completion(record);
            let observed = observe_request(&view);
            assert_eq!((observed.result, observed.tag, observed.count), (0, 0, 2));
        }
        assert!(!events.borrow().iter().any(|event| matches!(
            event,
            Event::Complete { .. } | Event::ReleaseSource(_) | Event::Cancel(_)
        )));
        // A host invokes its ordinary method outside C and decodes its reply
        // through the same request protocol as the native callback.
        let terminal = {
            let view = DrPendingRequestViewV1::completion(driver.record(0));
            view.completed_reply(
                0,
                DrAdapterOutcomeV1 {
                    tag: 0,
                    count: 3,
                    has_count: 1,
                },
                crate::empty_error_carrier(),
            )
        };
        driver.accept_completion(0, &mut request, terminal);
        assert_eq!((*driver.record(0)).caller_data, ptr::null_mut());
        assert!((*driver.record(0)).configuration.is_none());
        assert!(matches!(
            driver.query(0, &mut request),
            Query::Terminal(AdapterTerminal::Data(3))
        ));
        assert!((*driver.record(0)).validate_idle_source_update().is_ok());
    });
    assert_eq!(allocations, 0);
    let snapshot = events.borrow();
    assert_eq!(
        snapshot
            .iter()
            .filter(|event| matches!(event, Event::Query(0)))
            .count(),
        2
    );
    assert_eq!(
        snapshot
            .iter()
            .filter(|event| matches!(event, Event::Retire(0)))
            .count(),
        1
    );
    assert_eq!(
        snapshot
            .iter()
            .filter(|event| matches!(event, Event::ReleaseSource(0)))
            .count(),
        1
    );
    assert_eq!((context.prepares, context.completes), (1, 0));
}

#[test]
fn retired_drain_accepts_owned_host_error_once_after_all_cancellation_without_allocating() {
    let descriptor = error_descriptor();
    let drops = Rc::new(Cell::new(0));
    let (error, original) = owned_error(&descriptor, &drops, Some(3));
    let events = ledger();
    let behavior = Rc::new(RefCell::new(Behavior::default()));
    let mut a = Context::new(0, &events);
    let mut b = Context::new(1, &events);
    let mut states = records(&mut [&mut a, &mut b]);
    let mut requests = [NativeRequest::new(), NativeRequest::new()];
    let mut driver =
        unsafe { PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null()) };
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        for (index, request) in requests.iter_mut().enumerate() {
            let mut buffer = Buffer::Write(b"loan");
            driver.reset_request(index, request);
            assert!(driver.prepare_request(index, request, &mut buffer).is_ok());
            assert!(matches!(
                driver.submit(index, request, &mut buffer),
                Submission::Pending
            ));
        }
        for (index, request) in requests.iter_mut().enumerate() {
            assert!(driver.cancel(index, request).is_ok());
        }
        for (index, request) in requests.iter_mut().enumerate() {
            driver.retire_drain(index, request);
            driver.retire_drain(index, request);
            assert!(!request.live);
        }
        assert!(!events
            .borrow()
            .iter()
            .any(|event| matches!(event, Event::Complete { .. } | Event::ReleaseSource(_))));
        let terminal = {
            let view = DrPendingRequestViewV1::completion(driver.record(0));
            view.completed_reply(
                1,
                DrAdapterOutcomeV1 {
                    tag: 0,
                    count: 3,
                    has_count: 1,
                },
                error.into_carrier(),
            )
        };
        driver.accept_completion(0, &mut requests[0], terminal);
        assert_eq!(drops.get(), 0);
        let Some(AdapterTerminal::CheckedError { error, progress }) =
            driver.drain(0, &mut requests[0])
        else {
            panic!("lost original host error")
        };
        assert_eq!(error.carrier().object, original);
        assert_eq!(error.carrier().vtable, ptr::from_ref(&descriptor));
        assert_eq!(progress, Some(3));
        drop(error);
        assert_eq!(drops.get(), 1);
        // The other adapter may still use its pinned source and selected bytes.
        assert!(matches!((*driver.record(1)).phase, Phase::Finalizing(_)));
        driver.complete_retired(1, &mut requests[1]);
        assert!(matches!(
            driver.drain(1, &mut requests[1]),
            Some(AdapterTerminal::Accepted(2))
        ));
    });
    assert_eq!(allocations, 0);
    let snapshot = events.borrow();
    for index in [0, 1] {
        assert_eq!(
            snapshot
                .iter()
                .filter(|event| matches!(event, Event::Retire(actual) if *actual == index))
                .count(),
            1
        );
        assert_eq!(
            snapshot
                .iter()
                .filter(|event| matches!(event, Event::ReleaseSource(actual) if *actual == index))
                .count(),
            1
        );
        let retirement = position(&snapshot, |event| *event == Event::Retire(index));
        for cancelled in [0, 1] {
            assert!(position(&snapshot, |event| *event == Event::Cancel(cancelled)) < retirement);
        }
    }
    assert_eq!((a.completes, b.completes), (0, 1));
}

#[test]
fn retired_unsubmitted_cleanup_keeps_absence_and_actual_zero_distinct_without_allocating() {
    for reading in [true, false] {
        for immediate in [None, Some((0, 0)), Some((1, 0))] {
            let events = ledger();
            let behavior = Rc::new(RefCell::new(Behavior::default()));
            let mut context = Context::new(0, &events);
            context.immediate = immediate;
            let mut states = records(&mut [&mut context]);
            let mut request = NativeRequest::new();
            let mut bytes = [0; 4];
            let mut buffer = if reading {
                Buffer::Read(&mut bytes)
            } else {
                Buffer::Write(&bytes)
            };
            let mut driver = unsafe {
                PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null())
            };
            let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
                driver.reset_request(0, &mut request);
                assert!(driver.prepare_request(0, &mut request, &mut buffer).is_ok());
                driver.retire_drain(0, &mut request);
                driver.retire_drain(0, &mut request);
                assert!(!request.live);
                let terminal = {
                    let view = DrPendingRequestViewV1::completion(driver.record(0));
                    assert_eq!(view.submitted, 0);
                    let observed = observe_request(&view);
                    assert_eq!(observed.result, if immediate.is_some() { 0 } else { 4 });
                    assert_eq!(observed.count, 0);
                    view.completed_reply(
                        if immediate.is_some() { 0 } else { 4 },
                        DrAdapterOutcomeV1 {
                            tag: immediate.map_or(0, |outcome| outcome.0),
                            count: 0,
                            has_count: 1,
                        },
                        crate::empty_error_carrier(),
                    )
                };
                driver.accept_completion(0, &mut request, terminal);
                let terminal = driver.drain(0, &mut request).unwrap();
                assert!(matches!(
                    (reading, immediate, terminal),
                    (_, None, AdapterTerminal::NoTransfer)
                        | (true, Some((0, 0)), AdapterTerminal::Data(0))
                        | (false, Some((0, 0)), AdapterTerminal::Accepted(0))
                        | (_, Some((1, 0)), AdapterTerminal::WouldBlock)
                ));
            });
            assert_eq!(allocations, 0);
            let snapshot = events.borrow();
            assert!(!snapshot.iter().any(|event| matches!(
                event,
                Event::Submit(..) | Event::Query(_) | Event::Retire(_) | Event::Complete { .. }
            )));
            assert_eq!(
                snapshot
                    .iter()
                    .filter(|event| matches!(event, Event::ReleaseSource(0)))
                    .count(),
                1
            );
            assert_eq!((context.prepares, context.completes), (1, 0));
        }
    }
}

#[test]
fn retired_selected_storage_stays_exclusive_until_completion_returns() {
    let events = ledger();
    let behavior = Rc::new(RefCell::new(Behavior::default()));
    behavior.borrow_mut().ready[0] = true;
    let mut a = Context::new(0, &events);
    let mut b = Context::new(1, &events);
    b.selection = Selection::Storage;
    let mut states = records(&mut [&mut a, &mut b]);
    let mut requests = [NativeRequest::new(), NativeRequest::new()];
    let mut second_bytes = [0; 4];
    let mut driver =
        unsafe { PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null()) };
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        {
            let mut buffer = Buffer::Read(&mut b.storage[..4]);
            driver.reset_request(0, &mut requests[0]);
            assert!(driver
                .prepare_request(0, &mut requests[0], &mut buffer)
                .is_ok());
            assert!(matches!(
                driver.submit(0, &mut requests[0], &mut buffer),
                Submission::Pending
            ));
            assert!(matches!(
                driver.retire_query(0, &mut requests[0]),
                Query::Terminal(())
            ));
        }
        let mut buffer = Buffer::Read(&mut second_bytes);
        driver.reset_request(1, &mut requests[1]);
        assert!(
            matches!(driver.prepare_request(1, &mut requests[1], &mut buffer),
                Err(AdapterTerminal::PlatformFailure(failure)) if failure == Failure::invalid_input()
            )
        );
        assert!(matches!((*driver.record(0)).phase, Phase::Finalizing(_)));
        driver.complete_retired(0, &mut requests[0]);
        assert!(matches!(
            driver.query(0, &mut requests[0]),
            Query::Terminal(AdapterTerminal::Data(2))
        ));
        // After the first complete adapter returns, the same storage is usable.
        driver.reset_request(1, &mut requests[1]);
        assert!(driver
            .prepare_request(1, &mut requests[1], &mut buffer)
            .is_ok());
        assert!(matches!(
            driver.drain(1, &mut requests[1]),
            Some(AdapterTerminal::NoTransfer)
        ));
    });
    assert_eq!(allocations, 0);
    assert_eq!((a.completes, b.prepares, b.completes), (1, 2, 2));
    assert_eq!(
        events
            .borrow()
            .iter()
            .filter(|event| matches!(event, Event::Retire(0)))
            .count(),
        1
    );
    assert!(!events
        .borrow()
        .iter()
        .any(|event| matches!(event, Event::Submit(1, ..))));
}

#[test]
fn completion_reply_keeps_absence_separate_from_native_and_immediate_results() {
    let mut context = Context::new(0, &ledger());
    let mut record = PendingState::new(context.adapter()).unwrap();
    record.bound = 4;
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        for reading in [true, false] {
            record.reading = reading;
            let native = DrIoStatusV1::default();
            let view = completion_view(&mut record, 4, false, &native, None);
            for status in [0, 4] {
                assert!(matches!(
                    view.completed_reply(
                        status,
                        DrAdapterOutcomeV1 {
                            tag: u8::MAX,
                            count: usize::MAX,
                            has_count: u8::MAX
                        },
                        crate::empty_error_carrier(),
                    ),
                    AdapterTerminal::NoTransfer
                ));
            }
            for tag in 0..=if reading { 3 } else { 1 } {
                let actual = DrAdapterOutcomeV1 {
                    tag,
                    count: 0,
                    has_count: u8::from(tag == 0),
                };
                for native_tag in [0, 1, 2, 3, 4] {
                    let immediate = (native_tag == 4).then_some(&actual);
                    let view = completion_view(
                        &mut record,
                        native_tag,
                        native_tag != 4,
                        &native,
                        immediate,
                    );
                    assert!(matches!(
                        view.completed_reply(4, DrAdapterOutcomeV1::default(), crate::empty_error_carrier()),
                        AdapterTerminal::PlatformFailure(failure) if failure == Failure::invalid_input()
                    ));
                    let terminal = view.completed_reply(0, actual, crate::empty_error_carrier());
                    assert!(matches!(
                        (reading, tag, terminal),
                        (true, 0, AdapterTerminal::Data(0))
                            | (false, 0, AdapterTerminal::Accepted(0))
                            | (_, 1, AdapterTerminal::WouldBlock)
                            | (true, 2, AdapterTerminal::EndOfStream)
                            | (true, 3, AdapterTerminal::TimedOut)
                    ));
                }
            }
        }
    });
    assert_eq!(allocations, 0);
    assert_eq!((context.prepares, context.completes), (0, 0));
}

#[test]
fn request_replies_move_original_checked_errors_once_with_bounded_prefixes() {
    let descriptor = error_descriptor();
    let drops = Rc::new(Cell::new(0));
    let mut context = Context::new(0, &ledger());
    let mut record = PendingState::new(context.adapter()).unwrap();
    record.bound = 4;
    for preparation in [true, false] {
        for reading in [true, false] {
            record.reading = reading;
            for count in [0, 3, 8] {
                let (error, original) = owned_error(&descriptor, &drops, Some(count));
                let carrier = error.into_carrier();
                let previous_drops = drops.get();
                let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
                    let outcome = DrAdapterOutcomeV1 {
                        tag: 0,
                        count: count as usize,
                        has_count: 1,
                    };
                    let terminal = if preparation {
                        record.phase = Phase::Preparing;
                        let view = DrPendingRequestViewV1::preparation(&mut record);
                        let Prepared::Failed(terminal) = view.prepared_reply(1, outcome, carrier)
                        else {
                            panic!("lost preparation error")
                        };
                        terminal
                    } else {
                        let view =
                            completion_view(&mut record, 4, false, &DrIoStatusV1::default(), None);
                        view.completed_reply(1, outcome, carrier)
                    };
                    let AdapterTerminal::CheckedError { error, progress } = terminal else {
                        panic!("lost original checked error")
                    };
                    assert_eq!(progress, (count <= 4).then_some(count as usize));
                    assert_eq!(error.carrier().object, original);
                    assert_eq!(error.carrier().vtable, &descriptor as *const _);
                    assert_eq!((*original.cast::<ErrorPayload>()).progress, Some(count));
                    assert_eq!(drops.get(), previous_drops);
                    drop(error);
                });
                assert_eq!(allocations, 0);
                assert_eq!(drops.get(), previous_drops + 1);
            }
        }
    }
    assert_eq!(drops.get(), 12);
    assert_eq!((context.prepares, context.completes), (0, 0));
}

#[test]
fn checked_request_observation_preserves_actual_path_unknown_zero_and_partial_failure_facts() {
    unsafe {
        let path = crate::dr_v1_string_from_utf8(b"actual-origin.bin".as_ptr(), 17);
        let mut context = Context::new(0, &ledger());
        let mut record = PendingState::new(context.adapter()).unwrap();
        record.reading = true;
        record.origin = Some(PhysicalOrigin::File {
            file: ptr::dangling_mut(),
            origin: FileOrigin {
                payload: ptr::dangling_mut(),
                path,
                resource: ptr::null_mut(),
            },
        });
        for code in [None, Some(0), Some(13)] {
            for progress in [None, Some(0), Some(3)] {
                let failure = Failure {
                    reason: crate::platform_io::Reason::Other,
                    system_code: code,
                    transferred_byte_count: progress,
                };
                let mut status = DrIoStatusV1::default();
                assert_eq!(status.fail(failure), 3);
                let view = completion_view(&mut record, 3, false, &status, None);
                let observed = observe_request(&view);
                assert_eq!(observed.result, 1);
                assert_eq!(observed.path, path);
                assert_eq!(observed.code, code.unwrap_or(0));
                assert_eq!(observed.transferred, progress.unwrap_or(0));
                assert_eq!((observed.meta >> 32) & 1, u64::from(code.is_some()));
                assert_eq!((observed.meta >> 34) & 1, u64::from(progress.is_some()));
                assert_eq!((observed.meta >> 16) & 0xff, 7);
            }
        }
        record.phase = Phase::Preparing;
        let view = DrPendingRequestViewV1::preparation(&mut record);
        let observed = observe_request(&view);
        assert_eq!(observed.result, 1);
        assert_eq!(observed.path, path);
        assert_eq!((observed.meta >> 16) & 0xff, 2);
        assert_eq!(observed.meta & 0xff, 1);
        drop(observed);
        crate::dr_v1_string_release(path);
    }
}

#[test]
fn standard_request_origin_preserves_device_target_without_a_file_owner_or_path() {
    use crate::device_io::{standard_device, StandardStream};
    unsafe {
        for stream in [
            StandardStream::Stdin,
            StandardStream::Stdout,
            StandardStream::Stderr,
        ] {
            let mut context = Context::new(0, &ledger());
            let (mut record, origin) = PendingState::for_source(
                context.adapter(),
                standard_device(stream).source().value(),
            )
            .unwrap();
            assert!(origin.file().is_none());
            record.reading = stream == StandardStream::Stdin;
            for progress in [None, Some(0), Some(3)] {
                let failure = Failure::from_system_code(0).with_transferred_byte_count(progress);
                let mut status = DrIoStatusV1::default();
                status.fail(failure);
                let view = completion_view(&mut record, 3, false, &status, None);
                let observed = observe_request(&view);
                assert_eq!(observed.result, 1);
                assert!(observed.path.is_null());
                assert_eq!((observed.meta >> 8) & 0xff, stream as u64 + 1);
                assert_eq!(
                    (observed.meta >> 32) & 1,
                    1,
                    "system code zero remains present"
                );
                assert_eq!((observed.meta >> 34) & 1, u64::from(progress.is_some()));
                assert_eq!(observed.transferred, progress.unwrap_or(0));
            }
            record.bound = 3;
            record.phase = Phase::Preparing;
            let mut view = DrPendingRequestViewV1::preparation(&mut record);
            dr_v1_pending_request_configure_standard(
                ptr::from_mut(&mut view).cast(),
                stream as u8,
                ptr::dangling_mut(),
                3,
            );
            assert!(!record.invalid_configuration);
            assert!(
                matches!(record.configuration, Some(Configuration::Native(region))
                if region.endpoint == ptr::from_ref(standard_device(stream)).cast())
            );
        }
    }
}

#[test]
fn request_view_layout_has_only_borrowed_words_and_configuration_never_overwrites_first_selection()
{
    assert_eq!(
        mem::size_of::<DrPendingRequestViewV1>(),
        4 * mem::size_of::<usize>()
    );
    let mut context = Context::new(0, &ledger());
    let mut record = PendingState::new(context.adapter()).unwrap();
    record.bound = 4;
    record.phase = Phase::Preparing;
    let mut view = DrPendingRequestViewV1::preparation(&mut record);
    unsafe {
        dr_v1_pending_request_complete(ptr::from_mut(&mut view).cast(), 0, 0);
        dr_v1_pending_request_complete(ptr::from_mut(&mut view).cast(), 1, 0);
    }
    assert!(record.invalid_configuration);
    assert!(matches!(
        record.configuration,
        Some(Configuration::Immediate(DrAdapterOutcomeV1 {
            tag: 0,
            count: 0,
            ..
        }))
    ));
}

#[test]
fn external_source_update_validation_rejects_every_live_phase_without_mutating_origin() {
    let mut context = Context::new(0, &ledger());
    let mut record = PendingState::new(context.adapter()).unwrap();
    let original = PhysicalOrigin::Standard(crate::device_io::StandardStream::Stdin);
    record.origin = Some(original);
    for phase in [
        Phase::Preparing,
        Phase::Prepared(DrPendingRegionV1::default()),
        Phase::Native,
        Phase::Finalizing(CompletionFacts::new(false, None, None)),
        Phase::Immediate(DrAdapterOutcomeV1::default()),
        Phase::Ready(AdapterTerminal::Data(0)),
    ] {
        record.phase = phase;
        assert!(matches!(record.validate_idle_source_update(), Err(failure)
            if failure.reason == crate::platform_io::Reason::InvalidInput));
        assert!(record.origin.unwrap().same_source(original));
    }
    assert_eq!((context.prepares, context.completes), (0, 0));
}

#[test]
fn external_source_update_commit_is_infallible_after_idle_or_taken_validation() {
    let mut context = Context::new(0, &ledger());
    let mut record = PendingState::new(context.adapter()).unwrap();
    let replacement = PhysicalOrigin::Standard(crate::device_io::StandardStream::Stdout);
    for phase in [Phase::Idle, Phase::Taken] {
        record.phase = phase;
        record.validate_idle_source_update().unwrap();
        unsafe {
            record.set_idle_origin(replacement);
        }
        assert!(record.origin.unwrap().same_source(replacement));
    }
    assert_eq!((context.prepares, context.completes), (0, 0));
}

#[test]
fn file_request_configuration_borrows_actual_payload_origin_accepts_empty_and_rejects_foreign_identity(
) {
    use crate::file_io::stream::transport::object::{
        dr_v1_file_object_close, dr_v1_file_object_drop, dr_v1_file_object_layout,
        dr_v1_file_object_open, dr_v1_file_object_pending_endpoint,
        dr_v1_file_object_readiness_source, dr_v1_file_object_readiness_source_home,
    };
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_file(&self.0).unwrap();
        }
    }
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "doria-request-origin-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
    )));
    std::fs::write(&fixture.0, b"kept").unwrap();
    unsafe {
        let path_bytes = fixture.0.to_str().unwrap().as_bytes();
        let path = crate::dr_v1_string_from_utf8(path_bytes.as_ptr(), path_bytes.len());
        let (mut size, mut align) = (0, 0);
        dr_v1_file_object_layout(&mut size, &mut align);
        let payload = crate::dr_v2_class_allocate(ptr::null(), size, align).cast();
        let mut status = DrIoStatusV1::default();
        assert_eq!(
            dr_v1_file_object_open(path, 0, 0, 0, payload, &mut status),
            0
        );
        {
            let mut context = Context::new(0, &ledger());
            let references = (*path).references;
            let ((mut record, owner), allocations) = crate::probe_runtime_allocations(|| {
                let home = dr_v1_file_object_readiness_source_home(payload);
                let forwarded = *home.cast::<*const c_void>();
                assert_eq!(forwarded, dr_v1_file_object_readiness_source(payload));
                let (direct, direct_owner) =
                    PendingState::for_file_object(context.adapter(), payload).unwrap();
                let (forwarded, physical) =
                    PendingState::for_source(context.adapter(), forwarded).unwrap();
                let owner = physical.file().unwrap().0;
                assert_eq!(direct_owner, owner);
                assert_eq!(
                    direct.origin.unwrap().file().unwrap().1.payload,
                    forwarded.origin.unwrap().file().unwrap().1.payload
                );
                (forwarded, owner)
            });
            assert_eq!(allocations, 0);
            assert_eq!((*path).references, references);
            assert_eq!(
                owner.cast::<c_void>(),
                dr_v1_file_object_pending_endpoint(payload)
            );
            assert_eq!(record.origin.unwrap().file().unwrap().1.path, path);
            record.bound = 4;
            record.phase = Phase::Preparing;
            // Registration owns the origin BEFORE a callback selects a file,
            // including a callback that observes the request in the wrong phase.
            for reading in [true, false] {
                record.reading = reading;
                let view = DrPendingRequestViewV1::preparation(&mut record);
                let observed = observe_request(&view);
                assert_eq!(observed.result, 1);
                assert_eq!(observed.path, path);
                assert_eq!(observed.meta & 0xff, if reading { 1 } else { 2 });
                assert_eq!((observed.meta >> 8) & 0xff, 0);
                assert_eq!((observed.meta >> 16) & 0xff, 2);
                assert_eq!((observed.meta >> 32) & 1, 0);
                assert_eq!((observed.meta >> 34) & 1, 0);
                assert!(record.configuration.is_none());
                assert!(!record.invalid_configuration);
                assert_eq!(record.origin.unwrap().file().unwrap().1.payload, payload);
            }
            record.reading = true;
            let mut view = DrPendingRequestViewV1::preparation(&mut record);
            let (_, allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_pending_request_configure_file(
                    ptr::from_mut(&mut view).cast(),
                    payload,
                    ptr::null_mut(),
                    0,
                );
            });
            assert_eq!(allocations, 0);
            assert!(!record.invalid_configuration);
            assert!(
                matches!(record.configuration, Some(Configuration::Native(DrPendingRegionV1 { endpoint, data, length: 0 }))
                if endpoint == owner.cast_const().cast() && data.is_null())
            );
            let observed = observe_request(&view);
            assert_eq!(observed.result, 1);
            assert_eq!(observed.path, path);
            assert_eq!((observed.meta >> 16) & 0xff, 2);
        }
        {
            let mut context = Context::new(0, &ledger());
            let mut raw = PendingState::new(context.adapter()).unwrap();
            raw.phase = Phase::Preparing;
            let mut view = DrPendingRequestViewV1::preparation(&mut raw);
            // Raw fixture admission cannot be upgraded into a public checked
            // source by later selection, even of an actual valid file.
            dr_v1_pending_request_configure_file(
                ptr::from_mut(&mut view).cast(),
                payload,
                ptr::null_mut(),
                0,
            );
            assert!(raw.invalid_configuration);
            assert!(raw.configuration.is_none() && raw.origin.is_none());
        }
        {
            let mut context = Context::new(0, &ledger());
            let owner = dr_v1_file_object_pending_endpoint(payload)
                .cast::<crate::file_io::stream::OwnedFile>();
            let mut projection = mem::MaybeUninit::<ReadinessProjection>::uninit();
            // Deliberately coincident complete-owner/first-field addresses.
            // This is valid raw-owner provenance, never a FileObject layout
            // promise. Rejection must precede reading the raw owner's bytes
            // as a FileObject resource pointer.
            ReadinessProjection::initialize_file(
                projection.as_mut_ptr(),
                owner.cast(),
                owner,
                path,
            );
            let source = projection.assume_init_ref().value();
            let (mut raw, physical) = PendingState::for_source(context.adapter(), source).unwrap();
            let (_, origin) = physical.file().unwrap();
            assert_eq!(origin.payload, owner.cast());
            assert!(origin.resource.is_null());
            raw.phase = Phase::Preparing;
            let mut view = DrPendingRequestViewV1::preparation(&mut raw);
            let (_, allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_pending_request_configure_file(
                    ptr::from_mut(&mut view).cast(),
                    owner.cast(),
                    ptr::null_mut(),
                    0,
                );
            });
            assert_eq!(allocations, 0);
            assert!(raw.invalid_configuration);
            assert!(raw.configuration.is_none());
            assert_eq!(raw.origin.unwrap().file().unwrap().1.payload, owner.cast());
        }
        {
            let mut context = Context::new(0, &ledger());
            let (mut record, _) =
                PendingState::for_file_object(context.adapter(), payload).unwrap();
            record.phase = Phase::Preparing;
            let mut view = DrPendingRequestViewV1::preparation(&mut record);
            dr_v1_pending_request_configure_file(
                ptr::from_mut(&mut view).cast(),
                ptr::dangling_mut(),
                ptr::null_mut(),
                0,
            );
            assert!(record.invalid_configuration);
            assert!(record.configuration.is_none());
            assert_eq!(record.origin.unwrap().file().unwrap().1.payload, payload);
            let observed = observe_request(&view);
            assert_eq!(observed.result, 1);
            assert_eq!(observed.path, path);
            assert_eq!((observed.meta >> 16) & 0xff, 2);
        }
        assert_eq!(dr_v1_file_object_close(payload, &mut status), 0);
        dr_v1_file_object_drop(ptr::null(), payload);
        crate::dr_v1_class_free(payload.cast());
        crate::dr_v1_string_release(path);
    }
}
