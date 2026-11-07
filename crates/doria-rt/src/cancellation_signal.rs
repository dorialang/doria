use crate::native_handle::{NativeHandle, OwnedHandle};
use crate::platform_io::Failure;

#[cfg(any(unix, windows))]
pub(crate) use wake_lock::{WakeCondition, WakeGuard, WakeLock};

/// Shared pinned native mutex used by wake registration and standard-device
/// native helper retirement. Unsupported targets expose no substitute lock.
#[cfg(any(unix, windows))]
mod wake_lock {
    use core::cell::UnsafeCell;
    use core::marker::{PhantomData, PhantomPinned};
    use core::pin::Pin;
    use core::sync::atomic::{AtomicBool, Ordering};

    /// A nonrecursive, blocking lock for the cancellation wake-registration list.
    /// Pin before first acquisition and retain that address through destruction.
    pub(crate) struct WakeLock {
        native: UnsafeCell<NativeLock>,
        held: AtomicBool,
        _pinned: PhantomPinned,
    }

    // Native acquisition/release synchronize access to the registration list.
    // Pinning excludes moving a live native mutex or a lock with waiting threads.
    unsafe impl Send for WakeLock {}
    unsafe impl Sync for WakeLock {}

    impl WakeLock {
        pub(crate) const fn new() -> Self {
            Self {
                native: UnsafeCell::new(native_new()),
                held: AtomicBool::new(false),
                _pinned: PhantomPinned,
            }
        }

        /// The caller must not recursively acquire the same lock. The returned
        /// guard releases on this thread, including host-test unwinding.
        pub(crate) fn lock(self: Pin<&Self>) -> WakeGuard<'_> {
            unsafe { native_lock(self.native.get()) };
            self.held.store(true, Ordering::Release);
            WakeGuard {
                lock: self.get_ref(),
                _thread_bound: PhantomData,
            }
        }
    }

    impl Drop for WakeLock {
        fn drop(&mut self) {
            // A forgotten guard is possible in safe Rust. Never destroy a held
            // pthread mutex, whose destruction would violate its native contract.
            if self.held.load(Ordering::Acquire) {
                unsafe { crate::runtime_invariant_panic() };
            }
            unsafe { native_destroy(self.native.get()) };
        }
    }

    /// Unlock must execute on the thread that acquired the native lock.
    pub(crate) struct WakeGuard<'a> {
        lock: &'a WakeLock,
        _thread_bound: PhantomData<*mut ()>,
    }

    pub(crate) struct WakeCondition {
        native: UnsafeCell<NativeCondition>,
        _pinned: PhantomPinned,
    }

    unsafe impl Send for WakeCondition {}
    unsafe impl Sync for WakeCondition {}

    impl WakeCondition {
        pub(crate) const fn new() -> Self {
            Self {
                native: UnsafeCell::new(condition_new()),
                _pinned: PhantomPinned,
            }
        }

        pub(crate) fn wake_all(self: Pin<&Self>) {
            unsafe { condition_wake(self.get_ref().native.get()) }
        }
    }

    impl Drop for WakeCondition {
        fn drop(&mut self) {
            unsafe { condition_destroy(self.native.get()) }
        }
    }

    impl WakeGuard<'_> {
        /// The caller retains both pinned objects and rechecks its protected
        /// predicate after every wake. Native wait atomically unlocks/relocks.
        pub(crate) fn wait(&mut self, condition: Pin<&WakeCondition>) {
            self.lock.held.store(false, Ordering::Release);
            unsafe { condition_wait(condition.get_ref().native.get(), self.lock.native.get()) }
            self.lock.held.store(true, Ordering::Release);
        }
    }

    impl Drop for WakeGuard<'_> {
        fn drop(&mut self) {
            self.lock.held.store(false, Ordering::Release);
            unsafe { native_unlock(self.lock.native.get()) };
        }
    }

    #[cfg(unix)]
    type NativeLock = libc::pthread_mutex_t;

    #[cfg(unix)]
    const fn native_new() -> NativeLock {
        libc::PTHREAD_MUTEX_INITIALIZER
    }

    #[cfg(unix)]
    unsafe fn native_lock(lock: *mut NativeLock) {
        if libc::pthread_mutex_lock(lock) != 0 {
            crate::runtime_invariant_panic();
        }
    }

    #[cfg(unix)]
    unsafe fn native_unlock(lock: *mut NativeLock) {
        if libc::pthread_mutex_unlock(lock) != 0 {
            crate::runtime_invariant_panic();
        }
    }

    #[cfg(unix)]
    unsafe fn native_destroy(lock: *mut NativeLock) {
        if libc::pthread_mutex_destroy(lock) != 0 {
            crate::runtime_invariant_panic();
        }
    }

    #[cfg(unix)]
    type NativeCondition = libc::pthread_cond_t;
    #[cfg(unix)]
    const fn condition_new() -> NativeCondition {
        libc::PTHREAD_COND_INITIALIZER
    }
    #[cfg(unix)]
    unsafe fn condition_wait(condition: *mut NativeCondition, lock: *mut NativeLock) {
        if libc::pthread_cond_wait(condition, lock) != 0 {
            crate::runtime_invariant_panic();
        }
    }
    #[cfg(unix)]
    unsafe fn condition_wake(condition: *mut NativeCondition) {
        if libc::pthread_cond_broadcast(condition) != 0 {
            crate::runtime_invariant_panic();
        }
    }
    #[cfg(unix)]
    unsafe fn condition_destroy(condition: *mut NativeCondition) {
        if libc::pthread_cond_destroy(condition) != 0 {
            crate::runtime_invariant_panic();
        }
    }

    // SRWLOCK and CONDITION_VARIABLE are opaque words that only the SRW API
    // below reads or writes.
    #[cfg(windows)]
    type NativeLock = crate::win32::SrwLock;

    #[cfg(windows)]
    type NativeCondition = crate::win32::ConditionVariable;
    #[cfg(windows)]
    const fn condition_new() -> NativeCondition {
        NativeCondition::new()
    }
    #[cfg(windows)]
    unsafe fn condition_wait(condition: *mut NativeCondition, lock: *mut NativeLock) {
        if SleepConditionVariableSRW(condition, lock, u32::MAX, 0) == 0 {
            crate::runtime_invariant_panic();
        }
    }
    #[cfg(windows)]
    unsafe fn condition_wake(condition: *mut NativeCondition) {
        WakeAllConditionVariable(condition);
    }
    #[cfg(windows)]
    unsafe fn condition_destroy(_: *mut NativeCondition) {}

    #[cfg(windows)]
    const fn native_new() -> NativeLock {
        NativeLock::new()
    }

    #[cfg(windows)]
    unsafe fn native_lock(lock: *mut NativeLock) {
        AcquireSRWLockExclusive(lock);
    }

    #[cfg(windows)]
    unsafe fn native_unlock(lock: *mut NativeLock) {
        ReleaseSRWLockExclusive(lock);
    }

    // An unlocked SRW lock without waiting threads needs no destruction API.
    #[cfg(windows)]
    unsafe fn native_destroy(_: *mut NativeLock) {}

    #[cfg(windows)]
    use crate::win32::{
        AcquireSRWLockExclusive, ReleaseSRWLockExclusive, SleepConditionVariableSRW,
        WakeAllConditionVariable,
    };

    #[cfg(test)]
    mod tests {
        use super::*;
        use core::sync::atomic::AtomicUsize;
        use std::panic::{catch_unwind, AssertUnwindSafe};

        #[test]
        fn guarded_native_lock_serializes_contending_threads() {
            let lock = core::pin::pin!(WakeLock::new());
            let count = AtomicUsize::new(0);
            std::thread::scope(|scope| {
                for _ in 0..4 {
                    let lock = lock.as_ref();
                    let count = &count;
                    scope.spawn(move || {
                        for _ in 0..250 {
                            let _guard = lock.lock();
                            count.store(count.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
                        }
                    });
                }
            });
            assert_eq!(count.load(Ordering::Relaxed), 1_000);
        }

        #[test]
        fn guard_unlocks_during_host_unwinding_without_poisoning() {
            let lock = core::pin::pin!(WakeLock::new());
            assert!(catch_unwind(AssertUnwindSafe(|| {
                let _guard = lock.as_ref().lock();
                panic!("test-only unwind while holding native wake lock");
            }))
            .is_err());
            let _guard = lock.as_ref().lock();
        }
    }
}

/// A persistent, noninheritable native notification, borrowed by readiness
/// registrations. Zero denotes no signal; Unix descriptors are encoded as fd+1.
pub(crate) struct Signal(usize);

impl Signal {
    pub(crate) fn new() -> Result<Self, Failure> {
        #[cfg(test)]
        crate::io_cost_probe::record(crate::io_cost_probe::Operation::SignalCreate);
        platform::create().map(Self)
    }

    pub(crate) fn get(&self) -> usize {
        self.0
    }

    pub(crate) fn into_raw(self) -> usize {
        let handle = self.0;
        core::mem::forget(self);
        handle
    }
}

impl Drop for Signal {
    fn drop(&mut self) {
        unsafe { close(self.0) }
    }
}

pub(crate) fn set(signal: usize) -> Result<(), Failure> {
    platform::set(signal)
}

/// Only reusable helper notices reset. Cancellation signals remain latched.
/// Caller excludes publication of a new job until reset finishes.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn reset(signal: usize) -> Result<(), Failure> {
    platform::reset(signal)
}

/// # Safety
/// `signal` must be a live signal returned by Signal::into_raw, closed once
/// after every borrowed registration and waiter has released its state claim.
pub(crate) unsafe fn close(signal: usize) {
    drop(OwnedHandle::from_raw(native_handle(signal)));
}

pub(crate) fn native_handle(signal: usize) -> NativeHandle {
    #[cfg(unix)]
    {
        (signal - 1) as NativeHandle
    }
    #[cfg(windows)]
    {
        signal as NativeHandle
    }
    #[cfg(not(any(unix, windows)))]
    {
        signal
    }
}

#[cfg(test)]
pub(crate) fn wait_for_test(signal: usize, milliseconds: u32) -> Result<bool, Failure> {
    platform::wait_for_test(signal, milliseconds)
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod platform {
    use super::*;
    use core::ffi::c_void;

    pub(super) fn create() -> Result<usize, Failure> {
        const EFD_CLOEXEC: i32 = 0x80000;
        const EFD_NONBLOCK: i32 = 0x800;
        let descriptor = unsafe { eventfd(0, EFD_CLOEXEC | EFD_NONBLOCK) };
        if descriptor < 0 {
            Err(crate::platform_io::last_failure())
        } else {
            Ok(descriptor as usize + 1)
        }
    }

    pub(super) fn set(signal: usize) -> Result<(), Failure> {
        let one = 1u64;
        loop {
            let count = unsafe { write(native_handle(signal), (&one as *const u64).cast(), 8) };
            if count == 8 {
                return Ok(());
            }
            let failure = crate::platform_io::last_failure();
            match failure.system_code {
                Some(4) => continue,
                Some(11) => return Ok(()), // Saturation is already readable.
                _ => return Err(failure),
            }
        }
    }

    #[cfg(target_os = "linux")]
    pub(super) fn reset(signal: usize) -> Result<(), Failure> {
        let mut count = 0u64;
        loop {
            let result =
                unsafe { libc::read(native_handle(signal), (&mut count as *mut u64).cast(), 8) };
            if result == 8 {
                continue;
            }
            let failure = crate::platform_io::last_failure();
            match failure.system_code {
                Some(4) => continue,
                Some(11) => return Ok(()),
                _ => return Err(failure),
            }
        }
    }

    unsafe extern "C" {
        fn eventfd(value: u32, flags: i32) -> i32;
        fn write(descriptor: i32, bytes: *const c_void, length: usize) -> isize;
    }

    #[cfg(test)]
    pub(super) fn wait_for_test(signal: usize, milliseconds: u32) -> Result<bool, Failure> {
        #[repr(C)]
        struct PollFd {
            descriptor: i32,
            events: i16,
            returned: i16,
        }
        unsafe extern "C" {
            fn poll(descriptors: *mut PollFd, count: usize, milliseconds: i32) -> i32;
        }
        let mut descriptor = PollFd {
            descriptor: native_handle(signal),
            events: 1,
            returned: 0,
        };
        let count = unsafe { poll(&mut descriptor, 1, milliseconds as i32) };
        if count < 0 {
            Err(crate::platform_io::last_failure())
        } else {
            Ok(count > 0 && descriptor.returned & 1 != 0)
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod platform {
    use super::*;
    use core::ffi::{c_long, c_void};
    use core::ptr;

    #[repr(C, packed(4))]
    struct Event {
        identity: usize,
        filter: i16,
        flags: u16,
        filter_flags: u32,
        data: isize,
        user_data: *mut c_void,
    }

    #[repr(C)]
    struct Timespec {
        seconds: c_long,
        nanoseconds: c_long,
    }

    fn event(flags: u16, filter_flags: u32) -> Event {
        Event {
            identity: 1,
            filter: -10,
            flags,
            filter_flags,
            data: 0,
            user_data: ptr::null_mut(),
        }
    }

    pub(super) fn create() -> Result<usize, Failure> {
        let descriptor = unsafe { kqueue() };
        if descriptor < 0 {
            return Err(crate::platform_io::last_failure());
        }
        let owner = unsafe { OwnedHandle::from_raw(descriptor) };
        // Darwin kqueues are not inherited by fork; mark close-on-exec as well.
        if unsafe { fcntl(descriptor, 2, 1) } < 0 {
            return Err(crate::platform_io::last_failure());
        }
        let registration = event(0x1 | 0x4, 0); // EV_ADD | EV_ENABLE, not EV_CLEAR.
        if unsafe {
            kevent(
                descriptor,
                &registration,
                1,
                ptr::null_mut(),
                0,
                ptr::null(),
            )
        } < 0
        {
            return Err(crate::platform_io::last_failure());
        }
        core::mem::forget(owner);
        Ok(descriptor as usize + 1)
    }

    pub(super) fn set(signal: usize) -> Result<(), Failure> {
        let trigger = event(0, 0x0100_0000); // NOTE_TRIGGER.
        loop {
            if unsafe {
                kevent(
                    native_handle(signal),
                    &trigger,
                    1,
                    ptr::null_mut(),
                    0,
                    ptr::null(),
                )
            } >= 0
            {
                return Ok(());
            }
            let failure = crate::platform_io::last_failure();
            if failure.system_code != Some(4) {
                return Err(failure);
            }
        }
    }

    #[cfg(target_os = "macos")]
    pub(super) fn reset(signal: usize) -> Result<(), Failure> {
        // Recreate only the user-event registration, not its live kqueue handle.
        let changes = [event(0x2, 0), event(0x1 | 0x4, 0)];
        loop {
            if unsafe {
                kevent(
                    native_handle(signal),
                    changes.as_ptr(),
                    2,
                    ptr::null_mut(),
                    0,
                    ptr::null(),
                )
            } >= 0
            {
                return Ok(());
            }
            let failure = crate::platform_io::last_failure();
            if failure.system_code != Some(4) {
                return Err(failure);
            }
        }
    }

    unsafe extern "C" {
        fn kqueue() -> i32;
        fn fcntl(descriptor: i32, command: i32, ...) -> i32;
        fn kevent(
            descriptor: i32,
            changes: *const Event,
            count: i32,
            events: *mut Event,
            capacity: i32,
            timeout: *const Timespec,
        ) -> i32;
    }

    #[cfg(test)]
    pub(super) fn wait_for_test(signal: usize, milliseconds: u32) -> Result<bool, Failure> {
        let timeout = Timespec {
            seconds: c_long::from(milliseconds / 1_000),
            nanoseconds: c_long::from(milliseconds % 1_000) * 1_000_000,
        };
        let mut output = event(0, 0);
        let count = unsafe {
            kevent(
                native_handle(signal),
                ptr::null(),
                0,
                &mut output,
                1,
                &timeout,
            )
        };
        if count < 0 {
            Err(crate::platform_io::last_failure())
        } else {
            Ok(count > 0 && output.filter == -10 && output.identity == 1)
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use crate::win32::{CreateEventW, SetEvent};
    use core::ptr;

    pub(super) fn create() -> Result<usize, Failure> {
        let handle = unsafe { CreateEventW(ptr::null_mut(), 1, 0, ptr::null()) };
        if handle.is_null() {
            Err(crate::platform_io::last_failure())
        } else {
            Ok(handle as usize)
        }
    }

    pub(super) fn set(signal: usize) -> Result<(), Failure> {
        if unsafe { SetEvent(native_handle(signal)) } == 0 {
            Err(crate::platform_io::last_failure())
        } else {
            Ok(())
        }
    }

    #[cfg(test)]
    pub(super) fn wait_for_test(signal: usize, milliseconds: u32) -> Result<bool, Failure> {
        use crate::win32::WaitForSingleObject;
        match unsafe { WaitForSingleObject(native_handle(signal), milliseconds) } {
            0 => Ok(true),
            258 => Ok(false),
            _ => Err(crate::platform_io::last_failure()),
        }
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "android",
    target_os = "macos",
    target_os = "ios",
    windows
)))]
mod platform {
    use super::*;

    pub(super) fn create() -> Result<usize, Failure> {
        Err(Failure {
            reason: crate::platform_io::Reason::Unsupported,
            system_code: None,
            transferred_byte_count: None,
        })
    }

    pub(super) fn set(_: usize) -> Result<(), Failure> {
        create().map(|_| ())
    }

    #[cfg(test)]
    pub(super) fn wait_for_test(_: usize, _: u32) -> Result<bool, Failure> {
        create().map(|_| false)
    }
}
