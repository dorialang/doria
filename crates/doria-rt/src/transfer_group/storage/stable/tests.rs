use std::cell::RefCell;
use std::rc::Rc;

use super::*;

struct Tracked {
    id: usize,
    dropped: Rc<RefCell<Vec<usize>>>,
}

impl Tracked {
    fn new(id: usize, dropped: &Rc<RefCell<Vec<usize>>>) -> Self {
        Self {
            id,
            dropped: Rc::clone(dropped),
        }
    }
}

impl Drop for Tracked {
    fn drop(&mut self) {
        self.dropped.borrow_mut().push(self.id);
    }
}

#[test]
fn sparse_storage_keeps_live_addresses_through_growth_and_first_hole_reuse() {
    let mut slots = StableSlots::new().unwrap();
    assert_eq!(slots.insert(10).unwrap(), 0);
    assert_eq!(slots.insert(20).unwrap(), 1);
    let addresses = [slots.raw(0).unwrap(), slots.raw(1).unwrap()];
    slots.reserve(9).unwrap();
    assert_eq!((slots.len(), slots.occupied_len()), (9, 2));
    assert_eq!([slots.raw(0).unwrap(), slots.raw(1).unwrap()], addresses);
    slots.insert_at(8, 80).unwrap();
    let last_address = slots.raw(8).unwrap();
    assert_eq!(slots.remove(0), Some(10));
    assert!(!slots.contains(0));
    assert!(slots.get(0).is_none());
    assert!(slots.get_mut(0).is_none());
    assert!(slots.raw(0).is_none());
    assert_eq!(slots.insert(30).unwrap(), 0);
    assert_eq!(slots.insert(40).unwrap(), 2);
    *slots.get_mut(2).unwrap() = 42;
    assert_eq!(slots.get(2), Some(&42));
    assert_eq!(slots.raw(1), Some(addresses[1]));
    assert_eq!(slots.raw(8), Some(last_address));
    assert_eq!((slots.len(), slots.occupied_len()), (9, 4));
    assert_eq!(slots.remove(99), None);
    assert!(!slots.contains(99));
    assert!(slots.raw(99).is_none());
}

#[test]
fn sparse_storage_removal_moves_one_owner_once_and_drops_live_slots_in_reverse() {
    let dropped = Rc::new(RefCell::new(Vec::new()));
    let mut slots = StableSlots::new().unwrap();
    slots.insert_at(0, Tracked::new(0, &dropped)).unwrap();
    slots.insert_at(4, Tracked::new(4, &dropped)).unwrap();
    slots.insert_at(2, Tracked::new(2, &dropped)).unwrap();
    let moved = slots.remove(2).unwrap();
    assert_eq!(moved.id, 2);
    assert!(dropped.borrow().is_empty());
    assert!(slots.remove(2).is_none());
    assert_eq!((slots.len(), slots.occupied_len()), (5, 2));
    slots.reserve(11).unwrap();
    assert!(dropped.borrow().is_empty());
    drop(moved);
    drop(slots);
    assert_eq!(&*dropped.borrow(), &[2, 4, 0]);
}

#[test]
fn sparse_storage_drops_in_reverse_acquisition_order_after_hole_reuse_and_growth() {
    let dropped = Rc::new(RefCell::new(Vec::new()));
    let mut slots = StableSlots::new().unwrap();
    slots.insert_at(4, Tracked::new(10, &dropped)).unwrap();
    slots.insert_at(0, Tracked::new(20, &dropped)).unwrap();
    slots.insert_at(2, Tracked::new(30, &dropped)).unwrap();
    // Remove the oldest, newest and middle entries without disturbing survivors.
    drop(slots.remove(4).unwrap());
    slots.insert_at(4, Tracked::new(40, &dropped)).unwrap();
    drop(slots.remove(4).unwrap());
    slots.insert_at(1, Tracked::new(50, &dropped)).unwrap();
    drop(slots.remove(2).unwrap());
    slots.insert_at(2, Tracked::new(60, &dropped)).unwrap();
    drop(slots.remove(0).unwrap());
    slots.insert_at(0, Tracked::new(70, &dropped)).unwrap();
    assert_eq!(&*dropped.borrow(), &[10, 40, 30, 20]);
    assert_eq!(slots.occupied_len(), 3);
    let addresses = [slots.raw(0), slots.raw(1), slots.raw(2)];
    slots.reserve(33).unwrap();
    assert_eq!([slots.raw(0), slots.raw(1), slots.raw(2)], addresses);
    drop(slots);
    assert_eq!(&*dropped.borrow(), &[10, 40, 30, 20, 70, 60, 50]);
}

#[test]
fn sparse_storage_drops_out_of_order_slots_in_reverse_acquisition_order() {
    let dropped = Rc::new(RefCell::new(Vec::new()));
    let mut slots = StableSlots::new().unwrap();
    for (index, id) in [(9, 1), (1, 2), (5, 3), (0, 4)] {
        slots.insert_at(index, Tracked::new(id, &dropped)).unwrap();
    }
    drop(slots);
    assert_eq!(&*dropped.borrow(), &[4, 3, 2, 1]);
}

#[test]
fn sparse_storage_rejected_insertion_and_capacity_failure_preserve_membership() {
    let dropped = Rc::new(RefCell::new(Vec::new()));
    let mut slots = StableSlots::new().unwrap();
    slots.insert(Tracked::new(7, &dropped)).unwrap();
    let address = slots.raw(0);
    let (occupied, allocations) =
        crate::probe_runtime_allocations(|| slots.insert_at(0, Tracked::new(8, &dropped)));
    assert!(occupied.is_err());
    assert_eq!(allocations, 0);
    assert_eq!(&*dropped.borrow(), &[8]);
    let (invalid, allocations) =
        crate::probe_runtime_allocations(|| slots.insert_at(usize::MAX, Tracked::new(9, &dropped)));
    assert!(invalid.is_err());
    assert_eq!(allocations, 0);
    // Failure after staging the independent value owner still publishes no
    // membership and never releases or relocates an existing live value.
    assert!(slots
        .insert_at(usize::MAX - 1, Tracked::new(10, &dropped))
        .is_err());
    assert!(slots.reserve(usize::MAX).is_err());
    assert_eq!(slots.raw(0), address);
    assert_eq!(slots.get(0).unwrap().id, 7);
    assert_eq!((slots.len(), slots.occupied_len()), (1, 1));
    assert_eq!(&*dropped.borrow(), &[8, 9, 10]);
    drop(slots);
    assert_eq!(&*dropped.borrow(), &[8, 9, 10, 7]);
}

#[test]
fn sparse_storage_failed_staging_never_takes_the_value() {
    let mut slots = StableSlots::new().unwrap();
    slots.insert_at(0, 17).unwrap();
    let address = slots.raw(0);
    let mut takes = 0;
    for index in [0, usize::MAX, usize::MAX - 1] {
        assert!(slots
            .insert_at_with(index, || {
                takes += 1;
                19
            })
            .is_err());
    }
    assert_eq!(takes, 0);
    assert_eq!((slots.len(), slots.occupied_len()), (1, 1));
    assert_eq!(slots.raw(0), address);
    slots
        .insert_at_with(3, || {
            takes += 1;
            23
        })
        .unwrap();
    assert_eq!(takes, 1);
    assert_eq!(slots.get(3), Some(&23));
}

#[test]
fn sparse_storage_allocates_only_when_configuring_owners_or_growing_the_table() {
    let (mut slots, allocations) =
        crate::probe_runtime_allocations(|| StableSlots::<u64>::new().unwrap());
    assert_eq!(allocations, 0);
    let ((), allocations) = crate::probe_runtime_allocations(|| slots.reserve(8).unwrap());
    assert_eq!(allocations, 1);
    let (index, allocations) = crate::probe_runtime_allocations(|| slots.insert(11).unwrap());
    assert_eq!((index, allocations), (0, 1));
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        slots.reserve(3).unwrap();
        for _ in 0..32 {
            assert!(slots.contains(0));
            assert_eq!(slots.get(0), Some(&11));
            assert_eq!(unsafe { slots.raw(0).unwrap().read() }, 11);
            assert_eq!(slots.occupied_len(), 1);
        }
        assert_eq!(slots.remove(0), Some(11));
        drop(slots);
    });
    assert_eq!(allocations, 0);
}

#[test]
fn sparse_storage_reuses_table_capacity_without_extending_logical_membership() {
    let mut slots = StableSlots::new().unwrap();
    let mut first_address = None;
    for index in 0_usize..64 {
        let (slot, allocations) =
            crate::probe_runtime_allocations(|| slots.insert(index as u64).unwrap());
        assert_eq!(slot, index);
        let table_growth = usize::from(index == 0 || index.is_power_of_two());
        assert_eq!(allocations, 1 + table_growth, "inserting slot {index}");
        assert_eq!((slots.len(), slots.occupied_len()), (index + 1, index + 1));
        if let Some(address) = first_address {
            assert_eq!(slots.raw(0), Some(address));
        } else {
            first_address = slots.raw(0);
        }
    }
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        slots.reserve(65).unwrap();
        assert_eq!(slots.len(), 65);
        slots.reserve(128).unwrap();
    });
    assert_eq!(allocations, 1);
    assert_eq!((slots.len(), slots.occupied_len()), (128, 64));
    assert_eq!(slots.raw(0), first_address);
}

#[test]
fn sparse_storage_preserves_individual_alignment_and_zero_sized_membership() {
    #[repr(align(128))]
    struct Aligned(u8);
    let mut aligned = StableSlots::new().unwrap();
    aligned.insert(Aligned(17)).unwrap();
    let address = aligned.raw(0).unwrap();
    aligned.reserve(17).unwrap();
    assert_eq!(aligned.raw(0), Some(address));
    assert_eq!(address as usize % 128, 0);
    assert_eq!(aligned.remove(0).unwrap().0, 17);

    let mut empty_values = StableSlots::new().unwrap();
    assert_eq!(empty_values.insert(()).unwrap(), 0);
    empty_values.insert_at(4, ()).unwrap();
    assert_eq!((empty_values.len(), empty_values.occupied_len()), (5, 2));
    assert_eq!(empty_values.remove(4), Some(()));
    assert_eq!(empty_values.occupied_len(), 1);
}
