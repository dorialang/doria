# Decision 0117: Namespaces, Compile-Time Autoloading, Hybrid Source Layout, And Package Compilation Graphs

- **Status:** Accepted
- **Accepted:** 2026-08-14
- **Date:** 2026-08-14
- **Implementation status:** Stage 31 Slices 1 and 2 implemented; Stage 31
  Complete; Stage 33 Slice 1 Implemented In The PHP Product-Contract Bootstrap;
  Pre-Stage-45
  Doria-Native Baton Transition Scheduled By Decision 0124
- **Current pipeline:** Stage 30 — Complete; Stage 31 Slice 1 — Complete;
  Stage 31 Slice 2 — Complete; Stage 31 — Complete; Stage 32 — Complete;
  Stage 33 Slice 1 — Complete; Stage 33 Slice 2 — Next;
  E0641, E0671, And E0672 — Historical And Reserved
- **Scope:** Package source discovery, file layout, compilation inputs, package
  visibility, and the Baton-to-compiler boundary
- **Amends:** Decision 0028
- **Implementation ownership:** Decision 0124

## Context

Decision 0028 separated `namespace`, `use`, `include`, and `declare`, but it did
not define how a project discovers source files or how several packages become
one checked compilation graph. That gap would otherwise let filesystem habits,
PHP runtime loading, or an early build-tool implementation define Doria by
accident.

This decision fixes the language and package-graph semantics. Baton and
`doriac` implement these rules at their scheduled stages; neither tool defines
them independently.

## Public Model

- `namespace` gives declarations their logical names.
- `use` lets a source file refer to a declaration by a shorter name.
- `autoload` tells Baton where a package's source files live.
- `include` explicitly adds one required source file at compile time.
- `dependencies` add other packages to the resolved build graph.

The public manifest term is `autoload`. Internal implementations may use names
such as `SourceRoot`, `SourceMapping`, `PackageSourceGraph`, or
`CompilationUnit`, but those names are not public alternatives.

Doria autoloading happens during compilation. There is no runtime Doria source
autoloader. A finished program never searches for, parses, or loads `.doria`
files.

## Namespace Syntax And Resolution

The namespace separator is `\`. `::` remains static member access, `.` remains
concatenation, and `/` remains division. Any name containing `\` is absolute;
Doria has no leading-`\` form and no relative qualified-name form. An
unqualified name follows one resolution chain for every symbol kind and every
type or value position:

```text
explicit imports -> current namespace -> edition prelude
```

File-scope `use` supports individual imports, aliases, and grouped imports:

```doria
use Doria\Std\Math\{Vector2, Vector3, Quaternion};
use Acme\Http\Client as HttpClient;
```

Wildcard imports such as `use Doria\Std\Math\*;` are rejected. They make name
resolution depend on later library additions and can silently collide with
local declarations.

The standard-library root is `Doria\Std`, with PascalCase namespace segments
and folded acronyms such as `Doria\Std\Io`, `Doria\Std\Http`, and
`Doria\Std\Json`. First-party modules outside the standard library use
`Doria\<Module>`.

The compiler injects a small documented prelude rather than a wildcard import.
It contains compiler-known core interfaces, beginning with `Displayable` and
later including `Comparable`, `Equatable`, and `Cloneable`; primitive
companions from `Int` and the fixed-width integer companions through `Float`
and `Bool`; and the core collections `List`, `Dictionary`, and `Set`. Domain
modules such as `Console` and `Vector3` require explicit imports.

Prelude additions are edition-scoped because a new prelude name can collide
with user source. User declarations may shadow ordinary prelude conveniences,
but compiler-known names remain reserved under their owning decisions.

Language intrinsics are resolved before namespace lookup, have no namespace or
prelude entry, and cannot be redeclared. This includes the accepted I/O and
formatting intrinsic family, byte I/O intrinsics, and `panic`. Library objects
such as the future `Doria\Std\Io` stream types remain ordinary namespaced
declarations rather than intrinsics.

Decision 0119 originally permitted the compiler to recognize only the exact
canonical `Doria\Std\Io` checked-error identities before general namespaces
executed. Stage 31 Slice 1 now resolves those identities through the same
general resolver as every other global name. They remain compiler-known and
reserved, with no temporary short aliases or second namespace mechanism.

## Manifest Source Mappings

The canonical main-source table is:

```toml
[autoload.namespaces]
"Acme\\Blog\\" = "src/"
```

The canonical development-source table is:

```toml
[autoload-dev.namespaces]
"Acme\\Blog\\Tests\\" = "tests/"
```

The string mapping is shorthand for the ordinary case. Advanced mappings use
the same operation with filters:

```toml
[autoload.namespaces]
"Acme\\Blog\\" = {
    path = "src/",
    include = ["**/*.doria"],
    exclude = ["**/Fixtures/**"]
}
```

The default file pattern is `**/*.doria`. Baton recursively discovers matching
files, produces a deterministic package source inventory, and gives that
inventory to `doriac`. Every discovered file is parsed, indexed, and checked;
an invalid file cannot hide merely because no reachable declaration uses it.

Mappings require project-relative paths, package-root containment, canonical
path resolution, deterministic ordering, no symlink loops or escapes, no
duplicate canonical file, exact namespace and path casing, and diagnostics for
cross-platform case collisions. If namespace prefixes overlap, the longest
matching prefix governs layout validation. Ambiguous duplicate mappings are
rejected. Filesystem enumeration order is never semantic.

## Source Scopes

### Main

Main sources participate in `check`, `build`, `run`, library compilation, and
normal dependency compilation. They come from `[autoload.namespaces]`.

### Development

Development sources participate in tests, examples, benchmarks, and
development-only tooling. They come from `[autoload-dev.namespaces]`, belong to
the same package for `internal` access, and do not enter normal release
artifacts.

### Generated

Generated roots are injected by Baton or explicitly registered processors.
They belong to one package, live under its build directory, are checked like
handwritten Doria, and participate only in the selected build. Generated
sources do not recursively trigger another processor round. Users do not
maintain a separate generated-autoload table in v1.

Dependency package sources and explicitly included files are also checked in
full whenever their scope is active.

## Hybrid Strict Source Layout

### Namespace Directories

Namespace directories are strict. Given `"Acme\\Blog\\" = "src/"`, a file
declaring `namespace Acme\Blog\Http;` belongs beneath `src/Http/`. Directory
segments match namespace segments exactly, including case.

### Externally Accessible Types

An externally accessible class, enum, interface, trait, attribute class, or
future externally accessible type named `PostController` belongs in
`PostController.doria`. A file has one primary externally accessible type.

Closely related `internal` helper declarations may share that file. Doria does
not require every internal helper to have a separate file.

### Bundles And Entries

Free functions and constants may use descriptive bundle files such as
`responses.doria`, `constants.doria`, or `assertions.doria`; the strict
namespace-directory rule still applies. Generated roots may contain bundle
files with several generated declarations and are exempt from the external-type
filename rule, while still requiring valid namespaces, unique fully qualified
symbols, correct package identity, and ordinary type and ownership checking.

A selected binary entry such as `src/main.doria` is the bounded filename
exception and may also contain top-level executable statements.

## Top-Level Execution

In Baton package mode, only a selected binary target entry file may contain
top-level executable statements. Every autoloaded non-entry file, library file,
development file, generated file, dependency file, and included file is
declaration-only. Each selected binary target has one entry; a library target
has none.

Autoload mapping order, glob order, directory order, filename sorting,
filesystem enumeration, and dependency traversal do not define module
initialization order. Any future source-initialization model requires a separate
decision.

## `include` And `autoload`

`autoload` discovers package sources and constructs the source inventory.
`include` adds one specifically named same-package file.

An include path is a string literal resolved relative to the including source
file. Its canonical target must remain inside the current package root. The
operation is required and include-once: failure to resolve is a compilation
error, and a canonical file reached through several includes or both autoload
and include enters the compilation once. An include may add a same-package file
excluded from normal autoload discovery, and `doriac` reports it as an input.

Cross-package `../` traversal, remote includes, computed paths,
environment-dependent includes, and runtime includes are rejected. Dependencies
are the cross-package mechanism. Autoload is not lowered into hidden source
`include` statements.

## Package Identity And Targets

The canonical publishable identity is lowercase `vendor/package`, for example
`acme/http`. Package identity and Doria namespace identity are separate; a
package named `acme/http` may expose `Acme\Web`. Neither is derived from the
other.

An unscoped short name is allowed only with explicit `publishable = false`.
Baton maps it to the reserved synthetic compiler identity `local/<name>` while
retaining the unscoped manifest name in user-facing output. Decision 0126 owns
the exact spelling without reopening package/namespace separation.

A package may have at most one library target, zero or more binary targets, and
a future PHP-library target under the existing `php-lib` direction. The common
single-binary shorthand remains `kind = "binary"` with one `entry`. Decision
0126 fixes `[targets.library]`, `[[targets.binary]]`, `--library`, and
`--binary <name>` as the additional target and selection spellings.

## Package-Wide `internal`

`internal` means accessible anywhere inside the declaring package and
inaccessible to every other package. The boundary spans files, namespaces,
main and development roots, and generated package sources. Package-owned tests
may use package internals. Dependencies may not.

A workspace never merges the access boundaries of its member packages.
`internal` is not file-private, namespace-private, or workspace-private.

## Namespaces Across Packages

Namespace names are not package ownership claims. Several packages may
contribute distinct fully qualified symbols to the same namespace. A namespace
mapping does not reserve that prefix for one package.

Duplicate fully qualified symbols are compile errors. Diagnostics identify the
symbol, declaration kind, both packages, and both source files. Dependency order
never selects a winner.

## Versioned Build Plan

Baton resolves project structure and emits a versioned JSON build plan. It does
not make `doriac` parse `Baton.toml`. The plan carries explicit semantic inputs,
including:

- schema version and selected target;
- root package identity and package roots;
- source files, scopes, origins, and stable source identities;
- namespace mappings, entries, and generated sources;
- dependency edges and direct dependency identities;
- package `internal` boundaries;
- compiler options, target platform, and build profile.

Exact reversible JSON field names follow implementation conventions, but none
of these facts may remain implicit.

Schema 1 is now the exact compiler-owned contract:

```json
{
  "schemaVersion": 1,
  "edition": "2026",
  "rootPackage": "acme/application",
  "selectedTarget": {
    "package": "acme/application",
    "name": "application",
    "kind": "binary",
    "entrySource": "acme/application:main.doria",
    "activeScopes": ["main"]
  },
  "packages": [
    {
      "identity": "acme/application",
      "root": ".",
      "namespaceMappings": [
        { "prefix": "Acme\\", "path": "src", "scope": "main", "generatedFor": null }
      ],
      "sources": [
        {
          "identity": "acme/application:main.doria",
          "path": "src/main.doria",
          "scope": "main",
          "origin": "entry",
          "generatedFor": null
        }
      ],
      "dependencies": []
    }
  ],
  "compiler": {
    "target": "native",
    "nativeProfile": "fast",
    "targetTriple": null
  }
}
```

The parser rejects unknown schema versions and unknown schema-1 fields. Source
inventory is explicit and authoritative; `doriac` does not scan directories or
parse `Baton.toml`. Binary targets identify one active entry source, while
library targets identify none. Compiler target and profile are plan-owned and
cannot be overridden by CLI flags.

`doriac` owns parsing, declaration indexing, namespace and `use` resolution,
type checking, `internal` and direct-dependency visibility, duplicate-symbol
diagnostics, MIR, code generation, and compiler caches.

Baton owns `Baton.toml`, autoload discovery, dependency and workspace
resolution, source fetching, the lockfile, package cache, generated-root
orchestration, build-plan construction, and project commands. Baton does not
parse Doria declarations. `doriac` does not resolve Git repositories or package
versions.

Standalone `doriac` remains available for explicit files and compiler-owned
build plans. It never requires `Baton.toml` and does not become a package
manager.

## Incremental Boundary

Baton caches manifest parsing, dependency resolution, source fetching,
autoload discovery, source inventory, workspace graphs, and generated source
inventory. `doriac` now provides an in-memory compilation session with stable
build-plan, graph, target, source-content, source-context, declaration-surface,
semantic-dependency, and backend-input fingerprints. It reuses unchanged parsed
syntax and declaration indexes, and invalidates through compiler-owned semantic
and include dependency edges. MIR and backend rebuilding remains conservative.
Persistent compiler cache storage is not part of Stage 31.

The versioned build plan and build receipt are the boundary. Baton does not
duplicate a semantic symbol index, and `doriac` does not rediscover packages.

## Stage Ownership

Stage 31 Slice 1 implements name syntax and resolution, the prelude, group
`use`, `include` grammar, package identity in compiler-facing inputs, and LSP
multi-file symbol identity.

Stage 31 Slice 2 implements the versioned JSON build plan, package/source
identity, multi-file indexing, direct-dependency visibility, hybrid layout
validation, include-once behavior, declaration-only non-entry files, duplicate
symbol diagnostics, cross-file diagnostics, and compiler incremental inputs.

Stage 33 Slice 1 implements Baton manifest schema 2, schema 1 compatibility,
autoload and autoload-dev discovery, scopes, targets, and single-package build
plans in the disposable PHP UX bootstrap. Decision 0118 owns the rest of Stage
33. Decision 0124 requires the later Pre-Stage-45 Doria-native port to preserve
this exact build-plan boundary and observable behavior.

Stage 30 is complete. Stage 31 Slice 1 implements source-preserving qualified
names, namespace/import/include grammar, same-file canonical resolution, the
edition prelude, compiler package/source context, and namespace-aware backend
execution. Stage 31 Slice 2 implements the versioned build plan, complete
package graph, package-wide visibility, strict source layout, include-once,
multi-source diagnostics and lowering, backend execution, and in-memory
incremental inputs. Stages 31 and 32 and Stage 33 Slice 1 are complete; Stage 33
Slice 2 is next. Stage 33 remains In Progress, Not Complete, and freezes the product contract in
`dorialang/baton-php` rather than selecting Baton's permanent implementation
language. The mandatory native transition remains separately scheduled before
Stage 45.

## Safe Deferrals

| Item | Owner | Reopens | Fixed constraint |
| --- | --- | --- | --- |
| Package re-exports | Language and package graph | Separate post-v1 decision | Direct dependency visibility remains the default |
| Module initialization | Language and compiler | Separate decision | Discovery order has no runtime meaning |
| Workspace package-selection flags | Baton CLI | Stage 33 Slice 3 | Selection cannot merge package boundaries |

## Consequences

- Projects gain predictable discovery without a runtime loader.
- File placement becomes portable across case-sensitive and case-insensitive
  systems.
- All active source is checked, so dead code cannot hide invalid programs.
- Package visibility and compiler responsibility remain explicit.
- Stage 31 and Stage 33 can implement one approved model without allowing their
  first backend or bootstrap parser to define it.
- The Pre-Stage-45 port reuses the same versioned plan boundary; it does not
  rediscover source through Doria parsing or reinterpret namespace semantics.

## Non-Goals

Stage 31 implements the compiler-owned portions of this decision. Stage 33 Slice
1 implements Baton manifest schema 2, autoload discovery, targets, source scopes,
and single-package plans under Decision 0126. Dependency resolution, lockfiles,
workspaces, processors, package caches, and registry behavior remain later Stage
33 work in the PHP UX bootstrap. Porting the completed behavior to production
Doria Baton is owned by Decision 0124's Pre-Stage-45 transition.
