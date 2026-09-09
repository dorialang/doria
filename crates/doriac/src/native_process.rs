//! Launch freshly emitted executables with one bounded, platform-aware policy.

use std::io;
use std::process::{Child, Command};
use std::time::Duration;

const MAX_ATTEMPTS: usize = 20;
const RETRY_DELAY: Duration = Duration::from_millis(25);

/// Retry only a failed spawn, never a running child's wait, output, or exit status.
pub fn spawn(command: &mut Command) -> io::Result<Child> {
    retry_spawn(|| command.spawn(), std::thread::sleep)
}

fn retry_spawn<T>(
    mut spawn: impl FnMut() -> io::Result<T>,
    mut sleep: impl FnMut(Duration),
) -> io::Result<T> {
    for attempt in 0..MAX_ATTEMPTS {
        match spawn() {
            Err(error) if is_transient_launch_error(&error) && attempt + 1 < MAX_ATTEMPTS => {
                sleep(RETRY_DELAY);
            }
            result => return result,
        }
    }
    unreachable!("the final attempt always returns")
}

fn is_transient_launch_error(error: &io::Error) -> bool {
    // ETXTBSY on Unix; macOS can also transiently report EBADMACHO immediately
    // after linking. Preserve the existing native-runner policy on both hosts.
    cfg!(unix)
        && (error.raw_os_error() == Some(26)
            || (cfg!(target_os = "macos") && error.raw_os_error() == Some(88)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn retries_a_real_executable_write_lock_without_replaying_the_child() {
        use std::fs::{self, OpenOptions};
        use std::os::unix::fs::PermissionsExt;
        use std::process::Stdio;
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("doriac-spawn-{}-{nonce}", std::process::id()));
        // Linux denies exec while an executable is open for writing, including
        // scripts. Releasing our handle need not clear all launch contention, so
        // subsequent attempts use the production delay and bounded retry policy.
        fs::write(&path, b"#!/bin/sh\nprintf 'ran once\\n'\nexit 42\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let mut writer = Some(OpenOptions::new().write(true).open(&path).unwrap());
        let mut command = Command::new(&path);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut attempts = 0;
        let result = retry_spawn(
            || {
                attempts += 1;
                command.spawn()
            },
            |delay| {
                drop(writer.take());
                std::thread::sleep(delay);
            },
        );
        drop(writer);
        let output = result.and_then(|child| child.wait_with_output());
        fs::remove_file(path).unwrap();
        let output = output.expect("the unlocked executable should run");
        assert!((2..=MAX_ATTEMPTS).contains(&attempts), "{attempts}");
        assert_eq!(output.status.code(), Some(42));
        assert_eq!(output.stdout, b"ran once\n");
        assert!(output.stderr.is_empty());
    }

    #[test]
    fn successful_spawn_is_never_repeated() {
        let mut attempts = 0;
        let result = retry_spawn(
            || {
                attempts += 1;
                Ok(42)
            },
            |_| panic!("successful spawn must not sleep"),
        );
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts, 1);
    }

    #[test]
    fn unrelated_errors_are_returned_immediately() {
        let mut attempts = 0;
        let result = retry_spawn::<()>(
            || {
                attempts += 1;
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            },
            |_| panic!("permanent errors must not sleep"),
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(attempts, 1);
    }

    #[test]
    fn transient_error_codes_are_platform_specific() {
        assert_eq!(
            is_transient_launch_error(&io::Error::from_raw_os_error(26)),
            cfg!(unix)
        );
        assert_eq!(
            is_transient_launch_error(&io::Error::from_raw_os_error(88)),
            cfg!(target_os = "macos")
        );
        assert!(!is_transient_launch_error(&io::Error::from_raw_os_error(5)));
    }

    #[cfg(unix)]
    #[test]
    fn transient_spawn_retries_are_bounded_and_preserve_the_last_error() {
        for succeeds_on in [3, MAX_ATTEMPTS, MAX_ATTEMPTS + 1] {
            let mut attempts = 0;
            let mut sleeps = 0;
            let result = retry_spawn(
                || {
                    attempts += 1;
                    if attempts == succeeds_on {
                        Ok(42)
                    } else {
                        Err(io::Error::from_raw_os_error(26))
                    }
                },
                |delay| {
                    assert_eq!(delay, RETRY_DELAY);
                    sleeps += 1;
                },
            );
            assert_eq!(attempts, succeeds_on.min(MAX_ATTEMPTS));
            assert_eq!(sleeps, attempts - 1);
            if succeeds_on <= MAX_ATTEMPTS {
                assert_eq!(result.unwrap(), 42);
            } else {
                assert_eq!(result.unwrap_err().raw_os_error(), Some(26));
            }
        }
    }
}
