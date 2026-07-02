# 0023 Stage 5b native structured if blocks

Status: Accepted

Decision 0024 extends this native smoke subset with Stage 6a bounded `while` loops whose assignment-only bodies are proven to terminate within the current native smoke verification cap. This decision remains the accepted record for Stage 5b structured `if` blocks.

## Decision

Stage 5b extends the current Cranelift native smoke backend from single-return branch bodies to structured native blocks.

A supported native block contains:

- zero or more supported `int` local declarations or writable `int` local assignments
- one supported native terminator

Supported terminators are:

- `return <supported integer expression>;`
- terminal `if` / `else` where every branch is a supported native block
- terminal `if` / `else if` / `else` chains where every branch is a supported native block
- guard-style `if` without `else` followed by a supported fallback block

This keeps `if` as statement control flow. `if` without `else` is valid Doria; Stage 5b only lowers it to native code when it is a supported guard whose then-branch returns and whose fallback continuation is also a supported native block.

## Supported Shape

Stage 5b continues to accept exactly one top-level native entrypoint:

```doria
function main(): int
{
    let writable $code = 40;

    if ($code == 40) {
        $code += 2;

        return $code;
    }

    return 0;
}
```

The supported local forms inside any supported native block are:

```doria
let $code = 42;
int $code = 42;
let writable $code = 40;
writable int $code = 40;
```

The supported assignment forms inside any supported native block are:

```doria
$code = expr;
$code += expr;
$code -= expr;
```

The target must be a declared writable `int` local visible in that block.

## Branch Scoping

Branch-local declarations are scoped to the branch. They do not leak to fallback code or code outside the branch.

Native validation clones the local-state environment when validating branch blocks. Branch-local declarations are added only to the branch environment. Assignments to outer writable locals may update that branch environment, but Stage 5b performs no state merge after an `if`.

No merge is needed for the supported branch forms because supported branches terminate.

Guard-style fallback validation uses the original outer state. Mutations in the returning guard branch do not affect the fallback continuation.

## Backend Notes

Cranelift lowering may clone the current lowered local-value map for each supported returning branch. That is an implementation detail of this smoke backend slice. It does not define Doria locals as SSA values and does not commit Doria to a storage model.

Doria `int` remains signed 64-bit. The `0..125` restriction remains only the current portable process-exit boundary for values returned from `main()`. It is not the range of Doria `int` or local integer values.

## Non-goals

Stage 5b does not add:

- non-terminating `if` branch merging
- assignment merging after an `if`
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
