# Native MIR Parity Matrix

Documentation role: working note.

Source of truth for sequencing remains `docs/doria-end-to-end-plan.md`. The durable executable manifest is `crates/doriac/tests/fixtures/native_parity_examples.txt`. The differential test reads that manifest, executes each finite source through the MIR interpreter and MIR-backed Cranelift backend, and compares exact stdout bytes, stderr bytes, and process status.

`Covered` means the interpreter and Cranelift consume the same MIR and the behavior has focused or manifest-driven differential coverage.

| Feature / example | MIR interpreter | Cranelift from MIR | Status | Notes |
| --- | --- | --- | --- | --- |
| `main(): int` literal return | Covered | Covered | Covered | Both produce the same explicit status. |
| `main(): void` fallthrough | Covered | Covered | Covered | Both map normal completion to status 0. |
| String-literal echo | Covered | Covered | Covered | Exact bytes, no implicit newline. |
| Readonly and writable integer locals | Covered | Covered | Covered | MIR records canonical width and signedness; Cranelift uses matching backend-private stack slots. |
| `int8` | Covered | Covered | Covered | Signed 8-bit locals, parameters, returns, arithmetic, comparison, and panic boundaries. |
| `int16` | Covered | Covered | Covered | Signed 16-bit values retain their declared width through MIR and native ABI lowering. |
| `int32` | Covered | Covered | Covered | Signed 32-bit values retain their declared width through MIR and native ABI lowering. |
| `int` / `int64` | Covered | Covered | Covered | One canonical signed 64-bit type; both source spellings lower identically. |
| `uint8` | Covered | Covered | Covered | Unsigned 8-bit arithmetic, shifts, conversions, ABI values, and overflow panic. |
| `uint16` | Covered | Covered | Covered | Unsigned 16-bit division/remainder and narrow helper transport. |
| `uint32` | Covered | Covered | Covered | Unsigned 32-bit comparison and bitwise behavior select unsigned lowering. |
| `uint64` | Covered | Covered | Covered | Full `0..18446744073709551615` values survive locals, calls, returns, and comparison. |
| Contextual integer literals | Covered | Covered | Covered | Declaration, argument, return, assignment, and typed-operand contexts preserve the selected canonical type. |
| Checked integer arithmetic `+`, `-`, `*` | Covered | Covered | Covered | Every width and signedness panics through the shared runtime instead of exposing a backend trap. |
| Signed division | Covered | Covered | Covered | Truncates toward zero; zero divisor and `MIN / -1` have distinct deterministic panics. |
| Unsigned division | Covered | Covered | Covered | Uses unsigned division and the shared divide-by-zero panic path. |
| Signed remainder | Covered | Covered | Covered | Remainder follows the truncating quotient; `MIN % -1` is zero. |
| Unsigned remainder | Covered | Covered | Covered | Uses unsigned remainder and panics on a zero divisor. |
| Fixed-width left shift | Covered | Covered | Covered | Count is validated; high bits are discarded without arithmetic-overflow panic. |
| Signed right shift | Covered | Covered | Covered | Arithmetic shift propagates the sign bit. |
| Unsigned right shift | Covered | Covered | Covered | Logical shift introduces zero bits. |
| Bitwise `&`, `|`, `^`, `~` | Covered | Covered | Covered | Operators preserve fixed-width two's-complement bit patterns. |
| Unary negation | Covered | Covered | Covered | Signed-only; negating the signed minimum uses the shared overflow panic. |
| Explicit companion `from` conversion | Covered | Covered | Covered | Same-type/exact conversions preserve values; out-of-range conversions panic. |
| `++`, `--`, and compound assignments | Covered | Covered | Covered | Lower through the same checked/operator rules as their corresponding integer operations. |
| `if` / `else if` / `else` | Covered | Covered | Covered | Includes returning and fallthrough branches. |
| `while` | Covered | Covered | Covered | MIR CFG backedges lower directly; long finite loops exceed the former interpreter budget. |
| `break` / `continue` | Covered | Covered | Covered | Nested loop targets and loop-specific continue blocks are covered. |
| Traditional `for` | Covered | Covered | Covered | `continue` reaches the increment block. |
| Integer range `foreach` | Covered | Covered | Covered | Inclusive/exclusive ranges and terminal overflow guards are covered. |
| Top-level integer helpers | Covered | Covered | Covered | Parameters and returns preserve every declared width and signedness. |
| Void helper calls | Covered | Covered | Covered | Shared stdout preserves source call order. |
| Readonly string locals and concat | Covered | Covered | Covered | Compile-time-known bytes only; no runtime string ABI. |
| String echo in int-returning functions | Covered | Covered | Covered | Statement validity is independent of function return type. |
| Short-circuit conditions with helper calls | Covered | Covered | Covered | `and`/`or` short-circuit; `xor` evaluates both in order. |
| Process exit boundary | Covered | Covered | Covered | Only `main(): int` is restricted to `0..125`. |
| Recursion and mutual recursion | Covered | Covered | Covered | Explicit interpreter frames remove the former 256-frame semantic cap. |
| Return from nested control flow | Covered | Covered | Covered | Source CFG reachability permits return anywhere and rejects reachable fallthrough. |
| Explicit panic | Covered | Covered | Covered | Exact stderr stack trace and status 101 agree. |
| Checked overflow panic | Covered | Covered | Covered | Addition, subtraction, and multiplication messages agree exactly. |
| Signed negation overflow panic | Covered | Covered | Covered | Exact message, Doria stack frames, and status 101 agree. |
| Divide-by-zero and signed-division-overflow panic | Covered | Covered | Covered | Both failure classes keep their distinct deterministic messages. |
| Remainder-by-zero panic | Covered | Covered | Covered | Exact message, Doria stack frames, and status 101 agree. |
| Shift-count panic | Covered | Covered | Covered | Negative and width-or-greater counts use one deterministic panic message. |
| Conversion-out-of-range panic | Covered | Covered | Covered | Checked companion conversion failure agrees on stderr and status 101. |
| Fixed-width function ABI | Covered | Covered | Covered | Narrow signed/unsigned parameters and returns preserve canonical type and bit pattern. |
| `uint64` boundary transport | Covered | Covered | Covered | Maximum unsigned 64-bit value survives local, call, return, and comparison paths. |
| Invalid process status panic | Covered | Covered | Covered | Runtime entry validates `main(): int` and exits 101 on failure. |
| Native compile without execution preflight | Covered | Covered | Covered | Infinite-loop source compiles but is excluded from executable parity. |
| Cranelift lowering source | MIR | MIR | Covered | `codegen_cranelift` has no HIR or retired-smoke dependency. |
| Complete differential harness | Manifest-driven | Manifest-driven | Covered | CI requires a runtime artifact and linker; stdout, stderr, and status are exact. |

## Retirement Gate

Status: Passed.

All accepted Stage <=13 lowering passes through typed MIR, the interpreter and Cranelift consume the same MIR, every finite native example is required in the executable manifest, and the Stage 7-10 native smoke module remains retired and deleted. Stage 13 is the highest completed stage; Stage 14 float execution and bool runtime values are next.
