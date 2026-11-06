use super::super::locking::*;
use super::*;
use crate::native_handle::OwnedHandle;

struct GuardStorage {
    pointer: *mut c_void,
    size: usize,
    initialized: bool,
}

impl GuardStorage {
    fn new() -> Self {
        let (mut size, mut alignment) = (0, 0);
        unsafe { dr_v1_file_object_lock_layout(&mut size, &mut alignment) };
        Self {
            pointer: unsafe { dr_v2_class_allocate(ptr::null(), size, alignment).cast() },
            size,
            initialized: false,
        }
    }

    fn acquire(
        &mut self,
        file: &mut ObjectStorage,
        mode: u8,
        ownership: u8,
        status: &mut DrFileStatusV1,
    ) -> u8 {
        assert!(!self.initialized);
        assert!(file.initialized);
        let result = unsafe {
            dr_v1_file_object_try_lock(file.pointer, mode, ownership, self.pointer, status)
        };
        if result == 0 {
            self.initialized = true;
            if ownership == 1 {
                // Only successful owning acquisition transfers this existing
                // payload pointer. The wrapper/storage itself never moves.
                file.initialized = false;
                file.pointer = ptr::null_mut();
            }
        }
        result
    }

    fn release(&mut self, file: &mut ObjectStorage, status: &mut DrFileStatusV1) -> u8 {
        assert!(self.initialized);
        let mut returned = ptr::dangling_mut::<c_void>();
        let result = unsafe { dr_v1_file_object_lock_release(self.pointer, &mut returned, status) };
        if result == 0 {
            self.initialized = false;
            if !returned.is_null() {
                assert!(file.pointer.is_null());
                assert!(!file.initialized);
                file.pointer = returned;
                file.initialized = true;
            }
        } else {
            assert!(
                returned.is_null(),
                "failed unlock must not return a second owner"
            );
        }
        result
    }

    fn file(&mut self) -> *mut c_void {
        assert!(self.initialized);
        unsafe { dr_v1_file_object_lock_file(self.pointer) }
    }
}

impl Drop for GuardStorage {
    fn drop(&mut self) {
        unsafe {
            if self.initialized {
                let destroyed = dr_v1_file_object_lock_drop(self.pointer);
                if !destroyed.is_null() {
                    // This fixture used the class allocator. Hosted/inline
                    // consumers own different backing storage and never call
                    // native class free on it.
                    dr_v1_class_free(destroyed.cast());
                }
            }
            dr_v1_class_free(self.pointer.cast());
        }
    }
}

fn opened(fixture: &Fixture, path: &RuntimeString) -> ObjectStorage {
    fs::write(&fixture.path, b"record").unwrap();
    let mut file = ObjectStorage::new();
    assert_eq!(file.open(path, 2, 0, 0, &mut stale_status()), 0);
    file
}

fn contender(path: &RuntimeString) -> ObjectStorage {
    let mut file = ObjectStorage::new();
    assert_eq!(file.open(path, 2, 0, 0, &mut stale_status()), 0);
    file
}

fn assert_status_cleared(status: &DrFileStatusV1) {
    assert_eq!(status.value, 0);
    assert_eq!(status.system_code, 0);
    assert_eq!(status.transferred_byte_count, 0);
    assert_eq!(status.has_system_code, 0);
    assert_eq!(status.has_transferred_byte_count, 0);
}

#[test]
fn borrowed_lock_preserves_payload_readiness_path_and_native_identity_without_allocation() {
    let fixture = Fixture::new("borrowed-lock");
    let path = RuntimeString::new(&fixture.path_bytes);
    let mut file = opened(&fixture, &path);
    let original = file.pointer;
    let source = unsafe { dr_v1_file_object_readiness_source(original) };
    let resource = unsafe {
        pending_source(original)
            .unwrap()
            .origin
            .file()
            .unwrap()
            .1
            .resource
    };
    let handle = unsafe { (*original.cast::<FileObject>()).owner().native_handle() };
    let references = unsafe { (*path.0).references };
    let mut guard = GuardStorage::new();
    let mut status = stale_status();
    let (((), allocations), closes) = with_close_probe(|| {
        probe_runtime_allocations(|| unsafe {
            assert_eq!(guard.acquire(&mut file, 1, 0, &mut status), 0);
            assert_status_cleared(&status);
            assert_eq!(guard.file(), original);
            assert_eq!(dr_v1_file_object_lock_path(guard.pointer), path.0);
            assert_eq!(dr_v1_file_object_readiness_source(guard.file()), source);
            let projection = pending_source(guard.file()).unwrap();
            let (owner, origin) = projection.origin.file().unwrap();
            assert_eq!(origin.payload, original);
            assert_eq!(origin.resource, resource);
            assert_eq!((*owner).native_handle(), handle);
            assert_eq!((*path.0).references, references);
            status = stale_status();
            assert_eq!(guard.release(&mut file, &mut status), 0);
            assert_status_cleared(&status);
        })
    });
    assert_eq!((allocations, closes), (0, 0));
    assert_eq!(file.pointer, original);
    assert_eq!(
        unsafe { dr_v1_file_object_control_int(file.pointer, 5, 0, 0, &mut status) },
        0
    );
    assert_eq!(status.value, 6);
}

#[test]
fn owning_lock_returns_the_original_class_payload_instead_of_rebuilding_the_owner() {
    let fixture = Fixture::new("owning-lock");
    let path = RuntimeString::new(&fixture.path_bytes);
    let mut file = opened(&fixture, &path);
    let original = file.pointer;
    let source = unsafe { dr_v1_file_object_readiness_source(original) };
    let mut guard = GuardStorage::new();
    let mut status = stale_status();
    let (((), allocations), closes) = with_close_probe(|| {
        probe_runtime_allocations(|| {
            assert_eq!(guard.acquire(&mut file, 1, 1, &mut status), 0);
            assert!(file.pointer.is_null());
            assert_eq!(guard.file(), original);
            assert_eq!(
                unsafe { dr_v1_file_object_readiness_source(guard.file()) },
                source
            );
            assert_eq!(guard.release(&mut file, &mut status), 0);
        })
    });
    assert_eq!((allocations, closes), (0, 0));
    assert_eq!(file.pointer, original);
    let (result, closes) =
        with_close_probe(|| unsafe { dr_v1_file_object_close(file.pointer, &mut status) });
    assert_eq!(
        (result, closes),
        (0, 1),
        "active lock pin must retire before file close"
    );
    let ((), closes) = with_close_probe(|| file.destroy());
    assert_eq!(closes, 0);
}

#[test]
fn dropping_a_borrowed_guard_releases_only_its_lock_and_resource_pin() {
    let fixture = Fixture::new("borrowed-drop");
    let path = RuntimeString::new(&fixture.path_bytes);
    let mut file = opened(&fixture, &path);
    let mut competitor = contender(&path);
    let mut other = GuardStorage::new();
    let mut guard = GuardStorage::new();
    let mut status = stale_status();
    assert_eq!(guard.acquire(&mut file, 1, 0, &mut status), 0);
    assert_eq!(other.acquire(&mut competitor, 1, 0, &mut status), 1);
    let ((), closes) = with_close_probe(|| drop(guard));
    assert_eq!(closes, 0);
    assert_eq!(other.acquire(&mut competitor, 1, 0, &mut status), 0);
    assert_eq!(other.release(&mut competitor, &mut status), 0);
    let (result, closes) =
        with_close_probe(|| unsafe { dr_v1_file_object_close(file.pointer, &mut status) });
    assert_eq!((result, closes), (0, 1));
}

#[test]
fn dropping_an_owning_guard_unlocks_closes_and_frees_its_original_file_once() {
    let fixture = Fixture::new("owning-drop");
    let path = RuntimeString::new(&fixture.path_bytes);
    let mut file = opened(&fixture, &path);
    let mut competitor = contender(&path);
    let mut guard = GuardStorage::new();
    let mut other = GuardStorage::new();
    let mut status = stale_status();
    let references = unsafe { (*path.0).references };
    assert_eq!(guard.acquire(&mut file, 1, 1, &mut status), 0);
    assert_eq!(other.acquire(&mut competitor, 1, 0, &mut status), 1);
    let ((), closes) = with_close_probe(|| drop(guard));
    assert_eq!(closes, 1);
    assert_eq!(unsafe { (*path.0).references }, references - 1);
    assert_eq!(other.acquire(&mut competitor, 1, 0, &mut status), 0);
    assert_eq!(other.release(&mut competitor, &mut status), 0);
    let ((), closes) = with_close_probe(|| drop(file));
    assert_eq!(closes, 0);
}

#[test]
fn contention_preserves_both_original_payload_ownership_forms_and_path_claims() {
    let fixture = Fixture::new("contention");
    let path = RuntimeString::new(&fixture.path_bytes);
    let mut first = opened(&fixture, &path);
    let mut first_guard = GuardStorage::new();
    let mut status = stale_status();
    assert_eq!(first_guard.acquire(&mut first, 1, 0, &mut status), 0);
    for ownership in [0, 1] {
        let mut file = contender(&path);
        let original = file.pointer;
        let references = unsafe { (*path.0).references };
        let mut guard = GuardStorage::new();
        status = stale_status();
        let ((tag, allocations), closes) = with_close_probe(|| {
            probe_runtime_allocations(|| guard.acquire(&mut file, 1, ownership, &mut status))
        });
        assert_eq!((tag, allocations, closes), (1, 0, 0));
        assert_status_cleared(&status);
        assert_eq!(file.pointer, original);
        assert!(file.initialized);
        assert!(!guard.initialized);
        assert_eq!(unsafe { (*path.0).references }, references);
        assert_eq!(
            unsafe { dr_v1_file_object_close(file.pointer, &mut status) },
            0,
            "failed acquisition must release its temporary active pin"
        );
    }
}

#[test]
fn shared_object_locks_compose_with_the_same_native_exclusion_mechanism() {
    let fixture = Fixture::new("shared-locks");
    let path = RuntimeString::new(&fixture.path_bytes);
    let mut first = opened(&fixture, &path);
    let mut second = contender(&path);
    let mut third = contender(&path);
    let mut first_guard = GuardStorage::new();
    let mut second_guard = GuardStorage::new();
    let mut third_guard = GuardStorage::new();
    let mut status = stale_status();
    assert_eq!(first_guard.acquire(&mut first, 0, 0, &mut status), 0);
    assert_eq!(second_guard.acquire(&mut second, 0, 1, &mut status), 0);
    assert_eq!(third_guard.acquire(&mut third, 1, 0, &mut status), 1);
    assert_eq!(first_guard.release(&mut first, &mut status), 0);
    assert_eq!(third_guard.acquire(&mut third, 1, 0, &mut status), 1);
    assert_eq!(second_guard.release(&mut second, &mut status), 0);
    assert_eq!(third_guard.acquire(&mut third, 1, 0, &mut status), 0);
}

#[test]
fn closed_payload_and_invalid_tags_fail_without_new_claims_or_unchecked_projection() {
    let fixture = Fixture::new("invalid-lock");
    let path = RuntimeString::new(&fixture.path_bytes);
    let mut file = opened(&fixture, &path);
    let original = file.pointer;
    let mut guard = GuardStorage::new();
    let mut status = stale_status();
    let references = unsafe { (*path.0).references };
    for (mode, ownership) in [(2, 0), (0, 2), (255, 255)] {
        assert_eq!(guard.acquire(&mut file, mode, ownership, &mut status), 3);
        assert_eq!(status.failure(), Some(Failure::invalid_input()));
        assert_eq!(file.pointer, original);
    }
    assert_eq!(
        unsafe { dr_v1_file_object_try_lock(ptr::null_mut(), 0, 0, guard.pointer, &mut status) },
        3
    );
    assert_eq!(status.failure(), Some(Failure::invalid_input()));
    assert_eq!(
        unsafe { dr_v1_file_object_close(file.pointer, &mut status) },
        0
    );
    for ownership in [0, 1] {
        assert_eq!(guard.acquire(&mut file, 1, ownership, &mut status), 3);
        let failure = status.failure().unwrap();
        assert_eq!(failure.reason, Reason::Closed);
        assert_eq!(failure.system_code, None);
        assert_eq!(failure.transferred_byte_count, None);
        assert_eq!(file.pointer, original);
        assert_eq!(unsafe { (*path.0).references }, references);
    }
}

#[cfg(unix)]
fn invalid_handle() -> crate::native_handle::NativeHandle {
    -1
}
#[cfg(windows)]
fn invalid_handle() -> crate::native_handle::NativeHandle {
    usize::MAX as _
}

#[test]
fn native_acquisition_failure_preserves_payload_path_and_original_failure() {
    let fixture = Fixture::new("failed-lock");
    let path = RuntimeString::new(&fixture.path_bytes);
    let mut file = opened(&fixture, &path);
    let original = file.pointer;
    let mut guard = GuardStorage::new();
    let mut status = stale_status();
    unsafe {
        let owner = (*file.pointer.cast::<FileObject>()).owner();
        let real = mem::replace(&mut owner.handle, OwnedHandle::from_raw(invalid_handle()));
        for ownership in [0, 1] {
            assert_eq!(guard.acquire(&mut file, 1, ownership, &mut status), 3);
            let failure = status.failure().unwrap();
            assert_eq!(failure.reason, Reason::Closed);
            assert!(failure.system_code.is_some());
            assert_eq!(failure.transferred_byte_count, None);
            assert_eq!(file.pointer, original);
            assert_eq!(dr_v1_file_object_path(file.pointer), path.0);
        }
        (*file.pointer.cast::<FileObject>()).owner().handle = real;
        assert_eq!(dr_v1_file_object_close(file.pointer, &mut status), 0);
    }
}

#[test]
fn unlock_failure_keeps_the_same_pin_payload_path_and_owned_obligation_for_retry() {
    let fixture = Fixture::new("failed-unlock");
    let path = RuntimeString::new(&fixture.path_bytes);
    let mut file = opened(&fixture, &path);
    let original = file.pointer;
    let mut guard = GuardStorage::new();
    let mut status = stale_status();
    assert_eq!(guard.acquire(&mut file, 1, 1, &mut status), 0);
    unsafe {
        let owner = (*guard.file().cast::<FileObject>()).owner();
        let real = mem::replace(&mut owner.handle, OwnedHandle::from_raw(invalid_handle()));
        let (tag, closes) = with_close_probe(|| guard.release(&mut file, &mut status));
        assert_eq!((tag, closes), (3, 0));
        assert!(file.pointer.is_null());
        assert!(guard.initialized);
        assert_eq!(guard.file(), original);
        assert_eq!(dr_v1_file_object_lock_path(guard.pointer), path.0);
        assert_eq!(status.failure().unwrap().reason, Reason::Closed);
        assert!(status.failure().unwrap().system_code.is_some());
        assert_eq!(status.failure().unwrap().transferred_byte_count, None);
        (*guard.file().cast::<FileObject>()).owner().handle = real;
        assert_eq!(guard.release(&mut file, &mut status), 0);
        assert_eq!(file.pointer, original);
        assert_eq!(dr_v1_file_object_close(file.pointer, &mut status), 0);
    }
}

#[test]
fn retained_error_path_outlives_owning_guard_cleanup_without_extra_success_retains() {
    let fixture = Fixture::new("retained-path");
    let path = RuntimeString::new(&fixture.path_bytes);
    let mut file = opened(&fixture, &path);
    let mut guard = GuardStorage::new();
    assert_eq!(guard.acquire(&mut file, 1, 1, &mut stale_status()), 0);
    path.release();
    let retained = unsafe {
        let borrowed = dr_v1_file_object_lock_path(guard.pointer);
        assert_eq!((*borrowed).references, 1);
        dr_v1_string_retain(borrowed)
    };
    drop(guard);
    unsafe {
        assert_eq!((*retained).references, 1);
        assert_eq!(
            core::slice::from_raw_parts(string_bytes(retained), (*retained).byte_length),
            fixture.path_bytes
        );
        dr_v1_string_release(retained);
    }
}

#[test]
fn inline_guard_relocation_keeps_the_original_file_resource_home() {
    let fixture = Fixture::new("moved-guard");
    let path = RuntimeString::new(&fixture.path_bytes);
    let mut file = opened(&fixture, &path);
    let original = file.pointer;
    let source = unsafe { dr_v1_file_object_readiness_source(original) };
    let mut first = GuardStorage::new();
    let mut moved = GuardStorage::new();
    let mut status = stale_status();
    assert_eq!(first.acquire(&mut file, 1, 1, &mut status), 0);
    assert_eq!(first.size, moved.size);
    unsafe {
        ptr::copy_nonoverlapping(
            first.pointer.cast::<u8>(),
            moved.pointer.cast::<u8>(),
            first.size,
        )
    };
    first.initialized = false;
    moved.initialized = true;
    drop(first);
    assert_eq!(moved.file(), original);
    assert_eq!(
        unsafe { dr_v1_file_object_readiness_source(moved.file()) },
        source
    );
    assert_eq!(moved.release(&mut file, &mut status), 0);
    assert_eq!(file.pointer, original);
}

#[test]
fn owning_cleanup_leaves_caller_owned_inline_storage_to_its_original_allocator() {
    let fixture = Fixture::new("inline-payload");
    fs::write(&fixture.path, b"inline").unwrap();
    let path = RuntimeString::new(&fixture.path_bytes);
    let mut file = MaybeUninit::<FileObject>::uninit();
    let payload = file.as_mut_ptr().cast::<c_void>();
    let mut guard = GuardStorage::new();
    let mut status = stale_status();
    unsafe {
        assert_eq!(
            dr_v1_file_object_open(path.0, 2, 0, 0, payload, &mut status),
            0
        );
        assert_eq!(
            dr_v1_file_object_try_lock(payload, 1, 1, guard.pointer, &mut status),
            0
        );
        guard.initialized = true;
        assert_eq!(guard.file(), payload);
        let (destroyed, closes) = with_close_probe(|| dr_v1_file_object_lock_drop(guard.pointer));
        guard.initialized = false;
        assert_eq!(destroyed, payload);
        assert_eq!(closes, 1);
        // Both inline regions are now uninitialized. Only GuardStorage's
        // allocation is freed; the original payload is stack-owned memory.
        assert_eq!((*path.0).references, 1);
    }
}
