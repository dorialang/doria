# Benchmark-driven bug findings

> Documentation role: working note / bug handoff. Three reproducible defects and
> one feature gap surfaced while writing the cross-language benchmark suite
> (`languages/benchmarks/`, Stage 23). This is a to-do handoff for a fixing agent,
> not a decision record. Each item has a minimal repro, observed vs expected
> behavior, scope, and severity. Verify the repros before fixing; fix globally,
> add regression fixtures across the interpreter/Cranelift/LLVM parity manifest.

Toolchain at time of writing: `doriac` on develop with Stage 23 Slices 1–3
landed; native profiles Cranelift (fast) and LLVM (`--release`).

## Bug 1 — `bool`-typed collection/array element reads are miscompiled (B0001) — RESOLVED

**RESOLVED.** Root cause: the shared MIR `BoolExpression::Use` operand validation
omitted `mir::Operand::CollectionIndex` (present for the integer operand), so a
`bool` element load was rejected before any backend. Fixed by adding the
`CollectionIndex` arm to the bool operand validation (the interpreter and both
native backends already lowered it). Regression coverage:
`examples/native/main_stage23_bool_collections.doria` (parity manifest) plus a
`stage23_tests` test.

The bool operand match kept its catch-all arm after that fix, and the same gap
was still open for floats — see Bug 3, which closed it for both by making the
matches exhaustive. That `stage23_tests` case also only called
`lower_source_to_mir`, which does not run `validate_program`, so it did not
actually guard the fix until Bug 3 added the validation call.

**Severity: high.** Any `bool` element stored in a collection or array is unusable
in native code, so `bool` collections are effectively broken. This blocks the
natural sieve idiom (a `List<bool>`/`bool[]` flag buffer) and any boolean-payload
container.

**Minimal repro:**
```doria
function main(): void {
    writable List<bool> $b = [];
    $b->add(true);
    if ($b[0]) { echo "ok\n"; }
}
```
**Expected:** prints `ok`.
**Actual:** `Error[B0001]: backend emission failure: malformed MIR: bool expression has an incompatible operand`.

**Scope (verified across kinds):**

| form                                        | result |
|---------------------------------------------|--------|
| plain `bool` local in `if`                  | ok     |
| `List<int>` element `== 1` (control)        | ok     |
| `bool[]` element                            | B0001  |
| `List<bool>` element                        | B0001  |
| `Dictionary<int, bool>` value               | B0001  |
| `List<bool>` element read into a local, then used | B0001 |

The last row is the key diagnostic: it fails even when the element is bound to a
local *before* any boolean context, so the fault is in **reading a `bool`-typed
element out of a collection/array** (the element-load lowering for a `bool`
payload), not in the `if`/boolean-operator lowering. Non-`bool` payloads
(`int`/`string`/class elements) are fine. The error is raised at MIR emission, so
it affects every backend.

**Fix direction:** the collection/array element-load path produces a MIR operand
whose type/representation is rejected by the shared "bool expression operand"
validation — likely a `bool` element is loaded as a wider/!=`i1` value (or without
the expected bool representation). Make the `bool` element load produce the same
well-formed bool operand a plain `bool` local does. Add parity fixtures: `bool[]`,
`List<bool>`, and `Dictionary<K, bool>` element read → used in `if`, in a boolean
operator, and bound-to-local-then-used, across interpreter/Cranelift/LLVM.

## Bug 2 — `Int::parse` is unresolved (E0420) — RESOLVED

**RESOLVED.** `Int::parse(string): ?int` and `Float::parse(string): ?float` are
now wired end to end (resolution, typing, a `NullableScalarExpression::Parse` MIR
node, `doria-rt` `dr_v1_int_parse`/`dr_v1_float_parse`, and interpreter/Cranelift/
LLVM lowering). Grammar: surrounding ASCII whitespace is ignored, then Rust's
base-10 `i64` / `f64` parse; unparseable text and out-of-range integers yield
`null`. Fixed-width companions (`Int8::parse`, `Float32::parse`, …) are deferred
under a two-clocks diagnostic (parse with `Int`/`Float`, convert with `::from`).
Regression: `examples/native/main_stage23_int_parse.doria` (parity manifest) plus
a `stage23_tests` typing/deferral test.

**Severity: medium.** `Int::parse(string): ?int` is in the documented stdlib
surface (stdlib-reference; decision 0016 numeric companions) but is not wired into
name resolution / typing, so reading a number from input is impossible today. (It
is why the benchmarks use a literal `n` rather than reading it.)

**Minimal repro:**
```doria
function main(): void {
    let $line = read_line();
    if ($line != null) {
        let $n = Int::parse($line);   // expected ?int
        if ($n != null) { echo "parsed\n"; }
    }
}
```
**Expected:** `Int::parse` returns `?int`.
**Actual:** the call yields type `Unknown`, so the follow-on `$n != null` fails
with `Error[E0420]: equality operands must have compatible types, got Unknown and null`.

**Fix direction:** wire the `Int::parse` companion (and check the sibling
`Float::parse`) through resolution, typing, MIR, and the runtime, returning the
nullable per the companion contract. Add fixtures for parse-success and
parse-failure (`null`) across the three backends.

## Bug 3 — `float`-typed collection element and static reads are miscompiled (B0001) — RESOLVED

**RESOLVED.** The same defect as Bug 1, one scalar family over: the shared MIR
`FloatExpression::Use` operand match accepted `Property`, `CollectionKeyAt`, and
`MixedPayload` but ended in a catch-all, so `mir::Operand::CollectionIndex` —
which the interpreter and both native backends already lowered — was reported as
malformed MIR. Reading a `float`/`float32` element out of any indexable
collection failed before any backend ran.

**Minimal repro:**
```doria
function main(): void {
    float[] $s = [2.5, 1.5];
    float $v = $s[0];
    echo "{$v}\n";
}
```
**Expected:** prints `2.5`.
**Actual:** `Error[B0001]: backend emission failure: malformed MIR: float expression has an incompatible operand`.

**Second gap, found by enumerating rather than patching the reported case:** the
same match also omitted `mir::Operand::Static`, so `Config::ratio` on a
`static float` property failed identically while the `bool` and `int` statics
beside it compiled. It was not in the bug report.

**Fix.** Both the float and bool operand surfaces became exhaustive
`validate_float_operand` / `validate_bool_operand` functions modelled on
`validate_integer_operand`, which never had a catch-all and never had this bug.
A future `mir::Operand` variant is now a compile error in all three, not a
runtime B0001. Each previously-unreachable variant reports what is actually
wrong (`collection length is used as float instead of int/int64`) instead of the
one generic message.

**Scope (every combination compiled and run on Cranelift and LLVM, 270 cases,
byte-identical output):** `bool`, `int8/16/32/64`, `uint8/16/32/64`, `float32`,
`float` × typed array, `List`, `Dictionary` value, `SortedDictionary` value ×
bound-to-local, interpolation, concat, arithmetic, comparison. `Deque` is not
indexable (E0520) and is read by iteration and `popFront`; `Set`, `SortedSet`,
and `PriorityQueue` refuse float elements by design (E0523 — floats are not
`Hashable`, and NaN/signed zero deny the total order). Both were verified
separately and pass.

Regression coverage: `examples/native/main_float_collection_elements.doria`
(parity manifest, with an `expected_stdout` fixture), restored `float32`/`float`
lines in `examples/native/main_collection_fill_widths.doria`, and
`mir_validation_tests` cases that run `validate_program` over the accepted
surface and over a deliberately mistyped element read.

## Resolved — runtime-sized sequence fill

Decision 0102 added `[value; count]`, so the sieve can now build a runtime-sized
`bool[]` or `List<bool>` directly (for example, `[true; n]`) instead of issuing
`n` individual `add` calls. This is the sole runtime-sized `T[]` constructor.
The separate `withCapacity` capacity hint remains deliberately deferred.

## Where the repros come from

`benchmarks/` (sibling checkout of the `doria` repo): `fib/`, `sieve/`,
`mandelbrot/`, each with a Doria source plus C/C++/Rust/Go/C#/Java/JS/PHP/Python
peers and a shared `bench.php` harness. The sieve's Doria source no longer
carries the Bug 1 workaround; it builds a `bool[]` directly through the
Decision 0102 fill literal.
