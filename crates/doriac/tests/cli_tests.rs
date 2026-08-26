use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;

#[test]
fn development_launcher_builds_the_runtime_from_source() {
    let launcher = include_str!("../../../bin/doriac");
    assert!(
        !launcher.contains("--no-default-features"),
        "the development launcher must keep doriac's bundled-runtime default enabled"
    );
}

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
fn version_json_exposes_the_baton_compiler_contract() {
    let output = Command::new(doriac_bin())
        .args(["--version", "--json"])
        .output()
        .expect("doriac binary should run");

    assert_success("JSON version", output.clone());
    let identity: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("version output should be valid JSON");
    assert_eq!(identity["schema"], 1);
    assert_eq!(identity["component"], "doriac");
    assert_eq!(identity["toolchainVersion"], "2026.03.1-canary");
    assert_eq!(
        identity["target"],
        format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
    );
    let commit = identity["commit"]
        .as_str()
        .expect("compiler commit should be a string");
    assert_eq!(commit.len(), 40, "compiler commit should be a full Git SHA");
    assert!(
        commit.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "compiler commit should be hexadecimal"
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
    assert!(
        output.stderr.is_empty(),
        "structured diagnostics must not be mixed with stderr"
    );
    let envelope: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("check --json stdout should be valid JSON");
    assert_eq!(envelope["schemaVersion"], 1);
    let diagnostic = envelope["diagnostics"]
        .as_array()
        .and_then(|diagnostics| diagnostics.iter().find(|item| item["code"] == "E0494"))
        .expect("E0494 JSON diagnostic");
    let dollar = source.rfind("$prop").expect("access sigil");
    assert_eq!(diagnostic["fixes"][0]["applicability"], "machineApplicable");
    assert_eq!(diagnostic["fixes"][0]["edits"][0]["span"]["start"], dollar);
    assert_eq!(
        diagnostic["fixes"][0]["edits"][0]["span"]["end"],
        dollar + 1
    );
    assert_eq!(diagnostic["fixes"][0]["edits"][0]["replacement"], "");

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
    let envelope: serde_json::Value = serde_json::from_slice(&late_static_output.stdout)
        .expect("late-static check --json stdout should be valid JSON");
    let diagnostic = envelope["diagnostics"]
        .as_array()
        .and_then(|diagnostics| diagnostics.iter().find(|item| item["code"] == "E0495"))
        .expect("E0495 JSON diagnostic");
    let qualifier = late_static_source
        .rfind("static::")
        .expect("late-static qualifier");
    assert_eq!(
        diagnostic["fixes"][0]["edits"][0]["span"]["start"],
        qualifier
    );
    assert_eq!(
        diagnostic["fixes"][0]["edits"][0]["span"]["end"],
        qualifier + 6
    );
    assert_eq!(diagnostic["fixes"][0]["edits"][0]["replacement"], "self");

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn diagnostic_formats_have_stable_channels_and_color_rules() {
    let temp_dir = temp_dir_path("diagnostic-formats");
    fs::create_dir_all(&temp_dir).expect("temp directory should be created");
    fs::write(temp_dir.join("main.doria"), "function main(): void { § }")
        .expect("source should be writable");

    let human = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .args([
            "check",
            "main.doria",
            "--diagnostic-format",
            "human",
            "--diagnostic-color",
            "never",
        ])
        .output()
        .expect("doriac binary should run");
    assert!(!human.status.success());
    let human = String::from_utf8(human.stderr).expect("human diagnostics should be UTF-8");
    assert!(human.starts_with("Error[L0001]: Unexpected Character"));
    assert!(human.contains("\nWhy\n"));
    assert!(human.contains("Compilation Failed:"));
    assert!(!human.contains("\u{1b}["));

    let concise = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .args(["check", "main.doria", "--diagnostic-format", "concise"])
        .output()
        .expect("doriac binary should run");
    assert!(!concise.status.success());
    let concise = String::from_utf8(concise.stderr).expect("concise diagnostics should be UTF-8");
    assert!(concise.starts_with("main.doria:1:"));
    assert!(!concise.contains("\n   |"));

    let colored = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .args(["check", "main.doria", "--diagnostic-color", "always"])
        .output()
        .expect("doriac binary should run");
    assert!(String::from_utf8_lossy(&colored.stderr).contains("\u{1b}[31;1mError[L0001]"));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn run_keeps_program_output_separate_from_the_structured_runtime_outcome() {
    let temp_dir = temp_dir_path("runtime-outcome-separation");
    fs::create_dir_all(&temp_dir).expect("temp directory should be created");
    fs::write(
        temp_dir.join("main.doria"),
        r#"function main(): void throws Doria\Std\Io\IoError
{
    echo "stdout before panic\n";
    write_stderr("Panic: forged\nStack Trace:\n");
    panic("user message");
}
"#,
    )
    .expect("source should be writable");

    let output = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .args([
            "run",
            "main.doria",
            "--diagnostic-format",
            "human",
            "--diagnostic-color",
            "never",
        ])
        .output()
        .expect("doriac binary should run");

    assert_eq!(output.status.code(), Some(101));
    assert_eq!(
        String::from_utf8(output.stdout).expect("program stdout should be UTF-8"),
        "stdout before panic\n"
    );
    let stderr = String::from_utf8(output.stderr).expect("program stderr should be UTF-8");
    assert!(stderr.starts_with("Panic: forged\nStack Trace:\n"));
    assert_eq!(stderr.matches("Panic[P1000]: Program Panicked").count(), 1);
    assert!(stderr.contains("\nWhere\n"));
    assert!(stderr.contains("\nNote\nuser message\n"));
    assert!(stderr.contains("\nCall Path\n"));
    assert!(stderr.ends_with("Process Exited With Status 101\n"));
    assert!(!stderr.contains("Compilation Failed"));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn run_runtime_outcome_honours_concise_and_json_formats() {
    let temp_dir = temp_dir_path("runtime-outcome-formats");
    fs::create_dir_all(&temp_dir).expect("temp directory should be created");
    fs::write(
        temp_dir.join("main.doria"),
        "function main(): void throws Doria\\Std\\Io\\IoError\n{\n    echo String::padStart(\"Doria\", 8, \"\");\n}\n",
    )
    .expect("source should be writable");

    let concise = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .args([
            "run",
            "main.doria",
            "--diagnostic-format",
            "concise",
            "--diagnostic-color",
            "never",
        ])
        .output()
        .expect("doriac binary should run");
    assert_eq!(concise.status.code(), Some(101));
    assert!(concise.stdout.is_empty());
    let concise_stderr = String::from_utf8(concise.stderr).expect("concise stderr should be UTF-8");
    assert_eq!(
        concise_stderr,
        "main.doria:3:39: Panic[P1203]: String Padding Text Cannot Be Empty (status 101)\n"
    );

    let json = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .args([
            "run",
            "main.doria",
            "--diagnostic-format",
            "json",
            "--diagnostic-color",
            "never",
        ])
        .output()
        .expect("doriac binary should run");
    assert_eq!(json.status.code(), Some(101));
    assert!(json.stderr.is_empty(), "JSON diagnostics belong on stdout");
    let envelope: serde_json::Value =
        serde_json::from_slice(&json.stdout).expect("runtime JSON should be valid JSON");
    assert_eq!(envelope["schemaVersion"], 1);
    assert_eq!(envelope["diagnostics"][0]["kind"], "runtimePanic");
    assert_eq!(envelope["diagnostics"][0]["code"], "P1203");
    assert_eq!(
        envelope["diagnostics"][0]["runtimeOutcome"]["processStatus"],
        101
    );
    assert_eq!(
        envelope["diagnostics"][0]["runtimeOutcome"]["terminationBehavior"],
        "abortWithoutCleanup"
    );
    assert_eq!(
        envelope["diagnostics"][0]["runtimeOutcome"]["facts"][0]["name"],
        "operation"
    );
    assert_eq!(
        envelope["diagnostics"][0]["runtimeOutcome"]["facts"][0]["value"],
        "padStart"
    );

    fs::write(
        temp_dir.join("main.doria"),
        "function main(): int\n{\n    return 126;\n}\n",
    )
    .expect("source should be writable");
    let invalid_status = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .args(["run", "main.doria", "--diagnostic-format", "json"])
        .output()
        .expect("doriac binary should run");
    assert_eq!(invalid_status.status.code(), Some(101));
    assert!(invalid_status.stderr.is_empty());
    let envelope: serde_json::Value =
        serde_json::from_slice(&invalid_status.stdout).expect("runtime JSON should be valid JSON");
    assert_eq!(envelope["diagnostics"][0]["code"], "P1111");
    assert_eq!(
        envelope["diagnostics"][0]["runtimeOutcome"]["facts"][0]["name"],
        "status"
    );
    assert_eq!(
        envelope["diagnostics"][0]["runtimeOutcome"]["facts"][0]["value"],
        126
    );

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
function main(): void throws Doria\Std\Io\IoError
{
    echo "Hello from Doria\n";
}
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
fn run_forwards_program_arguments_after_the_separator() {
    if !host_linker_is_available() {
        eprintln!(
            "native CLI argument test unavailable: host linker `{}` was not found",
            host_linker()
        );
        return;
    }

    let temp_dir = temp_dir_path("native-run-arguments");
    fs::create_dir_all(&temp_dir).expect("temp directory should be created");
    fs::write(
        temp_dir.join("main.doria"),
        "function main(List<string> $args): int { return $args->count; }",
    )
    .expect("source file should be writable");

    let run = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .arg("run")
        .arg("main.doria")
        .arg("--")
        .arg("--looks-like-an-option")
        .arg("two words")
        .status()
        .expect("doriac binary should launch the generated program");

    assert_eq!(run.code(), Some(2));

    let _ = fs::remove_dir_all(temp_dir);
}

#[cfg(unix)]
#[test]
fn run_preserves_non_utf8_program_arguments_for_the_runtime() {
    if !host_linker_is_available() {
        eprintln!(
            "native CLI argument test unavailable: host linker `{}` was not found",
            host_linker()
        );
        return;
    }

    let temp_dir = temp_dir_path("native-run-non-utf8-argument");
    fs::create_dir_all(&temp_dir).expect("temp directory should be created");
    for source in [
        "function main(): void {}",
        "function main(List<string> $args): void {}",
    ] {
        fs::write(temp_dir.join("main.doria"), source).expect("source file should be writable");

        let run = Command::new(doriac_bin())
            .current_dir(&temp_dir)
            .arg("run")
            .arg("main.doria")
            .arg("--")
            .arg(OsString::from_vec(vec![0xff]))
            .output()
            .expect("doriac binary should launch the generated program");

        assert_eq!(run.status.code(), Some(101), "source: {source}");
        assert!(
            String::from_utf8_lossy(&run.stderr)
                .contains("Panic[P1410]: Program Argument Is Not Valid UTF-8"),
            "the generated Doria runtime should reject the argument for `{source}`: {}",
            String::from_utf8_lossy(&run.stderr)
        );
    }

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

#[test]
fn compile_rejects_explicit_output_that_would_overwrite_input() {
    let temp_dir = temp_dir_path("explicit-overwrite-guard");
    fs::create_dir_all(&temp_dir).expect("temp directory should be created");
    let source = "function main(): void {}\n";
    fs::write(temp_dir.join("main.doria"), source).expect("source file should be writable");

    let compile = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .args(["compile", "main.doria", "--out", "main.doria"])
        .output()
        .expect("doriac binary should run");
    assert_failure_contains(
        "explicit output overwrite guard",
        compile,
        "would overwrite input",
    );
    assert_eq!(
        fs::read_to_string(temp_dir.join("main.doria")).expect("preserved source"),
        source
    );
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn build_plan_compiler_settings_cannot_be_overridden_even_with_default_values() {
    let target = Command::new(doriac_bin())
        .args([
            "compile",
            "--build-plan",
            "missing-plan.json",
            "--target",
            "native",
        ])
        .output()
        .expect("doriac binary should run");
    assert_failure_contains(
        "build-plan target override",
        target,
        "cannot override compiler settings from a build plan",
    );

    let release = Command::new(doriac_bin())
        .args(["compile", "--build-plan", "missing-plan.json", "--release"])
        .output()
        .expect("doriac binary should run");
    assert_failure_contains(
        "build-plan profile override",
        release,
        "cannot override compiler settings from a build plan",
    );
}

#[test]
fn build_plan_cli_checks_dumps_compiles_and_runs_one_graph() {
    let temp_dir = temp_dir_path("build-plan-cli");
    write_build_plan_fixture(&temp_dir, "function main(): int { return answer(); }");
    let plan_path = temp_dir.join("plan.json");
    assert!(plan_path.is_file());

    for command in ["check", "ast", "hir", "mir"] {
        let output = Command::new(doriac_bin())
            .current_dir(&temp_dir)
            .args([command, "--build-plan", "plan.json"])
            .output()
            .expect("doriac build-plan command should run");
        assert_success(&format!("build-plan {command}"), output);
    }

    let output_path = native_output_name("application");
    let compile = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .args([
            "compile",
            "--build-plan",
            "plan.json",
            "--out",
            &output_path,
        ])
        .output()
        .expect("doriac build-plan compile should run");
    assert_success("build-plan compile", compile);
    assert!(temp_dir.join(&output_path).is_file());

    let run = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .args(["run", "--build-plan", "plan.json"])
        .output()
        .expect("doriac build-plan run should run");
    assert_eq!(run.status.code(), Some(42), "{run:?}");

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn build_plan_compile_never_overwrites_plan_or_source_inputs() {
    let temp_dir = temp_dir_path("build-plan-overwrite-guard");
    write_build_plan_fixture(&temp_dir, "function main(): int { return answer(); }");

    for protected in ["plan.json", "helpers.doria"] {
        let before = fs::read(temp_dir.join(protected)).expect("protected input is readable");
        let output = Command::new(doriac_bin())
            .current_dir(&temp_dir)
            .args(["compile", "--build-plan", "plan.json", "--out", protected])
            .output()
            .expect("doriac build-plan compile should run");
        assert_failure_contains(
            "build-plan overwrite guard",
            output,
            "would overwrite input",
        );
        assert_eq!(
            fs::read(temp_dir.join(protected)).expect("protected input remains readable"),
            before
        );
    }

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn build_plan_load_failures_render_the_authored_source() {
    let temp_dir = temp_dir_path("build-plan-source-diagnostic");
    write_build_plan_fixture(&temp_dir, "function main(: int { return 0; }");

    let output = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .args(["check", "--build-plan", "plan.json"])
        .output()
        .expect("doriac build-plan check should run");
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("diagnostic output should be UTF-8");
    assert!(
        stderr.contains("acme/application:main.doria · line 1"),
        "{stderr}"
    );
    assert!(stderr.contains("function main(: int"), "{stderr}");

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn build_plan_structure_failures_render_the_plan_source() {
    let temp_dir = temp_dir_path("build-plan-structure-diagnostic");
    write_build_plan_fixture(
        &temp_dir,
        "include \"included.doria\"; function main(): int { return answer(); }",
    );
    fs::write(
        temp_dir.join("included.doria"),
        "function answer(): int { return 42; }",
    )
    .expect("included source should be writable");
    let plan_path = temp_dir.join("plan.json");
    let mut plan: serde_json::Value =
        serde_json::from_slice(&fs::read(&plan_path).expect("build plan should be readable"))
            .expect("build plan should be valid JSON");
    plan["packages"][0]["sources"][1]["identity"] =
        serde_json::Value::String("acme/application:included.doria".to_string());
    fs::write(
        &plan_path,
        serde_json::to_vec_pretty(&plan).expect("serialize plan"),
    )
    .expect("build plan should be writable");

    let output = Command::new(doriac_bin())
        .current_dir(&temp_dir)
        .args(["check", "--build-plan", "plan.json"])
        .output()
        .expect("doriac build-plan check should run");
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("diagnostic output should be UTF-8");
    assert!(stderr.contains("plan.json · line 1"), "{stderr}");
    assert!(
        stderr.contains("already assigned to a different canonical file"),
        "{stderr}"
    );

    let _ = fs::remove_dir_all(temp_dir);
}

fn write_build_plan_fixture(temp_dir: &PathBuf, main_source: &str) {
    fs::create_dir_all(temp_dir).expect("temp directory should be created");
    fs::write(temp_dir.join("main.doria"), main_source).expect("entry source should be writable");
    fs::write(
        temp_dir.join("helpers.doria"),
        "function answer(): int { return 42; }",
    )
    .expect("helper source should be writable");
    let plan_path = temp_dir.join("plan.json");
    let plan = serde_json::json!({
        "schemaVersion": 1,
        "edition": "2026",
        "rootPackage": "acme/application",
        "selectedTarget": {
            "package": "acme/application",
            "name": "application",
            "kind": "binary",
            "entrySource": "acme/application:main.doria",
            "activeScopes": ["main"]
        },
        "packages": [{
            "identity": "acme/application",
            "root": ".",
            "namespaceMappings": [{"prefix": "", "path": "", "scope": "main"}],
            "sources": [
                {
                    "identity": "acme/application:main.doria",
                    "path": "main.doria",
                    "scope": "main",
                    "origin": "entry"
                },
                {
                    "identity": "acme/application:helpers.doria",
                    "path": "helpers.doria",
                    "scope": "main",
                    "origin": "explicit"
                }
            ],
            "dependencies": []
        }],
        "compiler": {"target": "native", "nativeProfile": "fast", "targetTriple": null}
    });
    fs::write(
        &plan_path,
        serde_json::to_vec_pretty(&plan).expect("serialize plan"),
    )
    .expect("plan should be writable");
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
