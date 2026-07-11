use doriac::backend::{Backend, BackendOutput, BackendTarget, NativeBackend};
use doriac::diagnostics::Diagnostic;

#[test]
fn native_compilation_does_not_execute_an_infinite_program() {
    let source = include_str!("../../../examples/compile-only/main_infinite_while.doria");
    let hir = doriac::lower_source("main_infinite_while.doria", source)
        .expect("infinite loop should pass frontend checking");
    let output = NativeBackend
        .emit(&hir)
        .expect("native compilation should not execute user code as a preflight");
    let BackendOutput::Executable { bytes, .. } = output else {
        panic!("native backend should emit an executable");
    };
    assert!(!bytes.is_empty());
}

#[test]
fn native_and_debug_share_remaining_mir_coverage_diagnostics() {
    for (name, source, expected) in [
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
            "Stage 14 MIR supports scalar parameters",
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
            "Stage 14 MIR supports scalar and void returns",
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
        .expect_err("source should be outside current MIR coverage")
}
