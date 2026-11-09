//! Shared private transfer transport, preserving platform failure facts without
//! constructing a second public error model or allocating ordinary outcomes.
use crate::byte_io::{ReadProgress, WriteProgress};
use crate::platform_io::{Failure, Reason};

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct DrIoStatusV1 {
    pub value: u64,
    pub system_code: i64,
    pub transferred_byte_count: usize,
    pub reason: u8,
    pub has_system_code: u8,
    pub has_transferred_byte_count: u8,
}

impl DrIoStatusV1 {
    /// Decode only an actual failure record. Absence is a separate transport
    /// fact, never the zeroed record's default reason. No I/O or allocation.
    pub(crate) fn failure(&self) -> Option<Failure> {
        if self.has_system_code > 1 || self.has_transferred_byte_count > 1 {
            return None;
        }
        let reason = match self.reason {
            0 => Reason::NotFound,
            1 => Reason::PermissionDenied,
            2 => Reason::InvalidInput,
            3 => Reason::Interrupted,
            4 => Reason::ResourceExhausted,
            5 => Reason::Unsupported,
            6 => Reason::Closed,
            7 => Reason::Other,
            _ => return None,
        };
        Some(Failure {
            reason,
            system_code: (self.has_system_code != 0).then_some(self.system_code),
            transferred_byte_count: (self.has_transferred_byte_count != 0)
                .then_some(self.transferred_byte_count),
        })
    }

    pub(crate) fn fail(&mut self, failure: Failure) -> u8 {
        *self = Self {
            value: 0,
            reason: failure.reason as u8,
            system_code: failure.system_code.unwrap_or(0),
            has_system_code: u8::from(failure.system_code.is_some()),
            transferred_byte_count: failure.transferred_byte_count.unwrap_or(0),
            has_transferred_byte_count: u8::from(failure.transferred_byte_count.is_some()),
        };
        3
    }

    pub(crate) fn complete(&mut self, result: Result<u64, Failure>) -> u8 {
        match result {
            Ok(value) => {
                self.value = value;
                0
            }
            Err(failure) => self.fail(failure),
        }
    }

    pub(crate) fn read(&mut self, result: Result<ReadProgress, Failure>) -> u8 {
        match result {
            Ok(ReadProgress::Data(count)) => self.complete(Ok(count as u64)),
            Ok(ReadProgress::WouldBlock(_)) => 1,
            Ok(ReadProgress::EndOfStream) => 2,
            Err(failure) => self.fail(failure),
        }
    }

    pub(crate) fn write(&mut self, result: Result<WriteProgress, Failure>) -> u8 {
        match result {
            Ok(WriteProgress::Accepted(count)) => self.complete(Ok(count as u64)),
            Ok(WriteProgress::WouldBlock(_)) => 1,
            Err(failure) => self.fail(failure),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_failed_records_clears_stale_optional_facts() {
        let mut status = DrIoStatusV1 {
            value: 99,
            system_code: 99,
            transferred_byte_count: 99,
            reason: 255,
            has_system_code: 1,
            has_transferred_byte_count: 1,
        };
        for system_code in [None, Some(0), Some(13), None] {
            for transferred_byte_count in [None, Some(0), Some(3), None] {
                let failure = Failure {
                    reason: Reason::Other,
                    system_code,
                    transferred_byte_count,
                };
                assert_eq!(status.fail(failure), 3);
                assert_eq!(status.failure(), Some(failure));
                assert_eq!(status.value, 0);
                assert_eq!(status.system_code, system_code.unwrap_or(0));
                assert_eq!(
                    status.transferred_byte_count,
                    transferred_byte_count.unwrap_or(0)
                );
            }
        }
    }

    #[test]
    fn failure_projection_is_shared_and_preserves_absent_zero_and_partial_facts() {
        for reason in [
            Reason::NotFound,
            Reason::PermissionDenied,
            Reason::InvalidInput,
            Reason::Interrupted,
            Reason::ResourceExhausted,
            Reason::Unsupported,
            Reason::Closed,
            Reason::Other,
        ] {
            for code in [None, Some(0), Some(13)] {
                for progress in [None, Some(0), Some(3)] {
                    let failure = Failure {
                        reason,
                        system_code: code,
                        transferred_byte_count: progress,
                    };
                    let mut status = DrIoStatusV1::default();
                    assert_eq!(status.fail(failure), 3);
                    assert_eq!(status.failure(), Some(failure));
                }
            }
        }
        let mut status = DrIoStatusV1 {
            reason: u8::MAX,
            ..DrIoStatusV1::default()
        };
        assert!(status.failure().is_none());
        status.reason = Reason::Other as u8;
        status.has_system_code = 2;
        assert!(status.failure().is_none());
        status.has_system_code = 0;
        status.has_transferred_byte_count = 2;
        assert!(status.failure().is_none());
    }

    #[test]
    fn transfer_tags_preserve_zero_data_partial_progress_and_eof() {
        let mut status = DrIoStatusV1::default();
        assert_eq!(status.read(Ok(ReadProgress::Data(0))), 0);
        assert_eq!(status.value, 0);
        assert_eq!(status.read(Ok(ReadProgress::Data(3))), 0);
        assert_eq!(status.value, 3);
        assert_eq!(status.write(Ok(WriteProgress::Accepted(2))), 0);
        assert_eq!(status.value, 2);
        let would_block = Failure::other().with_transferred_byte_count(Some(0));
        assert_eq!(status.read(Ok(ReadProgress::WouldBlock(would_block))), 1);
        assert_eq!(status.write(Ok(WriteProgress::WouldBlock(would_block))), 1);
        assert_eq!(status.read(Ok(ReadProgress::EndOfStream)), 2);
        assert_eq!(status.has_transferred_byte_count, 0);
    }

    #[test]
    fn failed_reads_and_writes_preserve_unknown_zero_and_partial_progress() {
        for transferred in [None, Some(0), Some(3)] {
            for code in [None, Some(13)] {
                let failure = Failure {
                    reason: crate::platform_io::Reason::Other,
                    system_code: code,
                    transferred_byte_count: transferred,
                };
                for reading in [true, false] {
                    let mut status = DrIoStatusV1::default();
                    let tag = if reading {
                        status.read(Err(failure))
                    } else {
                        status.write(Err(failure))
                    };
                    assert_eq!(tag, 3);
                    assert_eq!(status.reason, failure.reason as u8);
                    assert_eq!(status.has_system_code, u8::from(code.is_some()));
                    assert_eq!(status.system_code, code.unwrap_or(0));
                    assert_eq!(
                        status.has_transferred_byte_count,
                        u8::from(transferred.is_some())
                    );
                    assert_eq!(status.transferred_byte_count, transferred.unwrap_or(0));
                }
            }
        }
    }
}
