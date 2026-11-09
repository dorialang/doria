#[cfg(windows)]
use core::ffi::c_void;
use core::ptr;

#[cfg(unix)]
use libc::{
    open, unlink, O_APPEND, O_CLOEXEC, O_CREAT, O_EXCL, O_RDONLY, O_RDWR, O_TRUNC, O_WRONLY,
};

#[cfg(any(unix, windows))]
use crate::byte_io::{self, ReadProgress};
use crate::native_handle::OwnedHandle;
#[cfg(any(unix, windows))]
use crate::platform_io::last_failure;

mod checked_stream;
pub(crate) mod stream;
use stream::{FileAccess, FileCreation, FileOpenRequest, FileSharing, OwnedFile};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FileError {
    PathNul,
    Open(crate::platform_io::Failure),
    Read(crate::platform_io::Failure),
    Write(crate::platform_io::Failure),
    Allocation,
}

pub(crate) struct OwnedBytes {
    pub(crate) bytes: *mut u8,
    pub(crate) length: usize,
}

impl OwnedBytes {
    pub(crate) fn into_raw_parts(self) -> (*mut u8, usize) {
        let result = (self.bytes, self.length);
        core::mem::forget(self);
        result
    }
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

#[cfg(any(unix, windows))]
pub(crate) unsafe fn read_file(path: &[u8]) -> Result<OwnedBytes, FileError> {
    let mut file = OwnedFile::open(
        path,
        FileOpenRequest {
            sharing: FileSharing::ReadOnly,
            ..FileOpenRequest::READ
        },
    )?;
    let result = read_with(|destination, capacity| {
        file.read(core::slice::from_raw_parts_mut(destination, capacity))
    });
    finish_read(result, file.close())
}

#[cfg(any(unix, windows))]
unsafe fn read_with(
    mut transfer: impl FnMut(*mut u8, usize) -> Result<ReadProgress, crate::platform_io::Failure>,
) -> Result<OwnedBytes, FileError> {
    let mut capacity = 4096_usize;
    let mut contents = OwnedBytes {
        bytes: super::allocate(capacity),
        length: 0,
    };
    if contents.bytes.is_null() {
        return Err(FileError::Allocation);
    }
    loop {
        if contents.length == capacity {
            let next = capacity.checked_mul(2).ok_or(FileError::Allocation)?;
            let replacement = super::allocate(next);
            if replacement.is_null() {
                return Err(FileError::Allocation);
            }
            ptr::copy_nonoverlapping(contents.bytes, replacement, contents.length);
            super::deallocate(contents.bytes);
            contents.bytes = replacement;
            capacity = next;
        }
        let remaining = capacity - contents.length;
        match transfer(contents.bytes.add(contents.length), remaining) {
            Ok(ReadProgress::Data(count)) if count <= remaining => contents.length += count,
            Ok(ReadProgress::Data(_)) => {
                return Err(FileError::Read(crate::platform_io::Failure::other()))
            }
            Ok(ReadProgress::EndOfStream) => return Ok(contents),
            Ok(ReadProgress::WouldBlock(failure)) | Err(failure) => {
                let failure = if failure
                    .transferred_byte_count
                    .is_some_and(|n| n > remaining)
                {
                    failure.with_transferred_byte_count(None)
                } else {
                    failure.after_transferred(contents.length)
                };
                return Err(FileError::Read(failure));
            }
        }
    }
}

#[cfg(any(unix, windows))]
fn finish_read(
    result: Result<OwnedBytes, FileError>,
    close: Result<(), crate::platform_io::Failure>,
) -> Result<OwnedBytes, FileError> {
    let contents = result?;
    close.map_err(|failure| {
        FileError::Read(failure.with_transferred_byte_count(Some(contents.length)))
    })?;
    Ok(contents)
}

#[cfg(any(unix, windows))]
fn finish_write(
    result: Result<(), FileError>,
    close: Result<(), crate::platform_io::Failure>,
    length: usize,
) -> Result<(), FileError> {
    result?;
    close.map_err(|failure| FileError::Write(failure.with_transferred_byte_count(Some(length))))
}

#[cfg(any(unix, windows))]
pub(crate) unsafe fn write_file(path: &[u8], contents: &[u8]) -> Result<(), FileError> {
    write_file_mode(path, contents, false)
}

#[cfg(any(unix, windows))]
pub(crate) unsafe fn append_file(path: &[u8], contents: &[u8]) -> Result<(), FileError> {
    write_file_mode(path, contents, true)
}

#[cfg(unix)]
pub(crate) unsafe fn remove_file(path: &[u8]) -> Result<(), FileError> {
    let path = unix_path(path)?;
    let result = unlink(path.bytes.cast());
    if result == 0 {
        Ok(())
    } else {
        Err(FileError::Write(last_failure()))
    }
}

#[cfg(any(unix, windows))]
unsafe fn write_file_mode(path: &[u8], contents: &[u8], append: bool) -> Result<(), FileError> {
    let mut file = OwnedFile::open(
        path,
        FileOpenRequest {
            access: if append {
                FileAccess::AppendOnly
            } else {
                FileAccess::Write
            },
            creation: if append {
                FileCreation::OpenOrCreate
            } else {
                FileCreation::CreateOrTruncate
            },
            append,
            sharing: FileSharing::Exclusive,
        },
    )?;
    let result = byte_io::write_all(contents.len(), |offset, length| {
        file.write(&contents[offset..offset + length])
    })
    .map_err(FileError::Write);
    finish_write(result, file.close(), contents.len())
}

#[cfg(unix)]
unsafe fn open_file(path: &[u8], request: FileOpenRequest) -> Result<OwnedHandle, FileError> {
    let path = unix_path(path)?;
    let access = match request.access {
        FileAccess::Read => O_RDONLY,
        FileAccess::Write | FileAccess::AppendOnly => O_WRONLY,
        FileAccess::ReadWrite => O_RDWR,
    };
    let creation = match request.creation {
        FileCreation::OpenExisting => 0,
        FileCreation::OpenOrCreate => O_CREAT,
        FileCreation::CreateNew => O_CREAT | O_EXCL,
        FileCreation::TruncateExisting => O_TRUNC,
        FileCreation::CreateOrTruncate => O_CREAT | O_TRUNC,
    };
    // Runtime ownership must not leak into an exec'd child. Set this atomically
    // during open, not through a racy open-then-fcntl sequence.
    let flags = access | creation | O_CLOEXEC | if request.append { O_APPEND } else { 0 };
    // C varargs promote Darwin's 16-bit mode_t to int; the permission bits
    // also fit the Linux mode_t argument without changing their value.
    let descriptor = open(path.bytes.cast(), flags, 0o666 as libc::c_int);
    if descriptor < 0 {
        Err(FileError::Open(last_failure()))
    } else {
        Ok(OwnedHandle::from_raw(descriptor))
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

#[cfg(windows)]
const GENERIC_READ: u32 = 0x8000_0000;
#[cfg(windows)]
const GENERIC_WRITE: u32 = 0x4000_0000;
#[cfg(windows)]
const FILE_APPEND_DATA: u32 = 0x0000_0004;
#[cfg(windows)]
const FILE_SHARE_READ: u32 = 1;
#[cfg(windows)]
const FILE_SHARE_WRITE: u32 = 2;
#[cfg(windows)]
const OPEN_EXISTING: u32 = 3;
#[cfg(windows)]
const CREATE_ALWAYS: u32 = 2;
#[cfg(windows)]
const OPEN_ALWAYS: u32 = 4;
#[cfg(windows)]
const CREATE_NEW: u32 = 1;
#[cfg(windows)]
const TRUNCATE_EXISTING: u32 = 5;
#[cfg(windows)]
const FILE_ATTRIBUTE_NORMAL: u32 = 0x80;
#[cfg(windows)]
const INVALID_HANDLE_VALUE: *mut c_void = -1_isize as *mut c_void;

#[cfg(windows)]
unsafe fn open_file(path: &[u8], request: FileOpenRequest) -> Result<OwnedHandle, FileError> {
    let path = windows_path(path)?;
    let access = match request.access {
        FileAccess::Read => GENERIC_READ,
        FileAccess::Write => GENERIC_WRITE,
        FileAccess::ReadWrite => GENERIC_READ | GENERIC_WRITE,
        // Preserve the legacy append helper's append-only permission contract.
        FileAccess::AppendOnly => FILE_APPEND_DATA,
    };
    let creation = match request.creation {
        FileCreation::OpenExisting => OPEN_EXISTING,
        FileCreation::OpenOrCreate => OPEN_ALWAYS,
        FileCreation::CreateNew => CREATE_NEW,
        FileCreation::TruncateExisting => TRUNCATE_EXISTING,
        FileCreation::CreateOrTruncate => CREATE_ALWAYS,
    };
    let handle = CreateFileW(
        path.bytes.cast::<u16>(),
        access,
        match request.sharing {
            FileSharing::ReadWrite => FILE_SHARE_READ | FILE_SHARE_WRITE,
            FileSharing::ReadOnly => FILE_SHARE_READ,
            FileSharing::Exclusive => 0,
        },
        ptr::null_mut(),
        creation,
        FILE_ATTRIBUTE_NORMAL,
        ptr::null_mut(),
    );
    if handle == INVALID_HANDLE_VALUE {
        Err(FileError::Open(last_failure()))
    } else {
        Ok(OwnedHandle::from_raw(handle))
    }
}

#[cfg(windows)]
pub(crate) unsafe fn remove_file(path: &[u8]) -> Result<(), FileError> {
    let path = windows_path(path)?;
    let result = DeleteFileW(path.bytes.cast::<u16>());
    if result != 0 {
        Ok(())
    } else {
        Err(FileError::Write(last_failure()))
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
unsafe fn open_file(_path: &[u8], _request: FileOpenRequest) -> Result<OwnedHandle, FileError> {
    Err(FileError::Open(crate::platform_io::Failure::unsupported()))
}

#[cfg(not(any(unix, windows)))]
pub(crate) unsafe fn read_file(_path: &[u8]) -> Result<OwnedBytes, FileError> {
    Err(FileError::Read(crate::platform_io::Failure::unsupported()))
}

#[cfg(not(any(unix, windows)))]
pub(crate) unsafe fn write_file(_path: &[u8], _contents: &[u8]) -> Result<(), FileError> {
    Err(FileError::Write(crate::platform_io::Failure::unsupported()))
}

#[cfg(not(any(unix, windows)))]
pub(crate) unsafe fn append_file(_path: &[u8], _contents: &[u8]) -> Result<(), FileError> {
    Err(FileError::Write(crate::platform_io::Failure::unsupported()))
}

#[cfg(not(any(unix, windows)))]
pub(crate) unsafe fn remove_file(_path: &[u8]) -> Result<(), FileError> {
    Err(FileError::Write(crate::platform_io::Failure::unsupported()))
}

#[cfg(windows)]
use crate::win32::{CreateFileW, DeleteFileW};

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn path(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "doria-stage17-{}-{name}-Dória-漢字",
            std::process::id()
        ))
    }

    fn path_bytes(path: &std::path::Path) -> Vec<u8> {
        path.to_string_lossy().as_bytes().to_vec()
    }

    #[test]
    fn file_layer_creates_appends_truncates_and_preserves_exact_bytes() {
        let path = path("roundtrip");
        let path_bytes = path_bytes(&path);
        unsafe {
            write_file(&path_bytes, b"long initial contents").expect("initial write");
            write_file(&path_bytes, "Dória\n漢字\0🎮".as_bytes()).expect("truncate write");
            append_file(&path_bytes, b"\0\x80\xff").expect("binary append");
            let contents = read_file(&path_bytes).expect("round-trip read");
            let mut expected = "Dória\n漢字\0🎮".as_bytes().to_vec();
            expected.extend_from_slice(b"\0\x80\xff");
            assert_eq!(
                core::slice::from_raw_parts(contents.bytes, contents.length),
                expected
            );
            write_file(&path_bytes, b"").expect("empty write");
            let empty = read_file(&path_bytes).expect("empty read");
            assert_eq!(empty.length, 0);
            remove_file(&path_bytes).expect("runtime fixture cleanup");
        }
        assert!(!path.exists());
    }

    #[test]
    fn text_file_layer_reports_missing_and_embedded_nul_paths() {
        let missing = path("missing");
        let _ = fs::remove_file(&missing);
        unsafe {
            assert!(matches!(
                read_file(&path_bytes(&missing)),
                Err(FileError::Open(crate::platform_io::Failure {
                    reason: crate::platform_io::Reason::NotFound,
                    system_code: Some(_),
                    transferred_byte_count: None,
                }))
            ));
            assert!(matches!(read_file(b"bad\0path"), Err(FileError::PathNul)));
            assert!(matches!(
                write_file(b"bad\0path", b"x"),
                Err(FileError::PathNul)
            ));
            assert!(matches!(
                append_file(b"bad\0path", b"x"),
                Err(FileError::PathNul)
            ));
            assert!(matches!(remove_file(b"bad\0path"), Err(FileError::PathNul)));
        }
    }

    #[test]
    fn file_reads_preserve_bytes_across_repeated_buffer_growth() {
        let path = path("growth");
        let path_bytes = path_bytes(&path);
        let expected: Vec<u8> = (0..20_003).map(|index| (index % 251) as u8).collect();
        unsafe {
            write_file(&path_bytes, &expected).expect("large write");
            let contents = read_file(&path_bytes).expect("growing read");
            assert_eq!(contents.length, expected.len());
            assert_eq!(
                core::slice::from_raw_parts(contents.bytes, contents.length),
                expected
            );
            remove_file(&path_bytes).expect("fixture cleanup");
        }
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn aggregate_reads_preserve_known_failure_progress_without_inventing_unknown_counts() {
        for raw in [None, Some(0), Some(2)] {
            for would_block in [false, true] {
                let failure = crate::platform_io::Failure::from_system_code(5)
                    .with_transferred_byte_count(raw);
                let mut calls = 0;
                let result = unsafe {
                    read_with(|destination, capacity| {
                        calls += 1;
                        if calls == 1 {
                            assert!(capacity >= 3);
                            ptr::copy_nonoverlapping(b"abc".as_ptr(), destination, 3);
                            Ok(ReadProgress::Data(3))
                        } else {
                            if raw == Some(2) {
                                ptr::copy_nonoverlapping(b"de".as_ptr(), destination, 2);
                            }
                            if would_block {
                                Ok(ReadProgress::WouldBlock(failure))
                            } else {
                                Err(failure)
                            }
                        }
                    })
                };
                assert_eq!(
                    result.err(),
                    Some(FileError::Read(failure.after_transferred(3)))
                );
                assert_eq!(calls, 2);
            }
        }
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn close_failures_preserve_successful_transfer_counts_and_primary_failures() {
        let close = crate::platform_io::Failure::from_system_code(5);
        let primary =
            crate::platform_io::Failure::from_system_code(13).with_transferred_byte_count(Some(2));
        assert_eq!(
            finish_write(Ok(()), Err(close), 7),
            Err(FileError::Write(close.with_transferred_byte_count(Some(7))))
        );
        assert_eq!(
            finish_write(Ok(()), Err(close), 0),
            Err(FileError::Write(close.with_transferred_byte_count(Some(0))))
        );
        assert_eq!(
            finish_write(Err(FileError::Write(primary)), Err(close), 7),
            Err(FileError::Write(primary))
        );
        assert_eq!(
            finish_read(Err(FileError::Read(primary)), Err(close)).err(),
            Some(FileError::Read(primary))
        );
        let mut calls = 0;
        let contents = unsafe {
            read_with(|destination, _| {
                calls += 1;
                if calls == 1 {
                    ptr::copy_nonoverlapping(b"abc".as_ptr(), destination, 3);
                    Ok(ReadProgress::Data(3))
                } else {
                    Ok(ReadProgress::EndOfStream)
                }
            })
        };
        assert_eq!(
            finish_read(contents, Err(close)).err(),
            Some(FileError::Read(close.with_transferred_byte_count(Some(3))))
        );
    }
}
