use doriac::diagnostics::Diagnostic;
use doriac::hir;
use doriac::numeric::FloatType;

fn check(source: &str) {
    doriac::check_source("test.doria", source)
        .unwrap_or_else(|diagnostics| panic!("Stage 13 program should check: {diagnostics:#?}"));
}

fn reject(source: &str) -> Vec<Diagnostic> {
    doriac::check_source("test.doria", source).expect_err("Stage 13 program should be rejected")
}

#[test]
fn accepts_every_integer_spelling_and_int64_alias_assignment() {
    check(
        r#"
function main(): int64
{
    int8 $a = -128;
    int16 $b = -32768;
    int32 $c = -2147483648;
    int64 $d = -9223372036854775808;
    uint8 $e = 255;
    uint16 $f = 65535;
    uint32 $g = 4294967295;
    uint64 $h = 18446744073709551615;
    int $alias = $d;
    int64 $roundTrip = $alias;
    return $roundTrip;
}
"#,
    );
}

#[test]
fn accepts_float_spellings_in_semantic_signatures() {
    check(
        r#"
function values(float32 $single, float64 $wide): float
{
    float $default = 1.0;
    return $default;
}

function single(): float32
{
    return 1.0;
}
"#,
    );
}

#[test]
fn preserves_float32_and_float64_as_distinct_semantic_types() {
    for source in [
        r#"
float64 $wide = 1.0;
float32 $narrow = $wide;
"#,
        r#"
function acceptFloat32(float32 $value): void
{
}

float64 $wide = 1.0;
acceptFloat32($wide);
"#,
        r#"
float32 $left = 1.0;
float64 $right = 2.0;
let $result = $left + $right;
"#,
    ] {
        let diagnostics = reject(source);
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "E0403"
                    || diagnostic.code == "E0408"
                    || diagnostic.code == "E0441"
            }),
            "float-width mismatch was not rejected: {diagnostics:#?}"
        );
    }
}

#[test]
fn hir_semantic_info_retains_contextual_float_widths() {
    let hir = doriac::lower_source(
        "test.doria",
        r#"
function values(): float64
{
    float32 $single = 1.0;
    float64 $wide = 2.0;
    return $wide;
}
"#,
    )
    .expect("width-correct float declarations should lower to HIR");

    let hir::Item::Function(function) = &hir.items[0] else {
        panic!("expected a function");
    };
    let hir::Stmt::VarDecl(single) = &function.body.statements[0] else {
        panic!("expected the float32 declaration");
    };
    let hir::Stmt::VarDecl(wide) = &function.body.statements[1] else {
        panic!("expected the float64 declaration");
    };

    assert_eq!(
        hir.semantic_info.float_type(single.initializer.span()),
        Some(FloatType::Float32)
    );
    assert_eq!(
        hir.semantic_info.float_type(wide.initializer.span()),
        Some(FloatType::Float64)
    );
}

#[test]
fn inferred_narrow_literals_survive_let_and_for_defaulting() {
    doriac::lower_source_to_mir(
        "test.doria",
        r#"
function main(): int
{
    int8 $x = 1;
    let $y = $x + 1;
    for (let writable $z = $x + 1; $z < 3; $z += 1) {
    }
    return Int::from($y);
}
"#,
    )
    .expect("inferred int8 literals must remain int8 through MIR lowering");
}

#[test]
fn contextual_literals_flow_through_destinations_and_integer_expressions() {
    check(
        r#"
function add(uint8 $left, uint8 $right): uint8
{
    return $left + $right;
}

function main(): int
{
    writable uint8 $value = 40 + 1;
    $value = $value + 1;
    return Int::from(add($value, 0));
}
"#,
    );
}

#[test]
fn rejects_every_required_contextual_literal_boundary() {
    for source in [
        "int8 $value = 128;",
        "int8 $value = -129;",
        "uint8 $value = -1;",
        "uint64 $value = 18446744073709551616;",
        "let $value = 18446744073709551615;",
        "mixed $value = 18446744073709551615;",
    ] {
        let diagnostics = reject(source);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0417"),
            "unexpected diagnostics for `{source}`: {diagnostics:#?}"
        );
    }
}

#[test]
fn rejects_implicit_mixed_width_operations_and_comparisons() {
    let arithmetic = reject(
        r#"
int8 $left = 1;
int16 $right = 2;
let $result = $left + $right;
"#,
    );
    assert!(arithmetic.iter().any(|diagnostic| {
        diagnostic.code == "E0441"
            && diagnostic
                .help
                .as_deref()
                .is_some_and(|help| help.contains("::from"))
    }));

    let comparison = reject(
        r#"
int8 $left = 1;
int16 $right = 1;
if ($left == $right) {
}
"#,
    );
    assert!(comparison
        .iter()
        .any(|diagnostic| diagnostic.code == "E0420"));
}

#[test]
fn rejects_float_remainder_and_remainder_assignment() {
    for source in [
        r#"
let $result = 5.0 % 2.0;
"#,
        r#"
writable float $value = 5.0;
$value %= 2.0;
"#,
    ] {
        let diagnostics = reject(source);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0441"),
            "float remainder was not rejected: {diagnostics:#?}"
        );
    }
}

#[test]
fn validates_integer_companion_conversion_intrinsics() {
    check(
        r#"
function main(): int
{
    int $source = 256;
    uint8 $runtimeChecked = UInt8::from($source);
    uint8 $literalIsStillDefaultInt = UInt8::from(256);
    int8 $same = Int8::from(Int8::from(1));
    uint64 $wide = UInt64::from($same);
    return Int64::from($wide);
}
"#,
    );

    for source in [
        "let $value = UInt8::from();",
        "let $value = UInt8::from(1, 2);",
        "let $value = UInt8::from(\"1\");",
        "let $value = UInt8::parse(1);",
    ] {
        assert!(!reject(source).is_empty());
    }
}

#[test]
fn checks_unary_bitwise_and_all_compound_assignments() {
    check(
        r#"
writable int8 $value = 1;
int8 $negative = -$value;
int8 $inverted = ~$negative;
$value += 1;
$value -= 1;
$value *= 1;
$value /= 1;
$value %= 2;
$value <<= 1;
$value >>= 1;
$value &= 1;
$value |= 1;
$value ^= 1;
$value++;
--$value;
"#,
    );

    let diagnostics = reject(
        r#"
uint8 $value = 1;
let $negative = -$value;
"#,
    );
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0440" && diagnostic.message.contains("signed integer")
    }));
}

#[test]
fn limits_process_main_but_not_helper_integer_signatures() {
    check(
        r#"
function helper(uint16 $value): uint16
{
    return $value;
}

function main(): int64
{
    return Int::from(helper(42));
}
"#,
    );

    for return_type in ["int8", "int16", "int32", "uint8", "uint64"] {
        let diagnostics = reject(&format!("function main(): {return_type} {{ return 0; }}"));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0442"));
    }
}

#[test]
fn reserves_companions_and_guides_non_doria_integer_spellings() {
    let companion = reject("class UInt8 {}");
    assert!(companion
        .iter()
        .any(|diagnostic| diagnostic.message.contains("integer companion")));

    for (spelling, expected) in [
        ("i8", "Doria uses `int8`, not `i8`"),
        ("u8", "Doria uses `uint8`, not `u8`"),
        ("uint", "Doria has no bare `uint`"),
    ] {
        let diagnostics = reject(&format!("{spelling} $value = 1;"));
        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains(expected)));
    }
}
