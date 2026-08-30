use core::mem;
use core::ptr;

use doria_unicode::{CaseMapping, PadSide, StringError, TrimMode};

use crate::{
    allocate, allocate_string_with_frame, bytes, collection, deallocate, dr_v1_string_retain,
    dr_v2_panic_code, dr_v2_panic_signed_fact, dr_v2_panic_string_padding_empty, string_bytes_mut,
    DrBytesV1, DrCollectionV1, DrStackFrameV2, DrStringV1,
};

unsafe fn text<'a>(value: *const DrStringV1) -> &'a str {
    let bytes = core::slice::from_raw_parts(
        crate::dr_v1_string_data(value),
        crate::dr_v1_string_byte_length(value),
    );
    core::str::from_utf8_unchecked(bytes)
}

unsafe fn panic_error(frame: *const DrStackFrameV2, error: StringError) -> ! {
    let code = match error {
        StringError::SliceLengthNegative => b"P1201",
        StringError::PaddingLengthNegative => b"P1202",
        StringError::PaddingTextEmpty => b"P1203",
        StringError::RepetitionCountNegative => b"P1204",
        StringError::ResultTooLarge => b"P1205",
    };
    let message = error.panic_message().as_bytes();
    dr_v2_panic_code(
        frame,
        code.as_ptr(),
        code.len(),
        message.as_ptr(),
        message.len(),
    )
}

unsafe fn new_result(frame: *const DrStackFrameV2, byte_length: usize) -> *mut DrStringV1 {
    allocate_string_with_frame(frame, byte_length)
}

unsafe fn copy_range(
    frame: *const DrStackFrameV2,
    source: &str,
    range: core::ops::Range<usize>,
) -> *mut DrStringV1 {
    let source = &source.as_bytes()[range];
    let result = new_result(frame, source.len());
    if !source.is_empty() {
        ptr::copy_nonoverlapping(source.as_ptr(), string_bytes_mut(result), source.len());
    }
    result
}

const ASSERTION_PRESENTATION_LIMIT: usize = 4096;
const ASSERTION_TRUNCATION_MARKER: &[u8] = b"...<truncated>";

const fn assertion_escape_length(byte: u8) -> usize {
    match byte {
        b'\"' | b'\\' | b'\n' | b'\r' | b'\t' => 2,
        0..=0x1f | 0x7f => 6,
        _ => 1,
    }
}

unsafe fn write_assertion_escape(output: *mut u8, byte: u8) -> usize {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    match byte {
        b'\"' | b'\\' => {
            output.write(b'\\');
            output.add(1).write(byte);
            2
        }
        b'\n' => {
            output.write(b'\\');
            output.add(1).write(b'n');
            2
        }
        b'\r' => {
            output.write(b'\\');
            output.add(1).write(b'r');
            2
        }
        b'\t' => {
            output.write(b'\\');
            output.add(1).write(b't');
            2
        }
        0..=0x1f | 0x7f => {
            ptr::copy_nonoverlapping(b"\\u00".as_ptr(), output, 4);
            output.add(4).write(HEX[usize::from(byte >> 4)]);
            output.add(5).write(HEX[usize::from(byte & 0x0f)]);
            6
        }
        _ => {
            output.write(byte);
            1
        }
    }
}

/// Produces the compiler-owned bounded quoted presentation used only on a
/// failed assertion edge. The source string is borrowed and unchanged.
#[no_mangle]
pub unsafe extern "C" fn dr_v4_string_assertion_quote(
    frame: *const DrStackFrameV2,
    value: *const DrStringV1,
) -> *mut DrStringV1 {
    let source = text(value).as_bytes();
    let escaped_length = source.iter().fold(0usize, |length, byte| {
        length.saturating_add(assertion_escape_length(*byte))
    });
    let truncated = escaped_length.saturating_add(2) > ASSERTION_PRESENTATION_LIMIT;
    let content_limit = if truncated {
        ASSERTION_PRESENTATION_LIMIT - 2 - ASSERTION_TRUNCATION_MARKER.len()
    } else {
        escaped_length
    };
    let mut consumed = 0usize;
    let mut content_length = 0usize;
    while consumed < source.len() {
        let byte = source[consumed];
        let unit_length = if byte < 0x80 {
            assertion_escape_length(byte)
        } else {
            let width = if byte < 0xe0 {
                2
            } else if byte < 0xf0 {
                3
            } else {
                4
            };
            if content_length.saturating_add(width) > content_limit {
                break;
            }
            content_length += width;
            consumed += width;
            continue;
        };
        if content_length.saturating_add(unit_length) > content_limit {
            break;
        }
        content_length += unit_length;
        consumed += 1;
    }
    let result_length = 2
        + content_length
        + if truncated {
            ASSERTION_TRUNCATION_MARKER.len()
        } else {
            0
        };
    let result = new_result(frame, result_length);
    let output = string_bytes_mut(result);
    output.write(b'\"');
    let mut source_index = 0usize;
    let mut output_index = 1usize;
    while source_index < consumed {
        let byte = source[source_index];
        if byte < 0x80 {
            output_index += write_assertion_escape(output.add(output_index), byte);
            source_index += 1;
        } else {
            let width = if byte < 0xe0 {
                2
            } else if byte < 0xf0 {
                3
            } else {
                4
            };
            ptr::copy_nonoverlapping(
                source.as_ptr().add(source_index),
                output.add(output_index),
                width,
            );
            source_index += width;
            output_index += width;
        }
    }
    if truncated {
        ptr::copy_nonoverlapping(
            ASSERTION_TRUNCATION_MARKER.as_ptr(),
            output.add(output_index),
            ASSERTION_TRUNCATION_MARKER.len(),
        );
        output_index += ASSERTION_TRUNCATION_MARKER.len();
    }
    output.add(output_index).write(b'\"');
    result
}

unsafe fn transform(
    frame: *const DrStackFrameV2,
    value: *const DrStringV1,
    mapping: CaseMapping,
) -> *mut DrStringV1 {
    let source = text(value);
    let length = doria_unicode::case_output_length(source, mapping)
        .unwrap_or_else(|error| panic_error(frame, error));
    let result = new_result(frame, length);
    let output = core::slice::from_raw_parts_mut(string_bytes_mut(result), length);
    if doria_unicode::write_case(source, mapping, output).is_err() {
        panic_error(frame, StringError::ResultTooLarge);
    }
    result
}

unsafe fn transform_first(
    frame: *const DrStackFrameV2,
    value: *const DrStringV1,
    mapping: CaseMapping,
) -> *mut DrStringV1 {
    let source = text(value);
    let length = doria_unicode::first_case_output_length(source, mapping)
        .unwrap_or_else(|error| panic_error(frame, error));
    let result = new_result(frame, length);
    let output = core::slice::from_raw_parts_mut(string_bytes_mut(result), length);
    if doria_unicode::write_first_case(source, mapping, output).is_err() {
        panic_error(frame, StringError::ResultTooLarge);
    }
    result
}

#[no_mangle]
pub unsafe extern "C" fn dr_v1_string_grapheme_length(value: *const DrStringV1) -> usize {
    doria_unicode::grapheme_count(text(value))
}

#[no_mangle]
pub unsafe extern "C" fn dr_v1_string_is_empty(value: *const DrStringV1) -> u8 {
    u8::from(crate::dr_v1_string_byte_length(value) == 0)
}

#[no_mangle]
pub unsafe extern "C" fn dr_v1_string_to_bytes(value: *const DrStringV1) -> *mut DrBytesV1 {
    bytes::copy(
        crate::dr_v1_string_data(value),
        crate::dr_v1_string_byte_length(value),
    )
}

unsafe fn trim(
    frame: *const DrStackFrameV2,
    value: *const DrStringV1,
    mode: TrimMode,
) -> *mut DrStringV1 {
    let source = text(value);
    let range = doria_unicode::trim_range(source, mode);
    if range.start == 0 && range.end == source.len() {
        return dr_v1_string_retain(value.cast_mut());
    }
    copy_range(frame, source, range)
}

#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_trim(
    frame: *const DrStackFrameV2,
    value: *const DrStringV1,
) -> *mut DrStringV1 {
    trim(frame, value, TrimMode::Both)
}

#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_trim_start(
    frame: *const DrStackFrameV2,
    value: *const DrStringV1,
) -> *mut DrStringV1 {
    trim(frame, value, TrimMode::Start)
}

#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_trim_end(
    frame: *const DrStackFrameV2,
    value: *const DrStringV1,
) -> *mut DrStringV1 {
    trim(frame, value, TrimMode::End)
}

#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_lower(
    frame: *const DrStackFrameV2,
    value: *const DrStringV1,
) -> *mut DrStringV1 {
    transform(frame, value, CaseMapping::Lower)
}

#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_upper(
    frame: *const DrStackFrameV2,
    value: *const DrStringV1,
) -> *mut DrStringV1 {
    transform(frame, value, CaseMapping::Upper)
}

#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_lower_first(
    frame: *const DrStackFrameV2,
    value: *const DrStringV1,
) -> *mut DrStringV1 {
    transform_first(frame, value, CaseMapping::Lower)
}

#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_upper_first(
    frame: *const DrStackFrameV2,
    value: *const DrStringV1,
) -> *mut DrStringV1 {
    transform_first(frame, value, CaseMapping::Upper)
}

#[no_mangle]
pub unsafe extern "C" fn dr_v1_string_contains(
    text_value: *const DrStringV1,
    needle: *const DrStringV1,
) -> u8 {
    u8::from(doria_unicode::contains(text(text_value), text(needle)))
}

#[no_mangle]
pub unsafe extern "C" fn dr_v1_string_starts_with(
    text_value: *const DrStringV1,
    prefix: *const DrStringV1,
) -> u8 {
    u8::from(doria_unicode::starts_with(text(text_value), text(prefix)))
}

#[no_mangle]
pub unsafe extern "C" fn dr_v1_string_ends_with(
    text_value: *const DrStringV1,
    suffix: *const DrStringV1,
) -> u8 {
    u8::from(doria_unicode::ends_with(text(text_value), text(suffix)))
}

unsafe fn ignore_case_predicate(
    frame: *const DrStackFrameV2,
    text_value: *const DrStringV1,
    needle: *const DrStringV1,
    predicate: impl FnOnce(&str, &str) -> Result<bool, StringError>,
) -> u8 {
    u8::from(
        predicate(text(text_value), text(needle)).unwrap_or_else(|error| panic_error(frame, error)),
    )
}

#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_contains_ignore_case(
    frame: *const DrStackFrameV2,
    text_value: *const DrStringV1,
    needle: *const DrStringV1,
) -> u8 {
    ignore_case_predicate(
        frame,
        text_value,
        needle,
        doria_unicode::contains_ignore_case,
    )
}

#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_starts_with_ignore_case(
    frame: *const DrStackFrameV2,
    text_value: *const DrStringV1,
    prefix: *const DrStringV1,
) -> u8 {
    ignore_case_predicate(
        frame,
        text_value,
        prefix,
        doria_unicode::starts_with_ignore_case,
    )
}

#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_ends_with_ignore_case(
    frame: *const DrStackFrameV2,
    text_value: *const DrStringV1,
    suffix: *const DrStringV1,
) -> u8 {
    ignore_case_predicate(
        frame,
        text_value,
        suffix,
        doria_unicode::ends_with_ignore_case,
    )
}

#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_equals_ignore_case(
    frame: *const DrStackFrameV2,
    left: *const DrStringV1,
    right: *const DrStringV1,
) -> u8 {
    let left = text(left);
    let right = text(right);
    let scratch_length = doria_unicode::case_output_length(left, CaseMapping::Fold)
        .unwrap_or_else(|error| panic_error(frame, error));
    let scratch = if scratch_length == 0 {
        ptr::null_mut()
    } else {
        let scratch = allocate(scratch_length);
        if scratch.is_null() {
            panic_error(frame, StringError::ResultTooLarge);
        }
        scratch
    };
    let mut empty_scratch = [];
    let scratch_slice = if scratch_length == 0 {
        empty_scratch.as_mut_slice()
    } else {
        core::slice::from_raw_parts_mut(scratch, scratch_length)
    };
    let equal = doria_unicode::equals_ignore_case(left, right, scratch_slice)
        .unwrap_or_else(|error| panic_error(frame, error));
    if !scratch.is_null() {
        deallocate(scratch);
    }
    u8::from(equal)
}

unsafe fn index_of(
    text_value: *const DrStringV1,
    needle: *const DrStringV1,
    found: *mut u8,
    last: bool,
) -> i64 {
    let result = if last {
        doria_unicode::last_index_of(text(text_value), text(needle))
    } else {
        doria_unicode::first_index_of(text(text_value), text(needle))
    };
    if let Some(index) = result.and_then(|index| i64::try_from(index).ok()) {
        *found = 1;
        index
    } else {
        *found = 0;
        0
    }
}

unsafe fn index_of_ignore_case(
    frame: *const DrStackFrameV2,
    text_value: *const DrStringV1,
    needle: *const DrStringV1,
    found: *mut u8,
    last: bool,
) -> i64 {
    let result = if last {
        doria_unicode::last_index_of_ignore_case(text(text_value), text(needle))
    } else {
        doria_unicode::first_index_of_ignore_case(text(text_value), text(needle))
    }
    .unwrap_or_else(|error| panic_error(frame, error));
    if let Some(index) = result.and_then(|index| i64::try_from(index).ok()) {
        *found = 1;
        index
    } else {
        *found = 0;
        0
    }
}

#[no_mangle]
pub unsafe extern "C" fn dr_v1_string_index_of(
    text_value: *const DrStringV1,
    needle: *const DrStringV1,
    found: *mut u8,
) -> i64 {
    index_of(text_value, needle, found, false)
}

#[no_mangle]
pub unsafe extern "C" fn dr_v1_string_last_index_of(
    text_value: *const DrStringV1,
    needle: *const DrStringV1,
    found: *mut u8,
) -> i64 {
    index_of(text_value, needle, found, true)
}

#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_index_of_ignore_case(
    frame: *const DrStackFrameV2,
    text_value: *const DrStringV1,
    needle: *const DrStringV1,
    found: *mut u8,
) -> i64 {
    index_of_ignore_case(frame, text_value, needle, found, false)
}

#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_last_index_of_ignore_case(
    frame: *const DrStackFrameV2,
    text_value: *const DrStringV1,
    needle: *const DrStringV1,
    found: *mut u8,
) -> i64 {
    index_of_ignore_case(frame, text_value, needle, found, true)
}

#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_count_occurrences(
    frame: *const DrStackFrameV2,
    text_value: *const DrStringV1,
    needle: *const DrStringV1,
) -> i64 {
    let count = doria_unicode::count_occurrences(text(text_value), text(needle))
        .unwrap_or_else(|error| panic_error(frame, error));
    i64::try_from(count).unwrap_or_else(|_| panic_error(frame, StringError::ResultTooLarge))
}

#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_replace(
    frame: *const DrStackFrameV2,
    text_value: *const DrStringV1,
    search: *const DrStringV1,
    replacement: *const DrStringV1,
) -> *mut DrStringV1 {
    let source = text(text_value);
    let search = text(search);
    let replacement = text(replacement);
    let length = doria_unicode::replacement_output_length(source, search, replacement)
        .unwrap_or_else(|error| panic_error(frame, error));
    let result = new_result(frame, length);
    let output = core::slice::from_raw_parts_mut(string_bytes_mut(result), length);
    doria_unicode::write_replacement(source, search, replacement, output)
        .unwrap_or_else(|error| panic_error(frame, error));
    result
}

#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_split(
    frame: *const DrStackFrameV2,
    text_value: *const DrStringV1,
    separator: *const DrStringV1,
) -> *mut DrCollectionV1 {
    let source = text(text_value);
    let separator = text(separator);
    let count = doria_unicode::split_field_count(source, separator)
        .unwrap_or_else(|error| panic_error(frame, error));
    let result = collection::new(count, false, false, mem::size_of::<*mut DrStringV1>() as u8);
    doria_unicode::split_fields(source, separator, |field| {
        let value = crate::dr_v1_string_from_utf8(field.as_ptr(), field.len());
        collection::push(result, value as u64);
    });
    result
}

#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_join(
    frame: *const DrStackFrameV2,
    separator: *const DrStringV1,
    values: *const DrCollectionV1,
) -> *mut DrStringV1 {
    let separator_length = crate::dr_v1_string_byte_length(separator);
    let count = collection::length(values);
    let mut length = doria_unicode::checked_mul(separator_length, count.saturating_sub(1))
        .unwrap_or_else(|error| panic_error(frame, error));
    for index in 0..count {
        let value = collection::value_at(frame, values, index) as usize as *const DrStringV1;
        length = doria_unicode::checked_add(length, crate::dr_v1_string_byte_length(value))
            .unwrap_or_else(|error| panic_error(frame, error));
    }
    let result = new_result(frame, length);
    let mut cursor = 0usize;
    for index in 0..count {
        if index != 0 {
            ptr::copy_nonoverlapping(
                crate::dr_v1_string_data(separator),
                string_bytes_mut(result).add(cursor),
                separator_length,
            );
            cursor += separator_length;
        }
        let value = collection::value_at(frame, values, index) as usize as *const DrStringV1;
        let value_length = crate::dr_v1_string_byte_length(value);
        ptr::copy_nonoverlapping(
            crate::dr_v1_string_data(value),
            string_bytes_mut(result).add(cursor),
            value_length,
        );
        cursor += value_length;
    }
    result
}

#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_slice(
    frame: *const DrStackFrameV2,
    value: *const DrStringV1,
    start: i64,
    length: i64,
    has_length: u8,
) -> *mut DrStringV1 {
    let source = text(value);
    let range = doria_unicode::slice_range(source, start, (has_length != 0).then_some(length))
        .unwrap_or_else(|error| match error {
            StringError::SliceLengthNegative => dr_v2_panic_signed_fact(
                frame,
                b"P1201".as_ptr(),
                5,
                doria_diagnostic_catalogue::STRING_SLICE_LENGTH_FACT.as_ptr(),
                doria_diagnostic_catalogue::STRING_SLICE_LENGTH_FACT.len(),
                length,
            ),
            error => panic_error(frame, error),
        });
    copy_range(frame, source, range)
}

#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_repeat(
    frame: *const DrStackFrameV2,
    value: *const DrStringV1,
    count: i64,
) -> *mut DrStringV1 {
    let source = text(value);
    let length =
        doria_unicode::repetition_output_length(source, count).unwrap_or_else(
            |error| match error {
                StringError::RepetitionCountNegative => dr_v2_panic_signed_fact(
                    frame,
                    b"P1204".as_ptr(),
                    5,
                    doria_diagnostic_catalogue::STRING_REPETITION_COUNT_FACT.as_ptr(),
                    doria_diagnostic_catalogue::STRING_REPETITION_COUNT_FACT.len(),
                    count,
                ),
                error => panic_error(frame, error),
            },
        );
    let result = new_result(frame, length);
    let output = core::slice::from_raw_parts_mut(string_bytes_mut(result), length);
    doria_unicode::write_repetition(source, count, output)
        .unwrap_or_else(|error| panic_error(frame, error));
    result
}

unsafe fn pad(
    frame: *const DrStackFrameV2,
    value: *const DrStringV1,
    target_length: i64,
    padding: *const DrStringV1,
    side: PadSide,
) -> *mut DrStringV1 {
    let source = text(value);
    let padding = text(padding);
    let panic = |error| match error {
        StringError::PaddingTextEmpty => dr_v2_panic_string_padding_empty(
            frame,
            u8::from(side == PadSide::Start),
            value,
            doria_unicode::grapheme_count(source),
            target_length,
            doria_unicode::grapheme_count(padding),
        ),
        StringError::PaddingLengthNegative => dr_v2_panic_signed_fact(
            frame,
            b"P1202".as_ptr(),
            5,
            doria_diagnostic_catalogue::STRING_PADDING_REQUESTED_LENGTH_FACT.as_ptr(),
            doria_diagnostic_catalogue::STRING_PADDING_REQUESTED_LENGTH_FACT.len(),
            target_length,
        ),
        error => panic_error(frame, error),
    };
    let length =
        doria_unicode::padding_output_length(source, target_length, padding).unwrap_or_else(panic);
    let result = new_result(frame, length);
    let output = core::slice::from_raw_parts_mut(string_bytes_mut(result), length);
    doria_unicode::write_padding(source, target_length, padding, side, output)
        .unwrap_or_else(panic);
    result
}

#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_pad_start(
    frame: *const DrStackFrameV2,
    value: *const DrStringV1,
    target_length: i64,
    padding: *const DrStringV1,
) -> *mut DrStringV1 {
    pad(frame, value, target_length, padding, PadSide::Start)
}

#[no_mangle]
pub unsafe extern "C" fn dr_v2_string_pad_end(
    frame: *const DrStackFrameV2,
    value: *const DrStringV1,
    target_length: i64,
    padding: *const DrStringV1,
) -> *mut DrStringV1 {
    pad(frame, value, target_length, padding, PadSide::End)
}

#[no_mangle]
pub unsafe extern "C" fn dr_v1_string_from_bytes(value: *const DrBytesV1) -> *mut DrStringV1 {
    let length = bytes::length(value);
    let source = core::slice::from_raw_parts(bytes::data(value), length);
    if core::str::from_utf8(source).is_err() {
        return ptr::null_mut();
    }
    crate::dr_v1_string_from_utf8(source.as_ptr(), source.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe fn string(value: &str) -> *mut DrStringV1 {
        crate::dr_v1_string_from_utf8(value.as_ptr(), value.len())
    }

    unsafe fn read(value: *const DrStringV1) -> &'static str {
        text(value)
    }

    #[test]
    fn runtime_string_operations_preserve_unicode_and_ownership() {
        unsafe {
            let family = string("👨‍👩‍👧‍👦");
            assert_eq!(dr_v1_string_grapheme_length(family), 1);
            assert_eq!(crate::dr_v1_string_byte_length(family), 25);
            assert_eq!(dr_v1_string_is_empty(family), 0);

            let source = string("\u{2003}Straße 👍🏾\u{00a0}");
            let trimmed = dr_v2_string_trim(ptr::null(), source);
            let upper = dr_v2_string_upper(ptr::null(), trimmed);
            assert_eq!(read(trimmed), "Straße 👍🏾");
            assert_eq!(read(upper), "STRASSE 👍🏾");

            let folded = string("STRASSE 👍🏾");
            assert_eq!(
                dr_v2_string_equals_ignore_case(ptr::null(), trimmed, folded),
                1
            );
            let empty_left = string("");
            let empty_right = string("");
            assert_eq!(
                dr_v2_string_equals_ignore_case(ptr::null(), empty_left, empty_right),
                1
            );
            let first = string("ßTRASSE");
            let upper_first = dr_v2_string_upper_first(ptr::null(), first);
            assert_eq!(read(upper_first), "SSTRASSE");

            let case_needle = string("strasse");
            let mut found = 0;
            assert_eq!(
                dr_v2_string_index_of_ignore_case(ptr::null(), trimmed, case_needle, &mut found,),
                0
            );
            assert_eq!(found, 1);
            let repeated = string("aaaa");
            let occurrence = string("aa");
            assert_eq!(
                dr_v2_string_count_occurrences(ptr::null(), repeated, occurrence),
                2
            );

            crate::dr_v1_string_release(family);
            crate::dr_v1_string_release(source);
            crate::dr_v1_string_release(trimmed);
            crate::dr_v1_string_release(upper);
            crate::dr_v1_string_release(folded);
            crate::dr_v1_string_release(empty_left);
            crate::dr_v1_string_release(empty_right);
            crate::dr_v1_string_release(first);
            crate::dr_v1_string_release(upper_first);
            crate::dr_v1_string_release(case_needle);
            crate::dr_v1_string_release(repeated);
            crate::dr_v1_string_release(occurrence);
        }
    }

    #[test]
    fn split_join_and_bytes_conversion_create_independent_owned_values() {
        unsafe {
            let source = string("one,👍🏾,three");
            let separator = string(",");
            let fields = dr_v2_string_split(ptr::null(), source, separator);
            assert_eq!(collection::length(fields), 3);
            let joined = dr_v2_string_join(ptr::null(), separator, fields);
            assert_eq!(read(joined), "one,👍🏾,three");

            let bytes = dr_v1_string_to_bytes(source);
            bytes::set(ptr::null(), bytes, 0, b'O');
            assert_eq!(read(source), "one,👍🏾,three");
            let round_trip = dr_v1_string_from_bytes(bytes);
            assert_eq!(read(round_trip), "One,👍🏾,three");

            for index in 0..collection::length(fields) {
                crate::dr_v1_string_release(
                    collection::value_at(ptr::null(), fields, index) as usize as *mut DrStringV1
                );
            }
            collection::free(fields);
            bytes::free(bytes);
            crate::dr_v1_string_release(round_trip);
            crate::dr_v1_string_release(joined);
            crate::dr_v1_string_release(separator);
            crate::dr_v1_string_release(source);
        }
    }

    #[test]
    fn legacy_length_symbol_forwards_to_explicit_byte_length() {
        unsafe {
            let value = string("👍🏾");
            assert_eq!(crate::dr_v1_string_length(value), 8);
            assert_eq!(crate::dr_v1_string_byte_length(value), 8);
            assert_eq!(dr_v1_string_grapheme_length(value), 1);
            crate::dr_v1_string_release(value);
        }
    }

    #[test]
    fn invalid_utf8_bytes_produce_an_absent_string_without_aliasing() {
        unsafe {
            let invalid = [0xff, 0xfe];
            let bytes = bytes::copy(invalid.as_ptr(), invalid.len());
            assert!(dr_v1_string_from_bytes(bytes).is_null());
            bytes::set(ptr::null(), bytes, 0, b'D');
            assert!(dr_v1_string_from_bytes(bytes).is_null());
            bytes::free(bytes);
        }
    }
}
