# Decision 0074: Stage 17 stdio and Formatted I/O

Status: Accepted

> Terminology note (added post-acceptance). This record predates record 0085, which settled stdlib namespace spelling. Forward references to the terminal and I/O modules originally read std::term / std::io — an informal Rust-shaped shorthand that was never a decided spelling — and have been corrected to Doria\Std\Term / Doria\Std\Io. The decision this record makes is unchanged.

## Context

Stage 16 established immutable UTF-8 runtime strings and canonical primitive display conversion
across the interpreter, Cranelift, LLVM, and `doria-rt`. Stage 17 needs synchronous text I/O and
compile-time-checked formatting without exposing platform handles or prematurely designing the
future `Doria\Std\Term` API. Existing accepted decisions already occupy 0071 through 0073, so this
record uses the next unused repository number. The future terminal decision receives its number
when it is authored.

## Decision

### Source surface

Stage 17 adds compiler-known free functions that cannot be redeclared:

- `read_line(): ?string`
- `sprintf(string-literal-format, ...arguments): string`
- `printf(string-literal-format, ...arguments): void`
- `read_file(string $path): string`
- `write_file(string $path, string $contents): void`
- `write_stderr(string $value): void`

These are a narrow v0 text-I/O facade, not the final `Doria\Std\Io` object API. `print` is permanently
rejected with guidance to use `echo`. `printf` writes exact bytes, adds no newline, and returns
void. Format arguments evaluate left-to-right exactly once. Stage 17 requires a direct string
literal format; interpolated and local formats are rejected.

The accepted stdin spelling is `read_line`, not PHP's fused `readline`. The unknown-function
diagnostic consults shared PHP-to-Doria spelling data and offers `read_line` as a fixit. The same
data is intended for the future `doriac migrate php` command.

### Optional-string seed

Stage 17 introduces the Stage 22 nullable model early in this one return position and implements
`?string` through declarations, inference, parameters, returns, assignments,
arguments, calls, MIR, and native ABI. Native null is a null runtime-string pointer; a non-null
value is a normal `DrStringV1` pointer, and the existing retain/release ABI remains null-safe. An
empty string is never null. `string` and `null` assign to `?string`; a possibly-null string does
not assign to `string` and is not display-convertible. Equality and inequality with null are
supported. A `$value != null` true edge narrows the binding to string until assignment invalidates
the fact. This is not an I/O-only optional type and will not be replaced; Stage 22 generalizes the
same model. Ordered nullable comparison and the remaining Stage 22 feature set remain deferred.

### Line input

`read_line` reads stdin synchronously. It removes one LF or one CRLF line ending and no other
whitespace. Empty lines return a non-null empty string. EOF after bytes returns the final line;
EOF before bytes returns null. Embedded NUL is preserved. Invalid UTF-8 panics with `stdin
contained invalid UTF-8`; an OS failure panics with `failed to read stdin`. Buffered bytes beyond
one newline remain available for the next call.

### Text files and stderr

`read_file` reads a complete file, validates UTF-8, and preserves its bytes without newline
normalization. `write_file` creates or truncates and writes exact bytes. Paths reject embedded
NUL. Failures panic with stable messages: `failed to read file`, `file contained invalid UTF-8`,
`failed to write file`, or `file path contained an embedded NUL`. `write_stderr` writes exact bytes
without a newline. I/O errors are fatal status-101 panics until checked errors land at Stage 29.
At that stage these free functions migrate to declared `throws` signatures; `null` from
`read_line` remains EOF only. The text/binary/stream tiers and migration are recorded separately
in Decision 0075.

### Checked formatting

The compiler parses a literal format once and places a validated backend-independent plan in MIR.
Backends never parse format text and never call C or PHP varargs formatters. Supported conversions
are `%s`, `%d`, `%f`, `%.Nf`, `%x`, `%X`, `%o`, `%b`, and `%%`, with fixed `u32` width/precision,
`-` alignment, and numeric `0` padding. Duplicate flags, invalid combinations, positional or
dynamic width, unsupported precision, `%e`, `%g`, unknown conversions, and trailing `%` are
compile errors.

`%s` accepts Stage 16 display-convertible primitives and applies byte-counted width. `%d` accepts
all integers and zero-pads after a minus sign. Integer bases use the declared fixed-width
two's-complement bit pattern for negative signed values. `%f` accepts float32/float64, defaults to
six fractional digits, uses round-to-nearest ties-to-even from the actual source width, is
locale-independent, preserves signed zero, and uses `NaN`, `Infinity`, and `-Infinity` for special
values. Formatting logic is shared by the interpreter and runtime.

### MIR and ownership

MIR represents nullable strings, null checks, stdin/file/stderr operations, and validated format
plans without backend or operating-system handles. Shared MIR validation checks all operand,
result, format-plan, and optional-string invariants before either native backend runs. Runtime
strings produced by input, file reads, and formatting follow the Stage 16 ownership contract;
normal returns release non-returned locals and panic remains abort-only.

### Runtime I/O layering

`doria-rt` separates:

1. raw standard-device reads, writes, explicit stdout/stderr flush, and independent stdin/stdout/
   stderr interactivity queries;
2. UTF-8 text and read_line line discipline, including buffering and newline removal;
3. language operations used by echo, printf, write_stderr, read_line, panic, and future stdlib
   wrappers.

Raw reads never detect newlines. Flush may be an intentional no-op while raw writes are
unbuffered; it is not replaced by `fsync`. Unix interactivity uses `isatty` or an equivalent.
Windows interactivity uses `GetConsoleMode` or an equivalent and distinguishes console handles
from redirected files and pipes.

On Windows, redirected stdout/stderr use exact length-delimited UTF-8 `WriteFile`. Console output
validates UTF-8, converts to UTF-16, and uses `WriteConsoleW`; interactive console input uses a
Unicode-aware wide path, while redirected stdin reads UTF-8 bytes. Echo, printf, write_stderr, and
panic share this substrate. Linux, macOS, and Windows land together.

### Interpreter and parity

The MIR interpreter uses a reusable I/O host. Tests use deterministic stdin and an in-memory
filesystem; CLI/debug execution may use a system host. Durable parity sidecars seed stdin and files
and compare stdout, stderr, status, and resulting files in isolated working directories across the
interpreter, Cranelift, and LLVM.

### Future terminal boundary

Stage 17 does not implement `Console`. The future canonical API is the static `Doria\Std\Term Console`
facade, and its decision number is assigned when authored. Stage 17 exposes no public TTY query,
handles, descriptors, terminal encodings, ANSI semantic values, raw-mode API, key events, cursor
or styling operations, terminal screen cache, `Grid`, or `ScreenBuffer`. Raw ANSI may be a private
Unix implementation detail but is never the public terminal model. TermUtil is a design reference,
not the runtime architecture. Stage 46 can build on the Stage 17 raw-device substrate without
replacing it; raw-mode ownership/destruction, key and resize events, cursor/style/screen behavior,
and the stateless-versus-ScreenBuffer choice remain deferred.

## Consequences

- Stage 17 is one end-to-end checked-HIR-to-MIR-to-runtime capability across all native profiles.
- PHP compatibility may implement exact subsets but must diagnose shapes it cannot preserve.
- `Bytes`, binary I/O, handles, async I/O, `sscanf`, dynamic formats, full expression
  interpolation/`Displayable`, and general nullable types remain deferred.
- The internal runtime ABI grows but remains private and versioned with `dr_v1_*` symbols.

## Affected components

Lexer/parser/type model, semantic analysis and narrowing, HIR/MIR and shared validation, MIR
interpreter host, Cranelift, LLVM, PHP compatibility, `doria-rt` device/line/file/format layers,
LSP/editor tooling, examples, durable parity, CI, documentation, and leak checks.
