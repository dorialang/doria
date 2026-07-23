fn diagnostics(source: &str) -> Vec<doriac::diagnostics::Diagnostic> {
    doriac::check_source("stage23.doria", source).expect_err("source should be rejected")
}

fn diagnostic(source: &str, code: &str) -> doriac::diagnostics::Diagnostic {
    diagnostics(source)
        .into_iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("expected {code}"))
}

#[test]
fn collection_and_typed_array_surface_checks_and_lowers_to_shared_mir() {
    for (path, source) in [
        (
            "main_stage23_collections.doria",
            include_str!("../../../examples/native/main_stage23_collections.doria"),
        ),
        (
            "main_stage23_collection_ownership.doria",
            include_str!("../../../examples/native/main_stage23_collection_ownership.doria"),
        ),
    ] {
        let mir = doriac::lower_source_to_mir(path, source)
            .expect("Stage 23 source should check and lower through shared MIR");
        doriac::mir_interpreter::interpret(&mir)
            .expect("Stage 23 source should run through the shared MIR interpreter");
    }
}

#[test]
fn empty_set_construction_uses_context_and_existing_sources_are_borrowed() {
    doriac::lower_source_to_mir(
        "stage23-set-construction.doria",
        r#"
function main(): void
{
    Set<int> $empty = Set::from([]);
    List<int> $source = [1, 2];
    Set<int> $values = Set::from($source);
    echo "{$source->count} {$empty->count} {$values->count}";
}
"#,
    )
    .expect("Set::from should context-type an empty source and borrow an existing source");
}

#[test]
fn nested_typed_array_indexing_materializes_borrowed_places() {
    doriac::lower_source_to_mir(
        "stage23-nested-index.doria",
        r#"
function main(): void
{
    writable int[][] $matrix = [[1, 2], [3, 4]];
    $matrix[1][0]++;
    echo "{$matrix[1][0]}";
}
"#,
    )
    .expect("nested typed-array reads and writes should lower through borrowed MIR places");
}

#[test]
fn foreach_materializes_collection_expression_with_scoped_ownership() {
    let mir = doriac::lower_source_to_mir(
        "stage23-foreach-expression.doria",
        r#"
function main(): void
{
    foreach ([1, 2] as int $value) {
        echo "{$value}";
    }
}
"#,
    )
    .expect("a collection expression should be a valid foreach iterable");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("materialized foreach collection should execute");
    assert_eq!(output.stdout, b"12");
}

#[test]
fn collection_ingestion_moves_class_values() {
    let error = diagnostic(
        r#"
class Token { function __construct(int $id) {} }
function main(): void
{
    writable List<Token> $tokens = [];
    let $token = new Token(1);
    $tokens->add($token);
    echo "{$token->id}";
}
"#,
        "E0470",
    );
    assert!(error.message.contains("given away"));
}

#[test]
fn borrowed_collection_results_cannot_become_owners_but_removals_can() {
    let error = diagnostic(
        r#"
class Token { function __construct(int $id) {} }
function main(): void
{
    writable Dictionary<string, Token> $tokens = [];
    let $token = new Token(1);
    $tokens->set("one", $token);
    ?Token $borrowed = $tokens->get("one");
}
"#,
        "E0478",
    );
    assert!(error.message.contains("borrowed result"));

    doriac::lower_source_to_mir(
        "stage23-owned-removal.doria",
        r#"
class Token { function __construct(int $id) {} }
function main(): void
{
    writable Dictionary<string, Token> $tokens = [];
    let $token = new Token(1);
    $tokens->set("one", $token);
    ?Token $owned = $tokens->remove("one");
}
"#,
    )
    .expect("Dictionary::remove should hand ownership back");
}

#[test]
fn dictionary_projections_are_foreach_only_readonly_borrows() {
    let stored = diagnostic(
        r#"
function main(): void
{
    Dictionary<string, int> $values = ["one" => 1];
    let $keys = $values->keys;
}
"#,
        "E0522",
    );
    assert!(stored.message.contains("foreach-only"));

    let writable = diagnostic(
        r#"
function main(): void
{
    Dictionary<string, int> $values = ["one" => 1];
    foreach ($values->values as writable int $value) {}
}
"#,
        "E0522",
    );
    assert!(writable.message.contains("readonly"));
}

#[test]
fn hash_collections_reject_non_hashable_float_types() {
    let errors = diagnostics(
        r#"
function main(): void
{
    Dictionary<float, int> $dictionary = [];
    Set<float> $set = Set::from([1.0]);
}
"#,
    );
    assert_eq!(
        errors
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0523")
            .count(),
        2
    );
    assert!(errors
        .iter()
        .filter(|diagnostic| diagnostic.code == "E0523")
        .all(|diagnostic| diagnostic.message.contains("Hashable")));
}

#[test]
fn later_collection_family_and_closure_members_keep_named_gates() {
    let closures = diagnostic(
        r#"
function main(): void
{
    List<int> $values = [1];
    $values->map(unknown);
}
"#,
        "E0521",
    );
    assert!(closures.message.contains("Stage 30"));

    let family = diagnostic(
        r#"
function main(): void
{
    List<int> $values = [1];
    $values->sort();
}
"#,
        "E0521",
    );
    assert!(family.message.contains("Decision 0100"));
}
