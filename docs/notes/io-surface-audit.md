# I/O surface completeness audit

> Documentation role: working note / findings for Andrew's decision. This is a
> design-completeness audit, not a decision record and not an implementation.
> Every open question below is a stop-and-ask: options and a marked recommendation
> are given, but nothing here is decided. Where an item is already settled, it is
> cited and left alone. Approved resolutions become plan/SPEC amendments and
> decision records in a later pass.

## Read (authoritative sources consulted)

- `AGENTS.md` — blast-radius, two-clocks, verifying-claims, documentation-authority rules.
- `docs/doria-end-to-end-plan.md` — §8.6 platform tiers (every syscall lands with its Windows impl same-stage); §9 stdlib (the `Doria\Std\Io` / `Doria\Std\Fs` / `Doria\Std\Term` modules, the formatted-I/O minimal set, the three-tier file family); §3.1 RAII / §3.6 panic; §4.6 strings & `Bytes`; §5 error propagation runs `__destruct`.
- `SPEC.md` — "Stage 17 text I/O and checked formatting" (the intrinsic signatures, `read_line`/`read_file`/`write_file`/`write_stderr` contracts, runtime layering), panic path, non-goals ("public stream/file objects, binary I/O, terminal APIs beyond Stage 17 helpers").
- `docs/decisions/` — **0074** (Stage 17 stdio & formatted I/O), **0075** (I/O family tiers & failure-semantics migration), **0045** (runtime strings/`Bytes`/canonical display), **0006** (console/terminal — deferred), **0081** (abort-only panic runs no cleanup), **0035** (checked throw/throws direction).
- Code (verifying claims): `crates/doriac/src/builtins.rs` (shipped intrinsic set), `crates/doriac/src/mir_lowering.rs` (echo lowering), `crates/doria-rt/src/lib.rs` (`ignore_sigpipe` → EPIPE), `crates/doria-rt/src/device_io.rs` (broken-pipe detection).

**Shipped surface today:** `echo` (statement), `read_line(): ?string`, `sprintf`, `printf`, `read_file(string): string`, `write_file(string, string): void`, `write_stderr(string): void`. `read_file_bytes`/`write_file_bytes` are **planned (Stage 23), not shipped**.

## Already settled (item → citation)

- **Three-tier file family** (text now / `Bytes` Stage 23 / `File`+stream post-Stage-29) — 0075 §Decision; plan §9.
- **`write_file` truncates** ("creates or truncates and writes exact bytes") — 0074 §Text files; SPEC "Stage 17" (`write_file creates or truncates`). *Truncate is defined and shipped; only append is open (Q3).*
- **Text tier does no newline normalization; byte-exact read & write** — 0074 ("preserves its bytes without newline normalization"; "writes exact bytes"). `read_line` strips exactly one LF or one CRLF at the line boundary — 0074 §Line input; SPEC.
- **Invalid UTF-8 on read → panic, no lossy/replacement path** ("file contained invalid UTF-8" / "stdin contained invalid UTF-8") — 0074. Raw/undecoded bytes are the `Bytes` tier's job (Stage 23), not a lossy string path.
- **Text-tier failure model:** panic + status 101 until Stage 29, then declared `throws`; `read_line` `null` = EOF only, never error — 0074, 0075.
- **Terminal layer deferred and bounded:** capability-based `Console` static facade, no escape sequences/handles/ANSI in any public value, Stage 46 build-out, decision number assigned when authored — 0074 §Future terminal boundary; plan §9; 0006.
- **RAII flush/close on normal exit and on `throws` propagation** (drop elaboration runs `__destruct` at every scope boundary) — plan §3.1, §5. **Abort-only panic runs no cleanup** — 0081 (this is the root of D6).
- **SIGPIPE is ignored at the runtime** so a closed-pipe write reports EPIPE instead of killing by signal — `doria-rt/src/lib.rs:940` (impl). *The language-level contract for what happens to that EPIPE is not specified — that is D1.*
- **Binary-tier parameters beyond the path are a Stage 23 decision** — 0075; SPEC.
- **Windows:** redirected stdout/stderr write exact length-delimited UTF-8; interactive console validates UTF-8 → UTF-16 → `WriteConsoleW`; all three OSes land together — 0074 §Runtime I/O layering.

## Open questions (the six named + everything derived)

Format per item: **Status · Options · Tradeoffs · Recommendation (marked) · Blast radius.**

### Q1 — stdout byte-write surface / the stderr asymmetry [OPEN]
- **Status.** `echo` writes text to stdout (display-converted, no newline — verified in lowering); `write_stderr(string)` writes exact text bytes to stderr. There is **no byte-level output to either stdout or stderr**. The asymmetry (`write_stderr` exists, no `write_stdout`) is real but, for *text*, cosmetic: `echo` is the stdout text writer.
- **Options.** (a) Add `write_stdout(string)` now to mirror `write_stderr`. (b) Treat the text asymmetry as intentional (`echo` = the one text-stdout spelling; `write_stderr` = the error-channel escape hatch) and name the **byte** path as a Stage 23 `Bytes`-tier addition: `write_stdout_bytes(Bytes)` + `write_stderr_bytes(Bytes)`. (c) Leave byte output to files only.
- **Tradeoffs.** (a) duplicates `echo` — reintroduces the exact `print`/`echo` redundancy Doria bans. (b) closes the *real* gap (binary piping, non-UTF-8) with symmetric names and honors "one output spelling." (c) leaves binary pipelines impossible.
- **Recommendation → (b).** Do **not** add `write_stdout(string)`. Declare the text asymmetry intentional (`echo` is stdout text; `write_stderr` is the stderr text escape hatch), and reserve `write_stdout_bytes(Bytes): void` and `write_stderr_bytes(Bytes): void` for the Stage 23 `Bytes` tier. This names the byte path explicitly and keeps the naming symmetric where it matters (bytes).
- **Blast radius.** Stage 23 scope; 0075 tier-2 text, plan §9 three-tier bullet, SPEC. No shipped signature changes.

### Q2 — stdin byte tier [OPEN]
- **Status.** `read_line` (text stdin), `read_file` (text file), `read_file_bytes` (binary **file**, Stage 23). No byte-level **stdin** read; the binary tier as written covers files only.
- **Options.** (a) `read_stdin_bytes(): Bytes` free function in the Stage 23 `Bytes` tier (whole-stdin slurp), sibling of `read_file_bytes`. (b) byte stdin only via the post-Stage-29 stream tier.
- **Tradeoffs.** (a) completes the binary tier across file **and** stdin at one stage; whole-stdin slurp only (chunked reads wait for streams). (b) leaves binary piping impossible until post-Stage-29, inconsistent with `read_file_bytes` landing at Stage 23.
- **Recommendation → (a),** with chunked/incremental byte reads deferred to the stream tier. The binary tier should cover stdin the moment it covers files.
- **Blast radius.** Stage 23 scope; 0075 tier-2, plan §9, SPEC.

### Q3 — `write_file` append vs truncate [PARTIALLY SETTLED — truncate done; append OPEN · HIGH PRIORITY]
- **Status.** Truncate is **already decided and shipped** (0074/SPEC: "creates or truncates"). The gap is append, and how to spell a mode without an options bag.
- **Options.** (a) A distinct free function `append_file(string $path, string $contents): void` (verb_noun charter, no flag). (b) A mode enum arg on `write_file` (`write_file(path, contents, WriteMode::Append)`). (c) Defer append to the stream tier (open a `File` in append mode, post-Stage-29).
- **Tradeoffs.** (a) matches the `read_file`/`write_file` naming family, no options bag, and **requires no change to `write_file` — so no breaking change**. (b) changes a shipped signature (breaking) and starts the options-bag slide. (c) leaves the common "append a line to a log" case with no free-function answer until post-Stage-29.
- **Recommendation → (a) `append_file`.** Crucially, this **defuses the breaking-change concern the prompt flags**: `write_file` stays truncate-only exactly as documented, and append is *additive*. Decide the name now so the Stage 23 binary tier can mirror it (`append_file_bytes`). Implementation is a separate, later prompt.
- **Blast radius.** Adds one intrinsic (`builtins.rs`, MIR, three backends, `doria-rt` file layer — which already opens for write); plan §9, 0074/0075, SPEC surface list. `write_file` **unchanged**. This is the one item whose *approval* is higher priority (a shipped sibling), though its code is deferred.

### Q4 — path typing [OPEN — deferrable, with a flagged cost]
- **Status.** Every I/O signature takes `string $path`; there is **no Doria `Path` type** (verified). `Doria\Std\Fs` is listed as a future module; the namespace decision makes single-quoted strings the home for Windows paths.
- **Options.** (a) Path is permanently `string`; cross-platform manipulation (join/normalize/split) lives in `Doria\Std\Fs` as free functions over strings. (b) A `Path` value type (in `Doria\Std\Fs`) wrapping a string with join/normalize/component methods, accepted by I/O signatures — the `Sql`-newtype pattern applied to path correctness. (c) Defer the type question to the `Doria\Std\Fs` design.
- **Tradeoffs.** (a) simplest; but a bare `string` puts separator/normalization correctness on the *program*, which conflicts with the tier-1 Windows promise. (b) makes path-join correctness a type property (hard to get platform-wrong), consistent with Doria's provenance-newtype instinct; costs a type and conversions at the boundary. (c) keeps options open but leaves portability guidance unwritten.
- **Recommendation → (c) defer to `Doria\Std\Fs`, leaning (b).** Path stays `string` at the raw I/O free-function tier (they hand the path to the OS). Record the portability implication **now**: until `Fs` exists, cross-platform path handling is unaided, which is a tier-1 gap. Flag `Path`-as-type as a live design case for `Fs`, leaning toward a value type. **Reopen trigger:** authoring `Doria\Std\Fs`.
- **Blast radius.** None today (all signatures already take `string`). Future: if `Path` becomes a type I/O signatures accept, that is a future signature evolution — so design the `Fs` boundary knowing it may narrow path parameters later.

### Q5 — buffering and flush + stdout/stderr ordering [PARTIALLY SETTLED — unbuffered now; public flush + ordering OPEN]
- **Status.** Raw device writes are **unbuffered** today; the raw-layer flush "may be an intentional no-op … not `fsync`" (0074). The stream tier is described as "buffered." There is **no public flush** and **no stated stdout/stderr ordering guarantee**.
- **Options.** (a) Keep the free-function tier unbuffered; put buffering + an explicit `flush()` **method** on the post-Stage-29 stream objects (where `__destruct` flushes). (b) Buffer stdout now (line-buffered on a TTY, block-buffered when piped, C-stdio style) + a public `flush` free function.
- **Tradeoffs.** (a) no lost-output surprises, simplest, matches current behavior; interleaving is naturally in write order. (b) throughput for chatty output, but reintroduces flush bugs and the abort-panic data-loss surface (D6) at the free-function tier.
- **Recommendation → (a).** Keep free-function output unbuffered; buffering + `flush()` are stream-tier object methods. **State the ordering guarantee explicitly:** because free-function output is unbuffered, stdout and stderr appear in exact write order with no reordering. Do not add a public flush free function (nothing to flush while unbuffered). **Reopen trigger:** stream-tier design.
- **Blast radius.** Mostly documenting an existing guarantee (0074/SPEC); stream-tier flush is post-Stage-29.

### Q6 — standard streams as first-class values [OPEN — post-Stage-29]
- **Status.** Tier 3 is "`File` and stream objects"; the plan says "richer stdin APIs live on the post-Stage-29 `Doria\Std\Io` stream types." Whether `stdin`/`stdout`/`stderr` **themselves** become stream values (passable, storable) is undecided.
- **Options.** (a) **Unify:** the standard streams are obtainable as stream objects (e.g. `Io::stdout(): Stream`), so code can write generically to "a stream"; the intrinsics (`echo`, `read_line`, `write_stderr`) stay as the ergonomic fast path over the same underlying device layer. (b) **Parallel:** the stream tier is for `File`/opened streams only; the standard streams remain intrinsic-only forever.
- **Tradeoffs.** (a) makes I/O composable — a filter can target a file or stdout through one type; must share the doria-rt device layer with the intrinsics to avoid double-buffering/ordering hazards (they already share it per 0074's layering). (b) simpler surface, but every stream-consuming API must special-case the standard streams; a CLI/service language will feel this constantly.
- **Recommendation → (a) unify,** designed so the intrinsics and the stream accessors sit on the same doria-rt substrate. **Reopen trigger:** `Doria\Std\Io` stream-tier design.
- **Blast radius.** Post-Stage-29 stream tier; the stream type and standard-stream accessors must be designed together.

### D1 — closed stdout / EPIPE / SIGPIPE (the `head` case) [OPEN · IMPORTANT · touches shipped behavior]
- **Status.** Derived, not in the six. SIGPIPE is ignored (verified), so a closed-pipe write yields EPIPE. Today an EPIPE surfaces as an OS write failure → the text-tier **panic path → status 101 + a stderr stack trace**. For a filter piped into `head`, that turns a *normal* early-close into a crash dump. No document specifies this.
- **Options.** (a) Broken-pipe/EPIPE on a **standard stream** → **clean exit** (status 0, no panic trace) — the Unix-filter convention (coreutils, Go, Rust-with-reset-handler). (b) Keep the current panic (status 101 + trace). (c) Exit 141 (128+SIGPIPE), no trace.
- **Tradeoffs.** (a) correct for the CLI-filter product; a program piping into `head`/`less` behaves. (b) treats a normal scenario as a crash — unacceptable for product 1. (c) preserves the "signal" convention but still noisy for scripts expecting 0.
- **Recommendation → (a).** A write failing with broken-pipe/EPIPE on `stdout`/`stderr` terminates the program cleanly, no panic trace. Keep genuine file write failures on the panic→`throws` path; broken-pipe on a standard stream is a *carve-out*, not a throw the user must handle. This is a **must** for the CLI-filter use case and, like Q3, touches shipped behavior — so it needs an early decision even though the code is a separate prompt.
- **Blast radius.** `doria-rt` device write path (EPIPE / `ERROR_BROKEN_PIPE` → clean exit); 0074/0075 failure model (carve broken-pipe out of the generic write-panic); the 0074 panic-message set; SPEC; any example/fixture asserting write-failure panics. **Code follow-up required** (second such item alongside Q3).

### D2 — binary stderr write [OPEN — empty cell]
- **Status.** `write_stderr` is string-only; no `Bytes` path to stderr.
- **Recommendation.** Covered by Q1(b): reserve `write_stderr_bytes(Bytes)` beside `write_stdout_bytes(Bytes)` at the Stage 23 tier. **Blast radius:** Stage 23.

### D3 — byte-tier (Stage 23) failure model not explicit [minor gap]
- **Status.** 0075 states the *text*-tier failure model precisely but does not spell out the Stage 23 byte functions'.
- **Recommendation.** State that byte-tier functions follow the same model — panic until Stage 29, then migrate to `throws` with the text functions — so it is not re-invented at Stage 23. **Blast radius:** one-clause 0075/SPEC clarification.

### D4 — partial writes / short reads / EINTR [OPEN — contract clarification]
- **Status.** The free-function tier is all-or-nothing (whole file / whole line; failure panics). Partial writes, short reads, and `EINTR` are handled inside `doria-rt` and never surface. **Verify:** confirm `doria-rt` actually retries `EINTR` and loops short writes to completion (I did not audit every write loop — flag for the implementer).
- **Options.** (a) Free-function tier stays all-or-nothing by contract (retry/loop hidden; genuine failure panics/throws); partial-I/O *counts* are a stream-tier concern (`read()`/`write()` returning a length). (b) Expose counts at the free-function tier.
- **Recommendation → (a).** Keep partial I/O invisible at the free-function tier; the stream tier exposes counts. **Blast radius:** 0074/SPEC contract clarification; stream-tier design; a `doria-rt` verification pass on the write/read loops.

### D5 — BOM policy [derivable — state it]
- **Status.** Unspecified. Derivable from byte-exactness: `read_file` preserves bytes, so a leading UTF-8 BOM (`EF BB BF`) enters the string as `U+FEFF`; nothing strips it.
- **Recommendation.** State explicitly: **no BOM stripping** (a BOM is data); a future text helper in `Doria\Std\Fs`/`Io` may offer a BOM-aware reader. Consistent with "byte-exact, no normalization." **Blast radius:** one-line 0074/SPEC clarification.

### D6 — abort-only panic vs unflushed buffered writes [state honestly]
- **Status.** Follows from 0081 (abort-only panic runs no cleanup). The **current free-function tier is unbuffered, so it has no exposure.** The future **buffered stream tier does**: a panic while a buffered stream is open drops the unflushed buffer (no `__destruct` → no flush) — real data loss.
- **Options.** (a) Accept it and document it (durability needs an explicit `flush()` before risky work). (b) Offer a write-through/unbuffered stream mode for durability-sensitive writers. (c) A panic hook that best-effort flushes (fragile; fights abort-only).
- **Recommendation → (a), with (b) available.** Document the data-loss window plainly; let durability-sensitive code choose an unbuffered stream or flush explicitly. Do not add a panic hook. **Blast radius:** stream-tier design note; cross-reference 0081. Not a new decision — a stated consequence.

### D7 — concurrency: sendable streams, cross-task handles, stdout synchronization [DEFER · flag as design cases]
- **Status.** Unaddressed. DDO already flags "connections are not `Sendable`."
- **Recommendation → defer to the async/`Sendable`/`Shareable` decision, but flag three explicit design cases:** (1) is a `File` handle `Sendable` (movable across tasks)? (2) is shared stream access allowed only via `SharedMut`? (3) is stdout writing synchronized across concurrent tasks (interleaving hazard)? Lean: stdout writes should be process-globally synchronized so concurrent `echo` cannot interleave mid-write. **Reopen trigger:** async decision. **Blast radius:** async-decision design cases; no current impact.

### D8 — seek/tell/truncate/metadata + the `Doria\Std\Io` vs `Doria\Std\Fs` line [OPEN — draw the line]
- **Status.** The stream tier mentions seek; tell, truncate-in-place, size, existence, permissions, timestamps, directory ops are unplaced. The plan lists both `Io` and `Fs` without a boundary.
- **Recommendation (a line, marked as a recommendation).** Adopt the proven `std::io` vs `std::fs` split:
  - **`Doria\Std\Io`** — operations *through an open handle* (stream state): read, write, `seek`, `tell`, `flush`, `close` (RAII), truncate-in-place on an open handle. Stream-object methods (post-Stage-29).
  - **`Doria\Std\Fs`** — operations *on the filesystem namespace* without an open handle: existence, size/metadata (`stat`), permissions, timestamps, rename, delete, `mkdir`, directory listing, **and** path manipulation (join/normalize/split/extension — see Q4). Free functions and/or a `Path` type (unscheduled).
- **Blast radius.** Scopes two future modules and tells each operation which one it belongs to; prevents overlapping/ambiguous module design. No current impact.

## Empty cells found in the tier matrix

Matrix = {read, write} × {file, stdin, stdout, stderr} × {text, binary}:

| | file | stdin | stdout | stderr |
|---|---|---|---|---|
| read text | `read_file` ✓ | `read_line` ✓ (line-oriented; whole-stdin-text slurp ⚠ minor gap) | n/a | n/a |
| read binary | `read_file_bytes` ✓ (S23) | **❌ Q2** | n/a | n/a |
| write text | `write_file` ✓ (truncate) · **append ❌ Q3** | n/a | `echo` ✓ | `write_stderr` ✓ |
| write binary | `write_file_bytes` ✓ (S23) · **append ❌** | n/a | **❌ Q1** | **❌ D2** |

Empty cells: **binary stdin read (Q2), binary stdout write (Q1), binary stderr write (D2), text file append (Q3), binary file append (Q3 sibling)**; minor: **whole-stdin text slurp** (covered by looping `read_line`).

## Recommended deferrals (reason · reopen trigger)

- **Byte std-stream I/O** (Q1/Q2/D2) — deferred to the **Stage 23 `Bytes` tier**; names reserved now. *Reopen:* Stage 23.
- **`Path` type** (Q4) — deferred to **`Doria\Std\Fs`** design; portability cost recorded now. *Reopen:* authoring `Fs`.
- **Public flush + stream buffering** (Q5) — deferred to the **stream tier**; unbuffered + write-order guarantee stated now. *Reopen:* `Io` stream design.
- **First-class standard streams** (Q6) — deferred to **post-Stage-29**; unify-direction recommended. *Reopen:* `Io` stream design.
- **Stream concurrency / `Sendable`** (D7) — deferred to the **async decision**; three design cases flagged. *Reopen:* async authoring.
- **`Io`/`Fs` operation placement** (D8) — the *line* is recommended now; the *operations* land `Io` (post-29) / `Fs` (unscheduled).

**Not deferrable (decide soon — touch shipped behavior):** **Q3** (`append_file` name — additive, non-breaking) and **D1** (closed-stdout/EPIPE clean-exit — changes the shipped write-panic behavior). Both need decisions now; both have a *separate* implementation prompt.

## Invalidated elsewhere (if the recommendations are approved)

- **0074** — add: byte std-stream writers (Stage 23); `append_file` as the append spelling; broken-pipe carve-out from the write-panic (D1); no-BOM-strip and no-newline-normalization stated (D5); unbuffered + stdout/stderr write-order guarantee (Q5). Panic-message set changes for the stdout-broken-pipe case.
- **0075** — add byte stdin/stdout/stderr to tier-2; state the byte-tier failure model (D3); note append across tiers.
- **SPEC.md** "Stage 17 text I/O" — mirror the 0074 clarifications; state closed-stdout behavior.
- **plan §9** formatted-I/O / three-tier bullets — `append_file`, byte std streams, the `Io`/`Fs` line, EPIPE behavior.
- **`doria-rt`** — device write path: EPIPE/`ERROR_BROKEN_PIPE` → clean exit (D1, code); verify EINTR-retry / short-write loops (D4).
- **`write_file` signature — explicitly UNCHANGED** (append is additive; no breaking change). Recorded so no later pass "adds a mode."
- **Examples / fixtures / website & playground** — any example that pipes into `head` or asserts write-failure panics; the parity manifest's broken-pipe handling already treats `BrokenPipe` as ok in the harness (`native_mir_parity_tests.rs:255`) — reconcile with the chosen D1 semantics. No current playground example uses append or binary std streams (confirm before amending).

## Proposed deliverable path

`docs/notes/io-surface-audit.md` (this file) — a findings note under "supporting context" per `docs/information-architecture.md`. It is **not** a decision record: records are for settled decisions, and every item here is a stop-and-ask. On Andrew's approval, the subset he accepts becomes plan/SPEC amendments and one or more decision records (next free number, subject-cited until authored, `scripts/check_docs_authority.php` green), and the two code items (Q3 append, D1 EPIPE) become a separate implementation prompt.
