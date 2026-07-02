# 0022 Stage 5a native writable integer locals

Status: Accepted

Decision 0023 extends this native smoke subset with Stage 5b structured `if` blocks that can contain supported local declarations and writable integer assignments before returning. This decision remains the accepted record for Stage 5a writable integer locals.

## Decision

Stage 5a extends the current Cranelift native smoke backend with direct-body writable `int` locals and direct-body integer assignments before the existing native terminator shapes.

This is a narrow native backend implementation slice. It does not change Doria's language-level readonly/writable semantics:

- locals remain readonly by default
- `writable` is still required for reassignment
- readonly assignment remains a semantic error
- `int` means signed 64-bit `int64`

## Supported Native Shape

Stage 5a continues to accept exactly one top-level native entrypoint:

```doria
function main(): int
{
    let writable $code = 40;

    $code += 2;

    return $code;
}
```

The accepted body shape is:

```text
zero or more supported direct-body pre-terminator statements
then one supported native terminator
```

Supported pre-terminator statements are:

- readonly `int` local declarations
- writable `int` local declarations
- direct `=` assignment to writable `int` locals
- direct `+=` assignment to writable `int` locals
- direct `-=` assignment to writable `int` locals

The terminator remains one of:

- a final supported return
- a terminal `if` / `else` whose branch bodies each contain exactly one supported return
- a guard-style `if` followed by a fallback return

## Supported Assignments

Supported declaration forms include:

```doria
let $code = 42;
int $code = 42;
let writable $code = 42;
writable int $code = 42;
```

Supported assignment forms are:

```doria
$code = <supported integer expression>;
$code += <supported integer expression>;
$code -= <supported integer expression>;
```

The target must be a declared writable `int` local. Assignment expressions are limited to the existing supported integer expression subset:

- integer literals
- supported integer locals
- grouped integer expressions
- `+`, `-`, and `*` over supported integer expressions

Division, modulo, unary minus syntax, bitwise operators, increment/decrement, and compound assignments beyond `+=` and `-=` remain unsupported in this native slice.

## Range And Overflow

Doria `int` remains signed 64-bit. Stage 5a supports writable locals that hold the full accepted Doria `int` range:

```doria
function main(): int
{
    let writable $large = 9223372036854775807;
    $large = 0;

    return $large;
}
```

The current native process-exit boundary remains:

```text
0..125
```

That boundary applies only to values returned from `main()` as the current portable native smoke-test process exit code. It is not the range of Doria `int`.

Checked arithmetic is required for `+`, `-`, `*`, `+=`, and `-=` in this slice. Overflow must be diagnosed before Cranelift lowering. Doria must not inherit wrapping behavior from Rust, Cranelift, LLVM, C, PHP, or the host platform.

## Backend Notes

The implementation may track local names as current Cranelift SSA values in source order for this straight-line, compile-time-evaluable slice. Updating that implementation-private map for assignment does not define Doria mutable locals as SSA and does not commit Doria to a public local storage model.

No stack slots, heap allocation, runtime variable storage, or native-oriented MIR are required for this slice.

## Non-goals

Stage 5a does not add:

- assignments inside `if` / `else` branches
- branch-local declarations
- nested `if`
- `else if`
- `while` or `foreach`
- `break` or `continue`
- division or modulo
- unary minus syntax
- bitwise operators
- bool locals
- native bool return values
- logical operators outside supported `if` conditions
- function calls, methods, static calls, strings, interpolation, echo/stdout, classes, objects, collections, FFI, runtime errors, panic machinery, LLVM backend support, Baton, or explicit fixed-width numeric type implementation

Unsupported backend coverage must remain an unsupported native backend diagnostic, not a claim that otherwise valid Doria is invalid.
