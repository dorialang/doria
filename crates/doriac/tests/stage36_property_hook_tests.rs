#[path = "common/native_execution.rs"]
mod native_execution;

fn lower_hook_integration_hir(source: &str) -> doriac::hir::Program {
    doriac::lower_source("hooks.doria", source)
        .unwrap_or_else(|diagnostics| panic!("{diagnostics:#?}"))
}

fn assert_hook_execution(source: &str, stdout: &str) {
    let hir = lower_hook_integration_hir(source);
    let mir = doriac::mir_lowering::lower_program(&hir).unwrap();
    doriac::mir_validation::validate_program(&mir)
        .unwrap_or_else(|error| panic!("{error:?}\n{mir}"));
    let result =
        doriac::mir_interpreter::interpret(&mir).unwrap_or_else(|error| panic!("{error:?}\n{mir}"));
    assert_eq!(result.stdout, stdout.as_bytes());
    assert_eq!(result.stderr, b"");
    assert_eq!(result.exit_status, 0);
    assert!(result.runtime_diagnostic.is_none());
    native_execution::assert_native_execution(&mir, doriac::backend::NativeProfile::Fast, stdout);
    #[cfg(feature = "llvm-backend")]
    native_execution::assert_native_execution(
        &mir,
        doriac::backend::NativeProfile::Release,
        stdout,
    );
}

#[test]
fn mutated_callback_collections_preserve_checked_error_transport() {
    assert_hook_execution(
        include_str!("../../../examples/native/main_collection_callback_mutation.doria"),
        "caught: direct;helper;capture;array;dictionary;deque;\n",
    );
}

#[test]
fn void_arrow_callback_failures_preserve_php_transport() {
    assert_hook_php_execution(
        r#"
function fail(): void { read_file("missing-stage36-callback-input.txt"); }
function appendCallback(writable List<function(): void> $callbacks): void {
    $callbacks->add(fn() => fail());
}
function main(): void {
    writable List<function(): void> $callbacks = [function(): void {}];
    appendCallback($callbacks);
    foreach ($callbacks as function(): void $callback) {
        try { $callback(); } catch (Error) { echo "caught\n"; }
    }
}
"#,
        "caught\n",
    );
}

fn assert_hook_php_execution(source: &str, stdout: &str) {
    assert_hook_execution(source, stdout);
    let hir = lower_hook_integration_hir(source);
    let mir = doriac::mir_lowering::lower_program(&hir).unwrap();
    let php = doriac::codegen_php::generate(&hir, Some(&mir)).unwrap();
    let Ok(version) = std::process::Command::new("php").arg("--version").output() else {
        eprintln!("PHP unavailable; hook compatibility execution skipped");
        return;
    };
    assert!(version.status.success());
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").unwrap()
    );
    let run = std::process::Command::new("php")
        .args(["-r", &script])
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        run.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(run.stdout, stdout.as_bytes());
}

#[test]
fn setter_input_type_matches_the_resolved_property_type_in_every_owner() {
    for source in [
        "class Value { writable float $amount { set (string $value) {} } }",
        "interface Value { writable float $amount { set (string $value); } }",
        "trait Values { writable float $amount { set (string $value) {} } }",
        "class Value<T> { writable T $amount { set (int $value) {} } }",
        "class Value { writable ?int $amount { set (int $value) {} } }",
        "class Value { writable int8 $amount { set (int16 $value) {} } }",
        "class Value { writable float32 $amount { set (float64 $value) {} } }",
    ] {
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0769"),
            "{source}: {:?}",
            analysis.diagnostics,
        );
    }
    for source in [
        "class Value { writable int $amount { set (int64 $value) {} } }",
        "class Value { writable float $amount { set (float64 $value) {} } }",
        "class Value<T> { writable T $amount { set (T $value) {} } }",
        "interface Value<T> { writable T $amount { set (T $value); } }",
        "trait Values<T> { writable T $amount { set (T $value) {} } }",
        "class Value { writable ?int $amount { set (?int $value) {} } }",
    ] {
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
        assert!(
            analysis.diagnostics.is_empty(),
            "{source}: {:?}",
            analysis.diagnostics,
        );
    }
}

#[test]
fn backed_overrides_share_the_root_field_and_initialize_each_object_once() {
    let source = r#"
class Counts { static writable float $initializers = 0.0; }
function initial(): float { Counts::initializers += 1.0; return 3.0; }
open class Base {
    open writable float $value = initial() {
        get => $this->value;
        set (float $next) => $this->value = $next;
    }
}
open class Middle extends Base {
    override writable float $value {
        get => 99.0;
        set (float $next) {}
    }
}
class Derived extends Middle {
    override writable float $value {
        get => $this->value + 10.0;
        set (float $next) => $this->value = $next + 2.0;
    }
}
function read(Base $object): float { return $object->value; }
function main(): void {
    let writable $first = new Derived();
    let $second = new Derived();
    if (read($first) == 13.0 && read($second) == 13.0 && Counts::initializers == 2.0) {
        echo "initialized once per instance\n";
    }
    $first->value = 5.0;
    if (read($first) == 17.0 && read($second) == 13.0) {
        echo "independent inherited storage\n";
    }
}
"#;
    let hir = lower_hook_integration_hir(source);
    let fields = &hir.semantic_info.property_backing_fields;
    assert_eq!(fields.len(), 3);
    let root = fields.values().next().unwrap();
    assert_eq!(root.declaring_class, "Base");
    assert!(fields.values().all(|field| field == root));
    for class in
        hir.semantic_info.classes.iter().filter(|class| {
            ["Base", "Middle", "Derived"].contains(&class.declaration_name.as_str())
        })
    {
        assert_eq!(class.properties.len(), 1, "{}", class.name);
        assert_eq!(class.properties[0].declaring_class, "Base");
    }
    assert_hook_php_execution(
        source,
        "initialized once per instance\nindependent inherited storage\n",
    );
}

#[test]
fn shared_inherited_backing_destroys_each_owned_value_exactly_once() {
    assert_hook_php_execution(
        r#"
class Trace { static writable string $events = ""; }
class Item {
    function __construct(string $name) {}
    function __destruct() { Trace::events = Trace::events . $this->name . ";"; }
}
open class Base {
    open writable Item $item = new Item("root") {
        borrowed get => $this->item;
        set (take Item $next) => $this->item = $next;
    }
}
class Child extends Base {
    override writable Item $item {
        borrowed get => $this->item;
        set (take Item $next) => $this->item = $next;
    }
}
function main(): void {
    {
        let writable $first = new Child();
        let $second = new Child();
        $first->item = new Item("changed");
        echo "{$first->item->name} {$second->item->name}\n";
    }
    echo Trace::events;
}
"#,
        "changed root\nroot;root;changed;",
    );
}

#[test]
fn backed_override_cannot_narrow_the_shared_physical_field_type() {
    let source = r#"
open class Item {}
class SpecialItem extends Item {}
open class Base {
    open Item $value = new Item() { get => $this->value; }
}
class Derived extends Base {
    override SpecialItem $value { get => $this->value; }
}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "E0729" && diagnostic.message.contains("inherited field type")
        }),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn generic_backed_overrides_keep_specialized_root_storage() {
    let source = r#"
open class Base<T> {
    open writable ?T $value = null {
        get => $this->value;
        set (take ?T $next) => $this->value = $next;
    }
}
class Derived extends Base<int> {
    override writable ?int $value {
        get => $this->value;
        set (take ?int $next) => $this->value = $next;
    }
}
function main(): void {
    let writable $first = new Derived();
    let $second = new Derived();
    $first->value = 42;
    echo ($first->value ?? -1) . " " . ($second->value ?? -1) . "\n";
}
"#;
    assert_hook_php_execution(source, "42 -1\n");
}

#[test]
fn hooks_reject_blocking_work_even_when_helpers_catch_the_errors() {
    for source in [
        r#"class Value { int $number { get { echo "blocked"; return 1; } } }"#,
        r#"class Value { writable int $number { set (int $value) { try { write_stderr("blocked"); } catch (Error) {} } } }"#,
        r#"function output(): int { try { echo "blocked"; } catch (Error) {} return 1; }
           function helper(): int { return output(); }
           class Value { int $number { get => helper(); } }"#,
        r#"function first(bool $again): int { if ($again) { return second(); } echo "blocked"; return 1; }
           function second(): int { return first(false); }
           class Value { int $number { get => first(true); } }"#,
        r#"function output(): int { echo "blocked"; return 1; }
           class Value { int $number { get { let $action = fn() => output(); return $action(); } } }"#,
        r#"function output(): int { echo "blocked"; return 1; }
           function invoke(function(): int $action): int { return $action(); }
           class Value { int $number { get => invoke(fn() => output()); } }"#,
        r#"function output(): int { echo "blocked"; return 1; }
           function make(): function(): int { return fn() => output(); }
           class Value { int $number { get { let $action = make(); return $action(); } } }"#,
        r#"function output(): int { echo "blocked"; return 1; }
           class Value { function(): int $action = fn() => output(); int $number { get => $this->action(); } }"#,
        r#"class Helper { function __construct() { echo "blocked"; } }
           class Value { Helper $helper { get => new Helper(); } }"#,
    ] {
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0770"),
            "{source}: {:?}",
            analysis.diagnostics
        );
        assert!(
            analysis
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code == "E0770"),
            "{source}: {:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn hooks_distinguish_callback_creation_from_nonblocking_invocation() {
    for source in [
        r#"class Value { string $text { get => sprintf("%d", 7); } }"#,
        r#"function output(): int { echo "later"; return 1; }
           class Value { function(): int $action { get => fn() => output(); } }"#,
        r#"class Value { int $number { get { let $action = fn() => 7; return $action(); } } }"#,
        r#"function invoke(function(): int $action): int { return $action(); }
           class Value { int $number { get => invoke(fn() => 7); } }"#,
        r#"function make(): function(): int { return fn() => 7; }
           class Value { int $number { get { let $action = make(); return $action(); } } }"#,
        r#"class Value { function(): int $action = fn() => 7; int $number { get => $this->action(); } }"#,
    ] {
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
        assert!(
            analysis.diagnostics.is_empty(),
            "{source}: {:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn nonblocking_callback_proofs_do_not_mix_independent_calls_or_environments() {
    for helper in [
        "function wrap(take function(): int $action): function(): int { return $action; }",
        "function wrap(take function(): int $action): function(): int { return fn() with (take $action) => $action(); }",
    ] {
        let source = format!(r#"
function output(): int {{ echo "outside the hook"; return 0; }}
function invoke(function(): int $action): int {{ return $action(); }}
{helper}
class Value {{
    int $number {{ get {{
        let $pure = wrap(fn() => 7);
        return invoke($pure);
    }} }}
}}
function other(): int {{
    let $noisy = wrap(fn() => output());
    return invoke($noisy);
}}
"#);
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
        assert!(analysis.diagnostics.is_empty(), "{source}: {:?}", analysis.diagnostics);
    }
}

#[test]
fn nonblocking_proof_follows_callback_values_through_control_flow_and_collections() {
    for body in [
        "return when (true): int { return action(); } else { return 0; };",
        "let $callback = when (true): function(): int { return fn() => action(); } else { return fn() => 0; }; return $callback();",
        "List<function(): int> $callbacks = [fn() => action()]; foreach ($callbacks as function(): int $callback) { return $callback(); } return 0;",
        "List<function(): int> $callbacks = [fn() => action()]; let $mapped = $callbacks->map(fn(function(): int $callback) => $callback()); return $mapped[0];",
        "let $work = Work::Run(fn() => action()); return match (take $work) { Work::Run($callback) => $callback() };",
    ] {
        for (operation, blocks) in [("echo \"blocked\";", true), ("", false)] {
            let source = format!(
                "enum Work {{ case Run(function(): int $callback); }}
                 function action(): int {{ {operation} return 7; }}
                 class Value {{ int $number {{ get {{ {body} }} }} }}"
            );
            let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
            assert_eq!(analysis.diagnostics.iter().any(|diagnostic| diagnostic.code == "E0770"), blocks, "{source}: {:?}", analysis.diagnostics);
            assert!(analysis.diagnostics.iter().all(|diagnostic| diagnostic.code == "E0770"), "{source}: {:?}", analysis.diagnostics);
        }
    }
}

#[test]
fn hook_blocking_proof_includes_setter_arguments_and_dynamic_dispatch() {
    for operation in ["echo \"blocked\";", ""] {
        for source in [
            format!(
                r#"
function action(): int {{ {operation} return 7; }}
class Sink {{ writable function(): int $callback {{ set (take function(): int $next) {{ $next(); }} }} }}
class Value {{ int $number {{ get {{
    let writable $sink = new Sink();
    $sink->callback = fn() => action();
    return 7;
}} }} }}
"#
            ),
            format!(
                r#"
interface Action {{ function run(): int; }}
class Worker implements Action {{ function run(): int {{ {operation} return 7; }} }}
class Value {{ function __construct(take Action $action) {{}} int $number {{ get => $this->action->run(); }} }}
"#
            ),
        ] {
            let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
            assert_eq!(
                analysis
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "E0770"),
                !operation.is_empty(),
                "{source}: {:?}",
                analysis.diagnostics
            );
            assert!(
                analysis
                    .diagnostics
                    .iter()
                    .all(|diagnostic| diagnostic.code == "E0770"),
                "{source}: {:?}",
                analysis.diagnostics
            );
        }
    }
}

#[test]
fn known_callback_origins_do_not_erase_unresolved_incoming_paths() {
    let source = r#"
class Value {
    function __construct(take function(): int $callback) {}
    int $number { get => $this->callback(); }
}
function known(): Value { return new Value(fn() => 7); }
function unknown(take function(): int $callback): Value { return new Value($callback); }
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0770"),
        "{:?}",
        analysis.diagnostics
    );
    assert!(
        analysis
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code == "E0770"),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn hook_cannot_assume_an_unknown_interface_method_is_nonblocking() {
    let source = "interface Action { function run(): int; } class Value { function __construct(take Action $action) {} int $number { get => $this->action->run(); } }";
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0770"),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn hook_dispatch_does_not_include_an_unrelated_sibling_override() {
    let source = r#"
open class Base { open function evaluate(): int { return 0; } }
class Pure extends Base { override function evaluate(): int { return 7; } }
class Noisy extends Base { override function evaluate(): int { echo "outside"; return 1; } }
class Value {
    Pure $source = new Pure();
    int $number { get => $this->source->evaluate(); }
}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn generic_hook_cleanup_is_checked_for_every_concrete_property_type() {
    for (destructor, blocks) in [
        ("", false),
        ("try { echo \"drop\"; } catch (Error) {}", true),
    ] {
        let source = format!(
            r#"
class Item {{ function __destruct() {{ {destructor} }} }}
class Sink<T> {{ writable T $item {{ set(take T $value) {{}} }} }}
function useSink(Sink<Item> $objects, Sink<int> $numbers): void {{}}
"#
        );
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
        assert_eq!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0770"),
            blocks,
            "{source}: {:?}",
            analysis.diagnostics
        );
        assert!(
            analysis
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code == "E0770"),
            "{source}: {:?}",
            analysis.diagnostics
        );
    }
}
#[test]
fn generic_helper_cleanup_keeps_unrelated_specializations_separate() {
    for (hook_type, blocks) in [("Quiet", false), ("Noisy", true)] {
        let source = format!(
            r#"
class Quiet {{}}
class Noisy {{ function __destruct() {{ try {{ echo "drop"; }} catch (Error) {{}} }} }}
function discard<T>(take T $value): int {{ return 7; }}
class Value {{ int $number {{ get => discard(new {hook_type}()); }} }}
function other(): int {{ return discard(new Noisy()); }}
"#
        );
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
        assert_eq!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0770"),
            blocks,
            "{source}: {:?}",
            analysis.diagnostics
        );
        assert!(
            analysis
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code == "E0770"),
            "{source}: {:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn repeatable_callback_invocation_preserves_capture_cleanup_for_its_owner() {
    let source = r#"
class Item {
    int $value = 42;
    function __destruct() { try { echo "drop"; } catch (Error) {} }
}
class Factory {
    function(): int $callback {
        get {
            let $item = new Item();
            let $callback = fn() with (take $item) => $item->value;
            if ($callback() == 42) { return $callback; }
            return $callback;
        }
    }
}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:#?}\n{:#?}",
        analysis.diagnostics,
        analysis.info.cleanup
    );
}

#[test]
fn enum_callback_effects_preserve_case_and_field_identity_through_nested_calls() {
    for (selected, blocks) in [("first", false), ("second", true)] {
        let source = format!(
            r#"
enum Pair {{ case Values(function(): int $first, function(): int $second); }}
enum Wrapped {{ case Value(Pair $pair); }}
function identity(take Wrapped $value): Wrapped {{ return $value; }}
function select(take Wrapped $value): function(): int {{
    return match (take $value) {{
        Wrapped::Value($pair) => match (take $pair) {{
            Pair::Values($first, $second) => ${selected}
        }}
    }};
}}
function noisy(): int {{ echo "outside"; return 1; }}
class Value {{
    int $number {{ get {{
        let $pair = Pair::Values(second: fn() => noisy(), first: fn() => 7);
        let $callback = select(identity(Wrapped::Value($pair)));
        return $callback();
    }} }}
}}
"#
        );
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
        assert_eq!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0770"),
            blocks,
            "{source}: {:?}",
            analysis.diagnostics
        );
        assert!(
            analysis
                .diagnostics
                .iter()
                .all(|diagnostic| diagnostic.code == "E0770"),
            "{source}: {:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn cached_validated_and_temperature_properties_execute_through_public_backends() {
    for (source, stdout, php_boundary) in [
        (include_str!("../../../examples/native/main_stage36_temperature.doria"),
         "freezing: 32.0 F\nboiling: 100.0 C; 212.0 F\n",
         "canonical `%f` float formatting"),
        (include_str!("../../../examples/native/main_stage36_cached_property.doria"),
         "first: 39\nagain: 39\ncalculations: 1\n",
         "checked integer overflow"),
        (include_str!("../../../examples/native/main_stage36_validated_property.doria"),
         "price: choose a price before reading the total\ntotal: 36\nquantity: quantity must be positive\nkept: 3; total: 36\n",
         "checked integer overflow"),
    ] {
        assert_hook_execution(source, stdout);
        let diagnostics = doriac::compile_source_to_php("hook-numeric-compatibility.doria", source)
            .expect_err("PHP must retain its numeric compatibility boundaries inside hooks");
        assert!(diagnostics.iter().all(|diagnostic| diagnostic.code == "B1301"
            && diagnostic.message.contains(php_boundary)), "{diagnostics:?}");
    }
}

#[test]
fn php_temperature_caching_and_validation_preserve_hook_contracts() {
    assert_hook_php_execution(
        r#"
class InvalidQuantity implements Error { string $message = "positive quantity required"; }
class Temperature {
    writable float $celsius = 0.0;
    writable float $fahrenheit {
        get => $this->celsius * 9.0 / 5.0 + 32.0;
        set (float $value) => $this->celsius = ($value - 32.0) * 5.0 / 9.0;
    }
}
class Report {
    internal writable ?float $cached = null;
    writable float $calculations = 0.0;
    float $total {
        writable get {
            let $cached = $this->cached;
            if ($cached != null) { return $cached; }
            $this->cached = 3.0 * 12.0;
            $this->calculations = $this->calculations + 1.0;
            return $this->cached ?? 0.0;
        }
    }
    writable int $quantity = 1 {
        get => $this->quantity;
        set (int $value) throws InvalidQuantity {
            if ($value < 1) { throw new InvalidQuantity(); }
            $this->quantity = $value;
        }
    }
}
function main(): void {
    let writable $temperature = new Temperature();
    echo ($temperature->fahrenheit == 32.0) . " ";
    $temperature->fahrenheit = 212.0;
    echo ($temperature->celsius == 100.0) . " ";
    let writable $report = new Report();
    echo ($report->total == 36.0) . " ";
    echo ($report->total == 36.0) . " ";
    echo ($report->calculations == 1.0) . " ";
    $report->quantity = 3;
    try { $report->quantity = 0; }
    catch (InvalidQuantity $error) { echo $error->message . " "; }
    echo $report->quantity;
}
"#,
        "true true true true true positive quantity required 3",
    );
}

#[test]
fn computed_rectangle_getters_compose_expression_and_block_bodies() {
    assert_hook_execution(
        include_str!("../../../examples/native/main_stage36_computed_properties.doria"),
        "area: 42\npaint: 12.0 litres\n",
    );
}

#[test]
fn php_hooks_share_dispatch_backing_and_float_update_order() {
    assert_hook_php_execution(include_str!("../../../examples/native/main_stage36_hook_updates.doria"), "derived get\nrhs\nderived set\nderived get\nderived set\nderived get\ntrue\nreceiver\nderived get\nrhs\nderived set\ninitial\nchanged authored\n");
}

#[test]
fn php_hook_failures_drop_temporary_receivers_before_catch() {
    assert_hook_php_execution(
        include_str!("../../../examples/native/main_stage36_hook_failures.doria"),
        concat!(
            "receiver\nget\nrhs\nset\ndrop\n",
            "receiver\nget\ndrop\ncaught get\n",
            "receiver\nget\nrhs\ndrop\ncaught rhs\n",
            "receiver\nget\nrhs\nset\ndrop\ncaught set\n",
        ),
    );
}

#[test]
fn php_hook_getters_preserve_owned_borrowed_and_nullable_results() {
    assert_hook_php_execution(include_str!("../../../examples/native/main_stage36_hook_results.doria"), "stored\ndrop temporary\ntemporary\ndrop temporary\ntrue\nstored\ndrop store\ndrop stored\n");
}

#[test]
fn php_hook_contracts_do_not_need_a_concrete_implementation_to_emit() {
    assert_hook_php_execution(
        r#"
interface Reading { string $name { get; } }
function read(Reading $value): string { return $value->name; }
function main(): void { echo "no instance\n"; }
"#,
        "no instance\n",
    );
}

#[test]
fn property_updates_share_setter_dispatch_and_scalar_operations() {
    assert_hook_execution(
        r#"
interface Counter {
    writable int $value { get; set (int $next); }
}
open class Base implements Counter {
    writable int $raw = 0;
    open writable int $value {
        get => $this->raw;
        set (int $next) => $this->raw = $next;
    }
}
class Derived extends Base {
    override writable int $value {
        get => $this->raw;
        set (int $next) => $this->raw = $next + 10;
    }
}
class Temperature {
    internal writable float $celsius = 0.0;
    writable float $fahrenheit {
        get => $this->celsius * 9.0 / 5.0 + 32.0;
        set (float $next) => $this->celsius = ($next - 32.0) * 5.0 / 9.0;
    }
}
class Backed {
    writable int $number = 5 {
        get => $this->number;
        set (int $next) => $this->number = $next;
    }
}
function concrete(writable Base $counter): void { $counter->value = 1; }
function erased(writable Counter $counter): void { $counter->value += 2; }
function constrained<T implements Counter>(writable T $counter): void { $counter->value++; }
function main(): void {
    let writable $counter = new Derived();
    concrete($counter);
    echo "{$counter->value} ";
    erased($counter);
    echo "{$counter->value} ";
    constrained($counter);
    echo "{$counter->value}\n";
    let writable $backed = new Backed();
    $backed->number *= 3;
    $backed->number--;
    echo "{$backed->number}\n";
    let writable $temperature = new Temperature();
    $temperature->fahrenheit = 212.0;
    $temperature->fahrenheit += 9.0;
    $temperature->fahrenheit++;
    echo "{$temperature->fahrenheit}\n";
}
"#,
        "11 23 34\n14\n222.0\n",
    );
}

#[test]
fn property_update_failures_preserve_receiver_rhs_setter_and_cleanup_order() {
    assert_hook_execution(
        r#"
class Trace { static writable string $events = ""; }
function trace(string $event): void { Trace::events = Trace::events . $event; }
class Fault implements Error { function __construct(string $message) {} }
class Counter {
    internal writable int $raw = 1;
    function __construct(bool $failGet, bool $failSet) {}
    writable int $value {
        get throws Fault {
            trace("get\n");
            if ($this->failGet) { throw new Fault("get"); }
            return $this->raw;
        }
        set (int $next) throws Fault {
            trace("set {$next}\n");
            if ($this->failSet) { throw new Fault("set"); }
            $this->raw = $next;
        }
    }
    function __destruct() { trace("drop {$this->raw}\n"); }
}
function make(bool $failGet, bool $failSet): Counter {
    trace("receiver\n");
    return new Counter($failGet, $failSet);
}
function rhs(bool $fail): int throws Fault {
    trace("rhs\n");
    if ($fail) { throw new Fault("rhs"); }
    return 2;
}
function main(): void {
    try { make(false, false)->value += rhs(false); } catch (Fault) { trace("unexpected\n"); }
    try { make(true, false)->value += rhs(false); } catch (Fault $fault) { trace("caught {$fault->message}\n"); }
    try { make(false, false)->value += rhs(true); } catch (Fault $fault) { trace("caught {$fault->message}\n"); }
    try { make(false, true)->value += rhs(false); } catch (Fault $fault) { trace("caught {$fault->message}\n"); }
    echo Trace::events;
}
"#,
        concat!(
            "receiver\nget\nrhs\nset 3\ndrop 3\n",
            "receiver\nget\ndrop 1\ncaught get\n",
            "receiver\nget\nrhs\ndrop 1\ncaught rhs\n",
            "receiver\nget\nrhs\nset 3\ndrop 1\ncaught set\n"
        ),
    );
}

#[test]
fn setters_reuse_argument_ownership_for_copy_and_move_types() {
    assert_hook_php_execution(
        include_str!("../../../examples/native/main_stage36_hook_ownership.doria"),
        "drop 1\n2 hello true\nempty\ndrop store\ndrop 2\n",
    );
}

#[test]
fn inherited_hook_writes_and_getter_results_preserve_nominal_upcasts() {
    assert_hook_execution(
        r#"
open class Base {
    writable int $value = 1 { get => $this->value; set (int $next) => $this->value = $next; }
}
class Derived extends Base {}
class Factory { Derived $created { get => new Derived(); } }
function produce(): Derived { return new Derived(); }
function main(): void {
    let writable $derived = new Derived();
    $derived->value += 2;
    echo "{$derived->value}\n";
    let $factory = new Factory();
    Base $first = $factory->created;
    ?Base $second = $factory->created;
    Base $third = produce();
    ?Base $fourth = produce();
    echo "{$first->value} {$second->value} {$third->value} {$fourth->value}\n";
}
"#,
        "3\n1 1 1 1\n",
    );
}

#[test]
fn nullable_and_temporary_hook_receivers_keep_once_only_calls_and_owned_results() {
    assert_hook_execution(
        include_str!("../../../examples/native/main_stage36_writable_getters.doria"),
        "3\ndrop 7\n3\ndrop 7\nreads 0\nreads 3\n",
    );
}

#[test]
fn hook_hir_preserves_accessor_identity_storage_effects_and_generic_bodies() {
    use doriac::ast::PropertyHookKind;
    use doriac::hir::{ClassMember, Item};
    use doriac::property_hooks::PropertyHookStorage;

    let hir = lower_hook_integration_hir(
        r#"
class Fault implements Error { string $message = "fault"; }
class Container<T> {
    function __construct(take T $stored) {}
    T $computed { get throws Fault { throw new Fault(); } }
    writable int $backed = 0 {
        set (int $next) throws Fault { if ($next < 0) { throw new Fault(); } $this->backed = $next; }
        get => $this->backed;
    }
}
function main(): void { let $container = new Container<int>(1); }
"#,
    );
    let class = hir
        .items
        .iter()
        .find_map(|item| match item {
            Item::Class(class) if class.name == "Container" => Some(class),
            _ => None,
        })
        .unwrap();
    let properties = class
        .members
        .iter()
        .filter_map(|member| match member {
            ClassMember::Property(property) => Some(property),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(!properties[0].has_storage());
    assert!(properties[1].has_storage());
    let computed = properties[0].hooks.as_ref().unwrap();
    assert_eq!(computed.storage, PropertyHookStorage::Computed);
    assert_eq!(computed.accessors[0].identity.kind, PropertyHookKind::Get);
    assert_eq!(
        computed.accessors[0].identity.property_span,
        properties[0].span
    );
    assert_eq!(
        computed.accessors[0]
            .function
            .return_type
            .as_ref()
            .unwrap()
            .name,
        "T"
    );
    assert_eq!(
        computed.accessors[0]
            .function
            .required_checked_effects
            .len(),
        1
    );
    assert!(!computed.accessors[0].function.body.statements.is_empty());
    let backed = properties[1].hooks.as_ref().unwrap();
    assert_eq!(backed.storage, PropertyHookStorage::Backed);
    assert_eq!(
        backed
            .accessors
            .iter()
            .map(|accessor| accessor.identity.kind)
            .collect::<Vec<_>>(),
        [PropertyHookKind::Set, PropertyHookKind::Get]
    );
    assert!(backed.accessors[0].function.writable_this);
    assert_eq!(backed.accessors[0].function.params[0].name, "next");
    assert_eq!(
        backed.accessors[0].function.required_checked_effects.len(),
        1
    );
    assert!(backed.accessors[1].function.checked_effects.is_empty());
    assert_eq!(
        class
            .members
            .iter()
            .flat_map(ClassMember::callables)
            .count(),
        4
    );
}

#[test]
fn accessor_bodies_enter_validated_mir_without_computed_storage() {
    let hir = lower_hook_integration_hir(
        r#"
class Container<T> {
    function __construct(take T $stored) {}
    T $computed { get => $this->stored; }
    writable int $backed = 0 {
        get => $this->backed;
        set (int $next) => $this->backed = $next;
    }
}
function main(): void { let $container = new Container<int>(1); }
"#,
    );
    let mir = doriac::mir_lowering::lower_program(&hir).unwrap();
    doriac::mir_validation::validate_program(&mir).unwrap();
    for name in ["computed::get", "backed::get", "backed::set"] {
        assert!(
            mir.functions.iter().any(|function| function
                .method
                .as_ref()
                .is_some_and(|method| method.name == name)),
            "missing {name}"
        );
    }
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&mir)
        .unwrap()
        .is_empty());
    #[cfg(feature = "llvm-backend")]
    assert!(!doriac::codegen_llvm::lower_mir_to_object(&mir)
        .unwrap()
        .is_empty());
}

#[test]
fn getter_calls_share_concrete_interface_generic_and_virtual_dispatch() {
    assert_hook_php_execution(
        include_str!("../../../examples/native/main_stage36_hook_composition.doria"),
        "2 2 2 2\n2 -1\n3\n",
    );
}

#[test]
fn getters_preserve_copy_results_and_move_result_ownership() {
    let hir = lower_hook_integration_hir(
        r#"
class Item {
    function __construct(int $id) {}
    function __destruct() { try { echo "drop {$this->id}\n"; } catch (Error) {} }
}
interface Shelf { Item $item { borrowed get; } }
class Store implements Shelf {
    Item $item = new Item(1) { borrowed get => $this->item; }
    List<int> $numbers = [2, 3] { borrowed get => $this->numbers; }
    string $text = "text" { get => $this->text; }
    ?string $absent = null { get => $this->absent; }
    float $fraction { get => 1.5; }
    bool $yes { get => true; }
    Item $created { get => new Item(4); }
    function __destruct() { try { echo "drop store\n"; } catch (Error) {} }
}
function read(Shelf $shelf): int { return $shelf->item->id; }
function main(): void {
    let $store = new Store();
    echo "{read($store)} {$store->numbers[1]} {$store->text} ";
    echo ($store->absent ?? "none") . " {$store->fraction} {$store->yes}\n";
    { let $item = $store->created; echo "created {$item->id}\n"; }
    echo "alive {$store->item->id}\n";
}
"#,
    );
    let mir = doriac::mir_lowering::lower_program(&hir).unwrap();
    doriac::mir_validation::validate_program(&mir).unwrap();
    assert_eq!(
        doriac::mir_interpreter::interpret(&mir).unwrap().stdout,
        b"1 3 text none 1.5 true\ncreated 4\ndrop 4\nalive 1\ndrop store\ndrop 1\n"
    );
    assert!(!doriac::codegen_cranelift::lower_mir_to_object(&mir)
        .unwrap()
        .is_empty());
    #[cfg(feature = "llvm-backend")]
    assert!(!doriac::codegen_llvm::lower_mir_to_object(&mir)
        .unwrap()
        .is_empty());
}

#[test]
fn physical_layout_excludes_computed_hooks_but_public_surface_preserves_them() {
    use doriac::numeric::IntegerType;
    use doriac::types::ResolvedType;

    let source = r#"
trait Values<T> {
    T $computed { get => $this->stored; }
    ?T $backed = null { get => $this->backed; }
}
class Container<T> {
    uses Values<T>;
    function __construct(take T $stored) {}
}
function inspect(Container<int> $container): int { return $container->computed; }
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let class = analysis
        .info
        .classes
        .iter()
        .find(|class| class.name == "Container<int>")
        .unwrap();
    assert_eq!(
        class
            .properties
            .iter()
            .map(|property| property.name.as_str())
            .collect::<Vec<_>>(),
        ["backed", "stored"]
    );
    for (index, property) in class.properties.iter().enumerate() {
        assert_eq!(property.id.class, class.id);
        assert_eq!(property.id.index, index);
        let ty = ResolvedType::Integer(IntegerType::Int64);
        assert_eq!(
            property.ty,
            if property.name == "backed" {
                ResolvedType::Nullable(Box::new(ty))
            } else {
                ty
            }
        );
    }
    let surface = analysis
        .info
        .class_member_surfaces
        .iter()
        .find(|surface| {
            surface.receiver.name == "Container" && surface.receiver.arguments == class.arguments
        })
        .unwrap();
    for name in ["computed", "backed", "stored"] {
        let property = surface
            .members
            .iter()
            .find(|property| property.name == name)
            .unwrap();
        let ty = ResolvedType::Integer(IntegerType::Int64);
        assert_eq!(
            property.ty,
            Some(if name == "backed" {
                ResolvedType::Nullable(Box::new(ty))
            } else {
                ty
            })
        );
        if name == "stored" {
            assert!(property.hooks.is_none());
        } else {
            let hooks = property.hooks.as_ref().unwrap();
            assert_eq!(
                hooks.storage,
                Some(if name == "computed" {
                    doriac::property_hooks::PropertyHookStorage::Computed
                } else {
                    doriac::property_hooks::PropertyHookStorage::Backed
                })
            );
            let getter = hooks.getter.as_ref().unwrap();
            assert_eq!(
                getter.receiver_mode,
                doriac::symbols::ReceiverMode::Readonly
            );
            assert_eq!(Some(&getter.signature.return_type), property.ty.as_ref());
            assert!(getter.return_borrow.is_none(), "specialized Copy result");
            assert!(hooks.setter.is_none());
        }
    }
}

#[test]
fn class_member_surfaces_preserve_each_accessor_contract() {
    use doriac::symbols::{BorrowSource, ReceiverMode, ReturnBorrow};
    use doriac::types::{ClassType, ResolvedType};

    let source = r#"
class Failure implements Error { string $message = "failed"; }
class Item {}
open class Base<T> {
    function __construct(take T $stored) {}
    open T $item { borrowed get throws Failure { return $this->stored; } }
    function getItem(): T { return $this->stored; }
    int $cached { writable get => 42; }
    writable T $sink { set (take T $value) {} }
}
class Child extends Base<Item> {
    function __construct() { parent::__construct(new Item()); }
}
function inspect(Child $child, Base<int> $copy): void {}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let surface = analysis
        .info
        .class_member_surface(&ClassType::new("Child", Vec::new()))
        .unwrap();
    let member = |name| {
        surface
            .members
            .iter()
            .find(|member| member.name == name)
            .unwrap()
    };
    let getter = member("item")
        .hooks
        .as_ref()
        .unwrap()
        .getter
        .as_ref()
        .unwrap();
    assert_eq!(getter.receiver_mode, ReceiverMode::Readonly);
    assert_eq!(
        getter.signature.return_type,
        ResolvedType::Class(ClassType::new("Item", Vec::new()))
    );
    assert!(getter.signature.parameters.is_empty());
    assert_eq!(
        getter.checked_effects,
        [ResolvedType::Class(ClassType::new("Failure", Vec::new()))]
    );
    assert_eq!(
        getter.return_borrow,
        Some(ReturnBorrow {
            source: BorrowSource::Receiver,
            writable: false,
            kind: doriac::types::ReturnBorrowKind::Value,
        })
    );
    assert_eq!(member("item").declaring_class.name, "Base");
    assert!(member("item").is_open);
    let cached = member("cached").hooks.as_ref().unwrap();
    assert_eq!(
        cached.getter.as_ref().unwrap().receiver_mode,
        ReceiverMode::Writable
    );
    assert!(cached.setter.is_none());
    assert!(!member("cached").writable);
    let sink = member("sink").hooks.as_ref().unwrap();
    assert!(sink.getter.is_none());
    let setter = sink.setter.as_ref().unwrap();
    assert_eq!(setter.receiver_mode, ReceiverMode::Writable);
    assert_eq!(setter.signature.return_type, ResolvedType::Void);
    assert_eq!(setter.signature.parameters.len(), 1);
    assert_eq!(
        setter.signature.parameters[0].r#type,
        getter.signature.return_type
    );
    assert!(setter.signature.parameters[0].take);
    assert!(member("sink").writable);
    let copy = analysis
        .info
        .class_member_surfaces
        .iter()
        .find(|surface| {
            surface.receiver.name == "Base"
                && surface.receiver.arguments
                    == [ResolvedType::Integer(doriac::numeric::IntegerType::Int64)]
        })
        .unwrap();
    for member in &copy.members {
        assert!(member.return_borrow.is_none());
        if let Some(hooks) = &member.hooks {
            assert!(hooks
                .getter
                .as_ref()
                .is_none_or(|getter| getter.return_borrow.is_none()));
        }
    }
}

#[test]
fn overridden_accessors_do_not_erase_parent_storage_or_create_computed_fields() {
    let source = r#"
class Child extends Base {
    override int $backed { get => 4; }
    override int $computed = 5 { get => $this->computed; }
    int $own = 6;
}
open class Base {
    int $stored = 1;
    open int $backed = 2 { get => $this->backed; }
    open int $computed { get => 3; }
}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let class = analysis
        .info
        .classes
        .iter()
        .find(|class| class.name == "Child")
        .unwrap();
    assert_eq!(
        class
            .properties
            .iter()
            .map(|property| { (property.declaring_class.as_str(), property.name.as_str()) })
            .collect::<Vec<_>>(),
        [
            ("Base", "stored"),
            ("Base", "backed"),
            ("Child", "computed"),
            ("Child", "own")
        ]
    );
    let surface = analysis
        .info
        .class_member_surfaces
        .iter()
        .find(|surface| surface.receiver.name == "Child")
        .unwrap();
    assert_eq!(surface.members.len(), 4);
    for name in ["backed", "computed"] {
        let property = surface
            .members
            .iter()
            .find(|property| property.name == name)
            .unwrap();
        assert_eq!(property.declaring_class.name, "Child");
        assert!(property.is_override);
        assert!(!property.is_open);
    }
    let base = analysis
        .info
        .class_member_surfaces
        .iter()
        .find(|surface| surface.receiver.name == "Base")
        .unwrap();
    assert!(base
        .members
        .iter()
        .filter(|property| property.name != "stored")
        .all(|property| property.is_open));
}

#[test]
fn property_overrides_preserve_accessor_roots_across_generic_ancestors() {
    let source = r#"
class Observed { static writable int $last = 0; }
class Leaf extends Middle {
    function __construct(parameter int $stored) { parent::__construct($stored); }
    override writable int $value { get => 3; set (int $next) { Observed::last = $next; } }
}
open class Middle extends Base<int> {
    function __construct(parameter int $stored) { parent::__construct($stored); }
    override writable int $value { get => 2; set (int $next) { Observed::last = $next; } }
}
open class Base<T> {
    open writable T $value { get => $this->stored; set (T $next) {} }
    function __construct(take T $stored) {}
}
function readBase(Base<int> $value): int { return $value->value; }
function readMiddle(Middle $value): int { return $value->value; }
function readLeaf(Leaf $value): int { return $value->value; }
function writeBase(writable Base<int> $value): void { $value->value = 1; }
function writeMiddle(writable Middle $value): void { $value->value = 2; }
function writeLeaf(writable Leaf $value): void { $value->value = 3; }
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let calls = analysis
        .info
        .property_accessor_calls
        .values()
        .collect::<Vec<_>>();
    assert_eq!(calls.len(), 6);
    let getter_roots = calls
        .iter()
        .filter_map(|calls| calls.getter.as_ref().map(|call| call.virtual_root.unwrap()))
        .collect::<std::collections::HashSet<_>>();
    let setter_roots = calls
        .iter()
        .filter_map(|calls| calls.setter.as_ref().map(|call| call.virtual_root.unwrap()))
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(getter_roots.len(), 1);
    assert_eq!(setter_roots.len(), 1);
    assert_ne!(getter_roots, setter_roots);
    for root in getter_roots.into_iter().chain(setter_roots) {
        assert!(root.start > source.find("open class Base<T>").unwrap());
    }
}

#[test]
fn property_overrides_allow_compatible_refinements_and_track_added_accessors() {
    let source = r#"
class Fault implements Error { string $message = "fault"; }
open class Item {}
class SpecialItem extends Item {}
class Leaf extends Middle {
    override writable SpecialItem $value {
        get => new SpecialItem();
        set (take SpecialItem $next) {}
    }
}
open class Middle extends Base {
    override writable SpecialItem $value {
        get => new SpecialItem();
        set (take SpecialItem $next) {}
    }
}
open class Base {
    open Item $value { writable get throws Fault => new Item(); }
}
function middleWrite(writable Middle $receiver): void { $receiver->value = new SpecialItem(); }
function leafWrite(writable Leaf $receiver): void { $receiver->value = new SpecialItem(); }
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let setters = analysis
        .info
        .property_accessor_calls
        .values()
        .filter_map(|calls| calls.setter.as_ref())
        .collect::<Vec<_>>();
    assert_eq!(setters.len(), 2);
    assert!(setters[0].virtual_root.is_some());
    assert_eq!(setters[0].virtual_root, setters[1].virtual_root);
    let root = setters[0].virtual_root.unwrap();
    assert!(root.start > source.find("open class Middle").unwrap());
    assert!(root.end < source.find("open class Base").unwrap());
}

#[test]
fn property_overrides_check_the_whole_callable_contract() {
    for (base, child, code) in [
        (
            "open int $value { get => 1; }",
            "int $value { get => 2; }",
            "E0726",
        ),
        (
            "int $value { get => 1; }",
            "override int $value { get => 2; }",
            "E0727",
        ),
        (
            "int $value = 1;",
            "override int $value { get => 2; }",
            "E0727",
        ),
        ("open int $value { get => 1; }", "int $value = 2;", "E0727"),
        (
            "open int $value { get => 1; }",
            "override float $value { get => 2.0; }",
            "E0729",
        ),
        (
            "open int $value { get => 1; }",
            "override int $value { writable get => 2; }",
            "E0729",
        ),
        (
            "open int $value { get => 1; }",
            "internal override int $value { get => 2; }",
            "E0729",
        ),
        (
            "open writable int $value { get => 1; set (int $next) {} }",
            "override int $value { get => 2; }",
            "E0729",
        ),
        (
            "open writable int $value { get => 1; set (int $next) {} }",
            "override writable int $value { get => 2; set (int $other) {} }",
            "E0729",
        ),
        (
            "open Item $value { get => new Item(); }",
            "override Item $value { get => $this->item; } internal Item $item = new Item();",
            "E0729",
        ),
        (
            "open Item $value { get => $this->item; } internal Item $item = new Item();",
            "override Item $value { get => new Item(); }",
            "E0729",
        ),
        (
            "open writable Item $value { set (take Item $next) {} }",
            "override writable Item $value { set (Item $next) {} }",
            "E0729",
        ),
        (
            "open int $value { get => 1; }",
            "override int $value { get throws Fault => 2; }",
            "E0729",
        ),
        ("", "override int $value { get => 2; }", "E0730"),
    ] {
        let source = format!("class Item {{}} class Fault implements Error {{ string $message = \"fault\"; }} open class Base {{ {base} }} class Child extends Base {{ {child} }}");
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == code),
            "{base} / {child}: {:?}",
            analysis.diagnostics
        );
    }
    for source in [
        "class Closed { open int $value { get => 1; } }",
        "open class Base { internal open int $value { get => 1; } }",
    ] {
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0725"),
            "{source}: {:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn interface_and_constrained_property_calls_preserve_accessor_contracts() {
    use doriac::numeric::IntegerType;
    use doriac::types::ResolvedType;
    let source = r#"
interface Read<T> { T $value { get; } }
interface Left<T> extends Read<T> {}
interface Right<T> extends Read<T> {}
interface Counter extends Left<int>, Right<int> {
    writable int $value { get; set (int $next); }
}
function inspect(Counter $counter): int { return $counter->value; }
function optional(?Counter $counter): ?int { return $counter?->value; }
function update(writable Counter $counter): void {
    $counter->value = 2;
    $counter->value += 3;
    $counter->value++;
}
function generic<T implements Counter>(writable T $counter): int {
    $counter->value = 4;
    $counter->value += 5;
    return $counter->value;
}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert_eq!(analysis.info.property_accessor_calls.len(), 8);
    let mut constrained = 0;
    let mut reads_and_writes = 0;
    for calls in analysis.info.property_accessor_calls.values() {
        reads_and_writes += usize::from(calls.getter.is_some() && calls.setter.is_some());
        for call in calls.getter.iter().chain(calls.setter.iter()) {
            match &call.declaring_type {
                ResolvedType::Interface(interface) => assert_eq!(interface.name, "Counter"),
                ResolvedType::TypeParameter(parameter) => {
                    assert_eq!(parameter, "T");
                    constrained += 1;
                }
                unexpected => panic!("unexpected dispatch owner {unexpected:?}"),
            }
            if let Some(parameter) = &call.parameter {
                assert_eq!(parameter.r#type, ResolvedType::Integer(IntegerType::Int64));
                assert!(call.receiver_mode.is_writable());
            } else {
                assert_eq!(call.return_type, ResolvedType::Integer(IntegerType::Int64));
            }
        }
    }
    assert_eq!(constrained, 4);
    assert_eq!(reads_and_writes, 3);
    assert!(analysis
        .info
        .contracts
        .member_references
        .iter()
        .any(|reference| {
            &source[reference.span.start..reference.span.end] == "value"
                && reference.origins.len() > 1
        }));
}

#[test]
fn contract_property_calls_reject_missing_access_and_wrong_types() {
    for (contract, body, code) in [
        ("int $value { get; }", "$counter->value = 1;", "E0768"),
        (
            "writable int $value { set (int $next); }",
            "echo $counter->value;",
            "E0767",
        ),
        (
            "writable int $value { set (int $next); }",
            "$counter->value++;",
            "E0767",
        ),
        (
            "int $value { writable get; }",
            "echo $readonly->value;",
            "E0203",
        ),
        (
            "writable int $value { get; set (int $next); }",
            "$readonly->value = 1;",
            "E0203",
        ),
        (
            "writable int $value { get; set (int $next); }",
            "$counter->value = \"wrong\";",
            "E0408",
        ),
        (
            "string $value { get; }",
            "int $number = $counter->value;",
            "E0403",
        ),
        ("int $value { get; }", "$counter->value();", "E0304"),
    ] {
        for (generic, receiver) in [("", "Counter"), ("<T implements Counter>", "T")] {
            let source = format!("interface Counter {{ {contract} }} function inspect{generic}({receiver} $readonly, writable {receiver} $counter): void {{ {body} }}");
            let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
            let expected = if !generic.is_empty() && code == "E0304" {
                "E0537"
            } else {
                code
            };
            assert!(
                analysis
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == expected),
                "{source}: {:?}",
                analysis.diagnostics
            );
        }
    }
}

#[test]
fn contract_getter_ownership_controls_consumption_and_borrow_lifetime() {
    let declarations = r#"
class Item { int $value = 1; }
interface View { Item $value { borrowed get; } writable function change(): void; }
interface Factory { Item $value { get; } }
function consume(take Item $item): void {}
"#;
    for (contract, body, expected) in [
        (
            "Factory",
            "let $item = $source->value; consume($item);",
            None,
        ),
        (
            "View",
            "let $item = $source->value; echo $item->value; $source->change();",
            None,
        ),
        (
            "View",
            "let $item = $source->value; consume($item);",
            Some("E0474"),
        ),
        (
            "View",
            "let $item = $source->value; $source->change(); echo $item->value;",
            Some("E0477"),
        ),
    ] {
        for (generic, receiver) in [
            (String::new(), contract.to_string()),
            (format!("<T implements {contract}>"), "T".to_string()),
        ] {
            let source = format!("{declarations} function inspect{generic}(writable {receiver} $source): void {{ {body} }}");
            let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
            if let Some(code) = expected {
                assert!(
                    analysis
                        .diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.code == code),
                    "{source}: {:?}",
                    analysis.diagnostics
                );
            } else {
                assert!(
                    analysis.diagnostics.is_empty(),
                    "{source}: {:?}",
                    analysis.diagnostics
                );
            }
        }
    }
}

#[test]
fn contract_accessor_errors_are_checked_at_read_and_write_sites() {
    let declarations = r#"
class Failure implements Error { string $message = "failure"; }
interface Counter {
    writable int $value { get throws Failure; set (int $next) throws Failure; }
}
"#;
    for body in [
        "echo $counter->value;",
        "$counter->value = 1;",
        "$counter->value += 2;",
    ] {
        for (generic, receiver) in [("", "Counter"), ("<T implements Counter>", "T")] {
            for handled in [false, true] {
                let body = if handled {
                    format!("try {{ {body} }} catch (Failure) {{}}")
                } else {
                    body.to_string()
                };
                let source = format!("{declarations} function inspect{generic}(writable {receiver} $counter): void {{ {body} }}");
                let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
                assert_eq!(
                    analysis
                        .diagnostics
                        .iter()
                        .any(|diagnostic| diagnostic.code == "E0631"),
                    !handled,
                    "{source}: {:?}",
                    analysis.diagnostics
                );
                if handled {
                    assert!(
                        analysis.diagnostics.is_empty(),
                        "{source}: {:?}",
                        analysis.diagnostics
                    );
                }
            }
        }
    }
}

#[test]
fn accessor_result_provenance_survives_interface_and_generic_analysis() {
    use doriac::ast::Item;
    use doriac::symbols::BorrowSource;
    let source = r#"
class Item {}
interface View<T> { T $value { borrowed get; } }
interface Factory { Item $value { get; } }
function borrowItem(View<Item> $source): Item { return $source->value; }
function ownItem(Factory $source): Item { return $source->value; }
function genericBorrow<T implements View<Item>>(T $source): Item { return $source->value; }
function genericOwned<T implements Factory>(T $source): Item { return $source->value; }
function copy(View<int> $source): int { return $source->value; }
class Borrowing<T implements View<Item>> {
    function genericBorrow(T $source): Item { return $source->value; }
}
class Owning<T implements Factory> {
    function genericOwned(T $source): Item { return $source->value; }
}
"#;
    let (program, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let functions = program.items.iter().flat_map(|item| match item {
        Item::Function(function) => vec![function],
        Item::Class(class) => class
            .members
            .iter()
            .filter_map(|member| {
                if let doriac::ast::ClassMember::Method(method) = member {
                    Some(method)
                } else {
                    None
                }
            })
            .collect(),
        _ => Vec::new(),
    });
    for function in functions {
        let borrowed = matches!(function.name.as_str(), "borrowItem" | "genericBorrow");
        assert_eq!(
            analysis
                .info
                .return_borrows
                .get(&function.span)
                .map(|borrow| borrow.source),
            borrowed.then_some(BorrowSource::Parameter(0)),
            "{}",
            function.name
        );
        let call = analysis
            .info
            .property_accessor_calls
            .iter()
            .find(|(span, _)| span.start >= function.span.start && span.end <= function.span.end)
            .unwrap()
            .1
            .getter
            .as_ref()
            .unwrap();
        assert_eq!(
            call.return_borrow.map(|borrow| borrow.source),
            borrowed.then_some(BorrowSource::Receiver),
            "{}",
            function.name
        );
    }
}

#[test]
fn contract_setters_transfer_owned_arguments_without_transferring_the_receiver() {
    let declarations = r#"
class Item {}
interface Sink { writable Item $item { set (take Item $value); } }
"#;
    for (generic, receiver) in [("", "Sink"), ("<T implements Sink>", "T")] {
        for second in [false, true] {
            let body = if second { "$sink->item = $item;" } else { "" };
            let source = format!("{declarations} function feed{generic}(writable {receiver} $sink): void {{ let $item = new Item(); $sink->item = $item; {body} }}");
            let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
            assert_eq!(
                analysis
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "E0470"),
                second,
                "{source}: {:?}",
                analysis.diagnostics
            );
            if !second {
                assert!(
                    analysis.diagnostics.is_empty(),
                    "{source}: {:?}",
                    analysis.diagnostics
                );
            }
        }
    }
}

#[test]
fn callable_property_contracts_stay_properties_when_invoked() {
    let source = r#"
interface Actions { function(): int $next { get; } }
function direct(Actions $actions): int { return $actions->next(); }
function generic<T implements Actions>(T $actions): int { return $actions->next(); }
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert_eq!(analysis.info.property_accessor_calls.len(), 2);
    assert_eq!(analysis.info.callable_value_calls.len(), 2);
}

#[test]
fn constrained_accessor_intersections_select_compatible_contracts_not_order() {
    for constraints in ["Read, Cached", "Cached, Read"] {
        let source = format!(
            r#"
interface Read {{ int $value {{ get; }} }}
interface Cached {{ int $value {{ writable get; }} }}
function inspect<T implements {constraints}>(T $value): int {{ return $value->value; }}
"#
        );
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
        assert!(
            analysis.diagnostics.is_empty(),
            "{source}: {:?}",
            analysis.diagnostics
        );
        let call = analysis
            .info
            .property_accessor_calls
            .values()
            .next()
            .unwrap()
            .getter
            .as_ref()
            .unwrap();
        assert!(!call.receiver_mode.is_writable());
    }
    for constraints in ["Number, Text", "Text, Number"] {
        let source = format!(
            r#"
interface Number {{ int $value {{ get; }} }}
interface Text {{ string $value {{ get; }} }}
function inspect<T implements {constraints}>(T $value): void {{ echo $value->value; }}
"#
        );
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0753" && diagnostic.related.len() == 2),
            "{source}: {:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn interface_accessor_requirements_share_the_canonical_contract_graph() {
    use doriac::ast::PropertyHookKind;
    use doriac::semantics::contracts::ConformanceStatus;
    let source = r#"
interface Read<T> { T $value { get; } }
interface Left<T> extends Read<T> {}
interface Right<T> extends Read<T> {}
interface Both<T> extends Left<T>, Right<T> {
    writable T $value { get; set (T $next); }
}
class Counter implements Both<int> {
    writable int $value = 0 {
        get => $this->value;
        set (int $next) => $this->value = $next;
    }
}
function main(): void {}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
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
    assert_eq!(both.requirements.len(), 2);
    assert_eq!(both.requirements[0].accessor, Some(PropertyHookKind::Get));
    assert_eq!(both.requirements[0].origins.len(), 2);
    assert_eq!(both.requirements[1].accessor, Some(PropertyHookKind::Set));
    assert!(both.requirements[1].writable_receiver);
    assert!(analysis
        .info
        .contracts
        .conformances
        .iter()
        .all(|fact| fact.status == ConformanceStatus::Checked));
}

#[test]
fn interface_accessors_check_receiver_ownership_effects_and_missing_members() {
    let declarations = r#"
class Value {}
class Failure implements Error { string $message = "failure"; }
interface View { Value $value { borrowed get; } }
interface Factory { Value $value { get; } }
interface Read { int $value { get; } }
interface Write { writable int $value { set (int $next); } }
"#;
    for (implementation, expected) in [
        ("class Good implements Factory { Value $value { get => new Value(); } }", None),
        ("class Good implements View { Value $value = new Value() { borrowed get => $this->value; } }", None),
        ("class Missing implements Read {}", Some("E0754")),
        ("class Method implements Read { function value(): int { return 1; } }", Some("E0754")),
        ("class GetterOnly implements Write { int $value { get => 1; } }", Some("E0754")),
        ("class WrongReceiver implements Read { int $value { writable get => 1; } }", Some("E0755")),
        ("class WrongResult implements View { Value $value { get => new Value(); } }", Some("E0755")),
        ("class WrongResult implements Factory { Value $value = new Value() { borrowed get => $this->value; } }", Some("E0755")),
        ("class WrongEffect implements Read { int $value { get throws Failure { throw new Failure(); } } }", Some("E0755")),
    ] {
        let source = format!("{declarations} {implementation} function main(): void {{}}");
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
        if let Some(code) = expected {
            assert!(analysis.diagnostics.iter().any(|diagnostic| diagnostic.code == code), "{implementation}: {:?}", analysis.diagnostics);
        } else {
            assert!(analysis.diagnostics.is_empty(), "{implementation}: {:?}", analysis.diagnostics);
        }
    }
}

#[test]
fn interface_accessor_bodies_initializers_and_name_collisions_are_rejected() {
    for declaration in [
        "interface Invalid { int $value { get => 1; } }",
        "interface Invalid { int $value = 1 { get; } }",
    ] {
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", declaration).unwrap();
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0749"),
            "{:?}",
            analysis.diagnostics
        );
    }
    for declaration in [
        "interface Invalid { int $value { get; } int $value { get; } }",
        "interface Invalid { int $value { get; } function value(): int; }",
    ] {
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", declaration).unwrap();
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0753"),
            "{:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn trait_getters_implement_interface_contracts_without_becoming_methods() {
    let source = r#"
interface Counted { int $count { get; } }
trait Counting { int $count { get => 42; } }
class Example implements Counted { uses Counting; }
function main(): void { let $example = new Example(); echo $example->count; }
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let (_, invalid) = doriac::analyze_source_for_ide(
        "hooks.doria",
        source.replace("echo $example->count;", "echo $example->count();"),
    )
    .unwrap();
    assert!(
        invalid
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0304"),
        "{:?}",
        invalid.diagnostics
    );
}

#[test]
fn inherited_method_and_property_contracts_cannot_share_a_member_name() {
    let source = r#"
interface Method { function value(): int; }
interface Property { int $value { get; } }
interface Invalid extends Method, Property {}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0753"),
        "{:?}",
        analysis.diagnostics
    );
    assert!(
        !analysis
            .info
            .contracts
            .interfaces
            .iter()
            .find(|interface| interface.name == "Invalid")
            .unwrap()
            .valid
    );
}

#[test]
fn borrowed_interface_getters_specialize_copy_and_move_results() {
    let source = r#"
interface View<T> { T $value { borrowed get; } }
class Number implements View<int> { int $value { get => 42; } }
class Buffer implements View<List<int>> {
    List<int> $value = [1] { borrowed get => $this->value; }
}
interface Collections { List<int> $items { borrowed get; } }
class Items implements Collections {
    List<int> $items = [1] { borrowed get => $this->items; }
}
function main(): void {}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let collections = analysis
        .info
        .contracts
        .interfaces
        .iter()
        .find(|interface| interface.name == "Collections")
        .unwrap();
    assert!(collections.requirements[0].return_borrow.is_some());
}

#[test]
fn getter_callables_and_iterators_keep_their_capture_and_source_lifetimes() {
    let declarations = r#"
class Cursor implements Iterator<int> {
    function __construct(borrow List<int> $source) {}
    function hasCurrent(): bool { return true; }
    function getCurrent(): int { return $this->source[0]; }
    writable function advance(): void {}
}
class Source {
    int $value = 42;
    List<int> $values = [1];
    function once(): int $owned { get => fn() => 42; }
    function(): int $borrowed { get => fn() with ($this) => $this->value; }
    Cursor $cursor { get => new Cursor($this->values); }
    writable function change(): void {}
}
function consume(take Source $source): void {}
"#;
    for (body, expected) in [
        ("let $source = new Source(); echo $source->owned();", None),
        ("let $source = new Source(); let $callback = $source->owned; consume($source); echo $callback();", None),
        ("let $source = new Source(); let $callback = $source->borrowed; echo $callback(); consume($source);", None),
        ("let $source = new Source(); let $callback = $source->borrowed; consume($source); echo $callback();", Some("E0654")),
        ("let writable $source = new Source(); let $cursor = $source->cursor; $source->change(); echo $cursor->getCurrent();", Some("E0763")),
    ] {
        let source = format!("{declarations} function main(): void {{ {body} }}");
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
        if let Some(code) = expected {
            assert!(analysis.diagnostics.iter().any(|diagnostic| diagnostic.code == code), "{body}: {:?}", analysis.diagnostics);
        } else {
            assert!(analysis.diagnostics.is_empty(), "{body}: {:?}", analysis.diagnostics);
            assert_hook_php_execution(&source, "42");
        }
    }
}

#[test]
fn callable_getters_preserve_carrier_ownership_across_dispatch_and_nullability() {
    assert_hook_php_execution(
        r#"
interface Reader { function(): int $callback { get; } }
class Source implements Reader {
    int $value = 42;
    function(): int $callback { get => fn() with ($this) => $this->value; }
    ?function(): int $optional { get => fn() with ($this) => $this->value; }
}
function invoke(Reader $reader): int { return $reader->callback(); }
function generic<T implements Reader>(T $reader): int { return $reader->callback(); }
function maybe(bool $present): ?Source {
    if ($present) { return new Source(); }
    return null;
}
function main(): void {
    let $source = new Source();
    echo "{$source->callback()} {invoke($source)} {generic($source)}\n";
    let $callback = $source->optional;
    if ($callback != null) { echo "{$callback()}\n"; }
    let $present = maybe(true);
    let $nullable = $present?->callback;
    if ($nullable != null) { echo "{$nullable()}\n"; }
    let $absent = maybe(false);
    let $missing = $absent?->callback;
    if ($missing == null) { echo "none\n"; }
}
"#,
        "42 42 42\n42\n42\nnone\n",
    );
}

#[test]
fn stored_callable_getter_does_not_transfer_the_owners_environment() {
    assert_hook_php_execution(
        r#"
class Value { function __construct(int $number) {} }
class Source {
    function __construct(take function(): int $stored) {}
    function(): int $callback { borrowed get => $this->stored; }
}
function main(): void {
    let $value = new Value(42);
    let $source = new Source(fn() with (take $value) => $value->number);
    {
        let $callback = $source->callback;
        echo "{$callback()}\n";
    }
    echo "{$source->stored()}\n";
}
"#,
        "42\n42\n",
    );
}

#[test]
fn callable_getter_contracts_use_resolved_capture_provenance() {
    for (requirement, implementation, expected) in [
        (
            "borrowed get",
            "fn() with ($this) => $this->value",
            Some("E0755"),
        ),
        ("borrowed get", "fn() => 42", Some("E0755")),
        ("get", "fn() with ($this) => $this->value", None),
        ("get", "fn() => 42", None),
    ] {
        let source = format!(
            "interface Reader {{ function(): int $callback {{ {requirement}; }} }}
             class Source implements Reader {{
                 int $value = 42;
                 function(): int $callback {{ get => {implementation}; }}
             }}"
        );
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
        let errors = &analysis.diagnostics;
        match expected {
            Some(code) => assert!(
                errors.iter().any(|d| d.code == code),
                "{source}: {errors:?}"
            ),
            None => assert!(errors.is_empty(), "{source}: {errors:?}"),
        }
    }
    assert_hook_php_execution(
        r#"
open class Base {
    int $value = 42;
    open function(): int $callback { get => fn() with ($this) => $this->value; }
}
class Child extends Base {
    int $childValue = 43;
    override function(): int $callback { get => fn() with ($this) => $this->childValue; }
}
function main(): void {
    Base $source = new Child();
    echo "{$source->callback()}\n";
}
"#,
        "43\n",
    );
}

#[test]
fn property_updates_evaluate_receivers_once_and_keep_failure_ownership_order() {
    let declarations = r#"
class Value {}
class ReadFailure implements Error { string $message = "read"; }
class WriteFailure implements Error { string $message = "write"; }
class Counter {
    writable int $value = 0 {
        get throws ReadFailure => $this->value;
        set (int $next) throws WriteFailure => $this->value = $next;
    }
}
function create(take Value $token): Counter { return new Counter(); }
function consume(take Value $value): int { return 1; }
function inspect(Value $value): void {}
"#;
    for (body, moved) in [
        ("let $token = new Value(); create($token)->value += 1;", false),
        ("let $token = new Value(); create($token)->value++;", false),
        ("let writable $counter = new Counter(); let $value = new Value(); try { $counter->value += consume($value); } catch (ReadFailure) { inspect($value); } catch (WriteFailure) {}", false),
        ("let writable $counter = new Counter(); let $value = new Value(); try { $counter->value += consume($value); } catch (ReadFailure) {} catch (WriteFailure) { inspect($value); }", true),
    ] {
        let source = format!("{declarations} function main(): void {{ {body} }}");
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
        if moved {
            assert!(analysis.diagnostics.iter().any(|diagnostic| diagnostic.code == "E0470"), "{body}: {:?}", analysis.diagnostics);
        } else {
            assert!(analysis.diagnostics.is_empty(), "{body}: {:?}", analysis.diagnostics);
        }
    }
}

#[test]
fn setter_arguments_use_normal_parameter_ownership() {
    let declarations = r#"
class Value { writable function change(): void {} }
class Sink {
    writable Value $taken { set (take Value $value) {} }
    writable Value $read { set (Value $value) {} }
    writable Value $changed { set (writable Value $value) { $value->change(); } }
}
function inspect(Value $value): void {}
"#;
    for (body, expected) in [
        (
            "let $value = new Value(); $sink->read = $value; inspect($value);",
            None,
        ),
        (
            "let writable $value = new Value(); $sink->changed = $value; inspect($value);",
            None,
        ),
        (
            "let $value = new Value(); $sink->taken = $value; inspect($value);",
            Some("E0470"),
        ),
        (
            "let $value = new Value(); $sink->changed = $value;",
            Some("E0204"),
        ),
    ] {
        let source = format!(
            "{declarations} function main(): void {{ let writable $sink = new Sink(); {body} }}"
        );
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
        if let Some(code) = expected {
            assert!(
                analysis
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == code),
                "{body}: {:?}",
                analysis.diagnostics
            );
        } else {
            assert!(
                analysis.diagnostics.is_empty(),
                "{body}: {:?}",
                analysis.diagnostics
            );
        }
    }
    let source = format!("{declarations} function accept(writable Sink $sink, Value $value): void {{ $sink->taken = $value; }}");
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0474"),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn getter_results_preserve_owned_and_borrowed_call_contracts() {
    let declarations = r#"
class Value { writable function change(): void {} }
class Factory { Value $item { get => new Value(); } }
class Holder {
    function __construct(take Value $initial) {}
    writable Value $item {
        borrowed get => $this->initial;
        set (take Value $next) {}
    }
    writable function change(): void {}
}
function consume(take Value $value): void {}
function inspect(Value $value): void {}
function consumeFactory(take Factory $value): void {}
function forwarded(Factory $factory): Value { return $factory->item; }
"#;
    for (body, expected) in [
        ("let $factory = new Factory(); let $item = $factory->item; consumeFactory($factory); consume($item);", None),
        ("let $factory = new Factory(); $factory->item->change(); consume(forwarded($factory));", None),
        ("let $holder = new Holder(new Value()); consume($holder->item);", Some("E0474")),
        ("let writable $holder = new Holder(new Value()); $holder->item->change();", Some("E0203")),
        ("let writable $holder = new Holder(new Value()); let $item = $holder->item; $holder->change(); inspect($item);", Some("E0477")),
        ("let writable $holder = new Holder(new Value()); let $item = $holder->item; inspect($item); $holder->change();", None),
    ] {
        let source = format!("{declarations} function main(): void {{ {body} }}");
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
        if let Some(code) = expected {
            assert!(analysis.diagnostics.iter().any(|diagnostic| diagnostic.code == code), "{body}: {:?}", analysis.diagnostics);
        } else {
            assert!(analysis.diagnostics.is_empty(), "{body}: {:?}", analysis.diagnostics);
        }
    }
}

#[test]
fn property_operations_record_distinct_checked_accessor_calls() {
    let source = r#"
class ReadFailure implements Error { string $message = "read"; }
class WriteFailure implements Error { string $message = "write"; }
class Counter {
    writable int $value = 0 {
        get throws ReadFailure => $this->value;
        set (int $next) throws WriteFailure => $this->value = $next;
    }
}
function main(): void {
    let writable $counter = new Counter();
    $counter->value = 1;
    echo $counter->value;
    $counter->value += 2;
    $counter->value++;
}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let calls = &analysis.info.property_accessor_calls;
    assert_eq!(calls.len(), 4);
    let write = calls
        .iter()
        .find(|(span, _)| source[span.end..].starts_with(" = 1"))
        .unwrap()
        .1;
    assert!(write.getter.is_none());
    assert!(write.setter.is_some());
    let read = calls
        .iter()
        .find(|(span, _)| source[..span.start].ends_with("echo "))
        .unwrap()
        .1;
    assert!(read.getter.is_some());
    assert!(read.setter.is_none());
    assert_eq!(
        calls
            .values()
            .filter(|call| call.getter.is_some() && call.setter.is_some())
            .count(),
        2
    );
    for span in calls.keys() {
        assert_eq!(&source[span.start..span.end], "$counter->value");
    }
    assert!(analysis.info.callable_effective_checked_effects.values().any(|effects| {
        ["ReadFailure", "WriteFailure"].iter().all(|name| effects.iter().any(|effect| matches!(effect, doriac::types::ResolvedType::Class(class) if class.name == *name)))
    }));
}

#[test]
fn missing_accessors_and_receiver_permissions_are_checked_at_use() {
    for (declaration, operation, code) in [
        (
            "writable int $value { set (int $next) {} }",
            "echo $counter->value;",
            "E0767",
        ),
        ("int $value { get => 0; }", "$counter->value = 1;", "E0768"),
        (
            "int $value { writable get => 0; }",
            "echo $readonly->value;",
            "E0203",
        ),
        (
            "writable int $value { set (int $next) {} }",
            "$readonly->value = 1;",
            "E0203",
        ),
        (
            "writable int $value { set (int $next) {} }",
            "$counter->value += 1;",
            "E0767",
        ),
    ] {
        let source = format!("class Counter {{ {declaration} }} function inspect(Counter $readonly, writable Counter $counter): void {{ {operation} }}");
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
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
fn own_backing_is_lexical_and_other_property_reads_invoke_getters() {
    let source = r#"
class Counter {
    writable int $value = 0 {
        get {
            let $read = fn() with ($this) => $this->value;
            return $read();
        }
        set (int $next) => $this->value = $next;
    }
    int $other { get => $this->value; }
    function getValue(): int { return $this->value; }
}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let calls = &analysis.info.property_accessor_calls;
    assert_eq!(
        calls.len(),
        2,
        "only other hook and ordinary method call the getter"
    );
    for (span, call) in calls {
        assert_eq!(&source[span.start..span.end], "$this->value");
        assert!(call.getter.is_some());
        assert!(call.setter.is_none());
    }
    assert_hook_php_execution(
        &format!(
            r#"{source}
function main(): void {{
    let writable $counter = new Counter();
    $counter->value = 42;
    echo "{{$counter->value}} {{$counter->other}} " . $counter->getValue();
}}
"#
        ),
        "42 42 42",
    );
}

#[test]
fn accessor_calls_specialize_the_declaring_generic_owner() {
    let source = r#"
open class Value<T> {
    function __construct(take T $initial) {}
    T $value { get => $this->initial; }
}
class IntValue extends Value<int> {
    function __construct(override int $initial) { parent::__construct($initial); }
}
function inspect(IntValue $value): int { return $value->value; }
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let call = analysis
        .info
        .property_accessor_calls
        .values()
        .next()
        .unwrap()
        .getter
        .as_ref()
        .unwrap();
    let doriac::types::ResolvedType::Class(owner) = &call.declaring_type else {
        panic!("expected declaring class")
    };
    assert_eq!(owner.name, "Value");
    assert_eq!(
        owner.arguments,
        [doriac::types::ResolvedType::Integer(
            doriac::numeric::IntegerType::Int64
        )]
    );
    assert_eq!(call.return_type, owner.arguments[0]);
}

#[test]
fn accessor_errors_follow_the_normal_catch_and_declaration_rules() {
    let declarations = r#"
class Failure implements Error { string $message = "failure"; }
class Counter { int $value { get throws Failure { throw new Failure(); } } }
"#;
    for (body, unhandled) in [
        ("let $value = $counter->value;", true),
        (
            "try { let $value = $counter->value; } catch (Failure) {}",
            false,
        ),
    ] {
        let source =
            format!("{declarations} function inspect(Counter $counter): void {{ {body} }}");
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
        assert_eq!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0631"),
            unhandled,
            "{source}: {:?}",
            analysis.diagnostics
        );
        if !unhandled {
            assert!(
                analysis.diagnostics.is_empty(),
                "{:?}",
                analysis.diagnostics
            );
        }
    }
}

#[test]
fn accepted_hook_syntax_is_not_silently_lowered_as_stored_properties() {
    for (declaration, expected) in [
        (r#"class Example { int $value { get => 42; } }"#, "42"),
        (
            r#"class Example { writable string $value = "stored" {
            get => "hook:" . $this->value;
            set (string $next) => $this->value = $next;
        } }"#,
            "hook:stored",
        ),
        (
            r#"trait Values { int $value { get => 42; } }
            class Example { uses Values; }"#,
            "42",
        ),
        (
            r#"interface Value { int $value { get; } }
            class Example implements Value { int $value { get => 42; } }"#,
            "42",
        ),
    ] {
        let source = format!("{declaration} function main(): void {{ let $value = new Example(); echo $value->value; }}");
        assert_hook_php_execution(&source, expected);
    }
}

#[test]
fn ordinary_get_set_methods_and_stored_properties_keep_their_contracts() {
    let source = r#"
class Example {
    writable int $value = 0;
    function get(): int { return $this->value; }
    writable function set(int $next): void { $this->value = $next; }
}
function main(): void {
    let writable $example = new Example();
    $example->set(42);
    echo $example->get();
}
"#;
    doriac::lower_source_to_mir("ordinary.doria", source).expect("get/set remain contextual");
}

#[test]
fn hook_types_effects_and_bodies_use_the_shared_namespace_resolver() {
    use doriac::ast::{ClassMember, Expr, Item};

    let source = r#"
namespace App;
use App\Payload as Value;
use App\Failure as Problem;
use App\make as createValue;
class Payload {}
class Failure implements Error { string $message = "failure"; }
function make(): Payload { return new Payload(); }
interface Holder<T> {
    writable Value $value { get throws Problem; set (take Value $next) throws Problem; }
    T $generic { get; }
}
trait HasValue {
    writable Value $value {
        get throws Problem { return createValue(); }
        set (take Value $next) throws Problem { throw new Problem(); }
    }
}
class Example {
    writable Value $value = new Value() {
        get throws Problem { return createValue(); }
        set (take Value $next) throws Problem { throw new Problem(); }
    }
}
"#;
    let program = doriac::parse_source("hooks.doria", source).expect("hook declarations parse");
    let context = doriac::names::CompilationContext::standalone("hooks.doria");
    let resolved = doriac::names::resolve_program(&program, &context).expect("imports resolve");
    let mut properties = Vec::new();
    for item in &resolved.program.items {
        match item {
            Item::Interface(interface) => {
                assert_eq!(interface.properties[1].ty.name, "T");
                properties.push(&interface.properties[0]);
            }
            Item::Class(class) => {
                properties.extend(class.members.iter().filter_map(|member| match member {
                    ClassMember::Property(property) if !property.hooks.is_empty() => Some(property),
                    _ => None,
                }))
            }
            Item::Trait(trait_decl) => {
                properties.extend(trait_decl.members.iter().filter_map(|member| match member {
                    ClassMember::Property(property) => Some(property),
                    _ => None,
                }))
            }
            _ => {}
        }
    }
    assert_eq!(properties.len(), 3);
    let mut constructed = Vec::new();
    let mut called = Vec::new();
    for property in properties {
        assert_eq!(property.ty.name, "App\\Payload");
        assert_eq!(
            property.hooks[1].parameter.as_ref().unwrap().ty.name,
            "App\\Payload"
        );
        for hook in &property.hooks {
            assert_eq!(
                hook.throws.as_ref().unwrap().entries[0].ty.name,
                "App\\Failure"
            );
        }
        doriac::ast::visit::property(property, &mut |expression| match expression {
            Expr::New { class_type, .. } => constructed.push(class_type.name.clone()),
            Expr::FunctionCall { name, .. } => called.push(name.clone()),
            _ => {}
        });
    }
    assert_eq!(
        constructed,
        ["App\\Failure", "App\\Payload", "App\\Failure"]
    );
    assert_eq!(called, ["App\\make", "App\\make"]);
}

#[test]
fn hook_requirements_do_not_make_stored_interface_fields_valid() {
    let diagnostics = doriac::parse_source(
        "stored-interface.doria",
        "interface Invalid { int $value; }",
    )
    .expect_err("interface hooks do not introduce instance storage");
    assert!(diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "E0749"));
}

#[test]
fn setters_require_writable_properties_in_every_declaration_context() {
    use doriac::diagnostics::DiagnosticKind;

    for declaration in [
        "class Example { int $value { set (int $next) => $this->value = $next; } }",
        "class Example { int $value { get => 0; set (int $next) {} } }",
        "trait Example<T> { T $value { set (take T $next) => $this->value = $next; } }",
        "interface Example { int $value { set (int $next); } }",
        "interface Example<T> { T $value { get; set (take T $next); } }",
    ] {
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", declaration)
            .expect("setter declarations parse before semantic validation");
        let findings: Vec<_> = analysis
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0765")
            .collect();
        assert_eq!(
            findings.len(),
            1,
            "{declaration}: {:?}",
            analysis.diagnostics
        );
        let diagnostic = findings[0];
        assert_eq!(diagnostic.kind, DiagnosticKind::Language);
        assert_eq!(
            &declaration[diagnostic.span.start..diagnostic.span.end],
            "set"
        );
        assert!(diagnostic.message.contains("writable"));
    }
}

#[test]
fn writable_getter_and_setter_requirements_remain_distinct() {
    for declaration in [
        "class Example { writable int $value { set (int $next) => $this->value = $next; } }",
        "trait Example<T> { writable T $value { set (take T $next) => $this->value = $next; } }",
        "interface Example { writable int $value { get; set (int $next); } }",
        "class Example { int $value { writable get => 42; } }",
        "trait Example { int $value { writable get => 42; } }",
        "interface Example { int $value { writable get; } }",
    ] {
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", declaration)
            .expect("accepted accessor declarations parse");
        assert!(
            !analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0765"),
            "{declaration}: {:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn computed_hooks_do_not_require_field_initialization() {
    for declaration in [
        "class Example { int $value { get => 42; } }",
        "class Example { int $value { get => 42; } function __construct() {} }",
        "trait Computed { int $value { get => 42; } } class Example { uses Computed; }",
        "class Example { int $value { writable get => 42; } }",
        "class Example { writable int $value { get => 42; set (int $next) {} } }",
    ] {
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", declaration)
            .expect("computed hook declarations parse");
        assert!(
            !analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0500"),
            "{declaration}: {:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn backed_hooks_keep_definite_initialization_checks() {
    for (declaration, missing_initialization) in [
        (
            "class Example { int $value { get => $this->value; } }",
            true,
        ),
        (
            "class Example { int $value = 42 { get => $this->value; } }",
            false,
        ),
        (
            "class Example { int $value { get => ($this)->value; } function __construct() {} }",
            true,
        ),
        (
            "trait Backed { int $value { get => $this->value; } } class Example { uses Backed; }",
            true,
        ),
        ("class Example { int $value; }", true),
        ("class Example { int $value = 42; }", false),
    ] {
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", declaration)
            .expect("stored declarations parse");
        assert_eq!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0500"),
            missing_initialization,
            "{declaration}: {:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn hook_bodies_use_method_type_receiver_and_effect_checks() {
    for (source, code) in [
        (r#"class Example { int $value { get => "wrong"; } }"#, "E0404"),
        ("class Example { writable int $value = 0 { get { $this->value = 1; return $this->value; } } }", "E0201"),
        (r#"class Failure implements Error { string $message = "failure"; }
            class Example { int $value { get { throw new Failure(); } } }"#, "E0631"),
    ] {
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
        assert!(analysis.diagnostics.iter().any(|diagnostic| diagnostic.code == code), "{source}: {:?}", analysis.diagnostics);
    }
    let valid = r#"
class Failure implements Error { string $message = "failure"; }
class Example {
    writable int $value = 0 {
        writable get { $this->value += 1; return $this->value; }
        set (int $next) throws Failure {
            if ($next < 0) { throw new Failure(); }
            $this->value = $next;
        }
    }
}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", valid).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn borrowed_get_contracts_preserve_receiver_provenance() {
    use doriac::ast::{ClassMember, Item};
    use doriac::symbols::{BorrowSource, ReturnBorrow};

    let source = r#"
class Book {}
interface Shelf { Book $featured { borrowed get; } }
interface Factory { Book $created { get; } }
class Example {
    Book $book = new Book();
    Book $featured { borrowed get => $this->book; }
    Book $inferred { get => $this->book; }
    Book $created { get => new Book(); }
}
"#;
    let (program, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    for item in &program.items {
        let properties: Vec<_> = match item {
            Item::Interface(interface) => interface.properties.iter().collect(),
            Item::Class(class) => class
                .members
                .iter()
                .filter_map(|member| match member {
                    ClassMember::Property(property) if !property.hooks.is_empty() => Some(property),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        for property in properties {
            let borrow = analysis
                .info
                .return_borrows
                .get(&property.hooks[0].span)
                .copied();
            assert_eq!(
                borrow,
                (property.name != "created").then_some(ReturnBorrow {
                    source: BorrowSource::Receiver,
                    writable: false,
                    kind: doriac::types::ReturnBorrowKind::Value,
                }),
                "{}",
                property.name
            );
        }
    }
}

#[test]
fn hook_bodies_share_nullable_and_exact_type_flow_analysis() {
    for owner in ["class", "trait"] {
        let source = format!(
            r#"
class Missing implements Error {{ string $message = "missing"; }}
{owner} Example {{
    writable ?int $cached = null;
    int $total {{
        writable get {{
            let $value = $this->cached;
            if ($value != null) {{ return $value; }}
            $this->cached = 0;
            return 0;
        }}
    }}
    int $required {{
        get throws Missing {{
            let $value = $this->cached;
            if ($value == null) {{ throw new Missing(); }}
            return $value;
        }}
    }}
    writable ?int $number {{
        set (?int $value) {{
            if ($value is int) {{ $this->cached = $value + 1; }}
        }}
    }}
    int $nested {{
        get {{
            let $read = function (?int $value): int {{
                if ($value != null) {{ return $value; }}
                return 0;
            }};
            return $read($this->cached);
        }}
    }}
}}
"#
        );
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
        assert!(
            analysis.diagnostics.is_empty(),
            "{owner}: {:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn hook_body_flow_facts_are_invalidated_by_writable_calls() {
    for owner in ["class", "trait"] {
        let source = format!(
            r#"
function clear(writable ?int $value): void {{ $value = null; }}
{owner} Example {{
    ?int $cached = null;
    int $value {{
        get {{
            let writable $current = $this->cached;
            if ($current != null) {{
                clear($current);
                return $current;
            }}
            return 0;
        }}
    }}
}}
"#
        );
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0404"),
            "{owner}: {:?}",
            analysis.diagnostics
        );
    }
}

#[test]
fn borrowed_get_cannot_claim_an_owned_result() {
    let source = "class Book {} class Example { Book $book { borrowed get => new Book(); } }";
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    let diagnostic = analysis
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "E0766")
        .expect("owned result violates the explicit borrowed getter contract");
    assert_eq!(
        &source[diagnostic.span.start..diagnostic.span.end],
        "borrowed"
    );
}

#[test]
fn hook_borrow_inference_follows_methods_without_changing_result_access() {
    use doriac::ast::{ClassMember, Item};
    use doriac::symbols::{BorrowSource, ReturnBorrow};

    let source = r#"
class Book {}
class Example {
    Book $book = new Book();
    writable int $reads = 0;
    function getBook(): Book { return $this->book; }
    Book $featured { borrowed get => $this->getBook(); }
    Book $counted {
        writable borrowed get {
            $this->reads++;
            return $this->getBook();
        }
    }
}
"#;
    let (program, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let Item::Class(class) = &program.items[1] else {
        panic!("expected Example")
    };
    for member in &class.members {
        if let ClassMember::Property(property) = member {
            for hook in &property.hooks {
                assert_eq!(
                    analysis.info.return_borrows.get(&hook.span),
                    Some(&ReturnBorrow {
                        source: BorrowSource::Receiver,
                        writable: false,
                        kind: doriac::types::ReturnBorrowKind::Value,
                    })
                );
            }
        }
    }
}

#[test]
fn generic_hook_bodies_register_concrete_class_instantiations() {
    let source = r#"
class Box<T> {}
class Factory<T> {
    Box<T> $result { get => new Box<T>(); }
}
function main(): void { let $factory = new Factory<int>(); }
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert!(
        analysis
            .info
            .classes
            .iter()
            .any(|class| class.declaration_name == "Box"
                && class.arguments
                    == [doriac::types::ResolvedType::Integer(
                        doriac::numeric::IntegerType::Int64
                    )]),
        "{:?}",
        analysis.info.classes
    );
}

#[test]
fn hooks_cannot_invoke_parent_construction_outside_the_constructor() {
    let source = r#"
open class Base { function __construct() {} }
class Child extends Base {
    int $value { get { parent::__construct(); return 0; } }
}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0736"),
        "{:?}",
        analysis.diagnostics
    );
}

#[test]
fn trait_hook_bodies_keep_the_trait_parent_boundary() {
    for body in [
        "get => parent::number();",
        "get { let $callback = fn() => parent::number(); return $callback(); }",
    ] {
        let source = format!("trait Values {{ int $number {{ {body} }} }}");
        let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", &source).unwrap();
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0756"),
            "{:?}",
            analysis.diagnostics
        );
        assert!(!analysis.info.contracts.traits[0].valid);
    }
}

#[test]
fn hook_returned_closures_keep_owned_and_borrowed_provenance() {
    use doriac::ast::{ClassMember, Item};
    use doriac::symbols::{BorrowSource, ReturnBorrow};

    let source = r#"
class Example {
    int $value = 42;
    function(): int $stored = fn() => 42;
    function(): int $owned { get => fn() => 42; }
    function(): int $borrowed { get => fn() with ($this) => $this->value; }
    function(): int $explicit { borrowed get => $this->stored; }
}
"#;
    let (program, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert_eq!(analysis.info.closures.len(), 3);
    let Item::Class(class) = &program.items[0] else {
        panic!("expected Example")
    };
    for member in &class.members {
        if let ClassMember::Property(property) = member {
            for hook in &property.hooks {
                assert_eq!(
                    analysis.info.return_borrows.get(&hook.span).copied(),
                    (property.name != "owned").then_some(ReturnBorrow {
                        source: BorrowSource::Receiver,
                        writable: false,
                        kind: if property.name == "explicit" {
                            doriac::types::ReturnBorrowKind::Value
                        } else {
                            doriac::types::ReturnBorrowKind::Retained
                        },
                    }),
                    "{}",
                    property.name
                );
            }
        }
    }
}

#[test]
fn hook_owned_results_preserve_retained_source_dependencies() {
    let source = r#"
class Cursor implements Iterator<int> {
    function __construct(borrow List<int> $source) {}
    function hasCurrent(): bool { return false; }
    function getCurrent(): int { return $this->source[0]; }
    writable function advance(): void {}
}
class Source {
    List<int> $values = [1];
    Cursor $cursor { get => new Cursor($this->values); }
}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    let summary = analysis
        .info
        .retained_callables
        .iter()
        .filter(|(span, _)| span.source == doriac::source::SourceId::default())
        .find_map(|(span, summary)| {
            source[span.start..span.end]
                .starts_with("get =>")
                .then_some(summary)
        })
        .expect("getter is included in retained-source analysis");
    assert_eq!(
        summary.returns,
        [doriac::ownership::RetainedSource {
            source: doriac::symbols::BorrowSource::Receiver,
            inherited: false,
        }]
    );
}

#[test]
fn trait_getter_contracts_use_refined_borrow_and_capture_facts() {
    let source = r#"
class Book {}
trait Shelf {
    Book $book = new Book();
    int $count = 1;
    function getBook(): Book { return $this->book; }
    Book $featured { borrowed get => $this->getBook(); }
    function(): int $counter { get => fn() with ($this) => $this->count; }
}
"#;
    let (_, analysis) = doriac::analyze_source_for_ide("hooks.doria", source).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
}
