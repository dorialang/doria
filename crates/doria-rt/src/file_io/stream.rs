use crate::byte_io::{self, ReadProgress, WriteProgress};
use crate::native_handle::{self, NativeHandle, OwnedHandle, ReadinessSource};
use crate::platform_io::Failure;

use super::FileError;

pub(crate) mod locking;
#[path = "stream_platform.rs"]
mod platform;
pub(crate) mod transport;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileAccess {
    Read,
    Write,
    ReadWrite,
    /// Internal compatibility mode for the existing append-file intrinsic.
    AppendOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileCreation {
    OpenExisting,
    OpenOrCreate,
    CreateNew,
    TruncateExisting,
    CreateOrTruncate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileSharing {
    ReadWrite,
    ReadOnly,
    Exclusive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FileOpenRequest {
    pub(crate) access: FileAccess,
    pub(crate) creation: FileCreation,
    pub(crate) append: bool,
    pub(crate) sharing: FileSharing,
}

impl FileOpenRequest {
    pub(crate) const READ: Self = Self {
        access: FileAccess::Read,
        creation: FileCreation::OpenExisting,
        append: false,
        sharing: FileSharing::ReadWrite,
    };

    fn validate(self) -> Result<(), FileError> {
        if (self.access == FileAccess::Read
            && (self.append || self.creation != FileCreation::OpenExisting))
            // AppendOnly is solely the legacy append-file compatibility path,
            // not an extra seekable/truncating public access mode. Keep the
            // exact existing request and its narrower Windows permissions.
            || (self.access == FileAccess::AppendOnly
                && (!self.append || self.creation != FileCreation::OpenOrCreate))
        {
            Err(FileError::Open(Failure::invalid_input()))
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SeekOrigin {
    Start,
    Current,
    End,
}

/// Private runtime owner, not a new public Doria type or stable ABI layout.
/// Transfers borrow caller storage only for the call; the file has no buffer,
/// retained region, control-block allocation, or eager readiness machinery.
pub struct OwnedFile {
    handle: OwnedHandle,
    append: bool,
    readiness: ReadinessSource,
    /// The whole-file lock this owner holds through a Doria lock guard.
    lock: Option<locking::LockMode>,
}

impl OwnedFile {
    pub(crate) fn readiness_source(&self) -> &ReadinessSource {
        &self.readiness
    }

    /// Pure borrowed endpoint projection: no syscall, allocation or new close
    /// claim. The complete file owner must outlive every retained projection.
    pub(crate) fn native_handle(&self) -> NativeHandle {
        self.handle.get()
    }

    /// Private pending-file admission checks actual endpoint metadata once.
    /// Later preparation validates the captured handle with the pure getter;
    /// this does not narrow ordinary synchronous File operations.
    pub(crate) fn pending_regular_handle(&self) -> Result<NativeHandle, Failure> {
        let handle = self.native_handle();
        if native_handle::is_regular_file(handle)? {
            Ok(handle)
        } else {
            Err(Failure::unsupported())
        }
    }

    /// Wraps the parent end of a child pipe: a blocking byte stream with no
    /// path. Pending admission rejects it as a non-regular endpoint.
    pub(crate) fn adopt_stream(handle: OwnedHandle) -> Self {
        let readiness = ReadinessSource::new(handle.get());
        Self {
            handle,
            append: false,
            readiness,
            lock: None,
        }
    }

    pub(crate) fn open(path: &[u8], request: FileOpenRequest) -> Result<Self, FileError> {
        // Reject invalid options before allocating a path or touching the file.
        request.validate()?;
        let handle = unsafe { super::open_file(path, request)? };
        let readiness = ReadinessSource::new(handle.get());
        Ok(Self {
            handle,
            append: request.append,
            readiness,
            lock: None,
        })
    }

    pub(crate) fn read(&mut self, destination: &mut [u8]) -> Result<ReadProgress, Failure> {
        unsafe {
            byte_io::read(
                self.handle.get(),
                destination.as_mut_ptr(),
                destination.len(),
            )
        }
    }

    pub(crate) fn write(&mut self, source: &[u8]) -> Result<WriteProgress, Failure> {
        unsafe {
            if self.append {
                byte_io::append(self.handle.get(), source.as_ptr(), source.len())
            } else {
                byte_io::write(self.handle.get(), source.as_ptr(), source.len())
            }
        }
    }

    pub(crate) fn seek(&mut self, offset: i64, origin: SeekOrigin) -> Result<u64, Failure> {
        if origin == SeekOrigin::Start && offset < 0 {
            return Err(Failure::invalid_input());
        }
        platform::seek(self.handle.get(), offset, origin)
    }

    pub(crate) fn position(&self) -> Result<u64, Failure> {
        platform::seek(self.handle.get(), 0, SeekOrigin::Current)
    }

    pub(crate) fn length(&self) -> Result<u64, Failure> {
        platform::length(self.handle.get())
    }

    pub(crate) fn set_length(&mut self, length: i64) -> Result<(), Failure> {
        if length < 0 {
            return Err(Failure::invalid_input());
        }
        platform::set_length(self.handle.get(), length)
    }

    pub(crate) fn flush(&mut self) -> Result<(), Failure> {
        // This unbuffered owner has no runtime bytes to flush. In particular,
        // neither FlushFileBuffers nor fsync is a buffer-flush substitute.
        Ok(())
    }

    pub(crate) fn sync_data(&mut self) -> Result<(), Failure> {
        platform::sync(self.handle.get(), false)
    }

    pub(crate) fn sync_metadata(&mut self) -> Result<(), Failure> {
        platform::sync(self.handle.get(), true)
    }

    pub(crate) fn close(self) -> Result<(), Failure> {
        // Consumes the owner even when the single close attempt fails.
        self.handle.close()
    }
}

#[cfg(test)]
#[path = "stream_tests.rs"]
mod tests;

#[cfg(all(test, any(unix, windows)))]
mod pending_metadata_tests {
    use super::*;

    fn owned(file: std::fs::File) -> OwnedFile {
        #[cfg(unix)]
        let raw = {
            use std::os::fd::IntoRawFd;
            file.into_raw_fd()
        };
        #[cfg(windows)]
        let raw = {
            use std::os::windows::io::IntoRawHandle;
            file.into_raw_handle()
        };
        OwnedFile {
            // Transfer the one genuine host owner; admission never duplicates
            // it, and the readiness value is only borrowed wait metadata.
            handle: unsafe { OwnedHandle::from_raw(raw) },
            append: false,
            readiness: ReadinessSource::new(raw),
            lock: None,
        }
    }

    #[test]
    fn pending_file_admission_and_pure_getter_preserve_owner_and_cursor() {
        let file = owned(std::fs::File::open(std::env::current_exe().unwrap()).unwrap());
        let handle = file.native_handle();
        let readiness = file.readiness_source().value();
        let position = file.position().unwrap();
        let (((), allocations), closes) = native_handle::with_close_probe(|| {
            crate::probe_runtime_allocations(|| {
                for _ in 0..8 {
                    assert_eq!(file.native_handle(), handle);
                    assert_eq!(file.pending_regular_handle(), Ok(handle));
                    assert_eq!(file.readiness_source().value(), readiness);
                }
            })
        });
        assert_eq!(allocations, 0);
        assert_eq!(closes, 0);
        assert_eq!(file.position().unwrap(), position);
        assert!(file.length().unwrap() > 0);
        let (result, closes) = native_handle::with_close_probe(|| file.close());
        assert_eq!(result, Ok(()));
        assert_eq!(closes, 1);
    }

    #[test]
    fn nonregular_pending_admission_does_not_narrow_synchronous_file_operations() {
        #[cfg(unix)]
        let path = "/dev/null";
        #[cfg(windows)]
        let path = "NUL";
        let mut file = owned(std::fs::File::open(path).unwrap());
        let handle = file.native_handle();
        let failure = file.pending_regular_handle().unwrap_err();
        assert_eq!(failure.reason, crate::platform_io::Reason::Unsupported);
        assert_eq!(failure.system_code, None);
        assert_eq!(failure.transferred_byte_count, None);
        assert_eq!(file.native_handle(), handle);
        // This device remains usable on the existing synchronous path, even
        // though the new private pending-file path cannot admit it.
        assert_eq!(file.read(&mut [0; 1]), Ok(ReadProgress::EndOfStream));
        let ((), closes) = native_handle::with_close_probe(|| drop(file));
        assert_eq!(closes, 1);
    }
}
