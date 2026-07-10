#![cfg_attr(all(not(test), panic = "abort"), no_std)]

// Linked runtime artifacts always use panic=abort; unwind-mode builds exist only for check/test metadata.

use core::ffi::c_void;
use core::ptr;

const PANIC_STATUS: i32 = 101;
const EINTR: i32 = 4;

#[repr(C)]
pub struct DrStackFrameV1 {
    pub parent: *const DrStackFrameV1,
    pub function_name: *const u8,
    pub function_name_length: usize,
}

pub type DrMainIntV1 = unsafe extern "C" fn(*const DrStackFrameV1) -> i64;
pub type DrMainVoidV1 = unsafe extern "C" fn(*const DrStackFrameV1);

/// Invokes a generated Doria integer entry function and maps its result to a process status.
///
/// # Safety
///
/// `entry` must point to a generated function that implements `DrMainIntV1` and remains valid
/// for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_main_int(entry: DrMainIntV1) -> i32 {
    let status = entry(ptr::null());
    if (0..=125).contains(&status) {
        return status as i32;
    }

    static MAIN: &[u8] = b"main";
    static MESSAGE: &[u8] = b"main returned process status outside 0..125";
    let frame = DrStackFrameV1 {
        parent: ptr::null(),
        function_name: MAIN.as_ptr(),
        function_name_length: MAIN.len(),
    };
    dr_v1_panic(&frame, MESSAGE.as_ptr(), MESSAGE.len())
}

/// Invokes a generated Doria void entry function.
///
/// # Safety
///
/// `entry` must point to a generated function that implements `DrMainVoidV1` and remains valid
/// for the duration of the call.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_main_void(entry: DrMainVoidV1) -> i32 {
    entry(ptr::null());
    0
}

/// Writes an exact byte sequence to stdout or panics when the write fails.
///
/// # Safety
///
/// `bytes` must be readable for `byte_length` bytes. `current_frame` must be null or point to a
/// valid `DrStackFrameV1` chain whose frame and function-name storage remains live for the call.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_write_stdout(
    current_frame: *const DrStackFrameV1,
    bytes: *const u8,
    byte_length: usize,
) {
    if write_stream(Stream::Stdout, bytes, byte_length) {
        return;
    }
    static MESSAGE: &[u8] = b"failed to write stdout";
    dr_v1_panic(current_frame, MESSAGE.as_ptr(), MESSAGE.len())
}

/// Writes an exact byte sequence to stderr.
///
/// # Safety
///
/// `bytes` must be readable for `byte_length` bytes.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_write_stderr(bytes: *const u8, byte_length: usize) {
    if !write_stream(Stream::Stderr, bytes, byte_length) {
        exit_process(PANIC_STATUS);
    }
}

/// Reports a fatal Doria panic and exits the process with status 101.
///
/// # Safety
///
/// `message` must be readable for `message_length` bytes. `current_frame` must be null or point to
/// a finite, valid `DrStackFrameV1` chain whose frames and function-name byte ranges remain live
/// until process termination.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_panic(
    current_frame: *const DrStackFrameV1,
    message: *const u8,
    message_length: usize,
) -> ! {
    write_panic_fragment(b"panic: ");
    write_panic_bytes(message, message_length);
    write_panic_fragment(b"\nstack trace:\n");

    let mut frame = current_frame;
    while !frame.is_null() {
        write_panic_fragment(b"  at ");
        write_panic_bytes((*frame).function_name, (*frame).function_name_length);
        write_panic_fragment(b"\n");
        frame = (*frame).parent;
    }
    exit_process(PANIC_STATUS)
}

#[derive(Clone, Copy)]
enum Stream {
    Stdout,
    Stderr,
}

unsafe fn write_panic_fragment(bytes: &[u8]) {
    write_panic_bytes(bytes.as_ptr(), bytes.len());
}

unsafe fn write_panic_bytes(bytes: *const u8, byte_length: usize) {
    if !write_stream(Stream::Stderr, bytes, byte_length) {
        exit_process(PANIC_STATUS);
    }
}

#[cfg(unix)]
unsafe fn write_stream(stream: Stream, bytes: *const u8, byte_length: usize) -> bool {
    let descriptor = match stream {
        Stream::Stdout => 1,
        Stream::Stderr => 2,
    };
    let mut offset = 0;
    while offset < byte_length {
        let written = write(
            descriptor,
            bytes.add(offset).cast::<c_void>(),
            byte_length - offset,
        );
        if written > 0 {
            offset += written as usize;
            continue;
        }
        if written < 0 && last_errno() == EINTR {
            continue;
        }
        return false;
    }
    true
}

#[cfg(windows)]
unsafe fn write_stream(stream: Stream, bytes: *const u8, byte_length: usize) -> bool {
    let standard_handle = match stream {
        Stream::Stdout => STD_OUTPUT_HANDLE,
        Stream::Stderr => STD_ERROR_HANDLE,
    };
    let handle = GetStdHandle(standard_handle);
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return false;
    }

    let mut offset = 0;
    while offset < byte_length {
        let request = core::cmp::min(byte_length - offset, u32::MAX as usize) as u32;
        let mut written = 0_u32;
        let succeeded = WriteFile(
            handle,
            bytes.add(offset).cast::<c_void>(),
            request,
            &mut written,
            ptr::null_mut(),
        );
        if succeeded == 0 || written == 0 {
            return false;
        }
        offset += written as usize;
    }
    true
}

#[cfg(not(any(unix, windows)))]
unsafe fn write_stream(_stream: Stream, _bytes: *const u8, _byte_length: usize) -> bool {
    false
}

#[cfg(unix)]
unsafe fn exit_process(status: i32) -> ! {
    _exit(status)
}

#[cfg(windows)]
unsafe fn exit_process(status: i32) -> ! {
    ExitProcess(status as u32)
}

#[cfg(not(any(unix, windows)))]
unsafe fn exit_process(_status: i32) -> ! {
    loop {
        core::hint::spin_loop();
    }
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
    fn write(descriptor: i32, bytes: *const c_void, byte_length: usize) -> isize;
    fn _exit(status: i32) -> !;
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
const STD_OUTPUT_HANDLE: u32 = -11_i32 as u32;
#[cfg(windows)]
const STD_ERROR_HANDLE: u32 = -12_i32 as u32;
#[cfg(windows)]
const INVALID_HANDLE_VALUE: *mut c_void = -1_isize as *mut c_void;

#[cfg(windows)]
extern "system" {
    fn GetStdHandle(standard_handle: u32) -> *mut c_void;
    fn WriteFile(
        handle: *mut c_void,
        bytes: *const c_void,
        byte_length: u32,
        written: *mut u32,
        overlapped: *mut c_void,
    ) -> i32;
    fn ExitProcess(status: u32) -> !;
}

#[cfg(all(not(test), panic = "abort"))]
#[panic_handler]
fn rust_panic(_information: &core::panic::PanicInfo<'_>) -> ! {
    unsafe { exit_process(PANIC_STATUS) }
}

#[cfg(all(not(test), panic = "abort"))]
#[no_mangle]
pub extern "C" fn rust_eh_personality() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stack_frame_layout_is_three_pointer_words() {
        assert_eq!(
            core::mem::size_of::<DrStackFrameV1>(),
            3 * core::mem::size_of::<usize>()
        );
        assert_eq!(
            core::mem::align_of::<DrStackFrameV1>(),
            core::mem::align_of::<usize>()
        );
    }
}
