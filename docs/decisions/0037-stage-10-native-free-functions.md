# 0037 Stage 10 native free functions

Status: Accepted

## Decision

Stage 10 adds MVP native support for top-level free functions and calls.

Supported Stage 10 native function forms include:

```doria
function add(int $left, int $right): int
{
    return $left + $right;
}

function printHello(): void
{
    echo "Hello";
}

function main(): int
{
    return add(20, 22);
}

function main(): void
{
    printHello();
}
```

The native Stage 10 support boundary is:

- top-level free functions only
- function names must be unique
- exactly one top-level `main` entrypoint remains required
- `main` may return `int` or `void`
- non-`main` functions may return `int` or `void`
- supported native parameter types are `int` for Stage 10
- supported native return types are `int` and `void` for Stage 10
- `void` functions may fall through or use `return;`
- `int` functions must return `int` on all currently accepted paths
- calls may be used as statements when the callee returns `void`
- calls may be used in integer expressions when the callee returns `int`
- calls pass positional arguments only in Stage 10
- argument count and type checking are required

A Doria helper function returning `int` returns a Doria integer value. That value is not a process status. The portable `0..125` process-status boundary applies only when the exported process entrypoint observes the result of `main(): int`.

## Native backend boundary

The native smoke backend may lower supported functions directly with Cranelift function declarations and calls. Supported Doria `int` parameters and helper returns lower through native smoke as signed 64-bit values. The exported process entrypoint remains a backend wrapper around Doria `main`, returning an `I32` process status.

This is not a final Doria ABI, final MIR, public calling convention, or externally observable data layout. The Stage 10 native function model is implementation-private to the current native smoke backend.

## Non-goals

Stage 10 does not add:

- recursion
- mutual recursion
- function overloading
- default arguments in native
- named arguments in native
- generic functions
- string parameters in native
- string return values in native
- float, bool, object, collection, `mixed`, or resource parameters in native
- method calls in native
- static calls in native
- object construction in native
- function-local static variables
- closures
- nested functions
- generators
- async functions
- external declarations
- FFI
- stable native ABI
- full MIR
- LLVM backend work
- Baton work

Unsupported backend coverage must remain a clear unsupported native backend diagnostic. It must not be described as invalid Doria when the broader language already accepts the construct.

## Assumption

The implementation may use private local Cranelift symbols for supported Doria helper functions and an exported process `main` wrapper for the executable entrypoint. This is an implementation-private native smoke detail, not a stable Doria ABI or public calling convention.

## Stage 11 retirement pointer

This record remains the authority for the Stage 10 source subset and helper-value/process-status distinction. Its private native smoke implementation was retired by decision 0043 at Stage 11h. The interpreter and Cranelift now consume the same MIR, while internal function symbols and the exported process wrapper remain implementation-private rather than stable ABI.
