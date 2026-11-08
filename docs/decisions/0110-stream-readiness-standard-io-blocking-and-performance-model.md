# Decision 0110: Stream, Readiness, Standard I/O, Blocking Mode, And Performance Model

> **Stage 35 amendment:** Decision 0134 supplies nominal method-only capability
> interfaces and nonallocating erased dispatch. This record still owns the later
> stream capability names, members, and performance appendix.

- **Status:** Accepted
- **Date:** 2026-08-02
- **Owners:** Doria language and standard-library design
- **Scope:** Stage 36a stream foundation, its performance and memory contract, and the contracts consumed by later I/O domains

## Context

Doria needs one portable I/O foundation for files, standard devices, child-process pipes, networking, terminals, and future async execution. PHP's stream inventory is useful as a capability checklist, but its resources, global contexts, string modes, dynamic wrapper/filter registries, mixed metadata bags, and sentinel results are not Doria's model. The completed PHP Stream And I/O Completeness Audit exposed the decisions that had to be settled before Stage 26 could resume and before Stage 36a could be implemented.

This record accepts the semantic architecture and its binding performance and memory contract. It deliberately defers public type and member spellings that do not affect those requirements. Stage 36a is complete.

## Decision

Doria will use small byte-stream capability interfaces, owned handles, explicit typed outcomes, one portable readiness substrate, and typed adapters. Text, files, processes, networking, terminals, and async execution compose this foundation rather than defining competing I/O models. Correct behavior is necessary but not sufficient: efficient steady-state stream loops must be able to reuse storage, avoid hidden copies and mandatory allocation, process useful chunks, preserve bounded backpressure, retain static-specialization opportunities, and impose no async-runtime cost on synchronous programs.

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

### Approved Receiver Spelling

Andrew approved `consuming function` on 2026-10-04. Decision 0088 records this
general receiver mode: calling the method transfers ownership of `$this` into
the callee. Stage 36a owns its grammar and execution before stream close/finish
members use it. The spelling is settled; the remaining public-surface appendix
is not settled by this spelling amendment.

## First-Class Standard Streams

- Standard input, output, and error are first-class, non-owning, nonclosable views over the same runtime devices used by existing intrinsics.
- The views may be stored, passed, and wrapped. Closing the process-wide device through a view is not permitted.
- Existing intrinsics (`echo`, `read_line`, `write_stderr`, `read_stdin_bytes`, `write_stdout_bytes`, and `write_stderr_bytes`) and first-class views share one substrate, including redirection and test injection.
- Mutating a standard device's mode requires exclusive control and a restoring guard. Concurrency must use the same ownership and synchronization model rather than inventing a second standard-I/O path.

Andrew approved inline Copy standard views on 2026-10-18 (`1A`). Assignment
leaves both bindings usable and creates no handle, control block, buffer, or
heap object. Copies share device access and mode coordination with the existing
intrinsics. Decision 0138 records this amendment. The bounded appendix below
records the approved standard-stream accessors and type names.

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

The Byte Transfer Capabilities And Outcomes appendix fixes the result type and
case names without changing these distinctions.

## Write Results

- A primitive write distinguishes accepted byte count from would-block; partial progress is observable.
- Write-all is a v1 composition that continues through writable readiness while respecting timeout, cancellation, closure, backpressure, and checked operating-system failures.
- Implementations must keep memory bounded and must not restart or duplicate already accepted bytes.
- The existing ordinary standard-output and standard-error broken-pipe status-0 rule is unchanged. Other write failures become checked errors at Stage 29.

### Partial-Transfer Failure Facts

Andrew approved optional `IoError.transferredByteCount` on 2026-10-04. It is a
readonly `?int`: `null` means progress is unavailable or not applicable, `0`
means the operation is known to have transferred no bytes, and a positive value
is known progress within the failing operation. No backend may replace unknown
progress with zero. This fact does not promise rollback or durable storage.
Existing `IoError` catches remain valid; existing construction may omit the new
field, which defaults to `null`. A separate transfer-error type is not added.

## Readiness

- One multi-stream readiness core reports typed readable, writable, and closure facts.
- Public one-stream conveniences derive from that core; they are not a second primitive.
- Waiting supports immediate, finite, and indefinite forms plus cancellation and deadlines.
- Readiness is advisory: callers retry the operation and remain correct under spurious or stale readiness.
- Platform polling names and trigger mechanics are backend details.
- Process, network, terminal, and async facilities consume this same substrate.

### Readiness Infrastructure Errors

Andrew approved the existing `IoError` family on 2026-10-07 (`1: A`).
Readiness infrastructure failures use the ordinary reason and optional
`systemCode` fields, with `IoTarget::ReadinessSet`. `IoOperation::Register`,
`IoOperation::Unregister`, and `IoOperation::Wait` follow `Close`, preserving
every existing operation and target tag. Creating the registration resources
belongs to `Register`; removing native membership belongs to `Unregister`;
clock, cancellation-wakeup, and platform-wait failures belong to `Wait`.
These are not byte transfers: `transferredByteCount` is `null`, never invented
zero progress. Existing `IoError` catches remain valid; exhaustive operation
and target matches must cover the added cases.

Adapter preparation or completed transfers preserve their original owned
checked Error and optional byte progress. They are never rebuilt as readiness
errors. Wait expiry and cancellation are typed observations, not infrastructure
failures: expiry ends only the wait, whereas cancellation retires the transfers.

Andrew approved transfer-plan configuration failures on 2026-10-10 (`2: A`).
They use the same `IoError` reason and optional `systemCode` fields with
`IoOperation::Configure` and `IoTarget::TransferPlan`, appended after existing
cases without changing their tags. Configuration performs no transfer, so
`transferredByteCount` is `null`. Existing catches remain valid; exhaustive
operation and target matches require the added cases. This does not introduce
a separate configuration Error or change fatal owner-allocation behavior.
The shared error vocabulary and transport own these
facts across the interpreter, native profiles, and PHP compatibility.

Andrew approved `IoOperation::Access` on 2026-10-10 (`A. Add Access`). Failed
`getStream` / `getWritableStream` lookups use the existing `IoError` family with
`IoTarget::ReadinessSet`, optional `systemCode`, and `null` byte progress.
`Access` appends after `Configure` without changing existing tags. Stale or
foreign keys are rejected before dereferencing the complete registered stream;
successful lookup borrows that original value without removing or cloning it.

### Persistent Registration Ownership

Andrew approved owned registrations on 2026-10-05. A persistent readiness set
owns its registered streams: registration transfers ownership, access proceeds
through the registration, and removal returns ownership to the caller. The
original binding is unavailable after transfer; a registration is not a second
owner or an independently closable alias. Destroying the set cleans up streams
that remain registered through their ordinary deterministic ownership rules.
One-stream temporary waits may borrow for the call. This does not change the
separate owned-by-default and explicit-borrowing adapter contracts.

Andrew approved static-type-preserving removal on 2026-10-06 (`A`). Registration
retains the complete value and its registered static type `T`; its typed key
identifies that registration without owning the stream. Removal returns owned
`T`, without automatic capability erasure or runtime type discovery. Registering
a `ReadableFile` therefore preserves its file-specific operations and checked
`close()`. Deliberately registering an already-erased `ReadableStream` preserves
that interface type; removal does not recover a hidden concrete type.
Registration failure follows ordinary `take` cleanup: the caller's moved binding
is not restored, and a rejected native registration retains no handle claim.

### Adapter Readiness Sources

Andrew approved explicit borrowed readiness sources on 2026-10-06 (`A`).
Custom adapters expose an opaque borrowed readiness source of an underlying
stream through a nominal capability. It grants no
read/write operation, close obligation, public operating-system handle, or
independent lifetime. Its provenance follows the existing receiver-borrow and
elision rules, including forwarding through adapter properties.

Waiting uses that source through the shared native core. Transfers
dispatch through the complete registered value, including its buffering,
decoding, metering, or other adapter behavior; they never bypass the adapter
by transferring through a hidden underlying handle. A byte-transfer interface
alone does not imply readiness. An erased value must retain the explicit
readiness capability statically; registration cannot discover it at runtime.
Creating or forwarding a source performs no transfer and requires no allocation,
new handle owner, or reference count. Native association initializes lazily
when a wait or registration needs it, not on ordinary synchronous file opening.

### Concurrent Transfer Group Ownership

Andrew approved one exclusively borrowing transfer group on 2026-10-06
(`A`). The group acquires the readiness set and all participating source or
destination loans together. It controls simultaneous transfers through stable,
reusable request slots; resubmitting a transfer or observing a completion does
not require a new allocation. This applies to arbitrary supported concurrent
reads and writes, not only a pair of child-output pipes.

The group retains those loans for its lifetime. Original set and buffer
bindings remain unavailable until the group releases them; completing one
operation does not independently return its original binding. Completed data
may nevertheless be processed, and its storage reused, through the group while
other operations remain pending. Competing transfers cannot access the same
writable source place. Registration keys do not establish disjoint ownership
roots or authorize independently borrowed registrations.

Group teardown cancels and drains outstanding operations to terminal completion
before releasing their handles, request records, or byte-storage loans. Normal
and checked-error exits follow this same cleanup rule. The group is a narrow
compiler-checked retained-loan carrier, not permission for general borrowed
class fields or completion-dependent release of original bindings. Ordinary
synchronous `read` and `write` continue to borrow only for the call. Public
member spellings are settled through the bounded appendix before execution.

### Competing Operation Order

Andrew approved plan-order access on 2026-10-09 (`A`). Competing operations on
one stream follow plan order. Independent duplex read/write directions may
overlap; operations sharing a file cursor compete. This orders access to stream
bytes, not completion observation across independent streams.

For example, when two reads A then B each return three bytes from `abcdef`, A
receives `abc` and B receives `def`. Ordering does not turn a partial transfer
into an exact-length transfer or change cancellation, progress, and cleanup
contracts. It does not add public member spellings or a second stream owner.

### Completion Observation

Andrew approved next-completion observation on 2026-10-07 (`A`). A group
returns the next completion identifying the finished read or write, rather
than requiring callers to scan every participating transfer. Observation reuses
group storage and requires no per-completion allocation. It does not release
the group's retained loans: completed storage is processed and reused through
the group while other operations may remain pending. Absence of an available
completion is distinct from a completed operation with no transfer outcome.
Andrew also approved raising a failed transfer's original checked error when
fetching the next completion (`A`), rather than returning an error-bearing
completion whose outcome getter throws. The failed operation's identity must
be available before propagation, so its catch can identify which read or write
failed without scanning. The owned error transfers exactly once; it is not
cloned or reconstructed, and its optional progress remains intact. Catching
the error does not release the group's loans or cancel its other operations.
Public declarations and execution remain Stage 36a work.

### Completion Wait Budgets

Andrew approved wait-only expiry on 2026-10-08 (`A`). A completion-fetch wait
budget is separate from a transfer's deadline or the group's lifetime control.
Expiry stops only the wait; transfers remain active. An immediate or finite wait
that expires returns a typed timeout observation, distinct from a completion,
an exhausted group, and a transfer outcome. It does not fabricate an `IoError`
or a timed-out `ReadOutcome` for an operation that remains pending.

Expiry retains the group's storage loans, identities, and actual progress.
Callers may wait again and reuse completed storage through the same group;
expiry neither permanently stops the group nor prevents resubmission.
Cancellation or an explicit group deadline cancels and drains outstanding transfers.
Teardown retains its existing cancellation-and-drain contract. Untimed waits
require no clock sampling or per-wait allocation. Exact Time/cancellation types
and completion-observation spellings remain owned by the bounded appendix;
this approval does not claim public group execution is delivered.

### Explicit Pending-Transfer Adapters

Andrew approved an explicit pending-transfer capability on 2026-10-06 (`A`).
Concurrent adapters provide separate preparation and completion operations;
ordinary synchronous method bodies are not suspended or resumed by the compiler.
A readiness source is wait metadata, not a pending-transfer capability. The
compiler checks the required capability statically, including after deliberate
interface erasure. Ordinary synchronous adapters retain their existing contract.

The group owns stable reusable request slots and all outstanding storage loans.
Preparation runs through the complete adapter, and completion runs through that
adapter exactly once after its underlying operation has retired. Thus a counted
reader updates its count, and a buffering or decoding adapter applies its own
behavior; native submission never substitutes for the adapter's operations.
Neither callback retains a caller stack frame or gains an independent close
claim. Request reuse requires no per-transfer allocation or hidden byte copy.

Cancellation requests retirement rather than releasing a buffer. The shared
core cancels all outstanding operations before draining them; terminal progress
that wins cancellation is preserved and reaches adapter completion. A failure
before submission is distinct from a retired transfer. Original checked errors
and their optional progress facts remain owned values, not reconstructed errors.
This is a narrow group-owned protocol, not general borrowed fields, a general
worker model, an executor, or Stage 38 suspension machinery. Decision 0137
permits only the inherited synchronous-device transport exception. Its bounded public
declarations and compiler/runtime execution remain Stage 36a work.

## Timeouts, Deadlines, And Cancellation

- Durations and absolute deadlines are both semantic concepts. Timing is attached to an operation or wait, never mutable process-global stream state.
- Immediate and indefinite waits are explicit; negative numbers and ambiguous zero values are not magic encodings.
- One cancellation model composes with synchronous waiting, async operations, child-process pipes, networking, and terminal input.
- Exact `Doria\Std\Time` and cancellation API spellings remain owned by their dedicated design work, but Stage 36a must preserve these contracts.

### Shared Cancellation State

Andrew approved shared cancellation state on 2026-10-04. A token may outlive its
source and retains the same cancellation state. Creating a source may allocate
one shared state; producing a token, checking cancellation, and transferring a
chunk require no new allocation. Releasing a source drops its ownership claim;
it does not implicitly request cancellation. Waits observe the same state and
must be woken by cancellation rather than repeatedly polling it. This explicit
sharing is local to cancellation, not a change to ordinary Doria ownership.

Andrew approved inline Copy `CancellationToken` on 2026-10-11. Assigning
`let $second = $token;` leaves both bindings usable with the same state. Each
independently owned copy retains one claim and releases it during ordinary
cleanup; borrowed arguments do not gain an ownership claim. Copying allocates
nothing, does not request cancellation, and does not depend on the source
remaining alive. Andrew approved `Doria\Std\Concurrency\CancellationToken`
on 2026-10-12.

Andrew approved the source and notification-error declarations on 2026-10-12:
`Doria\Std\Concurrency\CancellationSource` is a Move owner constructed with
`new CancellationSource()`. Its readonly `token` property produces an owned
Copy token, its readonly `cancellationRequested` property reports the shared
state, and writable `cancel(): void throws CancellationError` requests
cancellation. Destruction still releases a claim without cancelling.
`Doria\Std\Concurrency\CancellationError implements Error` has readonly
`string $message` and `?int $systemCode` properties. Notification failure leaves
cancellation requested; missing system code remains null, distinct from zero.
No `create`, `getToken`, or `requestCancellation` aliases are introduced.

Andrew approved cancellation-token equality on 2026-10-13. `==` compares shared
cancellation-state identity, and `!=` is its inverse. Copies and
separately produced tokens from the same source compare equal; tokens from
different live states do not. Requesting cancellation or destroying the source
does not change that identity while token claims remain alive. Equality neither
requests cancellation nor acquires a claim, samples a clock, creates a signal,
or allocates storage. Nullable tokens follow the ordinary nullable equality
contract. No public address, integer conversion, ordering, or hashing contract
is introduced.

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
- Native platform locking is v1 functionality represented by an ownership guard. A nonblocking lock attempt uses a typed outcome, not flags or sentinel values.
- Temporary files, permissions, open-handle metadata, seeking, position, length, and supported sparse-file behavior remain typed file capabilities with checked platform/filesystem failures.
- Operations on an open handle belong to `Doria\Std\Io`; namespace/path operations belong to `Doria\Std\Fs`.
- The exact `Path` representation remains deferred to the filesystem design. Stage 36a must not prevent later typed-path evolution.

Andrew approved explicit borrowed and owning file-lock constructions and native
platform locking on 2026-10-18 (`2A, 3A`). Decision 0138 replaces this record's
advisory-only assumption: native enforcement and filesystem differences are
documented, including Windows restrictions on ordinary reads/writes. Borrowed
guards release the lock without closing the caller's file; owning guards carry
the file and lock obligations together. Both use the same underlying file
owner and deterministic cleanup. Remaining public declarations stay deferred.

## Child Processes And Pipes

- A child process is an owned move value. Active ownership must be resolved explicitly by waiting, detaching, or terminating according to a later process protocol; destruction must not silently choose among them.
- Child stdin supports half-close. Child stdout and stderr are typed readable pipes.
- Concurrent stdout/stderr drainage is mandatory to avoid deadlock. Capture is bounded.
- Direct executable invocation and shell invocation are separate typed operations.
- Exact process type-state and method names remain deferred to the process owner, but it must consume the shared stream, readiness, time, cancellation, and backpressure contracts.

### Explicit Child Cleanup Policy

Andrew approved this protocol on 2026-10-04: launching a child requires an
explicit cleanup policy, with no default. Choosing the policy in advance
satisfies the requirement that destruction must not silently choose a process
lifetime action. If an owned child is still active on normal scope exit or
checked-error propagation, cleanup follows that policy:

- **Wait:** wait for completion; this may block indefinitely.
- **Terminate:** attempt to stop the child; interruption of its work is explicit,
  and operating-system failure can prevent successful termination.
- **Detach:** release supervision and permit independent execution.

Destructor cleanup remains nonthrowing and best-effort. Explicit operations
remain available when completion status or checked failures must be observed.
An unsuccessful or timed-out explicit wait retains active ownership so the
caller can retry or choose another explicit action; timeout does not imply
termination. A completed resolution must not be repeated at destruction. The
policy travels with ownership, and abort-only panic runs no cleanup. Waiting
and capture must preserve concurrent bounded stdout/stderr drainage rather than
introduce pipe deadlock. No per-path mandatory-resolution language rule is added.

## Typed Adapters And Domain Ownership

- Typed composition replaces PHP wrappers, filters, contexts, registries, string options, and mixed bags.
- Stage 36a includes byte buffering, UTF-8 text adaptation, bounded reading/writing, and streaming copy.
- Compression and encoding adapters are recommended v1 follow-ons after Stage 36a. Their absence does not block Stage 36a acceptance.
- Hashing belongs to the cryptography/hash domain; rate limiting and progress reporting are later typed operation/adapter concerns; TLS belongs to `Doria\Std\Net`.
- Metadata is typed and capability-specific.

## Cross-Domain I/O Unification

Sync and async I/O, networking, processes, and terminals share one ownership, read, write, readiness, time, cancellation, and backpressure model. Stage 37 consumes it in the async design; Stage 38 lowers async operations over it; Stage 39 applies task ownership and deterministic cancellation; networking and terminal stages add only their domain-specific behavior. No later domain may create a second foundation.

## Performance And Memory Contract

The stream architecture must not impose avoidable allocation, copying, dynamic dispatch, buffering, readiness, timing, or async-runtime overhead. A steady-state read/process/write/readiness loop must be expressible without allocating on every iteration. Opening a stream, creating or growing a buffer, constructing an explicit adapter pipeline, creating a long-lived readiness registration set, or materializing a complete read-to-end result may allocate; each chunk read, partial write, ordinary outcome, readiness event, standard-stream access, or adapter call must not require allocation by design.

Common data, would-block, EOF, timeout, partial-progress, readable, writable, and closure outcomes use compact inline representations by default. Checked errors may carry their ordinary error objects. Standard-stream access returns lightweight views over the existing runtime devices and must not create a new operating-system handle, control block, buffer, or heap object per access.

Bulk I/O is chunk-oriented. One interface call per byte, one allocation per character, or one UTF-8 validation pass per character is not an acceptable implementation of a bulk operation. Convenience byte-at-a-time operations may exist, but bulk paths do not build on them.

Backpressure is bounded. A slow destination causes the producer to report partial progress, wait for readiness, slow down, or observe cancellation/deadline/closure. Write-all, stream-copy, process capture, async channels, and task pipelines may not convert backpressure into unbounded memory growth.

## Reusable Buffer And Byte-Region Requirement

Stage 36a must support reusable caller-owned or adapter-owned byte storage. A loop must be able to allocate storage once, read into it repeatedly, process only the valid region, write from the same storage, and preserve a partial-write offset without copying the remaining suffix.

The Reusable Bytes And Borrowed Regions appendix fixes the readonly and writable region representation and its public names. The representation preserves bounds, ownership, borrow lifetimes, readonly-versus-writable access, and invalidation after buffer reallocation. Raw pointer arithmetic in ordinary safe Doria is not the solution.

Reading into reusable storage must not construct a second buffer merely to return progress. Writing borrowed storage must not clone the complete input. Partial writes retain the original storage plus accepted offset and remaining length, or an equivalent bounded view, rather than allocating and copying an unwritten suffix. Adapter boundaries may copy only when their transformation semantics genuinely require distinct output.

## Dispatch And Specialization

Capability interfaces are public semantic contracts; they do not force dynamic dispatch on every operation. Concrete generic stream and adapter paths retain monomorphization, inlining, devirtualization, and static-dispatch opportunities. A concrete handle or adapter chain need not be boxed behind a universal interface object.

Dynamic dispatch remains valid for genuine runtime heterogeneity or deliberate type erasure. An erased path dispatches once per useful chunk or operation, does not allocate a new interface object per call, and uses compact handles where practical. Concrete adapter chains remain eligible for inline or stack storage, flattening, and monomorphized composition instead of requiring one heap allocation, reference-counted control block, or full-chunk copy per layer. The specialized and erased paths are measured separately, including their binary-size tradeoffs.

## Readiness Efficiency

Stable watched sets reuse registration storage, event storage, platform handles, and watcher identity. Ordinary waits do not rebuild the full platform readiness structure or allocate one event object per notification by default.

Immediate polling is explicit. Finite and indefinite waits use efficient platform-native mechanisms. Repeated zero-timeout checks, sleep-and-retry loops, and one thread per watched stream are rejected as the ordinary model; a narrowly required platform fallback must be documented and measured.

Timing is also opt-in. An untimed operation performs no unnecessary clock query or timer registration. Duration, deadline, and cancellation paths pay their own requested cost.

## Async Cost Isolation

A synchronous program that does not use async I/O does not initialize an executor, worker pool, timer wheel, async task allocator, readiness thread, or async cancellation registry merely because it performs stream I/O. Readiness and timing infrastructure initializes lazily when used. Stages 37 and 38 add async behavior over this foundation without imposing executor startup or task allocation on synchronous paths.

UTF-8 adapters validate incrementally, preserve decoder state across chunks, handle split multibyte sequences, and do not revalidate accepted prefixes or materialize the whole input. Line and delimiter adapters reuse their buffers where practical.

Portable public contracts hide platform differences while the runtime uses efficient native mechanisms on Linux, macOS, and Windows. No platform is forced through a lowest-common-denominator busy loop.

## Benchmark And Regression Requirements

Stage 36a owns its first performance acceptance gate. It records reproducible commands, sources, compiler/runtime versions, flags, machine, input, buffer sizes, and correctness hashes or exact output. Required metrics are compile time; cold startup; hot throughput and latency; wall, user, and system time; peak RSS; allocation count where available; system-call count where available; development, release, and stripped binary size; and generic-specialization/runtime-library growth.

The required cases are a large streaming file copy, repeated small writes, a non-blocking pipe transfer, concurrent bounded child stdout/stderr drainage, incremental UTF-8 line processing across multibyte boundaries and long lines, first-class versus intrinsic standard-output writes, many stable readiness registrations, synchronous startup, a concrete adapter chain, and an erased interface stream. File copy and process capture must not materialize unbounded complete inputs.

### Classified Unix Launch Evidence

Andrew approved PHP FFI for the Unix benchmark runner's launch evidence on
2026-10-04. The existing benchmark process adapter must distinguish an
operating-system launch failure from a program genuinely exiting 126 or 127,
including resource-measured runs. Executable preflight checks, exit status, and
stderr text are not launch proof. Without the required FFI capability, ambiguous
launch outcomes are inconclusive, never Pass. This prerequisite belongs to the
benchmark runner, not Doria programs or compiler development. Existing process
ownership, cleanup, correctness checking, and resource metrics remain required.

Where practical, equivalent algorithms and buffer sizes compare against direct runtime/operating-system, C, and Rust baselines. Unfavorable results are retained as readily as favorable ones. Claims remain benchmark-specific and include workload, version, flags, machine, input, correctness evidence, runtime, memory, and binary size; this record makes no broad superiority claim.

Before Stage 36a completion, portable correctness, structural baselines,
allocation/copy/dispatch guards, benchmark commands, and report schemas must
pass. Decision 0112's binding `1.30` native acceptance rule applies. When no
eligible controlled runner is available, record `Measurement Status: Pending
Available Runner`; pending evidence is not Pass, and hardware availability must
not block Stage 36a completion or later language development. Do not promote a
timing baseline or publish comparative performance claims without eligible
evidence. The first accepted controlled baseline defines self-regression timing
thresholds when that runner becomes available. Dedicated benchmark jobs or
curated release runs track timing; brittle wall-clock micro-thresholds do not
gate shared runners. Stage 43 incorporates and extends this suite for long-term
comparison but does not replace Stage 36a's portable gate.

## Stage Boundaries

- The architecture review is complete and this record unblocks Stage 26. Stage 26 remains collections work; it does not implement streams.
- Stage 29 supplies checked errors, including observable explicit close/finish failures. It does not implement the stream layer.
- Stage 35 supplies small capability-interface machinery while preserving static specialization for concrete generic paths and deliberate erasure only where needed.
- Stage 36a implements this record, including reusable storage, safe byte regions, structural allocation/copy constraints, and its own measured performance gate. It is complete.
- Stage 37 designs async by consuming Stage 36a readiness, ownership, reusable buffers, byte regions, time, cancellation, backpressure, and process-pipe semantics without taxing synchronous startup.
- Stages 38 and 39 lower and structure async execution without changing the I/O contracts or imposing eager executor/task infrastructure on synchronous programs.
- Stage 43 retains the Stage 36a stream cases in the long-term benchmark suite and tracks their accepted baselines.
- Stage 40a owns the first network-specific connection, HTTP-client, and TLS foundation over shared duplex streams, reusable buffers, byte regions, readiness, and backpressure. Stage 44 owns network-specific maturation and performance hardening beyond that foundation.
- Stage 46 owns terminal raw mode, events, screen, cursor, colour, and styling over the shared standard-device and readiness foundation.

## Stage 36a Public Surface Appendix

This appendix assigns spellings to the accepted contracts above. A declaration
here is language authority, not a claim of executable implementation; delivery
and validation remain recorded in the plan and current-pipeline notes.

### Standard-Stream Accessors

Andrew approved S-A on 2026-10-18: "Approve S-A: StandardStreams properties".
`Doria\Std\Io\StandardStreams` exposes readonly static properties:

- `input`: `Doria\Std\Io\StandardInput`
- `output`: `Doria\Std\Io\StandardOutput`
- `error`: `Doria\Std\Io\StandardError`

Access uses ordinary static property syntax, for example
`let writable $out = StandardStreams::output;` after importing `StandardStreams`.
Each property returns the corresponding non-owning, nonclosable inline Copy
view defined by Decision 0138. Repeated access and copying share the canonical
process device and its access/mode coordination, without a new handle, buffer,
control block, or heap object. This ruling fixes these declarations; remaining
blocking, control, adapter, process, and lock spellings stay in this appendix.

### Reusable Bytes And Borrowed Regions

`Bytes` remains the existing owned, fixed-length mutable byte container. The
new `Bytes::allocate(int $length): Bytes` creates zero-initialized storage once;
negative lengths are programmer-error panics, and allocation
failure keeps the existing byte-buffer allocation panic (P1302). Binary reads never create a
`string` or validate UTF-8. A separate growable-buffer type is not required to
reuse this storage.

The region types are `Doria\Std\Io\ByteRegion` (readonly) and
`Doria\Std\Io\WritableByteRegion`. Neither owns storage or extends its owner's
lifetime. Their source operations are:

- `Bytes->getRegion(int $offset, int $length): ByteRegion` borrows readonly.
- `writable Bytes->getWritableRegion(int $offset, int $length): WritableByteRegion`
  borrows writable.
- Both region types have readonly `int $length` and indexed `uint8` reads.
- Only a writable `WritableByteRegion` permits indexed writes and ordinary RMW.
- `ByteRegion->getRegion(int $offset, int $length): ByteRegion` narrows a readonly
  view. The corresponding writable operation on `WritableByteRegion` is
  `getWritableRegion`; its readonly `getRegion` produces a readonly reborrow.

- `writable WritableByteRegion->copyFrom(ByteRegion $source): int` copies the
  leading bytes of `$source` into the start of the region and returns the
  count, the smaller of the two lengths. The borrow rules keep the two
  regions on distinct storage.
- `writable WritableByteRegion->copyWithin(int $offset, int $length, int
  $destination): void` moves `$length` bytes from `$offset` to `$destination`
  inside the region. The ranges may overlap. The source range is checked
  first, then the destination range; either failure is the subregion panic.
  Settled 2026-11-06 under the Stage 36a standing approval (decision log
  entry 38). Both lower to one memmove.

These signatures describe receiver access, not additional grammar for declaring
methods. Source calls use ordinary `->`; nullable byte containers and regions
use `?->` for null-safe member access or narrow first. Null-safe results preserve
both absence and the original source loan; they do not grant writable access.
Offsets and lengths are canonical
`int`, byte-based, nonnegative, and checked without overflowing their sum.
Empty regions at the end of storage are valid. Out-of-bounds formation and
indexing follow ordinary bounds-panic behavior rather than unchecked pointers.

Regions use inline pointer/length carriers with compiler-tracked provenance;
forming, narrowing, passing, returning, or dropping a region allocates no heap
object, copies no underlying bytes, and does not retain a runtime reference
count. `ByteRegion` is a readonly borrowed Copy carrier. `WritableByteRegion`
is a borrowed move carrier, so copying its descriptor cannot create a second
writer. Making a binding writable cannot turn a readonly region into a
writable region.

Existing borrow rules apply to the complete source place: a live writable
region excludes competing access; live readonly regions exclude mutation;
neither permits moving or destroying the source. A subregion reborrows its
parent and keeps the original provenance; it does not create an independent
ownership claim. This does not add a disjoint-region borrowing exception or
general borrowed class fields. A return, capture, stored value, or adapter may
retain a region only when the existing ownership/lifetime analysis proves its
source outlives the use. Reallocation, when exposed by a future storage type,
must be rejected while a region is live, not repaired by hidden copying.
Named region returns obey Decision 0089's elision rule: all returned views
derive from the same borrowed parameter or from `$this`. A local control-flow
join may retain several possible source loans without authorizing an ambiguous
returned-borrow contract.

A transfer consumes neither its source bytes nor its destination storage.
Primitive reads borrow a writable region and report the initialized byte count;
primitive writes borrow a readonly region and report the accepted byte count.
Continuing a partial write narrows the same storage using the accepted offset.
The borrow ends through ordinary scope/liveness rules, so a loop can reuse its
one owned buffer for the next chunk without rebuilding a buffer per read.

### Byte Transfer Capabilities And Outcomes

The nominal interfaces and enums below belong to `Doria\Std\Io`. They use
ordinary interface conformance, generic specialization, checked effects, and
Decision 0114's inline enum representation; no stream-specific boxing model is
introduced.

```doria
enum ReadOutcome {
    case Data(int $byteCount);
    case WouldBlock;
    case EndOfStream;
    case TimedOut;
}
enum WriteOutcome {
    case Accepted(int $byteCount);
    case WouldBlock;
}
enum FlushOutcome {
    case Complete;
    case Pending;
}
interface ReadableStream {
    writable function read(writable WritableByteRegion $destination): ReadOutcome throws IoError;
}
interface WritableStream {
    writable function write(ByteRegion $source): WriteOutcome throws IoError;
}
interface DuplexStream extends ReadableStream, WritableStream {}
interface FlushableStream {
    writable function flush(): FlushOutcome throws IoError;
}
```

`read` and `write` borrow their storage for the call and do not retain a region.
`Data` reports how many destination bytes are initialized; `Accepted` reports
how many source bytes were accepted. Counts are between zero and the supplied
region's length. An empty request succeeds with zero progress; it does not
report EOF. `WouldBlock` reports no progress. `EndOfStream` is distinct from
empty data. `TimedOut` preserves the accepted read-outcome vocabulary for a
reader with an operation-local wait budget. These compact Copy outcomes never
own byte storage or require heap allocation. Operating-system failures use
`IoError`, including its optional byte-progress fact.

Andrew approved "A: One resumable flush contract (Recommended)" on 2026-11-03.
This amendment replaces `FlushableStream.flush(): void` with the inline Copy
`FlushOutcome` above. `Complete` reports completed drainage; `Pending` reports
retained output that cannot yet be drained, including after partial progress in
the same call. Retrying resumes the retained remainder without repeating bytes
already accepted by the destination. No readiness wait or busy polling is
inserted automatically. Checked failures still throw `IoError`; flushing does
not promise rollback or durable filesystem synchronization. The result owns
no storage and requires no heap allocation. Text-adapter declarations remain
a separate review.

Andrew approved "A: Return completion or owned retry (Recommended)" on
2026-11-03 for buffered-writer consuming finish. One non-waiting drainage
attempt returns an owned typed result: completion returns the underlying owned
source; pending returns the same wrapper, retaining its source, bytes and
position for retry without a new buffer copy. Borrowed forms release their
source loan on completion and retain it on pending, without transferring the
source owner. Checked I/O failures still throw and run ordinary consuming-call
cleanup; recovery of the consumed wrapper is not promised. Waiting remains
explicit. The direction approval alone established no result type spelling,
borrowed-field rule or general move-out permission.

Andrew subsequently approved "A: General generic finish enums (Recommended)"
on 2026-11-03, settling these declarations in `Doria\\Std\\Io`:

```doria
enum FinishOutcome<S, R>
{
    case Complete(S $source);
    case Pending(R $writer);
}

enum BorrowedFinishOutcome<R>
{
    case Complete;
    case Pending(R $writer);
}
```

An owned writer returns `FinishOutcome<T, ItsOwnType>`; a borrowed writer
returns `BorrowedFinishOutcome<ItsOwnType>`, where `ItsOwnType` denotes the
concrete adapter type, not new source syntax. Payloads transfer ownership and
retained source loans; matching releases or forwards those loans according to
the selected case. Decision 0114 owns the approved general generic-enum
implementation: inline tagged layout, Copy only when every substituted payload
is Copy, ordinary checked cleanup and no additional result allocation. Both
declarations execute on every backend as of 2026-11-05. The adapter storage and
extraction rulings below settle construction and source hand-back.

Receiver access is writable because a read may update stream position or
decoder state, and a write may update position or buffering. This does not
consume the stream or add a mode-changing capability. `DuplexStream` combines
only reading and writing; flushing is a separate capability and does not imply
durable filesystem synchronization. Neither transfer capability exposes close,
so nonclosable standard-device views can satisfy it. Owned handles expose their
consuming lifecycle operations separately.

### Owned File Family

Andrew approved this file-family spelling on 2026-10-05. All four names belong
to `Doria\\Std\\Io`; `File` is the factory companion, not a universal stream
handle or a `File<Capability>` generic family.

| Factory               | Owned Result   | Transfer Capability |
|-----------------------|----------------|---------------------|
| `File::openRead`      | `ReadableFile` | `ReadableStream`    |
| `File::openWrite`     | `WritableFile` | `WritableStream`    |
| `File::openReadWrite` | `DuplexFile`   | `DuplexStream`      |

For example, `File::openRead("input.bin")` returns an owned `ReadableFile`.
The concrete result determines available transfer capabilities statically;
it does not grant write access to a readable-only handle or require callers
to name a capability type argument. Transfers retain the writable receiver,
borrowed-region, compact-outcome, and checked-error contracts above.

Andrew approved explicit writable-open creation policies and canonical checked
close on 2026-10-05. `File::openWrite` and `File::openReadWrite` require an
authored `FileCreation` argument; no default creates, truncates, or opens a file
on the caller's behalf. The cases are `OpenExisting`, `OpenOrCreate`, `CreateNew`,
`TruncateExisting`, and `CreateOrTruncate`. They respectively open without
creation/truncation, create only when absent, require a new file, truncate only
an existing file, or create/truncate. `File::openRead` opens an existing file
without creating or truncating it. Append is the typed `FileWriteMode::Append`
operation, never a string mode or a creation policy.

Andrew approved required explicit write placement on 2026-10-05.
`FileWriteMode` has exactly `Positioned` and `Append`. Both writable factories
require the mode argument, independently of `FileCreation`; neither has a
default. Positioned writes start at the open handle's current position and
advance it by accepted bytes. Append places every primitive write at the then
current end of the file, including when another handle has extended it. Append
does not create or truncate a file by itself. Every open starts with its own
cursor at zero; duplex reads retain that cursor and their ordinary read contract.

The entry points have these ordinary callable signatures:

```doria
class File {
    static function openRead(string $path): ReadableFile throws IoError;
    static function openWrite(string $path, FileCreation $creation, FileWriteMode $mode): WritableFile throws IoError;
    static function openReadWrite(string $path, FileCreation $creation, FileWriteMode $mode): DuplexFile throws IoError;
}
class ReadableFile implements PendingReadableStream, SeekableStream {
    writable function read(writable WritableByteRegion $destination): ReadOutcome throws IoError;
    writable function prepareRead(writable ReadRequest $request, writable WritableByteRegion $destination): void throws IoError;
    writable function completeRead(ReadRequest $request, writable WritableByteRegion $destination): ?ReadOutcome throws IoError;
    consuming function close(): void throws IoError;
    ReadinessSource $readinessSource { borrowed get; }
}
class WritableFile implements PendingWritableStream, SeekableStream {
    writable function write(ByteRegion $source): WriteOutcome throws IoError;
    writable function prepareWrite(writable WriteRequest $request, ByteRegion $source): void throws IoError;
    writable function completeWrite(WriteRequest $request, ByteRegion $source): ?WriteOutcome throws IoError;
    consuming function close(): void throws IoError;
    ReadinessSource $readinessSource { borrowed get; }
}
class DuplexFile implements DuplexStream, PendingReadableStream, PendingWritableStream, SeekableStream {
    writable function read(writable WritableByteRegion $destination): ReadOutcome throws IoError;
    writable function write(ByteRegion $source): WriteOutcome throws IoError;
    writable function prepareRead(writable ReadRequest $request, writable WritableByteRegion $destination): void throws IoError;
    writable function prepareWrite(writable WriteRequest $request, ByteRegion $source): void throws IoError;
    writable function completeRead(ReadRequest $request, writable WritableByteRegion $destination): ?ReadOutcome throws IoError;
    writable function completeWrite(WriteRequest $request, ByteRegion $source): ?WriteOutcome throws IoError;
    consuming function close(): void throws IoError;
    ReadinessSource $readinessSource { borrowed get; }
}
```

Construction goes through the factory, not `new File` or `new ReadableFile`.
The runtime owns the private handle payload and native layout; the compiler
uses ordinary owned calls, interface dispatch, checked propagation, and static
drop glue. Reads use the caller's region directly, maintain an independent
cursor for each open, and return `Data(0)` for an empty destination, including
at EOF. A nonempty read at EOF returns `EndOfStream`.
Public file owners permit simultaneous readers and writers on all tier-1
platforms; Windows opens share read and write access, without granting deletion
sharing. Existing free file helpers retain their established sharing contracts.

```doria
enum FileCreation {
    case OpenExisting;
    case OpenOrCreate;
    case CreateNew;
    case TruncateExisting;
    case CreateOrTruncate;
}
enum FileWriteMode {
    case Positioned;
    case Append;
}
```

Each owned result exposes `consuming function close(): void throws IoError`.
Close failures use `IoOperation::Close`, appended after the existing operation
cases without changing their tags; existing `IoError` catches remain valid.
Both success and checked failure consume the receiver and resolve its one native
close obligation. Error construction preserves the opening path; destruction
does not retry a failed close. Ordinary destruction remains nonthrowing and
best-effort. Explicit close is not buffer flush or durable synchronization.

### Seeking And Readonly File Inspection

Andrew approved readonly `getPosition()` and `getLength()` methods on
2026-10-13. This replaces the proposed position/length properties: fresh native
queries can block, so the global nonblocking property-hook rule in Decision
0135 remains unchanged. There are no property aliases or hook exemptions.

Andrew approved both choices on 2026-10-13: seeking returns the new position,
and seek failures use a distinct appended `IoOperation::Seek` case. All names
belong to `Doria\\Std\\Io`.

```doria
enum SeekOrigin { case Start; case Current; case End; }
interface SeekableStream {
    writable function seek(int $offset, SeekOrigin $origin): int throws IoError;
}
// ReadableFile, WritableFile and DuplexFile each implement SeekableStream:
writable function seek(int $offset, SeekOrigin $origin): int throws IoError;
function getPosition(): int throws IoError;
function getLength(): int throws IoError;
```

Offsets and results count bytes, not text characters. Origin is required, with
no implicit default. Start is relative to byte zero; Current uses the handle's
current cursor; End uses its current length. A signed offset is valid when the
resulting position is nonnegative and supported by the platform/filesystem.
Seeking beyond EOF does not itself extend the file; later positioned writes
retain supported sparse-file behavior. Append writes still target the then
current EOF, irrespective of an earlier seek. Inspection is readonly, retains
no loan beyond the call, and does not change the cursor or contents. Length is
sampled from the open handle, not a cached path lookup. Neither inspection is a
stored field or a setter.

Seek failures use `Seek`; inspection failures use existing `Access`. Both use
the existing `IoError`, original opening-path `IoTarget::File`, null byte
progress and an optional platform code. Existing operation tags remain stable;
exhaustive matches need the appended Seek arm. Unrepresentable canonical-int
results are checked `InvalidInput`, never wrapped or truncated. Successful
primitive calls allocate no storage and start no readiness/worker infrastructure.

Remaining metadata and option-control member spellings
reopen through this appendix before public execution. The `Path` representation
remains owned by the filesystem design.

### Durable File Synchronization

Andrew approved both full-word names and distinct failure labels on 2026-10-13.
WritableFile and DuplexFile expose these writable methods; ReadableFile does not:

```doria
writable function synchronizeData(): void throws IoError;
writable function synchronizeMetadata(): void throws IoError;
```

Data synchronization requests durable file contents; metadata synchronization
requests full data-plus-metadata synchronization. Neither is a runtime-buffer
flush, and neither changes the cursor, length or bytes. A platform may provide
the stronger full guarantee for both calls, but may not silently substitute a
weaker guarantee. Linux uses fdatasync/fsync respectively; macOS uses F_FULLFSYNC
for both without a weaker fallback; Windows uses FlushFileBuffers for both.
These calls may block and therefore remain prohibited in property hooks,
including through helpers.

Failures append `IoOperation::SynchronizeData` and
`IoOperation::SynchronizeMetadata` after Seek without changing existing tags.
Both preserve the existing IoError, opening-path File target, optional system
code and null byte progress. Successful calls allocate no storage or initialize
readiness, worker or async infrastructure. Readback proves visible bytes and
successful transport, not power-loss durability.

### Resizing An Owned File

Andrew approved `setLength` and the distinct Resize failure label on 2026-10-13.
WritableFile and DuplexFile expose:

```doria
writable function setLength(int $length): void throws IoError;
```

Length is a required nonnegative byte count, with no default. Shrinking discards
the tail; growth makes the extended range read as zero, retaining supported
sparse-file behavior without a user-space fill. The cursor remains unchanged,
even when beyond the new EOF. Other opens retain their own cursors and observe
the new extent on fresh inspection. Append mode still targets the then-current
EOF on subsequent writes. The method borrows writable for the call and does
not consume the owner; ReadableFile exposes no resizing method or alias.

Negative lengths fail with InvalidInput. Other platform/filesystem failures
remain checked IoError, labelled `IoOperation::Resize`, appended after
SynchronizeMetadata without changing earlier tags. Errors preserve the original
opening-path File target, optional system code and null byte progress. A failed
operation does not promise rollback of previous file effects. Resizing may block
and remains prohibited in property hooks, including through helpers. It neither
flushes buffers nor requests durable synchronization.

Native controls allocate no storage. The interpreter's supplied virtual-file
storage grows fallibly and reports ResourceExhausted rather than panicking or
imposing an arbitrary extent cap; ordinary virtual-file writes share this path.
The Unix implementation uses [ftruncate](https://pubs.opengroup.org/onlinepubs/9699919799/functions/ftruncate.html).
Windows changes [FileEndOfFileInfo](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-setfileinformationbyhandle)
directly without temporarily repositioning the cursor or using SetFileValidData.

### PHP Native Runtime And Windows Completion

Andrew approved both directions on 2026-10-06 (`PHP: A; Windows: A`).
Writable-file compatibility uses the shared native runtime through PHP FFI,
not PHP stream modes or a second implementation of file policies. Deployment
therefore requires enabled PHP FFI and a matching native shared runtime; missing
or incompatible dependencies must fail explicitly, without a host-I/O fallback.
The compiler/toolchain artifact workflow owns library identity and delivery, not
an absolute Cargo target path. Creation, append placement, platform failure
facts, borrowed storage, and consuming close retain the same contracts.

Windows pipe transfers use submit-then-await completion rather than simulated
readiness or hidden read-ahead buffers. An outstanding transfer keeps its handle,
operation record, and source/destination borrow alive until terminal completion.
Cancellation requests do not release that storage: cancellation must be drained,
and completion that wins the race preserves actual progress. This distinct
pending-operation contract does not let synchronous `read` or `write` retain a
region after returning. Public pending-operation names reopen through the bounded
appendix before execution; ordinary synchronous programs need no executor,
worker thread, or per-transfer buffer allocation. Decision 0137 permits lazy,
bounded native-only helpers for pending transfers through inherited synchronous
standard devices when native completion or safe mode control is unavailable.
Their cancellation/drain may wait for actual host-operation retirement; the
same group, retained-buffer, and driver-thread callback contracts apply.

### Standalone Clock Failures

Andrew approved Time-owned `Doria\Std\Time\ClockError` on 2026-10-08 for
standalone clock-read failures. They are not stream operations and do not gain
clock-specific `IoOperation` or `IoTarget` cases. Readiness infrastructure still
reports failure through the accepted `IoError` wait contract, including a failed
clock query performed as part of a timed wait. Both paths preserve the original
platform failure facts through the shared private clock implementation. Exact
clock, duration, and deadline operations remain subject to the bounded surface
appendix before public execution; this ruling settles error ownership, not an
additional clock model. The error follows the ordinary checked Error contract:

```doria
class ClockError implements Error {
    function __construct(string $message, ?int $systemCode = null) {}
}
```

Both promoted properties are readonly. A missing platform code stays `null`,
distinct from a known zero; propagation and rethrow preserve the same owned
Error. No I/O operation, target, byte progress, or separate reason enum is added
to this Time error. `ClockError` is qualified or explicitly imported, not a
prelude alias.

### Inline Copy Time Values

Andrew approved `Doria\Std\Time\Duration` and `Doria\Std\Time\Deadline` as
compiler-known inline Copy values on 2026-10-08. Assignment, argument passing,
and returning a value leave the original usable; neither value requires a heap
object, ownership transfer, reference counting, or destruction. They retain
distinct nominal identities and do not implicitly convert to integers or to
each other. Nullable values preserve this Copy classification. This does not
introduce user-defined Copy classes: ordinary classes remain Move values even
when all their fields are Copy.

Andrew selected signed `int64` nanoseconds for `Duration` on 2026-10-08
(`B then`). Negative, zero, and positive intervals are valid, including the
entire `int64` range. `Duration::fromNanoseconds(int64 $nanoseconds): Duration`
constructs the value; its readonly `int64 $nanoseconds` getter returns the exact
interval. This explicit factory and getter do not introduce implicit integer
conversion. The choice supports signed time differences rather than restricting
a general-purpose duration to waiting budgets.

Andrew approved negative-wait and relative-deadline expiry on 2026-10-08
(`1: A`). A negative interval used for a timed wait or relative deadline is
already expired, not invalid, an immediate poll, or an indefinite wait. The
Duration value itself remains unchanged. Completion-fetch expiry stops only
observation; a group deadline retains its separate cancellation/drain contract.
Andrew approved exact past Deadline identity on 2026-10-08 (`2: A`). Expiry
does not erase its instant: two different past deadlines remain different
values, preserving their ordering and differences. The private inline carrier
must represent the unsigned monotonic clock plus the full signed Duration
range without truncating, saturating, or replacing past values with a marker.
Further Time operations and their checked overflow boundaries remain in the
bounded Time appendix.
Native carrier bits remain private compiler ABI, not a public clock epoch.

Andrew approved relative construction on 2026-10-08:
`static function Deadline::fromNow(Duration $duration): Deadline throws ClockError`.
The factory samples the shared monotonic clock once and adds the exact signed
interval. It retains negative past instants and needs no timer/readiness
infrastructure or native heap allocation on success. Clock-read failure uses
the Time-owned checked error above, including its original optional system code.
The factory lives on Deadline, matching Duration's factory pattern; it does not
introduce a public Clock companion or additional arithmetic operations.

### Explicit Wait Value

Andrew approved one ordinary inline Copy `Doria\Std\Io\Wait` enum on
2026-10-08 (`1: A`): `Immediate`, `For(Duration $duration)`,
`Until(Deadline $deadline)`, and `Indefinite`. It is qualified or explicitly
imported, not a prelude alias. The enum uses the shared payload representation
and introduces no heap container or timer infrastructure. Completion observation
uses `getNextCompletion(Wait $wait): TransferWaitResult`; the remaining group
declarations stay in the bounded appendix. `For` freezes one deadline when observation
starts, not once per retry. `Until` preserves the supplied Deadline; expiry
stops only that observation and leaves transfers available to a later wait.

### Readiness Source Capability

`Doria\Std\Io\ReadinessSource` is an opaque receiver-borrowed view, not a
constructible or independently owned handle. `ReadinessStream` is the nominal
capability that exposes it through an ordinary borrowed getter:

```doria
interface ReadinessStream {
    ReadinessSource $readinessSource { borrowed get; }
}
```

The readable, writable, and duplex file owners implement this capability in
addition to their transfer interfaces. A user-written adapter implements it
explicitly, for example with `borrowed get => $this->source->readinessSource`.
The getter does not perform I/O or change the public stream state. Its returned
loan prevents the original owner from being moved or destroyed while live;
returning a source of a local owned stream is rejected. Storing or capturing
the view remains subject to the existing borrowed-value rules, not a general
borrowed-field exemption. The carrier has no public constructor, members, or
platform-handle conversion. Public registration and transfer execution still
require their separate bounded declarations and validation.

### Typed Registration Keys

Andrew approved `Doria\Std\Io\RegistrationKey<T>` as an opaque inline Copy
value on 2026-10-09 (`A`). Assignment, argument passing, and returning a key
leave the original usable, including nullable keys. A key identifies a
registration without owning the stream or extending its lifetime; the set owns
the complete registered value. Copying a key allocates nothing and acquires no
handle, reference count, or independent stream loan.

The exact registered static type `T` remains part of the key's nominal identity
through compiler checking and validated MIR. Keys do not implicitly convert
between registered types or to integers, and expose no platform handle.
Registration-key copies may become stale. Stale and foreign keys are rejected
before accessing a registration; copying never bypasses the set/group's normal
borrowing rules. The private carrier and validation mechanism remain compiler
ABI, not public layout. Remaining set and group member spellings still require
their bounded declarations before public execution.

### Registration And Removal

Andrew approved the `register` / `unregister` pair on 2026-10-09 (`A`).
`$key = $set->register($stream)` transfers the complete stream into the set and
returns `RegistrationKey<T>` for its registered static type `T`.
`$stream = $set->unregister($key)` returns the same owned `T`, not a readiness
source or an automatically erased capability. Other key copies become stale.
Registration and removal require writable set access and retain the existing
checked `IoError` and ordinary `take` cleanup contracts. These names do not
settle borrowed access, wait, or group member declarations.

### Borrowed Registration Access

Andrew approved `getStream` / `getWritableStream` on 2026-10-10 (`1: A`).
`$set->getStream($key)` lends the registered `T` readonly;
`$set->getWritableStream($key)` requires writable set access and lends it
writable. Neither call removes, copies, erases, or separately owns the stream.
The set remains the loan source, so ordinary borrowing prevents incompatible
set mutation, group creation, or destruction while that loan is live.
Stale and foreign keys are rejected before accessing the registered value.
These names do not settle the remaining wait or group members.

### Registered Source Changes

Andrew approved source-following registration on 2026-10-10 (`B`). Replacing an
adapter's underlying file does not require unregistering the complete adapter
or changing its key. Future waits and transfers follow its current readiness
source. Unchanged sources reuse their native registration; replacing a source
must not leave a cached pointer, handle, or error path referring to destroyed
storage.

An already-started operation retains its selected source and required native
resources through retirement. A later source change does not retarget that
operation or release its storage early. Cancellation and teardown still drain
pending work before releasing these obligations. This applies to native,
hosted, and compatibility paths without changing ordinary borrowing rules or
giving the public `ReadinessSource` view independent ownership.

### Empty Set Construction

Andrew approved `new ReadinessSet()` on 2026-10-09 (`A`). The constructor
creates an empty owning set and declares checked `IoError` setup failures.
It registers no stream and starts no transfer. Native association remains lazy;
construction does not choose or restrict the capabilities of future members.
There is no separate `create` factory.

### Transfer Plan Setup

Andrew approved the consuming-plan setup on 2026-10-06 (`Public API choice: A`).
`Doria\Std\Io\TransferPlan` describes all participating reads and writes before
the group starts. Andrew approved the exact pairs `read` / `andReadFrom` and
`writeTo` / `andWriteTo` on 2026-10-11 (`1: B; 2: B`). This replaces the earlier
`andRead` spelling; it does not introduce `readFrom`, `write`, or `andWrite`
aliases. `TransferPlan::read` starts a read plan; consuming `andReadFrom` adds
another read and returns the plan with both retained destination loans.
`TransferPlan::writeTo` starts a write plan; consuming `andWriteTo` adds a write.
Read operations retain exclusive writable destination loans, while write
operations retain shared readonly source loans. Either append may follow either
factory, preserving all earlier loans and the new operation's exact pending
capability. Shared write sources may overlap; a conflicting read destination
cannot overlap another retained region.
Configuration may allocate request storage, but it performs no transfer.
Moving the plan forwards its retained loans; dropping an unstarted plan
releases them without starting or cancelling I/O.

```doria
let $plan = TransferPlan::read($outKey, $out->getWritableRegion(0, 4096))
    ->andReadFrom($errKey, $err->getWritableRegion(0, 4096));
let writable $group = $set->begin($plan);
```

`begin` takes the plan through an ordinary `take TransferPlan` parameter, not a
call-site `take` expression. It exclusively borrows the set and forwards every
plan source loan into the resulting group. The plan is consumed on success and
checked failure. Failed setup releases the plan's loans without leaving an
active group; any started operation is drained before release. All validation
and configuration allocations precede submission. Completed buffers are
processed and reused through the group, not their still-borrowed original
bindings. Teardown follows the Concurrent Transfer Group Ownership contract.

Andrew approved group lifetime-control parameters on 2026-10-13:

```doria
writable function begin(
    take TransferPlan $plan,
    ?Doria\Std\Time\Deadline $deadline = null,
    ?Doria\Std\Concurrency\CancellationToken $cancellation = null
): TransferGroup throws IoError;
```

`deadline` and `cancellation` are optional named parameters using the existing
types; null omits that lifetime control. No `TransferControl` type is added.
The group retains the token's shared-state claim, not a CancellationSource loan,
and freezes the supplied exact deadline for its lifetime. Reusing the readiness
set for another group does not inherit the previous group's controls.
Cancellation or group-deadline expiry cancels and drains outstanding transfers
before storage release. `getNextCompletion` keeps its separate wait-only
budget, whose expiry leaves transfers active. Controls do not change plan
consumption, setup-error labels, or original adapter-error propagation.
Their executable compiler/runtime integration remains Stage 36a work.

Andrew approved `IoOperation::Configure` / `IoTarget::TransferGroup` for
`begin`'s own checked setup failures on 2026-10-11. Plan validation and native
setup use the same caller-facing labels, preserving reason and system-code
details. An adapter's original owned checked error propagates unchanged,
not relabeled or reconstructed. Failure still consumes the plan and leaves
no active group or submitted work.

This approves the plan-versus-callback shape, not new general borrowed fields,
an independent registration borrower, or public readiness spellings absent
from this appendix. The remaining registration, wait, completion, and control
members must preserve this setup and lifetime contract when their bounded
spelling review is finalized. Delivery remains in progress.

### Typed Transfer Identifiers

Andrew approved `Doria\Std\Io\ReadTransferKey` and `WriteTransferKey` on
2026-10-11 (`1: B`), instead of integer transfer indexes. These are distinct
nominal inline Copy values with opaque private bits and no public constructor.
A write key cannot stand in for a read key, either key cannot stand in for
`RegistrationKey<T>`, and neither converts to or from an integer. Copies and
transport require no independent heap allocation or resource ownership.
Andrew approved positional retrieval on 2026-10-11 (`A: Position lookup, typed
keys`): readonly `getReadKey(int $position): ReadTransferKey throws IoError`
and `getWriteKey(int $position): WriteTransferKey throws IoError` use zero-based
positions across the entire mixed plan. Negative, out-of-range, or wrong-direction
positions fail with existing `IoError`; lookup performs no I/O. The shared
lookup failure projection uses `Access`, `TransferPlan`, and `InvalidInput`,
with no path, system code, or byte progress. Earlier keys
retain their identity through consuming appends, repeated operations on one
registration have distinct keys, and separate plans have distinct identities.
Copying a key retains no plan, buffer, or stream loan.
Remaining group member names must use these exact operation identities;
this ruling does not settle those members or their executable integration.

### Completion And Wait Results

Andrew approved `Doria\Std\Io\TransferGroup` as the reusable transfer owner
on 2026-10-11 (`A: TransferGroup`). Its readonly `observedReadKey` and
`observedWriteKey` properties have types `?ReadTransferKey` and
`?WriteTransferKey`, respectively (`A: Two directional properties`). Only the
fetched operation's direction is present. Both are null before the first fetch
or when a fetch reports timeout or exhaustion. They expose the consumed plan's
original operation identity, including its position in a mixed plan, before
propagating an original checked error. Resubmission preserves that identity;
copying an observed key retains no group or storage loan. These properties do
not replace the typed key carried by a successful completion.

Andrew approved separate wait and completion results on 2026-10-11 (`B`):

```doria
enum TransferCompletion {
    case Read(ReadTransferKey $key, ?ReadOutcome $outcome);
    case Write(WriteTransferKey $key, ?WriteOutcome $outcome);
}
enum TransferWaitResult {
    case Completed(TransferCompletion $completion);
    case TimedOut;
    case Exhausted;
}
```

Both are ordinary nominal inline Copy enums in `Doria\Std\Io`, not heap
containers, ownership claims, or public constructors for operation keys.
`Completed(Read($key, null))` identifies a retired read with no transfer outcome;
it is distinct from `TimedOut` and `Exhausted`. Actual zero-byte, would-block,
EOF, and timed-out read outcomes remain inside their applicable completion.
Failed fetches still throw the original owned checked Error, rather than
storing or recreating it in either enum. Timeout and exhaustion observations
do not permanently stop the group or prevent later resubmission.
Constructing or copying an enum does not retire work, release loans, or grant
buffer access. The group owns those transitions. This result-shape ruling
does not claim public group execution is delivered.

Andrew approved wait-infrastructure failure first on 2026-10-12. When a fetch
encounters such a failure and a transfer result is also available,
`getNextCompletion` reports the wait failure without taking that result.
Subsequent fetches still observe the completed bytes or the original owned
transfer Error, exactly once. The wait failure has no transfer identity or
invented byte progress; its fetch leaves both observed-key properties null.

### Completed Read Storage

Andrew approved nullable completed-read access on 2026-10-11 (`B: Nullable
region when unavailable`). A valid `ReadTransferKey` returns `?ByteRegion`:
null while that operation is active or no completed byte range is known,
otherwise a readonly view of exactly its completed prefix. Known zero-byte
completion, including EOF, yields a present empty region rather than null.
Readonly `getReadRegion(ReadTransferKey $key): ?ByteRegion throws IoError` is
the approved member spelling. An invalid or foreign key throws existing
`IoError`; it is not unavailability. Group access uses `IoOperation::Access`,
the appended payload-free `IoTarget::TransferGroup`, and
`IoErrorReason::InvalidInput`, with no path, system code or byte progress.
Existing target tags are preserved; exhaustive matches need the new arm.

This makes normal availability a nullable value, not a checked failure, at
the cost of requiring null handling. Access performs no I/O, allocation or
completion observation. It preserves the group's loans and cannot expose
active storage. Resubmission makes the previous completed prefix unavailable
until the new transfer retires. A borrowed region prevents conflicting group
mutation and cannot outlive its owner. An original failed transfer's known
byte progress may supply its prefix without taking or recreating its Error.
This ruling does not claim public group execution is delivered.

### Transfer Resubmission Names

Andrew approved `resubmitRead` / `resubmitWrite` on 2026-10-11. The names
describe reuse of the original directional operation, not creation of a new
operation or key. `begin` submits the initial transfers. A subsequent
`$group->resubmitRead($readKey, 0, 4096)` reuses a range of that operation's
retained buffer, subject to the existing range, retirement and loan checks.
Read and write keys remain distinct; resubmission preserves their identities
and invalidates the previous completed prefix. These names introduce no
`submitRead` / `submitWrite` aliases or alternative synchronous stream methods.
Andrew approved `IoOperation::Configure` / `IoTarget::TransferGroup` for
invalid resubmission keys or ranges. These failures use existing `IoError`
and `IoErrorReason::InvalidInput`, with no path, system code or byte progress.
The failed validation does not submit work or consume a stored transfer Error;
actual transfer failures still propagate unchanged at `getNextCompletion`.
Public execution and the remaining group control declarations remain Stage 36a work.

### Pending Adapter Capabilities

The explicit pending-transfer direction above uses these nominal
`Doria\Std\Io` capabilities. A stream may offer either or both; ordinary
synchronous transfer capabilities remain separate from pending execution.

```doria
interface PendingReadableStream extends ReadableStream, ReadinessStream {
    writable function prepareRead(writable ReadRequest $request, writable WritableByteRegion $destination): void throws IoError;
    writable function completeRead(ReadRequest $request, writable WritableByteRegion $destination): ?ReadOutcome throws IoError;
}
interface PendingWritableStream extends WritableStream, ReadinessStream {
    writable function prepareWrite(writable WriteRequest $request, ByteRegion $source): void throws IoError;
    writable function completeWrite(WriteRequest $request, ByteRegion $source): ?WriteOutcome throws IoError;
}
class ReadRequest {
    ?ReadOutcome $outcome { get throws IoError; }
    writable function complete(ReadOutcome $outcome): void;
}
class WriteRequest {
    ?WriteOutcome $outcome { get throws IoError; }
    writable function complete(WriteOutcome $outcome): void;
}
```

Requests are opaque borrowed views of group-owned reusable slots, not public
constructors or independent ownership claims. They cannot escape a preparation
or completion call, be consumed, or be retained in an adapter. Preparation
configures exactly one transfer through the underlying pending capability, or
calls `complete` to configure an immediate outcome from buffered/in-memory data.
It performs no native submission. All preparation, validation, and allocation
finish before any native request starts. A preparation error submits nothing
from the failed setup and preserves the original checked error.

An underlying file's preparation selects the supplied region. An adapter may
instead select reusable storage within its complete registered owner. Only
storage rooted in that owner or the group's retained buffers may survive the
preparation call; a local buffer cannot be lent to an outstanding transfer.
Completion receives the original group region, so a buffering or transforming
adapter can fill it after its selected native storage is no longer in use.
These rules do not introduce general borrowed fields or an ownership exemption.
Decision 0136 subsequently permits disjoint stored owned-property paths to be
borrowed independently across Doria, resolving the whole-owner conflict between
an adapter's file and reusable buffer. Parent lifetime and overlapping-storage
protection remain unchanged; this is not an adapter-only exception.

`outcome` is observable during completion, after native retirement or immediate
preparation. It preserves the original checked failure and optional progress.
Reading it is not a wait. `complete` is a preparation operation, not permission
to declare a live native operation retired. An explicit `WouldBlock` outcome
is no-progress data, distinct from an outstanding request. Every successful
preparation reaches the complete adapter's completion operation exactly once;
completion that wins cancellation retains actual progress. Both operations
must be nonblocking, including their helpers and cleanup, and may not reenter
the exclusively borrowed set/group. Ordinary synchronous methods are unchanged.

Andrew approved nullable pending outcomes: `outcome`, `completeRead`, and
`completeWrite` return `null` only when no transfer outcome exists. If cancellation
or a later setup failure prevents submission after successful preparation,
completion still runs exactly once to release prepared storage. Buffered or
in-memory preparation that already supplied an immediate outcome retains that
outcome even without native submission. Zero-byte success, `WouldBlock`, and
`EndOfStream` remain actual outcomes; checked failures still throw and preserve
their optional byte progress. Request `complete` requires a nonnullable actual
outcome. Ordinary `ReadOutcome` and `WriteOutcome` enums gain no `NotSubmitted`
case, and ordinary synchronous `read` and `write` remain nonnullable.

These declarations settle the pending-adapter shape, not a claim that public
group execution has landed. Remaining readiness, wait, and control names keep
their bounded spelling review below.

## Deferred Public Spellings

### Approved Generic Adapter Capability Strategy

On 2026-10-31 Andrew chose general explicitly conditional nominal conformance
and constrained member bodies, checked after generic substitution, for generic
stream adapters. Decision 0139 owns the accepted detailed language contract,
including Andrew's subsequent local-clause approval. The shared public compiler
path is implemented with focused macOS evidence; full acceptance and integration
remain pending.
A synchronous adapter preserves readiness when its source's static contract
supports it and
preserves the separate pending protocol when its source supports that protocol.
Owned and borrowed forms, complete adapter behavior and this record's binding
performance contract remain required. Separate capability-tier adapter families
are not the selected strategy. Buffering and text type/member names still reopen
through this bounded appendix; the strategy ruling does not settle those names.

### Borrowed Adapter Storage Review

On 2026-11-03 Andrew approved the ordinary-class constructor and lifetime
contract. Promoted `borrow T $source` retains readonly access and
`borrow writable T $source` retains exclusive writable access to a checked
caller place. The field is bound once; it cannot be replaced or taken.
Writable access through the field mutates the original source. The wrapper
remains an ordinary Move class and the source owner remains responsible for
destroying the source.

Moves and returns preserve every retained dependency, including through
generic payloads. Escape or erasure is permitted only when the destination
preserves those dependencies. Construction failure and cleanup release the
loans without destroying the source; the destructor may access the source
before release. Pending borrowed finish transfers the loan with the wrapper;
completion releases it.

This extends Decision 0134's readonly iterator promotion through the shared
ownership mechanism. Owned-field extraction and the remaining adapter
spellings retain their separate review gates.

### Owned-Field Extraction

Andrew approved "Consuming take, no destructor (Recommended)" on 2026-11-05.
Inside a `consuming function`, `take $this->field` moves an owned stored field
out of the consumed receiver. The extraction consumes the receiver: its
remaining fields are destroyed at that point and the object's destructor does
not run, so a body extracts at most one field and cannot use `$this` afterwards.
The class's own destructor does not run for the consumed object; the
extraction is the class author's explicit alternative ending (amended
2026-11-05 under Andrew's Stage 36a approval of recommended forks, decision
log entry 16). Extraction is rejected when an ancestor declares `__destruct`,
whose author did not consent to skipping it; for an open class, whose subclass
could add one; and for retained `borrow` fields, which the wrapper never owns
(E0780). The rule applies to every class, not only standard
adapters.

### Buffered Byte Adapters

Andrew approved "Paired owned/Borrowed classes (Recommended)" on 2026-11-05,
settling these `Doria\Std\Io` declarations. Capacity is required, so
buffering is bounded; the unread window is a borrowed region, not a copy.
Conditional conformance forwards readiness and pending tiers when the source
provides them.

```doria
class BufferedReader<S implements ReadableStream> implements ReadableStream {
    function __construct(take S $source, int $capacity) throws IoError;
    writable function read(writable WritableByteRegion $destination): ReadOutcome throws IoError;
    writable function fill(): ReadOutcome throws IoError;
    ByteRegion $buffered { borrowed get; }
    writable function consume(int $byteCount): void throws IoError;
    consuming function finish(): S;
}
class BufferedWriter<S implements WritableStream> implements WritableStream, FlushableStream {
    function __construct(take S $source, int $capacity) throws IoError;
    writable function write(ByteRegion $source): WriteOutcome throws IoError;
    writable function flush(): FlushOutcome throws IoError;
    consuming function finish(): FinishOutcome<S, BufferedWriter<S>> throws IoError;
}
```

`BorrowedBufferedReader<S>` and `BorrowedBufferedWriter<S>` take
`borrow writable S $source`. The borrowed reader has every owning-reader member
except `finish`: its source stays with the caller, and unread buffered bytes
are discarded when the adapter ends. The borrowed writer keeps `finish`, which
returns `BorrowedFinishOutcome<BorrowedBufferedWriter<S>>`. The owning reader's
finish discards unread bytes explicitly.

The adapters implement `ReadinessStream` and the pending tier of their
direction when the source does (Stage 36a decision log entry 44), and forward
the source's `readinessSource`. A reader's pending read completes immediately
from buffered bytes; once the buffer is empty, reads pass through to the
source with the caller's region in plan order, so a transfer waits on the
source's readiness only when no buffered byte remains. A read prepared after
the buffered bytes run out, while immediate reads are outstanding, completes
with `WouldBlock`. A writer carries one pending write at a time: free space
accepts it immediately, an empty buffer passes a write of at least the
capacity through, and a full buffer drains through one transfer before taking
the caller's bytes. A second concurrent write on one writer fails at
preparation with `IoError` (`Configure`, `TransferGroup`, `InvalidInput`).

Dropping either writer flushes what it can (Stage 36a decision log entry 42,
which gave destructors writable access to the dying object). Failures cannot
escape a destructor, so a caller that must observe them calls `flush` or
`finish` first, as with Rust's `BufWriter`. Bytes a non-blocking sink refuses
at drop are lost; `finish` is the way to keep them.

### Text Adapters

Settled 2026-11-06 under the Stage 36a standing approval (Stage 36a decision
log entry 33):

```doria
enum LineOutcome {
    case Line(string $text);
    case WouldBlock;
    case EndOfStream;
    case TimedOut;
    case TooLong;
}
class LineReader<S implements ReadableStream> {
    function __construct(take S $source, int $capacity);
    writable function readLine(): LineOutcome throws IoError, InvalidUtf8Error;
    consuming function finish(): S;
}
```

`BorrowedLineReader<S>` takes `borrow writable S $source` and has every member
except `finish`. A line ends at `\n`; one `\r` before it is removed. The last
line may end at the end of the stream without a terminator. `capacity` bounds a
line in bytes: a longer line reports `TooLong` and consumes nothing, so the
reader makes no further progress on it. Each line is validated once as it is
taken. Invalid input throws `InvalidUtf8Error` with source
`Utf8InputSource::Stream`. `validByteCount` is the stream offset of the first
invalid byte; `invalidByteCount` is the invalid sequence's length, or `null`
when the stream ends inside a sequence. A terminator inside a sequence makes it
invalid, not incomplete. `WouldBlock` and `TimedOut` pass through from the
source with no bytes lost. Text writing passes `$text->byteRegion` (Decision
0103 amendment, Stage 36a decision log entry 49) to any byte writer, which
borrows the string's UTF-8 bytes without copying.

### Child Process Scheduling

Andrew approved "Spelling now, build after the rest (Recommended)" on
2026-11-05. Child processes remain Stage 36a scope. Their declarations reopen
through this appendix after buffering, text adapters, file locks and the stage
vet, with a new estimate before the runtime work starts.

### Child Process Declarations

Settled 2026-11-06 under the Stage 36a standing approval (Stage 36a decision
log entry 34, which also records the estimate; entry 40 amends the pipe access
and the owner model found during implementation). `Doria\Std\Process`:

```doria
enum ChildCleanup { case Wait; case Terminate; case Detach; }
enum ExitStatus { case Exited(int $code); case Signaled(int $signal); }
enum WaitOutcome { case Exited(ExitStatus $status); case TimedOut; }
class Command {
    static function executable(string $path): Command;
    static function shell(string $script): Command;
    writable function argument(string $value): void;
    writable function environment(string $name, string $value): void;
    writable function removeEnvironment(string $name): void;
    writable function directory(string $path): void;
    function spawn(ChildCleanup $cleanup): Child throws IoError;
}
class Child {
    writable function getInput(): ChildInput;
    writable function getOutput(): ChildOutput;
    writable function getErrors(): ChildOutput;
    function getId(): int;
    writable function waitUntil(Doria\Std\Time\Deadline $deadline): WaitOutcome throws IoError;
    consuming function wait(): ExitStatus throws IoError;
    consuming function terminate(): ExitStatus throws IoError;
    consuming function detach(): void;
    consuming function collect(int $limit): CollectedOutput throws IoError;
}
class ChildInput implements WritableStream, PendingWritableStream {
    writable function closeInput(): void throws IoError;
}
class ChildOutput implements ReadableStream, PendingReadableStream {}
class CollectedOutput {
    ExitStatus $status { get; }
    Bytes $output { borrowed get; }
    Bytes $errors { borrowed get; }
    bool $truncated { get; }
}
```

`Command::executable` runs the program directly with the given arguments; a
name without a separator is found on the child's own `PATH`. `Command::shell`
runs the platform shell (`/bin/sh -c`, `cmd.exe /d /s /c`) on one script
string and takes no arguments: `spawn` rejects them as `InvalidInput`. They are
separate constructors of one value, which satisfies "separate typed
operations" without two builder types. The last environment edit of a name
wins. Invalid strings fail at `spawn`, not in the builder. The cleanup policy
has no default.

`Child` is a runtime-owned value holding the handle, the policy, the cached
status and the three pipes. The pipe getters are writable loans, like
`ReadinessSet::getWritableStream`; a `writable borrowed get` property would
lend them readonly (Decision 0135). Each getter lends only its own pipe, so
loans of different pipes of one child may be live together (Decision 0136's
sibling-path rule; Stage 36a decision log, "Child pipes in transfer groups"). The parent's end of each
pipe is nonblocking (Unix) or overlapped (Windows), so it joins a transfer
group directly; the child's end stays an ordinary blocking inheritable handle.
A plain `read` or `write` waits in the runtime and never reports `WouldBlock`. `waitUntil` keeps ownership on `TimedOut`.
`wait` closes the input, drains and discards both outputs concurrently, then
waits. `collect` does the same but keeps at most `$limit` bytes of each stream
(`truncated`). Both drain with the platform's readiness wait: `poll` on Unix, a
helper thread for the error output on Windows. `terminate` forces termination
(Unix `SIGKILL`, so `Signaled(9)`; Windows exit code 1), closes the pipes
without draining and waits. `detach` stops the policy. Destruction closes the
pipes first, then applies the policy once unless the child was resolved, so a
`Wait` policy cannot deadlock on a full pipe. `closeInput` half-closes stdin and
is idempotent; later transfers fail as `Closed`. `Signaled` appears only on
Unix. A detached Unix child that exits stays a zombie until this process exits.

Failures use `IoOperation::Spawn`, `Wait`, `Terminate`, `Read`, `Write` and
`Close` with `IoTarget::Process`, `ProcessInput`, `ProcessOutput` and
`ProcessErrors`, each carrying the program actually launched (the shell's path
for `Command::shell`).

### Remaining Declarations

The accepted operational contracts remain fixed. The remaining public
declarations reopen with their named owners before execution. Duration/Deadline
inline Copy ownership and Duration's signed numeric representation are settled
above; remaining Time operations stay in the bounded appendix.

| Surface                                                                                        | Owner and reopen trigger                                                                                                                                                                                                                                                                                    |
|------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Remaining blocking-mode, readiness, close/finish, text, file option/control, and process names | Reopen through this appendix before their Stage 36a implementation begins. Standard-stream accessors and view types, byte transfer capabilities, outcomes, seek, readonly inspection, synchronization and resizing are settled above. Other names preserve the accepted semantics and performance contract. |
| Byte-region and reusable-buffer shape                                                          | Settled in the Reusable Bytes And Borrowed Regions appendix above; compiler execution and provenance regressions remain Stage 36a work.                                                                                                                                                                     |
| `Path` representation and conversions                                                          | Reopen in `Doria\Std\Fs` design. Stage 36a preserves typed-path evolution.                                                                                                                                                                                                                                  |
| Public Time operations and cancellation types                                                  | Reopen remaining Time operations in the bounded appendix, and cancellation in concurrency design, no later than Stage 36a surface finalization.                                                                                                                                                             |
| Compression and encoding adapter names                                                         | Reopen after the Stage 36a foundation and before their v1 implementation. They remain typed streaming adapters without whole-input materialization by default.                                                                                                                                              |
| TLS configuration                                                                              | Reopen in the Stage 40a `Doria\Std\Net` design; Stage 44 matures the delivered contract. TLS wraps the shared duplex foundation.                                                                                                                                                                            |
| Progress reporting                                                                             | Reopen at the first operation or adapter that requires it. It must be typed and operation-local, never global, an untyped callback-code protocol, or a source of unbounded event allocation.                                                                                                                |

Candidate names in the audit remain illustrative and noncanonical until the owning trigger is reached. The owned file family, creation policies, append case, and consuming checked close above are settled; remaining control names are still deferred.

## Reopening Rules

A deferred spelling may be settled without reopening this architecture. Reopening a semantic rule requires a new accepted decision that names this record, explains the conflicting use case, preserves cross-domain unification, and updates the audit ledger and guards. An implementation inconvenience is not sufficient grounds to create a parallel model.

## Consequences

- Execution order belongs to the master plan's
  [approved execution sequence](../doria-end-to-end-plan.md#approved-execution-sequence-after-stage-36a),
  not this decision's historical review checkpoint.
- Stage 36a's accepted operational and performance contracts do not imply that every public declaration, ownership classification, or representation has been settled or delivered.
- PHP capabilities are either preserved through typed owners, deliberately rejected as dynamic/resource-shaped mechanisms, or deferred with an owner and trigger.
- Existing free-function intrinsics remain valid and share the future substrate; they are not evidence that Stage 36a is implemented.
- Explicit failure, ownership, reusable-storage, bounded-memory, allocation/copy, dispatch, startup, and cross-platform behavior are designed once and reused across later domains.

## Invalidated Elsewhere

- Cancellation token identity, compiler metadata and cleanup, LSP descriptions
  and nominal-identity tests, and the website's versioned guide and API reference
  must use `Doria\Std\Concurrency\CancellationToken`, not the I/O namespace.
  Source construction, token/status properties, `cancel`, and `CancellationError`
  now follow the approved concurrency-owned declarations above.
- The canonical operation/target inventory, exhaustive matches, shared runtime
  error transport and vocabulary, LSP hovers, and website versioned guide and
  API reference must include readiness infrastructure failures in `IoError`,
  without replacing original transfer errors or inventing byte progress.
- Completion-observation declarations, compiler MIR and runtime transport,
  structural regressions, LSP hovers, and website versioned guide and API
  reference must identify the finished operation without scanning or mandatory
  per-completion allocation, while retaining the group-wide borrowing scope.
  Fetching a failed completion raises its original checked error, with the
  failed operation's identity available to its catch.
- Pending request outcome getters and adapter completion signatures, compiler
  metadata and nullable transport, LSP signatures, and website versioned guide,
  API reference, and adapter examples must distinguish absence from an actual
  transfer outcome without changing ordinary synchronous outcome contracts.
- Concurrent adapter requirements, compiler capability/effect/lifetime checks,
  MIR and runtime dispatch, LSP hovers, and website versioned guide, API reference,
  and playground must distinguish explicit pending-transfer support from
  synchronous byte transfers and readiness-source exposure. Neither adapter
  bypass nor implicit suspension of synchronous method bodies is permitted.
- Concurrent-transfer documentation, compiler retained-loan analysis, MIR
  validation, runtime cleanup, tooling, and website versioned guide/API must
  preserve one group-wide borrowing scope. Completion permits processing and
  reuse through the group, not independent access through original bindings.
- Completion waits, shared runtime control, compiler/MIR transport, backend
  regressions, LSP signatures/hovers, and website versioned guide/API/playground
  must separate wait-only timeout observations from cancellation or a group
  deadline. A wait expiry must not cancel transfers or release their loans.
- PHP backend validation, emitted file/byte transport, runtime artifact delivery,
  parity fixtures, tooling, and website versioned guide/API/playground must use
  the approved shared-runtime deployment contract. Windows pipe documentation
  and future pending-operation signatures must preserve completion-owned buffer
  lifetimes and cancellation drainage, not pretend Unix readiness is portable.
- Canonical `IoOperation` inventories, exhaustive matches, error-message
  transport, tooling, and website versioned guide/API/playground must include
  `Close`. Writable-open examples must state a `FileCreation` choice rather than
  relying on an implicit creation or truncation policy.

- File-family documentation, compiler/tooling name inventories, and website
  versioned guide, API reference, and playground must use `ReadableFile`,
  `WritableFile`, and `DuplexFile` with the `File` companion factories above,
  not an illustrative `File<Capability>` family. This settles names, not a
  claim that public file execution or Stage 36a is complete.
- Persistent readiness documentation, tooling, and website versioned guide/API
  must not present registration as retaining a caller-owned stream borrow.
  Registration transfers ownership; removal returns it. Temporary waits and
  explicit borrowed adapters retain their separate lifetime contracts.
- The canonical `IoError` inventory, error construction/transport, diagnostic
  facts, LSP hovers, and website versioned guide/API must carry optional byte
  progress without replacing unavailable progress with zero.
- Cancellation documentation must not require the source to outlive its tokens
  or describe source destruction as an implicit cancellation request.
- Unix benchmark launch classification must not infer launch success or failure
  from exit 126/127, stderr, a resource wrapper, or a path preflight check.
- Planning text that called Andrew's stream review “next” or Stage 26 “blocked pending review” is stale.
- Text that called the stream architecture unauthored or its semantics pending review is stale. Transfer capability and outcome names are settled; only the remaining public spellings are deferred.
- Text that treats stream allocation, copying, dispatch, readiness reuse, backpressure, or sync/async cost as optional optimization work rather than Stage 36a acceptance criteria is stale.
- A stream design that requires a new buffer per read, copies each unwritten suffix, boxes every concrete stream, allocates common outcomes, rebuilds stable readiness registrations, busy-polls ordinary waits, or starts async infrastructure for synchronous programs is rejected.
- Any design that models Doria I/O as PHP resources, global contexts, dynamic wrappers/filters, string modes, mixed metadata bags, sentinel results, or one universal stream object is rejected.
- Any later async, network, process, or terminal API that creates a separate ownership, readiness, timeout, cancellation, or backpressure foundation is rejected.
