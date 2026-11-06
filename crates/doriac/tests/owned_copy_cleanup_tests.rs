#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;

use doriac::backend::{BackendTarget, NativeProfile};
use doriac::mir::{self, PayloadEnumUseMode, Rvalue, Statement, Terminator};

const SOURCE: &str = r#"
use Doria\Std\Concurrency\CancellationError;
enum Packet { case Value(string $message); }
enum Wrapped { case Value(?Packet $packet); }
function create(string $prefix): Packet { return Packet::Value($prefix . " claim"); }
function forward(Packet $input): Packet {
    try { return $input; } finally { echo "forward\n"; }
}
function nullable(?Wrapped $input): ?Wrapped {
    try { return $input; } finally { echo "nullable\n"; }
}
function abandoned(Packet $input): Packet throws CancellationError {
    try { return $input; } finally { throw new CancellationError("replacement"); }
}
function abandonedNullable(?Wrapped $input): ?Wrapped throws CancellationError {
    try { return $input; } finally { throw new CancellationError("nullable replacement"); }
}
function text(Packet $packet): string {
    return match ($packet) { Packet::Value($message) => $message, };
}
function main(): void {
    let $original = create("owned");
    let $copy = forward($original);
    echo text($original) . "|" . text($copy) . "\n";
    let $wrapped = Wrapped::Value($original);
    let $again = nullable($wrapped);
    if ($again != null) {
        let $inner = match ($again) { Wrapped::Value($packet) => $packet, };
        if ($inner != null) { echo text($inner) . "\n"; }
    }
    let $absent = nullable(null);
    if ($absent == null) { echo "absent\n"; }
    try { abandoned($original); }
    catch (CancellationError $error) { echo $error->message . "\n"; }
    try { abandonedNullable($wrapped); }
    catch (CancellationError $error) { echo $error->message . "\n"; }
    try { abandonedNullable(null); }
    catch (CancellationError $error) { echo $error->message . "\n"; }
    echo text($original) . "\n";
}
"#;

#[test]
fn conditional_copy_results_preserve_cleanup_across_backends() {
    assert_execution(
        include_str!("fixtures/inline_copy_when_results.doria"),
        "17|20|9\n",
    );
}

#[test]
fn statement_when_results_preserve_cleanup_across_backends() {
    assert_execution(
        include_str!("fixtures/statement_when_results.doria"),
        "23|0|2,f,1,\n7|3,o,\n3,o,7,\nowned claim\n7|t,4,\nreplaced|a,8,\nouter|i,r,6,\n",
    );
}

#[test]
fn statement_when_reused_results_survive_predicate_errors_across_backends() {
    assert_execution(
        include_str!("fixtures/statement_when_reuse.doria"),
        "5|u,0,u,c,5,\n",
    );
}

#[test]
fn conditional_copy_loop_exits_preserve_cleanup_across_backends() {
    assert_execution(
        include_str!("fixtures/inline_copy_when_loop_exits.doria"),
        "35\n",
    );
}

#[test]
fn trivial_copy_branch_results_leave_temporary_storage_as_independent_values() {
    assert_execution(
        r#"
enum Packet { case Number(int $count); case Empty; }
function selected(bool $first): Packet {
    return when ($first): Packet { return Packet::Number(11); }
        else { return Packet::Empty; };
}
function mapped(Packet $input): Packet {
    return match ($input) { Packet::Number($number) => Packet::Number(12), Packet::Empty => Packet::Empty, };
}
function optional(bool $present): ?Packet {
    return when ($present): ?Packet { return Packet::Number(0); }
        else { return null; };
}
function coalesced(bool $present): Packet { return optional($present) ?? Packet::Number(13); }
function count(Packet $value): int {
    return match ($value) { Packet::Number($number) => $number, Packet::Empty => -1, };
}
function main(): void {
    echo count(selected(true)) . "|" . count(mapped(Packet::Number(1))) . "|"
        . count(coalesced(false)) . "|" . count(coalesced(true)) . "\n";
}
"#,
        "11|12|13|0\n",
    );
}

#[test]
fn owned_copy_finalizer_returns_and_replacements_preserve_payloads_across_backends() {
    let expected = "forward\nowned claim|owned claim\nnullable\nowned claim\nnullable\nabsent\nreplacement\nnullable replacement\nnullable replacement\nowned claim\n";
    assert_execution(SOURCE, expected);
}

fn assert_execution(source: &str, expected: &str) {
    let program = doriac::lower_source_to_mir("owned-copy-finalizer.doria", source).unwrap();
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(output.stdout, expected.as_bytes());
    assert_eq!(output.exit_status, 0);
    assert!(output.stderr.is_empty());
    native_execution::assert_native_execution(&program, NativeProfile::Fast, expected);
    #[cfg(feature = "llvm-backend")]
    native_execution::assert_native_execution(&program, NativeProfile::Release, expected);
    let artifact =
        doriac::compile_source("owned-copy-finalizer.doria", source, BackendTarget::Php).unwrap();
    let directory = native_execution::ExecutionDirectory::new();
    let path = directory.path().join("main.php");
    doriac::backend::publish_output(&path, artifact).unwrap();
    let output = std::process::Command::new("php")
        .arg(path)
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty(), "{output:?}");
}

#[test]
fn grouped_copy_values_preserve_independent_payload_claims() {
    assert_execution(
        r#"
use Doria\Std\Time\Duration;
enum Packet { case Value(string $message); }
enum Wrapped { case Value(?Packet $packet); }
function text(Packet $packet): string {
    return match ($packet) { Packet::Value($message) => $message, };
}
function main(): void {
    let $packet = Packet::Value("owned" . " claim");
    {
        Packet $first, $second = $packet;
        echo text($first) . "|" . text($second) . "\n";
        ?Wrapped $left, $right = Wrapped::Value($first);
        if ($left != null && $right != null) { echo "present\n"; }
        ?Packet $absent, $alsoAbsent = null;
        if ($absent == null && $alsoAbsent == null) { echo "absent\n"; }
        Duration $duration, $same = Duration::fromNanoseconds(-17);
        ?Duration $maybe, $again = $duration;
        echo "{$duration->nanoseconds}|{$same->nanoseconds}|{($maybe ?? $same)->nanoseconds}|{($again ?? $duration)->nanoseconds}\n";
    }
    echo text($packet) . "\n";
}
"#,
        "owned claim|owned claim\npresent\nabsent\n-17|-17|-17|-17\nowned claim\n",
    );
}

#[test]
fn public_iteration_keeps_captured_copy_values_in_distinct_storage() {
    assert_execution(
        r#"
function inspect(Iterable<int> $numbers, Iterable<string> $words): void {
    foreach ($numbers as int $value) {
        let $read = fn() with ($value) => $value;
        echo "{$read()}|{$read()}\n";
        if ($value == 3) { continue; }
        if ($value == 7) { break; }
    }
    foreach ($words as string $value) {
        let $read = fn() with ($value) => $value;
        echo "{$read()}|{$read()}\n";
    }
}
function main(): void {
    List<int> $numbers = [3, 7, 11];
    List<string> $words = ["first", "second"];
    inspect($numbers, $words);
    echo "{$numbers[2]}|{$words[0]}\n";
}
"#,
        "3|3\n7|7\nfirst|first\nsecond|second\n11|first\n",
    );
}

#[test]
fn taking_copy_parameters_preserve_the_callers_value_across_call_paths() {
    assert_execution(
        r#"
use Doria\Std\Concurrency\CancellationError;
enum Packet { case Value(string $message); }
function text(Packet $packet): string {
    return match ($packet) { Packet::Value($message) => $message, };
}

function forward(take Packet $packet): Packet { return $packet; }
function identity<T>(take T $value): T { return $value; }
function fail(take Packet $packet): void throws CancellationError {
    echo text($packet) . "\n";
    throw new CancellationError("failed");
}
function apply(function(take Packet): Packet $callback, Packet $packet): Packet {
    return $callback($packet);
}
interface Forwarder {
    function forward(take Packet $packet): Packet;
    function fail(take Packet $packet): void throws CancellationError;
}
class Relay implements Forwarder {
    function forward(take Packet $packet): Packet { return $packet; }
    function fail(take Packet $packet): void throws CancellationError {
        echo text($packet) . "\n";
        throw new CancellationError("interface failed");
    }
}
function relay(Forwarder $forwarder, Packet $packet): Packet {
    return $forwarder->forward($packet);
}
function failThrough(Forwarder $forwarder, Packet $packet): void throws CancellationError {
    $forwarder->fail($packet);
}
function forwardNullable(take ?Packet $packet): ?Packet { return $packet; }
function main(): void {
    let $packet = Packet::Value("owned" . " claim");
    let $copy = forward($packet);
    let $callback = function(take Packet $input): Packet { return $input; };
    let $again = apply($callback, $packet);
    echo text($copy) . "|" . text($again) . "|" . text($packet) . "\n";
    try { fail($packet); }
    catch (CancellationError $error) { echo $error->message . "\n"; }
    let $forwarder = new Relay();
    let $relayed = relay($forwarder, $packet);
    echo text($relayed) . "\n";
    try { failThrough($forwarder, $packet); }
    catch (CancellationError $error) { echo $error->message . "\n"; }
    let $present = forwardNullable($packet);
    if ($present != null) { echo text($present) . "\n"; }
    let $absent = forwardNullable(null);
    if ($absent == null) { echo "absent\n"; }
    let $generic = identity($packet);
    echo text($generic) . "|" . text($packet) . "\n";
    echo text($packet) . "\n";
}
"#,
        "owned claim|owned claim|owned claim\nowned claim\nfailed\nowned claim\nowned claim\ninterface failed\nowned claim\nabsent\nowned claim|owned claim\nowned claim\n",
    );
}

#[test]
fn inline_copy_collection_fills_conversions_and_nullable_slots_preserve_values() {
    assert_execution(
        r#"
use Doria\Std\Time\Duration;
function main(): void {
    let $tick = Duration::fromNanoseconds(-17);
    Duration[] $filled = [$tick; 3];
    List<Duration> $empty = [$tick; 0];
    writable Deque<Duration> $deque = Deque::from($filled);
    $deque->pushBack($tick);
    let $front = $deque->popFront();
    echo "{$filled->length}|{$empty->count}|{$deque->count}\n";
    if ($front != null) { echo "{$front->nanoseconds}\n"; }
    Dictionary<int, ?Duration> $source = [1 => $tick, 2 => null];
    let $sorted = SortedDictionary::from($source);
    let $present = $sorted->get(1);
    let $absent = $sorted->get(2);
    if ($present != null) { echo "{$present->nanoseconds}\n"; }
    if ($absent == null) { echo "absent\n"; }
    echo "{$filled[2]->nanoseconds}|{$tick->nanoseconds}\n";
}
"#,
        "3|0|3\n-17\n-17\nabsent\n-17|-17\n",
    );
}

#[test]
fn wide_inline_and_nullable_scalar_fills_preserve_the_complete_carrier() {
    assert_execution(
        r#"
use Doria\Std\Time\{ClockError, Deadline, Duration};
enum Color { case Red; case Green; }
function main(): void throws ClockError {
    let $deadline = Deadline::fromNow(Duration::fromNanoseconds(-17));
    Deadline[] $filled = [$deadline; 3];
    let $deque = Deque::from($filled);
    if ($filled[2] == $deadline && $deque->peekFront == $deadline) { echo "wide\n"; }
    ?Deadline $present = $deadline;
    List<?Deadline> $optional = [$present; 2];
    ?Deadline $absent = null;
    List<?Deadline> $empty = [$absent; 0];
    List<?Deadline> $missing = [$absent; 2];
    Dictionary<int, ?Deadline> $source = [2 => null, 1 => $present];
    let $sorted = SortedDictionary::from($source);
    if ($optional[1] == $present && $sorted->get(1) == $present) { echo "nullable wide\n"; }
    if ($missing[1] == null && $empty->count == 0 && $sorted->get(2) == null) { echo "absent\n"; }
    ?int $number = -42;
    List<?int> $numbers = [$number; 2];
    ?bool $flag = null;
    List<?bool> $flags = [$flag; 2];
    Color[] $colors = [Color::Green; 2];
    if (($numbers[1] ?? 0) == -42 && $flags[1] == null && $colors[1] == Color::Green) { echo "scalars\n"; }
}
"#,
        "wide\nnullable wide\nabsent\nscalars\n",
    );
}

#[test]
fn copy_enum_fills_preserve_nested_claims_and_nullable_absence() {
    assert_execution(
        r#"
enum Packet { case Value(string $message); }
enum Wrapped { case Value(?Packet $packet); }
function text(Wrapped $wrapper): string {
    let $packet = match ($wrapper) { Wrapped::Value($value) => $value, };
    if ($packet != null) {
        return match ($packet) { Packet::Value($message) => $message, };
    }
    return "absent";
}
function main(): void {
    let $original = Wrapped::Value(Packet::Value("owned" . " claim"));
    {
        Wrapped[] $filled = [$original; 3];
        List<Wrapped> $empty = [$original; 0];
        ?Wrapped $present = $original;
        List<?Wrapped> $optional = [$present; 2];
        ?Wrapped $absent = null;
        List<?Wrapped> $missing = [$absent; 2];
        let $copy = Deque::from($filled);
        foreach ($copy as Wrapped $item) { echo text($item) . "\n"; }
        let $observed = $optional[1];
        if ($observed != null) { echo text($observed) . "\n"; }
        if ($missing[1] == null && $empty->count == 0) { echo "absent\n"; }
    }
    echo text($original) . "\n";
}
"#,
        "owned claim\nowned claim\nowned claim\nowned claim\nabsent\nowned claim\n",
    );
}

#[test]
fn nullable_copy_enum_reads_require_the_branch_presence_proof() {
    for condition in [
        "$input != null",
        "null != $input",
        "!($input == null)",
        "$input is Packet",
    ] {
        let source = format!(
            r#"
enum Packet {{ case Value(int $number); }}
function read(Packet $input): int {{
    return match ($input) {{ Packet::Value($number) => $number, }};
}}
function probe(?Packet $input, bool $gate): int {{
    if ({condition}) {{
        return read($input);
    }}
    return 0;
}}
function main(): void {{}}
"#
        );
        let mut program = doriac::lower_source_to_mir("enum-presence.doria", source).unwrap();
        let probe = program
            .functions
            .iter_mut()
            .find(|function| function.name == "probe")
            .unwrap();
        let gate = probe
            .locals
            .iter()
            .find(|local| local.name == "gate")
            .unwrap()
            .id;
        let condition = probe
            .blocks
            .iter_mut()
            .find_map(|block| match &mut block.terminator {
                Terminator::Branch { condition, .. } => Some(condition),
                _ => None,
            })
            .unwrap();
        *condition = mir::BoolExpression::Use {
            operand: mir::Operand::Local(gate),
        };
        let error = doriac::mir_validation::validate_program(&program)
            .expect_err(&format!("missing presence proof was accepted:\n{program}"));
        assert!(
            error.message.contains("dominating presence proof"),
            "{error:?}"
        );
    }
}

const ONCE_CAPTURE_SOURCE: &str = r#"
use Doria\Std\Concurrency\CancellationError;
use Doria\Std\Io\IoError;
enum Packet { case Value(string $message); }
enum Wrapped { case Value(?Packet $packet); }
class Guard {
    function __construct(int $id) {}
    function __destruct() {
        try { echo "drop {$this->id}\n"; }
        catch (IoError) {}
    }
}
function text(Packet $packet): string {
    return match ($packet) { Packet::Value($message) => $message, };
}
function main(): void {
    let $packet = Packet::Value("owned" . " claim");
    let $wrapped = Wrapped::Value($packet);
    ?Wrapped $absent = null;
    {
        let $guard = new Guard(1);
        let $consume = function(): Guard with (take $guard, take $packet, take $wrapped, take $absent) {
            echo text($packet) . "\n";
            let $inner = match ($wrapped) { Wrapped::Value($value) => $value, };
            if ($inner != null) { echo text($inner) . "\n"; }
            if ($absent == null) { echo "absent\n"; }
            return $guard;
        };
        let $result = $consume();
        echo "success {$result->id}\n";
    }
    {
        let $guard = new Guard(2);
        let $fail = function(): Guard with (take $guard, take $packet) {
            echo text($packet) . "\n";
            if (true) { throw new CancellationError("failed"); }
            return $guard;
        };
        try { let $result = $fail(); }
        catch (CancellationError $error) { echo $error->message . "\n"; }
    }
    {
        let $guard = new Guard(3);
        let $uncalled = function(): Guard with (take $guard, take $packet) {
            echo text($packet) . "\n";
            return $guard;
        };
    }
    let $read = function(): string with (take $packet) { return text($packet); };
    if ($read() != $read() || $read() != text($packet)) { panic("capture changed"); }
    echo $read() . "|" . $read() . "|" . text($packet) . "\n";
}
"#;

#[test]
fn consuming_copy_captures_have_one_local_cleanup_owner() {
    let mut program =
        doriac::lower_source_to_mir("once-copy-capture.doria", ONCE_CAPTURE_SOURCE).unwrap();
    let mut checked = 0;
    for function in &program.functions {
        let Some(closure) = &function.closure else {
            continue;
        };
        let invocation = program.function_types[closure.function_type.0].invocation_mode;
        for (_, capture) in &closure.capture_locals {
            let local = &function.locals[capture.0];
            if !matches!(
                local.ty,
                mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_)
            ) {
                continue;
            }
            assert_eq!(local.owned, invocation == mir::FunctionInvocationMode::Once);
            checked += 1;
        }
    }
    assert_eq!(checked, 6);
    let function = program
        .functions
        .iter_mut()
        .find(|function| {
            function.closure.as_ref().is_some_and(|closure| {
                program.function_types[closure.function_type.0].invocation_mode
                    == mir::FunctionInvocationMode::Once
            })
        })
        .unwrap();
    let capture = function
        .closure
        .as_ref()
        .unwrap()
        .capture_locals
        .iter()
        .find(|(_, local)| matches!(function.locals[local.0].ty, mir::Type::PayloadEnum(_)))
        .unwrap()
        .1;
    function.locals[capture.0].owned = false;
    let error = doriac::mir_validation::validate_program(&program).unwrap_err();
    assert!(
        error.message.contains("closure environment field binding"),
        "{error:?}"
    );
}

#[test]
fn consuming_copy_captures_preserve_values_and_cleanup_on_all_exits() {
    assert_execution(
        ONCE_CAPTURE_SOURCE,
        "owned claim\nowned claim\nabsent\nsuccess 1\ndrop 1\nowned claim\ndrop 2\nfailed\ndrop 3\nowned claim|owned claim|owned claim\n",
    );
}

#[test]
fn owned_copy_branch_and_yield_results_keep_their_claims_through_finalizers() {
    assert_execution(
        r#"
use Doria\Std\Concurrency\CancellationError;
enum Packet { case Value(string $message); }
function create(string $prefix): Packet { return Packet::Value($prefix . " claim"); }
function selected(bool $first, Packet $input): Packet {
    return match ($first) { true => $input, false => create("second"), };
}
function yielded(bool $first, Packet $input): ?Packet {
    return when ($first): ?Packet { return $input; }
        else { return null; } finally { echo "yield\n"; };
}
function replacedYield(Packet $input): ?Packet throws CancellationError {
    return when (true): ?Packet { return $input; }
        else { return null; }
        finally { throw new CancellationError("yield replaced"); };
}
function text(Packet $packet): string {
    return match ($packet) { Packet::Value($message) => $message, };
}
function main(): void {
    let $original = create("owned");
    let $selected = selected(true, $original);
    let $second = selected(false, $original);
    echo text($selected) . "|" . text($second) . "\n";
    let $value = yielded(true, $original);
    if ($value != null) { echo text($value) . "\n"; }
    let $absent = yielded(false, $original);
    if ($absent == null) { echo "absent\n"; }
    try { replacedYield($original); }
    catch (CancellationError $error) { echo $error->message . "\n"; }
    echo text($original) . "\n";
}
"#,
        "owned claim|second claim\nyield\nowned claim\nyield\nabsent\nyield replaced\nowned claim\n",
    );
}

#[test]
fn pending_copy_claims_have_one_handoff_or_one_replacement_drop() {
    let program = doriac::lower_source_to_mir("owned-copy-structure.doria", SOURCE).unwrap();
    let mut handoffs = 0;
    let mut replacements = 0;
    for function in &program.functions {
        for block in &function.blocks {
            if let Terminator::Return(value) = &block.terminator {
                if handoff(value).is_some() {
                    let local = value.transferred_owned_local().unwrap();
                    assert!(function.locals[local.0].owned);
                    assert!(function.locals[local.0].synthetic);
                    handoffs += 1;
                }
            }
            for statement in &block.statements {
                let Statement::ControlFlowPlan(mir::ControlFlowPlan::Finalizer(plan)) = statement
                else {
                    continue;
                };
                for replacement in &plan.replacements {
                    for case in &replacement.cases {
                        let Some(local) = case.dropped_payload else {
                            continue;
                        };
                        if !matches!(
                            function.locals[local.0].ty,
                            mir::Type::PayloadEnum(_) | mir::Type::NullablePayloadEnum(_)
                        ) {
                            continue;
                        }
                        assert!(function.locals[local.0].owned);
                        let drops = function.blocks[case.entry.0].statements.iter().filter(|statement| {
                            matches!(statement, Statement::DropPayloadEnum { local: dropped, .. } if *dropped == local)
                        }).count();
                        assert_eq!(drops, 1);
                        replacements += 1;
                    }
                }
            }
        }
    }
    assert!(handoffs >= 2);
    assert!(replacements >= 2);
}

fn handoff(value: &Rvalue) -> Option<mir::LocalId> {
    match value {
        Rvalue::PayloadEnum(mir::PayloadEnumExpression::Use {
            place: mir::PayloadEnumPlace::Local(local),
            mode: PayloadEnumUseMode::TransferOwnedCopy,
            ..
        })
        | Rvalue::NullablePayloadEnum(mir::NullablePayloadEnumExpression::Use {
            place: mir::PayloadEnumPlace::Local(local),
            mode: PayloadEnumUseMode::TransferOwnedCopy,
            ..
        }) => Some(*local),
        _ => None,
    }
}

#[test]
fn validator_rejects_owned_copy_handoffs_from_authored_or_borrowed_locals() {
    let valid = doriac::lower_source_to_mir("owned-copy-proof.doria", SOURCE).unwrap();
    for invalidate_ownership in [false, true] {
        let mut program = valid.clone();
        let function = program
            .functions
            .iter_mut()
            .find(|function| function.name == "forward")
            .unwrap();
        let local = function
            .blocks
            .iter()
            .find_map(|block| match &block.terminator {
                Terminator::Return(value) => handoff(value),
                _ => None,
            })
            .unwrap();
        if invalidate_ownership {
            function.locals[local.0].owned = false;
        } else {
            function.locals[local.0].synthetic = false;
        }
        let error = doriac::codegen_cranelift::lower_mir_to_object(&program).unwrap_err();
        assert!(
            format!("{error:?}").contains("owned Copy handoff")
                || format!("{error:?}").contains("drop ownership"),
            "{error:?}"
        );
    }
}
