//! A fetch owns only its wait budget, never transfer retirement or storage.
//! Platform timeout representations are limited here before the shared loop
//! waits; no fetch-local reference survives the call.

use core::task::Poll;

use super::{
    Completion, Control, GroupDriver, Query, Stop, TerminalOutcome, TransferGroup, Wait, Wake,
};
use crate::platform_io::Failure;
use crate::wait_budget::DrWaitBudgetV1;

pub(crate) enum Observation<T: TerminalOutcome> {
    Completion(Completion<T>),
    /// The watch at this index among the plan's watches completed: its
    /// source is ready, or the group stopped while it was armed.
    Watch(usize),
    /// Report once before taking any retained terminal transfer.
    WaitFailed(Failure),
    /// The group has no outstanding transfer or queued completion.
    NoCompletion,
    /// This observation found no completion within its finite/immediate budget;
    /// outstanding transfers and retained loans remain live.
    TimedOut,
}

pub(crate) trait WaitTimeout: Copy {
    fn indefinite() -> Self;
    fn limited_by(self, observation: Self) -> Self;
    /// This timeout, ending no later than `deadline` on the monotonic clock.
    fn limited_until(self, deadline: u64) -> Result<Self, Failure>;
}

pub(crate) trait ObservationBudget<T> {
    /// None means this fetch's deadline expired, not that a transfer stopped.
    fn next_wait(&mut self) -> Result<Option<Wait<T>>, Failure>;
}

impl<T, B: ObservationBudget<T> + ?Sized> ObservationBudget<T> for &mut B {
    fn next_wait(&mut self) -> Result<Option<Wait<T>>, Failure> {
        (**self).next_wait()
    }
}

#[derive(Clone, Copy)]
enum Phase<T> {
    Start,
    Queries {
        step: usize,
        wait: Wait<T>,
        pending: bool,
    },
    Dispatch(Wait<T>),
    Stopping,
    Finished,
}

/// A fetch owns its frozen budget and traversal, and exclusively borrows the
/// original group across host handoffs. It owns no second request, I/O engine,
/// buffer, source pin or Error queue. Dropping a fetch does not stop transfers.
pub(crate) struct ObservationCall<
    'g,
    's,
    'b,
    D: GroupDriver<T>,
    C: Control<D::Timeout>,
    B: ObservationBudget<D::Timeout>,
    T: TerminalOutcome = Result<usize, Failure>,
> {
    group: &'g mut TransferGroup<'s, 'b, D, C, T>,
    budget: B,
    state: ObservationState<D::Timeout>,
}

/// Traversal owns values only. A hosted owner lends its pinned group and
/// budget to each poll, so no self-reference or caller home survives yield.
pub(crate) struct ObservationState<T> {
    phase: Phase<T>,
    immediate_polled: bool,
    observation_polled: bool,
    resample_observation: bool,
}

impl<T: Copy> ObservationState<T> {
    pub(crate) fn new() -> Self {
        Self {
            phase: Phase::Start,
            immediate_polled: false,
            observation_polled: false,
            resample_observation: false,
        }
    }

    /// # Safety
    /// The original group/budget remain exclusively retained between polls.
    /// Frame is current for this poll only; no host reentry is allowed.
    pub(crate) unsafe fn poll<D, C, B, O>(
        &mut self,
        group: &mut TransferGroup<'_, '_, D, C, O>,
        budget: &mut B,
        frame: *const crate::DrStackFrameV2,
    ) -> Poll<Observation<O>>
    where
        D: GroupDriver<O, Timeout = T>,
        C: Control<T>,
        B: ObservationBudget<T>,
        O: TerminalOutcome,
    {
        let result = group.with_call_frame(frame, |group| {
            ObservationCall::<D, C, B, O>::advance(
                group,
                budget,
                &mut self.phase,
                &mut self.immediate_polled,
                &mut self.observation_polled,
                &mut self.resample_observation,
            )
        });
        if result.is_ready() {
            self.phase = Phase::Finished;
        }
        result
    }
}

impl<
        'g,
        's,
        'b,
        D: GroupDriver<T>,
        C: Control<D::Timeout>,
        B: ObservationBudget<D::Timeout>,
        T: TerminalOutcome,
    > ObservationCall<'g, 's, 'b, D, C, B, T>
{
    pub(crate) fn new(group: &'g mut TransferGroup<'s, 'b, D, C, T>, budget: B) -> Self {
        Self {
            group,
            budget,
            state: ObservationState::new(),
        }
    }

    /// # Safety
    /// Frame is null or current for this poll only. Host work runs on the same
    /// thread without reentry; it cannot replace this fetch's frozen budget.
    /// No driver home, temporary buffer, frame or output address survives yield.
    pub(crate) unsafe fn poll(
        &mut self,
        frame: *const crate::DrStackFrameV2,
    ) -> Poll<Observation<T>> {
        self.state.poll(self.group, &mut self.budget, frame)
    }

    fn advance(
        group: &mut TransferGroup<'s, 'b, D, C, T>,
        budget: &mut B,
        phase: &mut Phase<D::Timeout>,
        immediate_polled: &mut bool,
        observation_polled: &mut bool,
        resample_observation: &mut bool,
    ) -> Poll<Observation<T>> {
        use doria_diagnostic_catalogue::IoMessageOperation::Wait as WaitOperation;
        loop {
            match *phase {
                Phase::Start => {
                    if group.stop_drain.is_some() {
                        *phase = Phase::Stopping;
                        continue;
                    }
                    if let Some(failure) = group.take_wait_failure() {
                        return Poll::Ready(failure);
                    }
                    // A watch reports once per arming, so asking it before
                    // queued transfers cannot starve them, and a key is never
                    // held behind a stream that always has bytes.
                    if let Some(index) = group.take_watch() {
                        return Poll::Ready(Observation::Watch(index));
                    }
                    if let Some(completion) = group.take_ready() {
                        return Poll::Ready(Observation::Completion(completion));
                    }
                    if !group.has_active() && !group.has_armed_watch() {
                        return Poll::Ready(Observation::NoCompletion);
                    }
                    match budget.next_wait() {
                        Ok(Some(wait)) => {
                            *phase = Phase::Queries {
                                step: 0,
                                wait,
                                pending: false,
                            }
                        }
                        Ok(None) => {
                            if let Some(stop) = group.preflight() {
                                group.begin_stop(stop);
                                *phase = Phase::Stopping;
                            } else {
                                return Poll::Ready(Observation::TimedOut);
                            }
                        }
                        Err(failure) => {
                            group.begin_stop(Stop::Failure(WaitOperation, failure));
                            *phase = Phase::Stopping;
                        }
                    }
                }
                Phase::Queries {
                    step,
                    wait,
                    pending,
                } => {
                    if step == group.len() {
                        *phase = Phase::Dispatch(wait);
                        continue;
                    }
                    let index = group.rotated(step);
                    if !group.buffers.entries.as_slice()[index].submitted {
                        *phase = Phase::Queries {
                            step: step + 1,
                            wait,
                            pending: false,
                        };
                        continue;
                    }
                    if !pending && D::QUERY_CAN_TRANSFER {
                        if let Some(stop) = group.preflight() {
                            group.begin_stop(stop);
                            *phase = Phase::Stopping;
                            continue;
                        }
                    }
                    let (registered, raw) = group.request(index);
                    if !pending && D::QUERY_REFRESHES_SOURCE {
                        if let Err(terminal) =
                            unsafe { group.driver.prepare_query(registered, raw) }
                        {
                            group.finish(index, Some(terminal), None);
                            *phase = Phase::Queries {
                                step: step + 1,
                                wait,
                                pending: false,
                            };
                            continue;
                        }
                        if group.refresh_query_resource(index) {
                            *phase = if group.stopped.is_some() {
                                Phase::Stopping
                            } else {
                                Phase::Queries {
                                    step: step + 1,
                                    wait,
                                    pending: false,
                                }
                            };
                            continue;
                        }
                    }
                    match unsafe { group.driver.poll_query(registered, raw) } {
                        Poll::Pending => {
                            *phase = Phase::Queries {
                                step,
                                wait,
                                pending: true,
                            };
                            *resample_observation = true;
                            // Consume a returned reply before a racing stop; a
                            // still-unreturned reply keeps all loans through drain.
                            if let Some(stop) = group.preflight() {
                                group.begin_stop(stop);
                                *phase = Phase::Stopping;
                                continue;
                            }
                            return Poll::Pending;
                        }
                        Poll::Ready(Query::Terminal(result)) => {
                            group.finish(index, Some(result), None)
                        }
                        Poll::Ready(Query::PendingFailure(operation, failure)) => {
                            group.begin_stop(Stop::Failure(operation, failure));
                            *phase = Phase::Stopping;
                            continue;
                        }
                        Poll::Ready(Query::Pending) => {
                            if D::QUERY_REFRESHES_SOURCE {
                                group.refresh_query_resource(index);
                                if group.stopped.is_some() {
                                    *phase = Phase::Stopping;
                                    continue;
                                }
                            }
                        }
                    }
                    *phase = Phase::Queries {
                        step: step + 1,
                        wait,
                        pending: false,
                    };
                }
                Phase::Dispatch(mut observation_wait) => {
                    if group.poll_dispatch_queued(true).is_pending() {
                        *phase = Phase::Stopping;
                        return Poll::Pending;
                    }
                    if let Some(result) = group.take_observation() {
                        return Poll::Ready(result);
                    }
                    if !group.has_active() && !group.has_armed_watch() {
                        return Poll::Ready(Observation::NoCompletion);
                    }
                    if D::QUERY_CAN_TRANSFER && *immediate_polled {
                        group.begin_stop(Stop::BudgetExhausted);
                        *phase = Phase::Stopping;
                        continue;
                    }
                    if *observation_polled {
                        return Poll::Ready(Observation::TimedOut);
                    }
                    if core::mem::take(resample_observation) {
                        // Time spent in the host must not extend a finite fetch.
                        match budget.next_wait() {
                            Ok(Some(wait)) => observation_wait = wait,
                            Ok(None) => {
                                if let Some(stop) = group.preflight() {
                                    group.begin_stop(stop);
                                    *phase = Phase::Stopping;
                                    continue;
                                }
                                return Poll::Ready(Observation::TimedOut);
                            }
                            Err(failure) => {
                                group.begin_stop(Stop::Failure(WaitOperation, failure));
                                *phase = Phase::Stopping;
                                continue;
                            }
                        }
                    }
                    let wait = match group.control.next_wait() {
                        Ok(Ok(wait)) => wait,
                        Ok(Err(stop)) => {
                            group.begin_stop(stop);
                            *phase = Phase::Stopping;
                            continue;
                        }
                        Err(failure) => {
                            group.begin_stop(Stop::Failure(WaitOperation, failure));
                            *phase = Phase::Stopping;
                            continue;
                        }
                    };
                    let mut timeout = wait.timeout.limited_by(observation_wait.timeout);
                    if let Some(deadline) = group.watch_limit() {
                        // A watched source becomes ready at this moment with
                        // nothing arriving; the pass after the wait asks it.
                        timeout = match timeout.limited_until(deadline) {
                            Ok(timeout) => timeout,
                            Err(failure) => {
                                group.begin_stop(Stop::Failure(WaitOperation, failure));
                                *phase = Phase::Stopping;
                                continue;
                            }
                        };
                    }
                    match group
                        .driver
                        .wait_any(timeout, group.control.cancellation_signal())
                    {
                        Ok(Wake::Timeout) if wait.immediate => {
                            group.begin_stop(Stop::BudgetExhausted);
                            *phase = Phase::Stopping;
                        }
                        Ok(Wake::Timeout) if observation_wait.immediate => {
                            return Poll::Ready(Observation::TimedOut)
                        }
                        Ok(Wake::Completion | Wake::Timeout) => {
                            *immediate_polled = D::QUERY_CAN_TRANSFER && wait.immediate;
                            *observation_polled = observation_wait.immediate;
                            *phase = Phase::Start;
                        }
                        Ok(Wake::Cancellation) => {
                            group.begin_stop(Stop::Cancellation);
                            *phase = Phase::Stopping;
                        }
                        Err(failure) => {
                            group.begin_stop(Stop::Failure(WaitOperation, failure));
                            *phase = Phase::Stopping;
                        }
                    }
                }
                Phase::Stopping => {
                    if group.poll_stop_all(group.stopped.unwrap()).is_pending() {
                        return Poll::Pending;
                    }
                    *phase = Phase::Start;
                }
                Phase::Finished => unsafe { crate::runtime_invariant_panic() },
            }
        }
    }
}

pub(super) struct Indefinite;

impl<T: WaitTimeout> ObservationBudget<T> for Indefinite {
    fn next_wait(&mut self) -> Result<Option<Wait<T>>, Failure> {
        Ok(Some(Wait {
            timeout: T::indefinite(),
            immediate: false,
        }))
    }
}

impl WaitTimeout for DrWaitBudgetV1 {
    fn indefinite() -> Self {
        Self::indefinite()
    }

    fn limited_by(self, observation: Self) -> Self {
        self.limited_by(observation)
    }

    fn limited_until(self, deadline: u64) -> Result<Self, Failure> {
        Ok(self.limited_by(DrWaitBudgetV1::new(3, deadline, || Ok(0))?))
    }
}

impl ObservationBudget<DrWaitBudgetV1> for DrWaitBudgetV1 {
    fn next_wait(&mut self) -> Result<Option<Wait<DrWaitBudgetV1>>, Failure> {
        let (kind, _) = self.remaining(1, u64::MAX, crate::monotonic_clock::now)?;
        Ok((kind != 3).then_some(Wait {
            timeout: *self,
            immediate: kind == 0,
        }))
    }
}

impl WaitTimeout for u32 {
    fn indefinite() -> Self {
        u32::MAX
    }

    fn limited_by(self, observation: Self) -> Self {
        self.min(observation)
    }

    /// Milliseconds, rounded up so the wait does not end before the moment.
    fn limited_until(self, deadline: u64) -> Result<Self, Failure> {
        let remaining = deadline.saturating_sub(crate::monotonic_clock::now()?);
        let milliseconds = remaining.div_ceil(1_000_000).min(u64::from(u32::MAX - 1));
        Ok(self.min(milliseconds as u32))
    }
}

impl ObservationBudget<u32> for DrWaitBudgetV1 {
    fn next_wait(&mut self) -> Result<Option<Wait<u32>>, Failure> {
        let (kind, units) = self.remaining(
            1_000_000,
            u64::from(u32::MAX - 1),
            crate::monotonic_clock::now,
        )?;
        Ok(match kind {
            3 => None,
            1 => Some(Wait {
                timeout: u32::MAX,
                immediate: false,
            }),
            _ => Some(Wait {
                timeout: units as u32,
                immediate: kind == 0,
            }),
        })
    }
}

#[cfg(test)]
impl WaitTimeout for i64 {
    fn indefinite() -> Self {
        -1
    }

    fn limited_by(self, observation: Self) -> Self {
        match (self, observation) {
            (-1, _) => observation,
            (_, -1) => self,
            _ => self.min(observation),
        }
    }

    fn limited_until(self, _: u64) -> Result<Self, Failure> {
        Ok(self)
    }
}
