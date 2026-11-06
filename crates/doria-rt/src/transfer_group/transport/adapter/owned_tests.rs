use super::*;
use crate::file_io::stream::transport::*;
use crate::transfer_group::adapter::{DrAdapterContextV1, ReadCallback, WriteCallback};
use crate::transfer_group::pending::{
    dr_v1_pending_request_complete, dr_v1_pending_request_configure_native, pending_callback_facts,
    CompleteReadCallback, CompleteWriteCallback, PrepareReadCallback, PrepareWriteCallback,
};
use crate::DrInterfaceVtableV1;
use std::cell::RefCell;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

mod borrowed_tests;
mod checked_observation_tests;
mod source_follow_tests;

struct Stream {
    file: OwnedFile,
    path: *mut crate::DrStringV1,
    source: mem::MaybeUninit<ReadinessProjection>,
    drops: *const RefCell<Vec<usize>>,
    id: usize,
    calls: usize,
    expected_descriptor: *const c_void,
    expected_drop_frame: *const DrStackFrameV2,
    dispatch_calls: usize,
    written: [u8; 2],
    pending: bool,
    completion_error: Option<crate::transfer_group::adapter::OwnedIoError>,
}

impl Drop for Stream {
    fn drop(&mut self) {
        assert!(
            !self.pending,
            "drain must precede complete-value destruction"
        );
        unsafe {
            (*self.drops).borrow_mut().push(self.id);
            crate::dr_v1_string_release(self.path);
        }
    }
}

unsafe extern "C" fn drop_stream(frame: *const DrStackFrameV2, object: *mut u8) {
    assert_eq!(frame, (*object.cast::<Stream>()).expected_drop_frame);
    drop(Box::from_raw(object.cast::<Stream>()));
}

fn descriptor() -> DrInterfaceVtableV1 {
    DrInterfaceVtableV1 {
        type_name: b"Fixture\\CompleteStream".as_ptr(),
        type_name_length: b"Fixture\\CompleteStream".len(),
        message_offset: 0,
        drop: drop_stream,
        class_size: mem::size_of::<Stream>(),
        origin_offset: 0,
        assertion_offsets: 0,
        assertion_magic: 0,
        class_descriptor: ptr::null(),
        interface_id: 17,
        io_transferred_byte_count_offset: 0,
        io_progress_magic: 0,
    }
}

unsafe fn observed_stream(context: *const DrAdapterContextV1) -> *mut Stream {
    let stream = (*context).object.cast::<Stream>();
    assert_eq!((*context).descriptor, (*stream).expected_descriptor);
    (*stream).dispatch_calls += 1;
    stream
}

unsafe extern "C" fn read(
    _: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    data: *mut u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    _: *mut DrInterfaceCarrierV1,
) -> u8 {
    let stream = &mut *observed_stream(context);
    stream.calls += 1;
    let mut status = DrIoStatusV1::default();
    let tag = dr_v1_file_read(
        ptr::from_mut(&mut stream.file).cast(),
        data,
        length,
        &mut status,
    );
    assert!(matches!(tag, 0 | 2));
    *outcome = DrAdapterOutcomeV1 {
        tag,
        count: status.value as usize,
        has_count: u8::from(tag == 0),
    };
    for byte in core::slice::from_raw_parts_mut(data, status.value as usize) {
        *byte = byte.to_ascii_uppercase();
    }
    0
}

unsafe extern "C" fn prepare(
    _: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    data: *mut u8,
    length: usize,
    _: *mut DrAdapterOutcomeV1,
    _: *mut DrInterfaceCarrierV1,
) -> u8 {
    let stream = &mut *observed_stream(context);
    stream.calls += 1;
    stream.pending = true;
    // This fixture owns a raw OwnedFile, not a compiler File payload. Select
    // its actual native owner without assuming the public FileObject layout.
    dr_v1_pending_request_configure_native(
        view,
        ptr::from_mut(&mut stream.file).cast(),
        data,
        length,
    );
    0
}

unsafe extern "C" fn complete(
    _: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    data: *mut u8,
    _: usize,
    outcome: *mut DrAdapterOutcomeV1,
    _: *mut DrInterfaceCarrierV1,
) -> u8 {
    (*observed_stream(context)).pending = false;
    let (tag, _, native, _, _) = pending_callback_facts(view);
    if tag == 4 {
        return 4;
    }
    assert!(matches!(tag, 0 | 2));
    *outcome = DrAdapterOutcomeV1 {
        tag,
        count: (*native).value as usize,
        has_count: u8::from(tag == 0),
    };
    for byte in core::slice::from_raw_parts_mut(data, (*native).value as usize) {
        *byte = byte.to_ascii_uppercase();
    }
    0
}

struct Fixture {
    directory: std::path::PathBuf,
    drops: RefCell<Vec<usize>>,
}

impl Fixture {
    fn new() -> Self {
        static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);
        let directory = std::env::temp_dir().join(format!(
            "doria-owned-adapter-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
        ));
        fs::create_dir(&directory).unwrap();
        Self {
            directory,
            drops: RefCell::new(Vec::new()),
        }
    }

    unsafe fn stream(&self, id: usize, table: &DrInterfaceVtableV1) -> DrInterfaceCarrierV1 {
        let path = self.directory.join(format!("{id}.bin"));
        fs::write(&path, b"ab").unwrap();
        self.stream_path(id, table, &path)
    }

    unsafe fn stream_path(
        &self,
        id: usize,
        table: &DrInterfaceVtableV1,
        path: &std::path::Path,
    ) -> DrInterfaceCarrierV1 {
        let path = path.to_str().unwrap().as_bytes();
        let mut file = mem::MaybeUninit::<OwnedFile>::uninit();
        let mut status = DrIoStatusV1::default();
        assert_eq!(
            dr_v1_file_open(
                path.as_ptr(),
                path.len(),
                0,
                0,
                0,
                file.as_mut_ptr().cast(),
                &mut status
            ),
            0
        );
        let path = crate::dr_v1_string_from_utf8(path.as_ptr(), path.len());
        let stream = Box::into_raw(Box::new(Stream {
            file: file.assume_init(),
            path,
            source: mem::MaybeUninit::uninit(),
            drops: &self.drops,
            id,
            calls: 0,
            expected_descriptor: ptr::null(),
            expected_drop_frame: ptr::null(),
            dispatch_calls: 0,
            written: [0; 2],
            pending: false,
            completion_error: None,
        }));
        ReadinessProjection::initialize_file(
            (*stream).source.as_mut_ptr(),
            stream.cast(),
            ptr::addr_of_mut!((*stream).file),
            path,
        );
        DrInterfaceCarrierV1 {
            object: stream.cast(),
            vtable: table,
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.directory).unwrap();
    }
}

unsafe fn new_set(pending: bool) -> *mut c_void {
    let mut status = DrIoStatusV1::default();
    let set = if pending {
        dr_v1_pending_transfer_adapter_set_new(core::ptr::null(), ptr::null_mut(), &mut status)
    } else {
        dr_v1_transfer_adapter_set_new(core::ptr::null(), ptr::null_mut(), &mut status)
    };
    assert!(!set.is_null());
    set
}

unsafe fn register(
    pending: bool,
    set: *mut c_void,
    value: &mut DrInterfaceCarrierV1,
    source: *const c_void,
) -> (u8, PlanKey) {
    register_in_frame(ptr::null(), pending, set, value, source)
}

unsafe fn register_in_frame(
    frame: *const DrStackFrameV2,
    pending: bool,
    set: *mut c_void,
    value: &mut DrInterfaceCarrierV1,
    source: *const c_void,
) -> (u8, PlanKey) {
    let mut status = DrIoStatusV1::default();
    let mut key = PlanKey { owner: 0, value: 0 };
    let output = (&mut key as *mut PlanKey).cast();
    let result = if pending {
        let adapter = DrPendingTransferAdapterV1 {
            context: DrAdapterContextV1 {
                object: value.object.cast(),
                descriptor: core::ptr::null(),
            },
            prepare_read: Some(prepare),
            prepare_write: None,
            complete_read: Some(complete),
            complete_write: None,
        };
        dr_v1_pending_transfer_adapter_set_add_owned(
            frame,
            set,
            source,
            Some(resolve_source),
            &adapter,
            value,
            output,
            &mut status,
        )
    } else {
        let adapter = DrTransferAdapterV1 {
            context: DrAdapterContextV1 {
                object: value.object.cast(),
                descriptor: core::ptr::null(),
            },
            read: Some(read),
            write: None,
        };
        dr_v1_transfer_adapter_set_add_owned(
            frame,
            set,
            source,
            Some(resolve_source),
            &adapter,
            value,
            output,
            &mut status,
        )
    };
    assert!(value.object.is_null() && value.vtable.is_null());
    (result, key)
}

unsafe fn source(value: &DrInterfaceCarrierV1) -> *const c_void {
    (*value.object.cast::<Stream>())
        .source
        .assume_init_ref()
        .value()
}

unsafe extern "C" fn resolve_source(
    _: *const DrStackFrameV2,
    home: *const DrInterfaceCarrierV1,
    source_out: *mut *const c_void,
    context_out: *mut DrAdapterContextV1,
    _: *mut DrInterfaceCarrierV1,
) -> u8 {
    let stream = (*home).object.cast::<Stream>();
    *source_out = source(&*home);
    *context_out = DrAdapterContextV1 {
        object: stream.cast(),
        descriptor: (*stream).expected_descriptor,
    };
    0
}

unsafe fn remove(set: *mut c_void, key: &PlanKey, value: &mut DrInterfaceCarrierV1) -> u8 {
    let mut status = DrIoStatusV1::default();
    dr_v1_transfer_adapter_set_remove_owned(set, (key as *const PlanKey).cast(), value, &mut status)
}

unsafe extern "C" fn prepare_buffered_write(
    _: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    data: *const u8,
    length: usize,
    _: *mut DrAdapterOutcomeV1,
    _: *mut DrInterfaceCarrierV1,
) -> u8 {
    let stream = &mut *observed_stream(context);
    stream.calls += 1;
    if length == stream.written.len() {
        stream
            .written
            .copy_from_slice(core::slice::from_raw_parts(data, length));
    }
    stream.pending = true;
    dr_v1_pending_request_complete(view, 0, length);
    0
}

unsafe extern "C" fn complete_buffered_write(
    _: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    view: *mut c_void,
    _: *const u8,
    _: usize,
    outcome: *mut DrAdapterOutcomeV1,
    _: *mut DrInterfaceCarrierV1,
) -> u8 {
    (*observed_stream(context)).pending = false;
    let (tag, _, _, _, immediate) = pending_callback_facts(view);
    if tag == 4 {
        return 4;
    }
    assert!(!immediate.is_null());
    *outcome = *immediate;
    0
}

unsafe extern "C" fn buffered_write(
    _: *const DrStackFrameV2,
    context: *const DrAdapterContextV1,
    data: *const u8,
    length: usize,
    outcome: *mut DrAdapterOutcomeV1,
    _: *mut DrInterfaceCarrierV1,
) -> u8 {
    let stream = &mut *observed_stream(context);
    stream.calls += 1;
    assert_eq!(length, stream.written.len());
    stream
        .written
        .copy_from_slice(core::slice::from_raw_parts(data, length));
    *outcome = DrAdapterOutcomeV1 {
        tag: 0,
        count: length,
        has_count: 1,
    };
    0
}

// Ordinary interface prefix followed by typed dispatch slots. The callback
// must recover these slots from the original erased descriptor, not a source
// projection, a type name, or a replacement interface descriptor.
#[repr(C)]
struct ErasedDispatchTable {
    base: DrInterfaceVtableV1,
    read: ReadCallback,
    write: WriteCallback,
    prepare_read: PrepareReadCallback,
    prepare_write: PrepareWriteCallback,
    complete_read: CompleteReadCallback,
    complete_write: CompleteWriteCallback,
}

macro_rules! erased_callback {
    ($name:ident, $slot:ident, $($argument:ident: $ty:ty),+) => {
        unsafe extern "C" fn $name(
            frame: *const DrStackFrameV2,
            context: *const DrAdapterContextV1,
            $($argument: $ty,)+
            outcome: *mut DrAdapterOutcomeV1,
            error: *mut DrInterfaceCarrierV1,
        ) -> u8 {
            let table = &*(*context).descriptor.cast::<ErasedDispatchTable>();
            (table.$slot)(frame, context, $($argument,)+ outcome, error)
        }
    };
}

erased_callback!(erased_read, read, data: *mut u8, length: usize);
erased_callback!(erased_write, write, data: *const u8, length: usize);
erased_callback!(erased_prepare_read, prepare_read, view: *mut c_void, data: *mut u8, length: usize);
erased_callback!(erased_prepare_write, prepare_write, view: *mut c_void, data: *const u8, length: usize);
erased_callback!(erased_complete_read, complete_read, view: *mut c_void, data: *mut u8, length: usize);
erased_callback!(erased_complete_write, complete_write, view: *mut c_void, data: *const u8, length: usize);

unsafe fn exercise_owned_erased_callbacks(pending: bool, checked: bool) {
    let fixture = Fixture::new();
    let mut table = ErasedDispatchTable {
        base: descriptor(),
        read,
        write: buffered_write,
        prepare_read: prepare,
        prepare_write: prepare_buffered_write,
        complete_read: complete,
        complete_write: complete_buffered_write,
    };
    table.base.type_name = b"Renamed\\ErasedSurface".as_ptr();
    table.base.type_name_length = b"Renamed\\ErasedSurface".len();
    let mut status = DrIoStatusV1::default();
    let set = if pending {
        dr_v1_readiness_set_new(ptr::null(), &mut status)
    } else {
        new_set(false)
    };
    assert!(!set.is_null());
    let mut members = Vec::new();
    for id in 1..=3 {
        let mut value = fixture.stream(id, &table.base);
        let original = (value.object, value.vtable);
        let object = value.object.cast::<Stream>();
        let context = DrAdapterContextV1 {
            object: value.object.cast(),
            descriptor: value.vtable.cast(),
        };
        (*object).expected_descriptor = context.descriptor;
        let source = source(&value);
        assert_ne!(context.descriptor, source);
        let mut key = PlanKey { owner: 0, value: 0 };
        let tag = if pending {
            let adapter = DrPendingTransferAdapterV1 {
                context,
                prepare_read: Some(erased_prepare_read),
                prepare_write: Some(erased_prepare_write),
                complete_read: Some(erased_complete_read),
                complete_write: Some(erased_complete_write),
            };
            dr_v1_readiness_set_add_owned(
                ptr::null(),
                set,
                source,
                Some(resolve_source),
                if id == 3 { ptr::null() } else { &adapter },
                &mut value,
                ptr::from_mut(&mut key).cast(),
                &mut status,
            )
        } else {
            let adapter = DrTransferAdapterV1 {
                context,
                read: Some(erased_read),
                write: Some(erased_write),
            };
            dr_v1_transfer_adapter_set_add_owned(
                ptr::null(),
                set,
                source,
                Some(resolve_source),
                &adapter,
                &mut value,
                ptr::from_mut(&mut key).cast(),
                &mut status,
            )
        };
        assert_eq!(tag, 0);
        assert!(value.object.is_null() && value.vtable.is_null());
        assert_eq!((*object).dispatch_calls, 0);
        members.push((key, original, object));
        // Each stack adapter/context home expires here. Only borrowed words
        // are copied into the existing registration; the original T is retained.
    }
    let mut bytes = [0; 2];
    let writes = *b"xy";
    let descriptors = [
        DrTransferBufferV1 {
            data: bytes.as_mut_ptr(),
            length: bytes.len(),
            reading: 1,
        },
        DrTransferBufferV1 {
            data: writes.as_ptr().cast_mut(),
            length: writes.len(),
            reading: 0,
        },
    ];
    let plan = plan::dr_v1_transfer_plan_new(
        ptr::null(),
        ptr::from_ref(&members[0].0).cast(),
        &descriptors[0],
        &mut status,
    );
    assert!(!plan.is_null());
    let plan = plan::dr_v1_transfer_plan_append(
        ptr::null(),
        plan,
        ptr::from_ref(&members[1].0).cast(),
        &descriptors[1],
        &mut status,
    );
    assert!(!plan.is_null());
    let operation_keys = [
        (*plan.cast::<TransferPlan<'static>>())
            .operation_key(0, true)
            .unwrap(),
        (*plan.cast::<TransferPlan<'static>>())
            .operation_key(1, false)
            .unwrap(),
    ];
    let budget = immediate_budget();
    let mut error = crate::empty_error_carrier();
    let group = if pending {
        dr_v1_pending_transfer_adapter_plan_begin(
            ptr::null(),
            set,
            plan,
            &budget,
            &mut error,
            &mut status,
        )
    } else {
        dr_v1_transfer_adapter_plan_begin(ptr::null(), set, plan, &budget, &mut error, &mut status)
    };
    assert!(!group.is_null());
    let mut seen = [false; 2];
    for _ in 0..2 {
        if checked {
            let observation = checked_observation_tests::fetch(group, 1, 0, 0, &mut error);
            assert_eq!(
                (observation.tag, observation.allocations, observation.result),
                (0, 0, 0)
            );
            let index = observation.key.position as usize;
            assert!(index < seen.len() && !seen[index]);
            assert_eq!(observation.key, operation_keys[index]);
            assert_eq!(
                (observation.reading, observation.outcome, observation.count),
                (u8::from(index == 0), 1, 2)
            );
            observation.fields.assert_success();
            assert_eq!(
                (
                    observation.control.stop,
                    observation.control.has_cancellation_failure
                ),
                (0, 0)
            );
            seen[index] = true;
            continue;
        }
        let (mut key, mut index) = (0, usize::MAX);
        let mut outcome = DrAdapterOutcomeV1::default();
        let mut control = DrCompletionStatusV1::default();
        let (tag, allocations) = crate::probe_runtime_allocations(|| {
            dr_v1_transfer_adapter_group_next(
                ptr::null(),
                group,
                &mut key,
                &mut index,
                &mut outcome,
                &mut error,
                ptr::from_mut(&mut control).cast(),
            )
        });
        assert_eq!((tag, allocations), (0, 0));
        assert!(index < seen.len() && !seen[index]);
        assert_eq!(
            (key, outcome.tag, outcome.count),
            (members[index].0.value, 0, 2)
        );
        assert!(error.object.is_null() && error.vtable.is_null());
        seen[index] = true;
    }
    if checked {
        assert!(super::tests::observed(group).is_some());
        let rejected = checked_observation_tests::fetch(group, 255, 0, 0, &mut error);
        assert_eq!((rejected.tag, rejected.allocations), (1, 1));
        rejected.fields.assert_failure(
            doria_diagnostic_catalogue::IoMessageOperation::Wait,
            Reason::InvalidInput,
        );
        assert_eq!(super::tests::observed(group), None);
        let exhausted = checked_observation_tests::fetch(group, 1, 0, 0, &mut error);
        assert_eq!(
            (exhausted.tag, exhausted.allocations, exhausted.result),
            (0, 0, 2)
        );
        assert_eq!(
            (
                exhausted.key.owner,
                exhausted.key.position,
                exhausted.outcome,
                exhausted.count
            ),
            (0, 0, 0, 0)
        );
        exhausted.fields.assert_success();
        assert_eq!(super::tests::observed(group), None);
    }
    dr_v1_transfer_adapter_group_drop(ptr::null(), group);
    assert_eq!(bytes, *b"AB");
    assert_eq!((*members[1].2).written, writes);
    let expected_calls = if pending { 2 } else { 1 };
    assert_eq!(
        (
            (*members[0].2).dispatch_calls,
            (*members[1].2).dispatch_calls
        ),
        (expected_calls, expected_calls)
    );
    assert_eq!((*members[2].2).dispatch_calls, 0);
    assert!(members.iter().all(|(_, _, object)| !(**object).pending));
    let mut returned = crate::empty_error_carrier();
    let (tag, allocations) =
        crate::probe_runtime_allocations(|| remove(set, &members[0].0, &mut returned));
    assert_eq!((tag, allocations), (0, 0));
    assert_eq!((returned.object, returned.vtable), members[0].1);
    drop(OwnedInterfaceValue::from_carrier(returned));
    dr_v1_transfer_adapter_set_drop(ptr::null(), set);
    assert_eq!(*fixture.drops.borrow(), [1, 3, 2]);
}

#[test]
fn owned_erased_sync_callbacks_preserve_dispatch_descriptor_and_original_carriers() {
    unsafe { exercise_owned_erased_callbacks(false, false) }
}

#[test]
fn owned_erased_pending_callbacks_preserve_dispatch_descriptor_and_retire_before_drop() {
    unsafe { exercise_owned_erased_callbacks(true, false) }
}

unsafe fn readiness_register(
    set: *mut c_void,
    value: &mut DrInterfaceCarrierV1,
    reading: bool,
    writing: bool,
) -> (u8, PlanKey) {
    let source = source(value);
    let adapter = DrPendingTransferAdapterV1 {
        context: DrAdapterContextV1 {
            object: value.object.cast(),
            descriptor: core::ptr::null(),
        },
        prepare_read: reading.then_some(prepare),
        prepare_write: writing.then_some(prepare_buffered_write),
        complete_read: reading.then_some(complete),
        complete_write: writing.then_some(complete_buffered_write),
    };
    let mut key = PlanKey { owner: 0, value: 0 };
    let mut status = DrIoStatusV1::default();
    let tag = dr_v1_readiness_set_add_owned(
        ptr::null(),
        set,
        source,
        Some(resolve_source),
        if reading || writing {
            &adapter
        } else {
            ptr::null()
        },
        value,
        (&mut key as *mut PlanKey).cast(),
        &mut status,
    );
    assert!(value.object.is_null() && value.vtable.is_null());
    (tag, key)
}

unsafe fn readiness_remove(
    set: *mut c_void,
    key: &PlanKey,
    value: &mut DrInterfaceCarrierV1,
) -> u8 {
    let mut status = DrIoStatusV1::default();
    dr_v1_readiness_set_remove_owned(set, (key as *const PlanKey).cast(), value, &mut status)
}

unsafe fn immediate_budget() -> DrWaitBudgetV1 {
    let mut budget = mem::MaybeUninit::uninit();
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
    budget.assume_init()
}

#[test]
fn owned_set_cleanup_uses_current_not_acquisition_frame() {
    unsafe {
        let acquired = diagnostic_frame(b"acquired");
        let cleanup = diagnostic_frame(b"cleanup");
        for mode in 0..3 {
            let fixture = Fixture::new();
            let table = descriptor();
            let mut status = DrIoStatusV1::default();
            let set = match mode {
                0 => dr_v1_transfer_adapter_set_new(&acquired, ptr::null_mut(), &mut status),
                1 => {
                    dr_v1_pending_transfer_adapter_set_new(&acquired, ptr::null_mut(), &mut status)
                }
                _ => dr_v1_readiness_set_new(&acquired, &mut status),
            };
            assert!(!set.is_null());
            for id in [1, 2] {
                let mut value = fixture.stream(id, &table);
                (*value.object.cast::<Stream>()).expected_drop_frame = &cleanup;
                let source = source(&value);
                let tag = if mode == 2 {
                    let mut key = PlanKey { owner: 0, value: 0 };
                    dr_v1_readiness_set_add_owned(
                        &acquired,
                        set,
                        source,
                        Some(resolve_source),
                        ptr::null(),
                        &mut value,
                        ptr::from_mut(&mut key).cast(),
                        &mut status,
                    )
                } else {
                    register_in_frame(&acquired, mode == 1, set, &mut value, source).0
                };
                assert_eq!(tag, 0);
                assert!(value.object.is_null());
            }
            assert!(fixture.drops.borrow().is_empty());
            let ((), allocations) =
                crate::probe_runtime_allocations(|| dr_v1_transfer_adapter_set_drop(&cleanup, set));
            assert_eq!(allocations, 0);
            assert_eq!(*fixture.drops.borrow(), [2, 1]);
        }
    }
}

#[test]
fn owned_admission_rejection_uses_current_frame_before_and_after_staging() {
    unsafe {
        let frame = diagnostic_frame(b"rejected");
        for pending in [false, true] {
            let fixture = Fixture::new();
            let table = descriptor();
            let set = new_set(pending);
            for (id, owner) in [(1, ptr::null_mut()), (2, set)] {
                let mut value = fixture.stream(id, &table);
                (*value.object.cast::<Stream>()).expected_drop_frame = &frame;
                assert_ne!(
                    register_in_frame(&frame, pending, owner, &mut value, ptr::null()).0,
                    0
                );
                assert!(value.object.is_null() && value.vtable.is_null());
            }
            assert_eq!(*fixture.drops.borrow(), [1, 2]);
            assert_eq!((*set.cast::<AdapterSetOwner>()).retained.occupied_len(), 0);
            dr_v1_transfer_adapter_set_drop(ptr::null(), set);
        }
    }
}

fn diagnostic_frame(name: &[u8]) -> DrStackFrameV2 {
    DrStackFrameV2 {
        parent: ptr::null(),
        function_name: name.as_ptr(),
        function_name_length: name.len(),
        source_path: b"owned-stream-test".as_ptr(),
        source_path_length: b"owned-stream-test".len(),
        source_text: b"".as_ptr(),
        source_text_length: 0,
        function_span_start: 0,
        function_span_end: 0,
        active_span_start: 0,
        active_span_end: 0,
    }
}

#[test]
fn readiness_constructor_allocates_only_one_empty_owner_without_native_acquisition() {
    unsafe {
        let mut status = DrIoStatusV1::default();
        let (set, allocations) = crate::probe_runtime_allocations(|| {
            dr_v1_readiness_set_new(core::ptr::null(), &mut status)
        });
        assert!(!set.is_null());
        assert_eq!(allocations, 1);
        assert_eq!(
            (
                status.value,
                status.reason,
                status.has_system_code,
                status.has_transferred_byte_count
            ),
            (0, 0, 0, 0)
        );
        let header = set.cast::<AdapterSetOwner>();
        assert!((*header).readiness && !(*header).active);
        assert_eq!((*header).retained.occupied_len(), 0);
        let AdapterSetMode::Pending(native) = AdapterSetOwner::native(header) else {
            unreachable!()
        };
        assert!(!native.initialized());
        let ((), allocations) =
            crate::probe_runtime_allocations(|| dr_v1_readiness_set_drop(ptr::null(), set));
        assert_eq!(allocations, 0);
    }
}

#[test]
fn checked_readiness_construction_returns_the_same_lazy_owner_through_shared_io_transport() {
    unsafe {
        let (mut owner, mut message, mut path) =
            (ptr::null_mut(), ptr::null_mut(), ptr::null_mut());
        let (mut system_code, mut transferred, mut meta) = (0, 0, 0);
        let (mut valid, mut invalid) = (17, 19);
        let (tag, allocations) = crate::probe_runtime_allocations(|| {
            crate::checked_io::dr_v4_checked_io(
                ptr::null(),
                crate::checked_io::READINESS_SET_NEW,
                ptr::null_mut(),
                ptr::null(),
                0,
                &mut owner,
                &mut message,
                &mut path,
                &mut system_code,
                &mut valid,
                &mut invalid,
                &mut transferred,
                &mut meta,
            )
        });
        assert_eq!((tag, allocations), (0, 1));
        assert!(!owner.is_null());
        assert!(message.is_null() && path.is_null());
        assert_eq!(
            (system_code, valid, invalid, transferred, meta),
            (0, 0, 0, 0, 0)
        );
        let header = owner.cast::<AdapterSetOwner>();
        assert!((*header).readiness && !(*header).active);
        let AdapterSetMode::Pending(native) = AdapterSetOwner::native(header) else {
            unreachable!()
        };
        assert!(!native.initialized());
        dr_v1_readiness_set_drop(ptr::null(), owner);
    }
}

struct CheckedFields {
    message: *mut crate::DrStringV1,
    path: *mut crate::DrStringV1,
    system_code: i64,
    transferred: usize,
    meta: u64,
}

impl CheckedFields {
    fn empty() -> Self {
        Self {
            message: ptr::null_mut(),
            path: ptr::null_mut(),
            system_code: -7,
            transferred: usize::MAX,
            meta: u64::MAX,
        }
    }

    fn assert_success(&self) {
        assert!(self.message.is_null() && self.path.is_null());
        assert_eq!((self.system_code, self.transferred, self.meta), (0, 0, 0));
    }

    unsafe fn assert_failure(
        &self,
        operation: doria_diagnostic_catalogue::IoMessageOperation,
        reason: Reason,
    ) {
        assert!(!self.message.is_null());
        assert!(self.path.is_null());
        assert_eq!((self.system_code, self.transferred), (0, 0));
        // Existing checked-I/O encoding: ReadinessSet target 4, ordinary IoError
        // kind 0, no system-code, UTF-8, or byte-transfer-progress presence bits.
        assert_eq!(
            self.meta,
            operation as u64 | (4 << 8) | ((reason as u64) << 16)
        );
        let message = core::slice::from_raw_parts(
            crate::string_bytes(self.message),
            (*self.message).byte_length,
        );
        let message = core::str::from_utf8(message).unwrap();
        assert!(message.contains("readiness set"), "{message}");
        assert!(message.contains(operation.word()), "{message}");
    }
}

impl Drop for CheckedFields {
    fn drop(&mut self) {
        unsafe {
            crate::dr_v1_string_release(self.message);
            crate::dr_v1_string_release(self.path);
        }
    }
}

unsafe fn checked_add(
    set: *mut c_void,
    source: *const c_void,
    adapter: *const DrPendingTransferAdapterV1,
    value: *mut DrInterfaceCarrierV1,
    key: *mut c_void,
    fields: &mut CheckedFields,
) -> u8 {
    dr_v1_checked_readiness_set_add_owned(
        ptr::null(),
        set,
        source,
        Some(resolve_source),
        adapter,
        value,
        key,
        &mut fields.message,
        &mut fields.path,
        &mut fields.system_code,
        &mut fields.transferred,
        &mut fields.meta,
    )
}

unsafe fn checked_remove(
    set: *mut c_void,
    key: *const c_void,
    value: *mut DrInterfaceCarrierV1,
    fields: &mut CheckedFields,
) -> u8 {
    dr_v1_checked_readiness_set_remove_owned(
        ptr::null(),
        set,
        key,
        value,
        &mut fields.message,
        &mut fields.path,
        &mut fields.system_code,
        &mut fields.transferred,
        &mut fields.meta,
    )
}

#[test]
fn checked_membership_roundtrip_preserves_original_carriers_and_mixed_capabilities() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let set = dr_v1_readiness_set_new(ptr::null(), &mut DrIoStatusV1::default());
        let mut members = Vec::new();
        for (id, reading, writing) in [
            (1, false, false),
            (2, true, false),
            (3, false, true),
            (4, true, true),
        ] {
            let mut value = fixture.stream(id, &table);
            let original = (value.object, value.vtable);
            let adapter = DrPendingTransferAdapterV1 {
                context: DrAdapterContextV1 {
                    object: value.object.cast(),
                    descriptor: core::ptr::null(),
                },
                prepare_read: reading.then_some(prepare),
                prepare_write: writing.then_some(prepare_buffered_write),
                complete_read: reading.then_some(complete),
                complete_write: writing.then_some(complete_buffered_write),
            };
            let source = source(&value);
            let mut key = PlanKey { owner: 0, value: 0 };
            let mut fields = CheckedFields::empty();
            assert_eq!(
                checked_add(
                    set,
                    source,
                    if reading || writing {
                        &adapter
                    } else {
                        ptr::null()
                    },
                    &mut value,
                    ptr::from_mut(&mut key).cast(),
                    &mut fields
                ),
                0
            );
            fields.assert_success();
            assert!(value.object.is_null() && value.vtable.is_null());
            assert_eq!((*original.0.cast::<Stream>()).calls, 0);
            members.push((key, original));
        }
        for (key, original) in members {
            let mut returned = crate::empty_error_carrier();
            let mut fields = CheckedFields::empty();
            let (tag, allocations) = crate::probe_runtime_allocations(|| {
                checked_remove(set, ptr::from_ref(&key).cast(), &mut returned, &mut fields)
            });
            assert_eq!((tag, allocations), (0, 0));
            fields.assert_success();
            assert_eq!((returned.object, returned.vtable), original);
            drop(OwnedInterfaceValue::from_carrier(returned));
        }
        dr_v1_readiness_set_drop(ptr::null(), set);
        assert_eq!(*fixture.drops.borrow(), [1, 2, 3, 4]);
    }
}

#[test]
fn checked_admission_rejection_consumes_and_cleans_the_taken_complete_value() {
    unsafe {
        for case in 0..5 {
            let fixture = Fixture::new();
            let table = descriptor();
            let set = dr_v1_readiness_set_new(ptr::null(), &mut DrIoStatusV1::default());
            let mut value = fixture.stream(1, &table);
            let source = source(&value);
            let malformed = DrPendingTransferAdapterV1 {
                context: DrAdapterContextV1 {
                    object: if case == 4 {
                        ptr::null_mut()
                    } else {
                        value.object.cast()
                    },
                    descriptor: core::ptr::null(),
                },
                prepare_read: Some(prepare),
                prepare_write: None,
                complete_read: (case == 4).then_some(complete),
                complete_write: None,
            };
            let mut key = PlanKey {
                owner: usize::MAX,
                value: u64::MAX,
            };
            let mut fields = CheckedFields::empty();
            assert_eq!(
                checked_add(
                    if case == 0 { ptr::null_mut() } else { set },
                    if case == 1 { ptr::null() } else { source },
                    if case >= 3 { &malformed } else { ptr::null() },
                    &mut value,
                    if case == 2 {
                        ptr::null_mut()
                    } else {
                        ptr::from_mut(&mut key).cast()
                    },
                    &mut fields
                ),
                1
            );
            fields.assert_failure(
                doria_diagnostic_catalogue::IoMessageOperation::Register,
                Reason::InvalidInput,
            );
            assert!(value.object.is_null() && value.vtable.is_null());
            if case != 2 {
                assert_eq!(key, PlanKey { owner: 0, value: 0 });
            }
            assert_eq!(*fixture.drops.borrow(), [1]);
            assert_eq!((*set.cast::<AdapterSetOwner>()).retained.occupied_len(), 0);
            let AdapterSetMode::Pending(native) = AdapterSetOwner::native(set.cast()) else {
                unreachable!()
            };
            assert!(!native.initialized());
            dr_v1_readiness_set_drop(ptr::null(), set);
            assert_eq!(*fixture.drops.borrow(), [1]);
        }
    }
}

#[test]
fn checked_registration_counter_exhaustion_preserves_actual_resource_failure() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let set = dr_v1_readiness_set_new(ptr::null(), &mut DrIoStatusV1::default());
        (*set.cast::<AdapterSetOwner>()).next_registration = u64::MAX - 1;
        let mut value = fixture.stream(1, &table);
        let source = source(&value);
        let mut key = PlanKey { owner: 0, value: 0 };
        let mut fields = CheckedFields::empty();
        assert_eq!(
            checked_add(
                set,
                source,
                ptr::null(),
                &mut value,
                ptr::from_mut(&mut key).cast(),
                &mut fields
            ),
            1
        );
        fields.assert_failure(
            doria_diagnostic_catalogue::IoMessageOperation::Register,
            Reason::ResourceExhausted,
        );
        assert!(value.object.is_null() && value.vtable.is_null());
        assert_eq!(key, PlanKey { owner: 0, value: 0 });
        assert_eq!(
            (*set.cast::<AdapterSetOwner>()).next_registration,
            u64::MAX - 1
        );
        assert_eq!(*fixture.drops.borrow(), [1]);
        dr_v1_readiness_set_drop(ptr::null(), set);
    }
}

#[test]
fn checked_removal_rejection_preserves_membership_and_original_output_carrier() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let set = dr_v1_readiness_set_new(ptr::null(), &mut DrIoStatusV1::default());
        let other = dr_v1_readiness_set_new(ptr::null(), &mut DrIoStatusV1::default());
        let mut value = fixture.stream(1, &table);
        let original = (value.object, value.vtable);
        let (tag, key) = readiness_register(set, &mut value, false, false);
        assert_eq!(tag, 0);
        let stale = PlanKey {
            owner: key.owner,
            value: 0,
        };
        for (owner, supplied) in [
            (ptr::null_mut(), ptr::from_ref(&key).cast()),
            (other, ptr::from_ref(&key).cast()),
            (set, ptr::null()),
            (set, ptr::from_ref(&stale).cast()),
        ] {
            let mut fields = CheckedFields::empty();
            assert_eq!(checked_remove(owner, supplied, &mut value, &mut fields), 1);
            fields.assert_failure(
                doria_diagnostic_catalogue::IoMessageOperation::Unregister,
                Reason::InvalidInput,
            );
            assert!(value.object.is_null() && value.vtable.is_null());
            assert!(fixture.drops.borrow().is_empty());
        }
        let mut fields = CheckedFields::empty();
        assert_eq!(
            checked_remove(
                set,
                ptr::from_ref(&key).cast(),
                ptr::null_mut(),
                &mut fields
            ),
            1
        );
        fields.assert_failure(
            doria_diagnostic_catalogue::IoMessageOperation::Unregister,
            Reason::InvalidInput,
        );
        assert!(fixture.drops.borrow().is_empty());
        let mut occupied = fixture.stream(9, &table);
        let occupied_original = (occupied.object, occupied.vtable);
        let mut fields = CheckedFields::empty();
        assert_eq!(
            checked_remove(set, ptr::from_ref(&key).cast(), &mut occupied, &mut fields),
            1
        );
        fields.assert_failure(
            doria_diagnostic_catalogue::IoMessageOperation::Unregister,
            Reason::InvalidInput,
        );
        assert_eq!((occupied.object, occupied.vtable), occupied_original);
        drop(OwnedInterfaceValue::from_carrier(occupied));
        let mut fields = CheckedFields::empty();
        assert_eq!(
            checked_remove(set, ptr::from_ref(&key).cast(), &mut value, &mut fields),
            0
        );
        fields.assert_success();
        assert_eq!((value.object, value.vtable), original);
        dr_v1_readiness_set_drop(ptr::null(), set);
        dr_v1_readiness_set_drop(ptr::null(), other);
        assert_eq!(*fixture.drops.borrow(), [9]);
        drop(OwnedInterfaceValue::from_carrier(value));
        assert_eq!(*fixture.drops.borrow(), [9, 1]);
    }
}

#[test]
fn checked_removed_key_remains_stale_after_slot_reuse_and_cleanup_stays_ordered() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let set = dr_v1_readiness_set_new(ptr::null(), &mut DrIoStatusV1::default());
        let mut keys = Vec::new();
        for id in 1..=2 {
            let mut value = fixture.stream(id, &table);
            let source = source(&value);
            let mut key = PlanKey { owner: 0, value: 0 };
            let mut fields = CheckedFields::empty();
            assert_eq!(
                checked_add(
                    set,
                    source,
                    ptr::null(),
                    &mut value,
                    ptr::from_mut(&mut key).cast(),
                    &mut fields
                ),
                0
            );
            fields.assert_success();
            keys.push(key);
        }
        let mut value = crate::empty_error_carrier();
        let mut fields = CheckedFields::empty();
        assert_eq!(
            checked_remove(set, ptr::from_ref(&keys[0]).cast(), &mut value, &mut fields),
            0
        );
        drop(OwnedInterfaceValue::from_carrier(value));
        let mut value = fixture.stream(3, &table);
        let source = source(&value);
        let mut newer = PlanKey { owner: 0, value: 0 };
        assert_eq!(
            checked_add(
                set,
                source,
                ptr::null(),
                &mut value,
                ptr::from_mut(&mut newer).cast(),
                &mut fields
            ),
            0
        );
        assert!(newer.value > keys[1].value);
        assert_eq!(
            checked_remove(set, ptr::from_ref(&keys[0]).cast(), &mut value, &mut fields),
            1
        );
        fields.assert_failure(
            doria_diagnostic_catalogue::IoMessageOperation::Unregister,
            Reason::InvalidInput,
        );
        assert!(value.object.is_null() && value.vtable.is_null());
        dr_v1_readiness_set_drop(ptr::null(), set);
        assert_eq!(*fixture.drops.borrow(), [1, 3, 2]);
    }
}

#[test]
fn checked_active_membership_rejection_preserves_retirement_before_owner_cleanup() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let set = dr_v1_readiness_set_new(ptr::null(), &mut DrIoStatusV1::default());
        let mut value = fixture.stream(1, &table);
        let original = (value.object, value.vtable);
        let (tag, key) = readiness_register(set, &mut value, true, false);
        assert_eq!(tag, 0);
        let mut bytes = [0; 2];
        let buffer = DrTransferBufferV1 {
            data: bytes.as_mut_ptr(),
            length: 2,
            reading: 1,
        };
        let mut status = DrIoStatusV1::default();
        let plan = plan::dr_v1_transfer_plan_new(
            ptr::null(),
            ptr::from_ref(&key).cast(),
            &buffer,
            &mut status,
        );
        let mut error = crate::empty_error_carrier();
        let group = super::tests::checked_begin(ptr::null(), set, plan, &mut error, 0);
        assert!(!group.is_null());
        let mut fields = CheckedFields::empty();
        assert_eq!(
            checked_remove(set, ptr::from_ref(&key).cast(), &mut value, &mut fields),
            1
        );
        fields.assert_failure(
            doria_diagnostic_catalogue::IoMessageOperation::Unregister,
            Reason::InvalidInput,
        );
        assert!(value.object.is_null() && value.vtable.is_null());
        let mut rejected = fixture.stream(2, &table);
        let source = source(&rejected);
        let mut rejected_key = PlanKey { owner: 0, value: 0 };
        let mut fields = CheckedFields::empty();
        assert_eq!(
            checked_add(
                set,
                source,
                ptr::null(),
                &mut rejected,
                ptr::from_mut(&mut rejected_key).cast(),
                &mut fields
            ),
            1
        );
        fields.assert_failure(
            doria_diagnostic_catalogue::IoMessageOperation::Register,
            Reason::InvalidInput,
        );
        assert!(rejected.object.is_null() && rejected.vtable.is_null());
        assert_eq!(*fixture.drops.borrow(), [2]);
        dr_v1_transfer_adapter_group_drop(ptr::null(), group);
        assert!(!(*original.0.cast::<Stream>()).pending);
        let mut fields = CheckedFields::empty();
        assert_eq!(
            checked_remove(set, ptr::from_ref(&key).cast(), &mut value, &mut fields),
            0
        );
        fields.assert_success();
        assert_eq!((value.object, value.vtable), original);
        drop(OwnedInterfaceValue::from_carrier(value));
        dr_v1_readiness_set_drop(ptr::null(), set);
        assert_eq!(*fixture.drops.borrow(), [2, 1]);
    }
}

#[test]
fn externally_owned_readiness_members_share_keys_and_retire_before_owner_cleanup() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let set = dr_v1_readiness_set_new(ptr::null(), &mut DrIoStatusV1::default());
        let other = dr_v1_readiness_set_new(ptr::null(), &mut DrIoStatusV1::default());
        let mut status = DrIoStatusV1::default();
        let first = fixture.stream(1, &table);
        let second = fixture.stream(2, &table);
        let mut key = PlanKey {
            owner: 99,
            value: 99,
        };
        assert_eq!(
            dr_v1_readiness_set_add_source(
                set,
                source(&first),
                ptr::from_mut(&mut key).cast(),
                &mut status
            ),
            0
        );
        assert!(fixture.drops.borrow().is_empty());
        assert_eq!((*first.object.cast::<Stream>()).calls, 0);
        assert_eq!(
            dr_v1_readiness_set_remove_source(other, ptr::from_ref(&key).cast(), &mut status),
            3
        );
        assert_eq!(status.failure(), Some(Failure::invalid_input()));
        let mut returned = crate::empty_error_carrier();
        assert_eq!(readiness_remove(set, &key, &mut returned), 3);
        let (tag, allocations) = crate::probe_runtime_allocations(|| {
            dr_v1_readiness_set_remove_source(set, ptr::from_ref(&key).cast(), &mut status)
        });
        assert_eq!((tag, allocations), (0, 0));
        assert_eq!(
            dr_v1_readiness_set_remove_source(set, ptr::from_ref(&key).cast(), &mut status),
            3
        );
        drop(OwnedInterfaceValue::from_carrier(first));
        let mut newer = PlanKey { owner: 0, value: 0 };
        assert_eq!(
            dr_v1_readiness_set_add_source(
                set,
                source(&second),
                ptr::from_mut(&mut newer).cast(),
                &mut status
            ),
            0
        );
        assert!(newer.value > key.value);
        dr_v1_readiness_set_drop(ptr::null(), set);
        dr_v1_readiness_set_drop(ptr::null(), other);
        assert_eq!(*fixture.drops.borrow(), [1]);
        drop(OwnedInterfaceValue::from_carrier(second));
        assert_eq!(*fixture.drops.borrow(), [1, 2]);
    }
}

#[test]
fn external_readiness_rejection_preserves_owners_and_cannot_remove_native_owned_members() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let set = dr_v1_readiness_set_new(ptr::null(), &mut DrIoStatusV1::default());
        let mut status = DrIoStatusV1::default();
        let value = fixture.stream(1, &table);
        let mut key = PlanKey {
            owner: 99,
            value: 99,
        };
        for source in [ptr::null(), source(&value)] {
            (*set.cast::<AdapterSetOwner>()).active = !source.is_null();
            assert_eq!(
                dr_v1_readiness_set_add_source(
                    set,
                    source,
                    ptr::from_mut(&mut key).cast(),
                    &mut status
                ),
                3
            );
            assert_eq!((key.owner, key.value), (0, 0));
            assert_eq!(status.failure(), Some(Failure::invalid_input()));
            assert!(fixture.drops.borrow().is_empty());
        }
        (*set.cast::<AdapterSetOwner>()).active = false;
        drop(OwnedInterfaceValue::from_carrier(value));
        let mut owned = fixture.stream(2, &table);
        let (tag, key) = readiness_register(set, &mut owned, false, false);
        assert_eq!(tag, 0);
        assert_eq!(
            dr_v1_readiness_set_remove_source(set, ptr::from_ref(&key).cast(), &mut status),
            3
        );
        dr_v1_readiness_set_drop(ptr::null(), set);
        assert_eq!(*fixture.drops.borrow(), [1, 2]);
    }
}

#[test]
fn readiness_only_read_write_and_duplex_members_coexist_and_return_exact_carriers() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let set = dr_v1_readiness_set_new(core::ptr::null(), &mut DrIoStatusV1::default());
        let mut members = Vec::new();
        for (id, reading, writing) in [
            (1, false, false),
            (2, true, false),
            (3, false, true),
            (4, true, true),
        ] {
            let mut value = fixture.stream(id, &table);
            let original = (value.object, value.vtable);
            let (tag, key) = readiness_register(set, &mut value, reading, writing);
            assert_eq!(tag, 0);
            assert_eq!((*original.0.cast::<Stream>()).calls, 0);
            members.push((key, original));
        }
        for (key, original) in members {
            let mut value = crate::empty_error_carrier();
            let (tag, allocations) =
                crate::probe_runtime_allocations(|| readiness_remove(set, &key, &mut value));
            assert_eq!((tag, allocations), (0, 0));
            assert_eq!((value.object, value.vtable), original);
            drop(OwnedInterfaceValue::from_carrier(value));
            let mut empty = crate::empty_error_carrier();
            assert_eq!(readiness_remove(set, &key, &mut empty), 3);
        }
        assert_eq!(*fixture.drops.borrow(), [1, 2, 3, 4]);
        dr_v1_readiness_set_drop(ptr::null(), set);
    }
}

#[test]
fn readiness_mixed_plan_selects_both_duplex_directions_without_dispatching_source_only_members() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let set = dr_v1_readiness_set_new(core::ptr::null(), &mut DrIoStatusV1::default());
        let mut members = Vec::new();
        for (id, reading, writing) in [
            (1, false, false),
            (2, true, false),
            (3, false, true),
            (4, true, true),
        ] {
            let mut value = fixture.stream(id, &table);
            let object = value.object.cast::<Stream>();
            let (tag, key) = readiness_register(set, &mut value, reading, writing);
            assert_eq!(tag, 0);
            members.push((key, object));
        }
        let mut first_read = [0; 2];
        let mut duplex_read = [0; 2];
        let writes = *b"xy";
        let descriptors = [
            DrTransferBufferV1 {
                data: first_read.as_mut_ptr(),
                length: 2,
                reading: 1,
            },
            DrTransferBufferV1 {
                data: writes.as_ptr().cast_mut(),
                length: 2,
                reading: 0,
            },
            DrTransferBufferV1 {
                data: duplex_read.as_mut_ptr(),
                length: 2,
                reading: 1,
            },
            DrTransferBufferV1 {
                data: writes.as_ptr().cast_mut(),
                length: 2,
                reading: 0,
            },
        ];
        let selected = [members[1].0, members[2].0, members[3].0, members[3].0];
        let mut status = DrIoStatusV1::default();
        let mut plan = plan::dr_v1_transfer_plan_new(
            core::ptr::null(),
            (&selected[0] as *const PlanKey).cast(),
            &descriptors[0],
            &mut status,
        );
        for index in 1..descriptors.len() {
            plan = plan::dr_v1_transfer_plan_append(
                core::ptr::null(),
                plan,
                (&selected[index] as *const PlanKey).cast(),
                &descriptors[index],
                &mut status,
            );
            assert!(!plan.is_null());
        }
        let budget = immediate_budget();
        let mut error = crate::empty_error_carrier();
        let group = dr_v1_pending_transfer_adapter_plan_begin(
            ptr::null(),
            set,
            plan,
            &budget,
            &mut error,
            &mut status,
        );
        assert!(!group.is_null());
        let mut seen = [false; 4];
        for _ in 0..selected.len() {
            let (mut key, mut index) = (0, usize::MAX);
            let mut outcome = DrAdapterOutcomeV1::default();
            let mut control = DrCompletionStatusV1::default();
            let (tag, allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_transfer_adapter_group_next(
                    ptr::null(),
                    group,
                    &mut key,
                    &mut index,
                    &mut outcome,
                    &mut error,
                    (&mut control as *mut DrCompletionStatusV1).cast(),
                )
            });
            assert_eq!((tag, allocations), (0, 0));
            assert!(index < selected.len() && !seen[index]);
            seen[index] = true;
            assert_eq!(
                (key, outcome.tag, outcome.count),
                (selected[index].value, 0, 2)
            );
        }
        dr_v1_transfer_adapter_group_drop(ptr::null(), group);
        assert_eq!(first_read, *b"AB");
        assert_eq!(duplex_read, *b"AB");
        assert_eq!(
            members
                .iter()
                .map(|(_, object)| (**object).calls)
                .collect::<Vec<_>>(),
            [0, 1, 1, 2]
        );
        assert!(members.iter().all(|(_, object)| !(**object).pending));
        dr_v1_readiness_set_drop(ptr::null(), set);
        assert_eq!(*fixture.drops.borrow(), [4, 3, 2, 1]);
    }
}

#[test]
fn readiness_invalid_pending_metadata_consumes_taken_values_before_native_acquisition() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let set = dr_v1_readiness_set_new(core::ptr::null(), &mut DrIoStatusV1::default());
        for id in 1..=3 {
            let mut value = fixture.stream(id, &table);
            let source = source(&value);
            let adapter = DrPendingTransferAdapterV1 {
                context: DrAdapterContextV1 {
                    object: if id == 1 {
                        ptr::null_mut()
                    } else {
                        value.object.cast()
                    },
                    descriptor: core::ptr::null(),
                },
                prepare_read: (id != 3).then_some(prepare),
                prepare_write: None,
                complete_read: (id == 1).then_some(complete),
                complete_write: None,
            };
            let mut key = PlanKey {
                owner: usize::MAX,
                value: u64::MAX,
            };
            let mut status = DrIoStatusV1::default();
            assert_eq!(
                dr_v1_readiness_set_add_owned(
                    ptr::null(),
                    set,
                    source,
                    Some(resolve_source),
                    &adapter,
                    &mut value,
                    (&mut key as *mut PlanKey).cast(),
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
        }
        assert_eq!(*fixture.drops.borrow(), [1, 2, 3]);
        dr_v1_readiness_set_drop(ptr::null(), set);
    }
}

#[test]
fn readiness_plan_rejects_absent_or_wrong_direction_capability_before_any_preparation() {
    unsafe {
        for capabilities in [(false, false), (false, true)] {
            let fixture = Fixture::new();
            let table = descriptor();
            let set = dr_v1_readiness_set_new(core::ptr::null(), &mut DrIoStatusV1::default());
            let mut first = fixture.stream(1, &table);
            let first_object = first.object.cast::<Stream>();
            let (tag, first_key) = readiness_register(set, &mut first, true, false);
            assert_eq!(tag, 0);
            let mut second = fixture.stream(2, &table);
            let second_object = second.object.cast::<Stream>();
            let (tag, second_key) =
                readiness_register(set, &mut second, capabilities.0, capabilities.1);
            assert_eq!(tag, 0);
            let mut a = [0; 2];
            let mut b = [0; 2];
            let first_buffer = DrTransferBufferV1 {
                data: a.as_mut_ptr(),
                length: 2,
                reading: 1,
            };
            let second_buffer = DrTransferBufferV1 {
                data: b.as_mut_ptr(),
                length: 2,
                reading: 1,
            };
            let mut status = DrIoStatusV1::default();
            let plan = plan::dr_v1_transfer_plan_new(
                core::ptr::null(),
                (&first_key as *const PlanKey).cast(),
                &first_buffer,
                &mut status,
            );
            let plan = plan::dr_v1_transfer_plan_append(
                core::ptr::null(),
                plan,
                (&second_key as *const PlanKey).cast(),
                &second_buffer,
                &mut status,
            );
            let budget = immediate_budget();
            let mut error = crate::empty_error_carrier();
            let group = dr_v1_pending_transfer_adapter_plan_begin(
                ptr::null(),
                set,
                plan,
                &budget,
                &mut error,
                &mut status,
            );
            assert!(group.is_null());
            assert_eq!(status.failure(), Some(Failure::invalid_input()));
            assert!(error.object.is_null() && error.vtable.is_null());
            assert_eq!(((*first_object).calls, (*second_object).calls), (0, 0));
            assert!(!(*set.cast::<AdapterSetOwner>()).active);
            dr_v1_readiness_set_drop(ptr::null(), set);
            assert_eq!(*fixture.drops.borrow(), [2, 1]);
        }
    }
}

#[test]
fn readiness_pending_subset_dispatches_complete_objects_and_preserves_active_membership_gates() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let set = dr_v1_readiness_set_new(core::ptr::null(), &mut DrIoStatusV1::default());
        let mut unused = fixture.stream(1, &table);
        let unused_object = unused.object.cast::<Stream>();
        let (_, unused_key) = readiness_register(set, &mut unused, false, false);
        let mut value = fixture.stream(2, &table);
        let object = value.object.cast::<Stream>();
        let (tag, key) = readiness_register(set, &mut value, true, false);
        assert_eq!(tag, 0);
        let mut data = [0; 2];
        let buffer = DrTransferBufferV1 {
            data: data.as_mut_ptr(),
            length: 2,
            reading: 1,
        };
        let mut status = DrIoStatusV1::default();
        let plan = plan::dr_v1_transfer_plan_new(
            core::ptr::null(),
            (&key as *const PlanKey).cast(),
            &buffer,
            &mut status,
        );
        let budget = immediate_budget();
        let mut error = crate::empty_error_carrier();
        let group = dr_v1_pending_transfer_adapter_plan_begin(
            ptr::null(),
            set,
            plan,
            &budget,
            &mut error,
            &mut status,
        );
        assert!(!group.is_null());
        assert_eq!(readiness_remove(set, &unused_key, &mut unused), 3);
        let mut rejected = fixture.stream(3, &table);
        assert_eq!(readiness_register(set, &mut rejected, false, false).0, 3);
        assert_eq!(*fixture.drops.borrow(), [3]);
        let (mut returned_key, mut index) = (0, usize::MAX);
        let mut outcome = DrAdapterOutcomeV1::default();
        let mut control = DrCompletionStatusV1::default();
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
        assert_eq!((returned_key, index, outcome.count), (key.value, 0, 2));
        assert_eq!(data, *b"AB");
        assert_eq!((*unused_object).calls, 0);
        assert!(!(*object).pending);
        dr_v1_transfer_adapter_group_drop(ptr::null(), group);
        assert_eq!(readiness_remove(set, &unused_key, &mut unused), 0);
        drop(OwnedInterfaceValue::from_carrier(unused));
        dr_v1_readiness_set_drop(ptr::null(), set);
        assert_eq!(*fixture.drops.borrow(), [3, 1, 2]);
    }
}

#[test]
fn readiness_admission_failure_consumes_taken_value_without_initializing_native_state() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let set = dr_v1_readiness_set_new(core::ptr::null(), &mut DrIoStatusV1::default());
        let mut value = fixture.stream(1, &table);
        let mut status = DrIoStatusV1::default();
        let mut key = PlanKey {
            owner: usize::MAX,
            value: u64::MAX,
        };
        assert_eq!(
            dr_v1_readiness_set_add_owned(
                ptr::null(),
                set,
                ptr::null(),
                Some(resolve_source),
                ptr::null(),
                &mut value,
                (&mut key as *mut PlanKey).cast(),
                &mut status
            ),
            3
        );
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

#[test]
fn readiness_nonregular_file_membership_does_not_require_pending_file_support() {
    unsafe {
        for pending in [false, true] {
            let fixture = Fixture::new();
            let table = descriptor();
            let set = dr_v1_readiness_set_new(core::ptr::null(), &mut DrIoStatusV1::default());
            #[cfg(unix)]
            let path = std::path::Path::new("/dev/null");
            #[cfg(windows)]
            let path = std::path::Path::new("NUL");
            let mut value = fixture.stream_path(1, &table, path);
            let original = (value.object, value.vtable);
            let (tag, key) = readiness_register(set, &mut value, pending, false);
            assert_eq!(tag, 0);
            if pending {
                let mut data = [0; 2];
                let buffer = DrTransferBufferV1 {
                    data: data.as_mut_ptr(),
                    length: 2,
                    reading: 1,
                };
                let mut status = DrIoStatusV1::default();
                let plan = plan::dr_v1_transfer_plan_new(
                    core::ptr::null(),
                    (&key as *const PlanKey).cast(),
                    &buffer,
                    &mut status,
                );
                let budget = immediate_budget();
                let mut error = crate::empty_error_carrier();
                assert!(dr_v1_pending_transfer_adapter_plan_begin(
                    ptr::null(),
                    set,
                    plan,
                    &budget,
                    &mut error,
                    &mut status
                )
                .is_null());
                assert_eq!(status.failure(), Some(Failure::unsupported()));
            }
            assert_eq!((*original.0.cast::<Stream>()).calls, 0);
            assert_eq!(readiness_remove(set, &key, &mut value), 0);
            assert_eq!((value.object, value.vtable), original);
            drop(OwnedInterfaceValue::from_carrier(value));
            dr_v1_readiness_set_drop(ptr::null(), set);
        }
    }
}

#[test]
fn readiness_foreign_nonempty_and_reused_membership_preserves_owners_and_logical_drop_order() {
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let set = dr_v1_readiness_set_new(core::ptr::null(), &mut DrIoStatusV1::default());
        let other = dr_v1_readiness_set_new(core::ptr::null(), &mut DrIoStatusV1::default());
        let mut values = Vec::new();
        for id in 1..=3 {
            let mut value = fixture.stream(id, &table);
            let (tag, key) = readiness_register(set, &mut value, id == 2, false);
            assert_eq!(tag, 0);
            values.push(key);
        }
        let mut empty = crate::empty_error_carrier();
        assert_eq!(readiness_remove(other, &values[0], &mut empty), 3);
        let mut occupied = fixture.stream(9, &table);
        let original = (occupied.object, occupied.vtable);
        assert_eq!(readiness_remove(set, &values[0], &mut occupied), 3);
        assert_eq!((occupied.object, occupied.vtable), original);
        drop(OwnedInterfaceValue::from_carrier(occupied));
        assert_eq!(readiness_remove(set, &values[1], &mut empty), 0);
        drop(OwnedInterfaceValue::from_carrier(empty));
        let mut value = fixture.stream(4, &table);
        let (tag, newer) = readiness_register(set, &mut value, false, true);
        assert_eq!(tag, 0);
        assert!(newer.value > values[2].value);
        dr_v1_readiness_set_drop(ptr::null(), set);
        dr_v1_readiness_set_drop(ptr::null(), other);
        assert_eq!(*fixture.drops.borrow(), [9, 2, 4, 3, 1]);
    }
}

#[test]
fn owned_registration_returns_both_original_carrier_words_without_an_allocation() {
    unsafe {
        for pending in [false, true] {
            let fixture = Fixture::new();
            let table = descriptor();
            let set = new_set(pending);
            let mut value = fixture.stream(1, &table);
            let original = (value.object, value.vtable);
            let source = source(&value);
            let (tag, key) = register(pending, set, &mut value, source);
            assert_eq!(tag, 0);
            assert!(fixture.drops.borrow().is_empty());
            let (tag, allocations) =
                crate::probe_runtime_allocations(|| remove(set, &key, &mut value));
            assert_eq!((tag, allocations), (0, 0));
            assert_eq!((value.object, value.vtable), original);
            assert_eq!((*value.object.cast::<Stream>()).calls, 0);
            dr_v1_transfer_adapter_set_drop(ptr::null(), set);
            assert!(fixture.drops.borrow().is_empty());
            let stream = value.object.cast::<Stream>();
            let mut data = [0; 2];
            let mut status = DrIoStatusV1::default();
            assert_eq!(
                dr_v1_file_read(
                    ptr::addr_of_mut!((*stream).file).cast(),
                    data.as_mut_ptr(),
                    2,
                    &mut status
                ),
                0
            );
            assert_eq!(data, *b"ab");
            drop(OwnedInterfaceValue::from_carrier(value));
            assert_eq!(*fixture.drops.borrow(), [1]);
        }
    }
}

#[test]
fn owned_adapters_dispatch_after_transfer_and_retire_before_their_values_drop() {
    unsafe {
        for pending in [false, true] {
            for fetch in [false, true] {
                let fixture = Fixture::new();
                let table = descriptor();
                let set = new_set(pending);
                let mut value = fixture.stream(1, &table);
                let stream = value.object.cast::<Stream>();
                let source = source(&value);
                let (tag, key) = register(pending, set, &mut value, source);
                assert_eq!(tag, 0);
                assert_eq!((*stream).calls, 0);
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
                let mut data = [0; 2];
                let buffer = DrTransferBufferV1 {
                    data: data.as_mut_ptr(),
                    length: 2,
                    reading: 1,
                };
                let mut status = DrIoStatusV1::default();
                let plan = plan::dr_v1_transfer_plan_new(
                    core::ptr::null(),
                    (&key as *const PlanKey).cast(),
                    &buffer,
                    &mut status,
                );
                let mut error = crate::empty_error_carrier();
                let group = if pending {
                    dr_v1_pending_transfer_adapter_plan_begin(
                        ptr::null(),
                        set,
                        plan,
                        &budget,
                        &mut error,
                        &mut status,
                    )
                } else {
                    dr_v1_transfer_adapter_plan_begin(
                        ptr::null(),
                        set,
                        plan,
                        &budget,
                        &mut error,
                        &mut status,
                    )
                };
                assert!(!group.is_null());
                assert_eq!((*stream).calls, 1);
                assert_eq!(remove(set, &key, &mut value), 3);
                if fetch {
                    let (mut returned_key, mut index) = (0, usize::MAX);
                    let mut outcome = DrAdapterOutcomeV1::default();
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
                    assert_eq!((tag, allocations), (0, 0));
                    assert_eq!((returned_key, index, outcome.count), (key.value, 0, 2));
                    assert_eq!(
                        data, *b"AB",
                        "the complete adapter must transform the transfer"
                    );
                }
                let ((), allocations) = crate::probe_runtime_allocations(|| {
                    dr_v1_transfer_adapter_group_drop(ptr::null(), group)
                });
                assert_eq!(allocations, 0);
                assert!(!(*stream).pending);
                assert!(fixture.drops.borrow().is_empty());
                dr_v1_transfer_adapter_set_drop(ptr::null(), set);
                assert_eq!(*fixture.drops.borrow(), [1]);
            }
        }
    }
}

#[test]
fn native_set_metadata_drops_before_complete_registered_values() {
    struct Native<'a>(&'a RefCell<Vec<usize>>);
    impl Drop for Native<'_> {
        fn drop(&mut self) {
            self.0.borrow_mut().push(0);
        }
    }
    unsafe {
        let fixture = Fixture::new();
        let table = descriptor();
        let mut retained = StableSlots::new().unwrap();
        retained
            .insert_at(
                0,
                OwnedRegistration {
                    key: 1,
                    value: OwnedInterfaceValue::from_carrier(fixture.stream(1, &table)),
                },
            )
            .unwrap();
        let storage = AdapterSetStorage {
            owner: AdapterSetOwner {
                pending: false,
                readiness: false,
                active: false,
                identity: 1,
                next_registration: 1,
                retained,
            },
            native: ManuallyDrop::new(Native(&fixture.drops)),
        };
        drop(storage);
        assert_eq!(*fixture.drops.borrow(), [0, 1]);
    }
}

#[test]
fn owned_admission_failure_consumes_and_cleans_the_complete_value_once() {
    unsafe {
        for pending in [false, true] {
            for invalid_set in [false, true] {
                let fixture = Fixture::new();
                let table = descriptor();
                let set = new_set(pending);
                let mut value = fixture.stream(1, &table);
                let source = if invalid_set {
                    source(&value)
                } else {
                    ptr::null()
                };
                let (tag, key) = register(
                    pending,
                    if invalid_set { ptr::null_mut() } else { set },
                    &mut value,
                    source,
                );
                assert_eq!(tag, 3);
                assert_eq!((key.owner, key.value), (0, 0));
                assert_eq!(*fixture.drops.borrow(), [1]);
                dr_v1_transfer_adapter_set_drop(ptr::null(), set);
                assert_eq!(*fixture.drops.borrow(), [1]);
            }
        }
    }
}

#[test]
fn active_admission_cleans_only_the_new_taken_value_and_preserves_membership() {
    unsafe {
        for pending in [false, true] {
            let fixture = Fixture::new();
            let table = descriptor();
            let set = new_set(pending);
            let mut first = fixture.stream(1, &table);
            let source = source(&first);
            let (tag, key) = register(pending, set, &mut first, source);
            assert_eq!(tag, 0);
            (*set.cast::<AdapterSetOwner>()).active = true;
            let mut second = fixture.stream(2, &table);
            let (tag, failed) = register(pending, set, &mut second, ptr::dangling());
            assert_eq!(tag, 3);
            assert_eq!((failed.owner, failed.value), (0, 0));
            assert_eq!(*fixture.drops.borrow(), [2]);
            (*set.cast::<AdapterSetOwner>()).active = false;
            assert_eq!(remove(set, &key, &mut first), 0);
            dr_v1_transfer_adapter_set_drop(ptr::null(), set);
            drop(OwnedInterfaceValue::from_carrier(first));
            assert_eq!(*fixture.drops.borrow(), [2, 1]);
        }
    }
}

#[test]
fn owned_removal_rejects_a_nonempty_output_without_overwriting_either_owner() {
    unsafe {
        for pending in [false, true] {
            let fixture = Fixture::new();
            let table = descriptor();
            let set = new_set(pending);
            let mut first = fixture.stream(1, &table);
            let source = source(&first);
            let (tag, key) = register(pending, set, &mut first, source);
            assert_eq!(tag, 0);
            let mut second = fixture.stream(2, &table);
            let original = (second.object, second.vtable);
            assert_eq!(remove(set, &key, &mut second), 3);
            assert_eq!((second.object, second.vtable), original);
            assert!(fixture.drops.borrow().is_empty());
            assert_eq!(remove(set, &key, &mut first), 0);
            dr_v1_transfer_adapter_set_drop(ptr::null(), set);
            drop(OwnedInterfaceValue::from_carrier(second));
            drop(OwnedInterfaceValue::from_carrier(first));
            assert_eq!(*fixture.drops.borrow(), [2, 1]);
        }
    }
}

#[test]
fn foreign_stale_active_and_metadata_only_removal_never_release_an_owned_value() {
    unsafe {
        for pending in [false, true] {
            let fixture = Fixture::new();
            let table = descriptor();
            let set = new_set(pending);
            let foreign = new_set(pending);
            let mut value = fixture.stream(1, &table);
            let source = source(&value);
            let (tag, key) = register(pending, set, &mut value, source);
            assert_eq!(tag, 0);
            assert_eq!(remove(foreign, &key, &mut value), 3);
            let mut status = DrIoStatusV1::default();
            assert_eq!(
                dr_v1_transfer_adapter_set_remove(
                    set,
                    (&key as *const PlanKey).cast(),
                    &mut status
                ),
                3
            );
            (*set.cast::<AdapterSetOwner>()).active = true;
            assert_eq!(remove(set, &key, &mut value), 3);
            (*set.cast::<AdapterSetOwner>()).active = false;
            assert!(fixture.drops.borrow().is_empty());
            assert!(value.object.is_null() && value.vtable.is_null());
            assert_eq!(remove(set, &key, &mut value), 0);
            let mut empty = crate::empty_error_carrier();
            assert_eq!(remove(set, &key, &mut empty), 3);
            assert!(empty.object.is_null() && empty.vtable.is_null());
            dr_v1_transfer_adapter_set_drop(ptr::null(), foreign);
            dr_v1_transfer_adapter_set_drop(ptr::null(), set);
            drop(OwnedInterfaceValue::from_carrier(value));
            assert_eq!(*fixture.drops.borrow(), [1]);
        }
    }
}

#[test]
fn retained_values_drop_in_reverse_acquisition_order_after_hole_reuse_and_growth() {
    unsafe {
        for pending in [false, true] {
            let fixture = Fixture::new();
            let table = descriptor();
            let set = new_set(pending);
            let mut keys = Vec::new();
            for id in 0..3 {
                let mut value = fixture.stream(id, &table);
                let source = source(&value);
                let (tag, key) = register(pending, set, &mut value, source);
                assert_eq!(tag, 0);
                keys.push(key);
            }
            let mut removed = crate::empty_error_carrier();
            assert_eq!(remove(set, &keys[1], &mut removed), 0);
            drop(OwnedInterfaceValue::from_carrier(removed));
            for id in 3..10 {
                let mut value = fixture.stream(id, &table);
                let source = source(&value);
                assert_eq!(register(pending, set, &mut value, source).0, 0);
            }
            let ((), allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_transfer_adapter_set_drop(ptr::null(), set)
            });
            assert_eq!(allocations, 0);
            assert_eq!(*fixture.drops.borrow(), [1, 9, 8, 7, 6, 5, 4, 3, 2, 0]);
        }
    }
}
