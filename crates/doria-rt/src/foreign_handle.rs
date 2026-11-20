//! The box behind the handle of a two-word carrier (Decision 0148 section
//! 11, amendment Readable Names of 2026-11-20).
//!
//! A one-word value is its own handle. A two-word carrier, an open class
//! value (its object and class descriptor, because its dynamic class may be
//! a descendant of its static one) or a shared reference to an interface
//! (its two words), cannot fit in the one word C keeps, so its handle is the
//! address of one runtime allocation holding both words; `Handle::reclaim`
//! and the shared families' `reclaim` read them back, so the value keeps its
//! exact dynamic type, and free the box. The allocator is the runtime's
//! private one (Decision 0044); C only keeps and passes back the address.

use crate::{allocate, deallocate, panic_catalogued, DrStackFrameV2};

const ALLOCATION_FAILED: &[u8] = b"P1601";

/// The two words of a two-word carrier, in the order generated code stores
/// and loads them: the first at offset zero and the second one word after
/// it (an open class value's object, then its descriptor).
#[repr(C)]
pub struct DrHandleBoxV1 {
    pub first: *mut u8,
    pub second: *const u8,
}

/// Boxes a two-word carrier for `Handle::create` or `toHandle`, which hand
/// its ownership to the box. Allocation failure panics with P1601.
///
/// # Safety
///
/// `frame` is the caller's frame; `first` and `second` are the two words of
/// a live, owned open class value or shared interface reference.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_handle_box_create(
    frame: *const DrStackFrameV2,
    first: *mut u8,
    second: *const u8,
) -> *mut DrHandleBoxV1 {
    let handle = allocate(core::mem::size_of::<DrHandleBoxV1>()).cast::<DrHandleBoxV1>();
    if handle.is_null() {
        panic_catalogued(frame, ALLOCATION_FAILED);
    }
    handle.write(DrHandleBoxV1 { first, second });
    handle
}

/// Releases the box `Handle::reclaim` has read the value out of. The value
/// itself is the caller's again and is not touched.
///
/// # Safety
///
/// `handle` came from [`dr_v1_handle_box_create`] and is released once.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_handle_box_free(handle: *mut DrHandleBoxV1) {
    deallocate(handle.cast());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_holds_the_first_word_then_the_second() {
        let mut object = 7_u8;
        let descriptor = 9_u8;
        unsafe {
            let handle = dr_v1_handle_box_create(
                core::ptr::null(),
                core::ptr::addr_of_mut!(object),
                core::ptr::addr_of!(descriptor),
            );
            let words = handle.cast::<usize>();
            assert_eq!(words.read(), core::ptr::addr_of_mut!(object) as usize);
            assert_eq!(
                words.add(1).read(),
                core::ptr::addr_of!(descriptor) as usize
            );
            dr_v1_handle_box_free(handle);
        }
    }
}
