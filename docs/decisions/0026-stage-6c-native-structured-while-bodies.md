# 0026 Stage 6c native structured while bodies

Status: Accepted

Decision 0031 extends this native smoke subset with Stage 7b `break` and `continue` inside supported bounded/proven `while` loops. This decision remains the accepted record for Stage 6c structured `while` bodies.

## Decision

Stage 6c extends the Cranelift native smoke backend from assignment-only `while` bodies to structured, scoped `while` bodies.

This is a native backend support slice only. It does not change Doria's language-level `while` semantics, block scoping, readonly/writable rules, integer semantics, or future runtime overflow policy.

## Supported Shape

Stage 6c continues to accept exactly one top-level native entrypoint:

```doria
function main(): int
{
    let writable $total = 0;
    let writable $index = 0;

    while ($index < 6) {
        let $step = 7;

        if ($index < 3) {
            $total += $step;
        } else {
            $total += 7;
        }

        $index += 1;
    }

    return $total;
}
```

A supported native block contains:

- zero or more supported local declarations
- zero or more supported writable integer assignments
- zero or more supported bounded `while` statements
- zero or more supported fallthrough `if` statements
- one supported native terminator

Supported `while` bodies may contain:

- supported `int` local declarations
- supported assignments to visible writable `int` locals
- supported assignments to loop-body writable `int` locals
- supported fallthrough `if` statements

Empty branches in loop-body fallthrough `if` statements are supported and preserve the incoming visible local state for that branch.

`while` body locals are scoped to the loop body and are recreated for each iteration. They do not leak outside the loop body or overwrite outer locals with the same name.

## Verification

Every Stage 6c native loop must be proven by native validation before Cranelift lowering.

Validation must prove:

- the condition is in the supported boolean condition subset
- the body contains only supported local declarations, assignments, and fallthrough `if` statements
- every assignment target is a writable `int` local visible at that statement
- loop-body local declarations obey Doria block scoping
- branch-local declarations inside loop-body `if` statements do not leak
- visible outer locals have well-defined values after each iteration
- every executed iteration remains within the signed 64-bit Doria `int` range
- the loop terminates within the current native smoke verification cap

The Stage 6c verification cap remains:

```text
10_000 iterations
```

This cap is a backend support limit for the current native smoke slice. It is not Doria language semantics.

## Lowering

Accepted loops must lower to real Cranelift control flow. They must not be compiled by replacing the whole program with a precomputed constant return.

Accepted loop bodies lower as structured statement blocks. Fallthrough `if` statements inside loop bodies lower to real branch and merge blocks. Visible loop-carried locals are passed through Cranelift block parameters.

The implementation may carry all visible native locals through loop headers and merge blocks rather than only changed locals. That is an implementation detail and does not define Doria's future local storage model.

Doria `int` remains signed 64-bit. The current `0..125` boundary remains only the portable process-exit range for values returned from `main()`. It is not the range of Doria `int` or local integer values.

## Non-goals

Stage 6c does not add:

- full native control-flow graph support
- nested `while` loops
- `return` inside `while` bodies
- `return` inside fallthrough branch bodies
- `break` or `continue`
- `foreach`
- division, modulo, unary minus syntax, or bitwise operators
- bool locals or native bool return values
- function calls, methods, static calls, strings, interpolation, echo/stdout, classes, objects, collections, FFI, runtime errors, panic machinery, LLVM backend support, Baton, or explicit fixed-width numeric type implementation

Unsupported backend coverage must remain an unsupported native backend diagnostic, not a claim that otherwise valid Doria is invalid.
