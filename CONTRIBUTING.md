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

## Build artifact storage

Cargo reuses `target/` between builds but does not garbage-collect obsolete hashed artifacts. This repository therefore uses line-table debug information and disables incremental compilation for the test profile while leaving ordinary development builds incremental.

Run the non-destructive size guard before and after a full Rust validation:

```bash
php scripts/check_cargo_target_size.php
```

The command exits nonzero when `target/` exceeds 15 GiB and never removes anything. Inspect an oversized directory with an appropriate disk-usage tool and use `cargo clean --dry-run` to preview Cargo's cleanup. Run `cargo clean` only as an intentional, approved maintenance action; cleaning after every build would discard useful artifacts and force unnecessary cold rebuilds.

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
