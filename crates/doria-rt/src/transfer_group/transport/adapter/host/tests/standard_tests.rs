use super::*;
use crate::device_io::{standard_device, test_lock, StandardStream, TestInput};

unsafe fn close(
    set: *mut c_void,
    owner: *mut c_void,
    stream: StandardStream,
    calls: &mut CallStorage,
) {
    dr_v1_host_transfer_group_close(owner);
    let (closed, allocations) = crate::probe_runtime_allocations(|| calls.drive(owner, stream));
    assert_eq!((closed.0, closed.1, allocations), (0, 3, 0));
    dr_v1_host_transfer_group_drop(ptr::null(), owner);
    dr_v1_transfer_adapter_set_drop(ptr::null(), set);
    assert!(standard_device(stream).try_borrow_for_io().is_ok());
}

#[test]
fn hosted_standard_input_reuses_one_request_and_shared_queue_through_eof_without_allocating() {
    let _serial = test_lock();
    unsafe {
        let _input = TestInput::new(b"\xff\0Qend", true);
        let stream = StandardStream::Stdin;
        let mut bytes = [0; 3];
        let (set, owner, key) = start(stream, &mut bytes);
        let mut calls = CallStorage::new();
        let (started, allocations) =
            crate::probe_runtime_allocations(|| calls.drive(owner, stream));
        assert_eq!((started.0, started.1, allocations), (0, 0, 0));
        assert!(standard_device(stream).try_borrow_for_io().is_err());

        for (attempt, expected) in [Some(&b"\xff\0Q"[..]), Some(&b"end"[..]), None]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                dr_v1_host_transfer_group_observe_wait(
                    owner,
                    1,
                    0,
                    0,
                    &mut DrIoStatusV1::default()
                ),
                0
            );
            let (observed, allocations) =
                crate::probe_runtime_allocations(|| calls.drive(owner, stream));
            assert_eq!(
                (observed.0, observed.1, observed.2, allocations),
                (0, 1, 0, 0)
            );
            assert_eq!(
                (observed.3.tag, observed.3.count),
                expected.map_or((2, 0), |bytes| (0, bytes.len()))
            );
            let (mut data, mut length) = (ptr::null(), usize::MAX);
            assert_eq!(
                dr_v1_host_transfer_group_read_region(
                    owner,
                    ptr::from_ref(&key).cast(),
                    &mut data,
                    &mut length,
                    &mut DrIoStatusV1::default()
                ),
                0
            );
            assert_eq!(length, expected.map_or(0, <[u8]>::len));
            if let Some(expected) = expected {
                assert_eq!(core::slice::from_raw_parts(data, length), expected);
            }
            if attempt != 2 {
                assert_eq!(
                    dr_v1_host_transfer_group_resubmit(
                        owner,
                        ptr::from_ref(&key).cast(),
                        1,
                        0,
                        3,
                        &mut DrIoStatusV1::default()
                    ),
                    0
                );
                let (reused, allocations) =
                    crate::probe_runtime_allocations(|| calls.drive(owner, stream));
                assert_eq!((reused.0, reused.1, allocations), (0, 2, 0));
            }
        }
        assert_eq!(calls.actions, [0, 1, 3, 3, 3]);
        close(set, owner, stream, &mut calls);
    }
}

#[test]
fn hosted_empty_standard_requests_preserve_admission_and_never_consume_queued_input() {
    let _serial = test_lock();
    unsafe {
        let _input = TestInput::new(b"saved", true);
        for stream in [
            StandardStream::Stdin,
            StandardStream::Stdout,
            StandardStream::Stderr,
        ] {
            let (set, owner, key) = start(stream, &mut []);
            let mut calls = CallStorage::new();
            let (started, allocations) =
                crate::probe_runtime_allocations(|| calls.drive(owner, stream));
            assert_eq!((started.0, started.1, allocations), (0, 0, 0));
            assert!(standard_device(stream).try_borrow_for_io().is_err());
            for attempt in 0..2 {
                assert_eq!(
                    dr_v1_host_transfer_group_observe_wait(
                        owner,
                        1,
                        0,
                        0,
                        &mut DrIoStatusV1::default()
                    ),
                    0
                );
                let (observed, allocations) =
                    crate::probe_runtime_allocations(|| calls.drive(owner, stream));
                assert_eq!(
                    (
                        observed.0,
                        observed.1,
                        observed.3.tag,
                        observed.3.count,
                        allocations
                    ),
                    (0, 1, 0, 0, 0)
                );
                if attempt == 0 {
                    assert_eq!(
                        dr_v1_host_transfer_group_resubmit(
                            owner,
                            ptr::from_ref(&key).cast(),
                            TestSource::from(stream).reading(),
                            0,
                            0,
                            &mut DrIoStatusV1::default()
                        ),
                        0
                    );
                    assert_eq!(calls.drive(owner, stream).0, 0);
                }
            }
            assert_eq!(calls.actions, [0, 1, 2, 2, 2]);
            close(set, owner, stream, &mut calls);
        }
        let mut bytes = [0; 5];
        assert_eq!(
            crate::device_io::read_progress(StandardStream::Stdin, bytes.as_mut_ptr(), bytes.len()),
            Ok(crate::byte_io::ReadProgress::Data(5))
        );
        assert_eq!(&bytes, b"saved");
    }
}

#[test]
fn hosted_standard_configuration_rejects_wrong_devices_with_actual_target_and_no_path() {
    let _serial = test_lock();
    unsafe {
        let _input = TestInput::new(b"saved", true);
        for (stream, wrong) in [
            (StandardStream::Stdin, StandardStream::Stdout),
            (StandardStream::Stdout, StandardStream::Stderr),
            (StandardStream::Stderr, StandardStream::Stdout),
        ] {
            let mut bytes = [0; 3];
            let (set, owner, _) = start(stream, &mut bytes);
            let mut calls = CallStorage::new();
            reach_preparation(owner, stream, &mut calls);
            let mut error = crate::empty_error_carrier();
            let (mut message, mut path, mut system, mut transferred, mut meta) =
                (ptr::null_mut(), ptr::null_mut(), -99, usize::MAX, u64::MAX);
            assert_eq!(
                dr_v1_host_pending_request_configure_standard(
                    ptr::null(),
                    owner,
                    calls.view(),
                    wrong as u8,
                    standard_device(stream).source().value(),
                    calls.caller,
                    calls.length,
                    0,
                    calls.length,
                    &mut error,
                    &mut message,
                    &mut path,
                    &mut system,
                    &mut transferred,
                    &mut meta,
                ),
                3
            );
            assert!(error.object.is_null() && error.vtable.is_null());
            assert!(!message.is_null() && path.is_null());
            assert_eq!(
                (system, transferred, meta),
                (0, 0, 9 | ((wrong as u64 + 1) << 8) | (2 << 16))
            );
            crate::dr_v1_string_release(message);
            assert!(standard_device(stream).try_borrow_for_io().is_err());
            assert_eq!(
                dr_v1_host_pending_prepare_reply(calls.view(), ptr::null_mut()),
                0
            );
            assert_eq!(calls.drive(owner, stream).0, 3);
            assert!(matches!(
                (*owner.cast::<HostGroupOwner>()).call,
                Call::Closed
            ));
            assert_eq!(calls.actions, [0, 1, 1, 1, 1]);
            dr_v1_host_transfer_group_drop(ptr::null(), owner);
            dr_v1_transfer_adapter_set_drop(ptr::null(), set);
            assert!(standard_device(stream).try_borrow_for_io().is_ok());
            assert_eq!(bytes, [0; 3]);
        }
    }
}

#[test]
fn hosted_standard_cancellation_drains_one_completion_and_preserves_prepared_progress() {
    use crate::cancellation::*;
    let _serial = test_lock();
    unsafe {
        let _input = TestInput::new(b"saved", true);
        let stream = StandardStream::Stdin;
        let cancellation = dr_v1_cancellation_new();
        let mut bytes = [0; 3];
        let (set, owner, _) = start_with_control(
            stream,
            &mut bytes,
            cancellation,
            &DrWaitBudgetV1::indefinite(),
        );
        let mut calls = CallStorage::new();
        reach_preparation(owner, stream, &mut calls);
        calls.configure(owner, stream);
        let (mut reason, mut system, mut has_system) = (0, 0, 0);
        assert_eq!(
            dr_v1_cancellation_request(cancellation, &mut reason, &mut system, &mut has_system),
            0
        );
        dr_v1_cancellation_release(cancellation);
        assert_eq!(calls.drive(owner, stream).0, 0);
        assert_eq!(
            dr_v1_host_transfer_group_observe_wait(owner, 1, 0, 0, &mut DrIoStatusV1::default()),
            0
        );
        let (observed, allocations) =
            crate::probe_runtime_allocations(|| calls.drive(owner, stream));
        assert_eq!(
            (
                observed.0,
                observed.1,
                observed.3.tag,
                observed.3.count,
                allocations
            ),
            (0, 1, 0, 3, 0)
        );
        assert_eq!(calls.actions, [0, 1, 1, 1, 1]);
        close(set, owner, stream, &mut calls);
        assert_eq!(&bytes, b"sav");
        let mut saved = [0; 2];
        assert_eq!(
            crate::device_io::read_progress(stream, saved.as_mut_ptr(), saved.len()),
            Ok(crate::byte_io::ReadProgress::Data(2))
        );
        assert_eq!(&saved, b"ed");
    }
}

#[test]
fn hosted_standard_cancellation_before_preparation_leaves_the_queue_untouched() {
    use crate::cancellation::*;
    let _serial = test_lock();
    unsafe {
        let _input = TestInput::new(b"saved", true);
        let stream = StandardStream::Stdin;
        let cancellation = dr_v1_cancellation_new();
        let mut bytes = [0; 3];
        let (set, owner, _) = start_with_control(
            stream,
            &mut bytes,
            cancellation,
            &DrWaitBudgetV1::indefinite(),
        );
        let (mut reason, mut system, mut has_system) = (0, 0, 0);
        assert_eq!(
            dr_v1_cancellation_request(cancellation, &mut reason, &mut system, &mut has_system),
            0
        );
        dr_v1_cancellation_release(cancellation);
        let mut calls = CallStorage::new();
        assert_eq!(calls.drive(owner, stream).0, 0);
        assert_eq!(
            dr_v1_host_transfer_group_observe_wait(owner, 1, 0, 0, &mut DrIoStatusV1::default()),
            0
        );
        let (observed, allocations) =
            crate::probe_runtime_allocations(|| calls.drive(owner, stream));
        assert_eq!((observed.0, observed.1, allocations), (4, 1, 0));
        assert_eq!(calls.actions, [0; 5]);
        close(set, owner, stream, &mut calls);
        assert_eq!(bytes, [0; 3]);
        let mut saved = [0; 5];
        assert_eq!(
            crate::device_io::read_progress(stream, saved.as_mut_ptr(), saved.len()),
            Ok(crate::byte_io::ReadProgress::Data(5))
        );
        assert_eq!(&saved, b"saved");
    }
}
