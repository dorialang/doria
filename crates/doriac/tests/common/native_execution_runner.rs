//! Shared execution for success and runtime-failure assertions.

use doriac::{backend::NativeProfile, mir::Program};
use std::io::Write;
use std::process::{Command, Stdio};

pub use crate::execution_directory::ExecutionDirectory;

pub fn execute_native(mir: &Program, profile: NativeProfile) -> std::process::Output {
    execute_native_with_input(mir, profile, None)
}

pub fn execute_native_with_input(
    mir: &Program,
    profile: NativeProfile,
    input: Option<&[u8]>,
) -> std::process::Output {
    let Some(input) = input else {
        return execute_native_with_args(mir, profile, &[]);
    };
    let directory = ExecutionDirectory::new();
    execute_native_with_io(mir, profile, &[], &directory, &[], Some(input))
}

pub fn execute_native_with_args(
    mir: &Program,
    profile: NativeProfile,
    args: &[&str],
) -> std::process::Output {
    let directory = ExecutionDirectory::new();
    execute_native_in_directory(mir, profile, args, &directory)
}

pub fn execute_native_in_directory(
    mir: &Program,
    profile: NativeProfile,
    args: &[&str],
    directory: &ExecutionDirectory,
) -> std::process::Output {
    execute_native_in_directory_with_env(mir, profile, args, directory, &[])
}

pub fn execute_native_in_directory_with_env(
    mir: &Program,
    profile: NativeProfile,
    args: &[&str],
    directory: &ExecutionDirectory,
    environment: &[(&str, &std::ffi::OsStr)],
) -> std::process::Output {
    execute_native_with_io(mir, profile, args, directory, environment, None)
}

/// Builds `mir` with `profile` and puts the executable in `directory`.
pub fn stage_executable(
    mir: &Program,
    profile: NativeProfile,
    directory: &ExecutionDirectory,
) -> std::path::PathBuf {
    let bytes = doriac::codegen_native::generate_executable(mir, profile)
        .unwrap_or_else(|error| panic!("{profile:?}: {error:?}"));
    let executable = directory.path().join(if cfg!(windows) {
        "program.exe"
    } else {
        "program"
    });
    // Replace by rename, never rewrite in place: macOS kills a rewritten
    // executable at launch against the code signature it cached for the
    // previous contents, and a directory runs several programs in turn.
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let staged = directory.path().join(format!(
        ".program-{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&staged, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::fs::rename(&staged, &executable).unwrap();
    executable
}

fn execute_native_with_io(
    mir: &Program,
    profile: NativeProfile,
    args: &[&str],
    directory: &ExecutionDirectory,
    environment: &[(&str, &std::ffi::OsStr)],
    input: Option<&[u8]>,
) -> std::process::Output {
    let executable = stage_executable(mir, profile, directory);
    let mut child = doriac::native_process::spawn(
        Command::new(&executable)
            .args(args)
            .envs(environment.iter().copied())
            .current_dir(directory.path())
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .unwrap();
    if let Some(input) = input {
        let mut stdin = child.stdin.take().unwrap();
        // Feed while wait_with_output drains both captures, so bounded tests
        // do not depend on the host's pipe capacities or producer ordering.
        std::thread::scope(|scope| {
            let writer = scope.spawn(move || stdin.write_all(input));
            let output = child.wait_with_output().unwrap();
            match writer.join().unwrap() {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {}
                Err(error) => panic!("native fixture input failed: {error}"),
            }
            output
        })
    } else {
        child.wait_with_output().unwrap()
    }
}
