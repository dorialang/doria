//! Private lifetime home for a public File's sole native obligation. Metadata
//! pins do not keep an unused handle open; active operation pins do. This is
//! not public shared ownership and grants no mutable File/buffer access.
use core::{
    cell::{Cell, UnsafeCell},
    ffi::c_void,
    mem::MaybeUninit,
    ptr,
};

use super::{Failure, OwnedFile, ReadinessProjection};
use crate::native_handle::NativeHandle;
use crate::{dr_v1_string_release, dr_v1_string_retain, DrStringV1};

pub(crate) struct FileResource {
    references: Cell<usize>,
    active_uses: Cell<usize>,
    owner_present: Cell<bool>,
    file: UnsafeCell<Option<OwnedFile>>,
    /// The file's path, or the program a child pipe belongs to.
    path: *mut DrStringV1,
    /// The canonical IoTarget tag its failures report (checked_io TARGET_*).
    target: u8,
    source: MaybeUninit<ReadinessProjection>,
    regular: Cell<Option<bool>>,
}

const _: () = assert!(core::mem::align_of::<FileResource>() <= crate::NATURAL_ALLOCATION_ALIGNMENT);

impl FileResource {
    /// Allocate once at open. Getters/forwarding never allocate or retain.
    /// # Safety
    /// The file is the sole close obligation; path and payload are live.
    pub(super) unsafe fn new(
        file: OwnedFile,
        path: *mut DrStringV1,
        target: u8,
        payload: *mut c_void,
    ) -> *mut Self {
        let resource = crate::allocate(core::mem::size_of::<Self>()).cast::<Self>();
        if resource.is_null() {
            drop(file);
            return resource;
        }
        resource.write(Self {
            references: Cell::new(1),
            active_uses: Cell::new(0),
            owner_present: Cell::new(true),
            file: UnsafeCell::new(Some(file)),
            path: dr_v1_string_retain(path),
            target,
            source: MaybeUninit::uninit(),
            regular: Cell::new(None),
        });
        let owner = (*resource).file().expect("new file resource");
        ReadinessProjection::initialize_resource(
            ptr::addr_of_mut!((*resource).source).cast(),
            payload,
            owner,
            path,
            resource,
        );
        resource
    }

    pub(super) fn path(&self) -> *mut DrStringV1 {
        self.path
    }

    pub(crate) fn target(&self) -> u8 {
        self.target
    }

    /// Closes the handle if it is still open; later operations fail Closed.
    /// A second call is a no-op, which half-closing a child's input needs.
    pub(super) unsafe fn close_if_open(&self) -> Result<(), Failure> {
        debug_assert_eq!(self.active_uses.get(), 0);
        match (*self.file.get()).take() {
            Some(file) => file.close(),
            None => Ok(()),
        }
    }

    pub(super) unsafe fn source(&self) -> &ReadinessProjection {
        self.source.assume_init_ref()
    }

    /// # Safety
    /// Metadata access is serialized by the existing owner/group discipline.
    /// The returned file is accessed exclusively under the selected operation
    /// and ordinary compiler buffer/cursor loans, not by this metadata pin.
    pub(crate) unsafe fn file(&self) -> Result<*mut OwnedFile, Failure> {
        (*self.file.get())
            .as_mut()
            .map(ptr::from_mut)
            .ok_or_else(closed)
    }

    /// # Safety
    /// The live owner or resource pin excludes close/destruction and exclusive
    /// file access while the returned shared pointer is used.
    pub(crate) unsafe fn shared_file(&self) -> Result<*const OwnedFile, Failure> {
        (*self.file.get())
            .as_ref()
            .map(ptr::from_ref)
            .ok_or_else(closed)
    }

    pub(crate) unsafe fn handle(&self) -> Result<NativeHandle, Failure> {
        Ok((*self.shared_file()?).native_handle())
    }

    pub(crate) unsafe fn is_regular(&self) -> Result<bool, Failure> {
        let handle = self.handle()?;
        if let Some(regular) = self.regular.get() {
            return Ok(regular);
        }
        let regular = crate::native_handle::is_regular_file(handle)?;
        self.regular.set(Some(regular));
        Ok(regular)
    }

    pub(super) unsafe fn close_with(
        &self,
        close: impl FnOnce(OwnedFile) -> Result<(), Failure>,
    ) -> Result<(), Failure> {
        // The compiler proves consuming close is unborrowed. This assertion is
        // a runtime implementation invariant, not a new public access guard.
        debug_assert_eq!(self.active_uses.get(), 0);
        close((*self.file.get()).take().expect("live owned file object"))
    }

    pub(super) unsafe fn drop_owner(resource: *mut Self) {
        let value = &*resource;
        let had_owner = value.owner_present.replace(false);
        debug_assert!(had_owner);
        if value.active_uses.get() == 0 {
            drop((*value.file.get()).take());
        }
        Self::release(resource);
    }

    unsafe fn retain(&self) {
        self.references.set(
            self.references
                .get()
                .checked_add(1)
                .expect("file resource reference count"),
        );
    }

    unsafe fn release(resource: *mut Self) {
        let value = &*resource;
        let remaining = value.references.get() - 1;
        value.references.set(remaining);
        if remaining == 0 {
            debug_assert!(!value.owner_present.get());
            debug_assert_eq!(value.active_uses.get(), 0);
            ptr::drop_in_place(resource);
            crate::deallocate(resource.cast());
        }
    }
}

impl Drop for FileResource {
    fn drop(&mut self) {
        // Usually already closed by consuming close or last active retirement.
        // Taking first preserves the one-attempt implicit destruction rule.
        drop(self.file.get_mut().take());
        unsafe { dr_v1_string_release(self.path) };
    }
}

fn closed() -> Failure {
    Failure {
        reason: crate::platform_io::Reason::Closed,
        system_code: None,
        transferred_byte_count: None,
    }
}

pub(crate) struct ResourcePin {
    resource: *mut FileResource,
    active: bool,
}

impl ResourcePin {
    /// # Safety
    /// Resource is live under a current owner or another pin. All metadata
    /// operations occur on the group/owner thread, never a helper worker.
    pub(crate) unsafe fn metadata(resource: *mut FileResource) -> Self {
        (*resource).retain();
        Self {
            resource,
            active: false,
        }
    }

    /// # Safety
    /// Same serialized lifetime contract as metadata. Only the current owner's
    /// source may select new work; an old pin never revives a retired owner.
    pub(crate) unsafe fn selected(resource: *mut FileResource) -> Result<Self, Failure> {
        let value = &*resource;
        if !value.owner_present.get() {
            return Err(closed());
        }
        value.shared_file()?;
        let next = value
            .active_uses
            .get()
            .checked_add(1)
            .expect("file active use count");
        value.retain();
        value.active_uses.set(next);
        Ok(Self {
            resource,
            active: true,
        })
    }
}

impl Drop for ResourcePin {
    fn drop(&mut self) {
        unsafe {
            let value = &*self.resource;
            if self.active {
                let remaining = value.active_uses.get() - 1;
                value.active_uses.set(remaining);
                if remaining == 0 && !value.owner_present.get() {
                    // Native retirement and adapter completion must precede
                    // dropping this pin. A cancellation request is not enough.
                    drop((*value.file.get()).take());
                }
            }
            FileResource::release(self.resource);
        }
    }
}
