use doriac::backend::{BackendOutput, BackendTarget};
use doriac::mir::{
    BasicBlock, BinaryOp, BlockId, Function, FunctionId, Local, LocalId, Operand, Program,
    ReturnType, Rvalue, Statement, Terminator, Type,
};

fn lower(source: &str) -> doriac::mir::Program {
    doriac::lower_source_to_mir("test.doria", source).expect("source should lower to MIR")
}

fn interpret(source: &str) -> doriac::mir_interpreter::InterpreterOutput {
    let program = lower(source);
    doriac::mir_interpreter::interpret(&program).expect("MIR should interpret")
}

fn unsupported(source: &str) -> Vec<doriac::diagnostics::Diagnostic> {
    doriac::lower_source_to_mir("test.doria", source)
        .expect_err("source should be outside Stage 11b MIR coverage")
}

fn assert_stage_11b_unsupported(diagnostics: &[doriac::diagnostics::Diagnostic], detail: &str) {
    assert_eq!(diagnostics[0].code, "M1101");
    assert!(
        diagnostics[0]
            .message
            .contains("unsupported MIR Stage 11b coverage"),
        "unexpected diagnostic: {}",
        diagnostics[0].message
    );
    assert!(
        diagnostics[0].message.contains(detail),
        "diagnostic `{}` did not contain `{detail}`",
        diagnostics[0].message
    );
}

fn debug_contents(source: &str) -> String {
    let output = doriac::compile_source("test.doria", source, BackendTarget::Debug)
        .expect("debug target should compile");

    let BackendOutput::Text {
        extension,
        contents,
    } = output
    else {
        panic!("debug target should emit text output");
    };
    assert_eq!(extension, "debug");
    contents
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
    assert!(function.locals.is_empty());
    assert_eq!(
        function.blocks[0].terminator,
        Terminator::Return(Operand::Int(42))
    );
    assert_eq!(
        program.to_string(),
        "function main(): int {\nblock0:\n    return 42\n}\n"
    );
}

#[test]
fn lowers_return_add_42_to_mir_arithmetic() {
    let program = lower(
        r#"function main(): int
{
    return 40 + 2;
}
"#,
    );

    let function = &program.functions[0];
    assert_eq!(
        function.locals,
        vec![Local {
            id: LocalId(0),
            name: "_tmp0".to_string(),
            ty: Type::Int,
            writable: false,
            synthetic: true,
        }]
    );
    assert_eq!(
        function.blocks[0].statements,
        vec![Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Binary {
                op: BinaryOp::Add,
                left: Operand::Int(40),
                right: Operand::Int(2),
            },
        }]
    );
    assert_eq!(
        function.blocks[0].terminator,
        Terminator::Return(Operand::Local(LocalId(0)))
    );
    assert_eq!(
        program.to_string(),
        "function main(): int {\nlocals:\n    local0 temp _tmp0: int\nblock0:\n    local0 = 40 + 2\n    return local0\n}\n"
    );
}

#[test]
fn lowers_readonly_int_local_to_slot_assignment() {
    let program = lower(
        r#"function main(): int
{
    let $base = 40;

    return $base;
}
"#,
    );

    let function = &program.functions[0];
    assert_eq!(function.locals.len(), 1);
    assert_eq!(function.locals[0].name, "base");
    assert!(!function.locals[0].writable);
    assert!(!function.locals[0].synthetic);
    assert_eq!(
        function.blocks[0].statements,
        vec![Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Use(Operand::Int(40)),
        }]
    );
    assert_eq!(
        function.blocks[0].terminator,
        Terminator::Return(Operand::Local(LocalId(0)))
    );
}

#[test]
fn lowers_typed_readonly_int_local_to_slot_assignment() {
    let program = lower(
        r#"function main(): int
{
    int $base = 40;

    return $base;
}
"#,
    );

    assert_eq!(program.functions[0].locals[0].ty, Type::Int);
    assert_eq!(program.functions[0].locals[0].name, "base");
}

#[test]
fn lowers_writable_int_local_to_writable_slot_assignment() {
    let program = lower(
        r#"function main(): int
{
    let writable $value = 40;

    return $value;
}
"#,
    );

    let function = &program.functions[0];
    assert_eq!(function.locals.len(), 1);
    assert_eq!(function.locals[0].name, "value");
    assert!(function.locals[0].writable);
    assert!(!function.locals[0].synthetic);
    assert_eq!(
        function.blocks[0].statements,
        vec![Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Use(Operand::Int(40)),
        }]
    );
}

#[test]
fn lowers_typed_writable_int_local_to_writable_slot_assignment() {
    let program = lower(
        r#"function main(): int
{
    writable int $value = 40;

    return $value;
}
"#,
    );

    assert!(program.functions[0].locals[0].writable);
    assert_eq!(program.functions[0].locals[0].ty, Type::Int);
}

#[test]
fn lowers_plain_assignment_to_local_assignment() {
    let program = lower(
        r#"function main(): int
{
    let writable $value = 0;

    $value = 42;

    return $value;
}
"#,
    );

    assert_eq!(
        program.functions[0].blocks[0].statements,
        vec![
            Statement::AssignLocal {
                target: LocalId(0),
                value: Rvalue::Use(Operand::Int(0)),
            },
            Statement::AssignLocal {
                target: LocalId(0),
                value: Rvalue::Use(Operand::Int(42)),
            },
        ]
    );
}

#[test]
fn lowers_add_assign_to_read_add_write() {
    let program = lower(
        r#"function main(): int
{
    let writable $value = 40;

    $value += 2;

    return $value;
}
"#,
    );

    assert_eq!(
        program.functions[0].blocks[0].statements[1],
        Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Binary {
                op: BinaryOp::Add,
                left: Operand::Local(LocalId(0)),
                right: Operand::Int(2),
            },
        }
    );
}

#[test]
fn lowers_sub_assign_to_read_subtract_write() {
    let program = lower(
        r#"function main(): int
{
    let writable $value = 43;

    $value -= 1;

    return $value;
}
"#,
    );

    assert_eq!(
        program.functions[0].blocks[0].statements[1],
        Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Binary {
                op: BinaryOp::Subtract,
                left: Operand::Local(LocalId(0)),
                right: Operand::Int(1),
            },
        }
    );
}

#[test]
fn lowers_post_and_pre_increment_equivalently() {
    let post = lower(
        r#"function main(): int
{
    let writable $value = 41;

    $value++;

    return $value;
}
"#,
    );
    let pre = lower(
        r#"function main(): int
{
    let writable $value = 41;

    ++$value;

    return $value;
}
"#,
    );

    assert_eq!(post, pre);
    assert_eq!(
        post.functions[0].blocks[0].statements[1],
        Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Binary {
                op: BinaryOp::Add,
                left: Operand::Local(LocalId(0)),
                right: Operand::Int(1),
            },
        }
    );
}

#[test]
fn lowers_post_and_pre_decrement_equivalently() {
    let post = lower(
        r#"function main(): int
{
    let writable $value = 43;

    $value--;

    return $value;
}
"#,
    );
    let pre = lower(
        r#"function main(): int
{
    let writable $value = 43;

    --$value;

    return $value;
}
"#,
    );

    assert_eq!(post, pre);
    assert_eq!(
        post.functions[0].blocks[0].statements[1],
        Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Binary {
                op: BinaryOp::Subtract,
                left: Operand::Local(LocalId(0)),
                right: Operand::Int(1),
            },
        }
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
fn interprets_return_addition_42() {
    let output = interpret(
        r#"function main(): int
{
    return 40 + 2;
}
"#,
    );

    assert_eq!(output.exit_status, 42);
}

#[test]
fn interprets_return_subtraction_42() {
    let output = interpret(
        r#"function main(): int
{
    return 50 - 8;
}
"#,
    );

    assert_eq!(output.exit_status, 42);
}

#[test]
fn interprets_return_multiplication_42() {
    let output = interpret(
        r#"function main(): int
{
    return 6 * 7;
}
"#,
    );

    assert_eq!(output.exit_status, 42);
}

#[test]
fn interprets_int_local_return_42() {
    let output = interpret(
        r#"function main(): int
{
    let $base = 40;

    return $base + 2;
}
"#,
    );

    assert_eq!(output.exit_status, 42);
}

#[test]
fn interprets_writable_assignment_42() {
    let output = interpret(
        r#"function main(): int
{
    let writable $value = 0;

    $value = 42;

    return $value;
}
"#,
    );

    assert_eq!(output.exit_status, 42);
}

#[test]
fn interprets_add_assign_42() {
    let output = interpret(
        r#"function main(): int
{
    let writable $value = 40;

    $value += 2;

    return $value;
}
"#,
    );

    assert_eq!(output.exit_status, 42);
}

#[test]
fn interprets_sub_assign_42() {
    let output = interpret(
        r#"function main(): int
{
    let writable $value = 43;

    $value -= 1;

    return $value;
}
"#,
    );

    assert_eq!(output.exit_status, 42);
}

#[test]
fn interprets_increment_42() {
    let output = interpret(
        r#"function main(): int
{
    let writable $value = 41;

    $value++;

    return $value;
}
"#,
    );

    assert_eq!(output.exit_status, 42);
}

#[test]
fn interprets_decrement_42() {
    let output = interpret(
        r#"function main(): int
{
    let writable $value = 43;

    --$value;

    return $value;
}
"#,
    );

    assert_eq!(output.exit_status, 42);
}

#[test]
fn interprets_main_void_with_int_local_statements() {
    let output = interpret(
        r#"function main(): void
{
    let writable $value = 0;

    $value = 42;

    return;
}
"#,
    );

    assert_eq!(output.exit_status, 0);
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
fn interpreter_reports_arithmetic_overflow() {
    let program = Program {
        functions: vec![Function {
            id: FunctionId(0),
            name: "main".to_string(),
            return_type: ReturnType::Int,
            locals: vec![Local {
                id: LocalId(0),
                name: "_tmp0".to_string(),
                ty: Type::Int,
                writable: false,
                synthetic: true,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                statements: vec![Statement::AssignLocal {
                    target: LocalId(0),
                    value: Rvalue::Binary {
                        op: BinaryOp::Add,
                        left: Operand::Int(i64::MAX),
                        right: Operand::Int(1),
                    },
                }],
                terminator: Terminator::Return(Operand::Local(LocalId(0))),
            }],
            entry_block: BlockId(0),
        }],
        entry: FunctionId(0),
    };

    let error = doriac::mir_interpreter::interpret(&program).expect_err("overflow should fail");
    assert!(error
        .message
        .contains("MIR interpreter integer overflow during addition"));
}

#[test]
fn interpreter_rejects_main_int_exit_status_126() {
    let program = lower(
        r#"function main(): int
{
    return 126;
}
"#,
    );

    let error =
        doriac::mir_interpreter::interpret(&program).expect_err("main exit status 126 should fail");
    assert!(error.message.contains("0..125"));
}

#[test]
fn debug_target_surfaces_interpreter_exit_status_diagnostic() {
    let diagnostics = doriac::compile_source(
        "test.doria",
        r#"function main(): int
{
    return 126;
}
"#,
        BackendTarget::Debug,
    )
    .expect_err("debug backend should preserve interpreter diagnostics");

    assert_eq!(diagnostics[0].code, "M1102");
    assert!(diagnostics[0].message.contains("0..125"));
}

#[test]
fn debug_target_emits_interpreter_artifact() {
    assert_eq!(
        debug_contents(
            r#"function main(): void
{
    echo "Hello Doria!";
}
"#,
        ),
        "exit_status: 0\nstdout: Hello Doria!\n"
    );
}

#[test]
fn debug_backend_emit_handles_stage_11b_hir_directly() {
    let hir = doriac::lower_source(
        "test.doria",
        r#"function main(): int
{
    return 40 + 2;
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
fn debug_target_handles_stage_11b_examples() {
    for source in [
        include_str!("../../../examples/debug/main_return_add_42.doria"),
        include_str!("../../../examples/debug/main_int_local_42.doria"),
        include_str!("../../../examples/debug/main_writable_int_assignment_42.doria"),
        include_str!("../../../examples/debug/main_writable_int_increment_42.doria"),
    ] {
        assert_eq!(debug_contents(source), "exit_status: 42\nstdout:\n");
    }

    assert_eq!(
        debug_contents(include_str!(
            "../../../examples/debug/main_void_int_statements.doria"
        )),
        "exit_status: 0\nstdout:\n"
    );
}

#[test]
fn rejects_unsupported_division_as_mir_coverage() {
    let diagnostics = unsupported(
        r#"function main(): int
{
    return 84 / 2;
}
"#,
    );

    assert_stage_11b_unsupported(&diagnostics, "division and modulo");
}

#[test]
fn rejects_unsupported_comparison_as_mir_coverage() {
    let diagnostics = unsupported(
        r#"function main(): void
{
    let $ok = 40 < 42;
}
"#,
    );

    assert_stage_11b_unsupported(&diagnostics, "comparisons");
}

#[test]
fn rejects_unsupported_if_statement_as_mir_coverage() {
    let diagnostics = unsupported(
        r#"function main(): int
{
    if (true) {
        return 42;
    }
    return 0;
}
"#,
    );

    assert_stage_11b_unsupported(&diagnostics, "if statements");
}

#[test]
fn rejects_unsupported_string_local_as_mir_coverage() {
    let diagnostics = unsupported(
        r#"function main(): void
{
    let $message = "Hello Doria!";
    echo $message;
}
"#,
    );

    assert_stage_11b_unsupported(&diagnostics, "string locals");
}

#[test]
fn rejects_unsupported_string_concat_echo_as_mir_coverage() {
    let diagnostics = unsupported(
        r#"function main(): void
{
    echo "Hello " . "Doria!";
}
"#,
    );

    assert_stage_11b_unsupported(&diagnostics, "string concatenation");
}

#[test]
fn rejects_unsupported_helper_function_as_mir_coverage() {
    let diagnostics = unsupported(
        r#"function helper(): int
{
    return 1;
}

function main(): int
{
    return 42;
}
"#,
    );

    assert_stage_11b_unsupported(&diagnostics, "no helper functions");
}

#[test]
fn mirrors_native_smoke_exit_for_stage_11b_int_shapes_without_linker() {
    for (source, expected) in [
        (
            r#"function main(): int
{
    return 40 + 2;
}
"#,
            42,
        ),
        (
            r#"function main(): int
{
    let $base = 40;

    return $base + 2;
}
"#,
            42,
        ),
        (
            r#"function main(): int
{
    let writable $value = 40;

    $value += 2;

    return $value;
}
"#,
            42,
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

#[test]
fn mirrors_native_smoke_exit_for_stage_11a_literal_shapes_without_linker() {
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
