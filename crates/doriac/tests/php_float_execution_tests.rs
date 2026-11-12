//! Float arithmetic and conversions in PHP output agree with the interpreter
//! and both native profiles, bit for bit, over the values where a 64-bit host
//! float could differ from a Doria `float32`: subnormals, infinities, NaN,
//! signed zero and rounding ties. Float text (canonical display and `%f`) is
//! held to the same standard.

// The float literals below are chosen for their bits, and several carry more
// digits than a float keeps.
#![allow(clippy::excessive_precision)]

#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;

#[path = "common/php_execution.rs"]
mod php_execution;

use doriac::backend::NativeProfile;
use std::process::Output;

fn php(source: &str) -> Output {
    let php = doriac::compile_source_to_php("float-parity.doria", source)
        .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").unwrap()
    );
    php_execution::Script::new(&script)
        .command(&[])
        .output()
        .unwrap()
}

/// The interpreter's answer is the reference; the native profiles and PHP
/// must print the same.
fn assert_backends_agree(source: &str) -> String {
    let program = doriac::lower_source_to_mir("float-parity.doria", source)
        .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
    let interpreted = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(interpreted.exit_status, 0, "{source}");
    let expected = String::from_utf8(interpreted.stdout).unwrap();
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, &expected);
    }
    let run = php(source);
    assert_eq!(run.status.code(), Some(0), "{run:?}");
    assert_eq!(String::from_utf8_lossy(&run.stdout), expected, "{source}");
    assert!(run.stderr.is_empty(), "{run:?}");
    expected
}

/// The exact decimal text of a finite float, as a Doria literal. Doria has no
/// exponent syntax, and a binary32 value needs at most 149 fraction digits.
fn exact_literal(value: f64) -> String {
    assert!(value.is_finite());
    let text = format!("{:.1100}", value.abs());
    let text = text.trim_end_matches('0');
    let text = if text.ends_with('.') {
        format!("{text}0")
    } else {
        text.to_string()
    };
    if value.is_sign_negative() {
        format!("-{text}")
    } else {
        text
    }
}

/// A Doria expression that evaluates to `value` as a `float32` argument.
fn operand32(value: f32) -> String {
    if value.is_nan() {
        "nan()".to_string()
    } else if value == f32::INFINITY {
        "infinity()".to_string()
    } else if value == f32::NEG_INFINITY {
        "-infinity()".to_string()
    } else {
        exact_literal(f64::from(value))
    }
}

/// A Doria `bool` expression that is true exactly when `actual` holds the
/// bits of `expected`, with every NaN treated as one value.
fn holds32(actual: &str, expected: f32) -> String {
    if expected.is_nan() {
        format!("isNan({actual})")
    } else if expected == 0.0 {
        format!("isZero({actual}, {})", expected.is_sign_negative())
    } else {
        format!("same({actual}, {})", operand32(expected))
    }
}

const FLOAT32_SUPPORT: &str = r#"
function add(float32 $left, float32 $right): float32 { return $left + $right; }
function subtract(float32 $left, float32 $right): float32 { return $left - $right; }
function multiply(float32 $left, float32 $right): float32 { return $left * $right; }
function divide(float32 $left, float32 $right): float32 { return $left / $right; }
function negate(float32 $value): float32 { return -$value; }
function same(float32 $actual, float32 $expected): bool { return $actual == $expected; }
function isNan(float32 $value): bool { return $value != $value; }
function isZero(float32 $value, bool $negative): bool {
    float32 $zero = 0.0;
    float32 $one = 1.0;
    if ($value != $zero) { return false; }
    return (($one / $value) < $zero) == $negative;
}
function infinity(): float32 {
    float32 $largest = 340282346638528859811704183484516925440.0;
    return $largest + $largest;
}
function nan(): float32 {
    float32 $infinite = infinity();
    return $infinite - $infinite;
}
"#;

/// Binary32 bit patterns: both zeros, the subnormal range and its edges, one
/// and its neighbors, values that do not terminate in binary, the 2^24
/// integer boundary where sums tie, the halves and quarters of an ulp, the
/// largest values, and values whose products and quotients are subnormal.
const FLOAT32_EDGES: [u32; 26] = [
    0x0000_0000,
    0x8000_0000,
    0x0000_0001,
    0x8000_0001,
    0x007f_ffff,
    0x0080_0000,
    0x0080_0001,
    0x3f80_0000,
    0xbf80_0000,
    0x3f80_0001,
    0x3f7f_ffff,
    0x4000_0000,
    0x3fc0_0000,
    0x3eaa_aaab,
    0x3dcc_cccd,
    0x4b80_0000,
    0x4b7f_ffff,
    0x4b80_0001,
    0x3380_0000,
    0x3300_0000,
    0x7f7f_ffff,
    0xff7f_ffff,
    0x7f00_0000,
    0x1e00_0000,
    0x0c80_0000,
    0x4049_0fdb,
];

fn float32_operands() -> Vec<f32> {
    let mut operands: Vec<f32> = FLOAT32_EDGES
        .iter()
        .map(|bits| f32::from_bits(*bits))
        .collect();
    operands.extend([f32::INFINITY, f32::NEG_INFINITY, f32::NAN]);
    operands
}

#[test]
fn float32_arithmetic_rounds_to_binary32_on_every_backend() {
    // One program per pair of left operands. The fast profile's compile
    // memory grows with the square of a program's size (117 MB at 117 of
    // these statements, 1.0 GB at 936, measured 2026-11-11), so the whole
    // table as one program does not fit a development machine.
    let operands = float32_operands();
    for lefts in operands.chunks(2) {
        let mut statements = Vec::new();
        for &left in lefts {
            for &right in &operands {
                let (a, b) = (operand32(left), operand32(right));
                for (name, expected) in [
                    ("add", left + right),
                    ("subtract", left - right),
                    ("multiply", left * right),
                    ("divide", left / right),
                ] {
                    statements.push(format!(
                        "echo {};",
                        holds32(&format!("{name}({a}, {b})"), expected)
                    ));
                }
            }
            statements.push(format!(
                "echo {};",
                holds32(&format!("negate({})", operand32(left)), -left)
            ));
        }

        // The table above is computed in Rust `f32`. The interpreter must
        // agree with it before it serves as the reference for the other
        // backends.
        let printed = assert_backends_agree(&program(FLOAT32_SUPPORT, &statements));
        assert_eq!(printed, "true".repeat(statements.len()), "{printed}");
    }
}

/// A Doria statement that updates `$value`, and the same step in `f32`.
type Float32Step = (&'static str, fn(f32) -> f32);

#[test]
fn float32_updates_and_literals_round_at_every_step() {
    // Each step is computed in `f32`, so a backend that carries the running
    // value at binary64 precision drifts from the expected literals.
    let mut value = 16_777_215.0_f32;
    let mut body = format!("    writable float32 $value = {};\n", operand32(value));
    let mut checks = 0;
    for (statement, next) in [
        ("$value += 1.0;", value + 1.0),
        ("$value += 1.0;", value + 1.0 + 1.0),
        ("$value++;", value + 1.0 + 1.0 + 1.0),
        ("$value -= 0.5;", value + 1.0 + 1.0 + 1.0 - 0.5),
    ] {
        body.push_str(&format!(
            "    {statement}\n    echo {};\n",
            holds32("$value", next)
        ));
        checks += 1;
    }
    value = 0.1;
    body.push_str("    $value = 0.1;\n");
    let steps: [Float32Step; 7] = [
        ("$value *= 3.0;", |v| v * 3.0),
        ("$value /= 7.0;", |v| v / 7.0),
        ("$value += 0.2;", |v| v + 0.2),
        ("$value -= 0.3;", |v| v - 0.3),
        ("--$value;", |v| v - 1.0),
        ("$value *= $value;", |v| v * v),
        ("$value /= 0.0000001;", |v| v / 0.000_000_1),
    ];
    for (statement, apply) in steps {
        value = apply(value);
        body.push_str(&format!(
            "    {statement}\n    echo {};\n",
            holds32("$value", value)
        ));
        checks += 1;
    }
    // A float32 constant and a default hold the binary32 value of their text.
    let source = format!(
        r#"{FLOAT32_SUPPORT}
const float32 TENTH = 0.1;
function scaled(float32 $factor = 0.1): float32 {{ return multiply($factor, 3.0); }}
function main(): void {{
{body}    echo {};
    echo {};
}}
"#,
        holds32("TENTH", 0.1),
        holds32("scaled()", 0.1_f32 * 3.0),
    );
    let printed = assert_backends_agree(&source);
    assert_eq!(printed, "true".repeat(checks + 2), "{printed}");
}

#[test]
fn int_to_float_rounds_to_nearest_even_on_every_backend() {
    let mut main = String::from("function main(): void {\n");
    let values: [i64; 16] = [
        0,
        1,
        -1,
        9_007_199_254_740_992,
        9_007_199_254_740_993,
        9_007_199_254_740_995,
        -9_007_199_254_740_993,
        4_611_686_018_427_388_415,
        4_611_686_018_427_388_416,
        4_611_686_018_427_388_417,
        4_611_686_018_427_389_440,
        i64::MAX,
        i64::MAX - 511,
        i64::MAX - 512,
        i64::MIN + 1,
        i64::MIN,
    ];
    for value in values {
        let argument = if value == i64::MIN {
            "-9223372036854775807 - 1".to_string()
        } else {
            value.to_string()
        };
        main.push_str(&format!(
            "    echo Int::toFloat({argument}) == {};\n",
            exact_literal(value as f64)
        ));
    }
    main.push_str("}\n");
    let printed = assert_backends_agree(&main);
    assert_eq!(printed, "true".repeat(values.len()), "{printed}");
}

#[test]
fn float_to_int_truncates_toward_zero_on_every_backend() {
    let mut main = String::from("function main(): void {\n");
    let mut expected = String::new();
    for value in [
        0.0_f64,
        -0.0,
        0.999_999_999_999_999_9,
        -0.999_999_999_999_999_9,
        1.5,
        -1.5,
        2.5,
        -2.5,
        0.000_001,
        123_456_789.987_654_33,
        9_007_199_254_740_992.0,
        9_223_372_036_854_774_784.0,
        -9_223_372_036_854_775_808.0,
    ] {
        main.push_str(&format!(
            "    echo Float::toInt({});\n    echo \";\";\n",
            exact_literal(value)
        ));
        expected.push_str(&format!("{};", value.trunc() as i64));
    }
    main.push_str("}\n");
    let printed = assert_backends_agree(&main);
    assert_eq!(printed, expected);
}

#[test]
fn float_to_int_out_of_range_panics_identically_in_php_output() {
    for (label, value) in [
        ("above", "9223372036854775808.0"),
        ("below", "-9223372036854777856.0"),
        ("infinite", "$huge * $huge"),
        ("negative infinite", "-($huge * $huge)"),
        ("not a number", "($huge * $huge) - ($huge * $huge)"),
    ] {
        let source = format!(
            r#"
function convert(float $huge): int {{
    return Float::toInt({value});
}}
function main(): void {{ echo convert(1{zeros}.0); }}
"#,
            zeros = "0".repeat(200),
        );
        let program = doriac::lower_source_to_mir("float-parity.doria", &source)
            .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
        let interpreted = doriac::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(interpreted.exit_status, 101, "{label}");
        let reference = String::from_utf8(interpreted.stderr).unwrap();
        assert!(reference.contains("Panic[P1110]"), "{label}: {reference}");
        let run = php(&source);
        assert_eq!(run.status.code(), Some(101), "{label}: {run:?}");
        assert!(run.stdout.is_empty(), "{label}: {run:?}");
        // The same report, down to the value the panic points at.
        assert_eq!(String::from_utf8(run.stderr).unwrap(), reference, "{label}");
    }
}

/// Splits statements into small functions called from `main`, so that no one
/// function grows with the size of the table under test.
fn program(prelude: &str, statements: &[String]) -> String {
    let mut source = String::from(prelude);
    let mut main = String::from("function main(): void {\n");
    for (index, chunk) in statements.chunks(48).enumerate() {
        source.push_str(&format!("function part{index}(): void {{\n"));
        for statement in chunk {
            source.push_str(&format!("    {statement}\n"));
        }
        source.push_str("}\n");
        main.push_str(&format!("    part{index}();\n"));
    }
    main.push_str("}\n");
    source.push_str(&main);
    source
}

/// Binary64 values on both sides of every layout rule of canonical display
/// (plain digits, a leading `0.`, an exponent), values whose shortest text is
/// hard to find, and the ends of the range.
const FLOAT64_DISPLAY: [f64; 44] = [
    1.0,
    0.1,
    0.5,
    1.5,
    3.0,
    100.0,
    1e15,
    1e16,
    1e17,
    9_007_199_254_740_993.0,
    1e21,
    1e22,
    1e23,
    9.999_999_999_999_999e22,
    f64::MAX,
    f64::MIN_POSITIVE,
    5e-324,
    1e-5,
    1e-4,
    0.001,
    0.000_012_34,
    0.000_001,
    123_456.789,
    1e300,
    1e-300,
    4.35,
    0.3,
    2.675,
    1.005,
    1.0 / 3.0,
    2.0 / 3.0,
    0.300_000_000_000_000_04,
    9.5e-7,
    1_234_567_890_123_456.0,
    12_345_678_901_234_567.0,
    1e-7,
    123_456_789_012_345_680_000.0,
    9_223_372_036_854_775_808.0,
    1.5e-323,
    8.41e21,
    5e-320,
    8.988_465_674_311_58e307,
    2.225_073_858_507_201e-308,
    4_503_599_627_370_496.5,
];

/// Binary32 bit patterns chosen the same way.
const FLOAT32_DISPLAY: [u32; 24] = [
    0x0000_0001,
    0x0000_0002,
    0x007f_ffff,
    0x0080_0000,
    0x3f80_0000,
    0x3f80_0001,
    0x3dcc_cccd,
    0x3eaa_aaab,
    0x7f7f_ffff,
    0x4b80_0000,
    0x4b7f_ffff,
    0x461c_4000,
    0x5015_02f9,
    0x5d5e_0b6b,
    0x3a83_126f,
    0x3586_37bd,
    0x33d6_bf95,
    0x4974_2400,
    0x4cbe_bc20,
    0x55ae_9f4b,
    0x5863_5fa9,
    0x4049_0fdb,
    0x1e3c_e508,
    0x60ad_78ec,
];

#[test]
fn floats_print_canonical_text_on_every_backend() {
    use doriac::numeric::FloatValue;

    let mut statements = Vec::new();
    let mut expected = String::new();
    for magnitude in FLOAT64_DISPLAY {
        for value in [magnitude, -magnitude] {
            statements.push(format!("echo {}; echo \"\\n\";", exact_literal(value)));
            expected.push_str(&FloatValue::from_f64(value).display());
            expected.push('\n');
        }
    }
    for bits in FLOAT32_DISPLAY {
        for value in [f32::from_bits(bits), -f32::from_bits(bits)] {
            statements.push(format!("show({});", operand32(value)));
            expected.push_str(&FloatValue::from_f32(value).display());
            expected.push('\n');
        }
    }
    // Zeros and the values no literal can spell, at both widths, then the
    // same text through interpolation, concatenation and `%s`.
    statements.extend(
        [
            "echo 0.0; echo \"\\n\";",
            "echo -0.0; echo \"\\n\";",
            "echo huge() * huge(); echo \"\\n\";",
            "echo -(huge() * huge()); echo \"\\n\";",
            "echo huge() * huge() - huge() * huge(); echo \"\\n\";",
            "show(0.0);",
            "show(-0.0);",
            "show(infinity());",
            "show(-infinity());",
            "show(nan());",
            "together(0.1, 0.1);",
        ]
        .map(str::to_string),
    );
    expected.push_str("0\n-0\nInfinity\n-Infinity\nNaN\n0\n-0\nInfinity\n-Infinity\nNaN\n");
    expected.push_str("0.1 and 0.1; 0.1|0.1|     0.1\n");
    let prelude = format!(
        r#"{FLOAT32_SUPPORT}
function show(float32 $value): void {{ echo $value; echo "\n"; }}
function huge(): float {{ return 1{zeros}.0; }}
function together(float $wide, float32 $narrow): void {{
    echo "{{$wide}} and {{$narrow}}; " . $wide . "|" . $narrow;
    printf("|%8s\n", $narrow);
}}
"#,
        zeros = "0".repeat(200),
    );
    let printed = assert_backends_agree(&program(&prelude, &statements));
    assert_eq!(printed, expected);
}

#[test]
fn fixed_float_formatting_is_exact_on_every_backend() {
    // The first cases are ones a floating-point scaling gets wrong: 1.15 is
    // stored below 1.15, yet 1.15 * 10 rounds to 11.5 exactly; 1e40 scaled by
    // 10^6 does not fit 128 bits.
    let cases: [(f64, usize); 30] = [
        (1.15, 1),
        (1e40, 6),
        (0.25, 1),
        (0.35, 1),
        (1.005, 2),
        (0.5, 0),
        (1.5, 0),
        (2.5, 0),
        (2.0005, 3),
        (123_456.789, 3),
        (0.000_001_234, 8),
        (9.995, 2),
        (0.999_999, 3),
        (1e22, 2),
        (5e-324, 3),
        (2.675, 2),
        (1e-7, 6),
        (0.1, 20),
        (0.1, 60),
        (f64::MAX, 1),
        (0.045, 2),
        (8.345, 2),
        (-1.15, 1),
        (-0.000_4, 3),
        (99.99, 1),
        (0.5, 6),
        (4.35, 1),
        (-0.0, 2),
        (5e-324, 1080),
        (1.0, 40),
    ];
    let mut statements = Vec::new();
    let mut expected = String::new();
    for (value, precision) in cases {
        statements.push(format!(
            "printf(\"%.{precision}f\\n\", {});",
            exact_literal(value)
        ));
        expected.push_str(&format!("{value:.precision$}\n"));
    }
    // The default precision is six; a float32 prints its own exact value;
    // width pads with spaces or, after the sign, with zeros.
    statements.extend(
        [
            "printf(\"%f\\n\", 1.5);",
            "narrow(0.1);",
            "printf(\"[%12.3f][%-12.3f][%012.3f][%012.3f][%3.1f]\\n\", 3.14159, 3.14159, -3.14159, 2.5, 123.25);",
            "printf(\"[%08.1f][%-8.1f][%08.1f]\\n\", huge() * huge(), -(huge() * huge()), huge() * huge() - huge() * huge());",
        ]
        .map(str::to_string),
    );
    expected.push_str("1.500000\n0.100000001490116119384765625000\n");
    expected.push_str("[       3.142][3.142       ][-0000003.142][00000002.500][123.2]\n");
    expected.push_str("[Infinity][-Infinity][00000NaN]\n");
    let prelude = format!(
        r#"
function narrow(float32 $value): void {{ printf("%.30f\n", $value); }}
function huge(): float {{ return 1{zeros}.0; }}
"#,
        zeros = "0".repeat(200),
    );
    let printed = assert_backends_agree(&program(&prelude, &statements));
    assert_eq!(printed, expected);
}
