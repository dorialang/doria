use doriac::mir_interpreter::interpret;

fn lower(source: &str) -> doriac::mir::Program {
    doriac::lower_source_to_mir("stage16.doria", source)
        .unwrap_or_else(|diagnostics| panic!("Stage 16 source should lower: {diagnostics:#?}"))
}

fn run(source: &str) -> doriac::mir_interpreter::InterpreterOutput {
    interpret(&lower(source)).expect("Stage 16 MIR should interpret")
}

#[test]
fn runtime_strings_cross_locals_calls_returns_concat_and_interpolation() {
    let output = run(r#"
function join(string $left, string $right): string
{
    let writable $result = $left;
    $result = $result . "-" . $right;
    return $result;
}

function main(): void
{
    let $count = 42;
    let $enabled = false;
    echo join("Doria", "runtime") . "; count=" . $count . "; enabled={$enabled}";
}
"#);
    assert_eq!(output.stdout, b"Doria-runtime; count=42; enabled=false");
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
}

#[test]
fn canonical_float_and_bool_display_is_exact() {
    let output = run(r#"
function main(): void
{
    uint64 $maximum = 18446744073709551615;
    float32 $narrow = 1.5;
    echo "values=" . $maximum . "," . $narrow . "," . -0.0 . "," . true . "," . false;
}
"#);
    assert_eq!(
        output.stdout,
        b"values=18446744073709551615,1.5,-0,true,false"
    );
}

#[test]
fn string_comparison_is_utf8_byte_lexicographic() {
    let output = run(r#"
function main(): int
{
    if ("abc" == "abc" and "abc" != "abd" and "" < "a" and "z" < "é") {
        return 42;
    }
    return 0;
}
"#);
    assert_eq!(output.exit_status, 42);
}

#[test]
fn copy_then_rebind_preserves_source_value_and_self_assignment() {
    let output = run(r#"
function main(): void
{
    let $original = "x";
    let writable $copy = $original;
    $copy = $copy;
    $copy = $copy . "y";
    echo $original;
    echo $copy;
}
"#);
    assert_eq!(output.stdout, b"xxy");
}

#[test]
fn runtime_string_panic_message_preserves_trace_and_status() {
    let output = run(r#"
function fail(string $subject): void
{
    panic("runtime " . $subject . " " . 16);
}
function main(): void { fail("string"); }
"#);
    assert_eq!(output.stdout, b"");
    assert_eq!(
        output.stderr,
        b"Panic: runtime string 16\nStack Trace:\n  at fail\n  at main\n"
    );
    assert_eq!(output.exit_status, 101);
}

#[test]
fn cranelift_accepts_the_complete_runtime_string_slice() {
    let program = lower(
        r#"
function identity(string $value): string { return $value; }
function main(): void { echo identity("Doria") . 16 . false; }
"#,
    );
    let object = doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("Cranelift should lower Stage 16 strings");
    assert!(!object.is_empty());
}

#[test]
fn rejects_non_string_concat_and_non_display_echo() {
    let concat = doriac::check_source(
        "invalid.doria",
        "function main(): void { let $value = 1 . 2; }",
    )
    .expect_err("integer-only concat must be rejected");
    assert!(concat.iter().any(|diagnostic| diagnostic
        .message
        .contains("concatenation requires at least one string operand")));

    let echo = doriac::check_source("invalid.doria", "function main(): void { echo null; }")
        .expect_err("null display must be rejected");
    assert!(echo
        .iter()
        .any(|diagnostic| diagnostic.message.contains("cannot be displayed")));
}

#[test]
fn rejects_string_scalar_boundary_mismatches_and_string_main() {
    for source in [
        "function acceptString(string $value): void {} function main(): void { acceptString(1); }",
        "function bad(): string { return 1; } function main(): void {}",
        "function bad(): int { return \"x\"; } function main(): void {}",
        "function main(): string { return \"x\"; }",
        "function main(): void { echo \"x\" == 1; }",
    ] {
        assert!(
            doriac::check_source("invalid.doria", source).is_err(),
            "{source}"
        );
    }
}
