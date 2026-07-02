# 0025 Stage 6b native if fallthrough merges

Status: Accepted

## Decision

Stage 6b extends the Cranelift native smoke backend with supported fallthrough `if` statements before later native statements.

This is a native backend support slice only. It does not change Doria's language-level `if` semantics: `if` remains statement control flow, `else` remains optional, and `when` remains the future value-returning conditional construct.

## Supported Shape

Stage 6b continues to accept exactly one top-level native entrypoint:

```doria
function main(): int
{
    let writable $code = 40;

    if ($code == 40) {
        $code += 2;
    }

    return $code;
}
```

A supported native block contains:

- zero or more supported local declarations
- zero or more supported writable integer assignments
- zero or more supported bounded `while` statements from Stage 6a
- zero or more supported fallthrough `if` statements
- one supported native terminator

Supported fallthrough branch bodies may contain:

- supported `int` local declarations
- supported assignments to visible writable `int` locals
- supported bounded `while` statements
- nested supported fallthrough `if` statements

Branch bodies used as fallthrough statements do not contain `return`. Existing terminating `if` / `else` and guard-return shapes remain terminators.

## Local State Merging

Stage 6b merges visible local values after supported fallthrough branches.

Only locals that were visible before the `if` are merged back into the containing block. Branch-local declarations do not leak outside the branch.

If there is no `else`, the missing branch is treated as an empty branch that preserves the pre-if local state.

For validation and smoke execution, the current compiler still evaluates the supported subset at compile time and updates the containing validation state with the branch selected by the evaluated condition. This is a backend support limit for the current native smoke slice; it is not a new Doria language semantic.

## Lowering

Accepted fallthrough `if` statements lower to real Cranelift control flow. Both branches jump to a merge block, and visible merged locals are passed through Cranelift block parameters.

The implementation may merge all visible native locals rather than only changed locals. That is an implementation detail and does not define Doria's future local storage model.

Doria `int` remains signed 64-bit. The current `0..125` boundary remains only the portable process-exit range for values returned from `main()`. It is not the range of Doria `int` or local integer values.

## Non-goals

Stage 6b does not add:

- full native control-flow graph support
- `return` inside fallthrough branch bodies
- `if` inside `while` bodies
- declarations inside `while` bodies
- general native loop bodies
- `break` or `continue`
- `foreach`
- division, modulo, unary minus syntax, or bitwise operators
- bool locals or native bool return values
- function calls, methods, static calls, strings, interpolation, echo/stdout, classes, objects, collections, FFI, runtime errors, panic machinery, LLVM backend support, Baton, or explicit fixed-width numeric type implementation

Unsupported backend coverage must remain an unsupported native backend diagnostic, not a claim that otherwise valid Doria is invalid.
