#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution_runner.rs"]
mod native_execution;

#[path = "common/php_execution.rs"]
mod php_execution;

use doriac::backend::{BackendOutput, BackendTarget, NativeProfile};
use doriac::hir;
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn copy_parameter_values_and_writable_callback_borrows_match_native() {
    for (name, source, stdout) in [
        (
            "copy-parameters.doria",
            include_str!("fixtures/copy_parameter_isolation.doria"),
            b"copy parameters passed".as_slice(),
        ),
        (
            "managed-copy-captures.doria",
            include_str!("fixtures/managed_copy_parameter_capture.doria"),
            b"managed copy captures passed".as_slice(),
        ),
        (
            "field-argument-transport.doria",
            include_str!("fixtures/field_argument_transport.doria"),
            b"6;6;6;6;6;6;6;6;6;6;6;6;6;6;6;6;6;6;6;6;6;6;6;6".as_slice(),
        ),
        (
            "field-callback-transport.doria",
            include_str!("fixtures/field_callback_transport.doria"),
            b"4;-1;4;4;9;9;4;4;4;-1;6;-1;5;4;7;drop;2;8;drop;".as_slice(),
        ),
        (
            "field-callback-results.doria",
            include_str!("fixtures/field_callback_results.doria"),
            b"6;6;6;6;6;6;drop 10;10;drop 11;null;drop 12;caught;13;".as_slice(),
        ),
        (
            "outcome-storage.doria",
            include_str!("fixtures/outcome_storage.doria"),
            b"6;6;6;6;6;6;7".as_slice(),
        ),
        (
            "outcome-property-storage.doria",
            include_str!("fixtures/outcome_property_storage.doria"),
            b"6;6;6;6;6;6;6;6;6;6;6;39;7;7;receiver;rhs;drop;receiver;fail;drop;caught;6;6;6;-1;-1;plain receiver;plain rhs;plain drop;plain receiver;plain rhs;plain fail;plain caught;74;".as_slice(),
        ),
        (
            "field-argument-evaluation.doria",
            include_str!("fixtures/field_argument_evaluation.doria"),
            b"make 3;scalar 2;make 1;consume;sum 6;receiver;new reader;make 4;scalar 5;make 6;consume;drop reader;present 15;receiver;absent;callback;make 10;scalar 11;make 12;consume;answer 33;make 13;new argument;fail;drop argument;caught free;new receiver;make 14;new argument;fail;drop argument;drop receiver;caught receiver;new success;make 15;new argument;make 16;finish success;drop argument;drop success;finished 31;".as_slice(),
        ),
        (
            "nested-copy-transport.doria",
            include_str!("fixtures/nested_copy_transport.doria"),
            b"6;6;6;6;6;6;6;6;6;6;6;6;6;6;6;6;6;6;2;1;3;6;7;7;8;7;-1;-2;".as_slice(),
        ),
    ] {
        let mir = doriac::lower_source_to_mir(name, source).unwrap();
        let expected = doriac::mir_interpreter::interpret(&mir).unwrap();
        assert!(
            expected.runtime_diagnostic.is_none(),
            "{name}: {expected:?}"
        );
        assert_eq!(expected.stdout, stdout);
        assert_eq!(expected.exit_status, 0);
        assert!(expected.stderr.is_empty());
        assert_php_artifact_main(name, source, stdout);
        for profile in [NativeProfile::Fast]
            .into_iter()
            .chain(cfg!(feature = "llvm-backend").then_some(NativeProfile::Release))
        {
            let actual = native_execution::execute_native(&mir, profile);
            assert_eq!(
                actual.status.code(),
                Some(expected.exit_status),
                "{name}: {actual:?}"
            );
            assert_eq!(actual.stdout, expected.stdout, "{name}: {profile:?}");
            assert_eq!(actual.stderr, expected.stderr, "{name}: {profile:?}");
        }
    }
}

#[test]
fn once_callable_argument_failure_releases_owned_environment_before_catch() {
    let name = "once-callable-argument-failure.doria";
    let source = include_str!("fixtures/once_callable_argument_failure.doria");
    let stdout = b"new once;make;fail;drop once;caught once;";
    assert_php_artifact_main(name, source, stdout);
    let mir = doriac::lower_source_to_mir(name, source).unwrap();
    let interpreted = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert!(interpreted.runtime_diagnostic.is_none(), "{interpreted:?}");
    assert_eq!(interpreted.exit_status, 0);
    assert!(interpreted.stderr.is_empty());
    let mut observations = vec![("interpreter".to_string(), interpreted.stdout)];
    for profile in [NativeProfile::Fast]
        .into_iter()
        .chain(cfg!(feature = "llvm-backend").then_some(NativeProfile::Release))
    {
        let actual = native_execution::execute_native(&mir, profile);
        assert_eq!(actual.status.code(), Some(0), "{actual:?}");
        assert!(actual.stderr.is_empty(), "{actual:?}");
        observations.push((format!("{profile:?}"), actual.stdout));
    }
    assert!(
        observations.iter().all(|(_, actual)| actual == stdout),
        "owned callback environment must be released before catch: {observations:?}"
    );
}

#[test]
fn pending_once_calls_release_operands_before_catch_and_transfer_only_at_invocation() {
    let name = "pending-once-calls.doria";
    let source = include_str!("fixtures/pending_once_calls.doria");
    let stdout = b"new local;new arg;fail;drop arg;drop local;caught local;new nested;new nested arg;fail;drop nested arg;drop nested;caught nested;new temporary;new temp arg;fail;drop temp arg;drop temporary;caught temporary;new first;fail token;drop first;caught first;new repeat;new repeat arg;fail;drop repeat arg;drop repeat;caught repeat;new success;new success arg;enter 1;drop success arg;drop success;success 1;new body;new body arg;enter 0;drop body arg;drop body;caught body;";
    assert_php_artifact_main(name, source, stdout);
    let mir = doriac::lower_source_to_mir(name, source).unwrap();
    let interpreted = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert!(interpreted.runtime_diagnostic.is_none(), "{interpreted:?}");
    assert_eq!(interpreted.exit_status, 0);
    assert!(interpreted.stderr.is_empty());
    assert_eq!(interpreted.stdout, stdout);
    for profile in [NativeProfile::Fast]
        .into_iter()
        .chain(cfg!(feature = "llvm-backend").then_some(NativeProfile::Release))
    {
        let actual = native_execution::execute_native(&mir, profile);
        assert_eq!(actual.status.code(), Some(0), "{profile:?}: {actual:?}");
        assert!(actual.stderr.is_empty(), "{profile:?}: {actual:?}");
        assert_eq!(actual.stdout, stdout, "{profile:?}");
    }
}

fn assert_php_artifact_main(name: &str, source: &str, stdout: &[u8]) {
    let php = doriac::compile_source(name, source, BackendTarget::Php).unwrap();
    let directory = native_execution::ExecutionDirectory::new();
    let path = directory.path().join("main.php");
    doriac::backend::publish_output(&path, php).unwrap();
    let actual = Command::new("php")
        .args(["-d", "ffi.enable=1"])
        .arg(path)
        .output()
        .unwrap();
    assert_eq!(actual.status.code(), Some(0), "{name}: {actual:?}");
    assert_eq!(actual.stdout, stdout, "{name}: {actual:?}");
    assert!(actual.stderr.is_empty(), "{name}: {actual:?}");
}

#[test]
fn managed_copy_parameter_replacement_matches_native() {
    let name = "managed-copy-replacement.doria";
    let source = include_str!("fixtures/managed_copy_parameter_isolation.doria");
    let stdout = b"managed copy parameters passed";
    assert_php_artifact_main(name, source, stdout);
    let mir = doriac::lower_source_to_mir(name, source).unwrap_or_else(|errors| {
        panic!("PHP matched exact managed Copy output; native MIR rejected the same source: {errors:?}")
    });
    let expected = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert!(expected.runtime_diagnostic.is_none(), "{expected:?}");
    assert_eq!(expected.stdout, stdout);
    for profile in [NativeProfile::Fast]
        .into_iter()
        .chain(cfg!(feature = "llvm-backend").then_some(NativeProfile::Release))
    {
        let actual = native_execution::execute_native(&mir, profile);
        assert_eq!(
            actual.status.code(),
            Some(expected.exit_status),
            "{actual:?}"
        );
        assert_eq!(actual.stdout, expected.stdout, "{profile:?}");
        assert_eq!(actual.stderr, expected.stderr, "{profile:?}");
    }
}

#[test]
fn empty_collection_constructors_do_not_require_input_callbacks_or_cloning() {
    let template = r#"
class State { static writable string $log = ""; }
class Ordered implements Comparable<Ordered> {
    function __construct(int $id) {}
    function compare(Ordered $other): Ordering {
        if ($this->id < $other->id) { return Ordering::Less; }
        if ($this->id > $other->id) { return Ordering::Greater; }
        return Ordering::Equal;
    }
    function __destruct() { State::log = State::log . "O{$this->id};"; }
}
class Hashed implements Equatable<Hashed>, Hashable {
    function __construct(int $id) {}
    function equals(Hashed $other): bool { return $this->id == $other->id; }
    function hash(): uint64 { return 7; }
    function __destruct() { State::log = State::log . "H{$this->id};"; }
}
class Item {
    function __construct(int $id) {}
    function __destruct() { State::log = State::log . "I{$this->id};"; }
}
class Factory {
    static function from(List<int> $source): SortedDictionary<Ordered, int> {
        echo "F";
        return SortedDictionary::from(EMPTY);
    }
}
function exercise(): void {
    let $authored = Factory::from(EMPTY);
    echo $authored->isEmpty; echo "|";
    writable Set<Hashed> $hashed = Set::from(EMPTY);
    writable SortedSet<Ordered> $sorted = SortedSet::from(EMPTY);
    writable SortedDictionary<Ordered, int> $map = SortedDictionary::from(EMPTY);
    writable PriorityQueue<Ordered> $heap = PriorityQueue::from(EMPTY);
    writable Deque<Item> $deque = Deque::from(EMPTY);
    echo $hashed->count; echo $sorted->count; echo $map->count;
    echo $heap->count; echo $deque->count; echo "|";
    $hashed->add(new Hashed(1));
    $sorted->add(new Ordered(2));
    $map[new Ordered(3)] = 3;
    $heap->push(new Ordered(4));
    $deque->pushBack(new Item(5));
    echo $hashed->count; echo $sorted->count; echo $map->count;
    echo $heap->count; echo $deque->count; echo "|";
}
function main(): void { exercise(); echo State::log; }
"#;
    for input in ["[]", "([])", "(([]))"] {
        let source = template.replace("EMPTY", input);
        let mir = doriac::lower_source_to_mir("empty-constructors.doria", &source)
            .unwrap_or_else(|error| panic!("{input}: {error:?}"));
        let expected = doriac::mir_interpreter::interpret(&mir).unwrap();
        assert_eq!(expected.stdout, b"Ftrue|00000|11111|I5;O4;O3;O2;H1;");
        assert_eq!(expected.exit_status, 0);
        assert!(expected.stderr.is_empty());
        let php = doriac::compile_source_to_php("empty-constructors.doria", &source).unwrap();
        assert!(!php_function_body(&php, "exercise").contains("->fromSource("));
        assert_php_main_execution(&php, &expected.stdout, expected.exit_status);
        for profile in [NativeProfile::Fast]
            .into_iter()
            .chain(cfg!(feature = "llvm-backend").then_some(NativeProfile::Release))
        {
            let output = native_execution::execute_native(&mir, profile);
            assert_eq!(
                output.status.code(),
                Some(expected.exit_status),
                "{output:?}"
            );
            assert_eq!(output.stdout, expected.stdout, "{profile:?}: {input}");
            assert_eq!(output.stderr, expected.stderr, "{profile:?}: {input}");
        }
    }
}

#[test]
fn receiver_aliases_preserve_lazy_calls_and_statement_owner_cleanup_across_backends() {
    let source = r#"
class State { static writable string $log = ""; }
class Failure implements Error { function __construct(string $message) {} }
class Item {
    function __construct(int $id) {}
    function __destruct() { State::log = State::log . "I{$this->id};"; }
}
class Owner {
    List<Item> $items;
    function __construct(int $id) { $this->items = [new Item($id)]; }
    function pick(int $marker): Item {
        State::log = State::log . "P{$marker};";
        return $this->items[0];
    }
    function withItem(Item $argument, int $marker): Item {
        State::log = State::log . "P{$marker};";
        return $this->items[0];
    }
    function values(): List<Item> { return $this->items; }
    function __destruct() { State::log = State::log . "O{$this->items[0]->id};"; }
}
function make(int $id): Owner {
    State::log = State::log . "M{$id};";
    return new Owner($id);
}
function event(int $id): int {
    State::log = State::log . "A{$id};";
    return $id;
}
function fail(): int throws Failure {
    State::log = State::log . "F;";
    throw new Failure("expected");
}
function simple(Owner $owner): Item { return $owner->pick(0); }
function optional(?Owner $owner, int $marker): ?Item { return $owner?->pick($marker); }
function maybe(?Owner $owner): ?Item { return $owner?->pick(event(1)); }
function iterate(Owner $owner): void {
    foreach ($owner->values() as Item $item) { echo $item->id; }
}
function exercise(): void {
    let $owner = make(1);
    echo simple($owner)->id;
    iterate($owner);
    let $missing = maybe(null);
    echo $missing == null;
    let $present = maybe($owner);
    echo $present?->id ?? 0;
    echo make(2)->pick(event(2))->id;
    try { echo make(3)->pick(fail())->id; } catch (Failure) { echo "c"; }
    echo make(4)->withItem(new Item(5), event(4))->id;
    try { echo make(6)->withItem(new Item(7), fail())->id; } catch (Failure) { echo "f"; }
    echo optional($owner, 0)?->id ?? 0;
}
function main(): void { exercise(); echo "|"; echo State::log; echo "\n"; }
"#;
    let mir = doriac::lower_source_to_mir("receiver-aliases.doria", source).unwrap();
    let expected = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(
        expected.stdout,
        b"11true12c4f1|M1;P0;A1;P1;M2;A2;P2;O2;I2;M3;F;O3;I3;M4;A4;P4;I5;O4;I4;M6;F;I7;O6;I6;P0;O1;I1;\n"
    );
    assert_eq!(expected.exit_status, 0);
    assert!(expected.stderr.is_empty());
    let php = doriac::compile_source_to_php("receiver-aliases.doria", source).unwrap();
    for function in ["simple", "optional"] {
        let body = php_function_body(&php, function);
        assert!(body.contains("$__doria_receiver_home_"), "{body}");
        assert!(body.contains("unset($__doria_receiver_home_"), "{body}");
        assert!(!body.contains("(function()"), "{body}");
    }
    assert!(php_function_body(&php, "iterate").contains("unset($__doria_receiver_home_"));
    assert!(!php.contains("__doria_drop_value($__doria_receiver_home_"));
    assert_php_main_execution(&php, &expected.stdout, expected.exit_status);
    for profile in [NativeProfile::Fast]
        .into_iter()
        .chain(cfg!(feature = "llvm-backend").then_some(NativeProfile::Release))
    {
        let output = native_execution::execute_native(&mir, profile);
        assert_eq!(
            output.status.code(),
            Some(expected.exit_status),
            "{profile:?}: {output:?}"
        );
        assert_eq!(output.stdout, expected.stdout, "{profile:?}");
        assert_eq!(output.stderr, expected.stderr, "{profile:?}");
    }
}

#[test]
fn foreach_property_names_use_checked_families_across_backends() {
    let source = r#"
class Store {
    List<int> $keys = [1, 2];
    Deque<int> $values = Deque::from([3, 4]);
    Dictionary<string, int> $map = ["k" => 5];
}
function main(): void {
    let $store = new Store();
    foreach ((($store->keys)) as int $index => int $value) { echo $index; echo $value; }
    foreach ((($store->values)) as int $value) { echo $value; }
    foreach ((($store->map->keys)) as string $key) { echo $key; }
    foreach ((($store->map->values)) as int $value) { echo $value; }
}
"#;
    let mir = doriac::lower_source_to_mir("foreach-properties.doria", source).unwrap();
    let projections = mir
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.statements)
        .filter_map(|statement| match statement {
            doriac::mir::Statement::ControlFlowPlan(doriac::mir::ControlFlowPlan::Foreach(
                plan,
            )) => Some(plan.projection),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(projections.len(), 4);
    assert_eq!(
        projections
            .iter()
            .filter(|projection| **projection == doriac::mir::ForeachProjection::Main)
            .count(),
        2
    );
    for projection in [
        doriac::mir::ForeachProjection::Keys,
        doriac::mir::ForeachProjection::Values,
    ] {
        assert_eq!(
            projections
                .iter()
                .filter(|actual| **actual == projection)
                .count(),
            1
        );
    }
    let expected = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(expected.stdout, b"011234k5");
    assert_eq!(expected.exit_status, 0);
    assert!(expected.stderr.is_empty());
    let php = doriac::compile_source_to_php("foreach-properties.doria", source).unwrap();
    assert_php_main_execution(&php, &expected.stdout, expected.exit_status);
    for profile in [NativeProfile::Fast]
        .into_iter()
        .chain(cfg!(feature = "llvm-backend").then_some(NativeProfile::Release))
    {
        let output = native_execution::execute_native(&mir, profile);
        assert_eq!(
            output.status.code(),
            Some(expected.exit_status),
            "{profile:?}: {output:?}"
        );
        assert_eq!(output.stdout, expected.stdout, "{profile:?}");
        assert_eq!(output.stderr, expected.stderr, "{profile:?}");
    }
}

#[test]
fn core_value_adapters_reuse_named_functions_across_equality_search_and_collections() {
    let source = r#"
class Failure implements Error { function __construct(string $message) {} }
class State { static writable string $order = ""; }
class Key implements Comparable<Key>, Equatable<Key>, Hashable, Cloneable {
    function __construct(int $id) {}
    function compare(Key $other): Ordering {
        if ($this->id < $other->id) { return Ordering::Less; }
        if ($this->id > $other->id) { return Ordering::Greater; }
        return Ordering::Equal;
    }
    function equals(Key $other): bool { return $this->id == $other->id; }
    function hash(): uint64 { return 7; }
    function clone(): self { return new Key($this->id); }
}
class Cache { SortedSet<Key> $values = SortedSet::from([new Key(2), new Key(1)]); }
function same(Key $left, Key $right): bool { return $left == $right; }
function genericSame<T implements Equatable<T>>(T $left, T $right): bool {
    return $left == $right;
}
function nullableSame(?Key $left, ?Key $right): bool { return $left == $right; }
function locate(List<Key> $values, Key $wanted): bool { return $values->contains($wanted); }
function make(string $label): Key {
    State::order = State::order . $label;
    return new Key(1);
}
function fail(): Key throws Failure {
    State::order = State::order . "F";
    throw new Failure("expected");
}
function main(): void {
    let $left = new Key(1);
    let $right = new Key(1);
    echo same($left, $right);
    echo genericSame($left, $right);
    echo nullableSame($left, null);
    echo nullableSame(null, null);
    let $values = [new Key(1), new Key(2)];
    echo locate($values, $right);
    let $set = Set::from([new Key(1), new Key(2)]);
    echo $set->contains($left);
    let $cache = new Cache();
    echo "|"; echo $cache->values->count; echo "|";
    foreach ($cache->values as Key $key) { echo $key->id; }
    let $deque = Deque::from([new Key(3)]);
    echo "|"; echo $deque->count; echo "|";
    foreach ($deque as Key $key) { echo $key->id; }
    echo "|"; echo same(make("L"), make("R"));
    try { echo same(fail(), make("X")); } catch (Failure) {}
    echo "|"; echo State::order; echo "\n";
}
"#;
    let mir = doriac::lower_source_to_mir("core-adapters.doria", source).unwrap();
    let expected = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(
        expected.stdout,
        b"truetruefalsetruetruetrue|2|12|1|3|true|LRF\n"
    );
    assert_eq!(expected.exit_status, 0);
    assert!(expected.stderr.is_empty());
    let php = doriac::compile_source_to_php("core-adapters.doria", source).unwrap();
    let helpers = php
        .lines()
        .filter(|line| line.starts_with("function __doria_core_callback_"))
        .collect::<Vec<_>>();
    assert_eq!(
        helpers.len(),
        4,
        "one adapter per checked type and operation"
    );
    for helper in helpers {
        let name = helper
            .strip_prefix("function ")
            .unwrap()
            .split('(')
            .next()
            .unwrap();
        assert!(
            php.contains(&format!("\"{name}\"")),
            "reused callback {name}"
        );
    }
    for allocation in [
        "static fn($left, $right)",
        "static fn($value)",
        "(static function ($left, $right) { return $left === null",
        "static function ($left, $right): bool { return $left === null",
    ] {
        assert!(
            !php.contains(allocation),
            "capture-free adapter {allocation}"
        );
    }
    assert_php_main_execution(&php, &expected.stdout, expected.exit_status);
}

#[test]
fn local_assignment_regions_preserve_borrowed_storage_and_failed_replacement() {
    let source = r#"
class Failure implements Error { function __construct(string $message) {} }
class State { static writable string $log = ""; }
class Token {
    function __construct(string $label) {}
    function __destruct() { State::log = State::log . $this->label . "|"; }
}
function number(): int { return 7; }
function failedNumber(): int throws Failure { throw new Failure("expected"); }
function make(string $label): Token {
    State::log = State::log . "make" . $label . "|";
    return new Token($label);
}
function failedToken(): Token throws Failure { throw new Failure("expected"); }
function scalar(): int {
    let writable $value = 1;
    (($value)) = ((number()));
    try { $value = failedNumber(); } catch (Failure) {}
    return $value;
}
function copyParameter(writable int $value): void {
    $value = ((number()));
    try { $value = failedNumber(); } catch (Failure) {}
}
function borrowed(writable Token $value): void {
    $value = ((make("c")));
    try { $value = failedToken(); } catch (Failure) {}
}
function borrowing(): string {
    let writable $value = make("a");
    borrowed($value);
    return $value->label;
}
function owning(): string {
    let writable $value = make("a");
    $value = (((make("b"))));
    try { $value = failedToken(); } catch (Failure) {}
    return $value->label;
}
function main(): void {
    echo scalar(); echo "|";
    let writable $value = 1;
    copyParameter($value);
    echo $value; echo "|";
    echo borrowing(); echo "|";
    echo owning(); echo "|";
    echo State::log; echo "\n";
}
"#;
    let mir = doriac::lower_source_to_mir("assignment-regions.doria", source).unwrap();
    let expected = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(
        expected.stdout,
        b"7|1|c|b|makea|makec|a|c|makea|makeb|a|b|\n"
    );
    let php = doriac::compile_source_to_php("assignment-regions.doria", source).unwrap();
    for name in ["scalar", "copyParameter", "borrowed", "owning"] {
        assert!(
            !php_function_body(&php, name).contains("(function()"),
            "{name}"
        );
    }
    assert_php_main_execution(&php, &expected.stdout, expected.exit_status);
}

#[test]
fn grouped_root_operations_reuse_statement_regions_and_preserve_panic_sites() {
    for depth in [0, 1, 3] {
        let grouped = |value: &str| format!("{}{}{}", "(".repeat(depth), value, ")".repeat(depth));
        let source = format!(
            r#"
class Failure implements Error {{ function __construct(string $message) {{}} }}
function produce(string $value): string {{ echo "p"; return $value; }}
function fail(): string throws Failure {{ echo "f"; throw new Failure("!"); }}
function main(): void {{
    let $value = {};
    echo {};
    try {{ echo {}; }} catch (Failure $error) {{ echo $error->message; }}
    finally {{ echo "|"; }}
}}
"#,
            grouped("produce(\"x\")"),
            grouped("produce($value)"),
            grouped("fail()")
        );
        let mir = doriac::lower_source_to_mir("grouped-roots.doria", &source).unwrap();
        let expected = doriac::mir_interpreter::interpret(&mir).unwrap();
        assert_eq!(expected.stdout, b"ppxf!|");
        let php = doriac::compile_source_to_php("grouped-roots.doria", &source).unwrap();
        assert!(!php_function_body(&php, "main").contains("(function()"));
        assert_php_main_execution(&php, &expected.stdout, expected.exit_status);

        let source = format!(
            "function main(): void {{ let $minimum = -9223372036854775808; echo {}; }}",
            grouped("-$minimum")
        );
        let mir = doriac::lower_source_to_mir("grouped-panic.doria", &source).unwrap();
        let expected = doriac::mir_interpreter::interpret(&mir).unwrap();
        assert_eq!(expected.exit_status, 101);
        let php = doriac::compile_source_to_php("grouped-panic.doria", &source).unwrap();
        assert!(!php_function_body(&php, "main").contains("(function()"));
        let script = format!(
            "{}\n{}();",
            php.strip_prefix("<?php").unwrap(),
            php_function_name("main")
        );
        let output = php_execution::Script::new(&script)
            .command(&[])
            .env_remove("DORIA_RUNTIME_OUTCOME_V2")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(expected.exit_status));
        assert_eq!(output.stdout, expected.stdout);
        assert_eq!(output.stderr, expected.stderr);
    }
}

#[test]
fn statement_display_conversions_reuse_root_operation_regions() {
    let source = r#"
class Failure implements Error { function __construct(string $message) {} }
class State { static writable string $log = ""; }
class Label implements Displayable {
    function __construct(string $text) {}
    function toString(): string { return $this->text; }
    function __destruct() { State::log = State::log . "drop|"; }
}
function make(): Label { echo "made|"; return new Label("text|"); }
function fail(): string throws Failure { throw new Failure("caught|"); }
function main(): void {
    echo 7;
    echo make();
    echo State::log;
    try { echo fail(); } catch (Failure $error) { echo $error->message; }
    finally { echo "finally\n"; }
}
"#;
    for source in [
        source.to_string(),
        source
            .replace("echo make();", "echo (((make())));")
            .replace("echo fail();", "echo (((fail())));"),
    ] {
        let program = doriac::lower_source_to_mir("statement-display.doria", &source).unwrap();
        let expected = doriac::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(expected.stdout, b"7made|text|drop|caught|finally\n");
        let php = doriac::compile_source_to_php("statement-display.doria", &source).unwrap();
        for line in php_function_body(&php, "main")
            .lines()
            .filter(|line| line.contains("__doria_operation_enter("))
        {
            assert!(!line.contains("(function()"), "{line}");
        }
        let script = format!(
            "{}\n{}();",
            php.strip_prefix("<?php").unwrap(),
            php_function_name("main")
        );
        let output = php_execution::Script::new(&script)
            .command(&[])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(expected.exit_status),
            "{output:?}"
        );
        assert_eq!(output.stdout, expected.stdout);
        assert_eq!(output.stderr, expected.stderr);
    }
}

#[test]
fn primitive_comparisons_reuse_helpers_and_evaluate_operands_once() {
    let source = r#"
class Failure implements Error { function __construct(string $message) {} }
function order<T implements Comparable<T>>(T $left, T $right): Ordering {
    return $left->compare($right);
}
function label(Ordering $value): string {
    return match ($value) {
        Ordering::Less => "less", Ordering::Equal => "equal", Ordering::Greater => "greater",
    };
}
function left(): int { echo "L"; return -2; }
function right(): int { echo "R"; return 3; }
function fail(): int throws Failure { echo "F"; throw new Failure("expected"); }
function main(): void {
    uint64 $high = 18446744073709551615;
    uint64 $low = 1;
    echo label(order(left(), right())) . "|";
    echo label(order(3, 3)) . "|";
    echo label(order(3, -2)) . "|";
    echo label(order("10", "2")) . "|";
    echo label(order($high, $low)) . "|";
    echo label(order($low, $high)) . "|";
    try { order(fail(), right()); } catch (Failure $error) { echo $error->message; }
    echo "\n";
}
"#;
    let program = doriac::lower_source_to_mir("primitive-comparisons.doria", source).unwrap();
    let expected = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(
        expected.stdout,
        b"LRless|equal|greater|less|greater|less|Fexpected\n"
    );
    let php = doriac::compile_source_to_php("primitive-comparisons.doria", source).unwrap();
    for helper in ["__doria_compare_primitive", "__doria_compare_uint64"] {
        assert_eq!(php.matches(&format!("function {helper}(")).count(), 1);
        assert!(php.matches(&format!("{helper}(")).count() > 1);
    }
    assert!(!php.contains("static function ($left, $right) { return $left === $right ?"));
    let script = format!(
        "{}\n{}();",
        php.strip_prefix("<?php").unwrap(),
        php_function_name("main")
    );
    let output = php_execution::Script::new(&script)
        .command(&[])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(expected.exit_status),
        "{output:?}"
    );
    assert_eq!(output.stdout, expected.stdout);
    assert_eq!(output.stderr, expected.stderr);
}

#[test]
fn runtime_generated_time_and_cancellation_errors_use_the_checked_carrier() {
    let artifact = doriac::compile_source(
        "runtime-error-carriers.doria",
        r#"
use Doria\Std\Concurrency\{CancellationSource, CancellationError};
use Doria\Std\Time\{Deadline, Duration, ClockError};
function main(): void throws CancellationError, ClockError {
    let writable $source = new CancellationSource();
    $source->cancel();
    let $deadline = Deadline::fromNow(Duration::fromNanoseconds(1));
}
"#,
        BackendTarget::Php,
    )
    .unwrap();
    let BackendOutput::TextWithRuntime { contents, .. } = artifact else {
        panic!("clock and cancellation require the shared native runtime");
    };
    for error in [
        doriac::compiler_known_concurrency::CANCELLATION_ERROR,
        doriac::compiler_known_time::CLOCK_ERROR,
    ] {
        let encoded: String = error
            .as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert!(contents.contains(&format!("__doria_throw(new __DoriaQualified_{encoded}(")));
    }
    assert!(!contents.contains("throw new __DoriaQualified_"));
}

#[test]
fn php_executes_concrete_nominal_contracts_without_erasure() {
    let name = "concrete";
    let source = include_str!("../../../examples/native/main_stage35_concrete_conformance.doria");
    let expected = "report\n";
    let php = doriac::compile_source_to_php(name, source).unwrap();
    assert!(!php.contains("interface Renderable"));
    let Ok(version) = Command::new("php").arg("--version").output() else {
        eprintln!("PHP unavailable; execution skipped for {name}");
        return;
    };
    assert!(version.status.success());
    let script = format!(
        "{}\n{}();",
        php.strip_prefix("<?php").unwrap(),
        php_function_name("main")
    );
    let output = php_execution::Script::new(&script)
        .command(&[])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{name}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(output.stdout, expected.as_bytes());
}

fn php_function_name(name: &str) -> String {
    let encoded = name
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("__DoriaFunction_{encoded}")
}

#[test]
fn php_shared_payload_borrows_keep_the_control_block_home() {
    let source = r#"
interface Named { function getName(): string; }
class Name implements Named {
    function __construct(string $name) {}
    function getName(): string { return $this->name; }
    function __destruct(): void { try { echo "drop;"; } catch (Error) {} }
}
function identity(Named $value): Named { return $value; }
function main(): void {
    SharedReference<Named> $owner = shared new Name("value");
    let $view = identity($owner->referencedValue);
    echo $view->getName();
}
"#;
    let php = doriac::compile_source_to_php("shared-payload-home.doria", source).unwrap();
    assert!(php.contains("payloadCell()"));
    assert!(!php.contains("$receiver->referencedValue"));
    assert_php_main_execution(&php, b"valuedrop;", 0);
}

fn assert_php_operation(php: &str, value: &str) {
    // Inspect the actual executable operation inside its diagnostic scope;
    // do not strip frames or rewrite generated code to the pre-metadata shape.
    assert!(
        php.contains(&format!(
            "try {{ return {value}; }} finally {{ __doria_operation_leave("
        )) || php.lines().any(|line| {
            line.contains(" = __doria_operation_enter(")
                && line.contains(&format!(" = {value};"))
                && line.contains("finally { __doria_operation_leave(")
        }),
        "missing scoped operation {value:?} in:\n{php}"
    );
}

fn php_operation_results<'a>(php: &'a str, value: &str) -> Vec<&'a str> {
    assert_php_operation(php, value);
    let assignment = format!(" = {value};");
    php.lines()
        .filter_map(|line| {
            let (_, body) = line.split_once("try { ")?;
            let (name, _) = body.split_once(&assignment)?;
            name.starts_with("$__doria_operation_value").then_some(name)
        })
        .collect()
}

fn php_staged_operands<'a>(php: &'a str, value: &str) -> Vec<&'a str> {
    let assignment = format!(" = {value};");
    php.lines()
        .map(str::trim)
        .filter_map(|line| {
            line.strip_suffix(&assignment)
                .filter(|name| name.starts_with("$__doria_expression_result"))
        })
        .collect()
}

fn assert_php_staged_numeric_update(php: &str, place: &str, op: &str, right: &str) {
    let previous_assignment = format!(" = {place};");
    let previous = php
        .lines()
        .map(str::trim)
        .find_map(|line| {
            line.strip_suffix(&previous_assignment)
                .filter(|name| name.starts_with("$__doria_numeric_previous"))
        })
        .expect("numeric update must snapshot its previous value");
    let right_assignment = format!(" = {right};");
    let staged = php
        .lines()
        .map(str::trim)
        .find_map(|line| {
            line.strip_suffix(&right_assignment)
                .filter(|name| name.starts_with("$__doria_statement_value"))
        })
        .expect("numeric update must stage its right operand");
    let commit = format!(
        "__doria_numeric_assign({place}, __doria_float_operation({op:?}, {previous}, {staged}));"
    );
    let read = php
        .find(&format!("{previous}{previous_assignment}"))
        .unwrap();
    let evaluate = php.find(&format!("{staged}{right_assignment}")).unwrap();
    let commit = php
        .find(&commit)
        .expect("numeric update must commit its staged operands");
    assert!(read < evaluate && evaluate < commit, "{php}");
}

fn assert_php_display_write(php: &str, value: &str) {
    assert_php_operation(php, &format!("__doria_display({value})"));
    let assignment = format!(" = __doria_display({value});");
    let staged = php
        .lines()
        .find_map(|line| {
            let (_, body) = line.split_once("try { ")?;
            let (name, _) = body.split_once(&assignment)?;
            name.starts_with("$__doria_operation_value").then_some(name)
        })
        .expect("display conversion must be staged in its statement region");
    assert!(
        php.contains(&format!("__doria_write_stdout({staged},")),
        "missing output of staged display value {staged}"
    );
}

fn assert_php_statement_operation_prefix(php: &str, value: &str) {
    assert!(
        php.lines().any(|line| {
            line.contains(" = __doria_operation_enter(")
                && line.contains("try { $__doria_operation_value")
                && line.contains(&format!(" = {value}"))
                && line.contains("finally { __doria_operation_leave(")
        }),
        "missing statement operation starting with {value:?}"
    );
}

fn assert_php_execution(php: &str, invocation: &str, stdout: &[u8], status: i32) {
    let script = format!("{}\n{invocation}", php.strip_prefix("<?php").unwrap());
    let output = php_execution::Script::new(&script)
        .command(&["-d", "ffi.enable=0"])
        .env_remove("DORIA_RUNTIME_OUTCOME_V2")
        .output()
        .expect("PHP is required for compatibility execution coverage");
    assert_eq!(output.status.code(), Some(status), "{output:?}");
    assert_eq!(output.stdout, stdout, "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}

fn assert_php_main_execution(php: &str, stdout: &[u8], status: i32) {
    assert_php_execution(
        php,
        &format!("exit({}() ?? 0);", php_function_name("main")),
        stdout,
        status,
    );
}

fn php_function_body<'a>(php: &'a str, name: &str) -> &'a str {
    let start = php
        .find(&format!("function {}(", php_function_name(name)))
        .unwrap();
    let function = &php[start..];
    // Top-level closing braces are unindented; nested blocks are not.
    &function[..function.find("\n}\n").unwrap() + 3]
}

#[test]
fn php_statement_evaluation_preserves_field_result_setup_and_checked_failures() {
    let source = r#"
enum Packet { case Number(int $value); case Empty; }
class Failure implements Error { string $message = "failed"; }
function produce(int $value): Packet throws Failure {
    if ($value < 0) { throw new Failure(); }
    return Packet::Number($value);
}
function forward(int $value): Packet throws Failure { return produce($value); }
function select(): int throws Failure {
    let $result = forward(7);
    return match ($result) { Packet::Number($value) => $value, Packet::Empty => -1, };
}
function main(): void {
    try {
        echo select();
        let $saved = forward(8);
        try { let $failed = forward(-1); } catch (Failure) { echo "c"; }
        echo match ($saved) { Packet::Number($value) => $value, Packet::Empty => -1, };
    } catch (Failure) { echo "unexpected"; }
}
"#;
    let program = doriac::lower_source_to_mir("statement-field-results.doria", source).unwrap();
    let interpreted = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(interpreted.stdout, b"7c8");
    assert_eq!(interpreted.exit_status, 0);
    let php = doriac::compile_source_to_php("statement-field-results.doria", source).unwrap();
    let body = php_function_body(&php, "select");
    let clear = body.find("__DoriaValueResult::clear();").unwrap();
    let evaluation = body.find(" = __doria_operation_enter(").unwrap();
    let capture = body.find(" = __DoriaValueResult::$field0;").unwrap();
    assert!(clear < evaluation && evaluation < capture, "{body}");
    assert_php_main_execution(&php, b"7c8", 0);
}

#[test]
fn php_statement_conditions_preserve_lazy_gates_and_loop_continuations() {
    let source = r#"
function probe(int $event, bool $accepted): bool { echo "{$event},"; return $accepted; }
function main(): void {
    given { let $ready = probe(1, false); $ready; probe(2, true); }
    if (probe(3, true)) { echo "wrong"; }
    else if (probe(4, true)) { echo "wrong"; }
    else { echo "5;"; }
    given { let writable $count = 0; probe(6, $count < 2); }
    while (probe(7, true)) { echo "8;"; $count++; continue; }
    for (let writable $index = 0; $index < 3; $index++) {
        if (probe(9, $index == 1)) { continue; }
        echo "{$index};";
    }
}
"#;
    let expected = b"1,5;6,7,8;6,7,8;6,9,0;9,9,2;";
    let program = doriac::lower_source_to_mir("statement-conditions.doria", source).unwrap();
    let interpreted = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(interpreted.stdout, expected);
    assert_eq!(interpreted.exit_status, 0);
    let php = doriac::compile_source_to_php("statement-conditions.doria", source).unwrap();
    assert_php_main_execution(&php, expected, 0);
}

#[test]
fn php_statement_returned_borrows_preserve_writable_collection_homes() {
    let source = r#"
function lend(writable List<int> $items): List<int> { return $items; }
function forward(writable List<int> $items): List<int> { return lend($items); }
function main(): void {
    writable List<int> $owner = [1];
    let writable $view = forward($owner);
    $view[0] = 7;
    $view->add(2);
    echo $owner[0] . ":" . $owner[1];
}
"#;
    let program = doriac::lower_source_to_mir("statement-borrow-home.doria", source).unwrap();
    let interpreted = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(interpreted.stdout, b"7:2");
    assert_eq!(interpreted.exit_status, 0);
    let php = doriac::compile_source_to_php("statement-borrow-home.doria", source).unwrap();
    let body = php_function_body(&php, "forward");
    assert!(!body.contains("(function()"), "{body}");
    assert_php_main_execution(&php, b"7:2", 0);
}

#[test]
fn php_generic_closures_share_callable_identities_with_inline_copy_types() {
    let source = r#"
use Doria\Std\Time\Duration;
use Doria\Std\Io\Wait;
class Sample {
    Duration $value = (fn(Duration $input) => $input)(Duration::fromNanoseconds(-17));
}
function callback<T>(T $input): function(T): T { return fn(T $value) => $value; }
function copy<T>(T $input): T {
    let $callback = callback($input);
    return $callback($input);
}
function replace(writable int $input, int $other): int { $input = $other; return $input; }
function copyParameter(int $input): int { return replace($input, 7); }
function main(): void {
    let $duration = Duration::fromNanoseconds(-17);
    let $sample = new Sample();
    if ($sample->value != $duration) { panic("initializer callback changed"); }
    let $copyDuration = callback($duration);
    if ($copyDuration($duration) != $duration) { panic("duration changed"); }
    let $copyInt = callback(42);
    if ($copyInt(42) != 42) { panic("integer changed"); }
    if (copy($duration) != $duration) { panic("nested duration changed"); }
    if (copy(42) != 42) { panic("nested integer changed"); }
    let $wait = Wait::For($duration);
    if (copy($wait) != $wait) { panic("nested enum changed"); }
    if (copyParameter(42) != 7) { panic("local parameter storage changed"); }
}
"#;
    let php = doriac::compile_source_to_php("generic-copy-callback.doria", source).unwrap();
    let script = format!(
        "{}\n{}();",
        php.strip_prefix("<?php").unwrap(),
        php_function_name("main")
    );
    let output = php_execution::Script::new(&script)
        .command(&["-d", "ffi.enable=0"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn php_list_remove_at_transfers_ownership_from_every_stable_collection_place() {
    for (declaration, receiver) in [
        (
            "writable List<Item> $items = [new Item(1), new Item(2)];",
            "$items",
        ),
        ("let writable $holder = new Holder();", "$holder->items"),
        ("let writable $nested = new Nested();", "$nested->items[0]"),
        ("", "makeItems()"),
        ("", "(new Holder())->items"),
    ] {
        let source = format!(
            r#"
class Item {{
    function __construct(int $value) {{}}
    function __destruct(): void {{
        try {{ echo "drop {{$this->value}};"; }} catch (Doria\Std\Io\IoError) {{}}
    }}
}}
class Holder {{ writable List<Item> $items = [new Item(1), new Item(2)]; }}
class Nested {{ writable List<List<Item>> $items = [[new Item(1), new Item(2)]]; }}
function makeItems(): List<Item> {{ return [new Item(1), new Item(2)]; }}
function position(): int throws Doria\Std\Io\IoError {{ echo "index;"; return 0; }}
function main(): void throws Doria\Std\Io\IoError {{
    {declaration}
    {{
        let $removed = {receiver}->removeAt(position());
        echo "removed {{$removed->value}};";
    }}
    echo "leave;";
}}
"#
        );
        let php = doriac::compile_source_to_php("remove-at.doria", &source)
            .unwrap_or_else(|diagnostics| panic!("{receiver}: {diagnostics:?}"));
        let script = format!(
            "{}\n{}();",
            php.strip_prefix("<?php").unwrap(),
            php_function_name("main")
        );
        let output = php_execution::Script::new(&script)
            .command(&[])
            .output()
            .unwrap();
        assert!(output.status.success(), "{receiver}: {output:?}");
        assert!(output.stderr.is_empty(), "{receiver}: {output:?}");
        let expected = if declaration.is_empty() {
            "index;drop 2;removed 1;drop 1;leave;"
        } else {
            "index;removed 1;drop 1;leave;drop 2;"
        };
        assert_eq!(output.stdout, expected.as_bytes(), "{receiver}");
    }
}

#[test]
fn php_nullable_collection_removals_transfer_owned_values_from_stable_places() {
    for (ty, values, operation, removed) in [
        ("List<Item>", "[new Item(1), new Item(2)]", "pop()", 2),
        (
            "Dictionary<int, Item>",
            "[0 => new Item(1), 1 => new Item(2)]",
            "remove(position())",
            1,
        ),
    ] {
        for (declaration, receiver) in [
            (format!("writable {ty} $items = {values};"), "$items"),
            (
                "let writable $holder = new Holder();".into(),
                "$holder->items",
            ),
            (
                "let writable $nested = new Nested();".into(),
                "$nested->items[0]",
            ),
            (String::new(), "makeItems()"),
            (String::new(), "(new Holder())->items"),
        ] {
            let source = format!(
                r#"
class Item {{
    function __construct(int $value) {{}}
    function __destruct(): void {{
        try {{ echo "drop {{$this->value}};"; }} catch (Doria\Std\Io\IoError) {{}}
    }}
}}
class Holder {{ writable {ty} $items = {values}; }}
class Nested {{ writable List<{ty}> $items = [{values}]; }}
function makeItems(): {ty} {{ return {values}; }}
function position(): int throws Doria\Std\Io\IoError {{ echo "index;"; return 0; }}
function main(): void throws Doria\Std\Io\IoError {{
    {declaration}
    {{
        let $removed = {receiver}->{operation};
        if ($removed != null) {{ echo "removed {{$removed->value}};"; }}
        else {{ panic("present element was lost"); }}
    }}
    echo "leave;";
}}
"#
            );
            let php = doriac::compile_source_to_php("nullable-removal.doria", &source)
                .unwrap_or_else(|errors| panic!("{receiver}->{operation}: {errors:?}"));
            let script = format!(
                "{}\n{}();",
                php.strip_prefix("<?php").unwrap(),
                php_function_name("main")
            );
            let output = php_execution::Script::new(&script)
                .command(&["-d", "ffi.enable=0"])
                .output()
                .unwrap();
            assert!(
                output.status.success() && output.stderr.is_empty(),
                "{receiver}->{operation}: {output:?}"
            );
            let prefix = if removed == 1 { "index;" } else { "" };
            let other = 3 - removed;
            let expected = if declaration.is_empty() {
                format!("{prefix}drop {other};removed {removed};drop {removed};leave;")
            } else {
                format!("{prefix}removed {removed};drop {removed};leave;drop {other};")
            };
            assert_eq!(
                output.stdout,
                expected.as_bytes(),
                "{receiver}->{operation}"
            );
        }
    }
}

#[test]
fn php_list_remove_at_preserves_readonly_receiver_rejections() {
    for receiver in ["borrowItems($items)", "(new ReadonlyHolder())->items"] {
        let source = format!(
            "class ReadonlyHolder {{ List<int> $items = [7]; }} \
             function borrowItems(List<int> $items): List<int> {{ return $items; }} \
             function main(): void {{ writable List<int> $items = [7]; {receiver}->removeAt(0); }}"
        );
        let diagnostics = doriac::compile_source_to_php("remove-at-readonly.doria", &source)
            .expect_err("a readonly receiver must not become writable during PHP lowering");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0201"),
            "{receiver}: {diagnostics:?}"
        );
    }
}

#[test]
fn php_list_remove_at_checks_bounds_before_splicing() {
    for index in [-1, 1] {
        let source = format!("function main(): void {{ writable List<int> $items = [7]; $items->removeAt({index}); }}");
        let php = doriac::compile_source_to_php("remove-at-bounds.doria", &source).unwrap();
        let script = format!(
            "{}\n{}();",
            php.strip_prefix("<?php").unwrap(),
            php_function_name("main")
        );
        let output = php_execution::Script::new(&script)
            .command(&[])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(101), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("Panic[P1310]"),
            "{output:?}"
        );
    }
}

#[test]
fn php_io_transfer_failures_distinguish_unknown_from_exact_prefix_without_rollback() {
    let php = doriac::compile_source_to_php(
        "php-progress.doria",
        r#"
function main(): void {
    try { write_file("data", "AéBC"); } catch (Doria\Std\Io\IoError) {}
}
"#,
    )
    .unwrap();
    let script = format!(
        "{}\n{}",
        php.strip_prefix("<?php").unwrap(),
        r#"
class ProgressWriteStream {
    public $context;
    public static int $limit = 0;
    public static bool $unknown = false;
    public static string $written = "";
    public function stream_open($path, $mode, $options, &$openedPath): bool { return true; }
    public function stream_write($value): int|false {
        $remaining = self::$limit - strlen(self::$written);
        if ($remaining > 0) {
            $count = min($remaining, strlen($value));
            self::$written .= substr($value, 0, $count);
            return $count;
        }
        trigger_error("transfer failed errno=28", E_USER_WARNING);
        return self::$unknown ? false : 0;
    }
    public function stream_eof(): bool { return false; }
    public function stream_stat(): array { return []; }
}
stream_wrapper_register("progress", ProgressWriteStream::class);
foreach ([false, true] as $unknown) {
    foreach ([0, 3] as $limit) {
        ProgressWriteStream::$limit = $limit;
        ProgressWriteStream::$unknown = $unknown;
        ProgressWriteStream::$written = "";
        try {
            __doria_write_file_mode("progress://file", "AéBC", false, 0, 0, "main");
            echo "unexpected success;";
        } catch (__DoriaCheckedError $caught) {
            $error = $caught->error();
            echo $error->transferredByteCount === null ? "unknown:" : $error->transferredByteCount . ":";
            echo bin2hex(ProgressWriteStream::$written) . ";";
            $caught->dropError();
        }
    }
}
"#
    );
    let Ok(output) = php_execution::Script::new(&script).command(&[]).output() else {
        eprintln!("PHP unavailable; exact/unknown progress execution skipped");
        return;
    };
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(output.stdout, b"0:;3:41c3a9;unknown:;unknown:41c3a9;");
}

#[test]
fn php_failed_read_does_not_promote_a_known_prefix_to_exact_unknown_transfer_progress() {
    let php = doriac::compile_source_to_php(
        "php-read-progress.doria",
        r#"
function main(): void {
    try { read_file("data"); } catch (Error) {}
}
"#,
    )
    .unwrap();
    let script = format!(
        "{}\n{}",
        php.strip_prefix("<?php").unwrap(),
        r#"
class ProgressReadStream {
    public $context;
    public static int $calls = 0;
    public function stream_open($path, $mode, $options, &$openedPath): bool { return true; }
    public function stream_eof(): bool { return false; }
    public function stream_read($count): string|false {
        if (self::$calls++ === 0) { return "ABC"; }
        trigger_error("read failed errno=28", E_USER_WARNING);
        return false;
    }
    public function stream_close(): void {
        trigger_error("close failed errno=13", E_USER_WARNING);
    }
    public function stream_stat(): array { return []; }
}
stream_wrapper_register("progressread", ProgressReadStream::class);
try {
    __doria_read_file("progressread://file", 0, 0, "main");
    echo "unexpected success";
} catch (__DoriaCheckedError $caught) {
    $error = $caught->error();
    echo ($error->transferredByteCount === null ? "unknown:" : "incorrect:") .
        $error->systemCode . ":" . ProgressReadStream::$calls;
    $caught->dropError();
}
"#
    );
    let Ok(output) = php_execution::Script::new(&script).command(&[]).output() else {
        eprintln!("PHP unavailable; failed-read unknown progress execution skipped");
        return;
    };
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(output.stdout, b"unknown:28:2");
}

#[test]
fn php_file_broken_pipe_is_a_checked_transfer_failure_not_clean_standard_output_exit() {
    let php = doriac::compile_source_to_php(
        "php-file-pipe.doria",
        r#"
function main(): void {
    try { write_file("data", "x"); } catch (Doria\Std\Io\IoError) {}
}
"#,
    )
    .unwrap();
    let script = format!(
        "{}\n{}",
        php.strip_prefix("<?php").unwrap(),
        r#"
class BrokenFileStream {
    public $context;
    public function stream_open($path, $mode, $options, &$openedPath): bool { return true; }
    public function stream_write($value): int {
        trigger_error("Broken pipe errno=32", E_USER_WARNING);
        return 0;
    }
    public function stream_stat(): array { return []; }
}
stream_wrapper_register("brokenfile", BrokenFileStream::class);
try {
    __doria_write_file_mode("brokenfile://file", "x", false, 0, 0, "main");
    echo "unexpected success";
} catch (__DoriaCheckedError $caught) {
    echo "caught:" . $caught->error()->transferredByteCount;
    $caught->dropError();
}
"#
    );
    let Ok(output) = php_execution::Script::new(&script).command(&[]).output() else {
        eprintln!("PHP unavailable; file broken-pipe execution skipped");
        return;
    };
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(output.stdout, b"caught:0");
}

#[test]
fn php_io_progress_outcome_v5_preserves_nullable_signed_counts_and_v3_compatibility() {
    static NEXT_OUTCOME: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    for (literal, expected) in [
        ("null", None),
        ("0", Some(0)),
        ("3", Some(3)),
        ("-3", Some(-3)),
        ("9223372036854775807", Some(i64::MAX)),
    ] {
        let source = format!(
            r#"
function fail(): void throws Doria\Std\Io\IoError {{
    throw new Doria\Std\Io\IoError("failed", Doria\Std\Io\IoOperation::Write,
        Doria\Std\Io\IoTarget::StandardOutput, Doria\Std\Io\IoErrorReason::Other,
        transferredByteCount: {literal});
}}
function main(): void throws Doria\Std\Io\IoError {{
    try {{ fail(); }} catch (Doria\Std\Io\IoError $error) {{ throw $error; }}
}}"#
        );
        let php = doriac::compile_source_to_php("php-outcome-progress.doria", source).unwrap();
        let script = format!(
            "{}\n{}();",
            php.strip_prefix("<?php").unwrap(),
            php_function_name("main")
        );
        for version in [3, 5] {
            let path = std::env::temp_dir().join(format!(
                "doria-php-progress-{}-{}",
                std::process::id(),
                NEXT_OUTCOME.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            let execution = php_execution::Script::new(&script);
            let mut command = execution.command(&[]);
            command
                .env_remove("DORIA_RUNTIME_OUTCOME_V3")
                .env_remove("DORIA_RUNTIME_OUTCOME_V5")
                .env(format!("DORIA_RUNTIME_OUTCOME_V{version}"), &path);
            if version == 5 {
                // A new consumer may advertise both channels; V5 must win.
                command.env("DORIA_RUNTIME_OUTCOME_V3", &path);
            }
            let Ok(output) = command.output() else {
                eprintln!("PHP unavailable; nullable signed progress transport skipped");
                return;
            };
            assert_eq!(output.status.code(), Some(70), "{output:?}");
            assert!(output.stdout.is_empty(), "{output:?}");
            assert!(output.stderr.is_empty(), "{output:?}");
            let record = std::fs::read(&path).unwrap();
            std::fs::remove_file(&path).unwrap();
            assert_eq!(&record[..8], format!("DORIAO{version}\0").as_bytes());
            assert_eq!(
                u16::from_le_bytes(record[8..10].try_into().unwrap()),
                version
            );
            let text_start = if version == 5 {
                assert_eq!(record[51], u8::from(expected.is_some()));
                assert_eq!(
                    i64::from_le_bytes(record[52..60].try_into().unwrap()),
                    expected.unwrap_or(0)
                );
                60
            } else {
                51
            };
            let type_len = u32::from_le_bytes(record[10..14].try_into().unwrap()) as usize;
            assert_eq!(
                &record[text_start..text_start + type_len],
                b"Doria\\Std\\Io\\IoError"
            );
        }
        let Ok(output) = php_execution::Script::new(&script)
            .command(&[])
            .env_remove("DORIA_RUNTIME_OUTCOME_V3")
            .env_remove("DORIA_RUNTIME_OUTCOME_V5")
            .output()
        else {
            return;
        };
        assert_eq!(output.status.code(), Some(70));
        let stderr = String::from_utf8(output.stderr).unwrap();
        if let Some(count) = expected {
            assert!(
                stderr.contains(&format!("Transferred Byte Count\n  {count}")),
                "{stderr}"
            );
        } else {
            assert!(!stderr.contains("Transferred Byte Count"));
        }
    }
}

fn php_type_name(name: &str) -> String {
    let encoded = name
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("__DoriaType_{encoded}")
}

#[test]
fn php_preserves_temporary_returned_borrow_statement_cleanup() {
    // Isolate returned-borrow cleanup; php_integer_execution_tests covers
    // checked arithmetic and its statement-temporary lifetime independently.
    let source = r#"
class Guard {
    function __construct(int $value) { echo "new{$value};"; }
    function __destruct() { try { echo "drop{$this->value};"; } catch (Error) {} }
}
class Failure implements Error { function __construct(string $message) {} }
function observe(Guard $value): void { echo "see{$value->value};"; }
function observeNullable(?Guard $value): void {
    if ($value == null) { echo "absent;"; }
    else { echo "see{$value->value};"; }
}
function main(): void {
    let $identity = function (Guard $value): Guard { return $value; };
    observe($identity(new Guard(1)));
    echo "after1;";
    let $nullable = function (?Guard $value): ?Guard { return $value; };
    observeNullable($nullable(new Guard(2)));
    echo "after2;";
    observeNullable($nullable(null));
    let $checked = function (Guard $value, bool $fail): Guard {
        if ($fail) { throw new Failure("failed"); }
        return $value;
    };
    try { observe($checked(new Guard(3), false)); }
    catch (Failure) { echo "unexpected;"; }
    echo "after3;";
    try { observe($checked(new Guard(4), true)); }
    catch (Failure) { echo "caught;"; }
    echo "after4;";
}
"#;
    let expected = "new1;see1;drop1;after1;new2;see2;drop2;after2;absent;new3;see3;drop3;after3;new4;drop4;caught;after4;";
    let php = doriac::compile_source_to_php("temporary-returned-borrows.doria", source)
        .expect("temporary returned borrows should compile to PHP");
    let script = format!(
        "{}\n{}();",
        php.strip_prefix("<?php").unwrap(),
        php_function_name("main")
    );
    let Ok(output) = php_execution::Script::new(&script).command(&[]).output() else {
        eprintln!("PHP unavailable; temporary returned borrow execution skipped");
        return;
    };
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(output.stdout, expected.as_bytes());
}

#[test]
fn php_backend_isolates_nominal_types_from_host_names_and_case_folding() {
    let source = r#"
open class Parent {
    open string $label = "parent" { get => $this->label; }
}
class Child extends Parent {
    override string $label = "child" { get => $this->label; }
}
class Exception { function __construct(string $label) {} }
class DateTime { function __construct(string $label) {} }
class Label { function __construct(string $text) {} }
class LABEL { function __construct(string $text) {} }
enum Match { case Ready; }
enum MATCH { case Waiting; }
enum Switch { case Value(string $text); }
class Key implements Comparable<Key>, Cloneable {
    function __construct(string $text) {}
    function compare(Key $other): Ordering {
        if ($this->text < $other->text) { return Ordering::Less; }
        if ($this->text > $other->text) { return Ordering::Greater; }
        return Ordering::Equal;
    }
    function clone(): self { return new Key($this->text); }
}
function order<T implements Comparable<T>>(T $left, T $right): Ordering {
    return $left->compare($right);
}
function label(Parent $value): string { return $value->label; }
function main(): void {
    let $child = new Child();
    let $exception = new Exception("exception");
    let $date = new DateTime("date");
    let $first = new Label("first");
    let $second = new LABEL("second");
    let $state = Match::Ready;
    let $other = MATCH::Waiting;
    let $payload = Switch::Value("payload");
    echo "{label($child)} {$exception->label} {$date->label} {$first->text} {$second->text} ";
    echo match ($state) { Match::Ready => "ready " };
    echo match ($other) { MATCH::Waiting => "waiting " };
    echo match ($payload) { Switch::Value($text) => $text };
    echo " " . (order("a", "b") == Ordering::Less) . " ";
    SortedSet<Key> $keys = SortedSet::from([new Key("b"), new Key("a")]);
    foreach ($keys as Key $key) { echo $key->text; }
    let $identity = fn(Match $value) => $value;
    let $nullable = fn(?Match $value) => $value;
    echo " " . ($identity(Match::Ready) == Match::Ready);
    echo " " . ($nullable(null) == null);
    echo match (order("a", "b")) {
        Ordering::Less => " less",
        Ordering::Equal => " equal",
        Ordering::Greater => " greater",
    };
}
"#;
    let php = doriac::compile_source_to_php("host-type-collisions.doria", source).unwrap();
    assert!(php.contains(&format!("class {}", php_type_name("Parent"))));
    assert!(php.contains(&format!("extends {}", php_type_name("Parent"))));
    let script = format!(
        "{}\n{}();",
        php.strip_prefix("<?php").unwrap(),
        php_function_name("main")
    );
    let Ok(output) = php_execution::Script::new(&script).command(&[]).output() else {
        eprintln!("PHP unavailable; host type-name execution skipped");
        return;
    };
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        output.stdout,
        b"child exception date first second ready waiting payload true ab true true less"
    );
}

#[test]
fn php_preserves_optimizer_fixture_ownership_and_cleanup() {
    for (name, source, expected) in [
        (
            "stack classes",
            include_str!("../../../examples/native/main_stage35a_stack_classes.doria"),
            include_str!("fixtures/native_io/main_stage35a_stack_classes/expected_stdout"),
        ),
        (
            "transferred closure",
            include_str!("../../../examples/native/main_stage35a_transferred_closure.doria"),
            include_str!("fixtures/native_io/main_stage35a_transferred_closure/expected_stdout"),
        ),
    ] {
        let php = doriac::compile_source_to_php(name, source).unwrap();
        let script = format!(
            "{}\n{}();",
            php.strip_prefix("<?php").unwrap(),
            php_function_name("main")
        );
        let output = php_execution::Script::new(&script)
            .command(&[])
            .output()
            .expect("PHP fixture runner");
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty(), "{name}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            expected,
            "{name}"
        );
    }
}

#[test]
fn unhandled_error_cleanup_does_not_depend_on_php_shutdown() {
    let php = doriac::compile_source_to_php(
        "unhandled-cleanup.doria",
        r#"
class Failure implements Error {
    function __construct(string $message) {}
    function __destruct(): void { try { echo "drop error\n"; } catch (Error) {} }
}
function main(): void throws Failure { throw new Failure("expected failure"); }
"#,
    )
    .unwrap();
    let output = php_execution::Script::new(&format!(
        "{}\n{}();",
        php.strip_prefix("<?php").unwrap(),
        php_function_name("main")
    ))
    .command(&[])
    .output()
    .unwrap();
    assert_eq!(output.status.code(), Some(70), "{output:?}");
    assert_eq!(output.stdout, b"drop error\n");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Error[R1000]"),
        "{output:?}"
    );
}

#[test]
fn stage32_attributes_are_metadata_only_in_php_output() {
    let source = include_str!("../../../examples/native/main_stage32_attributes.doria");
    let php = doriac::compile_source_to_php("main_stage32_attributes.doria", source)
        .expect("attributed source should compile to PHP");
    for forbidden in [
        "#[",
        "PHPExport",
        "attribute registry",
        "test registry",
        "new Route(",
    ] {
        assert!(
            !php.contains(forbidden),
            "unexpected PHP output: {forbidden}"
        );
    }

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\n{}();",
        php.strip_prefix("<?php").expect("generated PHP header"),
        php_function_name("main")
    );
    let run = php_execution::Script::new(&script)
        .command(&["-d", "display_errors=1"])
        .output()
        .expect("attributed PHP should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"Stage 32 metadata only\n");
}

#[test]
fn emits_php_for_simple_program() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    let writable $count = 0;
    $count = 1;
    echo $count;
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.starts_with("<?php"));
    assert!(php.contains("$count = 0;"));
    assert!(php.contains("$count = 1;"));
    assert_php_display_write(&php, "$count");
    assert_php_main_execution(&php, b"1", 0);
}

#[test]
fn php_backend_executes_writable_null_safe_property_calls() {
    let source =
        include_str!("../../../examples/native/main_stage22_writable_nullable_property.doria");
    let php = doriac::compile_source_to_php("writable-null-safe-property.doria", source)
        .expect("the PHP backend should lower writable null-safe property calls");

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\n{}();",
        php.strip_prefix("<?php").expect("generated PHP header"),
        php_function_name("main")
    );
    let run = php_execution::Script::new(&script)
        .command(&["-d", "display_errors=1"])
        .output()
        .expect("generated null-safe property PHP should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        run.stdout,
        include_bytes!(
            "fixtures/native_io/main_stage22_writable_nullable_property/expected_stdout"
        )
    );
}

#[test]
fn php_backend_emits_native_unit_and_backed_enums() {
    let php = doriac::compile_source_to_php(
        "stage27.doria",
        r#"
enum Status { case Draft; case Published; }
enum Priority: int { case Low = 1; case High = 10; }
enum Transport: string { case Road = "road"; case Rail = "rail"; }
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    Status $status = Status::Draft;
    Priority $priority = Priority::High;
    Transport $transport = Transport::Rail;
    echo $status == Status::Draft;
    echo " {$priority->value} {$transport->value}\n";
}
"#,
    )
    .expect("unit and backed enums should lower to PHP native enums");

    assert!(php.contains(&format!("enum {}", php_type_name("Status"))));
    assert!(php.contains("case Draft;"));
    assert!(php.contains(&format!("enum {}: int", php_type_name("Priority"))));
    assert!(php.contains("case High = 10;"));
    assert!(php.contains(&format!("enum {}: string", php_type_name("Transport"))));
    assert!(php.contains("case Rail = 'rail';") || php.contains("case Rail = \"rail\";"));

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&["-d", "display_errors=1"])
        .output()
        .expect("generated enum PHP should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"true 10 rail\n");
}

#[test]
fn php_backend_executes_core_match_payloads_narrowing_conditions_and_ternary() {
    let php = doriac::compile_source_to_php(
        "stage28.doria",
        r#"
enum Delivery { case Waiting; case Sent(string $reference); }
class Note { function __construct(string $text) {} }
enum NoteResult { case Found(Note $note); case Missing; }
function condition(string $name, bool $value): bool throws Doria\Std\Io\IoError { echo $name; return $value; }
function describe(Delivery $delivery): string throws Doria\Std\Io\IoError
{
    return match ($delivery) {
        Delivery::Waiting => "waiting",
        Delivery::Sent($reference) if condition("g", false) => "wrong",
        Delivery::Sent($reference) => "sent {$reference}",
    };
}
function consume(): string
{
    NoteResult $result = NoteResult::Found(new Note("owned"));
    return match (take $result) {
        NoteResult::Found($note) if $note->text == "skip" => "wrong",
        NoteResult::Found($note) => $note->text,
        NoteResult::Missing => "missing",
    };
}
function inspect(mixed $value): string
{
    return match ($value) {
        bool $flag => "bool {$flag}",
        string $text => "string {$text}",
        default => "other",
    };
}
function choose(): string throws Doria\Std\Io\IoError
{
    return match (true) {
        condition("a", false) => "A",
        condition("b", true) => "B",
        condition("c", true) => "C",
        default => "D",
    };
}
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    echo describe(Delivery::Sent("R-12")) . "\n";
    echo inspect(true) . " " . inspect("text") . "\n";
    echo choose() . "\n";
    echo false ? "wrong" : true ? "ready" : "wrong";
    echo " " . consume();
}
"#,
    )
    .expect("checked Stage 28 match should lower through the PHP compatibility backend");

    assert!(php.contains("__doriaMatchesCase"));
    assert!(php.contains("__doriaPayloadAt"));
    assert!(php.contains("__doria_mixed_is("));
    assert!(!php.contains("get_debug_type("));
    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&["-d", "display_errors=1"])
        .output()
        .expect("generated match PHP should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        run.stdout,
        b"gsent R-12\nbool true string text\nabB\nready owned"
    );
}

#[test]
fn php_backend_preserves_exact_doria_types_after_mixed() {
    let php = doriac::compile_source_to_php(
        "stage28-exact-mixed.doria",
        r#"
enum UnitState { case Ready; }
enum BackedState: string { case Ready = "ready"; }
enum PayloadState { case Ready(int $code); }
class Document {}

function inspect(mixed $value): string
{
    return match ($value) {
        int8 $item => "int8",
        int16 $item => "int16",
        int32 $item => "int32",
        int $item => "int",
        uint8 $item => "uint8",
        uint16 $item => "uint16",
        uint32 $item => "uint32",
        uint64 $item => "uint64",
        float32 $item => "float32",
        float $item => "float",
        bool $item => "bool",
        string $item => "string",
        UnitState $item => "unit",
        BackedState $item => "backed",
        PayloadState $item => "payload",
        Document $item => "class",
        default => "other",
    };
}

function makeInt(): mixed
{
    return 7;
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    int8 $int8Value = 1;
    int16 $int16Value = 1;
    int32 $int32Value = 1;
    int $intValue = 1;
    uint8 $uint8Value = 1;
    uint16 $uint16Value = 1;
    uint32 $uint32Value = 1;
    uint64 $uint64Value = 1;
    float32 $float32Value = 1.25;
    float $floatValue = 1.25;
    echo inspect($int8Value) . " " . inspect($int16Value) . " " . inspect($int32Value) . " " . inspect($intValue) . "\n";
    echo inspect($uint8Value) . " " . inspect($uint16Value) . " " . inspect($uint32Value) . " " . inspect($uint64Value) . "\n";
    echo inspect($float32Value) . " " . inspect($floatValue) . " " . inspect(true) . " " . inspect("text") . "\n";
    echo inspect(UnitState::Ready) . " " . inspect(BackedState::Ready) . " " . inspect(PayloadState::Ready(7)) . " " . inspect(new Document()) . "\n";
    echo inspect(makeInt());
}
"#,
    )
    .expect("PHP mixed values must retain exact Doria type tags");

    assert!(!php.contains("get_debug_type("));
    assert!(php.contains("__doria_box_mixed("));
    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&["-d", "display_errors=1"])
        .output()
        .expect("generated exact-type PHP should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        run.stdout,
        b"int8 int16 int32 int\nuint8 uint16 uint32 uint64\nfloat32 float bool string\nunit backed payload class\nint"
    );
}

#[test]
fn php_backend_executes_payload_enum_construction_equality_and_storage() {
    let php = doriac::compile_source_to_php(
        "stage27-payload.doria",
        r#"
enum Coordinate
{
    case Origin;
    case Point(int $x, int $y);
}

class Drawing
{
    function __construct(Coordinate $coordinate)
    {
    }
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    Coordinate $point = Coordinate::Point(y: 22, x: 20);
    Coordinate $copy = $point;
    ?Coordinate $nullable = $copy;
    List<Coordinate> $items = [Coordinate::Origin, $copy];
    let $drawing = new Drawing($point);

    echo $copy == Coordinate::Point(20, 22);
    echo " {$nullable == $point}";
    echo " {$drawing->coordinate == $point}\n";
}
"#,
    )
    .expect("payload enum compatibility lowering should compile");

    assert!(php.contains(&format!(
        "final class {} implements __DoriaValueEquatable",
        php_type_name("Coordinate")
    )));
    assert!(!php.contains(&format!("enum {}", php_type_name("Coordinate"))));

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&["-d", "display_errors=1"])
        .output()
        .expect("generated payload enum PHP should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"true true true\n");
}

#[test]
fn php_backend_distinguishes_omitted_payload_defaults_from_explicit_null() {
    let php = doriac::compile_source_to_php(
        "nullable-payload-default.doria",
        r#"
enum Coordinate
{
    case Origin;
    case Point(int $x, int $y);
}

function describe(?Coordinate $value = Coordinate::Origin): string
{
    if ($value == null) { return "null"; }
    if ($value == Coordinate::Origin) { return "origin"; }
    return "point";
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    echo describe(null);
    echo " ";
    echo describe();
}
"#,
    )
    .expect("nullable Copy payload defaults should lower to PHP");

    assert!(php.contains(&format!(
        "{}|array|null $value = []",
        php_type_name("Coordinate")
    )));
    assert!(php.contains("if ($value === [])"));
    assert!(!php.contains("$value ??="));

    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&[])
        .output()
        .expect("PHP should execute nullable payload defaults");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"null origin");
}

#[test]
fn php_backend_initializes_internal_static_payloads_in_declaring_class_scope() {
    let php = doriac::compile_source_to_php(
        "internal-static-payload.doria",
        r#"
enum Label
{
    case Empty;
    case Text(string $value);
}

class Vault
{
    internal static Label $label = Label::Text("secret");

    static function reveal(): Label
    {
        return self::label;
    }
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    echo Vault::reveal() == Label::Text("secret");
}
"#,
    )
    .expect("internal static payload initializers should lower to PHP");

    assert!(php.contains(&format!(
        "private static {} $label;",
        php_type_name("Label")
    )));
    assert!(php.contains("(\\Closure::bind(static function (): void {"));
    assert!(php.contains(&format!(
        "self::$label = {}::Text(\"secret\");",
        php_type_name("Label")
    )));
    assert!(!php.contains(&format!("{}::$label =", php_type_name("Vault"))));

    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&[])
        .output()
        .expect("PHP should execute internal static payload initialization");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"true");
}

#[test]
fn php_grouped_declarations_use_one_collision_safe_temporary_in_order() {
    let php = doriac::compile_source_to_php(
        "stage26a.doria",
        r#"
function value(): string throws Doria\Std\Io\IoError { echo "once\n"; return "shared"; }
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let $__doria_grouped_value0 = "user";
    let $first, $second, $third = value();
    echo "{$first}:{$second}:{$third}:{$__doria_grouped_value0}\n";
}
"#,
    )
    .expect("PHP should lower grouped Copy declarations");

    assert!(php.contains("$__doria_grouped_value0 = \"user\";"));
    let value_call = format!("{}()", php_function_name("value"));
    assert_php_operation(&php, &value_call);
    assert_eq!(php.matches(&format!(" = {value_call};")).count(), 1);
    let initializer = php
        .lines()
        .find(|line| {
            line.trim().starts_with("$__doria_grouped_value")
                && line.contains(" = $__doria_operation_value")
        })
        .expect("grouped initializer temporary");
    let (temporary, _) = initializer.trim().split_once(" = ").unwrap();
    assert_ne!(temporary, "$__doria_grouped_value0");
    let first = php.find(&format!("$first = {temporary};")).unwrap();
    let second = php.find(&format!("$second = {temporary};")).unwrap();
    let third = php.find(&format!("$third = {temporary};")).unwrap();
    let cleanup = php.find(&format!("unset({temporary});")).unwrap();
    assert!(first < second && second < third && third < cleanup);
    assert!(!php.contains("$first = $second ="));

    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&[])
        .output()
        .expect("PHP should execute grouped declarations");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"once\nshared:shared:shared:user\n");
}

#[test]
fn php_backend_emits_folded_copy_scalar_parameter_defaults() {
    let php = doriac::compile_source_to_php(
        "folded-defaults.doria",
        r#"
function sample(int $count = 1 + 2, float $ratio = 1.0 / 2.0, bool $ordered = 1 < 2): void
{
}
"#,
    )
    .expect("const-evaluable Copy-scalar defaults should lower to PHP literals");

    assert!(php.contains(&format!(
        "function {}(int $count = 3, float $ratio = 0.5, bool $ordered = true): void",
        php_function_name("sample")
    )));
    assert!(!php.contains("$count = 1 + 2"));
    assert!(!php.contains("$ratio = fdiv("));
    assert!(!php.contains("$ordered = __doria_less("));
}

#[test]
fn php_backend_normalizes_defaulted_cell_parameters_without_leaking_transport() {
    let source = include_str!("fixtures/defaulted_copy_parameter_homes.doria");
    let mir = doriac::lower_source_to_mir("defaulted-cell-parameters.doria", source).unwrap();
    let expected = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert!(expected.runtime_diagnostic.is_none(), "{expected:?}");
    assert_eq!(expected.stdout, b"true:false:true:false:true\n");
    let php = doriac::compile_source_to_php("defaulted-cell-parameters.doria", source)
        .expect("defaulted cell parameters should lower to PHP");

    assert!(php.contains(&format!(
        "function {}(bool $value = false): bool",
        php_function_name("invert")
    )));
    assert!(php.contains("public bool $value;"));
    assert!(php.contains("function __construct(bool $value = false)"));
    assert!(php.contains("$value = new __DoriaCell($value);"));
    assert!(php.contains("$this->value = $value->value;"));

    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&[])
        .output()
        .expect("PHP should execute defaulted and explicit cell arguments");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, expected.stdout);
    for profile in [NativeProfile::Fast]
        .into_iter()
        .chain(cfg!(feature = "llvm-backend").then_some(NativeProfile::Release))
    {
        let actual = native_execution::execute_native(&mir, profile);
        assert_eq!(
            actual.status.code(),
            Some(expected.exit_status),
            "{actual:?}"
        );
        assert_eq!(actual.stdout, expected.stdout, "{profile:?}");
        assert_eq!(actual.stderr, expected.stderr, "{profile:?}");
    }
}

#[test]
fn emits_php_for_boolean_word_operators() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    echo true and false;
    echo false or true;
    echo not false;
    echo true xor false;
}
"#,
    )
    .expect("compilation should succeed");

    assert_php_operation(&php, "((true) && (false))");
    assert_php_operation(&php, "((false) || (true))");
    assert_php_operation(&php, "!(false)");
    assert_php_operation(&php, "((true) !== (false))");
    assert_php_main_execution(&php, b"falsetruetruetrue", 0);
}

#[test]
fn parenthesizes_logical_operands_for_php() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    echo true and null ?? true;
    echo false or null ?? true;
}
"#,
    )
    .expect("compilation should succeed");

    let results = php_operation_results(&php, "null ?? true");
    assert_eq!(results.len(), 2);
    for (index, literal, operator, gate) in [(0, "true", "&&", "{}"), (1, "false", "||", "!({})")] {
        let left = php_staged_operands(&php, literal);
        let right = php_staged_operands(&php, results[index]);
        assert_eq!(left.len(), 1);
        assert_eq!(right.len(), 1);
        assert_php_operation(&php, &format!("(({}) {operator} ({}))", left[0], right[0]));
        assert!(php.contains(&format!("if ({}) {{", gate.replace("{}", left[0]))));
    }
    assert!(!php_function_body(&php, "main").contains("(function("));
    assert!(!php.contains("true && null ?? true"));
    assert!(!php.contains("false || null ?? true"));
    assert_php_main_execution(&php, b"truetrue", 0);
}

#[test]
fn parenthesizes_xor_operands_for_php() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    echo true == true xor false;
    echo false xor true != false;
}
"#,
    )
    .expect("compilation should succeed");

    let equal = php_operation_results(&php, "true === true");
    let unequal = php_operation_results(&php, "true !== false");
    assert_eq!(equal.len(), 1);
    assert_eq!(unequal.len(), 1);
    let left = php_staged_operands(&php, equal[0]);
    let right = php_staged_operands(&php, unequal[0]);
    let false_values = php_staged_operands(&php, "false");
    assert_eq!(left.len(), 1);
    assert_eq!(right.len(), 1);
    assert_eq!(false_values.len(), 2);
    assert_php_operation(&php, &format!("(({}) !== ({}))", left[0], false_values[0]));
    assert_php_operation(&php, &format!("(({}) !== ({}))", false_values[1], right[0]));
    assert!(!php_function_body(&php, "main").contains("(function("));
    assert!(!php.contains("true === true !== false"));
    assert!(!php.contains("false !== true !== false"));
    assert_php_main_execution(&php, b"truetrue", 0);
}

#[test]
fn emits_typed_php_comparisons() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    echo "01" == "1";
    echo "01" != "1";
}
"#,
    )
    .expect("compilation should succeed");

    assert_php_operation(&php, "\"01\" === \"1\"");
    assert_php_operation(&php, "\"01\" !== \"1\"");
    assert!(!php.contains("echo \"01\" == \"1\";"));
    assert!(!php.contains("echo \"01\" != \"1\";"));
    assert_php_main_execution(&php, b"falsetrue", 0);
}

#[test]
fn php_backend_preserves_byte_lexicographic_string_ordering() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): int throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    if ("10" < "2") {
        return 42;
    }
    return 0;
}
"#,
    )
    .expect("PHP should preserve Doria string ordering");

    assert!(php.contains("__doria_less(\"10\", \"2\")"));
    assert!(php.contains("strcmp($left, $right)"));

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\nexit(__DoriaFunction_6d61696e());",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&[])
        .output()
        .expect("PHP should execute generated output");
    assert_eq!(run.status.code(), Some(42));
}

#[test]
fn php_backend_keeps_exact_int64_alias_and_signed_comparison_subset() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function isLess(int64 $left, int $right): bool
{
    return $left < $right;
}

function identity(int64 $value): int64
{
    return $value;
}
"#,
    )
    .expect("the exact signed integer subset should remain supported by PHP");

    assert!(php.contains(&format!(
        "function {}(int $left, int $right): bool",
        php_function_name("isLess")
    )));
    assert_php_operation(&php, "__doria_less($left, $right)");
    assert!(php_function_body(&php, "isLess").contains("return $__doria_operation_value"));
    assert!(php.contains(&format!(
        "function {}(int $value): int",
        php_function_name("identity")
    )));
}

#[test]
fn php_backend_rejects_stage_13_integer_shapes_it_cannot_preserve() {
    let cases = [
        (
            "integer division",
            r#"
function divide(int $left, int $right): int
{
    return $left / $right;
}
"#,
            "Doria integer division semantics for `/`",
        ),
        (
            "integer shift",
            r#"
function shift(int $value, int $count): int
{
    return $value << $count;
}
"#,
            "Doria integer shift semantics for `<<`",
        ),
        (
            "fixed-width bitwise",
            r#"
function mask(int $left, int $right): int
{
    return $left & $right;
}
"#,
            "fixed-width Doria bitwise semantics for `&`",
        ),
        (
            "checked conversion",
            r#"
function convert(): void
{
    let $value = Int8::from(1);
}
"#,
            "checked Doria integer conversion semantics for `Int8::from(...)`",
        ),
    ];

    for (name, source, expected) in cases {
        let diagnostics = match doriac::compile_source_to_php("test.doria", source) {
            Ok(php) => panic!("{name} unexpectedly generated PHP:\n{php}"),
            Err(diagnostics) => diagnostics,
        };

        assert_eq!(diagnostics[0].code, "B1301", "{name}: {diagnostics:?}");
        assert!(
            diagnostics[0]
                .message
                .contains("PHP compatibility backend cannot preserve"),
            "{name}: {diagnostics:?}"
        );
        assert!(
            diagnostics[0].message.contains(expected),
            "{name}: {diagnostics:?}"
        );
    }
}

#[test]
fn php_capability_failure_does_not_make_valid_doria_fail_check() {
    let source = r#"
function divide(int $left, int $right): int
{
    return $left / $right;
}
"#;

    doriac::check_source("test.doria", source)
        .expect("PHP compatibility limitations must not affect Doria checking");

    let diagnostics = doriac::compile_source_to_php("test.doria", source)
        .expect_err("PHP generation must reject integer division rather than emit PHP `/`");
    assert_eq!(diagnostics[0].code, "B1301");
    assert!(diagnostics[0].message.contains("integer division"));
}

#[test]
fn php_backend_maps_float64_and_allows_float_arithmetic() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function total(): float64
{
    writable float $value = 1.5 + 2.5;
    $value += 1.0;
    return $value;
}
"#,
    )
    .expect("PHP should preserve default float arithmetic");

    assert!(php.contains(&format!("function {}(): float", php_function_name("total"))));
    let body = php_function_body(&php, "total");
    assert!(body.contains("$value = $__doria_operation_value"));
    assert_php_operation(body, "__doria_float_operation(\"+\", 1.5, 2.5)");
    assert_php_staged_numeric_update(body, "$value", "+", "1.0");
    assert!(!php.contains("float64"));
    assert_php_execution(
        &php,
        &format!("exit({}() === 5.0 ? 0 : 1);", php_function_name("total")),
        b"",
        0,
    );
}

#[test]
fn php_backend_rejects_noncanonical_float_display() {
    for source in [
        "function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { echo 10000000000.0 * 10000000000.0; }",
        "function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { echo \"value=\" . 1.5; }",
        "function show(float $value): void throws Doria\\Std\\Io\\IoError { echo \"value={$value}\"; } function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error {}",
    ] {
        let diagnostics = doriac::compile_source_to_php("test.doria", source)
            .expect_err("PHP must reject float display it cannot preserve canonically");
        assert_eq!(diagnostics[0].code, "B1301");
        assert!(diagnostics[0]
            .message
            .contains("canonical float display formatting"));
    }
}

#[test]
fn php_backend_uses_fdiv_and_rejects_inexact_cross_kind_conversions() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function divide(float $left, float64 $right): float
{
    writable float $value = $left / $right;
    $value /= 2.0;
    return $value;
}
"#,
    )
    .expect("PHP should lower IEEE float64 division through fdiv");
    let body = php_function_body(&php, "divide");
    assert!(body.contains("fdiv($left, $right)"), "{body}");
    assert_php_staged_numeric_update(body, "$value", "/", "2.0");
    assert!(!php.contains(" / "), "{php}");
    assert!(!php.contains("/="), "{php}");
    assert!(!php.contains("float64"), "{php}");

    let divide = php_function_name("divide");
    assert_php_execution(
        &php,
        &format!(
            "exit({divide}(5.0, 2.0) === 1.25 && {divide}(1.0, 0.0) === INF && {divide}(-1.0, 0.0) === -INF && is_nan({divide}(0.0, 0.0)) ? 0 : 1);"
        ),
        b"",
        0,
    );

    for source in [
        "function main(): int throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { return Float::toInt(42.0); }",
        "function helper(): float { return Int::toFloat(42); } function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error {}",
    ] {
        let diagnostics = doriac::compile_source_to_php("test.doria", source)
            .expect_err("PHP must reject conversions it cannot prove exact");
        assert_eq!(diagnostics[0].code, "B1301");
        assert!(diagnostics[0].message.contains("conversion semantics"));
    }
}

#[test]
fn php_backend_accepts_float32_storage_but_rejects_unrounded_arithmetic() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function identity(float32 $value): float32
{
    return $value;
}
"#,
    )
    .expect("PHP can carry an already-quantized float32 value without arithmetic");
    assert!(php.contains(&format!(
        "function {}(float $value): float",
        php_function_name("identity")
    )));

    let diagnostics = doriac::compile_source_to_php(
        "test.doria",
        "function add(float32 $left, float32 $right): float32 { return $left + $right; }",
    )
    .expect_err("PHP must reject float32 operations without binary32 rounding");
    assert_eq!(diagnostics[0].code, "B1301");
    assert!(diagnostics[0].message.contains("binary32 rounding"));
}

#[test]
fn php_backend_constant_folds_exact_tests_on_concrete_numeric_types() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        "function narrow(int $value): bool { return $value is int8; } function exact(int8 $value): bool { return $value is int8; }",
    )
    .expect("known concrete Doria types do not need PHP host-type inference");
    for (name, value) in [("narrow", "false"), ("exact", "true")] {
        let body = php_function_body(&php, name);
        assert!(body.contains("(int $value): bool\n{"));
        assert!(body.contains(&format!("return ({value});")));
        assert!(!body.contains("__doria_mixed_is("), "{body}");
    }
    assert_php_execution(
        &php,
        &format!(
            "echo {}(42) ? 'true' : 'false'; echo {}(42) ? 'true' : 'false';",
            php_function_name("narrow"),
            php_function_name("exact")
        ),
        b"falsetrue",
        0,
    );
}

#[test]
fn php_backend_parenthesizes_type_tests_as_expression_atoms() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function test(mixed $value): bool { return $value is int == true; }
function main(): void { echo test(1); echo test("x"); }
"#,
    )
    .expect("PHP should preserve nested type-test precedence");

    let body = php_function_body(&php, "test");
    let left = php_staged_operands(body, "(__doria_mixed_is($value, \"int\"))");
    let right = php_staged_operands(body, "true");
    assert_eq!(left.len(), 1);
    assert_eq!(right.len(), 1);
    assert_php_operation(body, &format!("{} === {}", left[0], right[0]));
    assert!(php_function_body(&php, "test").contains("return $__doria_operation_value"));
    assert_php_main_execution(&php, b"truefalse", 0);
}

#[test]
fn php_backend_allows_negative_integer_literals_and_checks_runtime_negation() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function negativeOne(): int
{
    return -1;
}

function minimum(): int
{
    return -9223372036854775808;
}
"#,
    )
    .expect("in-range signed integer literals should lower to PHP");

    assert_php_operation(&php, "-(1)");
    assert!(php_function_body(&php, "negativeOne").contains("return $__doria_operation_value"));
    assert_php_operation(&php, "(-9223372036854775807 - 1)");
    assert!(php_function_body(&php, "minimum").contains("return $__doria_operation_value"));
    assert_php_execution(
        &php,
        &format!(
            "echo {}(), '|', {}();",
            php_function_name("negativeOne"),
            php_function_name("minimum")
        ),
        b"-1|-9223372036854775808",
        0,
    );

    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function negate(int $value): int
{
    return -$value;
}
"#,
    )
    .expect("runtime negation must use Doria's checked arithmetic");
    assert!(php.contains("__doria_integer_operation(\"neg\", $value, 0, 64, true,"));
}

#[test]
fn parenthesizes_unary_not_operands_for_php() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    echo not (1 < 2);
}
"#,
    )
    .expect("compilation should succeed");

    let results = php_operation_results(&php, "__doria_less(1, 2)");
    assert_eq!(results.len(), 1);
    let operand = php_staged_operands(&php, &format!("({})", results[0]));
    assert_eq!(operand.len(), 1);
    assert_php_operation(&php, &format!("!({})", operand[0]));
    assert!(!php_function_body(&php, "main").contains("(function("));
    assert!(!php.contains("echo !1 < 2;"));
    assert_php_main_execution(&php, b"false", 0);
}

#[test]
fn php_backend_preserves_main_string_local_echo() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let $message = "Hello Doria!";
    echo $message;
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("$message = \"Hello Doria!\";"));
    assert_php_display_write(&php, "$message");
    assert_php_main_execution(&php, b"Hello Doria!", 0);
}

#[test]
fn php_backend_preserves_main_string_concat_echo() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let $name = "Doria";
    echo "Hello " . $name . "!";
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("$name = \"Doria\";"));
    assert!(php.contains("__doria_display($name)"));
}

#[test]
fn php_backend_preserves_main_string_concat_local_initializer() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let $name = "Doria";
    let $message = "Hello " . $name . "!";
    echo $message;
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("$message = $__doria_operation_value"));
    assert_php_operation(&php, "__doria_display($name)");
    assert_php_display_write(&php, "$message");
    assert_php_main_execution(&php, b"Hello Doria!", 0);
}

#[test]
fn emits_php_for_stage_10_integer_helper_function_call() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function identity(int $value): int
{
    return $value;
}

function main(): int throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    return identity(42);
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains(&format!(
        "function {}(int $value): int",
        php_function_name("identity")
    )));
    assert!(php.contains("return $value;"));
    assert!(php.contains(&format!("function {}(): int", php_function_name("main"))));
    assert_php_operation(&php, &format!("{}(42)", php_function_name("identity")));
    assert!(php_function_body(&php, "main").contains("return $__doria_operation_value"));
}

#[test]
fn php_backend_mangles_free_functions_away_from_the_host_global_namespace() {
    let php = doriac::compile_source_to_php(
        "host-function-collision.doria",
        r#"
function log(string $message): void
{
    echo "{$message}\n";
}

function main(): void
{
    log("Hello");
}
"#,
    )
    .expect("Doria function names may overlap PHP built-ins");

    assert!(php.contains("function __DoriaFunction_6c6f67(string $message): void"));
    assert!(php.contains("__DoriaFunction_6c6f67(\"Hello\")"));
    assert!(!php.contains("function log("));

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&["-d", "display_errors=1"])
        .output()
        .expect("generated PHP should execute without redeclaring PHP's log function");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"Hello\n");
}

#[test]
fn emits_php_for_bool_helper_condition() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function isAnswer(int $value): bool
{
    return $value == 42;
}

function main(): int throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    if (isAnswer(42)) {
        return 42;
    }

    return 0;
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains(&format!(
        "function {}(int $value): bool",
        php_function_name("isAnswer")
    )));
    assert!(php_function_body(&php, "main").contains("if ($__doria_expression_result"));
    assert_php_operation(&php, &format!("{}(42)", php_function_name("isAnswer")));
    assert_php_main_execution(&php, b"", 42);
}

#[test]
fn emits_php_for_string_helper_echo() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function greet(string $name): void throws Doria\Std\Io\IoError
{
    echo "Hello " . $name . "!";
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    greet("Doria");
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains(&format!(
        "function {}(string $name): void",
        php_function_name("greet")
    )));
    assert!(php.contains("__doria_display($name)"));
    assert!(php.contains(&format!("{}(\"Doria\");", php_function_name("greet"))));
}

#[test]
fn emits_php_for_stage_10_void_helper_call() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function hello(): void throws Doria\Std\Io\IoError
{
    echo "Hello Doria!";
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    hello();
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains(&format!("function {}(): void", php_function_name("hello"))));
    assert_php_display_write(&php, "\"Hello Doria!\"");
    assert!(php.contains(&format!("function {}(): void", php_function_name("main"))));
    assert!(php.contains(&format!("{}();", php_function_name("hello"))));
    assert_php_main_execution(&php, b"Hello Doria!", 0);
}

#[test]
fn lowers_checked_program_to_hir() {
    let lowered = doriac::lower_source(
        "test.doria",
        r#"
let $name = "Doria";
let $copy = $name;
"#,
    )
    .expect("lowering should succeed");

    assert!(matches!(
        &lowered.items[0],
        hir::Item::Statement(hir::Stmt::VarDecl(decl))
            if decl.bindings.len() == 1 && decl.bindings[0].name == "name"
    ));
}

#[test]
fn lowers_control_flow_to_hir() {
    let lowered = doriac::lower_source(
        "test.doria",
        r#"
let writable $count = 0;
if ($count < 10) {
    let $label = "small";
} else {
    let $label = "large";
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
    assert!(php.contains("$person->value->name = \"Lucy\";"));
    assert!(!php.contains("($count) = 1;"));
    assert!(!php.contains("($person->name) = \"Lucy\";"));
}

#[test]
fn emits_php_for_basic_control_flow() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
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
        $count = 10;
    }
}
"#,
    )
    .expect("compilation should succeed");

    let body = php_function_body(&php, "main");
    assert_eq!(body.matches("if ($__doria_expression_result").count(), 2);
    assert!(body.contains("else {\n"));
    assert!(body.contains("while (true) {"));
    assert!(body.contains("if (!$__doria_expression_result"));
    assert_php_operation(&php, "__doria_less($count, 10)");
    assert_php_operation(&php, "__doria_less($count, 20)");
    for value in ["\"small\"", "\"medium\"", "\"large\"", "$count"] {
        assert_php_display_write(&php, value);
    }
    assert!(php.contains("$count = 10;"));
    assert_php_main_execution(&php, b"small0", 0);
}

#[test]
fn php_backend_executes_stage28a_slice1_control_flow() {
    let php = doriac::compile_source_to_php(
        "stage28a.doria",
        r#"
function probe(bool $value): bool throws Doria\Std\Io\IoError
{
    echo $value ? "predicate true\n" : "predicate false\n";
    return $value;
}

function skipped(): bool throws Doria\Std\Io\IoError
{
    echo "condition\n";
    return true;
}

function whenGate(): bool throws Doria\Std\Io\IoError
{
    echo "when gate\n";
    return true;
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    string $label = given {
        let $prepared = "ready";
        false;
    } when (skipped()): string {
        return $prepared;
    } else when (skipped()) {
        return "alternate";
    } else {
        return "fallback {$prepared}";
    };
    echo $label . "\n";

    string $alternate = given {
        whenGate();
    } when (false): string {
        return "wrong";
    } else when (true) {
        return "alternate";
    } else {
        return "fallback";
    };
    echo $alternate . "\n";

    given {
        let writable $running = true;
        probe($running);
    } while ($running) {
        echo "body\n";
        $running = false;
        continue;
    }

    do {
        echo "once\n";
    } while (false);
}
"#,
    )
    .expect("Stage 28a Slice 1 control flow should lower to PHP");

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&["-d", "display_errors=1"])
        .output()
        .expect("generated Stage 28a PHP should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        run.stdout,
        b"fallback ready\nwhen gate\nalternate\npredicate true\nbody\npredicate false\nonce\n"
    );
}

#[test]
fn php_backend_executes_stage28a_finalizers() {
    let php = doriac::compile_source_to_php(
        "stage28a-finalizers.doria",
        r#"
function record(string $message): void
{
    try {
        echo $message;
    } catch (Doria\Std\Io\IoError) {
    }
}

function returnThroughFinalizer(): int throws Doria\Std\Io\IoError
{
    if (true) {
        return 42;
    } finally {
        record("return cleanup\n");
    }

    return 0;
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    given {
        let $prepared = "prepared";
    } if (true) {
        echo "if {$prepared}\n";
    } finally {
        record("if cleanup {$prepared}\n");
    }

    string $selected = when (true): string {
        return "selected";
    } else {
        return "wrong";
    } finally {
        record("when cleanup\n");
    };
    echo "{$selected}\n";

    let writable $count = 0;
    while ($count < 3) {
        if ($count == 0) {
            $count = 1;
            continue;
        }
        $count = 2;
        break;
    } finally {
        record("while cleanup {$count}\n");
    }

    do {
        echo "do body\n";
    } while (false) finally {
        record("do cleanup\n");
    }

    if (true) {
        if (true) {
            echo "nested body\n";
        } finally {
            record("inner cleanup\n");
        }
    } finally {
        record("outer cleanup\n");
    }

    echo "return {returnThroughFinalizer()}\n";
}
"#,
    )
    .expect("Stage 28a finalizers should lower to PHP");

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&["-d", "display_errors=1"])
        .output()
        .expect("generated Stage 28a finalizer PHP should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        run.stdout,
        b"if prepared\nif cleanup prepared\nwhen cleanup\nselected\nwhile cleanup 2\ndo body\ndo cleanup\nnested body\ninner cleanup\nouter cleanup\nreturn cleanup\nreturn 42\n"
    );

    let panic_php = doriac::compile_source_to_php(
        "stage28a-panic-finalizers.doria",
        r#"
function record(string $message): void
{
    try {
        echo $message;
    } catch (Doria\Std\Io\IoError) {
    }
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    if (true) {
        if (true) {
            panic("stop");
        } finally {
            record("wrong inner\n");
        }
    } finally {
        record("wrong outer\n");
    }
}
"#,
    )
    .expect("fatal panic inside finalizers should lower to PHP");
    let panic_script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        panic_php
            .strip_prefix("<?php")
            .expect("generated PHP header")
    );
    let panic_run = php_execution::Script::new(&panic_script)
        .command(&["-d", "display_errors=1"])
        .output()
        .expect("generated panic finalizer PHP should execute");
    assert_eq!(panic_run.status.code(), Some(101));
    assert!(panic_run.stdout.is_empty());
    assert!(String::from_utf8_lossy(&panic_run.stderr).contains("stop"));
}

#[test]
fn php_fatal_panic_bypasses_every_stage28a_finalizer_entry() {
    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }

    for (name, source, expected_stdout) in [
        (
            "if-condition",
            include_str!("../../../examples/native/main_finally_panic_if_condition.doria"),
            &b"before\n"[..],
        ),
        (
            "given-setup",
            include_str!("../../../examples/native/main_finally_panic_given_setup.doria"),
            &b"before\n"[..],
        ),
        (
            "given-predicate",
            include_str!("../../../examples/native/main_finally_panic_given_predicate.doria"),
            &b"before\n"[..],
        ),
        (
            "when-branch",
            include_str!("../../../examples/native/main_finally_panic_when_branch.doria"),
            &b"before\n"[..],
        ),
        (
            "when-condition",
            include_str!("../../../examples/native/main_finally_panic_when_condition.doria"),
            &b"before\n"[..],
        ),
        (
            "while-body",
            include_str!("../../../examples/native/main_finally_panic_while_body.doria"),
            &b"before\n"[..],
        ),
        (
            "while-condition",
            include_str!("../../../examples/native/main_finally_panic_while_condition.doria"),
            &b"before\n"[..],
        ),
        (
            "do-body",
            include_str!("../../../examples/native/main_finally_panic_do_body.doria"),
            &b"before\n"[..],
        ),
        (
            "do-condition",
            include_str!("../../../examples/native/main_finally_panic_do_condition.doria"),
            &b"before\n"[..],
        ),
        (
            "inner-finalizer",
            include_str!("../../../examples/native/main_finally_panic_inner_finalizer.doria"),
            &b"before\ninner\n"[..],
        ),
    ] {
        let php = doriac::compile_source_to_php(format!("{name}.doria"), source)
            .unwrap_or_else(|diagnostics| panic!("{name} should lower to PHP: {diagnostics:#?}"));
        let script = format!(
            "{}\n__DoriaFunction_6d61696e();",
            php.strip_prefix("<?php").expect("generated PHP header")
        );
        let run = php_execution::Script::new(&script)
            .command(&["-d", "display_errors=1"])
            .output()
            .unwrap_or_else(|error| panic!("{name} generated PHP should execute: {error}"));

        assert_eq!(run.status.code(), Some(101), "{name}");
        assert_eq!(run.stdout, expected_stdout, "{name}");
        assert!(
            String::from_utf8_lossy(&run.stderr).contains("stop"),
            "{name} should preserve its panic transport"
        );
    }
}

#[test]
fn emits_php_for_loop_control() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let writable $code = 0;

    while ($code < 10) {
        $code = 10;

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
fn emits_php_for_stage_9_range_iteration() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    foreach (0..<10 as int $i) {
        echo "x";
    }

    foreach (0..10 as int $i) {
        echo "x";
    }

    foreach ((0..2) as int $k) {
        echo "x";
    }
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("__doria_range_start"));
    assert!(php.contains("; $i__doria"));
    assert!(php.contains(" < $__doria_range_end"));
    assert!(php.contains(" <= $__doria_range_end"));
    assert!(php.matches("__doria_range_start").count() >= 3);
    assert!(!php.contains("unsupported range expression"));
}

#[test]
fn guards_inclusive_php_ranges_before_terminal_increment() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    foreach (9223372036854775807..9223372036854775807 as int $i) {
        echo "x";
    }
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("$__doria_range_done"));
    assert!(php.contains("= false;"));
    assert!(php.contains("!$__doria_range_done"));
    assert!(php.contains("&& $i <= $__doria_range_end"));
    assert!(php.contains("$i < $__doria_range_end"));
    assert!(php.contains("? $i++ : ($__doria_range_done"));
    assert!(php.contains("= true)"));
    assert!(!php.contains("; $i++)"));
}

#[test]
fn rejects_standalone_range_before_php_codegen() {
    let err = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let $range = 0..10;
}
"#,
    )
    .expect_err("semantic checking should reject standalone ranges before PHP codegen");

    assert!(
        err.iter().any(|diagnostic| diagnostic.code == "E0426"),
        "expected E0426, got {err:?}"
    );
}

#[test]
fn emits_void_main_without_exit_wrapper_for_php() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    echo "Hello Doria!";
    return;
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains(&format!("function {}(): void", php_function_name("main"))));
    assert_php_display_write(&php, "\"Hello Doria!\"");
    assert!(php.contains("return;"));
    assert!(!php.contains("exit(__DoriaFunction_6d61696e())"));
    assert_php_main_execution(&php, b"Hello Doria!", 0);
}

#[test]
fn preserves_block_local_bindings_in_php_output() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function greet(string $name): string
{
    if (true) {
        let $name = "inner";
        return $name;
    }

    return $name;
}

function main(): void throws Doria\Std\Io\IoError
{
    let $name = "outer";
    if (true) {
        let $name = $name . " inner";
        echo "block {$name}";
    }
    echo $name;
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("$name = \"outer\";"));
    let shadow = php_function_body(&php, "main")
        .lines()
        .find_map(|line| {
            let (name, value) = line.trim().split_once(" = ")?;
            (name.starts_with("$name__doria") && value.starts_with("$__doria_operation_value"))
                .then_some(name)
        })
        .expect("block-local binding must receive its own staged value");
    assert_php_operation(&php, "__doria_display($name)");
    assert_php_operation(&php, "__doria_display(\" inner\")");
    assert_php_operation(&php, &format!("__doria_display({shadow})"));
    assert_php_display_write(&php, "$name");
    assert!(php.contains(&format!(
        "function {}(string $name): string",
        php_function_name("greet")
    )));
    let inner = php_function_body(&php, "greet")
        .lines()
        .find_map(|line| {
            let (name, _) = line.trim().split_once(" = \"inner\";")?;
            name.starts_with("$name__doria").then_some(name)
        })
        .expect("shadowed parameter must receive a distinct local name");
    assert!(php_function_body(&php, "greet").contains(&format!("return {inner};")));
    assert!(php.contains("return $name;"));
    assert!(!php.contains("$name = $name . \" inner\";"));
    assert_php_execution(
        &php,
        &format!(
            "{}(); echo {}('input');",
            php_function_name("main"),
            php_function_name("greet")
        ),
        b"block outer innerouterinner",
        0,
    );
}

#[test]
fn debug_backend_emits_stage_11_artifact_and_supports_runtime_string_output() {
    let output = doriac::compile_source(
        "test.doria",
        include_str!("../../../examples/debug/main_for_count_10.doria"),
        BackendTarget::Debug,
    )
    .expect("debug backend should emit the Stage 11g artifact");

    let BackendOutput::Text {
        extension,
        contents,
    } = output
    else {
        panic!("debug backend should return text output");
    };
    assert_eq!(extension, "debug");
    assert_eq!(contents, "exit_status: 10\nstdout:\n");

    let output = doriac::compile_source(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    let $name = "Doria";
    echo $name;
}
"#,
        BackendTarget::Debug,
    )
    .expect("debug backend should execute supported runtime string output");
    let BackendOutput::Text {
        extension,
        contents,
    } = output
    else {
        panic!("debug backend should return text output");
    };
    assert_eq!(extension, "debug");
    assert_eq!(contents, "exit_status: 0\nstdout: Doria\n");
}

#[test]
fn php_backend_lowers_panic_to_stderr_and_status_101() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    panic("boom");
}
"#,
    )
    .expect("panic should lower through the compatibility backend");

    assert!(php.contains("__doria_panic(\"P1000\""));
    assert!(php.contains("\"P1000\" => [\"Program Panicked\""));
    assert!(php.contains("__doria_frame_enter("));
    assert!(!php.contains("debug_backtrace("));
    assert!(php.contains("\"\\n\\nCall Path\""));
    assert!(php.contains("\"\\n\\nProcess Exited With Status 101\\n\""));
    assert!(php.contains("exit(101);"));
    let panic_helper = php
        .split("function __doria_panic")
        .nth(1)
        .and_then(|tail| tail.split("function __doria_read_line").next())
        .expect("panic helper should be emitted before I/O helpers");
    assert!(!panic_helper.contains("throw new"));
}

#[test]
fn php_backend_panic_trace_preserves_doria_function_frames() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"function panicNow(): void
{
    panic("boom");
}

function middle(): void
{
    panicNow();
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    middle();
}
"#,
    )
    .expect("panic should lower through the compatibility backend");

    assert!(php.contains("__doria_panic_frames($operation ?? $site)"));
    assert!(php.contains("__doria_operation_enter("));
    assert!(!php.contains("debug_backtrace("));
    assert!(php.contains("\"\\n\" . $name . \" · \""));

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }

    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&[])
        .output()
        .expect("PHP should execute generated output");

    assert_eq!(run.status.code(), Some(101));
    assert!(run.stdout.is_empty());
    let stderr = String::from_utf8(run.stderr).expect("diagnostic must be UTF-8");
    assert!(stderr.starts_with("Panic[P1000]: Program Panicked\n\nWhere\n"));
    assert!(stderr.contains("\n\nWhy\nThe program explicitly raised a fatal panic."));
    assert!(stderr.contains("\n\nNote\nboom"));
    assert!(stderr.contains("\n\nCall Path\npanicNow · test.doria:"));
    assert!(stderr.contains("\nmiddle · test.doria:"));
    assert!(stderr.contains("\nmain · test.doria:"));
    assert!(stderr.ends_with("\n\nProcess Exited With Status 101\n"));
    assert!(!stderr.contains("Stack Trace"));
}

#[test]
fn php_io_panic_trace_preserves_allowed_doria_helper_named_methods() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
class Reader
{
    function __doria_read_file(): void
        throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
    {
        read_file("__doria_missing_stage17_frame_test__/missing.txt");
    }
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let $reader = new Reader();
    $reader->__doria_read_file();
}
"#,
    )
    .expect("allowed helper-named methods should lower through PHP compatibility");

    assert!(php.contains("__doria_frame_enter(\"Reader::__doria_read_file\""));
    assert!(!php.contains("$__doria_function_names"));
    assert!(!php.contains("debug_backtrace("));
    assert!(!php.contains("$helperFunctions"));

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }

    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&[])
        .output()
        .expect("PHP should execute generated output");

    assert_eq!(run.status.code(), Some(70));
    assert!(run.stdout.is_empty());
    let stderr = String::from_utf8(run.stderr).expect("diagnostic must be UTF-8");
    assert!(stderr.starts_with("Error[R1000]: Unhandled Doria\\Std\\Io\\IoError\n\nWhere\n"));
    assert!(stderr.contains(" · Reader::__doria_read_file\n\n"));
    assert!(stderr.contains("read_file(\"__doria_missing_stage17_frame_test__/missing.txt\")"));
    assert!(!stderr.contains("Call Path"));
    assert!(stderr.ends_with("\n\nProcess Exited With Status 70\n"));
    assert!(!stderr.contains("PHP"));
    assert!(!stderr.contains("Stack Trace"));
}

#[test]
fn php_io_warning_fallback_preserves_portable_not_found_reason() {
    let php = doriac::compile_source_to_php(
        "php-io-reason.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    try {
        read_file("__doria_missing_php_reason_test__/missing.txt");
    } catch (Doria\Std\Io\IoError $error) {
        echo $error->reason == Doria\Std\Io\IoErrorReason::NotFound;
        echo " ";
        echo $error->systemCode ?? -1;
    }
}
"#,
    )
    .expect("missing-file reason fixture should lower to PHP");

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&[])
        .output()
        .expect("generated PHP reason fixture should execute");

    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(run.stdout.starts_with(b"true "), "{:?}", run.stdout);
    assert!(
        run.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn php_backend_uses_text_output_shape() {
    let output = doriac::compile_source(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    let $name = "Doria";
    echo $name;
}
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

    function __construct() { $this->name = ""; }

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
    assert!(!php.contains("writable string $name"));
    assert!(!php.contains("writable function rename"));
}

#[test]
fn emits_internal_members_as_private_php_members() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
class Person
{
    internal string $secret;

    function __construct() { $this->secret = ""; }

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
fn omits_constructor_return_type_for_php() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
class Person
{
    function __construct(): void
    {
        return;
    }
}
"#,
    )
    .expect("compilation should succeed");

    assert!(php.contains("public function __construct()"));
    assert!(!php.contains("__construct(): void"));
}

#[test]
fn php_backend_schedules_inherited_destruction_in_doria_order() {
    let php = doriac::compile_source_to_php(
        "inheritance-destruction.doria",
        r#"
open class Base
{
    static writable string $log = "";
    open function value(): int { return 1; }
    function __destruct(): void { Base::log = Base::log . "base "; }
}

class Child extends Base
{
    override function value(): int { return 42; }
    function __destruct(): void { Base::log = Base::log . "child "; return; }
}

function main(): void
{
    {
        Base $value = new Child();
        echo "value {$value->value()}\n";
    }
    echo Base::log . "\n";
}
"#,
    )
    .expect("PHP compatibility output should preserve inherited destruction");

    assert!(php.contains(&format!(
        "class {} extends {}",
        php_type_name("Child"),
        php_type_name("Base")
    )));
    assert!(php.contains("parent::__destruct();"));
    assert!(!php.contains("Reflection"));
    assert!(!php.contains("static::"));
    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&["-d", "display_errors=1"])
        .output()
        .expect("generated inheritance PHP should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"value 42\nchild base \n");
}

#[test]
fn allows_take_on_copy_parameters_in_php() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        "function identity(take int $value): int { return $value; } function main(): int throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { return identity(42); }",
    )
    .expect("take on a Copy value is a semantic no-op");

    assert!(php.contains(&format!(
        "function {}(int $value): int",
        php_function_name("identity")
    )));
    assert!(!php.contains("take int"));
}

#[test]
fn take_on_move_parameters_preserves_explicit_cleanup_in_php() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
class Guard
{
    function __construct(string $name) {}
    function __destruct(): void {
        try { echo "drop {$this->name};"; } catch (Error $error) {}
    }
}
function consume(take Guard $guard): void { echo "consume;"; }
function consumeList(take List<Guard> $guards): void { echo "list;"; }
function consumeMixed(take mixed $guard): void { echo "mixed;"; }
function main(): void
{
    let $guard = new Guard("argument");
    consume($guard);
    let $guards = [new Guard("first"), new Guard("second")];
    consumeList($guards);
    mixed $erased = new Guard("erased");
    consumeMixed($erased);
    echo "done;";
}
"#,
    )
    .expect("checked Move arguments transfer their cleanup responsibility");

    let run = php_execution::Script::new(&format!(
        "{}\n{}();",
        php.strip_prefix("<?php").unwrap(),
        php_function_name("main")
    ))
    .command(&[])
    .output()
    .expect("PHP should execute the ownership regression");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stderr, b"");
    assert_eq!(
        run.stdout,
        b"consume;drop argument;list;drop second;drop first;mixed;drop erased;done;"
    );
}

#[test]
fn php_backend_rejects_shared_ownership_in_every_declared_type_position() {
    for source in [
        "class Node {} function inspect(SharedReference<Node> $node): void {}",
        "class Node {} function make(): SharedReference<Node> { return shared new Node(); }",
        "class Node {} class Box { ?SharedReference<Node> $node = null; }",
        "class Node {} function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { ?SharedReference<Node> $node = null; }",
    ] {
        let diagnostics = doriac::compile_source_to_php("shared-type.doria", source)
            .expect_err("PHP must reject shared ownership at the type boundary");
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.code == "B2301"
                    && diagnostic
                        .message
                        .contains("cannot preserve Doria shared ownership")
            }),
            "expected shared-ownership capability diagnostic, got {diagnostics:?}"
        );
    }
}

#[test]
fn rejects_static_lifecycle_methods_before_php_emission() {
    let err = doriac::compile_source_to_php(
        "test.doria",
        r#"
class Person
{
    static function __construct()
    {
    }
}
"#,
    )
    .expect_err("semantic checking should reject static construction before PHP codegen");

    assert!(err.iter().any(|diagnostic| {
        diagnostic.code == "E0465"
            && diagnostic
                .message
                .contains("invoked by `new` and cannot be `static`")
    }));
}

#[test]
fn rejects_resource_type_before_php_codegen() {
    let err = doriac::compile_source_to_php(
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
    .expect_err("semantic checking should reject resource before PHP codegen");

    assert!(err.iter().any(|diagnostic| {
        diagnostic.code == "E0432"
            && diagnostic
                .message
                .contains("`resource` is reserved for PHP interop")
    }));
}

#[test]
fn rejects_array_callable_name_before_php_codegen() {
    let err = doriac::compile_source_to_php(
        "test.doria",
        r#"
function array(): void
{
}
"#,
    )
    .expect_err("semantic checking should reject array as a callable before PHP codegen");

    assert!(err.iter().any(|diagnostic| {
        diagnostic.code == "E0310" && diagnostic.message.contains("`array`")
    }));
}

#[test]
fn php_backend_uses_valid_callable_hints_for_type_only_function_signatures() {
    let source = r#"
function accept(function(int): int $callback): void
{
}

function main(): void
{
}
"#;

    let php = doriac::compile_source_to_php("type-only-function.doria", source)
        .expect("type-only structural function syntax should remain PHP-lowerable");
    assert!(php.contains(&format!(
        "function {}(__DoriaFunctionValue $callback): void",
        php_function_name("accept")
    )));
    assert!(php.contains("interface __DoriaFunctionValue"));
    assert!(!php.contains(&format!(
        "function {}(callable $callback)",
        php_function_name("accept")
    )));
    assert!(!php.contains(&format!(
        "function {}(function $callback)",
        php_function_name("accept")
    )));
}

#[test]
fn rejects_compiler_helper_function_namespace_before_php_codegen() {
    let err = doriac::compile_source_to_php(
        "test.doria",
        r#"
function __doria_read_line(): void
{
}
"#,
    )
    .expect_err("compiler helper names must be rejected before PHP codegen");

    assert!(err.iter().any(|diagnostic| {
        diagnostic.code == "E0310" && diagnostic.message.contains("`__doria_`")
    }));
}

#[test]
fn lowers_interpolated_string_to_hir() {
    let lowered = doriac::lower_source(
        "test.doria",
        r#"
let $name = "Doria";
let $message = "Hello, {$name}";
"#,
    )
    .expect("lowering should succeed");

    let hir::Item::Statement(hir::Stmt::VarDecl(declaration)) = &lowered.items[1] else {
        panic!("expected interpolated-string declaration");
    };
    let hir::Expr::InterpolatedString { parts, .. } = &declaration.initializer else {
        panic!("expected interpolated string in HIR");
    };

    assert!(matches!(
        &parts[0],
        hir::InterpolatedStringPart::Text { value, span }
            if value == "Hello, " && span.start < span.end
    ));
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
function main(): void throws Doria\Std\Io\IoError
{
    let $name = "Andrew";
    echo "Hello, {$name}!";
}
"#,
    )
    .expect("compilation should succeed");

    assert_php_statement_operation_prefix(&php, "\"Hello, \" . (function()");
    assert_php_operation(&php, "__doria_display($name)");
    assert!(php.contains(" . \"!\"; } finally { __doria_operation_leave("));
    assert!(!php.contains("{$name}"));
    assert_php_main_execution(&php, b"Hello, Andrew!", 0);

    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
class Person
{
    function __construct(string $name)
    {
    }

    function greet(): void throws Doria\Std\Io\IoError
    {
        echo "Hello, {$this->name}";
    }
}
"#,
    )
    .expect("compilation should succeed");

    assert_php_statement_operation_prefix(&php, "\"Hello, \" . (function()");
    assert_php_operation(&php, "__doria_display($this->name)");
    assert!(!php.contains("{$this->name}"));
    assert_php_execution(
        &php,
        &format!("(new {}('Andrew'))->greet();", php_type_name("Person")),
        b"Hello, Andrew",
        0,
    );
}

#[test]
fn php_backend_preserves_control_bytes_in_literal_and_constant_contexts() {
    let payload = "\x000123abcdef\x01\x07\x1f\x7fDória$currency\\literal";
    let source = format!(
        r#"
const VALUE = "{payload}";
class Holder {{ string $value = VALUE; }}
function defaulted(string $value = VALUE): void {{ echo $value; }}
function main(): void {{
    echo "{payload}";
    echo VALUE;
    defaulted();
    let $holder = new Holder();
    echo $holder->value;
    let $number = 7;
    echo "start{}{{$number}}end";
}}
"#,
        '\0',
    );
    let mir = doriac::lower_source_to_mir("control-literals.doria", &source).unwrap();
    let expected = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(expected.exit_status, 0);
    assert!(expected.stderr.is_empty());
    let mut bytes = payload.repeat(4).into_bytes();
    bytes.extend_from_slice(b"start\x007end");
    assert_eq!(expected.stdout, bytes);
    let php = doriac::compile_source_to_php("control-literals.doria", &source).unwrap();
    assert!(
        !php.as_bytes().contains(&0),
        "generated PHP must remain text"
    );
    assert!(php.contains("\\x000123abcdef"));
    let script = format!(
        "{}\n{}();",
        php.strip_prefix("<?php").unwrap(),
        php_function_name("main"),
    );
    let output = php_execution::Script::new(&script)
        .command(&["-d", "ffi.enable=0"])
        .env_remove("DORIA_RUNTIME_OUTCOME_V2")
        .output()
        .expect("PHP is required for exact control-byte serialization parity");
    assert_eq!(
        output.status.code(),
        Some(expected.exit_status),
        "{output:?}"
    );
    assert_eq!(output.stdout, expected.stdout);
    assert_eq!(output.stderr, expected.stderr);
}

#[test]
fn escapes_php_interpolation_markers_in_string_text() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError
{
    let $name = "Andrew";
    let $amount = 10;
    echo "Hello, $name";
    echo 'Literal $name';
    echo "Total: {$amount} ($currency)";
}
"#,
    )
    .expect("compilation should succeed");

    assert_php_display_write(&php, "\"Hello, \\$name\"");
    assert_php_display_write(&php, "\"Literal \\$name\"");
    assert_php_statement_operation_prefix(&php, "\"Total: \" . (function()");
    assert_php_operation(&php, "__doria_display($amount)");
    assert!(php.contains(" . \" (\\$currency)\"; } finally { __doria_operation_leave("));
    assert_php_main_execution(&php, b"Hello, $nameLiteral $nameTotal: 10 ($currency)", 0);
}

#[test]
fn compiles_person_example_with_explicit_interpolation() {
    let example_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/php/person.doria");
    let source = std::fs::read_to_string(&example_path).expect("read person example");
    let php = doriac::compile_source_to_php("examples/php/person.doria", &source)
        .expect("person example should compile");

    assert_php_statement_operation_prefix(&php, "\"Hello, my name is \" . (function()");
    assert_php_operation(&php, "__doria_display($this->name)");
    assert_php_operation(&php, "__doria_display($this->age)");
    assert!(php.contains(" . \" and I am \" . (function()"));
    assert!(php.contains(" . \" years old!\"; } finally { __doria_operation_leave("));
    assert!(!php.contains("{$this->name}"));
    assert!(!php.contains("{$this->age}"));
    assert_php_main_execution(&php, b"Hello, my name is Andrew Masiye and I am 37 years old!\n---\nHello, my name is Lucy and I am 37 years old!", 0);
}

#[test]
fn php_backend_lowers_stage17_io_with_doria_failure_checks() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let writable $line = read_line();
    if ($line != null) { write_stderr($line); }
    let $contents = read_file("input.txt");
    write_file("copy.txt", $contents);
    append_file("copy.txt", $contents);
    printf("enabled=%s", false);
    echo sprintf("%05d", 42);
}
"#,
    )
    .expect("Stage 17 PHP compatibility lowering should succeed");

    assert!(php.contains(
        "function __doria_read_line(string $prompt, int $start, int $end, string $callable): ?string"
    ));
    // The prompt is written exactly and stdout is flushed before stdin is read,
    // without depending on PHP's optional readline extension.
    assert!(php.contains("if ($prompt !== \"\")"));
    assert!(php.contains("__DoriaStdIoIoTarget::__doriaCaseStandardOutput()"));
    assert!(php.contains("__doria_flush_stdout($start, $end, $callable);"));
    assert!(php.contains("if (@fflush(STDOUT)) { return; }"));
    assert!(php.contains("if (__doria_is_broken_pipe(error_get_last())) { exit(0); }"));
    assert!(!php.contains("readline("));
    assert!(php.contains("function __doria_panic("));
    assert!(php.contains("?string $message = null,"));
    assert!(php.contains("?string $callable = null,"));
    let panic_signature = php.split("function __doria_panic(").nth(1).unwrap();
    assert!(!panic_signature
        .split('{')
        .next()
        .unwrap()
        .contains(": never"));
    assert!(php.contains("if ($line === false)"));
    assert!(php.contains("if (feof(STDIN)) { return null; }"));
    assert!(php.contains("__DoriaStdIoIoOperation::Read"));
    assert!(php.contains("new __DoriaStdIoInvalidUtf8Error("));
    assert!(!php.contains("__doria_panic(\"P1403\""));
    assert!(!php.contains("__doria_panic(\"P1404\""));
    assert!(php.contains("str_ends_with($line, \"\\n\")"));
    assert!(php.contains("str_ends_with($line, \"\\r\")"));
    assert!(php.contains("__doria_read_file(\"input.txt\", start:"));
    assert!(php.contains("$file === false"));
    assert!(php.contains("$chunk = @fread($file, 8192)"));
    assert!(php.contains("__doria_write_file(\"copy.txt\", $contents, start:"));
    assert!(php.contains("__doria_append_file(\"copy.txt\", $contents, start:"));
    assert!(php.contains("@fopen($path, $append ? \"ab\" : \"wb\")"));
    assert!(!php.contains("__doria_panic(\"P1402\""));
    assert!(php.contains("while ($offset < $length)"));
    assert!(php.contains("$written === false || $written === 0"));
    assert!(php.contains("__doria_write_stderr($line, start:"));
    assert!(php.contains("__doria_printf("));
    assert!(php.contains("\"enabled=%s\", (function()"));
    assert_php_operation(&php, "__doria_display(false)");
    assert!(php.contains("__doria_sprintf(\"%05d\", 42)"));
    // Execute the formatting expressions independently of this fixture's
    // required input files, retaining every checked-I/O transport assertion.
    let formatting = doriac::compile_source_to_php(
        "formatting.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError {
    printf("enabled=%s", false);
    echo sprintf("%05d", 42);
}
"#,
    )
    .unwrap();
    assert_php_main_execution(&formatting, b"enabled=false00042", 0);
}

#[test]
fn php_prompted_read_line_preserves_prompt_and_line_discipline() {
    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }

    let php = doriac::compile_source_to_php(
        "prompted-input.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let $first = read_line("P: ");
    if ($first != null) { echo "<{$first}>\n"; }

    let $blank = read_line();
    if ($blank != null) { echo "[{$blank}]\n"; }

    let $eof = read_line("E: ");
    if ($eof == null) { echo "EOF"; }
}
"#,
    )
    .expect("prompted input should lower to PHP");
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let execution = php_execution::Script::new(&script);
    let mut child = execution
        .command(&[])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("PHP should execute prompted input");
    child
        .stdin
        .take()
        .expect("PHP stdin should be piped")
        .write_all(b"alpha\r\n\n")
        .expect("PHP fixture input should be writable");
    let output = child.wait_with_output().expect("PHP fixture should exit");

    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"P: <alpha>\n[]\nE: EOF");
    assert!(output.stderr.is_empty());
}

#[test]
fn php_backend_exits_cleanly_only_for_user_output_to_closed_pipes() {
    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }

    for (name, statement, close_stdout) in [
        ("stdout-echo", "echo \"Doria output.\\n\";", true),
        ("stdout-printf", "printf(\"Doria output.\\n\");", true),
        ("stderr", "write_stderr(\"Doria output.\\n\");", false),
    ] {
        let source = format!(
            r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{{
    let $line = read_line();
    {statement}
}}
"#
        );
        let php = doriac::compile_source_to_php(format!("closed-{name}.doria"), &source)
            .expect("closed-pipe fixture should lower to PHP");
        let script = format!(
            "{}\n__DoriaFunction_6d61696e();",
            php.strip_prefix("<?php").expect("generated PHP header")
        );
        let execution = php_execution::Script::new(&script);
        let mut child = execution
            .command(&[])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("PHP should execute the closed-pipe fixture");
        if close_stdout {
            drop(child.stdout.take());
        } else {
            drop(child.stderr.take());
        }
        child
            .stdin
            .take()
            .expect("PHP stdin should be piped")
            .write_all(b"\n")
            .expect("PHP input should unblock the fixture");
        let output = child.wait_with_output().expect("PHP fixture should exit");

        assert_eq!(
            output.status.code(),
            Some(0),
            "closed {name} must be a clean exit"
        );
        assert!(output.stdout.is_empty(), "{name} fixture wrote stdout");
        assert!(output.stderr.is_empty(), "{name} fixture wrote stderr");
    }

    let php = doriac::compile_source_to_php(
        "closed-prompt.doria",
        "function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { let $line = read_line(\"Prompt: \" ); }",
    )
    .expect("closed prompt fixture should lower to PHP");
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let execution = php_execution::Script::new(&script);
    let mut child = execution
        .command(&[])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("PHP should execute the closed-prompt fixture");
    drop(child.stdout.take());
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("PHP fixture should exit");
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty());
}

#[test]
fn php_backend_keeps_panic_fatal_when_stderr_is_closed() {
    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }

    let php = doriac::compile_source_to_php(
        "panic-closed-stderr.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    let $line = read_line();
    panic("boom");
}
"#,
    )
    .expect("panic fixture should lower to PHP");
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let execution = php_execution::Script::new(&script);
    let mut child = execution
        .command(&[])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("PHP should execute the panic fixture");
    drop(child.stderr.take());
    child
        .stdin
        .take()
        .expect("PHP stdin should be piped")
        .write_all(b"\n")
        .expect("PHP input should unblock the fixture");
    let output = child.wait_with_output().expect("PHP fixture should exit");

    assert_eq!(output.status.code(), Some(101));
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn php_backend_rejects_noncanonical_float_display_in_checked_formats() {
    for source in [
        "function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { echo sprintf(\"%s\", 1.5); }",
        "function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { printf(\"%s\", 1.5); }",
    ] {
        let diagnostics = doriac::compile_source_to_php("test.doria", source)
            .expect_err("PHP must reject float display formatting it cannot preserve canonically");
        assert_eq!(diagnostics[0].code, "B1301");
        assert!(diagnostics[0]
            .message
            .contains("canonical float display formatting"));
    }
}

#[test]
fn php_backend_keeps_stage17_frontend_rejections() {
    for source in [
        "function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { print(\"x\"); }",
        "function main(): void throws Doria\\Std\\Io\\IoError, Doria\\Std\\Io\\InvalidUtf8Error { let $format = \"%d\"; echo sprintf($format, 1); }",
    ] {
        doriac::compile_source_to_php("test.doria", source)
            .expect_err("invalid Doria must fail before PHP lowering");
    }
}

#[test]
fn php_backend_preserves_stage_18_expression_interpolation_order() {
    let php = doriac::compile_source_to_php(
        "test.doria",
        r#"
function left(): int throws Doria\Std\Io\IoError
{
    echo "L";
    return 20;
}

function right(): int throws Doria\Std\Io\IoError
{
    echo "R";
    return 22;
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    echo "={left() == 20 and right() == 22}";
}
"#,
    )
    .expect("Stage 18 expression interpolation should lower to PHP");

    let left_call = format!("{}()", php_function_name("left"));
    let right_call = format!("{}()", php_function_name("right"));
    let expression = php
        .lines()
        .find(|line| {
            line.contains("__doria_display(")
                && line.contains(&left_call)
                && line.contains(&right_call)
        })
        .expect("ordered interpolation expression");
    assert_eq!(expression.matches(&left_call).count(), 1);
    assert_eq!(expression.matches(&right_call).count(), 1);
    assert!(expression.find(&left_call) < expression.find(&right_call));
    assert!(expression.contains("=== 20") && expression.contains("=== 22"));
    assert!(expression.contains(" && "));

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&[])
        .output()
        .expect("PHP should execute generated Stage 18 output");
    assert!(run.status.success());
    assert_eq!(run.stdout, b"LR=true");
    assert!(run.stderr.is_empty());
}

#[test]
fn php_backend_preserves_checked_integer_interpolation() {
    let php = doriac::compile_source_to_php(
        "main_expression_interpolation.doria",
        include_str!("../../../examples/native/main_expression_interpolation.doria"),
    )
    .expect("checked arithmetic now has an exact PHP lowering");
    let script = format!(
        "{}\n{}();",
        php.strip_prefix("<?php").unwrap(),
        php_function_name("main")
    );
    let run = php_execution::Script::new(&script)
        .command(&[])
        .output()
        .unwrap();
    assert_eq!(run.status.code(), Some(0), "{run:?}");
    assert_eq!(run.stdout, b"sum: 42");
    assert!(run.stderr.is_empty(), "{run:?}");
}

#[test]
fn php_backend_preserves_the_exact_displayable_contract() {
    let php = doriac::compile_source_to_php(
        "displayable.doria",
        include_str!("../../../examples/php/displayable.doria"),
    )
    .expect("the exact Displayable subset should lower to PHP");

    assert!(php.contains("interface __DoriaDisplayable"));
    assert!(php.contains(&format!(
        "class {} implements __DoriaDisplayable",
        php_type_name("Label")
    )));
    assert!(php.contains("public function toString(): string"));
    assert!(php.contains("$value->toString()"));
    assert!(!php.contains("__toString"));

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&[])
        .output()
        .expect("PHP should execute generated Displayable output");
    assert!(run.status.success());
    assert_eq!(run.stdout, b"Doria Doria Doria Doria");
    assert!(run.stderr.is_empty());
}

#[test]
fn php_backend_reserves_helper_class_names_case_insensitively() {
    for name in [
        "__DoriaDisplayable",
        "__doriadisplayable",
        "__DORIADISPLAYABLE",
        "__DoriaValueEquatable",
        "__doriavalueequatable",
        "__DoriaMixedValue",
        "__doriamixedvalue",
        "__DoriaFunctionValue",
        "__doriafunctionvalue",
        "__DoriaClosureEnvironment42",
        "__doriafuturehelper",
    ] {
        let diagnostics = doriac::compile_source_to_php(
            "reserved_display_helper.doria",
            format!("class {name} {{}}"),
        )
        .expect_err("PHP display helper name variants must be reserved");

        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "E0309"
                && diagnostic.message.contains("`__Doria` type prefix")
                && diagnostic.message.contains(&format!("`{name}`"))
                && diagnostic.message.contains("reserved")
        }));
    }
}

#[test]
fn php_backend_distinguishes_stage20_constants_and_static_properties() {
    let php = doriac::compile_source_to_php(
        "statics.doria",
        r#"
const TOP_LIMIT = 42;

class Counter
{
    const LABEL = "ready";
    static int $initial = TOP_LIMIT;
    static writable string $current = Counter::LABEL;

    static function read(): string
    {
        return Counter::current;
    }
}

function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    Counter::current = "done";
    echo Counter::LABEL;
    echo Counter::read();
}
"#,
    )
    .expect("Stage 20 statics should lower to the PHP compatibility backend");

    assert!(php.contains("const __DORIA_CONST_TOP_LIMIT = 42;"));
    assert!(php.contains("public const LABEL = \"ready\";"));
    assert!(php.contains("public static int $initial = 42;"));
    assert!(php.contains("public static string $current = \"ready\";"));
    assert!(php.contains("public static function read(): string"));
    assert!(php.contains(&format!("return {}::$current;", php_type_name("Counter"))));
    assert!(php.contains(&format!(
        "{}::$current = \"done\";",
        php_type_name("Counter")
    )));
    assert!(php.contains(&format!("{}::LABEL", php_type_name("Counter"))));
}

#[test]
fn php_backend_emits_evaluated_constants_and_static_initializers() {
    let php = doriac::compile_source_to_php(
        "evaluated-constants.doria",
        r#"
const ANSWER = LATER + 1;
const LATER = 41;

class Counter
{
    static int $initial = ANSWER;
    static writable int $value = Counter::initial + 1;
}

function main(): void throws Doria\Std\Io\IoError
{
    echo ANSWER;
    echo Counter::value;
}
"#,
    )
    .expect("evaluated declarations should lower to PHP literals");

    assert!(php.contains("const __DORIA_CONST_ANSWER = 42;"));
    assert!(php.contains("const __DORIA_CONST_LATER = 41;"));
    assert!(php.contains("public static int $initial = 42;"));
    assert!(php.contains("public static int $value = 43;"));
    assert!(!php.contains("= LATER + 1"));
    assert!(!php.contains(&format!("= {}::$initial + 1", php_type_name("Counter"))));

    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&[])
        .output()
        .expect("PHP should execute evaluated declarations");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"4243");
    assert!(run.stderr.is_empty());
}

#[test]
fn php_backend_mangles_every_top_level_constant_away_from_php_names() {
    let php = doriac::compile_source_to_php(
        "predefined-constants.doria",
        r#"
const CLASS = 1;
const INF = 2;
const NAN = 3;
function main(): void throws Doria\Std\Io\IoError
{
    echo CLASS;
    echo INF;
    echo NAN;
}
"#,
    )
    .expect("top-level Doria constants should not collide with PHP names");

    for name in ["CLASS", "INF", "NAN"] {
        assert!(php.contains(&format!("const __DORIA_CONST_{name} =")));
        assert!(!php.contains(&format!("const {name} =")));
        assert_php_display_write(&php, &format!("__DORIA_CONST_{name}"));
    }
    assert_php_main_execution(&php, b"123", 0);
}

#[test]
fn php_backend_rejects_class_constant_named_class_case_insensitively() {
    let diagnostics =
        doriac::compile_source_to_php("class-constant.doria", "class Counter { const CLASS = 1; }")
            .expect_err("PHP reserves the CLASS class-constant name");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "B2001"
            && diagnostic.message.contains("reserves `class`")));
}

#[test]
fn php_backend_moves_runtime_static_property_initializers_into_the_constructor() {
    let source = r#"
class Counter
{
    static int $seed = 41;
    int $value = Counter::seed;
}
"#;
    doriac::check_source("static-read-in-property.doria", source)
        .expect("the Doria initializer is valid independently of PHP restrictions");
    let php = doriac::compile_source_to_php("static-read-in-property.doria", source)
        .expect("runtime Doria initializers should lower into the PHP constructor");
    assert!(php.contains("public int $value;"));
    assert!(php.contains(&format!(
        "$this->value = {}::$seed;",
        php_type_name("Counter")
    )));
}

#[test]
fn php_backend_moves_static_calls_in_instance_initializers_before_constructor_body() {
    let source = r#"
class Factory
{
    int $value = Factory::seed();
    internal static function seed(): int { return 42; }
    function __construct() {
        if ($this->value != 42) { panic("initializer ran after constructor body"); }
    }
}
function main(): void { let $factory = new Factory(); echo $factory->value; }
"#;
    doriac::check_source("static-call-in-property.doria", source)
        .expect("Doria property initializers may call declaring-class static methods");
    let php = doriac::compile_source_to_php("static-call-in-property.doria", source)
        .expect("runtime Doria initializers should lower into the PHP constructor");
    assert!(php.contains("public int $value;"));
    assert!(php.contains("$this->value = (function()"));
    assert!(php.contains(&format!("return {}::seed();", php_type_name("Factory"))));
    assert_eq!(
        php.matches("__doria_frame_enter(\"Factory::__construct\",")
            .count(),
        1
    );
    assert_initializer_php_parity("static-call-in-property.doria", source, &php, b"42");
}

#[test]
fn php_backend_moves_executable_instance_initializers_into_generated_constructors() {
    let cases = [
        (
            "function-call-in-property.doria",
            r#"
function seed(): int { return 42; }
class Counter { int $value = seed(); }
function main(): void { let $counter = new Counter(); echo $counter->value; }
"#,
            "Counter",
            "value",
            format!("return {}();", php_function_name("seed")),
            "42",
        ),
        (
            "construction-in-property.doria",
            r#"
class Person {
    int $id = 7;
    function __destruct(): void {
        try { echo "drop;"; } catch (Doria\Std\Io\IoError) {}
    }
}
class Office { Person $manager = new Person(); }
function main(): void { let $office = new Office(); echo $office->manager->id; }
"#,
            "Office",
            "manager",
            format!("return new {}();", php_type_name("Person")),
            "7drop;",
        ),
    ];

    for (path, source, class, property, expected_invocation, expected_stdout) in cases {
        doriac::check_source(path, source)
            .expect("executable property defaults are valid Doria independently of PHP");
        let php = doriac::compile_source_to_php(path, source)
            .expect("the PHP backend should lower executable defaults into a constructor");
        assert!(
            php.contains(&format!("$this->{property} = (function()")),
            "generated PHP:\n{php}"
        );
        assert!(php.contains(&expected_invocation), "generated PHP:\n{php}");
        assert_eq!(
            php.matches(&format!("__doria_frame_enter(\"{class}::__construct\","))
                .count(),
            1
        );
        assert_initializer_php_parity(path, source, &php, expected_stdout.as_bytes());
    }
}

fn assert_initializer_php_parity(
    path: &str,
    source: &str,
    php: &str,
    stdout: &[u8],
) -> doriac::mir::Program {
    let mir = doriac::lower_source_to_mir(path, source).unwrap();
    let expected = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(expected.exit_status, 0, "{path}: {expected:?}");
    assert_eq!(expected.stdout, stdout, "{path}: {expected:?}");
    assert!(expected.stderr.is_empty(), "{path}: {expected:?}");
    assert_php_main_execution(php, stdout, expected.exit_status);
    mir
}

#[test]
fn php_generic_property_writes_transfer_owned_values_and_preserve_copy_values() {
    for (ty, first, second, third, read, expected) in [
        (
            "Item",
            "new Item(1)",
            "new Item(2)",
            "makeItem(3)",
            "$holder->value->id",
            "value 1;drop 1;value 2;make 3;drop 2;value 3;drop 3;left;",
        ),
        (
            "List<Item>",
            "[new Item(1)]",
            "[new Item(2)]",
            "[makeItem(3)]",
            "$holder->value[0]->id",
            "value 1;drop 1;value 2;make 3;drop 2;value 3;drop 3;left;",
        ),
        (
            "int",
            "1",
            "2",
            "3",
            "$holder->value",
            "value 1;value 2;value 3;left;",
        ),
    ] {
        let source = format!(
            r#"
class Item {{
    function __construct(int $id) {{}}
    function __destruct(): void {{
        try {{ echo "drop {{$this->id}};"; }} catch (Doria\Std\Io\IoError) {{}}
    }}
}}
class Holder<T> {{
    writable T $value;
    function __construct(parameter take T $value) {{ $this->value = $value; }}
    writable function replace(take T $value): void {{ $this->value = $value; }}
}}
function makeItem(int $id): Item throws Doria\Std\Io\IoError {{
    echo "make {{$id}};";
    return new Item($id);
}}
function main(): void throws Doria\Std\Io\IoError {{
    {{
        let writable $holder = new Holder<{ty}>({first});
        echo "value " . {read} . ";";
        let $replacement = {second};
        $holder->replace($replacement);
        echo "value " . {read} . ";";
        $holder->value = {third};
        echo "value " . {read} . ";";
    }}
    echo "left;";
}}
"#
        );
        let php = doriac::compile_source_to_php("generic-owned-property.doria", &source)
            .unwrap_or_else(|diagnostics| panic!("{ty}: {diagnostics:?}"));
        let script = format!(
            "{}\n{}();",
            php.strip_prefix("<?php").unwrap(),
            php_function_name("main")
        );
        let output = php_execution::Script::new(&script)
            .command(&[])
            .output()
            .unwrap();
        assert!(output.status.success(), "{ty}: {output:?}");
        assert!(output.stderr.is_empty(), "{ty}: {output:?}");
        assert_eq!(output.stdout, expected.as_bytes(), "{ty}");
    }
}

#[test]
fn php_backend_executes_constructor_rooted_and_owned_property_writes() {
    let source =
        include_str!("../../../examples/native/main_constructor_owned_property_writes.doria");
    let php = doriac::compile_source_to_php("constructor-owned-property.doria", source)
        .expect("constructor-rooted and owned property writes should lower to PHP");

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&["-d", "display_errors=1"])
        .output()
        .expect("generated constructor property PHP should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        run.stdout,
        include_bytes!("fixtures/native_io/main_constructor_owned_property_writes/expected_stdout")
    );
}

#[test]
fn php_backend_keeps_php_constant_expression_property_defaults() {
    let source = r#"
const int SEED = 41;
class Config
{
    const int OFFSET = 1;
    List<int> $values = [SEED, Config::OFFSET];
    bool $enabled = true && !false;
    List<bool> $flags = [true and !false, false or true, true xor false];
    int $fallback = null ?? SEED;
}
function main(): void {
    let $config = new Config();
    echo $config->values[0] . ":" . $config->values[1] . ":" . $config->enabled;
    echo ":" . $config->flags[0] . ":" . $config->flags[1] . ":" . $config->flags[2];
    echo ":" . $config->fallback;
}
"#;
    let php = doriac::compile_source_to_php("constant-property-defaults.doria", source)
        .expect("PHP constant expressions remain valid property defaults");

    assert!(php.contains(&format!(
        "public array $values = [__DORIA_CONST_SEED, {}::OFFSET];",
        php_type_name("Config")
    )));
    assert!(php.contains("public bool $enabled = ((true) && (!(false)));"));
    assert!(php.contains("public array $flags = [((true) && (!(false))), ((false) || (true)), ((true) !== (false))];"));
    assert!(php.contains("public int $fallback = ((null) ?? (__DORIA_CONST_SEED));"));
    assert!(!php.contains("__doria_frame_enter(\"Config::__construct\","));
    assert_initializer_php_parity(
        "constant-property-defaults.doria",
        source,
        &php,
        b"41:1:true:true:true:true:41",
    );
}

#[test]
fn php_backend_runs_constant_looking_checked_initializers_as_authored_operations() {
    let source = r#"
const int SEED = 41;
class Counter {
    int $value = SEED + 1;
    float $ratio = 5.0 / 2.0;
    string $label = "value=" . SEED;
    bool $less = SEED < 43;
    List<int> $values = [SEED + 1, 5];
}
function main(): void {
    let $counter = new Counter();
    if ($counter->ratio != 2.5 || !$counter->less) { panic("initializer changed"); }
    echo $counter->value . ":" . $counter->label . ":" . $counter->values[0] . ":" . $counter->values[1];
}
"#;
    let php = doriac::compile_source_to_php("checked-property-defaults.doria", source).unwrap();
    assert!(php.contains("public int $value;"));
    assert!(php.contains("public array $values;"));
    assert!(php.contains("$this->value = (function()"));
    assert!(php.contains("$this->values = [(function()"));
    assert!(php.contains("__doria_integer_operation(\"+\","));
    assert_php_operation(&php, "fdiv(5.0, 2.0)");
    assert_eq!(
        php.matches("__doria_frame_enter(\"Counter::__construct\",")
            .count(),
        1
    );
    assert_initializer_php_parity(
        "checked-property-defaults.doria",
        source,
        &php,
        b"42:value=41:42:5",
    );
}

#[test]
fn php_backend_materializes_constant_looking_owned_storage_per_instance() {
    let source = r#"
class Key implements Equatable<Key>, Hashable {
    function __construct(int $id) {}
    function equals(Key $other): bool { return $this->id == $other->id; }
    function hash(): uint64 { return 7; }
    function __destruct(): void {
        try { echo ":dropKey{$this->id}"; } catch (Doria\Std\Io\IoError) {}
    }
}
class Holder {
    mixed $value = 42;
    List<mixed> $items = [7, true];
    writable Set<Key> $keys = Set::from([]);
}
function describe(mixed $value): string {
    if ($value is int) { return "" . $value; }
    if ($value is bool) { return "" . $value; }
    panic("stored type changed");
}
function main(): void {
    {
        let writable $left = new Holder();
        let $right = new Holder();
        $left->keys->add(new Key(1));
        echo describe($left->value) . ":" . describe($right->value);
        echo ":" . describe($left->items[0]) . ":" . describe($left->items[1]);
        echo ":" . $left->keys->count . ":" . $right->keys->count;
    }
    echo ":done";
}
"#;
    let path = "owned-storage-property-defaults.doria";
    let php = doriac::compile_source_to_php(path, source).unwrap();
    assert!(php.contains("public mixed $value;"), "{php}");
    assert!(php.contains("public array $items;"), "{php}");
    assert!(php.contains("public __DoriaCoreCollection $keys;"), "{php}");
    for property in ["value", "items", "keys"] {
        assert!(php.contains(&format!("$this->{property} = ")), "{php}");
    }
    let expected = b"42:42:7:true:1:0:dropKey1:done";
    let mir = assert_initializer_php_parity(path, source, &php, expected);
    for profile in [NativeProfile::Fast]
        .into_iter()
        .chain(cfg!(feature = "llvm-backend").then_some(NativeProfile::Release))
    {
        let output = native_execution::execute_native(&mir, profile);
        assert_eq!(output.status.code(), Some(0), "{profile:?}: {output:?}");
        assert_eq!(output.stdout, expected, "{profile:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{profile:?}: {output:?}");
    }
}

#[test]
fn php_backend_keeps_collection_property_defaults_per_instance() {
    let php = doriac::compile_source_to_php(
        "collection-property-defaults.doria",
        r#"
class Scene {}
class SceneManager
{
    writable List<Scene> $scenes = [];
}
"#,
    )
    .expect("PHP compatibility output should preserve constant empty collection defaults");

    assert!(php.contains("public array $scenes = [];"));
    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }

    let script = format!(
        "{}\n$left = new {manager}(); $right = new {manager}(); $left->scenes[] = new {scene}(); echo count($left->scenes) . ':' . count($right->scenes);",
        php.strip_prefix("<?php").expect("generated PHP header"),
        manager = php_type_name("SceneManager"),
        scene = php_type_name("Scene"),
    );
    let run = php_execution::Script::new(&script)
        .command(&["-d", "display_errors=1"])
        .output()
        .expect("generated collection property defaults should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"1:0");
}

#[test]
fn php_backend_emits_int_min_constants_without_php_literal_overflow() {
    let php = doriac::compile_source_to_php(
        "int-min-constant.doria",
        r#"
const int MINIMUM = -9223372036854775808;
class Limits { static int $minimum = MINIMUM; }
"#,
    )
    .expect("the full signed int range should lower to PHP");

    assert!(php.contains("const __DORIA_CONST_MINIMUM = (-9223372036854775807 - 1);"));
    assert!(php.contains("public static int $minimum = (-9223372036854775807 - 1);"));

    if let Ok(version) = Command::new("php").arg("--version").output() {
        if version.status.success() {
            let mut child = Command::new("php")
                .arg("-l")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("PHP lint should start");
            use std::io::Write;
            child
                .stdin
                .take()
                .expect("PHP stdin")
                .write_all(php.as_bytes())
                .expect("generated PHP should be written");
            let output = child.wait_with_output().expect("PHP lint should finish");
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

#[test]
fn php_backend_parenthesizes_composite_member_receivers() {
    let php = doriac::compile_source_to_php(
        "coalesced-receiver.doria",
        r#"
class Label
{
    function text(): string { return "label"; }
}

function read(?Label $left, ?Label $right): ?string
{
    return ($left ?? $right)?->text();
}
"#,
    )
    .expect("coalesced member receivers should lower to PHP");

    assert_php_operation(&php, "$left ?? $right");
    // The coalesced receiver is staged once in its statement region and the
    // null-safe call reads that operand; no expression closure is emitted.
    let body = php_function_body(&php, "read");
    assert!(!body.contains("function() use"), "{php}");
    assert!(
        body.contains("?->text(); } finally { __doria_operation_leave("),
        "{php}"
    );
    assert!(body.contains("return $__doria_operation_value"));
    let read = php_function_name("read");
    let label = php_type_name("Label");
    assert_php_execution(&php, &format!("echo {read}(null, null) === null ? 'null:' : 'wrong:'; echo {read}(new {label}(), null); echo ':'; echo {read}(null, new {label}());"), b"null:label:label", 0);
}

#[test]
fn php_collection_boundaries_distinguish_implemented_search_from_deferred_operations() {
    for (name, source) in [
        (
            "list indexOf",
            r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    List<int> $items = [1];
    echo $items->indexOf(1) ?? -1;
}
"#,
        ),
        (
            "list remove",
            r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    writable List<int> $items = [1];
    $items->remove(1);
}
"#,
        ),
        (
            "dictionary containsValue",
            r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    Dictionary<string, int> $items = ["answer" => 42];
    echo $items->containsValue(42);
}
"#,
        ),
        (
            "set endpoints",
            r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    Set<int> $items = Set::from([1]);
    echo $items->first ?? -1;
    echo $items->last ?? -1;
}
"#,
        ),
        (
            "default collection clear",
            r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    writable List<int> $items = [1];
    $items->clear();
}
"#,
        ),
        (
            "writable foreach",
            r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    writable List<int> $items = [1];
    foreach ($items as writable int $item) {
        $item += 1;
    }
}
"#,
        ),
        (
            "byte I/O",
            r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    write_stdout_bytes(read_stdin_bytes());
}
"#,
        ),
    ] {
        let expected = match name {
            "list indexOf" => Some("0"),
            "list remove" => Some(""),
            "dictionary containsValue" => Some("true"),
            "set endpoints" => Some("11"),
            "default collection clear" => Some(""),
            "byte I/O" => Some(""),
            _ => None,
        };
        if let Some(expected) = expected {
            let php = doriac::compile_source("search.doria", source, BackendTarget::Php).unwrap();
            let directory = native_execution::ExecutionDirectory::new();
            let path = directory.path().join("main.php");
            doriac::backend::publish_output(&path, php).unwrap();
            let output = Command::new("php").arg(path).output().unwrap();
            assert!(
                output.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                output.stderr.is_empty(),
                "{name}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(output.stdout, expected.as_bytes(), "{name}");
            continue;
        }
        let diagnostics = doriac::compile_source_to_php("stage23.doria", source)
            .err()
            .unwrap_or_else(|| panic!("PHP accepted unsupported {name}"));
        assert_eq!(diagnostics[0].code, "B2301", "{name}: {diagnostics:?}");
        assert!(
            diagnostics[0].message.contains("native") && diagnostics[0].message.contains("debug"),
            "{name}: {diagnostics:?}"
        );
    }
}

#[test]
fn php_backend_keeps_readonly_dictionary_projections_iterable() {
    let php = doriac::compile_source_to_php(
        "dictionary-projection.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    Dictionary<string, int> $items = ["answer" => 42];
    foreach ($items->keys as string $key) {
        echo $key;
    }
}
"#,
    )
    .expect("readonly dictionary projections have a faithful PHP foreach lowering");

    assert!(php.contains("foreach (__doria_collection_projection($items, true) as $key)"));
}

#[test]
fn php_ordered_collection_hints_execute_across_source_resolved_and_mir_paths() {
    for (family, ty, factory) in [
        (
            "SortedDictionary",
            "SortedDictionary<int, int>",
            "SortedDictionary::from([7 => 9])",
        ),
        ("SortedSet", "SortedSet<int>", "SortedSet::from([7])"),
        (
            "PriorityQueue",
            "PriorityQueue<int>",
            "PriorityQueue::from([7])",
        ),
        ("Deque", "Deque<int>", "Deque::from([7])"),
    ] {
        assert_php_ordered_hint_transport(family, ty, factory, false);
    }
}

#[test]
fn php_ordered_collection_hints_preserve_core_comparable_and_class_payload_transport() {
    for (family, ty, factory, core) in [
        (
            "SortedDictionary",
            "SortedDictionary<Key, int>",
            "SortedDictionary::from([new Key(7) => 9])",
            true,
        ),
        (
            "SortedSet",
            "SortedSet<Key>",
            "SortedSet::from([new Key(7)])",
            true,
        ),
        (
            "PriorityQueue",
            "PriorityQueue<Key>",
            "PriorityQueue::from([new Key(7)])",
            true,
        ),
        ("Deque", "Deque<Key>", "Deque::from([new Key(7)])", false),
    ] {
        assert_php_ordered_hint_transport(family, ty, factory, core);
    }
}

fn assert_php_ordered_hint_transport(family: &str, ty: &str, factory: &str, core: bool) {
    let source = format!(
        r#"
class Key implements Comparable<Key>, Equatable<Key>, Hashable, Cloneable {{
    function __construct(int $id) {{}}
    function compare(Key $other): Ordering {{
        if ($this->id < $other->id) {{ return Ordering::Less; }}
        if ($this->id > $other->id) {{ return Ordering::Greater; }}
        return Ordering::Equal;
    }}
    function clone(): self {{ return new Key($this->id); }}
    function equals(Key $other): bool {{ return $this->id == $other->id; }}
    function hash(): uint64 {{ return 7; }}
}}
function identity<T>(take T $values): T {{ return $values; }}
function annotated(take (?{ty}) $values): ((?{ty})) {{ return $values; }}
function main(): void {{
    let $source = annotated({factory});
    if ($source != null) {{ echo $source->count; }} else {{ echo "missing"; }}
    let $generic = identity({factory});
    echo ":" . $generic->count;
    let $call = fn(take {ty} $values) => $values;
    let $inferred = $call({factory});
    echo ":" . $inferred->count;
    let $nullableCall = fn(take (?{ty}) $values) => $values;
    let $nullableInferred = $nullableCall({factory});
    echo ":";
    if ($nullableInferred != null) {{ echo $nullableInferred->count; }} else {{ echo "missing"; }}
    ?{ty} $emptySource = null;
    ?{ty} $emptyGeneric = null;
    ?{ty} $emptyClosure = null;
    echo ":" . (annotated($emptySource) == null);
    echo ":" . (identity($emptyGeneric) == null);
    echo ":" . ($nullableCall($emptyClosure) == null) . "\n";
}}
"#
    );
    let path = format!("ordered-hints-{family}-{core}.doria");
    let mir = doriac::lower_source_to_mir(&path, &source)
        .unwrap_or_else(|diagnostics| panic!("{ty}: {diagnostics:?}"));
    let expected = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(expected.exit_status, 0, "{ty}: {expected:?}");
    assert!(expected.stderr.is_empty(), "{ty}: {expected:?}");
    assert_eq!(expected.stdout, b"1:1:1:1:true:true:true\n", "{ty}");
    let php = doriac::compile_source_to_php(&path, &source)
        .unwrap_or_else(|diagnostics| panic!("{ty}: {diagnostics:?}"));
    for invalid in [
        "__DoriaSortedDictionary",
        "__DoriaSortedSet",
        "__DoriaPriorityQueue",
        "__DoriaDeque",
    ] {
        assert!(!php.contains(invalid), "{ty}: undeclared carrier {invalid}");
    }
    let carrier = if core {
        "__DoriaCoreCollection"
    } else {
        family
    };
    assert!(
        php.contains(&format!(
            "public function __invoke(__DoriaCell $argument0): {carrier}"
        )),
        "{ty}: owned argument cell with non-nullable result"
    );
    assert!(
        php.contains(&format!(
            "public function __invoke(__DoriaCell $argument0): ?{carrier}"
        )),
        "{ty}: owned argument cell with nullable result"
    );
    let script = format!(
        "{}\n{}();",
        php.strip_prefix("<?php").unwrap(),
        php_function_name("main"),
    );
    let output = php_execution::Script::new(&script)
        .command(&["-d", "ffi.enable=0"])
        .output()
        .expect("PHP is required for ordered-collection hint execution regressions");
    assert_eq!(
        output.status.code(),
        Some(expected.exit_status),
        "{ty}: {output:?}"
    );
    assert_eq!(output.stdout, expected.stdout, "{ty}");
    assert_eq!(output.stderr, expected.stderr, "{ty}");
}

#[test]
fn php_backend_executes_the_stage26_collection_family_with_doria_ordering() {
    let php = doriac::compile_source_to_php(
        "stage26.php.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    writable SortedDictionary<int, string> $map =
        SortedDictionary::from([2 => "two", -1 => "minus", 1 => "one"]);
    $map->set(3, "three");
    foreach ($map->keys as int $key) { echo "{$key} "; }
    echo "\n";

    writable SortedSet<int> $set = SortedSet::from([3, -1, 1, 3]);
    $set->add(0);
    foreach ($set as int $value) { echo "{$value} "; }
    echo "\n";

    writable PriorityQueue<int> $queue = PriorityQueue::from([4, -2, 1]);
    $queue->push(-3);
    while (!$queue->isEmpty) {
        let $value = $queue->pop() ?? 99;
        echo "{$value} ";
    }
    echo "\n";

    writable Deque<string> $deque = Deque::from(["middle"]);
    $deque->pushFront("first");
    $deque->pushBack("last");
    foreach ($deque as string $value) { echo "{$value} "; }
    echo "\n";

    $map->clear();
    $map->set(9, "nine");
    foreach ($map->keys as int $key) { echo "map {$key} "; }
    $set->clear();
    $set->add(2);
    foreach ($set as int $value) { echo "set {$value} "; }
    $queue->clear();
    $queue->push(6);
    echo "queue " . ($queue->pop() ?? 99) . " ";
    $deque->clear();
    $deque->pushBack("new");
    echo "deque " . ($deque->peekFront ?? "none");
}
"#,
    )
    .expect("Stage 26 collections should lower to PHP compatibility helpers");

    assert!(php.contains("final class SortedDictionary"));
    assert!(php.contains("final class SortedSet"));
    assert!(php.contains("final class PriorityQueue"));
    assert!(php.contains("final class Deque"));

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&["-d", "display_errors=1"])
        .output()
        .expect("generated Stage 26 PHP should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        run.stdout,
        b"-1 1 2 3 \n-1 0 1 3 \n-3 -2 1 4 \nfirst middle last \nmap 9 set 2 queue 6 deque new"
    );
    assert!(
        run.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn php_backend_executes_checked_errors_with_doria_catch_coverage() {
    let fixtures = [
        (
            "checked-error-catch.doria",
            include_str!("../../../examples/native/main_checked_error_catch.doria"),
            "caught exact\n",
        ),
        (
            "checked-error-catch-all.doria",
            include_str!("../../../examples/native/main_checked_error_catch_all.doria"),
            "catch all\n",
        ),
        (
            "checked-error-hierarchy.doria",
            include_str!("../../../examples/native/main_stage34_inheritance_error_catch.doria"),
            "catch missing\n",
        ),
        (
            "checked-error-optional-binding.doria",
            include_str!("../../../examples/native/main_checked_error_optional_binding.doria"),
            "handled without binding\n",
        ),
        (
            "checked-error-rethrow.doria",
            include_str!("../../../examples/native/main_checked_error_rethrow.doria"),
            "relay\noriginal\n",
        ),
        (
            "checked-error-finally.doria",
            include_str!("../../../examples/native/main_checked_error_finally.doria"),
            "caught\nfinally\n",
        ),
        (
            "checked-error-constructor.doria",
            include_str!("../../../examples/native/main_checked_error_constructor.doria"),
            "caught constructor\n",
        ),
        (
            "ambient-io-fallible-finalizers.doria",
            include_str!("../../../examples/native/main_ambient_io_fallible_finalizers.doria"),
            "normal body normal\nreturn\nwhen\nbreak\ncontinue\nerror\nouter finalizer\nlocal handled return 42\n",
        ),
    ];

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }

    for (path, source, expected_stdout) in fixtures {
        let php = doriac::compile_source_to_php(path, source)
            .unwrap_or_else(|error| panic!("{path} should compile for PHP: {error:?}"));
        assert!(php.contains("catch (__DoriaCheckedError"));
        assert!(php.contains("::__doriaErrorType()"));
        if path == "checked-error-catch-all.doria" {
            assert!(!php.contains("->descriptor ==="));
        } else {
            assert!(php.contains("->error() instanceof "), "{path}");
        }
        assert!(!php.contains("->descriptor ==="));

        let script = format!(
            "{}\n__DoriaFunction_6d61696e();",
            php.strip_prefix("<?php").expect("generated PHP header")
        );
        let run = php_execution::Script::new(&script)
            .command(&["-d", "display_errors=1"])
            .output()
            .unwrap_or_else(|error| panic!("{path} generated PHP should execute: {error}"));

        assert!(
            run.status.success(),
            "{path}: {}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&run.stdout),
            expected_stdout,
            "{path}"
        );
        assert!(
            run.stderr.is_empty(),
            "{path}: {}",
            String::from_utf8_lossy(&run.stderr)
        );
    }
}

#[test]
fn php_catches_release_transport_carriers_before_handler_cleanup() {
    let source = r#"
class Failure implements Error {
    static writable int $drops = 0;
    function __construct(string $message) {}
    function __destruct() { self::drops++; }
}
function fail(): void throws Failure { throw new Failure("failed"); }
function main(): void {
    try { fail(); } catch (Failure $error) { echo $error->message; }
    echo ":"; echo Failure::drops; echo "|";
    try {
        try { fail(); } finally {}
    } catch (Failure) { echo "discarded"; }
    echo ":"; echo Failure::drops;
}
"#;
    let mir = doriac::lower_source_to_mir("catch-carrier-lifetime.doria", source).unwrap();
    let expected = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(expected.stdout, b"failed:1|discarded:2");
    assert_eq!(expected.exit_status, 0);
    assert!(expected.stderr.is_empty());

    let mut php = doriac::compile_source_to_php("catch-carrier-lifetime.doria", source).unwrap();
    assert_eq!(php.matches("new __DoriaCheckedError(").count(), 2);
    php = php.replace("new __DoriaCheckedError(", "__test_checked_carrier(");
    let drop_header = "function __doria_drop_value(mixed &$value): void\n{";
    assert_eq!(php.matches(drop_header).count(), 1);
    // Observe carrier lifetime without owning it or changing Error cleanup.
    php = php.replace(
        drop_header,
        &format!(
            "{drop_header}{}",
            r#"
    if ($value instanceof __DoriaErrorValue) {
        foreach ($GLOBALS['__test_carriers'] as $reference) {
            if ($reference->get() !== null) {
                throw new LogicException('catch retained a consumed host carrier');
            }
        }
        $GLOBALS['__test_cleanup_checks'] = ($GLOBALS['__test_cleanup_checks'] ?? 0) + 1;
    }
"#
        ),
    );
    php.push_str(
        r#"
function __test_checked_carrier(__DoriaErrorValue $error, __DoriaErrorDescriptor $descriptor,
    int $origin): __DoriaCheckedError
{
    $carrier = new __DoriaCheckedError($error, $descriptor, $origin);
    $GLOBALS['__test_carriers'][] = WeakReference::create($carrier);
    return $carrier;
}
"#,
    );
    assert_php_execution(
        &php,
        &format!(
            "{}(); if (($GLOBALS['__test_cleanup_checks'] ?? 0) !== 2) {{ throw new LogicException('missing carrier cleanup checks'); }}",
            php_function_name("main")
        ),
        &expected.stdout,
        expected.exit_status,
    );
}

#[test]
fn php_finalizer_replacements_drop_and_detach_superseded_errors() {
    let source = r#"
class FirstError implements Error
{
    function __construct(string $message) {}
}

class FinalError implements Error
{
    function __construct(string $message) {}
}

function failTwice(): void throws FirstError, FinalError
{
    try {
        throw new FirstError("first");
    } finally {
        throw new FinalError("replacement");
    }
}

function main(): void {}
"#;
    let php = doriac::compile_source_to_php("php-finalizer-replacement.doria", source)
        .expect("fallible finalizer source should compile for PHP");
    assert!(php.contains("__doria_detach_checked_error"));
    let drop_previous = php
        .find("$previous->dropError();")
        .expect("the superseded carrier payload must be dropped explicitly");
    let take_replacement = php
        .find("$caught->takeError()")
        .expect("the replacement payload must move into a detached carrier");
    assert!(drop_previous < take_replacement);

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\ntry {{ __DoriaFunction_6661696c5477696365(); }} catch (__DoriaCheckedError $error) {{ echo $error->getPrevious() === null ? 'detached' : 'chained'; }}",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&["-d", "display_errors=1"])
        .output()
        .expect("generated replacement PHP should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert_eq!(stdout, "detached");
    assert!(
        run.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn php_backend_preserves_ambient_effects_through_callbacks_and_list_algorithms() {
    let source = include_str!("../../../examples/native/main_ambient_io_callbacks.doria");
    let php = doriac::compile_source_to_php("ambient-io-callbacks.doria", source)
        .expect("ambient callbacks should compile for PHP compatibility output");

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }

    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&["-d", "display_errors=1"])
        .output()
        .expect("generated ambient callback PHP should execute");

    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        run.stdout,
        include_bytes!("fixtures/native_io/main_ambient_io_callbacks/expected_stdout")
    );
    assert!(
        run.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn php_collection_clear_releases_owned_deque_values_in_doria_order() {
    let php = doriac::compile_source_to_php(
        "stage26-clear-order.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    writable Deque<int> $values = Deque::from([1]);
    $values->clear();
}
"#,
    )
    .expect("a Stage 26 Deque should emit the PHP compatibility helper");

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        r#"{}
final class ClearOrderToken
{{
    public function __construct(private int $id) {{}}
    public function __destruct() {{ echo "drop {{$this->id}}\n"; }}
}}
$values = Deque::from([new ClearOrderToken(1), new ClearOrderToken(2)]);
$values->pushFront(new ClearOrderToken(3));
$values->clear();"#,
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&["-d", "display_errors=1"])
        .output()
        .expect("generated PHP collection clear should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "drop 2\ndrop 1\ndrop 3\n"
    );
}

#[test]
fn php_backend_executes_ordered_slice3_members_with_strict_nullable_semantics() {
    let php = doriac::compile_source_to_php(
        "stage26-slice3.php.doria",
        r#"
function main(): void throws Doria\Std\Io\IoError, Doria\Std\Io\InvalidUtf8Error
{
    writable SortedDictionary<string, ?int> $numbers = SortedDictionary::from([]);
    $numbers->set("empty", null);
    $numbers->set("zero", 0);
    $numbers->set("answer", 42);
    echo $numbers->containsValue(null);
    echo $numbers->containsValue(0);
    echo $numbers->containsValue(7);

    writable SortedDictionary<string, ?bool> $flags = SortedDictionary::from([]);
    $flags->set("empty", null);
    $flags->set("false", false);
    echo $flags->containsValue(null);
    echo $flags->containsValue(false);
    echo $flags->containsValue(true);
    echo "\n";

    SortedSet<int> $values = SortedSet::from([30, 10, 20]);
    echo $values->first ?? -1;
    echo " ";
    echo $values->last ?? -1;
    echo "\n";

    SortedSet<int> $empty = SortedSet::from([]);
    echo $empty->first ?? -1;
    echo " ";
    echo $empty->last ?? -1;
}

"#,
    )
    .expect("ordered Slice 3 members should lower to PHP compatibility helpers");

    assert!(php.contains("if ($entry[1] === $value) { return true; }"));
    assert!(php.contains("if ($name === 'first') { return $this->values[0] ?? null; }"));
    assert!(php.contains("if ($name === 'last')"));

    let Ok(version) = Command::new("php").arg("--version").output() else {
        return;
    };
    if !version.status.success() {
        return;
    }
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").expect("generated PHP header")
    );
    let run = php_execution::Script::new(&script)
        .command(&["-d", "display_errors=1"])
        .output()
        .expect("generated ordered Slice 3 PHP should execute");
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, b"truetruefalsetruetruefalse\n10 30\n-1 -1");
    assert!(
        run.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

#[test]
fn php_entry_boundary_uses_main_effective_checked_effects() {
    let nonthrowing =
        doriac::compile_source_to_php("nonthrowing-main.doria", "function main(): void {}")
            .expect("clause-free nonthrowing main should compile");
    let entry = nonthrowing
        .rsplit_once("if (isset($_SERVER['SCRIPT_FILENAME'])")
        .unwrap()
        .1;
    assert!(!entry.contains("catch (__DoriaCheckedError"));

    let inferred = doriac::compile_source_to_php(
        "inferred-main.doria",
        r#"
class Failure implements Error
{
    function __construct(string $message)
    {
    }
}

function main(): void
{
    throw new Failure("inferred");
}
"#,
    )
    .expect("clause-free throwing main should compile through the checked boundary");
    assert!(inferred.contains("catch (__DoriaCheckedError"));
    assert!(inferred.contains("__doria_report_unhandled_error($error);"));
}

#[test]
fn php_backend_preserves_explicit_parent_qualification_and_arguments() {
    let php = doriac::compile_source_to_php(
        "parent-calls.doria",
        r#"
open class Base
{
    function __construct(string $label) {}
    function describe(): string { return "base"; }
}

class Child extends Base
{
    function __construct() { parent::__construct("typed"); }
    function parentLabel(): string { return parent::describe(); }
}

function main(): void
{
    let $child = new Child();
    echo $child->parentLabel();
}
"#,
    )
    .expect("explicit parent calls should compile to PHP");

    assert_eq!(php.matches("parent::__construct(\"typed\");").count(), 1);
    assert_php_operation(&php, "parent::describe()");
    assert!(php.contains("return $__doria_operation_value"));
    assert!(!php.contains(&format!(
        "{}::__construct(\"typed\")",
        php_type_name("Base")
    )));
    assert!(!php.contains(&format!("{}::describe()", php_type_name("Base"))));
}

#[test]
fn php_backend_emits_constructor_roles_as_ordinary_parameters() {
    let source = r#"
open class Base
{
    function __construct(string $title) {}
}

class Child extends Base
{
    string $normalized;

    function __construct(override string $title, parameter string $raw, string $category)
    {
        parent::__construct($title);
        $this->normalized = $raw;
    }
}

function main(): void
{
    let $child = new Child("title", "value", "article");
}
"#;
    let php = doriac::compile_source_to_php("constructor-roles.doria", source)
        .expect("constructor roles should compile to PHP");
    assert!(php.contains(&format!(
        "class {} extends {}",
        php_type_name("Child"),
        php_type_name("Base")
    )));
    assert!(
        php.contains("public function __construct(string $title, string $raw, string $category)")
    );
    assert!(php.contains("public string $category;"));
    assert!(php.contains("parent::__construct($title);"));
    let parent_call = php
        .find("parent::__construct($title);")
        .expect("parent constructor call");
    let child_promotion = php
        .find("$this->category = $category;")
        .expect("manual child promotion");
    assert!(parent_call < child_promotion);
    assert!(!php.contains("public string $raw"));
    assert_eq!(php.matches("public string $title").count(), 1);
}
