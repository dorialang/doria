# 0072 Stage 14 floats and bool runtime

Status: Accepted

## Context

Stage 13 established typed fixed-width integer execution through MIR, the debug
interpreter, and Cranelift. The semantic type model already recognizes
`float32`, `float`/`float64`, and `bool`, but those values need one canonical
runtime and MIR model rather than backend-specific or condition-only paths.

## Canonical float types and representation

Doria has two canonical floating-point types:

```text
float32
float64
```

`float` is an exact source alias of `float64`; the two spellings identify one
semantic and runtime type. `float32` remains distinct. There is no implicit
conversion between the two widths or between integers and floats.

`float32` follows IEEE 754 binary32 and `float`/`float64` follows IEEE 754
binary64. Basic arithmetic rounds to nearest, ties to even. Every `float32`
operation rounds to binary32 and every `float64` operation rounds to binary64;
extended-precision intermediates must not become visible. Signed zero and
subnormal values are preserved. Overflow produces signed infinity. Gradual
underflow may produce a subnormal or signed zero.

Division by zero follows IEEE 754 and does not panic: nonzero divided by zero
produces signed infinity and zero divided by zero produces NaN. Float
arithmetic does not use integer-overflow panic rules. Fast-math,
reassociation, and NaN-eliding transformations are forbidden.

Supported float operators are unary `-`, binary `+`, `-`, `*`, `/`, all six
numeric comparisons, `++`, `--`, and `+=`, `-=`, `*=`, `/=`. Remainder,
shifts, bitwise operators, `%=`, and bitwise compound assignments are invalid
for floats.

NaN compares unequal to every value including itself. `!=` is true when either
operand is NaN, while `<`, `<=`, `>`, and `>=` are false. Positive and negative
zero compare equal. NaN payload bits and sign are not Doria-language semantics
in v1.0 and Stage 14 exposes no payload API. Non-NaN parity is bit-exact; NaN
parity is classification plus Doria-visible comparison behavior.

Float expressions and function arguments evaluate left-to-right. Backends may
not reorder visible operations.

## Float literals

Stage 14 retains the decimal syntax already accepted by the lexer and parser:
`0.0`, `1.0`, `42.5`, and `0.125`. It does not add exponent, leading-dot,
trailing-dot, suffix, NaN, or infinity syntax. Negative values remain unary
negation.

An unconstrained floating literal defaults to canonical `float64`. In an
expected `float32` context it rounds directly to binary32; in an expected
`float`/`float64` context it rounds directly to binary64. Contextual literal
typing is not an implicit conversion. Integer literals never adopt float
context and float literals never adopt integer context. A nonzero literal that
rounds to infinity is a compile-time out-of-range diagnostic; subnormal and
zero-rounded literals remain valid.

## Runtime bool

`bool` is a real Copy scalar value type. `true` and `false` may be stored in
readonly or writable locals and passed to or returned from helper functions.
`main(): bool` remains invalid.

Supported bool operators are `!`/`not`, `&&`/`and`, `||`/`or`, `xor`, `==`,
and `!=`. The word and symbolic forms are exact synonyms. And/or short-circuit
in condition and value position; xor evaluates both operands. Bool equality
compares the two bool values. Ordered comparison, arithmetic, increment,
decrement, bitwise operators, and numeric conversions are invalid.

Conditions consume bool values directly. Integer, float, string, object, and
mixed truthiness remain invalid.

## Runtime and backend representation

MIR uses one scalar model for fixed-width integers, floats, and bools across
locals, operands, arguments, returns, assignments, and branch values. Strings
remain separate and compile-time-known in this stage.

The interpreter uses the compiler's bit-preserving float engine. Cranelift maps
`float32` to `F32`, `float64` to `F64`, and bool to canonical `I8` values 0 and
1. Bool value materialization uses the same short-circuit control flow as
branches. Cranelift enables no fast-math flags.

The PHP compatibility backend may implement exact bool and float64 subsets.
Float division uses `fdiv`, not PHP `/`. Float32 coverage and any conversion
whose exact Doria contract cannot be preserved must produce a clear backend
unsupported-feature diagnostic.

## Consequences

- Float and bool helper parameters, returns, calls, locals, and assignments are
  part of the durable interpreter/native parity contract.
- `main` remains limited to canonical `int`/`int64` or `void`.
- No implicit numeric conversion, truthiness, public bit API, float formatting,
  runtime string work, or LLVM backend is introduced.
- Decision 0042 defines the only Stage 14 cross-kind companion conversions.

## Affected components

- Numeric types and values, semantic literal typing, and diagnostics
- Typed MIR, lowering, deterministic dumps, and interpreter execution
- Cranelift scalar ABI, float operations, bool control flow, and conversions
- PHP compatibility diagnostics/lowering, LSP and editor status
- Examples, durable parity manifest/matrix, documentation, and CI
