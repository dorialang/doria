# 0043 Stage 11 MIR and interpreter oracle

Status: Accepted

## Decision

Stage 11 introduces MIR as Doria native-oriented, control-flow-oriented compiler IR. MIR is backend-independent, compiler-internal, and unstable until the compiler reaches the later v1.0 stabilization gates. It is not PHP output, Cranelift IR, LLVM IR, or the existing Doria IR/HIR.

Stage 11 also introduces a MIR interpreter as the debug and semantic-oracle path. The interpreter is intended to validate Doria-visible behavior without relying on PHP, Cranelift, LLVM, or host linker behavior.

Stage 11 eventually retires NativeSmokeModule. Stage 11a seeded the MIR architecture and interpreter for a tiny executable subset, Stage 11b expanded that seed to integer expressions and integer local slots, Stage 11c added condition evaluation and structured `if` control flow, and Stage 11d adds structured `while` loops and loop control. NativeSmokeModule remains temporary in Stage 11d and must not receive new language capability expansion.

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

## Stage 11c scope

Stage 11c expands MIR and the MIR interpreter to condition evaluation and structured `if` / `else` control flow.

Stage 11c supports:

- Backend-independent MIR branch and jump terminators.
- MIR condition representation without user-authored bool runtime locals.
- Bool literals in condition position.
- Integer comparisons in condition position: `==`, `!=`, `<`, `<=`, `>`, and `>=`.
- Condition operators `!` / `not`, `&&` / `and`, `||` / `or`, and `xor`.
- Short-circuit evaluation for `&&` / `and` and `||` / `or`.
- Bool-only, non-short-circuiting evaluation for `xor`.
- `if` without `else`, `if` with `else`, and `else if` chains.
- Nested `if` statements within the Stage 11c subset.
- Early return inside branches and fallthrough after an `if` statement.
- Stage 11b int local declarations, assignments, and mutations inside branches.
- Reads of existing int local slots after branch merges without introducing SSA or phi nodes.
- Exact string-literal `echo` inside branches of `main(): void`.
- Debug-target artifacts and interpreter execution for the Stage 11c subset.

Doria condition semantics are typed. Integer, string, object, or other truthiness is not introduced. MIR condition expressions preserve Doria's accepted operator semantics rather than PHP precedence or coercion rules.

Unsupported Doria constructs in Stage 11c must be rejected as unsupported MIR Stage 11c coverage, not reclassified as invalid Doria.

## Stage 11c non-goals

Stage 11c does not add:

- full Stage <=10 MIR port
- deletion of NativeSmokeModule
- replacement of Cranelift lowering
- ownership or borrow checking
- doria-rt
- runtime strings
- string locals
- string concatenation in MIR
- user-authored bool locals as runtime values
- returning bool from `main` or other functions
- division, modulo, shifts, or bitwise operators
- loops
- `break` or `continue` in MIR
- function calls
- helper functions
- `match` or value-returning `when`
- `try` / `catch` / `finally`
- classes, objects, or collections in MIR
- checked errors
- LLVM
- Baton

## Stage 11d scope

Stage 11d expands MIR and the MIR interpreter to structured `while` loops and loop control.

Stage 11d supports:

- `while` lowering through the existing backend-independent MIR branch and jump terminators.
- Reuse of Stage 11c typed condition evaluation for loop conditions.
- Explicit loop header, body, and exit basic blocks.
- Body fallthrough and `continue` jumps to the current loop header.
- `break` jumps to the current loop exit.
- An explicit loop-target stack so nested loops select the innermost `break` and `continue` targets.
- Nested `while` loops and loop-body-local int declarations.
- Stage 11b int local assignment and mutation inside loops.
- Stage 11c `if` / `else if` / `else` inside loops.
- Early return from inside loops.
- Exact string-literal `echo` inside loops of `main(): void`.
- Interpreter execution and debug-target artifacts for the Stage 11d subset.
- Finite loop execution without a fixed interpreter step limit. The deterministic interpreter detects a non-terminating cycle only when it revisits the same basic block with the same integer-local state.

MIR does not gain a high-level loop node. `while` is structured source syntax lowered into the same basic-block control-flow primitives that future backends consume.

Unsupported Doria constructs in Stage 11d must be rejected as unsupported MIR Stage 11d coverage, not reclassified as invalid Doria.

## Stage 11d non-goals

Stage 11d does not add:

- `for`
- `foreach`
- range or collection iteration
- labeled or numeric `break` / `continue`
- function calls or helper functions
- recursion
- runtime strings or string locals
- classes, objects, or collections in MIR
- ownership or borrow checking
- doria-rt
- deletion of NativeSmokeModule
- replacement of Cranelift lowering
- LLVM
- Baton

## Consequences

NativeSmokeModule remains temporary implementation-private bootstrap infrastructure for current native smoke coverage. It must not receive new language capability expansion unless an explicit later task approves a temporary compatibility fix.

MIR is the place to grow future native control-flow, interpreter, ownership, borrow-checking, and backend-lowering work. Stage 11d proves that the Stage 11c CFG model can represent backward edges and nested loop control without loop-specific MIR nodes, but Stage 11 is still incomplete. `for`, `foreach`, function calls, the full Stage <=10 MIR port, Cranelift-over-MIR lowering, and NativeSmokeModule retirement remain future work.
