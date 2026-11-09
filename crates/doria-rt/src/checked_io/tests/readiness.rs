use super::*;

#[test]
fn readiness_status_projects_every_reason_and_preserves_absent_system_and_progress_facts() {
    for operation in [OP_REGISTER, OP_UNREGISTER, OP_WAIT, OP_ACCESS] {
        for reason in [
            Reason::NotFound,
            Reason::PermissionDenied,
            Reason::InvalidInput,
            Reason::Interrupted,
            Reason::ResourceExhausted,
            Reason::Unsupported,
            Reason::Closed,
            Reason::Other,
        ] {
            for code in [None, Some(0), Some(13), Some(-7)] {
                let failure = Failure {
                    reason,
                    system_code: code,
                    transferred_byte_count: None,
                };
                let mut status = DrIoStatusV1::default();
                assert_eq!(status.fail(failure), 3);
                // Presence flags, not stale payload bytes, own optional facts.
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
                        dr_v1_checked_readiness_failure(
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
                    assert!(!message.is_null());
                    assert!(path.is_null());
                    assert_eq!(system_code, code.unwrap_or(0));
                    assert_eq!(transferred_byte_count, 0);
                    assert_eq!(
                        meta,
                        metadata(
                            operation,
                            TARGET_READINESS_SET,
                            reason as u8,
                            ERROR_IO,
                            code.is_some(),
                            false,
                            false,
                        )
                    );
                    let rendered =
                        core::slice::from_raw_parts(string_bytes(message), (*message).byte_length);
                    let rendered = core::str::from_utf8(rendered).unwrap();
                    assert!(rendered.contains("readiness set"), "{rendered}");
                    assert!(
                        rendered.contains(IoMessageOperation::ALL[operation as usize].word()),
                        "{rendered}"
                    );
                    if operation == OP_ACCESS && reason == Reason::InvalidInput {
                        assert_eq!(rendered, "failed to access readiness set: invalid input");
                    }
                    crate::dr_v1_string_release(message);
                }
                assert_eq!(status.failure(), Some(failure));
            }
        }
    }
}

#[test]
fn existing_operation_and_target_tags_stay_unchanged_when_readiness_is_appended() {
    assert_eq!(
        [OP_OPEN, OP_READ, OP_WRITE, OP_APPEND, OP_FLUSH, OP_CLOSE],
        [0, 1, 2, 3, 4, 5]
    );
    assert_eq!([OP_REGISTER, OP_UNREGISTER, OP_WAIT], [6, 7, 8]);
    assert_eq!([OP_CONFIGURE, OP_ACCESS], [9, 10]);
    assert_eq!(
        [TARGET_FILE, TARGET_STDIN, TARGET_STDOUT, TARGET_STDERR],
        [0, 1, 2, 3]
    );
    assert_eq!(TARGET_READINESS_SET, 4);
}
