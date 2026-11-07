//! Decision 0137's narrow inherited-synchronous-device transport. The existing
//! group owns this pinned reusable record and its original selected region.
//! A helper executes only native byte I/O and publishes copyable terminal facts;
//! all adapter/error construction and device policy stay on the driving thread.
use core::{cell::UnsafeCell, pin::Pin, ptr};

use crate::cancellation_signal::{WakeCondition, WakeLock};
use crate::native_handle::NativeHandle;
use crate::platform_io::Failure;
use crate::transfer_group::pending::FileTerminal;
use crate::transfer_group::FixedSlice;

#[cfg(test)]
std::thread_local! {
    static STARTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static START_FAILURE: std::cell::Cell<Option<Failure>> = const { std::cell::Cell::new(None) };
}
#[cfg(test)]
pub(crate) fn starts_for_test() -> usize {
    STARTS.with(std::cell::Cell::get)
}

#[cfg(test)]
pub(crate) fn fail_next_start_for_test(failure: Failure) {
    START_FAILURE.with(|next| next.set(Some(failure)));
}

#[derive(Clone, Copy)]
pub(crate) struct Job {
    pub(crate) handle: NativeHandle,
    pub(crate) data: *mut u8,
    pub(crate) length: usize,
    pub(crate) reading: bool,
    // Native wake transport only, never a Doria adapter or foreign callback.
    pub(crate) notify: unsafe fn(usize),
    pub(crate) notice: usize,
}

struct State {
    job: Option<Job>,
    running: bool,
    cancelled: bool,
    stopping: bool,
    result: Option<FileTerminal>,
}

pub(crate) struct Helper {
    record: Option<FixedSlice<Worker>>,
    thread: Option<platform::Thread>,
}

struct Worker {
    lock: WakeLock,
    condition: WakeCondition,
    state: UnsafeCell<State>,
}

impl Helper {
    pub(crate) const fn new() -> Self {
        Self {
            record: None,
            thread: None,
        }
    }

    fn worker(&self) -> Option<&Worker> {
        self.record.as_ref().map(|record| &record.as_slice()[0])
    }

    /// # Safety
    /// The group's selected region and native notification context stay live
    /// and inaccessible to competing consumers until actual retirement.
    pub(crate) unsafe fn submit(&mut self, job: Job) -> Result<(), Failure> {
        #[cfg(test)]
        crate::io_cost_probe::record(crate::io_cost_probe::Operation::HelperSubmit);
        if self.record.is_none() {
            let mut record = FixedSlice::new(1)?;
            record.push(Worker::new())?;
            self.record = Some(record);
        }
        if self.thread.is_none() {
            #[cfg(test)]
            if let Some(failure) = START_FAILURE.with(std::cell::Cell::take) {
                return Err(failure);
            }
            self.thread = Some(platform::Thread::start(
                ptr::from_ref(self.worker().unwrap()).cast(),
            )?);
            #[cfg(test)]
            STARTS.with(|starts| starts.set(starts.get() + 1));
        }
        self.worker().unwrap().submit(job);
        Ok(())
    }

    pub(crate) fn query(&self) -> Option<FileTerminal> {
        self.worker()?.query()
    }

    pub(crate) fn cancel(&self) -> Result<(), Failure> {
        let Some(worker) = self.worker() else {
            return Ok(());
        };
        let _guard = worker.lock();
        let state = unsafe { &mut *worker.state.get() };
        if state.cancelled {
            return Ok(());
        }
        state.cancelled = true;
        if state.running {
            if let Some(thread) = &self.thread {
                thread.cancel()?;
            }
        }
        worker.condition().wake_all();
        Ok(())
    }

    pub(crate) fn drain(&self) -> Option<FileTerminal> {
        self.worker()?.drain()
    }

    /// Group teardown calls this before releasing its device and storage loans.
    /// No cancellation return or notification substitutes for the native join.
    pub(crate) fn shutdown(&mut self) {
        if self.thread.is_none() {
            return;
        }
        // Normally the shared group has already fanned out cancellation and
        // drained. Also make isolated private-owner destruction safe; cancellation
        // is idempotent and never erases a result or substitutes for retirement.
        let _ = self.cancel();
        let worker = self.worker().unwrap();
        let _ = worker.drain();
        {
            let _guard = worker.lock();
            unsafe {
                (*worker.state.get()).stopping = true;
            }
            worker.condition().wake_all();
        }
        self.thread.take().unwrap().join();
        let worker = self.worker().unwrap();
        let _guard = worker.lock();
        unsafe {
            let state = &mut *worker.state.get();
            state.stopping = false;
            state.cancelled = false;
            state.result = None;
        }
    }

    #[cfg(test)]
    pub(crate) fn started(&self) -> bool {
        self.thread.is_some()
    }
}

impl Worker {
    const fn new() -> Self {
        Self {
            lock: WakeLock::new(),
            condition: WakeCondition::new(),
            state: UnsafeCell::new(State {
                job: None,
                running: false,
                cancelled: false,
                stopping: false,
                result: None,
            }),
        }
    }

    fn lock(&self) -> crate::cancellation_signal::WakeGuard<'_> {
        unsafe { Pin::new_unchecked(&self.lock).lock() }
    }

    fn condition(&self) -> Pin<&WakeCondition> {
        unsafe { Pin::new_unchecked(&self.condition) }
    }

    unsafe fn submit(&self, job: Job) {
        let _guard = self.lock();
        let state = &mut *self.state.get();
        if state.job.is_some() || state.running || state.stopping {
            crate::runtime_invariant_panic();
        }
        state.cancelled = false;
        state.result = None;
        state.job = Some(job);
        self.condition().wake_all();
    }

    pub(crate) fn query(&self) -> Option<FileTerminal> {
        let _guard = self.lock();
        unsafe { (*self.state.get()).result }
    }

    pub(crate) fn drain(&self) -> Option<FileTerminal> {
        let mut guard = self.lock();
        loop {
            let state = unsafe { &*self.state.get() };
            if !state.running && state.job.is_none() {
                return state.result;
            }
            guard.wait(self.condition());
        }
    }

    unsafe fn run(&self) {
        loop {
            let mut guard = self.lock();
            while (*self.state.get()).job.is_none() && !(*self.state.get()).stopping {
                guard.wait(self.condition());
            }
            if (*self.state.get()).stopping {
                return;
            }
            let job = (*self.state.get()).job.take().unwrap();
            let cancelled = (*self.state.get()).cancelled;
            (*self.state.get()).running = true;
            self.condition().wake_all();
            drop(guard);
            let result = if cancelled {
                // No host operation was issued. None is actual retirement with
                // no transfer, not a fabricated Interrupted/aborted I/O error.
                None
            } else {
                Some(native_transfer(job))
            };
            let _guard = self.lock();
            (*self.state.get()).result = result;
            (*self.state.get()).running = false;
            // Publish/wake under the same predicate lock. A driving-thread
            // resubmission cannot reset a notification before this publication.
            (job.notify)(job.notice);
            self.condition().wake_all();
        }
    }
}

unsafe fn native_transfer(job: Job) -> FileTerminal {
    #[cfg(windows)]
    if let Err(failure) = crate::native_handle::require_synchronous_device(job.handle) {
        return if job.reading {
            FileTerminal::Read(Err(failure))
        } else {
            FileTerminal::Write(Err(failure))
        };
    }
    // Controlled Unix work performs one attempt. The driving group owns retry,
    // readiness, and cancellation/deadline checks between attempts.
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        if job.reading {
            FileTerminal::Read(crate::byte_io::read_attempt(
                job.handle, job.data, job.length,
            ))
        } else {
            FileTerminal::Write(crate::byte_io::write_attempt(
                job.handle, job.data, job.length,
            ))
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        if job.reading {
            FileTerminal::Read(crate::byte_io::read(job.handle, job.data, job.length))
        } else {
            FileTerminal::Write(crate::byte_io::write(job.handle, job.data, job.length))
        }
    }
}

#[cfg(test)]
mod tests;

impl Drop for Helper {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    pub(super) struct Thread(libc::pthread_t);
    impl Thread {
        pub(super) unsafe fn start(context: *const core::ffi::c_void) -> Result<Self, Failure> {
            let mut thread = core::mem::MaybeUninit::uninit();
            let code =
                libc::pthread_create(thread.as_mut_ptr(), ptr::null(), run, context.cast_mut());
            if code != 0 {
                Err(Failure::from_system_code(code.into()))
            } else {
                Ok(Self(thread.assume_init()))
            }
        }
        // POSIX thread cancellation could discard progress/retirement evidence.
        // The approved contract permits waiting for the host call to return.
        pub(super) fn cancel(&self) -> Result<(), Failure> {
            Ok(())
        }
        pub(super) fn join(self) {
            if unsafe { libc::pthread_join(self.0, ptr::null_mut()) } != 0 {
                unsafe {
                    crate::runtime_invariant_panic();
                }
            }
        }
    }
    extern "C" fn run(context: *mut core::ffi::c_void) -> *mut core::ffi::c_void {
        unsafe {
            (&*context.cast::<Worker>()).run();
        }
        ptr::null_mut()
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use crate::native_handle::OwnedHandle;
    pub(super) struct Thread(OwnedHandle);
    impl Thread {
        pub(super) unsafe fn start(context: *const core::ffi::c_void) -> Result<Self, Failure> {
            let handle = CreateThread(
                ptr::null_mut(),
                0,
                run,
                context.cast_mut(),
                0,
                ptr::null_mut(),
            );
            if handle.is_null() {
                Err(crate::platform_io::last_failure())
            } else {
                Ok(Self(OwnedHandle::from_raw(handle)))
            }
        }
        pub(super) fn cancel(&self) -> Result<(), Failure> {
            if unsafe { CancelSynchronousIo(self.0.get()) } != 0 {
                return Ok(());
            }
            let failure = crate::platform_io::last_failure();
            // A start/completion race can have no current kernel request. This
            // is NOT retirement proof: drain and join still await the worker.
            if failure.system_code == Some(1168) {
                Ok(())
            } else {
                Err(failure)
            }
        }
        pub(super) fn join(self) {
            if unsafe {
                wait_for_single_object(self.0.get(), INFINITE, "the device helper thread to exit")
            } != 0
            {
                unsafe {
                    crate::runtime_invariant_panic();
                }
            }
        }
    }
    extern "system" fn run(context: *mut core::ffi::c_void) -> u32 {
        unsafe {
            (&*context.cast::<Worker>()).run();
        }
        0
    }
    use crate::win32::{wait_for_single_object, CancelSynchronousIo, CreateThread, INFINITE};
}
