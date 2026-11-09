//! Compiler-owned adapter callbacks and move-only terminal facts. Readiness
//! controls waiting; only the complete adapter performs a transfer. No callback
//! context, checked error, or caller buffer is copied into a second I/O owner.

use core::ffi::c_void;

use super::TerminalOutcome;
use crate::io_status::DrIoStatusV1;
use crate::platform_io::Failure;
use crate::{DrInterfaceCarrierV1, DrStackFrameV2};

pub(crate) mod host_error;
use host_error::OwnedHostError;
#[cfg(test)]
pub(crate) mod test_object;

/// Success tags: read 0 Data, 1 WouldBlock, 2 EOF, 3 TimedOut; write 0 Accepted,
/// 1 WouldBlock. On checked failure has_count/count describe only an optional
/// completed-prefix projection, not the original IoError's signed property.
/// Compiler callbacks project only nonnegative, usize-representable progress;
/// runtime bounds the prefix without changing the owned checked error.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub(crate) struct DrAdapterOutcomeV1 {
    pub(crate) count: usize,
    pub(crate) tag: u8,
    pub(crate) has_count: u8,
}

/// Borrowed ordinary dispatch metadata for the retained complete receiver.
/// Descriptor is null for a closed concrete receiver, a class descriptor for
/// an open or inline receiver, or the original interface vtable for erasure.
/// Inline receiver bits occupy the object word and may be zero.
/// Copying these two words acquires no owner, access registration, or allocation.
/// A callback borrows this record for its call only and must not retain its home.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct DrAdapterContextV1 {
    pub object: *mut c_void,
    pub descriptor: *const c_void,
}

impl DrAdapterContextV1 {
    pub(crate) fn valid(&self) -> bool {
        !self.object.is_null() || !self.descriptor.is_null()
    }
}

pub(crate) type ReadCallback = unsafe extern "C" fn(
    *const DrStackFrameV2,
    *const DrAdapterContextV1,
    *mut u8,
    usize,
    *mut DrAdapterOutcomeV1,
    *mut DrInterfaceCarrierV1,
) -> u8;

pub(crate) type WriteCallback = unsafe extern "C" fn(
    *const DrStackFrameV2,
    *const DrAdapterContextV1,
    *const u8,
    usize,
    *mut DrAdapterOutcomeV1,
    *mut DrInterfaceCarrierV1,
) -> u8;

/// Copying this record copies borrowed metadata, never the complete stream or
/// an access/close claim. Context stays stable and uniquely accessible for each
/// synchronous callback. A fresh source resolver supplies its current dispatch
/// descriptor; the selected context stays unchanged during that callback. Callbacks do
/// not retain the context home, frame, region or output slots,
/// unwind through C, or reenter their exclusively borrowed set/group.
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct DrTransferAdapterV1 {
    pub(crate) context: DrAdapterContextV1,
    pub(crate) read: Option<ReadCallback>,
    pub(crate) write: Option<WriteCallback>,
}

impl DrTransferAdapterV1 {
    pub(crate) fn valid(&self) -> bool {
        self.context.valid() && (self.read.is_some() || self.write.is_some())
    }

    pub(crate) fn validate(&self, reading: bool) -> Result<(), Failure> {
        if self.valid()
            && if reading {
                self.read.is_some()
            } else {
                self.write.is_some()
            }
        {
            Ok(())
        } else {
            Err(Failure::invalid_input())
        }
    }

    /// # Safety
    /// The validated descriptor/context and uniquely borrowed writable region
    /// remain live for this call. Frame is null or live for this call only.
    pub(crate) unsafe fn read(
        &self,
        frame: *const DrStackFrameV2,
        data: *mut u8,
        length: usize,
    ) -> AdapterAttempt {
        let callback = self
            .read
            .unwrap_or_else(|| crate::runtime_invariant_panic());
        let mut outcome = DrAdapterOutcomeV1::default();
        let mut error = crate::empty_error_carrier();
        let status = callback(frame, &self.context, data, length, &mut outcome, &mut error);
        Self::decode(status, outcome, error, true, length)
    }

    /// # Safety
    /// The validated descriptor/context and borrowed readonly region remain
    /// live for this call. Frame is null or live for this call only.
    pub(crate) unsafe fn write(
        &self,
        frame: *const DrStackFrameV2,
        data: *const u8,
        length: usize,
    ) -> AdapterAttempt {
        let callback = self
            .write
            .unwrap_or_else(|| crate::runtime_invariant_panic());
        let mut outcome = DrAdapterOutcomeV1::default();
        let mut error = crate::empty_error_carrier();
        let status = callback(frame, &self.context, data, length, &mut outcome, &mut error);
        Self::decode(status, outcome, error, false, length)
    }

    pub(super) unsafe fn decode(
        status: u8,
        outcome: DrAdapterOutcomeV1,
        error: DrInterfaceCarrierV1,
        reading: bool,
        length: usize,
    ) -> AdapterAttempt {
        if status == 1 {
            if outcome.has_count > 1 {
                crate::runtime_invariant_panic();
            }
            return AdapterAttempt::Terminal(AdapterTerminal::CheckedError {
                error: OwnedIoError::from_carrier(error),
                progress: (outcome.has_count != 0 && outcome.count <= length)
                    .then_some(outcome.count),
            });
        }
        if status != 0 || !error.object.is_null() || !error.vtable.is_null() {
            crate::runtime_invariant_panic();
        }
        let terminal = match outcome.tag {
            0 if outcome.count <= length => {
                if reading {
                    AdapterTerminal::Data(outcome.count)
                } else {
                    AdapterTerminal::Accepted(outcome.count)
                }
            }
            1 => return AdapterAttempt::Pending,
            2 if reading => AdapterTerminal::EndOfStream,
            3 if reading => AdapterTerminal::TimedOut,
            _ => crate::runtime_invariant_panic(),
        };
        AdapterAttempt::Terminal(terminal)
    }
}

pub(crate) enum AdapterAttempt {
    Pending,
    Terminal(AdapterTerminal),
}

pub(crate) use crate::owned_interface::OwnedInterfaceValue as OwnedIoError;

#[derive(Debug)]
pub(crate) enum AdapterTerminal {
    Data(usize),
    Accepted(usize),
    /// A separate pending operation retired or completed in preparation with
    /// an explicit WouldBlock result. Synchronous callbacks still retry it.
    WouldBlock,
    EndOfStream,
    TimedOut,
    CheckedError {
        error: OwnedIoError,
        /// Safe completed prefix only; the original signed progress stays in
        /// the owned error even when this projection is unavailable.
        progress: Option<usize>,
    },
    /// The complete host adapter's real Error stays in its retained slot. This
    /// claim owns its return/drop disposition without pretending it is native.
    HostError {
        error: OwnedHostError,
        progress: Option<usize>,
    },
    /// Infrastructure faults are distinct from an adapter's owned checked error.
    PlatformFailure(Failure),
    /// Successful preparation was cleaned up without a submitted transfer.
    NoTransfer,
}

impl TerminalOutcome for AdapterTerminal {
    fn is_checked_error(&self) -> bool {
        matches!(self, Self::CheckedError { .. } | Self::HostError { .. })
    }

    fn success(count: usize, reading: bool) -> Self {
        if reading {
            Self::Data(count)
        } else {
            Self::Accepted(count)
        }
    }

    fn failure(failure: Failure) -> Self {
        Self::PlatformFailure(failure)
    }

    fn bounded(self, bound: usize) -> Self {
        if let Self::CheckedError { error, progress } = self {
            return Self::CheckedError {
                error,
                progress: progress.filter(|count| *count <= bound),
            };
        }
        if let Self::HostError { error, progress } = self {
            return Self::HostError {
                error,
                progress: progress.filter(|count| *count <= bound),
            };
        }
        if let Self::PlatformFailure(failure) = self {
            return Self::PlatformFailure(match Err(failure).bounded(bound) {
                Err(failure) => failure,
                Ok(_) => unreachable!(),
            });
        }
        if self.progress().is_some_and(|count| count > bound) {
            // Successful adapter outcomes retain their bounded-count contract.
            // Arbitrary signed checked-error properties are handled above.
            unsafe { crate::runtime_invariant_panic() };
        }
        self
    }

    fn progress(&self) -> Option<usize> {
        match self {
            Self::Data(count) | Self::Accepted(count) => Some(*count),
            Self::EndOfStream => Some(0),
            Self::CheckedError { progress, .. } | Self::HostError { progress, .. } => *progress,
            Self::PlatformFailure(failure) => failure.transferred_byte_count,
            Self::WouldBlock | Self::TimedOut | Self::NoTransfer => None,
        }
    }

    fn into_transfer(self) -> Option<Self> {
        if matches!(self, Self::NoTransfer) {
            None
        } else {
            Some(self)
        }
    }

    unsafe fn drop_in_frame(self, frame: *const DrStackFrameV2) {
        if let Self::CheckedError { error, .. } = self {
            error.drop_in_frame(frame);
        }
    }
}

impl AdapterTerminal {
    /// Returns 0 explicit outcome, 1 moved original checked error, 3 platform
    /// infrastructure failure. NoTransfer is erased by the common core.
    /// # Safety
    /// Error points to a writable EMPTY existing-carrier slot disjoint from all
    /// metadata outputs. Ownership transfers to that caller on tag 1 only.
    pub(crate) unsafe fn project(
        self,
        outcome: &mut DrAdapterOutcomeV1,
        error: *mut DrInterfaceCarrierV1,
        platform: &mut DrIoStatusV1,
    ) -> u8 {
        *outcome = DrAdapterOutcomeV1::default();
        *platform = DrIoStatusV1::default();
        match self {
            Self::Data(count) | Self::Accepted(count) => {
                outcome.count = count;
                outcome.has_count = 1;
                0
            }
            Self::EndOfStream => {
                outcome.tag = 2;
                0
            }
            Self::WouldBlock => {
                outcome.tag = 1;
                0
            }
            Self::TimedOut => {
                outcome.tag = 3;
                0
            }
            Self::CheckedError {
                error: owned,
                progress,
            } => {
                if error.is_null() || !(*error).object.is_null() || !(*error).vtable.is_null() {
                    crate::runtime_invariant_panic();
                }
                if let Some(count) = progress {
                    outcome.count = count;
                    outcome.has_count = 1;
                }
                error.write(owned.into_carrier());
                1
            }
            Self::PlatformFailure(failure) => platform.fail(failure),
            Self::NoTransfer | Self::HostError { .. } => crate::runtime_invariant_panic(),
        }
    }
}

#[cfg(test)]
mod tests;
