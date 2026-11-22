//! The companion library's side of a region entry (Decision 0148 section
//! 23, entry ABI version 1).
//!
//! A host that runs a program's unsafe regions in the native companion
//! library attaches itself once, before its first region call, through
//! `dr_v1_attach_region_host`. That records its thread as the thread running
//! Doria code, the host the companion's callback entries forward to, and the
//! panic sink: from then on a panic in the companion reaches the host, which
//! renders it as a native panic and ends the process with status 101.
//!
//! The slot accessors move content values between a slot record and the
//! companion's own values for both native backends, so the layout rules of a
//! content slot have one implementation: a slot is `{data, length}`, a
//! present value has a non-null `data` even when it is empty, and a null
//! `data` is a value that is absent or held by no one.

use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};

use crate::{
    bytes, collection, foreign_string, panic_catalogued, DrBytesV1, DrRuntimeFactV2,
    DrStackFrameV2, DrStringV1,
};

const ALLOCATION_FAILED: &[u8] = b"P1302";

/// The host a companion runs on. `call` is at offset 0, where companion
/// code finds it.
#[repr(C)]
pub struct DrRegionHostV1 {
    /// `call(host, operation, arguments, results)` runs host operation
    /// `operation` and returns 0.
    pub call: unsafe extern "C" fn(*mut DrRegionHostV1, u32, *mut u8, *mut u8) -> i32,
    /// Renders a panic of the companion and ends the process with status
    /// 101; it never returns. Its arguments are those of
    /// `dr_v2_panic_code_with_facts`, after their validation.
    pub panic: unsafe extern "C" fn(
        *mut DrRegionHostV1,
        *const DrStackFrameV2,
        *const u8,
        usize,
        *const u8,
        usize,
        *const DrRuntimeFactV2,
        usize,
    ),
}

/// One content slot: a string, a buffer, a region or a foreign string.
#[repr(C)]
pub struct DrRegionSlotV1 {
    pub data: *const u8,
    pub length: usize,
}

static HOST: AtomicPtr<DrRegionHostV1> = AtomicPtr::new(ptr::null_mut());

/// Attaches `host`: its thread becomes the thread running Doria code, and
/// companion panics and callbacks reach it.
///
/// # Safety
///
/// `host` points to a host that outlives every later region call.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_attach_region_host(host: *mut DrRegionHostV1) {
    HOST.store(host, Ordering::Relaxed);
    crate::c_entry::dr_v1_record_doria_thread();
}

/// The attached host, or null.
#[no_mangle]
pub extern "C" fn dr_v1_region_host() -> *mut DrRegionHostV1 {
    HOST.load(Ordering::Relaxed)
}

/// Hands a validated panic to the attached host, which never returns.
/// Returns when no host is attached.
pub(crate) unsafe fn report_to_host(
    frame: *const DrStackFrameV2,
    code: &[u8],
    message: *const u8,
    message_length: usize,
    facts: *const DrRuntimeFactV2,
    fact_count: usize,
) {
    let host = dr_v1_region_host();
    if !host.is_null() {
        ((*host).panic)(
            host,
            frame,
            code.as_ptr(),
            code.len(),
            message,
            message_length,
            facts,
            fact_count,
        );
    }
}

unsafe fn store(slot: *mut DrRegionSlotV1, data: *const u8, length: usize, owner: *const u8) {
    // A present value is never null, even when it is empty.
    let data = if data.is_null() { owner } else { data };
    slot.write(DrRegionSlotV1 { data, length });
}

unsafe fn absent(slot: *mut DrRegionSlotV1) {
    slot.write(DrRegionSlotV1 {
        data: ptr::null(),
        length: 0,
    });
}

/// Writes `value`, a string or null, into `slot`, lending its bytes.
///
/// # Safety
///
/// `slot` is writable and `value` null or a live string.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_region_slot_store_text(
    slot: *mut DrRegionSlotV1,
    value: *const DrStringV1,
) {
    if value.is_null() {
        return absent(slot);
    }
    store(
        slot,
        crate::dr_v1_string_data(value),
        crate::dr_v1_string_byte_length(value),
        value.cast(),
    );
}

/// The string `slot` holds, copied, or null when it holds none. Releases
/// `previous`, the value it replaces, after building it.
///
/// # Safety
///
/// `slot` is readable and holds valid UTF-8; `previous` is null or a live
/// string the caller owns.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_region_slot_load_text(
    slot: *const DrRegionSlotV1,
    previous: *mut DrStringV1,
) -> *mut DrStringV1 {
    let value = match (*slot).data {
        data if data.is_null() => ptr::null_mut(),
        data => crate::dr_v1_string_from_utf8(data, (*slot).length),
    };
    crate::dr_v1_string_release(previous);
    value
}

/// Writes `value`, a `Bytes` buffer or null, into `slot`.
///
/// # Safety
///
/// `slot` is writable and `value` null or a live buffer.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_region_slot_store_bytes(
    slot: *mut DrRegionSlotV1,
    value: *const DrBytesV1,
) {
    if value.is_null() {
        return absent(slot);
    }
    store(slot, bytes::data(value), bytes::length(value), value.cast());
}

/// The `Bytes` buffer `slot` holds, copied, or null. Frees `previous`.
///
/// # Safety
///
/// `slot` is readable; `previous` is null or a buffer the caller owns.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_region_slot_load_bytes(
    frame: *const DrStackFrameV2,
    slot: *const DrRegionSlotV1,
    previous: *mut DrBytesV1,
) -> *mut DrBytesV1 {
    let value = match (*slot).data {
        data if data.is_null() => ptr::null_mut(),
        data => {
            let value = bytes::copy(data, (*slot).length);
            if value.is_null() {
                panic_catalogued(frame, ALLOCATION_FAILED);
            }
            value
        }
    };
    bytes::free(previous);
    value
}

/// Writes `value`, a typed array of C-layout elements or null, into `slot`.
///
/// # Safety
///
/// `slot` is writable and `value` null or a live typed array.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_region_slot_store_array(
    slot: *mut DrRegionSlotV1,
    value: *const collection::DrCollectionV1,
) {
    if value.is_null() {
        return absent(slot);
    }
    let (data, _) = collection::element_storage(value);
    store(slot, data, collection::length(value), value.cast());
}

/// Fills `target`, an empty typed array the caller built, with the
/// elements `slot` holds.
///
/// # Safety
///
/// `slot` is readable and holds elements of `target`'s C layout.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_region_slot_load_array(
    frame: *const DrStackFrameV2,
    slot: *const DrRegionSlotV1,
    target: *mut collection::DrCollectionV1,
) {
    collection::replace_elements(frame, target, (*slot).data, (*slot).length);
}

/// Writes `value`, a `CString` or `Utf16String` buffer of `unit_bytes`-byte
/// code units or null, into `slot`: its code units without the NUL.
///
/// # Safety
///
/// `slot` is writable and `value` null or a live NUL-terminated buffer.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_region_slot_store_foreign_string(
    slot: *mut DrRegionSlotV1,
    value: *const u8,
    unit_bytes: usize,
) {
    if value.is_null() {
        return absent(slot);
    }
    let mut length = 0;
    while (0..unit_bytes).any(|byte| *value.add(length * unit_bytes + byte) != 0) {
        length += 1;
    }
    store(slot, value, length, value);
}

/// The foreign string `slot` holds, copied into a new NUL-terminated
/// buffer, or null. Frees `previous`.
///
/// # Safety
///
/// `slot` is readable; `previous` is null or a buffer the caller owns.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_region_slot_load_foreign_string(
    frame: *const DrStackFrameV2,
    slot: *const DrRegionSlotV1,
    unit_bytes: usize,
    previous: *mut u8,
) -> *mut u8 {
    let value = match (*slot).data {
        data if data.is_null() => ptr::null_mut(),
        data => {
            let length = (*slot).length;
            let buffer = foreign_string::buffer(frame, length, unit_bytes);
            ptr::copy_nonoverlapping(data, buffer, length * unit_bytes);
            ptr::write_bytes(buffer.add(length * unit_bytes), 0, unit_bytes);
            buffer
        }
    };
    foreign_string::dr_v1_foreign_string_free(previous);
    value
}
