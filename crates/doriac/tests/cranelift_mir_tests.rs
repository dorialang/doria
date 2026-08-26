use doriac::mir::{
    BasicBlock, BlockId, FloatBinaryOp, FloatExpression, Function, FunctionId, IntegerExpression,
    LocalId, Operand, Program, ReturnType, Rvalue, ScalarType, Statement, Terminator, Type,
    ValueExpression,
};
use doriac::numeric::{FloatType, FloatValue, IntegerType};

fn assert_object(source: &str) {
    let program =
        doriac::lower_source_to_mir("test.doria", source).expect("source should lower to MIR");
    let object = doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("MIR should lower to an object without a linker");
    assert!(!object.is_empty());
}

fn object_contains(object: &[u8], symbol: &str) -> bool {
    object
        .windows(symbol.len())
        .any(|window| window == symbol.as_bytes())
}

#[test]
fn closure_objects_use_static_descriptors_and_escape_selected_environments() {
    let local = doriac::lower_source_to_mir(
        "cranelift-local-closure.doria",
        r#"
function main(): void
{
    let $value = 42;
    let $callback = fn() with ($value) => $value;
    echo "{$callback()}\n";
}
"#,
    )
    .expect("nonescaping closure should lower");
    let local_object = doriac::codegen_cranelift::lower_mir_to_object(&local)
        .expect("nonescaping closure should lower to an object");
    assert!(object_contains(
        &local_object,
        "__doria_closure_descriptor_0"
    ));
    assert!(object_contains(
        &local_object,
        "__doria_drop_closure_environment_0"
    ));
    assert!(!object_contains(
        &local_object,
        "dr_v1_closure_environment_allocate"
    ));

    let escaping = doriac::lower_source_to_mir(
        "cranelift-escaping-closure.doria",
        r#"
function bind(string $value): function(): string
{
    return fn() with (take $value) => $value;
}

function main(): void
{
    let $callback = bind("owned");
    echo $callback() . "\n";
}
"#,
    )
    .expect("escaping closure should lower");
    let escaping_object = doriac::codegen_cranelift::lower_mir_to_object(&escaping)
        .expect("escaping closure should lower to an object");
    assert!(object_contains(
        &escaping_object,
        "dr_v1_closure_environment_allocate"
    ));
    assert!(object_contains(
        &escaping_object,
        "dr_v1_closure_environment_free"
    ));
    for forbidden in ["closure_retain", "closure_release", "closure_registry"] {
        assert!(!object_contains(&escaping_object, forbidden));
    }
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
fn lowers_stage_18_expression_interpolation_to_object() {
    for source in [
        include_str!("../../../examples/native/main_expression_interpolation.doria"),
        include_str!("../../../examples/native/main_expression_interpolation_order.doria"),
    ] {
        assert_object(source);
    }
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
fn lowers_complete_stage17_io_and_format_mir_to_object() {
    for source in [
        include_str!("../../../examples/native/main_read_line_echo.doria"),
        include_str!("../../../examples/native/main_file_copy.doria"),
        include_str!("../../../examples/native/main_sprintf_matrix.doria"),
        include_str!("../../../examples/native/main_printf_42.doria"),
        include_str!("../../../examples/native/main_write_stderr.doria"),
        include_str!("../../../examples/native/main_missing_file_panic.doria"),
        r#"
function identity(?string $value): ?string { return $value; }
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let $line = identity(read_line());
    if ($line != null) { echo $line; }
}
"#,
    ] {
        assert_object(source);
    }
}

#[test]
fn lowers_stage_13_integer_widths_operators_and_conversions_to_object() {
    assert_object(
        r#"
function countdown(int8 $value): int8
{
    if ($value == 0) {
        return 42;
    }
    return countdown($value - 1);
}

function mix(uint8 $input): uint8
{
    writable uint8 $value = $input;
    $value += 3;
    $value *= 4;
    $value /= 2;
    $value %= 7;
    $value <<= 2;
    $value >>= 1;
    $value |= 8;
    $value ^= 1;
    $value &= 15;
    $value -= 1;
    $value++;
    $value--;
    return $value;
}

function identityInt32(int32 $value): int32
{
    return $value;
}

function identityUInt32(uint32 $value): uint32
{
    return $value;
}

function identityInt64(int64 $value): int64
{
    return $value;
}

function identityUInt64(uint64 $value): uint64
{
    return $value;
}

function signedMath(int16 $value): int16
{
    let $negated = -$value;
    let $restored = -$negated;
    return ($restored / 3) * 3 + ($restored % 3);
}

function quotient(int16 $left, int16 $right): int16
{
    return $left / $right;
}

function remainder(int16 $left, int16 $right): int16
{
    return $left % $right;
}

function main(): int
{
    uint64 $maximum = 18446744073709551615;
    uint16 $wide = UInt16::from(mix(5));
    uint8 $back = UInt8::from($wide);
    int8 $negative = -1;
    int16 $signedWide = Int16::from($negative);
    int16 $unsignedWide = Int16::from($back);
    if (countdown(2) == 42
        && $back == 12
        && $signedWide == -1
        && $unsignedWide == 12
        && signedMath(-7) == -7
        && quotient(-7, 3) == -2
        && remainder(-7, 3) == -1
        && (~$back & 15) == 3
        && (-8 >> 2) == -2
        && identityInt32(2147483647) == 2147483647
        && identityUInt32(4294967295) == 4294967295
        && identityInt64(-9223372036854775808) == -9223372036854775808
        && identityUInt64($maximum) == 18446744073709551615
        && $maximum > 9223372036854775807) {
        return 42;
    }
    return 0;
}
"#,
    );
}

#[test]
fn lowers_stage_14_float_bool_calls_short_circuit_and_conversions_to_object() {
    for source in [
        r#"
function choose(float32 $left, float32 $right): float32
{
    return -(($left + $right) * 2.0 / 4.0);
}
function main(): int
{
    if (choose(20.0, 22.0) < 0.0) { return 42; }
    return 0;
}
"#,
        r#"
function identity(float $value): float { return $value; }
function main(): int { return Float::toInt(identity(Int::toFloat(42))); }
"#,
        r#"
function left(): bool { return false; }
function right(): bool { return true; }
function combine(bool $left, bool $right): bool
{
    return ($left and $right) or ($left xor $right);
}
function main(): int
{
    bool $answer = combine(left(), right());
    if ($answer) { return 42; }
    return 0;
}
"#,
        r#"
function convert(float $value): int { return Float::toInt($value); }
function main(): int { return convert(0.0 / 0.0); }
"#,
    ] {
        assert_object(source);
    }
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
            span: Default::default(),
        });

    let error = doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect_err("malformed FunctionId should fail before object emission");
    assert!(error.message.contains("malformed MIR"));
    assert!(error.message.contains("FunctionId function99"));
}

#[test]
fn rejects_malformed_local_id() {
    let mut program = void_program();
    program.functions[0].return_type = ReturnType::Value(Type::Scalar(
        doriac::mir::ScalarType::Integer(IntegerType::Int64),
    ));
    program.functions[0].blocks[0].terminator =
        Terminator::Return(Rvalue::Value(doriac::mir::ValueExpression::Integer(
            IntegerExpression::use_operand(IntegerType::Int64, Operand::Local(LocalId(99))),
        )));

    let error = doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect_err("malformed LocalId should fail before object emission");
    assert!(error.message.contains("malformed MIR"));
    assert!(error.message.contains("LocalId local99"));
}

#[test]
fn rejects_non_int64_process_main_return_type() {
    let mut program = void_program();
    program.functions[0].return_type = ReturnType::Value(Type::Scalar(
        doriac::mir::ScalarType::Integer(IntegerType::UInt8),
    ));

    let error = doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect_err("narrow process entry return should fail before object emission");
    assert!(error.message.contains("malformed MIR"));
    assert!(error
        .message
        .contains("entry function must return void or int/int64"));
}

#[test]
fn rejects_mixed_width_float_binary_operands() {
    let mut program = void_program();
    program.functions.push(Function {
        id: FunctionId(1),
        name: "mixedWidth".to_string(),
        source_span: Default::default(),
        method: None,
        receiver_mode: None,
        params: Vec::new(),
        parameter_modes: Vec::new(),
        return_type: ReturnType::Value(Type::Scalar(ScalarType::Float(FloatType::Float64))),
        return_borrow: None,
        checked_effects: Vec::new(),
        locals: Vec::new(),
        blocks: vec![BasicBlock {
            id: BlockId(0),
            statements: Vec::new(),
            terminator: Terminator::Return(Rvalue::Value(ValueExpression::Float(
                FloatExpression::Binary {
                    ty: FloatType::Float64,
                    op: FloatBinaryOp::Add,
                    left: Box::new(FloatExpression::constant(FloatValue::from_f32(1.0))),
                    right: Box::new(FloatExpression::constant(FloatValue::from_f64(2.0))),
                },
            ))),
        }],
        entry_block: BlockId(0),
        closure: None,
    });

    let error = doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect_err("mixed-width float operands should fail before object emission");
    assert!(error.message.contains("malformed MIR"));
    assert!(error
        .message
        .contains("float binary expression has float32 and float operands"));
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
        sources: vec![doriac::mir::SourceUnit::standalone("<test>", "")],
        packages: vec![doriac::mir::PackageUnit::standalone()],
        selected_target: doriac::mir::SelectedTarget::standalone("<test>"),
        source: doriac::source::SourceFile::new("<test>", ""),
        compilation_context: doriac::names::CompilationContext::standalone("<test>"),
        namespace: None,
        global_symbols: doriac::names::GlobalSymbolFacts::default(),
        enums: vec![],
        classes: vec![],
        collection_types: vec![],
        statics: vec![],
        error_descriptors: Vec::new(),
        error_origins: Vec::new(),
        function_types: Vec::new(),
        closure_descriptors: Vec::new(),
        closure_environment_layouts: Vec::new(),
        functions: vec![Function {
            id: FunctionId(0),
            name: "main".to_string(),
            source_span: Default::default(),
            method: None,
            receiver_mode: None,
            params: Vec::new(),
            parameter_modes: Vec::new(),
            return_type: ReturnType::Void,
            return_borrow: None,
            checked_effects: Vec::new(),
            locals: Vec::new(),
            blocks: vec![BasicBlock {
                id: BlockId(0),
                statements: Vec::new(),
                terminator: Terminator::ReturnVoid,
            }],
            entry_block: BlockId(0),
            closure: None,
        }],
        selected_entry: Some(FunctionId(0)),
        entry: FunctionId(0),
    }
}
