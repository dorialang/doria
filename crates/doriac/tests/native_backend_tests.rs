use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use doriac::backend::BackendTarget;

#[test]
fn compiles_and_runs_stage_2b_native_smoke_examples() {
    if !host_linker_is_available() {
        eprintln!(
            "native smoke test unavailable: host linker `{}` was not found",
            host_linker()
        );
        return;
    }

    let workspace = workspace_root();
    let cases = [
        (
            "main_return_zero",
            "examples/native/main_return_zero.doria",
            0,
        ),
        ("main_return_42", "examples/native/main_return_42.doria", 42),
        ("main_return_125", "inline_main_return_125.doria", 125),
        (
            "main_readonly_local",
            "examples/native/main_readonly_local.doria",
            42,
        ),
        (
            "main_typed_readonly_local",
            "inline_main_typed_readonly_local.doria",
            42,
        ),
        (
            "main_unused_large_local",
            "inline_main_unused_large_local.doria",
            0,
        ),
    ];

    for (stem, source, expected_code) in cases {
        let output = temp_executable_path(stem);

        if source.ends_with(".doria") && source.starts_with("examples/") {
            compile_native_file(&workspace.join(source), &output);
        } else {
            compile_native_source(native_smoke_source(stem), &output);
        }

        let run = Command::new(&output)
            .status()
            .expect("native executable should run");
        assert_eq!(run.code(), Some(expected_code), "{stem}");

        let _ = fs::remove_file(output);
    }
}

fn native_smoke_source(stem: &str) -> &'static str {
    match stem {
        "main_return_125" => {
            r#"
function main(): int
{
    return 125;
}
"#
        }
        "main_typed_readonly_local" => {
            r#"
function main(): int
{
    int $code = 42;
    return $code;
}
"#
        }
        "main_unused_large_local" => {
            r#"
function main(): int
{
    let $value = 9223372036854775807;
    return 0;
}
"#
        }
        _ => unreachable!("unexpected inline native smoke source `{stem}`"),
    }
}

#[test]
fn rejects_unsupported_stage_2b_native_shapes() {
    let cases = [
        ("no main", "", "B0001", "no native entrypoint found"),
        (
            "main returns void",
            r#"
function main(): void
{
    return;
}
"#,
            "B0001",
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
            "B0001",
            "must not declare parameters",
        ),
        (
            "return nonzero literal",
            r#"
function main(): int
{
    return 126;
}
"#,
            "B0001",
            "native Stage 2b exit code must be in the range 0..125",
        ),
        (
            "return 255",
            r#"
function main(): int
{
    return 255;
}
"#,
            "B0001",
            "native Stage 2b exit code must be in the range 0..125",
        ),
        (
            "return out of Doria int range",
            r#"
function main(): int
{
    return 9223372036854775808;
}
"#,
            "E0417",
            "integer literal is outside the Doria `int` range",
        ),
        (
            "return string",
            r#"
function main(): int
{
    return "0";
}
"#,
            "E0404",
            "cannot return value of type `string`",
        ),
        (
            "return bool",
            r#"
function main(): int
{
    return true;
}
"#,
            "E0404",
            "cannot return value of type `bool`",
        ),
        (
            "return undeclared variable",
            r#"
function main(): int
{
    return $code;
}
"#,
            "E0101",
            "undeclared variable `$code`",
        ),
        (
            "returned local outside exit-code range",
            r#"
function main(): int
{
    let $code = 126;
    return $code;
}
"#,
            "B0001",
            "native Stage 2b exit code must be in the range 0..125",
        ),
        (
            "return binary expression",
            r#"
function main(): int
{
    return 20 + 22;
}
"#,
            "B0001",
            "expected integer literal or readonly integer local",
        ),
        (
            "writable local",
            r#"
function main(): int
{
    let writable $code = 42;
    return $code;
}
"#,
            "B0001",
            "unsupported native local for Stage 2b",
        ),
        (
            "non-int local",
            r#"
function main(): int
{
    string $message = "hello";
    return 0;
}
"#,
            "B0001",
            "unsupported native local for Stage 2b",
        ),
        (
            "local initialized from binary expression",
            r#"
function main(): int
{
    let $code = 20 + 22;
    return $code;
}
"#,
            "B0001",
            "unsupported native local for Stage 2b",
        ),
        (
            "local initialized from another local",
            r#"
function main(): int
{
    let $first = 42;
    let $second = $first;
    return $second;
}
"#,
            "B0001",
            "unsupported native local for Stage 2b",
        ),
        (
            "local outside Doria int range",
            r#"
function main(): int
{
    let $value = 9223372036854775808;
    return 0;
}
"#,
            "E0417",
            "integer literal is outside the Doria `int` range",
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
            "B0001",
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
            "B0001",
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
            "B0001",
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
            "B0001",
            "extra top-level function `helper`",
        ),
    ];

    for (name, source, expected_code, expected_message) in cases {
        let diagnostics =
            doriac::compile_source(format!("{name}.doria"), source, BackendTarget::Native)
                .expect_err("unsupported native Stage 2b source should fail");

        assert_eq!(diagnostics[0].code, expected_code, "{name}");
        assert!(
            diagnostics[0].message.contains(expected_message),
            "{name}: expected message containing `{expected_message}`, got `{}`",
            diagnostics[0].message
        );
    }
}

#[test]
fn native_backend_returns_executable_output_for_stage_2b_literal_shape() {
    if !host_linker_is_available() {
        eprintln!(
            "native executable output test unavailable: host linker `{}` was not found",
            host_linker()
        );
        return;
    }

    let output = doriac::compile_source(
        "test.doria",
        r#"
function main(): int
{
    return 42;
}
"#,
        BackendTarget::Native,
    )
    .expect("Stage 2b source should compile");

    match output {
        doriac::backend::BackendOutput::Executable { bytes, .. } => {
            assert!(!bytes.is_empty());
        }
        other => panic!("native backend should return executable output, got {other:?}"),
    }
}

#[test]
fn native_backend_returns_executable_output_for_stage_2b_local_shape() {
    if !host_linker_is_available() {
        eprintln!(
            "native executable output test unavailable: host linker `{}` was not found",
            host_linker()
        );
        return;
    }

    let output = doriac::compile_source(
        "test.doria",
        r#"
function main(): int
{
    let $code = 42;
    return $code;
}
"#,
        BackendTarget::Native,
    )
    .expect("Stage 2b local source should compile");

    match output {
        doriac::backend::BackendOutput::Executable { bytes, .. } => {
            assert!(!bytes.is_empty());
        }
        other => panic!("native backend should return executable output, got {other:?}"),
    }
}

fn compile_native_file(input: &Path, output: &Path) {
    let doriac = env!("CARGO_BIN_EXE_doriac");
    let compile = Command::new(doriac)
        .arg("compile")
        .arg(input)
        .arg("--target")
        .arg("native")
        .arg("--out")
        .arg(output)
        .output()
        .expect("doriac binary should run");

    assert_native_compile_succeeded(compile);
    assert!(output.exists(), "native executable should exist");
}

fn compile_native_source(source: &str, output: &Path) {
    let native = doriac::compile_source("test.doria", source, BackendTarget::Native)
        .expect("Stage 2b source should compile");
    let doriac::backend::BackendOutput::Executable { bytes, .. } = native else {
        panic!("native backend should return executable output, got {native:?}");
    };
    fs::write(output, bytes).expect("native executable bytes should be writable");
    make_executable(output);
}

fn assert_native_compile_succeeded(compile: std::process::Output) {
    if !compile.status.success() {
        let stderr = String::from_utf8_lossy(&compile.stderr);
        let stdout = String::from_utf8_lossy(&compile.stdout);
        panic!(
            "native smoke compile failed\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
            compile.status, stdout, stderr
        );
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

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .expect("native executable metadata should be readable")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("native executable should be made executable");
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}
