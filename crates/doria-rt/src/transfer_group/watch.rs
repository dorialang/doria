//! Sources a group waits on beside its transfers (Decision 0140). A watch
//! moves no bytes. The group asks the source whether an event wait on it
//! would return at once and reports that, once per arming; the source keeps
//! its own queue and the program takes events from it as before. While a
//! watch is armed the group's one wait also wakes for the source, so there is
//! no second wait, reader or thread.

use super::{Control, FixedSlice, GroupDriver, TerminalOutcome, TransferGroup};
use crate::native_handle::NativeHandle;
use crate::platform_io::Failure;
use crate::terminal::Watched;

/// A runtime-backed source a plan watches: its kind above bit 32 and that
/// kind's token below. The compiler-known `Doria\Std\Io\WatchSource` carries
/// exactly these bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WatchSource(u64);

/// A terminal raw mode. The token is its generation.
const TERMINAL: u64 = 1;
/// A source a test scripts on its own thread. The token is its index.
#[cfg(test)]
const SCRIPTED: u64 = 0xFFFF;

#[cfg(test)]
std::thread_local! {
    static SCRIPT: std::cell::RefCell<std::vec::Vec<Watched>> =
        const { std::cell::RefCell::new(std::vec::Vec::new()) };
}

impl WatchSource {
    /// Bits of a kind this runtime does not know are refused before a plan
    /// keeps them.
    pub(crate) fn from_bits(bits: u64) -> Result<Self, Failure> {
        match bits >> 32 {
            TERMINAL => Ok(Self(bits)),
            #[cfg(test)]
            SCRIPTED => Ok(Self(bits)),
            _ => Err(Failure::invalid_input()),
        }
    }

    fn token(self) -> u32 {
        self.0 as u32
    }

    #[cfg(test)]
    pub(crate) fn scripted(index: u32, state: Watched) -> Self {
        SCRIPT.with(|script| {
            let mut script = script.borrow_mut();
            if script.len() <= index as usize {
                script.resize(index as usize + 1, Watched::Waiting { until: None });
            }
            script[index as usize] = state;
        });
        Self((SCRIPTED << 32) | u64::from(index))
    }

    /// Whether an event wait on the source would return at once.
    unsafe fn poll(self) -> Watched {
        match self.0 >> 32 {
            TERMINAL => crate::terminal::watched(self.token()),
            #[cfg(test)]
            SCRIPTED => SCRIPT.with(|script| script.borrow()[self.token() as usize]),
            _ => crate::runtime_invariant_panic(),
        }
    }

    /// What the group's wait includes while a watch on the source is armed.
    /// `None` when the source needs no wake because it is already ready.
    unsafe fn wake(self) -> Option<NativeHandle> {
        match self.0 >> 32 {
            TERMINAL => crate::terminal::watch_handle(self.token()),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// Reported and not armed again, or never armed. Nothing is waited for.
    Idle,
    Armed,
    /// The group stopped while the watch was armed. Like a transfer retired
    /// by the stop, it is reported once and carries no outcome.
    Stopped,
}

pub(super) struct Watch {
    source: WatchSource,
    state: State,
    /// Attached to the driver's wait on behalf of this watch.
    wake: Option<NativeHandle>,
    /// When the source said it becomes ready without anything arriving.
    until: Option<u64>,
}

impl<D: GroupDriver<T>, C: Control<D::Timeout>, T: TerminalOutcome> TransferGroup<'_, '_, D, C, T> {
    /// Arms the plan's watches, in plan order. Called once, after the group
    /// has begun and before it is observed. A failure leaves nothing armed.
    pub(crate) fn watch(&mut self, sources: &[WatchSource]) -> Result<(), Failure> {
        if sources.is_empty() {
            return Ok(());
        }
        if self.watches.is_some() {
            return Err(Failure::invalid_input());
        }
        let mut watches = FixedSlice::new(sources.len())?;
        for source in sources {
            watches.push(Watch {
                source: *source,
                state: State::Idle,
                wake: None,
                until: None,
            })?;
        }
        self.watches = Some(watches);
        if self.stopped.is_some() {
            // Controls that had already ended the group when it began retire
            // its watches with its transfers.
            for index in 0..sources.len() {
                self.watch_mut(index).state = State::Stopped;
            }
            return Ok(());
        }
        for index in 0..sources.len() {
            if let Err(failure) = self.arm_watch(index) {
                self.release_watches();
                return Err(failure);
            }
        }
        Ok(())
    }

    pub(crate) fn watch_count(&self) -> usize {
        self.watches.as_ref().map_or(0, FixedSlice::len)
    }

    /// Arms a watch again after its completion was fetched. An armed watch, a
    /// retirement not fetched yet and a stopped group are refused, as they are
    /// for a transfer.
    pub(crate) fn rearm_watch(&mut self, index: usize) -> Result<(), Failure> {
        let idle = self
            .watches
            .as_ref()
            .and_then(|watches| watches.as_slice().get(index))
            .is_some_and(|watch| watch.state == State::Idle);
        if !idle || self.stopped.is_some() {
            return Err(Failure::invalid_input());
        }
        // Controls that ended the group before this call retire the watch at
        // once, the way they retire a transfer resubmitted in the same moment.
        if let Some(stop) = self.preflight() {
            self.watch_mut(index).state = State::Stopped;
            self.begin_stop(stop);
            return Ok(());
        }
        self.arm_watch(index)
    }

    fn watch_mut(&mut self, index: usize) -> &mut Watch {
        match self.watches.as_mut() {
            Some(watches) => &mut watches.as_mut_slice()[index],
            None => unsafe { crate::runtime_invariant_panic() },
        }
    }

    fn wake_attached(&self, handle: NativeHandle) -> bool {
        self.watches.as_ref().is_some_and(|watches| {
            watches
                .as_slice()
                .iter()
                .any(|watch| watch.wake == Some(handle))
        })
    }

    fn arm_watch(&mut self, index: usize) -> Result<(), Failure> {
        let source = self.watch_mut(index).source;
        let wake = unsafe { source.wake() };
        if let Some(handle) = wake {
            // Watches of one source share its place in the wait.
            if !self.wake_attached(handle) {
                unsafe { self.driver.attach_wake(handle)? };
            }
        }
        let watch = self.watch_mut(index);
        watch.wake = wake;
        watch.until = None;
        watch.state = State::Armed;
        Ok(())
    }

    fn disarm_watch(&mut self, index: usize, state: State) {
        let watch = self.watch_mut(index);
        watch.state = state;
        watch.until = None;
        if let Some(handle) = watch.wake.take() {
            if !self.wake_attached(handle) {
                unsafe { self.driver.detach_wake(handle) };
            }
        }
    }

    /// The watch the next observation reports, if any: in plan order, one the
    /// stop retired, or an armed one whose source is ready now. Asking at the
    /// moment of the report is what keeps a source the program has already
    /// emptied from being reported.
    pub(super) fn take_watch(&mut self) -> Option<usize> {
        for index in 0..self.watch_count() {
            match self.watch_mut(index).state {
                State::Idle => {}
                State::Stopped => {
                    self.watch_mut(index).state = State::Idle;
                    return Some(index);
                }
                State::Armed => match unsafe { self.watch_mut(index).source.poll() } {
                    Watched::Ready => {
                        self.disarm_watch(index, State::Idle);
                        return Some(index);
                    }
                    Watched::Waiting { until } => self.watch_mut(index).until = until,
                },
            }
        }
        None
    }

    pub(super) fn has_armed_watch(&self) -> bool {
        self.watches.as_ref().is_some_and(|watches| {
            watches
                .as_slice()
                .iter()
                .any(|watch| watch.state == State::Armed)
        })
    }

    /// The earliest moment an armed watch becomes ready with nothing
    /// arriving, as its source last said. The wait must not outlast it.
    pub(super) fn watch_limit(&self) -> Option<u64> {
        self.watches.as_ref().and_then(|watches| {
            watches
                .as_slice()
                .iter()
                .filter(|watch| watch.state == State::Armed)
                .filter_map(|watch| watch.until)
                .min()
        })
    }

    /// A stopping group retires its armed watches with its transfers.
    pub(super) fn stop_watches(&mut self) {
        for index in 0..self.watch_count() {
            if self.watch_mut(index).state == State::Armed {
                self.disarm_watch(index, State::Stopped);
            }
        }
    }

    /// Teardown: nothing stays in the driver's wait on behalf of a watch.
    pub(super) fn release_watches(&mut self) {
        for index in 0..self.watch_count() {
            self.disarm_watch(index, State::Idle);
        }
    }
}
