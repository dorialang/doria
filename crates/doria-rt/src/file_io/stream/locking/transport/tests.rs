use super::*;
use crate::file_io::stream::locking::tests::Fixture;
use crate::native_handle::{with_close_probe, OwnedHandle};
use core::mem::MaybeUninit;

fn stale_status() -> DrIoStatusV1 {
    DrIoStatusV1 {
        value: 77,
        system_code: 77,
        transferred_byte_count: 77,
        reason: u8::MAX,
        has_system_code: 1,
        has_transferred_byte_count: 1,
    }
}

fn assert_cleared(status: DrIoStatusV1) {
    assert_eq!(status.value, 0);
    assert_eq!(status.system_code, 0);
    assert_eq!(status.transferred_byte_count, 0);
    assert_eq!(status.has_system_code, 0);
    assert_eq!(status.has_transferred_byte_count, 0);
}

#[test]
fn opaque_layout_reports_the_real_inline_guard_storage() {
    let (mut size, mut alignment) = (0, 0);
    unsafe { dr_v1_file_lock_layout(&mut size, &mut alignment) };
    assert_eq!(size, mem::size_of::<NativeLock>());
    assert_eq!(alignment, mem::align_of::<NativeLock>());
}

#[test]
fn borrowed_transport_projects_and_returns_original_file_without_allocation_or_close() {
    let fixture = Fixture::new();
    let mut file = fixture.open();
    let file_ptr = (&mut file as *mut OwnedFile).cast();
    let mut guard = MaybeUninit::<NativeLock>::uninit();
    let mut status = stale_status();
    let (((), allocations), closes) = with_close_probe(|| {
        crate::probe_runtime_allocations(|| unsafe {
            assert_eq!(
                dr_v1_file_try_lock(file_ptr, 1, 0, guard.as_mut_ptr().cast(), &mut status),
                0
            );
            assert_cleared(status);
            assert_eq!(dr_v1_file_lock_file(guard.as_mut_ptr().cast()), file_ptr);
            status = stale_status();
            assert_eq!(
                dr_v1_file_lock_release(guard.as_mut_ptr().cast(), ptr::null_mut(), &mut status),
                0
            );
            assert_cleared(status);
        })
    });
    assert_eq!(allocations, 0);
    assert_eq!(closes, 0);
    assert_eq!(file.length(), Ok(12));
}

#[test]
fn owning_transport_consumes_and_returns_one_original_file_owner() {
    let fixture = Fixture::new();
    let mut file = MaybeUninit::new(fixture.open());
    let handle = unsafe { file.assume_init_ref() }.native_handle();
    let mut guard = MaybeUninit::<NativeLock>::uninit();
    let mut returned = MaybeUninit::<OwnedFile>::uninit();
    let mut status = stale_status();
    let ((), closes) = with_close_probe(|| unsafe {
        assert_eq!(
            dr_v1_file_try_lock(
                file.as_mut_ptr().cast(),
                1,
                1,
                guard.as_mut_ptr().cast(),
                &mut status
            ),
            0
        );
        assert_cleared(status);
        let projected = dr_v1_file_lock_file(guard.as_mut_ptr().cast()).cast::<OwnedFile>();
        assert_eq!((*projected).native_handle(), handle);
        status = stale_status();
        assert_eq!(
            dr_v1_file_lock_release(
                guard.as_mut_ptr().cast(),
                returned.as_mut_ptr().cast(),
                &mut status
            ),
            0
        );
        assert_cleared(status);
    });
    assert_eq!(closes, 0);
    let file = unsafe { returned.assume_init() };
    assert_eq!(file.native_handle(), handle);
    let (result, closes) = with_close_probe(|| file.close());
    assert_eq!(result, Ok(()));
    assert_eq!(closes, 1);
}

#[test]
fn contention_does_not_consume_original_owned_transport_storage() {
    let fixture = Fixture::new();
    let first = super::super::try_lock(fixture.open(), LockMode::Exclusive);
    let Ok(LockAttempt::Acquired(first)) = first else {
        panic!("initial lock failed")
    };
    let mut file = MaybeUninit::new(fixture.open());
    let handle = unsafe { file.assume_init_ref() }.native_handle();
    let mut guard = MaybeUninit::<NativeLock>::uninit();
    let mut status = stale_status();
    let (tag, closes) = with_close_probe(|| unsafe {
        dr_v1_file_try_lock(
            file.as_mut_ptr().cast(),
            1,
            1,
            guard.as_mut_ptr().cast(),
            &mut status,
        )
    });
    assert_eq!(tag, 1);
    assert_cleared(status);
    assert_eq!(closes, 0);
    let file = unsafe { file.assume_init() };
    assert_eq!(file.native_handle(), handle);
    drop(first);
    let Ok(LockAttempt::Acquired(guard)) = try_lock(file, LockMode::Exclusive) else {
        panic!("restored file unusable")
    };
    drop(guard);
}

#[test]
fn invalid_tags_and_missing_pointers_are_rejected_before_owner_transfer() {
    let fixture = Fixture::new();
    let mut file = fixture.open();
    let handle = file.native_handle();
    let mut guard = MaybeUninit::<NativeLock>::uninit();
    for (mode, ownership) in [(2, 0), (0, 2), (u8::MAX, u8::MAX)] {
        let mut status = stale_status();
        assert_eq!(
            unsafe {
                dr_v1_file_try_lock(
                    (&mut file as *mut OwnedFile).cast(),
                    mode,
                    ownership,
                    guard.as_mut_ptr().cast(),
                    &mut status,
                )
            },
            3
        );
        assert_eq!(status.failure(), Some(Failure::invalid_input()));
        assert_eq!(file.native_handle(), handle);
    }
    let mut status = stale_status();
    assert_eq!(
        unsafe {
            dr_v1_file_try_lock(
                ptr::null_mut(),
                0,
                0,
                guard.as_mut_ptr().cast(),
                &mut status,
            )
        },
        3
    );
    assert_eq!(status.failure(), Some(Failure::invalid_input()));
    assert_eq!(file.length(), Ok(12));
}

#[test]
fn acquisition_failure_restores_owner_and_exact_native_status() {
    let fixture = Fixture::new();
    let mut file = fixture.open();
    let real = mem::replace(&mut file.handle, unsafe {
        OwnedHandle::from_raw(super::super::tests::invalid_handle())
    });
    let mut file = MaybeUninit::new(file);
    let expected =
        super::super::platform_try_lock(super::super::tests::invalid_handle(), LockMode::Shared)
            .unwrap_err();
    let mut guard = MaybeUninit::<NativeLock>::uninit();
    let mut status = stale_status();
    assert_eq!(
        unsafe {
            dr_v1_file_try_lock(
                file.as_mut_ptr().cast(),
                0,
                1,
                guard.as_mut_ptr().cast(),
                &mut status,
            )
        },
        3
    );
    assert_eq!(status.failure(), Some(expected));
    let mut file = unsafe { file.assume_init() };
    file.handle = real;
    file.close().unwrap();
}

#[test]
fn failed_unlock_restores_live_guard_for_checked_retry() {
    let fixture = Fixture::new();
    let mut file = MaybeUninit::new(fixture.open());
    let mut guard = MaybeUninit::<NativeLock>::uninit();
    let mut returned = MaybeUninit::<OwnedFile>::uninit();
    let mut status = stale_status();
    unsafe {
        assert_eq!(
            dr_v1_file_try_lock(
                file.as_mut_ptr().cast(),
                1,
                1,
                guard.as_mut_ptr().cast(),
                &mut status
            ),
            0
        );
        let projected = dr_v1_file_lock_file(guard.as_mut_ptr().cast()).cast::<OwnedFile>();
        let real = mem::replace(
            &mut (*projected).handle,
            OwnedHandle::from_raw(super::super::tests::invalid_handle()),
        );
        let expected =
            super::super::platform_unlock(super::super::tests::invalid_handle()).unwrap_err();
        assert_eq!(
            dr_v1_file_lock_release(
                guard.as_mut_ptr().cast(),
                returned.as_mut_ptr().cast(),
                &mut status
            ),
            3
        );
        assert_eq!(status.failure(), Some(expected));
        let projected = dr_v1_file_lock_file(guard.as_mut_ptr().cast()).cast::<OwnedFile>();
        (*projected).handle = real;
        assert_eq!(
            dr_v1_file_lock_release(
                guard.as_mut_ptr().cast(),
                returned.as_mut_ptr().cast(),
                &mut status
            ),
            0
        );
        assert_cleared(status);
        returned.assume_init().close().unwrap();
    }
}

#[test]
fn wrong_release_output_is_rejected_without_consuming_or_unlocking_either_guard() {
    let fixture = Fixture::new();
    for ownership in [0, 1] {
        let mut file = MaybeUninit::new(fixture.open());
        let mut guard = MaybeUninit::<NativeLock>::uninit();
        let mut returned = MaybeUninit::<OwnedFile>::uninit();
        let mut status = stale_status();
        unsafe {
            assert_eq!(
                dr_v1_file_try_lock(
                    file.as_mut_ptr().cast(),
                    1,
                    ownership,
                    guard.as_mut_ptr().cast(),
                    &mut status
                ),
                0
            );
            let wrong_output = if ownership == 0 {
                returned.as_mut_ptr().cast()
            } else {
                ptr::null_mut()
            };
            assert_eq!(
                dr_v1_file_lock_release(guard.as_mut_ptr().cast(), wrong_output, &mut status),
                3
            );
            assert_eq!(status.failure(), Some(Failure::invalid_input()));
            let Ok(LockAttempt::WouldBlock(competitor)) =
                try_lock(fixture.open(), LockMode::Exclusive)
            else {
                panic!("invalid release unlocked or consumed guard")
            };
            competitor.close().unwrap();
            let output = if ownership == 0 {
                ptr::null_mut()
            } else {
                returned.as_mut_ptr().cast()
            };
            assert_eq!(
                dr_v1_file_lock_release(guard.as_mut_ptr().cast(), output, &mut status),
                0
            );
            if ownership == 0 {
                file.assume_init().close().unwrap();
            } else {
                returned.assume_init().close().unwrap();
            }
        }
    }
}

#[test]
fn transport_drop_closes_only_the_owning_form_and_releases_both_locks() {
    let fixture = Fixture::new();
    for ownership in [0, 1] {
        let mut file = MaybeUninit::new(fixture.open());
        let mut guard = MaybeUninit::<NativeLock>::uninit();
        let mut status = stale_status();
        unsafe {
            assert_eq!(
                dr_v1_file_try_lock(
                    file.as_mut_ptr().cast(),
                    1,
                    ownership,
                    guard.as_mut_ptr().cast(),
                    &mut status
                ),
                0
            );
            let ((), closes) = with_close_probe(|| dr_v1_file_lock_drop(guard.as_mut_ptr().cast()));
            assert_eq!(closes, usize::from(ownership));
            if ownership == 0 {
                file.assume_init().close().unwrap();
            }
        }
        let Ok(LockAttempt::Acquired(guard)) = try_lock(fixture.open(), LockMode::Exclusive) else {
            panic!("drop did not release lock")
        };
        drop(guard);
    }
}
