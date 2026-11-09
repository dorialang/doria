use super::*;

unsafe fn checked_get(
    set: *mut c_void,
    key: *const c_void,
    value: *mut DrInterfaceCarrierV1,
    fields: &mut CheckedFields,
) -> u8 {
    let mut home = ptr::NonNull::<DrInterfaceCarrierV1>::dangling().as_ptr();
    let tag = checked_get_home(set, key, value, &mut home, fields);
    assert_eq!(home.is_null(), tag != 0);
    tag
}

unsafe fn checked_get_home(
    set: *mut c_void,
    key: *const c_void,
    value: *mut DrInterfaceCarrierV1,
    home: *mut *mut DrInterfaceCarrierV1,
    fields: &mut CheckedFields,
) -> u8 {
    dr_v1_checked_readiness_set_get_owned(
        ptr::null(),
        set,
        key,
        value,
        home,
        &mut fields.message,
        &mut fields.path,
        &mut fields.system_code,
        &mut fields.transferred,
        &mut fields.meta,
    )
}

unsafe fn assert_rejected(set: *mut c_void, key: *const c_void) {
    let mut value = crate::empty_error_carrier();
    let mut status = DrIoStatusV1::default();
    let mut home = ptr::NonNull::<DrInterfaceCarrierV1>::dangling().as_ptr();
    let (tag, allocations) = crate::probe_runtime_allocations(|| {
        dr_v1_readiness_set_get_owned(set, key, &mut value, &mut home, &mut status)
    });
    assert_eq!((tag, allocations), (3, 0));
    assert_eq!(status.failure(), Some(Failure::invalid_input()));
    assert!(value.object.is_null() && value.vtable.is_null());
    assert!(home.is_null());
    let mut fields = CheckedFields::empty();
    let (tag, allocations) =
        crate::probe_runtime_allocations(|| checked_get(set, key, &mut value, &mut fields));
    assert_eq!((tag, allocations), (1, 1));
    fields.assert_failure(
        doria_diagnostic_catalogue::IoMessageOperation::Access,
        Reason::InvalidInput,
    );
    assert!(value.object.is_null() && value.vtable.is_null());
}

#[test]
fn checked_borrow_preserves_complete_carriers_without_allocation_io_or_ownership_transfer() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let mut other_table = descriptor();
        other_table.type_name = b"Fixture\\OtherStream".as_ptr();
        other_table.type_name_length = b"Fixture\\OtherStream".len();
        other_table.interface_id = 29;
        let set = dr_v1_readiness_set_new(ptr::null(), &mut DrIoStatusV1::default());
        assert!(!set.is_null());
        let mut members = Vec::new();
        for (id, reading, writing) in [
            (1, false, false),
            (2, true, false),
            (3, false, true),
            (4, true, true),
        ] {
            let mut value = fixture.stream(id, if id % 2 == 0 { &table } else { &other_table });
            let original = (value.object, value.vtable);
            let (tag, key) = readiness_register(set, &mut value, reading, writing);
            assert_eq!(tag, 0);
            members.push((key, original, id));
        }
        let header = set.cast::<AdapterSetOwner>();
        let registrations = (*header).next_registration;
        let initialized = {
            let AdapterSetMode::Pending(native) = AdapterSetOwner::native(header) else {
                unreachable!()
            };
            native.initialized()
        };
        assert!(initialized);
        for (key, original, id) in &members {
            let index = (*header).retained_index(key.value).unwrap();
            let member = (*header).retained.raw(index).unwrap();
            let original_home =
                OwnedInterfaceValue::carrier_home(ptr::addr_of_mut!((*member).value));
            for _ in 0..3 {
                let mut loan = crate::empty_error_carrier();
                let mut home = ptr::null_mut();
                let mut fields = CheckedFields::empty();
                let (tag, allocations) = crate::probe_runtime_allocations(|| {
                    checked_get_home(
                        set,
                        ptr::from_ref(key).cast(),
                        &mut loan,
                        &mut home,
                        &mut fields,
                    )
                });
                assert_eq!((tag, allocations), (0, 0));
                fields.assert_success();
                assert_eq!((loan.object, loan.vtable), *original);
                assert_eq!(home, original_home);
                assert_ne!(home, ptr::from_mut(&mut loan));
                assert_ne!(home.cast::<c_void>(), set);
                assert_eq!(((*home).object, (*home).vtable), *original);
                let stream = &mut *(*home).object.cast::<Stream>();
                assert_eq!((stream.calls, stream.dispatch_calls), (0, 0));
                // Mutating a loan's payload reaches the original complete
                // stream, not a copied handle or newly reconstructed adapter.
                stream.written = [*id as u8, 17];
                assert_eq!(
                    fs::read(fixture.directory.join(format!("{id}.bin"))).unwrap(),
                    b"ab"
                );
                // loan carries no close/drop claim and is never taken/dropped.
            }
            assert_eq!((*header).retained.occupied_len(), 4);
            assert_eq!((*header).next_registration, registrations);
            assert!(fixture.drops.borrow().is_empty());
        }
        let AdapterSetMode::Pending(native) = AdapterSetOwner::native(header) else {
            unreachable!()
        };
        assert_eq!(native.initialized(), initialized);
        for (key, original, id) in members {
            let mut returned = crate::empty_error_carrier();
            assert_eq!(readiness_remove(set, &key, &mut returned), 0);
            assert_eq!((returned.object, returned.vtable), original);
            assert_eq!((*returned.object.cast::<Stream>()).written, [id as u8, 17]);
            drop(OwnedInterfaceValue::from_carrier(returned));
        }
        dr_v1_readiness_set_drop(ptr::null(), set);
        assert_eq!(*fixture.drops.borrow(), [1, 2, 3, 4]);
    }
}

#[test]
fn member_home_is_stable_across_successive_loans_and_metadata_growth_or_hole_reuse() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let set = dr_v1_readiness_set_new(ptr::null(), &mut DrIoStatusV1::default());
        let mut first = fixture.stream(1, &table);
        let original = (first.object, first.vtable);
        let (tag, first_key) = readiness_register(set, &mut first, false, false);
        assert_eq!(tag, 0);

        // Each observation ends its loan before membership changes. Retain
        // only an integer identity for comparisons, not usable borrowed access.
        let observe_home = || {
            let mut value = crate::empty_error_carrier();
            let mut home = ptr::null_mut();
            let mut fields = CheckedFields::empty();
            let (tag, allocations) = crate::probe_runtime_allocations(|| {
                checked_get_home(
                    set,
                    ptr::from_ref(&first_key).cast(),
                    &mut value,
                    &mut home,
                    &mut fields,
                )
            });
            assert_eq!((tag, allocations), (0, 0));
            fields.assert_success();
            assert_eq!((value.object, value.vtable), original);
            assert_eq!(((*home).object, (*home).vtable), original);
            assert_ne!(home, ptr::from_mut(&mut value));
            let mut raw_value = crate::empty_error_carrier();
            let mut raw_home = ptr::NonNull::<DrInterfaceCarrierV1>::dangling().as_ptr();
            let mut status = DrIoStatusV1::default();
            status.fail(Failure::from_system_code(13).with_transferred_byte_count(Some(7)));
            let (tag, allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_readiness_set_get_owned(
                    set,
                    ptr::from_ref(&first_key).cast(),
                    &mut raw_value,
                    &mut raw_home,
                    &mut status,
                )
            });
            assert_eq!((tag, allocations), (0, 0));
            assert_eq!(raw_home, home);
            assert_eq!((raw_value.object, raw_value.vtable), original);
            assert_eq!(
                (
                    status.value,
                    status.reason,
                    status.system_code,
                    status.has_system_code,
                    status.transferred_byte_count,
                    status.has_transferred_byte_count
                ),
                (0, 0, 0, 0, 0, 0)
            );
            home as usize
        };
        let address = observe_home();
        let mut keys = Vec::new();
        for id in 2..=18 {
            let mut value = fixture.stream(id, &table);
            let (tag, key) = readiness_register(set, &mut value, false, false);
            assert_eq!(tag, 0);
            keys.push((id, key));
            assert_eq!(observe_home(), address);
        }
        for id in [3, 7, 15] {
            let key = keys.iter().find(|(member, _)| *member == id).unwrap().1;
            let mut returned = crate::empty_error_carrier();
            assert_eq!(readiness_remove(set, &key, &mut returned), 0);
            drop(OwnedInterfaceValue::from_carrier(returned));
            assert_eq!(observe_home(), address);
        }
        // Reuse a freed logical slot; the original member's pinned allocation
        // remains its home, independently of sparse table indices.
        let mut newer = fixture.stream(19, &table);
        let (tag, _) = readiness_register(set, &mut newer, false, false);
        assert_eq!(tag, 0);
        assert_eq!(observe_home(), address);
        assert_eq!((*original.0.cast::<Stream>()).calls, 0);
        dr_v1_readiness_set_drop(ptr::null(), set);
        let mut expected = vec![3, 7, 15, 19];
        expected.extend((1..=18).rev().filter(|id| ![3, 7, 15].contains(id)));
        assert_eq!(*fixture.drops.borrow(), expected);
    }
}

#[test]
fn stale_foreign_null_and_wrong_mode_lookup_preserve_membership_and_other_outputs() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let set = dr_v1_readiness_set_new(ptr::null(), &mut DrIoStatusV1::default());
        let other = dr_v1_readiness_set_new(ptr::null(), &mut DrIoStatusV1::default());
        let mut value = fixture.stream(1, &table);
        let original = (value.object, value.vtable);
        let (tag, stale) = readiness_register(set, &mut value, false, false);
        assert_eq!(tag, 0);
        assert_eq!(readiness_remove(set, &stale, &mut value), 0);
        let (tag, key) = readiness_register(set, &mut value, false, false);
        assert_eq!(tag, 0);
        assert!(key.value > stale.value);
        let mut foreign_value = fixture.stream(2, &table);
        let (tag, foreign) = readiness_register(other, &mut foreign_value, false, false);
        assert_eq!(tag, 0);
        let initialized = {
            let AdapterSetMode::Pending(native) = AdapterSetOwner::native(set.cast()) else {
                unreachable!()
            };
            native.initialized()
        };
        assert!(initialized);
        for rejected in [
            stale,
            foreign,
            PlanKey {
                owner: key.owner,
                value: 0,
            },
            PlanKey {
                owner: key.owner,
                value: u64::MAX,
            },
        ] {
            assert_rejected(set, ptr::from_ref(&rejected).cast());
        }
        assert_rejected(set, ptr::null());
        assert_rejected(ptr::null_mut(), ptr::from_ref(&key).cast());
        for pending in [false, true] {
            let wrong_mode = new_set(pending);
            assert_rejected(wrong_mode, ptr::from_ref(&key).cast());
            dr_v1_transfer_adapter_set_drop(ptr::null(), wrong_mode);
        }
        let mut status = DrIoStatusV1::default();
        let mut home = ptr::NonNull::<DrInterfaceCarrierV1>::dangling().as_ptr();
        assert_eq!(
            dr_v1_readiness_set_get_owned(
                set,
                ptr::from_ref(&key).cast(),
                ptr::null_mut(),
                &mut home,
                &mut status
            ),
            3
        );
        assert_eq!(status.failure(), Some(Failure::invalid_input()));
        assert!(home.is_null());
        assert_eq!(
            dr_v1_readiness_set_get_owned(
                set,
                ptr::from_ref(&key).cast(),
                &mut value,
                ptr::null_mut(),
                &mut status
            ),
            3
        );
        assert_eq!(status.failure(), Some(Failure::invalid_input()));
        assert!(value.object.is_null() && value.vtable.is_null());
        let mut fields = CheckedFields::empty();
        assert_eq!(
            checked_get_home(
                set,
                ptr::from_ref(&key).cast(),
                &mut value,
                ptr::null_mut(),
                &mut fields
            ),
            1
        );
        fields.assert_failure(
            doria_diagnostic_catalogue::IoMessageOperation::Access,
            Reason::InvalidInput,
        );
        let mut occupied = fixture.stream(9, &table);
        let occupied_original = (occupied.object, occupied.vtable);
        let mut fields = CheckedFields::empty();
        assert_eq!(
            checked_get(set, ptr::from_ref(&key).cast(), &mut occupied, &mut fields),
            1
        );
        fields.assert_failure(
            doria_diagnostic_catalogue::IoMessageOperation::Access,
            Reason::InvalidInput,
        );
        assert_eq!((occupied.object, occupied.vtable), occupied_original);
        {
            let AdapterSetMode::Pending(native) = AdapterSetOwner::native(set.cast()) else {
                unreachable!()
            };
            assert_eq!(native.initialized(), initialized);
        }
        assert!(fixture.drops.borrow().is_empty());
        drop(OwnedInterfaceValue::from_carrier(occupied));
        assert_eq!((*set.cast::<AdapterSetOwner>()).retained.occupied_len(), 1);
        let mut loan = crate::empty_error_carrier();
        let mut fields = CheckedFields::empty();
        assert_eq!(
            checked_get(set, ptr::from_ref(&key).cast(), &mut loan, &mut fields),
            0
        );
        fields.assert_success();
        assert_eq!((loan.object, loan.vtable), original);
        {
            let AdapterSetMode::Pending(native) = AdapterSetOwner::native(set.cast()) else {
                unreachable!()
            };
            assert_eq!(native.initialized(), initialized);
        }
        assert_eq!((*loan.object.cast::<Stream>()).calls, 0);
        loan = crate::empty_error_carrier();
        assert!(loan.object.is_null());
        dr_v1_readiness_set_drop(ptr::null(), set);
        dr_v1_readiness_set_drop(ptr::null(), other);
        assert_eq!(*fixture.drops.borrow(), [9, 1, 2]);
    }
}

#[test]
fn external_lookup_validates_only_and_cannot_project_a_fake_native_owned_value() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let set = dr_v1_readiness_set_new(ptr::null(), &mut DrIoStatusV1::default());
        let other = dr_v1_readiness_set_new(ptr::null(), &mut DrIoStatusV1::default());
        let external = fixture.stream(1, &table);
        let original = (external.object, external.vtable);
        let mut key = PlanKey { owner: 0, value: 0 };
        let mut status = DrIoStatusV1::default();
        assert_eq!(
            dr_v1_readiness_set_add_source(
                set,
                source(&external),
                ptr::from_mut(&mut key).cast(),
                &mut status
            ),
            0
        );
        let mut owned = fixture.stream(2, &table);
        let (tag, owned_key) = readiness_register(set, &mut owned, false, false);
        assert_eq!(tag, 0);
        for _ in 0..3 {
            status.fail(Failure::from_system_code(13).with_transferred_byte_count(Some(7)));
            let (tag, allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_readiness_set_get_source(set, ptr::from_ref(&key).cast(), &mut status)
            });
            assert_eq!((tag, allocations), (0, 0));
            assert_eq!(
                (
                    status.value,
                    status.system_code,
                    status.transferred_byte_count,
                    status.reason,
                    status.has_system_code,
                    status.has_transferred_byte_count
                ),
                (0, 0, 0, 0, 0, 0)
            );
            assert_eq!((external.object, external.vtable), original);
            assert_eq!((*external.object.cast::<Stream>()).calls, 0);
        }
        assert_rejected(set, ptr::from_ref(&key).cast());
        for (owner, rejected_key) in [(other, &key), (set, &owned_key)] {
            let (tag, allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_readiness_set_get_source(
                    owner,
                    ptr::from_ref(rejected_key).cast(),
                    &mut status,
                )
            });
            assert_eq!((tag, allocations), (3, 0));
            assert_eq!(status.failure(), Some(Failure::invalid_input()));
        }
        assert_eq!(
            dr_v1_readiness_set_remove_source(set, ptr::from_ref(&key).cast(), &mut status),
            0
        );
        assert_eq!(
            dr_v1_readiness_set_get_source(set, ptr::from_ref(&key).cast(), &mut status),
            3
        );
        assert_eq!(status.failure(), Some(Failure::invalid_input()));
        assert!(fixture.drops.borrow().is_empty());
        dr_v1_readiness_set_drop(ptr::null(), set);
        dr_v1_readiness_set_drop(ptr::null(), other);
        assert_eq!(*fixture.drops.borrow(), [2]);
        drop(OwnedInterfaceValue::from_carrier(external));
        assert_eq!(*fixture.drops.borrow(), [2, 1]);
    }
}

#[test]
fn active_group_rejects_borrow_before_key_access_and_leaves_retirement_in_charge() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let set = dr_v1_readiness_set_new(ptr::null(), &mut DrIoStatusV1::default());
        let mut unused = fixture.stream(1, &table);
        let unused_original = (unused.object, unused.vtable);
        let (tag, unused_key) = readiness_register(set, &mut unused, false, false);
        assert_eq!(tag, 0);
        let mut value = fixture.stream(2, &table);
        let object = value.object.cast::<Stream>();
        let (tag, key) = readiness_register(set, &mut value, true, false);
        assert_eq!(tag, 0);
        let external = fixture.stream(3, &table);
        let mut external_key = PlanKey { owner: 0, value: 0 };
        let mut status = DrIoStatusV1::default();
        assert_eq!(
            dr_v1_readiness_set_add_source(
                set,
                source(&external),
                ptr::from_mut(&mut external_key).cast(),
                &mut status
            ),
            0
        );
        let mut data = [0; 2];
        let buffer = DrTransferBufferV1 {
            data: data.as_mut_ptr(),
            length: 2,
            reading: 1,
        };
        let plan = plan::dr_v1_transfer_plan_new(
            ptr::null(),
            ptr::from_ref(&key).cast(),
            &buffer,
            &mut status,
        );
        let budget = immediate_budget();
        let mut error = crate::empty_error_carrier();
        let group = dr_v1_pending_transfer_adapter_plan_begin(
            ptr::null(),
            set,
            plan,
            &budget,
            &mut error,
            &mut status,
        );
        assert!(!group.is_null());
        let calls = ((*object).calls, (*object).dispatch_calls);
        assert_rejected(set, ptr::from_ref(&key).cast());
        assert_rejected(set, ptr::from_ref(&unused_key).cast());
        // This address is aligned but unreadable. The active-group gate must
        // reject it before even reading a key, not just before stream access.
        let unreadable = ptr::NonNull::<PlanKey>::dangling()
            .as_ptr()
            .cast::<c_void>()
            .cast_const();
        assert_rejected(set, unreadable);
        for rejected in [ptr::from_ref(&external_key).cast(), unreadable] {
            let (tag, allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_readiness_set_get_source(set, rejected, &mut status)
            });
            assert_eq!((tag, allocations), (3, 0));
            assert_eq!(status.failure(), Some(Failure::invalid_input()));
        }
        assert_eq!(((*object).calls, (*object).dispatch_calls), calls);
        assert_eq!((*unused_original.0.cast::<Stream>()).calls, 0);
        assert_eq!((*external.object.cast::<Stream>()).calls, 0);
        assert!(fixture.drops.borrow().is_empty());
        assert!((*set.cast::<AdapterSetOwner>()).active);
        dr_v1_transfer_adapter_group_drop(ptr::null(), group);
        let mut fields = CheckedFields::empty();
        let (tag, allocations) = crate::probe_runtime_allocations(|| {
            checked_get(
                set,
                ptr::from_ref(&unused_key).cast(),
                &mut unused,
                &mut fields,
            )
        });
        assert_eq!((tag, allocations), (0, 0));
        fields.assert_success();
        assert_eq!((unused.object, unused.vtable), unused_original);
        unused = crate::empty_error_carrier();
        assert!(unused.object.is_null());
        assert_eq!(
            dr_v1_readiness_set_get_source(set, ptr::from_ref(&external_key).cast(), &mut status),
            0
        );
        dr_v1_readiness_set_drop(ptr::null(), set);
        assert_eq!(*fixture.drops.borrow(), [2, 1]);
        drop(OwnedInterfaceValue::from_carrier(external));
        assert_eq!(*fixture.drops.borrow(), [2, 1, 3]);
    }
}
