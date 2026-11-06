//! Opaque inline transport over the existing raw OwnedFile ABI. This has the
//! same caller-proved storage/loan contract as dr_v1_file_readiness_source.

use core::{ffi::c_void, mem, ptr};

use super::{try_lock, FileLock, LockAttempt, LockMode, OwnedFile};
use crate::io_status::DrIoStatusV1;
use crate::platform_io::Failure;

enum NativeLock {
    // The ABI caller proves the actual retained lifetime, excludes access to
    // the original binding, and keeps this complete owner at a stable address.
    Borrowed(FileLock<&'static mut OwnedFile>),
    Owned(FileLock<OwnedFile>),
}

/// # Safety
/// Outputs are valid disjoint writable usize locations.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_lock_layout(size: *mut usize, alignment: *mut usize) {
    *size = mem::size_of::<NativeLock>();
    *alignment = mem::align_of::<NativeLock>();
}

/// Tries one native whole-file acquisition. Mode: 0 shared, 1 exclusive.
/// Ownership: 0 borrow, 1 own. Returns 0 acquired, 1 contention, 3 failure.
/// Failure/contended acquisition leaves the original owner initialized.
/// # Safety
/// File is a live exclusively accessible raw OwnedFile. Guard storage is
/// uninitialized, properly sized/aligned, and disjoint from file/status. A
/// successful borrowed guard retains the complete file at this address and
/// excludes its independent access/move/close/drop until release or drop.
/// Owned success consumes the file storage. Only success initializes guard.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_try_lock(
    file: *mut c_void,
    mode: u8,
    ownership: u8,
    guard: *mut c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    let mode = match mode {
        0 => LockMode::Shared,
        1 => LockMode::Exclusive,
        _ => return (*status).fail(Failure::invalid_input()),
    };
    if ownership > 1 || file.is_null() || guard.is_null() {
        return (*status).fail(Failure::invalid_input());
    }
    let guard = guard.cast::<NativeLock>();
    let file = file.cast::<OwnedFile>();
    if ownership == 0 {
        match try_lock(&mut *file, mode) {
            Ok(LockAttempt::Acquired(lock)) => {
                guard.write(NativeLock::Borrowed(lock));
                0
            }
            Ok(LockAttempt::WouldBlock(_)) => 1,
            Err(error) => (*status).fail(error.failure),
        }
    } else {
        match try_lock(file.read(), mode) {
            Ok(LockAttempt::Acquired(lock)) => {
                guard.write(NativeLock::Owned(lock));
                0
            }
            Ok(LockAttempt::WouldBlock(owner)) => {
                file.write(owner);
                1
            }
            Err(error) => {
                file.write(error.owner);
                (*status).fail(error.failure)
            }
        }
    }
}

/// Projects the guard's file without creating another owner.
/// # Safety
/// Guard is live and exclusively accessible. The complete guard, and original
/// file for a borrowed guard, stay live and unmoved for the projection's loan.
/// The caller may borrow this file, but never close, move, or drop it directly.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_lock_file(guard: *mut c_void) -> *mut c_void {
    let file = match &mut *guard.cast::<NativeLock>() {
        NativeLock::Borrowed(lock) => lock.file_mut(),
        NativeLock::Owned(lock) => lock.file_mut(),
    };
    (file as *mut OwnedFile).cast()
}

/// Explicit unlock returns 0 success or 3 checked failure. Failure preserves
/// the live guard; success consumes it. Owned success initializes file output;
/// borrowed success returns access to the original binding without closing it.
/// # Safety
/// Guard is live, exclusively owned and unborrowed. Status is writable and
/// disjoint. For an owning guard, file output is properly sized/aligned,
/// uninitialized OwnedFile storage disjoint from guard/status. For a borrowed
/// guard, file output is null. Consumed guard storage must not be dropped.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_lock_release(
    guard: *mut c_void,
    file: *mut c_void,
    status: *mut DrIoStatusV1,
) -> u8 {
    *status = DrIoStatusV1::default();
    if guard.is_null() {
        return (*status).fail(Failure::invalid_input());
    }
    let guard = guard.cast::<NativeLock>();
    let owned = matches!(&*guard, NativeLock::Owned(_));
    if owned == file.is_null() {
        return (*status).fail(Failure::invalid_input());
    }
    match guard.read() {
        NativeLock::Borrowed(lock) => match lock.release() {
            Ok(_) => 0,
            Err(error) => {
                guard.write(NativeLock::Borrowed(error.owner));
                (*status).fail(error.failure)
            }
        },
        NativeLock::Owned(lock) => match lock.release() {
            Ok(owner) => {
                file.cast::<OwnedFile>().write(owner);
                0
            }
            Err(error) => {
                guard.write(NativeLock::Owned(error.owner));
                (*status).fail(error.failure)
            }
        },
    }
}

/// Best-effort nonthrowing release; only an owning guard closes the file.
/// # Safety
/// Guard is live, exclusively owned and unborrowed. Its storage becomes
/// uninitialized and must not be used or destroyed again.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_lock_drop(guard: *mut c_void) {
    ptr::drop_in_place(guard.cast::<NativeLock>());
}

#[cfg(all(test, any(unix, windows)))]
mod tests;
