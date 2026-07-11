# Current Pipeline

Documentation role: working note. This file prevents duplicated in-flight work. It is not a roadmap; `docs/doria-end-to-end-plan.md` owns the roadmap.

## Recently merged

- PR #68: Stage 11 completion, direct MIR-to-Cranelift lowering, the differential parity suite, and retirement of the Stage 7-10 native smoke architecture.
- PR #69: Stage 12 reusable CFG/dataflow analysis, recursion and mutual recursion, `doria-rt`, abort-only panic with Doria stack traces, and exact stdout/stderr/status parity.

## Active

- Stage 13 is complete locally on the feature branch: the fixed-width integer family, integer operators, contextual literals, checked conversions, typed MIR/interpreter/Cranelift execution, PHP compatibility boundary, and durable native parity coverage.
- `int` is the canonical signed 64-bit integer and `int64` is its exact alias. The explicit unsigned family has no bare `uint` alias.
- Arithmetic, division/remainder, shifts, conversions, fixed-width function ABI values, and `uint64` boundary transport share exact interpreter/native results and panic outcomes.

## Next

- Stage 14: `float32`/`float64` execution and bool runtime values over the same MIR/interpreter/native architecture.

## Do not duplicate

- PR #68 Stage 11 completion work.
- PR #69 Stage 12 CFG/dataflow, recursion, runtime, panic, and durable parity work.
- Stage 13 integer-model, operator, conversion, and parity work.
- ROADMAP-style planning outside the end-to-end plan.

## Deferred

- Full TypeId/TypeKind refactor.
- Native float values and operations, float/integer conversions, and bool runtime values until Stage 14.
