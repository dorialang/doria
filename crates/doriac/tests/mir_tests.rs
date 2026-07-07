use doriac::backend::{BackendOutput, BackendTarget};
use doriac::mir::{ReturnType, Statement, Terminator};

fn lower(source: &str) -> doriac::mir::Program {
    doriac::lower_source_to_mir("test.doria", source).expect("source should lower to MIR")
}

fn interpret(source: &str) -> doriac::mir_interpreter::InterpreterOutput {
    let program = lower(source);
    doriac::mir_interpreter::interpret(&program).expect("MIR should interpret")
}

#[test]
fn lowers_main_int_return_42() {
    let program = lower(
        r#"function main(): int
{
    return 42;
}
"#,
    );

    let function = &program.functions[program.entry.0];
    assert_eq!(function.name, "main");
    assert_eq!(function.return_type, ReturnType::Int);
    assert_eq!(function.blocks[0].terminator, Terminator::ReturnInt(42));
    assert_eq!(
        program.to_string(),
        "function main(): int {\nblock0:\n    return 42\n}\n"
    );
}

#[test]
fn lowers_main_void_empty_to_return_void() {
    let program = lower(
        r#"function main(): void
{
}
"#,
    );

    let function = &program.functions[program.entry.0];
    assert_eq!(function.return_type, ReturnType::Void);
    assert_eq!(function.blocks[0].terminator, Terminator::ReturnVoid);
}

#[test]
fn lowers_main_void_bare_return_to_return_void() {
    let program = lower(
        r#"function main(): void
{
    return;
}
"#,
    );

    assert_eq!(
        program.functions[0].blocks[0].terminator,
        Terminator::ReturnVoid
    );
}

#[test]
fn lowers_string_literal_echo() {
    let program = lower(
        r#"function main(): void
{
    echo "Hello Doria!";
}
"#,
    );

    assert_eq!(
        program.functions[0].blocks[0].statements,
        vec![Statement::EchoStringLiteral("Hello Doria!".to_string())]
    );
    assert_eq!(
        program.to_string(),
        "function main(): void {\nblock0:\n    echo \"Hello Doria!\"\n    return\n}\n"
    );
}

#[test]
fn lowers_multiple_string_literal_echoes_in_order() {
    let program = lower(
        r#"function main(): void
{
    echo "Hello ";
    echo "Doria!";
}
"#,
    );

    assert_eq!(
        program.functions[0].blocks[0].statements,
        vec![
            Statement::EchoStringLiteral("Hello ".to_string()),
            Statement::EchoStringLiteral("Doria!".to_string()),
        ]
    );
}

#[test]
fn interprets_main_int_return_42() {
    let output = interpret(
        r#"function main(): int
{
    return 42;
}
"#,
    );

    assert_eq!(output.exit_status, 42);
    assert!(output.stdout.is_empty());
}

#[test]
fn interprets_main_void_empty() {
    let output = interpret(
        r#"function main(): void
{
}
"#,
    );

    assert_eq!(output.exit_status, 0);
    assert!(output.stdout.is_empty());
}

#[test]
fn interprets_main_void_bare_return() {
    let output = interpret(
        r#"function main(): void
{
    return;
}
"#,
    );

    assert_eq!(output.exit_status, 0);
    assert!(output.stdout.is_empty());
}

#[test]
fn interprets_main_void_hello() {
    let output = interpret(
        r#"function main(): void
{
    echo "Hello Doria!";
}
"#,
    );

    assert_eq!(output.exit_status, 0);
    assert_eq!(output.stdout, b"Hello Doria!");
}

#[test]
fn interprets_multiple_echoes_without_newline() {
    let output = interpret(
        r#"function main(): void
{
    echo "Hello ";
    echo "Doria!";
}
"#,
    );

    assert_eq!(output.exit_status, 0);
    assert_eq!(output.stdout, b"Hello Doria!");
}

#[test]
fn debug_target_emits_interpreter_artifact() {
    let output = doriac::compile_source(
        "test.doria",
        r#"function main(): void
{
    echo "Hello Doria!";
}
"#,
        BackendTarget::Debug,
    )
    .expect("debug target should compile");

    let BackendOutput::Text {
        extension,
        contents,
    } = output
    else {
        panic!("debug target should emit text output");
    };
    assert_eq!(extension, "debug");
    assert_eq!(contents, "exit_status: 0\nstdout: Hello Doria!\n");
}

#[test]
fn debug_backend_emit_handles_stage_11a_hir_directly() {
    let hir = doriac::lower_source(
        "test.doria",
        r#"function main(): int
{
    return 42;
}
"#,
    )
    .expect("source should lower to HIR");

    let output = doriac::backend::emit(&hir, BackendTarget::Debug)
        .expect("available debug target should emit through the public backend layer");

    let BackendOutput::Text {
        extension,
        contents,
    } = output
    else {
        panic!("debug backend should emit text output");
    };
    assert_eq!(extension, "debug");
    assert_eq!(contents, "exit_status: 42\nstdout:\n");
}
#[test]
fn rejects_unsupported_arithmetic_return_as_mir_coverage() {
    let diagnostics = doriac::lower_source_to_mir(
        "test.doria",
        r#"function main(): int
{
    return 40 + 2;
}
"#,
    )
    .expect_err("arithmetic return should be outside Stage 11a MIR coverage");

    assert_eq!(diagnostics[0].code, "M1101");
    assert!(diagnostics[0]
        .message
        .contains("unsupported MIR Stage 11a coverage"));
    assert!(diagnostics[0].message.contains("non-literal expression"));
}

#[test]
fn rejects_unsupported_string_concat_echo_as_mir_coverage() {
    let diagnostics = doriac::lower_source_to_mir(
        "test.doria",
        r#"function main(): void
{
    echo "Hello " . "Doria!";
}
"#,
    )
    .expect_err("string concat echo should be outside Stage 11a MIR coverage");

    assert_eq!(diagnostics[0].code, "M1101");
    assert!(diagnostics[0].message.contains("string concatenation"));
}

#[test]
fn rejects_unsupported_local_variable_as_mir_coverage() {
    let diagnostics = doriac::lower_source_to_mir(
        "test.doria",
        r#"function main(): void
{
    let $message = "Hello Doria!";
    echo $message;
}
"#,
    )
    .expect_err("local variable should be outside Stage 11a MIR coverage");

    assert_eq!(diagnostics[0].code, "M1101");
    assert!(diagnostics[0].message.contains("local variables"));
}

#[test]
fn rejects_unsupported_helper_function_as_mir_coverage() {
    let diagnostics = doriac::lower_source_to_mir(
        "test.doria",
        r#"function helper(): int
{
    return 1;
}

function main(): int
{
    return 42;
}
"#,
    )
    .expect_err("helper functions should be outside Stage 11a MIR coverage");

    assert_eq!(diagnostics[0].code, "M1101");
    assert!(diagnostics[0]
        .message
        .contains("unsupported MIR Stage 11a coverage"));
    assert!(diagnostics[0].message.contains("no helper functions"));
}

#[test]
fn mirrors_native_smoke_exit_for_literal_main_shapes_without_linker() {
    for (source, expected) in [
        (
            r#"function main(): int
{
    return 42;
}
"#,
            42,
        ),
        (
            r#"function main(): void
{
}
"#,
            0,
        ),
        (
            r#"function main(): void
{
    return;
}
"#,
            0,
        ),
    ] {
        let hir = doriac::lower_source("test.doria", source).expect("source should lower to HIR");
        let native_exit = doriac::codegen_native::validate_stage_2d(&hir)
            .expect("native smoke validator should accept source");
        let mir_exit = interpret(source).exit_status;

        assert_eq!(native_exit, expected);
        assert_eq!(mir_exit, native_exit);
    }
}
