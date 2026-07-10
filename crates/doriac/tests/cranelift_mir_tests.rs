use doriac::mir::{
    BasicBlock, BlockId, Function, FunctionId, LocalId, Operand, Program, ReturnType, Statement,
    Terminator,
};

fn assert_object(source: &str) {
    let program =
        doriac::lower_source_to_mir("test.doria", source).expect("source should lower to MIR");
    let object = doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("MIR should lower to an object without a linker");
    assert!(!object.is_empty());
}

#[test]
fn lowers_literal_return_main_to_object() {
    assert_object("function main(): int\n{\n    return 42;\n}\n");
}

#[test]
fn lowers_void_main_to_object() {
    assert_object("function main(): void\n{\n}\n");
}

#[test]
fn lowers_integer_local_arithmetic_to_object() {
    assert_object(include_str!(
        "../../../examples/native/main_int_arithmetic.doria"
    ));
}

#[test]
fn lowers_if_else_to_object() {
    assert_object(include_str!(
        "../../../examples/native/main_if_else_42.doria"
    ));
}

#[test]
fn lowers_while_to_object() {
    assert_object(include_str!(
        "../../../examples/native/main_structured_while_42.doria"
    ));
}

#[test]
fn lowers_traditional_for_to_object() {
    assert_object(include_str!("../../../examples/native/main_for_42.doria"));
}

#[test]
fn lowers_integer_range_foreach_to_object() {
    assert_object(include_str!(
        "../../../examples/native/main_foreach_range_45.doria"
    ));
}

#[test]
fn lowers_integer_helper_call_to_object() {
    assert_object(include_str!(
        "../../../examples/native/main_function_add_42.doria"
    ));
}

#[test]
fn lowers_void_helper_call_to_object() {
    assert_object(include_str!(
        "../../../examples/native/main_function_echo_hello.doria"
    ));
}

#[test]
fn lowers_string_literal_echo_to_object() {
    assert_object(include_str!(
        "../../../examples/native/main_void_hello.doria"
    ));
}

#[test]
fn lowers_string_local_concat_echo_to_object() {
    assert_object(include_str!(
        "../../../examples/native/main_string_concat_hello.doria"
    ));
}

#[test]
fn lowers_echo_inside_int_returning_helper_to_object() {
    assert_object(include_str!(
        "../../../examples/native/main_int_helper_echo_success.doria"
    ));
}

#[test]
fn lowers_recursive_calls_to_object() {
    assert_object(include_str!(
        "../../../examples/native/main_recursive_fibonacci_55.doria"
    ));
}

#[test]
fn lowers_explicit_panic_to_object() {
    assert_object(include_str!(
        "../../../examples/native/main_nested_panic_stack.doria"
    ));
}

#[test]
fn lowers_non_terminating_loop_without_executing_it() {
    assert_object(include_str!(
        "../../../examples/compile-only/main_infinite_while.doria"
    ));
}

#[test]
fn rejects_malformed_function_id() {
    let mut program = void_program();
    program.functions[0].blocks[0]
        .statements
        .push(Statement::CallVoid {
            function: FunctionId(99),
            args: Vec::new(),
        });

    let error = doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect_err("malformed FunctionId should fail before object emission");
    assert!(error.message.contains("malformed MIR"));
    assert!(error.message.contains("FunctionId function99"));
}

#[test]
fn rejects_malformed_local_id() {
    let mut program = void_program();
    program.functions[0].return_type = ReturnType::Int;
    program.functions[0].blocks[0].terminator = Terminator::Return(Operand::Local(LocalId(99)));

    let error = doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect_err("malformed LocalId should fail before object emission");
    assert!(error.message.contains("malformed MIR"));
    assert!(error.message.contains("LocalId local99"));
}

#[test]
fn rejects_malformed_block_id() {
    let mut program = void_program();
    program.functions[0].blocks[0].terminator = Terminator::Jump(BlockId(99));

    let error = doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect_err("malformed BlockId should fail before object emission");
    assert!(error.message.contains("malformed MIR"));
    assert!(error.message.contains("BlockId block99"));
}

fn void_program() -> Program {
    Program {
        functions: vec![Function {
            id: FunctionId(0),
            name: "main".to_string(),
            params: Vec::new(),
            return_type: ReturnType::Void,
            locals: Vec::new(),
            blocks: vec![BasicBlock {
                id: BlockId(0),
                statements: Vec::new(),
                terminator: Terminator::ReturnVoid,
            }],
            entry_block: BlockId(0),
        }],
        entry: FunctionId(0),
    }
}
