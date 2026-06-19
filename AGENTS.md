# AGENTS.md

## Project

Doria is a compiled programming language for native applications, command-line tools, services, games, and systems software. The compiler is `doriac`, and the current `doriac` is a Rust bootstrap compiler. The native backend and standalone native executables are the primary product target. PHP is only an optional compatibility, migration, debugging, and inspection backend.

A strategic early goal is **self-hosting**: as Doria matures, more of `doriac` should be written in Doria itself. Rust is the bootstrap implementation language, not the permanent identity of the compiler.

Doria is intended for native CLI tools, native desktop applications, game tooling, game engines, graphics/media work, C-library bindings, and future raylib bindings.

The accepted native backend strategy is dual-profile:

```text
Fast native profile       -> Cranelift
Optimized native profile  -> LLVM
```

The fast profile is for local development feedback and smoke builds. The optimized profile is for release/shipping builds. Both profiles must preserve the same Doria-visible semantics for supported code.

Doria may intentionally support features PHP cannot express directly, including executable instance property initializers and richer attribute/metadata expressions. PHP backend limitations must not define Doria semantics.

Doria may eventually include a PHP-to-Doria migration converter, but that converter is a migration tool, not the Doria parser and not the core compiler identity.

## Non-negotiable engineering guardrails

- Correctness and accuracy outrank quick demos, fast runnable output, and compatibility shortcuts.
- Design Doria as a native-first language even when a native backend slice is not yet implemented.
- Do not choose syntax, type rules, runtime behavior, standard-library shape, or IR structure because it is easier for the PHP backend.
- Do not treat generated PHP as a semantic oracle. It is backend output, not the definition of Doria.
- Do not silently import behavior from PHP, Rust, JavaScript, C, C++, or any backend/runtime ecosystem.
- If implementation hits a language-design fork not explicitly settled by `SPEC.md`, `docs/decisions/`, or the current task prompt, stop and ask Andrew.
- When stopping for a design decision, report the question, viable options, tradeoffs, affected files, and a recommendation clearly marked as a recommendation.
- Do not implement a workaround that makes the current backend pass while leaving Doria semantics ambiguous.
- Prefer clear unsupported-feature diagnostics over permissive behavior that may become wrong.
- Preserve the ability to lower Doria to native code safely, even if the immediate task only touches frontend code or a compatibility backend.

## Working rules

- Treat `docs/brand-positioning.md`, `docs/doria-development-plan.md`, `docs/self-hosting.md`, `docs/executable-initializers-and-attributes.md`, `docs/php-interop-and-migration.md`, `docs/performance-and-benchmarking.md`, `docs/mutability-ergonomics.md`, `docs/api-design-guidelines.md`, `docs/decisions/`, and `SPEC.md` as the product direction.
- Keep compiler work incremental and tested, but never use incremental delivery as an excuse to make unsound language decisions.
- Do not make PHP the public identity of Doria. PHP is development context, migration context, and one optional compatibility backend; Doria should be described as its own native-first language.
- Do not describe Doria as a Rust language. Rust is only the bootstrap implementation language for the current `doriac`.
- Preserve the public compiler pipeline: lexer -> parser -> AST -> semantic/type checking -> Doria IR -> backend.
- Treat Doria IR as the checked compiler-owned representation. A lowered/native IR may come later for control flow, memory layout, runtime calls, and native backend code generation.
- Do not let PHP backend needs leak into the parser, AST, semantic model, Doria IR, or native-oriented IR design.
- For native work, keep the fast Cranelift profile and optimized LLVM profile semantically equivalent for supported code. Differences may be in compile time, optimization, debug information, and binary quality, not Doria behavior.
- Do not let Cranelift or LLVM semantics decide Doria semantics. Backend-specific assumptions must remain behind Doria IR or native-oriented IR lowering.
- Do not reintroduce `public`, `protected`, or `private` as Doria member visibility modifiers. Doria class members are externally accessible by default; use `internal` for implementation details.
- Keep `writable` and `internal` separate: `writable` controls mutation, while `internal` controls API surface.
- Keep self-hosting in mind when designing compiler APIs, diagnostics, source management, Doria IR, and the standard library.
- Keep native desktop, game engine, C-library binding, and raylib goals visible when designing Doria IR, future native-oriented IR, runtime, memory representation, FFI, and performance benchmarks.
- Keep executable initializers and attribute expressions represented as Doria concepts, not PHP workarounds.
- Keep PHP-to-Doria migration architecturally separate from the Doria parser. The migration tool may parse PHP, but Doria itself should parse Doria.
- Preserve readonly-by-default as the language default. Use class-level ergonomics such as `writable class`/`readonly class` before adding shorter aliases for `writable`.
- Treat basic `if` / `else if` / `else` and `while` as MVP control flow. Keep `finally`, `do ... while`, `given`, value-returning `when`, `match`, `break`, and `continue` as planned control-flow ideas only until their grammar and semantics are specified.
- Prefer nouns as properties and verbs as methods in Doria APIs and examples. Use property hooks for computed, validated, lazy, or guarded values instead of vague zero-argument noun methods such as `body()`.
- Do not introduce external Rust crates unless they remove real complexity and the repository is ready to manage that dependency.

## MVP non-goals

- Full PHP compatibility.
- Treating PHP transpilation as a correctness milestone for Doria.
- Complete native code generation and runtime support in the current v0.1 slice.
- LLVM or MLIR integration in the current v0.1 slice.
- Full self-hosting in the current v0.1 slice.
- Full attribute evaluation in the current v0.1 slice.
- PHP-to-Doria migration in the current v0.1 slice.
- Desktop application framework work in the current v0.1 slice.
- Game engine work in the current v0.1 slice.
- Raylib bindings in the current v0.1 slice.
- FFI implementation in the current v0.1 slice.
- Async/await.
- Borrow checking across tasks.
- Interfaces, traits, namespaces, reflection, macros, or package management.
- `Vec` as a collection alias.

Small native backend smoke tests are not ruled out by these non-goals. They are preferred once the required semantics are explicit enough to avoid backend-shaped shortcuts.

## Verification

Run core checks for compiler changes:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --workspace --all-targets --locked --verbose
cargo test --workspace --all-targets --locked --verbose
cargo run -p doriac -- check examples/person.doria
cargo run -p doriac -- hir examples/person.doria
```

Run backend-specific checks only when the touched task depends on that backend. For the current PHP compatibility backend:

```bash
cargo run -p doriac -- compile examples/person.doria --target php --out build/person.php
```

When a native backend target exists, native smoke tests must be part of the relevant definition of done.
