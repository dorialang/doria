# 0013 Stage 2 native integer execution

Status: Proposed

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

This note is proposed only. It does not implement Stage 2 and does not change Doria semantics until accepted.

## Recommendation summary

Recommendation:

```text
- Split Stage 2 into small sub-stages instead of one large integer slice.
- Use fixed-width signed 64-bit `int` for early native integer semantics.
- Require `int` literals to fit the accepted Doria `int` range.
- Reject compile-time constant overflow before native lowering.
- Defer non-constant runtime overflow until Doria has an accepted panic/error/runtime policy.
- For native smoke exit-code tests, support only a narrow portable exit-code literal range at first.
- Require future LLVM support to match Cranelift/Doria behavior exactly for accepted Stage 2 programs.
```

These are recommendations, not accepted decisions.

## Proposed sub-stages

Stage 2 does not need to be one implementation jump.

Recommended sequence:

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

Recommendation:

```text
Use fixed-width signed 64-bit `int` for early native semantics.
```

This should be accepted explicitly before implementation. Cranelift and LLVM should lower Doria `int` according to this Doria decision, not according to backend defaults.

## Integer literal rules

Questions to settle:

```text
- What decimal literal range is accepted for `int`?
- Are negative integers parsed as negative literals or unary expressions?
- What diagnostic is reported when an integer literal does not fit Doria `int`?
- Is compile-time literal overflow a semantic error?
```

Current syntax already parses integer literals as unsigned digit text. A spelling such as `-1` should be treated as a unary expression only after unary syntax is specified; Stage 2 should not smuggle unary operators into the native backend by treating `-1` as a special case.

Recommendation:

```text
- Decimal integer literals in `int` contexts must fit the accepted Doria `int` range.
- Out-of-range literals are semantic diagnostics before Doria IR/native lowering.
- Negative values are deferred until unary expression syntax and semantics are specified.
- Compile-time literal overflow is a semantic error.
```

For an accepted signed 64-bit `int`, the positive literal range for Stage 2 would be:

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

Recommendation:

```text
Doria must not inherit LLVM undefined behavior or Cranelift wrapping behavior by accident.

For early native integer arithmetic:
- reject compile-time constant overflow before native lowering
- defer non-constant runtime overflow until Doria has an accepted panic/error/runtime policy
```

If Stage 2c needs arithmetic before runtime overflow policy exists, it should initially accept only expressions whose overflow can be decided at compile time. For example, arithmetic over integer literals and readonly locals initialized from integer literals can be constant-folded and checked before lowering.

Do not introduce debug-checked/release-unchecked behavior. The fast Cranelift profile and optimized LLVM profile are backend profiles, not different Doria languages.

## Process exit-code mapping

Stage 1 only returns `0`, so it avoids platform exit-code differences.

Stage 2a may want:

```doria
function main(): int
{
    return 42;
}
```

Questions to settle:

```text
- What range is portable for native smoke tests?
- Should Doria reject out-of-range exit-code literals for now?
- Should Doria map or truncate according to platform convention?
- Is the full `int` return value semantic while the observed process status is platform-specific?
```

Recommendation:

```text
For Stage 2 native smoke tests, support portable non-negative exit-code literals in a narrow range first.
```

Possible accepted range:

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

This is familiar on Unix-like systems, but still risks platform-specific interpretation and shell conventions. If this range is chosen, the decision should explicitly state that the observed process status is constrained to the platform process-exit mechanism.

Recommendation:

```text
- Stage 2a should reject out-of-range exit-code literals before native lowering.
- Stage 2 should not silently allow platform truncation.
- A later process-exit decision can specify how full Doria `int` return values map to platform exit status.
```

## Proposed Stage 2 implementation scope

Possible Stage 2a accepted shape:

```doria
function main(): int
{
    return 42;
}
```

Possible Stage 2b accepted shape:

```doria
function main(): int
{
    let $code = 42;
    return $code;
}
```

Possible Stage 2c/2d accepted shape:

```doria
function main(): int
{
    let $x = 20;
    let $y = 22;
    return $x + $y;
}
```

The implementation should still require:

```text
- exactly one top-level `function main(): int`
- no parameters
- no top-level executable statements
- no extra top-level functions
- no classes
- no unsupported statements inside `main`
- parser and semantic checks before native lowering
```

Out of scope for Stage 2:

```text
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

Those belong to later accepted native slices.

## Conformance expectations

When LLVM later supports Stage 2 integer execution, it must match Cranelift/Doria behavior exactly for accepted Stage 2 integer programs.

Conformance checks should include:

```text
- process exit code for supported values
- diagnostics for out-of-range integer literals
- diagnostics for unsupported native expressions
- diagnostics for unsupported native statements
- diagnostics for compile-time integer overflow
- identical accepted/rejected behavior between Cranelift fast profile and LLVM optimized profile once LLVM claims Stage 2 support
```

Expected tests should compare Doria-visible behavior and diagnostics, not backend IR or backend-specific instruction choices.

## Stage 1 hardening note

Implementation note for the next code task:

```text
Stage 1 native tests should mirror the backend linker selection behavior, including `CC` if set, so tests do not skip or fail differently from the backend.
```

This note does not implement that change. It records a hardening issue for the implementation follow-up.

## Non-goals of this proposal

This proposal does not:

```text
- implement Stage 2 native code generation
- change the Stage 1 native backend
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

## Open questions for review

Andrew should explicitly decide:

```text
1. Should early Doria `int` be signed 64-bit?
2. Should Stage 2a use exit-code range `0..125`, `0..255`, or another range?
3. Should negative integer expressions remain out of scope until unary operators are specified?
4. Should Stage 2c accept only compile-time-checkable arithmetic until runtime overflow policy exists?
5. What diagnostic wording should Doria use for out-of-range literals and constant overflow?
```

Until those questions are accepted, Stage 2 implementation should not begin.
