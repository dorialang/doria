//! The terminal layer behind `Doria\Std\Term` (Decision 0140).
//!
//! `wire`, `render` and `decode` are pure and shared with the debug
//! interpreter's scripted terminal, which is why they are public. This module
//! owns what is common to every platform: the operations, the ledger of what
//! the program changed, and putting it back when the process ends. A platform
//! module owns the terminal's input mode, its size and the event wait.

pub mod decode;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
mod queue;
pub mod render;
pub mod wire;

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
use wire::{operation, ColorSupport, Observation, Style};

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

/// Why an operation did not happen: a `TerminalErrorReason` and the native
/// code behind it, when there is one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Refusal {
    status: u8,
    system_code: Option<i64>,
}

impl Refusal {
    pub(crate) const fn not_a_terminal() -> Self {
        Self {
            status: wire::STATUS_NOT_A_TERMINAL,
            system_code: None,
        }
    }

    pub(crate) const fn in_use() -> Self {
        Self {
            status: wire::STATUS_IN_USE,
            system_code: None,
        }
    }

    pub(crate) const fn unsupported() -> Self {
        Self {
            status: wire::STATUS_UNSUPPORTED,
            system_code: None,
        }
    }

    /// The terminal went away while it was being asked something.
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    pub(crate) const fn device_failure() -> Self {
        Self {
            status: wire::STATUS_DEVICE_FAILURE,
            system_code: None,
        }
    }

    const fn to_result(self) -> i64 {
        wire::failure(self.status, self.system_code)
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

/// What standard output is, decided once: a redirected stream stays
/// redirected for the life of the process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OutputClass {
    pub(crate) terminal: bool,
    /// The terminal accepts the renderer's sequences. Only an old Windows
    /// console refuses them.
    pub(crate) sequences: bool,
    pub(crate) color: ColorSupport,
}

impl OutputClass {
    pub(crate) const NOT_A_TERMINAL: Self = Self {
        terminal: false,
        sequences: false,
        color: ColorSupport::None,
    };

    const KNOWN: u8 = 0x80;

    const fn to_bits(self) -> u8 {
        Self::KNOWN
            | ((self.terminal as u8) << 3)
            | ((self.sequences as u8) << 2)
            | self.color as u8
    }

    const fn from_bits(bits: u8) -> Self {
        Self {
            terminal: bits & 8 != 0,
            sequences: bits & 4 != 0,
            color: ColorSupport::from_code(bits & 3),
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

/// The ledger: one bit for each terminal setting the program has moved away
/// from the value it had at start. Cursor visibility, line wrap and the screen
/// in use cannot be read back from a terminal, so their starting values are
/// the ones every terminal starts a program with: visible, wrapping, main
/// screen. The input mode and the Windows output mode are read before they
/// are changed and kept by the platform module.
pub(crate) mod ledger {
    pub(crate) const CURSOR_HIDDEN: u32 = 1;
    pub(crate) const LINE_WRAP_DISABLED: u32 = 1 << 1;
    pub(crate) const ALTERNATE_SCREEN: u32 = 1 << 2;
    pub(crate) const RAW_MODE: u32 = 1 << 3;
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) const OUTPUT_MODE: u32 = 1 << 4;
    /// The entries undone by writing a sequence to the terminal.
    pub(crate) const RENDERED: u32 = CURSOR_HIDDEN | LINE_WRAP_DISABLED | ALTERNATE_SCREEN;
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

fn unmark_when_written(entry: u32) {
    UNDONE_WHEN_WRITTEN.fetch_or(entry, Ordering::AcqRel);
}

/// The device wrote all waiting console output.
pub(crate) fn console_output_written() {
    let undone = UNDONE_WHEN_WRITTEN.swap(0, Ordering::AcqRel);
    if undone != 0 {
        unmark(undone);
    }
}

/// The sequences that undo `entries`, with room to spare for the abort
/// prefix. Fixed storage: a signal handler builds this too.
pub(crate) struct Undo {
    bytes: [u8; 32],
    length: usize,
}

impl Undo {
    /// `interrupted` says a console write was cut short, so the terminal may
    /// be inside an escape sequence or a styled run that must end first.
    pub(crate) fn new(entries: u32, interrupted: bool) -> Self {
        let mut undo = Self {
            bytes: [0; 32],
            length: 0,
        };
        if entries & ledger::RENDERED == 0 {
            return undo;
        }
        if interrupted {
            undo.push(render::ABORT_SEQUENCE);
        }
        if entries & ledger::ALTERNATE_SCREEN != 0 {
            undo.push(render::LEAVE_ALTERNATE_SCREEN);
        }
        if entries & ledger::LINE_WRAP_DISABLED != 0 {
            undo.push(render::ENABLE_LINE_WRAP);
        }
        if entries & ledger::CURSOR_HIDDEN != 0 {
            undo.push(render::SHOW_CURSOR);
        }
        undo
    }

    /// The sequences that make `entries` again, for a program continued
    /// after a stop that undid them.
    #[cfg(any(target_os = "linux", target_os = "macos", test))]
    pub(crate) fn again(entries: u32) -> Self {
        let mut redo = Self {
            bytes: [0; 32],
            length: 0,
        };
        if entries & ledger::ALTERNATE_SCREEN != 0 {
            redo.push(render::ENTER_ALTERNATE_SCREEN);
        }
        if entries & ledger::LINE_WRAP_DISABLED != 0 {
            redo.push(render::DISABLE_LINE_WRAP);
        }
        if entries & ledger::CURSOR_HIDDEN != 0 {
            redo.push(render::HIDE_CURSOR);
        }
        redo
    }

    fn push(&mut self, sequence: &[u8]) {
        self.bytes[self.length..self.length + sequence.len()].copy_from_slice(sequence);
        self.length += sequence.len();
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.length]
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

unsafe fn append(output: &DeviceLoan<'_>, bytes: &[u8]) -> Result<(), Refusal> {
    output.append_console_output(bytes).map_err(Refusal::from)
}

fn count(argument: i64) -> u32 {
    argument.clamp(0, i64::from(wire::COORDINATE_LIMIT)) as u32
}

unsafe fn write(
    output: &DeviceLoan<'_>,
    class: OutputClass,
    style: Style,
    text: &[u8],
) -> Result<(), Refusal> {
    let prefix = render::style_prefix(style, class.color);
    append(output, prefix.as_bytes())?;
    append(output, text)?;
    if !prefix.is_empty() {
        append(output, render::STYLE_SUFFIX)?;
    }
    Ok(())
}

unsafe fn operate(operation: u32, first: i64, second: i64, text: &[u8]) -> Result<u64, Refusal> {
    match operation {
        operation::ENTER_RAW_MODE => return enter_raw_mode(),
        operation::LEAVE_RAW_MODE => return leave_raw_mode(first as u32),
        operation::CURSOR_POSITION => return cursor_position(first as u32),
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
    if operation == operation::WRITE {
        write(&output, class, Style::from_bits(first), text)?;
        return Ok(0);
    }
    if !class.terminal {
        // Nothing to put back on a stream that was never a terminal.
        return if operation == operation::RESET {
            Ok(0)
        } else {
            Err(Refusal::not_a_terminal())
        };
    }
    if !class.sequences {
        return Err(Refusal::unsupported());
    }
    match operation {
        operation::SIZE => {
            let (columns, rows) = platform::size()?;
            return Ok(wire::pair(columns, rows));
        }
        operation::WRITE_AT => {
            let cell = render::move_to(wire::cell_column(first), wire::cell_row(first));
            append(&output, cell.as_bytes())?;
            write(&output, class, Style::from_bits(second), text)?;
        }
        operation::CLEAR => append(&output, render::CLEAR)?,
        operation::CLEAR_LINE => append(&output, render::CLEAR_LINE)?,
        operation::CLEAR_LINE_FROM_CURSOR => append(&output, render::CLEAR_LINE_FROM_CURSOR)?,
        operation::ERASE_AT => {
            let cell = render::move_to(wire::cell_column(first), wire::cell_row(first));
            append(&output, cell.as_bytes())?;
            append(&output, render::ERASE_CHARACTER)?;
        }
        operation::SET_TITLE => {
            append(&output, render::TITLE_PREFIX)?;
            for run in render::title_runs(text) {
                append(&output, run)?;
            }
            append(&output, render::TITLE_SUFFIX)?;
        }
        operation::SET_LINE_WRAP => {
            if first == 0 {
                append(&output, render::ENABLE_LINE_WRAP)?;
                unmark_when_written(ledger::LINE_WRAP_DISABLED);
            } else {
                mark(ledger::LINE_WRAP_DISABLED);
                append(&output, render::DISABLE_LINE_WRAP)?;
            }
        }
        operation::MOVE_TO => {
            let cell = render::move_to(wire::cell_column(first), wire::cell_row(first));
            append(&output, cell.as_bytes())?;
        }
        operation::MOVE_UP
        | operation::MOVE_DOWN
        | operation::MOVE_LEFT
        | operation::MOVE_RIGHT => {
            let direction = match operation {
                operation::MOVE_UP => render::Direction::Up,
                operation::MOVE_DOWN => render::Direction::Down,
                operation::MOVE_LEFT => render::Direction::Left,
                _ => render::Direction::Right,
            };
            append(&output, render::move_by(direction, count(first)).as_bytes())?;
        }
        operation::SHOW_CURSOR => {
            append(&output, render::SHOW_CURSOR)?;
            unmark_when_written(ledger::CURSOR_HIDDEN);
        }
        operation::HIDE_CURSOR => {
            mark(ledger::CURSOR_HIDDEN);
            append(&output, render::HIDE_CURSOR)?;
        }
        operation::ENTER_ALTERNATE_SCREEN => {
            if active(ledger::ALTERNATE_SCREEN) {
                return Err(Refusal::in_use());
            }
            mark(ledger::ALTERNATE_SCREEN);
            if let Err(refusal) = append(&output, render::ENTER_ALTERNATE_SCREEN) {
                unmark(ledger::ALTERNATE_SCREEN);
                return Err(refusal);
            }
            let generation = SCREEN_GENERATION
                .fetch_add(1, Ordering::AcqRel)
                .wrapping_add(1);
            return Ok(u64::from(generation));
        }
        operation::LEAVE_ALTERNATE_SCREEN => {
            if active(ledger::ALTERNATE_SCREEN)
                && SCREEN_GENERATION.load(Ordering::Acquire) == first as u32
            {
                append(&output, render::LEAVE_ALTERNATE_SCREEN)?;
                unmark_when_written(ledger::ALTERNATE_SCREEN);
            }
        }
        operation::RESET => {
            append(&output, render::SHOW_CURSOR)?;
            append(&output, render::ENABLE_LINE_WRAP)?;
            append(&output, render::STYLE_SUFFIX)?;
            unmark_when_written(ledger::CURSOR_HIDDEN | ledger::LINE_WRAP_DISABLED);
        }
        _ => return Err(Refusal::unsupported()),
    }
    Ok(0)
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

/// One `Console`, `RawMode` or `AlternateScreen` operation. The result packs
/// a status and a payload as `wire` defines.
///
/// # Safety
/// `text` is readable for `text_length` bytes, or `text_length` is zero.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_terminal_operate(
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
    match operate(operation, first, second, text) {
        Ok(payload) => wire::success(payload),
        Err(refusal) => refusal.to_result(),
    }
}

/// Waits for the next key or resize of the raw mode `generation` entered.
///
/// # Safety
/// `budget` is a live wait budget. `cancellation` is null or a live
/// cancellation state.
#[no_mangle]
pub unsafe extern "C" fn dr_v1_terminal_await_event(
    generation: u32,
    budget: *const DrWaitBudgetV1,
    cancellation: *mut DrCancellationV1,
) -> i64 {
    match await_event(generation, &*budget, cancellation) {
        Ok(observation) => wire::success(observation.to_payload()),
        Err(refusal) => refusal.to_result(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_keeps_its_reason_and_native_code_through_the_wire() {
        let refusal = Refusal::from(Failure::from_system_code(5));
        let result = refusal.to_result();
        assert_eq!(wire::status(result), wire::STATUS_DEVICE_FAILURE);
        assert_eq!(wire::system_code(result), Some(5));
        let result = Refusal::from(Failure::unsupported()).to_result();
        assert_eq!(wire::status(result), wire::STATUS_UNSUPPORTED);
        assert_eq!(wire::system_code(result), None);
        for (refusal, status) in [
            (Refusal::not_a_terminal(), wire::STATUS_NOT_A_TERMINAL),
            (Refusal::in_use(), wire::STATUS_IN_USE),
            (Refusal::unsupported(), wire::STATUS_UNSUPPORTED),
            (Refusal::device_failure(), wire::STATUS_DEVICE_FAILURE),
        ] {
            assert_eq!(wire::status(refusal.to_result()), status);
        }
    }

    #[test]
    fn an_output_class_survives_its_cached_form() {
        for class in [
            OutputClass::NOT_A_TERMINAL,
            OutputClass {
                terminal: true,
                sequences: false,
                color: ColorSupport::None,
            },
            OutputClass {
                terminal: true,
                sequences: true,
                color: ColorSupport::TrueColor,
            },
            OutputClass {
                terminal: true,
                sequences: true,
                color: ColorSupport::Basic,
            },
        ] {
            assert_ne!(class.to_bits(), 0);
            assert_eq!(OutputClass::from_bits(class.to_bits()), class);
        }
    }

    #[test]
    fn undo_writes_only_what_the_program_changed() {
        assert!(Undo::new(0, true).as_bytes().is_empty());
        assert!(Undo::new(ledger::RAW_MODE | ledger::OUTPUT_MODE, true)
            .as_bytes()
            .is_empty());
        assert_eq!(
            Undo::new(ledger::CURSOR_HIDDEN, false).as_bytes(),
            b"\x1b[?25h"
        );
        assert_eq!(
            Undo::new(ledger::RENDERED, false).as_bytes(),
            b"\x1b[?1049l\x1b[?7h\x1b[?25h"
        );
        assert_eq!(
            Undo::new(ledger::RENDERED | ledger::RAW_MODE, true).as_bytes(),
            b"\x18\x1b[0m\x1b[?1049l\x1b[?7h\x1b[?25h"
        );
        assert!(Undo::again(ledger::RAW_MODE).as_bytes().is_empty());
        assert_eq!(
            Undo::again(ledger::RENDERED).as_bytes(),
            b"\x1b[?1049h\x1b[?7l\x1b[?25l"
        );
    }
}
