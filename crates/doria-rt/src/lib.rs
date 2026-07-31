#![cfg_attr(all(not(test), panic = "abort"), no_std)]

// Linked runtime artifacts always use panic=abort; unwind-mode builds exist only for check/test metadata.

use core::ffi::c_void;
use core::mem;
use core::ptr;
use unicode_width::UnicodeWidthChar;

mod bytes;
mod collection;
mod device_io;
mod entry_args;
mod file_io;
mod line_io;
mod mixed;
mod string_ops;

use device_io::{StandardStream, WriteOutcome};

const PANIC_STATUS: i32 = 101;
#[cfg(unix)]
const SIGPIPE: i32 = 13;
#[cfg(unix)]
const SIG_IGN: usize = 1;

#[cfg(all(not(test), panic = "abort"))]
struct RuntimeAllocator;

#[cfg(all(not(test), panic = "abort"))]
unsafe impl core::alloc::GlobalAlloc for RuntimeAllocator {
    unsafe fn alloc(&self, layout: core::alloc::Layout) -> *mut u8 {
        let header = mem::size_of::<*mut u8>();
        let Some(byte_length) = layout
            .size()
            .checked_add(layout.align() - 1)
            .and_then(|length| length.checked_add(header))
        else {
            return ptr::null_mut();
        };
        let allocation = allocate(byte_length);
        if allocation.is_null() {
            return allocation;
        }
        let start = allocation.add(header);
        let aligned = start
            .add(layout.align() - 1)
            .map_addr(|address| address & !(layout.align() - 1));
        aligned.sub(header).cast::<*mut u8>().write(allocation);
        aligned
    }

    unsafe fn dealloc(&self, memory: *mut u8, _layout: core::alloc::Layout) {
        let allocation = memory
            .sub(mem::size_of::<*mut u8>())
            .cast::<*mut u8>()
            .read();
        deallocate(allocation);
    }

    unsafe fn realloc(
        &self,
        memory: *mut u8,
        layout: core::alloc::Layout,
        new_size: usize,
    ) -> *mut u8 {
        let Ok(new_layout) = core::alloc::Layout::from_size_align(new_size, layout.align()) else {
            return ptr::null_mut();
        };
        let replacement = self.alloc(new_layout);
        if !replacement.is_null() {
            ptr::copy_nonoverlapping(memory, replacement, layout.size().min(new_size));
            self.dealloc(memory, layout);
        }
        replacement
    }
}

#[cfg(all(not(test), panic = "abort"))]
#[global_allocator]
static RUNTIME_ALLOCATOR: RuntimeAllocator = RuntimeAllocator;

#[repr(C)]
pub struct DrStackFrameV2 {
    pub parent: *const DrStackFrameV2,
    pub function_name: *const u8,
    pub function_name_length: usize,
    pub source_path: *const u8,
    pub source_path_length: usize,
    pub source_text: *const u8,
    pub source_text_length: usize,
    pub function_span_start: usize,
    pub function_span_end: usize,
    pub active_span_start: usize,
    pub active_span_end: usize,
}

#[repr(C)]
pub struct DrRuntimeFactV2 {
    pub name: *const u8,
    pub name_length: usize,
    /// 1 = signed, 2 = unsigned, 3 = boolean, 4 = static string.
    pub kind: usize,
    pub value: u64,
    pub value_length: usize,
}

/// Opaque shared-ownership control block.
///
/// The class payload remains a separately allocated, headerless native class
/// value. Weak references keep this block alive after the payload is destroyed.
#[repr(C)]
pub struct DrSharedControlV1 {
    strong_references: usize,
    weak_references: usize,
    payload: *mut u8,
    drop_payload: unsafe extern "C" fn(*const DrStackFrameV2, *mut u8),
}

/// Opaque writable shared-ownership control block.
///
/// Unlike the readonly family, every writable-family handle and access object
/// observes the access state stored in this single per-allocation block.
#[repr(C)]
pub struct DrWritableSharedControlV1 {
    strong_references: usize,
    weak_references: usize,
    readonly_accesses: usize,
    writable_access_active: bool,
    payload: *mut u8,
    drop_payload: unsafe extern "C" fn(*const DrStackFrameV2, *mut u8),
}

/// Opaque outside doria-rt. Bytes immediately follow this header.
#[repr(C)]
pub struct DrStringV1 {
    references: usize,
    byte_length: usize,
}

pub use bytes::DrBytesV1;
pub use collection::{
    DrCollectionV1, DR_COLLECTION_CAPACITY_OFFSET, DR_COLLECTION_FIXED_OFFSET,
    DR_COLLECTION_KEYED_OFFSET, DR_COLLECTION_KEYS_OFFSET, DR_COLLECTION_LENGTH_OFFSET,
    DR_COLLECTION_VALUES_OFFSET, DR_COLLECTION_VALUE_WIDTH_OFFSET,
};
pub use mixed::DrMixedV1;

/// # Safety
///
/// `source` must be readable for `length` bytes, or may be null when `length` is zero.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_bytes_copy(source: *const u8, length: usize) -> *mut DrBytesV1 {
    bytes::copy(source, length)
}

/// # Safety
///
/// `value` must be null or a live, uniquely owned byte buffer and must not be used afterward.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_bytes_free(value: *mut DrBytesV1) {
    bytes::free(value)
}

/// # Safety
///
/// `value` must point to a live byte buffer.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_bytes_length(value: *const DrBytesV1) -> usize {
    bytes::length(value)
}

/// # Safety
///
/// `value` must point to a live byte buffer and `current_frame` must be null or valid.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_bytes_get(
    current_frame: *const DrStackFrameV2,
    value: *const DrBytesV1,
    index: usize,
) -> u8 {
    bytes::get(current_frame, value, index)
}

/// # Safety
///
/// `value` must be a uniquely borrowed live byte buffer and `current_frame` must be null or valid.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_bytes_set(
    current_frame: *const DrStackFrameV2,
    value: *mut DrBytesV1,
    index: usize,
    byte: u8,
) {
    bytes::set(current_frame, value, index, byte)
}

/// # Safety
///
/// `left` and `right` must point to live byte buffers.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_bytes_equal(left: *const DrBytesV1, right: *const DrBytesV1) -> u8 {
    u8::from(bytes::equal(left, right))
}

/// # Safety
///
/// `collection` must point to a live canonical `uint8[]` runtime collection.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_bytes_from_collection(
    collection: *const DrCollectionV1,
) -> *mut DrBytesV1 {
    let length = collection::length(collection);
    let data = allocate(length);
    if length != 0 && data.is_null() {
        panic_catalogued(ptr::null(), b"P1302");
    }
    for index in 0..length {
        *data.add(index) = collection::value_at(ptr::null(), collection, index) as u8;
    }
    bytes::from_owned(data, length)
}

/// # Safety
///
/// `value` must point to a live byte buffer.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_bytes_to_collection(value: *const DrBytesV1) -> *mut DrCollectionV1 {
    let length = bytes::length(value);
    let collection = collection::new(length, false, true, mem::size_of::<u8>() as u8);
    for index in 0..length {
        collection::set_at(
            ptr::null(),
            collection,
            index,
            u64::from(bytes::get(ptr::null(), value, index)),
        );
    }
    collection
}

/// Allocates collection storage for generated Doria code.
///
/// # Safety
///
/// `keyed` and `fixed` must be canonical boolean bytes, and `value_width` must
/// be 1, 2, 4, or 8. The returned pointer must be released exactly once with
/// `dr_v1_collection_free`.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_collection_new(
    length: usize,
    keyed: u8,
    fixed: u8,
    value_width: u8,
) -> *mut DrCollectionV1 {
    collection::new(length, keyed != 0, fixed != 0, value_width)
}

/// Allocates a sequence containing `count` bitwise copies of `value`.
///
/// # Safety
///
/// `current_frame` must be null or a valid generated frame chain, and `fixed`
/// must be a canonical boolean byte. The returned pointer must be released
/// exactly once with `dr_v1_collection_free`.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_collection_fill_word(
    current_frame: *const DrStackFrameV2,
    value: u64,
    count: i64,
    fixed: u8,
    value_width: u8,
) -> *mut DrCollectionV1 {
    if count < 0 {
        panic_catalogued(current_frame, b"P1311");
    }
    collection::fill_word(
        current_frame,
        value,
        count as usize,
        fixed != 0,
        value_width,
    )
}

/// Allocates a sequence containing `count` retained references to `value`.
///
/// # Safety
///
/// `current_frame` must be null or a valid generated frame chain, `value` must
/// be null or a live Doria string, and `fixed` must be a canonical boolean
/// byte. Every retained slot is released when the collection is dropped.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_collection_fill_string(
    current_frame: *const DrStackFrameV2,
    value: *mut DrStringV1,
    count: i64,
    fixed: u8,
) -> *mut DrCollectionV1 {
    if count < 0 {
        panic_catalogued(current_frame, b"P1311");
    }
    collection::fill_string(current_frame, value, count as usize, fixed != 0)
}

/// # Safety
///
/// `collection` must be null or a live pointer returned by
/// `dr_v1_collection_new`, and it must not be used after this call.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_collection_free(collection: *mut DrCollectionV1) {
    collection::free(collection)
}

/// # Safety
///
/// `collection` must point to a live collection allocation.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_collection_length(collection: *const DrCollectionV1) -> usize {
    collection::length(collection)
}

/// # Safety
///
/// `collection` must be a uniquely borrowed live growable collection, and
/// `value` must use the element representation declared by generated MIR.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_collection_push(collection: *mut DrCollectionV1, value: u64) {
    collection::push(collection, value)
}

/// # Safety
///
/// `current_frame` must be null or a valid generated frame chain.
/// `collection` must be a uniquely borrowed live growable collection, and
/// `value` must use its element representation.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_collection_insert_at(
    current_frame: *const DrStackFrameV2,
    collection: *mut DrCollectionV1,
    index: usize,
    value: u64,
) {
    collection::insert_at(current_frame, collection, index, value)
}

/// # Safety
///
/// `current_frame` must be null or a valid generated frame chain, and
/// `collection` must be a uniquely borrowed live growable collection.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_collection_remove_at(
    current_frame: *const DrStackFrameV2,
    collection: *mut DrCollectionV1,
    index: usize,
) -> u64 {
    collection::remove_at(current_frame, collection, index)
}

/// # Safety
///
/// `collection` must be a uniquely borrowed live growable collection and
/// `found` must point to writable storage for one byte.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_collection_pop(
    collection: *mut DrCollectionV1,
    found: *mut u8,
) -> u64 {
    collection::pop(collection, found)
}

/// # Safety
///
/// `current_frame` must be null or a valid generated frame chain, and
/// `collection` must point to a live collection allocation.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_collection_value_at(
    current_frame: *const DrStackFrameV2,
    collection: *const DrCollectionV1,
    index: usize,
) -> u64 {
    collection::value_at(current_frame, collection, index)
}

/// # Safety
///
/// `current_frame` must be null or a valid generated frame chain, and
/// `collection` must point to a live keyed collection allocation.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_collection_key_at(
    current_frame: *const DrStackFrameV2,
    collection: *const DrCollectionV1,
    index: usize,
) -> u64 {
    collection::key_at(current_frame, collection, index)
}

/// # Safety
///
/// `current_frame` must be null or a valid generated frame chain.
/// `collection` must be a uniquely borrowed live collection, and `value` must
/// use its element representation.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_collection_set_at(
    current_frame: *const DrStackFrameV2,
    collection: *mut DrCollectionV1,
    index: usize,
    value: u64,
) -> u64 {
    collection::set_at(current_frame, collection, index, value)
}

/// # Safety
///
/// `collection` must point to a live keyed collection. `key` must match
/// `key_kind`, and `found` must point to writable storage for one byte.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_collection_keyed_get(
    collection: *const DrCollectionV1,
    key: u64,
    key_kind: u8,
    found: *mut u8,
) -> u64 {
    collection::keyed_get(collection, key, key_kind, found)
}

/// # Safety
///
/// `collection` must be a uniquely borrowed live keyed collection. `key` and
/// `value` must match its declared representations, and `replaced` must point
/// to writable storage for one byte.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_collection_keyed_set(
    collection: *mut DrCollectionV1,
    key: u64,
    value: u64,
    key_kind: u8,
    replaced: *mut u8,
) -> u64 {
    collection::keyed_set(collection, key, value, key_kind, replaced)
}

/// # Safety
///
/// `collection` must point to a live keyed collection and `key` must match
/// `key_kind`.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_collection_keyed_has(
    collection: *const DrCollectionV1,
    key: u64,
    key_kind: u8,
) -> u8 {
    u8::from(collection::keyed_has(collection, key, key_kind))
}

/// # Safety
///
/// `collection` must be a uniquely borrowed live keyed collection. `key` must
/// match `key_kind`; `found` and `removed_key` must point to writable storage.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_collection_keyed_remove(
    collection: *mut DrCollectionV1,
    key: u64,
    key_kind: u8,
    found: *mut u8,
    removed_key: *mut u64,
) -> u64 {
    collection::keyed_remove(collection, key, key_kind, found, removed_key)
}

/// # Safety
///
/// `collection` must be a live collection, uniquely borrowed for mutating
/// access modes. `key` must match `key_kind`; `found` and `removed_key` must
/// point to writable storage.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_collection_nullable_access(
    collection: *mut DrCollectionV1,
    key: u64,
    key_kind: u8,
    access: u8,
    found: *mut u8,
    removed_key: *mut u64,
) -> u64 {
    collection::nullable_access(collection, key, key_kind, access, found, removed_key)
}

/// # Safety
///
/// `collection` must point to a live collection and `value` must match
/// `value_kind`.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_collection_contains(
    collection: *const DrCollectionV1,
    value: u64,
    value_kind: u8,
) -> u8 {
    u8::from(collection::contains(collection, value, value_kind))
}

/// # Safety
///
/// `payload` must use the representation implied by `tag` and `type_id`.
/// The returned box owns that payload until generated code explicitly drops it.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_mixed_new(tag: u8, type_id: u32, payload: u64) -> *mut DrMixedV1 {
    let value = mixed::new_owned(tag, type_id, payload);
    if value.is_null() {
        panic_catalogued(ptr::null(), b"P1320");
    }
    value
}

/// # Safety
///
/// `payload` must remain live for the returned shell's lifetime.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_mixed_new_borrowed(
    tag: u8,
    type_id: u32,
    payload: u64,
) -> *mut DrMixedV1 {
    let value = mixed::new_borrowed(tag, type_id, payload);
    if value.is_null() {
        panic_catalogued(ptr::null(), b"P1320");
    }
    value
}

/// # Safety
///
/// `value` must point to a live mixed box.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_mixed_clone_owned(value: *const DrMixedV1) -> *mut DrMixedV1 {
    let clone = mixed::clone_owned(value);
    if clone.is_null() {
        panic_catalogued(ptr::null(), b"P1322");
    }
    clone
}

/// Returns whether this released the final claim to an owned payload.
///
/// # Safety
///
/// `value` must point to a live mixed box.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_mixed_release_owned(value: *mut DrMixedV1) -> u8 {
    u8::from(mixed::release_owned(value))
}

/// # Safety
///
/// `value` must be null or a live box returned by `dr_v1_mixed_new`.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_mixed_free(value: *mut DrMixedV1) {
    mixed::free(value)
}

/// # Safety
///
/// `value` must point to a live mixed box.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_mixed_tag(value: *const DrMixedV1) -> u8 {
    (*value).tag
}

/// # Safety
///
/// `value` must point to a live mixed box.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_mixed_type_id(value: *const DrMixedV1) -> u32 {
    (*value).type_id
}

/// # Safety
///
/// `value` must point to a live mixed box.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_mixed_payload(value: *const DrMixedV1) -> u64 {
    (*value).payload
}

/// # Safety
///
/// `collection` must be a uniquely borrowed live Set and `value` must match
/// `value_kind`.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_collection_push_unique(
    collection: *mut DrCollectionV1,
    value: u64,
    value_kind: u8,
) -> u8 {
    u8::from(collection::push_unique(collection, value, value_kind))
}

/// # Safety
///
/// `collection` must be a uniquely borrowed live Set, `value` must match
/// `value_kind`, and `removed` must point to writable storage.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_collection_remove_value(
    collection: *mut DrCollectionV1,
    value: u64,
    value_kind: u8,
    removed: *mut u64,
) -> u8 {
    u8::from(collection::remove_value(
        collection, value, value_kind, removed,
    ))
}

/// # Safety
///
/// `left` and `right` must point to live Sets with the same element
/// representation, and `value_kind` must describe that representation. The
/// returned collection must be released exactly once.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_collection_set_algebra(
    left: *const DrCollectionV1,
    right: *const DrCollectionV1,
    operation: u8,
    value_kind: u8,
) -> *mut DrCollectionV1 {
    collection::set_algebra(left, right, operation, value_kind)
}

const STRING_HEADER_SIZE: usize = mem::size_of::<DrStringV1>();
const IMMORTAL_STRING_REFERENCES: usize = usize::MAX;

pub type DrMainIntV2 = unsafe extern "C" fn(*const DrStackFrameV2) -> i64;
pub type DrMainVoidV2 = unsafe extern "C" fn(*const DrStackFrameV2);

/// Entry forms that take the program arguments (decision 0099). The list is
/// owned by the glue and borrowed by `main`, so the callee never releases it.
pub type DrMainIntArgsV2 = unsafe extern "C" fn(*const DrStackFrameV2, *mut DrCollectionV1) -> i64;
pub type DrMainVoidArgsV2 = unsafe extern "C" fn(*const DrStackFrameV2, *mut DrCollectionV1);

/// Validates process arguments for an entrypoint that does not request the list.
///
/// # Safety
///
/// `argv` must be the argument vector the process was started with, valid for
/// `argc` entries.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_validate_entry_args(argc: i32, argv: *const *const u8) {
    entry_args::validate(argc, argv);
}

/// Allocates a headerless native class payload.
///
/// This is a private, versioned compiler/runtime ABI. `byte_alignment` is
/// currently bounded by the platform allocator alignment because every Stage
/// 19 property is at most pointer/f64 aligned. Empty classes receive a unique,
/// freeable one-byte allocation. Allocation failure panics with status 101.
///
/// # Safety
///
/// `current_frame` must be null or a valid generated frame chain. The returned
/// pointer must be released exactly once with `dr_v1_class_free`.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_class_allocate(
    current_frame: *const DrStackFrameV2,
    byte_length: usize,
    byte_alignment: usize,
) -> *mut u8 {
    let supported_alignment = mem::align_of::<u64>().max(mem::align_of::<usize>());
    if byte_alignment == 0
        || !byte_alignment.is_power_of_two()
        || byte_alignment > supported_alignment
    {
        panic_catalogued(current_frame, b"P1601");
    }
    let payload = allocate(byte_length.max(1));
    if payload.is_null() {
        panic_catalogued(current_frame, b"P1601");
    }
    // Stage 19 constructors may initialize a proven subset of fields in their
    // body. Zeroing the private payload keeps replacement-style stores safe
    // before those slots acquire their first owned value; no zeroed slot is
    // observable as a Doria value before construction completes.
    ptr::write_bytes(payload, 0, byte_length.max(1));
    payload
}

/// Frees a payload returned by `dr_v2_class_allocate`.
///
/// # Safety
///
/// `payload` must be null or a live class payload allocated by the matching
/// runtime. A live payload may be passed exactly once.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_class_free(payload: *mut u8) {
    if !payload.is_null() {
        deallocate(payload);
    }
}

/// Creates the first strong reference for an already-constructed class payload.
///
/// # Safety
///
/// `payload` must be a uniquely owned live class payload and `drop_payload` must
/// be its matching generated drop glue. Ownership transfers to the returned
/// control block.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_shared_create(
    current_frame: *const DrStackFrameV2,
    payload: *mut u8,
    drop_payload: unsafe extern "C" fn(*const DrStackFrameV2, *mut u8),
) -> *mut DrSharedControlV1 {
    let control = allocate(mem::size_of::<DrSharedControlV1>()).cast::<DrSharedControlV1>();
    if control.is_null() {
        panic_catalogued(current_frame, b"P1502");
    }
    control.write(DrSharedControlV1 {
        strong_references: 1,
        weak_references: 0,
        payload,
        drop_payload,
    });
    control
}

/// Creates one additional strong owner.
///
/// # Safety
///
/// `control` must point to a live control block with a live payload.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_shared_retain(
    current_frame: *const DrStackFrameV2,
    control: *mut DrSharedControlV1,
) -> *mut DrSharedControlV1 {
    let Some(next) = (*control).strong_references.checked_add(1) else {
        panic_catalogued(current_frame, b"P1503");
    };
    (*control).strong_references = next;
    control
}

/// Releases one strong owner and destroys the payload exactly once.
///
/// # Safety
///
/// `control` must be null or hold a live strong reference. The released handle
/// must not be used afterward.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_shared_release(
    current_frame: *const DrStackFrameV2,
    control: *mut DrSharedControlV1,
) {
    if control.is_null() {
        return;
    }
    debug_assert!((*control).strong_references != 0);
    (*control).strong_references -= 1;
    if (*control).strong_references != 0 {
        return;
    }
    let payload = (*control).payload;
    (*control).payload = ptr::null_mut();
    ((*control).drop_payload)(current_frame, payload);
    if (*control).weak_references == 0 {
        deallocate(control.cast());
    }
}

/// Creates one weak reference without retaining the payload.
///
/// # Safety
///
/// `control` must point to a live control block with a strong reference.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_shared_create_weak(
    current_frame: *const DrStackFrameV2,
    control: *mut DrSharedControlV1,
) -> *mut DrSharedControlV1 {
    let Some(next) = (*control).weak_references.checked_add(1) else {
        panic_catalogued(current_frame, b"P1504");
    };
    (*control).weak_references = next;
    control
}

/// Releases one weak reference.
///
/// # Safety
///
/// `control` must hold a live weak reference. The released handle must not be
/// used afterward.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_shared_release_weak(control: *mut DrSharedControlV1) {
    debug_assert!(!control.is_null());
    debug_assert!((*control).weak_references != 0);
    (*control).weak_references -= 1;
    if (*control).weak_references == 0 && (*control).strong_references == 0 {
        deallocate(control.cast());
    }
}

/// Attempts to create a strong owner from a weak reference.
///
/// # Safety
///
/// `control` must hold a live weak reference.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_shared_acquire(
    current_frame: *const DrStackFrameV2,
    control: *mut DrSharedControlV1,
) -> *mut DrSharedControlV1 {
    if (*control).strong_references == 0 {
        return ptr::null_mut();
    }
    dr_v2_shared_retain(current_frame, control)
}

/// Returns the live class payload behind a strong reference.
///
/// # Safety
///
/// `control` must point to a live control block with a strong reference.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_shared_payload(control: *const DrSharedControlV1) -> *mut u8 {
    debug_assert!(!control.is_null());
    debug_assert!((*control).strong_references != 0);
    debug_assert!(!(*control).payload.is_null());
    (*control).payload
}

/// Creates the first writable-family strong reference for an owned payload.
///
/// # Safety
///
/// `payload` must be uniquely owned and live, and `drop_payload` must match it.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_writable_shared_create(
    current_frame: *const DrStackFrameV2,
    payload: *mut u8,
    drop_payload: unsafe extern "C" fn(*const DrStackFrameV2, *mut u8),
) -> *mut DrWritableSharedControlV1 {
    let control =
        allocate(mem::size_of::<DrWritableSharedControlV1>()).cast::<DrWritableSharedControlV1>();
    if control.is_null() {
        panic_catalogued(current_frame, b"P1502");
    }
    control.write(DrWritableSharedControlV1 {
        strong_references: 1,
        weak_references: 0,
        readonly_accesses: 0,
        writable_access_active: false,
        payload,
        drop_payload,
    });
    control
}

/// Creates one additional writable-family strong owner.
///
/// # Safety
///
/// `control` must point to a live writable control block with a live payload.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_writable_shared_retain(
    current_frame: *const DrStackFrameV2,
    control: *mut DrWritableSharedControlV1,
) -> *mut DrWritableSharedControlV1 {
    let Some(next) = (*control).strong_references.checked_add(1) else {
        panic_catalogued(current_frame, b"P1503");
    };
    (*control).strong_references = next;
    control
}

/// Releases one writable-family strong owner.
///
/// # Safety
///
/// `control` must be null or hold a live strong reference.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_writable_shared_release(
    current_frame: *const DrStackFrameV2,
    control: *mut DrWritableSharedControlV1,
) {
    if control.is_null() {
        return;
    }
    debug_assert!((*control).strong_references != 0);
    (*control).strong_references -= 1;
    if (*control).strong_references != 0 {
        return;
    }
    debug_assert_eq!((*control).readonly_accesses, 0);
    debug_assert!(!(*control).writable_access_active);
    let payload = (*control).payload;
    (*control).payload = ptr::null_mut();
    ((*control).drop_payload)(current_frame, payload);
    if (*control).weak_references == 0 {
        deallocate(control.cast());
    }
}

/// Creates one writable-family weak reference.
///
/// # Safety
///
/// `control` must point to a live writable control block with a strong owner.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_writable_shared_create_weak(
    current_frame: *const DrStackFrameV2,
    control: *mut DrWritableSharedControlV1,
) -> *mut DrWritableSharedControlV1 {
    let Some(next) = (*control).weak_references.checked_add(1) else {
        panic_catalogued(current_frame, b"P1504");
    };
    (*control).weak_references = next;
    control
}

/// Releases one writable-family weak reference.
///
/// # Safety
///
/// `control` must hold a live weak reference.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_writable_shared_release_weak(
    control: *mut DrWritableSharedControlV1,
) {
    debug_assert!(!control.is_null());
    debug_assert!((*control).weak_references != 0);
    (*control).weak_references -= 1;
    if (*control).weak_references == 0 && (*control).strong_references == 0 {
        deallocate(control.cast());
    }
}

/// Attempts to create a writable-family strong owner from a weak reference.
///
/// # Safety
///
/// `control` must hold a live writable-family weak reference.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_writable_shared_acquire(
    current_frame: *const DrStackFrameV2,
    control: *mut DrWritableSharedControlV1,
) -> *mut DrWritableSharedControlV1 {
    if (*control).strong_references == 0 {
        return ptr::null_mut();
    }
    dr_v2_writable_shared_retain(current_frame, control)
}

/// Acquires one readonly access object and its strong ownership claim.
///
/// # Safety
///
/// `control` must point to a live writable control block.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_writable_shared_acquire_readonly_access(
    current_frame: *const DrStackFrameV2,
    control: *mut DrWritableSharedControlV1,
) -> *mut DrWritableSharedControlV1 {
    if (*control).writable_access_active {
        panic_catalogued(current_frame, b"P1501");
    }
    let Some(next_accesses) = (*control).readonly_accesses.checked_add(1) else {
        panic_catalogued(current_frame, b"P1505");
    };
    let Some(next_strong) = (*control).strong_references.checked_add(1) else {
        panic_catalogued(current_frame, b"P1503");
    };
    (*control).readonly_accesses = next_accesses;
    (*control).strong_references = next_strong;
    control
}

/// Acquires one exclusive writable access object and its strong ownership claim.
///
/// # Safety
///
/// `control` must point to a live writable control block.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_writable_shared_acquire_writable_access(
    current_frame: *const DrStackFrameV2,
    control: *mut DrWritableSharedControlV1,
) -> *mut DrWritableSharedControlV1 {
    if (*control).readonly_accesses != 0 {
        panic_catalogued(current_frame, b"P1501");
    }
    if (*control).writable_access_active {
        panic_catalogued(current_frame, b"P1501");
    }
    let Some(next_strong) = (*control).strong_references.checked_add(1) else {
        panic_catalogued(current_frame, b"P1503");
    };
    (*control).strong_references = next_strong;
    (*control).writable_access_active = true;
    control
}

/// Releases a readonly access registration, then its strong ownership claim.
///
/// # Safety
///
/// `control` must hold a live readonly access registration.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_writable_shared_release_readonly_access(
    current_frame: *const DrStackFrameV2,
    control: *mut DrWritableSharedControlV1,
) {
    debug_assert!(!control.is_null());
    debug_assert!((*control).readonly_accesses != 0);
    (*control).readonly_accesses -= 1;
    dr_v2_writable_shared_release(current_frame, control);
}

/// Releases a writable access registration, then its strong ownership claim.
///
/// # Safety
///
/// `control` must hold the active writable access registration.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_writable_shared_release_writable_access(
    current_frame: *const DrStackFrameV2,
    control: *mut DrWritableSharedControlV1,
) {
    debug_assert!(!control.is_null());
    debug_assert!((*control).writable_access_active);
    (*control).writable_access_active = false;
    dr_v2_writable_shared_release(current_frame, control);
}

/// Returns the payload behind a live readonly access object.
///
/// # Safety
///
/// `control` must hold a live readonly access registration.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_writable_shared_readonly_payload(
    control: *const DrWritableSharedControlV1,
) -> *mut u8 {
    debug_assert!(!control.is_null());
    debug_assert!((*control).readonly_accesses != 0);
    debug_assert!(!(*control).payload.is_null());
    (*control).payload
}

/// Returns the payload behind a live writable access object.
///
/// # Safety
///
/// `control` must hold the active writable access registration.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_writable_shared_writable_payload(
    control: *const DrWritableSharedControlV1,
) -> *mut u8 {
    debug_assert!(!control.is_null());
    debug_assert!((*control).writable_access_active);
    debug_assert!(!(*control).payload.is_null());
    (*control).payload
}

/// Invokes a generated Doria integer entry function and maps its result to a process status.
///
/// # Safety
///
/// `entry` must point to a generated function that implements `DrMainIntV2` and remains valid
/// for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_main_int(
    entry: DrMainIntV2,
    source_path: *const u8,
    source_path_length: usize,
    source_text: *const u8,
    source_text_length: usize,
    source_span_start: usize,
    source_span_end: usize,
) -> i32 {
    process_status(
        entry(ptr::null()),
        source_path,
        source_path_length,
        source_text,
        source_text_length,
        source_span_start,
        source_span_end,
    )
}

/// Maps a Doria `main(): int` result to a process status, panicking outside the
/// representable `0..=125` range.
unsafe fn process_status(
    status: i64,
    source_path: *const u8,
    source_path_length: usize,
    source_text: *const u8,
    source_text_length: usize,
    source_span_start: usize,
    source_span_end: usize,
) -> i32 {
    if (0..=125).contains(&status) {
        return status as i32;
    }

    static MAIN: &[u8] = b"main";
    let frame = DrStackFrameV2 {
        parent: ptr::null(),
        function_name: MAIN.as_ptr(),
        function_name_length: MAIN.len(),
        source_path,
        source_path_length,
        source_text,
        source_text_length,
        function_span_start: source_span_start,
        function_span_end: source_span_end,
        active_span_start: source_span_start,
        active_span_end: source_span_end,
    };
    panic_catalogued(&frame, b"P1111")
}

/// Invokes a Doria `main(List<string> $args): int`, building the argument list
/// from the platform argument vector and releasing it after `main` returns.
///
/// # Safety
///
/// `entry` must implement `DrMainIntArgsV2`. `argv` must be the argument vector
/// the process was started with, valid for `argc` entries.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_main_int_args(
    entry: DrMainIntArgsV2,
    argc: i32,
    argv: *const *const u8,
    source_path: *const u8,
    source_path_length: usize,
    source_text: *const u8,
    source_text_length: usize,
    source_span_start: usize,
    source_span_end: usize,
) -> i32 {
    let args = entry_args::build(argc, argv);
    // Validate before cleanup: an invalid status is an abort-only panic path.
    let status = process_status(
        entry(ptr::null(), args),
        source_path,
        source_path_length,
        source_text,
        source_text_length,
        source_span_start,
        source_span_end,
    );
    entry_args::release(args);
    status
}

/// Invokes a Doria `main(List<string> $args): void`.
///
/// # Safety
///
/// `entry` must implement `DrMainVoidArgsV2`. `argv` must be the argument vector
/// the process was started with, valid for `argc` entries.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_main_void_args(
    entry: DrMainVoidArgsV2,
    argc: i32,
    argv: *const *const u8,
) -> i32 {
    let args = entry_args::build(argc, argv);
    entry(ptr::null(), args);
    entry_args::release(args);
    0
}

/// Invokes a generated Doria void entry function.
///
/// # Safety
///
/// `entry` must point to a generated function that implements `DrMainVoidV2` and remains valid
/// for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_main_void(entry: DrMainVoidV2) -> i32 {
    entry(ptr::null());
    0
}

/// Terminates a CRT-free native process with the supplied status.
///
/// Windows process wrappers call this instead of returning from the custom PE
/// entrypoint. Unix process wrappers return through their C startup code.
#[no_mangle]
pub extern "C" fn dr_v1_exit_process(status: i32) -> ! {
    unsafe { exit_process(status) }
}

/// Writes an exact byte sequence to stdout.
///
/// A closed downstream pipe exits cleanly with status 0. Other write failures panic.
///
/// # Safety
///
/// `bytes` must be readable for `byte_length` bytes. `current_frame` must be null or point to a
/// valid `DrStackFrameV2` chain whose frame and function-name storage remains live for the call.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_write_stdout(
    current_frame: *const DrStackFrameV2,
    bytes: *const u8,
    byte_length: usize,
) {
    match write_standard_stream(StandardStream::Stdout, bytes, byte_length) {
        WriteOutcome::Success => return,
        WriteOutcome::BrokenPipe => exit_process(0),
        WriteOutcome::OtherFailure => {}
    }
    panic_catalogued(current_frame, b"P1407")
}

/// Writes an exact byte sequence to stderr.
///
/// A closed downstream pipe exits cleanly with status 0. Other failures exit with status 101.
///
/// # Safety
///
/// `bytes` must be readable for `byte_length` bytes.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_write_stderr(bytes: *const u8, byte_length: usize) {
    match write_standard_stream(StandardStream::Stderr, bytes, byte_length) {
        WriteOutcome::Success => {}
        WriteOutcome::BrokenPipe => exit_process(0),
        WriteOutcome::OtherFailure => exit_process(PANIC_STATUS),
    }
}

/// # Safety
///
/// `value` must point to a live byte buffer and `current_frame` must be null or valid.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_write_stdout_bytes(
    current_frame: *const DrStackFrameV2,
    value: *const DrBytesV1,
) {
    write_byte_stream(
        current_frame,
        StandardStream::Stdout,
        bytes::data(value),
        bytes::length(value),
        b"failed to write stdout",
    )
}

/// # Safety
///
/// `value` must point to a live byte buffer and `current_frame` must be null or valid.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_write_stderr_bytes(
    current_frame: *const DrStackFrameV2,
    value: *const DrBytesV1,
) {
    write_byte_stream(
        current_frame,
        StandardStream::Stderr,
        bytes::data(value),
        bytes::length(value),
        b"failed to write stderr",
    )
}

/// # Safety
///
/// `current_frame` must be null or a valid generated frame chain.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_read_stdin_bytes(
    current_frame: *const DrStackFrameV2,
) -> *mut DrBytesV1 {
    let buffered = line_io::take_buffered_input();
    let mut capacity = 4096_usize;
    while capacity < buffered.length {
        let Some(next) = capacity.checked_mul(2) else {
            panic_catalogued(current_frame, b"P1302");
        };
        capacity = next;
    }
    let mut data = allocate(capacity);
    if data.is_null() {
        panic_catalogued(current_frame, b"P1302");
    }
    if buffered.length != 0 {
        ptr::copy_nonoverlapping(buffered.bytes, data, buffered.length);
    }
    let mut length = buffered.length;
    if buffered.eof {
        return bytes::from_owned(data, length);
    }
    loop {
        if length == capacity {
            let Some(next) = capacity.checked_mul(2) else {
                panic_catalogued(current_frame, b"P1302");
            };
            let replacement = allocate(next);
            if replacement.is_null() {
                deallocate(data);
                panic_catalogued(current_frame, b"P1302");
            }
            ptr::copy_nonoverlapping(data, replacement, length);
            deallocate(data);
            data = replacement;
            capacity = next;
        }
        match device_io::read_bytes(StandardStream::Stdin, data.add(length), capacity - length) {
            Ok(0) => return bytes::from_owned(data, length),
            Ok(read) => length += read,
            Err(()) => {
                deallocate(data);
                panic_catalogued(current_frame, b"P1403");
            }
        }
    }
}

/// Flushes stdout through the implementation-private standard-device abstraction.
///
/// # Safety
/// `current_frame` must be null or a valid frame chain for panic reporting.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_flush_stdout(current_frame: *const DrStackFrameV2) {
    match device_io::flush(StandardStream::Stdout) {
        WriteOutcome::Success => return,
        // A closed downstream pipe is the permanent clean status-0 exit, exactly as
        // for an ordinary write. It must never become a status-101 panic.
        WriteOutcome::BrokenPipe => exit_process(0),
        WriteOutcome::OtherFailure => {}
    }
    panic_catalogued(current_frame, b"P1407")
}

/// Flushes stderr through the implementation-private standard-device abstraction.
///
/// # Safety
/// `current_frame` must be null or a valid frame chain for panic reporting.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_flush_stderr(current_frame: *const DrStackFrameV2) {
    match device_io::flush(StandardStream::Stderr) {
        WriteOutcome::Success => return,
        // A closed downstream pipe is the permanent clean status-0 exit, exactly as
        // for an ordinary write. It must never become a status-101 panic.
        WriteOutcome::BrokenPipe => exit_process(0),
        WriteOutcome::OtherFailure => {}
    }
    panic_catalogued(current_frame, b"P1407")
}

/// Returns whether one standard stream is attached to an interactive terminal.
///
/// Stream identifiers are 0=stdin, 1=stdout, and 2=stderr. Unknown identifiers return false.
///
/// # Safety
/// This operation has no pointer preconditions.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_stream_is_interactive(stream: u8) -> u8 {
    let stream = match stream {
        0 => StandardStream::Stdin,
        1 => StandardStream::Stdout,
        2 => StandardStream::Stderr,
        _ => return 0,
    };
    u8::from(device_io::is_interactive(stream))
}

/// Reads one UTF-8 line from stdin, returning null only for EOF before bytes.
///
/// The returned non-null string is owned. LF and CRLF endings are removed.
///
/// # Safety
/// `current_frame` must be null or a valid frame chain for panic reporting.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_read_stdin_line(
    current_frame: *const DrStackFrameV2,
) -> *mut DrStringV1 {
    match line_io::read_line() {
        Ok(Some((bytes, length))) => dr_v1_string_from_utf8(bytes, length),
        Ok(None) => ptr::null_mut(),
        Err(line_io::ReadLineError::InvalidUtf8) => panic_catalogued(current_frame, b"P1404"),
        Err(line_io::ReadLineError::Read) => panic_catalogued(current_frame, b"P1403"),
        Err(line_io::ReadLineError::Allocation) => panic_catalogued(current_frame, b"P1206"),
    }
}

/// Writes a prompt to stdout, flushes stdout, and then reads one line from stdin.
///
/// This is the single runtime operation behind `read_line(string $prompt = "")`.
/// The ordering is observable and mandatory: the prompt is written exactly as
/// supplied with no added newline, stdout is flushed, and only then is stdin read.
/// The flush happens even when the prompt is empty, so output written earlier with
/// `echo` is visible before the program blocks for input.
///
/// A closed stdout pipe during the prompt write or flush is the permanent clean
/// status-0 exit and never reaches stdin. Other output failures raise P1407; input
/// failures keep their existing identities.
///
/// The prompt is borrowed for the duration of the write and is never retained.
///
/// # Safety
/// `prompt` must be null (treated as empty) or a valid `DrStringV1`. `current_frame`
/// must be null or a valid `DrStackFrameV2` chain for panic reporting.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_read_stdin_line_prompted(
    current_frame: *const DrStackFrameV2,
    prompt: *const DrStringV1,
) -> *mut DrStringV1 {
    // 1. Write the prompt exactly. A zero-length write is skipped, but the flush
    //    below is never skipped.
    let prompt_length = if prompt.is_null() {
        0
    } else {
        (*prompt).byte_length
    };
    if prompt_length > 0 {
        match write_standard_stream(StandardStream::Stdout, string_bytes(prompt), prompt_length) {
            WriteOutcome::Success => {}
            WriteOutcome::BrokenPipe => exit_process(0),
            WriteOutcome::OtherFailure => panic_catalogued(current_frame, b"P1407"),
        }
    }

    // 2. Flush stdout before reading, so the prompt is observable while the program
    //    waits for input.
    match device_io::flush(StandardStream::Stdout) {
        WriteOutcome::Success => {}
        WriteOutcome::BrokenPipe => exit_process(0),
        WriteOutcome::OtherFailure => panic_catalogued(current_frame, b"P1407"),
    }

    // 3. Read one line under the existing line discipline.
    dr_v2_read_stdin_line(current_frame)
}

/// Reads a complete UTF-8 text file into an owned runtime string.
///
/// # Safety
/// `path` must identify a live runtime string and `current_frame` must be null or valid.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_read_file(
    current_frame: *const DrStackFrameV2,
    path: *const DrStringV1,
) -> *mut DrStringV1 {
    let path = core::slice::from_raw_parts(string_bytes(path), (*path).byte_length);
    match file_io::read_file(path) {
        Ok(contents) => {
            let bytes = core::slice::from_raw_parts(contents.bytes, contents.length);
            if core::str::from_utf8(bytes).is_err() {
                panic_catalogued(current_frame, b"P1406");
            }
            dr_v1_string_from_utf8(bytes.as_ptr(), bytes.len())
        }
        Err(file_io::FileError::PathNul) => panic_catalogued(current_frame, b"P1405"),
        Err(file_io::FileError::Allocation) => panic_catalogued(current_frame, b"P1206"),
        Err(_) => panic_catalogued(current_frame, b"P1401"),
    }
}

/// Creates or truncates a text file and writes exact runtime-string bytes.
///
/// # Safety
/// Both strings must be live and `current_frame` must be null or valid.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_write_file(
    current_frame: *const DrStackFrameV2,
    path: *const DrStringV1,
    contents: *const DrStringV1,
) {
    let path = core::slice::from_raw_parts(string_bytes(path), (*path).byte_length);
    let contents = core::slice::from_raw_parts(string_bytes(contents), (*contents).byte_length);
    match file_io::write_file(path, contents) {
        Ok(()) => {}
        Err(file_io::FileError::PathNul) => panic_catalogued(current_frame, b"P1405"),
        Err(file_io::FileError::Allocation) => panic_catalogued(current_frame, b"P1206"),
        Err(_) => panic_catalogued(current_frame, b"P1402"),
    }
}

/// # Safety
///
/// `path` and `contents` must point to live runtime strings and `current_frame` must be null or valid.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_append_file(
    current_frame: *const DrStackFrameV2,
    path: *const DrStringV1,
    contents: *const DrStringV1,
) {
    write_file_contents(
        current_frame,
        path,
        string_bytes(contents),
        (*contents).byte_length,
        true,
        b"P1206",
    )
}

/// # Safety
///
/// `path` must point to a live runtime string and `current_frame` must be null or valid.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_read_file_bytes(
    current_frame: *const DrStackFrameV2,
    path: *const DrStringV1,
) -> *mut DrBytesV1 {
    let path = core::slice::from_raw_parts(string_bytes(path), (*path).byte_length);
    match file_io::read_file(path) {
        Ok(contents) => {
            let (data, length) = contents.into_raw_parts();
            bytes::from_owned(data, length)
        }
        Err(file_io::FileError::PathNul) => panic_catalogued(current_frame, b"P1405"),
        Err(file_io::FileError::Allocation) => panic_catalogued(current_frame, b"P1302"),
        Err(_) => panic_catalogued(current_frame, b"P1401"),
    }
}

/// # Safety
///
/// `path` and `contents` must point to live values and `current_frame` must be null or valid.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_write_file_bytes(
    current_frame: *const DrStackFrameV2,
    path: *const DrStringV1,
    contents: *const DrBytesV1,
) {
    write_file_contents(
        current_frame,
        path,
        bytes::data(contents),
        bytes::length(contents),
        false,
        b"P1302",
    )
}

/// # Safety
///
/// `path` and `contents` must point to live values and `current_frame` must be null or valid.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_append_file_bytes(
    current_frame: *const DrStackFrameV2,
    path: *const DrStringV1,
    contents: *const DrBytesV1,
) {
    write_file_contents(
        current_frame,
        path,
        bytes::data(contents),
        bytes::length(contents),
        true,
        b"P1302",
    )
}

unsafe fn write_file_contents(
    current_frame: *const DrStackFrameV2,
    path: *const DrStringV1,
    contents: *const u8,
    length: usize,
    append: bool,
    allocation_failure_code: &'static [u8],
) {
    let path = core::slice::from_raw_parts(string_bytes(path), (*path).byte_length);
    let contents = if length == 0 {
        &[]
    } else {
        core::slice::from_raw_parts(contents, length)
    };
    let result = if append {
        file_io::append_file(path, contents)
    } else {
        file_io::write_file(path, contents)
    };
    match result {
        Ok(()) => {}
        Err(file_io::FileError::PathNul) => panic_catalogued(current_frame, b"P1405"),
        Err(file_io::FileError::Allocation) => {
            panic_catalogued(current_frame, allocation_failure_code)
        }
        Err(_) => panic_catalogued(current_frame, b"P1402"),
    }
}

unsafe fn panic_catalogued(current_frame: *const DrStackFrameV2, code: &'static [u8]) -> ! {
    dr_v2_panic_code(current_frame, code.as_ptr(), code.len(), ptr::null(), 0)
}

/// Reports a fatal Doria panic and exits the process with status 101.
///
/// # Safety
///
/// `message` must be readable for `message_length` bytes. `current_frame` must be null or point to
/// a finite, valid `DrStackFrameV2` chain whose frames and function-name byte ranges remain live
/// until process termination.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_panic(
    current_frame: *const DrStackFrameV2,
    message: *const u8,
    message_length: usize,
) -> ! {
    dr_v2_panic_code(current_frame, b"P1000".as_ptr(), 5, message, message_length)
}

/// Reports a catalogued fatal Doria panic and exits with status 101.
///
/// This is the source-aware V2 panic ABI. `code` must identify an entry in the
/// shared diagnostic catalogue. The optional message is used only for
/// user-authored `panic(...)`; built-in outcomes derive their public prose from
/// the catalogue.
///
/// # Safety
///
/// All byte ranges and every frame in `current_frame` must remain readable
/// until process termination.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_panic_code(
    current_frame: *const DrStackFrameV2,
    code: *const u8,
    code_length: usize,
    message: *const u8,
    message_length: usize,
) -> ! {
    dr_v2_panic_code_with_facts(
        current_frame,
        code,
        code_length,
        message,
        message_length,
        ptr::null(),
        0,
    )
}

#[no_mangle]
/// Reports the catalogued empty-padding panic with the facts required by
/// `P1203`.
///
/// # Safety
///
/// `current_frame` must be null or identify a valid bounded V2 frame chain,
/// and `value` must identify a live runtime string until process termination.
pub unsafe extern "C" fn dr_v2_panic_string_padding_empty(
    current_frame: *const DrStackFrameV2,
    value: *const DrStringV1,
    current_grapheme_length: usize,
    requested_grapheme_length: i64,
    padding_grapheme_length: usize,
) -> ! {
    let facts = [
        DrRuntimeFactV2 {
            name: b"value".as_ptr(),
            name_length: 5,
            kind: 4,
            value: dr_v1_string_data(value) as u64,
            value_length: dr_v1_string_byte_length(value),
        },
        DrRuntimeFactV2 {
            name: b"currentGraphemeLength".as_ptr(),
            name_length: 21,
            kind: 2,
            value: current_grapheme_length as u64,
            value_length: 0,
        },
        DrRuntimeFactV2 {
            name: b"requestedGraphemeLength".as_ptr(),
            name_length: 23,
            kind: 1,
            value: requested_grapheme_length as u64,
            value_length: 0,
        },
        DrRuntimeFactV2 {
            name: b"paddingGraphemeLength".as_ptr(),
            name_length: 21,
            kind: 2,
            value: padding_grapheme_length as u64,
            value_length: 0,
        },
    ];
    dr_v2_panic_code_with_facts(
        current_frame,
        b"P1203".as_ptr(),
        5,
        ptr::null(),
        0,
        facts.as_ptr(),
        facts.len(),
    )
}

/// Source-aware V2 panic entry with typed dynamic facts.
///
/// # Safety
///
/// `facts` must contain `fact_count` valid records whose names remain live.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_panic_code_with_facts(
    current_frame: *const DrStackFrameV2,
    code: *const u8,
    code_length: usize,
    message: *const u8,
    message_length: usize,
    facts: *const DrRuntimeFactV2,
    fact_count: usize,
) -> ! {
    if code.is_null()
        || (message_length != 0 && message.is_null())
        || (fact_count != 0 && facts.is_null())
    {
        emergency_runtime_panic();
    }
    let code_bytes = core::slice::from_raw_parts(code, code_length);
    let code_text = core::str::from_utf8(code_bytes).unwrap_or("P1001");
    let Some(entry) = doria_diagnostic_catalogue::runtime_entry(code_text) else {
        emergency_runtime_panic();
    };
    if !runtime_facts_match(entry.fact_names, facts, fact_count) {
        emergency_runtime_panic();
    }

    if write_runtime_outcome_record(
        current_frame,
        entry.code.as_bytes(),
        message,
        message_length,
        facts,
        fact_count,
    ) {
        exit_process(PANIC_STATUS)
    }

    write_panic_fragment(b"Panic[");
    write_panic_fragment(entry.code.as_bytes());
    write_panic_fragment(b"]: ");
    write_panic_fragment(entry.title.as_bytes());

    if !current_frame.is_null() {
        render_runtime_where(current_frame, entry.primary_label.as_bytes());
    }
    write_panic_fragment(b"\n\nWhy\n");
    if entry.code == "P1203" && fact_count == 4 {
        write_panic_fragment(b"`padEnd` was asked to extend `\"");
        write_panic_bytes((*facts).value as *const u8, (*facts).value_length);
        write_panic_fragment(b"\"` from ");
        write_usize((*facts.add(1)).value as usize);
        write_panic_fragment(b" to ");
        write_usize((*facts.add(2)).value as usize);
        write_panic_fragment(b" graphemes,\nbut an empty padding string cannot add any graphemes.");
    } else {
        write_panic_fragment(entry.explanation.as_bytes());
    }
    if entry.code == "P1000" && message_length != 0 {
        write_panic_fragment(b"\n\nNote\n");
        write_panic_bytes(message, message_length);
    }
    if !current_frame.is_null() {
        write_panic_fragment(b"\n\nCall Path");
        let mut frame = current_frame;
        while !frame.is_null() {
            write_panic_fragment(b"\n");
            write_panic_bytes((*frame).function_name, (*frame).function_name_length);
            if (*frame).source_path_length != 0 {
                write_panic_fragment(b" \xc2\xb7 ");
                write_panic_bytes((*frame).source_path, (*frame).source_path_length);
                write_panic_fragment(b":");
                write_usize(source_line(
                    (*frame).source_text,
                    (*frame).source_text_length,
                    (*frame).active_span_start,
                ));
            }
            frame = (*frame).parent;
        }
    }
    write_panic_fragment(b"\n\nProcess Exited With Status 101\n");
    exit_process(PANIC_STATUS)
}

unsafe fn runtime_facts_match(
    expected: &[&str],
    facts: *const DrRuntimeFactV2,
    fact_count: usize,
) -> bool {
    if fact_count == 0 {
        return true;
    }
    if fact_count != expected.len() {
        return false;
    }
    for (index, expected_name) in expected.iter().enumerate() {
        let fact = &*facts.add(index);
        if fact.name.is_null()
            || fact.name_length != expected_name.len()
            || !(1..=4).contains(&fact.kind)
            || core::slice::from_raw_parts(fact.name, fact.name_length) != expected_name.as_bytes()
            || (fact.kind == 4 && fact.value_length != 0 && fact.value == 0)
        {
            return false;
        }
    }
    true
}

unsafe fn emergency_runtime_panic() -> ! {
    write_panic_fragment(
        b"Panic[P1001]: Runtime Diagnostic Failed\n\nProcess Exited With Status 101\n",
    );
    exit_process(PANIC_STATUS)
}

unsafe fn write_runtime_outcome_record(
    current_frame: *const DrStackFrameV2,
    code: &[u8],
    message: *const u8,
    message_length: usize,
    facts: *const DrRuntimeFactV2,
    fact_count: usize,
) -> bool {
    const MAX_CODE: usize = 16;
    const MAX_MESSAGE: usize = 64 * 1024;
    const MAX_PATH: usize = 4096;
    const MAX_SOURCE: usize = 4 * 1024 * 1024;
    const MAX_FUNCTION: usize = 1024;
    const MAX_FRAMES: usize = 128;
    const MAX_FACTS: usize = 32;

    let mut path_buffer = [0_u8; MAX_PATH];
    let Some(channel_path) = runtime_outcome_channel_path(&mut path_buffer) else {
        return false;
    };
    if current_frame.is_null()
        || code.len() > MAX_CODE
        || message_length > MAX_MESSAGE
        || (*current_frame).source_path_length > MAX_PATH
        || (*current_frame).source_text_length > MAX_SOURCE
        || (*current_frame).function_name_length > MAX_FUNCTION
        || fact_count > MAX_FACTS
    {
        return false;
    }
    let mut frame_count = 0_usize;
    let mut frame = current_frame;
    while !frame.is_null() {
        if frame_count == MAX_FRAMES
            || (*frame).function_name_length > MAX_FUNCTION
            || (*frame).source_path_length > MAX_PATH
        {
            return false;
        }
        frame_count += 1;
        frame = (*frame).parent;
    }

    for index in 0..fact_count {
        if (*facts.add(index)).name_length > 1024
            || !(1..=4).contains(&(*facts.add(index)).kind)
            || (*facts.add(index)).value_length > MAX_MESSAGE
        {
            return false;
        }
    }

    let mut header = [0_u8; 46];
    header[..8].copy_from_slice(b"DORIAO2\0");
    put_u16(&mut header[8..10], 2);
    put_u16(&mut header[10..12], code.len() as u16);
    put_u32(&mut header[12..16], message_length as u32);
    put_u32(
        &mut header[16..20],
        (*current_frame).source_path_length as u32,
    );
    put_u32(
        &mut header[20..24],
        (*current_frame).source_text_length as u32,
    );
    put_u16(
        &mut header[24..26],
        (*current_frame).function_name_length as u16,
    );
    put_u16(&mut header[26..28], frame_count as u16);
    put_u16(&mut header[28..30], fact_count as u16);
    put_u64(
        &mut header[30..38],
        (*current_frame).active_span_start as u64,
    );
    put_u64(&mut header[38..46], (*current_frame).active_span_end as u64);

    if file_io::write_file(channel_path, &header).is_err()
        || file_io::append_file(channel_path, code).is_err()
        || (message_length != 0
            && file_io::append_file(
                channel_path,
                core::slice::from_raw_parts(message, message_length),
            )
            .is_err())
        || file_io::append_file(
            channel_path,
            core::slice::from_raw_parts(
                (*current_frame).source_path,
                (*current_frame).source_path_length,
            ),
        )
        .is_err()
        || file_io::append_file(
            channel_path,
            core::slice::from_raw_parts(
                (*current_frame).source_text,
                (*current_frame).source_text_length,
            ),
        )
        .is_err()
        || file_io::append_file(
            channel_path,
            core::slice::from_raw_parts(
                (*current_frame).function_name,
                (*current_frame).function_name_length,
            ),
        )
        .is_err()
    {
        return false;
    }

    for index in 0..fact_count {
        let fact = &*facts.add(index);
        let mut fact_header = [0_u8; 15];
        put_u16(&mut fact_header[..2], fact.name_length as u16);
        fact_header[2] = fact.kind as u8;
        put_u64(&mut fact_header[3..11], fact.value);
        put_u32(&mut fact_header[11..15], fact.value_length as u32);
        if file_io::append_file(channel_path, &fact_header).is_err()
            || file_io::append_file(
                channel_path,
                core::slice::from_raw_parts(fact.name, fact.name_length),
            )
            .is_err()
            || (fact.kind == 4
                && fact.value_length != 0
                && file_io::append_file(
                    channel_path,
                    core::slice::from_raw_parts(fact.value as *const u8, fact.value_length),
                )
                .is_err())
        {
            return false;
        }
    }

    frame = current_frame;
    while !frame.is_null() {
        let mut frame_header = [0_u8; 22];
        put_u16(&mut frame_header[..2], (*frame).function_name_length as u16);
        put_u32(&mut frame_header[2..6], (*frame).source_path_length as u32);
        put_u64(&mut frame_header[6..14], (*frame).active_span_start as u64);
        put_u64(&mut frame_header[14..22], (*frame).active_span_end as u64);
        if file_io::append_file(channel_path, &frame_header).is_err()
            || file_io::append_file(
                channel_path,
                core::slice::from_raw_parts((*frame).function_name, (*frame).function_name_length),
            )
            .is_err()
            || file_io::append_file(
                channel_path,
                core::slice::from_raw_parts((*frame).source_path, (*frame).source_path_length),
            )
            .is_err()
        {
            return false;
        }
        frame = (*frame).parent;
    }
    true
}

fn put_u16(target: &mut [u8], value: u16) {
    target.copy_from_slice(&value.to_le_bytes());
}

fn put_u32(target: &mut [u8], value: u32) {
    target.copy_from_slice(&value.to_le_bytes());
}

fn put_u64(target: &mut [u8], value: u64) {
    target.copy_from_slice(&value.to_le_bytes());
}

#[cfg(unix)]
unsafe fn runtime_outcome_channel_path(buffer: &mut [u8]) -> Option<&[u8]> {
    let name = b"DORIA_RUNTIME_OUTCOME_V2\0";
    let value = getenv(name.as_ptr());
    if value.is_null() {
        return None;
    }
    for index in 0..buffer.len() {
        let byte = *value.add(index);
        if byte == 0 {
            return Some(&buffer[..index]);
        }
        buffer[index] = byte;
    }
    None
}

#[cfg(windows)]
unsafe fn runtime_outcome_channel_path(buffer: &mut [u8]) -> Option<&[u8]> {
    let name = b"DORIA_RUNTIME_OUTCOME_V2\0";
    let length = GetEnvironmentVariableA(
        name.as_ptr(),
        buffer.as_mut_ptr(),
        buffer.len().try_into().ok()?,
    ) as usize;
    (length != 0 && length < buffer.len()).then_some(&buffer[..length])
}

unsafe fn render_runtime_where(frame: *const DrStackFrameV2, primary_label: &[u8]) {
    let line = source_line(
        (*frame).source_text,
        (*frame).source_text_length,
        (*frame).active_span_start,
    );
    write_panic_fragment(b"\n\nWhere\n");
    write_panic_bytes((*frame).source_path, (*frame).source_path_length);
    write_panic_fragment(b" \xc2\xb7 line ");
    write_usize(line);
    write_panic_fragment(b" \xc2\xb7 ");
    write_panic_bytes((*frame).function_name, (*frame).function_name_length);

    if (*frame).source_text_length == 0 {
        return;
    }
    let source = core::slice::from_raw_parts((*frame).source_text, (*frame).source_text_length);
    if core::str::from_utf8(source).is_err() {
        return;
    }
    let start = (*frame).active_span_start.min(source.len());
    let end = (*frame)
        .active_span_end
        .max(start.saturating_add(1))
        .min(source.len());
    let mut line_number = line;
    let mut line_start = source[..start]
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |index| index + 1);
    let final_line = source_line(source.as_ptr(), source.len(), end);
    let gutter = decimal_digits(final_line).max(3);
    while line_number <= final_line && line_start <= source.len() {
        let mut line_end = source[line_start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(source.len(), |index| line_start + index);
        if line_end > line_start && source[line_end - 1] == b'\r' {
            line_end -= 1;
        }
        let marker_start = start.max(line_start).min(line_end);
        let marker_end = end.min(line_end).max(marker_start);
        let digits = decimal_digits(line_number);
        if line_number == line {
            write_panic_fragment(b"\n\n");
        } else {
            write_panic_fragment(b"\n");
        }
        for _ in digits..gutter {
            write_panic_fragment(b" ");
        }
        write_usize(line_number);
        write_panic_fragment(b"      ");
        write_source_line(&source[line_start..line_end]);
        write_panic_fragment(b"\n");
        let marker_offset = display_width_with_tabs(&source[line_start..marker_start], 4);
        let marker_width = display_width_with_tabs(&source[marker_start..marker_end], 4).max(1);
        for _ in 0..(gutter + 6 + marker_offset) {
            write_panic_fragment(b" ");
        }
        for _ in 0..marker_width {
            write_panic_fragment(b"^");
        }
        if line_number == line {
            write_panic_fragment(b"\n");
            for _ in 0..(gutter + 6 + marker_offset) {
                write_panic_fragment(b" ");
            }
            write_panic_fragment(primary_label);
        }
        let next_line_start = source[line_start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| line_start + index + 1);
        let Some(next_line_start) = next_line_start else {
            break;
        };
        line_start = next_line_start;
        line_number += 1;
    }
}

fn display_width_with_tabs(bytes: &[u8], tab_width: usize) -> usize {
    let Ok(text) = core::str::from_utf8(bytes) else {
        return 0;
    };
    let mut width = 0;
    for character in text.chars() {
        if character == '\t' {
            width += tab_width - (width % tab_width);
        } else {
            width += character.width().unwrap_or(0);
        }
    }
    width
}

unsafe fn write_source_line(bytes: &[u8]) {
    let Ok(text) = core::str::from_utf8(bytes) else {
        return;
    };
    let mut width = 0;
    for character in text.chars() {
        if character == '\t' {
            let spaces = 4 - (width % 4);
            for _ in 0..spaces {
                write_panic_fragment(b" ");
            }
            width += spaces;
        } else {
            let mut encoded = [0_u8; 4];
            let value = character.encode_utf8(&mut encoded);
            write_panic_fragment(value.as_bytes());
            width += character.width().unwrap_or(0);
        }
    }
}

unsafe fn source_line(source: *const u8, source_length: usize, offset: usize) -> usize {
    if source.is_null() || source_length == 0 {
        return 1;
    }
    let source = core::slice::from_raw_parts(source, source_length);
    1 + source[..offset.min(source.len())]
        .iter()
        .filter(|byte| **byte == b'\n')
        .count()
}

fn decimal_digits(mut value: usize) -> usize {
    let mut digits = 1;
    while value >= 10 {
        value /= 10;
        digits += 1;
    }
    digits
}

unsafe fn write_usize(value: usize) {
    let mut buffer = [0_u8; 20];
    let (start, length) = unsigned_decimal(value as u64, &mut buffer);
    write_panic_fragment(&buffer[start..start + length]);
}

/// Allocates an immutable runtime string from an explicit byte range.
///
/// # Safety
/// `bytes` must be readable for `byte_length` bytes and contain valid UTF-8.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_string_from_utf8(
    bytes: *const u8,
    byte_length: usize,
) -> *mut DrStringV1 {
    let string = allocate_string(byte_length);
    if byte_length != 0 {
        ptr::copy_nonoverlapping(bytes, string_bytes_mut(string), byte_length);
    }
    string
}

pub(crate) unsafe fn allocate_string(byte_length: usize) -> *mut DrStringV1 {
    allocate_string_with_frame(ptr::null(), byte_length)
}

pub(crate) unsafe fn allocate_string_with_frame(
    frame: *const DrStackFrameV2,
    byte_length: usize,
) -> *mut DrStringV1 {
    let total = STRING_HEADER_SIZE
        .checked_add(byte_length)
        .unwrap_or_else(|| panic_catalogued(frame, b"P1205"));
    let string = allocate(total).cast::<DrStringV1>();
    if string.is_null() {
        panic_catalogued(frame, b"P1205");
    }
    ptr::write(
        string,
        DrStringV1 {
            references: 1,
            byte_length,
        },
    );
    string
}

pub(crate) unsafe fn string_equal(left: *const DrStringV1, right: *const DrStringV1) -> bool {
    if left == right {
        return true;
    }
    if left.is_null() || right.is_null() || (*left).byte_length != (*right).byte_length {
        return false;
    }
    core::slice::from_raw_parts(string_bytes(left), (*left).byte_length)
        == core::slice::from_raw_parts(string_bytes(right), (*right).byte_length)
}

/// Retains one owned reference.
///
/// # Safety
/// `string` must be null or a live doria-rt string.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_string_retain(string: *mut DrStringV1) -> *mut DrStringV1 {
    if !string.is_null() && (*string).references != IMMORTAL_STRING_REFERENCES {
        (*string).references = (*string)
            .references
            .checked_add(1)
            .unwrap_or_else(|| runtime_invariant_panic());
    }
    string
}

/// Releases one owned reference and frees the final reference.
///
/// # Safety
/// `string` must be null or a live owned doria-rt string reference.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_string_release(string: *mut DrStringV1) {
    if string.is_null() {
        return;
    }
    let references = (*string).references;
    if references == IMMORTAL_STRING_REFERENCES {
        return;
    }
    if references == 0 {
        runtime_invariant_panic();
    }
    if references == 1 {
        deallocate(string.cast::<u8>());
    } else {
        (*string).references = references - 1;
    }
}

/// Concatenates two borrowed strings into a new owned string.
///
/// # Safety
/// Both pointers must identify live doria-rt strings.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_string_concat(
    left: *const DrStringV1,
    right: *const DrStringV1,
) -> *mut DrStringV1 {
    let length = (*left)
        .byte_length
        .checked_add((*right).byte_length)
        .unwrap_or_else(|| string_result_too_large());
    let result = allocate_string(length);
    ptr::copy_nonoverlapping(
        string_bytes(left),
        string_bytes_mut(result),
        (*left).byte_length,
    );
    ptr::copy_nonoverlapping(
        string_bytes(right),
        string_bytes_mut(result).add((*left).byte_length),
        (*right).byte_length,
    );
    result
}

/// Returns -1, 0, or 1 using unsigned byte-lexicographic ordering.
///
/// # Safety
/// Both pointers must identify live doria-rt strings.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_string_compare(
    left: *const DrStringV1,
    right: *const DrStringV1,
) -> i32 {
    let common = core::cmp::min((*left).byte_length, (*right).byte_length);
    for index in 0..common {
        let left_byte = *string_bytes(left).add(index);
        let right_byte = *string_bytes(right).add(index);
        if left_byte < right_byte {
            return -1;
        }
        if left_byte > right_byte {
            return 1;
        }
    }
    match (*left).byte_length.cmp(&(*right).byte_length) {
        core::cmp::Ordering::Less => -1,
        core::cmp::Ordering::Equal => 0,
        core::cmp::Ordering::Greater => 1,
    }
}

/// Compares two nullable runtime strings for value equality.
///
/// # Safety
/// Each pointer must be null or identify a live doria-rt string.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_nullable_string_equal(
    left: *const DrStringV1,
    right: *const DrStringV1,
) -> u8 {
    if left.is_null() || right.is_null() {
        return u8::from(left == right);
    }
    u8::from(dr_v1_string_compare(left, right) == 0)
}

#[no_mangle]
/// Returns the explicit byte pointer for a live string.
///
/// # Safety
/// `string` must identify a live doria-rt string for the duration of byte access.
pub unsafe extern "C" fn dr_v1_string_data(string: *const DrStringV1) -> *const u8 {
    string_bytes(string)
}

#[no_mangle]
/// Returns the explicit byte length for a live string.
///
/// # Safety
/// `string` must identify a live doria-rt string.
pub unsafe extern "C" fn dr_v1_string_length(string: *const DrStringV1) -> usize {
    dr_v1_string_byte_length(string)
}

#[no_mangle]
/// Returns the exact UTF-8 byte length for a live string.
///
/// # Safety
/// `string` must identify a live doria-rt string.
pub unsafe extern "C" fn dr_v1_string_byte_length(string: *const DrStringV1) -> usize {
    (*string).byte_length
}

#[no_mangle]
/// Writes a borrowed string to stdout without adding a newline.
///
/// # Safety
/// `string` must identify a live doria-rt string and `current_frame` must be null or a valid frame chain.
pub unsafe extern "C" fn dr_v2_write_string_stdout(
    current_frame: *const DrStackFrameV2,
    string: *const DrStringV1,
) {
    dr_v2_write_stdout(current_frame, string_bytes(string), (*string).byte_length)
}

/// Writes a borrowed string to stderr without adding a newline.
///
/// # Safety
/// `string` must identify a live runtime string and `current_frame` must be null or valid.
#[no_mangle]
pub unsafe extern "C" fn dr_v2_write_string_stderr(
    current_frame: *const DrStackFrameV2,
    string: *const DrStringV1,
) {
    match write_standard_stream(
        StandardStream::Stderr,
        string_bytes(string),
        (*string).byte_length,
    ) {
        WriteOutcome::Success => return,
        WriteOutcome::BrokenPipe => exit_process(0),
        WriteOutcome::OtherFailure => {}
    }
    panic_catalogued(current_frame, b"P1407");
}

#[no_mangle]
/// Creates an owned string containing canonical signed decimal display text.
///
/// # Safety
/// The returned owned reference must eventually be released on a normal execution path.
pub unsafe extern "C" fn dr_v1_string_from_i64(value: i64) -> *mut DrStringV1 {
    let mut buffer = [0_u8; 20];
    let (start, length) = signed_decimal(value, &mut buffer);
    dr_v1_string_from_utf8(buffer.as_ptr().add(start), length)
}

#[no_mangle]
/// Creates an owned string containing canonical unsigned decimal display text.
///
/// # Safety
/// The returned owned reference must eventually be released on a normal execution path.
pub unsafe extern "C" fn dr_v1_string_from_u64(value: u64) -> *mut DrStringV1 {
    let mut buffer = [0_u8; 20];
    let (start, length) = unsigned_decimal(value, &mut buffer);
    dr_v1_string_from_utf8(buffer.as_ptr().add(start), length)
}

#[no_mangle]
/// Creates an owned string containing canonical binary32 display text.
///
/// # Safety
/// The returned owned reference must eventually be released on a normal execution path.
pub unsafe extern "C" fn dr_v1_string_from_f32(value: f32) -> *mut DrStringV1 {
    float_string_f32(value)
}

#[no_mangle]
/// Creates an owned string containing canonical binary64 display text.
///
/// # Safety
/// The returned owned reference must eventually be released on a normal execution path.
pub unsafe extern "C" fn dr_v1_string_from_f64(value: f64) -> *mut DrStringV1 {
    float_string_f64(value)
}

#[no_mangle]
/// Creates an owned string containing `true` or `false`.
///
/// # Safety
/// The returned owned reference must eventually be released on a normal execution path.
pub unsafe extern "C" fn dr_v1_string_from_bool(value: u8) -> *mut DrStringV1 {
    let bytes: &[u8] = if value == 0 { b"false" } else { b"true" };
    dr_v1_string_from_utf8(bytes.as_ptr(), bytes.len())
}

const FORMAT_LEFT_ALIGN: u8 = 1;
const FORMAT_ZERO_PAD: u8 = 2;
const FORMAT_DECIMAL: u8 = 1;
const FORMAT_HEX_LOWER: u8 = 2;
const FORMAT_HEX_UPPER: u8 = 3;
const FORMAT_OCTAL: u8 = 4;
const FORMAT_BINARY: u8 = 5;

/// Applies Stage 17 byte-counted width to a borrowed string.
///
/// # Safety
/// `value` must identify a live runtime string.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_format_string(
    value: *const DrStringV1,
    width: u32,
    flags: u8,
) -> *mut DrStringV1 {
    padded_string(
        string_bytes(value),
        (*value).byte_length,
        width,
        flags,
        false,
    )
}

/// Formats a signed integer using a validated Stage 17 integer conversion.
///
/// # Safety
/// `conversion`, `bit_width`, and flags must come from validated MIR.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_format_i64(
    value: i64,
    bit_width: u8,
    conversion: u8,
    width: u32,
    flags: u8,
) -> *mut DrStringV1 {
    let mut buffer = [0_u8; 65];
    let (start, length) = if conversion == FORMAT_DECIMAL {
        signed_decimal(
            value,
            (&mut buffer[..20]).try_into().expect("20-byte prefix"),
        )
    } else {
        let mask = if bit_width == 64 {
            u64::MAX
        } else {
            (1_u64 << bit_width) - 1
        };
        unsigned_base((value as u64) & mask, conversion, &mut buffer)
    };
    padded_string(
        buffer.as_ptr().add(start),
        length,
        width,
        flags,
        value < 0 && conversion == FORMAT_DECIMAL,
    )
}

/// Formats an unsigned integer using a validated Stage 17 integer conversion.
///
/// # Safety
/// `conversion` and flags must come from validated MIR.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_format_u64(
    value: u64,
    conversion: u8,
    width: u32,
    flags: u8,
) -> *mut DrStringV1 {
    let mut buffer = [0_u8; 65];
    let (start, length) = if conversion == FORMAT_DECIMAL {
        let mut decimal = [0_u8; 20];
        let result = unsigned_decimal(value, &mut decimal);
        let start = buffer.len() - result.1;
        buffer[start..].copy_from_slice(&decimal[result.0..]);
        (start, result.1)
    } else {
        unsigned_base(value, conversion, &mut buffer)
    };
    padded_string(buffer.as_ptr().add(start), length, width, flags, false)
}

/// Formats a binary32 value with validated fixed precision and width.
///
/// # Safety
/// Precision and flags must come from validated MIR.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_format_f32(
    value: f32,
    precision: u32,
    width: u32,
    flags: u8,
) -> *mut DrStringV1 {
    format_fixed_float(
        value as f64,
        value.is_sign_negative(),
        precision,
        width,
        flags,
    )
}

/// Formats a binary64 value with validated fixed precision and width.
///
/// # Safety
/// Precision and flags must come from validated MIR.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_format_f64(
    value: f64,
    precision: u32,
    width: u32,
    flags: u8,
) -> *mut DrStringV1 {
    format_fixed_float(value, value.is_sign_negative(), precision, width, flags)
}

unsafe fn format_fixed_float(
    value: f64,
    negative: bool,
    precision: u32,
    width: u32,
    flags: u8,
) -> *mut DrStringV1 {
    if value.is_nan() {
        return padded_string(b"NaN".as_ptr(), 3, width, flags, false);
    }
    if value == f64::INFINITY {
        return padded_string(b"Infinity".as_ptr(), 8, width, flags, false);
    }
    if value == f64::NEG_INFINITY {
        return padded_string(b"-Infinity".as_ptr(), 9, width, flags, true);
    }
    let precision = precision as usize;
    let mut factor = 1_u128;
    for _ in 0..precision {
        factor = factor
            .checked_mul(10)
            .unwrap_or_else(|| string_result_too_large());
    }
    let scaled = value.abs() * factor as f64;
    if !scaled.is_finite() || scaled > u128::MAX as f64 {
        string_result_too_large();
    }
    let truncated = scaled as u128;
    let fraction = scaled - truncated as f64;
    let rounded = if fraction > 0.5 || (fraction == 0.5 && truncated & 1 == 1) {
        truncated
            .checked_add(1)
            .unwrap_or_else(|| string_result_too_large())
    } else {
        truncated
    };
    let integer = rounded / factor;
    let fractional = rounded % factor;
    let mut digits = [0_u8; 39];
    let (integer_start, integer_length) = unsigned_decimal_u128(integer, &mut digits);
    let sign_length = usize::from(negative);
    let decimal_length = usize::from(precision != 0);
    let length = sign_length
        .checked_add(integer_length)
        .and_then(|length| length.checked_add(decimal_length))
        .and_then(|length| length.checked_add(precision))
        .unwrap_or_else(|| string_result_too_large());
    let raw = allocate_string(length);
    let mut cursor = 0;
    if negative {
        *string_bytes_mut(raw) = b'-';
        cursor += 1;
    }
    ptr::copy_nonoverlapping(
        digits.as_ptr().add(integer_start),
        string_bytes_mut(raw).add(cursor),
        integer_length,
    );
    cursor += integer_length;
    if precision != 0 {
        *string_bytes_mut(raw).add(cursor) = b'.';
        cursor += 1;
        let mut divisor = factor / 10;
        for _ in 0..precision {
            *string_bytes_mut(raw).add(cursor) = b'0' + ((fractional / divisor) % 10) as u8;
            cursor += 1;
            divisor = core::cmp::max(divisor / 10, 1);
        }
    }
    let padded = padded_string(string_bytes(raw), length, width, flags, negative);
    dr_v1_string_release(raw);
    padded
}

unsafe fn padded_string(
    bytes: *const u8,
    length: usize,
    width: u32,
    flags: u8,
    negative_decimal: bool,
) -> *mut DrStringV1 {
    let width = width as usize;
    if width <= length {
        return dr_v1_string_from_utf8(bytes, length);
    }
    let result = allocate_string(width);
    let padding = width - length;
    let left = flags & FORMAT_LEFT_ALIGN != 0;
    let zero = flags & FORMAT_ZERO_PAD != 0 && !left;
    if left {
        ptr::copy_nonoverlapping(bytes, string_bytes_mut(result), length);
        ptr::write_bytes(string_bytes_mut(result).add(length), b' ', padding);
    } else if zero && negative_decimal {
        *string_bytes_mut(result) = b'-';
        ptr::write_bytes(string_bytes_mut(result).add(1), b'0', padding);
        ptr::copy_nonoverlapping(
            bytes.add(1),
            string_bytes_mut(result).add(1 + padding),
            length - 1,
        );
    } else {
        ptr::write_bytes(
            string_bytes_mut(result),
            if zero { b'0' } else { b' ' },
            padding,
        );
        ptr::copy_nonoverlapping(bytes, string_bytes_mut(result).add(padding), length);
    }
    result
}

fn unsigned_base(mut value: u64, conversion: u8, buffer: &mut [u8; 65]) -> (usize, usize) {
    let radix = match conversion {
        FORMAT_HEX_LOWER | FORMAT_HEX_UPPER => 16,
        FORMAT_OCTAL => 8,
        FORMAT_BINARY => 2,
        _ => 10,
    };
    let uppercase = conversion == FORMAT_HEX_UPPER;
    let mut cursor = buffer.len();
    loop {
        cursor -= 1;
        let digit = (value % radix) as u8;
        buffer[cursor] = match digit {
            0..=9 => b'0' + digit,
            _ if uppercase => b'A' + digit - 10,
            _ => b'a' + digit - 10,
        };
        value /= radix;
        if value == 0 {
            break;
        }
    }
    (cursor, buffer.len() - cursor)
}

fn unsigned_decimal_u128(mut value: u128, buffer: &mut [u8; 39]) -> (usize, usize) {
    let mut cursor = buffer.len();
    loop {
        cursor -= 1;
        buffer[cursor] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    (cursor, buffer.len() - cursor)
}

unsafe fn float_string_f32(value: f32) -> *mut DrStringV1 {
    if value.is_nan() {
        return string_from_static(b"NaN");
    }
    if value == f32::INFINITY {
        return string_from_static(b"Infinity");
    }
    if value == f32::NEG_INFINITY {
        return string_from_static(b"-Infinity");
    }
    if value == 0.0 {
        return string_from_static(if value.is_sign_negative() {
            b"-0"
        } else {
            b"0"
        });
    }
    let mut buffer = ryu::Buffer::new();
    let text = buffer.format_finite(value);
    dr_v1_string_from_utf8(text.as_ptr(), text.len())
}

unsafe fn float_string_f64(value: f64) -> *mut DrStringV1 {
    if value.is_nan() {
        return string_from_static(b"NaN");
    }
    if value == f64::INFINITY {
        return string_from_static(b"Infinity");
    }
    if value == f64::NEG_INFINITY {
        return string_from_static(b"-Infinity");
    }
    if value == 0.0 {
        return string_from_static(if value.is_sign_negative() {
            b"-0"
        } else {
            b"0"
        });
    }
    let mut buffer = ryu::Buffer::new();
    let text = buffer.format_finite(value);
    dr_v1_string_from_utf8(text.as_ptr(), text.len())
}

unsafe fn string_from_static(bytes: &[u8]) -> *mut DrStringV1 {
    dr_v1_string_from_utf8(bytes.as_ptr(), bytes.len())
}

fn signed_decimal(value: i64, buffer: &mut [u8; 20]) -> (usize, usize) {
    let negative = value < 0;
    let magnitude = value.unsigned_abs();
    let (mut start, mut length) = unsigned_decimal(magnitude, buffer);
    if negative {
        start -= 1;
        buffer[start] = b'-';
        length += 1;
    }
    (start, length)
}

fn unsigned_decimal(mut value: u64, buffer: &mut [u8; 20]) -> (usize, usize) {
    let mut cursor = buffer.len();
    loop {
        cursor -= 1;
        buffer[cursor] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    (cursor, buffer.len() - cursor)
}

unsafe fn string_bytes(string: *const DrStringV1) -> *const u8 {
    string.cast::<u8>().add(STRING_HEADER_SIZE)
}

pub(crate) unsafe fn string_bytes_mut(string: *mut DrStringV1) -> *mut u8 {
    string.cast::<u8>().add(STRING_HEADER_SIZE)
}

/// Parses `text` as a base-10 64-bit signed integer, ignoring surrounding ASCII
/// whitespace. Writes `1` to `found` and returns the value reinterpreted as a
/// `u64` word on success; writes `0` and returns `0` when the text is not a valid
/// `int` (including out-of-range values and non-UTF-8 bytes).
///
/// # Safety
/// `text` must be null or a valid `DrStringV1` pointer; `found` must be writable.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_int_parse(text: *const DrStringV1, found: *mut u8) -> u64 {
    *found = 0;
    if text.is_null() {
        return 0;
    }
    let bytes = core::slice::from_raw_parts(string_bytes(text), (*text).byte_length);
    let Ok(text) = core::str::from_utf8(bytes) else {
        return 0;
    };
    match text.trim().parse::<i64>() {
        Ok(value) => {
            *found = 1;
            value as u64
        }
        Err(_) => 0,
    }
}

/// Parses `text` as a 64-bit float, ignoring surrounding ASCII whitespace. Writes
/// `1` to `found` and returns the IEEE-754 bit pattern on success; writes `0` and
/// returns `0` when the text is not a valid `float` (including non-UTF-8 bytes).
///
/// # Safety
/// `text` must be null or a valid `DrStringV1` pointer; `found` must be writable.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_float_parse(text: *const DrStringV1, found: *mut u8) -> u64 {
    *found = 0;
    if text.is_null() {
        return 0;
    }
    let bytes = core::slice::from_raw_parts(string_bytes(text), (*text).byte_length);
    let Ok(text) = core::str::from_utf8(bytes) else {
        return 0;
    };
    match text.trim().parse::<f64>() {
        Ok(value) => {
            *found = 1;
            value.to_bits()
        }
        Err(_) => 0,
    }
}

unsafe fn string_result_too_large() -> ! {
    panic_catalogued(ptr::null(), b"P1205")
}

unsafe fn runtime_invariant_panic() -> ! {
    panic_catalogued(ptr::null(), b"P1001")
}

#[cfg(unix)]
unsafe fn allocate(byte_length: usize) -> *mut u8 {
    malloc(byte_length).cast::<u8>()
}
#[cfg(unix)]
unsafe fn deallocate(memory: *mut u8) {
    free(memory.cast::<c_void>());
}

#[cfg(windows)]
unsafe fn allocate(byte_length: usize) -> *mut u8 {
    HeapAlloc(GetProcessHeap(), 0, byte_length).cast::<u8>()
}
#[cfg(windows)]
unsafe fn deallocate(memory: *mut u8) {
    let _ = HeapFree(GetProcessHeap(), 0, memory.cast::<c_void>());
}

#[cfg(not(any(unix, windows)))]
unsafe fn allocate(_byte_length: usize) -> *mut u8 {
    ptr::null_mut()
}
#[cfg(not(any(unix, windows)))]
unsafe fn deallocate(_memory: *mut u8) {}

#[cfg(unix)]
unsafe fn ignore_sigpipe() {
    // Ignoring it makes write(2) report EPIPE instead of terminating the process by signal.
    signal(SIGPIPE, SIG_IGN);
}

unsafe fn write_panic_fragment(bytes: &[u8]) {
    write_panic_bytes(bytes.as_ptr(), bytes.len());
}

unsafe fn write_panic_bytes(bytes: *const u8, byte_length: usize) {
    // Panic diagnostics are best effort; their sink must not replace the fatal status.
    let _ = write_standard_stream(StandardStream::Stderr, bytes, byte_length);
}

unsafe fn write_standard_stream(
    stream: StandardStream,
    bytes: *const u8,
    byte_length: usize,
) -> WriteOutcome {
    #[cfg(unix)]
    ignore_sigpipe();

    device_io::write(stream, bytes, byte_length)
}

unsafe fn write_byte_stream(
    current_frame: *const DrStackFrameV2,
    stream: StandardStream,
    data: *const u8,
    length: usize,
    failure: &'static [u8],
) {
    #[cfg(unix)]
    ignore_sigpipe();

    match device_io::write_bytes(stream, data, length) {
        WriteOutcome::Success => {}
        WriteOutcome::BrokenPipe => exit_process(0),
        WriteOutcome::OtherFailure => {
            let _ = failure;
            panic_catalogued(current_frame, b"P1407")
        }
    }
}

#[cfg(unix)]
unsafe fn exit_process(status: i32) -> ! {
    _exit(status)
}

#[cfg(windows)]
unsafe fn exit_process(status: i32) -> ! {
    ExitProcess(status as u32)
}

#[cfg(not(any(unix, windows)))]
unsafe fn exit_process(_status: i32) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

#[cfg(unix)]
extern "C" {
    fn signal(signal: i32, handler: usize) -> usize;
    fn getenv(name: *const u8) -> *const u8;
    fn _exit(status: i32) -> !;
    fn malloc(byte_length: usize) -> *mut c_void;
    fn free(memory: *mut c_void);
}

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn GetEnvironmentVariableA(name: *const u8, buffer: *mut u8, size: u32) -> u32;
}

// Doria's Windows executables deliberately do not link the C runtime. Rust and ryu still lower
// byte copies/fills and floating-point use to these MSVC support symbols, so the runtime owns the
// small subset they require. Hosted Rust binaries link the CRT instead and disable this feature
// through their dependency declaration so both providers can never define the same symbol.
#[cfg(all(windows, feature = "standalone-windows-support"))]
#[no_mangle]
pub static _fltused: i32 = 0;

// LLVM emits this MSVC stack-probe call when a generated x86-64 function reserves more than one
// page of stack. Probe each page before the function adjusts RSP so Windows can extend the stack's
// guard region. The MSVC convention passes the allocation size in RAX and preserves RAX and RCX.
#[cfg(all(
    windows,
    target_env = "msvc",
    target_arch = "x86_64",
    feature = "standalone-windows-support"
))]
core::arch::global_asm!(
    r#"
    .text
    .def __chkstk; .scl 2; .type 32; .endef
    .globl __chkstk
    .p2align 4, 0x90
__chkstk:
    push rax
    push rcx
    cmp rax, 0x1000
    lea rcx, [rsp + 24]
    jb 2f
1:
    sub rcx, 0x1000
    test qword ptr [rcx], rcx
    sub rax, 0x1000
    cmp rax, 0x1000
    ja 1b
2:
    sub rcx, rax
    test qword ptr [rcx], rcx
    pop rcx
    pop rax
    ret
"#
);

/// Copies `count` bytes from `source` to the non-overlapping `destination`.
///
/// # Safety
///
/// `source` and `destination` must be valid for `count` bytes and must not overlap.
#[cfg(all(windows, feature = "standalone-windows-support"))]
#[no_mangle]
pub unsafe extern "C" fn memcpy(
    destination: *mut c_void,
    source: *const c_void,
    count: usize,
) -> *mut c_void {
    let destination_bytes = destination.cast::<u8>();
    let source_bytes = source.cast::<u8>();
    for index in 0..count {
        let byte = ptr::read_volatile(source_bytes.add(index));
        ptr::write_volatile(destination_bytes.add(index), byte);
    }
    destination
}

/// Copies `count` bytes from `source` to `destination`, including when they overlap.
///
/// # Safety
///
/// `source` and `destination` must be valid for `count` bytes.
#[cfg(all(windows, feature = "standalone-windows-support"))]
#[no_mangle]
pub unsafe extern "C" fn memmove(
    destination: *mut c_void,
    source: *const c_void,
    count: usize,
) -> *mut c_void {
    let destination_bytes = destination.cast::<u8>();
    let source_bytes = source.cast::<u8>();
    let destination_address = destination_bytes as usize;
    let source_address = source_bytes as usize;

    if destination_address <= source_address
        || destination_address.wrapping_sub(source_address) >= count
    {
        for index in 0..count {
            let byte = ptr::read_volatile(source_bytes.add(index));
            ptr::write_volatile(destination_bytes.add(index), byte);
        }
    } else {
        for index in (0..count).rev() {
            let byte = ptr::read_volatile(source_bytes.add(index));
            ptr::write_volatile(destination_bytes.add(index), byte);
        }
    }
    destination
}

/// Compares `count` bytes lexicographically as unsigned values.
///
/// # Safety
///
/// `left` and `right` must both be valid for reads of `count` bytes.
#[cfg(all(windows, feature = "standalone-windows-support"))]
#[no_mangle]
pub unsafe extern "C" fn memcmp(left: *const c_void, right: *const c_void, count: usize) -> i32 {
    let left = left.cast::<u8>();
    let right = right.cast::<u8>();
    for index in 0..count {
        let left_byte = ptr::read_volatile(left.add(index));
        let right_byte = ptr::read_volatile(right.add(index));
        if left_byte != right_byte {
            return i32::from(left_byte) - i32::from(right_byte);
        }
    }
    0
}

/// Returns the byte length of a null-terminated string.
///
/// # Safety
///
/// `value` must point to a readable null-terminated byte string.
#[cfg(all(windows, feature = "standalone-windows-support"))]
#[no_mangle]
pub unsafe extern "C" fn strlen(value: *const u8) -> usize {
    let mut length = 0;
    while ptr::read_volatile(value.add(length)) != 0 {
        length += 1;
    }
    length
}

/// Lets Windows continue searching when precompiled `core` unwind metadata is inspected.
///
/// Doria's runtime is abort-only and never initiates SEH/C++ unwinding. The Rust-distributed
/// `core` archive can nevertheless reference the MSVC language-specific handler, while Doria
/// deliberately links without the CRT. Returning `ExceptionContinueSearch` preserves the
/// abort-only boundary if an unrelated structured exception reaches this metadata.
///
/// # Safety
///
/// This function may only be entered by the Windows exception dispatcher with its four native
/// dispatcher pointers. Doria code must never call it directly.
#[cfg(all(windows, target_env = "msvc", feature = "standalone-windows-support"))]
#[no_mangle]
pub unsafe extern "C" fn __CxxFrameHandler3(
    _exception_record: *mut c_void,
    _establisher_frame: *mut c_void,
    _context_record: *mut c_void,
    _dispatcher_context: *mut c_void,
) -> i32 {
    const EXCEPTION_CONTINUE_SEARCH: i32 = 1;
    EXCEPTION_CONTINUE_SEARCH
}

/// Fills `count` bytes at `destination` with the low byte of `value`.
///
/// # Safety
///
/// `destination` must be valid for writes of `count` bytes.
#[cfg(all(windows, feature = "standalone-windows-support"))]
#[no_mangle]
pub unsafe extern "C" fn memset(destination: *mut c_void, value: i32, count: usize) -> *mut c_void {
    let destination_bytes = destination.cast::<u8>();
    for index in 0..count {
        ptr::write_volatile(destination_bytes.add(index), value as u8);
    }
    destination
}

#[cfg(windows)]
extern "system" {
    fn GetProcessHeap() -> *mut c_void;
    fn HeapAlloc(heap: *mut c_void, flags: u32, byte_length: usize) -> *mut c_void;
    fn HeapFree(heap: *mut c_void, flags: u32, memory: *mut c_void) -> i32;
    fn ExitProcess(status: u32) -> !;
}

#[cfg(all(not(test), panic = "abort"))]
#[panic_handler]
fn rust_panic(_information: &core::panic::PanicInfo<'_>) -> ! {
    unsafe { exit_process(PANIC_STATUS) }
}

#[cfg(all(not(test), panic = "abort"))]
#[no_mangle]
pub extern "C" fn rust_eh_personality() {}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::Cell;

    std::thread_local! {
        static SHARED_PAYLOAD_DROPS: Cell<usize> = const { Cell::new(0) };
    }

    fn reset_shared_payload_drops() {
        SHARED_PAYLOAD_DROPS.set(0);
    }

    fn shared_payload_drops() -> usize {
        SHARED_PAYLOAD_DROPS.get()
    }

    unsafe extern "C" fn drop_test_shared_payload(
        _current_frame: *const DrStackFrameV2,
        payload: *mut u8,
    ) {
        SHARED_PAYLOAD_DROPS.set(SHARED_PAYLOAD_DROPS.get() + 1);
        dr_v1_class_free(payload);
    }

    unsafe fn bytes(string: *const DrStringV1) -> &'static [u8] {
        core::slice::from_raw_parts(dr_v1_string_data(string), dr_v1_string_length(string))
    }

    #[test]
    fn stack_frame_v2_layout_is_source_aware() {
        assert_eq!(
            core::mem::size_of::<DrStackFrameV2>(),
            11 * core::mem::size_of::<usize>()
        );
        assert_eq!(
            core::mem::align_of::<DrStackFrameV2>(),
            core::mem::align_of::<usize>()
        );
    }

    #[test]
    fn headerless_class_allocation_handles_empty_and_nonempty_payloads() {
        unsafe {
            for size in [0, 1, 24] {
                let payload = dr_v2_class_allocate(ptr::null(), size, 8);
                assert!(!payload.is_null());
                if size > 0 {
                    assert!(core::slice::from_raw_parts(payload, size)
                        .iter()
                        .all(|byte| *byte == 0));
                    ptr::write_bytes(payload, 0xa5, size);
                }
                dr_v1_class_free(payload);
            }
        }
    }

    #[test]
    fn shared_control_drops_payload_once_and_outlives_it_for_weak_references() {
        unsafe {
            reset_shared_payload_drops();
            let payload = dr_v2_class_allocate(ptr::null(), 8, 8);
            let control = dr_v2_shared_create(ptr::null(), payload, drop_test_shared_payload);

            assert_eq!((*control).strong_references, 1);
            assert_eq!((*control).weak_references, 0);
            assert_eq!(dr_v1_shared_payload(control), payload);

            assert_eq!(dr_v2_shared_retain(ptr::null(), control), control);
            assert_eq!(dr_v2_shared_create_weak(ptr::null(), control), control);
            assert_eq!((*control).strong_references, 2);
            assert_eq!((*control).weak_references, 1);

            dr_v2_shared_release(ptr::null(), control);
            let acquired = dr_v2_shared_acquire(ptr::null(), control);
            assert_eq!(acquired, control);
            assert_eq!((*control).strong_references, 2);

            dr_v2_shared_release(ptr::null(), acquired);
            dr_v2_shared_release(ptr::null(), control);
            assert_eq!(shared_payload_drops(), 1);
            assert_eq!((*control).strong_references, 0);
            assert!((*control).payload.is_null());
            assert!(dr_v2_shared_acquire(ptr::null(), control).is_null());

            dr_v1_shared_release_weak(control);
        }
    }

    #[test]
    fn writable_shared_control_tracks_ownership_access_and_payload_lifetime() {
        unsafe {
            reset_shared_payload_drops();
            let payload = dr_v2_class_allocate(ptr::null(), 8, 8);
            let control =
                dr_v2_writable_shared_create(ptr::null(), payload, drop_test_shared_payload);

            assert_eq!((*control).strong_references, 1);
            assert_eq!((*control).weak_references, 0);
            assert_eq!((*control).readonly_accesses, 0);
            assert!(!(*control).writable_access_active);

            assert_eq!(dr_v2_writable_shared_retain(ptr::null(), control), control);
            assert_eq!(
                dr_v2_writable_shared_create_weak(ptr::null(), control),
                control
            );
            assert_eq!((*control).strong_references, 2);
            assert_eq!((*control).weak_references, 1);

            let first = dr_v2_writable_shared_acquire_readonly_access(ptr::null(), control);
            let second = dr_v2_writable_shared_acquire_readonly_access(ptr::null(), control);
            assert_eq!(first, control);
            assert_eq!(second, control);
            assert_eq!((*control).strong_references, 4);
            assert_eq!((*control).readonly_accesses, 2);
            assert_eq!(dr_v1_writable_shared_readonly_payload(first), payload);

            dr_v2_writable_shared_release_readonly_access(ptr::null(), first);
            dr_v2_writable_shared_release_readonly_access(ptr::null(), second);
            assert_eq!((*control).strong_references, 2);
            assert_eq!((*control).readonly_accesses, 0);

            let writable = dr_v2_writable_shared_acquire_writable_access(ptr::null(), control);
            assert_eq!((*control).strong_references, 3);
            assert!((*control).writable_access_active);
            assert_eq!(dr_v1_writable_shared_writable_payload(writable), payload);
            dr_v2_writable_shared_release_writable_access(ptr::null(), writable);
            assert_eq!((*control).strong_references, 2);
            assert!(!(*control).writable_access_active);

            dr_v2_writable_shared_release(ptr::null(), control);
            let acquired = dr_v2_writable_shared_acquire(ptr::null(), control);
            assert_eq!(acquired, control);
            dr_v2_writable_shared_release(ptr::null(), acquired);
            dr_v2_writable_shared_release(ptr::null(), control);

            assert_eq!(shared_payload_drops(), 1);
            assert_eq!((*control).strong_references, 0);
            assert!((*control).payload.is_null());
            assert!(dr_v2_writable_shared_acquire(ptr::null(), control).is_null());

            dr_v1_writable_shared_release_weak(control);
        }
    }

    #[test]
    fn writable_access_object_can_be_the_final_strong_owner() {
        unsafe {
            reset_shared_payload_drops();
            let payload = dr_v2_class_allocate(ptr::null(), 8, 8);
            let control =
                dr_v2_writable_shared_create(ptr::null(), payload, drop_test_shared_payload);
            let weak = dr_v2_writable_shared_create_weak(ptr::null(), control);
            let access = dr_v2_writable_shared_acquire_readonly_access(ptr::null(), control);

            dr_v2_writable_shared_release(ptr::null(), control);
            let acquired = dr_v2_writable_shared_acquire(ptr::null(), weak);
            assert_eq!(acquired, control);
            dr_v2_writable_shared_release(ptr::null(), acquired);
            assert_eq!(shared_payload_drops(), 0);

            dr_v2_writable_shared_release_readonly_access(ptr::null(), access);
            assert_eq!(shared_payload_drops(), 1);
            assert!(dr_v2_writable_shared_acquire(ptr::null(), weak).is_null());
            dr_v1_writable_shared_release_weak(weak);
        }
    }

    #[test]
    fn explicit_lengths_preserve_empty_embedded_nul_and_utf8() {
        unsafe {
            for expected in [b"".as_slice(), b"a\0b".as_slice(), "Dória".as_bytes()] {
                let string = dr_v1_string_from_utf8(expected.as_ptr(), expected.len());
                assert_eq!(bytes(string), expected);
                dr_v1_string_release(string);
            }
        }
    }

    #[test]
    fn runtime_source_markers_use_terminal_display_width() {
        assert_eq!(display_width_with_tabs(b"\t", 4), 4);
        assert_eq!(display_width_with_tabs("e\u{301}".as_bytes(), 4), 1);
        assert_eq!(display_width_with_tabs("🙂".as_bytes(), 4), 2);
        assert_eq!(display_width_with_tabs("界".as_bytes(), 4), 2);
        assert_eq!(display_width_with_tabs(b"a\tb", 4), 5);
    }

    #[test]
    fn retain_release_and_concat_preserve_immutable_values() {
        unsafe {
            let left = dr_v1_string_from_utf8(b"Dor".as_ptr(), 3);
            let retained = dr_v1_string_retain(left);
            let right = dr_v1_string_from_utf8(b"ia".as_ptr(), 2);
            let joined = dr_v1_string_concat(left, right);
            assert_eq!(bytes(joined), b"Doria");
            assert_eq!(dr_v1_string_compare(left, retained), 0);
            dr_v1_string_release(left);
            dr_v1_string_release(retained);
            dr_v1_string_release(right);
            dr_v1_string_release(joined);
        }
    }

    #[test]
    fn sequence_fill_copies_words_and_retains_each_string_slot() {
        unsafe {
            for (width, value) in [
                (1, 0xabu64),
                (2, 0xabcdu64),
                (4, 0xabcdef01u64),
                (8, 0xabcdef0123456789u64),
            ] {
                let words = dr_v2_collection_fill_word(ptr::null(), value, 3, 1, width);
                assert_eq!(dr_v1_collection_length(words), 3);
                for index in 0..3 {
                    assert_eq!(dr_v2_collection_value_at(ptr::null(), words, index), value);
                }
                dr_v1_collection_free(words);
            }

            let string = dr_v1_string_from_utf8(b"shared".as_ptr(), 6);
            let strings = dr_v2_collection_fill_string(ptr::null(), string, 3, 0);
            assert_eq!((*string).references, 4);
            for index in 0..3 {
                let slot =
                    dr_v2_collection_value_at(ptr::null(), strings, index) as *mut DrStringV1;
                assert_eq!(slot, string);
                dr_v1_string_release(slot);
            }
            assert_eq!((*string).references, 1);
            dr_v1_collection_free(strings);
            dr_v1_string_release(string);
        }
    }

    #[test]
    fn retain_and_release_leave_compile_time_strings_immortal() {
        unsafe {
            let mut string = DrStringV1 {
                references: IMMORTAL_STRING_REFERENCES,
                byte_length: 0,
            };
            let pointer = &mut string as *mut DrStringV1;
            assert_eq!(dr_v1_string_retain(pointer), pointer);
            assert_eq!(string.references, IMMORTAL_STRING_REFERENCES);
            dr_v1_string_release(pointer);
            assert_eq!(string.references, IMMORTAL_STRING_REFERENCES);
        }
    }

    #[test]
    fn canonical_primitive_display_is_exact() {
        unsafe {
            let cases = [
                (
                    dr_v1_string_from_i64(i64::MIN),
                    b"-9223372036854775808".as_slice(),
                ),
                (
                    dr_v1_string_from_u64(u64::MAX),
                    b"18446744073709551615".as_slice(),
                ),
                (dr_v1_string_from_bool(0), b"false".as_slice()),
                (dr_v1_string_from_bool(1), b"true".as_slice()),
                (dr_v1_string_from_f32(-0.0), b"-0".as_slice()),
                (dr_v1_string_from_f64(f64::NAN), b"NaN".as_slice()),
                (dr_v1_string_from_f64(f64::INFINITY), b"Infinity".as_slice()),
                (
                    dr_v1_string_from_f64(f64::NEG_INFINITY),
                    b"-Infinity".as_slice(),
                ),
            ];
            for (string, expected) in cases {
                assert_eq!(bytes(string), expected);
                dr_v1_string_release(string);
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn no_crt_memory_support_symbols_preserve_bytes_and_overlap() {
        unsafe {
            let source = [1_u8, 2, 3, 4];
            let mut copied = [0_u8; 4];
            memcpy(
                copied.as_mut_ptr().cast(),
                source.as_ptr().cast(),
                source.len(),
            );
            assert_eq!(copied, source);

            memset(copied.as_mut_ptr().cast(), 0xab, copied.len());
            assert_eq!(copied, [0xab; 4]);

            let mut moved = [1_u8, 2, 3, 4, 5];
            memmove(moved.as_mut_ptr().add(1).cast(), moved.as_ptr().cast(), 4);
            assert_eq!(moved, [1, 1, 2, 3, 4]);

            memmove(moved.as_mut_ptr().cast(), moved.as_ptr().add(1).cast(), 4);
            assert_eq!(moved, [1, 2, 3, 4, 4]);

            assert_eq!(memcmp(b"abc".as_ptr().cast(), b"abc".as_ptr().cast(), 3), 0);
            assert!(memcmp(b"abc".as_ptr().cast(), b"abd".as_ptr().cast(), 3) < 0);
            assert!(memcmp(b"abe".as_ptr().cast(), b"abd".as_ptr().cast(), 3) > 0);
            assert_eq!(strlen(b"\0".as_ptr()), 0);
            assert_eq!(strlen(b"doria\0".as_ptr()), 5);

            #[cfg(target_env = "msvc")]
            assert_eq!(
                __CxxFrameHandler3(
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                ),
                1
            );
        }
    }
}
