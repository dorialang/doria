# Decision 0079: Stage 18 Expression Interpolation and Displayable

Status: Accepted

## Context

Stage 16 established immutable runtime strings and one canonical primitive display conversion. The initial interpolation parser accepted only variable/property paths. Stage 18 completes ordinary expression interpolation and introduces the class conformance path without pulling native class layout, method dispatch, or the general interface system forward.

## Decision

### Ordinary expression interpolation

Double-quoted interpolation braces contain the ordinary Doria expression grammar. The compiler uses the same lexer, parser, semantic rules, expression AST, checked HIR, and typed MIR operations as the equivalent expression outside a string. It does not maintain a reduced interpolation language or preserve raw expression source in MIR.

Interpolation parts evaluate left-to-right and exactly once. Each embedded expression is evaluated before its canonical display conversion. Literal text retains exact bytes, interpolation adds no newline, and the compiler never evaluates user expressions during compilation.

The native-supported primitive/string subset runs through the MIR interpreter, Cranelift fast profile, and LLVM release profile. The existing ordered MIR string concatenation and scalar display nodes are sufficient; Stage 18 adds no backend-specific interpolation IR.

### Literal braces

In a double-quoted string:

- `\{` is required for a literal opening brace.
- A bare `}` is literal outside an open interpolation.
- `\}` is accepted for symmetry but is not required.
- `{{` and `}}` are not a second escape mechanism.
- An unescaped `{` must begin a valid interpolation expression.
- A bare `{` that does not begin a valid expression is P0002 with the help `write \{ for a literal brace` and a machine-applicable replacement of that brace with `\{`.

Single-quoted strings remain non-interpolating. Malformed interpolation does not fall back to literal text.

This is an accepted pre-1.0 breaking change. The repository's Doria examples and documentation snippets were swept, and the machine-applicable fix repairs the common literal-brace case in one edit.

The compiler-owned PHP migration metadata records that literal `{` text from a PHP double-quoted string must become `\{`, while real migrated interpolation remains interpolation and a bare `}` requires no rewrite. This policy lives beside PHP function-spelling migration data so a future `doriac migrate php` command consumes one source.

### Canonical display conversion

One semantic classifier serves interpolation, `.`, `echo`, and Stage 17 `%s`:

- strings display unchanged;
- signed and unsigned integers display in decimal;
- floats use the deterministic shortest-round-trip conversion from decision 0045;
- bool displays exactly `true` or `false`;
- a class displays only through valid nominal `Displayable` conformance;
- internal recovery `Unknown` prevents cascading diagnostics but is not a Doria value category.

Nullable values without a non-null proof, `mixed`, typed arrays, `List`, `Dictionary`, `Set`, enums, closures, pointers, and `null` are not display-convertible.

Display conversion does not apply to assignment, equality, ordering, arithmetic, ordinary string parameters, paths, conditions, or overload selection.

### Compiler-known Displayable contract

Stage 18 recognizes this narrow compiler-known interface contract:

```doria
interface Displayable
{
    function toString(): string;
}
```

A class conforms only when it explicitly declares `implements Displayable` and supplies exactly:

```doria
function toString(): string
```

The method is an externally accessible readonly instance method with no parameters. It is not `static`, `writable`, or `internal`. Method-name coincidence is not conformance. `to_string`, alternate casing, and `__toString` do not conform. Doria does not use PHP string-conversion magic.

Conformance makes a class value display-convertible in interpolation, `echo`, string-anchored `.`, and `%s`. Each conversion invokes `toString()` exactly once. It does not permit implicit class-to-string assignment.

A nonconforming class in a display context receives E0462 and materially states:

```text
`Token` cannot be displayed; implement `Displayable` with `function toString(): string`
```

Invalid explicit conformance receives E0463 with guidance for the exact method contract. The compiler-known `Displayable` name cannot be redeclared.

### Interface boundary

The class AST and HIR retain an extensible list of declared interface names. Stage 18 activates only `Displayable`. Other conformance receives an honest Stage 35 unsupported-feature diagnostic. General interface declarations, interface-typed values, dispatch tables, default methods, inheritance, traits, and structural conformance remain outside this decision.

### Backend boundary

Frontend conformance and display-context diagnostics are complete in Stage 18. Primitive/string expression interpolation executes through all three native paths.

The PHP compatibility backend lowers the exact subset it can preserve by generating a private backend interface and calling Doria's `toString()` method. It never relies on PHP `__toString` behavior.

Native class values, object layout, ownership, construction, and destruction begin in Stage 19; native instance/static method dispatch begins in Stage 20. A valid Displayable class therefore receives the existing clear unsupported native-class diagnostic before MIR until those stages land. No class placeholder or interface fat pointer is added to Stage 18 MIR.

### Parser resilience

The interpolation boundary scanner tracks balanced braces and quoted strings while preserving original UTF-8 byte offsets. The bounded contents are tokenized by the ordinary lexer and parsed by the ordinary expression parser. Empty, unterminated, malformed, and semantically invalid expressions retain focused diagnostics at their original source positions.

The parser fuzz target feeds arbitrary byte input through UTF-8 replacement decoding and parsing. Its corpus seeds empty input, ordinary and interpolated strings, adjacent and nested expressions, escaped and malformed braces, unterminated interpolation, long inputs, and multibyte offsets. CI runs it with a bounded time budget and per-input timeout.

## Consequences

Stage 18 completes the display architecture established by decision 0045 without changing primitive conversion. AST/HIR retain ordinary expression structure and order, typed MIR remains backend-independent, and all native primitive behavior remains differentially tested.

The strict literal-opening-brace rule deliberately catches ambiguous text rather than silently changing output. General interface work and native class execution remain visibly staged instead of being approximated through PHP behavior or backend convenience.
