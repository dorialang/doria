# Decision 0045: Runtime Strings and Canonical Display Conversion

Status: Accepted

## Context

Earlier native stages could emit string literals and resolve a restricted set of readonly string expressions at compile time. Stage 16 makes `string` a runtime value shared by checked HIR, typed MIR, the interpreter, Cranelift, LLVM, and `doria-rt`.

## Decision

### Source model

`string` contains valid UTF-8 and is immutable. It is a Copy type at the Doria source level: copying preserves value semantics and is pointer-cheap. The native representation is an opaque, implementation-private, reference-counted immutable buffer and may change before 1.0. Doria exposes neither a `String`/borrowed-string split nor pointers or reference counts.

Every string operation uses an explicit byte length. Empty strings and embedded NUL bytes are valid; `strlen` and NUL termination are never used.

A writable string binding permits rebinding, not mutation:

```doria
let writable $message = "Hello";
$message = $message . " Doria!";
```

Functions may accept and return strings. The process entrypoint remains `main(): void` or `main(): int`; `main(): string` is invalid.

### Comparison

`==` and `!=` compare exact UTF-8 bytes. `<`, `<=`, `>`, and `>=` use unsigned byte-lexicographic ordering. No locale, collation, grapheme, or display conversion participates.

### Canonical display conversion

One conversion feeds `echo`, each operand of `.`, and the interpolation forms accepted by the current parser:

- `string`: itself
- signed integers: base-10 ASCII, with `-` only for negative values
- unsigned integers: base-10 ASCII
- `float32`: deterministic shortest-round-trip text from the actual binary32 value
- `float`/`float64`: deterministic shortest-round-trip text from the binary64 value
- `bool`: exactly `true` or `false`

Float special values are `NaN`, `Infinity`, `-Infinity`, `0`, and `-0`. Formatting is locale-independent. Interpreter and runtime use the pinned no-std `ryu` implementation for finite values.

The concatenation operator produces `string` and accepts display-convertible primitives only when at least one operand of that binary operation is statically `string`. Thus `"x=" . 1` and `1 . "x"` are valid, while `1 . 2` is invalid. Evaluation is left-to-right. Conversion is not implicit outside display contexts; `string $value = 42;` remains invalid.

Stage 16 intentionally deferred full arbitrary-expression interpolation and class display conversion. Stage 18, recorded in decision 0079, completes ordinary expression interpolation and adds the compiler-known nominal `Displayable` path without changing this decision's primitive conversion or runtime-string history. Native class execution still waits for Stages 19 and 20. `Bytes` remains Stage 23.

### Native representation and ownership

`doria-rt` stores a non-atomic reference count and explicit byte length in a private header followed by immutable bytes. Stage 16 has no concurrency and makes no cross-thread sharing guarantee.

- A non-parameter string local owns one reference.
- The incoming string argument is borrowed at the ABI boundary; the callee prologue retains it into its parameter local and releases that owned local on normal exit.
- A returned string transfers one owned reference.
- Evaluating a literal, local copy, call result, concatenation, or display conversion produces an owned temporary.
- Rebinding acquires the new value before releasing the old local value.
- Normal returns preserve the returned reference, then release owned string locals.
- Ordinary call argument temporaries are released after the call. A constructor
  argument promoted into a property transfers that value to the property and is
  released with the object instead.
- Panic is abort-only and does not unwind or run cleanup.

The runtime checks allocation size, allocation failure, and reference-count overflow/underflow. Fatal failures use the existing status-101 panic path.

Unix hosts use system `malloc`/`free`; Windows uses the process heap (`GetProcessHeap`, `HeapAlloc`, and `HeapFree`) without introducing CRT startup. The ABI uses versioned private `dr_v1_*` symbols for construction, retain/release, concatenation, comparison, data/length access, display conversion, and output.

## Consequences

Strings are real typed MIR values across locals, parameters, calls, returns, comparisons, panic messages, and output. The interpreter remains the semantic oracle. Cranelift fast and LLVM release profiles consume the same validated MIR and private runtime ABI. PHP compatibility must use explicit Doria display behavior and must report unsupported coverage instead of silently inheriting PHP coercion.
