# 0027 Stage 7a native smoke IR boundary

Status: Accepted

## Decision

Stage 7a extracts the current native smoke backend behind an implementation-private `NativeSmokeModule` boundary.

This is a compiler architecture slice only. It preserves the Stage 6c Doria source subset and all Stage 6c behavior. It does not add new language semantics, new native source support, new control-flow forms, or broader runtime behavior.

## Pipeline Boundary

The current native smoke backend is organized as:

```text
checked HIR
-> native smoke validation/lowering
-> NativeSmokeModule
-> compile-time smoke evaluation/proof
-> Cranelift lowering
-> object emission
-> host linker
```

`NativeSmokeModule` is private to the bootstrap compiler implementation. It is not the public Doria IR, final MIR, or a permanent native storage model.

The boundary exists to keep responsibilities separate:

- validation accepts only the supported Stage 6c native smoke subset
- smoke evaluation proves the accepted program behavior without depending on Cranelift
- Cranelift lowering consumes the native smoke module rather than raw checked HIR details
- object emission and host linking remain outside validation and evaluation

## Requirements

Stage 7a must preserve:

- Doria `int` as signed 64-bit
- the current `0..125` portable process-exit boundary for observable native smoke `main()` return values
- checked integer arithmetic for supported native smoke expressions and assignments
- Stage 6c block scoping and shadowing rules
- Stage 6c loop termination proof with the current smoke verification cap
- real Cranelift control flow for accepted `if` and `while` shapes

The compile-time evaluator must not depend on Cranelift values, blocks, builders, or object emission. Cranelift lowering must not be the source of Doria language semantics.

## Non-goals

Stage 7a does not add:

- new Doria source syntax
- public Doria MIR
- final native-oriented IR design
- broader native code generation beyond the Stage 6c smoke subset
- general control-flow graph lowering
- nested `while`
- `break` or `continue`
- function calls, methods, classes, objects, collections, strings, stdout, FFI, runtime services, or LLVM output

Unsupported backend coverage must remain an unsupported native backend diagnostic. It must not be described as invalid Doria.

Stage 10 later extends this private native smoke boundary with top-level free functions and calls. That extension remains implementation-private and does not make `NativeSmokeModule` public Doria IR, final MIR, or a stable ABI. See `docs/decisions/0037-stage-10-native-free-functions.md`.

## Retirement

This record describes the historical Stage 7-10 bootstrap architecture. Decision 0043 retired and deleted `NativeSmokeModule` at Stage 11h after accepted Stage <=10 behavior moved to MIR, Cranelift consumed MIR directly, and the manifest-driven differential suite passed.
