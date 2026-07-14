#![cfg_attr(all(not(test), panic = "abort"), no_std)]

// Linked runtime artifacts always use panic=abort; unwind-mode builds exist only for check/test metadata.

use core::ffi::c_void;
use core::mem;
use core::ptr;

mod device_io;
mod file_io;
mod line_io;

use device_io::StandardStream;

const PANIC_STATUS: i32 = 101;
#[cfg(unix)]
const SIGPIPE: i32 = 13;
#[cfg(unix)]
const SIG_IGN: usize = 1;

#[repr(C)]
pub struct DrStackFrameV1 {
    pub parent: *const DrStackFrameV1,
    pub function_name: *const u8,
    pub function_name_length: usize,
}

/// Opaque outside doria-rt. Bytes immediately follow this header.
#[repr(C)]
pub struct DrStringV1 {
    references: usize,
    byte_length: usize,
}

const STRING_HEADER_SIZE: usize = mem::size_of::<DrStringV1>();

pub type DrMainIntV1 = unsafe extern "C" fn(*const DrStackFrameV1) -> i64;
pub type DrMainVoidV1 = unsafe extern "C" fn(*const DrStackFrameV1);

/// Allocates a headerless native class payload.
///
/// This is a private, versioned compiler/runtime ABI. `byte_alignment` is
/// currently bounded by the platform allocator alignment because every Stage
/// 19 property is at most pointer/f64 aligned. Empty classes receive a unique,
/// freeable one-byte allocation. Allocation failure panics with status 101.
///
/// # Safety
///
/// `current_frame` must be null or a valid generated frame chain. The returned
/// pointer must be released exactly once with `dr_v1_class_free`.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_class_allocate(
    current_frame: *const DrStackFrameV1,
    byte_length: usize,
    byte_alignment: usize,
) -> *mut u8 {
    let supported_alignment = mem::align_of::<u64>().max(mem::align_of::<usize>());
    if byte_alignment == 0
        || !byte_alignment.is_power_of_two()
        || byte_alignment > supported_alignment
    {
        static MESSAGE: &[u8] = b"class allocation failed";
        dr_v1_panic(current_frame, MESSAGE.as_ptr(), MESSAGE.len());
    }
    let payload = allocate(byte_length.max(1));
    if payload.is_null() {
        static MESSAGE: &[u8] = b"class allocation failed";
        dr_v1_panic(current_frame, MESSAGE.as_ptr(), MESSAGE.len());
    }
    payload
}

/// Frees a payload returned by `dr_v1_class_allocate`.
///
/// # Safety
///
/// `payload` must be null or a live class payload allocated by the matching
/// runtime. A live payload may be passed exactly once.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_class_free(payload: *mut u8) {
    if !payload.is_null() {
        deallocate(payload);
    }
}

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
    #[cfg(unix)]
    ignore_sigpipe();

    if device_io::write(StandardStream::Stdout, bytes, byte_length) {
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
    if !device_io::write(StandardStream::Stderr, bytes, byte_length) {
        exit_process(PANIC_STATUS);
    }
}

/// Flushes stdout through the implementation-private standard-device abstraction.
///
/// # Safety
/// `current_frame` must be null or a valid frame chain for panic reporting.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_flush_stdout(current_frame: *const DrStackFrameV1) {
    if !device_io::flush(StandardStream::Stdout) {
        static MESSAGE: &[u8] = b"failed to flush stdout";
        dr_v1_panic(current_frame, MESSAGE.as_ptr(), MESSAGE.len());
    }
}

/// Flushes stderr through the implementation-private standard-device abstraction.
///
/// # Safety
/// `current_frame` must be null or a valid frame chain for panic reporting.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_flush_stderr(current_frame: *const DrStackFrameV1) {
    if !device_io::flush(StandardStream::Stderr) {
        static MESSAGE: &[u8] = b"failed to flush stderr";
        dr_v1_panic(current_frame, MESSAGE.as_ptr(), MESSAGE.len());
    }
}

/// Returns whether one standard stream is attached to an interactive terminal.
///
/// Stream identifiers are 0=stdin, 1=stdout, and 2=stderr. Unknown identifiers return false.
///
/// # Safety
/// This operation has no pointer preconditions.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_stream_is_interactive(stream: u8) -> u8 {
    let stream = match stream {
        0 => StandardStream::Stdin,
        1 => StandardStream::Stdout,
        2 => StandardStream::Stderr,
        _ => return 0,
    };
    u8::from(device_io::is_interactive(stream))
}

/// Reads one UTF-8 line from stdin, returning null only for EOF before bytes.
///
/// The returned non-null string is owned. LF and CRLF endings are removed.
///
/// # Safety
/// `current_frame` must be null or a valid frame chain for panic reporting.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_read_stdin_line(
    current_frame: *const DrStackFrameV1,
) -> *mut DrStringV1 {
    match line_io::read_line() {
        Ok(Some((bytes, length))) => dr_v1_string_from_utf8(bytes, length),
        Ok(None) => ptr::null_mut(),
        Err(line_io::ReadLineError::InvalidUtf8) => {
            static MESSAGE: &[u8] = b"stdin contained invalid UTF-8";
            dr_v1_panic(current_frame, MESSAGE.as_ptr(), MESSAGE.len())
        }
        Err(line_io::ReadLineError::Read) => {
            static MESSAGE: &[u8] = b"failed to read stdin";
            dr_v1_panic(current_frame, MESSAGE.as_ptr(), MESSAGE.len())
        }
        Err(line_io::ReadLineError::Allocation) => {
            static MESSAGE: &[u8] = b"string allocation failed";
            dr_v1_panic(current_frame, MESSAGE.as_ptr(), MESSAGE.len())
        }
    }
}

/// Reads a complete UTF-8 text file into an owned runtime string.
///
/// # Safety
/// `path` must identify a live runtime string and `current_frame` must be null or valid.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_read_file(
    current_frame: *const DrStackFrameV1,
    path: *const DrStringV1,
) -> *mut DrStringV1 {
    let path = core::slice::from_raw_parts(string_bytes(path), (*path).byte_length);
    match file_io::read_file(path) {
        Ok(contents) => {
            let bytes = core::slice::from_raw_parts(contents.bytes, contents.length);
            if core::str::from_utf8(bytes).is_err() {
                static MESSAGE: &[u8] = b"file contained invalid UTF-8";
                dr_v1_panic(current_frame, MESSAGE.as_ptr(), MESSAGE.len());
            }
            dr_v1_string_from_utf8(bytes.as_ptr(), bytes.len())
        }
        Err(file_io::FileError::PathNul) => {
            static MESSAGE: &[u8] = b"file path contained an embedded NUL";
            dr_v1_panic(current_frame, MESSAGE.as_ptr(), MESSAGE.len())
        }
        Err(file_io::FileError::Allocation) => {
            static MESSAGE: &[u8] = b"string allocation failed";
            dr_v1_panic(current_frame, MESSAGE.as_ptr(), MESSAGE.len())
        }
        Err(_) => {
            static MESSAGE: &[u8] = b"failed to read file";
            dr_v1_panic(current_frame, MESSAGE.as_ptr(), MESSAGE.len())
        }
    }
}

/// Creates or truncates a text file and writes exact runtime-string bytes.
///
/// # Safety
/// Both strings must be live and `current_frame` must be null or valid.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_write_file(
    current_frame: *const DrStackFrameV1,
    path: *const DrStringV1,
    contents: *const DrStringV1,
) {
    let path = core::slice::from_raw_parts(string_bytes(path), (*path).byte_length);
    let contents = core::slice::from_raw_parts(string_bytes(contents), (*contents).byte_length);
    match file_io::write_file(path, contents) {
        Ok(()) => {}
        Err(file_io::FileError::PathNul) => {
            static MESSAGE: &[u8] = b"file path contained an embedded NUL";
            dr_v1_panic(current_frame, MESSAGE.as_ptr(), MESSAGE.len())
        }
        Err(file_io::FileError::Allocation) => {
            static MESSAGE: &[u8] = b"string allocation failed";
            dr_v1_panic(current_frame, MESSAGE.as_ptr(), MESSAGE.len())
        }
        Err(_) => {
            static MESSAGE: &[u8] = b"failed to write file";
            dr_v1_panic(current_frame, MESSAGE.as_ptr(), MESSAGE.len())
        }
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
    write_panic_fragment(b"Panic: ");
    write_panic_bytes(message, message_length);
    write_panic_fragment(b"\nStack Trace:\n");

    let mut frame = current_frame;
    while !frame.is_null() {
        write_panic_fragment(b"  at ");
        write_panic_bytes((*frame).function_name, (*frame).function_name_length);
        write_panic_fragment(b"\n");
        frame = (*frame).parent;
    }
    exit_process(PANIC_STATUS)
}

/// Allocates an immutable runtime string from an explicit byte range.
///
/// # Safety
/// `bytes` must be readable for `byte_length` bytes and contain valid UTF-8.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_string_from_utf8(
    bytes: *const u8,
    byte_length: usize,
) -> *mut DrStringV1 {
    let string = allocate_string(byte_length);
    if byte_length != 0 {
        ptr::copy_nonoverlapping(bytes, string_bytes_mut(string), byte_length);
    }
    string
}

unsafe fn allocate_string(byte_length: usize) -> *mut DrStringV1 {
    let total = STRING_HEADER_SIZE
        .checked_add(byte_length)
        .unwrap_or_else(|| string_runtime_panic(b"string length overflow"));
    let string = allocate(total).cast::<DrStringV1>();
    if string.is_null() {
        string_runtime_panic(b"string allocation failed");
    }
    ptr::write(
        string,
        DrStringV1 {
            references: 1,
            byte_length,
        },
    );
    string
}

/// Retains one owned reference.
///
/// # Safety
/// `string` must be null or a live doria-rt string.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_string_retain(string: *mut DrStringV1) -> *mut DrStringV1 {
    if !string.is_null() {
        (*string).references = (*string)
            .references
            .checked_add(1)
            .unwrap_or_else(|| string_runtime_panic(b"string reference count overflow"));
    }
    string
}

/// Releases one owned reference and frees the final reference.
///
/// # Safety
/// `string` must be null or a live owned doria-rt string reference.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_string_release(string: *mut DrStringV1) {
    if string.is_null() {
        return;
    }
    let references = (*string).references;
    if references == 0 {
        string_runtime_panic(b"string reference count underflow");
    }
    if references == 1 {
        deallocate(string.cast::<u8>());
    } else {
        (*string).references = references - 1;
    }
}

/// Concatenates two borrowed strings into a new owned string.
///
/// # Safety
/// Both pointers must identify live doria-rt strings.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_string_concat(
    left: *const DrStringV1,
    right: *const DrStringV1,
) -> *mut DrStringV1 {
    let length = (*left)
        .byte_length
        .checked_add((*right).byte_length)
        .unwrap_or_else(|| string_runtime_panic(b"string length overflow"));
    let result = allocate_string(length);
    ptr::copy_nonoverlapping(
        string_bytes(left),
        string_bytes_mut(result),
        (*left).byte_length,
    );
    ptr::copy_nonoverlapping(
        string_bytes(right),
        string_bytes_mut(result).add((*left).byte_length),
        (*right).byte_length,
    );
    result
}

/// Returns -1, 0, or 1 using unsigned byte-lexicographic ordering.
///
/// # Safety
/// Both pointers must identify live doria-rt strings.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_string_compare(
    left: *const DrStringV1,
    right: *const DrStringV1,
) -> i32 {
    let common = core::cmp::min((*left).byte_length, (*right).byte_length);
    for index in 0..common {
        let left_byte = *string_bytes(left).add(index);
        let right_byte = *string_bytes(right).add(index);
        if left_byte < right_byte {
            return -1;
        }
        if left_byte > right_byte {
            return 1;
        }
    }
    match (*left).byte_length.cmp(&(*right).byte_length) {
        core::cmp::Ordering::Less => -1,
        core::cmp::Ordering::Equal => 0,
        core::cmp::Ordering::Greater => 1,
    }
}

/// Compares two nullable runtime strings for value equality.
///
/// # Safety
/// Each pointer must be null or identify a live doria-rt string.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_nullable_string_equal(
    left: *const DrStringV1,
    right: *const DrStringV1,
) -> u8 {
    if left.is_null() || right.is_null() {
        return u8::from(left == right);
    }
    u8::from(dr_v1_string_compare(left, right) == 0)
}

#[no_mangle]
/// Returns the explicit byte pointer for a live string.
///
/// # Safety
/// `string` must identify a live doria-rt string for the duration of byte access.
pub unsafe extern "C" fn dr_v1_string_data(string: *const DrStringV1) -> *const u8 {
    string_bytes(string)
}

#[no_mangle]
/// Returns the explicit byte length for a live string.
///
/// # Safety
/// `string` must identify a live doria-rt string.
pub unsafe extern "C" fn dr_v1_string_length(string: *const DrStringV1) -> usize {
    (*string).byte_length
}

#[no_mangle]
/// Writes a borrowed string to stdout without adding a newline.
///
/// # Safety
/// `string` must identify a live doria-rt string and `current_frame` must be null or a valid frame chain.
pub unsafe extern "C" fn dr_v1_write_string_stdout(
    current_frame: *const DrStackFrameV1,
    string: *const DrStringV1,
) {
    dr_v1_write_stdout(current_frame, string_bytes(string), (*string).byte_length)
}

/// Writes a borrowed string to stderr without adding a newline.
///
/// # Safety
/// `string` must identify a live runtime string and `current_frame` must be null or valid.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_write_string_stderr(
    current_frame: *const DrStackFrameV1,
    string: *const DrStringV1,
) {
    if !device_io::write(
        StandardStream::Stderr,
        string_bytes(string),
        (*string).byte_length,
    ) {
        static MESSAGE: &[u8] = b"failed to write stderr";
        dr_v1_panic(current_frame, MESSAGE.as_ptr(), MESSAGE.len());
    }
}

#[no_mangle]
/// Creates an owned string containing canonical signed decimal display text.
///
/// # Safety
/// The returned owned reference must eventually be released on a normal execution path.
pub unsafe extern "C" fn dr_v1_string_from_i64(value: i64) -> *mut DrStringV1 {
    let mut buffer = [0_u8; 20];
    let (start, length) = signed_decimal(value, &mut buffer);
    dr_v1_string_from_utf8(buffer.as_ptr().add(start), length)
}

#[no_mangle]
/// Creates an owned string containing canonical unsigned decimal display text.
///
/// # Safety
/// The returned owned reference must eventually be released on a normal execution path.
pub unsafe extern "C" fn dr_v1_string_from_u64(value: u64) -> *mut DrStringV1 {
    let mut buffer = [0_u8; 20];
    let (start, length) = unsigned_decimal(value, &mut buffer);
    dr_v1_string_from_utf8(buffer.as_ptr().add(start), length)
}

#[no_mangle]
/// Creates an owned string containing canonical binary32 display text.
///
/// # Safety
/// The returned owned reference must eventually be released on a normal execution path.
pub unsafe extern "C" fn dr_v1_string_from_f32(value: f32) -> *mut DrStringV1 {
    float_string_f32(value)
}

#[no_mangle]
/// Creates an owned string containing canonical binary64 display text.
///
/// # Safety
/// The returned owned reference must eventually be released on a normal execution path.
pub unsafe extern "C" fn dr_v1_string_from_f64(value: f64) -> *mut DrStringV1 {
    float_string_f64(value)
}

#[no_mangle]
/// Creates an owned string containing `true` or `false`.
///
/// # Safety
/// The returned owned reference must eventually be released on a normal execution path.
pub unsafe extern "C" fn dr_v1_string_from_bool(value: u8) -> *mut DrStringV1 {
    let bytes: &[u8] = if value == 0 { b"false" } else { b"true" };
    dr_v1_string_from_utf8(bytes.as_ptr(), bytes.len())
}

const FORMAT_LEFT_ALIGN: u8 = 1;
const FORMAT_ZERO_PAD: u8 = 2;
const FORMAT_DECIMAL: u8 = 1;
const FORMAT_HEX_LOWER: u8 = 2;
const FORMAT_HEX_UPPER: u8 = 3;
const FORMAT_OCTAL: u8 = 4;
const FORMAT_BINARY: u8 = 5;

/// Applies Stage 17 byte-counted width to a borrowed string.
///
/// # Safety
/// `value` must identify a live runtime string.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_format_string(
    value: *const DrStringV1,
    width: u32,
    flags: u8,
) -> *mut DrStringV1 {
    padded_string(
        string_bytes(value),
        (*value).byte_length,
        width,
        flags,
        false,
    )
}

/// Formats a signed integer using a validated Stage 17 integer conversion.
///
/// # Safety
/// `conversion`, `bit_width`, and flags must come from validated MIR.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_format_i64(
    value: i64,
    bit_width: u8,
    conversion: u8,
    width: u32,
    flags: u8,
) -> *mut DrStringV1 {
    let mut buffer = [0_u8; 65];
    let (start, length) = if conversion == FORMAT_DECIMAL {
        signed_decimal(
            value,
            (&mut buffer[..20]).try_into().expect("20-byte prefix"),
        )
    } else {
        let mask = if bit_width == 64 {
            u64::MAX
        } else {
            (1_u64 << bit_width) - 1
        };
        unsigned_base((value as u64) & mask, conversion, &mut buffer)
    };
    padded_string(
        buffer.as_ptr().add(start),
        length,
        width,
        flags,
        value < 0 && conversion == FORMAT_DECIMAL,
    )
}

/// Formats an unsigned integer using a validated Stage 17 integer conversion.
///
/// # Safety
/// `conversion` and flags must come from validated MIR.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_format_u64(
    value: u64,
    conversion: u8,
    width: u32,
    flags: u8,
) -> *mut DrStringV1 {
    let mut buffer = [0_u8; 65];
    let (start, length) = if conversion == FORMAT_DECIMAL {
        let mut decimal = [0_u8; 20];
        let result = unsigned_decimal(value, &mut decimal);
        let start = buffer.len() - result.1;
        buffer[start..].copy_from_slice(&decimal[result.0..]);
        (start, result.1)
    } else {
        unsigned_base(value, conversion, &mut buffer)
    };
    padded_string(buffer.as_ptr().add(start), length, width, flags, false)
}

/// Formats a binary32 value with validated fixed precision and width.
///
/// # Safety
/// Precision and flags must come from validated MIR.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_format_f32(
    value: f32,
    precision: u32,
    width: u32,
    flags: u8,
) -> *mut DrStringV1 {
    format_fixed_float(
        value as f64,
        value.is_sign_negative(),
        precision,
        width,
        flags,
    )
}

/// Formats a binary64 value with validated fixed precision and width.
///
/// # Safety
/// Precision and flags must come from validated MIR.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_format_f64(
    value: f64,
    precision: u32,
    width: u32,
    flags: u8,
) -> *mut DrStringV1 {
    format_fixed_float(value, value.is_sign_negative(), precision, width, flags)
}

unsafe fn format_fixed_float(
    value: f64,
    negative: bool,
    precision: u32,
    width: u32,
    flags: u8,
) -> *mut DrStringV1 {
    if value.is_nan() {
        return padded_string(b"NaN".as_ptr(), 3, width, flags, false);
    }
    if value == f64::INFINITY {
        return padded_string(b"Infinity".as_ptr(), 8, width, flags, false);
    }
    if value == f64::NEG_INFINITY {
        return padded_string(b"-Infinity".as_ptr(), 9, width, flags, true);
    }
    let precision = precision as usize;
    let mut factor = 1_u128;
    for _ in 0..precision {
        factor = factor
            .checked_mul(10)
            .unwrap_or_else(|| string_runtime_panic(b"formatted float precision is too large"));
    }
    let scaled = value.abs() * factor as f64;
    if !scaled.is_finite() || scaled > u128::MAX as f64 {
        string_runtime_panic(b"formatted float magnitude is too large");
    }
    let truncated = scaled as u128;
    let fraction = scaled - truncated as f64;
    let rounded = if fraction > 0.5 || (fraction == 0.5 && truncated & 1 == 1) {
        truncated
            .checked_add(1)
            .unwrap_or_else(|| string_runtime_panic(b"formatted float magnitude is too large"))
    } else {
        truncated
    };
    let integer = rounded / factor;
    let fractional = rounded % factor;
    let mut digits = [0_u8; 39];
    let (integer_start, integer_length) = unsigned_decimal_u128(integer, &mut digits);
    let sign_length = usize::from(negative);
    let decimal_length = usize::from(precision != 0);
    let length = sign_length
        .checked_add(integer_length)
        .and_then(|length| length.checked_add(decimal_length))
        .and_then(|length| length.checked_add(precision))
        .unwrap_or_else(|| string_runtime_panic(b"string length overflow"));
    let raw = allocate_string(length);
    let mut cursor = 0;
    if negative {
        *string_bytes_mut(raw) = b'-';
        cursor += 1;
    }
    ptr::copy_nonoverlapping(
        digits.as_ptr().add(integer_start),
        string_bytes_mut(raw).add(cursor),
        integer_length,
    );
    cursor += integer_length;
    if precision != 0 {
        *string_bytes_mut(raw).add(cursor) = b'.';
        cursor += 1;
        let mut divisor = factor / 10;
        for _ in 0..precision {
            *string_bytes_mut(raw).add(cursor) = b'0' + ((fractional / divisor) % 10) as u8;
            cursor += 1;
            divisor = core::cmp::max(divisor / 10, 1);
        }
    }
    let padded = padded_string(string_bytes(raw), length, width, flags, negative);
    dr_v1_string_release(raw);
    padded
}

unsafe fn padded_string(
    bytes: *const u8,
    length: usize,
    width: u32,
    flags: u8,
    negative_decimal: bool,
) -> *mut DrStringV1 {
    let width = width as usize;
    if width <= length {
        return dr_v1_string_from_utf8(bytes, length);
    }
    let result = allocate_string(width);
    let padding = width - length;
    let left = flags & FORMAT_LEFT_ALIGN != 0;
    let zero = flags & FORMAT_ZERO_PAD != 0 && !left;
    if left {
        ptr::copy_nonoverlapping(bytes, string_bytes_mut(result), length);
        ptr::write_bytes(string_bytes_mut(result).add(length), b' ', padding);
    } else if zero && negative_decimal {
        *string_bytes_mut(result) = b'-';
        ptr::write_bytes(string_bytes_mut(result).add(1), b'0', padding);
        ptr::copy_nonoverlapping(
            bytes.add(1),
            string_bytes_mut(result).add(1 + padding),
            length - 1,
        );
    } else {
        ptr::write_bytes(
            string_bytes_mut(result),
            if zero { b'0' } else { b' ' },
            padding,
        );
        ptr::copy_nonoverlapping(bytes, string_bytes_mut(result).add(padding), length);
    }
    result
}

fn unsigned_base(mut value: u64, conversion: u8, buffer: &mut [u8; 65]) -> (usize, usize) {
    let radix = match conversion {
        FORMAT_HEX_LOWER | FORMAT_HEX_UPPER => 16,
        FORMAT_OCTAL => 8,
        FORMAT_BINARY => 2,
        _ => 10,
    };
    let uppercase = conversion == FORMAT_HEX_UPPER;
    let mut cursor = buffer.len();
    loop {
        cursor -= 1;
        let digit = (value % radix) as u8;
        buffer[cursor] = match digit {
            0..=9 => b'0' + digit,
            _ if uppercase => b'A' + digit - 10,
            _ => b'a' + digit - 10,
        };
        value /= radix;
        if value == 0 {
            break;
        }
    }
    (cursor, buffer.len() - cursor)
}

fn unsigned_decimal_u128(mut value: u128, buffer: &mut [u8; 39]) -> (usize, usize) {
    let mut cursor = buffer.len();
    loop {
        cursor -= 1;
        buffer[cursor] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    (cursor, buffer.len() - cursor)
}

unsafe fn float_string_f32(value: f32) -> *mut DrStringV1 {
    if value.is_nan() {
        return string_from_static(b"NaN");
    }
    if value == f32::INFINITY {
        return string_from_static(b"Infinity");
    }
    if value == f32::NEG_INFINITY {
        return string_from_static(b"-Infinity");
    }
    if value == 0.0 {
        return string_from_static(if value.is_sign_negative() {
            b"-0"
        } else {
            b"0"
        });
    }
    let mut buffer = ryu::Buffer::new();
    let text = buffer.format_finite(value);
    dr_v1_string_from_utf8(text.as_ptr(), text.len())
}

unsafe fn float_string_f64(value: f64) -> *mut DrStringV1 {
    if value.is_nan() {
        return string_from_static(b"NaN");
    }
    if value == f64::INFINITY {
        return string_from_static(b"Infinity");
    }
    if value == f64::NEG_INFINITY {
        return string_from_static(b"-Infinity");
    }
    if value == 0.0 {
        return string_from_static(if value.is_sign_negative() {
            b"-0"
        } else {
            b"0"
        });
    }
    let mut buffer = ryu::Buffer::new();
    let text = buffer.format_finite(value);
    dr_v1_string_from_utf8(text.as_ptr(), text.len())
}

unsafe fn string_from_static(bytes: &[u8]) -> *mut DrStringV1 {
    dr_v1_string_from_utf8(bytes.as_ptr(), bytes.len())
}

fn signed_decimal(value: i64, buffer: &mut [u8; 20]) -> (usize, usize) {
    let negative = value < 0;
    let magnitude = value.unsigned_abs();
    let (mut start, mut length) = unsigned_decimal(magnitude, buffer);
    if negative {
        start -= 1;
        buffer[start] = b'-';
        length += 1;
    }
    (start, length)
}

fn unsigned_decimal(mut value: u64, buffer: &mut [u8; 20]) -> (usize, usize) {
    let mut cursor = buffer.len();
    loop {
        cursor -= 1;
        buffer[cursor] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    (cursor, buffer.len() - cursor)
}

unsafe fn string_bytes(string: *const DrStringV1) -> *const u8 {
    string.cast::<u8>().add(STRING_HEADER_SIZE)
}

unsafe fn string_bytes_mut(string: *mut DrStringV1) -> *mut u8 {
    string.cast::<u8>().add(STRING_HEADER_SIZE)
}

unsafe fn string_runtime_panic(message: &[u8]) -> ! {
    dr_v1_panic(ptr::null(), message.as_ptr(), message.len())
}

#[cfg(unix)]
unsafe fn allocate(byte_length: usize) -> *mut u8 {
    malloc(byte_length).cast::<u8>()
}
#[cfg(unix)]
unsafe fn deallocate(memory: *mut u8) {
    free(memory.cast::<c_void>());
}

#[cfg(windows)]
unsafe fn allocate(byte_length: usize) -> *mut u8 {
    HeapAlloc(GetProcessHeap(), 0, byte_length).cast::<u8>()
}
#[cfg(windows)]
unsafe fn deallocate(memory: *mut u8) {
    let _ = HeapFree(GetProcessHeap(), 0, memory.cast::<c_void>());
}

#[cfg(not(any(unix, windows)))]
unsafe fn allocate(_byte_length: usize) -> *mut u8 {
    ptr::null_mut()
}
#[cfg(not(any(unix, windows)))]
unsafe fn deallocate(_memory: *mut u8) {}

#[cfg(unix)]
unsafe fn ignore_sigpipe() {
    // Ignoring it makes write(2) report EPIPE instead of terminating the process by signal.
    signal(SIGPIPE, SIG_IGN);
}

unsafe fn write_panic_fragment(bytes: &[u8]) {
    write_panic_bytes(bytes.as_ptr(), bytes.len());
}

unsafe fn write_panic_bytes(bytes: *const u8, byte_length: usize) {
    if !device_io::write(StandardStream::Stderr, bytes, byte_length) {
        exit_process(PANIC_STATUS);
    }
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

#[cfg(unix)]
extern "C" {
    fn signal(signal: i32, handler: usize) -> usize;
    fn _exit(status: i32) -> !;
    fn malloc(byte_length: usize) -> *mut c_void;
    fn free(memory: *mut c_void);
}

// Doria's Windows executables deliberately do not link the C runtime. Rust and ryu still lower
// byte copies/fills and floating-point use to these MSVC support symbols, so the runtime owns the
// small subset they require.
#[cfg(windows)]
#[no_mangle]
pub static _fltused: i32 = 0;

/// Copies `count` bytes from `source` to the non-overlapping `destination`.
///
/// # Safety
///
/// `source` and `destination` must be valid for `count` bytes and must not overlap.
#[cfg(windows)]
#[no_mangle]
pub unsafe extern "C" fn memcpy(
    destination: *mut c_void,
    source: *const c_void,
    count: usize,
) -> *mut c_void {
    let destination_bytes = destination.cast::<u8>();
    let source_bytes = source.cast::<u8>();
    for index in 0..count {
        let byte = ptr::read_volatile(source_bytes.add(index));
        ptr::write_volatile(destination_bytes.add(index), byte);
    }
    destination
}

/// Copies `count` bytes from `source` to `destination`, including when they overlap.
///
/// # Safety
///
/// `source` and `destination` must be valid for `count` bytes.
#[cfg(windows)]
#[no_mangle]
pub unsafe extern "C" fn memmove(
    destination: *mut c_void,
    source: *const c_void,
    count: usize,
) -> *mut c_void {
    let destination_bytes = destination.cast::<u8>();
    let source_bytes = source.cast::<u8>();
    let destination_address = destination_bytes as usize;
    let source_address = source_bytes as usize;

    if destination_address <= source_address
        || destination_address.wrapping_sub(source_address) >= count
    {
        for index in 0..count {
            let byte = ptr::read_volatile(source_bytes.add(index));
            ptr::write_volatile(destination_bytes.add(index), byte);
        }
    } else {
        for index in (0..count).rev() {
            let byte = ptr::read_volatile(source_bytes.add(index));
            ptr::write_volatile(destination_bytes.add(index), byte);
        }
    }
    destination
}

/// Compares `count` bytes lexicographically as unsigned values.
///
/// # Safety
///
/// `left` and `right` must both be valid for reads of `count` bytes.
#[cfg(windows)]
#[no_mangle]
pub unsafe extern "C" fn memcmp(left: *const c_void, right: *const c_void, count: usize) -> i32 {
    let left = left.cast::<u8>();
    let right = right.cast::<u8>();
    for index in 0..count {
        let left_byte = ptr::read_volatile(left.add(index));
        let right_byte = ptr::read_volatile(right.add(index));
        if left_byte != right_byte {
            return i32::from(left_byte) - i32::from(right_byte);
        }
    }
    0
}

/// Lets Windows continue searching when precompiled `core` unwind metadata is inspected.
///
/// Doria's runtime is abort-only and never initiates SEH/C++ unwinding. The Rust-distributed
/// `core` archive can nevertheless reference the MSVC language-specific handler, while Doria
/// deliberately links without the CRT. Returning `ExceptionContinueSearch` preserves the
/// abort-only boundary if an unrelated structured exception reaches this metadata.
///
/// # Safety
///
/// This function may only be entered by the Windows exception dispatcher with its four native
/// dispatcher pointers. Doria code must never call it directly.
#[cfg(all(windows, target_env = "msvc"))]
#[no_mangle]
pub unsafe extern "C" fn __CxxFrameHandler3(
    _exception_record: *mut c_void,
    _establisher_frame: *mut c_void,
    _context_record: *mut c_void,
    _dispatcher_context: *mut c_void,
) -> i32 {
    const EXCEPTION_CONTINUE_SEARCH: i32 = 1;
    EXCEPTION_CONTINUE_SEARCH
}

/// Fills `count` bytes at `destination` with the low byte of `value`.
///
/// # Safety
///
/// `destination` must be valid for writes of `count` bytes.
#[cfg(windows)]
#[no_mangle]
pub unsafe extern "C" fn memset(destination: *mut c_void, value: i32, count: usize) -> *mut c_void {
    let destination_bytes = destination.cast::<u8>();
    for index in 0..count {
        ptr::write_volatile(destination_bytes.add(index), value as u8);
    }
    destination
}

#[cfg(windows)]
extern "system" {
    fn GetProcessHeap() -> *mut c_void;
    fn HeapAlloc(heap: *mut c_void, flags: u32, byte_length: usize) -> *mut c_void;
    fn HeapFree(heap: *mut c_void, flags: u32, memory: *mut c_void) -> i32;
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

    unsafe fn bytes(string: *const DrStringV1) -> &'static [u8] {
        core::slice::from_raw_parts(dr_v1_string_data(string), dr_v1_string_length(string))
    }

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

    #[test]
    fn headerless_class_allocation_handles_empty_and_nonempty_payloads() {
        unsafe {
            for size in [0, 1, 24] {
                let payload = dr_v1_class_allocate(ptr::null(), size, 8);
                assert!(!payload.is_null());
                if size > 0 {
                    ptr::write_bytes(payload, 0xa5, size);
                }
                dr_v1_class_free(payload);
            }
        }
    }

    #[test]
    fn explicit_lengths_preserve_empty_embedded_nul_and_utf8() {
        unsafe {
            for expected in [b"".as_slice(), b"a\0b".as_slice(), "Dória".as_bytes()] {
                let string = dr_v1_string_from_utf8(expected.as_ptr(), expected.len());
                assert_eq!(bytes(string), expected);
                dr_v1_string_release(string);
            }
        }
    }

    #[test]
    fn retain_release_and_concat_preserve_immutable_values() {
        unsafe {
            let left = dr_v1_string_from_utf8(b"Dor".as_ptr(), 3);
            let retained = dr_v1_string_retain(left);
            let right = dr_v1_string_from_utf8(b"ia".as_ptr(), 2);
            let joined = dr_v1_string_concat(left, right);
            assert_eq!(bytes(joined), b"Doria");
            assert_eq!(dr_v1_string_compare(left, retained), 0);
            dr_v1_string_release(left);
            dr_v1_string_release(retained);
            dr_v1_string_release(right);
            dr_v1_string_release(joined);
        }
    }

    #[test]
    fn canonical_primitive_display_is_exact() {
        unsafe {
            let cases = [
                (
                    dr_v1_string_from_i64(i64::MIN),
                    b"-9223372036854775808".as_slice(),
                ),
                (
                    dr_v1_string_from_u64(u64::MAX),
                    b"18446744073709551615".as_slice(),
                ),
                (dr_v1_string_from_bool(0), b"false".as_slice()),
                (dr_v1_string_from_bool(1), b"true".as_slice()),
                (dr_v1_string_from_f32(-0.0), b"-0".as_slice()),
                (dr_v1_string_from_f64(f64::NAN), b"NaN".as_slice()),
                (dr_v1_string_from_f64(f64::INFINITY), b"Infinity".as_slice()),
                (
                    dr_v1_string_from_f64(f64::NEG_INFINITY),
                    b"-Infinity".as_slice(),
                ),
            ];
            for (string, expected) in cases {
                assert_eq!(bytes(string), expected);
                dr_v1_string_release(string);
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn no_crt_memory_support_symbols_preserve_bytes_and_overlap() {
        unsafe {
            let source = [1_u8, 2, 3, 4];
            let mut copied = [0_u8; 4];
            memcpy(
                copied.as_mut_ptr().cast(),
                source.as_ptr().cast(),
                source.len(),
            );
            assert_eq!(copied, source);

            memset(copied.as_mut_ptr().cast(), 0xab, copied.len());
            assert_eq!(copied, [0xab; 4]);

            let mut moved = [1_u8, 2, 3, 4, 5];
            memmove(moved.as_mut_ptr().add(1).cast(), moved.as_ptr().cast(), 4);
            assert_eq!(moved, [1, 1, 2, 3, 4]);

            memmove(moved.as_mut_ptr().cast(), moved.as_ptr().add(1).cast(), 4);
            assert_eq!(moved, [1, 2, 3, 4, 4]);

            assert_eq!(memcmp(b"abc".as_ptr().cast(), b"abc".as_ptr().cast(), 3), 0);
            assert!(memcmp(b"abc".as_ptr().cast(), b"abd".as_ptr().cast(), 3) < 0);
            assert!(memcmp(b"abe".as_ptr().cast(), b"abd".as_ptr().cast(), 3) > 0);

            #[cfg(target_env = "msvc")]
            assert_eq!(
                __CxxFrameHandler3(
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                ),
                1
            );
        }
    }
}
