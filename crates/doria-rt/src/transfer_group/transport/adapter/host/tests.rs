use super::*;
use crate::file_io::stream::transport::object::*;
use crate::transfer_group::pending::host::*;
use crate::transfer_group::pending::{
    dr_v1_checked_pending_request_outcome, dr_v1_pending_request_complete,
};
use crate::transfer_group::plan::OperationKey;
use crate::transfer_group::transport::{plan::dr_v1_transfer_plan_new, DrTransferBufferV1};
use std::sync::atomic::{AtomicUsize, Ordering};

struct File {
    object: *mut c_void,
    path: *mut DrStringV1,
    filename: std::path::PathBuf,
}

impl File {
    unsafe fn new(contents: &[u8]) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let filename = std::env::temp_dir().join(format!(
            "doria-host-transfer-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&filename, contents).unwrap();
        let bytes = filename.to_str().unwrap().as_bytes();
        let path = crate::dr_v1_string_from_utf8(bytes.as_ptr(), bytes.len());
        let (mut size, mut alignment) = (0, 0);
        dr_v1_file_object_layout(&mut size, &mut alignment);
        let object = crate::dr_v2_class_allocate(ptr::null(), size, alignment).cast();
        assert_eq!(
            dr_v1_file_object_open(path, 0, 0, 0, object, &mut DrIoStatusV1::default()),
            0
        );
        Self {
            object,
            path,
            filename,
        }
    }
    unsafe fn source(&self) -> *const c_void {
        dr_v1_file_object_readiness_source(self.object)
    }
}

impl Drop for File {
    fn drop(&mut self) {
        unsafe {
            dr_v1_file_object_drop(ptr::null(), self.object);
            crate::dr_v1_class_free(self.object.cast());
            crate::dr_v1_string_release(self.path);
        }
        std::fs::remove_file(&self.filename).unwrap();
    }
}

#[derive(Clone, Copy)]
enum TestSource<'a> {
    File(&'a File),
    Standard(crate::device_io::StandardStream),
}

impl<'a> From<&'a File> for TestSource<'a> {
    fn from(file: &'a File) -> Self {
        Self::File(file)
    }
}

impl From<crate::device_io::StandardStream> for TestSource<'_> {
    fn from(stream: crate::device_io::StandardStream) -> Self {
        Self::Standard(stream)
    }
}

impl TestSource<'_> {
    unsafe fn source(self) -> *const c_void {
        match self {
            Self::File(file) => file.source(),
            Self::Standard(stream) => crate::device_io::standard_device(stream).source().value(),
        }
    }

    fn reading(self) -> u8 {
        u8::from(matches!(
            self,
            Self::File(_) | Self::Standard(crate::device_io::StandardStream::Stdin)
        ))
    }
}

struct CallStorage {
    view: Vec<u64>,
    record: *mut c_void,
    caller: *mut u8,
    length: usize,
    actions: [usize; 5],
    immediate: Option<(u8, usize)>,
}

impl CallStorage {
    unsafe fn new() -> Self {
        let (mut size, mut alignment) = (0, 0);
        dr_v1_host_pending_view_layout(&mut size, &mut alignment);
        assert!(alignment <= mem::align_of::<u64>());
        Self {
            view: vec![0; size.div_ceil(8)],
            record: ptr::null_mut(),
            caller: ptr::null_mut(),
            length: 0,
            actions: [0; 5],
            immediate: None,
        }
    }
    fn view(&mut self) -> *mut c_void {
        self.view.as_mut_ptr().cast()
    }

    unsafe fn action(&mut self, owner: *mut c_void) -> u8 {
        let (mut index, mut record, mut caller, mut length, mut reading) =
            (99, ptr::null_mut(), ptr::null_mut(), 99, 99);
        let action = dr_v1_host_transfer_group_action(
            owner,
            &mut index,
            &mut record,
            self.view(),
            &mut caller,
            &mut length,
            &mut reading,
        );
        assert!((1..=4).contains(&action));
        assert_eq!(index, 0);
        if self.record.is_null() {
            self.record = record;
        } else {
            assert_eq!(self.record, record, "request home changed");
        }
        self.caller = caller;
        self.length = length;
        self.actions[action as usize] += 1;
        action
    }

    unsafe fn configure<'a>(&mut self, owner: *mut c_void, source: impl Into<TestSource<'a>>) {
        if let Some((tag, count)) = self.immediate {
            dr_v1_pending_request_complete(self.view(), tag, count);
            assert_eq!(
                dr_v1_host_pending_prepare_reply(self.view(), ptr::null_mut()),
                0
            );
            return;
        }
        let mut error = crate::empty_error_carrier();
        let (mut message, mut path, mut system, mut transferred, mut meta) =
            (ptr::null_mut(), ptr::null_mut(), 0, 0, 0);
        let source = source.into();
        let code = match source {
            TestSource::File(file) => dr_v1_host_pending_request_configure_file(
                ptr::null(),
                owner,
                self.view(),
                file.object,
                source.source(),
                self.caller,
                self.length,
                0,
                self.length,
                &mut error,
                &mut message,
                &mut path,
                &mut system,
                &mut transferred,
                &mut meta,
            ),
            TestSource::Standard(stream) => dr_v1_host_pending_request_configure_standard(
                ptr::null(),
                owner,
                self.view(),
                stream as u8,
                source.source(),
                self.caller,
                self.length,
                0,
                self.length,
                &mut error,
                &mut message,
                &mut path,
                &mut system,
                &mut transferred,
                &mut meta,
            ),
        };
        assert_eq!(code, 0);
        assert!(
            error.object.is_null() && error.vtable.is_null() && message.is_null() && path.is_null()
        );
        assert_eq!(
            dr_v1_host_pending_prepare_reply(self.view(), ptr::null_mut()),
            0
        );
    }

    unsafe fn complete(&mut self) {
        let (mut tag, mut count, mut message, mut path, mut system, mut transferred, mut meta) =
            (0, 0, ptr::null_mut(), ptr::null_mut(), 0, 0, 0);
        let code = dr_v1_checked_pending_request_outcome(
            ptr::null(),
            self.view(),
            &mut tag,
            &mut count,
            &mut message,
            &mut path,
            &mut system,
            &mut transferred,
            &mut meta,
        );
        assert!(matches!(code, 0 | 4));
        assert!(message.is_null() && path.is_null());
        let reply = if code == 4 { 4 } else { tag as u8 };
        assert_eq!(
            dr_v1_host_pending_complete_reply(self.view(), reply, count, ptr::null_mut()),
            0
        );
    }

    unsafe fn drive<'a>(
        &mut self,
        owner: *mut c_void,
        source: impl Into<TestSource<'a>>,
    ) -> (u8, u8, usize, DrAdapterOutcomeV1) {
        let source = source.into();
        loop {
            let result = Self::poll(owner);
            if result.0 != 8 {
                return result;
            }
            match self.action(owner) {
                1 | 2 => assert_eq!(
                    dr_v1_host_pending_source_reply(self.record, source.source(), ptr::null_mut()),
                    0
                ),
                3 => self.configure(owner, source),
                4 => self.complete(),
                _ => unreachable!(),
            }
        }
    }

    unsafe fn poll(owner: *mut c_void) -> (u8, u8, usize, DrAdapterOutcomeV1) {
        // Fresh call-local output homes must not be written after a yield.
        let (mut kind, mut index, mut outcome, mut terminal, mut control) = (
            99,
            99,
            DrAdapterOutcomeV1::default(),
            mem::MaybeUninit::<AdapterTerminal>::uninit(),
            DrCompletionStatusV1::default(),
        );
        let code = dr_v1_host_transfer_group_poll(
            ptr::null(),
            owner,
            &mut kind,
            &mut index,
            &mut outcome,
            terminal.as_mut_ptr().cast(),
            ptr::from_mut(&mut control).cast(),
        );
        assert_ne!(code, 1, "this fixture does not consume hosted Error claims");
        (code, kind, index, outcome)
    }
}

unsafe fn reach_preparation<'a>(
    owner: *mut c_void,
    source: impl Into<TestSource<'a>>,
    storage: &mut CallStorage,
) {
    let source = source.into();
    for expected in [1, 2, 3] {
        let (mut kind, mut index, mut outcome, mut terminal, mut control) = (
            0,
            0,
            DrAdapterOutcomeV1::default(),
            mem::MaybeUninit::<AdapterTerminal>::uninit(),
            DrCompletionStatusV1::default(),
        );
        assert_eq!(
            dr_v1_host_transfer_group_poll(
                ptr::null(),
                owner,
                &mut kind,
                &mut index,
                &mut outcome,
                terminal.as_mut_ptr().cast(),
                ptr::from_mut(&mut control).cast()
            ),
            8
        );
        assert_eq!(storage.action(owner), expected);
        if expected != 3 {
            assert_eq!(
                dr_v1_host_pending_source_reply(storage.record, source.source(), ptr::null_mut()),
                0
            );
        }
    }
}

unsafe fn start<'a>(
    source: impl Into<TestSource<'a>>,
    bytes: &mut [u8],
) -> (*mut c_void, *mut c_void, OperationKey) {
    start_with_control(
        source,
        bytes,
        ptr::null_mut(),
        &DrWaitBudgetV1::indefinite(),
    )
}

unsafe fn start_with_control<'a>(
    source: impl Into<TestSource<'a>>,
    bytes: &mut [u8],
    cancellation: *mut crate::cancellation::DrCancellationV1,
    budget: &DrWaitBudgetV1,
) -> (*mut c_void, *mut c_void, OperationKey) {
    let source = source.into();
    let mut status = DrIoStatusV1::default();
    let set = if cancellation.is_null() {
        dr_v1_readiness_set_new(ptr::null(), &mut status)
    } else {
        let set = dr_v1_pending_transfer_adapter_set_new(ptr::null(), cancellation, &mut status);
        assert!(!set.is_null(), "{:?}", status.failure());
        // The fixture combines the existing retained-cancellation constructor
        // with owning readiness admission; no public control spelling is assumed.
        (*set.cast::<AdapterSetOwner>()).readiness = true;
        set
    };
    assert!(!set.is_null(), "{:?}", status.failure());
    let mut key = PlanKey { owner: 0, value: 0 };
    assert_eq!(
        dr_v1_readiness_set_add_host_pending_source(
            set,
            source.source(),
            source.reading(),
            1 - source.reading(),
            ptr::from_mut(&mut key).cast(),
            &mut status
        ),
        0
    );
    let buffer = DrTransferBufferV1 {
        data: bytes.as_mut_ptr(),
        length: bytes.len(),
        reading: source.reading(),
    };
    let plan = dr_v1_transfer_plan_new(
        ptr::null(),
        ptr::from_ref(&key).cast(),
        &buffer,
        &mut status,
    );
    assert!(!plan.is_null());
    let operation = (*plan.cast::<TransferPlan<'static>>())
        .operation_key(0, source.reading() != 0)
        .unwrap();
    let owner = dr_v1_host_transfer_plan_configure(ptr::null(), set, plan, budget, &mut status);
    assert!(!owner.is_null(), "{:?}", status.failure());
    (set, owner, operation)
}

mod standard_tests;

#[test]
fn hosted_cancellation_preserves_returned_preparation_and_retains_state_after_source_release() {
    use crate::cancellation::*;
    unsafe {
        let file = File::new(b"untouched");
        for immediate in [None, Some((0, 1)), Some((1, 0)), Some((2, 0)), Some((3, 0))] {
            let source = dr_v1_cancellation_new();
            assert!(!source.is_null());
            let token = dr_v1_cancellation_retain(source);
            assert!(!token.is_null());
            let mut bytes = [0; 3];
            let (set, owner, key) =
                start_with_control(&file, &mut bytes, token, &DrWaitBudgetV1::indefinite());
            dr_v1_cancellation_release(source);
            assert_eq!(dr_v1_cancellation_is_requested(token), 0);
            let mut storage = CallStorage::new();
            storage.immediate = immediate;
            reach_preparation(owner, &file, &mut storage);
            storage.configure(owner, &file);
            let (mut reason, mut system, mut has_system) = (0, 0, 0);
            let (requested, allocations) = crate::probe_runtime_allocations(|| {
                dr_v1_cancellation_request(token, &mut reason, &mut system, &mut has_system)
            });
            assert_eq!((requested, allocations), (0, 0));
            dr_v1_cancellation_release(token);
            // The set and group retain their own claims across host handoffs.
            let (started, allocations) =
                crate::probe_runtime_allocations(|| storage.drive(owner, &file));
            assert_eq!(
                (started.0, started.1, storage.actions[4], allocations),
                (0, 0, 1, 0)
            );
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
            let (completed, allocations) =
                crate::probe_runtime_allocations(|| storage.drive(owner, &file));
            assert_eq!((completed.1, completed.2, allocations), (1, 0, 0));
            match immediate {
                None => assert_eq!(completed.0, 4),
                Some((tag, count)) => {
                    assert_eq!(
                        (completed.0, completed.3.tag, completed.3.count),
                        (0, tag, count)
                    );
                }
            }
            let mut observed = OperationKey {
                owner: 0,
                position: 0,
            };
            assert_eq!(
                dr_v1_host_transfer_group_observed(owner, 1, ptr::from_mut(&mut observed).cast()),
                0
            );
            assert_eq!(observed, key);
            dr_v1_host_transfer_group_close(owner);
            assert_eq!(storage.drive(owner, &file).0, 0);
            assert_eq!(storage.actions, [0, 1, 1, 1, 1]);
            dr_v1_host_transfer_group_drop(ptr::null(), owner);
            dr_v1_transfer_adapter_set_drop(ptr::null(), set);
            assert_eq!(bytes, [0; 3], "cancelled preparation performed native I/O");
        }
        let mut check = [0; 9];
        assert_eq!(
            dr_v1_file_object_read(
                file.object,
                check.as_mut_ptr(),
                check.len(),
                &mut DrIoStatusV1::default()
            ),
            0
        );
        assert_eq!(&check, b"untouched");
    }
}

#[test]
fn hosted_cancellation_during_completion_preserves_retired_native_bytes() {
    use crate::cancellation::*;
    unsafe {
        let file = File::new(b"abcDEF");
        let source = dr_v1_cancellation_new();
        assert!(!source.is_null());
        let mut bytes = [0; 3];
        let (set, owner, key) =
            start_with_control(&file, &mut bytes, source, &DrWaitBudgetV1::indefinite());
        let mut storage = CallStorage::new();
        loop {
            let result = CallStorage::poll(owner);
            if result.0 == 0 {
                assert_eq!(result.1, 0, "completion was exposed before its host reply");
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
                continue;
            }
            assert_eq!(result.0, 8);
            match storage.action(owner) {
                1 | 2 => assert_eq!(
                    dr_v1_host_pending_source_reply(storage.record, file.source(), ptr::null_mut()),
                    0
                ),
                3 => storage.configure(owner, &file),
                4 => break,
                _ => unreachable!(),
            }
        }
        assert!((*set.cast::<AdapterSetOwner>()).active);
        let mut observed = OperationKey {
            owner: 0,
            position: 0,
        };
        assert_eq!(
            dr_v1_host_transfer_group_observed(owner, 1, ptr::from_mut(&mut observed).cast()),
            1
        );
        let (mut reason, mut system, mut has_system) = (0, 0, 0);
        let (requested, allocations) = crate::probe_runtime_allocations(|| {
            dr_v1_cancellation_request(source, &mut reason, &mut system, &mut has_system)
        });
        assert_eq!((requested, allocations), (0, 0));
        dr_v1_cancellation_release(source);
        let ((), allocations) = crate::probe_runtime_allocations(|| storage.complete());
        assert_eq!(allocations, 0);
        let (mut result, allocations) =
            crate::probe_runtime_allocations(|| storage.drive(owner, &file));
        assert_eq!(allocations, 0);
        if result.1 == 0 {
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
            let (completed, allocations) =
                crate::probe_runtime_allocations(|| storage.drive(owner, &file));
            assert_eq!(allocations, 0);
            result = completed;
        }
        assert_eq!(
            (result.0, result.1, result.3.tag, result.3.count),
            (0, 1, 0, 3)
        );
        let (mut data, mut length) = (ptr::null(), 0);
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
        assert_eq!(core::slice::from_raw_parts(data, length), b"abc");
        dr_v1_host_transfer_group_close(owner);
        assert_eq!(storage.drive(owner, &file).0, 0);
        assert_eq!(storage.actions[4], 1, "native retirement completed twice");
        dr_v1_host_transfer_group_drop(ptr::null(), owner);
        dr_v1_transfer_adapter_set_drop(ptr::null(), set);
        assert_eq!(&bytes, b"abc");
        let mut check = [0; 3];
        assert_eq!(
            dr_v1_file_object_read(
                file.object,
                check.as_mut_ptr(),
                check.len(),
                &mut DrIoStatusV1::default()
            ),
            0
        );
        assert_eq!(&check, b"DEF");
    }
}

#[test]
fn hosted_controller_reads_reuses_the_same_request_and_drains_before_owner_release() {
    unsafe {
        let file = File::new(b"abcDEF");
        let mut bytes = [0; 3];
        let (set, owner, key) = start(&file, &mut bytes);
        let mut storage = CallStorage::new();
        let (started, allocations) =
            crate::probe_runtime_allocations(|| storage.drive(owner, &file));
        assert_eq!((started.0, started.1, allocations), (0, 0, 0));
        assert!((*set.cast::<AdapterSetOwner>()).active);
        for expected in [b"abc", b"DEF"] {
            assert_eq!(
                dr_v1_host_transfer_group_observe(
                    owner,
                    &DrWaitBudgetV1::indefinite(),
                    &mut DrIoStatusV1::default()
                ),
                0
            );
            let (completed, allocations) =
                crate::probe_runtime_allocations(|| storage.drive(owner, &file));
            assert_eq!(
                (
                    completed.0,
                    completed.1,
                    completed.2,
                    completed.3.tag,
                    completed.3.count,
                    allocations
                ),
                (0, 1, 0, 0, 3, 0)
            );
            let (mut data, mut length) = (ptr::null(), 0);
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
            assert_eq!(core::slice::from_raw_parts(data, length), expected);
            let mut observed = OperationKey {
                owner: 0,
                position: 0,
            };
            assert_eq!(
                dr_v1_host_transfer_group_observed(owner, 1, ptr::from_mut(&mut observed).cast()),
                0
            );
            assert_eq!(observed, key);
            if expected == b"abc" {
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
                    crate::probe_runtime_allocations(|| storage.drive(owner, &file));
                assert_eq!((reused.0, reused.1, allocations), (0, 2, 0));
            }
        }
        assert_eq!(storage.actions, [0, 1, 2, 2, 2]);
        dr_v1_host_transfer_group_close(owner);
        let (closed, allocations) =
            crate::probe_runtime_allocations(|| storage.drive(owner, &file));
        assert_eq!((closed.0, closed.1, allocations), (0, 3, 0));
        dr_v1_host_transfer_group_drop(ptr::null(), owner);
        assert!(!(*set.cast::<AdapterSetOwner>()).active);
        dr_v1_transfer_adapter_set_drop(ptr::null(), set);
        assert_eq!(bytes, *b"DEF");
    }
}

#[test]
fn hosted_controller_abandoned_startup_accepts_returned_preparation_before_cleanup() {
    unsafe {
        let file = File::new(b"untouched");
        let mut bytes = [0; 3];
        let (set, owner, _) = start(&file, &mut bytes);
        let mut storage = CallStorage::new();
        reach_preparation(owner, &file, &mut storage);
        storage.configure(owner, &file);
        dr_v1_host_transfer_group_close(owner);
        let (closed, allocations) =
            crate::probe_runtime_allocations(|| storage.drive(owner, &file));
        assert_eq!((closed.0, closed.1, allocations), (0, 3, 0));
        assert_eq!(storage.actions, [0, 1, 1, 1, 1]);
        dr_v1_host_transfer_group_drop(ptr::null(), owner);
        dr_v1_transfer_adapter_set_drop(ptr::null(), set);
        assert_eq!(bytes, [0; 3], "abandoned startup must submit no bytes");
        let mut check = [0; 9];
        let mut status = DrIoStatusV1::default();
        assert_eq!(
            dr_v1_file_object_read(file.object, check.as_mut_ptr(), check.len(), &mut status),
            0
        );
        assert_eq!(
            &check, b"untouched",
            "startup cleanup changed the file cursor"
        );
    }
}

#[test]
fn hosted_controller_throwing_preparation_returns_original_claim_after_shared_cleanup() {
    use crate::transfer_group::adapter::host_error::*;
    unsafe {
        let file = File::new(b"untouched");
        let mut bytes = [0; 3];
        let (set, owner, _) = start(&file, &mut bytes);
        let mut storage = CallStorage::new();
        reach_preparation(owner, &file, &mut storage);
        let (mut size, mut align, mut terminal_size, mut terminal_align) = (0, 0, 0, 0);
        dr_v1_host_adapter_error_layout(
            &mut size,
            &mut align,
            &mut terminal_size,
            &mut terminal_align,
        );
        assert!(
            align <= mem::align_of::<u64>() && terminal_align <= mem::align_of::<AdapterTerminal>()
        );
        let mut slot = vec![0u64; size.div_ceil(8)];
        let slot = slot.as_mut_ptr().cast();
        dr_v1_host_adapter_error_initialize(slot, 73);
        let mut terminal = mem::MaybeUninit::<AdapterTerminal>::uninit();
        assert_eq!(
            dr_v1_host_adapter_error_claim(slot, terminal.as_mut_ptr().cast(), 1, 2, 3),
            0
        );
        assert_eq!(
            dr_v1_host_pending_prepare_reply(storage.view(), terminal.as_mut_ptr().cast()),
            0
        );
        let (mut kind, mut index, mut outcome, mut control) = (
            0,
            0,
            DrAdapterOutcomeV1::default(),
            DrCompletionStatusV1::default(),
        );
        let (code, allocations) = crate::probe_runtime_allocations(|| {
            dr_v1_host_transfer_group_poll(
                ptr::null(),
                owner,
                &mut kind,
                &mut index,
                &mut outcome,
                terminal.as_mut_ptr().cast(),
                ptr::from_mut(&mut control).cast(),
            )
        });
        assert_eq!((code, allocations), (1, 0));
        assert!(matches!(
            (*owner.cast::<HostGroupOwner>()).call,
            Call::Closed
        ));
        assert_eq!(
            dr_v1_host_adapter_error_return(terminal.as_mut_ptr().cast()),
            73
        );
        assert_eq!(dr_v1_host_adapter_error_disposition(slot), 3);
        assert_eq!(dr_v1_host_adapter_error_acknowledge(slot), 0);
        assert_eq!(
            storage.actions,
            [0, 1, 1, 1, 0],
            "throwing preparation owes no completion"
        );
        dr_v1_host_transfer_group_drop(ptr::null(), owner);
        dr_v1_transfer_adapter_set_drop(ptr::null(), set);
        assert_eq!(bytes, [0; 3]);
    }
}

#[test]
fn hosted_immediate_outcomes_retire_at_submission_and_reuse_without_early_identity() {
    unsafe {
        let file = File::new(b"untouched");
        for (tag, count) in [(0, 1), (1, 0), (2, 0), (3, 0)] {
            let mut bytes = [0; 3];
            let (set, owner, key) = start(&file, &mut bytes);
            let mut storage = CallStorage::new();
            storage.immediate = Some((tag, count));
            let (started, allocations) =
                crate::probe_runtime_allocations(|| storage.drive(owner, &file));
            assert_eq!((started.0, storage.actions[4], allocations), (0, 1, 0));
            let mut observed = OperationKey {
                owner: 0,
                position: 0,
            };
            assert_eq!(
                dr_v1_host_transfer_group_observed(owner, 1, ptr::from_mut(&mut observed).cast()),
                1
            );
            for completion in 1..=2 {
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
                let (result, allocations) =
                    crate::probe_runtime_allocations(|| storage.drive(owner, &file));
                assert_eq!(
                    (
                        result.0,
                        result.3.tag,
                        result.3.count,
                        storage.actions[4],
                        allocations
                    ),
                    (0, tag, count, completion, 0)
                );
                assert_eq!(
                    dr_v1_host_transfer_group_observed(
                        owner,
                        1,
                        ptr::from_mut(&mut observed).cast()
                    ),
                    0
                );
                assert_eq!(observed, key);
                // A failed new fetch clears the old key without consuming another result.
                assert_ne!(
                    dr_v1_host_transfer_group_observe_wait(
                        owner,
                        99,
                        0,
                        0,
                        &mut DrIoStatusV1::default()
                    ),
                    0
                );
                assert_eq!(
                    dr_v1_host_transfer_group_observed(
                        owner,
                        1,
                        ptr::from_mut(&mut observed).cast()
                    ),
                    1
                );
                if completion == 1 {
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
                        crate::probe_runtime_allocations(|| storage.drive(owner, &file));
                    assert_eq!((reused.0, storage.actions[4], allocations), (0, 2, 0));
                    assert_eq!(
                        dr_v1_host_transfer_group_observed(
                            owner,
                            1,
                            ptr::from_mut(&mut observed).cast()
                        ),
                        1
                    );
                }
            }
            dr_v1_host_transfer_group_close(owner);
            assert_eq!(storage.drive(owner, &file).0, 0);
            assert_eq!(storage.actions[4], 2);
            dr_v1_host_transfer_group_drop(ptr::null(), owner);
            dr_v1_transfer_adapter_set_drop(ptr::null(), set);
            assert_eq!(bytes, [0; 3], "immediate completion performed native I/O");
        }
    }
}
