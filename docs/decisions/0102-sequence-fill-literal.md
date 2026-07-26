# Decision 0102: Sequence fill literal `[value; count]`

**Status:** Accepted (design direction; Copy-scalar and `string` elements in v1.0,
move-type fills deferred to `Cloneable`). Amends decision 0100's fill deferral and
plan §4.9's bracket-literal forms. Un-parks the *fill* half of 0100's deferred
constructors; the *capacity hint* (`withCapacity`) stays parked.

## Context

Creating a runtime-sized, pre-filled sequence has no spelling today. The
motivating case is a flag buffer — a sieve marks composites in a `bool[]`/
`List<bool>` of size `n` known only at run time. Two facts force the gap:

- **`T[]` can only be literal-constructed.** A typed array is
  fixed-length-after-creation with its length chosen at creation (§4.9), and the
  only constructor is a bracket literal whose elements are written out at compile
  time. So a **runtime-sized `T[]` is impossible** — the engine-grade buffer type
  cannot be used for the exact buffer workloads it exists for.
- **`List<T>` can only grow one element at a time.** The workaround is `n`
  individual `add` calls (0100), which is O(n) appends plus reallocations plus
  verbosity, and still cannot produce a `T[]`.

Decision 0100 parked "capacity/fill constructors" together as a profiling-driven
addition. That was right for the *capacity hint* (a pure performance knob) but
conflated it with *fill*, which is a capability gap, not an optimization. This
record separates them and adds the fill form.

## Decision

### The repeat literal

A bracket literal gains a second form, `[value; count]`, producing a sequence of
`count` copies of `value`:

```doria
bool[] $sieve = [true; n];        // a fixed bool buffer of runtime length n
List<int> $counts = [0; n];       // a List<int> of n zeros
let $flags = [false; n];          // no expected type -> List<bool>
```

- **Contextually typed exactly like the existing element-list literal (§4.9):**
  `T[] $a = [v; n]` builds a typed array; `List<T> $l = [v; n]` builds a list;
  with no expected type, `[v; n]` defaults to `List<T>` (the growable
  PHP-intuitive reading), with `T` inferred from `value`.
- **Sequences only.** The repeat form is valid only where a sequence literal is
  (`T[]`, `List<T>`). It is rejected for `Set` (filling a set with duplicates
  yields one element — meaningless) and has no keyed analogue for `Dictionary`.
- **`count` is a runtime `int` expression**, evaluated once. A negative `count`
  **panics** (`fill count is negative`); a const-negative `count` is a compile
  error. `count == 0` yields an empty sequence. An allocation that cannot be
  satisfied panics like any other allocation failure.
- **`value` is evaluated once** and replicated `count` times, in source order
  (value before count).

### Which element types can be filled (v1.0)

Replicating a value `count` times requires copying it, so the element type must be
replicable without a `Cloneable` contract (which does not exist until Stage 35):

- **Copy scalars** (`bool`, `int` and the fixed-width integers, `float`/`float32`/
  `float64`) — a bit copy. This covers the motivating buffer workloads.
- **`string`** — immutable and reference-counted, so `count` copies are `count`
  retained handles to the same `DrStringV1`; sound and cheap.

Move-type elements — concrete classes, collections, `Bytes`, `mixed`, nullable
payloads — are **deferred**: a single-owner value cannot be replicated, and the
`[value; count]` form over such a type is rejected with a stage/record-named
diagnostic. When `Cloneable` lands (Stage 35), the fill form extends to
`Cloneable` element types by cloning `value` per slot; nothing here forecloses
that.

### Not in scope

- **`withCapacity(count)`** (an empty, pre-allocated sequence) remains parked as a
  0100 capacity hint — a pure performance knob, not a capability, and not needed
  for a filled buffer.
- A `List::filled` / `Array::filled` **static method**. Rejected in favor of the
  literal: the literal reuses §4.9's contextual typing to cover `T[]` and `List`
  with one spelling, whereas a method would be `List`-only (no `T[]` runtime
  sizing) and would introduce a constructor method where sequences use literals.

## Alternatives considered

- **`List::filled(int $count, T $value): List<T>` method.** Simpler (no grammar
  change) but does not give `T[]` a runtime-sized constructor — the actual gap —
  and splits sequence construction between literals and a method. Rejected.
- **A sized-array constructor `T[n]` (n zeroed slots).** Rejected as the primary
  form: it needs a per-type "default/zero value" concept and only zero-fills;
  `[value; count]` subsumes it (`[0; n]`, `[false; n]`) and fills with any value.
- **Deferring the whole thing (keep 0100's parking).** Rejected for the fill half:
  it is a capability gap for `T[]`, not a micro-optimization, and the buffer
  idiom it blocks is common (the benchmark sieve).
- **Allowing the repeat form for `Set`/`Dictionary`.** Rejected — duplicate fill
  is meaningless for a set and there is no sensible keyed fill.

## Consequences

- `T[]` becomes usable for runtime-sized buffers, the workload it exists for.
- `List` gains a one-shot filled construction, replacing `n` `add` calls.
- The bracket literal now has two forms — an element list `[a, b, c]` and a repeat
  `[value; count]` — disambiguated by the `;` separator; both are contextually
  typed identically.
- The Copy+`string` restriction is an honest, self-describing boundary that lifts
  cleanly when `Cloneable` arrives, not a special case.

## Sequencing

Small, additive, and independent of Stage 23a/23b. Implemented as the focused
collections slice **Stage 23c**, after the benchmark correctness bugs landed. It
touches the parser (the `;` repeat form), HIR/MIR (a fill construction node),
shared MIR validation, `doria-rt` (a sequence fill routine; `string` fill retains
per slot), and the interpreter/Cranelift/LLVM lowering, with parity fixtures for
`bool[]`/`int[]`/`List<bool>`/`string` fills across all three backends.

## Affected components

Lexer/parser (repeat-literal grammar), AST/HIR, semantic analysis (contextual
typing, element-type eligibility, const-negative-count diagnostic), HIR/MIR and
shared validation, `doria-rt` fill routines, the interpreter and both native
backends, plan §4.9 and decision 0100 (amended below), the stdlib reference, and
durable parity fixtures. SPEC is updated when the form is implemented.

## Invalidated elsewhere

- Decision 0100's line parking "capacity hints (`withCapacity`) and other
  performance-shaped constructors" — the *fill* constructor is un-parked here and
  spelled as the `[value; count]` literal; `withCapacity` alone stays parked.
- Plan §4.9's "Bracket literals are collection literals" bullet — extended to name
  the `[value; count]` repeat form alongside the element-list form.
- `docs/notes/benchmark-bug-findings.md`'s "No sized/fill array constructor" note —
  its fill half is now decided (this record); its capacity half stays parked.
- Any statement that a `T[]` can only be constructed from a compile-time element
  list.
