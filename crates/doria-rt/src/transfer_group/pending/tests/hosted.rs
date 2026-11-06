use super::super::host::*;
use super::*;
use crate::transfer_group::adapter::host_error::{
    dr_v1_host_adapter_error_acknowledge, dr_v1_host_adapter_error_claim,
    dr_v1_host_adapter_error_disposition, dr_v1_host_adapter_error_initialize, HostErrorSlot,
};
use crate::transfer_group::setup::GroupSetup;
use core::{mem::MaybeUninit, task::Poll};

fn hosted_records() -> FixedSlice<PendingState> {
    let mut records = FixedSlice::new(1).unwrap();
    records
        .push(
            PendingState::with_adapter(PendingAdapter::Host {
                readable: true,
                writable: false,
            })
            .unwrap(),
        )
        .unwrap();
    records
}

unsafe fn action(record: *mut PendingState, view: &mut MaybeUninit<DrPendingRequestViewV1>) -> u8 {
    let mut caller = ptr::null_mut();
    let mut length = 0;
    let mut reading = 0;
    dr_v1_host_pending_action(
        record.cast(),
        view.as_mut_ptr().cast(),
        &mut caller,
        &mut length,
        &mut reading,
    )
}

unsafe fn source_reply(record: *mut PendingState) {
    assert_eq!(
        dr_v1_host_pending_source_reply(record.cast(), endpoint(0), ptr::null_mut()),
        0
    );
    assert_eq!(
        dr_v1_host_pending_source_reply(record.cast(), endpoint(0), ptr::null_mut()),
        1
    );
}

#[test]
fn ordinary_host_dispatch_retires_once_before_accepting_completion_without_allocations() {
    let events = ledger();
    let behavior = Rc::new(RefCell::new(Behavior::default()));
    behavior.borrow_mut().ready[0] = true;
    let mut states = hosted_records();
    let record = states.raw(0);
    let mut request = NativeRequest::new();
    let request = ptr::from_mut(&mut request);
    let mut bytes = [0; 4];
    let data = bytes.as_mut_ptr();
    let mut buffer = Buffer::Read(&mut bytes);
    let mut driver =
        unsafe { PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null()) };
    let mut view = MaybeUninit::uninit();
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        assert!(driver.poll_setup_request(0, request).is_pending());
        assert_eq!(action(record, &mut view), 1);
        source_reply(record);
        assert_eq!(action(record, &mut view), 0);
        assert!(matches!(
            driver.poll_setup_request(0, request),
            Poll::Ready(Ok(()))
        ));
        driver.reset_request(0, request);
        assert!(driver
            .poll_prepare_request(0, request, &mut buffer)
            .is_pending());
        assert_eq!(action(record, &mut view), 2);
        source_reply(record);
        assert!(driver
            .poll_prepare_request(0, request, &mut buffer)
            .is_pending());
        assert_eq!(action(record, &mut view), 3);
        assert_eq!((*record).caller_data, data);
        assert_eq!((*record).bound, 4);
        let home = view.as_mut_ptr().cast();
        dr_v1_pending_request_configure_native(home, endpoint(0), data, 4);
        assert_eq!(dr_v1_host_pending_prepare_reply(home, ptr::null_mut()), 0);
        assert_eq!(dr_v1_host_pending_prepare_reply(home, ptr::null_mut()), 1);
        assert!(matches!(
            driver.poll_prepare_request(0, request, &mut buffer),
            Poll::Ready(Ok(()))
        ));
        assert!(matches!(
            driver.submit(0, request, &mut buffer),
            Submission::Pending
        ));
        assert!(driver.poll_query(0, request).is_pending());
        assert!(!(*request).live && (*request).data.is_null());
        assert_eq!(action(record, &mut view), 4);
        assert_eq!(pending_callback_facts(home).0, 0);
        assert_eq!(pending_callback_facts(home).1, 1);
        assert!(driver.poll_query(0, request).is_pending());
        assert!(!events
            .borrow()
            .iter()
            .any(|event| matches!(event, Event::ReleaseSource(_))));
        assert_eq!(
            dr_v1_host_pending_complete_reply(home, 0, 3, ptr::null_mut()),
            0
        );
        assert_eq!(
            dr_v1_host_pending_complete_reply(home, 0, 3, ptr::null_mut()),
            1
        );
        assert!(matches!(
            driver.poll_query(0, request),
            Poll::Ready(Query::Terminal(AdapterTerminal::Data(3)))
        ));
        assert_eq!(action(record, &mut view), 0);
        assert_eq!(
            events
                .borrow()
                .iter()
                .filter(|event| **event == Event::Query(0))
                .count(),
            1
        );
        assert_eq!(
            events
                .borrow()
                .iter()
                .filter(|event| **event == Event::ReleaseSource(0))
                .count(),
            1
        );
        assert!(!events
            .borrow()
            .iter()
            .any(|event| matches!(event, Event::Prepare(..) | Event::Complete { .. })));
    });
    assert_eq!(allocations, 0);
}

#[test]
fn returned_host_preparation_is_completed_during_abandoned_startup_not_aborted() {
    let events = ledger();
    let behavior = Rc::new(RefCell::new(Behavior::default()));
    let cancelled = Rc::new(Cell::new(false));
    let mut states = hosted_records();
    let record = states.raw(0);
    let mut slots = RequestSlots::new(&[17], |_| NativeRequest::new()).unwrap();
    let mut bytes = [0; 4];
    let data = bytes.as_mut_ptr();
    let buffers = BufferList::from_iter([Buffer::Read(&mut bytes)].into_iter()).unwrap();
    let driver =
        unsafe { PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null()) };
    let mut setup =
        GroupSetup::configure(&mut slots, buffers, driver, Budget::new(&cancelled), None).unwrap();
    let mut view = MaybeUninit::uninit();
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        assert!(setup.poll(ptr::null()).is_pending());
        source_reply(record);
        assert!(setup.poll(ptr::null()).is_pending());
        source_reply(record);
        assert!(setup.poll(ptr::null()).is_pending());
        assert_eq!(action(record, &mut view), 3);
        let home = view.as_mut_ptr().cast();
        dr_v1_pending_request_configure_native(home, endpoint(0), data, 4);
        assert_eq!(dr_v1_host_pending_prepare_reply(home, ptr::null_mut()), 0);
        assert!(setup.poll_cleanup(ptr::null()).is_pending());
        assert_eq!(action(record, &mut view), 4);
        assert_eq!(pending_callback_facts(home).0, 4);
        assert_eq!(pending_callback_facts(home).1, 0);
        assert!((*record).validate_idle_source_update().is_err());
        assert_eq!(
            dr_v1_host_pending_complete_reply(home, 4, 0, ptr::null_mut()),
            0
        );
        assert!(setup.poll_cleanup(ptr::null()).is_ready());
        assert!((*record).validate_idle_source_update().is_ok());
        setup.drop_in_frame(ptr::null());
    });
    assert_eq!(allocations, 0);
    let events = events.borrow();
    assert!(!events.iter().any(|event| matches!(
        event,
        Event::Submit(..) | Event::Cancel(_) | Event::Retire(_)
    )));
    assert_eq!(
        events
            .iter()
            .filter(|event| **event == Event::ReleaseSource(0))
            .count(),
        1
    );
}

#[test]
fn immediate_and_rejected_host_submissions_have_one_deferred_completion() {
    for immediate in [true, false] {
        let events = ledger();
        let behavior = Rc::new(RefCell::new(Behavior::default()));
        behavior.borrow_mut().specs[0].submission =
            NativeSubmission::Rejected(Failure::unsupported());
        let mut states = hosted_records();
        let record = states.raw(0);
        let mut request = NativeRequest::new();
        let request = ptr::from_mut(&mut request);
        let mut bytes = [0; 4];
        let data = bytes.as_mut_ptr();
        let mut buffer = Buffer::Read(&mut bytes);
        let mut driver = unsafe {
            PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null())
        };
        let mut view = MaybeUninit::uninit();
        let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
            driver.reset_request(0, request);
            assert!(driver
                .poll_prepare_request(0, request, &mut buffer)
                .is_pending());
            source_reply(record);
            assert!(driver
                .poll_prepare_request(0, request, &mut buffer)
                .is_pending());
            assert_eq!(action(record, &mut view), 3);
            let home = view.as_mut_ptr().cast();
            if immediate {
                dr_v1_pending_request_complete(home, 0, 2)
            } else {
                dr_v1_pending_request_configure_native(home, endpoint(0), data, 4)
            }
            assert_eq!(dr_v1_host_pending_prepare_reply(home, ptr::null_mut()), 0);
            assert!(matches!(
                driver.poll_prepare_request(0, request, &mut buffer),
                Poll::Ready(Ok(()))
            ));
            assert!(matches!(
                driver.submit(0, request, &mut buffer),
                Submission::Complete
            ));
            assert!(driver.take_submitted_result(0, request).is_none());
            assert!(driver.poll_query(0, request).is_pending());
            assert_eq!(action(record, &mut view), 4);
            assert_eq!(pending_callback_facts(home).1, 0);
            assert!(driver.cancel(0, request).is_ok());
            assert_eq!(
                dr_v1_host_pending_complete_reply(home, 0, 1, ptr::null_mut()),
                0
            );
            assert!(matches!(
                driver.poll_drain(0, request),
                Poll::Ready(Some(AdapterTerminal::Data(1)))
            ));
        });
        assert_eq!(allocations, 0);
        let events = events.borrow();
        assert!(!events
            .iter()
            .any(|event| matches!(event, Event::Query(_) | Event::Retire(_) | Event::Cancel(_))));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, Event::Submit(..)))
                .count(),
            usize::from(!immediate)
        );
    }
}

#[test]
fn host_errors_keep_original_identity_through_preparation_and_validation_cleanup() {
    for throwing in [true, false] {
        let events = ledger();
        let behavior = Rc::new(RefCell::new(Behavior::default()));
        let mut states = hosted_records();
        let record = states.raw(0);
        let mut request = NativeRequest::new();
        let request = ptr::from_mut(&mut request);
        let mut bytes = [0; 4];
        let data = bytes.as_mut_ptr();
        let mut buffer = Buffer::Read(&mut bytes);
        let mut driver = unsafe {
            PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null())
        };
        let mut view = MaybeUninit::uninit();
        let mut error_slot = MaybeUninit::<HostErrorSlot>::uninit();
        let mut error = MaybeUninit::<AdapterTerminal>::uninit();
        let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
            let slot = error_slot.as_mut_ptr().cast();
            let terminal = error.as_mut_ptr().cast();
            dr_v1_host_adapter_error_initialize(slot, 73);
            assert_eq!(dr_v1_host_adapter_error_claim(slot, terminal, 1, 2, 4), 0);
            driver.reset_request(0, request);
            assert!(driver
                .poll_prepare_request(0, request, &mut buffer)
                .is_pending());
            source_reply(record);
            assert!(driver
                .poll_prepare_request(0, request, &mut buffer)
                .is_pending());
            assert_eq!(action(record, &mut view), 3);
            let home = view.as_mut_ptr().cast();
            if throwing {
                assert_eq!(dr_v1_host_pending_prepare_reply(home, terminal), 0);
            } else {
                dr_v1_pending_request_configure_native(home, endpoint(0), data, 4);
                dr_v1_pending_request_configure_native(home, endpoint(0), data, 1);
                assert_eq!(dr_v1_host_pending_prepare_reply(home, ptr::null_mut()), 0);
                assert!(driver
                    .poll_prepare_request(0, request, &mut buffer)
                    .is_pending());
                assert_eq!(action(record, &mut view), 4);
                assert!((*record).validate_idle_source_update().is_err());
                assert_eq!(dr_v1_host_pending_complete_reply(home, 0, 0, terminal), 0);
            }
            let Poll::Ready(Err(AdapterTerminal::HostError { error, progress })) =
                driver.poll_prepare_request(0, request, &mut buffer)
            else {
                panic!("original hosted error must win, without a native carrier");
            };
            assert_eq!(progress, Some(2));
            assert_eq!(dr_v1_host_adapter_error_disposition(slot), 1);
            assert_eq!(error.into_identity(), 73);
            assert_eq!(dr_v1_host_adapter_error_disposition(slot), 3);
            assert_eq!(dr_v1_host_adapter_error_acknowledge(slot), 0);
            assert_eq!(action(record, &mut view), 0);
            assert!((*record).validate_idle_source_update().is_ok());
        });
        assert_eq!(allocations, 0);
        let events = events.borrow();
        assert!(!events.iter().any(|event| matches!(
            event,
            Event::Submit(..) | Event::Query(_) | Event::Retire(_) | Event::Complete { .. }
        )));
        assert_eq!(
            events
                .iter()
                .filter(|event| **event == Event::ReleaseSource(0))
                .count(),
            1
        );
    }
}

#[test]
fn unreturned_host_preparation_releases_selection_without_completion() {
    let events = ledger();
    let behavior = Rc::new(RefCell::new(Behavior::default()));
    let mut states = hosted_records();
    let record = states.raw(0);
    let mut request = NativeRequest::new();
    let request = ptr::from_mut(&mut request);
    let mut bytes = [0; 4];
    let mut buffer = Buffer::Read(&mut bytes);
    let mut driver =
        unsafe { PendingDriver::new(Native::new(&events, &behavior), &mut states, ptr::null()) };
    let mut view = MaybeUninit::uninit();
    let ((), allocations) = crate::probe_runtime_allocations(|| unsafe {
        driver.reset_request(0, request);
        assert!(driver
            .poll_prepare_request(0, request, &mut buffer)
            .is_pending());
        source_reply(record);
        assert!(driver
            .poll_prepare_request(0, request, &mut buffer)
            .is_pending());
        assert_eq!(action(record, &mut view), 3);
        assert!(!driver.preparation_reply_ready(0, request));
        driver.abort_preparation(0, request);
        assert_eq!(action(record, &mut view), 0);
        assert!((*record).validate_idle_source_update().is_ok());
        assert_eq!(
            dr_v1_host_pending_prepare_reply(view.as_mut_ptr().cast(), ptr::null_mut()),
            1
        );
    });
    assert_eq!(allocations, 0);
    let events = events.borrow();
    assert_eq!(
        events
            .iter()
            .filter(|event| **event == Event::ReleaseSource(0))
            .count(),
        1
    );
    assert!(!events.iter().any(|event| matches!(
        event,
        Event::Submit(..) | Event::Query(_) | Event::Retire(_) | Event::Complete { .. }
    )));
}
