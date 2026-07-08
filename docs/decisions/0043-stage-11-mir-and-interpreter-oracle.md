# 0043 Stage 11 MIR and interpreter oracle

Status: Accepted

## Decision

Stage 11 introduces MIR as Doria native-oriented, control-flow-oriented compiler IR. MIR is backend-independent, compiler-internal, and unstable until the compiler reaches the later v1.0 stabilization gates. It is not PHP output, Cranelift IR, LLVM IR, or the existing Doria IR/HIR.

Stage 11 also introduces a MIR interpreter as the debug and semantic-oracle path. The interpreter is intended to validate Doria-visible behavior without relying on PHP, Cranelift, LLVM, or host linker behavior.

Stage 11 eventually retires NativeSmokeModule. Stage 11a only seeded the MIR architecture and interpreter for a tiny executable subset; Stage 11b expands that seed to integer expressions and integer local slots. NativeSmokeModule remains temporary in Stage 11b and must not receive new language capability expansion.

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

## Stage 11b scope

Stage 11b expands the MIR seed to integer expressions and integer local slots.

Stage 11b supports:

- MIR int constants.
- MIR int local slots.
- Readonly int locals.
- Writable int locals.
- Int local assignment.
- `+=` and `-=` on writable int locals.
- Standalone `++` and `--` on writable int locals in statement position.
- Int arithmetic `+`, `-`, and `*`.
- Return of supported int expressions from `main(): int`.
- Execution of supported int local statements in `main(): void`.
- Interpreter evaluation of the Stage 11b subset with checked int64 arithmetic.
- Debug-target artifacts for the Stage 11b subset.

Unsupported Doria constructs in Stage 11b must be rejected as unsupported MIR Stage 11b coverage, not as invalid Doria.

## Non-goals

Stage 11b does not add:

- full Stage <=10 MIR port
- deletion of NativeSmokeModule
- replacement of Cranelift lowering
- ownership or borrow checking
- doria-rt
- runtime strings
- string locals
- string concatenation in MIR
- bool runtime values
- comparisons or conditions
- division, modulo, shifts, or bitwise operators
- `if` / `else`
- loops
- `break` or `continue` in MIR
- function calls
- helper functions
- classes, objects, or collections in MIR
- checked errors
- LLVM
- Baton

## Consequences

NativeSmokeModule remains temporary implementation-private bootstrap infrastructure for current native smoke coverage. It must not receive new language capability expansion unless an explicit later task approves a temporary compatibility fix.

MIR becomes the place to grow future native control-flow, interpreter, ownership, borrow-checking, and backend-lowering work. Stage 11b broadens the seed to integer locals and expressions, but Stage 11 is still incomplete and the full Stage <=10 MIR port remains future work.