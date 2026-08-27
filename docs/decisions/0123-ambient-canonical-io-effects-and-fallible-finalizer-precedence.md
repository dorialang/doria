# Decision 0123: Ambient Canonical I/O Effects And Fallible Finalizer Precedence

- **Status:** Accepted
- **Accepted:** 2026-08-26
- **Date:** 2026-08-26
- **Implementation Status:** Implemented By The Pre-Stage-32 Corrective Beat
- **Amends:** Decisions 0074, 0116, 0119, and 0121
- **Preserves:** Decisions 0109 and 0122

## Context

Doria's checked-error model correctly preserved exact I/O failures at runtime,
but required routine I/O to appear in every source `throws` contract. The same
implementation also rejected any checked Error leaving a source `finally` block.
Both restrictions added ceremony without improving correctness. They also made
ordinary application cleanup difficult even though the shared structured-exit
model already knew how to route returns, loop transfers, and Errors through
finalizer regions.

This decision changes source obligations while preserving exact checked runtime
transport. It also completes the existing finalizer-region model instead of
introducing unwinding, runtime finalizer objects, or backend-specific behavior.

## Decision

### Exact ambient I/O identities

Exactly these compiler-known Error identities are ambient:

```text
Doria\Std\Io\IoError
Doria\Std\Io\InvalidUtf8Error
```

Classification uses normalized semantic identity. A source alias resolves to
the same identity. A user class named `IoError`, a subclass, another Error, and
the open `Error` interface are not ambient.

Ambient means that authors need not catch or declare the Error. It does not mean
infallible, ignored, swallowed, or fatal. Ambient Errors remain operation
precise, catchable, and present in HIR, MIR, cleanup routing, backend ABI
selection, and the selected program boundary. An unhandled ambient Error is
reported as R1000 with status 70. The closed-standard-pipe status-0 rule is
unchanged.

The semantic effect profile is split into:

```text
required = nonambient checked Errors that impose catch-or-declare obligations
ambient  = canonical I/O Errors that propagate automatically
complete = required union ambient, used for executable transport
```

Source order and source operation origins are preserved within the complete
profile. Matching catches subtract both required and ambient effects normally.

### Source compatibility and function identity

Canonical I/O builtins and `echo` omit ambient Errors from their source-facing
`throws` spelling. Explicit ambient `throws` entries remain accepted and are
preserved as authored syntax. They are normalized into the ambient profile and
do not produce a warning or change structural function identity. A broad
`throws Error` remains required and is never normalized away.

Required nonambient effects remain an exact structural function-type axis.
Ambient-only differences do not affect assignment, generic specialization,
closure contextual typing, callable properties, List callbacks, or exact
function tests through `mixed`.

Direct ambient-free functions may retain the ordinary ABI. A known direct
function with a complete ambient profile uses the existing checked status and
Error out-slot transport. Structural function values use one compiler-private,
ambient-capable indirect convention. This convention adds no per-call heap
allocation and exposes no runtime effect reflection.

### Fallible source finalizers

A source `finally` block may propagate a checked Error. Finalizer effects are
analyzed independently and joined after protected-body catch selection. A catch
attached to a `try` therefore cannot catch an Error raised later by that same
try's finalizer. A nested catch inside the finalizer or an outer catch may handle
it. Required nonambient effects remain subject to the enclosing callable's
ordinary contract; ambient I/O propagates without source boilerplate.

If a finalizer succeeds, the pending outcome resumes unchanged. If it produces
a checked Error, that Error replaces pending normal completion, function return,
`when` result, `break`, `continue`, or an earlier checked Error. The replacement
Error is acquired first, retains its own first-throw origin, and is not dropped
as a finalizer local. Any superseded owned return, `when` result, or Error is
destroyed exactly once before propagation. The old destination is never resumed.

Nested finalizers execute inner-to-outer. An inner Error continues through a
successful outer finalizer. A failing outer finalizer supersedes the currently
pending inner Error and destroys it exactly once. Doria creates no automatic
cause, suppressed-error list, aggregate Error, runtime finalizer object, cleanup
stack, native unwind edge, or host-exception authority.

Fatal panic remains non-catchable and cleanup-free with status 101. Explicit
escaping `return`, `break`, `continue`, and `when` transfers from a finalizer
remain rejected. Destructors still may not leak checked Errors. Static and
constant initializers remain nonthrowing.

### Shared compiler and backend obligations

HIR preserves authored syntax plus required, ambient, and complete profiles.
MIR preserves those profiles and explicitly records finalizer pending outcomes,
replacement Error acquisition, superseded-payload destruction, and outer-region
continuation. Shared MIR validation rejects profile overlap, noncanonical
ambient descriptors, incomplete unions, finalizer re-entry, missing or duplicate
payload destruction, replacement loss, and old-destination resumption.

The interpreter, Cranelift, LLVM, and PHP compatibility backend consume this
validated model. Cranelift and LLVM lower direct CFG and function-scoped scratch
storage; no native unwinding or dynamic cleanup stack is introduced. Generated
PHP owns checked Error carriers explicitly so host garbage collection and host
`finally` behavior do not define Doria precedence or destruction.

Language-server diagnostics remain compiler-owned. Hovers omit ambient-only
effects from structural `throws` identity while documenting canonical ambient
I/O behavior separately. Tooling does not suggest adding or removing ambient
`throws`, and does not synthesize nested catches.

## Consequences

Ordinary I/O helpers, constructors, closures, callbacks, catch bodies, and
source finalizers no longer require canonical I/O boilerplate. Programs can
still recover explicitly, and failures remain exact across package graphs and
all executable backends. E0632 has no active source route and remains catalogued
as Historical And Reserved.

Ambient-free direct calls retain their compact path. Ambient-capable direct and
indirect calls reuse the established checked transport with no per-call effect
allocation. Finalizer replacement uses structured CFG and function-scoped
locals, not runtime finalizer allocation.

## Invalidated Elsewhere

- Active wording that requires canonical I/O Errors in every source `throws`
  clause.
- Builtin signatures that print ambient I/O as required `throws` entries.
- Active wording or tests saying no checked Error may escape `finally`.
- E0632 diagnostics or help directing authors to catch every finalizer Error
  locally.
- Function-type identity that includes ambient-only differences.
- ABI selection based only on authored `throws` syntax.
- Backend finalizer paths that resume an outcome superseded by a finalizer Error.
