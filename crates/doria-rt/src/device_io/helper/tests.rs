use super::*;
use crate::byte_io::{ReadProgress, WriteProgress};
use crate::native_handle::OwnedHandle;

unsafe fn notice(_: usize) {}

unsafe fn stop_after_notice(worker: usize) {
    // run publishes while already holding this worker's predicate lock.
    (*(*(worker as *const Worker)).state.get()).stopping = true;
}

#[test]
fn cancellation_before_issue_retires_without_a_synthetic_native_result() {
    unsafe {
        let mut record = FixedSlice::new(1).unwrap();
        record.push(Worker::new()).unwrap();
        let worker = &record.as_slice()[0];
        #[cfg(unix)]
        let invalid = -1;
        #[cfg(windows)]
        let invalid = usize::MAX as NativeHandle;
        worker.submit(Job {
            handle: invalid,
            data: ptr::null_mut(),
            length: 1,
            reading: true,
            notify: stop_after_notice,
            notice: ptr::from_ref(worker) as usize,
        });
        {
            let _guard = worker.lock();
            (*worker.state.get()).cancelled = true;
        }
        worker.run();
        assert!(worker.drain().is_none(), "no host call means NoTransfer");
        assert!((*worker.state.get()).job.is_none());
        assert!(!(*worker.state.get()).running);
    }
}

fn pipe() -> (OwnedHandle, OwnedHandle) {
    #[cfg(unix)]
    unsafe {
        let mut descriptors = [-1; 2];
        assert_eq!(libc::pipe(descriptors.as_mut_ptr()), 0);
        (
            OwnedHandle::from_raw(descriptors[0]),
            OwnedHandle::from_raw(descriptors[1]),
        )
    }
    #[cfg(windows)]
    unsafe {
        unsafe extern "system" {
            fn CreatePipe(
                read: *mut NativeHandle,
                write: *mut NativeHandle,
                attributes: *mut core::ffi::c_void,
                size: u32,
            ) -> i32;
        }
        let (mut read, mut write) = (ptr::null_mut(), ptr::null_mut());
        assert_ne!(CreatePipe(&mut read, &mut write, ptr::null_mut(), 0), 0);
        assert!(crate::native_handle::require_synchronous_device(read).is_ok());
        assert!(crate::native_handle::require_synchronous_device(write).is_ok());
        (OwnedHandle::from_raw(read), OwnedHandle::from_raw(write))
    }
}

#[test]
fn helper_is_lazy_reuses_pinned_records_and_original_storage_without_transfer_allocations() {
    let (read, write) = pipe();
    let mut helper = Helper::new();
    let mut bytes = [0; 2];
    assert!(!helper.started());
    assert!(helper.record.is_none());
    unsafe {
        assert_eq!(
            crate::byte_io::write(write.get(), b"ab".as_ptr(), 2),
            Ok(WriteProgress::Accepted(2))
        );
        let job = Job {
            handle: read.get(),
            data: bytes.as_mut_ptr(),
            length: 2,
            reading: true,
            notify: notice,
            notice: 0,
        };
        let ((), costs) = crate::io_cost_probe::measure(|| helper.submit(job).unwrap());
        assert_eq!(
            costs[crate::io_cost_probe::Operation::HelperSubmit as usize],
            1
        );
        assert!(helper.started());
        assert!(matches!(
            helper.drain(),
            Some(FileTerminal::Read(Ok(ReadProgress::Data(2))))
        ));
        assert_eq!(&bytes, b"ab");
        let home = ptr::from_ref(helper.worker().unwrap());
        let starts = starts_for_test();
        let (_, allocations) = crate::probe_runtime_allocations(|| {
            assert_eq!(
                crate::byte_io::write(write.get(), b"cd".as_ptr(), 2),
                Ok(WriteProgress::Accepted(2))
            );
            helper.submit(job).unwrap();
            assert!(matches!(
                helper.drain(),
                Some(FileTerminal::Read(Ok(ReadProgress::Data(2))))
            ));
        });
        assert_eq!(allocations, 0);
        assert_eq!(&bytes, b"cd");
        assert_eq!(ptr::from_ref(helper.worker().unwrap()), home);
        assert_eq!(starts_for_test(), starts);
        helper.shutdown();
        assert!(!helper.started());
        assert_eq!(ptr::from_ref(helper.worker().unwrap()), home);
    }
}

#[test]
fn cancellation_never_frees_storage_or_replaces_an_actual_host_result() {
    let (read, write) = pipe();
    let mut helper = Helper::new();
    let mut bytes = [0xa5; 2];
    unsafe {
        helper
            .submit(Job {
                handle: read.get(),
                data: bytes.as_mut_ptr(),
                length: 2,
                reading: true,
                notify: notice,
                notice: 0,
            })
            .unwrap();
        // Wait for the issuing phase through its native predicate, not polling.
        let worker = helper.worker().unwrap();
        let mut guard = worker.lock();
        while !(*worker.state.get()).running && (*worker.state.get()).result.is_none() {
            guard.wait(worker.condition());
        }
        drop(guard);
        helper.cancel().unwrap();
        // The Unix host call is allowed to remain blocked after cancellation.
        // Windows may win cancellation or completion. Supply real host progress
        // before drain; either outcome must retain its original progress facts.
        assert_eq!(
            crate::byte_io::write(write.get(), b"xy".as_ptr(), 2),
            Ok(WriteProgress::Accepted(2))
        );
        let terminal = helper.drain().expect("actual native retirement");
        match terminal {
            FileTerminal::Read(Ok(ReadProgress::Data(2))) => assert_eq!(&bytes, b"xy"),
            #[cfg(windows)]
            FileTerminal::Read(Err(failure)) => {
                assert_eq!(failure.system_code, Some(995));
                assert_eq!(failure.transferred_byte_count, None);
            }
            _ => panic!("lost actual transfer result"),
        }
        helper.shutdown();
        assert!(!helper.started());
    }
}

#[test]
fn helper_failure_preserves_native_error_and_optional_progress() {
    let (read, write) = pipe();
    drop(read);
    let mut helper = Helper::new();
    unsafe {
        helper
            .submit(Job {
                handle: write.get(),
                data: b"x".as_ptr().cast_mut(),
                length: 1,
                reading: false,
                notify: notice,
                notice: 0,
            })
            .unwrap();
        let Some(FileTerminal::Write(Err(failure))) = helper.drain() else {
            panic!("missing host failure");
        };
        #[cfg(unix)]
        assert_eq!(failure.system_code, Some(libc::EPIPE.into()));
        #[cfg(windows)]
        assert!(matches!(failure.system_code, Some(109 | 232 | 233)));
        assert_eq!(
            failure.transferred_byte_count, None,
            "unproved zero is not progress"
        );
    }
}

#[cfg(windows)]
#[test]
fn unproved_async_standard_handles_never_use_synchronous_byte_operations() {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "doria-standard-async-{}-{unique}",
        std::process::id()
    ));
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .custom_flags(0x4000_0000)
        .open(&path)
        .unwrap();
    let handle = file.as_raw_handle();
    let mut data = *b"q";
    let unsupported = Failure::unsupported();
    unsafe {
        assert_eq!(
            crate::device_io::read_from_handle(handle, data.as_mut_ptr(), 1),
            Err(unsupported)
        );
        assert_eq!(
            crate::device_io::write_file_bytes(handle, data.as_ptr(), 1),
            crate::device_io::WriteOutcome::OtherFailure(unsupported)
        );
        assert_eq!(
            crate::device_io::read_from_handle(handle, ptr::null_mut(), 0),
            Ok(0)
        );
        assert_eq!(
            crate::device_io::write_file_bytes(handle, ptr::null(), 0),
            crate::device_io::WriteOutcome::Success
        );
        let mut helper = Helper::new();
        helper
            .submit(Job {
                handle,
                data: data.as_mut_ptr(),
                length: 1,
                reading: false,
                notify: notice,
                notice: 0,
            })
            .unwrap();
        assert!(
            matches!(helper.drain(), Some(FileTerminal::Write(Err(failure))) if failure == unsupported)
        );
        helper.shutdown();
    }
    assert_eq!(data, *b"q");
    assert_eq!(file.metadata().unwrap().len(), 0);
    drop(file);
    std::fs::remove_file(path).unwrap();
}
