use core::mem;
use core::ptr;

use crate::{allocate, deallocate};

#[repr(C)]
pub struct DrMixedV1 {
    pub tag: u8,
    pub type_id: u32,
    pub payload: u64,
}

pub unsafe fn new(tag: u8, type_id: u32, payload: u64) -> *mut DrMixedV1 {
    let value = allocate(mem::size_of::<DrMixedV1>()).cast::<DrMixedV1>();
    if value.is_null() {
        return ptr::null_mut();
    }
    ptr::write(
        value,
        DrMixedV1 {
            tag,
            type_id,
            payload,
        },
    );
    value
}

pub unsafe fn free(value: *mut DrMixedV1) {
    if value.is_null() {
        return;
    }
    deallocate(value.cast::<u8>());
}
