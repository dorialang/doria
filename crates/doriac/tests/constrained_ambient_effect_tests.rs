#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;

use doriac::backend::{BackendTarget, NativeProfile};

/// Every call form whose implementation is selected after its body is
/// checked: constrained methods through a generic parameter and a generic
/// field (void and value-returning), a catch around a constrained call,
/// trait requirements through `$this` and `self::`, and an explicit
/// `toString` on a Displayable type parameter, including primitive receivers.
const SELECTED_CALLS: &str = r#"
use Doria\Std\Io\IoError;
interface Rel {
    function release(): void;
    function count(): int;
}
class Handle implements Rel {
    function release(): void { echo "released;"; }
    function count(): int { echo "counted;"; return 7; }
}
class Quiet implements Rel {
    function release(): void {}
    function count(): int { return 1; }
}
function go<F implements Rel>(F $f): void { $f->release(); }
function tally<F implements Rel>(F $f): int { return $f->count(); }
function guarded<F implements Rel>(F $f): int {
    try { return $f->count(); } catch (IoError $error) { return -1; }
}
class Owner<F implements Rel> {
    function __construct(take F $item) {}
    function close(): void { $this->item->release(); }
    function total(): int { return $this->item->count(); }
}
trait Closing {
    function release(): void;
    function closeTwice(): void { $this->release(); $this->release(); }
}
trait Counting {
    static function tick(): void;
    static function twice(): void { self::tick(); self::tick(); }
}
class Door {
    uses Closing;
    uses Counting;
    function release(): void { echo "shut;"; }
    static function tick(): void { echo "tick;"; }
}
class Label implements Displayable {
    function toString(): string { echo "shown;"; return "label"; }
}
function show<T implements Displayable>(T $value): string { return $value->toString(); }
function main(): void {
    let $handle = new Handle();
    go($handle);
    echo tally($handle) . ";";
    echo guarded($handle) . ";";
    let $quiet = new Quiet();
    go($quiet);
    echo tally($quiet) . ";";
    echo guarded($quiet) . ";";
    let $owner = new Owner<Handle>(new Handle());
    $owner->close();
    echo $owner->total() . ";";
    let $door = new Door();
    $door->closeTwice();
    Door::twice();
    let $label = new Label();
    echo show($label) . "\n";
    echo show(5) . show(true) . show("s") . "\n";
}
"#;

const SELECTED_CALLS_OUTPUT: &str =
    "released;counted;7;counted;7;1;1;released;counted;7;shut;shut;tick;tick;shown;label\n5trues\n";

#[test]
fn selected_implementations_route_their_ambient_io_on_every_backend() {
    // `go<Handle>` panicked in MIR lowering, `tally<Handle>` and the field
    // forms failed MIR validation with B0001, the catch in `guarded` was a
    // false E0629, and `show<Label>` panicked: each template recorded only the
    // requirement's declared effects, not the ambient I/O its selected
    // implementation performs.
    let program = doriac::lower_source_to_mir("selected-calls.doria", SELECTED_CALLS)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    doriac::mir_validation::validate_program(&program).unwrap();
    let result = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&result.stdout),
        SELECTED_CALLS_OUTPUT
    );
    assert!(result.stderr.is_empty());
    assert_eq!(result.exit_status, 0);
    native_execution::assert_native_execution(&program, NativeProfile::Fast, SELECTED_CALLS_OUTPUT);
    #[cfg(feature = "llvm-backend")]
    native_execution::assert_native_execution(
        &program,
        NativeProfile::Release,
        SELECTED_CALLS_OUTPUT,
    );

    let directory = execution_directory::ExecutionDirectory::new();
    let artifact =
        doriac::compile_source("selected-calls.doria", SELECTED_CALLS, BackendTarget::Php).unwrap();
    let path = directory.path().join("main.php");
    doriac::backend::publish_output(&path, artifact).unwrap();
    let output = std::process::Command::new("php")
        .current_dir(directory.path())
        .arg(path)
        .output()
        .expect("PHP is required for compatibility parity");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        SELECTED_CALLS_OUTPUT
    );
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn trait_requirement_calls_carry_ambient_io_and_trait_bodies_keep_their_own() {
    // The composer supplies a bodyless requirement, so the authored trait body
    // may catch its ambient I/O. A trait member with a body is the trait's own
    // implementation and keeps its exact profile. A program with no interface
    // or I/O syntax still receives the canonical I/O identities for the
    // requirement's transport.
    for (source, output, status) in [
        (
            r#"
use Doria\Std\Io\IoError;
trait Closing {
    function release(): void;
    function safe(): void { try { $this->release(); } catch (IoError $error) { echo "caught\n"; } }
    function note(): void { echo "noted\n"; }
    function noted(): void { try { $this->note(); } catch (IoError) {} }
}
class Loud { uses Closing; function release(): void { echo "released\n"; } }
function main(): void { let $loud = new Loud(); $loud->safe(); $loud->noted(); }
"#,
            "released\nnoted\n",
            0,
        ),
        (
            "trait Valued { function value(): int; function twice(): int { return $this->value() + $this->value(); } } class Two { uses Valued; function value(): int { return 2; } } function main(): int { return (new Two())->twice(); }",
            "",
            4,
        ),
    ] {
        let program = doriac::lower_source_to_mir("trait-requirements.doria", source)
            .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
        doriac::mir_validation::validate_program(&program).unwrap();
        let result = doriac::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(String::from_utf8_lossy(&result.stdout), output, "{source}");
        assert_eq!(result.exit_status, status, "{source}");
        let (_, analysis) = doriac::analyze_source_for_ide("trait-requirements.doria", source)
            .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
        assert!(analysis.diagnostics.is_empty(), "{:#?}", analysis.diagnostics);
    }
}

/// The specialization of `name` whose first parameter is the class `class`.
fn specialization<'a>(
    program: &'a doriac::mir::Program,
    name: &str,
    class: &str,
) -> &'a doriac::mir::Function {
    program
        .functions
        .iter()
        .filter(|function| function.name == name)
        .find(|function| {
            function.params.first().is_some_and(|parameter| {
                function.locals.iter().any(|local| {
                    local.id == *parameter
                        && matches!(local.ty, doriac::mir::Type::Class(id)
                            if program.classes[id.0].declaration_name == class)
                })
            })
        })
        .unwrap_or_else(|| {
            panic!(
                "{name}<{class}> not lowered: {:?}",
                program
                    .functions
                    .iter()
                    .map(|function| &function.name)
                    .collect::<Vec<_>>()
            )
        })
}

fn function_named<'a>(program: &'a doriac::mir::Program, name: &str) -> &'a doriac::mir::Function {
    program
        .functions
        .iter()
        .find(|function| function.name == name)
        .unwrap_or_else(|| panic!("{name} not lowered"))
}

#[test]
fn every_specialization_carries_only_what_it_selects() {
    // Decision 0123: `go<Quiet>` selects a body that performs no I/O, so its
    // ABI carries no status or Error slot, while `go<Handle>` keeps both. The
    // template's diagnostics are unaffected.
    let program = doriac::lower_source_to_mir("selected-calls.doria", SELECTED_CALLS)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let quiet = specialization(&program, "go", "Quiet");
    assert!(
        quiet.checked_effects.is_empty(),
        "{:?}",
        quiet.checked_effects
    );
    assert!(quiet.ambient_checked_effects.is_empty());
    let handle = specialization(&program, "go", "Handle");
    assert!(!handle.ambient_checked_effects.is_empty());
    assert_eq!(handle.checked_effects, handle.ambient_checked_effects);
}

/// Every form a specialization selects through, in a quiet and a loud version.
const EXACT_CONTRACTS: &str = r#"
use Doria\Std\Io\IoError;
interface Rel {
    function release(): void;
    function count(): int;
}
class Handle implements Rel {
    function release(): void { echo "released;"; }
    function count(): int { echo "counted;"; return 7; }
}
class Quiet implements Rel {
    function release(): void {}
    function count(): int { return 1; }
}
function go<F implements Rel>(F $f): void { $f->release(); }
function logged<F implements Rel>(F $f): int {
    try { return $f->count(); } catch (IoError $error) { echo "failed;"; return -1; }
}
function rethrown<F implements Rel>(F $f): int {
    try { return $f->count(); } catch (IoError $error) { throw $error; }
}
function spin<F implements Rel>(F $f, int $n): void {
    if ($n > 0) { $f->release(); spin($f, $n - 1); }
}
function quietly(): void { go(new Quiet()); }
function loudly(): void { go(new Handle()); }
class Key implements Equatable<Key> {
    function equals(Key $other): bool { return true; }
}
class LoudKey implements Equatable<LoudKey> {
    function equals(LoudKey $other): bool { echo "compared;"; return true; }
}
function same<T implements Equatable<T>>(T $left, T $right): bool { return $left == $right; }
class Tag implements Displayable { function toString(): string { return "tag"; } }
class LoudTag implements Displayable { function toString(): string { echo "shown;"; return "loud"; } }
function render<T implements Displayable>(T $value): string { return "<{$value}>"; }
interface Gauge { int $level { get; } }
class Still implements Gauge { int $level { get => 3; } }
function levelOf<G implements Gauge>(G $gauge): int { return $gauge->level; }
class Cell<T implements Rel> {
    function __construct(take T $item) {}
    function close(): void { $this->item->release(); }
}
function main(): void {
    go(new Handle());
    go(new Quiet());
    echo logged(new Handle()) . ";";
    echo logged(new Quiet()) . ";";
    echo rethrown(new Handle()) . ";";
    echo rethrown(new Quiet()) . ";";
    spin(new Handle(), 1);
    spin(new Quiet(), 1);
    quietly();
    loudly();
    echo (same(new Key(), new Key()) ? "y" : "n") . ";";
    echo (same(new LoudKey(), new LoudKey()) ? "y" : "n") . ";";
    echo render(new Tag()) . render(new LoudTag()) . ";";
    echo levelOf(new Still());
    echo ";";
    let $still = new Cell<Quiet>(new Quiet());
    $still->close();
    let $loud = new Cell<Handle>(new Handle());
    $loud->close();
    echo "\n";
}
"#;

const EXACT_CONTRACTS_OUTPUT: &str = "released;counted;7;1;counted;7;1;released;released;y;compared;y;shown;<tag><loud>;3;released;\n";

fn assert_on_every_backend(name: &str, source: &str, expected: &str) {
    let program =
        doriac::lower_source_to_mir(name, source).unwrap_or_else(|errors| panic!("{errors:#?}"));
    doriac::mir_validation::validate_program(&program).unwrap();
    let result = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(String::from_utf8_lossy(&result.stdout), expected);
    assert!(result.stderr.is_empty());
    assert_eq!(result.exit_status, 0);
    native_execution::assert_native_execution(&program, NativeProfile::Fast, expected);
    #[cfg(feature = "llvm-backend")]
    native_execution::assert_native_execution(&program, NativeProfile::Release, expected);

    let directory = execution_directory::ExecutionDirectory::new();
    let artifact = doriac::compile_source(name, source, BackendTarget::Php).unwrap();
    let path = directory.path().join("main.php");
    doriac::backend::publish_output(&path, artifact).unwrap();
    let output = std::process::Command::new("php")
        .current_dir(directory.path())
        .arg(path)
        .output()
        .expect("PHP is required for compatibility parity");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn quiet_and_loud_specializations_run_on_every_backend() {
    assert_on_every_backend(
        "exact-contracts.doria",
        EXACT_CONTRACTS,
        EXACT_CONTRACTS_OUTPUT,
    );
}

#[test]
fn each_selection_form_narrows_its_specialization() {
    let program = doriac::lower_source_to_mir("exact-contracts.doria", EXACT_CONTRACTS)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    for (name, quiet, loud) in [
        // A constrained method, void and through a handler.
        ("go", "Quiet", "Handle"),
        // The handler's own I/O belongs to specializations that can enter it.
        ("logged", "Quiet", "Handle"),
        ("rethrown", "Quiet", "Handle"),
        // Recursion that never reaches an operation stays empty.
        ("spin", "Quiet", "Handle"),
        // Constrained core operations.
        ("same", "Key", "LoudKey"),
        // Display conversion of a type parameter.
        ("render", "Tag", "LoudTag"),
    ] {
        let narrowed = specialization(&program, name, quiet);
        assert!(
            narrowed.checked_effects.is_empty(),
            "{name}<{quiet}>: {:?}",
            narrowed.checked_effects
        );
        assert!(
            !specialization(&program, name, loud)
                .ambient_checked_effects
                .is_empty(),
            "{name}<{loud}>"
        );
    }
    // A constrained accessor selects the hook of its class. Hooks cannot block,
    // so only the quiet form exists.
    assert!(specialization(&program, "levelOf", "Still")
        .checked_effects
        .is_empty());
    // A caller's contract follows the specialization it calls.
    assert!(function_named(&program, "quietly")
        .checked_effects
        .is_empty());
    assert!(!function_named(&program, "loudly")
        .checked_effects
        .is_empty());
    // A method of a generic class specializes with its class.
    let closes = program
        .functions
        .iter()
        .filter(|function| function.name.ends_with("::close"))
        .map(|function| function.checked_effects.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(closes.len(), 2, "{closes:?}");
    assert!(
        closes.contains(&true) && closes.contains(&false),
        "{closes:?}"
    );
}

#[test]
fn a_specialization_keeps_the_errors_its_template_declares() {
    // Required errors and written ambient `throws` entries are authored
    // contract, unlike inferred ambient I/O, so a quiet implementation keeps its
    // template's declared transport and its callers must route it.
    let source = r#"
use Doria\Std\Io\IoError;
class Boom implements Error { function __construct(string $message) {} }
interface Task { function run(): void throws Boom; }
class Safe implements Task { function run(): void {} }
function drive<T implements Task>(T $task): void throws Boom { $task->run(); }
interface Step { function step(): void; }
class Mute implements Step { function step(): void {} }
function declared<T implements Step>(T $step): void throws IoError { $step->step(); }
function forwarder(): void { declared(new Mute()); }
function main(): void {
    try { drive(new Safe()); echo "ran\n"; } catch (Boom $boom) { echo "boom\n"; }
    forwarder();
}
"#;
    assert_on_every_backend("declared-errors.doria", source, "ran\n");
    let program = doriac::lower_source_to_mir("declared-errors.doria", source)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let drive = specialization(&program, "drive", "Safe");
    assert_eq!(drive.required_checked_effects.len(), 1);
    assert!(drive.ambient_checked_effects.is_empty());
    assert_eq!(
        specialization(&program, "declared", "Mute")
            .ambient_checked_effects
            .len(),
        1
    );
    // The declared entry is in the callee's ABI, so its caller carries it too.
    assert_eq!(
        function_named(&program, "forwarder")
            .ambient_checked_effects
            .len(),
        1
    );
}

#[test]
fn the_entry_frame_owns_the_effects_of_top_level_statements() {
    // MIR runs the entry source's top-level statements in main's frame, so
    // their I/O needs main's transport even when main's own body has none.
    assert_on_every_backend(
        "entry-prelude.doria",
        "echo \"top;\";\nfunction main(): void {}\n",
        "top;",
    );
}

/// Calls whose implementation is chosen outside the body: a callback
/// parameter, a callback forwarded through a named helper, a function value
/// in a local, a trait requirement satisfied by a quiet and a loud composer,
/// a quiet composer's copy called from a guarded caller, a destructor that
/// guards a stored callback, and a destructor that maps with a pure literal.
/// Every caller passes only pure closures and every composer of `safe` may be
/// quiet, so each catch here is never taken at run time.
const CALLER_INDEPENDENT_CATCHES: &str = r#"
use Doria\Std\Io\InvalidUtf8Error;
use Doria\Std\Io\IoError;
function run(function(): int $cb): int {
    try { return $cb(); } catch (IoError $error) { return -1; }
}
function relay(function(): int $cb): int { return $cb(); }
function guardedRelay(): int {
    try { return relay(fn() => 6); } catch (IoError) { return -3; }
}
function viaLocal(): int {
    function(): int $local = fn() => 4;
    try { return $local(); } catch (IoError) { return -2; }
}
trait Closing {
    function release(): void;
    function safe(): void { try { $this->release(); } catch (IoError $error) { echo "caught;"; } }
    function closeAll(): void { $this->release(); }
}
class Quiet { uses Closing; function release(): void {} }
class Loud { uses Closing; function release(): void { echo "loud;"; } }
function guardedClose(): int {
    let $quiet = new Quiet();
    try { $quiet->closeAll(); return 7; } catch (IoError) { return -4; }
}
class Hook {
    function __construct(take function(): void $callback) {}
    function __destruct() {
        try { $this->callback(); } catch (IoError) {} catch (InvalidUtf8Error) {}
    }
}
class Tally {
    function __destruct() {
        List<int> $values = [1, 2];
        List<int> $doubled = $values->map(fn(int $value) => $value * 2);
    }
}
function main(): void {
    echo run(fn() => 3) . ";";
    echo guardedRelay() . ";";
    echo viaLocal() . ";";
    let $quiet = new Quiet();
    $quiet->safe();
    let $loud = new Loud();
    $loud->safe();
    echo guardedClose() . ";";
    {
        let $hook = new Hook(function (): void {});
        let $tally = new Tally();
    }
    echo "done\n";
}
"#;

const CALLER_INDEPENDENT_OUTPUT: &str = "3;6;4;loud;7;done\n";

/// One unrelated conditional member moves a program onto the authored source
/// check plus the executable recheck of admitted and composed bodies.
const CONDITIONAL_DECLARATION: &str =
    "interface Ready {}\nclass Gate<T> { function admit(): void where (T implements Ready) {} }\n";

#[test]
fn callers_and_composers_never_decide_catch_reachability_on_every_backend() {
    // Each catch above was a false E0629 or each destructor a false
    // E0631 once the only closures or composer in the program were quiet:
    // the narrowed transport of the callback or the composed copy decided
    // the authored body's diagnostics. Transport stays narrow, so the
    // catches compile to routes that are never taken.
    let conditional = format!("{CALLER_INDEPENDENT_CATCHES}{CONDITIONAL_DECLARATION}");
    for source in [CALLER_INDEPENDENT_CATCHES, conditional.as_str()] {
        let (_, analysis) = doriac::analyze_source_for_ide("caller-independent.doria", source)
            .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
        assert!(
            analysis.diagnostics.is_empty(),
            "{source}\n{:#?}",
            analysis.diagnostics
        );
        let program = doriac::lower_source_to_mir("caller-independent.doria", source)
            .unwrap_or_else(|errors| panic!("{source}\n{errors:#?}"));
        doriac::mir_validation::validate_program(&program).unwrap();
        let result = doriac::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&result.stdout),
            CALLER_INDEPENDENT_OUTPUT
        );
        assert!(result.stderr.is_empty());
        assert_eq!(result.exit_status, 0);
        native_execution::assert_native_execution(
            &program,
            NativeProfile::Fast,
            CALLER_INDEPENDENT_OUTPUT,
        );
        #[cfg(feature = "llvm-backend")]
        native_execution::assert_native_execution(
            &program,
            NativeProfile::Release,
            CALLER_INDEPENDENT_OUTPUT,
        );

        let directory = execution_directory::ExecutionDirectory::new();
        let artifact =
            doriac::compile_source("caller-independent.doria", source, BackendTarget::Php).unwrap();
        let path = directory.path().join("main.php");
        doriac::backend::publish_output(&path, artifact).unwrap();
        let output = std::process::Command::new("php")
            .current_dir(directory.path())
            .arg(path)
            .output()
            .expect("PHP is required for compatibility parity");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            CALLER_INDEPENDENT_OUTPUT
        );
        assert!(output.stderr.is_empty(), "{output:?}");
    }
}

#[test]
fn narrowed_transport_keeps_callers_and_composers_off_the_catch_route() {
    // Reach decides diagnostics; transport still decides execution. A
    // quiet composer's copy and a callback parameter that only ever receives
    // pure closures keep their narrowed contracts, so `Quiet::closeAll` and
    // `relay` use the ordinary ABI, while the loud composer's copy keeps its
    // exact I/O transport. Both hold for ordinary and conditional programs.
    let conditional = format!("{CALLER_INDEPENDENT_CATCHES}{CONDITIONAL_DECLARATION}");
    for source in [CALLER_INDEPENDENT_CATCHES, conditional.as_str()] {
        assert_narrowed_transport(source);
    }
}

fn assert_narrowed_transport(source: &str) {
    let program = doriac::lower_source_to_mir("caller-independent.doria", source)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let contract = |name: &str| {
        program
            .functions
            .iter()
            .find(|function| function.name == name)
            .unwrap_or_else(|| {
                panic!(
                    "{name} not lowered: {:?}",
                    program
                        .functions
                        .iter()
                        .map(|function| &function.name)
                        .collect::<Vec<_>>()
                )
            })
            .ambient_checked_effects
            .clone()
    };
    for quiet in ["relay", "Quiet::closeAll"] {
        assert!(contract(quiet).is_empty(), "{quiet}: {:?}", contract(quiet));
    }
    assert!(!contract("Loud::closeAll").is_empty());
}

#[test]
fn the_authored_body_still_rejects_catches_it_cannot_reach() {
    // The authored verdict is not weakened: a named callee that handles every
    // Error, a closure literal invoked in place, and a trait member's own
    // quiet body still make the catch unreachable. A trait catch is reported
    // once at the authored trait body, never once per composer.
    for (label, source, expected) in [
        (
            "handled by a named callee",
            "use Doria\\Std\\Io\\IoError;\nfunction shield(function(): int $cb): int { try { return $cb(); } catch (Error) { return -1; } }\nfunction main(): int { try { return shield(fn() => 3); } catch (IoError) { return -2; } }",
            1,
        ),
        (
            "closure literal invoked in place",
            "use Doria\\Std\\Io\\IoError;\nfunction main(): int { try { return (fn() => 3)(); } catch (IoError) { return -2; } }",
            1,
        ),
        (
            "trait member's own quiet body, two composers",
            "use Doria\\Std\\Io\\IoError;\ntrait Noting { function note(): void {} function noted(): void { try { $this->note(); } catch (IoError) {} } }\nclass First { uses Noting; }\nclass Second { uses Noting; }\nfunction main(): void { (new First())->noted(); (new Second())->noted(); }",
            1,
        ),
    ] {
        let conditional = format!("{source}\n{CONDITIONAL_DECLARATION}");
        for source in [source, conditional.as_str()] {
            let diagnostics = doriac::check_source("authored-verdict.doria", source)
                .expect_err(label);
            let unreachable = diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "E0629")
                .collect::<Vec<_>>();
            assert_eq!(unreachable.len(), expected, "{label}\n{source}\n{diagnostics:#?}");
            assert!(
                diagnostics.iter().all(|diagnostic| diagnostic.code == "E0629"),
                "{label}\n{diagnostics:#?}"
            );
        }
    }
}

#[test]
fn destructors_judge_stored_callbacks_by_their_contract_not_by_what_is_stored() {
    // Only an empty closure is ever stored, yet the destructor may run any
    // `function(): void` its callers supply, so leaving ambient I/O uncaught
    // is E0631 whatever the program's callers pass.
    let source = r#"
class Hook {
    function __construct(take function(): void $callback) {}
    function __destruct() { $this->callback(); }
}
function main(): void { let $hook = new Hook(function (): void {}); }
"#;
    let conditional = format!("{source}{CONDITIONAL_DECLARATION}");
    for source in [source, conditional.as_str()] {
        let diagnostics = doriac::check_source("stored-callback.doria", source)
            .expect_err("a destructor must catch its stored callback's ambient I/O");
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0631"),
            "{diagnostics:#?}"
        );
    }
}
