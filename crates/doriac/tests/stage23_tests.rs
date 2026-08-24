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
function main(): void throws Doria\Std\Io\IoError
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
function main(): void throws Doria\Std\Io\IoError
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
fn collection_properties_and_indexed_class_values_preserve_writable_borrows() {
    let mir = doriac::lower_source_to_mir(
        "stage23-collection-places.doria",
        r#"
class Counter
{
    function __construct(writable int $value) {}
    writable function increment(): void { $this->value++; }
    function current(): int { return $this->value; }
}
class Holder
{
    writable List<int> $items = [1];
    writable function append(int $value): void { $this->items->add($value); }
    writable function reset(): void { $this->items->clear(); }
}
function main(): void throws Doria\Std\Io\IoError
{
    writable List<Counter> $counters = [new Counter(1)];
    $counters[0]->increment();

    let writable $holder = new Holder();
    $holder->append(2);
    $holder->reset();
    $holder->append(3);
    echo "{$counters[0]->current()}:{$holder->items->count}:{$holder->items[0]}";
}
"#,
    )
    .expect("indexed class and collection property places should lower as writable borrows");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("borrowed collection places should execute through shared MIR");
    assert_eq!(output.stdout, b"2:1:3");
}

#[test]
fn native_collection_property_initializers_cover_concrete_storage_types() {
    let source =
        include_str!("../../../examples/native/main_native_collection_property_initializers.doria");
    let mir = doriac::lower_source_to_mir("collection-property-initializers.doria", source).expect(
        "concrete collection property types should be interned before initializer lowering",
    );
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("collection property initializers should execute through shared MIR");
    assert_eq!(
        output.stdout,
        include_bytes!(
            "fixtures/native_io/main_native_collection_property_initializers/expected_stdout"
        )
    );
}

#[test]
fn unsupported_collection_property_capabilities_precede_native_lowering() {
    let errors = diagnostics(
        r#"
class Scene {}
class Unsupported
{
    Set<Scene> $set = Set::from([]);
    SortedSet<Scene> $sorted = SortedSet::from([]);
}
function main(): void
{
    let $value = new Unsupported();
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
    assert!(!errors.iter().any(|diagnostic| diagnostic.code == "N1101"));
}

#[test]
fn foreach_materializes_collection_expression_with_scoped_ownership() {
    let mir = doriac::lower_source_to_mir(
        "stage23-foreach-expression.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
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
function main(): void throws Doria\Std\Io\IoError
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
fn nested_collection_ingestion_tracks_moved_arguments() {
    let errors = diagnostics(
        r#"
class Token { function __construct(int $id) {} }
function main(): void throws Doria\Std\Io\IoError
{
    writable List<List<List<Token>>> $outer = [[[]]];
    List<Token> $inner = [new Token(1)];
    $outer[0]->add($inner);
    echo "{$inner->count}";
}
"#,
    );
    assert!(
        errors
            .iter()
            .any(|diagnostic| diagnostic.message.contains("given away")),
        "{errors:#?}"
    );
}

#[test]
fn static_collection_constructors_observe_moved_sources() {
    for source in [
        r#"
function main(): void
{
    uint8[] $source = [1];
    let $moved = $source;
    Bytes $bytes = Bytes::fromArray($source);
}
"#,
        r#"
function main(): void
{
    List<int> $source = [1];
    let $moved = $source;
    Set<int> $set = Set::from($source);
}
"#,
    ] {
        assert!(diagnostics(source)
            .iter()
            .any(|diagnostic| diagnostic.message.contains("given away")));
    }
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
function main(): void throws Doria\Std\Io\IoError
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
fn non_bytes_collection_equality_is_rejected_before_mir() {
    for declaration in [
        "List<int> $left = [1]; List<int> $right = [1];",
        "Dictionary<string, int> $left = [\"one\" => 1]; Dictionary<string, int> $right = [\"one\" => 1];",
        "Set<int> $left = Set::from([1]); Set<int> $right = Set::from([1]);",
        "int[] $left = [1]; int[] $right = [1];",
    ] {
        let source = format!(
            "function main(): void throws Doria\\Std\\Io\\IoError {{ {declaration} if ($left == $right) {{ echo \"same\"; }} }}"
        );
        let error = diagnostic(&source, "E0525");
        assert!(error.message.contains("only `Bytes`"));
    }
}

#[test]
fn dictionary_literals_require_explicit_keys_for_every_entry() {
    let error = diagnostic(
        r#"
function main(): void
{
    Dictionary<int, string> $values = [0 => "zero", "one"];
}
"#,
        "E0403",
    );
    assert!(error.message.contains("cannot assign"));
}

#[test]
fn bytes_io_accepts_readonly_borrows_and_materializes_expression_temporaries() {
    doriac::lower_source_to_mir(
        "stage23-bytes-io.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
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
fn runtime_mixed_collection_values_lower_to_stage23_slice3_boxes() {
    let program = doriac::lower_source_to_mir(
        "stage23-mixed-collection.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    List<mixed> $values = [1];
    foreach ($values as mixed $value) {
        if ($value is int) {
            echo "{$value}";
        }
    }
}
"#,
    )
    .expect("runtime mixed collection elements should lower after Slice 3");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("runtime mixed collection elements should execute");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "1");
}

#[test]
fn clear_releases_old_owned_values_once_and_scope_drop_releases_only_refill() {
    let program = doriac::lower_source_to_mir(
        "collection-clear-ownership.doria",
        r#"
class Token
{
    function __construct(int $id) {}
    function __destruct()
    {
        try { echo "drop {$this->id}\n"; } catch (Doria\Std\Io\IoError) {}
    }
}
function main(): void
{
    writable List<Token> $tokens = [new Token(1), new Token(2)];
    $tokens->clear();
    $tokens->clear();
    $tokens->add(new Token(3));

    writable Dictionary<string, Token> $named = ["old" => new Token(4)];
    $named->clear();
    $named->set("new", new Token(5));

    writable List<List<string>> $nested = [["old-a", "old-b"]];
    $nested->clear();
    $nested->add(["new"]);
}
"#,
    )
    .expect("owned collection clear should lower");
    let output = doriac::mir_interpreter::interpret(&program)
        .expect("owned collection clear should execute");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "drop 2\ndrop 1\ndrop 4\ndrop 5\ndrop 3\n"
    );
}

#[test]
fn bytes_uses_move_ownership_and_readonly_borrow_parameters() {
    doriac::lower_source_to_mir(
        "stage23-bytes-borrow.doria",
        r#"
function inspect(Bytes $contents): int { return $contents->length; }
function consume(take Bytes $contents): int { return $contents->length; }
function main(): void throws Doria\Std\Io\IoError
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
function main(): void throws Doria\Std\Io\IoError
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
function main(): void throws Doria\Std\Io\IoError
{
    let $contents = read_stdin_bytes();
    consume($contents);
    echo "{$contents->length}";
}
"#,
        r#"
function consume(take Bytes $contents): void {}
function main(): void throws Doria\Std\Io\IoError
{
    let $contents = read_file_bytes("data.bin");
    consume($contents);
    echo "{$contents->length}";
}
"#,
        r#"
function consume(take uint8[] $contents): void {}
function main(): void throws Doria\Std\Io\IoError
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
fn clear_conflicts_with_live_collection_borrows_but_accepts_last_use() {
    for mutation in ["$values->clear();", "$values->add(new Token(2));"] {
        let source = format!(
            r#"
class Token {{ function __construct(int $id) {{}} }}
function main(): void throws Doria\Std\Io\IoError
{{
    writable List<Token> $values = [new Token(1)];
    foreach ($values as Token $value) {{
        {mutation}
        echo "{{$value->id}}";
    }}
}}
"#
        );
        assert!(diagnostics(&source).iter().any(|diagnostic| {
            diagnostic.code == "E0477" && diagnostic.message.contains("earlier live access")
        }));
    }

    let live_list = diagnostics(
        r#"
class Token { function __construct(int $id) {} }
function main(): void throws Doria\Std\Io\IoError
{
    writable List<Token> $values = [new Token(1)];
    let $first = $values->first;
    $values->clear();
    if ($first != null) { echo "{$first->id}"; }
}
"#,
    );
    assert!(live_list.iter().any(|diagnostic| {
        diagnostic.code == "E0477" && diagnostic.message.contains("writable")
    }));

    let live_dictionary = diagnostics(
        r#"
class Token { function __construct(int $id) {} }
function main(): void throws Doria\Std\Io\IoError
{
    writable Dictionary<string, Token> $values = ["one" => new Token(1)];
    let $found = $values->get("one");
    $values->clear();
    if ($found != null) { echo "{$found->id}"; }
}
"#,
    );
    assert!(live_dictionary
        .iter()
        .any(|diagnostic| diagnostic.code == "E0477"));

    doriac::lower_source_to_mir(
        "clear-after-last-use.doria",
        r#"
class Token { function __construct(int $id) {} }
function main(): void throws Doria\Std\Io\IoError
{
    writable List<Token> $values = [new Token(1)];
    let $first = $values->first;
    if ($first != null) { echo "{$first->id}"; }
    $values->clear();
}
"#,
    )
    .expect("a collection borrow that has reached its last use must not block clear");
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
fn deferred_collection_family_members_keep_named_gates() {
    let closures = diagnostic(
        r#"
function main(): void
{
    Set<int> $values = Set::from([1]);
    $values->map(unknown);
}
"#,
        "E0521",
    );
    assert!(closures.message.contains("Decision 0113"));

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
    // 0113 amends 0100 and is now the record the surface gate names.
    assert!(family.message.contains("Decision 0113"));
}

#[test]
fn mixed_collection_index_bindings_retain_without_removing_elements() {
    let mir = doriac::lower_source_to_mir(
        "stage23-mixed-index-ownership.doria",
        r#"
class Token { function __construct(string $name) {} }
function main(): void
{
    List<mixed> $items = [new Token("list")];
    mixed $item = $items[0];
    Dictionary<string, mixed> $named = ["token" => new Token("dictionary")];
    mixed $token = $named["token"];
}
"#,
    )
    .expect("mixed index bindings should receive retained ownership claims");

    let transfers = mir
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.statements)
        .filter(|statement| {
            matches!(
                statement,
                doriac::mir::Statement::AssignLocal {
                    value: doriac::mir::Rvalue::Mixed(
                        doriac::mir::MixedExpression::CollectionIndex { transfer: true, .. }
                    ),
                    ..
                }
            )
        })
        .count();
    assert_eq!(transfers, 2);
}

#[test]
fn non_narrowed_nullable_mixed_cannot_flow_into_non_null_mixed() {
    let errors = doriac::lower_source_to_mir(
        "stage23-nullable-mixed-sink.doria",
        r#"
function consume(mixed $value): void {}
function main(): void
{
    ?mixed $value = null;
    consume($value);
}
"#,
    )
    .expect_err("nullable mixed must be narrowed before entering a mixed sink");
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.message.contains("nullable")
            || diagnostic
                .message
                .contains("mixed expression could not be lowered")
    }));
}

#[test]
fn bare_mixed_cannot_be_compared_to_null() {
    let errors = doriac::lower_source_to_mir(
        "stage23-bare-mixed-null.doria",
        r#"
function inspect(mixed $value): void
{
    if ($value == null) {
        echo "null\n";
    }
}
function main(): void
{
    inspect(1);
}
"#,
    )
    .expect_err("a non-null mixed cannot be compared to null without narrowing");
    assert!(errors
        .iter()
        .any(|diagnostic| diagnostic.message.contains("before narrowing")));
}

#[test]
fn int_and_float_parse_type_and_lower_and_fixed_width_is_deferred() {
    doriac::lower_source_to_mir(
        "stage23-parse.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    let $n = Int::parse("42");
    if ($n != null) {
        echo "{$n}\n";
    }
    let $f = Float::parse("3.5");
    if ($f != null) {
        echo "{$f}\n";
    }
}
"#,
    )
    .expect("Int::parse and Float::parse must type as nullable and lower");

    let errors = doriac::lower_source_to_mir(
        "stage23-int8-parse.doria",
        r#"
function main(): void
{
    let $n = Int8::parse("1");
}
"#,
    )
    .expect_err("fixed-width parse is not available yet");
    assert!(errors
        .iter()
        .any(|diagnostic| diagnostic.message.contains("Int8::parse")
            && diagnostic.message.contains("not available")));
}

#[test]
fn bool_collection_element_read_lowers_without_malformed_mir() {
    // Regression: reading a `bool` element out of a collection/array used to fail
    // shared MIR validation ("bool expression has an incompatible operand") because
    // the bool operand surface omitted `Operand::CollectionIndex`. The defect was
    // in validation, not lowering, so the validator has to run for this to guard
    // anything.
    let program = doriac::lower_source_to_mir(
        "stage23-bool-collection.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    writable List<bool> $flags = [];
    $flags->add(true);
    if ($flags[0]) {
        echo "ok\n";
    }
    bool[] $mask = [true, false];
    if (!$mask[1]) {
        echo "ok\n";
    }
    writable Dictionary<int, bool> $seen = [1 => true];
    if ($seen[1]) {
        echo "ok\n";
    }
}
"#,
    )
    .expect("bool collection/array element reads must lower");

    doriac::mir_validation::validate_program(&program)
        .expect("bool collection/array element reads must pass shared MIR validation");
}

#[test]
fn mixed_remove_at_lowers_to_a_removing_collection_index() {
    let program = doriac::lower_source_to_mir(
        "stage23-mixed-removeat.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    writable List<mixed> $items = [1, 2, 3];
    mixed $first = $items->removeAt(0);
    if ($first is int) {
        echo "{$first}\n";
    }
}
"#,
    )
    .expect("mixed removeAt should lower");
    let removals = program
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.statements)
        .filter(|statement| {
            matches!(
                statement,
                doriac::mir::Statement::AssignLocal {
                    value: doriac::mir::Rvalue::Mixed(
                        doriac::mir::MixedExpression::CollectionIndex { remove: true, .. }
                    ),
                    ..
                }
            )
        })
        .count();
    assert_eq!(removals, 1);
}
