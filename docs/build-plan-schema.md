# Doria Compiler Build-Plan Schema

`doriac` accepts a strict, versioned JSON build plan for complete multi-file
compilation graphs. The plan is resolved compiler input produced by project
tooling. It is not `Baton.toml`, `Baton.lock`, a package manifest, or a build
receipt.

## Schema Versions

`doriac` accepts schema 1 and schema 2. Every object rejects unknown fields,
and `edition` must be `"2026"`. Schema 2 adds the native link facts of
Decision 0148 (section 18, amended 2026-11-20): each package's
`nativeLibraries` and `unsafePolicy`, and the selected target's
`windowsSubsystem`. A schema-1 plan keeps its meaning and must not contain
them; a schema-2 plan requires `unsafePolicy` on every package and
`windowsSubsystem` on a binary target.

## Schema 1

```json
{
  "schemaVersion": 1,
  "edition": "2026",
  "rootPackage": "acme/application",
  "selectedTarget": {
    "package": "acme/application",
    "name": "application",
    "kind": "binary",
    "entrySource": "acme/application:src/main.doria",
    "activeScopes": ["main", "generated"]
  },
  "packages": [
    {
      "identity": "acme/application",
      "root": ".",
      "namespaceMappings": [
        {
          "prefix": "Acme\\Application\\",
          "path": "src/",
          "scope": "main"
        }
      ],
      "sources": [
        {
          "identity": "acme/application:src/main.doria",
          "path": "src/main.doria",
          "scope": "main",
          "origin": "entry"
        }
      ],
      "dependencies": [
        {
          "package": "acme/support",
          "kind": "normal"
        }
      ]
    }
  ],
  "compiler": {
    "target": "native",
    "nativeProfile": "fast",
    "targetTriple": null
  }
}
```

## Schema 2

Schema 2 is schema 1 with these fields:

```json
{
  "schemaVersion": 2,
  "selectedTarget": {
    "kind": "binary",
    "windowsSubsystem": "console"
  },
  "packages": [
    {
      "identity": "acme/application",
      "unsafePolicy": "allow",
      "nativeLibraries": [
        {
          "name": "zlib",
          "link": "dynamic",
          "inputs": [
            { "kind": "library", "name": "zlib" },
            { "kind": "searchPath", "path": "native/windows/lib" }
          ],
          "runtimeFiles": ["native/windows/bin/zlib1.dll"]
        }
      ]
    }
  ]
}
```

`windowsSubsystem` is `"console"` or `"gui"` for a binary target and is absent
for a library target. Other platforms ignore it. A `"gui"` program starts
with no console, and each absent standard stream is the null device (Decision
0148 section 21): writing discards, reading reports end of input, and
`isInteractive` is `false`.

`unsafePolicy` is `"allow"` or `"deny"` (Decision 0148 section 25). Under
`"deny"` every source of the package may contain no unsafe form (E0815), and
a file cannot relax it with `declare(unsafe: "allow")` (E0816).

Each entry of `nativeLibraries` gives the resolved link facts of one logical
library that the package's sources name with `#[Library("name")]`:

- `name`: the logical library name, unique within the package.
- `link`: `"dynamic"` or `"static"`.
- `inputs`: typed inputs, each with a `kind`: `library` (`name`, linked
  through the search paths), `searchPath` (`path`), `framework` (`name`,
  macOS only), or `file` (`path`). At least one input is a `library`,
  `framework` or `file`. Library and framework names use letters, digits,
  `_`, `-`, `+` and `.`, and start with a letter, digit or `_`, so no name
  reads as a linker option.
- `runtimeFiles`: files the linked program loads at run time; may be omitted
  when empty.

A relative path resolves against the package root and must not traverse to a
parent; an absolute path, such as one pkg-config resolved, is accepted because
a plan is machine-local input. Baton parses pkg-config `--libs` output into
these kinds and rejects any other flag, so `doriac` never receives raw linker
arguments.

A source finds the libraries of the package that declares it. Only libraries
that emitted code calls are linked; one that emitted code calls with no entry
reports E0804 (Native Library Has No Link Input), and an input the target
cannot use reports E0805. On Unix a library links as `-L<path>`, `-l<name>`,
`-framework <name>` or the file path; a static library links between
`-Wl,-Bstatic` and `-Wl,-Bdynamic` on Linux, and by the `lib<name>.a` file a
search path holds on macOS, whose linker has no archive preference. With MSVC
a library links as `<name>.lib` after `/LIBPATH:<path>`, for either link mode.
When the program has runtime files it gets an rpath relative to itself
(`$ORIGIN` on Linux, `@loader_path` on macOS), and `doriac compile` and
`doriac run` place the files beside the executable; Windows loads them from
the executable's directory.

## Fields

`rootPackage` and `selectedTarget.package` are canonical package identities.
Every schema requires the selected target to belong to the root package.

`selectedTarget.kind` is `binary` or `library`. A binary requires an active
`entrySource`; a library requires `entrySource: null`. `activeScopes` contains
`main`, `development`, and/or `generated`.

Each package declares its root, namespace mappings, explicit source inventory,
and direct dependencies. Source identities are globally unique within the plan.
Source paths and namespace-mapping paths are package-relative.

Source scopes are:

- `main`: normal package compilation.
- `development`: active only when selected by the target and may use declared
  development dependencies.
- `generated`: explicitly inventoried generated input. It must declare
  `generatedFor` as `main` or `development`. Generated sources normally use
  `origin: "generated"`; a generated source selected as the binary entry uses
  `origin: "entry"` so generated test dispatchers and similar managed entries
  remain both generated-scope inputs and unambiguous selected entry sources.

Source origins are `entry`, `autoload`, `explicit`, and `generated`. `autoload`
records how project tooling selected a source; it does not ask `doriac` to scan
directories.

Dependencies are `normal` or `development`. A source may use only its package,
compiler-known symbols, and direct dependencies permitted by its effective
scope. Packages present only through transitive edges are not visible.

Compiler targets are `debug`, `native`, and `php`. Native plans use
`nativeProfile: "fast"` or `"release"`; debug and PHP plans use `null`.
`targetTriple` is optional resolved target metadata.

## Paths And Layout

Relative package roots resolve from the build-plan directory. Absolute package
roots are accepted because a plan is ephemeral machine-local input. Every named
source and namespace mapping is canonicalized and must remain inside its package
root after following symlinks. Exact filesystem case and portable
case-collisions are validated without discovering additional sources.

Namespace mappings use strict hybrid layout. The longest matching prefix maps a
source namespace to a package-relative directory. Externally accessible types
normally use a matching filename; generated files and the selected entry file
have the documented bounded filename exemptions.

## Include

`include "path.doria";` is required compile-time inclusion. The path resolves
relative to the including source, remains inside the same package, and is added
once by canonical file identity. Included files are checked in full, retain
their own namespace and imports, inherit the including source's effective scope,
and are declaration-only. Include cycles terminate through include-once.

No backend emits runtime include, require, autoload, or source-loading behavior.

## Complete And Partial Graphs

A schema-1 build plan is a complete graph. An unresolved qualified symbol is a
language error. Standalone source and editor APIs may supply a partial graph;
when an owning source is absent, the compiler reports a compiler-input fact
rather than claiming the Doria source is malformed.

## CLI

```bash
doriac check --build-plan build/plan.json
doriac ast --build-plan build/plan.json
doriac hir --build-plan build/plan.json
doriac mir --build-plan build/plan.json
doriac compile --build-plan build/plan.json --out build/application
doriac run --build-plan build/plan.json -- argument-one
```

The plan owns compiler target and native profile selection. CLI `--target` and
`--release` do not override those fields. `--out` remains an invocation-specific
destination.

Without a build plan, `doriac compile` and `doriac run` take the same link facts
through repeatable flags keyed by logical library, and reject them beside
`--build-plan`:

```bash
doriac compile main.doria --link-library zlib=z
doriac compile main.doria --link-library zlib=static:z --link-search zlib=native/lib
doriac compile main.doria --link-framework appkit=AppKit
doriac compile main.doria --link-file zlib=native/lib/zlib.lib --runtime-file zlib=native/bin/zlib1.dll
doriac compile main.doria --windows-subsystem gui
```

A relative flag path resolves against the working directory. Every
`--link-library` of one library states the same link mode.

## Baton Boundary

Baton reads project manifests, discovers configured sources, resolves and fetches
dependencies, manages workspaces and caches, and emits this resolved plan.
`doriac` parses only the build plan: it loads the explicit inventory, resolves
literal includes, builds and checks the package graph, and emits the selected
target. Stage 33 completes Baton schema 2, dependency solving, `Baton.lock`,
processors, package installation, and incremental project inventory without
changing compiler build-plan schema 1. Compiler build-plan schema 2 comes from
Stage 40: Baton resolves `[native-libraries]`, `unsafe` and
`windows-subsystem` from `Baton.toml` into it.

## Baton Requirements

Baton implements its side of Decision 0148 sections 18 to 21 and 25 as
follows. `doriac` already accepts every field below.

1. Emit schema 2 (`schemaVersion: 2`). Every package carries `unsafePolicy`:
   `"deny"` when its `Baton.toml` sets `unsafe = "deny"`, otherwise
   `"allow"`. A binary `selectedTarget` carries `windowsSubsystem` from the
   target's `windows-subsystem`, `"console"` by default; a library target
   omits it.
2. Write `nativeLibraries` per package for the selected platform only, each
   entry `{name, link, inputs, runtimeFiles}`. `system = "z"` becomes a
   `library` input; `library` with `search-paths` becomes `searchPath` inputs
   followed by a `library` input; `framework` becomes a `framework` input
   (macOS). A library with no entry for the platform writes no entry. Paths
   are package-relative without `..` unless absolute. Each entry has at least
   one `library`, `framework` or `file` input, and names match
   `[A-Za-z0-9_][A-Za-z0-9_+.-]*`.
3. Run `pkg-config --libs <name>` for a `pkg-config` entry and map each token:
   `-L<dir>` or `-L <dir>` to `searchPath`, `-l<name>` to `library`,
   `-framework <name>` to `framework`, and an absolute path ending in `.a`,
   `.so` (with or without a version suffix), `.dylib`, `.tbd` or `.lib` to
   `file`. Any other token is rejected with a Baton diagnostic naming the
   token and the logical library.
4. Keep each package's `nativeLibraries` on its own package entry. `doriac`
   links every referenced library across the graph and attributes each input
   to its package.
5. Record in the receipt, per package and logical library, the resolved
   inputs, link mode, runtime files, and the raw pkg-config output and
   version where pkg-config ran. `doriac` places runtime files beside the
   built program, so Baton records them and does not copy them.
6. `baton run` relies on `doriac` having placed the runtime files.
7. Document `[native-libraries]` and `unsafe` in `project-manifest.md`,
   `windows-subsystem` in `targets.md`, schema 2 in `build-plan.md`,
   transitive link inputs in `dependencies.md`, and pkg-config in
   `toolchain.md`.
