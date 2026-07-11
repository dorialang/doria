use doriac::mir::{
    IntegerBinaryOp, IntegerExpression, ReturnType, Rvalue, ScalarType, Statement, Type,
    ValueExpression,
};
use doriac::mir_interpreter::{interpret, InterpreterOutput};
use doriac::numeric::IntegerType;

fn lower(source: &str) -> doriac::mir::Program {
    doriac::lower_source_to_mir("stage13_mir_test.doria", source)
        .unwrap_or_else(|diagnostics| panic!("source should lower to MIR: {diagnostics:?}"))
}

fn run(source: &str) -> InterpreterOutput {
    interpret(&lower(source)).expect("well-formed Stage 13 MIR should interpret")
}

#[test]
fn carries_fixed_width_types_and_uint64_max_through_calls() {
    let source = r#"
function identity(uint64 $value): uint64
{
    return $value;
}

function main(): int
{
    uint64 $maximum = 18446744073709551615;

    if (identity($maximum) == 18446744073709551615) {
        return 42;
    }

    return 0;
}
"#;
    let program = lower(source);

    assert_eq!(
        program.functions[0].return_type,
        ReturnType::Value(ScalarType::Integer(IntegerType::UInt64))
    );
    assert_eq!(
        program.functions[0].locals[0].ty,
        Type::Scalar(ScalarType::Integer(IntegerType::UInt64))
    );
    assert_eq!(
        program.functions[1].locals[0].ty,
        Type::Scalar(ScalarType::Integer(IntegerType::UInt64))
    );
    let dump = program.to_string();
    assert!(dump.contains("18446744073709551615: uint64"), "{dump}");

    let output = interpret(&program).expect("uint64 transport should interpret");
    assert_eq!(output.exit_status, 42);
    assert!(output.stderr.is_empty());
}

#[test]
fn typed_literal_binary_expression_keeps_uint16_context_in_mir() {
    let program = lower(
        r#"
function answer(): uint16
{
    uint16 $value = 84 / 2;
    return $value;
}

function main(): int
{
    return Int::from(answer());
}
"#,
    );

    assert!(matches!(
        &program.functions[0].blocks[0].statements[0],
        Statement::AssignLocal {
            value: Rvalue::Value(ValueExpression::Integer(IntegerExpression::Binary {
                ty: IntegerType::UInt16,
                op: IntegerBinaryOp::Divide,
                left,
                right,
            })),
            ..
        } if left.ty() == IntegerType::UInt16 && right.ty() == IntegerType::UInt16
    ));
    assert_eq!(
        interpret(&program)
            .expect("typed uint16 arithmetic should interpret")
            .exit_status,
        42
    );
}

#[test]
fn interprets_division_remainder_shifts_bitwise_and_unary_operations() {
    let output = run(r#"
function main(): int
{
    int8 $negative = -85;
    int8 $two = 2;
    int8 $quotient = $negative / $two;
    int8 $remainder = $negative % $two;
    int8 $minimum = -128;
    int8 $minusOne = -1;
    int8 $minimumRemainder = $minimum % $minusOne;
    int8 $shiftedNegative = -84;
    int8 $one = 1;
    int8 $signedShift = $shiftedNegative >> $one;
    uint8 $bits = 170;
    uint8 $mask = 63;
    uint8 $shiftedUnsigned = 21;
    uint8 $unsignedOne = 1;
    uint8 $unsignedShift = $shiftedUnsigned << $unsignedOne;
    uint8 $complement = ~$bits;

    if ($quotient == -42
        and $remainder == -1
        and $minimumRemainder == 0
        and $signedShift == -42
        and $unsignedShift == 42
        and $complement == 85) {
        return Int::from($bits & $mask);
    }

    return 0;
}
"#);

    assert_eq!(output.exit_status, 42);
    assert!(output.stderr.is_empty());
}

#[test]
fn interprets_every_stage_13_compound_assignment_and_increment() {
    let output = run(r#"
function main(): int
{
    writable uint8 $value = 1;
    $value += 2;
    $value *= 16;
    $value -= 6;
    $value /= 2;
    $value %= 20;
    $value <<= 5;
    $value >>= 1;
    $value |= 32;
    $value &= 42;
    $value ^= 10;
    $value++;
    $value--;

    return Int::from($value);
}
"#);

    assert_eq!(output.exit_status, 42);
    assert!(output.stderr.is_empty());
}

#[test]
fn inclusive_uint8_range_stops_at_maximum_without_increment_overflow() {
    let output = run(r#"
function main(): int
{
    writable int $count = 0;

    foreach (254..255 as uint8 $value) {
        $count += 1;
    }

    if ($count == 2) {
        return 42;
    }

    return 0;
}
"#);

    assert_eq!(output.exit_status, 42);
    assert!(output.stderr.is_empty());
}

#[test]
fn stage_13_runtime_failures_preserve_exact_messages_and_helper_frames() {
    let cases = [
        (
            "uint8 $one = 1; writable uint8 $value = 255; return $value + $one;",
            "uint8",
            "integer overflow during addition",
        ),
        (
            "uint8 $one = 1; writable uint8 $value = 0; return $value - $one;",
            "uint8",
            "integer overflow during subtraction",
        ),
        (
            "uint8 $two = 2; writable uint8 $value = 128; return $value * $two;",
            "uint8",
            "integer overflow during multiplication",
        ),
        (
            "writable int8 $value = -128; return -$value;",
            "int8",
            "integer overflow during negation",
        ),
        (
            "return -(-128);",
            "int8",
            "integer overflow during negation",
        ),
        (
            "int8 $zero = 0; writable int8 $value = 42; return $value / $zero;",
            "int8",
            "integer division by zero",
        ),
        (
            "int8 $minusOne = -1; writable int8 $value = -128; return $value / $minusOne;",
            "int8",
            "integer division overflow",
        ),
        (
            "uint8 $zero = 0; writable uint8 $value = 42; return $value % $zero;",
            "uint8",
            "integer remainder by zero",
        ),
        (
            "int8 $count = -1; writable int8 $value = 1; return $value << $count;",
            "int8",
            "integer shift count out of range",
        ),
        (
            "uint8 $count = 8; writable uint8 $value = 1; return $value << $count;",
            "uint8",
            "integer shift count out of range",
        ),
        (
            "uint8 $count = 9; writable uint8 $value = 1; return $value >> $count;",
            "uint8",
            "integer shift count out of range",
        ),
        (
            "int $value = 256; return UInt8::from($value);",
            "uint8",
            "integer conversion out of range",
        ),
    ];

    for (body, return_type, message) in cases {
        let source = format!(
            "function fail(): {return_type}\n{{\n    {body}\n}}\n\nfunction main(): int\n{{\n    return Int::from(fail());\n}}\n"
        );
        let output = run(&source);
        assert_eq!(output.exit_status, 101, "case: {message}");
        assert_eq!(
            String::from_utf8(output.stderr).expect("panic stderr should be UTF-8"),
            format!("Panic: {message}\nStack Trace:\n  at fail\n  at main\n"),
            "case: {message}"
        );
    }
}
