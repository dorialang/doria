pub(crate) use crate::native_handle::NativeHandle;
use crate::platform_io::Failure;

// Explicit completion-capable pipe owners use this path. Ordinary synchronous
// files and borrowed standard devices retain the platform transfers below.
#[cfg(any(windows, test))]
mod windows_completion;
#[cfg(windows)]
pub(crate) use windows_completion::{
    NativeAdapterGroup, NativeAdapterSet, NativePendingGroup, NativePendingSet, NativePendingSetup,
    NativeTransferGroup, NativeTransferSet, PipeAttachment,
};

/// What the parent's end of a child pipe holds beyond its handle.
///
/// The runtime creates this end itself, so it may choose the end's mode:
/// nonblocking on Unix, overlapped on Windows. That is what lets the pipe join
/// a transfer group. The program still sees an ordinary blocking pipe, so the
/// wait a plain `read` or `write` needs lives here, once. Pending transfers do
/// not come through here; they use the transfer group's own wait.
pub(crate) struct PipeEnd {
    /// Signals a plain transfer's completion. Made on first use.
    #[cfg(windows)]
    event: Option<crate::win32::KernelObject>,
    /// This end's completion-port association, made when it first joins a set.
    #[cfg(windows)]
    attachment: Option<windows_completion::PipeAttachment>,
}

impl PipeEnd {
    pub(crate) const fn new() -> Self {
        Self {
            #[cfg(windows)]
            event: None,
            #[cfg(windows)]
            attachment: None,
        }
    }

    /// Waits for data or the end of the stream; never reports would-block.
    ///
    /// # Safety
    /// `handle` is this end's handle. `destination` is writable for `capacity`
    /// bytes, or may be null when `capacity` is zero.
    pub(crate) unsafe fn read(
        &mut self,
        handle: NativeHandle,
        destination: *mut u8,
        capacity: usize,
    ) -> Result<ReadProgress, Failure> {
        if capacity == 0 {
            return Ok(ReadProgress::Data(0));
        }
        platform::read_pipe(self, handle, destination, capacity)
    }

    /// Waits until the child has taken the bytes, as a blocking pipe does.
    /// A failure after some bytes were accepted reports those bytes; the
    /// failure itself returns on the next write.
    ///
    /// # Safety
    /// `handle` is this end's handle. `source` is readable for `length` bytes,
    /// or may be null when `length` is zero.
    pub(crate) unsafe fn write(
        &mut self,
        handle: NativeHandle,
        source: *const u8,
        length: usize,
    ) -> Result<WriteProgress, Failure> {
        if length == 0 {
            return Ok(WriteProgress::Accepted(0));
        }
        platform::write_pipe(self, handle, source, length)
    }

    /// This end's completion-port association, made on first use.
    ///
    /// # Safety
    /// `handle` is this end's handle and has no transfer in progress.
    #[cfg(windows)]
    pub(crate) unsafe fn attachment(
        &mut self,
        handle: NativeHandle,
    ) -> Result<&windows_completion::PipeAttachment, Failure> {
        if self.attachment.is_none() {
            self.attachment = Some(windows_completion::PipeAttachment::attach(handle)?);
        }
        Ok(self.attachment.as_ref().unwrap())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadProgress {
    Data(usize),
    EndOfStream,
    WouldBlock(Failure),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WriteProgress {
    Accepted(usize),
    WouldBlock(Failure),
}

/// Performs a single byte transfer, retrying interruption but not would-block.
/// The destination is caller-owned; empty requests are data, never EOF.
///
/// # Safety
/// The handle must identify a synchronous byte device, and `destination` must
/// be writable for `capacity` bytes (or may be null when capacity is zero).
pub(crate) unsafe fn read(
    handle: NativeHandle,
    destination: *mut u8,
    capacity: usize,
) -> Result<ReadProgress, Failure> {
    if capacity == 0 {
        return Ok(ReadProgress::Data(0));
    }
    platform::read(handle, destination, capacity)
}

/// Performs a single byte transfer without copying an unaccepted suffix.
///
/// # Safety
/// The handle must identify a synchronous byte device, and `source` must be
/// readable for `length` bytes (or may be null when length is zero).
pub(crate) unsafe fn write(
    handle: NativeHandle,
    source: *const u8,
    length: usize,
) -> Result<WriteProgress, Failure> {
    if length == 0 {
        return Ok(WriteProgress::Accepted(0));
    }
    platform::write(handle, source, length)
}

/// A controlled Unix transfer returns after one syscall so the group can
/// recheck its original budget/cancellation between interrupted attempts.
/// # Safety
/// The synchronous handle and caller-owned region obey read/write's contracts.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) unsafe fn read_attempt(
    handle: NativeHandle,
    destination: *mut u8,
    capacity: usize,
) -> Result<ReadProgress, Failure> {
    if capacity == 0 {
        return Ok(ReadProgress::Data(0));
    }
    platform::read_attempt(handle, destination, capacity)
}

/// # Safety
/// The synchronous handle and caller-owned region obey write's contract.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) unsafe fn write_attempt(
    handle: NativeHandle,
    source: *const u8,
    length: usize,
) -> Result<WriteProgress, Failure> {
    if length == 0 {
        return Ok(WriteProgress::Accepted(0));
    }
    platform::write_attempt(handle, source, length)
}

/// Appends one chunk without a seek/write race or an unwritten-suffix copy.
/// # Safety
/// The handle must be a synchronous file, opened with O_APPEND on Unix, and
/// source must be readable for length bytes (null is allowed for length zero).
pub(crate) unsafe fn append(
    handle: NativeHandle,
    source: *const u8,
    length: usize,
) -> Result<WriteProgress, Failure> {
    if length == 0 {
        return Ok(WriteProgress::Accepted(0));
    }
    #[cfg(windows)]
    {
        platform::append(handle, source, length)
    }
    #[cfg(not(windows))]
    {
        platform::write(handle, source, length)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum WriteAllError<E> {
    Transfer(Failure),
    Wait {
        error: E,
        transferred_byte_count: usize,
    },
}

/// Legacy output stops at would-block rather than implicitly waiting.
pub(crate) fn write_all(
    length: usize,
    transfer: impl FnMut(usize, usize) -> Result<WriteProgress, Failure>,
) -> Result<(), Failure> {
    match write_all_with_wait(length, transfer, Err) {
        Ok(()) => Ok(()),
        Err(WriteAllError::Transfer(failure)) => Err(failure),
        Err(WriteAllError::Wait {
            error,
            transferred_byte_count,
        }) => Err(error.after_transferred(transferred_byte_count)),
    }
}

/// Resumes the same suffix after readiness, including stale notifications.
/// The caller owns its one wait budget and cancellation policy. A wait transfers
/// no bytes, so an interrupted wait retains an exact completed-prefix count;
/// a failed transfer still preserves unknown progress.
pub(crate) fn write_all_with_wait<E>(
    length: usize,
    mut transfer: impl FnMut(usize, usize) -> Result<WriteProgress, Failure>,
    mut wait: impl FnMut(Failure) -> Result<(), E>,
) -> Result<(), WriteAllError<E>> {
    let mut offset = 0;
    while offset < length {
        let remaining = length - offset;
        match transfer(offset, remaining) {
            Ok(WriteProgress::Accepted(count)) if count > 0 && count <= remaining => {
                offset += count;
            }
            Ok(WriteProgress::WouldBlock(failure)) if failure.transferred_byte_count == Some(0) => {
                wait(failure).map_err(|error| WriteAllError::Wait {
                    error,
                    transferred_byte_count: offset,
                })?;
            }
            Ok(WriteProgress::WouldBlock(failure)) | Err(failure) => {
                let failure = if failure
                    .transferred_byte_count
                    .is_some_and(|n| n > remaining)
                {
                    failure.with_transferred_byte_count(None)
                } else {
                    failure.after_transferred(offset)
                };
                return Err(WriteAllError::Transfer(failure));
            }
            Ok(WriteProgress::Accepted(_)) => {
                return Err(WriteAllError::Transfer(
                    Failure::other().with_transferred_byte_count(Some(offset)),
                ));
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
mod platform {
    use core::ffi::c_void;
    use core::sync::atomic::{AtomicBool, Ordering};

    use super::*;

    const EINTR: i64 = 4;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const EAGAIN: i64 = 11;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    const EAGAIN: i64 = 35;

    static BROKEN_PIPE_ERRORS: AtomicBool = AtomicBool::new(false);

    unsafe fn enable_broken_pipe_errors() -> Result<(), Failure> {
        if !BROKEN_PIPE_ERRORS.load(Ordering::Acquire) {
            if signal(13, 1) == usize::MAX {
                return Err(crate::platform_io::last_failure().with_transferred_byte_count(Some(0)));
            }
            // Concurrent first writers may both install the same handler;
            // publish only after installation so neither can write too early.
            BROKEN_PIPE_ERRORS.store(true, Ordering::Release);
        }
        Ok(())
    }

    pub(super) unsafe fn read(
        handle: NativeHandle,
        destination: *mut u8,
        capacity: usize,
    ) -> Result<ReadProgress, Failure> {
        retry_interrupted(|| read_attempt(handle, destination, capacity))
    }

    pub(super) unsafe fn read_attempt(
        handle: NativeHandle,
        destination: *mut u8,
        capacity: usize,
    ) -> Result<ReadProgress, Failure> {
        let count = libc_read(
            handle,
            destination.cast(),
            capacity.min(isize::MAX as usize),
        );
        if count > 0 {
            return Ok(ReadProgress::Data(count as usize));
        }
        if count == 0 {
            return Ok(ReadProgress::EndOfStream);
        }
        let code = crate::platform_io::last_system_code();
        match code {
            EAGAIN => Ok(ReadProgress::WouldBlock(
                Failure::from_system_code(code).with_transferred_byte_count(Some(0)),
            )),
            // POSIX EINTR is returned only before any bytes were transferred.
            EINTR => Err(Failure::from_system_code(code).with_transferred_byte_count(Some(0))),
            _ => Err(Failure::from_system_code(code)),
        }
    }

    pub(super) unsafe fn write(
        handle: NativeHandle,
        source: *const u8,
        length: usize,
    ) -> Result<WriteProgress, Failure> {
        retry_interrupted(|| write_attempt(handle, source, length))
    }

    pub(super) unsafe fn write_attempt(
        handle: NativeHandle,
        source: *const u8,
        length: usize,
    ) -> Result<WriteProgress, Failure> {
        // All callers must observe EPIPE, not be terminated by the host signal.
        enable_broken_pipe_errors()?;
        let count = libc_write(handle, source.cast(), length.min(isize::MAX as usize));
        if count >= 0 {
            return Ok(WriteProgress::Accepted(count as usize));
        }
        let code = crate::platform_io::last_system_code();
        match code {
            EAGAIN => Ok(WriteProgress::WouldBlock(
                Failure::from_system_code(code).with_transferred_byte_count(Some(0)),
            )),
            EINTR => Err(Failure::from_system_code(code).with_transferred_byte_count(Some(0))),
            _ => Err(Failure::from_system_code(code)),
        }
    }

    pub(super) unsafe fn read_pipe(
        _: &mut PipeEnd,
        handle: NativeHandle,
        destination: *mut u8,
        capacity: usize,
    ) -> Result<ReadProgress, Failure> {
        loop {
            match read(handle, destination, capacity)? {
                ReadProgress::WouldBlock(_) => wait_ready(handle, libc::POLLIN)?,
                progress => return Ok(progress),
            }
        }
    }

    pub(super) unsafe fn write_pipe(
        _: &mut PipeEnd,
        handle: NativeHandle,
        source: *const u8,
        length: usize,
    ) -> Result<WriteProgress, Failure> {
        let mut accepted = 0;
        while accepted < length {
            let step = match write(handle, source.add(accepted), length - accepted) {
                Ok(WriteProgress::Accepted(count)) if count > 0 => {
                    accepted += count;
                    continue;
                }
                Ok(_) => wait_ready(handle, libc::POLLOUT),
                Err(failure) => Err(failure),
            };
            if let Err(failure) = step {
                return if accepted == 0 {
                    Err(failure)
                } else {
                    Ok(WriteProgress::Accepted(accepted))
                };
            }
        }
        Ok(WriteProgress::Accepted(accepted))
    }

    /// Blocks until the descriptor may be ready. The caller's next transfer
    /// decides what actually happened, including a closed peer or a bad handle.
    unsafe fn wait_ready(handle: NativeHandle, events: libc::c_short) -> Result<(), Failure> {
        let mut descriptor = libc::pollfd {
            fd: handle,
            events,
            revents: 0,
        };
        loop {
            if libc::poll(&mut descriptor, 1, -1) >= 0 {
                return Ok(());
            }
            let failure = crate::platform_io::last_failure();
            if failure.system_code != Some(EINTR) {
                return Err(failure);
            }
        }
    }

    pub(super) fn retry_interrupted<T>(
        mut attempt: impl FnMut() -> Result<T, Failure>,
    ) -> Result<T, Failure> {
        loop {
            match attempt() {
                Err(failure)
                    if failure.system_code == Some(EINTR)
                        && failure.transferred_byte_count == Some(0) => {}
                result => return result,
            }
        }
    }

    unsafe extern "C" {
        #[link_name = "read"]
        fn libc_read(handle: i32, destination: *mut c_void, capacity: usize) -> isize;
        #[link_name = "write"]
        fn libc_write(handle: i32, source: *const c_void, length: usize) -> isize;
        fn signal(number: i32, handler: usize) -> usize;
    }
}

#[cfg(windows)]
mod platform {
    use core::ptr;

    use super::*;
    use crate::win32::{
        pipe_write_quota_available, wait_for_single_object, CreateEventW, GetFileType,
        GetLastError, GetOverlappedResult, KernelObject, Overlapped, ReadFile, WriteFile, INFINITE,
    };

    pub(super) unsafe fn read(
        handle: NativeHandle,
        destination: *mut u8,
        capacity: usize,
    ) -> Result<ReadProgress, Failure> {
        let mut count = 0;
        let requested = capacity.min(u32::MAX as usize);
        if ReadFile(
            handle,
            destination.cast(),
            requested as u32,
            &mut count,
            ptr::null_mut(),
        ) == 0
        {
            let code = GetLastError();
            return match code {
                109 if count == 0 => Ok(ReadProgress::EndOfStream),
                232 if count == 0 => Ok(ReadProgress::WouldBlock(
                    Failure::from_system_code(code.into()).with_transferred_byte_count(Some(0)),
                )),
                _ => Err(failed_windows_transfer(code, count, requested)),
            };
        }
        // A successful zero-byte pipe read is an empty write, not EOF:
        // https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-readfile
        if count == 0 && GetFileType(handle) != 3 {
            Ok(ReadProgress::EndOfStream)
        } else {
            Ok(ReadProgress::Data(count as usize))
        }
    }

    pub(super) unsafe fn read_pipe(
        pipe: &mut PipeEnd,
        handle: NativeHandle,
        destination: *mut u8,
        capacity: usize,
    ) -> Result<ReadProgress, Failure> {
        let requested = capacity.min(u32::MAX as usize);
        let event = pipe_event(pipe)?;
        let result = overlapped_transfer(handle, event, |request| {
            ReadFile(
                handle,
                destination.cast(),
                requested as u32,
                ptr::null_mut(),
                request,
            )
        });
        match result {
            // A successful empty read is an empty write by the child, not EOF.
            Ok(count) => Ok(ReadProgress::Data(count as usize)),
            Err((109, 0)) => Ok(ReadProgress::EndOfStream),
            Err((code, count)) => Err(failed_windows_transfer(code, count, requested)),
        }
    }

    pub(super) unsafe fn write_pipe(
        pipe: &mut PipeEnd,
        handle: NativeHandle,
        source: *const u8,
        length: usize,
    ) -> Result<WriteProgress, Failure> {
        let requested = length.min(u32::MAX as usize);
        let event = pipe_event(pipe)?;
        let result = overlapped_transfer(handle, event, |request| {
            WriteFile(
                handle,
                source.cast(),
                requested as u32,
                ptr::null_mut(),
                request,
            )
        });
        match result {
            Ok(count) => Ok(WriteProgress::Accepted(count as usize)),
            Err((_, count)) if count > 0 && count as usize <= requested => {
                Ok(WriteProgress::Accepted(count as usize))
            }
            Err((code, count)) => Err(failed_windows_transfer(code, count, requested)),
        }
    }

    unsafe fn pipe_event(pipe: &mut PipeEnd) -> Result<NativeHandle, Failure> {
        if pipe.event.is_none() {
            let event = CreateEventW(ptr::null(), 1, 0, ptr::null());
            if event.is_null() {
                return Err(crate::platform_io::last_failure());
            }
            pipe.event = Some(KernelObject::from_raw(event));
        }
        Ok(pipe.event.as_ref().unwrap().get())
    }

    /// Issues one overlapped transfer and waits for it on `event`. Returns the
    /// byte count, or the system code and any count the failure reports. The
    /// tagged event keeps this transfer's completion off the completion port
    /// the pipe may have joined.
    unsafe fn overlapped_transfer(
        handle: NativeHandle,
        event: NativeHandle,
        issue: impl FnOnce(*mut Overlapped) -> i32,
    ) -> Result<u32, (u32, u32)> {
        let tagged = (event as usize | 1) as NativeHandle;
        let mut request = core::pin::pin!(Overlapped::new(tagged));
        // The request stays pinned here until the transfer has finished.
        let raw: *mut Overlapped = request.as_mut().get_unchecked_mut();
        match windows_completion::issued(issue(raw), || GetLastError()) {
            windows_completion::Issued::Failed(code) => return Err((code, 0)),
            windows_completion::Issued::Completed => {}
            windows_completion::Issued::Pending => {
                // A failed wait on this runtime-owned event is a broken runtime
                // invariant, not permission to release a live kernel request.
                if wait_for_single_object(event, INFINITE, "a child pipe transfer to finish") != 0 {
                    crate::runtime_invariant_panic();
                }
            }
        }
        let mut count = 0;
        if GetOverlappedResult(handle, raw, &mut count, 0) != 0 {
            Ok(count)
        } else {
            Err((GetLastError(), count))
        }
    }

    pub(super) unsafe fn write(
        handle: NativeHandle,
        source: *const u8,
        length: usize,
    ) -> Result<WriteProgress, Failure> {
        write_at(handle, source, length, ptr::null_mut())
    }

    pub(super) unsafe fn append(
        handle: NativeHandle,
        source: *const u8,
        length: usize,
    ) -> Result<WriteProgress, Failure> {
        let mut operation = Overlapped::at(u64::MAX, ptr::null_mut());
        // Win32 explicitly supports this EOF offset on synchronous handles.
        // WriteFile completes and updates the file pointer before returning;
        // neither the stack operation nor caller region escapes this call.
        write_at(handle, source, length, &raw mut operation)
    }

    unsafe fn write_at(
        handle: NativeHandle,
        source: *const u8,
        length: usize,
        operation: *mut Overlapped,
    ) -> Result<WriteProgress, Failure> {
        let mut count = 0;
        let requested = length.min(u32::MAX as usize);
        if WriteFile(
            handle,
            source.cast(),
            requested as u32,
            &mut count,
            operation,
        ) == 0
        {
            return Err(failed_windows_transfer(GetLastError(), count, requested));
        }
        // A nonblocking (PIPE_NOWAIT) pipe writes nothing when the request is
        // larger than the free outbound quota, even into an empty pipe, so a
        // large write would report WouldBlock forever. Retry once with exactly
        // the free quota; zero free quota is real backpressure.
        if count == 0 && requested > 0 && GetFileType(handle) == 3 {
            let available = (pipe_write_quota_available(handle)? as usize).min(requested);
            if available > 0
                && WriteFile(
                    handle,
                    source.cast(),
                    available as u32,
                    &mut count,
                    operation,
                ) == 0
            {
                return Err(failed_windows_transfer(GetLastError(), count, available));
            }
        }
        if count == 0 && GetFileType(handle) == 3 {
            Ok(WriteProgress::WouldBlock(
                Failure::other().with_transferred_byte_count(Some(0)),
            ))
        } else {
            Ok(WriteProgress::Accepted(count as usize))
        }
    }
}

#[cfg(any(windows, test))]
fn failed_windows_transfer(code: u32, count: u32, requested: usize) -> Failure {
    // Synchronous byte APIs report actual positive progress even on failure.
    // Their preinitialized zero alone does not establish that no bytes moved.
    // Pending asynchronous work cannot provide a completed synchronous count.
    let count = (code != 997 && count > 0 && count as usize <= requested).then_some(count as usize);
    Failure::from_system_code(code.into()).with_transferred_byte_count(count)
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use super::*;

    pub(super) unsafe fn read(
        _: NativeHandle,
        _: *mut u8,
        _: usize,
    ) -> Result<ReadProgress, Failure> {
        Err(Failure::unsupported())
    }

    pub(super) unsafe fn write(
        _: NativeHandle,
        _: *const u8,
        _: usize,
    ) -> Result<WriteProgress, Failure> {
        Err(Failure::unsupported())
    }

    pub(super) unsafe fn read_pipe(
        _: &mut PipeEnd,
        _: NativeHandle,
        _: *mut u8,
        _: usize,
    ) -> Result<ReadProgress, Failure> {
        Err(Failure::unsupported())
    }

    pub(super) unsafe fn write_pipe(
        _: &mut PipeEnd,
        _: NativeHandle,
        _: *const u8,
        _: usize,
    ) -> Result<WriteProgress, Failure> {
        Err(Failure::unsupported())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_all_keeps_offsets_and_reports_failure_without_replaying_bytes() {
        let mut calls = std::vec::Vec::new();
        let failure = Failure::other().with_transferred_byte_count(Some(0));
        let result = write_all(9, |offset, remaining| {
            calls.push((offset, remaining));
            if offset == 4 {
                Err(failure)
            } else {
                Ok(WriteProgress::Accepted(4))
            }
        });
        assert_eq!(result, Err(failure.after_transferred(4)));
        assert_eq!(calls, [(0, 9), (4, 5)]);
    }

    #[test]
    #[cfg(unix)]
    fn legacy_interruption_retry_requires_proven_zero_progress() {
        for progress in [None, Some(0), Some(1)] {
            let failure = Failure::from_system_code(4).with_transferred_byte_count(progress);
            let mut calls = 0;
            let result = platform::retry_interrupted(|| {
                calls += 1;
                if calls == 1 {
                    Err(failure)
                } else {
                    Ok(3)
                }
            });
            assert_eq!(calls, if progress == Some(0) { 2 } else { 1 });
            assert_eq!(
                result,
                if progress == Some(0) {
                    Ok(3)
                } else {
                    Err(failure)
                }
            );
        }
    }

    #[test]
    fn write_all_never_spins_on_would_block_or_zero_progress() {
        let failure = Failure::other();
        assert_eq!(
            write_all(1, |_, _| Ok(WriteProgress::WouldBlock(failure))),
            Err(failure)
        );
        assert_eq!(
            write_all(1, |_, _| Ok(WriteProgress::Accepted(0))),
            Err(failure.with_transferred_byte_count(Some(0)))
        );
        assert_eq!(
            write_all(1, |_, _| Ok(WriteProgress::Accepted(2))),
            Err(failure.with_transferred_byte_count(Some(0)))
        );
        assert_eq!(
            write_all(0, |_, _| panic!("empty transfer must not call the host")),
            Ok(())
        );
    }

    #[test]
    fn write_all_combines_known_partial_failures_and_preserves_unknown_progress() {
        for raw in [None, Some(0), Some(2), Some(6)] {
            for would_block in [false, true] {
                let failure = Failure::from_system_code(5).with_transferred_byte_count(raw);
                let mut calls = std::vec::Vec::new();
                let result = write_all(9, |offset, remaining| {
                    calls.push((offset, remaining));
                    if offset == 0 {
                        Ok(WriteProgress::Accepted(4))
                    } else if would_block {
                        Ok(WriteProgress::WouldBlock(failure))
                    } else {
                        Err(failure)
                    }
                })
                .unwrap_err();
                assert_eq!(
                    result.transferred_byte_count,
                    raw.filter(|n| *n <= 5).map(|n| n + 4)
                );
                assert_eq!(result.reason, failure.reason);
                assert_eq!(result.system_code, failure.system_code);
                assert_eq!(calls, [(0, 9), (4, 5)]);
            }
        }
        assert_eq!(
            write_all(9, |offset, _| {
                Ok(WriteProgress::Accepted(if offset == 0 { 4 } else { 0 }))
            })
            .unwrap_err()
            .transferred_byte_count,
            Some(4)
        );
    }

    #[test]
    fn write_all_waits_between_retries_without_replaying_the_completed_prefix() {
        let blocked = Failure::other().with_transferred_byte_count(Some(0));
        let mut steps = [
            WriteProgress::Accepted(3),
            WriteProgress::WouldBlock(blocked),
            WriteProgress::WouldBlock(blocked),
            WriteProgress::Accepted(2),
            WriteProgress::WouldBlock(blocked),
            WriteProgress::Accepted(4),
        ]
        .into_iter();
        let mut calls = std::vec::Vec::new();
        let mut waits = 0;
        assert_eq!(
            write_all_with_wait(
                9,
                |offset, remaining| {
                    calls.push((offset, remaining));
                    Ok(steps.next().unwrap())
                },
                |failure| {
                    assert_eq!(failure, blocked);
                    waits += 1;
                    Ok::<(), ()>(())
                },
            ),
            Ok(())
        );
        assert_eq!(waits, 3);
        assert_eq!(calls, [(0, 9), (3, 6), (3, 6), (3, 6), (5, 4), (5, 4)]);
        assert_eq!(steps.next(), None);
    }

    #[test]
    fn an_interrupted_write_all_wait_preserves_its_typed_cause_and_exact_progress() {
        for cause in ["deadline", "cancellation", "closure"] {
            let mut calls = 0;
            assert_eq!(
                write_all_with_wait(
                    9,
                    |offset, _| {
                        calls += 1;
                        Ok(if offset == 0 {
                            WriteProgress::Accepted(4)
                        } else {
                            WriteProgress::WouldBlock(
                                Failure::other().with_transferred_byte_count(Some(0)),
                            )
                        })
                    },
                    |_| Err(cause),
                ),
                Err(WriteAllError::Wait {
                    error: cause,
                    transferred_byte_count: 4,
                })
            );
            assert_eq!(calls, 2);
        }
    }

    #[test]
    fn write_all_does_not_retry_when_would_block_has_unproven_or_positive_progress() {
        for raw in [None, Some(2), Some(6)] {
            let failure = Failure::from_system_code(5).with_transferred_byte_count(raw);
            let result = write_all_with_wait(
                9,
                |offset, _| {
                    Ok(if offset == 0 {
                        WriteProgress::Accepted(4)
                    } else {
                        WriteProgress::WouldBlock(failure)
                    })
                },
                |_| -> Result<(), ()> { panic!("unproven progress must not be retried") },
            );
            assert_eq!(
                result,
                Err(WriteAllError::Transfer(
                    failure.with_transferred_byte_count(raw.filter(|n| *n <= 5).map(|n| n + 4))
                ))
            );
        }
    }

    #[test]
    fn failed_windows_transfer_keeps_only_completed_bounded_counts() {
        for (code, count, requested, expected) in [
            (5, 0, 8, None),
            (234, 3, 8, Some(3)),
            (5, 9, 8, None),
            (997, 3, 8, None),
        ] {
            let failure = failed_windows_transfer(code, count, requested);
            assert_eq!(failure.system_code, Some(code.into()));
            assert_eq!(failure.transferred_byte_count, expected);
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_partial_transfers_distinguish_empty_would_block_data_and_eof() {
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixStream;

        let (reader, writer) = UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        let mut buffer = [0; 8];
        unsafe {
            assert_eq!(
                read(reader.as_raw_fd(), buffer.as_mut_ptr(), 0),
                Ok(ReadProgress::Data(0))
            );
            assert!(matches!(
                read(reader.as_raw_fd(), buffer.as_mut_ptr(), buffer.len()),
                Ok(ReadProgress::WouldBlock(_))
            ));
            assert_eq!(
                write(writer.as_raw_fd(), b"abc".as_ptr(), 3),
                Ok(WriteProgress::Accepted(3))
            );
            assert_eq!(
                read(reader.as_raw_fd(), buffer.as_mut_ptr(), buffer.len()),
                Ok(ReadProgress::Data(3))
            );
            assert_eq!(&buffer[..3], b"abc");
            drop(writer);
            assert_eq!(
                read(reader.as_raw_fd(), buffer.as_mut_ptr(), buffer.len()),
                Ok(ReadProgress::EndOfStream)
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn nonblocking_write_exposes_backpressure_without_allocating_or_spinning() {
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixStream;

        let (reader, writer) = UnixStream::pair().unwrap();
        reader.set_nonblocking(true).unwrap();
        writer.set_nonblocking(true).unwrap();
        let bytes = core::array::from_fn::<_, 16_384, _>(|index| (index % 251) as u8);
        let mut accepted = 0;
        let mut blocked = false;
        for _ in 0..4096 {
            let pending = &bytes[accepted % bytes.len()..];
            match unsafe { write(writer.as_raw_fd(), pending.as_ptr(), pending.len()) }.unwrap() {
                WriteProgress::Accepted(count) => {
                    assert!((1..=pending.len()).contains(&count));
                    accepted += count;
                }
                WriteProgress::WouldBlock(_) => {
                    blocked = true;
                    break;
                }
            }
        }
        assert!(
            blocked,
            "undrained local socket did not produce bounded backpressure"
        );
        assert!(accepted > 0);
        let mut buffer = [0; 4096];
        let mut drained = 0;
        while drained < accepted {
            let requested = (accepted - drained).min(buffer.len());
            let ReadProgress::Data(count) =
                unsafe { read(reader.as_raw_fd(), buffer.as_mut_ptr(), requested) }.unwrap()
            else {
                panic!("accepted bytes must be available to the reader");
            };
            assert!((1..=requested).contains(&count));
            for (index, byte) in buffer[..count].iter().enumerate() {
                assert_eq!(*byte, bytes[(drained + index) % bytes.len()]);
            }
            drained += count;
        }
        assert!(matches!(
            unsafe { read(reader.as_raw_fd(), buffer.as_mut_ptr(), buffer.len()) },
            Ok(ReadProgress::WouldBlock(_))
        ));
        assert_eq!(
            unsafe { write(writer.as_raw_fd(), b"end".as_ptr(), 3) },
            Ok(WriteProgress::Accepted(3))
        );
        assert_eq!(
            unsafe { read(reader.as_raw_fd(), buffer.as_mut_ptr(), buffer.len()) },
            Ok(ReadProgress::Data(3))
        );
        assert_eq!(&buffer[..3], b"end");
    }

    #[cfg(windows)]
    #[test]
    fn windows_pipe_distinguishes_empty_requests_data_and_closed_writer_eof() {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

        use crate::win32::CreatePipe;

        unsafe {
            let mut read_handle = core::ptr::null_mut();
            let mut write_handle = core::ptr::null_mut();
            assert_ne!(
                CreatePipe(&mut read_handle, &mut write_handle, core::ptr::null(), 0),
                0
            );
            let reader = OwnedHandle::from_raw_handle(read_handle);
            let writer = OwnedHandle::from_raw_handle(write_handle);
            let mut buffer = [0; 8];
            assert_eq!(
                read(reader.as_raw_handle(), buffer.as_mut_ptr(), 0),
                Ok(ReadProgress::Data(0))
            );
            assert_eq!(
                write(writer.as_raw_handle(), b"abc".as_ptr(), 3),
                Ok(WriteProgress::Accepted(3))
            );
            assert_eq!(
                read(reader.as_raw_handle(), buffer.as_mut_ptr(), buffer.len()),
                Ok(ReadProgress::Data(3))
            );
            assert_eq!(&buffer[..3], b"abc");
            drop(writer);
            assert_eq!(
                read(reader.as_raw_handle(), buffer.as_mut_ptr(), buffer.len()),
                Ok(ReadProgress::EndOfStream)
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_nonblocking_pipe_exposes_backpressure_and_resumes_without_replay() {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

        use crate::win32::{ConnectNamedPipe, CreateNamedPipeW, SetNamedPipeHandleState};

        let name = format!(
            r"\\.\pipe\doria-byte-io-backpressure-{}",
            std::process::id()
        );
        let wide: std::vec::Vec<_> = name.encode_utf16().chain(Some(0)).collect();
        unsafe {
            // Exercise an inherited synchronous nonblocking byte pipe, not an
            // overlapped operation: PIPE_ACCESS_DUPLEX | FIRST_PIPE_INSTANCE,
            // PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT.
            let handle = CreateNamedPipeW(
                wide.as_ptr(),
                3 | 0x0008_0000,
                1,
                1,
                4096,
                4096,
                0,
                core::ptr::null_mut(),
            );
            assert_ne!(
                handle,
                -1_isize as NativeHandle,
                "{}",
                std::io::Error::last_os_error()
            );
            let server = OwnedHandle::from_raw_handle(handle);
            let client = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&name)
                .unwrap();
            assert_eq!(ConnectNamedPipe(handle, core::ptr::null_mut()), 0);
            assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(535));
            assert_ne!(
                SetNamedPipeHandleState(
                    client.as_raw_handle(),
                    &1,
                    core::ptr::null(),
                    core::ptr::null(),
                ),
                0,
                "{}",
                std::io::Error::last_os_error(),
            );
            let mut buffer = [0; 4096];
            assert!(matches!(
                read(handle, buffer.as_mut_ptr(), buffer.len()),
                Ok(ReadProgress::WouldBlock(_))
            ));
            let input = core::array::from_fn::<_, 16_384, _>(|index| (index % 251) as u8);
            let mut accepted = 0;
            let mut blocked = false;
            for _ in 0..4096 {
                let pending = &input[accepted % input.len()..];
                match write(handle, pending.as_ptr(), pending.len()).unwrap() {
                    WriteProgress::Accepted(count) => {
                        assert!((1..=pending.len()).contains(&count));
                        accepted += count;
                    }
                    WriteProgress::WouldBlock(_) => {
                        blocked = true;
                        break;
                    }
                }
            }
            assert!(
                blocked,
                "an undrained pipe must report bounded backpressure"
            );
            assert!(accepted > 0);
            let mut drained = 0;
            while drained < accepted {
                let requested = (accepted - drained).min(buffer.len());
                let ReadProgress::Data(count) =
                    read(client.as_raw_handle(), buffer.as_mut_ptr(), requested).unwrap()
                else {
                    panic!("accepted bytes must be available to the reader");
                };
                assert!((1..=requested).contains(&count));
                for (index, byte) in buffer[..count].iter().enumerate() {
                    assert_eq!(*byte, input[(drained + index) % input.len()]);
                }
                drained += count;
            }
            assert!(matches!(
                read(client.as_raw_handle(), buffer.as_mut_ptr(), buffer.len()),
                Ok(ReadProgress::WouldBlock(_))
            ));
            assert_eq!(
                write(handle, b"end".as_ptr(), 3),
                Ok(WriteProgress::Accepted(3))
            );
            assert_eq!(
                read(client.as_raw_handle(), buffer.as_mut_ptr(), 3),
                Ok(ReadProgress::Data(3))
            );
            assert_eq!(&buffer[..3], b"end");
            drop(client);
            assert_eq!(
                read(handle, buffer.as_mut_ptr(), 1),
                Ok(ReadProgress::EndOfStream)
            );
            drop(server);
        }
    }
}
