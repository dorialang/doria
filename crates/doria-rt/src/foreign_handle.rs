//! The box behind the handle of an open class value (Decision 0148 section
//! 11, amendment Readable Names of 2026-11-20).
//!
//! A closed class's handle is its object pointer. An open class value is
//! two words, the object and its class descriptor, because its dynamic
//! class may be a descendant of its static one; its handle is the address of
//! one runtime allocation holding both, so `Handle::reclaim` returns the
//! value with its exact dynamic type. The allocator is the runtime's private
//! one (Decision 0044); C only keeps and passes back the address.

use crate::{allocate, deallocate, panic_catalogued, DrStackFrameV2};

const ALLOCATION_FAILED: &[u8] = b"P1601";

/// The two words of an open class value, in the order generated code loads
/// them: the object at offset zero and the descriptor one word after it.
#[repr(C)]
pub struct DrHandleBoxV1 {
    pub object: *mut u8,
    pub descriptor: *const u8,
}

/// Boxes an open class value for `Handle::create`, which hands its
/// ownership to the box. Allocation failure panics with P1601.
///
/// # Safety
///
/// `frame` is the caller's frame; `object` and `descriptor` are the two
/// words of a live, owned open class value.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_handle_box_create(
    frame: *const DrStackFrameV2,
    object: *mut u8,
    descriptor: *const u8,
) -> *mut DrHandleBoxV1 {
    let handle = allocate(core::mem::size_of::<DrHandleBoxV1>()).cast::<DrHandleBoxV1>();
    if handle.is_null() {
        panic_catalogued(frame, ALLOCATION_FAILED);
    }
    handle.write(DrHandleBoxV1 { object, descriptor });
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
    fn a_box_holds_the_object_then_the_descriptor() {
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
