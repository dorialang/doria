use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use doriac::backend::BackendTarget;

#[test]
fn compiles_and_runs_stage_1_native_smoke_example() {
    if !host_linker_is_available() {
        eprintln!(
            "native smoke test unavailable: host linker `{}` was not found",
            default_linker()
        );
        return;
    }

    let workspace = workspace_root();
    let input = workspace.join("examples/native/main_return_zero.doria");
    let output = temp_executable_path("main_return_zero");
    let doriac = env!("CARGO_BIN_EXE_doriac");

    let compile = Command::new(doriac)
        .arg("compile")
        .arg(&input)
        .arg("--target")
        .arg("native")
        .arg("--out")
        .arg(&output)
        .output()
        .expect("doriac binary should run");

    if !compile.status.success() {
        let stderr = String::from_utf8_lossy(&compile.stderr);
        let stdout = String::from_utf8_lossy(&compile.stdout);
        panic!(
            "native smoke compile failed\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
            compile.status, stdout, stderr
        );
    }

    assert!(output.exists(), "native executable should exist");

    let run = Command::new(&output)
        .status()
        .expect("native executable should run");
    assert_eq!(run.code(), Some(0));

    let _ = fs::remove_file(output);
}

#[test]
fn rejects_unsupported_stage_1_native_shapes() {
    let cases = [
        ("no main", "", "no native entrypoint found"),
        (
            "main returns void",
            r#"
function main(): void
{
    return;
}
"#,
            "wrong main signature",
        ),
        (
            "main has parameter",
            r#"
function main(int $code): int
{
    return 0;
}
"#,
            "must not declare parameters",
        ),
        (
            "return nonzero literal",
            r#"
function main(): int
{
    return 42;
}
"#,
            "unsupported native expression",
        ),
        (
            "local variable",
            r#"
function main(): int
{
    let $value = 0;
    return 0;
}
"#,
            "local variable declaration",
        ),
        (
            "echo",
            r#"
function main(): int
{
    echo 0;
    return 0;
}
"#,
            "echo statement",
        ),
        (
            "top-level statement",
            r#"
echo 0;

function main(): int
{
    return 0;
}
"#,
            "unsupported top-level item",
        ),
        (
            "class",
            r#"
class Person
{
}

function main(): int
{
    return 0;
}
"#,
            "class `Person`",
        ),
        (
            "extra top-level function",
            r#"
function helper(): int
{
    return 0;
}

function main(): int
{
    return 0;
}
"#,
            "extra top-level function `helper`",
        ),
    ];

    for (name, source, expected_message) in cases {
        let diagnostics =
            doriac::compile_source(format!("{name}.doria"), source, BackendTarget::Native)
                .expect_err("unsupported native Stage 1 source should fail");

        assert_eq!(diagnostics[0].code, "B0001", "{name}");
        assert!(
            diagnostics[0].message.contains(expected_message),
            "{name}: expected message containing `{expected_message}`, got `{}`",
            diagnostics[0].message
        );
    }
}

#[test]
fn native_backend_returns_executable_output_for_stage_1_shape() {
    if !host_linker_is_available() {
        eprintln!(
            "native executable output test unavailable: host linker `{}` was not found",
            default_linker()
        );
        return;
    }

    let output = doriac::compile_source(
        "test.doria",
        r#"
function main(): int
{
    return 0;
}
"#,
        BackendTarget::Native,
    )
    .expect("Stage 1 source should compile");

    match output {
        doriac::backend::BackendOutput::Executable { bytes, .. } => {
            assert!(!bytes.is_empty());
        }
        other => panic!("native backend should return executable output, got {other:?}"),
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate should live under crates/doriac")
        .to_path_buf()
}

fn temp_executable_path(stem: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let extension = if cfg!(windows) { ".exe" } else { "" };
    std::env::temp_dir().join(format!(
        "doriac-{stem}-{}-{nanos}{extension}",
        std::process::id()
    ))
}

fn host_linker_is_available() -> bool {
    let mut command = Command::new(default_linker());
    if cfg!(windows) {
        command.arg("/?");
    } else {
        command.arg("--version");
    }
    command.output().is_ok()
}

fn default_linker() -> &'static str {
    if cfg!(windows) {
        "link.exe"
    } else {
        "cc"
    }
}
