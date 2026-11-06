use doriac::diagnostics::DiagnosticKind;

fn analyze(source: &str) -> doriac::semantics::SemanticAnalysis {
    doriac::analyze_source_for_ide("condition-bodies.doria", source)
        .expect("source parses")
        .1
}

fn assert_accepted(source: &str) {
    let analysis = analyze(source);
    assert!(
        analysis.diagnostics.is_empty(),
        "{source}: {:?}",
        analysis.diagnostics
    );
}

#[test]
fn authored_member_forwarding_preserves_checked_contracts() {
    for call in [
        "$this->relay($value)",
        "self::relayStatic($value)",
        "$this->relay(value: $value)",
        "self::relayStatic(value: $value)",
    ] {
        let declarations = format!(
            r#"
use Doria\Std\Io\ByteRegion;
interface Ready {{}}
class Yes implements Ready {{}}
trait Forward<S> {{
    function relay<N>(N $value): N where (S implements Ready) {{ return $value; }}
    static function relayStatic<N>(N $value): N where (S implements Ready) {{ return $value; }}
    function forward<M>(M $value): M where (S implements Ready) {{ return {call}; }}
}}
trait Middle<T> {{ uses Forward<T> {{ Forward<T>::forward as alias; }} }}
class Adapter<V> {{ uses Middle<V>; }}
"#
        );
        let callers = "function integer(Adapter<Yes> $adapter): int { return $adapter->alias(7); } \
            function region(take Adapter<Yes> $adapter, ByteRegion $value): ByteRegion { return $adapter->alias($value); }";
        for source in [
            format!("{declarations}{callers}"),
            format!("{callers}{declarations}"),
        ] {
            assert_accepted(&source);
        }
    }
}

#[test]
fn guarded_generic_forwarding_preserves_single_source_return_elision() {
    for composed in [false, true] {
        let method = "function forward<U>(U $value): U where (T implements Ready) { let $identity = fn(U $item) => $item; return $identity($value); }";
        let owner = if composed {
            format!("trait Forward<T> {{ {method} }} trait Middle<M> {{ uses Forward<M> {{ Forward<M>::forward as alias; }} }} class Adapter<V> {{ uses Middle<V>; }}")
        } else {
            format!("class Adapter<T> {{ {method} }}")
        };
        let declarations =
            format!("interface Ready {{}} class Yes implements Ready {{}} class Item {{}} {owner}");
        for take in ["", "take "] {
            let source = format!("{declarations} function item({take}Adapter<Yes> $adapter, Item $value): Item {{ return $adapter->forward($value); }}");
            if take.is_empty() {
                let analysis = analyze(&source);
                let findings = analysis.diagnostics.iter().collect::<Vec<_>>();
                assert_eq!(findings.len(), 1, "{source}: {:?}", analysis.diagnostics);
                assert_eq!(findings[0].code, "E0478");
                assert_eq!(
                    &source[findings[0].span.start..findings[0].span.end],
                    "$adapter->forward($value)"
                );
            } else {
                assert_accepted(&source);
            }
        }
    }
}

#[test]
fn guarded_methods_and_getters_reject_transferred_values_in_the_authored_body() {
    for owner in ["class", "trait"] {
        for body in [
            "function inspect(take Item $item): void where (T implements Ready) { consume($item); consume($item); }",
            "static function inspect(take Item $item): void where (T implements Ready) { consume($item); consume($item); }",
            "int $value where (T implements Ready) { get { let $item = new Item(); consume($item); consume($item); return 0; } }",
        ] {
            let source = format!(
                "interface Ready {{}} class Item {{}} function consume(take Item $item): void {{}} {owner} Adapter<T> {{ {body} }}"
            );
            let analysis = analyze(&source);
            let findings = analysis.diagnostics.iter().collect::<Vec<_>>();
            assert_eq!(findings.len(), 1, "{source}: {:?}", analysis.diagnostics);
            assert_eq!(findings[0].code, "E0470");
            assert_eq!(&source[findings[0].span.start..findings[0].span.end], "$item");
            assert!(!findings[0].related.is_empty());
        }
    }
}

#[test]
fn guarded_authored_bodies_preserve_valid_borrowed_and_owned_returns() {
    for owner in ["class", "trait"] {
        for body in [
            "function keep(Item $item): Item where (T implements Ready) { return $item; }",
            "function takeItem(take Item $item): Item where (T implements Ready) { return $item; }",
            "function bind(take Item $item): function(): int where (T implements Ready) { return fn() with (take $item) => $item->value; }",
        ] {
            assert_accepted(&format!("interface Ready {{}} class Item {{ int $value = 7; }} {owner} Adapter<T> {{ {body} }}"));
        }
    }
}

#[test]
fn imported_trait_aliases_preserve_taking_contracts_for_source_ownership() {
    for is_static in [false, true] {
        let modifier = if is_static { "static " } else { "" };
        let call = if is_static {
            "self::discard($item)"
        } else {
            "$this->discard($item)"
        };
        let root = format!(
            "interface Ready {{}} class Item {{}} trait Sink<T> {{ \
        {modifier}function accept(take Item $item): void where (T implements Ready) {{}} }}"
        );
        let forwarded = format!(
            r#"
trait Forward<U> {{
    uses Sink<U> {{ Sink<U>::accept as discard; }}
    {modifier}function inspect(take Item $item): void where (U implements Ready) {{
        {call};
        {call};
    }}
}}
class Adapter<V> {{ uses Forward<V>; }}
"#
        );
        for source in [format!("{root}{forwarded}"), format!("{forwarded}{root}")] {
            let analysis = analyze(&source);
            let findings = analysis.diagnostics.iter().collect::<Vec<_>>();
            assert_eq!(findings.len(), 1, "{source}: {:?}", analysis.diagnostics);
            assert_eq!(findings[0].code, "E0470");
            assert_eq!(
                &source[findings[0].span.start..findings[0].span.end],
                "$item"
            );
            assert_eq!(findings[0].span.expansion, doriac::source::ExpansionId(0));
        }
    }
}

#[test]
fn composed_class_callers_preserve_taking_contracts_for_methods_and_static_aliases() {
    let declarations = r#"
interface Ready {}
class Item {}
trait Sink<S> {
    function accept(take Item $item): void where (S implements Ready) {}
    static function acceptStatic(take Item $item): void where (S implements Ready) {}
}
"#;
    let uses =
        "uses Sink<U> { Sink<U>::accept as discard; Sink<U>::acceptStatic as discardStatic; }";
    let mut callers = Vec::new();
    for call in ["$adapter->accept($item)", "$adapter->discard($item)"] {
        callers.push(format!("class Adapter<U> {{ {uses} }} function invalid<T implements Ready>(Adapter<T> $adapter, take Item $item): void {{ {call}; {call}; }}"));
    }
    for call in ["self::acceptStatic($item)", "self::discardStatic($item)"] {
        callers.push(format!("class Adapter<U> {{ {uses} static function invalid(take Item $item): void where (U implements Ready) {{ {call}; {call}; }} }}"));
    }
    for caller in callers {
        for source in [
            format!("{declarations}{caller}"),
            format!("{caller}{declarations}"),
        ] {
            let analysis = analyze(&source);
            let findings = analysis.diagnostics.iter().collect::<Vec<_>>();
            assert_eq!(findings.len(), 1, "{source}: {:?}", analysis.diagnostics);
            assert_eq!(findings[0].code, "E0470");
            assert_eq!(
                &source[findings[0].span.start..findings[0].span.end],
                "$item"
            );
        }
    }
}

#[test]
fn guarded_source_storage_paths_do_not_require_executable_generic_instances() {
    for owner in ["class", "trait"] {
        for (declaration, instance) in [("Pair", "Pair"), ("Pair<T>", "Pair<int>")] {
            let source = format!(
                r#"
interface Ready {{}}
class Item {{ writable int $value = 0; }}
function write(writable Item $left, writable Item $right): int {{
    $left->value = 1;
    $right->value = 2;
    return $left->value + $right->value;
}}
class {declaration} {{
    writable Item $left = new Item();
    writable Item $right = new Item();
}}
{owner} Guard<S> {{
    static function inspect(): int where (S implements Ready) {{
        writable {instance} $pair = new {instance}();
        return write($pair->left, $pair->right);
    }}
}}
"#
            );
            assert_accepted(&source);
            let overlap = source.replace(
                "write($pair->left, $pair->right)",
                "write($pair->left, $pair->left)",
            );
            let analysis = analyze(&overlap);
            assert!(analysis
                .diagnostics
                .iter()
                .any(|finding| finding.code == "E0477"));
            assert!(
                analysis
                    .diagnostics
                    .iter()
                    .all(|finding| matches!(finding.code, "E0477")),
                "{overlap}: {:?}",
                analysis.diagnostics
            );
        }
    }
}

#[test]
fn unused_trait_ownership_is_checked_without_publishing_executable_closure_facts() {
    let source = "class Item {} function consume(take Item $item): void {} trait Actions { function inspect(take Item $item): void { consume($item); consume($item); } }";
    let analysis = doriac::analyze_source_for_ide("unused-trait.doria", source)
        .unwrap()
        .1;
    assert_eq!(analysis.diagnostics.len(), 1, "{:?}", analysis.diagnostics);
    assert_eq!(analysis.diagnostics[0].code, "E0470");
    let source = "class Item { int $value = 7; } trait Unused { function bind(take Item $item): function(): int { return fn() with (take $item) => $item->value; } } function main(): int { return 0; }";
    let analysis = doriac::analyze_source_for_ide("unused-trait-closure.doria", source)
        .unwrap()
        .1;
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert!(analysis.info.closure_ownership.is_empty());
    doriac::lower_source_to_mir("unused-trait-closure.doria", source).unwrap();
}

#[test]
fn source_ownership_does_not_admit_traits_as_value_types() {
    let source = r#"
interface Ready {}
trait Actions {}
class Guard<T> { function enable(): void where (T implements Ready) {} }
function consume(take Actions $value): void {}
function invalid(take Actions $value): void { consume($value); consume($value); }
"#;
    let analysis = analyze(source);
    assert!(analysis
        .diagnostics
        .iter()
        .any(|finding| finding.code == "E0750"));
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|finding| matches!(finding.code, "E0750")),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn active_conformance_conflicts_are_rejected_during_public_iteration_analysis() {
    let declarations = r#"
interface Ready {}
interface Writable {}
interface Left<E> extends Iterable<E> {}
interface Right<E> extends Iterable<E> {}
class Both implements Ready, Writable {}
class Adapter<T> implements Left<int> where (T implements Ready), Right<string> where (T implements Writable) {}
"#;
    let caller =
        "function inspect(Adapter<Both> $values): void { foreach ($values as int $value) {} }";
    for source in [
        format!("{declarations}{caller}"),
        format!("{caller}{declarations}"),
    ] {
        let analysis = analyze(&source);
        let failures = analysis.diagnostics.iter().collect::<Vec<_>>();
        assert_eq!(failures.len(), 1, "{:?}", analysis.diagnostics);
        assert_eq!(failures[0].code, "E0752");
        assert_eq!(
            &source[failures[0].span.start..failures[0].span.end],
            "Right<string>"
        );
        for guard in ["Ready", "Writable"] {
            assert!(failures[0]
                .related
                .iter()
                .any(|related| &source[related.span.start..related.span.end] == guard));
        }
    }
}

#[test]
fn inactive_duplicate_conformances_are_rejected_by_public_analysis() {
    let declarations = "interface Ready {} interface Writable {} class Neither {} \
        class Adapter<T> implements Iterable<int> where (T implements Ready), Iterable<int> where (T implements Writable) {}";
    let caller =
        "function inspect(Adapter<Neither> $values): void { foreach ($values as int $value) {} }";
    for source in [
        declarations.to_string(),
        format!("{declarations}{caller}"),
        format!("{caller}{declarations}"),
    ] {
        let analysis = analyze(&source);
        let failures = analysis.diagnostics.iter().collect::<Vec<_>>();
        assert_eq!(failures.len(), 1, "{:?}", analysis.diagnostics);
        assert_eq!(failures[0].code, "E0752");
        assert_eq!(
            &source[failures[0].span.start..failures[0].span.end],
            "Iterable<int>"
        );
        assert_eq!(
            failures[0]
                .related
                .iter()
                .filter(|related| &source[related.span.start..related.span.end] == "Iterable<int>")
                .count(),
            1
        );
    }
}

#[test]
fn composed_calls_use_effective_members_and_authored_body_premises_in_both_orders() {
    let declarations = r#"
interface Ready { function read(): int; }
class Yes implements Ready { function read(): int { return 1; } }
class No {}
trait Root<T> {
    function read(T $value): int where (T implements Ready) { return $value->read(); }
}
trait Middle<U> { uses Root<U> { Root<U>::read as inspect; } }
class Adapter<V> { uses Middle<V>; }
class Inactive { uses Root<No>; }
"#;
    let callers = r#"
function inspect<T implements Ready>(Adapter<T> $adapter, T $source): int {
    return $adapter->inspect($source) + $adapter->read($source);
}
function concrete(Adapter<Yes> $adapter, Yes $source): int { return $adapter->read($source); }
"#;
    for source in [
        format!("{declarations}{callers}"),
        format!("{callers}{declarations}"),
    ] {
        assert_accepted(&source);
    }
}

#[test]
fn inactive_composed_calls_report_the_callers_unproved_condition() {
    let source = r#"
interface Ready { function read(): int; }
class No {}
trait Inspection<T> {
    function read(T $value): int where (T implements Ready) { return $value->read(); }
}
class Adapter { uses Inspection<No>; }
function inspect(Adapter $adapter, No $source): int { return $adapter->read($source); }
"#;
    let analysis = analyze(source);
    let failures = analysis.diagnostics.iter().collect::<Vec<_>>();
    assert_eq!(failures.len(), 1, "{:?}", analysis.diagnostics);
    assert_eq!(failures[0].code, "E0779");
    assert_eq!(
        &source[failures[0].span.start..failures[0].span.end],
        "$adapter->read($source)"
    );
    assert!(failures[0]
        .related
        .iter()
        .any(|related| { source[related.span.start..related.span.end].starts_with("where (") }));
}

#[test]
fn inactive_trait_specializations_preserve_symbolic_method_results() {
    let declarations = r#"
interface Ready<T> { function read(): T; }
class Payload {}
class Other {}
class Source implements Ready<Payload> { function read(): Payload { return new Payload(); } }
class No {}
trait Inspection<T> {
    function read<U>(T $source): U where (T implements Ready<U>) { return $source->read(); }
}
class Adapter { uses Inspection<Source>; }
class Inactive { uses Inspection<No>; }
"#;
    let callers = r#"
function good(Adapter $adapter, Source $source): Payload { return $adapter->read($source); }
function bad(Adapter $adapter, Source $source): Other { return $adapter->read($source); }
"#;
    for source in [
        format!("{declarations}{callers}"),
        format!("{callers}{declarations}"),
    ] {
        let analysis = analyze(&source);
        let failures = analysis.diagnostics.iter().collect::<Vec<_>>();
        assert_eq!(failures.len(), 1, "{:?}", analysis.diagnostics);
        assert_eq!(failures[0].code, "E0779");
        assert_eq!(
            failures[0].span.start,
            source.rfind("$adapter->read($source)").unwrap()
        );
    }
}

#[test]
fn composed_computed_property_calls_share_the_checked_predicate() {
    let declarations = r#"
interface Ready {}
class Yes implements Ready {}
class No {}
trait State<T> {
    writable int $state where (T implements Ready) { get => 1; set (int $next) {} }
}
trait Middle<U> { uses State<U>; }
class Adapter<V> { uses Middle<V>; }
"#;
    let callers = r#"
function good<T implements Ready>(writable Adapter<T> $adapter): void { $adapter->state += 1; }
function bad(writable Adapter<No> $adapter): void { $adapter->state += 1; }
"#;
    for source in [
        format!("{declarations}{callers}"),
        format!("{callers}{declarations}"),
    ] {
        let analysis = analyze(&source);
        let failures = analysis.diagnostics.iter().collect::<Vec<_>>();
        assert_eq!(failures.len(), 1, "{:?}", analysis.diagnostics);
        assert_eq!(failures[0].code, "E0779");
    }
}

#[test]
fn concrete_locals_in_guarded_trait_bodies_keep_their_concrete_member_types() {
    assert_accepted(
        r#"
interface Ready<T> { function read(): T; }
class Payload {}
class Source implements Ready<Payload> { function read(): Payload { return new Payload(); } }
trait Inspection<T> {
    function read<U>(T $source, Source $concrete): U where (T implements Ready<U>) {
        Payload $value = $concrete->read();
        return $source->read();
    }
}
class Adapter { uses Inspection<Source>; }
"#,
    );
}

#[test]
fn authored_trait_calls_prove_imported_member_guards_in_both_source_orders() {
    let root = r#"
interface Ready {}
trait Read<T> { function read(): int where (T implements Ready) { return 1; } }
"#;
    let caller = r#"
trait Forward<U> {
    uses Read<U>;
    function good(): int where (U implements Ready) { return $this->read(); }
    function bad(): int { return $this->read(); }
}
class Adapter<V> { uses Forward<V>; }
"#;
    for source in [format!("{root}{caller}"), format!("{caller}{root}")] {
        let analysis = analyze(&source);
        let findings = analysis.diagnostics.iter().collect::<Vec<_>>();
        assert_eq!(findings.len(), 1, "{:?}", analysis.diagnostics);
        assert_eq!(findings[0].code, "E0779");
        assert_eq!(
            findings[0].span.start,
            source.rfind("$this->read()").unwrap()
        );
    }
}

#[test]
fn authored_trait_property_reads_check_the_imported_getter_condition() {
    let root = r#"
interface Ready {}
trait State<T> { int $state where (T implements Ready) { get => 1; } }
"#;
    let caller = r#"
trait Forward<U> {
    uses State<U>;
    function good(): int where (U implements Ready) { return $this->state; }
    function bad(): int { return $this->state; }
}
class Adapter<V> { uses Forward<V>; }
"#;
    for source in [format!("{root}{caller}"), format!("{caller}{root}")] {
        let analysis = analyze(&source);
        let findings = analysis.diagnostics.iter().collect::<Vec<_>>();
        assert_eq!(findings.len(), 1, "{:?}", analysis.diagnostics);
        assert_eq!(findings[0].code, "E0779");
        assert_eq!(
            findings[0].span.start,
            source.rfind("$this->state").unwrap()
        );
    }
}

#[test]
fn authored_trait_alias_calls_preserve_adaptations_and_reject_unproved_guards() {
    let source = r#"
interface Ready {}
trait Left<T> { function read(): int where (T implements Ready) { return 1; } }
trait Right<T> { function read(): int where (T implements Ready) { return 2; } }
trait Forward<U> {
    uses Left<U>, Right<U> { Left<U>::read insteadof Right<U>; Right<U>::read as alternative; }
    function good(): int where (U implements Ready) { return $this->read() + $this->alternative(); }
    function bad(): int { return $this->alternative(); }
}
class Adapter<V> { uses Forward<V>; }
"#;
    let analysis = analyze(source);
    let findings = analysis.diagnostics.iter().collect::<Vec<_>>();
    assert_eq!(findings.len(), 1, "{:?}", analysis.diagnostics);
    assert_eq!(findings[0].code, "E0779");
    assert_eq!(
        findings[0].span.start,
        source.rfind("$this->alternative()").unwrap()
    );
}

#[test]
fn authored_trait_static_calls_keep_owner_and_method_arguments_distinct() {
    let source = r#"
interface Ready {}
trait Factory<T> {
    static function make<U>(U $value): U where (T implements Ready) { return $value; }
}
trait Forward<U> {
    uses Factory<U>;
    static function good(int $value): int where (U implements Ready) { return self::make($value); }
    static function bad(int $value): int { return self::make($value); }
}
class Adapter<V> { uses Forward<V>; }
"#;
    let analysis = analyze(source);
    let findings = analysis.diagnostics.iter().collect::<Vec<_>>();
    assert_eq!(findings.len(), 1, "{:?}", analysis.diagnostics);
    assert_eq!(findings[0].code, "E0779");
    assert_eq!(
        findings[0].span.start,
        source.rfind("self::make($value)").unwrap()
    );
}

#[test]
fn composing_class_bodies_can_call_guarded_trait_members() {
    let declarations = r#"
interface Ready { function read(): int; }
class No {}
trait Read<T> {
    function read(T $source): int where (T implements Ready) { return $source->read(); }
}
class Adapter<U> {
    uses Read<U>;
    function forward(U $source): int where (U implements Ready) { return $this->read($source); }
}
class Inactive { uses Read<No>; }
"#;
    let callers = r#"
function forward<T implements Ready>(Adapter<T> $adapter, T $source): int {
    return $adapter->forward($source);
}
"#;
    for source in [
        format!("{declarations}{callers}"),
        format!("{callers}{declarations}"),
    ] {
        assert_accepted(&source);
    }
}

#[test]
fn nested_trait_bodies_can_call_their_composed_guarded_members() {
    assert_accepted(
        r#"
interface Ready { function read(): int; }
class No {}
trait Read<T> {
    function read(T $source): int where (T implements Ready) { return $source->read(); }
}
trait Forward<U> {
    uses Read<U>;
    function forward(U $source): int where (U implements Ready) { return $this->read($source); }
}
class Adapter<V> { uses Forward<V>; }
class Inactive { uses Forward<No>; }
"#,
    );
}

#[test]
fn guarded_bodies_share_method_property_and_generic_bound_premises() {
    assert_accepted(
        r#"
interface Ready { function read(): int; int $state { get; } }
function inspect<U implements Ready>(U $source): int { return $source->read(); }
class Adapter<T> {
    T $source;
    function __construct(parameter take T $source) { $this->source = $source; }
    function read(): int where (T implements Ready) { return inspect($this->source); }
    int $state where (T implements Ready) { get { return $this->source->state; } }
    static function inspect<U>(U $source): int where (U implements Ready) { return $source->read(); }
}
trait Inspection<T> {
    function inspect(T $source): int where (T implements Ready) { return $source->read(); }
}
"#,
    );
}

#[test]
fn callable_premises_do_not_leak_to_sibling_or_free_bodies() {
    let source = r#"
interface Ready { function read(): int; }
class Adapter<T> {
    function guarded(T $source): int where (T implements Ready) { return $source->read(); }
    function unguarded(T $source): int { return $source->read(); }
}
function unguarded<T>(T $source): int { return $source->read(); }
"#;
    let analysis = analyze(source);
    assert_eq!(
        analysis
            .diagnostics
            .iter()
            .filter(|finding| finding.code == "E0537")
            .count(),
        2,
        "{:?}",
        analysis.diagnostics
    );
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|finding| matches!(finding.code, "E0537")),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn instance_call_conditions_use_frozen_caller_bounds_in_both_source_orders() {
    let declarations = "interface Ready {} class Adapter<T> { function read(): int where (T implements Ready) { return 1; } }";
    let callers = "function good<T implements Ready>(Adapter<T> $value): int { return $value->read(); } function bad<T>(Adapter<T> $value): int { return $value->read(); }";
    for source in [
        format!("{declarations} {callers}"),
        format!("{callers} {declarations}"),
    ] {
        let analysis = analyze(&source);
        let findings = analysis
            .diagnostics
            .iter()
            .filter(|finding| finding.code == "E0779")
            .collect::<Vec<_>>();
        assert_eq!(findings.len(), 1, "{:?}", analysis.diagnostics);
        assert_eq!(findings[0].kind, DiagnosticKind::Language);
        assert_eq!(
            &source[findings[0].span.start..findings[0].span.end],
            "$value->read()"
        );
        assert_eq!(
            findings[0].span.start,
            source.rfind("$value->read()").unwrap()
        );
        assert!(findings[0]
            .related
            .iter()
            .any(|related| source[related.span.start..related.span.end].starts_with("where (")));
        assert!(
            analysis
                .diagnostics
                .iter()
                .all(|finding| matches!(finding.code, "E0779")),
            "{:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn concrete_and_null_safe_calls_cannot_invent_capabilities() {
    let source = r#"
interface Ready {}
class Yes implements Ready {}
class No {}
class Adapter<T> { function read(): int where (T implements Ready) { return 1; } }
function good(Adapter<Yes> $value): int { return $value->read(); }
function bad(Adapter<No> $value): int { return $value->read(); }
function nullable(?Adapter<No> $value): ?int { return $value?->read(); }
"#;
    let analysis = analyze(source);
    assert_eq!(
        analysis
            .diagnostics
            .iter()
            .filter(|finding| finding.code == "E0779")
            .count(),
        2,
        "{:?}",
        analysis.diagnostics
    );
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|finding| matches!(finding.code, "E0779")),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn static_method_conditions_follow_argument_and_result_inference() {
    let source = r#"
interface Ready {}
class Yes implements Ready {}
class No {}
class Factory {
    static function read<T>(T $source): int where (T implements Ready) { return 1; }
    static function make<T>(): ?T where (T implements Ready) { return null; }
}
function good<T implements Ready>(T $source): int { return Factory::read(source: $source); }
function bad<T>(T $source): int { return Factory::read(source: $source); }
function goodResult(): ?Yes { return Factory::make(); }
function badResult(): ?No { return Factory::make(); }
"#;
    let analysis = analyze(source);
    assert_eq!(
        analysis
            .diagnostics
            .iter()
            .filter(|finding| finding.code == "E0779")
            .count(),
        2,
        "{:?}",
        analysis.diagnostics
    );
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|finding| matches!(finding.code, "E0779")),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn unresolved_composed_method_arguments_report_authored_parameter_names() {
    let source = r#"
interface Ready {}
trait Factory {
    static function make<T>(): ?T where (T implements Ready) { return null; }
}
class Adapter { uses Factory; }
function inspect(): void { Adapter::make(); }
"#;
    let analysis = analyze(source);
    let findings = analysis.diagnostics.iter().collect::<Vec<_>>();
    assert_eq!(findings.len(), 1, "{:?}", analysis.diagnostics);
    assert_eq!(findings[0].code, "E0531");
    assert!(findings[0].message.contains("`T`"));
    assert!(!findings[0].message.contains("#trait"));
    assert_eq!(
        &source[findings[0].span.start..findings[0].span.end],
        "Adapter::make()"
    );
}

#[test]
fn unresolved_method_arguments_keep_the_ordinary_inference_diagnostic() {
    let source = "interface Ready {} class Factory { static function make<T>(): ?T where (T implements Ready) { return null; } } function inspect(): void { Factory::make(); }";
    let analysis = analyze(source);
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|finding| finding.code == "E0531"),
        "{:?}",
        analysis.diagnostics
    );
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|finding| matches!(finding.code, "E0531")),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn computed_property_read_modify_write_checks_one_shared_condition() {
    let source = r#"
interface Ready {}
class Adapter<T> {
    writable int $value where (T implements Ready) { get => 1; set (int $next) {} }
}
function good<T implements Ready>(writable Adapter<T> $value): void { $value->value += 1; }
function bad<T>(writable Adapter<T> $value): void { $value->value += 1; }
"#;
    let analysis = analyze(source);
    assert_eq!(
        analysis
            .diagnostics
            .iter()
            .filter(|finding| finding.code == "E0779")
            .count(),
        1,
        "{:?}",
        analysis.diagnostics
    );
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|finding| matches!(finding.code, "E0779")),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn guarded_core_bounds_keep_display_equality_and_ordering_contracts() {
    assert_accepted(
        r#"
class Inspector {
    static function display<T>(T $value): string where (T implements Displayable) { return "{$value}"; }
    static function equal<T>(T $left, T $right): bool where (T implements Equatable<T>) { return $left == $right; }
    static function before<T>(T $left, T $right): bool where (T implements Comparable<T>) { return $left < $right; }
}
"#,
    );
}

#[test]
fn symbolic_generic_call_bounds_are_proved_without_later_instantiation() {
    let source = "interface Ready {} function required<T implements Ready>(T $value): int { return 1; } function good<U implements Ready>(U $value): int { return required($value); } function bad<U>(U $value): int { return required($value); }";
    let analysis = doriac::analyze_source_for_ide("generic-call-bounds.doria", source)
        .unwrap()
        .1;
    assert_eq!(
        analysis
            .diagnostics
            .iter()
            .filter(|finding| finding.code == "E0535")
            .count(),
        1,
        "{:?}",
        analysis.diagnostics
    );
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|finding| finding.code == "E0535"),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn conditional_display_conversions_reject_only_inactive_specializations() {
    let declarations = r#"
interface Enabled {}
class Yes implements Enabled {}
class No {}
class Adapter<T> implements Displayable where (T implements Enabled) {
    function toString(): string { return "adapter"; }
}
"#;
    let callers = r#"
function good(Adapter<Yes> $value): string {
    echo $value;
    printf("%s", $value);
    return "{$value}" . $value . sprintf("%s", $value);
}
function badEcho(Adapter<No> $value): void { echo $value; }
function badConcat(Adapter<No> $value): string { return "value:" . $value; }
function badInterpolation(Adapter<No> $value): string { return "{$value}"; }
function badFormat(Adapter<No> $value): string { return sprintf("%s", $value); }
"#;
    for source in [
        format!("{declarations}{callers}"),
        format!("{callers}{declarations}"),
    ] {
        let analysis = analyze(&source);
        assert_eq!(
            analysis
                .diagnostics
                .iter()
                .filter(|finding| finding.code == "E0462")
                .count(),
            4,
            "{:?}",
            analysis.diagnostics
        );
        assert!(
            analysis
                .diagnostics
                .iter()
                .all(|finding| matches!(finding.code, "E0462" | "E0404")),
            "{:?}",
            analysis.diagnostics
        );
        assert_eq!(
            analysis
                .diagnostics
                .iter()
                .filter(|finding| finding.code == "E0404")
                .count(),
            1,
            "{:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn conditional_core_bounds_reject_inactive_generic_arguments() {
    let declarations = r#"
interface Enabled {}
class Yes implements Enabled {}
class No {}
class Adapter<T> implements Displayable where (T implements Enabled), Error where (T implements Enabled) {
    function __construct(string $message) {}
    function toString(): string { return "adapter"; }
}
function display<T implements Displayable>(T $value): string { return "{$value}"; }
function message<T implements Error>(T $value): string { return $value->message; }
"#;
    let callers = r#"
function good(Adapter<Yes> $value): string { return display($value) . message($value); }
function bad(Adapter<No> $value): string { return display($value) . message($value); }
"#;
    for source in [
        format!("{declarations}{callers}"),
        format!("{callers}{declarations}"),
    ] {
        let analysis = analyze(&source);
        assert_eq!(
            analysis
                .diagnostics
                .iter()
                .filter(|finding| finding.code == "E0535")
                .count(),
            2,
            "{:?}",
            analysis.diagnostics
        );
        assert!(
            analysis
                .diagnostics
                .iter()
                .all(|finding| matches!(finding.code, "E0535")),
            "{:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn guarded_error_message_and_throw_use_the_body_premises() {
    assert_accepted(
        r#"
interface Named { string $name { get; } }
class Inspector {
    static function message<T>(T $error): string where (T implements Error) { return $error->message; }
    static function nullable<T>(?T $error): ?string where (T implements Error) { return $error?->message; }
    static function name<T>(T $error): string where (T implements Error, T implements Named) { return $error->name; }
    static function propagate<T>(take T $error): void throws T where (T implements Error) { throw $error; }
}
"#,
    );
}

#[test]
fn conditional_error_admission_checks_throws_catch_and_throw() {
    let source = r#"
interface Enabled {}
class Yes implements Enabled {}
class No {}
class Failure<T> implements Error where (T implements Enabled) { function __construct(string $message) {} }
function good(take Failure<Yes> $error): void throws Failure<Yes> { throw $error; }
function bad(take Failure<No> $error): void throws Failure<No> { throw $error; }
function goodCatch(take Failure<Yes> $error): void { try { throw $error; } catch (Failure<Yes>) {} }
function badCatch(): void { try {} catch (Failure<No>) {} }
"#;
    let analysis = analyze(source);
    for code in ["E0618", "E0623", "E0626"] {
        assert_eq!(
            analysis
                .diagnostics
                .iter()
                .filter(|finding| finding.code == code)
                .count(),
            1,
            "{code}: {:?}",
            analysis.diagnostics
        );
    }
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|finding| matches!(finding.code, "E0618" | "E0623" | "E0626")),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn false_core_conditions_still_validate_authored_member_shapes() {
    let source = r#"
interface Enabled {}
class No {}
class BadDisplay<T> implements Displayable where (T implements Enabled) { function toString(): int { return 1; } }
class BadError<T> implements Error where (T implements Enabled) {}
function inspect(BadDisplay<No> $display, BadError<No> $error): void {}
"#;
    let analysis = analyze(source);
    for code in ["E0463", "E0613"] {
        assert_eq!(
            analysis
                .diagnostics
                .iter()
                .filter(|finding| finding.code == code)
                .count(),
            1,
            "{code}: {:?}",
            analysis.diagnostics
        );
    }
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|finding| matches!(finding.code, "E0463" | "E0613")),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn null_safe_symbolic_receivers_still_require_nominal_members() {
    let source = r#"
interface Ready { int $value { get; } }
class Inspector {
    static function good<T>(?T $source): ?int where (T implements Ready) { return $source?->value; }
    static function bad<T>(?T $source): ?int { return $source?->value; }
}
"#;
    let analysis = analyze(source);
    assert_eq!(
        analysis
            .diagnostics
            .iter()
            .filter(|finding| finding.code == "E0537")
            .count(),
        1,
        "{:?}",
        analysis.diagnostics
    );
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|finding| matches!(finding.code, "E0537")),
        "{:?}",
        analysis.diagnostics
    );
}
