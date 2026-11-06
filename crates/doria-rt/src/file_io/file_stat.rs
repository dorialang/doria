use crate::native_handle::NativeHandle;
use crate::platform_io::Failure;

#[cfg(unix)]
pub(super) fn length(handle: NativeHandle) -> Result<u64, Failure> {
    // libc owns the complete platform layout and symbol redirects, including
    // GNU/musl differences. Do not copy an architecture inventory here.
    let mut information = core::mem::MaybeUninit::<libc::stat>::zeroed();
    if unsafe { libc::fstat(handle, information.as_mut_ptr()) } != 0 {
        Err(crate::platform_io::last_failure())
    } else {
        // Inspect only st_size after success, not padding or reserved fields.
        let size = unsafe { core::ptr::addr_of!((*information.as_ptr()).st_size).read() };
        u64::try_from(size).map_err(|_| Failure::invalid_input())
    }
}

#[cfg(windows)]
pub(super) fn length(handle: NativeHandle) -> Result<u64, Failure> {
    unsafe extern "system" {
        fn GetFileSizeEx(handle: NativeHandle, length: *mut i64) -> i32;
    }
    let mut length = 0;
    if unsafe { GetFileSizeEx(handle, &mut length) } == 0 {
        Err(crate::platform_io::last_failure())
    } else {
        u64::try_from(length).map_err(|_| Failure::invalid_input())
    }
}

#[cfg(not(any(unix, windows)))]
pub(super) fn length(_: NativeHandle) -> Result<u64, Failure> {
    Err(super::unsupported())
}
