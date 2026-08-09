# Decision 0112: Performance Baseline, Provenance, And Regression Measurement

- **Status:** Accepted
- **Date:** 2026-08-03
- **Owners:** Doria language, compiler, runtime, and benchmark design
- **Scope:** Repository-owned performance evidence and the rules for comparing it over time

## Context

Earlier cross-language tables were useful exploratory observations, but their
three grouped samples, single warmup, incomplete build provenance, and dependence
on the first runnable target's output could not support durable baselines or
causal claims. Stage 26b needs one measurement foundation before later language
and runtime stages make the performance surface larger.

## Decision

The sibling `dorialang/benchmarks` repository owns one manifest-driven benchmark
engine. It has two measurement tiers and three evidence tracks.

The **diagnostic tier** compares a Doria feature with a matched Doria control.
The **comparative tier** runs equivalent workloads across Doria and peer
toolchains. The evidence tracks are compiler performance, generated-program
performance, and runtime-subsystem performance. A future `baton bench` may
orchestrate this engine but does not replace or fork it.

## Correctness Contract

Every case names committed expected stdout bytes, stderr bytes, and process
status, or a committed verifier with equivalent authority. Correctness is checked
before warmup or timing. Whitespace is exact. No runnable target becomes an
implicit oracle, and a mismatch invalidates that target's measurement.

## Manifest And Report Contract

Cases live in a strict, versioned manifest. Unknown fields, duplicate identities,
invalid shapes, and missing files are rejected. Generated reports use a strict
versioned JSON schema, retain all raw samples in integer nanoseconds, and derive
min, max, mean, median, p95, and population standard deviation from those samples.

Five warmups and at least fifteen measured runs are the default controlled shape.
Targets run round-robin with a rotating first target. Quick smoke reports are
always marked ineligible for baselines. Missing platform metrics are recorded as
unavailable with a reason; they are never invented as zero.

## Provenance Contract

A report records the benchmark repository revision; Doria repository and
compiler revisions, toolchain version, path, backend, profile, and driver; Baton
identity when used; exact build and run commands; peer tool versions; host OS,
architecture, and CPU-affinity information; inputs; and runner identity.

`DORIA_REPO`, `DORIAC`, and `BATON` provide explicit selection. The runner accepts
only a compiled `doriac`, rejects the Cargo source launcher, and rejects a
compiler/repository commit mismatch unless the operator explicitly opts into that
non-baseline condition. Baton and direct-compiler modes are distinct and never
silently fall back to one another. Cranelift and LLVM retain distinct target
identities.

## Compiler Evidence

`doriac compile --performance-report <file>` is the opt-in compiler measurement
surface. The report records source loading, parsing, semantic analysis, HIR and
MIR lowering, MIR validation, the selected Cranelift or LLVM code-generation
phase, runtime-artifact selection, linking, total duration, output size,
source/AST/MIR structure, runtime artifact size when known, program counts, and
generic callable/class specialization counts. The structural counts come from
the AST and MIR already produced by the opt-in compile; no counting pass is
added. An integrated phase,
such as current borrow checking within semantic analysis, is explicitly marked
unavailable as a separate duration rather than double-counted.

The report destination is explicit and written atomically. Write failure is a
structured Title Case compiler diagnostic. Without the flag, the compiler does
not create report state, run an extra pass, serialize JSON, write a file, or emit
extra output.

## Initial Diagnostic Pairs

- `call_overhead`: a leaf call versus an inline control.
- `checked_arith`: checked integer arithmetic versus the closest currently
  expressible `float64` control. This is a documented proxy because Doria has no
  unchecked integer mode.
- `element_access`: typed-array reads/writes versus a local accumulator.

Each pair evaluates to identical observable output. Its delta is evidence about
the complete pair, not automatic proof that one compiler mechanism caused it.

## Regression Policy

Shared CI enforces deterministic structure: valid manifests and schemas, exact
correctness fixtures, provenance and pin rules, repository hygiene, and stable
compiler-report fields. Wall-clock thresholds run only on controlled runners.
Unfavorable results remain visible. A performance claim names the workload,
revisions, commands, machine, sample shape, and uncertainty; one benchmark never
supports general superiority.

Later stages that change runtime representation, allocation, ownership, dispatch,
code generation, control flow, I/O, concurrency, or FFI must update relevant
cases and record their performance impact under the master-plan rule.

## Native Runtime Acceptance Standard

Doria aims to match or beat C, C++, and Rust on comparable native workloads.
For each acceptance-bearing workload, the native peer is the fastest valid,
semantically eligible C, C++, or Rust result. PHP is adoption evidence and is
never used as the native acceptance peer.

The maximum passing runtime ratio is exact:

```text
Doria Median / Fastest Valid Native Peer Median <= 1.30
```

A ratio at or below `1.30` is **Pass**. A ratio greater than `1.30` is
**Fail**. Incorrect, incomparable, startup-dominated, noisy, unstable, or
incompletely attributed evidence is **Inconclusive**, never Pass. An unfinished
compiler, a stronger semantic guarantee, or work scheduled to a later stage may
explain a failure but does not change it into a pass. Semantic differences may
require separate semantics-matched and idiomatic product-cost comparisons; they
do not create a qualified pass.

Only Andrew, as Doria's language designer, may change the `1.30` boundary.

This cross-language status is separate from a Doria self-regression threshold.
A Doria build may pass against its own accepted baseline while the baseline
remains a cross-language Fail. Compile time, link time, startup, peak memory,
and artifact size remain separate dimensions and do not offset a runtime Fail.

## Slicing

Stage 26b Slice 1 establishes this measurement foundation, compiler reports,
initial paired diagnostics, provenance, and deterministic checks. Slice 2 records
the initial compiler/program/runtime matrix, process-resource adapters, optional
separate Callgrind and DHAT executions, compiler scaling generators, candidate
evidence, and an accepted exact structural baseline. Slice 3 adds peer sources
for the new runtime cases, controlled timing thresholds, and the
stage-completion workflow.
Stage 27 remains blocked until all Stage 26b slices complete.

Slice 3 is split. Part 1 delivers the peer matrix, peer fairness and
semantic-equivalence records, controlled candidate measurement, and a timing
threshold proposal, together with the controlled-runner and baseline-promotion
workflows. Part 1 does not accept a threshold or promote a baseline. The timing
threshold review is a separate step that consumes Part 1's proposal.

## Explicit Non-Goals

- Optimizing compiler output or runtime implementations.
- Approving timing thresholds from shared CI machines.
- Beginning Stage 27 or later performance stages.
- Treating the interpreter as a native performance competitor.
- Claiming causation from a timing delta without a confirming experiment.

## Consequences

- The pre-existing comparative source layouts and historical evidence remain;
  new evidence gains exact correctness authority and reproducible provenance.
- Stage 26b is in progress with Slices 1 and 2 complete and Slice 3 in progress.
  Slice 3's non-timing closure work and native acceptance policy are in place.
  Runtime selection is reproducible, but no eligible controlled Linux session
  exists yet, so no timing baseline is promoted and Slice 3 remains blocked.
- Stage 27 remains blocked until Stage 26b completes.
- The compiler pins the benchmark repository revision used by its coordinated
  checks in `benchmarks-revision.json` without requiring network access. The
  optional sibling checkout is validated when present and absence remains valid
  in isolated compiler CI.

## Slice 2 Structural Contract

Report schema version 2 is required because the schema is strict and the new
matrix, metric, resource, compiler-structure, artifact, and profiler fields
would be rejected as unknown by version 1. Version 1 remains readable; unknown
future versions are rejected.

The compiler performance report remains version 1: its `metrics` object is an
extensible opt-in compiler contract, so adding derived counters is additive for
consumers that follow the record's unknown-metric rule. The benchmark report is
different because its committed JSON Schema rejects unknown fields.

Numeric metrics are either available with a numeric value, canonical unit,
source, exactness, and baseline eligibility, or unavailable with a reason.
Unavailable never means zero. Process accounting uses GNU time on Linux and BSD
time on macOS when present; Windows records an explicit unavailable reason until
a repository-supported adapter exists. Callgrind and DHAT run separately from
timed samples and remove temporary output afterward.

The accepted structural baseline contains exact output hashes and status plus
stable compiler/MIR/specialization counts for the minimum Slice 2
matrix. It contains no wall-time, RSS, Callgrind, DHAT, or cross-platform size
threshold. Capture defaults to candidate; acceptance is explicit. Comparison
results are Pass, Fail, Not Comparable, or Unavailable.

## Slice 3 Peer Fairness Contract

Every comparative case that ranks Doria against another language carries a peer
equivalence record, enforced when the manifest loads rather than left to review.
The record states the workload, the observable result, any optimisation
sensitivity, the Doria construct, and one entry per peer naming that peer's
construct and its known semantic differences. Each difference declares whether it
favours Doria, the peer, or neither, so an advantage cannot be recorded without
saying who holds it. A declared peer target must have a committed source, that
source must appear in `inputFiles`, and the target must have a runner entry.

Peers use each language's idiomatic construct for the same requirement. Where
Doria is stricter than idiomatic peer code, the peer is not handicapped to match
and the resulting advantage is recorded against the peer. Where a case exists to
measure a discipline, the discipline is preserved in every peer rather than
removed to improve the peer's number. Neither rule may be relaxed to make a
comparison more favourable.

A case whose workload an optimiser can fold, hoist, or eliminate is flagged, so a
result that mostly measures optimisation is not read as a runtime comparison.

## Slice 3 Threshold Contract

A timing threshold is proposed only for a case and target whose measured median
clearly exceeds that target's process-startup floor, established by the `startup`
case. A pair dominated by process startup carries no threshold at any tolerance,
because a tolerance loose enough to survive the measurement noise cannot detect a
regression. The remedy for such a pair is a larger workload, not a looser bound.

A proposal is not an acceptance. A threshold document records `status:
proposal` and `accepted: false`, states the limitations of the sessions it came
from, and is reviewed separately before any threshold binds. Evidence from a
session that did not achieve its controls remains valid candidate evidence and is
published with its reasons rather than withheld or presented as eligible.

Controlled sessions run on a curated runner through a manual workflow. Baseline
promotion is a separate manual, confirmation-gated workflow that accepts the
deterministic structural baseline only and cannot install a timing threshold.
Shared CI runs structural checks and peer correctness qualification, never
wall-clock gates.

Controlled-runner controls are platform-specific. Where a platform cannot supply
a control -- affinity verification, CPU model, memory size, or power mode -- the
session records the control as unachieved with a reason and is not timing
baseline eligible. The harness never infers that a control was achieved.

Recorded provenance covers every artifact that executes, not only the compiler
that produced it. The linked runtime archive is part of the measured program, so
its identity belongs in the report alongside the compiler revision, commands,
and driver. Two builds of one compiler revision produced materially different
runtime archives during Slice 3 and moved a case median by more than four times;
because the archive was unrecorded, the substitution left no trace and a finding
was published that later had to be withdrawn. Timing evidence is not comparable
across compiler rebuilds unless the runtime archive is identified, and a result
that cannot be attributed to a specific runtime artifact is an observation about
an unknown program.

Runtime artifact selection must not depend on whatever happens to sit in a
developer's build directory. A profile that prefers a workspace archive over the
runtime the compiler bundled can link a runtime from a different revision than
the compiler performing code generation, which silently violates the
single-selected-revision rule this decision already requires.

The compiler now builds and bundles the runtime deterministically from the
runtime manifest, lockfile, target, profile, and revision. Reports identify the
compiler binary and runtime archive by SHA-256 and reject cross-session identity
drift. This closes the earlier runtime-selection defect; it does not substitute
for controlled Linux evidence.

## Invalidated Elsewhere

- A future `baton bench` must orchestrate the same manifest/report contracts and
  must not create a second benchmark engine.
- Public or website performance material may consume curated target-state
  evidence only; it must not expose development-stage status or broad claims.
