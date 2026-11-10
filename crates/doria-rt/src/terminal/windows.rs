//! Windows: console modes for the input and output modes, input records for
//! keys and resizes, and the same renderer and decoder as everywhere else.
//! The console turns keys into the terminal sequences the shared decoder
//! reads, and accepts the sequences the shared renderer writes. A console too
//! old to do either refuses the mode, and the operation reports unsupported.
use core::mem;
use core::sync::atomic::{AtomicU32, Ordering};

use super::queue::{Queue, Utf16, READ_CAPACITY};
use super::wire::{ColorSupport, Observation};
use super::{ledger, Exclusive, OutputClass, Refusal, Shared};
use crate::cancellation::{CancellationClaim, DrCancellationV1};
use crate::cancellation_signal;
use crate::device_io::{byte_handle, standard_device, StandardStream};
use crate::native_handle::NativeHandle;
use crate::platform_io::{self, Failure, Reason};
use crate::wait_budget::DrWaitBudgetV1;
use crate::win32::{
    wait_for_multiple_objects, ConsoleScreenBufferInfo, GetConsoleMode, GetConsoleScreenBufferInfo,
    GetEnvironmentVariableA, InputRecord, ReadConsoleInputW, SetConsoleCtrlHandler, SetConsoleMode,
    INFINITE, WAIT_TIMEOUT,
};

const ENABLE_PROCESSED_INPUT: u32 = 0x0001;
const ENABLE_LINE_INPUT: u32 = 0x0002;
const ENABLE_ECHO_INPUT: u32 = 0x0004;
const ENABLE_WINDOW_INPUT: u32 = 0x0008;
const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;
const ENABLE_PROCESSED_OUTPUT: u32 = 0x0001;
const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;

const KEY_EVENT: u16 = 0x0001;
const WINDOW_BUFFER_SIZE_EVENT: u16 = 0x0004;
/// The Alt key. A character composed on the numeric pad arrives when Alt is
/// released, on a key-up record.
const VK_MENU: u16 = 0x12;
/// What a console that does not know a mode bit answers.
const ERROR_INVALID_PARAMETER: i64 = 87;
const RECORDS_PER_READ: usize = 32;

struct Raw {
    queue: Queue,
    units: Utf16,
    /// A size record arrived. Any number of them are one event carrying the
    /// size the window has when it is reported.
    resized: bool,
}

static RAW: Shared<Option<Raw>> = Shared::new(None);
/// Valid while the ledger holds `RAW_MODE`.
static ORIGINAL_INPUT_MODE: AtomicU32 = AtomicU32::new(0);
/// Valid while the ledger holds `OUTPUT_MODE`.
static ORIGINAL_OUTPUT_MODE: AtomicU32 = AtomicU32::new(0);

fn exhausted() -> Failure {
    Failure {
        reason: Reason::ResourceExhausted,
        system_code: None,
        transferred_byte_count: None,
    }
}

/// A console that refuses a mode it does not know is one this layer cannot
/// drive, which is a different answer from a console that failed.
fn mode_refusal(failure: Failure) -> Refusal {
    if failure.system_code == Some(ERROR_INVALID_PARAMETER) {
        Refusal::unsupported()
    } else {
        failure.into()
    }
}

unsafe fn no_color() -> bool {
    // The length alone answers: zero is unset or empty.
    let mut value = [0_u8; 2];
    GetEnvironmentVariableA(c"NO_COLOR".as_ptr().cast(), value.as_mut_ptr(), 2) != 0
}

pub(super) unsafe fn classify_output() -> OutputClass {
    let Ok(output) = byte_handle(StandardStream::Stdout) else {
        return OutputClass::NOT_A_TERMINAL;
    };
    let mut mode = 0;
    if GetConsoleMode(output, &mut mode) == 0 {
        return OutputClass::NOT_A_TERMINAL;
    }
    let wanted = mode | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING;
    let sequences = if wanted == mode {
        true
    } else {
        ORIGINAL_OUTPUT_MODE.store(mode, Ordering::Release);
        super::mark(ledger::OUTPUT_MODE);
        let accepted = SetConsoleMode(output, wanted) != 0;
        if !accepted {
            super::unmark(ledger::OUTPUT_MODE);
        }
        accepted
    };
    OutputClass {
        terminal: true,
        sequences,
        // A console that takes the sequences takes all of them.
        color: if sequences && !no_color() {
            ColorSupport::TrueColor
        } else {
            ColorSupport::None
        },
    }
}

unsafe fn screen() -> Result<ConsoleScreenBufferInfo, Failure> {
    let output = byte_handle(StandardStream::Stdout)?;
    let mut info = mem::zeroed::<ConsoleScreenBufferInfo>();
    if GetConsoleScreenBufferInfo(output, &mut info) == 0 {
        return Err(platform_io::last_failure());
    }
    Ok(info)
}

/// The window, not the scrollback buffer behind it.
pub(super) unsafe fn size() -> Result<(u32, u32), Refusal> {
    let window = screen()?.window;
    let columns = i32::from(window.right) - i32::from(window.left) + 1;
    let rows = i32::from(window.bottom) - i32::from(window.top) + 1;
    if columns <= 0 || rows <= 0 {
        return Err(Refusal::unsupported());
    }
    Ok((columns as u32, rows as u32))
}

pub(super) unsafe fn enter_raw_mode() -> Result<(), Refusal> {
    let _exclusive = Exclusive::acquire()?;
    let Ok(input) = byte_handle(StandardStream::Stdin) else {
        return Err(Refusal::not_a_terminal());
    };
    let mut original = 0;
    if GetConsoleMode(input, &mut original) == 0 {
        return Err(Refusal::not_a_terminal());
    }
    // Raw mode owns standard input until it ends: line and byte reads are
    // refused admission by this loan, and so is a second raw mode.
    let device = standard_device(StandardStream::Stdin);
    let loan = device.try_borrow().ok_or_else(Refusal::in_use)?;

    // Input a line read took from the console and has not handed out yet
    // belongs to the event stream now. It is copied first and taken from the
    // line reader only once raw mode is on, so an entry that fails leaves
    // standard input as it was.
    let buffered = &mut *device.input();
    let length = buffered.end - buffered.start;
    let capacity = length.checked_add(READ_CAPACITY).ok_or_else(exhausted)?;
    let mut raw = Raw {
        queue: Queue::new(capacity).ok_or_else(exhausted)?,
        units: Utf16::default(),
        resized: false,
    };
    if length != 0 {
        raw.queue.push(core::slice::from_raw_parts(
            buffered.bytes.add(buffered.start),
            length,
        ))?;
    }

    // Keys arrive one at a time, unechoed and uninterpreted, Control+C
    // included, as the sequences a terminal sends. Size changes arrive in the
    // same stream.
    let mode = (original & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT))
        | ENABLE_WINDOW_INPUT
        | ENABLE_VIRTUAL_TERMINAL_INPUT;
    ORIGINAL_INPUT_MODE.store(original, Ordering::Release);
    super::mark(ledger::RAW_MODE);
    if SetConsoleMode(input, mode) == 0 {
        let failure = platform_io::last_failure();
        super::unmark(ledger::RAW_MODE);
        return Err(mode_refusal(failure));
    }
    buffered.take_buffered_input();
    *RAW.0.get() = Some(raw);
    loan.hold();
    Ok(())
}

pub(super) unsafe fn leave_raw_mode() -> Result<(), Refusal> {
    let _exclusive = Exclusive::acquire()?;
    let Some(raw) = (*RAW.0.get()).take() else {
        return Ok(());
    };
    let restored = match byte_handle(StandardStream::Stdin) {
        Ok(input) if SetConsoleMode(input, ORIGINAL_INPUT_MODE.load(Ordering::Acquire)) != 0 => {
            Ok(())
        }
        Ok(_) => Err(platform_io::last_failure()),
        Err(failure) => Err(failure),
    };
    super::unmark(ledger::RAW_MODE);

    // Keys typed but never asked for are still the program's input.
    let device = standard_device(StandardStream::Stdin);
    let unread = raw.queue.queued();
    if !unread.is_empty() {
        let input = &mut *device.input();
        if input.prepare_write_space(unread.len()).is_ok() {
            core::ptr::copy_nonoverlapping(
                unread.as_ptr(),
                input.bytes.add(input.end),
                unread.len(),
            );
            input.end += unread.len();
        }
    }
    drop(raw);
    device.end_hold();
    restored.map_err(Refusal::from)
}

/// One read of the records the console has. The input handle was signaled,
/// so this does not block.
unsafe fn read_records(raw: &mut Raw, input: NativeHandle) -> Result<(), Failure> {
    let mut records = [mem::zeroed::<InputRecord>(); RECORDS_PER_READ];
    let mut read = 0;
    if ReadConsoleInputW(
        input,
        records.as_mut_ptr(),
        RECORDS_PER_READ as u32,
        &mut read,
    ) == 0
    {
        return Err(platform_io::last_failure());
    }
    for record in &records[..read as usize] {
        match record.event_type {
            KEY_EVENT => {
                let key = record.event.key;
                if key.character == 0 || (key.key_down == 0 && key.virtual_key_code != VK_MENU) {
                    continue;
                }
                let queue = &mut raw.queue;
                let mut failure = None;
                raw.units.accept(key.character, |scalar| {
                    let mut encoded = [0_u8; 4];
                    let bytes = scalar.encode_utf8(&mut encoded).as_bytes();
                    // A held key repeats. Repeats that do not fit are dropped
                    // whole; nothing has asked for the ones already queued.
                    for _ in 0..key.repeat_count.max(1) {
                        match queue.push(bytes) {
                            Ok(true) => {}
                            Ok(false) => break,
                            Err(error) => {
                                failure = Some(error);
                                break;
                            }
                        }
                    }
                });
                if let Some(failure) = failure {
                    return Err(failure);
                }
            }
            WINDOW_BUFFER_SIZE_EVENT => raw.resized = true,
            _ => {}
        }
    }
    Ok(())
}

pub(super) unsafe fn await_event(
    budget: &DrWaitBudgetV1,
    cancellation: *mut DrCancellationV1,
) -> Result<Observation, Refusal> {
    let _exclusive = Exclusive::acquire()?;
    let Some(raw) = (*RAW.0.get()).as_mut() else {
        return Ok(Observation::Closed);
    };
    let input = byte_handle(StandardStream::Stdin)?;
    // The claim keeps the cancellation's event alive for the wait.
    let claim = CancellationClaim::acquire(cancellation)?;
    let mut handles = [input, input];
    let mut count = 1;
    if let Some(claim) = &claim {
        handles[1] = cancellation_signal::native_handle((*claim.0).signal()?);
        count = 2;
    }
    let mut expired = false;
    loop {
        // Keys already read come first, in the order they were typed.
        if let Some(key) = raw.queue.next_key()? {
            return Ok(key);
        }
        if mem::take(&mut raw.resized) {
            let (columns, rows) = size()?;
            return Ok(Observation::Resized { columns, rows });
        }
        if expired {
            return Ok(Observation::TimedOut);
        }
        let limit = raw.queue.wait_limit(budget)?;
        let milliseconds = match limit.remaining(
            1_000_000,
            u64::from(INFINITE - 1),
            crate::monotonic_clock::now,
        )? {
            (0, _) => 0,
            (1, _) => INFINITE,
            (2, milliseconds) => milliseconds as u32,
            _ => {
                expired = raw.queue.caller_expired()?;
                continue;
            }
        };
        match wait_for_multiple_objects(
            count,
            handles.as_ptr(),
            0,
            milliseconds,
            "terminal input or cancellation",
        ) {
            0 => read_records(raw, input)?,
            1 => return Ok(Observation::Cancelled),
            WAIT_TIMEOUT => expired = raw.queue.caller_expired()?,
            _ => return Err(platform_io::last_failure().into()),
        }
    }
}

/// The console knows where its cursor is; nothing is asked of the terminal
/// through the input stream here.
pub(super) unsafe fn cursor_position() -> Result<(u32, u32), Refusal> {
    let _exclusive = Exclusive::acquire()?;
    // The loan writes waiting console output first, so the answer is the
    // position after everything the program has drawn.
    drop(
        standard_device(StandardStream::Stdout)
            .try_borrow()
            .ok_or_else(Refusal::in_use)?,
    );
    let info = match screen() {
        Ok(info) => info,
        Err(_) => return Err(Refusal::not_a_terminal()),
    };
    let column = i32::from(info.cursor.x) - i32::from(info.window.left);
    let row = i32::from(info.cursor.y) - i32::from(info.window.top);
    Ok((column.max(0) as u32, row.max(0) as u32))
}

pub(super) unsafe fn write_at_process_end(bytes: &[u8]) {
    crate::device_io::write_stdout_at_process_end(bytes);
}

/// The output mode goes back after the undo sequences, which need it.
pub(super) unsafe fn restore_at_process_end(entries: u32) {
    if entries & ledger::OUTPUT_MODE != 0 {
        if let Ok(output) = byte_handle(StandardStream::Stdout) {
            SetConsoleMode(output, ORIGINAL_OUTPUT_MODE.load(Ordering::Acquire));
        }
    }
    if entries & ledger::RAW_MODE != 0 {
        if let Ok(input) = byte_handle(StandardStream::Stdin) {
            SetConsoleMode(input, ORIGINAL_INPUT_MODE.load(Ordering::Acquire));
        }
    }
}

/// Runs on a thread the console creates. Every control event here ends the
/// process unless a handler after this one says otherwise, so the terminal is
/// put back first; answering "not handled" passes the event on.
unsafe extern "system" fn on_control(_event: u32) -> i32 {
    super::restore_on_interruption(true);
    0
}

pub(super) unsafe fn watch_for_interruption() {
    SetConsoleCtrlHandler(Some(on_control), 1);
}

pub(super) unsafe fn stop_watching_for_interruption() {
    SetConsoleCtrlHandler(Some(on_control), 0);
}
