use core::mem;
use core::ptr;

use crate::{allocate, deallocate};

#[repr(C)]
pub struct DrMixedV1 {
    pub tag: u8,
    pub type_id: u32,
    pub payload: u64,
    owner: *mut DrMixedOwnerV1,
}

struct DrMixedOwnerV1 {
    references: usize,
    owns_payload: bool,
    aggregate_payload_offset: usize,
}

fn align_up(value: usize, alignment: usize) -> Option<usize> {
    debug_assert!(alignment.is_power_of_two());
    value
        .checked_add(alignment - 1)
        .map(|value| value & !(alignment - 1))
}

unsafe fn allocate_owner(
    owns_payload: bool,
    aggregate_size: usize,
    aggregate_alignment: usize,
) -> *mut DrMixedOwnerV1 {
    let aggregate_payload_offset = if aggregate_size == 0 {
        0
    } else {
        let alignment = aggregate_alignment.max(mem::align_of::<DrMixedOwnerV1>());
        if !alignment.is_power_of_two() || alignment > mem::align_of::<u128>() {
            return ptr::null_mut();
        }
        let Some(offset) = align_up(mem::size_of::<DrMixedOwnerV1>(), alignment) else {
            return ptr::null_mut();
        };
        offset
    };
    let Some(allocation_size) = aggregate_payload_offset.checked_add(aggregate_size) else {
        return ptr::null_mut();
    };
    let owner =
        allocate(allocation_size.max(mem::size_of::<DrMixedOwnerV1>())).cast::<DrMixedOwnerV1>();
    if owner.is_null() {
        return ptr::null_mut();
    }
    ptr::write(
        owner,
        DrMixedOwnerV1 {
            references: 1,
            owns_payload,
            aggregate_payload_offset,
        },
    );
    owner
}

unsafe fn allocate_box(
    tag: u8,
    type_id: u32,
    payload: u64,
    owner: *mut DrMixedOwnerV1,
) -> *mut DrMixedV1 {
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
            owner,
        },
    );
    value
}

pub unsafe fn new_owned(tag: u8, type_id: u32, payload: u64) -> *mut DrMixedV1 {
    let owner = allocate_owner(true, 0, 1);
    if owner.is_null() {
        return ptr::null_mut();
    }
    let value = allocate_box(tag, type_id, payload, owner);
    if value.is_null() {
        deallocate(owner.cast::<u8>());
    }
    value
}

pub unsafe fn new_borrowed(tag: u8, type_id: u32, payload: u64) -> *mut DrMixedV1 {
    let owner = allocate_owner(false, 0, 1);
    if owner.is_null() {
        return ptr::null_mut();
    }
    let value = allocate_box(tag, type_id, payload, owner);
    if value.is_null() {
        deallocate(owner.cast::<u8>());
    }
    value
}

pub unsafe fn new_owned_aggregate(
    tag: u8,
    type_id: u32,
    source: *const u8,
    byte_length: usize,
    alignment: usize,
) -> *mut DrMixedV1 {
    if source.is_null() || byte_length == 0 {
        return ptr::null_mut();
    }
    let owner = allocate_owner(true, byte_length, alignment);
    if owner.is_null() {
        return ptr::null_mut();
    }
    let payload = owner.cast::<u8>().add((*owner).aggregate_payload_offset);
    ptr::copy_nonoverlapping(source, payload, byte_length);
    let value = allocate_box(tag, type_id, payload as u64, owner);
    if value.is_null() {
        deallocate(owner.cast::<u8>());
    }
    value
}

pub unsafe fn clone_owned(value: *const DrMixedV1) -> *mut DrMixedV1 {
    if value.is_null() {
        return ptr::null_mut();
    }
    (*(*value).owner).references += 1;
    let clone = allocate_box(
        (*value).tag,
        (*value).type_id,
        (*value).payload,
        (*value).owner,
    );
    if clone.is_null() {
        (*(*value).owner).references -= 1;
    }
    clone
}

pub unsafe fn release_owned(value: *mut DrMixedV1) -> bool {
    if value.is_null() || (*value).owner.is_null() {
        return false;
    }
    let owner = (*value).owner;
    (*owner).references -= 1;
    if (*owner).references != 0 {
        (*value).owner = ptr::null_mut();
        return false;
    }
    let owns_payload = (*owner).owns_payload;
    if (*owner).aggregate_payload_offset == 0 {
        deallocate(owner.cast::<u8>());
        (*value).owner = ptr::null_mut();
    }
    owns_payload
}

pub unsafe fn free(value: *mut DrMixedV1) {
    if value.is_null() {
        return;
    }
    if !(*value).owner.is_null() {
        let owner = (*value).owner;
        if (*owner).references == 0 {
            deallocate(owner.cast::<u8>());
        } else {
            (*owner).references -= 1;
            if (*owner).references == 0 {
                deallocate(owner.cast::<u8>());
            }
        }
    }
    deallocate(value.cast::<u8>());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_clones_release_the_payload_only_for_the_final_claim() {
        unsafe {
            let value = new_owned(1, 0, 42);
            let clone = clone_owned(value);
            assert!(!release_owned(value));
            free(value);
            assert!(release_owned(clone));
            free(clone);
        }
    }

    #[test]
    fn borrowed_clones_never_claim_payload_ownership() {
        unsafe {
            let value = new_borrowed(1, 0, 42);
            let clone = clone_owned(value);
            free(value);
            assert!(!release_owned(clone));
            free(clone);
        }
    }

    #[test]
    fn aggregate_payload_uses_aligned_owner_tail_until_the_shell_is_freed() {
        unsafe {
            let source = [1_u64, 2_u64];
            let value = new_owned_aggregate(
                15,
                7,
                source.as_ptr().cast::<u8>(),
                mem::size_of_val(&source),
                mem::align_of_val(&source),
            );
            assert!(!value.is_null());
            let payload = (*value).payload as *const u64;
            assert_eq!(*payload, 1);
            assert_eq!(*payload.add(1), 2);
            assert!(release_owned(value));
            assert_eq!(*payload.add(1), 2);
            free(value);
        }
    }
}
