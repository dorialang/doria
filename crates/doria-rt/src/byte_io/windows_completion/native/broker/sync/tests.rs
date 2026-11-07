//! Windows execution is required; cross-compilation alone is not this evidence.

#![cfg(windows)]

use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{self, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use core::ffi::c_void;

use super::*;

const TEST_WAIT: Duration = Duration::from_secs(5);

// Every spawned caller owns a pinned Arc. If a bounded receive fails, no test
// stack pointer or native synchronization storage can be freed under that caller.
fn wait_for_true(mut guard: Guard<'_, bool>) -> Result<(), Failure> {
    let deadline = Instant::now() + TEST_WAIT;
    while !*guard {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(Failure::from_system_code(ERROR_TIMEOUT));
        }
        let milliseconds = remaining.as_millis().clamp(1, u128::from(u32::MAX - 1)) as u32;
        if !guard.wait(milliseconds)? && !*guard {
            return Err(Failure::from_system_code(ERROR_TIMEOUT));
        }
    }
    Ok(())
}

#[test]
fn native_storage_has_sdk_pointer_layout_and_zero_initializer() {
    assert_eq!(
        core::mem::size_of::<NativeLock>(),
        core::mem::size_of::<*mut c_void>()
    );
    assert_eq!(
        core::mem::align_of::<NativeLock>(),
        core::mem::align_of::<*mut c_void>()
    );
    assert_eq!(
        core::mem::size_of::<NativeCondition>(),
        core::mem::size_of::<*mut c_void>()
    );
    assert_eq!(
        core::mem::align_of::<NativeCondition>(),
        core::mem::align_of::<*mut c_void>()
    );
    let cell = SyncCell::new(());
    assert!(unsafe { NativeLock::is_initial(cell.lock.get()) });
    assert!(unsafe { NativeCondition::is_initial(cell.condition.get()) });
}

#[test]
fn exclusive_contention_preserves_every_mutation() {
    const CALLERS: usize = 4;
    const INCREMENTS: usize = 250;
    let cell = Arc::pin(SyncCell::new(0usize));
    let held = cell.as_ref().lock();
    let (started_tx, started_rx) = mpsc::sync_channel(CALLERS);
    let (done_tx, done_rx) = mpsc::sync_channel(CALLERS);
    let mut callers = Vec::new();
    for _ in 0..CALLERS {
        let cell = cell.clone();
        let started = started_tx.clone();
        let done = done_tx.clone();
        callers.push(thread::spawn(move || {
            started.send(()).unwrap();
            for _ in 0..INCREMENTS {
                *cell.as_ref().lock() += 1;
            }
            done.send(()).unwrap();
        }));
    }
    for _ in 0..CALLERS {
        started_rx.recv_timeout(TEST_WAIT).unwrap();
    }
    assert_eq!(done_rx.try_recv(), Err(TryRecvError::Empty));
    assert_eq!(*held, 0);
    drop(held);
    for _ in 0..CALLERS {
        done_rx.recv_timeout(TEST_WAIT).unwrap();
    }
    for caller in callers {
        caller.join().unwrap();
    }
    assert_eq!(*cell.as_ref().lock(), CALLERS * INCREMENTS);
}

#[test]
fn published_predicate_survives_wake_before_sleep() {
    let cell = Arc::pin(SyncCell::new(false));
    {
        let mut guard = cell.as_ref().lock();
        *guard = true;
        cell.as_ref().wake_all();
    }
    let caller_cell = cell.clone();
    let (done_tx, done_rx) = mpsc::sync_channel(1);
    let caller = thread::spawn(move || {
        let guard = caller_cell.as_ref().lock();
        let already_published = *guard;
        let result = wait_for_true(guard);
        done_tx.send((already_published, result)).unwrap();
    });
    let (already_published, result) = done_rx.recv_timeout(TEST_WAIT).unwrap();
    caller.join().unwrap();
    assert!(already_published);
    assert_eq!(result, Ok(()));
}

#[test]
fn publication_wakes_all_sleeping_guards_and_reacquires_lock() {
    const CALLERS: usize = 3;
    let cell = Arc::pin(SyncCell::new(false));
    let (started_tx, started_rx) = mpsc::sync_channel(CALLERS);
    let (done_tx, done_rx) = mpsc::sync_channel(CALLERS);
    let mut callers = Vec::new();
    for _ in 0..CALLERS {
        let cell = cell.clone();
        let started = started_tx.clone();
        let done = done_tx.clone();
        callers.push(thread::spawn(move || {
            let guard = cell.as_ref().lock();
            started.send(()).unwrap();
            done.send(wait_for_true(guard)).unwrap();
        }));
    }
    for _ in 0..CALLERS {
        started_rx.recv_timeout(TEST_WAIT).unwrap();
    }
    {
        // Each caller signaled while holding its guard, then atomically
        // released the lock in wait. Acquisition here closes that race.
        let mut guard = cell.as_ref().lock();
        assert!(!*guard);
        assert_eq!(unsafe { *guard.cell.guards.get() }, CALLERS + 1);
        *guard = true;
        cell.as_ref().wake_all();
    }
    for _ in 0..CALLERS {
        assert_eq!(done_rx.recv_timeout(TEST_WAIT).unwrap(), Ok(()));
    }
    for caller in callers {
        caller.join().unwrap();
    }
    let guard = cell.as_ref().lock();
    assert!(*guard);
    assert_eq!(unsafe { *guard.cell.guards.get() }, 1);
}

#[test]
fn zero_timeout_returns_false_with_exclusive_guard_reacquired() {
    let cell = core::pin::pin!(SyncCell::new(10usize));
    {
        let mut guard = cell.as_ref().lock();
        assert_eq!(guard.wait(0), Ok(false));
        *guard += 1;
        assert_eq!(unsafe { *guard.cell.guards.get() }, 1);
    }
    assert_eq!(*cell.as_ref().lock(), 11);
}

#[test]
fn rust_host_unwind_releases_guard_without_poisoning_cell() {
    let cell = Arc::pin(SyncCell::new(0usize));
    assert!(panic::catch_unwind(AssertUnwindSafe(|| {
        let mut guard = cell.as_ref().lock();
        *guard = 41;
        panic!("test-only Rust host unwind");
    }))
    .is_err());

    let caller_cell = cell.clone();
    let (done_tx, done_rx) = mpsc::sync_channel(1);
    let caller = thread::spawn(move || {
        let result = {
            let mut guard = caller_cell.as_ref().lock();
            *guard += 1;
            *guard
        };
        done_tx.send(result).unwrap();
    });
    assert_eq!(done_rx.recv_timeout(TEST_WAIT).unwrap(), 42);
    caller.join().unwrap();
}
