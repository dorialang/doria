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

#[test]
fn every_specialization_shares_the_conservative_template_contract() {
    // Decision log entry 51: a template is checked once for every
    // implementation it may select, so `go<Quiet>` carries the same automatic
    // I/O contract as `go<Handle>`. Exact per-specialization contracts are a
    // recorded follow-up; this assertion changes with that work.
    let program = doriac::lower_source_to_mir("selected-calls.doria", SELECTED_CALLS)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let specializations = program
        .functions
        .iter()
        .filter(|function| function.name == "go" || function.name.starts_with("go<"))
        .collect::<Vec<_>>();
    assert_eq!(
        specializations.len(),
        2,
        "{:?}",
        program
            .functions
            .iter()
            .map(|function| &function.name)
            .collect::<Vec<_>>()
    );
    for function in specializations {
        assert!(
            !function.ambient_checked_effects.is_empty(),
            "{} has no ambient contract",
            function.name
        );
    }
}
