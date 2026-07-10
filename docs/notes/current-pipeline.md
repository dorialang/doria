# Current Pipeline

Documentation role: working note. This file prevents duplicated in-flight work. It is not a roadmap; `docs/doria-end-to-end-plan.md` owns the roadmap.

## Recently merged

- PR #58: Stage 11b MIR integer locals, expressions, arithmetic, debug interpreter expansion.
- PR #59: dynamic boundary type cleanup, explicit parameter types, `mixed`, `object`, `resource`, `null` rules.
- PR #62: documentation source-of-truth reset and documentation authority guardrails.
- PR #63: Stage 11c MIR conditions, structured `if` control flow, and multi-block interpreter execution.
- PR #64: Stage 11d MIR structured `while`, nested `break` / `continue`, and bounded debug interpretation.
- PR #65: Stage 11e MIR traditional `for`, integer range `foreach`, and mixed nested-loop control.
- PR #66: Stage 11f MIR top-level free functions, calls, and bounded interpreter frames.
- PR #67: Stage 11g MIR readonly string locals, string-expression echo parity, and the parity matrix.

## Active

- Stage 11h: complete MIR-native migration, differential suite, and retirement of the Stage 7-10 native smoke architecture.

## Next after Stage 11h

- Stage 12: general control flow and the minimal doria-rt runtime/panic foundation.

## Do not duplicate

- PR #58 Stage 11b MIR integer work.
- PR #59 dynamic boundary type work.
- PR #62 documentation source-of-truth work.
- PR #63 Stage 11c MIR condition and `if` control-flow work.
- PR #64 Stage 11d MIR `while` and loop-control work.
- PR #65 Stage 11e MIR `for`, integer range `foreach`, and mixed-loop work.
- PR #66 Stage 11f MIR top-level free functions and calls.
- PR #67 Stage 11g MIR readonly string locals and string-expression echo work.
- ROADMAP-style planning outside the end-to-end plan.

## Deferred

- Full TypeId/TypeKind refactor.
- Stage 12 general control flow.
