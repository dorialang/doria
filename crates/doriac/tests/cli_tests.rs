use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn version_uses_canonical_toolchain_calver() {
    let output = Command::new(doriac_bin())
        .arg("--version")
        .output()
        .expect("doriac binary should run");

    assert_success("version", output.clone());
    assert_eq!(
        String::from_utf8(output.stdout).expect("version output should be UTF-8"),
        "doriac 2026.03.1-canary\n"
    );
}

#[test]
fn check_json_exposes_static_identity_fix_ranges() {
    let temp_dir = temp_dir_path("check-json-static-fix");
    fs::create_dir_all(&temp_dir).expect("temp directory should be created");
    let source = "class Foo { static int $prop = 1; function read(): int { return Foo::$prop; } }";
    fs::write(temp_dir.join("main.doria"), source).expect("source should be writable");

    let output = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .arg("check")
        .arg("main.doria")
        .arg("--json")
        .output()
        .expect("doriac binary should run");
    assert!(!output.status.success());
    let diagnostics: serde_json::Value =
        serde_json::from_slice(&output.stderr).expect("check --json stderr should be valid JSON");
    let diagnostic = diagnostics
        .as_array()
        .and_then(|diagnostics| diagnostics.iter().find(|item| item["code"] == "E0494"))
        .expect("E0494 JSON diagnostic");
    let dollar = source.rfind("$prop").expect("access sigil");
    assert_eq!(diagnostic["fix"]["span"]["start"], dollar);
    assert_eq!(diagnostic["fix"]["span"]["end"], dollar + 1);
    assert_eq!(diagnostic["fix"]["replacement"], "");

    let late_static_source = "class Foo { static function create(): int { return 1; } function read(): int { return static::create(); } }";
    fs::write(temp_dir.join("main.doria"), late_static_source)
        .expect("late-static source should be writable");
    let late_static_output = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .arg("check")
        .arg("main.doria")
        .arg("--json")
        .output()
        .expect("doriac binary should run");
    assert!(!late_static_output.status.success());
    let diagnostics: serde_json::Value = serde_json::from_slice(&late_static_output.stderr)
        .expect("late-static check --json stderr should be valid JSON");
    let diagnostic = diagnostics
        .as_array()
        .and_then(|diagnostics| diagnostics.iter().find(|item| item["code"] == "E0495"))
        .expect("E0495 JSON diagnostic");
    let qualifier = late_static_source
        .rfind("static::")
        .expect("late-static qualifier");
    assert_eq!(diagnostic["fix"]["span"]["start"], qualifier);
    assert_eq!(diagnostic["fix"]["span"]["end"], qualifier + 6);
    assert_eq!(diagnostic["fix"]["replacement"], "self");

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn compile_defaults_to_native_executable() {
    if !host_linker_is_available() {
        eprintln!(
            "native CLI default test unavailable: host linker `{}` was not found",
            host_linker()
        );
        return;
    }

    let temp_dir = temp_dir_path("native-default");
    fs::create_dir_all(&temp_dir).expect("temp directory should be created");
    fs::write(
        temp_dir.join("main.doria"),
        r#"
function main(): int
{
    return 42;
}
"#,
    )
    .expect("source file should be writable");

    let compile = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .arg("compile")
        .arg("main.doria")
        .output()
        .expect("doriac binary should run");

    assert_success("native default compile", compile);

    let output_path = temp_dir.join(native_output_name("main"));
    assert!(output_path.exists(), "native executable should exist");

    let run = Command::new(&output_path)
        .status()
        .expect("native executable should run");
    assert_eq!(run.code(), Some(42));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn compile_php_target_defaults_to_php_output() {
    let temp_dir = temp_dir_path("php-default");
    fs::create_dir_all(&temp_dir).expect("temp directory should be created");
    fs::write(
        temp_dir.join("main.doria"),
        r#"
echo "Hello from Doria\n";
"#,
    )
    .expect("source file should be writable");

    let compile = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .arg("compile")
        .arg("main.doria")
        .arg("--target")
        .arg("php")
        .output()
        .expect("doriac binary should run");

    assert_success("php default compile", compile);

    let output_path = temp_dir.join("main.php");
    assert!(output_path.exists(), "PHP output should exist");

    let php = fs::read_to_string(&output_path).expect("PHP output should be readable");
    assert!(php.starts_with("<?php"));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn run_compiles_source_to_native_and_returns_program_status() {
    if !host_linker_is_available() {
        eprintln!(
            "native CLI run test unavailable: host linker `{}` was not found",
            host_linker()
        );
        return;
    }

    let temp_dir = temp_dir_path("native-run");
    fs::create_dir_all(&temp_dir).expect("temp directory should be created");
    fs::write(
        temp_dir.join("main.doria"),
        r#"
function main(): int
{
    return 42;
}
"#,
    )
    .expect("source file should be writable");

    let run = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .arg("run")
        .arg("main.doria")
        .output()
        .expect("doriac binary should run");

    assert_eq!(
        run.status.code(),
        Some(42),
        "doriac run should return the native program status"
    );

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn run_rejects_binary_input_with_source_hint() {
    let temp_dir = temp_dir_path("run-binary-input");
    fs::create_dir_all(&temp_dir).expect("temp directory should be created");
    fs::write(temp_dir.join("main"), [0, 159, 146, 150])
        .expect("binary-like file should be writable");

    let run = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .arg("run")
        .arg("main")
        .output()
        .expect("doriac binary should run");

    assert_failure_contains("binary run input", run, "expects a `.doria` source file");

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn release_rejects_non_native_targets() {
    for target in ["php", "debug", "wasm"] {
        let output = Command::new(doriac_bin())
            .arg("compile")
            .arg("missing.doria")
            .arg("--target")
            .arg(target)
            .arg("--release")
            .output()
            .expect("doriac binary should run");
        assert_failure_contains(
            &format!("release {target} target"),
            output,
            "--release is only valid for the native target",
        );
    }
}

#[cfg(not(feature = "llvm-backend"))]
#[test]
fn release_never_falls_back_when_llvm_support_is_disabled() {
    let temp_dir = temp_dir_path("release-disabled");
    fs::create_dir_all(&temp_dir).expect("temp directory should be created");
    fs::write(
        temp_dir.join("main.doria"),
        "function main(): int { return 42; }",
    )
    .expect("source file should be writable");

    for command in ["compile", "run"] {
        let output = Command::new(doriac_bin())
            .current_dir(&temp_dir)
            .arg(command)
            .arg("main.doria")
            .arg("--release")
            .output()
            .expect("doriac binary should run");
        assert_failure_contains(
            &format!("{command} without LLVM support"),
            output,
            "LLVM release support is not available in this doriac build",
        );
    }
    let _ = fs::remove_dir_all(temp_dir);
}

#[cfg(feature = "llvm-backend")]
#[test]
fn release_compile_and_run_use_the_enabled_llvm_profile() {
    if !host_linker_is_available() {
        return;
    }
    let temp_dir = temp_dir_path("release-enabled");
    fs::create_dir_all(&temp_dir).expect("temp directory should be created");
    fs::write(
        temp_dir.join("main.doria"),
        "function main(): int { return 42; }",
    )
    .expect("source file should be writable");

    let compile = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .arg("compile")
        .arg("main.doria")
        .arg("--release")
        .arg("--out")
        .arg(native_output_name("release-main"))
        .output()
        .expect("doriac binary should run");
    assert_success("LLVM release compile", compile);

    let run = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .arg("run")
        .arg("main.doria")
        .arg("--release")
        .output()
        .expect("doriac binary should run");
    assert_eq!(run.status.code(), Some(42));
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn compile_rejects_inferred_native_output_that_would_overwrite_input() {
    let temp_dir = temp_dir_path("native-overwrite-guard");
    fs::create_dir_all(&temp_dir).expect("temp directory should be created");

    let source = r#"
function main(): int
{
    return 0;
}
"#;
    let input_name = native_output_name("main");
    fs::write(temp_dir.join(&input_name), source).expect("source file should be writable");

    let compile = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .arg("compile")
        .arg(&input_name)
        .output()
        .expect("doriac binary should run");

    assert_failure_contains(
        "native inferred output overwrite guard",
        compile,
        "would overwrite input",
    );

    let preserved =
        fs::read_to_string(temp_dir.join(&input_name)).expect("source file should remain readable");
    assert_eq!(preserved, source);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn compile_rejects_inferred_php_output_that_would_overwrite_input() {
    let temp_dir = temp_dir_path("php-overwrite-guard");
    fs::create_dir_all(&temp_dir).expect("temp directory should be created");

    let source = r#"
echo "Hello from Doria\n";
"#;
    fs::write(temp_dir.join("main.php"), source).expect("source file should be writable");

    let compile = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .arg("compile")
        .arg("main.php")
        .arg("--target")
        .arg("php")
        .output()
        .expect("doriac binary should run");

    assert_failure_contains(
        "php inferred output overwrite guard",
        compile,
        "would overwrite input",
    );

    let preserved =
        fs::read_to_string(temp_dir.join("main.php")).expect("source file should remain readable");
    assert_eq!(preserved, source);

    let _ = fs::remove_dir_all(temp_dir);
}

fn doriac_bin() -> &'static str {
    env!("CARGO_BIN_EXE_doriac")
}

fn assert_success(label: &str, output: Output) {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        panic!(
            "{label} failed\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
            output.status, stdout, stderr
        );
    }
}

fn assert_failure_contains(label: &str, output: Output, expected: &str) {
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        panic!("{label} unexpectedly succeeded\nstdout:\n{stdout}");
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(expected),
        "{label}: expected stderr containing `{expected}`, got `{stderr}`"
    );
}

fn temp_dir_path(stem: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();

    std::env::temp_dir().join(format!("doriac-cli-{stem}-{}-{nanos}", std::process::id()))
}

fn native_output_name(stem: &str) -> String {
    if cfg!(windows) {
        format!("{stem}.exe")
    } else {
        stem.to_string()
    }
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
