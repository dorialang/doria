# 0021 Stage 4b native boolean conditions

Status: Accepted

Decisions 0022 and 0023 extend the native smoke subset with writable integer locals, writable integer assignments, and structured returning `if` blocks. This decision remains the accepted record for Stage 4b boolean condition support.

## Decision

Stage 4b extends the current Cranelift native smoke backend so supported `if` conditions can use the accepted Doria boolean operators from decision 0020:

```doria
!
not

&&
and

||
or

xor
```

This is a native backend implementation slice only. It does not change Doria language semantics. Doria conditions already require `bool`; this slice teaches the narrow native backend to lower more of those valid conditions.

## Supported Native Shape

Stage 4b continues the Stage 4a entrypoint and statement-shape limits:

```doria
function main(): int
{
    let $left = 20;
    let $right = 22;

    if (($left + $right == 42) and not false) {
        return 42;
    }

    return 0;
}
```

The native backend still accepts exactly one top-level `function main(): int` with zero or more supported readonly integer local declarations followed by one of:

- a final supported return
- a terminal `if` / `else` whose branch bodies each contain exactly one supported return
- a guard-style `if` followed by a fallback return

## Supported Conditions

Stage 4b supports these condition forms in the accepted native `if` shapes:

- `true` and `false`
- grouped conditions
- signed Doria `int` comparisons over supported integer expressions
- `!` and `not`
- `&&` and `and`
- `||` and `or`
- `xor`

`&&` / `and` and `||` / `or` must lower with short-circuit behavior. `xor` evaluates both operands and does not short-circuit.

## Boundaries

This slice does not add:

- bool locals
- bool return values
- boolean expression lowering outside supported `if` conditions
- bitwise operators
- `given`, `finally`, or `when`
- nested `if`
- `else if`
- branch-local declarations
- `while`, `foreach`, `break`, or `continue`
- writable locals or assignments
- function calls, methods, strings, classes, objects, collections, runtime support, or FFI

The `0..125` process-exit boundary remains only the current portable native smoke-test boundary for values returned from `main()`. It is not a limit on Doria `int`.

## Backend Notes

The implementation should lower native conditions as control flow, not as PHP-like truthiness. `and` / `or` lowering must branch around the right operand when short-circuiting. `xor` may lower by evaluating both operands to bool values and comparing them for inequality.

Cranelift implements Doria semantics here; it does not define them. Later LLVM support for the same slice must preserve the same Doria-visible behavior.
