fn diagnostics(source: &str) -> Vec<doriac::diagnostics::Diagnostic> {
    doriac::check_source("stage26.doria", source).expect_err("source should be rejected")
}

fn diagnostic(source: &str, code: &str) -> doriac::diagnostics::Diagnostic {
    diagnostics(source)
        .into_iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("expected {code}"))
}

fn interpret(source: &str) -> doriac::mir_interpreter::InterpreterOutput {
    let mir = doriac::lower_source_to_mir("stage26.doria", source)
        .expect("Stage 26 source should lower through shared MIR");
    doriac::mir_interpreter::interpret(&mir).expect("Stage 26 MIR should execute")
}

#[test]
fn complete_family_example_executes_in_the_semantic_oracle() {
    let source = include_str!("../../../examples/native/main_stage26_collections.doria");
    let output = interpret(source);
    assert_eq!(
        output.stdout,
        b"Alice:1\nBob:2\nCharlie:3\n135\n258\nfirst\nmiddle\nlast\nnullable deque: none\nnullable deque: 2\nnullable indexed: -1 20 0\nnullable map: -1 20 0\nnullable deque values: -1 -1 0 2 3\nnullable deque updated: 9 9 0 2 3\nzero deque: 0\n"
    );
    assert_eq!(output.exit_status, 0);
}

#[test]
fn signed_ordering_mutation_and_set_algebra_are_exact() {
    let output = interpret(
        r#"
function main(): void
{
    writable SortedDictionary<int, string> $map =
        SortedDictionary::from([2 => "two", -1 => "minus", 1 => "one"]);
    $map[1] = "ONE";
    $map->set(3, "three");
    let $removed = $map->remove(2) ?? "none";
    echo "{$map[-1]} {$map->has(2)} {$removed}\n";

    writable SortedSet<int> $left = SortedSet::from([3, -1, 1, 3]);
    SortedSet<int> $right = SortedSet::from([1, 2, 3]);
    echo "{$left->add(0)} {$left->add(0)} {$left->remove(-1)}\n";
    foreach ($left->union($right) as int $value) { echo "{$value}"; }
    echo "\n";
    foreach ($left->intersect($right) as int $value) { echo "{$value}"; }
    echo "\n";
    foreach ($left->difference($right) as int $value) { echo "{$value}"; }
}
"#,
    );
    assert_eq!(
        output.stdout,
        b"minus false two\ntrue false true\n0123\n13\n0"
    );
}

#[test]
fn sorted_dictionary_projections_follow_sorted_key_order() {
    let output = interpret(
        r#"
function main(): void
{
    SortedDictionary<int, string> $priorities = SortedDictionary::from([
        30 => "low",
        10 => "high",
        20 => "normal",
    ]);
    foreach ($priorities->keys as int $priority) { echo "{$priority} "; }
    echo "\n";
    foreach ($priorities->values as string $label) { echo "{$label} "; }
}
"#,
    );
    assert_eq!(output.stdout, b"10 20 30 \nhigh normal low ");
}

#[test]
fn priority_queue_and_deque_preserve_their_authored_orders() {
    let output = interpret(
        r#"
function main(): void
{
    writable PriorityQueue<int> $queue = PriorityQueue::from([4, -2, 4, 1]);
    let $peek = $queue->peek ?? -99;
    echo "{$peek} ";
    while (!$queue->isEmpty) {
        let $popped = $queue->pop() ?? -99;
        echo "{$popped} ";
    }
    let $missing = $queue->pop() ?? -99;
    echo "{$missing}\n";

    writable Deque<string> $deque = Deque::from([]);
    $deque->pushBack("b");
    $deque->pushFront("a");
    $deque->pushBack("c");
    let $first = $deque->peekFront ?? "none";
    let $last = $deque->peekBack ?? "none";
    echo "{$first}{$last} ";
    let $front = $deque->popFront() ?? "none";
    let $back = $deque->popBack() ?? "none";
    echo "{$front}{$back} ";
    foreach ($deque as string $value) { echo $value; }
}
"#,
    );
    assert_eq!(output.stdout, b"-2 -2 1 4 4 -99\nac ac b");
}

#[test]
fn existing_sources_are_preserved() {
    let output = interpret(
        r#"
function main(): void
{
    List<int> $source = [3, 1, 2];
    SortedSet<int> $set = SortedSet::from($source);
    PriorityQueue<int> $queue = PriorityQueue::from($source);
    Deque<int> $deque = Deque::from($source);
    Dictionary<string, int> $pairs = ["b" => 2, "a" => 1];
    SortedDictionary<string, int> $map = SortedDictionary::from($pairs);
    echo "{$source->count}:{$source[0]} {$pairs->count}:{$pairs["b"]} ";
    echo "{$set->count}:{$queue->count}:{$deque->count}:{$map->count}";
}
"#,
    );
    assert_eq!(output.stdout, b"3:3 2:2 3:3:3:2");
}

#[test]
fn nested_and_move_elements_preserve_single_ownership() {
    let output = interpret(
        r#"
class Token
{
    function __construct(string $name) {}
    function __destruct() { echo "drop {$this->name}\n"; }
}
function main(): void
{
    writable Deque<Token> $tokens = Deque::from([]);
    $tokens->pushBack(new Token("back"));
    $tokens->pushFront(new Token("front"));
    let $front = $tokens->popFront();
    if ($front != null) { echo "take {$front->name}\n"; }

    writable Deque<List<int>> $nested = Deque::from([]);
    $nested->pushBack([1, 2]);
    echo "{$nested->count}\n";
}
"#,
    );
    assert_eq!(output.stdout, b"take front\n1\ndrop front\ndrop back\n");
}

#[test]
fn direct_literals_and_ambiguous_empty_sources_have_actionable_diagnostics() {
    let direct = diagnostic(
        "function main(): void { SortedSet<int> $values = [1, 2]; }",
        "E0538",
    );
    assert_eq!(direct.title, "Explicit Collection Construction Required");
    assert!(direct.help.as_deref().unwrap().contains("SortedSet::from"));

    let empty = diagnostic(
        "function main(): void { let $values = Deque::from([]); }",
        "E0539",
    );
    assert_eq!(empty.title, "Collection Type Cannot Be Inferred");
    assert!(empty.help.as_deref().unwrap().contains("Deque<int>"));
}

#[test]
fn ordered_constraints_reject_floats_and_unproven_type_parameters() {
    let float = diagnostic(
        "function main(): void { PriorityQueue<float> $q = PriorityQueue::from([1.0]); }",
        "E0523",
    );
    assert_eq!(float.title, "Float Has No Collection Order");
    assert!(float
        .explanation
        .as_deref()
        .unwrap()
        .contains("total order"));

    let generic = diagnostic("function f<T>(PriorityQueue<T> $values): void {}", "E0537");
    assert_eq!(generic.title, "Comparable Constraint Required");
}

#[test]
fn set_iteration_is_readonly_and_priority_queue_has_no_iteration() {
    for family in ["Set", "SortedSet"] {
        let source = format!(
            "function main(): void {{ writable {family}<int> $s = {family}::from([1]); foreach ($s as writable int $v) {{}} }}"
        );
        let value = diagnostic(&source, "E0530");
        assert_eq!(value.title, "Set Elements Cannot Be Written In Place");
        assert!(value.help.as_deref().unwrap().contains("remove"));
    }
    let queue = diagnostic(
        "function main(): void { PriorityQueue<int> $q = PriorityQueue::from([1]); foreach ($q as int $v) {} }",
        "E0529",
    );
    assert_eq!(queue.title, "PriorityQueue Has No Foreach Order");
}

#[test]
fn non_consuming_operations_reject_move_values_without_clone_suggestions() {
    let from = diagnostic(
        r#"
class Token {}
function main(): void
{
    List<Token> $source = [new Token()];
    Deque<Token> $values = Deque::from($source);
}
"#,
        "E0528",
    );
    assert!(from.message.contains("Deque::from"));
    assert!(from.message.contains("Cloneable"));
    assert!(!from.message.contains("clone()"));
}

#[test]
fn writable_shared_payloads_accept_all_stage26_collection_kinds() {
    doriac::lower_source_to_mir(
        "stage26-shared.doria",
        r#"
function main(): void
{
    let $map = new WritableSharedReference<SortedDictionary<string, int>>(
        SortedDictionary::from(["a" => 1])
    );
    let $set = new WritableSharedReference<SortedSet<int>>(SortedSet::from([1]));
    let $queue = new WritableSharedReference<PriorityQueue<int>>(PriorityQueue::from([1]));
    let $deque = new WritableSharedReference<Deque<int>>(Deque::from([1]));
    let $mapAccess = $map->acquireWritableAccess();
    let $setAccess = $set->acquireWritableAccess();
    let $queueAccess = $queue->acquireWritableAccess();
    let $dequeAccess = $deque->acquireWritableAccess();
    $mapAccess->set("b", 2);
    $setAccess->add(2);
    $queueAccess->push(2);
    $dequeAccess->pushBack(2);
}
"#,
    )
    .expect("all Stage 26 collection kinds should be valid writable shared payloads");
}

#[test]
fn nullable_containers_preserve_presence_for_every_stage26_family() {
    let output = interpret(
        r#"
function noMap(): ?SortedDictionary<int, string> { return null; }
function aSet(): ?SortedSet<int> { return SortedSet::from([2, 1]); }
function noQueue(): ?PriorityQueue<int> { return null; }
function aDeque(): ?Deque<int> { return Deque::from([1, 2]); }

function main(): void
{
    let $map = noMap();
    let $set = aSet();
    let $queue = noQueue();
    let $deque = aDeque();
    if ($map != null) { echo "map "; } else { echo "no-map "; }
    if ($set != null) { echo "set{$set->count} "; } else { echo "no-set "; }
    if ($queue != null) { echo "queue "; } else { echo "no-queue "; }
    if ($deque != null) { echo "deque{$deque->count}"; } else { echo "no-deque"; }
}
"#,
    );
    assert_eq!(output.stdout, b"no-map set2 no-queue deque2");
}

#[test]
fn nullable_elements_use_the_shared_sequence_and_dictionary_paths() {
    let output = interpret(
        r#"
class Marker
{
    function __construct(int $value)
    {
    }
}

function main(): void
{
    writable Dictionary<int, ?int> $plain = [1 => null, 2 => 0];
    $plain->set(3, null);
    foreach ($plain->values as ?int $value) { echo $value ?? -1; echo " "; }

    Dictionary<int, ?int> $mapSource = [2 => null, 1 => 7, 3 => 0];
    writable SortedDictionary<int, ?int> $map = SortedDictionary::from($mapSource);
    $map->set(2, 20);
    $map->set(1, null);
    foreach ($map->values as ?int $value) { echo $value ?? -1; echo " "; }
    let $missing = $map->get(99) ?? -1;
    echo "{$missing}\n";

    Dictionary<int, ?string> $words = [1 => null, 2 => "Doria"];
    echo ($words[1] ?? "none") . " " . ($words[2] ?? "none") . "\n";

    Dictionary<int, ?Marker> $markers = [1 => null, 2 => new Marker(42)];
    if ($markers[1] == null) { echo "no marker "; }
    echo ($markers[2]?->value ?? -1) . "\n";

    writable List<?int> $list = [null, 0, 2];
    $list->insertAt(1, null);
    $list[2] = null;
    foreach ($list as ?int $value) { echo $value ?? -1; echo " "; }
    echo "\n";

    List<?int> $dequeSource = [null, 0, 2];
    writable Deque<?int> $deque = Deque::from($dequeSource);
    $deque->pushFront(null);
    $deque->pushBack(3);
    foreach ($deque as ?int $value) { echo $value ?? -1; echo " "; }
    let $front = $deque->popFront() ?? -1;
    let $back = $deque->popBack() ?? -1;
    echo "{$front}:{$back}\n";

    List<?string> $stringSource = [null, "Doria"];
    Deque<?string> $strings = Deque::from($stringSource);
    foreach ($strings as ?string $value) { echo $value ?? "none"; echo " "; }
}
"#,
    );
    assert_eq!(
        output.stdout,
        b"-1 0 -1 -1 20 0 -1\nnone Doria\nno marker 42\n-1 -1 -1 2 \n-1 -1 0 2 3 -1:3\nnone Doria "
    );
}

#[test]
fn nullable_dictionary_index_keeps_missing_keys_distinct_from_stored_null() {
    let output = interpret(
        r#"
function main(): void
{
    Dictionary<int, ?int> $values = [1 => null];
    echo $values[2] ?? -1;
}
"#,
    );
    assert_eq!(output.exit_status, 101);
    assert_eq!(
        output
            .runtime_diagnostic
            .expect("missing indexed key should retain a structured diagnostic")
            .code,
        "P1312"
    );
}
