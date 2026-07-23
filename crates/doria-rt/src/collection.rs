use core::mem;
use core::ptr;

use crate::{allocate, deallocate, dr_v1_panic, DrStackFrameV1, DrStringV1};

#[repr(C)]
pub struct DrCollectionV1 {
    length: usize,
    capacity: usize,
    keys: *mut u64,
    values: *mut u64,
    keyed: u8,
    fixed: u8,
}

unsafe fn allocate_words(capacity: usize) -> *mut u64 {
    if capacity == 0 {
        return ptr::null_mut();
    }
    let bytes = capacity
        .checked_mul(mem::size_of::<u64>())
        .unwrap_or_else(|| collection_panic(b"collection capacity overflow"));
    let words = allocate(bytes).cast::<u64>();
    if words.is_null() {
        collection_panic(b"collection allocation failed");
    }
    ptr::write_bytes(words, 0, capacity);
    words
}

unsafe fn grow(collection: *mut DrCollectionV1) {
    if (*collection).fixed != 0 {
        collection_panic(b"cannot grow a fixed-length typed array");
    }
    let next = (*collection)
        .capacity
        .checked_mul(2)
        .unwrap_or_else(|| collection_panic(b"collection capacity overflow"))
        .max(4);
    let values = allocate_words(next);
    if (*collection).length != 0 {
        ptr::copy_nonoverlapping((*collection).values, values, (*collection).length);
    }
    if !(*collection).values.is_null() {
        deallocate((*collection).values.cast::<u8>());
    }
    (*collection).values = values;

    if (*collection).keyed != 0 {
        let keys = allocate_words(next);
        if (*collection).length != 0 {
            ptr::copy_nonoverlapping((*collection).keys, keys, (*collection).length);
        }
        if !(*collection).keys.is_null() {
            deallocate((*collection).keys.cast::<u8>());
        }
        (*collection).keys = keys;
    }
    (*collection).capacity = next;
}

pub unsafe fn new(length: usize, keyed: bool, fixed: bool) -> *mut DrCollectionV1 {
    let capacity = if fixed { length } else { length.max(4) };
    let collection = allocate(mem::size_of::<DrCollectionV1>()).cast::<DrCollectionV1>();
    if collection.is_null() {
        collection_panic(b"collection allocation failed");
    }
    ptr::write(
        collection,
        DrCollectionV1 {
            length: if fixed { length } else { 0 },
            capacity,
            keys: if keyed {
                allocate_words(capacity)
            } else {
                ptr::null_mut()
            },
            values: allocate_words(capacity),
            keyed: u8::from(keyed),
            fixed: u8::from(fixed),
        },
    );
    collection
}

pub unsafe fn free(collection: *mut DrCollectionV1) {
    if collection.is_null() {
        return;
    }
    if !(*collection).keys.is_null() {
        deallocate((*collection).keys.cast::<u8>());
    }
    if !(*collection).values.is_null() {
        deallocate((*collection).values.cast::<u8>());
    }
    deallocate(collection.cast::<u8>());
}

pub unsafe fn length(collection: *const DrCollectionV1) -> usize {
    (*collection).length
}

pub unsafe fn push(collection: *mut DrCollectionV1, value: u64) {
    if (*collection).length == (*collection).capacity {
        grow(collection);
    }
    ptr::write((*collection).values.add((*collection).length), value);
    (*collection).length += 1;
}

pub unsafe fn insert_at(
    frame: *const DrStackFrameV1,
    collection: *mut DrCollectionV1,
    index: usize,
    value: u64,
) {
    if index > (*collection).length {
        static MESSAGE: &[u8] = b"collection index out of bounds";
        dr_v1_panic(frame, MESSAGE.as_ptr(), MESSAGE.len());
    }
    if (*collection).length == (*collection).capacity {
        grow(collection);
    }
    let tail = (*collection).length - index;
    if tail != 0 {
        ptr::copy(
            (*collection).values.add(index),
            (*collection).values.add(index + 1),
            tail,
        );
    }
    *(*collection).values.add(index) = value;
    (*collection).length += 1;
}

pub unsafe fn remove_at(
    frame: *const DrStackFrameV1,
    collection: *mut DrCollectionV1,
    index: usize,
) -> u64 {
    if index >= (*collection).length {
        static MESSAGE: &[u8] = b"collection index out of bounds";
        dr_v1_panic(frame, MESSAGE.as_ptr(), MESSAGE.len());
    }
    let removed = *(*collection).values.add(index);
    let tail = (*collection).length - index - 1;
    if tail != 0 {
        ptr::copy(
            (*collection).values.add(index + 1),
            (*collection).values.add(index),
            tail,
        );
    }
    (*collection).length -= 1;
    removed
}

pub unsafe fn pop(collection: *mut DrCollectionV1, found: *mut u8) -> u64 {
    if (*collection).length == 0 {
        *found = 0;
        return 0;
    }
    *found = 1;
    (*collection).length -= 1;
    *(*collection).values.add((*collection).length)
}

pub unsafe fn value_at(
    frame: *const DrStackFrameV1,
    collection: *const DrCollectionV1,
    index: usize,
) -> u64 {
    if index >= (*collection).length {
        static MESSAGE: &[u8] = b"collection index out of bounds";
        dr_v1_panic(frame, MESSAGE.as_ptr(), MESSAGE.len());
    }
    *(*collection).values.add(index)
}

pub unsafe fn key_at(
    frame: *const DrStackFrameV1,
    collection: *const DrCollectionV1,
    index: usize,
) -> u64 {
    if (*collection).keyed == 0 {
        collection_panic(b"collection does not have keys");
    }
    if index >= (*collection).length {
        dr_v1_panic(
            frame,
            b"collection index out of bounds".as_ptr(),
            b"collection index out of bounds".len(),
        );
    }
    *(*collection).keys.add(index)
}

pub unsafe fn set_at(
    frame: *const DrStackFrameV1,
    collection: *mut DrCollectionV1,
    index: usize,
    value: u64,
) -> u64 {
    if index >= (*collection).length {
        static MESSAGE: &[u8] = b"collection index out of bounds";
        dr_v1_panic(frame, MESSAGE.as_ptr(), MESSAGE.len());
    }
    let slot = (*collection).values.add(index);
    let previous = *slot;
    *slot = value;
    previous
}

unsafe fn keys_equal(left: u64, right: u64, key_kind: u8) -> bool {
    match key_kind {
        1 => {
            let left = left as *const DrStringV1;
            let right = right as *const DrStringV1;
            crate::string_equal(left, right)
        }
        _ => left == right,
    }
}

unsafe fn find(collection: *const DrCollectionV1, key: u64, key_kind: u8) -> Option<usize> {
    (0..(*collection).length)
        .find(|index| keys_equal(*(*collection).keys.add(*index), key, key_kind))
}

pub unsafe fn keyed_get(
    collection: *const DrCollectionV1,
    key: u64,
    key_kind: u8,
    found: *mut u8,
) -> u64 {
    if let Some(index) = find(collection, key, key_kind) {
        *found = 1;
        *(*collection).values.add(index)
    } else {
        *found = 0;
        0
    }
}

pub unsafe fn keyed_set(
    collection: *mut DrCollectionV1,
    key: u64,
    value: u64,
    key_kind: u8,
    replaced: *mut u8,
) -> u64 {
    if let Some(index) = find(collection, key, key_kind) {
        *replaced = 1;
        let slot = (*collection).values.add(index);
        let previous = *slot;
        *slot = value;
        return previous;
    }
    *replaced = 0;
    if (*collection).length == (*collection).capacity {
        grow(collection);
    }
    let index = (*collection).length;
    *(*collection).keys.add(index) = key;
    *(*collection).values.add(index) = value;
    (*collection).length += 1;
    0
}

pub unsafe fn keyed_has(collection: *const DrCollectionV1, key: u64, key_kind: u8) -> bool {
    find(collection, key, key_kind).is_some()
}

pub unsafe fn keyed_remove(
    collection: *mut DrCollectionV1,
    key: u64,
    key_kind: u8,
    found: *mut u8,
    removed_key: *mut u64,
) -> u64 {
    let Some(index) = find(collection, key, key_kind) else {
        *found = 0;
        *removed_key = 0;
        return 0;
    };
    *found = 1;
    *removed_key = *(*collection).keys.add(index);
    let removed_value = *(*collection).values.add(index);
    let tail = (*collection).length - index - 1;
    if tail != 0 {
        ptr::copy(
            (*collection).keys.add(index + 1),
            (*collection).keys.add(index),
            tail,
        );
        ptr::copy(
            (*collection).values.add(index + 1),
            (*collection).values.add(index),
            tail,
        );
    }
    (*collection).length -= 1;
    removed_value
}

pub unsafe fn nullable_access(
    collection: *mut DrCollectionV1,
    key: u64,
    key_kind: u8,
    access: u8,
    found: *mut u8,
    removed_key: *mut u64,
) -> u64 {
    *removed_key = 0;
    match access {
        0 => keyed_get(collection, key, key_kind, found),
        1 => keyed_remove(collection, key, key_kind, found, removed_key),
        2 => {
            if (*collection).length == 0 {
                *found = 0;
                0
            } else {
                *found = 1;
                *(*collection).values
            }
        }
        3 => {
            if (*collection).length == 0 {
                *found = 0;
                0
            } else {
                *found = 1;
                *(*collection).values.add((*collection).length - 1)
            }
        }
        4 => pop(collection, found),
        _ => collection_panic(b"invalid nullable collection access"),
    }
}

pub unsafe fn contains(collection: *const DrCollectionV1, value: u64, value_kind: u8) -> bool {
    (0..(*collection).length)
        .any(|index| keys_equal(*(*collection).values.add(index), value, value_kind))
}

pub unsafe fn push_unique(collection: *mut DrCollectionV1, value: u64, value_kind: u8) -> bool {
    if contains(collection, value, value_kind) {
        false
    } else {
        push(collection, value);
        true
    }
}

pub unsafe fn remove_value(
    collection: *mut DrCollectionV1,
    value: u64,
    value_kind: u8,
    removed: *mut u64,
) -> bool {
    let Some(index) = (0..(*collection).length)
        .find(|index| keys_equal(*(*collection).values.add(*index), value, value_kind))
    else {
        *removed = 0;
        return false;
    };
    *removed = *(*collection).values.add(index);
    let tail = (*collection).length - index - 1;
    if tail != 0 {
        ptr::copy(
            (*collection).values.add(index + 1),
            (*collection).values.add(index),
            tail,
        );
    }
    (*collection).length -= 1;
    true
}

pub unsafe fn set_algebra(
    left: *const DrCollectionV1,
    right: *const DrCollectionV1,
    operation: u8,
    value_kind: u8,
) -> *mut DrCollectionV1 {
    let result = new(0, false, false);
    for index in 0..(*left).length {
        let value = *(*left).values.add(index);
        let include = match operation {
            0 => true,
            1 => contains(right, value, value_kind),
            2 => !contains(right, value, value_kind),
            _ => collection_panic(b"invalid set operation"),
        };
        if include {
            push_retained(result, value, value_kind);
        }
    }
    if operation == 0 {
        for index in 0..(*right).length {
            let value = *(*right).values.add(index);
            if !contains(result, value, value_kind) {
                push_retained(result, value, value_kind);
            }
        }
    }
    result
}

unsafe fn push_retained(collection: *mut DrCollectionV1, value: u64, value_kind: u8) {
    if value_kind == 1 {
        crate::dr_v1_string_retain(value as *mut DrStringV1);
    }
    push(collection, value);
}

fn collection_panic(message: &'static [u8]) -> ! {
    unsafe { dr_v1_panic(ptr::null(), message.as_ptr(), message.len()) }
}
