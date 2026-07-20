# Contributing

Contributions should keep the compiler architecture honest: PHP is a backend, while the core compiler should remain ready for native code generation.

Language-server transport and editor integrations live in [`dorialang/doria-language-server`](https://github.com/dorialang/doria-language-server). Compiler changes should expose reusable frontend services and coordinate editor-visible follow-up there rather than adding IDE clients to this repository.

## Development

Run these before opening a pull request:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --all-targets
```

## Pull requests

- Keep changes focused.
- Add tests for compiler behavior changes.
- Prefer clear diagnostics over permissive parsing.
- Do not add dependencies without explaining the tradeoff.
- Update `SPEC.md` when language behavior changes.

## Architecture

The current pipeline is:

```text
lexer -> parser -> AST -> semantic checks -> HIR -> backend
```

MIR and native backend work should be introduced as explicit phases rather than by bending HIR or the PHP backend around native needs.
