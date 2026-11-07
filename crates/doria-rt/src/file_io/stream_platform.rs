use crate::native_handle::NativeHandle;
use crate::platform_io::Failure;

use super::SeekOrigin;

#[path = "file_stat.rs"]
mod file_stat;

#[cfg(any(test, not(any(target_os = "linux", target_os = "macos", windows))))]
fn unsupported() -> Failure {
    Failure {
        reason: crate::platform_io::Reason::Unsupported,
        system_code: None,
        transferred_byte_count: None,
    }
}

#[cfg(unix)]
pub(super) fn seek(handle: NativeHandle, offset: i64, origin: SeekOrigin) -> Result<u64, Failure> {
    // off_t and symbol selection belong to the platform bindings, not a
    // copied pointer-width/architecture inventory in the runtime.
    let offset = libc::off_t::try_from(offset).map_err(|_| Failure::invalid_input())?;
    let origin = match origin {
        SeekOrigin::Start => libc::SEEK_SET,
        SeekOrigin::Current => libc::SEEK_CUR,
        SeekOrigin::End => libc::SEEK_END,
    };
    let position = unsafe { libc::lseek(handle, offset, origin) };
    if position < 0 {
        Err(crate::platform_io::last_failure())
    } else {
        Ok(position as u64)
    }
}

#[cfg(windows)]
pub(super) fn seek(handle: NativeHandle, offset: i64, origin: SeekOrigin) -> Result<u64, Failure> {
    use crate::win32::SetFilePointerEx;
    let origin = match origin {
        SeekOrigin::Start => 0,
        SeekOrigin::Current => 1,
        SeekOrigin::End => 2,
    };
    let mut position = 0;
    if unsafe { SetFilePointerEx(handle, offset, &mut position, origin) } == 0 {
        Err(crate::platform_io::last_failure())
    } else {
        u64::try_from(position).map_err(|_| Failure::invalid_input())
    }
}

#[cfg(not(any(unix, windows)))]
pub(super) fn seek(_: NativeHandle, _: i64, _: SeekOrigin) -> Result<u64, Failure> {
    Err(unsupported())
}

pub(super) fn length(handle: NativeHandle) -> Result<u64, Failure> {
    file_stat::length(handle)
}

#[cfg(unix)]
pub(super) fn set_length(handle: NativeHandle, length: i64) -> Result<(), Failure> {
    let length = libc::off_t::try_from(length).map_err(|_| Failure::invalid_input())?;
    if unsafe { libc::ftruncate(handle, length) } == 0 {
        Ok(())
    } else {
        Err(crate::platform_io::last_failure())
    }
}

#[cfg(windows)]
pub(super) fn set_length(handle: NativeHandle, length: i64) -> Result<(), Failure> {
    #[repr(C)]
    struct FileEndOfFileInfo {
        end_of_file: i64,
    }
    use crate::win32::SetFileInformationByHandle;
    let mut information = FileEndOfFileInfo {
        end_of_file: length,
    };
    // Set the extent directly; no temporary cursor movement or restoration.
    if unsafe {
        SetFileInformationByHandle(
            handle,
            6,
            core::ptr::from_mut(&mut information).cast(),
            core::mem::size_of::<FileEndOfFileInfo>() as u32,
        )
    } != 0
    {
        Ok(())
    } else {
        Err(crate::platform_io::last_failure())
    }
}

#[cfg(not(any(unix, windows)))]
pub(super) fn set_length(_: NativeHandle, _: i64) -> Result<(), Failure> {
    Err(unsupported())
}

#[cfg(target_os = "linux")]
pub(super) fn sync(handle: NativeHandle, metadata: bool) -> Result<(), Failure> {
    let result = unsafe {
        if metadata {
            libc::fsync(handle)
        } else {
            libc::fdatasync(handle)
        }
    };
    if result == 0 {
        Ok(())
    } else {
        Err(crate::platform_io::last_failure())
    }
}

#[cfg(target_os = "macos")]
pub(super) fn sync(handle: NativeHandle, _metadata: bool) -> Result<(), Failure> {
    // Darwin has no data-only equivalent with the accepted durability
    // guarantee. Both requests use the stronger full sync. Never fall back
    // to weaker fsync when the filesystem rejects F_FULLFSYNC.
    if unsafe { libc::fcntl(handle, libc::F_FULLFSYNC) } == 0 {
        Ok(())
    } else {
        Err(crate::platform_io::last_failure())
    }
}

#[cfg(windows)]
pub(super) fn sync(handle: NativeHandle, _metadata: bool) -> Result<(), Failure> {
    use crate::win32::FlushFileBuffers;
    // Win32's documented primitive provides the stronger full-file sync.
    if unsafe { FlushFileBuffers(handle) } != 0 {
        Ok(())
    } else {
        Err(crate::platform_io::last_failure())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub(super) fn sync(_: NativeHandle, _: bool) -> Result<(), Failure> {
    Err(unsupported())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_operations_do_not_invent_host_codes_or_progress() {
        let error = unsupported();
        assert_eq!(error.reason, crate::platform_io::Reason::Unsupported);
        assert_eq!(error.system_code, None);
        assert_eq!(error.transferred_byte_count, None);
    }
}
