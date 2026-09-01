use doriac::ast::{ClassMember, Item};
use doriac::mir;
use doriac::types::ResolvedType;

fn diagnostics(source: &str) -> Vec<doriac::diagnostics::Diagnostic> {
    doriac::check_source("stage34.doria", source).expect_err("source should be rejected")
}

fn assert_diagnostic(source: &str, code: &str) {
    let found = diagnostics(source);
    assert!(
        found.iter().any(|diagnostic| diagnostic.code == code),
        "expected {code}, got {found:#?}"
    );
}

fn lower(source: &str) -> mir::Program {
    doriac::lower_source_to_mir("stage34.doria", source)
        .unwrap_or_else(|diagnostics| panic!("Stage 34 source should lower: {diagnostics:#?}"))
}

fn interpret(source: &str) -> doriac::mir_interpreter::InterpreterOutput {
    doriac::mir_interpreter::interpret(&lower(source)).expect("Stage 34 MIR should interpret")
}

#[test]
fn parser_preserves_open_override_and_generic_parent_syntax() {
    let source = r#"
#[Marker]
internal open class Base<T> {}

#[Marker]
open class Child<T> extends Base<List<T>>
{
    #[Marker]
    open writable function update(writable T $value): void {}

    #[Marker]
    override function label(): string { return "child"; }
}
"#;
    let program = doriac::parse_source("stage34-parser.doria", source)
        .expect("Stage 34 declarations should parse");
    let classes = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Class(class) => Some(class),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(classes.len(), 2);
    assert!(classes[0].is_open);
    assert!(classes[0].open_span.is_some());
    assert_eq!(
        classes[1].parent.as_ref().map(ToString::to_string),
        Some("Base<List<T>>".to_string())
    );
    assert!(classes[1].extends_span.is_some());
    assert!(classes[1].parent_span.is_some());
    let methods = classes[1]
        .members
        .iter()
        .filter_map(|member| match member {
            ClassMember::Method(method) => Some(method),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(methods[0].is_open && methods[0].open_span.is_some());
    assert!(methods[1].is_override && methods[1].override_span.is_some());
    assert!(classes[1].modifier_prefix_span.start < classes[1].span.end);

    let parent_call = doriac::parse_source(
        "parent-call.doria",
        "open class A { function __construct() {} } class B extends A { function __construct() { parent::__construct(); } }",
    )
    .expect("parent constructor syntax should remain source preserving");
    assert_eq!(parent_call.items.len(), 2);
}

#[test]
fn hierarchy_validation_rejects_closed_cycles_invalid_overrides_and_constructor_protocols() {
    for (source, code) in [
        ("class Base {} class Child extends Base {}", "E0722"),
        ("open class Loop extends Loop {}", "E0724"),
        ("open class A extends B {} open class B extends A {}", "E0724"),
        ("class Root { open function f(): void {} }", "E0725"),
        ("open class Root { internal open function f(): void {} }", "E0725"),
        ("open class Root { open static function f(): void {} }", "E0725"),
        ("open class Root { open function f(): void {} } class Child extends Root { function f(): void {} }", "E0726"),
        ("open class Root { function f(): void {} } class Child extends Root { function f(): void {} }", "E0727"),
        ("open class Root { open function f(int $v): int { return $v; } } class Child extends Root { override function f(string $v): int { return 1; } }", "E0729"),
        ("open class Root {} class Child extends Root { override function missing(): void {} }", "E0730"),
        ("function main(): void { parent::missing(); }", "E0731"),
        ("open class Root { function __construct(int $v) {} } class Child extends Root {}", "E0732"),
        ("open class Root { function __construct(int $v) {} } class Child extends Root { function __construct() { let $x = 1; parent::__construct($x); } }", "E0733"),
        ("open class Root { function __construct() {} } class Child extends Root { function __construct() { parent::__construct(); parent::__construct(); } }", "E0734"),
        ("open class Root { internal function __construct() {} } class Child extends Root {}", "E0735"),
        ("open class Root { function __construct() {} } class Child extends Root { function other(): void { parent::__construct(); } }", "E0736"),
    ] {
        assert_diagnostic(source, code);
    }
}

#[test]
fn inherited_members_use_one_namespace_and_declaring_class_internal_access() {
    let valid = r#"
open class Base
{
    int $read = 1;
    writable int $write = 2;
    const int ANSWER = 40;
    static writable int $count = 1;
    function value(): int { return $this->read + $this->write; }
    static function answer(): int { return self::ANSWER; }
    internal function secret(): int { return 9; }
    function ownSecret(): int { return $this->secret(); }
}

class Child extends Base
{
    writable function total(): int
    {
        $this->write += 1;
        Child::count += 1;
        return $this->value() + Child::answer() + Child::ANSWER + Child::count + $this->ownSecret();
    }
}

function main(): void { let writable $child = new Child(); echo "{$child->total()}\n"; }
"#;
    let output = interpret(valid);
    assert_eq!(output.stdout, b"95\n");

    for source in [
        "open class A { internal int $x = 1; } class B extends A { function f(): int { return $this->x; } }",
        "open class A { internal function f(): int { return 1; } } class B extends A { function g(): int { return $this->f(); } }",
        "open class A { internal static int $x = 1; } class B extends A { function f(): int { return B::x; } }",
        "open class A { internal const int X = 1; } class B extends A { function f(): int { return B::X; } }",
    ] {
        let found = diagnostics(source);
        assert!(found.iter().any(|diagnostic| matches!(diagnostic.code, "E0303" | "E0304" | "E0488")), "{found:#?}");
    }

    doriac::check_source(
        "internal-shadow.doria",
        "open class A { internal int $x = 1; } class B extends A { int $x = 2; function value(): int { return $this->x; } }",
    )
    .expect("declaring-class-only internal members do not occupy the child namespace");
}

#[test]
fn override_contracts_preserve_defaults_receivers_returns_and_effect_subsets() {
    let valid = r#"
open class Animal {}
class Dog extends Animal {}
open class Failure implements Error { function __construct(string $message) {} }
class SpecificFailure extends Failure { function __construct() { parent::__construct("specific"); } }

open class Base
{
    open writable function mutate(int $amount = 2): Animal throws Failure { return new Animal(); }
}

class Child extends Base
{
    override function mutate(int $amount): Dog throws SpecificFailure { return new Dog(); }
}

function main(): void
{
    let writable $child = new Child();
    let $animal = $child->mutate();
}
"#;
    doriac::check_source("override-valid.doria", valid)
        .expect("compatible override and inherited default");

    for source in [
        "open class A { open function f(int $x = 1): int { return $x; } } class B extends A { override function f(int $x = 2): int { return $x; } }",
        "open class A { open function f(int $x): int { return $x; } } class B extends A { override function f(take int $x): int { return $x; } }",
        "open class A { open function f(int $x): int { return $x; } } class B extends A { override function f(int $y): int { return $y; } }",
        "open class A { open function f(): A { return new A(); } } class B extends A { override function f(): mixed { return 1; } }",
        "open class A { open function f(): mixed { return 1; } } class B extends A { override function f(): int { return 1; } }",
        "open class A { open function f(): ?A { return null; } } class B extends A { override function f(): B { return new B(); } }",
        "open class A { open function f(): void {} } class B extends A { override writable function f(): void {} }",
    ] {
        assert_diagnostic(source, "E0729");
    }
}

#[test]
fn covariant_virtual_returns_and_shadowed_initializers_execute_in_mir() {
    let covariant =
        include_str!("../../../examples/native/main_stage34_inheritance_covariant_virtual.doria");
    assert_eq!(interpret(covariant).stdout, b"covariant dog dog\n");

    let initializers = include_str!(
        "../../../examples/native/main_stage34_inheritance_shadowed_initializer.doria"
    );
    assert_eq!(
        interpret(initializers).stdout,
        b"initializers parent child\n"
    );
}

#[test]
fn automatic_effects_are_closed_over_the_entire_virtual_family() {
    let source =
        include_str!("../../../examples/native/main_stage34_inheritance_ambient_virtual.doria");
    let program = doriac::lower_source_to_mir("ambient-virtual.doria", source)
        .expect("automatic effects on an override must reach the root dispatch contract");
    let profiles = program
        .functions
        .iter()
        .filter(|function| {
            function.virtual_slot.is_some()
                && function
                    .method
                    .as_ref()
                    .is_some_and(|method| method.name == "report")
        })
        .map(|function| function.checked_effects.clone())
        .collect::<Vec<_>>();
    assert!(profiles.len() >= 2);
    assert!(!profiles[0].is_empty(), "{profiles:?}");
    assert!(profiles.iter().all(|profile| profile == &profiles[0]));
}

#[test]
fn parent_construction_virtual_dispatch_and_derived_destruction_are_ordered() {
    let source = r#"
open class Base
{
    static writable string $trace = "";
    int $baseValue = Base::mark("base-init ");

    function __construct(int $seed = 1)
    {
        Base::log("base-ctor ");
        $this->hook();
    }

    open function hook(): void { Base::log("base-hook "); }
    function callHook(): void { $this->hook(); }
    function __destruct(): void { Base::trace = Base::trace . "base-drop "; }
    static function mark(string $text): int { Base::trace = Base::trace . $text; return 1; }
    static function log(string $text): void { Base::trace = Base::trace . $text; }
}

class Child extends Base
{
    int $childValue = Base::mark("child-init ");
    function __construct() { parent::__construct(); Base::log("child-body "); $this->hook(); }
    override function hook(): void { Base::log("child-hook "); }
    function parentHook(): void { parent::hook(); }
    function __destruct(): void { Base::trace = Base::trace . "child-drop "; }
}

function main(): void
{
    {
        Base $value = new Child();
        $value->callHook();
        Child $exact = new Child();
        $exact->parentHook();
    }
    echo Base::trace . "\n";
}
"#;
    let output = interpret(source);
    assert_eq!(output.exit_status, 0);
    let text = String::from_utf8(output.stdout).expect("UTF-8 trace");
    assert!(
        text.starts_with(
            "base-init base-ctor base-hook child-init child-body child-hook child-hook "
        ),
        "{text}"
    );
    assert!(
        text.contains("base-hook child-drop base-drop child-drop base-drop"),
        "{text}"
    );
}

#[test]
fn upcasts_narrowing_mixed_collections_and_shared_payloads_keep_dynamic_identity() {
    let source = r#"
open class Shape
{
    open function name(): string { return "shape"; }
}
class Circle extends Shape
{
    override function name(): string { return "circle"; }
}

function inspect(Shape $shape): string
{
    if ($shape is Circle) { return $shape->name(); }
    return "other";
}

function main(): void
{
    Shape $shape = new Circle();
    ?Shape $maybe = new Circle();
    mixed $boxed = new Circle();
    List<Shape> $values = [new Circle()];
    SharedReference<Shape> $view = shared new Circle();
    let writable $maybeName = "other";
    let writable $mixedName = "other";
    if ($maybe != null) { $maybeName = inspect($maybe); }
    if ($boxed is Circle) { $mixedName = inspect($boxed); }
    echo "{inspect($shape)} {$maybeName} {$mixedName} {inspect($values[0])} {$view->name()}\n";
}
"#;
    let output = interpret(source);
    assert_eq!(output.stdout, b"circle circle circle circle circle\n");

    assert_diagnostic(
        "open class Shape {} class Circle extends Shape {} class Square extends Shape {} function f(): void { Square $value = new Circle(); }",
        "E0403",
    );
    assert_diagnostic(
        "open class Box<T> {} class IntBox extends Box<int> {} function f(): void { Box<string> $value = new IntBox(); }",
        "E0403",
    );
}

#[test]
fn error_hierarchy_coverage_and_dynamic_catches_use_ancestor_contracts() {
    let source = r#"
open class Failure implements Error
{
    function __construct(string $message) {}
}
class Missing extends Failure
{
    function __construct() { parent::__construct("missing"); }
}
class Other extends Failure
{
    function __construct() { parent::__construct("other"); }
}

function fail(): void throws Missing { throw new Missing(); }
function forward(): void throws Failure { fail(); }

function main(): void
{
    try { forward(); }
    catch (Failure $error) { echo "{$error->message}\n"; }
}
"#;
    let output = interpret(source);
    assert_eq!(output.stdout, b"missing\n");

    assert_diagnostic(
        r#"open class Failure implements Error { function __construct(string $message) {} } class Missing extends Failure { function __construct() { parent::__construct("missing"); } } function fail(): void throws Failure { throw new Failure("base"); } function main(): void { try { fail(); } catch (Missing) {} }"#,
        "E0629",
    );
    let unreachable = diagnostics(
        r#"open class Failure implements Error { function __construct(string $message) {} } class Missing extends Failure { function __construct() { parent::__construct("missing"); } } function fail(): void throws Missing { throw new Missing(); } function main(): void { try { fail(); } catch (Failure) {} catch (Missing) {} }"#,
    );
    assert!(
        unreachable
            .iter()
            .any(|diagnostic| diagnostic.code == "E0628"),
        "{unreachable:#?}"
    );
}

#[test]
fn mir_records_parent_prefixes_virtual_slots_and_open_carriers_deterministically() {
    let source = r#"
open class Base<T>
{
    int $base = 1;
    open function first(): int { return 1; }
    open function second(): int { return 2; }
}
open class Middle<T> extends Base<T>
{
    int $middle = 2;
    override function first(): int { return 10; }
}
class Leaf extends Middle<int>
{
    int $leaf = 3;
    override function second(): int { return 20; }
}
function main(): void { Base<int> $value = new Leaf(); echo "{$value->first()} {$value->second()}\n"; }
"#;
    let first = lower(source);
    let second = lower(source);
    assert_eq!(first, second);
    let base = first
        .classes
        .iter()
        .find(|class| class.name.starts_with("Base<"))
        .expect("base specialization");
    let middle = first
        .classes
        .iter()
        .find(|class| class.name.starts_with("Middle<"))
        .expect("middle specialization");
    let leaf = first
        .classes
        .iter()
        .find(|class| class.name == "Leaf")
        .expect("leaf class");
    assert!(base.is_open && middle.is_open && !leaf.is_open);
    assert_eq!(middle.parent, Some(base.id));
    assert_eq!(leaf.parent, Some(middle.id));
    assert_eq!(leaf.ancestors, vec![middle.id, base.id]);
    assert_eq!(base.virtual_methods.len(), 2);
    assert_eq!(middle.virtual_methods.len(), 2);
    assert_eq!(leaf.virtual_methods.len(), 2);
    assert!(middle.layout.size >= base.layout.size);
    assert!(leaf.layout.size >= middle.layout.size);
    assert_eq!(interpret(source).stdout, b"10 20\n");
}

#[test]
fn inherited_static_storage_and_constants_are_canonical_in_defaults_and_parent_calls() {
    let source = r#"
open class Base
{
    const int ANSWER = 40;
    static writable int $count = 1;
    static function value(): int { return self::ANSWER; }
}
class Child extends Base
{
    function bump(): int
    {
        Child::count += 1;
        parent::count += 1;
        return Child::value() + parent::value() + Child::count + Child::ANSWER;
    }
}
function answer(int $value = Child::ANSWER): int { return $value; }
function main(): void { let $child = new Child(); echo "{$child->bump()} {answer()}\n"; }
"#;
    assert_eq!(interpret(source).stdout, b"123 40\n");
}

#[test]
fn deep_hierarchies_and_many_virtual_slots_have_stable_linear_metadata() {
    let mut source = String::from("open class C0 { open function m0(): int { return 0; } }\n");
    for index in 1..32 {
        source.push_str(&format!(
            "open class C{index} extends C{} {{ override function m0(): int {{ return {index}; }} open function m{index}(): int {{ return {index}; }} }}\n",
            index - 1
        ));
    }
    source
        .push_str("function main(): void { C0 $value = new C31(); echo \"{$value->m0()}\\n\"; }\n");
    let mir = lower(&source);
    let last = mir
        .classes
        .iter()
        .find(|class| class.name == "C31")
        .expect("last class");
    assert_eq!(last.ancestors.len(), 31);
    assert_eq!(last.virtual_methods.len(), 32);
    assert_eq!(interpret(&source).stdout, b"31\n");
}

#[test]
fn semantic_hierarchy_facts_preserve_generic_ancestors_without_variance() {
    let source = r#"
open class Root<T> {}
open class Middle<T> extends Root<List<T>> {}
class Leaf extends Middle<int> {}
function main(): void { Root<List<int>> $root = new Leaf(); }
"#;
    let (_, analysis) =
        doriac::analyze_source_for_ide("stage34-facts.doria", source).expect("source should parse");
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}",
        analysis.diagnostics
    );
    let leaf = analysis
        .info
        .classes
        .iter()
        .find(|class| class.declaration_name == "Leaf")
        .expect("leaf facts");
    assert_eq!(leaf.ancestors.len(), 2);
    assert!(
        matches!(leaf.ancestors.last(), Some(parent) if parent.name == "Root" && matches!(parent.arguments.as_slice(), [ResolvedType::List(_)]))
    );
}
