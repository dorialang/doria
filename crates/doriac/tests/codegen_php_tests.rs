use doriac::backend::BackendTarget;
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
fn recognizes_debug_as_planned_backend() {
    let err = doriac::compile_source(
        "test.doria",
        r#"
let $name = "Doria";
echo $name;
"#,
        BackendTarget::Debug,
    )
    .expect_err("debug backend is planned but not implemented yet");

    assert_eq!(err[0].code, "B0001");
    assert!(err[0].message.contains("backend `debug`"));
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
fn lowers_resource_type_to_php_mixed() {
    let php = doriac::compile_source_to_php(
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
    .expect("compilation should succeed");

    assert!(php.contains("public mixed $handle;"));
    assert!(php.contains("public function read(mixed $handle): mixed"));
    assert!(!php.contains("resource $handle"));
    assert!(!php.contains("): resource"));
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
