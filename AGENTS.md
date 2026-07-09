# AGENTS.md

## Project

Doria is a compiled programming language for native applications, command-line tools, services, games, and systems software. The compiler is `doriac`, and the current `doriac` is a Rust bootstrap compiler. The native backend and standalone native executables are the primary product target. PHP is only an optional compatibility, migration, debugging, and inspection backend.

A strategic early goal is **self-hosting**: as Doria matures, more of `doriac` should be written in Doria itself. Rust is the bootstrap implementation language, not the permanent identity of the compiler.

Doria is intended for native CLI tools, native desktop applications, game tooling, game engines, graphics/media work, C-library bindings, and future raylib bindings.

The accepted native backend strategy is dual-profile:

```text
Fast native profile       -> Cranelift
Optimized native profile  -> LLVM
```

The fast profile is for local development feedback and smoke builds. The optimized profile is for release/shipping builds. Both profiles must preserve the same Doria-visible semantics for supported code.

Doria may intentionally support features PHP cannot express directly, including executable instance property initializers and richer attribute/metadata expressions. PHP backend limitations must not define Doria semantics.

Doria may eventually include a PHP-to-Doria migration converter, but that converter is a migration tool, not the Doria parser and not the core compiler identity.

The accepted project-tool name is Baton. Baton is the planned user-facing project, package, build, and application orchestration tool. `doriac` remains the compiler.

## Non-negotiable engineering guardrails

- Correctness and accuracy outrank quick demos, fast runnable output, and compatibility shortcuts.
- Design Doria as a native-first language even when a native backend slice is not yet implemented.
- Do not choose syntax, type rules, runtime behavior, standard-library shape, or IR structure because it is easier for the PHP backend.
- Use PHP-shaped syntax, vocabulary, and spelling by default when the choice is only surface familiarity. Stop and ask only when the choice affects Doria-visible semantics, safety, memory/runtime behavior, ABI/layout, Cranelift/LLVM conformance, backend independence, ownership/lifetime behavior, or a costly-to-reverse public design.
- Do not treat generated PHP as a semantic oracle. It is backend output, not the definition of Doria.
- Do not silently import behavior from PHP, Rust, JavaScript, C, C++, or any backend/runtime ecosystem.
- If implementation hits a language-design fork not explicitly settled by `SPEC.md`, `docs/decisions/`, or the current task prompt, stop and ask Andrew.
- When stopping for a design decision, report the question, viable options, tradeoffs, affected files, and a recommendation clearly marked as a recommendation.
- Do not implement a workaround that makes the current backend pass while leaving Doria semantics ambiguous.
- Prefer clear unsupported-feature diagnostics over permissive behavior that may become wrong.
- Preserve the ability to lower Doria to native code safely, even if the immediate task only touches frontend code or a compatibility backend.
- Do not silently rename or replace Baton.
- Do not claim Baton is implemented until it exists.
- Do not turn Baton into a separate compiler or semantic authority.
- Do not present `doriac check` as a mandatory public workflow stage.
- Public onboarding uses write/build/run.
- Compiler-oriented documentation may still document direct `doriac` commands.
- If Baton design encounters an unresolved product or language fork, stop and ask Andrew.

## Decision triage

Stop and ask Andrew only when a decision affects one or more of:

- Doria-visible language semantics
- safety or memory guarantees
- ABI or externally observable data layout
- Cranelift/LLVM conformance
- public APIs that would be costly to reverse
- syntax, type conversions, ownership, destruction, or runtime behavior

For reversible implementation details:

- choose the simplest correct backend-independent option
- explicitly record the assumption
- test it
- proceed without blocking

At completion, report assumptions made and critical decisions encountered. If no critical decision requires Andrew's input, say so directly.

## Working rules

- Treat docs/doria-end-to-end-plan.md as the master execution plan for future work. It answers future-work forks unless Andrew later amends it.
- Treat supporting specification, notes, and decision files as subordinate where they conflict with the end-to-end plan.
- Stage 11 is MIR + interpreter oracle. Stage 11a seeds MIR dumps and the interpreter for a tiny executable subset, Stage 11b adds int expressions and int local slots, Stage 11c adds typed conditions plus structured `if` control flow, and Stage 11d adds structured `while` with nested `break` and `continue`. Stage 11d still excludes MIR `for`, `foreach`, function calls, runtime strings, user-authored bool runtime locals, classes, ownership/borrow checking, doria-rt, LLVM, and NativeSmokeModule deletion. New language capability work should go through MIR/interpreter rather than NativeSmokeModule.
- Do not expand NativeSmokeModule for new language capability slices unless Andrew explicitly approves a temporary compatibility fix. The plan retires it in Stage 11.
- Doria has a real ownership/borrow checker model in Doria spelling: readonly is shared borrow, writable is exclusive borrow, and take transfers ownership.
- public, private, and protected are not Doria visibility keywords. Members are externally accessible by default, internal is the only access-surface keyword, and protected is permanently excluded.
- use is namespace import/alias, uses is trait composition, and with is closure capture. These three keywords are not interchangeable.

- Treat `docs/doria-end-to-end-plan.md`, `docs/decisions/`, `SPEC.md`, `README.md`, `AGENTS.md`, and `docs/information-architecture.md` according to the documentation authority model. Supporting design notes are subordinate to the end-to-end plan and accepted decisions.
- Keep compiler work incremental and tested, but never use incremental delivery as an excuse to make unsound language decisions.
- Do not make PHP the public identity of Doria. PHP is development context, migration context, and one optional compatibility backend; Doria should be described as its own native-first language.
- Do not describe Doria as a Rust language. Rust is only the bootstrap implementation language for the current `doriac`.
- Preserve the public compiler pipeline: lexer -> parser -> AST -> semantic/type checking -> Doria IR -> backend.
- Treat Doria IR as the checked compiler-owned representation. A lowered/native IR may come later for control flow, memory layout, runtime calls, and native backend code generation.
- Do not let PHP backend needs leak into the parser, AST, semantic model, Doria IR, or native-oriented IR design.
- Treat PHP-shaped spellings such as `function`, `class`, `interface`, `trait`, `extends`, `implements`, `namespace`, `use`, `as`, `include`, `declare`, `echo`, `return`, `if`, `else if`, `else`, `while`, `for`, `foreach`, `try`, `catch`, `throw`, `new`, `->`, `::`, `.`, and `#[...]` as the default surface direction unless contradicted by an accepted decision.
- Do not inherit PHP runtime semantics: loose typing, truthiness, `===` / `!==`, dynamic properties, variable variables, `eval`, runtime include behavior, PHP autoloading, PHP arrays as every collection model, PHP references as-is, PHP trait conflict rules, and PHP magic behavior all require deliberate Doria decisions.
- Bigger coherent MVP slices are acceptable when they implement one capability end to end, but they are not permission to skip correctness, semantic checks, tests, backend independence, or documentation.
- For native work, keep the fast Cranelift profile and optimized LLVM profile semantically equivalent for supported code. Differences may be in compile time, optimization, debug information, and binary quality, not Doria behavior.
- Do not let Cranelift or LLVM semantics decide Doria semantics. Backend-specific assumptions must remain behind Doria IR or native-oriented IR lowering.
- Preserve the accepted fixed-width numeric direction: `int` means `int64`, `float` means `float64`, and the accepted explicit numeric spellings are `int8`/`int16`/`int32`/`int64`, `uint8`/`uint16`/`uint32`/`uint64`, and `float32`/`float64`.
- Do not treat `array` as a Doria type. Doria has C-style typed arrays spelled `T[]`, such as `int[] $numbers`; broader collection APIs use `List<T>`, `Dictionary<K, V>`, `Set<T>`, and future named collection types such as `Queue<T>` or `Stack<T>`. `array $items` and `List<array>` are invalid Doria surface syntax. PHP backend output may still use PHP `array` internally when lowering Doria collections.
- Preserve the accepted typed equality and boolean operator direction: `==` and `!=` are typed equality/inequality; `===` and `!==` are not Doria syntax; Doria does not use PHP loose comparison.
- Treat `not` as an exact synonym for `!`, `and` as an exact synonym for `&&`, and `or` as an exact synonym for `||`. Do not import PHP `and` / `or` precedence.
- Treat `xor` as a bool-only, non-short-circuiting boolean exclusive OR. It is not bitwise XOR.
- Treat `&`, `|`, `^`, and `~` as integer bitwise operators. Do not make `&` or `|` boolean aliases, and do not add `^^`.
- Do not add `nand`, `nor`, `implies`, `iff`, or `unless` without a new accepted decision.
- Do not treat the native Stage 2a `0..125` process-exit range as the range of Doria integer values.
- Do not require `main` to return `int`. `main(): void` is valid; falling through or using bare `return;` means successful process status `0`.
- Do not allow `return <expr>;` in `main(): void`; it is a void-return semantic error.
- Treat native string-literal `echo` as a narrow smoke path only. It writes exact literal bytes with no implicit newline; do not lower it through newline-adding helpers such as `puts`.
- Do not claim native strings, string locals, interpolation, stdout formatting, `Console`, runtime I/O, or nonzero runtime error statuses are implemented because string-literal native `echo` works.
- Do not reintroduce `public`, `protected`, or `private` as Doria member visibility modifiers. Doria class members are externally accessible by default; use `internal` for implementation details.
- Keep `writable` and `internal` separate: `writable` controls mutation, while `internal` controls API surface.
- Preserve the accepted namespace/import/include/directive direction: namespaces are semantic symbol ownership, `use` is semantic import/name aliasing, `include` is required include-once compile-time source inclusion, and `declare` is a structured compiler/source directive.
- Do not describe `include` as PHP runtime include, and do not treat `include` as the normal import mechanism.
- Do not confuse `use` with `include`, and do not confuse `use` with Baton package resolution.
- Use `use` only for namespace/file-scope semantic imports and aliases.
- Use `uses` for class-body or trait-body trait composition.
- Do not document or implement class-body `use TraitName;` as accepted Doria; PHP migration should rewrite it to `uses TraitName;`.
- Do not add `require`, `require_once`, or `include_once`; Doria `include` already means required include-once.
- Do not import C/C++ textual macro behavior without an accepted decision. Do not add `#define` or `#undef` macro substitution.
- Do not implement `goto` without a separate accepted decision.
- Do not confuse source/compiler directives with runtime control flow.
- Treat `class`, `interface`, `trait`, `extends`, and `implements` as accepted Doria OOP declaration vocabulary.
- Do not treat accepted PHP-shaped OOP syntax as permission to import all PHP runtime behavior.
- Do not assume PHP magic methods, autoloading, reflection, dynamic properties, or trait conflict rules without accepted decisions.
- Do not confuse namespace/file-scope import `use` with trait-composition `uses`.
- Do not make PHP output the semantic oracle for Doria OOP behavior.
- Do not make Doria editor support VS Code-only. Keep VS Code and IntelliJ / JetBrains syntax highlighting aligned.
- Treat TextMate/editor highlighting as editor UX only, not lexer, parser, compiler, or LSP semantic-token support.
- Use `doria` fences for Doria Markdown examples. Keep `php` fences only for generated PHP, PHP interop, or PHP migration input/output.
- Planned Doria keywords may be highlighted in editor tooling to keep docs readable, but highlighting must never be described as compiler implementation.
- Do not mark rejected Doria syntax such as `===`, `!==`, `#define`, or `#include` as accepted language syntax in editor tooling.
- Keep self-hosting in mind when designing compiler APIs, diagnostics, source management, Doria IR, and the standard library.
- Keep native desktop, game engine, C-library binding, and raylib goals visible when designing Doria IR, future native-oriented IR, runtime, memory representation, FFI, and performance benchmarks.
- Keep Baton architecturally outside the compiler pipeline. Baton may orchestrate projects and invoke `doriac`; it must not duplicate parsing, semantic analysis, type checking, Doria IR lowering, or code generation.
- Keep executable initializers and attribute expressions represented as Doria concepts, not PHP workarounds.
- Keep PHP-to-Doria migration architecturally separate from the Doria parser. The migration tool may parse PHP, but Doria itself should parse Doria.
- Preserve readonly-by-default as the language default. Use class-level ergonomics such as `writable class`/`readonly class` before adding shorter aliases for `writable`.
- Doria is strongly typed in every parameter position. Free functions, methods, constructors, anonymous functions, arrow functions, interface requirements, trait requirements, property hook setters, callbacks, and future function-like forms must show explicit parameter types in docs, examples, tests, fixtures, and implementation grammar. Do not infer omitted parameter types and do not publish untyped arrow-function or anonymous-function parameters.
- Treat basic `if` / `else if` / `else`, `while`, traditional `for`, integer range `foreach`, standalone `++` / `--`, and unlabeled `break;` / `continue;` as MVP control flow. `if` is statement control flow and does not return a value; `when` is the planned value-returning conditional/control construct. `for` is the explicit counter/index loop; `foreach` is preferred for collections and ranges. `0..10` is inclusive, `0..<10` is exclusive-end, range `foreach` bindings are readonly per iteration, and `++` / `--` require writable `int` targets. `break` exits the nearest enclosing loop, and `continue` jumps to the next nearest-loop iteration. Keep `finally`, `do ... while`, `given`, value-returning `when`, `match`, labeled or numeric loop control, value-producing `++` / `--`, and broader control-flow semantics as planned implementation work until their remaining grammar and semantics are specified.
- Treat `throw` / `throws` as the accepted checked-error spelling direction. `throw` raises checked errors, `throws` declares checked thrown error types, and callers must catch or declare thrown errors once implemented. Do not implement checked-error compiler behavior, `try` / `catch`, runtime exception machinery, or `Result<T, E>` as the default Doria error model without a dedicated accepted implementation decision.
- Do not confuse unsupported native backend coverage with invalid Doria. If a construct is valid Doria but unsupported by the current native slice, call it unsupported native backend coverage, especially for `if` without `else`, `else if`, `given`, `finally`, `when`, wider boolean expressions, and broader control-flow shapes.
- Prefer nouns as properties and verbs as methods in Doria APIs and examples. Use property hooks for computed, validated, lazy, or guarded values instead of vague zero-argument noun methods such as `body()`.
- Do not introduce external Rust crates unless they remove real complexity and the repository is ready to manage that dependency.
- Do not add repository utility scripts in Python, JavaScript, shell, or another scripting language out of habit. Prefer Rust for compiler/project tooling and PHP for small repository text/JSON/regex helpers unless a different tool has an explicit, documented advantage for that specific task.

## Global planning and documentation hygiene

- The end-to-end plan is the skeleton.
- Implementation prompts must start from the skeleton, not from local file edits.
- Before generating or executing a prompt, check whether an open PR already covers the work.
- Before adding docs, check `docs/information-architecture.md`.
- Do not create parallel roadmaps.
- Do not patch stale planning docs when deletion or redirection is the correct fix.
- Do not list deleted or superseded docs in "Read first."
- If a file duplicates the end-to-end plan, stop and classify it.
- A clear picture is required before implementation; a complete picture is not required.
- Local MVP work must not undermine the long-term architecture.
- When a design decision affects parser, AST, HIR, MIR, backend, LSP, editor grammar, docs, and tests, plan the full surface area up front, even if implementation is sliced.

Prompt checklist before implementation:

- What stage or decision in the end-to-end plan does this belong to?
- Is there an open PR already doing this?
- Which source-of-truth docs own this topic?
- Which files are active vs historical?
- Is this a local patch or a skeleton-aligned change?
- What future speed bumps will this remove?
- What future work must this avoid duplicating?

## MVP non-goals

- Full PHP compatibility.
- Treating PHP transpilation as a correctness milestone for Doria.
- Complete native code generation and runtime support in the current v0.1 slice.
- LLVM or MLIR integration in the current v0.1 slice.
- Full self-hosting in the current v0.1 slice.
- Full attribute evaluation in the current v0.1 slice.
- PHP-to-Doria migration in the current v0.1 slice.
- Desktop application framework work in the current v0.1 slice.
- Game engine work in the current v0.1 slice.
- Raylib bindings in the current v0.1 slice.
- FFI implementation in the current v0.1 slice.
- Async/await.
- Borrow checking across tasks.
- Interfaces, traits, namespaces, reflection, macros, or package management.
- `Vec` as a collection alias.

Small native backend smoke tests are not ruled out by these non-goals. They are preferred once the required semantics are explicit enough to avoid backend-shaped shortcuts.

## Verification

Run core checks for compiler changes:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --workspace --all-targets --locked --verbose
cargo test --workspace --all-targets --locked --verbose
cargo run -p doriac -- check examples/person.doria
cargo run -p doriac -- hir examples/person.doria
cargo run -p doriac -- compile examples/native/main_return_zero.doria --target native --out build/native/main_return_zero
cargo run -p doriac -- compile examples/native/main_return_42.doria --target native --out build/native/main_return_42
cargo run -p doriac -- compile examples/native/main_void_hello.doria --target native --out build/native/main_void_hello
```

Run documentation and editor guardrails for docs/editor changes:

```bash
php scripts/check_docs_authority.php
php scripts/check_editor_highlighting.php
```

Run backend-specific checks only when the touched task depends on that backend. For the current PHP compatibility backend:

```bash
cargo run -p doriac -- compile examples/person.doria --target php --out build/person.php
```

When a native backend target exists, native smoke tests must be part of the relevant definition of done. The current native target is the Stage 10 Cranelift-backed smoke backend for top-level free functions, exactly one `main`, `main(): int` explicit process status, `main(): void` implicit success, helper functions with `int` parameters and `int`/`void` returns, calls to `int` helpers in supported integer expressions, calls to `void` helpers as statements, final returns, structured returning `if` / `else` and `else if` blocks, guard-style `if` returns with supported fallback blocks, fallthrough `if` statements with visible-local merges, bounded/proven structured `while` loops, bounded/proven traditional `for` loops, bounded/proven integer range `foreach`, supported integer locals, writable integer assignments, fallthrough `if` statements in loop bodies, unlabeled `break` and `continue` inside supported loops, accepted boolean conditions, supported writable integer local assignments, standalone integer `++` / `--`, exact string-literal `echo` stdout, compile-time-known readonly string-local `echo`, and compile-time-known string-concat `echo`; do not treat it as full native code generation, stable native ABI, recursion, string parameters or returns in native, full native mutable-variable support, general native loop support, nested loops, return-bearing loop or fallthrough branch bodies, labeled/numeric loop control, general native strings, interpolation, runtime string concatenation, stdout formatting, runtime I/O, runtime error handling, or full native control-flow support. Stage 7a routes that smoke backend through a private native smoke module boundary; do not treat that private module as public Doria IR, final MIR, or a permanent native storage model.
