//! Callback-independent acceptance of a prepared operation. Normal return owns
//! one completion even when validation fails; throwing preparation owns none.

use super::{
    numeric_status, AdapterTerminal, Buffer, Configuration, PendingDriver, PendingNativeDriver,
    PendingRecords, Phase, Prepared,
};
use crate::{platform_io::Failure, transfer_group::TerminalOutcome};
use core::ptr;

pub(super) enum Preparation {
    Ready,
    Failed(AdapterTerminal),
    /// Native work never started. Retain the successful preparation and its
    /// source/storage loans until completion has returned, then report failure.
    Completion(Failure),
}

impl<D: PendingNativeDriver, R: PendingRecords> PendingDriver<'_, D, R> {
    /// # Safety
    /// Index/request belong to this group. The selected caller region remains
    /// borrowed through preparation and its eventual completion or failure.
    /// Source resolution uses only this call's frame, never a retained one.
    pub(super) unsafe fn begin_preparation(
        &mut self,
        index: usize,
        request: *mut D::Request,
        buffer: &mut Buffer<'_>,
    ) -> Result<(), AdapterTerminal> {
        self.begin_preparation_source(index, request, buffer, None)
    }

    pub(super) unsafe fn begin_preparation_source(
        &mut self,
        index: usize,
        request: *mut D::Request,
        buffer: &mut Buffer<'_>,
        external: Option<*const core::ffi::c_void>,
    ) -> Result<(), AdapterTerminal> {
        let record = &mut *self.record(index);
        record.reading = matches!(buffer, Buffer::Read(_));
        record.bound = buffer.len();
        record.caller_data = match buffer {
            Buffer::Read(bytes) => bytes.as_mut_ptr(),
            Buffer::Write(bytes) => bytes.as_ptr().cast_mut(),
        };
        if let Err(failure) = record.validate(record.reading) {
            record.caller_data = ptr::null_mut();
            record.phase = Phase::Taken;
            return Err(AdapterTerminal::PlatformFailure(failure));
        }
        match self
            .native
            .select_source(index, request, self.frame, external)
        {
            Ok(Some(source)) => {
                record.origin = Some(source.origin);
                if let Some(context) = source.context {
                    record.adapter.update_context(context);
                }
            }
            Ok(None) => {}
            Err(terminal) => {
                record.caller_data = ptr::null_mut();
                record.phase = Phase::Taken;
                self.native.release_source(index, request);
                return Err(terminal);
            }
        }
        record.phase = Phase::Preparing;
        Ok(())
    }

    /// # Safety
    /// Prepared is the once-decoded reply from this operation's preparation.
    /// User dispatch has returned; no configurator, view or output home remains
    /// borrowed. Caller, selected storage and source stay live across Completion.
    pub(super) unsafe fn accept_preparation(
        &mut self,
        index: usize,
        request: *mut D::Request,
        prepared: Prepared,
    ) -> Preparation {
        let record = &mut *self.record(index);
        if !matches!(record.phase, Phase::Preparing) || record.configurator.is_some() {
            crate::runtime_invariant_panic();
        }
        let region = match prepared {
            Prepared::Immediate(outcome) => {
                record.phase = Phase::Immediate(outcome);
                return Preparation::Ready;
            }
            Prepared::Failed(terminal) => {
                record.caller_data = ptr::null_mut();
                record.configuration = None;
                record.phase = Phase::Taken;
                self.native.release_source(index, request);
                return Preparation::Failed(terminal);
            }
            Prepared::Native(region) => region,
            Prepared::Invalid => {
                // Keep the first selection for cleanup; later malformed calls
                // cannot erase it or fabricate native submission/progress.
                let immediate = match record.configuration {
                    Some(Configuration::Native(region)) => {
                        record.native_region = region;
                        None
                    }
                    Some(Configuration::Immediate(outcome)) => Some(outcome),
                    None => None,
                };
                record.begin_completion(
                    false,
                    immediate.as_ref().map(|_| numeric_status(Ok(0))),
                    immediate.as_ref(),
                );
                return Preparation::Completion(Failure::invalid_input());
            }
        };
        record.native_region = region;
        record.native_bound = region.length;
        record.phase = Phase::Prepared(region);
        let validation = if (region.data.is_null() && region.length != 0)
            || region.length > D::MAXIMUM_TRANSFER_BYTES.min(isize::MAX as usize)
            || (region.data as usize).checked_add(region.length).is_none()
            || (0..self.records.extent()).any(|other| {
                let Some(prior) = self.records.record(other) else {
                    return false;
                };
                if prior == ptr::from_mut(record) {
                    return false;
                }
                let prior = &*prior;
                matches!(
                    prior.phase,
                    Phase::Prepared(_) | Phase::Native | Phase::Immediate(_) | Phase::Finalizing(_)
                ) && super::super::regions_conflict(
                    (region.data as usize, region.length, record.reading),
                    (
                        prior.native_region.data as usize,
                        prior.native_region.length,
                        prior.reading,
                    ),
                )
            }) {
            Err(Failure::invalid_input())
        } else {
            self.native
                .validate_endpoint(index, request, region.endpoint)
        };
        if let Err(failure) = validation {
            record.phase = Phase::Preparing;
            record.begin_completion(false, None, None);
            return Preparation::Completion(failure);
        }
        if let Some(outcome) = self
            .native
            .prepare_region(index, request, record.reading, region)
        {
            record.phase = Phase::Immediate(outcome);
        }
        Preparation::Ready
    }

    /// # Safety
    /// This rejected preparation's completion was accepted exactly once. The
    /// original owned cleanup Error takes precedence over validation failure.
    pub(super) unsafe fn preparation_failure(
        &mut self,
        index: usize,
        failure: Failure,
    ) -> AdapterTerminal {
        match (*self.record(index)).take_ready() {
            terminal if terminal.is_checked_error() => terminal,
            terminal => {
                terminal.drop_in_frame(self.frame);
                AdapterTerminal::PlatformFailure(failure)
            }
        }
    }
}
