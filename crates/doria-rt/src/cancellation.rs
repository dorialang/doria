use core::mem;
use core::ptr;
use core::sync::atomic::{fence, AtomicBool, AtomicUsize, Ordering};

use crate::cancellation_signal::{self, Signal};
use crate::platform_io::Failure;

#[cfg(all(any(windows, test), any(unix, windows)))]
use crate::cancellation_signal::WakeLock;
#[cfg(all(any(windows, test), any(unix, windows)))]
use core::{cell::UnsafeCell, marker::PhantomData, pin::Pin};

/// Source and token claims share one allocation. The native wake signal is
/// created only when a wait requests it and stays live until the last claim.
#[repr(C)]
pub struct DrCancellationV1 {
    references: AtomicUsize,
    cancelled: AtomicBool,
    signal: AtomicUsize,
    #[cfg(all(any(windows, test), any(unix, windows)))]
    wakes: WakeList,
}

/// An independently retained runtime claim, never a borrowed source loan.
pub(crate) struct CancellationClaim(pub(crate) *mut DrCancellationV1);

impl CancellationClaim {
    /// # Safety
    /// A nonnull state is live for acquisition; the returned claim owns retention.
    pub(crate) unsafe fn acquire(state: *mut DrCancellationV1) -> Result<Option<Self>, Failure> {
        if state.is_null() {
            return Ok(None);
        }
        let retained = dr_v1_cancellation_retain(state);
        if retained.is_null() {
            return Err(Failure {
                reason: crate::platform_io::Reason::ResourceExhausted,
                system_code: None,
                transferred_byte_count: None,
            });
        }
        Ok(Some(Self(retained)))
    }
}

impl Drop for CancellationClaim {
    fn drop(&mut self) {
        unsafe { dr_v1_cancellation_release(self.0) };
    }
}

#[cfg(test)]
pub(crate) unsafe fn owned_claim_count(state: *const DrCancellationV1) -> usize {
    (*state).references.load(Ordering::Acquire)
}

impl DrCancellationV1 {
    fn cancel(&self) -> Result<(), Failure> {
        self.cancelled.store(true, Ordering::SeqCst);
        let signal = self.signal.load(Ordering::SeqCst);
        let mut failure = None;
        if signal != 0 {
            failure = cancellation_signal::set(signal).err();
        }
        #[cfg(all(any(windows, test), any(unix, windows)))]
        {
            // Notification failure must not skip other registered waiters.
            let additional = unsafe { self.wakes.notify() }.err();
            if failure.is_none() {
                failure = additional;
            }
        }
        failure.map_or(Ok(()), Err)
    }

    pub(crate) fn signal(&self) -> Result<usize, Failure> {
        let mut signal = self.signal.load(Ordering::SeqCst);
        if signal == 0 {
            let candidate = Signal::new()?;
            signal = match self.signal.compare_exchange(
                0,
                candidate.get(),
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => candidate.into_raw(),
                Err(existing) => existing,
            };
        }
        // Sequential consistency prevents cancellation and publication from
        // both observing the other's old value and losing the wakeup.
        if self.cancelled.load(Ordering::SeqCst) {
            cancellation_signal::set(signal)?;
        }
        Ok(signal)
    }
}

/// Returns one owned source claim, or null if allocation fails. This does not
/// create a platform event, clock, thread, or executor.
#[no_mangle]
pub extern "C" fn dr_v1_cancellation_new() -> *mut DrCancellationV1 {
    #[cfg(test)]
    crate::io_cost_probe::record(crate::io_cost_probe::Operation::CancellationCreate);
    unsafe {
        let state = crate::allocate(mem::size_of::<DrCancellationV1>()).cast::<DrCancellationV1>();
        if !state.is_null() {
            state.write(DrCancellationV1 {
                references: AtomicUsize::new(1),
                cancelled: AtomicBool::new(false),
                signal: AtomicUsize::new(0),
                #[cfg(all(any(windows, test), any(unix, windows)))]
                wakes: WakeList::new(),
            });
        }
        state
    }
}

/// Constructs the source's one owned shared-state allocation. Allocation failure
/// is the ordinary source-aware class-allocation panic, not a nullable source.
/// No notification handle, worker, clock, or executor is initialized here.
///
/// # Safety
/// `current_frame` is null or live call-only diagnostic storage, never retained.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_cancellation_new(
    current_frame: *const crate::DrStackFrameV2,
) -> *mut DrCancellationV1 {
    let state = dr_v1_cancellation_new();
    if state.is_null() {
        crate::panic_catalogued(current_frame, b"P1601");
    }
    state
}

/// Adds a token/source claim without another allocation. Returns null if the
/// reference count is exhausted; the original claim remains unchanged.
///
/// # Safety
/// `state` must hold a live owned or borrowed claim for the entire call.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_cancellation_retain(
    state: *mut DrCancellationV1,
) -> *mut DrCancellationV1 {
    if (*state)
        .references
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |count| {
            count.checked_add(1).filter(|_| count < isize::MAX as usize)
        })
        .is_ok()
    {
        state
    } else {
        ptr::null_mut()
    }
}

/// Acquires one independently owned Copy-token claim without allocation.
/// An implicit copy cannot return a missing claim or introduce a checked effect;
/// count exhaustion uses the existing source-aware shared-ownership panic.
///
/// # Safety
/// `state` must hold a live owned or borrowed claim for the entire call.
/// `current_frame` is null or current call-only diagnostic storage, never retained.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_cancellation_retain(
    current_frame: *const crate::DrStackFrameV2,
    state: *mut DrCancellationV1,
) -> *mut DrCancellationV1 {
    let retained = dr_v1_cancellation_retain(state);
    if retained.is_null() {
        crate::panic_catalogued(current_frame, b"P1503");
    }
    retained
}

/// Releases a source or token claim without requesting cancellation.
///
/// # Safety
/// `state` must be null or one live owned claim, released exactly once.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_cancellation_release(state: *mut DrCancellationV1) {
    if state.is_null() || (*state).references.fetch_sub(1, Ordering::Release) != 1 {
        return;
    }
    fence(Ordering::Acquire);
    let signal = (*state).signal.load(Ordering::Relaxed);
    if signal != 0 {
        cancellation_signal::close(signal);
    }
    // Native locks are owned resources too. Every wake registration retains a
    // claim, so this is reached only after their synchronized unregistration.
    ptr::drop_in_place(state);
    crate::deallocate(state.cast());
}

/// A private, borrowed wake target; registration never takes target ownership.
///
/// # Safety
/// `wake` must not reenter cancellation registration/request on this state,
/// unwind, wait for the waiter, or allocate per notification. It can be called
/// by another thread under the registration lock. Queued native notifications
/// must not retain a pointer into this borrowed target after unregistration.
#[cfg(all(any(windows, test), any(unix, windows)))]
pub(crate) unsafe trait WakeTarget: Sync {
    fn wake(&self) -> Result<(), Failure>;
}

#[cfg(all(any(windows, test), any(unix, windows)))]
struct WakeNode {
    next: *mut WakeNode,
    target: *const (),
    notify: unsafe fn(*const ()) -> Result<(), Failure>,
}

#[cfg(all(any(windows, test), any(unix, windows)))]
struct WakeList {
    lock: WakeLock,
    head: UnsafeCell<*mut WakeNode>,
}

#[cfg(all(any(windows, test), any(unix, windows)))]
impl WakeList {
    const fn new() -> Self {
        Self {
            lock: WakeLock::new(),
            head: UnsafeCell::new(ptr::null_mut()),
        }
    }

    /// The cancellation state is allocated once and never moved while claimed.
    unsafe fn notify(&self) -> Result<(), Failure> {
        let _guard = Pin::new_unchecked(&self.lock).lock();
        let mut current = *self.head.get();
        let mut failure = None;
        while !current.is_null() {
            let result = ((*current).notify)((*current).target);
            if failure.is_none() {
                failure = result.err();
            }
            current = (*current).next;
        }
        failure.map_or(Ok(()), Err)
    }
}

/// Owns one stable registration node and a shared cancellation-state claim.
/// The target borrow prevents closing/moving its resource until synchronized
/// unregistration. Creating a group may allocate this node; notification never
/// allocates. The guard itself can move without moving the registered node.
#[cfg(all(any(windows, test), any(unix, windows)))]
pub(crate) struct WakeRegistration<'a> {
    state: *mut DrCancellationV1,
    node: *mut WakeNode,
    target: PhantomData<&'a ()>,
}

#[cfg(all(any(windows, test), any(unix, windows)))]
impl<'a> WakeRegistration<'a> {
    /// # Safety
    /// `state` must hold a live claim throughout this call. The returned guard
    /// retains its own claim and never depends on the original source binding.
    pub(crate) unsafe fn register<T: WakeTarget>(
        state: *mut DrCancellationV1,
        target: &'a T,
    ) -> Result<Self, Failure> {
        unsafe fn notify<T: WakeTarget>(target: *const ()) -> Result<(), Failure> {
            (&*target.cast::<T>()).wake()
        }
        if state.is_null() {
            return Err(Failure::invalid_input());
        }
        let claim = dr_v1_cancellation_retain(state);
        if claim.is_null() {
            return Err(wake_allocation_failure());
        }
        let node = crate::allocate(mem::size_of::<WakeNode>()).cast::<WakeNode>();
        if node.is_null() {
            dr_v1_cancellation_release(claim);
            return Err(wake_allocation_failure());
        }
        node.write(WakeNode {
            next: ptr::null_mut(),
            target: (target as *const T).cast(),
            notify: notify::<T>,
        });
        let result = {
            let wakes = &(*state).wakes;
            let _guard = Pin::new_unchecked(&wakes.lock).lock();
            (*node).next = *wakes.head.get();
            *wakes.head.get() = node;
            // Pair publication with the same lock used by cancellation. Either
            // this check sees the request, or its notifier sees our node.
            if (*state).cancelled.load(Ordering::SeqCst) {
                let result = target.wake();
                if result.is_err() {
                    *wakes.head.get() = (*node).next;
                }
                result
            } else {
                Ok(())
            }
        };
        if let Err(failure) = result {
            crate::deallocate(node.cast());
            dr_v1_cancellation_release(claim);
            return Err(failure);
        }
        Ok(Self {
            state: claim,
            node,
            target: PhantomData,
        })
    }
}

#[cfg(all(any(windows, test), any(unix, windows)))]
impl Drop for WakeRegistration<'_> {
    fn drop(&mut self) {
        unsafe {
            {
                let wakes = &(*self.state).wakes;
                let _guard = Pin::new_unchecked(&wakes.lock).lock();
                let mut link = wakes.head.get();
                while *link != self.node {
                    if (*link).is_null() {
                        crate::runtime_invariant_panic();
                    }
                    link = ptr::addr_of_mut!((**link).next);
                }
                *link = (*self.node).next;
            }
            crate::deallocate(self.node.cast());
            dr_v1_cancellation_release(self.state);
        }
    }
}

#[cfg(all(any(windows, test), any(unix, windows)))]
fn wake_allocation_failure() -> Failure {
    Failure {
        reason: crate::platform_io::Reason::ResourceExhausted,
        system_code: None,
        transferred_byte_count: None,
    }
}

#[cfg(all(test, any(unix, windows)))]
#[path = "cancellation/wake_tests.rs"]
mod wake_tests;

/// # Safety
/// `state` must hold a live owned or borrowed claim for the entire call.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_cancellation_is_requested(state: *const DrCancellationV1) -> u8 {
    u8::from((*state).cancelled.load(Ordering::SeqCst))
}

/// Requests cancellation and signals every present or future native waiter.
/// A failed notification keeps the requested state and may be retried.
///
/// # Safety
/// `state` must hold a live claim. All failure outputs must point to writable
/// storage of the declared types and must not alias the state or one another.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_cancellation_request(
    state: *const DrCancellationV1,
    reason: *mut u8,
    system_code: *mut i64,
    has_system_code: *mut u8,
) -> u8 {
    failure_outputs((*state).cancel(), reason, system_code, has_system_code)
}

/// Requests cancellation and returns fields for the ordinary owned
/// CancellationError. Success allocates nothing; failure owns one message
/// string. The caller constructs the existing Error carrier, not an IoError.
/// Notification failure leaves the shared cancellation request published.
///
/// # Safety
/// `state` holds a live claim. Frame is null or live call-only diagnostic storage.
/// Outputs are disjoint writable empty slots, disjoint from the state and frame.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_checked_cancellation_request(
    current_frame: *const crate::DrStackFrameV2,
    state: *const DrCancellationV1,
    message: *mut *mut crate::DrStringV1,
    system_code: *mut i64,
    has_system_code: *mut u8,
) -> u8 {
    *message = ptr::null_mut();
    *system_code = 0;
    *has_system_code = 0;
    match (*state).cancel() {
        Ok(()) => 0,
        Err(failure) => {
            if failure.transferred_byte_count.is_some() {
                crate::runtime_invariant_panic();
            }
            let text = doria_diagnostic_catalogue::CANCELLATION_NOTIFICATION_FAILURE_MESSAGE;
            let owned = crate::allocate_string_with_frame(current_frame, text.len());
            ptr::copy_nonoverlapping(text.as_ptr(), crate::string_bytes_mut(owned), text.len());
            *message = owned;
            if let Some(code) = failure.system_code {
                *system_code = code;
                *has_system_code = 1;
            }
            1
        }
    }
}

/// Returns a borrowed private platform signal for the shared readiness core.
/// Callers retain a state claim for as long as the signal is registered or used;
/// they never close, drain, or reset the returned signal.
///
/// # Safety
/// `state` must hold a live claim. Outputs must point to disjoint writable
/// storage of the declared types and must not alias the state.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_cancellation_signal(
    state: *const DrCancellationV1,
    signal: *mut usize,
    reason: *mut u8,
    system_code: *mut i64,
    has_system_code: *mut u8,
) -> u8 {
    *signal = 0;
    let result = (*state).signal().map(|handle| *signal = handle);
    failure_outputs(result, reason, system_code, has_system_code)
}

unsafe fn failure_outputs(
    result: Result<(), Failure>,
    reason: *mut u8,
    system_code: *mut i64,
    has_system_code: *mut u8,
) -> u8 {
    *reason = 0;
    *system_code = 0;
    *has_system_code = 0;
    match result {
        Ok(()) => 0,
        Err(failure) => {
            *reason = failure.reason as u8;
            if let Some(code) = failure.system_code {
                *system_code = code;
                *has_system_code = 1;
            }
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Claim(*mut DrCancellationV1);

    // Claims keep the allocation and lazy signal alive; all shared state is atomic.
    unsafe impl Send for Claim {}
    unsafe impl Sync for Claim {}

    impl Claim {
        fn new() -> Self {
            let state = dr_v1_cancellation_new();
            assert!(!state.is_null());
            Self(state)
        }

        fn cancelled(&self) -> bool {
            unsafe { dr_v1_cancellation_is_requested(self.0) != 0 }
        }

        fn signal(&self) -> usize {
            unsafe { (*self.0).signal().unwrap() }
        }

        fn cancel(&self) {
            unsafe { (*self.0).cancel().unwrap() }
        }
    }

    impl Clone for Claim {
        fn clone(&self) -> Self {
            let state = unsafe { dr_v1_cancellation_retain(self.0) };
            assert_eq!(state, self.0);
            Self(state)
        }
    }

    impl Drop for Claim {
        fn drop(&mut self) {
            unsafe { dr_v1_cancellation_release(self.0) }
        }
    }

    #[test]
    fn source_factory_allocates_one_claim_and_successful_request_allocates_nothing() {
        let (source, allocations) = crate::probe_runtime_allocations(|| {
            Claim(unsafe { dr_v2_cancellation_new(ptr::null()) })
        });
        assert_eq!(allocations, 1);
        assert!(!source.cancelled());
        let mut message = ptr::null_mut();
        let mut code = 91;
        let mut has_code = 1;
        let (status, allocations) = crate::probe_runtime_allocations(|| unsafe {
            dr_v2_checked_cancellation_request(
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
        assert!(source.cancelled());
        unsafe {
            assert_eq!((*source.0).references.load(Ordering::Relaxed), 1);
            assert_eq!((*source.0).signal.load(Ordering::Relaxed), 0);
        }
    }

    #[test]
    fn allocation_failure_probe_is_scoped_and_fails_only_one_allocation() {
        crate::with_runtime_allocation_failure(1, || {
            let first = Claim::new();
            assert!(dr_v1_cancellation_new().is_null());
            let third = Claim::new();
            assert!(!first.cancelled());
            assert!(!third.cancelled());
        });
        crate::with_runtime_allocation_failure(0, || {});
        let source = Claim::new();
        assert!(!source.cancelled());
        let panic = crate::test_report::expect_panic(|| {
            crate::with_runtime_allocation_failure(0, || panic!("test probe reset"));
        });
        assert!(panic.is_err());
        let source = Claim::new();
        assert!(!source.cancelled());
    }

    #[test]
    fn source_allocation_failure_reports_the_authored_constructor_site() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "cancellation::tests::source_allocation_failure_panic_child",
                "--nocapture",
            ])
            .env("DORIA_RT_TEST_CANCELLATION_SOURCE_OOM", "1")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(101));
        let stderr = String::from_utf8(output.stderr).unwrap();
        for expected in [
            "P1601",
            "Class Allocation Failed",
            "cancellation-source.doria",
            "new CancellationSource()",
        ] {
            assert!(stderr.contains(expected), "{stderr}");
        }
    }

    #[test]
    fn source_allocation_failure_panic_child() {
        if std::env::var("DORIA_RT_TEST_CANCELLATION_SOURCE_OOM").as_deref() != Ok("1") {
            return;
        }
        let source = b"function main(): void { let $source = new CancellationSource(); }\n";
        let path = b"cancellation-source.doria";
        let name = b"main";
        let expression = b"new CancellationSource()";
        let start = source
            .windows(expression.len())
            .position(|bytes| bytes == expression)
            .unwrap();
        let frame = crate::DrStackFrameV2 {
            parent: ptr::null(),
            function_name: name.as_ptr(),
            function_name_length: name.len(),
            source_path: path.as_ptr(),
            source_path_length: path.len(),
            source_text: source.as_ptr(),
            source_text_length: source.len(),
            function_span_start: 0,
            function_span_end: source.len() - 1,
            active_span_start: start,
            active_span_end: start + expression.len(),
        };
        crate::with_runtime_allocation_failure(0, || unsafe {
            dr_v2_cancellation_new(&frame);
        });
        unreachable!("a failed source allocation cannot return an owned source");
    }

    #[test]
    fn tokens_share_one_allocation_and_outlive_the_source_without_implicit_cancellation() {
        let source = Claim::new();
        let token = source.clone();
        let other = token.clone();
        assert_eq!(source.0, token.0);
        assert_eq!(token.0, other.0);
        unsafe {
            assert_eq!((*source.0).references.load(Ordering::Relaxed), 3);
            assert_eq!((*source.0).signal.load(Ordering::Relaxed), 0);
        }
        drop(source);
        assert!(!token.cancelled());
        assert!(!other.cancelled());
        token.cancel();
        drop(token);
        assert!(other.cancelled());
        unsafe {
            assert_eq!((*other.0).references.load(Ordering::Relaxed), 1);
            assert_eq!((*other.0).signal.load(Ordering::Relaxed), 0);
        }
    }

    #[test]
    fn an_exhausted_reference_count_is_rejected_without_wrapping() {
        let source = Claim::new();
        unsafe {
            let maximum = isize::MAX as usize;
            (*source.0).references.store(maximum - 1, Ordering::Relaxed);
            assert_eq!(dr_v1_cancellation_retain(source.0), source.0);
            assert_eq!((*source.0).references.load(Ordering::Relaxed), maximum);
            for count in [maximum, maximum + 1, usize::MAX] {
                (*source.0).references.store(count, Ordering::Relaxed);
                assert!(dr_v1_cancellation_retain(source.0).is_null());
                assert_eq!((*source.0).references.load(Ordering::Relaxed), count);
                assert!(!source.cancelled());
                assert_eq!((*source.0).signal.load(Ordering::Relaxed), 0);
            }
            (*source.0).references.store(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn copy_token_transport_retains_each_owned_claim_without_allocation_or_signal() {
        let (source, allocations) = crate::probe_runtime_allocations(Claim::new);
        assert_eq!(allocations, 1);
        unsafe {
            let ((first, second), allocations) = crate::probe_runtime_allocations(|| {
                (
                    dr_v2_cancellation_retain(ptr::null(), source.0),
                    dr_v2_cancellation_retain(ptr::null(), source.0),
                )
            });
            assert_eq!(allocations, 0);
            assert_eq!((first, second), (source.0, source.0));
            let (first, second) = (Claim(first), Claim(second));
            assert_eq!((*first.0).references.load(Ordering::Relaxed), 3);
            drop(source);
            let ((), allocations) = crate::probe_runtime_allocations(|| {
                for _ in 0..128 {
                    assert_eq!(dr_v1_cancellation_is_requested(first.0), 0);
                }
            });
            assert_eq!(allocations, 0);
            assert_eq!((*first.0).references.load(Ordering::Relaxed), 2);
            drop(first);
            assert_eq!((*second.0).references.load(Ordering::Relaxed), 1);
            assert_eq!((*second.0).signal.load(Ordering::Relaxed), 0);
            assert!(!second.cancelled());
            let ((), allocations) = crate::probe_runtime_allocations(|| drop(second));
            assert_eq!(allocations, 0);
        }
    }

    #[test]
    fn copy_token_count_exhaustion_reports_the_authored_copy_site() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "cancellation::tests::copy_token_count_exhaustion_panic_child",
                "--nocapture",
            ])
            .env("DORIA_RT_TEST_CANCELLATION_COPY_OVERFLOW", "1")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(101));
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.contains("P1503"), "{stderr}");
        assert!(
            stderr.contains("Shared Ownership Count Overflowed"),
            "{stderr}"
        );
        assert!(stderr.contains("cancellation-copy.doria"), "{stderr}");
        assert!(stderr.contains("let $second = $token;"), "{stderr}");
    }

    #[test]
    fn copy_token_count_exhaustion_panic_child() {
        if std::env::var("DORIA_RT_TEST_CANCELLATION_COPY_OVERFLOW").as_deref() != Ok("1") {
            return;
        }
        let source = b"use Doria\\Std\\Concurrency\\CancellationToken;\nfunction duplicate(CancellationToken $token): CancellationToken { let $second = $token; return $second; }\n";
        let path = b"cancellation-copy.doria";
        let name = b"duplicate";
        let expression = b"let $second = $token;";
        let start = source
            .windows(expression.len())
            .position(|bytes| bytes == expression)
            .unwrap();
        let frame = crate::DrStackFrameV2 {
            parent: ptr::null(),
            function_name: name.as_ptr(),
            function_name_length: name.len(),
            source_path: path.as_ptr(),
            source_path_length: path.len(),
            source_text: source.as_ptr(),
            source_text_length: source.len(),
            function_span_start: source.iter().position(|byte| *byte == b'\n').unwrap() + 1,
            function_span_end: source.len() - 1,
            active_span_start: start,
            active_span_end: start + expression.len(),
        };
        let source = Claim::new();
        unsafe {
            (*source.0)
                .references
                .store(isize::MAX as usize, Ordering::Relaxed);
            dr_v2_cancellation_retain(&frame, source.0);
        }
        unreachable!("an implicit Copy cannot return without its owned claim");
    }

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn cancellation_before_signal_creation_is_persistent_and_reuses_the_signal() {
        let source = Claim::new();
        source.cancel();
        let token = source.clone();
        drop(source);
        let signal = token.signal();
        for _ in 0..8 {
            assert_eq!(token.signal(), signal);
            assert!(cancellation_signal::wait_for_test(signal, 0).unwrap());
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn cancellation_wakes_multiple_waiters_without_draining_the_signal() {
        use std::sync::Barrier;

        let source = Claim::new();
        let barrier = Barrier::new(5);
        let signal = source.signal();
        assert!(!cancellation_signal::wait_for_test(signal, 0).unwrap());
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let token = source.clone();
                let barrier = &barrier;
                scope.spawn(move || {
                    assert_eq!(token.signal(), signal);
                    barrier.wait();
                    assert!(cancellation_signal::wait_for_test(signal, 5_000).unwrap());
                    assert!(token.cancelled());
                    assert!(cancellation_signal::wait_for_test(signal, 0).unwrap());
                });
            }
            barrier.wait();
            source.cancel();
        });
    }

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn racing_signal_creation_and_cancellation_never_loses_the_wakeup() {
        use std::sync::Barrier;

        for _ in 0..32 {
            let source = Claim::new();
            let barrier = Barrier::new(4);
            std::thread::scope(|scope| {
                for _ in 0..3 {
                    let token = source.clone();
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        let signal = token.signal();
                        assert!(cancellation_signal::wait_for_test(signal, 5_000).unwrap());
                        assert!(token.cancelled());
                    });
                }
                barrier.wait();
                source.cancel();
            });
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn successful_private_abi_calls_clear_stale_failure_outputs() {
        let source = Claim::new();
        let (mut signal, mut reason, mut code, mut has_code) = (999, 255, -1, 1);
        unsafe {
            assert_eq!(
                dr_v1_cancellation_signal(
                    source.0,
                    &mut signal,
                    &mut reason,
                    &mut code,
                    &mut has_code
                ),
                0
            );
            assert_ne!(signal, 0);
            assert_eq!((reason, code, has_code), (0, 0, 0));
            assert_eq!(
                dr_v1_cancellation_request(source.0, &mut reason, &mut code, &mut has_code),
                0
            );
            assert_eq!((reason, code, has_code), (0, 0, 0));
        }
    }
}
