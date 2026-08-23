use doriac::backend::BackendTarget;

fn debug(source: &str) -> String {
    doriac::compile_source_to_debug("stage30d.doria", source)
        .unwrap_or_else(|diagnostics| panic!("Stage 30d source should execute: {diagnostics:#?}"))
}

fn target_error(source: &str, target: BackendTarget) -> doriac::diagnostics::Diagnostic {
    doriac::compile_source("stage30d.doria", source, target)
        .expect_err("executable closure route should stop at the target boundary")
        .into_iter()
        .next()
        .expect("target boundary should produce one diagnostic")
}

#[test]
fn debug_executes_no_capture_arrow_and_block_closures() {
    let source = r#"
function main(): void
{
    let $double = fn(int $value) => $value * 2;
    let $increment = function (int $value): int {
        return $value + 1;
    };

    echo "{$double(20)} {$increment(1)}\n";
}
"#;

    assert_eq!(debug(source), "exit_status: 0\nstdout: 40 2\n\n");
}

#[test]
fn debug_preserves_readonly_and_writable_capture_places() {
    let source = r#"
function main(): void
{
    let $base = 40;
    let writable $count = 0;
    let writable $next = function (): int with ($base, writable $count) {
        $count += 1;
        return $base + $count;
    };

    echo "{$next()} {$next()}\n";
}
"#;

    assert_eq!(debug(source), "exit_status: 0\nstdout: 41 42\n\n");
}

#[test]
fn debug_moves_owned_captures_into_once_closures() {
    let source = r#"
function main(): void
{
    let $message = "done";
    let $consume = function (): string with (take $message) {
        return $message;
    };

    echo $consume() . "\n";
}
"#;

    assert_eq!(debug(source), "exit_status: 0\nstdout: done\n\n");
}

#[test]
fn debug_executes_nested_factory_closures() {
    let source = r#"
function main(): void
{
    let $base = 10;
    let $factory = fn(int $left) with (take $base) =>
        fn(int $right) with (take $left, take $base) => $left + $right + $base;
    let $add = $factory(20);

    echo "{$add(12)}\n";
}
"#;

    assert_eq!(debug(source), "exit_status: 0\nstdout: 42\n\n");
}

#[test]
fn debug_executes_narrowed_nullable_function_values() {
    let source = r#"
function main(): void
{
    writable ?function(): int $callback = null;
    $callback = fn() => 42;

    if ($callback != null) {
        echo "{$callback()}\n";
    }

    $callback = null;
}
"#;

    assert_eq!(debug(source), "exit_status: 0\nstdout: 42\n\n");
}

#[test]
fn debug_preserves_writable_nullable_places_for_indirect_calls() {
    let source = r#"
class Box {}

function main(): void
{
    let $accept = function (writable ?Box $value): void {};
    writable ?Box $value = null;
    $accept($value);
    echo "ok\n";
}
"#;

    assert_eq!(debug(source), "exit_status: 0\nstdout: ok\n\n");
}

#[test]
fn debug_executes_function_parameters_returns_and_properties() {
    let source = r#"
function apply(function(int): int $callback, int $value): int
{
    return $callback($value);
}

function makeAdder(int $base): function(int): int
{
    return fn(int $value) with (take $base) => $base + $value;
}

class Runner
{
    writable function(int): int $callback = fn(int $value) => $value;

    writable function replace(take function(int): int $callback): void
    {
        $this->callback = $callback;
    }

    function run(int $value): int
    {
        return $this->callback($value);
    }
}

function main(): void
{
    let $add = makeAdder(40);
    let writable $runner = new Runner();
    echo "{apply($add, 1)} {$runner->run(2)} ";
    $runner->replace(fn(int $value) => $value * 2);
    echo "{$runner->run(21)}\n";
}
"#;

    assert_eq!(debug(source), "exit_status: 0\nstdout: 41 2 42\n\n");
}

#[test]
fn debug_executes_function_values_in_collections_and_payload_enums() {
    let source = r#"
enum Work
{
    case Run(function(): int $callback);
}

function execute(take Work $work): int
{
    return match (take $work) {
        Work::Run($callback) => $callback()
    };
}

function main(): void
{
    List<function(): int> $callbacks = [fn() => 20, fn() => 22];
    Dictionary<string, function(): int> $named = ["answer" => fn() => 42];
    let $work = Work::Run(fn() => 42);

    echo "{$callbacks[0]()} {$callbacks[1]()} {$named["answer"]()} {execute($work)}\n";
}
"#;

    assert_eq!(debug(source), "exit_status: 0\nstdout: 20 22 42 42\n\n");
}

#[test]
fn debug_propagates_and_catches_checked_closure_effects() {
    let source = r#"
class Failure implements Error
{
    function __construct(string $message)
    {
    }
}

function main(): void
{
    let $fail = function (): int {
        throw new Failure("closure failed");
    };

    try {
        $fail();
    } catch (Failure $error) {
        echo "caught\n";
    }
}
"#;

    assert_eq!(debug(source), "exit_status: 0\nstdout: caught\n\n");
}

#[test]
fn debug_executes_this_capture_from_a_returned_closure() {
    let source = r#"
class Box
{
    function __construct(int $value)
    {
    }

    function reader(): function(): int
    {
        return fn() with ($this) => $this->value;
    }
}

function main(): void
{
    let $box = new Box(42);
    let $read = $box->reader();
    echo "{$read()}\n";
}
"#;

    assert_eq!(debug(source), "exit_status: 0\nstdout: 42\n\n");
}

#[test]
fn returned_borrow_bound_closure_keeps_the_callers_parameter_root() {
    let source = r#"
class Box
{
    function __construct(writable int $value)
    {
    }
}

function reader(Box $box): function(): int
{
    return fn() with ($box) => $box->value;
}

function main(): void
{
    let $box = new Box(42);
    let $read = reader($box);
    echo "{$read()}\n";
}
"#;

    assert_eq!(debug(source), "exit_status: 0\nstdout: 42\n\n");
}

#[test]
fn executable_closures_stop_only_at_native_and_php_boundaries() {
    let source = "function main(): void { let $callback = fn() => 1; echo \"{$callback()}\\n\"; }";

    let native = target_error(source, BackendTarget::Native);
    assert_eq!(native.code, "E0641");
    assert_eq!(
        native.title,
        "Closure Native Execution Is Not Yet Available"
    );
    assert!(native.message.contains("Stage 30e"));

    let php = target_error(source, BackendTarget::Php);
    assert_eq!(php.code, "E0641");
    assert_eq!(php.title, "Closure PHP Output Is Not Yet Available");
    assert!(php.message.contains("Stage 30f"));
}

#[test]
fn type_only_function_syntax_does_not_trigger_target_boundaries() {
    let source = r#"
function accept(function(int): int $callback): void
{
}

function main(): void
{
}
"#;

    doriac::compile_source("stage30d-type-only.doria", source, BackendTarget::Native)
        .expect("type-only function syntax should remain native-lowerable");
    doriac::compile_source("stage30d-type-only.doria", source, BackendTarget::Php)
        .expect("type-only function syntax should remain PHP-lowerable");
}
