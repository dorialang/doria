# 0011 Native execution path

Status: Accepted

## Decision

Doria's first native milestone is not "compile the whole language."
The first native milestone is to prove that `doriac` can produce a standalone native executable through a Doria-owned path.

The accepted Stage 1 native executable smoke target is:

```doria
function main(): int
{
    return 0;
}
```

For Stage 1:

```text
- A native executable requires exactly one top-level `function main(): int`.
- The returned `int` is the process exit code.
- Top-level executable statements are not accepted for native executable output yet.
- Top-level statements may remain valid for checking, Doria IR work, and compatibility/debugging backends.
- `main(): void` is not part of Stage 1.
- `main` with parameters is not part of Stage 1.
- Any future argument-passing form is deferred until strings and collections are designed.
- Any future fallible/recoverable-error entrypoint form is deferred until Doria's error model is designed.
```

This decision accepts only the smallest Stage 1 native execution path and a staged Cranelift/LLVM native backend direction. Cranelift is the first native smoke/backend route because it keeps early native iteration small. LLVM is the accepted longer-term optimizing backend path once Doria's native-oriented IR, runtime, object layout, and debug-info needs are clearer. This decision does not settle strings, objects, collections, heap allocation, FFI, standard library, recoverable errors, broad numeric semantics, or every backend integration detail.

## Context

Decision `0010-native-first-correctness.md` establishes that Doria is native-first, that PHP transpilation is optional and non-authoritative, and that backend convenience must not define Doria semantics.

This decision keeps that boundary intact:

```text
Doria source
-> lexer
-> parser
-> AST
-> semantic analysis
-> Doria IR
-> native execution path
```

Generated PHP is not part of the correctness proof for native execution. Backends implement Doria; they do not define it.

## Accepted Stage 1 boundary

Stage 1 exists to answer one question:

```text
Can checked Doria produce a standalone native executable through a Doria-owned path?
```

The answer should be proven with exactly one top-level entrypoint:

```doria
function main(): int
{
    return 0;
}
```

This is deliberately tiny. It avoids strings, objects, classes, heap allocation, collections, exceptions/errors, a standard library, and runtime complexity. It tests only process entry, a Doria semantic integer return value, and process exit-code integration.

The accepted rule is not a full language-wide entrypoint model. It is the first native executable rule.

## Staged native path

Accepted now:

```text
Stage 0: Native execution design note.
Stage 1: Compile exactly one top-level `function main(): int { return 0; }` shape to a standalone native executable whose exit code is the returned int.
```

Recommended later stages, not accepted by this decision:

```text
Stage 2: Add integer literals, local variables, arithmetic, and return values.
Stage 3: Add bool literals, comparisons, `if`, and `while`.
Stage 4: Add direct function calls.
Stage 5: Add minimal stdout/echo for simple string literals.
Stage 6: Add a deliberate string representation.
Stage 7: Add objects/classes/constructors only after object layout is specified.
Stage 8: Add collections only after List/Dictionary/Set representations are specified.
```

The tradeoff is intentional: Stage 1 is not an impressive demo, but it is a clean correctness milestone that does not smuggle in string, object, allocation, runtime, or standard-library assumptions.

## Entrypoint rules

### Is `main` required for Stage 1 native executables?

Accepted: yes. Stage 1 native executable output requires exactly one top-level `function main(): int`.

An explicit entrypoint avoids silently deciding how top-level statements initialize, order, fail, or map to process startup.

### What signatures are allowed for `main` initially?

Accepted for Stage 1:

```doria
function main(): int
```

Not accepted for Stage 1:

```text
function main(): void
function main(List<string> $args): int
```

A future argument-passing entrypoint form may be considered after strings and collections are designed.

A future recoverable-error entrypoint form may be considered after Doria's error model is designed.

### Are top-level statements allowed for native executables?

Accepted for Stage 1: no. Top-level executable statements are not accepted for native executable output yet.

Top-level statements may remain valid for parser/check/Doria IR work and compatibility/debugging backends. That keeps existing language work usable without turning top-level execution into the native startup model prematurely.

### How do top-level statements relate to `main`?

Deferred. Possible future designs include:

```text
1. Native executable mode requires `main` and rejects top-level executable statements.
2. Top-level statements lower into an implicit main when no explicit main exists.
3. Top-level statements run before explicit main as module initialization.
4. Top-level statements remain available only for script/debug/compatibility modes.
```

This remains a language-design question and must not be decided silently by the first backend implementation.

### What is the process exit code?

Accepted for Stage 1: the `int` returned by `main` is the process exit code.

Deferred: exact exit-code range/truncation behavior across operating systems. Stage 1 may map the returned integer through the platform process-exit mechanism, but a later decision should specify diagnostics or behavior for out-of-range literal and computed values.

### What happens if there is no `main`?

Accepted for Stage 1: native executable output reports a clear unsupported-entrypoint diagnostic.

Example diagnostic direction:

```text
native executable output requires exactly one top-level `function main(): int`
```

This should be a native-backend diagnostic, not a general parse or semantic error, while other backends still support their valid subsets.

### What happens if there are multiple `main` functions?

Accepted for Stage 1: native executable output reports a clear diagnostic. The general semantic checker may already reject duplicate top-level functions, but the native backend should still guard entrypoint selection.

## Primitive representation boundary

For the first native smoke target, `int` is treated as a Doria semantic integer suitable for a process exit code.

That is the only accepted primitive representation assumption in this note.

Deferred decisions:

```text
- exact full-width `int` semantics
- integer overflow behavior
- explicit exit-code range/truncation rules
- float semantics
- bool ABI/layout
- void ABI details beyond no returned value for non-Stage-1 entrypoints
- null representation
- string representation
```

Recommendation for the next numeric stage, not accepted here as a full language-wide decision:

```text
Use fixed-width signed 64-bit `int` for early native arithmetic semantics unless Andrew decides otherwise.
Use 64-bit floating point for early `float` semantics unless Andrew decides otherwise.
Treat `bool` as a Doria semantic bool while allowing the backend to choose an internal machine representation, as long as Doria-visible behavior stays fixed.
```

Tradeoffs to revisit:

```text
i64:
  stable cross-platform behavior, simple tests, familiar enough for application code
  but may surprise users expecting machine-word `int`

target pointer width:
  convenient for some systems code and host ABI interactions
  but 32-bit and 64-bit targets can behave differently

arbitrary precision:
  safer for overflow and friendlier for some users
  but requires runtime support and is too heavy for the first native smoke path
```

String representation is deferred until a string-native stage. Strings affect literals, interpolation, stdout, ownership, allocation, encoding, slices, FFI, object display hooks, and collections.

## Backend route status

Accepted: Doria will begin with the smallest standalone native executable smoke target.

Accepted: the native backend direction is a staged Cranelift/LLVM route. Start with Cranelift for the smallest native executable smoke work and early backend iteration. Preserve the Doria IR and native-oriented IR boundary so LLVM can become the longer-term optimizing backend without letting either backend define Doria semantics.

Deferred: exact object/linker integration, runtime packaging, debug-info strategy, and the milestone where LLVM first becomes useful.

This note does not accept a C backend, debug interpreter, PHP backend, or any other route as the native product direction. Those may still be useful auxiliary tools, but they are not competing final backend choices.

Backend route roles:

| Route | Strengths | Risks |
| --- | --- | --- |
| Cranelift backend | Accepted first native smoke/backend route. Rust-friendly, practical path to a small native executable, lower integration burden than LLVM, good for fast compiler iteration. | Smaller ecosystem than LLVM, long-term optimization/debug-info story may need revisiting, still a meaningful dependency. |
| LLVM backend | Accepted longer-term optimizing backend path. Mature optimization pipeline, broad platform support, object generation, debug-info ecosystem, strong long-term potential. | Heavy dependency burden, larger integration surface, easy to spend time on backend plumbing before Doria semantics are settled. |
| C backend as bootstrap/native portability bridge | Possible auxiliary inspection or portability experiment. | C semantics can leak into Doria if not guarded; object/layout/runtime details still need explicit design; generated C is another backend output, not an oracle. |
| Interpreter/debug backend | Possible auxiliary correctness tool for expected-output checks and backend-independent semantic validation. | Not a standalone native executable by itself; cannot satisfy the native product target and should not delay the first binary. |

Implementation direction: build the smallest Cranelift smoke backend first. Keep the IR boundaries backend-independent so LLVM can be introduced later as the optimizing backend. Treat a C backend or debug interpreter as optional support tools, not as product-direction alternatives.

## Doria IR and native-oriented IR boundary

Doria IR is the checked compiler-owned representation.

A later native-oriented lowered IR may be useful for:

```text
- explicit control-flow graphs
- temporaries and storage locations
- lowered expressions
- function call ABI preparation
- process entry/exit integration
- runtime calls
- memory layout
- object allocation
- string and collection operations
- backend emission
```

Do not distort Doria IR to match Cranelift, LLVM, C, PHP, or any one backend. Doria IR should preserve Doria concepts after semantic analysis. Native-oriented IR can be introduced when native code generation needs a simpler representation.

## Runtime boundaries

The Stage 1 native smoke target:

```doria
function main(): int
{
    return 0;
}
```

needs no Doria runtime beyond process entry/exit integration.

It does not need:

```text
- heap allocator
- string runtime
- object layout
- class metadata
- dynamic dispatch
- arrays/collections
- standard library
- error handling
- panic implementation
- destructors
- FFI
- async runtime
```

Runtime needs appear as the native slice expands:

```text
`main(): int { return 0; }`
  needs process entry and exit-code integration.

integer arithmetic and locals
  need value representation, storage, and overflow policy.

`if` and `while`
  need lowered control flow and branch code generation.

direct function calls
  need calling convention decisions and stack/register value passing.

`echo "hello";`
  needs stdout, string literal emission, and a string representation or temporary backend-specific literal path.

`new Person()`
  needs object layout, allocation, constructor lowering, initializer order, readonly initialization rules, and destruction policy.

List/Dictionary/Set
  need collection representation, allocation, ownership/lifetime rules, iteration order decisions, and equality/hash behavior.
```

## Correctness gates for implementation

Future native implementation must follow these gates:

```text
- Parser and semantic checks must pass before native lowering.
- Native lowering must consume checked AST/Doria IR, not raw syntax.
- Native backend must not accept code rejected by semantic analysis.
- Native backend must not rely on PHP output for expected behavior.
- Backend-specific unsupported features must produce clear diagnostics.
- Every native smoke test should compare expected process exit code or stdout directly.
- Native tests should include negative unsupported-feature diagnostics, not only successful output.
- Doria IR dumps should remain available for debugging native lowering.
- Any language-design fork discovered during backend work must stop for Andrew's decision.
```

## Future native tests

These are future implementation tests only. This decision note does not add native tests.

Accepted Stage 1 success shape:

```doria
function main(): int
{
    return 0;
}
```

Recommended later tests, after later stages are accepted:

```doria
function main(): int
{
    return 42;
}
```

```doria
function main(): int
{
    let $x = 20;
    let $y = 22;
    return $x + $y;
}
```

```doria
function main(): int
{
    if (true) {
        return 1;
    }

    return 0;
}
```

Potential early negative tests:

```text
- native executable requested with no `main`
- native executable requested with wrong `main` signature
- native executable requested with top-level executable statements but no accepted native entrypoint
- native executable requested with unsupported string/object/collection features before their stages
- code rejected by semantic analysis is not lowered to native output
```

## Deferred questions for Andrew

These remain open beyond Stage 1:

```text
1. Should `main(): void` ever be allowed for native executables?
2. Should top-level statements become script mode, implicit main, module initialization, or compatibility-only execution?
3. Should Doria define process exit-code range/truncation explicitly?
4. Should early arithmetic `int` semantics be fixed signed 64-bit?
5. What should integer overflow do once arithmetic lands?
6. What exact Cranelift object/linking path should implement the first native smoke backend?
7. At what milestone should LLVM become part of the native backend implementation?
8. Should a debug interpreter be built before, alongside, or after the first native executable smoke target?
9. What diagnostics should distinguish general semantic errors from backend-unsupported native features?
10. What future argument-passing form should `main` use after strings and collections are designed?
11. What future fallible/recoverable-error entrypoint form should exist after Doria's error model is designed?
```

## Non-goals of this decision

This decision does not:

```text
- add Cranelift
- add LLVM
- add a C backend
- add a debug interpreter
- add a native backend module
- change CLI behavior
- change Cargo dependencies
- add native tests
- change PHP backend behavior
- settle every backend integration detail
- settle strings
- settle objects/classes/constructors
- settle heap allocation
- settle collections
- settle FFI
- settle standard library shape
- settle recoverable errors
- settle broad numeric semantics
```

It accepts only the Stage 1 native execution rule and its immediate correctness boundary.
