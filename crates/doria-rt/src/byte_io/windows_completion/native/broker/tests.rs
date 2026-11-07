//! Genuine Windows fixtures. Cross-compilation checks the source only; these
//! assertions require the Windows kernel, not the host routing model.

use std::os::windows::io::{IntoRawHandle, OwnedHandle as StdHandle};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, MutexGuard};
use std::time::{Duration, Instant};

use super::super::tests::{budget, connected_pair, TestClaim};
use super::*;
use crate::transfer_group::{BufferList, GroupDriver};

const TEST_WAIT: Duration = Duration::from_secs(5);
const TEST_NANOSECONDS: u64 = 5_000_000_000;

#[test]
fn one_attachment_routes_distinct_duplex_operations_without_reassociation() {
    let _serial = serial();
    let (server, client) = connected_pair();
    let owner = attached(server);
    let identity = owner.identity_for_test();
    let mut peer = peer(client);
    let mut set = configured(&[81], [owner]);
    let mut first_routes = None;
    for _ in 0..3 {
        let wait = budget(2, TEST_NANOSECONDS);
        let mut input = [0xa5; 4];
        let pointer = input.as_ptr();
        let buffers =
            BufferList::from_iter([Buffer::Read(&mut input), Buffer::Write(b"ping")].into_iter())
                .unwrap();
        let mut group = set
            .begin_selected(buffers, Some(&wait), None, selected_indices([0, 0]))
            .unwrap();
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
        let ((), allocations) = crate::probe_runtime_allocations(|| {
            while let Some(completion) = group.next() {
                assert_eq!(completion.key, 81);
                assert!(!seen[completion.index]);
                seen[completion.index] = true;
                assert_eq!(completion.outcome, Awaited::complete(Ok(4)));
            }
        });
        assert_eq!(allocations, 0);
        assert_eq!(seen, [true; 2]);
        assert_eq!(group.completed(0).unwrap(), b"pong");
        assert_eq!(group.read_storage_mut(0).unwrap().as_ptr(), pointer);
        drop(group);
        set.assert_retired_for_test();
        assert_eq!(set.owners.as_slice()[0].identity_for_test(), identity);
        let routes = [
            set.operation_snapshot_for_test(0, 0),
            set.operation_snapshot_for_test(0, 1),
        ];
        assert_ne!(routes[0].0, routes[1].0);
        assert_ne!(routes[0].1, routes[1].1);
        if let Some(first) = first_routes {
            assert_eq!(routes, first);
        } else {
            first_routes = Some(routes);
        }
    }
}

#[test]
fn one_attachment_arbitrary_operation_slots_conserve_identity_not_order() {
    let _serial = serial();
    let (server, client) = connected_pair();
    let mut set = configured(&[82], [attached(server)]);
    let mut peer = peer(client);
    let payloads = [[6], [2], [5], [1], [4], [3]];
    let buffers = BufferList::from_iter(payloads.iter().map(|bytes| Buffer::Write(bytes))).unwrap();
    let wait = budget(2, TEST_NANOSECONDS);
    let mut group = set
        .begin_selected(buffers, Some(&wait), None, selected_indices([0; 6]))
        .unwrap();
    let mut output = [0; 6];
    let mut seen = [false; 6];
    while let Some(completion) = group.next() {
        assert_eq!(completion.key, 82);
        assert!(!seen[completion.index]);
        seen[completion.index] = true;
        assert_eq!(completion.outcome, Awaited::complete(Ok(1)));
    }
    assert!(seen.into_iter().all(|present| present));
    // Queued writes advance when their predecessor retires through the group.
    let mut offset = 0;
    while offset < output.len() {
        let result = transfer(&mut peer, Buffer::Read(&mut output[offset..]))
            .transfer
            .unwrap()
            .unwrap();
        assert!(result != 0 && result <= output.len() - offset);
        offset += result;
    }
    output.sort_unstable();
    assert_eq!(output, [1, 2, 3, 4, 5, 6]);
    drop(group);
    set.assert_retired_for_test();
}

#[test]
fn competing_pipe_reads_assign_bytes_in_plan_order_and_reuse_one_association() {
    let _serial = serial();
    let (server, client) = connected_pair();
    let mut set = configured(&[82], [attached(server)]);
    let mut peer = peer(client);
    let (mut a, mut b) = ([0xa5; 3], [0xb5; 3]);
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut a), Buffer::Read(&mut b)].into_iter()).unwrap();
    let wait = budget(2, TEST_NANOSECONDS);
    let mut group = set
        .begin_selected(buffers, Some(&wait), None, selected_indices([0; 2]))
        .unwrap();
    assert!(group.read_storage_mut(1).is_none());
    assert_eq!(
        transfer(&mut peer, Buffer::Write(b"abcdef")),
        Awaited::complete(Ok(6))
    );
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        for _ in 0..2 {
            assert_eq!(group.next().unwrap().outcome, Awaited::complete(Ok(3)));
        }
        assert_eq!(group.completed(0).unwrap(), b"abc");
        assert_eq!(group.completed(1).unwrap(), b"def");
        group.submit(0, 0, 3).unwrap();
        group.submit(1, 0, 3).unwrap();
    });
    assert_eq!(allocations, 0);
    assert_eq!(
        transfer(&mut peer, Buffer::Write(b"ghijkl")),
        Awaited::complete(Ok(6))
    );
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        for _ in 0..2 {
            assert_eq!(group.next().unwrap().outcome, Awaited::complete(Ok(3)));
        }
        assert_eq!(group.completed(0).unwrap(), b"ghi");
        assert_eq!(group.completed(1).unwrap(), b"jkl");
        assert!(group.next().is_none());
        drop(group);
    });
    assert_eq!(allocations, 0);
    set.assert_retired_for_test();
}

#[test]
fn cancelling_competing_reads_drains_only_the_submitted_packet_and_keeps_queued_storage() {
    let _serial = serial();
    let (server, _client) = connected_pair();
    let mut set = configured(&[82], [attached(server)]);
    let (mut a, mut b) = ([0xa5; 3], [0xb5; 3]);
    let buffers =
        BufferList::from_iter([Buffer::Read(&mut a), Buffer::Read(&mut b)].into_iter()).unwrap();
    let wait = budget(0, 0);
    let before = transport::pending_native_dequeues();
    let mut group = set
        .begin_selected(buffers, Some(&wait), None, selected_indices([0; 2]))
        .unwrap();
    assert!(group.read_storage_mut(1).is_none());
    let mut seen = [false; 2];
    while let Some(completion) = group.next() {
        seen[completion.index] = true;
        assert_eq!(completion.outcome.stop, Some(Stop::BudgetExhausted));
        if completion.index == 1 {
            assert_eq!(completion.outcome.transfer, None);
            assert_eq!(completion.outcome.cancellation_failure, None);
            assert!(group.completed(1).is_none());
        }
    }
    assert_eq!(seen, [true; 2]);
    assert_eq!(transport::pending_native_dequeues() - before, 1);
    drop(group);
    assert_eq!(b, [0xb5; 3]);
    set.assert_retired_for_test();
}

fn serial() -> MutexGuard<'static, ()> {
    TEST_BROKER_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn attached(handle: StdHandle) -> AttachedStream {
    // The shared factory creates independent, idle overlapped byte pipes with
    // no previous IOCP/APC/notification-suppression or external owner.
    unsafe { AttachedStream::adopt(OwnedHandle::from_raw(handle.into_raw_handle())).unwrap() }
}

fn configured<const N: usize>(keys: &[u64; N], owners: [AttachedStream; N]) -> CompletionSet {
    let mut storage = FixedSlice::new(N).unwrap();
    for owner in owners {
        storage.push(owner).unwrap();
    }
    CompletionSet::from_attached(keys, storage).unwrap()
}

fn selected_indices<const N: usize>(indices: [usize; N]) -> FixedSlice<usize> {
    let mut selection = FixedSlice::new(N).unwrap();
    for index in indices {
        selection.push(index).unwrap();
    }
    selection
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

fn lifecycle_snapshot() -> (usize, usize) {
    let state = lifecycle().lock();
    (state.weak as usize, state.claims)
}

fn cached(claim: &BrokerClaim, group: usize) -> bool {
    let routing = claim.broker().routing().lock();
    // Registered nodes remain pinned until removal under this same lock;
    // group_ready tests membership before dereferencing a numeric identity.
    // Query would consume readiness and perturb the fixture.
    unsafe { routing.registry.group_ready(group as *mut Group) }
}

#[test]
fn stream_owners_regroup_from_two_former_sets_without_native_reattachment() {
    let _serial = serial();
    let (server_a, client_a) = connected_pair();
    let (server_b, client_b) = connected_pair();
    let owner_a = attached(server_a);
    let owner_b = attached(server_b);
    let identities = [owner_a.identity_for_test(), owner_b.identity_for_test()];
    assert_ne!(identities[0].1, identities[1].1);
    assert_eq!(identities[0].2, identities[1].2);
    let mut peer_a = peer(client_a);
    let mut peer_b = peer(client_b);
    let mut set_a = configured(&[101], [owner_a]);
    let mut set_b = configured(&[202], [owner_b]);
    let wait = budget(2, TEST_NANOSECONDS);
    let (mut bytes_a, mut bytes_b) = ([0_u8; 8], [0_u8; 8]);
    let mut group_a = set_a
        .begin(
            BufferList::from_iter([Buffer::Read(&mut bytes_a)].into_iter()).unwrap(),
            Some(&wait),
            None,
        )
        .unwrap();
    let mut group_b = set_b
        .begin(
            BufferList::from_iter([Buffer::Read(&mut bytes_b)].into_iter()).unwrap(),
            Some(&wait),
            None,
        )
        .unwrap();
    assert_eq!(
        transfer(&mut peer_a, Buffer::Write(b"a")),
        Awaited::complete(Ok(1))
    );
    assert_eq!(
        transfer(&mut peer_b, Buffer::Write(b"b")),
        Awaited::complete(Ok(1))
    );
    let completion_a = group_a.next().unwrap();
    let completion_b = group_b.next().unwrap();
    assert_eq!(
        (completion_a.key, completion_a.outcome),
        (101, Awaited::complete(Ok(1)))
    );
    assert_eq!(
        (completion_b.key, completion_b.outcome),
        (202, Awaited::complete(Ok(1)))
    );
    assert_eq!(group_a.completed(0), Some(b"a".as_slice()));
    assert_eq!(group_b.completed(0), Some(b"b".as_slice()));
    drop((group_a, group_b));
    set_a.assert_retired_for_test();
    set_b.assert_retired_for_test();

    let mut owners = FixedSlice::new(2).unwrap();
    for owner in set_a
        .into_attached()
        .into_iter()
        .chain(set_b.into_attached())
    {
        owners.push(owner).unwrap();
    }
    assert_eq!(owners.as_slice()[0].identity_for_test(), identities[0]);
    assert_eq!(owners.as_slice()[1].identity_for_test(), identities[1]);
    let mut combined = CompletionSet::from_attached(&[101, 202], owners).unwrap();
    let wait = budget(2, TEST_NANOSECONDS);
    let mut group = combined
        .begin(
            BufferList::from_iter(
                [Buffer::Read(&mut bytes_a), Buffer::Read(&mut bytes_b)].into_iter(),
            )
            .unwrap(),
            Some(&wait),
            None,
        )
        .unwrap();
    assert_eq!(
        transfer(&mut peer_a, Buffer::Write(b"new-a")),
        Awaited::complete(Ok(5))
    );
    assert_eq!(
        transfer(&mut peer_b, Buffer::Write(b"new-b")),
        Awaited::complete(Ok(5))
    );
    let mut seen = [false; 2];
    for _ in 0..2 {
        let completion = group.next().unwrap();
        assert!(!seen[completion.index]);
        seen[completion.index] = true;
        assert_eq!(completion.key, [101, 202][completion.index]);
        assert_eq!(completion.outcome, Awaited::complete(Ok(5)));
    }
    assert_eq!(group.completed(0), Some(b"new-a".as_slice()));
    assert_eq!(group.completed(1), Some(b"new-b".as_slice()));
    assert!(group.next().is_none());
    drop(group);
    combined.assert_retired_for_test();
    let owners = combined.into_attached();
    assert_eq!(owners.as_slice()[0].identity_for_test(), identities[0]);
    assert_eq!(owners.as_slice()[1].identity_for_test(), identities[1]);
}

#[test]
fn simultaneous_groups_cache_foreign_completions_without_stealing_them() {
    let _serial = serial();
    let (server_a, client_a) = connected_pair();
    let (server_b, client_b) = connected_pair();
    let mut set_a = configured(&[301], [attached(server_a)]);
    let mut set_b = configured(&[302], [attached(server_b)]);
    let broker = BrokerClaim::acquire().unwrap();
    let group_a_address = set_a.snapshot_for_test(0).1;
    let group_b_address = set_b.snapshot_for_test(0).1;
    let mut peer_a = peer(client_a);
    let mut peer_b = peer(client_b);
    let (mut bytes_a, mut bytes_b) = ([0_u8; 8], [0_u8; 8]);
    let cache_deadline = Instant::now() + TEST_WAIT;
    let wait = budget(2, TEST_NANOSECONDS);
    let mut group_a = set_a
        .begin(
            BufferList::from_iter([Buffer::Read(&mut bytes_a)].into_iter()).unwrap(),
            Some(&wait),
            None,
        )
        .unwrap();
    let mut group_b = set_b
        .begin(
            BufferList::from_iter([Buffer::Read(&mut bytes_b)].into_iter()).unwrap(),
            Some(&wait),
            None,
        )
        .unwrap();
    // A peer write's return is not an ordering guarantee for the remote READ
    // packet. Observe B's real cached fact before producing any A data instead.
    // A's normal elected pump must retain B's fact, never steal or repost it.
    assert_eq!(
        transfer(&mut peer_b, Buffer::Write(b"foreign")),
        Awaited::complete(Ok(7))
    );
    while !cached(&broker, group_b_address) {
        let remaining = cache_deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "B's packet was not routed within its bound"
        );
        let milliseconds = remaining.as_millis().min(u128::from(INFINITE - 1)) as u32;
        unsafe {
            broker.broker().wait(
                group_a_address as *mut Group,
                milliseconds,
                ptr::null(),
                None,
            )
        }
        .unwrap();
    }
    assert_eq!(
        transfer(&mut peer_a, Buffer::Write(b"local")),
        Awaited::complete(Ok(5))
    );
    let own = group_a.next().unwrap();
    assert_eq!((own.key, own.outcome), (301, Awaited::complete(Ok(5))));
    assert_eq!(group_a.completed(0), Some(b"local".as_slice()));
    assert!(cached(&broker, group_b_address));
    let foreign = group_b.next().unwrap();
    assert_eq!(
        (foreign.key, foreign.outcome),
        (302, Awaited::complete(Ok(7)))
    );
    assert_eq!(group_b.completed(0), Some(b"foreign".as_slice()));
    assert!(!cached(&broker, group_b_address));
    assert!(group_a.next().is_none());
    assert!(group_b.next().is_none());
    drop((group_a, group_b));
    set_a.assert_retired_for_test();
    set_b.assert_retired_for_test();
}

#[test]
fn cancelling_one_group_does_not_cancel_another_group_on_the_same_broker() {
    let _serial = serial();
    let (server_a, _client_a) = connected_pair();
    let (server_b, client_b) = connected_pair();
    let mut set_a = configured(&[401], [attached(server_a)]);
    let mut set_b = configured(&[402], [attached(server_b)]);
    let source_a = TestClaim::new();
    let source_b = TestClaim::new();
    let (mut bytes_a, mut bytes_b) = ([0x55_u8; 8], [0x66_u8; 8]);
    let wait = budget(2, TEST_NANOSECONDS);
    let mut group_a = set_a
        .begin(
            BufferList::from_iter([Buffer::Read(&mut bytes_a)].into_iter()).unwrap(),
            Some(&wait),
            Some(unsafe { &*source_a.state() }),
        )
        .unwrap();
    let mut group_b = set_b
        .begin(
            BufferList::from_iter([Buffer::Read(&mut bytes_b)].into_iter()).unwrap(),
            Some(&wait),
            Some(unsafe { &*source_b.state() }),
        )
        .unwrap();
    source_a.request();
    let cancelled = group_a.next().unwrap();
    assert_eq!(cancelled.key, 401);
    assert_eq!(cancelled.outcome.stop, Some(Stop::Cancellation));
    assert_eq!(cancelled.outcome.cancellation_failure, None);
    assert_eq!(
        cancelled.outcome.transfer,
        Some(Err(
            Failure::from_system_code(995).with_transferred_byte_count(Some(0))
        ))
    );
    assert!(group_a.next().is_none());
    assert!(group_b.read_storage_mut(0).is_none());
    assert_eq!(
        unsafe { cancellation::dr_v1_cancellation_is_requested(source_b.state()) },
        0
    );
    let mut peer_b = peer(client_b);
    assert_eq!(
        transfer(&mut peer_b, Buffer::Write(b"ok")),
        Awaited::complete(Ok(2))
    );
    let unaffected = group_b.next().unwrap();
    assert_eq!(
        (unaffected.key, unaffected.outcome),
        (402, Awaited::complete(Ok(2)))
    );
    assert_eq!(group_b.stop(), None);
    assert_eq!(group_b.completed(0), Some(b"ok".as_slice()));
    drop((group_a, group_b));
    set_a.assert_retired_for_test();
    set_b.assert_retired_for_test();
    assert_eq!(bytes_a, [0x55; 8]);
}

struct DrainReport {
    stable: bool,
    first: [u8; 8],
    second: [u8; 8],
}

struct ReuseReport {
    stable: bool,
    completed: [bool; 2],
    first: [u8; 8],
    second: [u8; 8],
}

#[test]
fn drop_drains_pending_earlier_slot_despite_cached_later_slot_then_reuses_storage() {
    let _serial = serial();
    let (server_a, client_a) = connected_pair();
    let (server_b, client_b) = connected_pair();
    let (server_controller, client_controller) = connected_pair();
    let owners = [attached(server_a), attached(server_b)];
    let broker = BrokerClaim::acquire().unwrap();
    let mut controller = configured(&[599], [attached(server_controller)]);
    let controller_address = controller.snapshot_for_test(0).1;
    let mut controller_bytes = [0_u8; 8];
    let controller_wait = budget(2, TEST_NANOSECONDS);
    let mut controller_group = controller
        .begin(
            BufferList::from_iter([Buffer::Read(&mut controller_bytes)].into_iter()).unwrap(),
            Some(&controller_wait),
            None,
        )
        .unwrap();
    let (started_tx, started_rx) = mpsc::sync_channel(1);
    let (cached_tx, cached_rx) = mpsc::sync_channel(1);
    let (drained_tx, drained_rx) = mpsc::sync_channel(1);
    let (reuse_tx, reuse_rx) = mpsc::sync_channel(1);
    let (finished_tx, finished_rx) = mpsc::sync_channel(1);
    // Own every buffer, slot, handle and claim inside the worker. A regression
    // cannot strand borrowed parent storage when the watchdog reports failure.
    let worker = std::thread::spawn(move || {
        let mut set = configured(&[501, 502], owners);
        let snapshots = [set.snapshot_for_test(0), set.snapshot_for_test(1)];
        let identities = [
            set.owners.as_slice()[0].identity_for_test(),
            set.owners.as_slice()[1].identity_for_test(),
        ];
        let (mut first, mut second) = ([0x55_u8; 8], [0x66_u8; 8]);
        let wait = budget(2, TEST_NANOSECONDS);
        let group = set
            .begin(
                BufferList::from_iter(
                    [Buffer::Read(&mut first), Buffer::Read(&mut second)].into_iter(),
                )
                .unwrap(),
                Some(&wait),
                None,
            )
            .unwrap();
        if started_tx.send(snapshots[0].1).is_err() {
            return;
        }
        let _ = cached_rx.recv_timeout(TEST_WAIT);
        // Index zero remains pending; index one is cached but unconsumed. Drop
        // must cancel all requests and pump specifically for index zero's fact.
        drop(group);
        set.assert_retired_for_test();
        if drained_tx
            .send(DrainReport {
                stable: snapshots == [set.snapshot_for_test(0), set.snapshot_for_test(1)],
                first,
                second,
            })
            .is_err()
        {
            return;
        }
        if reuse_rx.recv_timeout(TEST_WAIT).is_err() {
            return;
        }
        first.fill(0x11);
        second.fill(0x22);
        let wait = budget(2, TEST_NANOSECONDS);
        let mut group = set
            .begin(
                BufferList::from_iter(
                    [Buffer::Read(&mut first), Buffer::Read(&mut second)].into_iter(),
                )
                .unwrap(),
                Some(&wait),
                None,
            )
            .unwrap();
        let mut completed = [false; 2];
        for _ in 0..2 {
            if let Some(completion) = group.next() {
                completed[completion.index] = completion.key == [501, 502][completion.index]
                    && completion.outcome == Awaited::complete(Ok(5));
            }
        }
        drop(group);
        set.assert_retired_for_test();
        let stable = snapshots == [set.snapshot_for_test(0), set.snapshot_for_test(1)]
            && identities
                == [
                    set.owners.as_slice()[0].identity_for_test(),
                    set.owners.as_slice()[1].identity_for_test(),
                ];
        // A final report is sent only after all packet obligations and owner
        // cleanup have finished, so joining after receiving it is bounded work.
        drop(set);
        let _ = finished_tx.send(ReuseReport {
            stable,
            completed,
            first,
            second,
        });
    });
    let mut peer_a = peer(client_a);
    let mut peer_b = peer(client_b);
    let mut peer_controller = peer(client_controller);
    let started = started_rx.recv_timeout(TEST_WAIT);
    let write_old = transfer(&mut peer_b, Buffer::Write(b"old"));
    let cache_ready = started.as_ref().ok().is_some_and(|address| {
        let woke = unsafe {
            broker
                .broker()
                .wait(controller_address as *mut Group, 5000, ptr::null(), None)
        };
        woke == Ok(Wake::Completion) && cached(&broker, *address)
    });
    let _ = cached_tx.send(());
    // One original watchdog deadline covers initial observation and recovery.
    // Recovery uses the elected pump and a live controller route, never receive
    // directly. Running recovery is itself a failed assertion, not a pass.
    let retirement_deadline = Instant::now() + TEST_WAIT + TEST_WAIT;
    let mut drained = drained_rx.recv_timeout(TEST_WAIT);
    let recovery_ran = drained.is_err();
    if recovery_ran {
        let remaining = retirement_deadline.saturating_duration_since(Instant::now());
        let milliseconds = remaining.as_millis().min(u128::from(INFINITE - 1)) as u32;
        let _ = unsafe {
            broker.broker().wait(
                controller_address as *mut Group,
                milliseconds,
                ptr::null(),
                None,
            )
        };
        drained =
            drained_rx.recv_timeout(retirement_deadline.saturating_duration_since(Instant::now()));
    }
    let _ = reuse_tx.send(());
    let write_a = transfer(&mut peer_a, Buffer::Write(b"new-a"));
    let write_b = transfer(&mut peer_b, Buffer::Write(b"new-b"));
    let finished = finished_rx.recv_timeout(TEST_WAIT);
    // Complete the controller normally before any assertion can unwind. Its
    // wait may legitimately cache a worker packet, still through pump election.
    let controller_write = transfer(&mut peer_controller, Buffer::Write(b"done"));
    let controller_completion = controller_group.next();
    drop(controller_group);
    controller.assert_retired_for_test();
    let joined = if finished.is_ok() || worker.is_finished() {
        Some(worker.join())
    } else {
        // A still-pending worker owns its entire pinned frame. Do not join an
        // unbounded drain or manufacture retirement by closing aliases to it.
        drop(worker);
        None
    };
    assert!(started.is_ok(), "worker did not configure within its bound");
    assert_eq!(write_old, Awaited::complete(Ok(3)));
    assert!(cache_ready, "later-slot packet was not cached before Drop");
    assert!(!recovery_ran, "Drop needed controller watchdog recovery");
    let drained = drained.expect("Drop did not retire the owned pending frame");
    assert!(drained.stable);
    assert_eq!(drained.first, [0x55; 8]);
    assert_eq!(&drained.second[..3], b"old");
    assert_eq!(write_a, Awaited::complete(Ok(5)));
    assert_eq!(write_b, Awaited::complete(Ok(5)));
    assert_eq!(controller_write, Awaited::complete(Ok(4)));
    assert_eq!(
        controller_completion.unwrap().outcome,
        Awaited::complete(Ok(4))
    );
    let finished = finished.expect("reused group did not finish within its bound");
    assert!(finished.stable);
    assert_eq!(finished.completed, [true; 2]);
    assert_eq!(&finished.first[..5], b"new-a");
    assert_eq!(&finished.second[..5], b"new-b");
    assert!(joined.expect("worker still owns a pending frame").is_ok());
}

#[test]
fn completion_and_terminal_resubmission_allocate_no_runtime_storage() {
    let _serial = serial();
    let (server_a, client_a) = connected_pair();
    let (server_b, client_b) = connected_pair();
    let mut set = configured(&[601, 602], [attached(server_a), attached(server_b)]);
    let snapshots = [set.snapshot_for_test(0), set.snapshot_for_test(1)];
    let (mut first, mut second) = ([0_u8; 8], [0_u8; 8]);
    let source = TestClaim::new();
    let wait = budget(2, TEST_NANOSECONDS);
    // Buffer/configuration storage and the cancellation registration are
    // deliberately outside the transfer-path allocation probe.
    let mut group = set
        .begin(
            BufferList::from_iter(
                [Buffer::Read(&mut first), Buffer::Read(&mut second)].into_iter(),
            )
            .unwrap(),
            Some(&wait),
            Some(unsafe { &*source.state() }),
        )
        .unwrap();
    assert_eq!(group.request_info(0), Some((true, 8)));
    assert_eq!(group.request_info(1), Some((true, 8)));
    assert_eq!(group.request_info(2), None);
    let mut peer_a = peer(client_a);
    let mut peer_b = peer(client_b);
    assert_eq!(
        transfer(&mut peer_a, Buffer::Write(b"one")),
        Awaited::complete(Ok(3))
    );
    let (completion, allocations) = crate::probe_runtime_allocations(|| group.next().unwrap());
    assert_eq!(allocations, 0);
    assert_eq!(
        (completion.index, completion.key, completion.outcome),
        (0, 601, Awaited::complete(Ok(3)))
    );
    assert_eq!(group.completed(0), Some(b"one".as_slice()));
    assert!(group.read_storage_mut(1).is_none());
    let (reuse, allocations) = crate::probe_runtime_allocations(|| {
        group.read_storage_mut(0).unwrap().fill(0x77);
        group.submit(0, 1, 5)
    });
    assert_eq!(allocations, 0);
    assert_eq!(reuse, Ok(()));
    assert_eq!(group.request_info(0), Some((true, 5)));
    assert_eq!(group.request_info(1), Some((true, 8)));
    assert!(group.completed(0).is_none());
    assert_eq!(
        transfer(&mut peer_a, Buffer::Write(b"again")),
        Awaited::complete(Ok(5))
    );
    let (completion, allocations) = crate::probe_runtime_allocations(|| group.next().unwrap());
    assert_eq!(allocations, 0);
    assert_eq!(
        (completion.index, completion.key, completion.outcome),
        (0, 601, Awaited::complete(Ok(5)))
    );
    assert_eq!(group.completed(0), Some(b"again".as_slice()));
    assert_eq!(
        transfer(&mut peer_b, Buffer::Write(b"other")),
        Awaited::complete(Ok(5))
    );
    let (completion, allocations) = crate::probe_runtime_allocations(|| group.next().unwrap());
    assert_eq!(allocations, 0);
    assert_eq!(
        (completion.index, completion.key, completion.outcome),
        (1, 602, Awaited::complete(Ok(5)))
    );
    assert_eq!(group.completed(1), Some(b"other".as_slice()));
    let ((), allocations) = crate::probe_runtime_allocations(|| drop(group));
    assert_eq!(allocations, 0);
    set.assert_retired_for_test();
    assert_eq!(
        snapshots,
        [set.snapshot_for_test(0), set.snapshot_for_test(1)]
    );
    assert_eq!(first[0], 0x77);
    assert_eq!(&first[1..6], b"again");
}

struct TestEvent(OwnedHandle);

// Manual-reset event use is natively concurrent; Arc retains the unique close
// owner until both thread operations have returned, including watchdog failure.
unsafe impl Send for TestEvent {}
unsafe impl Sync for TestEvent {}

impl TestEvent {
    fn new() -> Arc<Self> {
        let raw = unsafe { CreateEventW(ptr::null_mut(), 1, 0, ptr::null()) };
        assert!(!raw.is_null());
        Arc::new(Self(unsafe { OwnedHandle::from_raw(raw) }))
    }

    fn signal(&self) -> bool {
        unsafe { SetEvent(self.0.get()) != 0 }
    }

    fn wait(&self) -> bool {
        unsafe { wait_for_single_object(self.0.get(), 5000, "a test event") == 0 }
    }
}

struct DoneOnDrop(Arc<AtomicBool>);

impl Drop for DoneOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

struct CallerReport {
    outcome: Option<Awaited>,
    key: Option<u64>,
    bytes: [u8; 8],
    dequeues: usize,
    synchronization: bool,
    other_finished_while_runnable: bool,
}

struct EnteredWait<'a> {
    inner: driver::Dispatcher<'a>,
    entry: Option<mpsc::SyncSender<()>>,
}

// This wrapper changes no native request, query, cancellation, or drain path.
// Its one notification proves method entry, not a blocked kernel wait.
unsafe impl GroupDriver for EnteredWait<'_> {
    type Request = Overlapped;
    type Timeout = u32;
    const MAXIMUM_TRANSFER_BYTES: usize = u32::MAX as usize;
    unsafe fn resource(
        &self,
        index: usize,
        request: *mut Overlapped,
    ) -> Result<crate::transfer_group::TransferResource, Failure> {
        self.inner.resource(index, request)
    }

    unsafe fn reset_request(&mut self, index: usize, request: *mut Overlapped) {
        unsafe { self.inner.reset_request(index, request) };
    }

    unsafe fn submit(
        &mut self,
        index: usize,
        request: *mut Overlapped,
        buffer: &mut Buffer<'_>,
    ) -> Submission {
        unsafe { self.inner.submit(index, request, buffer) }
    }

    unsafe fn query(&mut self, index: usize, request: *mut Overlapped) -> Query {
        unsafe { self.inner.query(index, request) }
    }

    fn wait_any(&mut self, timeout: u32, cancellation: Option<usize>) -> Result<Wake, Failure> {
        if let Some(entry) = self.entry.take() {
            let _ = entry.send(());
        }
        self.inner.wait_any(timeout, cancellation)
    }

    unsafe fn cancel(
        &mut self,
        index: usize,
        request: *mut Overlapped,
    ) -> Result<(), (doria_diagnostic_catalogue::IoMessageOperation, Failure)> {
        unsafe { self.inner.cancel(index, request) }
    }

    unsafe fn drain(
        &mut self,
        index: usize,
        request: *mut Overlapped,
    ) -> Option<Result<usize, Failure>> {
        unsafe { self.inner.drain(index, request) }
    }
}

fn waiting_caller(
    owner: AttachedStream,
    key: u64,
    entry: mpsc::SyncSender<()>,
    report: mpsc::SyncSender<CallerReport>,
) -> std::thread::JoinHandle<()> {
    // The caller constructs non-Send configuration and loans on its own stack.
    // Only the already-Send unique stream owner crosses the thread boundary.
    std::thread::spawn(move || {
        let mut set = configured(&[key], [owner]);
        let mut bytes = [0_u8; 8];
        let wait = budget(2, TEST_NANOSECONDS);
        let mut group = set
            .begin_with_driver(
                BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap(),
                Some(&wait),
                None,
                |inner| EnteredWait {
                    inner,
                    entry: Some(entry),
                },
            )
            .unwrap();
        let before = native_dequeues_for_test();
        let completion = group.next();
        let dequeues = native_dequeues_for_test() - before;
        let (key, outcome) = completion.map_or((None, None), |completion| {
            (Some(completion.key), Some(completion.outcome))
        });
        drop(group);
        set.assert_retired_for_test();
        drop(set);
        let _ = report.send(CallerReport {
            outcome,
            key,
            bytes,
            dequeues,
            synchronization: true,
            other_finished_while_runnable: false,
        });
    })
}

#[test]
fn overlapping_callers_enter_wait_then_receive_only_their_own_terminal_values() {
    let _serial = serial();
    let (server_a, client_a) = connected_pair();
    let (server_b, client_b) = connected_pair();
    let (entered_a_tx, entered_a_rx) = mpsc::sync_channel(1);
    let (entered_b_tx, entered_b_rx) = mpsc::sync_channel(1);
    let (report_a_tx, report_a_rx) = mpsc::sync_channel(1);
    let (report_b_tx, report_b_rx) = mpsc::sync_channel(1);
    let worker_a = waiting_caller(attached(server_a), 751, entered_a_tx, report_a_tx);
    let worker_b = waiting_caller(attached(server_b), 752, entered_b_tx, report_b_tx);
    let entry_deadline = Instant::now() + TEST_WAIT;
    let entered_a = entered_a_rx
        .recv_timeout(entry_deadline.saturating_duration_since(Instant::now()))
        .is_ok();
    let entered_b = entered_b_rx
        .recv_timeout(entry_deadline.saturating_duration_since(Instant::now()))
        .is_ok();
    let mut peer_a = peer(client_a);
    let mut peer_b = peer(client_b);
    // Both next() calls have entered the real driver's wait method before
    // either payload is produced. This is not proof of kernel-block entry.
    // Release both callers even if the entry watchdog observed a failure.
    let write_b = transfer(&mut peer_b, Buffer::Write(b"second"));
    let write_a = transfer(&mut peer_a, Buffer::Write(b"first"));
    let terminal_deadline = Instant::now() + TEST_WAIT;
    let report_a =
        report_a_rx.recv_timeout(terminal_deadline.saturating_duration_since(Instant::now()));
    let report_b =
        report_b_rx.recv_timeout(terminal_deadline.saturating_duration_since(Instant::now()));
    let joined_a = if report_a.is_ok() || worker_a.is_finished() {
        Some(worker_a.join())
    } else {
        drop(worker_a);
        None
    };
    let joined_b = if report_b.is_ok() || worker_b.is_finished() {
        Some(worker_b.join())
    } else {
        drop(worker_b);
        None
    };
    assert!(
        entered_a && entered_b,
        "both callers must enter their native wait method before either payload"
    );
    assert_eq!(write_a, Awaited::complete(Ok(5)));
    assert_eq!(write_b, Awaited::complete(Ok(6)));
    let report_a = report_a.expect("first caller lost its routed completion or wake");
    let report_b = report_b.expect("second caller lost its routed completion or wake");
    assert_eq!(
        (report_a.key, report_a.outcome),
        (Some(751), Some(Awaited::complete(Ok(5))))
    );
    assert_eq!(
        (report_b.key, report_b.outcome),
        (Some(752), Some(Awaited::complete(Ok(6))))
    );
    assert_eq!(&report_a.bytes[..5], b"first");
    assert_eq!(&report_b.bytes[..6], b"second");
    assert!(joined_a
        .expect("first caller still owns pending resources")
        .is_ok());
    assert!(joined_b
        .expect("second caller still owns pending resources")
        .is_ok());
}

#[test]
fn former_native_dequeuer_can_remain_runnable_while_another_caller_dequeues() {
    let _serial = serial();
    let (server_a, client_a) = connected_pair();
    let (server_b, client_b) = connected_pair();
    let owner_a = attached(server_a);
    let owner_b = attached(server_b);
    let submitted_a = TestEvent::new();
    let submitted_b = TestEvent::new();
    let runnable_a = TestEvent::new();
    let start_b = TestEvent::new();
    let b_done = Arc::new(AtomicBool::new(false));
    let (report_a_tx, report_a_rx) = mpsc::sync_channel(1);
    let (report_b_tx, report_b_rx) = mpsc::sync_channel(1);
    let worker_a = {
        let submitted = Arc::clone(&submitted_a);
        let runnable = Arc::clone(&runnable_a);
        let b_done = Arc::clone(&b_done);
        std::thread::spawn(move || {
            let mut set = configured(&[701], [owner_a]);
            let mut bytes = [0_u8; 8];
            let deadline = Instant::now() + TEST_WAIT;
            let wait = budget(2, TEST_NANOSECONDS);
            let mut group = set
                .begin(
                    BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap(),
                    Some(&wait),
                    None,
                )
                .unwrap();
            let submitted = submitted.signal();
            let before = native_dequeues_for_test();
            let completion = group.next();
            let dequeues = native_dequeues_for_test() - before;
            let runnable = runnable.signal();
            // After a real dequeue this thread deliberately performs only
            // runnable caller work: no lock, channel, wait, sleep, or yield.
            // Keep the group alive so teardown cannot dissociate it first.
            let mut work = 0_u64;
            while !b_done.load(Ordering::Acquire) && Instant::now() < deadline {
                work = work.wrapping_add(1).rotate_left(3);
                std::hint::black_box(work);
            }
            let other_finished_while_runnable = b_done.load(Ordering::Acquire);
            let (key, outcome) = completion.map_or((None, None), |completion| {
                (Some(completion.key), Some(completion.outcome))
            });
            drop(group);
            set.assert_retired_for_test();
            drop(set);
            let _ = report_a_tx.send(CallerReport {
                outcome,
                key,
                bytes,
                dequeues,
                synchronization: submitted && runnable,
                other_finished_while_runnable,
            });
        })
    };
    let worker_b = {
        let submitted = Arc::clone(&submitted_b);
        let start = Arc::clone(&start_b);
        let b_done = Arc::clone(&b_done);
        std::thread::spawn(move || {
            let done = DoneOnDrop(b_done);
            let mut set = configured(&[702], [owner_b]);
            let mut bytes = [0_u8; 8];
            let wait = budget(2, TEST_NANOSECONDS);
            let mut group = set
                .begin(
                    BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap(),
                    Some(&wait),
                    None,
                )
                .unwrap();
            let submitted = submitted.signal();
            let start = start.wait();
            let before = native_dequeues_for_test();
            let completion = group.next();
            let dequeues = native_dequeues_for_test() - before;
            let (key, outcome) = completion.map_or((None, None), |completion| {
                (Some(completion.key), Some(completion.outcome))
            });
            drop(group);
            set.assert_retired_for_test();
            drop(set);
            // Record completion before the report/channel operation; A's
            // proof is its runnable observation, not a successful join later.
            drop(done);
            let _ = report_b_tx.send(CallerReport {
                outcome,
                key,
                bytes,
                dequeues,
                synchronization: submitted && start,
                other_finished_while_runnable: false,
            });
        })
    };
    let submitted = submitted_a.wait() && submitted_b.wait();
    let mut peer_a = peer(client_a);
    let mut peer_b = peer(client_b);
    let write_a = transfer(&mut peer_a, Buffer::Write(b"a"));
    let runnable = runnable_a.wait();
    let write_b = transfer(&mut peer_b, Buffer::Write(b"b"));
    // Release B even if an earlier observation failed, before any assertion.
    let released_b = start_b.signal();
    let report_b = report_b_rx.recv_timeout(TEST_WAIT + TEST_WAIT);
    let report_a = report_a_rx.recv_timeout(TEST_WAIT);
    let joined_a = if report_a.is_ok() || worker_a.is_finished() {
        Some(worker_a.join())
    } else {
        drop(worker_a);
        None
    };
    let joined_b = if report_b.is_ok() || worker_b.is_finished() {
        Some(worker_b.join())
    } else {
        drop(worker_b);
        None
    };
    assert!(submitted && runnable && released_b);
    assert_eq!(write_a, Awaited::complete(Ok(1)));
    assert_eq!(write_b, Awaited::complete(Ok(1)));
    let report_a = report_a.expect("runnable caller did not finish within its original bound");
    let report_b = report_b.expect("second native dequeuer did not finish within its bound");
    assert!(report_a.synchronization && report_b.synchronization);
    assert!(
        report_a.dequeues > 0 && report_b.dequeues > 0,
        "each caller must dequeue its own genuine native packet"
    );
    assert!(
        report_a.other_finished_while_runnable,
        "IOCP admission blocked B while former dequeuer A remained runnable"
    );
    assert_eq!(
        (report_a.key, report_a.outcome),
        (Some(701), Some(Awaited::complete(Ok(1))))
    );
    assert_eq!(
        (report_b.key, report_b.outcome),
        (Some(702), Some(Awaited::complete(Ok(1))))
    );
    assert_eq!(report_a.bytes[0], b'a');
    assert_eq!(report_b.bytes[0], b'b');
    assert!(joined_a
        .expect("caller A still owns pending resources")
        .is_ok());
    assert!(joined_b
        .expect("caller B still owns pending resources")
        .is_ok());
}

#[test]
fn broker_is_lazy_shared_by_stream_owners_and_released_after_last_owner() {
    let _serial = serial();
    assert_eq!(lifecycle_snapshot(), (0, 0));
    let (server_a, _client_a) = connected_pair();
    let (server_b, _client_b) = connected_pair();
    // Creating unrelated native resources does not initialize the broker.
    assert_eq!(lifecycle_snapshot(), (0, 0));
    let ((owner_a, first_allocations), costs) =
        crate::io_cost_probe::measure(|| crate::probe_runtime_allocations(|| attached(server_a)));
    assert_eq!(
        costs[crate::io_cost_probe::Operation::CompletionBrokerAcquire as usize],
        1
    );
    let identity_a = owner_a.identity_for_test();
    assert_eq!(
        first_allocations, 2,
        "broker and immutable association storage are configuration allocations"
    );
    assert_eq!(lifecycle_snapshot(), (identity_a.2, 1));
    let ((owner_b, second_allocations), costs) =
        crate::io_cost_probe::measure(|| crate::probe_runtime_allocations(|| attached(server_b)));
    assert_eq!(
        costs[crate::io_cost_probe::Operation::CompletionBrokerAcquire as usize],
        1
    );
    let identity_b = owner_b.identity_for_test();
    assert_eq!(
        second_allocations, 1,
        "second owner reuses the existing broker"
    );
    assert_eq!(identity_a.2, identity_b.2);
    assert_ne!(identity_a.1, identity_b.1);
    assert_eq!(lifecycle_snapshot(), (identity_a.2, 2));
    let set = configured(&[801, 802], [owner_a, owner_b]);
    assert_eq!(lifecycle_snapshot(), (identity_a.2, 3));
    let owners = set.into_attached();
    assert_eq!(lifecycle_snapshot(), (identity_a.2, 2));
    assert_eq!(owners.as_slice()[0].identity_for_test(), identity_a);
    assert_eq!(owners.as_slice()[1].identity_for_test(), identity_b);
    // Moving and then regrouping owners changes neither immutable key nor
    // claim identity. Set configuration owns only its extra broker claim.
    let moved = owners;
    let set = CompletionSet::from_attached(&[901, 902], moved).unwrap();
    assert_eq!(lifecycle_snapshot(), (identity_a.2, 3));
    let owners = set.into_attached();
    assert_eq!(lifecycle_snapshot(), (identity_a.2, 2));
    assert_eq!(owners.as_slice()[0].identity_for_test(), identity_a);
    assert_eq!(owners.as_slice()[1].identity_for_test(), identity_b);
    let mut owners = owners.into_iter();
    let first = owners.next().unwrap();
    let last = owners.next().unwrap();
    assert!(owners.next().is_none());
    drop(owners);
    // Consuming close attempts the first device exactly once; its automatic
    // cleanup must neither close it twice nor release the surviving claim.
    let (closed, first_closes) = crate::native_handle::with_close_probe(|| first.close());
    assert_eq!(closed, Ok(()));
    assert_eq!(first_closes, 1);
    assert_eq!(lifecycle_snapshot(), (identity_a.2, 1));
    assert_eq!(last.identity_for_test(), identity_b);
    let ((), last_closes) = crate::native_handle::with_close_probe(|| drop(last));
    // The shared port is a private kernel object, not a stream: the release
    // of the last claim is proven by the lifecycle snapshot below, not counted
    // as a stream close.
    assert_eq!(last_closes, 1, "the last stream closes once");
    assert_eq!(
        first_closes + last_closes,
        2,
        "two valid stream owners each close once"
    );
    assert_eq!(lifecycle_snapshot(), (0, 0));
}

use crate::win32::SetEvent;

#[test]
fn reversed_noncontiguous_subset_preserves_native_slots_and_unused_stream_data() {
    let _serial = serial();
    let (server_a, client_a) = connected_pair();
    let (server_b, client_b) = connected_pair();
    let (server_c, client_c) = connected_pair();
    let mut set = configured(
        &[9101, 9102, 9103],
        [attached(server_a), attached(server_b), attached(server_c)],
    );
    let identities =
        core::array::from_fn::<_, 3, _>(|index| set.owners.as_slice()[index].identity_for_test());
    let snapshots = core::array::from_fn::<_, 3, _>(|index| set.snapshot_for_test(index));
    assert_eq!(set.index_of(9103), Some(2));
    assert_eq!(set.index_of(9101), Some(0));
    assert_eq!(set.index_of(9999), None);
    let mut peer_a = peer(client_a);
    let mut peer_b = peer(client_b);
    let mut peer_c = peer(client_c);
    let (mut selected_c, mut selected_a) = ([0_u8; 16], [0_u8; 16]);
    let wait = budget(2, TEST_NANOSECONDS);
    let selection = selected_indices([2, 0]);
    let before = native_dequeues_for_test();
    let mut group = set
        .begin_selected(
            BufferList::from_iter(
                [Buffer::Read(&mut selected_c), Buffer::Read(&mut selected_a)].into_iter(),
            )
            .unwrap(),
            Some(&wait),
            None,
            selection,
        )
        .unwrap();
    // Slot one has bytes available, but no buffer or request in this plan.
    assert_eq!(
        transfer(&mut peer_b, Buffer::Write(b"untouched")),
        Awaited::complete(Ok(9))
    );
    assert_eq!(
        transfer(&mut peer_c, Buffer::Write(b"cee")),
        Awaited::complete(Ok(3))
    );
    let (completion, allocations) = crate::probe_runtime_allocations(|| group.next().unwrap());
    assert_eq!(allocations, 0);
    assert_eq!(
        (completion.index, completion.key, completion.outcome),
        (0, 9103, Awaited::complete(Ok(3)))
    );
    assert_eq!(group.completed(0), Some(b"cee".as_slice()));
    assert!(group.completed(1).is_none());
    assert!(group.read_storage_mut(1).is_none());
    assert_eq!(group.request_info(2), None);
    let (submitted, allocations) = crate::probe_runtime_allocations(|| {
        group.read_storage_mut(0).unwrap().fill(0x77);
        group.submit(0, 2, 5)
    });
    assert_eq!((submitted, allocations), (Ok(()), 0));
    // Plan index zero must reuse registered slot two, never slot zero.
    assert_eq!(
        transfer(&mut peer_c, Buffer::Write(b"again")),
        Awaited::complete(Ok(5))
    );
    let (completion, allocations) = crate::probe_runtime_allocations(|| group.next().unwrap());
    assert_eq!(allocations, 0);
    assert_eq!(
        (completion.index, completion.key, completion.outcome),
        (0, 9103, Awaited::complete(Ok(5)))
    );
    assert_eq!(group.completed(0), Some(b"again".as_slice()));
    assert_eq!(
        transfer(&mut peer_a, Buffer::Write(b"aye")),
        Awaited::complete(Ok(3))
    );
    let completion = group.next().unwrap();
    assert_eq!(
        (completion.index, completion.key, completion.outcome),
        (1, 9101, Awaited::complete(Ok(3)))
    );
    assert_eq!(group.completed(1), Some(b"aye".as_slice()));
    assert!(group.next().is_none());
    assert_eq!(group.stop(), None);
    drop(group);
    assert_eq!(native_dequeues_for_test() - before, 3);
    set.assert_retired_for_test();
    assert_eq!(&selected_c[2..7], b"again");
    assert_eq!(&selected_a[..3], b"aye");
    assert_eq!(
        snapshots,
        core::array::from_fn::<_, 3, _>(|index| set.snapshot_for_test(index))
    );
    assert_eq!(
        identities,
        core::array::from_fn::<_, 3, _>(|index| set.owners.as_slice()[index].identity_for_test())
    );

    let (mut full_a, mut full_b, mut full_c) = ([0_u8; 16], [0_u8; 16], [0_u8; 16]);
    let wait = budget(2, TEST_NANOSECONDS);
    let mut group = set
        .begin(
            BufferList::from_iter(
                [
                    Buffer::Read(&mut full_a),
                    Buffer::Read(&mut full_b),
                    Buffer::Read(&mut full_c),
                ]
                .into_iter(),
            )
            .unwrap(),
            Some(&wait),
            None,
        )
        .unwrap();
    // Only B has data yet. Its entire earlier payload must still be present.
    let untouched = group.next().unwrap();
    assert_eq!(
        (untouched.index, untouched.key, untouched.outcome),
        (1, 9102, Awaited::complete(Ok(9)))
    );
    assert_eq!(group.completed(1), Some(b"untouched".as_slice()));
    assert_eq!(
        transfer(&mut peer_a, Buffer::Write(b"fresh-a")),
        Awaited::complete(Ok(7))
    );
    assert_eq!(
        transfer(&mut peer_c, Buffer::Write(b"fresh-c")),
        Awaited::complete(Ok(7))
    );
    let mut seen = [false, true, false];
    for _ in 0..2 {
        let completion = group.next().unwrap();
        assert!(!seen[completion.index]);
        seen[completion.index] = true;
        assert_eq!(completion.key, [9101, 9102, 9103][completion.index]);
        assert_eq!(completion.outcome, Awaited::complete(Ok(7)));
    }
    assert_eq!(seen, [true; 3]);
    assert_eq!(group.completed(0), Some(b"fresh-a".as_slice()));
    assert_eq!(group.completed(2), Some(b"fresh-c".as_slice()));
    assert!(group.next().is_none());
    drop(group);
    set.assert_retired_for_test();
    assert_eq!(
        snapshots,
        core::array::from_fn::<_, 3, _>(|index| set.snapshot_for_test(index))
    );
    assert_eq!(
        identities,
        core::array::from_fn::<_, 3, _>(|index| set.owners.as_slice()[index].identity_for_test())
    );
}

#[test]
fn subset_cancellation_and_drop_retire_only_participants_then_full_set_is_reusable() {
    let _serial = serial();
    let (server_a, client_a) = connected_pair();
    let (server_b, client_b) = connected_pair();
    let (server_c, client_c) = connected_pair();
    let mut set = configured(
        &[9201, 9202, 9203],
        [attached(server_a), attached(server_b), attached(server_c)],
    );
    let snapshots = core::array::from_fn::<_, 3, _>(|index| set.snapshot_for_test(index));
    let identities =
        core::array::from_fn::<_, 3, _>(|index| set.owners.as_slice()[index].identity_for_test());
    let mut peer_a = peer(client_a);
    let mut peer_b = peer(client_b);
    let mut peer_c = peer(client_c);
    for request_cancellation in [true, false] {
        let source = TestClaim::new();
        let (mut selected_c, mut selected_a) = ([0x55_u8; 8], [0x66_u8; 8]);
        let wait = budget(2, TEST_NANOSECONDS);
        let before = native_dequeues_for_test();
        let mut group = set
            .begin_selected(
                BufferList::from_iter(
                    [Buffer::Read(&mut selected_c), Buffer::Read(&mut selected_a)].into_iter(),
                )
                .unwrap(),
                Some(&wait),
                Some(unsafe { &*source.state() }),
                selected_indices([2, 0]),
            )
            .unwrap();
        assert_eq!(
            transfer(&mut peer_b, Buffer::Write(b"kept")),
            Awaited::complete(Ok(4))
        );
        if request_cancellation {
            source.request();
            let mut seen = [false; 2];
            for _ in 0..2 {
                let completion = group.next().unwrap();
                assert!(!seen[completion.index]);
                seen[completion.index] = true;
                assert_eq!(completion.key, [9203, 9201][completion.index]);
                assert_eq!(completion.outcome.stop, Some(Stop::Cancellation));
                assert_eq!(completion.outcome.cancellation_failure, None);
                assert_eq!(
                    completion.outcome.transfer,
                    Some(Err(
                        Failure::from_system_code(995).with_transferred_byte_count(Some(0))
                    ))
                );
            }
            assert_eq!(seen, [true; 2]);
            assert!(group.next().is_none());
            assert_eq!(group.stop(), Some(Stop::Cancellation));
        }
        // In the second pass both selected requests remain pending until Drop.
        // Cancellation's pointer-free poke is excluded from this native count.
        let ((), closes) = crate::native_handle::with_close_probe(|| drop(group));
        assert_eq!(closes, 0);
        assert_eq!(native_dequeues_for_test() - before, 2);
        set.assert_retired_for_test();
        assert_eq!(selected_c, [0x55; 8]);
        assert_eq!(selected_a, [0x66; 8]);

        let (mut full_a, mut full_b, mut full_c) = ([0_u8; 8], [0_u8; 8], [0_u8; 8]);
        let wait = budget(2, TEST_NANOSECONDS);
        let mut group = set
            .begin(
                BufferList::from_iter(
                    [
                        Buffer::Read(&mut full_a),
                        Buffer::Read(&mut full_b),
                        Buffer::Read(&mut full_c),
                    ]
                    .into_iter(),
                )
                .unwrap(),
                Some(&wait),
                None,
            )
            .unwrap();
        let untouched = group.next().unwrap();
        assert_eq!(
            (untouched.index, untouched.key, untouched.outcome),
            (1, 9202, Awaited::complete(Ok(4)))
        );
        assert_eq!(group.completed(1), Some(b"kept".as_slice()));
        assert_eq!(
            transfer(&mut peer_a, Buffer::Write(b"a")),
            Awaited::complete(Ok(1))
        );
        assert_eq!(
            transfer(&mut peer_c, Buffer::Write(b"c")),
            Awaited::complete(Ok(1))
        );
        let mut seen = [false, true, false];
        for _ in 0..2 {
            let completion = group.next().unwrap();
            assert!(!seen[completion.index]);
            seen[completion.index] = true;
            assert_eq!(completion.key, [9201, 9202, 9203][completion.index]);
            assert_eq!(completion.outcome, Awaited::complete(Ok(1)));
        }
        assert_eq!(seen, [true; 3]);
        assert_eq!(group.stop(), None);
        assert!(group.next().is_none());
        drop(group);
        set.assert_retired_for_test();
        assert_eq!(full_a[0], b'a');
        assert_eq!(full_c[0], b'c');
        assert_eq!(
            snapshots,
            core::array::from_fn::<_, 3, _>(|index| set.snapshot_for_test(index))
        );
        assert_eq!(
            identities,
            core::array::from_fn::<_, 3, _>(
                |index| set.owners.as_slice()[index].identity_for_test()
            )
        );
    }
}

#[test]
fn borrowed_membership_growth_and_holes_preserve_unaffected_native_registrations() {
    let _serial = serial();
    let mut owners = Vec::new();
    let mut peers = Vec::new();
    for _ in 0..3 {
        let (server, client) = connected_pair();
        owners.push(attached(server));
        peers.push(peer(client));
    }
    let identities = core::array::from_fn::<_, 3, _>(|index| owners[index].identity_for_test());
    let mut set = CompletionSet::borrowed_new().unwrap();
    for (index, key) in [101, 202, 303].into_iter().enumerate() {
        assert_eq!(
            unsafe { set.add_borrowed(key, (&owners[index] as *const AttachedStream).cast()) },
            Ok(index)
        );
    }
    let snapshot_a = set.borrowed_snapshot_for_test(0);
    let snapshot_c = set.borrowed_snapshot_for_test(2);
    // Every insertion grows the sparse pointer tables. Growing this Vec also
    // moves inline stream owners: only their pinned associations were captured.
    for offset in 0..4 {
        let (server, client) = connected_pair();
        owners.push(attached(server));
        peers.push(peer(client));
        let owner = owners.last().unwrap();
        unsafe { set.add_borrowed(1000 + offset, (owner as *const AttachedStream).cast()) }
            .unwrap();
        assert_eq!(set.borrowed_snapshot_for_test(0), snapshot_a);
        assert_eq!(set.borrowed_snapshot_for_test(2), snapshot_c);
        assert_eq!(owners[0].identity_for_test(), identities[0]);
        assert_eq!(owners[2].identity_for_test(), identities[2]);
    }
    assert_eq!(set.slots.occupied_len(), 7);
    let ((removed, stale), closes) = crate::native_handle::with_close_probe(|| {
        (set.remove_borrowed(202), unsafe {
            set.add_borrowed(202, (&owners[1] as *const AttachedStream).cast())
        })
    });
    assert_eq!(removed, Ok(()));
    assert_eq!(stale, Err(Failure::invalid_input()));
    assert_eq!(closes, 0);
    assert_eq!(set.key(1), None);
    assert_eq!(set.index_of(202), None);
    assert_eq!(set.borrowed_snapshot_for_test(0), snapshot_a);
    assert_eq!(set.borrowed_snapshot_for_test(2), snapshot_c);
    assert_eq!(owners[1].identity_for_test(), identities[1]);
    assert_eq!(
        transfer(&mut peers[1], Buffer::Write(b"untouched")),
        Awaited::complete(Ok(9))
    );

    let (mut bytes_c, mut bytes_a) = ([0x33_u8; 12], [0x11_u8; 12]);
    let address_c = bytes_c.as_mut_ptr();
    let wait = budget(2, TEST_NANOSECONDS);
    let before = native_dequeues_for_test();
    let mut group = set
        .begin_selected(
            BufferList::from_iter(
                [Buffer::Read(&mut bytes_c), Buffer::Read(&mut bytes_a)].into_iter(),
            )
            .unwrap(),
            Some(&wait),
            None,
            selected_indices([2, 0]),
        )
        .unwrap();
    assert_eq!(
        transfer(&mut peers[2], Buffer::Write(b"cee")),
        Awaited::complete(Ok(3))
    );
    let (completion, allocations) = crate::probe_runtime_allocations(|| group.next().unwrap());
    assert_eq!(allocations, 0);
    assert_eq!(
        (completion.index, completion.key, completion.outcome),
        (0, 303, Awaited::complete(Ok(3)))
    );
    assert_eq!(group.read_storage_mut(0).unwrap().as_mut_ptr(), address_c);
    let (submitted, allocations) = crate::probe_runtime_allocations(|| group.submit(0, 2, 3));
    assert_eq!((submitted, allocations), (Ok(()), 0));
    assert_eq!(
        transfer(&mut peers[2], Buffer::Write(b"new")),
        Awaited::complete(Ok(3))
    );
    let completion = group.next().unwrap();
    assert_eq!(
        (completion.index, completion.key, completion.outcome),
        (0, 303, Awaited::complete(Ok(3)))
    );
    assert_eq!(group.completed(0), Some(b"new".as_slice()));
    assert_eq!(
        transfer(&mut peers[0], Buffer::Write(b"aye")),
        Awaited::complete(Ok(3))
    );
    let completion = group.next().unwrap();
    assert_eq!(
        (completion.index, completion.key, completion.outcome),
        (1, 101, Awaited::complete(Ok(3)))
    );
    assert!(group.next().is_none());
    drop(group);
    assert_eq!(native_dequeues_for_test() - before, 3);
    set.assert_retired_for_test();
    assert_eq!(&bytes_c[2..5], b"new");
    assert_eq!(&bytes_a[..3], b"aye");
    assert_eq!(set.borrowed_snapshot_for_test(0), snapshot_a);
    assert_eq!(set.borrowed_snapshot_for_test(2), snapshot_c);

    let index_b =
        unsafe { set.add_borrowed(2000, (&owners[1] as *const AttachedStream).cast()) }.unwrap();
    assert_eq!(set.index_of(2000), Some(index_b));
    assert_eq!(set.index_of(202), None);
    assert_eq!(set.borrowed_snapshot_for_test(0), snapshot_a);
    assert_eq!(set.borrowed_snapshot_for_test(2), snapshot_c);
    let mut bytes_b = [0_u8; 12];
    let wait = budget(2, TEST_NANOSECONDS);
    let mut group = set
        .begin_selected(
            BufferList::from_iter([Buffer::Read(&mut bytes_b)].into_iter()).unwrap(),
            Some(&wait),
            None,
            selected_indices([index_b]),
        )
        .unwrap();
    let completion = group.next().unwrap();
    assert_eq!(
        (completion.index, completion.key, completion.outcome),
        (0, 2000, Awaited::complete(Ok(9)))
    );
    assert_eq!(group.completed(0), Some(b"untouched".as_slice()));
    drop(group);
    set.assert_retired_for_test();
    let ((), closes) = crate::native_handle::with_close_probe(|| drop(set));
    assert_eq!(closes, 0);
    for (owner, identity) in owners.iter().zip(identities) {
        assert_eq!(owner.identity_for_test(), identity);
    }
}

#[test]
fn borrowed_native_set_rejection_remove_and_drop_never_consume_stream_owner() {
    let _serial = serial();
    let (server, client) = connected_pair();
    let owner = attached(server);
    let identity = owner.identity_for_test();
    let projection = (&owner as *const AttachedStream).cast();
    let mut peer = peer(client);
    let mut set = unsafe { transport::NativeTransferSet::borrowed_new(ptr::null_mut()) }.unwrap();
    assert!(set.is_borrowed());
    assert_eq!(set.len(), 0);
    let (((zero, sentinel, missing), allocations), closes) =
        crate::native_handle::with_close_probe(|| {
            crate::probe_runtime_allocations(|| unsafe {
                (
                    set.add_borrowed(0, projection),
                    set.add_borrowed(u64::MAX, projection),
                    set.add_borrowed(1, ptr::null()),
                )
            })
        });
    assert_eq!(
        (zero, sentinel, missing),
        (
            Err(Failure::invalid_input()),
            Err(Failure::invalid_input()),
            Err(Failure::invalid_input())
        )
    );
    assert_eq!((allocations, closes), (0, 0));
    let index = unsafe { set.add_borrowed(10, projection) }.unwrap();
    assert_eq!(set.len(), 1);
    assert_eq!(set.key(index), Some(10));
    let ((duplicate, allocations), closes) = crate::native_handle::with_close_probe(|| {
        crate::probe_runtime_allocations(|| unsafe { set.add_borrowed(11, projection) })
    });
    assert_eq!(duplicate, Err(Failure::invalid_input()));
    assert_eq!((allocations, closes), (0, 0));
    let ((removed, allocations), closes) = crate::native_handle::with_close_probe(|| {
        crate::probe_runtime_allocations(|| set.remove_borrowed(10))
    });
    assert_eq!(removed, Ok(()));
    assert_eq!((allocations, closes), (0, 0));
    assert_eq!(set.len(), 0);
    assert_eq!(set.key(index), None);
    assert_eq!(set.index_of(10), None);
    let ((), closes) = crate::native_handle::with_close_probe(|| drop(set));
    assert_eq!(closes, 0);
    assert_eq!(owner.identity_for_test(), identity);

    // The same complete owner registers in a new borrowed set without another
    // native attachment, and the original device remains usable after removal.
    let mut set = unsafe { transport::NativeTransferSet::borrowed_new(ptr::null_mut()) }.unwrap();
    let index = unsafe { set.add_borrowed(1, projection) }.unwrap();
    let mut bytes = [0_u8; 8];
    let wait = budget(2, TEST_NANOSECONDS);
    let mut group = set
        .begin_selected(
            BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap(),
            &wait,
            selected_indices([index]),
        )
        .unwrap();
    assert_eq!(
        transfer(&mut peer, Buffer::Write(b"live")),
        Awaited::complete(Ok(4))
    );
    let completion = group.next().unwrap();
    assert_eq!(
        (completion.key, completion.outcome),
        (1, Awaited::complete(Ok(4)))
    );
    drop(group);
    let ((), closes) = crate::native_handle::with_close_probe(|| drop(set));
    assert_eq!(closes, 0);
    assert_eq!(owner.identity_for_test(), identity);
    assert_eq!(&bytes[..4], b"live");
}

#[test]
fn borrowed_sparse_subgroups_use_shared_cancellation_and_drop_retirement() {
    let _serial = serial();
    let (server_a, client_a) = connected_pair();
    let (server_b, client_b) = connected_pair();
    let (server_c, client_c) = connected_pair();
    let owners = [attached(server_a), attached(server_b), attached(server_c)];
    let identities = core::array::from_fn::<_, 3, _>(|index| owners[index].identity_for_test());
    let mut set = CompletionSet::borrowed_new().unwrap();
    for (index, key) in [11, 12, 13].into_iter().enumerate() {
        assert_eq!(
            unsafe { set.add_borrowed(key, (&owners[index] as *const AttachedStream).cast()) },
            Ok(index)
        );
    }
    set.remove_borrowed(12).unwrap();
    let snapshots = [
        set.borrowed_snapshot_for_test(0),
        set.borrowed_snapshot_for_test(2),
    ];
    let _peers = [client_a, client_b, client_c];
    for requested in [true, false] {
        let source = TestClaim::new();
        let (mut bytes_c, mut bytes_a) = ([0x33_u8; 8], [0x11_u8; 8]);
        let wait = budget(2, TEST_NANOSECONDS);
        let before = native_dequeues_for_test();
        let mut group = set
            .begin_selected(
                BufferList::from_iter(
                    [Buffer::Read(&mut bytes_c), Buffer::Read(&mut bytes_a)].into_iter(),
                )
                .unwrap(),
                Some(&wait),
                Some(unsafe { &*source.state() }),
                selected_indices([2, 0]),
            )
            .unwrap();
        if requested {
            source.request();
            let mut seen = [false; 2];
            for _ in 0..2 {
                let completion = group.next().unwrap();
                assert!(!seen[completion.index]);
                seen[completion.index] = true;
                assert_eq!(completion.key, [13, 11][completion.index]);
                assert_eq!(completion.outcome.stop, Some(Stop::Cancellation));
                assert_eq!(completion.outcome.cancellation_failure, None);
                assert_eq!(
                    completion.outcome.transfer,
                    Some(Err(
                        Failure::from_system_code(995).with_transferred_byte_count(Some(0))
                    ))
                );
            }
            assert_eq!(seen, [true; 2]);
            assert!(group.next().is_none());
        }
        let ((), closes) = crate::native_handle::with_close_probe(|| drop(group));
        assert_eq!(closes, 0);
        assert_eq!(native_dequeues_for_test() - before, 2);
        set.assert_retired_for_test();
        assert_eq!(bytes_c, [0x33; 8]);
        assert_eq!(bytes_a, [0x11; 8]);
        assert_eq!(
            [
                set.borrowed_snapshot_for_test(0),
                set.borrowed_snapshot_for_test(2)
            ],
            snapshots
        );
        for (owner, identity) in owners.iter().zip(identities) {
            assert_eq!(owner.identity_for_test(), identity);
        }
    }
    let ((), closes) = crate::native_handle::with_close_probe(|| drop(set));
    assert_eq!(closes, 0);
}
