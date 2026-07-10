<div align="center">
  <img src="res/images/doria-app-icon-warm.svg" alt="Doria Logo" width="200" height="200">
</div>

# Doria

Doria is a compiled programming language for building native applications, command-line tools, services, games, and systems software with expressive syntax, strong static typing, safe defaults, and modern concurrency.

The compiler is called `doriac`. The current bootstrap implementation is written in Rust, but Rust is not the permanent identity of the compiler. Doria's primary target is native machine code and standalone executables.

A strategic long-term goal is self-hosting: as Doria matures, more of `doriac` should become writable in Doria itself.

## Toolchain Direction

`doriac` is the compiler. Baton is the planned user-facing project, package, build, and application orchestration tool.

The eventual public workflow is write/build/run. Baton is not implemented yet, so direct `doriac` commands remain appropriate for current compiler development and backend smoke testing.

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

Native machine code and standalone executables are the authoritative product direction. The accepted native strategy is a dual-backend path: a Cranelift-backed fast native profile for local development feedback and an LLVM-backed optimized native profile for release builds, with identical Doria-visible semantics across both profiles.

PHP is currently an implemented compatibility, migration, debugging, and inspection backend, but PHP output is not Doria's semantic reference and is not required to be perfect for Doria to succeed.

## Native-first correctness policy

Doria must be designed from its own semantics outward:

```text
Doria semantics -> Doria IR -> backend-specific lowering
```

The project must not choose language behavior because it is convenient for PHP transpilation, Rust implementation details, or any future native backend library. Correctness, safety, and clear semantics outrank quick runnable demos.

When an implementation task exposes a language-design fork, the correct behavior is to stop and ask the language designer. Do not silently choose behavior for syntax, types, runtime semantics, memory behavior, object layout, error handling, string conversion, collections, or standard-library APIs.

## Influence and migration

Doria's surface syntax is intentionally familiar to developers coming from PHP-like and C-like languages, but Doria is its own language. PHP does not define Doria's semantics, and PHP output must not shape the core compiler architecture.

PHP support belongs in compatibility, migration, debugging, and inspection contexts. Future migration tooling may expose a command such as:

```bash
doriac migrate php src --out migrated
```

That would be a PHP-to-Doria migration converter, not the Doria parser and not the core compiler identity.

## Current status

This repository contains the first working vertical slices of `doriac`:

- Lexes a useful Doria token set.
- Parses a small subset of declarations, classes, functions, statements, and expressions.
- Builds an AST.
- Checks undeclared assignment and readonly/writable mutation rules for locals, properties, `$this`, and writable methods.
- Checks assignment compatibility, declared returns, typed equality/inequality, bool-only boolean operators, positional call arguments, constructor init access, control-flow conditions, and string interpolation constraints for the supported subset.
- Lowers the checked AST to Doria IR, the compiler-owned representation used before backend output.
- Lowers the accepted native subset to deterministic MIR functions and basic blocks. The normal debug interpreter has no artificial block or call-depth cap, uses explicit isolated frames, shares exact stdout/stderr bytes without adding a newline, and applies the process-status boundary only to `main(): int`.
- Emits Cranelift-backed native executables from the same MIR. The current subset includes path-sensitive returns, nested control flow, recursion and mutual recursion, integer locals and checked arithmetic, top-level helpers with `int` parameters and `int`/`void` returns, and compile-time-known readonly string expressions for `echo` and `panic`.
- Links the allocation-free bootstrap `doria-rt` static library for process entry, exact native output, panic formatting, and Doria function-name stack traces.
- Runs the durable executable manifest through both the MIR interpreter and Cranelift, comparing exact stdout bytes, stderr bytes, and process status. The retired Stage 7-10 native smoke module is no longer an active compiler path.
- Emits PHP for supported syntax through the optional PHP compatibility backend, including `not`, `and`, `or`, and `xor` lowering that preserves Doria boolean semantics.
- Provides CLI commands and integration tests.

It is intentionally not a complete language yet. Stages 11 and 12 are complete: the interpreter and Cranelift consume one MIR, general supported control flow and recursion share one execution model, and panic behavior is differentially tested. The current compile-time-known string subset does not define heap strings, allocation, layout, ownership, or a stable string ABI. Stage 13 adds the remaining fixed-width integer types and integer operators; string parameters/returns, writable runtime strings, collection iteration, methods/classes, ownership/borrow checking, LLVM, and later runtime capabilities remain future work.

## Quick start

```bash
cargo test
cargo run -p doriac -- --help
cargo run -p doriac -- check examples/native/main_void_hello.doria
cargo run -p doriac -- hir examples/native/main_void_hello.doria
cargo run -p doriac -- mir examples/native/main_void_hello.doria
cargo run -p doriac -- compile examples/native/main_void_hello.doria --out build/native/main_void_hello
./build/native/main_void_hello
```

The currently implemented compatibility backend can also emit PHP for supported syntax:

```bash
cargo run -p doriac -- compile examples/php/person.doria --target php --out build/php/person.php
php build/php/person.php
```

The native backend currently supports the Stage 12 MIR subset: top-level free functions, exactly one parameterless `main`, path-sensitive `return` checking, nested supported control flow, recursion and mutual recursion, integer locals and checked `+`/`-`/`*`, `int` helper parameters, `int`/`void` helper returns, and compile-time-known readonly string expressions for exact-byte `echo` and fatal `panic`. `main(): int` returns an explicit process status in the portable `0..125` range; helper `int` returns are full Doria `int` values. `main(): void` maps normal completion to status `0`. Native compilation never executes user code as a preflight. PHP output remains a compatibility/debugging backend, not the semantic oracle.

```bash
cargo run -p doriac -- compile examples/native/main_return_zero.doria
./main_return_zero

cargo run -p doriac -- compile examples/native/main_void_hello.doria
./main_void_hello

cargo run -p doriac -- compile examples/native/main_string_local_hello.doria
./main_string_local_hello

cargo run -p doriac -- compile examples/native/main_string_concat_hello.doria
./main_string_concat_hello

cargo run -p doriac -- compile examples/native/main_return_42.doria
./main_return_42

cargo run -p doriac -- compile examples/native/main_readonly_local.doria
./main_readonly_local

cargo run -p doriac -- compile examples/native/main_return_arithmetic_42.doria
./main_return_arithmetic_42

cargo run -p doriac -- compile examples/native/main_if_42.doria
./main_if_42

cargo run -p doriac -- compile examples/native/main_if_else_42.doria
./main_if_else_42

cargo run -p doriac -- compile examples/native/main_boolean_condition_42.doria
./main_boolean_condition_42

cargo run -p doriac -- compile examples/native/main_writable_local_42.doria
./main_writable_local_42

cargo run -p doriac -- compile examples/native/main_structured_if_42.doria
./main_structured_if_42

cargo run -p doriac -- compile examples/native/main_if_fallthrough_42.doria
./main_if_fallthrough_42

cargo run -p doriac -- compile examples/native/main_while_42.doria
./main_while_42

cargo run -p doriac -- compile examples/native/main_structured_while_42.doria
./main_structured_while_42

cargo run -p doriac -- compile examples/native/main_for_42.doria
./main_for_42

cargo run -p doriac -- compile examples/native/main_foreach_range_45.doria
./main_foreach_range_45

cargo run -p doriac -- compile examples/native/main_foreach_range_55.doria
./main_foreach_range_55

cargo run -p doriac -- compile examples/native/main_function_add_42.doria
./main_function_add_42

cargo run -p doriac -- compile examples/native/main_function_echo_hello.doria
./main_function_echo_hello

cargo run -p doriac -- compile examples/native/main_function_loop_42.doria
./main_function_loop_42
```

Native compilation lowers MIR to a host object and links it with `doria-rt` through the platform toolchain. This is not a C backend and does not use PHP output. `doria-rt` currently owns process entry, exact stdout/stderr writes, and abort-only panic behavior; it does not introduce runtime strings or heap allocation. Division/modulo, shifts, bitwise operators, labeled or numeric loop control, writable runtime strings, interpolation, native string parameters or returns, methods, static calls, object construction, classes, collections, a stable runtime ABI, and LLVM output remain future work.

## CLI

```bash
doriac check <source.doria>
doriac ast <source.doria>
doriac hir <source.doria>
doriac compile <source.doria> [--out <file>]
doriac compile <source.doria> --target php [--out <file>]
doriac run <source.doria>
```

`compile` defaults to native output and infers an output file name from the input file. `php` is implemented as an explicit compatibility backend. The `debug` target emits a MIR interpreter artifact for inspection, while `wasm` remains planned.

`doriac run` expects a Doria source file, compiles it through the native backend, and runs a temporary executable. To run an executable you already built, run that executable directly, for example `./build/native/main_if_else_42`.

## Editor Support

Doria has first-pass editor tooling for `.doria` files:

- `doria-lsp` is a stdio Language Server Protocol binary that reuses the compiler pipeline for diagnostics, hover, and completion.
- `editors/vscode/doria` contains a VS Code extension with TextMate syntax highlighting, bracket/comment configuration, and a small built-in LSP client.
- `editors/intellij/doria` contains an IntelliJ Platform plugin with `.doria` file recognition, syntax highlighting, editor settings, and `doria-lsp` integration.

VS Code and IntelliJ / JetBrains highlighting should stay aligned as accepted Doria vocabulary evolves. The shared smoke fixture is `editors/fixtures/latest-tokens.doria`, and `scripts/check_editor_highlighting.php` checks the current editor token guardrails.

Syntax highlighting is editor grammar support, not compiler support. Planned keywords may be highlighted so docs and examples are readable before their compiler stages land. Markdown examples that contain Doria source should use the `doria` fence; generated PHP or PHP interop examples should keep the `php` fence. JetBrains Markdown highlighting depends on the IntelliJ plugin registering Doria as a language id that Markdown can inject for `doria` fences, while `.doria` diagnostics, hover, and completion come from `doria-lsp` when configured.

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

- Doria is its own native-first language; PHP is syntax influence, migration context, and optional compatibility backend.
- Valid PHP should be easy to migrate to Doria, but Doria-specific syntax does not need to run directly in PHP.
- Variables must be declared with `let` or an explicit type.
- Bare assignment never declares a variable.
- Bindings, properties, parameters, and `$this` are readonly by default.
- Intentional mutation uses `writable`.
- Class members are externally accessible by default; use `internal` for implementation details that should not be accessed from outside the declaring class.
- `writable` controls mutation. `internal` controls API surface.
- Namespace/file-scope `use` is for semantic imports and aliases; class-body or trait-body `uses` is for trait composition.
- `for` is the explicit counter/index loop. `foreach` is preferred for collections and ranges.
- `0..10` is an inclusive integer range. `0..<10` is an exclusive-end integer range.
- Range `foreach` variables are readonly per iteration and do not leak outside the loop body.
- `++` and `--` require writable `int` targets; value-producing `++`/`--` expressions are future work.
- `throw` raises checked errors, and `throws` declares checked thrown error types in signatures. Compiler behavior for checked errors is future work.
- `Result<T, E>` is not Doria's default surface error model unless a later decision explicitly adopts it.
- Built-in free functions use `snake_case`, for example `get_time()` and `str_starts_with()`.
- Userland free functions and all member-style APIs use `camelCase`, including methods, static/companion APIs, properties, parameters, and named arguments. Examples include `Int::wrappingAdd()`, `$s->isEmpty()`, `$message->tenantId`, and `$message->retryAfter(seconds: 30)`.
- Types and enum cases use `PascalCase`, constants use `SCREAMING_SNAKE_CASE`, and type parameters use single Pascal capitals such as `T`, `K`, and `V`. The inherited magic methods keep `__construct` and `__destruct`.
- Typed arrays are spelled `T[]`, for example `int[] $numbers`.
- Collection aliases are `List<T>`, `Dictionary<K, V>`, and `Set<T>`.
- `array` is not a Doria type spelling; use `T[]` or a named collection type.
- `int` means `int64`, `float` means `float64`, and the accepted fixed-width numeric family is documented in `docs/decisions/0016-fixed-width-numeric-types.md`; compiler support for those explicit spellings is future work.
- The compiler must reject invalid Doria before lowering to Doria IR or emitting backend output.
- The native and debug backends consume the same MIR and the durable parity matrix compares stdout, stderr, and process status. Helper `int` returns are Doria `int` values; the `0..125` range is only a process-exit boundary for explicit `main(): int` status values, not the range of Doria `int`.
- Doria may support features PHP cannot express directly, such as executable instance property initializers and richer attribute expressions.
- If a language behavior is not specified, implementation work should pause for an explicit design decision rather than inventing behavior silently.

## Where Things Live

- Current quickstart and implementation snapshot: `README.md`
- Current language specification: `SPEC.md`
- Master future execution plan: `docs/doria-end-to-end-plan.md`
- Accepted design decisions: `docs/decisions/`
- Historical notes: `docs/notes/`
- Documentation authority model: `docs/information-architecture.md`

Run the documentation authority guardrail after changing docs:

```bash
php scripts/check_docs_authority.php
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
│   ├── decisions/
│   ├── doria-end-to-end-plan.md
│   ├── information-architecture.md
│   ├── notes/
│   ├── executable-initializers-and-attributes.md
│   ├── php-interop-and-migration.md
│   └── self-hosting.md
├── editors/
│   ├── intellij/
│   │   └── doria/
│   └── vscode/
│       └── doria/
└── examples/
    ├── native/
    ├── hello.doria
    ├── variables.doria
    ├── person.doria
    └── errors/
```

The plan originally listed top-level Rust test files. Cargo runs integration tests from the crate that owns the implementation, so the active tests live in `crates/doriac/tests`.
