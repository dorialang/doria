# Current Pipeline

Documentation role: working note. This file prevents duplicated in-flight work. It is not a roadmap; `docs/doria-end-to-end-plan.md` owns the roadmap.

## Recently merged

- PR #68: Stage 11 completion, direct MIR-to-Cranelift lowering, the differential parity suite, and retirement of the Stage 7-10 native smoke architecture.

## Completed locally

- Stage 12: general control flow and the minimal `doria-rt` runtime/panic foundation.
- Path-sensitive return analysis now uses a reusable source CFG and dataflow engine.
- Recursion and mutual recursion are supported without ordinary interpreter block or call-depth caps.
- Native compilation no longer executes user code as a preflight.
- `doria-rt` owns process entry, exact native output, abort-only panic status 101, and Doria function-name stack traces.

## Next

- Stage 13: full fixed-width integer family and remaining integer operators.

## Do not duplicate

- PR #68 Stage 11 completion work.
- Stage 12 CFG/dataflow, recursion, runtime, panic, and durable parity work.
- ROADMAP-style planning outside the end-to-end plan.

## Deferred

- Full TypeId/TypeKind refactor.
- Stage 13 numeric expansion until Stage 12 is reviewed and merged.
