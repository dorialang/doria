use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use doriac::backend::BackendTarget;

#[test]
fn compiles_and_runs_stage_2d_native_smoke_examples() {
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
            "main_return_arithmetic_literal",
            "inline_main_return_arithmetic_literal.doria",
            42,
        ),
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
        (
            "main_arithmetic_42",
            "examples/native/main_arithmetic_42.doria",
            42,
        ),
        (
            "main_return_arithmetic_42",
            "examples/native/main_return_arithmetic_42.doria",
            42,
        ),
        (
            "main_return_arithmetic_locals",
            "inline_main_return_arithmetic_locals.doria",
            42,
        ),
        (
            "main_return_product_arithmetic",
            "inline_main_return_product_arithmetic.doria",
            42,
        ),
        (
            "main_return_grouped_arithmetic",
            "inline_main_return_grouped_arithmetic.doria",
            42,
        ),
        (
            "main_stage_2c_arithmetic_local",
            "inline_main_stage_2c_arithmetic_local.doria",
            42,
        ),
        (
            "main_local_to_local",
            "inline_main_local_to_local.doria",
            42,
        ),
        (
            "main_negative_unused_local",
            "inline_main_negative_unused_local.doria",
            0,
        ),
        (
            "main_unused_arithmetic_126",
            "inline_main_unused_arithmetic_126.doria",
            0,
        ),
        (
            "main_if_else_42",
            "examples/native/main_if_else_42.doria",
            42,
        ),
        ("main_if_42", "examples/native/main_if_42.doria", 42),
        ("main_if_true_42", "inline_main_if_true_42.doria", 42),
        ("main_if_false_42", "inline_main_if_false_42.doria", 42),
        (
            "main_guard_if_false_fallback_42",
            "inline_main_guard_if_false_fallback_42.doria",
            42,
        ),
        (
            "main_if_less_than_local",
            "inline_main_if_less_than_local.doria",
            42,
        ),
        (
            "main_if_large_local",
            "inline_main_if_large_local.doria",
            42,
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
        "main_return_arithmetic_literal" => {
            r#"
function main(): int
{
    return 20 + 22;
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
        "main_local_to_local" => {
            r#"
function main(): int
{
    let $first = 42;
    let $second = $first;
    return $second;
}
"#
        }
        "main_negative_unused_local" => {
            r#"
function main(): int
{
    let $negative = 1 - 2;
    return 0;
}
"#
        }
        "main_return_arithmetic_locals" => {
            r#"
function main(): int
{
    let $left = 20;
    let $right = 22;
    return $left + $right;
}
"#
        }
        "main_return_product_arithmetic" => {
            r#"
function main(): int
{
    let $base = 6;
    let $scale = 7;
    return $base * $scale;
}
"#
        }
        "main_return_grouped_arithmetic" => {
            r#"
function main(): int
{
    let $left = 20;
    let $right = 22;
    return ($left + $right) * 1;
}
"#
        }
        "main_stage_2c_arithmetic_local" => {
            r#"
function main(): int
{
    let $base = 6;
    let $scale = 7;
    let $code = $base * $scale;
    return $code;
}
"#
        }
        "main_unused_arithmetic_126" => {
            r#"
function main(): int
{
    let $value = 100 + 26;
    return 0;
}
"#
        }
        "main_if_true_42" => {
            r#"
function main(): int
{
    if (true) {
        return 42;
    } else {
        return 0;
    }
}
"#
        }
        "main_if_false_42" => {
            r#"
function main(): int
{
    if (false) {
        return 0;
    } else {
        return 42;
    }
}
"#
        }
        "main_guard_if_false_fallback_42" => {
            r#"
function main(): int
{
    if (false) {
        return 0;
    }

    return 42;
}
"#
        }
        "main_if_less_than_local" => {
            r#"
function main(): int
{
    let $x = 10;

    if ($x < 20) {
        return $x + 32;
    } else {
        return 0;
    }
}
"#
        }
        "main_if_large_local" => {
            r#"
function main(): int
{
    let $value = 126;

    if ($value > 100) {
        return 42;
    } else {
        return 0;
    }
}
"#
        }
        _ => unreachable!("unexpected inline native smoke source `{stem}`"),
    }
}

#[test]
fn rejects_unsupported_stage_2d_native_shapes() {
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
            "native Stage 4a exit code must be in the range 0..125",
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
            "native Stage 4a exit code must be in the range 0..125",
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
            "native Stage 4a exit code must be in the range 0..125",
        ),
        (
            "return arithmetic outside exit-code range",
            r#"
function main(): int
{
    return 100 + 26;
}
"#,
            "B0001",
            "native Stage 4a exit code must be in the range 0..125",
        ),
        (
            "returned arithmetic local outside exit-code range",
            r#"
function main(): int
{
    let $value = 100 + 26;
    return $value;
}
"#,
            "B0001",
            "native Stage 4a exit code must be in the range 0..125",
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
            "unsupported native local for Stage 2d",
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
            "unsupported native local for Stage 2d",
        ),
        (
            "return division",
            r#"
function main(): int
{
    return 42 / 1;
}
"#,
            "B0001",
            "unsupported native arithmetic operator for Stage 2d",
        ),
        (
            "return modulo",
            r#"
function main(): int
{
    return 42 % 5;
}
"#,
            "B0001",
            "unsupported native arithmetic operator for Stage 2d",
        ),
        (
            "local initialized from division",
            r#"
function main(): int
{
    let $code = 84 / 2;
    return $code;
}
"#,
            "B0001",
            "unsupported native arithmetic operator for Stage 2d",
        ),
        (
            "local initialized from function call",
            r#"
function main(): int
{
    let $code = calculate();
    return $code;
}
"#,
            "E0309",
            "unknown function `calculate`",
        ),
        (
            "return function call",
            r#"
function main(): int
{
    return calculate();
}
"#,
            "E0309",
            "unknown function `calculate`",
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
            "return arithmetic overflow",
            r#"
function main(): int
{
    return 9223372036854775807 + 1;
}
"#,
            "E0418",
            "integer arithmetic overflows the Doria `int` range",
        ),
        (
            "return multiplication overflow",
            r#"
function main(): int
{
    return 9223372036854775807 * 2;
}
"#,
            "E0418",
            "integer arithmetic overflows the Doria `int` range",
        ),
        (
            "constant arithmetic overflow",
            r#"
function main(): int
{
    let $value = 9223372036854775807 + 1;
    return 0;
}
"#,
            "E0418",
            "integer arithmetic overflows the Doria `int` range",
        ),
        (
            "constant multiplication overflow",
            r#"
function main(): int
{
    let $value = 9223372036854775807 * 2;
    return 0;
}
"#,
            "E0418",
            "integer arithmetic overflows the Doria `int` range",
        ),
        (
            "returned negative local outside exit-code range",
            r#"
function main(): int
{
    let $code = 1 - 2;
    return $code;
}
"#,
            "B0001",
            "native Stage 4a exit code must be in the range 0..125",
        ),
        (
            "if else branch outside exit-code range",
            r#"
function main(): int
{
    if (true) {
        return 0;
    } else {
        return 126;
    }
}
"#,
            "B0001",
            "native Stage 4a exit code must be in the range 0..125",
        ),
        (
            "guard if branch outside exit-code range",
            r#"
function main(): int
{
    if (true) {
        return 126;
    }

    return 0;
}
"#,
            "B0001",
            "native Stage 4a exit code must be in the range 0..125",
        ),
        (
            "if integer condition",
            r#"
function main(): int
{
    if (1) {
        return 42;
    } else {
        return 0;
    }
}
"#,
            "E0416",
            "condition must be `bool`",
        ),
        (
            "if arithmetic integer condition",
            r#"
function main(): int
{
    if (20 + 22) {
        return 42;
    } else {
        return 0;
    }
}
"#,
            "E0416",
            "condition must be `bool`",
        ),
        (
            "if condition division",
            r#"
function main(): int
{
    if (42 / 1 == 42) {
        return 42;
    } else {
        return 0;
    }
}
"#,
            "B0001",
            "unsupported native arithmetic operator for Stage 2d",
        ),
        (
            "if branch local declaration",
            r#"
function main(): int
{
    if (true) {
        let $code = 42;
        return $code;
    } else {
        return 0;
    }
}
"#,
            "B0001",
            "unsupported native branch for Stage 4a",
        ),
        (
            "if without fallback return",
            r#"
function main(): int
{
    if (true) {
        return 42;
    }
}
"#,
            "E0406",
            "must return a value",
        ),
        (
            "else if",
            r#"
function main(): int
{
    if (true) {
        return 42;
    } else if (false) {
        return 1;
    } else {
        return 0;
    }
}
"#,
            "B0001",
            "else-if is not supported",
        ),
        (
            "statement after terminal if else",
            r#"
function main(): int
{
    if (true) {
        return 42;
    } else {
        return 0;
    }

    return 1;
}
"#,
            "B0001",
            "unsupported native statement for Stage 4a",
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
                .expect_err("unsupported native Stage 2d source should fail");

        assert_eq!(diagnostics[0].code, expected_code, "{name}");
        assert!(
            diagnostics[0].message.contains(expected_message),
            "{name}: expected message containing `{expected_message}`, got `{}`",
            diagnostics[0].message
        );
    }
}

#[test]
fn native_backend_returns_executable_output_for_stage_2d_literal_shape() {
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
    .expect("Stage 2d source should compile");

    match output {
        doriac::backend::BackendOutput::Executable { bytes, .. } => {
            assert!(!bytes.is_empty());
        }
        other => panic!("native backend should return executable output, got {other:?}"),
    }
}

#[test]
fn native_backend_returns_executable_output_for_stage_2d_arithmetic_shape() {
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
    let $base = 20;
    let $code = $base * 2 + 2;
    return $code;
}
"#,
        BackendTarget::Native,
    )
    .expect("Stage 2d arithmetic source should compile");

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
        .expect("Stage 2d source should compile");
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
