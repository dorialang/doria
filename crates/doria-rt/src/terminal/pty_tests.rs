//! The terminal layer against a real pseudo-terminal. Each test is the
//! terminal: it holds the pty's master side, runs this test binary again as a
//! child with the slave side as its standard input and output, types at it,
//! resizes it and reads exactly what it drew. The child calls the same entry
//! points compiled Doria code calls.
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};
use std::vec::Vec;
use std::{eprintln, format, string::String};

use super::wire::{self, operation, Color, Key, Observation, Style};
use super::{dr_v1_host_terminal_operate, dr_v1_terminal_await_event, dr_v1_terminal_operate};
use crate::device_io::StandardStream;

const SCENARIO: &str = "DORIA_RT_TERMINAL_SCENARIO";
const PATIENCE: Duration = Duration::from_secs(20);

struct Terminal {
    master: OwnedFd,
    /// The side handed to the child as its standard input and output.
    slave: OwnedFd,
    drawn: Vec<u8>,
}

fn same_mode(left: &libc::termios, right: &libc::termios) -> bool {
    left.c_iflag == right.c_iflag
        && left.c_oflag == right.c_oflag
        && left.c_cflag == right.c_cflag
        && left.c_lflag == right.c_lflag
        && left.c_cc == right.c_cc
}

fn input_mode(descriptor: i32) -> libc::termios {
    unsafe {
        let mut mode = core::mem::MaybeUninit::<libc::termios>::zeroed();
        assert_eq!(
            libc::tcgetattr(descriptor, mode.as_mut_ptr()),
            0,
            "{}",
            std::io::Error::last_os_error()
        );
        mode.assume_init()
    }
}

impl Terminal {
    fn open(columns: u16, rows: u16) -> Self {
        let size = libc::winsize {
            ws_row: rows,
            ws_col: columns,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let (mut master, mut slave) = (-1, -1);
        // `openpty` names the slave through a static buffer on macOS, so two
        // tests opening terminals at once can be handed each other's.
        static OPENING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _opening = OPENING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        unsafe {
            assert_eq!(
                libc::openpty(
                    &mut master,
                    &mut slave,
                    core::ptr::null_mut(),
                    core::ptr::null_mut(),
                    &size as *const libc::winsize as *mut libc::winsize,
                ),
                0
            );
            let flags = libc::fcntl(master, libc::F_GETFL);
            assert_eq!(
                libc::fcntl(master, libc::F_SETFL, flags | libc::O_NONBLOCK),
                0
            );
            Self {
                master: OwnedFd::from_raw_fd(master),
                slave: OwnedFd::from_raw_fd(slave),
                drawn: Vec::new(),
            }
        }
    }

    /// The child gets the pty as its controlling terminal, so resizing the
    /// pty signals it the way a real terminal emulator's resize does.
    fn run(&self, scenario: &str) -> Child {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "terminal::pty_tests::child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(SCENARIO, scenario)
            .env("TERM", "xterm-256color")
            .env_remove("NO_COLOR")
            .env_remove("COLORTERM")
            .stdin(Stdio::from(self.slave.try_clone().unwrap()))
            .stdout(Stdio::from(self.slave.try_clone().unwrap()))
            .stderr(Stdio::piped());
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                // A test run started in the background inherits ignored
                // interrupt and quit signals. A program started from a
                // terminal has the defaults, and that is what is under test.
                for signal in [
                    libc::SIGINT,
                    libc::SIGQUIT,
                    libc::SIGTERM,
                    libc::SIGHUP,
                    libc::SIGTSTP,
                ] {
                    libc::signal(signal, libc::SIG_DFL);
                }
                Ok(())
            });
        }
        command.spawn().unwrap()
    }

    /// Read through the master side: once a child that held the pty as its
    /// controlling terminal exits, every other slave descriptor is revoked.
    fn mode(&self) -> libc::termios {
        input_mode(self.master.as_raw_fd())
    }

    fn type_bytes(&self, bytes: &[u8]) {
        let written =
            unsafe { libc::write(self.master.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
        assert_eq!(written, bytes.len() as isize);
    }

    fn resize(&self, columns: u16, rows: u16) {
        let size = libc::winsize {
            ws_row: rows,
            ws_col: columns,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        assert_eq!(
            unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ as _, &size) },
            0
        );
    }

    /// Reads what the child has drawn for up to `wait`. False when nothing
    /// arrived in that time.
    fn read_for(&mut self, wait: Duration) -> bool {
        let mut ready = libc::pollfd {
            fd: self.master.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut ready, 1, wait.as_millis() as i32) } <= 0 {
            return false;
        }
        let mut bytes = [0_u8; 4096];
        let count = unsafe {
            libc::read(
                self.master.as_raw_fd(),
                bytes.as_mut_ptr().cast(),
                bytes.len(),
            )
        };
        if count <= 0 {
            return false;
        }
        self.drawn.extend_from_slice(&bytes[..count as usize]);
        true
    }

    /// Waits until the child has drawn `marker`, and returns what follows it.
    fn wait_for(&mut self, child: &mut Child, marker: &[u8]) -> usize {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Some(found) = find(&self.drawn, marker) {
                return found + marker.len();
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let mut diagnostics = String::new();
                let _ = child
                    .stderr
                    .take()
                    .unwrap()
                    .read_to_string(&mut diagnostics);
                panic!(
                    "the child never drew {:?}; it drew {:?} and reported:\n{diagnostics}",
                    String::from_utf8_lossy(marker),
                    String::from_utf8_lossy(&self.drawn),
                );
            }
            self.read_for(Duration::from_millis(50));
        }
    }

    /// Everything the child drew, once it has exited and the pty is quiet.
    /// A terminal keeps reading while a program exits: a process cannot
    /// finish closing a terminal that still holds its unread output.
    fn finish(&mut self, child: Child) -> (Option<i32>, String) {
        let (status, report) = self.finish_with_status(child);
        (status.code(), report)
    }

    fn finish_with_status(&mut self, mut child: Child) -> (ExitStatus, String) {
        let deadline = Instant::now() + PATIENCE;
        let status = loop {
            self.read_for(Duration::from_millis(20));
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                let mut diagnostics = String::new();
                let _ = child
                    .stderr
                    .take()
                    .unwrap()
                    .read_to_string(&mut diagnostics);
                panic!(
                    "the child never exited; it drew {:?} and reported:\n{diagnostics}",
                    String::from_utf8_lossy(&self.drawn),
                );
            }
        };
        while self.read_for(Duration::from_millis(100)) {}
        let mut report = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut report)
            .unwrap();
        (status, report)
    }

    /// What was drawn after `start`, up to and including `end`. A child
    /// that returns from its test is followed by the harness's own report.
    fn drawn_between(&self, start: &[u8], end: &[u8]) -> &[u8] {
        let drawn = self.drawn_after(start);
        let stop = find(drawn, end).expect("end marker was drawn") + end.len();
        &drawn[..stop]
    }

    fn drawn_after(&self, marker: &[u8]) -> &[u8] {
        let start = find(&self.drawn, marker).expect("marker was drawn") + marker.len();
        &self.drawn[start..]
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn printable(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace('\x1b', "ESC")
}

// ---- What the child does -------------------------------------------------

type Refused = (u8, Option<i64>);

unsafe fn operate(operation: u32, first: i64, second: i64, text: &[u8]) -> Result<u64, Refused> {
    let result = dr_v1_host_terminal_operate(operation, first, second, text.as_ptr(), text.len());
    if wire::status(result) == wire::STATUS_OK {
        Ok(wire::payload(result))
    } else {
        Err((wire::status(result), wire::system_code(result)))
    }
}

unsafe fn perform(operation: u32) {
    operate(operation, 0, 0, b"").unwrap();
}

/// Draws through the entry compiled Doria uses, with a real Doria string.
unsafe fn draw(text: &[u8]) {
    let string = crate::allocate_string(text.len());
    core::ptr::copy_nonoverlapping(text.as_ptr(), crate::string_bytes_mut(string), text.len());
    let result = dr_v1_terminal_operate(operation::WRITE, 0, 0, string);
    assert_eq!(wire::status(result), wire::STATUS_OK);
    crate::dr_v1_string_release(string);
}

/// Draws a marker and makes sure the terminal has it.
unsafe fn announce(marker: &str) {
    draw(marker.as_bytes());
    perform(operation::FLUSH);
}

/// A `Doria\Std\Io\Wait` as compiled code passes it: kind and nanoseconds.
type Wait = (u8, u64);

fn indefinite() -> Wait {
    (1, 0)
}

fn within(nanoseconds: u64) -> Wait {
    (2, nanoseconds)
}

unsafe fn next_event(
    generation: u64,
    wait: Wait,
    cancellation: *mut crate::cancellation::DrCancellationV1,
) -> Observation {
    let result = dr_v1_terminal_await_event(generation as u32, wait.0, wait.1, 0, cancellation);
    assert_eq!(wire::status(result), wire::STATUS_OK, "event wait failed");
    Observation::from_payload(wire::payload(result)).unwrap()
}

unsafe fn enter_raw_mode() -> u64 {
    operate(operation::ENTER_RAW_MODE, 0, 0, b"").unwrap()
}

unsafe fn leave_raw_mode(generation: u64) {
    operate(operation::LEAVE_RAW_MODE, generation as i64, 0, b"").unwrap();
}

fn key(character: char) -> Observation {
    Observation::Key {
        key: Key::Character(character),
        modifiers: 0,
    }
}

#[test]
fn child() {
    let Ok(scenario) = std::env::var(SCENARIO) else {
        return;
    };
    unsafe {
        match scenario.as_str() {
            "frame" => child_frame(),
            "keys" => child_keys(),
            "waits" => child_waits(),
            "exit" | "panic" | "return" => child_ending(&scenario),
            "unread" => child_unread(),
            "refusals" => child_refusals(),
            "cursor" => child_cursor(),
            "redirected" => child_redirected(),
            "interrupted" => child_interrupted(),
            "terminated" => child_job(0),
            "discarded" => child_job(1),
            "job" => child_job(2),
            "suspended" => child_shell(),
            "watching" => child_watching(),
            "layered" => child_layered(false),
            "layered-terminated" => child_layered(true),
            "grouped" => child_grouped(),
            _ => panic!("unknown scenario {scenario}"),
        }
    }
}

unsafe fn child_frame() {
    announce("<frame>");
    perform(operation::HIDE_CURSOR);
    let red_bold = Style {
        foreground: Color::Named(1),
        background: Color::Default,
        attributes: wire::ATTRIBUTE_BOLD,
    };
    operate(
        operation::WRITE_AT,
        wire::cell(2, 1),
        red_bold.to_bits(),
        b"hi",
    )
    .unwrap();
    operate(operation::MOVE_TO, wire::cell(0, 3), 0, b"").unwrap();
    operate(operation::MOVE_RIGHT, 4, 0, b"").unwrap();
    operate(operation::MOVE_LEFT, 0, 0, b"").unwrap();
    perform(operation::CLEAR_LINE_FROM_CURSOR);
    operate(operation::ERASE_AT, wire::cell(7, 0), 0, b"").unwrap();
    operate(operation::SET_TITLE, 0, 0, b"a\x1bb\x07c").unwrap();
    draw(b"A");
    // An ordinary write to standard output lands after the console output
    // before it and ahead of the console output after it.
    crate::dr_v2_write_stdout(core::ptr::null(), b"B".as_ptr(), 1);
    let true_red = Style {
        foreground: Color::Rgb(255, 0, 0),
        background: Color::Default,
        attributes: 0,
    };
    operate(operation::WRITE, true_red.to_bits(), 0, b"C").unwrap();
    perform(operation::SHOW_CURSOR);
    let size = operate(operation::SIZE, 0, 0, b"").unwrap();
    eprintln!(
        "size={}x{} color={}",
        wire::pair_first(size),
        wire::pair_second(size),
        operate(operation::COLOR_SUPPORT, 0, 0, b"").unwrap()
    );
    crate::dr_v1_exit_process(0);
}

unsafe fn child_keys() {
    let before = input_mode(0);
    let generation = enter_raw_mode();
    let raw = input_mode(0);
    eprintln!(
        "canonical={} echo={} signals={} output_processing={}",
        raw.c_lflag & libc::ICANON != 0,
        raw.c_lflag & libc::ECHO != 0,
        raw.c_lflag & libc::ISIG != 0,
        raw.c_oflag & libc::OPOST == before.c_oflag & libc::OPOST,
    );
    announce("<ready>");
    for count in 1.. {
        let event = next_event(generation, indefinite(), core::ptr::null_mut());
        eprintln!("{event:?}");
        announce(&format!("<{count}>"));
        if event == key('q') {
            break;
        }
    }
    leave_raw_mode(generation);
    eprintln!("restored={}", same_mode(&before, &input_mode(0)));
    // A guard that has left and is then destroyed asks again; nothing happens.
    leave_raw_mode(generation);
    eprintln!("restored={}", same_mode(&before, &input_mode(0)));
}

unsafe fn child_waits() {
    let generation = enter_raw_mode();
    let immediate: Wait = (0, 0);
    eprintln!(
        "immediate={:?}",
        next_event(generation, immediate, core::ptr::null_mut())
    );
    let started = Instant::now();
    let event = next_event(generation, within(30_000_000), core::ptr::null_mut());
    eprintln!(
        "timed={event:?} waited={}",
        started.elapsed() >= Duration::from_millis(30)
    );

    let cancelled = crate::cancellation::dr_v1_cancellation_new();
    let (mut reason, mut code, mut has_code) = (0, 0, 0);
    crate::cancellation::dr_v1_cancellation_request(
        cancelled,
        &mut reason,
        &mut code,
        &mut has_code,
    );
    eprintln!(
        "cancelled_before={:?}",
        next_event(generation, indefinite(), cancelled)
    );
    crate::cancellation::dr_v1_cancellation_release(cancelled);

    let pending = crate::cancellation::dr_v1_cancellation_new() as usize;
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        let (mut reason, mut code, mut has_code) = (0, 0, 0);
        crate::cancellation::dr_v1_cancellation_request(
            pending as *const _,
            &mut reason,
            &mut code,
            &mut has_code,
        );
    });
    eprintln!(
        "cancelled_during={:?}",
        next_event(generation, indefinite(), pending as *mut _)
    );
    canceller.join().unwrap();
    crate::cancellation::dr_v1_cancellation_release(pending as *mut _);

    // Any number of resizes before the next wait are one event.
    libc::raise(libc::SIGWINCH);
    libc::raise(libc::SIGWINCH);
    eprintln!(
        "resized={:?} then={:?}",
        next_event(generation, immediate, core::ptr::null_mut()),
        next_event(generation, immediate, core::ptr::null_mut())
    );

    // A wait that was cancelled took nothing: the next one still sees input.
    announce("<ready>");
    eprintln!(
        "after={:?}",
        next_event(generation, indefinite(), core::ptr::null_mut())
    );
    leave_raw_mode(generation);
}

unsafe fn child_ending(scenario: &str) {
    announce("<start>");
    enter_raw_mode();
    perform(operation::ENTER_ALTERNATE_SCREEN);
    perform(operation::HIDE_CURSOR);
    operate(operation::SET_LINE_WRAP, 1, 0, b"").unwrap();
    // Never flushed by the program: ending the process writes it.
    draw(b"X");
    match scenario {
        "exit" => crate::dr_v1_exit_process(3),
        "panic" => crate::panic_catalogued(core::ptr::null(), b"P1001"),
        _ => {
            // What a returning `main` does, without the test harness's own
            // output after it.
            let status = crate::finish_main(7);
            libc::_exit(status);
        }
    }
}

unsafe fn child_unread() {
    let generation = enter_raw_mode();
    announce("<ready>");
    eprintln!(
        "{:?}",
        next_event(generation, indefinite(), core::ptr::null_mut())
    );
    // Let the rest of what was typed arrive before raw mode ends.
    let event = next_event(generation, indefinite(), core::ptr::null_mut());
    eprintln!("{event:?}");
    leave_raw_mode(generation);
    announce("<left>");
    let mut line = [0_u8; 16];
    for _ in 0..2 {
        match crate::device_io::read_progress(StandardStream::Stdin, line.as_mut_ptr(), line.len())
        {
            Ok(crate::byte_io::ReadProgress::Data(count)) => {
                eprintln!("read={:?}", String::from_utf8_lossy(&line[..count]));
            }
            other => eprintln!("read failed: {other:?}"),
        }
    }
}

unsafe fn child_refusals() {
    let generation = enter_raw_mode();
    eprintln!(
        "second_raw={:?}",
        operate(operation::ENTER_RAW_MODE, 0, 0, b"")
    );
    let mut byte = 0_u8;
    eprintln!(
        "read_in_raw={}",
        crate::device_io::read_progress(StandardStream::Stdin, &mut byte, 1).is_err()
    );
    eprintln!(
        "stale_cursor={:?}",
        operate(operation::CURSOR_POSITION, generation as i64 + 1, 0, b"")
    );
    leave_raw_mode(generation);
    let again = enter_raw_mode();
    eprintln!("raw_again={}", again != generation);
    leave_raw_mode(again);

    let screen = operate(operation::ENTER_ALTERNATE_SCREEN, 0, 0, b"").unwrap();
    eprintln!(
        "second_screen={:?}",
        operate(operation::ENTER_ALTERNATE_SCREEN, 0, 0, b"")
    );
    operate(operation::LEAVE_ALTERNATE_SCREEN, screen as i64, 0, b"").unwrap();
    // Left but not yet written out: the screen is free to enter again.
    let next = operate(operation::ENTER_ALTERNATE_SCREEN, 0, 0, b"").unwrap();
    eprintln!("screen_again={}", next != screen);
    // The first guard's destructor must not end the second guard's screen.
    operate(operation::LEAVE_ALTERNATE_SCREEN, screen as i64, 0, b"").unwrap();
    draw(b"<kept>");
    operate(operation::LEAVE_ALTERNATE_SCREEN, next as i64, 0, b"").unwrap();
    eprintln!("unknown={:?}", operate(9999, 0, 0, b""));
    crate::dr_v1_exit_process(0);
}

unsafe fn child_cursor() {
    let generation = enter_raw_mode();
    draw(b"<drawn>");
    let position = operate(operation::CURSOR_POSITION, generation as i64, 0, b"").unwrap();
    eprintln!(
        "position={},{}",
        wire::pair_first(position),
        wire::pair_second(position)
    );
    eprintln!(
        "{:?}",
        next_event(generation, indefinite(), core::ptr::null_mut())
    );
    announce("<again>");
    let started = Instant::now();
    eprintln!(
        "unanswered={:?} waited={}",
        operate(operation::CURSOR_POSITION, generation as i64, 0, b""),
        started.elapsed() >= Duration::from_millis(900)
    );
    leave_raw_mode(generation);
}

unsafe fn child_redirected() {
    eprintln!("raw={:?}", operate(operation::ENTER_RAW_MODE, 0, 0, b""));
    eprintln!(
        "move={:?}",
        operate(operation::MOVE_TO, wire::cell(1, 1), 0, b"")
    );
    eprintln!("size={:?}", operate(operation::SIZE, 0, 0, b""));
    eprintln!("hide={:?}", operate(operation::HIDE_CURSOR, 0, 0, b""));
    eprintln!(
        "screen={:?}",
        operate(operation::ENTER_ALTERNATE_SCREEN, 0, 0, b"")
    );
    eprintln!("title={:?}", operate(operation::SET_TITLE, 0, 0, b"t"));
    eprintln!("color={:?}", operate(operation::COLOR_SUPPORT, 0, 0, b""));
    eprintln!("reset={:?}", operate(operation::RESET, 0, 0, b""));
    let loud = Style {
        foreground: Color::Rgb(1, 2, 3),
        background: Color::Indexed(4),
        attributes: wire::ATTRIBUTE_BOLD | wire::ATTRIBUTE_UNDERLINE,
    };
    eprintln!(
        "write={:?}",
        operate(operation::WRITE, loud.to_bits(), 0, b"<plain>")
    );
    eprintln!("flush={:?}", operate(operation::FLUSH, 0, 0, b""));
    crate::dr_v1_exit_process(0);
}

/// Not in raw mode: Control+C is the terminal's interrupt, not a key.
unsafe fn child_interrupted() {
    announce("<start>");
    perform(operation::ENTER_ALTERNATE_SCREEN);
    perform(operation::HIDE_CURSOR);
    announce("<ready>");
    loop {
        libc::pause();
    }
}

/// A full-screen program that stops itself `stops` times, the way Control+Z
/// through a shell would stop it, and then runs until `q`.
unsafe fn child_job(stops: usize) {
    announce("<start>");
    let generation = enter_raw_mode();
    let screen = operate(operation::ENTER_ALTERNATE_SCREEN, 0, 0, b"").unwrap();
    perform(operation::HIDE_CURSOR);
    operate(operation::SET_LINE_WRAP, 1, 0, b"").unwrap();
    announce("<ready>");
    let raw = input_mode(0);
    for count in 1..=stops {
        libc::raise(libc::SIGTSTP);
        eprintln!("raw_again={}", same_mode(&raw, &input_mode(0)));
        let event = next_event(generation, indefinite(), core::ptr::null_mut());
        eprintln!("{event:?}");
        announce(&format!("<{count}>"));
    }
    loop {
        let event = next_event(generation, indefinite(), core::ptr::null_mut());
        eprintln!("{event:?}");
        if event == key('q') {
            break;
        }
    }
    operate(operation::SET_LINE_WRAP, 0, 0, b"").unwrap();
    perform(operation::SHOW_CURSOR);
    operate(operation::LEAVE_ALTERNATE_SCREEN, screen as i64, 0, b"").unwrap();
    leave_raw_mode(generation);
    announce("<left>");
}

/// The least a job-control shell does: run the job in its own process group
/// in the foreground, take the terminal when the job stops, look at it, hand
/// it back and continue the job.
unsafe fn child_shell() {
    let before = input_mode(0);
    libc::signal(libc::SIGTTOU, libc::SIG_IGN);
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "terminal::pty_tests::child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(SCENARIO, "job");
    command.pre_exec(|| {
        libc::setpgid(0, 0);
        libc::tcsetpgrp(0, libc::getpgrp());
        libc::signal(libc::SIGTTOU, libc::SIG_DFL);
        Ok(())
    });
    let job = command.spawn().unwrap().id() as i32;
    loop {
        let mut status = 0;
        assert_eq!(libc::waitpid(job, &mut status, libc::WUNTRACED), job);
        if !libc::WIFSTOPPED(status) {
            eprintln!(
                "job_succeeded={}",
                libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0
            );
            return;
        }
        libc::tcsetpgrp(0, libc::getpgrp());
        eprintln!("stopped restored={}", same_mode(&before, &input_mode(0)));
        let marker = b"<stopped>";
        libc::write(1, marker.as_ptr().cast(), marker.len());
        libc::tcsetpgrp(0, job);
        libc::kill(job, libc::SIGCONT);
    }
}

unsafe fn handler_of(signal: i32) -> libc::sighandler_t {
    let mut action = core::mem::MaybeUninit::<libc::sigaction>::zeroed();
    assert_eq!(
        libc::sigaction(signal, core::ptr::null(), action.as_mut_ptr()),
        0
    );
    action.assume_init().sa_sigaction
}

unsafe fn child_watching() {
    libc::signal(libc::SIGINT, libc::SIG_IGN);
    eprintln!("before={}", handler_of(libc::SIGTERM) == libc::SIG_DFL);
    perform(operation::HIDE_CURSOR);
    eprintln!(
        "watching={} ignored_kept={}",
        handler_of(libc::SIGTERM) != libc::SIG_DFL,
        handler_of(libc::SIGINT) == libc::SIG_IGN
    );
    perform(operation::SHOW_CURSOR);
    // Undone, but not yet written: the terminal still has a hidden cursor.
    eprintln!(
        "still_watching={}",
        handler_of(libc::SIGTERM) != libc::SIG_DFL
    );
    perform(operation::FLUSH);
    eprintln!(
        "released={} ignored_kept={}",
        handler_of(libc::SIGTERM) == libc::SIG_DFL,
        handler_of(libc::SIGINT) == libc::SIG_IGN
    );
    let generation = enter_raw_mode();
    eprintln!("raw_watching={}", handler_of(libc::SIGHUP) != libc::SIG_DFL);
    leave_raw_mode(generation);
    eprintln!(
        "raw_released={} resize_released={}",
        handler_of(libc::SIGHUP) == libc::SIG_DFL,
        handler_of(libc::SIGWINCH) == libc::SIG_DFL
    );
}

/// The signals a library the program uses takes over in [`child_layered`].
const HOSTED: [i32; 4] = [libc::SIGWINCH, libc::SIGCONT, libc::SIGTSTP, libc::SIGTERM];
static HOST_CALLS: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
/// The handler each host handler replaced, and whether it takes three
/// arguments.
static HOST_BELOW: [AtomicUsize; 4] = [const { AtomicUsize::new(0) }; 4];
static HOST_BELOW_DETAILED: [AtomicBool; 4] = [const { AtomicBool::new(false) }; 4];

/// What a well-behaved library installs: it does its own work, here a count,
/// and calls the handler it replaced.
extern "C" fn host_handler(signal: i32, info: *mut libc::siginfo_t, context: *mut c_void) {
    let Some(index) = HOSTED.iter().position(|hosted| *hosted == signal) else {
        return;
    };
    HOST_CALLS[index].fetch_add(1, Ordering::SeqCst);
    let below = HOST_BELOW[index].load(Ordering::SeqCst);
    if below == libc::SIG_DFL || below == libc::SIG_IGN {
        return;
    }
    unsafe {
        if HOST_BELOW_DETAILED[index].load(Ordering::SeqCst) {
            let below: extern "C" fn(i32, *mut libc::siginfo_t, *mut c_void) =
                core::mem::transmute(below);
            below(signal, info, context);
        } else {
            let below: extern "C" fn(i32) = core::mem::transmute(below);
            below(signal);
        }
    }
}

fn host_handler_address() -> libc::sighandler_t {
    host_handler as *const () as libc::sighandler_t
}

unsafe fn install_host_handlers() {
    for (index, signal) in HOSTED.into_iter().enumerate() {
        let mut action: libc::sigaction = core::mem::zeroed();
        action.sa_sigaction = host_handler_address();
        action.sa_flags = libc::SA_SIGINFO | libc::SA_RESTART;
        libc::sigemptyset(&mut action.sa_mask);
        let mut below = core::mem::MaybeUninit::<libc::sigaction>::zeroed();
        assert_eq!(libc::sigaction(signal, &action, below.as_mut_ptr()), 0);
        let below = below.assume_init();
        HOST_BELOW[index].store(below.sa_sigaction, Ordering::SeqCst);
        HOST_BELOW_DETAILED[index].store(below.sa_flags & libc::SA_SIGINFO != 0, Ordering::SeqCst);
    }
}

unsafe fn host_handlers_installed() -> bool {
    HOSTED
        .into_iter()
        .all(|signal| handler_of(signal) == host_handler_address())
}

fn host_calls() -> [usize; 4] {
    [0, 1, 2, 3].map(|index| HOST_CALLS[index].load(Ordering::SeqCst))
}

/// A library installs its handlers over this layer's while raw mode is on,
/// and the program then leaves raw mode and enters it again. The library's
/// handlers keep their place, each signal still reaches both, once, and the
/// second raw mode ends like the first.
unsafe fn child_layered(terminated: bool) {
    let before = input_mode(0);
    let first = enter_raw_mode();
    let under: Vec<bool> = HOSTED
        .into_iter()
        .map(|signal| handler_of(signal) != libc::SIG_DFL)
        .collect();
    install_host_handlers();
    leave_raw_mode(first);
    eprintln!(
        "under={under:?} kept_after_leaving={}",
        host_handlers_installed()
    );
    let second = enter_raw_mode();
    eprintln!("kept_after_entering={}", host_handlers_installed());

    announce("<resize>");
    let event = next_event(second, indefinite(), core::ptr::null_mut());
    eprintln!("{event:?} calls={:?}", host_calls());

    libc::raise(libc::SIGCONT);
    eprintln!("continued calls={:?}", host_calls());

    // The child leads its session, so the system discards the stop and the
    // handler runs straight through to taking the terminal again.
    libc::raise(libc::SIGTSTP);
    let event = next_event(second, indefinite(), core::ptr::null_mut());
    eprintln!(
        "{event:?} calls={:?} kept_after_stopping={}",
        host_calls(),
        host_handlers_installed()
    );

    announce("<ready>");
    let event = next_event(second, indefinite(), core::ptr::null_mut());
    if terminated {
        // Only the terminating signal ends this wait.
        panic!("{event:?} arrived instead of the terminating signal");
    }
    assert_eq!(event, key('q'));
    leave_raw_mode(second);
    eprintln!(
        "kept_at_the_end={} restored={}",
        host_handlers_installed(),
        same_mode(&before, &input_mode(0))
    );
}

/// A transfer group that watches raw mode (Decision 0140): the group's wait
/// is what wakes for a key, for a lone Escape's delay and for a resize, and
/// the program's own event waits still see every event exactly once.
unsafe fn child_grouped() {
    use crate::transfer_group::transport::watch_fixture::{Fetched, WatchingGroup};
    const IMMEDIATE: Wait = (0, 0);
    let generation = enter_raw_mode();
    let source = (1 << 32) | generation;
    let drain = |expected: &[Observation]| {
        for event in expected {
            assert_eq!(
                next_event(generation, IMMEDIATE, core::ptr::null_mut()),
                *event
            );
        }
        assert_eq!(
            next_event(generation, IMMEDIATE, core::ptr::null_mut()),
            Observation::TimedOut
        );
    };
    {
        let mut group = WatchingGroup::begin(source, None);
        // Armed, with nothing typed yet. An armed watch cannot be armed again.
        assert_eq!(group.fetch(IMMEDIATE), Fetched::TimedOut);
        assert_eq!(group.fetch(within(20_000_000)), Fetched::TimedOut);
        assert!(!group.resubmit());
        assert!(!group.resubmit_foreign());

        announce("<key>");
        assert_eq!(group.fetch(indefinite()), Fetched::Ready);
        // Reported once. Until it is armed again the watch is retired, and
        // with nothing else in the plan the group has nothing left.
        assert_eq!(group.fetch(IMMEDIATE), Fetched::Exhausted);
        assert_eq!(group.fetch(indefinite()), Fetched::Exhausted);
        drain(&[key('a')]);
        assert!(group.resubmit());
        assert!(!group.resubmit());
        assert_eq!(group.fetch(IMMEDIATE), Fetched::TimedOut);
        eprintln!("key");

        // A lone Escape is a key only once its delay has run out. The group
        // must wake for that moment, and not report before it.
        announce("<escape>");
        assert_eq!(group.fetch(indefinite()), Fetched::Ready);
        drain(&[Observation::Key {
            key: Key::Escape,
            modifiers: 0,
        }]);
        assert!(group.resubmit());
        eprintln!("escape");

        announce("<resize>");
        assert_eq!(group.fetch(indefinite()), Fetched::Ready);
        drain(&[Observation::Resized {
            columns: 100,
            rows: 30,
        }]);
        assert!(group.resubmit());
        eprintln!("resize");

        // Events that were waiting before the watch was armed count, and a
        // program that takes them itself is not told about them afterwards.
        announce("<typed-ahead>");
        assert_eq!(
            next_event(generation, indefinite(), core::ptr::null_mut()),
            key('b')
        );
        assert_eq!(group.fetch(IMMEDIATE), Fetched::TimedOut);
        announce("<waiting>");
        assert_eq!(
            next_event(generation, indefinite(), core::ptr::null_mut()),
            key('c')
        );
        assert_eq!(group.fetch(within(20_000_000)), Fetched::TimedOut);
        eprintln!("drained");

        // A finite wait ends on its budget and leaves the watch armed.
        assert_eq!(group.fetch(within(20_000_000)), Fetched::TimedOut);
        announce("<again>");
        assert_eq!(group.fetch(within(20_000_000_000)), Fetched::Ready);
        drain(&[key('d')]);

        // A second group on the same readiness set, begun while the first
        // one's watch was armed: the set holds nothing of the first.
        assert!(group.resubmit());
        group.begin_again(source, None);
        assert_eq!(group.fetch(within(20_000_000)), Fetched::TimedOut);

        // A group whose deadline has passed retires its armed watch the way
        // it retires a pending transfer: reported once, then nothing is left
        // and nothing can be armed.
        let past = crate::deadline::DrDeadlineV1::from_clock_and_duration(0, -1);
        group.begin_again(source, Some(past));
        assert_eq!(group.fetch(indefinite()), Fetched::Ready);
        assert_eq!(group.fetch(indefinite()), Fetched::Exhausted);
        assert!(!group.resubmit());
        drain(&[]);
    }
    // Teardown leaves raw mode in effect and the event wait working.
    let raw = input_mode(0);
    eprintln!(
        "still_raw={}",
        raw.c_lflag & (libc::ICANON | libc::ECHO) == 0
    );
    announce("<after>");
    assert_eq!(
        next_event(generation, indefinite(), core::ptr::null_mut()),
        key('e')
    );
    // Input that reached the terminal before a group began is there when
    // its watch is armed, so the first fetch reports it.
    announce("<before>");
    let mut typed = libc::pollfd {
        fd: 0,
        events: libc::POLLIN,
        revents: 0,
    };
    assert_eq!(libc::poll(&mut typed, 1, 20_000), 1);
    {
        let mut group = WatchingGroup::begin(source, None);
        assert_eq!(group.fetch(IMMEDIATE), Fetched::Ready);
        drain(&[key('f')]);
    }
    leave_raw_mode(generation);
    {
        // A raw mode that has ended is closed, which an event wait reports
        // at once, so a watch of it is ready.
        let mut group = WatchingGroup::begin(source, None);
        assert_eq!(group.fetch(IMMEDIATE), Fetched::Ready);
    }
    eprintln!("done");
}

// ---- What the terminal sees ----------------------------------------------

#[test]
fn a_frame_reaches_the_terminal_in_order_as_the_renderer_wrote_it() {
    let mut terminal = Terminal::open(80, 24);
    let child = terminal.run("frame");
    let (status, report) = terminal.finish(child);
    assert_eq!(status, Some(0), "{report}");
    assert!(report.contains("size=80x24 color=2"), "{report}");
    let expected: &[u8] = b"\x1b[?25l\
        \x1b[2;3H\x1b[1;31mhi\x1b[0m\
        \x1b[4;1H\x1b[4C\x1b[K\
        \x1b[1;8H\x1b[X\
        \x1b]2;abc\x07\
        AB\
        \x1b[38;5;196mC\x1b[0m\
        \x1b[?25h";
    assert_eq!(
        printable(terminal.drawn_after(b"<frame>")),
        printable(expected)
    );
}

#[test]
fn keys_and_resizes_arrive_as_one_ordered_stream_and_the_input_mode_returns() {
    let mut terminal = Terminal::open(80, 24);
    let before = terminal.mode();
    let mut child = terminal.run("keys");
    terminal.wait_for(&mut child, b"<ready>");
    assert!(!same_mode(&before, &terminal.mode()));
    terminal.type_bytes(b"a\x1b[A");
    terminal.wait_for(&mut child, b"<2>");
    // Escape on its own is a key once nothing follows it.
    terminal.type_bytes(b"\x1b");
    terminal.wait_for(&mut child, b"<3>");
    // Control+C is a key in raw mode, not an interrupt.
    terminal.type_bytes(b"\x03");
    terminal.type_bytes("é".as_bytes());
    terminal.type_bytes(b"\x1b[1;5C");
    terminal.wait_for(&mut child, b"<6>");
    terminal.resize(120, 40);
    terminal.wait_for(&mut child, b"<7>");
    terminal.type_bytes(b"q");
    let (status, report) = terminal.finish(child);
    assert_eq!(status, Some(0), "{report}");
    let expected = [
        "canonical=false echo=false signals=false output_processing=true",
        "Key { key: Character('a'), modifiers: 0 }",
        "Key { key: Up, modifiers: 0 }",
        "Key { key: Escape, modifiers: 0 }",
        "Key { key: Character('c'), modifiers: 1 }",
        "Key { key: Character('é'), modifiers: 0 }",
        "Key { key: Right, modifiers: 1 }",
        "Resized { columns: 120, rows: 40 }",
        "Key { key: Character('q'), modifiers: 0 }",
        "restored=true",
        "restored=true",
    ];
    let reported: Vec<&str> = report
        .lines()
        .filter(|line| !line.is_empty())
        .take(expected.len())
        .collect();
    assert_eq!(reported, expected, "{report}");
    assert!(same_mode(&before, &terminal.mode()));
}

#[test]
fn a_transfer_group_wakes_for_a_key_an_escape_delay_and_a_resize_and_takes_no_event() {
    let mut terminal = Terminal::open(80, 24);
    let before = terminal.mode();
    let mut child = terminal.run("grouped");
    terminal.wait_for(&mut child, b"<key>");
    terminal.type_bytes(b"a");
    terminal.wait_for(&mut child, b"<escape>");
    terminal.type_bytes(b"\x1b");
    terminal.wait_for(&mut child, b"<resize>");
    terminal.resize(100, 30);
    terminal.wait_for(&mut child, b"<typed-ahead>");
    terminal.type_bytes(b"b");
    terminal.wait_for(&mut child, b"<waiting>");
    terminal.type_bytes(b"c");
    terminal.wait_for(&mut child, b"<again>");
    terminal.type_bytes(b"d");
    terminal.wait_for(&mut child, b"<after>");
    terminal.type_bytes(b"e");
    terminal.wait_for(&mut child, b"<before>");
    terminal.type_bytes(b"f");
    let (status, report) = terminal.finish(child);
    assert_eq!(status, Some(0), "{report}");
    let expected = [
        "key",
        "escape",
        "resize",
        "drained",
        "still_raw=true",
        "done",
    ];
    let reported: Vec<&str> = report
        .lines()
        .filter(|line| !line.is_empty())
        .take(expected.len())
        .collect();
    assert_eq!(reported, expected, "{report}");
    assert!(same_mode(&before, &terminal.mode()));
}

#[test]
fn a_wait_ends_on_its_budget_or_its_cancellation_without_taking_input() {
    let mut terminal = Terminal::open(80, 24);
    let mut child = terminal.run("waits");
    terminal.wait_for(&mut child, b"<ready>");
    terminal.type_bytes(b"k");
    let (status, report) = terminal.finish(child);
    assert_eq!(status, Some(0), "{report}");
    for line in [
        "immediate=TimedOut",
        "timed=TimedOut waited=true",
        "cancelled_before=Cancelled",
        "cancelled_during=Cancelled",
        "resized=Resized { columns: 80, rows: 24 } then=TimedOut",
        "after=Key { key: Character('k'), modifiers: 0 }",
    ] {
        assert!(report.contains(line), "missing {line:?} in:\n{report}");
    }
}

#[test]
fn every_way_the_process_ends_puts_the_terminal_back() {
    for (scenario, status) in [("exit", 3), ("panic", 101), ("return", 7)] {
        let mut terminal = Terminal::open(80, 24);
        let before = terminal.mode();
        let child = terminal.run(scenario);
        let (code, report) = terminal.finish(child);
        assert_eq!(code, Some(status), "{scenario}: {report}");
        assert_eq!(
            printable(terminal.drawn_after(b"<start>")),
            printable(b"\x1b[?1049h\x1b[?25l\x1b[?7lX\x1b[?1049l\x1b[?7h\x1b[?25h"),
            "{scenario}"
        );
        assert!(same_mode(&before, &terminal.mode()), "{scenario}");
        assert_eq!(report.contains("P1001"), scenario == "panic", "{report}");
    }
}

#[test]
fn input_typed_in_raw_mode_and_never_asked_for_stays_the_programs_input() {
    let mut terminal = Terminal::open(80, 24);
    let mut child = terminal.run("unread");
    terminal.wait_for(&mut child, b"<ready>");
    terminal.type_bytes(b"xyz");
    terminal.wait_for(&mut child, b"<left>");
    terminal.type_bytes(b"\r");
    let (status, report) = terminal.finish(child);
    assert_eq!(status, Some(0), "{report}");
    for line in [
        "Key { key: Character('x'), modifiers: 0 }",
        "Key { key: Character('y'), modifiers: 0 }",
        "read=\"z\"",
        "read=\"\\n\"",
    ] {
        assert!(report.contains(line), "missing {line:?} in:\n{report}");
    }
}

#[test]
fn one_owner_at_a_time_holds_raw_mode_and_the_alternate_screen() {
    let mut terminal = Terminal::open(80, 24);
    let child = terminal.run("refusals");
    let (status, report) = terminal.finish(child);
    assert_eq!(status, Some(0), "{report}");
    let in_use = format!("Err(({}, None))", wire::STATUS_IN_USE);
    for line in [
        format!("second_raw={in_use}"),
        "read_in_raw=true".into(),
        format!("stale_cursor={in_use}"),
        "raw_again=true".into(),
        format!("second_screen={in_use}"),
        "screen_again=true".into(),
        format!("unknown=Err(({}, None))", wire::STATUS_UNSUPPORTED),
    ] {
        assert!(report.contains(&line), "missing {line:?} in:\n{report}");
    }
    let drawn = printable(&terminal.drawn);
    assert!(
        drawn.ends_with("ESC[?1049hESC[?1049lESC[?1049h<kept>ESC[?1049l"),
        "{drawn}"
    );
}

#[test]
fn the_cursor_position_is_the_terminals_answer_and_keys_around_it_stay_queued() {
    let mut terminal = Terminal::open(80, 24);
    let mut child = terminal.run("cursor");
    // The query follows what the program had drawn but not flushed.
    terminal.wait_for(&mut child, b"<drawn>\x1b[6n");
    terminal.type_bytes(b"k\x1b[5;10R");
    terminal.wait_for(&mut child, b"<again>\x1b[6n");
    let (status, report) = terminal.finish(child);
    assert_eq!(status, Some(0), "{report}");
    for line in [
        "position=9,4".into(),
        "Key { key: Character('k'), modifiers: 0 }".into(),
        format!(
            "unanswered=Err(({}, None)) waited=true",
            wire::STATUS_UNSUPPORTED
        ),
    ] {
        assert!(report.contains(&line), "missing {line:?} in:\n{report}");
    }
}

#[test]
fn a_redirected_program_writes_plain_text_and_is_told_it_has_no_terminal() {
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "terminal::pty_tests::child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(SCENARIO, "redirected")
        .env("TERM", "xterm-256color")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    let report = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "{report}");
    let not_a_terminal = format!("Err(({}, None))", wire::STATUS_NOT_A_TERMINAL);
    for line in [
        format!("raw={not_a_terminal}"),
        format!("move={not_a_terminal}"),
        format!("size={not_a_terminal}"),
        format!("hide={not_a_terminal}"),
        format!("screen={not_a_terminal}"),
        format!("title={not_a_terminal}"),
        "color=Ok(0)".into(),
        "reset=Ok(0)".into(),
        "write=Ok(0)".into(),
        "flush=Ok(0)".into(),
    ] {
        assert!(report.contains(&line), "missing {line:?} in:\n{report}");
    }
    let drawn = String::from_utf8_lossy(&output.stdout);
    assert!(drawn.ends_with("<plain>"), "{drawn:?}");
    assert!(!drawn.contains('\x1b'), "{drawn:?}");
}

#[test]
fn the_terminals_interrupt_key_ends_the_program_with_the_terminal_put_back() {
    let mut terminal = Terminal::open(80, 24);
    let mut child = terminal.run("interrupted");
    terminal.wait_for(&mut child, b"<ready>");
    terminal.type_bytes(b"\x03");
    let (status, report) = terminal.finish_with_status(child);
    assert_eq!(status.signal(), Some(libc::SIGINT), "{report}");
    let drawn = printable(terminal.drawn_after(b"<start>"));
    assert!(drawn.starts_with("ESC[?1049hESC[?25l<ready>"), "{drawn}");
    assert!(drawn.ends_with("ESC[?1049lESC[?25h"), "{drawn}");
}

#[test]
fn a_terminating_signal_puts_the_terminal_back_and_still_ends_the_program_as_itself() {
    let mut terminal = Terminal::open(80, 24);
    let before = terminal.mode();
    let mut child = terminal.run("terminated");
    terminal.wait_for(&mut child, b"<ready>");
    assert!(!same_mode(&before, &terminal.mode()));
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGTERM) }, 0);
    let (status, report) = terminal.finish_with_status(child);
    assert_eq!(status.signal(), Some(libc::SIGTERM), "{report}");
    assert_eq!(
        printable(terminal.drawn_after(b"<ready>")),
        "ESC[?1049lESC[?7hESC[?25h"
    );
    assert!(same_mode(&before, &terminal.mode()));
}

#[test]
fn a_stopped_program_gives_the_terminal_back_and_takes_it_again_when_continued() {
    let mut terminal = Terminal::open(80, 24);
    let before = terminal.mode();
    let mut child = terminal.run("suspended");
    terminal.wait_for(&mut child, b"<2>");
    terminal.type_bytes(b"q");
    let (status, report) = terminal.finish(child);
    assert_eq!(status, Some(0), "{report}");
    // Continuing is reported as a resize, which is what makes a program draw
    // again on the screen it has just been given back.
    for (line, times) in [
        ("stopped restored=true", 2),
        ("raw_again=true", 2),
        ("Resized { columns: 80, rows: 24 }", 2),
        ("job_succeeded=true", 1),
    ] {
        assert_eq!(
            report.matches(line).count(),
            times,
            "{line:?} in:\n{report}"
        );
    }
    let stop = "ESC[?1049lESC[?7hESC[?25h<stopped>ESC[?1049hESC[?7lESC[?25l";
    assert_eq!(
        printable(terminal.drawn_between(b"<ready>", b"<left>")),
        format!("{stop}<1>{stop}<2>ESC[?7hESC[?25hESC[?1049l<left>")
    );
    assert!(same_mode(&before, &terminal.mode()));
}

#[test]
fn a_stop_the_system_discards_leaves_the_program_with_its_terminal() {
    // The child is a session leader with no shell above it, so nothing could
    // ever continue it and the system does not stop it.
    let mut terminal = Terminal::open(80, 24);
    let before = terminal.mode();
    let mut child = terminal.run("discarded");
    terminal.wait_for(&mut child, b"<1>");
    assert!(!same_mode(&before, &terminal.mode()));
    terminal.type_bytes(b"q");
    let (status, report) = terminal.finish(child);
    assert_eq!(status, Some(0), "{report}");
    assert!(report.contains("raw_again=true"), "{report}");
    assert_eq!(
        printable(terminal.drawn_between(b"<ready>", b"<left>")),
        "ESC[?1049lESC[?7hESC[?25hESC[?1049hESC[?7lESC[?25l<1>ESC[?7hESC[?25hESC[?1049l<left>"
    );
    assert!(same_mode(&before, &terminal.mode()));
}

#[test]
fn handlers_a_library_installed_over_ours_keep_working_when_raw_mode_is_entered_again() {
    let mut terminal = Terminal::open(80, 24);
    let before = terminal.mode();
    let mut child = terminal.run("layered");
    terminal.wait_for(&mut child, b"<resize>");
    terminal.resize(100, 30);
    terminal.wait_for(&mut child, b"<ready>");
    terminal.type_bytes(b"q");
    let (status, report) = terminal.finish(child);
    assert_eq!(status, Some(0), "{report}");
    // Each signal reached the library's handler once and ours once: the
    // resize and the end of the stop are each reported as one event.
    for line in [
        "under=[true, true, true, true] kept_after_leaving=true",
        "kept_after_entering=true",
        "Resized { columns: 100, rows: 30 } calls=[1, 0, 0, 0]",
        "continued calls=[1, 1, 0, 0]",
        "Resized { columns: 100, rows: 30 } calls=[1, 1, 1, 0] kept_after_stopping=true",
        "kept_at_the_end=true restored=true",
    ] {
        assert!(report.contains(line), "missing {line:?} in:\n{report}");
    }
    assert!(same_mode(&before, &terminal.mode()));
}

#[test]
fn a_terminating_signal_through_a_librarys_handler_still_puts_the_terminal_back() {
    let mut terminal = Terminal::open(80, 24);
    let before = terminal.mode();
    let mut child = terminal.run("layered-terminated");
    terminal.wait_for(&mut child, b"<resize>");
    terminal.resize(100, 30);
    terminal.wait_for(&mut child, b"<ready>");
    assert!(!same_mode(&before, &terminal.mode()));
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGTERM) }, 0);
    let (status, report) = terminal.finish_with_status(child);
    assert_eq!(status.signal(), Some(libc::SIGTERM), "{report}");
    assert!(same_mode(&before, &terminal.mode()));
}

#[test]
fn signal_handlers_exist_only_while_there_is_something_to_put_back() {
    let mut terminal = Terminal::open(80, 24);
    let child = terminal.run("watching");
    let (status, report) = terminal.finish(child);
    assert_eq!(status, Some(0), "{report}");
    for line in [
        "before=true",
        "watching=true ignored_kept=true",
        "still_watching=true",
        "released=true ignored_kept=true",
        "raw_watching=true",
        "raw_released=true resize_released=true",
    ] {
        assert!(report.contains(line), "missing {line:?} in:\n{report}");
    }
}
