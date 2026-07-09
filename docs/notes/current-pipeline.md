# Current Pipeline

Documentation role: working note. This file prevents duplicated in-flight work. It is not a roadmap; `docs/doria-end-to-end-plan.md` owns the roadmap.

## Recently merged

- PR #58: Stage 11b MIR integer locals, expressions, arithmetic, debug interpreter expansion.
- PR #59: dynamic boundary type cleanup, explicit parameter types, `mixed`, `object`, `resource`, `null` rules.
- PR #62: documentation source-of-truth reset and documentation authority guardrails.
- PR #63: Stage 11c MIR conditions, structured `if` control flow, and multi-block interpreter execution.

## Active

- Stage 11d: MIR `while` loops and loop control.

## Next after Stage 11d

- Stage 11e: MIR traditional `for` loops and integer range `foreach`, unless Stage 11d review identifies a blocker.

## Do not duplicate

- PR #58 Stage 11b MIR integer work.
- PR #59 dynamic boundary type work.
- PR #62 documentation source-of-truth work.
- PR #63 Stage 11c MIR condition and `if` control-flow work.
- ROADMAP-style planning outside the end-to-end plan.

## Deferred

- Full TypeId/TypeKind refactor.
- Stage 12 general control flow.
- NativeSmokeModule deletion until MIR covers the accepted Stage <=10 native subset.
