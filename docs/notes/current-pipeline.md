# Current Pipeline

Documentation role: working note. This file prevents duplicated in-flight work. It is not a roadmap; `docs/doria-end-to-end-plan.md` owns the roadmap.

## Recently merged

- PR #69: Stage 12 reusable CFG/dataflow analysis, recursion and mutual recursion, `doria-rt`, abort-only panic with Doria stack traces, and exact stdout/stderr/status parity.
- PR #70: Stage 13 fixed-width integers, operators, contextual literals, checked conversions, scalar-width ABI coverage, and durable panic parity.

## Active

- Stage 14 floats and bool runtime is complete locally on `feature/stage-14-floats-and-bool-runtime` after the full formatting, clippy, build, workspace-test, editor/docs, and durable parity validation gate passed.
- The implementation uses one MIR scalar path for fixed-width integers, binary32/binary64 floats, and bool locals, parameters, returns, calls, assignments, and branches.
- IEEE special values, bool short-circuiting, `Int::toFloat`, checked `Float::toInt`, PHP boundaries, LSP/editor status, and durable interpreter/native parity are included.

## Next

- Stage 15: LLVM release backend over the same Stage 14 MIR. No Stage 15 implementation is included here.

## Do not duplicate

- PR #69 Stage 12 CFG/dataflow, recursion, runtime, panic, and durable parity work.
- PR #70 Stage 13 integer-model, operator, conversion, and parity work.
- ROADMAP-style planning outside the end-to-end plan.

## Deferred

- Runtime strings and heap allocation.
- LLVM/release lowering until Stage 15 begins separately.
