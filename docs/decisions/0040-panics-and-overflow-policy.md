# 0040 Panics and overflow policy

Status: Accepted

## Decision

A Doria panic is a fatal runtime condition. It is distinct from the checked `throw` / `throws` error model: panics are not catchable, do not unwind, and do not run cleanup or destructors while aborting in v1.0.

Panic writes a deterministic message and Doria function-name stack trace to stderr, then exits with status 101. Stage 12 established, and Stage 13 retains, this basic output shape:

```text
Panic: <message>
Stack Trace:
  at <currentFunction>
  at <callerFunction>
  at main
```

Source file and line information are not required in the current Stage 13 implementation.

The explicit spelling is:

```doria
panic("message");
```

`panic` is a compiler-known built-in free function/intrinsic, not a keyword. User code cannot redeclare it. Stage 12 accepts string literals, readonly compile-time-known string locals, and concatenations of that same string-expression subset as panic messages.

Checked integer addition, subtraction, multiplication, and signed negation overflow at runtime by panicking for every Stage 13 width and signedness. Division by zero, signed minimum divided by `-1`, remainder by zero, an invalid shift count, and an out-of-range explicit integer conversion also panic. Decisions 0041 and 0042 define the exact conditions and deterministic messages.

Returning a process status outside `0..125` from `main(): int` also panics at runtime. Interpreter and native execution must produce identical panic stderr, Doria stack trace, and status for every supported panic path.

Stage 14 adds one float-related panic reason: `Float::toInt` panics with
`float-to-integer conversion out of range` for NaN, infinity, or a truncated
mathematical value outside the canonical signed 64-bit integer range. Float
arithmetic itself follows IEEE 754 and does not panic for overflow or division
by zero. Indexing panic behavior remains later-stage work.

## Consequences

- Panic is a completed runtime outcome, not a compiler or malformed-MIR error.
- The PHP compatibility backend writes explicit supported panic text to stderr and terminates with 101; PHP exceptions do not define or emulate Doria panic semantics. It diagnoses Stage 13 integer behavior that it cannot preserve exactly instead of emitting misleading PHP.
- No surface `never` type is introduced by Stage 12.
