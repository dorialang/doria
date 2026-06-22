use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

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
