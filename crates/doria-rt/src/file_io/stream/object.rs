//! Headerless compiler-owned class storage around the existing file owner.
//! Only this runtime knows the payload layout. Source-visible move restrictions
//! remain compiler-owned; clearing the native obligation only permits ordinary
//! callee cleanup after consuming close, not reuse or idempotent public close.
use core::{ffi::c_void, mem, mem::MaybeUninit, ptr};

use crate::native_handle::ReadinessProjection;
use crate::{string_bytes, DrStackFrameV2, DrStringV1};

use super::{
    dr_v1_file_control, dr_v1_file_inspect, dr_v1_file_open, dr_v1_file_read, dr_v1_file_write,
    DrFileStatusV1, Failure, OwnedFile,
};

#[path = "object/resource.rs"]
mod resource;
pub(crate) use resource::{FileResource, ResourcePin};

#[path = "object/locking.rs"]
mod locking;

struct FileObject {
    resource: *mut FileResource,
}

impl FileObject {
    fn owner(&mut self) -> &mut OwnedFile {
        unsafe { &mut *(*self.resource).file().expect("live owned file object") }
    }

    fn close_with(
        &mut self,
        close: impl FnOnce(OwnedFile) -> Result<(), Failure>,
    ) -> Result<(), Failure> {
        // Clear the obligation before the one attempt. Path remains retained
        // until ordinary destruction, including checked-error construction.
        unsafe { (*self.resource).close_with(close) }
    }
}

impl Drop for FileObject {
    fn drop(&mut self) {
        unsafe { FileResource::drop_owner(self.resource) };
    }
}

/// Reports the runtime-owned payload size/alignment for common class allocation.
/// The payload has no allocation header or vtable. Its private resource home
/// remains stable independently of replacement/destruction of this wrapper.
/// # Safety
/// Outputs must be valid, disjoint writable usize locations.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_object_layout(size: *mut usize, alignment: *mut usize) {
    *size = mem::size_of::<FileObject>();
    *alignment = mem::align_of::<FileObject>();
}

/// Initializes uninitialized class payload only after raw file open succeeds.
/// Uses the raw open access/creation/append tags and DrFileStatusV1 unchanged:
/// 0 success, 3 checked failure, 4 allocation failure (never an IoError).
/// Failure retains no path and creates no native obligation; free storage
/// through common class free without invoking file-object drop.
/// # Safety
/// Path must be a live borrowed DrStringV1. File must be uninitialized storage
/// satisfying file_object_layout and disjoint from the path and writable status.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_object_open(
    path: *mut DrStringV1,
    access: u8,
    creation: u8,
    append: u8,
    file: *mut c_void,
    status: *mut DrFileStatusV1,
) -> u8 {
    let mut owner = MaybeUninit::<OwnedFile>::uninit();
    let result = dr_v1_file_open(
        string_bytes(path),
        (*path).byte_length,
        access,
        creation,
        append,
        owner.as_mut_ptr().cast(),
        status,
    );
    if result == 0 {
        let resource = FileResource::new(
            owner.assume_init(),
            path,
            crate::checked_io::TARGET_FILE,
            file,
        );
        if resource.is_null() {
            // Resource creation consumed/dropped the one opened obligation.
            // As with raw open allocation failure, never construct an IoError.
            return 4;
        }
        file.cast::<FileObject>().write(FileObject { resource });
    }
    result
}

/// Initializes file-object storage around a child pipe end the runtime just
/// created. Failures name `name` (the program) under `target`. Returns false
/// when the resource home cannot be allocated; the handle is then closed.
/// # Safety
/// Storage is uninitialized file-object storage; name is a live string.
pub(crate) unsafe fn adopt_pipe(
    handle: crate::native_handle::OwnedHandle,
    target: u8,
    name: *mut DrStringV1,
    storage: *mut c_void,
) -> bool {
    let resource = FileResource::new(OwnedFile::adopt_stream(handle), name, target, storage);
    if resource.is_null() {
        return false;
    }
    storage.cast::<FileObject>().write(FileObject { resource });
    true
}

/// The canonical IoTarget tag this object's failures report.
/// # Safety
/// File must be an initialized, undestroyed payload.
pub(crate) unsafe fn file_object_target(file: *const c_void) -> u8 {
    (*(*file.cast::<FileObject>()).resource).target()
}

/// The open handle, or None once closed. Borrowed: no close claim moves.
/// # Safety
/// File must be an initialized, undestroyed payload.
pub(crate) unsafe fn file_object_handle(
    file: *const c_void,
) -> Option<crate::native_handle::NativeHandle> {
    (*(*file.cast::<FileObject>()).resource).handle().ok()
}

/// Closes the handle if still open, keeping the object for later Closed
/// failures and ordinary destruction. Idempotent.
/// # Safety
/// File must be an initialized, undestroyed, unborrowed payload.
pub(crate) unsafe fn close_file_object_handle(file: *mut c_void) -> Result<(), Failure> {
    (*(*file.cast::<FileObject>()).resource).close_if_open()
}

/// Borrows the retained opening path, without allocating or retaining again.
/// It remains available after consuming close until ordinary class cleanup,
/// so canonical IoError construction can retain its own target reference.
/// # Safety
/// File must be an initialized, undestroyed payload, disjoint from mutations.
/// The result must not be released or outlive file unless retained separately.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_object_path(file: *const c_void) -> *mut DrStringV1 {
    (*(*file.cast::<FileObject>()).resource).path()
}

/// Projects the actual resource owner for explicit pending-file admission and
/// preparation. Only this runtime knows the class payload/Option layout;
/// readiness metadata and the class address are not owner reinterpretations.
/// Formation is pure: no metadata query, I/O, allocation, path retain, native
/// association or additional close claim. Admission checks file kind separately.
/// # Safety
/// File is a live initialized open payload during formation. The consumer must
/// keep its actual resource live through every use, under the current owner or
/// an explicit selected-operation pin. Such a pin expires only after native
/// retirement and adapter completion, independently of wrapper replacement.
/// The result is opaque borrowed storage, never an owner to drop or a handle.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_object_pending_endpoint(file: *mut c_void) -> *mut c_void {
    ptr::from_mut((*file.cast::<FileObject>()).owner()).cast()
}

/// Pure typed projection for pending configuration and canonical error origin.
/// # Safety
/// A nonnull input is an initialized live file payload during formation. The
/// returned resource owner/path are borrowed, not retained by this projection;
/// the consumer keeps the owner or an explicit selected-operation pin live.
pub(crate) unsafe fn pending_projection(
    file: *mut c_void,
) -> Result<(*mut OwnedFile, *mut DrStringV1), Failure> {
    let source = pending_source(file)?;
    let (file, origin) = source.origin.file().ok_or_else(Failure::invalid_input)?;
    Ok((file, origin.path))
}

/// Borrow the actual immutable source record, rejecting closed payloads before
/// admission. Forwarding its opaque value preserves both identity and origin.
/// # Safety
/// Same resource/owner loan contract as pending_projection. A cached opaque
/// projection is not itself a pin and never revives a replaced closed source.
pub(crate) unsafe fn pending_source<'a>(
    file: *const c_void,
) -> Result<&'a ReadinessProjection, Failure> {
    if file.is_null() {
        return Err(Failure::invalid_input());
    }
    let resource = &*(*file.cast::<FileObject>()).resource;
    resource.shared_file()?;
    Ok(resource.source())
}

/// Borrows the typed resource-embedded source, carrying actual native readiness
/// identity and opening-path origin through opaque adapter forwarding. This is
/// distinct from raw fixture metadata; no class/owner layout alias is assumed.
/// No association, allocation, path retention or I/O occurs here. A half-closed
/// child input still forms its source; admission reports it as Closed.
/// # Safety
/// File is a live initialized payload during formation. Ordinary borrowed
/// use remains within its owner loan. Runtime routes explicitly pin metadata;
/// selected native work additionally pins the resource's native obligation.
/// The result grants no independent close claim or access to a platform handle.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_object_readiness_source(file: *const c_void) -> *const c_void {
    (*(*file.cast::<FileObject>()).resource).source().value()
}

/// Exact embedded source home, with the same pure formation and receiver loan
/// as file_object_readiness_source. Class and inline-owner layouts never alias.
/// # Safety
/// The payload is initialized and open during formation. The returned readonly
/// home has the same ordinary-owner or explicit resource-pin lifetime as the
/// source value, not the replaceable wrapper's storage lifetime.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_object_readiness_source_home(
    file: *const c_void,
) -> *const c_void {
    (*(*file.cast::<FileObject>()).resource).source().home()
}

/// Borrows a live owner and writable caller region; returns raw read statuses.
/// # Safety
/// File must be initialized, open, exclusively borrowed and disjoint from
/// destination/status. Destination is writable for capacity bytes until return
/// (null is permitted for zero capacity). Status must be writable and disjoint.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_object_read(
    file: *mut c_void,
    destination: *mut u8,
    capacity: usize,
    status: *mut DrFileStatusV1,
) -> u8 {
    // A half-closed child input stays an object: its transfers fail Closed.
    match (*(*file.cast::<FileObject>()).resource).file() {
        Ok(owner) => dr_v1_file_read(owner.cast(), destination, capacity, status),
        Err(failure) => {
            *status = DrFileStatusV1::default();
            (*status).fail(failure)
        }
    }
}

/// Borrows a live owner and readonly caller region; returns raw write statuses.
/// # Safety
/// File must be initialized, open, exclusively borrowed and disjoint from
/// source/status. Source is readable for length bytes until return (null is
/// permitted for zero length). Status must be writable and disjoint.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_object_write(
    file: *mut c_void,
    source: *const u8,
    length: usize,
    status: *mut DrFileStatusV1,
) -> u8 {
    match (*(*file.cast::<FileObject>()).resource).file() {
        Ok(owner) => dr_v1_file_write(owner.cast(), source, length, status),
        Err(failure) => {
            *status = DrFileStatusV1::default();
            (*status).fail(failure)
        }
    }
}

/// `dr_v1_file_object_read` for a PHP host, which names the destination by range.
/// # Safety
/// As `dr_v1_file_object_read`, for the range `byte_region::host_range` forms.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_file_object_read(
    file: *mut c_void,
    base: *mut u8,
    capacity: usize,
    offset: usize,
    length: usize,
    status: *mut DrFileStatusV1,
) -> u8 {
    let destination = crate::byte_region::host_range(base, capacity, offset, length);
    dr_v1_file_object_read(file, destination, length, status)
}

/// `dr_v1_file_object_write` for a PHP host, which names the source by range.
/// # Safety
/// As `dr_v1_file_object_write`, for the range `byte_region::host_range` forms.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_file_object_write(
    file: *mut c_void,
    base: *const u8,
    capacity: usize,
    offset: usize,
    length: usize,
    status: *mut DrFileStatusV1,
) -> u8 {
    let source = crate::byte_region::host_range(base.cast_mut(), capacity, offset, length);
    dr_v1_file_object_write(file, source, length, status)
}

/// Inspects an initialized opaque owner without acquiring exclusive file access.
/// Tags: 4 position, 5 length. A retired native obligation reports Closed.
/// # Safety
/// File is initialized shared FileObject storage, disjoint from writable status.
/// Mutation, move, close and destruction are excluded for this call.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_object_inspect(
    file: *const c_void,
    operation: u8,
    status: *mut DrFileStatusV1,
) -> u8 {
    *status = DrFileStatusV1::default();
    let resource = &*(*file.cast::<FileObject>()).resource;
    match resource.shared_file() {
        Ok(owner) => dr_v1_file_inspect(owner.cast(), operation, status),
        Err(failure) => (*status).fail(failure),
    }
}

/// Borrows the opaque owner for the existing private file-control operations.
/// Tags: 0 buffer flush, 1 data sync, 2 metadata sync, 3 seek, 4 position,
/// 5 length, 6 set length. Seek origin: 0 start, 1 current, 2 end. Flush never implies sync.
/// Returns 0 success or 3 checked failure through the unchanged status record.
/// # Safety
/// File must be initialized, open, exclusively borrowed and disjoint from the
/// writable status.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_object_control(
    file: *mut c_void,
    operation: u8,
    offset: i64,
    origin: u8,
    status: *mut DrFileStatusV1,
) -> u8 {
    if matches!(operation, 4 | 5) {
        return dr_v1_file_object_inspect(file.cast_const(), operation, status);
    }
    dr_v1_file_control(
        ((*file.cast::<FileObject>()).owner() as *mut OwnedFile).cast(),
        operation,
        offset,
        origin,
        status,
    )
}

/// File controls shared by native and hosted calls.
/// Tags: 1 data sync, 2 full sync, 3 seek, 4 position, 5 length, 6 set length,
/// 7 lock, 8 try lock, 9 unlock, 10 best-effort unlock (see `dr_v1_file_control`).
/// Unrepresentable canonical-int results are InvalidInput.
/// # Safety
/// File is live initialized storage, disjoint from writable status. Mutating
/// controls require exclusive access; inspection permits shared access. No move/close/drop may
/// occur during either call. Offset/origin are ignored for inspection.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_object_control_int(
    file: *mut c_void,
    operation: u8,
    offset: i64,
    origin: u8,
    status: *mut DrFileStatusV1,
) -> u8 {
    let code = match operation {
        // Half-closes a child's input pipe; repeating it is a no-op.
        11 => {
            *status = DrFileStatusV1::default();
            return (*status).complete(close_file_object_handle(file).map(|()| 0));
        }
        1..=3 | 6..=10 => {
            *status = DrFileStatusV1::default();
            let resource = &*(*file.cast::<FileObject>()).resource;
            match resource.file() {
                Ok(owner) => dr_v1_file_control(owner.cast(), operation, offset, origin, status),
                Err(failure) => (*status).fail(failure),
            }
        }
        4 | 5 => dr_v1_file_object_inspect(file.cast_const(), operation, status),
        _ => {
            *status = DrFileStatusV1::default();
            return (*status).fail(Failure::invalid_input());
        }
    };
    if code == 0 && (*status).value > i64::MAX as u64 {
        *status = DrFileStatusV1::default();
        return (*status).fail(Failure::invalid_input());
    }
    code
}

/// Consumes the native obligation exactly once, even on checked close failure.
/// Retains the path and payload for subsequent canonical error construction
/// and ordinary callee cleanup; neither a second close nor transfers may follow.
/// # Safety
/// File must be independently owned, initialized, open, unborrowed and disjoint
/// from writable status. File-object drop must run once before common class free.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_object_close(
    file: *mut c_void,
    status: *mut DrFileStatusV1,
) -> u8 {
    *status = DrFileStatusV1::default();
    (*status).complete(
        (*file.cast::<FileObject>())
            .close_with(OwnedFile::close)
            .map(|()| 0),
    )
}

/// Nonthrowing cleanup closes any remaining obligation and releases the path.
/// Does not free payload storage: generated class cleanup calls common class
/// free afterwards. A preceding explicit close never causes another kernel close.
/// # Safety
/// Frame is null or live for this call only. File must be independently owned,
/// initialized, undestroyed and unborrowed.
/// Its contents become uninitialized and must not be used or dropped again.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_file_object_drop(_frame: *const DrStackFrameV2, file: *mut c_void) {
    ptr::drop_in_place(file.cast::<FileObject>());
}

#[cfg(test)]
#[path = "object_tests.rs"]
mod tests;
