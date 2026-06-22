# 0012 Dual native backend strategy

Status: Accepted

## Decision

Doria will pursue a dual native backend strategy:

```text
Fast native profile       -> Cranelift
Optimized native profile  -> LLVM
```

The profiles serve different parts of the same native-first goal:

```text
Fast profile:
  Prioritize local feedback speed for development loops, editor run commands, game iteration, CLI tool iteration, service development, tests, and smoke builds.

Optimized profile:
  Prioritize production performance, aggressive optimization, runtime throughput, artifact quality, and deployment/shipping builds.
```

These are backend profiles, not different Doria languages. The same checked Doria program must have the same Doria-visible behavior regardless of whether it is compiled through the fast profile or the optimized profile.

## Why this is accepted

Doria needs both:

```text
- fast local compile/run loops while developers are creating
- aggressive native optimization when developers are shipping
```

Game developers, CLI tool authors, service developers, and compiler contributors all benefit from fast edit-run-test cycles. Production services, production games, game engines, and systems-oriented tools also need high-quality optimized binaries.

A single backend can be forced to serve both goals, but that tends to compromise one side. The accepted strategy is to keep the goals explicit:

```text
Fast when creating.
Aggressive when shipping.
Same Doria semantics in both.
```

## Relationship to prior decisions

This decision builds on:

```text
0010 Native-first correctness
0011 Native execution path
```

Decision 0010 says correctness and safety outrank quick demos, and generated PHP is not a semantic oracle.

Decision 0011 accepts only the Stage 1 native execution rule:

```doria
function main(): int
{
    return 0;
}
```

This decision accepts the backend strategy for native work. It does not broaden Stage 1 language support.

## Stage 1 implementation route

Accepted for Stage 1 implementation:

```text
Use the Cranelift-backed fast native profile for the first standalone native executable smoke target.
```

Stage 1 remains deliberately tiny:

```doria
function main(): int
{
    return 0;
}
```

The Stage 1 implementation should prove only this:

```text
checked Doria -> Doria IR/native path -> standalone native executable -> process exit code
```

Stage 1 must not use PHP output as the expected behavior or as an implementation step.

## LLVM timing

LLVM is accepted as the intended optimized native profile, but LLVM implementation is not required for Stage 1.

LLVM should wait until Doria has enough of the native path to make release optimization meaningful, such as:

```text
- a stable enough native-oriented IR boundary
- conformance tests for native behavior
- integer/local/control-flow support
- enough runtime decisions to avoid LLVM shaping Doria semantics
```

LLVM must not be introduced early in a way that causes Doria to inherit LLVM-specific assumptions about overflow, undefined behavior, object layout, strings, exceptions, or runtime behavior.

## Backend profile semantics

The backend profile must not affect Doria language semantics.

Allowed differences:

```text
- compile time
- optimization level
- binary size
- debug information quality
- internal code generation strategy
- register allocation and instruction selection
- build pipeline details
```

Forbidden differences:

```text
- type checking behavior
- accepted source syntax
- assignment compatibility
- readonly/writable behavior
- integer semantics
- bool semantics
- string semantics
- object layout visible through Doria APIs
- collection semantics
- recoverable error behavior
- panic behavior
- destructor behavior
- FFI-visible behavior once FFI exists
```

If fast and optimized profiles produce different Doria-visible behavior for supported code, that is a compiler bug unless a later accepted decision explicitly allows a difference.

## Native pipeline shape

The intended long-term native pipeline is:

```text
Doria source
-> lexer
-> parser
-> AST
-> semantic/type checking
-> Doria IR
-> native-oriented IR
-> backend profile
   -> Cranelift fast profile
   -> LLVM optimized profile
```

Doria IR remains the checked compiler-owned representation. A later native-oriented IR may simplify control flow, temporaries, storage locations, runtime calls, and backend emission.

Neither Cranelift nor LLVM should distort Doria IR. Backend-specific lowering belongs after Doria semantics are already established.

## Conformance requirement

Once both native profiles support the same feature, they must share conformance tests.

The test rule should be:

```text
same Doria source
same semantic checks
same Doria-visible behavior
across fast and optimized native profiles
```

Early conformance checks should compare:

```text
- exit code
- stdout once stdout exists
- diagnostics for rejected programs
- unsupported-feature diagnostics where a backend legitimately lacks support
```

The fast profile may support a feature earlier than the optimized profile, but once both claim support, behavior must match.

## CLI naming is deferred

This decision does not settle final CLI spelling.

Possible future shapes include:

```bash
doriac compile app.doria --profile fast
doriac compile app.doria --profile release
```

or:

```bash
doriac compile app.doria --backend cranelift
doriac compile app.doria --backend llvm
```

Recommendation: prefer user-facing build profiles such as `fast` and `release` over exposing backend names as the main user workflow. Backend names may still be useful for diagnostics, internal tests, and advanced users.

This is only a recommendation; CLI spelling should be finalized in the implementation task or a later CLI decision.

## Stage 1 scope remains narrow

The first Cranelift-backed implementation may only target the accepted Stage 1 native shape.

Allowed for Stage 1:

```text
- locate exactly one top-level `function main(): int`
- verify the accepted Stage 1 body shape
- produce a standalone native executable
- return the main integer as the process exit code
- produce clear diagnostics for unsupported native features
```

Not allowed for Stage 1 unless a later decision explicitly expands it:

```text
- locals
- arithmetic
- if/while
- strings
- echo/stdout
- direct function calls
- classes/objects
- property initialization
- collections
- top-level executable statements
- FFI
- recoverable errors
- standard library runtime
```

## Diagnostics direction

The Stage 1 native backend should eventually distinguish:

```text
- general semantic errors
- missing native entrypoint
- wrong main signature
- multiple native entrypoints
- unsupported native statement
- unsupported native expression
- backend emission failure
- linker/toolchain failure
```

A Doria program rejected by the semantic checker must not reach native backend emission.

Unsupported native features should be reported as backend unsupported-feature diagnostics, not as general language errors, when the source remains valid Doria.

## Consequences

This decision means Stage 1 can move forward with Cranelift without pretending Cranelift is the permanent or only native backend.

It also means LLVM is part of the strategic native story, but it should arrive when Doria has enough native IR and conformance infrastructure to keep LLVM from shaping Doria semantics.

The cost is that Doria will eventually carry two native backend implementations. The benefit is a better developer experience during creation and stronger binaries when shipping.

## Non-goals

This decision does not:

```text
- add Cranelift as a dependency
- add LLVM as a dependency
- implement native code generation
- change CLI behavior
- define final CLI profile names
- implement native-oriented IR
- settle string representation
- settle object layout
- settle collection representation
- settle integer overflow
- settle FFI
- settle debug information
- settle release packaging
- change PHP backend behavior
```

It accepts the dual native backend strategy and the Cranelift Stage 1 implementation route only.
