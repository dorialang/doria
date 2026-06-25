# 0018 Stage 2d native returned integer expressions

Status: Accepted

Accepted by the Stage 2d implementation task. Keep this slice narrow; later native stages must still be accepted separately.

## Decision

Stage 2d adds native support for final returned integer expressions inside the accepted native entrypoint:

```doria
function main(): int
{
    let $left = 20;
    let $right = 22;

    return $left + $right;
}
```

```doria
function main(): int
{
    return 20 + 22;
}
```

Supported returned expression forms are:

```text
- integer literals
- supported readonly integer locals
- binary expressions using +, -, and *
- parenthesized/grouped forms already parsed as expressions
```

Stage 2d continues to support Stage 2c readonly local initializer arithmetic:

```doria
function main(): int
{
    let $code = 20 + 22;
    return $code;
}
```

## Rationale

Stage 2d proves returned integer expressions without widening native output into general expression lowering, control flow, assignments, calls, strings, or runtime support.

The Stage 2d implementation was allowed to evaluate supported integer expressions at compile time for validation and smoke output. That was not a general Doria `const` feature, not a compile-time execution engine, and not permission to evaluate arbitrary calls or side effects.

Stage 3a preserves this accepted source subset while changing the backend architecture: the native backend validates into a small implementation-private native expression model, evaluates supported expressions only for checked arithmetic and process-exit range validation, then lowers the retained integer expression tree into Cranelift `i64` values before reducing the final validated result to the platform `main` return type. This is still not a public native IR, full native arithmetic support, or broader runtime support.

## Overflow and range

Doria `int` is signed 64-bit for early native integer semantics.

Compile-time overflow in supported integer arithmetic is a semantic diagnostic before Doria IR/native lowering.

The process exit-code range remains:

```text
0..125
```

That range applies only to the value returned from `main()` as the current portable native smoke-test process exit code. It is not the range of Doria `int` or local integer values.

Therefore:

```doria
function main(): int
{
    let $value = 100 + 26;
    return 0;
}
```

is valid Stage 2d native output, while:

```doria
function main(): int
{
    return 100 + 26;
}
```

is rejected by the native backend until a broader process-exit mapping decision exists.

## Division and modulo

Integer division and modulo are not part of Stage 2d.

They need an explicit Doria semantics decision before native output supports them, including behavior for division by zero, modulo by zero, negative operands, rounding or truncation direction, and overflow edge cases.

The native backend must not silently inherit division or modulo behavior from Rust, Cranelift, LLVM, C, PHP, or the host platform.

## Non-goals

This decision does not:

- support division or modulo
- support unary minus syntax
- support writable locals or assignments
- support compound assignments
- support runtime arithmetic over values not known in this narrow native slice
- support `if`, `while`, `foreach`, or other control flow in native output
- support function calls, method calls, static calls, object construction, strings, interpolation, collections, FFI, stdout, or runtime machinery
- define final process-exit behavior for all Doria integer values
- define a general constant evaluation feature
- define a Doria local storage model
- implement LLVM backend support
- implement Baton
- implement explicit fixed-width numeric types beyond the accepted documentation
