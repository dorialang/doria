//! Resumable configuration of the existing transfer core. Host handoffs pause
//! ordinary adapter calls; they do not introduce another I/O or cleanup engine.

use core::{ptr, task::Poll};

use super::{
    Buffer, BufferList, Control, FixedSlice, GroupDriver, RequestSlots, SetupFailure,
    TerminalOutcome, TransferGroup,
};
use crate::{platform_io::Failure, DrStackFrameV2};

#[derive(Clone, Copy)]
enum Phase {
    Sources { index: usize, pending: bool },
    Preparations(usize),
    Admission,
    Dispatch,
    Ready,
    Failed,
}

/// The complete group guard retains its set, request records and buffer loans
/// even between host calls. Only successful startup exposes a TransferGroup.
pub(crate) struct GroupSetup<
    's,
    'b,
    D: GroupDriver<T>,
    C: Control<D::Timeout>,
    T: TerminalOutcome = Result<usize, Failure>,
> {
    pub(super) group: TransferGroup<'s, 'b, D, C, T>,
    phase: Phase,
}

impl<'s, 'b, D: GroupDriver<T>, C: Control<D::Timeout>, T: TerminalOutcome>
    GroupSetup<'s, 'b, D, C, T>
{
    pub(crate) fn configure(
        slots: &'s mut RequestSlots<D::Request>,
        buffers: BufferList<'b, T>,
        driver: D,
        control: C,
        selection: Option<FixedSlice<usize>>,
    ) -> Result<Self, SetupFailure<T>> {
        let count = selection.as_ref().map_or(slots.len(), FixedSlice::len);
        if count != buffers.entries.len() {
            return Err(Failure::invalid_input().into());
        }
        if let Some(indices) = &selection {
            if indices
                .as_slice()
                .iter()
                .enumerate()
                .any(|(index, selected)| {
                    !slots.contains(*selected) || indices.as_slice()[..index].contains(selected)
                })
            {
                return Err(Failure::invalid_input().into());
            }
        } else if slots.occupied_len() != slots.len() {
            return Err(Failure::invalid_input().into());
        }
        let mut group = TransferGroup {
            slots,
            selection,
            buffers,
            driver,
            control,
            cursor: 0,
            stopped: None,
            stop_drain: None,
            wait_failure_reported: false,
        };
        // Install the whole cleanup owner before acquisition or any user call.
        for index in 0..group.len() {
            let (registered, request) = group.request(index);
            unsafe {
                group.driver.acquire_request(registered, request)?;
                if !D::PREPARE_BEFORE_SUBMISSION {
                    group.buffers.entries.as_mut_slice()[index].resource =
                        Some(group.driver.resource(registered, request)?);
                }
            }
        }
        if !D::QUERY_CAN_TRANSFER
            && group
                .buffers
                .entries
                .as_slice()
                .iter()
                .any(|entry| entry.length != 0)
        {
            group.stopped = group.preflight();
        }
        group.driver.set_call_frame(ptr::null());
        Ok(Self {
            group,
            phase: if D::PREPARE_BEFORE_SUBMISSION {
                Phase::Sources {
                    index: 0,
                    pending: false,
                }
            } else {
                Phase::Admission
            },
        })
    }

    /// # Safety
    /// Frame is null or current for this call only. The host executes any
    /// requested adapter work on this same driver thread, without reentrancy.
    /// No caller frame/output address survives a Pending return.
    pub(crate) unsafe fn poll(
        &mut self,
        frame: *const DrStackFrameV2,
    ) -> Poll<Result<(), SetupFailure<T>>> {
        let phase = &mut self.phase;
        self.group
            .with_call_frame(frame, |group| Self::advance(group, phase))
    }

    fn advance(
        group: &mut TransferGroup<'s, 'b, D, C, T>,
        phase: &mut Phase,
    ) -> Poll<Result<(), SetupFailure<T>>> {
        loop {
            match *phase {
                Phase::Sources { index, pending } => {
                    if group.stopped.is_some() || index == group.len() {
                        *phase = Phase::Preparations(0);
                        continue;
                    }
                    let length = group.buffers.entries.as_slice()[index].length;
                    if length == 0 && !D::EMPTY_MUST_DISPATCH {
                        *phase = Phase::Sources {
                            index: index + 1,
                            pending: false,
                        };
                        continue;
                    }
                    // A staged reply wins a racing stop; never suppress an
                    // already-returned original getter Error by polling first.
                    if !pending && length != 0 && Self::stop_before_work(group) {
                        continue;
                    }
                    let (registered, request) = group.request(index);
                    match unsafe { group.driver.poll_setup_request(registered, request) } {
                        Poll::Ready(Ok(())) => {
                            *phase = Phase::Sources {
                                index: index + 1,
                                pending: false,
                            };
                        }
                        Poll::Ready(Err(error)) => {
                            *phase = Phase::Failed;
                            return Poll::Ready(Err(SetupFailure::Preparation(error)));
                        }
                        Poll::Pending => {
                            *phase = Phase::Sources {
                                index,
                                pending: true,
                            };
                            if length != 0 && Self::stop_before_work(group) {
                                continue;
                            }
                            return Poll::Pending;
                        }
                    }
                }
                Phase::Preparations(index) => {
                    if group.stopped.is_some() || index == group.len() {
                        *phase = Phase::Admission;
                        continue;
                    }
                    let entry = &group.buffers.entries.as_slice()[index];
                    if entry.length == 0 && !D::EMPTY_MUST_DISPATCH {
                        *phase = Phase::Preparations(index + 1);
                        continue;
                    }
                    if !entry.preparing
                        && entry.preparation_failure.is_none()
                        && entry.length != 0
                        && Self::stop_before_work(group)
                    {
                        continue;
                    }
                    match group.poll_prepare(index) {
                        Poll::Ready(Ok(())) => *phase = Phase::Preparations(index + 1),
                        Poll::Ready(Err(error)) => {
                            *phase = Phase::Failed;
                            return Poll::Ready(Err(error));
                        }
                        Poll::Pending => {
                            let entry = &group.buffers.entries.as_slice()[index];
                            if entry.preparation_failure.is_none()
                                && entry.length != 0
                                && Self::stop_before_work(group)
                            {
                                continue;
                            }
                            return Poll::Pending;
                        }
                    }
                }
                Phase::Admission => {
                    for index in 0..group.len() {
                        let entry = &group.buffers.entries.as_slice()[index];
                        if entry.ready.is_some() {
                            continue;
                        }
                        if entry.length == 0 && !D::EMPTY_MUST_DISPATCH {
                            let reading = matches!(entry.buffer, Buffer::Read(_));
                            group.finish(index, Some(T::success(0, reading)), None);
                        } else if let Some(stop) = group.stopped {
                            group.unsubmitted(index, stop);
                        } else {
                            group.enqueue(index);
                        }
                    }
                    *phase = Phase::Dispatch;
                }
                Phase::Dispatch => {
                    match group
                        .poll_dispatch_queued(D::QUERY_CAN_TRANSFER || D::PREFLIGHT_EACH_SUBMISSION)
                    {
                        Poll::Ready(()) => *phase = Phase::Ready,
                        Poll::Pending => return Poll::Pending,
                    }
                }
                Phase::Ready => return Poll::Ready(Ok(())),
                Phase::Failed => return Poll::Ready(Err(Failure::invalid_input().into())),
            }
        }
    }

    fn stop_before_work(group: &mut TransferGroup<'s, 'b, D, C, T>) -> bool {
        if let Some(stop) = group.preflight() {
            group.begin_stop(stop);
            true
        } else {
            false
        }
    }

    // Both variants transfer the existing inline guard; boxing it would add an
    // allocation merely to return unfinished startup to its current owner.
    #[allow(clippy::result_large_err)]
    pub(crate) fn into_group(self) -> Result<TransferGroup<'s, 'b, D, C, T>, Self> {
        if matches!(self.phase, Phase::Ready) {
            Ok(self.group)
        } else {
            Err(self)
        }
    }

    /// Native callbacks finish inline; hosted callers retain this same guard
    /// and use poll instead. No additional group or cleanup owner is created.
    /// # Safety
    /// Frame is current for startup/cleanup only. This synchronous path must
    /// not be used for adapters requiring ordinary host handoffs.
    pub(crate) unsafe fn finish(
        mut self,
        frame: *const DrStackFrameV2,
    ) -> Result<TransferGroup<'s, 'b, D, C, T>, SetupFailure<T>> {
        let failure = match self.poll(frame) {
            Poll::Ready(Ok(())) => {
                return Ok(match self.into_group() {
                    Ok(group) => group,
                    Err(_) => crate::runtime_invariant_panic(),
                });
            }
            Poll::Ready(Err(failure)) => failure,
            Poll::Pending => SetupFailure::Platform(Failure::unsupported()),
        };
        self.drop_in_frame(frame);
        Err(failure)
    }

    /// # Safety
    /// Frame is null or current for this cleanup call only.
    pub(crate) unsafe fn drop_in_frame(mut self, frame: *const DrStackFrameV2) {
        if self.poll_cleanup(frame).is_pending() {
            crate::runtime_invariant_panic();
        }
        self.group.drop_in_frame(frame);
    }

    /// # Safety
    /// Frame is current only for this cleanup poll. The hosted owner retains
    /// this whole guard and drives every requested completion before dropping.
    pub(crate) unsafe fn poll_cleanup(&mut self, frame: *const DrStackFrameV2) -> Poll<()> {
        self.group.with_call_frame(frame, |group| {
            group.poll_stop_all(super::Stop::Cancellation)
        })
    }
}
