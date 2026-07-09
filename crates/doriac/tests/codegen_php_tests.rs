use doriac::backend::{BackendOutput, BackendTarget};
use doriac::hir;

#[test]
fn emits_php_for_simple_program() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
let writable $count = 0;
$count += 1;
echo $count;
"#,
    )
    .expect("compilation should succeed");

    assert!(php.starts_with("<?php"));
    assert!(php.contains("$count = 0;"));
    assert!(php.contains("$count += 1;"));
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
fn emits_php_for_stage_10_helper_function_call() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function add(int $left, int $right): int
{
    return $left + $right;
}

function main(): int
{
    return add(20, 22);
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("function add(int $left, int $right): int"));
    assert!(php.contains("return $left + $right;"));
    assert!(php.contains("function main(): int"));
    assert!(php.contains("return add(20, 22);"));
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
    $count += 1;
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("if ($count < 10)\n{\n    echo \"small\";\n}"));
    assert!(php.contains("else if ($count < 20)\n{\n    echo \"medium\";\n}"));
    assert!(php.contains("else\n{\n    echo \"large\";\n}"));
    assert!(php.contains("while ($count < 10)\n{\n    echo $count;\n    $count += 1;\n}"));
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
        $code += 1;

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
fn emits_php_for_stage_9_iteration() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void
{
    for (let writable $i = 0; $i < 10; $i++) {
        echo "x";
    }

    foreach (0..<10 as $i) {
        echo "x";
    }

    foreach (0..10 as $i) {
        echo "x";
    }

    foreach ((0..2) as $k) {
        echo "x";
    }

    let writable $j = 0;
    ++$j;
    $j--;
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("for ($i = 0; $i < 10; $i++)"));
    assert!(php.contains("__doria_range_start"));
    assert!(php.contains("; $i__doria"));
    assert!(php.contains(" < $__doria_range_end"));
    assert!(php.contains(" <= $__doria_range_end"));
    assert!(php.matches("__doria_range_start").count() >= 3);
    assert!(!php.contains("unsupported range expression"));
    assert!(php.contains("++$j;"));
    assert!(php.contains("$j--;"));
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
fn debug_backend_emits_stage_11d_artifact_and_rejects_broader_source() {
    let output = doriac::compile_source(
        "test.doria",
        r#"function main(): int
{
    return 42;
}
"#,
        BackendTarget::Debug,
    )
    .expect("debug backend should emit the Stage 11d artifact");

    let BackendOutput::Text {
        extension,
        contents,
    } = output
    else {
        panic!("debug backend should return text output");
    };
    assert_eq!(extension, "debug");
    assert_eq!(contents, "exit_status: 42\nstdout:\n");

    let err = doriac::compile_source(
        "test.doria",
        r#"
let $name = "Doria";
echo $name;
"#,
        BackendTarget::Debug,
    )
    .expect_err("broader source should remain outside Stage 11d MIR coverage");

    assert_eq!(err[0].code, "M1101");
    assert!(err[0]
        .message
        .contains("unsupported MIR Stage 11d coverage"));
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
