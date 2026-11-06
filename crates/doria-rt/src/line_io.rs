#[cfg(test)]
use core::ptr;

use crate::device_io::{self, BufferedInput, InputBuffer, StandardStream};
use crate::utf8_stream::Utf8Validator;

const INITIAL_CAPACITY: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReadLineError {
    Read(crate::platform_io::Failure),
    InvalidUtf8 {
        valid_byte_count: usize,
        invalid_byte_count: Option<usize>,
    },
    Allocation,
}

/// Transfers unread bytes from the shared standard-input owner.
///
/// The returned bytes remain device-owned and must be copied before any other stdin read.
pub(crate) unsafe fn take_buffered_input(
) -> Result<(device_io::DeviceLoan<'static>, BufferedInput), crate::platform_io::Failure> {
    let loan = device_io::standard_device(StandardStream::Stdin).try_borrow_for_io()?;
    let input = (&mut *device_io::stdin_input()).take_buffered_input();
    Ok((loan, input))
}

/// Reads one UTF-8 line above the raw standard-device layer.
///
/// The device loan remains live until the caller has copied the borrowed bytes.
pub(crate) struct BorrowedLine {
    pub(crate) bytes: Option<(*const u8, usize)>,
    _loan: device_io::DeviceLoan<'static>,
}

pub(crate) unsafe fn read_line() -> Result<BorrowedLine, ReadLineError> {
    let loan = device_io::standard_device(StandardStream::Stdin)
        .try_borrow_for_io()
        .map_err(ReadLineError::Read)?;
    let line = read_line_with(&mut *device_io::stdin_input(), |destination, capacity| {
        device_io::read_text(StandardStream::Stdin, destination, capacity)
    })?;
    Ok(BorrowedLine {
        bytes: line,
        _loan: loan,
    })
}

pub(crate) unsafe fn read_line_with(
    input: &mut InputBuffer,
    mut transfer: impl FnMut(*mut u8, usize) -> Result<usize, crate::platform_io::Failure>,
) -> Result<Option<(*const u8, usize)>, ReadLineError> {
    if input.eof && input.start == input.end {
        input.release();
        return Ok(None);
    }
    input
        .ensure_capacity(INITIAL_CAPACITY)
        .map_err(|_| ReadLineError::Allocation)?;
    let mut utf8 = Utf8Validator::default();
    let mut validated = 0;
    let mut transferred = 0_usize;
    loop {
        if let Some(newline) = find_newline(input) {
            let line_start = input.start;
            let raw =
                core::slice::from_raw_parts(input.bytes.add(line_start), newline + 1 - line_start);
            let line_length = strip_line_ending(raw).len();
            validate_prefix(&mut utf8, &mut validated, &raw[..line_length]);
            input.start = newline + 1;
            finish_utf8(utf8)?;
            return Ok(Some((input.bytes.add(line_start), line_length)));
        }

        if input.eof {
            let line_start = input.start;
            let line_length = input.end - input.start;
            if line_length == 0 {
                input.release();
                return Ok(None);
            }
            validate_prefix(
                &mut utf8,
                &mut validated,
                core::slice::from_raw_parts(input.bytes.add(line_start), line_length),
            );
            input.start = input.end;
            finish_utf8(utf8)?;
            return Ok(Some((input.bytes.add(line_start), line_length)));
        }

        // A trailing CR might belong to CRLF and must not affect UTF-8 errors
        // until the next chunk tells us whether it is part of the line.
        let available =
            core::slice::from_raw_parts(input.bytes.add(input.start), input.end - input.start);
        validate_prefix(
            &mut utf8,
            &mut validated,
            available.strip_suffix(b"\r").unwrap_or(available),
        );
        input
            .prepare_write_space(device_io::TEXT_READ_MINIMUM)
            .map_err(|_| ReadLineError::Allocation)?;
        let remaining = input.capacity - input.end;
        match transfer(input.bytes.add(input.end), remaining) {
            Ok(0) => input.eof = true,
            Ok(read) if read <= remaining => {
                input.end += read;
                transferred = transferred
                    .checked_add(read)
                    .ok_or(ReadLineError::Allocation)?;
            }
            Ok(_) => return Err(ReadLineError::Read(crate::platform_io::Failure::other())),
            Err(failure) => {
                let failure = match failure.transferred_byte_count {
                    Some(count) if count <= remaining => {
                        // The failed read may have filled a prefix. Keep it for
                        // the next invocation; start has not consumed this line.
                        input.end += count;
                        failure.after_transferred(transferred)
                    }
                    Some(_) => failure.with_transferred_byte_count(None),
                    None => failure,
                };
                return Err(ReadLineError::Read(failure));
            }
        }
    }
}

fn strip_line_ending(bytes: &[u8]) -> &[u8] {
    let Some(without_lf) = bytes.strip_suffix(b"\n") else {
        return bytes;
    };
    without_lf.strip_suffix(b"\r").unwrap_or(without_lf)
}

unsafe fn find_newline(input: &mut InputBuffer) -> Option<usize> {
    scan_newline(
        core::slice::from_raw_parts(input.bytes, input.end),
        &mut input.scanned,
    )
}

fn scan_newline(bytes: &[u8], scanned: &mut usize) -> Option<usize> {
    let newline = bytes[*scanned..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map(|offset| *scanned + offset);
    *scanned = newline.map_or(bytes.len(), |index| index + 1);
    newline
}

fn validate_prefix(utf8: &mut Utf8Validator, validated: &mut usize, bytes: &[u8]) {
    utf8.push(&bytes[*validated..]);
    *validated = bytes.len();
}

fn finish_utf8(utf8: Utf8Validator) -> Result<(), ReadLineError> {
    utf8.finish().map_err(|error| ReadLineError::InvalidUtf8 {
        valid_byte_count: error.valid_byte_count,
        invalid_byte_count: error.invalid_byte_count,
    })
}

#[cfg(test)]
mod tests {
    use super::{finish_utf8, scan_newline, strip_line_ending, validate_prefix, ReadLineError};
    use crate::utf8_stream::Utf8Validator;

    #[test]
    fn failed_line_reads_count_only_fresh_transfers_and_retain_partial_bytes_for_retry() {
        use super::*;
        let failure = crate::platform_io::Failure::from_system_code(13);
        unsafe {
            let mut input = InputBuffer::new();
            input.ensure_capacity(INITIAL_CAPACITY).unwrap();
            ptr::copy_nonoverlapping(b"old".as_ptr(), input.bytes, 3);
            input.end = 3;
            let mut calls = 0;
            let result = read_line_with(&mut input, |destination, _| {
                calls += 1;
                if calls == 1 {
                    ptr::copy_nonoverlapping(b"new".as_ptr(), destination, 3);
                    Ok(3)
                } else {
                    ptr::copy_nonoverlapping(b"!!".as_ptr(), destination, 2);
                    Err(failure.with_transferred_byte_count(Some(2)))
                }
            });
            assert_eq!(
                result,
                Err(ReadLineError::Read(
                    failure.with_transferred_byte_count(Some(5))
                ))
            );
            assert_eq!(calls, 2);
            assert_eq!(
                core::slice::from_raw_parts(input.bytes, input.end),
                b"oldnew!!"
            );
            assert_eq!(
                read_line_with(&mut input, |_, _| Err(
                    failure.with_transferred_byte_count(Some(0))
                )),
                Err(ReadLineError::Read(
                    failure.with_transferred_byte_count(Some(0))
                ))
            );
            calls = 0;
            assert_eq!(
                read_line_with(&mut input, |destination, _| {
                    calls += 1;
                    if calls == 1 {
                        ptr::copy_nonoverlapping(b"more".as_ptr(), destination, 4);
                        Ok(4)
                    } else {
                        Err(failure)
                    }
                }),
                Err(ReadLineError::Read(failure))
            );
            assert_eq!(
                core::slice::from_raw_parts(input.bytes, input.end),
                b"oldnew!!more"
            );
            let (line, length) = read_line_with(&mut input, |destination, _| {
                ptr::copy_nonoverlapping(b"\nnext".as_ptr(), destination, 5);
                Ok(5)
            })
            .unwrap()
            .unwrap();
            assert_eq!(core::slice::from_raw_parts(line, length), b"oldnew!!more");
            let pending = input.take_buffered_input();
            assert_eq!(
                core::slice::from_raw_parts(pending.bytes, pending.length),
                b"next"
            );
        }
    }

    #[test]
    fn chunked_validation_excludes_crlf_but_preserves_a_lone_cr() {
        for line in [
            b"valid\r\n".as_slice(),
            b"\xe2\x82\xac\r\n",
            b"prefix\xe2\r\n",
            b"prefix\xe2\x82\r\n",
            b"prefix\xe2\r",
            b"prefix\xf0\x90\r\n",
            b"prefix\xffmore\r\n",
            b"prefix\r\r\n",
        ] {
            let expected = core::str::from_utf8(strip_line_ending(line))
                .map(|_| ())
                .map_err(|error| ReadLineError::InvalidUtf8 {
                    valid_byte_count: error.valid_up_to(),
                    invalid_byte_count: error.error_len(),
                });
            for split in 0..line.len() {
                let mut utf8 = Utf8Validator::default();
                let mut validated = 0;
                let prefix = &line[..split];
                validate_prefix(
                    &mut utf8,
                    &mut validated,
                    prefix.strip_suffix(b"\r").unwrap_or(prefix),
                );
                validate_prefix(&mut utf8, &mut validated, strip_line_ending(line));
                assert_eq!(finish_utf8(utf8), expected, "{line:?}, split {split}");
            }
        }
    }

    #[test]
    fn line_scanning_resumes_after_each_chunk_and_delimiter() {
        let mut scanned = 0;
        for chunk in [b"a".as_slice(), b"abc", b"abcdef"] {
            assert_eq!(scan_newline(chunk, &mut scanned), None);
            assert_eq!(scanned, chunk.len());
        }
        assert_eq!(scan_newline(b"abcdef\nsecond\nlast", &mut scanned), Some(6));
        assert_eq!(scanned, 7);
        assert_eq!(
            scan_newline(b"abcdef\nsecond\nlast", &mut scanned),
            Some(13)
        );
        assert_eq!(scanned, 14);
        assert_eq!(scan_newline(b"abcdef\nsecond\nlast", &mut scanned), None);
        assert_eq!(scanned, 18);

        // Compaction preserves the scan position relative to the remaining bytes.
        scanned -= 14;
        assert_eq!(scan_newline(b"last\r\nnext", &mut scanned), Some(5));
        assert_eq!(scanned, 6);
        assert_eq!(scan_newline(b"last\r\nnext", &mut scanned), None);
        assert_eq!(scanned, 10);
    }

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
