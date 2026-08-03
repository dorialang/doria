use core::mem;
use core::ptr;

use crate::{
    allocate, deallocate, dr_v2_panic_code, dr_v2_panic_index_out_of_bounds, DrStackFrameV2,
    DrStringV1,
};

const COMPARE_STRING: u8 = 1;
const COMPARE_FLOAT32: u8 = 2;
const COMPARE_FLOAT64: u8 = 3;
const COMPARE_SIGNED_8: u8 = 4;
const COMPARE_SIGNED_16: u8 = 5;
const COMPARE_SIGNED_32: u8 = 6;
const COMPARE_SIGNED_64: u8 = 7;
const COMPARE_UNSIGNED_8: u8 = 8;
const COMPARE_UNSIGNED_16: u8 = 9;
const COMPARE_UNSIGNED_32: u8 = 10;
const COMPARE_UNSIGNED_64: u8 = 11;
const COMPARE_BOOL: u8 = 12;

pub const KIND_LEGACY: u8 = 0;
pub const KIND_SORTED_DICTIONARY: u8 = 1;
pub const KIND_SORTED_SET: u8 = 2;
pub const KIND_PRIORITY_QUEUE: u8 = 3;
pub const KIND_DEQUE: u8 = 4;

#[repr(C)]
pub struct DrCollectionV1 {
    length: usize,
    capacity: usize,
    keys: *mut u64,
    values: *mut u8,
    keyed: u8,
    fixed: u8,
    value_width: u8,
    // Compatible tail extension: all V1 field offsets above remain unchanged.
    // Older constructors initialize the legacy behavior; Stage 26 constructors
    // select the specialized semantics explicitly.
    kind: u8,
    comparator: u8,
    finalized: u8,
    value_nullable: u8,
    head: usize,
}

pub const DR_COLLECTION_LENGTH_OFFSET: usize = mem::offset_of!(DrCollectionV1, length);
pub const DR_COLLECTION_CAPACITY_OFFSET: usize = mem::offset_of!(DrCollectionV1, capacity);
pub const DR_COLLECTION_KEYS_OFFSET: usize = mem::offset_of!(DrCollectionV1, keys);
pub const DR_COLLECTION_VALUES_OFFSET: usize = mem::offset_of!(DrCollectionV1, values);
pub const DR_COLLECTION_KEYED_OFFSET: usize = mem::offset_of!(DrCollectionV1, keyed);
pub const DR_COLLECTION_FIXED_OFFSET: usize = mem::offset_of!(DrCollectionV1, fixed);
pub const DR_COLLECTION_VALUE_WIDTH_OFFSET: usize = mem::offset_of!(DrCollectionV1, value_width);

fn valid_value_width(width: u8) -> bool {
    matches!(width, 1 | 2 | 4 | 8 | 16)
}

unsafe fn value_address(collection: *const DrCollectionV1, index: usize) -> *mut u8 {
    let index = if (*collection).kind == KIND_DEQUE && (*collection).capacity != 0 {
        ((*collection).head + index) % (*collection).capacity
    } else {
        index
    };
    (*collection)
        .values
        .add(index * usize::from((*collection).value_width))
}

unsafe fn read_value(collection: *const DrCollectionV1, index: usize) -> u64 {
    let address = value_address(collection, index);
    match (*collection).value_width {
        1 => u64::from(*address),
        2 => u64::from(*address.cast::<u16>()),
        4 => u64::from(*address.cast::<u32>()),
        8 => *address.cast::<u64>(),
        16 => *address.add(8).cast::<u64>(),
        _ => collection_panic(b"P1001"),
    }
}

unsafe fn write_value(collection: *mut DrCollectionV1, index: usize, value: u64) {
    let address = value_address(collection, index);
    match (*collection).value_width {
        1 => *address = value as u8,
        2 => *address.cast::<u16>() = value as u16,
        4 => *address.cast::<u32>() = value as u32,
        8 => *address.cast::<u64>() = value,
        16 => {
            *address.cast::<u64>() = 1;
            *address.add(8).cast::<u64>() = value;
        }
        _ => collection_panic(b"P1001"),
    }
}

unsafe fn read_present(collection: *const DrCollectionV1, index: usize) -> bool {
    if (*collection).value_nullable == 0 {
        true
    } else if (*collection).value_width == 16 {
        *value_address(collection, index).cast::<u64>() != 0
    } else {
        read_value(collection, index) != 0
    }
}

unsafe fn write_nullable_value(
    collection: *mut DrCollectionV1,
    index: usize,
    present: bool,
    value: u64,
) {
    (*collection).value_nullable = 1;
    if (*collection).value_width == 16 {
        let address = value_address(collection, index);
        *address.cast::<u64>() = u64::from(present);
        *address.add(8).cast::<u64>() = value;
    } else {
        write_value(collection, index, if present { value } else { 0 });
    }
}

unsafe fn allocate_words_with_frame(frame: *const DrStackFrameV2, capacity: usize) -> *mut u64 {
    if capacity == 0 {
        return ptr::null_mut();
    }
    let bytes = capacity
        .checked_mul(mem::size_of::<u64>())
        .unwrap_or_else(|| collection_panic_with_frame(frame, b"P1313"));
    let words = allocate(bytes).cast::<u64>();
    if words.is_null() {
        collection_panic_with_frame(frame, b"P1313");
    }
    ptr::write_bytes(words, 0, capacity);
    words
}

unsafe fn allocate_words(capacity: usize) -> *mut u64 {
    allocate_words_with_frame(ptr::null(), capacity)
}

unsafe fn allocate_values_with_frame(
    frame: *const DrStackFrameV2,
    capacity: usize,
    value_width: u8,
) -> *mut u8 {
    if capacity == 0 {
        return ptr::null_mut();
    }
    // Preserve the implementation-wide maximum collection length even when a
    // compact scalar representation needs fewer physical bytes.
    capacity
        .checked_mul(mem::size_of::<u64>())
        .unwrap_or_else(|| collection_panic_with_frame(frame, b"P1313"));
    let bytes = capacity
        .checked_mul(usize::from(value_width))
        .unwrap_or_else(|| collection_panic_with_frame(frame, b"P1313"));
    let values = allocate(bytes);
    if values.is_null() {
        collection_panic_with_frame(frame, b"P1313");
    }
    ptr::write_bytes(values, 0, bytes);
    values
}

unsafe fn grow(collection: *mut DrCollectionV1) {
    if (*collection).fixed != 0 {
        collection_panic(b"P1001");
    }
    let next = (*collection)
        .capacity
        .checked_mul(2)
        .unwrap_or_else(|| collection_panic(b"P1313"))
        .max(4);
    let values = allocate_values_with_frame(ptr::null(), next, (*collection).value_width);
    if (*collection).length != 0 {
        if (*collection).kind == KIND_DEQUE {
            for index in 0..(*collection).length {
                write_raw_value(
                    values,
                    (*collection).value_width,
                    index,
                    read_present(collection, index),
                    read_value(collection, index),
                );
            }
        } else {
            ptr::copy_nonoverlapping(
                (*collection).values,
                values,
                (*collection).length * usize::from((*collection).value_width),
            );
        }
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
    if (*collection).kind == KIND_DEQUE {
        (*collection).head = 0;
    }
}

unsafe fn write_raw_value(values: *mut u8, width: u8, index: usize, present: bool, value: u64) {
    let address = values.add(index * usize::from(width));
    match width {
        1 => *address = value as u8,
        2 => *address.cast::<u16>() = value as u16,
        4 => *address.cast::<u32>() = value as u32,
        8 => *address.cast::<u64>() = value,
        16 => {
            *address.cast::<u64>() = u64::from(present);
            *address.add(8).cast::<u64>() = value;
        }
        _ => collection_panic(b"P1001"),
    }
}

pub unsafe fn new(length: usize, keyed: bool, fixed: bool, value_width: u8) -> *mut DrCollectionV1 {
    new_with_frame(ptr::null(), length, keyed, fixed, value_width)
}

unsafe fn new_with_frame(
    frame: *const DrStackFrameV2,
    length: usize,
    keyed: bool,
    fixed: bool,
    value_width: u8,
) -> *mut DrCollectionV1 {
    if !valid_value_width(value_width) {
        collection_panic_with_frame(frame, b"P1001");
    }
    let capacity = if fixed { length } else { length.max(4) };
    let collection = allocate(mem::size_of::<DrCollectionV1>()).cast::<DrCollectionV1>();
    if collection.is_null() {
        collection_panic_with_frame(frame, b"P1313");
    }
    ptr::write(
        collection,
        DrCollectionV1 {
            length: if fixed { length } else { 0 },
            capacity,
            keys: if keyed {
                allocate_words_with_frame(frame, capacity)
            } else {
                ptr::null_mut()
            },
            values: allocate_values_with_frame(frame, capacity, value_width),
            keyed: u8::from(keyed),
            fixed: u8::from(fixed),
            value_width,
            kind: KIND_LEGACY,
            comparator: 0,
            finalized: 1,
            value_nullable: 0,
            head: 0,
        },
    );
    collection
}

pub unsafe fn new_stage26(
    length: usize,
    keyed: bool,
    value_width: u8,
    kind: u8,
    comparator: u8,
) -> *mut DrCollectionV1 {
    if !matches!(
        kind,
        KIND_SORTED_DICTIONARY | KIND_SORTED_SET | KIND_PRIORITY_QUEUE | KIND_DEQUE
    ) {
        collection_panic(b"P1001");
    }
    let collection = new(length, keyed, false, value_width);
    (*collection).kind = kind;
    (*collection).comparator = comparator;
    (*collection).finalized = 0;
    collection
}

pub unsafe fn fill_word(
    frame: *const DrStackFrameV2,
    value: u64,
    count: usize,
    fixed: bool,
    value_width: u8,
) -> *mut DrCollectionV1 {
    let collection = new_with_frame(frame, count, false, true, value_width);
    for index in 0..count {
        write_value(collection, index, value);
    }
    (*collection).fixed = u8::from(fixed);
    collection
}

pub unsafe fn fill_string(
    frame: *const DrStackFrameV2,
    value: *mut DrStringV1,
    count: usize,
    fixed: bool,
) -> *mut DrCollectionV1 {
    let collection = new_with_frame(
        frame,
        count,
        false,
        true,
        mem::size_of::<*mut DrStringV1>() as u8,
    );
    for index in 0..count {
        let retained = crate::dr_v1_string_retain(value);
        write_value(collection, index, retained as u64);
    }
    (*collection).fixed = u8::from(fixed);
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
    write_value(collection, (*collection).length, value);
    (*collection).length += 1;
    restore_priority_queue_order(collection);
}

pub unsafe fn push_nullable(collection: *mut DrCollectionV1, present: bool, value: u64) {
    if (*collection).length == (*collection).capacity {
        grow(collection);
    }
    write_nullable_value(collection, (*collection).length, present, value);
    (*collection).length += 1;
    restore_priority_queue_order(collection);
}

unsafe fn restore_priority_queue_order(collection: *mut DrCollectionV1) {
    if (*collection).kind == KIND_PRIORITY_QUEUE && (*collection).finalized != 0 {
        let mut child = (*collection).length - 1;
        while child != 0 {
            let parent = (child - 1) / 2;
            if compare_words(
                read_value(collection, parent),
                read_value(collection, child),
                (*collection).comparator,
            ) != core::cmp::Ordering::Greater
            {
                break;
            }
            swap_values(collection, parent, child);
            child = parent;
        }
    }
}

pub unsafe fn insert_at(
    frame: *const DrStackFrameV2,
    collection: *mut DrCollectionV1,
    index: usize,
    value: u64,
) {
    if index > (*collection).length {
        collection_bounds_panic(frame, index, (*collection).length);
    }
    if (*collection).length == (*collection).capacity {
        grow(collection);
    }
    let tail = (*collection).length - index;
    if tail != 0 {
        ptr::copy(
            value_address(collection, index),
            value_address(collection, index + 1),
            tail * usize::from((*collection).value_width),
        );
    }
    write_value(collection, index, value);
    (*collection).length += 1;
}

pub unsafe fn insert_at_nullable(
    frame: *const DrStackFrameV2,
    collection: *mut DrCollectionV1,
    index: usize,
    present: bool,
    value: u64,
) {
    if index > (*collection).length {
        collection_bounds_panic(frame, index, (*collection).length);
    }
    if (*collection).length == (*collection).capacity {
        grow(collection);
    }
    let tail = (*collection).length - index;
    if tail != 0 {
        ptr::copy(
            value_address(collection, index),
            value_address(collection, index + 1),
            tail * usize::from((*collection).value_width),
        );
    }
    write_nullable_value(collection, index, present, value);
    (*collection).length += 1;
}

pub unsafe fn remove_at(
    frame: *const DrStackFrameV2,
    collection: *mut DrCollectionV1,
    index: usize,
) -> u64 {
    if index >= (*collection).length {
        collection_bounds_panic(frame, index, (*collection).length);
    }
    let removed = read_value(collection, index);
    let tail = (*collection).length - index - 1;
    if tail != 0 {
        ptr::copy(
            value_address(collection, index + 1),
            value_address(collection, index),
            tail * usize::from((*collection).value_width),
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
    if (*collection).kind == KIND_PRIORITY_QUEUE {
        let removed = read_value(collection, 0);
        (*collection).length -= 1;
        if (*collection).length != 0 {
            copy_value_slot(collection, (*collection).length, 0);
            sift_down_min(collection, 0);
        }
        removed
    } else {
        (*collection).length -= 1;
        read_value(collection, (*collection).length)
    }
}

unsafe fn swap_values(collection: *mut DrCollectionV1, left: usize, right: usize) {
    ptr::swap_nonoverlapping(
        value_address(collection, left),
        value_address(collection, right),
        usize::from((*collection).value_width),
    );
}

unsafe fn copy_value_slot(collection: *mut DrCollectionV1, source: usize, target: usize) {
    ptr::copy_nonoverlapping(
        value_address(collection, source),
        value_address(collection, target),
        usize::from((*collection).value_width),
    );
}

unsafe fn swap_entries(collection: *mut DrCollectionV1, left: usize, right: usize, keyed: bool) {
    swap_values(collection, left, right);
    if keyed {
        ptr::swap((*collection).keys.add(left), (*collection).keys.add(right));
    }
}

unsafe fn sift_down_min(collection: *mut DrCollectionV1, mut parent: usize) {
    loop {
        let left = parent * 2 + 1;
        if left >= (*collection).length {
            return;
        }
        let right = left + 1;
        let child = if right < (*collection).length
            && compare_words(
                read_value(collection, right),
                read_value(collection, left),
                (*collection).comparator,
            ) == core::cmp::Ordering::Less
        {
            right
        } else {
            left
        };
        if compare_words(
            read_value(collection, parent),
            read_value(collection, child),
            (*collection).comparator,
        ) != core::cmp::Ordering::Greater
        {
            return;
        }
        swap_values(collection, parent, child);
        parent = child;
    }
}

unsafe fn sift_down_max(
    collection: *mut DrCollectionV1,
    mut parent: usize,
    length: usize,
    keyed: bool,
) {
    loop {
        let left = parent * 2 + 1;
        if left >= length {
            return;
        }
        let right = left + 1;
        let ordered = |index| {
            if keyed {
                *(*collection).keys.add(index)
            } else {
                read_value(collection, index)
            }
        };
        let child = if right < length
            && compare_words(ordered(right), ordered(left), (*collection).comparator)
                == core::cmp::Ordering::Greater
        {
            right
        } else {
            left
        };
        if compare_words(ordered(parent), ordered(child), (*collection).comparator)
            != core::cmp::Ordering::Less
        {
            return;
        }
        swap_entries(collection, parent, child, keyed);
        parent = child;
    }
}

unsafe fn sort_ordered(collection: *mut DrCollectionV1, keyed: bool) {
    let length = (*collection).length;
    if length < 2 {
        return;
    }
    for root in (0..=(length / 2)).rev() {
        sift_down_max(collection, root, length, keyed);
    }
    for end in (1..length).rev() {
        swap_entries(collection, 0, end, keyed);
        sift_down_max(collection, 0, end, keyed);
    }
}

unsafe fn deduplicate_sorted_set(collection: *mut DrCollectionV1) {
    if (*collection).length < 2 {
        return;
    }
    let mut write = 1usize;
    for read in 1..(*collection).length {
        let value = read_value(collection, read);
        if compare_words(
            read_value(collection, write - 1),
            value,
            (*collection).comparator,
        ) == core::cmp::Ordering::Equal
        {
            if (*collection).comparator == COMPARE_STRING {
                crate::dr_v1_string_release(value as *mut DrStringV1);
            }
            continue;
        }
        if write != read {
            copy_value_slot(collection, read, write);
        }
        write += 1;
    }
    (*collection).length = write;
}

pub unsafe fn finalize_stage26(collection: *mut DrCollectionV1) {
    match (*collection).kind {
        KIND_SORTED_DICTIONARY => sort_ordered(collection, true),
        KIND_SORTED_SET => {
            sort_ordered(collection, false);
            deduplicate_sorted_set(collection);
        }
        KIND_PRIORITY_QUEUE => {
            if (*collection).length > 1 {
                for root in (0..=((*collection).length / 2)).rev() {
                    sift_down_min(collection, root);
                }
            }
        }
        KIND_DEQUE => {}
        _ => collection_panic(b"P1001"),
    }
    (*collection).finalized = 1;
}

pub unsafe fn from_copy(
    source: *const DrCollectionV1,
    kind: u8,
    comparator: u8,
    keyed: bool,
    value_width: u8,
    key_kind: u8,
    value_kind: u8,
) -> *mut DrCollectionV1 {
    let result = new_stage26((*source).length, keyed, value_width, kind, comparator);
    (*result).value_nullable = (*source).value_nullable;
    for index in 0..(*source).length {
        let mut value = read_value(source, index);
        if value_kind == COMPARE_STRING && read_present(source, index) {
            value = crate::dr_v1_string_retain(value as *mut DrStringV1) as u64;
        }
        if keyed && kind == KIND_SORTED_DICTIONARY {
            let mut key = *(*source).keys.add(index);
            if key_kind == COMPARE_STRING {
                key = crate::dr_v1_string_retain(key as *mut DrStringV1) as u64;
            }
            // Dictionary sources already have unique keys. Append in O(1) and
            // let finalization perform one O(n log n) sort instead of repeatedly
            // inserting into a growing ordered vector.
            push_stored_value(result, read_present(source, index), value);
            *(*result).keys.add((*result).length - 1) = key;
        } else if kind == KIND_SORTED_SET {
            // Append all values, sort once, then deduplicate in one linear pass.
            push_stored_value(result, read_present(source, index), value);
        } else {
            push_stored_value(result, read_present(source, index), value);
        }
    }
    finalize_stage26(result);
    result
}

unsafe fn push_stored_value(collection: *mut DrCollectionV1, present: bool, value: u64) {
    if (*collection).value_nullable != 0 {
        push_nullable(collection, present, value);
    } else {
        push(collection, value);
    }
}

pub unsafe fn push_front(collection: *mut DrCollectionV1, value: u64) {
    push_front_value(collection, true, value, false);
}

pub unsafe fn push_front_nullable(collection: *mut DrCollectionV1, present: bool, value: u64) {
    push_front_value(collection, present, value, true);
}

unsafe fn push_front_value(
    collection: *mut DrCollectionV1,
    present: bool,
    value: u64,
    nullable: bool,
) {
    if (*collection).kind != KIND_DEQUE {
        collection_panic(b"P1001");
    }
    if (*collection).length == (*collection).capacity {
        grow(collection);
    }
    (*collection).head = if (*collection).head == 0 {
        (*collection).capacity - 1
    } else {
        (*collection).head - 1
    };
    (*collection).length += 1;
    if nullable {
        write_nullable_value(collection, 0, present, value);
    } else {
        write_value(collection, 0, value);
    }
}

pub unsafe fn pop_front(collection: *mut DrCollectionV1, found: *mut u8) -> u64 {
    if (*collection).kind != KIND_DEQUE {
        collection_panic(b"P1001");
    }
    if (*collection).length == 0 {
        *found = 0;
        return 0;
    }
    *found = 1;
    let value = read_value(collection, 0);
    (*collection).head = ((*collection).head + 1) % (*collection).capacity;
    (*collection).length -= 1;
    value
}

pub unsafe fn value_at(
    frame: *const DrStackFrameV2,
    collection: *const DrCollectionV1,
    index: usize,
) -> u64 {
    if index >= (*collection).length {
        collection_bounds_panic(frame, index, (*collection).length);
    }
    read_value(collection, index)
}

pub unsafe fn key_at(
    frame: *const DrStackFrameV2,
    collection: *const DrCollectionV1,
    index: usize,
) -> u64 {
    if (*collection).keyed == 0 {
        collection_panic(b"P1001");
    }
    if index >= (*collection).length {
        collection_bounds_panic(frame, index, (*collection).length);
    }
    *(*collection).keys.add(index)
}

pub unsafe fn set_at(
    frame: *const DrStackFrameV2,
    collection: *mut DrCollectionV1,
    index: usize,
    value: u64,
) -> u64 {
    if index >= (*collection).length {
        collection_bounds_panic(frame, index, (*collection).length);
    }
    let previous = read_value(collection, index);
    write_value(collection, index, value);
    previous
}

pub unsafe fn set_at_nullable(
    frame: *const DrStackFrameV2,
    collection: *mut DrCollectionV1,
    index: usize,
    present: bool,
    value: u64,
    previous_present: *mut u8,
) -> u64 {
    if index >= (*collection).length {
        collection_bounds_panic(frame, index, (*collection).length);
    }
    *previous_present = u8::from(read_present(collection, index));
    let previous = read_value(collection, index);
    write_nullable_value(collection, index, present, value);
    previous
}

unsafe fn keys_equal(left: u64, right: u64, key_kind: u8) -> bool {
    match key_kind {
        COMPARE_STRING => {
            let left = left as *const DrStringV1;
            let right = right as *const DrStringV1;
            crate::string_equal(left, right)
        }
        COMPARE_FLOAT32 => f32::from_bits(left as u32) == f32::from_bits(right as u32),
        COMPARE_FLOAT64 => f64::from_bits(left) == f64::from_bits(right),
        _ => left == right,
    }
}

unsafe fn compare_words(left: u64, right: u64, kind: u8) -> core::cmp::Ordering {
    match kind {
        COMPARE_STRING => {
            crate::dr_v1_string_compare(left as *const DrStringV1, right as *const DrStringV1)
                .cmp(&0)
        }
        COMPARE_SIGNED_8 => (left as i8).cmp(&(right as i8)),
        COMPARE_SIGNED_16 => (left as i16).cmp(&(right as i16)),
        COMPARE_SIGNED_32 => (left as i32).cmp(&(right as i32)),
        COMPARE_SIGNED_64 => (left as i64).cmp(&(right as i64)),
        COMPARE_UNSIGNED_8 => (left as u8).cmp(&(right as u8)),
        COMPARE_UNSIGNED_16 => (left as u16).cmp(&(right as u16)),
        COMPARE_UNSIGNED_32 => (left as u32).cmp(&(right as u32)),
        COMPARE_UNSIGNED_64 => left.cmp(&right),
        COMPARE_BOOL => (left != 0).cmp(&(right != 0)),
        _ => {
            collection_panic(b"P1001");
        }
    }
}

unsafe fn ordered_position(
    collection: *const DrCollectionV1,
    value: u64,
    keyed: bool,
) -> Result<usize, usize> {
    let mut low = 0;
    let mut high = (*collection).length;
    while low < high {
        let middle = low + (high - low) / 2;
        let current = if keyed {
            *(*collection).keys.add(middle)
        } else {
            read_value(collection, middle)
        };
        match compare_words(current, value, (*collection).comparator) {
            core::cmp::Ordering::Less => low = middle + 1,
            core::cmp::Ordering::Greater => high = middle,
            core::cmp::Ordering::Equal => return Ok(middle),
        }
    }
    Err(low)
}

unsafe fn find(collection: *const DrCollectionV1, key: u64, key_kind: u8) -> Option<usize> {
    if (*collection).kind == KIND_SORTED_DICTIONARY && (*collection).finalized != 0 {
        return ordered_position(collection, key, true).ok();
    }
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
        read_value(collection, index)
    } else {
        *found = 0;
        0
    }
}

pub unsafe fn keyed_get_nullable(
    collection: *const DrCollectionV1,
    key: u64,
    key_kind: u8,
    found: *mut u8,
    present: *mut u8,
) -> u64 {
    if let Some(index) = find(collection, key, key_kind) {
        *found = 1;
        *present = u8::from(read_present(collection, index));
        read_value(collection, index)
    } else {
        *found = 0;
        *present = 0;
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
        let previous = read_value(collection, index);
        write_value(collection, index, value);
        return previous;
    }
    *replaced = 0;
    insert_keyed_value(collection, key, true, value, false)
}

pub unsafe fn keyed_set_nullable(
    collection: *mut DrCollectionV1,
    key: u64,
    value: u64,
    present: bool,
    key_kind: u8,
    replaced: *mut u8,
    previous_present: *mut u8,
) -> u64 {
    if let Some(index) = find(collection, key, key_kind) {
        *replaced = 1;
        *previous_present = u8::from(read_present(collection, index));
        let previous = read_value(collection, index);
        write_nullable_value(collection, index, present, value);
        return previous;
    }
    *replaced = 0;
    *previous_present = 0;
    insert_keyed_value(collection, key, present, value, true)
}

unsafe fn insert_keyed_value(
    collection: *mut DrCollectionV1,
    key: u64,
    present: bool,
    value: u64,
    nullable: bool,
) -> u64 {
    if (*collection).length == (*collection).capacity {
        grow(collection);
    }
    let index = if (*collection).kind == KIND_SORTED_DICTIONARY && (*collection).finalized != 0 {
        ordered_position(collection, key, true).unwrap_or_else(|index| index)
    } else {
        (*collection).length
    };
    if index < (*collection).length {
        ptr::copy(
            (*collection).keys.add(index),
            (*collection).keys.add(index + 1),
            (*collection).length - index,
        );
        ptr::copy(
            value_address(collection, index),
            value_address(collection, index + 1),
            ((*collection).length - index) * usize::from((*collection).value_width),
        );
    }
    *(*collection).keys.add(index) = key;
    if nullable {
        write_nullable_value(collection, index, present, value);
    } else {
        write_value(collection, index, value);
    }
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
    let removed_value = read_value(collection, index);
    let tail = (*collection).length - index - 1;
    if tail != 0 {
        ptr::copy(
            (*collection).keys.add(index + 1),
            (*collection).keys.add(index),
            tail,
        );
        ptr::copy(
            value_address(collection, index + 1),
            value_address(collection, index),
            tail * usize::from((*collection).value_width),
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
        0 => {
            let Some(index) = find(collection, key, key_kind) else {
                *found = 0;
                return 0;
            };
            *found = u8::from(read_present(collection, index));
            read_value(collection, index)
        }
        1 => {
            let present = find(collection, key, key_kind)
                .is_some_and(|index| read_present(collection, index));
            let mut existed = 0;
            let value = keyed_remove(collection, key, key_kind, &mut existed, removed_key);
            *found = u8::from(existed != 0 && present);
            value
        }
        2 => {
            if (*collection).length == 0 {
                *found = 0;
                0
            } else {
                *found = u8::from(read_present(collection, 0));
                read_value(collection, 0)
            }
        }
        3 => {
            if (*collection).length == 0 {
                *found = 0;
                0
            } else {
                *found = u8::from(read_present(collection, (*collection).length - 1));
                read_value(collection, (*collection).length - 1)
            }
        }
        4 => nullable_pop(collection, found, false),
        5 => nullable_pop(collection, found, true),
        6 => nullable_pop(collection, found, false),
        7 => {
            let index = key as usize;
            if index >= (*collection).length {
                *found = 0;
                0
            } else {
                *found = u8::from(read_present(collection, index));
                read_value(collection, index)
            }
        }
        _ => collection_panic(b"P1001"),
    }
}

unsafe fn nullable_pop(collection: *mut DrCollectionV1, found: *mut u8, front: bool) -> u64 {
    if (*collection).length == 0 {
        *found = 0;
        return 0;
    }
    let index = if front { 0 } else { (*collection).length - 1 };
    let present = read_present(collection, index);
    let mut existed = 0;
    let value = if front {
        pop_front(collection, &mut existed)
    } else {
        pop(collection, &mut existed)
    };
    *found = u8::from(existed != 0 && present);
    value
}

pub unsafe fn contains(collection: *const DrCollectionV1, value: u64, value_kind: u8) -> bool {
    if (*collection).kind == KIND_SORTED_SET && (*collection).finalized != 0 {
        return ordered_position(collection, value, false).is_ok();
    }
    (0..(*collection).length)
        .any(|index| keys_equal(read_value(collection, index), value, value_kind))
}

pub unsafe fn push_unique(collection: *mut DrCollectionV1, value: u64, value_kind: u8) -> bool {
    if contains(collection, value, value_kind) {
        false
    } else {
        if (*collection).kind == KIND_SORTED_SET && (*collection).finalized != 0 {
            if (*collection).length == (*collection).capacity {
                grow(collection);
            }
            let index = ordered_position(collection, value, false).unwrap_or_else(|index| index);
            let tail = (*collection).length - index;
            if tail != 0 {
                ptr::copy(
                    value_address(collection, index),
                    value_address(collection, index + 1),
                    tail * usize::from((*collection).value_width),
                );
            }
            write_value(collection, index, value);
            (*collection).length += 1;
        } else {
            push(collection, value);
        }
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
        .find(|index| keys_equal(read_value(collection, *index), value, value_kind))
    else {
        *removed = 0;
        return false;
    };
    *removed = read_value(collection, index);
    let tail = (*collection).length - index - 1;
    if tail != 0 {
        ptr::copy(
            value_address(collection, index + 1),
            value_address(collection, index),
            tail * usize::from((*collection).value_width),
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
    let result = if (*left).kind == KIND_SORTED_SET {
        new_stage26(
            0,
            false,
            (*left).value_width,
            KIND_SORTED_SET,
            (*left).comparator,
        )
    } else {
        new(0, false, false, (*left).value_width)
    };
    for index in 0..(*left).length {
        let value = read_value(left, index);
        let include = match operation {
            0 => true,
            1 => contains(right, value, value_kind),
            2 => !contains(right, value, value_kind),
            _ => collection_panic(b"P1001"),
        };
        if include {
            push_retained(result, value, value_kind);
        }
    }
    if operation == 0 {
        for index in 0..(*right).length {
            let value = read_value(right, index);
            if !contains(result, value, value_kind) {
                push_retained(result, value, value_kind);
            }
        }
    }
    if (*result).kind == KIND_SORTED_SET {
        finalize_stage26(result);
    }
    result
}

unsafe fn push_retained(collection: *mut DrCollectionV1, value: u64, value_kind: u8) {
    if value_kind == COMPARE_STRING {
        crate::dr_v1_string_retain(value as *mut DrStringV1);
    }
    push(collection, value);
}

fn collection_panic(code: &'static [u8]) -> ! {
    collection_panic_with_frame(ptr::null(), code)
}

fn collection_panic_with_frame(frame: *const DrStackFrameV2, code: &'static [u8]) -> ! {
    unsafe { dr_v2_panic_code(frame, code.as_ptr(), code.len(), ptr::null(), 0) }
}

fn collection_bounds_panic(frame: *const DrStackFrameV2, index: usize, length: usize) -> ! {
    unsafe { dr_v2_panic_index_out_of_bounds(frame, b"P1310".as_ptr(), 5, index as i64, length) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Reverse;
    use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};

    struct Generator(u64);

    impl Generator {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0
        }

        fn signed(&mut self) -> i64 {
            (self.next() % 97) as i64 - 48
        }
    }

    #[test]
    fn sorted_dictionary_matches_btree_reference_model() {
        unsafe {
            let collection = new_stage26(0, true, 8, KIND_SORTED_DICTIONARY, COMPARE_SIGNED_64);
            finalize_stage26(collection);
            let mut expected = BTreeMap::new();
            let mut random = Generator(0x5eed_2601);
            for step in 0..512u64 {
                let key = random.signed();
                match random.next() % 3 {
                    0 | 1 => {
                        let value = step + 100;
                        let mut replaced = 0;
                        let previous = keyed_set(
                            collection,
                            key as u64,
                            value,
                            COMPARE_SIGNED_64,
                            &mut replaced,
                        );
                        let reference = expected.insert(key, value);
                        assert_eq!(replaced != 0, reference.is_some());
                        assert_eq!(reference.unwrap_or(0), previous);
                    }
                    _ => {
                        let mut found = 0;
                        let mut removed_key = 0;
                        let removed = keyed_remove(
                            collection,
                            key as u64,
                            COMPARE_SIGNED_64,
                            &mut found,
                            &mut removed_key,
                        );
                        let reference = expected.remove(&key);
                        assert_eq!(found != 0, reference.is_some());
                        assert_eq!(reference.unwrap_or(0), removed);
                    }
                }
                assert_eq!(length(collection), expected.len());
                let actual = (0..length(collection))
                    .map(|index| {
                        (
                            key_at(ptr::null(), collection, index) as i64,
                            value_at(ptr::null(), collection, index),
                        )
                    })
                    .collect::<Vec<_>>();
                assert_eq!(
                    actual,
                    expected.iter().map(|(&k, &v)| (k, v)).collect::<Vec<_>>()
                );
            }
            free(collection);
        }
    }

    #[test]
    fn sorted_set_matches_btree_reference_model() {
        unsafe {
            let collection = new_stage26(0, false, 8, KIND_SORTED_SET, COMPARE_SIGNED_64);
            finalize_stage26(collection);
            let mut expected = BTreeSet::new();
            let mut random = Generator(0x5eed_2602);
            for _ in 0..512 {
                let value = random.signed();
                if random.next() & 1 == 0 {
                    assert_eq!(
                        push_unique(collection, value as u64, COMPARE_SIGNED_64),
                        expected.insert(value)
                    );
                } else {
                    let mut removed = 0;
                    assert_eq!(
                        remove_value(collection, value as u64, COMPARE_SIGNED_64, &mut removed),
                        expected.remove(&value)
                    );
                }
                let actual = (0..length(collection))
                    .map(|index| value_at(ptr::null(), collection, index) as i64)
                    .collect::<Vec<_>>();
                assert_eq!(actual, expected.iter().copied().collect::<Vec<_>>());
            }
            free(collection);
        }
    }

    #[test]
    fn priority_queue_matches_min_heap_reference_model() {
        unsafe {
            let collection = new_stage26(0, false, 8, KIND_PRIORITY_QUEUE, COMPARE_SIGNED_64);
            finalize_stage26(collection);
            let mut expected = BinaryHeap::<Reverse<i64>>::new();
            let mut random = Generator(0x5eed_2603);
            for _ in 0..512 {
                if expected.is_empty() || !random.next().is_multiple_of(3) {
                    let value = random.signed();
                    push(collection, value as u64);
                    expected.push(Reverse(value));
                } else {
                    let mut found = 0;
                    let actual = pop(collection, &mut found) as i64;
                    assert_ne!(found, 0);
                    assert_eq!(actual, expected.pop().unwrap().0);
                }
                assert_eq!(length(collection), expected.len());
                if let Some(expected) = expected.peek() {
                    assert_eq!(value_at(ptr::null(), collection, 0) as i64, expected.0);
                }
            }
            free(collection);
        }
    }

    #[test]
    fn deque_wrap_and_growth_match_vecdeque_reference_model() {
        unsafe {
            let collection = new_stage26(0, false, 8, KIND_DEQUE, COMPARE_UNSIGNED_64);
            finalize_stage26(collection);
            let mut expected = VecDeque::new();
            let mut random = Generator(0x5eed_2604);
            for step in 0..1_024u64 {
                match random.next() % 4 {
                    0 => {
                        push_front(collection, step);
                        expected.push_front(step);
                    }
                    1 => {
                        push(collection, step);
                        expected.push_back(step);
                    }
                    2 if !expected.is_empty() => {
                        let mut found = 0;
                        assert_eq!(
                            pop_front(collection, &mut found),
                            expected.pop_front().unwrap()
                        );
                        assert_ne!(found, 0);
                    }
                    3 if !expected.is_empty() => {
                        let mut found = 0;
                        assert_eq!(pop(collection, &mut found), expected.pop_back().unwrap());
                        assert_ne!(found, 0);
                    }
                    _ => {}
                }
                assert_eq!(length(collection), expected.len());
                let actual = (0..length(collection))
                    .map(|index| value_at(ptr::null(), collection, index))
                    .collect::<Vec<_>>();
                assert_eq!(actual, expected.iter().copied().collect::<Vec<_>>());
            }
            free(collection);
        }
    }

    #[test]
    fn nullable_deque_growth_preserves_presence_and_zero_payloads() {
        unsafe {
            let collection = new_stage26(0, false, 16, KIND_DEQUE, COMPARE_SIGNED_64);
            finalize_stage26(collection);
            push_nullable(collection, false, 0);
            push_nullable(collection, true, 0);
            push_nullable(collection, true, 2);
            push_front_nullable(collection, false, 0);
            push_nullable(collection, true, 3);

            let actual = (0..length(collection))
                .map(|index| {
                    (
                        read_present(collection, index),
                        read_value(collection, index),
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(
                actual,
                vec![(false, 0), (false, 0), (true, 0), (true, 2), (true, 3)]
            );
            free(collection);
        }
    }

    #[test]
    fn nonnullable_scalar_presence_does_not_depend_on_payload_bits() {
        unsafe {
            for (width, comparator) in [(1, COMPARE_BOOL), (8, COMPARE_SIGNED_64)] {
                let collection = new_stage26(0, false, width, KIND_DEQUE, comparator);
                finalize_stage26(collection);
                push(collection, 0);

                let mut found = 0;
                let mut removed_key = 0;
                assert_eq!(
                    nullable_access(collection, 0, 0, 5, &mut found, &mut removed_key),
                    0
                );
                assert_eq!(found, 1);
                free(collection);
            }
        }
    }

    #[test]
    fn nullable_sorted_dictionary_orders_keys_and_tracks_replacement_presence() {
        unsafe {
            let collection = new_stage26(0, true, 16, KIND_SORTED_DICTIONARY, COMPARE_SIGNED_64);
            finalize_stage26(collection);
            for (key, present, value) in [(2, false, 0), (1, true, 7), (3, true, 0)] {
                let mut replaced = 0;
                let mut previous_present = 0;
                keyed_set_nullable(
                    collection,
                    key,
                    value,
                    present,
                    COMPARE_SIGNED_64,
                    &mut replaced,
                    &mut previous_present,
                );
                assert_eq!(replaced, 0);
                assert_eq!(previous_present, 0);
            }
            let keys = (0..length(collection))
                .map(|index| key_at(ptr::null(), collection, index))
                .collect::<Vec<_>>();
            assert_eq!(keys, vec![1, 2, 3]);

            let mut replaced = 0;
            let mut previous_present = 0;
            assert_eq!(
                keyed_set_nullable(
                    collection,
                    2,
                    20,
                    true,
                    COMPARE_SIGNED_64,
                    &mut replaced,
                    &mut previous_present,
                ),
                0
            );
            assert_eq!((replaced, previous_present), (1, 0));
            assert!(read_present(collection, 1));
            assert_eq!(read_value(collection, 1), 20);

            assert_eq!(
                keyed_set_nullable(
                    collection,
                    1,
                    0,
                    false,
                    COMPARE_SIGNED_64,
                    &mut replaced,
                    &mut previous_present,
                ),
                7
            );
            assert_eq!((replaced, previous_present), (1, 1));
            let mut found = 0;
            let mut present = 1;
            assert_eq!(
                keyed_get_nullable(collection, 1, COMPARE_SIGNED_64, &mut found, &mut present),
                0
            );
            assert_eq!((found, present), (1, 0));
            free(collection);
        }
    }

    #[test]
    fn nullable_sorted_dictionary_bulk_sort_preserves_presence_tags() {
        unsafe {
            let source = new(0, true, false, 16);
            for (key, present, value) in [(2, false, 0), (1, true, 7), (3, true, 0)] {
                let mut replaced = 0;
                let mut previous_present = 0;
                keyed_set_nullable(
                    source,
                    key,
                    value,
                    present,
                    COMPARE_SIGNED_64,
                    &mut replaced,
                    &mut previous_present,
                );
            }

            let sorted = from_copy(
                source,
                KIND_SORTED_DICTIONARY,
                COMPARE_SIGNED_64,
                true,
                16,
                COMPARE_SIGNED_64,
                COMPARE_SIGNED_64,
            );
            let actual = (0..length(sorted))
                .map(|index| {
                    (
                        key_at(ptr::null(), sorted, index),
                        read_present(sorted, index),
                        read_value(sorted, index),
                    )
                })
                .collect::<Vec<_>>();
            assert_eq!(actual, vec![(1, true, 7), (2, false, 0), (3, true, 0)]);

            free(sorted);
            free(source);
        }
    }
}
