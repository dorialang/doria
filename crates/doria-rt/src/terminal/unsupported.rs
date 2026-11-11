//! Platforms with no terminal backend. Standard output is never classified
//! as a terminal here, so `Console::write` emits plain text and every
//! terminal operation reports that there is no terminal.
use super::wire::Observation;
use super::{OutputClass, Refusal};
use crate::cancellation::DrCancellationV1;
use crate::wait_budget::DrWaitBudgetV1;

pub(super) unsafe fn classify_output() -> OutputClass {
    OutputClass::NOT_A_TERMINAL
}

pub(super) unsafe fn size() -> Result<(u32, u32), Refusal> {
    Err(Refusal::not_a_terminal())
}

pub(super) unsafe fn enter_raw_mode() -> Result<(), Refusal> {
    Err(Refusal::not_a_terminal())
}

pub(super) unsafe fn leave_raw_mode() -> Result<(), Refusal> {
    Ok(())
}

pub(super) unsafe fn await_event(
    _budget: &DrWaitBudgetV1,
    _cancellation: *mut DrCancellationV1,
) -> Result<Observation, Refusal> {
    Ok(Observation::Closed)
}

pub(super) unsafe fn events_waiting() -> Result<super::Watched, Refusal> {
    Ok(super::Watched::Ready)
}

pub(super) unsafe fn event_handle() -> Option<crate::native_handle::NativeHandle> {
    None
}

pub(super) unsafe fn cursor_position() -> Result<(u32, u32), Refusal> {
    Err(Refusal::not_a_terminal())
}

pub(super) unsafe fn write_at_process_end(_bytes: &[u8]) {}

pub(super) unsafe fn restore_at_process_end(_entries: u32) {}

pub(super) unsafe fn watch_for_interruption() {}

pub(super) unsafe fn stop_watching_for_interruption() {}
