# 0071 Stage 12 general control flow and dataflow

Status: Accepted

## Decision

Stage 12 replaces final-statement return checks with an explicit source control-flow graph and one reusable deterministic worklist/dataflow framework.

The CFG represents function entry, normal statements, branches, loop headers, return exits, panic/diverge exits, fallthrough exits, break edges, and continue edges. Nodes that can produce diagnostics retain source spans. The first analysis is reachability: a non-void function is valid exactly when its fallthrough exit is unreachable.

`return` may appear anywhere. A diverging `panic()` path does not require a return. A proven non-terminating `while (true)` loop without a reachable `break` does not require a return. A reachable loop exit still requires a later return in a non-void function. Void functions may fall through, and `main(): void` retains implicit success.

Recursion and mutual recursion are accepted. Artificial execution-fuel, exact-state-cycle, and call-depth limits are not Doria semantics. Native compilation never executes user code as a preflight. Normal interpreter execution has no fixed language-visible cap; explicitly limited interpreter APIs may exist only for tests, fuzzing, and malformed-MIR defense.

## Reuse

The dataflow engine exposes graph nodes and deterministic predecessor/successor traversal, cloneable/equatable state, transfer, and join operations. Later constructor definite-initialization, null narrowing, move analysis, and borrow checking must reuse or extend this foundation rather than grow isolated terminal-block helpers.

## Non-goals

Stage 12 does not add new numeric families or operators, floats, runtime bool values, runtime strings, collections, classes, ownership, borrowing, LLVM, or another IR.
