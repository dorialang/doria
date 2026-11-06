#[cfg(any(unix, windows))]
use crate::platform_io::last_failure;
use crate::platform_io::Failure;

#[cfg(unix)]
pub(crate) type NativeHandle = i32;
#[cfg(windows)]
pub(crate) type NativeHandle = *mut core::ffi::c_void;
#[cfg(not(any(unix, windows)))]
pub(crate) type NativeHandle = usize;

/// Proves that a real inherited device accepts synchronous byte operations.
/// File kind alone is not opening-mode evidence. This borrows native metadata;
/// it never changes a mode, replaces/associates a handle, or acquires ownership.
/// Call on the issuing helper for pending work, not the driving thread.
#[cfg(windows)]
pub(crate) fn require_synchronous_device(handle: NativeHandle) -> Result<(), Failure> {
    #[repr(C)]
    struct IoStatus {
        status: usize,
        information: usize,
    }
    type Query = unsafe extern "system" fn(
        NativeHandle,
        *mut IoStatus,
        *mut core::ffi::c_void,
        u32,
        u32,
    ) -> i32;
    type ErrorCode = unsafe extern "system" fn(i32) -> u32;
    unsafe extern "system" {
        fn GetConsoleMode(handle: NativeHandle, mode: *mut u32) -> i32;
        fn GetModuleHandleW(name: *const u16) -> NativeHandle;
        fn GetProcAddress(module: NativeHandle, name: *const u8) -> *mut core::ffi::c_void;
    }
    let mut mode = 0u32;
    if unsafe { GetConsoleMode(handle, &mut mode) } != 0 {
        return Ok(());
    }
    const NTDLL: &[u16] = &[110, 116, 100, 108, 108, 46, 100, 108, 108, 0];
    unsafe {
        // Already process-loaded; never add module ownership or a private
        // native link dependency to standalone compiler clients.
        let module = GetModuleHandleW(NTDLL.as_ptr());
        if module.is_null() {
            return Err(last_failure());
        }
        let query = GetProcAddress(module, c"NtQueryInformationFile".as_ptr().cast());
        if query.is_null() {
            return Err(last_failure());
        }
        let error = GetProcAddress(module, c"RtlNtStatusToDosError".as_ptr().cast());
        if error.is_null() {
            return Err(last_failure());
        }
        let query: Query = core::mem::transmute(query);
        let error: ErrorCode = core::mem::transmute(error);
        let mut status = IoStatus {
            status: 0,
            information: 0,
        };
        // FileModeInformation(16) reports actual CreateOptions, independent of
        // disk/pipe classification. The query contract returns final status.
        // https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntqueryinformationfile
        let code = query(handle, &mut status, (&mut mode as *mut u32).cast(), 4, 16);
        if code != 0 {
            return Err(Failure::from_system_code(error(code).into()));
        }
    }
    if mode & 0x30 != 0 {
        Ok(())
    } else {
        Err(Failure::unsupported())
    }
}

/// Classifies the actual borrowed endpoint without acquiring a close claim or
/// changing its cursor. Pending-file admission and advisory readiness share
/// this metadata owner; neither may infer a file kind from readiness bits.
#[cfg(unix)]
pub(crate) fn is_regular_file(handle: NativeHandle) -> Result<bool, Failure> {
    let mut status = core::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(handle, status.as_mut_ptr()) } < 0 {
        return Err(last_failure());
    }
    Ok(unsafe { status.assume_init() }.st_mode & libc::S_IFMT == libc::S_IFREG)
}

#[cfg(windows)]
pub(crate) fn is_regular_file(handle: NativeHandle) -> Result<bool, Failure> {
    // Win32 FILE_STANDARD_INFO, including its byte-sized BOOLEAN fields:
    // https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_standard_info
    #[repr(C)]
    struct FileStandardInfo {
        allocation_size: i64,
        end_of_file: i64,
        number_of_links: u32,
        delete_pending: u8,
        directory: u8,
    }
    unsafe extern "system" {
        fn GetFileType(handle: NativeHandle) -> u32;
        fn SetLastError(code: u32);
        fn GetLastError() -> u32;
        fn GetFileInformationByHandleEx(
            handle: NativeHandle,
            class: i32,
            information: *mut core::ffi::c_void,
            size: u32,
        ) -> i32;
    }
    const FILE_TYPE_UNKNOWN: u32 = 0;
    const FILE_TYPE_DISK: u32 = 1;
    const FILE_STANDARD_INFO: i32 = 1;

    // UNKNOWN may be a valid type or a failure. Clear the per-thread code so
    // a valid unknown endpoint cannot inherit an unrelated host failure.
    let kind = unsafe {
        SetLastError(0);
        GetFileType(handle)
    };
    if kind == FILE_TYPE_UNKNOWN {
        let code = unsafe { GetLastError() };
        if code != 0 {
            return Err(Failure::from_system_code(i64::from(code)));
        }
        return Ok(false);
    }
    if kind != FILE_TYPE_DISK {
        // In particular, never query FileStandardInfo on a pipe handle.
        return Ok(false);
    }

    // DISK also includes directories; only this actual metadata flag admits a
    // regular file. Failure is not permission to fall back to the broad type.
    let mut status = core::mem::MaybeUninit::<FileStandardInfo>::uninit();
    if unsafe {
        GetFileInformationByHandleEx(
            handle,
            FILE_STANDARD_INFO,
            status.as_mut_ptr().cast(),
            core::mem::size_of::<FileStandardInfo>() as u32,
        )
    } == 0
    {
        return Err(last_failure());
    }
    Ok(unsafe { status.assume_init() }.directory == 0)
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn is_regular_file(_: NativeHandle) -> Result<bool, Failure> {
    Err(Failure::unsupported())
}

/// Group access classification belongs to the borrowed native endpoint. This
/// is a setup-only metadata query; it neither duplicates nor closes the owner.
pub(crate) fn transfer_resource(
    handle: NativeHandle,
) -> Result<crate::transfer_group::TransferResource, Failure> {
    Ok(crate::transfer_group::TransferResource {
        identity: handle as usize,
        shared_cursor: is_regular_file(handle)?,
    })
}

/// Private raw-owner/fixture wait metadata, without a public physical origin.
/// The raw owner may move before a loan; neither this word nor its home has a
/// close obligation. It is NOT the compiler-owned ReadinessSource projection.
#[repr(transparent)]
pub(crate) struct ReadinessSource(usize);

impl ReadinessSource {
    pub(crate) fn new(handle: NativeHandle) -> Self {
        // Descriptor zero is valid. Reserve the absent opaque value without
        // allocating, rebasing a self-pointer or exposing a public handle.
        #[cfg(unix)]
        let word = handle as usize + 1;
        #[cfg(not(unix))]
        let word = handle as usize;
        Self(word)
    }

    pub(crate) fn value(&self) -> *const core::ffi::c_void {
        self.0 as *const core::ffi::c_void
    }

    pub(crate) fn handle(value: *const core::ffi::c_void) -> Result<NativeHandle, Failure> {
        if value.is_null() {
            return Err(Failure::invalid_input());
        }
        #[cfg(unix)]
        {
            i32::try_from(value as usize - 1).map_err(|_| Failure::invalid_input())
        }
        #[cfg(not(unix))]
        {
            Ok(value as NativeHandle)
        }
    }
}

/// Physical file facts. Public sources name the stable private resource, not
/// the replaceable wrapper. Raw test/unsafe-owner projections have no resource
/// and retain their explicit complete-owner lifetime contract. Merely copying
/// these facts is not retention; routes and selected work own explicit pins.
#[derive(Clone, Copy)]
pub(crate) struct FileOrigin {
    pub(crate) payload: *mut core::ffi::c_void,
    pub(crate) path: *mut crate::DrStringV1,
    pub(crate) resource: *mut crate::file_io::stream::transport::object::FileResource,
}

impl FileOrigin {
    /// The IoTarget tag its failures report: a file, or a child pipe.
    pub(crate) fn error_target(self) -> u8 {
        if self.resource.is_null() {
            crate::checked_io::TARGET_FILE
        } else {
            unsafe { (*self.resource).target() }
        }
    }
}

/// Borrowed physical provenance, independent of any close obligation. Standard
/// devices have neither a file payload nor a filename. Forwarding a source must
/// preserve this variant rather than reconstructing an owner from handle bits.
#[derive(Clone, Copy)]
pub(crate) enum PhysicalOrigin {
    File {
        file: *mut crate::file_io::stream::OwnedFile,
        origin: FileOrigin,
    },
    Standard(crate::device_io::StandardStream),
}

impl PhysicalOrigin {
    pub(crate) fn file(self) -> Option<(*mut crate::file_io::stream::OwnedFile, FileOrigin)> {
        match self {
            Self::File { file, origin } => Some((file, origin)),
            Self::Standard(_) => None,
        }
    }

    /// # Safety
    /// The file owner is live and retained, or the canonical standard device is
    /// exclusively controlled against endpoint replacement during this use.
    pub(crate) unsafe fn handle(self) -> Result<NativeHandle, Failure> {
        match self {
            Self::File { file, origin } => {
                if origin.resource.is_null() {
                    Ok((*file).native_handle())
                } else {
                    (*origin.resource).handle()
                }
            }
            Self::Standard(stream) => crate::device_io::byte_handle(stream),
        }
    }

    /// Stable physical source identity for rebinding, distinct from handle
    /// identity used for competition (standard outputs may share a handle).
    pub(crate) fn same_source(self, other: Self) -> bool {
        match (self, other) {
            (
                Self::File {
                    file: a,
                    origin: ao,
                },
                Self::File {
                    file: b,
                    origin: bo,
                },
            ) => {
                if ao.resource.is_null() || bo.resource.is_null() {
                    ao.resource == bo.resource && a == b && ao.payload == bo.payload
                } else {
                    ao.resource == bo.resource
                }
            }
            (Self::Standard(a), Self::Standard(b)) => a == b,
            _ => false,
        }
    }

    /// # Safety
    /// Source is live under the current owner or existing metadata pin. Raw
    /// sources remain caller-retained; standard sources are static.
    pub(crate) unsafe fn metadata_pin(
        self,
    ) -> Option<crate::file_io::stream::transport::object::ResourcePin> {
        match self {
            Self::File { origin, .. } if !origin.resource.is_null() => Some(
                crate::file_io::stream::transport::object::ResourcePin::metadata(origin.resource),
            ),
            _ => None,
        }
    }

    /// # Safety
    /// Source is current and exclusively selected by the owner/group. Release
    /// only after actual native retirement and adapter completion.
    pub(crate) unsafe fn selected_pin(
        self,
    ) -> Result<Option<crate::file_io::stream::transport::object::ResourcePin>, Failure> {
        match self {
            Self::File { origin, .. } if !origin.resource.is_null() => {
                crate::file_io::stream::transport::object::ResourcePin::selected(origin.resource)
                    .map(Some)
            }
            _ => Ok(None),
        }
    }

    /// # Safety
    /// The endpoint is live under the current owner/selected use. Public File
    /// kind is cached at its resource home, never re-queried per chunk/getter.
    pub(crate) unsafe fn is_regular(self) -> Result<bool, Failure> {
        match self {
            Self::File { origin, .. } if !origin.resource.is_null() => {
                (*origin.resource).is_regular()
            }
            _ => is_regular_file(self.handle()?),
        }
    }
}

/// The public opaque readiness value points to this typed, resource-embedded
/// projection, NOT to a platform handle or raw fixture source. Its first word
/// is the exact borrowed value home required by compiler borrow forwarding.
/// Initialize once at its final stable resource home. Formation stays immutable
/// and nonretaining; explicit route/use pins extend the resource's lifetime.
#[repr(C)]
pub(crate) struct ReadinessProjection {
    value: *const Self,
    pub(crate) origin: PhysicalOrigin,
}

impl ReadinessProjection {
    /// # Safety
    /// Storage is uninitialized embedded metadata at its final stable address.
    /// Payload/file/path are the actual live fields of that complete owner.
    /// Fixed-owner test fixtures only; production File projections are resource-backed.
    #[cfg(test)]
    pub(crate) unsafe fn initialize_file(
        storage: *mut Self,
        payload: *mut core::ffi::c_void,
        file: *mut crate::file_io::stream::OwnedFile,
        path: *mut crate::DrStringV1,
    ) {
        storage.write(Self {
            value: storage,
            origin: PhysicalOrigin::File {
                file,
                origin: FileOrigin {
                    payload,
                    path,
                    resource: core::ptr::null_mut(),
                },
            },
        });
    }

    /// # Safety
    /// All fields are at the final live resource address. Only the File runtime
    /// may construct a resource-backed projection; fixed-owner test fixtures use
    /// their separate initializer and do not gain a resource lifetime claim.
    pub(crate) unsafe fn initialize_resource(
        storage: *mut Self,
        payload: *mut core::ffi::c_void,
        file: *mut crate::file_io::stream::OwnedFile,
        path: *mut crate::DrStringV1,
        resource: *mut crate::file_io::stream::transport::object::FileResource,
    ) {
        storage.write(Self {
            value: storage,
            origin: PhysicalOrigin::File {
                file,
                origin: FileOrigin {
                    payload,
                    path,
                    resource,
                },
            },
        });
    }

    pub(crate) const fn standard(
        value: *const Self,
        stream: crate::device_io::StandardStream,
    ) -> Self {
        Self {
            value,
            origin: PhysicalOrigin::Standard(stream),
        }
    }

    pub(crate) fn value(&self) -> *const core::ffi::c_void {
        self.value.cast()
    }

    pub(crate) fn home(&self) -> *const core::ffi::c_void {
        core::ptr::from_ref(&self.value).cast()
    }

    /// # Safety
    /// A nonnull value is a live compiler-retained typed readiness projection,
    /// never raw handle bits, a raw fixture source, or an arbitrary class.
    /// The returned facts are borrowed only for the retained owner's lifetime.
    pub(crate) unsafe fn borrow<'a>(value: *const core::ffi::c_void) -> Result<&'a Self, Failure> {
        if value.is_null() || !(value as usize).is_multiple_of(core::mem::align_of::<Self>()) {
            return Err(Failure::invalid_input());
        }
        let source = &*value.cast::<Self>();
        let valid_origin = match source.origin {
            PhysicalOrigin::File { file, origin } => {
                !file.is_null() && !origin.payload.is_null() && !origin.path.is_null()
            }
            PhysicalOrigin::Standard(stream) => {
                core::ptr::eq(source, crate::device_io::standard_device(stream).source())
            }
        };
        if source.value() != value || !valid_origin {
            return Err(Failure::invalid_input());
        }
        Ok(source)
    }
}

/// Owns one device handle without allocating a separate control block.
pub(crate) struct OwnedHandle(Option<NativeHandle>);

impl OwnedHandle {
    /// # Safety
    /// `handle` must be a valid, independently owned device handle. No other
    /// owner may close it after this transfer.
    pub(crate) unsafe fn from_raw(handle: NativeHandle) -> Self {
        Self(Some(handle))
    }

    pub(crate) fn get(&self) -> NativeHandle {
        self.0.expect("live native handle")
    }

    /// Non-owning native endpoint projection. Its pointer names the actual
    /// handle cell, never a getter-local copy. The borrowing owner must remain
    /// live and unmoved until every registration/prepared loan has retired.
    #[cfg(all(test, unix))]
    pub(crate) fn native_handle_home(&self) -> *const core::ffi::c_void {
        self.0.as_ref().map_or(core::ptr::null(), |handle| {
            core::ptr::from_ref(handle).cast()
        })
    }

    pub(crate) fn close(self) -> Result<(), Failure> {
        self.close_with(|handle| unsafe { close(handle) })
    }

    fn close_with(
        mut self,
        close: impl FnOnce(NativeHandle) -> Result<(), Failure>,
    ) -> Result<(), Failure> {
        let handle = self.0.take().expect("live native handle");
        // Do not retry close, including EINTR: the host may already have freed
        // and reused the descriptor. Drop must never repeat this attempt.
        close(handle)
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            let _ = unsafe { close(handle) };
        }
    }
}

#[cfg(unix)]
unsafe fn close(handle: NativeHandle) -> Result<(), Failure> {
    #[cfg(test)]
    record_close();
    unsafe extern "C" {
        #[link_name = "close"]
        fn libc_close(handle: i32) -> i32;
    }
    if libc_close(handle) == 0 {
        Ok(())
    } else {
        Err(last_failure())
    }
}

#[cfg(windows)]
unsafe fn close(handle: NativeHandle) -> Result<(), Failure> {
    #[cfg(test)]
    record_close();
    unsafe extern "system" {
        fn CloseHandle(handle: NativeHandle) -> i32;
    }
    if CloseHandle(handle) != 0 {
        Ok(())
    } else {
        Err(last_failure())
    }
}

#[cfg(not(any(unix, windows)))]
unsafe fn close(_: NativeHandle) -> Result<(), Failure> {
    #[cfg(test)]
    record_close();
    Err(Failure::unsupported())
}

#[cfg(test)]
std::thread_local! {
    static CLOSE_ATTEMPTS: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn record_close() {
    CLOSE_ATTEMPTS.with(|count| {
        if let Some(value) = count.get() {
            count.set(Some(value + 1));
        }
    });
}

#[cfg(test)]
/// Counts this thread's close attempts over a scoped interval. A callback can
/// measure a nested interval without resetting its caller's inclusive total.
pub(crate) fn with_close_probe<R>(operation: impl FnOnce() -> R) -> (R, usize) {
    struct Probe {
        outermost: bool,
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            if self.outermost {
                CLOSE_ATTEMPTS.with(|count| count.set(None));
            }
        }
    }
    let (baseline, outermost) = CLOSE_ATTEMPTS.with(|count| match count.get() {
        Some(value) => (value, false),
        None => {
            count.set(Some(0));
            (0, true)
        }
    });
    let probe = Probe { outermost };
    let result = operation();
    let count = CLOSE_ATTEMPTS.with(|count| count.get().unwrap() - baseline);
    drop(probe);
    (result, count)
}

#[cfg(test)]
mod tests {
    #[test]
    fn nested_close_probes_measure_intervals_without_resetting_outer_total() {
        let ((inner, deepest), outer) = super::with_close_probe(|| {
            super::record_close();
            let (deepest, inner) = super::with_close_probe(|| {
                super::record_close();
                let ((), deepest) = super::with_close_probe(super::record_close);
                super::record_close();
                deepest
            });
            super::record_close();
            (inner, deepest)
        });
        assert_eq!((inner, deepest, outer), (3, 1, 5));
        let ((), fresh) = super::with_close_probe(|| {});
        assert_eq!(fresh, 0);
    }

    #[test]
    fn nested_close_probe_unwind_preserves_outer_counting() {
        let ((), outer) = super::with_close_probe(|| {
            let failure = std::panic::catch_unwind(|| {
                super::with_close_probe(|| {
                    super::record_close();
                    panic!("fixture nested probe unwind");
                });
            });
            assert!(failure.is_err());
            super::record_close();
        });
        assert_eq!(outer, 2);
        let ((), fresh) = super::with_close_probe(|| {});
        assert_eq!(fresh, 0);
    }

    #[test]
    fn outer_close_probe_unwind_restores_unobserved_state() {
        let failure = std::panic::catch_unwind(|| {
            super::with_close_probe(|| {
                super::record_close();
                panic!("fixture outer probe unwind");
            });
        });
        assert!(failure.is_err());
        super::CLOSE_ATTEMPTS.with(|count| assert_eq!(count.get(), None));
        let ((), fresh) = super::with_close_probe(|| {});
        assert_eq!(fresh, 0);
    }

    #[cfg(any(unix, windows))]
    fn borrowed(file: &std::fs::File) -> super::NativeHandle {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            file.as_raw_fd()
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            file.as_raw_handle()
        }
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn regular_metadata_distinguishes_files_from_directories_without_ownership() {
        let file = std::fs::File::open(std::env::current_exe().unwrap()).unwrap();
        let mut directory_options = std::fs::OpenOptions::new();
        directory_options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // FILE_FLAG_BACKUP_SEMANTICS permits a genuine directory handle.
            directory_options.custom_flags(0x0200_0000);
        }
        let directory = directory_options.open(std::env::temp_dir()).unwrap();
        let (((), allocations), closes) = super::with_close_probe(|| {
            crate::probe_runtime_allocations(|| {
                assert_eq!(super::is_regular_file(borrowed(&file)), Ok(true));
                assert_eq!(super::is_regular_file(borrowed(&directory)), Ok(false));
            })
        });
        assert_eq!(allocations, 0);
        assert_eq!(closes, 0);
        // Both original owners remain live and usable after classification.
        assert!(file.metadata().unwrap().is_file());
        assert!(directory.metadata().unwrap().is_dir());
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn invalid_endpoint_metadata_retains_actual_host_failure_without_progress() {
        #[cfg(unix)]
        let invalid = -1;
        #[cfg(windows)]
        let invalid = usize::MAX as super::NativeHandle;
        let failure = super::is_regular_file(invalid).unwrap_err();
        assert_eq!(failure.reason, crate::platform_io::Reason::Closed);
        #[cfg(unix)]
        assert_eq!(failure.system_code, Some(i64::from(libc::EBADF)));
        #[cfg(windows)]
        assert_eq!(failure.system_code, Some(6)); // ERROR_INVALID_HANDLE
        assert_eq!(failure.transferred_byte_count, None);
    }

    #[cfg(unix)]
    #[test]
    fn unix_device_and_pipe_metadata_are_not_regular_files() {
        let device = std::fs::File::open("/dev/null").unwrap();
        assert_eq!(super::is_regular_file(borrowed(&device)), Ok(false));
        let mut pipe = [-1; 2];
        assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
        let reader = unsafe { super::OwnedHandle::from_raw(pipe[0]) };
        let writer = unsafe { super::OwnedHandle::from_raw(pipe[1]) };
        assert_eq!(super::is_regular_file(reader.get()), Ok(false));
        assert_eq!(super::is_regular_file(writer.get()), Ok(false));
    }

    #[cfg(windows)]
    #[test]
    fn windows_regular_file_kind_does_not_prove_synchronous_opening_mode() {
        use std::os::windows::fs::OpenOptionsExt;
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "doria-opening-mode-{}-{unique}",
            std::process::id()
        ));
        let synchronous = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let asynchronous = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(0x4000_0000)
            .open(&path)
            .unwrap();
        assert_eq!(super::is_regular_file(borrowed(&synchronous)), Ok(true));
        assert_eq!(super::is_regular_file(borrowed(&asynchronous)), Ok(true));
        let (((), allocations), closes) = super::with_close_probe(|| {
            crate::probe_runtime_allocations(|| {
                assert_eq!(
                    super::require_synchronous_device(borrowed(&synchronous)),
                    Ok(())
                );
                assert_eq!(
                    super::require_synchronous_device(borrowed(&asynchronous)),
                    Err(crate::platform_io::Failure::unsupported())
                );
            })
        });
        assert_eq!((allocations, closes), (0, 0));
        drop(asynchronous);
        drop(synchronous);
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_device_and_pipe_metadata_are_not_regular_files() {
        unsafe extern "system" {
            fn CreatePipe(
                read: *mut super::NativeHandle,
                write: *mut super::NativeHandle,
                attributes: *mut core::ffi::c_void,
                size: u32,
            ) -> i32;
            fn SetLastError(code: u32);
        }
        let device = std::fs::File::open("NUL").unwrap();
        // A previous unrelated failure must not classify a valid device as an
        // invalid endpoint. Non-disk handles never reach FileStandardInfo.
        unsafe { SetLastError(5) };
        assert_eq!(super::is_regular_file(borrowed(&device)), Ok(false));
        let mut reader = core::ptr::null_mut();
        let mut writer = core::ptr::null_mut();
        assert_ne!(
            unsafe { CreatePipe(&mut reader, &mut writer, core::ptr::null_mut(), 0) },
            0
        );
        let reader = unsafe { super::OwnedHandle::from_raw(reader) };
        let writer = unsafe { super::OwnedHandle::from_raw(writer) };
        assert_eq!(super::is_regular_file(reader.get()), Ok(false));
        assert_eq!(super::is_regular_file(writer.get()), Ok(false));
    }

    #[cfg(not(any(unix, windows)))]
    #[test]
    fn unsupported_metadata_does_not_invent_host_failure_or_progress() {
        assert_eq!(
            super::is_regular_file(0),
            Err(crate::platform_io::Failure::unsupported())
        );
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn close_attempts_are_exactly_once_on_success_failure_drop_and_unwind() {
        use std::fs::{self, OpenOptions};
        #[cfg(unix)]
        use std::os::fd::IntoRawFd;
        #[cfg(windows)]
        use std::os::windows::io::IntoRawHandle;
        use std::panic::{catch_unwind, AssertUnwindSafe};

        let path = std::env::temp_dir().join(format!("doria-close-probe-{}", std::process::id()));
        for operation in 0..4 {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)
                .unwrap();
            #[cfg(unix)]
            let handle = file.into_raw_fd();
            #[cfg(windows)]
            let handle = file.into_raw_handle();
            let owner = unsafe { super::OwnedHandle::from_raw(handle) };
            let (_, count) = super::with_close_probe(|| match operation {
                0 => owner.close().unwrap(),
                1 => {
                    // Close the real handle, then inject the observed failure at
                    // the shared consuming seam. Drop must not attempt it again.
                    let failure = crate::platform_io::Failure::other();
                    assert_eq!(
                        owner.close_with(|handle| {
                            unsafe {
                                super::close(handle)?;
                            }
                            Err(failure)
                        }),
                        Err(failure)
                    );
                }
                2 => drop(owner),
                _ => {
                    assert!(catch_unwind(AssertUnwindSafe(|| {
                        let _owner = owner;
                        panic!("test-only host unwind");
                    }))
                    .is_err());
                }
            });
            assert_eq!(count, 1);
        }
        fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn owned_handle_closes_explicitly_and_at_scope_exit() {
        use std::io::Read;
        use std::os::fd::IntoRawFd;
        use std::os::unix::net::UnixStream;
        use std::time::Duration;

        for explicit in [false, true] {
            let (mut reader, writer) = UnixStream::pair().unwrap();
            reader
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let writer = unsafe { super::OwnedHandle::from_raw(writer.into_raw_fd()) };
            if explicit {
                writer.close().unwrap();
            } else {
                drop(writer);
            }
            assert_eq!(reader.read(&mut [0; 1]).unwrap(), 0);
        }
    }

    #[cfg(windows)]
    #[test]
    fn owned_handle_closes_explicitly_and_at_scope_exit() {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

        use super::NativeHandle;
        unsafe extern "system" {
            fn CreatePipe(
                read: *mut NativeHandle,
                write: *mut NativeHandle,
                attributes: *mut core::ffi::c_void,
                size: u32,
            ) -> i32;
            fn PeekNamedPipe(
                pipe: NativeHandle,
                buffer: *mut core::ffi::c_void,
                length: u32,
                read: *mut u32,
                available: *mut u32,
                remaining: *mut u32,
            ) -> i32;
            fn GetLastError() -> u32;
        }

        for explicit in [false, true] {
            unsafe {
                let mut reader = core::ptr::null_mut();
                let mut writer = core::ptr::null_mut();
                assert_ne!(
                    CreatePipe(&mut reader, &mut writer, core::ptr::null_mut(), 0),
                    0
                );
                let reader = OwnedHandle::from_raw_handle(reader);
                let writer = super::OwnedHandle::from_raw(writer);
                if explicit {
                    writer.close().unwrap();
                } else {
                    drop(writer);
                }
                // A nonblocking probe makes a missing close fail, not hang.
                assert_eq!(
                    PeekNamedPipe(
                        reader.as_raw_handle(),
                        core::ptr::null_mut(),
                        0,
                        core::ptr::null_mut(),
                        core::ptr::null_mut(),
                        core::ptr::null_mut(),
                    ),
                    0
                );
                assert_eq!(GetLastError(), 109);
            }
        }
    }
}
