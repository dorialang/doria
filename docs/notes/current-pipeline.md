# Current Pipeline

Documentation role: working note. This file prevents duplicated in-flight work. It is not a roadmap; `docs/doria-end-to-end-plan.md` owns the roadmap.

## Recently merged

- PR #75: Stage 17 integration, parity, examples, editor, docs, and CI closure.
- PR #76: Stage 17 naming, I/O-tier, and migration-guidance corrections.

## Active

- Stage 18 full expression interpolation and compiler-known `Displayable` is merged.
- Stage 19 ownership, moves, destruction, and native class layout is complete on the current branch.
- Stage 20 statically resolved instance/static methods, Copy-type static properties, class/top-level constants, `internal` enforcement, and concrete native `Displayable` execution are complete on the current branch. Static access is sigil-free, `self` resolves to the declaring class, and one class-level index rejects cross-kind member-name collisions.
- Stage 20a/20b const-evaluable defaults are complete for Copy scalars and readonly strings across free functions, instance methods, static methods, and constructors through one caller-side MIR splice. Writable Copy scalars remain supported; `?string`, `writable string`, `take string`, and other move/`take` defaults retain explicit temporary diagnostics.
- Stage 21 non-lexical borrowing, returned-borrow elision, and constructor definite initialization are complete on the current branch. Constructor paths use decision 0090's uninitialized/initialized/maybe-initialized lattice, and shared MIR validation independently enforces the normal-exit and readonly exactly-once invariants.
- `Shared<T>`/`Weak<T>`/`SharedMut<T>` are rescheduled to Stage 25a, after nullable/narrowing and generic classes provide the machinery their separately unauthored API depends on.
- The parser accepts generalized `parent::member()` and trait-local `self::member` under the two-clocks rule; semantic checking names Stage 34 and Stage 35 respectively and stops those forms before MIR. `Foo::$prop` and `static::` are permanent errors with precise fixes.
- Native remains one target: direct compile/run uses the Cranelift fast profile, while `--release` selects LLVM 18 over the same validated typed MIR.
- Ordinary expression interpolation of primitive/string values lowers through the existing ordered MIR string and display operations consumed by all three execution paths.
- Native classes now cover construction, property initialization/access, class-valued locals/arguments/returns, `take` transfer, lifecycle bodies, recursive destruction, and deterministic normal structured-exit cleanup through the interpreter, Cranelift, and LLVM.
- Concrete `Displayable` conversion lowers to an ordinary direct `toString()` method call for interpolation, `.`, `echo`, and `%s`; interface-typed values and general interface dispatch remain deferred.
- The durable manifest supports raw stdin, isolated seeded files, and exact interpreter/Cranelift/LLVM stdout, stderr, status, generated-file, and class-lifetime comparison.

## Next

- Stage 22 nullable types, narrowing, `is`, and `mixed` static semantics.

## Do not duplicate

- Stage 17 I/O and formatting work from PRs #75 and #76.
- ROADMAP-style planning outside the end-to-end plan.

## Deferred

- General interface declarations and conformance until Stage 35.
- Runtime-initialized and owned statics until separately accepted lifetime/concurrency decisions.
- Parent lookup/dispatch until Stage 34 and trait composition until Stage 35; their accepted grammar is already represented.
- `Bytes` until Stage 23.
