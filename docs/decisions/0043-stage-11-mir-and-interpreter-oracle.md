# 0043 Stage 11 MIR and interpreter oracle

Status: Accepted

## Decision

Stage 11 introduces MIR as Doria native-oriented, control-flow-oriented compiler IR. MIR is backend-independent, compiler-internal, and unstable until the compiler reaches the later v1.0 stabilization gates. It is not PHP output, Cranelift IR, LLVM IR, or the existing Doria IR/HIR.

Stage 11 also introduces a MIR interpreter as the debug and semantic-oracle path. The interpreter is intended to validate Doria-visible behavior without relying on PHP, Cranelift, LLVM, or host linker behavior.

Stage 11 eventually retires NativeSmokeModule. Stage 11a only seeds the MIR architecture and interpreter for a tiny executable subset; it does not delete NativeSmokeModule yet and does not port all Stage <=10 native lowering.

## Stage 11a scope

Stage 11a supports:

- MIR representation for executable main functions returning int or void.
- MIR representation for exact string-literal echo statements in void main.
- Deterministic MIR dumps through a compiler inspection command.
- MIR interpreter execution for main(): int literal returns, main(): void fallthrough, main(): void bare return, and exact string-literal echo.
- Debug-target compile artifacts when the CLI can route target debug cleanly.
- Tests for the tiny lowering and interpreter subset.
- Tiny parity coverage with current native smoke behavior where it does not require expanding NativeSmokeModule.

Unsupported Doria constructs in Stage 11a must be rejected as unsupported MIR Stage 11a coverage, not as invalid Doria.

## Non-goals

Stage 11a does not add:

- full Stage <=10 MIR port
- deletion of NativeSmokeModule
- replacement of Cranelift lowering
- ownership or borrow checking
- doria-rt
- runtime strings
- bool or string primitive-helper native support
- function calls in MIR
- loops in MIR
- local variables in MIR
- classes, objects, or collections in MIR
- checked errors
- LLVM
- Baton

## Consequences

NativeSmokeModule remains temporary implementation-private bootstrap infrastructure for current native smoke coverage. It must not receive new language capability expansion unless an explicit later task approves a temporary compatibility fix.

MIR becomes the place to grow future native control-flow, interpreter, ownership, borrow-checking, and backend-lowering work. Stage 11b/11c may broaden lowering and parity, but Stage 11a deliberately keeps the supported subset small enough to test end to end.
