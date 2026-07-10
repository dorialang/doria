use doriac::backend::BackendTarget;
use doriac::diagnostics::Diagnostic;

#[test]
fn native_preflight_rejects_process_status_outside_portable_boundary() {
    let diagnostics = compile_error(
        r#"function main(): int
{
    return 126;
}
"#,
        BackendTarget::Native,
    );

    assert_eq!(diagnostics[0].code, "B0001");
    assert!(diagnostics[0]
        .message
        .contains("process exit status must be in the range 0..125"));
}

#[test]
fn native_and_debug_preflight_preserve_checked_integer_arithmetic() {
    for (name, source, operation) in [
        (
            "addition",
            r#"function add(int $left, int $right): int
{
    return $left + $right;
}

function main(): int
{
    return add(9223372036854775807, 1);
}
"#,
            "addition",
        ),
        (
            "subtraction",
            r#"function subtract(int $left, int $right): int
{
    return $left - $right;
}

function main(): int
{
    return subtract(0 - 9223372036854775807, 2);
}
"#,
            "subtraction",
        ),
        (
            "multiplication",
            r#"function multiply(int $left, int $right): int
{
    return $left * $right;
}

function main(): int
{
    return multiply(4611686018427387904, 2);
}
"#,
            "multiplication",
        ),
    ] {
        let native = compile_error(source, BackendTarget::Native);
        let debug = compile_error(source, BackendTarget::Debug);

        assert_eq!(native[0].code, "B0001", "{name}");
        assert_eq!(debug[0].code, "M1102", "{name}");
        assert!(native[0].message.contains(operation), "{name}");
        assert!(debug[0].message.contains(operation), "{name}");
    }
}

#[test]
fn native_preflight_preserves_bounded_execution() {
    let diagnostics = compile_error(
        r#"function main(): void
{
    let writable $i = 0;

    while (true) {
        $i++;
    }
}
"#,
        BackendTarget::Native,
    );

    assert_eq!(diagnostics[0].code, "B0001");
    assert!(diagnostics[0]
        .message
        .contains("exhausted its bounded execution fuel"));
}

#[test]
fn native_and_debug_share_mir_coverage_diagnostics() {
    for (name, source, expected) in [
        (
            "direct recursion",
            r#"function count(int $value): int
{
    return count($value);
}

function main(): int
{
    return count(1);
}
"#,
            "recursive calls are not supported",
        ),
        (
            "mutual recursion",
            r#"function left(): int
{
    return right();
}

function right(): int
{
    return left();
}

function main(): int
{
    return left();
}
"#,
            "mutual recursion is not supported",
        ),
        (
            "string parameter",
            r#"function greet(string $name): void
{
    echo $name;
}

function main(): void
{
}
"#,
            "supports only int parameters",
        ),
        (
            "string return",
            r#"function greeting(): string
{
    return "Hello";
}

function main(): void
{
}
"#,
            "supports only int and void returns",
        ),
        (
            "writable string local",
            r#"function main(): void
{
    let writable $message = "Hello";
    echo $message;
}
"#,
            "writable string locals",
        ),
        (
            "collection foreach",
            r#"function main(): void
{
    foreach ([1, 2] as int $item) {
    }
}
"#,
            "collection and general iterable foreach",
        ),
        (
            "class",
            r#"class Person
{
}

function main(): void
{
}
"#,
            "classes are not lowered to MIR",
        ),
    ] {
        let native = compile_error(source, BackendTarget::Native);
        let debug = compile_error(source, BackendTarget::Debug);

        assert_eq!(native[0].code, "M1101", "{name}");
        assert_eq!(debug[0].code, native[0].code, "{name}");
        assert_eq!(debug[0].message, native[0].message, "{name}");
        assert!(
            native[0].message.contains(expected),
            "{name}: expected `{expected}` in `{}`",
            native[0].message
        );
    }
}

fn compile_error(source: &str, target: BackendTarget) -> Vec<Diagnostic> {
    doriac::compile_source("test.doria", source, target)
        .expect_err("source should be rejected by the selected Stage 11 path")
}
