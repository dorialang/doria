use std::collections::BTreeMap;

use doriac::diagnostics::{ColorChoice, DiagnosticFormat, RenderOptions};
use doriac::mir_interpreter::{interpret_with_io, MirIo, MirIoFaults, MirIoWriteFailure};

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
            args: Vec::new(),
            ..MirIo::default()
        },
    )
    .expect("MIR should interpret")
}

fn interpret_with_host(source: &str, io: MirIo) -> doriac::mir_interpreter::InterpreterIoOutput {
    let program = doriac::lower_source_to_mir("test.doria", source).expect("source should lower");
    interpret_with_io(&program, io).expect("MIR should interpret")
}

const PROMPTED_READ: &str = r#"
function main(): void
{
    let $line = read_line("Name: ");
    if ($line != null) { echo $line; }
}
"#;

#[test]
fn prompted_read_line_observes_write_flush_and_read_in_order() {
    let output = interpret_with_host(
        PROMPTED_READ,
        MirIo {
            stdin: b"Doria\n".to_vec(),
            ..MirIo::default()
        },
    );

    assert_eq!(output.output.stdout, b"Name: Doria");
    assert_eq!(output.trace.prompt_writes, 1);
    assert_eq!(output.trace.stdout_flushes, 1);
    assert_eq!(output.trace.stdin_line_reads, 1);

    let empty = interpret_with_host(
        "function main(): void { let $line = read_line(); }",
        MirIo::default(),
    );
    assert_eq!(empty.trace.prompt_writes, 0);
    assert_eq!(empty.trace.stdout_flushes, 1);
    assert_eq!(empty.trace.stdin_line_reads, 1);
}

#[test]
fn prompted_read_line_broken_pipes_exit_cleanly_before_stdin() {
    for faults in [
        MirIoFaults {
            prompt_write: Some(MirIoWriteFailure::BrokenPipe),
            ..MirIoFaults::default()
        },
        MirIoFaults {
            stdout_flush: Some(MirIoWriteFailure::BrokenPipe),
            ..MirIoFaults::default()
        },
    ] {
        let output = interpret_with_host(
            PROMPTED_READ,
            MirIo {
                stdin: b"must not be read\n".to_vec(),
                faults,
                ..MirIo::default()
            },
        );
        assert_eq!(output.output.exit_status, 0);
        assert!(output.output.stderr.is_empty());
        assert!(output.output.runtime_diagnostic.is_none());
        assert_eq!(output.trace.stdin_line_reads, 0);
    }
}

#[test]
fn prompted_read_line_host_failures_keep_their_runtime_codes() {
    let cases = [
        (
            MirIoFaults {
                prompt_write: Some(MirIoWriteFailure::Other),
                ..MirIoFaults::default()
            },
            b"".as_slice(),
            "P1407",
        ),
        (
            MirIoFaults {
                stdout_flush: Some(MirIoWriteFailure::Other),
                ..MirIoFaults::default()
            },
            b"Name: ".as_slice(),
            "P1407",
        ),
        (
            MirIoFaults {
                stdin_line_read: true,
                ..MirIoFaults::default()
            },
            b"Name: ".as_slice(),
            "P1403",
        ),
        (
            MirIoFaults {
                line_allocation: true,
                ..MirIoFaults::default()
            },
            b"Name: ".as_slice(),
            "P1206",
        ),
    ];

    for (faults, stdout, code) in cases {
        let output = interpret_with_host(
            PROMPTED_READ,
            MirIo {
                stdin: b"Doria\n".to_vec(),
                faults,
                ..MirIo::default()
            },
        );
        assert_eq!(output.output.stdout, stdout, "{code}");
        assert_eq!(output.output.exit_status, 101, "{code}");
        assert_eq!(
            output
                .output
                .runtime_diagnostic
                .as_ref()
                .map(|diagnostic| diagnostic.code),
            Some(code),
        );
    }

    let invalid_utf8 = interpret_with_host(
        PROMPTED_READ,
        MirIo {
            stdin: vec![0xff, b'\n'],
            ..MirIo::default()
        },
    );
    assert_eq!(invalid_utf8.output.stdout, b"Name: ");
    assert_eq!(invalid_utf8.output.exit_status, 101);
    assert_eq!(
        invalid_utf8
            .output
            .runtime_diagnostic
            .as_ref()
            .map(|diagnostic| diagnostic.code),
        Some("P1404"),
    );
}

#[test]
fn read_line_distinguishes_empty_lines_final_bytes_and_eof() {
    let output = interpret(
        r#"
function main(): void
{
    let writable $line = read_line();
    while ($line != null) {
        echo "[" . $line . "]\n";
        $line = read_line();
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
fn php_readline_spelling_is_rejected_with_doria_guidance() {
    let diagnostics = doriac::check_source(
        "test.doria",
        "function main(): void { let $line = readline(); }",
    )
    .expect_err("the PHP readline spelling must not be accepted");

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0461"
            && diagnostic.message.contains("`read_line`")
            && diagnostic.help.as_deref() == Some("replace `readline()` with `read_line()`")
    }));

    let diagnostics = doriac::check_source(
        "test.doria",
        "function readline(): void {} function main(): void {}",
    )
    .expect_err("the PHP spelling must not be available for userland declarations");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0310" && diagnostic.message.contains("`read_line`")
    }));
}

#[test]
fn prompted_read_line_diagnostics_survive_every_presentation() {
    let cases = [
        ("function main(): void { read_line(42); }", "E0453"),
        ("function main(): void { read_line(null); }", "E0453"),
        (
            "function main(): void { read_line(\"a\", \"b\"); }",
            "E0450",
        ),
    ];

    for (source, code) in cases {
        let diagnostics = doriac::check_source("prompt-error.doria", source)
            .expect_err("invalid prompted input must be rejected");
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == code)
            .unwrap_or_else(|| panic!("expected {code}, got {diagnostics:?}"));
        assert!(diagnostic.span.end > diagnostic.span.start);

        for format in [
            DiagnosticFormat::Human,
            DiagnosticFormat::Concise,
            DiagnosticFormat::Json,
        ] {
            let rendered = doriac::render_diagnostics_with_options(
                "prompt-error.doria",
                source,
                &diagnostics,
                RenderOptions {
                    format,
                    color: ColorChoice::Never,
                    ..RenderOptions::default()
                },
            );
            assert!(rendered.contains(code), "{format:?}: {rendered}");
            if format == DiagnosticFormat::Json {
                let value: serde_json::Value =
                    serde_json::from_str(&rendered).expect("JSON diagnostics should parse");
                assert_eq!(value["schemaVersion"], 1);
            }
        }
    }
}

#[test]
fn php_readline_fixit_matches_diagnostic_snapshot() {
    let diagnostics = doriac::check_source(
        "test.doria",
        "function main(): void { let $line = readline(); }",
    )
    .expect_err("the PHP spelling must produce a diagnostic");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "E0461")
        .expect("the PHP spelling diagnostic should be present");
    let snapshot = format!(
        "code: {}\nmessage: {}\nhelp: {}\nspan: {}..{}\n",
        diagnostic.code,
        diagnostic.message,
        diagnostic.help.as_deref().unwrap_or(""),
        diagnostic.span.start,
        diagnostic.span.end,
    );

    let expected =
        include_str!("fixtures/diagnostics/php_readline_fixit.txt").replace("\r\n", "\n");
    assert_eq!(snapshot, expected);
}

#[test]
fn unimplemented_doria_replacements_are_not_suggested() {
    let diagnostics = doriac::check_source(
        "test.doria",
        "function main(): void { let $result = strcasecmp(\"a\", \"b\"); }",
    )
    .expect_err("an undeclared function should remain unknown");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0309"
            && diagnostic.message == "unknown function `strcasecmp`"
            && diagnostic.help.is_none()
    }));

    doriac::check_source(
        "test.doria",
        r#"
function strcasecmp(string $left, string $right): int
{
    return 0;
}

function main(): void
{
    let $result = strcasecmp("a", "b");
}
"#,
    )
    .expect("the fixit table must not shadow a declared userland function");
}

#[test]
fn read_file_rejects_invalid_utf8_before_constructing_a_string() {
    let mut files = BTreeMap::new();
    files.insert("invalid.txt".to_string(), vec![b'D', 0xff, b'a']);
    let output = interpret(
        r#"
function main(): void
{
    echo read_file("invalid.txt");
}
"#,
        b"",
        files,
    );

    assert!(output.output.stdout.is_empty());
    assert_eq!(output.output.exit_status, 101);
    let diagnostic = output
        .output
        .runtime_diagnostic
        .expect("invalid UTF-8 should retain a structured diagnostic");
    assert_eq!(diagnostic.code, "P1406");
    assert_eq!(diagnostic.title, "File Text Is Not Valid UTF-8");
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
        "function main(): void { let $line = read_line(); echo $line; }",
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
    let writable $line = read_line();
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
    let writable $line = read_line();
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
fn narrowed_read_line_values_lower_as_strings_in_calls_and_returns() {
    let output = interpret(
        r#"
function identity(string $value): string { return $value; }
function nextLine(): string
{
    let $line = read_line();
    if ($line != null) { return identity($line); }
    return "fallback";
}
function main(): void { echo nextLine(); }
"#,
        "Dória\n".as_bytes(),
        BTreeMap::new(),
    );
    assert_eq!(output.output.stdout, "Dória".as_bytes());
    assert!(output.output.stderr.is_empty());
    assert_eq!(output.output.exit_status, 0);
}

#[test]
fn nullable_flow_joins_loops_and_nested_guards_are_sound() {
    for source in [
        r#"
function check(bool $condition): void
{
    let writable $line = read_line();
    if ($condition) { $line = "known"; } else { $line = null; }
    echo $line;
}
function main(): void { check(true); }
"#,
        r#"
function check(bool $condition): void
{
    let writable $line = read_line();
    if ($condition) { $line = "known"; }
    echo $line;
}
function main(): void { check(true); }
"#,
        r#"
function main(): void
{
    let writable $line = read_line();
    while ($line != null) {
        $line = read_line();
        echo $line;
    }
}
"#,
    ] {
        let diagnostics = doriac::check_source("test.doria", source)
            .expect_err("nullable value should escape its flow proof");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0445"),
            "expected nullable display diagnostic, got {diagnostics:?} for {source}"
        );
    }

    doriac::lower_source_to_mir(
        "test.doria",
        r#"
function main(): void
{
    let $line = read_line();
    if ($line != null) {
        if ($line != "") {
            echo $line;
        }
    }
}
"#,
    )
    .expect("nested guards should preserve the established non-null fact");
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

#[test]
fn accepted_format_matrix_is_exact_and_width_aware() {
    let output = interpret(
        r#"
function main(): void
{
    int8 $i8 = -1;
    int16 $i16 = -1;
    int32 $i32 = -1;
    int64 $i64 = -1;
    uint8 $u8 = 255;
    uint16 $u16 = 65535;
    uint32 $u32 = 4294967295;
    uint64 $u64 = 18446744073709551615;
    float32 $f32 = 1.5;
    float64 $f64 = 1.5;

    echo sprintf("%d|%d|%05d|%05d|%-5d", 42, -42, 42, -42, 42);
    echo sprintf("|%s|%s|%-6s|%6s", false, true, "Doria", "é");
    echo sprintf("|%x|%X|%o|%b|%08x", 255, 255, 255, 10, 255);
    echo sprintf("|%f|%.2f|%.0f|%.0f|%.2f|%s", 1.5, 3.14159, 2.5, 3.5, -0.0, -0.0);
    echo sprintf("|%f|%f|%f|%f|%f", $f32, $f64, 0.0 / 0.0, 1.0 / 0.0, -1.0 / 0.0);
    echo sprintf("|%d|%d|%d|%d", $u8, $u16, $u32, $u64);
    echo sprintf("|%x,%X,%o,%b", $i8, $i8, $i8, $i8);
    echo sprintf("|%x,%X,%o,%b", $i16, $i16, $i16, $i16);
    echo sprintf("|%x,%X,%o,%b", $i32, $i32, $i32, $i32);
    echo sprintf("|%x,%X,%o,%b", $i64, $i64, $i64, $i64);
    echo sprintf("|%-5d|%06d|%%|before:%d:between:%s:after", 42, -42, 7, "x");
}
"#,
        b"",
        BTreeMap::new(),
    );
    assert_eq!(
        String::from_utf8(output.output.stdout).expect("format output should be UTF-8"),
        concat!(
            "42|-42|00042|-0042|42   |false|true|Doria |    é|ff|FF|377|1010|000000ff",
            "|1.500000|3.14|2|4|-0.00|-0|1.500000|1.500000|NaN|Infinity|-Infinity",
            "|255|65535|4294967295|18446744073709551615",
            "|ff,FF,377,11111111",
            "|ffff,FFFF,177777,1111111111111111",
            "|ffffffff,FFFFFFFF,37777777777,11111111111111111111111111111111",
            "|ffffffffffffffff,FFFFFFFFFFFFFFFF,1777777777777777777777,",
            "1111111111111111111111111111111111111111111111111111111111111111",
            "|42   |-00042|%|before:7:between:x:after"
        )
    );
}

#[test]
fn format_arguments_evaluate_left_to_right_once_and_printf_adds_no_newline() {
    let output = interpret(
        r#"
function mark(int $value): int
{
    echo $value;
    return $value;
}

function main(): void
{
    echo sprintf("[%d,%d,%d]", mark(1), mark(2), mark(3));
    printf("<%s>", false);
}
"#,
        b"",
        BTreeMap::new(),
    );
    assert_eq!(output.output.stdout, b"123[1,2,3]<false>");
}

#[test]
fn rejected_format_matrix_is_diagnosed_before_mir() {
    for source in [
        "function main(): void { let $f = \"%d\"; echo sprintf($f, 1); }",
        "function main(): void { let $x = 1; echo sprintf(\"{$x}\", 1); }",
        "function main(): void { echo sprintf(\"%d\"); }",
        "function main(): void { echo sprintf(\"%d\", 1, 2); }",
        "function main(): void { echo sprintf(\"%f\", 1); }",
        "function main(): void { echo sprintf(\"%x\", \"x\"); }",
        "function main(): void { let $line = read_line(); echo sprintf(\"%s\", $line); }",
        "function main(): void { echo sprintf(\"%.2d\", 1); }",
        "function main(): void { echo sprintf(\"%.2s\", \"x\"); }",
        "function main(): void { echo sprintf(\"%0s\", \"x\"); }",
        "function main(): void { echo sprintf(\"%--5d\", 1); }",
        "function main(): void { echo sprintf(\"%e\", 1.0); }",
        "function main(): void { echo sprintf(\"%g\", 1.0); }",
        "function main(): void { echo sprintf(\"%1$s\", \"x\"); }",
        "function main(): void { echo sprintf(\"%*d\", 5, 1); }",
        "function main(): void { echo sprintf(\"%\"); }",
        "function main(): void { echo sprintf(\"%q\", 1); }",
        "function main(): void { echo sprintf(\"%4294967296d\", 1); }",
        "function main(): void { echo sprintf(\"%.4294967296f\", 1.0); }",
        "function main(): void { echo sprintf(\"%..2f\", 1.0); }",
        "function main(): void { let $result = printf(\"%d\", 1); }",
    ] {
        assert!(
            doriac::lower_source_to_mir("test.doria", source).is_err(),
            "invalid format source reached MIR: {source}"
        );
    }
}
