# 0040 Panics and overflow policy

Status: Accepted

## Decision

A Doria panic is a fatal runtime condition. It is distinct from the checked `throw` / `throws` error model: panics are not catchable, do not unwind, and do not run cleanup or destructors while aborting in v1.0.

Panic writes a deterministic message and Doria function-name stack trace to stderr, then exits with status 101. Stage 12 uses this basic output shape:

```text
panic: <message>
stack trace:
  at <currentFunction>
  at <callerFunction>
  at main
```

Source file and line information are not required in Stage 12.

The explicit spelling is:

```doria
panic("message");
```

`panic` is a compiler-known built-in free function/intrinsic, not a keyword. User code cannot redeclare it. Stage 12 accepts string literals, readonly compile-time-known string locals, and concatenations of that same string-expression subset as panic messages.

Existing checked `int` addition, subtraction, and multiplication overflow at runtime by panicking. Returning a process status outside `0..125` from `main(): int` also panics at runtime. Interpreter and native execution must produce identical panic stderr and status.

Division, modulo, shifts, indexing, numeric conversions, and their panic conditions remain later-stage work.

## Consequences

- Panic is a completed runtime outcome, not a compiler or malformed-MIR error.
- The PHP compatibility backend writes panic text to stderr and terminates with 101; PHP exceptions do not define or emulate Doria panic semantics.
- No surface `never` type is introduced by Stage 12.
