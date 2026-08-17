use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use doriac::backend::NativeProfile;

const MANIFEST: &str = include_str!("fixtures/native_parity_examples.txt");
const BROKEN_PIPE_STDOUT: &str = include_str!("fixtures/native_io/broken_pipe_stdout.doria");
const BROKEN_PIPE_STDERR: &str = include_str!("fixtures/native_io/broken_pipe_stderr.doria");
const BROKEN_PIPE_STDOUT_BYTES: &str =
    include_str!("fixtures/native_io/broken_pipe_stdout_bytes.doria");
const BROKEN_PIPE_STDERR_BYTES: &str =
    include_str!("fixtures/native_io/broken_pipe_stderr_bytes.doria");
const BROKEN_PIPE_READ_LINE_PROMPT: &str = r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let $line = read_line("Name: ");
}
"#;

#[test]
fn release_panic_projection_preserves_output_and_omits_only_frames() {
    let expected = b"prefix\nPanic[P1000]: Program Panicked\n\nCall Path\nfail \xc2\xb7 source.doria:2\nmain \xc2\xb7 source.doria:5\n\nProcess Exited With Status 101\n";
    assert!(release_stderr_is_projection(
        expected,
        b"prefix\nPanic[P1000]: Program Panicked\n\nCall Path\nmain \xc2\xb7 source.doria:5\n\nProcess Exited With Status 101\n"
    ));
    assert!(!release_stderr_is_projection(
        expected,
        b"prefix\nPanic[P1000]: Program Panicked\n\nCall Path\nother \xc2\xb7 source.doria:5\n\nProcess Exited With Status 101\n"
    ));
}

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
fn interpreter_matches_every_durable_io_fixture() {
    let workspace = workspace_root();
    for relative_path in manifest_paths() {
        let path = workspace.join(&relative_path);
        let source = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("failed to read parity source {relative_path}: {error}")
        });
        let hir =
            doriac::lower_source(relative_path.clone(), source).unwrap_or_else(|diagnostics| {
                panic!("frontend rejected parity source {relative_path}: {diagnostics:#?}")
            });
        let mir = doriac::mir_lowering::lower_program(&hir).unwrap_or_else(|diagnostics| {
            panic!("MIR rejected parity source {relative_path}: {diagnostics:#?}")
        });
        let fixture = IoFixture::load(&workspace, &relative_path);
        let interpreted = doriac::mir_interpreter::interpret_with_io(
            &mir,
            doriac::mir_interpreter::MirIo {
                stdin: fixture.stdin.clone(),
                files: fixture.files.clone(),
                args: fixture.args.clone(),
                ..doriac::mir_interpreter::MirIo::default()
            },
        )
        .unwrap_or_else(|error| {
            panic!("interpreter rejected parity source {relative_path}: {error}")
        });
        fixture.assert_expected(&relative_path, &interpreted);
    }
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
                args: fixture.args.clone(),
                ..doriac::mir_interpreter::MirIo::default()
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
        assert_matches_interpreter(
            &relative_path,
            "Cranelift fast",
            NativeProfile::Fast,
            &interpreted,
            &fast,
        );

        #[cfg(feature = "llvm-backend")]
        {
            let release = compile_and_run(
                &mir,
                NativeProfile::Release,
                &relative_path,
                "LLVM",
                &fixture,
            );
            assert_matches_interpreter(
                &relative_path,
                "LLVM release",
                NativeProfile::Release,
                &interpreted,
                &release,
            );
        }
    }
}

#[test]
fn enabled_native_backends_exit_cleanly_when_an_output_pipe_closes() {
    if !host_linker_is_available() {
        let message = format!("native parity requires host linker {}", host_linker());
        if std::env::var_os("CI").is_some() {
            panic!("{message}; CI must not skip the parity matrix");
        }
        eprintln!("{message}; skipping local executable parity");
        return;
    }

    let binary_input = vec![0xa5; 128 * 1024];
    for (name, source, closed_stream, stdin) in [
        ("stdout", BROKEN_PIPE_STDOUT, ClosedStream::Stdout, &[][..]),
        ("stderr", BROKEN_PIPE_STDERR, ClosedStream::Stderr, &[][..]),
        (
            "stdout-bytes",
            BROKEN_PIPE_STDOUT_BYTES,
            ClosedStream::Stdout,
            binary_input.as_slice(),
        ),
        (
            "stderr-bytes",
            BROKEN_PIPE_STDERR_BYTES,
            ClosedStream::Stderr,
            binary_input.as_slice(),
        ),
    ] {
        let hir = doriac::lower_source(format!("broken_pipe_{name}.doria"), source.to_string())
            .unwrap_or_else(|diagnostics| {
                panic!("frontend rejected broken-pipe {name} fixture: {diagnostics:#?}")
            });
        let mir = doriac::mir_lowering::lower_program(&hir).unwrap_or_else(|diagnostics| {
            panic!("MIR rejected broken-pipe {name} fixture: {diagnostics:#?}")
        });
        let interpreted = doriac::mir_interpreter::interpret_with_io(
            &mir,
            doriac::mir_interpreter::MirIo {
                stdin: stdin.to_vec(),
                files: BTreeMap::new(),
                args: Vec::new(),
                ..doriac::mir_interpreter::MirIo::default()
            },
        )
        .unwrap_or_else(|error| panic!("interpreter rejected broken-pipe {name} fixture: {error}"))
        .output;
        assert_eq!(interpreted.exit_status, 0);
        let emitted = match closed_stream {
            ClosedStream::Stdout => &interpreted.stdout,
            ClosedStream::Stderr => &interpreted.stderr,
        };
        assert!(
            emitted.len() > 64 * 1024,
            "{name} fixture must exceed a typical pipe buffer"
        );

        assert_closed_output_pipe(&mir, NativeProfile::Fast, name, closed_stream, stdin);
        #[cfg(feature = "llvm-backend")]
        assert_closed_output_pipe(&mir, NativeProfile::Release, name, closed_stream, stdin);
    }
}

#[test]
fn prompted_read_line_exits_cleanly_when_stdout_closes_before_stdin() {
    if !host_linker_is_available() {
        let message = format!("native parity requires host linker {}", host_linker());
        if std::env::var_os("CI").is_some() {
            panic!("{message}; CI must not skip the prompted-input broken-pipe check");
        }
        eprintln!("{message}; skipping local prompted-input broken-pipe check");
        return;
    }

    let mir = doriac::lower_source_to_mir(
        "broken_pipe_read_line_prompt.doria",
        BROKEN_PIPE_READ_LINE_PROMPT,
    )
    .expect("prompted read_line broken-pipe fixture should lower");
    assert_closed_output_pipe(
        &mir,
        NativeProfile::Fast,
        "read-line-prompt",
        ClosedStream::Stdout,
        b"must not be read\n",
    );
    #[cfg(feature = "llvm-backend")]
    assert_closed_output_pipe(
        &mir,
        NativeProfile::Release,
        "read-line-prompt",
        ClosedStream::Stdout,
        b"must not be read\n",
    );
}

#[derive(Clone, Copy)]
enum ClosedStream {
    Stdout,
    Stderr,
}

fn assert_closed_output_pipe(
    mir: &doriac::mir::Program,
    profile: NativeProfile,
    name: &str,
    closed_stream: ClosedStream,
    stdin: &[u8],
) {
    let backend = match profile {
        NativeProfile::Fast => "Cranelift",
        NativeProfile::Release => "LLVM",
    };
    let bytes = doriac::codegen_native::generate_executable(mir, profile)
        .unwrap_or_else(|error| panic!("{backend} rejected broken-pipe {name} fixture: {error:?}"));
    let directory = temp_working_directory(&format!("broken-pipe-{backend}-{name}"));
    fs::create_dir_all(&directory).expect("broken-pipe working directory should be created");
    let executable = directory.join(if cfg!(windows) {
        "program.exe"
    } else {
        "program"
    });
    fs::write(&executable, bytes).expect("broken-pipe executable should be writable");
    make_executable(&executable);

    let mut child = retry_transient_executable_busy(|| {
        Command::new(&executable)
            .current_dir(&directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    })
    .unwrap_or_else(|error| panic!("failed to start {backend} {name} fixture: {error}"));
    match closed_stream {
        ClosedStream::Stdout => drop(child.stdout.take()),
        ClosedStream::Stderr => drop(child.stderr.take()),
    }
    if let Some(mut child_stdin) = child.stdin.take() {
        write_stdin_tolerating_early_close(&mut child_stdin, stdin)
            .unwrap_or_else(|error| panic!("failed to feed {backend} {name} fixture: {error}"));
    }
    let output = child
        .wait_with_output()
        .unwrap_or_else(|error| panic!("failed to wait for {backend} {name} fixture: {error}"));

    assert_eq!(
        output.status.code(),
        Some(0),
        "{backend} {name} broken pipe must be a clean exit"
    );
    assert!(output.stdout.is_empty(), "{backend} {name} wrote stdout");
    assert!(output.stderr.is_empty(), "{backend} {name} wrote stderr");
    fs::remove_dir_all(directory).expect("broken-pipe working directory should be removed");
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
    let output = run_native_executable(
        &executable,
        &working_directory,
        &fixture.stdin,
        &fixture.args,
    )
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
    profile: NativeProfile,
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
    if profile == NativeProfile::Release && interpreted.output.exit_status == 101 {
        assert!(
            release_stderr_is_projection(&interpreted.output.stderr, &native.output.stderr),
            "stderr mismatch for {relative_path} ({backend})\nexpected projection of: {:?}\nactual: {:?}",
            interpreted.output.stderr,
            native.output.stderr
        );
    } else {
        assert_eq!(
            native.output.stderr, interpreted.output.stderr,
            "stderr mismatch for {relative_path} ({backend})"
        );
    }
    assert_eq!(
        native.files, interpreted.files,
        "file side-effect mismatch for {relative_path} ({backend})"
    );
}

fn release_stderr_is_projection(expected: &[u8], actual: &[u8]) -> bool {
    const CALL_PATH_HEADER: &[u8] = b"Call Path\n";
    const STATUS_HEADER: &[u8] = b"\n\nProcess Exited With Status ";
    let Some(expected_header) = expected
        .windows(CALL_PATH_HEADER.len())
        .rposition(|window| window == CALL_PATH_HEADER)
    else {
        return actual == expected;
    };
    let Some(actual_header) = actual
        .windows(CALL_PATH_HEADER.len())
        .rposition(|window| window == CALL_PATH_HEADER)
    else {
        return false;
    };
    let expected_frames = expected_header + CALL_PATH_HEADER.len();
    let actual_frames = actual_header + CALL_PATH_HEADER.len();
    let Some(expected_status) = expected[expected_frames..]
        .windows(STATUS_HEADER.len())
        .position(|window| window == STATUS_HEADER)
        .map(|index| expected_frames + index)
    else {
        return false;
    };
    let Some(actual_status) = actual[actual_frames..]
        .windows(STATUS_HEADER.len())
        .position(|window| window == STATUS_HEADER)
        .map(|index| actual_frames + index)
    else {
        return false;
    };
    if expected[..expected_frames] != actual[..actual_frames]
        || expected[expected_status..] != actual[actual_status..]
    {
        return false;
    }

    let expected_frames = expected[expected_frames..expected_status]
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let mut next_expected = 0;
    let actual_frames = actual[actual_frames..actual_status]
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if actual_frames.is_empty() {
        return false;
    }
    for frame in actual_frames {
        if !frame.windows(3).any(|window| window == b" \xc2\xb7") {
            return false;
        }
        let Some(offset) = expected_frames[next_expected..]
            .iter()
            .position(|expected| *expected == frame)
        else {
            return false;
        };
        next_expected += offset + 1;
    }
    true
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

fn run_native_executable(
    executable: &Path,
    cwd: &Path,
    stdin: &[u8],
    args: &[String],
) -> io::Result<Output> {
    let mut child = retry_transient_executable_busy(|| {
        Command::new(executable)
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    })?;
    let mut child_stdin = child.stdin.take().expect("piped stdin should be available");
    write_stdin_tolerating_early_close(&mut child_stdin, stdin)?;
    drop(child_stdin);
    child.wait_with_output()
}

fn retry_transient_executable_busy<T>(
    mut operation: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    const MAX_ATTEMPTS: usize = 20;

    for attempt in 0..MAX_ATTEMPTS {
        match operation() {
            Ok(value) => return Ok(value),
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
    /// Program arguments for a `main(List<string> $args)` fixture, one per line
    /// of the fixture's `args` file (decision 0099).
    args: Vec<String>,
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
            args: read_optional(&root.join("args"))
                .map(|bytes| {
                    String::from_utf8(bytes)
                        .expect("fixture args should be UTF-8")
                        .lines()
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
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
    if path.exists() {
        return Some(
            fs::read(path)
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display())),
        );
    }
    let encoded = path.with_file_name(format!(
        "{}.hex",
        path.file_name()
            .expect("fixture path should have a file name")
            .to_string_lossy()
    ));
    encoded.exists().then(|| read_hex_fixture(&encoded))
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
            let mut relative = path
                .strip_prefix(root)
                .expect("fixture file should be under its root")
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = if relative.ends_with(".hex") {
                relative.truncate(relative.len() - ".hex".len());
                read_hex_fixture(&path)
            } else {
                fs::read(&path)
                    .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
            };
            files.insert(relative, bytes);
        }
    }
}

fn read_hex_fixture(path: &Path) -> Vec<u8> {
    let source = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    let digits = source
        .bytes()
        .filter(|byte| !byte.is_ascii_whitespace())
        .collect::<Vec<_>>();
    assert!(
        digits.len().is_multiple_of(2),
        "{} must contain complete hexadecimal byte pairs",
        path.display()
    );
    digits
        .chunks_exact(2)
        .map(|pair| {
            let high = (pair[0] as char)
                .to_digit(16)
                .unwrap_or_else(|| panic!("{} contains a non-hexadecimal digit", path.display()));
            let low = (pair[1] as char)
                .to_digit(16)
                .unwrap_or_else(|| panic!("{} contains a non-hexadecimal digit", path.display()));
            ((high << 4) | low) as u8
        })
        .collect()
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

/// Proves the prompt is observable *before* input is supplied, not merely ordered
/// correctly in the final output after the process has exited.
///
/// The test reads the exact prompt bytes from the child's stdout first, and only
/// then writes the input line. If the runtime deferred the prompt until input
/// arrived, or skipped the pre-read flush, this would deadlock rather than pass —
/// so the bounded read is the assertion. The reader thread bounds the wait and the
/// child is killed and reaped on every exit path.
#[test]
fn prompted_read_line_writes_its_prompt_before_input_is_supplied() {
    const PROMPT: &[u8] = b"Name: ";
    let source = r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let $name = read_line("Name: ");

    if ($name != null) {
        echo "Hello, {$name}!";
    }
}
"#;

    let mir = doriac::lower_source_to_mir("prompt-timing.doria", source)
        .expect("prompted read_line should lower");

    #[cfg(feature = "llvm-backend")]
    let profiles = vec![NativeProfile::Fast, NativeProfile::Release];
    #[cfg(not(feature = "llvm-backend"))]
    let profiles = vec![NativeProfile::Fast];

    for profile in profiles {
        let backend = match profile {
            NativeProfile::Fast => "Cranelift",
            NativeProfile::Release => "LLVM",
        };
        let bytes =
            doriac::codegen_native::generate_executable(&mir, profile).unwrap_or_else(|error| {
                panic!("{backend} rejected the prompt-timing fixture: {error:?}")
            });
        let directory = temp_working_directory(&format!("prompt-timing-{backend}"));
        fs::create_dir_all(&directory).expect("prompt-timing working directory should be created");
        let executable = directory.join(if cfg!(windows) {
            "program.exe"
        } else {
            "program"
        });
        fs::write(&executable, bytes).expect("prompt-timing executable should be writable");
        make_executable(&executable);

        let mut child = retry_transient_executable_busy(|| {
            Command::new(&executable)
                .current_dir(&directory)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
        })
        .unwrap_or_else(|error| {
            panic!("failed to start the {backend} prompt-timing fixture: {error}")
        });

        // Read exactly the prompt on a worker thread so the wait is bounded.
        let mut stdout = child.stdout.take().expect("child stdout should be piped");
        let (sender, receiver) = std::sync::mpsc::channel();
        let reader = thread::spawn(move || {
            use std::io::Read;
            let mut seen = vec![0_u8; PROMPT.len()];
            let result = stdout.read_exact(&mut seen).map(|()| seen);
            let _ = sender.send(result.map_err(|error| error.to_string()));
            stdout
        });

        let observed = receiver.recv_timeout(Duration::from_secs(30));
        let observed = match observed {
            Ok(observed) => observed,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                panic!(
                    "the {backend} prompt-timing fixture never produced its prompt before input: {error}"
                );
            }
        };
        let observed = match observed {
            Ok(observed) => observed,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                panic!("failed to read the {backend} prompt before supplying input: {error}");
            }
        };
        assert_eq!(
            observed, PROMPT,
            "{backend} must write the exact prompt bytes before reading stdin"
        );

        // Only now supply the input.
        let mut child_stdin = child.stdin.take().expect("child stdin should be piped");
        write_stdin_tolerating_early_close(&mut child_stdin, b"Dorothy\n").unwrap_or_else(
            |error| panic!("failed to feed the {backend} prompt-timing fixture: {error}"),
        );
        drop(child_stdin);

        let mut stdout = reader
            .join()
            .expect("prompt reader thread should not panic");
        let mut rest = Vec::new();
        {
            use std::io::Read;
            stdout
                .read_to_end(&mut rest)
                .expect("remaining stdout should be readable");
        }
        let status = child.wait().unwrap_or_else(|error| {
            panic!("failed to wait for the {backend} prompt-timing fixture: {error}")
        });
        let mut stderr = Vec::new();
        if let Some(mut handle) = child.stderr.take() {
            use std::io::Read;
            let _ = handle.read_to_end(&mut stderr);
        }

        assert_eq!(
            rest, b"Hello, Dorothy!",
            "{backend} produced unexpected output"
        );
        assert!(stderr.is_empty(), "{backend} produced unexpected stderr");
        assert_eq!(
            status.code(),
            Some(0),
            "{backend} produced unexpected status"
        );
    }
}

/// How many passes each stack-growth fixture makes over its loop.
///
/// A scratch slot emitted outside its function's entry block is a dynamic stack
/// allocation: it moves the stack pointer when it executes and is not reclaimed
/// until the function returns, so a loop carrying one walks the stack down until
/// it strikes the guard page. The defect that prompted these fixtures cost
/// between 15 and 44 bytes per pass, which on a default 8 MB stack killed them
/// between roughly 173,000 and 533,000 iterations. This count clears the most
/// forgiving of those by nearly four times, so it still fails on a leak a
/// quarter that size.
///
/// Each fixture's checksum accumulates over every pass, so the exact-output
/// assertions below also prove the loop ran this many times: a fixture that
/// stopped early would report a different number rather than pass quietly.
const STACK_GROWTH_ITERATIONS: u64 = 2_000_000;

const STACK_GROWTH_MODULUS: u64 = 1_000_003;

/// Loop bodies whose lowering allocates a scratch slot, run far past the point
/// where a per-iteration leak exhausts the stack.
///
/// This asserts the behaviour a user sees — status and bytes on stdout — rather
/// than the shape of the emitted IR, and it asserts it on every native profile
/// the build enables. `llvm_mir_tests::allocates_every_scratch_slot_in_the_entry_block`
/// covers the same invariant structurally, on the module the backend emits
/// before optimization; this covers the program that reaches the user.
///
/// The expected output comes from an independent Rust model of each loop rather
/// than from a Doria backend, so a backend cannot supply the value it is checked
/// against. The MIR interpreter is not used as the oracle here because these
/// counts are far beyond what it can execute in test time.
#[test]
fn native_profiles_keep_loop_body_stack_use_constant() {
    if !host_linker_is_available() {
        let message = format!(
            "native stack-growth coverage requires host linker {}",
            host_linker()
        );
        if std::env::var_os("CI").is_some() {
            panic!("{message}; CI must not skip the stack-growth matrix");
        }
        eprintln!("{message}; skipping local stack-growth coverage");
        return;
    }

    let iterations = STACK_GROWTH_ITERATIONS;
    let cases = [
        (
            "dictionary_get",
            include_str!("fixtures/native_stack/dictionary_get.doria"),
            expected_dictionary_get(iterations),
        ),
        (
            "dictionary_set_remove",
            include_str!("fixtures/native_stack/dictionary_set_remove.doria"),
            expected_dictionary_set_remove(iterations),
        ),
        (
            "set_add_remove",
            include_str!("fixtures/native_stack/set_add_remove.doria"),
            expected_set_add_remove(iterations),
        ),
        (
            "set_contains",
            include_str!("fixtures/native_stack/set_contains.doria"),
            expected_set_contains(iterations),
        ),
        (
            "list_pop",
            include_str!("fixtures/native_stack/list_pop.doria"),
            expected_list_pop(iterations),
        ),
        (
            "list_index",
            include_str!("fixtures/native_stack/list_index.doria"),
            expected_list_index(iterations),
        ),
        (
            "string_search",
            include_str!("fixtures/native_stack/string_search.doria"),
            expected_string_search(iterations),
        ),
        (
            "string_interpolation",
            include_str!("fixtures/native_stack/string_interpolation.doria"),
            expected_string_interpolation(iterations),
        ),
        (
            "int_parse",
            include_str!("fixtures/native_stack/int_parse.doria"),
            expected_int_parse(iterations),
        ),
        (
            "dictionary_index",
            include_str!("fixtures/native_stack/dictionary_index.doria"),
            expected_dictionary_index(iterations),
        ),
        (
            "list_first",
            include_str!("fixtures/native_stack/list_first.doria"),
            expected_list_first(iterations),
        ),
        (
            "collection_clear",
            include_str!("fixtures/native_stack/collection_clear.doria"),
            expected_collection_clear(iterations),
        ),
    ];

    #[cfg(feature = "llvm-backend")]
    let profiles = [NativeProfile::Fast, NativeProfile::Release];
    #[cfg(not(feature = "llvm-backend"))]
    let profiles = [NativeProfile::Fast];

    for (name, source, expected) in cases {
        let mir = doriac::lower_source_to_mir(format!("{name}.doria"), source)
            .unwrap_or_else(|error| panic!("stack-growth fixture {name} should lower: {error:?}"));

        for profile in profiles {
            let backend = match profile {
                NativeProfile::Fast => "Cranelift fast",
                NativeProfile::Release => "LLVM release",
            };
            let output = run_stack_growth_fixture(&mir, profile, backend, name, iterations);

            assert!(
                output.status.success(),
                "{backend} exited {} after {iterations} passes over the {name} loop; \
                 a scratch slot allocated outside the entry block grows the frame on \
                 every pass until the stack is exhausted\nstderr:\n{}",
                describe_exit_status(&output.status),
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                String::from_utf8_lossy(&output.stdout),
                expected,
                "{backend} produced unexpected output for {name}"
            );
            assert!(
                output.stderr.is_empty(),
                "{backend} produced unexpected stderr for {name}:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

fn run_stack_growth_fixture(
    mir: &doriac::mir::Program,
    profile: NativeProfile,
    backend: &str,
    name: &str,
    iterations: u64,
) -> Output {
    let bytes = doriac::codegen_native::generate_executable(mir, profile).unwrap_or_else(|error| {
        panic!("{backend} rejected stack-growth fixture {name}: {error:?}")
    });
    let directory = temp_working_directory(&format!("stack-growth-{name}-{backend}"));
    fs::create_dir_all(&directory)
        .unwrap_or_else(|error| panic!("failed to create working directory for {name}: {error}"));
    let executable = directory.join(if cfg!(windows) {
        "program.exe"
    } else {
        "program"
    });
    fs::write(&executable, bytes)
        .unwrap_or_else(|error| panic!("failed to write {backend} executable for {name}: {error}"));
    make_executable(&executable);

    let output = run_native_executable(&executable, &directory, &[], &[iterations.to_string()])
        .unwrap_or_else(|error| panic!("failed to run {backend} executable for {name}: {error}"));

    fs::remove_dir_all(&directory)
        .unwrap_or_else(|error| panic!("failed to clean working directory for {name}: {error}"));
    output
}

/// Names the way a process ended. A stack-exhausted program dies by signal and
/// reports no exit code at all, so a bare code would print `None` for exactly
/// the failure these fixtures guard.
fn describe_exit_status(status: &std::process::ExitStatus) -> String {
    if let Some(code) = status.code() {
        return format!("with status {code}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            let name = match signal {
                6 => " (SIGABRT)",
                11 => " (SIGSEGV)",
                _ => "",
            };
            return format!("by signal {signal}{name}");
        }
    }
    "for an unknown reason".to_string()
}

// Independent Rust models of the fixture loops. Each mirrors the Doria source
// statement for statement so the expected output is derived from the loop the
// fixture spells, not from a backend's answer to it.

fn expected_dictionary_get(iterations: u64) -> String {
    // The fixture fills the dictionary so that "key{j}" maps to j for j < 64.
    let mut checksum = 0;
    for index in 0..iterations {
        checksum = (checksum + index % 64) % STACK_GROWTH_MODULUS;
    }
    format!("{checksum}\n")
}

fn expected_collection_clear(iterations: u64) -> String {
    format!("{}:0:0:0:0\n", (iterations * 4) % STACK_GROWTH_MODULUS)
}

fn expected_dictionary_set_remove(iterations: u64) -> String {
    let mut values: BTreeMap<u64, u64> = BTreeMap::new();
    let mut checksum = 0;
    for index in 0..iterations {
        values.insert(index % 64, index);
        checksum = (checksum + values.remove(&(index % 64)).unwrap_or(0)) % STACK_GROWTH_MODULUS;
    }
    format!("{checksum}:{}\n", values.len())
}

fn expected_set_add_remove(iterations: u64) -> String {
    let mut members: BTreeSet<u64> = BTreeSet::new();
    let mut changes = 0;
    for index in 0..iterations {
        if members.insert(index % 64) {
            changes = (changes + 1) % STACK_GROWTH_MODULUS;
        }
        if members.remove(&(index % 64)) {
            changes = (changes + 2) % STACK_GROWTH_MODULUS;
        }
    }
    format!("{changes}:{}\n", members.len())
}

fn expected_set_contains(iterations: u64) -> String {
    let members: BTreeSet<u64> = (0..64).map(|index| index * 2).collect();
    let mut hits = 0;
    for index in 0..iterations {
        if members.contains(&(index % 16)) {
            hits = (hits + 1) % STACK_GROWTH_MODULUS;
        }
    }
    format!("{hits}\n")
}

fn expected_list_pop(iterations: u64) -> String {
    let mut values: Vec<u64> = Vec::new();
    let mut checksum = 0;
    for index in 0..iterations {
        values.push(index % 97);
        checksum = (checksum + values.pop().unwrap_or(0)) % STACK_GROWTH_MODULUS;
    }
    format!("{checksum}:{}\n", values.len())
}

fn expected_list_index(iterations: u64) -> String {
    let values: Vec<u64> = (0..64).collect();
    let mut checksum = 0;
    for index in 0..iterations {
        checksum = (checksum + values[(index % 64) as usize]) % STACK_GROWTH_MODULUS;
    }
    format!("{checksum}\n")
}

fn expected_dictionary_index(iterations: u64) -> String {
    // The fixture maps every key j to j for j < 64, so no lookup is missing and
    // the `?? 0` fallback never applies.
    let values: BTreeMap<u64, u64> = (0..64).map(|index| (index, index)).collect();
    let mut checksum = 0;
    for index in 0..iterations {
        checksum =
            (checksum + values.get(&(index % 64)).copied().unwrap_or(0)) % STACK_GROWTH_MODULUS;
    }
    format!("{checksum}\n")
}

fn expected_list_first(iterations: u64) -> String {
    let values: Vec<u64> = (0..64).collect();
    let first = values.first().copied().unwrap_or(0);
    let last = values.last().copied().unwrap_or(0);
    let mut checksum = 0;
    for _ in 0..iterations {
        checksum = (checksum + first + last) % STACK_GROWTH_MODULUS;
    }
    format!("{checksum}\n")
}

fn expected_string_search(iterations: u64) -> String {
    // The fixture's haystack and needle are ASCII, so the byte offset Rust
    // reports is also the grapheme index Doria reports.
    const HAYSTACK: &str = "the quick brown fox jumps over the lazy dog";
    const NEEDLE: &str = "fox";
    let position = HAYSTACK
        .find(NEEDLE)
        .expect("the fixture needle occurs in its haystack") as u64;
    let mut checksum = 0;
    for _ in 0..iterations {
        checksum = (checksum + position) % STACK_GROWTH_MODULUS;
    }
    format!("{checksum}\n")
}

fn expected_string_interpolation(iterations: u64) -> String {
    // "value {index} of {iterations}" is ASCII, so its grapheme length is the
    // literal text plus the digits of each interpolated number.
    let literal_text = "value ".len() as u64 + " of ".len() as u64;
    let trailing = literal_text + decimal_digits(iterations);
    let mut lengths = 0;
    for index in 0..iterations {
        lengths = (lengths + decimal_digits(index) + trailing) % STACK_GROWTH_MODULUS;
    }
    format!("{lengths}\n")
}

fn expected_int_parse(iterations: u64) -> String {
    let parsed: u64 = "1234".parse().expect("the fixture literal parses");
    let mut checksum = 0;
    for _ in 0..iterations {
        checksum = (checksum + parsed) % STACK_GROWTH_MODULUS;
    }
    format!("{checksum}\n")
}

fn decimal_digits(value: u64) -> u64 {
    let mut digits = 1;
    let mut remaining = value;
    while remaining >= 10 {
        remaining /= 10;
        digits += 1;
    }
    digits
}
