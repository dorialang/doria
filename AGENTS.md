# AGENTS.md

## Project

Doria is a new PHP-shaped compiled programming language. The compiler is `doriac`, and `doriac` is initially implemented in Rust. The native backend is the primary long-term target; PHP is only a compatibility, migration, debugging, and transpilation backend.

A strategic early goal is **self-hosting**: as Doria matures, more of `doriac` should be written in Doria itself. Rust is the bootstrap implementation language, not the permanent identity of the compiler.

Doria may intentionally support features PHP cannot express directly, including executable instance property initializers and richer attribute/metadata expressions. PHP backend limitations must not define Doria semantics.

## Working rules

- Treat `docs/doria-development-plan.md`, `docs/self-hosting.md`, `docs/executable-initializers-and-attributes.md`, and `SPEC.md` as the product direction.
- Keep compiler work incremental and tested.
- Do not describe Doria as a Rust language. Rust is only the initial implementation language for `doriac`.
- Preserve the backend-independent pipeline: lexer -> parser -> AST -> semantic analysis -> type checker -> readonly/writable checker -> borrow/lifetime analysis later -> HIR -> MIR later -> backend.
- Do not let PHP backend needs leak into the parser, AST, semantic model, HIR, or MIR design.
- Keep self-hosting in mind when designing compiler APIs, diagnostics, source management, HIR/MIR, and the standard library.
- Keep executable initializers and attribute expressions represented as Doria concepts, not PHP workarounds.
- Favor clear diagnostics over permissive parsing.
- Do not introduce external Rust crates unless they remove real complexity and the repository is ready to manage that dependency.

## MVP non-goals

- Full PHP compatibility.
- Native code generation in the current v0.1 slice.
- LLVM or MLIR integration in the current v0.1 slice.
- Full self-hosting in the current v0.1 slice.
- Full attribute evaluation in the current v0.1 slice.
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
