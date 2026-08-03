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

Five warmups and at least ten measured runs are the default controlled shape.
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
phase, runtime-artifact selection, linking, total duration, output size, program
counts, and generic callable/class specialization counts. An integrated phase,
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

## Slicing

Stage 26b Slice 1 establishes this measurement foundation, compiler reports,
initial paired diagnostics, provenance, and deterministic checks. Slice 2 records
the initial controlled compiler/program/runtime baseline matrix. Slice 3 adds the
accepted deterministic regression baselines and stage-completion workflow.
Stage 27 remains blocked until all Stage 26b slices complete.

## Explicit Non-Goals

- Optimizing compiler output or runtime implementations.
- Approving timing thresholds from shared CI machines.
- Beginning Stage 27 or later performance stages.
- Treating the interpreter as a native performance competitor.
- Claiming causation from a timing delta without a confirming experiment.

## Consequences

- The pre-existing comparative source layouts and historical evidence remain;
  new evidence gains exact correctness authority and reproducible provenance.
- Stage 26b is in progress with Slice 1 complete and Slice 2 next.
- Stage 27 remains blocked until Stage 26b completes.
- The compiler pins the benchmark repository revision used by its coordinated
  checks in `benchmarks-revision.json` without requiring network access. The
  optional sibling checkout is validated when present and absence remains valid
  in isolated compiler CI.

## Invalidated Elsewhere

- A future `baton bench` must orchestrate the same manifest/report contracts and
  must not create a second benchmark engine.
- Public or website performance material may consume curated target-state
  evidence only; it must not expose development-stage status or broad claims.
