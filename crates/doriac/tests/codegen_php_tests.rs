use doriac::backend::{BackendOutput, BackendTarget};
use doriac::hir;
use std::process::Command;

#[test]
fn emits_php_for_simple_program() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
let writable $count = 0;
$count = 1;
echo $count;
"#,
    )
    .expect("compilation should succeed");

    assert!(php.starts_with("<?php"));
    assert!(php.contains("$count = 0;"));
    assert!(php.contains("$count = 1;"));
    assert!(php.contains("echo $count;"));
}

#[test]
fn emits_php_for_boolean_word_operators() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
echo true and false;
echo false or true;
echo not false;
echo true xor false;
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("echo ((true) && (false));"));
    assert!(php.contains("echo ((false) || (true));"));
    assert!(php.contains("echo !(false);"));
    assert!(php.contains("echo ((true) !== (false));"));
}

#[test]
fn parenthesizes_logical_operands_for_php() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
echo true and null ?? true;
echo false or null ?? true;
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("echo ((true) && (null ?? true));"));
    assert!(php.contains("echo ((false) || (null ?? true));"));
    assert!(!php.contains("true && null ?? true"));
    assert!(!php.contains("false || null ?? true"));
}

#[test]
fn parenthesizes_xor_operands_for_php() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
echo true == true xor false;
echo false xor true != false;
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("echo ((true === true) !== (false));"));
    assert!(php.contains("echo ((false) !== (true !== false));"));
    assert!(!php.contains("true === true !== false"));
    assert!(!php.contains("false !== true !== false"));
}

#[test]
fn emits_typed_php_comparisons() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
echo "01" == "1";
echo "01" != "1";
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("echo \"01\" === \"1\";"));
    assert!(php.contains("echo \"01\" !== \"1\";"));
    assert!(!php.contains("echo \"01\" == \"1\";"));
    assert!(!php.contains("echo \"01\" != \"1\";"));
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
    assert!(php.contains("return $left < $right;"));
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
            "nondefault width",
            r#"
function identity(int8 $value): int8
{
    return $value;
}
"#,
            "Doria `int8` width and signedness",
        ),
        (
            "uint64 maximum",
            r#"
function maximum(): uint64
{
    return 18446744073709551615;
}
"#,
            "Doria `uint64` width and signedness",
        ),
        (
            "unsigned comparison",
            r#"
function isLess(uint32 $left, uint32 $right): bool
{
    return $left < $right;
}
"#,
            "Doria `uint32` width and signedness",
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
fn php_backend_rejects_float32_precision() {
    let diagnostics = doriac::compile_source_to_php(
        "test.doria",
        r#"
function identity(float32 $value): float32
{
    return $value;
}
"#,
    )
    .expect_err("PHP must not emit `float32` as an unknown PHP type");

    assert_eq!(diagnostics[0].code, "B1301");
    assert!(diagnostics[0].message.contains("`float32` precision"));
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
echo not (1 < 2);
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("echo !((1 < 2));"));
    assert!(!php.contains("echo !1 < 2;"));
}

#[test]
fn php_backend_preserves_main_string_local_echo() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void
{
    let $message = "Hello Doria!";
    echo $message;
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("$message = \"Hello Doria!\";"));
    assert!(php.contains("echo $message;"));
}

#[test]
fn php_backend_preserves_main_string_concat_echo() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void
{
    let $name = "Doria";
    echo "Hello " . $name . "!";
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("$name = \"Doria\";"));
    assert!(php.contains("echo \"Hello \" . $name . \"!\";"));
}

#[test]
fn php_backend_preserves_main_string_concat_local_initializer() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void
{
    let $name = "Doria";
    let $message = "Hello " . $name . "!";
    echo $message;
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("$message = \"Hello \" . $name . \"!\";"));
    assert!(php.contains("echo $message;"));
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

function main(): int
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

function main(): int
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
function greet(string $name): void
{
    echo "Hello " . $name . "!";
}

function main(): void
{
    greet("Doria");
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("function greet(string $name): void"));
    assert!(php.contains("echo \"Hello \" . $name . \"!\";"));
    assert!(php.contains("greet(\"Doria\");"));
}

#[test]
fn emits_php_for_stage_10_void_helper_call() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function hello(): void
{
    echo "Hello Doria!";
}

function main(): void
{
    hello();
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("function hello(): void"));
    assert!(php.contains("echo \"Hello Doria!\";"));
    assert!(php.contains("function main(): void"));
    assert!(php.contains("hello();"));
}

#[test]
fn lowers_checked_program_to_hir() {
    let lowered = doriac::lower_source(
        "test.doria",
        r#"
let $name = "Doria";
echo $name;
"#,
    )
    .expect("lowering should succeed");

    assert!(matches!(
        &lowered.items[0],
        hir::Item::Statement(hir::Stmt::VarDecl(decl)) if decl.name == "name"
    ));
}

#[test]
fn lowers_control_flow_to_hir() {
    let lowered = doriac::lower_source(
        "test.doria",
        r#"
let writable $count = 0;
if ($count < 10) {
    echo "small";
} else {
    echo "large";
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
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("if ($count < 10)\n{\n    echo \"small\";\n}"));
    assert!(php.contains("else if ($count < 20)\n{\n    echo \"medium\";\n}"));
    assert!(php.contains("else\n{\n    echo \"large\";\n}"));
    assert!(php.contains("while ($count < 10)\n{\n    echo $count;\n    $count = 10;\n}"));
}

#[test]
fn emits_php_for_loop_control() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void
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
function main(): void
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
function main(): void
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
function main(): void
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
function main(): void
{
    echo "Hello Doria!";
    return;
}

main();
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("function main(): void"));
    assert!(php.contains("echo \"Hello Doria!\";"));
    assert!(php.contains("return;"));
    assert!(php.contains("main();"));
    assert!(!php.contains("exit(main())"));
}

#[test]
fn preserves_block_local_bindings_in_php_output() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
let $name = "outer";
if (true) {
    let $name = $name . " inner";
    echo "block {$name}";
}
echo $name;

function greet(string $name): string
{
    if (true) {
        let $name = "inner";
        return $name;
    }

    return $name;
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("$name = \"outer\";"));
    assert!(php.contains("$name__doria1 = $name . \" inner\";"));
    assert!(php.contains("echo \"block \" . $name__doria1;"));
    assert!(php.contains("echo $name;"));
    assert!(php.contains("function greet(string $name): string"));
    assert!(php.contains("$name__doria1 = \"inner\";"));
    assert!(php.contains("return $name__doria1;"));
    assert!(php.contains("return $name;"));
    assert!(!php.contains("$name = $name . \" inner\";"));
}

#[test]
fn debug_backend_emits_stage_11_artifact_and_rejects_broader_source() {
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

    let err = doriac::compile_source(
        "test.doria",
        r#"
let $name = "Doria";
echo $name;
"#,
        BackendTarget::Debug,
    )
    .expect_err("broader source should remain outside Stage 11g MIR coverage");

    assert_eq!(err[0].code, "M1101");
    assert!(err[0].message.contains("unsupported MIR Stage 11 coverage"));
}

#[test]
fn php_backend_lowers_panic_to_stderr_and_status_101() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"function main(): void
{
    panic("boom");
}
"#,
    )
    .expect("panic should lower through the compatibility backend");

    assert!(php.contains("fwrite(STDERR, \"Panic: \" . \"boom\" . \"\\nStack Trace:\\n\");"));
    assert!(php.contains("debug_backtrace(DEBUG_BACKTRACE_IGNORE_ARGS)"));
    assert!(php.contains("fwrite(STDERR, \"  at \""));
    assert!(php.contains("exit(101);"));
    assert!(!php.contains("throw new"));
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

function main(): void
{
    middle();
}
"#,
    )
    .expect("panic should lower through the compatibility backend");

    assert!(php.contains("foreach (debug_backtrace(DEBUG_BACKTRACE_IGNORE_ARGS)"));
    assert!(php.contains("[\"function\"]"));
    assert!(php.contains("\"  at \""));

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
    assert_eq!(
        run.stderr,
        b"Panic: boom\nStack Trace:\n  at panicNow\n  at middle\n  at main\n"
    );
}

#[test]
fn php_backend_uses_text_output_shape() {
    let output = doriac::compile_source(
        "test.doria",
        r#"
let $name = "Doria";
echo $name;
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
fn omits_lifecycle_method_return_types_for_php() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
class Person
{
    function __construct(): void
    {
        return;
    }

    function __destruct(): void
    {
        return;
    }
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("public function __construct()"));
    assert!(php.contains("public function __destruct()"));
    assert!(!php.contains("__construct(): void"));
    assert!(!php.contains("__destruct(): void"));
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
fn lowers_interpolated_string_to_hir() {
    let lowered = doriac::lower_source(
        "test.doria",
        r#"
let $name = "Doria";
echo "Hello, {$name}";
"#,
    )
    .expect("lowering should succeed");

    let hir::Item::Statement(hir::Stmt::Echo { expr, .. }) = &lowered.items[1] else {
        panic!("expected echo statement");
    };
    let hir::Expr::InterpolatedString { parts, .. } = expr else {
        panic!("expected interpolated string in HIR");
    };

    assert!(matches!(&parts[0], hir::InterpolatedStringPart::Text(text) if text == "Hello, "));
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
let $name = "Andrew";
echo "Hello, {$name}!";
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("echo \"Hello, \" . $name . \"!\";"));
    assert!(!php.contains("{$name}"));

    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
class Person
{
    function __construct(string $name)
    {
    }

    function greet(): void
    {
        echo "Hello, {$this->name}";
    }
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("echo \"Hello, \" . $this->name;"));
    assert!(!php.contains("{$this->name}"));
}

#[test]
fn escapes_php_interpolation_markers_in_string_text() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
let $name = "Andrew";
let $amount = 10;
echo "Hello, $name";
echo 'Literal $name';
echo "Total: {$amount} ($currency)";
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("echo \"Hello, \\$name\";"));
    assert!(php.contains("echo \"Literal \\$name\";"));
    assert!(php.contains("echo \"Total: \" . $amount . \" (\\$currency)\";"));
}

#[test]
fn compiles_person_example_with_explicit_interpolation() {
    let example_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/php/person.doria");
    let source = std::fs::read_to_string(&example_path).expect("read person example");
    let php = doriac::compile_source_to_php("examples/php/person.doria", &source)
        .expect("person example should compile");

    assert!(php.contains(
        "return \"Hello, my name is \" . $this->name . \" and I am \" . $this->age . \" years old!\";"
    ));
    assert!(!php.contains("{$this->name}"));
    assert!(!php.contains("{$this->age}"));
}
