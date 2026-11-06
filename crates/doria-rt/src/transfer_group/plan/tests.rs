use super::*;

const OWNER: usize = 17;

#[test]
fn operation_keys_are_allocation_free_directional_and_stable_across_appends() {
    let source = [7, 8];
    let mut destination = [0; 2];
    let registration = PlanKey {
        owner: OWNER,
        value: 10,
    };
    let plan = TransferPlan::new(registration, Buffer::Write(&source))
        .unwrap_or_else(|_| panic!("plan failed"));
    let (first, allocations) = crate::probe_runtime_allocations(|| plan.operation_key(0, false));
    assert_eq!(allocations, 0);
    let first = first.unwrap();
    let plan = plan
        .append(registration, Buffer::Read(&mut destination))
        .unwrap_or_else(|_| panic!("append failed"));
    let plan = plan
        .append(registration, Buffer::Write(&source))
        .unwrap_or_else(|_| panic!("append failed"));
    assert_eq!(first, plan.operation_key(0, false).unwrap());
    let read = plan.operation_key(1, true).unwrap();
    let last = plan.operation_key(2, false).unwrap();
    assert_ne!(first, last);
    assert_eq!(first.owner, read.owner);
    assert_eq!((first.position, read.position, last.position), (0, 1, 2));
    for (position, reading) in [
        (-1, false),
        (3, false),
        (i64::MAX, true),
        (0, true),
        (1, false),
        (2, true),
    ] {
        assert_eq!(
            plan.operation_key(position, reading),
            Err(Failure::invalid_input())
        );
    }
    let independent = TransferPlan::new(registration, Buffer::Write(&source))
        .unwrap_or_else(|_| panic!("plan failed"));
    assert_ne!(first, independent.operation_key(0, false).unwrap());
    let prepared = plan
        .prepare(OWNER, |_| Some(2))
        .unwrap_or_else(|_| panic!("preparation failed"));
    assert_eq!(prepared.selection.as_slice(), &[2, 2, 2]);
    let mut identity = GroupIdentity::new(prepared.identity);
    let ((), allocations) = crate::probe_runtime_allocations(|| {
        assert_eq!(
            identity.operation_position(read, true, |index| prepared.buffers.request_info(index)),
            Ok(1)
        );
        for key in [
            first,
            last,
            OperationKey {
                position: u64::MAX,
                ..read
            },
        ] {
            assert_eq!(
                identity
                    .operation_position(key, true, |index| prepared.buffers.request_info(index)),
                Err(Failure::invalid_input())
            );
        }
        for owner in [0, independent.operation_key(0, false).unwrap().owner] {
            assert_eq!(
                identity.operation_position(OperationKey { owner, ..read }, true, |_| {
                    panic!("foreign keys must be rejected before inspecting requests")
                }),
                Err(Failure::invalid_input())
            );
        }
        assert!(identity.observed(true).is_none() && identity.observed(false).is_none());
        for (position, (key, reading)) in [(first, false), (read, true), (last, false)]
            .into_iter()
            .enumerate()
        {
            identity.observe(position, reading);
            assert_eq!(
                identity
                    .operation_position(key, reading, |index| prepared.buffers.request_info(index)),
                Ok(position)
            );
            assert_eq!(
                identity.operation_position(key, !reading, |index| prepared
                    .buffers
                    .request_info(index)),
                Err(Failure::invalid_input())
            );
            assert_eq!(identity.observed(reading), Some(key));
            assert_eq!(identity.observed(!reading), None);
        }
        assert_eq!(
            identity.operation_position(read, true, |index| prepared.buffers.request_info(index)),
            Ok(1)
        );
        assert_eq!(identity.observed(false), Some(last));
        identity.clear();
        assert!(identity.observed(true).is_none() && identity.observed(false).is_none());
    });
    assert_eq!(allocations, 0, "observed keys are metadata, not new owners");
    drop(prepared);
    assert_eq!(destination, [0; 2]);
    assert_eq!(source, [7, 8]);
    assert_ne!(first.owner, 0);
}

fn bytes<'a>(buffer: &'a Buffer<'_>) -> &'a [u8] {
    match buffer {
        Buffer::Read(bytes) => bytes,
        Buffer::Write(bytes) => bytes,
    }
}

#[test]
fn moving_appending_and_dropping_plan_preserves_caller_storage() {
    let mut destination = [0xa5; 4];
    let source = [1, 2, 3];
    let destination_pointer = destination.as_ptr();
    let source_pointer = source.as_ptr();
    {
        let plan = TransferPlan::new(
            PlanKey {
                owner: OWNER,
                value: 10,
            },
            Buffer::Read(&mut destination),
        )
        .unwrap_or_else(|_| panic!("plan construction failed"));
        assert_eq!(
            bytes(&plan.entries.as_slice()[0].buffer).as_ptr(),
            destination_pointer
        );
        let plan = plan
            .append(
                PlanKey {
                    owner: OWNER,
                    value: 30,
                },
                Buffer::Write(&source),
            )
            .unwrap_or_else(|_| panic!("plan append failed"));
        let moved = plan;
        let entries = moved.entries.as_slice();
        assert_eq!(entries.len(), 2);
        assert_eq!(bytes(&entries[0].buffer).as_ptr(), destination_pointer);
        assert_eq!(bytes(&entries[0].buffer), &[0xa5; 4]);
        assert_eq!(bytes(&entries[1].buffer).as_ptr(), source_pointer);
        assert_eq!(bytes(&entries[1].buffer), &[1, 2, 3]);
    }
    assert_eq!(destination.as_ptr(), destination_pointer);
    assert_eq!(destination, [0xa5; 4]);
    assert_eq!(source.as_ptr(), source_pointer);
    assert_eq!(source, [1, 2, 3]);
    destination[0] = 0x5a;
    assert_eq!(destination[0], 0x5a);
}

#[test]
fn out_of_order_subset_preparation_moves_loans_without_copying_bytes() {
    let mut third = [0x33; 5];
    let first = [0x11; 3];
    let untouched = [0x22; 7];
    let third_pointer = third.as_ptr();
    let first_pointer = first.as_ptr();
    {
        let plan = TransferPlan::new(
            PlanKey {
                owner: OWNER,
                value: 30,
            },
            Buffer::Read(&mut third),
        )
        .unwrap_or_else(|_| panic!("plan construction failed"))
        .append(
            PlanKey {
                owner: OWNER,
                value: 10,
            },
            Buffer::Write(&first),
        )
        .unwrap_or_else(|_| panic!("plan append failed"));
        let prepared = plan
            .prepare(OWNER, |key| match key {
                10 => Some(0),
                20 => Some(1),
                30 => Some(2),
                _ => None,
            })
            .unwrap_or_else(|_| panic!("plan preparation failed"));
        assert_eq!(prepared.selection.as_slice(), &[2, 0]);
        let buffers = prepared.buffers.entries.as_slice();
        assert_eq!(buffers.len(), 2);
        assert!(matches!(&buffers[0].buffer, Buffer::Read(_)));
        assert!(matches!(&buffers[1].buffer, Buffer::Write(_)));
        assert_eq!(bytes(&buffers[0].buffer).as_ptr(), third_pointer);
        assert_eq!(bytes(&buffers[0].buffer), &[0x33; 5]);
        assert_eq!(bytes(&buffers[1].buffer).as_ptr(), first_pointer);
        assert_eq!(bytes(&buffers[1].buffer), &[0x11; 3]);
        assert_eq!((buffers[0].length, buffers[1].length), (5, 3));
        assert!(!buffers[0].active && !buffers[1].active);
    }
    assert_eq!(third.as_ptr(), third_pointer);
    assert_eq!(third, [0x33; 5]);
    assert_eq!(first.as_ptr(), first_pointer);
    assert_eq!(first, [0x11; 3]);
    assert_eq!(untouched, [0x22; 7]);
    third.fill(0x44);
    assert_eq!(third, [0x44; 5]);
}

#[test]
fn invalid_keys_reject_before_append_allocation_and_return_both_loans() {
    let mut zero_owner_bytes = [0x77; 2];
    let (result, allocations) = crate::probe_runtime_allocations(|| {
        TransferPlan::new(
            PlanKey {
                owner: 0,
                value: 10,
            },
            Buffer::Read(&mut zero_owner_bytes),
        )
    });
    assert_eq!(result.err(), Some(Failure::invalid_input()));
    assert_eq!(allocations, 0);
    assert_eq!(zero_owner_bytes, [0x77; 2]);

    for rejected in [
        PlanKey {
            owner: OWNER + 1,
            value: 30,
        },
        PlanKey {
            owner: 0,
            value: 30,
        },
    ] {
        let mut original = [0xa1; 3];
        let mut incoming = [0xb2; 4];
        let original_pointer = original.as_ptr();
        let incoming_pointer = incoming.as_ptr();
        {
            let plan = TransferPlan::new(
                PlanKey {
                    owner: OWNER,
                    value: 10,
                },
                Buffer::Read(&mut original),
            )
            .unwrap_or_else(|_| panic!("plan construction failed"));
            let descriptor_pointer = plan.entries.raw(0);
            let (result, allocations) = crate::probe_runtime_allocations(|| {
                plan.append(rejected, Buffer::Read(&mut incoming))
            });
            let (failure, returned, buffer) = result
                .err()
                .unwrap_or_else(|| panic!("invalid append was accepted"));
            assert_eq!(failure, Failure::invalid_input());
            assert_eq!(allocations, 0);
            assert_eq!(returned.len(), 1);
            assert_eq!(returned.entries.raw(0), descriptor_pointer);
            let entry = &returned.entries.as_slice()[0];
            assert_eq!(
                entry.key,
                PlanKey {
                    owner: OWNER,
                    value: 10
                }
            );
            assert!(matches!(&entry.buffer, Buffer::Read(_)));
            assert_eq!(bytes(&entry.buffer).as_ptr(), original_pointer);
            assert_eq!(bytes(&entry.buffer), &[0xa1; 3]);
            assert!(matches!(&buffer, Buffer::Read(_)));
            assert_eq!(bytes(&buffer).as_ptr(), incoming_pointer);
            assert_eq!(bytes(&buffer), &[0xb2; 4]);
        }
        assert_eq!(original, [0xa1; 3]);
        assert_eq!(incoming, [0xb2; 4]);
        original[0] = 0;
        incoming[0] = 0;
        assert_eq!((original[0], incoming[0]), (0, 0));
    }
}

#[test]
fn repeated_registration_keys_preserve_distinct_plan_operations_and_loans() {
    let mut destination = [0xa5; 3];
    let source = [1, 2];
    let destination_pointer = destination.as_ptr();
    let key = PlanKey {
        owner: OWNER,
        value: 10,
    };
    let plan = TransferPlan::new(key, Buffer::Read(&mut destination))
        .unwrap_or_else(|_| panic!("read operation"))
        .append(key, Buffer::Write(&source))
        .unwrap_or_else(|_| panic!("write operation on the same registration"));
    let prepared = plan
        .prepare(OWNER, |value| (value == 10).then_some(7))
        .unwrap_or_else(|_| panic!("shared registration selection"));
    assert_eq!(prepared.selection.as_slice(), &[7, 7]);
    let entries = prepared.buffers.entries.as_slice();
    assert_eq!(entries.len(), 2);
    assert_eq!(bytes(&entries[0].buffer).as_ptr(), destination_pointer);
    assert_eq!(bytes(&entries[1].buffer).as_ptr(), source.as_ptr());
}

#[test]
fn invalid_preparation_preserves_original_plan_before_runtime_allocation() {
    // Zero owner, foreign owner, stale/missing key, and distinct keys whose
    // lookup aliases one registered slot all fail before preparation storage.
    for case in 0..4 {
        let mut first = [0xc3; 3];
        let mut third = [0xd4; 4];
        let first_pointer = first.as_ptr();
        let third_pointer = third.as_ptr();
        {
            let plan = TransferPlan::new(
                PlanKey {
                    owner: OWNER,
                    value: 10,
                },
                Buffer::Read(&mut first),
            )
            .unwrap_or_else(|_| panic!("plan construction failed"))
            .append(
                PlanKey {
                    owner: OWNER,
                    value: 30,
                },
                Buffer::Read(&mut third),
            )
            .unwrap_or_else(|_| panic!("plan append failed"));
            let descriptor_pointer = plan.entries.raw(0);
            let owner = match case {
                0 => 0,
                1 => OWNER + 1,
                _ => OWNER,
            };
            let (result, allocations) = crate::probe_runtime_allocations(|| {
                plan.prepare(owner, |key| match key {
                    10 => Some(0),
                    30 if case == 2 => None,
                    30 if case == 3 => Some(0),
                    30 => Some(2),
                    _ => None,
                })
            });
            let (failure, returned) = result
                .err()
                .unwrap_or_else(|| panic!("invalid preparation was accepted"));
            assert_eq!(failure, Failure::invalid_input());
            assert_eq!(allocations, 0);
            assert_eq!(returned.len(), 2);
            assert_eq!(returned.entries.raw(0), descriptor_pointer);
            let entries = returned.entries.as_slice();
            assert_eq!(
                entries[0].key,
                PlanKey {
                    owner: OWNER,
                    value: 10
                }
            );
            assert_eq!(
                entries[1].key,
                PlanKey {
                    owner: OWNER,
                    value: 30
                }
            );
            assert_eq!(bytes(&entries[0].buffer).as_ptr(), first_pointer);
            assert_eq!(bytes(&entries[1].buffer).as_ptr(), third_pointer);
            assert_eq!(bytes(&entries[0].buffer), &[0xc3; 3]);
            assert_eq!(bytes(&entries[1].buffer), &[0xd4; 4]);
        }
        assert_eq!(first, [0xc3; 3]);
        assert_eq!(third, [0xd4; 4]);
        first[0] = 0;
        third[0] = 0;
        assert_eq!((first[0], third[0]), (0, 0));
    }
}
