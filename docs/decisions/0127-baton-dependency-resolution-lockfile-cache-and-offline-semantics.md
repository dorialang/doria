# Decision 0127: Baton Dependency Resolution, Lockfile, Cache, And Offline Semantics

- **Status:** Accepted
- **Accepted:** 2026-08-28
- **Date:** 2026-08-28
- **Implementation Status:** Implemented By Stage 33 Slice 2
- **Amends:** Decisions 0118, 0124, and 0126
- **Preserves:** Decision 0117's compiler build-plan authority; Decision 0125's attribute processor protocol; schema-1 manifest compatibility; package/namespace separation; direct-dependency compiler visibility; package-wide `internal`; and the mandatory Pre-Stage-45 Doria-native Baton transition

## Context

Decision 0118 fixed the durable dependency, lockfile, cache, and offline model.
Decision 0126 fixed the schema-2 package, target, and selection spellings used by
Stage 33 Slice 1. Stage 33 Slice 2 now exercises the dependency portion of that
product contract in the disposable `dorialang/baton-php` bootstrap.

This record captures the observable behavior proved by that implementation. It
does not make the PHP bootstrap permanent, turn Baton into a compiler, or allow
bootstrap implementation structure to define the later Doria-native design.

## Normal Dependencies

Manifest schema 2 accepts normal dependencies through `[dependencies]`.

```toml
[dependencies]
"acme/database" = { path = "../database", version = "^2.0" }
"acme/http" = {
    git = "https://code.example.com/acme/http.git",
    tag = "v1.4.0",
    version = "^1.4"
}
```

The dependency key is the dependency package's authored `package.name` and must
match the resolved manifest exactly. An unscoped dependency is valid only as a
path dependency whose manifest explicitly sets `publishable = false`; its
compiler identity is `local/<name>`. Git dependencies require scoped package
identities. User-authored identities beginning with `local/` remain reserved.

Each entry selects exactly one source transport. Path entries accept `path` and
an optional `version`. Git entries accept `git`, exactly one of `rev`, `tag`, or
`branch`, and an optional `version`. Unknown or cross-transport fields are
errors rather than ignored future behavior.

`[dev-dependencies]` remains deferred to Stage 33 Slice 3. Slice 2 neither
resolves nor records development dependency edges.

## Package Sources And Versions

Path dependencies are live inputs. Their paths are relative to the declaring
manifest, may traverse to sibling directories, and must resolve to readable,
contained schema-2 packages. They are not copied into a vendor directory or the
global dependency cache. The lockfile stores a portable path relative to the
root manifest using `/` separators and never stores an absolute local path.

Git dependencies accept credential-free `https://` and `ssh://` URLs and one
validated selector. `rev` resolves to one full commit; `tag` and `branch` select
one commit at resolution time. Tags and branches are not package-version
registries. A selected checkout's manifest version, not its ref spelling, is
validated against the dependency's optional SemVer constraint.

The initial constraint language accepts exact versions, caret ranges, tilde
ranges, and bounded comparator ranges. It rejects OR expressions, wildcards,
stability flags, development aliases, empty constraints, and toolchain CalVer.
Prereleases must be requested explicitly.

A dependency package must use manifest schema 2 and edition 2026, match its
authored dependency key, declare a library target, and satisfy source discovery,
layout, and version rules. Binary-only and schema-1 packages cannot be consumed
as dependencies. Dependency compilation selects the declared library and omits
the package's binary entries and development sources.

## Resolution Graph

One resolved graph contains one node for each compiler package identity. Every
dependency chain, constraint, source descriptor, selector, and selected version
contributes to that node. Baton rejects:

- incompatible version constraints or several resolved versions;
- source substitution between path and Git, between Git URLs, or between
  distinct canonical path roots;
- conflicting Git selectors;
- one canonical root claiming several identities;
- every normal dependency cycle.

Conflict diagnostics report all contributing dependency chains. Resolution does
not choose a winner by declaration order, traversal order, lexical order,
newness, or root-package preference. Cycle diagnostics report the complete
deterministic package cycle and remain distinct from compiler include-once
cycles.

## `Baton.lock`

`Baton.lock` uses strict deterministic JSON schema 1. Unknown fields, duplicate
identities, malformed edges, invalid source descriptors, noncanonical ordering,
and inconsistent root/package facts are errors. The lock records package
identity and version, normal dependency edges, source kind, portable path or
canonical credential-free Git URL, declared selector, and the exact Git commit.

The lockfile never records credentials, cache locations, absolute package
roots, build paths, compiler paths, host facts, or path-package content hashes.
Git commits are immutable locked identities. Path dependencies remain live and
are revalidated against their manifests. Lock writes are atomic and
deterministic; manifest edits performed by Baton and their resulting lock update
form one recoverable transaction.

An existing valid lock is used exactly. Install, check, build, and run do not
move tags or branches and do not silently rewrite a valid lock. Missing,
malformed, stale, or manifest-incompatible locks receive precise diagnostics.

## Commands And Update Intent

- `baton install` uses an existing lock exactly, or performs fresh resolution
  and writes the first lock when none exists.
- `baton add` validates and adds one normal dependency, resolves the complete
  graph, and commits the manifest and lock transaction together.
- `baton remove` removes one direct normal dependency and prunes unreachable
  locked packages through the same transaction.
- `baton update` refreshes all dependencies or explicitly selected package
  identities. Unselected Git packages retain exact locked commits; path
  dependencies are always reread because they are live.
- `baton fetch` acquires the exact locked Git content without changing the
  graph or lockfile.

A selected update may introduce or remove transitive packages, but it does not
silently change an unselected pinned package. If the new graph requires that
change, Baton reports the conflict and asks for a broader update.

`baton tree` and `baton why` remain recognized Stage 33 Slice 3 commands. They
are not implemented by Slice 2.

## Cache And Offline Policy

Git content uses one platform-appropriate global cache outside project trees.
The cache is derived from canonical source identity, keeps resolver-owned bare
mirrors, and materializes immutable exact-commit checkouts. Cache publication is
atomic and locked across processes. Cache metadata and checkouts are validated
before use; corruption is diagnosed or safely replaced without trusting mutable
project state.

Offline behavior is one resolver-level network policy shared by every command.
It permits live path dependencies and already cached exact Git commits, and
forbids every network-capable Git operation. Missing offline content is reported
without attempting network access. No project-local vendor directory, registry
resolver, archive resolver, or arbitrary remote archive is introduced.

Resolver-owned Git execution is noninteractive, uses argument vectors rather
than shell command construction, disables prompts, hooks, user configuration,
LFS smudge filters, and submodules, and sanitizes diagnostics. Lockfiles and
diagnostics do not expose credentials.

## Compiler Boundary And Receipts

Baton resolves packages, discovers each package's sources, and emits
multi-package compiler build plans through the existing compiler-owned
build-plan schema 1. The plan preserves package nodes,
normal dependency edges, direct dependency identities, package roots, source
origins, entries, scopes, and package-wide `internal` boundaries. Transitive
packages are not flattened into direct dependencies.

`doriac` does not parse `Baton.toml` or `Baton.lock`, fetch dependencies, or
discover package inventories. Decision 0117's direct-dependency visibility and
package-wide `internal` rules remain compiler authority.

Build receipts record the lockfile SHA-256 and live path-dependency content
fingerprints alongside the existing build-plan and compiler facts. These
machine/build facts do not enter `Baton.lock`, and receipts do not expose cache
paths or absolute dependency roots.

## Slice Boundary And Native Transition

Stage 33 Slice 1 and Slice 2 are complete in the disposable PHP UX bootstrap.
Stage 33 Slice 3 is next and owns development dependencies, workspaces, graph
commands, tests, processors, generated-source orchestration, incremental project
inventory, and Phase F closure. Stage 33 remains In Progress, Not Complete.

Decision 0124's mandatory Pre-Stage-45 transition remains scheduled. The clean
`dorialang/baton` repository must parity-port every accepted Stage 33 behavior,
including this resolver, lockfile, cache, command, security, and offline
contract, before production release ownership transfers or the first
unsuffixed `2026.03.1` toolchain ships.

## Consequences

- Dependency-aware builds are reproducible without making live path packages
  immutable or lockfiles host-specific.
- One-version and source-substitution checks preserve stable Doria type identity.
- Locked Git installs do not drift when a branch or tag moves.
- Offline mode is predictable because network permission is a resolver policy,
  not a command-specific convention.
- The compiler receives an explicit package graph without becoming coupled to
  Baton's manifest or cache implementation.
- Slice 3 behavior remains unavailable rather than being implied by partial
  parser or command support.

## Invalidated Elsewhere

- `docs/doria-end-to-end-plan.md` and `docs/notes/current-pipeline.md`: Stage 33
  Slice 2 is complete, Slice 3 is next, and Stage 33 remains incomplete.
- Decisions 0117, 0118, 0124, 0125, and 0126: Stage 33 implementation status and
  the dependency/processor boundary.
- `README.md` and `SPEC.md`: current Baton dependency, lockfile, cache, offline,
  bootstrap, and native-transition wording.
- Doria authority guards and status-bearing notes: Decision 0127 and the current
  pipeline must be enforced mechanically.
- `dorialang/baton-php`: commit `ad25dfcba2311ebf4e0f4b426612b815ba851293`
  implements and validates this record across its ordinary and private-runtime
  CI matrices.
- `dorialang/doria-language-server`: no compiler pin or semantic change is
  required for this documentation-only authority update.
