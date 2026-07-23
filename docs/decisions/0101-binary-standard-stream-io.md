# Decision 0101: Binary standard-stream I/O

**Status:** Accepted (ratifies io-surface-audit Q1 recommendation (b) and Q2
recommendation (a)). Extends the three-tier I/O family (0074/0075/0091) with the
binary standard-stream functions; implemented at Stage 23 alongside `Bytes` and
the binary file tier.

## Context

The binary I/O tier was authored for **files** only — `read_file_bytes` /
`write_file_bytes` / `append_file_bytes` (0075/0091, Stage 23). The I/O
completeness audit (`docs/notes/io-surface-audit.md`) found two matching gaps in
the **standard streams**:

- **Q1** — there is no byte-level output to stdout or stderr. `echo` writes
  display-converted text to stdout and `write_stderr(string)` writes text to
  stderr, but binary pipelines (piping non-UTF-8 bytes, images, encoded frames)
  have no output path on the standard streams.
- **Q2** — there is no byte-level **stdin** read. `read_line` (text) and
  `read_file_bytes` (binary file) exist, but the binary tier does not cover stdin,
  so a binary pipe *into* the program cannot be read.

This record ratifies the audit's marked recommendations for both, closing the
binary tier across files **and** standard streams at one stage.

## Decision

### The stderr/stdout text asymmetry is intentional — no `write_stdout(string)`

Doria has **one** text-stdout spelling: `echo`. `write_stderr(string)` is the
text escape hatch for the *error* channel; there is deliberately no
`write_stdout(string)` sibling, because it would duplicate `echo` and reintroduce
exactly the `print`/`echo` redundancy Doria bans (0074). The asymmetry is by
design and is not "fixed" — the symmetry that matters is on the **byte** path,
below.

### Binary standard-stream functions (Q1(b), Q2(a))

Three compiler-known, unshadowable intrinsics join the byte tier, siblings of the
binary file functions and recognized before name resolution (plan line 842):

- `write_stdout_bytes(Bytes $contents): void` — writes exact bytes to stdout, no
  newline added, no text/console translation.
- `write_stderr_bytes(Bytes $contents): void` — writes exact bytes to stderr,
  same contract.
- `read_stdin_bytes(): Bytes` — reads **all** of stdin to EOF as raw bytes
  (whole-stream slurp), no UTF-8 validation, no newline normalization. Immediate
  EOF yields an empty `Bytes` (never null — this is a slurp, not a line read, so
  there is no EOF sentinel). `$contents`/the result is a `Bytes` and follows the
  `Bytes` ownership contract.

Chunked/incremental byte reads and byte writes with backpressure are **not** here:
they belong to the post-Stage-29 `Doria\Std\Io` stream tier, exactly as the audit
recommends. This is the whole-stream tier only.

### Failure semantics — inherit the standard-stream model

- **Output** (`write_stdout_bytes` / `write_stderr_bytes`) inherits the ordinary
  standard-stream rule from 0074/0091: a write that reports a **closed pipe**
  during ordinary program output exits immediately with status 0, no panic, no
  trace — the same carve-out `echo` and `write_stderr` already have. Any other
  write failure is a fatal status-101 panic until Stage 29.
- **Input** (`read_stdin_bytes`) panics on an OS read failure with a stable
  message (`failed to read stdin`, matching `read_line`). There is no UTF-8
  validation and therefore no invalid-UTF-8 panic — raw bytes are the point.
- At Stage 29 these migrate to declared `throws` with the rest of the family,
  except the closed-output-pipe carve-out, which is permanent and never thrown
  (0075/0091).

### Platform parity

Byte I/O is byte-exact on Linux, macOS, and Windows and **bypasses** the
text-console path: no UTF-8→UTF-16 `WriteConsoleW` conversion, no newline
translation. On Windows the byte functions use the length-delimited exact-bytes
path (the same substrate 0074 uses for redirected streams), never the interactive
console-wide path. All three OSes land together.

## Alternatives considered

- **Add `write_stdout(string)` (Q1 option a).** Rejected — duplicates `echo`;
  the banned `print`/`echo` redundancy.
- **Byte output to files only (Q1 option c) / byte stdin only via streams
  (Q2 option b).** Rejected — leaves binary pipelines impossible until
  post-Stage-29, inconsistent with `read_file_bytes` landing at Stage 23; the
  binary tier should cover the standard streams the moment it covers files.
- **`read_stdin_bytes(): ?Bytes` with null at EOF.** Rejected — a whole-stream
  slurp has no line boundary, so empty-on-EOF is the correct and simpler contract;
  `?Bytes` would invite a false parallel with `read_line`'s EOF sentinel.

## Consequences

- The binary tier is symmetric across files and standard streams: read/write bytes
  to a file or a pipe with one consistent naming family (`*_bytes`).
- No new text-stdout spelling; `echo` remains the single stdout text writer and
  the asymmetry with `write_stderr` is a documented, deliberate choice.
- These functions depend on `Bytes` (Stage 23) and land with it and the binary
  file tier in the same slice.
- The stream tier (post-Stage-29) still owns chunked/seekable byte I/O; this record
  does not pre-empt it.

## Affected components

The compiler's intrinsic table and name resolution (three new unshadowable
functions), semantic analysis (their `Bytes` signatures), HIR/MIR and shared MIR
validation (no OS handles in MIR; reuse 0074's runtime I/O layering), `doria-rt`
device layer (raw stdin slurp, raw stdout/stderr byte writes, the closed-pipe
carve-out), the interpreter I/O host and durable parity (seeded binary stdin,
captured binary stdout/stderr), plan §9 and the stdlib reference, the
io-surface-audit (Q1/Q2 discharged), and SPEC (updated when implemented).

## Invalidated elsewhere

- io-surface-audit Q1 and Q2 move from OPEN to ACCEPTED, discharged by this
  record; their recommendations (b) and (a) are the decision.
- The stdlib reference's binary-I/O line — now cite 0101 for the standard-stream
  byte functions (it already lists them under Stage 23; the surface is now
  ratified, not merely planned).
- The Stage 23 byte/file I/O slice — its scope now includes these three
  standard-stream functions, no longer fenced off as unratified.
- Any statement that binary I/O is "files only" or that the stdout/stderr text
  asymmetry is an oversight — both are now settled the other way.
