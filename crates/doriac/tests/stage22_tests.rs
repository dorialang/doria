use doriac::ast::{BinaryOp, Expr, Item, Stmt};

fn diagnostics(source: &str) -> Vec<doriac::diagnostics::Diagnostic> {
    doriac::check_source("stage22.doria", source).expect_err("source should be rejected")
}

fn assert_code(source: &str, code: &str) -> doriac::diagnostics::Diagnostic {
    let diagnostics = diagnostics(source);
    diagnostics
        .into_iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("expected {code}"))
}

fn snapshot_entry(source: &str, code: &str) -> String {
    let diagnostic = assert_code(source, code);
    format!(
        "[{}] {}\nhelp: {}\n",
        diagnostic.code,
        diagnostic.message,
        diagnostic.help.as_deref().unwrap_or("-")
    )
}

#[test]
fn parser_preserves_nullable_null_safe_coalesce_and_is_syntax() {
    let program = doriac::parse_source(
        "stage22-syntax.doria",
        r#"
class Label { function text(): string { return "label"; } }
function choose(?Label $label, mixed $value): string
{
    let $text = $label?->text() ?? "none";
    if ($value is string) { return $value; }
    return $text;
}
"#,
    )
    .expect("Stage 22 syntax should parse");

    let Item::Function(function) = &program.items[1] else {
        panic!("expected function");
    };
    assert!(function.params[0].ty.nullable);
    let Stmt::VarDecl(declaration) = &function.body.statements[0] else {
        panic!("expected declaration");
    };
    let Expr::Binary {
        left,
        op: BinaryOp::Coalesce,
        ..
    } = &declaration.initializer
    else {
        panic!("expected coalesce expression");
    };
    assert!(matches!(
        left.as_ref(),
        Expr::MethodCall {
            null_safe: true,
            ..
        }
    ));
    let Stmt::If(if_statement) = &function.body.statements[1] else {
        panic!("expected if statement");
    };
    assert!(matches!(if_statement.condition, Expr::IsType { .. }));
}

#[test]
fn nullable_members_require_narrowing_or_null_safe_access() {
    let diagnostic = assert_code(
        r#"
class Label { function text(): string { return "label"; } }
function read(?Label $label): string { return $label->text(); }
"#,
        "E0506",
    );
    assert!(diagnostic.message.contains("possibly-null"));

    doriac::check_source(
        "stage22-narrowed.doria",
        r#"
class Label { function text(): string { return "label"; } }
function read(?Label $label): string
{
    if ($label == null) { return "none"; }
    return $label->text();
}
function safe(?Label $label): string
{
    return $label?->text() ?? "none";
}
"#,
    )
    .expect("null guards and null-safe access should be accepted");
}

#[test]
fn narrowing_is_lexical_path_sensitive_and_short_circuit_aware() {
    doriac::check_source(
        "stage22-flow.doria",
        r#"
class Label { function text(): string { return "label"; } }
function read(?Label $label, mixed $value): string
{
    if ($label != null && $label->text() == "label") {
        let $value = 1;
        echo $value;
    }
    if ($value is string && $value == "text") {
        return $value;
    }
    return $label?->text() ?? "none";
}
"#,
    )
    .expect("narrowing should apply only to the selected binding and path");

    assert_code(
        r#"
class Label { function text(): string { return "label"; } }
function read(?Label $label): string
{
    if ($label != null) { echo $label->text(); }
    return $label->text();
}
"#,
        "E0506",
    );
}

#[test]
fn branch_assignments_join_without_leaking_sibling_scope_facts() {
    let invalid = diagnostics(
        r#"
function read(bool $condition): int
{
    writable ?int $value = null;
    if ($condition) {} else { $value = 1; }
    return $value + 1;
}
"#,
    );
    assert!(
        invalid
            .iter()
            .any(|diagnostic| diagnostic.message.contains("nullable")
                || diagnostic.message.contains("?int")),
        "one branch must not narrow the post-if value: {invalid:#?}"
    );

    doriac::check_source(
        "stage22-branch-join.doria",
        r#"
function read(bool $condition): int
{
    writable ?int $value = null;
    if ($condition) { $value = 1; } else { $value = 2; }
    return $value + 1;
}
"#,
    )
    .expect("matching non-null assignments on every path should narrow after the join");
}

#[test]
fn short_circuit_rhs_uses_see_prior_call_mutations() {
    let invalid = diagnostics(
        r#"
function clear(writable ?int $value): bool { $value = null; return true; }
function read(?int $value): bool
{
    return $value != null && clear($value) && $value > 0;
}
"#,
    );
    assert!(
        invalid
            .iter()
            .any(|diagnostic| diagnostic.message.contains("nullable")
                || diagnostic.message.contains("?int")),
        "the final operand must observe the writable call's invalidation: {invalid:#?}"
    );
}

#[test]
fn impossible_is_tests_do_not_change_variable_types() {
    let invalid = diagnostics(
        r#"
function read(?int $value): string
{
    if ($value is string) { return $value; }
    return "none";
}
"#,
    );
    assert!(
        invalid
            .iter()
            .any(|diagnostic| diagnostic.message.contains("return")
                || diagnostic.message.contains("string")),
        "an impossible test must not retag the source binding: {invalid:#?}"
    );

    let program = doriac::lower_source_to_mir(
        "stage22-impossible-is.doria",
        r#"
function main(): void
{
    ?int $value = 1;
    if ($value is string) { echo "wrong"; }
    echo "ok";
}
"#,
    )
    .expect("an impossible exact test with a valid body should lower as false");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("the impossible exact test should execute");
    assert_eq!(output.stdout, b"ok");
}

#[test]
fn self_resolves_in_exact_type_tests() {
    doriac::check_source(
        "stage22-self-is.doria",
        r#"
class Label
{
    function matches(?Label $value): bool { return $value is self; }
}
"#,
    )
    .expect("self should resolve to the declaring class in an exact type test");
}

#[test]
fn nullable_equality_matches_the_native_lowering_boundary() {
    for source in [
        "function equal(?int $left, ?int $right): bool { return $left == $right; }",
        "function equal(): bool { return null == null; }",
        r#"
class Label {}
function equal(?Label $left, ?Label $right): bool { return $left == $right; }
"#,
    ] {
        assert_code(source, "E0420");
    }

    doriac::check_source(
        "stage22-nullable-string-equality.doria",
        "function equal(?string $left, ?string $right): bool { return $left == $right; }",
    )
    .expect("nullable string equality has native lowering support");
    doriac::check_source(
        "stage22-null-equality.doria",
        "function empty(?int $value): bool { return $value == null; }",
    )
    .expect("literal null checks remain supported for every nullable payload");
}

#[test]
fn mixed_rejects_operations_until_is_narrows_the_value() {
    for (operation, expected_code) in [
        ("let $result = $value->name;", "E0433"),
        ("$value->show();", "E0433"),
        ("let $result = $value + 1;", "E0433"),
        ("let $result = $value . \"x\";", "E0433"),
        ("echo \"{$value}\";", "E0415"),
        ("let $result = $value == 1;", "E0433"),
    ] {
        let source = format!("function inspect(mixed $value): void {{ {operation} }}");
        let diagnostics = diagnostics(&source);
        let diagnostic = diagnostics
            .into_iter()
            .find(|diagnostic| diagnostic.code == expected_code)
            .unwrap_or_else(|| panic!("expected {expected_code} for `{operation}`"));
        if expected_code == "E0433" {
            assert!(diagnostic.help.is_some_and(|help| help.contains("`is`")));
        }
    }

    doriac::check_source(
        "stage22-mixed.doria",
        r#"
class Label
{
    string $name = "label";
    function show(): string { return $this->name; }
}
function describe(mixed $value): string
{
    if ($value is Label) { return $value->show() . $value->name; }
    if ($value is string) { return "{$value}" . $value; }
    if ($value is int && $value == 42) { return "number"; }
    return "other";
}
"#,
    )
    .expect("an is test should establish an exact type inside its branch");
}

#[test]
fn null_assignability_and_nullable_ownership_follow_the_payload_type() {
    doriac::check_source(
        "stage22-null-assignments.doria",
        r#"
class Label {}
function inspect(?int $left, ?int $right): void {}
function accepts(mixed $value): void {}
function valid(?Label $label): void
{
    ?int $number = null;
    ?string $text = null;
    inspect($number, $number);
    accepts(null);
}
"#,
    )
    .expect("null should enter nullable and mixed slots, and nullable scalars remain Copy");

    assert_code("int $value = null;", "E0403");

    let moved = diagnostics(
        r#"
class Label {}
function consume(take ?Label $label): void {}
function invalid(?Label $label): void
{
    consume($label);
    consume($label);
}
"#,
    );
    assert!(
        moved
            .iter()
            .any(|diagnostic| diagnostic.message.contains("given away")),
        "nullable classes should retain class move semantics: {moved:#?}"
    );
}

#[test]
fn mixed_runtime_representation_remains_a_stage23_boundary() {
    let diagnostics = doriac::lower_source_to_mir(
        "stage22-mixed-runtime.doria",
        r#"
function main(): void { mixed $value = 1; }
"#,
    )
    .expect_err("mixed runtime values should not lower before Stage 23");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "M1101")
        .unwrap_or_else(|| panic!("expected native-stage diagnostic, got {diagnostics:#?}"));
    assert!(diagnostic.message.contains("Stage 23"));
}

#[test]
fn hierarchy_and_interface_is_tests_fail_at_their_owned_stages() {
    let hierarchy_source = r#"
class Base {}
class Child extends Base {}
function inspect(mixed $value): bool { return $value is Base; }
"#;
    let hierarchy_diagnostics = diagnostics(hierarchy_source);
    assert!(!hierarchy_diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code.starts_with('P')));
    let hierarchy = hierarchy_diagnostics
        .into_iter()
        .find(|diagnostic| diagnostic.code == "E0509")
        .expect("expected hierarchy-stage diagnostic");
    assert!(hierarchy.message.contains("Stage 34"));

    let interface_source = "function inspect(mixed $value): bool { return $value is Displayable; }";
    let interface_diagnostics = diagnostics(interface_source);
    assert!(!interface_diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code.starts_with('P')));
    let interface = interface_diagnostics
        .into_iter()
        .find(|diagnostic| diagnostic.code == "E0510")
        .expect("expected interface-stage diagnostic");
    assert!(interface.message.contains("Stage 35"));
}

#[test]
fn reserved_and_position_only_types_have_targeted_diagnostics() {
    assert_code("null $value = null;", "E0431");
    assert_code("void $value = null;", "E0430");
    assert_code("object $value = null;", "E0401");
    assert_code("resource $value = null;", "E0432");
}

#[test]
fn stage22_diagnostic_contract_matches_snapshot() {
    let snapshot = [
        snapshot_entry(
            r#"
class Label { function text(): string { return "label"; } }
function read(?Label $label): string { return $label->text(); }
"#,
            "E0506",
        ),
        snapshot_entry(
            "function add(mixed $value): int { return $value + 1; }",
            "E0433",
        ),
        snapshot_entry("null $value = null;", "E0431"),
        snapshot_entry("void $value = null;", "E0430"),
        snapshot_entry("object $value = null;", "E0401"),
        snapshot_entry("resource $value = null;", "E0432"),
    ]
    .concat();

    assert_eq!(
        snapshot,
        include_str!("fixtures/diagnostics/stage22_boundaries.txt")
    );
}

#[test]
fn nullable_native_fixture_lowers_to_valid_mir_and_interprets_exactly() {
    let source = include_str!("../../../examples/native/main_stage22_nullable.doria");
    let program = doriac::lower_source_to_mir("main_stage22_nullable.doria", source)
        .expect("Stage 22 fixture should lower");
    doriac::mir_validation::validate_program(&program).expect("Stage 22 MIR should validate");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("Stage 22 fixture should execute in the debug backend");
    assert_eq!(output.stdout, b"42:7:typed:text:empty:label:none:label\n");
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
}

#[test]
fn nullable_scalar_destinations_contextualize_literals_and_static_initializers() {
    let source = r#"
class Limits
{
    static ?int8 $small = 1;
    static ?float32 $ratio = 1.5;
    static ?bool $enabled = null;
    writable ?uint8 $count = 2;
}

function accept(?int16 $value): void {}

function main(): void
{
    ?int8 $small = 1;
    writable ?float32 $ratio = 1.5;
    $ratio = 2.5;
    accept(3);
    echo "ok";
}
"#;
    let program = doriac::lower_source_to_mir("stage22-nullable-scalars.doria", source)
        .expect("nullable scalar literals and statics should lower with their payload context");
    doriac::mir_validation::validate_program(&program)
        .expect("nullable scalar literal and static MIR should validate");
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("Cranelift should lower nullable scalar statics")
        .is_empty());
    #[cfg(feature = "llvm-backend")]
    assert!(!doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("LLVM should lower nullable scalar statics")
        .is_empty());
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("nullable scalar statics should execute");
    assert_eq!(output.stdout, b"ok");
}

#[test]
fn null_safe_property_access_is_never_a_write_place() {
    for statement in [
        "$value?->count = 1;",
        "$value?->count += 1;",
        "$value?->count++;",
    ] {
        let source = format!(
            "class Counter {{ writable int $count = 0; }} function update(?Counter $value): void {{ {statement} }}"
        );
        let diagnostic = assert_code(&source, "E0511");
        assert!(diagnostic.message.contains("write target"));
    }
}

#[test]
fn writable_call_arguments_invalidate_narrowing_facts() {
    let invalid = diagnostics(
        r#"
function clear(writable ?int $value): void { $value = null; }
function read(writable ?int $value): int
{
    if ($value == null) { return 0; }
    clear($value);
    return $value + 1;
}
"#,
    );
    assert!(
        invalid
            .iter()
            .any(|diagnostic| diagnostic.message.contains("nullable")
                || diagnostic.message.contains("?int")),
        "the nullable use after a writable call should be rejected: {invalid:#?}"
    );

    doriac::check_source(
        "stage22-readonly-call-fact.doria",
        r#"
function inspect(?int $value): void {}
function read(?int $value): int
{
    if ($value == null) { return 0; }
    inspect($value);
    return $value + 1;
}
"#,
    )
    .expect("a readonly call must preserve the caller's narrowing fact");
}

#[test]
fn nullable_class_ownership_and_null_safe_calls_execute_consistently() {
    let source = r#"
class Other {}

class Tracked
{
    ?string $alias = null;
    ?int8 $code = 7;
    ?Other $other = null;
    function __construct(string $name) {}
    function __destruct() { echo "<" . $this->name . ">"; }
    function maybeName(bool $present): ?string
    {
        if ($present) { return "value"; }
        return null;
    }
    function maybeOther(): ?Tracked { return null; }
    function announce(string $message): void { echo $message; }
}

class Holder
{
    function __construct() {}
    function inspect(?Tracked $value): void {}
    static function inspectStatic(?Tracked $value): void {}
}

function make(bool $present, string $name): ?Tracked
{
    if ($present) { return new Tracked($name); }
    return null;
}

function inspect(?Tracked $value): void {}
function forward(?Tracked $value): ?Tracked { return $value; }
function sideEffect(): string { echo "wrong"; return "wrong"; }

function main(): void
{
    let $owner = new Tracked("owner");
    let $holder = new Holder();
    $holder->inspect(new Tracked("method"));
    Holder::inspectStatic(new Tracked("static"));
    inspect($owner);
    inspect(new Tracked("temp"));
    echo ":" . (forward($owner)?->maybeName(true) ?? "none");
    echo ":" . (forward($owner)?->alias ?? "empty");
    echo ":";
    if (forward($owner)?->code != null) { echo "code"; }
    if (forward($owner)?->maybeOther() == null) { echo ":none"; }
    if (forward($owner)?->other == null) { echo ":none"; }

    make(false, "missing")?->announce(sideEffect());
    make(true, "present")?->announce(":called");

    let $maybe = make(true, "chosen");
    let $chosen = $maybe ?? new Tracked("fallback");
    echo ":" . ($chosen->maybeName(true) ?? "none");
}
"#;
    let program = doriac::lower_source_to_mir("stage22-nullable-ownership.doria", source)
        .expect("nullable ownership and null-safe statement calls should lower");
    doriac::mir_validation::validate_program(&program)
        .expect("nullable ownership MIR should validate");
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("Cranelift should lower nullable ownership paths")
        .is_empty());
    #[cfg(feature = "llvm-backend")]
    assert!(!doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("LLVM should lower nullable ownership paths")
        .is_empty());
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("nullable ownership paths should execute");
    assert_eq!(
        output.stdout,
        b"<method><static><temp>:value:empty:code:none:none:called<present>:value<chosen><owner>"
    );
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
}

#[test]
fn owned_nullable_call_results_live_through_their_statement() {
    let source = r#"
class Tracked
{
    ?string $label = "property";
    function __construct(string $name) {}
    function __destruct() { echo "<" . $this->name . ">"; }
    function maybeLabel(): ?string { return "method"; }
}

function make(string $name): ?Tracked { return new Tracked($name); }
function inspect(?Tracked $value): void {}

function main(): void
{
    echo make("call")?->maybeLabel() ?? "none";
    echo ":";
    echo make("property")?->label ?? "none";
    inspect(make("argument"));
}
"#;
    let program = doriac::lower_source_to_mir("stage22-nullable-temporaries.doria", source)
        .expect("owned nullable call results should lower in receiver and argument positions");
    doriac::mir_validation::validate_program(&program)
        .expect("nullable temporary MIR should validate");
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("Cranelift should reserve and consume nullable temporary slots")
        .is_empty());
    #[cfg(feature = "llvm-backend")]
    assert!(!doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("LLVM should reserve and consume nullable temporary slots")
        .is_empty());
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("nullable receivers and arguments should drop after their statement");
    assert_eq!(output.stdout, b"method<call>:property<property><argument>");
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
}
