use core::mem;
use core::ptr;

use crate::{allocate, deallocate, dr_v2_panic_code, DrStackFrameV2};

#[repr(C)]
pub struct DrBytesV1 {
    length: usize,
    data: *mut u8,
}

pub unsafe fn copy(source: *const u8, length: usize) -> *mut DrBytesV1 {
    let data = allocate_data(length);
    if length != 0 {
        ptr::copy_nonoverlapping(source, data, length);
    }
    allocate_header(data, length)
}

pub unsafe fn from_owned(data: *mut u8, length: usize) -> *mut DrBytesV1 {
    allocate_header(data, length)
}

unsafe fn allocate_header(data: *mut u8, length: usize) -> *mut DrBytesV1 {
    let bytes = allocate(mem::size_of::<DrBytesV1>()).cast::<DrBytesV1>();
    if bytes.is_null() {
        if !data.is_null() {
            deallocate(data);
        }
        bytes_panic(b"P1302");
    }
    ptr::write(bytes, DrBytesV1 { length, data });
    bytes
}

unsafe fn allocate_data(length: usize) -> *mut u8 {
    if length == 0 {
        return ptr::null_mut();
    }
    let data = allocate(length);
    if data.is_null() {
        bytes_panic(b"P1302");
    }
    data
}

pub unsafe fn free(bytes: *mut DrBytesV1) {
    if bytes.is_null() {
        return;
    }
    if !(*bytes).data.is_null() {
        deallocate((*bytes).data);
    }
    deallocate(bytes.cast());
}

pub unsafe fn length(bytes: *const DrBytesV1) -> usize {
    (*bytes).length
}

pub unsafe fn data(bytes: *const DrBytesV1) -> *const u8 {
    (*bytes).data
}

pub unsafe fn get(frame: *const DrStackFrameV2, bytes: *const DrBytesV1, index: usize) -> u8 {
    if index >= (*bytes).length {
        dr_v2_panic_code(frame, b"P1301".as_ptr(), 5, ptr::null(), 0);
    }
    *(*bytes).data.add(index)
}

pub unsafe fn set(frame: *const DrStackFrameV2, bytes: *mut DrBytesV1, index: usize, value: u8) {
    if index >= (*bytes).length {
        dr_v2_panic_code(frame, b"P1301".as_ptr(), 5, ptr::null(), 0);
    }
    *(*bytes).data.add(index) = value;
}

pub unsafe fn equal(left: *const DrBytesV1, right: *const DrBytesV1) -> bool {
    if (*left).length != (*right).length {
        return false;
    }
    let length = (*left).length;
    if length == 0 {
        return true;
    }
    core::slice::from_raw_parts((*left).data, length)
        == core::slice::from_raw_parts((*right).data, length)
}

fn bytes_panic(code: &'static [u8]) -> ! {
    unsafe { dr_v2_panic_code(ptr::null(), code.as_ptr(), code.len(), ptr::null(), 0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_buffers_copy_mutate_and_compare_by_value() {
        unsafe {
            let source = [0, 128, 255];
            let left = copy(source.as_ptr(), source.len());
            let right = copy(source.as_ptr(), source.len());
            assert_eq!(length(left), 3);
            assert_eq!(get(ptr::null(), left, 1), 128);
            assert!(equal(left, right));

            set(ptr::null(), left, 1, 42);
            assert_eq!(get(ptr::null(), left, 1), 42);
            assert!(!equal(left, right));
            assert_eq!(source[1], 128, "construction must copy the source");

            free(left);
            free(right);
        }
    }

    #[test]
    fn empty_byte_buffers_have_value_equality() {
        unsafe {
            let left = copy(ptr::null(), 0);
            let right = copy(ptr::null(), 0);
            assert_eq!(length(left), 0);
            assert!(equal(left, right));
            free(left);
            free(right);
        }
    }
}
