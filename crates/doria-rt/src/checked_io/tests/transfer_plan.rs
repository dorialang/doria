use super::*;

#[test]
fn configuration_projection_preserves_every_reason_and_optional_fact_without_transfer_progress() {
    for (reason, message_reason) in [
        (Reason::NotFound, IoMessageReason::NotFound),
        (Reason::PermissionDenied, IoMessageReason::PermissionDenied),
        (Reason::InvalidInput, IoMessageReason::InvalidInput),
        (Reason::Interrupted, IoMessageReason::Interrupted),
        (
            Reason::ResourceExhausted,
            IoMessageReason::ResourceExhausted,
        ),
        (Reason::Unsupported, IoMessageReason::Unsupported),
        (Reason::Closed, IoMessageReason::Closed),
        (Reason::Other, IoMessageReason::Other),
    ] {
        for code in [None, Some(0), Some(13), Some(-7)] {
            let failure = Failure {
                reason,
                system_code: code,
                transferred_byte_count: None,
            };
            let mut status = DrIoStatusV1::default();
            assert_eq!(status.fail(failure), 3);
            // Optional facts belong to presence bits, not unused payloads.
            status.transferred_byte_count = usize::MAX;
            if code.is_none() {
                status.system_code = -99;
            }
            let mut message = ptr::null_mut();
            let mut path = ptr::null_mut();
            let mut system_code = -1;
            let mut transferred_byte_count = usize::MAX;
            let mut meta = u64::MAX;
            unsafe {
                let (tag, allocations) = crate::probe_runtime_allocations(|| {
                    dr_v1_checked_transfer_plan_failure(
                        ptr::null(),
                        &status,
                        &mut message,
                        &mut path,
                        &mut system_code,
                        &mut transferred_byte_count,
                        &mut meta,
                    )
                });
                assert_eq!(tag, 1);
                // Only the existing message string is allocated. There is no
                // synthetic path or second public Error/transport owner.
                assert_eq!(allocations, 1);
                assert!(!message.is_null());
                assert!(path.is_null());
                assert_eq!(system_code, code.unwrap_or(0));
                assert_eq!(transferred_byte_count, 0);
                assert_eq!(meta & 0xff, 9);
                assert_eq!((meta >> META_TARGET_SHIFT) & 0xff, 5);
                assert_eq!((meta >> META_REASON_SHIFT) & 0xff, reason as u64);
                assert_eq!((meta >> META_KIND_SHIFT) & 0xff, ERROR_IO as u64);
                assert_eq!(
                    (meta >> META_HAS_SYSTEM_CODE_SHIFT) & 1,
                    u64::from(code.is_some())
                );
                assert_eq!((meta >> META_HAS_INVALID_COUNT_SHIFT) & 1, 0);
                assert_eq!((meta >> META_HAS_TRANSFERRED_COUNT_SHIFT) & 1, 0);
                assert_eq!(meta >> 35, 0);
                let mut expected = Vec::new();
                visit_io_error_message_parts(
                    IoMessageOperation::Configure,
                    IoMessageTarget::TransferPlan,
                    message_reason,
                    |part| expected.extend_from_slice(part),
                );
                let actual =
                    core::slice::from_raw_parts(string_bytes(message), (*message).byte_length);
                assert_eq!(actual, expected);
                crate::dr_v1_string_release(message);
            }
            assert_eq!(status.failure(), Some(failure));
        }
    }
}

#[test]
fn configuration_tags_append_without_changing_existing_readiness_or_intrinsic_tags() {
    assert_eq!(OP_CONFIGURE, 9);
    assert_eq!(TARGET_TRANSFER_PLAN, 5);
    assert_eq!(TARGET_TRANSFER_GROUP, 6);
    assert_eq!([OP_REGISTER, OP_UNREGISTER, OP_WAIT], [6, 7, 8]);
    assert_eq!(TARGET_READINESS_SET, 4);
    assert_eq!(
        [
            READ_LINE,
            READ_FILE_TEXT,
            READ_FILE_BYTES,
            READ_STDIN_BYTES,
            WRITE_FILE,
            APPEND_FILE,
            WRITE_STDOUT,
            WRITE_STDERR,
            READINESS_SET_NEW,
        ],
        [0, 1, 2, 3, 4, 5, 6, 7, 8]
    );
}

#[test]
fn invalid_group_access_and_configuration_project_the_group_target_without_path_or_progress() {
    for (operation, expected) in [
        (OP_ACCESS, "failed to access transfer group: invalid input"),
        (
            OP_CONFIGURE,
            "failed to configure transfer group: invalid input",
        ),
    ] {
        let failure = Failure::invalid_input();
        let mut status = DrIoStatusV1::default();
        assert_eq!(status.fail(failure), 3);
        status.system_code = -99;
        status.transferred_byte_count = usize::MAX;
        let mut message = ptr::null_mut();
        let mut path = ptr::null_mut();
        let mut system_code = -1;
        let mut transferred_byte_count = usize::MAX;
        let mut meta = u64::MAX;
        unsafe {
            let (tag, allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_checked_transfer_group_failure(
                    ptr::null(),
                    operation,
                    &status,
                    &mut message,
                    &mut path,
                    &mut system_code,
                    &mut transferred_byte_count,
                    &mut meta,
                )
            });
            assert_eq!((tag, allocations), (1, 1));
            assert!(path.is_null());
            assert_eq!((system_code, transferred_byte_count), (0, 0));
            assert_eq!(
                meta,
                metadata(
                    operation,
                    6,
                    Reason::InvalidInput as u8,
                    ERROR_IO,
                    false,
                    false,
                    false
                )
            );
            let actual = core::slice::from_raw_parts(string_bytes(message), (*message).byte_length);
            assert_eq!(actual, expected.as_bytes());
            crate::dr_v1_string_release(message);
        }
        assert_eq!(status.failure(), Some(failure));
    }
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
fn rejected_plan_setup_projects_its_actual_status_without_starting_io() {
    let mut status = DrIoStatusV1::default();
    let mut message = ptr::null_mut();
    let mut path = ptr::null_mut();
    let mut system_code = -1;
    let mut transferred_byte_count = usize::MAX;
    let mut meta = u64::MAX;
    unsafe {
        let (plan, allocations) = crate::probe_runtime_allocations(|| {
            crate::transfer_group::transport::plan::dr_v1_transfer_plan_new(
                ptr::null(),
                ptr::null(),
                ptr::null(),
                &mut status,
            )
        });
        assert!(plan.is_null());
        assert_eq!(allocations, 0);
        assert_eq!(status.failure(), Some(Failure::invalid_input()));
        assert_eq!(
            dr_v1_checked_transfer_plan_failure(
                ptr::null(),
                &status,
                &mut message,
                &mut path,
                &mut system_code,
                &mut transferred_byte_count,
                &mut meta,
            ),
            1
        );
        assert_eq!(
            meta,
            metadata(
                OP_CONFIGURE,
                TARGET_TRANSFER_PLAN,
                Reason::InvalidInput as u8,
                ERROR_IO,
                false,
                false,
                false
            )
        );
        assert!(path.is_null());
        assert_eq!((system_code, transferred_byte_count), (0, 0));
        let actual = core::slice::from_raw_parts(string_bytes(message), (*message).byte_length);
        assert_eq!(actual, b"failed to configure transfer plan: invalid input");
        crate::dr_v1_string_release(message);
    }
    assert_eq!(status.failure(), Some(Failure::invalid_input()));
}

#[test]
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
fn rejected_consuming_append_preserves_caller_bytes_and_projects_actual_configuration_failure() {
    use crate::transfer_group::transport::{plan, DrTransferBufferV1};
    use crate::transfer_group::PlanKey;

    for case in [
        "foreign",
        "null-key",
        "null-buffer",
        "mode",
        "overflow",
        "overlap",
    ] {
        let mut original = [0xa1; 4];
        let mut incoming = [0xb2; 4];
        let key = PlanKey { owner: 1, value: 7 };
        let rejected_key = PlanKey {
            owner: if case == "foreign" { 2 } else { 1 },
            value: 8,
        };
        let original_buffer = DrTransferBufferV1 {
            data: original.as_mut_ptr(),
            length: original.len(),
            reading: 1,
        };
        let buffer = DrTransferBufferV1 {
            data: match case {
                "overflow" => ptr::without_provenance_mut(usize::MAX),
                "overlap" => unsafe { original_buffer.data.add(1) },
                _ => incoming.as_mut_ptr(),
            },
            length: 2,
            reading: if case == "mode" { 2 } else { 0 },
        };
        let mut status = DrIoStatusV1::default();
        unsafe {
            let owner = plan::dr_v1_transfer_plan_new(
                ptr::null(),
                (&key as *const PlanKey).cast(),
                &original_buffer,
                &mut status,
            );
            assert!(!owner.is_null(), "{case}");
            let (returned, allocations) = crate::probe_runtime_allocations(|| {
                plan::dr_v1_transfer_plan_append(
                    ptr::null(),
                    owner,
                    if case == "null-key" {
                        ptr::null()
                    } else {
                        (&rejected_key as *const PlanKey).cast()
                    },
                    if case == "null-buffer" {
                        ptr::null()
                    } else {
                        &buffer
                    },
                    &mut status,
                )
            });
            assert!(returned.is_null(), "{case}");
            assert_eq!(allocations, 0, "{case}");
            assert_eq!(status.failure(), Some(Failure::invalid_input()), "{case}");
            let (mut message, mut path) = (ptr::null_mut(), ptr::null_mut());
            let (mut system_code, mut transferred, mut meta) = (-1, usize::MAX, u64::MAX);
            assert_eq!(
                dr_v1_checked_transfer_plan_failure(
                    ptr::null(),
                    &status,
                    &mut message,
                    &mut path,
                    &mut system_code,
                    &mut transferred,
                    &mut meta,
                ),
                1
            );
            assert_eq!(
                meta,
                metadata(
                    OP_CONFIGURE,
                    TARGET_TRANSFER_PLAN,
                    Reason::InvalidInput as u8,
                    ERROR_IO,
                    false,
                    false,
                    false,
                )
            );
            assert!(path.is_null());
            assert_eq!((system_code, transferred), (0, 0));
            assert_eq!(
                core::slice::from_raw_parts(string_bytes(message), (*message).byte_length),
                b"failed to configure transfer plan: invalid input",
            );
            crate::dr_v1_string_release(message);
        }
        assert_eq!(original, [0xa1; 4], "{case}");
        assert_eq!(incoming, [0xb2; 4], "{case}");
        original[0] = 0;
        incoming[0] = 0;
        assert_eq!((original[0], incoming[0]), (0, 0));
    }
}

#[test]
fn nontransfer_projection_rejects_supplied_progress_and_malformed_status_in_both_paths() {
    for projection in [
        "readiness",
        "access",
        "plan",
        "group-access",
        "group-configure",
    ] {
        for invalid in [
            "zero-progress",
            "progress",
            "reason",
            "system-flag",
            "progress-flag",
            "operation",
        ] {
            if projection == "plan" && invalid == "operation" {
                continue;
            }
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "checked_io::tests::transfer_plan::nontransfer_projection_invariant_child",
                    "--nocapture",
                ])
                .env(
                    "DORIA_RT_TEST_NONTRANSFER_FAILURE",
                    format!("{projection}:{invalid}"),
                )
                .env_remove("DORIA_RUNTIME_OUTCOME_V2")
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(101),
                "{projection}:{invalid}: {output:?}"
            );
            assert!(String::from_utf8(output.stderr).unwrap().contains("P1001"));
        }
    }
}

#[test]
fn nontransfer_projection_invariant_child() {
    let Ok(case) = std::env::var("DORIA_RT_TEST_NONTRANSFER_FAILURE") else {
        return;
    };
    let (projection, invalid) = case.split_once(':').unwrap();
    let mut status = DrIoStatusV1::default();
    status.fail(Failure::invalid_input());
    match invalid {
        "zero-progress" => {
            status.has_transferred_byte_count = 1;
            status.transferred_byte_count = 0;
        }
        "progress" => {
            status.has_transferred_byte_count = 1;
            status.transferred_byte_count = 3;
        }
        "reason" => status.reason = u8::MAX,
        "system-flag" => status.has_system_code = 2,
        "progress-flag" => status.has_transferred_byte_count = 2,
        "operation" => {}
        _ => unreachable!("unknown invariant case"),
    }
    let mut message = ptr::null_mut();
    let mut path = ptr::null_mut();
    let mut system_code = 0;
    let mut transferred_byte_count = 0;
    let mut meta = 0;
    unsafe {
        match projection {
            "readiness" | "access" => dr_v1_checked_readiness_failure(
                ptr::null(),
                if invalid == "operation" {
                    OP_CONFIGURE
                } else if projection == "access" {
                    OP_ACCESS
                } else {
                    OP_REGISTER
                },
                &status,
                &mut message,
                &mut path,
                &mut system_code,
                &mut transferred_byte_count,
                &mut meta,
            ),
            "plan" => dr_v1_checked_transfer_plan_failure(
                ptr::null(),
                &status,
                &mut message,
                &mut path,
                &mut system_code,
                &mut transferred_byte_count,
                &mut meta,
            ),
            "group-access" | "group-configure" => dr_v1_checked_transfer_group_failure(
                ptr::null(),
                if invalid == "operation" {
                    OP_READ
                } else if projection == "group-access" {
                    OP_ACCESS
                } else {
                    OP_CONFIGURE
                },
                &status,
                &mut message,
                &mut path,
                &mut system_code,
                &mut transferred_byte_count,
                &mut meta,
            ),
            _ => unreachable!("unknown projection"),
        };
    }
    unreachable!("non-transfer invariants must fail fatally, not return an Error");
}
