# 0043 Stage 11 MIR and interpreter oracle

Status: Accepted

## Decision

Stage 11 introduces MIR as Doria native-oriented, control-flow-oriented compiler IR. MIR is backend-independent, compiler-internal, and unstable until the compiler reaches the later v1.0 stabilization gates. It is not PHP output, Cranelift IR, LLVM IR, or the existing Doria IR/HIR.

Stage 11 also introduces a MIR interpreter as the debug and semantic-oracle path. The interpreter is intended to validate Doria-visible behavior without relying on PHP, Cranelift, LLVM, or host linker behavior.

Stage 11 is complete. Stage 11a seeded the MIR architecture and interpreter for a tiny executable subset, Stage 11b expanded that seed to integer expressions and integer local slots, Stage 11c added condition evaluation and structured `if` control flow, Stage 11d added structured `while` loops and loop control, Stage 11e added traditional `for` loops and integer range `foreach`, Stage 11f added top-level free functions and calls, Stage 11g added readonly string locals plus string-expression echo parity, and Stage 11h completed parity, moved Cranelift directly onto MIR, established the manifest-driven differential suite, and retired the Stage 7-10 native smoke architecture.

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
- Finite loop execution under a bounded interpreter budget of 100,000 executed basic blocks. Exact-state cycle detection rejects deterministic cycles earlier when the interpreter revisits the same basic block with the same integer-local state, while the execution budget also bounds changing-state loops.

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

## Stage 11e scope

Stage 11e expands MIR lowering and debug interpretation to traditional `for` loops and integer range `foreach`.

Stage 11e supports:

- MIR lowering for traditional `for` loops.
- MIR lowering for inclusive (`0..10`) and exclusive-end (`0..<10`) integer range `foreach`.
- Readonly range `foreach` bindings scoped to each loop body.
- Loop-local assignment and mutation through the Stage 11b integer-local support.
- Stage 11c conditions and `if` control flow inside loop bodies.
- Stage 11d `break` and `continue` inside `for` and integer range `foreach`.
- A distinct `for` increment block so `continue` executes the increment before the next condition check.
- A range-update block so `continue` advances the range before the next condition check.
- A terminal-value guard for inclusive ranges so an endpoint of `int64::MAX` is not incremented past the Doria `int` range.
- Nested loop-target handling across `while`, `for`, and integer range `foreach`.
- Stage 11b integer expressions in range bounds, evaluated once before iteration.
- Exact string-literal `echo` and early return inside supported loops.
- Interpreter execution and debug-target artifacts for the Stage 11e subset.
- The existing bounded interpreter fuel and exact-state cycle detection from Stage 11d.

Traditional `for` and integer range `foreach` are structured source syntax lowered into the existing MIR locals, assignments, branches, and jumps. Stage 11e does not add high-level loop nodes, SSA, or phi nodes.

Unsupported Doria constructs in Stage 11e must be rejected as unsupported MIR Stage 11e coverage, not reclassified as invalid Doria.

## Stage 11e non-goals

Stage 11e does not add:

- collection iteration
- user-defined iterable protocols
- public `Iterable<T>` / `Iterator<T>` conformance
- function calls or helper functions
- recursion
- runtime strings or string locals
- string concatenation in MIR
- classes, objects, or collections in MIR
- ownership or borrow checking
- doria-rt
- deletion of NativeSmokeModule
- replacement of Cranelift lowering
- LLVM
- Baton

## Stage 11f scope

Stage 11f expands MIR lowering and debug interpretation to top-level free functions and calls in the Stage 10-supported subset.

Stage 11f supports:

- Multiple top-level free functions with stable declaration-order MIR function IDs.
- Exactly one `main` entry point, taking no parameters and returning `int` or `void`.
- Helper functions with `int` parameters and `int` or `void` returns.
- Calls to `int` helpers in supported integer expressions.
- Calls to `void` helpers in statement position.
- Positional arguments using Stage 11b integer expressions.
- Helper bodies using the Stage 11a-11e supported statement and expression subset, including `if`, `while`, traditional `for`, integer range `foreach`, assignment, mutation, early return, and loop control.
- Exact string-literal `echo` in void helpers.
- Interpreter call frames that evaluate arguments, bind parameter locals, preserve caller locals, return `int` or `void`, and share stdout.
- Full Doria `int` values for helper returns. The portable process-status boundary applies only to `main(): int`.
- A global 100,000-basic-block execution budget, per-frame exact-state cycle detection, and a defensive 256-frame call-depth limit.
- Deterministic pre-interpretation rejection of direct and mutual recursion.
- Debug-target artifacts and parity checks with existing native smoke support where that requires no NativeSmokeModule expansion.

Unsupported Doria constructs in Stage 11f must be rejected as unsupported MIR Stage 11f coverage, not reclassified as invalid Doria.

## Stage 11f non-goals

Stage 11f does not add:

- methods or static calls
- constructors
- classes or objects
- string parameters or string returns
- bool helper returns
- named arguments or default arguments
- recursion or mutual recursion
- closures
- runtime strings, string locals, or string concatenation in MIR
- ownership or borrow checking
- doria-rt
- collection iteration or user-defined iterators
- deletion of NativeSmokeModule
- replacement of Cranelift lowering
- LLVM
- Baton

## Stage 11g scope

Stage 11g expands MIR lowering and debug interpretation to readonly string locals and string concatenation in echo expressions, then records the current Stage <=10 MIR parity matrix.

Stage 11g supports:

- MIR `string` local slots for readonly inferred and explicitly typed string locals.
- MIR string expressions containing string literals, readonly string-local reads, and ordered `.` concatenation.
- Readonly string-local initialization from the supported string-expression subset.
- Echo of supported string expressions in `main(): void` and void helper functions.
- String-local and concat echo inside supported `if`, `while`, traditional `for`, and integer range `foreach` bodies.
- Interpreter-private string values in isolated call frames without defining a Doria runtime representation or ABI.
- Exact-byte stdout with no implicit newline.
- The Stage 11d global execution budget and exact-state cycle detection, now including string local state.
- The Stage 11f defensive call-depth limit and shared stdout across helper calls.
- Debug-target artifacts for the Stage 11g examples.
- A Stage <=10 native-smoke versus MIR/debug parity matrix in `docs/notes/stage-11-mir-parity-matrix.md`.

Unsupported Doria constructs in Stage 11g must be rejected as unsupported MIR Stage 11g coverage, unless an earlier semantic diagnostic already rejects the source as invalid Doria.

## Stage 11g non-goals

Stage 11g does not add:

- a runtime heap string model or stable string ABI
- writable string locals
- string assignment
- string parameters or string returns
- string comparison
- broader string interpolation expansion
- implicit display or string conversion
- echo of int, bool, mixed, objects, or collections
- methods or static calls
- constructors
- classes, objects, or collections in MIR
- collection iteration or user-defined iterators
- recursion or mutual recursion
- ownership or borrow checking
- doria-rt
- direct MIR-to-Cranelift lowering
- deletion of NativeSmokeModule
- LLVM
- Baton

The interpreter may use Rust `String` values internally to execute this compile-time-known subset. That implementation detail does not define Doria allocation, ownership, layout, termination, or runtime concatenation semantics.

## Stage 11h - Stage 11 completion

Stage 11h completes the architecture migration:

- All accepted Stage <=10 executable source lowering passes through MIR.
- String-expression `echo` is supported in both int-returning and void functions; function return type constrains returns, not unrelated statements.
- The debug backend executes MIR through the bounded interpreter oracle.
- The native backend lowers checked HIR to MIR, runs the temporary bounded Stage 11 interpreter preflight, lowers MIR independently to a Cranelift object, and invokes the host linker.
- Cranelift consumes MIR only. Its lowering module does not depend on HIR, AST, the parser, or the retired smoke representation.
- Integer parameters and locals lower as signed I64 values using backend-private stack slots. Emitted `+`, `-`, and `*` operations retain checked overflow behavior.
- All MIR functions are declared before body lowering with deterministic implementation-private symbols. A separate exported process wrapper maps `main(): void` to status 0 and validates `main(): int` against the portable `0..125` process boundary.
- MIR branch and jump terminators lower directly to Cranelift control flow. Recursive condition lowering preserves left-to-right evaluation, short-circuit `and`/`or`, and eager bool-only `xor`.
- Compile-time-known readonly string locals and concatenations resolve to exact bytes during object lowering. Unix-like and Windows stdout paths use explicit pointer/length writes and retry short writes without defining runtime strings.
- `crates/doriac/tests/fixtures/stage_11_native_parity_examples.txt` is the executable accepted-example manifest. The differential suite compares exact interpreter/native stdout and process status for every entry and guards against unclassified native examples.
- The private Stage 7-10 native smoke module, evaluator, Cranelift lowerer, fallback paths, and implementation-structure tests have been removed. Equivalent behavior is covered by MIR lowering/interpreter tests, linker-independent MIR-to-object tests, native preflight diagnostics, and the differential suite.

The interpreter preflight is temporary migration-boundary behavior for the deterministic Stage <=10 subset. It is not used to generate machine code and its captured stdout is discarded during native compilation. Stage 12's runtime/panic foundation will revisit which checked failures remain compile-time preflight diagnostics and which become runtime panics.

## Consequences

MIR is now the single active native-oriented compiler IR. New native language capability work must extend MIR, its validation/interpreter path, and backend lowering together rather than introducing a parallel representation or direct HIR-to-Cranelift route.

Stage 11 is complete. Stage 12 is next: general control flow plus the minimal doria-rt runtime/panic foundation. Runtime strings, collection iteration, methods/classes, ownership and borrow checking, stable ABI work, LLVM, and other later capabilities remain outside Stage 11.
