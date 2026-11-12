//! Linux and macOS: termios for the input mode, the window-size query, and
//! the resize signal woken through the shared readiness set.
use core::ffi::{c_int, c_void, CStr};
use core::mem::{self, MaybeUninit};
use core::ptr;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::queue::{Queue, READ_CAPACITY};
use super::wire::{ColorSupport, Observation};
use super::{ledger, render, Exclusive, OutputClass, Refusal, Shared, Watched};
use crate::cancellation::DrCancellationV1;
use crate::cancellation_signal::{self, Signal};
use crate::device_io::{standard_device, StandardStream, WriteOutcome};
use crate::platform_io::{self, Failure, Reason};
use crate::unix_readiness::terminal_watch::{TerminalWake, TerminalWatch};
use crate::wait_budget::DrWaitBudgetV1;

const INPUT: c_int = 0;
const OUTPUT: c_int = 1;
/// How long a terminal gets to answer a cursor-position query.
const CURSOR_REPORT_NANOSECONDS: u64 = 1_000_000_000;

struct Raw {
    watch: TerminalWatch,
    queue: Queue,
}

/// One read of whatever the terminal has. A terminal that has gone away
/// closes the stream of events; it is not a failure of the wait.
unsafe fn fill(queue: &mut Queue) -> Result<(), Failure> {
    let (space, available) = queue.space();
    if available == 0 {
        return Ok(());
    }
    loop {
        let count = libc::read(INPUT, space.cast::<c_void>(), available);
        if count > 0 {
            return queue.filled(count as usize);
        }
        if count == 0 {
            queue.closed = true;
            return Ok(());
        }
        let failure = platform_io::last_failure();
        match failure.system_code.map(|code| code as c_int) {
            Some(libc::EINTR) => continue,
            Some(libc::EAGAIN) => return Ok(()),
            Some(libc::EIO) => {
                queue.closed = true;
                return Ok(());
            }
            _ => return Err(failure),
        }
    }
}

static RAW: Shared<Option<Raw>> = Shared::new(None);
/// The input mode before raw mode, valid while the ledger holds `RAW_MODE`.
static ORIGINAL_INPUT: Shared<MaybeUninit<libc::termios>> = Shared::new(MaybeUninit::zeroed());
static RESIZE: Handled = Handled::new();
/// The wake the resize handler sets. It is made on the first entry to raw
/// mode and lives as long as the process, so a handler that another library
/// has chained behind its own can never reach a closed descriptor.
static NOTICE: AtomicUsize = AtomicUsize::new(0);
static RESIZED: AtomicBool = AtomicBool::new(false);

fn exhausted() -> Failure {
    Failure {
        reason: Reason::ResourceExhausted,
        system_code: None,
        transferred_byte_count: None,
    }
}

unsafe fn environment(name: &CStr) -> Option<&'static [u8]> {
    let value = libc::getenv(name.as_ptr());
    if value.is_null() {
        None
    } else {
        Some(CStr::from_ptr(value).to_bytes())
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// What the environment says the terminal can show. `NO_COLOR` wins over
/// everything; a terminal that names no type, or names `dumb`, gets none.
fn color_support(
    no_color: Option<&[u8]>,
    term: Option<&[u8]>,
    color_term: Option<&[u8]>,
) -> ColorSupport {
    if no_color.is_some_and(|value| !value.is_empty()) {
        return ColorSupport::None;
    }
    let term = match term {
        Some(term) if !term.is_empty() && term != b"dumb" => term,
        _ => return ColorSupport::None,
    };
    if matches!(color_term, Some(b"truecolor" | b"24bit")) || contains(term, b"direct") {
        ColorSupport::TrueColor
    } else if contains(term, b"256color") {
        ColorSupport::Indexed
    } else {
        ColorSupport::Basic
    }
}

pub(super) unsafe fn classify_output() -> OutputClass {
    if libc::isatty(OUTPUT) != 1 {
        return OutputClass::NOT_A_TERMINAL;
    }
    OutputClass {
        terminal: true,
        sequences: true,
        color: color_support(
            environment(c"NO_COLOR"),
            environment(c"TERM"),
            environment(c"COLORTERM"),
        ),
    }
}

unsafe fn window_size(descriptor: c_int) -> Result<(u32, u32), Failure> {
    let mut size = MaybeUninit::<libc::winsize>::zeroed();
    if libc::ioctl(descriptor, libc::TIOCGWINSZ as _, size.as_mut_ptr()) != 0 {
        return Err(platform_io::last_failure());
    }
    let size = size.assume_init();
    Ok((u32::from(size.ws_col), u32::from(size.ws_row)))
}

/// A terminal that reports no columns or no rows has not been told its size;
/// there is no size to give.
fn usable(size: (u32, u32)) -> Result<(u32, u32), Refusal> {
    if size.0 == 0 || size.1 == 0 {
        Err(Refusal::unsupported())
    } else {
        Ok(size)
    }
}

pub(super) unsafe fn size() -> Result<(u32, u32), Refusal> {
    usable(window_size(OUTPUT)?)
}

unsafe fn set_input_mode(mode: *const libc::termios) -> Result<(), Failure> {
    loop {
        if libc::tcsetattr(INPUT, libc::TCSANOW, mode) == 0 {
            return Ok(());
        }
        let failure = platform_io::last_failure();
        if failure.system_code != Some(i64::from(libc::EINTR)) {
            return Err(failure);
        }
    }
}

unsafe fn errno() -> *mut c_int {
    #[cfg(target_os = "linux")]
    {
        libc::__errno_location()
    }
    #[cfg(target_os = "macos")]
    {
        libc::__error()
    }
}

/// This layer's place in one signal's chain of handlers.
///
/// A signal has one disposition, and a handler that wants the one before it
/// to run as well saves it and calls it. This layer does that, and so may a
/// library the program uses, over this layer's handler. The predecessor is
/// therefore saved once, when the handler is installed, and stays the
/// predecessor for as long as the handler stands in the chain. Installing it
/// again over a handler that already calls it would save that handler as the
/// predecessor, and the two would call each other without end.
struct Handled {
    /// What the signal did before the handler was installed, valid while
    /// `standing`.
    previous: Shared<MaybeUninit<libc::sigaction>>,
    /// Set from the time the handler is installed until this layer puts
    /// `previous` back.
    standing: AtomicBool,
}

impl Handled {
    const fn new() -> Self {
        Self {
            previous: Shared::new(MaybeUninit::zeroed()),
            standing: AtomicBool::new(false),
        }
    }

    fn previous(&self) -> *const libc::sigaction {
        self.previous.0.get().cast()
    }

    /// Makes sure `action` receives `signal`. When another handler was
    /// installed over ours and is still the installed one, ours is reached
    /// through it and nothing changes. A default or ignored disposition, or
    /// the predecessor itself, means ours was taken out of the chain, and it
    /// is installed again.
    unsafe fn install(&self, signal: c_int, action: &libc::sigaction) -> Result<(), Failure> {
        let mut current = MaybeUninit::<libc::sigaction>::zeroed();
        if libc::sigaction(signal, ptr::null(), current.as_mut_ptr()) != 0 {
            return Err(platform_io::last_failure());
        }
        let current = current.assume_init();
        let installed = current.sa_sigaction;
        if self.standing.load(Ordering::Acquire) {
            if installed == action.sa_sigaction {
                return Ok(());
            }
            let over_ours = installed != libc::SIG_DFL
                && installed != libc::SIG_IGN
                && installed != (*self.previous()).sa_sigaction;
            if over_ours {
                return Ok(());
            }
        }
        // The predecessor is in place before the handler can run.
        *self.previous.0.get() = MaybeUninit::new(current);
        if libc::sigaction(signal, action, ptr::null_mut()) != 0 {
            return Err(platform_io::last_failure());
        }
        self.standing.store(true, Ordering::Release);
        Ok(())
    }

    /// Puts the predecessor back while `handler` is still the installed one.
    /// A handler installed over it since then keeps its place: ours stays in
    /// the chain under it, safe to reach, with its predecessor unchanged.
    unsafe fn remove(&self, signal: c_int, handler: libc::sighandler_t) {
        if !self.standing.load(Ordering::Acquire) {
            return;
        }
        let mut current = MaybeUninit::<libc::sigaction>::zeroed();
        if libc::sigaction(signal, ptr::null(), current.as_mut_ptr()) == 0
            && current.assume_init().sa_sigaction == handler
            && libc::sigaction(signal, self.previous(), ptr::null_mut()) == 0
        {
            self.standing.store(false, Ordering::Release);
        }
    }

    /// The handler itself gave the signal back to its predecessor.
    fn gave_way(&self) {
        self.standing.store(false, Ordering::Release);
    }
}

unsafe fn chain(
    previous: *const libc::sigaction,
    signal: c_int,
    info: *mut libc::siginfo_t,
    context: *mut c_void,
) {
    let handler = (*previous).sa_sigaction;
    if handler == libc::SIG_DFL || handler == libc::SIG_IGN {
        return;
    }
    if (*previous).sa_flags & libc::SA_SIGINFO != 0 {
        let handler: extern "C" fn(c_int, *mut libc::siginfo_t, *mut c_void) =
            mem::transmute(handler);
        handler(signal, info, context);
    } else {
        let handler: extern "C" fn(c_int) = mem::transmute(handler);
        handler(signal);
    }
}

/// Only atomics and one wake: everything else waits for the event call.
extern "C" fn on_resize(signal: c_int, info: *mut libc::siginfo_t, context: *mut c_void) {
    unsafe {
        let location = errno();
        let saved = *location;
        // The flag is set before the wake so a waiter that resets the wake
        // and then reads the flag cannot miss a resize between the two.
        RESIZED.store(true, Ordering::SeqCst);
        let notice = NOTICE.load(Ordering::Acquire);
        if notice != 0 {
            let _ = cancellation_signal::set(notice);
        }
        chain(RESIZE.previous(), signal, info, context);
        *location = saved;
    }
}

fn resize_handler() -> libc::sighandler_t {
    on_resize as *const () as libc::sighandler_t
}

unsafe fn install_resize_handler() -> Result<(), Failure> {
    let mut action: libc::sigaction = mem::zeroed();
    action.sa_sigaction = resize_handler();
    action.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART;
    libc::sigemptyset(&mut action.sa_mask);
    RESIZE.install(libc::SIGWINCH, &action)
}

unsafe fn remove_resize_handler() {
    RESIZE.remove(libc::SIGWINCH, resize_handler());
}

/// The signals that end the process by default, then stop and continue.
const INTERRUPTIONS: [c_int; 6] = [
    libc::SIGINT,
    libc::SIGTERM,
    libc::SIGHUP,
    libc::SIGQUIT,
    libc::SIGTSTP,
    libc::SIGCONT,
];
const STOP: usize = 4;

static INTERRUPTION: [Handled; 6] = [const { Handled::new() }; 6];
/// Which of [`INTERRUPTIONS`] this layer is watching: every one that was not
/// ignored when the program came to have something to put back.
static INTERRUPTION_WATCHED: [AtomicBool; 6] = [
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
    AtomicBool::new(false),
];

fn interruption_handler() -> libc::sighandler_t {
    on_interruption as *const () as libc::sighandler_t
}

/// Set by the stop handler once it has undone the program's changes, and
/// cleared by whichever handler makes them again.
static GAVE_TERMINAL_BACK: AtomicBool = AtomicBool::new(false);
/// Held by the one handler that is undoing the changes or making them again.
static TAKING_TERMINAL: AtomicBool = AtomicBool::new(false);
/// Set from the moment a stop starts undoing the changes until its end has
/// made them again.
static STOPPING: AtomicBool = AtomicBool::new(false);
/// What stood in the stop signal's way when the stop handler lent the signal
/// to its predecessor, valid while `STOP_LENT`: this layer's handler, or one
/// installed over it.
static STOP_DISPLACED: Shared<MaybeUninit<libc::sigaction>> = Shared::new(MaybeUninit::zeroed());
static STOP_LENT: AtomicBool = AtomicBool::new(false);

/// The handlers block one another while they work, except that a continue
/// may arrive inside the stop handler, which is where a stopped process is.
unsafe fn interruption_action() -> libc::sigaction {
    let mut action: libc::sigaction = mem::zeroed();
    action.sa_sigaction = interruption_handler();
    action.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART;
    libc::sigemptyset(&mut action.sa_mask);
    for signal in INTERRUPTIONS {
        if signal != libc::SIGCONT {
            libc::sigaddset(&mut action.sa_mask, signal);
        }
    }
    action
}

/// A process in the background does not own the terminal: the shell does,
/// and changing its modes from here would either stop this process again or
/// wreck the shell's prompt. A terminal that is not this process's
/// controlling terminal has no such owner.
unsafe fn in_foreground() -> bool {
    let descriptor = if libc::isatty(INPUT) == 1 {
        INPUT
    } else {
        OUTPUT
    };
    let owner = libc::tcgetpgrp(descriptor);
    owner < 0 || owner == libc::getpgrp()
}

fn same_mode(left: &libc::termios, right: &libc::termios) -> bool {
    left.c_iflag == right.c_iflag
        && left.c_oflag == right.c_oflag
        && left.c_cflag == right.c_cflag
        && left.c_lflag == right.c_lflag
        && left.c_cc == right.c_cc
}

/// Makes the program's changes again on a terminal a shell has had in the
/// meantime. It makes only the ones that are missing, because a continue and
/// the end of a stop can both arrive here, on different threads and in
/// either order: the sequences are written once for each time they were
/// undone, and the input mode is set only when it is not already raw. That
/// last check also covers a stop that cannot be caught, after which a shell
/// may have put its own input mode on the terminal.
///
/// The screen is whatever the shell left on it, so a resize event is
/// flagged: that is what tells a program to draw again. In the background
/// this does nothing, and the continue that brings the program forward does
/// it then.
///
/// A continue and the end of a stop can both arrive here, on different
/// threads, in either order, and a continue can arrive late: while the next
/// stop is already being honored. So one arrival works at a time, and while
/// a stop is being honored only its own end takes the terminal. The end of a
/// stop waits its turn, because the program runs on from there and must find
/// its terminal ready. A continue never waits: it may have interrupted the
/// very thread that is doing the work.
unsafe fn take_terminal_again(stop_ended: bool) {
    while TAKING_TERMINAL.swap(true, Ordering::AcqRel) {
        if !stop_ended {
            return;
        }
        core::hint::spin_loop();
    }
    if stop_ended || !STOPPING.load(Ordering::SeqCst) {
        // Stopping lent the stop signal to its predecessor. What stood in
        // its way comes back: a handler installed over ours always, and ours
        // while the program still has something to put back.
        if STOP_LENT.swap(false, Ordering::AcqRel) {
            let displaced = STOP_DISPLACED.0.get().cast::<libc::sigaction>();
            if (*displaced).sa_sigaction != interruption_handler()
                || INTERRUPTION_WATCHED[STOP].load(Ordering::Acquire)
            {
                libc::sigaction(libc::SIGTSTP, displaced, ptr::null_mut());
            } else {
                INTERRUPTION[STOP].gave_way();
            }
        }
        if in_foreground() {
            make_changes_again();
        }
    }
    if stop_ended {
        STOPPING.store(false, Ordering::SeqCst);
    }
    TAKING_TERMINAL.store(false, Ordering::Release);
}

/// The stop handler's half: the program's changes are undone and the stop
/// signal is lent to its predecessor, as one step that no continue can
/// interleave with. A late continue would otherwise put a handler back in
/// the signal's way just before the stop is raised.
unsafe fn give_terminal_back(previous: *const libc::sigaction) {
    while TAKING_TERMINAL.swap(true, Ordering::AcqRel) {
        core::hint::spin_loop();
    }
    STOPPING.store(true, Ordering::SeqCst);
    if in_foreground() {
        super::restore_on_interruption(false);
        GAVE_TERMINAL_BACK.store(true, Ordering::SeqCst);
    }
    let mut displaced = MaybeUninit::<libc::sigaction>::zeroed();
    if libc::sigaction(libc::SIGTSTP, previous, displaced.as_mut_ptr()) == 0
        && !STOP_LENT.load(Ordering::Acquire)
    {
        *STOP_DISPLACED.0.get() = displaced;
        STOP_LENT.store(true, Ordering::Release);
    }
    TAKING_TERMINAL.store(false, Ordering::Release);
}

unsafe fn make_changes_again() {
    let entries = super::changes();
    let mut changed = false;
    if entries & ledger::RAW_MODE != 0 {
        let raw = raw_mode((*ORIGINAL_INPUT.0.get()).assume_init());
        let mut current = MaybeUninit::<libc::termios>::zeroed();
        if libc::tcgetattr(INPUT, current.as_mut_ptr()) != 0
            || !same_mode(&current.assume_init(), &raw)
        {
            let _ = set_input_mode(&raw);
            changed = true;
        }
    }
    if GAVE_TERMINAL_BACK.swap(false, Ordering::SeqCst) {
        write_at_process_end(super::Undo::again(entries).as_bytes());
        changed = true;
    }
    if changed {
        RESIZED.store(true, Ordering::SeqCst);
        let notice = NOTICE.load(Ordering::Acquire);
        if notice != 0 {
            let _ = cancellation_signal::set(notice);
        }
    }
}

/// Async-signal-safe throughout: atomics, `write`, `tcsetattr`, `sigaction`,
/// `pthread_sigmask` and `raise`.
extern "C" fn on_interruption(signal: c_int, info: *mut libc::siginfo_t, context: *mut c_void) {
    unsafe {
        let location = errno();
        let saved = *location;
        let Some(index) = INTERRUPTIONS.iter().position(|watched| *watched == signal) else {
            return;
        };
        let previous = INTERRUPTION[index].previous();
        match signal {
            libc::SIGCONT => {
                take_terminal_again(false);
                chain(previous, signal, info, context);
            }
            libc::SIGTSTP => {
                give_terminal_back(previous);
                // The stop happens here, under the signal's previous
                // disposition, and this handler is where the program finds
                // itself when it runs again: continued, or never stopped
                // because the system discards a stop for a process group
                // with nobody to continue it. Either way the terminal is the
                // program's again.
                let mut stop: libc::sigset_t = mem::zeroed();
                let mut blocked: libc::sigset_t = mem::zeroed();
                libc::sigemptyset(&mut stop);
                libc::sigaddset(&mut stop, signal);
                libc::pthread_sigmask(libc::SIG_UNBLOCK, &stop, &mut blocked);
                libc::raise(signal);
                libc::pthread_sigmask(libc::SIG_SETMASK, &blocked, ptr::null_mut());
                take_terminal_again(true);
            }
            _ => {
                if in_foreground() {
                    super::restore_on_interruption(true);
                }
                // The signal goes on to do what it would have done without
                // this layer: it is delivered again, under its previous
                // disposition, as soon as this handler returns.
                if libc::sigaction(signal, previous, ptr::null_mut()) == 0 {
                    INTERRUPTION[index].gave_way();
                }
                libc::raise(signal);
            }
        }
        *location = saved;
    }
}

/// A signal the program's parent chose to ignore stays ignored: taking it
/// would turn a signal that did nothing into one that ends the program.
pub(super) unsafe fn watch_for_interruption() {
    for (index, signal) in INTERRUPTIONS.into_iter().enumerate() {
        let mut current = MaybeUninit::<libc::sigaction>::zeroed();
        if libc::sigaction(signal, ptr::null(), current.as_mut_ptr()) != 0
            || current.assume_init().sa_sigaction == libc::SIG_IGN
        {
            continue;
        }
        if INTERRUPTION[index]
            .install(signal, &interruption_action())
            .is_ok()
        {
            INTERRUPTION_WATCHED[index].store(true, Ordering::Release);
        }
    }
}

pub(super) unsafe fn stop_watching_for_interruption() {
    for (index, signal) in INTERRUPTIONS.into_iter().enumerate() {
        if INTERRUPTION_WATCHED[index].swap(false, Ordering::AcqRel) {
            INTERRUPTION[index].remove(signal, interruption_handler());
        }
    }
}

fn notice() -> Result<usize, Failure> {
    let notice = NOTICE.load(Ordering::Acquire);
    if notice != 0 {
        return Ok(notice);
    }
    let created = Signal::new()?.into_raw();
    NOTICE.store(created, Ordering::Release);
    Ok(created)
}

/// Keys arrive one at a time, unechoed and uninterpreted: no line editing, no
/// signal keys, no flow control, no carriage-return translation. Output
/// processing is left alone, because raw mode changes input only.
fn raw_mode(mut mode: libc::termios) -> libc::termios {
    mode.c_iflag &= !(libc::IGNBRK
        | libc::BRKINT
        | libc::PARMRK
        | libc::INPCK
        | libc::ISTRIP
        | libc::INLCR
        | libc::IGNCR
        | libc::ICRNL
        | libc::IXON);
    mode.c_lflag &= !(libc::ECHO | libc::ECHONL | libc::ICANON | libc::ISIG | libc::IEXTEN);
    mode.c_cflag &= !(libc::CSIZE | libc::PARENB);
    mode.c_cflag |= libc::CS8;
    mode.c_cc[libc::VMIN] = 1;
    mode.c_cc[libc::VTIME] = 0;
    mode
}

pub(super) unsafe fn enter_raw_mode() -> Result<(), Refusal> {
    let _exclusive = Exclusive::acquire()?;
    if libc::isatty(INPUT) != 1 {
        return Err(Refusal::not_a_terminal());
    }
    // Raw mode owns standard input until it ends: line and byte reads are
    // refused admission by this loan, and so is a second raw mode.
    let device = standard_device(StandardStream::Stdin);
    let loan = device.try_borrow().ok_or_else(Refusal::in_use)?;
    let original = (*ORIGINAL_INPUT.0.get()).as_mut_ptr();
    if libc::tcgetattr(INPUT, original) != 0 {
        return Err(platform_io::last_failure().into());
    }

    // Input a line read took from the terminal and has not handed out yet
    // belongs to the event stream now, ahead of anything still unread. It is
    // copied first and taken from the line reader only once raw mode is on,
    // so an entry that fails leaves standard input as it was.
    let input = &mut *device.input();
    let buffered = input.end - input.start;
    let capacity = buffered.checked_add(READ_CAPACITY).ok_or_else(exhausted)?;
    let watch = TerminalWatch::new(INPUT, notice()?)?;
    let mut raw = Raw {
        watch,
        queue: Queue::new(capacity).ok_or_else(exhausted)?,
    };
    if buffered != 0 {
        raw.queue.push(core::slice::from_raw_parts(
            input.bytes.add(input.start),
            buffered,
        ))?;
    }
    install_resize_handler()?;

    super::mark(ledger::RAW_MODE);
    if let Err(failure) = set_input_mode(&raw_mode(*original)) {
        super::unmark(ledger::RAW_MODE);
        remove_resize_handler();
        return Err(failure.into());
    }
    input.take_buffered_input();
    RESIZED.store(false, Ordering::SeqCst);
    *RAW.0.get() = Some(raw);
    loan.hold();
    Ok(())
}

pub(super) unsafe fn leave_raw_mode() -> Result<(), Refusal> {
    let _exclusive = Exclusive::acquire()?;
    let Some(raw) = (*RAW.0.get()).take() else {
        return Ok(());
    };
    let restored = set_input_mode((*ORIGINAL_INPUT.0.get()).as_ptr());
    super::unmark(ledger::RAW_MODE);
    remove_resize_handler();

    // Keys typed but never asked for are still the program's input.
    let device = standard_device(StandardStream::Stdin);
    let unread = raw.queue.queued();
    if !unread.is_empty() {
        let input = &mut *device.input();
        if input.prepare_write_space(unread.len()).is_ok() {
            ptr::copy_nonoverlapping(unread.as_ptr(), input.bytes.add(input.end), unread.len());
            input.end += unread.len();
        }
    }
    drop(raw);
    device.end_hold();
    restored.map_err(Refusal::from)
}

unsafe fn resized() -> Result<Observation, Refusal> {
    let size = match window_size(OUTPUT) {
        Ok(size) => size,
        // Standard output may be redirected while the keyboard is a terminal.
        Err(_) => window_size(INPUT)?,
    };
    let (columns, rows) = usable(size)?;
    Ok(Observation::Resized { columns, rows })
}

pub(super) unsafe fn await_event(
    budget: &DrWaitBudgetV1,
    cancellation: *mut DrCancellationV1,
) -> Result<Observation, Refusal> {
    let _exclusive = Exclusive::acquire()?;
    let Some(raw) = (*RAW.0.get()).as_mut() else {
        return Ok(Observation::Closed);
    };
    let mut expired = false;
    loop {
        // Keys already read come first, in the order they were typed.
        if let Some(key) = raw.queue.next_key()? {
            return Ok(key);
        }
        // Any number of resizes since the last report are one event carrying
        // the size the terminal has now.
        if RESIZED.swap(false, Ordering::SeqCst) {
            return resized();
        }
        if raw.queue.closed {
            return Ok(Observation::Closed);
        }
        if expired {
            return Ok(Observation::TimedOut);
        }
        match raw
            .watch
            .wait(&raw.queue.wait_limit(budget)?, cancellation)?
        {
            TerminalWake::Ready { input, notice } => {
                if notice {
                    cancellation_signal::reset(NOTICE.load(Ordering::Acquire))?;
                }
                if input {
                    fill(&mut raw.queue)?;
                }
            }
            TerminalWake::TimedOut => expired = raw.queue.caller_expired()?,
            TerminalWake::Cancelled => return Ok(Observation::Cancelled),
        }
    }
}

/// Whether [`await_event`] with an immediate wait would observe an event.
/// Input the terminal already has is read into the queue; nothing is taken
/// out of it and the resize flag is left set.
pub(super) unsafe fn events_waiting() -> Result<Watched, Refusal> {
    let _exclusive = Exclusive::acquire()?;
    let Some(raw) = (*RAW.0.get()).as_mut() else {
        return Ok(Watched::Ready);
    };
    let immediate = DrWaitBudgetV1::new(0, 0, || Ok(0))?;
    loop {
        if raw.queue.has_key()? || RESIZED.load(Ordering::SeqCst) || raw.queue.closed {
            return Ok(Watched::Ready);
        }
        match raw.watch.wait(&immediate, ptr::null_mut())? {
            TerminalWake::Ready { input, notice } => {
                if notice {
                    cancellation_signal::reset(NOTICE.load(Ordering::Acquire))?;
                }
                if input {
                    fill(&mut raw.queue)?;
                }
            }
            TerminalWake::TimedOut | TerminalWake::Cancelled => {
                return Ok(Watched::Waiting {
                    until: raw.queue.held_until(),
                });
            }
        }
    }
}

/// The terminal watch itself: one descriptor that is readable while standard
/// input or the resize notice is.
pub(super) unsafe fn event_handle() -> Option<c_int> {
    (*RAW.0.get())
        .as_ref()
        .and_then(|raw| raw.watch.handle().ok())
}

pub(super) unsafe fn cursor_position() -> Result<(u32, u32), Refusal> {
    let _exclusive = Exclusive::acquire()?;
    let Some(raw) = (*RAW.0.get()).as_mut() else {
        return Err(Refusal::in_use());
    };
    {
        // The loan writes waiting console output first, so the answer is the
        // position after everything the program has drawn.
        let output = standard_device(StandardStream::Stdout)
            .try_borrow()
            .ok_or_else(Refusal::in_use)?;
        if libc::isatty(OUTPUT) != 1 {
            return Err(Refusal::not_a_terminal());
        }
        let query = render::CURSOR_POSITION_QUERY;
        match output.write_text(query.as_ptr(), query.len()) {
            WriteOutcome::Success => {}
            WriteOutcome::BrokenPipe => crate::exit_process(0),
            WriteOutcome::OtherFailure(failure) => return Err(failure.into()),
        }
    }
    let budget = DrWaitBudgetV1::new(2, CURSOR_REPORT_NANOSECONDS, crate::monotonic_clock::now)?;
    loop {
        if let Some(position) = raw.queue.take_cursor_report() {
            return Ok(position);
        }
        if raw.queue.closed {
            return Err(Refusal::device_failure());
        }
        if raw.queue.space().1 == 0 {
            return Err(Refusal::unsupported());
        }
        match raw.watch.wait(&budget, ptr::null_mut())? {
            TerminalWake::Ready { input, notice } => {
                // The resize itself stays flagged for the next event wait.
                if notice {
                    cancellation_signal::reset(NOTICE.load(Ordering::Acquire))?;
                }
                if input {
                    fill(&mut raw.queue)?;
                }
            }
            // A terminal that does not answer does not support the query.
            TerminalWake::TimedOut | TerminalWake::Cancelled => return Err(Refusal::unsupported()),
        }
    }
}

/// Async-signal-safe: one `write` loop, no allocation, no locks.
pub(super) unsafe fn write_at_process_end(bytes: &[u8]) {
    let mut written = 0;
    while written < bytes.len() {
        let count = libc::write(
            OUTPUT,
            bytes.as_ptr().add(written).cast::<c_void>(),
            bytes.len() - written,
        );
        if count > 0 {
            written += count as usize;
        } else if count == 0 || *errno() != libc::EINTR {
            return;
        }
    }
}

/// Async-signal-safe: `tcsetattr` only.
pub(super) unsafe fn restore_at_process_end(entries: u32) {
    if entries & ledger::RAW_MODE != 0 {
        let _ = set_input_mode((*ORIGINAL_INPUT.0.get()).as_ptr());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_decides_color_support() {
        fn support(
            no_color: Option<&str>,
            term: Option<&str>,
            color_term: Option<&str>,
        ) -> ColorSupport {
            color_support(
                no_color.map(str::as_bytes),
                term.map(str::as_bytes),
                color_term.map(str::as_bytes),
            )
        }
        assert_eq!(support(None, None, None), ColorSupport::None);
        assert_eq!(support(None, Some(""), None), ColorSupport::None);
        assert_eq!(
            support(None, Some("dumb"), Some("truecolor")),
            ColorSupport::None
        );
        assert_eq!(support(None, Some("xterm"), None), ColorSupport::Basic);
        assert_eq!(
            support(None, Some("xterm-256color"), None),
            ColorSupport::Indexed
        );
        assert_eq!(
            support(None, Some("xterm-256color"), Some("truecolor")),
            ColorSupport::TrueColor
        );
        assert_eq!(
            support(None, Some("xterm"), Some("24bit")),
            ColorSupport::TrueColor
        );
        assert_eq!(
            support(None, Some("xterm-direct"), None),
            ColorSupport::TrueColor
        );
        // An empty NO_COLOR is not a request for no color.
        assert_eq!(
            support(Some(""), Some("xterm-256color"), None),
            ColorSupport::Indexed
        );
        assert_eq!(
            support(Some("1"), Some("xterm-256color"), Some("truecolor")),
            ColorSupport::None
        );
    }

    #[test]
    fn a_terminal_without_a_size_has_none_to_report() {
        assert_eq!(usable((80, 24)), Ok((80, 24)));
        assert_eq!(usable((0, 24)), Err(Refusal::unsupported()));
        assert_eq!(usable((80, 0)), Err(Refusal::unsupported()));
    }
}
