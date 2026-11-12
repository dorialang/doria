//! The terminal layer behind `Doria\Std\Term` (Decision 0140).
//!
//! The transport, renderer, decoder and operation core are pure and live in
//! `doria-terminal`, which the debug interpreter's scripted terminal shares.
//! This module owns what is common to every platform's real terminal: the
//! process-wide ledger of what the program changed, the device the output
//! collects on, and putting the terminal back when the process ends. A
//! platform module owns the input mode, the size and the event wait.

pub use doria_terminal::{console, decode, render, wire};

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
mod queue;

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use unix as platform;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod unsupported;
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
use unsupported as platform;

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod pty_tests;

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};

use crate::cancellation::DrCancellationV1;
use crate::device_io::{standard_device, DeviceLoan, StandardStream};
use crate::platform_io::{Failure, Reason};
use crate::wait_budget::DrWaitBudgetV1;
pub(crate) use console::{ledger, OutputClass, Refusal, Undo};
use wire::{operation, ColorSupport, Observation};

/// Platform state touched by one owner at a time, or written once before
/// the ledger entry that publishes it.
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
pub(crate) struct Shared<T>(pub(crate) core::cell::UnsafeCell<T>);

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
unsafe impl<T> Sync for Shared<T> {}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
impl<T> Shared<T> {
    pub(crate) const fn new(value: T) -> Self {
        Self(core::cell::UnsafeCell::new(value))
    }
}

/// One call at a time uses the raw-mode state: entering, leaving, waiting
/// for an event and asking for the cursor exclude each other.
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
pub(crate) struct Exclusive;

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
static RAW_BUSY: AtomicBool = AtomicBool::new(false);

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
impl Exclusive {
    pub(crate) fn acquire() -> Result<Self, Refusal> {
        RAW_BUSY
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map(|_| Self)
            .map_err(|_| Refusal::in_use())
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
impl Drop for Exclusive {
    fn drop(&mut self) {
        RAW_BUSY.store(false, Ordering::Release);
    }
}

impl From<Failure> for Refusal {
    fn from(failure: Failure) -> Self {
        Self {
            status: if matches!(failure.reason, Reason::Unsupported) {
                wire::STATUS_UNSUPPORTED
            } else {
                wire::STATUS_DEVICE_FAILURE
            },
            system_code: failure.system_code,
        }
    }
}

static OUTPUT_CLASS: AtomicU8 = AtomicU8::new(0);

/// # Safety
/// The caller holds the standard-output loan: classifying may change the
/// console's output mode on Windows.
unsafe fn output_class() -> OutputClass {
    let known = OUTPUT_CLASS.load(Ordering::Acquire);
    if known != 0 {
        return OutputClass::from_bits(known);
    }
    let mut class = platform::classify_output();
    if !class.terminal || !class.sequences {
        class.color = ColorSupport::None;
    }
    OUTPUT_CLASS.store(class.to_bits(), Ordering::Release);
    class
}

static LEDGER: AtomicU32 = AtomicU32::new(0);
/// Entries whose undoing sequence waits in the console output. They leave the
/// ledger when that output reaches the terminal, not before: until then the
/// terminal is still changed, and a process end must still undo it.
static UNDONE_WHEN_WRITTEN: AtomicU32 = AtomicU32::new(0);
static RAW_GENERATION: AtomicU32 = AtomicU32::new(0);
static SCREEN_GENERATION: AtomicU32 = AtomicU32::new(0);

/// Records a change before it is made, so no instant exists in which the
/// terminal is changed and the ledger does not say so.
pub(crate) fn mark(entry: u32) {
    UNDONE_WHEN_WRITTEN.fetch_and(!entry, Ordering::AcqRel);
    LEDGER.fetch_or(entry, Ordering::AcqRel);
    watch_for_interruption();
}

pub(crate) fn unmark(entry: u32) {
    LEDGER.fetch_and(!entry, Ordering::AcqRel);
    watch_for_interruption();
}

static WATCH_LOCK: AtomicBool = AtomicBool::new(false);
static WATCHING: AtomicBool = AtomicBool::new(false);

/// The platform's interruption handlers are installed exactly while the
/// ledger holds something to put back: the first change installs them and
/// undoing the last one removes them. A program that never changes the
/// terminal never has a handler installed on its behalf.
fn watch_for_interruption() {
    while WATCH_LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
    }
    // Read under the lock, so the last caller through decides.
    let wanted = LEDGER.load(Ordering::Acquire) != 0;
    if wanted != WATCHING.load(Ordering::Relaxed) {
        unsafe {
            if wanted {
                platform::watch_for_interruption();
            } else {
                platform::stop_watching_for_interruption();
            }
        }
        WATCHING.store(wanted, Ordering::Relaxed);
    }
    WATCH_LOCK.store(false, Ordering::Release);
}

/// The restore from a signal handler or a console control handler: atomics,
/// one write and the platform's mode calls, nothing else. Waiting console
/// output is not written; the handler cannot take the device.
///
/// `ending` takes the ledger, for an interruption the process will not
/// survive. A stop keeps it, so that continuing can make the changes again.
///
/// # Safety
/// Called from the platform's interruption handler only.
pub(crate) unsafe fn restore_on_interruption(ending: bool) {
    let entries = if ending {
        LEDGER.swap(0, Ordering::AcqRel)
    } else {
        LEDGER.load(Ordering::Acquire)
    };
    if entries == 0 {
        return;
    }
    let settling = standard_device(StandardStream::Stdout).is_settling();
    let undo = Undo::new(entries, settling);
    if !undo.as_bytes().is_empty() {
        platform::write_at_process_end(undo.as_bytes());
    }
    platform::restore_at_process_end(entries);
}

/// What the ledger holds, for a platform that makes the changes again after
/// the program was stopped and continued.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn changes() -> u32 {
    LEDGER.load(Ordering::Acquire)
}

/// The entry is in effect and the program has not already undone it.
fn active(entry: u32) -> bool {
    LEDGER.load(Ordering::Acquire) & !UNDONE_WHEN_WRITTEN.load(Ordering::Acquire) & entry != 0
}

/// The device wrote all waiting console output.
pub(crate) fn console_output_written() {
    let undone = UNDONE_WHEN_WRITTEN.swap(0, Ordering::AcqRel);
    if undone != 0 {
        unmark(undone);
    }
}

/// Puts the terminal back as the program found it. Every way the runtime ends
/// the process comes through here first; so does a panic, before its message
/// is written. It costs two atomic loads for a program that never touched the
/// terminal.
///
/// # Safety
/// The process is ending: nothing uses the terminal layer afterwards.
pub(crate) unsafe fn process_ending() {
    let output = standard_device(StandardStream::Stdout);
    if LEDGER.load(Ordering::Acquire) == 0 && !output.has_waiting_output() {
        return;
    }
    // A frame the program finished is shown before the screen it was drawn on
    // goes away. When another owner holds the device there is nobody to wait
    // for, and what waits is dropped.
    if let Some(loan) = output.try_borrow_for_console() {
        let _ = loan.flush_console_output();
    }
    let entries = LEDGER.swap(0, Ordering::AcqRel);
    UNDONE_WHEN_WRITTEN.store(0, Ordering::Release);
    let undo = Undo::new(entries, output.is_settling());
    if !undo.as_bytes().is_empty() {
        platform::write_at_process_end(undo.as_bytes());
    }
    platform::restore_at_process_end(entries);
}

/// The runtime's terminal: output collects on the standard-output device and
/// the ledger is the process-wide one a signal handler can read.
struct Device<'a> {
    output: DeviceLoan<'a>,
}

impl console::Host for Device<'_> {
    fn append(&mut self, bytes: &[u8]) -> Result<(), Refusal> {
        unsafe { self.output.append_console_output(bytes) }.map_err(Refusal::from)
    }

    fn mark(&mut self, entry: u32) {
        mark(entry);
    }

    fn unmark(&mut self, entry: u32) {
        unmark(entry);
    }

    fn unmark_when_written(&mut self, entry: u32) {
        UNDONE_WHEN_WRITTEN.fetch_or(entry, Ordering::AcqRel);
    }

    fn active(&self, entry: u32) -> bool {
        active(entry)
    }

    fn size(&mut self) -> Result<(u32, u32), Refusal> {
        unsafe { platform::size() }
    }

    fn enter_screen(&mut self) -> u32 {
        SCREEN_GENERATION
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
    }

    fn screen(&self) -> u32 {
        SCREEN_GENERATION.load(Ordering::Acquire)
    }
}

unsafe fn operate(operation: u32, first: i64, second: i64, text: &[u8]) -> Result<u64, Refusal> {
    match operation {
        operation::ENTER_RAW_MODE => return enter_raw_mode(),
        operation::LEAVE_RAW_MODE => return leave_raw_mode(first as u32),
        operation::CURSOR_POSITION => return cursor_position(first as u32),
        // A fact about the device itself, which a loan held elsewhere does
        // not change, so it is answered without borrowing the stream.
        operation::STREAM_IS_INTERACTIVE => {
            let stream = match first {
                0 => StandardStream::Stdin,
                1 => StandardStream::Stdout,
                2 => StandardStream::Stderr,
                _ => return Err(Refusal::unsupported()),
            };
            return Ok(u64::from(crate::device_io::is_interactive(stream)));
        }
        _ => {}
    }
    let output = standard_device(StandardStream::Stdout)
        .try_borrow_for_console()
        .ok_or_else(Refusal::in_use)?;
    let class = output_class();
    if operation == operation::COLOR_SUPPORT {
        return Ok(class.color as u64);
    }
    if operation == operation::FLUSH {
        output.flush_console_output()?;
        return Ok(0);
    }
    if let Some(failure) = output.take_console_failure() {
        return Err(failure.into());
    }
    console::operate(
        &mut Device { output },
        class,
        operation,
        first,
        second,
        text,
    )
}

unsafe fn enter_raw_mode() -> Result<u64, Refusal> {
    // The platform takes the standard-input loan, which is what makes a
    // second entry fail: only one owner can hold it.
    platform::enter_raw_mode()?;
    let generation = RAW_GENERATION
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    Ok(u64::from(generation))
}

fn raw_mode_is(generation: u32) -> bool {
    active(ledger::RAW_MODE) && RAW_GENERATION.load(Ordering::Acquire) == generation
}

unsafe fn leave_raw_mode(generation: u32) -> Result<u64, Refusal> {
    if raw_mode_is(generation) {
        platform::leave_raw_mode()?;
    }
    Ok(0)
}

unsafe fn cursor_position(generation: u32) -> Result<u64, Refusal> {
    if !raw_mode_is(generation) {
        return Err(Refusal::in_use());
    }
    let (column, row) = platform::cursor_position()?;
    Ok(wire::pair(column, row))
}

unsafe fn await_event(
    generation: u32,
    budget: &DrWaitBudgetV1,
    cancellation: *mut DrCancellationV1,
) -> Result<Observation, Refusal> {
    if !raw_mode_is(generation) {
        return Ok(Observation::Closed);
    }
    // What the program drew is on the screen before it waits for a reaction.
    drop(standard_device(StandardStream::Stdout).try_borrow());
    platform::await_event(budget, cancellation)
}

/// What a transfer group sees when it asks about a raw mode it watches
/// (Decision 0140).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Watched {
    /// An event wait would return at once: a key or a resize is waiting, the
    /// terminal has closed, or asking failed and the event wait is what
    /// reports the failure.
    Ready,
    /// Nothing yet. `until` is when a sequence held back for the escape delay
    /// becomes a key without more input, on the monotonic clock.
    Waiting { until: Option<u64> },
}

/// Whether [`dr_v1_terminal_await_event`] with an immediate wait would
/// observe something other than a timeout for raw mode `generation`. It reads
/// what the terminal already has into the event queue and takes nothing out,
/// so the program's own event waits see every event exactly once.
///
/// # Safety
/// Called on the thread that drives the terminal layer.
pub(crate) unsafe fn watched(generation: u32) -> Watched {
    if !raw_mode_is(generation) {
        return Watched::Ready;
    }
    match platform::events_waiting() {
        Ok(Watched::Ready) | Err(_) => Watched::Ready,
        Ok(waiting) => {
            // A wait for events follows: what the program drew is on the
            // screen first, as it is before the program's own event wait.
            drop(standard_device(StandardStream::Stdout).try_borrow());
            waiting
        }
    }
}

/// What a wait can include to wake when raw mode `generation` may have an
/// event: readable or signaled while the terminal has input or a notice.
/// `None` when that raw mode has ended, which [`watched`] reports as ready.
///
/// # Safety
/// Called on the thread that drives the terminal layer.
pub(crate) unsafe fn watch_handle(generation: u32) -> Option<crate::native_handle::NativeHandle> {
    if raw_mode_is(generation) {
        platform::event_handle()
    } else {
        None
    }
}

/// One `Console`, `RawMode` or `AlternateScreen` operation, as compiled Doria
/// calls it. The result packs a status and a payload as `wire` defines.
///
/// # Safety
/// `text` is null or a live Doria string.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_terminal_operate(
    operation: u32,
    first: i64,
    second: i64,
    text: *const crate::DrStringV1,
) -> i64 {
    let text = if text.is_null() || (*text).byte_length == 0 {
        &[]
    } else {
        core::slice::from_raw_parts(crate::string_bytes(text), (*text).byte_length)
    };
    console::result(operate(operation, first, second, text))
}

/// [`dr_v1_terminal_operate`] for a host whose strings are not Doria strings.
///
/// # Safety
/// `text` is readable for `text_length` bytes, or `text_length` is zero.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_host_terminal_operate(
    operation: u32,
    first: i64,
    second: i64,
    text: *const u8,
    text_length: usize,
) -> i64 {
    let text = if text_length == 0 {
        &[]
    } else {
        core::slice::from_raw_parts(text, text_length)
    };
    console::result(operate(operation, first, second, text))
}

/// Waits for the next key or resize of the raw mode `generation` entered.
/// The wait arrives as every `Doria\Std\Io\Wait` does: 0 immediate,
/// 1 indefinite, 2 a duration in `wait_low`, 3 a deadline in both words.
///
/// # Safety
/// `cancellation` is null or a live cancellation state.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_terminal_await_event(
    generation: u32,
    wait_kind: u8,
    wait_low: u64,
    wait_high: i64,
    cancellation: *mut DrCancellationV1,
) -> i64 {
    console::result(
        DrWaitBudgetV1::from_wait(wait_kind, wait_low, wait_high)
            .map_err(Refusal::from)
            .and_then(|budget| await_event(generation, &budget, cancellation))
            .map(Observation::to_payload),
    )
}

/// The restore for a host that ends the process itself instead of leaving
/// through the runtime's exits.
///
/// # Safety
/// The process is ending: nothing uses the terminal layer afterwards.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_terminal_process_ending() {
    process_ending();
}
