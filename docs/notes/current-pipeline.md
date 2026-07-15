# Current Pipeline

Documentation role: working note. This file prevents duplicated in-flight work. It is not a roadmap; `docs/doria-end-to-end-plan.md` owns the roadmap.

## Recently merged

- PR #75: Stage 17 integration, parity, examples, editor, docs, and CI closure.
- PR #76: Stage 17 naming, I/O-tier, and migration-guidance corrections.

## Active

- Stage 18 full expression interpolation and compiler-known `Displayable` is merged.
- Stage 19 ownership, moves, destruction, and native class layout is complete on the current branch.
- Native remains one target: direct compile/run uses the Cranelift fast profile, while `--release` selects LLVM 18 over the same validated typed MIR.
- Ordinary expression interpolation of primitive/string values lowers through the existing ordered MIR string and display operations consumed by all three execution paths.
- Native classes now cover construction, property initialization/access, class-valued locals/arguments/returns, `take` transfer, lifecycle bodies, recursive destruction, and deterministic normal structured-exit cleanup through the interpreter, Cranelift, and LLVM.
- `Displayable` conformance is checked by the frontend and executable through the PHP compatibility subset. Native `Displayable` dispatch remains Stage 20 method work.
- The durable manifest supports raw stdin, isolated seeded files, and exact interpreter/Cranelift/LLVM stdout, stderr, status, generated-file, and class-lifetime comparison.

## Next

- Stage 20: methods, statics, and `internal` native lowering.

## Do not duplicate

- Stage 17 I/O and formatting work from PRs #75 and #76.
- ROADMAP-style planning outside the end-to-end plan.

## Deferred

- Native `Displayable` method dispatch until Stage 20.
- General interface declarations and conformance until Stage 35.
- `Bytes` until Stage 23.
