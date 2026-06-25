# 0015 Stage 2b native readonly integer locals

Status: Proposed

Andrew must review this decision before implementation. Do not implement Stage 2b native code generation from this document until it is accepted.

## Question

Stage 2a native integer execution supports this shape:

```doria
function main(): int
{
    return 42;
}
```

Stage 2b should decide the next narrow native slice for readonly integer locals without confusing Doria integer semantics with process exit-code limitations or committing Doria locals to a backend storage model.

## Context

Accepted decisions already establish:

- Doria is native-first and backend output does not define Doria semantics.
- Cranelift and LLVM are backend profiles; they implement Doria, not separate languages.
- Doria `int` is a fixed-width signed 64-bit integer for early native integer semantics.
- `0016-fixed-width-numeric-types.md` accepts the fixed-width numeric family and clarifies that `int` means `int64`.
- Decimal integer literals in `int` contexts must fit the Doria `int` range before Doria IR/native lowering.
- The current `0..125` range is a portable native smoke-test process exit-code range. It is not the range of Doria `int`.

The current AST and Doria IR already represent local declarations with mutability, an optional parsed type annotation, a name, and an initializer expression.

The current semantic checker already handles declaration rules, readonly/writable bindings, duplicate locals, undeclared variables, type resolution, assignment compatibility, return types, and integer literal range diagnostics.

## Proposed Stage 2b objective

Stage 2b should add native support for readonly local integer bindings initialized from integer literals inside the accepted `main(): int` entrypoint.

The smallest useful target shape is:

```doria
function main(): int
{
    let $code = 42;
    return $code;
}
```

Stage 2b should also support the explicitly typed readonly spelling:

```doria
function main(): int
{
    int $code = 42;
    return $code;
}
```

The Stage 2b native body shape is:

```text
zero or more supported readonly integer local declarations
then exactly one supported return statement
```

Supported return statements are:

```doria
return 42;
return $code;
```

No statement may follow the return.

## Proposed accepted source forms

Stage 2b native output should accept:

```doria
function main(): int
{
    let $code = 42;
    return $code;
}
```

```doria
function main(): int
{
    int $code = 42;
    return $code;
}
```

```doria
function main(): int
{
    let $unused = 9223372036854775807;
    return 0;
}
```

```doria
function main(): int
{
    let $zero = 0;
    int $code = 42;
    return $code;
}
```

The third example is important: a local may hold any accepted Doria `int` literal value. The `0..125` restriction applies only when a value is used as the observable Stage 2 native process exit code.

## Proposed unsupported native forms

These forms may remain valid Doria but should be rejected by the native Stage 2b backend with unsupported-feature diagnostics until later stages:

Writable locals:

```doria
function main(): int
{
    let writable $code = 42;
    return $code;
}
```

Assignments:

```doria
function main(): int
{
    let writable $code = 0;
    $code = 42;
    return $code;
}
```

Local initializers that are not integer literals:

```doria
function main(): int
{
    let $a = 20 + 22;
    return $a;
}
```

```doria
function main(): int
{
    let $a = 42;
    let $b = $a;
    return $b;
}
```

Returned expressions beyond an integer literal or supported readonly local:

```doria
function main(): int
{
    let $a = 20;
    return $a + 22;
}
```

Control flow:

```doria
function main(): int
{
    let $code = 42;
    if ($code == 42) {
        return $code;
    }

    return 0;
}
```

The arithmetic, local-to-local initializer, returned expression, and control-flow cases are future Stage 2c, Stage 2d, or later native work. This proposal keeps Stage 2b intentionally smaller.

## Doria integer range vs process exit-code range

Doria `int` is signed 64-bit for early native integer semantics.

For current decimal positive literal syntax, valid `int` literal values in `int` contexts are:

```text
0 through 9223372036854775807
```

Stage 2b must not treat `0..125` as the range of local integer values.

The `0..125` range applies only to the value returned from `main()` for the current portable native smoke-test process exit-code rule.

Therefore:

```doria
function main(): int
{
    let $value = 126;
    return 0;
}
```

should be accepted by Stage 2b, while:

```doria
function main(): int
{
    let $value = 126;
    return $value;
}
```

should be rejected by Stage 2b native output because the observable process exit code is outside the accepted portable `0..125` range.

Out-of-range Doria `int` literals remain semantic diagnostics before Doria IR/native lowering:

```doria
function main(): int
{
    let $value = 9223372036854775808;
    return 0;
}
```

This is a Doria semantic error, not a backend storage failure.

## Local binding semantics

Stage 2b local bindings should follow existing Doria semantics:

- `let $code = 42;` declares a readonly inferred local.
- `int $code = 42;` declares a readonly explicitly typed local.
- Bare assignment never declares a local.
- Duplicate locals in the same scope are errors.
- Reads of undeclared locals are errors.
- Readonly locals cannot be assigned after declaration.
- Explicit non-`int` local annotations are not part of Stage 2b native output.

The semantic checker remains responsible for language errors. The native backend should reject only valid-but-unsupported Doria shapes for the current native slice.

## No storage-model commitment

Stage 2b must not define Doria locals in terms of Cranelift stack slots, registers, SSA values, or machine storage.

A Doria local is a semantic binding with a type, name, scope, mutability, and initializer.

For Stage 2b implementation, a backend may lower a supported readonly local to any internal representation that preserves Doria-visible behavior:

- an immediate constant
- an SSA value
- a stack slot
- a register allocation result
- another backend-internal value representation

Those choices are implementation details. They must not become user-facing language semantics, Doria IR requirements, or constraints on the future LLVM optimized profile.

## Narrow compile-time value tracking

To support `return $code;`, Stage 2b native validation may track the literal value of supported readonly integer locals.

That tracking is a backend validation fact for this native slice. It is not a general Doria `const` feature, not general constant folding, and not permission to evaluate arbitrary expressions at compile time.

The tracking should be limited to:

```text
readonly local name -> signed 64-bit integer literal value
```

Only values that reach the `main()` return position need the portable process exit-code check.

## Diagnostics direction

Recommended diagnostics:

- For valid Doria that Stage 2b native output does not support, use backend unsupported-feature diagnostics.
- For integer literals outside the Doria `int` range, preserve the existing semantic diagnostic before native lowering.
- For `return $code;` where `$code` is a supported local with a value outside `0..125`, report a native exit-code diagnostic that clearly says the process exit-code range is the issue.
- Do not report local integer values outside `0..125` unless that value is returned as the process exit code.

Example wording:

```text
native Stage 2b exit code must be in the range 0..125
```

```text
unsupported native local for Stage 2b: expected readonly `int` local initialized from an integer literal
```

```text
unsupported native expression for Stage 2b: expected integer literal or readonly integer local
```

## Conformance requirement

When both native profiles support Stage 2b, they must share conformance tests:

```text
same Doria source
same semantic checks
same accepted/rejected native shapes
same process exit behavior
```

Cranelift and LLVM may use different internal storage strategies. They must preserve the same Doria-visible behavior.

## Proposed implementation boundary

If this decision is accepted, the implementation should remain narrow:

- Do not add arithmetic.
- Do not add assignment support.
- Do not add mutable local support.
- Do not add local-to-local initializers.
- Do not add top-level statement execution.
- Do not add `if`, `while`, or other control flow.
- Do not add strings, stdout, function calls, classes, objects, collections, FFI, or runtime support.
- Do not change Doria `int` semantics.
- Do not change the public process exit-code rule beyond tracking returned readonly local values.
- Do not reshape Doria IR around Cranelift storage needs.

## Open review questions

Andrew should review these proposed boundaries before implementation:

1. Should Stage 2b include only integer-literal local initializers, or also allow `let $b = $a;` when `$a` is a supported readonly integer local?
2. Should Stage 2b support both `let $code = 42;` and `int $code = 42;`, as proposed here?
3. Should a returned local outside `0..125` reuse the Stage 2a exit-code diagnostic wording or use Stage 2b-specific wording?
4. Should unused readonly locals be accepted in Stage 2b native output, as proposed here, or rejected until native code generation can prove they are harmless?

Until these are accepted, this document remains Proposed.
