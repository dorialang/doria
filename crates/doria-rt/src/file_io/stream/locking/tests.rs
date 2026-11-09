use super::*;
use crate::byte_io::ReadProgress;
use crate::file_io::stream::{FileAccess, FileOpenRequest, SeekOrigin};
use crate::native_handle::{with_close_probe, OwnedHandle};
use crate::platform_io::Reason;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(super) struct Fixture(PathBuf);

impl Fixture {
    pub(super) fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "doria-file-lock-{}-{}-{}.bin",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        drop(file);
        fs::write(&path, b"locked bytes").unwrap();
        Self(path)
    }

    pub(super) fn open(&self) -> OwnedFile {
        OwnedFile::open(
            self.0.to_str().unwrap().as_bytes(),
            FileOpenRequest {
                access: FileAccess::ReadWrite,
                ..FileOpenRequest::READ
            },
        )
        .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let cleanup = fs::remove_file(&self.0);
        if !std::thread::panicking() {
            cleanup.unwrap();
        }
    }
}

fn acquire<F: FileOwner>(owner: F, mode: LockMode) -> FileLock<F> {
    match try_lock(owner, mode) {
        Ok(LockAttempt::Acquired(guard)) => guard,
        Ok(LockAttempt::WouldBlock(_)) => panic!("unexpected lock contention"),
        Err(error) => panic!("unexpected lock failure: {:?}", error.failure),
    }
}

fn release<F: FileOwner>(guard: FileLock<F>) -> F {
    match guard.release() {
        Ok(owner) => owner,
        Err(error) => panic!("unexpected unlock failure: {:?}", error.failure),
    }
}

fn contended<F: FileOwner>(owner: F, mode: LockMode) -> F {
    match try_lock(owner, mode) {
        Ok(LockAttempt::WouldBlock(owner)) => owner,
        Ok(LockAttempt::Acquired(_)) => panic!("conflicting lock acquired"),
        Err(error) => panic!("contention became a checked failure: {:?}", error.failure),
    }
}

#[test]
fn borrowed_exclusive_guard_preserves_file_cursor_and_allocates_nothing() {
    let fixture = Fixture::new();
    let mut file = fixture.open();
    let mut competitor = fixture.open();
    let handle = file.native_handle();
    file.seek(3, SeekOrigin::Start).unwrap();
    let (((), allocations), closes) = with_close_probe(|| {
        crate::probe_runtime_allocations(|| {
            for _ in 0..8 {
                let mut guard = acquire(&mut file, LockMode::Exclusive);
                assert_eq!(guard.file_mut().native_handle(), handle);
                assert_eq!(guard.file_mut().position(), Ok(3));
                contended(&mut competitor, LockMode::Shared);
                contended(&mut competitor, LockMode::Exclusive);
                release(guard);
            }
        })
    });
    assert_eq!(allocations, 0);
    assert_eq!(closes, 0);
    assert_eq!(file.position(), Ok(3));
    let mut bytes = [0; 3];
    assert_eq!(file.read(&mut bytes), Ok(ReadProgress::Data(3)));
    assert_eq!(&bytes, b"ked");
    release(acquire(&mut competitor, LockMode::Exclusive));
}

#[test]
fn shared_guards_coexist_and_exclude_an_exclusive_guard() {
    let fixture = Fixture::new();
    let mut first = fixture.open();
    let mut second = fixture.open();
    let mut competitor = fixture.open();
    let first_guard = acquire(&mut first, LockMode::Shared);
    let second_guard = acquire(&mut second, LockMode::Shared);
    contended(&mut competitor, LockMode::Exclusive);
    release(first_guard);
    contended(&mut competitor, LockMode::Exclusive);
    release(second_guard);
    release(acquire(&mut competitor, LockMode::Exclusive));
}

#[test]
fn borrowed_guard_cleanup_unlocks_without_closing_on_checked_error_exit() {
    let fixture = Fixture::new();
    let mut file = fixture.open();
    let mut competitor = fixture.open();
    let (result, closes) = with_close_probe(|| -> Result<(), u8> {
        let _guard = acquire(&mut file, LockMode::Exclusive);
        contended(&mut competitor, LockMode::Exclusive);
        Err(7)?;
        Ok(())
    });
    assert_eq!(result, Err(7));
    assert_eq!(closes, 0);
    assert_eq!(file.length(), Ok(12));
    release(acquire(&mut competitor, LockMode::Exclusive));
}

fn returned_locked_file(fixture: &Fixture) -> FileLock<OwnedFile> {
    acquire(fixture.open(), LockMode::Exclusive)
}

#[test]
fn owning_guard_can_be_returned_and_drop_closes_exactly_once() {
    let fixture = Fixture::new();
    let guard = returned_locked_file(&fixture);
    let mut competitor = fixture.open();
    contended(&mut competitor, LockMode::Exclusive);
    let ((), closes) = with_close_probe(|| drop(guard));
    assert_eq!(closes, 1);
    release(acquire(&mut competitor, LockMode::Exclusive));
}

#[test]
fn explicit_owning_release_returns_original_owner_without_close() {
    let fixture = Fixture::new();
    let file = fixture.open();
    let handle = file.native_handle();
    let (file, closes) = with_close_probe(|| release(acquire(file, LockMode::Exclusive)));
    assert_eq!(closes, 0);
    assert_eq!(file.native_handle(), handle);
    let (result, closes) = with_close_probe(|| file.close());
    assert_eq!(result, Ok(()));
    assert_eq!(closes, 1);
}

#[test]
fn contention_returns_owned_file_unchanged_for_later_acquisition() {
    let fixture = Fixture::new();
    let mut first = fixture.open();
    let guard = acquire(&mut first, LockMode::Exclusive);
    let mut second = fixture.open();
    let handle = second.native_handle();
    second.seek(5, SeekOrigin::Start).unwrap();
    let (second, closes) = with_close_probe(|| contended(second, LockMode::Exclusive));
    assert_eq!(closes, 0);
    assert_eq!(second.native_handle(), handle);
    assert_eq!(second.position(), Ok(5));
    drop(guard);
    release(acquire(second, LockMode::Exclusive))
        .close()
        .unwrap();
}

#[cfg(unix)]
pub(super) fn invalid_handle() -> NativeHandle {
    -1
}

#[cfg(windows)]
pub(super) fn invalid_handle() -> NativeHandle {
    usize::MAX as NativeHandle
}

#[test]
fn acquisition_error_preserves_original_failure_and_owned_file() {
    let fixture = Fixture::new();
    let mut file = fixture.open();
    let real_owner = core::mem::replace(&mut file.handle, unsafe {
        OwnedHandle::from_raw(invalid_handle())
    });
    let expected = platform_try_lock(invalid_handle(), LockMode::Exclusive).unwrap_err();
    let mut error = match try_lock(file, LockMode::Exclusive) {
        Err(error) => error,
        Ok(_) => panic!("invalid handle was treated as lock contention or success"),
    };
    assert_eq!(error.failure, expected);
    assert_eq!(error.failure.reason, Reason::Closed);
    assert!(error.failure.system_code.is_some());
    assert_eq!(error.failure.transferred_byte_count, None);
    assert_eq!(error.owner.native_handle(), invalid_handle());
    error.owner.handle = real_owner;
    release(acquire(error.owner, LockMode::Exclusive));
}

#[test]
fn unlock_error_retains_guard_and_owner_until_a_successful_retry() {
    let fixture = Fixture::new();
    let mut guard = returned_locked_file(&fixture);
    let handle = guard.file_mut().native_handle();
    let real_owner = core::mem::replace(&mut guard.file_mut().handle, unsafe {
        OwnedHandle::from_raw(invalid_handle())
    });
    let expected = platform_unlock(invalid_handle()).unwrap_err();
    let error = match guard.release() {
        Err(error) => error,
        Ok(_) => panic!("invalid unlock succeeded"),
    };
    assert_eq!(error.failure, expected);
    assert_eq!(error.failure.transferred_byte_count, None);
    let mut guard = error.owner;
    let mut competitor = fixture.open();
    contended(&mut competitor, LockMode::Exclusive);
    guard.file_mut().handle = real_owner;
    assert_eq!(guard.file_mut().native_handle(), handle);
    release(guard).close().unwrap();
    release(acquire(&mut competitor, LockMode::Exclusive));
}

#[test]
fn closing_an_unrelated_open_does_not_release_the_guard() {
    let fixture = Fixture::new();
    let guard = returned_locked_file(&fixture);
    fixture.open().close().unwrap();
    let mut competitor = fixture.open();
    contended(&mut competitor, LockMode::Exclusive);
    drop(guard);
    release(acquire(&mut competitor, LockMode::Exclusive));
}

#[test]
fn native_lock_child() {
    let Some(path) = std::env::var_os("DORIA_RT_LOCK_CHILD_PATH") else {
        return;
    };
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    let result = file.try_lock();
    match std::env::var("DORIA_RT_LOCK_CHILD_EXPECT")
        .unwrap()
        .as_str()
    {
        "blocked" => assert!(matches!(result, Err(fs::TryLockError::WouldBlock))),
        "acquired" => result.unwrap(),
        _ => panic!("invalid lock-child expectation"),
    }
    println!("doria-native-lock-child-ok");
}

fn native_child(fixture: &Fixture, expected: &str) {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "file_io::stream::locking::tests::native_lock_child",
            "--nocapture",
        ])
        .env("DORIA_RT_LOCK_CHILD_PATH", &fixture.0)
        .env("DORIA_RT_LOCK_CHILD_EXPECT", expected)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "native lock child failed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("doria-native-lock-child-ok"));
}

#[test]
fn locks_interoperate_with_native_locks_in_another_process() {
    let fixture = Fixture::new();
    for mode in [LockMode::Shared, LockMode::Exclusive] {
        let guard = acquire(fixture.open(), mode);
        native_child(&fixture, "blocked");
        release(guard).close().unwrap();
        native_child(&fixture, "acquired");
    }
}

#[test]
fn owner_lock_state_records_one_lock_and_release_is_explicit() {
    let fixture = Fixture::new();
    let mut first = fixture.open();
    let mut second = fixture.open();

    assert!(first.acquire_lock(LockMode::Exclusive, true).unwrap());
    assert_eq!(
        first
            .acquire_lock(LockMode::Shared, false)
            .err()
            .map(|failure| failure.reason),
        Some(Reason::InvalidInput),
        "a held lock is never silently converted"
    );
    assert!(!second.acquire_lock(LockMode::Shared, false).unwrap());
    assert!(second.lock.is_none(), "contention records nothing");

    first.release_lock().unwrap();
    assert_eq!(
        first.release_lock().err().map(|failure| failure.reason),
        Some(Reason::InvalidInput)
    );
    assert!(second.acquire_lock(LockMode::Shared, false).unwrap());
    assert!(first.acquire_lock(LockMode::Shared, false).unwrap());
    assert!(!fixture
        .open()
        .acquire_lock(LockMode::Exclusive, false)
        .unwrap());
}

#[test]
fn abandoning_releases_a_held_lock_once_and_is_otherwise_inert() {
    let fixture = Fixture::new();
    let mut first = fixture.open();
    let mut second = fixture.open();

    first.abandon_lock();
    assert!(first.acquire_lock(LockMode::Exclusive, false).unwrap());
    assert!(!second.acquire_lock(LockMode::Exclusive, false).unwrap());
    first.abandon_lock();
    assert!(first.lock.is_none());
    first.abandon_lock();
    assert!(second.acquire_lock(LockMode::Exclusive, true).unwrap());
}
