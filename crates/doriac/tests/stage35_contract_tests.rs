use doriac::semantics::contracts::ConformanceStatus;

fn analyze(source: &str) -> doriac::semantics::SemanticAnalysis {
    doriac::analyze_source_for_ide("contracts.doria", source)
        .expect("declarations parse")
        .1
}

#[test]
fn concrete_generic_instances_publish_substituted_conformance_facts() {
    let source = "interface I<T> { function identity(take T $value): T; } class Box<T> implements I<T> { function identity(take T $value): T { return $value; } } function main(): void { let $box = new Box<int>(); echo $box->identity(42); }";
    let analysis = analyze(source);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert!(
        analysis.info.contracts.conformances.iter().any(|fact| {
            matches!(&fact.implementing_type, doriac::types::ResolvedType::Class(class)
                if class.name == "Box" && class.arguments == fact.interface.arguments
                    && matches!(class.arguments.as_slice(), [doriac::types::ResolvedType::Integer(_)]))
                && fact.interface.name == "I"
                && fact.status == ConformanceStatus::Checked
        }),
        "{:?}",
        analysis.info.contracts.conformances
    );
}

#[test]
fn compiler_known_contract_vocabulary_does_not_create_runtime_enums() {
    let source = "function main(): void { echo 42; }";
    let analysis = analyze(source);
    assert!(analysis
        .info
        .contracts
        .interfaces
        .iter()
        .any(|interface| interface.name == "Iterator"));
    let mir = doriac::lower_source_to_mir("plain.doria", source).unwrap();
    doriac::mir_interpreter::interpret(&mir).unwrap();
    for expression in ["Ordering::Less", "Ordering::Equal", "Ordering::Greater"] {
        let source = format!("function main(): void {{ let $order = {expression}; }}");
        let analysis = analyze(&source);
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0759"),
            "{:?}",
            analysis.diagnostics
        );
        assert!(doriac::lower_source("pending.doria", &source).is_err());
    }
}

#[test]
fn iterator_current_requires_receiver_borrow_for_move_elements() {
    let valid = "class Value {} class Cursor implements Iterator<Value> { Value $value = new Value(); function hasCurrent(): bool { return true; } function current(): Value { return $this->value; } writable function advance(): void {} }";
    let analysis = analyze(valid);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let invalid = valid.replace("return $this->value;", "return new Value();");
    let analysis = analyze(&invalid);
    assert!(
        analysis
            .info
            .contracts
            .conformances
            .iter()
            .any(|fact| fact
                .implementations
                .iter()
                .any(|implementation| implementation
                    .failures
                    .contains(&doriac::semantics::contracts::ContractMismatch::ReturnProvenance))),
        "{:?}",
        analysis.info.contracts.conformances
    );
}

#[test]
fn trait_local_members_are_checked_without_composer_injection() {
    let valid = "trait Counter { writable int $value = 0; writable function increment(): void { $this->value++; } function read(): int { return $this->value; } } function main(): void {}";
    assert!(
        analyze(valid).diagnostics.is_empty(),
        "{:?}",
        analyze(valid).diagnostics
    );
    for (source, code) in [
        (valid.replace("writable int", "int"), "E0202"),
        (valid.replace("writable function", "function"), "E0201"),
        (
            valid.replace(
                "writable int $value = 0",
                "writable string $value = \"text\"",
            ),
            "E0423",
        ),
    ] {
        let analysis = analyze(&source);
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == code),
            "{source}: {:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn trait_local_calls_preserve_static_receiver_and_argument_rules() {
    let valid = "trait Local { static function number(int $value): int { return $value; } function read(): int { return self::number(42); } }";
    assert!(analyze(valid).diagnostics.is_empty());
    for (source, code) in [
        (
            valid.replace("self::number(42)", "$this->number(42)"),
            "E0487",
        ),
        (valid.replace("static function", "function"), "E0487"),
        (valid.replace("number(42)", "number(\"text\")"), "E0408"),
    ] {
        let analysis = analyze(&source);
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == code),
            "{source}: {:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn nested_trait_property_paths_preserve_each_writable_edge() {
    let valid = "class Node { writable int $value = 0; } trait Local { writable Node $node = new Node(); writable function update(): void { $this->node->value++; } }";
    let analysis = analyze(valid);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    for source in [
        valid.replace("writable Node", "Node"),
        valid.replace("writable function", "function"),
    ] {
        let analysis = analyze(&source);
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0201"),
            "{:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn every_shared_family_and_nested_contract_position_has_a_pre_hir_boundary() {
    for ty in [
        "SharedReference<I>",
        "WeakReference<I>",
        "WritableSharedReference<I>",
        "WritableWeakReference<I>",
        "ReadonlySharedReferenceAccess<I>",
        "WritableSharedReferenceAccess<I>",
        "List<?I>",
        "Dictionary<string, I>",
        "function(I): void",
    ] {
        let source = format!(
            "interface I {{ function run(): void; }} function hold({ty} $value): void {{}}"
        );
        let analysis = analyze(&source);
        assert!(!analysis.diagnostics.is_empty(), "{ty}");
        assert!(
            analysis
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code == "E0758"),
            "{ty}: {:?}",
            analysis.diagnostics
        );
        assert!(doriac::lower_source("pending.doria", &source).is_err());
    }
}

#[test]
fn new_core_operations_are_pending_without_changing_concrete_methods() {
    let declarations = r#"
class Value implements Equatable<Value>, Hashable, Cloneable {
    function equals(Value $other): bool { return true; }
    function hash(): uint64 { return 1; }
    function clone(): self { return new Value(); }
}
"#;
    let concrete = format!(
        "{declarations} function main(): void {{ let $value = new Value(); echo $value->hash(); }}"
    );
    assert!(
        analyze(&concrete).diagnostics.is_empty(),
        "{:?}",
        analyze(&concrete).diagnostics
    );
    for body in [
        "let $left = new Value(); let $right = new Value(); echo $left == $right;",
        "Set<Value> $values = Set::from([]);",
        "List<Value> $values = [new Value(); 2];",
        "List<Value> $values = [new Value()]; let $other = new Value(); echo $values->contains($other);",
        "List<Value> $values = [new Value()]; let $copy = Deque::from($values);",
    ] {
        let source = format!("{declarations} function main(): void {{ {body} }}");
        let analysis = analyze(&source);
        assert!(!analysis.diagnostics.is_empty(), "{body}");
        assert!(analysis.diagnostics.iter().all(|diagnostic| diagnostic.code == "E0759"), "{body}: {:?}", analysis.diagnostics);
        assert!(doriac::lower_source("core.doria", &source).is_err());
    }
    let generic = format!("{declarations} function equal<T implements Equatable>(T $left, T $right): bool {{ return $left == $right; }} function main(): void {{ let $left = new Value(); let $right = new Value(); echo equal($left, $right); }}");
    assert!(
        analyze(&generic)
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0759"),
        "{:?}",
        analyze(&generic).diagnostics
    );
    let primitive = generic.replace(
        "let $left = new Value(); let $right = new Value();",
        "let $left = 1; let $right = 1;",
    );
    assert!(
        analyze(&primitive).diagnostics.is_empty(),
        "{:?}",
        analyze(&primitive).diagnostics
    );
}

#[test]
fn public_iteration_is_resolved_but_waits_for_slice_three() {
    let source = r#"
class Cursor implements Iterator<int> {
    function hasCurrent(): bool { return false; }
    function current(): int { return 0; }
    writable function advance(): void {}
}
class Values implements Iterable<int> {
    function iterator(): Iterator<int> { return new Cursor(); }
}
function main(): void { let $values = new Values(); foreach ($values as int $value) { echo $value; } }
"#;
    let analysis = analyze(source);
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0759"),
        "{:?}",
        analysis.diagnostics
    );
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|diagnostic| matches!(diagnostic.code, "E0758" | "E0759")),
        "{:?}",
        analysis.diagnostics
    );
    let builtin = analyze("function count<T implements Iterable<int>>(T $values): void {} function main(): void { List<int> $values = [1]; count($values); }");
    assert!(builtin.diagnostics.is_empty(), "{:?}", builtin.diagnostics);
}

#[test]
fn invalid_graphs_never_publish_checked_or_deferred_conformance() {
    for source in [
        "interface I {} class A implements I, I {} class B extends A {}",
        "interface I {} trait Broken { function __construct() {} } class A implements I { uses Broken; }",
    ] {
        let analysis = analyze(source);
        assert!(!analysis.diagnostics.is_empty());
        assert!(analysis.info.contracts.conformances.iter().all(|fact| fact.status == ConformanceStatus::Invalid), "{:?}", analysis.info.contracts);
    }
    let body = analyze("trait Broken { function answer(): int { return \"wrong\"; } }");
    assert!(
        body.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("int")),
        "{:?}",
        body.diagnostics
    );
}
#[test]
fn user_interface_constrained_calls_specialize_without_erasure() {
    let source = r#"
interface Renderable { function render(string $prefix): string; }
class Report implements Renderable {
    function render(string $prefix): string { return $prefix . "report"; }
}
function render<T implements Renderable>(T $value): string { return $value->render("typed "); }
function main(): void { let $report = new Report(); echo render($report); }
"#;
    let analysis = analyze(source);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let mir =
        doriac::lower_source_to_mir("constrained.doria", source).expect("static constrained call");
    assert_eq!(
        doriac::mir_interpreter::interpret(&mir).unwrap().stdout,
        b"typed report"
    );
    assert!(analysis
        .info
        .contracts
        .member_references
        .iter()
        .any(|reference| !reference.origins.is_empty()));
    let invalid = analyze(&source.replace("render(\"typed \")", "render(1)"));
    assert!(
        invalid
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("string")),
        "{:?}",
        invalid.diagnostics
    );
}

#[test]
fn declarations_and_direct_concrete_conformance_do_not_require_erasure() {
    let source = include_str!("../../../examples/native/main_stage35_concrete_conformance.doria");
    let analysis = analyze(source);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let conformance = &analysis.info.contracts.conformances[0];
    assert_eq!(conformance.status, ConformanceStatus::Checked);
    assert!(conformance.implementations[0].implementation.is_some());
    let hir =
        doriac::lower_source("contracts.doria", source).expect("concrete execution has no carrier");
    assert!(!hir
        .items
        .iter()
        .any(|item| matches!(item, doriac::hir::Item::Class(class) if class.name == "Renderable")));
    doriac::lower_source_to_mir("contracts.doria", source).expect("existing concrete MIR path");
}

#[test]
fn inherited_conformance_specializes_generic_requirements_and_concrete_defaults() {
    let source = include_str!("../../../examples/native/main_stage35_generic_conformance.doria");
    let analysis = analyze(source);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let mir =
        doriac::lower_source_to_mir("generic.doria", source).expect("existing specialization path");
    assert_eq!(
        doriac::mir_interpreter::interpret(&mir).unwrap().stdout,
        b"42\n2\n"
    );
}

#[test]
fn diamond_requirements_keep_canonical_origins_and_order() {
    let analysis = analyze(
        r#"
interface Root<T> { function first(T $value): int; }
interface Left<T> extends Root<T> { function left(): int; }
interface Right<T> extends Root<T> { function right(): int; }
interface Both<T> extends Left<T>, Right<T> { function last(): int; }
"#,
    );
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let both = analysis
        .info
        .contracts
        .interfaces
        .iter()
        .find(|interface| interface.name == "Both")
        .unwrap();
    assert_eq!(
        both.requirements
            .iter()
            .map(|requirement| requirement.name.as_str())
            .collect::<Vec<_>>(),
        ["first", "left", "right", "last"]
    );
    assert_eq!(both.requirements[0].origins.len(), 1);
}

#[test]
fn missing_and_incompatible_implementations_are_distinct() {
    for (method, code) in [
        ("", "E0754"),
        ("function render(int $value): int { return 1; }", "E0755"),
    ] {
        let analysis = analyze(&format!("interface Renderable {{ function render(string $value): string; }} class Report implements Renderable {{ {method} }}"));
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == code),
            "{:?}",
            analysis.diagnostics
        );
        assert_eq!(
            analysis.info.contracts.conformances[0].status,
            ConformanceStatus::Invalid
        );
    }
}

#[test]
fn recursive_generic_parents_terminate_before_specialization_expands() {
    for source in ["interface Loop<T> extends Loop<List<T>> {}", "interface First<T> extends Second<List<T>> {} interface Second<T> extends First<List<T>> {}"] {
        let analysis = analyze(source);
        assert!(analysis.diagnostics.iter().any(|diagnostic| diagnostic.code == "E0751"), "{:?}", analysis.diagnostics);
    }
}

#[test]
fn interface_parameters_receive_the_slice_two_boundary() {
    let source = "interface Renderable { function render(): string; } function renderReport(Renderable $report): string { return $report->render(); }";
    let analysis = analyze(source);
    assert_eq!(analysis.diagnostics.len(), 1, "{:?}", analysis.diagnostics);
    assert_eq!(analysis.diagnostics[0].code, "E0758");
    assert!(analysis.diagnostics[0].message.contains("Stage 35 Slice 2"));
    assert!(doriac::lower_source("contracts.doria", source).is_err());
}

#[test]
fn substituted_generic_contracts_share_override_compatibility() {
    let analysis = analyze(
        r#"
interface Relates<T> {}
interface Base<T> { function apply<U implements Relates<T>>(U $value): int; }
interface Child extends Base<int> { function apply<V implements Relates<int>>(V $value): int; }
class Concrete implements Child {
    function apply<W implements Relates<int>>(W $value): int { return 1; }
}
"#,
    );
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let child = analysis
        .info
        .contracts
        .interfaces
        .iter()
        .find(|interface| interface.name == "Child")
        .unwrap();
    assert_eq!(child.requirements[0].origins.len(), 2);
    assert!(analysis
        .info
        .contracts
        .conformances
        .iter()
        .all(|fact| fact.status == ConformanceStatus::Checked));
}

#[test]
fn exact_dynamic_self_coalesces_across_interface_origins() {
    let analysis = analyze(
        r#"
interface Left { function copy(): self; }
interface Right { function copy(): self; }
interface Both extends Left, Right {}
class Value implements Both { function copy(): self { return new Value(); } }
"#,
    );
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert_eq!(
        analysis.info.contracts.interfaces[2].requirements[0]
            .origins
            .len(),
        2
    );
}

#[test]
fn unused_traits_are_compile_time_declarations_and_composition_defers_conformance() {
    let declarations = r#"
interface Renderable { function render(): string; }
trait Formatting<T> {
    function render(): string { return "value"; }
    function dependency(T $value): void;
}
trait Nested { uses Formatting<int>; }
"#;
    assert!(analyze(declarations).diagnostics.is_empty());
    let source = format!("{declarations} class Value implements Renderable {{ uses Nested; function format(): string {{ return $this->render(); }} }}");
    let analysis = analyze(&source);
    assert_eq!(
        analysis
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.code)
            .collect::<Vec<_>>(),
        ["E0493"],
        "{:?}",
        analysis.diagnostics
    );
    assert_eq!(
        analysis.info.contracts.conformances[0].status,
        ConformanceStatus::DeferredComposition
    );
    assert!(analysis.info.contracts.conformances[0].implementations[0]
        .implementation
        .is_none());
}

#[test]
fn trait_parent_access_is_rejected_in_nested_expressions() {
    let analysis = analyze(
        r#"
trait Invalid {
    function format(): string {
        return "{parent::format()}";
    }
}
"#,
    );
    assert_eq!(analysis.diagnostics.len(), 1, "{:?}", analysis.diagnostics);
    assert_eq!(analysis.diagnostics[0].code, "E0756");
}

#[test]
fn nullable_return_narrowing_uses_the_existing_virtual_return_transport() {
    let source = r#"
open class Base { open function copy(): ?Base { return null; } }
class Child extends Base { override function copy(): Child { return new Child(); } }
function useBase(Base $value): void {
    let $copy = $value->copy();
    if ($copy != null) { echo "present\n"; }
}
function main(): void { useBase(new Child()); }
"#;
    let mir = doriac::lower_source_to_mir("nullable-contract.doria", source)
        .expect("covariant contract lowers");
    let result = doriac::mir_interpreter::interpret(&mir).expect("existing carrier transport");
    assert_eq!(result.stdout, b"present\n");
}

#[test]
fn nominal_generic_constraints_reject_structural_coincidence_and_primitives() {
    for implementation in [
        "class Value {}",
        "class Value { function render(): string { return \"value\"; } }",
    ] {
        let source = format!("interface Renderable {{ function render(): string; }} {implementation} class Holder<T implements Renderable> {{}} function main(): void {{ let $holder = new Holder<Value>(); }}");
        let analysis = analyze(&source);
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0535"),
            "{:?}",
            analysis.diagnostics
        );
    }
    let valid = analyze("interface Mark {} class Holder<T implements Mark> {} class Child extends Base {} open class Base implements Mark {} function main(): void { let $holder = new Holder<Child>(); }");
    assert!(valid.diagnostics.is_empty(), "{:?}", valid.diagnostics);
    let shadowed = analyze("interface Comparable {} function select<T implements Comparable>(T $value): int { return 1; } function main(): void { select(1); }");
    assert!(
        shadowed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0535"),
        "{:?}",
        shadowed.diagnostics
    );
}

#[test]
fn interface_value_matrix_resolves_types_then_stops_before_hir() {
    let declarations = "interface Renderable { function render(): string; } class Report implements Renderable { function render(): string { return \"report\"; } }";
    for body in [
        "Renderable $value = new Report(); echo $value->render();",
        "?Renderable $value = null; if ($value != null) { echo $value->render(); }",
        "List<Renderable> $values = [];",
        "?SharedReference<Renderable> $value = null;",
        "mixed $value = new Report(); if ($value is Renderable) { echo $value->render(); }",
        "mixed $value = new Report(); echo match ($value) { Renderable $report => $report->render(), default => \"other\" };",
    ] {
        let source = format!("{declarations} function main(): void {{ {body} }}");
        let analysis = analyze(&source);
        assert_eq!(analysis.diagnostics.iter().map(|diagnostic| diagnostic.code).collect::<Vec<_>>(), ["E0758"], "{body}: {:?}", analysis.diagnostics);
        assert!(doriac::lower_source("boundaries.doria", &source).is_err());
        assert!(doriac::compile_source_to_php("boundaries.doria", &source).is_err());
    }
    let invalid =
        analyze("interface Renderable {} function main(): void { Renderable $value = 1; }");
    assert!(
        invalid
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0760"),
        "{:?}",
        invalid.diagnostics
    );
}

#[test]
fn invalid_trait_dependencies_do_not_publish_valid_graphs() {
    let analysis =
        analyze("trait Consumer { uses Cycle<int>; } trait Cycle<T> { uses Cycle<List<T>>; }");
    assert!(analysis
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0751"));
    assert!(analysis
        .info
        .contracts
        .traits
        .iter()
        .all(|declaration| !declaration.valid));
}

#[test]
fn requirement_and_trait_attributes_stay_authored_once_in_every_schema() {
    let source = r#"
#[Attribute] class Tag {}
#[Tag] interface Renderable {
    #[Tag] function render(#[Tag] int $value): string;
}
#[Tag] trait Formatting {
    #[Tag] function format(#[Tag] int $value): string { return "value"; }
}
class Report implements Renderable { function render(int $value): string { return "report"; } }
"#;
    let analysis = analyze(source);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    // #[Attribute] declares the schema; only the six authored uses are applications.
    assert_eq!(analysis.info.attributes.applications.len(), 6);
    let one = doriac::metadata_source("attributes.doria", source).unwrap();
    let two = doriac::metadata_source_v2("attributes.doria", source).unwrap();
    let three = doriac::metadata_source_v3("attributes.doria", source).unwrap();
    assert_eq!(one.schema_version, 1);
    assert_eq!(two.schema_version, 2);
    assert_eq!(three.schema_version, 3);
    assert_eq!(one.applications.len(), 6);
    assert_eq!(two.applications.len(), 6);
    assert_eq!(three.applications.len(), 6);
}

#[test]
fn existing_core_contracts_participate_in_interface_inheritance() {
    let source = r#"
interface Named extends Displayable {}
class Name implements Named { function toString(): string { return "name"; } }
function main(): void { let $name = new Name(); echo $name; }
"#;
    let analysis = analyze(source);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert!(analysis
        .info
        .contracts
        .conformances
        .iter()
        .all(|fact| fact.status == ConformanceStatus::Checked));
    let mir = doriac::lower_source_to_mir("core-contracts.doria", source).unwrap();
    assert_eq!(
        doriac::mir_interpreter::interpret(&mir).unwrap().stdout,
        b"name"
    );
    let php = doriac::compile_source_to_php("core-contracts.doria", source).unwrap();
    assert!(php.contains("implements __DoriaDisplayable"));

    let analysis = analyze(
        "interface StorageError extends Error {} class Missing implements StorageError {} ",
    );
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0613"),
        "{:?}",
        analysis.diagnostics
    );
    let analysis = analyze("interface StorageError extends Error {} function propagate(take StorageError $error): void throws StorageError { throw $error; }");
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code == "E0758"),
        "{:?}",
        analysis.diagnostics
    );
    assert!(!analysis.diagnostics.is_empty());
}

#[test]
fn every_callable_substitution_axis_is_retained_in_conformance_facts() {
    use doriac::semantics::contracts::ContractMismatch as M;
    for (requirement, implementation, failure) in [
        (
            "function act(int $value): void;",
            "function act(): void {}",
            M::ParameterCount,
        ),
        (
            "function act(int $value): void;",
            "function act(int $other): void {}",
            M::ParameterNames,
        ),
        (
            "function act(int $value): void;",
            "function act(string $value): void {}",
            M::ParameterTypes,
        ),
        (
            "function act(Box $value): void;",
            "function act(take Box $value): void {}",
            M::ParameterOwnership,
        ),
        (
            "function act(): void;",
            "writable function act(): void {}",
            M::Receiver,
        ),
        (
            "function act(): int;",
            "function act(): string { return \"wrong\"; }",
            M::ReturnType,
        ),
        (
            "function act(Box $value): Box;",
            "function act(Box $value): Box { return $value; }",
            M::ReturnProvenance,
        ),
        (
            "function act<T>(T $value): void;",
            "function act<T implements Displayable>(T $value): void {}",
            M::GenericParameters,
        ),
        (
            "function act(): void;",
            "internal function act(): void {}",
            M::Accessibility,
        ),
        (
            "function act(): void;",
            "static function act(): void {}",
            M::StaticMethod,
        ),
        (
            "function act(): void;",
            "function act(): void throws Failure {}",
            M::CheckedEffects,
        ),
    ] {
        let source = format!("class Box {{}} class Failure implements Error {{ function __construct(string $message) {{}} }} interface Contract {{ {requirement} }} class Implementation implements Contract {{ {implementation} }}");
        let analysis = analyze(&source);
        let fact = analysis
            .info
            .contracts
            .conformances
            .iter()
            .find(|fact| fact.interface.name == "Contract")
            .unwrap();
        assert_eq!(
            fact.status,
            ConformanceStatus::Invalid,
            "{implementation}: {:?}",
            analysis.diagnostics
        );
        assert!(
            fact.implementations[0].failures.contains(&failure),
            "{implementation}: {fact:?}"
        );
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0755" && !diagnostic.related.is_empty()),
            "{:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn compatible_receiver_effect_and_default_narrowing_remain_concrete() {
    let source = r#"
class Failure implements Error { function __construct(string $message) {} }
interface Contract { writable function act(int $value): int throws Error; }
class Implementation implements Contract { function act(int $value = 7): int throws Failure { return $value; } }
function main(): void { let $value = new Implementation(); echo $value->act(); }
"#;
    let analysis = analyze(source);
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let analysis = analyze("interface Contract { function act(int $value): int; } function useContract(Contract $value): int { return $value->act(); }");
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("argument")),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn inherited_contract_conflicts_require_an_explicit_substitutable_redeclaration() {
    let declarations = "open class Base {} class Child extends Base {} interface Left { function make(): ?Base; } interface Right { function make(): Base; }";
    let invalid = analyze(&format!(
        "{declarations} interface Combined extends Left, Right {{}}"
    ));
    assert!(
        invalid
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0753"),
        "{:?}",
        invalid.diagnostics
    );
    let valid = analyze(&format!(
        "{declarations} interface Combined extends Left, Right {{ function make(): Child; }}"
    ));
    assert!(valid.diagnostics.is_empty(), "{:?}", valid.diagnostics);
    let requirement = &valid
        .info
        .contracts
        .interfaces
        .iter()
        .find(|interface| interface.name == "Combined")
        .unwrap()
        .requirements[0];
    assert_eq!(requirement.origins.len(), 3);
}

#[test]
fn exact_duplicate_parent_fixes_preserve_comments_and_source_identity() {
    let source = "interface Root<T> {} interface Child extends Root<int>, Root<int> {}";
    let analysis = analyze(source);
    let duplicate = analysis
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "E0752")
        .unwrap();
    let fix = duplicate
        .fix
        .as_ref()
        .expect("exact duplicate removal is safe");
    assert_eq!(fix.span.source, duplicate.span.source);
    let mut fixed = source.to_string();
    fixed.replace_range(fix.span.start..fix.span.end, &fix.replacement);
    assert!(analyze(&fixed).diagnostics.is_empty());
    let commented = analyze(
        "interface Root<T> {} interface Child extends Root<int>, /* reason */ Root<int> {}",
    );
    assert!(commented
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "E0752")
        .unwrap()
        .fix
        .is_none());
}

#[test]
fn invalid_contract_operations_are_language_errors_not_future_execution() {
    for source in [
        "interface Contract {} function main(): void { let $value = new Contract(); }",
        "trait Formatting {} function main(): void { let $value = new Formatting(); }",
    ] {
        let analysis = analyze(source);
        assert_eq!(analysis.diagnostics.len(), 1, "{:?}", analysis.diagnostics);
        assert_eq!(analysis.diagnostics[0].code, "E0750");
    }
    let analysis = analyze(
        "interface Contract {} function get(Contract $value): int { return $value->missing; }",
    );
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0303"),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn trait_adaptations_resolve_canonical_specializations_and_exact_method_tokens() {
    let source = "trait Formatting<T> { function format(T $value): string { return \"value\"; } } class Record { uses Formatting<int> { Formatting<int64>::format as text; } }";
    let analysis = analyze(source);
    assert_eq!(analysis.diagnostics.len(), 1, "{:?}", analysis.diagnostics);
    assert_eq!(analysis.diagnostics[0].code, "E0493");
    assert_eq!(analysis.info.contracts.member_references.len(), 1);
    let reference = &analysis.info.contracts.member_references[0];
    assert_eq!(&source[reference.span.start..reference.span.end], "format");
    assert_eq!(reference.origins.len(), 1);
    assert!(
        source[reference.origins[0].start..reference.origins[0].end].starts_with("function format")
    );
}

#[test]
fn decidably_invalid_adaptations_do_not_receive_composition_boundaries() {
    for adaptation in [
        "Formatting::format as text; Formatting::other as text;",
        "Formatting::format as format;",
        "Formatting::format insteadof Formatting;",
        "Formatting::value as text;",
        "Formatting::missing as text;",
    ] {
        let source = format!("trait Formatting {{ int $value = 1; function format(): string {{ return \"value\"; }} function other(): string {{ return \"other\"; }} }} class Record {{ uses Formatting {{ {adaptation} }} }}");
        let analysis = analyze(&source);
        assert!(!analysis.diagnostics.is_empty());
        assert!(
            analysis
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code == "E0757"),
            "{adaptation}: {:?}",
            analysis.diagnostics
        );
    }
    let analysis = analyze("trait Formatting<T implements Displayable> {} class Value {} class Record { uses Formatting<Value>; }");
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0535"),
        "{:?}",
        analysis.diagnostics
    );
    assert!(analysis.info.contracts.boundaries.is_empty());
}
