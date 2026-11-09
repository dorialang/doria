#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InvalidUtf8 {
    pub valid_byte_count: usize,
    pub invalid_byte_count: Option<usize>,
}

/// Validates chunks without retaining their storage or rescanning valid prefixes.
/// Only an unfinished code point crosses a chunk boundary. The first failure
/// remains available while a caller drains the rest of its record.
#[derive(Default)]
pub(crate) struct Utf8Validator {
    valid_byte_count: usize,
    pending: [u8; 4],
    pending_length: usize,
    failure: Option<InvalidUtf8>,
}

impl Utf8Validator {
    pub(crate) fn push(&mut self, mut bytes: &[u8]) {
        if self.failure.is_some() || bytes.is_empty() {
            return;
        }
        if self.pending_length != 0 {
            // from_utf8 has already accepted this as an incomplete prefix.
            let width = match self.pending[0] {
                0xc2..=0xdf => 2,
                0xe0..=0xef => 3,
                _ => 4,
            };
            let count = (width - self.pending_length).min(bytes.len());
            self.pending[self.pending_length..self.pending_length + count]
                .copy_from_slice(&bytes[..count]);
            self.pending_length += count;
            bytes = &bytes[count..];
            match core::str::from_utf8(&self.pending[..self.pending_length]) {
                Ok(_) => {
                    self.valid_byte_count += self.pending_length;
                    self.pending_length = 0;
                }
                Err(error) => {
                    if error.error_len().is_some() {
                        self.fail(error);
                    }
                    return;
                }
            }
        }
        match core::str::from_utf8(bytes) {
            Ok(_) => self.valid_byte_count += bytes.len(),
            Err(error) if error.error_len().is_some() => self.fail(error),
            Err(error) => {
                self.valid_byte_count += error.valid_up_to();
                let remaining = &bytes[error.valid_up_to()..];
                self.pending[..remaining.len()].copy_from_slice(remaining);
                self.pending_length = remaining.len();
            }
        }
    }

    fn fail(&mut self, error: core::str::Utf8Error) {
        self.failure = Some(InvalidUtf8 {
            valid_byte_count: self.valid_byte_count + error.valid_up_to(),
            invalid_byte_count: error.error_len(),
        });
    }

    pub(crate) fn finish(self) -> Result<(), InvalidUtf8> {
        match self.failure {
            Some(failure) => Err(failure),
            None if self.pending_length != 0 => Err(InvalidUtf8 {
                valid_byte_count: self.valid_byte_count,
                invalid_byte_count: None,
            }),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_partitions(bytes: &[u8]) {
        let expected = core::str::from_utf8(bytes)
            .map(|_| ())
            .map_err(|error| InvalidUtf8 {
                valid_byte_count: error.valid_up_to(),
                invalid_byte_count: error.error_len(),
            });
        for first in 0..=bytes.len() {
            for second in first..=bytes.len() {
                let mut validator = Utf8Validator::default();
                validator.push(&bytes[..first]);
                validator.push(&[]);
                validator.push(&bytes[first..second]);
                validator.push(&bytes[second..]);
                assert_eq!(validator.finish(), expected, "{bytes:?}: {first}, {second}");
            }
        }
    }

    #[test]
    fn accepts_every_split_of_valid_code_points() {
        for bytes in [
            b"".as_slice(),
            b"ascii\0\r\n",
            b"a\xc2\xa2\xe2\x82\xac\xf0\x9f\x8e\xae!",
            b"\xc2\x80\xdf\xbf\xe0\xa0\x80\xed\x9f\xbf\xf0\x90\x80\x80\xf4\x8f\xbf\xbf",
        ] {
            check_partitions(bytes);
        }
    }

    #[test]
    fn invalid_and_incomplete_sequences_preserve_exact_error_offsets() {
        for bytes in [
            b"a\x80z".as_slice(),
            b"ab\xc0\x80z",
            b"ab\xe0\x80\x80z",
            b"ab\xed\xa0\x80z",
            b"ab\xf4\x90\x80\x80z",
            b"ab\xf5\x80\x80\x80z",
            b"ab\xe2z",
            b"ab\xe2\x82z",
            b"ab\xf0\x90\x80z",
            b"ab\xc2",
            b"ab\xe2\x82",
            b"ab\xf0\x90\x80",
            b"a\xc2\xa2\xff\xe2",
        ] {
            check_partitions(bytes);
        }
    }

    #[test]
    fn chunk_state_retains_only_the_unfinished_code_point() {
        let mut validator = Utf8Validator::default();
        for _ in 0..1024 {
            validator.push(&[b'x'; 4096]);
            assert_eq!(validator.pending_length, 0);
        }
        assert_eq!(validator.valid_byte_count, 1024 * 4096);
        validator.push(b"\xf0\x9f");
        assert_eq!(validator.pending_length, 2);
        validator.push(b"\x8e\xae!");
        assert_eq!(validator.pending_length, 0);
        assert_eq!(validator.valid_byte_count, 1024 * 4096 + 5);
        assert_eq!(validator.finish(), Ok(()));
    }
}
