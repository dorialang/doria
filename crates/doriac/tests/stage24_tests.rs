use doriac::ast::{ClassMember, Item};

#[path = "common/execution_directory.rs"]
mod execution_directory;
#[path = "common/native_execution.rs"]
mod native_execution;
#[path = "common/php_execution.rs"]
mod php_execution;

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
fn compiler_known_constraints_are_checked_and_unknown_constraints_are_rejected() {
    doriac::check_source(
        "stage24-constraint-ok.doria",
        r#"
function constrained<T implements Displayable>(T $value): T { return $value; }
function main(): int { return constrained(42); }
"#,
    )
    .expect("compiler-known constraints should accept conforming type arguments");

    let unsatisfied = diagnostics(
        r#"
function constrained<T implements Comparable<T>>(T $value): T { return $value; }
function main(): float { return constrained(1.5); }
"#,
    );
    assert!(unsatisfied.iter().any(|diagnostic| {
        diagnostic.code == "E0535"
            && diagnostic.message.contains("float")
            && diagnostic.message.contains("Comparable")
    }));

    let errors = diagnostics(
        r#"
function constrained<T implements UserComparable>(T $value): T { return $value; }
function main(): void {}
"#,
    );
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0750" && diagnostic.message.contains("UserComparable")
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
fn null_safe_generic_calls_infer_the_method_result_before_lifting() {
    let source =
        include_str!("../../../examples/native/main_stage35_interface_null_safe_generics.doria");
    let program = doriac::lower_source_to_mir("null-safe-generics.doria", source).unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(
        output.stdout,
        include_bytes!(
            "fixtures/native_io/main_stage35_interface_null_safe_generics/expected_stdout"
        )
    );
    assert_eq!(output.exit_status, 0);
    let errors = diagnostics(
        "class Choice { function choose<T>(T $one, T $two): T { return $one; } } function bad(?Choice $choice): ?int { return $choice?->choose(1, \"wrong\"); }",
    );
    assert!(errors.iter().any(|diagnostic| diagnostic.code == "E0532"));
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
fn expected_results_preserve_argument_inference_and_allow_ordinary_upcasts() {
    let source = r#"
interface Named { function getValue(): int; }
class Entry implements Named { function getValue(): int { return 7; } }
class Relay {
    function pass<T>(T $value): T { return $value; }
    static function passStatic<T>(take T $value): T { return $value; }
}
function identity<T>(T $value): T { return $value; }
function owned<T>(take T $value): T { return $value; }
function accept(Named $value): int { return $value->getValue(); }
function main(): int {
    let $entry = new Entry();
    let $relay = new Relay();
    Named $first = owned(new Entry());
    ?Named $second = Relay::passStatic(new Entry());
    return accept($relay->pass($entry)) + accept(identity($entry));
}
"#;
    let program = doriac::lower_source_to_mir("generic-upcast.doria", source).unwrap();
    doriac::mir_validation::validate_program(&program).unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.exit_status, 14);

    let errors = diagnostics(&source.replace(
        "Named $first = owned(new Entry());",
        "List<Named> $first = owned([new Entry()]);",
    ));
    assert!(
        errors
            .iter()
            .any(|diagnostic| diagnostic.message.contains("List<")),
        "{errors:?}"
    );
    let errors = diagnostics(
        r#"
function pair<T>(T $first, T $second): T { return $first; }
function main(): int { return pair(1, "different"); }
"#,
    );
    assert!(
        errors.iter().any(|diagnostic| diagnostic.code == "E0532"),
        "{errors:?}"
    );
}

#[test]
fn result_context_does_not_rebind_argument_inferred_outer_type_parameters() {
    let source = r#"
class Relay {
    function pass<T>(T $value): T { return $value; }
    static function passStatic<T>(T $value): T { return $value; }
}
function identity<T>(T $value): T { return $value; }
function count<T>(List<T> $items): int { return $items->count; }
function wrong(List<string> $items): int { return $items->count; }
function relay<T>(List<T> $items): int {
    let $relay = new Relay();
    return count(identity($items)) + count($relay->pass($items)) + count(Relay::passStatic($items));
}
function main(): int { return relay([42]); }
"#;
    let program = doriac::lower_source_to_mir("generic-outer-context.doria", source).unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.exit_status, 3);
    assert!(doriac::check_source(
        "wrong-context.doria",
        source.replace(
            "return count(identity($items))",
            "return wrong(identity($items))"
        )
    )
    .is_err());
}

#[test]
fn mir_monomorphization_deduplicates_equal_type_sets() {
    let source = r#"
function identity<T>(T $value): T { return $value; }
function main(): int throws Doria\Std\Io\IoError
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
fn readonly_generic_parameters_are_tracked_as_potential_move_borrows() {
    let errors = diagnostics(
        r#"
function append<T>(writable List<T> $items, T $value): void
{
    $items->add($value);
}
function main(): void {}
"#,
    );
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0474"
            && diagnostic.message.contains("$value")
            && diagnostic.message.contains("cannot be given away")
    }));
}

#[test]
fn generic_inference_rejects_null_and_void_as_value_types() {
    let errors = diagnostics(
        r#"
function consume<T>(T $value): void {}
function nothing(): void {}
function main(): void
{
    consume(null);
    consume(nothing());
}
"#,
    );
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0538"
            && diagnostic.message.contains("from `null`")
            && diagnostic.message.contains("nullable expected type")
    }));
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0538" && diagnostic.message.contains("from a `void` expression")
    }));
}

#[test]
fn displayable_protocol_method_cannot_be_generic() {
    let errors = diagnostics(
        r#"
class Label implements Displayable
{
    function toString<T>(): string { return "label"; }
}
function main(): void {}
"#,
    );
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0463"
            && diagnostic
                .message
                .contains("incompatible `Displayable::toString`")
    }));
}

#[test]
fn expanding_recursive_specializations_fail_finitely() {
    let errors = doriac::lower_source_to_mir(
        "stage24-expanding-recursion.doria",
        r#"
function grow<T>(take T $value): void
{
    grow([$value]);
}
function main(): void { grow(1); }
"#,
    )
    .expect_err("recursive type growth has no finite monomorphization");
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0539"
            && diagnostic.message.contains("recursively expands")
            && diagnostic.message.contains("finite monomorphization")
    }));

    doriac::lower_source_to_mir(
        "stage24-bounded-recursion.doria",
        r#"
function normalize<T>(T $value): void
{
    normalize(1);
}
function main(): void { normalize("value"); }
"#,
    )
    .expect("a bounded type-changing recursive call should monomorphize finitely");
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
function countWrapped<T>(take T $value): int
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
fn php_backend_emits_concrete_generic_callable_instances() {
    let php = doriac::compile_source_to_php(
        "stage24-php.doria",
        r#"
function identity<T>(T $value): T { return $value; }
function main(): int { identity(true); return identity(42); }
"#,
    )
    .expect("checked generic instances should lower to PHP");
    assert!(php.contains("$value): bool"));
    assert!(php.contains("$value): int"));
    assert!(!php.contains("(T $value)"));
}
#[test]
fn inherited_generic_calls_keep_owner_arguments_out_of_method_inference() {
    let source = r#"
open class Parent<T> {
    function first<U>(T $owner, U $value): T { return $owner; }
    function second<U>(T $owner, U $value): U { return $value; }
}
class Child<U> extends Parent<U> {
    function inspect(U $owner): U { return $this->first($owner, 7); }
    function inspectSecond(U $owner): int { return $this->second($owner, 7); }
}
function main(): void {
    let $value = new Child<string>();
    echo $value->inspect("owner");
    echo $value->inspectSecond("owner");
}
"#;
    let mir = doriac::lower_source_to_mir("inherited-generic-identities.doria", source)
        .unwrap_or_else(|findings| panic!("{findings:#?}"));
    let output = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(output.stdout, b"owner7");
    assert_eq!(output.exit_status, 0);
    assert_eq!(output.stderr, b"");
    for &profile in doriac::backend::NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&mir, profile, "owner7");
    }
    let php = doriac::compile_source_to_php("inherited-generic-identities.doria", source).unwrap();
    let body = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").unwrap()
    );
    let script = php_execution::Script::new(&body);
    let output = script.command(&[]).output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stderr, b"");
    assert_eq!(output.stdout, b"owner7");
}

#[test]
fn virtual_generic_overrides_keep_concrete_owner_families_and_body_dependencies() {
    let source = r#"
class Box<T> {}
open class Base<T> {
    open function inspect<U>(T $owner, U $value): int { return 1; }
}
class Child<V> extends Base<V> {
    override function inspect<W>(V $owner, W $value): int {
        let $box = new Box<W>();
        return 2;
    }
}
function inspectInt(Base<int> $source): int { return $source->inspect(1, 7); }
function inspectString(Base<string> $source): int { return $source->inspect("source", true); }
function main(): void {
    echo inspectInt(new Child<int>());
    echo inspectString(new Child<string>());
}
"#;
    let mir = doriac::lower_source_to_mir("generic-virtual-owner-families.doria", source)
        .unwrap_or_else(|findings| panic!("{findings:#?}"));
    let interpreted = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(interpreted.exit_status, 0);
    assert_eq!(interpreted.stderr, b"");
    assert_eq!(interpreted.stdout, b"22");
    for &profile in doriac::backend::NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&mir, profile, "22");
    }
    let php =
        doriac::compile_source_to_php("generic-virtual-owner-families.doria", source).unwrap();
    let body = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").unwrap()
    );
    let script = php_execution::Script::new(&body);
    let output = script.command(&["-d", "ffi.enable=0"]).output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stderr, b"");
    assert_eq!(output.stdout, b"22");
}

/// Runs `source` on the interpreter, every native profile and PHP output,
/// expecting `stdout` and status 0 from each.
fn assert_runs_on_every_target(path: &str, source: &str, stdout: &str) {
    let mir = doriac::lower_source_to_mir(path, source)
        .unwrap_or_else(|findings| panic!("{findings:#?}"));
    let interpreted = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(
        (
            String::from_utf8_lossy(&interpreted.stdout).into_owned(),
            interpreted.exit_status
        ),
        (stdout.to_string(), 0),
        "interpreter: {:?}",
        interpreted.runtime_diagnostic
    );
    for &profile in doriac::backend::NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&mir, profile, stdout);
    }
    let php = doriac::compile_source_to_php(path, source).unwrap();
    let body = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").unwrap()
    );
    let output = php_execution::Script::new(&body)
        .command(&[])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), stdout);
    assert_eq!(output.stderr, b"");
}

const STATED_GENERICS: &str = r#"
class Box<T>
{
    function __construct(take T $value) {}

    function get(): T
    {
        return $this->value;
    }
}

class Factory
{
    static function make<T>(take T $value): Box<T>
    {
        return new Box<T>($value);
    }

    function wrap<T>(take T $value): Box<T>
    {
        return new Box<T>($value);
    }

    function plain(int $value): int
    {
        return $value;
    }
}

function none<T>(): ?T
{
    return null;
}

function pick<T, U>(T $left, U $right): U
{
    return $right;
}
"#;

#[test]
fn calls_state_type_arguments_on_every_target() {
    // Decision 0105, Amendment: Type Arguments At The Call. A type parameter
    // that only the return type names, free functions, static and instance
    // methods, a list with a nested `>>`, and List::map.
    let source = format!(
        r#"{STATED_GENERICS}
function main(): void
{{
    let $missing = none<int>();
    echo $missing ?? 7;
    echo " " . pick<int, string>(1, "two");
    echo " " . Factory::make<int>(5)->get();
    let $factory = new Factory();
    echo " " . $factory->wrap<string>("w")->get();
    echo " " . pick<int, Box<Box<int>>>(1, new Box<Box<int>>(new Box<int>(9)))->get()->get();
    List<int> $values = [1, 2, 3];
    List<int> $doubled = $values->map<int>(fn(int $value) => $value * 2);
    echo " " . $doubled[2];
    int $total = $values->reduce<int>(0, function (writable int $sum, int $value): void {{
        $sum += $value;
    }});
    echo " " . $total;
}}
"#
    );
    assert_runs_on_every_target("stated-type-arguments.doria", &source, "7 two 5 w 9 6 6");
}

#[test]
fn plain_names_that_are_values_keep_their_comparison_meaning() {
    // Decision 0105, Amendment: Type Arguments At The Call. `<` and `>` share
    // one precedence, so these read as comparisons when a name between them
    // is a value; name resolution keeps that reading.
    let source = r#"
const int LIMIT = 10;
const int MAX = 5;
const int B = 3;
const int C = 4;

class Counter
{
    function __construct(writable int $value) {}

    function size(): int
    {
        return $this->value;
    }
}

class Config
{
    const int MAX = 20;
}

function show(bool $left, bool $right): void
{
    echo $left . "," . $right . " ";
}

function main(): void
{
    let $counter = new Counter(4);
    let $flag = false;
    let $d = 1;
    echo $counter->value < LIMIT > ($flag);
    echo " ";
    echo $counter->value<LIMIT>($flag);
    echo " ";
    show($counter->value < B, C > ($d));
    show(left: $counter->value < B, right: C > ($d));
    bool[] $pair = [$counter->value < B, C > ($d)];
    echo "{$pair[0]}{$pair[1]} ";
    echo Config::MAX < LIMIT > ($flag);
    echo " ";
    echo MAX < LIMIT > ($flag);
}
"#;
    assert_runs_on_every_target(
        "comparison-reading.doria",
        source,
        "true true false,true false,true falsetrue false true",
    );
}

#[test]
fn stated_type_arguments_are_checked_against_the_callee() {
    for (body, code) in [
        // Too many, too few, and on a callee with no type parameters.
        ("echo pick<int>(1, 2);", "E0812"),
        ("echo pick<int, int, int>(1, 2);", "E0812"),
        ("echo plain<int>(1);", "E0811"),
        (
            "let $factory = new Factory(); echo $factory->plain<int>(1);",
            "E0811",
        ),
        // A stated argument fixes the parameter, so the argument must fit it.
        ("echo pick<int, string>(1, 2);", "E0408"),
        // A stated argument meets the parameter's constraints.
        ("echo sorted<float>(1.5);", "E0535"),
    ] {
        let source = format!(
            "{STATED_GENERICS}
function plain(int $value): int {{ return $value; }}
function sorted<T implements Comparable>(T $value): T {{ return $value; }}
function main(): void {{ {body} }}"
        );
        let found = diagnostics(&source);
        assert!(
            found.iter().any(|diagnostic| diagnostic.code == code),
            "{body}: expected {code}, found {found:#?}"
        );
    }
    let accepted = format!(
        "{STATED_GENERICS}
function main(): void {{ echo none<string>() ?? \"none\"; }}"
    );
    doriac::check_source("stated-type-arguments.doria", &accepted)
        .unwrap_or_else(|findings| panic!("{findings:#?}"));
}

#[test]
fn a_type_parameter_only_the_return_names_still_reports_when_nothing_states_it() {
    let source = format!(
        "{STATED_GENERICS}
function main(): void {{ let $missing = none(); }}"
    );
    let found = diagnostics(&source);
    let unresolved = found
        .iter()
        .find(|diagnostic| diagnostic.code == "E0531")
        .unwrap_or_else(|| panic!("{found:#?}"));
    assert!(
        unresolved
            .help
            .iter()
            .any(|help| help.contains("first<int>($values)")),
        "{unresolved:#?}"
    );
}

#[test]
fn tooling_reads_the_authored_tree_as_name_resolution_read_it() {
    // Decision 0105, Amendment: Type Arguments At The Call: the parser keeps
    // both readings of `callee<A>(...)`, name resolution chooses one, and the
    // authored program tooling receives holds only that reading.
    let source = format!(
        "{STATED_GENERICS}
const int LIMIT = 3;
function main(): void {{ echo none<int>() ?? 1; echo pick<int, bool>(1, 2<LIMIT>(false)); }}"
    );
    let (program, analysis) =
        doriac::analyze_source_for_ide("tooling-readings.doria", source).expect("source parses");
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.severity != doriac::diagnostics::DiagnosticSeverity::Error),
        "{:#?}",
        analysis.diagnostics
    );
    let tree = format!("{program:?}");
    assert!(!tree.contains("TypeArgumentsOrComparison"), "{tree}");
    assert!(tree.contains("type_arguments: Some"), "{tree}");
}
