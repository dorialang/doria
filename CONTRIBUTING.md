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

Cargo target directories contain only reclaimable build state. They are not a
distribution layout and installed tools may not retain paths into them.
`doriac` embeds the deterministic fast and release `doria-rt` archives used by
the standalone compiler and materializes the selected, identity-checked archive
from its own bytes. The development-toolchain refresh deliberately cleans its
`target/toolchain-install` build directory before it verifies a native compile
and execution through the installed compiler.

The work-unit validator reports allocated size before and after validation. It automatically uses Cargo's own cleanup operation when the cache identity changes, free space is critically low, or the managed target exceeds 15 GiB. It does not clean after every run, because cold rebuilds waste time and increase SSD writes.

After successful cleanup it records the current configuration identity again.
Focused builds can then reuse that cache without making the next validation
mistake it for an untracked target; configuration changes still require cleanup.

Inspect size without changing anything:

```bash
php scripts/check_cargo_target_size.php
```

Move the reusable cache to another volume with `DORIA_VALIDATION_TARGET_DIR` or `--target-dir`. Custom targets must be dedicated empty directories on first use; the validator records repository ownership and refuses roots, ancestors, or unowned shared caches. Reclaim it deliberately with:

```bash
php scripts/validate_work_unit.php --reclaim
```

For selective cleanup, first inspect a read-only JSON plan:

```bash
php scripts/validate_work_unit.php --reclaim-obsolete --plan
php scripts/validate_work_unit.php --reclaim-obsolete --approve-plan <planId>
```

Use the same `--target-dir` for both commands when selecting an external cache.
Two output forms qualify: superseded workspace test executables and stale
test linker objects. Cargo's versioned dependency record must identify a
repository source input newer than both the record and the completed link.
Executables require a newer retained executable for the same target, compiler,
features, profile and flags. Objects require a valid completed-compilation
marker and a recognized Rust code-generation filename; objects newer than the
completed link are preserved. A missing executable additionally requires a
newer retained executable in the same configuration. Cleanup keeps the newest
executable, libraries, incremental data, dependency records, installed tools and
unrecognized outputs. The version-3 JSON plan deduplicates compilation proofs
and binds every selected path and retained executable to its exact reviewed
identity. It verifies those proofs again under Cargo's profile locks and checks
active output/executable users with `lsof` on Unix or exclusive file sharing on
Windows. Unavailable checks, changed proofs or active consumers stop cleanup.
Coordinate idle build owners
before applying a plan; arbitrary direct launches do not participate in Cargo's
locks. This mode never starts a build or falls through to whole-target cleanup.
No eligible output means no deletion; an insufficient storage budget still
requires holding builds. Allocated candidate bytes are an estimate; remeasure
the aggregate cache across repositories after cleanup.

Interrupted test compilations have a separate selection mode:

```bash
php scripts/validate_work_unit.php --reclaim-interrupted --plan
php scripts/validate_work_unit.php --reclaim-interrupted --approve-plan <planId>
```

It requires a retained executable, checked workspace dependency metadata, an
empty completion marker newer than the prior completed build, and Cargo's
matching invocation record. Only recognized objects created since that
invocation qualify. Missing or inconsistent proof, older objects and unknown
filenames are preserved. The reviewed plan binds the selection mode and uses
the same file-identity, build-lock and active-use checks as obsolete cleanup.
It never removes the retained executable or starts a build. Do not combine
the two selection modes or apply a plan through a different mode.

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
