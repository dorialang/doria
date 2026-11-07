//! Native whole-file lock ownership, shared by explicit borrowed/owning guards.
//! Public Doria declarations and checked-error transport remain separate.

use crate::native_handle::NativeHandle;
use crate::platform_io::Failure;

use super::OwnedFile;

mod transport;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LockMode {
    Shared,
    Exclusive,
}

pub(crate) trait FileOwner {
    fn file(&self) -> &OwnedFile;
    fn file_mut(&mut self) -> &mut OwnedFile;
}

impl FileOwner for OwnedFile {
    fn file(&self) -> &OwnedFile {
        self
    }

    fn file_mut(&mut self) -> &mut OwnedFile {
        self
    }
}

impl FileOwner for &mut OwnedFile {
    fn file(&self) -> &OwnedFile {
        self
    }

    fn file_mut(&mut self) -> &mut OwnedFile {
        self
    }
}

pub(crate) enum LockAttempt<F: FileOwner> {
    Acquired(FileLock<F>),
    WouldBlock(F),
}

pub(crate) struct LockFailure<F> {
    pub(crate) owner: F,
    pub(crate) failure: Failure,
}

pub(crate) struct FileLock<F: FileOwner> {
    // Taking the owner after successful explicit release disarms Drop. The
    // borrowed form releases only a loan; the owning form later closes once.
    owner: Option<F>,
}

pub(crate) fn try_lock<F: FileOwner>(
    owner: F,
    mode: LockMode,
) -> Result<LockAttempt<F>, LockFailure<F>> {
    match platform_try_lock(owner.file().native_handle(), mode) {
        Ok(true) => Ok(LockAttempt::Acquired(FileLock { owner: Some(owner) })),
        Ok(false) => Ok(LockAttempt::WouldBlock(owner)),
        Err(failure) => Err(LockFailure { owner, failure }),
    }
}

impl<F: FileOwner> FileLock<F> {
    pub(crate) fn owner(&self) -> &F {
        self.owner.as_ref().unwrap()
    }

    pub(crate) fn file_mut(&mut self) -> &mut OwnedFile {
        self.owner.as_mut().unwrap().file_mut()
    }

    /// Preserve the guard and its owner if explicit unlocking fails. The
    /// caller can inspect the failure and choose retry or best-effort cleanup.
    pub(crate) fn release(mut self) -> Result<F, LockFailure<Self>> {
        if let Err(failure) = platform_unlock(self.owner.as_ref().unwrap().file().native_handle()) {
            return Err(LockFailure {
                owner: self,
                failure,
            });
        }
        Ok(self.owner.take().unwrap())
    }
}

impl<F: FileOwner> Drop for FileLock<F> {
    fn drop(&mut self) {
        if let Some(owner) = &self.owner {
            let _ = platform_unlock(owner.file().native_handle());
        }
    }
}

/// The lock state a Doria `FileLock`/`LockedFile` guard keeps on its file.
/// One owner holds at most one whole-file lock; the guard that acquired it is
/// the only path that releases it.
impl OwnedFile {
    /// Acquires the whole-file lock, waiting for it when `wait` is set.
    /// Returns false only for contention without waiting. A second
    /// acquisition through the same owner is InvalidInput, never a silent
    /// conversion of the held mode.
    pub(crate) fn acquire_lock(&mut self, mode: LockMode, wait: bool) -> Result<bool, Failure> {
        if self.lock.is_some() {
            return Err(Failure::invalid_input());
        }
        let handle = self.native_handle();
        let acquired = if wait {
            platform_lock(handle, mode).map(|()| true)?
        } else {
            platform_try_lock(handle, mode)?
        };
        if acquired {
            self.lock = Some(mode);
        }
        Ok(acquired)
    }

    /// Explicit release. On failure the lock stays recorded, so a later
    /// best-effort release still tries again.
    pub(crate) fn release_lock(&mut self) -> Result<(), Failure> {
        if self.lock.is_none() {
            return Err(Failure::invalid_input());
        }
        platform_unlock(self.native_handle())?;
        self.lock = None;
        Ok(())
    }

    /// Best-effort release for guard destruction; nothing to do when the
    /// guard already released explicitly.
    pub(crate) fn abandon_lock(&mut self) {
        if self.lock.take().is_some() {
            let _ = platform_unlock(self.native_handle());
        }
    }
}

#[cfg(unix)]
fn platform_lock(handle: NativeHandle, mode: LockMode) -> Result<(), Failure> {
    let operation = match mode {
        LockMode::Shared => libc::LOCK_SH,
        LockMode::Exclusive => libc::LOCK_EX,
    };
    loop {
        if unsafe { libc::flock(handle, operation) } == 0 {
            return Ok(());
        }
        let failure = crate::platform_io::last_failure();
        // A signal interrupts the wait, not the request; keep waiting.
        if failure.system_code != Some(i64::from(libc::EINTR)) {
            return Err(failure);
        }
    }
}

#[cfg(unix)]
fn platform_try_lock(handle: NativeHandle, mode: LockMode) -> Result<bool, Failure> {
    // flock locks the open-file description, not the process's whole fd
    // table. Closing a separately opened file must not release this lock.
    let operation = match mode {
        LockMode::Shared => libc::LOCK_SH,
        LockMode::Exclusive => libc::LOCK_EX,
    } | libc::LOCK_NB;
    if unsafe { libc::flock(handle, operation) } == 0 {
        return Ok(true);
    }
    let failure = crate::platform_io::last_failure();
    if failure.system_code == Some(i64::from(libc::EWOULDBLOCK)) {
        Ok(false)
    } else {
        Err(failure)
    }
}

#[cfg(unix)]
fn platform_unlock(handle: NativeHandle) -> Result<(), Failure> {
    if unsafe { libc::flock(handle, libc::LOCK_UN) } == 0 {
        Ok(())
    } else {
        Err(crate::platform_io::last_failure())
    }
}

// Every lock covers the whole file: a request at offset zero for the maximum
// length.
#[cfg(windows)]
use crate::win32::Overlapped;

#[cfg(windows)]
fn platform_try_lock(handle: NativeHandle, mode: LockMode) -> Result<bool, Failure> {
    use crate::win32::LockFileEx;
    const FAIL_IMMEDIATELY: u32 = 1;
    const EXCLUSIVE_LOCK: u32 = 2;
    const LOCK_VIOLATION: i64 = 33;
    let flags = FAIL_IMMEDIATELY
        | match mode {
            LockMode::Shared => 0,
            LockMode::Exclusive => EXCLUSIVE_LOCK,
        };
    let mut request = Overlapped::new(core::ptr::null_mut());
    // FAIL_IMMEDIATELY keeps this request synchronous even on an overlapped
    // handle, so the kernel retains no pointer to this stack request.
    // https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-lockfileex
    if unsafe { LockFileEx(handle, flags, 0, u32::MAX, u32::MAX, &mut request) } != 0 {
        return Ok(true);
    }
    let failure = crate::platform_io::last_failure();
    if failure.system_code == Some(LOCK_VIOLATION) {
        Ok(false)
    } else {
        Err(failure)
    }
}

#[cfg(windows)]
fn platform_lock(handle: NativeHandle, mode: LockMode) -> Result<(), Failure> {
    use crate::win32::{
        CloseHandle, CreateEventW, GetOverlappedResult, LockFileEx, WaitForSingleObject,
    };
    const EXCLUSIVE_LOCK: u32 = 2;
    const IO_PENDING: i64 = 997;
    const INFINITE: u32 = u32::MAX;
    let flags = match mode {
        LockMode::Shared => 0,
        LockMode::Exclusive => EXCLUSIVE_LOCK,
    };
    let event = unsafe { CreateEventW(core::ptr::null(), 1, 0, core::ptr::null()) };
    if event.is_null() {
        return Err(crate::platform_io::last_failure());
    }
    // An overlapped handle may complete the wait asynchronously. The event's
    // low bit keeps that completion off any I/O completion port the handle is
    // associated with; this function waits for it before the request leaves
    // the stack.
    // https://learn.microsoft.com/en-us/windows/win32/api/ioapiset/nf-ioapiset-getqueuedcompletionstatus
    let mut request = Overlapped::new((event as usize | 1) as NativeHandle);
    let result = if unsafe { LockFileEx(handle, flags, 0, u32::MAX, u32::MAX, &mut request) } != 0 {
        Ok(())
    } else {
        let failure = crate::platform_io::last_failure();
        if failure.system_code == Some(IO_PENDING) {
            let mut transferred = 0;
            unsafe { WaitForSingleObject(event, INFINITE) };
            let pending = core::ptr::addr_of_mut!(request);
            if unsafe { GetOverlappedResult(handle, pending, &mut transferred, 0) } != 0 {
                Ok(())
            } else {
                Err(crate::platform_io::last_failure())
            }
        } else {
            Err(failure)
        }
    };
    unsafe { CloseHandle(event) };
    result
}

#[cfg(windows)]
fn platform_unlock(handle: NativeHandle) -> Result<(), Failure> {
    use crate::win32::UnlockFileEx;
    let mut request = Overlapped::new(core::ptr::null_mut());
    if unsafe { UnlockFileEx(handle, 0, u32::MAX, u32::MAX, &mut request) } != 0 {
        Ok(())
    } else {
        Err(crate::platform_io::last_failure())
    }
}

#[cfg(not(any(unix, windows)))]
fn platform_try_lock(_: NativeHandle, _: LockMode) -> Result<bool, Failure> {
    Err(Failure::unsupported())
}

#[cfg(not(any(unix, windows)))]
fn platform_lock(_: NativeHandle, _: LockMode) -> Result<(), Failure> {
    Err(Failure::unsupported())
}

#[cfg(not(any(unix, windows)))]
fn platform_unlock(_: NativeHandle) -> Result<(), Failure> {
    Err(Failure::unsupported())
}

#[cfg(all(test, any(unix, windows)))]
mod tests;
