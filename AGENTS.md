# AGENTS.md

## Project

Doria is a compiled programming language for native applications, command-line tools, services, games, and systems software. The compiler is `doriac`, and the current `doriac` is a Rust bootstrap compiler. The native backend is the primary long-term target; PHP is only a compatibility, migration, debugging, and inspection backend.

A strategic early goal is **self-hosting**: as Doria matures, more of `doriac` should be written in Doria itself. Rust is the bootstrap implementation language, not the permanent identity of the compiler.

Doria is intended for native CLI tools, native desktop applications, game tooling, game engines, graphics/media work, C-library bindings, and future raylib bindings.

Doria may intentionally support features PHP cannot express directly, including executable instance property initializers and richer attribute/metadata expressions. PHP backend limitations must not define Doria semantics.

Doria may eventually include a PHP-to-Doria migration converter, but that converter is a migration tool, not the Doria parser and not the core compiler identity.

## Working rules

- Treat `docs/brand-positioning.md`, `docs/doria-development-plan.md`, `docs/self-hosting.md`, `docs/executable-initializers-and-attributes.md`, `docs/php-interop-and-migration.md`, `docs/performance-and-benchmarking.md`, `docs/mutability-ergonomics.md`, and `SPEC.md` as the product direction.
- Keep compiler work incremental and tested.
- Do not make PHP the public identity of Doria. PHP is development context, migration context, and one compatibility backend; Doria should be described as its own language.
- Do not describe Doria as a Rust language. Rust is only the bootstrap implementation language for the current `doriac`.
- Preserve the public compiler pipeline: lexer -> parser -> AST -> semantic/type checking -> Doria IR -> backend.
- Treat Doria IR as the checked compiler-owned representation. A lowered/native IR may come later for control flow, memory layout, runtime calls, and native backend code generation.
- Do not let PHP backend needs leak into the parser, AST, semantic model, Doria IR, or native-oriented IR design.
- Do not reintroduce `public`, `protected`, or `private` as Doria member visibility modifiers. Doria class members are externally accessible by default; use `internal` for implementation details.
- Keep `writable` and `internal` separate: `writable` controls mutation, while `internal` controls API surface.
- Keep self-hosting in mind when designing compiler APIs, diagnostics, source management, Doria IR, and the standard library.
- Keep native desktop, game engine, C-library binding, and raylib goals visible when designing Doria IR, future native-oriented IR, runtime, memory representation, FFI, and performance benchmarks.
- Keep executable initializers and attribute expressions represented as Doria concepts, not PHP workarounds.
- Keep PHP-to-Doria migration architecturally separate from the Doria parser. The migration tool may parse PHP, but Doria itself should parse Doria.
- Preserve readonly-by-default as the language default. Use class-level ergonomics such as `writable class`/`readonly class` before adding shorter aliases for `writable`.
- Favor clear diagnostics over permissive parsing.
- Do not introduce external Rust crates unless they remove real complexity and the repository is ready to manage that dependency.

## MVP non-goals

- Full PHP compatibility.
- Native code generation in the current v0.1 slice.
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

## Verification

Run:

```bash
cargo test
cargo run -p doriac -- check examples/person.doria
cargo run -p doriac -- hir examples/person.doria
cargo run -p doriac -- compile examples/person.doria --target php --out build/person.php
```
