//! The system code a foreign call leaves (Decision 0148 section 16).
//!
//! A declaration with `#[CaptureSystemCode]` calls `dr_v1_capture_system_code`
//! immediately after its foreign call, before any other code runs, and
//! `ForeignCall::lastSystemCode` reads the stored value with
//! `dr_v1_last_system_code`. Only the thread running Doria code runs Doria
//! code and its foreign calls (section 13), so the slot that thread writes
//! is its own. The value before any capturing call is 0.

use core::sync::atomic::{AtomicI64, Ordering};

/// `SystemCodeSource::Errno`.
pub const SOURCE_ERRNO: u32 = 0;
/// `SystemCodeSource::WindowsLastError`.
pub const SOURCE_WINDOWS_LAST_ERROR: u32 = 1;

static LAST_SYSTEM_CODE: AtomicI64 = AtomicI64::new(0);

#[cfg(target_os = "macos")]
extern "C" {
    fn __error() -> *mut i32;
}

#[cfg(target_os = "linux")]
extern "C" {
    fn __errno_location() -> *mut i32;
}

#[cfg(windows)]
extern "C" {
    // The Universal CRT's thread `errno`; only programs that link foreign
    // code, and so the UCRT (section 20), capture it.
    fn _errno() -> *mut i32;
}

unsafe fn errno() -> i32 {
    #[cfg(target_os = "macos")]
    {
        *__error()
    }
    #[cfg(target_os = "linux")]
    {
        *__errno_location()
    }
    #[cfg(windows)]
    {
        *_errno()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        0
    }
}

/// Reads the code `source` names and stores it as the last system code.
///
/// # Safety
///
/// Called on the thread running Doria code, immediately after a foreign
/// call returns.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_capture_system_code(source: u32) {
    let code = match source {
        SOURCE_ERRNO => i64::from(errno()),
        #[cfg(windows)]
        SOURCE_WINDOWS_LAST_ERROR => i64::from(crate::win32::GetLastError()),
        _ => 0,
    };
    LAST_SYSTEM_CODE.store(code, Ordering::Relaxed);
}

/// `ForeignCall::lastSystemCode`: the code the last capturing call stored.
#[no_mangle]
pub extern "C" fn dr_v1_last_system_code() -> i64 {
    LAST_SYSTEM_CODE.load(Ordering::Relaxed)
}
