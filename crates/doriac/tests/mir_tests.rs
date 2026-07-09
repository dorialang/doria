use doriac::backend::{BackendOutput, BackendTarget};
use doriac::mir::{
    BasicBlock, BinaryOp, BlockId, CompareOp, Condition, ConditionBinaryOp, Function, FunctionId,
    IntExpression, Local, LocalId, Operand, Program, ReturnType, Rvalue, Statement, Terminator,
    Type,
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
        .expect_err("source should be outside Stage 11c MIR coverage")
}

fn unsupported_after_parsing(source: &str) -> Vec<doriac::diagnostics::Diagnostic> {
    let ast = doriac::parse_source("test.doria", source).expect("source should parse");
    let hir = doriac::lowering::lower_program(&ast);
    doriac::mir_lowering::lower_program(&hir)
        .expect_err("HIR should be outside Stage 11c MIR coverage")
}

fn assert_stage_11c_unsupported(diagnostics: &[doriac::diagnostics::Diagnostic], detail: &str) {
    assert_eq!(diagnostics[0].code, "M1101");
    assert!(
        diagnostics[0]
            .message
            .contains("unsupported MIR Stage 11c coverage"),
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
        functions: vec![Function {
            id: FunctionId(0),
            name: "main".to_string(),
            return_type: ReturnType::Int,
            locals: vec![Local {
                id: LocalId(0),
                name: "unassigned".to_string(),
                ty: Type::Int,
                writable: false,
                synthetic: true,
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
                    terminator: Terminator::Return(Operand::Int(then_status)),
                },
                BasicBlock {
                    id: BlockId(2),
                    statements: Vec::new(),
                    terminator: Terminator::Return(Operand::Int(else_status)),
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
        left: IntExpression::Use(Operand::Local(LocalId(0))),
        right: IntExpression::Use(Operand::Int(0)),
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
fn rejects_unsupported_division_as_mir_coverage() {
    let diagnostics = unsupported(
        r#"function main(): int
{
    return 84 / 2;
}
"#,
    );

    assert_stage_11c_unsupported(&diagnostics, "division and modulo");
}

#[test]
fn rejects_comparison_result_as_runtime_value() {
    let diagnostics = unsupported(
        r#"function main(): void
{
    let $ok = 40 < 42;
}
"#,
    );

    assert_stage_11c_unsupported(&diagnostics, "condition-only");
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

    assert_stage_11c_unsupported(&diagnostics, "string locals");
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

    assert_stage_11c_unsupported(&diagnostics, "string concatenation");
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

    assert_stage_11c_unsupported(&diagnostics, "no helper functions");
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
                left: IntExpression::Binary {
                    op: BinaryOp::Add,
                    left: Box::new(IntExpression::Use(Operand::Int(40))),
                    right: Box::new(IntExpression::Use(Operand::Int(2))),
                },
                right: IntExpression::Use(Operand::Int(42)),
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
        Terminator::Return(Operand::Local(LocalId(0)))
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
    assert_eq!(blocks[1].terminator, Terminator::Return(Operand::Int(42)));
    assert_eq!(blocks[2].terminator, Terminator::Return(Operand::Int(0)));
}

#[test]
fn lowers_else_if_chain_to_nested_branch_blocks() {
    let program = lower(include_str!(
        "../../../examples/debug/main_else_if_42.doria"
    ));
    let blocks = &program.functions[0].blocks;

    assert!(matches!(blocks[0].terminator, Terminator::Branch { .. }));
    assert!(matches!(blocks[2].terminator, Terminator::Branch { .. }));
    assert_eq!(blocks[3].terminator, Terminator::Return(Operand::Int(42)));
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
            value: Rvalue::Use(Operand::Int(42)),
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
            left: Box::new(Condition::Bool(false)),
            right: Box::new(condition_that_reads_unassigned_local()),
        },
        0,
        42,
    );
    let or = conditional_program(
        Condition::Binary {
            op: ConditionBinaryOp::Or,
            left: Box::new(Condition::Bool(true)),
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
            left: Box::new(Condition::Bool(true)),
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
fn interpreter_limits_malformed_mir_control_flow() {
    let program = Program {
        functions: vec![Function {
            id: FunctionId(0),
            name: "main".to_string(),
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

    let error = doriac::mir_interpreter::interpret(&program)
        .expect_err("malformed cyclic MIR should hit the defensive limit");
    assert_eq!(
        error.message,
        "MIR interpreter exceeded Stage 11c step limit"
    );
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
    assert_stage_11c_unsupported(&truthiness, "truthiness");

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
    assert_stage_11c_unsupported(&call, "calls in conditions");
}

#[test]
fn rejects_loops_as_stage_11c_mir_coverage() {
    let diagnostics = unsupported(
        r#"function main(): void
{
    while (false) {
    }
}
"#,
    );

    assert_stage_11c_unsupported(&diagnostics, "loops");
}

#[test]
fn mirrors_native_smoke_exit_for_stage_11c_if_shapes_without_linker() {
    for source in [
        include_str!("../../../examples/debug/main_if_return_42.doria"),
        include_str!("../../../examples/debug/main_if_assignment_42.doria"),
    ] {
        let hir = doriac::lower_source("test.doria", source).expect("source should lower to HIR");
        let native_exit = doriac::codegen_native::validate_stage_2d(&hir)
            .expect("native smoke validator should already accept this source");
        let mir_exit = interpret(source).exit_status;

        assert_eq!(native_exit, 42);
        assert_eq!(mir_exit, native_exit);
    }
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
