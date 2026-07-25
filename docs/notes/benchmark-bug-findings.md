# Benchmark-driven bug findings

> Documentation role: working note / bug handoff. Two reproducible defects and one
> feature gap surfaced while writing the cross-language benchmark suite
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
`stage23_tests` lowering test.

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

## Not a bug — noted so it isn't "fixed"

**No sized/fill array constructor** (`int[n]`, `List::filled(n, v)`,
`withCapacity`) exists, so the sieve builds its buffer with `n` individual `add`
calls. This is a **deliberate deferral** — decision 0100 parks capacity/fill
constructors as a profiling-driven addition, not a launch feature. Treat it as a
*feature request* (a fill constructor would materially help array/buffer
workloads, and the benchmark shows the cost), **not** a regression to fix
silently. If pursued, it wants its own decision/amendment, not an ad-hoc method.

## Where the repros come from

`languages/benchmarks/` (sibling of the `doria` repo): `fib/`, `sieve/`,
`mandelbrot/`, each with a Doria source plus C/C++/Rust/C#/Java/JS/PHP/Python
peers and a shared `bench.py`. The sieve's Doria source documents the Bug 1
workaround (`List<int>` with `== 1`) inline.
