//! The runtime's one declaration of each Win32 import.
//!
//! Every Win32 function the runtime or its tests call is declared here once,
//! with the parameter types the Windows SDK gives it, and every caller imports
//! it from here. All declarations of one foreign symbol bind to one function,
//! so two declarations that disagree describe the same call two ways, and
//! rustc rejects that (`clashing_extern_declarations`). Before this module, 24
//! files carried their own blocks and `CreatePipe` had drifted: five copies
//! took `*mut c_void` attributes, one took `*const SecurityAttributes`. The
//! structures these signatures name live here for the same reason. Three
//! modules had each defined their own `OVERLAPPED`.
//!
//! Conventions: handles are [`NativeHandle`]; `_In_` structures are `*const`;
//! buffers and out-parameters are `*mut`; optional parameters take null. Each
//! binding carries the `cfg` of the code that calls it, so `dead_code` still
//! reports a binding nothing uses.

use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::marker::{PhantomData, PhantomPinned};

#[cfg(windows)]
use crate::native_handle::NativeHandle;
#[cfg(windows)]
use crate::platform_io::{last_failure, Failure};

/// `OVERLAPPED`. While a request is pending the kernel writes `internal` (its
/// NTSTATUS) and `internal_high` (the byte count), so both are `UnsafeCell`
/// and Rust reads them only with volatile reads through a raw pointer. A
/// pending request must not move, so the type is `!Unpin`. The marker is
/// `PhantomData<PhantomPinned>` because `PhantomPinned` itself is not an FFI
/// type and the request crosses the extern signatures below by pointer.
#[repr(C)]
pub(crate) struct Overlapped {
    pub(crate) internal: UnsafeCell<usize>,
    pub(crate) internal_high: UnsafeCell<usize>,
    pub(crate) offset: u32,
    pub(crate) offset_high: u32,
    pub(crate) event: *mut c_void,
    pinned: PhantomData<PhantomPinned>,
}

impl Overlapped {
    /// A request at file offset zero that signals `event`, or no event when
    /// null. A tagged event (low bit set) keeps the completion off any I/O
    /// completion port associated with the handle.
    pub(crate) fn new(event: *mut c_void) -> Self {
        Self::at(0, event)
    }

    /// A request at file `offset`. On a synchronous handle `u64::MAX` writes
    /// at the end of the file.
    pub(crate) fn at(offset: u64, event: *mut c_void) -> Self {
        Self {
            internal: UnsafeCell::new(0),
            internal_high: UnsafeCell::new(0),
            offset: offset as u32,
            offset_high: (offset >> 32) as u32,
            event,
            pinned: PhantomData,
        }
    }
}

/// `SECURITY_ATTRIBUTES`.
#[cfg(windows)]
#[repr(C)]
pub(crate) struct SecurityAttributes {
    pub(crate) length: u32,
    pub(crate) descriptor: *mut c_void,
    pub(crate) inherit: i32,
}

/// `SRWLOCK`: one opaque pointer, zero when unlocked. Only the SRW API reads
/// or writes it.
#[cfg(windows)]
#[repr(C)]
pub(crate) struct SrwLock {
    pointer: *mut c_void,
}

#[cfg(windows)]
impl SrwLock {
    /// `SRWLOCK_INIT`.
    pub(crate) const fn new() -> Self {
        Self {
            pointer: core::ptr::null_mut(),
        }
    }

    /// Whether the lock still holds `SRWLOCK_INIT`.
    ///
    /// # Safety
    /// `lock` points to a live lock that no thread holds or waits on.
    #[cfg(test)]
    pub(crate) unsafe fn is_initial(lock: *const Self) -> bool {
        (*lock).pointer.is_null()
    }
}

/// `CONDITION_VARIABLE`: one opaque pointer, zero when it has no waiters.
/// Only the condition-variable API reads or writes it.
#[cfg(windows)]
#[repr(C)]
pub(crate) struct ConditionVariable {
    pointer: *mut c_void,
}

#[cfg(windows)]
impl ConditionVariable {
    /// `CONDITION_VARIABLE_INIT`.
    pub(crate) const fn new() -> Self {
        Self {
            pointer: core::ptr::null_mut(),
        }
    }

    /// Whether the condition variable still holds `CONDITION_VARIABLE_INIT`.
    ///
    /// # Safety
    /// `condition` points to a live condition variable with no waiters.
    #[cfg(test)]
    pub(crate) unsafe fn is_initial(condition: *const Self) -> bool {
        (*condition).pointer.is_null()
    }
}

/// `STARTUPINFOW`.
#[cfg(windows)]
#[repr(C)]
pub(crate) struct StartupInfo {
    pub(crate) cb: u32,
    pub(crate) reserved: *mut u16,
    pub(crate) desktop: *mut u16,
    pub(crate) title: *mut u16,
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) x_size: u32,
    pub(crate) y_size: u32,
    pub(crate) x_count_chars: u32,
    pub(crate) y_count_chars: u32,
    pub(crate) fill_attribute: u32,
    pub(crate) flags: u32,
    pub(crate) show_window: u16,
    pub(crate) reserved2_count: u16,
    pub(crate) reserved2: *mut u8,
    pub(crate) input: NativeHandle,
    pub(crate) output: NativeHandle,
    pub(crate) error: NativeHandle,
}

/// `STARTUPINFOEXW`. `CreateProcessW` takes it through its `StartupInfo`
/// prefix when `EXTENDED_STARTUPINFO_PRESENT` is set.
#[cfg(windows)]
#[repr(C)]
pub(crate) struct StartupInfoEx {
    pub(crate) startup: StartupInfo,
    pub(crate) attributes: *mut c_void,
}

/// `PROCESS_INFORMATION`.
#[cfg(windows)]
#[repr(C)]
pub(crate) struct ProcessInformation {
    pub(crate) process: NativeHandle,
    pub(crate) thread: NativeHandle,
    pub(crate) process_id: u32,
    pub(crate) thread_id: u32,
}

/// A thread entry point for `CreateThread`.
#[cfg(windows)]
pub(crate) type ThreadStart = extern "system" fn(parameter: *mut c_void) -> u32;

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    // Errors.
    pub(crate) fn GetLastError() -> u32;
    pub(crate) fn GetCurrentProcessId() -> u32;
    pub(crate) fn SetLastError(code: u32);

    // Handles.
    pub(crate) fn CloseHandle(handle: NativeHandle) -> i32;
    #[cfg(test)]
    pub(crate) fn GetHandleInformation(handle: NativeHandle, flags: *mut u32) -> i32;
    pub(crate) fn GetFileType(handle: NativeHandle) -> u32;
    pub(crate) fn GetModuleHandleW(name: *const u16) -> NativeHandle;
    pub(crate) fn GetProcAddress(module: NativeHandle, name: *const u8) -> *mut c_void;

    // Standard devices and consoles.
    pub(crate) fn GetStdHandle(which: u32) -> NativeHandle;
    #[cfg(test)]
    pub(crate) fn SetStdHandle(which: u32, handle: NativeHandle) -> i32;
    pub(crate) fn GetConsoleMode(handle: NativeHandle, mode: *mut u32) -> i32;
    pub(crate) fn ReadConsoleW(
        handle: NativeHandle,
        buffer: *mut c_void,
        characters: u32,
        read: *mut u32,
        input_control: *mut c_void,
    ) -> i32;
    pub(crate) fn WriteConsoleW(
        handle: NativeHandle,
        buffer: *const c_void,
        characters: u32,
        written: *mut u32,
        reserved: *mut c_void,
    ) -> i32;
    pub(crate) fn SetConsoleMode(handle: NativeHandle, mode: u32) -> i32;
    pub(crate) fn SetConsoleCtrlHandler(
        handler: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
    pub(crate) fn ReadConsoleInputW(
        handle: NativeHandle,
        records: *mut InputRecord,
        capacity: u32,
        read: *mut u32,
    ) -> i32;
    pub(crate) fn GetConsoleScreenBufferInfo(
        handle: NativeHandle,
        info: *mut ConsoleScreenBufferInfo,
    ) -> i32;

    // Byte transfer.
    pub(crate) fn ReadFile(
        handle: NativeHandle,
        buffer: *mut c_void,
        length: u32,
        read: *mut u32,
        overlapped: *mut Overlapped,
    ) -> i32;
    pub(crate) fn WriteFile(
        handle: NativeHandle,
        buffer: *const c_void,
        length: u32,
        written: *mut u32,
        overlapped: *mut Overlapped,
    ) -> i32;
    pub(crate) fn GetOverlappedResult(
        handle: NativeHandle,
        overlapped: *mut Overlapped,
        transferred: *mut u32,
        wait: i32,
    ) -> i32;
    pub(crate) fn CancelIoEx(handle: NativeHandle, overlapped: *mut Overlapped) -> i32;
    pub(crate) fn CancelSynchronousIo(thread: NativeHandle) -> i32;

    // Pipes.
    #[cfg(test)]
    pub(crate) fn CreatePipe(
        read: *mut NativeHandle,
        write: *mut NativeHandle,
        attributes: *const SecurityAttributes,
        size: u32,
    ) -> i32;
    pub(crate) fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        instances: u32,
        output_size: u32,
        input_size: u32,
        timeout: u32,
        attributes: *const SecurityAttributes,
    ) -> NativeHandle;
    #[cfg(test)]
    pub(crate) fn ConnectNamedPipe(pipe: NativeHandle, overlapped: *mut Overlapped) -> i32;
    #[cfg(test)]
    pub(crate) fn SetNamedPipeHandleState(
        pipe: NativeHandle,
        mode: *const u32,
        collection_count: *const u32,
        collection_timeout: *const u32,
    ) -> i32;
    #[cfg(test)]
    pub(crate) fn GetNamedPipeInfo(
        pipe: NativeHandle,
        flags: *mut u32,
        output_size: *mut u32,
        input_size: *mut u32,
        instances: *mut u32,
    ) -> i32;
    #[cfg(test)]
    pub(crate) fn PeekNamedPipe(
        pipe: NativeHandle,
        buffer: *mut c_void,
        length: u32,
        read: *mut u32,
        available: *mut u32,
        remaining: *mut u32,
    ) -> i32;

    // Events, waits and completion ports.
    pub(crate) fn CreateEventW(
        attributes: *const SecurityAttributes,
        manual_reset: i32,
        initial_state: i32,
        name: *const u16,
    ) -> NativeHandle;
    pub(crate) fn SetEvent(event: NativeHandle) -> i32;
    // The four waits stay private: callers use the wrappers below, which own
    // the test contract for waits that would otherwise never end.
    fn WaitForSingleObject(handle: NativeHandle, milliseconds: u32) -> u32;
    fn WaitForMultipleObjects(
        count: u32,
        handles: *const NativeHandle,
        wait_all: i32,
        milliseconds: u32,
    ) -> u32;
    pub(crate) fn CreateIoCompletionPort(
        handle: NativeHandle,
        port: NativeHandle,
        key: usize,
        concurrency: u32,
    ) -> NativeHandle;
    fn GetQueuedCompletionStatus(
        port: NativeHandle,
        transferred: *mut u32,
        key: *mut usize,
        overlapped: *mut *mut Overlapped,
        milliseconds: u32,
    ) -> i32;
    pub(crate) fn PostQueuedCompletionStatus(
        port: NativeHandle,
        transferred: u32,
        key: usize,
        overlapped: *mut Overlapped,
    ) -> i32;

    // Slim reader/writer locks and condition variables.
    pub(crate) fn AcquireSRWLockExclusive(lock: *mut SrwLock);
    pub(crate) fn ReleaseSRWLockExclusive(lock: *mut SrwLock);
    fn SleepConditionVariableSRW(
        condition: *mut ConditionVariable,
        lock: *mut SrwLock,
        milliseconds: u32,
        flags: u32,
    ) -> i32;
    pub(crate) fn WakeAllConditionVariable(condition: *mut ConditionVariable);

    // Files.
    pub(crate) fn CreateFileW(
        name: *const u16,
        access: u32,
        share: u32,
        attributes: *const SecurityAttributes,
        creation: u32,
        flags: u32,
        template: NativeHandle,
    ) -> NativeHandle;
    pub(crate) fn DeleteFileW(name: *const u16) -> i32;
    pub(crate) fn GetFileAttributesW(name: *const u16) -> u32;
    pub(crate) fn GetFileSizeEx(handle: NativeHandle, size: *mut i64) -> i32;
    pub(crate) fn GetFileInformationByHandleEx(
        handle: NativeHandle,
        class: i32,
        information: *mut c_void,
        size: u32,
    ) -> i32;
    pub(crate) fn SetFileInformationByHandle(
        handle: NativeHandle,
        class: u32,
        information: *mut c_void,
        size: u32,
    ) -> i32;
    pub(crate) fn SetFilePointerEx(
        handle: NativeHandle,
        distance: i64,
        position: *mut i64,
        method: u32,
    ) -> i32;
    pub(crate) fn FlushFileBuffers(handle: NativeHandle) -> i32;
    pub(crate) fn LockFileEx(
        handle: NativeHandle,
        flags: u32,
        reserved: u32,
        length_low: u32,
        length_high: u32,
        overlapped: *mut Overlapped,
    ) -> i32;
    pub(crate) fn UnlockFileEx(
        handle: NativeHandle,
        reserved: u32,
        length_low: u32,
        length_high: u32,
        overlapped: *mut Overlapped,
    ) -> i32;

    // Threads and processes.
    pub(crate) fn CreateThread(
        attributes: *const SecurityAttributes,
        stack_size: usize,
        start: ThreadStart,
        parameter: *mut c_void,
        flags: u32,
        thread_id: *mut u32,
    ) -> NativeHandle;
    pub(crate) fn CreateProcessW(
        application: *const u16,
        command_line: *mut u16,
        process_attributes: *const SecurityAttributes,
        thread_attributes: *const SecurityAttributes,
        inherit_handles: i32,
        flags: u32,
        environment: *const c_void,
        directory: *const u16,
        startup: *const StartupInfo,
        information: *mut ProcessInformation,
    ) -> i32;
    pub(crate) fn InitializeProcThreadAttributeList(
        list: *mut c_void,
        count: u32,
        flags: u32,
        size: *mut usize,
    ) -> i32;
    pub(crate) fn UpdateProcThreadAttribute(
        list: *mut c_void,
        flags: u32,
        attribute: usize,
        value: *const c_void,
        size: usize,
        previous: *mut c_void,
        returned: *mut usize,
    ) -> i32;
    pub(crate) fn DeleteProcThreadAttributeList(list: *mut c_void);
    pub(crate) fn GetExitCodeProcess(process: NativeHandle, code: *mut u32) -> i32;
    pub(crate) fn TerminateProcess(process: NativeHandle, code: u32) -> i32;
    pub(crate) fn ExitProcess(code: u32) -> !;

    // Environment and command line.
    pub(crate) fn GetCommandLineW() -> *const u16;
    pub(crate) fn GetEnvironmentStringsW() -> *mut u16;
    pub(crate) fn FreeEnvironmentStringsW(block: *mut u16) -> i32;
    pub(crate) fn GetEnvironmentVariableA(name: *const u8, buffer: *mut u8, size: u32) -> u32;
    pub(crate) fn GetEnvironmentVariableW(name: *const u16, buffer: *mut u16, size: u32) -> u32;

    // Memory.
    pub(crate) fn GetProcessHeap() -> NativeHandle;
    pub(crate) fn HeapAlloc(heap: NativeHandle, flags: u32, size: usize) -> *mut c_void;
    pub(crate) fn HeapFree(heap: NativeHandle, flags: u32, memory: *mut c_void) -> i32;
    pub(crate) fn LocalFree(memory: *mut c_void) -> *mut c_void;
}

#[cfg(windows)]
#[link(name = "shell32")]
unsafe extern "system" {
    pub(crate) fn CommandLineToArgvW(command_line: *const u16, count: *mut i32) -> *mut *const u16;
}

#[cfg(windows)]
#[link(name = "bcrypt")]
unsafe extern "system" {
    pub(crate) fn BCryptGenRandom(
        algorithm: *mut c_void,
        buffer: *mut u8,
        length: u32,
        flags: u32,
    ) -> i32;
}

/// `COORD`.
#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct Coord {
    pub(crate) x: i16,
    pub(crate) y: i16,
}

/// `SMALL_RECT`. All four edges are inclusive.
#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct SmallRect {
    pub(crate) left: i16,
    pub(crate) top: i16,
    pub(crate) right: i16,
    pub(crate) bottom: i16,
}

/// `CONSOLE_SCREEN_BUFFER_INFO`. `window` is the visible part of the buffer
/// and `cursor` is a buffer position, not a window position.
#[cfg(windows)]
#[repr(C)]
pub(crate) struct ConsoleScreenBufferInfo {
    pub(crate) size: Coord,
    pub(crate) cursor: Coord,
    pub(crate) attributes: u16,
    pub(crate) window: SmallRect,
    pub(crate) maximum_window_size: Coord,
}

/// `KEY_EVENT_RECORD`, with `uChar` read as its UTF-16 member.
#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct KeyEventRecord {
    pub(crate) key_down: i32,
    pub(crate) repeat_count: u16,
    pub(crate) virtual_key_code: u16,
    pub(crate) virtual_scan_code: u16,
    pub(crate) character: u16,
    pub(crate) control_key_state: u32,
}

/// The `Event` union of `INPUT_RECORD`. `other` gives it the size of its
/// largest member; which member is live is `InputRecord::event_type`.
#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) union InputEvent {
    pub(crate) key: KeyEventRecord,
    pub(crate) window_buffer_size: Coord,
    pub(crate) other: [u32; 4],
}

/// `INPUT_RECORD`.
#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct InputRecord {
    pub(crate) event_type: u16,
    pub(crate) event: InputEvent,
}

/// Win32 `INFINITE`: wait until the object is signaled.
#[cfg(windows)]
pub(crate) const INFINITE: u32 = u32::MAX;

/// `WAIT_TIMEOUT`, as `WaitForSingleObject` and `WaitForMultipleObjects`
/// return it and as `GetQueuedCompletionStatus` reports it.
#[cfg(windows)]
pub(crate) const WAIT_TIMEOUT: u32 = 258;

/// `ERROR_TIMEOUT`, as `SleepConditionVariableSRW` reports it.
#[cfg(windows)]
const ERROR_TIMEOUT: u32 = 1460;

/// The longest any wait may last in a test build.
///
/// The runtime waits forever in exactly the places where releasing early would
/// be unsound: draining a cancelled request whose buffer the kernel still
/// borrows, a group with no deadline, a child process or helper thread. In a
/// test, a wait that never ends is a hang. CI then reports only that the test
/// "has been running for over 60 seconds", with no assertion and no other
/// test's result. So in test builds an `INFINITE` wait is bounded, and running
/// out of it reports the test and what it waited for, then aborts. Unwinding
/// instead would release storage the kernel may still write. Release builds
/// keep `INFINITE`.
#[cfg(all(windows, test))]
pub(crate) const TEST_WAIT_LIMIT_MILLISECONDS: u32 = 120_000;

#[cfg(windows)]
fn native_timeout(milliseconds: u32) -> u32 {
    #[cfg(test)]
    if milliseconds == INFINITE {
        return TEST_WAIT_LIMIT_MILLISECONDS;
    }
    milliseconds
}

/// A bounded wait timed out. If the caller asked to wait forever, the test
/// hung: report it and abort. Otherwise the timeout is the caller's result.
#[cfg(windows)]
fn timed_out(milliseconds: u32, waiting_for: &str) {
    #[cfg(test)]
    if milliseconds == INFINITE {
        crate::test_report::wait_expired(waiting_for, TEST_WAIT_LIMIT_MILLISECONDS);
    }
    let _ = (milliseconds, waiting_for);
}

/// `WaitForSingleObject`. `waiting_for` names the event for a test-build hang
/// report.
///
/// # Safety
/// `handle` is a live waitable handle.
#[cfg(windows)]
pub(crate) unsafe fn wait_for_single_object(
    handle: NativeHandle,
    milliseconds: u32,
    waiting_for: &str,
) -> u32 {
    let result = WaitForSingleObject(handle, native_timeout(milliseconds));
    if result == WAIT_TIMEOUT {
        timed_out(milliseconds, waiting_for);
    }
    result
}

/// `WaitForMultipleObjects`. `waiting_for` names the events for a test-build
/// hang report.
///
/// # Safety
/// `handles` points to `count` live waitable handles.
#[cfg(windows)]
pub(crate) unsafe fn wait_for_multiple_objects(
    count: u32,
    handles: *const NativeHandle,
    wait_all: i32,
    milliseconds: u32,
    waiting_for: &str,
) -> u32 {
    let result = WaitForMultipleObjects(count, handles, wait_all, native_timeout(milliseconds));
    if result == WAIT_TIMEOUT {
        timed_out(milliseconds, waiting_for);
    }
    result
}

/// `GetQueuedCompletionStatus`. `waiting_for` names the packet for a
/// test-build hang report.
///
/// # Safety
/// `port` is a live completion port and the out-parameters are writable.
#[cfg(windows)]
pub(crate) unsafe fn get_queued_completion_status(
    port: NativeHandle,
    transferred: *mut u32,
    key: *mut usize,
    overlapped: *mut *mut Overlapped,
    milliseconds: u32,
    waiting_for: &str,
) -> i32 {
    let result = GetQueuedCompletionStatus(
        port,
        transferred,
        key,
        overlapped,
        native_timeout(milliseconds),
    );
    if result == 0 && (*overlapped).is_null() && GetLastError() == WAIT_TIMEOUT {
        timed_out(milliseconds, waiting_for);
        // The report may have called other APIs; keep the caller's error.
        SetLastError(WAIT_TIMEOUT);
    }
    result
}

/// `SleepConditionVariableSRW` in exclusive mode. `waiting_for` names the
/// predicate for a test-build hang report.
///
/// # Safety
/// The calling thread holds `lock` exclusively and `condition` is live.
#[cfg(windows)]
pub(crate) unsafe fn sleep_condition_variable(
    condition: *mut ConditionVariable,
    lock: *mut SrwLock,
    milliseconds: u32,
    waiting_for: &str,
) -> i32 {
    let result = SleepConditionVariableSRW(condition, lock, native_timeout(milliseconds), 0);
    if result == 0 && GetLastError() == ERROR_TIMEOUT {
        timed_out(milliseconds, waiting_for);
        SetLastError(ERROR_TIMEOUT);
    }
    result
}

/// `IO_STATUS_BLOCK`.
#[cfg(windows)]
#[repr(C)]
struct IoStatusBlock {
    status: usize,
    information: usize,
}

/// `NtQueryInformationFile`, resolved from the already loaded ntdll so the
/// runtime adds no module ownership or link dependency. A failing NTSTATUS is
/// mapped to its Win32 error with `RtlNtStatusToDosError`.
/// https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/ntifs/nf-ntifs-ntqueryinformationfile
///
/// # Safety
/// `handle` is live, and `information` is writable for `length` bytes of the
/// structure `class` names.
#[cfg(windows)]
pub(crate) unsafe fn query_information_file(
    handle: NativeHandle,
    information: *mut c_void,
    length: u32,
    class: u32,
) -> Result<(), Failure> {
    type Query =
        unsafe extern "system" fn(NativeHandle, *mut IoStatusBlock, *mut c_void, u32, u32) -> i32;
    type ErrorCode = unsafe extern "system" fn(i32) -> u32;
    const NTDLL: &[u16] = &[110, 116, 100, 108, 108, 46, 100, 108, 108, 0];
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
    let mut status = IoStatusBlock {
        status: 0,
        information: 0,
    };
    let code = query(handle, &mut status, information, length, class);
    if code != 0 {
        return Err(Failure::from_system_code(error(code).into()));
    }
    Ok(())
}

/// The free space in this pipe end's outbound buffer:
/// `FILE_PIPE_LOCAL_INFORMATION::WriteQuotaAvailable` (`FilePipeLocalInformation`, 24).
///
/// # Safety
/// `handle` is a live pipe handle.
#[cfg(windows)]
pub(crate) unsafe fn pipe_write_quota_available(handle: NativeHandle) -> Result<u32, Failure> {
    // NamedPipeType, NamedPipeConfiguration, MaximumInstances, CurrentInstances,
    // InboundQuota, ReadDataAvailable, OutboundQuota, WriteQuotaAvailable,
    // NamedPipeState, NamedPipeEnd: ten ULONGs.
    let mut information = [0u32; 10];
    query_information_file(handle, information.as_mut_ptr().cast(), 40, 24)?;
    Ok(information[7])
}

/// Owns one private kernel object the runtime created for itself: a helper
/// thread, a completion event or the shared completion port.
///
/// It is deliberately not an `OwnedHandle`, which owns a caller's device or
/// stream. The stream close contract ("this set closes no stream; the sole
/// owner closes it once") is what `with_close_probe` measures, and a private
/// object's lifetime is not part of it. A thread joined at shutdown, or the
/// process-wide port closed when its last claim drops, would otherwise count
/// as a stream close in whichever unrelated owner happened to release it.
#[cfg(windows)]
pub(crate) struct KernelObject(NativeHandle);

#[cfg(all(windows, test))]
std::thread_local! {
    static KERNEL_OBJECT_CLOSES: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// Counts this thread's private kernel-object closes over a scoped interval,
/// apart from device and stream closes. It is the lifecycle evidence for an
/// object such as the shared port, which closes with its last claim.
#[cfg(all(windows, test))]
pub(crate) fn with_kernel_object_close_probe<R>(operation: impl FnOnce() -> R) -> (R, usize) {
    crate::native_handle::measure_closes(&KERNEL_OBJECT_CLOSES, operation)
}

#[cfg(windows)]
impl KernelObject {
    /// # Safety
    /// `handle` is a valid kernel object handle that this owner alone closes.
    pub(crate) unsafe fn from_raw(handle: NativeHandle) -> Self {
        Self(handle)
    }

    pub(crate) fn get(&self) -> NativeHandle {
        self.0
    }

    /// Closes the object now and reports a failure. A close is never retried.
    pub(crate) fn close(self) -> Result<(), Failure> {
        let handle = self.0;
        core::mem::forget(self);
        #[cfg(test)]
        crate::native_handle::record_close_in(&KERNEL_OBJECT_CLOSES);
        if unsafe { CloseHandle(handle) } != 0 {
            Ok(())
        } else {
            Err(last_failure())
        }
    }
}

#[cfg(windows)]
impl Drop for KernelObject {
    fn drop(&mut self) {
        #[cfg(test)]
        crate::native_handle::record_close_in(&KERNEL_OBJECT_CLOSES);
        // As for a device handle, a failed close is not retried.
        let _ = unsafe { CloseHandle(self.0) };
    }
}
