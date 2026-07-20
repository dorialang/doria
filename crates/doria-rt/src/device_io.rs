use core::ffi::c_void;
#[cfg(windows)]
use core::ptr;

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
    OtherFailure,
}

#[cfg(unix)]
const EINTR: i32 = 4;
#[cfg(unix)]
const EPIPE: i32 = 32;

#[cfg(unix)]
const fn classify_unix_write_error(error: Option<i32>) -> WriteOutcome {
    match error {
        None => WriteOutcome::Success,
        Some(EPIPE) => WriteOutcome::BrokenPipe,
        Some(_) => WriteOutcome::OtherFailure,
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
/// Returns `Ok(0)` only for EOF. `destination` must be writable for `capacity` bytes.
#[cfg(unix)]
pub(crate) unsafe fn read(
    stream: StandardStream,
    destination: *mut u8,
    capacity: usize,
) -> Result<usize, ()> {
    if stream != StandardStream::Stdin {
        return Err(());
    }
    loop {
        let result = libc_read(descriptor(stream), destination.cast::<c_void>(), capacity);
        if result >= 0 {
            return Ok(result as usize);
        }
        if last_errno() != EINTR {
            return Err(());
        }
    }
}

/// Writes raw bytes without adding text or line semantics.
#[cfg(unix)]
pub(crate) unsafe fn write(
    stream: StandardStream,
    bytes: *const u8,
    byte_length: usize,
) -> WriteOutcome {
    if stream == StandardStream::Stdin {
        return WriteOutcome::OtherFailure;
    }
    let mut offset = 0;
    while offset < byte_length {
        let result = libc_write(
            descriptor(stream),
            bytes.add(offset).cast::<c_void>(),
            byte_length - offset,
        );
        if result > 0 {
            offset += result as usize;
        } else if result < 0 && last_errno() == EINTR {
            continue;
        } else {
            return if result < 0 {
                classify_unix_write_error(Some(last_errno()))
            } else {
                WriteOutcome::OtherFailure
            };
        }
    }
    WriteOutcome::Success
}

/// Raw writes are currently unbuffered, so flushing is intentionally a successful no-op.
pub(crate) unsafe fn flush(stream: StandardStream) -> bool {
    stream != StandardStream::Stdin
}

#[cfg(unix)]
pub(crate) unsafe fn is_interactive(stream: StandardStream) -> bool {
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
const fn classify_windows_write_error(error: Option<u32>) -> WriteOutcome {
    match error {
        None => WriteOutcome::Success,
        Some(ERROR_BROKEN_PIPE) | Some(ERROR_NO_DATA) => WriteOutcome::BrokenPipe,
        Some(_) => WriteOutcome::OtherFailure,
    }
}

#[cfg(windows)]
fn is_pipe_eof(error: u32) -> bool {
    error == ERROR_BROKEN_PIPE
}

#[cfg(windows)]
unsafe fn handle(stream: StandardStream) -> *mut c_void {
    let identifier = match stream {
        StandardStream::Stdin => STD_INPUT_HANDLE,
        StandardStream::Stdout => STD_OUTPUT_HANDLE,
        StandardStream::Stderr => STD_ERROR_HANDLE,
    };
    GetStdHandle(identifier)
}

#[cfg(windows)]
unsafe fn valid_handle(stream: StandardStream) -> Option<*mut c_void> {
    let handle = handle(stream);
    (!handle.is_null() && handle != INVALID_HANDLE_VALUE).then_some(handle)
}

#[cfg(windows)]
pub(crate) unsafe fn is_interactive(stream: StandardStream) -> bool {
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
pub(crate) unsafe fn write(
    stream: StandardStream,
    bytes: *const u8,
    byte_length: usize,
) -> WriteOutcome {
    if stream == StandardStream::Stdin {
        return WriteOutcome::OtherFailure;
    }
    let Some(handle) = valid_handle(stream) else {
        return WriteOutcome::OtherFailure;
    };
    if is_interactive(stream) {
        return write_console_utf8(handle, bytes, byte_length);
    }
    write_file_bytes(handle, bytes, byte_length)
}

#[cfg(windows)]
unsafe fn write_file_bytes(
    handle: *mut c_void,
    bytes: *const u8,
    byte_length: usize,
) -> WriteOutcome {
    let mut offset = 0;
    while offset < byte_length {
        let request = core::cmp::min(byte_length - offset, u32::MAX as usize) as u32;
        let mut written = 0_u32;
        let result = WriteFile(
            handle,
            bytes.add(offset).cast::<c_void>(),
            request,
            &mut written,
            ptr::null_mut(),
        );
        if result == 0 {
            return classify_windows_write_error(Some(GetLastError()));
        }
        if written == 0 {
            return WriteOutcome::OtherFailure;
        }
        offset += written as usize;
    }
    WriteOutcome::Success
}

#[cfg(windows)]
unsafe fn write_console_utf8(
    handle: *mut c_void,
    bytes: *const u8,
    byte_length: usize,
) -> WriteOutcome {
    let input = core::slice::from_raw_parts(bytes, byte_length);
    let Ok(text) = core::str::from_utf8(input) else {
        return WriteOutcome::OtherFailure;
    };
    let mut wide = [0_u16; 1024];
    let mut length = 0_usize;
    for character in text.chars() {
        let mut encoded = [0_u16; 2];
        let units = character.encode_utf16(&mut encoded);
        if length + units.len() > wide.len() {
            let outcome = write_console_units(handle, wide.as_ptr(), length);
            if outcome != WriteOutcome::Success {
                return outcome;
            }
            length = 0;
        }
        wide[length..length + units.len()].copy_from_slice(units);
        length += units.len();
    }
    if length == 0 {
        WriteOutcome::Success
    } else {
        write_console_units(handle, wide.as_ptr(), length)
    }
}

#[cfg(windows)]
unsafe fn write_console_units(
    handle: *mut c_void,
    units: *const u16,
    length: usize,
) -> WriteOutcome {
    let mut offset = 0;
    while offset < length {
        let request = core::cmp::min(length - offset, u32::MAX as usize) as u32;
        let mut written = 0_u32;
        let result = WriteConsoleW(
            handle,
            units.add(offset).cast::<c_void>(),
            request,
            &mut written,
            ptr::null_mut(),
        );
        if result == 0 {
            return classify_windows_write_error(Some(GetLastError()));
        }
        if written == 0 {
            return WriteOutcome::OtherFailure;
        }
        offset += written as usize;
    }
    WriteOutcome::Success
}

#[cfg(windows)]
static mut CONSOLE_INPUT_BYTES: [u8; 2048] = [0; 2048];
#[cfg(windows)]
static mut CONSOLE_INPUT_START: usize = 0;
#[cfg(windows)]
static mut CONSOLE_INPUT_END: usize = 0;

#[cfg(windows)]
pub(crate) unsafe fn read(
    stream: StandardStream,
    destination: *mut u8,
    capacity: usize,
) -> Result<usize, ()> {
    if stream != StandardStream::Stdin {
        return Err(());
    }
    let Some(handle) = valid_handle(stream) else {
        return Err(());
    };
    if !is_interactive(stream) {
        let request = core::cmp::min(capacity, u32::MAX as usize) as u32;
        let mut read = 0_u32;
        if ReadFile(
            handle,
            destination.cast::<c_void>(),
            request,
            &mut read,
            ptr::null_mut(),
        ) == 0
        {
            if is_pipe_eof(GetLastError()) {
                return Ok(0);
            }
            return Err(());
        }
        return Ok(read as usize);
    }

    if CONSOLE_INPUT_START == CONSOLE_INPUT_END {
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
            return Err(());
        }
        if read == 0 {
            return Ok(0);
        }
        CONSOLE_INPUT_START = 0;
        CONSOLE_INPUT_END = utf16_to_utf8(
            &wide[..read as usize],
            core::ptr::addr_of_mut!(CONSOLE_INPUT_BYTES).cast::<u8>(),
            2048,
        )
        .ok_or(())?;
    }

    let available = CONSOLE_INPUT_END - CONSOLE_INPUT_START;
    let copied = core::cmp::min(available, capacity);
    ptr::copy_nonoverlapping(
        core::ptr::addr_of!(CONSOLE_INPUT_BYTES)
            .cast::<u8>()
            .add(CONSOLE_INPUT_START),
        destination,
        copied,
    );
    CONSOLE_INPUT_START += copied;
    Ok(copied)
}

#[cfg(windows)]
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
pub(crate) unsafe fn read(
    _stream: StandardStream,
    _destination: *mut u8,
    _capacity: usize,
) -> Result<usize, ()> {
    Err(())
}

#[cfg(not(any(unix, windows)))]
pub(crate) unsafe fn write(
    _stream: StandardStream,
    _bytes: *const u8,
    _byte_length: usize,
) -> WriteOutcome {
    WriteOutcome::OtherFailure
}

#[cfg(not(any(unix, windows)))]
pub(crate) unsafe fn is_interactive(_stream: StandardStream) -> bool {
    false
}

#[cfg(all(unix, any(target_os = "linux", target_os = "android")))]
unsafe fn last_errno() -> i32 {
    *__errno_location()
}

#[cfg(all(
    unix,
    any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    )
))]
unsafe fn last_errno() -> i32 {
    *__error()
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    ))
))]
unsafe fn last_errno() -> i32 {
    0
}

#[cfg(unix)]
extern "C" {
    #[link_name = "read"]
    fn libc_read(descriptor: i32, bytes: *mut c_void, byte_length: usize) -> isize;
    #[link_name = "write"]
    fn libc_write(descriptor: i32, bytes: *const c_void, byte_length: usize) -> isize;
    fn isatty(descriptor: i32) -> i32;
}

#[cfg(all(unix, any(target_os = "linux", target_os = "android")))]
extern "C" {
    fn __errno_location() -> *mut i32;
}

#[cfg(all(
    unix,
    any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        target_os = "dragonfly"
    )
))]
extern "C" {
    fn __error() -> *mut i32;
}

#[cfg(windows)]
extern "system" {
    fn GetStdHandle(standard_handle: u32) -> *mut c_void;
    fn GetLastError() -> u32;
    fn GetConsoleMode(handle: *mut c_void, mode: *mut u32) -> i32;
    fn ReadFile(
        handle: *mut c_void,
        bytes: *mut c_void,
        byte_length: u32,
        read: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn ReadConsoleW(
        handle: *mut c_void,
        buffer: *mut c_void,
        characters: u32,
        read: *mut u32,
        input_control: *mut c_void,
    ) -> i32;
    fn WriteFile(
        handle: *mut c_void,
        bytes: *const c_void,
        byte_length: u32,
        written: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn WriteConsoleW(
        handle: *mut c_void,
        buffer: *const c_void,
        characters: u32,
        written: *mut u32,
        reserved: *mut c_void,
    ) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_identifiers_are_stable_and_independent() {
        assert_eq!(StandardStream::Stdin as u8, 0);
        assert_eq!(StandardStream::Stdout as u8, 1);
        assert_eq!(StandardStream::Stderr as u8, 2);
    }

    #[cfg(unix)]
    #[test]
    fn unix_write_outcomes_distinguish_broken_pipes() {
        assert_eq!(classify_unix_write_error(None), WriteOutcome::Success);
        assert_eq!(
            classify_unix_write_error(Some(EPIPE)),
            WriteOutcome::BrokenPipe
        );
        assert_eq!(
            classify_unix_write_error(Some(5)),
            WriteOutcome::OtherFailure
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_write_outcomes_distinguish_broken_pipes() {
        assert_eq!(classify_windows_write_error(None), WriteOutcome::Success);
        assert_eq!(
            classify_windows_write_error(Some(ERROR_BROKEN_PIPE)),
            WriteOutcome::BrokenPipe
        );
        assert_eq!(
            classify_windows_write_error(Some(ERROR_NO_DATA)),
            WriteOutcome::BrokenPipe
        );
        assert_eq!(
            classify_windows_write_error(Some(5)),
            WriteOutcome::OtherFailure
        );
    }

    #[cfg(windows)]
    #[test]
    fn closed_pipe_is_stdin_eof() {
        assert!(is_pipe_eof(ERROR_BROKEN_PIPE));
        assert!(!is_pipe_eof(5));
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

    #[cfg(windows)]
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
