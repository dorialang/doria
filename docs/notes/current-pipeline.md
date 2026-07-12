# Current Pipeline

Documentation role: working note. This file prevents duplicated in-flight work. It is not a roadmap; `docs/doria-end-to-end-plan.md` owns the roadmap.

## Recently merged

- PR #69: Stage 12 reusable CFG/dataflow analysis, recursion and mutual recursion, `doria-rt`, abort-only panic with Doria stack traces, and exact stdout/stderr/status parity.
- PR #70: Stage 13 fixed-width integers, operators, contextual literals, checked conversions, scalar-width ABI coverage, and durable panic parity.
- PR #71: Stage 14 IEEE floats, runtime bool values, explicit default numeric conversions, shared scalar MIR, and durable interpreter/Cranelift parity.
- PR #72: Stage 15 LLVM release backend over shared validated MIR and triple differential parity.
- PR #73: Stage 16 runtime strings and canonical display conversion.
- PR #74: Stage 17 compiler/runtime core for narrow nullable strings, checked formatting, and UTF-8 I/O.

## Active

- Stage 17 integration, parity, examples, documentation, editor, and CI closure is active on `feature/stage-17-completion-parity-docs`.
- Native remains one target: direct compile/run uses the Cranelift fast profile, while `--release` selects LLVM 18 over the same validated typed MIR.
- Immutable UTF-8 strings and the narrow Stage 17 `?string` seed are private refcounted runtime values. Checked format plans and I/O operations are validated MIR consumed by all three execution paths.
- The durable manifest supports raw stdin, isolated seeded files, and exact interpreter/Cranelift/LLVM stdout, stderr, status, and generated-file comparison.

## Next

- Stage 18: full expression interpolation and `Displayable`.

## Do not duplicate

- PR #69 Stage 12 CFG/dataflow, recursion, runtime, panic, and durable parity work.
- PR #70 Stage 13 integer-model, operator, conversion, and parity work.
- ROADMAP-style planning outside the end-to-end plan.

## Deferred

- Full arbitrary-expression interpolation and `Displayable` until Stage 18.
- `Bytes` until Stage 23.
