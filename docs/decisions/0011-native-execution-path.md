# 0011 Native execution path

Status: Proposed

## Purpose

Doria's first native milestone is not "compile the whole language."
The first native milestone is to prove that `doriac` can produce a standalone native executable through a Doria-owned path.

This note proposes that path. It does not implement a native backend, choose a final backend technology, or settle all native runtime details.

The proposed first native smoke target is deliberately tiny:

```doria
function main(): int
{
    return 0;
}
```

This avoids strings, objects, classes, heap allocation, collections, exceptions/errors, a standard library, and runtime complexity. It tests only the ability to take checked Doria through a native-owned path and produce a standalone executable with a process exit code.

## Context

Decision `0010-native-first-correctness.md` establishes that Doria is native-first, that PHP transpilation is optional and non-authoritative, and that backend convenience must not define Doria semantics.

This proposed decision keeps that boundary intact:

```text
Doria source
-> lexer
-> parser
-> AST
-> semantic analysis
-> Doria IR
-> native execution path
```

Generated PHP is not part of the correctness proof for native execution.

## Proposed minimum native slice

Recommended staged path:

```text
Stage 0: Native execution design note only.
Stage 1: Compile `function main(): int { return 0; }` to a native executable.
Stage 2: Add integer literals, local variables, arithmetic, and return values.
Stage 3: Add bool literals, comparisons, `if`, and `while`.
Stage 4: Add direct function calls.
Stage 5: Add minimal stdout/echo for simple string literals.
Stage 6: Add a deliberate string representation.
Stage 7: Add objects/classes/constructors only after object layout is specified.
Stage 8: Add collections only after List/Dictionary/Set representations are specified.
```

Recommendation: keep Stage 1 almost boring. A tiny executable returning `0` gives Doria its first native-owned proof point without accidentally deciding strings, object layout, heap allocation, or standard-library APIs.

Tradeoff: this stage will not be impressive as a demo. That is intentional. It is a correctness milestone, not a language-completeness milestone.

## Entrypoint questions

These are recommendations only until Andrew accepts or revises them.

### Is `main` required for native executables?

Recommendation: require `main` for the first native executable slice.

Reasoning: an explicit entrypoint avoids silently deciding how top-level statements initialize, order, fail, or map to process startup.

### What signatures are allowed for `main` initially?

Recommendation: initially allow exactly one top-level function with this signature:

```doria
function main(): int
```

The return value becomes the process exit code.

Deferred options:

```text
function main(): void
function main(List<string> $args): int
function main(List<string> $args): Result<int, Error>
```

Those options require decisions about process arguments, strings, collections, errors, and exit behavior, so they should not be included in Stage 1.

### Should top-level statements be allowed for native executables?

Recommendation: top-level statements should remain valid Doria for checking, Doria IR work, and compatibility backends, but native executable output should initially require `main`.

Reasoning: top-level statements raise questions about script mode, implicit entrypoint generation, initialization order, and how top-level side effects compose with an explicit `main`.

### How do top-level statements relate to `main`?

Recommendation: do not decide this in the first native slice.

Possible future designs:

```text
1. Native executable mode requires `main` and rejects top-level executable statements.
2. Top-level statements lower into an implicit main when no explicit main exists.
3. Top-level statements run before explicit main as module initialization.
4. Top-level statements remain available only for script/debug/compatibility modes.
```

This is a language-design fork and should be decided separately.

### What is the process exit code?

Recommendation: for Stage 1, the `int` returned by `main` is the process exit code.

Open question: should Doria define exit-code truncation/range behavior explicitly, such as limiting native process exit codes to `0..255` on platforms where that matters, or should the backend map the semantic `int` to the platform convention with diagnostics for out-of-range literals later?

### What happens if there is no `main`?

Recommendation: native executable output should report a clear unsupported-entrypoint diagnostic.

Example wording:

```text
native executable output requires exactly one top-level `function main(): int`
```

This should be a native-backend diagnostic, not a general parse or semantic error, while other backends still support their own valid subsets.

### What happens if there are multiple `main` functions?

Recommendation: report a clear diagnostic for native executable output. The general semantic checker may already reject duplicate top-level functions; the native backend should still guard its own entrypoint selection.

## Primitive representation questions

These choices must be explicit before or during early native implementation. They should not be inherited from PHP, Rust, C, LLVM, Cranelift, or the host machine by accident.

### `int`

Options:

```text
1. signed 64-bit integer
2. target pointer-width integer
3. arbitrary precision integer
4. separate sized integer family later
```

Recommendation: use a fixed-width signed 64-bit `int` for early native semantics unless Andrew decides otherwise.

Tradeoffs:

```text
i64:
  stable cross-platform behavior, simple tests, familiar enough for application code
  but may surprise users expecting machine-word `int`

pointer width:
  convenient for some systems code and host ABI interactions
  but 32-bit and 64-bit targets can behave differently

arbitrary precision:
  safer for overflow and friendlier for some users
  but requires runtime support and is too heavy for the first native smoke path
```

Overflow behavior remains an open question. Stage 1 does not need arithmetic, but Stage 2 does.

### `float`

Recommendation: use 64-bit floating point for early native semantics, unless Andrew decides otherwise.

Tradeoff: `f64` is common and practical, but exact floating semantics, NaN behavior, and formatting should be specified before float-heavy native tests.

### `bool`

Recommendation: treat `bool` as a semantic Doria boolean. The backend may choose an internal machine representation, such as 1 byte or 32 bits, as long as Doria-visible behavior is fixed.

Tradeoff: backend-native booleans are convenient, but ABI exposure and memory layout will matter later for objects, arrays, FFI, and packed data.

### `void`

Recommendation: `void` has no runtime value.

Tradeoff: this is straightforward for functions, but expression-position `void` rules should remain explicit if Doria later adds expression-oriented control flow.

### `null`

Recommendation: defer final native representation until nullable types, object references, and runtime values are specified.

Options include a dedicated singleton value, a tagged value representation, or null pointer representation for reference-like values. The first native smoke target does not need `null`.

### `string`

Recommendation: defer string representation until Stage 6.

String decisions affect literals, interpolation, stdout, ownership, allocation, encoding, slices, FFI, object display hooks, and collections. They should not be smuggled into Stage 1.

## Backend route options

No backend route is accepted by this proposed note. The options below frame the tradeoffs.

| Route | Strengths | Risks |
| --- | --- | --- |
| Cranelift backend | Rust-friendly, practical path to a small native executable, lower integration burden than LLVM, good for fast compiler iteration. | Smaller ecosystem than LLVM, long-term optimization/debug-info story may need revisiting, still a meaningful dependency. |
| LLVM backend | Mature optimization pipeline, broad platform support, object generation, debug-info ecosystem, strong long-term potential. | Heavy dependency burden, larger integration surface, easy to spend time on backend plumbing before Doria semantics are settled. |
| C backend as bootstrap/native portability bridge | Can produce native executables through existing C compilers, useful for portability experiments and simple inspection. | C semantics can leak into Doria if not guarded; object/layout/runtime details still need explicit design; generated C is another backend output, not an oracle. |
| Interpreter/debug backend | Excellent for correctness testing, simple expected-output checks, and backend-independent semantic validation. | Not a standalone native executable by itself; cannot satisfy Stage 1 unless paired with a native wrapper/runtime; may delay first binary if treated as a prerequisite. |

Assessment in Doria terms:

```text
Correctness:
  Debug interpreter is strongest as a semantic test tool.
  Any native backend is acceptable only after semantic checks and Doria IR remain authoritative.

Implementation complexity:
  Debug interpreter and tiny Cranelift smoke backend are likely simpler than LLVM.
  C backend may look simple but shifts complexity into C semantics and toolchain behavior.

Windows/macOS/Linux:
  LLVM and C compilers have broad coverage.
  Cranelift can be practical but target support and object/linking details must be checked.
  Debug interpreter is easiest to run cross-platform but is not the final native target.

Speed to first native executable:
  Cranelift or C are likely fastest.
  LLVM may be slower to integrate.
  Debug interpreter is fastest for execution semantics but not standalone native output.

Long-term optimization potential:
  LLVM is strongest.
  Cranelift may be enough for a long time depending on Doria's goals.
  C inherits host compiler optimization but weakens control over semantics.
  Interpreter is not an optimization path.

Debugging and testability:
  Debug interpreter is strongest.
  C output can be inspected.
  LLVM/Cranelift need good IR dumps and smoke tests.

Dependency burden:
  Debug interpreter has the least dependency burden.
  Cranelift is moderate.
  LLVM is heavy.
  C backend depends on external system toolchains.

Fit with self-hosting:
  A simple Doria-owned interpreter or native-oriented IR can be easier to port later.
  LLVM/Cranelift integration may remain in Rust or a low-level host layer for a long time.
  C output can support bootstrapping experiments but should not define Doria.
```

Recommendation: use a tiny debug interpreter as a correctness aid if it helps, and favor a Cranelift smoke backend for the first standalone native executable if its object/linking path is acceptable. Revisit LLVM and C backend tradeoffs after the native-oriented IR shape is clearer.

This is a recommendation, not a settled decision.

## Doria IR and native-oriented IR boundary

Doria IR remains the checked compiler-owned representation.

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

The first native smoke target:

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

Future native implementation should follow these gates:

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

## Proposed first native tests

These are proposed future tests only. This note does not add native tests.

```doria
function main(): int
{
    return 0;
}
```

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
- native executable requested with unsupported string/object/collection features before their stages
- code rejected by semantic analysis is not lowered to native output
```

## Open questions for Andrew

These need review before implementation:

```text
1. Accept exactly one top-level `function main(): int` for Stage 1?
2. Should `main(): void` ever be allowed for native executables?
3. Should top-level statements become script mode, implicit main, module initialization, or compatibility-only execution?
4. Should Doria define process exit-code range/truncation explicitly?
5. Should early `int` semantics be fixed signed 64-bit?
6. What should integer overflow do once arithmetic lands?
7. Should the first standalone native backend use Cranelift, C, LLVM, or another path?
8. Should a debug interpreter be built before, alongside, or after the first native executable smoke target?
9. What diagnostics should distinguish general semantic errors from backend-unsupported native features?
```

## Non-goals of this proposal

This note does not:

```text
- add Cranelift
- add LLVM
- add a C backend
- add a debug interpreter
- add a native backend module
- change CLI behavior
- change Doria semantics
- add native tests
- change Cargo dependencies
- change PHP backend behavior
```

It only proposes the first native execution path and the questions that should be settled before implementation.
