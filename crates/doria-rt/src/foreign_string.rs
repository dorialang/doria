//! Owned foreign strings and the readers of C text (Decision 0148 section 9).
//!
//! A `CString` or `Utf16String` value is the address of one runtime
//! allocation holding its NUL-terminated code units, so `pointer` is the
//! value itself and the buffer never moves while the value lives.
//! `dr_v1_foreign_string_free` releases it. The allocator is the runtime's
//! private one; foreign code never releases these buffers (section 10).

use core::ptr;

use crate::{
    allocate, allocate_string_with_frame, deallocate, panic_catalogued, string_bytes_mut,
    DrStackFrameV2, DrStringV1,
};

const INTERIOR_NUL: &[u8] = b"P1705";
const ALLOCATION_FAILED: &[u8] = b"P1302";

unsafe fn text<'a>(value: *const DrStringV1) -> &'a [u8] {
    match crate::dr_v1_string_byte_length(value) {
        0 => &[],
        length => core::slice::from_raw_parts(crate::dr_v1_string_data(value), length),
    }
}

/// A buffer of `units` code units of `unit_bytes` each plus a NUL unit.
pub(crate) unsafe fn buffer(
    frame: *const DrStackFrameV2,
    units: usize,
    unit_bytes: usize,
) -> *mut u8 {
    let total = units
        .checked_add(1)
        .and_then(|units| units.checked_mul(unit_bytes))
        .unwrap_or_else(|| panic_catalogued(frame, ALLOCATION_FAILED));
    let data = allocate(total);
    if data.is_null() {
        panic_catalogued(frame, ALLOCATION_FAILED);
    }
    data
}

unsafe fn c_string(
    frame: *const DrStackFrameV2,
    value: *const DrStringV1,
    panics: bool,
) -> *mut u8 {
    let source = text(value);
    if source.contains(&0) {
        if panics {
            panic_catalogued(frame, INTERIOR_NUL);
        }
        return ptr::null_mut();
    }
    let data = buffer(frame, source.len(), 1);
    ptr::copy_nonoverlapping(source.as_ptr(), data, source.len());
    data.add(source.len()).write(0);
    data
}

unsafe fn utf16_string(
    frame: *const DrStackFrameV2,
    value: *const DrStringV1,
    panics: bool,
) -> *mut u8 {
    // Every string holds valid UTF-8, and NUL is the code unit 0 in both
    // encodings.
    let source = core::str::from_utf8_unchecked(text(value));
    if source.as_bytes().contains(&0) {
        if panics {
            panic_catalogued(frame, INTERIOR_NUL);
        }
        return ptr::null_mut();
    }
    let units = source.encode_utf16().count();
    let data = buffer(frame, units, 2).cast::<u16>();
    for (index, unit) in source.encode_utf16().enumerate() {
        data.add(index).write(unit);
    }
    data.add(units).write(0);
    data.cast()
}

/// `CString::from`: a NUL-terminated copy of the string's UTF-8 bytes.
/// Panics with P1705 when the string contains NUL.
///
/// # Safety
/// `value` must be a live runtime string and `frame` null or valid.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_c_string_from(
    frame: *const DrStackFrameV2,
    value: *const DrStringV1,
) -> *mut u8 {
    c_string(frame, value, true)
}

/// `CString::tryFrom`: as [`dr_v2_c_string_from`], null when the string
/// contains NUL.
///
/// # Safety
/// `value` must be a live runtime string and `frame` null or valid.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_c_string_try_from(
    frame: *const DrStackFrameV2,
    value: *const DrStringV1,
) -> *mut u8 {
    c_string(frame, value, false)
}

/// `Utf16String::from`: the string as NUL-terminated UTF-16. Panics with
/// P1705 when the string contains NUL.
///
/// # Safety
/// `value` must be a live runtime string and `frame` null or valid.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_utf16_string_from(
    frame: *const DrStackFrameV2,
    value: *const DrStringV1,
) -> *mut u8 {
    utf16_string(frame, value, true)
}

/// `Utf16String::tryFrom`: as [`dr_v2_utf16_string_from`], null when the
/// string contains NUL.
///
/// # Safety
/// `value` must be a live runtime string and `frame` null or valid.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_utf16_string_try_from(
    frame: *const DrStackFrameV2,
    value: *const DrStringV1,
) -> *mut u8 {
    utf16_string(frame, value, false)
}

/// Releases a `CString` or `Utf16String` buffer.
///
/// # Safety
/// `buffer` must be null or a buffer one of the constructors above
/// returned, released once.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_foreign_string_free(buffer: *mut u8) {
    if !buffer.is_null() {
        deallocate(buffer);
    }
}

/// `String::fromCString`: the string the NUL-terminated bytes at `data`
/// hold, or null when they are not valid UTF-8.
///
/// # Safety
/// `data` must address readable memory up to and including a NUL byte, and
/// `frame` must be null or valid.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_from_c_string(
    frame: *const DrStackFrameV2,
    data: *const u8,
) -> *mut DrStringV1 {
    let mut length = 0;
    while data.add(length).read() != 0 {
        length += 1;
    }
    let source = core::slice::from_raw_parts(data, length);
    if core::str::from_utf8(source).is_err() {
        return ptr::null_mut();
    }
    let string = allocate_string_with_frame(frame, length);
    if length != 0 {
        ptr::copy_nonoverlapping(data, string_bytes_mut(string), length);
    }
    string
}

/// `String::fromUtf16`: the string the NUL-terminated UTF-16 code units at
/// `data` hold, or null when they contain an unpaired surrogate.
///
/// # Safety
/// `data` must address readable memory up to and including a NUL code unit,
/// and `frame` must be null or valid. `data` need not be aligned.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_from_utf16(
    frame: *const DrStackFrameV2,
    data: *const u16,
) -> *mut DrStringV1 {
    let mut units = 0;
    while data.add(units).read_unaligned() != 0 {
        units += 1;
    }
    let code_units = (0..units).map(|index| data.add(index).read_unaligned());
    let mut length = 0;
    for decoded in core::char::decode_utf16(code_units.clone()) {
        match decoded {
            Ok(character) => length += character.len_utf8(),
            Err(_) => return ptr::null_mut(),
        }
    }
    let string = allocate_string_with_frame(frame, length);
    let mut target = string_bytes_mut(string);
    for character in core::char::decode_utf16(code_units).flatten() {
        let mut encoded = [0_u8; 4];
        let encoded = character.encode_utf8(&mut encoded);
        ptr::copy_nonoverlapping(encoded.as_ptr(), target, encoded.len());
        target = target.add(encoded.len());
    }
    string
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe fn string(value: &str) -> *mut DrStringV1 {
        crate::dr_v1_string_from_utf8(value.as_ptr(), value.len())
    }

    unsafe fn read(value: *const DrStringV1) -> &'static str {
        core::str::from_utf8(text(value)).unwrap()
    }

    #[test]
    fn c_strings_copy_utf8_and_end_with_nul() {
        unsafe {
            let source = string("héllo");
            let buffer = dr_v2_c_string_from(ptr::null(), source);
            let bytes = core::slice::from_raw_parts(buffer, 7);
            assert_eq!(bytes, "héllo\0".as_bytes());
            let back = dr_v2_string_from_c_string(ptr::null(), buffer);
            assert_eq!(read(back), "héllo");
            dr_v1_foreign_string_free(buffer);
            let empty = dr_v2_c_string_try_from(ptr::null(), string(""));
            assert_eq!(empty.read(), 0);
            dr_v1_foreign_string_free(empty);
            assert!(dr_v2_c_string_try_from(ptr::null(), string("a\0b")).is_null());
        }
    }

    #[test]
    fn utf16_strings_round_trip_and_reject_unpaired_surrogates() {
        unsafe {
            let source = string("A😀");
            let buffer = dr_v2_utf16_string_from(ptr::null(), source).cast::<u16>();
            let units = core::slice::from_raw_parts(buffer, 4);
            assert_eq!(units, &[0x41, 0xD83D, 0xDE00, 0]);
            let back = dr_v2_string_from_utf16(ptr::null(), buffer);
            assert_eq!(read(back), "A😀");
            dr_v1_foreign_string_free(buffer.cast());
            assert!(dr_v2_utf16_string_try_from(ptr::null(), string("\0")).is_null());
            let unpaired = [0xD83D_u16, 0x41, 0];
            assert!(dr_v2_string_from_utf16(ptr::null(), unpaired.as_ptr()).is_null());
            let invalid = [0xFF_u8, 0];
            assert!(dr_v2_string_from_c_string(ptr::null(), invalid.as_ptr()).is_null());
        }
    }
}
