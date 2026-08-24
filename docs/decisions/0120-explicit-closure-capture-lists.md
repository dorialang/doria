# Decision 0120: Explicit Closure Capture Lists

- **Status:** Accepted
- **Accepted:** 2026-08-15
- **Date:** 2026-08-15
- **Implementation status:** Authority accepted; the pre-Stage-30 grammar slice,
  Stage 30a, Stage 30b semantic capture validation, and Stage 30c ownership,
  lifetime, and escape checking are complete; Stage 30d closure HIR/MIR and
  debug-interpreter execution are complete; Stage 30e native execution, Stage
  30f PHP compatibility, and Stage 30g List Algorithms are complete; Stage 30h
  is next and Stage 30 remains incomplete
- **Scope:** Closure capture spelling, ownership modes, diagnostics, Stage 30
  boundaries, and backend-independent behavior
- **Amends:** D10 and the Stage 30 plan; preserves Decision 0119's callable-effect
  foundation

## Context

The earlier D10 sketch gave arrow functions automatic readonly capture while
anonymous block functions used explicit capture lists. That asymmetry hid
environmental dependencies, made arrow-to-block refactoring change ownership
behavior, and created a special exception for syntax that should share one
closure model. It is superseded by this record.

## Decision

All captures of surrounding local bindings are explicit. Arrow functions and
anonymous block functions use the same `with` capture list and the same ownership
modes. Copy, readonly, writable, and Move bindings receive no implicit-capture
exception. A closure that uses no surrounding local omits `with`; Doria does not
write or recommend `with ()`.

The accepted forms are:

```doria
with ($value)
with (writable $value)
with (take $value)
```

They mean readonly borrow, exclusive writable borrow, and ownership transfer,
respectively. `with` is the closure-capture keyword. `use` remains exclusively a
namespace-import keyword and is not a closure alias.

## Plain-Language Model

Parameters are supplied by the caller. Captures are supplied by the surrounding
scope. The capture list states which surrounding values the closure depends on
and how the closure may use them.

That list exposes environmental dependencies, readonly borrowing, exclusive
writable borrowing, ownership transfer, escape restrictions, and closure
lifetime restrictions. The compiler can discover free variables, but discovery
does not make those dependencies harmless or obvious to a reader.

## Arrow Functions

Arrow parameters are explicitly typed. Their return type may be inferred. An
arrow that refers to an enclosing local must list it after the parameter list and
before `=>`:

```doria
let $minimum = 70;
let $passes = fn(int $score) with ($minimum) =>
    $score >= $minimum;
```

## Anonymous Functions

Anonymous block-function parameters are explicitly typed. An anonymous function
that refers to an enclosing local lists it after any declared return type and
before the block:

```doria
let $minimum = 70;
let $passes = function (int $score): bool with ($minimum) {
    return $score >= $minimum;
};
```

## No-Capture Closures

A closure using only its parameters, own locals, and statically resolved symbols
omits `with`:

```doria
let $double = fn(int $value) => $value * 2;

let $positive = function (int $value): bool {
    return $value > 0;
};
```

An empty capture clause is never required.

## Readonly Capture

`with ($value)` borrows the binding readonly. A closure holding that borrow is
borrow-bound and may not outlive its owner.

## Writable Capture

`with (writable $value)` takes an exclusive writable borrow. Stage 30 validates
that the source is writable, the closure's use agrees with the written mode, and
ordinary one-writer-XOR-many-readers rules hold.

## Taking Capture

`with (take $value)` transfers ownership into the closure environment. The source
binding is moved and ordinary use-after-move checking applies. The environment
acquires the value's cleanup obligation; there is no hidden clone or share.

## Copy Values

Copy values still require an explicit capture when they come from an enclosing
local scope. Copy semantics affect transfer cost and post-copy usability; they do
not make an environmental dependency implicit.

## Move Values

Move values still require an explicit capture. A readonly or writable capture
borrows the value; a taking capture owns it. Stage 30 must preserve the existing
Move, borrowing, and deterministic-cleanup rules.

## No Implicit Capture

There is no automatic capture for arrows, anonymous functions, Copy locals,
readonly locals, or Move locals. This rule is closed and may not be reopened by
the remaining Stage 30 design work.

## Refactoring Stability

Changing an arrow closure into an anonymous block closure must not change its
capture behavior. The capture list and each ownership mode remain explicit and
unchanged; only the body form and, where desired, the return annotation change.

## Names That Require Capture

A closure must capture every referenced binding from an enclosing lexical scope,
including readonly or writable Copy locals, readonly or writable Move locals,
outer function/method/constructor parameters, enclosing-block locals, pattern
bindings, catch bindings, and `given` bindings.

The required mode depends on use. This record fixes the written modes but does
not prematurely fix Stage 30's complete mode-validation algorithm.

## Names That Do Not Require Capture

A closure does not capture its own parameters or locals, top-level functions,
constants, static methods, static properties, type names, enum cases, or other
statically resolved symbols independent of an enclosing local binding. Capture
analysis must distinguish lexical bindings from ordinary symbol resolution.

## Capture-Specific Diagnostics

An uncaptured outer local is not merely unknown or undeclared. The canonical
diagnostic direction is:

```text
Closure Must Capture `$minimum`

This closure uses `$minimum` from the surrounding scope, but the binding is not
listed in its capture clause.

Help:
Add `with ($minimum)`.
```

For an arrow, a structured fix inserts the clause between the parameter list and
`=>`. For an anonymous function, it inserts the clause before the block. When a
capture list already exists, the fix adds the missing capture only if the mode is
unambiguous, source order and formatting are preserved, and no duplicate is
introduced. Otherwise the compiler gives help without an unsafe machine edit.
Stage 30 allocates the diagnostic identity when implementation requires it.

## Ownership Consequences

Readonly and writable captures retain their owner provenance and lifetime. A
borrow-capturing closure cannot escape its source; when transfer is appropriate,
the diagnostic should suggest a taking capture. Taking captures move their
sources, and environment cleanup follows normal reverse-acquisition and
structured-exit rules. Closure values remain Move types.

## Callable Effects

Decision 0119 owns source-ordered checked-effect sets on callable signatures and
subset-effect substitution. Decision 0121 applies that model to closure function
types: function types write `throws`, closure bodies infer effects after local
catch subtraction, and closure expressions do not gain effect annotations.

## Collection Algorithms

Decision 0100 grants `map`, `filter`, and `reduce` to `List<T>` at Stage 30. Their
callbacks use this exact closure model. The audit found no accepted grant of
those algorithms to `Dictionary`, `SortedDictionary`, `Set`, `SortedSet`,
`Deque`, `T[]`, `Iterable`, or a shared algorithm interface, so this record adds
none. Result shapes, traversal contracts, callback ownership, dictionary entry
shapes, and any wider shared algorithm surface remain unresolved until accepted
separately.

## Pre-Stage-30 Grammar Slice

The repository's two-clocks rule requires accepted syntax to parse before its
semantic execution stage. The bounded grammar slice is complete: it owns `fn`
and anonymous-function expression tokens and productions, explicit parameter
and return syntax, `function(T): R` type syntax, `with` capture-list syntax,
capture-mode syntax, source-preserving AST nodes, parser recovery, and
accepted-syntax regression tests. Stage 30b now validates free variables,
capture modes, `$this`, inferred invocation modes, and checked effects. Valid
closure source passes target-neutral checking. Stage 30c acquires captures,
tracks closure Move state and capture leases, validates escape and storage, and
records logical release plans. Stage 30d lowers closures and callable-value calls
to explicit HIR/MIR, validates the structural function carrier and environment,
and executes them through the debug interpreter. Stage 30e executes the same
validated MIR through Cranelift and LLVM. Stage 30f emits the PHP compatibility
route from semantic and validated MIR closure plans. Stage 30g executes
`List<T>::map`, Copy-only `filter`, and writable-accumulator `reduce` through
compiler-owned algorithm plans. `E0641` is catalogued but unreachable for the
supported Stage 30g surface until Stage 30h.

No collection algorithm was part of the grammar foundation or Stage 30d. Stage
30g has separate durable native and PHP parity fixtures; documentation snippets
remain distinct from executable fixtures.

## Stage 30 Ownership

Stage 30 consumes the source-preserving closure AST and owns explicit capture
validation, environment ownership and representation, function types, closure
calls, checked-effect integration, borrow-bound escape checking, accepted
closure-based collection algorithms, interpreter/Cranelift/LLVM execution, PHP
compatibility, language-server semantic integration, and website/playground
activation.

The later structured-concurrency stage owns async closures, spawned closures,
task-group closures, cross-task captures, `Sendable`, `Shareable`, and concurrent
writable capture. Stage 35 owns general interface conformance and any erased
interface interaction needed by closure types. Stage 41 owns the PHP bridge.

## Stage 30 Elaboration

Decision 0121 settles the questions this record deliberately left bounded.
`$this` is explicit through `with ($this)` or `with (writable $this)`; taking the
borrowed receiver is rejected. That record also owns mode validation, inferred
closure effects, the two-word runtime carrier, lifetime and escape rules, and the
Stage 30a through Stage 30h implementation sequence. None of those elaborations
reopens explicit capture for ordinary local bindings.

## Performance

Explicit syntax adds no runtime cost compared with compiler-discovered capture.
The compiler builds only the environment required by listed captures. Logical
capture order is source order; Decision 0121 permits private physical field
reordering while preserving logical acquisition and destruction. No-capture
closures use a zero-environment representation. Readonly and writable captures
use borrow-compatible entries; taking captures store owned values and cleanup
obligations. No hidden clone, share, or runtime reflection occurs, and closure
existence alone does not require heap allocation. Stage 30 and Stage 35a must
prove executable behavior; escape analysis may stack-allocate or eliminate
nonescaping environments. This record does not promise that every closure is
allocation-free.

## PHP Compatibility

Generated PHP follows Doria's written capture list through compiler-generated
carrier and environment objects. Stable places are selected by canonical
binding identity. The backend does not use PHP arrow automatic capture or
`use (...)` as an ownership model, and PHP references do not define Doria
borrowing. Readonly, writable, and taking behavior follows Doria ownership
semantics; backend helper identity is unobservable and no runtime reflection
discovers captures.

## Self-Hosting

The Doria-written compiler must be able to inspect explicit capture lists.
Compiler code therefore exposes environment dependencies and ownership without
requiring source-body scanning merely to understand a closure's contract. The
Rust bootstrap representation does not define the language model.

## Consequences

- Arrows and block closures now have one stable ownership contract.
- Source review and refactoring expose all local environmental dependencies.
- Stage 30 diagnostics can distinguish missing capture from missing name.
- No-capture closures remain concise.
- Decision 0121 settles `$this`, effect inference, ABI, and implementation
  sequencing without weakening explicit local capture.

## Affected Components

The pre-Stage-30 lexer/parser/AST grammar slice; Stage 30 HIR/MIR work, semantic
name and capture checking, ownership and borrow analysis, diagnostics and
structured fixes, function types and checked effects, `List` closure algorithms,
interpreter, Cranelift, LLVM, and PHP lowering; language-server and editor
tooling; future examples, website and playground activation; self-hosting compiler
code; performance and escape work.

## Invalidated Elsewhere

- D10's automatic arrow-capture wording is superseded.
- The master plan closure section must use explicit capture for both forms.
- Stage 30 must implement capture-specific diagnostics and preserve Decision
  0119's effect-set law.
- Accepted closure syntax must land in the pre-Stage-30 grammar slice rather than
  waiting for semantic/runtime implementation.
- Existing or future arrow examples that reference enclosing locals without
  `with` must be corrected; no-capture arrows remain unchanged.
- LSP, VS Code, IntelliJ, all execution backends, async work, self-hosting, and
  performance work must consume this decision when their owning stages begin.
- The pre-Stage-30 grammar slice is complete. Decision 0121 is accepted; Stage
  30a is next and Stage 30 remains unimplemented.
