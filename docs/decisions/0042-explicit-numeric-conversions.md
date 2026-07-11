# 0042 Explicit numeric conversions

Status: Accepted

## Context

Decision 0016 accepts distinct fixed-width integer types, and decision 0041 rejects implicit widening, narrowing, and integer promotions. Stage 13 therefore needs one explicit, checked conversion surface that is independent of compiler implementation language and backend behavior.

## Decision

There are no implicit conversions between distinct integer types. Contextual literal typing is not an implicit conversion: it determines the type of a literal before the literal becomes a typed value.

Explicit integer conversion uses these PascalCase companion APIs:

```doria
Int::from($value)
Int8::from($value)
Int16::from($value)
Int32::from($value)
Int64::from($value)

UInt8::from($value)
UInt16::from($value)
UInt32::from($value)
UInt64::from($value)
```

`Int` and `Int64` target the same canonical `int64` type.

Each conversion accepts exactly one integer expression.

- A same-type conversion is a no-op.
- Widening conversions are exact.
- Narrowing conversions are checked.
- Signed-to-unsigned conversion panics when the value is negative or too large.
- Unsigned-to-signed conversion panics when the value exceeds the target maximum.

Conversion failure panics with this exact message:

```text
integer conversion out of range
```

The panic follows decision 0040: it writes the deterministic panic message and Doria function-name stack trace to stderr, then exits with status 101.

These conversion APIs are compiler-known companion intrinsics in Stage 13. This does not generally implement static methods or classes.

Float/integer conversion remains Stage 14 work. Stage 13 introduces no `as` syntax and no wrapping, saturating, or unchecked conversion API.

## Alternatives considered

- Implicit widening was rejected because distinct integer types do not combine or assign silently in Doria.
- C-style casts and `as` casts were rejected in favor of the accepted PascalCase companion API direction.
- Wrapping, saturating, and unchecked conversion APIs were deferred because Stage 13's explicit conversion contract is checked.
- General static-method or class support was rejected as an implementation prerequisite; these APIs are focused compiler-known intrinsics.

## Consequences

- Conversion intent is explicit at every nonliteral integer-type boundary.
- The semantic checker must resolve the target companion to a canonical integer type and require exactly one integer expression.
- MIR and every execution backend must implement the same range check and preserve the same panic outcome.
- `Int::from(...)` and `Int64::from(...)` are aliases at the type level, not separate runtime conversions.

## Affected components

- Built-in companion-name reservation and semantic resolution
- Contextual literal and integer assignment checking
- HIR/MIR conversion representation and lowering
- MIR interpreter and Cranelift conversion execution
- `doria-rt` panic paths
- PHP compatibility-backend lowering
- Compiler diagnostics, LSP/editor support, examples, and differential tests
