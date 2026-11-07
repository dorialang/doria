//! Pinned, allocation-free Windows synchronization for the shared broker.

use core::cell::UnsafeCell;
use core::marker::{PhantomData, PhantomPinned};
use core::ops::{Deref, DerefMut};
use core::pin::Pin;

use crate::platform_io::{last_failure, Failure};
use crate::win32::{
    sleep_condition_variable, AcquireSRWLockExclusive, ConditionVariable as NativeCondition,
    ReleaseSRWLockExclusive, SrwLock as NativeLock, WakeAllConditionVariable,
};

const ERROR_TIMEOUT: i64 = 1460;

/// Pins the native synchronization storage, not the protected value.
/// Exclusive acquisition is nonrecursive and provides no fairness guarantee.
pub(super) struct SyncCell<T> {
    lock: UnsafeCell<NativeLock>,
    condition: UnsafeCell<NativeCondition>,
    value: UnsafeCell<T>,
    // Protected by the native lock, and unchanged while a guard sleeps. This
    // counts guards rather than lock holders: multiple guards can be sleeping
    // while another guard owns the temporarily released lock.
    guards: UnsafeCell<usize>,
    _pin: PhantomPinned,
}

// The native lock serializes every protected-value access. Moving ownership
// of T between callers requires Send, but T need not permit shared access.
unsafe impl<T: Send> Send for SyncCell<T> {}
unsafe impl<T: Send> Sync for SyncCell<T> {}

impl<T> SyncCell<T> {
    pub(super) const fn new(value: T) -> Self {
        Self {
            lock: UnsafeCell::new(NativeLock::new()),
            condition: UnsafeCell::new(NativeCondition::new()),
            value: UnsafeCell::new(value),
            guards: UnsafeCell::new(0),
            _pin: PhantomPinned,
        }
    }

    pub(super) fn lock(self: Pin<&Self>) -> Guard<'_, T> {
        let cell = self.get_ref();
        unsafe {
            AcquireSRWLockExclusive(cell.lock.get());
            let guards = &mut *cell.guards.get();
            *guards = guards
                .checked_add(1)
                .unwrap_or_else(|| crate::runtime_invariant_panic());
        }
        Guard {
            cell,
            _same_thread: PhantomData,
        }
    }

    /// Publish the caller's durable predicate under a guard before waking.
    /// A wake is not a stored event; every waiter must recheck its predicate.
    pub(super) fn wake_all(self: Pin<&Self>) {
        unsafe { WakeAllConditionVariable(self.get_ref().condition.get()) };
    }
}

impl<T> Drop for SyncCell<T> {
    fn drop(&mut self) {
        // Safe code can forget a guard and end its Rust borrow without unlocking
        // the native object. Do not free that in-use storage or its value. Live
        // borrows otherwise exclude destruction, including acquisition/wait.
        if *self.guards.get_mut() != 0 {
            unsafe { crate::runtime_invariant_panic() };
        }
        // An unlocked SRW lock with no waiting threads needs no native destroy.
    }
}

/// Keeps exclusive ownership on the acquiring thread, except inside wait().
pub(super) struct Guard<'a, T> {
    cell: &'a SyncCell<T>,
    _same_thread: PhantomData<*mut ()>,
}

impl<T> Guard<'_, T> {
    /// Atomically releases the exclusive lock and sleeps, then reacquires it.
    /// Returns true for a wake (possibly spurious), false for timeout. The
    /// protected predicate must be checked again after either result.
    pub(super) fn wait(&mut self, milliseconds: u32) -> Result<bool, Failure> {
        let success = unsafe {
            sleep_condition_variable(
                self.cell.condition.get(),
                self.cell.lock.get(),
                milliseconds,
                "another group's broker pump to publish this group's completion",
            )
        };
        if success != 0 {
            return Ok(true);
        }
        let failure = last_failure();
        if failure.system_code == Some(ERROR_TIMEOUT) {
            Ok(false)
        } else {
            Err(failure)
        }
    }
}

impl<T> Deref for Guard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.cell.value.get() }
    }
}

impl<T> DerefMut for Guard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.cell.value.get() }
    }
}

impl<T> Drop for Guard<'_, T> {
    fn drop(&mut self) {
        unsafe {
            {
                let guards = &mut *self.cell.guards.get();
                *guards = guards
                    .checked_sub(1)
                    .unwrap_or_else(|| crate::runtime_invariant_panic());
            }
            ReleaseSRWLockExclusive(self.cell.lock.get());
        }
    }
}

#[cfg(test)]
mod tests;
