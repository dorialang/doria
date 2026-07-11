# Current Pipeline

Documentation role: working note. This file prevents duplicated in-flight work. It is not a roadmap; `docs/doria-end-to-end-plan.md` owns the roadmap.

## Recently merged

- PR #69: Stage 12 reusable CFG/dataflow analysis, recursion and mutual recursion, `doria-rt`, abort-only panic with Doria stack traces, and exact stdout/stderr/status parity.
- PR #70: Stage 13 fixed-width integers, operators, contextual literals, checked conversions, scalar-width ABI coverage, and durable panic parity.
- PR #71: Stage 14 IEEE floats, runtime bool values, explicit default numeric conversions, shared scalar MIR, and durable interpreter/Cranelift parity.

## Active

- Stage 15 LLVM release lowering is complete on `feature/stage-15-llvm-release-backend` after default and LLVM-enabled validation.
- Native remains one target: direct compile/run uses the Cranelift fast profile, while `--release` selects LLVM 18 over the same validated typed MIR.
- The durable manifest compares exact interpreter, Cranelift, and LLVM stdout, stderr, and status, including panic fixtures.

## Next

- Stage 16: `doria-rt` runtime strings and canonical display conversion.

## Do not duplicate

- PR #69 Stage 12 CFG/dataflow, recursion, runtime, panic, and durable parity work.
- PR #70 Stage 13 integer-model, operator, conversion, and parity work.
- ROADMAP-style planning outside the end-to-end plan.

## Deferred

- Runtime strings and heap allocation.
- Runtime strings and canonical display conversion until Stage 16 begins separately.
