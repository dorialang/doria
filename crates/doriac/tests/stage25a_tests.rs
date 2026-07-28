//! Stage 25a — shared-ownership grammar, type model, and readonly runtime family
//! (record 0106).

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

    function __destruct() { echo "drop " . $this->name . "\n"; }
    function describe(): string { return $this->name; }
}

function consume(take SharedReference<Node> $value): void {}

function main(): void
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
    function __destruct() { echo "drop " . $this->name . "\n"; }
}

function makeStrong(string $name): SharedReference<Node>
{
    return shared new Node($name);
}

function makeWeak(string $name): WeakReference<Node>
{
    return makeStrong($name)->createWeakReference();
}

function main(): void
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
fn borrowed_dictionary_shared_results_do_not_acquire_cleanup_obligations() {
    let source = r#"
class Node
{
    function __destruct() { echo "drop\n"; }
}

function main(): void
{
    let $root = shared new Node();
    Dictionary<string, SharedReference<Node>> $values = ["node" => $root->share()];
    let $borrowed = $values->get("node");
    if ($borrowed != null) { echo "found\n"; }
}
"#;
    let diagnostics = doriac::check_source("stage25a-borrowed-dictionary.doria", source)
        .expect_err("stored collection borrows remain rejected until lifetime tracking lands");
    assert!(diagnostics.iter().any(|diagnostic| {
        diagnostic.code == "E0478" && diagnostic.message.contains("borrowed result")
    }));
}

#[test]
fn weak_reference_survives_payload_destruction_without_resurrection() {
    let source = r#"
class Node
{
    function __construct(string $name) {}
    function __destruct() { echo "drop " . $this->name . "\n"; }
}

function makeWeak(): WeakReference<Node>
{
    let $strong = shared new Node("Expired");
    return $strong->createWeakReference();
}

function main(): void
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
fn shared_handles_flow_through_borrowed_calls_collections_and_generics() {
    let source = r#"
class Node
{
    function __construct(string $name) {}
    function __destruct() { echo "drop " . $this->name . "\n"; }
}

class Box<T>
{
    function __construct(take T $value) {}
}

function inspect(SharedReference<Node> $node): void { echo $node->name . "\n"; }
function inspectNamed(int $marker, SharedReference<Node> $node): void
{
    echo "{$marker}:{$node->name}\n";
}
function marker(): int { echo "marker\n"; return 7; }
function consume(take SharedReference<Node> $node): void {}
function identity<T>(take T $value): T { return $value; }

function main(): void
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
    function __destruct() { echo "drop " . $this->name . "\n"; }
}

class Holder
{
    function __construct(
        take SharedReference<Node> $strong,
        take WeakReference<Node> $weak,
        take ?SharedReference<Node> $optional,
    ) {}
}

function inspect(SharedReference<Node> $node): void { echo $node->name . "\n"; }

function main(): void
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

function main(): void
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

function main(): void
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
