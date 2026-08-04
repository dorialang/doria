use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn opt_in_native_compile_writes_a_versioned_phase_report() {
    if !host_linker_is_available() {
        eprintln!("performance report test unavailable: host linker was not found");
        return;
    }
    let directory = fixture_directory("success");
    fs::create_dir_all(&directory).expect("fixture directory");
    fs::write(
        directory.join("main.doria"),
        "function main(): int { return 42; }\n",
    )
    .expect("source");
    let output = Command::new(doriac_bin())
        .current_dir(&directory)
        .args([
            "compile",
            "main.doria",
            "--out",
            executable_name(),
            "--performance-report",
            "performance.json",
            "--diagnostic-format",
            "json",
            "--diagnostic-color",
            "never",
        ])
        .output()
        .expect("doriac");
    assert!(
        output.status.success(),
        "compile failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(
        &fs::read(directory.join("performance.json")).expect("performance report"),
    )
    .expect("report JSON");
    assert_eq!(report["schemaVersion"], 1);
    assert_eq!(report["success"], true);
    assert_eq!(report["backend"], "cranelift");
    assert_eq!(report["source"]["bytes"], 36);
    assert_eq!(
        report["command"],
        serde_json::json!([
            doriac_bin(),
            "compile",
            "main.doria",
            "--out",
            executable_name(),
            "--performance-report",
            "performance.json",
            "--diagnostic-format",
            "json",
            "--diagnostic-color",
            "never",
        ])
    );
    assert!(report["totalDurationNs"]
        .as_u64()
        .is_some_and(|value| value > 0));
    assert_eq!(report["phases"]["parse"]["available"], true);
    assert_eq!(report["phases"]["semanticAnalysis"]["available"], true);
    assert_eq!(report["phases"]["borrowChecking"]["available"], false);
    assert_eq!(report["phases"]["mirLowering"]["available"], true);
    assert_eq!(report["phases"]["mirValidation"]["available"], true);
    assert_eq!(
        report["phases"]["craneliftCodeGeneration"]["available"],
        true
    );
    assert_eq!(report["phases"]["llvmCodeGeneration"]["available"], false);
    assert_eq!(report["phases"]["link"]["available"], true);
    assert!(report["metrics"]["outputBytes"]
        .as_u64()
        .is_some_and(|value| value > 0));
    assert!(report["metrics"]["functionCount"].as_u64().is_some());
    assert_eq!(report["metrics"]["sourceLineCount"], 1);
    assert_eq!(report["metrics"]["astItemCount"], 1);
    assert_eq!(
        report["metrics"]["mirFunctionCount"],
        report["metrics"]["functionCount"]
    );
    for field in [
        "mirBasicBlockCount",
        "mirStatementCount",
        "mirTerminatorCount",
    ] {
        assert!(report["metrics"][field].as_u64().is_some(), "{field}");
    }
    assert_eq!(
        report["metrics"]["mirBasicBlockCount"],
        report["metrics"]["mirTerminatorCount"]
    );
    assert!(report["metrics"]["runtimeArtifactBytes"]
        .as_u64()
        .is_some_and(|value| value > 0));
    assert!(report["artifacts"]["runtime"]["path"]
        .as_str()
        .is_some_and(|path| !path.is_empty()));
    assert_eq!(
        report["artifacts"]["runtime"]["bytes"],
        report["metrics"]["runtimeArtifactBytes"]
    );
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn report_path_cannot_alias_source_or_compiler_output() {
    let directory = fixture_directory("path-aliases");
    fs::create_dir_all(&directory).expect("fixture directory");
    let source = "function main(): void {}\n";
    fs::write(directory.join("main.doria"), source).expect("source");

    let source_alias = Command::new(doriac_bin())
        .current_dir(&directory)
        .args([
            "compile",
            "main.doria",
            "--out",
            executable_name(),
            "--performance-report",
            "main.doria",
        ])
        .output()
        .expect("doriac");
    assert!(!source_alias.status.success());
    assert!(String::from_utf8_lossy(&source_alias.stderr).contains("would overwrite input"));
    assert_eq!(
        fs::read_to_string(directory.join("main.doria")).unwrap(),
        source
    );
    assert!(!directory.join(executable_name()).exists());

    let output_alias = Command::new(doriac_bin())
        .current_dir(&directory)
        .args([
            "compile",
            "main.doria",
            "--out",
            executable_name(),
            "--performance-report",
            executable_name(),
        ])
        .output()
        .expect("doriac");
    assert!(!output_alias.status.success());
    assert!(
        String::from_utf8_lossy(&output_alias.stderr).contains("would overwrite compiler output")
    );
    assert_eq!(
        fs::read_to_string(directory.join("main.doria")).unwrap(),
        source
    );
    assert!(!directory.join(executable_name()).exists());

    let hard_link = directory.join("source-alias.json");
    if fs::hard_link(directory.join("main.doria"), &hard_link).is_ok() {
        let hard_link_alias = Command::new(doriac_bin())
            .current_dir(&directory)
            .args([
                "compile",
                "main.doria",
                "--out",
                executable_name(),
                "--performance-report",
                "source-alias.json",
            ])
            .output()
            .expect("doriac");
        assert!(!hard_link_alias.status.success());
        assert!(String::from_utf8_lossy(&hard_link_alias.stderr).contains("would overwrite input"));
        assert_eq!(
            fs::read_to_string(directory.join("main.doria")).unwrap(),
            source
        );
        assert!(!directory.join(executable_name()).exists());
    }
    let _ = fs::remove_dir_all(directory);
}

#[cfg(windows)]
#[test]
fn report_and_output_paths_are_compared_using_windows_identity_rules() {
    let directory = fixture_directory("case-insensitive-path-alias");
    fs::create_dir_all(&directory).expect("fixture directory");
    fs::write(directory.join("main.doria"), "function main(): void {}\n").expect("source");

    let output = Command::new(doriac_bin())
        .current_dir(&directory)
        .args([
            "compile",
            "main.doria",
            "--out",
            "program.exe",
            "--performance-report",
            "PROGRAM.EXE",
        ])
        .output()
        .expect("doriac");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("would overwrite compiler output"));
    assert!(!directory.join("program.exe").exists());
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn an_existing_report_is_replaced_atomically() {
    if !host_linker_is_available() {
        return;
    }
    let directory = fixture_directory("atomic-replacement");
    fs::create_dir_all(&directory).expect("fixture directory");
    fs::write(
        directory.join("main.doria"),
        "function main(): int { return 42; }\n",
    )
    .expect("source");
    fs::write(directory.join("performance.json"), "stale report\n").expect("stale report");
    let output = Command::new(doriac_bin())
        .current_dir(&directory)
        .args([
            "compile",
            "main.doria",
            "--out",
            executable_name(),
            "--performance-report",
            "performance.json",
        ])
        .output()
        .expect("doriac");
    assert!(
        output.status.success(),
        "compile failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("performance.json")).expect("report"))
            .expect("replacement report JSON");
    assert_eq!(report["schemaVersion"], 1);
    assert_eq!(report["success"], true);
    assert!(fs::read_dir(&directory)
        .expect("directory")
        .all(|entry| !entry
            .expect("entry")
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")));
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn ordinary_compile_does_not_create_performance_evidence() {
    if !host_linker_is_available() {
        return;
    }
    let directory = fixture_directory("absent");
    fs::create_dir_all(&directory).expect("fixture directory");
    fs::write(directory.join("main.doria"), "function main(): void {}\n").expect("source");
    let output = Command::new(doriac_bin())
        .current_dir(&directory)
        .args(["compile", "main.doria", "--out", executable_name()])
        .output()
        .expect("doriac");
    assert!(output.status.success());
    assert!(!directory.join("performance.json").exists());
    assert!(fs::read_dir(&directory)
        .expect("directory")
        .all(|entry| !entry
            .expect("entry")
            .file_name()
            .to_string_lossy()
            .contains("performance-report")));
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn report_write_failure_uses_a_structured_title_case_diagnostic() {
    if !host_linker_is_available() {
        return;
    }
    let directory = fixture_directory("write-failure");
    fs::create_dir_all(directory.join("occupied")).expect("fixture directory");
    fs::write(directory.join("main.doria"), "function main(): void {}\n").expect("source");
    let output = Command::new(doriac_bin())
        .current_dir(&directory)
        .args([
            "compile",
            "main.doria",
            "--out",
            executable_name(),
            "--performance-report",
            "occupied",
            "--diagnostic-format",
            "json",
        ])
        .output()
        .expect("doriac");
    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let envelope: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("diagnostic JSON");
    assert_eq!(envelope["diagnostics"][0]["code"], "B2601");
    assert_eq!(
        envelope["diagnostics"][0]["title"],
        "Performance Report Could Not Be Written"
    );
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn generic_specialization_fixture_reports_one_callable_specialization() {
    if !host_linker_is_available() {
        return;
    }
    let directory = fixture_directory("generic-specialization");
    fs::create_dir_all(&directory).expect("fixture directory");
    fs::write(
        directory.join("main.doria"),
        "function identity<T>(T $value): T { return $value; }\nfunction main(): int { return identity(42); }\n",
    )
    .expect("source");
    let output = Command::new(doriac_bin())
        .current_dir(&directory)
        .args([
            "compile",
            "main.doria",
            "--out",
            executable_name(),
            "--performance-report",
            "performance.json",
        ])
        .output()
        .expect("doriac");
    assert!(
        output.status.success(),
        "compile failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("performance.json")).expect("report"))
            .expect("JSON");
    assert_eq!(report["metrics"]["callableSpecializationCount"], 1);
    assert_eq!(report["metrics"]["classSpecializationCount"], 0);
    assert_eq!(report["metrics"]["totalGenericSpecializationCount"], 1);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn specialization_count_distinguishes_callables_with_the_same_type_arguments() {
    if !host_linker_is_available() {
        return;
    }
    let directory = fixture_directory("distinct-generic-specializations");
    fs::create_dir_all(&directory).expect("fixture directory");
    fs::write(
        directory.join("main.doria"),
        concat!(
            "function first<T>(T $value): T { return $value; }\n",
            "function second<T>(T $value): T { return $value; }\n",
            "function main(): int { return first(20) + second(22); }\n",
        ),
    )
    .expect("source");
    let output = Command::new(doriac_bin())
        .current_dir(&directory)
        .args([
            "compile",
            "main.doria",
            "--out",
            executable_name(),
            "--performance-report",
            "performance.json",
        ])
        .output()
        .expect("doriac");
    assert!(
        output.status.success(),
        "compile failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("performance.json")).expect("report"))
            .expect("JSON");
    assert_eq!(report["metrics"]["callableSpecializationCount"], 2);
    assert_eq!(report["metrics"]["classSpecializationCount"], 0);
    assert_eq!(report["metrics"]["totalGenericSpecializationCount"], 2);
    let _ = fs::remove_dir_all(directory);
}

#[test]
fn generic_class_fixture_reports_structural_and_class_specialization_counts() {
    if !host_linker_is_available() {
        return;
    }
    let directory = fixture_directory("generic-class-structure");
    fs::create_dir_all(&directory).expect("fixture directory");
    fs::write(
        directory.join("main.doria"),
        concat!(
            "class Box<T> { function __construct(take T $value) {} }\n",
            "function main(): void {\n",
            "    let $number = new Box<int>(42);\n",
            "    let $text = new Box<string>(\"doria\");\n",
            "    echo \"{$number->value}:{$text->value}\\n\";\n",
            "}\n",
        ),
    )
    .expect("source");
    let output = Command::new(doriac_bin())
        .current_dir(&directory)
        .args([
            "compile",
            "main.doria",
            "--out",
            executable_name(),
            "--performance-report",
            "performance.json",
        ])
        .output()
        .expect("doriac");
    assert!(
        output.status.success(),
        "compile failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("performance.json")).expect("report"))
            .expect("JSON");
    assert_eq!(report["metrics"]["classSpecializationCount"], 2);
    assert_eq!(report["metrics"]["totalGenericSpecializationCount"], 2);
    assert!(report["metrics"]["mirBasicBlockCount"]
        .as_u64()
        .is_some_and(|value| value >= 3));
    assert!(report["metrics"]["mirStatementCount"].as_u64().is_some());
    let _ = fs::remove_dir_all(directory);
}

#[cfg(feature = "llvm-backend")]
#[test]
fn release_report_identifies_llvm_without_cranelift_phase_data() {
    if !host_linker_is_available() {
        return;
    }
    let directory = fixture_directory("llvm");
    fs::create_dir_all(&directory).expect("fixture directory");
    fs::write(
        directory.join("main.doria"),
        "function main(): int { return 42; }\n",
    )
    .expect("source");
    let output = Command::new(doriac_bin())
        .current_dir(&directory)
        .args([
            "compile",
            "main.doria",
            "--release",
            "--out",
            executable_name(),
            "--performance-report",
            "performance.json",
        ])
        .output()
        .expect("doriac");
    assert!(
        output.status.success(),
        "compile failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.join("performance.json")).expect("report"))
            .expect("JSON");
    assert_eq!(report["backend"], "llvm");
    assert_eq!(report["phases"]["llvmCodeGeneration"]["available"], true);
    assert_eq!(
        report["phases"]["craneliftCodeGeneration"]["available"],
        false
    );
    let _ = fs::remove_dir_all(directory);
}

fn doriac_bin() -> &'static str {
    env!("CARGO_BIN_EXE_doriac")
}

fn executable_name() -> &'static str {
    if cfg!(windows) {
        "program.exe"
    } else {
        "program"
    }
}

fn host_linker_is_available() -> bool {
    let linker = if cfg!(all(windows, target_env = "msvc")) {
        "cl.exe"
    } else {
        "cc"
    };
    Command::new(linker).arg("--version").output().is_ok()
}

fn fixture_directory(label: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "doriac-performance-{label}-{}-{nanos}",
        std::process::id()
    ))
}
