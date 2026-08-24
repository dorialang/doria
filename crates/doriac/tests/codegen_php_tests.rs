use doriac::backend::{BackendOutput, BackendTarget};
use doriac::hir;
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn emits_php_for_simple_program() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    let writable $count = 0;
    $count = 1;
    echo $count;
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.starts_with("<?php"));
    assert!(php.contains("$count = 0;"));
    assert!(php.contains("$count = 1;"));
    assert!(php.contains("__doria_write_stdout(__doria_display($count), "));
}

#[test]
fn php_backend_emits_native_unit_and_backed_enums() {
    let php = doriac::compile_source_to_php(
        "stage27.doria",
        r#"
enum Status { case Draft; case Published; }
enum Priority: int { case Low = 1; case High = 10; }
enum Transport: string { case Road = "road"; case Rail = "rail"; }
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    Status $status = Status::Draft;
    Priority $priority = Priority::High;
    Transport $transport = Transport::Rail;
    echo $status == Status::Draft;
    echo " {$priority->value} {$transport->value}\n";
}
"#,
    )
    .expect("unit and backed enums should lower to PHP native enums");

    assert!(php.contains("enum Status"));
    assert!(php.contains("case Draft;"));
    assert!(php.contains("enum Priority: int"));
    assert!(php.contains("case High = 10;"));
    assert!(php.contains("enum Transport: string"));
    assert!(php.contains("case Rail = 'rail';") || php.contains("case Rail = \"rail\";"));

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\nmain();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = Command::new("php")
        .arg("-d")
        .arg("display_errors=1")
        .arg("-r")
        .arg(script)
        .output()
        .expect("generated enum PHP should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"true 10 rail\n");
}

#[test]
fn php_backend_executes_core_match_payloads_narrowing_conditions_and_ternary() {
    let php = doriac::compile_source_to_php(
        "stage28.doria",
        r#"
enum Delivery { case Waiting; case Sent(string $reference); }
class Note { function __construct(string $text) {} }
enum NoteResult { case Found(Note $note); case Missing; }
function condition(string $name, bool $value): bool throws Doria\Std\Io\IoError { echo $name; return $value; }
function describe(Delivery $delivery): string throws Doria\Std\Io\IoError
{
    return match ($delivery) {
        Delivery::Waiting => "waiting",
        Delivery::Sent($reference) if condition("g", false) => "wrong",
        Delivery::Sent($reference) => "sent {$reference}",
    };
}
function consume(): string
{
    NoteResult $result = NoteResult::Found(new Note("owned"));
    return match (take $result) {
        NoteResult::Found($note) if $note->text == "skip" => "wrong",
        NoteResult::Found($note) => $note->text,
        NoteResult::Missing => "missing",
    };
}
function inspect(mixed $value): string
{
    return match ($value) {
        bool $flag => "bool {$flag}",
        string $text => "string {$text}",
        default => "other",
    };
}
function choose(): string throws Doria\Std\Io\IoError
{
    return match (true) {
        condition("a", false) => "A",
        condition("b", true) => "B",
        condition("c", true) => "C",
        default => "D",
    };
}
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    echo describe(Delivery::Sent("R-12")) . "\n";
    echo inspect(true) . " " . inspect("text") . "\n";
    echo choose() . "\n";
    echo false ? "wrong" : true ? "ready" : "wrong";
    echo " " . consume();
}
"#,
    )
    .expect("checked Stage 28 match should lower through the PHP compatibility backend");

    assert!(php.contains("__doriaMatchesCase"));
    assert!(php.contains("__doriaPayloadAt"));
    assert!(php.contains("__doria_mixed_is("));
    assert!(!php.contains("get_debug_type("));
    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\nmain();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = Command::new("php")
        .arg("-d")
        .arg("display_errors=1")
        .arg("-r")
        .arg(script)
        .output()
        .expect("generated match PHP should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        run.stdout,
        b"gsent R-12\nbool true string text\nabB\nready owned"
    );
}

#[test]
fn php_backend_preserves_exact_doria_types_after_mixed() {
    let php = doriac::compile_source_to_php(
        "stage28-exact-mixed.doria",
        r#"
enum UnitState { case Ready; }
enum BackedState: string { case Ready = "ready"; }
enum PayloadState { case Ready(int $code); }
class Document {}

function inspect(mixed $value): string
{
    return match ($value) {
        int8 $item => "int8",
        int16 $item => "int16",
        int32 $item => "int32",
        int $item => "int",
        uint8 $item => "uint8",
        uint16 $item => "uint16",
        uint32 $item => "uint32",
        uint64 $item => "uint64",
        float32 $item => "float32",
        float $item => "float",
        bool $item => "bool",
        string $item => "string",
        UnitState $item => "unit",
        BackedState $item => "backed",
        PayloadState $item => "payload",
        Document $item => "class",
        default => "other",
    };
}

function makeInt(): mixed
{
    return 7;
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    int8 $int8Value = 1;
    int16 $int16Value = 1;
    int32 $int32Value = 1;
    int $intValue = 1;
    uint8 $uint8Value = 1;
    uint16 $uint16Value = 1;
    uint32 $uint32Value = 1;
    uint64 $uint64Value = 1;
    float32 $float32Value = 1.25;
    float $floatValue = 1.25;
    echo inspect($int8Value) . " " . inspect($int16Value) . " " . inspect($int32Value) . " " . inspect($intValue) . "\n";
    echo inspect($uint8Value) . " " . inspect($uint16Value) . " " . inspect($uint32Value) . " " . inspect($uint64Value) . "\n";
    echo inspect($float32Value) . " " . inspect($floatValue) . " " . inspect(true) . " " . inspect("text") . "\n";
    echo inspect(UnitState::Ready) . " " . inspect(BackedState::Ready) . " " . inspect(PayloadState::Ready(7)) . " " . inspect(new Document()) . "\n";
    echo inspect(makeInt());
}
"#,
    )
    .expect("PHP mixed values must retain exact Doria type tags");

    assert!(!php.contains("get_debug_type("));
    assert!(php.contains("__doria_box_mixed("));
    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\nmain();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = Command::new("php")
        .arg("-d")
        .arg("display_errors=1")
        .arg("-r")
        .arg(script)
        .output()
        .expect("generated exact-type PHP should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        run.stdout,
        b"int8 int16 int32 int\nuint8 uint16 uint32 uint64\nfloat32 float bool string\nunit backed payload class\nint"
    );
}

#[test]
fn php_backend_executes_payload_enum_construction_equality_and_storage() {
    let php = doriac::compile_source_to_php(
        "stage27-payload.doria",
        r#"
enum Coordinate
{
    case Origin;
    case Point(int $x, int $y);
}

class Drawing
{
    function __construct(Coordinate $coordinate)
    {
    }
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    Coordinate $point = Coordinate::Point(y: 22, x: 20);
    Coordinate $copy = $point;
    ?Coordinate $nullable = $copy;
    List<Coordinate> $items = [Coordinate::Origin, $copy];
    let $drawing = new Drawing($point);

    echo $copy == Coordinate::Point(20, 22);
    echo " {$nullable == $point}";
    echo " {$drawing->coordinate == $point}\n";
}
"#,
    )
    .expect("payload enum compatibility lowering should compile");

    assert!(php.contains("final class Coordinate implements __DoriaValueEquatable"));
    assert!(!php.contains("enum Coordinate"));

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\nmain();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = Command::new("php")
        .arg("-d")
        .arg("display_errors=1")
        .arg("-r")
        .arg(script)
        .output()
        .expect("generated payload enum PHP should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"true true true\n");
}

#[test]
fn php_backend_distinguishes_omitted_payload_defaults_from_explicit_null() {
    let php = doriac::compile_source_to_php(
        "nullable-payload-default.doria",
        r#"
enum Coordinate
{
    case Origin;
    case Point(int $x, int $y);
}

function describe(?Coordinate $value = Coordinate::Origin): string
{
    if ($value == null) { return "null"; }
    if ($value == Coordinate::Origin) { return "origin"; }
    return "point";
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    echo describe(null);
    echo " ";
    echo describe();
}
"#,
    )
    .expect("nullable Copy payload defaults should lower to PHP");

    assert!(php.contains("Coordinate|array|null $value = []"));
    assert!(php.contains("if ($value === [])"));
    assert!(!php.contains("$value ??="));

    let script = format!(
        "{}\nmain();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = Command::new("php")
        .arg("-r")
        .arg(script)
        .output()
        .expect("PHP should execute nullable payload defaults");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"null origin");
}

#[test]
fn php_backend_initializes_internal_static_payloads_in_declaring_class_scope() {
    let php = doriac::compile_source_to_php(
        "internal-static-payload.doria",
        r#"
enum Label
{
    case Empty;
    case Text(string $value);
}

class Vault
{
    internal static Label $label = Label::Text("secret");

    static function reveal(): Label
    {
        return self::label;
    }
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    echo Vault::reveal() == Label::Text("secret");
}
"#,
    )
    .expect("internal static payload initializers should lower to PHP");

    assert!(php.contains("private static Label $label;"));
    assert!(php.contains("(\\Closure::bind(static function (): void {"));
    assert!(php.contains("self::$label = Label::Text(\"secret\");"));
    assert!(!php.contains("Vault::$label ="));

    let script = format!(
        "{}\nmain();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = Command::new("php")
        .arg("-r")
        .arg(script)
        .output()
        .expect("PHP should execute internal static payload initialization");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"true");
}

#[test]
fn php_grouped_declarations_use_one_collision_safe_temporary_in_order() {
    let php = doriac::compile_source_to_php(
        "stage26a.doria",
        r#"
function value(): string throws Doria\Std\Io\IoError { echo "once\n"; return "shared"; }
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let $__doria_grouped_value0 = "user";
    let $first, $second, $third = value();
    echo "{$first}:{$second}:{$third}:{$__doria_grouped_value0}\n";
}
"#,
    )
    .expect("PHP should lower grouped Copy declarations");

    assert!(php.contains("$__doria_grouped_value0 = \"user\";"));
    let temporary = php
        .lines()
        .find(|line| line.trim().ends_with(" = value();"))
        .and_then(|line| line.trim().split_once(" = ").map(|(name, _)| name))
        .expect("grouped initializer temporary");
    assert_ne!(temporary, "$__doria_grouped_value0");
    let first = php.find(&format!("$first = {temporary};")).unwrap();
    let second = php.find(&format!("$second = {temporary};")).unwrap();
    let third = php.find(&format!("$third = {temporary};")).unwrap();
    let cleanup = php.find(&format!("unset({temporary});")).unwrap();
    assert!(first < second && second < third && third < cleanup);
    assert!(!php.contains("$first = $second ="));

    let script = format!(
        "{}\nmain();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = Command::new("php")
        .arg("-r")
        .arg(script)
        .output()
        .expect("PHP should execute grouped declarations");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"once\nshared:shared:shared:user\n");
}

#[test]
fn php_backend_emits_folded_copy_scalar_parameter_defaults() {
    let php = doriac::compile_source_to_php(
        "folded-defaults.doria",
        r#"
function sample(int $count = 1 + 2, float $ratio = 1.0 / 2.0, bool $ordered = 1 < 2): void
{
}
"#,
    )
    .expect("const-evaluable Copy-scalar defaults should lower to PHP literals");

    assert!(php.contains(
        "function sample(int $count = 3, float $ratio = 0.5, bool $ordered = true): void"
    ));
    assert!(!php.contains("$count = 1 + 2"));
    assert!(!php.contains("$ratio = fdiv("));
    assert!(!php.contains("$ordered = __doria_less("));
}

#[test]
fn php_backend_normalizes_defaulted_cell_parameters_without_leaking_transport() {
    let php = doriac::compile_source_to_php(
        "defaulted-cell-parameters.doria",
        r#"
function invert(writable bool $value = false): bool
{
    let writable $apply = function (): bool with (writable $value) {
        $value = !$value;
        return $value;
    };
    return $apply();
}

class Counter
{
    function __construct(writable bool $value = false)
    {
        let writable $read = function (): bool with (writable $value) {
            $value = !!$value;
            return $value;
        };
        $read();
    }

    function read(): bool { return $this->value; }
}

function main(): void throws Doria\Std\Io\IoError
{
    let writable $seed = true;
    let writable $constructorSeed = true;
    let $defaultCounter = new Counter();
    let $explicitCounter = new Counter($constructorSeed);
    echo invert() . ":" . invert($seed) . ":{$seed}:" .
        $defaultCounter->read() . ":" . $explicitCounter->read() . "\n";
}
"#,
    )
    .expect("defaulted cell parameters should lower to PHP");

    assert!(php.contains("function invert(__DoriaCell|bool $value = false): bool"));
    assert!(php.contains("public bool $value;"));
    assert!(php.contains("function __construct(__DoriaCell|bool $value = false)"));
    assert!(php.contains("if (!($value instanceof __DoriaCell))"));
    assert!(php.contains("$this->value = $value->value;"));

    let script = format!(
        "{}\nmain();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = Command::new("php")
        .arg("-r")
        .arg(script)
        .output()
        .expect("PHP should execute defaulted and explicit cell arguments");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"true:false:false:false:true\n");
}

#[test]
fn emits_php_for_boolean_word_operators() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    echo true and false;
    echo false or true;
    echo not false;
    echo true xor false;
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("__doria_write_stdout(__doria_display(((true) && (false))), "));
    assert!(php.contains("__doria_write_stdout(__doria_display(((false) || (true))), "));
    assert!(php.contains("__doria_write_stdout(__doria_display(!(false)), "));
    assert!(php.contains("__doria_write_stdout(__doria_display(((true) !== (false))), "));
}

#[test]
fn parenthesizes_logical_operands_for_php() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    echo true and null ?? true;
    echo false or null ?? true;
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("__doria_write_stdout(__doria_display(((true) && (null ?? true))), "));
    assert!(php.contains("__doria_write_stdout(__doria_display(((false) || (null ?? true))), "));
    assert!(!php.contains("true && null ?? true"));
    assert!(!php.contains("false || null ?? true"));
}

#[test]
fn parenthesizes_xor_operands_for_php() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    echo true == true xor false;
    echo false xor true != false;
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("__doria_write_stdout(__doria_display(((true === true) !== (false))), "));
    assert!(php.contains("__doria_write_stdout(__doria_display(((false) !== (true !== false))), "));
    assert!(!php.contains("true === true !== false"));
    assert!(!php.contains("false !== true !== false"));
}

#[test]
fn emits_typed_php_comparisons() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    echo "01" == "1";
    echo "01" != "1";
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("__doria_write_stdout(__doria_display(\"01\" === \"1\"), "));
    assert!(php.contains("__doria_write_stdout(__doria_display(\"01\" !== \"1\"), "));
    assert!(!php.contains("echo \"01\" == \"1\";"));
    assert!(!php.contains("echo \"01\" != \"1\";"));
}

#[test]
fn php_backend_preserves_byte_lexicographic_string_ordering() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): int throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    if ("10" < "2") {
        return 42;
    }
    return 0;
}
"#,
    )
    .expect("PHP should preserve Doria string ordering");

    assert!(php.contains("__doria_less(\"10\", \"2\")"));
    assert!(php.contains("strcmp($left, $right)"));

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\nexit(main());",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = Command::new("php")
        .arg("-r")
        .arg(script)
        .output()
        .expect("PHP should execute generated output");
    assert_eq!(run.status.code(), Some(42));
}

#[test]
fn php_backend_keeps_exact_int64_alias_and_signed_comparison_subset() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function isLess(int64 $left, int $right): bool
{
    return $left < $right;
}

function identity(int64 $value): int64
{
    return $value;
}
"#,
    )
    .expect("the exact signed integer subset should remain supported by PHP");

    assert!(php.contains("function isLess(int $left, int $right): bool"));
    assert!(php.contains("return __doria_less($left, $right);"));
    assert!(php.contains("function identity(int $value): int"));
}

#[test]
fn php_backend_rejects_stage_13_integer_shapes_it_cannot_preserve() {
    let cases = [
        (
            "checked overflow",
            r#"
function add(int $left, int $right): int
{
    return $left + $right;
}
"#,
            "checked integer overflow behavior for `+`",
        ),
        (
            "checked compound assignment",
            r#"
function update(): void
{
    let writable $value = 1;
    $value += 1;
}
"#,
            "checked integer overflow behavior for `+=`",
        ),
        (
            "checked increment",
            r#"
function update(): void
{
    let writable $value = 1;
    $value++;
}
"#,
            "checked integer overflow behavior for `++`",
        ),
        (
            "integer division",
            r#"
function divide(int $left, int $right): int
{
    return $left / $right;
}
"#,
            "Doria integer division semantics for `/`",
        ),
        (
            "integer shift",
            r#"
function shift(int $value, int $count): int
{
    return $value << $count;
}
"#,
            "Doria integer shift semantics for `<<`",
        ),
        (
            "fixed-width bitwise",
            r#"
function mask(int $left, int $right): int
{
    return $left & $right;
}
"#,
            "fixed-width Doria bitwise semantics for `&`",
        ),
        (
            "uint64 maximum",
            r#"
function maximum(): uint64
{
    return 18446744073709551615;
}
"#,
            "integer literal `18446744073709551615` outside PHP's signed integer range",
        ),
        (
            "checked conversion",
            r#"
function convert(): void
{
    let $value = Int8::from(1);
}
"#,
            "checked Doria integer conversion semantics for `Int8::from(...)`",
        ),
    ];

    for (name, source, expected) in cases {
        let diagnostics = match doriac::compile_source_to_php("test.doria", source) {
            Ok(php) => panic!("{name} unexpectedly generated PHP:\n{php}"),
            Err(diagnostics) => diagnostics,
        };

        assert_eq!(diagnostics[0].code, "B1301", "{name}: {diagnostics:?}");
        assert!(
            diagnostics[0]
                .message
                .contains("PHP compatibility backend cannot preserve"),
            "{name}: {diagnostics:?}"
        );
        assert!(
            diagnostics[0].message.contains(expected),
            "{name}: {diagnostics:?}"
        );
    }
}

#[test]
fn php_capability_failure_does_not_make_valid_doria_fail_check() {
    let source = r#"
function divide(int $left, int $right): int
{
    return $left / $right;
}
"#;

    doriac::check_source("test.doria", source)
        .expect("PHP compatibility limitations must not affect Doria checking");

    let diagnostics = doriac::compile_source_to_php("test.doria", source)
        .expect_err("PHP generation must reject integer division rather than emit PHP `/`");
    assert_eq!(diagnostics[0].code, "B1301");
    assert!(diagnostics[0].message.contains("integer division"));
}

#[test]
fn php_backend_maps_float64_and_allows_float_arithmetic() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function total(): float64
{
    writable float $value = 1.5 + 2.5;
    $value += 1.0;
    return $value;
}
"#,
    )
    .expect("PHP should preserve default float arithmetic");

    assert!(php.contains("function total(): float"));
    assert!(php.contains("$value = 1.5 + 2.5;"));
    assert!(php.contains("$value += 1.0;"));
    assert!(!php.contains("float64"));
}

#[test]
fn php_backend_rejects_noncanonical_float_display() {
    for source in [
        "function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { echo 10000000000.0 * 10000000000.0; }",
        "function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { echo \"value=\" . 1.5; }",
        "function show(float $value): void throws Doria\\Std\\Io\\IoError { echo \"value={$value}\"; } function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error {}",
    ] {
        let diagnostics = doriac::compile_source_to_php("test.doria", source)
            .expect_err("PHP must reject float display it cannot preserve canonically");
        assert_eq!(diagnostics[0].code, "B1301");
        assert!(diagnostics[0]
            .message
            .contains("canonical float display formatting"));
    }
}

#[test]
fn php_backend_uses_fdiv_and_rejects_inexact_cross_kind_conversions() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function divide(float $left, float64 $right): float
{
    writable float $value = $left / $right;
    $value /= 2.0;
    return $value;
}
"#,
    )
    .expect("PHP should lower IEEE float64 division through fdiv");
    assert!(php.contains("fdiv($left, $right)"), "{php}");
    assert!(php.contains("$value = fdiv($value, 2.0);"), "{php}");
    assert!(!php.contains(" / "), "{php}");
    assert!(!php.contains("/="), "{php}");
    assert!(!php.contains("float64"), "{php}");

    for source in [
        "function main(): int throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { return Float::toInt(42.0); }",
        "function helper(): float { return Int::toFloat(42); } function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error {}",
    ] {
        let diagnostics = doriac::compile_source_to_php("test.doria", source)
            .expect_err("PHP must reject conversions it cannot prove exact");
        assert_eq!(diagnostics[0].code, "B1301");
        assert!(diagnostics[0].message.contains("conversion semantics"));
    }
}

#[test]
fn php_backend_accepts_float32_storage_but_rejects_unrounded_arithmetic() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function identity(float32 $value): float32
{
    return $value;
}
"#,
    )
    .expect("PHP can carry an already-quantized float32 value without arithmetic");
    assert!(php.contains("function identity(float $value): float"));

    let diagnostics = doriac::compile_source_to_php(
        "test.doria",
        "function add(float32 $left, float32 $right): float32 { return $left + $right; }",
    )
    .expect_err("PHP must reject float32 operations without binary32 rounding");
    assert_eq!(diagnostics[0].code, "B1301");
    assert!(diagnostics[0].message.contains("binary32 rounding"));
}

#[test]
fn php_backend_constant_folds_exact_tests_on_concrete_numeric_types() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        "function narrow(int $value): bool { return $value is int8; } function exact(int8 $value): bool { return $value is int8; }",
    )
    .expect("known concrete Doria types do not need PHP host-type inference");
    assert!(php.contains("function narrow(int $value): bool\n{\n    return (false);"));
    assert!(php.contains("function exact(int $value): bool\n{\n    return (true);"));
}

#[test]
fn php_backend_parenthesizes_type_tests_as_expression_atoms() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        "function test(mixed $value): bool { return $value is int == true; }",
    )
    .expect("PHP should preserve nested type-test precedence");

    assert!(php.contains("return (__doria_mixed_is($value, \"int\")) === true;"));
}

#[test]
fn php_backend_allows_negative_integer_literals_but_rejects_runtime_negation() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function negativeOne(): int
{
    return -1;
}

function minimum(): int
{
    return -9223372036854775808;
}
"#,
    )
    .expect("in-range signed integer literals should lower to PHP");

    assert!(php.contains("return -(1);"));
    assert!(php.contains("return (-9223372036854775807 - 1);"));

    let diagnostics = doriac::compile_source_to_php(
        "test.doria",
        r#"
function negate(int $value): int
{
    return -$value;
}
"#,
    )
    .expect_err("runtime checked integer negation must remain unsupported in PHP");
    assert_eq!(diagnostics[0].code, "B1301");
    assert!(diagnostics[0].message.contains("unary `-`"));
}

#[test]
fn parenthesizes_unary_not_operands_for_php() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    echo not (1 < 2);
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("__doria_write_stdout(__doria_display(!((__doria_less(1, 2)))), "));
    assert!(!php.contains("echo !1 < 2;"));
}

#[test]
fn php_backend_preserves_main_string_local_echo() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let $message = "Hello Doria!";
    echo $message;
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("$message = \"Hello Doria!\";"));
    assert!(php.contains("__doria_write_stdout(__doria_display($message), "));
}

#[test]
fn php_backend_preserves_main_string_concat_echo() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let $name = "Doria";
    echo "Hello " . $name . "!";
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("$name = \"Doria\";"));
    assert!(php.contains("__doria_display($name)"));
}

#[test]
fn php_backend_preserves_main_string_concat_local_initializer() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let $name = "Doria";
    let $message = "Hello " . $name . "!";
    echo $message;
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("$message = __doria_display("));
    assert!(php.contains("__doria_display($name)"));
    assert!(php.contains("__doria_write_stdout(__doria_display($message), "));
}

#[test]
fn emits_php_for_stage_10_integer_helper_function_call() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function identity(int $value): int
{
    return $value;
}

function main(): int throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    return identity(42);
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("function identity(int $value): int"));
    assert!(php.contains("return $value;"));
    assert!(php.contains("function main(): int"));
    assert!(php.contains("return identity(42);"));
}

#[test]
fn emits_php_for_bool_helper_condition() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function isAnswer(int $value): bool
{
    return $value == 42;
}

function main(): int throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    if (isAnswer(42)) {
        return 42;
    }

    return 0;
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("function isAnswer(int $value): bool"));
    assert!(php.contains("if (isAnswer(42))"));
}

#[test]
fn emits_php_for_string_helper_echo() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function greet(string $name): void throws Doria\Std\Io\IoError
{
    echo "Hello " . $name . "!";
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    greet("Doria");
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("function greet(string $name): void"));
    assert!(php.contains("__doria_display($name)"));
    assert!(php.contains("greet(\"Doria\");"));
}

#[test]
fn emits_php_for_stage_10_void_helper_call() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function hello(): void throws Doria\Std\Io\IoError
{
    echo "Hello Doria!";
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    hello();
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("function hello(): void"));
    assert!(php.contains("__doria_write_stdout(__doria_display(\"Hello Doria!\"), "));
    assert!(php.contains("function main(): void"));
    assert!(php.contains("hello();"));
}

#[test]
fn lowers_checked_program_to_hir() {
    let lowered = doriac::lower_source(
        "test.doria",
        r#"
let $name = "Doria";
let $copy = $name;
"#,
    )
    .expect("lowering should succeed");

    assert!(matches!(
        &lowered.items[0],
        hir::Item::Statement(hir::Stmt::VarDecl(decl))
            if decl.bindings.len() == 1 && decl.bindings[0].name == "name"
    ));
}

#[test]
fn lowers_control_flow_to_hir() {
    let lowered = doriac::lower_source(
        "test.doria",
        r#"
let writable $count = 0;
if ($count < 10) {
    let $label = "small";
} else {
    let $label = "large";
}

while ($count < 10) {
    $count += 1;
}
"#,
    )
    .expect("lowering should succeed");

    assert!(matches!(
        &lowered.items[1],
        hir::Item::Statement(hir::Stmt::If(if_stmt))
            if matches!(if_stmt.condition, hir::Expr::Binary { .. })
                && if_stmt.else_branch.is_some()
    ));
    assert!(matches!(
        &lowered.items[2],
        hir::Item::Statement(hir::Stmt::While(while_stmt))
            if matches!(while_stmt.condition, hir::Expr::Binary { .. })
    ));
}

#[test]
fn omits_grouping_around_assignment_targets_for_php() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
let writable $count = 0;
($count) = 1;

class Person
{
    writable string $name;

    function __construct(string $initial)
    {
        $this->name = $initial;
    }
}

let writable $person = new Person("Ada");
($person->name) = "Lucy";
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("$count = 1;"));
    assert!(php.contains("$person->name = \"Lucy\";"));
    assert!(!php.contains("($count) = 1;"));
    assert!(!php.contains("($person->name) = \"Lucy\";"));
}

#[test]
fn emits_php_for_basic_control_flow() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    let writable $count = 0;
    if ($count < 10) {
        echo "small";
    } else if ($count < 20) {
        echo "medium";
    } else {
        echo "large";
    }

    while ($count < 10) {
        echo $count;
        $count = 10;
    }
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("if (__doria_less($count, 10))"));
    assert!(php.contains("__doria_write_stdout(__doria_display(\"small\"), "));
    assert!(php.contains("else if (__doria_less($count, 20))"));
    assert!(php.contains("__doria_write_stdout(__doria_display(\"medium\"), "));
    assert!(php.contains("__doria_write_stdout(__doria_display(\"large\"), "));
    assert!(php.contains("while (__doria_less($count, 10))"));
    assert!(php.contains("__doria_write_stdout(__doria_display($count), "));
    assert!(php.contains("$count = 10;"));
}

#[test]
fn php_backend_executes_stage28a_slice1_control_flow() {
    let php = doriac::compile_source_to_php(
        "stage28a.doria",
        r#"
function probe(bool $value): bool throws Doria\Std\Io\IoError
{
    echo $value ? "predicate true\n" : "predicate false\n";
    return $value;
}

function skipped(): bool throws Doria\Std\Io\IoError
{
    echo "condition\n";
    return true;
}

function whenGate(): bool throws Doria\Std\Io\IoError
{
    echo "when gate\n";
    return true;
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    string $label = given {
        let $prepared = "ready";
        false;
    } when (skipped()): string {
        return $prepared;
    } else when (skipped()) {
        return "alternate";
    } else {
        return "fallback {$prepared}";
    };
    echo $label . "\n";

    string $alternate = given {
        whenGate();
    } when (false): string {
        return "wrong";
    } else when (true) {
        return "alternate";
    } else {
        return "fallback";
    };
    echo $alternate . "\n";

    given {
        let writable $running = true;
        probe($running);
    } while ($running) {
        echo "body\n";
        $running = false;
        continue;
    }

    do {
        echo "once\n";
    } while (false);
}
"#,
    )
    .expect("Stage 28a Slice 1 control flow should lower to PHP");

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\nmain();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = Command::new("php")
        .arg("-d")
        .arg("display_errors=1")
        .arg("-r")
        .arg(script)
        .output()
        .expect("generated Stage 28a PHP should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        run.stdout,
        b"fallback ready\nwhen gate\nalternate\npredicate true\nbody\npredicate false\nonce\n"
    );
}

#[test]
fn php_backend_executes_stage28a_finalizers() {
    let php = doriac::compile_source_to_php(
        "stage28a-finalizers.doria",
        r#"
function record(string $message): void
{
    try {
        echo $message;
    } catch (Doria\Std\Io\IoError) {
    }
}

function returnThroughFinalizer(): int throws Doria\Std\Io\IoError
{
    if (true) {
        return 42;
    } finally {
        record("return cleanup\n");
    }

    return 0;
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    given {
        let $prepared = "prepared";
    } if (true) {
        echo "if {$prepared}\n";
    } finally {
        record("if cleanup {$prepared}\n");
    }

    string $selected = when (true): string {
        return "selected";
    } else {
        return "wrong";
    } finally {
        record("when cleanup\n");
    };
    echo "{$selected}\n";

    let writable $count = 0;
    while ($count < 3) {
        if ($count == 0) {
            $count = 1;
            continue;
        }
        $count = 2;
        break;
    } finally {
        record("while cleanup {$count}\n");
    }

    do {
        echo "do body\n";
    } while (false) finally {
        record("do cleanup\n");
    }

    if (true) {
        if (true) {
            echo "nested body\n";
        } finally {
            record("inner cleanup\n");
        }
    } finally {
        record("outer cleanup\n");
    }

    echo "return {returnThroughFinalizer()}\n";
}
"#,
    )
    .expect("Stage 28a finalizers should lower to PHP");

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\nmain();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = Command::new("php")
        .arg("-d")
        .arg("display_errors=1")
        .arg("-r")
        .arg(script)
        .output()
        .expect("generated Stage 28a finalizer PHP should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        run.stdout,
        b"if prepared\nif cleanup prepared\nwhen cleanup\nselected\nwhile cleanup 2\ndo body\ndo cleanup\nnested body\ninner cleanup\nouter cleanup\nreturn cleanup\nreturn 42\n"
    );

    let panic_php = doriac::compile_source_to_php(
        "stage28a-panic-finalizers.doria",
        r#"
function record(string $message): void
{
    try {
        echo $message;
    } catch (Doria\Std\Io\IoError) {
    }
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    if (true) {
        if (true) {
            panic("stop");
        } finally {
            record("wrong inner\n");
        }
    } finally {
        record("wrong outer\n");
    }
}
"#,
    )
    .expect("fatal panic inside finalizers should lower to PHP");
    let panic_script = format!(
        "{}\nmain();",
        panic_php
            .strip_prefix("<?php")
            .expect("generated PHP header")
    );
    let panic_run = Command::new("php")
        .arg("-d")
        .arg("display_errors=1")
        .arg("-r")
        .arg(panic_script)
        .output()
        .expect("generated panic finalizer PHP should execute");
    assert_eq!(panic_run.status.code(), Some(101));
    assert!(panic_run.stdout.is_empty());
    assert!(String::from_utf8_lossy(&panic_run.stderr).contains("stop"));
}

#[test]
fn php_fatal_panic_bypasses_every_stage28a_finalizer_entry() {
    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }

    for (name, source, expected_stdout) in [
        (
            "if-condition",
            include_str!("../../../examples/native/main_finally_panic_if_condition.doria"),
            &b"before\n"[..],
        ),
        (
            "given-setup",
            include_str!("../../../examples/native/main_finally_panic_given_setup.doria"),
            &b"before\n"[..],
        ),
        (
            "given-predicate",
            include_str!("../../../examples/native/main_finally_panic_given_predicate.doria"),
            &b"before\n"[..],
        ),
        (
            "when-branch",
            include_str!("../../../examples/native/main_finally_panic_when_branch.doria"),
            &b"before\n"[..],
        ),
        (
            "when-condition",
            include_str!("../../../examples/native/main_finally_panic_when_condition.doria"),
            &b"before\n"[..],
        ),
        (
            "while-body",
            include_str!("../../../examples/native/main_finally_panic_while_body.doria"),
            &b"before\n"[..],
        ),
        (
            "while-condition",
            include_str!("../../../examples/native/main_finally_panic_while_condition.doria"),
            &b"before\n"[..],
        ),
        (
            "do-body",
            include_str!("../../../examples/native/main_finally_panic_do_body.doria"),
            &b"before\n"[..],
        ),
        (
            "do-condition",
            include_str!("../../../examples/native/main_finally_panic_do_condition.doria"),
            &b"before\n"[..],
        ),
        (
            "inner-finalizer",
            include_str!("../../../examples/native/main_finally_panic_inner_finalizer.doria"),
            &b"before\ninner\n"[..],
        ),
    ] {
        let php = doriac::compile_source_to_php(format!("{name}.doria"), source)
            .unwrap_or_else(|diagnostics| panic!("{name} should lower to PHP: {diagnostics:#?}"));
        let script = format!(
            "{}\nmain();",
            php.strip_prefix("<?php").expect("generated PHP header")
        );
        let run = Command::new("php")
            .arg("-d")
            .arg("display_errors=1")
            .arg("-r")
            .arg(script)
            .output()
            .unwrap_or_else(|error| panic!("{name} generated PHP should execute: {error}"));

        assert_eq!(run.status.code(), Some(101), "{name}");
        assert_eq!(run.stdout, expected_stdout, "{name}");
        assert!(
            String::from_utf8_lossy(&run.stderr).contains("stop"),
            "{name} should preserve its panic transport"
        );
    }
}

#[test]
fn emits_php_for_loop_control() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let writable $code = 0;

    while ($code < 10) {
        $code = 10;

        if ($code == 5) {
            continue;
        }

        if ($code == 8) {
            break;
        }
    }
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("continue;"));
    assert!(php.contains("break;"));
}

#[test]
fn emits_php_for_stage_9_range_iteration() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    foreach (0..<10 as $i) {
        echo "x";
    }

    foreach (0..10 as $i) {
        echo "x";
    }

    foreach ((0..2) as $k) {
        echo "x";
    }
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("__doria_range_start"));
    assert!(php.contains("; $i__doria"));
    assert!(php.contains(" < $__doria_range_end"));
    assert!(php.contains(" <= $__doria_range_end"));
    assert!(php.matches("__doria_range_start").count() >= 3);
    assert!(!php.contains("unsupported range expression"));
}

#[test]
fn guards_inclusive_php_ranges_before_terminal_increment() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    foreach (9223372036854775807..9223372036854775807 as $i) {
        echo "x";
    }
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("$__doria_range_done"));
    assert!(php.contains("= false;"));
    assert!(php.contains("!$__doria_range_done"));
    assert!(php.contains("&& $i <= $__doria_range_end"));
    assert!(php.contains("$i < $__doria_range_end"));
    assert!(php.contains("? $i++ : ($__doria_range_done"));
    assert!(php.contains("= true)"));
    assert!(!php.contains("; $i++)"));
}

#[test]
fn rejects_standalone_range_before_php_codegen() {
    let err = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let $range = 0..10;
}
"#,
    )
    .expect_err("semantic checking should reject standalone ranges before PHP codegen");

    assert!(
        err.iter().any(|diagnostic| diagnostic.code == "E0426"),
        "expected E0426, got {err:?}"
    );
}

#[test]
fn emits_void_main_without_exit_wrapper_for_php() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    echo "Hello Doria!";
    return;
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("function main(): void"));
    assert!(php.contains("__doria_write_stdout(__doria_display(\"Hello Doria!\"), "));
    assert!(php.contains("return;"));
    assert!(!php.contains("exit(main())"));
}

#[test]
fn preserves_block_local_bindings_in_php_output() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function greet(string $name): string
{
    if (true) {
        let $name = "inner";
        return $name;
    }

    return $name;
}

function main(): void throws Doria\Std\Io\IoError
{
    let $name = "outer";
    if (true) {
        let $name = $name . " inner";
        echo "block {$name}";
    }
    echo $name;
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("$name = \"outer\";"));
    assert!(php.contains("$name__doria1 = __doria_display($name) . __doria_display(\" inner\");"));
    assert!(php.contains("__doria_display($name__doria1)"));
    assert!(php.contains("__doria_write_stdout(__doria_display($name), "));
    assert!(php.contains("function greet(string $name): string"));
    assert!(php.contains("$name__doria1 = \"inner\";"));
    assert!(php.contains("return $name__doria1;"));
    assert!(php.contains("return $name;"));
    assert!(!php.contains("$name = $name . \" inner\";"));
}

#[test]
fn debug_backend_emits_stage_11_artifact_and_supports_runtime_string_output() {
    let output = doriac::compile_source(
        "test.doria",
        include_str!("../../../examples/debug/main_for_count_10.doria"),
        BackendTarget::Debug,
    )
    .expect("debug backend should emit the Stage 11g artifact");

    let BackendOutput::Text {
        extension,
        contents,
    } = output
    else {
        panic!("debug backend should return text output");
    };
    assert_eq!(extension, "debug");
    assert_eq!(contents, "exit_status: 10\nstdout:\n");

    let output = doriac::compile_source(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    let $name = "Doria";
    echo $name;
}
"#,
        BackendTarget::Debug,
    )
    .expect("debug backend should execute supported runtime string output");
    let BackendOutput::Text {
        extension,
        contents,
    } = output
    else {
        panic!("debug backend should return text output");
    };
    assert_eq!(extension, "debug");
    assert_eq!(contents, "exit_status: 0\nstdout: Doria\n");
}

#[test]
fn php_backend_lowers_panic_to_stderr_and_status_101() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    panic("boom");
}
"#,
    )
    .expect("panic should lower through the compatibility backend");

    assert!(php.contains("__doria_panic(\"P1000\""));
    assert!(php.contains("\"P1000\" => [\"Program Panicked\""));
    assert!(php.contains("debug_backtrace(DEBUG_BACKTRACE_IGNORE_ARGS)"));
    assert!(php.contains("\"\\n\\nCall Path\""));
    assert!(php.contains("\"\\n\\nProcess Exited With Status 101\\n\""));
    assert!(php.contains("exit(101);"));
    let panic_helper = php
        .split("function __doria_panic")
        .nth(1)
        .and_then(|tail| tail.split("function __doria_read_line").next())
        .expect("panic helper should be emitted before I/O helpers");
    assert!(!panic_helper.contains("throw new"));
}

#[test]
fn php_backend_panic_trace_preserves_doria_function_frames() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"function panicNow(): void
{
    panic("boom");
}

function middle(): void
{
    panicNow();
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    middle();
}
"#,
    )
    .expect("panic should lower through the compatibility backend");

    assert!(php.contains("foreach (debug_backtrace(DEBUG_BACKTRACE_IGNORE_ARGS)"));
    assert!(php.contains("[\"function\"]"));
    assert!(php.contains("\"\\n\" . $name . \" · \""));

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }

    let script = format!(
        "{}\nmain();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = Command::new("php")
        .arg("-r")
        .arg(script)
        .output()
        .expect("PHP should execute generated output");

    assert_eq!(run.status.code(), Some(101));
    assert!(run.stdout.is_empty());
    let stderr = String::from_utf8(run.stderr).expect("diagnostic must be UTF-8");
    assert!(stderr.starts_with("Panic[P1000]: Program Panicked\n\nWhere\n"));
    assert!(stderr.contains("\n\nWhy\nThe program explicitly raised a fatal panic."));
    assert!(stderr.contains("\n\nNote\nboom"));
    assert!(stderr.contains("\n\nCall Path\npanicNow · test.doria:"));
    assert!(stderr.contains("\nmiddle · test.doria:"));
    assert!(stderr.contains("\nmain · test.doria:"));
    assert!(stderr.ends_with("\n\nProcess Exited With Status 101\n"));
    assert!(!stderr.contains("Stack Trace"));
}

#[test]
fn php_io_panic_trace_preserves_allowed_doria_helper_named_methods() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
class Reader
{
    function __doria_read_file(): void
        throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
    {
        read_file("__doria_missing_stage17_frame_test__/missing.txt");
    }
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let $reader = new Reader();
    $reader->__doria_read_file();
}
"#,
    )
    .expect("allowed helper-named methods should lower through PHP compatibility");

    assert!(php.contains("isset($frame[\"class\"])"));
    assert!(php.contains("in_array($frame[\"function\"], $helperFunctions, true)"));

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }

    let script = format!(
        "{}\nmain();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = Command::new("php")
        .arg("-r")
        .arg(script)
        .output()
        .expect("PHP should execute generated output");

    assert_eq!(run.status.code(), Some(70));
    assert!(run.stdout.is_empty());
    let stderr = String::from_utf8(run.stderr).expect("diagnostic must be UTF-8");
    assert!(stderr.starts_with("Error[R1000]: Unhandled Doria\\Std\\Io\\IoError\n\nWhere\n"));
    assert!(stderr.contains(" · Reader::__doria_read_file\n\n"));
    assert!(stderr.contains("read_file(\"__doria_missing_stage17_frame_test__/missing.txt\")"));
    assert!(!stderr.contains("Call Path"));
    assert!(stderr.ends_with("\n\nProcess Exited With Status 70\n"));
    assert!(!stderr.contains("PHP"));
    assert!(!stderr.contains("Stack Trace"));
}

#[test]
fn php_io_warning_fallback_preserves_portable_not_found_reason() {
    let php = doriac::compile_source_to_php(
        "php-io-reason.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    try {
        read_file("__doria_missing_php_reason_test__/missing.txt");
    } catch (Doria\Std\Io\IoError $error) {
        echo $error->reason == Doria\Std\Io\IoErrorReason::NotFound;
        echo " ";
        echo $error->systemCode ?? -1;
    }
}
"#,
    )
    .expect("missing-file reason fixture should lower to PHP");

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\nmain();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = Command::new("php")
        .arg("-r")
        .arg(script)
        .output()
        .expect("generated PHP reason fixture should execute");

    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(run.stdout.starts_with(b"true "), "{:?}", run.stdout);
    assert!(
        run.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn php_backend_uses_text_output_shape() {
    let output = doriac::compile_source(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    let $name = "Doria";
    echo $name;
}
"#,
        BackendTarget::Php,
    )
    .expect("php backend should emit output");

    assert!(matches!(
        output,
        doriac::backend::BackendOutput::Text { .. }
    ));
}

#[test]
fn strips_doria_writable_from_php_output() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
class Person
{
    writable string $name;

    function __construct() { $this->name = ""; }

    writable function rename(string $name): void
    {
        $this->name = $name;
    }
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("public string $name;"));
    assert!(php.contains("public function rename(string $name): void"));
    assert!(!php.contains("writable"));
}

#[test]
fn emits_internal_members_as_private_php_members() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
class Person
{
    internal string $secret;

    function __construct() { $this->secret = ""; }

    function reveal(): string
    {
        return $this->secret;
    }

    internal function message(): string
    {
        return "Hello";
    }
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("private string $secret;"));
    assert!(php.contains("public function reveal(): string"));
    assert!(php.contains("private function message(): string"));
}

#[test]
fn omits_constructor_return_type_for_php() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
class Person
{
    function __construct(): void
    {
        return;
    }
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("public function __construct()"));
    assert!(!php.contains("__construct(): void"));
}

#[test]
fn rejects_deterministic_destruction_that_php_cannot_preserve() {
    let diagnostics = doriac::compile_source_to_php(
        "test.doria",
        "class Person { function __destruct(): void { return; } }",
    )
    .expect_err("PHP destruction timing is not Doria scope destruction");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B1901"));
}

#[test]
fn allows_take_on_copy_parameters_in_php() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        "function identity(take int $value): int { return $value; } function main(): int throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { return identity(42); }",
    )
    .expect("take on a Copy value is a semantic no-op");

    assert!(php.contains("function identity(int $value): int"));
    assert!(!php.contains("take int"));
}

#[test]
fn rejects_take_on_move_parameters_in_php() {
    let diagnostics = doriac::compile_source_to_php(
        "test.doria",
        "class Guard {} function consume(take Guard $guard): void {}",
    )
    .expect_err("PHP cannot preserve class ownership transfer");

    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B1901"));
}

#[test]
fn php_backend_rejects_shared_ownership_in_every_declared_type_position() {
    for source in [
        "class Node {} function inspect(SharedReference<Node> $node): void {}",
        "class Node {} function make(): SharedReference<Node> { return shared new Node(); }",
        "class Node {} class Box { ?SharedReference<Node> $node = null; }",
        "class Node {} function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { ?SharedReference<Node> $node = null; }",
    ] {
        let diagnostics = doriac::compile_source_to_php("shared-type.doria", source)
            .expect_err("PHP must reject shared ownership at the type boundary");
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "B2301"
                    && diagnostic
                        .message
                        .contains("cannot preserve Doria shared ownership")
            }),
            "expected shared-ownership capability diagnostic, got {diagnostics:?}"
        );
    }
}

#[test]
fn rejects_static_lifecycle_methods_before_php_emission() {
    let err = doriac::compile_source_to_php(
        "test.doria",
        r#"
class Person
{
    static function __construct()
    {
    }
}
"#,
    )
    .expect_err("semantic checking should reject static construction before PHP codegen");

    assert!(err.iter().any(|diagnostic| {
        diagnostic.code == "E0465"
            && diagnostic
                .message
                .contains("invoked by `new` and cannot be `static`")
    }));
}

#[test]
fn rejects_resource_type_before_php_codegen() {
    let err = doriac::compile_source_to_php(
        "test.doria",
        r#"
class StreamBox
{
    resource $handle;

    function read(resource $handle): resource
    {
        return $handle;
    }
}
"#,
    )
    .expect_err("semantic checking should reject resource before PHP codegen");

    assert!(err.iter().any(|diagnostic| {
        diagnostic.code == "E0432"
            && diagnostic
                .message
                .contains("`resource` is reserved for PHP interop")
    }));
}

#[test]
fn rejects_array_callable_name_before_php_codegen() {
    let err = doriac::compile_source_to_php(
        "test.doria",
        r#"
function array(): void
{
}
"#,
    )
    .expect_err("semantic checking should reject array as a callable before PHP codegen");

    assert!(err.iter().any(|diagnostic| {
        diagnostic.code == "E0310" && diagnostic.message.contains("`array`")
    }));
}

#[test]
fn php_backend_uses_valid_callable_hints_for_type_only_function_signatures() {
    let source = r#"
function accept(function(int): int $callback): void
{
}

function main(): void
{
}
"#;

    let php = doriac::compile_source_to_php("type-only-function.doria", source)
        .expect("type-only structural function syntax should remain PHP-lowerable");
    assert!(php.contains("function accept(__DoriaFunctionValue $callback): void"));
    assert!(php.contains("interface __DoriaFunctionValue"));
    assert!(!php.contains("function accept(callable $callback)"));
    assert!(!php.contains("function accept(function $callback)"));
}

#[test]
fn rejects_compiler_helper_function_namespace_before_php_codegen() {
    let err = doriac::compile_source_to_php(
        "test.doria",
        r#"
function __doria_read_line(): void
{
}
"#,
    )
    .expect_err("compiler helper names must be rejected before PHP codegen");

    assert!(err.iter().any(|diagnostic| {
        diagnostic.code == "E0310" && diagnostic.message.contains("`__doria_`")
    }));
}

#[test]
fn lowers_interpolated_string_to_hir() {
    let lowered = doriac::lower_source(
        "test.doria",
        r#"
let $name = "Doria";
let $message = "Hello, {$name}";
"#,
    )
    .expect("lowering should succeed");

    let hir::Item::Statement(hir::Stmt::VarDecl(declaration)) = &lowered.items[1] else {
        panic!("expected interpolated-string declaration");
    };
    let hir::Expr::InterpolatedString { parts, .. } = &declaration.initializer else {
        panic!("expected interpolated string in HIR");
    };

    assert!(matches!(
        &parts[0],
        hir::InterpolatedStringPart::Text { value, span }
            if value == "Hello, " && span.start < span.end
    ));
    assert!(matches!(
        &parts[1],
        hir::InterpolatedStringPart::Expr(hir::Expr::Variable { name, .. }) if name == "name"
    ));
}

#[test]
fn emits_explicit_php_concat_for_interpolated_strings() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    let $name = "Andrew";
    echo "Hello, {$name}!";
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("\"Hello, \" . __doria_display($name) . \"!\""));
    assert!(!php.contains("{$name}"));

    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
class Person
{
    function __construct(string $name)
    {
    }

    function greet(): void throws Doria\Std\Io\IoError
    {
        echo "Hello, {$this->name}";
    }
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("\"Hello, \" . __doria_display($this->name)"));
    assert!(!php.contains("{$this->name}"));
}

#[test]
fn escapes_php_interpolation_markers_in_string_text() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    let $name = "Andrew";
    let $amount = 10;
    echo "Hello, $name";
    echo 'Literal $name';
    echo "Total: {$amount} ($currency)";
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("__doria_write_stdout(__doria_display(\"Hello, \\$name\"), "));
    assert!(php.contains("__doria_write_stdout(__doria_display(\"Literal \\$name\"), "));
    assert!(php.contains("\"Total: \" . __doria_display($amount) . \" (\\$currency)\""));
}

#[test]
fn compiles_person_example_with_explicit_interpolation() {
    let example_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/php/person.doria");
    let source = std::fs::read_to_string(&example_path).expect("read person example");
    let php = doriac::compile_source_to_php("examples/php/person.doria", &source)
        .expect("person example should compile");

    assert!(php.contains(
        "return \"Hello, my name is \" . __doria_display($this->name) . \" and I am \" . __doria_display($this->age) . \" years old!\";"
    ));
    assert!(!php.contains("{$this->name}"));
    assert!(!php.contains("{$this->age}"));
}

#[test]
fn php_backend_lowers_stage17_io_with_doria_failure_checks() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let writable $line = read_line();
    if ($line != null) { write_stderr($line); }
    let $contents = read_file("input.txt");
    write_file("copy.txt", $contents);
    append_file("copy.txt", $contents);
    printf("enabled=%s", false);
    echo sprintf("%05d", 42);
}
"#,
    )
    .expect("Stage 17 PHP compatibility lowering should succeed");

    assert!(php.contains(
        "function __doria_read_line(string $prompt, int $start, int $end, string $callable): ?string"
    ));
    // The prompt is written exactly and stdout is flushed before stdin is read,
    // without depending on PHP's optional readline extension.
    assert!(php.contains("if ($prompt !== \"\")"));
    assert!(php.contains("__DoriaStdIoIoTarget::__doriaCaseStandardOutput()"));
    assert!(php.contains("__doria_flush_stdout($start, $end, $callable);"));
    assert!(php.contains("if (@fflush(STDOUT)) { return; }"));
    assert!(php.contains("if (__doria_is_broken_pipe(error_get_last())) { exit(0); }"));
    assert!(!php.contains("readline("));
    assert!(php.contains("function __doria_panic("));
    assert!(php.contains("?string $message = null,"));
    assert!(php.contains("?string $callable = null,"));
    assert!(!php.contains("): never"));
    assert!(php.contains("if ($line === false)"));
    assert!(php.contains("if (feof(STDIN)) { return null; }"));
    assert!(php.contains("__DoriaStdIoIoOperation::Read"));
    assert!(php.contains("new __DoriaStdIoInvalidUtf8Error("));
    assert!(!php.contains("__doria_panic(\"P1403\""));
    assert!(!php.contains("__doria_panic(\"P1404\""));
    assert!(php.contains("str_ends_with($line, \"\\n\")"));
    assert!(php.contains("str_ends_with($line, \"\\r\")"));
    assert!(php.contains("__doria_read_file(\"input.txt\", start:"));
    assert!(php.contains("$file === false"));
    assert!(php.contains("$chunk = @fread($file, 8192)"));
    assert!(php.contains("__doria_write_file(\"copy.txt\", $contents, start:"));
    assert!(php.contains("__doria_append_file(\"copy.txt\", $contents, start:"));
    assert!(php.contains("@fopen($path, $append ? \"ab\" : \"wb\")"));
    assert!(!php.contains("__doria_panic(\"P1402\""));
    assert!(php.contains("while ($offset < $length)"));
    assert!(php.contains("$written === false || $written === 0"));
    assert!(php.contains("__doria_write_stderr($line, start:"));
    assert!(php.contains("__doria_printf("));
    assert!(php.contains("\"enabled=%s\", __doria_display(false))"));
    assert!(php.contains("__doria_sprintf(\"%05d\", 42)"));
}

#[test]
fn php_prompted_read_line_preserves_prompt_and_line_discipline() {
    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }

    let php = doriac::compile_source_to_php(
        "prompted-input.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let $first = read_line("P: ");
    if ($first != null) { echo "<{$first}>\n"; }

    let $blank = read_line();
    if ($blank != null) { echo "[{$blank}]\n"; }

    let $eof = read_line("E: ");
    if ($eof == null) { echo "EOF"; }
}
"#,
    )
    .expect("prompted input should lower to PHP");
    let script = format!(
        "{}\nmain();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let mut child = Command::new("php")
        .arg("-r")
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("PHP should execute prompted input");
    child
        .stdin
        .take()
        .expect("PHP stdin should be piped")
        .write_all(b"alpha\r\n\n")
        .expect("PHP fixture input should be writable");
    let output = child.wait_with_output().expect("PHP fixture should exit");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"P: <alpha>\n[]\nE: EOF");
    assert!(output.stderr.is_empty());
}

#[test]
fn php_backend_exits_cleanly_only_for_user_output_to_closed_pipes() {
    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }

    for (name, statement, close_stdout) in [
        ("stdout-echo", "echo \"Doria output.\\n\";", true),
        ("stdout-printf", "printf(\"Doria output.\\n\");", true),
        ("stderr", "write_stderr(\"Doria output.\\n\");", false),
    ] {
        let source = format!(
            r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{{
    let $line = read_line();
    {statement}
}}
"#
        );
        let php = doriac::compile_source_to_php(format!("closed-{name}.doria"), &source)
            .expect("closed-pipe fixture should lower to PHP");
        let script = format!(
            "{}\nmain();",
            php.strip_prefix("<?php").expect("generated PHP header")
        );
        let mut child = Command::new("php")
            .arg("-r")
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("PHP should execute the closed-pipe fixture");
        if close_stdout {
            drop(child.stdout.take());
        } else {
            drop(child.stderr.take());
        }
        child
            .stdin
            .take()
            .expect("PHP stdin should be piped")
            .write_all(b"\n")
            .expect("PHP input should unblock the fixture");
        let output = child.wait_with_output().expect("PHP fixture should exit");

        assert_eq!(
            output.status.code(),
            Some(0),
            "closed {name} must be a clean exit"
        );
        assert!(output.stdout.is_empty(), "{name} fixture wrote stdout");
        assert!(output.stderr.is_empty(), "{name} fixture wrote stderr");
    }

    let php = doriac::compile_source_to_php(
        "closed-prompt.doria",
        "function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { let $line = read_line(\"Prompt: \" ); }",
    )
    .expect("closed prompt fixture should lower to PHP");
    let script = format!(
        "{}\nmain();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let mut child = Command::new("php")
        .arg("-r")
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("PHP should execute the closed-prompt fixture");
    drop(child.stdout.take());
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("PHP fixture should exit");
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
}

#[test]
fn php_backend_keeps_panic_fatal_when_stderr_is_closed() {
    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }

    let php = doriac::compile_source_to_php(
        "panic-closed-stderr.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let $line = read_line();
    panic("boom");
}
"#,
    )
    .expect("panic fixture should lower to PHP");
    let script = format!(
        "{}\nmain();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let mut child = Command::new("php")
        .arg("-r")
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("PHP should execute the panic fixture");
    drop(child.stderr.take());
    child
        .stdin
        .take()
        .expect("PHP stdin should be piped")
        .write_all(b"\n")
        .expect("PHP input should unblock the fixture");
    let output = child.wait_with_output().expect("PHP fixture should exit");

    assert_eq!(output.status.code(), Some(101));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn php_backend_rejects_noncanonical_float_display_in_checked_formats() {
    for source in [
        "function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { echo sprintf(\"%s\", 1.5); }",
        "function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { printf(\"%s\", 1.5); }",
    ] {
        let diagnostics = doriac::compile_source_to_php("test.doria", source)
            .expect_err("PHP must reject float display formatting it cannot preserve canonically");
        assert_eq!(diagnostics[0].code, "B1301");
        assert!(diagnostics[0]
            .message
            .contains("canonical float display formatting"));
    }
}

#[test]
fn php_backend_keeps_stage17_frontend_rejections_and_uint64_honesty() {
    for source in [
        "function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { print(\"x\"); }",
        "function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { let $format = \"%d\"; echo sprintf($format, 1); }",
    ] {
        doriac::compile_source_to_php("test.doria", source)
            .expect_err("invalid Doria must fail before PHP lowering");
    }

    let error = doriac::compile_source_to_php(
        "test.doria",
        "function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { uint64 $value = 18446744073709551615; echo sprintf(\"%d\", $value); }",
    )
    .expect_err("PHP must reject uint64 formatting it cannot preserve");
    assert!(error.iter().any(|diagnostic| diagnostic.code == "B1301"));
}

#[test]
fn php_backend_preserves_stage_18_expression_interpolation_order() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function left(): int throws Doria\Std\Io\IoError
{
    echo "L";
    return 20;
}

function right(): int throws Doria\Std\Io\IoError
{
    echo "R";
    return 22;
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    echo "={left() == 20 and right() == 22}";
}
"#,
    )
    .expect("Stage 18 expression interpolation should lower to PHP");

    assert!(php.contains("__doria_display(((left() === 20) && (right() === 22)))"));

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\nmain();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = Command::new("php")
        .arg("-r")
        .arg(script)
        .output()
        .expect("PHP should execute generated Stage 18 output");
    assert!(run.status.success());
    assert_eq!(run.stdout, b"LR=true");
    assert!(run.stderr.is_empty());
}

#[test]
fn php_backend_rejects_checked_integer_interpolation_it_cannot_preserve() {
    let diagnostics = doriac::compile_source_to_php(
        "main_expression_interpolation.doria",
        include_str!("../../../examples/native/main_expression_interpolation.doria"),
    )
    .expect_err("PHP must not silently replace checked Doria integer arithmetic");

    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "B1301"
            && diagnostic
                .message
                .contains("checked integer overflow behavior for `+`")
    }));
}

#[test]
fn php_backend_preserves_the_exact_displayable_contract() {
    let php = doriac::compile_source_to_php(
        "displayable.doria",
        include_str!("../../../examples/php/displayable.doria"),
    )
    .expect("the exact Displayable subset should lower to PHP");

    assert!(php.contains("interface __DoriaDisplayable"));
    assert!(php.contains("class Label implements __DoriaDisplayable"));
    assert!(php.contains("public function toString(): string"));
    assert!(php.contains("$value->toString()"));
    assert!(!php.contains("__toString"));

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\nmain();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = Command::new("php")
        .arg("-r")
        .arg(script)
        .output()
        .expect("PHP should execute generated Displayable output");
    assert!(run.status.success());
    assert_eq!(run.stdout, b"Doria Doria Doria Doria");
    assert!(run.stderr.is_empty());
}

#[test]
fn php_backend_reserves_helper_class_names_case_insensitively() {
    for name in [
        "__DoriaDisplayable",
        "__doriadisplayable",
        "__DORIADISPLAYABLE",
        "__DoriaValueEquatable",
        "__doriavalueequatable",
        "__DoriaMixedValue",
        "__doriamixedvalue",
        "__DoriaFunctionValue",
        "__doriafunctionvalue",
        "__DoriaClosureEnvironment42",
        "__doriafuturehelper",
    ] {
        let diagnostics = doriac::compile_source_to_php(
            "reserved_display_helper.doria",
            format!("class {name} {{}}"),
        )
        .expect_err("PHP display helper name variants must be reserved");

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "E0309"
                && diagnostic.message.contains("`__Doria` type prefix")
                && diagnostic.message.contains(&format!("`{name}`"))
                && diagnostic.message.contains("reserved")
        }));
    }
}

#[test]
fn php_backend_distinguishes_stage20_constants_and_static_properties() {
    let php = doriac::compile_source_to_php(
        "statics.doria",
        r#"
const TOP_LIMIT = 42;

class Counter
{
    const LABEL = "ready";
    static int $initial = TOP_LIMIT;
    static writable string $current = Counter::LABEL;

    static function read(): string
    {
        return Counter::current;
    }
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    Counter::current = "done";
    echo Counter::LABEL;
    echo Counter::read();
}
"#,
    )
    .expect("Stage 20 statics should lower to the PHP compatibility backend");

    assert!(php.contains("const __DORIA_CONST_TOP_LIMIT = 42;"));
    assert!(php.contains("public const LABEL = \"ready\";"));
    assert!(php.contains("public static int $initial = 42;"));
    assert!(php.contains("public static string $current = \"ready\";"));
    assert!(php.contains("public static function read(): string"));
    assert!(php.contains("return Counter::$current;"));
    assert!(php.contains("Counter::$current = \"done\";"));
    assert!(php.contains("Counter::LABEL"));
}

#[test]
fn php_backend_emits_evaluated_constants_and_static_initializers() {
    let php = doriac::compile_source_to_php(
        "evaluated-constants.doria",
        r#"
const ANSWER = LATER + 1;
const LATER = 41;

class Counter
{
    static int $initial = ANSWER;
    static writable int $value = Counter::initial + 1;
}

function main(): void throws Doria\Std\Io\IoError
{
    echo ANSWER;
    echo Counter::value;
}
"#,
    )
    .expect("evaluated declarations should lower to PHP literals");

    assert!(php.contains("const __DORIA_CONST_ANSWER = 42;"));
    assert!(php.contains("const __DORIA_CONST_LATER = 41;"));
    assert!(php.contains("public static int $initial = 42;"));
    assert!(php.contains("public static int $value = 43;"));
    assert!(!php.contains("= LATER + 1"));
    assert!(!php.contains("= Counter::$initial + 1"));

    let script = format!(
        "{}\nmain();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = Command::new("php")
        .arg("-r")
        .arg(script)
        .output()
        .expect("PHP should execute evaluated declarations");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"4243");
    assert!(run.stderr.is_empty());
}

#[test]
fn php_backend_mangles_every_top_level_constant_away_from_php_names() {
    let php = doriac::compile_source_to_php(
        "predefined-constants.doria",
        r#"
const CLASS = 1;
const INF = 2;
const NAN = 3;
function main(): void throws Doria\Std\Io\IoError
{
    echo CLASS;
    echo INF;
    echo NAN;
}
"#,
    )
    .expect("top-level Doria constants should not collide with PHP names");

    for name in ["CLASS", "INF", "NAN"] {
        assert!(php.contains(&format!("const __DORIA_CONST_{name} =")));
        assert!(!php.contains(&format!("const {name} =")));
        assert!(php.contains(&format!(
            "__doria_write_stdout(__doria_display(__DORIA_CONST_{name}), "
        )));
    }
}

#[test]
fn php_backend_rejects_class_constant_named_class_case_insensitively() {
    let diagnostics =
        doriac::compile_source_to_php("class-constant.doria", "class Counter { const CLASS = 1; }")
            .expect_err("PHP reserves the CLASS class-constant name");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B2001"
            && diagnostic.message.contains("reserves `class`")));
}

#[test]
fn php_backend_moves_runtime_static_property_initializers_into_the_constructor() {
    let source = r#"
class Counter
{
    static int $seed = 41;
    int $value = Counter::seed;
}
"#;
    doriac::check_source("static-read-in-property.doria", source)
        .expect("the Doria initializer is valid independently of PHP restrictions");
    let php = doriac::compile_source_to_php("static-read-in-property.doria", source)
        .expect("runtime Doria initializers should lower into the PHP constructor");
    assert!(php.contains("public int $value;"));
    assert!(php.contains("$this->value = Counter::$seed;"));
}

#[test]
fn php_backend_moves_static_calls_in_instance_initializers_before_constructor_body() {
    let source = r#"
class Factory
{
    int $value = Factory::seed();
    internal static function seed(): int { return 42; }
}
"#;
    doriac::check_source("static-call-in-property.doria", source)
        .expect("Doria property initializers may call declaring-class static methods");
    let php = doriac::compile_source_to_php("static-call-in-property.doria", source)
        .expect("runtime Doria initializers should lower into the PHP constructor");
    assert!(php.contains("public int $value;"));
    assert!(php.contains("$this->value = Factory::seed();"));
}

#[test]
fn php_backend_moves_executable_instance_initializers_into_generated_constructors() {
    let cases = [
        (
            "function-call-in-property.doria",
            r#"
function seed(): int { return 42; }
class Counter { int $value = seed(); }
"#,
            "$this->value = seed();",
        ),
        (
            "construction-in-property.doria",
            r#"
class Person {}
class Office { Person $manager = new Person(); }
"#,
            "$this->manager = new Person();",
        ),
    ];

    for (path, source, expected_output) in cases {
        doriac::check_source(path, source)
            .expect("executable property defaults are valid Doria independently of PHP");
        let php = doriac::compile_source_to_php(path, source)
            .expect("the PHP backend should lower executable defaults into a constructor");
        assert!(php.contains(expected_output), "generated PHP:\n{php}");
    }
}

#[test]
fn php_backend_executes_constructor_rooted_and_owned_property_writes() {
    let source =
        include_str!("../../../examples/native/main_constructor_owned_property_writes.doria");
    let php = doriac::compile_source_to_php("constructor-owned-property.doria", source)
        .expect("constructor-rooted and owned property writes should lower to PHP");

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\nmain();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = Command::new("php")
        .arg("-d")
        .arg("display_errors=1")
        .arg("-r")
        .arg(script)
        .output()
        .expect("generated constructor property PHP should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        run.stdout,
        include_bytes!("fixtures/native_io/main_constructor_owned_property_writes/expected_stdout")
    );
}

#[test]
fn php_backend_keeps_php_constant_expression_property_defaults() {
    let php = doriac::compile_source_to_php(
        "constant-property-defaults.doria",
        r#"
const int SEED = 41;
class Config
{
    const int OFFSET = 1;
    List<int> $values = [SEED, Config::OFFSET];
    bool $enabled = true && !false;
}
"#,
    )
    .expect("PHP constant expressions remain valid property defaults");

    assert!(php.contains("public array $values = [__DORIA_CONST_SEED, Config::OFFSET];"));
    assert!(php.contains("public bool $enabled = ((true) && (!(false)));"));
}

#[test]
fn php_backend_keeps_collection_property_defaults_per_instance() {
    let php = doriac::compile_source_to_php(
        "collection-property-defaults.doria",
        r#"
class Scene {}
class SceneManager
{
    writable List<Scene> $scenes = [];
}
"#,
    )
    .expect("PHP compatibility output should preserve constant empty collection defaults");

    assert!(php.contains("public array $scenes = [];"));
    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }

    let script = format!(
        "{}\n$left = new SceneManager(); $right = new SceneManager(); $left->scenes[] = new Scene(); echo count($left->scenes) . ':' . count($right->scenes);",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = Command::new("php")
        .arg("-d")
        .arg("display_errors=1")
        .arg("-r")
        .arg(script)
        .output()
        .expect("generated collection property defaults should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"1:0");
}

#[test]
fn php_backend_emits_int_min_constants_without_php_literal_overflow() {
    let php = doriac::compile_source_to_php(
        "int-min-constant.doria",
        r#"
const int MINIMUM = -9223372036854775808;
class Limits { static int $minimum = MINIMUM; }
"#,
    )
    .expect("the full signed int range should lower to PHP");

    assert!(php.contains("const __DORIA_CONST_MINIMUM = (-9223372036854775807 - 1);"));
    assert!(php.contains("public static int $minimum = (-9223372036854775807 - 1);"));

    if let Ok(version) = Command::new("php").arg("--version").output() {
        if version.status.success() {
            let mut child = Command::new("php")
                .arg("-l")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("PHP lint should start");
            use std::io::Write;
            child
                .stdin
                .take()
                .expect("PHP stdin")
                .write_all(php.as_bytes())
                .expect("generated PHP should be written");
            let output = child.wait_with_output().expect("PHP lint should finish");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

#[test]
fn php_backend_parenthesizes_composite_member_receivers() {
    let php = doriac::compile_source_to_php(
        "coalesced-receiver.doria",
        r#"
class Label
{
    function text(): string { return "label"; }
}

function read(?Label $left, ?Label $right): ?string
{
    return ($left ?? $right)?->text();
}
"#,
    )
    .expect("coalesced member receivers should lower to PHP");

    assert!(php.contains("($left ?? $right)?->text()"), "{php}");
}

#[test]
fn php_backend_rejects_unimplemented_stage23_runtime_surfaces_consistently() {
    for (name, source) in [
        (
            "indexed read",
            r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    List<int> $items = [1];
    echo $items[0];
}
"#,
        ),
        (
            "list indexOf",
            r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    List<int> $items = [1];
    echo $items->indexOf(1) ?? -1;
}
"#,
        ),
        (
            "list remove",
            r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    writable List<int> $items = [1];
    $items->remove(1);
}
"#,
        ),
        (
            "dictionary containsValue",
            r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    Dictionary<string, int> $items = ["answer" => 42];
    echo $items->containsValue(42);
}
"#,
        ),
        (
            "set endpoints",
            r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    Set<int> $items = Set::from([1]);
    echo $items->first ?? -1;
    echo $items->last ?? -1;
}
"#,
        ),
        (
            "default collection clear",
            r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    writable List<int> $items = [1];
    $items->clear();
}
"#,
        ),
        (
            "writable foreach",
            r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    writable List<int> $items = [1];
    foreach ($items as writable int $item) {
        $item += 1;
    }
}
"#,
        ),
        (
            "byte I/O",
            r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    write_stdout_bytes(read_stdin_bytes());
}
"#,
        ),
    ] {
        let diagnostics = doriac::compile_source_to_php("stage23.doria", source)
            .expect_err("PHP must reject Stage 23 runtime behavior it cannot preserve");
        assert_eq!(diagnostics[0].code, "B2301", "{name}: {diagnostics:?}");
        assert!(
            diagnostics[0].message.contains("native") && diagnostics[0].message.contains("debug"),
            "{name}: {diagnostics:?}"
        );
    }
}

#[test]
fn php_backend_keeps_readonly_dictionary_projections_iterable() {
    let php = doriac::compile_source_to_php(
        "dictionary-projection.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    Dictionary<string, int> $items = ["answer" => 42];
    foreach ($items->keys as string $key) {
        echo $key;
    }
}
"#,
    )
    .expect("readonly dictionary projections have a faithful PHP foreach lowering");

    assert!(php.contains("foreach (__doria_collection_projection($items, true) as $key)"));
}

#[test]
fn php_backend_executes_the_stage26_collection_family_with_doria_ordering() {
    let php = doriac::compile_source_to_php(
        "stage26.php.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    writable SortedDictionary<int, string> $map =
        SortedDictionary::from([2 => "two", -1 => "minus", 1 => "one"]);
    $map->set(3, "three");
    foreach ($map->keys as int $key) { echo "{$key} "; }
    echo "\n";

    writable SortedSet<int> $set = SortedSet::from([3, -1, 1, 3]);
    $set->add(0);
    foreach ($set as int $value) { echo "{$value} "; }
    echo "\n";

    writable PriorityQueue<int> $queue = PriorityQueue::from([4, -2, 1]);
    $queue->push(-3);
    while (!$queue->isEmpty) {
        let $value = $queue->pop() ?? 99;
        echo "{$value} ";
    }
    echo "\n";

    writable Deque<string> $deque = Deque::from(["middle"]);
    $deque->pushFront("first");
    $deque->pushBack("last");
    foreach ($deque as string $value) { echo "{$value} "; }
    echo "\n";

    $map->clear();
    $map->set(9, "nine");
    foreach ($map->keys as int $key) { echo "map {$key} "; }
    $set->clear();
    $set->add(2);
    foreach ($set as int $value) { echo "set {$value} "; }
    $queue->clear();
    $queue->push(6);
    echo "queue " . ($queue->pop() ?? 99) . " ";
    $deque->clear();
    $deque->pushBack("new");
    echo "deque " . ($deque->peekFront ?? "none");
}
"#,
    )
    .expect("Stage 26 collections should lower to PHP compatibility helpers");

    assert!(php.contains("final class SortedDictionary"));
    assert!(php.contains("final class SortedSet"));
    assert!(php.contains("final class PriorityQueue"));
    assert!(php.contains("final class Deque"));

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\nmain();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = Command::new("php")
        .arg("-d")
        .arg("display_errors=1")
        .arg("-r")
        .arg(script)
        .output()
        .expect("generated Stage 26 PHP should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        run.stdout,
        b"-1 1 2 3 \n-1 0 1 3 \n-3 -2 1 4 \nfirst middle last \nmap 9 set 2 queue 6 deque new"
    );
    assert!(
        run.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn php_backend_executes_checked_errors_with_doria_descriptor_dispatch() {
    let fixtures = [
        (
            "checked-error-catch.doria",
            include_str!("../../../examples/native/main_checked_error_catch.doria"),
            "caught exact\n",
        ),
        (
            "checked-error-catch-all.doria",
            include_str!("../../../examples/native/main_checked_error_catch_all.doria"),
            "catch all\n",
        ),
        (
            "checked-error-optional-binding.doria",
            include_str!("../../../examples/native/main_checked_error_optional_binding.doria"),
            "handled without binding\n",
        ),
        (
            "checked-error-rethrow.doria",
            include_str!("../../../examples/native/main_checked_error_rethrow.doria"),
            "relay\noriginal\n",
        ),
        (
            "checked-error-finally.doria",
            include_str!("../../../examples/native/main_checked_error_finally.doria"),
            "caught\nfinally\n",
        ),
        (
            "checked-error-constructor.doria",
            include_str!("../../../examples/native/main_checked_error_constructor.doria"),
            "caught constructor\n",
        ),
    ];

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }

    for (path, source, expected_stdout) in fixtures {
        let php = doriac::compile_source_to_php(path, source)
            .unwrap_or_else(|error| panic!("{path} should compile for PHP: {error:?}"));
        assert!(php.contains("catch (__DoriaCheckedError"));
        assert!(php.contains("::__doriaErrorType()"));
        if path == "checked-error-catch-all.doria" {
            assert!(!php.contains("->descriptor ==="));
        } else {
            assert!(php.contains("->descriptor ==="));
        }
        assert!(!php.contains("instanceof Failure"));
        assert!(!php.contains("instanceof BuildError"));

        let script = format!(
            "{}\nmain();",
            php.strip_prefix("<?php").expect("generated PHP header")
        );
        let run = Command::new("php")
            .arg("-d")
            .arg("display_errors=1")
            .arg("-r")
            .arg(script)
            .output()
            .unwrap_or_else(|error| panic!("{path} generated PHP should execute: {error}"));

        assert!(
            run.status.success(),
            "{path}: {}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&run.stdout),
            expected_stdout,
            "{path}"
        );
        assert!(
            run.stderr.is_empty(),
            "{path}: {}",
            String::from_utf8_lossy(&run.stderr)
        );
    }
}

#[test]
fn php_collection_clear_releases_owned_deque_values_in_doria_order() {
    let php = doriac::compile_source_to_php(
        "stage26-clear-order.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    writable Deque<int> $values = Deque::from([1]);
    $values->clear();
}
"#,
    )
    .expect("a Stage 26 Deque should emit the PHP compatibility helper");

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        r#"{}
final class ClearOrderToken
{{
    public function __construct(private int $id) {{}}
    public function __destruct() {{ echo "drop {{$this->id}}\n"; }}
}}
$values = Deque::from([new ClearOrderToken(1), new ClearOrderToken(2)]);
$values->pushFront(new ClearOrderToken(3));
$values->clear();"#,
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = Command::new("php")
        .arg("-d")
        .arg("display_errors=1")
        .arg("-r")
        .arg(script)
        .output()
        .expect("generated PHP collection clear should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "drop 2\ndrop 1\ndrop 3\n"
    );
}

#[test]
fn php_backend_executes_ordered_slice3_members_with_strict_nullable_semantics() {
    let php = doriac::compile_source_to_php(
        "stage26-slice3.php.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    writable SortedDictionary<string, ?int> $numbers = SortedDictionary::from([]);
    $numbers->set("empty", null);
    $numbers->set("zero", 0);
    $numbers->set("answer", 42);
    echo $numbers->containsValue(null);
    echo $numbers->containsValue(0);
    echo $numbers->containsValue(7);

    writable SortedDictionary<string, ?bool> $flags = SortedDictionary::from([]);
    $flags->set("empty", null);
    $flags->set("false", false);
    echo $flags->containsValue(null);
    echo $flags->containsValue(false);
    echo $flags->containsValue(true);
    echo "\n";

    SortedSet<int> $values = SortedSet::from([30, 10, 20]);
    echo $values->first ?? -1;
    echo " ";
    echo $values->last ?? -1;
    echo "\n";

    SortedSet<int> $empty = SortedSet::from([]);
    echo $empty->first ?? -1;
    echo " ";
    echo $empty->last ?? -1;
}

"#,
    )
    .expect("ordered Slice 3 members should lower to PHP compatibility helpers");

    assert!(php.contains("if ($entry[1] === $value) { return true; }"));
    assert!(php.contains("if ($name === 'first') { return $this->values[0] ?? null; }"));
    assert!(php.contains("if ($name === 'last')"));

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\nmain();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = Command::new("php")
        .arg("-d")
        .arg("display_errors=1")
        .arg("-r")
        .arg(script)
        .output()
        .expect("generated ordered Slice 3 PHP should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"truetruefalsetruetruefalse\n10 30\n-1 -1");
    assert!(
        run.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn php_entry_boundary_uses_main_effective_checked_effects() {
    let nonthrowing =
        doriac::compile_source_to_php("nonthrowing-main.doria", "function main(): void {}")
            .expect("clause-free nonthrowing main should compile");
    assert!(!nonthrowing.contains("catch (__DoriaCheckedError"));

    let inferred = doriac::compile_source_to_php(
        "inferred-main.doria",
        r#"
class Failure implements Error
{
    function __construct(string $message)
    {
    }
}

function main(): void
{
    throw new Failure("inferred");
}
"#,
    )
    .expect("clause-free throwing main should compile through the checked boundary");
    assert!(inferred.contains("catch (__DoriaCheckedError"));
    assert!(inferred.contains("__doria_report_unhandled_error($error);"));
}
