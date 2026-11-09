use super::*;
use crate::transfer_group::transport::plan::*;
use std::{io::Write, os::fd::AsRawFd, os::unix::net::UnixStream};

unsafe fn add(set: *mut c_void, stream: &UnixStream) -> PlanKey {
    let mut key = PlanKey { owner: 0, value: 0 };
    let mut status = DrIoStatusV1::default();
    assert_eq!(
        dr_v1_transfer_set_add(
            set,
            stream.as_raw_fd(),
            (&mut key as *mut PlanKey).cast(),
            &mut status
        ),
        0
    );
    assert_eq!(status.reason, 0);
    key
}

unsafe fn immediate() -> DrWaitBudgetV1 {
    let mut budget = mem::MaybeUninit::uninit();
    let (mut reason, mut code, mut has_code) = (0, 0, 0);
    assert_eq!(
        crate::wait_budget::dr_v1_wait_budget_init(
            0,
            0,
            budget.as_mut_ptr(),
            &mut reason,
            &mut code,
            &mut has_code
        ),
        0
    );
    budget.assume_init()
}

#[test]
fn dynamic_abi_preserves_unaffected_keys_and_external_owners_across_growth_and_reuse() {
    unsafe {
        let (a, mut a_peer) = UnixStream::pair().unwrap();
        let (b, mut b_peer) = UnixStream::pair().unwrap();
        a.set_nonblocking(true).unwrap();
        b.set_nonblocking(true).unwrap();
        let mut status = DrIoStatusV1::default();
        let set = dr_v1_transfer_set_new(core::ptr::null(), ptr::null_mut(), &mut status);
        assert!(!set.is_null());
        let a_key = add(set, &a);
        let b_key = add(set, &b);
        assert_eq!(a_key.owner, b_key.owner);
        let mut duplicate = a_key;
        assert_eq!(
            dr_v1_transfer_set_add(
                set,
                a.as_raw_fd(),
                (&mut duplicate as *mut PlanKey).cast(),
                &mut status
            ),
            3
        );
        assert_eq!(duplicate, PlanKey { owner: 0, value: 0 });
        let mut foreign = a_key;
        foreign.owner += 1;
        assert_eq!(
            dr_v1_transfer_set_remove(set, (&foreign as *const PlanKey).cast(), &mut status),
            3
        );
        assert_eq!(
            dr_v1_transfer_set_remove(set, (&a_key as *const PlanKey).cast(), &mut status),
            0
        );
        assert_eq!(
            dr_v1_transfer_set_remove(set, (&a_key as *const PlanKey).cast(), &mut status),
            3
        );
        let replacement = add(set, &a);
        assert_eq!(replacement.owner, a_key.owner);
        assert_eq!(
            replacement.value,
            b_key.value + 2,
            "failed admission also consumes its value"
        );
        let mut unchanged = PlanKey { owner: 0, value: 0 };
        assert_eq!(
            dr_v1_transfer_key(set, 1, (&mut unchanged as *mut PlanKey).cast(), &mut status),
            0
        );
        assert_eq!(unchanged, b_key);

        let (mut a_bytes, mut b_bytes) = ([0x55; 8], [0x66; 8]);
        let a_buffer = DrTransferBufferV1 {
            data: a_bytes.as_mut_ptr(),
            length: 8,
            reading: 1,
        };
        let b_buffer = DrTransferBufferV1 {
            data: b_bytes.as_mut_ptr(),
            length: 8,
            reading: 1,
        };
        let stale = dr_v1_transfer_plan_new(
            core::ptr::null(),
            (&a_key as *const PlanKey).cast(),
            &a_buffer,
            &mut status,
        );
        let (rejected, allocations) = crate::probe_runtime_allocations(|| {
            dr_v1_transfer_plan_begin(core::ptr::null(), set, stale, ptr::null(), &mut status)
        });
        assert!(rejected.is_null());
        assert_eq!(
            allocations, 0,
            "retired key rejected before group allocation"
        );
        assert_eq!(a_bytes, [0x55; 8]);

        a_peer.write_all(b"left").unwrap();
        b_peer.write_all(b"right").unwrap();
        let plan = dr_v1_transfer_plan_new(
            core::ptr::null(),
            (&b_key as *const PlanKey).cast(),
            &b_buffer,
            &mut status,
        );
        let plan = dr_v1_transfer_plan_append(
            core::ptr::null(),
            plan,
            (&replacement as *const PlanKey).cast(),
            &a_buffer,
            &mut status,
        );
        let budget = immediate();
        let planned_keys = [0, 1].map(|position| {
            (*plan.cast::<TransferPlan<'static>>())
                .operation_key(position, true)
                .unwrap()
        });
        let group = dr_v1_transfer_plan_begin(core::ptr::null(), set, plan, &budget, &mut status);
        assert!(!group.is_null());
        for key in &planned_keys {
            let (mut data, mut length) = (a_bytes.as_ptr(), usize::MAX);
            assert_eq!(
                dr_v1_transfer_group_read_region(
                    group,
                    ptr::from_ref(key).cast(),
                    &mut data,
                    &mut length,
                    &mut status
                ),
                1
            );
            assert!(data.is_null());
            assert_eq!(length, 0);
        }
        let mut seen = [false; 2];
        for _ in 0..2 {
            let (mut key, mut index) = (0, usize::MAX);
            let mut outcome = DrCompletionStatusV1::default();
            assert_eq!(
                dr_v1_transfer_group_next(
                    group,
                    &mut key,
                    &mut index,
                    (&mut outcome as *mut DrCompletionStatusV1).cast()
                ),
                0
            );
            assert_eq!(key, [b_key.value, replacement.value][index]);
            let mut observed = super::super::super::plan::OperationKey {
                owner: 0,
                position: 0,
            };
            let (tag, allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_transfer_group_observed(group, 1, ptr::from_mut(&mut observed).cast())
            });
            assert_eq!((tag, allocations), (0, 0));
            assert_eq!(observed, planned_keys[index]);
            let (mut data, mut length) = (ptr::null(), 0);
            let (tag, allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_transfer_group_read_region(
                    group,
                    ptr::from_ref(&observed).cast(),
                    &mut data,
                    &mut length,
                    &mut status,
                )
            });
            assert_eq!((tag, allocations), (0, 0));
            assert_eq!(
                (data, length),
                [(b_bytes.as_ptr(), 5), (a_bytes.as_ptr(), 4)][index]
            );
            assert_eq!(
                dr_v1_transfer_group_observed(group, 0, ptr::from_mut(&mut observed).cast()),
                1
            );
            assert_eq!(
                observed,
                super::super::super::plan::OperationKey {
                    owner: 0,
                    position: 0
                }
            );
            seen[index] = true;
        }
        assert_eq!(seen, [true; 2]);
        for key in [
            super::super::super::plan::OperationKey {
                owner: 0,
                ..planned_keys[0]
            },
            super::super::super::plan::OperationKey {
                position: 2,
                ..planned_keys[0]
            },
        ] {
            let (mut data, mut length) = (a_bytes.as_ptr(), usize::MAX);
            assert_eq!(
                dr_v1_transfer_group_read_region(
                    group,
                    ptr::from_ref(&key).cast(),
                    &mut data,
                    &mut length,
                    &mut status
                ),
                3
            );
            assert!(data.is_null());
            assert_eq!(length, 0);
            assert_eq!(status.failure(), Some(Failure::invalid_input()));
        }
        for (key, reading, offset, length) in [
            (planned_keys[0], 0, 0, 8),
            (planned_keys[0], 2, 0, 8),
            (planned_keys[0], 1, usize::MAX, 1),
            (planned_keys[0], 1, 7, 2),
            (
                super::super::super::plan::OperationKey {
                    owner: 0,
                    ..planned_keys[0]
                },
                1,
                0,
                8,
            ),
        ] {
            let (tag, allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_transfer_group_submit_key(
                    group,
                    ptr::from_ref(&key).cast(),
                    reading,
                    offset,
                    length,
                    &mut status,
                )
            });
            assert_eq!((tag, allocations), (3, 0));
            assert_eq!(status.failure(), Some(Failure::invalid_input()));
            crate::transfer_group::transport::tests::assert_group_configuration_failure(&status);
            let (mut data, mut length) = (ptr::null(), 0);
            assert_eq!(
                dr_v1_transfer_group_read_region(
                    group,
                    ptr::from_ref(&planned_keys[0]).cast(),
                    &mut data,
                    &mut length,
                    &mut status
                ),
                0
            );
            assert_eq!((data, length), (b_bytes.as_ptr(), 5));
        }
        let (tag, allocations) = crate::probe_runtime_allocations(|| {
            dr_v1_transfer_group_submit_key(
                group,
                ptr::from_ref(&planned_keys[0]).cast(),
                1,
                2,
                3,
                &mut status,
            )
        });
        assert_eq!((tag, allocations), (0, 0));
        let (mut data, mut length) = (b_bytes.as_ptr(), usize::MAX);
        assert_eq!(
            dr_v1_transfer_group_read_region(
                group,
                ptr::from_ref(&planned_keys[0]).cast(),
                &mut data,
                &mut length,
                &mut status
            ),
            1
        );
        assert_eq!((data, length), (ptr::null(), 0));
        b_peer.write_all(b"xy").unwrap();
        let (mut key, mut index) = (0, usize::MAX);
        let mut outcome = DrCompletionStatusV1::default();
        assert_eq!(
            dr_v1_transfer_group_next(
                group,
                &mut key,
                &mut index,
                ptr::from_mut(&mut outcome).cast()
            ),
            0
        );
        assert_eq!((key, index), (b_key.value, 0));
        assert_eq!(
            dr_v1_transfer_group_read_region(
                group,
                ptr::from_ref(&planned_keys[0]).cast(),
                &mut data,
                &mut length,
                &mut status
            ),
            0
        );
        assert_eq!((data, length), (b_bytes[2..].as_ptr(), 2));
        let mut observed = super::super::super::plan::OperationKey {
            owner: 0,
            position: 0,
        };
        assert_eq!(
            dr_v1_transfer_group_observed(group, 1, ptr::from_mut(&mut observed).cast()),
            0
        );
        assert_eq!(observed, planned_keys[0]);
        dr_v1_transfer_group_drop(group);
        assert_eq!(&a_bytes[..4], b"left");
        assert_eq!(&b_bytes[..5], b"rixyt");
        let ((), closes) = crate::native_handle::with_close_probe(|| dr_v1_transfer_set_drop(set));
        assert_eq!(closes, 1, "only the native watcher closes");
        a_peer.write_all(b"a").unwrap();
        b_peer.write_all(b"b").unwrap();
    }
}

#[test]
fn dynamic_abi_active_and_legacy_mode_guards_precede_input_access_and_consumption() {
    unsafe {
        let (stream, mut peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        let mut status = DrIoStatusV1::default();
        let set = dr_v1_transfer_set_new(core::ptr::null(), ptr::null_mut(), &mut status);
        let key = add(set, &stream);
        assert!(
            take_set(set).is_err(),
            "owning release cannot consume a borrowed set"
        );
        assert!(dr_v1_transfer_group_begin(
            core::ptr::null(),
            set,
            ptr::null(),
            1,
            ptr::null(),
            &mut status
        )
        .is_null());
        let mut bytes = [0x55; 8];
        let buffer = DrTransferBufferV1 {
            data: bytes.as_mut_ptr(),
            length: 8,
            reading: 1,
        };
        let plan = dr_v1_transfer_plan_new(
            core::ptr::null(),
            (&key as *const PlanKey).cast(),
            &buffer,
            &mut status,
        );
        let budget = immediate();
        let group = dr_v1_transfer_plan_begin(core::ptr::null(), set, plan, &budget, &mut status);
        assert!(!group.is_null());
        let mut rejected_key = key;
        let (tag, allocations) = crate::probe_runtime_allocations(|| {
            dr_v1_transfer_set_add(
                set,
                -1,
                (&mut rejected_key as *mut PlanKey).cast(),
                &mut status,
            )
        });
        assert_eq!((tag, allocations), (3, 0));
        assert_eq!(rejected_key, PlanKey { owner: 0, value: 0 });
        assert_eq!(dr_v1_transfer_set_remove(set, ptr::null(), &mut status), 3);
        assert_eq!(
            dr_v1_transfer_set_remove(set, (&key as *const PlanKey).cast(), &mut status),
            3
        );
        assert!(take_set(set).is_err());
        dr_v1_transfer_group_drop(group);
        assert_eq!(bytes, [0x55; 8]);
        assert_eq!(
            dr_v1_transfer_set_remove(set, (&key as *const PlanKey).cast(), &mut status),
            0
        );
        assert_eq!(
            dr_v1_transfer_set_add(set, stream.as_raw_fd(), ptr::null_mut(), &mut status),
            3
        );
        let replacement = add(set, &stream);
        assert_eq!(
            replacement.value,
            key.value + 1,
            "rejected active/malformed calls do not consume a value"
        );
        dr_v1_transfer_set_drop(set);
        peer.write_all(b"still live").unwrap();
    }
}
