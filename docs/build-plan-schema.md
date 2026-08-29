# Doria Compiler Build-Plan Schema

`doriac` accepts a strict, versioned JSON build plan for complete multi-file
compilation graphs. The plan is resolved compiler input produced by project
tooling. It is not `Baton.toml`, `Baton.lock`, a package manifest, or a build
receipt.

## Schema 1

Every object rejects unknown fields. `schemaVersion` must be `1` and `edition`
must be `"2026"`.

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

## Fields

`rootPackage` and `selectedTarget.package` are canonical package identities.
Schema 1 requires the selected target to belong to the root package.

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

## Baton Boundary

Baton may read project manifests, discover configured sources, resolve and fetch
dependencies, manage workspaces and caches, and emit this resolved plan.
`doriac` parses only the build plan: it loads the explicit inventory, resolves
literal includes, builds and checks the package graph, and emits the selected
target. Stage 31 does not implement Baton schema 2, dependency solving,
`Baton.lock`, processors, package installation, or persistent compiler caches.
