use core::ffi::c_void;
use core::ptr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileError {
    PathNul,
    Read,
    Write,
    Allocation,
}

pub(crate) struct OwnedBytes {
    pub(crate) bytes: *mut u8,
    pub(crate) length: usize,
}

impl Drop for OwnedBytes {
    fn drop(&mut self) {
        unsafe {
            if !self.bytes.is_null() {
                super::deallocate(self.bytes);
            }
        }
    }
}

fn has_nul(path: &[u8]) -> bool {
    path.contains(&0)
}

#[cfg(unix)]
pub(crate) unsafe fn read_file(path: &[u8]) -> Result<OwnedBytes, FileError> {
    let path = unix_path(path)?;
    let descriptor = open(path.bytes.cast(), O_RDONLY, 0);
    drop(path);
    if descriptor < 0 {
        return Err(FileError::Read);
    }
    let result = read_descriptor(descriptor);
    if close(descriptor) != 0 && result.is_ok() {
        return Err(FileError::Read);
    }
    result
}

#[cfg(unix)]
unsafe fn read_descriptor(descriptor: i32) -> Result<OwnedBytes, FileError> {
    let mut capacity = 4096_usize;
    let mut bytes = super::allocate(capacity);
    if bytes.is_null() {
        return Err(FileError::Allocation);
    }
    let mut length = 0;
    loop {
        if length == capacity {
            let next = capacity.checked_mul(2).ok_or(FileError::Allocation)?;
            let replacement = super::allocate(next);
            if replacement.is_null() {
                super::deallocate(bytes);
                return Err(FileError::Allocation);
            }
            ptr::copy_nonoverlapping(bytes, replacement, length);
            super::deallocate(bytes);
            bytes = replacement;
            capacity = next;
        }
        let result = read(
            descriptor,
            bytes.add(length).cast::<c_void>(),
            capacity - length,
        );
        if result > 0 {
            length += result as usize;
        } else if result == 0 {
            return Ok(OwnedBytes { bytes, length });
        } else if last_errno() != EINTR {
            super::deallocate(bytes);
            return Err(FileError::Read);
        }
    }
}

#[cfg(unix)]
pub(crate) unsafe fn write_file(path: &[u8], contents: &[u8]) -> Result<(), FileError> {
    let path = unix_path(path)?;
    let descriptor = open(path.bytes.cast(), O_WRONLY | O_CREAT | O_TRUNC, 0o666);
    drop(path);
    if descriptor < 0 {
        return Err(FileError::Write);
    }
    let mut offset = 0;
    let mut failed = false;
    while offset < contents.len() {
        let result = write(
            descriptor,
            contents.as_ptr().add(offset).cast::<c_void>(),
            contents.len() - offset,
        );
        if result > 0 {
            offset += result as usize;
        } else if result < 0 && last_errno() == EINTR {
            continue;
        } else {
            failed = true;
            break;
        }
    }
    if close(descriptor) != 0 {
        failed = true;
    }
    if failed {
        Err(FileError::Write)
    } else {
        Ok(())
    }
}

#[cfg(unix)]
unsafe fn unix_path(path: &[u8]) -> Result<OwnedBytes, FileError> {
    if has_nul(path) {
        return Err(FileError::PathNul);
    }
    let length = path.len().checked_add(1).ok_or(FileError::Allocation)?;
    let bytes = super::allocate(length);
    if bytes.is_null() {
        return Err(FileError::Allocation);
    }
    ptr::copy_nonoverlapping(path.as_ptr(), bytes, path.len());
    *bytes.add(path.len()) = 0;
    Ok(OwnedBytes { bytes, length })
}

#[cfg(target_os = "linux")]
const O_CREAT: i32 = 0o100;
#[cfg(target_os = "linux")]
const O_TRUNC: i32 = 0o1000;
#[cfg(target_os = "macos")]
const O_CREAT: i32 = 0x0200;
#[cfg(target_os = "macos")]
const O_TRUNC: i32 = 0x0400;
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
const O_CREAT: i32 = 0o100;
#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
const O_TRUNC: i32 = 0o1000;
#[cfg(unix)]
const O_RDONLY: i32 = 0;
#[cfg(unix)]
const O_WRONLY: i32 = 1;
#[cfg(unix)]
const EINTR: i32 = 4;

#[cfg(windows)]
const GENERIC_READ: u32 = 0x8000_0000;
#[cfg(windows)]
const GENERIC_WRITE: u32 = 0x4000_0000;
#[cfg(windows)]
const FILE_SHARE_READ: u32 = 1;
#[cfg(windows)]
const OPEN_EXISTING: u32 = 3;
#[cfg(windows)]
const CREATE_ALWAYS: u32 = 2;
#[cfg(windows)]
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
#[cfg(windows)]
const INVALID_HANDLE_VALUE: *mut c_void = -1_isize as *mut c_void;

#[cfg(windows)]
pub(crate) unsafe fn read_file(path: &[u8]) -> Result<OwnedBytes, FileError> {
    let path = windows_path(path)?;
    let handle = CreateFileW(
        path.bytes.cast::<u16>(),
        GENERIC_READ,
        FILE_SHARE_READ,
        ptr::null_mut(),
        OPEN_EXISTING,
        FILE_ATTRIBUTE_NORMAL,
        ptr::null_mut(),
    );
    drop(path);
    if handle == INVALID_HANDLE_VALUE {
        return Err(FileError::Read);
    }
    let mut capacity = 4096_usize;
    let mut bytes = super::allocate(capacity);
    if bytes.is_null() {
        CloseHandle(handle);
        return Err(FileError::Allocation);
    }
    let mut length = 0;
    let mut failed = false;
    loop {
        if length == capacity {
            let next = capacity.checked_mul(2).ok_or(FileError::Allocation)?;
            let replacement = super::allocate(next);
            if replacement.is_null() {
                failed = true;
                break;
            }
            ptr::copy_nonoverlapping(bytes, replacement, length);
            super::deallocate(bytes);
            bytes = replacement;
            capacity = next;
        }
        let request = core::cmp::min(capacity - length, u32::MAX as usize) as u32;
        let mut read = 0_u32;
        if ReadFile(
            handle,
            bytes.add(length).cast(),
            request,
            &mut read,
            ptr::null_mut(),
        ) == 0
        {
            failed = true;
            break;
        }
        if read == 0 {
            break;
        }
        length += read as usize;
    }
    if CloseHandle(handle) == 0 {
        failed = true;
    }
    if failed {
        super::deallocate(bytes);
        Err(FileError::Read)
    } else {
        Ok(OwnedBytes { bytes, length })
    }
}

#[cfg(windows)]
pub(crate) unsafe fn write_file(path: &[u8], contents: &[u8]) -> Result<(), FileError> {
    let path = windows_path(path)?;
    let handle = CreateFileW(
        path.bytes.cast::<u16>(),
        GENERIC_WRITE,
        0,
        ptr::null_mut(),
        CREATE_ALWAYS,
        FILE_ATTRIBUTE_NORMAL,
        ptr::null_mut(),
    );
    drop(path);
    if handle == INVALID_HANDLE_VALUE {
        return Err(FileError::Write);
    }
    let mut offset = 0;
    let mut failed = false;
    while offset < contents.len() {
        let request = core::cmp::min(contents.len() - offset, u32::MAX as usize) as u32;
        let mut written = 0_u32;
        if WriteFile(
            handle,
            contents.as_ptr().add(offset).cast(),
            request,
            &mut written,
            ptr::null_mut(),
        ) == 0
            || written == 0
        {
            failed = true;
            break;
        }
        offset += written as usize;
    }
    if CloseHandle(handle) == 0 {
        failed = true;
    }
    if failed {
        Err(FileError::Write)
    } else {
        Ok(())
    }
}

#[cfg(windows)]
unsafe fn windows_path(path: &[u8]) -> Result<OwnedBytes, FileError> {
    if has_nul(path) {
        return Err(FileError::PathNul);
    }
    let text = core::str::from_utf8(path).map_err(|_| FileError::PathNul)?;
    let units = text.encode_utf16().count();
    let bytes_length = units
        .checked_add(1)
        .and_then(|units| units.checked_mul(core::mem::size_of::<u16>()))
        .ok_or(FileError::Allocation)?;
    let bytes = super::allocate(bytes_length);
    if bytes.is_null() {
        return Err(FileError::Allocation);
    }
    let wide = bytes.cast::<u16>();
    for (index, unit) in text.encode_utf16().enumerate() {
        *wide.add(index) = unit;
    }
    *wide.add(units) = 0;
    Ok(OwnedBytes {
        bytes,
        length: bytes_length,
    })
}

#[cfg(not(any(unix, windows)))]
pub(crate) unsafe fn read_file(_path: &[u8]) -> Result<OwnedBytes, FileError> {
    Err(FileError::Read)
}

#[cfg(not(any(unix, windows)))]
pub(crate) unsafe fn write_file(_path: &[u8], _contents: &[u8]) -> Result<(), FileError> {
    Err(FileError::Write)
}

#[cfg(all(unix, any(target_os = "linux", target_os = "android")))]
unsafe fn last_errno() -> i32 {
    *__errno_location()
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
unsafe fn last_errno() -> i32 {
    *__error()
}

#[cfg(unix)]
extern "C" {
    fn open(path: *const u8, flags: i32, mode: u32) -> i32;
    fn read(descriptor: i32, bytes: *mut c_void, byte_length: usize) -> isize;
    fn write(descriptor: i32, bytes: *const c_void, byte_length: usize) -> isize;
    fn close(descriptor: i32) -> i32;
}

#[cfg(all(unix, any(target_os = "linux", target_os = "android")))]
extern "C" {
    fn __errno_location() -> *mut i32;
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
extern "C" {
    fn __error() -> *mut i32;
}

#[cfg(windows)]
extern "system" {
    fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        security: *mut c_void,
        creation: u32,
        attributes: u32,
        template: *mut c_void,
    ) -> *mut c_void;
    fn ReadFile(
        handle: *mut c_void,
        buffer: *mut c_void,
        length: u32,
        read: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn WriteFile(
        handle: *mut c_void,
        buffer: *const c_void,
        length: u32,
        written: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn CloseHandle(handle: *mut c_void) -> i32;
}
