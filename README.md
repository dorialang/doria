# Doria

Doria is a new PHP-shaped compiled programming language. It uses familiar PHP syntax such as `$variables`, classes, functions, visibility modifiers, constructor property promotion, and C-like blocks, but it is statically checked before it runs.

The compiler is called `doriac` and is initially implemented in Rust. Doria's long-term primary target is native machine code and standalone executables. PHP is a compatibility, migration, debugging, and transpilation backend; it must not shape the core compiler architecture.

A strategic long-term goal is self-hosting: as Doria matures, more of `doriac` should become writable in Doria itself.

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

Future migration tooling may expose a command such as:

```bash
doriac migrate php src --out migrated
```

That would be a PHP-to-Doria migration converter, not the Doria parser and not the core compiler identity.

## Language principles

- Doria is PHP-shaped, not PHP-compatible at the parser level.
- Valid PHP should be easy to migrate to Doria, but Doria-specific syntax does not need to run directly in PHP.
- Variables must be declared with `let` or an explicit type.
- Bare assignment never declares a variable.
- Bindings, properties, parameters, and `$this` are readonly by default.
- Intentional mutation uses `writable`.
- Collection aliases are `List<T>`, `Dictionary<K, V>`, and `Set<T>`.
- The compiler must reject invalid Doria before lowering to HIR/MIR or emitting backend output.
- Doria may support features PHP cannot express directly, such as executable instance property initializers and richer attribute expressions.

## Design docs

Important project documents:

```text
SPEC.md
ROADMAP.md
AGENTS.md
docs/doria-development-plan.md
docs/self-hosting.md
docs/executable-initializers-and-attributes.md
docs/php-interop-and-migration.md
docs/chatgpt-project-context.md
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
│   ├── doria-development-plan.md
│   ├── executable-initializers-and-attributes.md
│   ├── php-interop-and-migration.md
│   ├── self-hosting.md
│   └── chatgpt-project-context.md
└── examples/
    ├── hello.doria
    ├── variables.doria
    ├── person.doria
    └── errors/
```

The plan originally listed top-level Rust test files. Cargo runs integration tests from the crate that owns the implementation, so the active tests live in `crates/doriac/tests`.
