use doriac::{backend::NativeProfile, mir::Program};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

struct NativeExecutionFiles {
    directory: PathBuf,
    executable: PathBuf,
}

impl Drop for NativeExecutionFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.executable);
        let _ = std::fs::remove_dir(&self.directory);
    }
}

pub fn assert_native_execution(mir: &Program, profile: NativeProfile, stdout: &str) {
    static NEXT_RUN: AtomicU64 = AtomicU64::new(0);
    let bytes = doriac::codegen_native::generate_executable(mir, profile)
        .unwrap_or_else(|error| panic!("{profile:?}: {error:?}"));
    let directory = std::env::temp_dir().join(format!(
        "doria-mir-execution-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_RUN.fetch_add(1, Ordering::Relaxed),
    ));
    std::fs::create_dir(&directory).unwrap();
    let executable = directory.join(if cfg!(windows) {
        "program.exe"
    } else {
        "program"
    });
    let files = NativeExecutionFiles {
        directory,
        executable,
    };
    std::fs::write(&files.executable, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&files.executable, std::fs::Permissions::from_mode(0o700))
            .unwrap();
    }
    let result = doriac::native_process::spawn(
        Command::new(&files.executable)
            .current_dir(&files.directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .unwrap()
    .wait_with_output()
    .unwrap();
    assert_eq!(result.status.code(), Some(0), "{profile:?}: {result:?}");
    assert_eq!(result.stderr, b"", "{profile:?}");
    assert_eq!(result.stdout, stdout.as_bytes(), "{profile:?}");
}
