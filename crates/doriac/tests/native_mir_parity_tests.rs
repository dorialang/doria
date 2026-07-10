use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use doriac::backend::{Backend, BackendOutput, NativeBackend};

const MANIFEST: &str = include_str!("fixtures/native_parity_examples.txt");

#[test]
fn manifest_covers_every_native_example() {
    let workspace = workspace_root();
    let manifest = manifest_paths();
    let native_directory = workspace.join("examples/native");
    let examples = fs::read_dir(native_directory)
        .expect("native examples directory should be readable")
        .map(|entry| {
            entry
                .expect("native example entry should be readable")
                .path()
        })
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "doria")
        })
        .map(|path| {
            path.strip_prefix(&workspace)
                .expect("native example should be inside the workspace")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<BTreeSet<_>>();

    assert_eq!(manifest, examples);
}

#[test]
fn interpreter_and_cranelift_match_for_the_durable_native_manifest() {
    if !host_linker_is_available() {
        let message = format!("native parity requires host linker `{}`", host_linker());
        if std::env::var_os("CI").is_some() {
            panic!("{message}; CI must not skip the parity matrix");
        }
        eprintln!("{message}; skipping local executable parity");
        return;
    }

    let workspace = workspace_root();
    for relative_path in manifest_paths() {
        let path = workspace.join(&relative_path);
        let source = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("failed to read parity source `{relative_path}`: {error}")
        });
        let hir = doriac::lower_source(relative_path.clone(), source.clone()).unwrap_or_else(
            |diagnostics| {
                panic!("frontend rejected parity source `{relative_path}`: {diagnostics:#?}")
            },
        );
        let mir = doriac::mir_lowering::lower_program(&hir).unwrap_or_else(|diagnostics| {
            panic!("MIR rejected parity source `{relative_path}`: {diagnostics:#?}")
        });
        let interpreted = doriac::mir_interpreter::interpret(&mir).unwrap_or_else(|error| {
            panic!("interpreter rejected parity source `{relative_path}`: {error}")
        });

        let native = NativeBackend.emit(&hir).unwrap_or_else(|error| {
            panic!("native backend rejected parity source `{relative_path}`: {error:?}")
        });
        let BackendOutput::Executable { bytes, .. } = native else {
            panic!("native backend returned non-executable output for `{relative_path}`");
        };
        let executable = temp_executable_path(&relative_path);
        fs::write(&executable, bytes).unwrap_or_else(|error| {
            panic!("failed to write parity executable for `{relative_path}`: {error}")
        });
        make_executable(&executable);
        let native_output = run_native_executable(&executable).unwrap_or_else(|error| {
            panic!("failed to run parity executable for `{relative_path}`: {error}")
        });
        let _ = fs::remove_file(&executable);

        let native_status = native_output.status.code();
        assert_eq!(
            native_status,
            Some(interpreted.exit_status),
            "status mismatch for {relative_path}\ninterpreter status: {}\nnative status: {:?}\ninterpreter stdout: {:?}\nnative stdout: {:?}",
            interpreted.exit_status,
            native_status,
            interpreted.stdout,
            native_output.stdout
        );
        assert_eq!(
            native_output.stdout,
            interpreted.stdout,
            "stdout mismatch for {relative_path}\ninterpreter status: {}\nnative status: {:?}\ninterpreter stdout: {:?}\nnative stdout: {:?}",
            interpreted.exit_status,
            native_status,
            interpreted.stdout,
            native_output.stdout
        );
        assert_eq!(
            native_output.stderr,
            interpreted.stderr,
            "stderr mismatch for {relative_path}\ninterpreter status: {}\nnative status: {:?}\ninterpreter stderr: {:?}\nnative stderr: {:?}",
            interpreted.exit_status,
            native_status,
            interpreted.stderr,
            native_output.stderr
        );
    }
}

fn manifest_paths() -> BTreeSet<String> {
    MANIFEST
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate should live under crates/doriac")
        .to_path_buf()
}

fn host_linker_is_available() -> bool {
    let linker = host_linker();
    let mut command = Command::new(&linker);
    if cfg!(windows) {
        command.arg("/?");
    } else {
        command.arg("--version");
    }
    command.output().is_ok()
}

fn host_linker() -> String {
    std::env::var("CC").unwrap_or_else(|_| default_linker().to_string())
}

fn default_linker() -> &'static str {
    if cfg!(windows) {
        "cl.exe"
    } else {
        "cc"
    }
}

fn temp_executable_path(source: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let stem = source
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let extension = if cfg!(windows) { ".exe" } else { "" };
    std::env::temp_dir().join(format!(
        "doriac-native-parity-{stem}-{}-{nanos}{extension}",
        std::process::id()
    ))
}

fn run_native_executable(executable: &Path) -> io::Result<Output> {
    const MAX_ATTEMPTS: usize = 20;

    for attempt in 0..MAX_ATTEMPTS {
        match Command::new(executable).output() {
            Ok(output) => return Ok(output),
            Err(error) if is_transient_executable_busy(&error) && attempt + 1 < MAX_ATTEMPTS => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("retry loop returns on its final attempt")
}

fn is_transient_executable_busy(error: &io::Error) -> bool {
    cfg!(unix) && error.raw_os_error() == Some(26)
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .expect("parity executable metadata should be readable")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("parity executable should be executable");
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}
