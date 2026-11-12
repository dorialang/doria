//! A program running inside a pseudo-terminal.

use std::ffi::OsString;
use std::path::Path;
use std::time::{Duration, Instant};

#[cfg(unix)]
#[path = "session/unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "session/windows.rs"]
mod platform;

/// How long a test waits for the program before it gives up and reports
/// what was drawn so far.
const PATIENCE: Duration = Duration::from_secs(30);

/// How the program ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exit {
    /// The exit status, when the program exited on its own.
    pub code: Option<i32>,
    /// The signal that ended it. Always `None` on Windows.
    pub signal: Option<i32>,
}

pub struct Session {
    platform: platform::Session,
    /// Every byte the terminal has received from the program so far.
    pub drawn: Vec<u8>,
}

impl Session {
    /// Runs `program` with the pseudo-terminal as its standard input, output
    /// and error. `environment` adds to or replaces the inherited variables;
    /// `NO_COLOR` and `COLORTERM` are never inherited, so color support is
    /// what the test says it is.
    pub fn start(
        program: &Path,
        arguments: &[&str],
        environment: &[(&str, &str)],
        columns: u16,
        rows: u16,
    ) -> Self {
        let mut variables: Vec<(OsString, OsString)> = std::env::vars_os()
            .filter(|(name, _)| name != "NO_COLOR" && name != "COLORTERM")
            .collect();
        for (name, value) in environment {
            variables.retain(|(existing, _)| existing != name);
            variables.push((OsString::from(name), OsString::from(value)));
        }
        Self {
            platform: platform::Session::start(program, arguments, &variables, columns, rows),
            drawn: Vec::new(),
        }
    }

    /// Types at the program.
    pub fn type_bytes(&mut self, bytes: &[u8]) {
        self.platform.write(bytes);
    }

    pub fn resize(&mut self, columns: u16, rows: u16) {
        self.platform.resize(columns, rows);
    }

    /// Reads what the program has drawn for up to `wait`. False when nothing
    /// arrived in that time.
    pub fn read_for(&mut self, wait: Duration) -> bool {
        self.platform.read(&mut self.drawn, wait)
    }

    /// Waits until `done` accepts what has been drawn. `what` names the
    /// thing awaited, for the failure report.
    pub fn wait_until(&mut self, what: &str, mut done: impl FnMut(&[u8]) -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !done(&self.drawn) {
            if Instant::now() > deadline {
                self.platform.kill();
                panic!(
                    "the program never drew {what}; it drew {:?}",
                    String::from_utf8_lossy(&self.drawn)
                );
            }
            self.read_for(Duration::from_millis(50));
        }
    }

    /// Waits for the program to end and returns how, with everything it
    /// drew. A terminal keeps reading while a program exits: a process cannot
    /// finish closing a terminal that still holds its unread output.
    pub fn finish(mut self) -> (Exit, Vec<u8>) {
        let deadline = Instant::now() + PATIENCE;
        let exit = loop {
            self.read_for(Duration::from_millis(20));
            if let Some(exit) = self.platform.exited() {
                break exit;
            }
            if Instant::now() > deadline {
                self.platform.kill();
                panic!(
                    "the program never exited; it drew {:?}",
                    String::from_utf8_lossy(&self.drawn)
                );
            }
        };
        self.platform.close(&mut self.drawn);
        (exit, self.drawn)
    }

    /// The terminal's input mode, read from the terminal's own side.
    #[cfg(unix)]
    pub fn input_mode(&self) -> libc::termios {
        self.platform.input_mode()
    }

    /// The program's process identifier.
    pub fn process(&self) -> u32 {
        self.platform.process()
    }
}
