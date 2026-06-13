# Doria

Doria is a new PHP-shaped programming language. It uses familiar PHP syntax such as `$variables`, classes, functions, visibility modifiers, constructor property promotion, and C-like blocks, but it is statically checked before it runs.

The compiler is called `doriac` and is implemented in Rust. The first backend emits PHP.

```text
Doria source
-> lexer
-> parser
-> AST
-> semantic checker / type checker
-> readonly/writable checker
-> PHP code generator
```

## Current status

This repository contains the first working vertical slice of `doriac`:

- Lexes a useful Doria token set.
- Parses a small subset of declarations, classes, functions, statements, and expressions.
- Builds an AST.
- Checks undeclared assignment and readonly/writable mutation rules for locals, properties, `$this`, and writable methods.
- Emits PHP for supported syntax.
- Provides CLI commands and integration tests.

It is intentionally not a complete language yet. The implementation should grow in small, tested compiler increments.

## Quick start

```bash
cargo test
cargo run -p doriac -- --help
cargo run -p doriac -- check examples/hello.doria
cargo run -p doriac -- compile examples/person.doria --target php --out build/person.php
php build/person.php
```

## CLI

```bash
doriac check <file>
doriac compile <file> --target php --out <file>
doriac run <file>
```

`doriac run` compiles to a temporary PHP file and runs it with the local `php` binary.

## Language principles

- Doria is PHP-shaped, not PHP-compatible at the parser level.
- Valid PHP should be easy to migrate to Doria, but Doria-specific syntax does not need to run directly in PHP.
- Variables must be declared with `let` or an explicit type.
- Bare assignment never declares a variable.
- Bindings, properties, parameters, and `$this` are readonly by default.
- Intentional mutation uses `writable`.
- Collection aliases are `List<T>`, `Dictionary<K, V>`, and `Set<T>`.
- The compiler must reject invalid Doria before emitting PHP.

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
└── examples/
    ├── hello.doria
    ├── variables.doria
    ├── person.doria
    └── errors/
```

The plan originally listed top-level Rust test files. Cargo runs integration tests from the crate that owns the implementation, so the active tests live in `crates/doriac/tests`.
