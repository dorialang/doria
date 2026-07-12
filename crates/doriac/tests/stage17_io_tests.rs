use std::collections::BTreeMap;

use doriac::mir_interpreter::{interpret_with_io, MirIo};

fn interpret(
    source: &str,
    stdin: &[u8],
    files: BTreeMap<String, Vec<u8>>,
) -> doriac::mir_interpreter::InterpreterIoOutput {
    let program = doriac::lower_source_to_mir("test.doria", source).expect("source should lower");
    interpret_with_io(
        &program,
        MirIo {
            stdin: stdin.to_vec(),
            files,
        },
    )
    .expect("MIR should interpret")
}

#[test]
fn readline_distinguishes_empty_lines_final_bytes_and_eof() {
    let output = interpret(
        r#"
function main(): void
{
    let writable $line = readline();
    while ($line != null) {
        echo "[" . $line . "]\n";
        $line = readline();
    }
}
"#,
        b"alpha\r\n\nfinal",
        BTreeMap::new(),
    );
    assert_eq!(output.output.stdout, b"[alpha]\n[]\n[final]\n");
    assert!(output.output.stderr.is_empty());
    assert_eq!(output.output.exit_status, 0);
}

#[test]
fn files_stderr_and_checked_formatting_share_deterministic_io() {
    let mut files = BTreeMap::new();
    files.insert("input.txt".to_string(), "Dória\0line".as_bytes().to_vec());
    let output = interpret(
        r#"
function main(): int
{
    let $contents = read_file("input.txt");
    write_file("copy.txt", $contents);
    write_stderr("err");
    printf("%05d|%-6s|%.2f|%x|%%", 42, "Doria", 3.14159, 255);
    return 42;
}
"#,
        b"",
        files,
    );
    assert_eq!(output.output.stdout, b"00042|Doria |3.14|ff|%");
    assert_eq!(output.output.stderr, b"err");
    assert_eq!(output.output.exit_status, 42);
    assert_eq!(output.files["copy.txt"], "Dória\0line".as_bytes());
}

#[test]
fn nullable_and_format_diagnostics_are_checked_before_mir() {
    for source in [
        "function main(): void { let $line = readline(); echo $line; }",
        "function main(): void { let $format = \"%d\"; echo sprintf($format, 1); }",
        "function main(): void { echo sprintf(\"%d\", 1.5); }",
        "function main(): void { echo sprintf(\"%0s\", \"x\"); }",
        "function main(): void { print(\"x\"); }",
    ] {
        assert!(
            doriac::check_source("test.doria", source).is_err(),
            "{source}"
        );
    }
}

#[test]
fn nullable_assignment_invalidates_non_null_narrowing() {
    let source = r#"
function main(): void
{
    let writable $line = readline();
    if ($line != null) {
        $line = null;
        echo $line;
    }
}
"#;
    let diagnostics = doriac::check_source("test.doria", source)
        .expect_err("a null assignment must invalidate the non-null flow fact");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0445"),
        "expected nullable display diagnostic, got {diagnostics:?}"
    );

    doriac::lower_source_to_mir(
        "test.doria",
        r#"
function main(): void
{
    let writable $line = readline();
    if ($line != null) {
        $line = "known";
        echo $line;
    }
}
"#,
    )
    .expect("assigning a non-null string should establish a new non-null flow fact");
}

#[test]
fn format_type_diagnostics_use_source_specifiers() {
    let diagnostics = doriac::check_source(
        "test.doria",
        "function main(): void { echo sprintf(\"%x\", 1.5); }",
    )
    .expect_err("float passed to %x should fail");
    assert!(
        diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "E0457" && diagnostic.message.contains("`%x`")
        }),
        "expected source format spelling, got {diagnostics:?}"
    );
}
