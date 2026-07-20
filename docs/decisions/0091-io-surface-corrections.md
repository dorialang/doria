# Decision 0091: I/O Surface Corrections

Status: Accepted

## Context

The pre-Stage-22 I/O surface audit identified two decisions that affect shipped
behavior or names. Standard-stream writes currently collapse every operating-system
failure into the fatal panic path, so a downstream command closing a pipe early
looks like a Doria failure. Separately, `write_file` is deliberately truncate-only,
but the additive text-file family has no settled append spelling.

These questions must be settled before Stage 29 migrates recoverable I/O failures
to checked errors and before Stage 23 adds the binary file tier. Decisions 0040
and 0081 remain authoritative for abort-only panics, while decisions 0074 and
0075 remain authoritative for the Stage 17 I/O surface and its later failure
migration.

## Decision

### Closed standard-stream pipes

An ordinary Doria program write to stdout or stderr that reports a closed pipe
terminates the process immediately and cleanly with status 0. It emits no panic
and no stack trace.
Native Unix recognizes `EPIPE`; native Windows recognizes
`ERROR_BROKEN_PIPE` and `ERROR_NO_DATA`. Unix continues to ignore `SIGPIPE` so
the write operation can report `EPIPE` instead of terminating by signal.

This rule is limited to ordinary program output on standard streams. Panic
diagnostic writes are best effort and never replace the fatal panic status: if
stderr is closed while a panic is being reported, diagnostic bytes may be absent
but the process still exits with status 101. A non-broken-pipe stdout failure
keeps the existing status-101 panic path, a non-broken-pipe raw stderr failure
keeps its existing status-101 exit, and file-write failures remain status-101
panics. The device layer reports success, broken pipe, or another failure;
process policy belongs to its callers. Compatibility backends must preserve the
same distinction rather than inheriting their host language's default pipe
behavior.

The clean exit is a permanent carve-out, not a recoverable checked error. Stage
29 must not turn a closed standard-stream pipe into a `throw`.

### Append spelling and schedule

The additive text-file spelling is:

```doria
append_file(string $path, string $contents): void
```

`write_file(string $path, string $contents): void` remains unchanged and always
creates or truncates. There is no mode argument or options bag. The binary
sibling is reserved as `append_file_bytes` so the text and `Bytes` tiers mirror
one another.

This record settles names and contracts only. `append_file` and
`append_file_bytes` are both implemented in Stage 23. Their names are reserved
against user declarations, but neither is callable or lowered through MIR,
backends, or the runtime before Stage 23.

## Alternatives considered

### Keep a broken-pipe panic

Rejected. A downstream filter exiting early is normal CLI behavior, not a Doria
program failure, and a panic trace makes pipelines noisy and misleading.

### Exit with status 141 or 128 plus SIGPIPE

Rejected. It preserves signal-shaped failure for scripts that expect a normal
pipeline termination and defeats the clean filter convention.

### Add a write mode to `write_file`

Rejected. It changes a shipped signature and starts an options-bag progression
for an operation with a clear additive verb.

### Defer append until the stream-object tier

Rejected. It leaves the common append-a-log-line operation unanswered until
after Stage 29 even though the free-function shape is small and coherent.

## Consequences

Closed stdout and stderr pipes for ordinary program output behave identically
across Linux, macOS, and Windows and across backends. The immediate status-0
exit runs no RAII `__destruct` cleanup, consistent with both default SIGPIPE termination and
decision 0081's abort-only model. Current free-function writes are unbuffered,
so no runtime buffer is abandoned. When buffered streams are designed after
Stage 29, the audit's D6 buffered-write/abort question must revisit this
interaction; this record adds no panic hook or partial cleanup.

The append family is regular and non-breaking, but its implementation remains a
Stage 23 obligation. All other open audit questions retain their existing
reopen triggers.

## Affected components

`doria-rt` standard-device writes and callers, PHP compatibility output helpers,
native and PHP broken-pipe and panic-sink tests, the interpreter/Cranelift/LLVM
parity harness, Linux/macOS/Windows CI, decisions 0074 and 0075, `SPEC.md`, the
end-to-end plan, and the I/O audit note. No new diagnostic requires a
language-server change.

## Invalidated elsewhere

- Decision 0074 and `SPEC.md` statements that every standard-stream operating-system
  write failure panics.
- Decision 0075 and plan statements that every free-function I/O failure migrates
  to `throws` at Stage 29 without a broken-pipe exception.
- The plan's text-file family where no additive append spelling or Stage 23
  landing was named.
- The native stdout broken-pipe regression that expected status 101 and a panic
  trace.
- The audit's D1 and Q3 open statuses and its corresponding invalidation list.
