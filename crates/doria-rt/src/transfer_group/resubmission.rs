//! Reuse keeps the original operation, key, request record and caller storage.
//! Host preparation/cleanup handoffs resume the same native submission path.

use core::task::Poll;

use super::{Buffer, Control, GroupDriver, TerminalOutcome, TransferGroup};
use crate::{platform_io::Failure, DrStackFrameV2};

#[derive(Clone, Copy)]
enum Phase {
    Preflight,
    Preparation,
    Dispatch,
    Finished,
}

pub(crate) struct ResubmissionCall<
    'g,
    's,
    'b,
    D: GroupDriver<T>,
    C: Control<D::Timeout>,
    T: TerminalOutcome = Result<usize, Failure>,
> {
    group: &'g mut TransferGroup<'s, 'b, D, C, T>,
    state: ResubmissionState,
}

pub(crate) struct ResubmissionState {
    index: usize,
    phase: Phase,
}

impl ResubmissionState {
    pub(crate) fn new<D: GroupDriver<T>, C: Control<D::Timeout>, T: TerminalOutcome>(
        group: &mut TransferGroup<'_, '_, D, C, T>,
        index: usize,
        offset: usize,
        length: usize,
    ) -> Result<Self, Failure> {
        let entry = group
            .buffers
            .entries
            .as_mut_slice()
            .get_mut(index)
            .ok_or_else(Failure::invalid_input)?;
        let end = offset
            .checked_add(length)
            .ok_or_else(Failure::invalid_input)?;
        if entry.active
            || entry.ready.is_some()
            || end > entry.buffer.len()
            || group.stopped.is_some()
        {
            return Err(Failure::invalid_input());
        }
        entry.offset = offset;
        entry.length = length;
        entry.progress = None;
        entry.cancellation_failure = None;
        if D::PREPARE_BEFORE_SUBMISSION {
            entry.resource = None;
        }
        Ok(Self {
            index,
            phase: Phase::Preflight,
        })
    }

    /// # Safety
    /// Group and all prepared loans are retained between polls. Frame is
    /// current for this call only, with no overlapping group access/reentry.
    pub(crate) unsafe fn poll<D: GroupDriver<T>, C: Control<D::Timeout>, T: TerminalOutcome>(
        &mut self,
        group: &mut TransferGroup<'_, '_, D, C, T>,
        frame: *const DrStackFrameV2,
    ) -> Poll<()> {
        group.with_call_frame(frame, |group| {
            ResubmissionCall::<D, C, T>::advance(group, self.index, &mut self.phase)
        })
    }
}

impl<'g, 's, 'b, D: GroupDriver<T>, C: Control<D::Timeout>, T: TerminalOutcome>
    ResubmissionCall<'g, 's, 'b, D, C, T>
{
    pub(crate) fn new(
        group: &'g mut TransferGroup<'s, 'b, D, C, T>,
        index: usize,
        offset: usize,
        length: usize,
    ) -> Result<Self, Failure> {
        let state = ResubmissionState::new(group, index, offset, length)?;
        Ok(Self { group, state })
    }

    /// # Safety
    /// Frame is null or current for this poll only. Requested host work runs
    /// without reentry on this same thread. The group, its record and buffer
    /// loans remain retained until preparation or cleanup has returned.
    pub(crate) unsafe fn poll(&mut self, frame: *const DrStackFrameV2) -> Poll<()> {
        self.state.poll(self.group, frame)
    }

    fn advance(
        group: &mut TransferGroup<'s, 'b, D, C, T>,
        index: usize,
        phase: &mut Phase,
    ) -> Poll<()> {
        loop {
            match *phase {
                Phase::Preflight => {
                    let entry = &group.buffers.entries.as_slice()[index];
                    if entry.length == 0 && !D::EMPTY_MUST_DISPATCH {
                        let reading = matches!(entry.buffer, Buffer::Read(_));
                        group.finish(index, Some(T::success(0, reading)), None);
                        *phase = Phase::Finished;
                    } else if entry.length != 0 && Self::stop_before_work(group, index) {
                        *phase = Phase::Dispatch;
                    } else if D::PREPARE_BEFORE_SUBMISSION {
                        *phase = Phase::Preparation;
                    } else {
                        group.enqueue(index);
                        *phase = Phase::Dispatch;
                    }
                }
                Phase::Preparation => match group.poll_prepare(index) {
                    Poll::Ready(Ok(())) => {
                        group.enqueue(index);
                        *phase = Phase::Dispatch;
                    }
                    Poll::Ready(Err(error)) => {
                        group.finish(
                            index,
                            Some(TransferGroup::<D, C, T>::preparation_terminal(error)),
                            None,
                        );
                        *phase = Phase::Finished;
                    }
                    Poll::Pending => {
                        let entry = &group.buffers.entries.as_slice()[index];
                        if entry.preparation_failure.is_none()
                            && entry.length != 0
                            && Self::stop_before_work(group, index)
                        {
                            *phase = Phase::Dispatch;
                        } else {
                            return Poll::Pending;
                        }
                    }
                },
                Phase::Dispatch => {
                    if group.poll_dispatch_queued(false).is_pending() {
                        return Poll::Pending;
                    }
                    *phase = Phase::Finished;
                }
                Phase::Finished => return Poll::Ready(()),
            }
        }
    }

    fn stop_before_work(group: &mut TransferGroup<'s, 'b, D, C, T>, index: usize) -> bool {
        if let Some(stop) = group.preflight() {
            if !group.buffers.entries.as_slice()[index].active {
                group.unsubmitted(index, stop);
            }
            group.begin_stop(stop);
            true
        } else {
            false
        }
    }
}
