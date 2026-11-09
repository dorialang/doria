//! Lock guards retain the canonical file resource and original class payload.
//! They never move its native owner out of its readiness/path identity home.

use core::{ffi::c_void, mem, ptr};

use crate::file_io::stream::locking::{try_lock, FileLock, FileOwner, LockAttempt, LockMode};
use crate::DrStringV1;

use super::{dr_v1_file_object_drop, DrFileStatusV1, Failure, FileObject, OwnedFile, ResourcePin};

struct FilePayload {
    payload: *mut c_void,
    file: *mut OwnedFile,
    path: *mut DrStringV1,
    pin: Option<ResourcePin>,
    owned: bool,
}

impl FilePayload {
    unsafe fn new(payload: *mut c_void, owned: bool) -> Result<Self, Failure> {
        if payload.is_null() {
            return Err(Failure::invalid_input());
        }
        let resource = (*payload.cast::<FileObject>()).resource;
        let pin = ResourcePin::selected(resource)?;
        Ok(Self {
            payload,
            file: (*resource).file()?,
            path: (*resource).path(),
            pin: Some(pin),
            owned,
        })
    }

    fn into_file(mut self) -> (*mut c_void, bool) {
        // Unlocking has already completed (or acquisition never succeeded).
        // End the active resource pin before returning/releasing ownership.
        drop(self.pin.take());
        let owned = mem::replace(&mut self.owned, false);
        (self.payload, owned)
    }
}

impl FileOwner for FilePayload {
    fn file(&self) -> &OwnedFile {
        // The active resource pin retains this exact owner, independently of
        // the caller's wrapper address. The ABI caller supplies access loans.
        unsafe { &*self.file }
    }

    fn file_mut(&mut self) -> &mut OwnedFile {
        unsafe { &mut *self.file }
    }
}

impl Drop for FilePayload {
    fn drop(&mut self) {
        // FileLock drops its native lock first. Resource retirement must then
        // precede consuming file-field cleanup. The caller owns backing
        // storage, just as for ordinary file_object_drop (native or hosted).
        drop(self.pin.take());
        if self.owned {
            unsafe {
                dr_v1_file_object_drop(ptr::null(), self.payload);
            }
        }
    }
}

type ObjectLock = FileLock<FilePayload>;

/// Reports opaque inline guard storage over the existing class payload.
/// # Safety
/// Outputs are valid disjoint writable usize locations.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_object_lock_layout(size: *mut usize, alignment: *mut usize) {
    *size = mem::size_of::<ObjectLock>();
    *alignment = mem::align_of::<ObjectLock>();
}

/// Mode: 0 shared, 1 exclusive. Ownership: 0 borrow, 1 own. Returns 0 acquired,
/// 1 contention, 3 failure. Only success initializes guard and transfers/loans
/// the original file value. Neither failure nor contention consumes the file.
/// # Safety
/// File is a live initialized FileObject payload with exclusive access and no
/// outstanding file/region/operation loans. Guard is uninitialized storage of
/// the reported size/alignment, disjoint from file/status. Successful borrow
/// retains the complete original payload at this address and excludes its
/// independent move, access, close and destruction until guard release/drop.
/// Successful ownership transfer gives the guard the existing payload/close
/// obligation; the original source value must not be accessed or destroyed.
/// The caller keeps the original payload storage alive and unmoved throughout
/// either guard form; native/hosted storage disposal stays caller-owned.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_object_try_lock(
    file: *mut c_void,
    mode: u8,
    ownership: u8,
    guard: *mut c_void,
    status: *mut DrFileStatusV1,
) -> u8 {
    *status = DrFileStatusV1::default();
    let mode = match mode {
        0 => LockMode::Shared,
        1 => LockMode::Exclusive,
        _ => return (*status).fail(Failure::invalid_input()),
    };
    if ownership > 1 || guard.is_null() {
        return (*status).fail(Failure::invalid_input());
    }
    let owner = match FilePayload::new(file, ownership != 0) {
        Ok(owner) => owner,
        Err(failure) => return (*status).fail(failure),
    };
    match try_lock(owner, mode) {
        Ok(LockAttempt::Acquired(lock)) => {
            guard.cast::<ObjectLock>().write(lock);
            0
        }
        Ok(LockAttempt::WouldBlock(owner)) => {
            owner.into_file();
            1
        }
        Err(error) => {
            error.owner.into_file();
            (*status).fail(error.failure)
        }
    }
}

/// Projects the original complete FileObject, including path/readiness identity.
/// # Safety
/// Guard is live and exclusively accessible; its complete original payload is
/// retained, open and unmoved. The result is borrowed for at most this guard's
/// lifetime; it must not be independently moved, closed, consumed or destroyed.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_object_lock_file(guard: *mut c_void) -> *mut c_void {
    (*guard.cast::<ObjectLock>()).owner().payload
}

/// Borrows the opening path without allocating or acquiring a string claim.
/// # Safety
/// Guard is live under a shared loan. Result is borrowed and cannot outlive the
/// guard unless the caller explicitly retains it, e.g. for checked IoError.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_object_lock_path(guard: *const c_void) -> *mut DrStringV1 {
    (*guard.cast::<ObjectLock>()).owner().path
}

/// Explicit unlock returns 0 success or 3 failure. Failure retains the live
/// guard, resource pin and path. Success consumes guard; an owning guard returns
/// its original FileObject pointer, while a borrowed guard returns null and
/// restores access to the original source binding without closing the file.
/// # Safety
/// Guard is live, exclusively owned and unborrowed. Outputs are valid mutually
/// disjoint writable storage, disjoint from guard and its retained file. File
/// output contains no earlier ownership claim. Consumed guard is uninitialized.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_object_lock_release(
    guard: *mut c_void,
    file: *mut *mut c_void,
    status: *mut DrFileStatusV1,
) -> u8 {
    *status = DrFileStatusV1::default();
    *file = ptr::null_mut();
    if guard.is_null() {
        return (*status).fail(Failure::invalid_input());
    }
    let guard = guard.cast::<ObjectLock>();
    match guard.read().release() {
        Ok(owner) => {
            let (payload, owned) = owner.into_file();
            if owned {
                *file = payload;
            }
            0
        }
        Err(error) => {
            guard.write(error.owner);
            (*status).fail(error.failure)
        }
    }
}

/// Nonthrowing cleanup unlocks first, then destroys only an owned file's fields.
/// Returns that destroyed payload for caller-owned storage disposal, or null
/// for a borrowed file. Never calls class free on native/hosted/inline storage.
/// # Safety
/// Guard is initialized, exclusively owned and unborrowed. Its storage becomes
/// uninitialized and must not be used or destroyed a second time. The caller
/// disposes returned storage with its original allocator, without dropping the
/// already-destroyed file fields again. Borrowed file storage stays initialized.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_object_lock_drop(guard: *mut c_void) -> *mut c_void {
    let owner = (*guard.cast::<ObjectLock>()).owner();
    let destroyed = if owner.owned {
        owner.payload
    } else {
        ptr::null_mut()
    };
    ptr::drop_in_place(guard.cast::<ObjectLock>());
    destroyed
}
