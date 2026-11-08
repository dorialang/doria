#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution.rs"]
mod native_execution;

use doriac::backend::{BackendTarget, NativeProfile};

/// The PHP backend executes byte regions through its own runtime helpers.
fn assert_php_execution(name: &str, source: &str, expected: &str) {
    let directory = execution_directory::ExecutionDirectory::new();
    let artifact = doriac::compile_source(name, source, BackendTarget::Php)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let path = directory.path().join("main.php");
    doriac::backend::publish_output(&path, artifact).unwrap();
    let output = std::process::Command::new("php")
        .current_dir(directory.path())
        .arg(path)
        .output()
        .expect("PHP is required for region requirement parity");
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), expected);
    assert!(output.stderr.is_empty(), "{output:?}");
}

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

/// Decision 0134: a bodyless requirement with a receiver elides its region
/// return to a retained loan of that receiver. An implementation may return a
/// region it holds, or one that is independent of the receiver, which keeps
/// more than the requirement asks for.
const REGION_REQUIREMENTS: &str = r#"
use Doria\Std\Io\{ByteRegion, WritableByteRegion};
interface Viewing {
    function view(): ?ByteRegion;
}
interface Exact {
    function exact(): ByteRegion;
    ByteRegion $whole { get; }
}
interface Editing {
    writable function edit(): WritableByteRegion;
    WritableByteRegion $slot { writable get; }
}
class Held implements Viewing, Exact, Editing {
    Bytes $bytes = Bytes::allocate(3);
    writable Bytes $edits = Bytes::allocate(4);
    function view(): ?ByteRegion { return $this->bytes->getRegion(0, 2); }
    function exact(): ByteRegion { return $this->bytes->getRegion(1, 2); }
    writable function edit(): WritableByteRegion { return $this->edits->getWritableRegion(0, 4); }
    WritableByteRegion $slot { writable get => $this->edits->getWritableRegion(1, 2); }
    ByteRegion $whole { get => $this->bytes->getRegion(0, 3); }
}
// Narrows `?ByteRegion` to `ByteRegion`, which returns in two registers
// instead of through the result slot.
class Narrow implements Viewing, Exact, Editing {
    Bytes $bytes = Bytes::allocate(5);
    writable Bytes $edits = Bytes::allocate(1);
    function view(): ByteRegion { return $this->bytes->getRegion(0, 5); }
    function exact(): ByteRegion { return $this->bytes->getRegion(0, 4); }
    writable function edit(): WritableByteRegion { return $this->edits->getWritableRegion(0, 1); }
    WritableByteRegion $slot { writable get => $this->edits->getWritableRegion(0, 1); }
    ByteRegion $whole { get => $this->bytes->getRegion(0, 5); }
}
class Absent implements Viewing {
    function view(): ?ByteRegion { return null; }
}
// The region comes from storage the caller lends, not from this object.
class Lent implements Viewing, Exact {
    function __construct(borrow Bytes $store) {}
    function view(): ?ByteRegion { return $this->store->getRegion(0, 1); }
    function exact(): ByteRegion { return $this->store->getRegion(0, 2); }
    ByteRegion $whole { get => $this->store->getRegion(0, 4); }
}
trait Exposing {
    function exposed(): ?ByteRegion;
    function viaTrait(): ?ByteRegion { return $this->exposed(); }
    function viaTraitLength(): int { return $this->exposed()?->length ?? -1; }
}
class TraitHeld {
    uses Exposing;
    Bytes $bytes = Bytes::allocate(6);
    function exposed(): ?ByteRegion { return $this->bytes->getRegion(0, 6); }
}
class TraitAbsent {
    uses Exposing;
    function exposed(): ?ByteRegion { return null; }
}
function report(?ByteRegion $view): void {
    if ($view != null) { echo "{$view->length}\n"; } else { echo "none\n"; }
}
function look(Viewing $viewing): void {
    let $view = $viewing->view();
    report($view);
}
function measure(Exact $exact): void {
    echo "{$exact->exact()->length}\n";
    echo "{$exact->whole->length}\n";
}
function forward(Viewing $viewing): ?ByteRegion { return $viewing->view(); }
function constrained<T implements Viewing>(T $value): ?ByteRegion { return $value->view(); }
function constrainedExact<T implements Exact>(T $value): int {
    let $view = $value->exact();
    return $view->length + $value->whole->length;
}
function fill(writable Editing $editing): void {
    let writable $region = $editing->edit();
    $region[0] = 9;
    echo "{$region->length}\n";
    let writable $slot = $editing->slot;
    $slot[0] = 8;
    echo "{$slot->length}\n";
}
function fillConstrained<T implements Editing>(writable T $value): void {
    let writable $region = $value->edit();
    $region[0] = 7;
    echo "{$region->length}\n";
}
function main(): void {
    let $store = Bytes::allocate(4);
    let $held = new Held();
    let $narrow = new Narrow();
    let $absent = new Absent();
    let $lent = new Lent($store);
    look($held);
    look($narrow);
    look($absent);
    look($lent);
    measure($held);
    measure($narrow);
    measure($lent);
    report(forward($held));
    report(forward($absent));
    report(constrained($held));
    report(constrained($narrow));
    report(constrained($absent));
    report(constrained($lent));
    echo constrainedExact($held) . "\n";
    echo constrainedExact($lent) . "\n";
    writable Editing $editing = new Held();
    fill($editing);
    writable Narrow $target = new Narrow();
    fillConstrained($target);
    writable Held $direct = new Held();
    fillConstrained($direct);
    let $traitHeld = new TraitHeld();
    report($traitHeld->viaTrait());
    echo "{$traitHeld->viaTraitLength()}\n";
    let $traitAbsent = new TraitAbsent();
    report($traitAbsent->viaTrait());
    echo "{$traitAbsent->viaTraitLength()}\n";
}
"#;

const REGION_REQUIREMENTS_OUTPUT: &str = "\
2\n5\nnone\n1\n\
2\n3\n4\n5\n2\n4\n\
2\nnone\n\
2\n5\nnone\n1\n\
5\n6\n\
4\n2\n1\n4\n\
6\n6\nnone\n-1\n";

#[test]
fn region_requirements_elide_to_a_retained_receiver_loan_on_every_backend() {
    let program = doriac::lower_source_to_mir("region-requirements.doria", REGION_REQUIREMENTS)
        .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        REGION_REQUIREMENTS_OUTPUT
    );
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(&program, profile, REGION_REQUIREMENTS_OUTPUT);
    }
    assert_php_execution(
        "region-requirements.doria",
        REGION_REQUIREMENTS,
        REGION_REQUIREMENTS_OUTPUT,
    );
}

/// The same contract through the other shapes a requirement reaches: checked
/// errors narrowed per specialization, generic interfaces, inherited and
/// composed implementations, interface inheritance and null-safe calls.
const REGION_REQUIREMENT_SHAPES: &str = r#"
use Doria\Std\Io\ByteRegion;
class Failure implements Error {
    function __construct(string $message) {}
}
interface Fallible {
    function read(bool $fail): ?ByteRegion throws Failure;
}
class Failing implements Fallible {
    Bytes $bytes = Bytes::allocate(2);
    function read(bool $fail): ?ByteRegion throws Failure {
        if ($fail) { throw new Failure("failed"); }
        return $this->bytes->getRegion(0, 2);
    }
}
class Infallible implements Fallible {
    Bytes $bytes = Bytes::allocate(3);
    function read(bool $fail): ByteRegion { return $this->bytes->getRegion(0, 3); }
}
function erased(Fallible $source, bool $fail): ?ByteRegion throws Failure { return $source->read($fail); }
function selected<T implements Fallible>(T $source): int throws Failure {
    return $source->read(false)?->length ?? -1;
}
interface Producing<T> {
    function make(): T;
}
class ByteProducer implements Producing<ByteRegion> {
    Bytes $bytes = Bytes::allocate(4);
    function make(): ByteRegion { return $this->bytes->getRegion(0, 4); }
}
class IntProducer implements Producing<int> {
    function make(): int { return 7; }
}
function produced(Producing<ByteRegion> $producer): ByteRegion { return $producer->make(); }
function producedOwned(take Producing<int> $producer): int { return $producer->make(); }
function forwardProduced<U>(Producing<U> $producer): U { return $producer->make(); }
function forwardErased(Producing<ByteRegion> $producer): ByteRegion { return forwardProduced($producer); }
function selectedProduct<P implements Producing<ByteRegion>>(P $producer): ByteRegion { return $producer->make(); }
interface Tagged { function tag<T>(T $label): ByteRegion; }
class Tagging implements Tagged {
    Bytes $bytes = Bytes::allocate(9);
    function tag<T>(T $label): ByteRegion { return $this->bytes->getRegion(0, 9); }
}
function tagged(Tagged $tagged): ByteRegion { return $tagged->tag(1); }
interface Viewing { function view(): ?ByteRegion; }
interface Sized extends Viewing { function size(): int; }
open class Base {
    Bytes $bytes = Bytes::allocate(5);
    function view(): ?ByteRegion { return $this->bytes->getRegion(0, 5); }
}
class Derived extends Base implements Sized {
    function size(): int { return 5; }
}
open class Opening implements Viewing {
    Bytes $bytes = Bytes::allocate(6);
    open function view(): ?ByteRegion { return $this->bytes->getRegion(0, 1); }
}
class Opened extends Opening {
    override function view(): ByteRegion { return $this->bytes->getRegion(0, 6); }
}
trait Provides {
    Bytes $held = Bytes::allocate(7);
    function view(): ?ByteRegion { return $this->held->getRegion(0, 7); }
}
class Composed implements Viewing {
    uses Provides;
}
class Boxed<T> implements Viewing {
    Bytes $bytes = Bytes::allocate(8);
    function view(): ?ByteRegion { return $this->bytes->getRegion(0, 8); }
}
function report(?ByteRegion $view): void {
    if ($view != null) { echo "{$view->length}\n"; } else { echo "none\n"; }
}
function viaParent(Sized $sized): ?ByteRegion { return $sized->view(); }
function maybe(?Viewing $viewing): ?ByteRegion { return $viewing?->view(); }
function constrainedView<T implements Viewing>(T $value): ?ByteRegion { return $value->view(); }
function consume(take Viewing $viewing): void { echo "consumed\n"; }
function main(): void throws Failure {
    let $failing = new Failing();
    let $infallible = new Infallible();
    report(erased($failing, false));
    report(erased($infallible, false));
    echo selected($failing) . "\n";
    echo selected($infallible) . "\n";
    try {
        report(erased($failing, true));
    } catch (Failure $error) {
        echo "caught\n";
    }
    let $producer = new ByteProducer();
    echo "{produced($producer)->length}\n";
    let $forwarded = forwardErased($producer);
    echo "{$forwarded->length}\n";
    let $selected = selectedProduct($producer);
    echo "{$selected->length}\n";
    echo producedOwned(new IntProducer()) . "\n";
    echo "{tagged(new Tagging())->length}\n";
    let $derived = new Derived();
    report(viaParent($derived));
    report(constrainedView($derived));
    let $opening = new Opening();
    let $opened = new Opened();
    report(maybe($opening));
    report(maybe($opened));
    report(maybe(null));
    let $composed = new Composed();
    report(maybe($composed));
    report(constrainedView($composed));
    let $boxed = new Boxed<int>();
    report(maybe($boxed));
    report(constrainedView($boxed));
    // A readonly region's loan ends at its last use, so the receiver can move.
    Viewing $erased = new Composed();
    let $view = $erased->view();
    report($view);
    consume($erased);
}
"#;

const REGION_REQUIREMENT_SHAPES_OUTPUT: &str = "\
2\n3\n2\n3\ncaught\n\
4\n4\n4\n7\n9\n\
5\n5\n\
1\n6\nnone\n\
7\n7\n\
8\n8\n\
7\nconsumed\n";

#[test]
fn region_requirements_reach_every_interface_and_generic_shape() {
    let program =
        doriac::lower_source_to_mir("region-requirement-shapes.doria", REGION_REQUIREMENT_SHAPES)
            .unwrap_or_else(|errors| panic!("{errors:#?}"));
    let output = doriac::mir_interpreter::interpret(&program).unwrap();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        REGION_REQUIREMENT_SHAPES_OUTPUT
    );
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
    for &profile in NativeProfile::AVAILABLE {
        native_execution::assert_native_execution(
            &program,
            profile,
            REGION_REQUIREMENT_SHAPES_OUTPUT,
        );
    }
    assert_php_execution(
        "region-requirement-shapes.doria",
        REGION_REQUIREMENT_SHAPES,
        REGION_REQUIREMENT_SHAPES_OUTPUT,
    );
}
