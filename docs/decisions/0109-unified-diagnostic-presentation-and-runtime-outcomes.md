# Decision 0109: Unified Diagnostic Presentation and Runtime Outcomes

Status: Accepted

## Context

Decision 0108 established one compiler-owned structured diagnostic model for
language, backend, external-tool, and internal-compiler findings. Runtime panic
still bypassed that model: the interpreter, native runtime, and PHP backend each
formatted message prose and function-only stack frames directly. That split
could not provide precise source attribution and would have encouraged checked
errors to introduce a third reporting system.

## Decision

The compiler-owned `Diagnostic` model is Doria's sole public diagnostic
representation. Compile-time findings and runtime outcomes share its source
identity, byte-span labels, catalogue infrastructure, human visual grammar,
concise presentation, versioned JSON transport, and tooling projections.

Runtime-specific facts extend `Diagnostic`; they do not form a sibling public
model. The extension carries process status, termination behavior, an origin,
typed bounded facts, and an execution path. A panic is
`DiagnosticKind::RuntimePanic`. A compact native record and source-aware shadow
frame are implementation-private ABI transports which the compiler host or
standalone runtime projects into the same facts. They are not public diagnostic
models or catalogues.

The global human grammar uses:

```text
<Kind>[<Code>]: <Title>

Where
<project-relative path> · line <line> · <Doria context>

<minimal source preview>
<marker>
<label>

Related
<secondary labelled sources>

Why
<explanation>

Note
<supporting fact>

Help
<action>

Suggested Fix
<applicability and edits>

Call Path
<Doria frames>

<compilation summary or runtime process status>
```

Only useful sections appear. Source previews use a minimal line-number gutter,
not rustc arrow or vertical-pipe scaffolding. `Call Path` replaces the former
`Stack Trace` runtime heading. Renderers never expose Rust backtraces, native
addresses, backend symbols, generated PHP frames, compiler temporary paths, or
absolute developer paths.

The central catalogue owns compile-time codes and runtime-outcome codes.
Existing `P00xx` parser codes remain legacy syntax-code identities; runtime
panic identities occupy the `P1xxx` range and are divided by domain. `P1203`
permanently means `String Padding Text Cannot Be Empty`. Catalogue data owns
titles, labels, explanations, status, domain, documentation, and dynamic-fact
schemas. A generated compact runtime table is a derivative, never authority.

Every panic-capable operation retains a reason-specific compiler-generated
panic-site identity through validated MIR. The site identifies the catalogue
entry, source, operation and primary spans, Doria function, label/template, and
fact schema. Native code uses an explicitly versioned source-aware ABI; it does
not reinterpret `DrStackFrameV1` as a larger layout. Interpreter, Cranelift,
LLVM, PHP compatibility output, standalone executables, and `doriac run`
preserve the same structured facts. `doriac run` receives native outcome data
through a private versioned channel distinct from stdout and stderr and never
parses rendered prose.

Panic semantics do not change. Panic is fatal, non-catchable, non-unwinding,
runs no cleanup or destructors, and exits with status 101.

## Checked-error compatibility

Decision 0119 implements checked-error grammar and static checking in Stage 29
Slice 1. Slice 2 execution and Slice 3 runtime reporting remain bound to this
architecture:

- compile-time checked-error violations are ordinary language diagnostics;
- a caught checked error is ordinary program control flow and emits no
  automatic diagnostic;
- an unhandled checked error is a runtime outcome, not a panic;
- propagation runs the cleanup and destructors required by checked-error
  semantics before termination;
- the accepted unhandled status remains 70 unless separately amended;
- the same diagnostic model, source identity, labels, renderer, JSON envelope,
  path normalization, tooling component, and versioned native transport family
  are reused.

The old conceptual lowercase `error: <Class>: <message>` line is not a separate
permanent rendering contract. Decision 0119 settles the shared presentation as
`Error[R1000]: Unhandled <ConcreteType>` with status 70, first-throw origin, no
default propagation path or automatic cause chain, and safe human rendering of
untrusted message text. Slice 3 implements that target through this decision's
existing diagnostic and runtime-outcome model.

## Consequences

- No public or compiler-facing panic report, checked-error report, runtime
  envelope, panic JSON schema, or panic-only catalogue may exist beside
  `Diagnostic`.
- No user-facing renderer or tool may parse another renderer's prose.
- Human, concise, JSON, LSP, Playground, interpreter, Cranelift, LLVM, and PHP
  compatibility paths consume the same structured facts.
- Standalone rendering is mechanically generated from, or snapshot-verified
  against, the compiler authority and has a bounded allocation-free emergency
  fallback.
- The previous function-only `DrStackFrameV1` and message-based built-in panic
  path are retired from production use rather than silently reinterpreted.
- Checked `throw`/`throws`, `try`, `catch`, Error conformance, and effect checking
  are implemented in Stage 29 Slice 1. Propagation and the runtime Error
  representation are implemented in Slice 2; R1000 process-outcome execution
  remains deferred to Slice 3 under Decision 0119.

## Invalidated elsewhere

- Decision 0035's freedom to choose a separate checked-error presentation.
- Decision 0040's function-only `Stack Trace` output and statement that source
  locations are unnecessary.
- Decision 0044's V1 frame and message-only panic ABI as the active contract.
- Decision 0108's claim that panic remains a byte-identical parallel envelope.
- Runtime fixtures, generated PHP, editor/Playground schema consumers, and
  documentation that treat `Stack Trace` or rendered panic prose as canonical.
