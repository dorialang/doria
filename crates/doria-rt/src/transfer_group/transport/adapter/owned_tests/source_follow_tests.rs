//! Real public File owners exercise source replacement through the actual
//! owning admission, shared pending group and platform driver, not a fixed raw
//! OwnedFile fixture that accidentally keeps destroyed storage alive.
use super::*;
use crate::file_io::stream::transport::object::*;
use crate::native_handle::with_close_probe;
use crate::transfer_group::pending::dr_v1_checked_pending_request_configure_file;

struct PublicFile {
    object: *mut c_void,
    path: *mut crate::DrStringV1,
}

impl PublicFile {
    unsafe fn new(fixture: &Fixture, name: &str, contents: &[u8]) -> Self {
        let filename = fixture.directory.join(name);
        fs::write(&filename, contents).unwrap();
        let bytes = filename.to_str().unwrap().as_bytes();
        let path = crate::dr_v1_string_from_utf8(bytes.as_ptr(), bytes.len());
        let (mut size, mut alignment) = (0, 0);
        dr_v1_file_object_layout(&mut size, &mut alignment);
        let object = crate::dr_v2_class_allocate(ptr::null(), size, alignment).cast();
        assert_eq!(
            dr_v1_file_object_open(path, 0, 0, 0, object, &mut DrIoStatusV1::default()),
            0
        );
        Self { object, path }
    }

    unsafe fn source(&self) -> *const c_void {
        dr_v1_file_object_readiness_source(self.object)
    }
}

impl Drop for PublicFile {
    fn drop(&mut self) {
        unsafe {
            dr_v1_file_object_drop(ptr::null(), self.object);
            crate::dr_v1_class_free(self.object.cast());
            crate::dr_v1_string_release(self.path);
        }
    }
}

#[test]
fn hosted_pending_admission_keeps_external_owner_and_nominal_direction_without_callbacks() {
    unsafe {
        let fixture = Fixture::new();
        let first = PublicFile::new(&fixture, "host-first.bin", b"first");
        let second = PublicFile::new(&fixture, "host-second.bin", b"second");
        let mut status = DrIoStatusV1::default();
        let set = dr_v1_readiness_set_new(ptr::null(), &mut status);
        assert!(!set.is_null());
        let mut key = PlanKey {
            owner: 99,
            value: 99,
        };
        let key_home = ptr::from_mut(&mut key).cast();
        for (reading, writing) in [(0, 0), (2, 0), (0, 2)] {
            let (tag, allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_readiness_set_add_host_pending_source(
                    set,
                    first.source(),
                    reading,
                    writing,
                    key_home,
                    &mut status,
                )
            });
            assert_eq!((tag, allocations), (3, 0));
            assert_eq!((key.owner, key.value), (0, 0));
            assert_eq!(status.failure(), Some(Failure::invalid_input()));
        }
        assert_eq!(
            dr_v1_readiness_set_add_host_pending_source(
                set,
                first.source(),
                1,
                0,
                key_home,
                &mut status
            ),
            0
        );
        let header = set.cast::<AdapterSetOwner>();
        assert_eq!(
            (*header).retained.occupied_len(),
            0,
            "host retains its real complete value"
        );
        let AdapterSetMode::Pending(native) = AdapterSetOwner::native(header) else {
            unreachable!()
        };
        let index = native.index_of(key.value).unwrap();
        let bytes = [0; 1];
        let buffers = BufferList::from_iter([Buffer::Write(&bytes)].into_iter()).unwrap();
        let mut selected = FixedSlice::new(1).unwrap();
        selected.push(index).unwrap();
        let budget = DrWaitBudgetV1::indefinite();
        assert!(
            matches!(native.begin_selected(ptr::null(), buffers, &budget, selected), Err(SetupFailure::Platform(failure)) if failure == Failure::invalid_input())
        );
        let ((), closes) = with_close_probe(|| {
            assert_eq!(
                dr_v1_readiness_set_get_source(set, key_home, &mut status),
                0
            );
            assert_eq!(
                dr_v1_readiness_set_update_source(set, key_home, second.source(), &mut status),
                0
            );
            assert_eq!(
                dr_v1_readiness_set_remove_source(set, key_home, &mut status),
                0
            );
        });
        assert_eq!(closes, 0);
        // Set teardown may close its own watcher, never either external File.
        dr_v1_transfer_adapter_set_drop(ptr::null(), set);
        for (file, expected) in [
            (&first, b"first".as_slice()),
            (&second, b"second".as_slice()),
        ] {
            let mut bytes = [0; 8];
            assert_eq!(
                dr_v1_file_object_read(file.object, bytes.as_mut_ptr(), bytes.len(), &mut status),
                0
            );
            assert_eq!(&bytes[..status.value as usize], expected);
        }
    }
}

struct SwitchingStream {
    file: PublicFile,
    replacement: Option<PublicFile>,
    home: *const DrInterfaceCarrierV1,
    resolutions: usize,
    completions: usize,
    sync_calls: usize,
    getter_error: Option<OwnedInterfaceValue>,
}

unsafe extern "C" fn drop_switching(_: *const DrStackFrameV2, object: *mut u8) {
    drop(Box::from_raw(object.cast::<SwitchingStream>()));
}

fn switching_descriptor() -> DrInterfaceVtableV1 {
    DrInterfaceVtableV1 {
        drop: drop_switching,
        class_size: mem::size_of::<SwitchingStream>(),
        ..descriptor()
    }
}

unsafe extern "C" fn resolve_switching(
    _: *const DrStackFrameV2,
    home: *const DrInterfaceCarrierV1,
    source_out: *mut *const c_void,
    context_out: *mut DrAdapterContextV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    let stream = &mut *(*home).object.cast::<SwitchingStream>();
    if stream.home.is_null() {
        stream.home = home;
    }
    assert_eq!(
        home, stream.home,
        "resolver must borrow the actual retained carrier home"
    );
    stream.resolutions += 1;
    if let Some(owned) = stream.getter_error.take() {
        *error = owned.into_carrier();
        return 1;
    }
    *source_out = stream.file.source();
    *context_out = DrAdapterContextV1 {
        object: (*home).object.cast(),
        descriptor: (*home).vtable.cast(),
    };
    0
}

unsafe extern "C" fn prepare_switching(
    frame: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    data: *mut u8,
    length: usize,
    _: *mut DrAdapterOutcomeV1,
    error: *mut DrInterfaceCarrierV1,
) -> u8 {
    let stream = &mut *(*context).object.cast::<SwitchingStream>();
    let replacing = stream.replacement.is_some();
    if let Some(replacement) = stream.replacement.take() {
        let old = mem::replace(&mut stream.file, replacement);
        let ((), closes) = with_close_probe(|| drop(old));
        assert_eq!(closes, 0, "selected use must survive wrapper destruction");
    }
    let mut fields = CheckedFields::empty();
    let ((tag, allocations), closes) = with_close_probe(|| {
        crate::probe_runtime_allocations(|| {
            dr_v1_checked_pending_request_configure_file(
                frame,
                view,
                stream.file.object,
                ptr::null(),
                data,
                length,
                error,
                &mut fields.message,
                &mut fields.path,
                &mut fields.system_code,
                &mut fields.transferred,
                &mut fields.meta,
            )
        })
    });
    assert_eq!((tag, allocations), (0, 0));
    assert_eq!(
        closes,
        usize::from(replacing),
        "unsubmitted old selection retires once after fresh configuration"
    );
    fields.assert_success();
    0
}

unsafe extern "C" fn complete_switching(
    _: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    _: *mut u8,
    _: usize,
    outcome: *mut DrAdapterOutcomeV1,
    _: *mut DrInterfaceCarrierV1,
) -> u8 {
    let stream = &mut *(*context).object.cast::<SwitchingStream>();
    stream.completions += 1;
    let (tag, _, native, _, _) = pending_callback_facts(view);
    if tag == 4 {
        return 4;
    }
    assert_eq!(tag, 0);
    *outcome = DrAdapterOutcomeV1 {
        count: (*native).value as usize,
        tag,
        has_count: 1,
    };
    0
}

unsafe fn exercise_switching(replace_during_prepare: bool) {
    let fixture = Fixture::new();
    let table = switching_descriptor();
    let first = PublicFile::new(&fixture, "first-Dória.bin", b"ab");
    let second = PublicFile::new(&fixture, "second-Dória.bin", b"xy");
    let stream = Box::into_raw(Box::new(SwitchingStream {
        file: first,
        replacement: None,
        home: ptr::null(),
        resolutions: 0,
        completions: 0,
        sync_calls: 0,
        getter_error: None,
    }));
    let mut value = DrInterfaceCarrierV1 {
        object: stream.cast(),
        vtable: &table,
    };
    let adapter = DrPendingTransferAdapterV1 {
        context: DrAdapterContextV1 {
            object: stream.cast(),
            descriptor: ptr::from_ref(&table).cast(),
        },
        prepare_read: Some(prepare_switching),
        prepare_write: None,
        complete_read: Some(complete_switching),
        complete_write: None,
    };
    let set = dr_v1_readiness_set_new(ptr::null(), &mut DrIoStatusV1::default());
    let mut key = PlanKey { owner: 0, value: 0 };
    assert_eq!(
        dr_v1_readiness_set_add_owned(
            ptr::null(),
            set,
            (*stream).file.source(),
            Some(resolve_switching),
            &adapter,
            &mut value,
            ptr::from_mut(&mut key).cast(),
            &mut DrIoStatusV1::default()
        ),
        0
    );
    assert!(value.object.is_null());
    assert_eq!(
        (*stream).resolutions,
        0,
        "admission must not invoke a getter"
    );
    if replace_during_prepare {
        (*stream).replacement = Some(second);
    } else {
        let old = mem::replace(&mut (*stream).file, second);
        let ((), closes) = with_close_probe(|| drop(old));
        assert_eq!(
            closes, 1,
            "idle metadata must not keep an unused old handle open"
        );
    }
    let mut bytes = [0; 2];
    let region = DrTransferBufferV1 {
        data: bytes.as_mut_ptr(),
        length: bytes.len(),
        reading: 1,
    };
    let mut status = DrIoStatusV1::default();
    let plan = plan::dr_v1_transfer_plan_new(
        ptr::null(),
        ptr::from_ref(&key).cast(),
        &region,
        &mut status,
    );
    assert!(!plan.is_null());
    let mut error = crate::empty_error_carrier();
    let group = dr_v1_pending_transfer_adapter_plan_begin(
        ptr::null(),
        set,
        plan,
        &immediate_budget(),
        &mut error,
        &mut status,
    );
    assert!(!group.is_null(), "{:?}", status.failure());
    let (mut completed_key, mut index) = (0, usize::MAX);
    let mut outcome = DrAdapterOutcomeV1::default();
    let mut control = DrCompletionStatusV1::default();
    assert_eq!(
        dr_v1_transfer_adapter_group_next(
            ptr::null(),
            group,
            &mut completed_key,
            &mut index,
            &mut outcome,
            &mut error,
            ptr::from_mut(&mut control).cast()
        ),
        0
    );
    assert_eq!((completed_key, index, outcome.count), (key.value, 0, 2));
    dr_v1_transfer_adapter_group_drop(ptr::null(), group);
    assert_eq!(bytes, *b"xy");
    assert_eq!(
        (*stream).resolutions,
        3,
        "setup, preparation and configuration each resolve fresh"
    );
    assert_eq!((*stream).completions, 1);
    dr_v1_readiness_set_drop(ptr::null(), set);
}

#[test]
fn native_group_follows_replaced_public_file_before_begin_without_stale_wrapper_access() {
    unsafe {
        exercise_switching(false);
    }
}

#[test]
fn native_configuration_follows_replacement_during_prepare_and_retires_old_selected_use_once() {
    unsafe {
        exercise_switching(true);
    }
}

unsafe extern "C" fn read_switching(
    _: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    data: *mut u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    _: *mut DrInterfaceCarrierV1,
) -> u8 {
    let stream = &mut *(*context).object.cast::<SwitchingStream>();
    stream.sync_calls += 1;
    if let Some(replacement) = stream.replacement.take() {
        let old = mem::replace(&mut stream.file, replacement);
        let ((), closes) = with_close_probe(|| drop(old));
        assert_eq!(closes, 0, "current callback retains its old selected use");
        (*outcome).tag = 1;
        return 0;
    }
    let mut status = DrIoStatusV1::default();
    let tag = dr_v1_file_object_read(stream.file.object, data, length, &mut status);
    assert!(matches!(tag, 0 | 2));
    *outcome = DrAdapterOutcomeV1 {
        tag,
        count: status.value as usize,
        has_count: u8::from(tag == 0),
    };
    0
}

unsafe fn exercise_synchronous_switching(replace_during_retry: bool) {
    let fixture = Fixture::new();
    let table = switching_descriptor();
    let first = PublicFile::new(&fixture, "sync-first.bin", b"ab");
    let second = PublicFile::new(&fixture, "sync-second.bin", b"xy");
    let stream = Box::into_raw(Box::new(SwitchingStream {
        file: first,
        replacement: None,
        home: ptr::null(),
        resolutions: 0,
        completions: 0,
        sync_calls: 0,
        getter_error: None,
    }));
    let mut value = DrInterfaceCarrierV1 {
        object: stream.cast(),
        vtable: &table,
    };
    let adapter = DrTransferAdapterV1 {
        context: DrAdapterContextV1 {
            object: stream.cast(),
            descriptor: ptr::from_ref(&table).cast(),
        },
        read: Some(read_switching),
        write: None,
    };
    let set = new_set(false);
    let mut key = PlanKey { owner: 0, value: 0 };
    assert_eq!(
        dr_v1_transfer_adapter_set_add_owned(
            ptr::null(),
            set,
            (*stream).file.source(),
            Some(resolve_switching),
            &adapter,
            &mut value,
            ptr::from_mut(&mut key).cast(),
            &mut DrIoStatusV1::default()
        ),
        0
    );
    assert!(value.object.is_null());
    assert_eq!((*stream).resolutions, 0);
    if replace_during_retry {
        (*stream).replacement = Some(second);
    } else {
        let old = mem::replace(&mut (*stream).file, second);
        let ((), closes) = with_close_probe(|| drop(old));
        assert_eq!(
            closes, 1,
            "idle registration retains metadata, not an unused handle"
        );
    }
    let mut bytes = [0; 2];
    let region = DrTransferBufferV1 {
        data: bytes.as_mut_ptr(),
        length: bytes.len(),
        reading: 1,
    };
    let mut status = DrIoStatusV1::default();
    let plan = plan::dr_v1_transfer_plan_new(
        ptr::null(),
        ptr::from_ref(&key).cast(),
        &region,
        &mut status,
    );
    assert!(!plan.is_null());
    let mut error = crate::empty_error_carrier();
    let (group, closes) = with_close_probe(|| {
        dr_v1_transfer_adapter_plan_begin(
            ptr::null(),
            set,
            plan,
            &immediate_budget(),
            &mut error,
            &mut status,
        )
    });
    assert!(!group.is_null(), "{:?}", status.failure());
    assert_eq!(closes, usize::from(replace_during_retry));
    let (mut completed_key, mut index) = (0, usize::MAX);
    let mut outcome = DrAdapterOutcomeV1::default();
    let mut control = DrCompletionStatusV1::default();
    let (tag, allocations) = crate::probe_runtime_allocations(|| {
        dr_v1_transfer_adapter_group_next(
            ptr::null(),
            group,
            &mut completed_key,
            &mut index,
            &mut outcome,
            &mut error,
            ptr::from_mut(&mut control).cast(),
        )
    });
    assert_eq!(
        (tag, allocations, completed_key, index, outcome.count),
        (0, 0, key.value, 0, 2)
    );
    dr_v1_transfer_adapter_group_drop(ptr::null(), group);
    assert_eq!(bytes, *b"xy");
    assert_eq!((*stream).sync_calls, 1 + usize::from(replace_during_retry));
    assert!(
        (*stream).resolutions >= 3,
        "setup, selection and dispatch resolve the current source"
    );
    dr_v1_transfer_adapter_set_drop(ptr::null(), set);
}

#[test]
fn synchronous_owned_group_follows_replaced_public_file_before_begin() {
    unsafe {
        exercise_synchronous_switching(false);
    }
}

#[test]
fn synchronous_retry_follows_replacement_and_retires_previous_use_before_waiting() {
    unsafe {
        exercise_synchronous_switching(true);
    }
}

struct GetterFailure {
    message: *mut crate::DrStringV1,
    drops: std::rc::Rc<std::cell::Cell<usize>>,
}

unsafe extern "C" fn drop_getter_failure(_: *const DrStackFrameV2, object: *mut u8) {
    let error = Box::from_raw(object.cast::<GetterFailure>());
    crate::dr_v1_string_release(error.message);
    error.drops.set(error.drops.get() + 1);
}

#[test]
fn synchronous_setup_preserves_original_getter_error_without_invoking_transfer() {
    unsafe {
        let fixture = Fixture::new();
        let table = switching_descriptor();
        let drops = std::rc::Rc::new(std::cell::Cell::new(0));
        let error_table = DrInterfaceVtableV1 {
            type_name: b"Fixture\\GetterFailure".as_ptr(),
            type_name_length: b"Fixture\\GetterFailure".len(),
            drop: drop_getter_failure,
            class_size: mem::size_of::<GetterFailure>(),
            message_offset: core::mem::offset_of!(GetterFailure, message),
            ..descriptor()
        };
        let error_object = Box::into_raw(Box::new(GetterFailure {
            message: crate::dr_v1_string_from_utf8(b"original getter failure".as_ptr(), 23),
            drops: std::rc::Rc::clone(&drops),
        }))
        .cast::<u8>();
        let stream = Box::into_raw(Box::new(SwitchingStream {
            file: PublicFile::new(&fixture, "sync-error.bin", b"ab"),
            replacement: None,
            home: ptr::null(),
            resolutions: 0,
            completions: 0,
            sync_calls: 0,
            getter_error: Some(OwnedInterfaceValue::from_carrier(DrInterfaceCarrierV1 {
                object: error_object,
                vtable: &error_table,
            })),
        }));
        let mut value = DrInterfaceCarrierV1 {
            object: stream.cast(),
            vtable: &table,
        };
        let adapter = DrTransferAdapterV1 {
            context: DrAdapterContextV1 {
                object: stream.cast(),
                descriptor: ptr::from_ref(&table).cast(),
            },
            read: Some(read_switching),
            write: None,
        };
        let set = new_set(false);
        let mut key = PlanKey { owner: 0, value: 0 };
        let mut status = DrIoStatusV1::default();
        assert_eq!(
            dr_v1_transfer_adapter_set_add_owned(
                ptr::null(),
                set,
                (*stream).file.source(),
                Some(resolve_switching),
                &adapter,
                &mut value,
                ptr::from_mut(&mut key).cast(),
                &mut status
            ),
            0
        );
        assert_eq!((*stream).resolutions, 0);
        let mut bytes = [0; 2];
        let region = DrTransferBufferV1 {
            data: bytes.as_mut_ptr(),
            length: bytes.len(),
            reading: 1,
        };
        let plan = plan::dr_v1_transfer_plan_new(
            ptr::null(),
            ptr::from_ref(&key).cast(),
            &region,
            &mut status,
        );
        let mut error = crate::empty_error_carrier();
        let group = dr_v1_transfer_adapter_plan_begin(
            ptr::null(),
            set,
            plan,
            &immediate_budget(),
            &mut error,
            &mut status,
        );
        assert!(group.is_null());
        assert_eq!(error.object, error_object);
        assert_eq!(error.vtable, ptr::from_ref(&error_table));
        assert_eq!((*stream).sync_calls, 0);
        assert_eq!(bytes, [0; 2]);
        assert_eq!(drops.get(), 0);
        OwnedInterfaceValue::from_carrier(error).drop_in_frame(ptr::null());
        assert_eq!(drops.get(), 1);
        dr_v1_transfer_adapter_set_drop(ptr::null(), set);
        assert_eq!(drops.get(), 1);
    }
}

#[test]
fn synchronous_owned_admission_rejects_null_resolver_and_consumes_taken_value() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let mut value = fixture.stream(81, &table);
        let original = value.object;
        let source = source(&value);
        let adapter = DrTransferAdapterV1 {
            context: DrAdapterContextV1 {
                object: original.cast(),
                descriptor: ptr::null(),
            },
            read: Some(read),
            write: None,
        };
        let set = new_set(false);
        let mut key = PlanKey { owner: 9, value: 9 };
        let mut status = DrIoStatusV1::default();
        assert_eq!(
            dr_v1_transfer_adapter_set_add_owned(
                ptr::null(),
                set,
                source,
                None,
                &adapter,
                &mut value,
                ptr::from_mut(&mut key).cast(),
                &mut status
            ),
            3
        );
        assert_eq!(status.failure(), Some(Failure::invalid_input()));
        assert!(value.object.is_null() && value.vtable.is_null());
        assert_eq!((key.owner, key.value), (0, 0));
        assert_eq!(&*fixture.drops.borrow(), &[81]);
        assert_eq!((*set.cast::<AdapterSetOwner>()).retained.occupied_len(), 0);
        dr_v1_transfer_adapter_set_drop(ptr::null(), set);
        assert_eq!(&*fixture.drops.borrow(), &[81]);
    }
}

#[test]
fn external_source_update_requires_fresh_input_and_reuses_metadata_without_allocating() {
    unsafe {
        let fixture = Fixture::new();
        let first = PublicFile::new(&fixture, "external-first.bin", b"ab");
        let second = PublicFile::new(&fixture, "external-second.bin", b"xy");
        let set = dr_v1_readiness_set_new(ptr::null(), &mut DrIoStatusV1::default());
        let mut key = PlanKey { owner: 0, value: 0 };
        let mut status = DrIoStatusV1::default();
        assert_eq!(
            dr_v1_readiness_set_add_source(
                set,
                first.source(),
                ptr::from_mut(&mut key).cast(),
                &mut status
            ),
            0
        );
        let ((), closes) = with_close_probe(|| drop(first));
        assert_eq!(closes, 1);
        assert_eq!(
            dr_v1_readiness_set_update_source(
                set,
                ptr::from_ref(&key).cast(),
                ptr::null(),
                &mut status
            ),
            3
        );
        assert_eq!(status.failure(), Some(Failure::invalid_input()));
        let (tag, allocations) = crate::probe_runtime_allocations(|| {
            dr_v1_readiness_set_update_source(
                set,
                ptr::from_ref(&key).cast(),
                second.source(),
                &mut status,
            )
        });
        assert_eq!((tag, allocations), (0, 0));
        assert_eq!(
            dr_v1_readiness_set_remove_source(set, ptr::from_ref(&key).cast(), &mut status),
            0
        );
        dr_v1_readiness_set_drop(ptr::null(), set);
        drop(second);
    }
}

#[test]
fn owned_admission_rejects_null_resolver_and_consumes_taken_value_without_native_initialization() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let mut value = fixture.stream(1, &table);
        let set = dr_v1_readiness_set_new(ptr::null(), &mut DrIoStatusV1::default());
        let mut key = PlanKey {
            owner: usize::MAX,
            value: u64::MAX,
        };
        let mut status = DrIoStatusV1::default();
        assert_eq!(
            dr_v1_readiness_set_add_owned(
                ptr::null(),
                set,
                source(&value),
                None,
                ptr::null(),
                &mut value,
                ptr::from_mut(&mut key).cast(),
                &mut status
            ),
            3
        );
        assert_eq!(status.failure(), Some(Failure::invalid_input()));
        assert!(value.object.is_null() && value.vtable.is_null());
        assert_eq!((key.owner, key.value), (0, 0));
        let AdapterSetMode::Pending(native) = AdapterSetOwner::native(set.cast()) else {
            unreachable!()
        };
        assert!(!native.initialized());
        assert_eq!(*fixture.drops.borrow(), [1]);
        dr_v1_readiness_set_drop(ptr::null(), set);
    }
}
