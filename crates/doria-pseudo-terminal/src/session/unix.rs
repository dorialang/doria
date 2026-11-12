//! A pty. The program becomes a session leader with the pty as its
//! controlling terminal, so keys typed and sizes set here reach it the way a
//! terminal emulator's do, signals included.

use std::ffi::OsString;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use super::Exit;

pub(super) struct Session {
    master: OwnedFd,
    child: Child,
}

fn size(columns: u16, rows: u16) -> libc::winsize {
    libc::winsize {
        ws_row: rows,
        ws_col: columns,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

impl Session {
    pub(super) fn start(
        program: &Path,
        arguments: &[&str],
        environment: &[(OsString, OsString)],
        columns: u16,
        rows: u16,
    ) -> Self {
        let (mut master, mut slave) = (-1, -1);
        let (master, slave) = {
            // `openpty` names the slave through a static buffer on macOS, so
            // two tests opening terminals at once can be handed each other's.
            static OPENING: std::sync::Mutex<()> = std::sync::Mutex::new(());
            let _opening = OPENING
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let size = size(columns, rows);
            unsafe {
                assert_eq!(
                    libc::openpty(
                        &mut master,
                        &mut slave,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        // Linux declares this parameter const, macOS mutable.
                        &size as *const libc::winsize as *mut libc::winsize,
                    ),
                    0,
                    "{}",
                    std::io::Error::last_os_error()
                );
                let flags = libc::fcntl(master, libc::F_GETFL);
                assert_eq!(
                    libc::fcntl(master, libc::F_SETFL, flags | libc::O_NONBLOCK),
                    0
                );
                (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave))
            }
        };
        let mut command = Command::new(program);
        command
            .args(arguments)
            .env_clear()
            .envs(environment.iter().map(|(name, value)| (name, value)))
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave));
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
        let child = command.spawn().unwrap();
        Self { master, child }
    }

    pub(super) fn write(&mut self, bytes: &[u8]) {
        let written =
            unsafe { libc::write(self.master.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
        assert_eq!(written, bytes.len() as isize);
    }

    pub(super) fn resize(&mut self, columns: u16, rows: u16) {
        let size = size(columns, rows);
        assert_eq!(
            unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ as _, &size) },
            0
        );
    }

    pub(super) fn read(&mut self, drawn: &mut Vec<u8>, wait: Duration) -> bool {
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
        drawn.extend_from_slice(&bytes[..count as usize]);
        true
    }

    pub(super) fn exited(&mut self) -> Option<Exit> {
        self.child.try_wait().unwrap().map(|status| Exit {
            code: status.code(),
            signal: status.signal(),
        })
    }

    pub(super) fn close(&mut self, drawn: &mut Vec<u8>) {
        while self.read(drawn, Duration::from_millis(100)) {}
    }

    pub(super) fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Read through the master side: once a program that held the pty as its
    /// controlling terminal exits, every slave descriptor is revoked.
    pub(super) fn input_mode(&self) -> libc::termios {
        unsafe {
            let mut mode = std::mem::MaybeUninit::<libc::termios>::zeroed();
            assert_eq!(
                libc::tcgetattr(self.master.as_raw_fd(), mode.as_mut_ptr()),
                0,
                "{}",
                std::io::Error::last_os_error()
            );
            mode.assume_init()
        }
    }

    pub(super) fn process(&self) -> u32 {
        self.child.id()
    }
}
