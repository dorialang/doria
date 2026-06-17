# Doria Self-Hosting Plan

Doria's compiler, `doriac`, is initially implemented in Rust, but one of the early strategic goals of the language is to make `doriac` increasingly writable in Doria itself.

This is called **self-hosting** or **bootstrapping**.

The goal is not to abandon Rust immediately. Rust is the practical bootstrap language that lets Doria get a reliable compiler, tests, diagnostics, and backend experiments quickly. Over time, Doria should become capable enough to express compiler code, and pieces of `doriac` should be ported into Doria in controlled stages.

---

## 1. Why self-hosting matters

Self-hosting is valuable because it proves that Doria is not only a language for small examples. It proves Doria is capable of building serious software, including its own compiler.

For Doria specifically, self-hosting should help validate:

```text
- the type system
- readonly-by-default semantics
- writable mutation rules
- module/namespacing design
- collections
- error handling
- diagnostics
- file I/O
- string handling
- Doria IR design
- future native-oriented IR design
- backend independence
- native compilation strategy
```

If Doria can eventually implement `doriac`, then Doria is expressive enough for real systems programming and tooling work.

---

## 2. Strategic rule

Doria should be designed with self-hosting in mind, but the project should not rush into self-hosting before the language is ready.

The rule is:

```text
Build doriac in Rust first, but avoid Rust-only architecture decisions that would make a Doria implementation unnecessarily hard later.
```

This means:

```text
- Keep compiler modules clean and portable.
- Avoid unnecessary clever Rust-specific abstractions in core compiler logic.
- Keep data structures explicit and easy to represent in Doria later.
- Keep diagnostics and source handling language-agnostic.
- Design Doria IR and any future native-oriented IR as Doria concepts, not Rust concepts.
- Let Doria's eventual standard library grow toward compiler needs.
```

---

## 3. What self-hosting does not mean yet

Self-hosting is an early strategic goal, but it is not a v0.1 implementation requirement.

Do not attempt full self-hosting before Doria has:

```text
- stable parsing for core syntax
- reliable diagnostics
- real semantic TypeId / TypeKind support
- assignment compatibility checking
- basic return type checking
- path-sensitive control-flow checks for required returns
- modules or namespaces
- enough collection support
- enough string support
- file I/O
- a usable testing story
- a stable Doria IR boundary
```

Avoid creating a huge Doria compiler rewrite too early. That would slow the project down and hide design problems instead of exposing them.

---

## 4. Bootstrapping stages

### Stage 0: Rust bootstrap compiler

Current stage.

`doriac` is written in Rust.

Main goals:

```text
- lexer
- parser
- AST
- semantic checker
- readonly/writable checker
- Doria IR
- future native-oriented IR design
- PHP compatibility backend
- native backend experiments later
```

Rust gives the project a stable foundation while the Doria language is still being designed.

---

### Stage 1: Doria can compile small real programs

Doria should be able to compile non-trivial programs before any compiler rewrite begins.

Minimum capabilities:

```text
- functions
- classes
- methods
- local variables
- readonly/writable checking
- useful diagnostics
- strings
- lists and dictionaries
- simple error handling
- basic modules or namespaces
```

At this stage, Doria is still mostly used for examples and tests.

---

### Stage 2: Doria standard-library and tooling experiments

Before porting compiler stages, write small Doria libraries that a compiler would need.

Possible examples:

```text
- source span helpers
- line/column mapping
- diagnostic formatting
- small string utilities
- collection helpers
- result/error helpers
- test fixtures
```

These are safer to port first because they are small and easy to compare against Rust behavior.

---

### Stage 3: Doria compiler-adjacent modules

Port leaf compiler components into Doria.

Good candidates:

```text
- token definitions
- diagnostic message formatting
- source file abstraction
- simple AST pretty-printer
- Doria IR debug printer
- small parser helper utilities
```

Avoid porting the parser, semantic checker, or backend too early.

---

### Stage 4: Doria frontend experiments

Once the language has enough expressive power, begin implementing a Doria-written frontend experiment.

Possible order:

```text
1. Lexer in Doria.
2. Token stream tests.
3. AST data structures in Doria.
4. Parser for a small Doria subset.
5. Semantic checks for a small Doria subset.
```

The Rust compiler remains the source of truth during this phase.

---

### Stage 5: Doria compiler subset

Create a Doria-written compiler for a deliberately small subset of Doria.

This subset should compile enough of itself to prove the path is realistic.

Target subset:

```text
- simple functions
- basic classes or records
- enums/tagged unions if available
- pattern-like branching or match if available
- strings
- lists
- dictionaries
- Result/Option-style error handling or equivalent
- modules/namespaces
```

This stage should focus on correctness and repeatability, not performance.

---

### Stage 6: First self-hosting chain

A successful bootstrap chain should look like this:

```text
Rust doriac
  -> compiles Doria-written doriac
  -> produced Doria doriac compiles the same Doria-written doriac again
  -> outputs are behaviorally equivalent
```

The exact binary output may not be byte-for-byte identical at first. The important early check is behavioral equivalence:

```text
- same diagnostics for the same invalid programs
- same Doria IR for the same valid programs, where practical
- same backend output for stable test cases
- same test suite results
```

Later, deterministic builds can become a stronger goal.

---

## 5. Design implications

Self-hosting affects many language decisions.

Doria will eventually need good answers for:

```text
- modules and namespaces
- sum types / enums / tagged unions
- pattern matching or equivalent branching
- generic collections
- efficient strings
- file I/O
- error handling
- CLI argument handling
- test framework
- build system integration
- stable compiler diagnostics
```

Do not add these all at once. But when designing them, remember that `doriac` should eventually use them.

---

## 6. Compiler architecture implications

The Rust implementation should be organized so it can be ported gradually.

Prefer clear concepts:

```text
SourceFile
Span
Token
TokenKind
AstNode
TypeRef
TypeId
Diagnostic
SymbolTable
Doria IR
Native-oriented IR
BackendOutput
```

For public architecture, describe the checked compiler-owned representation as Doria IR. A lower native-oriented IR may be introduced later if native codegen needs a simpler representation.

Avoid designs that only make sense because Rust has a specific feature. For example, Rust traits and lifetimes are useful for implementing the bootstrap compiler, but the core compiler model should still be expressible in Doria later.

This does not mean avoiding idiomatic Rust entirely. It means avoiding unnecessary Rust-specific cleverness in places that define Doria's own compiler model.

---

## 7. What should stay in Rust for a long time

Some parts may remain Rust-hosted until Doria is much more mature:

```text
- low-level native backend integration
- linker/toolchain integration
- temporary bootstrap runtime
- performance-sensitive backend code
- platform-specific code
```

Self-hosting can be gradual. It does not require every line of `doriac` to be Doria immediately.

---

## 8. Suggested near-term tasks

Do these before attempting Doria-written compiler modules:

```text
1. Keep Rust doriac modular and well tested.
2. Implement real semantic TypeId / TypeKind support.
3. Implement assignment compatibility checking.
4. Expand basic return checking into path-sensitive control-flow checks.
5. Design modules/namespaces.
6. Design Doria-owned string interpolation.
7. Design error handling for compiler-style code.
8. Stabilize Doria IR enough to print and compare it in tests.
9. Sketch native-oriented IR needs with simple functions and returns.
10. Add a tiny native backend experiment.
```

After that, consider Doria-written utilities.

---

## 9. Self-hosting checklist

Doria is ready to start serious self-hosting work when these are true:

```text
- Doria can compile multi-file programs.
- Doria has a module or namespace system.
- Doria has usable string, list, dictionary, and set support.
- Doria can represent compiler data structures cleanly.
- Doria can report rich diagnostics.
- Doria has enough error handling for parser/checker code.
- Doria has file I/O.
- Doria has a test runner or a practical testing convention.
- The Rust compiler has stable behavior to compare against.
```

Until then, self-hosting should influence design, not dominate implementation.

---

## 10. Guidance for future compiler work

When creating or reviewing Doria work, remember:

```text
- doriac is initially written in Rust.
- doriac should eventually be writable in Doria.
- Do not call Doria a Rust language.
- Rust is the bootstrap language.
- Doria should grow toward being able to implement serious compiler code.
- Keep architecture concepts portable from Rust to Doria.
- Do not rush the rewrite before the language is ready.
```

Useful phrasing:

```text
Doria starts with a Rust bootstrap compiler and should grow toward self-hosting.
```

Avoid phrasing:

```text
Doria is implemented in Rust forever.
```

Better phrasing:

```text
The current bootstrap implementation of doriac is Rust-based, but self-hosting doriac in Doria is an early strategic goal.
```
