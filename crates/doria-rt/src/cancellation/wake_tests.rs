use super::{
    dr_v1_cancellation_new, dr_v1_cancellation_release, dr_v1_cancellation_retain,
    DrCancellationV1, WakeRegistration, WakeTarget,
};
use crate::cancellation_signal::{self, Signal};
use crate::platform_io::{Failure, Reason};
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Barrier, Condvar, Mutex};
use std::time::{Duration, Instant};

struct Claim(*mut DrCancellationV1);

// Each test claim independently retains the stable state allocation. Shared
// access goes through its atomic facts or synchronized registration list.
unsafe impl Send for Claim {}
unsafe impl Sync for Claim {}

impl Claim {
    fn new() -> Self {
        let state = dr_v1_cancellation_new();
        assert!(!state.is_null());
        Self(state)
    }

    fn cancel(&self) -> Result<(), Failure> {
        unsafe { (*self.0).cancel() }
    }

    fn signal(&self) -> usize {
        unsafe { (*self.0).signal().unwrap() }
    }

    fn references(&self) -> usize {
        unsafe { (*self.0).references.load(Ordering::Relaxed) }
    }

    fn register<'a, T: WakeTarget>(&self, target: &'a T) -> Result<WakeRegistration<'a>, Failure> {
        unsafe { WakeRegistration::register(self.0, target) }
    }

    // Call only after the scoped notifier/registrar threads have joined.
    fn has_no_registrations(&self) -> bool {
        unsafe { (*(*self.0).wakes.head.get()).is_null() }
    }
}

impl Clone for Claim {
    fn clone(&self) -> Self {
        let retained = unsafe { dr_v1_cancellation_retain(self.0) };
        assert_eq!(retained, self.0);
        Self(retained)
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        unsafe { dr_v1_cancellation_release(self.0) }
    }
}

struct CountingTarget {
    calls: AtomicUsize,
    fail: AtomicBool,
    failure: Failure,
}

impl CountingTarget {
    fn new(fail: bool, code: i64) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            fail: AtomicBool::new(fail),
            failure: Failure {
                reason: Reason::Other,
                system_code: Some(code),
                transferred_byte_count: None,
            },
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

// No notification allocates, unwinds, reenters cancellation, or retains the
// borrowed target outside the synchronous callback.
unsafe impl WakeTarget for CountingTarget {
    fn wake(&self) -> Result<(), Failure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            Err(self.failure)
        } else {
            Ok(())
        }
    }
}

#[test]
fn checked_notification_failure_preserves_code_presence_and_notifies_every_waiter() {
    for expected_code in [None, Some(0), Some(-1), Some(i64::MAX)] {
        let source = Claim::new();
        let mut failed = CountingTarget::new(true, 0);
        failed.failure.system_code = expected_code;
        let healthy = CountingTarget::new(false, 0);
        let _failed_registration = source.register(&failed).unwrap();
        let _healthy_registration = source.register(&healthy).unwrap();
        let mut message = ptr::null_mut();
        let mut code = 91;
        let mut has_code = 1;
        let (status, allocations) = crate::probe_runtime_allocations(|| unsafe {
            super::dr_v2_checked_cancellation_request(
                ptr::null(),
                source.0,
                &mut message,
                &mut code,
                &mut has_code,
            )
        });
        assert_eq!(status, 1);
        assert_eq!(allocations, 1);
        assert!(!message.is_null());
        assert_eq!(code, expected_code.unwrap_or(0));
        assert_eq!(has_code, u8::from(expected_code.is_some()));
        assert_eq!(failed.calls(), 1);
        assert_eq!(healthy.calls(), 1);
        assert_eq!(source.references(), 3);
        unsafe {
            assert!((*source.0).cancelled.load(Ordering::SeqCst));
            assert_eq!((*source.0).signal.load(Ordering::Relaxed), 0);
            let bytes = core::slice::from_raw_parts(
                crate::string_bytes(message),
                crate::dr_v1_string_byte_length(message),
            );
            assert_eq!(
                bytes,
                doria_diagnostic_catalogue::CANCELLATION_NOTIFICATION_FAILURE_MESSAGE.as_bytes()
            );
            crate::dr_v1_string_release(message);
        }
        message = ptr::null_mut();
        failed.fail.store(false, Ordering::SeqCst);
        let (status, allocations) = crate::probe_runtime_allocations(|| unsafe {
            super::dr_v2_checked_cancellation_request(
                ptr::null(),
                source.0,
                &mut message,
                &mut code,
                &mut has_code,
            )
        });
        assert_eq!(status, 0);
        assert_eq!(allocations, 0);
        assert!(message.is_null());
        assert_eq!((code, has_code), (0, 0));
        assert_eq!(failed.calls(), 2);
        assert_eq!(healthy.calls(), 2);
    }
}

struct SignalTarget {
    signal: Signal,
    calls: AtomicUsize,
}

impl SignalTarget {
    fn new() -> Self {
        Self {
            signal: Signal::new().unwrap(),
            calls: AtomicUsize::new(0),
        }
    }

    fn signalled(&self) -> bool {
        cancellation_signal::wait_for_test(self.signal.get(), 0).unwrap()
    }
}

// The target owns its signal, which remains borrowed until guard destruction.
// Native set is non-consuming and queues no pointer into this target.
unsafe impl WakeTarget for SignalTarget {
    fn wake(&self) -> Result<(), Failure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        cancellation_signal::set(self.signal.get())
    }
}

#[test]
fn registration_claim_outlives_source_and_unregisters_before_target_close() {
    let source = Claim::new();
    let target = SignalTarget::new();
    let registration = source.register(&target).unwrap();
    assert_eq!(source.references(), 2);
    assert!(!target.signalled());
    assert_eq!(unsafe { (*source.0).signal.load(Ordering::Relaxed) }, 0);

    let (_, closes) = crate::native_handle::with_close_probe(|| drop(source));
    assert_eq!(closes, 0);
    assert!(!unsafe { (*registration.state).cancelled.load(Ordering::SeqCst) });
    assert_eq!(
        unsafe { (*registration.state).references.load(Ordering::Relaxed) },
        1
    );
    unsafe { (*registration.state).cancel().unwrap() };
    assert!(target.signalled());
    assert_eq!(target.calls.load(Ordering::SeqCst), 1);

    // Releasing the last state claim does not close the independently owned
    // target. Its Rust borrow ends only after synchronized unregistration.
    let (_, closes) = crate::native_handle::with_close_probe(|| drop(registration));
    assert_eq!(closes, 0);
    assert!(target.signalled());
    let (_, closes) = crate::native_handle::with_close_probe(|| drop(target));
    assert_eq!(closes, 1);
}

#[test]
fn already_requested_cancellation_notifies_a_new_registration_immediately() {
    let source = Claim::new();
    source.cancel().unwrap();
    let target = CountingTarget::new(false, 0);
    let registration = source.register(&target).unwrap();
    assert_eq!(target.calls(), 1);
    assert_eq!(source.references(), 2);
    assert_eq!(unsafe { (*source.0).signal.load(Ordering::Relaxed) }, 0);
    source.cancel().unwrap();
    assert_eq!(target.calls(), 2);
    drop(registration);
    assert!(source.has_no_registrations());
    assert_eq!(source.references(), 1);
    source.cancel().unwrap();
    assert_eq!(target.calls(), 2);
}

#[test]
fn moving_guards_keeps_stable_nodes_and_unlink_handles_head_middle_and_tail() {
    let source = Claim::new();
    let targets: [CountingTarget; 4] = std::array::from_fn(|_| CountingTarget::new(false, 0));
    let a = source.register(&targets[0]).unwrap();
    let b = source.register(&targets[1]).unwrap();
    let c = source.register(&targets[2]).unwrap();
    let d = source.register(&targets[3]).unwrap();
    let stable_node = c.node;
    let moved_c = Some(c);
    assert_eq!(moved_c.as_ref().unwrap().node, stable_node);

    drop(b); // Middle of the intrusive list.
    source.cancel().unwrap();
    assert_eq!(targets.each_ref().map(CountingTarget::calls), [1, 0, 1, 1]);
    drop(d); // Head.
    source.cancel().unwrap();
    assert_eq!(targets.each_ref().map(CountingTarget::calls), [2, 0, 2, 1]);
    drop(a); // Tail.
    source.cancel().unwrap();
    assert_eq!(targets.each_ref().map(CountingTarget::calls), [2, 0, 3, 1]);
    drop(moved_c);
    assert!(source.has_no_registrations());
    assert_eq!(source.references(), 1);
}

#[test]
fn registration_racing_cancellation_never_loses_notification_for_any_target() {
    for _ in 0..32 {
        let source = Claim::new();
        let targets: [CountingTarget; 3] = std::array::from_fn(|_| CountingTarget::new(false, 0));
        let barrier = Barrier::new(4);
        std::thread::scope(|scope| {
            for target in &targets {
                let token = source.clone();
                let barrier = &barrier;
                scope.spawn(move || {
                    barrier.wait();
                    let registration = token.register(target).unwrap();
                    // Hold every registration until the cancellation call has
                    // returned, regardless of publication/request ordering.
                    barrier.wait();
                    assert!(target.calls() >= 1);
                    drop(registration);
                });
            }
            barrier.wait();
            source.cancel().unwrap();
            barrier.wait();
        });
        assert_eq!(source.references(), 1);
        assert!(source.has_no_registrations());
    }
}

#[test]
fn notification_failure_preserves_first_error_without_skipping_targets_or_retry() {
    let source = Claim::new();
    let good_a = CountingTarget::new(false, 0);
    let bad_a = CountingTarget::new(true, 101);
    let good_b = CountingTarget::new(false, 0);
    let bad_b = CountingTarget::new(true, 202);
    let _good_a = source.register(&good_a).unwrap();
    let _bad_a = source.register(&bad_a).unwrap();
    let _good_b = source.register(&good_b).unwrap();
    let bad_b_registration = source.register(&bad_b).unwrap();

    // The last registration is the head and supplies the first failure.
    assert_eq!(source.cancel(), Err(bad_b.failure));
    assert_eq!(
        [good_a.calls(), bad_a.calls(), good_b.calls(), bad_b.calls()],
        [1; 4]
    );
    assert!(unsafe { (*source.0).cancelled.load(Ordering::SeqCst) });

    drop(bad_b_registration);
    assert_eq!(source.cancel(), Err(bad_a.failure));
    assert_eq!(
        [good_a.calls(), bad_a.calls(), good_b.calls(), bad_b.calls()],
        [2, 2, 2, 1]
    );
    bad_a.fail.store(false, Ordering::SeqCst);
    assert_eq!(source.cancel(), Ok(()));
    assert_eq!(
        [good_a.calls(), bad_a.calls(), good_b.calls(), bad_b.calls()],
        [3, 3, 3, 1]
    );
}

#[test]
fn immediate_notification_failure_rolls_back_only_the_new_node_and_claim() {
    let source = Claim::new();
    source.cancel().unwrap();
    let existing = CountingTarget::new(false, 0);
    let existing_registration = source.register(&existing).unwrap();
    let existing_head = unsafe { *(*source.0).wakes.head.get() };
    let failing = CountingTarget::new(true, 303);
    let (result, allocations) = crate::probe_runtime_allocations(|| source.register(&failing));
    assert_eq!(result.err(), Some(failing.failure));
    assert_eq!(allocations, 1);
    assert_eq!(failing.calls(), 1);
    assert_eq!(source.references(), 2);
    assert_eq!(unsafe { *(*source.0).wakes.head.get() }, existing_head);

    source.cancel().unwrap();
    assert_eq!(existing.calls(), 2);
    assert_eq!(failing.calls(), 1);
    failing.fail.store(false, Ordering::SeqCst);
    let retried_registration = source.register(&failing).unwrap();
    assert_eq!(failing.calls(), 2);
    assert_eq!(source.references(), 3);
    drop(retried_registration);
    drop(existing_registration);
    assert!(source.has_no_registrations());
    assert_eq!(source.references(), 1);
}

#[test]
fn null_and_exhausted_state_reject_registration_without_node_allocation() {
    let target = CountingTarget::new(false, 0);
    let (result, allocations) = crate::probe_runtime_allocations(|| unsafe {
        WakeRegistration::register(ptr::null_mut(), &target)
    });
    assert_eq!(result.err(), Some(Failure::invalid_input()));
    assert_eq!(allocations, 0);

    let source = Claim::new();
    unsafe {
        (*source.0)
            .references
            .store(isize::MAX as usize, Ordering::Relaxed);
    }
    let (result, allocations) = crate::probe_runtime_allocations(|| source.register(&target));
    let observed_references = source.references();
    // Restore the real claim count before assertions can unwind the fixture.
    unsafe { (*source.0).references.store(1, Ordering::Relaxed) };
    assert_eq!(result.err().unwrap().reason, Reason::ResourceExhausted);
    assert_eq!(allocations, 0);
    assert_eq!(observed_references, isize::MAX as usize);
    assert_eq!(target.calls(), 0);
    assert!(source.has_no_registrations());
}

#[test]
fn configuration_allocates_one_node_but_repeated_notification_and_unlink_do_not() {
    let (source, allocations) = crate::probe_runtime_allocations(Claim::new);
    assert_eq!(allocations, 1);
    let target = CountingTarget::new(false, 0);
    let (registration, allocations) = crate::probe_runtime_allocations(|| source.register(&target));
    let registration = registration.unwrap();
    assert_eq!(allocations, 1);
    let (_, allocations) = crate::probe_runtime_allocations(|| {
        for _ in 0..64 {
            source.cancel().unwrap();
        }
    });
    assert_eq!(allocations, 0);
    assert_eq!(target.calls(), 64);
    assert_eq!(unsafe { (*source.0).signal.load(Ordering::Relaxed) }, 0);
    let (_, allocations) = crate::probe_runtime_allocations(|| drop(registration));
    assert_eq!(allocations, 0);
    assert_eq!(source.references(), 1);
    assert!(source.has_no_registrations());
}

#[test]
fn registered_targets_coexist_with_persistent_native_signal_and_multiple_waiters() {
    let source = Claim::new();
    let signal = source.signal();
    let targets: [SignalTarget; 3] = std::array::from_fn(|_| SignalTarget::new());
    let registrations = targets
        .each_ref()
        .map(|target| source.register(target).unwrap());
    let barrier = Barrier::new(4);
    std::thread::scope(|scope| {
        for target in &targets {
            let token = source.clone();
            let barrier = &barrier;
            scope.spawn(move || {
                barrier.wait();
                assert!(cancellation_signal::wait_for_test(target.signal.get(), 5_000).unwrap());
                assert!(cancellation_signal::wait_for_test(token.signal(), 0).unwrap());
            });
        }
        barrier.wait();
        let (_, allocations) = crate::probe_runtime_allocations(|| source.cancel().unwrap());
        assert_eq!(allocations, 0);
    });
    drop(registrations);
    assert!(source.has_no_registrations());
    for _ in 0..8 {
        source.cancel().unwrap();
        assert_eq!(source.signal(), signal);
        assert!(cancellation_signal::wait_for_test(signal, 0).unwrap());
        for target in &targets {
            assert!(target.signalled());
            assert_eq!(target.calls.load(Ordering::SeqCst), 1);
        }
    }
}

#[derive(Default)]
struct GateState {
    entered: bool,
    unregister: bool,
    release: bool,
}

#[derive(Default)]
struct GatedTarget {
    state: Mutex<GateState>,
    changed: Condvar,
    calls: AtomicUsize,
    returned: AtomicBool,
    close_attempted: AtomicBool,
    closed: AtomicBool,
    closed_after_return: AtomicBool,
}

impl GatedTarget {
    fn wait_until(&self, ready: impl Fn(&GateState) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut state = self.state.lock().unwrap();
        while !ready(&state) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "test coordinator was not notified");
            state = self.changed.wait_timeout(state, remaining).unwrap().0;
        }
    }

    fn release_callback(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.release = true;
        self.changed.notify_all();
    }

    fn start_unregister(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.unregister = true;
        self.changed.notify_all();
    }
}

struct ReleaseGate<'a>(&'a GatedTarget);

impl Drop for ReleaseGate<'_> {
    fn drop(&mut self) {
        self.0.start_unregister();
        self.0.release_callback();
    }
}

// This callback is deliberately gated by a separate test coordinator so an
// in-flight notification overlaps unregistration. It never waits for the
// unregistering waiter, allocates, reenters cancellation, or retains its target.
unsafe impl WakeTarget for GatedTarget {
    fn wake(&self) -> Result<(), Failure> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.entered = true;
        self.changed.notify_all();
        while !state.release {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
        self.returned.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn target_close_after_unregister_waits_for_an_in_flight_notification() {
    let source = Claim::new();
    let target = GatedTarget::default();
    let registered = Barrier::new(2);
    std::thread::scope(|scope| {
        // Release both test-only gates even if a coordinator assertion fails,
        // before thread::scope joins its potentially blocked child threads.
        let _release_on_exit = ReleaseGate(&target);
        let token = source.clone();
        let target = &target;
        let registered = &registered;
        scope.spawn(move || {
            let registration = token.register(target).unwrap();
            registered.wait();
            target.wait_until(|state| state.unregister);
            {
                let _state = target.state.lock().unwrap();
                target.close_attempted.store(true, Ordering::SeqCst);
                target.changed.notify_all();
            }
            drop(registration);
            target
                .closed_after_return
                .store(target.returned.load(Ordering::SeqCst), Ordering::SeqCst);
            target.closed.store(true, Ordering::SeqCst);
        });
        registered.wait();
        let notifier = source.clone();
        scope.spawn(move || notifier.cancel().unwrap());
        target.wait_until(|state| state.entered);
        target.start_unregister();
        target.wait_until(|_| target.close_attempted.load(Ordering::SeqCst));
        let closed_while_notification_active = target.closed.load(Ordering::SeqCst);
        // Always release the test-only gate before asserting, so a failing
        // implementation cannot strand the scoped notifier during unwinding.
        target.release_callback();
        assert!(!closed_while_notification_active);
    });
    assert!(target.closed.load(Ordering::SeqCst));
    assert!(target.closed_after_return.load(Ordering::SeqCst));
    assert_eq!(target.calls.load(Ordering::SeqCst), 1);
    assert_eq!(source.references(), 1);
    assert!(source.has_no_registrations());
    source.cancel().unwrap();
    assert_eq!(target.calls.load(Ordering::SeqCst), 1);
}
