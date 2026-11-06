use super::*;
use crate::native_handle::OwnedHandle;
use crate::transfer_group::adapter::DrAdapterContextV1;
use crate::transfer_group::adapter::OwnedIoError;
use crate::transfer_group::pending::{
    dr_v1_pending_request_configure_native, pending_callback_facts,
};
use std::cell::Cell;
use std::io::{Read, Write};
use std::os::fd::IntoRawFd;
use std::os::unix::net::UnixStream;
use std::rc::Rc;

struct Context {
    endpoint: *const c_void,
    prepares: usize,
    completes: usize,
    cleaned: usize,
    last_native_tag: u8,
    completion_frame: usize,
    error: Option<OwnedIoError>,
}

impl Context {
    fn new(endpoint: *const c_void) -> Box<Self> {
        Box::new(Self {
            endpoint,
            prepares: 0,
            completes: 0,
            cleaned: 0,
            last_native_tag: u8::MAX,
            completion_frame: 0,
            error: None,
        })
    }

    fn adapter(&mut self) -> DrPendingTransferAdapterV1 {
        DrPendingTransferAdapterV1 {
            context: DrAdapterContextV1 {
                object: (self as *mut Self).cast(),
                descriptor: core::ptr::null(),
            },
            prepare_read: Some(prepare),
            prepare_write: None,
            complete_read: Some(complete),
            complete_write: None,
        }
    }
}

unsafe extern "C" fn prepare(
    _: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    data: *mut u8,
    length: usize,
    _: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    let context = &mut *(*context).object.cast::<Context>();
    context.prepares += 1;
    if let Some(owned) = context.error.take() {
        error.write(owned.into_carrier());
        return 1;
    }
    dr_v1_pending_request_configure_native(view, context.endpoint, data, length);
    0
}

unsafe extern "C" fn complete(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    data: *mut u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    _: *mut DrInterfaceCarrierV1,
) -> u8 {
    let (tag, submitted, native, selected, immediate) = pending_callback_facts(view);
    let context = &mut *(*context).object.cast::<Context>();
    context.completes += 1;
    context.completion_frame = frame as usize;
    context.last_native_tag = tag;
    assert_eq!((*selected).endpoint, context.endpoint);
    assert_eq!((*selected).data, data);
    assert_eq!((*selected).length, length);
    assert!(immediate.is_null());
    if tag == 4 {
        assert_eq!(submitted, 0);
        assert_eq!((*native).has_system_code, 0);
        assert_eq!((*native).has_transferred_byte_count, 0);
        context.cleaned += 1;
    } else if tag == 0 {
        assert_eq!((tag, submitted), (0, 1));
        let count = (*native).value as usize;
        for byte in core::slice::from_raw_parts_mut(data, count) {
            *byte = byte.to_ascii_uppercase();
        }
        outcome.write(DrAdapterOutcomeV1 {
            tag: 0,
            count,
            has_count: 1,
        });
    } else {
        assert!(matches!(tag, 1 | 2));
        assert_eq!(submitted, 1);
        outcome.write(DrAdapterOutcomeV1 {
            tag,
            count: 0,
            has_count: 0,
        });
    }
    0
}

fn leaf() -> (Box<OwnedHandle>, UnixStream) {
    let (local, peer) = UnixStream::pair().unwrap();
    local.set_nonblocking(true).unwrap();
    (
        Box::new(unsafe { OwnedHandle::from_raw(local.into_raw_fd()) }),
        peer,
    )
}

unsafe fn budget() -> DrWaitBudgetV1 {
    budget_for(0, 0)
}

unsafe fn budget_for(kind: u8, nanoseconds: u64) -> DrWaitBudgetV1 {
    let mut value = mem::MaybeUninit::uninit();
    let (mut reason, mut code, mut has_code) = (0, 0, 0);
    assert_eq!(
        crate::wait_budget::dr_v1_wait_budget_init(
            kind,
            nanoseconds,
            value.as_mut_ptr(),
            &mut reason,
            &mut code,
            &mut has_code
        ),
        0
    );
    value.assume_init()
}

unsafe fn add(set: *mut c_void, context: &mut Context) -> PlanKey {
    let mut status = DrIoStatusV1::default();
    let mut key = PlanKey { owner: 0, value: 0 };
    let adapter = context.adapter();
    assert_eq!(
        dr_v1_pending_transfer_adapter_set_add(
            set,
            context.endpoint,
            &adapter,
            (&mut key as *mut PlanKey).cast(),
            &mut status
        ),
        0
    );
    key
}

unsafe fn plan_for(key: &PlanKey, bytes: &mut [u8]) -> *mut c_void {
    let mut status = DrIoStatusV1::default();
    let buffer = DrTransferBufferV1 {
        data: bytes.as_mut_ptr(),
        length: bytes.len(),
        reading: 1,
    };
    let plan = plan::dr_v1_transfer_plan_new(
        core::ptr::null(),
        (key as *const PlanKey).cast(),
        &buffer,
        &mut status,
    );
    assert!(!plan.is_null());
    plan
}

#[test]
fn pending_ffi_uses_shared_gate_completion_region_reuse_and_sole_leaf_close() {
    unsafe {
        let (leaf, mut peer) = leaf();
        let mut context = Context::new(leaf.native_handle_home());
        let mut status = DrIoStatusV1::default();
        let set =
            dr_v1_pending_transfer_adapter_set_new(core::ptr::null(), ptr::null_mut(), &mut status);
        assert!(!set.is_null());
        let key = add(set, &mut context);
        assert_eq!((context.prepares, context.completes), (0, 0));
        let mut error = crate::empty_error_carrier();
        let unsupported_write = DrTransferBufferV1 {
            data: ptr::null_mut(),
            length: 0,
            reading: 0,
        };
        let invalid = plan::dr_v1_transfer_plan_new(
            ptr::null(),
            ptr::from_ref(&key).cast(),
            &unsupported_write,
            &mut status,
        );
        assert!(!invalid.is_null());
        super::tests::checked_begin(ptr::null(), set, invalid, &mut error, 1);
        assert_eq!((context.prepares, context.completes), (0, 0));
        assert!(!(*set.cast::<AdapterSetOwner>()).active);
        let mut bytes = [0x55; 4];
        peer.write_all(b"ab").unwrap();
        let group = super::tests::checked_begin(
            ptr::null(),
            set,
            plan_for(&key, &mut bytes),
            &mut error,
            0,
        );
        assert!(!group.is_null() && error.object.is_null());
        let mut rejected_bytes = [0x66; 4];
        super::tests::checked_begin(
            ptr::null(),
            set,
            plan_for(&key, &mut rejected_bytes),
            &mut error,
            1,
        );
        assert_eq!((context.prepares, context.completes), (1, 0));
        assert_eq!(rejected_bytes, [0x66; 4]);
        assert!(error.object.is_null() && error.vtable.is_null());
        let mut rejected = PlanKey { owner: 1, value: 2 };
        assert_eq!(
            dr_v1_pending_transfer_adapter_set_add(
                set,
                ptr::dangling(),
                ptr::dangling(),
                (&mut rejected as *mut PlanKey).cast(),
                &mut status
            ),
            3
        );
        let (mut returned, mut index) = (0, usize::MAX);
        let mut outcome = DrAdapterOutcomeV1::default();
        let mut control = DrCompletionStatusV1::default();
        let (tag, allocations) = crate::probe_runtime_allocations(|| {
            dr_v1_transfer_adapter_group_next(
                ptr::null(),
                group,
                &mut returned,
                &mut index,
                &mut outcome,
                &mut error,
                (&mut control as *mut DrCompletionStatusV1).cast(),
            )
        });
        assert_eq!((tag, allocations, outcome.count), (0, 0, 2));
        assert_eq!(bytes, [b'A', b'B', 0x55, 0x55]);
        assert_eq!((returned, index), (key.value, 0));
        peer.write_all(b"cd").unwrap();
        let (tag, allocations) = crate::probe_runtime_allocations(|| {
            assert_eq!(
                dr_v1_transfer_adapter_group_submit(ptr::null(), group, 0, 0, 4, &mut status),
                0
            );
            dr_v1_transfer_adapter_group_next(
                ptr::null(),
                group,
                &mut returned,
                &mut index,
                &mut outcome,
                &mut error,
                (&mut control as *mut DrCompletionStatusV1).cast(),
            )
        });
        assert_eq!((tag, allocations, outcome.count), (0, 0, 2));
        assert_eq!(bytes, [b'C', b'D', 0x55, 0x55]);
        assert_eq!((context.prepares, context.completes), (2, 2));
        let (_, closes) = with_close_probe(|| {
            dr_v1_transfer_adapter_group_drop(ptr::null(), group);
            assert_eq!(
                dr_v1_transfer_adapter_set_remove(
                    set,
                    (&key as *const PlanKey).cast(),
                    &mut status
                ),
                0
            );
        });
        assert_eq!(closes, 0);
        dr_v1_transfer_adapter_set_drop(ptr::null(), set);
        let (_, closes) = with_close_probe(|| drop(leaf));
        assert_eq!(closes, 1);
        let mut byte = [0];
        assert_eq!(peer.read(&mut byte).unwrap(), 0);
    }
}

struct ProgressError {
    drops: Rc<Cell<usize>>,
    progress: Option<i64>,
}

#[test]
fn negative_fetch_budget_keeps_pending_transfer_live_for_a_later_wait() {
    for nanoseconds in [i64::MIN, -250_000_000, -1] {
        unsafe {
            let (leaf, mut peer) = leaf();
            let mut context = Context::new(leaf.native_handle_home());
            let mut status = DrIoStatusV1::default();
            let set = dr_v1_pending_transfer_adapter_set_new(
                core::ptr::null(),
                ptr::null_mut(),
                &mut status,
            );
            assert!(!set.is_null());
            let key = add(set, &mut context);
            let lifetime = DrWaitBudgetV1::indefinite();
            let mut bytes = [0x55; 4];
            let mut error = crate::empty_error_carrier();
            let group = dr_v1_pending_transfer_adapter_plan_begin(
                ptr::null(),
                set,
                plan_for(&key, &mut bytes),
                &lifetime,
                &mut error,
                &mut status,
            );
            assert!(!group.is_null() && error.object.is_null());
            let (mut returned, mut index) = (999, 999);
            let mut outcome = DrAdapterOutcomeV1::default();
            let mut control = DrCompletionStatusV1::default();
            let expired = budget_for(2, nanoseconds as u64);
            let (tag, allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_transfer_adapter_group_observe(
                    ptr::null(),
                    group,
                    &expired,
                    &mut returned,
                    &mut index,
                    &mut outcome,
                    &mut error,
                    ptr::from_mut(&mut control).cast(),
                )
            });
            assert_eq!((tag, allocations), (6, 0));
            assert!(error.object.is_null());
            assert_eq!(
                (context.prepares, context.completes, context.cleaned),
                (1, 0, 0)
            );
            peer.write_all(b"ab").unwrap();
            assert_eq!(
                dr_v1_transfer_adapter_group_observe(
                    ptr::null(),
                    group,
                    &lifetime,
                    &mut returned,
                    &mut index,
                    &mut outcome,
                    &mut error,
                    ptr::from_mut(&mut control).cast(),
                ),
                0
            );
            assert_eq!((returned, index, outcome.count), (key.value, 0, 2));
            assert_eq!(
                (context.prepares, context.completes, context.cleaned),
                (1, 1, 0)
            );
            dr_v1_transfer_adapter_group_drop(ptr::null(), group);
            assert_eq!(bytes, [b'A', b'B', 0x55, 0x55]);
            assert_eq!(
                dr_v1_transfer_adapter_set_remove(set, ptr::from_ref(&key).cast(), &mut status),
                0
            );
            dr_v1_transfer_adapter_set_drop(ptr::null(), set);
        }
    }
}

#[test]
fn pending_fetch_timeout_defers_completion_until_teardown_drains_once() {
    unsafe {
        let acquisition = mem::zeroed::<DrStackFrameV2>();
        let cleanup = mem::zeroed::<DrStackFrameV2>();
        let (leaf, mut peer) = leaf();
        let mut context = Context::new(leaf.native_handle_home());
        let mut status = DrIoStatusV1::default();
        let set =
            dr_v1_pending_transfer_adapter_set_new(core::ptr::null(), ptr::null_mut(), &mut status);
        assert!(!set.is_null());
        let key = add(set, &mut context);
        let lifetime = DrWaitBudgetV1::indefinite();
        let mut bytes = [0x55; 4];
        let mut error = crate::empty_error_carrier();
        let group = dr_v1_pending_transfer_adapter_plan_begin(
            &acquisition,
            set,
            plan_for(&key, &mut bytes),
            &lifetime,
            &mut error,
            &mut status,
        );
        assert!(!group.is_null());
        let (mut returned, mut index) = (999, 999);
        let mut outcome = DrAdapterOutcomeV1::default();
        let mut control = DrCompletionStatusV1::default();
        assert_eq!(
            dr_v1_transfer_adapter_group_observe(
                ptr::null(),
                group,
                &budget(),
                &mut returned,
                &mut index,
                &mut outcome,
                &mut error,
                ptr::from_mut(&mut control).cast()
            ),
            6
        );
        assert_eq!(
            (context.prepares, context.completes, context.cleaned),
            (1, 0, 0)
        );
        dr_v1_transfer_adapter_group_drop(&cleanup, group);
        assert_eq!(context.completion_frame, ptr::from_ref(&cleanup) as usize);
        assert_eq!(
            (
                context.prepares,
                context.completes,
                context.cleaned,
                context.last_native_tag
            ),
            (1, 1, 1, 4)
        );
        assert_eq!(bytes, [0x55; 4]);
        peer.write_all(b"ab").unwrap();
        let group = dr_v1_pending_transfer_adapter_plan_begin(
            ptr::null(),
            set,
            plan_for(&key, &mut bytes),
            &lifetime,
            &mut error,
            &mut status,
        );
        assert!(!group.is_null());
        assert_eq!(
            dr_v1_transfer_adapter_group_observe(
                ptr::null(),
                group,
                &lifetime,
                &mut returned,
                &mut index,
                &mut outcome,
                &mut error,
                ptr::from_mut(&mut control).cast()
            ),
            0
        );
        assert_eq!((returned, index, outcome.count), (key.value, 0, 2));
        assert_eq!(
            (context.prepares, context.completes, context.cleaned),
            (2, 2, 1)
        );
        dr_v1_transfer_adapter_group_drop(ptr::null(), group);
        assert_eq!(
            dr_v1_transfer_adapter_set_remove(set, ptr::from_ref(&key).cast(), &mut status),
            0
        );
        dr_v1_transfer_adapter_set_drop(ptr::null(), set);
    }
}

unsafe extern "C" fn drop_progress_error(_: *const DrStackFrameV2, object: *mut u8) {
    let payload = Box::from_raw(object.cast::<ProgressError>());
    payload.drops.set(payload.drops.get() + 1);
}

unsafe extern "C" fn complete_with_original_error(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    data: *mut u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    complete(frame, context, view, data, length, outcome, error);
    let Some(owned) = (*(*context).object.cast::<Context>()).error.take() else {
        return 0;
    };
    let carrier = owned.into_carrier();
    let progress = (*carrier.object.cast::<ProgressError>()).progress;
    *outcome = DrAdapterOutcomeV1::default();
    if let Some(count) = progress.and_then(|count| usize::try_from(count).ok()) {
        (*outcome).count = count;
        (*outcome).has_count = 1;
    }
    error.write(carrier);
    1
}

#[test]
fn pending_fetch_expiry_keeps_preparation_live_then_moves_original_error_once() {
    unsafe {
        for progress in [None, Some(0), Some(2), Some(-3)] {
            let (leaf, mut peer) = leaf();
            let mut context = Context::new(leaf.native_handle_home());
            let mut status = DrIoStatusV1::default();
            let set = dr_v1_pending_transfer_adapter_set_new(
                core::ptr::null(),
                ptr::null_mut(),
                &mut status,
            );
            assert!(!set.is_null());
            let adapter = DrPendingTransferAdapterV1 {
                complete_read: Some(complete_with_original_error),
                ..context.adapter()
            };
            let mut key = PlanKey { owner: 0, value: 0 };
            assert_eq!(
                dr_v1_pending_transfer_adapter_set_add(
                    set,
                    context.endpoint,
                    &adapter,
                    ptr::from_mut(&mut key).cast(),
                    &mut status
                ),
                0
            );
            let lifetime = DrWaitBudgetV1::indefinite();
            let mut bytes = [0x55; 4];
            let mut error = crate::empty_error_carrier();
            let plan = plan_for(&key, &mut bytes);
            let read_key = (*plan.cast::<TransferPlan<'static>>())
                .operation_key(0, true)
                .unwrap();
            let group = dr_v1_pending_transfer_adapter_plan_begin(
                ptr::null(),
                set,
                plan,
                &lifetime,
                &mut error,
                &mut status,
            );
            assert!(!group.is_null());
            assert_eq!((context.prepares, context.completes), (1, 0));
            super::tests::assert_read_region(group, read_key, None);
            let (mut returned, mut index) = (999, 999);
            let mut outcome = DrAdapterOutcomeV1::default();
            let mut control = DrCompletionStatusV1::default();
            for wait in [
                budget(),
                budget_for(3, 0),
                budget_for(2, 0),
                budget_for(2, (-1_i64) as u64),
                budget_for(2, (-250_000_000_i64) as u64),
                budget_for(2, i64::MIN as u64),
            ] {
                let (tag, allocations) = crate::probe_runtime_allocations(|| {
                    dr_v1_transfer_adapter_group_observe(
                        ptr::null(),
                        group,
                        &wait,
                        &mut returned,
                        &mut index,
                        &mut outcome,
                        &mut error,
                        ptr::from_mut(&mut control).cast(),
                    )
                });
                assert_eq!((tag, allocations), (6, 0));
                assert_eq!(
                    (returned, index, outcome.has_count, control.stop),
                    (0, 0, 0, 0)
                );
                assert_eq!(
                    (context.prepares, context.completes, context.cleaned),
                    (1, 0, 0)
                );
                assert!(error.object.is_null());
                assert_eq!(super::tests::observed(group), None);
                super::tests::assert_read_region(group, read_key, None);
                assert_eq!(
                    dr_v1_transfer_adapter_set_remove(set, ptr::from_ref(&key).cast(), &mut status),
                    3
                );
            }
            let drops = Rc::new(Cell::new(0));
            let object = Box::into_raw(Box::new(ProgressError {
                drops: Rc::clone(&drops),
                progress,
            }));
            let descriptor = crate::DrInterfaceVtableV1 {
                type_name: crate::IO_ERROR_TYPE.as_ptr(),
                type_name_length: crate::IO_ERROR_TYPE.len(),
                message_offset: 0,
                drop: drop_progress_error,
                class_size: mem::size_of::<ProgressError>(),
                origin_offset: 0,
                assertion_offsets: 0,
                assertion_magic: 0,
                class_descriptor: ptr::null(),
                interface_id: 0,
                io_transferred_byte_count_offset: 0,
                io_progress_magic: 0,
            };
            context.error = Some(OwnedIoError::from_carrier(DrInterfaceCarrierV1 {
                object: object.cast(),
                vtable: &descriptor,
            }));
            peer.write_all(b"ab").unwrap();
            let (tag, allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_transfer_adapter_group_observe(
                    ptr::null(),
                    group,
                    &lifetime,
                    &mut returned,
                    &mut index,
                    &mut outcome,
                    &mut error,
                    ptr::from_mut(&mut control).cast(),
                )
            });
            assert_eq!((tag, allocations), (1, 0));
            assert_eq!((returned, index), (key.value, 0));
            assert_eq!(
                (context.prepares, context.completes, context.cleaned),
                (1, 1, 0)
            );
            assert_eq!(error.object, object.cast());
            assert_eq!(error.vtable, ptr::from_ref(&descriptor));
            assert_eq!((*object).progress, progress);
            assert_eq!(drops.get(), 0);
            assert_eq!(super::tests::observed(group), Some((read_key, true)));
            super::tests::assert_read_region(
                group,
                read_key,
                progress
                    .and_then(|count| usize::try_from(count).ok())
                    .map(|count| (bytes.as_ptr(), count)),
            );
            assert_eq!(
                dr_v1_transfer_adapter_group_observe(
                    ptr::null(),
                    group,
                    &lifetime,
                    &mut returned,
                    &mut index,
                    &mut outcome,
                    &mut error,
                    ptr::from_mut(&mut control).cast()
                ),
                3
            );
            assert_eq!(error.object, object.cast());
            assert_eq!(super::tests::observed(group), Some((read_key, true)));
            for (key, reading) in [
                (read_key, 0),
                (
                    crate::transfer_group::plan::OperationKey {
                        owner: 0,
                        ..read_key
                    },
                    1,
                ),
                (
                    crate::transfer_group::plan::OperationKey {
                        position: 1,
                        ..read_key
                    },
                    1,
                ),
            ] {
                let (tag, allocations) = crate::probe_runtime_allocations(|| {
                    dr_v1_transfer_adapter_group_submit_key(
                        ptr::null(),
                        group,
                        ptr::from_ref(&key).cast(),
                        reading,
                        0,
                        4,
                        &mut status,
                    )
                });
                assert_eq!((tag, allocations), (3, 0));
                assert_eq!(status.failure(), Some(Failure::invalid_input()));
                crate::transfer_group::transport::tests::assert_group_configuration_failure(
                    &status,
                );
                super::tests::assert_checked_resubmit(group, key, reading, 0, 4, false);
                assert_eq!(
                    (context.prepares, context.completes, context.cleaned),
                    (1, 1, 0)
                );
                assert_eq!(error.object, object.cast());
                assert_eq!(error.vtable, ptr::from_ref(&descriptor));
                assert_eq!((*object).progress, progress);
                assert_eq!(drops.get(), 0);
                assert_eq!(super::tests::observed(group), Some((read_key, true)));
            }
            drop(OwnedIoError::from_carrier(error));
            error = crate::empty_error_carrier();
            assert_eq!(drops.get(), 1);
            assert_eq!(
                dr_v1_transfer_adapter_group_observe(
                    ptr::null(),
                    group,
                    &budget(),
                    &mut returned,
                    &mut index,
                    &mut outcome,
                    &mut error,
                    ptr::from_mut(&mut control).cast()
                ),
                5
            );
            super::tests::assert_checked_resubmit(group, read_key, 1, 1, 3, true);
            super::tests::assert_read_region(group, read_key, None);
            assert_eq!(
                dr_v1_transfer_adapter_group_observe(
                    ptr::null(),
                    group,
                    &budget(),
                    &mut returned,
                    &mut index,
                    &mut outcome,
                    &mut error,
                    ptr::from_mut(&mut control).cast()
                ),
                6
            );
            assert_eq!((context.prepares, context.completes), (2, 1));
            super::tests::assert_read_region(group, read_key, None);
            peer.write_all(b"cd").unwrap();
            assert_eq!(
                dr_v1_transfer_adapter_group_observe(
                    ptr::null(),
                    group,
                    &lifetime,
                    &mut returned,
                    &mut index,
                    &mut outcome,
                    &mut error,
                    ptr::from_mut(&mut control).cast()
                ),
                0
            );
            assert_eq!(
                (returned, index, outcome.count, control.stop),
                (key.value, 0, 2, 0)
            );
            super::tests::assert_read_region(group, read_key, Some((bytes[1..].as_ptr(), 2)));
            assert_eq!(
                (context.prepares, context.completes, context.cleaned),
                (2, 2, 0)
            );
            dr_v1_transfer_adapter_group_drop(ptr::null(), group);
            assert_eq!(drops.get(), 1);
            assert_eq!(&bytes, b"ACDU");
            assert_eq!(
                dr_v1_transfer_adapter_set_remove(set, ptr::from_ref(&key).cast(), &mut status),
                0
            );
            dr_v1_transfer_adapter_set_drop(ptr::null(), set);
        }
    }
}

#[test]
fn pending_ffi_failed_setup_preserves_original_error_and_releases_shared_active_gate() {
    for (transport, progress) in [
        (0, None),
        (1, None),
        (2, None),
        (2, Some(0)),
        (2, Some(2)),
        (2, Some(-3)),
    ] {
        unsafe {
            let acquisition = mem::zeroed::<DrStackFrameV2>();
            let (first_leaf, mut first_peer) = leaf();
            let (second_leaf, mut second_peer) = leaf();
            let mut first = Context::new(first_leaf.native_handle_home());
            let mut second = Context::new(second_leaf.native_handle_home());
            let drops = Rc::new(Cell::new(0));
            let object = Box::into_raw(Box::new(ProgressError {
                drops: Rc::clone(&drops),
                progress,
            }))
            .cast::<u8>();
            let descriptor = crate::DrInterfaceVtableV1 {
                type_name: crate::IO_ERROR_TYPE.as_ptr(),
                type_name_length: crate::IO_ERROR_TYPE.len(),
                message_offset: 0,
                drop: drop_progress_error,
                class_size: mem::size_of::<ProgressError>(),
                origin_offset: 0,
                assertion_offsets: 0,
                assertion_magic: 0,
                class_descriptor: ptr::null(),
                interface_id: 0,
                io_transferred_byte_count_offset: 0,
                io_progress_magic: 0,
            };
            second.error = Some(OwnedIoError::from_carrier(DrInterfaceCarrierV1 {
                object,
                vtable: &descriptor,
            }));
            let mut status = DrIoStatusV1::default();
            let set = dr_v1_pending_transfer_adapter_set_new(
                core::ptr::null(),
                ptr::null_mut(),
                &mut status,
            );
            let a = add(set, &mut first);
            let b = add(set, &mut second);
            first_peer.write_all(b"a").unwrap();
            second_peer.write_all(b"b").unwrap();
            let (mut a_bytes, mut b_bytes) = ([0; 1], [0; 1]);
            let first_plan = plan_for(&a, &mut a_bytes);
            let buffer = DrTransferBufferV1 {
                data: b_bytes.as_mut_ptr(),
                length: 1,
                reading: 1,
            };
            let plan = plan::dr_v1_transfer_plan_append(
                core::ptr::null(),
                first_plan,
                (&b as *const PlanKey).cast(),
                &buffer,
                &mut status,
            );
            assert!(!plan.is_null());
            let mut error = crate::empty_error_carrier();
            let group = if transport == 2 {
                super::tests::checked_begin(&acquisition, set, plan, &mut error, 2)
            } else if transport == 1 {
                let result = begin_adapter_plan(&acquisition, set, plan, &budget(), &error, true);
                assert!(matches!(
                    &result,
                    Err(SetupFailure::Preparation(AdapterTerminal::CheckedError {
                        progress: None,
                        ..
                    }))
                ));
                assert!(error.object.is_null() && error.vtable.is_null());
                assert_eq!(drops.get(), 0);
                project_plan_begin(result, &mut error, &mut status)
            } else {
                dr_v1_pending_transfer_adapter_plan_begin(
                    &acquisition,
                    set,
                    plan,
                    &budget(),
                    &mut error,
                    &mut status,
                )
            };
            assert!(group.is_null());
            assert_eq!(error.object, object);
            assert_eq!(error.vtable, &descriptor as *const _);
            assert_eq!((*object.cast::<ProgressError>()).progress, progress);
            assert_eq!((status.reason, status.has_system_code), (0, 0));
            assert_eq!((first.prepares, first.completes, first.cleaned), (1, 1, 1));
            assert_eq!(first.completion_frame, ptr::from_ref(&acquisition) as usize);
            assert_eq!((second.prepares, second.completes), (1, 0));
            assert_eq!((a_bytes, b_bytes), ([0], [0]));
            // If either request had reached submission, these queued native bytes
            // would already have been consumed. No source-level substitute is used.
            assert!(matches!(
                crate::byte_io::read(first_leaf.get(), a_bytes.as_mut_ptr(), 1),
                Ok(crate::byte_io::ReadProgress::Data(1))
            ));
            assert_eq!(a_bytes, [b'a']);
            assert!(matches!(
                crate::byte_io::read(second_leaf.get(), b_bytes.as_mut_ptr(), 1),
                Ok(crate::byte_io::ReadProgress::Data(1))
            ));
            assert_eq!(b_bytes, [b'b']);
            assert_eq!(
                dr_v1_transfer_adapter_set_remove(set, (&a as *const PlanKey).cast(), &mut status),
                0
            );
            assert_eq!(
                dr_v1_transfer_adapter_set_remove(set, (&b as *const PlanKey).cast(), &mut status),
                0
            );
            assert_eq!(drops.get(), 0);
            drop(OwnedIoError::from_carrier(error));
            assert_eq!(drops.get(), 1);
            dr_v1_transfer_adapter_set_drop(ptr::null(), set);
        }
    }
}

#[test]
fn pending_file_ffi_preserves_typed_eof_reuse_identity_and_sole_close() {
    use crate::file_io::stream::{
        FileAccess, FileCreation, FileOpenRequest, FileSharing, OwnedFile,
    };
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_file(self.0.join("leaf.bin")).unwrap();
            std::fs::remove_dir(&self.0).unwrap();
        }
    }
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "doria-pending-file-ffi-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    )));
    std::fs::create_dir(&fixture.0).unwrap();
    let path = fixture.0.join("leaf.bin");
    std::fs::write(&path, b"ab").unwrap();
    let mut file = Box::new(
        OwnedFile::open(
            path.to_str().unwrap().as_bytes(),
            FileOpenRequest {
                access: FileAccess::Read,
                creation: FileCreation::OpenExisting,
                append: false,
                sharing: FileSharing::ReadWrite,
            },
        )
        .unwrap(),
    );
    unsafe {
        let endpoint = ptr::from_mut(file.as_mut()).cast::<c_void>();
        let mut context = Context::new(endpoint);
        let adapter = context.adapter();
        let mut status = DrIoStatusV1::default();
        let set =
            dr_v1_pending_transfer_adapter_set_new(core::ptr::null(), ptr::null_mut(), &mut status);
        assert!(!set.is_null());
        let mut key = PlanKey { owner: 0, value: 0 };
        assert_eq!(
            dr_v1_pending_transfer_adapter_set_add_file(
                set,
                endpoint,
                &adapter,
                (&mut key as *mut PlanKey).cast(),
                &mut status,
            ),
            0
        );
        assert_eq!((context.prepares, context.completes), (0, 0));
        let mut duplicate = PlanKey { owner: 1, value: 2 };
        assert_eq!(
            dr_v1_pending_transfer_adapter_set_add_file(
                set,
                endpoint,
                &adapter,
                (&mut duplicate as *mut PlanKey).cast(),
                &mut status,
            ),
            3
        );
        assert_eq!((duplicate.owner, duplicate.value), (0, 0));
        let mut bytes = [0; 2];
        let mut error = crate::empty_error_carrier();
        let budget = budget();
        let group = dr_v1_pending_transfer_adapter_plan_begin(
            ptr::null(),
            set,
            plan_for(&key, &mut bytes),
            &budget,
            &mut error,
            &mut status,
        );
        assert!(!group.is_null() && error.object.is_null());
        let (mut returned, mut index) = (0, usize::MAX);
        let mut outcome = DrAdapterOutcomeV1::default();
        let mut control = DrCompletionStatusV1::default();
        let ((), allocations) = crate::probe_runtime_allocations(|| {
            assert_eq!(
                dr_v1_transfer_adapter_group_next(
                    ptr::null(),
                    group,
                    &mut returned,
                    &mut index,
                    &mut outcome,
                    &mut error,
                    (&mut control as *mut DrCompletionStatusV1).cast(),
                ),
                0
            );
            assert_eq!(
                (outcome.tag, outcome.count, context.last_native_tag),
                (0, 2, 0)
            );
            assert_eq!((returned, index), (key.value, 0));
            assert_eq!(
                dr_v1_transfer_adapter_group_submit(ptr::null(), group, 0, 0, 0, &mut status,),
                0
            );
            assert_eq!(
                dr_v1_transfer_adapter_group_next(
                    ptr::null(),
                    group,
                    &mut returned,
                    &mut index,
                    &mut outcome,
                    &mut error,
                    (&mut control as *mut DrCompletionStatusV1).cast(),
                ),
                0
            );
            assert_eq!(
                (outcome.tag, outcome.count, context.last_native_tag),
                (0, 0, 0)
            );
            assert_eq!(
                dr_v1_transfer_adapter_group_submit(ptr::null(), group, 0, 0, 2, &mut status,),
                0
            );
            assert_eq!(
                dr_v1_transfer_adapter_group_next(
                    ptr::null(),
                    group,
                    &mut returned,
                    &mut index,
                    &mut outcome,
                    &mut error,
                    (&mut control as *mut DrCompletionStatusV1).cast(),
                ),
                0
            );
            assert_eq!(
                (outcome.tag, outcome.has_count, context.last_native_tag),
                (2, 0, 2)
            );
        });
        assert_eq!(allocations, 0);
        assert_eq!((context.prepares, context.completes), (3, 3));
        let ((), closes) = with_close_probe(|| {
            dr_v1_transfer_adapter_group_drop(ptr::null(), group);
            assert_eq!(
                dr_v1_transfer_adapter_set_remove(
                    set,
                    (&key as *const PlanKey).cast(),
                    &mut status,
                ),
                0
            );
        });
        assert_eq!(closes, 0);
        dr_v1_transfer_adapter_set_drop(ptr::null(), set);
        assert_eq!(file.position().unwrap(), 2);
        assert_eq!(bytes, *b"AB");
        let ((), closes) = with_close_probe(|| drop(file));
        assert_eq!(closes, 1);
    }
}
