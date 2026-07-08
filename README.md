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
- Emits Stage 10 Cranelift-backed native smoke executables for top-level free functions plus `function main(): int` and `function main(): void`. `main(): int` returns an explicit process status in the accepted `0..125` portable exit-code range, while `main(): void` may fall through or use `return;` for successful status `0`. The native smoke backend supports Stage 9 integer/control-flow/string shapes plus Stage 10 top-level helper functions with `int` parameters, `int` or `void` returns, calls to `int` helpers in supported integer expressions, and calls to `void` helpers in statement position. Helper function `int` returns are Doria `int` values, not process statuses. Internally, this smoke subset lowers through a private native smoke module before Cranelift lowering; that module is not public Doria IR, final MIR, or a stable ABI.
- Emits PHP for supported syntax through the optional PHP compatibility backend, including `not`, `and`, `or`, and `xor` lowering that preserves Doria boolean semantics.
- Provides CLI commands and integration tests.

It is intentionally not a complete language yet. The implementation should grow in small, tested compiler increments without compromising Doria's native-first semantics. The Doria End-to-End Development Plan is the master future execution plan; the active implementation slice is Stage 11b: MIR integer expressions and locals. Stage 11b extends inspectable MIR and the debug interpreter to readonly/writable int locals, simple writable int mutation, and checked +, -, * arithmetic while preserving Stage 11a literal return and exact string-literal echo coverage. It still does not complete Stage 11: MIR control flow, loops, function calls, classes, runtime strings, ownership/borrow checking, doria-rt, LLVM, and NativeSmokeModule deletion remain future work. NativeSmokeModule still exists temporarily for broader current native smoke coverage, but new language capability work should go through MIR/interpreter rather than expanding it.

## Quick start

```bash
cargo test
cargo run -p doriac -- --help
cargo run -p doriac -- check examples/native/main_void_hello.doria
cargo run -p doriac -- hir examples/native/main_void_hello.doria
cargo run -p doriac -- mir examples/debug/main_void_hello.doria
cargo run -p doriac -- compile examples/native/main_void_hello.doria --out build/native/main_void_hello
./build/native/main_void_hello
```

The currently implemented compatibility backend can also emit PHP for supported syntax:

```bash
cargo run -p doriac -- compile examples/php/person.doria --target php --out build/php/person.php
php build/php/person.php
```

The native backend currently supports a narrow Stage 10 smoke shape: top-level free functions, exactly one top-level `main`, and no top-level statements or classes. `main(): int` returns an explicit process status in the portable `0..125` exit-code range. `main(): void` may fall through or use `return;` and exits successfully with status `0`. Non-`main` helper functions may return `int` or `void`; Stage 10 native parameters are `int` only. The supported source subset includes the Stage 9 integer/control-flow/string shapes, calls to `int` helpers in supported integer expressions, and calls to `void` helpers in statement position. Helper `int` returns are full Doria `int` values and are not constrained by the process-status range. Native validation rejects recursion, mutual recursion, unsupported signatures such as native string parameters or string returns, classes, object construction, methods/static calls, generics, named/default arguments in native, FFI, and broader runtime features. PHP output remains a compatibility/debugging backend, not the semantic oracle.

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

For this slice, native compilation emits an object and links it through the host platform toolchain. This is not a C backend and does not use PHP output. General loops beyond the bounded/proven Stage 9 shapes, nested loops, returns inside loop bodies, `return` inside fallthrough branch bodies, labeled or numeric loop control, division/modulo, native string values beyond compile-time-known readonly string locals and supported `echo`, writable string locals, interpolation, runtime concatenation, native string parameters or string returns, recursion, methods, static calls, object construction, classes, collections, broader runtime features, stable ABI, and LLVM output remain future work.

## CLI

```bash
doriac check <source.doria>
doriac ast <source.doria>
doriac hir <source.doria>
doriac compile <source.doria> [--out <file>]
doriac compile <source.doria> --target php [--out <file>]
doriac run <source.doria>
```

`compile` defaults to native output and infers an output file name from the input file. `php` is implemented as an explicit compatibility backend. `debug` and `wasm` are recognized planned targets.

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
- Typed arrays are spelled `T[]`, for example `int[] $numbers`.
- Collection aliases are `List<T>`, `Dictionary<K, V>`, and `Set<T>`.
- `array` is not a Doria type spelling; use `T[]` or a named collection type.
- `int` means `int64`, `float` means `float64`, and the accepted fixed-width numeric family is documented in `docs/decisions/0016-fixed-width-numeric-types.md`; compiler support for those explicit spellings is future work.
- The compiler must reject invalid Doria before lowering to Doria IR or emitting backend output.
- The native backend currently accepts the Stage 10 smoke subset: top-level free functions, exactly one `main`, Stage 9 supported integer/control-flow/string shapes, `int` helper parameters, `int`/`void` helper returns, `int` helper calls in supported integer expressions, and `void` helper calls as statements. It rejects broader valid Doria with unsupported-feature diagnostics. Helper `int` returns are Doria `int` values; the `0..125` range is only a process-exit boundary for explicit `main(): int` status values, not the range of Doria `int`.
- Doria may support features PHP cannot express directly, such as executable instance property initializers and richer attribute expressions.
- If a language behavior is not specified, implementation work should pause for an explicit design decision rather than inventing behavior silently.

## Design docs

Important project documents:

```text
SPEC.md
AGENTS.md
docs/doria-end-to-end-plan.md
docs/doria-development-plan.md
docs/brand-positioning.md
docs/self-hosting.md
docs/executable-initializers-and-attributes.md
docs/php-interop-and-migration.md
docs/decisions/0010-native-first-correctness.md
docs/decisions/0011-native-execution-path.md
docs/decisions/0012-dual-native-backend-strategy.md
docs/decisions/0013-stage-2-native-integers.md
docs/decisions/0014-baton-project-tool.md
docs/decisions/0015-stage-2b-native-readonly-integer-locals.md
docs/decisions/0016-fixed-width-numeric-types.md
docs/decisions/0017-stage-2c-native-int-arithmetic.md
docs/decisions/0018-stage-2d-native-returned-int-expressions.md
docs/decisions/0019-stage-4a-native-if-else-returns.md
docs/decisions/0032-main-void-and-implicit-success.md
docs/decisions/0033-stage-8a-native-readonly-string-locals-and-echo.md
docs/decisions/0034-stage-9-mvp-iteration-syntax.md
docs/decisions/0035-checked-throw-throws-direction.md
docs/decisions/0036-stage-8-mvp-string-echo-path.md
docs/decisions/0037-stage-10-native-free-functions.md
docs/website-content-guidelines.md
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
    ├── native/
    ├── hello.doria
    ├── variables.doria
    ├── person.doria
    └── errors/
```

The plan originally listed top-level Rust test files. Cargo runs integration tests from the crate that owns the implementation, so the active tests live in `crates/doriac/tests`.
