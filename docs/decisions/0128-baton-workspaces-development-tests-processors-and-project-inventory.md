# Decision 0128: Baton Workspaces, Development, Tests, Processors, And Project Inventory

- **Status:** Accepted
- **Accepted:** 2026-08-29
- **Date:** 2026-08-29
- **Implementation Status:** Implemented By Stage 33 Slice 3
- **Amends:** Decisions 0118, 0124, 0125, 0126, and 0127
- **Preserves:** Decision 0117 compiler graph authority; compiler build-plan schema 1; schema-1 Baton compatibility; package/namespace separation; package-wide `internal`; direct-dependency visibility; Stage 32 metadata and processor protocol version 1; and the mandatory Pre-Stage-45 native transition

## Context

Stage 33 Slices 1 and 2 established Baton's strict schema-2 package model,
targets, source discovery, normal dependency resolution, lockfile, cache, and
offline contract in the disposable PHP bootstrap. Slice 3 completes the Phase F
product contract with canonical source descriptors, workspaces, development
graphs, tests, processors, graph inspection, and one project-inventory protocol
for editor tooling.

Baton owns project discovery and orchestration. `doriac` owns Doria parsing,
semantics, metadata, build-plan validation, and compilation. The language server
consumes Baton's inventory asynchronously; it does not become a second manifest,
dependency, workspace, or processor implementation.

## Dependency Source Descriptors

Manifest schema 2 separates source transport from its locator.

```toml
[dependencies]
"acme/database" = { source = "path", path = "../database", version = "^2.0" }
"acme/http" = {
    source = "git",
    url = "https://code.example.com/acme/http.git",
    tag = "v1.4.0",
    version = "^1.4"
}
```

`source = "path"` requires `path` and forbids Git locators. `source = "git"`
requires `url`, exactly one of `rev`, `tag`, or `branch`, and forbids `path`.
Unknown source transports and cross-transport fields are rejected. The schema-2
spellings `git = "..."` and CLI `--git` are rejected with migration guidance to
`source = "git"`, `url = "..."`, and `--source git --url ...`. Schema 1 remains
byte- and behavior-compatible.

This wording correction does not change lockfile source semantics. Lock schema 1
continues to record transport plus canonical path or URL under its existing
strict fields rather than mirroring TOML names.

## Workspaces

A schema-2 workspace root declares ordered member globs and may either be
virtual or contain its own package. Member discovery is deterministic,
root-contained, symlink-safe, duplicate-free, and sorted by portable relative
path. Every member has exactly one workspace root. Nested workspaces are
deferred and rejected rather than partially interpreted.

Commands at a workspace root require explicit package or target selection when
several valid choices remain. Commands within a member select that member by
location unless an explicit selector says otherwise. The selected package and
target are recorded in build plans, receipts, test plans, and project inventory.
Standalone schema-1 and schema-2 package behavior remains unchanged.

One workspace uses one root `Baton.lock` with strict schema version 2. It records
all member packages and their normal, development, and processor edges while
retaining exact source/version/commit facts. Workspace members do not write
member lockfiles. Standalone projects retain strict lock schema version 1;
schema 1 is never interpreted as schema 2 and schema 2 is not accepted for a
standalone project.

## Development Dependencies

Schema 2 accepts `[dev-dependencies]` using the same source descriptors,
resolution engine, cache, security policy, and version rules as normal
dependencies. Development edges are activated only for development operations,
including tests and development project inventory. They are excluded from
ordinary library consumers and release package graphs.

Normal package source cannot import a development-only package. Development
source may see the package's normal graph plus its direct development
dependencies under Decision 0117's ordinary direct-dependency and package-wide
`internal` rules. Resolution still permits one source and one version for each
compiler package identity across every active edge category.

## Test Discovery And Execution

`baton test` builds a development compilation graph and asks the compiler for
strict metadata schema 2. Tests are compiler-owned `#[Test]` applications whose
target identity resolves to a callable record. An executable test is a
non-generic top-level function with no parameters and `void` return. Methods,
constructors, destructors, parameters, and other supported metadata targets may
carry `#[Test]` as metadata but are not executable tests.

Discovery never parses Doria syntax in Baton. It uses compiler-provided callable
identity, kind, access, generic arity, parameter modes/types, return type,
required and ambient effects, package/source identity, and source location.
Tests execute deterministically with isolated output attribution, ordinary
checked-error/runtime reporting, development dependency activation, and the
same selected compiler/runtime provenance as other Baton operations.

## Metadata Schema 2

`doriac metadata` remains schema 1 by default. Explicit
`--schema-version 1` is byte-identical. `--schema-version 2` contains every
schema-1 field plus a strict deterministic `callables` array. Callable records
use the existing attribute-target identity vocabulary and distinguish global
functions, methods, constructors, and destructors. They preserve exact generic
arity, parameter order/names/types/ownership, return type, access, required and
ambient effects, package, source, and authoritative byte location.

Schema 2 exposes no compiler numeric IDs, Rust types, MIR, backend symbols,
runtime addresses, or reflection. Unknown metadata versions are rejected.
Attribute processor request and response protocols remain strict schema version
1; Baton derives those requests from fields common to the metadata documents.

## Processors

Schema 2 accepts explicit `[processors]` declarations using the same canonical
path and Git source transports as dependencies plus one declared processor
binary target. Processor keys must match package identity. Processor packages
may have normal dependencies, but may not define workspaces, processors, or
activate development dependencies. They are not visible to ordinary Doria
source and lock edges use kind `processor`.

Baton builds the exact locked processor package graph through `doriac` for the
host target. It does not accept PHP, shell snippets, arbitrary executable paths,
or a second source resolver. A processor declaration is explicit authorization
to execute that package binary with the user's account authority. Baton does not claim sandboxing,
network isolation, or permission mediation.

For one package/target build Baton:

1. prepares base metadata without new processor output;
2. invokes `doriac metadata --schema-version 2 --build-plan ...`;
3. constructs one strict processor request version 1 per applicable processor;
4. skips processors with no matching applications;
5. executes each processor at most once for the request;
6. validates strict response version 1 and its graph fingerprint;
7. publishes validated generated sources atomically;
8. rebuilds the final compiler plan with that inventory; and
9. performs no recursive processor pass.

Processor stdout is protocol JSON. Stderr is bounded log text and never parsed
as structured diagnostics. Timeouts, oversized output, malformed JSON, schema or
fingerprint mismatch, unsafe terminal text, compiler-code impersonation, and
nonzero status stop the operation before output enters the graph.

## Generated Sources And Caching

Validated generated source is physically owned beneath:

```text
build/generated/<processor-compiler-package>/<main|development>/<relativePath>
```

Paths remain root-contained and collision-checked against handwritten sources,
other processors, case aliases, and traversal. Publication is transactional.
Only stale files owned by the same processor/request are removed after a
replacement succeeds. Generated source is readonly input to the compiler and
does not trigger another processor pass.

Private processor state lives under `build/.baton/processors/`. Its exact cache
identity includes compiler identity, processor package/source/binary identity,
selected target, request bytes, metadata fingerprint, and relevant lock facts.
Offline mode never builds or launches a processor. It may reuse only an exact,
complete, validated cached result; absent, stale, or corrupt output is an error.

## Graph Inspection

`baton tree` displays the selected resolved graph with normal, development, and
processor edges labelled. `baton why <package>` reports every deterministic path
from the selected root to that package. Both commands operate only on validated
manifest/lock state. They do not fetch, compile, run processors, execute tests,
or contact the network, including when not passed `--offline`.

## Project Inventory

`baton project --json` is the sole public project-inventory boundary. Its strict
schema version 1 reports the workspace/project root, selected package and target,
packages, source roots and exact source files by main/development/generated
scope, namespace mappings, labelled dependency edges, compiler build-plan schema
1, lock/manifest/compiler identities, and deterministic diagnostics.

The command performs no fetch, compile, test, or processor execution. Generated
sources appear only when an exact valid cached processor result already exists.
Private incremental state under `build/.baton/` fingerprints manifests,
lockfiles, package roots, source inventories, metadata, processor requests and
responses, generated files, and compiler identity. Invalidations are precise,
but persisted inventory is an optimization rather than language authority.

The language server invokes this command off the UI thread, consumes only its
strict JSON, indexes unopened files from the returned inventory, watches the
reported manifests/locks/source roots/generated roots, and keeps the last valid
snapshot during transient refresh failures. It never resolves dependencies or
runs processors itself.

## Security And Offline Boundary

All project paths are normalized, root-contained, and symlink-aware. Git and
processor execution use argument vectors rather than shell construction.
Credentials, absolute cache locations, and private host paths are excluded from
public lockfiles, metadata, project JSON, receipts, and diagnostics. Offline
network prohibition remains one resolver-level policy shared by every command.

Graph inspection and project inventory are intrinsically non-executing.
Processor execution is explicit and unsandboxed; documentation must say so.
No implicit build script, processor recursion, arbitrary executable hook, or
editor-triggered processor execution is introduced.

## Phase F And Native Transition

Stage 33 Slice 3 implements this record across the compiler, the
`dorialang/baton-php` product-contract bootstrap, and the language server.
Stage 33 and Phase F are complete after cross-repository and installed-tool
validation. This does not complete Decision 0124's native transition.

Before the first unsuffixed `2026.03.1` release, the clean Doria-native
`dorialang/baton` repository must parity-port the complete Stage 33 behavior and
shared fixture suite. Production archives then remove the PHP bootstrap,
Composer, PHAR, and private PHP runtime. The compiler build-plan and processor
protocol boundaries remain implementation-neutral throughout that transition.

## Consequences

- Workspaces and standalone packages retain distinct strict lock schemas.
- Development code and processors use the same resolver without entering the
  ordinary source visibility graph accidentally.
- Test discovery and editor indexing consume compiler/Baton facts rather than
  reparsing source or manifests.
- Generated source has one transactional owner and cannot recursively schedule
  processors.
- Offline behavior never fabricates or executes missing processor state.
- Phase F gains one portable product contract suitable for the later native
  Baton parity port.

## Invalidated Elsewhere

- Decision 0118's deferred workspace, development, processor, lock, cache, and
  graph-command sections are superseded by this record where they differ.
- Decisions 0124, 0126, and 0127 gain the completed Slice 3 product contract but
  retain the mandatory native-transition gate.
- Decision 0125 keeps processor protocol version 1 while metadata output gains
  additive schema version 2 for callable facts.
- `README.md`, `SPEC.md`, `docs/doria-end-to-end-plan.md`, and
  `docs/notes/current-pipeline.md` record Slice 3, Stage 33, and Phase F as
  complete. Decision 0129 inserts the in-progress Native Testing Foundation;
  Slice 1 is complete, Slice 2 is next, and Stage 34 waits for the foundation.
- `dorialang/baton-php` owns workspace, test, processor, generated-source, graph,
  project-inventory, and security documentation for the bootstrap product.
- `dorialang/doria-language-server` consumes asynchronous `baton project --json`
  inventory while retaining the bounded synthetic open-document fallback.
- The website is a later synchronized documentation task and is not edited by
  this compiler/Baton/tooling slice.
