//! The terminal layer's wait (Decision 0140): standard input and the resize
//! notice on the shared readiness set, so terminal events use the same wait,
//! budget and cancellation as every other readiness wait in the runtime.
use core::ptr;

use super::{DrReadyEventV1, GroupCancellationWatch, Set, READ};
use crate::cancellation::DrCancellationV1;
use crate::platform_io::Failure;
use crate::wait_budget::DrWaitBudgetV1;

const INPUT: u64 = 0;
const NOTICE: u64 = 1;

pub(crate) struct TerminalWatch {
    set: Set,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalWake {
    /// `input` also covers a closed or failed terminal: the read that follows
    /// is what reports it.
    Ready {
        input: bool,
        notice: bool,
    },
    TimedOut,
    Cancelled,
}

impl TerminalWatch {
    /// # Safety
    /// `input` stays open and `notice` stays a live signal for this watch's
    /// lifetime.
    pub(crate) unsafe fn new(input: i32, notice: usize) -> Result<Self, Failure> {
        let mut set = Set::new(2, ptr::null_mut())?;
        set.add(input, INPUT, READ)?;
        set.add(
            crate::cancellation_signal::native_handle(notice),
            NOTICE,
            READ,
        )?;
        Ok(Self { set })
    }

    /// # Safety
    /// `cancellation` is null or a live cancellation state.
    pub(crate) unsafe fn wait(
        &mut self,
        budget: &DrWaitBudgetV1,
        cancellation: *mut DrCancellationV1,
    ) -> Result<TerminalWake, Failure> {
        let mut watch = GroupCancellationWatch::install(&mut self.set, cancellation)?;
        let mut ready = [DrReadyEventV1::default(); 4];
        let result = self.set.wait(budget, &mut ready);
        watch.detach(&mut self.set);
        Ok(match result? {
            (0, count) => {
                let ready = &ready[..count];
                TerminalWake::Ready {
                    input: ready.iter().any(|event| event.key == INPUT),
                    notice: ready.iter().any(|event| event.key == NOTICE),
                }
            }
            (1, _) => TerminalWake::TimedOut,
            _ => TerminalWake::Cancelled,
        })
    }
}
