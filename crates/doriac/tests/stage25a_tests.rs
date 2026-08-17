//! Stage 25a — shared-ownership grammar, type model, and readonly runtime family
//! (record 0106).

use doriac::diagnostics::{
    ColorChoice, Diagnostic, DiagnosticFormat, RenderOptions, RuntimeFactValue,
};

const NODE: &str = r#"
class Node
{
    string $name = "";
    writable int $count = 0;

    function describe(): string { return $this->name; }
    writable function rename(): void {}
}

class Other {}
"#;

fn check(body: &str) -> Result<(), Vec<Diagnostic>> {
    let source =
        format!("{NODE}\nfunction main(): void throws Doria\\Std\\Io\\IoError\n{{\n{body}\n}}\n");
    doriac::check_source("stage25a.doria", &source).map(|_| ())
}

fn accepted(body: &str) {
    if let Err(diagnostics) = check(body) {
        panic!(
            "expected Stage 25a surface to be accepted, got: {:?}",
            diagnostics
                .iter()
                .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
                .collect::<Vec<_>>()
        );
    }
}

fn rejected(body: &str) -> Vec<Diagnostic> {
    check(body).expect_err("source should be rejected")
}

fn codes(diagnostics: &[Diagnostic]) -> Vec<&str> {
    diagnostics.iter().map(|entry| entry.code).collect()
}

fn assert_code(body: &str, code: &str) {
    let diagnostics = rejected(body);
    assert!(
        codes(&diagnostics).contains(&code),
        "expected {code}, got {:?}",
        diagnostics
            .iter()
            .map(|entry| format!("{}: {}", entry.code, entry.message))
            .collect::<Vec<_>>()
    );
}

#[test]
fn shared_access_conflicts_preserve_their_exact_structured_reason() {
    for (path, source, expected_reason) in [
        (
            "examples/native/main_stage25a_conflict_readonly_then_writable.doria",
            include_str!(
                "../../../examples/native/main_stage25a_conflict_readonly_then_writable.doria"
            ),
            doria_diagnostic_catalogue::READONLY_THEN_WRITABLE_CONFLICT,
        ),
        (
            "examples/native/main_stage25a_conflict_writable_then_readonly.doria",
            include_str!(
                "../../../examples/native/main_stage25a_conflict_writable_then_readonly.doria"
            ),
            doria_diagnostic_catalogue::WRITABLE_THEN_READONLY_CONFLICT,
        ),
        (
            "examples/native/main_stage25a_conflict_writable_then_writable.doria",
            include_str!(
                "../../../examples/native/main_stage25a_conflict_writable_then_writable.doria"
            ),
            doria_diagnostic_catalogue::WRITABLE_THEN_WRITABLE_CONFLICT,
        ),
    ] {
        let program = doriac::lower_source_to_mir(path, source)
            .expect("conflict fixture should lower before its runtime panic");
        let output = doriac::mir_interpreter::interpret(&program)
            .expect("conflict fixture should produce a structured runtime panic");
        let diagnostic = output
            .runtime_diagnostic
            .as_ref()
            .expect("conflict fixture should retain its runtime diagnostic");
        assert_eq!(diagnostic.code, "P1501");
        assert_eq!(diagnostic.explanation.as_deref(), Some(expected_reason));
        assert!(diagnostic
            .runtime_outcome
            .as_ref()
            .expect("runtime outcome")
            .facts
            .iter()
            .any(|fact| {
                fact.name == doria_diagnostic_catalogue::SHARED_ACCESS_CONFLICT_REASON_FACT
                    && matches!(
                        &fact.value,
                        RuntimeFactValue::StaticString(value) if value == expected_reason
                    )
            }));
        assert!(String::from_utf8_lossy(&output.stderr).contains(expected_reason));

        let rendered_source = doriac::source::SourceFile::new(path, source);
        let json = doriac::diagnostics::render_diagnostics(
            &rendered_source,
            std::slice::from_ref(diagnostic),
            RenderOptions {
                format: DiagnosticFormat::Json,
                color: ColorChoice::Never,
                ..RenderOptions::default()
            },
        );
        assert!(json.contains("conflictReason"));
        assert!(json.contains(expected_reason));
    }
}

// --- Grammar -------------------------------------------------------------

#[test]
fn shared_new_parses_without_errors() {
    doriac::parse_source(
        "stage25a-syntax.doria",
        "function main(): void throws Doria\\Std\\Io\\IoError { let $node = shared new Node(); }",
    )
    .expect("`shared new` is accepted Stage 25a syntax and must parse cleanly");
}

#[test]
fn all_six_compiler_known_types_parse_in_type_position() {
    for name in [
        "SharedReference",
        "WeakReference",
        "WritableSharedReference",
        "WritableWeakReference",
        "ReadonlySharedReferenceAccess",
        "WritableSharedReferenceAccess",
    ] {
        let source = format!("function accept({name}<Node> $value): void {{}}");
        doriac::parse_source("stage25a-types.doria", &source)
            .unwrap_or_else(|_| panic!("`{name}<T>` must parse as accepted Stage 25a syntax"));
    }
}

#[test]
fn superseded_shared_declaration_form_is_rejected_with_migration_help() {
    let diagnostics = rejected("shared Node $node = shared new Node();");
    assert!(
        codes(&diagnostics).contains(&"E0541"),
        "expected the superseded `shared T $value` form to be rejected, got {:?}",
        codes(&diagnostics)
    );
}

#[test]
fn shared_writable_new_chain_is_rejected() {
    assert_code("let $bad = shared writable new Node();", "E0540");
}

#[test]
fn weak_new_is_not_doria_syntax() {
    let source =
        "function main(): void throws Doria\\Std\\Io\\IoError { let $bad = weak new Node(); }";
    assert!(
        doriac::parse_source("stage25a-weak-new.doria", source).is_err(),
        "`weak new` is not Doria syntax and must not parse"
    );
}

// --- Type model ----------------------------------------------------------

#[test]
fn shared_new_has_shared_reference_type_and_plain_new_stays_owned() {
    accepted(
        r#"
    SharedReference<Node> $shared = shared new Node();
    Node $owned = new Node();
"#,
    );
}

#[test]
fn shared_new_does_not_produce_an_owned_value() {
    assert_code("Node $wrong = shared new Node();", "E0403");
}

#[test]
fn plain_new_does_not_produce_a_shared_reference() {
    assert_code("SharedReference<Node> $wrong = new Node();", "E0403");
}

#[test]
fn each_compiler_known_type_takes_exactly_one_type_argument() {
    assert_code(
        "SharedReference<Node, Node> $bad = shared new Node();",
        "E0546",
    );
    assert_code("WeakReference $bad = shared new Node();", "E0546");
}

#[test]
fn superseded_names_are_rejected_with_replacements() {
    for (old, replacement) in [
        ("Shared", "SharedReference"),
        ("Weak", "WeakReference"),
        ("SharedMut", "WritableSharedReference"),
    ] {
        let diagnostics = rejected(&format!("{old}<Node> $value = shared new Node();"));
        let help = diagnostics
            .iter()
            .find(|entry| entry.code == "E0547")
            .and_then(|entry| entry.help.clone())
            .unwrap_or_default();
        assert!(
            help.contains(replacement),
            "`{old}<T>` should point at `{replacement}<T>`, got help {help:?}"
        );
    }
}

#[test]
fn compiler_known_names_cannot_be_redeclared() {
    let source =
        "class SharedReference {}\nfunction main(): void throws Doria\\Std\\Io\\IoError {}";
    let diagnostics =
        doriac::check_source("stage25a-redeclare.doria", source).expect_err("must be rejected");
    assert!(
        diagnostics
            .iter()
            .any(|entry| entry.message.contains("cannot be redeclared")),
        "compiler-known shared-ownership names must be protected, got {:?}",
        codes(&diagnostics)
    );
}

// --- Construction --------------------------------------------------------

#[test]
fn shared_reference_is_not_constructed_with_new() {
    assert_code("let $bad = new SharedReference(new Node());", "E0543");
}

#[test]
fn weak_and_access_types_cannot_be_constructed_directly() {
    for name in [
        "WeakReference",
        "WritableWeakReference",
        "ReadonlySharedReferenceAccess",
        "WritableSharedReferenceAccess",
    ] {
        assert_code(&format!("let $bad = new {name}(new Node());"), "E0543");
    }
}

#[test]
fn writable_shared_reference_takes_exactly_one_owned_value() {
    accepted("let $settings = new WritableSharedReference(new Node());");
    accepted("let $settings = new WritableSharedReference(value: new Node());");
    assert_code("let $bad = new WritableSharedReference();", "E0544");
    assert_code(
        "let $bad = new WritableSharedReference<Node>(new Other());",
        "E0408",
    );
    assert_code(
        "let $bad = new WritableSharedReference<Node>(payload: new Node());",
        "E0516",
    );
    assert_code(
        r#"
    let $node = new Node();
    let $settings = new WritableSharedReference<Node>($node);
    echo $node->name;
"#,
        "E0470",
    );
}

#[test]
fn shared_new_never_constructs_a_writable_family_value() {
    assert_code(
        "let $bad = shared new WritableSharedReference(new Node());",
        "E0542",
    );
}

// --- Members -------------------------------------------------------------

#[test]
fn compiler_known_members_resolve_with_their_approved_return_types() {
    accepted(
        r#"
    let $node = shared new Node();
    SharedReference<Node> $another = $node->share();
    WeakReference<Node> $weak = $node->createWeakReference();
    ?SharedReference<Node> $live = $weak->acquire();

    let $settings = new WritableSharedReference(new Node());
    WritableSharedReference<Node> $secondHandle = $settings->share();
    WritableWeakReference<Node> $writableWeak = $settings->createWeakReference();
    ?WritableSharedReference<Node> $writableLive = $writableWeak->acquire();
    ReadonlySharedReferenceAccess<Node> $readonlyAccess =
        $settings->acquireReadonlyAccess();
    WritableSharedReferenceAccess<Node> $writableAccess =
        $settings->acquireWritableAccess();
"#,
    );
}

#[test]
fn weak_acquisition_is_nullable_and_stays_within_its_family() {
    // A readonly weak reference never acquires the writable family.
    assert_code(
        r#"
    let $node = shared new Node();
    let $weak = $node->createWeakReference();
    ?WritableSharedReference<Node> $wrong = $weak->acquire();
"#,
        "E0403",
    );
}

#[test]
fn non_conflicting_members_forward_transparently_to_the_payload() {
    accepted(
        r#"
    let $node = shared new Node();
    echo $node->name;
    echo $node->describe();
"#,
    );
}

#[test]
fn forwarded_payload_methods_preserve_take_parameters_in_ownership_analysis() {
    let source = r#"
class Child {}

class Consumer
{
    function consume(take Child $child): void {}
}

function consumeAgain(take Child $child): void {}

function main(): void throws Doria\Std\Io\IoError
{
    let $consumer = shared new Consumer();
    let $child = new Child();
    $consumer->consume($child);
    consumeAgain($child);
}
"#;
    let diagnostics = doriac::check_source("stage25a-forwarded-take.doria", source)
        .expect_err("forwarded payload calls must preserve ownership signatures");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0470"),
        "expected use-after-move through forwarded payload method, got {diagnostics:?}"
    );
}

#[test]
fn access_objects_forward_without_a_value_wrapper() {
    accepted(
        r#"
    let $settings = new WritableSharedReference(new Node());
    let $access = $settings->acquireReadonlyAccess();
    echo $access->name;
    echo $access->describe();
"#,
    );
}

#[test]
fn writable_access_mutates_the_shared_class_payload() {
    let source = format!(
        "{NODE}
function main(): void throws Doria\\Std\\Io\\IoError
{{
    let $shared = new WritableSharedReference(new Node());
    let writable $write = $shared->acquireWritableAccess();
    $write->count = 7;
    echo \"{{$write->count}}\\n\";
}}
"
    );
    let program = doriac::lower_source_to_mir("stage25a-writable-access.doria", &source)
        .expect("writable class access should lower");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("writable class access should interpret");
    assert_eq!(output.stdout, b"7\n");
    doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("writable class access should lower through Cranelift");
}

#[test]
fn writable_access_mutates_the_shared_collection_payload() {
    let source = r#"
function main(): void throws Doria\Std\Io\IoError
{
    let $shared = new WritableSharedReference([1, 2, 3]);
    let writable $access = $shared->acquireWritableAccess();
    $access[0] = 10;
    $access->add(4);
    echo "{$access[0]}:{$access->count}\n";
}
"#;
    let program = doriac::lower_source_to_mir("stage25a-writable-collection.doria", source)
        .expect("writable collection access should lower");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("writable collection access should interpret");
    assert_eq!(output.stdout, b"10:4\n");
    doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("writable collection access should lower through Cranelift");
}

#[test]
fn writable_shared_reference_does_not_forward_directly() {
    assert_code(
        r#"
    let $settings = new WritableSharedReference(new Node());
    echo $settings->name;
"#,
        "E0548",
    );
}

#[test]
fn weak_references_have_no_live_value_to_access() {
    assert_code(
        r#"
    let $node = shared new Node();
    let $weak = $node->createWeakReference();
    echo $weak->name;
"#,
        "E0549",
    );
}

// --- referencedValue and collisions --------------------------------------

#[test]
fn referenced_value_resolves_as_the_readonly_payload() {
    accepted(
        r#"
    let $node = shared new Node();
    echo $node->referencedValue->name;
    echo $node->referencedValue->describe();
"#,
    );
}

#[test]
fn referenced_value_cannot_move_the_payload_out() {
    // Record 0106: the projection never moves or consumes the underlying `T`.
    assert_code(
        r#"
    let $node = shared new Node();
    Node $stolen = $node->referencedValue;
"#,
        "E0472",
    );
}

#[test]
fn wrapper_members_win_on_a_direct_collision_and_the_payload_stays_reachable() {
    let source = r#"
class Document
{
    string $title = "";
    function share(): string { return "domain"; }
}

function main(): void throws Doria\Std\Io\IoError
{
    let $document = shared new Document();
    // The wrapper member wins on the direct receiver.
    SharedReference<Document> $anotherOwner = $document->share();
    // The payload member stays reachable through the explicit projection.
    string $domain = $document->referencedValue->share();
    echo $document->title;
}
"#;
    doriac::check_source("stage25a-collision.doria", source)
        .expect("a payload member colliding with a wrapper member must remain reachable");
}

#[test]
fn referenced_value_exists_only_on_shared_reference() {
    assert_code(
        r#"
    let $settings = new WritableSharedReference(new Node());
    let $bad = $settings->referencedValue;
"#,
        "E0548",
    );
}

#[test]
fn writes_through_referenced_value_are_rejected() {
    assert_code(
        r#"
    let $node = shared new Node();
    $node->referencedValue->count = 5;
"#,
        "E0201",
    );
}

#[test]
fn writes_through_a_shared_reference_are_rejected() {
    assert_code(
        r#"
    let $node = shared new Node();
    $node->count = 5;
"#,
        "E0201",
    );
    assert_code(
        r#"
    let writable $node = shared new Node();
    $node->count = 5;
"#,
        "E0201",
    );
    assert_code(
        r#"
    let writable $node = shared new Node();
    $node->rename();
"#,
        "E0203",
    );
}

#[test]
fn nullable_shared_members_are_lazy_and_preserve_handle_families() {
    let source = r#"
class Node
{
    function __construct(string $name) {}
    function __destruct() { try { echo "drop " . $this->name . "\n"; } catch (Doria\Std\Io\IoError) {} }
    function label(): string { return $this->name; }
}

function main(): void throws Doria\Std\Io\IoError
{
    let $root = shared new Node("root");
    ?SharedReference<Node> $present = $root->share();
    ?SharedReference<Node> $missing = null;
    ?WeakReference<Node> $presentWeak = $present?->createWeakReference();
    ?WeakReference<Node> $missingWeak = $missing?->createWeakReference();
    ?SharedReference<Node> $sharedAgain = $present?->share();
    ?SharedReference<Node> $absentShare = $missing?->share();
    ?SharedReference<Node> $acquired = $presentWeak?->acquire();
    ?SharedReference<Node> $absentAcquire = $missingWeak?->acquire();
    echo ($present?->name ?? "missing") . "\n";
    echo ($missing?->name ?? "missing") . "\n";
    echo ($present?->label() ?? "missing") . "\n";
    echo ($missing?->label() ?? "missing") . "\n";
    echo ($present?->referencedValue?->name ?? "missing") . "\n";
    echo ($missing?->referencedValue?->name ?? "missing") . "\n";
    if ($sharedAgain != null) { echo $sharedAgain->name . "\n"; }
    if ($absentShare == null) { echo "no share\n"; }
    if ($acquired != null) { echo $acquired->name . "\n"; }
    if ($absentAcquire == null) { echo "no acquire\n"; }
}
"#;
    let program = doriac::lower_source_to_mir("stage25a-null-safe-shared.doria", source)
        .expect("nullable shared members should lower lazily");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("nullable shared members should interpret");
    assert_eq!(
        output.stdout,
        b"root\nmissing\nroot\nmissing\nroot\nmissing\nroot\nno share\nroot\nno acquire\ndrop root\n"
    );
    doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("nullable shared members should lower through Cranelift");
    #[cfg(feature = "llvm-backend")]
    doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("nullable shared members should lower through LLVM");
}

// --- Family disjointness -------------------------------------------------

#[test]
fn family_crossing_assignments_are_rejected() {
    assert_code(
        r#"
    let $node = shared new Node();
    WritableSharedReference<Node> $wrong = $node;
"#,
        "E0403",
    );
    assert_code(
        r#"
    let $settings = new WritableSharedReference(new Node());
    SharedReference<Node> $wrong = $settings;
"#,
        "E0403",
    );
}

#[test]
fn family_crossing_arguments_are_rejected() {
    let source = r#"
class Node { string $name = ""; }

function accept(SharedReference<Node> $value): void {}

function main(): void throws Doria\Std\Io\IoError
{
    let $settings = new WritableSharedReference(new Node());
    accept($settings);
}
"#;
    let diagnostics =
        doriac::check_source("stage25a-family-args.doria", source).expect_err("must be rejected");
    assert!(
        codes(&diagnostics).contains(&"E0408"),
        "a writable handle must not bind to a readonly-family parameter, got {:?}",
        codes(&diagnostics)
    );
}

#[test]
fn weak_families_do_not_convert() {
    assert_code(
        r#"
    let $settings = new WritableSharedReference(new Node());
    WeakReference<Node> $wrong = $settings->createWeakReference();
"#,
        "E0403",
    );
}

// --- Move classification -------------------------------------------------

#[test]
fn every_handle_and_access_object_is_a_move_type() {
    // Plain assignment transfers the handle; the source is moved-from.
    assert_code(
        r#"
    let $first = shared new Node();
    let $second = $first;
    echo $first->name;
"#,
        "E0470",
    );
}

#[test]
fn sharing_is_explicit_rather_than_implicit_retention() {
    accepted(
        r#"
    let $first = shared new Node();
    let $second = $first->share();
    echo $first->name;
    echo $second->name;
"#,
    );
}

#[test]
fn shared_handle_returns_transfer_instead_of_extending_a_borrow() {
    let source = r#"
class Node {}

function invalid(SharedReference<Node> $value): SharedReference<Node>
{
    return $value;
}
"#;
    let diagnostics = doriac::check_source("stage25a-return.doria", source)
        .expect_err("a borrowed handle cannot become an owning return");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0474"),
        "expected an ownership-return diagnostic, got {diagnostics:?}"
    );
}

// --- Readonly runtime family ---------------------------------------------

#[test]
fn readonly_shared_family_lowers_and_interprets() {
    let source = r#"
class Node
{
    string $name = "Root";

    function __destruct() { try { echo "drop " . $this->name . "\n"; } catch (Doria\Std\Io\IoError) {} }
    function describe(): string { return $this->name; }
}

function consume(take SharedReference<Node> $value): void {}

function main(): void throws Doria\Std\Io\IoError
{
    let $first = shared new Node();
    let $weak = $first->createWeakReference();
    let $second = $first->share();
    echo $first->name . " " . $second->referencedValue->describe() . "\n";
    consume($second);
    let $live = $weak->acquire();
    if ($live != null) { echo $live->name . "\n"; }
}
"#;
    let program = doriac::lower_source_to_mir("stage25a-runtime.doria", source)
        .expect("readonly shared ownership should lower to MIR");
    let output =
        doriac::mir_interpreter::interpret(&program).expect("readonly shared MIR should interpret");
    assert_eq!(output.stdout, b"Root Root\nRoot\ndrop Root\n");
    doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("readonly shared MIR should lower through Cranelift");
    #[cfg(feature = "llvm-backend")]
    doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("readonly shared MIR should lower through LLVM");
}

#[test]
fn derived_shared_operations_preserve_and_release_owned_receivers() {
    let source = r#"
class Node
{
    function __construct(string $name) {}
    function label(): string { return $this->name; }
    function __destruct() { try { echo "drop " . $this->name . "\n"; } catch (Doria\Std\Io\IoError) {} }
}

function makeStrong(string $name): SharedReference<Node>
{
    return shared new Node($name);
}

function makeWeak(string $name): WeakReference<Node>
{
    return (shared new Node($name))->createWeakReference();
}

function main(): void throws Doria\Std\Io\IoError
{
    let $shared = makeStrong("shared")->share();
    echo $shared->name . "\n";
    let $weak = makeStrong("weak")->createWeakReference();
    let $expired = $weak->acquire();
    if ($expired == null) { echo "expired\n"; }
    let $acquired = makeWeak("acquire")->acquire();
    if ($acquired == null) { echo "missing\n"; }
    echo makeStrong("payload")->referencedValue->label() . "\n";
}
"#;
    let program = doriac::lower_source_to_mir("stage25a-derived-temporaries.doria", source)
        .expect("derived shared operations should lower with explicit owner temporaries");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("derived shared operations should execute without leaking owners");
    assert_eq!(
        output.stdout,
        b"shared\ndrop weak\nexpired\ndrop acquire\nmissing\npayload\ndrop payload\ndrop shared\n"
    );
    doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("derived shared operations should lower through Cranelift");
    #[cfg(feature = "llvm-backend")]
    doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("derived shared operations should lower through LLVM");
}

#[test]
fn shared_rebinding_coalescing_and_foreach_preserve_handle_ownership() {
    let source = r#"
class Node
{
    function __construct(string $name) {}
    function __destruct() { try { echo "drop " . $this->name . "\n"; } catch (Doria\Std\Io\IoError) {} }
}

function show(SharedReference<Node> $node): void throws Doria\Std\Io\IoError
{
    echo $node->name . "\n";
}

function main(): void throws Doria\Std\Io\IoError
{
    let writable $current = shared new Node("old");
    $current = shared new Node("current");

    let $expired = (shared new Node("expired"))->createWeakReference();
    let $fallback = $expired->acquire() ?? shared new Node("fallback");
    show($fallback);

    let writable $weak = $current->createWeakReference();
    $weak = $fallback->createWeakReference();
    writable ?SharedReference<Node> $maybe = null;
    $maybe = $current->share();

    let $first = shared new Node("first");
    let $second = shared new Node("second");
    SharedReference<Node>[] $nodes = [$first->share(), $second->share()];
    foreach ($nodes as SharedReference<Node> $node) {
        show($node);
    }
    show($first);
    show($second);
}
"#;
    let program = doriac::lower_source_to_mir("stage25a-composition.doria", source)
        .expect("shared ownership composition fixture should lower");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("shared ownership composition fixture should interpret");
    assert_eq!(
        output.stdout,
        b"drop old\ndrop expired\nfallback\nfirst\nsecond\nfirst\nsecond\ndrop second\ndrop first\ndrop fallback\ndrop current\n"
    );
    doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("shared ownership composition should lower through Cranelift");
    #[cfg(feature = "llvm-backend")]
    doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("shared ownership composition should lower through LLVM");
}

#[test]
fn borrowed_coalesces_type_tests_and_temporary_receivers_preserve_lifetimes() {
    let source = r#"
class Node
{
    function __construct(string $name) {}
    function label(): string { return $this->name; }
    function __destruct() { try { echo "drop " . $this->name . "\n"; } catch (Doria\Std\Io\IoError) {} }
}

function makeStrong(string $name): SharedReference<Node> throws Doria\Std\Io\IoError
{
    echo "make " . $name . "\n";
    return shared new Node($name);
}

function inspect(?SharedReference<Node> $candidate, SharedReference<Node> $fallback): void throws Doria\Std\Io\IoError
{
    echo ($candidate ?? $fallback)->name . "\n";
    echo $fallback->name . "\n";
}

function main(): void throws Doria\Std\Io\IoError
{
    let $fallback = makeStrong("fallback");
    ?SharedReference<Node> $missing = null;
    inspect($missing, $fallback);

    if (makeStrong("tested") is Node) {
        echo "tested\n";
    }

    echo makeStrong("call")->name . "\n";
    let $root = makeStrong("root");
    echo $root->share()->label() . "\n";
}
"#;
    let program = doriac::lower_source_to_mir("stage25a-borrowed-composition.doria", source)
        .expect("borrowed shared composition should lower");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("borrowed shared composition should execute");
    assert_eq!(
        output.stdout,
        b"make fallback\nfallback\nfallback\nmake tested\ndrop tested\nmake call\ncall\ndrop call\nmake root\nroot\ndrop root\ndrop fallback\n"
    );
    doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("borrowed shared composition should lower through Cranelift");
    #[cfg(feature = "llvm-backend")]
    doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("borrowed shared composition should lower through LLVM");
}

#[test]
fn shared_coalesces_cleanup_only_the_selected_owned_branch() {
    let source = r#"
class Node
{
    function __construct(string $name) {}
    function __destruct() { try { echo "drop " . $this->name . "\n"; } catch (Doria\Std\Io\IoError) {} }
}

function makeStrong(string $name): SharedReference<Node> throws Doria\Std\Io\IoError
{
    echo "make strong " . $name . "\n";
    return shared new Node($name);
}

function makeWeak(
    SharedReference<Node> $strong,
    string $label,
): WeakReference<Node> throws Doria\Std\Io\IoError
{
    echo "make weak " . $label . "\n";
    return $strong->createWeakReference();
}

function inspect(SharedReference<Node> $node): void throws Doria\Std\Io\IoError
{
    echo "inspect " . $node->name . "\n";
}

function main(): void throws Doria\Std\Io\IoError
{
    let $owner = shared new Node("Existing");
    let $weak = $owner->createWeakReference();
    let $maybe = $weak->acquire();

    inspect($maybe ?? makeStrong("unused"));
    if ($maybe != null) { echo "maybe " . $maybe->name . "\n"; }
    echo "owner " . $owner->name . "\n";

    ?SharedReference<Node> $absentStrong = null;
    inspect($absentStrong ?? makeStrong("Fallback"));

    ?WeakReference<Node> $presentWeak = $weak;
    let $selectedPresent = $presentWeak
        ?? makeWeak($owner, "unused");

    ?WeakReference<Node> $absentWeak = null;
    let $selectedFallback = $absentWeak
        ?? makeWeak($owner, "fallback");

    let $presentLive = $selectedPresent->acquire();
    if ($presentLive != null) { echo "present " . $presentLive->name . "\n"; }
    let $fallbackLive = $selectedFallback->acquire();
    if ($fallbackLive != null) { echo "fallback " . $fallbackLive->name . "\n"; }
}
"#;
    let program = doriac::lower_source_to_mir("stage25a-coalesce-ownership.doria", source)
        .expect("strong and weak coalesces should preserve selected-branch ownership");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("strong and weak coalesces should execute without ownership loss");
    assert_eq!(
        output.stdout,
        b"inspect Existing\nmaybe Existing\nowner Existing\nmake strong Fallback\ninspect Fallback\ndrop Fallback\nmake weak fallback\npresent Existing\nfallback Existing\ndrop Existing\n"
    );
    doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("coalesce ownership should lower through Cranelift");
    #[cfg(feature = "llvm-backend")]
    doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("coalesce ownership should lower through LLVM");
}

#[test]
fn nullable_shared_coalesces_preserve_nullable_results_and_owned_branches() {
    let source = r#"
class Node
{
    function __construct(string $name) {}
    function __destruct() { try { echo "drop " . $this->name . "\n"; } catch (Doria\Std\Io\IoError) {} }
}

function chooseStrong(
    take ?SharedReference<Node> $left,
    take ?SharedReference<Node> $right,
): ?SharedReference<Node>
{
    return $left ?? $right;
}

function chooseWeak(
    take ?WeakReference<Node> $left,
    take ?WeakReference<Node> $right,
): ?WeakReference<Node>
{
    return $left ?? $right;
}

function chooseStrongOrNull(
    take ?SharedReference<Node> $value,
): ?SharedReference<Node>
{
    return $value ?? null;
}

function chooseWeakOrNull(
    take ?WeakReference<Node> $value,
): ?WeakReference<Node>
{
    return $value ?? null;
}

function main(): void throws Doria\Std\Io\IoError
{
    let $left = shared new Node("left");
    let $right = shared new Node("right");

    let $strongFallback = chooseStrong(null, $right->share());
    if ($strongFallback != null) {
        echo "strong fallback " . $strongFallback->name . "\n";
    }

    let $strongPresent = chooseStrong($left->share(), null);
    if ($strongPresent != null) {
        echo "strong present " . $strongPresent->name . "\n";
    }

    let $weakFallback = chooseWeak(null, $right->createWeakReference());
    if ($weakFallback != null) {
        echo "weak fallback\n";
    }

    let $weakPresent = chooseWeak($left->createWeakReference(), null);
    if ($weakPresent != null) {
        echo "weak present\n";
    }

    let $strongNull = chooseStrongOrNull(null);
    let $weakNull = chooseWeakOrNull(null);
    if ($strongNull == null && $weakNull == null) {
        echo "null fallbacks\n";
    }
}
"#;
    let program = doriac::lower_source_to_mir("stage25a-nullable-coalesce.doria", source)
        .expect("nullable strong and weak coalesces should lower to shared MIR");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("nullable strong and weak coalesces should preserve ownership");
    assert_eq!(
        output.stdout,
        b"strong fallback right\nstrong present left\nweak fallback\nweak present\nnull fallbacks\ndrop right\ndrop left\n"
    );
    doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("nullable shared coalesces should lower through Cranelift");
    #[cfg(feature = "llvm-backend")]
    doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("nullable shared coalesces should lower through LLVM");
}

#[test]
fn shared_handle_statics_report_the_native_capability_boundary() {
    for handle in ["SharedReference", "WeakReference"] {
        let source = format!(
            r#"
class Node {{}}

class Store
{{
    static writable ?{handle}<Node> $value = null;
}}

function main(): void throws Doria\Std\Io\IoError {{}}
"#
        );
        let diagnostics = doriac::check_source("stage25a-shared-static.doria", &source)
            .expect_err("owned static storage is deferred pending its concurrency model");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0486"
                    && diagnostic.message.contains("cannot use owned type")),
            "{diagnostics:?}"
        );
        assert!(
            diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code != "E0483"),
            "the owned-static diagnostic should not be obscured by const evaluation: {diagnostics:?}"
        );
    }
}

#[test]
fn borrowed_dictionary_shared_results_do_not_acquire_cleanup_obligations() {
    let source = r#"
class Node
{
    function __destruct() { try { echo "drop\n"; } catch (Doria\Std\Io\IoError) {} }
}

function main(): void throws Doria\Std\Io\IoError
{
    let $root = shared new Node();
    Dictionary<string, SharedReference<Node>> $values = ["node" => $root->share()];
    let $borrowed = $values->get("node");
    if ($borrowed != null) { echo "found\n"; }
}
"#;
    let program = doriac::lower_source_to_mir("stage25a-borrowed-dictionary.doria", source)
        .expect("a borrowed collection result may be bound while its owner remains live");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("a borrowed collection result must not gain a cleanup obligation");
    assert_eq!(output.stdout, b"found\ndrop\n");
}

#[test]
fn generic_repository_returns_transitive_borrows_from_its_receiver() {
    let source = r#"
class Repository<T>
{
    internal writable Dictionary<string, T> $items = [];

    writable function save(string $id, take T $item): void
    {
        $this->items[$id] = $item;
    }

    function find(string $id): ?T
    {
        return $this->items->get($id);
    }
}

class Customer { function __construct(string $name) {} }
class Invoice { function __construct(string $number) {} }

function main(): void throws Doria\Std\Io\IoError
{
    let writable $customers = new Repository<Customer>();
    let writable $invoices = new Repository<Invoice>();
    $customers->save("42", new Customer("Maya"));
    $invoices->save("42", new Invoice("INV-42"));

    let $customer = $customers->find("42");
    let $invoice = $invoices->find("42");
    echo ($customer?->name ?? "missing") . "\n";
    echo ($invoice?->number ?? "missing") . "\n";
}
"#;
    let program = doriac::lower_source_to_mir("stage25a-generic-repository.doria", source)
        .expect("property and collection projections must preserve receiver borrow provenance");
    doriac::mir_validation::validate_program(&program)
        .expect("transitive returned borrows must produce valid shared MIR");
    let output =
        doriac::mir_interpreter::interpret(&program).expect("generic repository should execute");
    assert_eq!(output.stdout, b"Maya\nINV-42\n");
}

#[test]
fn weak_reference_survives_payload_destruction_without_resurrection() {
    let source = r#"
class Node
{
    function __construct(string $name) {}
    function __destruct() { try { echo "drop " . $this->name . "\n"; } catch (Doria\Std\Io\IoError) {} }
}

function makeWeak(): WeakReference<Node>
{
    let $strong = shared new Node("Expired");
    return $strong->createWeakReference();
}

function main(): void throws Doria\Std\Io\IoError
{
    let $weak = makeWeak();
    let $live = $weak->acquire();
    if ($live == null) { echo "expired\n"; }
}
"#;
    let program = doriac::lower_source_to_mir("stage25a-expired.doria", source)
        .expect("weak lifetime fixture should lower");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("weak lifetime fixture should interpret");
    assert_eq!(output.stdout, b"drop Expired\nexpired\n");
}

#[test]
fn nullable_weak_references_preserve_weak_ownership_across_storage_paths() {
    let source = r#"
class Node
{
    function __construct(string $name) {}
    function __destruct() { try { echo "drop " . $this->name . "\n"; } catch (Doria\Std\Io\IoError) {} }
}

class Holder
{
    function __construct(take ?WeakReference<Node> $weak) {}
}

function maybeWeak(SharedReference<Node> $root): ?WeakReference<Node>
{
    return $root->createWeakReference();
}

function main(): void throws Doria\Std\Io\IoError
{
    let $root = shared new Node("root");
    writable ?WeakReference<Node> $maybe = null;
    if ($maybe == null) { echo "empty\n"; }

    $maybe = maybeWeak($root);
    if ($maybe != null) {
        let $live = $maybe->acquire();
        if ($live != null) { echo $live->name . "\n"; }
    }

    let $holder = new Holder($root->createWeakReference());
    if ($holder->weak != null) { echo "stored\n"; }

    Dictionary<string, WeakReference<Node>> $refs = [
        "root" => $root->createWeakReference(),
    ];
    if ($refs->get("root") != null) { echo "found\n"; }
    if ($refs->get("missing") == null) { echo "missing\n"; }
}
"#;
    let program = doriac::lower_source_to_mir("stage25a-nullable-weak.doria", source)
        .expect("nullable weak references should lower across supported storage paths");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("nullable weak references should interpret without acquiring strong ownership");
    assert_eq!(
        output.stdout,
        b"empty\nroot\nstored\nfound\nmissing\ndrop root\n"
    );
    doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("nullable weak references should lower through Cranelift");
    #[cfg(feature = "llvm-backend")]
    doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("nullable weak references should lower through LLVM");
}

#[test]
fn shared_handles_flow_through_borrowed_calls_collections_and_generics() {
    let source = r#"
class Node
{
    function __construct(string $name) {}
    function __destruct() { try { echo "drop " . $this->name . "\n"; } catch (Doria\Std\Io\IoError) {} }
}

class Box<T>
{
    function __construct(take T $value) {}
}

function inspect(SharedReference<Node> $node): void throws Doria\Std\Io\IoError { echo $node->name . "\n"; }
function inspectNamed(int $marker, SharedReference<Node> $node): void throws Doria\Std\Io\IoError
{
    echo "{$marker}:{$node->name}\n";
}
function marker(): int throws Doria\Std\Io\IoError { echo "marker\n"; return 7; }
function consume(take SharedReference<Node> $node): void {}
function identity<T>(take T $value): T { return $value; }

function main(): void throws Doria\Std\Io\IoError
{
    let $root = shared new Node("Root");
    inspect($root->share());
    List<SharedReference<Node>> $values = [$root->share()];
    inspect($values[0]);
    inspectNamed(node: $values[0], marker: marker());
    let writable $moving = $values;
    consume($moving->removeAt(0));
    let $box = new Box<SharedReference<Node>>($root->share());
    let $returned = identity($root->share());
    inspect($returned);
}
"#;
    let program = doriac::lower_source_to_mir("stage25a-storage.doria", source)
        .expect("shared storage fixture should lower");
    let output =
        doriac::mir_interpreter::interpret(&program).expect("shared storage should interpret");
    assert_eq!(
        output.stdout,
        b"Root\nRoot\nmarker\n7:Root\nRoot\ndrop Root\n"
    );
    doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("shared storage should lower through Cranelift");
}

#[test]
fn shared_handles_flow_through_properties_arrays_and_dictionary_values() {
    let source = r#"
class Node
{
    function __construct(string $name) {}
    function __destruct() { try { echo "drop " . $this->name . "\n"; } catch (Doria\Std\Io\IoError) {} }
}

class Holder
{
    function __construct(
        take SharedReference<Node> $strong,
        take WeakReference<Node> $weak,
        take ?SharedReference<Node> $optional,
    ) {}
}

function inspect(SharedReference<Node> $node): void throws Doria\Std\Io\IoError { echo $node->name . "\n"; }

function main(): void throws Doria\Std\Io\IoError
{
    let $root = shared new Node("stored");
    let $holder = new Holder(
        $root->share(),
        $root->createWeakReference(),
        $root->share(),
    );
    inspect($holder->strong);
    let $fromWeak = $holder->weak->acquire();
    if ($fromWeak != null) { inspect($fromWeak); }
    if ($holder->optional != null) { echo "optional\n"; }

    SharedReference<Node>[] $array = [$root->share()];
    inspect($array[0]);

    Dictionary<string, SharedReference<Node>> $named = [
        "node" => $root->share(),
    ];
    if ($named->get("node") != null) { echo "found\n"; }
}
"#;
    let program = doriac::lower_source_to_mir("stage25a-properties.doria", source)
        .expect("shared property and collection storage fixture should lower");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("shared property and collection storage fixture should interpret");
    assert_eq!(
        output.stdout,
        b"stored\nstored\noptional\nstored\nfound\ndrop stored\n"
    );
    doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("shared property and collection storage should lower through Cranelift");
    #[cfg(feature = "llvm-backend")]
    doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("shared property and collection storage should lower through LLVM");
}

#[test]
fn shared_handles_do_not_gain_implicit_hash_identity() {
    let diagnostics = rejected(
        r#"
    Dictionary<SharedReference<Node>, int> $named = [];
    Set<WeakReference<Node>> $weak = [];
"#,
    );
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0523")
            .count(),
        2
    );
    assert!(diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "E0523")
        .all(|diagnostic| diagnostic.message.contains("Hashable")));
}

#[test]
fn mixed_boundary_reports_runtime_pending_instead_of_losing_handle_ownership() {
    let source = r#"
class Node {}

function main(): void throws Doria\Std\Io\IoError
{
    let $node = shared new Node();
    mixed $boxed = $node;
}
"#;
    let diagnostics = doriac::lower_source_to_mir("stage25a-mixed.doria", source)
        .expect_err("shared handles through mixed require an explicit runtime representation");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "M1102")
        .expect("the mixed boundary should keep its Stage 25a runtime-pending diagnostic");
    assert!(diagnostic.message.contains("Through mixed"));
}

// --- Payload domains -----------------------------------------------------

#[test]
fn generic_shared_handle_payloads_specialize_before_mir_lowering() {
    let source = r#"
class Node
{
    function __construct(string $name) {}
}

function identity<T>(
    take SharedReference<T> $value,
): SharedReference<T>
{
    return $value;
}

function main(): void throws Doria\Std\Io\IoError
{
    let $node = identity(shared new Node("generic"));
    echo $node->name . "\n";
}
"#;
    let program = doriac::lower_source_to_mir("stage25a-generic-handle.doria", source)
        .expect("generic shared-handle payload should specialize to Node");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("specialized generic shared handle should interpret");
    assert_eq!(output.stdout, b"generic\n");
    doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("specialized generic shared handle should lower through Cranelift");
    #[cfg(feature = "llvm-backend")]
    doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("specialized generic shared handle should lower through LLVM");
}

#[test]
fn generic_class_shared_handle_payloads_are_revalidated_after_specialization() {
    let source = r#"
class Box<T>
{
    ?SharedReference<T> $value = null;
}

function main(): void throws Doria\Std\Io\IoError
{
    let $box = new Box<int>();
}
"#;
    let error = doriac::lower_source_to_mir("stage25a-generic-handle-reject.doria", source)
        .expect_err("a concrete readonly shared-handle payload must be a class");
    assert!(
        error.iter().any(|diagnostic| diagnostic.code == "E0545"
            && diagnostic.message.contains("SharedReference<int>")),
        "{error:?}"
    );
}

#[test]
fn nullable_weak_temporaries_transfer_through_reordered_calls_and_cleanup_returns() {
    let source = r#"
class Node
{
    function __construct(string $name) {}
    function __destruct() { try { echo "drop " . $this->name . "\n"; } catch (Doria\Std\Io\IoError) {} }
}

class Cleanup
{
    function __destruct() { try { echo "cleanup\n"; } catch (Doria\Std\Io\IoError) {} }
}

function marker(): int throws Doria\Std\Io\IoError
{
    echo "marker\n";
    return 7;
}

function makeWeak(SharedReference<Node> $node): ?WeakReference<Node>
{
    let $cleanup = new Cleanup();
    return $node->createWeakReference();
}

function consume(int $marker, take ?WeakReference<Node> $weak): void throws Doria\Std\Io\IoError
{
    if ($weak != null) { echo "{$marker}:alive\n"; }
}

function main(): void throws Doria\Std\Io\IoError
{
    let $node = shared new Node("root");
    consume(weak: makeWeak($node), marker: marker());
}
"#;
    let program = doriac::lower_source_to_mir("stage25a-nullable-weak-transfer.doria", source)
        .expect("nullable weak temporaries should transfer through cleanup and reordered calls");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("nullable weak temporary ownership should remain balanced");
    assert_eq!(output.stdout, b"cleanup\nmarker\n7:alive\ndrop root\n");
    doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("nullable weak temporary ownership should lower through Cranelift");
    #[cfg(feature = "llvm-backend")]
    doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("nullable weak temporary ownership should lower through LLVM");
}

#[test]
fn readonly_family_rejects_concrete_non_class_payloads() {
    // Record 0106: the readonly family accepts class payloads only in v1.0.
    for declaration in [
        "SharedReference<int> $bad = shared new Node();",
        "SharedReference<string> $bad = shared new Node();",
        "SharedReference<List<int>> $bad = shared new Node();",
        "WeakReference<int> $bad = shared new Node();",
        "WeakReference<string> $bad = shared new Node();",
        "WeakReference<List<int>> $bad = shared new Node();",
    ] {
        assert_code(declaration, "E0545");
    }
}

#[test]
fn shared_new_requires_a_class_payload() {
    assert_code("let $bad = shared new List<int>();", "E0545");
    assert_code("let $bad = shared new Bytes();", "E0545");
}

#[test]
fn shared_new_of_a_collection_does_not_also_report_an_unknown_class() {
    let diagnostics = rejected("let $bad = shared new List<int>();");
    assert!(
        !codes(&diagnostics).contains(&"E0305"),
        "the payload-domain rule should not also report an unknown class: {:?}",
        codes(&diagnostics)
    );
}

#[test]
fn readonly_family_accepts_class_payloads() {
    accepted(
        r#"
    SharedReference<Node> $shared = shared new Node();
    WeakReference<Node> $weak = $shared->createWeakReference();
"#,
    );
}

#[test]
fn a_symbolic_payload_is_deferred_to_its_concrete_specialization() {
    // An unresolved type parameter may stand in a generic declaration; the
    // class-payload requirement is checked where a concrete type is written.
    let source = r#"
class Node { string $name = ""; }

function keep<T>(SharedReference<T> $value): void {}

function main(): void throws Doria\Std\Io\IoError
{
    let $node = shared new Node();
    keep($node);
}
"#;
    doriac::check_source("stage25a-symbolic-payload.doria", source)
        .expect("a symbolic payload must be accepted in a generic declaration");
}

#[test]
fn writable_family_accepts_collection_payloads() {
    // The writable family has access objects that forward member and indexed
    // operations, so it carries no class-payload restriction.
    accepted(
        r#"
    let $values = [1, 2, 3];
    let $sharedValues = new WritableSharedReference($values);
    WritableSharedReference<List<int>> $typed = new WritableSharedReference([4, 5]);
"#,
    );
}

#[test]
fn writable_access_forwards_indexed_operations_to_a_collection_payload() {
    accepted(
        r#"
    let $values = [1, 2, 3];
    let $sharedValues = new WritableSharedReference($values);
    let writable $access = $sharedValues->acquireWritableAccess();
    $access[0] = 10;
    echo $access[0];
"#,
    );
}

#[test]
fn readonly_access_forwards_indexed_reads_to_a_collection_payload() {
    accepted(
        r#"
    let $values = [1, 2, 3];
    let $sharedValues = new WritableSharedReference($values);
    let $access = $sharedValues->acquireReadonlyAccess();
    echo $access[0];
"#,
    );
}

#[test]
fn writable_weak_family_also_accepts_collection_payloads() {
    accepted(
        r#"
    let $sharedValues = new WritableSharedReference([1, 2, 3]);
    WritableWeakReference<List<int>> $weak = $sharedValues->createWeakReference();
    ?WritableSharedReference<List<int>> $live = $weak->acquire();
"#,
    );
}

#[test]
fn writable_access_objects_flow_through_returns_properties_and_collection_slots() {
    for (name, source, expected) in [
        (
            "access-lifetime",
            include_str!("../../../examples/native/main_stage25a_access_lifetime.doria"),
            b"access guarded\nlive through access\ndrop guarded\nexpired after access\n".as_slice(),
        ),
        (
            "stored-access",
            include_str!("../../../examples/native/main_stage25a_stored_access.doria"),
            b"stored read 1\nstored write 5\nstored list 1\ndrop item\ndrop item\n".as_slice(),
        ),
    ] {
        let program = doriac::lower_source_to_mir(format!("{name}.doria"), source)
            .expect("writable access storage fixture should lower");
        let output = doriac::mir_interpreter::interpret(&program)
            .expect("writable access storage fixture should interpret");
        assert_eq!(output.stdout, expected);
        doriac::codegen_cranelift::lower_mir_to_object(&program)
            .expect("writable access storage fixture should lower through Cranelift");
        #[cfg(feature = "llvm-backend")]
        doriac::codegen_llvm::lower_mir_to_object(&program)
            .expect("writable access storage fixture should lower through LLVM");
    }
}

#[test]
fn property_rooted_collection_slots_accept_owned_values_and_replace_once() {
    let source = r#"
class Customer
{
    function __construct(string $name) {}
    function __destruct() { try { echo "drop {$this->name}\n"; } catch (Doria\Std\Io\IoError) {} }
}

class Repository<T>
{
    internal writable Dictionary<string, T> $items = [];

    writable function save(string $id, take T $item): void
    {
        $this->items[$id] = $item;
    }
}

function main(): void throws Doria\Std\Io\IoError
{
    let writable $repository = new Repository<Customer>();
    $repository->save("42", new Customer("first"));
    $repository->save("42", new Customer("second"));
    echo "done\n";
}
"#;
    let program = doriac::lower_source_to_mir("stage25a-indexed-property-move.doria", source)
        .expect("an indexed slot write must not be treated as complete-property replacement");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("property-rooted collection replacement should execute");
    assert_eq!(output.stdout, b"drop first\ndone\ndrop second\n");
}

#[test]
fn property_rooted_collection_slots_consume_move_values() {
    let diagnostics = doriac::check_source(
        "stage25a-indexed-property-use-after-move.doria",
        r#"
class Customer { function __construct(string $name) {} }
class Repository
{
    internal writable Dictionary<string, Customer> $items = [];
    writable function save(string $id, take Customer $item): void throws Doria\Std\Io\IoError
    {
        $this->items[$id] = $item;
        echo $item->name;
    }
}
"#,
    )
    .expect_err("the value stored in an owning slot must be moved");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0470"),
        "{diagnostics:#?}"
    );
}

#[test]
fn indexed_slot_writes_require_a_writable_root_and_property_path() {
    for source in [
        r#"
class Box
{
    writable List<int> $items = [1];
}
function update(Box $box): void { $box->items[0] = 2; }
"#,
        r#"
class Box
{
    List<int> $items = [1];
    writable function update(): void { $this->items[0] = 2; }
}
"#,
    ] {
        let diagnostics = doriac::check_source("stage25a-indexed-readonly-path.doria", source)
            .expect_err("every segment of an indexed write path must be writable");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| matches!(diagnostic.code, "E0201" | "E0202" | "E0479")),
            "{diagnostics:#?}"
        );
    }
}

#[test]
fn standalone_blocks_are_lexical_scopes_and_cleanup_boundaries() {
    let source = r#"
class Marker
{
    function __construct(string $name) {}
    function __destruct() { try { echo "drop {$this->name}\n"; } catch (Doria\Std\Io\IoError) {} }
}

function returnFromBlock(): void throws Doria\Std\Io\IoError
{
    {
        let $marker = new Marker("return");
        return;
    }
}

function main(): void throws Doria\Std\Io\IoError
{
    {
        let $outer = new Marker("outer");
        {
            let $inner = new Marker("inner");
            echo "inside\n";
        }
        echo "after inner\n";
    }

    returnFromBlock();

    let writable $iteration = 0;
    while ($iteration < 2) {
        {
            let $loop = new Marker("loop {$iteration}");
            $iteration++;
            if ($iteration == 1) { continue; }
            break;
        }
    }
    echo "done\n";
}
"#;
    let program = doriac::lower_source_to_mir("stage25a-lexical-block-cleanup.doria", source)
        .expect("standalone blocks should lower through the ordinary scope machinery");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("standalone block cleanup should execute");
    assert_eq!(
        output.stdout,
        b"inside\ndrop inner\nafter inner\ndrop outer\ndrop return\ndrop loop 0\ndrop loop 1\ndone\n"
    );

    let diagnostics = doriac::check_source(
        "stage25a-lexical-block-scope.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    { let $hidden = 1; }
    echo "{$hidden}";
}
"#,
    )
    .expect_err("a binding declared in a standalone block must not escape it");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("undeclared variable")),
        "{diagnostics:#?}"
    );
}

#[test]
fn standalone_blocks_end_writable_shared_access_before_readonly_access() {
    let source = r#"
class Settings
{
    function __construct(writable string $theme) {}
}

function main(): void throws Doria\Std\Io\IoError
{
    let $settings = new WritableSharedReference(new Settings("light"));
    {
        let writable $access = $settings->acquireWritableAccess();
        $access->theme = "dark";
    }
    let $readonly = $settings->acquireReadonlyAccess();
    echo $readonly->theme;
}
"#;
    let program = doriac::lower_source_to_mir("stage25a-access-block.doria", source)
        .expect("a lexical block should delimit writable access");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("readonly access should succeed after block cleanup");
    assert_eq!(output.stdout, b"dark");
}

#[test]
fn nullable_access_parameters_borrow_unless_declared_take() {
    let source = r#"
class Counter
{
    function __construct(string $name, writable int $value = 0) {}
}

function inspect(
    ?ReadonlySharedReferenceAccess<Counter> $access,
): void throws Doria\Std\Io\IoError
{
    if ($access != null) {
        echo "inspect {$access->name}\n";
    }
}

function increment(
    ?WritableSharedReferenceAccess<Counter> $access,
): void
{
    if ($access != null) {
        $access->value++;
    }
}

function main(): void throws Doria\Std\Io\IoError
{
    let $readShared = new WritableSharedReference(new Counter("read"));
    ?ReadonlySharedReferenceAccess<Counter> $read =
        $readShared->acquireReadonlyAccess();
    inspect($read);
    if ($read != null) {
        echo "after {$read->name}\n";
    }

    let $writeShared = new WritableSharedReference(new Counter("write"));
    ?WritableSharedReferenceAccess<Counter> $write =
        $writeShared->acquireWritableAccess();
    increment($write);
    if ($write != null) {
        $write->value++;
        echo "value {$write->value}\n";
    }
}
"#;
    let program = doriac::lower_source_to_mir("stage25a-nullable-access-borrow.doria", source)
        .expect("nullable access parameters should preserve default borrowing");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("borrowed nullable access parameters should remain live after calls");
    assert_eq!(output.stdout, b"inspect read\nafter read\nvalue 2\n");
    doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("borrowed nullable access parameters should lower through Cranelift");
    #[cfg(feature = "llvm-backend")]
    doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("borrowed nullable access parameters should lower through LLVM");
}

#[test]
fn reordered_nullable_access_temporaries_transfer_with_balanced_cleanup() {
    let source = r#"
class Counter
{
    function __construct(string $name, writable int $value = 0) {}
}

function marker(string $name): int throws Doria\Std\Io\IoError
{
    echo "marker {$name}\n";
    return 7;
}

function consumeReadonly(
    int $marker,
    take ?ReadonlySharedReferenceAccess<Counter> $access,
): void throws Doria\Std\Io\IoError
{
    if ($access != null) {
        echo "{$marker}:{$access->name}\n";
    }
}

function consumeWritable(
    int $marker,
    take ?WritableSharedReferenceAccess<Counter> $access,
): void throws Doria\Std\Io\IoError
{
    if ($access != null) {
        echo "{$marker}:{$access->name}:{$access->value}\n";
    }
}

function exerciseReordering(
    ?WritableSharedReference<Counter> $source,
): void throws Doria\Std\Io\IoError
{
    consumeReadonly(
        access: $source?->acquireReadonlyAccess(),
        marker: marker("read"),
    );
    consumeWritable(
        access: $source?->acquireWritableAccess(),
        marker: marker("write"),
    );
}

function main(): void throws Doria\Std\Io\IoError
{
    let $owner = new WritableSharedReference(new Counter("owned"));
    exerciseReordering($owner);
}
"#;
    let program = doriac::lower_source_to_mir("stage25a-nullable-access-reorder.doria", source)
        .expect("reordered nullable access temporaries should lower without panicking");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("reordered nullable access temporaries should transfer exactly once");
    assert_eq!(
        output.stdout,
        b"marker read\n7:owned\nmarker write\n7:owned:0\n"
    );
    doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("reordered nullable access temporaries should lower through Cranelift");
    #[cfg(feature = "llvm-backend")]
    doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("reordered nullable access temporaries should lower through LLVM");
}

#[test]
fn nullable_ownership_paths_preserve_transfer_and_cleanup() {
    let source =
        include_str!("../../../examples/native/main_stage25a_nullable_ownership_paths.doria");
    let program = doriac::lower_source_to_mir("stage25a-nullable-ownership-paths.doria", source)
        .expect("nullable ownership paths should lower");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("nullable ownership paths should execute with balanced cleanup");
    assert_eq!(
        output.stdout,
        include_bytes!("fixtures/native_io/main_stage25a_nullable_ownership_paths/expected_stdout")
    );
    doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("nullable ownership paths should lower through Cranelift");
    #[cfg(feature = "llvm-backend")]
    doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("nullable ownership paths should lower through LLVM");
}

#[test]
fn nullable_collection_access_forwards_lazily() {
    let source =
        include_str!("../../../examples/native/main_stage25a_nullable_collection_access.doria");
    let program = doriac::lower_source_to_mir("stage25a-nullable-collection-access.doria", source)
        .expect("nullable collection accesses should lower through presence-guarded forwarding");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("nullable collection accesses should execute lazily");
    assert_eq!(
        output.stdout,
        include_bytes!(
            "fixtures/native_io/main_stage25a_nullable_collection_access/expected_stdout"
        )
    );
    doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("nullable collection accesses should lower through Cranelift");
    #[cfg(feature = "llvm-backend")]
    doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("nullable collection accesses should lower through LLVM");
}

#[test]
fn forwarded_access_temporaries_end_with_their_statement() {
    let source =
        include_str!("../../../examples/native/main_stage25a_temporary_access_cleanup.doria");
    let program = doriac::lower_source_to_mir("stage25a-temporary-access-cleanup.doria", source)
        .expect("forwarded access temporaries should lower with statement cleanup");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("the next statement should be able to acquire incompatible access");
    assert_eq!(
        output.stdout,
        include_bytes!("fixtures/native_io/main_stage25a_temporary_access_cleanup/expected_stdout")
    );
    doriac::codegen_cranelift::lower_mir_to_object(&program)
        .expect("statement access cleanup should lower through Cranelift");
    #[cfg(feature = "llvm-backend")]
    doriac::codegen_llvm::lower_mir_to_object(&program)
        .expect("statement access cleanup should lower through LLVM");
}

#[test]
fn forwarded_access_temporaries_live_through_the_complete_statement() {
    let source = r#"
function acquireWritable(WritableSharedReference<List<int>> $owner): bool
{
    let writable $access = $owner->acquireWritableAccess();
    return true;
}

function main(): void throws Doria\Std\Io\IoError
{
    let $owner = new WritableSharedReference<List<int>>([1]);
    if (
        $owner->acquireReadonlyAccess()->contains(1)
        && acquireWritable($owner)
    ) {
        echo "unreachable\n";
    }
}
"#;
    let program = doriac::lower_source_to_mir("stage25a-statement-access-lifetime.doria", source)
        .expect("forwarded access lifetime fixture should lower");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("incompatible access should use Doria's abort-only panic path");
    assert_eq!(output.exit_status, 101);
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Panic[P1501]"),
        "the readonly expression temporary must remain active through the entire condition"
    );
}

#[test]
fn nested_shadows_do_not_extend_outer_borrow_lifetimes() {
    doriac::check_source(
        "stage25a-shadowed-borrow-liveness.doria",
        r#"
class Guard
{
    writable int $value = 0;
    writable function mutate(): void { $this->value++; }
}

function identity(Guard $guard): Guard { return $guard; }

function route(writable Guard $guard): void throws Doria\Std\Io\IoError
{
    let $alias = identity($guard);
    {
        let $alias = new Guard();
        echo "{$alias->value}";
        $guard->mutate();
    }
}
"#,
    )
    .expect("a nested shadow must end liveness of an inaccessible outer borrow");

    doriac::check_source(
        "stage25a-shadowed-owner-identity.doria",
        r#"
class Guard
{
    writable int $value = 0;
    writable function mutate(): void { $this->value++; }
}

function identity(Guard $guard): Guard { return $guard; }
function consume(take Guard $guard): void {}

function route(writable Guard $guard): void throws Doria\Std\Io\IoError
{
    let $alias = identity($guard);
    {
        let writable $guard = new Guard();
        $guard->mutate();
        consume($guard);
        echo "{$alias->value}";
    }
}
"#,
    )
    .expect("borrow conflicts must follow binding identity through nested shadows");
}
