# 0016 Fixed-width numeric types

Status: Accepted

## Decision

Doria has an accepted fixed-width numeric type family.

Accepted signed integer type spellings:

```doria
int8
int16
int32
int64
```

Accepted unsigned integer type spellings:

```doria
uint8
uint16
uint32
uint64
```

Accepted floating-point type spellings:

```doria
float32
float64
```

The default integer type spelling remains:

```doria
int
```

`int` means `int64`.

The default floating-point type spelling remains:

```doria
float
```

`float` means `float64`.

These are lowercase type-position spellings. They are Doria type names, not namespaces.

## Rationale

Doria needs predictable numeric behavior for native applications, command-line tools, games, graphics work, FFI, binary formats, and future raylib bindings.

The default numeric spellings should stay ergonomic:

```doria
int $count = 42;
float $ratio = 0.5;
```

When exact width matters, code should be able to say so directly:

```doria
int32 $x = 10;
uint8 $channel = 255;
float32 $delta = 0.016;
```

This avoids target-pointer-width defaults and avoids inheriting backend-specific integer or floating-point behavior from Cranelift, LLVM, C, Rust, PHP, or the host platform.

## Relationship to existing primitive names

Decision `0003-primitives-and-companions.md` establishes lowercase primitive type names and PascalCase companion/helper APIs.

This decision extends the lowercase primitive type family. It does not introduce Rust-shaped spellings such as:

```text
i8
i16
i32
i64
u8
u16
u32
u64
f32
f64
usize
isize
```

It also does not make primitive names into namespaces. Forms such as `int32::parse(...)` should not become valid. Future companion/helper API design may introduce PascalCase companions such as `Int32` or `Float64`, but that is a separate standard-library/API decision.

## Literal defaults

Integer literals default to `int`, which means `int64`, unless a more specific accepted numeric context is provided.

Floating-point literals default to `float`, which means `float64`, unless a more specific accepted numeric context is provided.

Examples:

```doria
let $count = 42;       // int, meaning int64
let $ratio = 0.5;      // float, meaning float64

int32 $port = 8080;
uint8 $alpha = 255;
float32 $seconds = 0.5;
```

Literal parsing, inference, and diagnostics must be Doria rules. They must not be copied from a backend or implementation language.

## Range and overflow rules

Each fixed-width integer type has the normal mathematical range implied by its signedness and bit width.

For the currently parsed non-negative decimal integer literal syntax, accepted ranges are:

```text
int8     0 through 127
int16    0 through 32767
int32    0 through 2147483647
int64    0 through 9223372036854775807

uint8    0 through 255
uint16   0 through 65535
uint32   0 through 4294967295
uint64   0 through 18446744073709551615
```

Negative values remain tied to the future unary-expression decision. A spelling such as `-1` should be treated as unary minus applied to an integer literal only after unary syntax and semantics are accepted.

Compile-time literal overflow must be diagnosed before Doria IR/native lowering. Future arithmetic overflow behavior still needs explicit accepted design where it has not already been settled.

## No implicit numeric widening yet

This decision does not accept implicit numeric widening, narrowing, or scalar coercion.

Examples that require a later conversion decision:

```doria
int64 $large = 1;
int32 $small = $large; // conversion rules not accepted here

float64 $wide = 1.0;
float32 $narrow = $wide; // conversion rules not accepted here
```

Any future safe widening, narrowing, checked conversion, wrapping conversion, or explicit conversion API must be designed separately. Doria must not inherit PHP-style scalar coercion, C-style implicit integer promotion, Rust-specific naming, or backend-specific conversion behavior by accident.

## Stage 2 native exit-code boundary

The Stage 2 native process-exit restriction that began in Stage 2a is:

```text
0..125
```

That restriction applies only to the observable native smoke-test process exit code returned from `main()`.

It is not the range of Doria `int`, `int64`, `uint64`, or any other numeric type.

For example, a later accepted native slice may allow:

```doria
function main(): int
{
    int $value = 9223372036854775807;
    return 0;
}
```

while still rejecting this as a process-exit value until a broader process-exit mapping decision exists:

```doria
function main(): int
{
    int $value = 126;
    return $value;
}
```

The error in the second example is about the temporary native smoke-test exit-code boundary, not about the range of `int`.

## Backend and IR boundaries

Cranelift and LLVM must lower Doria numeric types according to Doria semantics. They do not define those semantics.

The compiler's semantic type model and Doria IR should eventually distinguish these numeric types explicitly. Any backend-internal storage choice, ABI representation, register class, machine instruction selection, or optimization strategy must remain an implementation detail behind Doria semantics.

## Implementation status

Stage 13 implements the fixed-width integer family as real compiler and runtime types:

```doria
int8
int16
int32
int64
uint8
uint16
uint32
uint64
```

`int` and `int64` are the same canonical signed 64-bit integer type. Stage 13 also implements contextual integer-literal typing, the remaining integer operators, and explicit checked integer conversions under decisions 0041 and 0042.

Stage 14 implements `float32` and canonical `float`/`float64` as real semantic,
MIR, interpreter, and native runtime values. `float` and `float64` are one
binary64 type; `float32` is a distinct binary32 type. Decimal literals are
contextually rounded directly to the expected float width, while unconstrained
float literals default to `float64`. Decision 0072 defines the exact IEEE 754
arithmetic, comparison, literal, ABI, and parity contracts.

## Non-goals

This decision does not:

- add lexer keywords or numeric literal suffixes
- define unary minus
- define implicit numeric conversions
- define final process-exit behavior for all Doria integer values
- define public float payload/bit-inspection APIs
- define additional cross-kind conversion companions beyond decision 0042
