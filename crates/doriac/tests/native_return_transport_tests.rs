#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;

use doriac::backend::NativeProfile;

/// Three-slot values (`?ByteRegion`, and `?Deadline`, whose 16-byte inline
/// payload takes two registers beside its presence word) exceed the shared
/// direct return budget, so every call shape returns them through the caller's
/// result slot. That keeps them within x86-64's two integer return registers;
/// AArch64 runs the same transport.
const WIDE_RETURNS: &str = r#"
use Doria\Std\Io\ByteRegion;
use Doria\Std\Time\{ClockError, Deadline, Duration};
class Packet {
    Bytes $bytes = Bytes::allocate(3);
    function optional(bool $present): ?ByteRegion {
        if ($present) { return $this->bytes->getRegion(1, 1); }
        return null;
    }
}
function optional(Bytes $source, bool $present): ?ByteRegion {
    if ($present) { return $source->getRegion(0, 2); }
    return null;
}
function later(Deadline $deadline): Deadline { return $deadline; }
function pick(Deadline $deadline, bool $present): ?Deadline {
    if ($present) { return $deadline; }
    return null;
}
function forward<T>(take T $value): T { return $value; }
function through(function(?ByteRegion): ?ByteRegion $call, ?ByteRegion $value): ?ByteRegion {
    return $call($value);
}
function main(): void throws ClockError {
    writable Bytes $bytes = Bytes::allocate(2);
    $bytes[0] = 4;
    $bytes[1] = 6;
    let $call = fn(?ByteRegion $value) => $value;
    let $present = through($call, optional($bytes, true));
    if ($present != null) { echo "{$present->length}\n"; echo "{$present[1]}\n"; }
    if (through($call, optional($bytes, false)) == null) { echo "absent\n"; }
    let $packet = new Packet();
    let $own = $packet->optional(true);
    if ($own != null) { echo "{$own->length}\n"; }
    if ($packet->optional(false) == null) { echo "no view\n"; }
    let $deadline = Deadline::fromNow(Duration::fromNanoseconds(5));
    let $kept = forward(pick($deadline, true));
    if ($kept != null && pick($deadline, false) == null) { echo "deadline\n"; }
    if (later($deadline) == $deadline) { echo "later\n"; }
}
"#;

const WIDE_RETURNS_OUTPUT: &str = "2\n6\nabsent\n1\nno view\ndeadline\nlater\n";

#[test]
fn wide_values_return_through_the_result_slot_on_every_backend() {
    let program = doriac::lower_source_to_mir("wide-returns.doria", WIDE_RETURNS)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(String::from_utf8_lossy(&output.stdout), WIDE_RETURNS_OUTPUT);
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, WIDE_RETURNS_OUTPUT);
    }
}

/// A virtual call reaches its family's return adapter. Here the family returns
/// a three-slot `?Deadline` through the result slot while the override returns
/// a two-slot `Deadline` in registers, so the call must follow the adapter's
/// carrier, including a call whose static callee is an override returning
/// `Deadline`. An open class returned through a function value keeps its
/// descriptor word.
const COVARIANT_RETURNS: &str = r#"
use Doria\Std\Time\{ClockError, Deadline, Duration};
open class Clock {
    open function next(Deadline $deadline): ?Deadline { return null; }
}
open class Ticking extends Clock {
    override function next(Deadline $deadline): Deadline { return $deadline; }
}
class Steady extends Ticking {
    override function next(Deadline $deadline): Deadline { echo "steady;"; return $deadline; }
}
open class Shape { open function sides(): int { return 0; } }
class Square extends Shape { override function sides(): int { return 4; } }
function poll(Clock $clock, Deadline $deadline): void {
    if ($clock->next($deadline) == null) { echo "none\n"; } else { echo "next\n"; }
}
// The static callee `Ticking::next` returns `Deadline` in registers, but a
// virtual call through it reaches the family adapter, which returns `?Deadline`
// through the slot.
function tick(Ticking $ticking, Deadline $deadline): void {
    if ($ticking->next($deadline) == $deadline) { echo "tick\n"; }
}
function make(function(): Shape $factory): Shape { return $factory(); }
function main(): void throws ClockError {
    let $deadline = Deadline::fromNow(Duration::fromNanoseconds(5));
    poll(new Clock(), $deadline);
    poll(new Ticking(), $deadline);
    let $ticking = new Ticking();
    tick($ticking, $deadline);
    tick(new Steady(), $deadline);
    let $shape = make(function (): Shape { return new Square(); });
    echo "{$shape->sides()}\n";
}
"#;

const COVARIANT_RETURNS_OUTPUT: &str = "none\nnext\ntick\nsteady;tick\n4\n";

#[test]
fn virtual_and_function_value_returns_follow_the_entry_they_reach() {
    let program = doriac::lower_source_to_mir("covariant-returns.doria", COVARIANT_RETURNS)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        COVARIANT_RETURNS_OUTPUT
    );
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, COVARIANT_RETURNS_OUTPUT);
    }
}

/// Open-class region returns. The family returns a three-word `?ByteRegion`
/// through the result slot; the overrides return a two-word `ByteRegion` in
/// registers, one from an inherited property and one from the override's own.
/// Each method's MIR plan retains only its own body's sources, the family's
/// call boundary retains every override's, and each return adapter retains
/// what its implementation retains.
const OPEN_CLASS_REGIONS: &str = r#"
use Doria\Std\Io\ByteRegion;
open class Source {
    Bytes $bytes = Bytes::allocate(2);
    open function view(): ?ByteRegion { return $this->bytes->getRegion(0, 1); }
}
open class Filled extends Source {
    override function view(): ByteRegion { return $this->bytes->getRegion(0, 2); }
}
class Extra extends Filled {
    Bytes $more = Bytes::allocate(3);
    override function view(): ByteRegion { return $this->more->getRegion(0, 3); }
}
function look(Source $source): void {
    let $view = $source->view();
    if ($view != null) { echo "{$view->length}\n"; }
}
function filled(Filled $filled): void { echo "f{$filled->view()->length}\n"; }
function main(): void {
    look(new Source());
    look(new Filled());
    look(new Extra());
    filled(new Filled());
    filled(new Extra());
}
"#;

const OPEN_CLASS_REGIONS_OUTPUT: &str = "1\n2\n3\nf2\nf3\n";

#[test]
fn open_class_region_returns_retain_each_implementations_sources() {
    let program = doriac::lower_source_to_mir("open-class-regions.doria", OPEN_CLASS_REGIONS)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        OPEN_CLASS_REGIONS_OUTPUT
    );
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, OPEN_CLASS_REGIONS_OUTPUT);
    }
}
