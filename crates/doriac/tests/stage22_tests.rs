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
fn narrowing_resolves_non_null_members_against_the_flow_class() {
    doriac::check_source(
        "stage22-flow-class-members.doria",
        r#"
class A
{
    static int $setting = 1;
    static function value(): int { return 1; }
    static function inspect(?int $value): void {}
    function read(): int
    {
        ?int $value = self::value();
        ?int $setting = self::setting;
        return $value + $setting;
    }
    function preserve(?int $value): int
    {
        if ($value == null) { return 0; }
        self::inspect($value);
        return $value + 1;
    }
}

class B
{
    static ?int $setting = null;
    static function value(): ?int { return null; }
    static function inspect(writable ?int $value): void { $value = null; }
}

class Reader
{
    int $count = 1;
    function number(): int { return 1; }
    function inspect(?int $value): void {}
}

class Writer
{
    ?int $count = null;
    function number(): ?int { return null; }
    function inspect(writable ?int $value): void { $value = null; }
}

function read(mixed $reader, ?int $value): int
{
    if ($reader is Reader) {
        ?int $number = $reader->number();
        ?int $count = $reader->count;
        if ($value == null) { return 0; }
        $reader->inspect($value);
        return $number + $count + $value;
    }
    return 0;
}
"#,
    )
    .expect("self-qualified and exact-class receivers should use their qualified member facts");
}

#[test]
fn incompatible_coalesce_operands_are_rejected_before_lowering() {
    let diagnostic = assert_code(
        r#"
function reject(?int $value): void
{
    let $result = $value ?? "bad";
}
"#,
        "E0512",
    );
    assert!(diagnostic.message.contains("`?int`"));
    assert!(diagnostic.message.contains("`string`"));
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
fn flow_joins_preserve_the_common_non_null_refinement() {
    doriac::check_source(
        "stage22-refinement-join.doria",
        r#"
function read(bool $condition, ?int $value): int
{
    if ($condition) {
        if (!($value is int)) { return 0; }
    } else {
        if ($value == null) { return 0; }
    }
    return $value + 1;
}
"#,
    )
    .expect("exact and non-null paths should join at the shared non-null refinement");
}

#[test]
fn copying_a_narrowed_variable_preserves_its_exact_type() {
    doriac::check_source(
        "stage22-exact-copy.doria",
        r#"
function read(take mixed $value): int
{
    if ($value is int) {
        mixed $copy = $value;
        return $copy + 1;
    }
    return 0;
}
"#,
    )
    .expect("copying an exact-narrowed value should preserve the exact fact");
}

#[test]
fn coalesce_preserves_the_exact_fact_of_a_selected_mixed_value() {
    doriac::check_source(
        "stage22-exact-coalesce.doria",
        r#"
function read(take mixed $value): int
{
    if ($value is int) {
        mixed $selected = $value ?? 0;
        return $selected + 1;
    }
    return 0;
}
"#,
    )
    .expect("a proven-present coalesce arm should retain its exact narrowed type");
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
fn skipped_short_circuit_calls_preserve_narrowing_facts() {
    doriac::check_source(
        "stage22-skipped-short-circuit-call.doria",
        r#"
function clear(writable ?int $value): bool { $value = null; return true; }
function read(?int $value): int
{
    if ($value == null) { return 0; }
    false && clear($value);
    true || clear($value);
    return $value + 1;
}
"#,
    )
    .expect("calls in skipped short-circuit operands must not invalidate flow facts");
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
    assert_eq!(
        output.stdout,
        b"42:7:typed:text:empty:label:none:label:fallback\n<fallback><label>\n"
    );
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
    static ?int8 $missing = null;
    static ?int8 $coalesced = null ?? 4;
    static int8 $smallFallback = self::small ?? 2;
    static int8 $missingFallback = self::missing ?? 3;
    writable ?uint8 $count = 2;
}

function accept(?int16 $value): void {}
function narrow(?int8 $value): int8 { return $value ?? 1; }
function narrowRatio(?float32 $value): float32 { return $value ?? 2.5; }

function main(): void
{
    ?int8 $small = 1;
    writable ?float32 $ratio = 1.5;
    $ratio = 2.5;
    accept(3);
    echo narrow(null);
    echo ":";
    echo narrowRatio(null);
    echo ":";
    echo Limits::smallFallback;
    echo ":";
    echo Limits::missingFallback;
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
    assert_eq!(output.stdout, b"1:2.5:1:3");
}

#[test]
fn borrowed_nullable_coalesce_cannot_initialize_an_owner() {
    let source = r#"
class Box {}

function alias(?Box $value): ?Box { return $value; }

function main(): void
{
    let $owner = new Box();
    ?Box $saved = alias($owner) ?? null;
}
"#;
    let diagnostics = doriac::check_source("stage22-borrowed-coalesce.doria", source)
        .expect_err("a borrowed nullable coalesce must not initialize an owning local");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0478"));
}

#[test]
fn nullable_coalesce_returns_preserve_borrow_provenance() {
    let program = doriac::lower_source_to_mir(
        "stage22-borrowed-coalesce-return.doria",
        r#"
class Box {}

function maybe(?Box $box): ?Box { return $box ?? null; }
function relay(?Box $box): ?Box { return maybe($box); }
function main(): void {}
"#,
    )
    .expect("a nullable coalesce with null should preserve its borrowed source");
    doriac::mir_validation::validate_program(&program)
        .expect("the borrowed nullable coalesce return should validate");
}

#[test]
fn nullable_class_call_arguments_enforce_ownership_and_writability() {
    let owned = doriac::check_source(
        "stage22-borrowed-nullable-take.doria",
        r#"
class Box {}

function alias(?Box $value): ?Box { return $value; }
function consume(take ?Box $value): void {}

function main(): void
{
    let $owner = new Box();
    consume(alias($owner) ?? null);
}
"#,
    )
    .expect_err("a borrowed nullable call cannot satisfy a take parameter");
    assert!(owned.iter().any(|diagnostic| diagnostic.code == "E0474"));

    let diagnostics = diagnostics(
        r#"
class Box {}
class Holder { writable ?Box $value = null; }
function mutate(writable ?Box $value): void {}
function invalid(?Box $value): void { mutate($value); }
function invalidNullSafe(?Holder $holder): void { mutate($holder?->value); }
"#,
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| {
                diagnostic.code == "E0204"
                    && diagnostic
                        .message
                        .contains("must be a writable class value")
            })
            .count(),
        2,
        "readonly and null-safe nullable class arguments must fail semantic checking: {diagnostics:#?}"
    );

    let mut writable = doriac::lower_source_to_mir(
        "stage22-writable-nullable-argument.doria",
        r#"
class Box {}
function mutate(writable ?Box $value): void {}
function main(): void
{
    writable ?Box $value = null;
    mutate($value);
}
"#,
    )
    .expect("a writable nullable class local should lower");
    let main = writable
        .functions
        .iter_mut()
        .find(|function| function.name == "main")
        .expect("fixture should contain main");
    main.locals
        .iter_mut()
        .find(|local| local.name == "value")
        .expect("fixture should contain value")
        .writable = false;
    let error = doriac::mir_validation::validate_program(&writable)
        .expect_err("defensive MIR validation must reject a readonly nullable class argument");
    assert!(error
        .message
        .contains("requires a writable nullable class value"));
}

#[test]
fn coalesce_borrows_all_possible_arms_across_call_arguments() {
    for (file, source) in [
        (
            "stage22-coalesce-call-borrow.doria",
            r#"
class Box {}

function pair(Box $borrowed, take Box $owned): void {}

function main(): void
{
    ?Box $maybe = null;
    let $box = new Box();
    pair($maybe ?? $box, $box);
}
"#,
        ),
        (
            "stage22-nullable-coalesce-call-borrow.doria",
            r#"
class Box {}

function pair(?Box $borrowed, take Box $owned): void {}

function main(): void
{
    ?Box $maybe = null;
    let $box = new Box();
    pair($maybe ?? $box, $box);
}
"#,
        ),
    ] {
        let program = doriac::lower_source_to_mir(file, source)
            .expect("coalesce call arguments should reach shared MIR ownership validation");
        let error = doriac::mir_validation::validate_program(&program)
            .expect_err("a possible coalesce borrow must conflict with a later transfer");
        assert!(error.message.contains("both borrows and transfers"));
    }
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
fn copy_take_arguments_preserve_narrowing_facts() {
    doriac::check_source(
        "stage22-copy-take-flow.doria",
        r#"
function inspect(take int $value): void {}
function read(?int $value): int
{
    if ($value == null) { return 0; }
    inspect($value);
    return $value + 1;
}
"#,
    )
    .expect("take is a semantic no-op for Copy arguments");
}

#[test]
fn promoted_properties_contribute_declared_nullability_facts() {
    doriac::check_source(
        "stage22-promoted-property-flow.doria",
        r#"
class Counter
{
    function __construct(int $count) {}
    function next(): int
    {
        ?int $copy = $this->count;
        return $copy + 1;
    }
}
"#,
    )
    .expect("a promoted non-null property should establish a non-null assignment fact");
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

    let $fallback = make(false, "missing") ?? new Tracked("owned-fallback");
    echo ":" . ($fallback->maybeName(true) ?? "none");
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
        b"<method><static><temp>:value:empty:code:none:none:called<present>:value:value<owned-fallback><chosen><owner>"
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

#[test]
fn typed_non_null_expressions_establish_nullable_flow_facts() {
    doriac::check_source(
        "stage22-typed-flow-facts.doria",
        r#"
function one(): int { return 1; }
function add(bool $condition, int $input): int
{
    ?int $fromParameter = $input;
    ?int $fromCall = one();
    writable ?int $joined = null;
    if ($condition) { $joined = $input; } else { $joined = one(); }
    return $fromParameter + $fromCall + $joined;
}
"#,
    )
    .expect("known non-null expression types should establish flow facts at every assignment");
}

#[test]
fn non_null_call_results_wrap_at_nullable_destinations() {
    let source = r#"
class Item
{
    function __construct(string $name) {}
    function label(): string { return $this->name; }
}

class Factory
{
    function number(): int { return 2; }
    function text(): string { return "method"; }
    function item(): Item { return new Item("method-item"); }
    static function staticNumber(): int { return 3; }
    static function staticText(): string { return "static"; }
    static function staticItem(): Item { return new Item("static-item"); }
}

function number(): int { return 1; }
function text(): string { return "free"; }
function item(): Item { return new Item("free-item"); }

function main(): void
{
    let $factory = new Factory();
    ?int $freeNumber = number();
    ?int $methodNumber = $factory->number();
    ?int $staticNumber = Factory::staticNumber();
    ?string $freeText = text();
    ?string $methodText = $factory->text();
    ?string $staticText = Factory::staticText();
    ?Item $freeItem = item();
    ?Item $methodItem = $factory->item();
    ?Item $staticItem = Factory::staticItem();
    echo $freeText . ":" . $methodText . ":" . $staticText;
    echo ":{$freeNumber}:{$methodNumber}:{$staticNumber}";
    echo ":" . $freeItem->label() . ":" . $methodItem->label() . ":" . $staticItem->label();
}
"#;
    let program = doriac::lower_source_to_mir("stage22-call-wrapping.doria", source)
        .expect("non-null call results should wrap at nullable destinations");
    doriac::mir_validation::validate_program(&program).expect("wrapped-call MIR should validate");
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("Cranelift should lower wrapped call results")
        .is_empty());
    #[cfg(feature = "llvm-backend")]
    assert!(!doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("LLVM should lower wrapped call results")
        .is_empty());
    let output =
        doriac::mir_interpreter::interpret(&program).expect("wrapped call results should execute");
    assert_eq!(
        output.stdout,
        b"free:method:static:1:2:3:free-item:method-item:static-item"
    );
}

#[test]
fn nullable_coalesce_preserves_nullable_results_across_payload_kinds() {
    let source = r#"
class Item
{
    function __construct(string $name) {}
    function label(): string { return $this->name; }
}

function chooseInt(?int $left, ?int $right): ?int { return $left ?? $right; }
function chooseString(?string $left, ?string $right): ?string { return $left ?? $right; }
function inspect(?Item $value): void { echo $value?->label() ?? "none"; }

function main(): void
{
    ?int $emptyInt = null;
    ?int $number = 7;
    ?string $emptyString = null;
    ?string $text = "text";
    ?Item $emptyItem = null;
    ?Item $item = new Item("item");
    let $selectedInt = chooseInt($emptyInt, $number);
    let $selectedText = chooseString($emptyString, $text);
    if ($selectedInt != null) { echo "7:"; }
    echo ($selectedText ?? "none") . ":";
    inspect($emptyItem ?? $item);
}
"#;
    let program = doriac::lower_source_to_mir("stage22-nullable-coalesce.doria", source)
        .expect("nullable coalesce should lower for scalar, string, and class payloads");
    doriac::mir_validation::validate_program(&program)
        .expect("nullable coalesce MIR should validate");
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("Cranelift should lower nullable coalesce")
        .is_empty());
    #[cfg(feature = "llvm-backend")]
    assert!(!doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("LLVM should lower nullable coalesce")
        .is_empty());
    let output =
        doriac::mir_interpreter::interpret(&program).expect("nullable coalesce should execute");
    assert_eq!(output.stdout, b"7:text:item");
}

#[test]
fn conditional_class_temporaries_and_borrowed_returns_keep_lifetime_order() {
    let source = r#"
class Tracked
{
    function __construct(string $name) {}
    function __destruct() { echo "<" . $this->name . ">"; }
    function use(Tracked $value): void { echo "use"; }
}

function make(string $name): ?Tracked { return new Tracked($name); }
function inspect(?Tracked $value): void { echo "inspect"; }
function identity(?Tracked $value): ?Tracked { echo "call"; return $value; }
function forward(?Tracked $value): ?Tracked
{
    let $temporary = new Tracked("local");
    return identity($value);
}

function main(): void
{
    ?Tracked $empty = null;
    inspect($empty ?? new Tracked("fallback"));
    make("receiver")?->use(new Tracked("argument"));
    let $owner = new Tracked("owner");
    inspect(forward($owner));
}
"#;
    let program = doriac::lower_source_to_mir("stage22-lifetime-order.doria", source)
        .expect("conditional temporaries and borrowed nullable calls should lower");
    doriac::mir_validation::validate_program(&program)
        .expect("conditional temporary MIR should validate");
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("Cranelift should preserve conditional temporary order")
        .is_empty());
    #[cfg(feature = "llvm-backend")]
    assert!(!doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("LLVM should preserve conditional temporary order")
        .is_empty());
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("conditional temporary lifetimes should execute");
    assert_eq!(
        output.stdout,
        b"inspect<fallback>use<argument><receiver>call<local>inspect<owner>"
    );
}

#[test]
fn repeatable_loop_assignments_do_not_escape_their_flow_paths() {
    let diagnostics = doriac::check_source(
        "stage22-loop-facts.doria",
        r#"
function fromFor(bool $condition): int
{
    writable ?int $value = null;
    for (; $condition;) { $value = 1; break; }
    return $value + 1;
}

function fromForeach(List<int> $items): int
{
    writable ?int $value = null;
    foreach ($items as int $item) { $value = $item; }
    return $value + 1;
}
"#,
    )
    .expect_err("zero-iteration loop paths must keep nullable bindings nullable");
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0441")
            .count(),
        2
    );
}

#[test]
fn known_null_facts_preserve_the_declared_nullable_container() {
    doriac::check_source(
        "stage22-known-null-container.doria",
        r#"
class Label { function text(): string { return "label"; } }
function read(): string
{
    ?Label $label = null;
    return $label?->text() ?? "none";
}
"#,
    )
    .expect("known-null values should retain their declared nullable class for null-safe access");
}

#[test]
fn receiver_types_select_their_own_method_flow_contracts() {
    doriac::check_source(
        "stage22-qualified-method-flow.doria",
        r#"
class Reader
{
    function touch(?int $value): void {}
    function number(): int { return 1; }
}

class Writer
{
    function touch(writable ?int $value): void { $value = null; }
    function number(): ?int { return null; }
}

function keep(?int $value): int
{
    let $reader = new Reader();
    if ($value != null) {
        $reader->touch($value);
        ?int $number = $reader->number();
        return $value + $number;
    }
    return 0;
}
"#,
    )
    .expect("method mutation and nullability facts should use the receiver's static class");
}

#[test]
fn self_typed_receivers_use_the_declaring_class_flow_contract() {
    let diagnostics = doriac::check_source(
        "stage22-self-method-flow.doria",
        r#"
class Writer
{
    function clear(writable ?int $value): void { $value = null; }

    function read(self $other, writable ?int $value): int
    {
        if ($value == null) { return 0; }
        $other->clear($value);
        return $value + 1;
    }
}
"#,
    )
    .expect_err("a self-typed mutating receiver call must invalidate nullable facts");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0441"),
        "the nullable use after the self-typed receiver call should be rejected: {diagnostics:#?}"
    );
}

#[test]
fn exact_self_property_receivers_and_coalesce_use_precise_flow_contracts() {
    let self_diagnostics = doriac::check_source(
        "stage22-exact-self-method-flow.doria",
        r#"
class Writer
{
    function clear(writable ?int $value): void { $value = null; }

    function read(mixed $other, writable ?int $value): int
    {
        if ($value != null && $other is self) {
            $other->clear($value);
            return $value + 1;
        }
        return 0;
    }
}
"#,
    )
    .expect_err("an exact self receiver must use the declaring class mutation contract");
    assert!(
        self_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0441"),
        "the nullable use after the exact-self call should be rejected: {self_diagnostics:#?}"
    );

    doriac::check_source(
        "stage22-property-receiver-and-coalesce-flow.doria",
        r#"
class Reader { function number(): int { return 1; } }
class Writer { function number(): ?int { return null; } }
class Holder
{
    function __construct(take Reader $reader) {}
    function read(): int
    {
        ?int $number = $this->reader->number();
        return $number + 1;
    }
}

function clear(writable ?int $value): int { $value = null; return 0; }
function keep(writable ?int $value): int
{
    ?int $present = 1;
    if ($value == null) { return 0; }
    let $ignored = $present ?? clear($value);
    return $value + 1;
}
"#,
    )
    .expect("property receiver types and skipped coalesce fallbacks should preserve exact facts");

    let fallback_diagnostics = diagnostics(
        r#"
function clear(writable ?int $value): int { $value = null; return 0; }
function read(?int $maybe, writable ?int $value): int
{
    if ($value == null) { return 0; }
    let $ignored = $maybe ?? clear($value);
    return $value + 1;
}
"#,
    );
    assert!(
        fallback_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0441"),
        "a reachable coalesce fallback must still invalidate facts: {fallback_diagnostics:#?}"
    );
}

#[test]
fn shared_validation_checks_null_safe_method_receivers() {
    let program = doriac::lower_source_to_mir(
        "stage22-null-safe-receiver-validation.doria",
        r#"
class Box { writable function number(): int { return 1; } }
function make(): ?Box { return new Box(); }
function main(): void
{
    writable ?Box $value = make();
    let $number = $value?->number();
}
"#,
    )
    .expect("a writable nullable receiver should lower");
    doriac::mir_validation::validate_program(&program)
        .expect("the valid nullable receiver should pass shared validation");

    let mut readonly = program.clone();
    readonly
        .functions
        .iter_mut()
        .find(|function| function.name == "main")
        .and_then(|function| {
            function
                .locals
                .iter_mut()
                .find(|local| local.name == "value")
        })
        .expect("fixture should contain the nullable receiver")
        .writable = false;
    let error = doriac::mir_validation::validate_program(&readonly)
        .expect_err("a readonly nullable receiver cannot call a writable method");
    assert!(error
        .message
        .contains("requires a writable nullable class value"));

    let mut transferred = program;
    let object = transferred
        .functions
        .iter_mut()
        .find(|function| function.name == "main")
        .and_then(|function| {
            function.blocks.iter_mut().find_map(|block| {
                block.statements.iter_mut().find_map(|statement| {
                    let doriac::mir::Statement::AssignLocal {
                        value:
                            doriac::mir::Rvalue::NullableScalar(
                                doriac::mir::NullableScalarExpression::NullSafeCall {
                                    object, ..
                                },
                            ),
                        ..
                    } = statement
                    else {
                        return None;
                    };
                    Some(object.as_mut())
                })
            })
        })
        .expect("fixture should contain a null-safe call");
    let doriac::mir::NullableClassExpression::Local { transfer, .. } = object else {
        panic!("fixture should lower its receiver as a nullable local")
    };
    *transfer = true;
    let error = doriac::mir_validation::validate_program(&transferred)
        .expect_err("a null-safe method receiver cannot be transferred");
    assert!(error.message.contains("transfers its receiver"));
}

#[test]
fn narrowed_nullable_scalars_support_read_modify_write() {
    let source = r#"
function initialValue(): ?int { return 40; }

function main(): void
{
    writable ?int $value = initialValue();
    if ($value != null) {
        $value += 1;
        $value++;
        echo $value;
    }
}
"#;
    let program = doriac::lower_source_to_mir("stage22-nullable-rmw.doria", source)
        .expect("narrowed nullable scalar read-modify-write should lower");
    doriac::mir_validation::validate_program(&program)
        .expect("nullable scalar read-modify-write MIR should validate");
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("Cranelift should lower nullable scalar read-modify-write")
        .is_empty());
    #[cfg(feature = "llvm-backend")]
    assert!(!doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("LLVM should lower nullable scalar read-modify-write")
        .is_empty());
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("nullable scalar read-modify-write should execute");
    assert_eq!(output.stdout, b"42");
}

#[test]
fn nullable_scalar_static_null_checks_are_const_evaluable() {
    let source = r#"
class Values
{
    static ?int8 $number = 1;
    static ?float32 $ratio = 1.5;
    static ?bool $enabled = null;
    static bool $hasNumber = self::number != null;
    static bool $hasRatio = self::ratio != null;
    static bool $hasEnabled = self::enabled != null;
}

function main(): void
{
    if (Values::hasNumber && Values::hasRatio && !Values::hasEnabled) {
        echo "ok";
    }
}
"#;
    let program = doriac::lower_source_to_mir("stage22-nullable-static-null.doria", source)
        .expect("nullable scalar static null checks should be const-evaluable");
    doriac::mir_validation::validate_program(&program)
        .expect("nullable scalar static null-check MIR should validate");
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("Cranelift should lower nullable scalar const null checks")
        .is_empty());
    #[cfg(feature = "llvm-backend")]
    assert!(!doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("LLVM should lower nullable scalar const null checks")
        .is_empty());
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("nullable scalar const null checks should execute");
    assert_eq!(output.stdout, b"ok");
}

#[test]
fn null_safe_string_comparisons_keep_nullable_semantics() {
    let source = r#"
class Label
{
    string $name = "x";
    function text(): string { return "x"; }
}

function label(bool $present): ?Label
{
    if ($present) { return new Label(); }
    return null;
}

function main(): void
{
    ?Label $none = label(false);
    ?Label $some = label(true);
    if ($none?->name != "x") { echo "none-property:"; }
    if ($none?->text() != "x") { echo "none-method:"; }
    if ($some?->name == "x") { echo "some-property:"; }
    if ($some?->text() == "x") { echo "some-method"; }
}
"#;
    let program = doriac::lower_source_to_mir("stage22-null-safe-string-compare.doria", source)
        .expect("null-safe string comparisons should lower as nullable comparisons");
    doriac::mir_validation::validate_program(&program)
        .expect("null-safe string comparison MIR should validate");
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("Cranelift should lower null-safe string comparisons")
        .is_empty());
    #[cfg(feature = "llvm-backend")]
    assert!(!doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("LLVM should lower null-safe string comparisons")
        .is_empty());
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("null-safe string comparisons should execute");
    assert_eq!(
        output.stdout,
        b"none-property:none-method:some-property:some-method"
    );
}

#[test]
fn concrete_is_tests_evaluate_their_operands_once() {
    let source = r#"
class Tracked
{
    function __destruct() { echo "<drop>"; }
}

function make(): Tracked { echo "make"; return new Tracked(); }
function maybe(): ?Tracked { echo "maybe"; return null; }

function main(): void
{
    if (make() is Tracked) { echo "true"; }
    if (make() is string) { echo "bad"; } else { echo "false"; }
    if (maybe() is string) { echo "bad"; } else { echo "false"; }
    if (null is int) { echo "bad"; } else { echo "null"; }
}
"#;
    let program = doriac::lower_source_to_mir("stage22-is-effects.doria", source)
        .expect("concrete and impossible type tests should preserve operand evaluation");
    doriac::mir_validation::validate_program(&program).expect("type-test MIR should validate");
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("Cranelift should lower effectful type tests")
        .is_empty());
    #[cfg(feature = "llvm-backend")]
    assert!(!doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("LLVM should lower effectful type tests")
        .is_empty());
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("effectful type tests should execute exactly once");
    assert_eq!(
        output.stdout,
        b"make<drop>truemake<drop>falsemaybefalsenull"
    );
}
