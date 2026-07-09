# Current Pipeline

Documentation role: working note. This file prevents duplicated in-flight work. It is not a roadmap; `docs/doria-end-to-end-plan.md` owns the roadmap.

## Recently merged

- PR #58: Stage 11b MIR integer locals, expressions, arithmetic, debug interpreter expansion.
- PR #59: dynamic boundary type cleanup, explicit parameter types, `mixed`, `object`, `resource`, `null` rules.
- PR #62: documentation source-of-truth reset and documentation authority guardrails.

## Active

- Stage 11c: MIR conditions and `if` / `else if` / `else` control flow.

## Next after Stage 11c

- Stage 11d: MIR loops and loop control, unless Stage 11c review identifies a blocker.

## Do not duplicate

- PR #58 Stage 11b MIR integer work.
- PR #59 dynamic boundary type work.
- PR #62 documentation source-of-truth work.
- ROADMAP-style planning outside the end-to-end plan.

## Deferred

- Full TypeId/TypeKind refactor.
- Stage 12 general control flow.
- NativeSmokeModule deletion until MIR covers the accepted Stage <=10 native subset.
