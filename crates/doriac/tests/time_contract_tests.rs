#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[path = "common/unix_failure_injection.rs"]
mod unix_failure_injection;

use doriac::backend::{BackendTarget, NativeProfile};
use doriac::compiler_known_time::CLOCK_ERROR;
use doriac::inline_value::{InlineValue, InlineValueType};
use doriac::mir;
use doriac::names::{CompilerKnownProvenance, CompilerSymbolIdentity, GlobalSymbolOwner};

const SOURCE: &str = r#"
use Doria\Std\Time\ClockError as TimeFailure;
use Doria\Std\Io\IoError;

function failClock(?int $code = null): void throws TimeFailure {
    throw new TimeFailure("clock unavailable", systemCode: $code);
}
function forwardClock(): void throws TimeFailure {
    try { failClock(0); }
    catch (TimeFailure $error) { throw $error; }
}
function describeError(Error $error): string { return $error->message; }
function main(): void {
    try { forwardClock(); }
    catch (TimeFailure $error) {
        echo describeError($error) . ": " . ($error->systemCode ?? -1) . "\n";
    }
    try { failClock(); }
    catch (TimeFailure $error) {
        if ($error->systemCode == null) { echo "no system code\n"; }
    }
}
"#;

const DURATION_SOURCE: &str = include_str!("../../../examples/native/main_signed_duration.doria");
const DEADLINE_SOURCE: &str = include_str!("../../../examples/native/main_relative_deadline.doria");

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn real_clock_failure_preserves_the_time_owned_checked_error_across_backends() {
    unix_failure_injection::assert_execution(
        r#"
use Doria\Std\Time\{Deadline, Duration, ClockError};
function main(): void {
    try {
        let $deadline = Deadline::fromNow(Duration::fromNanoseconds(1));
        echo "clock available\n";
    } catch (ClockError $error) {
        if (($error->systemCode ?? -1) != 5) { panic("clock failure code lost"); }
        echo $error->message . "\n";
    }
}
"#,
        "clock",
        "clock available\n",
        &format!("{}\n", doriac::compiler_known_time::CLOCK_FAILURE_MESSAGE),
    );
}

#[test]
fn relative_deadline_factory_preserves_copy_transport_across_backends() {
    assert_backend_parity_with_runtime(
        "relative-deadline.doria",
        DEADLINE_SOURCE,
        include_str!("fixtures/native_io/main_relative_deadline/expected_stdout"),
        true,
    );
}

#[test]
fn escaping_object_preserves_wide_deadline_fields_across_backends() {
    assert_backend_parity_with_runtime(
        "deadline-object.doria",
        r#"
use Doria\Std\Time\{ClockError, Deadline, Duration};
class Holder {
    function __construct(int $prefix, Deadline $deadline, ?Deadline $optional) {}
}
function create(Deadline $deadline): Holder {
    return new Holder(17, $deadline, $deadline);
}
function main(): void throws ClockError {
    let $deadline = Deadline::fromNow(Duration::fromNanoseconds(-1));
    let $holder = create($deadline);
    echo ($holder->prefix == 17) . "\n";
    echo ($holder->deadline == $deadline) . "\n";
    echo (($holder->optional ?? $deadline) == $deadline) . "\n";
}
"#,
        "true\ntrue\ntrue\n",
        true,
    );
}

#[test]
fn relative_deadline_requires_duration_and_time_owned_checked_effect() {
    let source = r#"
use Doria\Std\Time\{ClockError, Deadline, Duration};
function create(Duration $duration): Deadline throws ClockError {
    return Deadline::fromNow(duration: $duration);
}
function main(): void {}
"#;
    doriac::check_source("deadline-factory.doria", source).unwrap();
    for invalid in [
        source.replace(" throws ClockError", ""),
        source.replace("throws ClockError", "throws Doria\\Std\\Io\\IoError"),
        source.replace("duration: $duration", "duration: 1"),
        source.replace("duration: $duration", "nanoseconds: $duration"),
    ] {
        assert!(doriac::check_source("invalid-deadline-factory.doria", invalid).is_err());
    }
}

#[test]
fn relative_deadline_mir_rejects_incompatible_slots_edges_and_effects() {
    let valid = doriac::lower_source_to_mir("deadline-mir.doria", DEADLINE_SOURCE).unwrap();
    for fault in 0..6 {
        let mut invalid = valid.clone();
        let function = invalid
            .functions
            .iter_mut()
            .find(|function| {
                function.blocks.iter().any(|block| {
                    matches!(
                        block.terminator,
                        mir::Terminator::CheckedDeadlineFromNow { .. }
                    )
                })
            })
            .unwrap();
        let terminator = &mut function
            .blocks
            .iter_mut()
            .find(|block| {
                matches!(
                    block.terminator,
                    mir::Terminator::CheckedDeadlineFromNow { .. }
                )
            })
            .unwrap()
            .terminator;
        let mir::Terminator::CheckedDeadlineFromNow {
            duration,
            result,
            error,
            success,
            failure,
            ..
        } = terminator
        else {
            unreachable!()
        };
        match fault {
            0 => {
                function.locals[duration.0].ty = mir::Type::Scalar(mir::ScalarType::Integer(
                    doriac::numeric::IntegerType::Int64,
                ))
            }
            1 => *result = None,
            2 => function.locals[error.0].owned = false,
            3 => *failure = *success,
            4 => function.checked_effects.clear(),
            5 => function.locals[result.unwrap().0].owned = true,
            _ => unreachable!(),
        }
        assert!(
            doriac::mir_validation::validate_program(&invalid).is_err(),
            "fault {fault}"
        );
    }
}

#[test]
fn wait_modes_use_ordinary_copy_enum_transport() {
    assert_backend_parity(
        "wait-value.doria",
        r#"
use Doria\Std\Io\Wait;
use Doria\Std\Time\Duration;
class Holder { function __construct(Wait $wait) {} }
function identity<T>(T $value): T { return $value; }
function main(): void {
    let $interval = Duration::fromNanoseconds(-17);
    let $wait = Wait::For($interval);
    let $copy = identity($wait);
    let $holder = new Holder($wait);
    List<Wait> $modes = [Wait::Immediate, $copy, Wait::Indefinite];
    List<?Wait> $optional = [null, $copy];
    if ($optional[0] != null || ($optional[1] ?? Wait::Immediate) != $wait) {
        panic("nullable wait copy changed");
    }
    if ($modes[1] != $wait || $holder->wait != $copy) { panic("wait copy changed"); }
    echo match ($copy) { Wait::For($duration) => $duration->nanoseconds, default => 0 } . "\n";
    echo match ($modes[0]) { Wait::Immediate => "immediate", default => "wrong" } . "\n";
    echo match ($modes[2]) { Wait::Indefinite => "indefinite", default => "wrong" } . "\n";
}
"#,
        "-17\nimmediate\nindefinite\n",
    );
}

#[test]
fn signed_duration_factory_and_getter_preserve_exact_values_across_all_backends() {
    let expected = include_str!("fixtures/native_io/main_signed_duration/expected_stdout");
    assert_backend_parity("signed-duration.doria", DURATION_SOURCE, expected);
}

fn assert_backend_parity(name: &str, source: &str, expected: &str) {
    assert_backend_parity_with_runtime(name, source, expected, false);
}

fn assert_backend_parity_with_runtime(
    name: &str,
    source: &str,
    expected: &str,
    native_runtime: bool,
) {
    let program = doriac::lower_source_to_mir(name, source).unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
    native_execution::assert_native_execution(&program, NativeProfile::Fast, expected);
    #[cfg(feature = "llvm-backend")]
    native_execution::assert_native_execution(&program, NativeProfile::Release, expected);
    let artifact = doriac::compile_source(name, source, BackendTarget::Php).unwrap();
    assert_eq!(
        matches!(
            artifact,
            doriac::backend::BackendOutput::TextWithRuntime { .. }
        ),
        native_runtime
    );
    let directory = native_execution::ExecutionDirectory::new();
    let path = directory.path().join("main.php");
    doriac::backend::publish_output(&path, artifact).unwrap();
    let output = std::process::Command::new("php")
        .arg("-d")
        .arg(if native_runtime {
            "ffi.enable=1"
        } else {
            "ffi.enable=0"
        })
        .arg(path)
        .current_dir(directory.path())
        .output()
        .expect("PHP is required for Time parity");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn duration_intrinsics_keep_scalar_abi_and_no_heap_lifecycle() {
    let program = doriac::lower_source_to_mir("duration-abi.doria", DURATION_SOURCE).unwrap();
    let methods = program
        .functions
        .iter()
        .filter(|function| {
            function.method.as_ref().is_some_and(|method| {
                program.classes[method.class.0].name == doriac::compiler_known_time::DURATION
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(methods.len(), 2);
    for function in methods {
        assert_eq!(function.params.len(), 1);
        assert!(function.locals.iter().all(|local| matches!(
            local.ty,
            mir::Type::Scalar(mir::ScalarType::Integer(
                doriac::numeric::IntegerType::Int64
            )) | mir::Type::Scalar(mir::ScalarType::Inline(InlineValueType::Duration))
        )));
        assert!(matches!(
            function.return_type,
            mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Integer(
                doriac::numeric::IntegerType::Int64
            ))) | mir::ReturnType::Value(mir::Type::Scalar(mir::ScalarType::Inline(
                InlineValueType::Duration
            )))
        ));
        assert!(function.checked_effects.is_empty());
        let class = &program.classes[function.method.as_ref().unwrap().class.0];
        assert!(class.constructor.is_none() && class.destructor.is_none());
    }
}

#[test]
fn duration_numeric_surface_does_not_allow_implicit_conversions_or_writes() {
    for body in [
        "Duration::fromNanoseconds(UInt64::from(1));",
        "Duration::fromNanoseconds(Int32::from(1));",
        "Duration::fromNanoseconds(1.0);",
        "Duration $interval = 1;",
        "int64 $number = $interval;",
        "$interval->nanoseconds = 1;",
        "$interval->nanoseconds++;",
    ] {
        let source = format!("use Doria\\Std\\Time\\Duration; function invalid(Duration $interval): void {{ {body} }}");
        assert!(
            doriac::check_source("invalid-duration.doria", source).is_err(),
            "{body}"
        );
    }
}

#[test]
fn mir_rejects_invalid_duration_factory_and_getter_operand_types() {
    let valid = doriac::lower_source_to_mir(
        "duration-operation-types.doria",
        r#"
use Doria\Std\Time\Duration;
function main(): void { echo Duration::fromNanoseconds(-1)->nanoseconds; }
"#,
    )
    .unwrap();
    for factory in [true, false] {
        let mut invalid = valid.clone();
        let mut replaced = false;
        for function in &mut invalid.functions {
            for block in &mut function.blocks {
                match &mut block.terminator {
                    mir::Terminator::Return(mir::Rvalue::Value(mir::ValueExpression::Inline(
                        mir::InlineValueExpression::DurationFromNanoseconds(value),
                    ))) if factory => {
                        **value =
                            mir::IntegerExpression::constant(doriac::numeric::IntegerValue::zero(
                                doriac::numeric::IntegerType::UInt64,
                            ));
                        replaced = true;
                    }
                    mir::Terminator::Return(mir::Rvalue::Value(mir::ValueExpression::Integer(
                        mir::IntegerExpression::DurationNanoseconds(value),
                    ))) if !factory => {
                        **value = mir::InlineValueExpression::Use {
                            ty: InlineValueType::Deadline,
                            mode: mir::InlineValueUseMode::Borrow,
                            operand: mir::Operand::Scalar(mir::ScalarValue::Inline(InlineValue {
                                ty: InlineValueType::Deadline,
                                bits: 0,
                            })),
                        };
                        replaced = true;
                    }
                    _ => {}
                }
            }
        }
        assert!(
            replaced,
            "intrinsic must lower to an explicit typed MIR operation"
        );
        let error = doriac::mir_validation::validate_program(&invalid).unwrap_err();
        let expected = if factory {
            "Duration factory requires int64 nanoseconds"
        } else {
            "nanoseconds getter requires Duration"
        };
        assert!(error.message.contains(expected), "{error:?}");
    }
}

#[test]
fn time_error_preserves_owned_failure_facts_across_all_backends() {
    let expected = "clock unavailable: 0\nno system code\n";
    let program = doriac::lower_source_to_mir("time-error.doria", SOURCE).unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
    native_execution::assert_native_execution(&program, NativeProfile::Fast, expected);
    #[cfg(feature = "llvm-backend")]
    native_execution::assert_native_execution(&program, NativeProfile::Release, expected);
    let artifact = doriac::compile_source("time-error.doria", SOURCE, BackendTarget::Php).unwrap();
    let directory = native_execution::ExecutionDirectory::new();
    let path = directory.path().join("main.php");
    doriac::backend::publish_output(&path, artifact).unwrap();
    let output = std::process::Command::new("php")
        .arg(path)
        .current_dir(directory.path())
        .output()
        .expect("PHP is required for checked Time error parity");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn ide_facts_keep_time_ownership_and_do_not_expose_synthetic_items() {
    let authored = doriac::parse_source("time-error.doria", SOURCE).unwrap();
    let (program, analysis) = doriac::analyze_source_for_ide("time-error.doria", SOURCE).unwrap();
    assert!(
        analysis.diagnostics.is_empty(),
        "{:?}",
        analysis.diagnostics
    );
    assert_eq!(program.items.len(), authored.items.len());
    let known = analysis
        .info
        .global_symbols
        .compiler_known
        .iter()
        .find(|fact| fact.id.qualified_name == CLOCK_ERROR)
        .unwrap();
    assert_eq!(known.provenance, CompilerKnownProvenance::StandardTime);
    assert!(matches!(&known.id.owner,
        GlobalSymbolOwner::CompilerKnown(CompilerSymbolIdentity::StandardTime(name))
        if name == CLOCK_ERROR));
    let hir = doriac::lower_source("time-error.doria", SOURCE).unwrap();
    assert!(hir.sources.iter().any(|source| {
        source.id == doriac::compiler_known_time::SOURCE_ID
            && source.package == doriac::names::PackageIdentity::CompilerKnown
    }));
}

#[test]
fn time_errors_have_exact_required_effects_not_automatic_io_effects() {
    let source = r#"
use Doria\Std\Time\ClockError;
function fail(): void throws ClockError { throw new ClockError("failed"); }
function main(): void { fail(); }
"#;
    let missing = source.replace("function main(): void {", "function caller(): void {");
    let diagnostics = doriac::check_source("missing-clock-effect.doria", missing).unwrap_err();
    assert!(
        diagnostics
            .iter()
            .any(|error| error.message.contains("ClockError")),
        "{diagnostics:?}"
    );
    for main_signature in [
        "function main(): void {",
        "function main(): void throws ClockError {",
    ] {
        let declared = source.replace("function main(): void {", main_signature);
        let hir = doriac::lower_source("declared-clock-effect.doria", declared).unwrap();
        let main = hir
            .items
            .iter()
            .find_map(|item| match item {
                doriac::hir::Item::Function(function) if function.name == "main" => Some(function),
                _ => None,
            })
            .unwrap();
        assert_eq!(main.required_checked_effects.len(), 1);
        assert_eq!(main.checked_effects.len(), 1);
        assert!(matches!(&main.checked_effects[0],
            doriac::types::ResolvedType::Class(class) if class.name == CLOCK_ERROR));
    }
    let wrong_catch = source.replace(
        "function main(): void { fail(); }",
        "function main(): void { try { fail(); } catch (Doria\\Std\\Io\\IoError) {} }",
    );
    let diagnostics = doriac::check_source("wrong-clock-catch.doria", wrong_catch).unwrap_err();
    assert!(
        diagnostics.iter().any(|error| error.code == "E0629"),
        "{diagnostics:?}"
    );
}

#[test]
fn canonical_time_error_is_reserved_but_not_an_unqualified_prelude_alias() {
    for declaration in [
        "class ClockError {}",
        "interface ClockError {}",
        "trait ClockError {}",
        "enum ClockError { case Failed; }",
    ] {
        let source = format!("namespace Doria\\Std\\Time; {declaration}");
        let diagnostics = doriac::check_source("reserved-clock-error.doria", source).unwrap_err();
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "E0640"),
            "{diagnostics:?}"
        );
    }
    assert!(doriac::check_source(
        "no-time-prelude.doria",
        "function inspect(ClockError $error): void {}"
    )
    .is_err());
    doriac::check_source(
        "qualified-clock-error.doria",
        "function inspect(Doria\\Std\\Time\\ClockError $error): void {}",
    )
    .unwrap();
}

#[test]
fn clock_error_facts_are_readonly_even_through_writable_access() {
    for assignment in ["$error->message = \"changed\";", "$error->systemCode = 0;"] {
        let source = format!("use Doria\\Std\\Time\\ClockError; function alter(writable ClockError $error): void {{ {assignment} }}");
        assert!(doriac::check_source("readonly-clock-error.doria", source).is_err());
    }
}

#[test]
fn package_graphs_share_the_same_canonical_time_error_and_source_context() {
    let mut document = doriac::build_plan::parse_build_plan_document(
        "time-error-plan.json",
        include_str!("fixtures/stage31_package_graph/cross_file_execution/build-plan.json"),
    )
    .unwrap();
    for package in &mut document.plan.packages {
        package.root = ".".to_string();
        if package.identity == "acme/support" {
            package.namespace_mappings[0].prefix = "Acme\\".to_string();
        }
    }
    let mut provider = doriac::source_provider::InMemorySourceProvider::new();
    provider.insert(
        "acme/support",
        "Support.doria",
        r#"
namespace Acme;
use Doria\Std\Time\ClockError;
class Support {
    static function failure(): ClockError { return new ClockError("graph clock", 17); }
}
"#,
    );
    provider.insert(
        "acme/application",
        "main.doria",
        r#"
use Acme\Support;
use Doria\Std\Time\ClockError as TimeFailure;
function main(): void {
    try { throw Support::failure(); }
    catch (TimeFailure $error) { echo "{$error->message} {$error->systemCode ?? -1}\n"; }
}
"#,
    );
    let graph = doriac::compilation_graph::load_compilation_graph(&document, &provider).unwrap();
    let hir = doriac::lower_compilation_graph(&graph).unwrap();
    assert!(hir
        .sources
        .iter()
        .any(|source| source.id == doriac::compiler_known_time::SOURCE_ID));
    let mir = doriac::lower_compilation_graph_to_mir(&graph).unwrap();
    doriac::mir_validation::validate_program(&mir).unwrap();
    let output = doriac::mir_interpreter::interpret(&mir).unwrap();
    assert_eq!(output.stdout, b"graph clock 17\n");
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
}

const COPY_SOURCE: &str = r#"
use Doria\Std\Time\Duration as Delay;
use Doria\Std\Time\Deadline;

class Holder {
    function __construct(Delay $delay, ?Deadline $deadline) {}
    Delay $copied { get => $this->delay; }
}
class Box<T> { function __construct(take T $value) {} }
class Defaults {
    const ?Delay ABSENT = null;
    static ?Delay $delay = self::ABSENT;
    static ?Deadline $deadline = null;
}
enum Packet { case Delay(Delay $value); }
function identity<T>(T $value): T { return $value; }
function replace(writable Delay $value, Delay $other): Delay { $value = $other; return $value; }
function keep(Delay $value): Delay {
    Delay $copy = $value;
    return $value;
}
function select(?Delay $value, Delay $fallback): Delay { return $value ?? $fallback; }
function optional(?Delay $value = null): ?Delay { return $value; }
function optionalDeadline(?Deadline $value = null): ?Deadline { return $value; }
function replaceDeadline(writable Deadline $value, Deadline $other): Deadline { $value = $other; return $value; }
function captureDeadline(Deadline $value): function(): Deadline {
    return fn() with (take $value) => $value;
}
function copyCallback<T>(T $input): function(T): T {
    return fn(T $value) => $value;
}
function capture(Delay $value): function(): Delay {
    return fn() with (take $value) => $value;
}
function probe(Delay $first, Delay $second, Deadline $deadline, Deadline $otherDeadline): void {
    if ($deadline == $otherDeadline) { panic("deadline high bits collapsed"); }
    if (keep($first) != $first || keep($first) != $first) { panic("copy changed"); }
    if ($first == $second) { panic("bits collapsed"); }
    ?Delay $present = $first;
    ?Delay $absent = null;
    if (select($present, $second) != $first || select($absent, $second) != $second) {
        panic("nullable copy changed");
    }
    let $holder = new Holder($first, $deadline);
    if ($holder->delay != $first || $holder->copied != $first || ($holder->deadline ?? $deadline) != $deadline) {
        panic("property copy changed");
    }
    List<Delay> $values = [$first, $second];
    if ($values[0] != $first || $values[1] != $second) { panic("collection copy changed"); }
    foreach ($values as int $index => Delay $value) {
        if ($value != $values[$index]) { panic("iteration copy changed"); }
    }
    List<?Delay> $optionalValues = [null, $first];
    if (select($optionalValues[0], $second) != $second || select($optionalValues[1], $second) != $first) {
        panic("nullable collection copy changed");
    }
    Delay[] $array = [$first, $second];
    Dictionary<string, Delay> $dictionary = ["first" => $first, "second" => $second];
    if ($array[0] != $first || $dictionary["second"] != $second) { panic("aggregate copy changed"); }
    writable List<Deadline> $deadlines = [$deadline, $otherDeadline];
    List<?Deadline> $optionalDeadlines = [null, $deadline];
    Deadline[] $deadlineArray = [$deadline, $otherDeadline];
    writable Dictionary<string, Deadline> $deadlineDictionary = ["past" => $deadline];
    if ($deadlines[1] != $otherDeadline || ($optionalDeadlines[1] ?? $otherDeadline) != $deadline
        || $optionalDeadlines[0] != null || $deadlineArray[0] != $deadline
        || ($deadlineDictionary->get("past") ?? $otherDeadline) != $deadline
        || $deadlineDictionary->get("absent") != null) { panic("wide collection copy changed"); }
    if ($deadlines->removeAt(0) != $deadline || ($deadlines->pop() ?? $deadline) != $otherDeadline
        || $deadlines->pop() != null || ($deadlineDictionary->remove("past") ?? $otherDeadline) != $deadline
        || $deadlineDictionary->remove("absent") != null) { panic("wide collection removal changed"); }
    let $wait = Doria\Std\Io\Wait::Until($deadline);
    let $until = match ($wait) { Doria\Std\Io\Wait::Until($value) => $value, default => $otherDeadline };
    if ($until != $deadline || identity($deadline) != $deadline) { panic("wide enum copy changed"); }
    let $reader = captureDeadline($deadline);
    if ($reader() != $deadline || $reader() != $deadline) { panic("wide capture copy changed"); }
    let $copyDeadline = copyCallback($deadline);
    if ($copyDeadline($deadline) != $deadline || $copyDeadline($otherDeadline) != $otherDeadline) {
        panic("generic inline callback changed");
    }
    writable Deadline $replacement = $deadline;
    if (replaceDeadline($replacement, $otherDeadline) != $otherDeadline || $replacement != $deadline) {
        panic("wide writable parameter changed");
    }
    let $generic = new Box<Delay>($first);
    if (identity($first) != $first || $generic->value != $first) { panic("generic copy changed"); }
    if (optional() != null || (optional($first) ?? $second) != $first) { panic("nullable default changed"); }
    if (Defaults::delay != null || Defaults::deadline != null || optionalDeadline() != null) {
        panic("static default changed");
    }
    if ((optionalDeadline($deadline) ?? $deadline) != $deadline || replace($first, $second) != $second || keep($first) != $first) {
        panic("parameter copy changed");
    }
    let $packet = Packet::Delay($first);
    let $packetCopy = $packet;
    let $unpacked = match ($packet) { Packet::Delay($value) => $value };
    let $again = match ($packetCopy) { Packet::Delay($value) => $value };
    if ($unpacked != $first || $again != $first) { panic("enum copy changed"); }
    let $read = capture($first);
    if ($read() != $first || $read() != $first) { panic("capture copy changed"); }
    mixed $box = $first;
    if ($box is Delay) {
        Delay $value = $box;
        if ($value != $first) { panic("mixed copy changed"); }
    } else { panic("mixed identity changed"); }
    mixed $other = $deadline;
    if ($other is Delay) { panic("Time identities collapsed"); }
    if (not ($other is Deadline)) { panic("deadline identity changed"); }
    if ($other is Deadline) {
        Deadline $unboxed = $other;
        if ($unboxed != $deadline || $unboxed == $otherDeadline) { panic("wide mixed copy changed"); }
    }
}
function seed(): void {}
function main(): void { seed(); echo "copy preserved\n"; }
"#;

// Opaque test-only constants exercise deterministic carrier boundaries without
// sampling the clock. Source still uses the ordinary checker and MIR lowering.
fn inline_copy_program(first: u64, second: u64) -> mir::Program {
    let mut program = doriac::lower_source_to_mir("inline-copy.doria", COPY_SOURCE).unwrap();
    let seed = program
        .functions
        .iter()
        .find(|function| function.name == "seed")
        .unwrap()
        .id;
    let probe = program
        .functions
        .iter()
        .find(|function| function.name == "probe")
        .unwrap()
        .id;
    let args = [
        (InlineValueType::Duration, u128::from(first)),
        (InlineValueType::Duration, u128::from(second)),
        (InlineValueType::Deadline, (-1_i128) as u128),
        (
            InlineValueType::Deadline,
            ((-1_i128) as u128) ^ (1_u128 << 64),
        ),
    ]
    .map(|(ty, bits)| {
        mir::Rvalue::Value(mir::ValueExpression::Inline(
            mir::InlineValueExpression::Use {
                ty,
                mode: mir::InlineValueUseMode::Borrow,
                operand: mir::Operand::Scalar(mir::ScalarValue::Inline(InlineValue { ty, bits })),
            },
        ))
    })
    .to_vec();
    let main = &mut program.functions[program.entry.0];
    let mut replaced = false;
    for block in &mut main.blocks {
        for statement in &mut block.statements {
            if let mir::Statement::CallVoid {
                function,
                args: inputs,
                ..
            } = statement
            {
                if *function == seed {
                    *function = probe;
                    *inputs = args.clone();
                    replaced = true;
                }
            }
        }
        if let mir::Terminator::CheckedCall {
            function,
            args: inputs,
            ..
        } = &mut block.terminator
        {
            if *function == seed {
                *function = probe;
                *inputs = args.clone();
                replaced = true;
            }
        }
    }
    assert!(replaced, "source seed call must remain explicit");
    doriac::mir_validation::validate_program(&program).unwrap();
    program
}

#[test]
fn inline_time_copy_transport_preserves_zero_and_full_width_in_all_native_profiles() {
    for (first, second) in [(0, u64::MAX), (u64::MAX, 0)] {
        let program = inline_copy_program(first, second);
        let output = doriac::mir_interpreter::interpret(&program).unwrap();
        assert_eq!(output.stdout, b"copy preserved\n", "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        assert_eq!(output.exit_status, 0);
        native_execution::assert_native_execution(
            &program,
            NativeProfile::Fast,
            "copy preserved\n",
        );
        #[cfg(feature = "llvm-backend")]
        native_execution::assert_native_execution(
            &program,
            NativeProfile::Release,
            "copy preserved\n",
        );
        assert!(
            program
                .functions
                .iter()
                .filter_map(|function| function.method.as_ref())
                .all(|method| {
                    InlineValueType::for_name(&program.classes[method.class.0].name).is_none()
                        || !matches!(method.name.as_str(), "__construct" | "__destruct")
                }),
            "inline values must not gain heap-class constructor or drop functions"
        );
        for class in &program.classes {
            if InlineValueType::for_name(&class.name).is_some() {
                assert!(class.constructor.is_none() && class.destructor.is_none());
            }
        }
        for collection in &program.collection_types {
            if let mir::Type::Scalar(mir::ScalarType::Inline(ty)) = collection.value {
                assert_eq!(
                    doriac::native_abi::collection_value_width(collection.value, 8),
                    Some(ty.storage_bytes() as u8)
                );
            }
        }
    }
}

#[test]
fn inline_time_identity_is_reserved_and_never_an_implicit_numeric_conversion() {
    for name in ["Duration", "Deadline"] {
        for declaration in [
            format!("class {name} {{}}"),
            format!("interface {name} {{}}"),
            format!("trait {name} {{}}"),
            format!("enum {name} {{ case Value; }}"),
        ] {
            let errors = doriac::check_source(
                "reserved-time.doria",
                format!("namespace Doria\\Std\\Time; {declaration}"),
            )
            .unwrap_err();
            assert!(
                errors.iter().any(|error| error.code == "E0640"),
                "{errors:?}"
            );
        }
        assert!(doriac::check_source(
            "time-prelude.doria",
            format!("function inspect({name} $value): void {{}}")
        )
        .is_err());
        assert!(doriac::check_source(
            "internal-time.doria",
            format!("function main(): void {{ let $value = new Doria\\Std\\Time\\{name}(); }}")
        )
        .is_err());
    }
    for body in [
        "Duration $value = 0;",
        "uint64 $value = $duration;",
        "Deadline $value = $duration;",
        "Duration $value = $deadline;",
        "SharedReference<Duration> $value = shared new Duration();",
    ] {
        let source = format!("use Doria\\Std\\Time\\{{Duration, Deadline}}; function invalid(Duration $duration, Deadline $deadline): void {{ {body} }}");
        assert!(
            doriac::check_source("time-identity.doria", source).is_err(),
            "{body}"
        );
    }
}

#[test]
fn ordinary_user_classes_with_time_basenames_remain_move_types() {
    for name in ["Duration", "Deadline"] {
        let source = format!("class {name} {{}} function invalid(take {name} $value): {name} {{ let $copy = $value; return $value; }}");
        assert!(doriac::check_source("user-time-name.doria", source).is_err());
    }
}

#[test]
fn php_deadline_copy_transport_preserves_both_private_words_without_ffi() {
    let mut artifact =
        doriac::compile_source("inline-copy.doria", COPY_SOURCE, BackendTarget::Php).unwrap();
    let (doriac::backend::BackendOutput::Text { contents, .. }
    | doriac::backend::BackendOutput::TextWithRuntime { contents, .. }) = &mut artifact
    else {
        panic!("PHP must emit source");
    };
    let symbol = |name: &str| {
        name.bytes()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    // Public factories are deliberately not invented here. Reflection supplies
    // opaque test-only values through the generated private carrier constructor.
    contents.push_str(&format!(
        r#"
$make = static function (string $type, array $words): mixed {{
    $class = new ReflectionClass($type);
    $value = $class->newInstanceWithoutConstructor();
    $class->getConstructor()->invokeArgs($value, $words);
    return $value;
}};
__DoriaFunction_{}(
    $make("__DoriaQualified_{}", [0]),
    $make("__DoriaQualified_{}", [-1]),
    $make("__DoriaQualified_{}", [-1, -1]),
    $make("__DoriaQualified_{}", [-1, -2])
);
"#,
        symbol("probe"),
        symbol(InlineValueType::Duration.name()),
        symbol(InlineValueType::Duration.name()),
        symbol(InlineValueType::Deadline.name()),
        symbol(InlineValueType::Deadline.name())
    ));
    let directory = native_execution::ExecutionDirectory::new();
    let path = directory.path().join("main.php");
    doriac::backend::publish_output(&path, artifact).unwrap();
    let output = std::process::Command::new("php")
        .args(["-d", "ffi.enable=0"])
        .arg(path)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, b"copy preserved\n");
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn mir_rejects_inline_constants_outside_their_nominal_width() {
    let mut program = inline_copy_program(0, u64::MAX);
    let main = &mut program.functions[program.entry.0];
    let mut replaced = false;
    for block in &mut main.blocks {
        for statement in &mut block.statements {
            if let mir::Statement::CallVoid { args, .. } = statement {
                if let Some(mir::Rvalue::Value(mir::ValueExpression::Inline(
                    mir::InlineValueExpression::Use {
                        operand: mir::Operand::Scalar(mir::ScalarValue::Inline(value)),
                        ..
                    },
                ))) = args.first_mut()
                {
                    value.bits = 1_u128 << 64;
                    replaced = true;
                }
            }
        }
    }
    assert!(replaced);
    assert!(doriac::mir_validation::validate_program(&program)
        .unwrap_err()
        .message
        .contains("nominal carrier width"));
}

#[test]
fn mir_rejects_inline_identity_changes_and_heap_class_carriers() {
    let valid = inline_copy_program(0, u64::MAX);
    for ty in [InlineValueType::Deadline, InlineValueType::Duration] {
        let mut invalid = valid.clone();
        let probe = invalid
            .functions
            .iter_mut()
            .find(|function| function.name == "probe")
            .unwrap();
        let local = probe
            .locals
            .iter_mut()
            .find(|local| local.ty == mir::Type::Scalar(mir::ScalarType::Inline(ty)))
            .unwrap();
        if ty == InlineValueType::Duration {
            local.ty = mir::Type::Scalar(mir::ScalarType::Inline(InlineValueType::Deadline));
        } else {
            let class = invalid
                .classes
                .iter()
                .find(|class| class.name == ty.name())
                .unwrap()
                .id;
            local.ty = mir::Type::Class(class);
        }
        assert!(doriac::mir_validation::validate_program(&invalid).is_err());
    }
}

#[test]
fn mir_rejects_every_heap_carrier_for_inline_time_values() {
    let valid = doriac::lower_source_to_mir(
        "inline-heap-carriers.doria",
        r#"
use Doria\Std\Time\{Duration, Deadline};
function unused(Duration $duration, Deadline $deadline): void {}
function main(): void {}
"#,
    )
    .unwrap();
    for ty in [InlineValueType::Duration, InlineValueType::Deadline] {
        let class = valid
            .classes
            .iter()
            .find(|class| class.name == ty.name())
            .unwrap()
            .id;
        let payload = mir::SharedPayload::Class(class);
        for carrier in [
            mir::Type::Class(class),
            mir::Type::NullableClass(class),
            mir::Type::SharedReference(payload),
            mir::Type::WeakReference(payload),
            mir::Type::NullableSharedReference(payload),
            mir::Type::NullableWeakReference(payload),
            mir::Type::WritableSharedReference(payload),
            mir::Type::WritableWeakReference(payload),
            mir::Type::NullableWritableSharedReference(payload),
            mir::Type::NullableWritableWeakReference(payload),
            mir::Type::ReadonlySharedReferenceAccess(payload),
            mir::Type::WritableSharedReferenceAccess(payload),
            mir::Type::NullableReadonlySharedReferenceAccess(payload),
            mir::Type::NullableWritableSharedReferenceAccess(payload),
        ] {
            let mut invalid = valid.clone();
            let unused = invalid
                .functions
                .iter_mut()
                .find(|function| function.name == "unused")
                .unwrap();
            unused
                .locals
                .iter_mut()
                .find(|local| local.ty == mir::Type::Scalar(mir::ScalarType::Inline(ty)))
                .unwrap()
                .ty = carrier;
            let error = doriac::mir_validation::validate_program(&invalid)
                .expect_err("inline Time values must not use heap-class carriers");
            assert!(
                error
                    .message
                    .contains("inline Copy value uses a heap-class carrier"),
                "{ty:?} as {carrier:?}: {error:?}"
            );
        }
    }
}
