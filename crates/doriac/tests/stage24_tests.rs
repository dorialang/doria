use doriac::ast::{ClassMember, Item};
use doriac::backend::BackendTarget;

fn diagnostics(source: &str) -> Vec<doriac::diagnostics::Diagnostic> {
    doriac::check_source("stage24.doria", source).expect_err("source should be rejected")
}

#[test]
fn parses_generic_function_and_method_parameter_lists() {
    let program = doriac::parse_source(
        "stage24-syntax.doria",
        r#"
function pair<T, U>(T $left, U $right): U { return $right; }
class Box {
    function constrained<T implements A, B>(T $value): T { return $value; }
    function composed<T implements A, U implements B>(T $left, U $right): U { return $right; }
}
"#,
    )
    .expect("Stage 24 generic parameter declarations should parse without errors");

    let Item::Function(pair) = &program.items[0] else {
        panic!("first declaration should be the generic function");
    };
    assert_eq!(
        pair.type_params
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<Vec<_>>(),
        ["T", "U"]
    );
    let Item::Class(class) = &program.items[1] else {
        panic!("second declaration should be the containing class");
    };
    let ClassMember::Method(method) = &class.members[0] else {
        panic!("class member should be the generic method");
    };
    assert_eq!(method.type_params[0].constraints.len(), 2);
    let ClassMember::Method(composed) = &class.members[1] else {
        panic!("second class member should be the composed generic method");
    };
    assert_eq!(composed.type_params.len(), 2);
    assert_eq!(composed.type_params[0].constraints.len(), 1);
    assert_eq!(composed.type_params[1].constraints.len(), 1);
}

#[test]
fn constraints_are_carried_but_named_as_pending_until_stage_35() {
    let errors = diagnostics(
        r#"
function constrained<T implements Displayable>(T $value): T { return $value; }
function main(): void {}
"#,
    );
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0533"
            && diagnostic.message.contains("constraint checking")
            && diagnostic.message.contains("Stage 35")
    }));
}

#[test]
fn inference_uses_arguments_and_typed_result_context() {
    let source = r#"
function first<T>(List<T> $items): ?T { return $items->first; }
function main(): int
{
    ?int $fromArgument = first([1]);
    ?int $fromExpectedResult = first([]);
    if ($fromArgument != null && $fromExpectedResult == null) { return $fromArgument; }
    return 0;
}
"#;
    let mir = doriac::lower_source_to_mir("stage24-inference.doria", source)
        .expect("argument and expected-result inference should lower");
    let output =
        doriac::mir_interpreter::interpret(&mir).expect("inferred specializations should execute");
    assert_eq!(output.exit_status, 1);
}

#[test]
fn unresolved_inference_points_to_a_typed_declaration() {
    let errors = diagnostics(
        r#"
function first<T>(List<T> $items): ?T { return $items->first; }
function main(): void { let $value = first([]); }
"#,
    );
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0531"
            && diagnostic.message.contains("cannot infer type parameter")
            && diagnostic
                .help
                .as_deref()
                .is_some_and(|help| help.contains("typed declaration"))
    }));
}

#[test]
fn mir_monomorphization_deduplicates_equal_type_sets() {
    let source = r#"
function identity<T>(T $value): T { return $value; }
function main(): int
{
    int $one = identity(1);
    int $two = identity(2);
    string $text = identity("three");
    echo $text;
    return $one + $two;
}
"#;
    let mir = doriac::lower_source_to_mir("stage24-dedup.doria", source)
        .expect("generic calls should monomorphize");
    assert_eq!(
        mir.functions
            .iter()
            .filter(|function| function.name == "identity")
            .count(),
        2,
        "two int calls share one instance while string gets a distinct instance"
    );
}

#[test]
fn inference_reuses_named_argument_binding() {
    let source = r#"
function choose<T, U>(T $first, U $second): U { return $second; }
function main(): int { return choose(second: 42, first: "ignored"); }
"#;
    let mir = doriac::lower_source_to_mir("stage24-named.doria", source)
        .expect("generic inference should consume named parameter binding");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("the named generic specialization should execute");
    assert_eq!(output.exit_status, 42);
}

#[test]
fn native_fixture_covers_free_instance_static_and_class_specializations() {
    let source = include_str!("../../../examples/native/main_stage24_generic_first.doria");
    let mir = doriac::lower_source_to_mir("stage24-native.doria", source)
        .expect("the Stage 24 native fixture should lower through shared MIR");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("the Stage 24 native fixture should execute");
    assert_eq!(output.exit_status, 0);
    assert_eq!(
        output.stdout,
        b"42\ngeneric\nclass\n7\nstatic\n3\n<drop:class>\n"
    );
    assert_eq!(
        mir.functions
            .iter()
            .filter(|function| function.name == "identity")
            .count(),
        1,
        "identical int calls should share one native specialization"
    );
}

#[test]
fn generic_take_parameters_preserve_use_after_move_checking() {
    let errors = diagnostics(
        r#"
class Token {}
function consume<T>(take T $value): void {}
function main(): void
{
    let $token = new Token();
    consume($token);
    consume($token);
}
"#,
    );
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0470" && diagnostic.message.contains("given away")
    }));
}

#[test]
fn generic_take_parameters_are_owned_inside_the_generic_body() {
    let errors = diagnostics(
        r#"
function sink<T>(take T $value): void {}
function twice<T>(take T $value): void
{
    sink($value);
    sink($value);
}
function main(): void {}
"#,
    );
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0470" && diagnostic.message.contains("given away")
    }));
}

#[test]
fn property_initializers_contribute_reachable_specializations() {
    let source = r#"
function identity<T>(T $value): T { return $value; }
class Box
{
    int $value = identity(42);
    function __construct() {}
}
function main(): int
{
    let $box = new Box();
    return $box->value;
}
"#;
    let mir = doriac::lower_source_to_mir("stage24-property-root.doria", source)
        .expect("property-initializer generic calls should collect their specialization");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("property-initializer generic specialization should execute");
    assert_eq!(output.exit_status, 42);
}

#[test]
fn inferred_collection_locals_are_interned_per_specialization() {
    let source = r#"
function countWrapped<T>(T $value): int
{
    let $items = [$value];
    return $items->count;
}
function main(): int { return countWrapped(42); }
"#;
    let mir = doriac::lower_source_to_mir("stage24-inferred-collection.doria", source)
        .expect("specialized inferred collection types should be interned");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("specialized inferred collection should execute");
    assert_eq!(output.exit_status, 1);
}

#[test]
fn borrowed_generic_collection_and_mixed_returns_retain_their_sources() {
    let source = r#"
function identity<T>(T $value): T { return $value; }
function count(List<int> $values): int { return identity($values)->count; }
function isInt(mixed $value): bool { return identity($value) is int; }
function main(): int
{
    List<int> $values = [41];
    mixed $value = 1;
    if (isInt($value)) { return count($values) + 41; }
    return 0;
}
"#;
    let mir = doriac::lower_source_to_mir("stage24-borrowed-move-return.doria", source)
        .expect("generic move-value borrows should survive MIR lowering");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("borrowed generic move-value returns should execute");
    assert_eq!(output.exit_status, 42);
}

#[test]
fn php_backend_rejects_generics_with_an_explicit_capability_diagnostic() {
    let errors = doriac::compile_source(
        "stage24-php.doria",
        r#"
function identity<T>(T $value): T { return $value; }
function main(): int { return identity(42); }
"#,
        BackendTarget::Php,
    )
    .expect_err("the PHP compatibility backend should reject native-only generics");

    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "B2401"
            && diagnostic
                .message
                .contains("generic function specialization")
            && diagnostic.message.contains("native target")
    }));
}
