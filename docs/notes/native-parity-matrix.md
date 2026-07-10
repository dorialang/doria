# Native MIR Parity Matrix

Documentation role: working note.

Source of truth for sequencing remains `docs/doria-end-to-end-plan.md`. The durable executable manifest is `crates/doriac/tests/fixtures/native_parity_examples.txt`. The differential test reads that manifest, executes each finite source through the MIR interpreter and MIR-backed Cranelift backend, and compares exact stdout bytes, stderr bytes, and process status.

`Covered` means the interpreter and Cranelift consume the same MIR and the behavior has focused or manifest-driven differential coverage.

| Feature / example | MIR interpreter | Cranelift from MIR | Status | Notes |
| --- | --- | --- | --- | --- |
| `main(): int` literal return | Covered | Covered | Covered | Both produce the same explicit status. |
| `main(): void` fallthrough | Covered | Covered | Covered | Both map normal completion to status 0. |
| String-literal echo | Covered | Covered | Covered | Exact bytes, no implicit newline. |
| Readonly and writable int locals | Covered | Covered | Covered | MIR uses typed slots; Cranelift uses backend-private stack slots. |
| Checked int arithmetic `+`, `-`, `*` | Covered | Covered | Covered | Overflow follows the shared panic path rather than exposing a backend trap. |
| `if` / `else if` / `else` | Covered | Covered | Covered | Includes returning and fallthrough branches. |
| `while` | Covered | Covered | Covered | MIR CFG backedges lower directly; long finite loops exceed the former interpreter budget. |
| `break` / `continue` | Covered | Covered | Covered | Nested loop targets and loop-specific continue blocks are covered. |
| Traditional `for` | Covered | Covered | Covered | `continue` reaches the increment block. |
| Integer range `foreach` | Covered | Covered | Covered | Inclusive/exclusive ranges and terminal overflow guards are covered. |
| Top-level int helpers | Covered | Covered | Covered | Integer parameters and returns use full Doria `int` values. |
| Void helper calls | Covered | Covered | Covered | Shared stdout preserves source call order. |
| Readonly string locals and concat | Covered | Covered | Covered | Compile-time-known bytes only; no runtime string ABI. |
| String echo in int-returning functions | Covered | Covered | Covered | Statement validity is independent of function return type. |
| Short-circuit conditions with helper calls | Covered | Covered | Covered | `and`/`or` short-circuit; `xor` evaluates both in order. |
| Process exit boundary | Covered | Covered | Covered | Only `main(): int` is restricted to `0..125`. |
| Recursion and mutual recursion | Covered | Covered | Covered | Explicit interpreter frames remove the former 256-frame semantic cap. |
| Return from nested control flow | Covered | Covered | Covered | Source CFG reachability permits return anywhere and rejects reachable fallthrough. |
| Explicit panic | Covered | Covered | Covered | Exact stderr stack trace and status 101 agree. |
| Checked overflow panic | Covered | Covered | Covered | Addition, subtraction, and multiplication messages agree exactly. |
| Invalid process status panic | Covered | Covered | Covered | Runtime entry validates `main(): int` and exits 101 on failure. |
| Native compile without execution preflight | Covered | Covered | Covered | Infinite-loop source compiles but is excluded from executable parity. |
| Cranelift lowering source | MIR | MIR | Covered | `codegen_cranelift` has no HIR or retired-smoke dependency. |
| Complete differential harness | Manifest-driven | Manifest-driven | Covered | CI requires a runtime artifact and linker; stdout, stderr, and status are exact. |

## Retirement Gate

Status: Passed.

All accepted Stage <=10 lowering passes through MIR, the interpreter and Cranelift consume the same MIR, the executable manifest passes exact differential checks, and the Stage 7-10 native smoke module remains retired and deleted. Stage 11 retirement passed. Stage 12 is the highest completed stage; Stage 13 is next.
