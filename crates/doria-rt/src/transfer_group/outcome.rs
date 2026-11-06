//! Inline private transport for terminal and wait-control facts. Public errors
//! are projected by the compiler; this record is not another diagnostic model.

use super::{Awaited, Stop};
use crate::byte_io::{ReadProgress, WriteProgress};
use crate::io_status::DrIoStatusV1;

#[repr(C)]
#[derive(Default)]
pub(crate) struct DrCompletionStatusV1 {
    pub(crate) transfer: DrIoStatusV1,
    pub(crate) wait_failure: DrIoStatusV1,
    pub(crate) cancellation_failure: DrIoStatusV1,
    // 0 uninterrupted, 1 cancellation, 2 exhausted budget, 3 wait/control failure.
    pub(crate) stop: u8,
    pub(crate) has_cancellation_failure: u8,
    pub(crate) wait_operation: u8,
    pub(crate) cancellation_operation: u8,
}

impl DrCompletionStatusV1 {
    pub(crate) fn project_wait_failure(&mut self, failure: crate::platform_io::Failure) -> u8 {
        self.project(
            Awaited {
                transfer: None,
                stop: Some(Stop::Failure(
                    doria_diagnostic_catalogue::IoMessageOperation::Wait,
                    failure,
                )),
                cancellation_failure: None,
            },
            false,
        );
        7
    }

    pub(crate) fn project(&mut self, awaited: Awaited, reading: bool) -> u8 {
        *self = Self::default();
        self.stop = match awaited.stop {
            None => 0,
            Some(Stop::Cancellation) => 1,
            Some(Stop::BudgetExhausted) => 2,
            Some(Stop::Failure(operation, failure)) => {
                self.wait_operation = operation as u8;
                self.wait_failure.fail(failure);
                3
            }
        };
        if let Some((operation, failure)) = awaited.cancellation_failure {
            self.has_cancellation_failure = 1;
            self.cancellation_operation = operation as u8;
            self.cancellation_failure.fail(failure);
        }
        match awaited.transfer {
            // Private tag 4: not submitted. No transfer outcome is invented.
            None => 4,
            Some(result) if reading => self.transfer.read(match result {
                Ok(count) => Ok(ReadProgress::Data(count)),
                Err(failure) => Err(failure),
            }),
            Some(result) => self.transfer.write(result.map(WriteProgress::Accepted)),
        }
    }
}

impl<T> Awaited<T> {
    pub(crate) fn after_wait_failure_observation(mut self) -> Self {
        if matches!(
            self.stop,
            Some(Stop::Failure(
                doria_diagnostic_catalogue::IoMessageOperation::Wait,
                _
            ))
        ) {
            self.stop = None;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform_io::Failure;
    use doria_diagnostic_catalogue::IoMessageOperation;

    #[test]
    fn wait_failure_transport_does_not_repeat_it_or_replace_transfer_progress() {
        let failure = Failure::other();
        let terminal = Failure::other().with_transferred_byte_count(Some(7));
        let mut status = DrCompletionStatusV1::default();
        let (tag, allocations) =
            crate::probe_runtime_allocations(|| status.project_wait_failure(failure));
        assert_eq!((tag, allocations), (7, 0));
        assert_eq!(
            (status.stop, status.wait_operation),
            (3, IoMessageOperation::Wait as u8)
        );
        assert_eq!(status.wait_failure.failure(), Some(failure));
        assert_eq!(status.wait_failure.has_transferred_byte_count, 0);
        assert_eq!(status.transfer.has_transferred_byte_count, 0);

        let completion = Awaited {
            transfer: Some(Err(terminal)),
            stop: Some(Stop::Failure(IoMessageOperation::Wait, failure)),
            cancellation_failure: None,
        }
        .after_wait_failure_observation();
        assert_eq!(status.project(completion, true), 3);
        assert_eq!(status.stop, 0);
        assert_eq!(status.transfer.failure(), Some(terminal));
        assert_eq!(
            (
                status.transfer.has_transferred_byte_count,
                status.transfer.transferred_byte_count
            ),
            (1, 7)
        );
    }
}
