use doria_diagnostic_catalogue::{
    BYTE_BUFFER_LENGTH_FACT, BYTE_REGION_OFFSET_FACT, BYTE_REGION_REQUESTED_LENGTH_FACT,
};

use crate::{
    bytes, dr_v2_panic_code_with_facts, dr_v2_panic_index_out_of_bounds, DrBytesV1,
    DrRuntimeFactV2, DrStackFrameV2, RUNTIME_FACT_SIGNED, RUNTIME_FACT_UNSIGNED,
};

fn checked_offset(source_length: usize, offset: i64, length: i64) -> Option<usize> {
    let offset = usize::try_from(offset).ok()?;
    let length = usize::try_from(length).ok()?;
    (offset <= source_length && length <= source_length - offset).then_some(offset)
}

/// Forms a bounded borrow without allocating storage or retaining the owner.
/// The compiler carries the returned pointer together with the requested length.
///
/// # Safety
/// `bytes` must remain live for the region's full lifetime. Generated ownership
/// proofs must enforce readonly or exclusive writable access to its storage.
/// `frame` must be null or a valid generated frame chain.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_bytes_region(
    frame: *const DrStackFrameV2,
    bytes: *const DrBytesV1,
    offset: i64,
    length: i64,
) -> *mut u8 {
    dr_v2_byte_region_subregion(
        frame,
        bytes::data(bytes).cast_mut(),
        bytes::length(bytes),
        offset,
        length,
    )
}

/// Narrows a region using subtraction-based bounds checks, without overflow.
///
/// # Safety
/// `data` must describe live storage for `source_length` bytes, or may be null
/// when that length is zero. The compiler must preserve the source loan for the
/// returned region and enforce its access mode. `frame` must be null or valid.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_byte_region_subregion(
    frame: *const DrStackFrameV2,
    data: *mut u8,
    source_length: usize,
    offset: i64,
    length: i64,
) -> *mut u8 {
    let Some(offset) = checked_offset(source_length, offset, length) else {
        region_panic(frame, source_length, offset, length);
    };
    // Null is permitted only for empty storage. Avoid even zero-offset pointer
    // arithmetic on it; nonempty sources may also form an empty end region.
    if offset == 0 {
        data
    } else {
        data.add(offset)
    }
}

/// # Safety
/// The pointer/length pair must be a live compiler-proven readonly byte borrow.
/// `frame` must be null or a valid generated frame chain.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_byte_region_get(
    frame: *const DrStackFrameV2,
    data: *const u8,
    length: usize,
    index: i64,
) -> u8 {
    let Some(index) = checked_offset(length, index, 1) else {
        dr_v2_panic_index_out_of_bounds(frame, b"P1301".as_ptr(), 5, index, length);
    };
    *data.add(index)
}

/// # Safety
/// The pointer/length pair must be a live compiler-proven exclusive byte borrow.
/// `frame` must be null or a valid generated frame chain.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_byte_region_set(
    frame: *const DrStackFrameV2,
    data: *mut u8,
    length: usize,
    index: i64,
    value: u8,
) {
    let Some(index) = checked_offset(length, index, 1) else {
        dr_v2_panic_index_out_of_bounds(frame, b"P1301".as_ptr(), 5, index, length);
    };
    *data.add(index) = value;
}

/// `WritableByteRegion::copyFrom`: copies the leading bytes of the source
/// region into the destination and returns the count, the smaller length.
/// Overlap is impossible under the borrow rules; memmove keeps it defined.
///
/// # Safety
/// Each pointer/length pair must be a live compiler-proven byte borrow, the
/// destination exclusive. A pointer may be null only when its length is zero.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_byte_region_copy(
    destination: *mut u8,
    destination_length: usize,
    source: *const u8,
    source_length: usize,
) -> usize {
    let count = destination_length.min(source_length);
    if count != 0 {
        core::ptr::copy(source, destination, count);
    }
    count
}

/// `WritableByteRegion::copyWithin`: moves `length` bytes from `offset` to
/// `destination` inside one region. The source range is checked first, then
/// the destination range, each with the subregion panic (P1304). The ranges
/// may overlap; the move is a memmove.
///
/// # Safety
/// The pointer/length pair must be a live compiler-proven exclusive byte
/// borrow (null only when empty). `frame` must be null or a valid frame chain.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_byte_region_copy_within(
    frame: *const DrStackFrameV2,
    data: *mut u8,
    region_length: usize,
    offset: i64,
    length: i64,
    destination: i64,
) {
    let Some(from) = checked_offset(region_length, offset, length) else {
        region_panic(frame, region_length, offset, length);
    };
    let Some(to) = checked_offset(region_length, destination, length) else {
        region_panic(frame, region_length, destination, length);
    };
    let count = length as usize;
    if count != 0 {
        core::ptr::copy(data.add(from), data.add(to), count);
    }
}

unsafe fn region_panic(
    frame: *const DrStackFrameV2,
    source_length: usize,
    offset: i64,
    length: i64,
) -> ! {
    let facts = [
        DrRuntimeFactV2 {
            name: BYTE_REGION_OFFSET_FACT.as_ptr(),
            name_length: BYTE_REGION_OFFSET_FACT.len(),
            kind: RUNTIME_FACT_SIGNED,
            value: offset as u64,
            value_length: 0,
        },
        DrRuntimeFactV2 {
            name: BYTE_REGION_REQUESTED_LENGTH_FACT.as_ptr(),
            name_length: BYTE_REGION_REQUESTED_LENGTH_FACT.len(),
            kind: RUNTIME_FACT_SIGNED,
            value: length as u64,
            value_length: 0,
        },
        DrRuntimeFactV2 {
            name: BYTE_BUFFER_LENGTH_FACT.as_ptr(),
            name_length: BYTE_BUFFER_LENGTH_FACT.len(),
            kind: RUNTIME_FACT_UNSIGNED,
            value: source_length as u64,
            value_length: 0,
        },
    ];
    dr_v2_panic_code_with_facts(
        frame,
        b"P1304".as_ptr(),
        5,
        core::ptr::null(),
        0,
        facts.as_ptr(),
        facts.len(),
    );
}

#[cfg(test)]
mod tests {
    use core::ptr;

    use super::*;

    #[test]
    fn bounds_include_empty_end_and_reject_negative_or_overflowing_ranges() {
        for source in 0..32 {
            for offset in -1..=33 {
                for length in -1..=33 {
                    let valid = offset >= 0 && length >= 0 && offset + length <= source as i64;
                    assert_eq!(checked_offset(source, offset, length).is_some(), valid);
                }
            }
        }
        assert_eq!(checked_offset(0, 0, 0), Some(0));
        assert_eq!(checked_offset(1, i64::MAX, i64::MAX), None);
        assert_eq!(checked_offset(1, i64::MIN, 0), None);
        assert_eq!(checked_offset(1, 0, i64::MIN), None);
        if usize::BITS >= 64 {
            assert_eq!(
                checked_offset(i64::MAX as usize, i64::MAX, 0),
                Some(i64::MAX as usize)
            );
            assert_eq!(checked_offset(i64::MAX as usize, i64::MAX, 1), None);
        }
    }

    #[test]
    fn regions_and_subregions_reuse_the_original_storage() {
        unsafe {
            let owner = bytes::zeroed(8);
            let region = dr_v2_bytes_region(ptr::null(), owner, 2, 4);
            assert_eq!(region, bytes::data(owner).add(2).cast_mut());
            dr_v2_byte_region_set(ptr::null(), region, 4, 1, 255);
            assert_eq!(bytes::get(ptr::null(), owner, 3), 255);
            let subregion = dr_v2_byte_region_subregion(ptr::null(), region, 4, 1, 2);
            assert_eq!(dr_v2_byte_region_get(ptr::null(), subregion, 2, 0), 255);
            dr_v2_byte_region_set(ptr::null(), subregion, 2, 1, 42);
            assert_eq!(bytes::get(ptr::null(), owner, 4), 42);
            let end = dr_v2_byte_region_subregion(ptr::null(), region, 4, 4, 0);
            assert_eq!(end, region.add(4));
            bytes::free(owner);
        }
    }

    #[test]
    fn empty_storage_and_nested_empty_regions_never_offset_a_null_pointer() {
        unsafe {
            let owner = bytes::zeroed(0);
            let region = dr_v2_bytes_region(ptr::null(), owner, 0, 0);
            assert!(region.is_null());
            assert!(dr_v2_byte_region_subregion(ptr::null(), region, 0, 0, 0).is_null());
            bytes::free(owner);
        }
    }
}
