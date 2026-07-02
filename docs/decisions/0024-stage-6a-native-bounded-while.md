# 0024 Stage 6a native bounded while

Status: Accepted

## Decision

Stage 6a extends the Cranelift native smoke backend with bounded, compile-time-verifiable `while` loops before an existing native terminator.

This is a native backend support slice only. It does not change Doria's language-level `while` semantics, readonly/writable rules, integer semantics, or future runtime overflow policy.

## Supported Shape

Stage 6a continues to accept exactly one top-level native entrypoint:

```doria
function main(): int
{
    let writable $code = 0;

    while ($code < 42) {
        $code += 1;
    }

    return $code;
}
```

A supported native block contains:

- zero or more supported local declarations
- zero or more supported writable integer assignments
- zero or more supported bounded `while` statements
- one supported native terminator

Supported `while` bodies contain one or more assignments to visible writable `int` locals:

```doria
while ($code < 42) {
    $code += 1;
}
```

The loop body does not yet support declarations, nested `if`, nested `while`, `return`, calls, strings, objects, collections, `break`, or `continue`.

## Verification

Every Stage 6a native loop must be proven by native validation before Cranelift lowering.

Validation must prove:

- the condition is in the supported boolean condition subset
- the body contains only supported assignments
- every assignment target is a visible writable `int` local
- every iteration remains within the signed 64-bit Doria `int` range
- the loop terminates within the current native smoke verification cap

The Stage 6a verification cap is:

```text
10_000 iterations
```

This cap is a backend support limit for the current native smoke slice. It is not Doria language semantics.

## Lowering

Accepted loops must lower to real Cranelift control flow. They must not be compiled by replacing the whole program with a precomputed constant return.

The implementation may use Cranelift block parameters for loop-carried writable integer locals. That is an implementation detail and does not define Doria's future local storage model.

Doria `int` remains signed 64-bit. The current `0..125` boundary remains only the portable process-exit range for values returned from `main()`. It is not the range of Doria `int` or local integer values.

## Non-goals

Stage 6a does not add:

- general native loop support
- declarations inside `while` bodies
- nested control flow inside `while` bodies
- `return` inside `while` bodies
- `break` or `continue`
- `foreach`
- post-if state merging
- division, modulo, unary minus syntax, or bitwise operators
- bool locals or native bool return values
- function calls, methods, static calls, strings, interpolation, echo/stdout, classes, objects, collections, FFI, runtime errors, panic machinery, LLVM backend support, Baton, or explicit fixed-width numeric type implementation

Unsupported backend coverage must remain an unsupported native backend diagnostic, not a claim that otherwise valid Doria is invalid.
