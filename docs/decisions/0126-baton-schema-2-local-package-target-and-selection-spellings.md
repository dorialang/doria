# Decision 0126: Baton Schema 2 Local Package, Target, And Selection Spellings

- **Status:** Accepted
- **Accepted:** 2026-08-27
- **Date:** 2026-08-27
- **Implementation Status:** Implemented By Stage 33 Slice 1
- **Amends:** Decisions 0117, 0118, and 0124
- **Preserves:** Decision 0125's metadata and processor protocol; compiler build-plan schema 1; package/namespace separation; direct dependency visibility; package-wide `internal`; and the mandatory Pre-Stage-45 Doria-native Baton transition

## Context

Decisions 0117 and 0118 fixed the package graph and complete manifest direction,
but deliberately left the exact local-package, target-table, and target-selection
spellings to Stage 33 Slice 1. The PHP Baton bootstrap needed those spellings to
exercise deterministic project planning without becoming a second compiler or
defining the permanent implementation architecture.

This decision records the public contract now exercised by
`dorialang/baton-php`. It does not make the PHP bootstrap permanent. Decision
0124 still requires a parity-gated port to the Doria-native `dorialang/baton`
repository before the first unsuffixed toolchain release.

## Manifest Compatibility

Manifest schema 1 retains its exact historical contract: one explicitly named
binary entry, direct compiler invocation, and the existing unscoped package-name
and build-layout rules. Baton does not reinterpret schema 1 as schema 2, emit a
schema-2 build plan for it, or add edition, publishability, autoload, target,
dependency, workspace, processor, or lockfile semantics to it.

Manifest schema 2 is parsed as strict TOML and validated into typed manifest
models. Unknown fields are errors rather than ignored input. Schema 2 requires
`name`, SemVer `version`, and the accepted `edition = "2026"`, plus exactly one
target-declaration mode.

## Package Identity And Publishability

An unscoped local package is explicit:

```toml
manifest-version = 2

[package]
name = "hello"
version = "0.1.0"
edition = "2026"
publishable = false
kind = "binary"
entry = "src/main.doria"
```

An unscoped package is valid only when `publishable = false` is present. Omitting
the field or setting it to `true` is rejected. A scoped lowercase
`vendor/package` identity is publishable by default and may explicitly set
`publishable` to either `true` or `false`.

Compiler build plans require a canonical `vendor/package` identity. Baton maps
the local manifest name `hello` deterministically to `local/hello`. The
synthetic vendor `local` is reserved; a user-authored scoped schema-2 package
whose identity begins with `local/` is rejected. User output retains the
manifest name, while build receipts record both the manifest name and compiler
package identity.

The general mapping is `local/<name>`.

Package identity, namespace identity, and filesystem location remain separate.
No namespace, path, or random hash participates in the local identity mapping.

## Targets

The package-level form remains the single-binary shorthand:

```toml
[package]
name = "acme/blog"
version = "1.0.0"
edition = "2026"
kind = "binary"
entry = "src/main.doria"
```

It creates one binary target named from the final package-name segment. It is
mutually exclusive with explicit target tables. There is no package-level
library shorthand.

Explicit targets use:

```toml
[targets.library]
name = "blog"

[[targets.binary]]
name = "web"
entry = "src/web.doria"

[[targets.binary]]
name = "worker"
entry = "src/worker.doria"
```

A package has at most one library, zero or more binaries, and at least one target
overall. Target names are unique across kinds and use the filesystem-safe
lowercase slug accepted for schema-1 package names. A library has no entry; each
binary has exactly one entry. Two binaries may intentionally share an entry.
No `[lib]`, `[[bin]]`, generic `[targets]`, or `default-target` spelling exists.

## Target Selection

Commands select targets with:

```console
baton check --binary web
baton build --binary web
baton run --binary web
baton check --library
baton build --library
```

`--binary <name>` and `--library` are mutually exclusive. There is no generic
`--target` option. `check` and `build` select automatically only when exactly one
target exists. `run` selects automatically only when exactly one binary exists,
never selects a library, and rejects `--library`. Ambiguous and unknown
selections report the available targets.

Schema 1 keeps its implicit binary behavior. It may accept a matching
`--binary <package-name>` for uniformity, but rejects `--library`.

## Source Discovery And Scopes

Schema 2 accepts `[autoload.namespaces]` and `[autoload-dev.namespaces]` with
simple directory mappings and advanced include/exclude mappings. Baton discovers
matching `.doria` files deterministically, validates source-root and symlink
containment, validates exact filesystem case, rejects portable case collisions,
and records stable source identities.

Main and development sources remain distinct. Development sources participate
only in development-aware commands. A selected binary entry is represented as
the target entry rather than rediscovered as an ordinary source. Generated
source has an internal input boundary for Stage 33 Slice 3; Slice 1 does not
discover processors, execute processors, or write generated source.

Baton parses no Doria declarations. It discovers project inputs and emits a
compiler plan; `doriac` owns parsing, semantics, lowering, and code generation.

## Build Plans, Layouts, And Receipts

Schema-2 commands emit compiler build-plan schema 1 and invoke `doriac` through
`--build-plan`. Baton owns manifest parsing, target selection, source discovery,
plan construction, target-scoped build directories, and build receipts. The
compiler owns the build-plan schema and compilation behavior.

Schema 2 uses a target-scoped layout:

```text
build/<host-target>/<profile>/<target-name>/
```

Schema 1 preserves its existing layout without a target-name directory:

```text
build/<host-target>/<profile>/
```

`baton check --library` performs a complete compiler check. `baton build
--library` performs the same check and writes the selected plan and successful
receipt, with `"artifact": null`. It does not invoke native library compilation,
invent an archive format, or create a placeholder artifact. `baton run
--library` is rejected.

## Slice Boundaries

Stage 33 Slices 1 through 3 are complete. Decision 0127 owns the implemented path
and Git dependency resolver, package SemVer validation, one-version conflicts,
strict deterministic `Baton.lock`, dependency commands, global
content-addressed cache, offline policy, multi-package plans, and receipt
identities. Implemented Stage 33 Slice 3 owns workspaces, development
orchestration, graph commands, tests, processors, generated-source writes, and
Phase F closure.

Stage 33 and Phase F are complete. The PHP bootstrap remains a
temporary UX oracle. Decision 0124's Pre-Stage-45 transition must port the
complete Stage 33 contract to Doria, transfer release ownership, remove the PHP
payload from production archives, and pass parity before the unsuffixed
`2026.03.1` release.

## Consequences

- Local projects have a portable compiler identity without pretending to be
  publishable packages.
- Target selection is explicit when ambiguity exists and stays terse otherwise.
- Source discovery is deterministic and secure across supported host filesystems.
- Schema 1 users retain exact compatibility while new projects use schema 2.
- A library can be checked before Doria has a public native library artifact.
- Dependency behavior is implemented without claiming workspace, graph-command,
  test, or processor behavior early.

## Invalidated Elsewhere

- `docs/doria-end-to-end-plan.md` and `docs/notes/current-pipeline.md`: all three
  slices, Stage 33, and Phase F are complete, with Stage 34 next.
- Decisions 0117, 0118, and 0124: deferred spellings and implementation status.
- `SPEC.md` and `README.md`: current Baton schema-2 project behavior.
- Doria authority guards: decision existence, public spellings, implementation
  status, and native-transition requirements.
- `dorialang/baton-php`: executable behavior and tests implement this record.
- `dorialang/doria-language-server`: consumes the completed Baton project
  inventory contract without parsing manifests or lockfiles.
