use core::ptr;

use crate::device_io::{self, StandardStream};

const INITIAL_CAPACITY: usize = 4096;

static mut BUFFER: *mut u8 = ptr::null_mut();
static mut CAPACITY: usize = 0;
static mut START: usize = 0;
static mut END: usize = 0;
static mut EOF: bool = false;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadLineError {
    Read,
    InvalidUtf8,
    Allocation,
}

pub(crate) struct BufferedInput {
    pub bytes: *const u8,
    pub length: usize,
    pub eof: bool,
}

/// Transfers unread line-discipline bytes back to the shared stdin consumer.
///
/// The returned bytes remain owned by this module and must be copied before another line read.
pub(crate) unsafe fn take_buffered_input() -> BufferedInput {
    let length = END - START;
    let bytes = if length == 0 {
        ptr::null()
    } else {
        BUFFER.add(START)
    };
    START = END;
    BufferedInput {
        bytes,
        length,
        eof: EOF,
    }
}

/// Reads one UTF-8 line above the raw standard-device layer.
///
/// `Ok(None)` is EOF before bytes. A returned slice remains valid only until the next call.
pub(crate) unsafe fn read_line() -> Result<Option<(*const u8, usize)>, ReadLineError> {
    ensure_capacity(INITIAL_CAPACITY)?;
    loop {
        if let Some(newline) = find_newline() {
            let line_start = START;
            let raw = core::slice::from_raw_parts(BUFFER.add(line_start), newline + 1 - line_start);
            let line_length = strip_line_ending(raw).len();
            START = newline + 1;
            validate_utf8(BUFFER.add(line_start), line_length)?;
            return Ok(Some((BUFFER.add(line_start), line_length)));
        }

        if EOF {
            if START == END {
                return Ok(None);
            }
            let line_start = START;
            let line_length = END - START;
            START = END;
            validate_utf8(BUFFER.add(line_start), line_length)?;
            return Ok(Some((BUFFER.add(line_start), line_length)));
        }

        prepare_write_space()?;
        let read = device_io::read(StandardStream::Stdin, BUFFER.add(END), CAPACITY - END)
            .map_err(|()| ReadLineError::Read)?;
        if read == 0 {
            EOF = true;
        } else {
            END += read;
        }
    }
}

fn strip_line_ending(bytes: &[u8]) -> &[u8] {
    let Some(without_lf) = bytes.strip_suffix(b"\n") else {
        return bytes;
    };
    without_lf.strip_suffix(b"\r").unwrap_or(without_lf)
}

unsafe fn find_newline() -> Option<usize> {
    let mut index = START;
    while index < END {
        if *BUFFER.add(index) == b'\n' {
            return Some(index);
        }
        index += 1;
    }
    None
}

unsafe fn validate_utf8(bytes: *const u8, length: usize) -> Result<(), ReadLineError> {
    let bytes = core::slice::from_raw_parts(bytes, length);
    core::str::from_utf8(bytes)
        .map(|_| ())
        .map_err(|_| ReadLineError::InvalidUtf8)
}

unsafe fn prepare_write_space() -> Result<(), ReadLineError> {
    if END < CAPACITY {
        return Ok(());
    }
    if START != 0 {
        let remaining = END - START;
        ptr::copy(BUFFER.add(START), BUFFER, remaining);
        START = 0;
        END = remaining;
        return Ok(());
    }
    let next = CAPACITY.checked_mul(2).ok_or(ReadLineError::Allocation)?;
    ensure_capacity(next)
}

unsafe fn ensure_capacity(required: usize) -> Result<(), ReadLineError> {
    if CAPACITY >= required {
        return Ok(());
    }
    let replacement = super::allocate(required);
    if replacement.is_null() {
        return Err(ReadLineError::Allocation);
    }
    if !BUFFER.is_null() {
        ptr::copy_nonoverlapping(BUFFER.add(START), replacement, END - START);
        END -= START;
        START = 0;
        super::deallocate(BUFFER);
    }
    BUFFER = replacement;
    CAPACITY = required;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::strip_line_ending;

    #[test]
    fn line_discipline_strips_only_lf_and_crlf() {
        for (input, expected) in [
            (b"alpha\n".as_slice(), b"alpha".as_slice()),
            (b"alpha\r\n".as_slice(), b"alpha".as_slice()),
            (b"\n".as_slice(), b"".as_slice()),
            (b"final".as_slice(), b"final".as_slice()),
            (b"space \t\r".as_slice(), b"space \t\r".as_slice()),
            (b"a\0b\n".as_slice(), b"a\0b".as_slice()),
            ("Dória 🎮\n".as_bytes(), "Dória 🎮".as_bytes()),
        ] {
            assert_eq!(strip_line_ending(input), expected);
        }
    }
}
