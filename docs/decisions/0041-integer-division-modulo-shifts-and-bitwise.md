# 0041 Integer division, modulo, shifts, and bitwise operators

Status: Accepted

## Context

Decision 0016 accepts Doria's fixed-width integer family, and decision 0040 establishes abort-only checked-arithmetic panics. Stage 13 needs backend-independent rules for the remaining integer operators so the semantic checker, MIR interpreter, Cranelift backend, PHP compatibility backend, and runtime agree on one Doria-visible result.

Host-language, PHP, Cranelift, LLVM, and machine-instruction behavior do not define these semantics.

## Decision

### Integer operator typing

Both operands of an integer binary operator must resolve to the same canonical integer type. `int` and `int64` are the same canonical type.

Contextual integer literals may adopt the other operand's integer type when they fit. Variables of different integer types do not combine implicitly. No C-style integer promotions exist, and no implicit widening or narrowing exists.

Integer comparison operands must also have the same canonical type.

### Signed division

Signed integer division truncates toward zero.

- Division by zero panics.
- `MIN / -1` panics because the quotient is unrepresentable.

### Unsigned division

Unsigned integer division uses ordinary unsigned integer division. Division by zero panics.

### Signed remainder

Signed integer remainder satisfies:

```text
dividend == quotient * divisor + remainder
```

It uses the quotient produced by truncation toward zero. A nonzero remainder has the sign of the dividend.

- Remainder by zero panics.
- `MIN % -1` produces zero; it does not panic.

### Unsigned remainder

Unsigned integer remainder uses ordinary unsigned remainder. Remainder by zero panics.

### Shift rules

Shift operands must resolve to the same canonical integer type. Contextual literals may adopt that type.

- A negative signed shift count panics.
- A shift count greater than or equal to the left operand's bit width panics.
- Left shift is a fixed-width bit operation. Bits shifted beyond the type width are discarded.
- Left shift does not use arithmetic-overflow panic rules beyond validating the shift count.
- Signed right shift is arithmetic and propagates the sign bit.
- Unsigned right shift is logical and shifts in zero bits.

### Bitwise rules

`&`, `|`, `^`, and `~` preserve the operand integer type. `^` is bitwise XOR. The word `xor` remains the boolean operator and is never a bitwise synonym.

Bitwise operations operate on the fixed-width two's-complement bit pattern. They do not overflow.

### Unary negation

Unary `-` is valid only for signed integer types. Runtime negation of the signed minimum value panics.

Contextual literal forms such as this are valid:

```doria
int8 $x = -128;
```

This is a compile-time literal-range error:

```doria
int8 $x = -129;
```

Unary `-` on an unsigned value is a compile-time error.

### Checked arithmetic and mutation

`+`, `-`, and `*` remain checked for every width and signedness. Signed overflow panics. Unsigned overflow and underflow panic.

`++` and `--` use the same checked rules as addition and subtraction.

The Stage 13 compound assignments are:

```doria
+=
-=
*=
/=
%=
<<=
>>=
&=
|=
^=
```

Each compound assignment uses the same typing, runtime, and panic rules as its corresponding binary operator.

### Deterministic panic messages

The integer operations and conversions use these exact panic messages:

```text
integer overflow during addition
integer overflow during subtraction
integer overflow during multiplication
integer overflow during negation
integer division by zero
integer division overflow
integer remainder by zero
integer shift count out of range
integer conversion out of range
```

Each panic follows decision 0040: it writes the deterministic panic message and Doria function-name stack trace to stderr, then exits with status 101.

## Alternatives considered

- C-style integer promotions and implicit widening were rejected because they would make mixed-width expressions depend on coercion rules instead of explicit Doria types.
- Backend- or host-defined division, remainder, shift, and overflow behavior was rejected because every supported execution path must preserve the same Doria-visible result.
- Treating `xor` as a bitwise synonym was rejected because Doria keeps boolean `xor` distinct from integer `^`.
- Applying checked-arithmetic overflow rules to valid fixed-width left shifts was rejected; a valid left shift is a bit operation that discards high bits.

## Consequences

- Operator checking must know the canonical integer type before lowering.
- Contextual literals can make a mixed literal/value expression well typed, but nonliteral values never convert implicitly.
- MIR and every execution backend must preserve signedness and bit width for integer operations.
- The interpreter, native backend, and PHP compatibility backend must agree on results, panic text, stack trace, and exit status.

## Affected components

- Lexer, parser, AST, and HIR operator representations and precedence
- Semantic typing and contextual integer-literal analysis
- MIR representation and lowering
- MIR interpreter and Cranelift lowering
- `doria-rt` panic paths
- PHP compatibility-backend lowering
- Compiler diagnostics, LSP/editor support, examples, and differential tests
