use std::collections::BTreeMap;

fn analyze(source: &str, expected: &[(&str, usize)]) -> doriac::semantics::SemanticAnalysis {
    let analysis = doriac::analyze_source_for_ide("type-formation.doria", source)
        .unwrap()
        .1;
    let mut actual = BTreeMap::new();
    for finding in &analysis.diagnostics {
        *actual.entry(finding.code).or_insert(0) += 1;
    }
    assert_eq!(
        actual,
        expected.iter().copied().collect(),
        "{source}\n{:?}",
        analysis.diagnostics
    );
    analysis
}

#[test]
fn symbolic_class_and_interface_arguments_require_source_bounds_in_both_orders() {
    let declarations =
        "interface Ready {} class Box<T implements Ready> {} interface View<T implements Ready> {}";
    let uses = "function good<U implements Ready>(Box<U> $box, View<U> $view): void {} function bad<U>(Box<U> $box, View<U> $view): void {}";
    for source in [
        format!("{declarations} {uses}"),
        format!("{uses} {declarations}"),
    ] {
        analyze(&source, &[("E0535", 2)]);
    }
}

#[test]
fn symbolic_parent_and_trait_arguments_require_owner_bounds() {
    analyze(
        r#"
interface Ready {}
open class Base<T implements Ready> {}
trait Members<T implements Ready> {}
class Good<U implements Ready> extends Base<U> { uses Members<U>; }
class BadParent<U> extends Base<U> {}
class BadTrait<U> { uses Members<U>; }
"#,
        &[("E0535", 2)],
    );
}

#[test]
fn malformed_symbolic_conformance_types_publish_no_checked_witness() {
    let analysis = analyze(
        r#"
interface Ready {}
interface Needs<T implements Ready> {}
class No {}
class Bad<T> implements Needs<T> {}
function inspect(Bad<No> $value): void {}
"#,
        &[("E0535", 1)],
    );
    assert!(analysis
        .info
        .contracts
        .conformances
        .iter()
        .filter(|fact| matches!(&fact.implementing_type, doriac::types::ResolvedType::Class(class) if class.name == "Bad"))
        .all(|fact| fact.status != doriac::semantics::contracts::ConformanceStatus::Checked), "{:?}", analysis.info.contracts.conformances);
}

#[test]
fn guarded_signatures_properties_and_method_bounds_use_own_premises() {
    analyze(
        r#"
interface Ready {}
class Box<T implements Ready> {}
interface View<T implements Ready> {}
class Inspector<T> {
    function accept(Box<T> $box): int where (T implements Ready) { return 1; }
    static function inspect<U>(Box<U> $box): int where (U implements Ready) { return 1; }
    function nested<U implements View<T>>(U $value): int where (T implements Ready) { return 1; }
    Box<T> $box where (T implements Ready) { get { return new Box<T>(); } }
}
trait Inspection<T> {
    function accept(Box<T> $box): int where (T implements Ready) { return 1; }
    Box<T> $box where (T implements Ready) { get { return new Box<T>(); } }
}
"#,
        &[],
    );
}

#[test]
fn guarded_type_premises_do_not_enable_unguarded_storage_or_sibling_signatures() {
    analyze(
        r#"
interface Ready {}
class Box<T implements Ready> {}
class Inspector<T> {
    Box<T> $box where (T implements Ready) { get { return new Box<T>(); } }
    ?Box<T> $stored = null;
    function unguarded(Box<T> $box): int { return 1; }
}
"#,
        &[("E0535", 2)],
    );
}

#[test]
fn conditioned_conformance_type_arguments_use_the_conformance_guard() {
    analyze(
        r#"
interface Ready {}
interface Needs<T implements Ready> {}
class Good<T> implements Needs<T> where (T implements Ready) {}
class Bad<T> implements Needs<T> {}
"#,
        &[("E0535", 1)],
    );
}

#[test]
fn predicate_type_formation_uses_the_whole_conjunction_in_either_order() {
    for clause in [
        "T implements Needs<U>, U implements Ready",
        "U implements Ready, T implements Needs<U>",
    ] {
        let source = format!("interface Ready {{}} interface Needs<U implements Ready> {{}} class Inspector<T, U> {{ function inspect(): int where ({clause}) {{ return 1; }} }}");
        analyze(&source, &[]);
    }
}

#[test]
fn malformed_predicate_specializations_publish_no_condition_even_when_inactive() {
    let analysis = analyze(
        r#"
interface Ready {}
interface Needs<U implements Ready> {}
class No {}
class Inspector<T> { function inspect(): int where (T implements Needs<No>) { return 1; } }
"#,
        &[("E0535", 1)],
    );
    assert!(analysis.info.contracts.conditions.is_empty());
}

#[test]
fn nested_nominal_bound_types_are_validated_after_simultaneous_substitution() {
    analyze(
        r#"
interface Ready {}
class Item implements Ready {}
class Other implements Ready {}
class Box<V implements Ready> {}
interface Wrap<V> {}
class Accept<T implements Wrap<Box<U>>, U implements Ready> {}
class Source implements Wrap<Box<Item>> {}
function good<X implements Wrap<Box<Y>>, Y implements Ready>(Accept<X, Y> $value): void {}
function concrete(Accept<Source, Item> $value): void {}
function mismatch(Accept<Source, Other> $value): void {}
"#,
        &[("E0535", 1)],
    );
}

#[test]
fn nested_collection_constraints_are_checked_under_predicate_premises() {
    analyze(
        r#"
interface Wrap<V> {}
class Inspector<T, U> {
    function hashed(): void where (T implements Wrap<Set<U>>, U implements Hashable) {}
    function sorted(): void where (T implements Wrap<SortedSet<U>>, U implements Comparable<U>) {}
    function queued(): void where (T implements Wrap<PriorityQueue<U>>, U implements Comparable<U>) {}
    function dictionary(): void where (T implements Wrap<Dictionary<U, int>>, U implements Hashable) {}
    function sortedDictionary(): void where (T implements Wrap<SortedDictionary<U, int>>, U implements Comparable<U>) {}
    function bad(): void where (T implements Wrap<Set<U>>) {}
}
"#,
        &[("E0535", 1)],
    );
}

#[test]
fn nested_nullable_arguments_do_not_inherit_payload_capabilities() {
    let analysis = analyze(
        r#"
interface Wrap<V> {}
class Inspector<T, U> {
    function bad(): void where (T implements Wrap<Set<?U>>, U implements Hashable) {}
}
"#,
        &[("E0535", 1)],
    );
    assert!(analysis.info.contracts.conditions.is_empty());
}

#[test]
fn nested_primitive_constraints_keep_hashable_and_equatable_distinct() {
    analyze(
        r#"
interface Wrap<V> {}
class Inspector<T> {
    function good(): void where (T implements Wrap<Set<int>>) {}
    function bad(): void where (T implements Wrap<Set<float>>) {}
}
"#,
        &[("E0535", 1)],
    );
}

#[test]
fn guarded_return_contracts_and_borrow_analysis_use_declaration_scopes() {
    analyze(
        r#"
interface Ready {}
class Box<T implements Ready> {}
class Inspector<T> {
    function make(): Box<T> where (T implements Ready) { return new Box<T>(); }
    static function makeOther<U>(): Box<U> where (U implements Ready) { return new Box<U>(); }
    function borrowValue(Box<T> $value): Box<T> where (T implements Ready) { return $value; }
    function forward(Box<T> $value): Box<T> where (T implements Ready) { return $this->borrowValue($value); }
}
"#,
        &[],
    );
}

#[test]
fn named_returns_are_written_even_under_local_capability_conditions() {
    for source in [
        "function inspect() {}",
        "function inspect() throws Error {}",
        "class Inspector { function inspect() {} }",
        "class Inspector { static function inspect() {} }",
        "trait Inspection { function inspect() {} }",
        "interface Ready {} class Inspector<T> { function inspect() where (T implements Ready) {} }",
        "interface Ready {} trait Inspection<T> { function inspect() where (T implements Ready) {} }",
    ] {
        analyze(source, &[("E0617", 1)]);
    }
    analyze(
        "class Inspector { function __construct() {} function __destruct() {} } function main(): void { let $arrow = fn() => 1; int $value = $arrow(); }",
        &[],
    );
}

#[test]
fn recursive_class_expansion_keeps_ancestry_across_metadata_queries() {
    for source in [
        "class Node<T> { ?Node<List<T>> $next = null; } function main(): void { let $node = new Node<int>(); }",
        "class First<T> { ?Second<List<T>> $next = null; } class Second<T> { ?First<T> $next = null; } function main(): void { let $node = new First<int>(); }",
    ] {
        analyze(source, &[("E0539", 1)]);
    }
    for source in [
        "class Node<T> { ?Node<T> $next = null; } function main(): void { let $node = new Node<int>(); }",
        "class Node<T> { ?Node<int> $next = null; } function main(): void { let $node = new Node<bool>(); }",
    ] {
        analyze(source, &[]);
    }
}

#[test]
fn recursion_diagnostics_share_cycle_identity_and_preserve_independent_cycles() {
    let first = "class First<T> { ?Second<List<T>> $next = null; }";
    let second = "class Second<T> { ?First<T> $next = null; }";
    let caller = "function inspect(First<int> $first, Second<int> $second): void {}";
    for source in [
        format!("{first}{second}{caller}"),
        format!("{caller}{second}{first}"),
    ] {
        analyze(&source, &[("E0539", 1)]);
    }

    let first = "class First<T> { ?First<List<T>> $next = null; }";
    let second = "class Second<T> { ?Second<List<T>> $next = null; }";
    for source in [
        format!("{first}{second}{caller}"),
        format!("{caller}{second}{first}"),
    ] {
        let analysis = analyze(&source, &[("E0539", 2)]);
        for name in ["First", "Second"] {
            let diagnostic = analysis
                .diagnostics
                .iter()
                .find(|finding| finding.message.contains(&format!("class `{name}`")))
                .expect("each independent expansion retains its finding");
            assert!(source[diagnostic.span.start..diagnostic.span.end]
                .starts_with(&format!("class {name}")));
        }
    }
}
