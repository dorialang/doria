//! Scheduler facts belong to each successful preparation, not admission or a
//! mutable registration. This model exercises the existing group state machine.
use super::*;
use std::cell::Cell;

struct HandoffState {
    source_blocked: [bool; 3],
    preparation_blocked: [bool; 3],
    completion_blocked: [bool; 3],
    terminal: [Option<Option<Result<usize, Failure>>>; 3],
    queries: [usize; 3],
    completions: [usize; 3],
    cancellation_error: Option<(usize, Failure)>,
    source_replies: [usize; 3],
    resets: [usize; 3],
    aborts: [usize; 3],
    homes: [Option<(usize, usize, usize, bool)>; 3],
    fail_source: Option<usize>,
    frame: *const crate::DrStackFrameV2,
}

impl HandoffState {
    fn new() -> Self {
        Self {
            source_blocked: [false; 3],
            preparation_blocked: [false; 3],
            completion_blocked: [false; 3],
            terminal: [None; 3],
            queries: [0; 3],
            completions: [0; 3],
            cancellation_error: None,
            source_replies: [0; 3],
            resets: [0; 3],
            aborts: [0; 3],
            homes: [None; 3],
            fail_source: None,
            frame: core::ptr::null(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreparedEvent {
    Prepare(usize),
    Resource(usize, usize, bool),
    Submit(usize),
    Cancel(usize),
    Drain(usize),
    Retire(usize),
    Complete(usize),
}

struct PreparedState {
    next: [TransferResource; 3],
    prepared: [bool; 3],
    native: [bool; 3],
    ready: [bool; 3],
    fail_prepare: Option<usize>,
    fail_resource: Option<usize>,
    change_sibling: Option<TransferResource>,
    stop_on_prepare: Option<usize>,
    stopped: Rc<Cell<bool>>,
    events: Vec<PreparedEvent>,
    handoff: Option<HandoffState>,
}

impl PreparedState {
    fn new() -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            next: core::array::from_fn(|identity| TransferResource {
                identity,
                shared_cursor: false,
            }),
            prepared: [false; 3],
            native: [false; 3],
            ready: [false; 3],
            fail_prepare: None,
            fail_resource: None,
            change_sibling: None,
            stop_on_prepare: None,
            stopped: Rc::new(Cell::new(false)),
            events: Vec::with_capacity(128),
            handoff: None,
        }))
    }

    fn submissions(&self) -> Vec<usize> {
        self.events
            .iter()
            .filter_map(|event| {
                if let PreparedEvent::Submit(index) = event {
                    Some(*index)
                } else {
                    None
                }
            })
            .collect()
    }
}

struct PreparedRequest {
    selected: Option<TransferResource>,
}

struct PreparedDriver(Rc<RefCell<PreparedState>>);

impl PreparedDriver {
    fn poll_completion(
        &mut self,
        index: usize,
        draining: bool,
    ) -> Poll<Option<Result<usize, Failure>>> {
        let mut state = self.0.borrow_mut();
        assert!(state.prepared[index]);
        if state.handoff.as_ref().unwrap().terminal[index].is_none() {
            if !draining {
                state.handoff.as_mut().unwrap().queries[index] += 1;
                if !state.ready[index] {
                    return Poll::Ready(None);
                }
            } else {
                state.events.push(PreparedEvent::Drain(index));
            }
            let terminal = state.native[index].then_some(Ok(1));
            state.ready[index] = false;
            state.native[index] = false;
            state.handoff.as_mut().unwrap().terminal[index] = Some(terminal);
            state.events.push(PreparedEvent::Retire(index));
        }
        if state.handoff.as_ref().unwrap().completion_blocked[index] {
            return Poll::Pending;
        }
        let handoff = state.handoff.as_mut().unwrap();
        let terminal = handoff.terminal[index].take().unwrap();
        handoff.completions[index] += 1;
        state.prepared[index] = false;
        state.events.push(PreparedEvent::Complete(index));
        Poll::Ready(terminal)
    }
}

// No raw byte access in this model. Cancel keeps staging/native storage live;
// only terminal query or drain retires it. Successful prepare owns one cleanup.
unsafe impl GroupDriver for PreparedDriver {
    type Request = PreparedRequest;
    type Timeout = u32;
    const MAXIMUM_TRANSFER_BYTES: usize = usize::MAX;
    const PREPARE_BEFORE_SUBMISSION: bool = true;

    fn set_call_frame(&mut self, frame: *const crate::DrStackFrameV2) {
        if let Some(handoff) = &mut self.0.borrow_mut().handoff {
            handoff.frame = frame;
        }
    }

    fn call_frame(&self) -> *const crate::DrStackFrameV2 {
        self.0
            .borrow()
            .handoff
            .as_ref()
            .map_or(core::ptr::null(), |handoff| handoff.frame)
    }

    unsafe fn poll_setup_request(
        &mut self,
        index: usize,
        _: *mut PreparedRequest,
    ) -> Poll<Result<(), Result<usize, Failure>>> {
        let mut state = self.0.borrow_mut();
        let Some(handoff) = &mut state.handoff else {
            return Poll::Ready(Ok(()));
        };
        if handoff.source_blocked[index] {
            return Poll::Pending;
        }
        handoff.source_replies[index] += 1;
        if handoff.fail_source == Some(index) {
            Poll::Ready(Err(Err(Failure::invalid_input())))
        } else {
            Poll::Ready(Ok(()))
        }
    }

    unsafe fn resource(
        &self,
        index: usize,
        request: *mut PreparedRequest,
    ) -> Result<TransferResource, Failure> {
        let mut state = self.0.borrow_mut();
        assert!(
            state.prepared[index],
            "resource classification preceded preparation"
        );
        assert!(!state.native[index]);
        let selected = (*request).selected.expect("prepared operation snapshot");
        state.events.push(PreparedEvent::Resource(
            index,
            selected.identity,
            selected.shared_cursor,
        ));
        if state.fail_resource == Some(index) {
            Err(Failure::unsupported())
        } else {
            Ok(selected)
        }
    }

    unsafe fn reset_request(&mut self, index: usize, request: *mut PreparedRequest) {
        let mut state = self.0.borrow_mut();
        assert!(!state.prepared[index] && !state.native[index]);
        if let Some(handoff) = &mut state.handoff {
            handoff.resets[index] += 1;
        }
        request.write(PreparedRequest { selected: None });
    }

    unsafe fn poll_prepare_request(
        &mut self,
        index: usize,
        request: *mut PreparedRequest,
        buffer: &mut Buffer<'_>,
    ) -> Poll<Result<(), Result<usize, Failure>>> {
        if let Some(handoff) = &mut self.0.borrow_mut().handoff {
            let (address, length, reading) = buffer.region();
            let home = (request as usize, address, length, reading);
            if let Some(previous) = handoff.homes[index] {
                assert_eq!(home, previous, "host handoff moved request or byte loan");
            }
            handoff.homes[index] = Some(home);
            if handoff.preparation_blocked[index] {
                return Poll::Pending;
            }
            handoff.homes[index] = None;
        }
        Poll::Ready(self.prepare_request(index, request, buffer))
    }

    unsafe fn abort_preparation(&mut self, index: usize, _: *mut PreparedRequest) {
        let mut state = self.0.borrow_mut();
        assert!(!state.prepared[index] && !state.native[index]);
        let handoff = state.handoff.as_mut().unwrap();
        assert!(handoff.homes[index].take().is_some());
        handoff.aborts[index] += 1;
    }

    unsafe fn prepare_request(
        &mut self,
        index: usize,
        request: *mut PreparedRequest,
        _: &mut Buffer<'_>,
    ) -> Result<(), Result<usize, Failure>> {
        let mut state = self.0.borrow_mut();
        assert!(!state.prepared[index] && !state.native[index]);
        state.events.push(PreparedEvent::Prepare(index));
        if state.fail_prepare == Some(index) {
            return Err(Err(Failure::invalid_input()));
        }
        (*request).selected = Some(state.next[index]);
        state.prepared[index] = true;
        if index == 0 {
            if let Some(replacement) = state.change_sibling {
                state.next[1] = replacement;
            }
        }
        if state.stop_on_prepare == Some(index) {
            state.stopped.set(true);
        }
        Ok(())
    }

    unsafe fn submit(
        &mut self,
        index: usize,
        _: *mut PreparedRequest,
        _: &mut Buffer<'_>,
    ) -> Submission {
        let mut state = self.0.borrow_mut();
        assert!(state.prepared[index] && !state.native[index]);
        state.native[index] = true;
        state.events.push(PreparedEvent::Submit(index));
        Submission::Pending
    }

    unsafe fn query(&mut self, index: usize, _: *mut PreparedRequest) -> Query {
        let mut state = self.0.borrow_mut();
        assert!(state.prepared[index] && state.native[index]);
        if !state.ready[index] {
            return Query::Pending;
        }
        state.ready[index] = false;
        state.prepared[index] = false;
        state.native[index] = false;
        Query::Terminal(Ok(1))
    }

    unsafe fn poll_query(&mut self, index: usize, request: *mut PreparedRequest) -> Poll<Query> {
        if self.0.borrow().handoff.is_none() {
            return Poll::Ready(self.query(index, request));
        }
        match self.poll_completion(index, false) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(terminal)) => Poll::Ready(Query::Terminal(terminal)),
            Poll::Ready(None) => Poll::Ready(Query::Pending),
        }
    }

    fn wait_any(&mut self, _: u32, _: Option<usize>) -> Result<Wake, Failure> {
        panic!("bounded fixtures must not wait for external native work")
    }

    unsafe fn cancel(
        &mut self,
        index: usize,
        _: *mut PreparedRequest,
    ) -> Result<(), (IoMessageOperation, Failure)> {
        let mut state = self.0.borrow_mut();
        if state
            .handoff
            .as_ref()
            .is_some_and(|handoff| handoff.terminal[index].is_some())
        {
            assert!(!state.native[index]);
            return Ok(());
        }
        assert!(state.prepared[index] && state.native[index]);
        state.events.push(PreparedEvent::Cancel(index));
        if let Some((failed, failure)) = state
            .handoff
            .as_ref()
            .and_then(|handoff| handoff.cancellation_error)
        {
            if failed == index {
                return Err((IoMessageOperation::Wait, failure));
            }
        }
        Ok(())
    }

    unsafe fn drain(
        &mut self,
        index: usize,
        _: *mut PreparedRequest,
    ) -> Option<Result<usize, Failure>> {
        let mut state = self.0.borrow_mut();
        assert!(
            state.prepared[index],
            "failed/unprepared staging received cleanup"
        );
        let native = state.native[index];
        state.events.push(PreparedEvent::Drain(index));
        state.prepared[index] = false;
        state.native[index] = false;
        native.then_some(Ok(1))
    }

    unsafe fn poll_drain(
        &mut self,
        index: usize,
        request: *mut PreparedRequest,
    ) -> Poll<Option<Result<usize, Failure>>> {
        if self.0.borrow().handoff.is_none() {
            return Poll::Ready(self.drain(index, request));
        }
        self.poll_completion(index, true)
    }
}

struct PreparedControl(Rc<Cell<bool>>);

impl Control<u32> for PreparedControl {
    fn next_wait(&mut self) -> Result<Result<Wait<u32>, Stop>, Failure> {
        Ok(if self.0.get() {
            Err(Stop::Cancellation)
        } else {
            Ok(Wait {
                timeout: 100,
                immediate: false,
            })
        })
    }

    fn cancellation_signal(&self) -> Option<usize> {
        None
    }
}

fn slots(count: usize) -> RequestSlots<PreparedRequest> {
    RequestSlots::new(&[11, 22, 33][..count], |_| PreparedRequest {
        selected: None,
    })
    .unwrap()
}

fn control(state: &Rc<RefCell<PreparedState>>) -> PreparedControl {
    PreparedControl(Rc::clone(&state.borrow().stopped))
}

fn handoff_state() -> Rc<RefCell<PreparedState>> {
    let state = PreparedState::new();
    state.borrow_mut().handoff = Some(HandoffState::new());
    state
}

fn setup<'s, 'b>(
    slots: &'s mut RequestSlots<PreparedRequest>,
    buffers: BufferList<'b>,
    state: &Rc<RefCell<PreparedState>>,
) -> super::super::setup::GroupSetup<'s, 'b, PreparedDriver, PreparedControl> {
    super::super::setup::GroupSetup::configure(
        slots,
        buffers,
        PreparedDriver(Rc::clone(state)),
        control(state),
        None,
    )
    .unwrap()
}

#[test]
fn resumable_startup_preserves_homes_order_frames_and_single_preparation_without_allocating() {
    let state = handoff_state();
    {
        let mut state = state.borrow_mut();
        let handoff = state.handoff.as_mut().unwrap();
        handoff.source_blocked[1] = true;
        handoff.preparation_blocked[1] = true;
    }
    let mut slots = slots(2);
    let mut destination = [0u8; 2];
    let buffers =
        BufferList::from_iter([Buffer::Write(b"ab"), Buffer::Read(&mut destination)].into_iter())
            .unwrap();
    let frame: crate::DrStackFrameV2 = unsafe { core::mem::zeroed() };
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        let mut setup = setup(&mut slots, buffers, &state);
        assert!(unsafe { setup.poll(&frame) }.is_pending());
        assert!(state.borrow().events.is_empty());
        let mut setup = match setup.into_group() {
            Err(setup) => setup,
            Ok(_) => panic!("unfinished startup exposed a group"),
        };
        assert!(unsafe { setup.poll(&frame) }.is_pending());
        state.borrow_mut().handoff.as_mut().unwrap().source_blocked[1] = false;
        assert!(unsafe { setup.poll(&frame) }.is_pending());
        assert!(state.borrow().submissions().is_empty());
        let home = state.borrow().handoff.as_ref().unwrap().homes[1];
        // Move the guard after yielding; backing storage, not its home, is pinned.
        let mut setup = (setup, 7).0;
        assert!(unsafe { setup.poll(&frame) }.is_pending());
        assert_eq!(state.borrow().handoff.as_ref().unwrap().homes[1], home);
        assert!(state.borrow().handoff.as_ref().unwrap().frame.is_null());
        state
            .borrow_mut()
            .handoff
            .as_mut()
            .unwrap()
            .preparation_blocked[1] = false;
        assert!(matches!(unsafe { setup.poll(&frame) }, Poll::Ready(Ok(()))));
        assert!(matches!(unsafe { setup.poll(&frame) }, Poll::Ready(Ok(()))));
        let mut group = match setup.into_group() {
            Ok(group) => group,
            Err(_) => panic!("successful startup did not expose a group"),
        };
        assert_eq!(state.borrow().submissions(), [0, 1]);
        state.borrow_mut().ready = [true; 3];
        assert_eq!(group.next().unwrap().index, 0);
        assert_eq!(group.next().unwrap().index, 1);
        assert!(group.next().is_none());
        unsafe { group.drop_in_frame(&frame) };
    });
    assert_eq!(allocations, 0);
    let state = state.borrow();
    let handoff = state.handoff.as_ref().unwrap();
    assert_eq!(handoff.source_replies, [1, 1, 0]);
    assert_eq!(handoff.resets, [1, 1, 0]);
    assert_eq!(handoff.aborts, [0; 3]);
    assert_eq!(handoff.homes, [None; 3]);
    assert_eq!(
        state
            .events
            .iter()
            .filter(|e| matches!(e, PreparedEvent::Prepare(_)))
            .count(),
        2
    );
}

#[test]
fn dropping_yielded_startup_aborts_unfinished_staging_and_drains_only_successful_preparations() {
    let state = handoff_state();
    state
        .borrow_mut()
        .handoff
        .as_mut()
        .unwrap()
        .preparation_blocked[1] = true;
    let mut slots = slots(2);
    let buffers =
        BufferList::from_iter([Buffer::Write(b"a"), Buffer::Write(b"b")].into_iter()).unwrap();
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        let mut setup = setup(&mut slots, buffers, &state);
        assert!(unsafe { setup.poll(core::ptr::null()) }.is_pending());
        drop(setup);
    });
    assert_eq!(allocations, 0);
    let state = state.borrow();
    assert_eq!(state.handoff.as_ref().unwrap().aborts, [0, 1, 0]);
    assert_eq!(state.handoff.as_ref().unwrap().homes, [None; 3]);
    assert_eq!(state.prepared, [false; 3]);
    assert_eq!(
        &state.events[state.events.len() - 3..],
        &[
            PreparedEvent::Drain(0),
            PreparedEvent::Retire(0),
            PreparedEvent::Complete(0),
        ]
    );
    assert!(state.submissions().is_empty());
}

#[test]
fn cancellation_aborts_pending_preparation_but_retires_a_successful_racing_reply_once() {
    for reply_ready in [false, true] {
        let state = handoff_state();
        state
            .borrow_mut()
            .handoff
            .as_mut()
            .unwrap()
            .preparation_blocked[1] = true;
        let mut slots = slots(2);
        let buffers =
            BufferList::from_iter([Buffer::Write(b"a"), Buffer::Write(b"b")].into_iter()).unwrap();
        let ((), allocations) = crate::probe_runtime_allocations(|| {
            let mut setup = setup(&mut slots, buffers, &state);
            assert!(unsafe { setup.poll(core::ptr::null()) }.is_pending());
            state.borrow().stopped.set(true);
            state
                .borrow_mut()
                .handoff
                .as_mut()
                .unwrap()
                .preparation_blocked[1] = !reply_ready;
            assert!(matches!(
                unsafe { setup.poll(core::ptr::null()) },
                Poll::Ready(Ok(()))
            ));
            let mut group = match setup.into_group() {
                Ok(group) => group,
                Err(_) => panic!("stopped startup did not retire into a group"),
            };
            for index in 0..2 {
                let completion = group.next().unwrap();
                assert_eq!(completion.index, index);
                assert_eq!(completion.outcome.transfer, None);
                assert_eq!(completion.outcome.stop, Some(Stop::Cancellation));
            }
            assert!(group.next().is_none());
        });
        assert_eq!(allocations, 0);
        let state = state.borrow();
        assert_eq!(
            state.handoff.as_ref().unwrap().aborts[1],
            usize::from(!reply_ready)
        );
        assert_eq!(
            state
                .events
                .iter()
                .filter(|e| **e == PreparedEvent::Drain(1))
                .count(),
            usize::from(reply_ready)
        );
        assert_eq!(state.prepared, [false; 3]);
        assert!(state.submissions().is_empty());
    }
}

#[test]
fn yielded_source_or_preparation_failure_wins_racing_cancellation_without_submission() {
    for source_failure in [true, false] {
        let state = handoff_state();
        {
            let mut state = state.borrow_mut();
            if source_failure {
                let handoff = state.handoff.as_mut().unwrap();
                handoff.source_blocked[1] = true;
                handoff.fail_source = Some(1);
            } else {
                state.fail_prepare = Some(1);
                state.handoff.as_mut().unwrap().preparation_blocked[1] = true;
            }
        }
        let mut slots = slots(2);
        let buffers =
            BufferList::from_iter([Buffer::Write(b"a"), Buffer::Write(b"b")].into_iter()).unwrap();
        let ((), allocations) = crate::probe_runtime_allocations(|| {
            let mut setup = setup(&mut slots, buffers, &state);
            assert!(unsafe { setup.poll(core::ptr::null()) }.is_pending());
            state.borrow().stopped.set(true);
            {
                let mut state = state.borrow_mut();
                let handoff = state.handoff.as_mut().unwrap();
                handoff.source_blocked[1] = false;
                handoff.preparation_blocked[1] = false;
            }
            assert!(matches!(unsafe { setup.poll(core::ptr::null()) },
                Poll::Ready(Err(SetupFailure::Preparation(Err(f))))
                    if f.reason == crate::platform_io::Reason::InvalidInput));
            assert!(matches!(
                unsafe { setup.poll(core::ptr::null()) },
                Poll::Ready(Err(SetupFailure::Platform(_)))
            ));
            drop(setup);
        });
        assert_eq!(allocations, 0);
        let state = state.borrow();
        assert_eq!(state.handoff.as_ref().unwrap().aborts, [0; 3]);
        assert_eq!(state.prepared, [false; 3]);
        assert_eq!(
            state
                .events
                .iter()
                .filter(|e| **e == PreparedEvent::Drain(0))
                .count(),
            usize::from(!source_failure)
        );
        assert!(state.submissions().is_empty());
    }
}

struct FetchBudget<'a> {
    expired: &'a Cell<bool>,
    calls: &'a Cell<usize>,
    failure: Option<Failure>,
}

impl ObservationBudget<u32> for FetchBudget<'_> {
    fn next_wait(&mut self) -> Result<Option<Wait<u32>>, Failure> {
        self.calls.set(self.calls.get() + 1);
        if let Some(failure) = self.failure {
            return Err(failure);
        }
        Ok((!self.expired.get()).then_some(Wait {
            timeout: 100,
            immediate: false,
        }))
    }
}

#[test]
fn yielded_observation_keeps_loans_and_consumes_one_reply_before_fetch_expiry() {
    let state = handoff_state();
    let mut slots = slots(3);
    let mut destination = [0u8; 2];
    let buffers = BufferList::from_iter(
        [
            Buffer::Read(&mut destination),
            Buffer::Write(b"ab"),
            Buffer::Write(b"cd"),
        ]
        .into_iter(),
    )
    .unwrap();
    let expired = Cell::new(false);
    let calls = Cell::new(0);
    let frame: crate::DrStackFrameV2 = unsafe { core::mem::zeroed() };
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        let mut group = TransferGroup::begin(
            &mut slots,
            buffers,
            PreparedDriver(Rc::clone(&state)),
            control(&state),
        )
        .unwrap();
        {
            let mut state = state.borrow_mut();
            state.ready[0] = true;
            state.handoff.as_mut().unwrap().completion_blocked[0] = true;
        }
        {
            let budget = FetchBudget {
                expired: &expired,
                calls: &calls,
                failure: None,
            };
            let mut fetch = super::super::observation::ObservationCall::new(&mut group, budget);
            assert!(unsafe { fetch.poll(&frame) }.is_pending());
            assert_eq!(state.borrow().native, [false, true, true]);
            assert_eq!(state.borrow().prepared, [true; 3]);
            assert!(state.borrow().handoff.as_ref().unwrap().frame.is_null());
            let mut fetch = (fetch, 7).0;
            assert!(unsafe { fetch.poll(&frame) }.is_pending());
            assert_eq!(state.borrow().handoff.as_ref().unwrap().queries[0], 1);
            expired.set(true);
            state
                .borrow_mut()
                .handoff
                .as_mut()
                .unwrap()
                .completion_blocked[0] = false;
            let Poll::Ready(Observation::Completion(completion)) = (unsafe { fetch.poll(&frame) })
            else {
                panic!("returned completion was hidden by fetch expiry")
            };
            assert_eq!((completion.key, completion.index), (11, 0));
            assert_eq!(completion.outcome.transfer, Some(Ok(1)));
            assert_eq!(completion.outcome.stop, None);
        }
        assert_eq!(calls.get(), 1);
        assert!(state.borrow().handoff.as_ref().unwrap().frame.is_null());
        assert_eq!(group.completed(0).map(<[u8]>::len), Some(1));
        assert!(group.completed(1).is_none());
        let queries = state.borrow().handoff.as_ref().unwrap().queries;
        let mut budget = FetchBudget {
            expired: &expired,
            calls: &calls,
            failure: None,
        };
        assert!(matches!(group.observe(&mut budget), Observation::TimedOut));
        assert_eq!(state.borrow().handoff.as_ref().unwrap().queries, queries);
        assert_eq!(state.borrow().native, [false, true, true]);
        assert_eq!(group.stop(), None);
        assert!(unsafe {
            group.with_call_frame(&frame, |group| group.poll_stop_all(Stop::Cancellation))
        }
        .is_ready());
        unsafe { group.drop_in_frame(&frame) };
    });
    assert_eq!(allocations, 0);
    assert_eq!(state.borrow().handoff.as_ref().unwrap().completions, [1; 3]);
}

#[test]
fn yielded_stop_drains_once_after_cancel_fanout_and_preserves_first_failure() {
    let state = handoff_state();
    let cancellation_failure = Failure::other().with_transferred_byte_count(Some(0));
    {
        let mut state = state.borrow_mut();
        let handoff = state.handoff.as_mut().unwrap();
        handoff.completion_blocked = [true; 3];
        handoff.cancellation_error = Some((1, cancellation_failure));
    }
    let mut slots = slots(3);
    let buffers = BufferList::from_iter(
        [
            Buffer::Write(b"ab"),
            Buffer::Write(b"cd"),
            Buffer::Write(b"ef"),
        ]
        .into_iter(),
    )
    .unwrap();
    let expired = Cell::new(false);
    let calls = Cell::new(0);
    let failure = Failure::other();
    let stop = Stop::Failure(IoMessageOperation::Wait, failure);
    let frame: crate::DrStackFrameV2 = unsafe { core::mem::zeroed() };
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        let mut group = TransferGroup::begin(
            &mut slots,
            buffers,
            PreparedDriver(Rc::clone(&state)),
            control(&state),
        )
        .unwrap();
        {
            let budget = FetchBudget {
                expired: &expired,
                calls: &calls,
                failure: Some(failure),
            };
            let mut fetch = super::super::observation::ObservationCall::new(&mut group, budget);
            assert!(unsafe { fetch.poll(&frame) }.is_pending());
            assert!(unsafe { fetch.poll(&frame) }.is_pending());
            let state = state.borrow();
            let cancel_end = state
                .events
                .iter()
                .rposition(|event| matches!(event, PreparedEvent::Cancel(_)))
                .unwrap();
            let drain_start = state
                .events
                .iter()
                .position(|event| matches!(event, PreparedEvent::Drain(_)))
                .unwrap();
            assert!(cancel_end < drain_start);
            assert_eq!(state.prepared, [true; 3]);
            assert!(state.handoff.as_ref().unwrap().frame.is_null());
        }
        assert_eq!(group.stop(), Some(stop));
        state
            .borrow_mut()
            .handoff
            .as_mut()
            .unwrap()
            .completion_blocked[0] = false;
        assert!(unsafe {
            group.with_call_frame(&frame, |group| group.poll_stop_all(Stop::Cancellation))
        }
        .is_pending());
        assert_eq!(state.borrow().prepared, [false, true, true]);
        assert!(group.completed(1).is_none());
        assert_eq!(group.stop(), Some(stop));
        state
            .borrow_mut()
            .handoff
            .as_mut()
            .unwrap()
            .completion_blocked = [false; 3];
        assert!(unsafe {
            group.with_call_frame(&frame, |group| group.poll_stop_all(Stop::Cancellation))
        }
        .is_ready());
        let mut budget = FetchBudget {
            expired: &expired,
            calls: &calls,
            failure: Some(Failure::invalid_input()),
        };
        assert!(matches!(group.observe(&mut budget),
            Observation::WaitFailed(observed) if observed == failure));
        for index in 0..3 {
            let Observation::Completion(completion) = group.observe(&mut budget) else {
                panic!("wait failure lost a retained transfer")
            };
            assert_eq!(completion.index, index);
            assert_eq!(completion.outcome.transfer, Some(Ok(1)));
            assert_eq!(completion.outcome.stop, Some(stop));
            assert_eq!(
                completion.outcome.cancellation_failure,
                (index == 1).then_some((IoMessageOperation::Wait, cancellation_failure))
            );
        }
        assert!(matches!(
            group.observe(&mut budget),
            Observation::NoCompletion
        ));
        assert_eq!(calls.get(), 1);
        unsafe { group.drop_in_frame(&frame) };
    });
    assert_eq!(allocations, 0);
    let state = state.borrow();
    for index in 0..3 {
        for event in [
            PreparedEvent::Cancel(index),
            PreparedEvent::Drain(index),
            PreparedEvent::Retire(index),
            PreparedEvent::Complete(index),
        ] {
            assert_eq!(
                state.events.iter().filter(|item| **item == event).count(),
                1
            );
        }
    }
    assert_eq!(state.handoff.as_ref().unwrap().completions, [1; 3]);
    assert_eq!(state.prepared, [false; 3]);
    assert_eq!(state.native, [false; 3]);
}

#[test]
fn yielded_preparation_failure_cleanup_wins_cancellation_without_repreparing() {
    let state = handoff_state();
    {
        let mut state = state.borrow_mut();
        state.fail_resource = Some(1);
        state.handoff.as_mut().unwrap().completion_blocked[1] = true;
    }
    let mut slots = slots(2);
    let buffers =
        BufferList::from_iter([Buffer::Write(b"a"), Buffer::Write(b"b")].into_iter()).unwrap();
    let frame: crate::DrStackFrameV2 = unsafe { core::mem::zeroed() };
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        let mut setup = setup(&mut slots, buffers, &state);
        assert!(unsafe { setup.poll(&frame) }.is_pending());
        state.borrow().stopped.set(true);
        assert!(unsafe { setup.poll(&frame) }.is_pending());
        state
            .borrow_mut()
            .handoff
            .as_mut()
            .unwrap()
            .completion_blocked[1] = false;
        assert!(matches!(unsafe { setup.poll(&frame) },
            Poll::Ready(Err(SetupFailure::Platform(failure)))
                if failure == Failure::unsupported()));
        assert_eq!(state.borrow().prepared, [true, false, false]);
        state
            .borrow_mut()
            .handoff
            .as_mut()
            .unwrap()
            .completion_blocked[0] = true;
        assert!(unsafe { setup.poll_cleanup(&frame) }.is_pending());
        assert!(state.borrow().handoff.as_ref().unwrap().frame.is_null());
        state
            .borrow_mut()
            .handoff
            .as_mut()
            .unwrap()
            .completion_blocked[0] = false;
        assert!(unsafe { setup.poll_cleanup(&frame) }.is_ready());
        unsafe { setup.drop_in_frame(&frame) };
    });
    assert_eq!(allocations, 0);
    let state = state.borrow();
    let handoff = state.handoff.as_ref().unwrap();
    assert_eq!(handoff.resets, [1, 1, 0]);
    assert_eq!(handoff.completions, [1, 1, 0]);
    assert_eq!(handoff.aborts, [0; 3]);
    assert_eq!(state.prepared, [false; 3]);
    assert!(state.submissions().is_empty());
}

#[test]
fn yielded_startup_stop_resumes_dispatch_without_repeating_admission_or_cleanup() {
    let state = handoff_state();
    {
        let mut state = state.borrow_mut();
        state.stop_on_prepare = Some(0);
        state.handoff.as_mut().unwrap().completion_blocked[0] = true;
    }
    let mut slots = slots(2);
    let buffers =
        BufferList::from_iter([Buffer::Write(b"a"), Buffer::Write(b"b")].into_iter()).unwrap();
    let frame: crate::DrStackFrameV2 = unsafe { core::mem::zeroed() };
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        let mut setup = setup(&mut slots, buffers, &state);
        assert!(unsafe { setup.poll(&frame) }.is_pending());
        assert!(unsafe { setup.poll(&frame) }.is_pending());
        assert_eq!(state.borrow().prepared, [true, false, false]);
        state
            .borrow_mut()
            .handoff
            .as_mut()
            .unwrap()
            .completion_blocked[0] = false;
        assert!(matches!(unsafe { setup.poll(&frame) }, Poll::Ready(Ok(()))));
        let mut group = match setup.into_group() {
            Ok(group) => group,
            Err(_) => panic!("stopped startup did not finish cleanup"),
        };
        for index in 0..2 {
            let completion = group.next().unwrap();
            assert_eq!(completion.index, index);
            assert_eq!(completion.outcome.transfer, None);
            assert_eq!(completion.outcome.stop, Some(Stop::Cancellation));
        }
        assert!(group.next().is_none());
        unsafe { group.drop_in_frame(&frame) };
    });
    assert_eq!(allocations, 0);
    let state = state.borrow();
    assert_eq!(state.handoff.as_ref().unwrap().resets, [1, 0, 0]);
    assert_eq!(state.handoff.as_ref().unwrap().completions, [1, 0, 0]);
    assert!(state.submissions().is_empty());
    assert!(!state
        .events
        .iter()
        .any(|event| matches!(event, PreparedEvent::Cancel(_))));
}

#[test]
fn yielded_resubmission_preserves_key_request_and_range_without_repreparing() {
    let state = handoff_state();
    let mut slots = slots(1);
    let mut destination = [0u8; 4];
    let buffers = BufferList::from_iter([Buffer::Read(&mut destination)].into_iter()).unwrap();
    let frame: crate::DrStackFrameV2 = unsafe { core::mem::zeroed() };
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        let mut group = TransferGroup::begin(
            &mut slots,
            buffers,
            PreparedDriver(Rc::clone(&state)),
            control(&state),
        )
        .unwrap();
        state.borrow_mut().ready[0] = true;
        assert_eq!(group.next().unwrap().key, 11);
        state
            .borrow_mut()
            .handoff
            .as_mut()
            .unwrap()
            .preparation_blocked[0] = true;
        {
            let mut call =
                super::super::resubmission::ResubmissionCall::new(&mut group, 0, 1, 2).unwrap();
            assert!(unsafe { call.poll(&frame) }.is_pending());
            let home = state.borrow().handoff.as_ref().unwrap().homes[0].unwrap();
            assert_eq!((home.2, home.3), (2, true));
            let mut call = (call, 7).0;
            assert!(unsafe { call.poll(&frame) }.is_pending());
            assert_eq!(
                state.borrow().handoff.as_ref().unwrap().homes[0],
                Some(home)
            );
            assert!(state.borrow().handoff.as_ref().unwrap().frame.is_null());
            state
                .borrow_mut()
                .handoff
                .as_mut()
                .unwrap()
                .preparation_blocked[0] = false;
            assert!(unsafe { call.poll(&frame) }.is_ready());
            assert!(unsafe { call.poll(&frame) }.is_ready());
        }
        assert!(group.completed(0).is_none());
        assert!(group.read_storage_mut(0).is_none());
        assert_eq!(state.borrow().submissions(), [0, 0]);
        state.borrow_mut().ready[0] = true;
        let completion = group.next().unwrap();
        assert_eq!((completion.key, completion.index), (11, 0));
        assert_eq!(completion.outcome.transfer, Some(Ok(1)));
        let entry = &group.buffers.entries.as_slice()[0];
        assert_eq!((entry.offset, entry.length), (1, 2));
        assert_eq!(group.completed(0).map(<[u8]>::len), Some(1));
        assert!(group.next().is_none());
        unsafe { group.drop_in_frame(&frame) };
    });
    assert_eq!(allocations, 0);
    let state = state.borrow();
    let handoff = state.handoff.as_ref().unwrap();
    assert_eq!(handoff.resets, [2, 0, 0]);
    assert_eq!(handoff.queries, [2, 0, 0]);
    assert_eq!(handoff.completions, [2, 0, 0]);
    assert_eq!(handoff.aborts, [0; 3]);
}

#[test]
fn cancelled_yielded_resubmission_aborts_staging_and_resumes_sibling_cleanup() {
    let state = handoff_state();
    let mut slots = slots(2);
    let buffers =
        BufferList::from_iter([Buffer::Write(b"a"), Buffer::Write(b"b")].into_iter()).unwrap();
    let frame: crate::DrStackFrameV2 = unsafe { core::mem::zeroed() };
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        let mut group = TransferGroup::begin(
            &mut slots,
            buffers,
            PreparedDriver(Rc::clone(&state)),
            control(&state),
        )
        .unwrap();
        state.borrow_mut().ready[0] = true;
        assert_eq!(group.next().unwrap().key, 11);
        {
            let mut state = state.borrow_mut();
            let handoff = state.handoff.as_mut().unwrap();
            handoff.preparation_blocked[0] = true;
            handoff.completion_blocked[1] = true;
        }
        {
            let mut call =
                super::super::resubmission::ResubmissionCall::new(&mut group, 0, 0, 1).unwrap();
            assert!(unsafe { call.poll(&frame) }.is_pending());
            state.borrow().stopped.set(true);
            assert!(unsafe { call.poll(&frame) }.is_pending());
            assert_eq!(state.borrow().prepared, [false, true, false]);
            assert_eq!(state.borrow().handoff.as_ref().unwrap().aborts, [1, 0, 0]);
            assert!(state.borrow().handoff.as_ref().unwrap().frame.is_null());
            state
                .borrow_mut()
                .handoff
                .as_mut()
                .unwrap()
                .completion_blocked[1] = false;
            assert!(unsafe { call.poll(&frame) }.is_ready());
        }
        assert_eq!(group.stop(), Some(Stop::Cancellation));
        for _ in 0..2 {
            let completion = group.next().unwrap();
            assert_eq!(completion.outcome.stop, Some(Stop::Cancellation));
            assert_eq!(
                completion.outcome.transfer,
                (completion.index == 1).then_some(Ok(1))
            );
        }
        assert!(group.next().is_none());
        unsafe { group.drop_in_frame(&frame) };
    });
    assert_eq!(allocations, 0);
    let state = state.borrow();
    assert_eq!(state.submissions(), [0, 1]);
    assert_eq!(state.handoff.as_ref().unwrap().completions, [1, 1, 0]);
    assert_eq!(
        state
            .events
            .iter()
            .filter(|event| **event == PreparedEvent::Cancel(1))
            .count(),
        1
    );
    assert_eq!(
        state
            .events
            .iter()
            .filter(|event| **event == PreparedEvent::Drain(1))
            .count(),
        1
    );
}

struct RetryState {
    sources: [usize; 2],
    calls: [usize; 2],
    events: Vec<(bool, usize)>,
}

struct RetryDriver(Rc<RefCell<RetryState>>);

// Each attempt is synchronous; pending retains only an advisory retry, never
// native work. Source selection is configuration-only and may change its cursor.
unsafe impl GroupDriver for RetryDriver {
    type Request = PreparedRequest;
    type Timeout = u32;
    const MAXIMUM_TRANSFER_BYTES: usize = usize::MAX;
    const QUERY_CAN_TRANSFER: bool = true;
    const QUERY_REFRESHES_SOURCE: bool = true;
    const PREPARE_BEFORE_SUBMISSION: bool = true;

    unsafe fn resource(
        &self,
        _: usize,
        request: *mut PreparedRequest,
    ) -> Result<TransferResource, Failure> {
        Ok((*request).selected.unwrap())
    }
    unsafe fn reset_request(&mut self, _: usize, request: *mut PreparedRequest) {
        (*request).selected = None;
    }
    unsafe fn prepare_request(
        &mut self,
        index: usize,
        request: *mut PreparedRequest,
        _: &mut Buffer<'_>,
    ) -> Result<(), Result<usize, Failure>> {
        self.prepare_query(index, request)
    }
    unsafe fn prepare_query(
        &mut self,
        index: usize,
        request: *mut PreparedRequest,
    ) -> Result<(), Result<usize, Failure>> {
        (*request).selected = Some(TransferResource {
            identity: self.0.borrow().sources[index],
            shared_cursor: true,
        });
        Ok(())
    }
    unsafe fn submit(
        &mut self,
        index: usize,
        _: *mut PreparedRequest,
        _: &mut Buffer<'_>,
    ) -> Submission {
        let mut state = self.0.borrow_mut();
        state.calls[index] += 1;
        state.events.push((false, index));
        Submission::Pending
    }
    unsafe fn query(&mut self, index: usize, _: *mut PreparedRequest) -> Query {
        let mut state = self.0.borrow_mut();
        state.events.push((true, index));
        Query::Terminal(Ok(1))
    }
    fn wait_any(&mut self, _: u32, _: Option<usize>) -> Result<Wake, Failure> {
        panic!("synchronous source arbitration fixtures must complete without waiting")
    }
    unsafe fn cancel(
        &mut self,
        _: usize,
        _: *mut PreparedRequest,
    ) -> Result<(), (IoMessageOperation, Failure)> {
        Ok(())
    }
    unsafe fn drain(
        &mut self,
        _: usize,
        request: *mut PreparedRequest,
    ) -> Option<Result<usize, Failure>> {
        (*request).selected = None;
        None
    }
}

#[test]
fn synchronous_retry_source_change_requeues_behind_the_already_selected_shared_cursor() {
    let state = Rc::new(RefCell::new(RetryState {
        sources: [41, 42],
        calls: [0; 2],
        events: Vec::with_capacity(32),
    }));
    let mut slots = slots(2);
    let buffers =
        BufferList::from_iter([Buffer::Write(b"a"), Buffer::Write(b"b")].into_iter()).unwrap();
    let mut group = TransferGroup::begin_prepared(
        &mut slots,
        buffers,
        RetryDriver(Rc::clone(&state)),
        PreparedControl(Rc::new(Cell::new(false))),
    )
    .unwrap();
    assert_eq!(state.borrow().calls, [1, 1]);
    // A current-source getter now chooses the cursor already selected by op 1.
    state.borrow_mut().sources[0] = 42;
    let (first, allocations) = crate::probe_runtime_allocations(|| group.next().unwrap());
    assert_eq!((first.key, allocations), (22, 0));
    assert_eq!(
        state.borrow().events,
        [(false, 0), (false, 1), (true, 1), (false, 0)]
    );
    assert_eq!(group.next().unwrap().key, 11);
    assert_eq!(state.borrow().calls, [2, 1]);
}

#[test]
fn preparation_can_replace_sibling_source_before_scheduler_classifies_shared_cursor() {
    let state = PreparedState::new();
    {
        let mut state = state.borrow_mut();
        state.next[0] = TransferResource {
            identity: 41,
            shared_cursor: true,
        };
        state.change_sibling = Some(state.next[0]);
    }
    let mut slots = slots(2);
    let mut destination = [0u8; 1];
    let buffers =
        BufferList::from_iter([Buffer::Write(b"a"), Buffer::Read(&mut destination)].into_iter())
            .unwrap();
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        let mut group = TransferGroup::begin_prepared(
            &mut slots,
            buffers,
            PreparedDriver(Rc::clone(&state)),
            control(&state),
        )
        .unwrap();
        assert_eq!(
            state.borrow().submissions(),
            [0],
            "same file cursor serializes opposite directions"
        );
        assert_eq!(
            &state.borrow().events[..4],
            &[
                PreparedEvent::Prepare(0),
                PreparedEvent::Resource(0, 41, true),
                PreparedEvent::Prepare(1),
                PreparedEvent::Resource(1, 41, true),
            ]
        );
        state.borrow_mut().ready[0] = true;
        assert_eq!(group.next().unwrap().index, 0);
        assert_eq!(state.borrow().submissions(), [0, 1]);
        state.borrow_mut().ready[1] = true;
        assert_eq!(group.next().unwrap().index, 1);
        assert!(group.next().is_none());
    });
    assert_eq!(allocations, 0);
}

#[test]
fn every_resubmission_refreshes_identity_and_cursor_facts_without_mutating_active_sibling() {
    let state = PreparedState::new();
    let mut slots = slots(2);
    let buffers =
        BufferList::from_iter([Buffer::Write(b"a"), Buffer::Write(b"b")].into_iter()).unwrap();
    let mut group = TransferGroup::begin_prepared(
        &mut slots,
        buffers,
        PreparedDriver(Rc::clone(&state)),
        control(&state),
    )
    .unwrap();
    assert_eq!(state.borrow().submissions(), [0, 1]);
    state.borrow_mut().ready[0] = true;
    assert_eq!(group.next().unwrap().index, 0);
    // The newly selected source now competes with sibling 1's still-live source.
    state.borrow_mut().next[0] = TransferResource {
        identity: 1,
        shared_cursor: true,
    };
    let ((), allocations) = crate::probe_runtime_allocations(|| group.submit(0, 0, 1).unwrap());
    assert_eq!(allocations, 0);
    assert_eq!(state.borrow().submissions(), [0, 1]);
    assert!(state
        .borrow()
        .events
        .contains(&PreparedEvent::Resource(0, 1, true)));
    state.borrow_mut().ready[1] = true;
    assert_eq!(group.next().unwrap().index, 1);
    assert_eq!(state.borrow().submissions(), [0, 1, 0]);
    state.borrow_mut().ready[0] = true;
    assert_eq!(group.next().unwrap().index, 0);
    assert!(group.next().is_none());
}

#[test]
fn resource_classification_failure_retires_successful_staging_once_without_submission() {
    let state = PreparedState::new();
    state.borrow_mut().fail_resource = Some(1);
    let mut slots = slots(2);
    let buffers =
        BufferList::from_iter([Buffer::Write(b"a"), Buffer::Write(b"b")].into_iter()).unwrap();
    let result = TransferGroup::begin_prepared(
        &mut slots,
        buffers,
        PreparedDriver(Rc::clone(&state)),
        control(&state),
    );
    assert!(
        matches!(result, Err(SetupFailure::Platform(f)) if f.reason == crate::platform_io::Reason::Unsupported)
    );
    let state = state.borrow();
    assert!(state.submissions().is_empty());
    assert_eq!(
        state
            .events
            .iter()
            .filter(|e| matches!(e, PreparedEvent::Drain(_)))
            .copied()
            .collect::<Vec<_>>(),
        [PreparedEvent::Drain(1), PreparedEvent::Drain(0)]
    );
    assert_eq!(state.prepared, [false; 3]);
}

#[test]
fn failed_preparation_never_classifies_or_retires_the_failed_slot() {
    let state = PreparedState::new();
    state.borrow_mut().fail_prepare = Some(1);
    let mut slots = slots(2);
    let buffers =
        BufferList::from_iter([Buffer::Write(b"a"), Buffer::Write(b"b")].into_iter()).unwrap();
    let result = TransferGroup::begin_prepared(
        &mut slots,
        buffers,
        PreparedDriver(Rc::clone(&state)),
        control(&state),
    );
    assert!(
        matches!(result, Err(SetupFailure::Preparation(Err(f))) if f.reason == crate::platform_io::Reason::InvalidInput)
    );
    assert_eq!(
        state.borrow().events,
        [
            PreparedEvent::Prepare(0),
            PreparedEvent::Resource(0, 0, false),
            PreparedEvent::Prepare(1),
            PreparedEvent::Drain(0)
        ]
    );
}

#[test]
fn resubmission_resource_failure_finishes_without_native_submission_or_second_cleanup() {
    let state = PreparedState::new();
    let mut slots = slots(1);
    let buffers = BufferList::from_iter([Buffer::Write(b"a")].into_iter()).unwrap();
    let mut group = TransferGroup::begin_prepared(
        &mut slots,
        buffers,
        PreparedDriver(Rc::clone(&state)),
        control(&state),
    )
    .unwrap();
    state.borrow_mut().ready[0] = true;
    assert_eq!(group.next().unwrap().outcome.transfer, Some(Ok(1)));
    state.borrow_mut().fail_resource = Some(0);
    group.submit(0, 0, 1).unwrap();
    assert!(
        matches!(group.next().unwrap().outcome.transfer, Some(Err(f)) if f.reason == crate::platform_io::Reason::Unsupported)
    );
    drop(group);
    let state = state.borrow();
    assert_eq!(state.submissions(), [0]);
    assert_eq!(
        state
            .events
            .iter()
            .filter(|e| **e == PreparedEvent::Drain(0))
            .count(),
        1
    );
    assert_eq!(state.prepared, [false; 3]);
}

#[test]
fn stop_during_preparation_cleans_only_successful_slots_and_leaves_unprepared_facts_absent() {
    let state = PreparedState::new();
    state.borrow_mut().stop_on_prepare = Some(1);
    let mut slots = slots(3);
    let buffers = BufferList::from_iter(
        [
            Buffer::Write(b"a"),
            Buffer::Write(b"b"),
            Buffer::Write(b"c"),
        ]
        .into_iter(),
    )
    .unwrap();
    let mut group = TransferGroup::begin_prepared(
        &mut slots,
        buffers,
        PreparedDriver(Rc::clone(&state)),
        control(&state),
    )
    .unwrap();
    for _ in 0..3 {
        assert_eq!(group.next().unwrap().outcome.stop, Some(Stop::Cancellation));
    }
    assert!(group.next().is_none());
    drop(group);
    assert_eq!(
        state.borrow().events,
        [
            PreparedEvent::Prepare(0),
            PreparedEvent::Resource(0, 0, false),
            PreparedEvent::Prepare(1),
            PreparedEvent::Resource(1, 1, false),
            PreparedEvent::Drain(0),
            PreparedEvent::Drain(1),
        ]
    );
}

#[test]
fn cancellation_requests_for_all_selected_work_precede_any_actual_retirement() {
    let state = PreparedState::new();
    let mut slots = slots(2);
    let buffers =
        BufferList::from_iter([Buffer::Write(b"a"), Buffer::Write(b"b")].into_iter()).unwrap();
    let mut group = TransferGroup::begin_prepared(
        &mut slots,
        buffers,
        PreparedDriver(Rc::clone(&state)),
        control(&state),
    )
    .unwrap();
    state.borrow().stopped.set(true);
    assert_eq!(group.next().unwrap().outcome.stop, Some(Stop::Cancellation));
    drop(group);
    let state = state.borrow();
    let terminal = state
        .events
        .iter()
        .filter(|e| matches!(e, PreparedEvent::Cancel(_) | PreparedEvent::Drain(_)))
        .copied()
        .collect::<Vec<_>>();
    assert_eq!(
        terminal,
        [
            PreparedEvent::Cancel(0),
            PreparedEvent::Cancel(1),
            PreparedEvent::Drain(0),
            PreparedEvent::Drain(1)
        ]
    );
    assert_eq!(state.prepared, [false; 3]);
    assert_eq!(state.native, [false; 3]);
}
