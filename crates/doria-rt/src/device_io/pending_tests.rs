//! These run on each native host, rather than using a fake native owner for
//! standard devices. Queue injection changes no process handle or close claim.
use crate::transfer_group::adapter::DrAdapterContextV1;
use core::{ffi::c_void, mem::MaybeUninit, ptr};

use super::{standard_device, test_lock, StandardStream, TestInput};
#[cfg(windows)]
use crate::byte_io::NativePendingSet as PendingSet;
use crate::transfer_group::adapter::{AdapterTerminal, DrAdapterOutcomeV1};
use crate::transfer_group::pending::{
    dr_v1_checked_pending_request_outcome, dr_v1_pending_request_configure_standard,
    pending_callback_facts, DrPendingTransferAdapterV1,
};
use crate::transfer_group::{Buffer, BufferList, FixedSlice};
#[cfg(any(target_os = "linux", target_os = "macos"))]
use crate::unix_readiness::pending::PendingSet;
use crate::wait_budget::{dr_v1_wait_budget_init, DrWaitBudgetV1};
use crate::{DrInterfaceCarrierV1, DrStackFrameV2};

#[path = "pending_tests/alias_tests.rs"]
mod alias_tests;

#[derive(Default)]
struct Adapter {
    storage: [u8; 2],
    prepares: usize,
    completes: usize,
    submitted: u8,
    observed: u64,
    driver: Option<std::thread::ThreadId>,
}

unsafe extern "C" fn prepare(
    _: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    request: *mut c_void,
    _: *mut u8,
    length: usize,
    _: *mut DrAdapterOutcomeV1,
    _: *mut DrInterfaceCarrierV1,
) -> u8 {
    let adapter = &mut *(*context).object.cast::<Adapter>();
    if let Some(driver) = adapter.driver {
        assert_eq!(std::thread::current().id(), driver);
    }
    adapter.prepares += 1;
    assert_eq!(length, 2);
    dr_v1_pending_request_configure_standard(request, 0, adapter.storage.as_mut_ptr(), length);
    0
}

unsafe extern "C" fn complete(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    request: *mut c_void,
    destination: *mut u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    _: *mut DrInterfaceCarrierV1,
) -> u8 {
    let adapter = &mut *(*context).object.cast::<Adapter>();
    if let Some(driver) = adapter.driver {
        assert_eq!(std::thread::current().id(), driver);
    }
    adapter.completes += 1;
    adapter.submitted = pending_callback_facts(request).1;
    let (mut tag, mut count) = (0, 0);
    let (mut message, mut path) = (ptr::null_mut(), ptr::null_mut());
    let (mut code, mut transferred, mut meta) = (0, 0, 0);
    let returned = dr_v1_checked_pending_request_outcome(
        frame,
        request,
        &mut tag,
        &mut count,
        &mut message,
        &mut path,
        &mut code,
        &mut transferred,
        &mut meta,
    );
    if returned == 4 {
        assert_eq!(adapter.submitted, 0);
        assert!(message.is_null() && path.is_null());
        adapter.observed = 4;
        return 0;
    }
    assert_eq!(returned, 0);
    assert!(message.is_null() && path.is_null());
    assert!(count <= length);
    adapter.observed = tag;
    if tag == 0 {
        // Actual buffered input still passes through the COMPLETE adapter,
        // including its staged region and transformation, before publication.
        for index in 0..count {
            destination.add(index).write(adapter.storage[index] ^ 0xff);
        }
    }
    *outcome = DrAdapterOutcomeV1 {
        count,
        tag: tag as u8,
        has_count: u8::from(tag == 0),
    };
    0
}

#[test]
fn standard_buffered_input_preserves_complete_adapter_transformation_and_eof_on_native_hosts() {
    let _serial = test_lock();
    unsafe {
        let _input = TestInput::new(b"\xff\0ab", true);
        let device = standard_device(StandardStream::Stdin);
        let mut adapter = Adapter::default();
        let descriptor = DrPendingTransferAdapterV1 {
            context: DrAdapterContextV1 {
                object: ptr::from_mut(&mut adapter).cast(),
                descriptor: core::ptr::null(),
            },
            prepare_read: Some(prepare),
            prepare_write: None,
            complete_read: Some(complete),
            complete_write: None,
        };
        let mut set = PendingSet::new(ptr::null_mut()).unwrap();
        let source = *device.source().home().cast::<*const c_void>();
        let index = set.add_source(1, source, descriptor).unwrap();
        let handle = super::byte_handle(StandardStream::Stdin).unwrap();
        let mut bytes = [0; 2];
        let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
        let mut selected = FixedSlice::new(1).unwrap();
        selected.push(index).unwrap();
        let mut budget = MaybeUninit::<DrWaitBudgetV1>::uninit();
        let (mut reason, mut code, mut has_code) = (0, 0, 0);
        assert_eq!(
            dr_v1_wait_budget_init(
                1,
                0,
                budget.as_mut_ptr(),
                &mut reason,
                &mut code,
                &mut has_code
            ),
            0
        );
        let budget = budget.assume_init();
        let starts = super::helper::starts_for_test();
        let mut group = set
            .begin_selected(ptr::null(), buffers, &budget, selected)
            .unwrap();
        let (_, allocations) = crate::probe_runtime_allocations(|| {
            assert!(matches!(
                group.next(ptr::null()).unwrap().outcome.transfer,
                Some(AdapterTerminal::Data(2))
            ));
            assert_eq!(group.completed(0).unwrap(), b"\0\xff");
            assert_eq!(
                (adapter.prepares, adapter.completes, adapter.submitted),
                (1, 1, 0)
            );
            assert!(device.try_borrow().is_none());
            group.submit(ptr::null(), 0, 0, 2).unwrap();
            assert!(matches!(
                group.next(ptr::null()).unwrap().outcome.transfer,
                Some(AdapterTerminal::Data(2))
            ));
            assert_eq!(group.completed(0).unwrap(), &[b'a' ^ 0xff, b'b' ^ 0xff]);
            group.submit(ptr::null(), 0, 0, 2).unwrap();
            assert!(matches!(
                group.next(ptr::null()).unwrap().outcome.transfer,
                Some(AdapterTerminal::EndOfStream)
            ));
            assert_eq!(
                (
                    adapter.prepares,
                    adapter.completes,
                    adapter.submitted,
                    adapter.observed
                ),
                (3, 3, 0, 2)
            );
        });
        assert_eq!(allocations, 0);
        assert_eq!(
            super::helper::starts_for_test(),
            starts,
            "queued data and EOF start no helper"
        );
        drop(group);
        assert!(device.try_borrow().is_some());
        let ((), closes) = crate::native_handle::with_close_probe(|| set.remove(1).unwrap());
        assert_eq!(closes, 0);
        drop(set);
        assert_eq!(super::byte_handle(StandardStream::Stdin).unwrap(), handle);
        assert!(
            crate::native_handle::is_regular_file(handle).is_ok(),
            "device handle remains valid"
        );
    }
}

#[test]
fn helper_start_failure_is_readiness_registration_failure_and_completes_no_transfer_once() {
    let _serial = test_lock();
    unsafe {
        let _input = TestInput::new(b"", false);
        let device = standard_device(StandardStream::Stdin);
        let mut adapter = Adapter {
            driver: Some(std::thread::current().id()),
            ..Adapter::default()
        };
        let descriptor = DrPendingTransferAdapterV1 {
            context: DrAdapterContextV1 {
                object: ptr::from_mut(&mut adapter).cast(),
                descriptor: core::ptr::null(),
            },
            prepare_read: Some(prepare),
            prepare_write: None,
            complete_read: Some(complete),
            complete_write: None,
        };
        let mut set = PendingSet::new(ptr::null_mut()).unwrap();
        let index = set
            .add_source(1, device.source().value(), descriptor)
            .unwrap();
        let mut selected = FixedSlice::new(1).unwrap();
        selected.push(index).unwrap();
        let mut bytes = [0xa5; 2];
        let budget = DrWaitBudgetV1::indefinite();
        let failure = crate::platform_io::Failure::from_system_code(8);
        let starts = super::helper::starts_for_test();
        super::helper::fail_next_start_for_test(failure);
        let mut group = set
            .begin_selected(
                ptr::null(),
                BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap(),
                &budget,
                selected,
            )
            .unwrap();
        let outcome = group.next(ptr::null()).unwrap().outcome;
        assert_eq!(
            outcome.stop,
            Some(crate::transfer_group::Stop::Failure(
                doria_diagnostic_catalogue::IoMessageOperation::Register,
                failure,
            ))
        );
        assert_eq!(
            (
                adapter.prepares,
                adapter.completes,
                adapter.submitted,
                adapter.observed
            ),
            (1, 1, 0, 4)
        );
        assert_eq!(super::helper::starts_for_test(), starts);
        assert!(device.try_borrow().is_none());
        assert!(group.next(ptr::null()).is_none());
        drop(group);
        assert_eq!(adapter.completes, 1);
        assert_eq!(bytes, [0xa5; 2]);
        assert!(device.try_borrow().is_some());
        set.remove(1).unwrap();
    }
}

#[test]
fn overlapping_standard_admission_is_checked_and_does_not_release_another_consumers_loan() {
    let _serial = test_lock();
    unsafe {
        let _input = TestInput::new(b"kept", true);
        let device = standard_device(StandardStream::Stdin);
        let mut adapter = Adapter::default();
        let descriptor = DrPendingTransferAdapterV1 {
            context: DrAdapterContextV1 {
                object: ptr::from_mut(&mut adapter).cast(),
                descriptor: core::ptr::null(),
            },
            prepare_read: Some(prepare),
            prepare_write: None,
            complete_read: Some(complete),
            complete_write: None,
        };
        let mut set = PendingSet::new(ptr::null_mut()).unwrap();
        let index = set
            .add_source(1, device.source().value(), descriptor)
            .unwrap();
        let existing = device.borrow();
        let mut bytes = [0; 2];
        let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
        let mut selected = FixedSlice::new(1).unwrap();
        selected.push(index).unwrap();
        let budget = DrWaitBudgetV1::indefinite();
        assert!(
            matches!(set.begin_selected(ptr::null(), buffers, &budget, selected),
            Err(crate::transfer_group::SetupFailure::Platform(failure))
            if failure.reason == crate::platform_io::Reason::InvalidInput)
        );
        assert_eq!((adapter.prepares, adapter.completes), (0, 0));
        assert!(
            device.try_borrow().is_none(),
            "rollback may not release a foreign loan"
        );
        assert!(existing.buffered_input_ready());
        assert_eq!((*super::stdin_input()).start, 0);
        drop(existing);
        assert!(device.try_borrow().is_some());
        set.remove(1).unwrap();
    }
}

#[test]
fn inherited_pipe_helpers_keep_driver_callbacks_and_survive_repeated_wait_expiry() {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "device_io::pending_tests::inherited_pending_child",
            "--nocapture",
        ])
        .env("DORIA_RT_TEST_INHERITED_PENDING", "reuse")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let (send, receive) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut all = Vec::new();
        let mut byte = [0; 1];
        while stderr.read_exact(&mut byte).is_ok() {
            all.push(byte[0]);
            let _ = send.send(byte[0]);
        }
        all
    });
    for bytes in [b"\xff\0".as_slice(), b"ab".as_slice()] {
        let ready = receive.recv_timeout(std::time::Duration::from_secs(10));
        if ready != Ok(b'R') {
            let _ = child.kill();
            let output = child.wait_with_output().unwrap();
            panic!(
                "helper fixture did not reach live wait: {ready:?}; stdout={:?}; stderr={:?}",
                output.stdout,
                reader.join().unwrap()
            );
        }
        child.stdin.as_mut().unwrap().write_all(bytes).unwrap();
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "stdout={:?}; stderr={:?}",
        output.stdout,
        reader.join().unwrap()
    );
}

#[test]
fn inherited_pending_child() {
    if std::env::var("DORIA_RT_TEST_INHERITED_PENDING").as_deref() != Ok("reuse") {
        return;
    }
    use std::io::Write;
    unsafe {
        let _input = TestInput::new(b"", false);
        let device = standard_device(StandardStream::Stdin);
        let mut adapter = Adapter {
            driver: Some(std::thread::current().id()),
            ..Adapter::default()
        };
        let descriptor = DrPendingTransferAdapterV1 {
            context: DrAdapterContextV1 {
                object: ptr::from_mut(&mut adapter).cast(),
                descriptor: core::ptr::null(),
            },
            prepare_read: Some(prepare),
            prepare_write: None,
            complete_read: Some(complete),
            complete_write: None,
        };
        let mut set = PendingSet::new(ptr::null_mut()).unwrap();
        let starts = super::helper::starts_for_test();
        let index = set
            .add_source(1, device.source().value(), descriptor)
            .unwrap();
        assert_eq!(
            super::helper::starts_for_test(),
            starts,
            "idle registration creates no worker"
        );
        let mut selected = FixedSlice::new(1).unwrap();
        selected.push(index).unwrap();
        let mut bytes = [0; 2];
        let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
        let indefinite = DrWaitBudgetV1::indefinite();
        let mut group = set
            .begin_selected(ptr::null(), buffers, &indefinite, selected)
            .unwrap();
        assert_eq!(super::helper::starts_for_test(), starts + 1);
        let mut immediate = MaybeUninit::uninit();
        let (mut reason, mut code, mut present) = (0, 0, 0);
        assert_eq!(
            dr_v1_wait_budget_init(
                0,
                0,
                immediate.as_mut_ptr(),
                &mut reason,
                &mut code,
                &mut present
            ),
            0
        );
        let immediate = immediate.assume_init();
        for (iteration, expected) in [b"\0\xff".as_slice(), &[b'a' ^ 0xff, b'b' ^ 0xff]]
            .into_iter()
            .enumerate()
        {
            let (_, allocations) = crate::probe_runtime_allocations(|| {
                for _ in 0..2 {
                    assert!(matches!(
                        group.observe(ptr::null(), immediate),
                        crate::transfer_group::Observation::TimedOut
                    ));
                    assert!(
                        device.try_borrow().is_none(),
                        "wait expiry retains the canonical claim"
                    );
                }
                assert_eq!(
                    adapter.completes, iteration,
                    "no worker-thread completion callback"
                );
                std::io::stderr().write_all(b"R").unwrap();
                assert!(matches!(
                    group.next(ptr::null()).unwrap().outcome.transfer,
                    Some(AdapterTerminal::Data(2))
                ));
                assert_eq!(group.completed(0).unwrap(), expected);
            });
            assert_eq!(allocations, 0);
            assert_eq!(adapter.submitted, 1);
            assert_eq!(
                super::helper::starts_for_test(),
                starts + 1,
                "bounded record/thread reuse"
            );
            if iteration == 0 {
                let (_, allocations) = crate::probe_runtime_allocations(|| {
                    group.submit(ptr::null(), 0, 0, 2).unwrap()
                });
                assert_eq!(allocations, 0);
            }
        }
        assert_eq!((adapter.prepares, adapter.completes), (2, 2));
        drop(group);
        assert!(device.try_borrow().is_some());
        set.remove(1).unwrap();
    }
}
