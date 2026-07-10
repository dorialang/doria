# Stage 11 MIR Parity Matrix

Documentation role: working note.

Source of truth for sequencing remains `docs/doria-end-to-end-plan.md`. The executable source of truth for accepted Stage <=10 examples is `crates/doriac/tests/fixtures/stage_11_native_parity_examples.txt`. The differential test reads that manifest, executes each source through the MIR interpreter and MIR-backed Cranelift backend, and compares exact stdout bytes and process status.

`Covered` means the interpreter and Cranelift consume the same MIR and the behavior has focused or manifest-driven differential coverage.

| Feature / example | MIR interpreter | Cranelift from MIR | Status | Notes |
| --- | --- | --- | --- | --- |
| `main(): int` literal return | Covered | Covered | Covered | Both produce the same explicit status. |
| `main(): void` fallthrough | Covered | Covered | Covered | Both map normal completion to status 0. |
| String-literal echo | Covered | Covered | Covered | Exact bytes, no implicit newline. |
| Readonly and writable int locals | Covered | Covered | Covered | MIR uses typed slots; Cranelift uses backend-private stack slots. |
| Checked int arithmetic `+`, `-`, `*` | Covered | Covered | Covered | Interpreter checks `int64`; emitted code uses signed overflow traps. |
| `if` / `else if` / `else` | Covered | Covered | Covered | Includes returning and fallthrough branches. |
| `while` | Covered | Covered | Covered | MIR CFG backedges lower directly. Preflight remains bounded. |
| `break` / `continue` | Covered | Covered | Covered | Nested loop targets and loop-specific continue blocks are covered. |
| Traditional `for` | Covered | Covered | Covered | `continue` reaches the increment block. |
| Integer range `foreach` | Covered | Covered | Covered | Inclusive/exclusive ranges and terminal overflow guards are covered. |
| Top-level int helpers | Covered | Covered | Covered | Integer parameters and returns use full Doria `int` values. |
| Void helper calls | Covered | Covered | Covered | Shared stdout preserves source call order. |
| Readonly string locals and concat | Covered | Covered | Covered | Compile-time-known bytes only; no runtime string ABI. |
| String echo in int-returning functions | Covered | Covered | Covered | Statement validity is independent of function return type. |
| Short-circuit conditions with helper calls | Covered | Covered | Covered | `and`/`or` short-circuit; `xor` evaluates both in order. |
| Process exit boundary | Covered | Covered | Covered | Only `main(): int` is restricted to `0..125`. |
| Bounded execution and recursion rejection | Covered | Covered | Covered | Native uses the MIR interpreter as temporary Stage 11 preflight. |
| Cranelift lowering source | MIR | MIR | Covered | `codegen_cranelift` has no HIR or retired-smoke dependency. |
| Complete differential harness | Manifest-driven | Manifest-driven | Covered | CI requires a linker and runs the complete manifest. |

## Retirement Gate

Status: Passed.

All accepted Stage <=10 lowering passes through MIR, the interpreter and Cranelift consume the same MIR, the executable manifest passes exact differential checks, and the Stage 7-10 native smoke module has been retired and deleted. Stage 11 is complete; Stage 12 is next.
