# AGENTS.md

## Project

Doria is a new PHP-shaped compiled programming language. The compiler is `doriac`, and `doriac` is implemented in Rust. The native backend is the primary long-term target; PHP is only a compatibility, migration, debugging, and transpilation backend.

## Working rules

- Treat `docs/doria-development-plan.md` and `SPEC.md` as the product direction.
- Keep compiler work incremental and tested.
- Do not describe Doria as a Rust language. Rust is only the implementation language for `doriac`.
- Preserve the backend-independent pipeline: lexer -> parser -> AST -> semantic analysis -> type checker -> readonly/writable checker -> borrow/lifetime analysis later -> HIR -> MIR later -> backend.
- Do not let PHP backend needs leak into the parser, AST, semantic model, HIR, or MIR design.
- Favor clear diagnostics over permissive parsing.
- Do not introduce external Rust crates unless they remove real complexity and the repository is ready to manage that dependency.

## MVP non-goals

- Full PHP compatibility.
- Native code generation in the current v0.1 slice.
- LLVM or MLIR integration in the current v0.1 slice.
- Async/await.
- Borrow checking across tasks.
- Interfaces, traits, namespaces, reflection, attributes, macros, or package management.
- `Vec` as a collection alias.

## Verification

Run:

```bash
cargo test
cargo run -p doriac -- check examples/person.doria
cargo run -p doriac -- hir examples/person.doria
cargo run -p doriac -- compile examples/person.doria --target php --out build/person.php
```
