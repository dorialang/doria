# Decision 0075: I/O Family Tiers and Failure-Semantics Migration

Status: Accepted

## Context

Stage 17 introduces synchronous text I/O before Doria has `Bytes`, checked errors, or owned stream
objects. Treating its file helpers as a complete file API would either weaken the UTF-8 invariant
of `string` or force future binary and streaming work into the wrong signatures.

## Decision

Doria's file I/O family has three deliberate tiers:

1. Stage 17 text functions: `read_file(string $path): string` and
   `write_file(string $path, string $contents): void`. Reads validate UTF-8 before constructing a
   string. Invalid bytes never enter `string`. Decision 0091 specifies the additive
   `append_file(string $path, string $contents): void` spelling without changing truncate-only
   `write_file`; its implementation lands at Stage 23.
2. Stage 23 binary functions: `read_file_bytes(string $path, ...)` and
   `write_file_bytes(string $path, ...)`, plus the reserved `append_file_bytes` sibling,
   introduced with the `Bytes` move type. A file operation always requires its path; any
   additional parameters and their complete contracts are settled in Stage 23 rather than
   inferred early from the text API.
3. Post-Stage 29 `File` and stream objects: owned RAII resources with buffered and seekable access.

Until checked errors land, Stage 17 free-function I/O failures panic with a clear message and status
101. `read_line(): ?string` returns `null` only for EOF; it does not encode failures as null. At
Stage 29 the I/O free functions migrate to declared `throws` signatures. That planned signature
change does not alter successful text behavior or the meaning of EOF. A closed stdout or stderr
pipe is the permanent decision-0091 exception: it exits cleanly with status 0 and is never a
Stage-29 throw. File failures and non-broken-pipe standard-stream failures remain on the migration
path.

Stage 17 `?string` is the first supported position for the nullable model generalized at Stage 22.
It is not a special I/O-only type and is not replaced by the later general implementation.

## Consequences

- Text APIs preserve Doria's immutable UTF-8 string invariant.
- Binary data waits for the ownership-aware `Bytes` representation instead of entering strings.
- Recoverable I/O uses Doria's checked-error model once that model exists, without inventing a
  temporary nullable or sentinel error convention.
- Stream ownership and destruction are designed with checked errors rather than retrofitted onto
  Stage 17 compiler-known functions.

## Affected Components

`SPEC.md`, Stage 17 semantic and runtime tests, `doria-rt` text I/O, future Stage 23 `Bytes` work,
and future Stage 29 checked-error and stream design.
