use doriac::diagnostics::Diagnostic;

fn check(source: &str) {
    doriac::check_source("stage14_semantic.doria", source)
        .unwrap_or_else(|diagnostics| panic!("Stage 14 program should check: {diagnostics:#?}"));
}

fn reject(source: &str) -> Vec<Diagnostic> {
    doriac::check_source("stage14_semantic.doria", source)
        .expect_err("Stage 14 program should be rejected")
}

#[test]
fn accepts_float_aliases_contextual_literals_and_runtime_operations() {
    check(
        r#"
function narrow(float32 $value): float32
{
    writable float32 $result = $value + 1.0;
    $result *= 2.0;
    $result++;
    $result--;
    return -$result / 2.0;
}

function wide(float64 $value): float
{
    float $alias = $value;
    return $alias + 0.1;
}

function main(): int
{
    float32 $a = 0.1;
    float $b = 0.1;
    float64 $c = $b;
    if (narrow($a) < 0.0 and wide($c) > 0.0) {
        return 42;
    }
    return 0;
}
"#,
    );
}

#[test]
fn rejects_implicit_numeric_conversions_and_invalid_float_operators() {
    for source in [
        "float $value = 1;",
        "float32 $value = 1;",
        "int $value = 1.0;",
        "float32 $small = 1.0; float $wide = $small;",
        "float $wide = 1.0; float32 $small = $wide;",
        "int $left = 1; float $right = 2.0; let $value = $left + $right;",
        "float32 $left = 1.0; float $right = 2.0; let $value = $left < $right;",
        "float $left = 5.0; float $right = 2.0; let $value = $left % $right;",
        "float $left = 5.0; float $right = 2.0; let $value = $left << $right;",
        "float $left = 5.0; let $value = ~$left;",
        "writable float $left = 5.0; $left %= 2.0;",
    ] {
        assert!(!reject(source).is_empty(), "source should fail: {source}");
    }
}

#[test]
fn accepts_bool_values_calls_assignment_and_short_circuit_operators() {
    check(
        r#"
function identity(bool $value): bool
{
    return $value;
}

function main(): int
{
    writable bool $ready = false;
    $ready = identity(true) and not false;
    bool $either = $ready or false;
    bool $different = $either xor false;
    bool $same = $different == true;
    if ($same) {
        return 42;
    }
    return 0;
}
"#,
    );
}

#[test]
fn rejects_bool_numeric_behavior_truthiness_and_nonportable_main() {
    for source in [
        "bool $value = 1;",
        "int $value = true;",
        "bool $a = true; bool $b = false; let $value = $a < $b;",
        "bool $a = true; let $value = $a + true;",
        "writable bool $a = true; $a++;",
        "function main(): int { if (1) { return 42; } return 0; }",
        "function main(): int { if (0.0) { return 42; } return 0; }",
        "function main(): bool { return true; }",
        "function main(): float { return 0.0; }",
        "function main(): float32 { return 0.0; }",
    ] {
        assert!(!reject(source).is_empty(), "source should fail: {source}");
    }
}

#[test]
fn validates_only_the_accepted_cross_kind_intrinsics() {
    check(
        r#"
function convert(int $value): int
{
    float $wide = Int::toFloat($value);
    return Float::toInt($wide);
}

function main(): int
{
    return convert(42);
}
"#,
    );

    for source in [
        "float $value = Int::toFloat(1.0);",
        "int $value = Float::toInt(1);",
        "int $value = Float32::toInt(1.0);",
        "float32 $value = Int::toFloat32(1);",
        "float32 $value = Float32::from(1.0);",
        "float $value = Float64::from(1.0);",
    ] {
        assert!(!reject(source).is_empty(), "source should fail: {source}");
    }
}

#[test]
fn reserves_float_and_bool_companion_names() {
    for name in ["Float", "Float32", "Float64", "Bool"] {
        let source = format!("class {name} {{}}");
        assert!(!reject(&source).is_empty(), "{name} should be reserved");
    }
}

#[test]
fn rejects_nonzero_float_literals_that_round_to_infinity() {
    let huge = format!("float32 $value = {}.0;", "9".repeat(80));
    assert!(reject(&huge)
        .iter()
        .any(|diagnostic| diagnostic.code == "E0444"));
}
