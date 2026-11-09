//! Test-authored adapter objects.
//!
//! The runtime treats `DrAdapterContextV1::object` as opaque: it keeps the
//! word and passes it back to the author's callbacks, which take `&mut` to the
//! object for the length of one call. Compiled Doria code reaches an adapter
//! object only through that word, and it never holds a reference into the
//! object while the runtime runs.
//!
//! A Rust test is the author on both sides, so it must hold its object the same
//! way. If a test casts `&mut local` (or `self`) to the word and later reads the
//! local's fields, or captures them in a closure, that reference overlaps the
//! callbacks' writes. That overlap is undefined behavior, and Rust 1.99's
//! optimizer returned stale counters for it.
//!
//! `TestObject` gives each object one heap home that is addressed only by its
//! raw pointer. The test reads and writes the object through raw places,
//! `(*object.as_ptr()).field`. Each access then covers only the field it
//! names, and no reference outlives the expression. A runtime call may
//! therefore keep using pointers it holds into other fields, such as staged
//! storage or a registered leaf, and a callback may run between two accesses.

use core::{ffi::c_void, marker::PhantomData, ptr::NonNull};

use super::DrAdapterContextV1;

pub(crate) struct TestObject<T> {
    object: NonNull<T>,
    owns: PhantomData<T>,
}

impl<T> TestObject<T> {
    pub(crate) fn new(value: T) -> Self {
        Self {
            // SAFETY: `Box::into_raw` never returns null.
            object: unsafe { NonNull::new_unchecked(Box::into_raw(Box::new(value))) },
            owns: PhantomData,
        }
    }

    /// The object's only address. Dereference it as a place for one access,
    /// never as a reference held across a runtime call.
    pub(crate) fn as_ptr(&self) -> *mut T {
        self.object.as_ptr()
    }

    /// The opaque word for `DrAdapterContextV1::object`.
    pub(crate) fn object(&self) -> *mut c_void {
        self.as_ptr().cast()
    }

    /// A context for a closed concrete receiver: this object and no descriptor.
    pub(crate) fn context(&self) -> DrAdapterContextV1 {
        DrAdapterContextV1 {
            object: self.object(),
            descriptor: core::ptr::null(),
        }
    }

    /// The callback side's borrow of the object behind `object`, the word a
    /// callback receives in its adapter context or interface carrier. It is
    /// valid for the length of one call.
    ///
    /// # Safety
    /// `object` is the word of a live `TestObject<T>`, no other borrow of the
    /// object is live, and the caller must not keep the reference after the
    /// callback returns.
    pub(crate) unsafe fn in_callback<'call, Word>(object: *mut Word) -> &'call mut T {
        &mut *object.cast::<T>()
    }
}

impl<T> Drop for TestObject<T> {
    fn drop(&mut self) {
        // SAFETY: `new` made this allocation, and only `drop` releases it.
        drop(unsafe { Box::from_raw(self.object.as_ptr()) });
    }
}
