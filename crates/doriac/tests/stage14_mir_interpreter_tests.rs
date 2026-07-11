use doriac::mir::{ReturnType, ScalarType, Type};
use doriac::mir_interpreter::{interpret, InterpreterOutput};
use doriac::numeric::{FloatType, IntegerType};

fn lower(source: &str) -> doriac::mir::Program {
    doriac::lower_source_to_mir("stage14_mir.doria", source)
        .unwrap_or_else(|diagnostics| panic!("source should lower: {diagnostics:#?}"))
}

fn run(source: &str) -> InterpreterOutput {
    interpret(&lower(source)).expect("well-formed Stage 14 MIR should interpret")
}

#[test]
fn mir_uses_one_scalar_path_for_float_bool_and_mixed_calls() {
    let program = lower(
        r#"
function choose(int $whole, float32 $narrow, float $wide, bool $ready): float
{
    if ($ready and $narrow < 2.0) {
        return $wide + Int::toFloat($whole);
    }
    return 0.0;
}

function main(): int
{
    return Float::toInt(choose(40, 1.0, 2.0, true));
}
"#,
    );
    let choose = &program.functions[0];
    assert_eq!(
        choose.return_type,
        ReturnType::Value(ScalarType::Float(FloatType::Float64))
    );
    assert_eq!(
        choose
            .locals
            .iter()
            .map(|local| local.ty)
            .collect::<Vec<_>>(),
        vec![
            Type::Scalar(ScalarType::Integer(IntegerType::Int64)),
            Type::Scalar(ScalarType::Float(FloatType::Float32)),
            Type::Scalar(ScalarType::Float(FloatType::Float64)),
            Type::Scalar(ScalarType::Bool),
        ]
    );
    let dump = program.to_string();
    assert!(dump.contains("$narrow: float32"), "{dump}");
    assert!(dump.contains("$wide: float"), "{dump}");
    assert!(dump.contains("$ready: bool"), "{dump}");
    assert_eq!(interpret(&program).unwrap().exit_status, 42);
}

#[test]
fn interpreter_preserves_float32_and_float64_precision() {
    let output = run(r#"
function main(): int
{
    writable float32 $narrow = 16777216.0;
    $narrow += 1.0;
    writable float $wide = 16777216.0;
    $wide += 1.0;
    if ($narrow == 16777216.0 and $wide == 16777217.0) {
        return 42;
    }
    return 0;
}
"#);
    assert_eq!(output.exit_status, 42);
}

#[test]
fn interpreter_follows_ieee_division_nan_and_signed_zero() {
    let output = run(r#"
function main(): int
{
    float $positive = 1.0 / 0.0;
    float $negativeZero = -0.0;
    float $negative = 1.0 / $negativeZero;
    float $nan = 0.0 / 0.0;
    if ($positive > 0.0
        and $negative < 0.0
        and $nan != $nan
        and not ($nan == $nan)
        and not ($nan < 1.0)
        and 0.0 == $negativeZero) {
        return 42;
    }
    return 0;
}
"#);
    assert_eq!(output.exit_status, 42);
}

#[test]
fn bool_value_short_circuits_while_xor_is_eager() {
    let short = run(r#"
function left(): bool { echo "L"; return false; }
function right(): bool { echo "R"; return true; }
function main(): int
{
    bool $value = left() and right();
    if (not $value) { return 42; }
    return 0;
}
"#);
    assert_eq!(short.stdout, b"L");
    assert_eq!(short.exit_status, 42);

    let eager = run(r#"
function left(): bool { echo "L"; return false; }
function right(): bool { echo "R"; return true; }
function main(): int
{
    bool $value = left() xor right();
    if ($value) { return 42; }
    return 0;
}
"#);
    assert_eq!(eager.stdout, b"LR");
    assert_eq!(eager.exit_status, 42);
}

#[test]
fn recursive_bool_helpers_work_in_value_and_condition_positions() {
    let output = run(r#"
function isZero(int $value): bool
{
    if ($value == 0) { return true; }
    return isZero($value - 1);
}
function main(): int
{
    bool $answer = isZero(10);
    if ($answer) { return 42; }
    return 0;
}
"#);
    assert_eq!(output.exit_status, 42);
}

#[test]
fn float_to_int_truncates_and_accepts_negative_minimum() {
    assert_eq!(
        run("function main(): int { return Float::toInt(42.9); }").exit_status,
        42
    );
    assert_eq!(
        run("function main(): int { return -Float::toInt(-42.9); }").exit_status,
        42
    );
    assert_eq!(
        run(r#"
function minimum(): int { return Float::toInt(-9223372036854775808.0); }
function main(): int { if (minimum() == -9223372036854775808) { return 42; } return 0; }
"#,)
        .exit_status,
        42
    );
}

#[test]
fn invalid_float_to_int_conversions_use_doria_panic_contract() {
    for expression in [
        "0.0 / 0.0",
        "1.0 / 0.0",
        "9223372036854775807.0",
        "-1.0 / 0.0",
    ] {
        let source = format!(
            "function convert(): int {{ return Float::toInt({expression}); }}\nfunction main(): int {{ return convert(); }}"
        );
        let output = run(&source);
        assert_eq!(output.exit_status, 101);
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            "Panic: float-to-integer conversion out of range\nStack Trace:\n  at convert\n  at main\n"
        );
    }
}
