use super::*;
use crate::file_io::stream::transport::*;
use crate::file_io::stream::OwnedFile;
use crate::native_handle::with_close_probe;
use crate::transfer_group::adapter::test_object::TestObject;
use crate::transfer_group::adapter::DrAdapterContextV1;
use std::fs;

struct Metered {
    file: *mut c_void,
    calls: usize,
    frame: *const DrStackFrameV2,
    zero_data: bool,
    error: Option<crate::transfer_group::adapter::OwnedIoError>,
}

unsafe extern "C" fn transformed_read(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    data: *mut u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    let context = TestObject::<Metered>::in_callback((*context).object);
    context.calls += 1;
    context.frame = frame;
    *outcome = DrAdapterOutcomeV1::default();
    if let Some(owned) = context.error.take() {
        (*outcome).count = 1;
        (*outcome).has_count = 1;
        *error = owned.into_carrier();
        return 1;
    }
    if context.zero_data || context.calls == 3 {
        (*outcome).tag = 0;
        (*outcome).has_count = 1;
        return 0;
    }
    let mut status = DrIoStatusV1::default();
    let tag = dr_v1_file_read(context.file, data, length.min(2), &mut status);
    assert!(tag == 0 || tag == 2, "fixture file transfer must succeed");
    (*outcome).tag = tag;
    if tag == 0 {
        (*outcome).count = status.value as usize;
        (*outcome).has_count = 1;
        for byte in core::slice::from_raw_parts_mut(data, status.value as usize) {
            *byte = byte.to_ascii_uppercase();
        }
    }
    0
}

unsafe extern "C" fn transformed_write(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    data: *const u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    assert!((*(*context).object.cast::<Metered>()).error.is_some());
    transformed_read(frame, context, data.cast_mut(), length, outcome, error)
}

pub(super) unsafe fn observed(
    group: *const c_void,
) -> Option<(crate::transfer_group::plan::OperationKey, bool)> {
    use crate::transfer_group::plan::OperationKey;
    let mut result = None;
    for reading in [false, true] {
        let mut key = OperationKey {
            owner: u64::MAX,
            position: u64::MAX,
        };
        let (tag, allocations) = crate::probe_runtime_allocations(|| {
            dr_v1_transfer_adapter_group_observed(
                group,
                u8::from(reading),
                ptr::from_mut(&mut key).cast(),
            )
        });
        assert_eq!(allocations, 0);
        match tag {
            0 => assert!(
                result.replace((key, reading)).is_none(),
                "only one direction may be observed"
            ),
            1 => assert_eq!(
                key,
                OperationKey {
                    owner: 0,
                    position: 0
                }
            ),
            _ => panic!("invalid observation status"),
        }
    }
    result
}

pub(super) unsafe fn assert_read_region(
    group: *const c_void,
    key: crate::transfer_group::plan::OperationKey,
    expected: Option<(*const u8, usize)>,
) {
    let mut data = ptr::dangling();
    let mut length = usize::MAX;
    let mut status = DrIoStatusV1 {
        reason: u8::MAX,
        ..DrIoStatusV1::default()
    };
    let (tag, allocations) = crate::probe_runtime_allocations(|| {
        dr_v1_transfer_adapter_group_read_region(
            group,
            ptr::from_ref(&key).cast(),
            &mut data,
            &mut length,
            &mut status,
        )
    });
    assert_eq!(allocations, 0);
    match expected {
        Some(region) => {
            assert_eq!(tag, 0);
            assert!(!data.is_null(), "known zero-byte completion is present");
            assert_eq!((data, length), region);
        }
        None => assert_eq!((tag, data, length), (1, ptr::null(), 0)),
    }
    assert_eq!(
        (
            status.reason,
            status.value,
            status.has_system_code,
            status.has_transferred_byte_count
        ),
        (0, 0, 0, 0)
    );
    assert_checked_read_region(group, key, Ok(expected));
}

struct CheckedGroupFields {
    message: *mut DrStringV1,
    path: *mut DrStringV1,
    code: i64,
    progress: usize,
    meta: u64,
}

impl CheckedGroupFields {
    fn new() -> Self {
        Self {
            message: ptr::null_mut(),
            path: ptr::null_mut(),
            code: -1,
            progress: usize::MAX,
            meta: u64::MAX,
        }
    }

    unsafe fn assert_result(&self, operation: Option<u8>) {
        assert!(self.path.is_null());
        assert_eq!((self.code, self.progress), (0, 0));
        match operation {
            None => {
                assert!(self.message.is_null());
                assert_eq!(self.meta, 0);
            }
            Some(operation) => {
                assert!(!self.message.is_null());
                assert_eq!(
                    self.meta,
                    operation as u64 | (6 << 8) | ((Reason::InvalidInput as u64) << 16)
                );
                let expected = if operation == crate::checked_io::OP_ACCESS {
                    b"failed to access transfer group: invalid input".as_slice()
                } else {
                    b"failed to configure transfer group: invalid input".as_slice()
                };
                assert_eq!(
                    slice::from_raw_parts(
                        crate::string_bytes(self.message),
                        (*self.message).byte_length
                    ),
                    expected
                );
                crate::dr_v1_string_release(self.message);
            }
        }
    }
}

unsafe fn assert_checked_read_region(
    group: *const c_void,
    key: crate::transfer_group::plan::OperationKey,
    expected: Result<Option<(*const u8, usize)>, ()>,
) {
    let (mut data, mut length, mut present) = (ptr::dangling(), usize::MAX, u8::MAX);
    let mut fields = CheckedGroupFields::new();
    let (tag, allocations) = crate::probe_runtime_allocations(|| {
        super::super::checked_group::dr_v1_checked_transfer_group_read_region(
            ptr::null(),
            group,
            ptr::from_ref(&key).cast(),
            &mut data,
            &mut length,
            &mut present,
            &mut fields.message,
            &mut fields.path,
            &mut fields.code,
            &mut fields.progress,
            &mut fields.meta,
        )
    });
    match expected {
        Ok(region) => {
            assert_eq!((tag, allocations), (0, 0));
            assert_eq!(present, u8::from(region.is_some()));
            assert_eq!((data, length), region.unwrap_or((ptr::null(), 0)));
            fields.assert_result(None);
        }
        Err(()) => {
            assert_eq!(
                (tag, allocations, present, data, length),
                (1, 1, 0, ptr::null(), 0)
            );
            fields.assert_result(Some(crate::checked_io::OP_ACCESS));
        }
    }
}

pub(super) unsafe fn assert_checked_resubmit(
    group: *mut c_void,
    key: crate::transfer_group::plan::OperationKey,
    reading: u8,
    offset: i64,
    length: i64,
    success: bool,
) {
    let mut fields = CheckedGroupFields::new();
    let (tag, allocations) = crate::probe_runtime_allocations(|| {
        super::super::checked_group::dr_v1_checked_transfer_group_resubmit(
            ptr::null(),
            group,
            ptr::from_ref(&key).cast(),
            reading,
            offset,
            length,
            &mut fields.message,
            &mut fields.path,
            &mut fields.code,
            &mut fields.progress,
            &mut fields.meta,
        )
    });
    assert_eq!((tag, allocations), if success { (0, 0) } else { (1, 1) });
    fields.assert_result(
        (!success).then_some(doria_diagnostic_catalogue::IoMessageOperation::Configure as u8),
    );
}

pub(super) unsafe fn checked_begin(
    frame: *const DrStackFrameV2,
    set: *mut c_void,
    plan: *mut c_void,
    error: *mut DrInterfaceCarrierV1,
    expected: u8,
) -> *mut c_void {
    let mut group = ptr::dangling_mut();
    let mut fields = CheckedGroupFields::new();
    let tag = super::super::checked_group::dr_v1_checked_transfer_group_begin(
        frame,
        set,
        plan,
        &mut group,
        error,
        &mut fields.message,
        &mut fields.path,
        &mut fields.code,
        &mut fields.progress,
        &mut fields.meta,
    );
    assert_eq!(tag, expected);
    assert_eq!(group.is_null(), tag != 0);
    fields.assert_result(
        (tag == 1).then_some(doria_diagnostic_catalogue::IoMessageOperation::Configure as u8),
    );
    group
}

#[test]
fn typed_group_setup_rejects_invalid_configuration_before_callbacks() {
    unsafe {
        let mut status = DrIoStatusV1::default();
        let set = dr_v1_transfer_adapter_set_new(ptr::null(), ptr::null_mut(), &mut status);
        assert!(!set.is_null());
        let key = PlanKey {
            owner: (*set.cast::<AdapterSetOwner>()).identity,
            value: 999,
        };
        let buffer = DrTransferBufferV1 {
            data: ptr::null_mut(),
            length: 0,
            reading: 1,
        };
        let budget = DrWaitBudgetV1::indefinite();
        for pending in [false, true] {
            let plan = plan::dr_v1_transfer_plan_new(
                ptr::null(),
                ptr::from_ref(&key).cast(),
                &buffer,
                &mut status,
            );
            assert!(!plan.is_null());
            let mut error = crate::empty_error_carrier();
            let result = begin_adapter_plan(ptr::null(), set, plan, &budget, &error, pending);
            assert!(matches!(
                &result,
                Err(SetupFailure::Platform(failure)) if *failure == Failure::invalid_input()
            ));
            assert!(!(*set.cast::<AdapterSetOwner>()).active);
            assert!(error.object.is_null() && error.vtable.is_null());
            assert!(project_plan_begin(result, &mut error, &mut status).is_null());
            assert_eq!(status.failure(), Some(Failure::invalid_input()));
        }
        dr_v1_transfer_adapter_set_drop(ptr::null(), set);
    }
}

#[test]
fn checked_group_setup_rejects_null_foreign_missing_and_nonempty_inputs() {
    unsafe {
        let mut status = DrIoStatusV1::default();
        let set = dr_v1_pending_transfer_adapter_set_new(ptr::null(), ptr::null_mut(), &mut status);
        assert!(!set.is_null());
        let identity = (*set.cast::<AdapterSetOwner>()).identity;
        for (target, key_owner, nonempty) in [
            (ptr::null_mut(), identity, false),
            (set, identity, false),
            (set, identity + 1, false),
            (set, identity, true),
        ] {
            let key = PlanKey {
                owner: key_owner,
                value: 999,
            };
            let buffer = DrTransferBufferV1 {
                data: ptr::null_mut(),
                length: 0,
                reading: 1,
            };
            let plan = plan::dr_v1_transfer_plan_new(
                ptr::null(),
                ptr::from_ref(&key).cast(),
                &buffer,
                &mut status,
            );
            assert!(!plan.is_null());
            let mut error = crate::empty_error_carrier();
            if nonempty {
                error.object = ptr::dangling_mut();
                error.vtable = ptr::dangling();
            }
            let original = (error.object, error.vtable);
            checked_begin(ptr::null(), target, plan, &mut error, 1);
            assert_eq!((error.object, error.vtable), original);
            assert!(!(*set.cast::<AdapterSetOwner>()).active);
        }
        let mut error = crate::empty_error_carrier();
        checked_begin(ptr::null(), set, ptr::null_mut(), &mut error, 1);
        dr_v1_transfer_adapter_set_drop(ptr::null(), set);
    }
}

#[test]
fn checked_group_bridges_reject_a_missing_owner_without_accessing_storage() {
    let key = crate::transfer_group::plan::OperationKey {
        owner: 1,
        position: 0,
    };
    unsafe {
        assert_checked_read_region(ptr::null(), key, Err(()));
        assert_checked_resubmit(ptr::null_mut(), key, 1, 0, 0, false);
    }
}

#[test]
fn adapter_ffi_dispatches_the_complete_receiver_and_never_claims_its_file() {
    unsafe {
        let directory = std::env::temp_dir().join(format!(
            "doria-adapter-ffi-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("source.bin");
        fs::write(&path, b"ab").unwrap();
        let bytes = path.to_str().unwrap().as_bytes();
        let mut file = Box::new(mem::MaybeUninit::<OwnedFile>::uninit());
        let file_pointer = file.as_mut_ptr().cast::<c_void>();
        let mut status = DrIoStatusV1::default();
        assert_eq!(
            dr_v1_file_open(
                bytes.as_ptr(),
                bytes.len(),
                0,
                0,
                0,
                file_pointer,
                &mut status
            ),
            0
        );
        let home = dr_v1_file_readiness_source_home(file_pointer);
        let source = *home.cast::<*const c_void>();
        assert_eq!(source, dr_v1_file_readiness_source(file_pointer));
        let receiver = TestObject::new(Metered {
            file: file_pointer,
            calls: 0,
            frame: ptr::null(),
            zero_data: false,
            error: None,
        });
        let adapter = DrTransferAdapterV1 {
            context: receiver.context(),
            read: Some(transformed_read),
            write: Some(transformed_write),
        };
        let set = dr_v1_transfer_adapter_set_new(core::ptr::null(), ptr::null_mut(), &mut status);
        assert!(!set.is_null());
        let mut key = PlanKey { owner: 0, value: 0 };
        assert_eq!(
            dr_v1_transfer_adapter_set_add(
                set,
                source,
                &adapter,
                (&mut key as *mut PlanKey).cast(),
                &mut status
            ),
            0
        );
        assert_eq!(
            (*receiver.as_ptr()).calls,
            0,
            "source admission must not transfer bytes"
        );
        let readonly_adapter = DrTransferAdapterV1 {
            write: None,
            ..adapter
        };
        let readonly_set =
            dr_v1_transfer_adapter_set_new(ptr::null(), ptr::null_mut(), &mut status);
        assert!(!readonly_set.is_null());
        let mut readonly_key = PlanKey { owner: 0, value: 0 };
        assert_eq!(
            dr_v1_transfer_adapter_set_add(
                readonly_set,
                source,
                &readonly_adapter,
                ptr::from_mut(&mut readonly_key).cast(),
                &mut status,
            ),
            0
        );
        let write = DrTransferBufferV1 {
            data: ptr::null_mut(),
            length: 0,
            reading: 0,
        };
        let invalid = plan::dr_v1_transfer_plan_new(
            ptr::null(),
            ptr::from_ref(&readonly_key).cast(),
            &write,
            &mut status,
        );
        assert!(!invalid.is_null());
        let mut setup_error = crate::empty_error_carrier();
        let rejected = begin_adapter_plan(
            ptr::null(),
            readonly_set,
            invalid,
            &DrWaitBudgetV1::indefinite(),
            &setup_error,
            false,
        );
        assert!(matches!(
            &rejected,
            Err(SetupFailure::Platform(failure))
                if *failure == Failure::invalid_input()
        ));
        assert!(!(*readonly_set.cast::<AdapterSetOwner>()).active);
        assert!(project_plan_begin(rejected, &mut setup_error, &mut status).is_null());
        assert_eq!(status.failure(), Some(Failure::invalid_input()));
        assert!(setup_error.object.is_null() && setup_error.vtable.is_null());
        assert_eq!((*receiver.as_ptr()).calls, 0);
        assert_eq!(
            dr_v1_transfer_adapter_set_remove(
                readonly_set,
                ptr::from_ref(&readonly_key).cast(),
                &mut status,
            ),
            0
        );
        dr_v1_transfer_adapter_set_drop(ptr::null(), readonly_set);
        let mut budget = mem::MaybeUninit::<DrWaitBudgetV1>::uninit();
        let (mut reason, mut code, mut has_code) = (0, 0, 0);
        assert_eq!(
            crate::wait_budget::dr_v1_wait_budget_init(
                0,
                0,
                budget.as_mut_ptr(),
                &mut reason,
                &mut code,
                &mut has_code
            ),
            0
        );
        let budget = budget.assume_init();
        let mut destination = [0x55; 4];
        let buffer = DrTransferBufferV1 {
            data: destination.as_mut_ptr(),
            length: 4,
            reading: 1,
        };
        let plan = plan::dr_v1_transfer_plan_new(
            core::ptr::null(),
            (&key as *const PlanKey).cast(),
            &buffer,
            &mut status,
        );
        let read_key = (*plan.cast::<TransferPlan<'static>>())
            .operation_key(0, true)
            .unwrap();
        let mut setup_error = crate::empty_error_carrier();
        let started = begin_adapter_plan(ptr::null(), set, plan, &budget, &setup_error, false);
        assert!(started.is_ok());
        let group = project_plan_begin(started, &mut setup_error, &mut status);
        assert!(!group.is_null());
        assert_eq!(observed(group), None);
        assert_read_region(group, read_key, None);
        let (mut returned_key, mut index) = (0, usize::MAX);
        let mut outcome = DrAdapterOutcomeV1::default();
        let mut error = crate::empty_error_carrier();
        let mut control = DrCompletionStatusV1::default();
        let (tag, allocations) = crate::probe_runtime_allocations(|| {
            dr_v1_transfer_adapter_group_next(
                ptr::null(),
                group,
                &mut returned_key,
                &mut index,
                &mut outcome,
                &mut error,
                (&mut control as *mut DrCompletionStatusV1).cast(),
            )
        });
        assert_eq!(tag, 0);
        assert_eq!(allocations, 0);
        assert_eq!(returned_key, key.value);
        assert_eq!(index, 0);
        assert_eq!(observed(group), Some((read_key, true)));
        assert_eq!(outcome.tag, 0);
        assert_eq!(outcome.count, 2);
        assert_eq!(outcome.has_count, 1);
        assert_eq!(destination, [b'A', b'B', 0x55, 0x55]);
        assert_read_region(group, read_key, Some((destination.as_ptr(), 2)));
        assert_eq!((*receiver.as_ptr()).calls, 1);
        for (key, reading, offset, length) in [
            (read_key, 0, 0, 4),
            (read_key, 2, 0, 4),
            (read_key, 1, usize::MAX, 1),
            (read_key, 1, 3, 2),
            (
                crate::transfer_group::plan::OperationKey {
                    owner: 0,
                    ..read_key
                },
                1,
                0,
                4,
            ),
            (
                crate::transfer_group::plan::OperationKey {
                    position: 1,
                    ..read_key
                },
                1,
                0,
                4,
            ),
        ] {
            let (tag, allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_transfer_adapter_group_submit_key(
                    ptr::null(),
                    group,
                    ptr::from_ref(&key).cast(),
                    reading,
                    offset,
                    length,
                    &mut status,
                )
            });
            assert_eq!((tag, allocations), (3, 0));
            assert_eq!(status.failure(), Some(Failure::invalid_input()));
            crate::transfer_group::transport::tests::assert_group_configuration_failure(&status);
            assert_read_region(group, read_key, Some((destination.as_ptr(), 2)));
            assert_eq!(observed(group), Some((read_key, true)));
            assert_eq!((*receiver.as_ptr()).calls, 1);
        }
        for (key, reading, offset, length) in [
            (read_key, 0, 0, 4),
            (read_key, 2, 0, 4),
            (read_key, 1, -1, 1),
            (read_key, 1, 0, -1),
            (read_key, 1, i64::MIN, 1),
            (read_key, 1, i64::MAX, i64::MAX),
            (read_key, 1, 3, 2),
            (
                crate::transfer_group::plan::OperationKey {
                    owner: 0,
                    ..read_key
                },
                1,
                0,
                4,
            ),
            (
                crate::transfer_group::plan::OperationKey {
                    position: 1,
                    ..read_key
                },
                1,
                0,
                4,
            ),
        ] {
            assert_checked_resubmit(group, key, reading, offset, length, false);
            assert_read_region(group, read_key, Some((destination.as_ptr(), 2)));
            assert_eq!(observed(group), Some((read_key, true)));
            assert_eq!((*receiver.as_ptr()).calls, 1);
        }
        assert_checked_resubmit(group, read_key, 1, 1, 3, true);
        assert_read_region(group, read_key, None);
        assert_eq!(
            dr_v1_transfer_adapter_group_next(
                ptr::null(),
                group,
                &mut returned_key,
                &mut index,
                &mut outcome,
                &mut error,
                (&mut control as *mut DrCompletionStatusV1).cast()
            ),
            0
        );
        assert_eq!(outcome.tag, 2, "EOF comes from the adapter's explicit tag");
        assert_eq!(outcome.has_count, 0);
        assert_eq!(observed(group), Some((read_key, true)));
        assert_read_region(group, read_key, Some((destination[1..].as_ptr(), 0)));
        assert_eq!(
            dr_v1_transfer_adapter_group_submit_key(
                ptr::null(),
                group,
                ptr::from_ref(&read_key).cast(),
                1,
                2,
                2,
                &mut status
            ),
            0
        );
        assert_read_region(group, read_key, None);
        assert_eq!(
            dr_v1_transfer_adapter_group_next(
                ptr::null(),
                group,
                &mut returned_key,
                &mut index,
                &mut outcome,
                &mut error,
                (&mut control as *mut DrCompletionStatusV1).cast()
            ),
            0
        );
        assert_eq!(outcome.tag, 0, "Data(0) must not be inferred as EOF");
        assert_eq!(outcome.count, 0);
        assert_eq!(outcome.has_count, 1);
        assert_eq!(observed(group), Some((read_key, true)));
        assert_read_region(group, read_key, Some((destination[2..].as_ptr(), 0)));
        assert_eq!(
            dr_v1_transfer_adapter_group_next(
                ptr::null(),
                group,
                &mut returned_key,
                &mut index,
                &mut outcome,
                &mut error,
                (&mut control as *mut DrCompletionStatusV1).cast(),
            ),
            5,
        );
        assert_eq!(observed(group), None);
        dr_v1_transfer_adapter_group_drop(ptr::null(), group);

        let drops = std::cell::Cell::new(0);
        let dropped_frame = std::cell::Cell::new(0);
        struct ErrorPayload<'a> {
            drops: &'a std::cell::Cell<usize>,
            dropped_frame: &'a std::cell::Cell<usize>,
            progress: Option<i64>,
        }
        unsafe extern "C" fn drop_error(frame: *const DrStackFrameV2, object: *mut u8) {
            let payload = Box::from_raw(object.cast::<ErrorPayload<'_>>());
            payload.drops.set(payload.drops.get() + 1);
            payload.dropped_frame.set(frame as usize);
        }
        let descriptor = crate::DrInterfaceVtableV1 {
            type_name: crate::IO_ERROR_TYPE.as_ptr(),
            type_name_length: crate::IO_ERROR_TYPE.len(),
            message_offset: 0,
            drop: drop_error,
            class_size: mem::size_of::<ErrorPayload<'_>>(),
            origin_offset: 0,
            assertion_offsets: 0,
            assertion_magic: 0,
            class_descriptor: ptr::null(),
            interface_id: 0,
            io_transferred_byte_count_offset: 0,
            io_progress_magic: 0,
        };
        for reading in [true, false] {
            let object = Box::into_raw(Box::new(ErrorPayload {
                drops: &drops,
                dropped_frame: &dropped_frame,
                progress: Some(1),
            }));
            (*receiver.as_ptr()).error = Some(
                crate::transfer_group::adapter::OwnedIoError::from_carrier(DrInterfaceCarrierV1 {
                    object: object.cast(),
                    vtable: &descriptor,
                }),
            );
            let buffer = DrTransferBufferV1 {
                reading: u8::from(reading),
                ..buffer
            };
            let plan = plan::dr_v1_transfer_plan_new(
                core::ptr::null(),
                ptr::from_ref(&key).cast(),
                &buffer,
                &mut status,
            );
            let operation_key = (*plan.cast::<TransferPlan<'static>>())
                .operation_key(0, reading)
                .unwrap();
            assert_ne!(
                operation_key.owner, read_key.owner,
                "a new plan must not reuse an old identity"
            );
            let group = dr_v1_transfer_adapter_plan_begin(
                ptr::null(),
                set,
                plan,
                &budget,
                &mut crate::empty_error_carrier(),
                &mut status,
            );
            assert!(!group.is_null());
            assert_eq!(observed(group), None);
            let (tag, allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_transfer_adapter_group_observe(
                    ptr::null(),
                    group,
                    &budget,
                    &mut returned_key,
                    &mut index,
                    &mut outcome,
                    &mut error,
                    ptr::from_mut(&mut control).cast(),
                )
            });
            assert_eq!((tag, allocations), (1, 0));
            assert_eq!(observed(group), Some((operation_key, reading)));
            assert_eq!(error.object, object.cast());
            assert_eq!(error.vtable, ptr::from_ref(&descriptor));
            assert_eq!((*object).progress, Some(1));
            assert_eq!((outcome.has_count, outcome.count), (1, 1));
            if reading {
                assert_read_region(group, operation_key, Some((destination.as_ptr(), 1)));
            }
            for invalid in [
                read_key,
                crate::transfer_group::plan::OperationKey {
                    position: 1,
                    ..operation_key
                },
            ]
            .into_iter()
            .chain((!reading).then_some(operation_key))
            {
                let (mut data, mut length) = (destination.as_ptr(), usize::MAX);
                assert_eq!(
                    dr_v1_transfer_adapter_group_read_region(
                        group,
                        ptr::from_ref(&invalid).cast(),
                        &mut data,
                        &mut length,
                        &mut status
                    ),
                    3
                );
                assert!(data.is_null());
                assert_eq!(length, 0);
                assert_eq!(status.failure(), Some(Failure::invalid_input()));
                assert_checked_read_region(group, invalid, Err(()));
            }
            assert_eq!(error.object, object.cast());
            assert_eq!(error.vtable, ptr::from_ref(&descriptor));
            assert_eq!(observed(group), Some((operation_key, reading)));
            assert_eq!(
                dr_v1_transfer_adapter_group_next(
                    ptr::null(),
                    group,
                    &mut returned_key,
                    &mut index,
                    &mut outcome,
                    &mut error,
                    ptr::from_mut(&mut control).cast(),
                ),
                3,
                "an undropped caller-owned error cannot be overwritten",
            );
            assert_eq!(observed(group), Some((operation_key, reading)));
            dr_v1_transfer_adapter_group_drop(ptr::null(), group);
            assert_eq!(drops.get(), usize::from(!reading));
            ((*error.vtable).drop)(ptr::null(), error.object);
            error = crate::empty_error_carrier();
            assert_eq!(drops.get(), if reading { 1 } else { 2 });
        }
        for reading in [true, false] {
            let object = Box::into_raw(Box::new(ErrorPayload {
                drops: &drops,
                dropped_frame: &dropped_frame,
                progress: None,
            }));
            (*receiver.as_ptr()).error = Some(
                crate::transfer_group::adapter::OwnedIoError::from_carrier(DrInterfaceCarrierV1 {
                    object: object.cast(),
                    vtable: &descriptor,
                }),
            );
            let buffer = DrTransferBufferV1 {
                reading: u8::from(reading),
                ..buffer
            };
            let plan = plan::dr_v1_transfer_plan_new(
                ptr::null(),
                ptr::from_ref(&key).cast(),
                &buffer,
                &mut status,
            );
            let acquisition = mem::zeroed::<DrStackFrameV2>();
            let cleanup = mem::zeroed::<DrStackFrameV2>();
            let group = dr_v1_transfer_adapter_plan_begin(
                &acquisition,
                set,
                plan,
                &budget,
                &mut crate::empty_error_carrier(),
                &mut status,
            );
            assert!(!group.is_null());
            let previous = drops.get();
            let (_, allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_transfer_adapter_group_drop(&cleanup, group);
            });
            assert_eq!(allocations, 0);
            assert_eq!(drops.get(), previous + 1);
            assert_eq!(dropped_frame.get(), ptr::from_ref(&cleanup) as usize);
        }
        let (_, closes) = with_close_probe(|| {
            assert_eq!(
                dr_v1_transfer_adapter_set_remove(
                    set,
                    (&key as *const PlanKey).cast(),
                    &mut status
                ),
                0
            );
            assert_eq!(
                dr_v1_transfer_adapter_set_remove(
                    set,
                    (&key as *const PlanKey).cast(),
                    &mut status
                ),
                3
            );
        });
        assert_eq!(
            closes, 0,
            "metadata teardown must not close the source file"
        );
        // Set destruction may close its native watcher, but not the file.
        dr_v1_transfer_adapter_set_drop(ptr::null(), set);
        assert_eq!(dr_v1_file_control(file_pointer, 4, 0, 0, &mut status), 0);
        assert_eq!(status.value, 2);
        let (tag, closes) = with_close_probe(|| dr_v1_file_close(file_pointer, &mut status));
        assert_eq!(tag, 0);
        assert_eq!(closes, 1);
        assert!(error.object.is_null() && error.vtable.is_null());
        fs::remove_file(&path).unwrap();
        fs::remove_dir(&directory).unwrap();
    }
}

#[test]
fn adapter_admission_rejects_invalid_set_before_source_inspection() {
    unsafe {
        let mut status = DrIoStatusV1::default();
        let mut key = PlanKey { owner: 3, value: 4 };
        let (tag, allocations) = crate::probe_runtime_allocations(|| {
            dr_v1_transfer_adapter_set_add(
                ptr::null_mut(),
                ptr::dangling(),
                ptr::dangling(),
                (&mut key as *mut PlanKey).cast(),
                &mut status,
            )
        });
        assert_eq!(tag, 3);
        assert_eq!(allocations, 0);
        assert_eq!(key, PlanKey { owner: 0, value: 0 });
        assert_eq!(status.reason, Reason::InvalidInput as u8);
    }
}

#[test]
fn adapter_modes_keep_native_state_in_one_correctly_aligned_owner_allocation() {
    unsafe fn check<T>(native: T, pending: bool) {
        assert_eq!(mem::offset_of!(AdapterSetStorage<T>, owner), 0);
        assert!(mem::align_of::<AdapterSetStorage<T>>() <= crate::NATURAL_ALLOCATION_ALIGNMENT);
        assert!(mem::size_of::<AdapterSetMode<'_>>() <= 2 * mem::size_of::<usize>());
        let identity = next_identity(&NEXT_SET_IDENTITY).unwrap();
        let mut status = DrIoStatusV1::default();
        let (owner, allocations) = crate::probe_runtime_allocations(|| {
            allocate_adapter_set(
                core::ptr::null(),
                Ok(native),
                identity,
                pending,
                &mut status,
            )
        });
        assert!(!owner.is_null());
        assert_eq!(allocations, 1);
        assert_eq!(status.reason, 0);
        let header = owner.cast::<AdapterSetOwner>();
        assert_eq!((*header).pending, pending);
        assert_eq!((*header).identity, identity);
        assert_eq!((*header).next_registration, 0);
        assert!(!(*header).active);
        let expected = ptr::addr_of_mut!((*owner.cast::<AdapterSetStorage<T>>()).native).cast();
        let actual = match AdapterSetOwner::native(header) {
            AdapterSetMode::Synchronous(native) => ptr::from_mut(native).cast::<c_void>(),
            AdapterSetMode::Pending(native) => ptr::from_mut(native).cast::<c_void>(),
        };
        assert_eq!(actual, expected);
        let ((), allocations) = crate::probe_runtime_allocations(|| {
            dr_v1_transfer_adapter_set_drop(ptr::null(), owner)
        });
        assert_eq!(allocations, 0);
    }

    unsafe {
        check(AdapterSet::new(ptr::null_mut()).unwrap(), false);
        check(PendingSet::new(ptr::null_mut()).unwrap(), true);
    }
}

#[test]
fn adapter_modes_reject_foreign_descriptors_before_inspection() {
    unsafe {
        let mut status = DrIoStatusV1::default();
        let synchronous =
            dr_v1_transfer_adapter_set_new(core::ptr::null(), ptr::null_mut(), &mut status);
        let pending =
            dr_v1_pending_transfer_adapter_set_new(core::ptr::null(), ptr::null_mut(), &mut status);
        assert!(!synchronous.is_null() && !pending.is_null());
        let mut file_key = PlanKey { owner: 1, value: 2 };
        assert_eq!(
            dr_v1_pending_transfer_adapter_set_add_file(
                synchronous,
                ptr::dangling_mut(),
                ptr::dangling(),
                (&mut file_key as *mut PlanKey).cast(),
                &mut status,
            ),
            3,
        );
        assert_eq!((file_key.owner, file_key.value), (0, 0));
        assert_ne!(
            (*synchronous.cast::<AdapterSetOwner>()).identity,
            (*pending.cast::<AdapterSetOwner>()).identity
        );
        let mut key = PlanKey { owner: 7, value: 8 };
        assert_eq!(
            dr_v1_pending_transfer_adapter_set_add(
                synchronous,
                ptr::dangling(),
                ptr::dangling(),
                (&mut key as *mut PlanKey).cast(),
                &mut status
            ),
            3
        );
        assert_eq!(key, PlanKey { owner: 0, value: 0 });
        assert_eq!(
            dr_v1_pending_transfer_adapter_set_add_source(
                synchronous,
                ptr::dangling(),
                ptr::dangling(),
                (&mut key as *mut PlanKey).cast(),
                &mut status,
            ),
            3
        );
        assert_eq!(key, PlanKey { owner: 0, value: 0 });
        assert_eq!(
            dr_v1_transfer_adapter_set_add_source(
                pending,
                ptr::dangling(),
                ptr::dangling(),
                (&mut key as *mut PlanKey).cast(),
                &mut status,
            ),
            3
        );
        assert_eq!(
            dr_v1_transfer_adapter_set_add(
                pending,
                ptr::dangling(),
                ptr::dangling(),
                (&mut key as *mut PlanKey).cast(),
                &mut status
            ),
            3
        );
        assert_eq!(
            (*synchronous.cast::<AdapterSetOwner>()).next_registration,
            0
        );
        assert_eq!((*pending.cast::<AdapterSetOwner>()).next_registration, 0);
        dr_v1_transfer_adapter_set_drop(ptr::null(), synchronous);
        dr_v1_transfer_adapter_set_drop(ptr::null(), pending);
    }
}

#[test]
fn forwarded_file_source_admission_preserves_origin_before_read_write_and_missing_selection() {
    use crate::file_io::stream::transport::object::*;
    use crate::transfer_group::pending::{
        dr_v1_checked_pending_request_outcome, dr_v1_pending_request_configure_file,
    };
    struct Context {
        file: *mut c_void,
        path: *mut crate::DrStringV1,
        reading: bool,
        missing: bool,
        prepares: usize,
        completes: usize,
    }
    unsafe fn observe(view: *mut c_void, context: &Context, preparation: bool) -> (u8, u64, usize) {
        let (mut tag, mut count, mut code, mut progress, mut meta) = (0, 0, 0, 0, 0);
        let (mut message, mut path) = (ptr::null_mut(), ptr::null_mut());
        let result = dr_v1_checked_pending_request_outcome(
            ptr::null(),
            view,
            &mut tag,
            &mut count,
            &mut message,
            &mut path,
            &mut code,
            &mut progress,
            &mut meta,
        );
        if preparation {
            assert_eq!(result, 1);
            assert_eq!(path, context.path);
            assert!(!message.is_null());
            assert_eq!(meta & 0xff, if context.reading { 1 } else { 2 });
            assert_eq!((meta >> 8) & 0xff, 0);
            assert_eq!((meta >> 16) & 0xff, Reason::InvalidInput as u64);
            assert_eq!((meta >> 32) & 1, 0);
            assert_eq!((meta >> 34) & 1, 0);
        } else {
            assert_eq!(result, if context.missing { 4 } else { 0 });
            assert!(message.is_null() && path.is_null());
        }
        crate::dr_v1_string_release(message);
        crate::dr_v1_string_release(path);
        (result, tag, count)
    }
    unsafe extern "C" fn prepare_read(
        _: *const DrStackFrameV2,
        context: *const DrAdapterContextV1,
        view: *mut c_void,
        data: *mut u8,
        length: usize,
        _: *mut DrAdapterOutcomeV1,
        _: *mut DrInterfaceCarrierV1,
    ) -> u8 {
        let context = TestObject::<Context>::in_callback((*context).object);
        context.prepares += 1;
        observe(view, context, true);
        if !context.missing {
            dr_v1_pending_request_configure_file(view, context.file, data, length);
        }
        0
    }
    unsafe extern "C" fn prepare_write(
        frame: *const DrStackFrameV2,
        context: *const DrAdapterContextV1,
        view: *mut c_void,
        data: *const u8,
        length: usize,
        progress: *mut DrAdapterOutcomeV1,
        error: *mut DrInterfaceCarrierV1,
    ) -> u8 {
        prepare_read(
            frame,
            context,
            view,
            data.cast_mut(),
            length,
            progress,
            error,
        )
    }
    unsafe extern "C" fn complete_read(
        _: *const DrStackFrameV2,
        context: *const DrAdapterContextV1,
        view: *mut c_void,
        _: *mut u8,
        _: usize,
        outcome: *mut DrAdapterOutcomeV1,
        _: *mut DrInterfaceCarrierV1,
    ) -> u8 {
        let context = TestObject::<Context>::in_callback((*context).object);
        context.completes += 1;
        let (result, tag, count) = observe(view, context, false);
        if result == 4 {
            return 4;
        }
        *outcome = DrAdapterOutcomeV1 {
            tag: tag as u8,
            count,
            has_count: u8::from(tag == 0),
        };
        0
    }
    unsafe extern "C" fn complete_write(
        frame: *const DrStackFrameV2,
        context: *const DrAdapterContextV1,
        view: *mut c_void,
        data: *const u8,
        length: usize,
        outcome: *mut DrAdapterOutcomeV1,
        error: *mut DrInterfaceCarrierV1,
    ) -> u8 {
        complete_read(
            frame,
            context,
            view,
            data.cast_mut(),
            length,
            outcome,
            error,
        )
    }
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_file(&self.0).unwrap();
        }
    }
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "doria-forwarded-origin-{}-{}", std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
    )));
    fs::write(&fixture.0, b"unchanged").unwrap();
    unsafe {
        let bytes = fixture.0.to_str().unwrap().as_bytes();
        let path = crate::dr_v1_string_from_utf8(bytes.as_ptr(), bytes.len());
        let (mut size, mut align) = (0, 0);
        dr_v1_file_object_layout(&mut size, &mut align);
        let file = crate::dr_v2_class_allocate(ptr::null(), size, align).cast();
        let mut status = DrIoStatusV1::default();
        assert_eq!(dr_v1_file_object_open(path, 2, 0, 0, file, &mut status), 0);
        let references = (*path).references;
        let (source, allocations) = crate::probe_runtime_allocations(|| {
            let home = dr_v1_file_object_readiness_source_home(file);
            let inner = *home.cast::<*const c_void>();
            let outer = inner;
            assert_eq!(outer, dr_v1_file_object_readiness_source(file));
            outer
        });
        assert_eq!(allocations, 0);
        assert_eq!((*path).references, references);
        let context = TestObject::new(Context {
            file,
            path,
            reading: true,
            missing: false,
            prepares: 0,
            completes: 0,
        });
        let adapter = DrPendingTransferAdapterV1 {
            context: context.context(),
            prepare_read: Some(prepare_read),
            prepare_write: Some(prepare_write),
            complete_read: Some(complete_read),
            complete_write: Some(complete_write),
        };
        let set =
            dr_v1_pending_transfer_adapter_set_new(core::ptr::null(), ptr::null_mut(), &mut status);
        assert!(!set.is_null());
        let mut key = PlanKey { owner: 0, value: 0 };
        assert_eq!(
            dr_v1_pending_transfer_adapter_set_add_source(
                set,
                source,
                &adapter,
                ptr::from_mut(&mut key).cast(),
                &mut status,
            ),
            0
        );
        assert_eq!(
            ((*context.as_ptr()).prepares, (*context.as_ptr()).completes),
            (0, 0)
        );
        assert_eq!((*path).references, references);
        let mut budget = mem::MaybeUninit::uninit();
        let (mut reason, mut code, mut has_code) = (0, 0, 0);
        assert_eq!(
            crate::wait_budget::dr_v1_wait_budget_init(
                0,
                0,
                budget.as_mut_ptr(),
                &mut reason,
                &mut code,
                &mut has_code,
            ),
            0
        );
        let budget = budget.assume_init();
        for (reading, missing, checked) in [
            (true, false, false),
            (false, false, false),
            (true, true, false),
            (false, true, false),
            (true, false, true),
            (false, false, true),
            (true, true, true),
            (false, true, true),
        ] {
            (*context.as_ptr()).reading = reading;
            (*context.as_ptr()).missing = missing;
            let prior = ((*context.as_ptr()).prepares, (*context.as_ptr()).completes);
            let buffer = DrTransferBufferV1 {
                data: ptr::null_mut(),
                length: 0,
                reading: u8::from(reading),
            };
            let plan = plan::dr_v1_transfer_plan_new(
                core::ptr::null(),
                ptr::from_ref(&key).cast(),
                &buffer,
                &mut status,
            );
            assert!(!plan.is_null());
            let mut error = crate::empty_error_carrier();
            let group = if checked {
                checked_begin(ptr::null(), set, plan, &mut error, u8::from(missing))
            } else {
                dr_v1_pending_transfer_adapter_plan_begin(
                    ptr::null(),
                    set,
                    plan,
                    &budget,
                    &mut error,
                    &mut status,
                )
            };
            if missing {
                assert!(group.is_null());
                if !checked {
                    assert_eq!(status.reason, Reason::InvalidInput as u8);
                }
            } else {
                assert!(!group.is_null());
                let (mut returned, mut index) = (0, usize::MAX);
                let mut outcome = DrAdapterOutcomeV1::default();
                let mut control = DrCompletionStatusV1::default();
                assert_eq!(
                    dr_v1_transfer_adapter_group_next(
                        ptr::null(),
                        group,
                        &mut returned,
                        &mut index,
                        &mut outcome,
                        &mut error,
                        ptr::from_mut(&mut control).cast(),
                    ),
                    0
                );
                assert_eq!((outcome.tag, outcome.count), (0, 0));
                assert_eq!((returned, index), (key.value, 0));
                dr_v1_transfer_adapter_group_drop(ptr::null(), group);
            }
            assert!(error.object.is_null() && error.vtable.is_null());
            assert_eq!(
                ((*context.as_ptr()).prepares, (*context.as_ptr()).completes),
                (prior.0 + 1, prior.1 + 1)
            );
            assert_eq!((*path).references, references);
        }
        assert_eq!(
            dr_v1_transfer_adapter_set_remove(set, ptr::from_ref(&key).cast(), &mut status),
            0
        );
        dr_v1_transfer_adapter_set_drop(ptr::null(), set);
        let (result, closes) = with_close_probe(|| dr_v1_file_object_close(file, &mut status));
        assert_eq!((result, closes), (0, 1));
        dr_v1_file_object_drop(ptr::null(), file);
        crate::dr_v1_class_free(file.cast());
        crate::dr_v1_string_release(path);
    }
    assert_eq!(fs::read(&fixture.0).unwrap(), b"unchanged");
}

#[cfg(unix)]
#[path = "pending_tests.rs"]
mod pending_tests;
