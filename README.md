# Doria

Doria is a new PHP-shaped compiled programming language. It uses familiar PHP syntax such as `$variables`, classes, functions, visibility modifiers, constructor property promotion, and C-like blocks, but it is statically checked before it runs.

The compiler is called `doriac`. The current bootstrap implementation of `doriac` is written in Rust, but an early strategic goal is to make `doriac` increasingly self-hosted in Doria. Doria's long-term primary target is native machine code and standalone executables. PHP is a compatibility, migration, debugging, and transpilation backend; it must not shape the core compiler architecture.

Doria is also intended for areas where PHP developers may want a PHP-like experience but where PHP itself is unsuitable, including native desktop applications, CLI tools, game development, game engines, graphics/multimedia tooling, native library bindings, and future raylib bindings.

```text
Doria source
-> lexer
-> parser
-> AST
-> semantic analysis
-> type checker
-> readonly/writable checker
-> borrow/lifetime analysis later
-> HIR today
-> MIR later
-> backend
```

Planned backends include native, PHP, debug/interpreter, and WebAssembly. The current working backend is PHP.

## Current status

This repository contains the first working vertical slice of `doriac`:

- Lexes a useful Doria token set.
- Parses a small subset of declarations, classes, functions, statements, and expressions.
- Builds an AST.
- Checks undeclared assignment and readonly/writable mutation rules for locals, properties, `$this`, and writable methods.
- Lowers the checked AST to a small backend-neutral HIR. MIR is planned for native-oriented lowering.
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

`doriac run` is currently a convenience command for the PHP backend: it compiles to a temporary PHP file and runs it with the local `php` binary.

## Editor support

Doria has first-pass editor tooling for `.doria` files:

- `doria-lsp` is a stdio Language Server Protocol binary that reuses the compiler pipeline for diagnostics.
- `editors/vscode/doria` contains a VS Code extension with TextMate syntax highlighting, bracket/comment configuration, and a small built-in LSP client.

Build the server before starting the extension:

```bash
cargo build -p doriac --bin doria-lsp
```

The VS Code extension looks for the server in this order:

```text
1. doria.languageServer.path setting
2. DORIA_LSP_PATH environment variable
3. target/debug/doria-lsp in the open workspace
4. doria-lsp on PATH
```

## Language principles

- Doria is PHP-shaped, not PHP-compatible at the parser level.
- Valid PHP should be easy to migrate to Doria, but Doria-specific syntax does not need to run directly in PHP.
- Variables must be declared with `let` or an explicit type.
- Bare assignment never declares a variable.
- Bindings, properties, parameters, and `$this` are readonly by default.
- Intentional mutation uses `writable`.
- Collection aliases are `List<T>`, `Dictionary<K, V>`, and `Set<T>`.
- The compiler must reject invalid Doria before lowering to HIR/MIR or emitting backend output.
- Rust is the current bootstrap implementation language for `doriac`; Doria self-hosting is an early strategic goal.

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
│   └── doria-development-plan.md
├── editors/
│   └── vscode/
│       └── doria/
└── examples/
    ├── hello.doria
    ├── variables.doria
    ├── person.doria
    └── errors/
```

The plan originally listed top-level Rust test files. Cargo runs integration tests from the crate that owns the implementation, so the active tests live in `crates/doriac/tests`.
