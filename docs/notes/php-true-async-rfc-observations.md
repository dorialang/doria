# PHP "True Async" RFC — observations for Stage 37 and the PHP bridge

> Documentation role: working note. This is **external input**, not a decision and
> not authority. Nothing here binds Doria's design. It exists so the Stage 37
> async design gate and the PHP-bridge work start with a real-world data point
> instead of assumptions.
>
> **Status caveat — read first.** The RFC (`wiki.php.net/rfc/true_async`, Draft
> v1.7, December 2025) is **not adopted**. Its stated vote window has closed with
> no recorded result visible on the page, and its structured-concurrency half is
> split into a separate RFC that is also unsettled. PHP's long-term concurrency
> strategy is therefore unknown. Do not cite this note as "PHP does X" or design
> against it as a fixed target. Treat every item below as *a plausible shape PHP
> concurrency might take*, useful for pressure-testing our own choices.

## What the RFC proposes (factual summary)

- **Cooperative stackful coroutines, single-threaded**, with an internal scheduler
  and a reactor for I/O and timers. Concurrency, not parallelism.
- `spawn()`, `await()`, `suspend()`, `current_coroutine()`, `shutdown()`,
  `finally()` — **functions in an `Async` namespace, not keywords**.
- **No function coloring.** Existing synchronous code (`file_get_contents`,
  `mysqli_query`) keeps its signature and runs unchanged inside a coroutine;
  actual non-blocking behavior arrives per-extension via later RFCs.
- **Cancellation-centric**: `Coroutine::cancel()` resumes the coroutine with a
  `Cancellation` throwable; cleanup runs through `finally`. "Cancellable by
  design" is stated as a principle. Deadlock detection raises `DeadlockError`.
- **Silent on data synchronization**: no channels, mutexes, or atomics. Shared
  mutable state is explicitly the developer's responsibility.
- **Open**: memory isolation of statics/globals across coroutines is unresolved.
- Backward compatibility: a new root-namespace `Cancellation` throwable.

## The structural contrast (why this mostly validates our sequencing)

PHP is adding concurrency **without** a memory model — coherent for PHP, because
single-threadedness means no *parallel* races and its extension/global state was
never thread-safe. Doria is building the memory model first (ownership Stage 19,
borrow checking Stage 21) so that data-race freedom **falls out** at the spawn
boundary via auto-derived `Sendable`/`Shareable` (D11, Stage 39).

Two consequences worth stating plainly:

- Our Stage 38 (single-threaded executor) lands roughly where this RFC lands.
  Our Stage 39 (multithreaded, compile-time data-race rejection) goes somewhere
  PHP structurally cannot follow. That is the payoff of doing ownership first.
- The RFC leaves statics-across-coroutines unresolved. We already closed that
  hole: writable statics are per-process globals and are rejected in
  `Sendable`/`Shareable`-checked contexts (§6.5).

---

## Part A — inputs for the Stage 37 async design

Stage 37 is the plan's one deliberate design gate (designer sign-off required).
These are questions the RFC sharpens; each is a stop-and-ask, not a proposal.

### A1. Cancellation must travel the checked-error path, not the panic path
The RFC models cancellation as a throwable that resumes the coroutine so cleanup
runs. Our equivalent is stronger — RAII `__destruct` through drop elaboration is
automatic rather than user-written `finally`. **But the constraint that falls out
is load-bearing:** abort-only panic runs *no* cleanup (record 0081), so a
cancellation delivered as a panic would leak every resource the task owns.
Cancellation must therefore be modeled on the `throws` propagation path, where
§5 already guarantees `__destruct` at every scope boundary. Stage 37 should state
this explicitly rather than leaving it implied.

### A2. Record *why* we chose explicit coloring
No-coloring is the RFC's biggest ergonomic win and the strongest argument against
`async function`/`await`, given our PHP-familiarity goal. Our justification
exists but is not written down: **"sync programs pay zero async cost" (Stage 38)
is only achievable because the split exists** — an uncoloured model must be able
to suspend anywhere, so the runtime is always present. Record that rationale, or
the choice reads as unexamined Rust inheritance.

### A3. Visible suspend points are a design property, not an accident
The RFC accepts that any call might suspend. That is the same class of
invisibility as implicit conversion, which D20 rejects everywhere else. Stage 37
should state that suspend points are visible **by design**, consistent with the
explicit-typing discipline — so the tradeoff is a recorded choice.

### A4. Deadlock and starvation have no story yet
The RFC ships deadlock detection (`DeadlockError`). Stage 37's scope lists
executor, task groups, cancellation, and `Shareable` rules — but not deadlock or
starvation behavior. Question: does Doria detect deadlock (all tasks blocked, no
progress), and is that a panic, a checked error, or a diagnostic-only dev-mode
aid? Add it as a design case.

### A5. Do not split structured concurrency off
The RFC ships `spawn` in the core and defers scopes to a separate, unsettled RFC
— unstructured spawn first, structure retrofitted. D11 puts task groups in from
the start, and §5 already guarantees no orphan tasks when the root task completes
with an error. This is a cautionary data point worth citing when "ship spawn now,
add scopes later" becomes tempting.

---

## Part B — PHP-bridge integration gaps this surfaces

This is the "did we miss anything" half. Our bridge invariant (§10.3) is about
**threads**: a PHP runtime context and its values belong to a designated thread;
`Sendable`/`Shareable` are never permission to move PHP-runtime-affined values.
A coroutine-capable PHP is still single-threaded, so that invariant holds — but
coroutines break a *different* assumption the bridge silently relies on: **that a
PHP request is linear**. Each item below is a gap, not a decision.

### B1. A Doria export blocks the caller's event loop
Our `baton build --php-lib` product positions Doria as the native power backend
(image resizing, hot loops). Under coroutines, a call into Doria is **opaque to
PHP's scheduler**: while native code runs, no other coroutine progresses — the
whole reactor stalls, not just the calling coroutine. Our invariant covers
threads and says nothing about event loops.
*Questions:* Is a Doria export documented as loop-blocking? Is there ever a way
for a long-running export to yield back to the caller's scheduler — and does that
require the export itself to be async? Or does the bridge stay "call fast native
functions" with long work explicitly out of scope?

### B2. Cancellation cannot cross the boundary
If a PHP coroutine is cancelled while inside a Doria call, native code already
executing cannot be interrupted cooperatively. **A Doria export is a
cancellation-opaque region.** That is a contract the bridge should state
explicitly, and it interacts with A1: PHP-side cancellation semantics and
Doria-side cancellation semantics meet at a boundary that supports neither.

### B3. Interleaving breaks the linear-request assumption behind handles
This is the one I think we genuinely missed. `#[PHPExport]` classes are marshaled
as opaque handles rooted as `Shared<T>`, with the generated PHP stub releasing in
its `__destruct`. That model was designed against PHP's linear request lifecycle.
With coroutines, **one PHP request can interleave**: coroutine A takes a handle
and suspends; coroutine B mutates through another handle to the same Doria
object; A resumes with stale assumptions. There is no thread involved, so no
`Sendable` check fires — but the aliasing hazard is real.
*Questions:* Do exported mutable handles need to route through `SharedMut<T>` so
the runtime overlapping-access check catches interleaved mutation? Or does the
bridge declare exported handles single-owner-per-coroutine? Either way, the
handle model needs re-examining against a non-linear caller.

### B4. Two runtimes, one process
If a Doria export ever wants internal async I/O, it would need its own executor
running inside a PHP coroutine call — nested schedulers with no relationship.
Stage 38's "executor started only when `main` is async" helps (a library export
has no executor by default), but the nested case should be explicitly ruled in or
out rather than discovered later.

### B5. Do not depend on PHP-side static state
The RFC leaves per-coroutine isolation of statics/globals unresolved. The bridge
should not rely on PHP-side static or global state surviving across a call, since
its semantics under coroutines are undefined. Cheap to state now.

---

## What this note does not do

It does not decide anything, does not amend the plan, and does not assume PHP
ships this RFC or any successor. If PHP's concurrency direction settles later,
re-read Part B — B1 and B3 are the items whose answers would change most with the
details. Items A1 and A5 stand on Doria's own model regardless of what PHP does.

## Invalidated elsewhere

None.
