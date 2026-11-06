use super::*;

#[test]
fn checked_key_lookup_preserves_plan_and_projects_exact_access_failure() {
    unsafe {
        let source = [5; 2];
        let registration = PlanKey { owner: 3, value: 7 };
        let mut outputs = Outputs::default();
        assert_eq!(
            outputs.configure_direction(
                None,
                &registration,
                source.as_ptr().cast_mut(),
                source.len(),
                0
            ),
            0
        );
        let mut copied = OperationKey {
            owner: 0,
            position: 0,
        };
        for (position, reading, present, code) in [
            (0, 0, true, 0),
            (-1, 0, true, 1),
            (1, 0, true, 1),
            (i64::MAX, 0, true, 1),
            (0, 1, true, 1),
            (0, 2, true, 1),
            (0, 0, false, 1),
        ] {
            let mut key = OperationKey {
                owner: 99,
                position: 99,
            };
            assert_eq!(
                dr_v1_checked_transfer_plan_key(
                    ptr::null(),
                    if present { outputs.owner } else { ptr::null() },
                    position,
                    reading,
                    (&mut key as *mut OperationKey).cast(),
                    &mut outputs.message,
                    &mut outputs.path,
                    &mut outputs.system_code,
                    &mut outputs.progress,
                    &mut outputs.meta
                ),
                code
            );
            if code == 0 {
                assert_ne!(key.owner, 0);
                assert_eq!(key.position, 0);
                copied = key;
                outputs.assert_success();
            } else {
                assert_eq!(
                    key,
                    OperationKey {
                        owner: 0,
                        position: 0
                    }
                );
                assert!(!outputs.message.is_null());
                assert!(outputs.path.is_null());
                assert_eq!((outputs.system_code, outputs.progress), (0, 0));
                assert_eq!(
                    outputs.meta,
                    u64::from(crate::checked_io::OP_ACCESS)
                        | (u64::from(crate::checked_io::TARGET_TRANSFER_PLAN) << 8)
                        | (u64::from(Reason::InvalidInput as u8) << 16)
                );
                crate::dr_v1_string_release(outputs.message);
                outputs.message = ptr::null_mut();
            }
        }
        let actual = (*outputs.owner.cast::<TransferPlan<'static>>())
            .operation_key(0, false)
            .unwrap();
        assert_eq!(copied, actual);
        dr_v1_transfer_plan_drop(outputs.owner);
        assert_eq!(source, [5; 2]);
    }
}

#[derive(Default)]
struct Outputs {
    owner: *mut c_void,
    message: *mut DrStringV1,
    path: *mut DrStringV1,
    system_code: i64,
    progress: usize,
    meta: u64,
}

impl Outputs {
    unsafe fn configure(
        &mut self,
        previous: Option<*mut c_void>,
        key: *const PlanKey,
        data: *mut u8,
        length: usize,
    ) -> u8 {
        self.configure_direction(previous, key, data, length, 1)
    }

    unsafe fn configure_direction(
        &mut self,
        previous: Option<*mut c_void>,
        key: *const PlanKey,
        data: *mut u8,
        length: usize,
        reading: u8,
    ) -> u8 {
        match previous {
            Some(previous) => dr_v1_checked_transfer_plan_append(
                ptr::null(),
                previous,
                key.cast(),
                data,
                length,
                reading,
                &mut self.owner,
                &mut self.message,
                &mut self.path,
                &mut self.system_code,
                &mut self.progress,
                &mut self.meta,
            ),
            None => dr_v1_checked_transfer_plan_new(
                ptr::null(),
                key.cast(),
                data,
                length,
                reading,
                &mut self.owner,
                &mut self.message,
                &mut self.path,
                &mut self.system_code,
                &mut self.progress,
                &mut self.meta,
            ),
        }
    }

    fn assert_success(&self) {
        assert!(!self.owner.is_null());
        assert!(self.message.is_null() && self.path.is_null());
        assert_eq!((self.system_code, self.progress, self.meta), (0, 0, 0));
    }
}

#[test]
fn checked_write_setup_shares_overlapping_sources_and_rejects_read_conflicts() {
    let source = [0xa1; 4];
    let key = PlanKey { owner: 3, value: 7 };
    for reading in [0, 1, 2] {
        unsafe {
            let mut outputs = Outputs::default();
            assert_eq!(
                outputs.configure_direction(
                    None,
                    &key,
                    source.as_ptr().cast_mut(),
                    source.len(),
                    0
                ),
                0
            );
            outputs.assert_success();
            let previous = mem::replace(&mut outputs.owner, ptr::null_mut());
            let code = outputs.configure_direction(
                Some(previous),
                &key,
                source.as_ptr().cast_mut(),
                source.len(),
                reading,
            );
            if reading == 0 {
                assert_eq!(code, 0);
                outputs.assert_success();
                let plan = take_plan(outputs.owner).unwrap();
                assert_eq!(plan.len(), 2);
                assert!(!plan.conflicts_with((source.as_ptr() as usize, source.len(), false)));
                assert!(plan.conflicts_with((source.as_ptr() as usize, source.len(), true)));
                drop(plan);
            } else {
                assert_eq!(code, 1);
                assert!(outputs.owner.is_null());
                crate::dr_v1_string_release(outputs.message);
            }
        }
        assert_eq!(source, [0xa1; 4]);
    }
}

#[test]
fn checked_read_setup_forwards_every_region_without_copying_or_transferring_bytes() {
    let mut first = [0xa1; 4];
    let mut second = [0xb2; 6];
    let first_address = first.as_mut_ptr();
    let second_address = second.as_mut_ptr();
    let first_length = first.len();
    let second_length = second.len();
    let keys = [
        PlanKey { owner: 3, value: 7 },
        PlanKey { owner: 3, value: 9 },
    ];
    unsafe {
        let mut outputs = Outputs::default();
        assert_eq!(
            outputs.configure(None, &keys[0], first_address, first_length),
            0
        );
        outputs.assert_success();
        let previous = outputs.owner;
        outputs.owner = ptr::null_mut();
        assert_eq!(
            outputs.configure(Some(previous), &keys[1], second_address, second_length),
            0
        );
        outputs.assert_success();
        let previous = outputs.owner;
        outputs.owner = ptr::null_mut();
        // A repeated key and a null, empty region are valid configuration.
        assert_eq!(
            outputs.configure(Some(previous), &keys[0], ptr::null_mut(), 0),
            0
        );
        outputs.assert_success();
        let plan = take_plan(outputs.owner).unwrap();
        assert_eq!(plan.len(), 3);
        assert!(plan.conflicts_with((first_address as usize, first_length, true)));
        assert!(plan.conflicts_with((second_address as usize, second_length, true)));
        let prepared = match plan.prepare(3, |key| match key {
            7 => Some(0),
            9 => Some(1),
            _ => None,
        }) {
            Ok(prepared) => prepared,
            Err(_) => panic!("checked plan lost valid configuration"),
        };
        assert_eq!(prepared.selection.as_slice(), [0, 1, 0]);
        drop(prepared);
    }
    assert_eq!(first, [0xa1; 4]);
    assert_eq!(second, [0xb2; 6]);
    first[0] = 0;
    second[0] = 0;
    assert_eq!((first[0], second[0]), (0, 0));
}

#[test]
fn checked_read_setup_and_consuming_failure_return_the_existing_io_error_fields() {
    for appending in [false, true] {
        for case in [
            "null-key",
            "foreign-key",
            "null-data",
            "overflow",
            "overlap",
            "null-plan",
        ] {
            let mut first = [0xa1; 4];
            let mut second = [0xb2; 4];
            let first_address = first.as_mut_ptr();
            let first_length = first.len();
            let key = PlanKey { owner: 3, value: 7 };
            let incoming = PlanKey {
                owner: if case == "foreign-key" { 4 } else { 3 },
                value: 9,
            };
            // A foreign first key is valid; only append can compare ownership.
            // Overlap likewise requires an existing retained region.
            if !appending && matches!(case, "foreign-key" | "overlap" | "null-plan") {
                continue;
            }
            unsafe {
                let mut outputs = Outputs::default();
                let previous = if appending {
                    if case == "null-plan" {
                        Some(ptr::null_mut())
                    } else {
                        assert_eq!(
                            outputs.configure(None, &key, first_address, first_length),
                            0
                        );
                        outputs.assert_success();
                        Some(mem::replace(&mut outputs.owner, ptr::null_mut()))
                    }
                } else {
                    None
                };
                let data = match case {
                    "null-data" => ptr::null_mut(),
                    "overflow" => ptr::without_provenance_mut(usize::MAX),
                    "overlap" => first_address.add(1),
                    _ => second.as_mut_ptr(),
                };
                let (tag, allocations) = crate::probe_runtime_allocations(|| {
                    outputs.configure(
                        previous,
                        if case == "null-key" {
                            ptr::null()
                        } else {
                            &incoming
                        },
                        data,
                        2,
                    )
                });
                assert_eq!(tag, 1, "{appending}/{case}");
                // Configuration rejection allocates only the ordinary message.
                assert_eq!(allocations, 1, "{appending}/{case}");
                assert!(outputs.owner.is_null() && outputs.path.is_null());
                assert_eq!((outputs.system_code, outputs.progress), (0, 0));
                assert_eq!(
                    outputs.meta,
                    9 | (5 << 8) | ((Reason::InvalidInput as u64) << 16)
                );
                assert_eq!(
                    slice::from_raw_parts(
                        crate::string_bytes(outputs.message),
                        (*outputs.message).byte_length
                    ),
                    b"failed to configure transfer plan: invalid input",
                );
                crate::dr_v1_string_release(outputs.message);
            }
            assert_eq!(first, [0xa1; 4]);
            assert_eq!(second, [0xb2; 4]);
            first[0] = 0;
            second[0] = 0;
            assert_eq!((first[0], second[0]), (0, 0));
        }
    }
}
