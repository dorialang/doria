use super::*;
use crate::transfer_group::plan::OperationKey;

unsafe fn controlled_member(
    fixture: &Fixture,
    table: &DrInterfaceVtableV1,
    prepare_read: PrepareReadCallback,
) -> (*mut c_void, PlanKey, *mut Stream) {
    let mut status = DrIoStatusV1::default();
    let set = dr_v1_readiness_set_new(ptr::null(), &mut status);
    let mut value = fixture.stream(1, table);
    let stream = value.object.cast::<Stream>();
    let adapter = DrPendingTransferAdapterV1 {
        context: DrAdapterContextV1 {
            object: value.object.cast(),
            descriptor: ptr::null(),
        },
        prepare_read: Some(prepare_read),
        prepare_write: None,
        complete_read: Some(complete),
        complete_write: None,
    };
    let projection = source(&value);
    let mut registration = PlanKey { owner: 0, value: 0 };
    assert_eq!(
        dr_v1_readiness_set_add_owned(
            ptr::null(),
            set,
            projection,
            Some(resolve_source),
            &adapter,
            &mut value,
            ptr::from_mut(&mut registration).cast(),
            &mut status,
        ),
        0
    );
    (set, registration, stream)
}

unsafe fn controlled_plan(registration: &PlanKey, bytes: &mut [u8]) -> *mut c_void {
    let buffer = DrTransferBufferV1 {
        data: bytes.as_mut_ptr(),
        length: bytes.len(),
        reading: 1,
    };
    let mut status = DrIoStatusV1::default();
    let plan = plan::dr_v1_transfer_plan_new(
        ptr::null(),
        ptr::from_ref(registration).cast(),
        &buffer,
        &mut status,
    );
    assert!(!plan.is_null());
    plan
}

unsafe fn controlled_begin(
    set: *mut c_void,
    plan: *mut c_void,
    deadline: *const crate::deadline::DrDeadlineV1,
    cancellation: *mut crate::cancellation::DrCancellationV1,
    error: &mut DrInterfaceCarrierV1,
    expected: u8,
) -> *mut c_void {
    let mut group = ptr::dangling_mut();
    let mut fields = CheckedFields::empty();
    let tag = super::super::super::checked_group::dr_v2_checked_transfer_group_begin(
        ptr::null(),
        set,
        plan,
        deadline,
        cancellation,
        &mut group,
        error,
        &mut fields.message,
        &mut fields.path,
        &mut fields.system_code,
        &mut fields.transferred,
        &mut fields.meta,
    );
    assert_eq!(tag, expected);
    assert_eq!(group.is_null(), tag != 0);
    if tag != 1 {
        fields.assert_success();
    }
    group
}

#[test]
fn checked_group_controls_retain_state_copy_deadlines_and_do_not_leak_to_reused_sets() {
    use crate::cancellation::*;
    use crate::deadline::DrDeadlineV1;
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let (set, registration, stream) = controlled_member(&fixture, &table, prepare);
        let source = dr_v1_cancellation_new();
        let token = dr_v1_cancellation_retain(source);
        let mut reason = 0;
        let mut code = 0;
        let mut has_code = 0;
        assert_eq!(
            dr_v1_cancellation_request(source, &mut reason, &mut code, &mut has_code),
            0
        );
        let mut bytes = [0x55; 2];
        let mut error = crate::empty_error_carrier();
        let group = controlled_begin(
            set,
            controlled_plan(&registration, &mut bytes),
            ptr::null(),
            token,
            &mut error,
            0,
        );
        assert_eq!(owned_claim_count(token), 3);
        dr_v1_cancellation_release(source);
        assert_eq!(owned_claim_count(token), 2);
        let output = fetch(group, 1, 0, 0, &mut error);
        assert_eq!(output.allocations, 0);
        assert_eq!(
            (
                output.tag,
                output.result,
                output.control.stop,
                output.outcome
            ),
            (0, 0, 1, 0)
        );
        assert_eq!(bytes, [0x55; 2]);
        dr_v1_transfer_adapter_group_drop(ptr::null(), group);
        assert_eq!(owned_claim_count(token), 1);
        assert!(!(*stream).pending);

        let expired = DrDeadlineV1::from_parts(u64::MAX, -1);
        let group = controlled_begin(
            set,
            controlled_plan(&registration, &mut bytes),
            &expired,
            ptr::null_mut(),
            &mut error,
            0,
        );
        let output = fetch(group, 1, 0, 0, &mut error);
        assert_eq!(output.allocations, 0);
        assert_eq!(
            (
                output.tag,
                output.result,
                output.control.stop,
                output.outcome
            ),
            (0, 0, 2, 0)
        );
        assert_eq!(bytes, [0x55; 2]);
        dr_v1_transfer_adapter_group_drop(ptr::null(), group);

        let fresh = dr_v1_cancellation_new();
        let mut deadline = DrDeadlineV1::from_parts(u64::MAX, i64::MAX);
        let group = controlled_begin(
            set,
            controlled_plan(&registration, &mut bytes),
            &deadline,
            fresh,
            &mut error,
            0,
        );
        assert_eq!(owned_claim_count(fresh), 2);
        deadline = expired;
        assert_eq!(deadline, expired);
        let output = fetch(group, 1, 0, 0, &mut error);
        assert_eq!(output.allocations, 0);
        assert_eq!(
            (
                output.tag,
                output.result,
                output.control.stop,
                output.outcome,
                output.count
            ),
            (0, 0, 0, 1, 2)
        );
        assert_eq!(bytes, *b"AB");
        dr_v1_transfer_adapter_group_drop(ptr::null(), group);
        assert_eq!(owned_claim_count(fresh), 1);

        let group = controlled_begin(
            set,
            controlled_plan(&registration, &mut bytes),
            ptr::null(),
            ptr::null_mut(),
            &mut error,
            0,
        );
        let output = fetch(group, 1, 0, 0, &mut error);
        assert_eq!(output.allocations, 0);
        assert_eq!(
            (
                output.tag,
                output.result,
                output.control.stop,
                output.outcome
            ),
            (0, 0, 0, 3)
        );
        dr_v1_transfer_adapter_group_drop(ptr::null(), group);
        dr_v1_cancellation_release(token);
        dr_v1_cancellation_release(fresh);
        dr_v1_transfer_adapter_set_drop(ptr::null(), set);
        assert_eq!(*fixture.drops.borrow(), [1]);
        assert!(error.object.is_null() && error.vtable.is_null());
    }
}

unsafe extern "C" fn fail_controlled_preparation(
    _: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    _: *mut c_void,
    _: *mut u8,
    _: usize,
    _: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    *error = (*observed_stream(context))
        .completion_error
        .take()
        .unwrap()
        .into_carrier();
    1
}

#[test]
fn controlled_setup_failure_preserves_original_error_and_releases_group_state() {
    unsafe extern "C" fn drop_error(_: *const DrStackFrameV2, object: *mut u8) {
        drop(Box::from_raw(object.cast::<usize>()));
    }
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let error_table = DrInterfaceVtableV1 {
            drop: drop_error,
            class_size: mem::size_of::<usize>(),
            ..descriptor()
        };
        let original = DrInterfaceCarrierV1 {
            object: Box::into_raw(Box::new(47_usize)).cast(),
            vtable: &error_table,
        };
        let identity = (original.object, original.vtable);
        let (set, registration, stream) =
            controlled_member(&fixture, &table, fail_controlled_preparation);
        (*stream).completion_error = Some(
            crate::transfer_group::adapter::OwnedIoError::from_carrier(original),
        );
        let state = crate::cancellation::dr_v1_cancellation_new();
        let mut bytes = [0x55; 2];
        let mut error = crate::empty_error_carrier();
        controlled_begin(
            set,
            controlled_plan(&registration, &mut bytes),
            ptr::null(),
            state,
            &mut error,
            2,
        );
        assert_eq!(crate::cancellation::owned_claim_count(state), 1);
        assert_eq!((error.object, error.vtable), identity);
        assert_eq!(*error.object.cast::<usize>(), 47);
        assert_eq!(bytes, [0x55; 2]);
        assert!(!(*set.cast::<AdapterSetOwner>()).active);
        drop(crate::transfer_group::adapter::OwnedIoError::from_carrier(
            error,
        ));
        crate::cancellation::dr_v1_cancellation_release(state);
        dr_v1_transfer_adapter_set_drop(ptr::null(), set);
        assert_eq!(*fixture.drops.borrow(), [1]);
    }
}

pub(super) struct Fetch {
    pub(super) tag: u8,
    pub(super) allocations: usize,
    pub(super) result: u8,
    pub(super) reading: u8,
    pub(super) key: OperationKey,
    pub(super) outcome: u8,
    pub(super) count: usize,
    pub(super) control: DrCompletionStatusV1,
    pub(super) fields: CheckedFields,
}

pub(super) unsafe fn fetch(
    group: *mut c_void,
    kind: u8,
    low: u64,
    high: i64,
    error: &mut DrInterfaceCarrierV1,
) -> Fetch {
    let mut result = Fetch {
        tag: 255,
        allocations: usize::MAX,
        result: 255,
        reading: 255,
        key: OperationKey {
            owner: u64::MAX,
            position: u64::MAX,
        },
        outcome: 255,
        count: usize::MAX,
        control: DrCompletionStatusV1::default(),
        fields: CheckedFields::empty(),
    };
    (result.tag, result.allocations) = crate::probe_runtime_allocations(|| {
        super::super::super::checked_group::dr_v1_checked_transfer_group_observe(
            ptr::null(),
            group,
            kind,
            low,
            high,
            &mut result.result,
            &mut result.reading,
            ptr::from_mut(&mut result.key).cast(),
            &mut result.outcome,
            &mut result.count,
            error,
            ptr::from_mut(&mut result.control).cast(),
            &mut result.fields.message,
            &mut result.fields.path,
            &mut result.fields.system_code,
            &mut result.fields.transferred,
            &mut result.fields.meta,
        )
    });
    result
}

#[test]
fn checked_erased_pending_fetch_preserves_inline_keys_and_no_allocation() {
    unsafe { exercise_owned_erased_callbacks(true, true) }
}

#[test]
fn checked_erased_synchronous_fetch_preserves_inline_keys_and_no_allocation() {
    unsafe { exercise_owned_erased_callbacks(false, true) }
}

#[test]
fn checked_fetch_rejects_null_group_without_dereferencing_or_inventing_progress() {
    unsafe {
        let mut error = crate::empty_error_carrier();
        let output = fetch(ptr::null_mut(), 1, 0, 0, &mut error);
        assert_eq!(
            (
                output.tag,
                output.allocations,
                output.reading,
                output.outcome,
                output.count
            ),
            (1, 1, 0, 0, 0)
        );
        assert_eq!((output.key.owner, output.key.position), (0, 0));
        output.fields.assert_failure(
            doria_diagnostic_catalogue::IoMessageOperation::Wait,
            Reason::InvalidInput,
        );
        assert!(error.object.is_null() && error.vtable.is_null());
    }
}

unsafe extern "C" fn prepare_ready<const TAG: u8>(
    _: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    _: *mut u8,
    length: usize,
    _: *mut DrAdapterOutcomeV1,
    _: *mut DrInterfaceCarrierV1,
) -> u8 {
    (*observed_stream(context)).pending = true;
    dr_v1_pending_request_complete(view, TAG, if TAG == 0 { length } else { 0 });
    0
}

unsafe extern "C" fn prepare_ready_write<const TAG: u8>(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    data: *const u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    prepare_ready::<TAG>(
        frame,
        context,
        view,
        data.cast_mut(),
        length,
        outcome,
        error,
    )
}

unsafe extern "C" fn complete_ready<const ABSENT: bool>(
    _: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    _: *mut u8,
    _: usize,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    let stream = &mut *observed_stream(context);
    stream.pending = false;
    if let Some(original) = stream.completion_error.take() {
        *error = original.into_carrier();
        *outcome = DrAdapterOutcomeV1 {
            tag: 0,
            has_count: 1,
            count: 1,
        };
        return 1;
    }
    if ABSENT {
        return 4;
    }
    let (_, _, _, _, immediate) = pending_callback_facts(view);
    assert!(!immediate.is_null());
    *outcome = *immediate;
    0
}

unsafe extern "C" fn complete_ready_write<const ABSENT: bool>(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    data: *const u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    complete_ready::<ABSENT>(
        frame,
        context,
        view,
        data.cast_mut(),
        length,
        outcome,
        error,
    )
}

unsafe fn start_ready<const ABSENT: bool>(
    fixture: &Fixture,
    table: &DrInterfaceVtableV1,
    bytes: &mut [u8],
    reading: bool,
    prepare_read: PrepareReadCallback,
    prepare_write: PrepareWriteCallback,
    original_error: Option<crate::transfer_group::adapter::OwnedIoError>,
) -> (*mut c_void, *mut c_void, OperationKey) {
    let mut status = DrIoStatusV1::default();
    let set = dr_v1_readiness_set_new(ptr::null(), &mut status);
    let mut value = fixture.stream(1, table);
    (*value.object.cast::<Stream>()).completion_error = original_error;
    let adapter = DrPendingTransferAdapterV1 {
        context: DrAdapterContextV1 {
            object: value.object.cast(),
            descriptor: ptr::null(),
        },
        prepare_read: reading.then_some(prepare_read),
        prepare_write: (!reading).then_some(prepare_write),
        complete_read: reading.then_some(complete_ready::<ABSENT>),
        complete_write: (!reading).then_some(complete_ready_write::<ABSENT>),
    };
    let source = source(&value);
    let mut registration = PlanKey { owner: 0, value: 0 };
    assert_eq!(
        dr_v1_readiness_set_add_owned(
            ptr::null(),
            set,
            source,
            Some(resolve_source),
            &adapter,
            &mut value,
            ptr::from_mut(&mut registration).cast(),
            &mut status
        ),
        0
    );
    let buffer = DrTransferBufferV1 {
        data: bytes.as_mut_ptr(),
        length: bytes.len(),
        reading: u8::from(reading),
    };
    let plan = plan::dr_v1_transfer_plan_new(
        ptr::null(),
        ptr::from_ref(&registration).cast(),
        &buffer,
        &mut status,
    );
    assert!(!plan.is_null());
    let key = (*plan.cast::<TransferPlan<'static>>())
        .operation_key(0, reading)
        .unwrap();
    let group = if ABSENT {
        // A group-wide expired deadline retires unsubmitted operations. There
        // is no native or immediate result for an absent completion to erase.
        let budget =
            DrWaitBudgetV1::from_deadline(crate::deadline::DrDeadlineV1::from_parts(u64::MAX, -1));
        dr_v1_pending_transfer_adapter_plan_begin(
            ptr::null(),
            set,
            plan,
            &budget,
            &mut crate::empty_error_carrier(),
            &mut status,
        )
    } else {
        super::super::tests::checked_begin(
            ptr::null(),
            set,
            plan,
            &mut crate::empty_error_carrier(),
            0,
        )
    };
    assert!(!group.is_null());
    (set, group, key)
}

#[test]
fn checked_fetch_preserves_every_explicit_outcome_and_reuses_original_keys() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let prepares: [(PrepareReadCallback, PrepareWriteCallback); 4] = [
            (prepare_ready::<0>, prepare_ready_write::<0>),
            (prepare_ready::<1>, prepare_ready_write::<1>),
            (prepare_ready::<2>, prepare_ready_write::<0>),
            (prepare_ready::<3>, prepare_ready_write::<0>),
        ];
        for reading in [true, false] {
            for (tag, &(read, write)) in
                prepares
                    .iter()
                    .enumerate()
                    .take(if reading { 4 } else { 2 })
            {
                let mut bytes = [0; 2];
                let (set, group, key) =
                    start_ready::<false>(&fixture, &table, &mut bytes, reading, read, write, None);
                let mut error = crate::empty_error_carrier();
                let rejected = fetch(group, 255, 0, 0, &mut error);
                assert_eq!((rejected.tag, rejected.allocations), (1, 1));
                rejected.fields.assert_failure(
                    doria_diagnostic_catalogue::IoMessageOperation::Wait,
                    Reason::InvalidInput,
                );
                assert_eq!(super::super::tests::observed(group), None);
                let output = fetch(group, 1, 0, 0, &mut error);
                assert_eq!((output.tag, output.allocations, output.result), (0, 0, 0));
                assert_eq!(
                    (output.key, output.reading, output.outcome, output.count),
                    (
                        key,
                        u8::from(reading),
                        tag as u8 + 1,
                        if tag == 0 { 2 } else { 0 }
                    )
                );
                output.fields.assert_success();
                assert_eq!(super::super::tests::observed(group), Some((key, reading)));
                if reading {
                    let prefix = match tag {
                        0 => Some((bytes.as_ptr(), 2)),
                        2 => Some((bytes.as_ptr(), 0)),
                        _ => None,
                    };
                    super::super::tests::assert_read_region(group, key, prefix);
                }
                let exhausted = fetch(group, 1, 0, 0, &mut error);
                assert_eq!(
                    (exhausted.tag, exhausted.allocations, exhausted.result),
                    (0, 0, 2)
                );
                exhausted.fields.assert_success();
                assert_eq!(super::super::tests::observed(group), None);
                super::super::tests::assert_checked_resubmit(
                    group,
                    key,
                    u8::from(reading),
                    0,
                    1,
                    true,
                );
                let reused = fetch(group, 1, 0, 0, &mut error);
                assert_eq!(
                    (
                        reused.tag,
                        reused.allocations,
                        reused.key,
                        reused.outcome,
                        reused.count
                    ),
                    (0, 0, key, tag as u8 + 1, usize::from(tag == 0))
                );
                reused.fields.assert_success();
                dr_v1_transfer_adapter_group_drop(ptr::null(), group);
                dr_v1_readiness_set_drop(ptr::null(), set);
            }
        }
        assert_eq!(fixture.drops.borrow().len(), 6);
    }
}

#[test]
fn checked_fetch_moves_the_original_error_once_without_reallocation_or_reconstruction() {
    struct OriginalError<'a> {
        dropped: &'a std::cell::Cell<usize>,
        progress: i64,
    }
    unsafe extern "C" fn drop_original(_: *const DrStackFrameV2, object: *mut u8) {
        let error = Box::from_raw(object.cast::<OriginalError<'_>>());
        error.dropped.set(error.dropped.get() + 1);
    }
    unsafe {
        let fixture = Fixture::new();
        let dropped = std::cell::Cell::new(0);
        let table = descriptor();
        let error_table = DrInterfaceVtableV1 {
            drop: drop_original,
            class_size: mem::size_of::<OriginalError<'_>>(),
            ..descriptor()
        };
        let original = Box::into_raw(Box::new(OriginalError {
            dropped: &dropped,
            progress: -17,
        }));
        let carrier = DrInterfaceCarrierV1 {
            object: original.cast(),
            vtable: &error_table,
        };
        let original_identity = (carrier.object, carrier.vtable);
        let owned = crate::transfer_group::adapter::OwnedIoError::from_carrier(carrier);
        let mut bytes = [0; 2];
        let (set, group, key) = start_ready::<false>(
            &fixture,
            &table,
            &mut bytes,
            true,
            prepare_ready::<0>,
            prepare_ready_write::<0>,
            Some(owned),
        );
        let mut error = crate::empty_error_carrier();
        let output = fetch(group, 1, 0, 0, &mut error);
        assert_eq!(
            (output.tag, output.allocations, output.key, output.reading),
            (2, 0, key, 1)
        );
        output.fields.assert_success();
        assert_eq!((error.object, error.vtable), original_identity);
        assert_eq!((*original).progress, -17);
        assert_eq!(super::super::tests::observed(group), Some((key, true)));
        super::super::tests::assert_read_region(group, key, Some((bytes.as_ptr(), 1)));
        let rejected = fetch(group, 1, 0, 0, &mut error);
        assert_eq!((rejected.tag, rejected.allocations), (1, 1));
        rejected.fields.assert_failure(
            doria_diagnostic_catalogue::IoMessageOperation::Wait,
            Reason::InvalidInput,
        );
        assert_eq!((error.object, error.vtable), original_identity);
        assert_eq!(super::super::tests::observed(group), Some((key, true)));
        ((*error.vtable).drop)(ptr::null(), error.object);
        error = crate::empty_error_carrier();
        assert_eq!(dropped.get(), 1);
        let exhausted = fetch(group, 1, 0, 0, &mut error);
        assert_eq!(
            (exhausted.tag, exhausted.allocations, exhausted.result),
            (0, 0, 2)
        );
        dr_v1_transfer_adapter_group_drop(ptr::null(), group);
        dr_v1_readiness_set_drop(ptr::null(), set);
        assert_eq!(dropped.get(), 1);
    }
}

#[test]
fn checked_fetch_distinguishes_absent_outcome_from_timeout_and_exhaustion() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        for reading in [true, false] {
            let mut bytes = [0; 2];
            let (set, group, key) = start_ready::<true>(
                &fixture,
                &table,
                &mut bytes,
                reading,
                prepare_ready::<0>,
                prepare_ready_write::<0>,
                None,
            );
            let mut error = crate::empty_error_carrier();
            let output = fetch(group, 1, 0, 0, &mut error);
            assert_eq!(
                (
                    output.tag,
                    output.allocations,
                    output.result,
                    output.outcome,
                    output.count
                ),
                (0, 0, 0, 0, 0)
            );
            assert_eq!((output.key, output.reading), (key, u8::from(reading)));
            assert_eq!(super::super::tests::observed(group), Some((key, reading)));
            assert_eq!(output.control.stop, 2);
            assert_eq!(output.control.transfer.has_transferred_byte_count, 0);
            if reading {
                super::super::tests::assert_read_region(group, key, None);
            }
            output.fields.assert_success();
            let exhausted = fetch(group, 1, 0, 0, &mut error);
            assert_eq!(
                (exhausted.tag, exhausted.allocations, exhausted.result),
                (0, 0, 2)
            );
            dr_v1_transfer_adapter_group_drop(ptr::null(), group);
            dr_v1_readiness_set_drop(ptr::null(), set);
        }
    }
}

unsafe extern "C" fn deferred_read(
    _: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    data: *mut u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    _: *mut DrInterfaceCarrierV1,
) -> u8 {
    let stream = &mut *observed_stream(context);
    stream.calls += 1;
    if stream.pending {
        *outcome = DrAdapterOutcomeV1 {
            tag: 1,
            ..DrAdapterOutcomeV1::default()
        };
    } else {
        assert!(length > 0);
        *data = b'k';
        *outcome = DrAdapterOutcomeV1 {
            tag: 0,
            count: 1,
            has_count: 1,
        };
    }
    0
}

#[test]
fn checked_expired_and_immediate_fetches_leave_transfer_available_for_later_wait() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let set = new_set(false);
        let mut value = fixture.stream(1, &table);
        let stream = value.object.cast::<Stream>();
        (*stream).pending = true;
        let source = source(&value);
        let adapter = DrTransferAdapterV1 {
            context: DrAdapterContextV1 {
                object: value.object.cast(),
                descriptor: ptr::null(),
            },
            read: Some(deferred_read),
            write: None,
        };
        let mut registration = PlanKey { owner: 0, value: 0 };
        let mut status = DrIoStatusV1::default();
        assert_eq!(
            dr_v1_transfer_adapter_set_add_owned(
                ptr::null(),
                set,
                source,
                Some(resolve_source),
                &adapter,
                &mut value,
                ptr::from_mut(&mut registration).cast(),
                &mut status
            ),
            0
        );
        let mut bytes = [0; 2];
        let buffer = DrTransferBufferV1 {
            data: bytes.as_mut_ptr(),
            length: bytes.len(),
            reading: 1,
        };
        let plan = plan::dr_v1_transfer_plan_new(
            ptr::null(),
            ptr::from_ref(&registration).cast(),
            &buffer,
            &mut status,
        );
        assert!(!plan.is_null());
        let key = (*plan.cast::<TransferPlan<'static>>())
            .operation_key(0, true)
            .unwrap();
        let mut error = crate::empty_error_carrier();
        let group = dr_v1_transfer_adapter_plan_begin(
            ptr::null(),
            set,
            plan,
            &DrWaitBudgetV1::indefinite(),
            &mut error,
            &mut status,
        );
        assert!(!group.is_null());
        let calls = (*stream).calls;
        for (kind, low, high) in [(2, (-1_i64) as u64, 0), (3, u64::MAX, -1)] {
            let output = fetch(group, kind, low, high, &mut error);
            assert_eq!((output.tag, output.allocations, output.result), (0, 0, 1));
            assert_eq!(
                (
                    output.key.owner,
                    output.key.position,
                    output.outcome,
                    output.count
                ),
                (0, 0, 0, 0)
            );
            output.fields.assert_success();
            assert_eq!((*stream).calls, calls);
            assert_eq!(super::super::tests::observed(group), None);
            super::super::tests::assert_read_region(group, key, None);
        }
        let output = fetch(group, 0, 0, 0, &mut error);
        assert_eq!((output.tag, output.allocations, output.result), (0, 0, 1));
        output.fields.assert_success();
        assert_eq!(super::super::tests::observed(group), None);
        (*stream).pending = false;
        let completed = fetch(group, 1, 0, 0, &mut error);
        assert_eq!(
            (
                completed.tag,
                completed.allocations,
                completed.result,
                completed.key,
                completed.outcome,
                completed.count
            ),
            (0, 0, 0, key, 1, 1)
        );
        completed.fields.assert_success();
        assert_eq!(bytes, [b'k', 0]);
        super::super::tests::assert_read_region(group, key, Some((bytes.as_ptr(), 1)));
        dr_v1_transfer_adapter_group_drop(ptr::null(), group);
        dr_v1_transfer_adapter_set_drop(ptr::null(), set);
        assert_eq!(*fixture.drops.borrow(), [1]);
    }
}
