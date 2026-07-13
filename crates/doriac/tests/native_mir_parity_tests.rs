use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use doriac::backend::NativeProfile;

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
fn interpreter_cranelift_and_enabled_llvm_match_for_the_durable_native_manifest() {
    if !host_linker_is_available() {
        let message = format!("native parity requires host linker {}", host_linker());
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
            panic!("failed to read parity source {relative_path}: {error}")
        });
        let hir = doriac::lower_source(relative_path.clone(), source.clone()).unwrap_or_else(
            |diagnostics| {
                panic!("frontend rejected parity source {relative_path}: {diagnostics:#?}")
            },
        );
        let mir = doriac::mir_lowering::lower_program(&hir).unwrap_or_else(|diagnostics| {
            panic!("MIR rejected parity source {relative_path}: {diagnostics:#?}")
        });
        let fixture = IoFixture::load(&workspace, &relative_path);
        let interpreted = doriac::mir_interpreter::interpret_with_io(
            &mir,
            doriac::mir_interpreter::MirIo {
                stdin: fixture.stdin.clone(),
                files: fixture.files.clone(),
            },
        )
        .unwrap_or_else(|error| {
            panic!("interpreter rejected parity source {relative_path}: {error}")
        });
        fixture.assert_expected(&relative_path, &interpreted);

        let fast = compile_and_run(
            &mir,
            NativeProfile::Fast,
            &relative_path,
            "Cranelift",
            &fixture,
        );
        assert_matches_interpreter(&relative_path, "Cranelift fast", &interpreted, &fast);

        #[cfg(feature = "llvm-backend")]
        {
            let release = compile_and_run(
                &mir,
                NativeProfile::Release,
                &relative_path,
                "LLVM",
                &fixture,
            );
            assert_matches_interpreter(&relative_path, "LLVM release", &interpreted, &release);
        }
    }
}

fn compile_and_run(
    mir: &doriac::mir::Program,
    profile: NativeProfile,
    relative_path: &str,
    backend: &str,
    fixture: &IoFixture,
) -> NativeRun {
    let bytes = doriac::codegen_native::generate_executable(mir, profile).unwrap_or_else(|error| {
        panic!("{backend} backend rejected parity source {relative_path}: {error:?}")
    });
    let working_directory = temp_working_directory(&format!("{backend}-{relative_path}"));
    fs::create_dir_all(&working_directory).unwrap_or_else(|error| {
        panic!("failed to create isolated directory for {relative_path}: {error}")
    });
    fixture.seed_native_files(&working_directory, relative_path);
    let executable = working_directory.join(if cfg!(windows) {
        "program.exe"
    } else {
        "program"
    });
    fs::write(&executable, bytes).unwrap_or_else(|error| {
        panic!("failed to write {backend} parity executable for {relative_path}: {error}")
    });
    make_executable(&executable);
    let output = run_native_executable(&executable, &working_directory, &fixture.stdin)
        .unwrap_or_else(|error| {
            panic!("failed to run {backend} parity executable for {relative_path}: {error}")
        });
    let mut files = read_tree(&working_directory);
    files.remove(if cfg!(windows) {
        "program.exe"
    } else {
        "program"
    });
    fs::remove_dir_all(&working_directory).unwrap_or_else(|error| {
        panic!("failed to clean isolated directory for {relative_path}: {error}")
    });
    NativeRun { output, files }
}

fn assert_matches_interpreter(
    relative_path: &str,
    backend: &str,
    interpreted: &doriac::mir_interpreter::InterpreterIoOutput,
    native: &NativeRun,
) {
    let native_status = native.output.status.code();
    assert_eq!(
        native_status,
        Some(interpreted.output.exit_status),
        "status mismatch for {relative_path} ({backend})"
    );
    assert_eq!(
        native.output.stdout, interpreted.output.stdout,
        "stdout mismatch for {relative_path} ({backend})"
    );
    assert_eq!(
        native.output.stderr, interpreted.output.stderr,
        "stderr mismatch for {relative_path} ({backend})"
    );
    assert_eq!(
        native.files, interpreted.files,
        "file side-effect mismatch for {relative_path} ({backend})"
    );
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

fn temp_working_directory(source: &str) -> PathBuf {
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
    std::env::temp_dir().join(format!(
        "doriac-native-parity-{stem}-{}-{nanos}",
        std::process::id()
    ))
}

fn run_native_executable(executable: &Path, cwd: &Path, stdin: &[u8]) -> io::Result<Output> {
    const MAX_ATTEMPTS: usize = 20;

    for attempt in 0..MAX_ATTEMPTS {
        match Command::new(executable)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(mut child) => {
                let mut child_stdin = child.stdin.take().expect("piped stdin should be available");
                write_stdin_tolerating_early_close(&mut child_stdin, stdin)?;
                drop(child_stdin);
                return child.wait_with_output();
            }
            Err(error) if is_transient_executable_busy(&error) && attempt + 1 < MAX_ATTEMPTS => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("retry loop returns on its final attempt")
}

fn write_stdin_tolerating_early_close(child_stdin: &mut dyn Write, stdin: &[u8]) -> io::Result<()> {
    match child_stdin.write_all(stdin) {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result,
    }
}

#[test]
fn parity_runner_tolerates_an_executable_closing_stdin_early() {
    struct ClosedStdin;

    impl Write for ClosedStdin {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "executable closed stdin",
            ))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    assert!(write_stdin_tolerating_early_close(&mut ClosedStdin, b"unused input").is_ok());
}

#[derive(Debug)]
struct NativeRun {
    output: Output,
    files: BTreeMap<String, Vec<u8>>,
}

#[derive(Debug, Default)]
struct IoFixture {
    stdin: Vec<u8>,
    files: BTreeMap<String, Vec<u8>>,
    expected_files: BTreeMap<String, Vec<u8>>,
    expected_stdout: Option<Vec<u8>>,
    expected_stderr: Option<Vec<u8>>,
    expected_status: Option<i32>,
}

impl IoFixture {
    fn load(workspace: &Path, relative_path: &str) -> Self {
        let stem = Path::new(relative_path)
            .file_stem()
            .expect("parity source should have a file stem");
        let root = workspace
            .join("crates/doriac/tests/fixtures/native_io")
            .join(stem);
        if !root.exists() {
            return Self::default();
        }
        let expected_status = read_optional(&root.join("expected_status")).map(|bytes| {
            std::str::from_utf8(&bytes)
                .expect("expected_status should be UTF-8")
                .trim()
                .parse()
                .expect("expected_status should contain a decimal process status")
        });
        Self {
            stdin: read_optional(&root.join("stdin")).unwrap_or_default(),
            files: read_tree(&root.join("files")),
            expected_files: read_tree(&root.join("expected_files")),
            expected_stdout: read_optional(&root.join("expected_stdout")),
            expected_stderr: read_optional(&root.join("expected_stderr")),
            expected_status,
        }
    }

    fn assert_expected(
        &self,
        relative_path: &str,
        interpreted: &doriac::mir_interpreter::InterpreterIoOutput,
    ) {
        if let Some(expected) = &self.expected_stdout {
            assert_eq!(
                &interpreted.output.stdout, expected,
                "stdout fixture mismatch for {relative_path}"
            );
        }
        if let Some(expected) = &self.expected_stderr {
            assert_eq!(
                &interpreted.output.stderr, expected,
                "stderr fixture mismatch for {relative_path}"
            );
        }
        if let Some(expected) = self.expected_status {
            assert_eq!(
                interpreted.output.exit_status, expected,
                "status fixture mismatch for {relative_path}"
            );
        }
        for (path, expected) in &self.expected_files {
            assert_eq!(
                interpreted.files.get(path),
                Some(expected),
                "file fixture mismatch for {relative_path}: {path}"
            );
        }
    }

    fn seed_native_files(&self, root: &Path, relative_path: &str) {
        for (path, bytes) in &self.files {
            let destination = root.join(path);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).unwrap_or_else(|error| {
                    panic!("failed to create seeded directory for {relative_path}: {error}")
                });
            }
            fs::write(&destination, bytes).unwrap_or_else(|error| {
                panic!("failed to seed {path} for {relative_path}: {error}")
            });
        }
    }
}

fn read_optional(path: &Path) -> Option<Vec<u8>> {
    path.exists().then(|| {
        fs::read(path).unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
    })
}

fn read_tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    if root.exists() {
        read_tree_into(root, root, &mut files);
    }
    files
}

fn read_tree_into(root: &Path, directory: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
    for entry in fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()))
    {
        let path = entry.expect("fixture entry should be readable").path();
        if path.is_dir() {
            read_tree_into(root, &path, files);
        } else {
            let relative = path
                .strip_prefix(root)
                .expect("fixture file should be under its root")
                .to_string_lossy()
                .replace('\\', "/");
            files.insert(
                relative,
                fs::read(&path)
                    .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display())),
            );
        }
    }
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
