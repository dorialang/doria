use doriac::ast::Item;
use doriac::backend::BackendTarget;

fn diagnostics(source: &str) -> Vec<doriac::diagnostics::Diagnostic> {
    doriac::check_source("stage25.doria", source).expect_err("source should be rejected")
}

#[test]
fn parses_generic_class_parameters_constraints_and_instantiations() {
    let program = doriac::parse_source(
        "stage25-syntax.doria",
        r#"
class Box<T, U implements Comparable<U>>
{
    T $left;
    U $right;
}
function accept(Box<int, string> $value): void {}
"#,
    )
    .expect("Stage 25 generic class syntax should parse without errors");

    let Item::Class(class) = &program.items[0] else {
        panic!("first declaration should be the generic class");
    };
    assert_eq!(class.type_params.len(), 2);
    assert_eq!(
        class.type_params[1].constraints[0].to_string(),
        "Comparable<U>"
    );
}

#[test]
fn generic_class_member_types_are_substituted_at_use_sites() {
    let mir = doriac::lower_source_to_mir(
        "stage25-check.doria",
        r#"
class Box<T>
{
    function __construct(take T $value) {}
}
function main(): int
{
    Box<int> $box = new Box<int>(42);
    return $box->value;
}
"#,
    )
    .expect("generic class construction and property access should lower");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("generic class construction should execute");
    assert_eq!(output.exit_status, 42);
}

#[test]
fn class_constraints_are_checked_at_instantiation() {
    let errors = diagnostics(
        r#"
class Sorted<T implements Comparable<T>> {}
function main(): void
{
    let $invalid = new Sorted<float>();
}
"#,
    );
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0535"
            && diagnostic.message.contains("float")
            && diagnostic.message.contains("Comparable")
    }));
}

#[test]
fn default_type_arguments_are_retained_for_a_decision_named_diagnostic() {
    let errors = diagnostics(
        r#"
class Box<T = int> {}
function main(): void {}
"#,
    );
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0534" && diagnostic.message.contains("decision 0105")
    }));
}

#[test]
fn generic_class_methods_are_specialized_per_class_instantiation() {
    let mir = doriac::lower_source_to_mir(
        "stage25-methods.doria",
        r#"
class Box<T>
{
    function __construct(take T $value) {}

    function get(): T
    {
        return $this->value;
    }

    function choose<U>(U $value): U
    {
        return $value;
    }
}
function main(): int
{
    let $number = new Box<int>(41);
    let $word = new Box<string>("generic");
    echo $word->get() . "\n";
    return $number->get() + $number->choose(1);
}
"#,
    )
    .expect("generic class and method specializations should lower");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("generic class and method specializations should execute");
    assert_eq!(output.stdout, b"generic\n");
    assert_eq!(output.exit_status, 42);
    assert_eq!(
        mir.classes
            .iter()
            .filter(|class| class.name.starts_with("Box<"))
            .count(),
        2
    );
}

#[test]
fn generic_class_drop_glue_uses_the_substituted_field_type() {
    let mir = doriac::lower_source_to_mir(
        "stage25-drop.doria",
        r#"
class Token
{
    function __construct(string $name) {}
    function __destruct() { echo "<drop:" . $this->name . ">\n"; }
}
class Box<T>
{
    function __construct(take T $value) {}
}
function main(): void
{
    let $number = new Box<int>(42);
    let $token = new Box<Token>(new Token("owned"));
}
"#,
    )
    .expect("generic class field drop glue should lower");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("generic class field drop glue should execute");
    assert_eq!(output.stdout, b"<drop:owned>\n");
}

#[test]
fn user_defined_constraints_are_deferred_to_stage_35() {
    let errors = diagnostics(
        r#"
class Box<T implements UserConstraint> {}
function main(): void {}
"#,
    );
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0533"
            && diagnostic.message.contains("UserConstraint")
            && diagnostic.message.contains("Stage 35")
    }));
}

#[test]
fn generic_class_instantiations_are_invariant_move_types() {
    let invariant = diagnostics(
        r#"
class Cat {}
class Animal {}
class Box<T> { function __construct(take T $value) {} }
function main(): void
{
    Box<Cat> $cats = new Box<Cat>(new Cat());
    Box<Animal> $animals = $cats;
}
"#,
    );
    assert!(invariant.iter().any(|diagnostic| {
        diagnostic.code == "E0403"
            && diagnostic.message.contains("Box<Cat>")
            && diagnostic.message.contains("Box<Animal>")
    }));

    let moved = diagnostics(
        r#"
class Box<T> { function __construct(take T $value) {} }
function consume(take Box<int> $value): void {}
function main(): void
{
    let $box = new Box<int>(42);
    consume($box);
    consume($box);
}
"#,
    );
    assert!(moved.iter().any(|diagnostic| {
        diagnostic.code == "E0470"
            && diagnostic
                .message
                .contains("after its value was given away")
    }));
}

#[test]
fn value_arguments_parse_then_receive_the_decision_0105_fence() {
    let errors = diagnostics(
        r#"
class Buffer<T> {}
function consume(Buffer<float32, 4096> $buffer): void {}
function main(): void {}
"#,
    );
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0536"
            && diagnostic.message.contains("decision 0105")
            && diagnostic.message.contains("4096")
    }));
    assert!(
        errors
            .iter()
            .all(|diagnostic| !diagnostic.code.starts_with('P')),
        "reserved value-argument syntax must pass the parser clock"
    );
}

#[test]
fn nested_generic_class_instantiations_round_trip_through_collections() {
    let mir = doriac::lower_source_to_mir(
        "stage25-nested.doria",
        r#"
class Pair<T, U>
{
    function __construct(take T $left, take U $right) {}
}
function main(): int
{
    writable List<Pair<int, string>> $pairs = [new Pair<int, string>(42, "answer")];
    Pair<int, string> $pair = $pairs->removeAt(0);
    echo $pair->right . "\n";
    return $pair->left;
}
"#,
    )
    .expect("nested generic class instantiations should lower");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("nested generic class instantiations should execute");
    assert_eq!(output.stdout, b"answer\n");
    assert_eq!(output.exit_status, 42);
}

#[test]
fn history_generic_class_specializes_collection_members() {
    let mir = doriac::lower_source_to_mir(
        "stage25-history.doria",
        r#"
class History<T>
{
    internal writable List<T> $entries = [];

    writable function push(take T $entry): void
    {
        $this->entries->add($entry);
    }

    writable function pop(): ?T
    {
        return $this->entries->pop();
    }
}
function main(): int
{
    writable History<int> $numbers = new History<int>();
    $numbers->push(42);
    ?int $number = $numbers->pop();
    if ($number != null) { return $number; }
    return 0;
}
"#,
    )
    .expect("History<int> should specialize its List<T> member");
    let output = doriac::mir_interpreter::interpret(&mir).expect("History<int> should execute");
    assert_eq!(output.exit_status, 42);
}

#[test]
fn native_fixture_covers_history_nested_layout_and_drop_specializations() {
    let source = include_str!("../../../examples/native/main_stage25_generic_classes.doria");
    let mir = doriac::lower_source_to_mir("stage25-native.doria", source)
        .expect("the Stage 25 native fixture should lower through shared MIR");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("the Stage 25 native fixture should execute");
    assert_eq!(output.exit_status, 0);
    assert_eq!(
        output.stdout,
        b"42\ngeneric\nnested:7\n1:2:1\ntrue:true\n<drop:history>\n"
    );
}

#[test]
fn class_body_instantiations_are_specialized_transitively() {
    let mir = doriac::lower_source_to_mir(
        "stage25-transitive.doria",
        r#"
class Inner<T>
{
}
class Outer<T>
{
    function pass(Inner<T> $inner): Inner<T>
    {
        return $inner;
    }
}
function main(): int
{
    let $outer = new Outer<int>();
    return 42;
}
"#,
    )
    .expect("class-body generic instantiations should specialize with their owner");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("transitively-specialized generic classes should execute");
    assert_eq!(output.exit_status, 42);
    assert!(mir.classes.iter().any(|class| class.name == "Inner<int>"));
}

#[test]
fn generic_callable_body_instantiations_are_specialized_at_the_call_site() {
    let mir = doriac::lower_source_to_mir(
        "stage25-callable-transitive.doria",
        r#"
class Box<T>
{
    function __construct(take T $value) {}
}
function discard<T>(take T $value): void
{
    let $box = new Box<T>($value);
}
function main(): int
{
    discard(42);
    return 7;
}
"#,
    )
    .expect("generic callable bodies should publish reached class instantiations");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("callable-body generic class instantiations should execute");
    assert_eq!(output.exit_status, 7);
    assert!(mir.classes.iter().any(|class| class.name == "Box<int>"));
}

#[test]
fn generic_callable_instantiations_follow_transitive_calls() {
    let mir = doriac::lower_source_to_mir(
        "stage25-callable-chain.doria",
        r#"
class Holder<T>
{
    function __construct(take T $value) {}
    function get(): T { return $this->value; }
}
function inner<U>(take U $value): Holder<U>
{
    return new Holder<U>($value);
}
function outer<T>(take T $value): Holder<T>
{
    return inner($value);
}
function main(): int
{
    let $holder = outer(42);
    return $holder->get();
}
"#,
    )
    .expect("concrete generic calls should specialize class templates in transitive callees");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("transitively-specialized generic callable bodies should execute");
    assert_eq!(output.exit_status, 42);
    assert!(mir.classes.iter().any(|class| class.name == "Holder<int>"));
}

#[test]
fn substituted_nested_class_instantiations_recheck_constraints() {
    let errors = diagnostics(
        r#"
class Sorted<T implements Comparable<T>> {}
class Outer<T>
{
    function create(): void
    {
        let $sorted = new Sorted<T>();
    }
}
function main(): void
{
    let $outer = new Outer<float>();
}
"#,
    );
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0535"
            && diagnostic.message.contains("float")
            && diagnostic.message.contains("Comparable")
    }));
}

#[test]
fn php_backend_rejects_generic_classes_explicitly() {
    let errors = doriac::compile_source(
        "stage25-php.doria",
        r#"
class Box<T> { function __construct(take T $value) {} }
function main(): void { let $box = new Box<int>(42); }
"#,
        BackendTarget::Php,
    )
    .expect_err("the PHP compatibility backend should reject generic classes");
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "B2401"
            && diagnostic.message.contains("generic class specialization")
            && diagnostic.message.contains("native target")
    }));
}

#[test]
fn generic_bodies_use_only_the_surface_guaranteed_by_constraints() {
    let mir = doriac::lower_source_to_mir(
        "stage25-constrained-body.doria",
        r#"
class Label implements Displayable
{
    function __construct(string $value) {}
    function toString(): string { return $this->value; }
}
class Plain
{
    function toString(): string { return "ordinary"; }
}
function render<T implements Displayable>(T $value): string
{
    return $value->toString();
}
function main(): void
{
    echo render(42) . "\n";
    echo render("text") . "\n";
    echo render(new Label("class")) . "\n";
    echo (new Plain())->toString() . "\n";
}
"#,
    )
    .expect("Displayable should expose toString inside a constrained generic body");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("constrained generic bodies should specialize without boxing");
    assert_eq!(output.stdout, b"42\ntext\nclass\nordinary\n");

    let errors = diagnostics(
        r#"
function invalid<T>(T $value): void
{
    $value->missing();
    echo $value->field;
}
function main(): void {}
"#,
    );
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0537" && diagnostic.message.contains("method `missing`")
    }));
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0537" && diagnostic.message.contains("property `field`")
    }));

    let unconstrained_operations = diagnostics(
        r#"
function compare<T>(T $left, T $right): bool
{
    Set<T> $values = [$left];
    return $left == $right;
}
function main(): void {}
"#,
    );
    assert!(unconstrained_operations.iter().any(|diagnostic| {
        diagnostic.code == "E0537"
            && diagnostic
                .message
                .contains("not guaranteed to implement `Hashable`")
    }));
    assert!(unconstrained_operations.iter().any(|diagnostic| {
        diagnostic.code == "E0537" && diagnostic.message.contains("equality is not guaranteed")
    }));
}

#[test]
fn generic_method_parameters_cannot_shadow_class_parameters() {
    let errors = diagnostics(
        r#"
class Box<T>
{
    function choose<T>(T $value): T { return $value; }
}
function main(): void {}
"#,
    );
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0530"
            && diagnostic.message.contains("shadows")
            && diagnostic.message.contains("Box::choose")
    }));
}

#[test]
fn compiler_known_constraint_arguments_are_substituted_and_checked() {
    doriac::check_source(
        "stage25-constraint-self.doria",
        r#"
function constrained<T implements Comparable<T>>(T $value): T { return $value; }
function main(): int { return constrained(42); }
"#,
    )
    .expect("the constrained type itself should satisfy Comparable<T>");

    let errors = diagnostics(
        r#"
function constrained<T implements Comparable<string>>(T $value): T { return $value; }
function main(): int { return constrained(42); }
"#,
    );
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0535"
            && diagnostic.message.contains("int")
            && diagnostic.message.contains("Comparable<string>")
    }));

    let class_errors = diagnostics(
        r#"
class Sorted<T implements Comparable<string>> {}
function main(): void { let $sorted = new Sorted<int>(); }
"#,
    );
    assert!(class_errors.iter().any(|diagnostic| {
        diagnostic.code == "E0535"
            && diagnostic.message.contains("int")
            && diagnostic.message.contains("Comparable<string>")
    }));
}

#[test]
fn comparable_constraints_enable_relational_operators() {
    let mir = doriac::lower_source_to_mir(
        "stage25-comparable.doria",
        r#"
function maximum<T implements Comparable<T>>(T $left, T $right): T
{
    if ($left >= $right) { return $left; }
    return $right;
}
function main(): int { return maximum(42, 7); }
"#,
    )
    .expect("Comparable<T> should guarantee relational operators");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("the specialized comparison should execute");
    assert_eq!(output.exit_status, 42);
}

#[test]
fn bool_comparable_specializations_use_the_canonical_false_before_true_order() {
    let mir = doriac::lower_source_to_mir(
        "stage25-bool-comparable.doria",
        r#"
function atLeast<T implements Comparable<T>>(T $left, T $right): bool
{
    return $left >= $right;
}
function main(): int
{
    if (atLeast(true, false) && false < true && true >= true) { return 42; }
    return 1;
}
"#,
    )
    .expect("bool Comparable specializations should lower ordered comparisons");
    let output =
        doriac::mir_interpreter::interpret(&mir).expect("bool ordered comparisons should execute");
    assert_eq!(output.exit_status, 42);
}

#[test]
fn recursively_expanding_class_specializations_are_rejected() {
    let errors = diagnostics(
        r#"
class Node<T>
{
    ?Node<List<T>> $next = null;
}
function main(): void
{
    let $node = new Node<int>();
}
"#,
    );
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0539"
            && diagnostic.message.contains("Node")
            && diagnostic.message.contains("no finite monomorphization")
    }));
}

#[test]
fn self_types_and_static_calls_preserve_the_enclosing_specialization() {
    let source = r#"
class Box<T>
{
    function __construct(take T $value) {}

    static function identity(T $value): T { return $value; }

    function same(): self { return $this; }

    function choose(T $value): T
    {
        return self::identity($value);
    }
}
function main(): int
{
    let $box = new Box<int>(40);
    return $box->value + $box->choose(2);
}
"#;
    let hir = doriac::lower_source("stage25-self.doria", source)
        .expect("self should retain Box<T> through HIR");
    let doriac::hir::Item::Class(class) = &hir.items[0] else {
        panic!("expected generic class");
    };
    let same = class
        .members
        .iter()
        .find_map(|member| match member {
            doriac::hir::ClassMember::Method(method) if method.name == "same" => Some(method),
            _ => None,
        })
        .expect("same method");
    assert_eq!(
        same.return_type
            .as_ref()
            .map(ToString::to_string)
            .as_deref(),
        Some("Box<T>")
    );

    let mir = doriac::lower_source_to_mir("stage25-self.doria", source)
        .expect("self should retain Box<T> through HIR and static call lowering");
    let output =
        doriac::mir_interpreter::interpret(&mir).expect("specialized self calls should execute");
    assert_eq!(output.exit_status, 42);
}

#[test]
fn self_static_properties_resolve_against_each_enclosing_specialization() {
    let mir = doriac::lower_source_to_mir(
        "stage25-self-static-property.doria",
        r#"
class Counter<T>
{
    static writable int $count = 0;

    function next(): int
    {
        self::count = self::count + 1;
        return self::count;
    }
}
function main(): int
{
    let $numbers = new Counter<int>();
    let $words = new Counter<string>();
    if ($numbers->next() == 1 && $numbers->next() == 2 && $words->next() == 1) {
        return 42;
    }
    return 1;
}
"#,
    )
    .expect("self static properties should retain the enclosing class specialization");
    let output = doriac::mir_interpreter::interpret(&mir)
        .expect("specialized self static properties should execute");
    assert_eq!(output.exit_status, 42);
}

#[test]
fn ownership_checks_class_type_parameters_in_method_signatures() {
    let errors = diagnostics(
        r#"
class Token
{
    function __construct(string $name) {}
}
class Sink<T>
{
    function consume(take T $value): void {}
}
function main(): void
{
    let $token = new Token("owned");
    let $sink = new Sink<Token>();
    $sink->consume($token);
    echo $token->name;
}
"#,
    );
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0470"
            && diagnostic
                .message
                .contains("after its value was given away")
    }));
}

#[test]
fn ownership_treats_class_type_parameter_properties_as_potential_move_values() {
    let errors = diagnostics(
        r#"
function discard<T>(take T $value): void {}
class Box<T>
{
    function __construct(take T $value) {}
    function discardValue(): void { discard($this->value); }
}
function main(): void {}
"#,
    );
    assert!(errors.iter().any(|diagnostic| {
        diagnostic.code == "E0472"
            && diagnostic
                .message
                .contains("direct moves out of owned properties")
    }));
}
