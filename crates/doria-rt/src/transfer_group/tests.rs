use std::cell::RefCell;
use std::panic::AssertUnwindSafe;
use std::rc::Rc;

use super::*;
use doria_diagnostic_catalogue::IoMessageOperation;

mod observation_tests;
mod prepared_source_tests;
mod watch_tests;

struct TestRequest {
    terminal: usize,
    transferred: usize,
}

impl TestRequest {
    fn new() -> Self {
        Self {
            terminal: 0,
            transferred: 0,
        }
    }
}

fn request_slots(keys: &[u64]) -> Result<RequestSlots<TestRequest>, Failure> {
    RequestSlots::new(keys, |_| TestRequest::new())
}

const SLOTS: usize = 5;
const STEPS: usize = 32;
const EVENTS: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Event {
    Submit(usize, usize, usize, usize, usize, bool),
    Query(usize),
    Wait(i64, Option<usize>, u8),
    Cancel(usize),
    Drain(usize),
    Terminal(usize),
    ResourcesReleased,
    BufferReleased(usize),
}

#[derive(Clone, Copy, Default)]
struct Row {
    active: bool,
    cancelled: bool,
    generation: usize,
    request: usize,
    buffer: usize,
    length: usize,
    reading: bool,
    expected: [u8; 16],
}

struct Ledger {
    rows: [Row; SLOTS],
    events: [Option<Event>; EVENTS],
    used: usize,
}

impl Ledger {
    fn new() -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            rows: [Row::default(); SLOTS],
            events: [None; EVENTS],
            used: 0,
        }))
    }

    fn record(&mut self, event: Event) {
        self.events[self.used] = Some(event);
        self.used += 1;
    }

    fn mask(&self) -> u8 {
        self.rows.iter().enumerate().fold(0, |mask, (index, row)| {
            mask | (u8::from(row.active) << index)
        })
    }

    fn count(&self, predicate: impl Fn(Event) -> bool) -> usize {
        self.events[..self.used]
            .iter()
            .flatten()
            .filter(|event| predicate(**event))
            .count()
    }
}

#[derive(Clone, Copy)]
enum SubmitStep {
    Pending,
    Complete,
    Failed(Failure),
    Unwind,
}

#[derive(Clone, Copy)]
enum QueryStep {
    Value(QueryValue),
    Unwind,
}

// Query itself deliberately does not require Copy in production.
#[derive(Clone, Copy)]
enum QueryValue {
    Pending,
    Terminal(Result<usize, Failure>),
    Failed(Failure),
}

struct ScriptDriver {
    resources: [TransferResource; SLOTS],
    ledger: Rc<RefCell<Ledger>>,
    submits: [[Option<SubmitStep>; STEPS]; SLOTS],
    queries: [[Option<QueryStep>; STEPS]; SLOTS],
    submit_at: [usize; SLOTS],
    query_at: [usize; SLOTS],
    wakes: [Option<Result<Wake, Failure>>; STEPS],
    wake_at: usize,
    cancellations: [Result<(), Failure>; SLOTS],
    drains: [Result<usize, Failure>; SLOTS],
    unwind_wait: bool,
}

impl ScriptDriver {
    fn new(ledger: &Rc<RefCell<Ledger>>) -> Self {
        Self {
            resources: core::array::from_fn(|identity| TransferResource {
                identity,
                shared_cursor: false,
            }),
            ledger: Rc::clone(ledger),
            submits: [[None; STEPS]; SLOTS],
            queries: [[None; STEPS]; SLOTS],
            submit_at: [0; SLOTS],
            query_at: [0; SLOTS],
            wakes: [None; STEPS],
            wake_at: 0,
            cancellations: [Ok(()); SLOTS],
            drains: [Ok(0); SLOTS],
            unwind_wait: false,
        }
    }

    fn query_script(&mut self, index: usize, steps: &[QueryValue]) {
        for (target, value) in self.queries[index].iter_mut().zip(steps) {
            *target = Some(QueryStep::Value(*value));
        }
    }

    unsafe fn check(&self, index: usize, request: *mut TestRequest) {
        let ledger = self.ledger.borrow();
        let row = &ledger.rows[index];
        assert!(row.active);
        assert_eq!(row.request, request as usize);
        let bytes = core::slice::from_raw_parts(row.buffer as *const u8, row.length);
        assert_eq!(bytes, &row.expected[..row.length]);
    }

    unsafe fn terminal(&mut self, index: usize, result: Result<usize, Failure>) {
        let mut ledger = self.ledger.borrow_mut();
        let row = &mut ledger.rows[index];
        let count = match result {
            Ok(count) => Some(count),
            Err(failure) => failure.transferred_byte_count,
        };
        if row.reading {
            if let Some(count) = count {
                let count = count.min(row.length);
                core::slice::from_raw_parts_mut(row.buffer as *mut u8, count)
                    .fill(b'A' + index as u8);
            }
        }
        let request = row.request as *mut TestRequest;
        (*request).terminal = 1;
        (*request).transferred = count.unwrap_or(0);
        row.active = false;
        ledger.record(Event::Terminal(index));
    }
}

// The model owns raw payload pointers only while active. Every terminal fact
// retires them and drains never unwind, including on test-only host unwind.
unsafe impl GroupDriver for ScriptDriver {
    type Request = TestRequest;
    type Timeout = i64;
    const MAXIMUM_TRANSFER_BYTES: usize = 12;
    unsafe fn resource(
        &self,
        index: usize,
        _: *mut TestRequest,
    ) -> Result<TransferResource, Failure> {
        Ok(self.resources[index])
    }

    unsafe fn reset_request(&mut self, index: usize, request: *mut TestRequest) {
        assert!(!self.ledger.borrow().rows[index].active);
        request.write(TestRequest::new());
    }

    unsafe fn submit(
        &mut self,
        index: usize,
        request: *mut TestRequest,
        buffer: &mut Buffer<'_>,
    ) -> Submission {
        let (address, bytes, reading) = match buffer {
            Buffer::Read(bytes) => (bytes.as_ptr() as usize, &bytes[..], true),
            Buffer::Write(bytes) => (bytes.as_ptr() as usize, &bytes[..], false),
        };
        {
            let mut ledger = self.ledger.borrow_mut();
            let row = &mut ledger.rows[index];
            assert!(!row.active);
            assert!(bytes.len() <= row.expected.len());
            assert_eq!((*request).terminal, 0);
            assert_eq!((*request).transferred, 0);
            if row.generation != 0 {
                assert_eq!(
                    row.request, request as usize,
                    "request moved between submissions"
                );
            }
            row.active = true;
            row.cancelled = false;
            row.generation += 1;
            row.request = request as usize;
            row.buffer = address;
            row.length = bytes.len();
            row.reading = reading;
            row.expected[..bytes.len()].copy_from_slice(bytes);
            let generation = row.generation;
            ledger.record(Event::Submit(
                index,
                generation,
                request as usize,
                address,
                bytes.len(),
                reading,
            ));
        }
        self.check(index, request);
        let position = self.submit_at[index];
        self.submit_at[index] += 1;
        match self.submits[index][position].unwrap_or(SubmitStep::Pending) {
            SubmitStep::Pending => Submission::Pending,
            SubmitStep::Complete => Submission::Complete,
            SubmitStep::Failed(failure) => {
                self.terminal(index, Err(failure));
                Submission::Failed(failure)
            }
            SubmitStep::Unwind => panic!("test-only submission unwind after acquiring storage"),
        }
    }

    unsafe fn query(&mut self, index: usize, request: *mut TestRequest) -> Query {
        self.check(index, request);
        self.ledger.borrow_mut().record(Event::Query(index));
        let position = self.query_at[index];
        self.query_at[index] += 1;
        match self.queries[index][position].unwrap_or(QueryStep::Value(QueryValue::Pending)) {
            QueryStep::Value(QueryValue::Pending) => Query::Pending,
            QueryStep::Value(QueryValue::Failed(failure)) => {
                Query::PendingFailure(IoMessageOperation::Wait, failure)
            }
            QueryStep::Value(QueryValue::Terminal(result)) => {
                self.terminal(index, result);
                Query::Terminal(result)
            }
            QueryStep::Unwind => panic!("test-only query unwind"),
        }
    }

    fn wait_any(&mut self, timeout: i64, cancellation: Option<usize>) -> Result<Wake, Failure> {
        let mut ledger = self.ledger.borrow_mut();
        let mask = ledger.mask();
        assert_ne!(mask, 0, "wait with no active requests");
        ledger.record(Event::Wait(timeout, cancellation, mask));
        drop(ledger);
        if self.unwind_wait {
            panic!("test-only wait unwind");
        }
        let result = self.wakes[self.wake_at].expect("unexpected group wait");
        self.wake_at += 1;
        result
    }

    unsafe fn cancel(
        &mut self,
        index: usize,
        request: *mut TestRequest,
    ) -> Result<(), (IoMessageOperation, Failure)> {
        self.check(index, request);
        let mut ledger = self.ledger.borrow_mut();
        assert!(!ledger.rows[index].cancelled, "request cancelled twice");
        ledger.rows[index].cancelled = true;
        ledger.record(Event::Cancel(index));
        self.cancellations[index].map_err(|failure| (IoMessageOperation::Wait, failure))
    }

    unsafe fn drain(
        &mut self,
        index: usize,
        request: *mut TestRequest,
    ) -> Option<Result<usize, Failure>> {
        self.check(index, request);
        let mut ledger = self.ledger.borrow_mut();
        assert!(
            ledger.rows.iter().all(|row| !row.active || row.cancelled),
            "blocking drain began before cancellation fan-out finished"
        );
        ledger.record(Event::Drain(index));
        drop(ledger);
        let result = self.drains[index];
        self.terminal(index, result);
        Some(result)
    }
}

impl Drop for ScriptDriver {
    fn drop(&mut self) {
        let mut ledger = self.ledger.borrow_mut();
        assert_eq!(
            ledger.mask(),
            0,
            "resources released while requests remain active"
        );
        ledger.record(Event::ResourcesReleased);
    }
}

#[derive(Clone, Copy)]
enum ControlStep {
    Wait(u32),
    ClampedPoll,
    Stop(Stop),
    Failed(Failure),
    Unwind,
}

struct ScriptControl {
    steps: [Option<ControlStep>; STEPS],
    position: usize,
}

impl ScriptControl {
    fn new(steps: &[ControlStep]) -> Self {
        let mut control = Self {
            steps: [None; STEPS],
            position: 0,
        };
        for (target, step) in control.steps.iter_mut().zip(steps) {
            *target = Some(*step);
        }
        control
    }
}

impl Control<i64> for ScriptControl {
    fn next_wait(&mut self) -> Result<Result<Wait<i64>, Stop>, Failure> {
        let step = self.steps[self.position].expect("unexpected budget query");
        self.position += 1;
        match step {
            ControlStep::Wait(timeout) => Ok(Ok(Wait {
                timeout: i64::from(timeout),
                immediate: timeout == 0,
            })),
            ControlStep::ClampedPoll => Ok(Ok(Wait {
                timeout: 0,
                immediate: false,
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
    index: usize,
    ledger: Rc<RefCell<Ledger>>,
}

impl BorrowedStorage {
    fn new(index: usize, ledger: &Rc<RefCell<Ledger>>) -> Self {
        Self {
            bytes: *b"sentinel",
            index,
            ledger: Rc::clone(ledger),
        }
    }
}

impl Drop for BorrowedStorage {
    fn drop(&mut self) {
        let mut ledger = self.ledger.borrow_mut();
        assert!(
            !ledger.rows[self.index].active,
            "buffer released while still active"
        );
        ledger.record(Event::BufferReleased(self.index));
    }
}

#[test]
fn fixed_storage_is_aligned_checked_and_drops_initialized_elements_in_reverse() {
    #[repr(align(128))]
    struct Aligned(u8);
    let mut aligned = FixedSlice::new(2).unwrap();
    aligned.push(Aligned(7)).unwrap();
    assert_eq!(aligned.raw(0) as usize % 128, 0);
    assert_eq!(aligned.as_slice()[0].0, 7);
    assert!(FixedSlice::<TestRequest>::new(usize::MAX).is_err());
    let ledger = Ledger::new();
    {
        let mut storage = FixedSlice::new(3).unwrap();
        storage.push(BorrowedStorage::new(0, &ledger)).unwrap();
        storage.push(BorrowedStorage::new(1, &ledger)).unwrap();
    }
    assert_eq!(
        &ledger.borrow().events[..2],
        &[
            Some(Event::BufferReleased(1)),
            Some(Event::BufferReleased(0)),
        ]
    );
    let (empty, allocations) =
        crate::probe_runtime_allocations(|| FixedSlice::<u8>::new(0).unwrap());
    assert_eq!(empty.len(), 0);
    assert_eq!(allocations, 0);
}

#[test]
fn consuming_fixed_storage_moves_owners_once_and_drops_only_unmoved_elements() {
    for phase in 0..3 {
        let ledger = Ledger::new();
        let mut storage = FixedSlice::new(4).unwrap();
        for index in 0..4 {
            storage.push(BorrowedStorage::new(index, &ledger)).unwrap();
        }
        let ((), allocations) = crate::probe_runtime_allocations(|| {
            let mut values = storage.into_iter();
            assert_eq!(values.len(), 4);
            match phase {
                0 => {}
                1 => {
                    drop(values.next().unwrap());
                    drop(values.next_back().unwrap());
                    assert_eq!(values.len(), 2);
                }
                _ => {
                    for value in &mut values {
                        drop(value);
                    }
                    assert_eq!(values.len(), 0);
                    assert!(values.next().is_none());
                    assert!(values.next_back().is_none());
                }
            }
        });
        assert_eq!(allocations, 0);
        let order = match phase {
            0 => [3, 2, 1, 0],
            1 => [0, 3, 2, 1],
            _ => [0, 1, 2, 3],
        };
        assert_eq!(ledger.borrow().used, 4);
        for (position, index) in order.into_iter().enumerate() {
            assert_eq!(
                ledger.borrow().events[position],
                Some(Event::BufferReleased(index))
            );
        }
    }
    let mut empty = FixedSlice::<u8>::new(0).unwrap().into_iter();
    assert!(empty.next().is_none());
    assert!(empty.next_back().is_none());
}

#[test]
fn consuming_fixed_storage_unwind_never_redrops_moved_values() {
    struct DropProbe {
        index: usize,
        drops: Rc<RefCell<Vec<usize>>>,
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.drops.borrow_mut().push(self.index);
            if self.index == 3 {
                panic!("test-only destructor failure");
            }
        }
    }

    let drops = Rc::new(RefCell::new(Vec::new()));
    let mut storage = FixedSlice::new(5).unwrap();
    for index in 0..5 {
        storage
            .push(DropProbe {
                index,
                drops: Rc::clone(&drops),
            })
            .unwrap();
    }
    let result = crate::test_report::expect_panic(std::panic::AssertUnwindSafe(|| {
        let mut values = storage.into_iter();
        drop(values.next().unwrap());
        drop(values.next_back().unwrap());
        drop(values);
    }));
    assert!(result.is_err());
    assert_eq!(&*drops.borrow(), &[0, 4, 3, 2, 1]);
}

#[test]
fn empty_requests_and_initial_stops_deliver_without_submission() {
    for stop in [
        Stop::Cancellation,
        Stop::BudgetExhausted,
        Stop::Failure(IoMessageOperation::Wait, Failure::other()),
    ] {
        let ledger = Ledger::new();
        let mut slots = request_slots(&[10, 20, 30]).unwrap();
        let mut read = [0x55; 4];
        let buffers = BufferList::from_iter(
            [
                Buffer::Read(&mut []),
                Buffer::Read(&mut read),
                Buffer::Write(b"write"),
            ]
            .into_iter(),
        )
        .unwrap();
        let mut group = TransferGroup::begin(
            &mut slots,
            buffers,
            ScriptDriver::new(&ledger),
            ScriptControl::new(&[ControlStep::Stop(stop)]),
        )
        .unwrap();
        assert_eq!(group.stop(), Some(stop));
        assert_eq!(group.next().unwrap().outcome, Awaited::complete(Ok(0)));
        for index in 1..3 {
            let completion = group.next().unwrap();
            assert_eq!(completion.index, index);
            assert_eq!(completion.outcome.transfer, None);
            assert_eq!(completion.outcome.stop, Some(stop));
        }
        assert!(group.completed(1).is_none());
        assert_eq!(group.read_storage_mut(1).unwrap(), &[0x55; 4]);
        assert!(group.submit(1, 0, 4).is_err());
        assert!(group.next().is_none());
        assert_eq!(ledger.borrow().used, 0);
    }
    let ledger = Ledger::new();
    let mut slots = request_slots(&[]).unwrap();
    let buffers = BufferList::from_iter([].into_iter()).unwrap();
    let mut group = TransferGroup::begin(
        &mut slots,
        buffers,
        ScriptDriver::new(&ledger),
        ScriptControl::new(&[]),
    )
    .unwrap();
    assert!(group.next().is_none());
    assert_eq!(group.control.position, 0);
}

#[test]
fn simultaneous_mixed_transfers_initialize_only_their_own_bounded_prefixes() {
    let ledger = Ledger::new();
    let mut slots = request_slots(&[50, 40, 30, 20, 10]).unwrap();
    let mut first = [0x55; 8];
    let mut third = [0x66; 8];
    let mut fifth = [0x77; 8];
    let mut driver = ScriptDriver::new(&ledger);
    for index in 0..SLOTS {
        driver.query_script(index, &[QueryValue::Terminal(Ok(index + 1))]);
    }
    driver.submits[1][0] = Some(SubmitStep::Complete);
    let buffers = BufferList::from_iter(
        [
            Buffer::Read(&mut first),
            Buffer::Write(b"12345678"),
            Buffer::Read(&mut third),
            Buffer::Write(b"abcdefgh"),
            Buffer::Read(&mut fifth),
        ]
        .into_iter(),
    )
    .unwrap();
    let mut group = TransferGroup::begin(
        &mut slots,
        buffers,
        driver,
        ScriptControl::new(&[ControlStep::Wait(100)]),
    )
    .unwrap();
    assert_eq!(ledger.borrow().mask(), 0b11111);
    assert!(group.completed(0).is_none());
    assert!(group.read_storage_mut(2).is_none());
    for (index, key) in [50, 40, 30, 20, 10].into_iter().enumerate() {
        let completion = group.next().unwrap();
        assert_eq!((completion.index, completion.key), (index, key));
        assert_eq!(completion.outcome.transfer, Some(Ok(index + 1)));
        assert_eq!(group.completed(index).unwrap().len(), index + 1);
    }
    assert!(group.next().is_none());
    assert_eq!(
        group.read_storage_mut(0).unwrap(),
        b"A\x55\x55\x55\x55\x55\x55\x55"
    );
    assert_eq!(
        group.read_storage_mut(2).unwrap(),
        b"CCC\x66\x66\x66\x66\x66"
    );
    assert_eq!(group.read_storage_mut(4).unwrap(), b"EEEEE\x77\x77\x77");
    assert!(group.read_storage_mut(1).is_none());
    assert_eq!(
        ledger
            .borrow()
            .count(|event| matches!(event, Event::Wait(..))),
        0
    );
}

#[test]
fn ready_service_rotates_even_when_first_slot_is_immediately_resubmitted() {
    let ledger = Ledger::new();
    let mut slots = request_slots(&[0, 1, 2]).unwrap();
    let mut bytes = [0x55; 8];
    let mut driver = ScriptDriver::new(&ledger);
    for index in 0..3 {
        driver.query_script(
            index,
            &[QueryValue::Terminal(Ok(2)), QueryValue::Terminal(Ok(1))],
        );
    }
    let buffers = BufferList::from_iter(
        [
            Buffer::Read(&mut bytes),
            Buffer::Write(b"second"),
            Buffer::Write(b"third"),
        ]
        .into_iter(),
    )
    .unwrap();
    let mut group = TransferGroup::begin(
        &mut slots,
        buffers,
        driver,
        ScriptControl::new(&[ControlStep::Wait(100), ControlStep::Wait(90)]),
    )
    .unwrap();
    assert_eq!(group.next().unwrap().index, 0);
    group.read_storage_mut(0).unwrap().fill(0x33);
    group.submit(0, 0, 8).unwrap();
    assert!(group.completed(0).is_none());
    assert!(group.read_storage_mut(0).is_none());
    assert_eq!(group.next().unwrap().index, 1);
    assert_eq!(group.next().unwrap().index, 2);
    assert_eq!(group.next().unwrap().index, 0);
    assert!(group.next().is_none());
    assert_eq!(ledger.borrow().rows[0].generation, 2);
}

#[test]
fn resubmission_rejects_active_unconsumed_and_invalid_ranges_then_continues_partial_write() {
    let ledger = Ledger::new();
    let mut slots = request_slots(&[10, 20]).unwrap();
    let mut driver = ScriptDriver::new(&ledger);
    driver.query_script(
        0,
        &[QueryValue::Terminal(Ok(2)), QueryValue::Terminal(Ok(4))],
    );
    let source = b"abcdef";
    let buffers =
        BufferList::from_iter([Buffer::Write(source), Buffer::Read(&mut [])].into_iter()).unwrap();
    let mut group = TransferGroup::begin(
        &mut slots,
        buffers,
        driver,
        ScriptControl::new(&[ControlStep::Wait(100), ControlStep::Wait(90)]),
    )
    .unwrap();
    assert!(group.submit(0, 0, 6).is_err());
    assert!(group.submit(1, 0, 0).is_err());
    assert_eq!(group.next().unwrap().index, 1);
    assert_eq!(group.next().unwrap().outcome.transfer, Some(Ok(2)));
    assert_eq!(group.completed(0).unwrap(), b"ab");
    for (index, offset, length) in [(2, 0, 0), (0, 7, 0), (0, 2, 5), (0, usize::MAX, 1)] {
        assert!(group.submit(index, offset, length).is_err());
    }
    group.submit(0, 2, 4).unwrap();
    assert_eq!(group.next().unwrap().outcome.transfer, Some(Ok(4)));
    assert_eq!(group.completed(0).unwrap(), b"cdef");
    let ledger = ledger.borrow();
    assert!(ledger.events[..ledger.used].contains(&Some(Event::Submit(
        0,
        2,
        ledger.rows[0].request,
        source.as_ptr() as usize + 2,
        4,
        false
    ))));
}

#[test]
fn cancellation_fans_out_then_drains_all_and_preserves_each_terminal_result() {
    let ledger = Ledger::new();
    let mut slots = request_slots(&[10, 20, 30]).unwrap();
    let mut first = BorrowedStorage::new(0, &ledger);
    let mut third = BorrowedStorage::new(2, &ledger);
    let aborted = Failure::from_system_code(995);
    let partial = Failure::from_system_code(5).with_transferred_byte_count(Some(2));
    let not_found = Failure::from_system_code(1168);
    let other = Failure::from_system_code(5);
    let mut driver = ScriptDriver::new(&ledger);
    driver.wakes[0] = Some(Ok(Wake::Cancellation));
    driver.cancellations[..3].copy_from_slice(&[Ok(()), Err(not_found), Err(other)]);
    driver.drains[..3].copy_from_slice(&[Ok(3), Err(aborted), Err(partial)]);
    let buffers = BufferList::from_iter(
        [
            Buffer::Read(&mut first.bytes),
            Buffer::Write(b"write"),
            Buffer::Read(&mut third.bytes),
        ]
        .into_iter(),
    )
    .unwrap();
    let mut group = TransferGroup::begin(
        &mut slots,
        buffers,
        driver,
        ScriptControl::new(&[ControlStep::Wait(100), ControlStep::Wait(90)]),
    )
    .unwrap();
    for (index, terminal) in [Ok(3), Err(aborted), Err(partial)].into_iter().enumerate() {
        let completion = group.next().unwrap();
        assert_eq!(completion.index, index);
        assert_eq!(completion.outcome.transfer, Some(terminal));
        assert_eq!(completion.outcome.stop, Some(Stop::Cancellation));
        assert_eq!(
            completion.outcome.cancellation_failure,
            [None, Some(not_found), Some(other)][index]
                .map(|failure| (IoMessageOperation::Wait, failure))
        );
    }
    assert_eq!(group.completed(0).unwrap(), b"AAA");
    assert!(group.completed(1).is_none());
    assert_eq!(group.completed(2).unwrap(), b"CC");
    assert_eq!(group.stop(), Some(Stop::Cancellation));
    assert!(group.next().is_none());
    drop(group);
    drop(third);
    drop(first);
    let ledger = ledger.borrow();
    let first_cancel = ledger.events[..ledger.used]
        .iter()
        .position(|event| *event == Some(Event::Cancel(0)))
        .unwrap();
    assert_eq!(
        &ledger.events[first_cancel..first_cancel + 3],
        &[
            Some(Event::Cancel(0)),
            Some(Event::Cancel(1)),
            Some(Event::Cancel(2)),
        ]
    );
    assert_eq!(
        ledger.count(|event| matches!(event, Event::ResourcesReleased)),
        1
    );
    assert_eq!(ledger.mask(), 0);
}

#[test]
fn stale_wakes_and_resumed_waits_use_one_decreasing_budget_and_signal() {
    let ledger = Ledger::new();
    let mut slots = request_slots(&[10, 20]).unwrap();
    let mut driver = ScriptDriver::new(&ledger);
    driver.query_script(0, &[QueryValue::Pending, QueryValue::Terminal(Ok(2))]);
    driver.wakes[..3].copy_from_slice(&[
        Some(Ok(Wake::Completion)),
        Some(Ok(Wake::Timeout)),
        Some(Ok(Wake::Completion)),
    ]);
    driver.drains[1] = Ok(1);
    let buffers =
        BufferList::from_iter([Buffer::Write(b"first"), Buffer::Write(b"second")].into_iter())
            .unwrap();
    let mut group = TransferGroup::begin(
        &mut slots,
        buffers,
        driver,
        ScriptControl::new(&[
            ControlStep::Wait(100),
            ControlStep::Wait(90),
            ControlStep::Wait(30),
            ControlStep::Wait(1),
            ControlStep::Stop(Stop::BudgetExhausted),
        ]),
    )
    .unwrap();
    assert_eq!(group.next().unwrap().outcome, Awaited::complete(Ok(2)));
    let stopped = group.next().unwrap();
    assert_eq!(stopped.index, 1);
    assert_eq!(stopped.outcome.transfer, Some(Ok(1)));
    assert_eq!(stopped.outcome.stop, Some(Stop::BudgetExhausted));
    assert_eq!(group.control.position, 5);
    let ledger = ledger.borrow();
    for (timeout, mask) in [(90, 0b11), (30, 0b10), (1, 0b10)] {
        assert!(ledger.events[..ledger.used].contains(&Some(Event::Wait(
            timeout,
            Some(0x7171),
            mask
        ))));
    }
    assert_eq!(ledger.count(|event| matches!(event, Event::Wait(..))), 3);
}

#[test]
fn immediate_timeout_and_query_wait_control_failures_drain_every_active_slot() {
    for phase in 0..5 {
        let ledger = Ledger::new();
        let mut slots = request_slots(&[10, 20]).unwrap();
        let failure = Failure::from_system_code(123);
        let mut driver = ScriptDriver::new(&ledger);
        let steps = match phase {
            0 => {
                driver.wakes[0] = Some(Ok(Wake::Timeout));
                [ControlStep::Wait(0), ControlStep::Wait(0)]
            }
            1 => {
                driver.query_script(1, &[QueryValue::Failed(failure)]);
                [ControlStep::Wait(100), ControlStep::Wait(90)]
            }
            2 => {
                driver.wakes[0] = Some(Err(failure));
                [ControlStep::Wait(100), ControlStep::Wait(90)]
            }
            3 => [ControlStep::Wait(100), ControlStep::Failed(failure)],
            _ => [
                ControlStep::Wait(100),
                ControlStep::Stop(Stop::BudgetExhausted),
            ],
        };
        driver.drains[..2].copy_from_slice(&[Ok(2), Ok(3)]);
        let buffers =
            BufferList::from_iter([Buffer::Write(b"first"), Buffer::Write(b"second")].into_iter())
                .unwrap();
        let mut group =
            TransferGroup::begin(&mut slots, buffers, driver, ScriptControl::new(&steps)).unwrap();
        let expected_stop = if phase == 0 || phase == 4 {
            Stop::BudgetExhausted
        } else {
            Stop::Failure(IoMessageOperation::Wait, failure)
        };
        for count in [2, 3] {
            let completion = group.next().unwrap();
            assert_eq!(completion.outcome.transfer, Some(Ok(count)));
            assert_eq!(completion.outcome.stop, Some(expected_stop));
        }
        assert_eq!(
            ledger
                .borrow()
                .count(|event| matches!(event, Event::Cancel(_))),
            2
        );
        assert_eq!(
            ledger
                .borrow()
                .count(|event| matches!(event, Event::Drain(_))),
            2
        );
        if phase == 0 {
            assert_eq!(
                ledger
                    .borrow()
                    .count(|event| matches!(event, Event::Wait(..))),
                1
            );
        }
    }
}

#[test]
fn known_unknown_zero_and_invalid_progress_remain_distinct() {
    for result in [
        Ok(9),
        Err(Failure::from_system_code(5)),
        Err(Failure::from_system_code(5).with_transferred_byte_count(Some(0))),
        Err(Failure::from_system_code(5).with_transferred_byte_count(Some(3))),
        Err(Failure::from_system_code(5).with_transferred_byte_count(Some(9))),
    ] {
        let ledger = Ledger::new();
        let mut slots = request_slots(&[10]).unwrap();
        let mut driver = ScriptDriver::new(&ledger);
        driver.query_script(0, &[QueryValue::Terminal(result)]);
        let mut bytes = [0x55; 8];
        let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
        let mut group = TransferGroup::begin(
            &mut slots,
            buffers,
            driver,
            ScriptControl::new(&[ControlStep::Wait(100)]),
        )
        .unwrap();
        let expected = match result {
            Ok(_) => Err(Failure::other()),
            Err(failure) if failure.transferred_byte_count == Some(9) => {
                Err(failure.with_transferred_byte_count(None))
            }
            other => other,
        };
        assert_eq!(group.next().unwrap().outcome.transfer, Some(expected));
        assert_eq!(
            group.completed(0).map(<[u8]>::len),
            expected
                .err()
                .and_then(|failure| failure.transferred_byte_count)
        );
    }
}

#[test]
fn armed_group_cleanup_covers_later_submission_query_wait_and_control_host_unwind() {
    for phase in 0..4 {
        let ledger = Ledger::new();
        let mut slots = request_slots(&[10, 20]).unwrap();
        let mut first = BorrowedStorage::new(0, &ledger);
        let mut second = BorrowedStorage::new(1, &ledger);
        let mut driver = ScriptDriver::new(&ledger);
        let second_step = if phase == 3 {
            ControlStep::Unwind
        } else {
            ControlStep::Wait(90)
        };
        match phase {
            0 => driver.submits[1][0] = Some(SubmitStep::Unwind),
            1 => driver.queries[0][0] = Some(QueryStep::Unwind),
            2 => driver.unwind_wait = true,
            _ => {}
        }
        assert!(crate::test_report::expect_panic(AssertUnwindSafe(|| {
            let buffers = BufferList::from_iter(
                [
                    Buffer::Read(&mut first.bytes),
                    Buffer::Read(&mut second.bytes),
                ]
                .into_iter(),
            )
            .unwrap();
            let mut group = TransferGroup::begin(
                &mut slots,
                buffers,
                driver,
                ScriptControl::new(&[ControlStep::Wait(100), second_step]),
            )
            .unwrap();
            let _ = group.next();
        }))
        .is_err());
        drop(second);
        drop(first);
        let ledger = ledger.borrow();
        assert_eq!(ledger.mask(), 0);
        assert_eq!(ledger.count(|event| matches!(event, Event::Cancel(_))), 2);
        assert_eq!(ledger.count(|event| matches!(event, Event::Drain(_))), 2);
        assert_eq!(
            ledger.count(|event| matches!(event, Event::ResourcesReleased)),
            1
        );
    }
}

#[test]
fn terminal_submission_failure_does_not_discard_other_active_transfers() {
    let ledger = Ledger::new();
    let mut slots = request_slots(&[10, 20]).unwrap();
    let failure = Failure::from_system_code(5);
    let mut driver = ScriptDriver::new(&ledger);
    driver.submits[1][0] = Some(SubmitStep::Failed(failure));
    driver.query_script(0, &[QueryValue::Terminal(Ok(3))]);
    let buffers =
        BufferList::from_iter([Buffer::Write(b"first"), Buffer::Write(b"second")].into_iter())
            .unwrap();
    let mut group = TransferGroup::begin(
        &mut slots,
        buffers,
        driver,
        ScriptControl::new(&[ControlStep::Wait(100)]),
    )
    .unwrap();
    assert_eq!(ledger.borrow().mask(), 1);
    assert_eq!(group.next().unwrap().outcome.transfer, Some(Err(failure)));
    assert_eq!(group.next().unwrap().outcome.transfer, Some(Ok(3)));
    assert_eq!(group.stop(), None);
    assert!(group.next().is_none());
    assert_eq!(
        ledger
            .borrow()
            .count(|event| matches!(event, Event::Cancel(_) | Event::Drain(_))),
        0
    );
}

#[test]
fn configuration_storage_and_repeated_resubmission_use_no_new_runtime_allocations() {
    let ledger = Ledger::new();
    let mut driver = ScriptDriver::new(&ledger);
    for target in &mut driver.queries[0][..16] {
        *target = Some(QueryStep::Value(QueryValue::Terminal(Ok(2))));
    }
    let mut control = ScriptControl::new(&[]);
    control.steps[..16].fill(Some(ControlStep::Wait(100)));
    let mut bytes = [0x55; 8];
    let (mut slots, slot_allocations) =
        crate::probe_runtime_allocations(|| request_slots(&[10, 20]).unwrap());
    let (buffers, buffer_allocations) = crate::probe_runtime_allocations(|| {
        BufferList::from_iter([Buffer::Read(&mut bytes), Buffer::Write(b"pending")].into_iter())
            .unwrap()
    });
    assert_eq!((slot_allocations, buffer_allocations), (1, 1));
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        let mut group = TransferGroup::begin(&mut slots, buffers, driver, control).unwrap();
        for generation in 1..=16 {
            let completion = group.next().unwrap();
            assert_eq!(completion.index, 0);
            assert_eq!(group.completed(0).unwrap(), b"AA");
            assert!(group.read_storage_mut(1).is_none());
            assert_eq!(ledger.borrow().rows[0].generation, generation);
            if generation != 16 {
                group.read_storage_mut(0).unwrap().fill(0x55);
                group.submit(0, 0, 8).unwrap();
            }
        }
        assert_eq!(ledger.borrow().mask(), 0b10);
        // Dropping the group must cancel/drain slot 1 before its driver releases
        // resources, while already completed slot 0 is not cancelled again.
    });
    assert_eq!(allocations, 0);
    assert_eq!(
        ledger
            .borrow()
            .count(|event| matches!(event, Event::Submit(0, ..))),
        16
    );
    assert_eq!(
        ledger
            .borrow()
            .count(|event| matches!(event, Event::Cancel(1))),
        1
    );
    assert_eq!(
        ledger
            .borrow()
            .count(|event| matches!(event, Event::Cancel(0))),
        0
    );
    assert_eq!(
        ledger
            .borrow()
            .count(|event| matches!(event, Event::ResourcesReleased)),
        1
    );
}

#[test]
fn clamped_native_poll_does_not_turn_a_finite_budget_into_immediate() {
    let ledger = Ledger::new();
    let mut slots = request_slots(&[10]).unwrap();
    let mut driver = ScriptDriver::new(&ledger);
    driver.query_script(0, &[QueryValue::Pending, QueryValue::Terminal(Ok(2))]);
    driver.wakes[0] = Some(Ok(Wake::Timeout));
    let buffers = BufferList::from_iter([Buffer::Write(b"bytes")].into_iter()).unwrap();
    let mut group = TransferGroup::begin(
        &mut slots,
        buffers,
        driver,
        ScriptControl::new(&[ControlStep::Wait(100), ControlStep::ClampedPoll]),
    )
    .unwrap();
    assert_eq!(group.next().unwrap().outcome, Awaited::complete(Ok(2)));
    assert_eq!(group.stop(), None);
    assert_eq!(
        ledger
            .borrow()
            .count(|event| matches!(event, Event::Wait(0, ..))),
        1
    );
    assert_eq!(
        ledger
            .borrow()
            .count(|event| matches!(event, Event::Cancel(_))),
        0
    );
}

#[test]
fn terminal_progress_obeys_the_driver_limit_not_a_windows_constant() {
    for result in [
        Ok(12),
        Ok(13),
        Err(Failure::other().with_transferred_byte_count(Some(12))),
        Err(Failure::other().with_transferred_byte_count(Some(13))),
    ] {
        let ledger = Ledger::new();
        let mut slots = request_slots(&[10]).unwrap();
        let mut driver = ScriptDriver::new(&ledger);
        driver.query_script(0, &[QueryValue::Terminal(result)]);
        let buffers =
            BufferList::from_iter([Buffer::Write(b"0123456789abcdef")].into_iter()).unwrap();
        let mut group = TransferGroup::begin(
            &mut slots,
            buffers,
            driver,
            ScriptControl::new(&[ControlStep::Wait(100)]),
        )
        .unwrap();
        let expected = match result {
            Ok(13) => Err(Failure::other()),
            Err(failure) if failure.transferred_byte_count == Some(13) => {
                Err(failure.with_transferred_byte_count(None))
            }
            other => other,
        };
        assert_eq!(group.next().unwrap().outcome.transfer, Some(expected));
        let progress = expected
            .map(Some)
            .unwrap_or_else(|failure| failure.transferred_byte_count);
        assert_eq!(group.completed(0).map(<[u8]>::len), progress);
    }
}

#[test]
fn inconsistent_configuration_and_exact_size_iterators_fail_before_submission() {
    struct IncorrectSize<'b> {
        value: Option<Buffer<'b>>,
        reported: usize,
    }
    impl<'b> Iterator for IncorrectSize<'b> {
        type Item = Buffer<'b>;
        fn next(&mut self) -> Option<Self::Item> {
            self.value.take()
        }
        fn size_hint(&self) -> (usize, Option<usize>) {
            (self.reported, Some(self.reported))
        }
    }
    impl ExactSizeIterator for IncorrectSize<'_> {}
    for reported in [0, 2] {
        assert!(
            BufferList::<Result<usize, Failure>>::from_iter(IncorrectSize {
                value: Some(Buffer::Write(b"bytes")),
                reported
            })
            .is_err()
        );
    }
    let ledger = Ledger::new();
    let mut slots = request_slots(&[10, 20]).unwrap();
    let buffers = BufferList::from_iter([Buffer::Write(b"bytes")].into_iter()).unwrap();
    assert!(TransferGroup::begin(
        &mut slots,
        buffers,
        ScriptDriver::new(&ledger),
        ScriptControl::new(&[])
    )
    .is_err());
    assert_eq!(ledger.borrow().events[0], Some(Event::ResourcesReleased));
    assert_eq!(ledger.borrow().used, 1);
}

fn selection(indices: &[usize]) -> FixedSlice<usize> {
    let mut selected = FixedSlice::new(indices.len()).unwrap();
    for index in indices {
        selected.push(*index).unwrap();
    }
    selected
}

#[test]
fn selected_group_preserves_plan_order_and_reuses_registered_slots_without_allocating() {
    let ledger = Ledger::new();
    let mut slots = request_slots(&[10, 20, 30]).unwrap();
    assert_eq!((slots.index_of(30), slots.index_of(99)), (Some(2), None));
    let addresses = [slots.raw(0), slots.raw(1), slots.raw(2)];
    let mut driver = ScriptDriver::new(&ledger);
    driver.query_script(
        2,
        &[QueryValue::Terminal(Ok(3)), QueryValue::Terminal(Ok(2))],
    );
    driver.query_script(
        0,
        &[
            QueryValue::Pending,
            QueryValue::Pending,
            QueryValue::Terminal(Ok(4)),
        ],
    );
    let mut bytes = [0x55; 8];
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut bytes), Buffer::Write(b"hello")].into_iter())
            .unwrap();
    let mut group = TransferGroup::begin_selected(
        &mut slots,
        buffers,
        driver,
        ScriptControl::new(&[ControlStep::Wait(100), ControlStep::Wait(90)]),
        selection(&[2, 0]),
    )
    .unwrap();
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        let first = group.next().unwrap();
        assert_eq!(
            (first.index, first.key, first.outcome.transfer),
            (0, 30, Some(Ok(3)))
        );
        assert_eq!(group.completed(0).unwrap(), b"CCC");
        group.read_storage_mut(0).unwrap().fill(0x77);
        group.submit(0, 4, 2).unwrap();
        let reused = group.next().unwrap();
        assert_eq!(
            (reused.index, reused.key, reused.outcome.transfer),
            (0, 30, Some(Ok(2)))
        );
        assert_eq!(group.completed(0).unwrap(), b"CC");
        let other = group.next().unwrap();
        assert_eq!(
            (other.index, other.key, other.outcome.transfer),
            (1, 10, Some(Ok(4)))
        );
        assert_eq!(group.completed(1).unwrap(), b"hell");
        assert!(group.next().is_none());
        drop(group);
    });
    assert_eq!(allocations, 0);
    assert_eq!(bytes, [0x77, 0x77, 0x77, 0x77, b'C', b'C', 0x77, 0x77]);
    assert_eq!([slots.raw(0), slots.raw(1), slots.raw(2)], addresses);
    let ledger = ledger.borrow();
    assert_eq!(ledger.rows[1].generation, 0);
    assert_eq!(ledger.rows[2].generation, 2);
    assert_eq!(ledger.rows[2].request, addresses[2] as usize);
    assert_eq!(ledger.rows[0].request, addresses[0] as usize);
}

#[test]
fn selected_group_rejects_duplicates_out_of_range_and_mismatched_counts_before_io() {
    for indices in [&[1, 1][..], &[0, 3][..], &[2][..]] {
        let ledger = Ledger::new();
        let mut slots = request_slots(&[10, 20, 30]).unwrap();
        let mut bytes = [0x55; 8];
        let buffers = BufferList::from_iter(
            [Buffer::Read(&mut bytes), Buffer::Write(b"unchanged")].into_iter(),
        )
        .unwrap();
        let indices = selection(indices);
        let (result, allocations) = crate::probe_runtime_allocations(|| {
            TransferGroup::begin_selected(
                &mut slots,
                buffers,
                ScriptDriver::new(&ledger),
                ScriptControl::new(&[]),
                indices,
            )
        });
        assert!(result.is_err());
        drop(result);
        assert_eq!(allocations, 0);
        assert_eq!(bytes, [0x55; 8]);
        assert_eq!(ledger.borrow().events[0], Some(Event::ResourcesReleased));
        assert_eq!(ledger.borrow().used, 1);
    }
}

#[test]
fn selected_group_drop_cancels_all_and_drains_only_participating_registered_requests() {
    let ledger = Ledger::new();
    let mut slots = request_slots(&[10, 20, 30]).unwrap();
    let mut bytes = [0x55; 8];
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut bytes), Buffer::Write(b"unchanged")].into_iter())
            .unwrap();
    let group = TransferGroup::begin_selected(
        &mut slots,
        buffers,
        ScriptDriver::new(&ledger),
        ScriptControl::new(&[ControlStep::Wait(100)]),
        selection(&[2, 0]),
    )
    .unwrap();
    drop(group);
    assert_eq!(bytes, [0x55; 8]);
    let ledger = ledger.borrow();
    let operations: Vec<_> = ledger.events[..ledger.used]
        .iter()
        .flatten()
        .filter_map(|event| matches!(event, Event::Cancel(_) | Event::Drain(_)).then_some(*event))
        .collect();
    assert_eq!(
        operations,
        [
            Event::Cancel(2),
            Event::Cancel(0),
            Event::Drain(2),
            Event::Drain(0)
        ]
    );
    assert_eq!(ledger.rows[1].generation, 0);
    assert_eq!(ledger.mask(), 0);
}

#[test]
fn dynamic_request_slots_keep_addresses_and_key_lookup_across_growth_and_holes() {
    let (mut slots, allocations) =
        crate::probe_runtime_allocations(|| RequestSlots::dynamic().unwrap());
    assert_eq!(allocations, 0);
    for key in [10, 20, 30] {
        slots.add(key, TestRequest::new()).unwrap();
    }
    let first = slots.raw(0);
    let third = slots.raw(2);
    slots.remove(1).unwrap();
    assert_eq!((slots.len(), slots.occupied_len()), (3, 2));
    assert!(!slots.contains(1));
    assert!(!slots.contains(3));
    assert_eq!((slots.index_of(20), slots.index_of(30)), (None, Some(2)));
    let (duplicate, allocations) =
        crate::probe_runtime_allocations(|| slots.add(30, TestRequest::new()));
    assert!(duplicate.is_err());
    assert_eq!(allocations, 0);
    assert_eq!(slots.add(40, TestRequest::new()).unwrap(), 1);
    assert_eq!(slots.add(50, TestRequest::new()).unwrap(), 3);
    assert_eq!((slots.raw(0), slots.raw(2)), (first, third));
    assert_eq!((slots.len(), slots.occupied_len()), (4, 4));
    assert_eq!(slots.key(1), 40);
    assert_eq!(slots.index_of(50), Some(3));
    assert!(slots.remove(99).is_err());
}

#[test]
fn operation_cache_separates_registration_membership_and_preserves_arbitrary_capacity() {
    for fixed in [false, true] {
        let mut slots = if fixed {
            request_slots(&[10, 20]).unwrap()
        } else {
            let mut slots = RequestSlots::dynamic().unwrap();
            slots
                .add_registration(0, 10, |_| Ok(TestRequest::new()))
                .unwrap();
            slots
                .add_registration(1, 20, |_| Ok(TestRequest::new()))
                .unwrap();
            slots
        };
        let original = [slots.raw(0), slots.raw(1)];
        let registrations = [0, 1, 0, 0, 0, 1, 0];
        let operations = slots
            .configure_operations(selection(&registrations), |_, _| Ok(TestRequest::new()))
            .unwrap();
        let pointers: Vec<_> = operations
            .as_slice()
            .iter()
            .map(|index| slots.raw(*index))
            .collect();
        assert_eq!([slots.raw(0), slots.raw(1)], original);
        for (position, index) in operations.as_slice().iter().copied().enumerate() {
            assert!(!operations.as_slice()[..position].contains(&index));
            assert_eq!(slots.registration(index), registrations[position]);
        }
        let selected = selection(&registrations);
        let (reused, allocations) = crate::probe_runtime_allocations(|| {
            slots
                .configure_operations(selected, |_, _| panic!("cached requests must be reused"))
                .unwrap()
        });
        assert_eq!(
            allocations, 0,
            "the plan descriptor and all cached requests are reused"
        );
        assert_eq!(reused.as_slice(), operations.as_slice());
        assert_eq!(
            pointers,
            reused
                .as_slice()
                .iter()
                .map(|index| slots.raw(*index))
                .collect::<Vec<_>>()
        );
        if !fixed {
            slots.remove_registration(0).unwrap();
            assert_eq!(slots.operation_of(0, 0), None);
            assert_eq!(slots.raw(1), original[1]);
            let new_primary = slots
                .add_registration(2, 30, |_| Ok(TestRequest::new()))
                .unwrap();
            assert_eq!(new_primary, 0);
            assert_eq!(slots.registration(new_primary), 2);
            assert_eq!(slots.index_of(10), None);
            assert_eq!(slots.index_of(20), Some(1));
        }
    }
}

#[test]
fn request_slot_removal_drops_exactly_once_and_fixed_membership_remains_immutable() {
    struct Dropped(Rc<RefCell<Vec<usize>>>, usize);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.borrow_mut().push(self.1);
        }
    }
    let dropped = Rc::new(RefCell::new(Vec::new()));
    let mut slots = RequestSlots::dynamic().unwrap();
    slots.add(10, Dropped(Rc::clone(&dropped), 0)).unwrap();
    slots.add(20, Dropped(Rc::clone(&dropped), 1)).unwrap();
    slots.add(30, Dropped(Rc::clone(&dropped), 2)).unwrap();
    slots.remove(1).unwrap();
    assert!(slots.remove(1).is_err());
    assert_eq!(&*dropped.borrow(), &[1]);
    drop(slots);
    assert_eq!(&*dropped.borrow(), &[1, 2, 0]);

    let mut fixed = request_slots(&[10, 20]).unwrap();
    let address = fixed.raw(1);
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        assert!(fixed.add(30, TestRequest::new()).is_err());
        assert!(fixed.remove(1).is_err());
    });
    assert_eq!(allocations, 0);
    assert_eq!((fixed.len(), fixed.occupied_len()), (2, 2));
    assert_eq!(fixed.raw(1), address);
    assert_eq!(fixed.index_of(20), Some(1));
}

#[test]
fn sparse_request_holes_are_rejected_before_io_for_selected_and_dense_groups() {
    for selected in [true, false] {
        let ledger = Ledger::new();
        let mut slots = RequestSlots::dynamic().unwrap();
        for key in [10, 20, 30] {
            slots.add(key, TestRequest::new()).unwrap();
        }
        slots.remove(1).unwrap();
        let mut bytes = [0x55; 8];
        let buffers = if selected {
            BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap()
        } else {
            BufferList::from_iter(
                [
                    Buffer::Read(&mut bytes),
                    Buffer::Write(b"unchanged"),
                    Buffer::Write(b"untouched"),
                ]
                .into_iter(),
            )
            .unwrap()
        };
        let indices = selection(&[1]);
        let (result, allocations) = crate::probe_runtime_allocations(|| {
            let driver = ScriptDriver::new(&ledger);
            let control = ScriptControl::new(&[]);
            if selected {
                TransferGroup::begin_selected(&mut slots, buffers, driver, control, indices)
            } else {
                TransferGroup::begin(&mut slots, buffers, driver, control)
            }
        });
        assert!(result.is_err());
        drop(result);
        assert_eq!(allocations, 0);
        assert_eq!(bytes, [0x55; 8]);
        assert_eq!(ledger.borrow().events[0], Some(Event::ResourcesReleased));
        assert_eq!(ledger.borrow().used, 1);
    }
}

#[test]
fn sparse_selected_group_reuses_live_requests_without_hot_path_allocation() {
    let ledger = Ledger::new();
    let mut slots = RequestSlots::dynamic().unwrap();
    for key in [10, 20, 30] {
        slots.add(key, TestRequest::new()).unwrap();
    }
    slots.remove(1).unwrap();
    let addresses = [slots.raw(0), slots.raw(2)];
    let mut driver = ScriptDriver::new(&ledger);
    driver.query_script(
        2,
        &[QueryValue::Terminal(Ok(2)), QueryValue::Terminal(Ok(1))],
    );
    driver.query_script(0, &[QueryValue::Terminal(Ok(3))]);
    let mut bytes = [0x55; 8];
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut bytes), Buffer::Write(b"hello")].into_iter())
            .unwrap();
    let indices = selection(&[2, 0]);
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        let mut group = TransferGroup::begin_selected(
            &mut slots,
            buffers,
            driver,
            ScriptControl::new(&[ControlStep::Wait(100), ControlStep::Wait(90)]),
            indices,
        )
        .unwrap();
        let first = group.next().unwrap();
        assert_eq!(
            (first.index, first.key, first.outcome.transfer),
            (0, 30, Some(Ok(2)))
        );
        assert_eq!(group.completed(0), Some(&b"CC"[..]));
        group.submit(0, 4, 1).unwrap();
        let other = group.next().unwrap();
        assert_eq!(
            (other.index, other.key, other.outcome.transfer),
            (1, 10, Some(Ok(3)))
        );
        let reused = group.next().unwrap();
        assert_eq!(
            (reused.index, reused.key, reused.outcome.transfer),
            (0, 30, Some(Ok(1)))
        );
        assert_eq!(group.completed(0), Some(&b"C"[..]));
        assert!(group.next().is_none());
        drop(group);
    });
    assert_eq!(allocations, 0);
    assert_eq!([slots.raw(0), slots.raw(2)], addresses);
    assert_eq!((slots.len(), slots.occupied_len()), (3, 2));
    let ledger = ledger.borrow();
    assert_eq!(ledger.rows[1].generation, 0);
    assert_eq!(ledger.rows[2].generation, 2);
    assert_eq!(ledger.mask(), 0);
}

#[test]
fn competing_requests_follow_plan_not_slot_order_and_reuse_joins_the_queue_without_allocation() {
    let ledger = Ledger::new();
    let mut slots = request_slots(&[10, 20, 30]).unwrap();
    let addresses = [slots.raw(0), slots.raw(1), slots.raw(2)];
    let mut driver = ScriptDriver::new(&ledger);
    driver.resources.fill(TransferResource {
        identity: 77,
        shared_cursor: false,
    });
    for index in 0..3 {
        driver.query_script(index, &[QueryValue::Terminal(Ok(1)); 2]);
    }
    let buffers = BufferList::from_iter(
        [
            Buffer::Write(b"a"),
            Buffer::Write(b"b"),
            Buffer::Write(b"c"),
        ]
        .into_iter(),
    )
    .unwrap();
    let indices = selection(&[2, 0, 1]);
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        let mut group = TransferGroup::begin_selected(
            &mut slots,
            buffers,
            driver,
            ScriptControl::new(&[ControlStep::Wait(100); STEPS]),
            indices,
        )
        .unwrap();
        assert_eq!(ledger.borrow().mask(), 4);
        assert_eq!(group.next().unwrap().index, 0);
        assert_eq!(ledger.borrow().mask(), 1);
        group.submit(0, 0, 1).unwrap();
        assert_eq!(
            ledger.borrow().mask(),
            1,
            "reuse cannot overtake queued plan entries"
        );
        assert_eq!(group.next().unwrap().index, 1);
        assert_eq!(group.next().unwrap().index, 2);
        assert_eq!(group.next().unwrap().index, 0);
        assert!(group.next().is_none());
    });
    assert_eq!(allocations, 0);
    assert_eq!([slots.raw(0), slots.raw(1), slots.raw(2)], addresses);
    let ledger = ledger.borrow();
    let mut submitted = [usize::MAX; 4];
    let mut count = 0;
    for event in ledger.events[..ledger.used].iter().flatten() {
        if let Event::Submit(index, ..) = event {
            submitted[count] = *index;
            count += 1;
        }
    }
    assert_eq!(submitted, [2, 0, 1, 2]);
}

#[test]
fn cancellation_never_submits_queries_cancels_or_drains_an_unprepared_queued_request() {
    let ledger = Ledger::new();
    let mut slots = request_slots(&[10, 20, 30]).unwrap();
    let mut driver = ScriptDriver::new(&ledger);
    driver.resources.fill(TransferResource {
        identity: 77,
        shared_cursor: false,
    });
    let failure = Failure::from_system_code(123).with_transferred_byte_count(Some(1));
    driver.drains[0] = Err(failure);
    let buffers = BufferList::from_iter(
        [
            Buffer::Write(b"a"),
            Buffer::Write(b"b"),
            Buffer::Write(b"c"),
        ]
        .into_iter(),
    )
    .unwrap();
    let mut group = TransferGroup::begin(
        &mut slots,
        buffers,
        driver,
        ScriptControl::new(&[
            ControlStep::Wait(100),
            ControlStep::Stop(Stop::Cancellation),
        ]),
    )
    .unwrap();
    assert_eq!(ledger.borrow().mask(), 1);
    assert!(group.read_storage_mut(1).is_none());
    assert!(group.submit(1, 0, 1).is_err());
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        let first = group.next().unwrap();
        assert_eq!(first.outcome.transfer, Some(Err(failure)));
        for index in 1..3 {
            let next = group.next().unwrap();
            assert_eq!(next.index, index);
            assert_eq!(next.outcome.transfer, None);
            assert_eq!(next.outcome.stop, Some(Stop::Cancellation));
            assert_eq!(group.completed(index), None);
        }
        assert!(group.next().is_none());
        drop(group);
    });
    assert_eq!(allocations, 0);
    let ledger = ledger.borrow();
    assert_eq!(ledger.count(|event| matches!(event, Event::Submit(..))), 1);
    assert_eq!(ledger.count(|event| matches!(event, Event::Query(_))), 1);
    assert_eq!(ledger.count(|event| matches!(event, Event::Cancel(_))), 1);
    assert_eq!(ledger.count(|event| matches!(event, Event::Drain(_))), 1);
}

#[test]
fn duplex_directions_overlap_but_shared_cursor_directions_and_queued_failures_are_ordered() {
    for shared_cursor in [false, true] {
        let ledger = Ledger::new();
        let mut slots = request_slots(&[10, 20]).unwrap();
        let mut driver = ScriptDriver::new(&ledger);
        driver.resources.fill(TransferResource {
            identity: 77,
            shared_cursor,
        });
        let mut input = [0; 1];
        let buffers =
            BufferList::from_iter([Buffer::Read(&mut input), Buffer::Write(b"w")].into_iter())
                .unwrap();
        let group = TransferGroup::begin(
            &mut slots,
            buffers,
            driver,
            ScriptControl::new(&[ControlStep::Wait(100)]),
        )
        .unwrap();
        assert_eq!(ledger.borrow().mask(), if shared_cursor { 1 } else { 3 });
        drop(group);
        assert_eq!(ledger.borrow().mask(), 0);
    }

    let ledger = Ledger::new();
    let mut slots = request_slots(&[10, 20]).unwrap();
    let mut driver = ScriptDriver::new(&ledger);
    driver.resources.fill(TransferResource {
        identity: 77,
        shared_cursor: true,
    });
    let failure = Failure::from_system_code(321).with_transferred_byte_count(None);
    driver.submits[0][0] = Some(SubmitStep::Failed(failure));
    driver.query_script(1, &[QueryValue::Terminal(Ok(1))]);
    let buffers =
        BufferList::from_iter([Buffer::Write(b"a"), Buffer::Write(b"b")].into_iter()).unwrap();
    let mut group = TransferGroup::begin(
        &mut slots,
        buffers,
        driver,
        ScriptControl::new(&[ControlStep::Wait(100); STEPS]),
    )
    .unwrap();
    assert_eq!(group.next().unwrap().outcome.transfer, Some(Err(failure)));
    assert_eq!(group.next().unwrap().outcome.transfer, Some(Ok(1)));
    assert!(group.next().is_none());
}
