use super::*;

#[test]
fn control_owns_the_frozen_budget_after_caller_storage_changes_and_expires() {
    for (kind, nanos) in [(0, 0), (1, 0), (2, u64::MAX), (2, 5_000_000_000)] {
        let (mut control, allocations) = crate::probe_runtime_allocations(|| {
            let mut original = budget(kind, nanos);
            let control = WaitControl::new(&original, ptr::null());
            let expected = original.remaining(1, u64::MAX, || Ok(0));
            original = budget(0, 0);
            assert_eq!(original.remaining(1, u64::MAX, || Ok(0)), Ok((0, 0)));
            assert_eq!(control.budget.remaining(1, u64::MAX, || Ok(0)), expected);
            control
        });
        assert_eq!(allocations, 0);
        let next = control.next_wait().unwrap();
        if kind == 2 && nanos == u64::MAX {
            assert!(matches!(next, Err(Stop::BudgetExhausted)));
        } else {
            assert_eq!(next.unwrap().immediate, kind == 0);
        }
    }
}

mod inherited_mode_tests {
    use super::*;

    #[test]
    fn inherited_nonblocking_mode_is_not_a_driver_thread_safety_proof() {
        use std::io::{Read, Write};
        use std::process::{Command, Stdio};
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "unix_readiness::transfer::tests::inherited_mode_tests::inherited_mode_child",
                "--nocapture",
            ])
            .env("DORIA_RT_TEST_FOREIGN_MODE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut control = child.stderr.take().unwrap();
        let (send, receive) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut bytes = std::vec::Vec::new();
            let mut byte = [0];
            while control.read_exact(&mut byte).is_ok() {
                bytes.push(byte[0]);
                let _ = send.send(byte[0]);
            }
            bytes
        });
        let ready = receive.recv_timeout(std::time::Duration::from_secs(10));
        if ready != Ok(b'R') {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "driver blocked on inherited mode: {ready:?}; stdout={:?}; stderr={:?}",
                output.stdout,
                reader.join().unwrap()
            );
        }
        child.stdin.as_mut().unwrap().write_all(b"x").unwrap();
        drop(child.stdin.take());
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "stdout={:?}; stderr={:?}",
            output.stdout,
            reader.join().unwrap()
        );
    }

    #[test]
    fn inherited_mode_child() {
        if std::env::var("DORIA_RT_TEST_FOREIGN_MODE").as_deref() != Ok("1") {
            return;
        }
        unsafe {
            let _input = crate::device_io::TestInput::new(b"", false);
            let original = libc::fcntl(0, libc::F_GETFL);
            assert!(original >= 0);
            // This fixture is the foreign owner; production never changes flags.
            assert_eq!(
                libc::fcntl(0, libc::F_SETFL, original | libc::O_NONBLOCK),
                0
            );
            let indefinite = DrWaitBudgetV1::indefinite();
            let mut set = TransferSet::borrowed_new(ptr::null_mut()).unwrap();
            let index = set
                .add_standard(1, crate::device_io::StandardStream::Stdin)
                .unwrap();
            let request = set.slots.raw(index);
            let (_, mut driver, _) = set.driver_parts(&indefinite).unwrap();
            driver.acquire_request(index, request).unwrap();
            driver.reset_request(index, request);
            let mut bytes = [0xa5; 2];
            assert!(matches!(
                driver.submit(index, request, &mut Buffer::Read(&mut bytes)),
                Submission::Pending
            ));
            assert_eq!(driver.wait_any(indefinite, None), Ok(Wake::Completion));
            assert!(matches!(driver.query(index, request), Query::Pending));
            assert!(
                !(*request).helper_active && (*request).watched,
                "would-block re-arms native readiness without completing the transfer"
            );
            let starts = crate::device_io::helper::starts_for_test();
            // An inherited alias can revert the mode after that would-block.
            assert_eq!(
                libc::fcntl(0, libc::F_SETFL, original & !libc::O_NONBLOCK),
                0
            );
            assert!(matches!(driver.query(index, request), Query::Pending));
            assert!((*request).helper_active);
            assert_eq!(crate::device_io::helper::starts_for_test(), starts);
            assert_eq!(
                byte_io::write(2, b"R".as_ptr(), 1),
                Ok(WriteProgress::Accepted(1))
            );
            loop {
                assert_eq!(driver.wait_any(indefinite, None), Ok(Wake::Completion));
                match driver.query(index, request) {
                    Query::Terminal(result) => {
                        assert_eq!(result, Ok(1));
                        break;
                    }
                    Query::Pending => {}
                    Query::PendingFailure(_, failure) => {
                        panic!("lost inherited transfer: {failure:?}")
                    }
                }
            }
            assert_eq!(bytes, [b'x', 0xa5]);
            driver.release_request(index, request);
            assert!(
                crate::device_io::standard_device(crate::device_io::StandardStream::Stdin)
                    .try_borrow()
                    .is_none(),
                "the group, not an operation, owns the device claim"
            );
            drop(driver);
            assert!(
                crate::device_io::standard_device(crate::device_io::StandardStream::Stdin)
                    .try_borrow()
                    .is_some()
            );
            assert_eq!(libc::fcntl(0, libc::F_GETFL), original & !libc::O_NONBLOCK);
            assert_eq!(libc::fcntl(0, libc::F_SETFL, original), 0);
        }
    }
}

#[test]
fn file_terminal_cache_preserves_would_block_eof_zero_data_and_actual_failure_facts() {
    // Deterministic projection coverage, not evidence that this host produced
    // WouldBlock from a real regular-file syscall.
    let would_block = Failure::other().with_transferred_byte_count(Some(0));
    for terminal in [
        FileTerminal::Read(Ok(ReadProgress::WouldBlock(would_block))),
        FileTerminal::Write(Ok(WriteProgress::WouldBlock(would_block))),
    ] {
        assert_eq!(terminal.result(), Ok(0));
        let (tag, status) = terminal.status();
        assert_eq!(tag, 1);
        assert_eq!(status.value, 0);
        assert_eq!(status.reason, 0);
        assert_eq!(status.has_system_code, 0);
        assert_eq!(status.has_transferred_byte_count, 0);
    }
    for (terminal, expected) in [
        (FileTerminal::Read(Ok(ReadProgress::Data(0))), 0),
        (FileTerminal::Read(Ok(ReadProgress::EndOfStream)), 2),
        (FileTerminal::Write(Ok(WriteProgress::Accepted(0))), 0),
    ] {
        assert_eq!(terminal.result(), Ok(0));
        assert_eq!(terminal.status().0, expected);
    }
    let failure = Failure {
        reason: Reason::PermissionDenied,
        system_code: Some(13),
        transferred_byte_count: None,
    };
    for terminal in [
        FileTerminal::Read(Err(failure)),
        FileTerminal::Write(Err(failure)),
    ] {
        assert_eq!(terminal.result(), Err(failure));
        let (tag, status) = terminal.status();
        assert_eq!(tag, 3);
        assert_eq!(status.reason, failure.reason as u8);
        assert_eq!((status.has_system_code, status.system_code), (1, 13));
        assert_eq!(status.has_transferred_byte_count, 0);
    }
    for terminal in [
        FileTerminal::Read(Ok(ReadProgress::Data(3))),
        FileTerminal::Write(Ok(WriteProgress::Accepted(3))),
        FileTerminal::Read(Err(failure.with_transferred_byte_count(Some(3)))),
    ] {
        let bounded = terminal.result().bounded(2);
        let (tag, status) = terminal.bounded_status(bounded, 2);
        assert_eq!(tag, 3);
        assert_eq!(status.has_transferred_byte_count, 0);
        let failure = bounded.unwrap_err();
        assert_eq!(status.reason, failure.reason as u8);
        assert_eq!(status.system_code, failure.system_code.unwrap_or(0));
    }
}
use crate::transfer_group::transport::plan::*;
use crate::transfer_group::transport::registration::*;
use crate::transfer_group::transport::*;
use crate::transfer_group::PlanKey;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, IntoRawFd};
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{sync_channel, SyncSender};
use std::time::Duration;

struct ObservedDriver<'s> {
    driver: Dispatcher<'s>,
    entered: Option<SyncSender<()>>,
    allowed_indices: Option<&'static [usize]>,
    forbidden_watch_keys: &'static [u64],
}

impl ObservedDriver<'_> {
    fn check_index(&self, index: usize) {
        if let Some(allowed) = self.allowed_indices {
            assert!(
                allowed.contains(&index),
                "unselected registered slot was accessed"
            );
        }
    }

    fn check_watches(&mut self) {
        let forbidden = self.forbidden_watch_keys;
        assert!(
            self.driver
                .watcher
                .slots()
                .iter()
                .all(|entry| { entry.interests == 0 || !forbidden.contains(&entry.key) }),
            "an unselected stream acquired a readiness watch"
        );
    }
}

// Delegates the buffer lifetime and retirement contract to the native driver;
// the observer synchronizes test input only after the first pending wait.
unsafe impl GroupDriver for ObservedDriver<'_> {
    type Request = Request;
    type Timeout = DrWaitBudgetV1;
    const MAXIMUM_TRANSFER_BYTES: usize = isize::MAX as usize;
    const QUERY_CAN_TRANSFER: bool = <Dispatcher<'_> as GroupDriver>::QUERY_CAN_TRANSFER;
    unsafe fn resource(
        &self,
        index: usize,
        request: *mut Request,
    ) -> Result<crate::transfer_group::TransferResource, Failure> {
        self.driver.resource(index, request)
    }
    unsafe fn reset_request(&mut self, index: usize, request: *mut Request) {
        self.check_index(index);
        self.driver.reset_request(index, request);
    }
    unsafe fn submit(
        &mut self,
        index: usize,
        request: *mut Request,
        buffer: &mut Buffer<'_>,
    ) -> Submission {
        self.check_index(index);
        let submission = self.driver.submit(index, request, buffer);
        self.check_watches();
        submission
    }
    unsafe fn query(&mut self, index: usize, request: *mut Request) -> Query {
        self.check_index(index);
        let query = self.driver.query(index, request);
        self.check_watches();
        query
    }
    fn wait_any(&mut self, budget: Self::Timeout, signal: Option<usize>) -> Result<Wake, Failure> {
        self.check_watches();
        if let Some(entered) = self.entered.take() {
            entered.send(()).unwrap();
        }
        self.driver.wait_any(budget, signal)
    }
    unsafe fn cancel(
        &mut self,
        index: usize,
        request: *mut Request,
    ) -> Result<(), (doria_diagnostic_catalogue::IoMessageOperation, Failure)> {
        self.check_index(index);
        let result = self.driver.cancel(index, request);
        self.check_watches();
        result
    }
    unsafe fn drain(
        &mut self,
        index: usize,
        request: *mut Request,
    ) -> Option<Result<usize, Failure>> {
        self.check_index(index);
        let result = self.driver.drain(index, request);
        self.check_watches();
        result
    }
}

fn observed_begin<'s, 'b>(
    set: &'s mut TransferSet,
    buffers: BufferList<'b>,
    budget: &DrWaitBudgetV1,
    entered: SyncSender<()>,
) -> TransferGroup<'s, 'b, ObservedDriver<'s>, WaitControl> {
    let state = set.watcher.cancellation;
    let cancellation = unsafe { GroupCancellationWatch::install(&mut set.watcher, state).unwrap() };
    let control = WaitControl {
        budget: *budget,
        cancellation: cancellation.state(),
    };
    let driver = ObservedDriver {
        driver: Dispatcher {
            _owners: set.owners.as_slice(),
            watcher: &mut set.watcher,
            events: set.events.as_mut_slice(),
            registrations: &mut set.registrations,
            sources: &mut set.sources,
            devices: [None, None, None],
            cancellation,
        },
        entered: Some(entered),
        allowed_indices: None,
        forbidden_watch_keys: &[],
    };
    TransferGroup::begin(&mut set.slots, buffers, driver, control).unwrap()
}

fn observed_begin_selected<'s, 'b>(
    set: &'s mut TransferSet,
    buffers: BufferList<'b>,
    budget: &DrWaitBudgetV1,
    selected: &'static [usize],
    forbidden_watch_keys: &'static [u64],
    entered: SyncSender<()>,
) -> TransferGroup<'s, 'b, ObservedDriver<'s>, WaitControl> {
    let selection = selection(selected);
    let state = set.watcher.cancellation;
    let cancellation = unsafe { GroupCancellationWatch::install(&mut set.watcher, state).unwrap() };
    let control = WaitControl {
        budget: *budget,
        cancellation: cancellation.state(),
    };
    let driver = ObservedDriver {
        driver: Dispatcher {
            _owners: set.owners.as_slice(),
            watcher: &mut set.watcher,
            events: set.events.as_mut_slice(),
            registrations: &mut set.registrations,
            sources: &mut set.sources,
            devices: [None, None, None],
            cancellation,
        },
        entered: Some(entered),
        allowed_indices: Some(selected),
        forbidden_watch_keys,
    };
    TransferGroup::begin_selected(&mut set.slots, buffers, driver, control, selection).unwrap()
}

fn selection(indices: &[usize]) -> FixedSlice<usize> {
    let mut selected = FixedSlice::new(indices.len()).unwrap();
    for index in indices {
        selected.push(*index).unwrap();
    }
    selected
}

#[test]
fn many_streams_reuse_one_group_buffers_requests_and_watcher_without_hot_allocations() {
    const COUNT: usize = 96;
    const ROUNDS: u8 = 32;
    let mut streams = Vec::with_capacity(COUNT);
    let mut peers = Vec::with_capacity(COUNT);
    for _ in 0..COUNT {
        let (stream, peer) = UnixStream::pair().unwrap();
        streams.push(stream);
        peers.push(peer);
    }
    let keys = std::array::from_fn::<_, COUNT, _>(|index| 1000 + index as u64);
    let mut set = set(&keys, streams.into_iter());
    let requests = std::array::from_fn::<_, COUNT, _>(|index| set.slots.raw(index));
    let watcher = (set.watcher.handle().unwrap(), set.watcher.memory);
    let events = set.events.as_slice().as_ptr();
    let mut storage = [[0xa5; 8]; COUNT];
    let addresses = std::array::from_fn::<_, COUNT, _>(|index| storage[index].as_ptr());
    let buffers = BufferList::from_iter(
        storage
            .iter_mut()
            .map(|bytes| Buffer::Read(bytes.as_mut_slice())),
    )
    .unwrap();
    let wait = budget(2, 5_000_000_000);
    let mut group = set.begin(buffers, &wait).unwrap();
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        for round in 0..ROUNDS {
            for (index, peer) in peers.iter_mut().enumerate() {
                assert!(group.completed(index).is_none());
                assert!(group.read_storage_mut(index).is_none());
                assert_eq!(
                    unsafe { (*requests[index]).data },
                    addresses[index].cast_mut()
                );
                peer.write_all(&[index as u8, round, 0, 0xff]).unwrap();
            }
            let mut seen = [false; COUNT];
            while let Some(completion) = group.next() {
                let index = completion.index;
                assert!(index < COUNT && !seen[index]);
                seen[index] = true;
                assert_eq!(completion.key, keys[index]);
                assert_eq!(completion.outcome.transfer, Some(Ok(4)));
                assert_eq!(completion.outcome.stop, None);
                assert_eq!(completion.outcome.cancellation_failure, None);
                assert_eq!(
                    group.completed(index).unwrap(),
                    &[index as u8, round, 0, 0xff]
                );
                let bytes = group.read_storage_mut(index).unwrap();
                assert_eq!(bytes.as_ptr(), addresses[index]);
                assert_eq!(bytes[4..], [0xa5; 4]);
                assert!(unsafe { (*requests[index]).data }.is_null());
            }
            assert_eq!(seen, [true; COUNT]);
            assert_eq!(group.stop(), None);
            if round + 1 < ROUNDS {
                for index in 0..COUNT {
                    group.submit(index, 0, 8).unwrap();
                }
            }
        }
    });
    assert_eq!(allocations, 0, "runtime storage, not kernel resources");
    let ((), closes) = crate::native_handle::with_close_probe(|| drop(group));
    assert_eq!(closes, 0, "retiring a group must not close its streams");
    assert_eq!(
        std::array::from_fn::<_, COUNT, _>(|index| set.slots.raw(index)),
        requests
    );
    assert_eq!((set.watcher.handle().unwrap(), set.watcher.memory), watcher);
    assert_eq!(set.events.as_slice().as_ptr(), events);
    assert_no_watches(&mut set);
    for (index, bytes) in storage.iter().enumerate() {
        assert_eq!(&bytes[..4], &[index as u8, ROUNDS - 1, 0, 0xff]);
        assert_eq!(bytes[4..], [0xa5; 4]);
    }
    let ((), closes) = crate::native_handle::with_close_probe(|| drop(set));
    assert_eq!(
        closes,
        COUNT + 1,
        "one owned stream per registration and one poller"
    );
}

#[test]
fn one_registration_duplex_operations_keep_independent_buffers_and_reuse_capacity() {
    let (stream, mut peer) = UnixStream::pair().unwrap();
    let mut set = set(&[42], [stream].into_iter());
    let mut first_addresses = None;
    for _ in 0..3 {
        let mut input = [0xa5; 4];
        let original = input.as_ptr();
        let buffers =
            BufferList::from_iter([Buffer::Read(&mut input), Buffer::Write(b"ping")].into_iter())
                .unwrap();
        let mut group = {
            let wait = budget(1, 0);
            set.begin_selected(buffers, &wait, selection(&[0, 0]))
                .unwrap()
        };
        let (written, allocations) = crate::probe_runtime_allocations(|| group.next().unwrap());
        assert_eq!(allocations, 0);
        assert_eq!(
            (written.key, written.index, written.outcome.transfer),
            (42, 1, Some(Ok(4)))
        );
        let mut actual = [0; 4];
        peer.read_exact(&mut actual).unwrap();
        assert_eq!(&actual, b"ping");
        assert!(group.completed(0).is_none());
        let ((), allocations) = crate::probe_runtime_allocations(|| {
            group.submit(1, 0, 4).unwrap();
        });
        assert_eq!(allocations, 0);
        peer.read_exact(&mut actual).unwrap();
        assert_eq!(&actual, b"ping");
        peer.write_all(b"pong").unwrap();
        let mut seen = [false; 2];
        let ((), allocations) = crate::probe_runtime_allocations(|| {
            while let Some(completion) = group.next() {
                assert_eq!(completion.key, 42);
                assert_eq!(completion.outcome.transfer, Some(Ok(4)));
                assert!(!seen[completion.index]);
                seen[completion.index] = true;
            }
        });
        assert_eq!(allocations, 0);
        assert_eq!(seen, [true, true]);
        assert_eq!(group.completed(0).unwrap(), b"pong");
        assert_eq!(group.read_storage_mut(0).unwrap().as_ptr(), original);
        drop(group);
        assert_no_watches(&mut set);
        let addresses = [
            set.slots.raw(0),
            set.slots.raw(set.cached_operation(0, 1).unwrap()),
        ];
        if let Some(first) = first_addresses {
            assert_eq!(addresses, first);
        } else {
            first_addresses = Some(addresses);
        }
        assert_ne!(addresses[0], addresses[1]);
        assert_eq!(&input, b"pong");
    }
}

#[test]
fn arbitrary_same_registration_operations_conserve_identity_without_asserting_order() {
    let (stream, mut peer) = UnixStream::pair().unwrap();
    let mut set = set(&[71], [stream].into_iter());
    let payloads = [[6], [2], [5], [1], [4], [3]];
    let buffers = BufferList::from_iter(payloads.iter().map(|bytes| Buffer::Write(bytes))).unwrap();
    let wait = budget(1, 0);
    let mut group = set
        .begin_selected(buffers, &wait, selection(&[0; 6]))
        .unwrap();
    let mut seen = [false; 6];
    while let Some(completion) = group.next() {
        assert_eq!(completion.key, 71);
        assert_eq!(completion.outcome.transfer, Some(Ok(1)));
        assert!(!seen[completion.index]);
        seen[completion.index] = true;
    }
    assert!(seen.into_iter().all(|present| present));
    drop(group);
    let mut actual = [0; 6];
    peer.read_exact(&mut actual).unwrap();
    actual.sort_unstable();
    assert_eq!(actual, [1, 2, 3, 4, 5, 6]);
}

#[test]
fn competing_reads_and_writes_assign_bytes_in_plan_order_without_hot_allocations() {
    let (stream, mut peer) = UnixStream::pair().unwrap();
    let mut set = set(&[71], [stream].into_iter());
    let (mut a, mut b) = ([0xa5; 3], [0xb5; 3]);
    let buffers = BufferList::from_iter(
        [
            Buffer::Read(&mut a),
            Buffer::Read(&mut b),
            Buffer::Write(b"XY"),
            Buffer::Write(b"ZW"),
        ]
        .into_iter(),
    )
    .unwrap();
    let wait = budget(1, 0);
    let mut group = set
        .begin_selected(buffers, &wait, selection(&[0; 4]))
        .unwrap();
    assert!(group.completed(1).is_none());
    assert!(group.read_storage_mut(1).is_none());
    peer.write_all(b"abcdef").unwrap();
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        let mut seen = [false; 4];
        while let Some(completion) = group.next() {
            assert!(!seen[completion.index]);
            seen[completion.index] = true;
            assert_eq!(
                completion.outcome.transfer,
                Some(Ok(if completion.index < 2 { 3 } else { 2 }))
            );
        }
        assert_eq!(seen, [true; 4]);
        assert_eq!(group.completed(0).unwrap(), b"abc");
        assert_eq!(group.completed(1).unwrap(), b"def");
        group.submit(0, 0, 3).unwrap();
        group.submit(1, 0, 3).unwrap();
    });
    assert_eq!(allocations, 0);
    let mut output = [0; 4];
    peer.read_exact(&mut output).unwrap();
    assert_eq!(&output, b"XYZW");
    peer.write_all(b"ghijkl").unwrap();
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        for _ in 0..2 {
            assert_eq!(group.next().unwrap().outcome.transfer, Some(Ok(3)));
        }
        assert_eq!(group.completed(0).unwrap(), b"ghi");
        assert_eq!(group.completed(1).unwrap(), b"jkl");
        assert!(group.next().is_none());
        drop(group);
    });
    assert_eq!(allocations, 0);
    assert_eq!(a, *b"ghi");
    assert_eq!(b, *b"jkl");
}

fn assert_unsubmitted(set: &TransferSet, index: usize) {
    // The group has ended, and Unix never leaves a kernel-owned buffer record.
    let request = unsafe { &*set.slots.raw(index) };
    assert!(request.data.is_null());
    assert_eq!(request.length, 0);
    assert!(!request.reading);
    assert!(!request.watched);
    assert_eq!(request.terminal, None);
    assert_eq!(request.control_failure, None);
}

fn assert_no_watches(set: &mut TransferSet) {
    assert!(set.watcher.slots().iter().all(|entry| entry.interests == 0));
}

fn budget(kind: u8, nanoseconds: u64) -> DrWaitBudgetV1 {
    let mut value = mem::MaybeUninit::uninit();
    let (mut reason, mut code, mut has_code) = (0, 0, 0);
    assert_eq!(
        unsafe {
            crate::wait_budget::dr_v1_wait_budget_init(
                kind,
                nanoseconds,
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

fn set(keys: &[u64], streams: impl ExactSizeIterator<Item = UnixStream>) -> TransferSet {
    set_with_cancellation(keys, streams, ptr::null_mut())
}

fn set_with_cancellation(
    keys: &[u64],
    streams: impl ExactSizeIterator<Item = UnixStream>,
    cancellation: *mut DrCancellationV1,
) -> TransferSet {
    let mut owners = FixedSlice::new(streams.len()).unwrap();
    for stream in streams {
        stream.set_nonblocking(true).unwrap();
        owners
            .push(unsafe { OwnedHandle::from_raw(stream.into_raw_fd()) })
            .unwrap();
    }
    unsafe { TransferSet::new(keys, owners, cancellation).unwrap() }
}

#[test]
fn simultaneous_read_write_and_reuse_keep_request_addresses_and_no_hot_path_allocations() {
    let (a, mut a_peer) = UnixStream::pair().unwrap();
    let (b, mut b_peer) = UnixStream::pair().unwrap();
    let (c, _c_peer) = UnixStream::pair().unwrap();
    let mut set = set(&[10, 20, 30], [a, b, c].into_iter());
    let addresses = [set.slots.raw(0), set.slots.raw(1), set.slots.raw(2)];
    let watcher = set.watcher.handle().unwrap();
    let poll = budget(0, 0);
    let mut a_bytes = [0x55; 8];
    let mut c_bytes = [0x77; 8];
    let buffers = BufferList::from_iter(
        [
            Buffer::Read(&mut a_bytes),
            Buffer::Write(b"hello"),
            Buffer::Read(&mut c_bytes),
        ]
        .into_iter(),
    )
    .unwrap();
    let mut group = set.begin(buffers, &poll).unwrap();
    assert!(group.completed(0).is_none());
    assert!(group.read_storage_mut(2).is_none());
    let first = group.next().unwrap();
    assert_eq!(
        (first.key, first.index, first.outcome.transfer),
        (20, 1, Some(Ok(5)))
    );
    b_peer.read_exact(&mut [0; 5]).unwrap();
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        for _ in 0..8 {
            a_peer.write_all(b"abc").unwrap();
            let complete = group.next().unwrap();
            assert_eq!((complete.key, complete.index), (10, 0));
            assert_eq!(complete.outcome.transfer, Some(Ok(3)));
            assert_eq!(group.completed(0).unwrap(), b"abc");
            assert_eq!(group.read_storage_mut(0).unwrap()[3..], [0x55; 5]);
            group.submit(0, 0, 8).unwrap();
        }
        assert!(group.read_storage_mut(0).is_none());
        assert!(group.read_storage_mut(2).is_none());
        // Drop cancels the two pending readiness loans, without closing streams.
        drop(group);
    });
    assert_eq!(allocations, 0);
    assert_eq!(
        [set.slots.raw(0), set.slots.raw(1), set.slots.raw(2)],
        addresses
    );
    assert_eq!(set.watcher.handle().unwrap(), watcher);
    assert!(set.watcher.slots().iter().all(|entry| entry.interests == 0));
    a_peer.write_all(b"next").unwrap();
    let buffers = BufferList::from_iter(
        [
            Buffer::Read(&mut a_bytes),
            Buffer::Write(&[]),
            Buffer::Read(&mut []),
        ]
        .into_iter(),
    )
    .unwrap();
    let mut group = set.begin(buffers, &poll).unwrap();
    let mut seen = [false; 3];
    for _ in 0..3 {
        let completion = group.next().unwrap();
        assert!(!seen[completion.index]);
        seen[completion.index] = true;
        assert_eq!(
            completion.outcome.transfer,
            Some(Ok(if completion.index == 0 { 4 } else { 0 }))
        );
    }
    assert_eq!(seen, [true; 3]);
    assert_eq!(group.completed(0).unwrap(), b"next");
}

#[test]
fn reversed_noncontiguous_subset_preserves_plan_indices_stable_slots_and_unused_bytes() {
    let (a, mut a_peer) = UnixStream::pair().unwrap();
    let (b, mut b_peer) = UnixStream::pair().unwrap();
    let (c, mut c_peer) = UnixStream::pair().unwrap();
    let (d, mut d_peer) = UnixStream::pair().unwrap();
    let keys = [10, 20, 30, 40];
    let mut set = set(&keys, [a, b, c, d].into_iter());
    let addresses = std::array::from_fn::<_, 4, _>(|index| set.slots.raw(index));
    let watcher = set.watcher.handle().unwrap();
    for (index, key) in keys.into_iter().enumerate() {
        assert_eq!(set.index_of(key), Some(index));
        assert_eq!(set.key(index), Some(key));
    }
    assert_eq!(set.index_of(u64::MAX), None);
    assert_eq!(set.key(4), None);
    assert_eq!(set.key(usize::MAX), None);
    a_peer.write_all(b"hold").unwrap();
    c_peer.write_all(b"keep").unwrap();
    let wait = budget(2, 5_000_000_000);
    let (mut d_bytes, mut b_bytes) = ([0x55; 8], [0x66; 8]);
    let d_address = d_bytes.as_ptr();
    let b_address = b_bytes.as_ptr();
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut d_bytes), Buffer::Read(&mut b_bytes)].into_iter())
            .unwrap();
    let (entered, waiting) = sync_channel(0);
    let mut group = observed_begin_selected(&mut set, buffers, &wait, &[3, 1], &[10, 30], entered);
    assert!(group.completed(0).is_none());
    assert!(group.completed(1).is_none());
    assert!(group.read_storage_mut(0).is_none());
    assert!(group.read_storage_mut(1).is_none());
    std::thread::scope(|scope| {
        let writer = &mut d_peer;
        scope.spawn(move || {
            waiting.recv_timeout(Duration::from_secs(5)).unwrap();
            writer.write_all(b"one").unwrap();
        });
        let complete = group.next().unwrap();
        assert_eq!((complete.index, complete.key), (0, 40));
        assert_eq!(complete.outcome.transfer, Some(Ok(3)));
        assert_eq!(complete.outcome.stop, None);
        assert_eq!(group.completed(0), Some(b"one".as_slice()));
        assert!(group.completed(1).is_none());
        assert!(group.read_storage_mut(1).is_none());
    });
    let storage = group.read_storage_mut(0).unwrap();
    assert_eq!(storage.as_ptr(), d_address);
    storage.fill(0x33);
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        for _ in 0..8 {
            group.submit(0, 1, 3).unwrap();
            assert!(group.read_storage_mut(0).is_none());
            assert!(group.read_storage_mut(1).is_none());
            d_peer.write_all(b"two").unwrap();
            let complete = group.next().unwrap();
            assert_eq!((complete.index, complete.key), (0, 40));
            assert_eq!(complete.outcome.transfer, Some(Ok(3)));
            assert_eq!(complete.outcome.stop, None);
            assert_eq!(group.completed(0), Some(b"two".as_slice()));
            let storage = group.read_storage_mut(0).unwrap();
            assert_eq!(storage.as_ptr(), d_address);
            assert_eq!(storage[0], 0x33);
            assert_eq!(storage[4..], [0x33; 4]);
        }
    });
    assert_eq!(
        allocations, 0,
        "resubmission and completion allocate no runtime storage"
    );
    b_peer.write_all(b"end").unwrap();
    let complete = group.next().unwrap();
    assert_eq!((complete.index, complete.key), (1, 20));
    assert_eq!(complete.outcome.transfer, Some(Ok(3)));
    assert_eq!(group.completed(1), Some(b"end".as_slice()));
    assert_eq!(group.read_storage_mut(1).unwrap().as_ptr(), b_address);
    assert!(group.next().is_none());
    assert_eq!(group.stop(), None);
    let ((), closes) = crate::native_handle::with_close_probe(|| drop(group));
    assert_eq!(closes, 0);
    assert_eq!(
        std::array::from_fn::<_, 4, _>(|index| set.slots.raw(index)),
        addresses
    );
    assert_eq!(set.watcher.handle().unwrap(), watcher);
    assert_no_watches(&mut set);
    assert_unsubmitted(&set, 0);
    assert_unsubmitted(&set, 2);
    assert_eq!(d_bytes, [0x33, b't', b'w', b'o', 0x33, 0x33, 0x33, 0x33]);
    assert_eq!(&b_bytes[..3], b"end");
    assert_eq!(&b_bytes[3..], &[0x66; 5]);

    // Full-set reuse reads the bytes already queued on the two unused streams.
    // Empty requests occupy the other registered slots without further I/O.
    let (mut a_bytes, mut c_bytes) = ([0x77; 4], [0x88; 4]);
    let buffers = BufferList::from_iter(
        [
            Buffer::Read(&mut a_bytes),
            Buffer::Read(&mut []),
            Buffer::Read(&mut c_bytes),
            Buffer::Read(&mut []),
        ]
        .into_iter(),
    )
    .unwrap();
    let mut group = set.begin(buffers, &wait).unwrap();
    let mut seen = [false; 4];
    while let Some(complete) = group.next() {
        assert!(!seen[complete.index]);
        seen[complete.index] = true;
        assert_eq!(complete.key, keys[complete.index]);
        assert_eq!(
            complete.outcome.transfer,
            Some(Ok(if complete.index % 2 == 0 { 4 } else { 0 }))
        );
    }
    assert_eq!(seen, [true; 4]);
    assert_eq!(group.completed(0), Some(b"hold".as_slice()));
    assert_eq!(group.completed(2), Some(b"keep".as_slice()));
    drop(group);
    assert_eq!(&a_bytes, b"hold");
    assert_eq!(&c_bytes, b"keep");
    assert_no_watches(&mut set);
}

#[test]
fn dropping_selected_mixed_transfers_retires_only_the_subset_then_reuses_the_full_set() {
    let (a, mut a_peer) = UnixStream::pair().unwrap();
    let (b, mut b_peer) = UnixStream::pair().unwrap();
    let (c, mut c_peer) = UnixStream::pair().unwrap();
    let (d, mut d_peer) = UnixStream::pair().unwrap();
    let keys = [10, 20, 30, 40];
    let mut set = set(&keys, [a, b, c, d].into_iter());
    let addresses = std::array::from_fn::<_, 4, _>(|index| set.slots.raw(index));
    let handles = std::array::from_fn::<_, 4, _>(|index| set.owners.as_slice()[index].get());
    let watcher = set.watcher.handle().unwrap();
    a_peer.write_all(b"hold").unwrap();
    c_peer.write_all(b"keep").unwrap();
    let wait = budget(2, 5_000_000_000);
    let mut d_bytes = [0x55; 4];
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut d_bytes), Buffer::Write(b"out")].into_iter())
            .unwrap();
    let group = set
        .begin_selected(buffers, &wait, selection(&[3, 1]))
        .unwrap();
    let ((), closes) = crate::native_handle::with_close_probe(|| drop(group));
    assert_eq!(closes, 0);
    assert_eq!(d_bytes, [0x55; 4]);
    assert_no_watches(&mut set);
    assert_unsubmitted(&set, 0);
    assert_unsubmitted(&set, 2);
    let mut output = [0; 3];
    b_peer.read_exact(&mut output).unwrap();
    assert_eq!(&output, b"out");
    b_peer.write_all(b"back").unwrap();
    d_peer.write_all(b"done").unwrap();
    let (mut a_bytes, mut b_bytes, mut c_bytes) = ([0x11; 4], [0x22; 4], [0x33; 4]);
    let buffers = BufferList::from_iter(
        [
            Buffer::Read(&mut a_bytes),
            Buffer::Read(&mut b_bytes),
            Buffer::Read(&mut c_bytes),
            Buffer::Read(&mut d_bytes),
        ]
        .into_iter(),
    )
    .unwrap();
    let mut group = set.begin(buffers, &wait).unwrap();
    let payloads: [&[u8]; 4] = [b"hold", b"back", b"keep", b"done"];
    let mut seen = [false; 4];
    while let Some(complete) = group.next() {
        assert!(!seen[complete.index]);
        seen[complete.index] = true;
        assert_eq!(complete.key, keys[complete.index]);
        assert_eq!(complete.outcome.transfer, Some(Ok(4)));
        assert_eq!(complete.outcome.stop, None);
        assert_eq!(
            group.completed(complete.index),
            Some(payloads[complete.index])
        );
    }
    assert_eq!(seen, [true; 4]);
    drop(group);
    assert_eq!(
        std::array::from_fn::<_, 4, _>(|index| set.slots.raw(index)),
        addresses
    );
    assert_eq!(
        std::array::from_fn::<_, 4, _>(|index| set.owners.as_slice()[index].get()),
        handles
    );
    assert_eq!(set.watcher.handle().unwrap(), watcher);
    assert_no_watches(&mut set);
    assert_eq!(&a_bytes, b"hold");
    assert_eq!(&b_bytes, b"back");
    assert_eq!(&c_bytes, b"keep");
    assert_eq!(&d_bytes, b"done");
}

#[test]
fn cancellation_retires_only_selected_reads_and_set_bound_cancellation_remains_persistent() {
    let (a, mut a_peer) = UnixStream::pair().unwrap();
    let (b, _b_peer) = UnixStream::pair().unwrap();
    let (c, mut c_peer) = UnixStream::pair().unwrap();
    let (d, _d_peer) = UnixStream::pair().unwrap();
    let state = cancellation::dr_v1_cancellation_new();
    assert!(!state.is_null());
    let mut set = set_with_cancellation(&[10, 20, 30, 40], [a, b, c, d].into_iter(), state);
    let requester = unsafe { cancellation::dr_v1_cancellation_retain(state) } as usize;
    assert_ne!(requester, 0);
    unsafe { cancellation::dr_v1_cancellation_release(state) };
    a_peer.write_all(b"hold").unwrap();
    c_peer.write_all(b"keep").unwrap();
    let wait = budget(2, 5_000_000_000);
    let (mut d_bytes, mut b_bytes) = ([0x55; 4], [0x66; 4]);
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut d_bytes), Buffer::Read(&mut b_bytes)].into_iter())
            .unwrap();
    let (entered, waiting) = sync_channel(0);
    let mut group = observed_begin_selected(&mut set, buffers, &wait, &[3, 1], &[10, 30], entered);
    std::thread::scope(|scope| {
        scope.spawn(move || {
            let observed = waiting.recv_timeout(Duration::from_secs(5));
            let (mut reason, mut code, mut has_code) = (0, 0, 0);
            let result = unsafe {
                let requester = requester as *mut DrCancellationV1;
                let result = cancellation::dr_v1_cancellation_request(
                    requester,
                    &mut reason,
                    &mut code,
                    &mut has_code,
                );
                cancellation::dr_v1_cancellation_release(requester);
                result
            };
            assert_eq!(result, 0);
            assert!(
                observed.is_ok(),
                "selected group never entered readiness wait"
            );
        });
        let mut seen = [false; 2];
        while let Some(complete) = group.next() {
            assert!(!seen[complete.index]);
            seen[complete.index] = true;
            assert_eq!(complete.key, [40, 20][complete.index]);
            assert_eq!(complete.outcome.stop, Some(Stop::Cancellation));
            assert_eq!(complete.outcome.transfer, None);
            assert_eq!(group.completed(complete.index), None);
            assert!(group.read_storage_mut(complete.index).is_some());
        }
        assert_eq!(seen, [true; 2]);
        assert_eq!(group.stop(), Some(Stop::Cancellation));
    });
    drop(group);
    assert_no_watches(&mut set);
    assert_unsubmitted(&set, 0);
    assert_unsubmitted(&set, 2);
    assert_eq!(d_bytes, [0x55; 4]);
    assert_eq!(b_bytes, [0x66; 4]);

    // Cancellation belongs to the set's retained watcher state and is not reset
    // by a new plan. Reuse returns unsubmitted stopped outcomes, not live I/O.
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut b_bytes), Buffer::Read(&mut d_bytes)].into_iter())
            .unwrap();
    let mut group = set
        .begin_selected(buffers, &wait, selection(&[1, 3]))
        .unwrap();
    let mut seen = [false; 2];
    while let Some(complete) = group.next() {
        assert!(!seen[complete.index]);
        seen[complete.index] = true;
        assert_eq!(complete.key, [20, 40][complete.index]);
        assert_eq!(complete.outcome.transfer, None);
        assert_eq!(complete.outcome.stop, Some(Stop::Cancellation));
    }
    assert_eq!(seen, [true; 2]);
    drop(group);
    assert_no_watches(&mut set);
    assert_unsubmitted(&set, 0);
    assert_unsubmitted(&set, 2);
    // The group has ended; inspect the owned unused streams directly to prove
    // their queued bytes survived both canceled plans. This is not token reset.
    for (index, expected) in [(0, b"hold"), (2, b"keep")] {
        let mut bytes = [0; 4];
        assert!(matches!(
            unsafe {
                byte_io::read_attempt(set.owners.as_slice()[index].get(), bytes.as_mut_ptr(), 4)
            },
            Ok(ReadProgress::Data(4))
        ));
        assert_eq!(&bytes, expected);
    }
}

#[test]
fn invalid_selected_indices_are_rejected_before_any_request_watch_or_byte_transfer() {
    let (a, mut a_peer) = UnixStream::pair().unwrap();
    let (b, mut b_peer) = UnixStream::pair().unwrap();
    let (c, mut c_peer) = UnixStream::pair().unwrap();
    let (d, mut d_peer) = UnixStream::pair().unwrap();
    let keys = [10, 20, 30, 40];
    let mut set = set(&keys, [a, b, c, d].into_iter());
    for peer in [&mut a_peer, &mut b_peer, &mut c_peer, &mut d_peer] {
        peer.write_all(b"keep").unwrap();
    }
    let wait = budget(0, 0);
    // Out-of-range registration and buffer-count mismatch. Repeated valid
    // registrations now configure separate operation slots, not duplicate slots.
    for (indices, count) in [(&[4][..], 1), (&[3, 1][..], 1)] {
        let (mut a_bytes, mut b_bytes) = ([0x55; 4], [0x66; 4]);
        let buffers = BufferList::from_iter(
            [Buffer::Read(&mut a_bytes), Buffer::Read(&mut b_bytes)]
                .into_iter()
                .take(count),
        )
        .unwrap();
        let selected = selection(indices);
        let (failure, allocations) = crate::probe_runtime_allocations(|| {
            set.begin_selected(buffers, &wait, selected).err().unwrap()
        });
        assert_eq!(failure.reason, Reason::InvalidInput);
        assert_eq!(allocations, 0);
        assert_eq!(a_bytes, [0x55; 4]);
        assert_eq!(b_bytes, [0x66; 4]);
        assert_no_watches(&mut set);
        for index in 0..4 {
            assert_unsubmitted(&set, index);
        }
    }
    let (mut a_bytes, mut b_bytes, mut c_bytes, mut d_bytes) = ([0; 4], [0; 4], [0; 4], [0; 4]);
    let buffers = BufferList::from_iter(
        [
            Buffer::Read(&mut a_bytes),
            Buffer::Read(&mut b_bytes),
            Buffer::Read(&mut c_bytes),
            Buffer::Read(&mut d_bytes),
        ]
        .into_iter(),
    )
    .unwrap();
    let mut group = set.begin(buffers, &wait).unwrap();
    let mut seen = [false; 4];
    while let Some(complete) = group.next() {
        assert!(!seen[complete.index]);
        seen[complete.index] = true;
        assert_eq!(complete.key, keys[complete.index]);
        assert_eq!(complete.outcome.transfer, Some(Ok(4)));
        assert_eq!(complete.outcome.stop, None);
        assert_eq!(group.completed(complete.index), Some(b"keep".as_slice()));
    }
    assert_eq!(seen, [true; 4]);
    drop(group);
    for bytes in [a_bytes, b_bytes, c_bytes, d_bytes] {
        assert_eq!(&bytes, b"keep");
    }
    assert_no_watches(&mut set);
}

#[test]
fn failed_watch_registration_reports_infrastructure_without_a_transfer_or_zero_progress() {
    let (stream, mut peer) = UnixStream::pair().unwrap();
    let mut set = set(&[42], [stream].into_iter());
    let mut bytes = [0x55; 8];
    let wait = budget(0, 0);
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let mut group = set
        .begin_with_driver(buffers, &wait, None, |driver| {
            // Admission succeeded; fail the sole native watch before the
            // first would-block attempt can register pending interest.
            drop(driver.watcher.watcher.take());
            driver
        })
        .unwrap();
    let completion = group.next().unwrap();
    assert_eq!((completion.key, completion.index), (42, 0));
    assert_eq!(completion.outcome.transfer, None);
    let Some(Stop::Failure(operation, failure)) = completion.outcome.stop else {
        panic!("registration infrastructure failure was lost")
    };
    assert_eq!(
        operation,
        doria_diagnostic_catalogue::IoMessageOperation::Register
    );
    assert_eq!(failure.reason, Reason::Closed);
    assert_eq!(failure.transferred_byte_count, None);
    assert_eq!(group.completed(0), None);
    let mut status = DrCompletionStatusV1::default();
    assert_eq!(status.project(completion.outcome, true), 4);
    assert_eq!((status.stop, status.wait_operation), (3, operation as u8));
    assert_eq!(status.wait_failure.failure(), Some(failure));
    assert_eq!(status.transfer.value, 0);
    assert_eq!(status.transfer.has_transferred_byte_count, 0);
    assert!(group.next().is_none());
    drop(group);
    assert_eq!(bytes, [0x55; 8]);
    peer.write_all(b"still open").unwrap();
}

#[test]
fn retirement_failure_survives_requery_without_discarding_terminal_progress() {
    let (stream, mut peer) = UnixStream::pair().unwrap();
    let mut set = set(&[42], [stream].into_iter());
    let mut request = Request::new(set.owners.as_slice()[0].get(), 42);
    let mut bytes = [0x55; 8];
    {
        let state = set.watcher.cancellation;
        let cancellation =
            unsafe { GroupCancellationWatch::install(&mut set.watcher, state).unwrap() };
        let mut driver = Dispatcher {
            _owners: set.owners.as_slice(),
            watcher: &mut set.watcher,
            events: set.events.as_mut_slice(),
            registrations: &mut set.registrations,
            sources: &mut set.sources,
            devices: [None, None, None],
            cancellation,
        };
        assert!(matches!(
            unsafe { driver.submit(0, &mut request, &mut Buffer::Read(&mut bytes)) },
            Submission::Pending
        ));
        peer.write_all(b"abc").unwrap();
        // Fault injection affects only the watcher, not the registered stream.
        drop(driver.watcher.watcher.take());
        for _ in 0..2 {
            match unsafe { driver.query(0, &mut request) } {
                Query::PendingFailure(operation, failure) => {
                    assert_eq!(
                        operation,
                        doria_diagnostic_catalogue::IoMessageOperation::Unregister
                    );
                    assert_eq!(failure.reason, Reason::Closed);
                    assert_eq!(failure.transferred_byte_count, None);
                }
                _ => panic!("retirement failure was lost"),
            }
        }
        assert_eq!(unsafe { driver.cancel(0, &mut request) }, Ok(()));
        let terminal = unsafe { driver.drain(0, &mut request) };
        assert_eq!(terminal, Some(Ok(3)));
        let (operation, failure) = request.control_failure.unwrap();
        let mut status = DrCompletionStatusV1::default();
        assert_eq!(
            status.project(
                crate::transfer_group::Awaited {
                    transfer: terminal,
                    stop: Some(Stop::Failure(operation, failure)),
                    cancellation_failure: None,
                },
                true
            ),
            0
        );
        assert_eq!((status.stop, status.wait_operation), (3, operation as u8));
        assert_eq!(status.transfer.value, 3);
        assert_eq!(status.wait_failure.failure(), Some(failure));
        assert_eq!(status.wait_failure.has_transferred_byte_count, 0);
        assert!(request.data.is_null());
    }
    assert_eq!(&bytes[..3], b"abc");
    assert_eq!(&bytes[3..], &[0x55; 5]);
    peer.write_all(b"still open").unwrap();
}

#[test]
fn real_multi_wait_collects_both_reads_without_resetting_the_budget() {
    let (a, mut a_peer) = UnixStream::pair().unwrap();
    let (b, mut b_peer) = UnixStream::pair().unwrap();
    let mut set = set(&[10, 20], [a, b].into_iter());
    let wait = budget(2, 5_000_000_000);
    let (mut a, mut b) = ([0x55; 4], [0x66; 4]);
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut a), Buffer::Read(&mut b)].into_iter()).unwrap();
    let (entered, waiting) = sync_channel(0);
    let mut group = observed_begin(&mut set, buffers, &wait, entered);
    std::thread::scope(|scope| {
        scope.spawn(move || {
            waiting.recv_timeout(Duration::from_secs(5)).unwrap();
            a_peer.write_all(b"abc").unwrap();
            b_peer.write_all(b"xy").unwrap();
        });
        let mut observed = [false; 2];
        for _ in 0..2 {
            let completion = group.next().unwrap();
            observed[completion.index] = true;
            assert_eq!(completion.outcome.stop, None);
            assert_eq!(
                group.completed(completion.index).unwrap(),
                [b"abc".as_slice(), b"xy".as_slice()][completion.index]
            );
        }
        assert_eq!(observed, [true, true]);
        assert!(group.next().is_none());
    });
}

#[test]
fn cancellation_after_wait_entry_retains_source_state_and_releases_all_byte_loans() {
    let (stream, mut peer) = UnixStream::pair().unwrap();
    stream.set_nonblocking(true).unwrap();
    let mut owners = FixedSlice::new(1).unwrap();
    owners
        .push(unsafe { OwnedHandle::from_raw(stream.into_raw_fd()) })
        .unwrap();
    let state = cancellation::dr_v1_cancellation_new();
    assert!(!state.is_null());
    let mut set = unsafe { TransferSet::new(&[42], owners, state).unwrap() };
    let requester = unsafe { cancellation::dr_v1_cancellation_retain(state) } as usize;
    unsafe {
        cancellation::dr_v1_cancellation_release(state);
    }
    let wait = budget(2, 5_000_000_000);
    let mut bytes = [0x55; 8];
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let (entered, waiting) = sync_channel(0);
    let mut group = observed_begin(&mut set, buffers, &wait, entered);
    std::thread::scope(|scope| {
        scope.spawn(move || {
            waiting.recv_timeout(Duration::from_secs(5)).unwrap();
            let (mut reason, mut code, mut has_code) = (0, 0, 0);
            unsafe {
                let requester = requester as *mut DrCancellationV1;
                assert_eq!(
                    cancellation::dr_v1_cancellation_request(
                        requester,
                        &mut reason,
                        &mut code,
                        &mut has_code
                    ),
                    0
                );
                cancellation::dr_v1_cancellation_release(requester);
            }
        });
        let complete = group.next().unwrap();
        assert_eq!(complete.outcome.stop, Some(Stop::Cancellation));
        assert_eq!(complete.outcome.transfer, None);
        assert_eq!(complete.outcome.cancellation_failure, None);
        assert_eq!(group.completed(0), None);
        assert_eq!(group.stop(), Some(Stop::Cancellation));
        let mut status = DrCompletionStatusV1::default();
        assert_eq!(status.project(complete.outcome, true), 4);
        assert_eq!((status.stop, status.has_cancellation_failure), (1, 0));
        assert_eq!(
            (status.wait_operation, status.cancellation_operation),
            (0, 0)
        );
        assert_eq!(
            (
                status.transfer.value,
                status.transfer.has_system_code,
                status.transfer.has_transferred_byte_count
            ),
            (0, 0, 0)
        );
        assert_eq!(group.read_storage_mut(0).unwrap(), &[0x55; 8]);
        assert!(group.next().is_none());
    });
    drop(group);
    assert_eq!(bytes, [0x55; 8]);
    assert!(set.watcher.slots().iter().all(|entry| entry.interests == 0));
    peer.write_all(b"still open").unwrap();
}

#[test]
fn immediate_and_expired_controls_stop_pending_loans_without_fabricated_progress() {
    for wait in [budget(0, 0), budget(3, 0)] {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let mut set = set(&[42], [stream].into_iter());
        let mut bytes = [0x55; 8];
        let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
        let mut group = set.begin(buffers, &wait).unwrap();
        let complete = group.next().unwrap();
        assert_eq!(complete.outcome.stop, Some(Stop::BudgetExhausted));
        assert_eq!(complete.outcome.transfer, None);
        assert_eq!(complete.outcome.cancellation_failure, None);
        assert_eq!(group.completed(0), None);
        assert_eq!(group.stop(), Some(Stop::BudgetExhausted));
        assert!(group.next().is_none());
        drop(group);
        assert_eq!(bytes, [0x55; 8]);
        peer.write_all(b"still live").unwrap();
    }
}

#[test]
fn readiness_retry_preflight_preserves_cached_write_and_does_not_consume_canceled_read() {
    let cancellation = cancellation::dr_v1_cancellation_new();
    assert!(!cancellation.is_null());
    let (reader, mut read_peer) = UnixStream::pair().unwrap();
    let (writer, mut write_peer) = UnixStream::pair().unwrap();
    let mut set = set_with_cancellation(&[10, 20], [reader, writer].into_iter(), cancellation);
    let indefinite = budget(1, 0);
    let mut bytes = [0xAA; 4];
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut bytes), Buffer::Write(b"won")].into_iter())
            .unwrap();
    let mut group = set.begin(buffers, &indefinite).unwrap();
    read_peer.write_all(b"stay").unwrap();
    let (mut reason, mut code, mut has_code) = (0, 0, 0);
    assert_eq!(
        unsafe {
            cancellation::dr_v1_cancellation_request(
                cancellation,
                &mut reason,
                &mut code,
                &mut has_code,
            )
        },
        0
    );
    let read = group.next().unwrap();
    let write = group.next().unwrap();
    assert_eq!((read.key, write.key), (10, 20));
    assert_eq!(read.outcome.stop, Some(Stop::Cancellation));
    assert_eq!(read.outcome.transfer, None);
    assert_eq!(write.outcome.transfer, Some(Ok(3)));
    assert_eq!(write.outcome.stop, Some(Stop::Cancellation));
    drop(group);
    assert_eq!(bytes, [0xAA; 4]);
    assert_no_watches(&mut set);
    let mut actual = [0; 4];
    assert_eq!(
        unsafe {
            libc::read(
                set.owners.as_slice()[0].get(),
                actual.as_mut_ptr().cast(),
                4,
            )
        },
        4
    );
    assert_eq!(&actual, b"stay");
    write_peer.read_exact(&mut actual[..3]).unwrap();
    assert_eq!(&actual[..3], b"won");
    drop(set);
    unsafe {
        cancellation::dr_v1_cancellation_release(cancellation);
    }
}

#[test]
fn expired_budget_rejects_queued_native_read_before_first_transfer() {
    let (stream, mut peer) = UnixStream::pair().unwrap();
    let mut set = set(&[42], [stream].into_iter());
    peer.write_all(b"stay").unwrap();
    let expired = budget(3, 0);
    let mut bytes = [0xAA; 4];
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let mut group = set.begin(buffers, &expired).unwrap();
    let stopped = group.next().unwrap();
    assert_eq!(stopped.outcome.stop, Some(Stop::BudgetExhausted));
    assert!(stopped.outcome.transfer.is_none());
    drop(group);
    assert_eq!(bytes, [0xAA; 4]);
    assert_unsubmitted(&set, 0);
    assert_no_watches(&mut set);
    let mut actual = [0; 4];
    assert_eq!(
        unsafe {
            libc::read(
                set.owners.as_slice()[0].get(),
                actual.as_mut_ptr().cast(),
                4,
            )
        },
        4
    );
    assert_eq!(&actual, b"stay");
}

#[test]
fn blocking_nonfiles_are_rejected_and_consumed_once_without_mode_changes() {
    let (stream, mut peer) = UnixStream::pair().unwrap();
    let handle = stream.into_raw_fd();
    let mut owners = FixedSlice::new(1).unwrap();
    owners
        .push(unsafe { OwnedHandle::from_raw(handle) })
        .unwrap();
    let (result, closes) = crate::native_handle::with_close_probe(|| unsafe {
        TransferSet::new(&[10], owners, ptr::null_mut())
    });
    assert_eq!(result.err().unwrap().reason, Reason::Unsupported);
    assert_eq!(closes, 1);
    assert_eq!(peer.read(&mut [0; 1]).unwrap(), 0);
}

#[test]
fn ffi_invalid_descriptors_leave_buffers_and_streams_untouched_before_submission() {
    unsafe {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        let handle = stream.into_raw_fd();
        let mut status = DrIoStatusV1::default();
        let set = dr_v1_unix_transfer_set_new(
            core::ptr::null(),
            &42,
            &handle,
            1,
            ptr::null_mut(),
            &mut status,
        );
        assert!(!set.is_null());
        peer.write_all(b"abc").unwrap();
        let wait = budget(0, 0);
        let mut bytes = [0x55; 8];
        // Count rejection must not inspect descriptors or the wait budget.
        assert!(dr_v1_transfer_group_begin(
            core::ptr::null(),
            set,
            ptr::null(),
            0,
            ptr::null(),
            &mut status,
        )
        .is_null());
        assert_eq!(status.reason, Reason::InvalidInput as u8);
        for (data, length, reading) in [
            (bytes.as_mut_ptr(), bytes.len(), 2),
            (ptr::null_mut(), bytes.len(), 1),
            (bytes.as_mut_ptr(), usize::MAX, 1),
        ] {
            let descriptor = DrTransferBufferV1 {
                data,
                length,
                reading,
            };
            status.value = 99;
            let (owner, allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_transfer_group_begin(
                    core::ptr::null(),
                    set,
                    &descriptor,
                    1,
                    &wait,
                    &mut status,
                )
            });
            assert!(owner.is_null());
            assert_eq!(allocations, 0);
            assert_eq!(status.reason, Reason::InvalidInput as u8);
            assert_eq!(status.value, 0);
            assert_eq!(bytes, [0x55; 8]);
        }
        let descriptor = DrTransferBufferV1 {
            data: bytes.as_mut_ptr(),
            length: bytes.len(),
            reading: 1,
        };
        let group =
            dr_v1_transfer_group_begin(core::ptr::null(), set, &descriptor, 1, &wait, &mut status);
        assert!(!group.is_null());
        let (mut key, mut index) = (0, 0);
        let mut outcome = DrCompletionStatusV1::default();
        assert_eq!(
            dr_v1_transfer_group_next(
                group,
                &mut key,
                &mut index,
                (&mut outcome as *mut DrCompletionStatusV1).cast(),
            ),
            0
        );
        assert_eq!((key, index, outcome.transfer.value), (42, 0, 3));
        dr_v1_transfer_group_drop(group);
        assert_eq!(&bytes[..3], b"abc");
        assert_eq!(&bytes[3..], &[0x55; 5]);
        dr_v1_transfer_set_drop(set);
        assert_eq!(peer.read(&mut [0; 1]).unwrap(), 0);
    }
}

#[test]
fn ffi_regions_and_resubmission_are_gated_while_a_request_is_pending() {
    unsafe {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        let handle = stream.into_raw_fd();
        let mut status = DrIoStatusV1::default();
        let set = dr_v1_unix_transfer_set_new(
            core::ptr::null(),
            &42,
            &handle,
            1,
            ptr::null_mut(),
            &mut status,
        );
        assert!(!set.is_null());
        let wait = budget(0, 0);
        let mut bytes = [0x55; 8];
        let descriptor = DrTransferBufferV1 {
            data: bytes.as_mut_ptr(),
            length: 8,
            reading: 1,
        };
        let group =
            dr_v1_transfer_group_begin(core::ptr::null(), set, &descriptor, 1, &wait, &mut status);
        assert!(!group.is_null());
        let (mut data, mut length) = (ptr::null_mut(), 999);
        assert_eq!(
            dr_v1_transfer_group_region(group, 0, 1, &mut data, &mut length),
            1
        );
        assert!(data.is_null());
        assert_eq!(length, 0);
        assert_eq!(dr_v1_transfer_group_submit(group, 0, 0, 8, &mut status), 3);
        // Access only disjoint status while the original buffer loan is held.
        assert!(dr_v1_transfer_group_begin(
            core::ptr::null(),
            set,
            ptr::null(),
            0,
            &wait,
            &mut status
        )
        .is_null());
        let (mut key, mut index) = (999, 999);
        let mut outcome = DrCompletionStatusV1::default();
        assert_eq!(
            dr_v1_transfer_group_next(
                group,
                &mut key,
                &mut index,
                (&mut outcome as *mut DrCompletionStatusV1).cast()
            ),
            4
        );
        assert_eq!((key, index, outcome.stop), (42, 0, 2));
        assert_eq!(
            (
                outcome.has_cancellation_failure,
                outcome.wait_operation,
                outcome.cancellation_operation
            ),
            (0, 0, 0)
        );
        assert_eq!(
            (
                outcome.transfer.value,
                outcome.transfer.has_system_code,
                outcome.transfer.has_transferred_byte_count
            ),
            (0, 0, 0)
        );
        assert_eq!(dr_v1_transfer_group_stop(group), 2);
        assert_eq!(
            dr_v1_transfer_group_region(group, 0, 0, &mut data, &mut length),
            1
        );
        assert!(data.is_null());
        assert_eq!(length, 0);
        assert_eq!(
            dr_v1_transfer_group_region(group, 0, 1, &mut data, &mut length),
            0
        );
        assert_eq!(length, 8);
        assert_eq!(slice::from_raw_parts(data, length), &[0x55; 8]);
        assert_eq!(
            dr_v1_transfer_group_next(
                group,
                &mut key,
                &mut index,
                (&mut outcome as *mut DrCompletionStatusV1).cast()
            ),
            5
        );
        assert_eq!(
            (key, index, outcome.stop, outcome.transfer.value),
            (0, 0, 0, 0)
        );
        dr_v1_transfer_group_drop(group);
        peer.write_all(b"abc").unwrap();
        let group =
            dr_v1_transfer_group_begin(core::ptr::null(), set, &descriptor, 1, &wait, &mut status);
        assert!(!group.is_null());
        assert_eq!(
            dr_v1_transfer_group_next(
                group,
                &mut key,
                &mut index,
                (&mut outcome as *mut DrCompletionStatusV1).cast()
            ),
            0
        );
        assert_eq!(outcome.transfer.value, 3);
        assert_eq!(
            dr_v1_transfer_group_region(group, 0, 0, &mut data, &mut length),
            0
        );
        assert_eq!(slice::from_raw_parts(data, length), b"abc");
        assert_eq!(
            dr_v1_transfer_group_next(
                group,
                &mut key,
                &mut index,
                (&mut outcome as *mut DrCompletionStatusV1).cast()
            ),
            5
        );
        assert_eq!(
            (key, index, outcome.stop, outcome.transfer.value),
            (0, 0, 0, 0)
        );
        dr_v1_transfer_group_drop(group);
        let ((), closes) = crate::native_handle::with_close_probe(|| dr_v1_transfer_set_drop(set));
        assert_eq!(closes, 2, "watcher and registered stream each close once");
        assert_eq!(peer.read(&mut [0; 1]).unwrap(), 0);
    }
}

#[test]
fn ffi_fetch_timeout_preserves_live_storage_and_differs_from_exhaustion() {
    unsafe {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        let handle = stream.into_raw_fd();
        let mut status = DrIoStatusV1::default();
        let set = dr_v1_unix_transfer_set_new(
            core::ptr::null(),
            &42,
            &handle,
            1,
            ptr::null_mut(),
            &mut status,
        );
        assert!(!set.is_null());
        let lifetime = budget(1, 0);
        let mut bytes = [0x55; 4];
        let descriptor = DrTransferBufferV1 {
            data: bytes.as_mut_ptr(),
            length: bytes.len(),
            reading: 1,
        };
        let group = dr_v1_transfer_group_begin(
            core::ptr::null(),
            set,
            &descriptor,
            1,
            &lifetime,
            &mut status,
        );
        assert!(!group.is_null());
        let (mut key, mut index) = (999, 999);
        let mut outcome = DrCompletionStatusV1::default();
        for wait in [budget(0, 0), budget(3, 0), budget(2, 1_000_000)] {
            let (tag, allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_transfer_group_observe(
                    group,
                    &wait,
                    &mut key,
                    &mut index,
                    ptr::from_mut(&mut outcome).cast(),
                )
            });
            assert_eq!((tag, allocations), (6, 0));
            assert_eq!(
                (
                    key,
                    index,
                    outcome.stop,
                    outcome.transfer.has_transferred_byte_count
                ),
                (0, 0, 0, 0)
            );
            assert_eq!(dr_v1_transfer_group_stop(group), 0);
            let (mut data, mut length) = (ptr::null_mut(), 99);
            assert_eq!(
                dr_v1_transfer_group_region(group, 0, 1, &mut data, &mut length),
                1
            );
            assert!(data.is_null());
            assert_eq!(length, 0);
            assert_eq!(dr_v1_transfer_group_submit(group, 0, 0, 4, &mut status), 3);
        }
        peer.write_all(b"ab").unwrap();
        assert_eq!(
            dr_v1_transfer_group_observe(
                group,
                &lifetime,
                &mut key,
                &mut index,
                ptr::from_mut(&mut outcome).cast()
            ),
            0
        );
        assert_eq!((key, index, outcome.transfer.value), (42, 0, 2));
        let (mut data, mut length) = (ptr::null_mut(), 0);
        assert_eq!(
            dr_v1_transfer_group_region(group, 0, 0, &mut data, &mut length),
            0
        );
        assert_eq!(slice::from_raw_parts(data, length), b"ab");
        assert_eq!(
            dr_v1_transfer_group_observe(
                group,
                &budget(0, 0),
                &mut key,
                &mut index,
                ptr::from_mut(&mut outcome).cast()
            ),
            5
        );
        assert_eq!(dr_v1_transfer_group_submit(group, 0, 0, 4, &mut status), 0);
        assert_eq!(
            dr_v1_transfer_group_observe(
                group,
                &budget(0, 0),
                &mut key,
                &mut index,
                ptr::from_mut(&mut outcome).cast()
            ),
            6
        );
        peer.write_all(b"cd").unwrap();
        assert_eq!(
            dr_v1_transfer_group_observe(
                group,
                &lifetime,
                &mut key,
                &mut index,
                ptr::from_mut(&mut outcome).cast()
            ),
            0
        );
        assert_eq!(
            (key, index, outcome.transfer.value, outcome.stop),
            (42, 0, 2, 0)
        );
        assert_eq!(
            dr_v1_transfer_group_region(group, 0, 0, &mut data, &mut length),
            0
        );
        assert_eq!(slice::from_raw_parts(data, length), b"cd");
        dr_v1_transfer_group_drop(group);
        assert_eq!(&bytes, b"cdUU");
        dr_v1_transfer_set_drop(set);
    }
}

#[test]
fn ffi_set_release_preserves_active_loans_then_returns_live_owners_for_regrouping() {
    unsafe {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        let handle = stream.into_raw_fd();
        let mut status = DrIoStatusV1::default();
        let set = dr_v1_unix_transfer_set_new(
            core::ptr::null(),
            &42,
            &handle,
            1,
            ptr::null_mut(),
            &mut status,
        );
        assert!(!set.is_null());
        let wait = budget(0, 0);
        let mut bytes = [0x55; 8];
        let descriptor = DrTransferBufferV1 {
            data: bytes.as_mut_ptr(),
            length: bytes.len(),
            reading: 1,
        };
        let group =
            dr_v1_transfer_group_begin(core::ptr::null(), set, &descriptor, 1, &wait, &mut status);
        assert!(!group.is_null());
        let mut returned = -55;
        assert_eq!(
            dr_v1_unix_transfer_set_release(set, &mut returned, &mut status),
            3
        );
        assert_eq!(returned, -55);
        dr_v1_transfer_group_drop(group);
        let (result, closes) = crate::native_handle::with_close_probe(|| {
            dr_v1_unix_transfer_set_release(set, &mut returned, &mut status)
        });
        assert_eq!(result, 0);
        assert_eq!(closes, 1, "release closes only the watcher");
        assert_eq!(returned, handle);
        assert_eq!(bytes, [0x55; 8]);
        peer.write_all(b"regroup").unwrap();
        let set = dr_v1_unix_transfer_set_new(
            core::ptr::null(),
            &99,
            &returned,
            1,
            ptr::null_mut(),
            &mut status,
        );
        assert!(!set.is_null());
        let group =
            dr_v1_transfer_group_begin(core::ptr::null(), set, &descriptor, 1, &wait, &mut status);
        assert!(!group.is_null());
        let (mut key, mut index) = (0, 0);
        let mut outcome = DrCompletionStatusV1::default();
        assert_eq!(
            dr_v1_transfer_group_next(
                group,
                &mut key,
                &mut index,
                (&mut outcome as *mut DrCompletionStatusV1).cast(),
            ),
            0
        );
        assert_eq!((key, index, outcome.transfer.value), (99, 0, 7));
        dr_v1_transfer_group_drop(group);
        assert_eq!(&bytes[..7], b"regroup");
        let ((), closes) = crate::native_handle::with_close_probe(|| dr_v1_transfer_set_drop(set));
        assert_eq!(closes, 2, "regrouped watcher and stream each close once");
        assert_eq!(peer.read(&mut [0; 1]).unwrap(), 0);
    }
}

#[test]
fn ffi_empty_requests_and_actual_eof_stay_distinct_and_outcome_layout_is_shared() {
    unsafe {
        let (stream, peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        drop(peer);
        let handle = stream.into_raw_fd();
        let mut status = DrIoStatusV1::default();
        let set = dr_v1_unix_transfer_set_new(
            core::ptr::null(),
            &42,
            &handle,
            1,
            ptr::null_mut(),
            &mut status,
        );
        let wait = budget(0, 0);
        let mut bytes = [0x55; 8];
        for length in [0, 8] {
            let descriptor = DrTransferBufferV1 {
                data: bytes.as_mut_ptr(),
                length,
                reading: 1,
            };
            let group = dr_v1_transfer_group_begin(
                core::ptr::null(),
                set,
                &descriptor,
                1,
                &wait,
                &mut status,
            );
            let (mut key, mut index) = (0, 0);
            let mut outcome = DrCompletionStatusV1::default();
            assert_eq!(
                dr_v1_transfer_group_next(
                    group,
                    &mut key,
                    &mut index,
                    (&mut outcome as *mut DrCompletionStatusV1).cast()
                ),
                if length == 0 { 0 } else { 2 }
            );
            assert_eq!(outcome.transfer.value, 0);
            dr_v1_transfer_group_drop(group);
        }
        let (mut size, mut align) = (0, 0);
        crate::transfer_group::transport::dr_v1_transfer_outcome_layout(&mut size, &mut align);
        assert_eq!(
            (size, align),
            (
                mem::size_of::<DrCompletionStatusV1>(),
                mem::align_of::<DrCompletionStatusV1>()
            )
        );
        dr_v1_transfer_set_drop(set);
        assert_eq!(bytes, [0x55; 8]);
    }
}

unsafe fn ffi_plan_key(set: *mut core::ffi::c_void, index: usize) -> PlanKey {
    let mut key = mem::MaybeUninit::<PlanKey>::uninit();
    let mut status = DrIoStatusV1::default();
    assert_eq!(
        dr_v1_transfer_key(set, index, key.as_mut_ptr().cast(), &mut status),
        0
    );
    key.assume_init()
}

unsafe fn assert_ffi_plan_data(group: *mut core::ffi::c_void, expected: &[(u64, *mut u8, &[u8])]) {
    assert!(expected.len() < usize::BITS as usize);
    let mut seen = 0usize;
    let (mut key, mut index) = (0, 0);
    let mut outcome = DrCompletionStatusV1::default();
    for _ in expected {
        assert_eq!(
            dr_v1_transfer_group_next(
                group,
                &mut key,
                &mut index,
                (&mut outcome as *mut DrCompletionStatusV1).cast(),
            ),
            0
        );
        assert!(index < expected.len());
        assert_eq!(seen & (1 << index), 0, "completion was reported twice");
        seen |= 1 << index;
        let (expected_key, original, bytes) = expected[index];
        assert_eq!(
            (key, outcome.transfer.value),
            (expected_key, bytes.len() as u64)
        );
        assert_eq!(outcome.stop, 0);
        let (mut data, mut length) = (ptr::null_mut(), 0);
        assert_eq!(
            dr_v1_transfer_group_region(group, index, 0, &mut data, &mut length),
            0
        );
        assert_eq!(data, original, "plan setup must not copy caller bytes");
        assert_eq!(length, bytes.len());
        assert_eq!(slice::from_raw_parts(data, length), bytes);
    }
    assert_eq!(seen, (1 << expected.len()) - 1);
    assert_eq!(
        dr_v1_transfer_group_next(
            group,
            &mut key,
            &mut index,
            (&mut outcome as *mut DrCompletionStatusV1).cast(),
        ),
        5
    );
}

#[test]
fn ffi_keyed_plan_reversed_subset_moves_loans_without_copying_or_consuming_unused_bytes() {
    unsafe {
        let (a, mut a_peer) = UnixStream::pair().unwrap();
        let (b, mut b_peer) = UnixStream::pair().unwrap();
        let (c, mut c_peer) = UnixStream::pair().unwrap();
        let owner =
            allocate_set(core::ptr::null(), set(&[10, 20, 30], [a, b, c].into_iter())).unwrap();
        let keys = [
            ffi_plan_key(owner, 0),
            ffi_plan_key(owner, 1),
            ffi_plan_key(owner, 2),
        ];
        let (mut size, mut align) = (0, 0);
        dr_v1_transfer_key_layout(&mut size, &mut align);
        assert_eq!(
            (size, align),
            (mem::size_of::<PlanKey>(), mem::align_of::<PlanKey>())
        );
        a_peer.write_all(b"a").unwrap();
        b_peer.write_all(b"unused").unwrap();
        c_peer.write_all(b"ccc").unwrap();
        let (mut a_bytes, mut c_bytes) = ([0x55; 8], [0x77; 8]);
        let mut a_buffer = DrTransferBufferV1 {
            data: a_bytes.as_mut_ptr(),
            length: 8,
            reading: 1,
        };
        let mut c_buffer = DrTransferBufferV1 {
            data: c_bytes.as_mut_ptr(),
            length: 8,
            reading: 1,
        };
        let mut status = DrIoStatusV1::default();
        let plan = dr_v1_transfer_plan_new(
            core::ptr::null(),
            (&keys[2] as *const PlanKey).cast(),
            &c_buffer,
            &mut status,
        );
        assert!(!plan.is_null());
        let moved = dr_v1_transfer_plan_append(
            core::ptr::null(),
            plan,
            (&keys[0] as *const PlanKey).cast(),
            &a_buffer,
            &mut status,
        );
        assert!(!moved.is_null());
        let ((), closes) =
            crate::native_handle::with_close_probe(|| dr_v1_transfer_plan_drop(moved));
        assert_eq!(closes, 0, "a dropped plan owns no native streams");
        assert_eq!(a_bytes, [0x55; 8]);
        assert_eq!(c_bytes, [0x77; 8]);
        // Dropping the moved plan releases both loans without submitting I/O.
        a_bytes.fill(0x44);
        c_bytes.fill(0x33);
        a_buffer.data = a_bytes.as_mut_ptr();
        c_buffer.data = c_bytes.as_mut_ptr();
        let plan = dr_v1_transfer_plan_new(
            core::ptr::null(),
            (&keys[2] as *const PlanKey).cast(),
            &c_buffer,
            &mut status,
        );
        let plan = dr_v1_transfer_plan_append(
            core::ptr::null(),
            plan,
            (&keys[0] as *const PlanKey).cast(),
            &a_buffer,
            &mut status,
        );
        assert!(!plan.is_null());
        let wait = budget(0, 0);
        let group = dr_v1_transfer_plan_begin(core::ptr::null(), owner, plan, &wait, &mut status);
        assert!(!group.is_null());
        assert_ffi_plan_data(
            group,
            &[(30, c_buffer.data, b"ccc"), (10, a_buffer.data, b"a")],
        );
        dr_v1_transfer_group_drop(group);
        assert_eq!(&a_bytes[..1], b"a");
        assert_eq!(&a_bytes[1..], &[0x44; 7]);
        assert_eq!(&c_bytes[..3], b"ccc");
        assert_eq!(&c_bytes[3..], &[0x33; 5]);
        let mut unused = [0x99; 8];
        let buffer = DrTransferBufferV1 {
            data: unused.as_mut_ptr(),
            length: 8,
            reading: 1,
        };
        let plan = dr_v1_transfer_plan_new(
            core::ptr::null(),
            (&keys[1] as *const PlanKey).cast(),
            &buffer,
            &mut status,
        );
        let group = dr_v1_transfer_plan_begin(core::ptr::null(), owner, plan, &wait, &mut status);
        assert!(!group.is_null());
        assert_ffi_plan_data(group, &[(20, buffer.data, b"unused")]);
        dr_v1_transfer_group_drop(group);
        assert_eq!(&unused[..6], b"unused");
        assert_eq!(&unused[6..], &[0x99; 2]);
        dr_v1_transfer_set_drop(owner);
    }
}

#[test]
fn ffi_foreign_and_retired_plan_keys_reject_same_numeric_registration_before_io() {
    unsafe {
        let (a, mut a_peer) = UnixStream::pair().unwrap();
        let (b, mut b_peer) = UnixStream::pair().unwrap();
        let a_owner = allocate_set(core::ptr::null(), set(&[77], [a].into_iter())).unwrap();
        let b_owner = allocate_set(core::ptr::null(), set(&[77], [b].into_iter())).unwrap();
        let old_a_key = ffi_plan_key(a_owner, 0);
        let b_key = ffi_plan_key(b_owner, 0);
        assert_eq!(old_a_key.value, b_key.value);
        assert_ne!(old_a_key.owner, b_key.owner);
        a_peer.write_all(b"a").unwrap();
        b_peer.write_all(b"b").unwrap();
        let mut status = DrIoStatusV1::default();
        let mut rejected = [0x55; 8];
        let buffer = DrTransferBufferV1 {
            data: rejected.as_mut_ptr(),
            length: 8,
            reading: 1,
        };
        let plan = dr_v1_transfer_plan_new(
            core::ptr::null(),
            (&old_a_key as *const PlanKey).cast(),
            &buffer,
            &mut status,
        );
        assert!(!plan.is_null());
        // Non-live budget storage is permitted only on the documented checked
        // rejection path. It must not be dereferenced or used to prepare I/O.
        let unopened_budget = ptr::NonNull::<DrWaitBudgetV1>::dangling().as_ptr();
        let (group, allocations) = crate::probe_runtime_allocations(|| {
            dr_v1_transfer_plan_begin(
                core::ptr::null(),
                b_owner,
                plan,
                unopened_budget,
                &mut status,
            )
        });
        assert!(group.is_null());
        assert_eq!(allocations, 0);
        assert_eq!(status.reason, Reason::InvalidInput as u8);
        assert_eq!(rejected, [0x55; 8]);
        rejected.fill(0x44);
        let mut returned = -1;
        assert_eq!(
            dr_v1_unix_transfer_set_release(a_owner, &mut returned, &mut status),
            0
        );
        let a_owner = dr_v1_unix_transfer_set_new(
            core::ptr::null(),
            &77,
            &returned,
            1,
            ptr::null_mut(),
            &mut status,
        );
        assert!(!a_owner.is_null());
        let fresh_a_key = ffi_plan_key(a_owner, 0);
        assert_eq!(fresh_a_key.value, old_a_key.value);
        assert_ne!(fresh_a_key.owner, old_a_key.owner);
        let buffer = DrTransferBufferV1 {
            data: rejected.as_mut_ptr(),
            length: 8,
            reading: 1,
        };
        let plan = dr_v1_transfer_plan_new(
            core::ptr::null(),
            (&old_a_key as *const PlanKey).cast(),
            &buffer,
            &mut status,
        );
        assert!(!plan.is_null());
        let (group, allocations) = crate::probe_runtime_allocations(|| {
            dr_v1_transfer_plan_begin(
                core::ptr::null(),
                a_owner,
                plan,
                unopened_budget,
                &mut status,
            )
        });
        assert!(group.is_null());
        assert_eq!(allocations, 0);
        assert_eq!(status.reason, Reason::InvalidInput as u8);
        assert_eq!(rejected, [0x44; 8]);
        rejected.fill(0x33);
        let wait = budget(0, 0);
        for (owner, key, payload) in [(a_owner, fresh_a_key, b"a"), (b_owner, b_key, b"b")] {
            let buffer = DrTransferBufferV1 {
                data: rejected.as_mut_ptr(),
                length: 8,
                reading: 1,
            };
            let plan = dr_v1_transfer_plan_new(
                core::ptr::null(),
                (&key as *const PlanKey).cast(),
                &buffer,
                &mut status,
            );
            let group =
                dr_v1_transfer_plan_begin(core::ptr::null(), owner, plan, &wait, &mut status);
            assert!(!group.is_null());
            assert_ffi_plan_data(group, &[(77, buffer.data, payload)]);
            dr_v1_transfer_group_drop(group);
            assert_eq!(&rejected[..1], payload);
            assert_eq!(&rejected[1..], &[0x33; 7]);
            dr_v1_transfer_set_drop(owner);
        }
    }
}

#[test]
fn ffi_plan_validation_consumes_rejected_appends_before_forming_overlapping_byte_loans() {
    unsafe {
        let (a, mut a_peer) = UnixStream::pair().unwrap();
        let (b, mut b_peer) = UnixStream::pair().unwrap();
        let owner = allocate_set(core::ptr::null(), set(&[11, 22], [a, b].into_iter())).unwrap();
        let keys = [ffi_plan_key(owner, 0), ffi_plan_key(owner, 1)];
        a_peer.write_all(b"left").unwrap();
        b_peer.write_all(b"right").unwrap();
        let (mut a_bytes, mut b_bytes) = ([0x55; 8], [0x66; 8]);
        let mut status = DrIoStatusV1::default();
        for (length, reading, address) in [(8, 2, 0), (8, 1, 1), (usize::MAX, 1, 0), (2, 1, 2)] {
            let valid = DrTransferBufferV1 {
                data: a_bytes.as_mut_ptr(),
                length: 8,
                reading: 1,
            };
            let malformed = DrTransferBufferV1 {
                data: match address {
                    0 => b_bytes.as_mut_ptr(),
                    1 => ptr::null_mut(),
                    _ => usize::MAX as *mut u8,
                },
                length,
                reading,
            };
            assert!(dr_v1_transfer_plan_new(
                core::ptr::null(),
                (&keys[1] as *const PlanKey).cast(),
                &malformed,
                &mut status
            )
            .is_null());
            assert_eq!(status.reason, Reason::InvalidInput as u8);
            let plan = dr_v1_transfer_plan_new(
                core::ptr::null(),
                (&keys[0] as *const PlanKey).cast(),
                &valid,
                &mut status,
            );
            assert!(!plan.is_null());
            let (plan, allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_transfer_plan_append(
                    core::ptr::null(),
                    plan,
                    (&keys[1] as *const PlanKey).cast(),
                    &malformed,
                    &mut status,
                )
            });
            assert!(plan.is_null());
            assert_eq!(allocations, 0);
            assert_eq!(status.reason, Reason::InvalidInput as u8);
            assert_eq!(a_bytes, [0x55; 8]);
            assert_eq!(b_bytes, [0x66; 8]);
        }
        let a_buffer = DrTransferBufferV1 {
            data: a_bytes.as_mut_ptr(),
            length: 8,
            reading: 1,
        };
        let b_buffer = DrTransferBufferV1 {
            data: b_bytes.as_mut_ptr(),
            length: 8,
            reading: 1,
        };
        let plan = dr_v1_transfer_plan_new(
            core::ptr::null(),
            (&keys[0] as *const PlanKey).cast(),
            &a_buffer,
            &mut status,
        );
        assert!(!plan.is_null());
        let plan = dr_v1_transfer_plan_append(
            core::ptr::null(),
            plan,
            (&keys[0] as *const PlanKey).cast(),
            &b_buffer,
            &mut status,
        );
        assert!(
            !plan.is_null(),
            "same registration with distinct storage is valid"
        );
        assert_eq!(status.reason, 0);
        dr_v1_transfer_plan_drop(plan);
        a_bytes.fill(0x55);
        b_bytes.fill(0x66);
        for reading in [1, 0] {
            let a_buffer = DrTransferBufferV1 {
                data: a_bytes.as_mut_ptr(),
                length: 8,
                reading: 1,
            };
            // Compute the conflicting metadata before creating the first loan;
            // validation must never turn it into a second overlapping slice.
            let overlap = DrTransferBufferV1 {
                data: a_buffer.data.add(1),
                length: 4,
                reading,
            };
            let plan = dr_v1_transfer_plan_new(
                core::ptr::null(),
                (&keys[0] as *const PlanKey).cast(),
                &a_buffer,
                &mut status,
            );
            assert!(!plan.is_null());
            let (plan, allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_transfer_plan_append(
                    core::ptr::null(),
                    plan,
                    (&keys[1] as *const PlanKey).cast(),
                    &overlap,
                    &mut status,
                )
            });
            assert!(plan.is_null());
            assert_eq!(allocations, 0);
            assert_eq!(status.reason, Reason::InvalidInput as u8);
            assert_eq!(a_bytes, [0x55; 8]);
            a_bytes.fill(0x55);
        }
        let a_buffer = DrTransferBufferV1 {
            data: a_bytes.as_mut_ptr(),
            length: 8,
            reading: 1,
        };
        let b_buffer = DrTransferBufferV1 {
            data: b_bytes.as_mut_ptr(),
            length: 8,
            reading: 1,
        };
        let plan = dr_v1_transfer_plan_new(
            core::ptr::null(),
            (&keys[0] as *const PlanKey).cast(),
            &a_buffer,
            &mut status,
        );
        let plan = dr_v1_transfer_plan_append(
            core::ptr::null(),
            plan,
            (&keys[1] as *const PlanKey).cast(),
            &b_buffer,
            &mut status,
        );
        assert!(!plan.is_null());
        let wait = budget(0, 0);
        let group = dr_v1_transfer_plan_begin(core::ptr::null(), owner, plan, &wait, &mut status);
        assert!(!group.is_null());
        assert_ffi_plan_data(
            group,
            &[(11, a_buffer.data, b"left"), (22, b_buffer.data, b"right")],
        );
        dr_v1_transfer_group_drop(group);
        assert_eq!(&a_bytes[4..], &[0x55; 4]);
        assert_eq!(&b_bytes[5..], &[0x66; 3]);
        dr_v1_transfer_set_drop(owner);
    }
}

#[test]
fn ffi_active_set_rejects_key_issuance_and_consumes_new_plan_without_touching_live_requests() {
    unsafe {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        let owner = allocate_set(core::ptr::null(), set(&[42], [stream].into_iter())).unwrap();
        let key = ffi_plan_key(owner, 0);
        let wait = budget(0, 0);
        let (mut bytes, mut other) = ([0x55; 8], [0x66; 8]);
        let buffer = DrTransferBufferV1 {
            data: bytes.as_mut_ptr(),
            length: 8,
            reading: 1,
        };
        let mut status = DrIoStatusV1::default();
        let plan = dr_v1_transfer_plan_new(
            core::ptr::null(),
            (&key as *const PlanKey).cast(),
            &buffer,
            &mut status,
        );
        let group = dr_v1_transfer_plan_begin(core::ptr::null(), owner, plan, &wait, &mut status);
        assert!(!group.is_null());
        let (mut data, mut length) = (ptr::null_mut(), 99);
        assert_eq!(
            dr_v1_transfer_group_region(group, 0, 1, &mut data, &mut length),
            1
        );
        assert!(data.is_null());
        assert_eq!(length, 0);
        let mut output = key;
        assert_eq!(
            dr_v1_transfer_key(owner, 0, (&mut output as *mut PlanKey).cast(), &mut status),
            3
        );
        assert_eq!(status.reason, Reason::InvalidInput as u8);
        assert_eq!(output, PlanKey { owner: 0, value: 0 });
        let other_buffer = DrTransferBufferV1 {
            data: other.as_mut_ptr(),
            length: 8,
            reading: 1,
        };
        let plan = dr_v1_transfer_plan_new(
            core::ptr::null(),
            (&key as *const PlanKey).cast(),
            &other_buffer,
            &mut status,
        );
        assert!(!plan.is_null());
        let (rejected, allocations) = crate::probe_runtime_allocations(|| {
            dr_v1_transfer_plan_begin(
                core::ptr::null(),
                owner,
                plan,
                ptr::NonNull::<DrWaitBudgetV1>::dangling().as_ptr(),
                &mut status,
            )
        });
        assert!(rejected.is_null());
        assert_eq!(allocations, 0);
        assert_eq!(status.reason, Reason::InvalidInput as u8);
        assert_eq!(other, [0x66; 8]);
        other.fill(0x33);
        assert_eq!(
            dr_v1_transfer_group_region(group, 0, 1, &mut data, &mut length),
            1
        );
        assert!(data.is_null());
        assert_eq!(length, 0);
        peer.write_all(b"ready").unwrap();
        assert_ffi_plan_data(group, &[(42, buffer.data, b"ready")]);
        dr_v1_transfer_group_drop(group);
        assert_eq!(&bytes[..5], b"ready");
        assert_eq!(&bytes[5..], &[0x55; 3]);
        assert_eq!(
            ffi_plan_key(owner, 0),
            key,
            "tickets last for the set configuration"
        );
        peer.write_all(b"later").unwrap();
        let other_buffer = DrTransferBufferV1 {
            data: other.as_mut_ptr(),
            length: 8,
            reading: 1,
        };
        let plan = dr_v1_transfer_plan_new(
            core::ptr::null(),
            (&key as *const PlanKey).cast(),
            &other_buffer,
            &mut status,
        );
        let group = dr_v1_transfer_plan_begin(core::ptr::null(), owner, plan, &wait, &mut status);
        assert!(!group.is_null());
        assert_ffi_plan_data(group, &[(42, other_buffer.data, b"later")]);
        dr_v1_transfer_group_drop(group);
        assert_eq!(&other[..5], b"later");
        assert_eq!(&other[5..], &[0x33; 3]);
        dr_v1_transfer_set_drop(owner);
    }
}

#[test]
fn borrowed_admission_rejection_removal_and_drop_never_close_external_stream_owners() {
    let (mut a, mut a_peer) = UnixStream::pair().unwrap();
    let (mut b, mut b_peer) = UnixStream::pair().unwrap();
    let (blocking, _blocking_peer) = UnixStream::pair().unwrap();
    a.set_nonblocking(true).unwrap();
    b.set_nonblocking(true).unwrap();
    let mut set = unsafe { TransferSet::borrowed_new(ptr::null_mut()).unwrap() };
    assert!(set.is_borrowed());
    assert_eq!(set.len(), 0);
    assert_eq!(set.key(0), None);
    let watcher = set.watcher.handle().unwrap();
    let a_index = unsafe { set.add_borrowed(10, a.as_raw_fd()).unwrap() };
    let a_request = set.slots.raw(a_index);
    for (key, handle, reason) in [
        (0, b.as_raw_fd(), Reason::InvalidInput),
        (11, a.as_raw_fd(), Reason::InvalidInput),
        (10, b.as_raw_fd(), Reason::InvalidInput),
        (20, blocking.as_raw_fd(), Reason::Unsupported),
        (20, -1, Reason::Closed),
        (u64::MAX, b.as_raw_fd(), Reason::InvalidInput),
    ] {
        let (result, closes) =
            crate::native_handle::with_close_probe(|| unsafe { set.add_borrowed(key, handle) });
        assert_eq!(result.unwrap_err().reason, reason);
        assert_eq!(closes, 0);
        assert_eq!(set.len(), 1);
        assert_eq!(set.slots.raw(a_index), a_request);
    }
    let b_index = unsafe { set.add_borrowed(20, b.as_raw_fd()).unwrap() };
    let b_request = set.slots.raw(b_index);
    assert_eq!(set.slots.raw(a_index), a_request);
    assert_eq!(set.watcher.handle().unwrap(), watcher);
    assert!(set.events.len() >= set.watcher.capacity * 2);
    let (result, closes) = crate::native_handle::with_close_probe(|| set.remove_borrowed(99));
    assert_eq!(result, Err(Failure::invalid_input()));
    assert_eq!(closes, 0);
    let (result, closes) = crate::native_handle::with_close_probe(|| set.remove_borrowed(10));
    result.unwrap();
    assert_eq!(closes, 0);
    assert_eq!(set.index_of(10), None);
    assert_eq!(set.key(a_index), None);
    assert_eq!(set.index_of(20), Some(b_index));
    assert_eq!(set.slots.raw(b_index), b_request);
    a_peer.write_all(b"removed").unwrap();
    let mut removed = [0; 7];
    a.read_exact(&mut removed).unwrap();
    assert_eq!(&removed, b"removed");
    let (result, closes) =
        crate::native_handle::with_close_probe(|| unsafe { set.add_borrowed(10, a.as_raw_fd()) });
    assert_eq!(
        result,
        Err(Failure::invalid_input()),
        "retired numeric keys cannot be reused"
    );
    assert_eq!(closes, 0);
    let readded = unsafe { set.add_borrowed(30, a.as_raw_fd()).unwrap() };
    assert_eq!(readded, a_index);
    assert_eq!(set.key(readded), Some(30));
    assert_eq!(set.index_of(10), None);
    assert_eq!(set.slots.raw(b_index), b_request);
    let ((), closes) = crate::native_handle::with_close_probe(|| drop(set));
    assert_eq!(closes, 1, "borrowed set owns only its watcher");
    a_peer.write_all(b"a").unwrap();
    b_peer.write_all(b"b").unwrap();
    let mut byte = [0];
    a.read_exact(&mut byte).unwrap();
    assert_eq!(&byte, b"a");
    b.read_exact(&mut byte).unwrap();
    assert_eq!(&byte, b"b");
    assert!(unsafe { libc::fcntl(blocking.as_raw_fd(), libc::F_GETFD) } >= 0);
}

#[test]
fn borrowed_selected_groups_keep_holes_and_stable_requests_across_drop_and_buffer_reuse() {
    let (a, mut a_peer) = UnixStream::pair().unwrap();
    let (mut b, mut b_peer) = UnixStream::pair().unwrap();
    let (c, mut c_peer) = UnixStream::pair().unwrap();
    let (d, mut d_peer) = UnixStream::pair().unwrap();
    for stream in [&a, &b, &c, &d] {
        stream.set_nonblocking(true).unwrap();
    }
    let mut set = unsafe { TransferSet::borrowed_new(ptr::null_mut()).unwrap() };
    let watcher = set.watcher.handle().unwrap();
    let mut indices = [0; 4];
    for (position, stream) in [&a, &b, &c, &d].into_iter().enumerate() {
        indices[position] = unsafe {
            set.add_borrowed((position as u64 + 1) * 10, stream.as_raw_fd())
                .unwrap()
        };
    }
    assert_eq!(indices, [0, 1, 2, 3]);
    let addresses = [set.slots.raw(0), set.slots.raw(2), set.slots.raw(3)];
    let memory = set.watcher.memory;
    let ready_storage = set.events.as_slice().as_ptr();
    set.remove_borrowed(20).unwrap();
    assert_eq!(set.len(), 3);
    assert_eq!(set.slots.len(), 4);
    assert_eq!(set.key(1), None);
    assert_eq!(set.index_of(20), None);
    let poll = budget(0, 0);
    let (mut c_bytes, mut d_bytes) = ([0x55; 8], [0x66; 8]);
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut d_bytes), Buffer::Read(&mut c_bytes)].into_iter())
            .unwrap();
    let group = set
        .begin_selected(buffers, &poll, selection(&[3, 2]))
        .unwrap();
    let ((), closes) = crate::native_handle::with_close_probe(|| drop(group));
    assert_eq!(closes, 0);
    assert_eq!(c_bytes, [0x55; 8]);
    assert_eq!(d_bytes, [0x66; 8]);
    assert_no_watches(&mut set);
    for index in [0, 2, 3] {
        assert!(unsafe { (*set.slots.raw(index)).data.is_null() });
    }
    a_peer.write_all(b"hold").unwrap();
    b_peer.write_all(b"free").unwrap();
    c_peer.write_all(b"one").unwrap();
    d_peer.write_all(b"end").unwrap();
    let mut rejected = [0x77; 8];
    let buffers = BufferList::from_iter([Buffer::Read(&mut rejected)].into_iter()).unwrap();
    let removed = selection(&[1]);
    let (result, allocations) =
        crate::probe_runtime_allocations(|| set.begin_selected(buffers, &poll, removed));
    assert_eq!(result.err(), Some(Failure::invalid_input()));
    assert_eq!(allocations, 0);
    assert_eq!(rejected, [0x77; 8]);
    assert_no_watches(&mut set);
    let d_pointer = d_bytes.as_mut_ptr();
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut d_bytes), Buffer::Read(&mut c_bytes)].into_iter())
            .unwrap();
    let mut group = set
        .begin_selected(buffers, &poll, selection(&[3, 2]))
        .unwrap();
    let complete = group.next().unwrap();
    assert_eq!(
        (complete.index, complete.key, complete.outcome.transfer),
        (0, 40, Some(Ok(3)))
    );
    assert_eq!(group.completed(0), Some(b"end".as_slice()));
    let storage = group.read_storage_mut(0).unwrap();
    assert_eq!(storage.as_mut_ptr(), d_pointer);
    storage.fill(0x44);
    let (result, allocations) = crate::probe_runtime_allocations(|| group.submit(0, 1, 3));
    result.unwrap();
    assert_eq!(allocations, 0);
    d_peer.write_all(b"two").unwrap();
    let complete = group.next().unwrap();
    assert_eq!(
        (complete.index, complete.key, complete.outcome.transfer),
        (1, 30, Some(Ok(3)))
    );
    assert_eq!(group.completed(1), Some(b"one".as_slice()));
    let complete = group.next().unwrap();
    assert_eq!(
        (complete.index, complete.key, complete.outcome.transfer),
        (0, 40, Some(Ok(3)))
    );
    assert_eq!(group.completed(0), Some(b"two".as_slice()));
    assert!(group.next().is_none());
    drop(group);
    assert_eq!(&c_bytes[..3], b"one");
    assert_eq!(&c_bytes[3..], &[0x55; 5]);
    assert_eq!(d_bytes, [0x44, b't', b'w', b'o', 0x44, 0x44, 0x44, 0x44]);
    assert_no_watches(&mut set);
    assert_eq!(
        [set.slots.raw(0), set.slots.raw(2), set.slots.raw(3)],
        addresses
    );
    assert_eq!(set.watcher.handle().unwrap(), watcher);
    assert_eq!(set.watcher.memory, memory);
    assert_eq!(set.events.as_slice().as_ptr(), ready_storage);
    let mut untouched = [0; 4];
    let buffers = BufferList::from_iter([Buffer::Read(&mut untouched)].into_iter()).unwrap();
    let mut group = set.begin_selected(buffers, &poll, selection(&[0])).unwrap();
    let complete = group.next().unwrap();
    assert_eq!(
        (complete.index, complete.key, complete.outcome.transfer),
        (0, 10, Some(Ok(4)))
    );
    assert_eq!(group.completed(0), Some(b"hold".as_slice()));
    drop(group);
    b.read_exact(&mut untouched).unwrap();
    assert_eq!(&untouched, b"free");
    let readded = unsafe { set.add_borrowed(50, b.as_raw_fd()).unwrap() };
    assert_eq!(readded, 1);
    assert_eq!(set.index_of(20), None);
    assert_eq!(set.index_of(10), Some(0));
    assert_eq!(set.index_of(30), Some(2));
    assert_eq!(set.index_of(40), Some(3));
    assert_eq!(
        [set.slots.raw(0), set.slots.raw(2), set.slots.raw(3)],
        addresses
    );
    assert_eq!(set.watcher.handle().unwrap(), watcher);
    assert_eq!(set.watcher.memory, memory);
    assert_eq!(set.events.as_slice().as_ptr(), ready_storage);
}

#[test]
fn ffi_fixed_membership_gates_and_borrowed_release_preserve_their_distinct_ownership() {
    unsafe {
        let (original, mut original_peer) = UnixStream::pair().unwrap();
        let (mut candidate, mut candidate_peer) = UnixStream::pair().unwrap();
        candidate.set_nonblocking(true).unwrap();
        let fixed = allocate_set(core::ptr::null(), set(&[42], [original].into_iter())).unwrap();
        let original_key = ffi_plan_key(fixed, 0);
        let mut status = DrIoStatusV1::default();
        // Unix projects a numeric fd, not a pointer. Zero projection and null
        // output are permitted here because fixed mode rejects before use.
        let ((tag, closes), allocations) = crate::probe_runtime_allocations(|| {
            crate::native_handle::with_close_probe(|| {
                dr_v1_transfer_set_add(fixed, 0, ptr::null_mut(), &mut status)
            })
        });
        assert_eq!((tag, allocations, closes), (3, 0, 0));
        assert_eq!(status.reason, Reason::InvalidInput as u8);
        for projection in [0, candidate.as_raw_fd()] {
            let mut output = original_key;
            let ((tag, closes), allocations) = crate::probe_runtime_allocations(|| {
                crate::native_handle::with_close_probe(|| {
                    dr_v1_transfer_set_add(
                        fixed,
                        projection,
                        (&mut output as *mut PlanKey).cast(),
                        &mut status,
                    )
                })
            });
            assert_eq!((tag, allocations, closes), (3, 0, 0));
            assert_eq!(status.reason, Reason::InvalidInput as u8);
            assert_eq!(output, PlanKey { owner: 0, value: 0 });
        }
        for key in [ptr::null(), (&original_key as *const PlanKey).cast()] {
            let ((tag, closes), allocations) = crate::probe_runtime_allocations(|| {
                crate::native_handle::with_close_probe(|| {
                    dr_v1_transfer_set_remove(fixed, key, &mut status)
                })
            });
            assert_eq!((tag, allocations, closes), (3, 0, 0));
            assert_eq!(status.reason, Reason::InvalidInput as u8);
        }
        assert_eq!(ffi_plan_key(fixed, 0), original_key);
        original_peer.write_all(b"owned").unwrap();
        let wait = budget(0, 0);
        let mut bytes = [0x55; 8];
        let buffer = DrTransferBufferV1 {
            data: bytes.as_mut_ptr(),
            length: 8,
            reading: 1,
        };
        let group =
            dr_v1_transfer_group_begin(core::ptr::null(), fixed, &buffer, 1, &wait, &mut status);
        assert!(!group.is_null());
        assert_ffi_plan_data(group, &[(42, buffer.data, b"owned")]);
        dr_v1_transfer_group_drop(group);
        assert_eq!(&bytes[..5], b"owned");
        let ((), closes) =
            crate::native_handle::with_close_probe(|| dr_v1_transfer_set_drop(fixed));
        assert_eq!(
            closes, 2,
            "fixed set retains its original stream and watcher owners"
        );
        assert_eq!(original_peer.read(&mut [0; 1]).unwrap(), 0);

        let borrowed = dr_v1_transfer_set_new(core::ptr::null(), ptr::null_mut(), &mut status);
        assert!(!borrowed.is_null());
        let mut key = mem::MaybeUninit::<PlanKey>::uninit();
        assert_eq!(
            dr_v1_transfer_set_add(
                borrowed,
                candidate.as_raw_fd(),
                key.as_mut_ptr().cast(),
                &mut status
            ),
            0
        );
        let key = key.assume_init();
        let ((tag, closes), allocations) = crate::probe_runtime_allocations(|| {
            crate::native_handle::with_close_probe(|| {
                dr_v1_unix_transfer_set_release(borrowed, ptr::null_mut(), &mut status)
            })
        });
        assert_eq!((tag, allocations, closes), (3, 0, 0));
        assert_eq!(status.reason, Reason::InvalidInput as u8);
        assert_eq!(
            ffi_plan_key(borrowed, 0),
            key,
            "rejected release did not consume membership"
        );
        candidate_peer.write_all(b"borrowed").unwrap();
        let mut bytes = [0x66; 8];
        let buffer = DrTransferBufferV1 {
            data: bytes.as_mut_ptr(),
            length: 8,
            reading: 1,
        };
        let plan = dr_v1_transfer_plan_new(
            core::ptr::null(),
            (&key as *const PlanKey).cast(),
            &buffer,
            &mut status,
        );
        assert!(!plan.is_null());
        let group =
            dr_v1_transfer_plan_begin(core::ptr::null(), borrowed, plan, &wait, &mut status);
        assert!(!group.is_null());
        assert_ffi_plan_data(group, &[(key.value, buffer.data, b"borrowed")]);
        dr_v1_transfer_group_drop(group);
        assert_eq!(&bytes, b"borrowed");
        let ((), closes) =
            crate::native_handle::with_close_probe(|| dr_v1_transfer_set_drop(borrowed));
        assert_eq!(closes, 1, "borrowed set closes only its watcher");
        candidate_peer.write_all(b"live").unwrap();
        let mut bytes = [0; 4];
        candidate.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"live");
    }
}
