# Contributing

Contributions should keep the compiler architecture honest: PHP is a backend, while the core compiler should remain ready for native code generation.

Language-server transport and editor integrations live in [`dorialang/doria-language-server`](https://github.com/dorialang/doria-language-server). Compiler changes should expose reusable frontend services and coordinate editor-visible follow-up there rather than adding IDE clients to this repository.

## Development

Run these before opening a pull request:

```bash
php scripts/validate_work_unit.php
```

## Build artifact storage

Cargo reuses `target/` between builds but does not garbage-collect obsolete hashed artifacts. This repository limits debug metadata, preserves useful incremental development artifacts, and gives feature-incompatible validation graphs stable cache namespaces.

The work-unit validator reports allocated size before and after validation. It automatically uses Cargo's own cleanup operation when the cache identity changes, free space is critically low, or the managed target exceeds 15 GiB. It does not clean after every run, because cold rebuilds waste time and increase SSD writes.

Inspect size without changing anything:

```bash
php scripts/check_cargo_target_size.php
```

Move the reusable cache to another volume with `DORIA_VALIDATION_TARGET_DIR` or `--target-dir`. Custom targets must be dedicated empty directories on first use; the validator records repository ownership and refuses roots, ancestors, or unowned shared caches. Reclaim it deliberately with:

```bash
php scripts/validate_work_unit.php --reclaim
```

Do not recursively delete `target/`; the repository-owned workflow remains portable across macOS, Linux, and Windows and lets Cargo own its artifacts.

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
