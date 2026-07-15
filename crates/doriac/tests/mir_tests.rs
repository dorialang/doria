use doriac::backend::{BackendOutput, BackendTarget};
use doriac::class_layout::compute_class_layout;
use doriac::mir::{
    BasicBlock, BlockId, BoolBinaryOp as ConditionBinaryOp, BoolExpression as Condition,
    ClassExpression, CompareOp, Function, FunctionId, IntegerBinaryOp, IntegerExpression, Local,
    LocalId, Operand, Program, ReturnType, Rvalue, ScalarType, ScalarValue, Statement,
    StringExpression, Terminator, Type, ValueExpression,
};
use doriac::numeric::{IntegerType, IntegerValue};

const DEFAULT_INT: IntegerType = IntegerType::Int64;

fn int_value(value: i64) -> IntegerValue {
    IntegerValue::from_i128(DEFAULT_INT, value as i128).expect("test int value must fit")
}

fn int_operand(value: i64) -> Operand {
    Operand::Scalar(ScalarValue::Integer(int_value(value)))
}

fn integer_expression(operand: Operand) -> IntegerExpression {
    IntegerExpression::use_operand(DEFAULT_INT, operand)
}

fn int_constant(value: i64) -> IntegerExpression {
    IntegerExpression::constant(int_value(value))
}

fn int_rvalue(value: i64) -> Rvalue {
    Rvalue::Value(ValueExpression::Integer(int_constant(value)))
}

fn local_integer_expression(id: LocalId) -> IntegerExpression {
    integer_expression(Operand::Local(id))
}

fn int_expression_value(expression: IntegerExpression) -> ValueExpression {
    ValueExpression::Integer(expression)
}

fn int_return(expression: IntegerExpression) -> Terminator {
    Terminator::Return(Rvalue::Value(int_expression_value(expression)))
}

fn bool_condition(value: bool) -> Condition {
    Condition::Use {
        operand: Operand::Scalar(ScalarValue::Bool(value)),
    }
}

fn int_compare(op: CompareOp, left: IntegerExpression, right: IntegerExpression) -> Condition {
    Condition::Compare {
        op,
        left: Box::new(int_expression_value(left)),
        right: Box::new(int_expression_value(right)),
    }
}

fn lower(source: &str) -> doriac::mir::Program {
    doriac::lower_source_to_mir("test.doria", source).expect("source should lower to MIR")
}

fn interpret(source: &str) -> doriac::mir_interpreter::InterpreterOutput {
    let program = lower(source);
    doriac::mir_interpreter::interpret(&program).expect("MIR should interpret")
}

fn lower_object(source: &str) -> Vec<u8> {
    let program = lower(source);
    doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("MIR should lower to a Cranelift object")
}

fn unsupported_after_parsing(source: &str) -> Vec<doriac::diagnostics::Diagnostic> {
    let ast = doriac::parse_source("test.doria", source).expect("source should parse");
    let hir = doriac::lowering::lower_program(&ast);
    doriac::mir_lowering::lower_program(&hir)
        .expect_err("HIR should be outside native compilation coverage")
}

fn assert_stage_11g_unsupported(diagnostics: &[doriac::diagnostics::Diagnostic], detail: &str) {
    assert_eq!(diagnostics[0].code, "M1101");
    assert!(
        !diagnostics[0].message.contains("Stage "),
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

fn conditional_program(condition: Condition, then_status: i64, else_status: i64) -> Program {
    Program {
        classes: vec![],
        functions: vec![Function {
            id: FunctionId(0),
            name: "main".to_string(),
            params: Vec::new(),
            return_type: ReturnType::Value(Type::Scalar(ScalarType::Integer(DEFAULT_INT))),
            locals: vec![Local {
                id: LocalId(0),
                name: "unassigned".to_string(),
                ty: Type::Scalar(ScalarType::Integer(DEFAULT_INT)),
                writable: false,
                synthetic: true,
                owned: false,
            }],
            blocks: vec![
                BasicBlock {
                    id: BlockId(0),
                    statements: Vec::new(),
                    terminator: Terminator::Branch {
                        condition,
                        then_block: BlockId(1),
                        else_block: BlockId(2),
                    },
                },
                BasicBlock {
                    id: BlockId(1),
                    statements: Vec::new(),
                    terminator: int_return(int_constant(then_status)),
                },
                BasicBlock {
                    id: BlockId(2),
                    statements: Vec::new(),
                    terminator: int_return(int_constant(else_status)),
                },
            ],
            entry_block: BlockId(0),
        }],
        entry: FunctionId(0),
    }
}

fn condition_that_reads_unassigned_local() -> Condition {
    Condition::Compare {
        op: CompareOp::Equal,
        left: Box::new(int_expression_value(local_integer_expression(LocalId(0)))),
        right: Box::new(int_expression_value(int_constant(0))),
    }
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
    assert_eq!(
        function.return_type,
        ReturnType::Value(Type::Scalar(ScalarType::Integer(DEFAULT_INT)))
    );
    assert!(function.locals.is_empty());
    assert_eq!(function.blocks[0].terminator, int_return(int_constant(42)));
    assert_eq!(
        program.to_string(),
        "function main(): int {\nblock0:\n    return 42: int\n}\n"
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
    assert!(function.locals.is_empty());
    assert!(function.blocks[0].statements.is_empty());
    assert_eq!(
        function.blocks[0].terminator,
        Terminator::Return(Rvalue::Value(ValueExpression::Integer(
            IntegerExpression::Binary {
                ty: DEFAULT_INT,
                op: IntegerBinaryOp::Add,
                left: Box::new(int_constant(40)),
                right: Box::new(int_constant(2)),
            }
        )))
    );
    assert_eq!(
        program.to_string(),
        "function main(): int {\nblock0:\n    return (40: int + 2: int): int\n}\n"
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
            value: int_rvalue(40),
        }]
    );
    assert_eq!(
        function.blocks[0].terminator,
        int_return(local_integer_expression(LocalId(0)))
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

    assert_eq!(
        program.functions[0].locals[0].ty,
        Type::Scalar(ScalarType::Integer(DEFAULT_INT))
    );
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
            value: int_rvalue(40),
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
    assert_eq!(
        program.functions[0].locals[0].ty,
        Type::Scalar(ScalarType::Integer(DEFAULT_INT))
    );
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
                value: int_rvalue(0),
            },
            Statement::AssignLocal {
                target: LocalId(0),
                value: int_rvalue(42),
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
            value: Rvalue::Value(ValueExpression::Integer(IntegerExpression::Binary {
                ty: DEFAULT_INT,
                op: IntegerBinaryOp::Add,
                left: Box::new(integer_expression(Operand::Local(LocalId(0)))),
                right: Box::new(integer_expression(int_operand(2))),
            })),
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
            value: Rvalue::Value(ValueExpression::Integer(IntegerExpression::Binary {
                ty: DEFAULT_INT,
                op: IntegerBinaryOp::Subtract,
                left: Box::new(integer_expression(Operand::Local(LocalId(0)))),
                right: Box::new(integer_expression(int_operand(1))),
            })),
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
            value: Rvalue::Value(ValueExpression::Integer(IntegerExpression::Binary {
                ty: DEFAULT_INT,
                op: IntegerBinaryOp::Add,
                left: Box::new(integer_expression(Operand::Local(LocalId(0)))),
                right: Box::new(integer_expression(int_operand(1))),
            })),
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
            value: Rvalue::Value(ValueExpression::Integer(IntegerExpression::Binary {
                ty: DEFAULT_INT,
                op: IntegerBinaryOp::Subtract,
                left: Box::new(integer_expression(Operand::Local(LocalId(0)))),
                right: Box::new(integer_expression(int_operand(1))),
            })),
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
fn interpreter_reports_arithmetic_overflow_as_runtime_panic() {
    let program = Program {
        classes: vec![],
        functions: vec![Function {
            id: FunctionId(0),
            name: "main".to_string(),
            params: Vec::new(),
            return_type: ReturnType::Value(Type::Scalar(ScalarType::Integer(DEFAULT_INT))),
            locals: vec![Local {
                id: LocalId(0),
                name: "_tmp0".to_string(),
                ty: Type::Scalar(ScalarType::Integer(DEFAULT_INT)),
                writable: false,
                synthetic: true,
                owned: false,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                statements: vec![Statement::AssignLocal {
                    target: LocalId(0),
                    value: Rvalue::Value(ValueExpression::Integer(IntegerExpression::Binary {
                        ty: DEFAULT_INT,
                        op: IntegerBinaryOp::Add,
                        left: Box::new(integer_expression(int_operand(i64::MAX))),
                        right: Box::new(integer_expression(int_operand(1))),
                    })),
                }],
                terminator: int_return(local_integer_expression(LocalId(0))),
            }],
            entry_block: BlockId(0),
        }],
        entry: FunctionId(0),
    };

    let output = doriac::mir_interpreter::interpret(&program).expect("overflow should panic");
    assert_eq!(output.exit_status, 101);
    assert!(output
        .stderr
        .starts_with(b"Panic: integer overflow during addition\n"));
}

#[test]
fn interpreter_panics_for_main_int_exit_status_126() {
    let program = lower(
        r#"function main(): int
{
    return 126;
}
"#,
    );

    let output = doriac::mir_interpreter::interpret(&program)
        .expect("main exit status 126 should produce a runtime outcome");
    assert_eq!(output.exit_status, 101);
    assert!(output
        .stderr
        .starts_with(b"Panic: main returned process status outside 0..125\n"));
}

#[test]
fn debug_target_renders_interpreter_process_status_panic() {
    let output = doriac::compile_source(
        "test.doria",
        r#"function main(): int
{
    return 126;
}
"#,
        BackendTarget::Debug,
    )
    .expect("debug backend should render the panic outcome");
    let BackendOutput::Text { contents, .. } = output else {
        panic!("debug backend should emit text");
    };
    assert!(contents.starts_with("exit_status: 101\nstdout:\nstderr: Panic: "));
    assert!(contents.contains("main returned process status outside 0..125"));
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
fn debug_backend_emit_handles_stage_11c_hir_directly() {
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
fn lowers_and_interprets_integer_division() {
    let program = lower(
        r#"function main(): int
{
    return 84 / 2;
}
"#,
    );

    assert!(matches!(
        program.functions[0].blocks[0].terminator,
        Terminator::Return(Rvalue::Value(ValueExpression::Integer(
            IntegerExpression::Binary {
                op: IntegerBinaryOp::Divide,
                ..
            }
        )))
    ));
    assert_eq!(
        doriac::mir_interpreter::interpret(&program)
            .expect("division should interpret")
            .exit_status,
        42
    );
}

#[test]
fn lowers_comparison_result_as_runtime_bool_value() {
    let program = lower(
        r#"function main(): void
{
    let $ok = 40 < 42;
}
"#,
    );
    assert_eq!(
        program.functions[0].locals[0].ty,
        Type::Scalar(ScalarType::Bool)
    );
}

#[test]
fn lowers_readonly_string_local_and_echo() {
    let program = lower(
        r#"function main(): void
{
    let $message = "Hello Doria!";
    echo $message;
}
"#,
    );

    let function = &program.functions[program.entry.0];
    assert_eq!(function.locals[0].ty, Type::String);
    assert!(!function.locals[0].writable);
    assert_eq!(
        function.blocks[0].statements,
        vec![
            Statement::AssignLocal {
                target: LocalId(0),
                value: Rvalue::String(StringExpression::Literal("Hello Doria!".to_string(),)),
            },
            Statement::EchoString(StringExpression::Local(LocalId(0))),
        ]
    );
}

#[test]
fn lowers_string_literal_concat_echo() {
    let program = lower(
        r#"function main(): void
{
    echo "Hello " . "Doria!";
}
"#,
    );

    assert_eq!(
        program.functions[program.entry.0].blocks[0].statements,
        vec![Statement::EchoString(StringExpression::Concat(vec![
            StringExpression::Literal("Hello ".to_string()),
            StringExpression::Literal("Doria!".to_string()),
        ]))]
    );
}

#[test]
fn lowers_multiple_top_level_functions_in_declaration_order() {
    let program = lower(
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

    assert_eq!(program.functions.len(), 2);
    assert_eq!(program.functions[0].id, FunctionId(0));
    assert_eq!(program.functions[0].name, "helper");
    assert_eq!(program.functions[1].id, FunctionId(1));
    assert_eq!(program.functions[1].name, "main");
    assert_eq!(program.entry, FunctionId(1));
}

#[test]
fn lowers_if_condition_to_branch_terminator() {
    let program = lower(
        r#"function main(): int
{
    if (40 + 2 == 42) {
        return 42;
    }

    return 0;
}
"#,
    );

    assert_eq!(
        program.functions[0].blocks[0].terminator,
        Terminator::Branch {
            condition: Condition::Compare {
                op: CompareOp::Equal,
                left: Box::new(ValueExpression::Integer(IntegerExpression::Binary {
                    ty: DEFAULT_INT,
                    op: IntegerBinaryOp::Add,
                    left: Box::new(int_constant(40)),
                    right: Box::new(int_constant(2)),
                })),
                right: Box::new(int_expression_value(int_constant(42))),
            },
            then_block: BlockId(1),
            else_block: BlockId(2),
        }
    );
}

#[test]
fn lowers_if_without_else_through_a_continuation_block() {
    let program = lower(
        r#"function main(): int
{
    let writable $value = 0;

    if (true) {
        $value = 42;
    }

    return $value;
}
"#,
    );
    let blocks = &program.functions[0].blocks;

    assert_eq!(blocks.len(), 4);
    assert_eq!(blocks[1].terminator, Terminator::Jump(BlockId(3)));
    assert_eq!(blocks[2].terminator, Terminator::Jump(BlockId(3)));
    assert_eq!(
        blocks[3].terminator,
        int_return(local_integer_expression(LocalId(0)))
    );
}

#[test]
fn lowers_if_else_to_distinct_return_blocks() {
    let program = lower(
        r#"function main(): int
{
    if (true) {
        return 42;
    } else {
        return 0;
    }
}
"#,
    );
    let blocks = &program.functions[0].blocks;

    assert_eq!(blocks.len(), 3);
    assert_eq!(blocks[1].terminator, int_return(int_constant(42)));
    assert_eq!(blocks[2].terminator, int_return(int_constant(0)));
}

#[test]
fn lowers_else_if_chain_to_nested_branch_blocks() {
    let program = lower(include_str!(
        "../../../examples/debug/main_else_if_42.doria"
    ));
    let blocks = &program.functions[0].blocks;

    assert!(matches!(blocks[0].terminator, Terminator::Branch { .. }));
    assert!(matches!(blocks[2].terminator, Terminator::Branch { .. }));
    assert_eq!(blocks[3].terminator, int_return(int_constant(42)));
}

#[test]
fn lowers_echo_and_assignment_inside_if_branches() {
    let echo = lower(include_str!(
        "../../../examples/debug/main_if_fallthrough_echo.doria"
    ));
    assert_eq!(
        echo.functions[0].blocks[1].statements,
        vec![Statement::EchoStringLiteral("Hello ".to_string())]
    );

    let assignment = lower(include_str!(
        "../../../examples/debug/main_if_assignment_42.doria"
    ));
    assert_eq!(
        assignment.functions[0].blocks[1].statements,
        vec![Statement::AssignLocal {
            target: LocalId(0),
            value: int_rvalue(42),
        }]
    );
}

#[test]
fn lowers_all_stage_11c_integer_comparisons() {
    for (operator, expected) in [
        ("==", CompareOp::Equal),
        ("!=", CompareOp::NotEqual),
        ("<", CompareOp::Less),
        ("<=", CompareOp::LessEqual),
        (">", CompareOp::Greater),
        (">=", CompareOp::GreaterEqual),
    ] {
        let source = format!(
            "function main(): int\n{{\n    if (1 {operator} 1) {{\n        return 42;\n    }}\n\n    return 0;\n}}\n"
        );
        let program = lower(&source);
        let Terminator::Branch { condition, .. } = &program.functions[0].blocks[0].terminator
        else {
            panic!("comparison should lower to a branch");
        };
        assert!(matches!(
            condition,
            Condition::Compare { op, .. } if *op == expected
        ));
    }
}

#[test]
fn lowers_word_and_symbol_condition_operators_equivalently() {
    for (word, symbol) in [("and", "&&"), ("or", "||"), ("not", "!")] {
        let word_source = if word == "not" {
            format!(
                "function main(): int\n{{\n    if ({word} false) {{\n        return 42;\n    }}\n\n    return 0;\n}}\n"
            )
        } else {
            format!(
                "function main(): int\n{{\n    if (true {word} true) {{\n        return 42;\n    }}\n\n    return 0;\n}}\n"
            )
        };
        let symbol_source = if symbol == "!" {
            "function main(): int\n{\n    if (!false) {\n        return 42;\n    }\n\n    return 0;\n}\n".to_string()
        } else {
            format!(
                "function main(): int\n{{\n    if (true {symbol} true) {{\n        return 42;\n    }}\n\n    return 0;\n}}\n"
            )
        };

        assert_eq!(lower(&word_source), lower(&symbol_source));
    }

    let xor = lower(
        r#"function main(): int
{
    if (true xor false) {
        return 42;
    }

    return 0;
}
"#,
    );
    let Terminator::Branch { condition, .. } = &xor.functions[0].blocks[0].terminator else {
        panic!("xor should lower to a branch");
    };
    assert!(matches!(
        condition,
        Condition::Binary {
            op: ConditionBinaryOp::Xor,
            ..
        }
    ));
}

#[test]
fn interprets_stage_11c_if_shapes() {
    for source in [
        include_str!("../../../examples/debug/main_if_return_42.doria"),
        include_str!("../../../examples/debug/main_if_else_42.doria"),
        include_str!("../../../examples/debug/main_if_assignment_42.doria"),
        include_str!("../../../examples/debug/main_else_if_42.doria"),
        include_str!("../../../examples/debug/main_condition_words_42.doria"),
    ] {
        assert_eq!(interpret(source).exit_status, 42);
    }

    assert_eq!(
        interpret(
            r#"function main(): int
{
    if (false) {
        return 1;
    }

    return 42;
}
"#,
        )
        .exit_status,
        42
    );
    assert_eq!(
        interpret(
            r#"function main(): int
{
    if (false) {
        return 0;
    } else {
        return 42;
    }
}
"#,
        )
        .exit_status,
        42
    );
}

#[test]
fn interprets_nested_if_and_preserves_branch_local_scope() {
    assert_eq!(
        interpret(
            r#"function main(): int
{
    if (true) {
        if (true) {
            return 42;
        }
    }

    return 0;
}
"#,
        )
        .exit_status,
        42
    );

    assert_eq!(
        interpret(
            r#"function main(): int
{
    let $value = 1;

    if (true) {
        let $value = 42;
    }

    return $value;
}
"#,
        )
        .exit_status,
        1
    );
}

#[test]
fn interprets_if_echo_across_blocks() {
    let output = interpret(include_str!(
        "../../../examples/debug/main_if_fallthrough_echo.doria"
    ));

    assert_eq!(output.exit_status, 0);
    assert_eq!(output.stdout, b"Hello Doria!");
}

#[test]
fn interprets_all_integer_comparisons() {
    for condition in ["42 == 42", "42 != 0", "1 < 2", "2 <= 2", "3 > 2", "3 >= 3"] {
        let source = format!(
            "function main(): int\n{{\n    if ({condition}) {{\n        return 42;\n    }}\n\n    return 0;\n}}\n"
        );
        assert_eq!(interpret(&source).exit_status, 42, "condition: {condition}");
    }
}

#[test]
fn interpreter_short_circuits_and_and_or() {
    let and = conditional_program(
        Condition::Binary {
            op: ConditionBinaryOp::And,
            left: Box::new(bool_condition(false)),
            right: Box::new(condition_that_reads_unassigned_local()),
        },
        0,
        42,
    );
    let or = conditional_program(
        Condition::Binary {
            op: ConditionBinaryOp::Or,
            left: Box::new(bool_condition(true)),
            right: Box::new(condition_that_reads_unassigned_local()),
        },
        42,
        0,
    );

    assert_eq!(
        doriac::mir_interpreter::interpret(&and)
            .expect("and should skip its right operand")
            .exit_status,
        42
    );
    assert_eq!(
        doriac::mir_interpreter::interpret(&or)
            .expect("or should skip its right operand")
            .exit_status,
        42
    );
}

#[test]
fn interpreter_evaluates_both_xor_operands() {
    let program = conditional_program(
        Condition::Binary {
            op: ConditionBinaryOp::Xor,
            left: Box::new(bool_condition(true)),
            right: Box::new(condition_that_reads_unassigned_local()),
        },
        42,
        0,
    );
    let error = doriac::mir_interpreter::interpret(&program)
        .expect_err("xor must evaluate its right operand");

    assert!(error.message.contains("read before assignment"));
}

#[test]
fn interpreter_computes_xor_truth_table() {
    for (left, right, expected) in [
        (false, false, 0),
        (false, true, 42),
        (true, false, 42),
        (true, true, 0),
    ] {
        let source = format!(
            "function main(): int\n{{\n    if ({left} xor {right}) {{\n        return 42;\n    }}\n\n    return 0;\n}}\n"
        );
        assert_eq!(interpret(&source).exit_status, expected);
    }
}

#[test]
fn interpreter_inverts_not_condition() {
    assert_eq!(
        interpret(
            r#"function main(): int
{
    if (not false) {
        return 42;
    }

    return 0;
}
"#,
        )
        .exit_status,
        42
    );
}

#[test]
fn interpreter_preserves_void_fallthrough_after_final_else_if() {
    let output = interpret(
        r#"function main(): void
{
    if (false) {
        return;
    } else if (false) {
        return;
    }
}
"#,
    );

    assert_eq!(output.exit_status, 0);
    assert!(output.stdout.is_empty());
}

#[test]
fn explicitly_limited_interpreter_stops_repeated_mir_state_cycles() {
    let program = Program {
        classes: vec![],
        functions: vec![Function {
            id: FunctionId(0),
            name: "main".to_string(),
            params: Vec::new(),
            return_type: ReturnType::Void,
            locals: Vec::new(),
            blocks: vec![BasicBlock {
                id: BlockId(0),
                statements: Vec::new(),
                terminator: Terminator::Jump(BlockId(0)),
            }],
            entry_block: BlockId(0),
        }],
        entry: FunctionId(0),
    };

    let error = doriac::mir_interpreter::interpret_with_limits(
        &program,
        doriac::mir_interpreter::InterpreterLimits {
            max_executed_blocks: Some(10),
            max_call_frames: None,
        },
    )
    .expect_err("explicit test limit should stop cyclic MIR");
    assert!(error.message.contains("explicit test execution limit"));
}

#[test]
fn debug_target_handles_stage_11c_examples() {
    for source in [
        include_str!("../../../examples/debug/main_if_return_42.doria"),
        include_str!("../../../examples/debug/main_if_else_42.doria"),
        include_str!("../../../examples/debug/main_if_assignment_42.doria"),
        include_str!("../../../examples/debug/main_else_if_42.doria"),
        include_str!("../../../examples/debug/main_condition_words_42.doria"),
    ] {
        assert_eq!(debug_contents(source), "exit_status: 42\nstdout:\n");
    }

    assert_eq!(
        debug_contents(include_str!(
            "../../../examples/debug/main_if_fallthrough_echo.doria"
        )),
        "exit_status: 0\nstdout: Hello Doria!\n"
    );
}

#[test]
fn rejects_truthiness_and_calls_as_stage_11c_conditions() {
    let truthiness = unsupported_after_parsing(
        r#"function main(): int
{
    if (42) {
        return 1;
    }

    return 0;
}
"#,
    );
    assert_stage_11g_unsupported(&truthiness, "truthiness");

    let call = unsupported_after_parsing(
        r#"function main(): int
{
    if (isReady()) {
        return 1;
    }

    return 0;
}
"#,
    );
    assert_stage_11g_unsupported(&call, "unknown top-level function");
}

#[test]
fn lowers_while_to_header_body_and_exit_blocks() {
    let program = lower(include_str!(
        "../../../examples/debug/main_while_count_42.doria"
    ));
    let blocks = &program.functions[0].blocks;

    assert_eq!(blocks.len(), 4);
    assert_eq!(blocks[0].terminator, Terminator::Jump(BlockId(1)));
    assert_eq!(
        blocks[1].terminator,
        Terminator::Branch {
            condition: int_compare(
                CompareOp::Less,
                local_integer_expression(LocalId(0)),
                int_constant(42),
            ),
            then_block: BlockId(2),
            else_block: BlockId(3),
        }
    );
    assert_eq!(blocks[2].terminator, Terminator::Jump(BlockId(1)));
    assert_eq!(
        blocks[3].terminator,
        int_return(local_integer_expression(LocalId(0)))
    );
}

#[test]
fn lowers_assignment_and_echo_inside_while() {
    let count = lower(include_str!(
        "../../../examples/debug/main_while_count_42.doria"
    ));
    assert_eq!(
        count.functions[0].blocks[2].statements,
        vec![Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Value(ValueExpression::Integer(IntegerExpression::Binary {
                ty: DEFAULT_INT,
                op: IntegerBinaryOp::Add,
                left: Box::new(integer_expression(Operand::Local(LocalId(0)))),
                right: Box::new(integer_expression(int_operand(1))),
            })),
        }]
    );

    let echo = lower(include_str!(
        "../../../examples/debug/main_while_echo_xxx.doria"
    ));
    assert_eq!(
        echo.functions[0].blocks[2].statements[0],
        Statement::EchoStringLiteral("x".to_string())
    );
}

#[test]
fn lowers_break_to_while_exit_and_continue_to_header() {
    let break_program = lower(include_str!(
        "../../../examples/debug/main_while_break_42.doria"
    ));
    assert_eq!(
        break_program.functions[0].blocks[4].terminator,
        Terminator::Jump(BlockId(3))
    );

    let continue_program = lower(include_str!(
        "../../../examples/debug/main_while_continue_6.doria"
    ));
    assert_eq!(
        continue_program.functions[0].blocks[4].terminator,
        Terminator::Jump(BlockId(1))
    );
}

#[test]
fn nested_while_uses_innermost_loop_targets() {
    let program = lower(
        r#"function main(): int
{
    while (true) {
        while (true) {
            break;
        }

        continue;
    }

    return 0;
}
"#,
    );
    let blocks = &program.functions[0].blocks;

    assert_eq!(blocks[5].terminator, Terminator::Jump(BlockId(6)));
    assert_eq!(blocks[6].terminator, Terminator::Jump(BlockId(1)));
}

#[test]
fn lowers_return_inside_while_as_return_terminator() {
    let program = lower(include_str!(
        "../../../examples/debug/main_while_return_42.doria"
    ));

    assert!(program.functions[0]
        .blocks
        .iter()
        .any(|block| { block.terminator == int_return(local_integer_expression(LocalId(0))) }));
}

#[test]
fn interprets_stage_11d_while_examples() {
    for (source, expected) in [
        (
            include_str!("../../../examples/debug/main_while_count_42.doria"),
            42,
        ),
        (
            include_str!("../../../examples/debug/main_while_break_42.doria"),
            42,
        ),
        (
            include_str!("../../../examples/debug/main_while_continue_6.doria"),
            6,
        ),
        (
            include_str!("../../../examples/debug/main_while_return_42.doria"),
            42,
        ),
        (
            include_str!("../../../examples/debug/main_while_nested_6.doria"),
            6,
        ),
    ] {
        assert_eq!(interpret(source).exit_status, expected);
    }
}

#[test]
fn interpreter_while_false_falls_through() {
    let output = interpret(
        r#"function main(): int
{
    while (false) {
        return 0;
    }

    return 42;
}
"#,
    );

    assert_eq!(output.exit_status, 42);
}

#[test]
fn interpreter_allows_finite_while_loops_beyond_the_old_fuel_limit() {
    let source = r#"function main(): int
{
    let writable $i = 0;

    while ($i < 5000) {
        $i++;
    }

    return 0;
}
"#;
    let output = interpret(source);

    assert_eq!(output.exit_status, 0);
    assert_eq!(debug_contents(source), "exit_status: 0\nstdout:\n");
}

#[test]
fn explicit_test_limit_bounds_changing_state_while_loops() {
    let program = lower(
        r#"function main(): void
{
    let writable $i = 0;

    while (true) {
        $i++;
    }
}
"#,
    );
    let error = doriac::mir_interpreter::interpret_with_limits(
        &program,
        doriac::mir_interpreter::InterpreterLimits {
            max_executed_blocks: Some(100),
            max_call_frames: None,
        },
    )
    .expect_err("explicit test limit should stop the loop");
    assert!(error.message.contains("explicit test execution limit"));
}

#[test]
fn explicitly_limited_interpreter_stops_non_terminating_source_while_loops() {
    let program = lower(
        r#"function main(): void
{
    while (true) {
    }
}
"#,
    );

    let error = doriac::mir_interpreter::interpret_with_limits(
        &program,
        doriac::mir_interpreter::InterpreterLimits {
            max_executed_blocks: Some(10),
            max_call_frames: None,
        },
    )
    .expect_err("explicit test limit should stop the loop");
    assert!(error.message.contains("explicit test execution limit"));
}

#[test]
fn interpreter_preserves_stdout_across_while_iterations() {
    let output = interpret(include_str!(
        "../../../examples/debug/main_while_echo_xxx.doria"
    ));

    assert_eq!(output.exit_status, 0);
    assert_eq!(output.stdout, b"xxx");
}

#[test]
fn nested_break_exits_only_the_inner_while() {
    let output = interpret(
        r#"function main(): int
{
    let writable $outer = 0;
    let writable $count = 0;

    while ($outer < 3) {
        let writable $inner = 0;

        while ($inner < 3) {
            $count++;
            break;
        }

        $outer++;
    }

    return $count;
}
"#,
    );

    assert_eq!(output.exit_status, 3);
}

#[test]
fn nested_continue_targets_only_the_inner_while() {
    let output = interpret(
        r#"function main(): int
{
    let writable $outer = 0;
    let writable $count = 0;

    while ($outer < 3) {
        let writable $inner = 0;

        while ($inner < 3) {
            $inner++;

            if ($inner < 3) {
                continue;
            }

            $count++;
        }

        $outer++;
    }

    return $count;
}
"#,
    );

    assert_eq!(output.exit_status, 3);
}

#[test]
fn rejects_break_and_continue_outside_loops_in_mir_lowering() {
    for (source, detail) in [
        (
            "function main(): void\n{\n    break;\n}\n",
            "`break` requires an enclosing loop",
        ),
        (
            "function main(): void\n{\n    continue;\n}\n",
            "`continue` requires an enclosing loop",
        ),
    ] {
        let diagnostics = unsupported_after_parsing(source);
        assert_stage_11g_unsupported(&diagnostics, detail);
    }
}

#[test]
fn debug_target_handles_stage_11d_examples() {
    for (source, expected) in [
        (
            include_str!("../../../examples/debug/main_while_count_42.doria"),
            "exit_status: 42\nstdout:\n",
        ),
        (
            include_str!("../../../examples/debug/main_while_break_42.doria"),
            "exit_status: 42\nstdout:\n",
        ),
        (
            include_str!("../../../examples/debug/main_while_continue_6.doria"),
            "exit_status: 6\nstdout:\n",
        ),
        (
            include_str!("../../../examples/debug/main_while_echo_xxx.doria"),
            "exit_status: 0\nstdout: xxx\n",
        ),
        (
            include_str!("../../../examples/debug/main_while_return_42.doria"),
            "exit_status: 42\nstdout:\n",
        ),
        (
            include_str!("../../../examples/debug/main_while_nested_6.doria"),
            "exit_status: 6\nstdout:\n",
        ),
    ] {
        assert_eq!(debug_contents(source), expected);
    }
}

#[test]
fn cranelift_object_covers_stage_11d_while_shapes_without_linker() {
    for source in [
        include_str!("../../../examples/debug/main_while_count_42.doria"),
        include_str!("../../../examples/debug/main_while_break_42.doria"),
    ] {
        let mir_exit = interpret(source).exit_status;

        assert_eq!(mir_exit, 42);
        assert!(!lower_object(source).is_empty());
    }
}

#[test]
fn cranelift_object_covers_stage_11c_if_shapes_without_linker() {
    for source in [
        include_str!("../../../examples/debug/main_if_return_42.doria"),
        include_str!("../../../examples/debug/main_if_assignment_42.doria"),
    ] {
        let mir_exit = interpret(source).exit_status;

        assert_eq!(mir_exit, 42);
        assert!(!lower_object(source).is_empty());
    }
}

#[test]
fn cranelift_object_covers_stage_11b_int_shapes_without_linker() {
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
        let mir_exit = interpret(source).exit_status;

        assert_eq!(mir_exit, expected);
        assert!(!lower_object(source).is_empty());
    }
}

#[test]
fn cranelift_object_covers_stage_11a_literal_shapes_without_linker() {
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
        let mir_exit = interpret(source).exit_status;

        assert_eq!(mir_exit, expected);
        assert!(!lower_object(source).is_empty());
    }
}

#[test]
fn lowers_for_to_initializer_header_body_increment_and_exit_blocks() {
    let program = lower(include_str!(
        "../../../examples/debug/main_for_count_10.doria"
    ));
    let function = &program.functions[0];
    let blocks = &function.blocks;

    assert_eq!(blocks.len(), 5);
    assert_eq!(
        blocks[0].statements,
        vec![
            Statement::AssignLocal {
                target: LocalId(0),
                value: int_rvalue(0),
            },
            Statement::AssignLocal {
                target: LocalId(1),
                value: int_rvalue(0),
            },
        ]
    );
    assert_eq!(blocks[0].terminator, Terminator::Jump(BlockId(1)));
    assert_eq!(
        blocks[1].terminator,
        Terminator::Branch {
            condition: int_compare(
                CompareOp::Less,
                local_integer_expression(LocalId(1)),
                int_constant(10),
            ),
            then_block: BlockId(2),
            else_block: BlockId(4),
        }
    );
    assert_eq!(
        blocks[2].statements,
        vec![Statement::AssignLocal {
            target: LocalId(0),
            value: Rvalue::Value(ValueExpression::Integer(IntegerExpression::Binary {
                ty: DEFAULT_INT,
                op: IntegerBinaryOp::Add,
                left: Box::new(integer_expression(Operand::Local(LocalId(0)))),
                right: Box::new(integer_expression(int_operand(1))),
            })),
        }]
    );
    assert_eq!(blocks[2].terminator, Terminator::Jump(BlockId(3)));
    assert_eq!(
        blocks[3].statements,
        vec![Statement::AssignLocal {
            target: LocalId(1),
            value: Rvalue::Value(ValueExpression::Integer(IntegerExpression::Binary {
                ty: DEFAULT_INT,
                op: IntegerBinaryOp::Add,
                left: Box::new(integer_expression(Operand::Local(LocalId(1)))),
                right: Box::new(integer_expression(int_operand(1))),
            })),
        }]
    );
    assert_eq!(blocks[3].terminator, Terminator::Jump(BlockId(1)));
    assert_eq!(
        blocks[4].terminator,
        int_return(local_integer_expression(LocalId(0)))
    );
}

#[test]
fn lowers_exclusive_range_foreach_to_counter_binding_update_and_exit_blocks() {
    let program = lower(include_str!(
        "../../../examples/debug/main_foreach_range_exclusive_10.doria"
    ));
    let function = &program.functions[0];
    let blocks = &function.blocks;

    assert_eq!(function.locals.len(), 4);
    assert!(function.locals[1].synthetic);
    assert!(function.locals[1].writable);
    assert!(function.locals[2].synthetic);
    assert!(!function.locals[2].writable);
    assert_eq!(function.locals[3].name, "i");
    assert!(!function.locals[3].synthetic);
    assert!(!function.locals[3].writable);
    assert_eq!(blocks.len(), 5);
    assert_eq!(
        blocks[1].terminator,
        Terminator::Branch {
            condition: int_compare(
                CompareOp::Less,
                local_integer_expression(LocalId(1)),
                local_integer_expression(LocalId(2)),
            ),
            then_block: BlockId(2),
            else_block: BlockId(4),
        }
    );
    assert_eq!(
        blocks[2].statements[0],
        Statement::AssignLocal {
            target: LocalId(3),
            value: Rvalue::Value(int_expression_value(local_integer_expression(LocalId(1)))),
        }
    );
    assert_eq!(blocks[2].terminator, Terminator::Jump(BlockId(3)));
    assert_eq!(
        blocks[3].statements,
        vec![Statement::AssignLocal {
            target: LocalId(1),
            value: Rvalue::Value(ValueExpression::Integer(IntegerExpression::Binary {
                ty: DEFAULT_INT,
                op: IntegerBinaryOp::Add,
                left: Box::new(integer_expression(Operand::Local(LocalId(1)))),
                right: Box::new(integer_expression(int_operand(1))),
            })),
        }]
    );
    assert_eq!(blocks[3].terminator, Terminator::Jump(BlockId(1)));
}

#[test]
fn lowers_inclusive_range_foreach_with_terminal_guard() {
    let program = lower(include_str!(
        "../../../examples/debug/main_foreach_range_inclusive_11.doria"
    ));
    let blocks = &program.functions[0].blocks;

    assert_eq!(blocks.len(), 6);
    assert_eq!(
        blocks[1].terminator,
        Terminator::Branch {
            condition: int_compare(
                CompareOp::LessEqual,
                local_integer_expression(LocalId(1)),
                local_integer_expression(LocalId(2)),
            ),
            then_block: BlockId(2),
            else_block: BlockId(5),
        }
    );
    assert_eq!(
        blocks[3].terminator,
        Terminator::Branch {
            condition: int_compare(
                CompareOp::Equal,
                local_integer_expression(LocalId(1)),
                local_integer_expression(LocalId(2)),
            ),
            then_block: BlockId(5),
            else_block: BlockId(4),
        }
    );
    assert_eq!(blocks[4].terminator, Terminator::Jump(BlockId(1)));
    assert_eq!(
        interpret(include_str!(
            "../../../examples/debug/main_foreach_range_inclusive_11.doria"
        ))
        .exit_status,
        11
    );
}

#[test]
fn for_and_range_foreach_route_continue_and_break_to_loop_specific_targets() {
    let for_continue = lower(include_str!(
        "../../../examples/debug/main_for_continue_6.doria"
    ));
    assert_eq!(
        for_continue.functions[0].blocks[5].terminator,
        Terminator::Jump(BlockId(3))
    );

    let for_break = lower(include_str!(
        "../../../examples/debug/main_for_break_5.doria"
    ));
    assert_eq!(
        for_break.functions[0].blocks[5].terminator,
        Terminator::Jump(BlockId(4))
    );

    let foreach_continue = lower(include_str!(
        "../../../examples/debug/main_foreach_range_continue_6.doria"
    ));
    assert_eq!(
        foreach_continue.functions[0].blocks[5].terminator,
        Terminator::Jump(BlockId(3))
    );

    let foreach_break = lower(include_str!(
        "../../../examples/debug/main_foreach_range_break_5.doria"
    ));
    assert_eq!(
        foreach_break.functions[0].blocks[5].terminator,
        Terminator::Jump(BlockId(4))
    );
}

#[test]
fn lowers_early_return_inside_for() {
    let program = lower(
        r#"function main(): int
{
    for (let writable $i = 0; $i < 10; $i++) {
        return 42;
    }

    return 0;
}
"#,
    );

    assert!(program.functions[0]
        .blocks
        .iter()
        .any(|block| block.terminator == int_return(int_constant(42))));
    assert_eq!(
        doriac::mir_interpreter::interpret(&program)
            .expect("early return should interpret")
            .exit_status,
        42
    );
}

#[test]
fn interprets_for_assignment_and_omitted_increment_forms() {
    for source in [
        r#"function main(): int
{
    let writable $i = 99;
    let writable $count = 0;

    for ($i = 0; $i < 3; $i += 1) {
        $count++;
    }

    return $count;
}
"#,
        r#"function main(): int
{
    let writable $i = 0;

    for (; $i < 3;) {
        $i++;
    }

    return $i;
}
"#,
    ] {
        assert_eq!(interpret(source).exit_status, 3);
    }
}

#[test]
fn for_initializers_and_foreach_bindings_preserve_shadowed_outer_locals() {
    let output = interpret(
        r#"function main(): int
{
    let writable $i = 5;

    for (let writable $i = 0; $i < 2; $i++) {
    }

    foreach (0..<2 as $i) {
    }

    return $i;
}
"#,
    );

    assert_eq!(output.exit_status, 5);
}

#[test]
fn interprets_grouped_range_with_stage_11b_expression_bounds() {
    let output = interpret(
        r#"function main(): int
{
    let $start = 1;
    let $end = 4;
    let writable $count = 0;

    foreach (($start - 1)..<($end + 1) as $i) {
        $count++;
    }

    return $count;
}
"#,
    );

    assert_eq!(output.exit_status, 5);
}

#[test]
fn inclusive_range_stops_before_incrementing_past_int64_max() {
    let output = interpret(
        r#"function main(): int
{
    let writable $count = 0;

    foreach (9223372036854775807..9223372036854775807 as $i) {
        $count++;
    }

    return $count;
}
"#,
    );

    assert_eq!(output.exit_status, 1);
}

#[test]
fn inclusive_range_continue_stops_before_incrementing_past_int64_max() {
    let output = interpret(
        r#"function main(): int
{
    let writable $count = 0;

    foreach (9223372036854775807..9223372036854775807 as $i) {
        $count++;
        continue;
    }

    return $count;
}
"#,
    );

    assert_eq!(output.exit_status, 1);
}

#[test]
fn mixed_nested_loops_use_innermost_break_and_continue_targets() {
    let output = interpret(
        r#"function main(): int
{
    let writable $count = 0;
    let writable $outer = 0;

    while ($outer < 2) {
        for (let writable $inner = 0; $inner < 2; $inner++) {
            foreach (0..<3 as $item) {
                if ($item < 2) {
                    continue;
                }

                $count++;
                break;
            }
        }

        $outer++;
    }

    return $count;
}
"#,
    );

    assert_eq!(output.exit_status, 4);
}

#[test]
fn interpreter_preserves_stdout_across_for_and_foreach_iterations() {
    let output = interpret(
        r#"function main(): void
{
    for (let writable $i = 0; $i < 3; $i++) {
        echo "x";
    }

    foreach (0..<2 as $i) {
        echo "y";
    }
}
"#,
    );

    assert_eq!(output.exit_status, 0);
    assert_eq!(output.stdout, b"xxxyy");
}

#[test]
fn interprets_stage_11e_for_and_range_foreach_examples() {
    for (source, expected) in [
        (
            include_str!("../../../examples/debug/main_for_count_10.doria"),
            10,
        ),
        (
            include_str!("../../../examples/debug/main_for_continue_6.doria"),
            6,
        ),
        (
            include_str!("../../../examples/debug/main_for_break_5.doria"),
            5,
        ),
        (
            include_str!("../../../examples/debug/main_foreach_range_exclusive_10.doria"),
            10,
        ),
        (
            include_str!("../../../examples/debug/main_foreach_range_inclusive_11.doria"),
            11,
        ),
        (
            include_str!("../../../examples/debug/main_foreach_range_break_5.doria"),
            5,
        ),
        (
            include_str!("../../../examples/debug/main_foreach_range_continue_6.doria"),
            6,
        ),
        (
            include_str!("../../../examples/debug/main_nested_loop_mix_6.doria"),
            6,
        ),
    ] {
        assert_eq!(interpret(source).exit_status, expected);
    }
}

#[test]
fn lowers_early_return_inside_range_foreach() {
    let output = interpret(
        r#"function main(): int
{
    foreach (0..<10 as $i) {
        if ($i == 3) {
            return $i;
        }
    }

    return 0;
}
"#,
    );

    assert_eq!(output.exit_status, 3);
}

#[test]
fn rejects_unsupported_stage_11e_foreach_shapes() {
    let collection = unsupported_after_parsing(
        r#"function main(): void
{
    foreach ($items as $item) {
    }
}
"#,
    );
    assert_stage_11g_unsupported(&collection, "supports `foreach` only over integer ranges");

    let key_value = unsupported_after_parsing(
        r#"function main(): void
{
    foreach (0..<10 as $key => $value) {
    }
}
"#,
    );
    assert_stage_11g_unsupported(&key_value, "key bindings");
}

#[test]
fn debug_target_handles_stage_11e_examples() {
    for (source, expected) in [
        (
            include_str!("../../../examples/debug/main_for_count_10.doria"),
            "exit_status: 10\nstdout:\n",
        ),
        (
            include_str!("../../../examples/debug/main_for_continue_6.doria"),
            "exit_status: 6\nstdout:\n",
        ),
        (
            include_str!("../../../examples/debug/main_for_break_5.doria"),
            "exit_status: 5\nstdout:\n",
        ),
        (
            include_str!("../../../examples/debug/main_foreach_range_exclusive_10.doria"),
            "exit_status: 10\nstdout:\n",
        ),
        (
            include_str!("../../../examples/debug/main_foreach_range_inclusive_11.doria"),
            "exit_status: 11\nstdout:\n",
        ),
        (
            include_str!("../../../examples/debug/main_foreach_range_break_5.doria"),
            "exit_status: 5\nstdout:\n",
        ),
        (
            include_str!("../../../examples/debug/main_foreach_range_continue_6.doria"),
            "exit_status: 6\nstdout:\n",
        ),
        (
            include_str!("../../../examples/debug/main_nested_loop_mix_6.doria"),
            "exit_status: 6\nstdout:\n",
        ),
    ] {
        assert_eq!(debug_contents(source), expected);
    }
}

#[test]
fn cranelift_object_covers_stage_11e_loop_shapes_without_linker() {
    for (source, expected) in [
        (
            include_str!("../../../examples/debug/main_for_count_10.doria"),
            10,
        ),
        (
            include_str!("../../../examples/debug/main_foreach_range_exclusive_10.doria"),
            10,
        ),
        (
            include_str!("../../../examples/debug/main_foreach_range_inclusive_11.doria"),
            11,
        ),
    ] {
        let mir_exit = interpret(source).exit_status;

        assert_eq!(mir_exit, expected);
        assert!(!lower_object(source).is_empty());
    }
}

#[test]
fn explicit_test_limit_bounds_changing_state_infinite_for_loops() {
    let program = lower(
        r#"function main(): void
{
    for (let writable $i = 0;; $i++) {
    }
}
"#,
    );
    let error = doriac::mir_interpreter::interpret_with_limits(
        &program,
        doriac::mir_interpreter::InterpreterLimits {
            max_executed_blocks: Some(100),
            max_call_frames: None,
        },
    )
    .expect_err("explicit test limit should stop the loop");
    assert!(error.message.contains("explicit test execution limit"));
}

#[test]
fn range_foreach_binding_remains_readonly_before_mir_lowering() {
    let diagnostics = doriac::lower_source_to_mir(
        "test.doria",
        r#"function main(): void
{
    foreach (0..<10 as $i) {
        $i++;
    }
}
"#,
    )
    .expect_err("semantic checking should reject mutation of a foreach binding");

    assert_eq!(diagnostics[0].code, "E0201");
}

#[test]
fn stage_11f_requires_exactly_one_main() {
    let missing = unsupported_after_parsing(
        r#"function helper(): int
{
    return 42;
}
"#,
    );
    assert_stage_11g_unsupported(&missing, "exactly one top-level `main` function");

    let duplicate = unsupported_after_parsing(
        r#"function main(): int
{
    return 0;
}

function main(): int
{
    return 1;
}
"#,
    );
    assert_stage_11g_unsupported(&duplicate, "exactly one top-level `main` function");
}

#[test]
fn stage_11f_lowers_int_parameters_to_function_locals() {
    let program = lower(include_str!(
        "../../../examples/debug/main_function_add_42.doria"
    ));
    let add = &program.functions[0];

    assert_eq!(add.name, "add");
    assert_eq!(add.params, vec![LocalId(0), LocalId(1)]);
    assert_eq!(add.locals.len(), 2);
    assert_eq!(add.locals[0].name, "left");
    assert_eq!(add.locals[1].name, "right");
    assert!(!add.locals[0].synthetic);
    assert!(!add.locals[1].synthetic);
    assert_eq!(
        add.locals[0].ty,
        Type::Scalar(ScalarType::Integer(DEFAULT_INT))
    );
    assert_eq!(
        add.locals[1].ty,
        Type::Scalar(ScalarType::Integer(DEFAULT_INT))
    );
}

#[test]
fn stage_11f_lowers_int_calls_in_returns_and_arithmetic() {
    let add_program = lower(include_str!(
        "../../../examples/debug/main_function_add_42.doria"
    ));
    let main = &add_program.functions[1];
    assert_eq!(
        main.blocks[0].terminator,
        Terminator::Return(Rvalue::Value(ValueExpression::Integer(
            IntegerExpression::Call {
                ty: DEFAULT_INT,
                function: FunctionId(0),
                args: vec![
                    Rvalue::Value(int_expression_value(int_constant(20))),
                    Rvalue::Value(int_expression_value(int_constant(22))),
                ],
            }
        )))
    );

    let chain_program = lower(include_str!(
        "../../../examples/debug/main_function_chain_42.doria"
    ));
    let answer = &chain_program.functions[1];
    assert!(matches!(
        answer.blocks[0].terminator,
        Terminator::Return(Rvalue::Value(ValueExpression::Integer(
            IntegerExpression::Binary {
                op: IntegerBinaryOp::Add,
                ..
            }
        )))
    ));
}

#[test]
fn stage_11f_lowers_int_calls_in_comparisons() {
    let program = lower(include_str!(
        "../../../examples/debug/main_function_if_condition_42.doria"
    ));
    let main = &program.functions[1];

    assert_eq!(
        main.blocks[0].terminator,
        Terminator::Branch {
            condition: Condition::Compare {
                op: CompareOp::Equal,
                left: Box::new(ValueExpression::Integer(IntegerExpression::Call {
                    ty: DEFAULT_INT,
                    function: FunctionId(0),
                    args: Vec::new(),
                })),
                right: Box::new(int_expression_value(int_constant(42))),
            },
            then_block: BlockId(1),
            else_block: BlockId(2),
        }
    );
}

#[test]
fn stage_11f_lowers_void_calls_and_literal_echo_helpers() {
    let program = lower(include_str!(
        "../../../examples/debug/main_function_echo_hello.doria"
    ));

    assert_eq!(
        program.functions[0].blocks[0].statements,
        vec![Statement::EchoStringLiteral("Hello ".to_string())]
    );
    assert_eq!(
        program.functions[1].blocks[0].statements,
        vec![Statement::EchoStringLiteral("Doria!".to_string())]
    );
    assert_eq!(
        program.functions[2].blocks[0].statements,
        vec![
            Statement::CallVoid {
                function: FunctionId(0),
                args: Vec::new(),
            },
            Statement::CallVoid {
                function: FunctionId(1),
                args: Vec::new(),
            },
        ]
    );
}

#[test]
fn stage_11f_helpers_reuse_existing_loop_lowering() {
    let while_program = lower(include_str!(
        "../../../examples/debug/main_function_loop_42.doria"
    ));
    assert!(while_program.functions[0].blocks.len() > 1);
    assert_eq!(
        interpret(include_str!(
            "../../../examples/debug/main_function_loop_42.doria"
        ))
        .exit_status,
        42
    );

    let for_output = interpret(
        r#"function countWithFor(): int
{
    let writable $count = 0;

    for (let writable $i = 0; $i < 3; $i++) {
        $count++;
    }

    return $count;
}

function main(): int
{
    return countWithFor();
}
"#,
    );
    assert_eq!(for_output.exit_status, 3);
}

#[test]
fn stage_16_accepts_string_helper_signatures() {
    let output = interpret(
        r#"function title(string $name): string
{
    return "Hello " . $name;
}

function main(): void
{
    echo title("Doria");
}
"#,
    );
    assert_eq!(output.stdout, b"Hello Doria");
}

#[test]
fn stage_11f_rejects_calls_with_the_wrong_result_context() {
    let ignored_int = unsupported_after_parsing(
        r#"function main(): void
{
    one();
}

function one(): int
{
    return 1;
}
"#,
    );
    assert_stage_11g_unsupported(&ignored_int, "cannot be used as a statement");

    let void_as_int = unsupported_after_parsing(
        r#"function hello(): void
{
}

function main(): int
{
    return hello();
}
"#,
    );
    assert_stage_11g_unsupported(&void_as_int, "cannot be used as a scalar expression");
}

#[test]
fn stage_13_lowers_nested_integer_call_arguments() {
    let output = interpret(
        r#"function one(): int
{
    return 1;
}

function add(int $left, int $right): int
{
    return $left + $right;
}

function main(): int
{
    return add(one(), 41);
}
"#,
    );

    assert_eq!(output.exit_status, 42);
}

#[test]
fn stage_12_interprets_direct_recursion() {
    let output = interpret(
        r#"function count(int $n): int
{
    if ($n == 0) {
        return 0;
    }

    return count($n - 1);
}

function main(): int
{
    return count(1);
}
"#,
    );

    assert_eq!(output.exit_status, 0);
}

#[test]
fn stage_12_interprets_mutual_recursion() {
    let output = interpret(
        r#"function a(int $remaining): int
{
    if ($remaining == 0) {
        return 42;
    }

    return b($remaining - 1);
}

function b(int $remaining): int
{
    if ($remaining == 0) {
        return 42;
    }

    return a($remaining - 1);
}

function main(): int
{
    return a(10);
}
"#,
    );

    assert_eq!(output.exit_status, 42);
}

#[test]
fn stage_11f_interprets_call_arguments_and_preserves_caller_locals() {
    let argument_output = interpret(
        r#"function add(int $left, int $right): int
{
    return $left + $right;
}

function main(): int
{
    return add(10 + 10, 22);
}
"#,
    );
    assert_eq!(argument_output.exit_status, 42);

    let caller_output = interpret(
        r#"function increment(writable int $value): int
{
    $value++;

    return $value;
}

function main(): int
{
    let writable $value = 41;
    let $incremented = increment($value);

    return $value;
}
"#,
    );
    assert_eq!(caller_output.exit_status, 41);
}

#[test]
fn stage_11f_helper_ints_are_not_process_status_bounded() {
    let output = interpret(include_str!(
        "../../../examples/debug/main_function_big_int_helper.doria"
    ));

    assert_eq!(output.exit_status, 0);
    assert!(output.stdout.is_empty());
}

#[test]
fn stage_11f_stdout_accumulates_across_void_helper_calls() {
    let output = interpret(include_str!(
        "../../../examples/debug/main_function_echo_hello.doria"
    ));

    assert_eq!(output.exit_status, 0);
    assert_eq!(output.stdout, b"Hello Doria!");
}

#[test]
fn stage_11f_debug_target_handles_all_examples() {
    for (source, expected) in [
        (
            include_str!("../../../examples/debug/main_function_add_42.doria"),
            "exit_status: 42\nstdout:\n",
        ),
        (
            include_str!("../../../examples/debug/main_function_chain_42.doria"),
            "exit_status: 42\nstdout:\n",
        ),
        (
            include_str!("../../../examples/debug/main_function_loop_42.doria"),
            "exit_status: 42\nstdout:\n",
        ),
        (
            include_str!("../../../examples/debug/main_function_echo_hello.doria"),
            "exit_status: 0\nstdout: Hello Doria!\n",
        ),
        (
            include_str!("../../../examples/debug/main_function_big_int_helper.doria"),
            "exit_status: 0\nstdout:\n",
        ),
        (
            include_str!("../../../examples/debug/main_function_if_condition_42.doria"),
            "exit_status: 42\nstdout:\n",
        ),
    ] {
        assert_eq!(debug_contents(source), expected);
    }
}

#[test]
fn explicitly_limited_interpreter_can_bound_call_frames() {
    let program = Program {
        classes: vec![],
        functions: vec![Function {
            id: FunctionId(0),
            name: "main".to_string(),
            params: Vec::new(),
            return_type: ReturnType::Value(Type::Scalar(ScalarType::Integer(DEFAULT_INT))),
            locals: vec![Local {
                id: LocalId(0),
                name: "_tmp0".to_string(),
                ty: Type::Scalar(ScalarType::Integer(DEFAULT_INT)),
                writable: false,
                synthetic: true,
                owned: false,
            }],
            blocks: vec![BasicBlock {
                id: BlockId(0),
                statements: vec![Statement::AssignLocal {
                    target: LocalId(0),
                    value: Rvalue::Value(ValueExpression::Integer(IntegerExpression::Call {
                        ty: DEFAULT_INT,
                        function: FunctionId(0),
                        args: Vec::new(),
                    })),
                }],
                terminator: int_return(local_integer_expression(LocalId(0))),
            }],
            entry_block: BlockId(0),
        }],
        entry: FunctionId(0),
    };

    let error = doriac::mir_interpreter::interpret_with_limits(
        &program,
        doriac::mir_interpreter::InterpreterLimits {
            max_executed_blocks: None,
            max_call_frames: Some(32),
        },
    )
    .expect_err("explicit test limit should bound recursive MIR");
    assert!(error
        .message
        .contains("explicit test call-frame limit of 32"));
}

#[test]
fn stage_11f_cranelift_object_supports_helpers_without_linker() {
    for source in [
        include_str!("../../../examples/debug/main_function_add_42.doria"),
        include_str!("../../../examples/debug/main_function_loop_42.doria"),
    ] {
        let mir_exit = interpret(source).exit_status;

        assert_eq!(mir_exit, 42);
        assert!(!lower_object(source).is_empty());
    }
}

#[test]
fn stage_11g_lowers_explicit_string_locals_and_local_concat_in_source_order() {
    let explicit = lower(
        r#"function main(): void
{
    string $name = "Doria";

    echo $name;
}
"#,
    );
    let main = &explicit.functions[explicit.entry.0];
    assert_eq!(main.locals[0].ty, Type::String);
    assert_eq!(
        main.blocks[0].statements[1],
        Statement::EchoString(StringExpression::Local(LocalId(0)))
    );

    let concat = lower(include_str!(
        "../../../examples/debug/main_string_concat_from_locals.doria"
    ));
    let main = &concat.functions[concat.entry.0];
    assert_eq!(
        main.blocks[0].statements.last(),
        Some(&Statement::EchoString(StringExpression::Concat(vec![
            StringExpression::Local(LocalId(0)),
            StringExpression::Local(LocalId(1)),
            StringExpression::Local(LocalId(2)),
        ])))
    );
}

#[test]
fn stage_11g_lowers_string_concat_inside_void_helpers() {
    let program = lower(include_str!(
        "../../../examples/debug/main_function_string_local_echo.doria"
    ));
    let helper = &program.functions[0];

    assert_eq!(helper.name, "hello");
    assert_eq!(helper.locals[0].ty, Type::String);
    assert_eq!(
        helper.blocks[0].statements[1],
        Statement::EchoString(StringExpression::Concat(vec![
            StringExpression::Literal("Hello ".to_string()),
            StringExpression::Local(LocalId(0)),
            StringExpression::Literal("!".to_string()),
        ]))
    );
}

#[test]
fn stage_11g_lowers_string_echo_inside_loop_blocks() {
    let program = lower(include_str!(
        "../../../examples/debug/main_string_loop_echo_xxx.doria"
    ));
    let main = &program.functions[program.entry.0];

    assert!(main.blocks.iter().any(|block| {
        block.statements.iter().any(|statement| {
            *statement == Statement::EchoString(StringExpression::Local(LocalId(1)))
        })
    }));

    let output = interpret(
        r#"function main(): void
{
    let $mark = "x";

    for (let writable $i = 0; $i < 1; $i++) {
        echo $mark;
    }

    foreach (0..<1 as $i) {
        echo $mark;
    }
}
"#,
    );
    assert_eq!(output.stdout, b"xx");
}

#[test]
fn stage_11g_supports_readonly_string_local_concat_initializers() {
    let output = interpret(
        r#"function main(): void
{
    let $name = "Doria";
    let $message = "Hello " . $name . "!";

    echo $message;
}
"#,
    );

    assert_eq!(output.exit_status, 0);
    assert_eq!(output.stdout, b"Hello Doria!");
}

#[test]
fn stage_16_accepts_writable_string_locals() {
    for source in [
        r#"function main(): void
{
    let writable $name = "Doria";
}
"#,
        r#"function main(): void
{
    writable string $name = "Doria";
}
"#,
    ] {
        let program = lower(source);
        assert!(program.functions[0].locals[0].writable);
    }
}

#[test]
fn stage_16_lowers_string_assignment() {
    let output = interpret(
        r#"function main(): void
{
    let writable $name = "Doria";
    $name = "Other";
    echo $name;
}
"#,
    );
    assert_eq!(output.stdout, b"Other");
}

#[test]
fn stage_16_displays_integer_in_string_concat() {
    let output = interpret(
        r#"function main(): void
{
    echo "count: " . 42;
}
"#,
    );

    assert_eq!(output.stdout, b"count: 42");
}

#[test]
fn stage_16_accepts_string_parameters_and_returns() {
    let output = interpret(
        r#"function greet(string $name): string
{
    return "Hello " . $name;
}

function main(): void
{
    echo greet("Doria");
}
"#,
    );
    assert_eq!(output.stdout, b"Hello Doria");
}

#[test]
fn stage_11g_interprets_all_string_examples() {
    for (source, expected) in [
        (
            include_str!("../../../examples/debug/main_string_local_hello.doria"),
            b"Doria".as_slice(),
        ),
        (
            include_str!("../../../examples/debug/main_string_concat_hello.doria"),
            b"Hello Doria!".as_slice(),
        ),
        (
            include_str!("../../../examples/debug/main_string_concat_from_locals.doria"),
            b"Hello Doria!".as_slice(),
        ),
        (
            include_str!("../../../examples/debug/main_function_string_local_echo.doria"),
            b"Hello Doria!".as_slice(),
        ),
        (
            include_str!("../../../examples/debug/main_string_loop_echo_xxx.doria"),
            b"xxx".as_slice(),
        ),
        (
            include_str!("../../../examples/debug/main_string_if_echo.doria"),
            b"Hello Doria!".as_slice(),
        ),
    ] {
        let output = interpret(source);
        assert_eq!(output.exit_status, 0);
        assert_eq!(output.stdout, expected);
    }
}

#[test]
fn stage_11g_stdout_accumulates_across_helpers_and_string_expressions() {
    let output = interpret(
        r#"function greeting(): void
{
    let $hello = "Hello ";

    echo $hello;
}

function subject(): void
{
    let $name = "Doria";

    echo $name . "!";
}

function main(): void
{
    greeting();
    subject();
}
"#,
    );

    assert_eq!(output.stdout, b"Hello Doria!");
}

#[test]
fn stage_11g_string_echo_adds_no_implicit_newline() {
    let output = interpret(include_str!(
        "../../../examples/debug/main_string_concat_hello.doria"
    ));

    assert_eq!(output.stdout, b"Hello Doria!");
    assert!(!output.stdout.ends_with(b"\n"));
}

#[test]
fn stage_11h_supports_string_echo_inside_int_returning_functions() {
    for source in [
        include_str!("../../../examples/debug/main_int_echo_success.doria"),
        include_str!("../../../examples/debug/main_int_helper_echo_success.doria"),
    ] {
        let program = lower(source);
        assert!(program.functions.iter().any(|function| {
            function.return_type
                == ReturnType::Value(Type::Scalar(ScalarType::Integer(DEFAULT_INT)))
                && function.blocks.iter().any(|block| {
                    block.statements.iter().any(|statement| {
                        matches!(
                            statement,
                            Statement::EchoStringLiteral(_) | Statement::EchoString(_)
                        )
                    })
                })
        }));

        let output = interpret(source);
        assert_eq!(output.exit_status, 0);
        assert_eq!(output.stdout, b"Hello Doria!");
        assert_eq!(
            debug_contents(source),
            "exit_status: 0\nstdout: Hello Doria!\n"
        );
    }
}

#[test]
fn stage_11h_ignores_unreachable_statements_after_terminators() {
    let output = interpret(
        r#"function main(): int
{
    let writable $value = 42;

    if (false) {
        return 0;
        $value = 0;
    }

    for (let writable $i = 0; $i < 1; $i++) {
        continue;
        $value = 0;
    }

    while (true) {
        break;
        $value = 0;
    }

    return $value;
}
"#,
    );

    assert_eq!(output.exit_status, 42);
}

#[test]
fn stage_11g_debug_target_handles_all_examples() {
    for (source, expected) in [
        (
            include_str!("../../../examples/debug/main_string_local_hello.doria"),
            "exit_status: 0\nstdout: Doria\n",
        ),
        (
            include_str!("../../../examples/debug/main_string_concat_hello.doria"),
            "exit_status: 0\nstdout: Hello Doria!\n",
        ),
        (
            include_str!("../../../examples/debug/main_string_concat_from_locals.doria"),
            "exit_status: 0\nstdout: Hello Doria!\n",
        ),
        (
            include_str!("../../../examples/debug/main_function_string_local_echo.doria"),
            "exit_status: 0\nstdout: Hello Doria!\n",
        ),
        (
            include_str!("../../../examples/debug/main_string_loop_echo_xxx.doria"),
            "exit_status: 0\nstdout: xxx\n",
        ),
        (
            include_str!("../../../examples/debug/main_string_if_echo.doria"),
            "exit_status: 0\nstdout: Hello Doria!\n",
        ),
    ] {
        assert_eq!(debug_contents(source), expected);
    }
}

#[test]
fn stage_11g_cranelift_object_supports_string_echo_without_linker() {
    for (source, expected_stdout) in [
        (
            include_str!("../../../examples/debug/main_string_concat_hello.doria"),
            b"Hello Doria!".as_slice(),
        ),
        (
            include_str!("../../../examples/debug/main_string_loop_echo_xxx.doria"),
            b"xxx".as_slice(),
        ),
    ] {
        let mir_output = interpret(source);

        assert_eq!(mir_output.exit_status, 0);
        assert_eq!(mir_output.stdout, expected_stdout);
        assert!(!lower_object(source).is_empty());
    }
}

#[test]
fn stage_18_expression_interpolation_reuses_ordered_display_mir() {
    let source = include_str!("../../../examples/native/main_expression_interpolation.doria");
    let first = lower(source);
    let second = lower(source);
    assert_eq!(first.to_string(), second.to_string());
    assert!(first.to_string().contains("display("));

    let output = doriac::mir_interpreter::interpret(&first).expect("MIR should interpret");
    assert_eq!(output.exit_status, 0);
    assert_eq!(output.stdout, b"sum: 42");
    assert!(output.stderr.is_empty());
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&first)
        .expect("Stage 18 primitive interpolation should lower to Cranelift")
        .is_empty());

    let ordered = interpret(include_str!(
        "../../../examples/native/main_expression_interpolation_order.doria"
    ));
    assert_eq!(ordered.exit_status, 0);
    assert_eq!(ordered.stdout, b"LR=42");
    assert!(ordered.stderr.is_empty());
}

#[test]
fn stage_19_class_metadata_precedes_top_level_execution_lowering() {
    let diagnostics = doriac::lower_source_to_mir(
        "displayable.doria",
        include_str!("../../../examples/php/displayable.doria"),
    )
    .expect_err("top-level executable statements remain outside native MIR");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "M1101"
            && diagnostic
                .message
                .contains("top-level executable statements are not supported")
    }));
}

#[test]
fn stage_19_class_property_access_and_destructor_execute_in_debug_mir() {
    let source = r#"class Message
{
    function __construct(string $text)
    {
    }

    function __destruct()
    {
        echo "message released\n";
    }
}

function deliver(take Message $message): void
{
    echo $message->text . "\n";
}

function main(): void
{
    let $message = new Message("Welcome to Doria");
    deliver($message);
}
"#;

    let output = interpret(source);
    assert_eq!(output.exit_status, 0);
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        "Welcome to Doria\nmessage released\n"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn stage_19_constructor_body_executes_after_promoted_properties_are_initialized() {
    let source = r#"class Message
{
    function __construct(string $text)
    {
        echo "constructed ";
        echo $this->text;
        echo "\n";
    }

    function __destruct()
    {
        echo "released ";
        echo $this->text;
        echo "\n";
    }
}

function main(): void
{
    let $message = new Message("once");
}
"#;

    let output = interpret(source);
    assert_eq!(output.exit_status, 0);
    assert_eq!(output.stdout, b"constructed once\nreleased once\n");
    assert!(output.stderr.is_empty());
    assert!(!lower_object(source).is_empty());
}

#[test]
fn stage_19_constructor_body_can_directly_initialize_a_proven_property() {
    let source = r#"class Message
{
    string $text;

    function __construct(string $value)
    {
        $this->text = $value;
    }

    function __destruct()
    {
        echo $this->text;
        echo " released\n";
    }
}

function main(): void
{
    let $message = new Message("direct");
}
"#;

    let output = interpret(source);
    assert_eq!(output.exit_status, 0);
    assert_eq!(output.stdout, b"direct released\n");
    assert!(output.stderr.is_empty());
    assert!(!lower_object(source).is_empty());
}

#[test]
fn stage_19_rejects_constructor_reads_before_direct_property_initialization() {
    let diagnostics = doriac::lower_source_to_mir(
        "constructor-read-before-init.doria",
        r#"class Message
{
    string $text;

    function __construct(string $value)
    {
        echo $this->text;
        $this->text = $value;
    }
}

function main(): void
{
    let $message = new Message("late");
}
"#,
    )
    .expect_err("the Stage 19 soundness gate must reject a read before initialization");

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "M1101"
            && diagnostic.message.contains("property `$text`")
            && diagnostic.message.contains("initialized")
    }));
}

#[test]
fn stage_19_rejects_constructor_initialization_after_unconditional_panic() {
    let diagnostics = doriac::lower_source_to_mir(
        "constructor-panic-before-init.doria",
        r#"class Message
{
    string $text;

    function __construct(string $value)
    {
        panic("bad");
        $this->text = $value;
    }
}

function main(): void
{
    let $message = new Message("unreachable");
}
"#,
    )
    .expect_err("an unreachable write cannot establish definite initialization");

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "M1101"
            && diagnostic.message.contains("property `$text`")
            && diagnostic.message.contains("not definitely initialized")
    }));
}

#[test]
fn stage_19_infers_string_property_loads_and_comparisons() {
    let source = r#"class Message
{
    function __construct(string $text)
    {
    }
}

function main(): void
{
    let $message = new Message("ready");
    let $copy = $message->text;
    if ($message->text == "ready") {
        echo $copy;
    }
}
"#;

    let output = interpret(source);
    assert_eq!(output.exit_status, 0);
    assert_eq!(output.stdout, b"ready");
    assert!(output.stderr.is_empty());
    assert!(!lower_object(source).is_empty());
}

#[test]
fn stage_19_infers_nullable_string_property_loads_and_comparisons() {
    let source = r#"class Box
{
    ?string $value = null;
}

function main(): void
{
    let $left = new Box();
    let $right = new Box();
    let $copy = $left->value;
    if ($left->value == $right->value && $copy == null) {
        echo "null";
    }
}
"#;

    let output = interpret(source);
    assert_eq!(output.exit_status, 0);
    assert_eq!(output.stdout, b"null");
    assert!(output.stderr.is_empty());
    assert!(!lower_object(source).is_empty());
}

#[test]
fn stage_19_allows_writable_property_mutation_after_constructor_initialization() {
    let source = r#"class Message
{
    writable string $text;

    function __construct(string $first, string $second)
    {
        $this->text = $first;
        $this->text = $second;
    }

    function __destruct()
    {
        echo $this->text;
    }
}

function main(): void
{
    let $message = new Message("first", "second");
}
"#;

    let output = interpret(source);
    assert_eq!(output.exit_status, 0);
    assert_eq!(output.stdout, b"second");
    assert!(output.stderr.is_empty());
    assert!(!lower_object(source).is_empty());
}

#[test]
fn stage_19_mir_drops_borrowed_constructor_temporaries_in_reverse_order() {
    let mut program = lower(
        r#"class Child
{
    function __construct(string $name)
    {
    }

    function __destruct()
    {
        echo "drop ";
        echo $this->name;
        echo "\n";
    }
}

class Parent
{
    function __construct(take Child $first, take Child $second)
    {
    }
}

function main(): void
{
    let $parent = new Parent(new Child("first"), new Child("second"));
}
"#,
    );

    // Source constructors promote every parameter, so a borrowed, unpromoted
    // class parameter is not source-reachable today. Keep the shared MIR and
    // all consumers sound for hand-built MIR and future constructor shapes by
    // removing the promotion while retaining the borrowed lifecycle parameter.
    let parent = program
        .classes
        .iter_mut()
        .find(|class| class.name == "Parent")
        .expect("Parent metadata");
    let parent_id = parent.id;
    parent.properties.clear();
    parent.layout = compute_class_layout(parent_id, [], std::mem::size_of::<usize>() as u32);
    for function in &mut program.functions {
        for block in &mut function.blocks {
            for statement in &mut block.statements {
                if let Statement::AssignLocal {
                    value:
                        Rvalue::Class(ClassExpression::New {
                            class, properties, ..
                        }),
                    ..
                } = statement
                {
                    if *class == parent_id {
                        properties.clear();
                    }
                }
            }
        }
    }

    doriac::mir_validation::validate_program(&program)
        .expect("borrowed unpromoted constructor MIR should remain well formed");
    let output = doriac::mir_interpreter::interpret(&program).expect("MIR should interpret");
    assert_eq!(output.stdout, b"drop second\ndrop first\n");
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("MIR should lower to Cranelift")
        .is_empty());
    #[cfg(feature = "llvm-backend")]
    assert!(!doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("MIR should lower to LLVM")
        .is_empty());
}

#[test]
fn stage_19_property_initializers_run_before_the_constructor_body() {
    let source = r#"function initializeText(): string
{
    echo "property initializer\n";
    return "ready";
}

class Message
{
    string $text = initializeText();

    function __construct(string $label)
    {
        echo "constructor ";
        echo $this->text;
        echo "\n";
    }
}

function main(): void
{
    let $message = new Message("message");
}
"#;

    let output = interpret(source);
    assert_eq!(output.exit_status, 0);
    assert_eq!(output.stdout, b"property initializer\nconstructor ready\n");
    assert!(output.stderr.is_empty());
    assert!(!lower_object(source).is_empty());
}

#[test]
fn stage_19_class_values_move_through_arguments_and_returns() {
    let source = r#"class Token
{
    function __construct(string $name)
    {
    }

    function __destruct()
    {
        echo "drop ";
        echo $this->name;
        echo "\n";
    }
}

function makeToken(): Token
{
    return new Token("returned");
}

function relay(take Token $token): Token
{
    return $token;
}

function main(): void
{
    let $token = relay(makeToken());
    echo $token->name;
    echo "\n";
}
"#;

    let output = interpret(source);
    assert_eq!(output.exit_status, 0);
    assert_eq!(output.stdout, b"returned\ndrop returned\n");
    assert!(output.stderr.is_empty());
    assert!(!lower_object(source).is_empty());
}
