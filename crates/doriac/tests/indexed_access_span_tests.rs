#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;

#[path = "common/php_execution.rs"]
mod php_execution;

use doriac::backend::NativeProfile;
use doriac::diagnostics::RuntimeFactValue;

const PATH: &str = "indexed-access-span.doria";
const SUPPORT: &str = r#"
class Marker {
    function __destruct() {
        try { echo "unexpected cleanup\n"; } catch (Doria\Std\Io\IoError) {}
    }
}
interface View { function read(): int; }
class Item implements View {
    function __construct(int $value) {}
    function read(): int { return $this->value; }
    function __destruct() {
        try { echo "unexpected item cleanup\n"; } catch (Doria\Std\Io\IoError) {}
    }
}
enum State { case Ready; }
enum Packet { case Value(int $value); }
function index(int $value): int { echo "index\n"; return $value; }
function key(): string { echo "key\n"; return "missing"; }
function replacement(): int { echo "rhs\n"; return 9; }
function byteReplacement(): uint8 { echo "rhs\n"; return 9; }
function itemReplacement(): Item { echo "rhs\n"; return new Item(9); }
function nullableReplacement(): ?int { echo "rhs\n"; return null; }
function payloadReplacement(): Packet { echo "rhs\n"; return Packet::Value(9); }
function callbackReplacement(): function(): int { echo "rhs\n"; return fn() => 9; }
"#;

fn source(setup: &str, statement: &str) -> String {
    format!(
        "{SUPPORT}\nfunction main(): void {{\n    let $marker = new Marker();\n    {setup}\n    echo \"before\\n\";\n    {statement}\n    echo \"unexpected continuation\\n\";\n}}\n"
    )
}

struct ExpectedPanic<'a> {
    selected: &'a str,
    code: &'static str,
    bounds: Option<(i64, u64)>,
    stdout: &'a str,
    php: bool,
}

fn assert_panic(source: &str, expected: ExpectedPanic<'_>) {
    let mir = doriac::lower_source_to_mir(PATH, source)
        .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
    let output = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(output.stdout, expected.stdout.as_bytes(), "{source}");
    assert_eq!(output.exit_status, 101, "{source}");
    let diagnostic = output
        .runtime_diagnostic
        .as_ref()
        .expect("structured panic");
    let start = source
        .rfind(expected.selected)
        .expect("selected source exists");
    assert_eq!(diagnostic.code, expected.code, "{source}\n{diagnostic:#?}");
    assert_eq!(diagnostic.span.start, start, "{source}\n{diagnostic:#?}");
    assert_eq!(
        diagnostic.span.end,
        start + expected.selected.len(),
        "{source}\n{diagnostic:#?}"
    );
    let outcome = diagnostic.runtime_outcome.as_ref().unwrap();
    assert_eq!(mir.source_for_span(diagnostic.span).unwrap().path, PATH);
    assert_eq!(outcome.process_status, 101);
    assert_eq!(
        outcome.termination_behavior,
        doriac::diagnostics::TerminationBehavior::AbortWithoutCleanup
    );
    assert_eq!(outcome.origin.span, diagnostic.span);
    assert_eq!(outcome.origin.function.as_deref(), Some("main"));
    assert_eq!(outcome.path.len(), 1);
    assert_eq!(outcome.path[0].function, "main");
    assert_eq!(outcome.path[0].span, diagnostic.span);
    match expected.bounds {
        Some((index, length)) => {
            assert_eq!(outcome.facts.len(), 2);
            assert_eq!(
                outcome.facts[0].name,
                doria_diagnostic_catalogue::INDEX_FACT
            );
            assert_eq!(outcome.facts[0].value, RuntimeFactValue::Signed(index));
            assert_eq!(
                outcome.facts[1].name,
                doria_diagnostic_catalogue::INDEXED_LENGTH_FACT
            );
            assert_eq!(outcome.facts[1].value, RuntimeFactValue::Unsigned(length));
        }
        None => assert!(outcome.facts.is_empty()),
    }
    let native = native_execution::execute_native(&mir, NativeProfile::Fast);
    assert_eq!(native.status.code(), Some(101), "{native:?}");
    assert_eq!(native.stdout, output.stdout);
    assert_eq!(native.stderr, output.stderr);
    #[cfg(feature = "llvm-backend")]
    {
        let native = native_execution::execute_native(&mir, NativeProfile::Release);
        assert_eq!(native.status.code(), Some(101), "{native:?}");
        assert_eq!(native.stdout, output.stdout);
        assert_eq!(native.stderr, output.stderr);
    }
    if expected.php {
        assert_php_panic(source, expected, &output.stderr);
    }
}

fn assert_php_panic(source: &str, expected: ExpectedPanic<'_>, canonical_stderr: &[u8]) {
    let php = doriac::compile_source_to_php(PATH, source)
        .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
    let Ok(version) = std::process::Command::new("php").arg("--version").output() else {
        eprintln!("PHP unavailable; indexed-access compatibility execution skipped");
        return;
    };
    assert!(version.status.success());
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").unwrap()
    );
    let output = php_execution::Script::new(&script)
        .command(&[])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(101), "{output:?}");
    assert_eq!(output.stdout, expected.stdout.as_bytes());
    // Structured spans, outcomes, frames and facts are checked above. All
    // backends then compare the complete compiler-owned presentation exactly.
    assert_eq!(output.stderr, canonical_stderr);
}

#[test]
fn sequence_reads_restore_the_access_span_after_nested_index_evaluation() {
    for (setup, code) in [
        ("Bytes $values = Bytes::allocate(1);", "P1301"),
        ("int[] $values = [7];", "P1310"),
        ("List<int> $values = [7];", "P1310"),
    ] {
        assert_panic(
            &source(setup, "let $value = $values[index(1)];"),
            ExpectedPanic {
                selected: "$values[index(1)]",
                code,
                bounds: Some((1, 1)),
                stdout: "before\nindex\n",
                php: true,
            },
        );
    }
}

#[test]
fn sequence_writes_restore_the_access_span_after_index_and_replacement_evaluation() {
    for (setup, statement, code) in [
        (
            "writable Bytes $values = Bytes::allocate(1);",
            "$values[index(1)] = byteReplacement();",
            "P1301",
        ),
        (
            "writable int[] $values = [7];",
            "$values[index(1)] = replacement();",
            "P1310",
        ),
        (
            "writable List<int> $values = [7];",
            "$values[index(1)] = replacement();",
            "P1310",
        ),
    ] {
        assert_panic(
            &source(setup, statement),
            ExpectedPanic {
                selected: "$values[index(1)]",
                code,
                bounds: Some((1, 1)),
                stdout: "before\nindex\nrhs\n",
                php: true,
            },
        );
    }
}

#[test]
fn compound_indexed_reads_fail_before_evaluating_the_replacement() {
    // Bytes and byte regions support PHP RMW; the general collection boundary remains separate.
    for (setup, statement, code, php) in [
        (
            "writable Bytes $values = Bytes::allocate(1);",
            "$values[index(1)] += byteReplacement();",
            "P1301",
            true,
        ),
        (
            "writable Bytes $owner = Bytes::allocate(1); let writable $values = $owner->getWritableRegion(0, 1);",
            "$values[index(1)] += byteReplacement();",
            "P1301",
            true,
        ),
        (
            "writable int[] $values = [7];",
            "$values[index(1)] += replacement();",
            "P1310",
            false,
        ),
        (
            "writable List<int> $values = [7];",
            "$values[index(1)]++;",
            "P1310",
            false,
        ),
    ] {
        assert_panic(
            &source(setup, statement),
            ExpectedPanic {
                selected: "$values[index(1)]",
                code,
                bounds: Some((1, 1)),
                stdout: "before\nindex\n",
                php,
            },
        );
    }
}

#[test]
fn successful_indexed_updates_evaluate_index_and_rhs_once_in_order() {
    for (collection, element, initial, replacement, result) in [
        ("int[]", "int", "3", "9", "12"),
        ("List<int>", "int", "3", "9", "12"),
        ("float[]", "float", "3.5", "9.0", "12.5"),
        ("List<float>", "float", "3.5", "9.0", "12.5"),
    ] {
        let source = format!(
            r#"
function index(): int {{ echo "index\n"; return 0; }}
function replacement(): {element} {{ echo "rhs\n"; return {replacement}; }}
function main(): void {{
    writable {collection} $values = [{initial}];
    $values[index()] += replacement();
    echo "result {{$values[0]}}\n";
}}
"#
        );
        assert_success(&source, &format!("index\nrhs\nresult {result}\n"), false);
    }
}

#[test]
fn byte_indexed_updates_share_exact_evaluation_order_across_backends() {
    for setup in [
        "writable Bytes $values = Bytes::allocate(1);",
        "writable Bytes $owner = Bytes::allocate(1); let writable $values = $owner->getWritableRegion(0, 1);",
    ] {
        let source = format!(
            r#"
function index(): int {{ echo "index\n"; return 0; }}
function replacement(): uint8 {{ echo "rhs\n"; return 9; }}
function main(): void {{
    {setup}
    $values[0] = 3;
    $values[index()] += replacement();
    echo "sum {{$values[0]}}\n";
    $values[index()]++;
    echo "increment {{$values[0]}}\n";
}}
"#
        );
        assert_success(&source, "index\nrhs\nsum 12\nindex\nincrement 13\n", true);
    }
}

#[test]
fn dictionary_reads_restore_the_access_span_after_nested_key_evaluation() {
    for setup in [
        "Dictionary<string, int> $values = [\"present\" => 7];",
        "Dictionary<string, ?int> $values = [\"present\" => null];",
        "SortedDictionary<string, int> $values = SortedDictionary::from([\"present\" => 7]);",
    ] {
        assert_panic(
            &source(setup, "let $value = $values[key()];"),
            ExpectedPanic {
                selected: "$values[key()]",
                code: "P1312",
                bounds: None,
                stdout: "before\nkey\n",
                php: true,
            },
        );
    }
}

#[test]
fn non_scalar_and_nullable_indexed_carriers_retain_the_same_access_span() {
    // Each value family selects a distinct MIR indexed carrier or nullable path.
    // PHP explicitly rejects concrete-class shared payloads (shared::supported_payload);
    // all other rows exercise PHP as well as the interpreter and native profiles.
    for (element, php) in [
        ("bool", true),
        ("?bool", true),
        ("float", true),
        ("?float", true),
        ("string", true),
        ("?string", true),
        ("?int", true),
        ("Item", true),
        ("?Item", true),
        ("Bytes", true),
        ("?Bytes", true),
        ("function(): int", true),
        ("?function(): int", true),
        ("View", true),
        ("?View", true),
        ("State", true),
        ("?State", true),
        ("Packet", true),
        ("?Packet", true),
        ("mixed", true),
        ("?mixed", true),
        ("SharedReference<Item>", false),
        ("?SharedReference<Item>", false),
        ("WeakReference<Item>", false),
        ("?WeakReference<Item>", false),
        ("WritableSharedReference<Item>", false),
        ("?WritableSharedReference<Item>", false),
        ("WritableWeakReference<Item>", false),
        ("?WritableWeakReference<Item>", false),
        ("ReadonlySharedReferenceAccess<Item>", false),
        ("?ReadonlySharedReferenceAccess<Item>", false),
        ("WritableSharedReferenceAccess<Item>", false),
        ("?WritableSharedReferenceAccess<Item>", false),
    ] {
        let setup = format!("List<{element}> $values = [];");
        let source = format!(
            "function inspect({element} $value): void {{}}\n{}",
            source(&setup, "inspect($values[index(0)]);")
        );
        assert_panic(
            &source,
            ExpectedPanic {
                selected: "$values[index(0)]",
                code: "P1310",
                bounds: Some((0, 0)),
                stdout: "before\nindex\n",
                php,
            },
        );
    }
}

#[test]
fn failed_owned_index_replacement_does_not_drop_old_new_or_surrounding_owners() {
    let source = source(
        "writable List<Item> $values = [new Item(1)];",
        "$values[index(1)] = itemReplacement();",
    );
    assert_panic(
        &source,
        ExpectedPanic {
            selected: "$values[index(1)]",
            code: "P1310",
            bounds: Some((1, 1)),
            stdout: "before\nindex\nrhs\n",
            php: true,
        },
    );
}

#[test]
fn nested_collection_access_attributes_the_first_failing_access() {
    for (setup, selected, bounds) in [
        ("List<Bytes> $outer = [];", "$outer[index(0)]", (0, 0)),
        (
            "List<Bytes> $outer = [Bytes::allocate(1)];",
            "$outer[0][index(1)]",
            (1, 1),
        ),
    ] {
        let statement = if bounds.1 == 0 {
            "let $value = $outer[index(0)][0];"
        } else {
            "let $value = $outer[0][index(1)];"
        };
        assert_panic(
            &source(setup, statement),
            ExpectedPanic {
                selected,
                code: if bounds.1 == 0 { "P1310" } else { "P1301" },
                bounds: Some(bounds),
                stdout: "before\nindex\n",
                php: true,
            },
        );
    }
}

#[test]
fn list_remove_at_uses_its_call_span_after_nested_index_evaluation() {
    assert_panic(
        &source(
            "writable List<int> $values = [7];",
            "let $value = $values->removeAt(index(1));",
        ),
        ExpectedPanic {
            selected: "$values->removeAt(index(1))",
            code: "P1310",
            bounds: Some((1, 1)),
            stdout: "before\nindex\n",
            php: false,
        },
    );
}

#[test]
fn list_insert_at_uses_its_call_span_after_nested_index_and_value_evaluation() {
    // These carriers select the scalar, nullable, payload, and paired insertion paths.
    for (element, initial, replacement) in [
        ("int", "7", "replacement()"),
        ("?int", "null", "nullableReplacement()"),
        ("Item", "new Item(1)", "itemReplacement()"),
        ("Packet", "Packet::Value(1)", "payloadReplacement()"),
        ("function(): int", "fn() => 7", "callbackReplacement()"),
    ] {
        let setup = format!("writable List<{element}> $values = [{initial}];");
        for index in [2, -1] {
            let selected = format!("$values->insertAt(index({index}), {replacement})");
            assert_panic(
                &source(&setup, &format!("{selected};")),
                ExpectedPanic {
                    selected: &selected,
                    code: "P1310",
                    bounds: Some((index, 1)),
                    stdout: "before\nindex\nrhs\n",
                    php: true,
                },
            );
        }
    }
}

#[test]
fn sorted_dictionary_indexed_replacement_acquires_new_before_releasing_old() {
    let source = r#"
class Item {
    function __construct(int $value) {}
    function read(): int { return $this->value; }
    function __destruct() {
        try { echo "drop {$this->value}\n"; } catch (Doria\Std\Io\IoError) {}
    }
}
function make(int $value): Item {
    echo "make {$value}\n";
    return new Item($value);
}
function inspect(Item $value): int { return $value->read(); }
function main(): void {
    writable SortedDictionary<string, Item> $values = SortedDictionary::from([]);
    $values["item"] = make(1);
    let $first = inspect($values["item"]);
    echo "first {$first}\n";
    $values["item"] = make(2);
    let $second = inspect($values["item"]);
    echo "second {$second}\n";
}
"#;
    let stdout = "make 1\nfirst 1\nmake 2\ndrop 1\nsecond 2\ndrop 2\n";
    assert_success(source, stdout, true);
}

#[test]
fn indexed_paired_shared_handles_borrow_without_removing_the_element() {
    let source = r#"
interface Named { function read(): int; }
class NamedItem implements Named {
    function __construct(int $value) {}
    function read(): int { return $this->value; }
    function __destruct() {
        try { echo "drop item\n"; } catch (Doria\Std\Io\IoError) {}
    }
}
function inspect(SharedReference<Named> $item): int { return $item->read(); }
function main(): void {
    SharedReference<Named> $owner = shared new NamedItem(7);
    List<SharedReference<Named>> $values = [$owner->share()];
    echo "before {$values->count}\n";
    echo "first {inspect($values[0])}\n";
    echo "second {inspect($values[0])}\n";
    echo "after {$values->count}\n";
}
"#;
    let stdout = "before 1\nfirst 7\nsecond 7\nafter 1\ndrop item\n";
    assert_success(source, stdout, true);
}

#[test]
fn nullable_collection_indices_borrow_present_and_null_values_without_removal() {
    let source = r#"
function index(int $number): int { echo "index {$number}\n"; return $number; }
function key(string $name): string { echo "key {$name}\n"; return $name; }
function inspectBytes(?Bytes $value): void {
    if ($value == null) { echo "null bytes\n"; }
    else if ($value->length == 0) { echo "empty bytes\n"; }
    else { echo "bytes {$value->length}:{$value[0]}\n"; }
}
function inspectList(?List<int> $value): void {
    if ($value == null) { echo "null list\n"; }
    else { echo "list {$value->count}:{$value[0]}\n"; }
}
function main(): void {
    List<?Bytes> $bytes = [null, Bytes::allocate(0), Bytes::allocate(1)];
    inspectBytes($bytes[index(0)]);
    inspectBytes($bytes[index(1)]);
    inspectBytes($bytes[index(2)]);
    inspectBytes($bytes[index(2)]);
    List<?List<int>> $lists = [null, [7]];
    inspectList($lists[index(0)]);
    inspectList($lists[index(1)]);
    Dictionary<string, ?Bytes> $dictionary = ["none" => null, "some" => Bytes::allocate(2)];
    inspectBytes($dictionary[key("none")]);
    inspectBytes($dictionary[key("some")]);
    echo "counts {$bytes->count}:{$lists->count}:{$dictionary->count}\n";
}
"#;
    assert_success(
        source,
        "index 0\nnull bytes\nindex 1\nempty bytes\nindex 2\nbytes 1:0\nindex 2\nbytes 1:0\nindex 0\nnull list\nindex 1\nlist 1:7\nkey none\nnull bytes\nkey some\nbytes 2:0\ncounts 3:2:2\n",
        true,
    );
}

#[test]
fn writable_nullable_collection_index_borrows_update_the_original_slot() {
    let source = r#"
function index(int $number): int { echo "index {$number}\n"; return $number; }
function key(): string { echo "key\n"; return "value"; }
function update(writable ?Bytes $value): void {
    if ($value != null) { $value[0] = 7; }
}
function inspectBytes(?Bytes $value): void {
    if ($value != null) { echo "bytes {$value->length}:{$value[0]}\n"; }
}
function inspectList(?List<int> $value): void {
    if ($value != null) { echo "list {$value->count}:{$value[1]}\n"; }
}
function main(): void {
    let $replace = function (writable ?Bytes $value): void {
        $value = Bytes::allocate(2);
        $value[1] = 9;
    };
    let $replaceList = function (writable ?List<int> $value): void { $value = [8, 9]; };
    writable List<?Bytes> $bytes = [Bytes::allocate(1), null];
    update($bytes[index(0)]);
    $replace($bytes[index(1)]);
    inspectBytes($bytes[0]);
    inspectBytes($bytes[1]);
    writable Dictionary<string, ?Bytes> $dictionary = ["value" => null];
    $replace($dictionary[key()]);
    inspectBytes($dictionary["value"]);
    writable List<?List<int>> $lists = [null];
    $replaceList($lists[index(0)]);
    inspectList($lists[0]);
}
"#;
    assert_success(
        source,
        "index 0\nindex 1\nbytes 1:7\nbytes 2:0\nkey\nbytes 2:0\nindex 0\nlist 2:9\n",
        true,
    );
}

#[test]
fn nullable_collection_removal_transfers_null_empty_and_present_values() {
    let source = r#"
function index(): int { echo "index\n"; return 0; }
function main(): void {
    writable List<?Bytes> $values = [null, Bytes::allocate(0), Bytes::allocate(1)];
    ?Bytes $missing = $values->removeAt(index());
    if ($missing == null) { echo "null:{$values->count}\n"; }
    ?Bytes $empty = $values->removeAt(index());
    if ($empty != null) { echo "empty:{$empty->length}:{$values->count}\n"; }
    ?Bytes $present = $values->removeAt(index());
    if ($present != null) { echo "present:{$present->length}:{$present[0]}:{$values->count}\n"; }
    if ($empty != null) { echo "still empty:{$empty->length}\n"; }
}
"#;
    // PHP's existing List::removeAt boundary is separate from indexed [] access.
    assert_success(
        source,
        "index\nnull:2\nindex\nempty:0:1\nindex\npresent:1:0:0\nstill empty:0\n",
        false,
    );
}

#[test]
fn nullable_collection_index_cannot_forge_owned_or_writable_access() {
    let source = r#"
function inspect(?Bytes $value): void {}
function main(): void {
    List<?Bytes> $values = [Bytes::allocate(1)];
    inspect($values[0]);
}
"#;
    let original = doriac::lower_source_to_mir(PATH, source).unwrap();
    for mutation in ["transfer", "writable", "owned"] {
        let mut program = original.clone();
        let function = program
            .functions
            .iter_mut()
            .find(|function| function.name == "main")
            .unwrap();
        let (target, expression) = function
            .blocks
            .iter_mut()
            .flat_map(|block| &mut block.statements)
            .find_map(|statement| match statement {
                doriac::mir::Statement::AssignLocal {
                    target,
                    value:
                        doriac::mir::Rvalue::NullableCollection(
                            expression @ doriac::mir::NullableCollectionExpression::Index { .. },
                        ),
                } => Some((*target, expression)),
                _ => None,
            })
            .expect("index argument must be materialized as a borrowed nullable collection");
        match mutation {
            "transfer" => {
                let doriac::mir::NullableCollectionExpression::Index { transfer, .. } = expression
                else {
                    unreachable!()
                };
                *transfer = true;
                function.locals[target.0].owned = true;
            }
            "writable" => function.locals[target.0].writable = true,
            "owned" => function.locals[target.0].owned = true,
            _ => unreachable!(),
        }
        let error = doriac::mir_validation::validate_program(&program).unwrap_err();
        let expected = match mutation {
            "transfer" => "transfer requires a writable List source",
            "writable" => "writable nullable collection borrow requires a writable source place",
            "owned" => "nullable collection local ownership disagrees with its source value",
            _ => unreachable!(),
        };
        assert!(error.message.contains(expected), "{mutation}: {error:?}");
    }
}

#[test]
fn ordinary_nullable_collection_parameter_replacement_writes_through_the_element() {
    let source = r#"
function replace(writable ?Bytes $value): void { $value = Bytes::allocate(2); }
function main(): void {
    writable List<?Bytes> $values = [null];
    replace($values[0]);
    if ($values[0] == null) { echo "null"; } else { echo "set"; }
    echo "\n";
}
"#;
    let mir = doriac::lower_source_to_mir(PATH, source)
        .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"));
    doriac::mir_validation::validate_program(&mir).unwrap();
    let result = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(result.stdout, b"set\n");
    native_execution::assert_native_execution(&mir, doriac::backend::NativeProfile::Fast, "set\n");
    #[cfg(feature = "llvm-backend")]
    native_execution::assert_native_execution(
        &mir,
        doriac::backend::NativeProfile::Release,
        "set\n",
    );
}

#[test]
fn nullable_mixed_indices_preserve_null_and_borrowed_payload_ownership() {
    let source = r#"
class Item {
    function __construct(int $number) {}
    function __destruct() {
        try { echo "drop {$this->number}\n"; } catch (Doria\Std\Io\IoError) {}
    }
}
function index(int $number): int { echo "index {$number}\n"; return $number; }
function key(string $name): string { echo "key {$name}\n"; return $name; }
function inspect(?mixed $value): void {
    if ($value == null) { echo "null\n"; }
    else if ($value is int) { echo "int {$value}\n"; }
    else if ($value is Item) { echo "item {$value->number}\n"; }
}
function main(): void {
    List<?mixed> $values = [null, 7, new Item(9)];
    inspect($values[index(0)]);
    inspect($values[index(1)]);
    inspect($values[index(2)]);
    inspect($values[index(2)]);
    Dictionary<string, ?mixed> $dictionary = ["none" => null, "some" => new Item(11)];
    inspect($dictionary[key("none")]);
    inspect($dictionary[key("some")]);
    echo "counts {$values->count}:{$dictionary->count}\n";
}
"#;
    assert_success(
        source,
        "index 0\nnull\nindex 1\nint 7\nindex 2\nitem 9\nindex 2\nitem 9\nkey none\nnull\nkey some\nitem 11\ncounts 3:2\ndrop 11\ndrop 9\n",
        true,
    );
    let removed = r#"
class Item {
    function __construct(int $number) {}
    function __destruct() {
        try { echo "drop {$this->number}\n"; } catch (Doria\Std\Io\IoError) {}
    }
}
function index(): int { echo "index\n"; return 0; }
function inspect(?mixed $value): void {
    if ($value == null) { echo "null\n"; }
    else if ($value is Item) { echo "item {$value->number}\n"; }
}
function main(): void {
    writable List<?mixed> $values = [null, new Item(9)];
    ?mixed $missing = $values->removeAt(index());
    inspect($missing);
    ?mixed $present = $values->removeAt(index());
    inspect($present);
    inspect($present);
    echo "count {$values->count}\n";
}
"#;
    assert_success(
        removed,
        "index\nnull\nindex\nitem 9\nitem 9\ncount 0\ndrop 9\n",
        false,
    );
}

#[test]
fn mixed_index_ownership_preserves_retained_reads_and_checks_removal() {
    use doriac::mir::{MixedExpression, NullableMixedExpression, Rvalue, Statement};
    for element in ["mixed", "?mixed"] {
        let retained_source = format!(
            "function main(): void {{ List<{element}> $values = [7]; {element} $retained = $values[0]; }}"
        );
        let retained = doriac::lower_source_to_mir(PATH, &retained_source).expect(
            "mixed indexed local bindings retain an owned box without removing the element",
        );
        doriac::mir_validation::validate_program(&retained).unwrap();
        let retained_function = retained
            .functions
            .iter()
            .find(|function| function.name == "main")
            .unwrap();
        let retained_local = retained_function
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .find_map(|statement| match statement {
                Statement::AssignLocal {
                    target,
                    value:
                        Rvalue::Mixed(MixedExpression::CollectionIndex {
                            transfer: true,
                            remove: false,
                            ..
                        }),
                }
                | Statement::AssignLocal {
                    target,
                    value:
                        Rvalue::NullableMixed(NullableMixedExpression::CollectionIndex {
                            transfer: true,
                            remove: false,
                            ..
                        }),
                } => Some(*target),
                _ => None,
            })
            .expect("retained indexed read must keep the collection element in place");
        assert!(retained_function.locals[retained_local.0].owned);

        let source = format!(
            "function inspect({element} $value): void {{}} function main(): void {{ List<{element}> $values = [7]; inspect($values[0]); }}"
        );
        let original = doriac::lower_source_to_mir(PATH, &source).unwrap();
        for (transfer, remove) in [(false, true), (true, true)] {
            let mut program = original.clone();
            let function = program
                .functions
                .iter_mut()
                .find(|function| function.name == "main")
                .unwrap();
            let target = function
                .blocks
                .iter_mut()
                .flat_map(|block| &mut block.statements)
                .find_map(|statement| match statement {
                    Statement::AssignLocal {
                        target,
                        value:
                            Rvalue::Mixed(MixedExpression::CollectionIndex {
                                transfer: found_transfer,
                                remove: found_remove,
                                ..
                            }),
                    }
                    | Statement::AssignLocal {
                        target,
                        value:
                            Rvalue::NullableMixed(NullableMixedExpression::CollectionIndex {
                                transfer: found_transfer,
                                remove: found_remove,
                                ..
                            }),
                    } => {
                        *found_transfer = transfer;
                        *found_remove = remove;
                        Some(*target)
                    }
                    _ => None,
                })
                .expect("indexed mixed argument must have a materialized borrow");
            function.locals[target.0].owned = transfer;
            let error = doriac::mir_validation::validate_program(&program).unwrap_err();
            let expected = if transfer {
                "element removal requires a writable List source"
            } else {
                "mixed element removal must transfer ownership"
            };
            assert!(
                error.message.contains(expected),
                "{element} {transfer}/{remove}: {error:?}"
            );
        }
    }
}

#[test]
fn indexed_copy_scalar_writable_callbacks_replace_the_original_value_once() {
    for (setup, element) in [
        ("writable List<int> $values = [3];", "int"),
        ("writable Bytes $values = Bytes::allocate(1);", "uint8"),
    ] {
        let source = format!(
            r#"
function index(): int {{ echo "index\n"; return 0; }}
function main(): void {{
    {setup}
    let $replace = function (writable {element} $value): void {{ $value = 9; }};
    $replace($values[index()]);
    echo "value {{$values[0]}}\n";
}}
"#
        );
        assert_success(&source, "index\nvalue 9\n", true);
    }
    assert_success(
        r#"
function index(): int { echo "index\n"; return 0; }
function main(): void {
    writable List<string> $values = ["before"];
    let $replace = function (writable string $value): void { $value = $value . "!"; };
    $replace($values[index()]);
    echo "{$values[0]}\n";
    $replace($values[index()]);
    echo "{$values[0]}\n";
}
"#,
        "index\nbefore!\nindex\nbefore!!\n",
        true,
    );
}

fn assert_success(source: &str, stdout: &str, php: bool) {
    let mir = doriac::lower_source_to_mir(PATH, source).unwrap();
    let output = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(output.stdout, stdout.as_bytes());
    assert_eq!(output.stderr, b"");
    assert_eq!(output.exit_status, 0);
    native_execution::assert_native_execution(&mir, NativeProfile::Fast, stdout);
    #[cfg(feature = "llvm-backend")]
    native_execution::assert_native_execution(&mir, NativeProfile::Release, stdout);
    if !php {
        return;
    }
    let php = doriac::compile_source_to_php(PATH, source).unwrap();
    let Ok(version) = std::process::Command::new("php").arg("--version").output() else {
        eprintln!("PHP unavailable; indexed-access compatibility execution skipped");
        return;
    };
    assert!(version.status.success());
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").unwrap()
    );
    let output = php_execution::Script::new(&script)
        .command(&[])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, stdout.as_bytes());
    assert_eq!(output.stderr, b"");
}
