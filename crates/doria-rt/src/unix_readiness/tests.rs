use super::*;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;

#[test]
fn group_cancellation_watch_retires_without_affecting_the_next_group() {
    unsafe {
        let mut set = Set::new(1, ptr::null_mut()).unwrap();
        let first = cancellation::dr_v1_cancellation_new();
        let second = cancellation::dr_v1_cancellation_new();
        let mut first_watch = GroupCancellationWatch::install(&mut set, first).unwrap();
        assert_eq!(cancellation::owned_claim_count(first), 2);
        let mut reason = 0;
        let mut code = 0;
        let mut has_code = 0;
        assert_eq!(
            cancellation::dr_v1_cancellation_request(first, &mut reason, &mut code, &mut has_code),
            0
        );
        let immediate =
            DrWaitBudgetV1::new(0, 0, || panic!("immediate wait sampled the clock")).unwrap();
        let mut events = [DrReadyEventV1::default(); 2];
        assert_eq!(set.wait(&immediate, &mut events).unwrap(), (2, 0));
        first_watch.detach(&mut set);
        drop(first_watch);
        assert_eq!(cancellation::owned_claim_count(first), 1);
        // Keep the first source/signal alive and requested: closing its last
        // claim must not hide a missing native unregistration.
        let mut second_watch = GroupCancellationWatch::install(&mut set, second).unwrap();
        assert_eq!(set.wait(&immediate, &mut events).unwrap(), (1, 0));
        assert_eq!(
            cancellation::dr_v1_cancellation_request(second, &mut reason, &mut code, &mut has_code),
            0
        );
        assert_eq!(set.wait(&immediate, &mut events).unwrap(), (2, 0));
        second_watch.detach(&mut set);
        drop(second_watch);
        assert_eq!(set.wait(&immediate, &mut events).unwrap(), (1, 0));
        cancellation::dr_v1_cancellation_release(first);
        cancellation::dr_v1_cancellation_release(second);
    }
}

#[test]
fn failed_group_wake_detachment_closes_only_the_owned_poller() {
    unsafe {
        let mut set = Set::new(1, ptr::null_mut()).unwrap();
        let (stream, mut peer) = UnixStream::pair().unwrap();
        set.add(stream.as_raw_fd(), 9, READ).unwrap();
        let state = cancellation::dr_v1_cancellation_new();
        let mut watch = GroupCancellationWatch::install(&mut set, state).unwrap();
        let signal = watch.registered_signal.unwrap();
        let poller = set.handle().unwrap();
        watch.detach_with(&mut set, |actual_poller, actual_signal| {
            assert_eq!((actual_poller, actual_signal), (poller, signal));
            Err(Failure::other())
        });
        assert!(set.watcher.is_none());
        assert_eq!(cancellation::owned_claim_count(state), 2);
        drop(watch);
        assert_eq!(cancellation::owned_claim_count(state), 1);
        peer.write_all(b"x").unwrap();
        let mut stream = stream;
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        assert_eq!(byte, *b"x");
        cancellation::dr_v1_cancellation_release(state);
    }
}

#[test]
fn shared_registration_watch_tracks_direction_counts_without_losing_siblings() {
    let (stream, _) = UnixStream::pair().unwrap();
    let mut set = unsafe { Set::new(1, ptr::null_mut()).unwrap() };
    let mut watch = RegistrationWatch::new(stream.as_raw_fd(), 3).unwrap();
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        watch.subscribe(&mut set, true).unwrap();
        let generation = set.slots()[0].generation;
        watch.subscribe(&mut set, false).unwrap();
        watch.subscribe(&mut set, true).unwrap();
        assert_eq!(set.slots()[0].interests, READ | WRITE);
        assert_eq!(set.slots()[0].generation, generation);
        watch.unsubscribe(&mut set, true).unwrap();
        assert_eq!(set.slots()[0].interests, READ | WRITE);
        watch.unsubscribe(&mut set, false).unwrap();
        assert_eq!(set.slots()[0].interests, READ);
        assert_eq!(set.slots()[0].generation, generation);
        watch.unsubscribe(&mut set, true).unwrap();
        assert_eq!(set.slots()[0].interests, 0);
        assert_eq!(watch.interests(), 0);
    });
    assert_eq!(allocations, 0);
}

#[test]
fn interest_replacement_changes_only_deltas_and_rolls_back_partial_failure() {
    let mut calls = Vec::new();
    change_interests(READ, READ | WRITE, |interest, adding| {
        calls.push((interest, adding));
        Ok(())
    })
    .unwrap();
    assert_eq!(calls, [(WRITE, true)]);
    for recovery_fails in [false, true] {
        calls.clear();
        let failure = Failure::other();
        let error = change_interests(READ, WRITE, |interest, adding| {
            calls.push((interest, adding));
            if calls.len() == 2 || (calls.len() == 3 && recovery_fails) {
                Err(failure)
            } else {
                Ok(())
            }
        })
        .unwrap_err();
        assert_eq!(error.failure, failure);
        assert_eq!(error.watcher_valid, !recovery_fails);
        assert_eq!(calls, [(READ, false), (WRITE, true), (READ, true)]);
    }
}

#[test]
fn queued_fact_for_retired_direction_is_ignored_without_retiring_live_direction() {
    let (stream, _) = UnixStream::pair().unwrap();
    let mut set = unsafe { Set::new(1, ptr::null_mut()).unwrap() };
    set.add(stream.as_raw_fd(), 8, READ | WRITE).unwrap();
    let token = u64::from(set.slots()[0].generation) << 32;
    set.update(8, WRITE).unwrap();
    let mut output = [DrReadyEventV1::default(); 2];
    let result = set
        .wait_with(
            &budget(0, 0),
            &mut output,
            |_, events, _, _| {
                #[cfg(target_os = "linux")]
                let event = platform::Event {
                    events: libc::EPOLLIN as u32,
                    u64: token,
                };
                #[cfg(target_os = "macos")]
                let event = platform::Event {
                    ident: stream.as_raw_fd() as usize,
                    filter: libc::EVFILT_READ,
                    flags: 0,
                    fflags: 0,
                    data: 1,
                    udata: token as usize as *mut c_void,
                };
                events[0].write(event);
                Ok(1)
            },
            || Ok(0),
        )
        .unwrap();
    assert_eq!(result, (1, 0));
    assert_eq!(set.slots()[0].interests, WRITE);
    assert_eq!(u64::from(set.slots()[0].generation) << 32, token);
}

fn budget(kind: u8, nanoseconds: u64) -> DrWaitBudgetV1 {
    let mut budget = mem::MaybeUninit::uninit();
    let (mut reason, mut code, mut has_code) = (0, 0, 0);
    assert_eq!(
        unsafe {
            crate::wait_budget::dr_v1_wait_budget_init(
                kind,
                nanoseconds,
                budget.as_mut_ptr(),
                &mut reason,
                &mut code,
                &mut has_code,
            )
        },
        0
    );
    unsafe { budget.assume_init() }
}

#[test]
fn persistent_multi_watch_reuses_storage_and_reports_advisory_closure() {
    let mut set = unsafe { Set::new(3, ptr::null_mut()).unwrap() };
    let watcher = set.handle().unwrap();
    assert_ne!(
        unsafe { libc::fcntl(watcher, libc::F_GETFD) } & libc::FD_CLOEXEC,
        0
    );
    let memory = set.memory;
    let events = set.events;
    let slots = set.slots;
    let (mut first, mut first_peer) = UnixStream::pair().unwrap();
    let (mut second, mut second_peer) = UnixStream::pair().unwrap();
    let (third, _third_peer) = UnixStream::pair().unwrap();
    set.add(first.as_raw_fd(), 10, READ).unwrap();
    set.add(second.as_raw_fd(), 20, READ).unwrap();
    set.add(third.as_raw_fd(), 30, WRITE).unwrap();
    let mut output = [DrReadyEventV1::default(); 6];
    let poll = budget(0, 0);
    for _ in 0..4 {
        first_peer.write_all(b"a").unwrap();
        second_peer.write_all(b"bc").unwrap();
        let (result, allocations) =
            crate::probe_runtime_allocations(|| set.wait(&poll, &mut output));
        let (tag, count) = result.unwrap();
        assert_eq!((tag, allocations), (0, 0));
        for (key, facts) in [(10, READ), (20, READ), (30, WRITE)] {
            assert!(output[..count]
                .iter()
                .any(|event| event.key == key && event.facts & facts != 0));
        }
        assert_eq!(
            (set.handle().unwrap(), set.memory, set.events, set.slots),
            (watcher, memory, events, slots)
        );
        first.read_exact(&mut [0; 1]).unwrap();
        second.read_exact(&mut [0; 2]).unwrap();
    }
    // Closure notification can coexist with unread data; it does not consume it.
    first_peer.write_all(b"last").unwrap();
    drop(first_peer);
    let (_, count) = set.wait(&poll, &mut output).unwrap();
    assert!(output[..count]
        .iter()
        .any(|event| event.key == 10 && event.facts & CLOSED != 0));
    let mut last = [0; 4];
    first.read_exact(&mut last).unwrap();
    assert_eq!(&last, b"last");
    assert_eq!(first.read(&mut [0; 1]).unwrap(), 0);
}

#[test]
fn registration_changes_preserve_borrowed_handles_and_reject_duplicate_identity() {
    let mut set = unsafe { Set::new(1, ptr::null_mut()).unwrap() };
    let (mut stream, mut peer) = UnixStream::pair().unwrap();
    assert_eq!(set.add(-1, 1, READ).unwrap_err().reason, Reason::Closed);
    assert_eq!(
        set.add(stream.as_raw_fd(), 1, 0),
        Err(Failure::invalid_input())
    );
    set.add(stream.as_raw_fd(), u64::MAX, READ).unwrap();
    assert_eq!(
        set.add(stream.as_raw_fd(), 2, READ),
        Err(Failure::invalid_input())
    );
    let (other, _) = UnixStream::pair().unwrap();
    assert_eq!(
        set.add(other.as_raw_fd(), u64::MAX, READ),
        Err(Failure::invalid_input())
    );
    assert_eq!(
        set.add(other.as_raw_fd(), 2, READ).unwrap_err().reason,
        Reason::ResourceExhausted
    );
    let generation = set.slots()[0].generation;
    set.remove(u64::MAX).unwrap();
    set.add(stream.as_raw_fd(), 2, READ | WRITE).unwrap();
    assert_eq!(set.slots()[0].generation, generation + 1);
    set.remove(2).unwrap();
    assert_eq!(set.remove(2), Err(Failure::invalid_input()));
    drop(set);
    peer.write_all(b"live").unwrap();
    let mut bytes = [0; 4];
    stream.read_exact(&mut bytes).unwrap();
    assert_eq!(&bytes, b"live");
}

#[test]
fn regular_file_is_usable_even_at_eof_and_does_not_lose_the_cursor() {
    use std::fs::OpenOptions;
    let path = std::env::temp_dir().join(format!("doria-readiness-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    let mut set = unsafe { Set::new(1, ptr::null_mut()).unwrap() };
    set.add(file.as_raw_fd(), 7, READ | WRITE).unwrap();
    let mut output = [DrReadyEventV1::default(); 2];
    assert_eq!(set.wait(&budget(1, 0), &mut output).unwrap(), (0, 1));
    assert_eq!(
        output[0],
        DrReadyEventV1 {
            key: 7,
            facts: READ | WRITE
        }
    );
    assert_eq!(file.read(&mut [0; 1]).unwrap(), 0);
    file.write_all(b"x").unwrap();
    assert_eq!(set.wait(&budget(0, 0), &mut output).unwrap(), (0, 1));
    assert_eq!(file.read(&mut [0; 1]).unwrap(), 0);
    set.remove(7).unwrap();
    drop(set);
    drop(file);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn stale_generation_does_not_report_a_reused_registration() {
    let mut set = unsafe { Set::new(1, ptr::null_mut()).unwrap() };
    let (mut stream, mut peer) = UnixStream::pair().unwrap();
    set.add(stream.as_raw_fd(), 1, READ).unwrap();
    peer.write_all(b"x").unwrap();
    // Model a queued notification from the preceding slot generation.
    set.slots()[0].generation += 1;
    let mut output = [DrReadyEventV1::default(); 2];
    assert_eq!(set.wait(&budget(0, 0), &mut output).unwrap(), (1, 0));
    set.slots()[0].generation -= 1;
    assert_eq!(set.wait(&budget(0, 0), &mut output).unwrap().0, 0);
    let mut byte = [0];
    stream.read_exact(&mut byte).unwrap();
    assert_eq!(&byte, b"x");
}

#[test]
fn paired_registration_changes_restore_the_previous_state_on_failure() {
    for adding in [true, false] {
        for fail_at in [None, Some(0), Some(1)] {
            let mut calls = Vec::new();
            let failure = Failure::from_system_code(i64::from(libc::ENOMEM));
            let result = change_pair(READ | WRITE, adding, |interest, add| {
                let index = calls.len();
                calls.push((interest, add));
                if Some(index) == fail_at {
                    Err(failure)
                } else {
                    Ok(())
                }
            });
            match fail_at {
                None => {
                    result.unwrap();
                    assert_eq!(calls, [(READ, adding), (WRITE, adding)]);
                }
                Some(0) => {
                    let error = result.unwrap_err();
                    assert_eq!(error.failure, failure);
                    assert!(error.watcher_valid);
                    assert_eq!(calls, [(READ, adding)]);
                }
                Some(1) => {
                    let error = result.unwrap_err();
                    assert_eq!(error.failure, failure);
                    assert!(error.watcher_valid);
                    assert_eq!(calls, [(READ, adding), (WRITE, adding), (READ, !adding)]);
                }
                _ => unreachable!(),
            }
        }
    }
}

#[test]
fn failed_registration_recovery_invalidates_only_the_watcher() {
    for adding in [true, false] {
        let failure = Failure::from_system_code(i64::from(libc::ENOMEM));
        let mut calls = 0;
        let error = change_pair(READ | WRITE, adding, |_, _| {
            calls += 1;
            if calls == 1 {
                Ok(())
            } else {
                Err(failure)
            }
        })
        .unwrap_err();
        assert!(!error.watcher_valid);
        let mut set = unsafe { Set::new(1, ptr::null_mut()).unwrap() };
        let (mut stream, mut peer) = UnixStream::pair().unwrap();
        set.add(stream.as_raw_fd(), 1, READ | WRITE).unwrap();
        let (result, closes) =
            crate::native_handle::with_close_probe(|| set.registration_result(Err(error)));
        assert_eq!((result, closes), (Err(failure), 1));
        assert_eq!(set.handle().unwrap_err().reason, Reason::Closed);
        assert_eq!(set.remove(1).unwrap_err().reason, Reason::Closed);
        assert_eq!(
            set.add(stream.as_raw_fd(), 2, READ).unwrap_err().reason,
            Reason::Closed
        );
        assert_eq!(
            set.wait(&budget(0, 0), &mut [DrReadyEventV1::default(); 2])
                .unwrap_err()
                .reason,
            Reason::Closed
        );
        let ((), closes) = crate::native_handle::with_close_probe(|| drop(set));
        assert_eq!(closes, 0);
        peer.write_all(b"x").unwrap();
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        assert_eq!(&byte, b"x");
    }
}

#[test]
fn immediate_expired_and_repeated_finite_waits_preserve_one_budget() {
    let mut set = unsafe { Set::new(1, ptr::null_mut()).unwrap() };
    let (stream, _peer) = UnixStream::pair().unwrap();
    set.add(stream.as_raw_fd(), 1, READ).unwrap();
    let mut output = [DrReadyEventV1::default(); 2];
    assert_eq!(set.wait(&budget(0, 0), &mut output).unwrap(), (1, 0));
    assert_eq!(set.wait(&budget(2, 0), &mut output).unwrap(), (1, 0));
    for nanoseconds in [i64::MIN, -250_000_000, -1] {
        let expired = budget(2, nanoseconds as u64);
        assert_eq!(
            set.wait_with(
                &expired,
                &mut output,
                |_, _, _, _| panic!("expired wait polled the operating system"),
                || panic!("negative wait queried the clock"),
            )
            .unwrap(),
            (1, 0)
        );
    }
    let finite = budget(2, 20_000_000);
    let began = std::time::Instant::now();
    assert_eq!(set.wait(&finite, &mut output).unwrap(), (1, 0));
    for _ in 0..8 {
        assert_eq!(set.wait(&finite, &mut output).unwrap(), (1, 0));
    }
    assert!(began.elapsed() < std::time::Duration::from_secs(2));
    assert_eq!(
        set.wait(&budget(0, 0), &mut output[..1]),
        Err(Failure::invalid_input())
    );
}

#[test]
fn interruptions_and_empty_wakes_spend_the_same_deadline() {
    let mut set = unsafe { Set::new(1, ptr::null_mut()).unwrap() };
    let watcher = set.handle().unwrap();
    let mut output = [DrReadyEventV1::default(); 2];
    let deadline = budget(3, 1_000_000_000);
    let mut times = [0, 300_000_000, 600_000_000, 1_000_000_000].into_iter();
    let mut waits = Vec::new();
    let result = set.wait_with(
        &deadline,
        &mut output,
        |handle, _, kind, units| {
            assert_eq!(handle, watcher);
            assert_eq!(kind, 2);
            waits.push(units);
            if waits.len() == 1 {
                Err(Failure::from_system_code(i64::from(libc::EINTR)))
            } else {
                Ok(0)
            }
        },
        || Ok(times.next().expect("wait restarted its deadline")),
    );
    assert_eq!(result.unwrap(), (1, 0));
    assert_eq!(
        waits,
        [1_000_000_000, 700_000_000, 400_000_000].map(|n| n / platform::UNIT)
    );
    assert!(times.next().is_none());
}

#[test]
fn wait_failures_preserve_unknown_progress_and_leave_the_watcher_reusable() {
    let mut set = unsafe { Set::new(1, ptr::null_mut()).unwrap() };
    let mut output = [DrReadyEventV1::default(); 2];
    let failure = Failure::from_system_code(i64::from(libc::ENOMEM));
    let untimed = budget(1, 0);
    assert_eq!(
        set.wait_with(
            &untimed,
            &mut output,
            |_, _, kind, _| {
                assert_eq!(kind, 1);
                Err(failure)
            },
            || panic!("indefinite wait queried the clock"),
        ),
        Err(failure)
    );
    let deadline = budget(3, 1_000_000_000);
    assert_eq!(
        set.wait_with(
            &deadline,
            &mut output,
            |_, _, _, _| panic!("clock failure started a native wait"),
            || Err(failure),
        ),
        Err(failure)
    );
    assert_eq!(failure.transferred_byte_count, None);
    let (stream, mut peer) = UnixStream::pair().unwrap();
    set.add(stream.as_raw_fd(), 1, READ).unwrap();
    peer.write_all(b"x").unwrap();
    assert_eq!(set.wait(&budget(0, 0), &mut output).unwrap(), (0, 1));
}

#[test]
fn cancellation_wakes_the_same_watcher_and_its_claim_outlives_the_source() {
    struct Claim(*mut DrCancellationV1);
    // The claim keeps the atomic cancellation owner alive through this thread.
    unsafe impl Send for Claim {}
    impl Claim {
        fn request(&self) {
            let (mut reason, mut code, mut has_code) = (0, 0, 0);
            assert_eq!(
                unsafe {
                    cancellation::dr_v1_cancellation_request(
                        self.0,
                        &mut reason,
                        &mut code,
                        &mut has_code,
                    )
                },
                0
            );
        }
    }
    impl Drop for Claim {
        fn drop(&mut self) {
            unsafe {
                cancellation::dr_v1_cancellation_release(self.0);
            }
        }
    }
    let source = cancellation::dr_v1_cancellation_new();
    assert!(!source.is_null());
    let mut set = unsafe { Set::new(1, source).unwrap() };
    let claim = Claim(unsafe { cancellation::dr_v1_cancellation_retain(source) });
    unsafe {
        cancellation::dr_v1_cancellation_release(source);
    }
    let (stream, _peer) = UnixStream::pair().unwrap();
    set.add(stream.as_raw_fd(), 1, READ).unwrap();
    let mut output = [DrReadyEventV1::default(); 2];
    assert_eq!(set.wait(&budget(0, 0), &mut output).unwrap(), (1, 0));
    let thread = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(20));
        claim.request();
    });
    assert_eq!(
        set.wait(&budget(2, 2_000_000_000), &mut output).unwrap(),
        (2, 0)
    );
    thread.join().unwrap();
    let (result, allocations) =
        crate::probe_runtime_allocations(|| set.wait(&budget(1, 0), &mut output));
    assert_eq!(result.unwrap(), (2, 0));
    assert_eq!(allocations, 0);
}

#[test]
fn many_registrations_have_no_small_wait_handle_limit_or_event_allocation() {
    let mut set = unsafe { Set::new(96, ptr::null_mut()).unwrap() };
    let mut streams = Vec::new();
    for key in 0..96 {
        let (stream, peer) = UnixStream::pair().unwrap();
        set.add(stream.as_raw_fd(), key, READ).unwrap();
        streams.push((stream, peer));
    }
    let storage = (set.handle().unwrap(), set.memory, set.events, set.slots);
    let tokens: Vec<_> = set
        .slots()
        .iter()
        .map(|entry| (entry.key, entry.generation))
        .collect();
    let mut output = [DrReadyEventV1::default(); 192];
    let poll = budget(0, 0);
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        for round in 0..32u8 {
            for (key, (_, peer)) in streams.iter_mut().enumerate() {
                peer.write_all(&[key as u8, round]).unwrap();
            }
            let (tag, count) = set.wait(&poll, &mut output).unwrap();
            assert_eq!((tag, count), (0, 96));
            let mut seen = [false; 96];
            for event in &output[..count] {
                let key = event.key as usize;
                assert!(key < seen.len() && !seen[key]);
                assert_ne!(event.facts & READ, 0);
                seen[key] = true;
            }
            assert_eq!(seen, [true; 96]);
            for (key, (stream, _)) in streams.iter_mut().enumerate() {
                let mut actual = [0; 2];
                stream.read_exact(&mut actual).unwrap();
                assert_eq!(actual, [key as u8, round]);
            }
            assert_eq!(set.wait(&poll, &mut output).unwrap(), (1, 0));
            assert_eq!(
                (set.handle().unwrap(), set.memory, set.events, set.slots),
                storage
            );
            for (entry, token) in set.slots().iter().zip(&tokens) {
                assert_eq!((entry.key, entry.generation), *token);
            }
        }
    });
    assert_eq!(allocations, 0, "runtime storage, not kernel resources");
}

#[test]
fn many_ready_watches_release_only_the_poller_not_borrowed_streams() {
    let mut set = unsafe { Set::new(96, ptr::null_mut()).unwrap() };
    let mut streams = Vec::new();
    for key in 0..96 {
        let (stream, peer) = UnixStream::pair().unwrap();
        set.add(stream.as_raw_fd(), key, READ).unwrap();
        streams.push((stream, peer));
    }
    let ((), closes) = crate::native_handle::with_close_probe(|| drop(set));
    assert_eq!(closes, 1);
    for (key, (stream, peer)) in streams.iter_mut().enumerate() {
        peer.write_all(&[key as u8]).unwrap();
        let mut actual = [0];
        stream.read_exact(&mut actual).unwrap();
        assert_eq!(actual, [key as u8]);
    }
}

#[test]
fn abi_resets_stale_status_and_never_closes_registered_streams() {
    unsafe {
        let mut status = DrIoStatusV1 {
            value: 99,
            system_code: 99,
            reason: 7,
            has_system_code: 1,
            transferred_byte_count: 99,
            has_transferred_byte_count: 1,
        };
        assert!(dr_v1_unix_readiness_new(0, ptr::null_mut(), &mut status).is_null());
        assert_eq!(status.reason, Reason::InvalidInput as u8);
        assert_eq!(
            (
                status.value,
                status.has_system_code,
                status.has_transferred_byte_count
            ),
            (0, 0, 0)
        );
        let owner = dr_v1_unix_readiness_new(1, ptr::null_mut(), &mut status);
        assert!(!owner.is_null());
        let (mut stream, mut peer) = UnixStream::pair().unwrap();
        assert_eq!(
            dr_v1_unix_readiness_add(owner, stream.as_raw_fd(), 42, READ, &mut status),
            0
        );
        let poll = budget(0, 0);
        let mut output = [DrReadyEventV1::default(); 2];
        assert_eq!(
            dr_v1_unix_readiness_wait(owner, &poll, output.as_mut_ptr(), output.len(), &mut status),
            1
        );
        assert_eq!(status.value, 0);
        peer.write_all(b"a").unwrap();
        assert_eq!(
            dr_v1_unix_readiness_wait(owner, &poll, output.as_mut_ptr(), output.len(), &mut status),
            0
        );
        assert!(status.value > 0);
        assert_eq!(output[0].key, 42);
        assert_eq!(dr_v1_unix_readiness_remove(owner, 42, &mut status), 0);
        assert_eq!(
            (
                status.reason,
                status.has_system_code,
                status.has_transferred_byte_count
            ),
            (0, 0, 0)
        );
        dr_v1_unix_readiness_drop(owner);
        assert_eq!(stream.read(&mut [0; 1]).unwrap(), 1);
        dr_v1_unix_readiness_drop(ptr::null_mut());
    }
}

#[test]
fn growing_watch_storage_preserves_native_watcher_tokens_and_cancellation_registration() {
    let state = cancellation::dr_v1_cancellation_new();
    assert!(!state.is_null());
    let mut set = unsafe { Set::new(2, state).unwrap() };
    let (mut a, mut a_peer) = UnixStream::pair().unwrap();
    let (mut b, mut b_peer) = UnixStream::pair().unwrap();
    set.add(a.as_raw_fd(), 10, READ).unwrap();
    set.add(b.as_raw_fd(), 20, READ).unwrap();
    let watcher = set.handle().unwrap();
    let cancellation = set.cancellation;
    let a_generation = set.slots()[0].generation;
    let b_generation = set.slots()[1].generation;
    b_peer.write_all(b"b").unwrap();
    set.remove(10).unwrap();
    let old_memory = set.memory;
    let (result, closes) = crate::native_handle::with_close_probe(|| set.reserve(8));
    result.unwrap();
    assert_eq!(closes, 0);
    assert_ne!(set.memory, old_memory);
    assert_eq!(set.handle().unwrap(), watcher);
    assert_eq!(set.cancellation, cancellation);
    assert_eq!(set.capacity, 8);
    assert_eq!(
        (set.slots()[0].interests, set.slots()[0].generation),
        (0, a_generation)
    );
    let b_slot = set.slots()[1];
    assert_eq!(
        (
            b_slot.handle,
            b_slot.key,
            b_slot.generation,
            b_slot.interests
        ),
        (b.as_raw_fd(), 20, b_generation, READ)
    );
    assert!(set.slots()[2..]
        .iter()
        .all(|slot| slot.interests == 0 && slot.generation == 0));
    set.add(a.as_raw_fd(), 30, READ).unwrap();
    assert_eq!(set.slots()[0].generation, a_generation + 1);
    let poll = budget(0, 0);
    let mut output = [DrReadyEventV1::default(); 16];
    let (result, allocations) = crate::probe_runtime_allocations(|| set.wait(&poll, &mut output));
    let (tag, count) = result.unwrap();
    assert_eq!((tag, allocations), (0, 0));
    assert!(output[..count]
        .iter()
        .any(|event| event.key == 20 && event.facts & READ != 0));
    let memory = set.memory;
    let (result, allocations) = crate::probe_runtime_allocations(|| set.reserve(usize::MAX));
    assert_eq!(result, Err(Failure::invalid_input()));
    assert_eq!(allocations, 0);
    assert_eq!(set.memory, memory);
    assert_eq!(set.handle().unwrap(), watcher);
    let (mut reason, mut code, mut has_code) = (0, 0, 0);
    assert_eq!(
        unsafe {
            cancellation::dr_v1_cancellation_request(state, &mut reason, &mut code, &mut has_code)
        },
        0
    );
    assert_eq!(set.wait(&poll, &mut output).unwrap(), (2, 0));
    // The test retains the original cancellation claim; only the watcher closes
    // here, and the projected stream owners remain independently usable.
    let ((), closes) = crate::native_handle::with_close_probe(|| drop(set));
    assert_eq!(closes, 1);
    a_peer.write_all(b"a").unwrap();
    let mut byte = [0];
    a.read_exact(&mut byte).unwrap();
    assert_eq!(&byte, b"a");
    b.read_exact(&mut byte).unwrap();
    assert_eq!(&byte, b"b");
    unsafe { cancellation::dr_v1_cancellation_release(state) };
}
