use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use doriac::backend::BackendOutput;
use doriac::build_plan::{BuildNativeProfile, CompilerTarget};

const FIXTURE_ROOT: &str = "tests/fixtures/stage31_package_graph";
const MANIFEST: &str = include_str!("fixtures/stage31_package_graph/manifest.txt");

#[test]
fn manifest_covers_every_stage31_package_project() {
    let manifest = fixture_names();
    let disk = fs::read_dir(fixture_root())
        .expect("Stage 31 fixture directory")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(manifest, disk);
}

#[test]
fn package_graph_executes_identically_across_all_enabled_backends() {
    for fixture in fixture_names() {
        let root = fixture_root().join(&fixture);
        let plan_path = root.join("build-plan.json");
        let expected_stdout = fs::read(root.join("expected_stdout")).expect("expected stdout");
        let (_, graph) = doriac::load_build_plan_file(&plan_path).expect("load package graph");

        let debug = doriac::compile_compilation_graph(&graph).expect("debug package graph");
        let BackendOutput::Text { contents, .. } = debug else {
            panic!("debug backend must emit text");
        };
        let expected_debug = format!(
            "exit_status: 0\nstdout: {}\n",
            String::from_utf8_lossy(&expected_stdout)
        );
        assert_eq!(contents, expected_debug, "debug fixture {fixture}");

        if host_linker_is_available() {
            let mut native_graph = graph.clone();
            native_graph.build_plan.compiler.target = CompilerTarget::Native;
            native_graph.build_plan.compiler.native_profile = Some(BuildNativeProfile::Fast);
            let native =
                doriac::compile_compilation_graph(&native_graph).expect("Cranelift package graph");
            assert_native_output(&fixture, "cranelift", native, &root, &expected_stdout);

            #[cfg(feature = "llvm-backend")]
            {
                native_graph.build_plan.compiler.native_profile = Some(BuildNativeProfile::Release);
                let llvm =
                    doriac::compile_compilation_graph(&native_graph).expect("LLVM package graph");
                assert_native_output(&fixture, "llvm", llvm, &root, &expected_stdout);
            }
        }

        if php_is_available() {
            let mut php_graph = graph;
            php_graph.build_plan.compiler.target = CompilerTarget::Php;
            php_graph.build_plan.compiler.native_profile = None;
            let php = doriac::compile_compilation_graph(&php_graph).expect("PHP package graph");
            let BackendOutput::Text { contents, .. } = php else {
                panic!("PHP backend must emit text");
            };
            assert!(!contents.lines().any(|line| {
                let line = line.trim_start();
                line.starts_with("include ")
                    || line.starts_with("include(")
                    || line.starts_with("require ")
                    || line.starts_with("require(")
            }));
            assert!(!contents.contains("spl_autoload_register"));
            let php_path = temporary_path(&fixture, "php");
            fs::write(&php_path, contents).expect("write generated PHP");
            let output = Command::new("php")
                .arg(&php_path)
                .current_dir(&root)
                .output()
                .expect("run generated PHP");
            let _ = fs::remove_file(&php_path);
            assert_run(&fixture, "php", &output, &expected_stdout);
        }
    }
}

fn assert_native_output(
    fixture: &str,
    backend: &str,
    output: BackendOutput,
    working_directory: &Path,
    expected_stdout: &[u8],
) {
    let BackendOutput::Executable { bytes, extension } = output else {
        panic!("{backend} backend must emit an executable");
    };
    let executable = temporary_path(fixture, &extension);
    fs::write(&executable, bytes).expect("write native executable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&executable)
            .expect("native executable metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable, permissions).expect("make native executable runnable");
    }
    let output = retry_transient_executable_busy(|| {
        Command::new(&executable)
            .current_dir(working_directory)
            .output()
    })
    .expect("run native executable");
    let _ = fs::remove_file(&executable);
    assert_run(fixture, backend, &output, expected_stdout);
}

fn assert_run(fixture: &str, backend: &str, output: &Output, expected_stdout: &[u8]) {
    assert_eq!(output.status.code(), Some(0), "{fixture} on {backend}");
    assert_eq!(output.stdout, expected_stdout, "{fixture} on {backend}");
    assert!(
        output.stderr.is_empty(),
        "{fixture} on {backend}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_ROOT)
}

fn fixture_names() -> BTreeSet<String> {
    MANIFEST
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect()
}

fn temporary_path(stem: &str, extension: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let mut path = std::env::temp_dir().join(format!(
        "doriac-stage31-{stem}-{}-{nanos}",
        std::process::id()
    ));
    if !extension.is_empty() {
        path.set_extension(extension);
    }
    path
}

fn retry_transient_executable_busy<T>(
    mut operation: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    const MAX_ATTEMPTS: usize = 20;
    for attempt in 0..MAX_ATTEMPTS {
        match operation() {
            Ok(value) => return Ok(value),
            Err(error)
                if is_transient_executable_launch_error(&error) && attempt + 1 < MAX_ATTEMPTS =>
            {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("retry loop returns on its final attempt")
}

fn is_transient_executable_launch_error(error: &io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(26)
            || (cfg!(target_os = "macos") && error.raw_os_error() == Some(88))
    }
    #[cfg(not(unix))]
    {
        let _ = error;
        false
    }
}

fn host_linker_is_available() -> bool {
    let linker = std::env::var("CC").unwrap_or_else(|_| {
        if cfg!(windows) {
            "cl.exe".to_string()
        } else {
            "cc".to_string()
        }
    });
    Command::new(linker).arg("--version").output().is_ok()
}

fn php_is_available() -> bool {
    Command::new("php").arg("--version").output().is_ok()
}
