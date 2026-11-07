//! Real Windows completion-port fixtures. Cross-compilation checks their types;
//! it does not execute kernel completion, cancellation, or drainage assertions.

use std::os::windows::io::{IntoRawHandle, OwnedHandle as StdHandle};
use std::sync::mpsc::{self, SyncSender};
use std::time::Duration;

use super::super::super::tests::{budget, connected_pair, TestClaim};
use super::super::TEST_BROKER_LOCK;
use super::*;

const TEST_NANOSECONDS: u64 = 5_000_000_000;

fn completion_set<const N: usize>(keys: &[u64; N], handles: [StdHandle; N]) -> CompletionSet {
    let mut owners = FixedSlice::new(N).unwrap();
    for handle in handles {
        owners
            .push(unsafe {
                AttachedStream::adopt(OwnedHandle::from_raw(handle.into_raw_handle())).unwrap()
            })
            .unwrap();
    }
    // connected_pair supplies independently owned, idle, overlapped BYTE/WAIT
    // handles without suppressed completion packets or previous port owners.
    CompletionSet::from_attached(keys, owners).unwrap()
}

fn peer(handle: StdHandle) -> Pipe<WindowsDriver> {
    Pipe {
        driver: unsafe { WindowsDriver::new(handle.into_raw_handle()).unwrap() },
    }
}

fn transfer(peer: &mut Pipe<WindowsDriver>, bytes: Buffer<'_>, wait: &DrWaitBudgetV1) -> Awaited {
    let mut control = unsafe { WaitControl::new(Some(wait), ptr::null_mut()).unwrap() };
    peer.transfer(bytes, &mut control)
}

// Inspect this set only after group teardown. The shared broker's global queue
// may still contain another group's packets or value-only pokes; emptiness of
// that queue is no longer an assertion about retirement of this set's requests.
fn assert_quiescent(set: &CompletionSet) {
    set.assert_retired_for_test();
    for index in 0..set.owners.len() {
        assert_ne!(
            unsafe { ptr::read_volatile((*set.slots.raw(index)).internal.get()) },
            STATUS_PENDING
        );
    }
}

#[test]
fn four_mixed_slots_preserve_stable_keys_and_exact_direct_borrowed_data() {
    let _broker = TEST_BROKER_LOCK.lock().unwrap();
    let (server_a, client_a) = connected_pair();
    let (server_b, client_b) = connected_pair();
    let (server_c, client_c) = connected_pair();
    let (server_d, client_d) = connected_pair();
    let mut peers = [
        peer(client_a),
        peer(client_b),
        peer(client_c),
        peer(client_d),
    ];
    let keys = [u64::MAX, 17, 1 << 48, 0];
    let mut set = completion_set(&keys, [server_a, server_b, server_c, server_d]);
    let requests = std::array::from_fn::<_, 4, _>(|index| set.slots.raw(index));
    let mut read_a = [0x55; 8];
    let mut read_b = [0x66; 8];
    let address_a = read_a.as_ptr();
    let address_b = read_b.as_ptr();
    let write_a = b"alpha";
    let write_b = b"beta";
    let buffers = BufferList::from_iter(
        [
            Buffer::Read(&mut read_a),
            Buffer::Read(&mut read_b),
            Buffer::Write(write_a),
            Buffer::Write(write_b),
        ]
        .into_iter(),
    )
    .unwrap();
    let mut group = {
        let wait = budget(2, TEST_NANOSECONDS);
        set.begin(buffers, Some(&wait), None).unwrap()
    };
    let wait = budget(2, TEST_NANOSECONDS);
    assert!(group.completed(0).is_none());
    assert!(group.read_storage_mut(0).is_none());
    assert!(group.read_storage_mut(1).is_none());

    // All four operations are submitted before peers supply or consume bytes.
    // Even a synchronously accepted write must retire its queued port packet.
    assert_eq!(
        transfer(&mut peers[0], Buffer::Write(b"red"), &wait),
        Awaited::complete(Ok(3))
    );
    assert_eq!(
        transfer(&mut peers[1], Buffer::Write(b"blue"), &wait),
        Awaited::complete(Ok(4))
    );
    let mut received_a = [0; 5];
    let mut received_b = [0; 4];
    assert_eq!(
        transfer(&mut peers[2], Buffer::Read(&mut received_a), &wait),
        Awaited::complete(Ok(5))
    );
    assert_eq!(
        transfer(&mut peers[3], Buffer::Read(&mut received_b), &wait),
        Awaited::complete(Ok(4))
    );
    assert_eq!(&received_a, write_a);
    assert_eq!(&received_b, write_b);

    let payloads: [&[u8]; 4] = [b"red", b"blue", write_a, write_b];
    let mut seen = [false; 4];
    while let Some(completion) = group.next() {
        let index = completion.index;
        assert!(!seen[index], "the same packet was completed twice");
        seen[index] = true;
        assert_eq!(completion.key, keys[index]);
        assert_eq!(
            completion.outcome,
            Awaited::complete(Ok(payloads[index].len()))
        );
        assert_eq!(group.completed(index), Some(payloads[index]));
    }
    assert_eq!(seen, [true; 4]);
    assert_eq!(group.stop(), None);
    assert_eq!(group.read_storage_mut(0).unwrap().as_ptr(), address_a);
    assert_eq!(group.read_storage_mut(1).unwrap().as_ptr(), address_b);
    drop(group);
    assert_eq!(&read_a[..3], b"red");
    assert_eq!(&read_a[3..], &[0x55; 5]);
    assert_eq!(&read_b[..4], b"blue");
    assert_eq!(&read_b[4..], &[0x66; 4]);
    for (index, request) in requests.into_iter().enumerate() {
        assert_eq!(set.slots.raw(index), request);
    }
    assert_quiescent(&set);
}

#[test]
fn completed_read_storage_is_reused_while_another_read_stays_pending() {
    let _broker = TEST_BROKER_LOCK.lock().unwrap();
    let (server_a, client_a) = connected_pair();
    let (server_b, client_b) = connected_pair();
    let mut peer_a = peer(client_a);
    let mut peer_b = peer(client_b);
    let keys = [11, 22];
    let mut set = completion_set(&keys, [server_a, server_b]);
    let requests = [set.slots.raw(0), set.slots.raw(1)];
    let mut a = [0x55; 4];
    let mut b = [0x66; 4];
    let address = a.as_ptr();
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut a), Buffer::Read(&mut b)].into_iter()).unwrap();
    let wait = budget(2, TEST_NANOSECONDS);
    let mut group = set.begin(buffers, Some(&wait), None).unwrap();
    assert_eq!(
        transfer(&mut peer_a, Buffer::Write(b"one"), &wait),
        Awaited::complete(Ok(3))
    );
    let first = group.next().unwrap();
    assert_eq!((first.index, first.key), (0, keys[0]));
    assert_eq!(first.outcome, Awaited::complete(Ok(3)));
    assert_eq!(group.completed(0), Some(&b"one"[..]));
    assert!(group.completed(1).is_none());
    assert!(group.read_storage_mut(1).is_none());
    let storage = group.read_storage_mut(0).unwrap();
    assert_eq!(storage.as_ptr(), address);
    storage.fill(0x33);

    let (_, allocations) = crate::probe_runtime_allocations(|| group.submit(0, 1, 3).unwrap());
    assert_eq!(allocations, 0);
    assert!(group.read_storage_mut(0).is_none());
    assert!(group.read_storage_mut(1).is_none());
    assert_eq!(
        transfer(&mut peer_a, Buffer::Write(b"two"), &wait),
        Awaited::complete(Ok(3))
    );
    let (second, allocations) = crate::probe_runtime_allocations(|| group.next().unwrap());
    assert_eq!(allocations, 0);
    assert_eq!((second.index, second.key), (0, keys[0]));
    assert_eq!(second.outcome, Awaited::complete(Ok(3)));
    assert_eq!(group.completed(0), Some(&b"two"[..]));
    assert!(group.read_storage_mut(1).is_none());
    assert_eq!(group.read_storage_mut(0).unwrap()[0], 0x33);

    assert_eq!(
        transfer(&mut peer_b, Buffer::Write(b"end"), &wait),
        Awaited::complete(Ok(3))
    );
    let last = group.next().unwrap();
    assert_eq!((last.index, last.key), (1, keys[1]));
    assert_eq!(last.outcome, Awaited::complete(Ok(3)));
    assert_eq!(group.completed(1), Some(&b"end"[..]));
    assert!(group.next().is_none());
    assert_eq!(group.stop(), None);
    drop(group);
    assert_eq!(a, [0x33, b't', b'w', b'o']);
    assert_eq!(b, [b'e', b'n', b'd', 0x66]);
    assert_eq!(set.slots.raw(0), requests[0]);
    assert_eq!(set.slots.raw(1), requests[1]);
    assert_quiescent(&set);
}

struct ObservedWait<'s> {
    driver: Dispatcher<'s>,
    entered: Option<SyncSender<()>>,
}

// Every operation, terminal fact, cancellation and drain comes from the real
// dispatcher. The one-shot test notification marks entry into wait_any, not
// proof that the OS has already scheduled a blocked GetQueuedCompletionStatus.
unsafe impl GroupDriver for ObservedWait<'_> {
    type Request = Overlapped;
    type Timeout = u32;
    const MAXIMUM_TRANSFER_BYTES: usize = u32::MAX as usize;
    unsafe fn resource(
        &self,
        index: usize,
        request: *mut Overlapped,
    ) -> Result<crate::transfer_group::TransferResource, Failure> {
        self.driver.resource(index, request)
    }

    unsafe fn reset_request(&mut self, index: usize, request: *mut Overlapped) {
        self.driver.reset_request(index, request);
    }

    unsafe fn submit(
        &mut self,
        index: usize,
        request: *mut Overlapped,
        buffer: &mut Buffer<'_>,
    ) -> Submission {
        self.driver.submit(index, request, buffer)
    }
    unsafe fn query(&mut self, index: usize, request: *mut Overlapped) -> Query {
        self.driver.query(index, request)
    }
    fn wait_any(&mut self, timeout: u32, cancellation: Option<usize>) -> Result<Wake, Failure> {
        if let Some(entered) = self.entered.take() {
            entered
                .send(())
                .expect("the wait-entry observer was dropped");
        }
        self.driver.wait_any(timeout, cancellation)
    }
    unsafe fn cancel(
        &mut self,
        index: usize,
        request: *mut Overlapped,
    ) -> Result<(), (doria_diagnostic_catalogue::IoMessageOperation, Failure)> {
        self.driver.cancel(index, request)
    }
    unsafe fn drain(
        &mut self,
        index: usize,
        request: *mut Overlapped,
    ) -> Option<Result<usize, Failure>> {
        self.driver.drain(index, request)
    }
}

fn observed_group<'s, 'b>(
    set: &'s mut CompletionSet,
    buffers: BufferList<'b>,
    wait: &DrWaitBudgetV1,
    source: &TestClaim,
    entered: SyncSender<()>,
) -> TransferGroup<'s, 'b, ObservedWait<'s>, BrokerControl<'s>> {
    set.begin_with_driver(
        buffers,
        Some(wait),
        Some(unsafe { &*source.state() }),
        |driver| ObservedWait {
            driver,
            entered: Some(entered),
        },
    )
    .unwrap()
}

#[test]
fn cancellation_after_wait_entry_wakes_one_port_and_drains_every_pending_read() {
    let _broker = TEST_BROKER_LOCK.lock().unwrap();
    let (server_a, client_a) = connected_pair();
    let (server_b, client_b) = connected_pair();
    let (server_c, client_c) = connected_pair();
    let _peers = [client_a, client_b, client_c];
    let keys = [31, 32, 33];
    let mut set = completion_set(&keys, [server_a, server_b, server_c]);
    let source = TestClaim::new();
    let token = source.clone();
    let mut a = [0x55; 4];
    let mut b = [0x66; 4];
    let mut c = [0x77; 4];
    let buffers = BufferList::from_iter(
        [
            Buffer::Read(&mut a),
            Buffer::Read(&mut b),
            Buffer::Read(&mut c),
        ]
        .into_iter(),
    )
    .unwrap();
    let wait = budget(2, TEST_NANOSECONDS);
    let (entered_tx, entered_rx) = mpsc::sync_channel(1);
    let mut group = observed_group(&mut set, buffers, &wait, &source, entered_tx);
    drop(source); // The group registration retains its independent state claim.
    std::thread::scope(|scope| {
        scope.spawn(move || {
            let observed = entered_rx.recv_timeout(Duration::from_secs(5));
            // Always request before asserting so a failed observer cannot leave
            // pending requests waiting for peer data during fixture unwinding.
            token.request();
            assert!(observed.is_ok(), "the fixture never entered the port wait");
        });
        let mut seen = [false; 3];
        while let Some(completion) = group.next() {
            let index = completion.index;
            assert!(!seen[index]);
            seen[index] = true;
            assert_eq!(completion.key, keys[index]);
            assert_eq!(completion.outcome.stop, Some(Stop::Cancellation));
            let failure = completion.outcome.transfer.unwrap().unwrap_err();
            assert_eq!(failure.system_code, Some(995));
            assert_eq!(failure.transferred_byte_count, Some(0));
            assert_eq!(group.completed(index), Some(&[][..]));
            assert!(group.read_storage_mut(index).is_some());
        }
        assert_eq!(seen, [true; 3]);
        assert_eq!(group.stop(), Some(Stop::Cancellation));
    });
    drop(group);
    assert_eq!(a, [0x55; 4]);
    assert_eq!(b, [0x66; 4]);
    assert_eq!(c, [0x77; 4]);
    assert_quiescent(&set);
}

#[test]
fn pre_requested_cancellation_submits_nothing_and_old_value_wake_does_not_cancel_reuse() {
    let _broker = TEST_BROKER_LOCK.lock().unwrap();
    let (server_a, client_a) = connected_pair();
    let (server_b, client_b) = connected_pair();
    let mut peer_a = peer(client_a);
    let mut peer_b = peer(client_b);
    let keys = [41, 42];
    let mut set = completion_set(&keys, [server_a, server_b]);
    let source = TestClaim::new();
    source.request();
    let mut bytes = [0x55; 4];
    let wait = budget(2, TEST_NANOSECONDS);
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut bytes), Buffer::Write(b"skip")].into_iter())
            .unwrap();
    let mut group = set
        .begin(buffers, Some(&wait), Some(unsafe { &*source.state() }))
        .unwrap();
    let mut seen = [false; 2];
    while let Some(completion) = group.next() {
        assert!(!seen[completion.index]);
        seen[completion.index] = true;
        assert_eq!(completion.key, keys[completion.index]);
        assert_eq!(completion.outcome.transfer, None);
        assert_eq!(completion.outcome.stop, Some(Stop::Cancellation));
        assert!(group.completed(completion.index).is_none());
    }
    assert_eq!(seen, [true; 2]);
    drop(group);
    assert_eq!(bytes, [0x55; 4]);
    assert_quiescent(&set);

    // Do not drain the prior group's value-only cancellation wake in the test.
    // The shared poke carries no group cancellation identity: this new group's
    // own control has no requested token and must not interpret the poke as stop.
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut bytes), Buffer::Write(b"live")].into_iter())
            .unwrap();
    let mut group = set.begin(buffers, Some(&wait), None).unwrap();
    assert_eq!(
        transfer(&mut peer_a, Buffer::Write(b"live"), &wait),
        Awaited::complete(Ok(4))
    );
    let mut received = [0; 4];
    assert_eq!(
        transfer(&mut peer_b, Buffer::Read(&mut received), &wait),
        Awaited::complete(Ok(4))
    );
    assert_eq!(&received, b"live");
    let mut seen = [false; 2];
    while let Some(completion) = group.next() {
        assert!(!seen[completion.index]);
        seen[completion.index] = true;
        assert_eq!(completion.key, keys[completion.index]);
        assert_eq!(completion.outcome, Awaited::complete(Ok(4)));
        assert_eq!(group.completed(completion.index), Some(&b"live"[..]));
    }
    assert_eq!(seen, [true; 2]);
    assert_eq!(group.stop(), None);
    drop(group);
    assert_eq!(&bytes, b"live");
    assert_quiescent(&set);
}

#[test]
fn dropping_group_drains_all_packets_before_same_handles_slots_and_buffers_are_reused() {
    let _broker = TEST_BROKER_LOCK.lock().unwrap();
    let (server_a, client_a) = connected_pair();
    let (server_b, client_b) = connected_pair();
    let (server_c, client_c) = connected_pair();
    let mut peers = [peer(client_a), peer(client_b), peer(client_c)];
    let keys = [51, 52, 53];
    let mut set = completion_set(&keys, [server_a, server_b, server_c]);
    let requests = std::array::from_fn::<_, 3, _>(|index| set.slots.raw(index));
    let handles = std::array::from_fn::<_, 3, _>(|index| set.owners.as_slice()[index].handle());
    let mut a = [0x55; 4];
    let mut b = [0x66; 4];
    let mut c = [0x77; 4];
    let buffers = BufferList::from_iter(
        [
            Buffer::Read(&mut a),
            Buffer::Read(&mut b),
            Buffer::Read(&mut c),
        ]
        .into_iter(),
    )
    .unwrap();
    let wait = budget(2, TEST_NANOSECONDS);
    let group = set.begin(buffers, Some(&wait), None).unwrap();
    let (_, closes) = crate::native_handle::with_close_probe(|| drop(group));
    assert_eq!(closes, 0, "ending a group must not close its owned set");
    for (index, owner) in set.owners.as_slice().iter().enumerate() {
        let mut count = u32::MAX;
        // Group teardown removes the value-only routing cache. The retired
        // native request still reports its exact canceled read result without
        // pumping, waiting, or borrowing any caller buffer again.
        let success = unsafe {
            GetOverlappedResult(owner.handle(), set.slots.raw(index).cast(), &mut count, 0)
        };
        let code = unsafe { GetLastError() };
        assert_eq!(success, 0);
        assert_eq!(code, 995);
        assert_eq!(count, 0);
    }
    assert_quiescent(&set);
    a.fill(0x11);
    b.fill(0x22);
    c.fill(0x33);

    let buffers = BufferList::from_iter(
        [
            Buffer::Read(&mut a),
            Buffer::Read(&mut b),
            Buffer::Read(&mut c),
        ]
        .into_iter(),
    )
    .unwrap();
    let mut group = set.begin(buffers, Some(&wait), None).unwrap();
    let payloads: [&[u8]; 3] = [b"one", b"two", b"tri"];
    for (peer, payload) in peers.iter_mut().zip(payloads) {
        assert_eq!(
            transfer(peer, Buffer::Write(payload), &wait),
            Awaited::complete(Ok(payload.len()))
        );
    }
    let mut seen = [false; 3];
    while let Some(completion) = group.next() {
        let index = completion.index;
        assert!(!seen[index]);
        seen[index] = true;
        assert_eq!(completion.key, keys[index]);
        assert_eq!(completion.outcome, Awaited::complete(Ok(3)));
        assert_eq!(group.completed(index), Some(payloads[index]));
    }
    assert_eq!(seen, [true; 3]);
    drop(group);
    assert_eq!(a, [b'o', b'n', b'e', 0x11]);
    assert_eq!(b, [b't', b'w', b'o', 0x22]);
    assert_eq!(c, [b't', b'r', b'i', 0x33]);
    for index in 0..3 {
        assert_eq!(set.slots.raw(index), requests[index]);
        assert_eq!(set.owners.as_slice()[index].handle(), handles[index]);
    }
    assert_quiescent(&set);
    let (_, closes) = crate::native_handle::with_close_probe(|| drop(set));
    // The shared port is a private kernel object; only stream closes count.
    assert_eq!(closes, 3, "three attached streams close exactly once");
}

#[test]
fn peer_closure_projects_read_eof_without_finishing_an_unrelated_pending_read() {
    let _broker = TEST_BROKER_LOCK.lock().unwrap();
    let (server_a, client_a) = connected_pair();
    let (server_b, client_b) = connected_pair();
    drop(client_a);
    let mut peer_b = peer(client_b);
    let keys = [61, 62];
    let mut set = completion_set(&keys, [server_a, server_b]);
    let mut closed = [0x55; 4];
    let mut live = [0x66; 4];
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut closed), Buffer::Read(&mut live)].into_iter())
            .unwrap();
    let wait = budget(2, TEST_NANOSECONDS);
    let mut group = set.begin(buffers, Some(&wait), None).unwrap();
    let completion = group.next().unwrap();
    assert_eq!((completion.index, completion.key), (0, keys[0]));
    assert_eq!(completion.outcome.stop, None);
    let mut status = DrCompletionStatusV1::default();
    assert_eq!(project_completion(&mut status, completion.outcome, true), 2);
    assert_eq!(status.stop, 0);
    assert_eq!(status.transfer.has_system_code, 0);
    assert_eq!(status.transfer.has_transferred_byte_count, 0);
    assert!(group.completed(1).is_none());
    assert!(group.read_storage_mut(1).is_none());
    assert_eq!(
        transfer(&mut peer_b, Buffer::Write(b"ok"), &wait),
        Awaited::complete(Ok(2))
    );
    let completion = group.next().unwrap();
    assert_eq!((completion.index, completion.key), (1, keys[1]));
    assert_eq!(completion.outcome, Awaited::complete(Ok(2)));
    assert_eq!(group.completed(1), Some(&b"ok"[..]));
    assert!(group.next().is_none());
    assert_eq!(group.stop(), None);
    drop(group);
    assert_eq!(closed, [0x55; 4]);
    assert_eq!(live, [b'o', b'k', 0x66, 0x66]);
    assert_quiescent(&set);
}
