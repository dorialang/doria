# AGENTS.md

## Project

Doria is a new PHP-shaped programming language. The compiler is `doriac`, and `doriac` is implemented in Rust. The first target backend emits PHP.

## Working rules

- Treat `docs/doria-development-plan.md` and `SPEC.md` as the product direction.
- Keep compiler work incremental and tested.
- Do not describe Doria as a Rust language. Rust is only the implementation language for `doriac`.
- Preserve the pipeline: lexer -> parser -> AST -> semantic/type checker -> readonly/writable checker -> PHP code generator.
- Favor clear diagnostics over permissive parsing.
- Do not introduce external Rust crates unless they remove real complexity and the repository is ready to manage that dependency.

## MVP non-goals

- Full PHP compatibility.
- Native code generation.
- LLVM or MLIR.
- Async/await.
- Borrow checking across tasks.
- Interfaces, traits, namespaces, reflection, attributes, macros, or package management.
- `Vec` as a collection alias.

## Verification

Run:

```bash
cargo test
cargo run -p doriac -- check examples/person.doria
cargo run -p doriac -- compile examples/person.doria --target php --out build/person.php
```
