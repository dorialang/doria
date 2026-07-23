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
fn bytes_surface_checks_lowers_and_executes_through_shared_mir() {
    let mir = doriac::lower_source_to_mir(
        "stage23-bytes.doria",
        r#"
function main(): void
{
    writable uint8[] $source = [0, 128, 255];
    writable Bytes $bytes = Bytes::fromArray($source);
    $source[0] = 99;

    $bytes[1] = 42;
    $bytes[0]++;
    $bytes[1] += 1;
    $bytes[2]--;

    writable uint8[] $copy = $bytes->toArray();
    Bytes $same = Bytes::fromArray($copy);
    $copy[0] = 77;
    Bytes $different = Bytes::fromArray([1]);

    echo "{$bytes->length}:{$bytes[0]}:{$bytes[1]}:{$bytes[2]}:";
    if ($bytes == $same) {
        echo "equal:";
    }
    if ($bytes != $different) {
        echo "different";
    }
}
"#,
    )
    .expect("the complete Stage 23 Slice 2 Bytes surface should lower");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("the complete Stage 23 Slice 2 Bytes surface should execute");
    assert_eq!(output.stdout, b"3:1:43:254:equal:different");
}

#[test]
fn bytes_io_accepts_readonly_borrows_and_materializes_expression_temporaries() {
    doriac::lower_source_to_mir(
        "stage23-bytes-io.doria",
        r#"
function main(): void
{
    uint8[] $source = [0, 128, 255];
    Bytes $bytes = Bytes::fromArray($source);

    write_file_bytes("data.bin", $bytes);
    append_file_bytes("data.bin", Bytes::fromArray($source));
    write_stdout_bytes(read_file_bytes("data.bin"));
    write_stderr_bytes($bytes);
}
"#,
    )
    .expect("byte I/O should borrow locals and owned expression temporaries");
}

#[test]
fn bytes_rejects_implicit_conversion_readonly_writes_and_unauthored_methods() {
    let wrong_source = diagnostic(
        r#"
function main(): void
{
    int[] $values = [1];
    Bytes $bytes = Bytes::fromArray($values);
}
"#,
        "E0403",
    );
    assert!(wrong_source.message.contains("uint8[]"));

    let readonly_write = diagnostic(
        r#"
function main(): void
{
    uint8[] $values = [1];
    Bytes $bytes = Bytes::fromArray($values);
    $bytes[0] = 2;
}
"#,
        "E0201",
    );
    assert!(readonly_write.message.contains("readonly"));

    let deferred = diagnostic(
        r#"
function main(): void
{
    uint8[] $values = [1];
    writable Bytes $bytes = Bytes::fromArray($values);
    $bytes->append(2);
}
"#,
        "E0524",
    );
    assert!(deferred
        .message
        .contains("future Bytes method-surface record"));
}

#[test]
fn runtime_mixed_collection_values_remain_at_stage23_slice3() {
    let diagnostics = doriac::lower_source_to_mir(
        "stage23-mixed-collection.doria",
        r#"
function main(): void
{
    List<mixed> $values = [1];
}
"#,
    )
    .expect_err("runtime mixed collection elements require the Slice 3 box");
    let boundary = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "M1101")
        .unwrap_or_else(|| panic!("expected a native-stage diagnostic: {diagnostics:#?}"));
    assert!(boundary.message.contains("Stage 23 Slice 3"));
    assert!(!diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code.starts_with('P')));
}

#[test]
fn bytes_uses_move_ownership_and_readonly_borrow_parameters() {
    doriac::lower_source_to_mir(
        "stage23-bytes-borrow.doria",
        r#"
function inspect(Bytes $contents): int { return $contents->length; }
function consume(take Bytes $contents): int { return $contents->length; }
function main(): void
{
    Bytes $contents = Bytes::fromArray([1]);
    echo "{inspect($contents)}";
    echo "{consume($contents)}";
}
"#,
    )
    .expect("readonly Bytes parameters should borrow while take parameters move");

    let moved = diagnostics(
        r#"
function consume(take Bytes $contents): void {}
function main(): void
{
    Bytes $contents = Bytes::fromArray([1]);
    consume($contents);
    echo "{$contents->length}";
}
"#,
    );
    assert!(moved
        .iter()
        .any(|diagnostic| diagnostic.message.contains("given away")));
}

#[test]
fn builtin_bytes_results_and_byte_arrays_preserve_move_ownership() {
    for source in [
        r#"
function consume(take Bytes $contents): void {}
function main(): void
{
    let $contents = read_stdin_bytes();
    consume($contents);
    echo "{$contents->length}";
}
"#,
        r#"
function consume(take Bytes $contents): void {}
function main(): void
{
    let $contents = read_file_bytes("data.bin");
    consume($contents);
    echo "{$contents->length}";
}
"#,
        r#"
function consume(take uint8[] $contents): void {}
function main(): void
{
    Bytes $bytes = Bytes::fromArray([1]);
    let $contents = $bytes->toArray();
    consume($contents);
    echo "{$contents->length}";
}
"#,
    ] {
        assert!(diagnostics(source)
            .iter()
            .any(|diagnostic| diagnostic.message.contains("given away")));
    }
}

#[test]
fn writable_foreach_borrows_collection_elements_but_ranges_remain_readonly() {
    doriac::lower_source_to_mir(
        "stage23-writable-foreach.doria",
        r#"
class Counter
{
    function __construct(writable int $value) {}
    writable function increment(): void { $this->value++; }
}
function main(): void
{
    writable List<Counter> $counters = [new Counter(1)];
    foreach ($counters as writable Counter $counter) {
        $counter->increment();
    }
}
"#,
    )
    .expect("writable collection foreach bindings should preserve writable borrows");

    let range = diagnostic(
        r#"
function main(): void
{
    foreach (0..<2 as writable int $value) {
        $value++;
    }
}
"#,
        "E0425",
    );
    assert!(range.message.contains("readonly"));
}

#[test]
fn discarded_collection_removals_lower_and_drop_their_results() {
    doriac::lower_source_to_mir(
        "stage23-discarded-removals.doria",
        r#"
class Token { function __construct(int $id) {} }
function main(): void
{
    writable List<Token> $tokens = [new Token(1), new Token(2)];
    $tokens->removeAt(0);
    $tokens->pop();

    writable Dictionary<string, Token> $named = ["three" => new Token(3)];
    $named->remove("three");
}
"#,
    )
    .expect("discarded owned and nullable removal results should lower");
}

#[test]
fn intrinsic_collection_type_names_cannot_be_redeclared_as_classes() {
    for name in ["Bytes", "List", "Dictionary", "Set"] {
        let errors = diagnostics(&format!("class {name} {{}}"));
        assert!(
            errors.iter().any(|diagnostic| diagnostic.code == "E0309"),
            "{name} should remain reserved for its intrinsic type"
        );
    }
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
