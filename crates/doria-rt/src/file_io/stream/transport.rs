//! Private native transport. This is not public Doria surface or a second
//! diagnostic model: failures preserve the existing platform_io::Failure facts.
use core::{ffi::c_void, mem, ptr};

pub use crate::io_status::DrIoStatusV1 as DrFileStatusV1;
use crate::platform_io::Failure;

use super::locking::LockMode;
use super::{FileAccess, FileCreation, FileOpenRequest, FileSharing, OwnedFile, SeekOrigin};

#[path = "object.rs"]
pub(crate) mod object;

/// Reports opaque inline-owner storage requirements, without allocating.
/// # Safety
/// Both output pointers must be valid and disjoint writable usize locations.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_layout(size: *mut usize, alignment: *mut usize) {
    *size = mem::size_of::<OwnedFile>();
    *alignment = mem::align_of::<OwnedFile>();
}

/// Forms the private raw-owner fixture loan, NOT the compiler-owned typed
/// FileObject readiness source. This does not associate, allocate, transfer bytes, retain the
/// owner, or create another close obligation. Registration is a separate call.
/// # Safety
/// File points to a live initialized OwnedFile. The caller keeps that complete
/// owner at this address and excludes its move, close and destruction for the
/// loan's lifetime. The result is opaque, not a native handle or owned value.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_readiness_source(file: *const c_void) -> *const c_void {
    (*file.cast::<OwnedFile>()).readiness_source().value()
}

/// Returns the exact pointer-sized private raw-source home embedded in the owner.
/// Loading one pointer-sized word from this home yields readiness_source's
/// value. Borrow forwarding/captures retain this address, never a getter-local
/// cell. Formation performs no mutation, allocation, association or I/O.
/// # Safety
/// The live complete owner remains at this address, unclosed and undestroyed,
/// throughout the resulting loan. The home is borrowed readonly and opaque.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_readiness_source_home(file: *const c_void) -> *const c_void {
    ((*file.cast::<OwnedFile>()).readiness_source() as *const crate::native_handle::ReadinessSource)
        .cast()
}

/// Opens into uninitialized inline owner storage. Access tags: 0 read, 1 write,
/// 2 read/write; creation tags: 0 existing, 1 open/create, 2 create-new,
/// 3 truncate-existing, 4 create/truncate. Append is 0 or 1.
/// Returns 0 success, 3 checked failure, 4 allocation failure (not an IoError).
/// Failure leaves owner storage uninitialized.
/// # Safety
/// Path describes live bytes (null only for length zero). Owner storage must
/// satisfy dr_v1_file_layout, be uninitialized, and be disjoint from all inputs
/// and the writable status. Only success creates a close obligation.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_open(
    path: *const u8,
    length: usize,
    access: u8,
    creation: u8,
    append: u8,
    file: *mut c_void,
    status: *mut DrFileStatusV1,
) -> u8 {
    *status = DrFileStatusV1::default();
    let access = match access {
        0 => FileAccess::Read,
        1 => FileAccess::Write,
        2 => FileAccess::ReadWrite,
        _ => return (*status).fail(Failure::invalid_input()),
    };
    let creation = match creation {
        0 => FileCreation::OpenExisting,
        1 => FileCreation::OpenOrCreate,
        2 => FileCreation::CreateNew,
        3 => FileCreation::TruncateExisting,
        4 => FileCreation::CreateOrTruncate,
        _ => return (*status).fail(Failure::invalid_input()),
    };
    if append > 1 {
        return (*status).fail(Failure::invalid_input());
    }
    let path = if length == 0 {
        &[]
    } else {
        core::slice::from_raw_parts(path, length)
    };
    match OwnedFile::open(
        path,
        FileOpenRequest {
            access,
            creation,
            append: append != 0,
            sharing: FileSharing::ReadWrite,
        },
    ) {
        Ok(owner) => {
            file.cast::<OwnedFile>().write(owner);
            0
        }
        Err(super::super::FileError::Allocation) => 4,
        Err(super::super::FileError::PathNul) => (*status).fail(Failure::invalid_input()),
        Err(
            super::super::FileError::Open(failure)
            | super::super::FileError::Read(failure)
            | super::super::FileError::Write(failure),
        ) => (*status).fail(failure),
    }
}

/// Borrows writable caller storage for this synchronous primitive read.
/// Returns 0 data, 1 would-block, 2 EOF, 3 checked failure; value is data count.
/// # Safety
/// File must be live and exclusively borrowed. Destination must be writable
/// for capacity bytes (null is permitted for zero capacity), disjoint from
/// file/status, and remain borrowed until this call returns.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_read(
    file: *mut c_void,
    destination: *mut u8,
    capacity: usize,
    status: *mut DrFileStatusV1,
) -> u8 {
    *status = DrFileStatusV1::default();
    let destination = if capacity == 0 {
        &mut []
    } else {
        core::slice::from_raw_parts_mut(destination, capacity)
    };
    (*status).read((*file.cast::<OwnedFile>()).read(destination))
}

/// Borrows readonly caller storage for this synchronous primitive write.
/// Returns 0 accepted, 1 would-block, 3 checked failure; value is accepted count.
/// # Safety
/// File must be live and exclusively borrowed. Source must be readable for
/// length bytes (null is permitted for zero length), disjoint from file/status,
/// and remain borrowed until this call returns.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_write(
    file: *mut c_void,
    source: *const u8,
    length: usize,
    status: *mut DrFileStatusV1,
) -> u8 {
    *status = DrFileStatusV1::default();
    let source = if length == 0 {
        &[]
    } else {
        core::slice::from_raw_parts(source, length)
    };
    (*status).write((*file.cast::<OwnedFile>()).write(source))
}

/// Reads the existing private position/length tags through shared owner access.
/// Tags: 4 position, 5 length. No cursor mutation, retention or byte transfer.
/// # Safety
/// File is a live shared OwnedFile, disjoint from writable status. Mutation,
/// move, close and destruction are excluded for this call.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_inspect(
    file: *const c_void,
    operation: u8,
    status: *mut DrFileStatusV1,
) -> u8 {
    *status = DrFileStatusV1::default();
    let file = &*file.cast::<OwnedFile>();
    let result = match operation {
        4 => file.position(),
        5 => file.length(),
        _ => Err(Failure::invalid_input()),
    };
    (*status).complete(result)
}

/// Private control tags: 0 buffer flush, 1 data sync, 2 metadata sync, 3 seek,
/// 4 position, 5 length, 6 set length, 7 lock (waits), 8 try lock (value 1
/// acquired, 0 contended), 9 unlock, 10 best-effort unlock (never fails).
/// Seek origin: 0 start, 1 current, 2 end. Lock mode, also in `origin`:
/// 0 shared, 1 exclusive. No implicit sync.
/// # Safety
/// File must be live and exclusively borrowed, disjoint from writable status.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_control(
    file: *mut c_void,
    operation: u8,
    offset: i64,
    origin: u8,
    status: *mut DrFileStatusV1,
) -> u8 {
    if matches!(operation, 4 | 5) {
        return dr_v1_file_inspect(file.cast_const(), operation, status);
    }
    *status = DrFileStatusV1::default();
    let file = &mut *file.cast::<OwnedFile>();
    let result = match operation {
        0 => file.flush().map(|()| 0),
        1 => file.sync_data().map(|()| 0),
        2 => file.sync_metadata().map(|()| 0),
        6 => file.set_length(offset).map(|()| 0),
        7 | 8 => {
            let mode = match origin {
                0 => LockMode::Shared,
                1 => LockMode::Exclusive,
                _ => return (*status).fail(Failure::invalid_input()),
            };
            file.acquire_lock(mode, operation == 7).map(u64::from)
        }
        9 => file.release_lock().map(|()| 0),
        10 => {
            file.abandon_lock();
            Ok(0)
        }
        3 => {
            let origin = match origin {
                0 => SeekOrigin::Start,
                1 => SeekOrigin::Current,
                2 => SeekOrigin::End,
                _ => return (*status).fail(Failure::invalid_input()),
            };
            file.seek(offset, origin)
        }
        _ => Err(Failure::invalid_input()),
    };
    (*status).complete(result)
}

/// Consumes the owner with exactly one checked close attempt, even on failure.
/// # Safety
/// File must be independently owned, live, unborrowed and disjoint from status.
/// Its storage becomes uninitialized; neither drop nor reuse may follow.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_close(file: *mut c_void, status: *mut DrFileStatusV1) -> u8 {
    *status = DrFileStatusV1::default();
    (*status).complete(ptr::read(file.cast::<OwnedFile>()).close().map(|()| 0))
}

/// Best-effort nonthrowing destruction, without flushing or durability promises.
/// # Safety
/// File must be independently owned, live and unborrowed. Its storage becomes
/// uninitialized and must not be destroyed a second time.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_drop(file: *mut c_void) {
    ptr::drop_in_place(file.cast::<OwnedFile>());
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
