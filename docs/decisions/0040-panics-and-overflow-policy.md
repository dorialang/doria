# 0040 Panics and overflow policy

Status: Accepted

## Decision

A Doria panic is a fatal runtime condition. It is distinct from the checked `throw` / `throws` error model: panics are not catchable, do not unwind, and do not run cleanup or destructors while aborting in v1.0.

Panic writes a deterministic structured Doria runtime outcome, then exits with
status 101. Decision 0109 supersedes the early function-only envelope with the
global diagnostic grammar:

```text
Panic[Pxxxx]: <Title>

Where
<project-relative source location and labelled preview>

Why
<explanation>

Call Path
<source-aware Doria frames>

Process Exited With Status 101
```

Every implemented built-in panic has a stable central-catalogue code and a
reason-specific source span. User-authored `panic(...)` has one stable generic
identity and preserves its message exactly.

The explicit spelling is:

```doria
panic("message");
```

`panic` is a compiler-known built-in free function/intrinsic, not a keyword. User code cannot redeclare it. Stage 12 accepts string literals, readonly compile-time-known string locals, and concatenations of that same string-expression subset as panic messages.

Checked integer addition, subtraction, multiplication, and signed negation overflow at runtime by panicking for every Stage 13 width and signedness. Division by zero, signed minimum divided by `-1`, remainder by zero, an invalid shift count, and an out-of-range explicit integer conversion also panic. Decisions 0041 and 0042 define the exact conditions and deterministic messages.

Returning a process status outside `0..125` from `main(): int` also panics at runtime. Interpreter and native execution must produce identical structured panic facts, human output, Doria `Call Path`, and status for every supported panic path.

Stage 14 adds one float-related panic reason: `Float::toInt` panics with
`float-to-integer conversion out of range` for NaN, infinity, or a truncated
mathematical value outside the canonical signed 64-bit integer range. Float
arithmetic itself follows IEEE 754 and does not panic for overflow or division
by zero. Indexing panic behavior remains later-stage work.

## Consequences

- Panic is a completed runtime outcome, not a compiler or malformed-MIR error.
- The PHP compatibility backend preserves the structured Doria panic facts and
  status 101; PHP exceptions, frames, and generated line numbers do not define
  or emulate Doria panic semantics. It diagnoses behavior it cannot preserve
  rather than emitting misleading PHP.
- No surface `never` type is introduced by Stage 12.
