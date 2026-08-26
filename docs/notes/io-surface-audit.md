# PHP Stream And I/O Completeness Audit

> Documentation role: completed capability-completeness audit and supporting
> context for accepted decision 0110. This is not an implementation. Candidate
> spellings remain illustrative only; decision 0110 accepts the architecture and
> binding performance/memory contract, and defers exact public spellings through
> its bounded appendix process.

This expanded audit **supersedes the previous partial completeness scope** while
preserving its accepted results, historical snapshots, Q1–Q6 reasoning, derived
findings, and decision references below. Andrew approved all thirteen
architecture recommendations on 2026-08-02; decision 0110 is authoritative for
their semantics and performance constraints. The machine-checkable companion ledger is
`php-stream-capability-inventory.json`; `scripts/check_stream_io_completeness.php`
proves the stored official inventory remains complete without network access.

## Audit authority and snapshot

- **Audit date:** 2026-08-02.
- **PHP manual banner:** `PHP 8.4.24 Released!` — an identifier for the audited
  manual snapshot, **not** a Doria compatibility target.
- **PHP manual copyright:** © 2001–2026 The PHP Documentation Group.
- **Normative inventory sources:** the current official PHP manual pages titled
  *Streams*, *Stream Functions*, *The php_user_filter class*, *The streamWrapper
  class*, *The StreamBucket class*, *Stream Filters*, *Stream Contexts*, *Stream
  Errors*, *Supported Protocols and Wrappers*, *Filesystem Functions*, *Program
  execution Functions*, and *Readline Functions*.
- **Stored counts:** 47 Stream Functions; 25 `streamWrapper` methods; 3
  `php_user_filter` methods; 30 relevant filesystem stream entries; 4 process-pipe
  entries; 13 Readline entries; 12 wrapper/protocol families; 153 total rows.

PHP is the capability and migration inventory, not Doria's semantic authority.
PHP generalizes files, network connections, compression, and other linear I/O
through resources, URL wrappers, filters, contexts, warnings, and sentinel
returns. Doria preserves useful capabilities while replacing those shapes with
owned typed values, checked errors, explicit outcomes, capability contracts, and
domain modules.

## Approval resolution — 2026-08-02

Andrew approved all 13 consolidated recommendations. Every load-bearing semantic
and performance stream decision raised by the review is settled in decision
0110. The remaining deferrals concern public spelling and the named dependent
domains. Stage 26 is unblocked. Stage 36a remains scheduled rather than
implemented and owns the initial performance/memory gate.

This resolution does not rewrite the audit as though the recommendations had
always been accepted. The alternatives, reasoning, and historical partial audit
remain below; decision 0110 is authoritative where their earlier review language
is now stale.

## Existing Doria I/O substrate (verified, not newly implemented)

| Layer | Current fact |
|---|---|
| Text intrinsics | `read_line(string $prompt = ""): ?string`, `read_file`, `write_file`, `append_file`, `write_stderr`, `echo`, `printf`, and `sprintf` are executable. |
| Binary intrinsics | `read_file_bytes`, `write_file_bytes`, `append_file_bytes`, `read_stdin_bytes`, `write_stdout_bytes`, and `write_stderr_bytes` are executable. |
| Standard devices | One internal `doria-rt` abstraction owns stdin/stdout/stderr on Unix and Windows; first-class public stream values do not yet exist. |
| Read discipline | Raw bytes sit below UTF-8 line validation; one LF or CRLF is stripped; EOF before bytes is distinct from a blank line. |
| Write discipline | Unix and Windows loop short writes; Unix retries `EINTR`; raw standard writes are unbuffered. |
| Flush | The standard-output flush is presently an intentional successful no-op over unbuffered writes, not durable filesystem synchronization. |
| TTY | Runtime interactivity detection uses `isatty` on Unix and console-handle detection on Windows. |
| Broken pipe | Unix ignores `SIGPIPE`; Unix `EPIPE` and Windows broken-pipe conditions cleanly exit status 0 for ordinary standard output. |
| Failures | Ordinary I/O failures use decision 0119's canonical checked `Doria\Std\Io` errors. Allocation and other fatal panics retain decision 0109's status-101 outcomes; closed standard pipes retain status 0. |

## Architectural constraints carried into review

1. Preserve capabilities, not PHP spellings. Rejecting a resource, global
   registry, string filter name, context bag, boolean flag, or sentinel does not
   reject the behavior it represents.
2. Generic streams move `Bytes`; UTF-8 text, lines, delimiters, and other
   encodings are explicit typed adapters above that byte foundation.
3. Interfaces expose only the capability required by the caller. Readability
   does not imply writing, seeking, truncating, locking, mode mutation, or socket
   metadata.
4. Non-blocking reads distinguish data, would-block, EOF, and timeout.
   Non-blocking writes report partial progress. Exact enum/result names remain
   deferred under decision 0110.
5. Readiness precedes async. Stage 37 consumes one Stage 36a readiness,
   ownership, timeout/deadline, backpressure, and process-pipe model.
6. First-class standard streams are a v1 design requirement unless review finds
   a compelling objection; they share the current intrinsic/runtime substrate.
7. Dynamic wrappers, filters, and process-global contexts are replaced by typed
   constructors, adapters, and explicit configuration.
8. `Console` remains the owner of raw mode, key/resize decoding, cursor, screen,
   color, and styling. It reuses generic readiness and standard devices.

## Capability-family review

### 1. Stream ownership and lifetime

Stream handles are owned move values by default; readonly/writable borrows grant
only their declared access, and `take` transfers the close obligation. Normal
scope exit and checked-error propagation close through deterministic destruction.
Explicit close/finish consumes the handle and reports checked failure; repeat use
is a compile-time error. Destructor cleanup is best-effort and nonthrowing.
Adapters own their underlying handle unless explicitly constructed as borrowing.
Half-shutdown remains a duplex/process/network capability. Abort-only panic runs
no cleanup; buffered data may therefore be lost, and the design must not promise
otherwise.

### 2. Read semantics

Stage 36a needs byte-oriented read-up-to, read-exact, read-to-end, read-available,
one-byte, EOF, would-block, timeout, interruption, and partial-read semantics.
Text/line/delimiter/peek behavior belongs to adapters with explicit maximum
allocation. A read result must separate data, would-block, and EOF; the candidate
`ReadOutcome<T>` illustration is not accepted syntax. Read-exact and read-to-end
are compositions that retain checked source failures and allocation limits.

### 3. Write semantics

The foundation distinguishes write-some from write-all. Write-some reports the
accepted byte count or would-block; write-all resumes after writable readiness,
observes cancellation/backpressure, and never pretends partial progress did not
occur. Broken standard-stream pipes keep the existing status-0 carve-out. Flush
of user/runtime buffers is distinct from `fdatasync`/durable synchronization, and
half-close is a duplex/network/process capability rather than ordinary close.

### 4. Seeking and position

Seek from start/current/end, tell, rewind, truncate, length, sparse-file behavior,
and unsupported seek are preserved through a seekable capability contract, so a
forward-only pipe cannot advertise seeking; platform/filesystem
failures still use checked errors. Open-handle operations belong to `Io`; path
namespace metadata belongs to `Fs`.

### 5. Blocking mode

Blocking control is a real, platform-dependent capability, not a cosmetic flag.
It belongs to a capable stream value/interface, never a global
`stream_set_blocking` clone. The current mode is readable and runtime mutation
uses a named typed state only where the capability and platform support it;
unsupported transitions are checked failures. Buffers coordinate mode changes.
The illustrative `setBlockingMode(BlockingMode::NonBlocking)` spelling remains
noncanonical because exact names are deferred.

### 6. Readiness and polling

One portable readiness substrate covers readable/writable readiness, exceptional
closure, immediate poll, finite/indefinite wait, multiple handles, fairness,
spurious readiness, cancellation, and deadlines. `select`, `poll`, `epoll`,
`kqueue`, and IOCP remain backend vocabulary. A multi-stream core is
authoritative; public one-stream conveniences may derive from it. Readiness is
advisory, preserving level/edge-trigger implementation freedom.

### 7. Timeouts and deadlines

Connection, accept, read, write, idle, overall-operation, and readiness-wait
timing are separate facts. A single ambiguous stream timeout is rejected.
Durations and absolute deadlines are both accepted, apply per operation or wait,
and compose with one cancellation model. Exact `Doria\Std\Time` spellings remain
deferred to their owner.

### 8. Buffering

Typed buffered reader/writer adapters own capacity, read-ahead, unread bytes,
line buffering, and flush. Mixing adapter reads with raw reads or seeking requires
an explicit state rule. Buffering is per value, not process-global. Blocking-mode
changes and seek must account for buffered data; panic may discard unflushed
buffers, while structured exits follow the accepted close/flush error contract.

### 9. Text adapters

Doria `string` remains valid UTF-8. A UTF-8 text reader/writer, bounded line and
delimiter readers, CRLF/LF policy, invalid-encoding checked errors, and explicit
other-encoding conversion sit above byte streams. Generic network/process bytes
never pass through `string` implicitly. Line and delimiter reads are bounded;
the exact limit/configuration spelling remains deferred.

### 10. Stream copying

The PHP copy capability becomes bounded streaming copy between readable and
writable contracts, not a whole-input allocation. It handles partial writes,
backpressure, cancellation, progress, maximum bytes, buffer reuse, EOF, and
independent source/destination failures. Copy-to-end and bounded-copy are required
v1 compositions. Progress reporting is deferred until the first operation or
adapter that needs it and must then be typed and operation-local.

### 11. Files

The v1 file surface needs typed open modes (read/write/append/create/create-new/
truncate), byte reads/writes, seek/tell, flush, durable data/full sync, close,
length/open-handle metadata, advisory locks, and temporary files. Permissions and
platform differences are explicit. Namespace/path operations stay in `Fs`; the
exact `Path` representation is safely deferred to the filesystem design, and
Stage 36a must preserve typed-path evolution.

### 12. Standard streams

Stdin/stdout/stderr become first-class non-owning, nonclosable views over the same
runtime devices used by existing intrinsics. They may be stored, passed, wrapped,
redirected, and injected for tests through that same model. Mode mutation needs
exclusive control and a restoring guard. Exact accessor names remain deferred.
Per-stream write order remains
exact; independent stdout/stderr handles do not create a reconstructable global
order unless explicitly merged.

### 13. Process pipes

`Doria\Std\Process` owns a child and typed stdin/stdout/stderr pipe values. The
model needs half-close, wait, termination, exit status, output limits, timeouts,
non-blocking mode, and readiness. Stdout and stderr must be drained concurrently
and capture must be bounded to avoid deadlock or unbounded memory. An active child
must be resolved explicitly by wait, detach, or terminate; destruction does not
silently choose. Direct and shell invocation remain separate typed operations.

### 14. Networking boundary

PHP socket-stream functions map to `Doria\Std\Net`: TCP clients/listeners,
accept, UDP, Unix-domain sockets, socket pairs, local/peer addresses, half-close,
connect/accept timeouts, and later TLS upgrade. Stage 36a supplies duplex byte
streams, readiness, partial writes, timing, and ownership; Stage 44 owns product
network/HTTP APIs and TLS implementation.

### 15. Terminal boundary

TTY detection, standard-input readiness, blocking mode, and platform device
abstraction are Stage 36a substrate. Portable key events additionally require raw
mode, canonical-mode changes, escape/console-event decoding, resize events, and
restoration, all exclusively Stage 46 `Console` work. Non-blocking stdin alone is
not a portable `pollKey` implementation.

### 16. Filters, wrappers, contexts, and adapters

Compression, TLS, encoding, hashing, limits, buffering, rate limiting, progress,
proxy/HTTP/certificate options are preserved through typed adapters or typed
domain configuration. Global wrapper registries, string-selected filters,
`mixed` bags, global default contexts, and scheme-dispatch as the core API are
rejected. Each family and protocol has a manifest row naming its owner/deferral.

### 17. Metadata and capability discovery

PHP's metadata array becomes typed properties, capability interfaces, operation
results, or domain metadata values: EOF, blocking, timeout, unread bytes,
seekability, readability, writability, locality, TTY status, peer/local addresses,
and stream kind. Doria does not expose `Dictionary<string, mixed>` metadata.

### 18. Locking

Files preserve shared/exclusive advisory lock, non-blocking try-lock, unlock, and
unsupported-lock behavior. A typed RAII lock guard is recommended over integer
flag combinations so release is deterministic. Non-blocking acquisition uses a
typed outcome, and unsupported locking is a capability/checked-failure fact rather
than a flag or sentinel.

### 19. Async integration

Stages 37–39 consume the synchronous ownership/results/readiness model for async
read/write/accept/connect, cancellation, deadlines, backpressure, bounded buffers,
closure, and failure propagation. Async lowering does not create a parallel
stream hierarchy or a second event loop/cancellation model. Structured task-group
cancellation releases operations deterministically under the accepted ownership
contract.

## Required v1.0 recommendation matrix

| Capability | Recommendation | Candidate API status | Landing |
|---|---|---|---|
| Readable, writable, duplex, seekable contracts | Required For v1.0 | Names deferred | Stage 36a |
| First-class standard streams | Required For v1.0 | Access/ownership names deferred | Stage 36a |
| File handles and typed open modes | Required For v1.0 | Exact types/modes deferred | Stage 36a |
| Read some/exact/to-end and explicit EOF | Required For v1.0 | Result spelling deferred | Stage 36a |
| Write some/all and partial progress | Required For v1.0 | Result spelling deferred | Stage 36a |
| Would-block and blocking-mode control | Required For v1.0 | Typed mode spelling deferred | Stage 36a |
| Readiness: one/many, immediate/finite/indefinite | Required For v1.0 | Waiter/poller shape deferred | Stage 36a |
| Timeouts and deadlines | Required For v1.0 | Depends on `Doria\Std\Time` design | Stage 36a |
| Buffered byte adapters | Required For v1.0 | Type names deferred | Stage 36a |
| UTF-8 text writer/reader and bounded line reader | Required For v1.0 | Type names deferred | Stage 36a |
| Streaming copy | Required For v1.0 | Operation name deferred | Stage 36a |
| Temporary files and advisory locking | Recommended For v1.0 | RAII lock spelling deferred | Stage 36a |
| Process pipes and concurrent child-output drainage | Required For v1.0 | Child type/member names deferred | Stage 36a |
| Socket readiness substrate | Required For v1.0 | Generic substrate only | Stage 36a |
| Network/TLS adapter boundary | Recommended For v1.0 | Product spelling deferred | Stage 44 |
| Compression and encoding adapters | Recommended For v1.0 | Typed adapter names deferred | Review after Stage 36a |
| Terminal readiness integration | Required For v1.0 | Generic substrate only | Stages 36a/46 |
| Async stream integration | Required For v1.0 | Reuses sync contracts | Stages 37–39 |

Functionality, candidate spelling, and implementation stage are deliberately
separate columns. No candidate name becomes authority merely by appearing here.

## PHP migration ledger

Every current official PHP Streams entry and each relevant external stream/
process/Readline entry appears exactly once below. Detailed EOF, would-block,
partial-progress, platform, dependency, alias, and semantic-difference facts live
in the JSON row with the same PHP name.

| PHP entry | Area/kind | Capability category | Doria classification / owner | v1 / stage | Migration |
|---|---|---|---|---|---|
| `stream_bucket_append` | streams / function | filter-bucket | Rejected Resource-Oriented Shape / Doria\Std\Io | Rejected / — | Rewrite Through Adapter |
| `stream_bucket_make_writeable` | streams / function | filter-bucket | Rejected Resource-Oriented Shape / Doria\Std\Io | Rejected / — | Rewrite Through Adapter |
| `stream_bucket_new` | streams / function | filter-bucket | Rejected Resource-Oriented Shape / Doria\Std\Io | Rejected / — | Rewrite Through Adapter |
| `stream_bucket_prepend` | streams / function | filter-bucket | Rejected Resource-Oriented Shape / Doria\Std\Io | Rejected / — | Rewrite Through Adapter |
| `stream_context_create` | streams / function | typed-configuration | Rejected Global Configuration / Domain Configuration Types | Rejected PHP Shape / 36a | Rewrite Through Domain Module |
| `stream_context_get_default` | streams / function | global-context | Rejected Global Configuration / — | Rejected / — | No Doria Equivalent By Design |
| `stream_context_get_options` | streams / function | typed-configuration | Rejected Global Configuration / Domain Configuration Types | Rejected PHP Shape / 36a | Rewrite Through Domain Module |
| `stream_context_get_params` | streams / function | typed-configuration | Rejected Global Configuration / Domain Configuration Types | Rejected PHP Shape / 36a | Rewrite Through Domain Module |
| `stream_context_set_default` | streams / function | global-context | Rejected Global Configuration / — | Rejected / — | No Doria Equivalent By Design |
| `stream_context_set_option` | streams / function | typed-configuration | Rejected Global Configuration / Domain Configuration Types | Rejected PHP Shape / 36a | Rewrite Through Domain Module |
| `stream_context_set_options` | streams / function | typed-configuration | Rejected Global Configuration / Domain Configuration Types | Rejected PHP Shape / 36a | Rewrite Through Domain Module |
| `stream_context_set_params` | streams / function | typed-configuration | Rejected Global Configuration / Domain Configuration Types | Rejected PHP Shape / 36a | Rewrite Through Domain Module |
| `stream_copy_to_stream` | streams / function | stream-copy | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `stream_filter_append` | streams / function | filter-composition | Proposed Doria Std Io / Typed Stream Adapters | Recommended For v1.0 / 36a | Rewrite Through Adapter |
| `stream_filter_prepend` | streams / function | filter-composition | Proposed Doria Std Io / Typed Stream Adapters | Recommended For v1.0 / 36a | Rewrite Through Adapter |
| `stream_filter_register` | streams / function | filter-registry | Rejected Dynamic Wrapper Mechanism / — | Rejected / — | No Doria Equivalent By Design |
| `stream_filter_remove` | streams / function | filter-composition | Proposed Doria Std Io / Typed Stream Adapters | Recommended For v1.0 / 36a | Rewrite Through Adapter |
| `stream_get_contents` | streams / function | read-to-end | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Rewrite With Semantic Warning |
| `stream_get_filters` | streams / function | filter-registry | Rejected Dynamic Wrapper Mechanism / — | Rejected / — | No Doria Equivalent By Design |
| `stream_get_line` | streams / function | delimiter-read | Proposed Text Adapter / Doria\Std\Io | Required For v1.0 / 36a | Rewrite Through Adapter |
| `stream_get_meta_data` | streams / function | capability-metadata | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Rewrite With Semantic Warning |
| `stream_get_transports` | streams / function | network-stream | Proposed Doria Std Net / Doria\Std\Net | Recommended For v1.0 / 44 | Rewrite Through Domain Module |
| `stream_get_wrappers` | streams / function | wrapper-registry-introspection | Rejected Dynamic Wrapper Mechanism / — | Rejected / — | No Doria Equivalent By Design |
| `stream_is_local` | streams / function | capability-metadata | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Rewrite With Semantic Warning |
| `stream_isatty` | streams / function | tty-detection | Existing Doria Runtime Substrate / Doria\Std\Term | Required For v1.0 / 46 | Rewrite Through Domain Module |
| `stream_notification_callback` | streams / function | progress-notification | Deferred Post-v1 / Operation-Specific Progress API | Deferred / operation-specific review | Deferred Until Named Stage |
| `stream_register_wrapper` | streams / function | wrapper-registry-alias | Rejected PHP Alias / — | Rejected / — | No Doria Equivalent By Design |
| `stream_resolve_include_path` | streams / function | path-resolution | Proposed Doria Std Fs / Doria\Std\Fs | Acceptable Post-v1 / Fs design | Rewrite Through Domain Module |
| `stream_select` | streams / function | readiness | Proposed Async Integration / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `stream_set_blocking` | streams / function | blocking-control | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Rewrite With Semantic Warning |
| `stream_set_chunk_size` | streams / function | buffering | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Rewrite Through Adapter |
| `stream_set_read_buffer` | streams / function | buffering | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Rewrite Through Adapter |
| `stream_set_timeout` | streams / function | timeouts | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Requires Human Review |
| `stream_set_write_buffer` | streams / function | buffering | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Rewrite Through Adapter |
| `stream_socket_accept` | streams / function | network-stream | Proposed Doria Std Net / Doria\Std\Net | Recommended For v1.0 / 44 | Rewrite Through Domain Module |
| `stream_socket_client` | streams / function | network-stream | Proposed Doria Std Net / Doria\Std\Net | Recommended For v1.0 / 44 | Rewrite Through Domain Module |
| `stream_socket_enable_crypto` | streams / function | network-stream | Proposed Doria Std Net / Doria\Std\Net | Recommended For v1.0 / 44 | Rewrite Through Domain Module |
| `stream_socket_get_name` | streams / function | network-stream | Proposed Doria Std Net / Doria\Std\Net | Recommended For v1.0 / 44 | Rewrite Through Domain Module |
| `stream_socket_pair` | streams / function | network-stream | Proposed Doria Std Net / Doria\Std\Net | Recommended For v1.0 / 44 | Rewrite Through Domain Module |
| `stream_socket_recvfrom` | streams / function | network-stream | Proposed Doria Std Net / Doria\Std\Net | Recommended For v1.0 / 44 | Rewrite Through Domain Module |
| `stream_socket_sendto` | streams / function | network-stream | Proposed Doria Std Net / Doria\Std\Net | Recommended For v1.0 / 44 | Rewrite Through Domain Module |
| `stream_socket_server` | streams / function | network-stream | Proposed Doria Std Net / Doria\Std\Net | Recommended For v1.0 / 44 | Rewrite Through Domain Module |
| `stream_socket_shutdown` | streams / function | network-stream | Proposed Doria Std Net / Doria\Std\Net | Recommended For v1.0 / 44 | Rewrite Through Domain Module |
| `stream_supports_lock` | streams / function | capability-metadata | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Rewrite With Semantic Warning |
| `stream_wrapper_register` | streams / function | wrapper-registry | Rejected Dynamic Wrapper Mechanism / — | Rejected / — | No Doria Equivalent By Design |
| `stream_wrapper_restore` | streams / function | wrapper-registry | Rejected Dynamic Wrapper Mechanism / — | Rejected / — | No Doria Equivalent By Design |
| `stream_wrapper_unregister` | streams / function | wrapper-registry | Rejected Dynamic Wrapper Mechanism / — | Rejected / — | No Doria Equivalent By Design |
| `php_user_filter` | streams / class | dynamic-stream-prototype | Rejected Dynamic Wrapper Mechanism / Typed Stream Adapters | Rejected / — | Rewrite Through Adapter |
| `streamWrapper` | streams / class | dynamic-stream-prototype | Rejected Dynamic Wrapper Mechanism / Typed Stream Adapters | Rejected / — | Rewrite Through Adapter |
| `StreamBucket` | filter / class | dynamic-stream-prototype | Rejected Resource-Oriented Shape / Typed Stream Adapters | Rejected / — | Rewrite Through Adapter |
| `php_user_filter::filter` | filter / method | filter-lifecycle | Proposed Doria Std Io / Typed Stream Adapters | Recommended For v1.0 / 36a | Rewrite Through Adapter |
| `php_user_filter::onClose` | filter / method | filter-lifecycle | Proposed Doria Std Io / Typed Stream Adapters | Recommended For v1.0 / 36a | Rewrite Through Adapter |
| `php_user_filter::onCreate` | filter / method | filter-lifecycle | Proposed Doria Std Io / Typed Stream Adapters | Recommended For v1.0 / 36a | Rewrite Through Adapter |
| `streamWrapper::__construct` | wrapper / method | wrapper-lifecycle | Rejected Dynamic Wrapper Mechanism / Typed Constructors | Rejected / — | No Doria Equivalent By Design |
| `streamWrapper::__destruct` | wrapper / method | wrapper-lifecycle | Rejected Dynamic Wrapper Mechanism / Typed Constructors | Rejected / — | No Doria Equivalent By Design |
| `streamWrapper::dir_closedir` | wrapper / method | filesystem-namespace | Proposed Doria Std Fs / Doria\Std\Fs | Recommended For v1.0 / Fs design | Rewrite Through Domain Module |
| `streamWrapper::dir_opendir` | wrapper / method | filesystem-namespace | Proposed Doria Std Fs / Doria\Std\Fs | Recommended For v1.0 / Fs design | Rewrite Through Domain Module |
| `streamWrapper::dir_readdir` | wrapper / method | filesystem-namespace | Proposed Doria Std Fs / Doria\Std\Fs | Recommended For v1.0 / Fs design | Rewrite Through Domain Module |
| `streamWrapper::dir_rewinddir` | wrapper / method | filesystem-namespace | Proposed Doria Std Fs / Doria\Std\Fs | Recommended For v1.0 / Fs design | Rewrite Through Domain Module |
| `streamWrapper::mkdir` | wrapper / method | filesystem-namespace | Proposed Doria Std Fs / Doria\Std\Fs | Recommended For v1.0 / Fs design | Rewrite Through Domain Module |
| `streamWrapper::rename` | wrapper / method | filesystem-namespace | Proposed Doria Std Fs / Doria\Std\Fs | Recommended For v1.0 / Fs design | Rewrite Through Domain Module |
| `streamWrapper::rmdir` | wrapper / method | filesystem-namespace | Proposed Doria Std Fs / Doria\Std\Fs | Recommended For v1.0 / Fs design | Rewrite Through Domain Module |
| `streamWrapper::stream_cast` | wrapper / method | wrapper-lifecycle | Rejected Dynamic Wrapper Mechanism / Typed Constructors | Rejected / — | No Doria Equivalent By Design |
| `streamWrapper::stream_close` | wrapper / method | stream-capability | Proposed Doria Std Io / Doria\Std\Io | Recommended For v1.0 / 36a | Rewrite Through Domain Module |
| `streamWrapper::stream_eof` | wrapper / method | stream-capability | Proposed Doria Std Io / Doria\Std\Io | Recommended For v1.0 / 36a | Rewrite Through Domain Module |
| `streamWrapper::stream_flush` | wrapper / method | stream-capability | Proposed Doria Std Io / Doria\Std\Io | Recommended For v1.0 / 36a | Rewrite Through Domain Module |
| `streamWrapper::stream_lock` | wrapper / method | stream-capability | Proposed Doria Std Io / Doria\Std\Io | Recommended For v1.0 / 36a | Rewrite Through Domain Module |
| `streamWrapper::stream_metadata` | wrapper / method | filesystem-namespace | Proposed Doria Std Fs / Doria\Std\Fs | Recommended For v1.0 / Fs design | Rewrite Through Domain Module |
| `streamWrapper::stream_open` | wrapper / method | stream-capability | Proposed Doria Std Io / Doria\Std\Io | Recommended For v1.0 / 36a | Rewrite Through Domain Module |
| `streamWrapper::stream_read` | wrapper / method | stream-capability | Proposed Doria Std Io / Doria\Std\Io | Recommended For v1.0 / 36a | Rewrite Through Domain Module |
| `streamWrapper::stream_seek` | wrapper / method | stream-capability | Proposed Doria Std Io / Doria\Std\Io | Recommended For v1.0 / 36a | Rewrite Through Domain Module |
| `streamWrapper::stream_set_option` | wrapper / method | stream-capability | Proposed Doria Std Io / Doria\Std\Io | Recommended For v1.0 / 36a | Rewrite Through Domain Module |
| `streamWrapper::stream_stat` | wrapper / method | stream-capability | Proposed Doria Std Io / Doria\Std\Io | Recommended For v1.0 / 36a | Rewrite Through Domain Module |
| `streamWrapper::stream_tell` | wrapper / method | stream-capability | Proposed Doria Std Io / Doria\Std\Io | Recommended For v1.0 / 36a | Rewrite Through Domain Module |
| `streamWrapper::stream_truncate` | wrapper / method | stream-capability | Proposed Doria Std Io / Doria\Std\Io | Recommended For v1.0 / 36a | Rewrite Through Domain Module |
| `streamWrapper::stream_write` | wrapper / method | stream-capability | Proposed Doria Std Io / Doria\Std\Io | Recommended For v1.0 / 36a | Rewrite Through Domain Module |
| `streamWrapper::unlink` | wrapper / method | filesystem-namespace | Proposed Doria Std Fs / Doria\Std\Fs | Recommended For v1.0 / Fs design | Rewrite Through Domain Module |
| `streamWrapper::url_stat` | wrapper / method | filesystem-namespace | Proposed Doria Std Fs / Doria\Std\Fs | Recommended For v1.0 / Fs design | Rewrite Through Domain Module |
| `Stream abstraction` | streams / concept | stream-concept | Rejected Resource-Oriented Shape / Doria\Std\Io | Rejected / — | Rewrite With Semantic Warning |
| `Stream Context Concepts` | streams / concept | stream-concept | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `Stream Filter Concepts` | streams / concept | stream-concept | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `Stream Error Concepts` | streams / concept | stream-concept | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `file://` | wrapper / wrapper | protocol-wrapper | Proposed Doria Std Fs / Doria\Std\Fs | Required For v1.0 / 36a | Rewrite Through Domain Module |
| `http://` | wrapper / wrapper | protocol-wrapper | Proposed Doria Std Net / Doria\Std\Http | Recommended For v1.0 / 44 | Rewrite Through Domain Module |
| `ftp://` | wrapper / wrapper | protocol-wrapper | Deferred Post-v1 / Doria\Std\Net | Acceptable Post-v1 / post-v1 network review | Deferred Until Named Stage |
| `php://` | wrapper / wrapper | protocol-wrapper | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `zlib://` | wrapper / wrapper | protocol-wrapper | Proposed Compression Adapter / Compression Adapter | Recommended For v1.0 / adapter review after 36a | Rewrite Through Adapter |
| `data://` | wrapper / wrapper | protocol-wrapper | Derivable From Existing Surface / Bytes/String Constructors | Recommended For v1.0 / 36a | Derivable Composition |
| `glob://` | wrapper / wrapper | protocol-wrapper | Proposed Doria Std Fs / Doria\Std\Fs | Acceptable Post-v1 / Fs design | Rewrite Through Domain Module |
| `phar://` | wrapper / wrapper | protocol-wrapper | Deferred Post-v1 / Domain Package | Acceptable Post-v1 / post-v1 package review | Deferred Until Named Stage |
| `ssh2://` | wrapper / wrapper | protocol-wrapper | Deferred Post-v1 / Doria\Std\Net | Acceptable Post-v1 / post-v1 network review | Deferred Until Named Stage |
| `rar://` | wrapper / wrapper | protocol-wrapper | Deferred Post-v1 / Domain Package | Acceptable Post-v1 / post-v1 package review | Deferred Until Named Stage |
| `ogg://` | wrapper / wrapper | protocol-wrapper | Deferred Post-v1 / Domain Package | Acceptable Post-v1 / post-v1 package review | Deferred Until Named Stage |
| `expect://` | wrapper / wrapper | protocol-wrapper | Proposed Doria Std Process / Doria\Std\Process | Acceptable Post-v1 / post-v1 process interaction review | Rewrite Through Domain Module |
| `String Filters` | filter / concept | filter-family | Proposed Text Adapter / Doria\Std\Io | Required For v1.0 / 36a | Rewrite Through Adapter |
| `Conversion Filters` | filter / concept | filter-family | Proposed Encoding Adapter / Encoding Adapter | Recommended For v1.0 / adapter review after 36a | Rewrite Through Adapter |
| `Compression Filters` | filter / concept | filter-family | Proposed Compression Adapter / Compression Adapter | Recommended For v1.0 / adapter review after 36a | Rewrite Through Adapter |
| `Encryption Filters` | filter / concept | filter-family | Proposed Doria Std Net / Doria\Std\Net | Recommended For v1.0 / 44 | Rewrite Through Adapter |
| `Socket context options` | context / concept | context-family | Proposed Doria Std Net / Doria\Std\Net | Recommended For v1.0 / 44 | Rewrite Through Domain Module |
| `HTTP context options` | context / concept | context-family | Proposed Doria Std Net / Doria\Std\Net | Recommended For v1.0 / 44 | Rewrite Through Domain Module |
| `FTP context options` | context / concept | context-family | Proposed Doria Std Net / Doria\Std\Net | Recommended For v1.0 / 44 | Rewrite Through Domain Module |
| `SSL context options` | context / concept | context-family | Proposed Doria Std Net / Doria\Std\Net | Recommended For v1.0 / 44 | Rewrite Through Domain Module |
| `Phar context options` | context / concept | context-family | Rejected Resource-Oriented Shape / Explicit Domain Configuration | Rejected / — | No Doria Equivalent By Design |
| `Context parameters` | context / concept | context-family | Rejected Resource-Oriented Shape / Explicit Domain Configuration | Rejected / — | No Doria Equivalent By Design |
| `Zip context options` | context / concept | context-family | Proposed Compression Adapter / Compression Adapter | Recommended For v1.0 / adapter review after 36a | Rewrite Through Domain Module |
| `Zlib context options` | context / concept | context-family | Proposed Compression Adapter / Compression Adapter | Recommended For v1.0 / adapter review after 36a | Rewrite Through Domain Module |
| `fclose` | filesystem / function | stream-lifetime-position | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `fdatasync` | filesystem / function | stream-write | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `feof` | filesystem / function | stream-read | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `fflush` | filesystem / function | stream-write | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `fgetc` | filesystem / function | stream-read | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `fgetcsv` | filesystem / function | stream-read | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `fgets` | filesystem / function | stream-read | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `fgetss` | filesystem / function | stream-read | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `file` | filesystem / function | stream-read | Derivable From Existing Surface / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `file_get_contents` | filesystem / function | stream-read | Derivable From Existing Surface / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `file_put_contents` | filesystem / function | stream-write | Derivable From Existing Surface / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `flock` | filesystem / function | stream-lifetime-position | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `fopen` | filesystem / function | stream-lifetime-position | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `fpassthru` | filesystem / function | stream-read | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `fputcsv` | filesystem / function | stream-write | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `fputs` | filesystem / function | stream-write | Rejected PHP Alias / — | Rejected / — | No Doria Equivalent By Design |
| `fread` | filesystem / function | stream-read | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `fscanf` | filesystem / function | stream-read | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `fseek` | filesystem / function | stream-lifetime-position | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `fstat` | filesystem / function | stream-lifetime-position | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `fsync` | filesystem / function | stream-write | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `ftell` | filesystem / function | stream-lifetime-position | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `ftruncate` | filesystem / function | stream-write | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `fwrite` | filesystem / function | stream-write | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `pclose` | filesystem / function | process-pipe | Proposed Doria Std Process / Doria\Std\Process | Required For v1.0 / 36a | Rewrite Through Domain Module |
| `popen` | filesystem / function | process-pipe | Proposed Doria Std Process / Doria\Std\Process | Required For v1.0 / 36a | Rewrite Through Domain Module |
| `readfile` | filesystem / function | stream-read | Derivable From Existing Surface / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `rewind` | filesystem / function | stream-lifetime-position | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `set_file_buffer` | filesystem / function | stream-write | Rejected PHP Alias / — | Rejected / — | No Doria Equivalent By Design |
| `tmpfile` | filesystem / function | stream-lifetime-position | Proposed Doria Std Io / Doria\Std\Io | Required For v1.0 / 36a | Direct Typed Rewrite |
| `proc_close` | process / function | child-process-pipes | Proposed Doria Std Process / Doria\Std\Process | Required For v1.0 / 36a | Rewrite Through Domain Module |
| `proc_get_status` | process / function | child-process-pipes | Proposed Doria Std Process / Doria\Std\Process | Required For v1.0 / 36a | Rewrite Through Domain Module |
| `proc_open` | process / function | child-process-pipes | Proposed Doria Std Process / Doria\Std\Process | Required For v1.0 / 36a | Rewrite Through Domain Module |
| `proc_terminate` | process / function | child-process-pipes | Proposed Doria Std Process / Doria\Std\Process | Required For v1.0 / 36a | Rewrite Through Domain Module |
| `readline` | readline / function | line-input | Existing Doria Intrinsic / read_line | Required For v1.0 / implemented | Rewrite With Semantic Warning |
| `readline_add_history` | readline / function | interactive-line-editor | Proposed Doria Std Term / Doria\Std\Term | Recommended For v1.0 / 46 | Rewrite Through Domain Module |
| `readline_callback_handler_install` | readline / function | interactive-line-editor | Proposed Doria Std Term / Doria\Std\Term | Recommended For v1.0 / 46 | Rewrite Through Domain Module |
| `readline_callback_handler_remove` | readline / function | interactive-line-editor | Proposed Doria Std Term / Doria\Std\Term | Recommended For v1.0 / 46 | Rewrite Through Domain Module |
| `readline_callback_read_char` | readline / function | interactive-line-editor | Proposed Doria Std Term / Doria\Std\Term | Recommended For v1.0 / 46 | Rewrite Through Domain Module |
| `readline_clear_history` | readline / function | interactive-line-editor | Proposed Doria Std Term / Doria\Std\Term | Recommended For v1.0 / 46 | Rewrite Through Domain Module |
| `readline_completion_function` | readline / function | interactive-line-editor | Proposed Doria Std Term / Doria\Std\Term | Recommended For v1.0 / 46 | Rewrite Through Domain Module |
| `readline_info` | readline / function | interactive-line-editor | Proposed Doria Std Term / Doria\Std\Term | Recommended For v1.0 / 46 | Rewrite Through Domain Module |
| `readline_list_history` | readline / function | interactive-line-editor | Proposed Doria Std Term / Doria\Std\Term | Recommended For v1.0 / 46 | Rewrite Through Domain Module |
| `readline_on_new_line` | readline / function | interactive-line-editor | Proposed Doria Std Term / Doria\Std\Term | Recommended For v1.0 / 46 | Rewrite Through Domain Module |
| `readline_read_history` | readline / function | interactive-line-editor | Proposed Doria Std Term / Doria\Std\Term | Recommended For v1.0 / 46 | Rewrite Through Domain Module |
| `readline_redisplay` | readline / function | interactive-line-editor | Proposed Doria Std Term / Doria\Std\Term | Recommended For v1.0 / 46 | Rewrite Through Domain Module |
| `readline_write_history` | readline / function | interactive-line-editor | Proposed Doria Std Term / Doria\Std\Term | Recommended For v1.0 / 46 | Rewrite Through Domain Module |

## Designer review table

Andrew approved these recommendations on 2026-08-02. Decision 0110 owns their
semantics and performance constraints. “Public spelling deferred” means only
the exact source vocabulary is open; it does not reopen the architecture,
allocation/copy model, reusable-buffer requirement, readiness reuse, or async
cost isolation. The performance column is Accepted for every reviewed decision;
the capability-specific detail is normalized in the machine-readable manifest.

| Decision                       | Recommendation                                                                                      | Alternatives                               | Why it matters                                    | PHP capability preserved     | Doria owner    | v1 status         | Landing   | Dependencies | Runtime impact        | Migration impact       | Performance constraints | Semantic status           | Public spelling status    | Authority     |
| ------------------------------ | --------------------------------------------------------------------------------------------------- | ------------------------------------------ | ------------------------------------------------- | ---------------------------- | -------------- | ----------------- | --------- | ------------ | --------------------- | ---------------------- | ----------------------- | ------------------------- | ------------------------- | ------------- |
| Stream interface decomposition | Small readable/writable/duplex/seekable/etc. capabilities                                           | One stream class; dynamic checks           | Prevents accidental authority                     | Resource polymorphism        | `Io`           | Required          | 36a       | 29, 35       | Vtables/adapters      | Typed resource rewrite | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| First-class standard streams   | Yes, on existing substrate                                                                          | Intrinsic-only forever                     | Enables generic CLI composition                   | `php://std*`                 | `Io`           | Required          | 36a       | 29, 35       | Borrowed device views | Direct typed rewrite   | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| Standard-stream ownership      | Non-owning views; no accidental close                                                               | Owned closable handles                     | Protects process-global devices                   | STDIN/OUT/ERR resources      | `Io`           | Required          | 36a       | ownership    | Device lifetime       | Human review           | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| Close semantics                | Consuming explicit close/finish reports checked failure; destruction is best-effort and nonthrowing | Idempotent nonconsuming close              | Prevents leaks, double close, and hidden failures | `fclose`                     | `Io`           | Required          | 36a       | 29           | Drop/close errors     | Semantic warning       | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| Blocking-mode naming           | Typed mode on capable value                                                                         | Boolean; separate types; construction-only | Mode is capability, not flag                      | `stream_set_blocking`        | `Io`           | Required          | 36a       | 35           | Platform mode calls   | Semantic warning       | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| Runtime mode mutation          | Permit only where supported                                                                         | Immutable mode types                       | Pipes/sockets/terminals need transitions          | Blocking mutation            | `Io`           | Required          | 36a       | readiness    | Capability checks     | Human review           | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| Capability discovery           | Interfaces plus typed facts                                                                         | Metadata bag; try-and-fail only            | Avoids `mixed` metadata                           | `stream_get_meta_data`       | `Io`           | Required          | 36a       | 35           | Typed metadata        | Domain rewrite         | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| Non-blocking read outcome      | Data / would-block / EOF distinction                                                                | Result nesting; separate calls             | Prevents sentinel ambiguity                       | `fread`/`feof`               | `Io`           | Required          | 36a       | 29           | Result tags           | Semantic warning       | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| Partial-write outcome          | Count / would-block / closed/error                                                                  | Boolean success                            | Prevents data loss                                | `fwrite` count               | `Io`           | Required          | 36a       | readiness    | Resume state          | Semantic warning       | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| Readiness API shape            | Portable typed readiness results                                                                    | Callback reactor; platform handles         | Foundation for sync and async                     | `stream_select`              | `Io`           | Required          | 36a       | Time         | OS pollers            | Direct typed rewrite   | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| One-stream versus waiter API   | Derive convenience from multi-stream core                                                           | Public one-stream only; both primitives    | Avoids duplicate readiness models                 | Single/multi select          | `Io`           | Required          | 36a       | readiness    | Wait registration     | Human review           | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| Timeout versus deadline        | Support durations and absolute deadlines per operation/wait                                         | Duration only; deadline only               | Different timeout meanings                        | Stream/socket timeouts       | `Io`/`Time`    | Required          | 36a       | Time         | Timer integration     | Semantic warning       | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| Buffering types                | Per-value typed adapters                                                                            | Flags on streams; globals                  | Makes read-ahead/flush ownership visible          | Buffer controls              | `Io`           | Required          | 36a       | 29, 35       | Adapter buffers       | Adapter rewrite        | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| Text adapter names             | Explicit UTF-8 reader/writer                                                                        | Text on every stream                       | Preserves string invariant                        | Text filters/line reads      | `Io`           | Required          | 36a       | Bytes        | Adapter rewrite       | Accepted               | Accepted                | Deferred where applicable | decision 0110             |
| Line-read limits               | Required explicit/default safe maximum                                                              | Unbounded allocation                       | Prevents hostile-input growth                     | `fgets`/delimiter read       | Text adapter   | Required          | 36a       | errors       | Bounded buffers       | Semantic warning       | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| File open modes                | Typed non-boolean modes                                                                             | Mode strings; builder                      | Makes create/truncate/append explicit             | `fopen` modes                | `Io`           | Required          | 36a       | 29           | OS open flags         | Semantic warning       | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| Path type                      | Defer exact representation while preserving typed evolution                                         | Keep `string`; dual acceptance             | Cross-platform path correctness                   | File wrapper targets         | `Fs`           | Deferred          | Fs review | 31           | Path conversion       | Human review           | Accepted                | Accepted                  | Deferred                  | decision 0110 |
| Flush versus durable sync      | Separate buffer flush, data sync, full sync                                                         | One `flush`                                | Prevents false durability claims                  | `fflush`/`fdatasync`/`fsync` | `Io`           | Required          | 36a       | errors       | OS sync calls         | Semantic warning       | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| Advisory locking               | Typed RAII guard                                                                                    | Integer flags; manual unlock               | Deterministic release                             | `flock`                      | `Io`           | Recommended       | 36a       | 29, 35       | Lock backend          | Semantic warning       | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| Process pipe model             | Owned child plus typed three pipes and explicit wait/detach/terminate resolution                    | Shell-only helpers                         | Enables safe composition                          | `proc_open`/`popen`          | `Process`      | Required          | 36a       | 29, 35       | Spawn/pipe backend    | Domain rewrite         | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| Child output drainage          | Readiness-driven concurrent drain                                                                   | Sequential reads; threads only             | Prevents deadlock                                 | Child stdout/stderr pipes    | `Process`/`Io` | Required          | 36a       | readiness    | Multi-handle wait     | Semantic warning       | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| Typed metadata                 | Properties/interfaces/results                                                                       | Dictionary of `mixed`                      | Static safety and portability                     | Metadata arrays              | Domain owners  | Required          | 36a/44/46 | 35           | Typed facts           | Domain rewrite         | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| Wrapper/filter replacement     | Typed constructors/adapters                                                                         | Dynamic registry                           | Keeps capability without global strings           | Wrappers/filters             | Domain owners  | Required          | 36a+      | 31, 35       | Composition           | Adapter/domain rewrite | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| Compression adapters           | Typed reader/writer adapters                                                                        | URL/filter strings                         | Streaming compression                             | zlib wrapper/filter          | Adapter        | Recommended       | after 36a | 35           | Codec state           | Adapter rewrite        | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| Encoding adapters              | Explicit byte/text converters                                                                       | Implicit locale conversion                 | Preserves UTF-8 invariant                         | conversion filters           | Adapter        | Recommended       | after 36a | Bytes        | Codec state           | Adapter rewrite        | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| TLS adapter boundary           | Network-owned typed upgrade/config                                                                  | SSL context bag                            | Secure duplex streams                             | crypto/socket contexts       | `Net`          | Recommended       | 44        | 36a, 37/38   | TLS state             | Domain rewrite         | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| Sync/async stream unification  | One ownership/result/readiness model                                                                | Async-specific streams                     | Avoids two incompatible stacks                    | Non-blocking streams         | `Io`           | Required          | 36a–39  | 37 design    | Await integration     | Direct typed rewrite   | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| Terminal integration           | Reuse readiness/devices; keep decoding in `Console`                                                 | Terminal-specific polling core             | Portable key/resize input                         | Readline/TTY                 | `Term`         | Required          | 46        | 36a          | Raw/event backends    | Domain rewrite         | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |
| Network integration            | Reuse duplex/readiness/timeouts/partial writes                                                      | Socket-specific contracts                  | Avoids parallel I/O semantics                     | Socket streams               | `Net`          | Required boundary | 44        | 36a–38     | Socket backend        | Domain rewrite         | Accepted                | Accepted                  | Deferred where applicable | decision 0110 |

## Scheduling and authority consequence

- Stage 25a is complete.
- This PHP Stream And I/O Completeness Audit is implemented.
- Andrew's Stream API Completeness Review is complete.
- Decision 0110's semantic and performance contract is accepted.
- Stage 26 is unblocked and next.
- Stage 36a is scheduled, not implemented; its semantics, allocation/copy
  constraints, reusable-buffer/byte-region model, readiness reuse, and async
  isolation are accepted, while exact public spellings remain deferred to the
  decision-0110 appendix. Stage 36a owns their initial benchmark and
  memory-regression gate; Stage 43 later continues and broadens it.
- No `BlockingMode`, read-outcome type, poller/waiter, stream interface, file
  object, process API, or first-class standard-stream spelling is executable yet.

## Invalidated elsewhere

- The end-to-end plan now places Stage 36a between Stages 36 and 37 and amends
  Stages 29, 35, 37–39, 43, 44, 46, the phase range, and the 1.0 gate.
- The standard-library inventory now distinguishes current intrinsics from the
  scheduled reviewed `Io`/`Fs`/`Process`/`Net`/`Term` capability boundaries.
- Decision 0110 supersedes this audit wherever recommendation language could be
  mistaken for an unresolved semantic or performance fork; candidate names
  remain noncanonical.
- The current-pipeline note now records the audit/review gate and refuses false
  implementation claims.
- The language-server and website audits found no stale final-stream claim, so
  their behavior, compiler pins, and repositories remain unchanged.

---
# Historical partial I/O audit (preserved)\n
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

> Superseded in part: the line-input signature recorded below as
> `read_line(): ?string` was amended by the Interactive Line-Input Amendment to
> `read_line(string $prompt = ""): ?string`. Decision 0074 is authoritative; the
> status line is preserved as the historical snapshot it was.

**Status at the time of this audit:** `echo` (statement), `read_line(): ?string`, `sprintf`, `printf`, `read_file(string): string`, `write_file(string, string): void`, and `write_stderr(string): void` were shipped; the byte tier was still planned. Stage 23 Slice 2 subsequently shipped `Bytes`, binary file/standard-stream I/O, and text `append_file` under the decisions recorded below.

## Already settled (item → citation)

- **Three-tier file family** (text now / `Bytes` Stage 23 / `File`+stream post-Stage-29) — 0075 §Decision; plan §9.
- **`write_file` truncates** ("creates or truncates and writes exact bytes") — 0074 §Text files; SPEC "Stage 17" (`write_file creates or truncates`). *At audit time truncate was defined and shipped while append was open (Q3); Decision 0091 has since settled the additive `append_file` spelling for Stage 23.*
- **Text tier does no newline normalization; byte-exact read & write** — 0074 ("preserves its bytes without newline normalization"; "writes exact bytes"). `read_line` strips exactly one LF or one CRLF at the line boundary — 0074 §Line input; SPEC.
- **Invalid UTF-8 on text read → `Doria\Std\Io\InvalidUtf8Error`, no lossy/replacement path** — 0074 and implemented 0119. Raw/undecoded bytes are the `Bytes` tier's job, not a lossy string path.
- **Text-tier failure model:** canonical checked errors are implemented and Decision 0123 classifies exactly `Doria\Std\Io\IoError` and `Doria\Std\Io\InvalidUtf8Error` as ambient at the source boundary; they remain exact, catchable, and R1000/status-70 outcomes. `read_line` `null` = EOF only, never error — 0074, 0075, 0119, 0123. P1401-P1407 remain historical and have no ordinary valid route.
- **Terminal layer deferred and bounded:** capability-based `Console` static facade, no escape sequences/handles/ANSI in any public value, Stage 46 build-out, decision number assigned when authored — 0074 §Future terminal boundary; plan §9; 0006.
- **RAII flush/close on normal exit and on `throws` propagation** (drop elaboration runs `__destruct` at every scope boundary) — plan §3.1, §5. **Abort-only panic runs no cleanup** — 0081 (this is the root of D6).
- **SIGPIPE is ignored at the runtime** so a closed-pipe write reports EPIPE instead of killing by signal — `doria-rt/src/lib.rs:940` (impl). *The language-level contract was the audit's D1 gap and is now settled by Decision 0091.*
- **Binary-tier parameters beyond the path are a Stage 23 decision** — 0075; SPEC.
- **Windows:** redirected stdout/stderr write exact length-delimited UTF-8; interactive console validates UTF-8 → UTF-16 → `WriteConsoleW`; all three OSes land together — 0074 §Runtime I/O layering.

## Open questions (the six named + everything derived)

Format per item: **Status · Options · Tradeoffs · Recommendation (marked) · Blast radius.**

### Q1 — stdout byte-write surface / the stderr asymmetry [ACCEPTED — Decision 0101; implementation Stage 23]
- **Resolution.** Recommendation (b) ratified in Decision 0101: no `write_stdout(string)` (the text asymmetry is intentional — `echo` is the sole stdout text writer, `write_stderr` the stderr text escape hatch); `write_stdout_bytes(Bytes): void` and `write_stderr_bytes(Bytes): void` join the Stage 23 byte tier.
- **Status.** `echo` writes text to stdout (display-converted, no newline — verified in lowering); `write_stderr(string)` writes exact text bytes to stderr. There is **no byte-level output to either stdout or stderr**. The asymmetry (`write_stderr` exists, no `write_stdout`) is real but, for *text*, cosmetic: `echo` is the stdout text writer.
- **Options.** (a) Add `write_stdout(string)` now to mirror `write_stderr`. (b) Treat the text asymmetry as intentional (`echo` = the one text-stdout spelling; `write_stderr` = the error-channel escape hatch) and name the **byte** path as a Stage 23 `Bytes`-tier addition: `write_stdout_bytes(Bytes)` + `write_stderr_bytes(Bytes)`. (c) Leave byte output to files only.
- **Tradeoffs.** (a) duplicates `echo` — reintroduces the exact `print`/`echo` redundancy Doria bans. (b) closes the *real* gap (binary piping, non-UTF-8) with symmetric names and honors "one output spelling." (c) leaves binary pipelines impossible.
- **Recommendation → (b).** Do **not** add `write_stdout(string)`. Declare the text asymmetry intentional (`echo` is stdout text; `write_stderr` is the stderr text escape hatch), and reserve `write_stdout_bytes(Bytes): void` and `write_stderr_bytes(Bytes): void` for the Stage 23 `Bytes` tier. This names the byte path explicitly and keeps the naming symmetric where it matters (bytes).
- **Blast radius.** Stage 23 scope; 0075 tier-2 text, plan §9 three-tier bullet, SPEC. No shipped signature changes.

### Q2 — stdin byte tier [ACCEPTED — Decision 0101; implementation Stage 23]
- **Resolution.** Recommendation (a) ratified in Decision 0101: `read_stdin_bytes(): Bytes` (whole-stdin slurp, empty on immediate EOF, no UTF-8 validation) joins the Stage 23 byte tier; chunked/incremental reads stay deferred to the post-Stage-29 stream tier.
- **Status.** `read_line` (text stdin), `read_file` (text file), `read_file_bytes` (binary **file**, Stage 23). No byte-level **stdin** read; the binary tier as written covers files only.
- **Options.** (a) `read_stdin_bytes(): Bytes` free function in the Stage 23 `Bytes` tier (whole-stdin slurp), sibling of `read_file_bytes`. (b) byte stdin only via the post-Stage-29 stream tier.
- **Tradeoffs.** (a) completes the binary tier across file **and** stdin at one stage; whole-stdin slurp only (chunked reads wait for streams). (b) leaves binary piping impossible until post-Stage-29, inconsistent with `read_file_bytes` landing at Stage 23.
- **Recommendation → (a),** with chunked/incremental byte reads deferred to the stream tier. The binary tier should cover stdin the moment it covers files.
- **Blast radius.** Stage 23 scope; 0075 tier-2, plan §9, SPEC.

### Q3 — `write_file` append vs truncate [ACCEPTED — Decision 0091; implementation Stage 23]
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
- **Tradeoffs.** (a) no lost-output surprises, simplest, matches current behavior, and preserves issue order within each stream. (b) throughput for chatty output, but reintroduces flush bugs and the abort-panic data-loss surface (D6) at the free-function tier.
- **Recommendation → (a).** Keep free-function output unbuffered; buffering + `flush()` are stream-tier object methods. **State the ordering guarantee precisely:** writes retain exact program order within stdout and within stderr. Cross-stream order is observable only when both streams target the same underlying handle (including explicit shell merging); separate pipes/descriptors have no single reconstructable order that Doria can guarantee. Do not add a public flush free function (nothing to flush while unbuffered). **Reopen trigger:** stream-tier design.
- **Blast radius.** Mostly documenting an existing guarantee (0074/SPEC); stream-tier flush is post-Stage-29.

### Q6 — standard streams as first-class values [OPEN — post-Stage-29]
- **Status.** Tier 3 is "`File` and stream objects"; the plan says "richer stdin APIs live on the post-Stage-29 `Doria\Std\Io` stream types." Whether `stdin`/`stdout`/`stderr` **themselves** become stream values (passable, storable) is undecided.
- **Options.** (a) **Unify:** the standard streams are obtainable as stream objects (e.g. `Io::stdout(): Stream`), so code can write generically to "a stream"; the intrinsics (`echo`, `read_line`, `write_stderr`) stay as the ergonomic fast path over the same underlying device layer. (b) **Parallel:** the stream tier is for `File`/opened streams only; the standard streams remain intrinsic-only forever.
- **Tradeoffs.** (a) makes I/O composable — a filter can target a file or stdout through one type; must share the doria-rt device layer with the intrinsics to avoid double-buffering/ordering hazards (they already share it per 0074's layering). (b) simpler surface, but every stream-consuming API must special-case the standard streams; a CLI/service language will feel this constantly.
- **Recommendation → (a) unify,** designed so the intrinsics and the stream accessors sit on the same doria-rt substrate. **Reopen trigger:** `Doria\Std\Io` stream-tier design.
- **Blast radius.** Post-Stage-29 stream tier; the stream type and standard-stream accessors must be designed together.

### D1 — closed stdout / EPIPE / SIGPIPE (the `head` case) [ACCEPTED AND IMPLEMENTED — Decision 0091]
- **Status at audit time.** Derived, not in the six. SIGPIPE was ignored (verified), so a closed-pipe write yielded EPIPE, which then followed the text-tier **panic path → status 101 + a stderr stack trace**. For a filter piped into `head`, that turned a *normal* early-close into a crash dump. Decision 0091 now specifies and implements the clean-exit correction.
- **Options.** (a) Broken-pipe/EPIPE on a **standard stream** → **clean exit** (status 0, no panic trace) — the Unix-filter convention (coreutils, Go, Rust-with-reset-handler). (b) Keep the current panic (status 101 + trace). (c) Exit 141 (128+SIGPIPE), no trace.
- **Tradeoffs.** (a) correct for the CLI-filter product; a program piping into `head`/`less` behaves. (b) treats a normal scenario as a crash — unacceptable for product 1. (c) preserves the "signal" convention but still noisy for scripts expecting 0.
- **Recommendation → (a).** An ordinary program write failing with broken-pipe/EPIPE on `stdout`/`stderr` terminates the program cleanly, no panic trace. Panic diagnostics remain fatal if stderr is unavailable. Keep genuine file write failures on the panic→`throws` path; broken-pipe on a standard stream is a *carve-out*, not a throw the user must handle. This is a **must** for the CLI-filter use case and, like Q3, touches shipped behavior — so it needs an early decision even though the code is a separate prompt.
- **Blast radius.** `doria-rt` device write path (EPIPE / `ERROR_BROKEN_PIPE` → clean exit); 0074/0075 failure model (carve broken-pipe out of the generic write-panic); the 0074 panic-message set; SPEC; any example/fixture asserting write-failure panics. **Completed under Decision 0091; Q3 code remains scheduled for Stage 23.**

### D2 — binary stderr write [OPEN — empty cell]
- **Status.** `write_stderr` is string-only; no `Bytes` path to stderr.
- **Recommendation.** Covered by Q1(b): reserve `write_stderr_bytes(Bytes)` beside `write_stdout_bytes(Bytes)` at the Stage 23 tier. **Blast radius:** Stage 23.

### D3 — byte-tier (Stage 23) failure model not explicit [minor gap]
- **Status.** 0075 states the *text*-tier failure model precisely but does not spell out the Stage 23 byte functions'.
- **Implemented.** Byte-tier functions use the same checked `IoError` model as text I/O without performing UTF-8 validation. Allocation failure remains P1302.

### D4 — partial writes / short reads / EINTR [OPEN — contract clarification]
- **Status.** The free-function tier is all-or-nothing (whole file / whole line; failure panics). Partial writes, short reads, and `EINTR` are handled inside `doria-rt` and never surface. **Verify:** confirm `doria-rt` actually retries `EINTR` and loops short writes to completion (I did not audit every write loop — flag for the implementer).
- **Options.** (a) Free-function tier stays all-or-nothing by contract (retry/loop hidden; genuine failure panics/throws); partial-I/O *counts* are a stream-tier concern (`read()`/`write()` returning a length). (b) Expose counts at the free-function tier.
- **Recommendation → (a).** Keep partial I/O invisible at the free-function tier; the stream tier exposes counts. A genuine failure is an ambient canonical checked Error, not a panic or mandatory source `throws` entry. **Blast radius:** 0074/SPEC contract clarification; stream-tier design; a `doria-rt` verification pass on the write/read loops.

### D5 — BOM policy [derivable — state it]
- **Status.** Unspecified. Derivable from byte-exactness: `read_file` preserves bytes, so a leading UTF-8 BOM (`EF BB BF`) enters the string as `U+FEFF`; nothing strips it.
- **Recommendation.** State explicitly: **no BOM stripping** (a BOM is data); a future text helper in `Doria\Std\Fs`/`Io` may offer a BOM-aware reader. Consistent with "byte-exact, no normalization." **Blast radius:** one-line 0074/SPEC clarification.

### D6 — abort-only panic vs unflushed buffered writes [state honestly]
- **Status.** Follows from 0081 (abort-only panic runs no cleanup). The **current free-function tier is unbuffered, so it has no exposure.** The future **buffered stream tier does**: a panic while a buffered stream is open drops the unflushed buffer (no `__destruct` → no flush) — real data loss.
- **Options.** (a) Accept it and document it (durability needs an explicit `flush()` before risky work). (b) Offer a write-through/unbuffered stream mode for durability-sensitive writers. (c) A panic hook that best-effort flushes (fragile; fights abort-only).
- **Recommendation → (a), with (b) available.** Document the data-loss window plainly; let durability-sensitive code choose an unbuffered stream or flush explicitly. Do not add a panic hook. **Blast radius:** stream-tier design note; cross-reference 0081. Not a new decision — a stated consequence.

### D7 — concurrency: sendable streams, cross-task handles, stdout synchronization [DEFER · flag as design cases]
- **Status.** Unaddressed. DDO already flags "connections are not `Sendable`."
- **Recommendation → defer to the async/`Sendable`/`Shareable` decision, but flag three explicit design cases:** (1) is a `File` handle `Sendable` (movable across tasks)? (2) is shared stream access allowed only via `WritableSharedReference`? (3) is stdout writing synchronized across concurrent tasks (interleaving hazard)? Lean: stdout writes should be process-globally synchronized so concurrent `echo` cannot interleave mid-write. **Reopen trigger:** async decision. **Blast radius:** async-decision design cases; no current impact.

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
- **Public flush + stream buffering** (Q5) — deferred to the **stream tier**; unbuffered + per-stream write-order guarantee stated now. *Reopen:* `Io` stream design.
- **First-class standard streams** (Q6) — deferred to **post-Stage-29**; unify-direction recommended. *Reopen:* `Io` stream design.
- **Stream concurrency / `Sendable`** (D7) — deferred to the **async decision**; three design cases flagged. *Reopen:* async authoring.
- **`Io`/`Fs` operation placement** (D8) — the *line* is recommended now; the *operations* land `Io` (post-29) / `Fs` (unscheduled).

**Accepted in Decision 0091:** **Q3** (`append_file` name — additive, non-breaking, implementation Stage 23) and **D1** (closed-stdout/stderr broken-pipe clean exit). Every other item keeps the deferral and reopen trigger recorded above.

## Invalidated elsewhere (recommendation map)

Decision 0091 accepts only D1 and Q3 from this list. Their corresponding clauses below are now
amended; every unrelated recommendation remains historical analysis rather than accepted authority.

- **0074** — add: byte std-stream writers (Stage 23); `append_file` as the append spelling; broken-pipe carve-out from the write-panic (D1); no-BOM-strip and no-newline-normalization stated (D5); unbuffered + per-stream/same-handle write-order guarantee (Q5). Panic-message set changes for the stdout-broken-pipe case. **D1/Q3 clauses amended in 0091; other clauses remain deferred.**
- **0075** — add byte stdin/stdout/stderr to tier-2; state the byte-tier failure model (D3); note append across tiers. **Q3 clause amended in 0091; other clauses remain deferred.**
- **SPEC.md** "Stage 17 text I/O" — mirror the 0074 clarifications; state closed-stdout behavior. **D1/Q3 clauses amended in 0091.**
- **plan §9** formatted-I/O / three-tier bullets — `append_file`, byte std streams, the `Io`/`Fs` line, EPIPE behavior. **D1/Q3 clauses amended in 0091; other clauses remain deferred.**
- **`doria-rt`** — device write path: EPIPE/`ERROR_BROKEN_PIPE` → clean exit (D1, code); verify EINTR-retry / short-write loops (D4). **D1 implemented under 0091; D4 remains a preserved runtime invariant, not a newly accepted audit item.**
- **`write_file` signature — explicitly UNCHANGED** (append is additive; no breaking change). **Recorded in 0091 so no later pass adds a mode.**
- **Examples / fixtures / website & playground** — any example that pipes into `head` or asserts write-failure panics; the parity manifest's broken-pipe handling already treats `BrokenPipe` as ok in the harness (`native_mir_parity_tests.rs:255`) — reconcile with the chosen D1 semantics. Verify playground coverage in the separate `doria-website` repository before amending it; this compiler-repository audit makes no claim about that coverage. **Compiler fixtures are reconciled under 0091; website verification remains external follow-up.**

## Proposed deliverable path

`docs/notes/io-surface-audit.md` (this file) — a findings note under "supporting context" per `docs/information-architecture.md`. It is **not** a decision record: records are for settled decisions. Decision 0091 promotes the approved D1/Q3 subset into authority; every other item here remains a stop-and-ask with its recorded reopen trigger. Q3 implementation remains a Stage 23 task.
