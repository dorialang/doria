<div align="center">
  <img src="res/images/doria-app-icon-warm.svg" alt="Doria Logo" width="200" height="200">
</div>

# Doria

Doria is a compiled programming language for building native applications, command-line tools, services, games, and systems software with expressive syntax, strong static typing, safe defaults, and modern concurrency.

The compiler is called `doriac`. The current bootstrap implementation is written in Rust, but Rust is not the permanent identity of the compiler. Doria's long-term primary target is native machine code and standalone executables.

A strategic long-term goal is self-hosting: as Doria matures, more of `doriac` should become writable in Doria itself.

```text
Doria source
-> lexer
-> parser
-> AST
-> semantic analysis
-> type checker
-> readonly/writable checker
-> Doria IR
-> backend
```

Native machine code and standalone executables are the primary long-term target. PHP is currently the implemented compatibility, migration, debugging, and inspection backend.

## Influence and migration

Doria's surface syntax is intentionally familiar to developers coming from PHP-like and C-like languages, but Doria is its own language. PHP does not define Doria's semantics, and PHP output must not shape the core compiler architecture.

PHP support belongs in compatibility, migration, debugging, and inspection contexts. Future migration tooling may expose a command such as:

```bash
doriac migrate php src --out migrated
```

That would be a PHP-to-Doria migration converter, not the Doria parser and not the core compiler identity.

## Influence and migration

Doria's surface syntax is intentionally familiar to developers coming from PHP-like and C-like languages, but Doria is its own language. PHP does not define Doria's semantics, and PHP output must not shape the core compiler architecture.

PHP support belongs in compatibility, migration, debugging, and transpilation contexts. Future migration tooling may expose a command such as:

```bash
doriac migrate php src --out migrated
```

That would be a PHP-to-Doria migration converter, not the Doria parser and not the core compiler identity.

## Current status

This repository contains the first working vertical slice of `doriac`:

- Lexes a useful Doria token set.
- Parses a small subset of declarations, classes, functions, statements, and expressions.
- Builds an AST.
- Checks undeclared assignment and readonly/writable mutation rules for locals, properties, `$this`, and writable methods.
- Lowers the checked AST to Doria IR, the compiler-owned representation used before backend output.
- Emits PHP for supported syntax through the PHP backend.
- Provides CLI commands and integration tests.

It is intentionally not a complete language yet. The implementation should grow in small, tested compiler increments.

## Quick start

```bash
cargo test
cargo run -p doriac -- --help
cargo run -p doriac -- check examples/hello.doria
cargo run -p doriac -- hir examples/hello.doria
cargo run -p doriac -- compile examples/person.doria --target php --out build/person.php
php build/person.php
```

## CLI

```bash
doriac check <file>
doriac ast <file>
doriac hir <file>
doriac compile <file> --target <target> --out <file>
doriac run <file>
```

`compile` requires an explicit target. `php` is currently implemented. `native`, `debug`, and `wasm` are recognized planned targets.

`doriac run` is currently a convenience command for the PHP compatibility backend: it compiles to a temporary PHP file and runs it with the local `php` binary.

## Editor Support

Doria has first-pass editor tooling for `.doria` files:

- `doria-lsp` is a stdio Language Server Protocol binary that reuses the compiler pipeline for diagnostics, hover, and completion.
- `editors/vscode/doria` contains a VS Code extension with TextMate syntax highlighting, bracket/comment configuration, and a small built-in LSP client.
- `editors/intellij/doria` contains an IntelliJ Platform plugin with `.doria` file recognition, syntax highlighting, editor settings, and `doria-lsp` integration.

Build the server before starting either editor extension:

```bash
cargo build -p doriac --bin doria-lsp
```

The editor integrations look for the server in this order:

```text
1. Editor setting for the Doria language server path
2. DORIA_LSP_PATH environment variable
3. target/debug/doria-lsp in the open workspace/project
4. doria-lsp on PATH
```

For VS Code, the setting is `doria.languageServer.path`. For IntelliJ IDEs, use the Doria settings page.

## Language principles

- Doria is its own language; PHP is syntax influence, migration context, and compatibility backend.
- Valid PHP should be easy to migrate to Doria, but Doria-specific syntax does not need to run directly in PHP.
- Variables must be declared with `let` or an explicit type.
- Bare assignment never declares a variable.
- Bindings, properties, parameters, and `$this` are readonly by default.
- Intentional mutation uses `writable`.
- Class members are externally accessible by default; use `internal` for implementation details that should not be accessed from outside the declaring class.
- `writable` controls mutation. `internal` controls API surface.
- Collection aliases are `List<T>`, `Dictionary<K, V>`, and `Set<T>`.
- The compiler must reject invalid Doria before lowering to Doria IR or emitting backend output.
- Doria may support features PHP cannot express directly, such as executable instance property initializers and richer attribute expressions.

## Design docs

Important project documents:

```text
SPEC.md
ROADMAP.md
AGENTS.md
docs/doria-development-plan.md
docs/brand-positioning.md
docs/self-hosting.md
docs/executable-initializers-and-attributes.md
docs/php-interop-and-migration.md
```

## Repository layout

```text
.
├── AGENTS.md
├── README.md
├── SPEC.md
├── Cargo.toml
├── crates/
│   └── doriac/
│       ├── Cargo.toml
│       ├── src/
│       └── tests/
├── docs/
│   ├── brand-positioning.md
│   ├── doria-development-plan.md
│   ├── executable-initializers-and-attributes.md
│   ├── php-interop-and-migration.md
│   └── self-hosting.md
├── editors/
│   ├── intellij/
│   │   └── doria/
│   └── vscode/
│       └── doria/
└── examples/
    ├── hello.doria
    ├── variables.doria
    ├── person.doria
    └── errors/
```

The plan originally listed top-level Rust test files. Cargo runs integration tests from the crate that owns the implementation, so the active tests live in `crates/doriac/tests`.
