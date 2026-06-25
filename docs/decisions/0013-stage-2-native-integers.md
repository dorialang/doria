# 0013 Stage 2 native integer execution

Status: Accepted

## Question

Stage 1 proved that checked Doria can produce a standalone native executable returning exit code 0.

Stage 2 must decide how Doria native execution handles integer values before expanding code generation.

The next native slice should not silently inherit integer behavior from Cranelift, LLVM, Rust, C, PHP, or the host operating system. Backends implement Doria; they do not define it.

## Context

Accepted Stage 1 source shape:

```doria
function main(): int
{
    return 0;
}
```

Stage 2 is expected to expand native support toward:

```text
- integer literal return values
- local integer variables
- integer arithmetic
- returned integer expressions
```

Those features require answers for:

```text
- what `int` means
- what integer literals mean
- what integer overflow means
- how `main(): int` maps to an observed process exit status
- how Cranelift fast profile and future LLVM optimized profile remain semantically identical
```

This decision accepts the Stage 2 native integer direction and the narrow Stage 2a implementation path. The current implementation supports Stage 2a only; Stage 2b, Stage 2c, and Stage 2d remain future implementation slices.

## Decision summary

Accepted decisions:

```text
- Split Stage 2 into small sub-stages instead of one large integer slice.
- Implement Stage 2a first: `return <portable integer exit-code literal>;` from `main`.
- Use fixed-width signed 64-bit `int` for early native integer semantics.
- Require decimal integer literals in `int` contexts to fit the signed 64-bit Doria `int` range.
- Support Stage 2a native exit-code literals only in the portable `0..125` range.
- Reject out-of-range integer literals before Doria IR/native lowering.
- Reject compile-time constant overflow before native lowering when arithmetic lands.
- Defer non-constant runtime overflow until Doria has an accepted panic/error/runtime policy.
- Require future LLVM support to match Cranelift/Doria behavior exactly for accepted Stage 2 programs.
```

Cranelift and LLVM are backend profiles. They implement Doria; they do not define Doria integer semantics.

## Accepted staged rollout

Stage 2 does not need to be one implementation jump.

Accepted sequence:

```text
Stage 2a:
  Support `return <portable integer exit-code literal>;` from `main`.

Stage 2b:
  Support readonly local integer declarations initialized from integer literals.

Stage 2c:
  Support simple integer arithmetic expressions with literals and locals.

Stage 2d:
  Support returning simple integer expressions from `main`.
```

Stage 2a is the current accepted and implemented native integer slice. Stage 2b, Stage 2c, and Stage 2d remain separate future implementation slices and should not be treated as implemented or ready to implement by this decision alone.

Rationale:

```text
- Stage 2a isolates process exit-code mapping from general integer arithmetic.
- Stage 2b adds local storage without arithmetic overflow.
- Stage 2c forces arithmetic semantics and overflow diagnostics into the open.
- Stage 2d proves returned expressions without broadening the language into control flow, calls, strings, or runtime support.
```

## `int` representation options

### Option: signed 64-bit integer

Pros:

```text
- Stable across 32-bit and 64-bit targets.
- Easy to specify and test.
- Familiar for application and CLI code.
- Keeps Cranelift and LLVM profiles aligned through explicit lowering.
```

Cons:

```text
- May surprise users expecting machine-word `int`.
- Some systems or FFI work may eventually want explicit sized integer types.
```

### Option: target pointer-width integer

Pros:

```text
- Convenient for low-level indexing and host ABI work.
- Matches some systems-language expectations.
```

Cons:

```text
- Makes behavior differ across 32-bit and 64-bit targets.
- Makes tests and cross-platform conformance harder.
- Lets target architecture leak into normal Doria semantics too early.
```

### Option: arbitrary precision integer

Pros:

```text
- Avoids fixed-width overflow for ordinary arithmetic.
- Friendly for some high-level application code.
```

Cons:

```text
- Requires runtime allocation and representation decisions.
- Too large for Stage 2 native smoke work.
- Adds performance and FFI complexity before Doria has a runtime model.
```

### Option: separate sized integer families later

Pros:

```text
- Useful for FFI, binary formats, graphics, game engines, and systems APIs.
- Lets Doria expose explicit sizes where size matters.
```

Cons:

```text
- Does not answer what the default `int` means now.
- Requires naming, conversion, overflow, and literal inference design.
```

Decision:

```text
Use fixed-width signed 64-bit `int` for early native semantics.
```

Doria `int` is a fixed-width signed 64-bit integer for early native integer semantics. Cranelift and LLVM should lower Doria `int` according to this Doria decision, not according to backend defaults.

Explicit fixed-width numeric spellings are accepted separately in `0016-fixed-width-numeric-types.md` for FFI, binary formats, graphics, game engines, and systems APIs. They are not implemented by Stage 2a.

## Integer literal rules

This decision answers:

```text
- Decimal integer literals in `int` contexts must fit the signed 64-bit Doria `int` range.
- Negative integers are not parsed as special negative literals in Stage 2a.
- Out-of-range integer literals must be diagnosed before Doria IR/native lowering.
- Compile-time literal overflow is a semantic error.
```

Current syntax already parses integer literals as unsigned digit text. A spelling such as `-1` should be treated as a unary expression only after unary syntax is specified; Stage 2 should not smuggle unary operators into the native backend by treating `-1` as a special case.

Decision:

```text
- Decimal integer literals in `int` contexts must fit the accepted Doria `int` range.
- Out-of-range literals are semantic diagnostics before Doria IR/native lowering.
- Negative values are deferred until unary expression syntax and semantics are specified.
- Compile-time literal overflow is a semantic error.
```

For the accepted signed 64-bit Doria `int`, the positive literal range for Stage 2 is:

```text
0 through 9223372036854775807
```

If negative expressions are later accepted, the minimum `int` value needs an explicit rule because `-9223372036854775808` is usually represented as unary minus applied to a literal one greater than the positive maximum.

## Overflow behavior

Overflow is the most important safety issue for Stage 2.

Options:

| Option | Benefits | Risks |
| --- | --- | --- |
| Wrapping overflow | Simple to lower; predictable for low-level bit work. | Unsafe default for application code; easy to inherit backend behavior accidentally. |
| Trapping or panic on overflow | Safer default; catches bugs. | Requires an accepted trap/panic/runtime policy and user-facing diagnostics. |
| Compile-time rejection only for constant overflow | Good first step for literal arithmetic; no runtime needed. | Does not answer runtime computed overflow. |
| Debug checked / release unchecked | Common in some ecosystems. | Creates profile-dependent Doria behavior, conflicting with the Cranelift/LLVM conformance rule. |
| Arbitrary precision promotion | Avoids fixed-width overflow. | Requires runtime representation and allocation decisions outside Stage 2. |

Decision:

```text
Doria must not inherit LLVM undefined behavior or Cranelift wrapping behavior by accident.

For Stage 2a:
- arithmetic is not supported, so arithmetic overflow is not yet applicable
- out-of-range integer literals are rejected before native lowering

For future Stage 2c:
- compile-time constant overflow should be rejected before native lowering
- non-constant runtime overflow remains deferred until Doria has an accepted panic/error/runtime policy
```

If Stage 2c needs arithmetic before runtime overflow policy exists, it should initially accept only expressions whose overflow can be decided at compile time. For example, arithmetic over integer literals and readonly locals initialized from integer literals can be constant-folded and checked before lowering.

Do not introduce debug-checked/release-unchecked behavior. The fast Cranelift profile and optimized LLVM profile are backend profiles, not different Doria languages.

## Process exit-code mapping

Stage 1 only returns `0`, so it avoids platform exit-code differences.

Stage 2a accepts:

```doria
function main(): int
{
    return 42;
}
```

This decision answers:

```text
- The portable Stage 2a native smoke-test range is `0..125`.
- Doria must reject out-of-range Stage 2a exit-code literals before native lowering.
- Doria must not map or truncate Stage 2a return values according to platform convention.
- Full `int` return-value process-exit behavior remains a later decision.
```

Decision:

```text
For Stage 2a native executable output, support portable non-negative exit-code literals in the `0..125` range.
```

Accepted range:

```text
0..125
```

Rationale:

```text
- `0` means success on common platforms.
- Small non-negative values are practical for smoke tests.
- `126` and `127` have conventional shell meanings on Unix-like systems.
- Larger values can be platform-truncated or interpreted differently.
```

Alternative possible range:

```text
0..255
```

This is familiar on Unix-like systems, but still risks platform-specific interpretation and shell conventions. Stage 2a does not choose this range.

Decision:

```text
- `return 0;` through `return 125;` are accepted for native Stage 2a.
- `return 126;` and above are rejected for native Stage 2a.
- Negative return values are out of scope because unary expressions are out of scope.
- Stage 2a must reject out-of-range exit-code literals before native lowering.
- Stage 2a must not silently allow platform truncation.
- A later process-exit decision can specify how full Doria `int` return values map to platform exit status.
```

## Accepted Stage 2a implementation scope

Stage 2a may compile only this shape:

```doria
function main(): int
{
    return 42;
}
```

The returned integer literal must be in the accepted `0..125` Stage 2a exit-code range.

Stage 2a must still require:

```text
- exactly one top-level `function main(): int`
- no parameters
- no top-level executable statements
- no extra top-level functions
- no classes
- one `return <portable integer literal>;` statement in `main`
- parser and semantic checks before native lowering
```

Stage 2a must not compile:

```text
- locals
- arithmetic
- strings
- echo/stdout
- bools and comparisons
- if/while
- foreach
- function calls
- methods or static calls
- classes/objects
- collections
- top-level native script mode
- FFI
- recoverable errors
- panic/runtime system
- standard library runtime
```

Those belong to later accepted native slices. Stage 2b, Stage 2c, and Stage 2d remain future work.

## Conformance expectations

When LLVM later supports Stage 2 integer execution, it must match Cranelift/Doria behavior exactly for accepted Stage 2 integer programs.

Conformance checks should include:

```text
- accepted/rejected behavior
- process exit code for supported values
- diagnostics for out-of-range integer literals
- diagnostics for unsupported native expressions
- diagnostics for unsupported native statements
- diagnostics for compile-time integer overflow
- identical accepted/rejected behavior between Cranelift fast profile and LLVM optimized profile once LLVM claims Stage 2 support
```

Expected tests should compare Doria-visible behavior and diagnostics, not backend IR or backend-specific instruction choices.

## Linker test hardening note

Implementation note:

```text
Stage 1 native tests should mirror the backend linker selection behavior, including `CC` if set, so tests do not skip or fail differently from the backend.
```

The Stage 2a implementation keeps this behavior hardened: native tests check the same `CC`-selected linker that the backend uses.

## Non-goals of this decision

This decision does not:

```text
- implement native code generation beyond Stage 2a
- change the native backend beyond Stage 2a
- change CLI behavior
- add Cranelift features
- add LLVM
- add a debug interpreter
- implement a native-oriented IR
- add tests
- change Cargo dependencies
- define string, bool, object, collection, FFI, or runtime semantics
- define a full panic or recoverable-error policy
- define final process-exit behavior for all Doria `int` values
```

## Resolved questions

This decision resolves:

```text
1. Early Doria `int` is signed 64-bit.
2. Stage 2a uses exit-code range `0..125`.
3. Negative integer expressions remain out of scope until unary operators are specified.
4. Future Stage 2c should reject compile-time constant overflow before native lowering.
5. Non-constant runtime overflow remains deferred until Doria has an accepted panic/error/runtime policy.
```

Diagnostic wording for out-of-range literals and constant overflow can be finalized during implementation, as long as diagnostics are clear and occur before Doria IR/native lowering.
