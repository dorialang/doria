# Decision 0110: Stream, Readiness, Standard I/O, And Blocking-Mode Model

- **Status:** Accepted
- **Date:** 2026-08-02
- **Owners:** Doria language and standard-library design
- **Scope:** Stage 36a stream foundation and the contracts consumed by later I/O domains

## Context

Doria needs one portable I/O foundation for files, standard devices, child-process pipes, networking, terminals, and future async execution. PHP's stream inventory is useful as a capability checklist, but its resources, global contexts, string modes, dynamic wrapper/filter registries, mixed metadata bags, and sentinel results are not Doria's model. The completed PHP Stream And I/O Completeness Audit exposed the decisions that had to be settled before Stage 26 could resume and before Stage 36a could be implemented.

This record accepts the semantic architecture. It deliberately defers public type and member spellings that do not affect those semantics. Stage 36a is scheduled and not implemented.

## Decision

Doria will use small byte-stream capability interfaces, owned handles, explicit typed outcomes, one portable readiness substrate, and typed adapters. Text, files, processes, networking, terminals, and async execution compose this foundation rather than defining competing I/O models.

## Core Stream Architecture

- Interfaces expose the smallest capability a consumer needs: reading, writing, duplex operation, seeking, flushing, blocking-mode control, or readiness. There is no universal stream god object.
- Generic streams carry bytes. UTF-8 text, lines, delimiters, and other encodings are typed layers above byte streams.
- Platform mechanisms remain backend details. Public contracts do not expose Unix file descriptors, Windows handles, `select`, `poll`, `epoll`, `kqueue`, IOCP, or PHP resources.
- Dynamic wrapper/filter registries, global stream contexts, string-keyed option bags, and mixed metadata bags are rejected. Typed constructors, adapters, domain requests, and metadata replace them.

This separation preserves capabilities while preventing unrelated operations from being advertised by every handle.

## Ownership And Lifetime

- Ordinary stream handles are owned move values. `take` transfers the close obligation; borrows grant only their declared access.
- Explicit close or finish consumes the owned handle. Reusing it is a compile-time use-after-move error.
- Explicit close/finish reports checked failures. Destructor cleanup is best-effort, nonthrowing, and cannot replace an explicit operation when failure matters.
- Adapters own their underlying handle by default. An explicit borrowing construction is required when the adapter must not own it.
- Structured exits, including checked-error propagation, run deterministic cleanup. Abort-only panic runs no cleanup and may lose buffered data.

These rules make ownership and failure observation visible instead of hiding them behind idempotent close conventions.

## First-Class Standard Streams

- Standard input, output, and error are first-class, non-owning, nonclosable views over the same runtime devices used by existing intrinsics.
- The views may be stored, passed, and wrapped. Closing the process-wide device through a view is not permitted.
- Existing intrinsics (`echo`, `read_line`, `write_stderr`, `read_stdin_bytes`, `write_stdout_bytes`, and `write_stderr_bytes`) and first-class views share one substrate, including redirection and test injection.
- Mutating a standard device's mode requires exclusive control and a restoring guard. Concurrency must use the same ownership and synchronization model rather than inventing a second standard-I/O path.

The ordinary stdout/stderr broken-pipe carve-out remains status 0. Panic reporting remains fatal if its stderr sink is unavailable.

## Blocking Modes

- Runtime blocking-mode mutation exists only for values and platforms that support it.
- Blocking mode is a named typed state, and a capable handle exposes its current mode.
- Unsupported transitions are checked failures; they are not silently ignored.
- Static capabilities prevent clearly meaningless mode changes before execution; host-dependent limitations remain checked failures.
- Mode changes coordinate with buffered adapters so unread or pending bytes are neither discarded nor reordered.
- `Doria\Std\Io` owns generic blocking-mode behavior. `Doria\Std\Term` owns terminal raw mode and terminal events.

## Read Results

- A primitive read distinguishes data, would-block, end-of-stream, and timed-out outcomes. An operating-system failure is a checked error.
- Empty data is not EOF. Partial data is valid progress.
- Read-exact composition may fail with a checked early-EOF or timeout condition after reporting no false completion.
- Async reads preserve the same outcome vocabulary and do not reinterpret sentinel values.

Exact result type and case names are deferred, but these distinctions are not.

## Write Results

- A primitive write distinguishes accepted byte count from would-block; partial progress is observable.
- Write-all is a v1 composition that continues through writable readiness while respecting timeout, cancellation, closure, backpressure, and checked operating-system failures.
- Implementations must keep memory bounded and must not restart or duplicate already accepted bytes.
- The existing ordinary standard-output and standard-error broken-pipe status-0 rule is unchanged. Other write failures become checked errors at Stage 29.

## Readiness

- One multi-stream readiness core reports typed readable, writable, and closure facts.
- Public one-stream conveniences derive from that core; they are not a second primitive.
- Waiting supports immediate, finite, and indefinite forms plus cancellation and deadlines.
- Readiness is advisory: callers retry the operation and remain correct under spurious or stale readiness.
- Platform polling names and trigger mechanics are backend details.
- Process, network, terminal, and async facilities consume this same substrate.

## Timeouts, Deadlines, And Cancellation

- Durations and absolute deadlines are both semantic concepts. Timing is attached to an operation or wait, never mutable process-global stream state.
- Immediate and indefinite waits are explicit; negative numbers and ambiguous zero values are not magic encodings.
- One cancellation model composes with synchronous waiting, async operations, child-process pipes, networking, and terminal input.
- Exact `Doria\Std\Time` and cancellation API spellings remain owned by their dedicated design work, but Stage 36a must preserve these contracts.

## Buffering And Text

- Buffering is typed and per value. UTF-8 text readers/writers and bounded line/delimiter readers adapt byte streams.
- Adapters own their underlying stream by default, with explicit borrowed forms where needed.
- The adapter controls read-ahead and must coordinate seeking and blocking-mode changes. No operation silently discards unread data or pending output.
- Bounds are mandatory for operations that can otherwise grow without limit.
- String fills and text adapters preserve UTF-8 validity; generic byte streams never decode implicitly.
- Flushing runtime buffers is distinct from durable data or full filesystem synchronization.
- Existing unbuffered intrinsics retain their accepted behavior.

## Files

- File opening uses typed request/options values; canonical public APIs reject PHP-style mode strings. Convenience constructors may delegate to typed requests.
- The semantic mode set covers read, write, append, create, create-new, and truncate.
- Buffer flush, durable data synchronization, and full metadata synchronization are distinct operations.
- Advisory locking is v1 functionality represented by an ownership guard. A nonblocking lock attempt uses a typed outcome, not flags or sentinel values.
- Temporary files, permissions, open-handle metadata, seeking, position, length, and supported sparse-file behavior remain typed file capabilities with checked platform/filesystem failures.
- Operations on an open handle belong to `Doria\Std\Io`; namespace/path operations belong to `Doria\Std\Fs`.
- The exact `Path` representation remains deferred to the filesystem design. Stage 36a must not prevent later typed-path evolution.

## Child Processes And Pipes

- A child process is an owned move value. Active ownership must be resolved explicitly by waiting, detaching, or terminating according to a later process protocol; destruction must not silently choose among them.
- Child stdin supports half-close. Child stdout and stderr are typed readable pipes.
- Concurrent stdout/stderr drainage is mandatory to avoid deadlock. Capture is bounded.
- Direct executable invocation and shell invocation are separate typed operations.
- Exact process type-state and method names remain deferred to the process owner, but it must consume the shared stream, readiness, time, cancellation, and backpressure contracts.

## Typed Adapters And Domain Ownership

- Typed composition replaces PHP wrappers, filters, contexts, registries, string options, and mixed bags.
- Stage 36a includes byte buffering, UTF-8 text adaptation, bounded reading/writing, and streaming copy.
- Compression and encoding adapters are recommended v1 follow-ons after Stage 36a. Their absence does not block Stage 36a acceptance.
- Hashing belongs to the cryptography/hash domain; rate limiting and progress reporting are later typed operation/adapter concerns; TLS belongs to `Doria\Std\Net`.
- Metadata is typed and capability-specific.

## Cross-Domain I/O Unification

Sync and async I/O, networking, processes, and terminals share one ownership, read, write, readiness, time, cancellation, and backpressure model. Stage 37 consumes it in the async design; Stage 38 lowers async operations over it; Stage 39 applies task ownership and deterministic cancellation; networking and terminal stages add only their domain-specific behavior. No later domain may create a second foundation.

## Stage Boundaries

- The architecture review is complete and this record unblocks Stage 26. Stage 26 remains collections work; it does not implement streams.
- Stage 29 supplies checked errors, including observable explicit close/finish failures. It does not implement the stream layer.
- Stage 35 supplies the small capability-interface machinery needed by the foundation.
- Stage 36a implements this record. It is scheduled and not implemented.
- Stage 37 designs async by consuming Stage 36a readiness, ownership, time, cancellation, backpressure, and process-pipe semantics.
- Stages 38 and 39 lower and structure async execution without changing the I/O contracts.
- Stage 44 owns network-specific connection, socket, HTTP, and TLS concerns over the shared duplex foundation.
- Stage 46 owns terminal raw mode, events, screen, cursor, colour, and styling over the shared standard-device and readiness foundation.

## Deferred Public Spellings

The following are safe spelling deferrals, not semantic design gaps:

| Surface | Owner and reopen trigger |
|---|---|
| Capability interface names and members; read/write outcome and case names; readiness types; standard-stream access; property-versus-method choices; close/finish names; buffer/text/file/process names | Reopen as a decision-0110 appendix before Stage 36a implementation begins. The appendix may choose names but may not change this record's semantics. |
| `Path` representation and conversions | Reopen in `Doria\Std\Fs` design. Stage 36a preserves typed-path evolution. |
| Public duration, deadline, and cancellation types | Reopen in `Doria\Std\Time` and concurrency design no later than Stage 36a surface finalization. Duration and absolute-deadline semantics are already accepted. |
| Compression and encoding adapter names | Reopen after the Stage 36a foundation and before their v1 implementation. They remain typed adapters. |
| TLS configuration | Reopen in the Stage 44 `Doria\Std\Net` design. TLS wraps the shared duplex foundation. |
| Progress reporting | Reopen at the first operation or adapter that requires it. It must be typed and operation-local, never global or an untyped callback-code protocol. |

Candidate names in the audit remain illustrative and noncanonical until the owning trigger is reached.

## Reopening Rules

A deferred spelling may be settled without reopening this architecture. Reopening a semantic rule requires a new accepted decision that names this record, explains the conflicting use case, preserves cross-domain unification, and updates the audit ledger and guards. An implementation inconvenience is not sufficient grounds to create a parallel model.

## Consequences

- Stage 26 is unblocked and next.
- Stage 36a has a complete semantic contract but no falsely published executable surface.
- PHP capabilities are either preserved through typed owners, deliberately rejected as dynamic/resource-shaped mechanisms, or deferred with an owner and trigger.
- Existing free-function intrinsics remain valid and share the future substrate; they are not evidence that Stage 36a is implemented.
- Explicit failure, ownership, bounded-memory, and cross-platform behavior are designed once and reused across later domains.

## Invalidated Elsewhere

- Planning text that called Andrew's stream review “next” or Stage 26 “blocked pending review” is stale.
- Text that called the stream architecture unauthored or its semantics pending review is stale; only exact public spellings remain deferred.
- Any design that models Doria I/O as PHP resources, global contexts, dynamic wrappers/filters, string modes, mixed metadata bags, sentinel results, or one universal stream object is rejected.
- Any later async, network, process, or terminal API that creates a separate ownership, readiness, timeout, cancellation, or backpressure foundation is rejected.
