//! Stage 25a Slice 1 — shared-ownership grammar and type model (record 0106).
//!
//! Slice 1 lands the surface and the type model only. The reference-counted
//! runtime arrives in the following slices, so these tests assert the static
//! model and the stage-named runtime diagnostic, never runtime behavior.

use doriac::diagnostics::Diagnostic;

const NODE: &str = r#"
class Node
{
    string $name = "";
    writable int $count = 0;

    function describe(): string { return $this->name; }
    function rename(): void {}
}
"#;

fn check(body: &str) -> Result<(), Vec<Diagnostic>> {
    let source = format!("{NODE}\nfunction main(): void\n{{\n{body}\n}}\n");
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

// --- Grammar -------------------------------------------------------------

#[test]
fn shared_new_parses_without_errors() {
    doriac::parse_source(
        "stage25a-syntax.doria",
        "function main(): void { let $node = shared new Node(); }",
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
    let source = "function main(): void { let $bad = weak new Node(); }";
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
    let source = "class SharedReference {}\nfunction main(): void {}";
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
    assert_code("let $bad = new WritableSharedReference();", "E0544");
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

function main(): void
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

function main(): void
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

// --- Two clocks ----------------------------------------------------------

#[test]
fn native_lowering_reports_the_stage_named_runtime_diagnostic() {
    let source = format!("{NODE}\nfunction main(): void {{ let $node = shared new Node(); }}\n");
    let diagnostics = doriac::lower_source_to_mir("stage25a-runtime.doria", &source)
        .expect_err("Stage 25a runtime support is not implemented in Slice 1");
    let message = diagnostics
        .iter()
        .map(|entry| entry.message.clone())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        message.contains("Stage 25a Shared-Ownership Runtime Support Is Not Yet Implemented"),
        "expected the stage-named runtime diagnostic, got: {message}"
    );
    assert!(
        !message.contains("SharedHandle("),
        "the diagnostic must not leak an internal type representation: {message}"
    );
}
