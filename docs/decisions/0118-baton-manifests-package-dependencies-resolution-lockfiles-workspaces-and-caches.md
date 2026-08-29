# Decision 0118: Baton Manifests, Package Dependencies, Resolution, Lockfiles, Workspaces, And Caches

- **Status:** Accepted
- **Accepted:** 2026-08-14
- **Date:** 2026-08-14
- **Implementation status:** Stage 33 Slices 1 through 3 and Phase F implemented
  in the disposable PHP UX bootstrap; Doria-native port and production cutover
  scheduled Pre-Stage-45
- **Scope:** Baton's package manifest, dependency resolver, lockfile, workspace,
  processor, cache, and offline contracts
- **Build-graph dependency:** Decision 0117
- **Implementation ownership and release gate:** Decision 0124

## Context

Baton's schema 1 bootstrap intentionally supports one binary entry and no
dependencies. Phase F needs a complete package contract before extending that
parser, otherwise bootstrap shortcuts could become permanent lockfile,
resolution, or workspace semantics.

This decision fixes the target model while preserving schema 1 exactly as it
exists. Decision 0126 records the Slice 1 schema-2 spellings and implementation;
Decision 0127 records the Slice 2 resolver, lockfile, cache, command, and offline
contract now exercised by the bootstrap. Decision 0124 fixes the implementation
sequence: Stage 33 exercises this model in the disposable PHP bootstrap, then
the mandatory Pre-Stage-45 transition ports the frozen behavior to production
Doria Baton.

## Manifest Schema 2

The complete Phase F manifest uses this common shape:

```toml
manifest-version = 2

[package]
name = "acme/blog"
version = "1.0.0"
edition = "2026"
kind = "binary"
entry = "src/main.doria"

[autoload.namespaces]
"Acme\\Blog\\" = "src/"

[autoload-dev.namespaces]
"Acme\\Blog\\Tests\\" = "tests/"

[dependencies]
"acme/database" = { path = "../database" }

"acme/http" = {
    git = "https://code.example.com/acme/http.git",
    tag = "v1.4.0",
    version = "^1.4"
}

[dev-dependencies]
"acme/test-support" = { path = "../test-support" }

[processors]
"acme/route-processor" = { path = "../route-processor" }
```

Decision 0117 owns autoload, layout, targets, source scopes, package identity,
and the build plan.

## Schema 1 Compatibility

Manifest schema 1 keeps its original bootstrap meaning: one binary target, one
explicit entry file, no autoload, no dependencies, no `Baton.lock`, and no
workspace. It is readable only under those semantics.

Schema 1 is not silently reinterpreted as schema 2 and never becomes
dependency-aware. Future migration guidance requires explicit user action;
Baton does not rewrite a manifest automatically.

## Dependency Visibility

Source may name symbols from its own package, the standard library and prelude,
and directly declared dependencies. Transitive dependencies are not implicitly
visible. A package must declare every package whose symbols it names.

An externally accessible API may not expose a type from an undeclared direct
dependency. The compiler diagnoses the signature, exposed type, owning package,
and missing declaration. Package re-exports are deferred rather than invented
as an exception.

## Resolution Identity

One complete build graph resolves one version of each canonical package
identity. Side-by-side versions are rejected because Doria type identity is not
package-version-qualified. A version conflict reports every dependency chain,
constraint, and source that contributes to it.

Every package dependency cycle is rejected, including cycles through normal,
active development, processor, and workspace-member edges. File references
inside one package are not package cycles.

## Initial Sources

The first resolver supports path and Git dependencies.

```toml
[dependencies]
"acme/database" = { path = "../database" }

"acme/http" = {
    git = "https://code.example.com/acme/http.git",
    tag = "v1.4.0",
    version = "^1.4"
}
```

A Git dependency uses exactly one selector: `rev`, `tag`, or `branch`. Tags and
branches select a revision but are not reproducible identities; `Baton.lock`
always records the exact commit. An optional SemVer constraint validates the
selected package's manifest version. Each dependency manifest owns its own
autoload mappings.

Path dependencies remain live development inputs. The lockfile records their
identity, manifest-relative path, version, and edges; the build receipt records
canonical local identity and content hash. A committed lockfile does not freeze
local path contents. Published packages may not retain unresolved path
dependencies, although publishing itself is deferred.

## Source-Neutral Model

Source transport and artifact role are separate typed concepts. Source kinds
leave room for `path`, `git`, `registry`, and `verified-archive`; only path and
Git execute initially. Artifact roles leave room for Doria source packages,
processors, native libraries, binary tools, and future prebuilt artifacts.

Remote lock entries record a canonical source URL without embedded credentials,
access tokens, or secret query parameters. Conflicting source URLs for one
package identity are diagnosed as source substitution rather than silently
accepted.

Registry resolution and publishing, arbitrary ZIP or tarball URLs, verified
archives, native package feeds, and binary artifact feeds are deferred. The
lock schema remains extensible without pretending those sources already work.

## Version Constraints

Packages use SemVer. Initial accepted forms are exact, caret, tilde, and bounded
comparator ranges, for example `1.4.2`, `^1.4`, `~1.4.2`, and `>=1.4 <2.0`.
Pre-release versions must be requested explicitly.

OR expressions, Composer stability flags, implicit development stability, and
toolchain CalVer ranges are rejected by the initial resolver. Doria toolchain
versions remain CalVer and are never matched by package SemVer constraints.

## `Baton.lock`

`Baton.lock` is machine-generated, deterministic JSON and is never hand-edited.
There is one lockfile at the workspace root; a standalone package is its own
workspace root. Applications, libraries, and workspaces all generate and commit
one for reproducible development and testing. A published library's consumers
resolve its manifest rather than inheriting the author's workspace lock.

Each lock entry records at least:

- lock schema version;
- package identity and exact package version;
- dependency edges and dependency category;
- source transport kind;
- canonical source URL or manifest-relative path;
- exact resolved source identity;
- source-appropriate integrity information;
- optional features and target predicates when those are implemented.

The source descriptor is generic rather than Git-specific. Committed path
entries never contain local absolute paths. Lock ordering is deterministic.

## Build Receipt

Build and machine facts do not belong in `Baton.lock`. A versioned `build.json`
or richer build receipt records exact compiler identity, toolchain version,
target, architecture, profile, compiler flags, native toolchain facts,
generated-source identities, path-dependency content hashes, lock identity, and
build-plan identity. This avoids lockfile churn across platforms, profiles, and
local toolchains.

## Install, Update, And Builds

`baton install` uses an existing lockfile exactly. Without one, it resolves,
writes `Baton.lock`, and fetches required sources.

`baton update` intentionally re-resolves all or selected dependencies, updates
the lockfile, and reports changed versions and sources.

`baton check`, `build`, `run`, and `test` may ensure the locked set is installed,
but they never silently update a valid lockfile.

## Workspaces

A basic workspace has member packages, one `Baton.lock`, one shared dependency
cache, shared build storage, and deterministic member discovery:

```toml
[workspace]
members = [
    "apps/*",
    "packages/*"
]
```

Each member retains its own `Baton.toml`, package identity, autoload mappings,
targets, dependencies, and `internal` boundary. Duplicate identities, duplicate
paths, members escaping the root, dependency cycles, and ambiguous glob results
are rejected. An outside package is an explicit path dependency. Workspace
membership never grants package-internal access.

## Dependency Categories

- `[dependencies]` participates in normal package compilation.
- `[dev-dependencies]` is visible only to tests, examples, benchmarks, and
  development-only tooling.
- `[processors]` contains explicit build tools for the accepted attribute
  processing model.

Processors are explicitly declared, version- and source-locked, visible in
build output, separate from source dependencies, and not automatically visible
to package source. Generated files stay under the build directory. Processors
do not modify handwritten source by default and do not recursively trigger
processor rounds.

Doria packages have no arbitrary `build.doria`, `build.php`, shell hook,
pre-build, post-install, or implicit command hook. Attribute processors are the
controlled extension mechanism. No sandbox or permission guarantee is claimed
until Baton provides one.

## Future Dimensions

The resolver's internal model reserves typed room for optional dependencies,
features, platform and architecture predicates, native libraries, processor
executables, binary tools, and prebuilt artifacts. Their exact public syntax is
deferred until a concrete use requires it. The model does not assume every
dependency is unconditional, target-independent Doria source.

## Cache And Incremental Inventory

Baton uses a global content-addressed dependency cache keyed by exact source
identity and source-appropriate integrity facts. Dependencies are not copied to
a project-local `vendor/` directory by default. A future `baton vendor` command
is separately deferred.

Workspace-local storage owns build plans and receipts, incremental source and
generated-source inventories, compiler-cache references, and target artifacts.
Baton tracks mappings, discovered files, hashes, scopes, dependency roots, and
build-plan inputs without parsing Doria declarations. `doriac` owns symbol and
semantic invalidation.

## Supply-Chain Rules

Dependency and processor sources are explicit. Exact resolutions and remote
source URLs are locked. Secrets are not written to `Baton.lock`. Processor
execution is visible, generated files stay under the build directory, and
dependencies do not modify handwritten source by default.

Offline mode never reaches the network. `baton install --offline` and offline
check/build/run/test use only the lockfile, live path dependencies, cached exact
remote sources, and valid existing generated inputs. Missing locked content is
reported completely; Baton neither substitutes another version nor falls back
to the network.

## Commands

Stage 33 adds `install`, `add`, `remove`, `update`, `fetch`, `tree`, and `why` to
the PHP UX bootstrap. Existing project commands remain `check`, `build`, `run`,
and `test`. This decision does not add future commands before their owning Stage
33 slice or make current diagnostics suggest unavailable commands.

## Stage Ownership

Stage 33 has three implementation slices:

1. **Complete:** Manifest schema 2, schema 1 compatibility, autoload, source
   scopes, targets, deterministic inventory, and single-package build plans.
2. **Complete:** Path and Git resolution, SemVer validation, one-version
   conflicts, strict deterministic `Baton.lock`, dependency commands, global
   cache, offline resolution, multi-package plans, and receipt identities.
3. **Next:** Workspaces, development dependencies, graph commands, incremental inventory,
   `baton test`, explicit processors, generated sources, and Phase F closure.

Stage 32 implements typed attribute metadata and the strict processor protocol
under Decision 0125, not automatic package processor orchestration. Stage 33
Slice 3 performs explicit orchestration, validates responses, writes generated
sources under the build directory, and inserts them into the compiler graph.

All three Stage 33 slices run in `dorialang/baton-php` to validate and freeze
the public contract. They do not promote PHP into Baton's permanent product
identity. Decision 0124's Pre-Stage-45 transition creates the clean
`dorialang/baton` repository, parity-ports every Stage 33 behavior to Doria,
transfers production release assembly, removes the private PHP payload from
archives, and blocks the unsuffixed `2026.03.1` release until cutover.

Stages 31 and 32 and all three Stage 33 slices are complete. Stage 33 and Phase F
are complete, and Stage 34 is next. The native port remains after
Stage 44 so it can consume the accepted filesystem, process, FFI, and network
foundations.

## Safe Deferrals

| Item | Owner | Reopens | Fixed constraint |
| --- | --- | --- | --- |
| Feature syntax | Baton resolver | Concrete package need or Stage 33 follow-up | Features remain typed and lockable |
| Target predicates | Baton resolver | Concrete cross-platform dependency | Predicates remain structured |
| Registry and publishing | Future package ecosystem | Separate accepted design | Source model remains typed and source-neutral |
| Verified archives | Resolver security | Separate supply-chain design | Arbitrary URLs stay rejected |
| Native and binary feeds | Stage 40 and packaging | Their authored stages | Artifact role stays separate from transport |
| Package re-exports | Language/package graph | Separate post-v1 decision | Direct declaration remains required |
| `baton vendor` | Baton CLI | Explicitly scheduled need | Global cache remains default |
| Processor permissions | Processor security | Before untrusted processors | No sandbox is claimed early |
| Workspace selection flags | Baton CLI | Stage 33 Slice 3 | Selection is deterministic |

## Consequences

- Builds can become reproducible without making the lockfile host-specific.
- Direct dependency declarations protect source and public API stability.
- Workspaces coordinate resolution without weakening package isolation.
- The resolver can later accommodate native and binary artifacts without being
  redesigned around Git-only assumptions.
- The bootstrap remains direct and disposable while Stage 33 implements and
  exercises this model; permanent abstractions belong to the Doria-native port.
- The production implementation cannot diverge silently because cutover is
  gated by one shared observable-behavior suite.

## Non-Goals

This decision does not implement schema 2, dependency resolution, network
fetching, `Baton.lock`, workspaces, processors, caches, package commands,
registries, publishing, or migration from schema 1.
