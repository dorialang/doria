#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/php_execution.rs"]
mod php_execution;

use std::io::Write;
use std::process::Stdio;

#[test]
fn large_php_sources_do_not_cross_the_process_argument_boundary() {
    let source = format!("/*{}*/ echo 'large source';", "x".repeat(512 * 1024));
    let script = php_execution::Script::new(&source);
    let command = script.command(&["-n"]);
    assert_eq!(command.get_args().count(), 2);
    assert!(command
        .get_args()
        .all(|argument| argument != std::ffi::OsStr::new(&source)));
    let path = std::path::PathBuf::from(command.get_args().last().unwrap());
    assert_eq!(
        std::fs::read_to_string(path.with_file_name("script.php")).unwrap(),
        format!("<?php\n{source}")
    );
    let output = script.command(&["-n"]).output().expect("PHP is required");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, b"large source");
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn generated_entry_guards_do_not_preempt_explicit_test_invocation() {
    let script = php_execution::Script::new(
        r#"
function fixtureMain(): int { echo 'called;'; return 17; }
if (isset($_SERVER['SCRIPT_FILENAME']) && realpath($_SERVER['SCRIPT_FILENAME']) === __FILE__) {
    exit(fixtureMain());
}
echo fixtureMain();
"#,
    );
    let output = script.command(&[]).output().expect("PHP is required");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, b"called;17");
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn script_files_preserve_program_stdin_status_and_php_options() {
    let script = php_execution::Script::new(
        "echo ini_get('memory_limit'), ':'; fwrite(STDOUT, stream_get_contents(STDIN)); exit(23);",
    );
    let mut child = script
        .command(&["-d", "memory_limit=16M"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("PHP is required");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"input\0\xff")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(23), "{output:?}");
    assert_eq!(output.stdout, b"16M:input\0\xff");
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn script_storage_is_released_after_success_and_failed_launch() {
    for fail_launch in [false, true] {
        let path = {
            let script = php_execution::Script::new("echo 'owned';");
            let mut command = script.command(&[]);
            let path = std::path::PathBuf::from(command.get_args().last().unwrap());
            if fail_launch {
                command.current_dir(path.join("not-a-directory"));
                assert!(command.output().is_err());
            } else {
                let output = command.output().expect("PHP is required");
                assert_eq!(output.status.code(), Some(0), "{output:?}");
                assert_eq!(output.stdout, b"owned");
                assert!(output.stderr.is_empty(), "{output:?}");
            }
            assert!(path.is_file());
            path
        };
        assert!(!path.exists());
        assert!(!path.parent().unwrap().exists());
    }
}
