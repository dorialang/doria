# ChatGPT Project Context

Use this document when asking ChatGPT, Codex, or another coding agent to work on Doria.

## Project Identity

Doria is a PHP-shaped, C-like compiled programming language. Doria is the language. `doriac` is the compiler.

The compiler is called `doriac`. The current bootstrap implementation of `doriac` is written in Rust, but an early strategic goal is to make `doriac` increasingly self-hosted in Doria.

Doria's long-term target is native machine code and standalone executables. PHP output is a compatibility, migration, debugging, and transpilation backend; it does not define Doria's semantics.

## Product Direction

Doria is intended for areas where PHP developers may want a PHP-like experience but where PHP itself is unsuitable, including native desktop applications, CLI tools, game development, game engines, graphics/multimedia tooling, native library bindings, and future raylib bindings.

This product direction is why Doria cares about native compilation, standalone executables, low-overhead runtime design, C-compatible FFI, predictable performance, binary size, and game/graphics-friendly APIs.

## ChatGPT and Codex Guidance

- Doria is not a Rust language.
- Rust is the current bootstrap implementation language for `doriac`; Doria self-hosting is an early strategic goal.
- Do not frame `doriac` as permanently Rust-bound.
- Do not frame Doria as primarily a PHP transpiler.
- Do not let PHP backend needs shape the parser, AST, semantic model, HIR, MIR, or language semantics.
- Keep native machine code and standalone executables as the long-term compiler target.
- Treat PHP output as a compatibility, migration, debugging, or transpilation backend.
- Keep docs and examples clear that Doria semantics are enforced by `doriac`, not by PHP runtime behavior.

## Settled Decisions

- Language name: Doria.
- Compiler name: `doriac`.
- File extension: `.doria`.
- Compiler bootstrap implementation language: Rust.
- Self-hosting goal: `doriac` should increasingly be writable in Doria itself.
- Long-term primary backend: native machine code.
- Current implemented backend: PHP compatibility/transpilation.
- Current compiler lowering target: HIR.
- Future compiler lowering target: MIR for native-oriented backend work.
- Collection aliases: `List<T>`, `Dictionary<K, V>`, and `Set<T>`.
- Do not use `Vec`.

## Reusable Codex Task Template

```text
You are working on Doria, a PHP-shaped, C-like compiled programming language.

Doria is the language. The compiler is `doriac`. Its current bootstrap implementation is written in Rust, but Doria self-hosting is an early strategic goal.

Keep the compiler architecture backend-independent:

Doria source
-> lexer
-> parser
-> AST
-> semantic analysis
-> type checker
-> readonly/writable checker
-> borrow/lifetime analysis later
-> HIR
-> MIR later
-> backend

Native machine code and standalone executables are the long-term primary target. PHP output is only a compatibility, migration, debugging, and transpilation backend.

Do not imply that Doria is a Rust language, that `doriac` must remain Rust forever, that Doria is primarily a PHP transpiler, or that PHP output defines Doria semantics.
```
