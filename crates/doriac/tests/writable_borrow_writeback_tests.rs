#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;

#[path = "common/php_execution.rs"]
mod php_execution;

const DECLARATIONS: &str = r#"
interface Counted { writable function bump(): void; function getValue(): int; }
open class Counter implements Counted {
    writable int $value = 7;
    writable function bump(): void { $this->value++; }
    function getValue(): int { return $this->value; }
}
class Child extends Counter {}
class Other implements Counted {
    writable function bump(): void {}
    function getValue(): int { return 99; }
}
class Holder {
    writable Counter $stored = new Counter();
    Counter $current { writable borrowed get => $this->stored; }
}
class NullableHolder {
    writable ?Counter $stored = new Counter();
}
function lend(writable Counter $counter): ?Counter { return $counter; }
function lendNullable(writable ?Counter $counter): ?Counter { return $counter; }
function lendBase(writable Child $child): Counter { return $child; }
function lendInterface(writable Counter $counter): Counted { return $counter; }
function invoke(function(writable ?Counter): void $callback, writable ?Counter $counter): void {
    $callback($counter);
}
function forward(function(writable ?Counter): void $callback, writable ?Counter $counter): void {
    invoke($callback, $counter);
}
function generic<T>(function(writable ?T): void $callback, writable T $value): void {
    $callback($value);
}
function recursive(function(writable ?Counter): void $callback, writable ?Counter $counter, int $depth): void {
    if ($depth > 0) { recursive($callback, $counter, $depth - 1); }
    else { $callback($counter); }
}
"#;

fn fixture_mir(name: &str, source: &str) -> doriac::mir::Program {
    doriac::lower_source_to_mir(name, source).unwrap_or_else(|diagnostics| {
        panic!("MIR lowering failed for {name}:\n{source}\n{diagnostics:#?}")
    })
}

fn validate_fixture(program: &doriac::mir::Program, source: &str) {
    doriac::mir_validation::validate_program(program).unwrap_or_else(|error| {
        panic!("Unmodified fixture validation failed:\n{source}\n{error:#?}")
    });
}

fn assert_native_profile(
    program: &doriac::mir::Program,
    profile: doriac::backend::NativeProfile,
    expected: &str,
    source: &str,
) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        native_execution::assert_native_execution(program, profile, expected);
    }));
    if let Err(payload) = result {
        let detail = payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| payload.downcast_ref::<&str>().copied())
            .unwrap_or("native runner panicked with a non-string payload");
        panic!("Native {profile:?} case failed:\n{source}\n{detail}");
    }
}

fn assert_native_output(program: &doriac::mir::Program, expected: &str, source: &str) {
    validate_fixture(program, source);
    let result = doriac::mir_interpreter::interpret(program)
        .unwrap_or_else(|error| panic!("Interpreter failed:\n{source}\n{error:#?}"));
    assert_eq!(result.stdout, expected.as_bytes(), "{source}");
    assert_eq!(result.stderr, b"", "{source}");
    assert_eq!(result.exit_status, 0, "{source}");
    assert_native_profile(
        program,
        doriac::backend::NativeProfile::Fast,
        expected,
        source,
    );
    #[cfg(feature = "llvm-backend")]
    assert_native_profile(
        program,
        doriac::backend::NativeProfile::Release,
        expected,
        source,
    );
}

fn assert_execution(source: &str, expected: &str) {
    let hir = doriac::lower_source("writeback-compatible.doria", source)
        .unwrap_or_else(|diagnostics| panic!("{source}\n{diagnostics:#?}"));
    let program = doriac::mir_lowering::lower_program(&hir)
        .unwrap_or_else(|diagnostics| panic!("MIR lowering failed:\n{source}\n{diagnostics:#?}"));
    assert_native_output(&program, expected, source);
    let php = doriac::codegen_php::generate(&hir, Some(&program))
        .unwrap_or_else(|diagnostics| panic!("PHP generation failed:\n{source}\n{diagnostics:#?}"));
    let script = format!(
        "{}\n__DoriaFunction_6d61696e();",
        php.strip_prefix("<?php").unwrap()
    );
    let result = php_execution::Script::new(&script)
        .command(&[])
        .output()
        .unwrap_or_else(|error| panic!("PHP runner failed:\n{source}\n{error:#?}"));
    assert_eq!(result.status.code(), Some(0), "{source}\n{result:?}");
    assert_eq!(result.stderr, b"", "{source}");
    assert_eq!(result.stdout, expected.as_bytes(), "{source}");
}

#[test]
fn array_collection_clear_keeps_receiver_homes_live_and_releases_contents_once() {
    assert_execution(
        r#"
class Token {
    function __construct(int $id) {}
    function __destruct() { try { echo $this->id; } catch (Error) {} }
}
function clearList(writable List<Token> $values): void { $values->clear(); }
function clearMap(writable Dictionary<string, Token> $values): void { $values->clear(); }
function main(): void {
    writable List<Token> $values = [new Token(1), new Token(2)];
    clearList($values);
    echo ":{$values->count};";
    $values->add(new Token(3));
    $values->clear();
    $values->clear();
    echo ":{$values->count};";
    writable Dictionary<string, Token> $map = ["a" => new Token(4), "b" => new Token(5)];
    clearMap($map);
    echo ":{$map->count};";
    $map->set("c", new Token(6));
    $map->clear();
    $map->clear();
    echo ":{$map->count};";
}
"#,
        "21:0;3:0;54:0;6:0;",
    );
}

#[test]
fn named_move_parameters_replace_caller_storage_and_destroy_previous_owners() {
    assert_execution(
        r#"
class Item {
    function __construct(int $value) {}
    function __destruct() { try { echo "drop{$this->value};"; } catch (Error) {} }
}
function replace(writable Item $target, int $value): void {
    $target = new Item($value);
}
function forward(writable Item $target): void { replace($target, 3); }
function changeCopy(writable int $value): void { $value = 99; }
function main(): void {
    {
        let writable $owner = new Item(1);
        replace($owner, 2);
        echo "{$owner->value};";
        forward($owner);
        echo "{$owner->value};";
    }
    let writable $copy = 7;
    changeCopy($copy);
    echo $copy;
}
"#,
        "drop1;2;drop2;3;drop3;7",
    );
}

#[test]
fn named_move_parameters_preserve_dispatch_captures_and_checked_replacement_order() {
    assert_execution(
        r#"
class Item {
    function __construct(int $value) {}
    function __destruct() { try { echo "drop{$this->value};"; } catch (Error) {} }
}
class Failure implements Error { string $message = "failure"; }
function replace(writable Item $target, int $value): void { $target = new Item($value); }
function replaceGeneric<T>(writable T $target, take T $value): void { $target = $value; }
interface Changer { function change(writable Item $target, int $value): void; }
open class ChangerBase implements Changer {
    open function change(writable Item $target, int $value): void { replace($target, $value); }
}
class ChangerChild extends ChangerBase {
    override function change(writable Item $target, int $value): void {
        replace($target, $value + 1);
    }
}
class Initializer {
    function __construct(parameter writable Item $target) { replace($target, 2); }
}
function replaceAndObserve(writable Item $target): function(): int {
    $target = new Item(7);
    return fn() with ($target) => $target->value;
}
function replaceThenFail(writable Item $target): void throws Failure {
    $target = new Item(8);
    throw new Failure();
}
function fail(): Item throws Failure { throw new Failure(); }
function relayFailure(int $depth): Item throws Failure {
    if ($depth > 0) { return relayFailure($depth - 1); }
    return fail();
}
function failedReplacement(writable Item $target): void throws Failure { $target = relayFailure(2); }
function main(): void {
    let writable $owner = new Item(1);
    let $initializer = new Initializer($owner);
    echo "{$owner->value};";
    ChangerBase $virtual = new ChangerChild();
    $virtual->change($owner, 3);
    echo "{$owner->value};";
    Changer $erased = new ChangerChild();
    $erased->change($owner, 5);
    echo "{$owner->value};";
    let $observe = replaceAndObserve($owner);
    echo "{$observe()};";
    try { replaceThenFail($owner); } catch (Failure) { echo "caught;"; }
    echo "{$owner->value};";
    try { failedReplacement($owner); } catch (Failure) { echo "kept;"; }
    echo "{$owner->value};";
    replaceGeneric($owner, new Item(9));
    echo "{$owner->value};";
}
"#,
        "drop1;2;drop2;4;drop4;6;drop6;7;drop7;caught;8;kept;8;drop8;9;drop9;",
    );
}

#[test]
fn named_move_parameters_replace_collection_mixed_and_callable_homes() {
    assert_execution(
        r#"
class Item {
    function __construct(int $value) {}
    function __destruct() { try { echo "drop{$this->value};"; } catch (Error) {} }
}
function replaceList(writable List<Item> $target): void { $target = [new Item(2)]; }
function replaceMixed(writable mixed $target): void { $target = new Item(4); }
function replaceCallback(writable function(): int $target): void { $target = fn() => 6; }
function main(): void {
    let writable $items = [new Item(1)];
    replaceList($items);
    echo "{$items[0]->value};";
    writable mixed $boxed = new Item(3);
    replaceMixed($boxed);
    if ($boxed is Item) { echo "{$boxed->value};"; }
    let writable $callback = fn() => 5;
    replaceCallback($callback);
    echo "{$callback()};";
}
"#,
        "drop1;2;drop3;4;6;drop4;drop2;",
    );
}

#[test]
fn named_move_parameters_refresh_the_replaced_owner_after_nested_rhs_calls() {
    for body in [
        "$owner = advance($owner);",
        "replace($owner);",
        "let $replace = function(writable Item $target): void { $target = advance($target); }; $replace($owner);",
    ] {
        let source = format!(
            r#"
class Item {{
    function __construct(int $value) {{}}
    function __destruct() {{ try {{ echo "drop{{$this->value}};"; }} catch (Error) {{}} }}
}}
function advance(writable Item $target): Item {{
    $target = new Item(2);
    return new Item(3);
}}
function replace(writable Item $target): void {{ $target = advance($target); }}
function main(): void {{
    let writable $owner = new Item(1);
    {body}
    echo "{{$owner->value}};";
}}
"#
        );
        assert_execution(&source, "drop1;drop2;3;drop3;");
    }
}

#[test]
fn named_move_parameters_preserve_generic_copy_and_move_specializations() {
    assert_execution(
        r#"
class Item {
    function __construct(int $value) {}
    function __destruct() { try { echo "drop{$this->value};"; } catch (Error) {} }
}
function clearGeneric<T>(writable ?T $target): void { $target = null; }
function forwardGeneric<T>(writable ?T $target): void { clearGeneric($target); }
function main(): void {
    let writable $copy = 7;
    forwardGeneric($copy);
    echo "{$copy};";
    writable ?Item $owner = new Item(1);
    forwardGeneric($owner);
    echo $owner == null;
}
"#,
        "7;drop1;true",
    );
}

#[test]
fn named_move_parameters_enforce_physical_home_types_in_source_and_mir() {
    let declarations = format!(
        r#"{DECLARATIONS}
function clear(writable ?Counter $target): void {{ $target = null; }}
function replace(writable Counter $target): void {{ $target = new Counter(); }}
function replaceInterface(writable Counted $target): void {{ $target = new Other(); }}
function observe(writable ?Counter $target): void {{ $target?->bump(); }}
function clearGeneric<T>(writable ?T $target): void {{ $target = null; }}
function forwardGeneric<T>(writable ?T $target): void {{ clearGeneric($target); }}
"#
    );
    for body in [
        "let writable $owner = new Counter(); clear($owner);",
        "let writable $owner = new Child(); replace($owner);",
        "let writable $owner = new Counter(); replaceInterface($owner);",
        "let writable $owner = new Counter(); forwardGeneric($owner);",
    ] {
        let source = format!("{declarations} function main(): void {{ {body} }}");
        let diagnostics = doriac::check_source("named-writeback-types.doria", &source)
            .err()
            .unwrap_or_else(|| {
                panic!("replacement must fit the caller's physical storage:\n{source}")
            });
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0403"
                    && diagnostic.message.contains("writable borrow can replace")),
            "{source}\n{diagnostics:#?}"
        );
    }
    for body in [
        "let writable $owner = new Counter(); observe($owner);",
        "let writable $owner = new Holder(); observe($owner->stored);",
        "writable ?Counter $left = null; let writable $right = new Counter(); let writable $view = lendNullable($left) ?? lend($right); observe($view);",
    ] {
        use doriac::mir::{NullableClassExpression, Rvalue, Statement, Terminator, Type};
        let source = format!("{declarations} function main(): void {{ {body} }}");
        let mut program = fixture_mir("forged-named-writeback.doria", &source);
        validate_fixture(&program, &source);
        let function = program
            .functions
            .iter_mut()
            .find(|function| function.name == "observe")
            .expect("named observing function");
        let target = function.params[0];
        assert!(function.is_writable_borrowed_place(target));
        let Type::NullableClass(class) = function.locals[target.0].ty else {
            unreachable!()
        };
        function
            .blocks
            .iter_mut()
            .find(|block| matches!(block.terminator, Terminator::ReturnVoid))
            .unwrap()
            .statements
            .push(Statement::AssignLocal {
                target,
                value: Rvalue::NullableClass(NullableClassExpression::Null(class)),
            });
        let error = doriac::mir_validation::validate_program(&program)
            .expect_err("forged named writes must preserve original nonnullable storage");
        assert!(error.message.contains("storage"), "{source}\n{error:#?}");
    }
}

#[test]
fn widened_writable_views_reject_incompatible_slot_replacements() {
    for body in [
        "let writable $left = new Counter(); let writable $right = new Child(); let writable $view = lend($left) ?? lendBase($right); let $replace = function(writable Counter $counter): void { $counter = new Counter(); }; $replace($view);",
        "let writable $left = new Child(); let writable $right = new Counter(); let writable $view = lend($left) ?? $right; let $replace = function(writable Counter $counter): void { $counter = new Counter(); }; $replace($view);",
        "writable ?Counter $left = null; let writable $right = new Counter(); let writable $view = lendNullable($left) ?? lend($right); let $clear = function(writable ?Counter $counter): void { $counter = null; }; $clear($view);",
        "let writable $owner = new Counter(); let writable $view = lend($owner); let $clear = function(writable ?Counter $counter): void { $counter = null; }; $clear($view);",
        "let writable $owner = new Counter(); let $clear = function(writable ?Counter $counter): void { $counter = null; }; $clear($owner);",
        "let writable $owner = new Child(); let writable $view = lendBase($owner); let $replace = function(writable Counter $counter): void { $counter = new Counter(); }; $replace($view);",
        "let writable $owner = new Counter(); let writable $view = lendInterface($owner); let $replace = function(writable Counted $counter): void { $counter = new Other(); }; $replace($view);",
        "let writable $owner = new Holder(); let writable $view = lend($owner->stored); let $clear = function(writable ?Counter $counter): void { $counter = null; }; $clear($view);",
        "let writable $owner = new Counter(); let writable $view = lend($owner); let $clear = function(writable ?Counter $counter): void { $counter = null; }; forward($clear, $view);",
        "let writable $owner = new Counter(); let $clear = function(writable ?Counter $counter): void { $counter = null; }; generic($clear, $owner);",
        "let writable $owner = new Counter(); let writable $view = lend($owner); let writable $clear = function(): void with (writable $view) { $view = null; }; $clear();",
        "let writable $owner = new Counter(); let writable $view = lend($owner); let $clear = function(writable ?Counter $counter): void { $counter = null; }; let $delegate = function(function(writable ?Counter): void $callback, writable ?Counter $counter): void { $callback($counter); }; $delegate($clear, $view);",
        "let writable $owner = new Counter(); let writable $view = lend($owner); let $clear = function(writable ?Counter $counter): void { $counter = null; }; let $delegate = function(writable ?Counter $counter): void with ($clear) { $clear($counter); }; forward($delegate, $view);",
        "let writable $owner = new Counter(); let $alias = function(writable Counter $counter): ?Counter { return $counter; }; let writable $view = $alias($owner); let $clear = function(writable ?Counter $counter): void { $counter = null; }; $clear($view);",
        "let writable $owner = new Counter(); let writable $view = lend($owner); let $clear = function(writable ?Counter $counter): void { $counter = null; }; recursive($clear, $view, 2);",
        "let writable $owners = [new Counter()]; let writable $view = lend($owners[0]); let $clear = function(writable ?Counter $counter): void { $counter = null; }; $clear($view);",
        "let writable $number = 7; let $clear = function(writable ?int $number): void { $number = null; }; $clear($number);",
    ] {
        let source = format!("{DECLARATIONS} function main(): void {{ {body} }}");
        let (_, analysis) = doriac::analyze_source_for_ide("writeback-rejection.doria", &source).unwrap();
        assert!(
            analysis.diagnostics.iter().any(|diagnostic| diagnostic.code == "E0403" && diagnostic.message.contains("writable borrow can replace")),
            "{body}\n{:#?}", analysis.diagnostics
        );
        assert!(doriac::lower_source_to_mir("writeback-rejection.doria", &source).is_err());
    }
}

#[test]
fn compatible_replacements_and_widened_in_place_mutation_remain_valid() {
    for (body, expected) in [
        ("writable ?Counter $left = null; writable ?Counter $right = new Counter(); { let writable $view = lendNullable($left) ?? lendNullable($right); let $clear = function(writable ?Counter $counter): void { $counter = null; }; $clear($view); } echo ($left?->value ?? 0); echo ($right?->value ?? 0);", "00"),
        ("let writable $owner = new Counter(); let writable $view = lend($owner); $view?->bump(); echo ($view?->value ?? 0);", "8"),
        ("let writable $owner = new Counter(); let writable $view = lend($owner); let $replace = function(writable ?Counter $counter): void { $counter = new Counter(); }; $replace($view); $view?->bump(); echo ($view?->value ?? 0);", "8"),
        ("let writable $owner = new Child(); let writable $view = lendBase($owner); let $replace = function(writable Counter $counter): void { $counter = new Child(); }; $replace($view); $view->bump(); echo $view->value;", "8"),
        ("let writable $owner = new Child(); let writable $view = lendInterface($owner); let $replace = function(writable Counted $counter): void { $counter = new Child(); }; $replace($view); $view->bump(); echo $view->getValue();", "8"),
        ("writable ?Counter $owner = new Counter(); let $clear = function(writable ?Counter $counter): void { $counter = null; }; $clear($owner); echo ($owner?->value ?? 0);", "0"),
        ("let writable $owner = new Counter(); let writable $view = lend($owner); let $bump = function(writable ?Counter $counter): void { $counter?->bump(); }; forward($bump, $view); echo ($view?->value ?? 0);", "8"),
        ("let writable $owner = new Counter(); let $bump = function(writable ?Counter $counter): void { $counter?->bump(); }; generic($bump, $owner); echo $owner->value;", "8"),
        ("writable ?Counter $nullable = new Counter(); let $clear = function(writable ?Counter $counter): void { $counter = null; }; forward($clear, $nullable); let writable $owner = new Counter(); let writable $view = lend($owner); let $bump = function(writable ?Counter $counter): void { $counter?->bump(); }; forward($bump, $view); echo ($view?->value ?? 0);", "8"),
        ("let writable $owner = new Counter(); let writable $view = lend($owner); let $bump = function(writable ?Counter $counter): void { $counter?->bump(); }; let $delegate = function(function(writable ?Counter): void $callback, writable ?Counter $counter): void { $callback($counter); }; $delegate($bump, $view); echo ($view?->value ?? 0);", "8"),
        ("let writable $owner = new Counter(); let writable $view = lend($owner); let $bump = function(writable ?Counter $counter): void { $counter?->bump(); }; let $delegate = function(writable ?Counter $counter): void with ($bump) { $bump($counter); }; forward($delegate, $view); echo ($view?->value ?? 0);", "8"),
        ("let writable $owner = new Counter(); let $alias = function(writable Counter $counter): ?Counter { return $counter; }; let writable $view = $alias($owner); $view?->bump(); echo ($view?->value ?? 0);", "8"),
        ("let writable $owner = new Counter(); let writable $view = lend($owner); let $bump = function(writable ?Counter $counter): void { $counter?->bump(); }; recursive($bump, $view, 2); echo ($view?->value ?? 0);", "8"),
        ("let writable $owner = new NullableHolder(); let writable $view = lendNullable($owner->stored); let $clear = function(writable ?Counter $counter): void { $counter = null; }; $clear($view); echo ($view?->value ?? 0);", "0"),
        ("let writable $number = 7; let $increment = function(writable ?int $number): void { $number = ($number ?? 0) + 1; }; $increment($number); echo $number;", "8"),
    ] {
        let source = format!("{DECLARATIONS} function main(): void {{ {body} }}");
        assert_execution(&source, expected);
    }
}

#[test]
fn uncovered_known_callers_are_checked_before_their_generic_helpers() {
    let source = format!(
        r#"{DECLARATIONS}
function unusedCaller(): void {{
    let writable $owner = new Counter();
    let $run = function(writable ?Counter $counter): void {{ $counter?->bump(); }};
    generic($run, $owner);
}}
function main(): void {{}}
"#
    );
    let program = fixture_mir("uncovered-known-caller.doria", &source);
    validate_fixture(&program, &source);
}

#[test]
fn uncovered_recursive_components_preserve_known_callback_contexts() {
    let source = format!(
        r#"{DECLARATIONS}
function relay(function(writable ?Counter): void $callback, writable Counter $owner, int $depth): void {{
    if ($depth > 0) {{ bounce($callback, $owner, $depth - 1); }}
    else {{ $callback($owner); }}
}}
function bounce(function(writable ?Counter): void $callback, writable Counter $owner, int $depth): void {{
    relay($callback, $owner, $depth);
}}
function unusedCaller(): void {{
    let writable $owner = new Counter();
    let $run = function(writable ?Counter $counter): void {{ $counter?->bump(); }};
    relay($run, $owner, 2);
}}
function main(): void {{}}
"#
    );
    let program = fixture_mir("uncovered-recursive-caller.doria", &source);
    validate_fixture(&program, &source);
}

#[test]
fn uncovered_callers_forward_known_contexts_through_closure_bodies() {
    let source = format!(
        r#"{DECLARATIONS}
function unusedCaller(): void {{
    let $deferred = function(): void {{
        let writable $owner = new Counter();
        let $run = function(writable ?Counter $counter): void {{ $counter?->bump(); }};
        generic($run, $owner);
    }};
    $deferred();
}}
function main(): void {{}}
"#
    );
    let program = fixture_mir("uncovered-closure-caller.doria", &source);
    validate_fixture(&program, &source);
}

#[test]
fn library_callers_do_not_close_independent_callback_boundaries() {
    let source = format!(
        r#"{DECLARATIONS}
function dispatch(function(writable ?Counter): void $callback, writable Counter $owner): void {{
    $callback($owner);
}}
function main(): void {{
    let writable $owner = new Counter();
    let $run = function(writable ?Counter $counter): void {{ $counter?->bump(); }};
    dispatch($run, $owner);
}}
"#
    );
    let mut program = fixture_mir("open-library-callback.doria", &source);
    validate_fixture(&program, &source);
    program.selected_target.kind = doriac::build_plan::TargetKind::Library;
    program.selected_target.entry_source = None;
    program.selected_entry = None;
    let error = doriac::mir_validation::validate_program(&program).expect_err(&format!(
        "a library input callback remains unbound despite a proved internal caller:\n{source}"
    ));
    assert!(error.message.contains("storage"), "{error:#?}");
}

#[test]
fn uninvoked_returned_closures_forward_captured_known_widened_callbacks() {
    let source = format!(
        r#"{DECLARATIONS}
function defer(writable Counter $owner): function writable(): void {{
    let $run = function(writable ?Counter $counter): void {{ $counter?->bump(); }};
    return function(): void with (writable $owner, take $run) {{
        generic($run, $owner);
    }};
}}
function main(): void {{
    let writable $owner = new Counter();
    {{
        let writable $pending = defer($owner);
        echo "retained;";
    }}
    echo $owner->value;
}}
"#
    );
    // The independently created callback is owned; only the owner contributes
    // a return-borrow root. Definition coverage must still prove the captured
    // callback without treating this never-invoked return as an executable call.
    assert_execution(&source, "retained;7");
}

#[test]
fn uninvoked_returned_closure_definitions_reject_forged_captured_home_writes() {
    use doriac::mir::{NullableClassExpression, Rvalue, Statement, Terminator, Type};

    let source = format!(
        r#"{DECLARATIONS}
function defer(writable ?Counter $view): function writable(): void {{
    return function(): void with (writable $view) {{ $view?->bump(); }};
}}
function main(): void {{
    let writable $owner = new Counter();
    let writable $view = lend($owner);
    let writable $pending = defer($view);
}}
"#
    );
    let mut program = fixture_mir("uninvoked-returned-capture-write.doria", &source);
    validate_fixture(&program, &source);
    let function = program
        .functions
        .iter_mut()
        .find(|function| {
            function.closure.as_ref().is_some_and(|closure| {
                closure.capture_locals.iter().any(|(_, local)| {
                    matches!(function.locals[local.0].ty, Type::NullableClass(_))
                        && function.is_writable_borrowed_place(*local)
                })
            })
        })
        .expect("returned closure's writable nullable capture");
    let target = function
        .closure
        .as_ref()
        .unwrap()
        .capture_locals
        .iter()
        .find_map(|(_, local)| {
            (matches!(function.locals[local.0].ty, Type::NullableClass(_))
                && function.is_writable_borrowed_place(*local))
            .then_some(*local)
        })
        .unwrap();
    let Type::NullableClass(class) = function.locals[target.0].ty else {
        unreachable!()
    };
    function
        .blocks
        .iter_mut()
        .find(|block| matches!(block.terminator, Terminator::ReturnVoid))
        .expect("normal uninvoked closure exit")
        .statements
        .push(Statement::AssignLocal {
            target,
            value: Rvalue::NullableClass(NullableClassExpression::Null(class)),
        });
    let error = doriac::mir_validation::validate_program(&program).expect_err(&format!(
        "an uninvoked returned definition must not write null through its captured nonnullable physical home:\n{source}"
    ));
    assert!(error.message.contains("storage"), "{error:#?}");
    // Forged MIR is checked only, never interpreted or executed natively.
}

#[test]
fn uninvoked_returned_definitions_keep_distinct_caller_capture_contexts() {
    for (owner, callback_body, owner_read) in [
        (
            "let writable $owner = new Counter();",
            "$counter?->bump();",
            "$owner->value",
        ),
        (
            "writable ?Counter $owner = new Counter();",
            "$counter = null;",
            "($owner?->value ?? 0)",
        ),
    ] {
        let source = format!(
            r#"{DECLARATIONS}
function defer(writable ?Counter $view): function writable(): void {{
    let $run = function(writable ?Counter $counter): void {{ {callback_body} }};
    return function(): void with (writable $view, take $run) {{ forward($run, $view); }};
}}
function main(): void {{
    {owner}
    writable ?Counter $nullable = new Counter();
    {{
        let writable $first = defer($owner);
        let writable $second = defer($nullable);
        echo "retained;";
    }}
    echo {owner_read};
    echo ($nullable?->value ?? 0);
}}
"#
        );
        // Each source uses one descriptor with two distinct caller homes. The
        // bump case includes nonnullable and nullable physical homes; the clear
        // case uses only nullable homes. Both callbacks are created inside the
        // factory, so owning them introduces no second borrowed return root.
        // Different callback bodies are covered in separate source fixtures,
        // not as arbitrary function parameters to the same factory.
        assert_execution(&source, "retained;77");
    }
}

#[test]
fn nullable_writable_copy_arguments_preserve_every_integer_width() {
    for ty in [
        "int", "int8", "int16", "int32", "int64", "uint8", "uint16", "uint32", "uint64",
    ] {
        let source = format!(
            r#"
function main(): void {{
    writable {ty} $number = 7;
    let $increment = function(writable ?{ty} $number): void {{
        {ty} $zero = 0;
        {ty} $one = 1;
        $number = ($number ?? $zero) + $one;
    }};
    $increment($number);
    $increment($number);
    echo $number;
}}
"#
        );
        assert_execution(&source, "9");
    }
}

#[test]
fn nullable_writable_copy_arguments_preserve_float_bool_and_repeated_strings() {
    for ty in ["float", "float64"] {
        let source = format!(
            r#"
function main(): void {{
    writable {ty} $number = 1.5;
    let $increment = function(writable ?{ty} $number): void {{
        {ty} $zero = 0.0;
        {ty} $one = 1.0;
        $number = ($number ?? $zero) + $one;
    }};
    $increment($number);
    $increment($number);
    if ($number == 3.5) {{ echo "updated"; }} else {{ echo "lost"; }}
}}
"#
        );
        assert_execution(&source, "updated");
    }
    for (body, expected) in [
        (
            r#"let writable $flag = true; let $toggle = function(writable ?bool $flag): void { $flag = !($flag ?? false); }; $toggle($flag); echo $flag; $toggle($flag); echo $flag;"#,
            "falsetrue",
        ),
        (
            r#"let writable $text = "seed" . "!"; let $append = function(writable ?string $text): void { $text = ($text ?? "") . "."; }; $append($text); $append($text); $append($text); echo $text;"#,
            "seed!...",
        ),
        (
            r#"writable ?int $number = 7; let $clear = function(writable ?int $number): void { $number = null; }; if ($number != null) { $clear($number); } echo ($number ?? 9);"#,
            "9",
        ),
        (
            r#"writable ?string $text = "seed" . "!"; let $clear = function(writable ?string $text): void { $text = null; }; if ($text != null) { $clear($text); } echo ($text ?? "cleared");"#,
            "cleared",
        ),
    ] {
        assert_execution(&format!("function main(): void {{ {body} }}"), expected);
    }
}

#[test]
fn nullable_writable_float32_arguments_preserve_native_precision() {
    let source = r#"
function main(): void {
    writable float32 $number = 1.5;
    let $increment = function(writable ?float32 $number): void {
        float32 $zero = 0.0;
        float32 $one = 1.0;
        $number = ($number ?? $zero) + $one;
    };
    $increment($number);
    $increment($number);
    echo $number;
}
"#;
    let program = fixture_mir("writeback-float32.doria", source);
    assert_native_output(&program, "3.5", source);
    // PHP has an existing, explicit float32 implementation boundary. It must
    // reject this program rather than silently change its numeric semantics.
    let diagnostics = doriac::compile_source(
        "writeback-float32.doria",
        source,
        doriac::backend::BackendTarget::Php,
    )
    .unwrap_err();
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "B1301" && diagnostic.message.contains("float32")),
        "{diagnostics:#?}"
    );
}

#[test]
fn nested_writable_copy_captures_keep_original_homes_coherent() {
    for (body, expected) in [
        (
            r#"let writable $number = 7; let writable $outer = function(): void with (writable $number) { let $increment = function(writable ?int $number): void { $number = ($number ?? 0) + 1; }; $increment($number); }; $outer(); $outer(); echo $number;"#,
            "9",
        ),
        (
            r#"let writable $text = "a" . "b"; let writable $outer = function(): void with (writable $text) { let $append = function(writable ?string $text): void { $text = ($text ?? "") . "c"; }; $append($text); }; $outer(); $outer(); echo $text;"#,
            "abcc",
        ),
        (
            r#"writable ?int $number = 7; let writable $outer = function(): void with (writable $number) { let writable $inner = function(): void with (writable $number) { $number = ($number ?? 0) + 1; }; $inner(); $inner(); }; $outer(); $outer(); echo ($number ?? 0);"#,
            "11",
        ),
        (
            r#"writable ?string $text = "a" . "b"; let writable $outer = function(): void with (writable $text) { let writable $inner = function(): void with (writable $text) { $text = ($text ?? "") . "c"; }; $inner(); $inner(); }; $outer(); $outer(); echo ($text ?? "lost");"#,
            "abcccc",
        ),
    ] {
        assert_execution(&format!("function main(): void {{ {body} }}"), expected);
    }
}

#[test]
fn managed_copy_payload_enums_preserve_repeated_and_nested_writable_updates() {
    let declarations = r#"
enum Message { case Text(string $text); }
function text(?Message $message): string {
    if ($message == null) { return "absent"; }
    return match ($message) { Message::Text($text) => $text };
}
"#;
    for (body, expected) in [
        (
            r#"writable Message $message = Message::Text("a" . "b"); let $append = function(writable ?Message $message): void { $message = Message::Text(text($message) . "c"); }; $append($message); $append($message); $append($message); echo text($message);"#,
            "abccc",
        ),
        (
            r#"writable ?Message $message = Message::Text("a" . "b"); let $replace = function(writable ?Message $message): void { $message = null; $message = Message::Text("c" . "d"); }; $replace($message); $replace($message); echo text($message);"#,
            "cd",
        ),
        (
            r#"writable Message $message = Message::Text("a" . "b"); let writable $outer = function(): void with (writable $message) { let $append = function(writable ?Message $message): void { $message = Message::Text(text($message) . "c"); }; $append($message); }; $outer(); $outer(); echo text($message);"#,
            "abcc",
        ),
        (
            r#"writable ?Message $message = Message::Text("a" . "b"); let writable $outer = function(): void with (writable $message) { let writable $inner = function(): void with (writable $message) { $message = Message::Text(text($message) . "c"); }; $inner(); $inner(); }; $outer(); $outer(); echo text($message);"#,
            "abcccc",
        ),
        (
            r#"writable ?Message $message = Message::Text("a" . "b"); let writable $copy = function(): string with (take $message) { let $clear = function(writable ?Message $value): void { $value = null; }; $clear($message); return text($message); }; echo $copy() . ":" . $copy() . ":" . text($message);"#,
            "absent:absent:ab",
        ),
    ] {
        assert_execution(
            &format!("{declarations} function main(): void {{ {body} }}"),
            expected,
        );
    }
}

#[test]
fn fetched_completion_results_and_wait_values_retain_no_group_loan() {
    use doriac::mir::{CheckedIoOperation, ControlFlowPlan, Statement, Terminator, Type};

    let source = r#"
use Doria\Std\Io\{TransferGroup, TransferWaitResult, ReadTransferKey, Wait, IoError};
function fetch(writable TransferGroup $group, Wait $wait): TransferWaitResult throws IoError {
    return $group->getNextCompletion($wait);
}
function probe(writable TransferGroup $group, ReadTransferKey $key, Wait $wait): TransferWaitResult throws IoError {
    let $copyWait = $wait;
    let $first = fetch($group, $wait);
    let $copy = $first;
    let $second = $group->getNextCompletion($copyWait);
    $group->resubmitRead($key, 0, 0);
    return $copy;
}
function main(): void {}
"#;
    let program = fixture_mir("completion-copy-loans.doria", source);
    validate_fixture(&program, source);
    for name in ["fetch", "probe"] {
        let function = program
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap();
        assert!(function.return_borrow.is_none(), "{name}");
        for plan in function
            .blocks
            .iter()
            .flat_map(|block| &block.statements)
            .filter_map(|statement| match statement {
                Statement::ControlFlowPlan(ControlFlowPlan::RetainedSources(plan)) => Some(plan),
                _ => None,
            })
        {
            assert!(plan.returns.is_empty(), "{name}: {plan:#?}");
        }
    }
    let mut observations = 0;
    for function in &program.functions {
        for block in &function.blocks {
            if let Terminator::CheckedIo {
                operation: CheckedIoOperation::ObserveTransferGroup { wait, outcome, .. },
                result: Some(result),
                ..
            } = &block.terminator
            {
                observations += 1;
                assert!(!function.locals[wait.0].ty.has_move_ownership());
                assert!(!Type::PayloadEnum(*outcome).has_move_ownership());
                assert_eq!(function.locals[result.0].ty, Type::PayloadEnum(*outcome));
            }
        }
    }
    assert!(
        observations > 0,
        "fixture must reach completion observation MIR"
    );
}

fn completion_writeback_fixtures() -> Vec<String> {
    let mut fixtures = Vec::new();
    for (key_type, completion) in [
        ("ReadTransferKey", "TransferCompletion::Read($key, null)"),
        (
            "ReadTransferKey",
            "TransferCompletion::Read($key, ReadOutcome::Data(0))",
        ),
        (
            "WriteTransferKey",
            "TransferCompletion::Write($key, WriteOutcome::Accepted(0))",
        ),
    ] {
        for body in [
            r#"writable TransferWaitResult $result = $group->getNextCompletion($wait); $replace($result); let $copy = $result; $group->getNextCompletion($wait); return $copy;"#,
            r#"let writable $holder = new CompletionHolder(); $replace($holder->value); $group->getNextCompletion($wait); return $holder->value;"#,
            r#"writable List<TransferWaitResult> $items = [$group->getNextCompletion($wait)]; $replace($items[0]); let $copy = $items[0]; $group->getNextCompletion($wait); return $copy;"#,
            r#"writable Dictionary<string, TransferWaitResult> $items = ["result" => $group->getNextCompletion($wait)]; $replace($items["result"]); let $copy = $items["result"]; $group->getNextCompletion($wait); return $copy;"#,
        ] {
            fixtures.push(format!(
                r#"
use Doria\Std\Io\{{TransferGroup, TransferWaitResult, TransferCompletion,
    ReadTransferKey, WriteTransferKey, ReadOutcome, WriteOutcome, Wait, IoError}};
class CompletionHolder {{ writable TransferWaitResult $value = TransferWaitResult::TimedOut; }}
function probe(writable TransferGroup $group, {key_type} $key, Wait $wait): TransferWaitResult throws IoError {{
    let $replace = function(writable ?TransferWaitResult $result): void with ($key) {{
        $result = TransferWaitResult::Completed({completion});
    }};
    {body}
}}
function main(): void {{}}
"#,
            ));
        }
    }
    fixtures
}

#[test]
fn completion_writeback_preserves_nested_inline_copy_homes() {
    for source in completion_writeback_fixtures() {
        let program = fixture_mir("completion-writeback.doria", &source);
        validate_fixture(&program, &source);
    }
    let nullable = r#"
use Doria\Std\Io\{TransferGroup, TransferWaitResult, Wait, IoError};
function probe(writable TransferGroup $group, Wait $wait): ?TransferWaitResult throws IoError {
    writable ?TransferWaitResult $result = $group->getNextCompletion($wait);
    let $clear = function(writable ?TransferWaitResult $value): void { $value = null; };
    $clear($result);
    $group->getNextCompletion($wait);
    return $result;
}
function main(): void {}
"#;
    let program = fixture_mir("nullable-completion-writeback.doria", nullable);
    validate_fixture(&program, nullable);
}

#[test]
fn shared_mir_validation_rejects_null_completion_writeback_into_nonnullable_homes() {
    use doriac::mir::{NullablePayloadEnumExpression, Rvalue, Statement, Terminator, Type};

    for source in completion_writeback_fixtures() {
        let mut program = fixture_mir("forged-completion-writeback.doria", &source);
        validate_fixture(&program, &source);
        let function = program
            .functions
            .iter_mut()
            .find(|function| {
                function.closure.is_some()
                    && function.params.iter().any(|local| {
                        matches!(function.locals[local.0].ty, Type::NullablePayloadEnum(_))
                            && function.is_writable_borrowed_place(*local)
                    })
            })
            .expect("nullable completion replacement closure");
        let (target, ty) = function
            .params
            .iter()
            .find_map(|local| match function.locals[local.0].ty {
                Type::NullablePayloadEnum(ty) if function.is_writable_borrowed_place(*local) => {
                    Some((*local, ty))
                }
                _ => None,
            })
            .unwrap();
        function
            .blocks
            .iter_mut()
            .find(|block| matches!(block.terminator, Terminator::ReturnVoid))
            .expect("normal completion replacement exit")
            .statements
            .push(Statement::AssignLocal {
                target,
                value: Rvalue::NullablePayloadEnum(NullablePayloadEnumExpression::Null(ty)),
            });
        // Forged MIR is independently checked, never interpreted or executed.
        let error = doriac::mir_validation::validate_program(&program).expect_err(&format!(
            "null must not replace a nonnullable completion home:\n{source}"
        ));
        assert!(
            error.message.contains("cannot fit original storage"),
            "{source}\n{error:#?}"
        );
    }
}

#[test]
fn completion_fetch_rejects_live_completed_region_loans_in_source_and_mir() {
    use doriac::mir::{self, CheckedIoOperation, Statement, Terminator};

    let source = r#"
use Doria\Std\Io\{TransferGroup, ReadTransferKey, Wait, IoError};
function probe(writable TransferGroup $group, ReadTransferKey $key, Wait $wait): void throws IoError {
    let $region = $group->getReadRegion($key);
    if ($region != null) {
        echo $region->length;
        let $completion = $group->getNextCompletion($wait);
    }
}
function main(): void {}
"#;
    let mut program = fixture_mir("completion-region-last-use.doria", source);
    validate_fixture(&program, source);
    let conflicting = source.replace(
        "        echo $region->length;\n        let $completion = $group->getNextCompletion($wait);",
        "        let $completion = $group->getNextCompletion($wait);\n        echo $region->length;",
    );
    assert_ne!(conflicting, source);
    assert!(doriac::check_source("completion-region-conflict.doria", &conflicting).is_err());

    let observation = program
        .functions
        .iter()
        .find(|function| {
            function.blocks.iter().any(|block| {
                matches!(
                    block.terminator,
                    Terminator::CheckedIo {
                        operation: CheckedIoOperation::ObserveTransferGroup { .. },
                        ..
                    }
                )
            })
        })
        .expect("completion observation implementation")
        .id;
    let function = program
        .functions
        .iter_mut()
        .find(|function| function.name == "probe")
        .unwrap();
    let (fetch_block, fetch_success) = function
        .blocks
        .iter()
        .find_map(|block| match block.terminator {
            Terminator::CheckedCall {
                function: callee,
                success,
                ..
            } if callee == observation => Some((block.id, success)),
            _ => None,
        })
        .expect("actual completion-fetch call in the guarded region branch");
    let (echo_block, echo_success, read_region) = function
        .blocks
        .iter()
        .find_map(|block| match &block.terminator {
            Terminator::CheckedIo {
                operation:
                    CheckedIoOperation::WriteStream {
                        contents:
                            mir::IoContents::String(mir::StringExpression::Display(
                                mir::ValueExpression::Integer(mir::IntegerExpression::Use {
                                    operand: mir::Operand::ByteRegionLength(region),
                                    ..
                                }),
                            )),
                        stderr: false,
                    },
                success,
                ..
            } => Some((block.id, *success, *region)),
            _ => None,
        })
        .expect("checked echo reads the completed-region length before the fetch");
    assert_eq!(echo_success, fetch_block, "echo must precede the fetch");
    let echo_setup = std::mem::take(&mut function.blocks[echo_block.0].statements);
    let mut fetch_setup = std::mem::take(&mut function.blocks[fetch_block.0].statements);
    let echo_cleanup_end = fetch_setup
        .iter()
        .position(|statement| {
            matches!(statement, Statement::EndBorrow { local } if *local == read_region)
        })
        .expect("the checked echo retires its narrowed region descriptor")
        + 1;
    // Echo's success first retires its statement-scoped descriptor, then sets
    // up the next call. Keep every setup and cleanup statement, but move the
    // descriptor projection with echo and retire it only after echo succeeds.
    let echo_cleanup: Vec<_> = fetch_setup.drain(..echo_cleanup_end).collect();
    function.blocks[echo_block.0].statements = fetch_setup;
    function.blocks[fetch_block.0].statements = echo_setup;
    function.blocks[fetch_success.0]
        .statements
        .splice(0..0, echo_cleanup);
    let mut fetch = function.blocks[fetch_block.0].terminator.clone();
    let mut echo = function.blocks[echo_block.0].terminator.clone();
    let Terminator::CheckedCall { success, .. } = &mut fetch else {
        unreachable!();
    };
    *success = echo_success;
    let Terminator::CheckedIo { success, .. } = &mut echo else {
        unreachable!();
    };
    *success = fetch_success;
    function.blocks[echo_block.0].terminator = fetch;
    function.blocks[fetch_block.0].terminator = echo;
    // Both failure routes retain their original cleanup: the fetch can fail
    // before echo forms its descriptor, and echo can fail after forming it.
    // The null guard still dominates the region-length read after the fetch.
    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("completion fetching must not mutate a live completed-region owner");
    assert!(error.message.contains("live"), "{error:#?}");
}

#[test]
fn writable_static_copy_slots_preserve_repeated_updates() {
    assert_execution(
        r#"
class Store {
    static writable int $number = 7;
    static writable string $text = "ab";
}
function main(): void {
    let $increment = function(writable ?int $number): void { $number = ($number ?? 0) + 1; };
    let $append = function(writable ?string $text): void { $text = ($text ?? "") . "c"; };
    $increment(Store::number);
    $increment(Store::number);
    $append(Store::text);
    $append(Store::text);
    echo Store::number . ":" . Store::text;
}
"#,
        "9:abcc",
    );
    assert_execution(
        r#"
class Store {
    static writable ?int $number = 7;
    static writable ?string $text = "ab";
}
function main(): void {
    let $clearNumber = function(writable ?int $number): void { $number = null; };
    let $clearText = function(writable ?string $text): void { $text = null; };
    if (Store::number != null) { $clearNumber(Store::number); }
    if (Store::text != null) { $clearText(Store::text); }
    echo (Store::number ?? 9) . ":" . (Store::text ?? "cleared");
}
"#,
        "9:cleared",
    );
}

#[test]
fn static_writable_support_preserves_readonly_and_exclusive_access_rules() {
    for (source, code) in [
        (
            r#"class Store { static int $number = 7; } function main(): void { let $run = function(writable int $number): void { $number++; }; $run(Store::number); }"#,
            "E0651",
        ),
        (
            r#"class Store { static string $text = "ab"; } function main(): void { let $run = function(writable string $text): void { $text = $text . "c"; }; $run(Store::text); }"#,
            "E0651",
        ),
        (
            r#"class Store { static writable int $number = 7; } function main(): void { let $run = function(writable int $left, writable int $right): void { $left++; $right++; }; $run(Store::number, Store::number); }"#,
            "E0477",
        ),
    ] {
        let (_, analysis) =
            doriac::analyze_source_for_ide("static-writeback-access.doria", source).unwrap();
        assert!(
            analysis
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == code),
            "{source}\n{:#?}",
            analysis.diagnostics
        );
        assert!(doriac::lower_source_to_mir("static-writeback-access.doria", source).is_err());
    }
}

#[test]
fn writable_getter_receivers_do_not_upgrade_readonly_borrow_results() {
    let source = format!("{DECLARATIONS} function main(): void {{ let writable $owner = new Holder(); let writable $view = $owner->current; }}");
    let (_, analysis) =
        doriac::analyze_source_for_ide("readonly-getter-result.doria", &source).unwrap();
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0478"),
        "{:#?}",
        analysis.diagnostics
    );
    assert!(doriac::lower_source_to_mir("readonly-getter-result.doria", &source).is_err());
}

#[test]
fn distinct_owner_callback_fields_do_not_contaminate_each_other() {
    let source = format!(
        r#"{DECLARATIONS}
class Callbacks {{
    writable function(writable ?Counter): void $run = function(writable ?Counter $counter): void {{ $counter?->bump(); }};
}}
function main(): void {{
    let writable $first = new Callbacks();
    let writable $second = new Callbacks();
    $first->run = function(writable ?Counter $counter): void {{ $counter?->bump(); }};
    $second->run = function(writable ?Counter $counter): void {{ $counter = null; }};
    let writable $owner = new Counter();
    let writable $view = lend($owner);
    $first->run($view);
    echo ($view?->value ?? 0);
}}
"#
    );
    assert_execution(&source, "8");
}

#[test]
fn narrowed_mixed_class_payloads_preserve_in_place_writable_updates() {
    let source = format!(
        r#"{DECLARATIONS}
function main(): void {{
    writable mixed $owner = new Counter();
    if ($owner is Counter) {{
        let $run = function(writable ?Counter $counter): void {{ $counter?->bump(); }};
        $run($owner);
    }}
    if ($owner is Counter) {{ echo $owner->value; }} else {{ echo "lost"; }}
}}
"#
    );
    assert_execution(&source, "8");
}

#[test]
fn narrowed_boxed_payloads_preserve_compatible_replacement_and_cleanup() {
    let observed = r#"
class Observed implements Counted {
    writable int $value = 7;
    function __construct(string $label) {}
    writable function bump(): void { $this->value++; }
    function getValue(): int { return $this->value; }
    function __destruct() { try { echo "drop{$this->label};"; } catch (Error) {} }
}
"#;
    for (body, expected) in [
        (
            r#"writable mixed $owner = new Counter();
if ($owner is Counter) {
    let $run = function(writable ?Counter $counter): void {
        $counter = new Counter();
        $counter->bump();
    };
    $run($owner);
}
if ($owner is Counter) { echo $owner->value; } else { echo "lost"; }"#,
            "8",
        ),
        (
            r#"writable Counted $view = new Counter();
writable mixed $owner = $view;
if ($owner is Counted) {
    let $run = function(writable ?Counted $counter): void {
        $counter = new Counter();
        $counter->bump();
    };
    $run($owner);
}
if ($owner is Counted) { echo $owner->getValue(); } else { echo "lost"; }"#,
            "8",
        ),
        (
            r#"writable mixed $owner = new Observed("old");
if ($owner is Observed) {
    let $run = function(writable ?Observed $counter): void {
        $counter = new Observed("new");
        $counter->bump();
    };
    $run($owner);
}
if ($owner is Observed) { echo $owner->value; } else { echo "lost"; }"#,
            "dropold;8dropnew;",
        ),
        (
            r#"writable Counted $view = new Observed("old");
writable mixed $owner = $view;
if ($owner is Counted) {
    let $run = function(writable ?Counted $counter): void {
        $counter = new Counter();
        $counter->bump();
    };
    $run($owner);
}
if ($owner is Counted) { echo $owner->getValue(); } else { echo "lost"; }"#,
            "dropold;8",
        ),
    ] {
        let source = format!("{DECLARATIONS}{observed} function main(): void {{ {body} }}");
        assert_execution(&source, expected);
    }
}

#[test]
fn literal_collection_selection_keeps_distinct_callback_writeback_targets() {
    for (collection, invocation) in [
        (
            "List<function(writable ?Counter): void> $callbacks = [$bump, $clear];",
            "$callbacks[0]($owner);",
        ),
        (
            r#"Dictionary<string, function(writable ?Counter): void> $callbacks = ["bump" => $bump, "clear" => $clear];"#,
            r#"$callbacks["bump"]($owner);"#,
        ),
    ] {
        let source = format!(
            r#"{DECLARATIONS}
function main(): void {{
    let $bump = function(writable ?Counter $counter): void {{ $counter?->bump(); }};
    let $clear = function(writable ?Counter $counter): void {{ $counter = null; }};
    {collection}
    let writable $owner = new Counter();
    {invocation}
    {invocation}
    echo $owner->value;
}}
"#
        );
        assert_execution(&source, "9");
    }
}

#[test]
fn boxed_base_typed_child_replacements_update_dynamic_dispatch_and_cleanup() {
    let declarations = r#"
open class Counter {
    writable int $value = 7;
    function __construct(string $label) {}
    writable function bump(): void { $this->value++; }
    open function getKind(): string { return "base"; }
    function __destruct() { try { echo "base-drop:{$this->label};"; } catch (Error) {} }
}
class Child extends Counter {
    function __construct(string $childLabel) { parent::__construct($childLabel); }
    override function getKind(): string { return "child"; }
    function __destruct() { try { echo "child-drop:{$this->label};"; } catch (Error) {} }
}
class OtherChild extends Counter {
    function __construct(string $otherLabel) { parent::__construct($otherLabel); }
    override function getKind(): string { return "other"; }
    function __destruct() { try { echo "other-drop:{$this->label};"; } catch (Error) {} }
}
"#;
    for (replacement, expected) in [
        (
            r#"new Counter("replacement")"#,
            "child7;child-drop:initial;base-drop:initial;base8;base-drop:replacement;base8;base-drop:replacement;",
        ),
        (
            r#"new OtherChild("replacement")"#,
            "child7;child-drop:initial;base-drop:initial;other8;other-drop:replacement;base-drop:replacement;other8;other-drop:replacement;base-drop:replacement;",
        ),
    ] {
        let source = format!(
            r#"{declarations}
function main(): void {{
    Counter $base = new Child("initial");
    writable mixed $owner = $base;
    if ($owner is Counter) {{
        echo "{{$owner->getKind()}}{{$owner->value}};";
        let $run = function(writable ?Counter $counter): void {{
            $counter = {replacement};
            $counter->bump();
        }};
        $run($owner);
        if ($owner is Counter) {{
            echo "{{$owner->getKind()}}{{$owner->value}};";
            $run($owner);
            if ($owner is Counter) {{
                echo "{{$owner->getKind()}}{{$owner->value}};";
            }} else {{ echo "lost"; }}
        }} else {{ echo "lost"; }}
    }} else {{ echo "lost"; }}
}}
"#
        );
        assert_execution(&source, expected);
    }
}

#[test]
fn replacing_a_borrowed_function_does_not_acquire_ownership() {
    let source = r#"
function consume(take function(): int $callback): void {}
function main(): void {
    let writable $callback = fn() => 7;
    let $replace = function(writable function(): int $borrow): void {
        $borrow = fn() => 9;
        consume($borrow);
    };
    $replace($callback);
}
"#;
    let (_, analysis) =
        doriac::analyze_source_for_ide("borrowed-function-writeback.doria", source).unwrap();
    assert!(
        analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "E0474"),
        "{:#?}",
        analysis.diagnostics
    );
    assert!(doriac::lower_source_to_mir("borrowed-function-writeback.doria", source).is_err());
}

#[test]
fn region_descriptor_replacements_keep_carrier_types_and_source_loans() {
    for replacement in ["$view->getRegion(1, 1)", "narrow($view)"] {
        for body in [
            r#"let writable $view = $bytes->getRegion(0, 2); $run($view); echo "{$view->length}:{$view[0]};";"#,
            r#"writable List<ByteRegion> $items = [$bytes->getRegion(0, 2)]; $run($items[0]); echo "{$items[0]->length}:{$items[0][0]};";"#,
            r#"writable Dictionary<string, ByteRegion> $items = ["view" => $bytes->getRegion(0, 2)]; $run($items["view"]); echo "{$items["view"]->length}:{$items["view"][0]};";"#,
        ] {
            let source = format!(
                r#"
use Doria\Std\Io\ByteRegion;
function narrow(ByteRegion $view): ByteRegion {{ return $view->getRegion(1, 1); }}
function main(): void {{
    writable Bytes $bytes = Bytes::allocate(2);
    $bytes[0] = 7;
    $bytes[1] = 9;
    {{
        let $run = function(writable ?ByteRegion $view): void {{
            if ($view != null) {{ $view = {replacement}; }}
        }};
        {body}
    }}
}}
"#,
            );
            assert_execution(&source, "1:9;");
        }
    }
}

fn writable_region_collection_writeback_fixtures() -> Vec<String> {
    [
        r#"writable List<WritableByteRegion> $items = [$bytes->getWritableRegion(0, 1)]; $run($items[0]); echo $items[0][0];"#,
        r#"writable List<WritableByteRegion> $items = [$bytes->getWritableRegion(0, 1)]; let writable $view = $items->first; $run($view); echo $items[0][0];"#,
        r#"writable Dictionary<string, WritableByteRegion> $items = ["view" => $bytes->getWritableRegion(0, 1)]; $run($items["view"]); echo $items["view"][0];"#,
        r#"writable Dictionary<string, WritableByteRegion> $items = ["view" => $bytes->getWritableRegion(0, 1)]; let writable $view = $items->get("view"); $run($view); echo $items["view"][0];"#,
    ]
    .into_iter()
    .map(|body| {
        format!(
            r#"
use Doria\Std\Io\WritableByteRegion;
function main(): void {{
    writable Bytes $bytes = Bytes::allocate(1);
    {{
        let $run = function(writable ?WritableByteRegion $view): void {{
            if ($view != null) {{ $view[0] = 9; }}
        }};
        {body}
    }}
}}
"#,
        )
    })
    .collect()
}

#[test]
fn nullable_region_collection_views_preserve_nonnullable_descriptor_homes() {
    for source in writable_region_collection_writeback_fixtures() {
        assert_execution(&source, "9");
    }
}

#[test]
fn nullable_region_collection_slots_allow_absent_descriptor_replacements() {
    for region in ["ByteRegion", "WritableByteRegion"] {
        let formation = if region == "ByteRegion" {
            "$bytes->getRegion(0, 1)"
        } else {
            "$bytes->getWritableRegion(0, 1)"
        };
        let source = format!(
            r#"
use Doria\Std\Io\{region};
function main(): void {{
    writable Bytes $bytes = Bytes::allocate(1);
    {{
        writable List<?{region}> $items = [{formation}];
        let $run = function(writable ?{region} $view): void {{ $view = null; }};
        $run($items[0]);
        if ($items[0] == null) {{ echo "cleared;"; }}
        else {{ echo "present;"; }}
    }}
    $bytes[0] = 9;
    echo $bytes[0];
}}
"#,
        );
        assert_execution(&source, "cleared;9");
    }
}

#[test]
fn shared_mir_validation_rejects_null_written_through_nonnullable_region_collection_views() {
    use doriac::mir::{
        ByteRegionKind, NullableByteRegionExpression, Rvalue, Statement, Terminator, Type,
    };

    for source in writable_region_collection_writeback_fixtures() {
        let mut program = fixture_mir("forged-region-collection-writeback.doria", &source);
        validate_fixture(&program, &source);
        let function = program
            .functions
            .iter_mut()
            .find(|function| {
                function.closure.is_some()
                    && function.locals.iter().any(|local| {
                        local.ty == Type::NullableByteRegion(ByteRegionKind::Writable)
                            && function.is_writable_borrowed_place(local.id)
                    })
            })
            .expect("writable nullable region closure input");
        let target = function
            .locals
            .iter()
            .find(|local| {
                local.ty == Type::NullableByteRegion(ByteRegionKind::Writable)
                    && function.is_writable_borrowed_place(local.id)
            })
            .unwrap()
            .id;
        function
            .blocks
            .iter_mut()
            .find(|block| matches!(block.terminator, Terminator::ReturnVoid))
            .expect("normal region closure exit")
            .statements
            .push(Statement::AssignLocal {
                target,
                value: Rvalue::NullableByteRegion(NullableByteRegionExpression::Null {
                    kind: ByteRegionKind::Writable,
                }),
            });
        let error = doriac::mir_validation::validate_program(&program).expect_err(&format!(
            "forged null must not replace the nonnullable region collection slot:\n{source}"
        ));
        assert!(
            error.message.contains("cannot fit original storage"),
            "{source}\n{error:#?}"
        );
    }
}

#[test]
fn shared_mir_validation_rejects_forged_widened_writes() {
    use doriac::mir::{NullableClassExpression, Rvalue, Statement, Terminator, Type};

    for body in [
        "writable ?Counter $left = null; let writable $right = new Counter(); let writable $view = lendNullable($left) ?? lend($right); let $run = function(writable ?Counter $counter): void { $counter?->bump(); }; $run($view);",
        "let writable $left = new Counter(); writable ?Counter $right = null; let writable $view = lend($left) ?? lendNullable($right); let $run = function(writable ?Counter $counter): void { $counter?->bump(); }; $run($view);",
        "let writable $owner = new Counter(); let writable $view = lend($owner); let $run = function(writable ?Counter $counter): void { $counter?->bump(); }; $run($view);",
        "let writable $owner = new Counter(); let $run = function(writable ?Counter $counter): void { $counter?->bump(); }; $run($owner);",
        "let writable $owner = new Counter(); let writable $view = lend($owner); let $run = function(writable ?Counter $counter): void { $counter?->bump(); }; forward($run, $view);",
        "let writable $owner = new Counter(); let writable $view = lend($owner); let $run = function(writable ?Counter $counter): void { $counter?->bump(); }; recursive($run, $view, 2);",
        "let writable $owner = new Counter(); let writable $view = lend($owner); let writable $run = function(): void with (writable $view) { $view?->bump(); }; $run();",
        "let writable $owner = new Holder(); let $run = function(writable ?Counter $counter): void { $counter?->bump(); }; $run($owner->stored);",
        "let writable $owner = new Holder(); let writable $view = lend($owner->stored); let $run = function(writable ?Counter $counter): void { $counter?->bump(); }; $run($view);",
        "let writable $owners = [new Counter()]; let $run = function(writable ?Counter $counter): void { $counter?->bump(); }; $run($owners[0]);",
        "let writable $owners = [new Counter()]; let writable $view = lend($owners[0]); let $run = function(writable ?Counter $counter): void { $counter?->bump(); }; $run($view);",
        "writable Dictionary<string, Counter> $owners = [\"key\" => new Counter()]; let writable $view = lend($owners[\"key\"]); let $run = function(writable ?Counter $counter): void { $counter?->bump(); }; $run($view);",
        "let writable $owner = new Counter(); let $run = function(writable ?Counter $counter): void { $counter?->bump(); }; generic($run, $owner);",
        "let writable $owner = new Counter(); let writable $view = lend($owner); let $run = function(writable ?Counter $counter): void { $counter?->bump(); }; let $delegate = function(function(writable ?Counter): void $callback, writable ?Counter $counter): void { $callback($counter); }; $delegate($run, $view);",
        "let writable $owner = new Counter(); let writable $view = lend($owner); let $run = function(writable ?Counter $counter): void { $counter?->bump(); }; let $delegate = function(writable ?Counter $counter): void with ($run) { $run($counter); }; forward($delegate, $view);",
        "let writable $owner = new Counter(); let $alias = function(writable Counter $counter): ?Counter { return $counter; }; let writable $view = $alias($owner); let $run = function(writable ?Counter $counter): void { $counter?->bump(); }; $run($view);",
        "let writable $owner = new Counter(); let writable $view = lend($owner); let writable $outer = function(): void with (writable $view) { let writable $run = function(): void with (writable $view) { $view?->bump(); }; $run(); }; $outer();",
        "writable mixed $owner = new Counter(); if ($owner is Counter) { let $run = function(writable ?Counter $counter): void { $counter?->bump(); }; $run($owner); }",
    ] {
        let source = format!("{DECLARATIONS} function main(): void {{ {body} }}");
        let mut program = fixture_mir("forged-writeback.doria", &source);
        validate_fixture(&program, &source);
        let function = program.functions.iter_mut().find(|function| {
            function.closure.is_some()
                && function.locals.iter().any(|local| {
                    matches!(local.ty, Type::NullableClass(_))
                        && function.is_writable_borrowed_place(local.id)
                })
        }).expect("writable nullable closure input");
        let local = function.locals.iter().find(|local| {
            matches!(local.ty, Type::NullableClass(_))
                && function.is_writable_borrowed_place(local.id)
        }).unwrap();
        let Type::NullableClass(class) = local.ty else { unreachable!() };
        let assignment = Statement::AssignLocal {
            target: local.id,
            value: Rvalue::NullableClass(NullableClassExpression::Null(class)),
        };
        let block = function.blocks.iter_mut().find(|block| {
            matches!(block.terminator, Terminator::ReturnVoid)
        }).expect("normal closure exit");
        block.statements.push(assignment);
        let error = doriac::mir_validation::validate_program(&program)
            .expect_err(&format!("MIR must reject null written through a view of non-nullable storage:\n{source}"));
        assert!(error.message.contains("storage"), "{body}\n{error:#?}");
    }
}

#[test]
fn shared_mir_validation_rejects_forged_copy_writes_into_nonnullable_homes() {
    use doriac::mir::{
        NullableScalarExpression, NullableStringExpression, Rvalue, Statement, Terminator, Type,
    };

    for source in [
        r#"function main(): void { let writable $number = 7; let $run = function(writable ?int $number): void { $number = ($number ?? 0) + 1; }; $run($number); }"#,
        r#"class Store { static writable int $number = 7; } function main(): void { let $run = function(writable ?int $number): void { $number = ($number ?? 0) + 1; }; $run(Store::number); }"#,
        r#"class Store { writable int $number = 7; } function main(): void { let writable $store = new Store(); let $run = function(writable ?int $number): void { $number = ($number ?? 0) + 1; }; $run($store->number); }"#,
        r#"function main(): void { let writable $text = "a" . "b"; let $run = function(writable ?string $text): void { $text = ($text ?? "") . "c"; }; $run($text); }"#,
        r#"class Store { static writable string $text = "ab"; } function main(): void { let $run = function(writable ?string $text): void { $text = ($text ?? "") . "c"; }; $run(Store::text); }"#,
        r#"class Store { writable string $text = "ab"; } function main(): void { let writable $store = new Store(); let $run = function(writable ?string $text): void { $text = ($text ?? "") . "c"; }; $run($store->text); }"#,
        r#"function main(): void { let writable $number = 7; let writable $outer = function(): void with (writable $number) { let $run = function(writable ?int $number): void { $number = ($number ?? 0) + 1; }; $run($number); }; $outer(); }"#,
    ] {
        let mut program = fixture_mir("forged-copy-writeback.doria", source);
        validate_fixture(&program, source);
        let function = program
            .functions
            .iter_mut()
            .find(|function| {
                function.closure.is_some()
                    && function.locals.iter().any(|local| {
                        matches!(local.ty, Type::NullableScalar(_) | Type::NullableString)
                            && function.is_writable_borrowed_place(local.id)
                    })
            })
            .expect("writable nullable Copy closure input");
        let local = function
            .locals
            .iter()
            .find(|local| {
                matches!(local.ty, Type::NullableScalar(_) | Type::NullableString)
                    && function.is_writable_borrowed_place(local.id)
            })
            .unwrap();
        let value = match local.ty {
            Type::NullableScalar(ty) => Rvalue::NullableScalar(NullableScalarExpression::Null(ty)),
            Type::NullableString => Rvalue::NullableString(NullableStringExpression::Null),
            _ => unreachable!(),
        };
        let assignment = Statement::AssignLocal {
            target: local.id,
            value,
        };
        function
            .blocks
            .iter_mut()
            .find(|block| matches!(block.terminator, Terminator::ReturnVoid))
            .expect("normal closure exit")
            .statements
            .push(assignment);
        let error = doriac::mir_validation::validate_program(&program).expect_err(&format!(
            "forged null must not replace nonnullable Copy storage:\n{source}"
        ));
        assert!(error.message.contains("storage"), "{source}\n{error:#?}");
    }
}

#[test]
fn shared_mir_validation_rejects_forged_covariant_replacements() {
    use doriac::mir::{
        ClassExpression, InterfaceExpression, InterfaceValue, ReturnType, Rvalue, Statement,
        Terminator, Type,
    };

    for (declaration, body) in [
        ("function replacement(): Counter { return new Counter(); }",
         "let writable $owner = new Child(); let writable $view = lendBase($owner); let $run = function(writable Counter $counter): void { $counter->bump(); }; $run($view);"),
        ("function replacement(): Counted { return new Other(); }",
         "let writable $owner = new Counter(); let writable $view = lendInterface($owner); let $run = function(writable Counted $counter): void { $counter->bump(); }; $run($view);"),
    ] {
        let source = format!("{DECLARATIONS} {declaration} function main(): void {{ {body} }}");
        let mut program = fixture_mir("forged-covariant-writeback.doria", &source);
        validate_fixture(&program, &source);
        let replacement = program.functions.iter().find(|function| function.name == "replacement")
            .expect("owned replacement factory");
        let value = match replacement.return_type {
            ReturnType::Value(Type::Class(class)) => Rvalue::Class(ClassExpression::Call {
                class, function: replacement.id, args: Vec::new(), return_borrow: None, span: replacement.source_span,
            }),
            ReturnType::Value(Type::Interface(interface)) => Rvalue::Interface(InterfaceExpression {
                interface, value: InterfaceValue::Call {
                    function: replacement.id, args: Vec::new(), return_borrow: None, span: replacement.source_span,
                },
            }),
            _ => unreachable!(),
        };
        let function = program.functions.iter_mut().find(|function| {
            function.closure.is_some() && function.locals.iter().any(|local| {
                matches!(local.ty, Type::Class(_) | Type::Interface(_))
                    && function.is_writable_borrowed_place(local.id)
            })
        }).expect("widened nominal closure input");
        let target = function.locals.iter().find(|local| {
            matches!(local.ty, Type::Class(_) | Type::Interface(_))
                && function.is_writable_borrowed_place(local.id)
        }).unwrap().id;
        function.blocks.iter_mut().find(|block| matches!(block.terminator, Terminator::ReturnVoid))
            .expect("normal closure exit").statements.push(Statement::AssignLocal { target, value });
        let error = doriac::mir_validation::validate_program(&program)
            .expect_err(&format!("forged replacement must fit the original concrete home, not only the widened view:\n{source}"));
        assert!(error.message.contains("storage"), "{body}\n{error:#?}");
    }
}

#[test]
fn shared_mir_validation_rejects_unknown_callback_replacement_domains() {
    use doriac::mir::{FunctionExpression, IndirectCallee, Terminator, Type};

    let source = format!(
        r#"{DECLARATIONS}
function unknown(function(writable ?Counter): void $callback, writable Counter $owner): void {{
    let $safe = function(writable ?Counter $counter): void {{ $counter?->bump(); }};
    $safe($owner);
}}
function main(): void {{}}
"#
    );
    let mut program = fixture_mir("forged-unknown-writeback.doria", &source);
    validate_fixture(&program, &source);
    let function = program
        .functions
        .iter_mut()
        .find(|function| function.name == "unknown")
        .unwrap();
    let callback = function.params[0];
    let Type::Function(function_type) = function.locals[callback.0].ty else {
        unreachable!()
    };
    let mut forged = false;
    for block in &mut function.blocks {
        if let Terminator::IndirectCall {
            callee,
            function_type: call_type,
            ..
        }
        | Terminator::CheckedIndirectCall {
            callee,
            function_type: call_type,
            ..
        } = &mut block.terminator
        {
            assert_eq!(
                *call_type, function_type,
                "forge preserves the actual lowered invocation type"
            );
            *callee = IndirectCallee::Closure(FunctionExpression::Local {
                function_type,
                local: callback,
                transfer: false,
            });
            forged = true;
        }
    }
    assert!(forged, "fixture must contain the actual closure invocation");
    let error = doriac::mir_validation::validate_program(&program).expect_err(&format!(
        "an unknown writable callback may replace with any value in its declared domain:\n{source}",
    ));
    assert!(error.message.contains("storage"), "{error:#?}");
}

#[test]
fn unknown_callback_replacements_reach_subsequent_copy_captures() {
    use doriac::mir::{
        FunctionExpression, IndirectCallee, NullableScalarExpression, Rvalue, Statement,
        Terminator, Type,
    };

    let source = r#"
function probe(function(writable ?int): void $callback, writable int $owner): void {
    writable ?int $maybe = 7;
    let $safe = function(writable ?int $number): void {};
    $safe($maybe);
    let $replace = function(writable ?int $value): void with ($maybe) {
        if ($maybe == null) { $value = 8; } else { $value = 8; }
    };
    $replace($owner);
}
function main(): void {}
"#;
    let mut program = fixture_mir("forged-unknown-copy-effects.doria", source);
    validate_fixture(&program, source);
    let function = program
        .functions
        .iter_mut()
        .find(|function| function.name == "probe")
        .unwrap();
    let callback = function.params[0];
    let Type::Function(function_type) = function.locals[callback.0].ty else {
        unreachable!()
    };
    let invocation = function
        .blocks
        .iter_mut()
        .find_map(|block| match &mut block.terminator {
            Terminator::IndirectCall {
                callee,
                function_type: actual,
                ..
            }
            | Terminator::CheckedIndirectCall {
                callee,
                function_type: actual,
                ..
            } => Some((callee, actual)),
            _ => None,
        })
        .expect("actual first callback invocation");
    assert_eq!(*invocation.1, function_type);
    *invocation.0 = IndirectCallee::Closure(FunctionExpression::Local {
        function_type,
        local: callback,
        transfer: false,
    });

    let function = program
        .functions
        .iter_mut()
        .find(|function| {
            function
                .closure
                .as_ref()
                .is_some_and(|closure| !closure.capture_locals.is_empty())
                && function
                    .locals
                    .iter()
                    .any(|local| function.is_writable_borrowed_place(local.id))
        })
        .expect("replacement closure with its explicit Copy capture");
    let target = function
        .params
        .iter()
        .copied()
        .find(|local| function.is_writable_borrowed_place(*local))
        .unwrap();
    let captured = function.closure.as_ref().unwrap().capture_locals[0].1;
    let Type::NullableScalar(ty) = function.locals[captured.0].ty else {
        unreachable!()
    };
    let assignment = Statement::AssignLocal {
        target,
        value: Rvalue::NullableScalar(NullableScalarExpression::Local {
            ty,
            local: captured,
        }),
    };
    function
        .blocks
        .iter_mut()
        .find(|block| matches!(block.terminator, Terminator::ReturnVoid))
        .expect("normal replacement closure exit")
        .statements
        .push(assignment);
    let error = doriac::mir_validation::validate_program(&program)
        .expect_err(&format!("unknown callback may clear nullable storage before it is copied into the later environment:\n{source}"));
    assert!(error.message.contains("storage"), "{error:#?}");
}
