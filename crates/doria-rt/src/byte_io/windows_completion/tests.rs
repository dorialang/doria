use std::cell::RefCell;
use std::collections::VecDeque;
use std::panic::AssertUnwindSafe;
use std::rc::Rc;

use super::*;

#[derive(Debug, PartialEq, Eq)]
enum Event {
    Submit(usize, usize, usize, bool),
    Query,
    Wait(u32, Option<usize>),
    Cancel,
    Drain,
    Terminal,
    BufferReleased,
    ResourcesReleased,
}

#[derive(Default)]
struct State {
    events: Vec<Event>,
    active: bool,
    premature_release: bool,
    request: usize,
    buffer: usize,
    expected_bytes: Vec<u8>,
}

enum SubmitStep {
    Complete,
    Pending,
    Failed(Failure),
    Unwind,
}

enum QueryStep {
    Value(Query),
    Unwind,
}

struct ScriptDriver {
    state: Rc<RefCell<State>>,
    submissions: VecDeque<SubmitStep>,
    queries: VecDeque<QueryStep>,
    wakes: VecDeque<Result<Wake, Failure>>,
    cancellations: VecDeque<Result<(), Failure>>,
    drains: VecDeque<Result<usize, Failure>>,
}

impl ScriptDriver {
    fn pending(state: &Rc<RefCell<State>>) -> Self {
        Self {
            state: Rc::clone(state),
            submissions: [SubmitStep::Pending].into(),
            queries: [QueryStep::Value(Query::Pending)].into(),
            wakes: VecDeque::new(),
            cancellations: VecDeque::new(),
            drains: VecDeque::new(),
        }
    }

    unsafe fn check_request(&self, request: *mut Overlapped) {
        let state = self.state.borrow();
        assert!(state.active);
        assert_eq!(request as usize, state.request);
        assert_eq!((*request).event as usize, 0x5151);
        // Inspect only while the fake kernel truthfully owns the request. This
        // covers both readonly source and exclusive destination loans.
        let bytes =
            core::slice::from_raw_parts(state.buffer as *const u8, state.expected_bytes.len());
        assert_eq!(bytes, state.expected_bytes);
    }

    fn terminal(&mut self) {
        let mut state = self.state.borrow_mut();
        state.active = false;
        state.events.push(Event::Terminal);
    }
}

// This fake retains pointers only while active. Every terminal fact releases
// them; drain always returns terminal and never unwinds, including cleanup.
unsafe impl Driver for ScriptDriver {
    fn event(&self) -> *mut c_void {
        0x5151_usize as *mut c_void
    }

    unsafe fn submit(&mut self, request: *mut Overlapped, buffer: &mut Buffer<'_>) -> Submission {
        let (address, bytes, reading) = match buffer {
            Buffer::Read(bytes) => (bytes.as_ptr() as usize, &bytes[..], true),
            Buffer::Write(bytes) => (bytes.as_ptr() as usize, &bytes[..], false),
        };
        {
            let mut state = self.state.borrow_mut();
            assert!(!state.active, "a prior request is still outstanding");
            assert_eq!(*(*request).internal.get(), 0);
            assert_eq!(*(*request).internal_high.get(), 0);
            assert_eq!((*request).offset, 0);
            assert_eq!((*request).offset_high, 0);
            state.active = true;
            state.request = request as usize;
            state.buffer = address;
            state.expected_bytes = bytes.to_vec();
            state.events.push(Event::Submit(
                request as usize,
                address,
                buffer.len(),
                reading,
            ));
        }
        self.check_request(request);
        match self.submissions.pop_front().expect("unexpected submission") {
            SubmitStep::Complete => Submission::Complete,
            SubmitStep::Pending => Submission::Pending,
            SubmitStep::Failed(failure) => {
                self.terminal();
                Submission::Failed(failure)
            }
            SubmitStep::Unwind => panic!("test-only unwind after acquiring request storage"),
        }
    }

    unsafe fn query(&mut self, request: *mut Overlapped) -> Query {
        self.check_request(request);
        self.state.borrow_mut().events.push(Event::Query);
        match self.queries.pop_front().expect("unexpected query") {
            QueryStep::Value(value) => {
                if matches!(value, Query::Terminal(_)) {
                    self.terminal();
                }
                value
            }
            QueryStep::Unwind => panic!("test-only query unwind"),
        }
    }

    fn wait(&mut self, timeout: u32, cancellation: Option<usize>) -> Result<Wake, Failure> {
        self.state
            .borrow_mut()
            .events
            .push(Event::Wait(timeout, cancellation));
        self.wakes.pop_front().expect("unexpected wait")
    }

    unsafe fn cancel(&mut self, request: *mut Overlapped) -> Result<(), Failure> {
        self.check_request(request);
        self.state.borrow_mut().events.push(Event::Cancel);
        self.cancellations.pop_front().unwrap_or(Ok(()))
    }

    unsafe fn drain(&mut self, request: *mut Overlapped) -> Result<usize, Failure> {
        self.check_request(request);
        self.state.borrow_mut().events.push(Event::Drain);
        let result = self.drains.pop_front().unwrap_or(Ok(0));
        self.terminal();
        result
    }
}

impl Drop for ScriptDriver {
    fn drop(&mut self) {
        let mut state = self.state.borrow_mut();
        state.premature_release |= state.active;
        state.events.push(Event::ResourcesReleased);
    }
}

enum ControlStep {
    Wait(u32),
    Stop(Stop),
    Failed(Failure),
    Unwind,
}

struct ScriptControl {
    steps: VecDeque<ControlStep>,
    calls: usize,
}

impl ScriptControl {
    fn new(steps: impl IntoIterator<Item = ControlStep>) -> Self {
        Self {
            steps: steps.into_iter().collect(),
            calls: 0,
        }
    }
}

impl Control for ScriptControl {
    fn next_wait(&mut self) -> Result<Result<Wait<u32>, Stop>, Failure> {
        self.calls += 1;
        match self.steps.pop_front().expect("unexpected budget query") {
            ControlStep::Wait(timeout) => Ok(Ok(Wait {
                timeout,
                immediate: timeout == 0,
            })),
            ControlStep::Stop(stop) => Ok(Err(stop)),
            ControlStep::Failed(failure) => Err(failure),
            ControlStep::Unwind => panic!("test-only control unwind"),
        }
    }

    fn cancellation_signal(&self) -> Option<usize> {
        Some(0x7171)
    }
}

struct BorrowedStorage {
    bytes: [u8; 8],
    state: Rc<RefCell<State>>,
}

impl BorrowedStorage {
    fn new(state: &Rc<RefCell<State>>) -> Self {
        Self {
            bytes: *b"borrowed",
            state: Rc::clone(state),
        }
    }
}

impl Drop for BorrowedStorage {
    fn drop(&mut self) {
        let mut state = self.state.borrow_mut();
        state.premature_release |= state.active;
        state.events.push(Event::BufferReleased);
    }
}

fn state() -> Rc<RefCell<State>> {
    Rc::new(RefCell::new(State::default()))
}

fn assert_finished(state: &Rc<RefCell<State>>) {
    let state = state.borrow();
    assert!(!state.active);
    assert!(!state.premature_release);
}

#[test]
fn native_request_lengths_clamp_without_fabricating_oversized_slices() {
    for (length, expected) in [
        (0, 0),
        (7, 7),
        (u32::MAX as usize, u32::MAX),
        (usize::MAX, u32::MAX),
    ] {
        assert_eq!(request_length(length), expected);
    }
    if let Some(length) = (u32::MAX as usize).checked_add(1) {
        assert_eq!(request_length(length), u32::MAX);
    }
}

#[test]
fn empty_requests_do_not_submit_or_query_control() {
    let state = state();
    let mut pipe = Pipe {
        driver: ScriptDriver::pending(&state),
    };
    let mut control = ScriptControl::new([]);
    assert_eq!(
        pipe.transfer(Buffer::Read(&mut []), &mut control),
        Awaited::complete(Ok(0))
    );
    assert_eq!(
        pipe.transfer(Buffer::Write(&[]), &mut control),
        Awaited::complete(Ok(0))
    );
    assert_eq!(control.calls, 0);
    assert!(state.borrow().events.is_empty());
}

#[test]
fn stopped_or_failed_initial_control_never_submits() {
    let failure = Failure::from_system_code(5);
    for step in [
        ControlStep::Stop(Stop::Cancellation),
        ControlStep::Stop(Stop::BudgetExhausted),
        ControlStep::Failed(failure),
    ] {
        let expected = match step {
            ControlStep::Stop(stop) => stop,
            ControlStep::Failed(failure) => Stop::Failure(IoMessageOperation::Wait, failure),
            _ => unreachable!(),
        };
        let state = state();
        let mut pipe = Pipe {
            driver: ScriptDriver::pending(&state),
        };
        assert_eq!(
            pipe.transfer(Buffer::Write(b"bytes"), &mut ScriptControl::new([step])),
            Awaited {
                transfer: None,
                stop: Some(expected),
                cancellation_failure: None
            }
        );
        assert!(state.borrow().events.is_empty());
    }
}

#[test]
fn immediate_success_and_terminal_submission_failure_do_not_wait_or_cancel() {
    for failed in [false, true] {
        let state = state();
        let failure = Failure::from_system_code(5);
        let mut driver = ScriptDriver::pending(&state);
        driver.submissions = [if failed {
            SubmitStep::Failed(failure)
        } else {
            SubmitStep::Complete
        }]
        .into();
        driver.queries = [QueryStep::Value(Query::Terminal(Ok(3)))].into();
        let mut pipe = Pipe { driver };
        let mut control = ScriptControl::new([ControlStep::Wait(u32::MAX)]);
        assert_eq!(
            pipe.transfer(Buffer::Write(b"bytes"), &mut control),
            Awaited::complete(if failed { Err(failure) } else { Ok(3) })
        );
        assert_eq!(control.calls, 1);
        assert!(!state
            .borrow()
            .events
            .iter()
            .any(|event| matches!(event, Event::Wait(..) | Event::Cancel | Event::Drain)));
        drop(pipe);
        assert_finished(&state);
    }
}

#[test]
fn stale_completion_and_finite_timeouts_requery_without_restarting_control() {
    let state = state();
    let mut driver = ScriptDriver::pending(&state);
    driver.queries = [
        QueryStep::Value(Query::Pending),
        QueryStep::Value(Query::Pending),
        QueryStep::Value(Query::Pending),
        QueryStep::Value(Query::Terminal(Ok(4))),
    ]
    .into();
    driver.wakes = [
        Ok(Wake::Completion),
        Ok(Wake::Timeout),
        Ok(Wake::Completion),
    ]
    .into();
    let mut pipe = Pipe { driver };
    let mut bytes = BorrowedStorage::new(&state);
    let address = bytes.bytes.as_ptr() as usize;
    let mut control = ScriptControl::new([
        ControlStep::Wait(100),
        ControlStep::Wait(90),
        ControlStep::Wait(30),
        ControlStep::Wait(1),
    ]);
    assert_eq!(
        pipe.transfer(Buffer::Read(&mut bytes.bytes), &mut control),
        Awaited::complete(Ok(4))
    );
    let state_ref = state.borrow();
    assert!(
        matches!(state_ref.events[0], Event::Submit(_, pointer, 8, true) if pointer == address)
    );
    assert_eq!(
        state_ref
            .events
            .iter()
            .filter_map(|event| if let Event::Wait(timeout, signal) = event {
                Some((*timeout, *signal))
            } else {
                None
            })
            .collect::<Vec<_>>(),
        [(90, Some(0x7171)), (30, Some(0x7171)), (1, Some(0x7171))]
    );
    assert!(!state_ref.events.contains(&Event::Cancel));
    drop(state_ref);
    drop(bytes);
    drop(pipe);
    assert_finished(&state);
}

#[test]
fn cancellation_preserves_terminal_success_error_and_cancel_request_failure() {
    for terminal in [
        Ok(3),
        Err(Failure::from_system_code(995)),
        Err(Failure::from_system_code(5).with_transferred_byte_count(Some(2))),
    ] {
        for cancellation in [
            Ok(()),
            Err(Failure::from_system_code(1168)),
            Err(Failure::from_system_code(5)),
        ] {
            let state = state();
            let mut driver = ScriptDriver::pending(&state);
            driver.wakes.push_back(Ok(Wake::Cancellation));
            driver.cancellations.push_back(cancellation);
            driver.drains.push_back(terminal);
            let mut pipe = Pipe { driver };
            let bytes = BorrowedStorage::new(&state);
            assert_eq!(
                pipe.transfer(
                    Buffer::Write(&bytes.bytes),
                    &mut ScriptControl::new([ControlStep::Wait(99), ControlStep::Wait(98)])
                ),
                Awaited {
                    transfer: Some(terminal),
                    stop: Some(Stop::Cancellation),
                    cancellation_failure: cancellation
                        .err()
                        .map(|failure| (IoMessageOperation::Wait, failure))
                }
            );
            drop(bytes);
            drop(pipe);
            let state_ref = state.borrow();
            assert_eq!(
                &state_ref.events[state_ref.events.len() - 5..],
                [
                    Event::Cancel,
                    Event::Drain,
                    Event::Terminal,
                    Event::BufferReleased,
                    Event::ResourcesReleased
                ]
            );
            drop(state_ref);
            assert_finished(&state);
        }
    }
}

#[test]
fn immediate_timeout_cancels_and_drains_once_instead_of_polling() {
    let state = state();
    let mut driver = ScriptDriver::pending(&state);
    driver.wakes.push_back(Ok(Wake::Timeout));
    driver.drains.push_back(Err(Failure::from_system_code(995)));
    let mut pipe = Pipe { driver };
    let mut control = ScriptControl::new([ControlStep::Wait(0), ControlStep::Wait(0)]);
    assert_eq!(
        pipe.transfer(Buffer::Write(b"bytes"), &mut control),
        Awaited {
            transfer: Some(Err(Failure::from_system_code(995))),
            stop: Some(Stop::BudgetExhausted),
            cancellation_failure: None
        }
    );
    assert_eq!(control.calls, 2);
    assert_eq!(
        state
            .borrow()
            .events
            .iter()
            .filter(|event| matches!(event, Event::Wait(..)))
            .count(),
        1
    );
    assert_eq!(
        &state.borrow().events[3..],
        [Event::Cancel, Event::Drain, Event::Terminal]
    );
    assert_finished(&state);
}

#[test]
fn query_wait_and_control_failures_drain_the_outstanding_request() {
    let failure = Failure::from_system_code(123);
    for phase in 0..4 {
        let state = state();
        let mut driver = ScriptDriver::pending(&state);
        let mut steps = vec![ControlStep::Wait(100)];
        match phase {
            0 => {
                driver.queries = [QueryStep::Value(Query::PendingFailure(
                    IoMessageOperation::Wait,
                    failure,
                ))]
                .into()
            }
            1 => {
                driver.wakes.push_back(Err(failure));
                steps.push(ControlStep::Wait(90));
            }
            2 => steps.push(ControlStep::Failed(failure)),
            _ => steps.push(ControlStep::Stop(Stop::BudgetExhausted)),
        }
        driver.drains.push_back(Ok(2));
        let mut pipe = Pipe { driver };
        let stop = if phase == 3 {
            Stop::BudgetExhausted
        } else {
            Stop::Failure(IoMessageOperation::Wait, failure)
        };
        assert_eq!(
            pipe.transfer(Buffer::Write(b"bytes"), &mut ScriptControl::new(steps)),
            Awaited {
                transfer: Some(Ok(2)),
                stop: Some(stop),
                cancellation_failure: None
            }
        );
        let state_ref = state.borrow();
        assert_eq!(
            &state_ref.events[state_ref.events.len() - 3..],
            [Event::Cancel, Event::Drain, Event::Terminal]
        );
        drop(state_ref);
        assert_finished(&state);
    }
}

#[test]
fn completed_count_guards_preserve_unknown_zero_and_bounded_partial_failure() {
    for drained in [false, true] {
        for result in [
            Ok(9),
            Err(Failure::from_system_code(5)),
            Err(Failure::from_system_code(5).with_transferred_byte_count(Some(0))),
            Err(Failure::from_system_code(5).with_transferred_byte_count(Some(3))),
            Err(Failure::from_system_code(5).with_transferred_byte_count(Some(9))),
        ] {
            let state = state();
            let mut driver = ScriptDriver::pending(&state);
            let mut steps = vec![ControlStep::Wait(100)];
            if drained {
                driver.drains.push_back(result);
                steps.push(ControlStep::Stop(Stop::Cancellation));
            } else {
                driver.queries = [QueryStep::Value(Query::Terminal(result))].into();
            }
            let mut pipe = Pipe { driver };
            let mut bytes = BorrowedStorage::new(&state);
            let expected = match result {
                Ok(_) => Err(Failure::other()),
                Err(failure) if failure.transferred_byte_count == Some(9) => {
                    Err(failure.with_transferred_byte_count(None))
                }
                other => other,
            };
            let awaited = pipe.transfer(
                Buffer::Read(&mut bytes.bytes),
                &mut ScriptControl::new(steps),
            );
            assert_eq!(awaited.transfer, Some(expected));
            assert_eq!(awaited.stop, drained.then_some(Stop::Cancellation));
            assert_finished(&state);
        }
    }
}

#[test]
fn cleanup_is_armed_before_submit_and_drains_during_host_unwind() {
    for phase in 0..3 {
        let state = state();
        let mut driver = ScriptDriver::pending(&state);
        let mut steps = vec![ControlStep::Wait(100)];
        match phase {
            0 => driver.submissions = [SubmitStep::Unwind].into(),
            1 => driver.queries = [QueryStep::Unwind].into(),
            _ => steps.push(ControlStep::Unwind),
        }
        let mut pipe = Pipe { driver };
        let mut bytes = BorrowedStorage::new(&state);
        assert!(crate::test_report::expect_panic(AssertUnwindSafe(|| {
            pipe.transfer(
                Buffer::Read(&mut bytes.bytes),
                &mut ScriptControl::new(steps),
            );
        }))
        .is_err());
        drop(bytes);
        drop(pipe);
        let state_ref = state.borrow();
        assert_eq!(
            &state_ref.events[state_ref.events.len() - 5..],
            [
                Event::Cancel,
                Event::Drain,
                Event::Terminal,
                Event::BufferReleased,
                Event::ResourcesReleased
            ]
        );
        drop(state_ref);
        assert_finished(&state);
    }
}

#[test]
fn one_owner_reuses_resources_only_after_terminal_completion() {
    let state = state();
    let mut driver = ScriptDriver::pending(&state);
    driver.submissions = [SubmitStep::Pending, SubmitStep::Pending].into();
    driver.queries = [
        QueryStep::Value(Query::Pending),
        QueryStep::Value(Query::Terminal(Ok(2))),
        QueryStep::Value(Query::Pending),
    ]
    .into();
    driver.wakes = [Ok(Wake::Completion), Ok(Wake::Cancellation)].into();
    driver.drains.push_back(Ok(1));
    let mut pipe = Pipe { driver };
    let mut bytes = BorrowedStorage::new(&state);
    assert_eq!(
        pipe.transfer(
            Buffer::Read(&mut bytes.bytes),
            &mut ScriptControl::new([ControlStep::Wait(100), ControlStep::Wait(99)])
        ),
        Awaited::complete(Ok(2))
    );
    assert_eq!(
        pipe.transfer(
            Buffer::Write(&bytes.bytes),
            &mut ScriptControl::new([ControlStep::Wait(100), ControlStep::Wait(99)])
        )
        .transfer,
        Some(Ok(1))
    );
    assert!(!state.borrow().events.contains(&Event::ResourcesReleased));
    assert_eq!(
        state
            .borrow()
            .events
            .iter()
            .filter(|event| matches!(event, Event::Submit(..)))
            .count(),
        2
    );
    drop(bytes);
    drop(pipe);
    assert_eq!(
        state
            .borrow()
            .events
            .iter()
            .filter(|event| **event == Event::ResourcesReleased)
            .count(),
        1
    );
    assert_finished(&state);
}
