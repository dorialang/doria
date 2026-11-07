#[cfg(windows)]
use core::ffi::c_void;
use core::ptr;

use crate::byte_io::{ReadProgress, WriteProgress};
use crate::platform_io::Failure;

mod checked_stream;
mod device;
#[cfg(any(unix, windows))]
pub(crate) mod helper;
#[cfg(all(test, any(target_os = "linux", target_os = "macos", windows)))]
mod pending_tests;
mod stream;
pub(crate) use device::{standard_device, DeviceLoan};
#[cfg(test)]
pub(crate) use device::{test_lock, TestInput};

/// One owner for bytes already consumed from stdin, including console-converted
/// bytes and line read-ahead. Binary consumers drain this queue before raw I/O.
pub(crate) struct InputBuffer {
    pub(crate) bytes: *mut u8,
    pub(crate) capacity: usize,
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) scanned: usize,
    pub(crate) eof: bool,
}

impl InputBuffer {
    pub(crate) const fn new() -> Self {
        Self {
            bytes: ptr::null_mut(),
            capacity: 0,
            start: 0,
            end: 0,
            scanned: 0,
            eof: false,
        }
    }

    pub(crate) unsafe fn ensure_capacity(&mut self, required: usize) -> Result<(), ()> {
        if self.capacity >= required {
            return Ok(());
        }
        let replacement = crate::allocate(required);
        if replacement.is_null() {
            return Err(());
        }
        if !self.bytes.is_null() {
            ptr::copy_nonoverlapping(
                self.bytes.add(self.start),
                replacement,
                self.end - self.start,
            );
            self.end -= self.start;
            self.scanned -= self.start;
            self.start = 0;
            crate::deallocate(self.bytes);
        }
        self.bytes = replacement;
        self.capacity = required;
        Ok(())
    }

    pub(crate) unsafe fn prepare_write_space(&mut self, minimum: usize) -> Result<(), ()> {
        if self.capacity - self.end >= minimum {
            return Ok(());
        }
        if self.start != 0 {
            let remaining = self.end - self.start;
            ptr::copy(self.bytes.add(self.start), self.bytes, remaining);
            self.scanned -= self.start;
            self.start = 0;
            self.end = remaining;
        }
        let required = self.end.checked_add(minimum).ok_or(())?;
        let mut capacity = self.capacity.max(4096);
        while capacity < required {
            capacity = capacity.checked_mul(2).ok_or(())?;
        }
        self.ensure_capacity(capacity)
    }

    /// A short read of queued bytes never starts another device operation.
    /// Zero capacity leaves both the queue and the producer untouched.
    pub(crate) unsafe fn read_bytes_with(
        &mut self,
        destination: *mut u8,
        capacity: usize,
        mut transfer: impl FnMut(*mut u8, usize) -> Result<usize, crate::platform_io::Failure>,
    ) -> Result<usize, crate::platform_io::Failure> {
        match self.read_progress_with(destination, capacity, |destination, capacity| {
            transfer(destination, capacity).map(|count| {
                if count == 0 {
                    ReadProgress::EndOfStream
                } else {
                    ReadProgress::Data(count)
                }
            })
        })? {
            ReadProgress::Data(count) => Ok(count),
            ReadProgress::EndOfStream => Ok(0),
            ReadProgress::WouldBlock(failure) => Err(failure),
        }
    }

    pub(crate) unsafe fn read_progress_with(
        &mut self,
        destination: *mut u8,
        capacity: usize,
        mut transfer: impl FnMut(*mut u8, usize) -> Result<ReadProgress, Failure>,
    ) -> Result<ReadProgress, Failure> {
        if capacity == 0 {
            return Ok(ReadProgress::Data(0));
        }
        let copied = (self.end - self.start).min(capacity);
        if copied != 0 {
            ptr::copy_nonoverlapping(self.bytes.add(self.start), destination, copied);
            self.start += copied;
            self.scanned = self.scanned.max(self.start);
            return Ok(ReadProgress::Data(copied));
        }
        if self.eof {
            return Ok(ReadProgress::EndOfStream);
        }
        // No conversion or buffering is introduced on the empty-queue path.
        transfer(destination, capacity)
    }

    pub(crate) unsafe fn take_buffered_input(&mut self) -> BufferedInput {
        let length = self.end - self.start;
        let bytes = if length == 0 {
            ptr::null()
        } else {
            self.bytes.add(self.start)
        };
        self.start = self.end;
        self.scanned = self.end;
        BufferedInput {
            bytes,
            length,
            eof: self.eof,
        }
    }

    pub(crate) unsafe fn release(&mut self) {
        if !self.bytes.is_null() {
            crate::deallocate(self.bytes);
        }
        self.bytes = ptr::null_mut();
        self.capacity = 0;
        self.start = 0;
        self.end = 0;
        self.scanned = 0;
    }
}

impl Drop for InputBuffer {
    fn drop(&mut self) {
        unsafe { self.release() }
    }
}

pub(crate) struct BufferedInput {
    pub(crate) bytes: *const u8,
    pub(crate) length: usize,
    pub(crate) eof: bool,
}

/// The caller must serialize stdin access and end this borrow before any other
/// stdin consumer runs. Returned line slices also expire on the next consumer.
pub(crate) unsafe fn stdin_input() -> *mut InputBuffer {
    standard_device(StandardStream::Stdin).input()
}

// ReadConsoleW returns up to 512 UTF-16 units. Its complete UTF-8 conversion
// lands directly in the shared queue, never in a second persistent buffer.
#[cfg(windows)]
pub(crate) const TEXT_READ_MINIMUM: usize = 2048;
#[cfg(not(windows))]
pub(crate) const TEXT_READ_MINIMUM: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum StandardStream {
    Stdin = 0,
    Stdout = 1,
    Stderr = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WriteOutcome {
    Success,
    BrokenPipe,
    OtherFailure(crate::platform_io::Failure),
}

#[cfg(unix)]
const EPIPE: i32 = 32;

#[cfg(unix)]
const fn classify_unix_write_error(error: Option<crate::platform_io::Failure>) -> WriteOutcome {
    match error {
        None => WriteOutcome::Success,
        Some(failure) if matches!(failure.system_code, Some(code) if code == EPIPE as i64) => {
            WriteOutcome::BrokenPipe
        }
        Some(failure) => WriteOutcome::OtherFailure(failure),
    }
}

#[cfg(unix)]
const fn descriptor(stream: StandardStream) -> i32 {
    match stream {
        StandardStream::Stdin => 0,
        StandardStream::Stdout => 1,
        StandardStream::Stderr => 2,
    }
}

/// Reads raw bytes without applying UTF-8 validation or line rules.
///
/// For a nonempty request, `Ok(0)` means EOF. Empty requests do no I/O.
/// `destination` must be writable for `capacity` bytes.
#[cfg(unix)]
pub(crate) unsafe fn read_text(
    stream: StandardStream,
    destination: *mut u8,
    capacity: usize,
) -> Result<usize, crate::platform_io::Failure> {
    if stream != StandardStream::Stdin {
        return Err(crate::platform_io::Failure::invalid_input());
    }
    read_from_handle(descriptor(stream), destination, capacity)
}

/// Writes raw bytes without adding text or line semantics.
#[cfg(unix)]
unsafe fn write_unclaimed(
    stream: StandardStream,
    bytes: *const u8,
    byte_length: usize,
) -> WriteOutcome {
    if stream == StandardStream::Stdin {
        return WriteOutcome::OtherFailure(crate::platform_io::Failure::invalid_input());
    }
    match crate::byte_io::write_all(byte_length, |offset, remaining| {
        crate::byte_io::write(descriptor(stream), bytes.add(offset), remaining)
    }) {
        Ok(()) => WriteOutcome::Success,
        Err(failure) => classify_unix_write_error(Some(failure)),
    }
}

#[cfg(any(unix, windows))]
unsafe fn read_from_handle(
    handle: crate::byte_io::NativeHandle,
    destination: *mut u8,
    capacity: usize,
) -> Result<usize, crate::platform_io::Failure> {
    #[cfg(windows)]
    if capacity != 0 {
        crate::native_handle::require_synchronous_device(handle)?;
    }
    loop {
        match crate::byte_io::read(handle, destination, capacity)? {
            // The legacy count-only contract reserves zero for EOF. A Windows
            // pipe's empty write must not prematurely end a line or read-all.
            crate::byte_io::ReadProgress::Data(0) if capacity != 0 => continue,
            crate::byte_io::ReadProgress::Data(count) => return Ok(count),
            crate::byte_io::ReadProgress::EndOfStream => return Ok(0),
            crate::byte_io::ReadProgress::WouldBlock(failure) => return Err(failure),
        }
    }
}

/// One primitive byte read, sharing line read-ahead and console-converted input.
/// Unlike the legacy count-only path, empty pipe writes are not retried or EOF.
pub(crate) unsafe fn read_progress(
    stream: StandardStream,
    destination: *mut u8,
    capacity: usize,
) -> Result<ReadProgress, Failure> {
    if stream != StandardStream::Stdin {
        return Err(Failure::invalid_input());
    }
    if capacity == 0 {
        return Ok(ReadProgress::Data(0));
    }
    let _loan = standard_device(stream).try_borrow_for_io()?;
    (&mut *stdin_input()).read_progress_with(destination, capacity, |destination, capacity| {
        let handle = byte_handle(stream)?;
        #[cfg(windows)]
        crate::native_handle::require_synchronous_device(handle)?;
        crate::byte_io::read(handle, destination, capacity)
    })
}

/// One raw byte write, never a text conversion or an implicit write-all.
pub(crate) unsafe fn write_progress(
    stream: StandardStream,
    source: *const u8,
    length: usize,
) -> Result<WriteProgress, Failure> {
    if stream == StandardStream::Stdin {
        return Err(Failure::invalid_input());
    }
    if length == 0 {
        return Ok(WriteProgress::Accepted(0));
    }
    let _loan = standard_device(stream).try_borrow_for_io()?;
    let handle = byte_handle(stream)?;
    #[cfg(windows)]
    crate::native_handle::require_synchronous_device(handle)?;
    crate::byte_io::write(handle, source, length)
}

/// Observes the process handle identifier without acquiring ownership or I/O
/// access. Using it still requires the canonical device loan and a live handle.
pub(crate) fn byte_handle(stream: StandardStream) -> Result<crate::byte_io::NativeHandle, Failure> {
    #[cfg(unix)]
    {
        Ok(descriptor(stream))
    }
    #[cfg(windows)]
    {
        valid_handle(stream).ok_or_else(|| Failure::from_system_code(6))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = stream;
        Err(Failure::unsupported())
    }
}

#[cfg(unix)]
unsafe fn write_bytes_unclaimed(
    stream: StandardStream,
    bytes: *const u8,
    byte_length: usize,
) -> WriteOutcome {
    write_unclaimed(stream, bytes, byte_length)
}

#[cfg(unix)]
unsafe fn read_raw_stdin(
    destination: *mut u8,
    capacity: usize,
) -> Result<usize, crate::platform_io::Failure> {
    read_from_handle(descriptor(StandardStream::Stdin), destination, capacity)
}

/// Flushes a standard output device.
///
/// Raw writes are currently unbuffered, so this is intentionally a successful
/// no-op for the output streams. It reports the same `WriteOutcome` vocabulary as
/// [`write`] rather than a bare boolean, so a caller can distinguish a closed pipe
/// — which is a clean status-0 exit under the permanent ordinary-output rule — from
/// a genuine device failure. The caller applies its checked-error or panic policy.
/// Flushing standard input is not a
/// meaningful operation and is reported as a failure.
pub(crate) unsafe fn flush(stream: StandardStream) -> WriteOutcome {
    if stream == StandardStream::Stdin {
        return WriteOutcome::OtherFailure(crate::platform_io::Failure::invalid_input());
    }
    WriteOutcome::Success
}

/// All legacy text/raw output enters through the same canonical device claim.
/// The internal unclaimed operations are used only with that live claim.
pub(crate) unsafe fn write(
    stream: StandardStream,
    bytes: *const u8,
    length: usize,
) -> WriteOutcome {
    if stream == StandardStream::Stdin {
        return WriteOutcome::OtherFailure(Failure::invalid_input());
    }
    if length == 0 {
        return WriteOutcome::Success;
    }
    match standard_device(stream).try_borrow_for_io() {
        Ok(loan) => loan.write_text(bytes, length),
        Err(failure) => WriteOutcome::OtherFailure(failure),
    }
}

pub(crate) unsafe fn write_bytes(
    stream: StandardStream,
    bytes: *const u8,
    length: usize,
) -> WriteOutcome {
    if stream == StandardStream::Stdin {
        return WriteOutcome::OtherFailure(Failure::invalid_input());
    }
    if length == 0 {
        return WriteOutcome::Success;
    }
    match standard_device(stream).try_borrow_for_io() {
        Ok(loan) => loan.write_bytes(bytes, length),
        Err(failure) => WriteOutcome::OtherFailure(failure),
    }
}

/// Fatal reporting may find stderr already unavailable through a retained loan.
/// Never acquire recursively, release that foreign loan or turn a panic into
/// normal cleanup; the original fatal status remains authoritative.
pub(crate) unsafe fn write_panic(bytes: *const u8, length: usize) -> WriteOutcome {
    let Some(loan) = standard_device(StandardStream::Stderr).try_borrow() else {
        return WriteOutcome::OtherFailure(Failure::invalid_input());
    };
    loan.write_text(bytes, length)
}

#[cfg(unix)]
unsafe fn is_interactive_unclaimed(stream: StandardStream) -> bool {
    is_interactive_descriptor(descriptor(stream))
}

#[cfg(unix)]
unsafe fn is_interactive_descriptor(descriptor: i32) -> bool {
    isatty(descriptor) == 1
}

#[cfg(windows)]
const STD_INPUT_HANDLE: u32 = -10_i32 as u32;
#[cfg(windows)]
const STD_OUTPUT_HANDLE: u32 = -11_i32 as u32;
#[cfg(windows)]
const STD_ERROR_HANDLE: u32 = -12_i32 as u32;
#[cfg(windows)]
const INVALID_HANDLE_VALUE: *mut c_void = -1_isize as *mut c_void;
#[cfg(windows)]
const ERROR_BROKEN_PIPE: u32 = 109;
#[cfg(windows)]
const ERROR_NO_DATA: u32 = 232;

#[cfg(windows)]
const fn classify_windows_write_error(error: Option<crate::platform_io::Failure>) -> WriteOutcome {
    match error {
        None => WriteOutcome::Success,
        Some(failure) if matches!(failure.system_code, Some(code) if code == ERROR_BROKEN_PIPE as i64 || code == ERROR_NO_DATA as i64) => {
            WriteOutcome::BrokenPipe
        }
        Some(failure) => WriteOutcome::OtherFailure(failure),
    }
}

#[cfg(windows)]
fn handle(stream: StandardStream) -> *mut c_void {
    let identifier = match stream {
        StandardStream::Stdin => STD_INPUT_HANDLE,
        StandardStream::Stdout => STD_OUTPUT_HANDLE,
        StandardStream::Stderr => STD_ERROR_HANDLE,
    };
    // The typed identifier is valid; lookup does not dereference, close, or
    // perform I/O on the returned handle, which remains process-owned.
    unsafe { GetStdHandle(identifier) }
}

#[cfg(windows)]
fn valid_handle(stream: StandardStream) -> Option<*mut c_void> {
    let handle = handle(stream);
    (!handle.is_null() && handle != INVALID_HANDLE_VALUE).then_some(handle)
}

#[cfg(windows)]
unsafe fn is_interactive_unclaimed(stream: StandardStream) -> bool {
    let Some(handle) = valid_handle(stream) else {
        return false;
    };
    is_console_handle(handle)
}

#[cfg(windows)]
unsafe fn is_console_handle(handle: *mut c_void) -> bool {
    let mut mode = 0_u32;
    GetConsoleMode(handle, &mut mode) != 0
}

#[cfg(windows)]
unsafe fn write_unclaimed(
    stream: StandardStream,
    bytes: *const u8,
    byte_length: usize,
) -> WriteOutcome {
    if stream == StandardStream::Stdin {
        return WriteOutcome::OtherFailure(crate::platform_io::Failure::invalid_input());
    }
    let Some(handle) = valid_handle(stream) else {
        return WriteOutcome::OtherFailure(crate::platform_io::Failure::from_system_code(
            i64::from(GetLastError()),
        ));
    };
    if is_console_handle(handle) {
        return write_console_utf8(handle, bytes, byte_length);
    }
    write_file_bytes(handle, bytes, byte_length)
}

#[cfg(windows)]
unsafe fn write_bytes_unclaimed(
    stream: StandardStream,
    bytes: *const u8,
    byte_length: usize,
) -> WriteOutcome {
    if stream == StandardStream::Stdin {
        return WriteOutcome::OtherFailure(crate::platform_io::Failure::invalid_input());
    }
    let Some(handle) = valid_handle(stream) else {
        return WriteOutcome::OtherFailure(crate::platform_io::Failure::from_system_code(
            i64::from(GetLastError()),
        ));
    };
    write_file_bytes(handle, bytes, byte_length)
}

#[cfg(windows)]
unsafe fn write_file_bytes(
    handle: *mut c_void,
    bytes: *const u8,
    byte_length: usize,
) -> WriteOutcome {
    if byte_length != 0 {
        if let Err(failure) = crate::native_handle::require_synchronous_device(handle) {
            return WriteOutcome::OtherFailure(failure);
        }
    }
    match crate::byte_io::write_all(byte_length, |offset, remaining| {
        crate::byte_io::write(handle, bytes.add(offset), remaining)
    }) {
        Ok(()) => WriteOutcome::Success,
        Err(failure) => classify_windows_write_error(Some(failure)),
    }
}

#[cfg(windows)]
unsafe fn write_console_utf8(
    handle: *mut c_void,
    bytes: *const u8,
    byte_length: usize,
) -> WriteOutcome {
    let input = core::slice::from_raw_parts(bytes, byte_length);
    let Ok(text) = core::str::from_utf8(input) else {
        return WriteOutcome::OtherFailure(crate::platform_io::Failure::invalid_input());
    };
    let mut wide = [0_u16; 1024];
    let mut length = 0_usize;
    let mut completed_bytes = 0_usize;
    let mut chunk_bytes = 0_usize;
    for character in text.chars() {
        let mut encoded = [0_u16; 2];
        let units = character.encode_utf16(&mut encoded);
        if length + units.len() > wide.len() {
            if let Err(error) = write_console_units(handle, &wide[..length]) {
                return console_write_failure(error, &wide[..length], completed_bytes);
            }
            completed_bytes += chunk_bytes;
            length = 0;
            chunk_bytes = 0;
        }
        wide[length..length + units.len()].copy_from_slice(units);
        length += units.len();
        chunk_bytes += character.len_utf8();
    }
    if length == 0 {
        WriteOutcome::Success
    } else {
        match write_console_units(handle, &wide[..length]) {
            Ok(()) => WriteOutcome::Success,
            Err(error) => console_write_failure(error, &wide[..length], completed_bytes),
        }
    }
}

#[cfg(windows)]
unsafe fn write_console_units(handle: *mut c_void, units: &[u16]) -> Result<(), ConsoleWriteError> {
    write_console_units_with(units.len(), |offset, request| {
        let mut written = 0_u32;
        if WriteConsoleW(
            handle,
            units.as_ptr().add(offset).cast::<c_void>(),
            request as u32,
            &mut written,
            ptr::null_mut(),
        ) == 0
        {
            Err(crate::platform_io::Failure::from_system_code(
                GetLastError().into(),
            ))
        } else {
            Ok(written as usize)
        }
    })
}

#[cfg(any(windows, test))]
struct ConsoleWriteError {
    failure: crate::platform_io::Failure,
    transferred_units: Option<usize>,
}

#[cfg(any(windows, test))]
fn write_console_units_with(
    length: usize,
    mut transfer: impl FnMut(usize, usize) -> Result<usize, crate::platform_io::Failure>,
) -> Result<(), ConsoleWriteError> {
    let mut offset = 0;
    while offset < length {
        let request = core::cmp::min(length - offset, u32::MAX as usize);
        match transfer(offset, request) {
            Ok(0) => {
                return Err(ConsoleWriteError {
                    failure: crate::platform_io::Failure::other(),
                    transferred_units: Some(offset),
                })
            }
            Ok(written) if written <= request => offset += written,
            Ok(_) => {
                return Err(ConsoleWriteError {
                    failure: crate::platform_io::Failure::other(),
                    transferred_units: None,
                })
            }
            Err(failure) => {
                return Err(ConsoleWriteError {
                    failure,
                    // WriteConsole's failure does not guarantee a valid count.
                    transferred_units: None,
                });
            }
        }
    }
    Ok(())
}

#[cfg(any(windows, test))]
fn utf16_prefix_byte_length(units: &[u16], count: usize) -> Option<usize> {
    core::char::decode_utf16(units.get(..count)?.iter().copied())
        .try_fold(0_usize, |length, character| {
            length.checked_add(character.ok()?.len_utf8())
        })
}

#[cfg(any(windows, test))]
fn console_write_failure(
    error: ConsoleWriteError,
    units: &[u16],
    completed_bytes: usize,
) -> WriteOutcome {
    let transferred = error
        .transferred_units
        .and_then(|count| utf16_prefix_byte_length(units, count))
        .and_then(|count| completed_bytes.checked_add(count));
    let failure = error.failure.with_transferred_byte_count(transferred);
    #[cfg(windows)]
    {
        classify_windows_write_error(Some(failure))
    }
    #[cfg(not(windows))]
    {
        WriteOutcome::OtherFailure(failure)
    }
}

/// Fills fresh text input into the owning queue's free tail. Nonempty console
/// destinations must have at least TEXT_READ_MINIMUM bytes available.
#[cfg(windows)]
pub(crate) unsafe fn read_text(
    stream: StandardStream,
    destination: *mut u8,
    capacity: usize,
) -> Result<usize, crate::platform_io::Failure> {
    if stream != StandardStream::Stdin {
        return Err(crate::platform_io::Failure::invalid_input());
    }
    if capacity == 0 {
        return Ok(0);
    }
    let Some(handle) = valid_handle(stream) else {
        return Err(crate::platform_io::Failure::from_system_code(i64::from(
            GetLastError(),
        )));
    };
    if !is_console_handle(handle) {
        return read_from_handle(handle, destination, capacity);
    }

    let mut wide = [0_u16; 512];
    let mut read = 0_u32;
    if ReadConsoleW(
        handle,
        wide.as_mut_ptr().cast::<c_void>(),
        wide.len() as u32,
        &mut read,
        ptr::null_mut(),
    ) == 0
    {
        return Err(crate::platform_io::Failure::from_system_code(i64::from(
            GetLastError(),
        )));
    }
    utf16_to_utf8(&wide[..read as usize], destination, capacity)
        .ok_or_else(crate::platform_io::Failure::invalid_input)
}

#[cfg(windows)]
unsafe fn read_raw_stdin(
    destination: *mut u8,
    capacity: usize,
) -> Result<usize, crate::platform_io::Failure> {
    let Some(handle) = valid_handle(StandardStream::Stdin) else {
        return Err(crate::platform_io::Failure::from_system_code(i64::from(
            GetLastError(),
        )));
    };
    read_from_handle(handle, destination, capacity)
}

#[cfg(any(windows, test))]
unsafe fn utf16_to_utf8(units: &[u16], destination: *mut u8, capacity: usize) -> Option<usize> {
    let mut written = 0_usize;
    for decoded in core::char::decode_utf16(units.iter().copied()) {
        let character = decoded.ok()?;
        let mut encoded = [0_u8; 4];
        let bytes = character.encode_utf8(&mut encoded).as_bytes();
        if written.checked_add(bytes.len())? > capacity {
            return None;
        }
        ptr::copy_nonoverlapping(bytes.as_ptr(), destination.add(written), bytes.len());
        written += bytes.len();
    }
    Some(written)
}

#[cfg(not(any(unix, windows)))]
pub(crate) unsafe fn read_text(
    _stream: StandardStream,
    _destination: *mut u8,
    _capacity: usize,
) -> Result<usize, crate::platform_io::Failure> {
    Err(crate::platform_io::Failure::unsupported())
}

#[cfg(not(any(unix, windows)))]
unsafe fn write_unclaimed(
    _stream: StandardStream,
    _bytes: *const u8,
    _byte_length: usize,
) -> WriteOutcome {
    WriteOutcome::OtherFailure(crate::platform_io::Failure::unsupported())
}

#[cfg(not(any(unix, windows)))]
unsafe fn write_bytes_unclaimed(
    stream: StandardStream,
    bytes: *const u8,
    byte_length: usize,
) -> WriteOutcome {
    write_unclaimed(stream, bytes, byte_length)
}

#[cfg(not(any(unix, windows)))]
unsafe fn read_raw_stdin(
    destination: *mut u8,
    capacity: usize,
) -> Result<usize, crate::platform_io::Failure> {
    read_text(StandardStream::Stdin, destination, capacity)
}

#[cfg(not(any(unix, windows)))]
unsafe fn is_interactive_unclaimed(_stream: StandardStream) -> bool {
    false
}

pub(crate) unsafe fn is_interactive(stream: StandardStream) -> bool {
    // Handle metadata does not access the mutable input queue or transfer bytes.
    // Windows text reads also query it while already holding their device loan.
    is_interactive_unclaimed(stream)
}

#[cfg(unix)]
extern "C" {
    fn isatty(descriptor: i32) -> i32;
}

#[cfg(windows)]
use crate::win32::{GetConsoleMode, GetLastError, GetStdHandle, ReadConsoleW, WriteConsoleW};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_identifiers_are_stable_and_independent() {
        assert_eq!(StandardStream::Stdin as u8, 0);
        assert_eq!(StandardStream::Stdout as u8, 1);
        assert_eq!(StandardStream::Stderr as u8, 2);
    }

    unsafe fn line_with(
        input: &mut InputBuffer,
        transfer: impl FnMut(*mut u8, usize) -> Result<usize, crate::platform_io::Failure>,
    ) -> Result<Option<std::vec::Vec<u8>>, crate::line_io::ReadLineError> {
        crate::line_io::read_line_with(input, transfer).map(|line| {
            line.map(|(bytes, length)| core::slice::from_raw_parts(bytes, length).to_vec())
        })
    }

    #[test]
    fn line_byte_line_consumers_share_pending_input_in_order() {
        unsafe {
            let mut input = InputBuffer::new();
            let source = b"first\nABCDEsecond\nlast";
            let mut calls = 0;
            assert_eq!(
                line_with(&mut input, |destination, capacity| {
                    calls += 1;
                    assert!(capacity >= source.len());
                    ptr::copy_nonoverlapping(source.as_ptr(), destination, source.len());
                    Ok(source.len())
                }),
                Ok(Some(b"first".to_vec()))
            );

            let mut bytes = [0; 16];
            assert_eq!(
                input.read_bytes_with(bytes.as_mut_ptr(), 3, |_, _| {
                    panic!("queued bytes must precede device input")
                }),
                Ok(3)
            );
            assert_eq!(&bytes[..3], b"ABC");
            assert_eq!(
                line_with(&mut input, |_, _| {
                    panic!("queued line must precede device input")
                }),
                Ok(Some(b"DEsecond".to_vec()))
            );
            assert_eq!(
                input.read_bytes_with(bytes.as_mut_ptr(), bytes.len(), |_, _| {
                    panic!("queued suffix must precede device input")
                }),
                Ok(4)
            );
            assert_eq!(&bytes[..4], b"last");
            assert_eq!(calls, 1);
            assert_eq!(
                line_with(&mut input, |_, _| {
                    calls += 1;
                    Ok(0)
                }),
                Ok(None)
            );
            assert_eq!(calls, 2);
        }
    }

    #[test]
    fn converted_console_bytes_share_the_queue_and_allow_partial_binary_drains() {
        unsafe {
            let mut input = InputBuffer::new();
            let source: std::vec::Vec<u16> = "first\né🎮\r\n".encode_utf16().collect();
            assert_eq!(
                line_with(&mut input, |destination, capacity| {
                    Ok(utf16_to_utf8(&source, destination, capacity).unwrap())
                }),
                Ok(Some(b"first".to_vec()))
            );
            let mut received = std::vec::Vec::new();
            for capacity in [1, 0, 2, 3, 8] {
                let before = (input.start, input.end, input.scanned);
                let mut bytes = [0; 8];
                let count = input
                    .read_bytes_with(bytes.as_mut_ptr(), capacity, |_, _| {
                        panic!("converted bytes must not be bypassed")
                    })
                    .unwrap();
                if capacity == 0 {
                    assert_eq!((input.start, input.end, input.scanned), before);
                }
                received.extend_from_slice(&bytes[..count]);
            }
            assert_eq!(received, "é🎮\r\n".as_bytes());
            let raw = b"\xff\0\xc3(";
            let mut bytes = [0; 8];
            assert_eq!(
                input.read_bytes_with(bytes.as_mut_ptr(), bytes.len(), |destination, _| {
                    ptr::copy_nonoverlapping(raw.as_ptr(), destination, raw.len());
                    Ok(raw.len())
                }),
                Ok(raw.len())
            );
            assert_eq!(&bytes[..raw.len()], raw);
        }
    }

    #[test]
    fn empty_byte_requests_do_not_allocate_consume_or_call_the_device() {
        let _serial = device::test_lock();
        unsafe {
            let mut input = InputBuffer::new();
            assert_eq!(
                input.read_bytes_with(ptr::null_mut(), 0, |_, _| panic!("empty request")),
                Ok(0)
            );
            assert!(input.bytes.is_null());
            assert_eq!(input.capacity, 0);
            assert_eq!(
                standard_device(StandardStream::Stdin)
                    .borrow()
                    .read_bytes(ptr::null_mut(), 0),
                Ok(0)
            );
            #[cfg(any(unix, windows))]
            assert_eq!(read_text(StandardStream::Stdin, ptr::null_mut(), 0), Ok(0));
        }
    }

    #[test]
    fn empty_queue_binary_reads_keep_raw_bytes_and_do_not_allocate_a_queue() {
        unsafe {
            let mut input = InputBuffer::new();
            let raw = b"\xff\0\xc3(";
            let mut bytes = [0; 4];
            let mut calls = 0;
            assert_eq!(
                input.read_bytes_with(bytes.as_mut_ptr(), bytes.len(), |destination, capacity| {
                    calls += 1;
                    assert_eq!(capacity, raw.len());
                    ptr::copy_nonoverlapping(raw.as_ptr(), destination, raw.len());
                    Ok(raw.len())
                }),
                Ok(raw.len())
            );
            assert_eq!(&bytes, raw);
            assert_eq!(calls, 1);
            assert!(input.bytes.is_null());
            assert_eq!(input.capacity, 0);
        }
    }

    #[test]
    fn primitive_empty_data_would_block_and_eof_are_distinct_without_read_ahead() {
        unsafe {
            let mut input = InputBuffer::new();
            let mut destination = [99; 4];
            let would_block = Failure::other().with_transferred_byte_count(Some(0));
            let failure = Failure::other().with_transferred_byte_count(Some(2));
            let results = [
                Ok(ReadProgress::Data(0)),
                Ok(ReadProgress::WouldBlock(would_block)),
                Err(failure),
                Ok(ReadProgress::EndOfStream),
            ];
            for result in results {
                let mut calls = 0;
                assert_eq!(
                    input.read_progress_with(destination.as_mut_ptr(), 4, |_, capacity| {
                        calls += 1;
                        assert_eq!(capacity, 4);
                        result
                    }),
                    result
                );
                assert_eq!(calls, 1);
                assert!(!input.eof);
                assert!(input.bytes.is_null());
                assert_eq!(input.capacity, 0);
            }
            assert_eq!(destination, [99; 4]);
            input.eof = true;
            assert_eq!(
                input.read_progress_with(ptr::null_mut(), 0, |_, _| panic!("empty request")),
                Ok(ReadProgress::Data(0))
            );
            assert_eq!(
                input.read_progress_with(destination.as_mut_ptr(), 4, |_, _| panic!("known EOF")),
                Ok(ReadProgress::EndOfStream)
            );
        }
    }

    #[test]
    fn primitive_and_legacy_consumers_share_one_queue_and_preserve_binary_bytes() {
        unsafe {
            let mut input = InputBuffer::new();
            let source = b"first\n\xff\0second\nlast";
            assert_eq!(
                line_with(&mut input, |destination, _| {
                    ptr::copy_nonoverlapping(source.as_ptr(), destination, source.len());
                    Ok(source.len())
                }),
                Ok(Some(b"first".to_vec()))
            );
            let original_storage = input.bytes;
            let mut destination = [0; 8];
            assert_eq!(
                input.read_progress_with(destination.as_mut_ptr(), 1, |_, _| panic!("read ahead")),
                Ok(ReadProgress::Data(1))
            );
            assert_eq!(destination[0], 255);
            assert_eq!(
                input.read_bytes_with(destination.as_mut_ptr(), 1, |_, _| panic!("read ahead")),
                Ok(1)
            );
            assert_eq!(destination[0], 0);
            assert_eq!(
                line_with(&mut input, |_, _| panic!("queued line")),
                Ok(Some(b"second".to_vec()))
            );
            input.eof = true;
            assert_eq!(
                input.read_progress_with(destination.as_mut_ptr(), 8, |_, _| panic!(
                    "queued suffix"
                )),
                Ok(ReadProgress::Data(4))
            );
            assert_eq!(&destination[..4], b"last");
            assert_eq!(input.bytes, original_storage);
            assert_eq!(
                input.read_progress_with(destination.as_mut_ptr(), 8, |_, _| panic!("known EOF")),
                Ok(ReadProgress::EndOfStream)
            );
        }
    }

    #[test]
    fn repeated_primitive_reads_reuse_caller_storage_without_runtime_allocation() {
        unsafe {
            let mut input = InputBuffer::new();
            let mut destination = [0; 64];
            let original_storage = destination.as_mut_ptr();
            let (_, allocations) = crate::probe_runtime_allocations(|| {
                for _ in 0..1024 {
                    assert_eq!(
                        input.read_progress_with(
                            original_storage,
                            destination.len(),
                            |bytes, capacity| {
                                assert_eq!(bytes, original_storage);
                                assert_eq!(capacity, destination.len());
                                *bytes = 255;
                                Ok(ReadProgress::Data(1))
                            }
                        ),
                        Ok(ReadProgress::Data(1))
                    );
                }
            });
            assert_eq!(allocations, 0);
            assert_eq!(destination[0], 255);
            assert!(input.bytes.is_null());
            assert_eq!(input.capacity, 0);
        }
    }

    #[test]
    fn failed_text_read_retains_known_progress_for_binary_drain_and_line_retry() {
        use crate::line_io::ReadLineError;
        unsafe {
            let mut input = InputBuffer::new();
            let failure = crate::platform_io::Failure::from_system_code(13);
            let mut calls = 0;
            assert_eq!(
                line_with(&mut input, |destination, _| {
                    calls += 1;
                    let (bytes, result) = if calls == 1 {
                        (b"abc".as_slice(), Ok(3))
                    } else {
                        (
                            b"de".as_slice(),
                            Err(failure.with_transferred_byte_count(Some(2))),
                        )
                    };
                    ptr::copy_nonoverlapping(bytes.as_ptr(), destination, bytes.len());
                    result
                }),
                Err(ReadLineError::Read(
                    failure.with_transferred_byte_count(Some(5))
                ))
            );
            assert_eq!(
                line_with(&mut input, |_, _| Err(
                    failure.with_transferred_byte_count(Some(0))
                )),
                Err(ReadLineError::Read(
                    failure.with_transferred_byte_count(Some(0))
                ))
            );
            let mut bytes = [0; 2];
            assert_eq!(
                input.read_bytes_with(bytes.as_mut_ptr(), bytes.len(), |_, _| panic!(
                    "pending prefix"
                )),
                Ok(2)
            );
            assert_eq!(&bytes, b"ab");
            assert_eq!(
                line_with(&mut input, |destination, _| {
                    ptr::copy_nonoverlapping(b"\nnext".as_ptr(), destination, 5);
                    Ok(5)
                }),
                Ok(Some(b"cde".to_vec()))
            );
            let pending = input.take_buffered_input();
            assert_eq!(
                core::slice::from_raw_parts(pending.bytes, pending.length),
                b"next"
            );
        }
    }

    #[test]
    fn unknown_text_failure_does_not_claim_zero_or_lose_the_successful_prefix() {
        use crate::line_io::ReadLineError;
        unsafe {
            let mut input = InputBuffer::new();
            let failure = crate::platform_io::Failure::other();
            let mut calls = 0;
            assert_eq!(
                line_with(&mut input, |destination, _| {
                    calls += 1;
                    if calls == 1 {
                        ptr::copy_nonoverlapping(b"abc".as_ptr(), destination, 3);
                        Ok(3)
                    } else {
                        Err(failure)
                    }
                }),
                Err(ReadLineError::Read(failure))
            );
            let mut bytes = [0; 3];
            assert_eq!(
                input.read_bytes_with(bytes.as_mut_ptr(), bytes.len(), |_, _| panic!(
                    "pending prefix"
                )),
                Ok(3)
            );
            assert_eq!(&bytes, b"abc");
        }
    }

    #[test]
    fn queued_bytes_are_delivered_before_observed_text_eof() {
        unsafe {
            let mut input = InputBuffer::new();
            input.ensure_capacity(4096).unwrap();
            ptr::copy_nonoverlapping(b"abc\nlast".as_ptr(), input.bytes, 8);
            input.end = 8;
            input.eof = true;
            let mut bytes = [0; 2];
            assert_eq!(
                input.read_bytes_with(bytes.as_mut_ptr(), 2, |_, _| panic!("already EOF")),
                Ok(2)
            );
            assert_eq!(&bytes, b"ab");
            assert_eq!(
                line_with(&mut input, |_, _| panic!("already EOF")),
                Ok(Some(b"c".to_vec()))
            );
            assert_eq!(
                line_with(&mut input, |_, _| panic!("already EOF")),
                Ok(Some(b"last".to_vec()))
            );
            assert_eq!(
                input.read_bytes_with(bytes.as_mut_ptr(), 2, |_, _| panic!("already EOF")),
                Ok(0)
            );
        }
    }

    #[test]
    fn console_conversion_space_preserves_pending_bytes_and_scan_position() {
        unsafe {
            let mut input = InputBuffer::new();
            input.ensure_capacity(4096).unwrap();
            ptr::write_bytes(input.bytes, b'a', 4090);
            input.start = 100;
            input.end = 4090;
            input.scanned = 4090;
            input.prepare_write_space(2048).unwrap();
            assert_eq!(input.start, 0);
            assert_eq!(input.end, 3990);
            assert_eq!(input.scanned, 3990);
            assert!(input.capacity - input.end >= 2048);
            assert!(core::slice::from_raw_parts(input.bytes, input.end)
                .iter()
                .all(|byte| *byte == b'a'));
            let wide = [0x6f22; 512];
            let count = utf16_to_utf8(
                &wide,
                input.bytes.add(input.end),
                input.capacity - input.end,
            )
            .unwrap();
            input.end += count;
            assert_eq!(count, 1536);
            assert_eq!(
                core::str::from_utf8(core::slice::from_raw_parts(input.bytes.add(3990), count))
                    .unwrap(),
                "漢".repeat(512)
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn unix_write_outcomes_distinguish_broken_pipes() {
        assert_eq!(classify_unix_write_error(None), WriteOutcome::Success);
        assert_eq!(
            classify_unix_write_error(Some(crate::platform_io::Failure::from_system_code(
                EPIPE as i64
            ))),
            WriteOutcome::BrokenPipe
        );
        let failure =
            crate::platform_io::Failure::from_system_code(5).with_transferred_byte_count(Some(17));
        assert_eq!(
            classify_unix_write_error(Some(failure)),
            WriteOutcome::OtherFailure(failure)
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_write_outcomes_distinguish_broken_pipes() {
        assert_eq!(classify_windows_write_error(None), WriteOutcome::Success);
        assert_eq!(
            classify_windows_write_error(Some(crate::platform_io::Failure::from_system_code(
                ERROR_BROKEN_PIPE as i64
            ))),
            WriteOutcome::BrokenPipe
        );
        assert_eq!(
            classify_windows_write_error(Some(crate::platform_io::Failure::from_system_code(
                ERROR_NO_DATA as i64
            ))),
            WriteOutcome::BrokenPipe
        );
        let failure =
            crate::platform_io::Failure::from_system_code(5).with_transferred_byte_count(Some(17));
        assert_eq!(
            classify_windows_write_error(Some(failure)),
            WriteOutcome::OtherFailure(failure)
        );
    }

    #[test]
    fn console_progress_maps_only_complete_utf8_scalar_prefixes() {
        let units: std::vec::Vec<u16> = "aé🎮z".encode_utf16().collect();
        for (count, expected) in [
            (0, Some(0)),
            (1, Some(1)),
            (2, Some(3)),
            (3, None),
            (4, Some(7)),
            (5, Some(8)),
            (6, None),
        ] {
            assert_eq!(utf16_prefix_byte_length(&units, count), expected);
        }
        for (accepted, expected) in [(2, Some(13)), (3, None)] {
            let error = write_console_units_with(units.len(), |offset, _| {
                Ok(if offset == 0 { accepted } else { 0 })
            })
            .err()
            .unwrap();
            let WriteOutcome::OtherFailure(failure) = console_write_failure(error, &units, 10)
            else {
                panic!("non-pipe failure");
            };
            assert_eq!(failure.transferred_byte_count, expected);
        }
        let unknown = crate::platform_io::Failure::other();
        let error =
            write_console_units_with(
                units.len(),
                |offset, _| {
                    if offset == 0 {
                        Ok(2)
                    } else {
                        Err(unknown)
                    }
                },
            )
            .err()
            .unwrap();
        let WriteOutcome::OtherFailure(failure) = console_write_failure(error, &units, 10) else {
            panic!("non-pipe failure");
        };
        assert_eq!(failure.transferred_byte_count, None);
    }

    #[cfg(unix)]
    #[test]
    fn pipes_and_redirected_files_are_not_interactive() {
        use std::fs::File;
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixStream;

        let (left, right) = UnixStream::pair().expect("Unix stream pair");
        let redirected = File::open("/dev/null").expect("/dev/null");
        unsafe {
            assert!(!is_interactive_descriptor(left.as_raw_fd()));
            assert!(!is_interactive_descriptor(right.as_raw_fd()));
            assert!(!is_interactive_descriptor(redirected.as_raw_fd()));
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pseudo_terminal_master_is_interactive() {
        use std::fs::OpenOptions;
        use std::os::fd::AsRawFd;

        let terminal = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/ptmx")
            .expect("Linux PTY multiplexer");
        assert!(unsafe { is_interactive_descriptor(terminal.as_raw_fd()) });
    }

    #[cfg(windows)]
    #[test]
    fn redirected_windows_file_handle_is_not_a_console() {
        use std::fs::File;
        use std::os::windows::io::AsRawHandle;

        let file = File::open("NUL").expect("Windows NUL device");
        assert!(!unsafe { is_console_handle(file.as_raw_handle().cast()) });
    }

    #[test]
    fn utf16_conversion_preserves_unicode_and_embedded_nul() {
        let source = "Doria — café — 漢字 — 🎮\0done"
            .encode_utf16()
            .collect::<std::vec::Vec<_>>();
        let mut output = [0_u8; 128];
        let length = unsafe { utf16_to_utf8(&source, output.as_mut_ptr(), output.len()) }
            .expect("valid UTF-16 should convert");
        assert_eq!(
            &output[..length],
            "Doria — café — 漢字 — 🎮\0done".as_bytes()
        );
    }
}
