//! Storage and index mechanics for compiler-selected core-contract calls.
//! These operations neither call user code nor interpret or destroy values.

use super::*;
use core::sync::atomic::{AtomicU64, Ordering};

pub(super) const COMPARE_CORE: u8 = u8::MAX;

pub(super) fn hash_seed() -> u64 {
    static SEED: AtomicU64 = AtomicU64::new(0);
    let present = SEED.load(Ordering::Acquire);
    if present != 0 {
        return present;
    }
    let seed = process_entropy().max(1);
    match SEED.compare_exchange(0, seed, Ordering::AcqRel, Ordering::Acquire) {
        Ok(_) => seed,
        Err(present) => present,
    }
}

fn process_entropy() -> u64 {
    let mut seed = 0u64;
    #[cfg(target_os = "macos")]
    unsafe {
        unsafe extern "C" {
            fn arc4random_buf(buffer: *mut u8, length: usize);
        }
        arc4random_buf((&mut seed as *mut u64).cast(), mem::size_of::<u64>());
    }
    #[cfg(target_os = "linux")]
    unsafe {
        unsafe extern "C" {
            fn getrandom(buffer: *mut u8, length: usize, flags: u32) -> isize;
            fn __errno_location() -> *mut i32;
        }
        let bytes = (&mut seed as *mut u64).cast::<u8>();
        let mut filled = 0;
        while filled < mem::size_of::<u64>() {
            let read = getrandom(bytes.add(filled), mem::size_of::<u64>() - filled, 0);
            if read > 0 {
                filled += read as usize;
            } else if read != -1 || *__errno_location() != 4 {
                collection_panic(b"P1001");
            }
        }
    }
    #[cfg(windows)]
    unsafe {
        #[link(name = "bcrypt")]
        unsafe extern "system" {
            fn BCryptGenRandom(algorithm: *mut u8, buffer: *mut u8, length: u32, flags: u32)
                -> i32;
        }
        if BCryptGenRandom(ptr::null_mut(), (&mut seed as *mut u64).cast(), 8, 2) < 0 {
            collection_panic(b"P1001");
        }
    }
    seed
}

/// # Safety
/// Layouts and kind come from a validated concrete collection specialization.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dr_v1_core_collection_new(
    frame: *const DrStackFrameV2,
    capacity: usize,
    key_size: usize,
    value_size: usize,
    value_alignment: usize,
    kind: u8,
    hashed: u8,
) -> *mut DrCollectionV1 {
    if !matches!(key_size, 0 | 8 | 16)
        || value_size == 0
        || value_alignment == 0
        || !value_alignment.is_power_of_two()
        || value_alignment > mem::align_of::<u128>()
        || !matches!(
            kind,
            KIND_LEGACY | KIND_SORTED_DICTIONARY | KIND_SORTED_SET | KIND_PRIORITY_QUEUE
        )
        || (hashed != 0 && kind != KIND_LEGACY)
    {
        collection_panic_with_frame(frame, b"P1001");
    }
    let capacity = capacity.max(4);
    let stride = align_to(value_size, value_alignment)
        .unwrap_or_else(|| collection_panic_with_frame(frame, b"P1313"));
    let collection = allocate(DR_COLLECTION_SIZE).cast::<DrCollectionV1>();
    if collection.is_null() {
        collection_panic_with_frame(frame, b"P1206");
    }
    collection.write(DrCollectionV1 {
        length: 0,
        capacity,
        keys: if key_size == 0 {
            ptr::null_mut()
        } else {
            allocate_values_with_frame(frame, capacity, key_size).cast()
        },
        values: allocate_values_with_frame(frame, capacity, stride),
        keyed: u8::from(key_size != 0),
        fixed: 0,
        value_width: if matches!(value_size, 1 | 2 | 4 | 8 | 16) {
            value_size as u8
        } else {
            0
        },
        kind,
        comparator: COMPARE_CORE,
        finalized: 1,
        value_nullable: 0,
        head: 0,
        index: ptr::null_mut(),
        index_slots: 0,
        index_kind: 0,
        index_keyed: 0,
        value_size,
        value_stride: stride,
        value_alignment,
        aggregate: 1,
        key_stride: key_size,
        hashes: if hashed == 0 {
            ptr::null_mut()
        } else {
            allocate_words_with_frame(frame, capacity)
        },
        hashed,
    });
    collection
}

unsafe fn require_core(collection: *const DrCollectionV1) {
    if collection.is_null() || (*collection).comparator != COMPARE_CORE {
        collection_panic(b"P1001");
    }
}

unsafe fn key_address(collection: *const DrCollectionV1, position: usize) -> *mut u8 {
    (*collection)
        .keys
        .cast::<u8>()
        .add(position * (*collection).key_stride)
}

/// # Safety
/// Returns a borrowed address bounded by the caller's collection loan.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dr_v1_core_collection_key_at(
    frame: *const DrStackFrameV2,
    collection: *const DrCollectionV1,
    position: usize,
) -> *mut u8 {
    require_core(collection);
    if (*collection).keyed == 0 || position >= (*collection).length {
        collection_bounds_panic(frame, position, (*collection).length);
    }
    key_address(collection, position)
}

/// # Safety
/// `key` and `value` hold complete, unaliased values of the declared layout.
/// Insertion transfers their ownership. `hash` is the checked raw uint64 hash.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dr_v1_core_collection_insert(
    frame: *const DrStackFrameV2,
    collection: *mut DrCollectionV1,
    position: usize,
    key: *const u8,
    value: *const u8,
    hash: u64,
) {
    require_core(collection);
    if value.is_null()
        || ((*collection).keyed != 0 && key.is_null())
        || position > (*collection).length
    {
        collection_panic_with_frame(frame, b"P1001");
    }
    if (*collection).length == (*collection).capacity {
        grow(collection);
    }
    let tail = (*collection).length - position;
    if tail != 0 {
        index_discard(collection);
        ptr::copy(
            value_address(collection, position),
            value_address(collection, position + 1),
            tail * (*collection).value_stride,
        );
        if (*collection).keyed != 0 {
            ptr::copy(
                key_address(collection, position),
                key_address(collection, position + 1),
                tail * (*collection).key_stride,
            );
        }
        if (*collection).hashed != 0 {
            ptr::copy(
                (*collection).hashes.add(position),
                (*collection).hashes.add(position + 1),
                tail,
            );
        }
    }
    ptr::copy_nonoverlapping(
        value,
        value_address(collection, position),
        (*collection).value_size,
    );
    if (*collection).keyed != 0 {
        ptr::copy_nonoverlapping(
            key,
            key_address(collection, position),
            (*collection).key_stride,
        );
    }
    if (*collection).hashed != 0 {
        *(*collection).hashes.add(position) = hash;
    }
    (*collection).length += 1;
    if (*collection).hashed != 0 {
        index_note_append(collection, position);
    }
}

/// # Safety
/// Output addresses are caller-owned scratch with the exact key/value layouts.
/// Removal transfers both owners to the caller; no destructor runs here.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dr_v1_core_collection_remove(
    frame: *const DrStackFrameV2,
    collection: *mut DrCollectionV1,
    position: usize,
    key: *mut u8,
    value: *mut u8,
) {
    require_core(collection);
    if position >= (*collection).length {
        collection_bounds_panic(frame, position, (*collection).length);
    }
    if value.is_null() || ((*collection).keyed != 0 && key.is_null()) {
        collection_panic_with_frame(frame, b"P1001");
    }
    ptr::copy_nonoverlapping(
        value_address(collection, position),
        value,
        (*collection).value_size,
    );
    if (*collection).keyed != 0 {
        ptr::copy_nonoverlapping(
            key_address(collection, position),
            key,
            (*collection).key_stride,
        );
    }
    index_note_removal(collection, position);
    let tail = (*collection).length - position - 1;
    if tail != 0 {
        ptr::copy(
            value_address(collection, position + 1),
            value_address(collection, position),
            tail * (*collection).value_stride,
        );
        if (*collection).keyed != 0 {
            ptr::copy(
                key_address(collection, position + 1),
                key_address(collection, position),
                tail * (*collection).key_stride,
            );
        }
        if (*collection).hashed != 0 {
            ptr::copy(
                (*collection).hashes.add(position + 1),
                (*collection).hashes.add(position),
                tail,
            );
        }
    }
    (*collection).length -= 1;
}

/// # Safety
/// Both positions belong to the same exclusively borrowed collection.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dr_v1_core_collection_swap(
    frame: *const DrStackFrameV2,
    collection: *mut DrCollectionV1,
    left: usize,
    right: usize,
) {
    require_core(collection);
    if left >= (*collection).length || right >= (*collection).length {
        collection_panic_with_frame(frame, b"P1001");
    }
    if left == right {
        return;
    }
    index_discard(collection);
    ptr::swap_nonoverlapping(
        value_address(collection, left),
        value_address(collection, right),
        (*collection).value_size,
    );
    if (*collection).keyed != 0 {
        ptr::swap_nonoverlapping(
            key_address(collection, left),
            key_address(collection, right),
            (*collection).key_stride,
        );
    }
    if (*collection).hashed != 0 {
        ptr::swap(
            (*collection).hashes.add(left),
            (*collection).hashes.add(right),
        );
    }
}

/// # Safety
/// A readonly collection loan must cover the complete probe sequence. Start
/// with `previous = usize::MAX`; subsequent calls pass the last returned slot.
/// Only hash candidates are returned. MIR must call equals before accepting one.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dr_v1_core_collection_hash_next(
    collection: *mut DrCollectionV1,
    hash: u64,
    previous: usize,
) -> usize {
    require_core(collection);
    if (*collection).hashed == 0 {
        collection_panic(b"P1001");
    }
    if !index_ready(collection, COMPARE_CORE, (*collection).keyed != 0) {
        collection_panic(b"P1206");
    }
    let mask = (*collection).index_slots - 1;
    if previous != usize::MAX && previous > mask {
        collection_panic(b"P1001");
    }
    let mut slot = if previous == usize::MAX {
        hash_word(hash, COMPARE_CORE) & mask
    } else {
        (previous + 1) & mask
    };
    loop {
        if index_slot_position((*collection).index, slot) == INDEX_EMPTY {
            return usize::MAX;
        }
        if index_slot_word((*collection).index, slot) == hash {
            return slot;
        }
        slot = (slot + 1) & mask;
    }
}

/// # Safety
/// `slot` was returned by hash_next under the same uninterrupted readonly loan.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dr_v1_core_collection_hash_position(
    collection: *const DrCollectionV1,
    slot: usize,
) -> usize {
    require_core(collection);
    if slot >= (*collection).index_slots {
        collection_panic(b"P1001");
    }
    let position = index_slot_position((*collection).index, slot);
    if position >= (*collection).length {
        collection_panic(b"P1001");
    }
    position
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe fn candidates(collection: *mut DrCollectionV1, hash: u64) -> Vec<usize> {
        let mut positions = Vec::new();
        let mut slot = usize::MAX;
        loop {
            slot = dr_v1_core_collection_hash_next(collection, hash, slot);
            if slot == usize::MAX {
                break;
            }
            positions.push(dr_v1_core_collection_hash_position(collection, slot));
        }
        positions.sort_unstable();
        positions
    }

    #[test]
    fn complete_keys_hash_collisions_and_removal_survive_growth() {
        unsafe {
            let collection = dr_v1_core_collection_new(ptr::null(), 0, 16, 16, 8, KIND_LEGACY, 1);
            for i in 0..257u64 {
                let key = [i, !i];
                let value = [i * 3, i * 5];
                dr_v1_core_collection_insert(
                    ptr::null(),
                    collection,
                    i as usize,
                    key.as_ptr().cast(),
                    value.as_ptr().cast(),
                    u64::MAX - i % 3,
                );
                assert_eq!(
                    candidates(collection, u64::MAX),
                    (0..=i as usize)
                        .filter(|index| index % 3 == 0)
                        .collect::<Vec<_>>()
                );
            }
            assert!(candidates(collection, 0).is_empty());
            for removed in [0usize, 128, 254] {
                let expected_key = *dr_v1_core_collection_key_at(ptr::null(), collection, removed)
                    .cast::<[u64; 2]>();
                let mut key = [0u64; 2];
                let mut value = [0u64; 2];
                dr_v1_core_collection_remove(
                    ptr::null(),
                    collection,
                    removed,
                    key.as_mut_ptr().cast(),
                    value.as_mut_ptr().cast(),
                );
                assert_eq!(key, expected_key);
                assert_eq!(value, [key[0] * 3, key[0] * 5]);
                assert_eq!(key[1], !key[0]);
                for hash in [u64::MAX, u64::MAX - 1, u64::MAX - 2] {
                    let expected = (0..length(collection))
                        .filter(|index| {
                            let key =
                                *dr_v1_core_collection_key_at(ptr::null(), collection, *index)
                                    .cast::<u64>();
                            u64::MAX - key % 3 == hash
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(candidates(collection, hash), expected);
                }
            }
            free(collection);
        }
    }

    #[test]
    fn core_entry_swaps_and_cleanup_preserve_complete_storage() {
        unsafe {
            let collection =
                dr_v1_core_collection_new(ptr::null(), 0, 16, 24, 8, KIND_SORTED_DICTIONARY, 0);
            for i in 0..3u64 {
                let key = [i, !i];
                let value = [i, i + 10, i + 20];
                dr_v1_core_collection_insert(
                    ptr::null(),
                    collection,
                    0,
                    key.as_ptr().cast(),
                    value.as_ptr().cast(),
                    0,
                );
            }
            dr_v1_core_collection_swap(ptr::null(), collection, 0, 2);
            for i in 0..3usize {
                assert_eq!(
                    *dr_v1_core_collection_key_at(ptr::null(), collection, i).cast::<[u64; 2]>(),
                    [i as u64, !(i as u64)]
                );
                assert_eq!(
                    *value_address(collection, i).cast::<[u64; 3]>(),
                    [i as u64, i as u64 + 10, i as u64 + 20]
                );
            }
            let mut detached = mem::MaybeUninit::uninit();
            detach_for_cleanup(ptr::null(), collection, detached.as_mut_ptr());
            assert_eq!(length(collection), 0);
            finish_detached_cleanup(collection, detached.as_mut_ptr());
            assert_eq!((*collection).key_stride, 16);
            free(collection);
        }
    }

    #[test]
    fn hashed_cleanup_handles_reentrant_refill_and_allocation_reuse() {
        unsafe {
            for refill in [false, true] {
                let collection = dr_v1_core_collection_new(ptr::null(), 0, 0, 8, 8, KIND_LEGACY, 1);
                let value = 42u64;
                dr_v1_core_collection_insert(
                    ptr::null(),
                    collection,
                    0,
                    ptr::null(),
                    (&value as *const u64).cast(),
                    99,
                );
                assert_eq!(candidates(collection, 99), vec![0]);
                let hashes = (*collection).hashes;
                let mut detached = mem::MaybeUninit::uninit();
                detach_for_cleanup(ptr::null(), collection, detached.as_mut_ptr());
                if refill {
                    dr_v1_core_collection_insert(
                        ptr::null(),
                        collection,
                        0,
                        ptr::null(),
                        (&value as *const u64).cast(),
                        100,
                    );
                }
                finish_detached_cleanup(collection, detached.as_mut_ptr());
                assert!(candidates(collection, 99).is_empty());
                if refill {
                    assert_eq!(candidates(collection, 100), vec![0]);
                } else {
                    assert_eq!((*collection).hashes, hashes);
                }
                free(collection);
            }
        }
    }

    #[test]
    fn process_hash_seed_is_stable_across_threads() {
        let seed = hash_seed();
        let threads = (0..8)
            .map(|_| std::thread::spawn(hash_seed))
            .collect::<Vec<_>>();
        for thread in threads {
            assert_eq!(thread.join().unwrap(), seed);
        }
    }
}
