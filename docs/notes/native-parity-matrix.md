# Native MIR Parity Matrix

Documentation role: working note.

Source of truth for sequencing remains `docs/doria-end-to-end-plan.md`. The durable executable manifest is `crates/doriac/tests/fixtures/native_parity_examples.txt`. The differential test reads that manifest, executes each finite source through the MIR interpreter, Cranelift fast profile, and LLVM release profile, and compares exact stdout bytes, stderr bytes, and process status.

`Covered` means the interpreter, Cranelift, and LLVM consume the same validated MIR and the behavior has focused or manifest-driven triple differential coverage.

| Feature / example | MIR interpreter | Cranelift fast | LLVM release | Status | Notes |
| --- | --- | --- | --- | --- | --- |
| `main(): int` literal return | Covered | Covered | Covered | Covered | All three produce the same explicit status. |
| `main(): void` fallthrough | Covered | Covered | Covered | Covered | All three map normal completion to status 0. |
| String-literal echo | Covered | Covered | Covered | Covered | Exact bytes, no implicit newline. |
| Readonly and writable integer locals | Covered | Covered | Covered | Covered | MIR records canonical width and signedness; Cranelift uses matching backend-private stack slots. |
| `int8` | Covered | Covered | Covered | Covered | Signed 8-bit locals, parameters, returns, arithmetic, comparison, and panic boundaries. |
| `int16` | Covered | Covered | Covered | Covered | Signed 16-bit values retain their declared width through MIR and native ABI lowering. |
| `int32` | Covered | Covered | Covered | Covered | Signed 32-bit values retain their declared width through MIR and native ABI lowering. |
| `int` / `int64` | Covered | Covered | Covered | Covered | One canonical signed 64-bit type; both source spellings lower identically. |
| `uint8` | Covered | Covered | Covered | Covered | Unsigned 8-bit arithmetic, shifts, conversions, ABI values, and overflow panic. |
| `uint16` | Covered | Covered | Covered | Covered | Unsigned 16-bit division/remainder and narrow helper transport. |
| `uint32` | Covered | Covered | Covered | Covered | Unsigned 32-bit comparison and bitwise behavior select unsigned lowering. |
| `uint64` | Covered | Covered | Covered | Covered | Full `0..18446744073709551615` values survive locals, calls, returns, and comparison. |
| Contextual integer literals | Covered | Covered | Covered | Covered | Declaration, argument, return, assignment, and typed-operand contexts preserve the selected canonical type. |
| Checked integer arithmetic `+`, `-`, `*` | Covered | Covered | Covered | Covered | Every width and signedness panics through the shared runtime instead of exposing a backend trap. |
| Signed division | Covered | Covered | Covered | Covered | Truncates toward zero; zero divisor and `MIN / -1` have distinct deterministic panics. |
| Unsigned division | Covered | Covered | Covered | Covered | Uses unsigned division and the shared divide-by-zero panic path. |
| Signed remainder | Covered | Covered | Covered | Covered | Remainder follows the truncating quotient; `MIN % -1` is zero. |
| Unsigned remainder | Covered | Covered | Covered | Covered | Uses unsigned remainder and panics on a zero divisor. |
| Fixed-width left shift | Covered | Covered | Covered | Covered | Count is validated; high bits are discarded without arithmetic-overflow panic. |
| Signed right shift | Covered | Covered | Covered | Covered | Arithmetic shift propagates the sign bit. |
| Unsigned right shift | Covered | Covered | Covered | Covered | Logical shift introduces zero bits. |
| Bitwise `&`, `|`, `^`, `~` | Covered | Covered | Covered | Covered | Operators preserve fixed-width two's-complement bit patterns. |
| Unary negation | Covered | Covered | Covered | Covered | Signed-only; negating the signed minimum uses the shared overflow panic. |
| Explicit companion `from` conversion | Covered | Covered | Covered | Covered | Same-type/exact conversions preserve values; out-of-range conversions panic. |
| `++`, `--`, and compound assignments | Covered | Covered | Covered | Covered | Lower through the same checked/operator rules as their corresponding integer operations. |
| `if` / `else if` / `else` | Covered | Covered | Covered | Covered | Includes returning and fallthrough branches. |
| `while` | Covered | Covered | Covered | Covered | MIR CFG backedges lower directly; long finite loops exceed the former interpreter budget. |
| `break` / `continue` | Covered | Covered | Covered | Covered | Nested loop targets and loop-specific continue blocks are covered. |
| Traditional `for` | Covered | Covered | Covered | Covered | `continue` reaches the increment block. |
| Integer range `foreach` | Covered | Covered | Covered | Covered | Inclusive/exclusive ranges and terminal overflow guards are covered. |
| Top-level integer helpers | Covered | Covered | Covered | Covered | Parameters and returns preserve every declared width and signedness. |
| Void helper calls | Covered | Covered | Covered | Covered | Shared stdout preserves source call order. |
| Runtime string locals, rebinding, parameters, returns and calls | Covered | Covered | Covered | Covered | Immutable UTF-8 Copy values use the private refcounted runtime ABI. |
| Runtime concat and primitive display | Covered | Covered | Covered | Covered | Decimal integers, shortest-round-trip floats, lowercase bools, and current interpolation agree exactly. |
| Arithmetic expression interpolation | Covered | Covered | Covered | Covered | `main_expression_interpolation.doria` produces exact `sum: 42` bytes. |
| Function-call interpolation | Covered | Covered | Covered | Covered | Ordinary calls lower inside interpolation without a second expression grammar. |
| Interpolation exactly-once evaluation | Covered | Covered | Covered | Covered | Side-effecting calls execute once per embedded expression. |
| Interpolation left-to-right evaluation | Covered | Covered | Covered | Covered | `main_expression_interpolation_order.doria` produces exact `LR=42`. |
| Bool expression interpolation | Covered | Covered | Covered | Covered | Reuses lowercase canonical bool display. |
| Float expression interpolation | Covered | Covered | Covered | Covered | Reuses deterministic shortest-round-trip display for each declared width. |
| String expression interpolation | Covered | Covered | Covered | Covered | Literal, local, concatenation, and string-call values retain exact bytes. |
| Literal-brace escaping | Frontend | Frontend | Frontend | Covered | `\{` is required for literal `{`; bare `}` and `\}` are accepted; doubling is rejected. |
| Malformed interpolation diagnostics | Frontend | Frontend | Frontend | Covered | Empty, unterminated, and malformed inner expressions retain original source spans. |
| Malformed literal-brace diagnostic | Frontend | Frontend | Frontend | Covered | P0002 carries a machine-applicable replacement of `{` with `\{`. |
| Non-Displayable diagnostics | Frontend | Frontend | Frontend | Covered | Every display context names the class and exact `Displayable::toString` contract. |
| `Displayable` frontend conformance | Frontend | Frontend | Frontend | Covered | Explicit nominal conformance and exact method shape are checked before MIR. |
| Native class allocation and construction | Covered | Covered | Covered | Covered | Headerless payloads use shared layout; property initializers and promotions run before the lifecycle body. |
| Narrow direct constructor initialization | Covered | Covered | Covered | Covered | The temporary soundness gate accepts only proven direct initialization shapes until Stage 21. |
| Class-valued locals, calls, and returns | Covered | Covered | Covered | Covered | Pointer-sized move values preserve ownership through free-function ABI boundaries. |
| `take` ownership transfer | Covered | Covered | Covered | Covered | Transfer invalidates the caller slot and cleanup becomes the callee's obligation. |
| Property loads and Stage 19 assignments | Covered | Covered | Covered | Covered | Shared class metadata supplies checked types and compiler-known offsets. |
| Deterministic class destruction | Covered | Covered | Covered | Covered | Lifecycle body runs first, owned properties drop in reverse order, allocation frees last. |
| Structured-exit cleanup | Covered | Covered | Covered | Covered | Fallthrough, return, break, and continue drop still-owned locals; panic intentionally does not unwind. |
| Statement class temporaries | Covered | Covered | Covered | Covered | Borrowed and transferred temporaries are released exactly once at their accepted ownership boundary. |
| Replacement-before-drop assignment | Covered | Covered | Covered | Covered | Replacement is acquired before the previous destination owner is destroyed. |
| Stage 19/20 native memory safety | N/A | Linux CI | Linux CI | Covered | Valgrind executes ownership-bearing class and method fixtures under both native profiles; ordinary parity remains cross-platform. |
| Instance methods | Covered | Covered | Covered | Covered | Concrete method identities carry explicit readonly/writable receivers through shared MIR. |
| Static methods | Covered | Covered | Covered | Covered | Qualified calls have no receiver and retain ordinary argument, return, panic, and ownership behavior. |
| Class and top-level constants | Covered | Covered | Covered | Covered | The bounded evaluator resolves forward dependencies and folds typed values before MIR. |
| Copy-type static properties | Covered | Covered | Covered | Covered | Plain per-process data symbols support readonly reads and qualified writable reassignment without runtime initialization. |
| Sigil-free static identity and `self` | Covered | Covered | Covered | Covered | Constants, static properties, and static methods resolve to concrete class-owned identities before MIR; `self` return types resolve before ABI lowering. |
| Constructor writable-static mutation | Covered | Covered | Covered | Covered | `main_stage20_static_constructor.doria` treats the write as ordinary mutation and preserves exact destructor output. |
| Stage 20 static identity diagnostics | Frontend | Frontend | Frontend | Covered | `Foo::$prop` and `static::` are rejected before MIR with exact `$` removal and `self` qualifier fixes. |
| Generalized `parent::` and trait-local `self::` grammar | Frontend | Frontend | Frontend | Covered | Accepted syntax produces Stage 34/35 semantic diagnostics without parser or LSP syntax errors. |
| `internal` member enforcement | Frontend | Frontend | Frontend | Covered | Rejected access never reaches MIR; same-class access covers instance, static, constant, and lifecycle members. |
| `Displayable` native execution | Covered | Covered | Covered | Covered | Statically known conforming classes call ordinary `toString()` MIR exactly once and left-to-right. |
| PHP `Displayable` subset | N/A | N/A | N/A | Covered | Generated private interface invokes Doria `toString` exactly once and never relies on `__toString`. |
| Parser fuzzing | Frontend | Frontend | Frontend | Covered | Bounded CI fuzzing seeds nested strings, braces, malformed expressions, and UTF-8 offsets. |
| String equality and ordering | Covered | Covered | Covered | Covered | Equality is exact-byte and ordering is unsigned byte-lexicographic. |
| String echo in int-returning functions | Covered | Covered | Covered | Covered | Statement validity is independent of function return type. |
| Short-circuit conditions with helper calls | Covered | Covered | Covered | Covered | `and`/`or` short-circuit; `xor` evaluates both in order. |
| Process exit boundary | Covered | Covered | Covered | Covered | Only `main(): int` is restricted to `0..125`. |
| Recursion and mutual recursion | Covered | Covered | Covered | Covered | Explicit interpreter frames remove the former 256-frame semantic cap. |
| Return from nested control flow | Covered | Covered | Covered | Covered | Source CFG reachability permits return anywhere and rejects reachable fallthrough. |
| Explicit panic | Covered | Covered | Covered | Covered | Exact stderr stack trace and status 101 agree. |
| Checked overflow panic | Covered | Covered | Covered | Covered | Addition, subtraction, and multiplication messages agree exactly. |
| Signed negation overflow panic | Covered | Covered | Covered | Covered | Exact message, Doria stack frames, and status 101 agree. |
| Divide-by-zero and signed-division-overflow panic | Covered | Covered | Covered | Covered | Both failure classes keep their distinct deterministic messages. |
| Remainder-by-zero panic | Covered | Covered | Covered | Covered | Exact message, Doria stack frames, and status 101 agree. |
| Shift-count panic | Covered | Covered | Covered | Covered | Negative and width-or-greater counts use one deterministic panic message. |
| Conversion-out-of-range panic | Covered | Covered | Covered | Covered | Checked companion conversion failure agrees on stderr and status 101. |
| Fixed-width function ABI | Covered | Covered | Covered | Covered | Narrow signed/unsigned parameters and returns preserve canonical type and bit pattern. |
| `uint64` boundary transport | Covered | Covered | Covered | Covered | Maximum unsigned 64-bit value survives local, call, return, and comparison paths. |
| `float` / `float64` alias | Covered | Covered | Covered | Covered | One canonical IEEE binary64 type across semantic analysis, MIR, calls, and ABI lowering. |
| `float32` | Covered | Covered | Covered | Covered | Distinct IEEE binary32 locals, parameters, returns, calls, and per-operation rounding. |
| Contextual float literal rounding | Covered | Covered | Covered | Covered | Literals round directly to their expected binary32/binary64 context; unconstrained literals default to binary64. |
| Float arithmetic | Covered | Covered | Covered | Covered | `+`, `-`, `*`, `/`, negation, increments, and compound assignment use the declared width without fast-math. |
| Float division by zero | Covered | Covered | Covered | Covered | Positive/negative infinity and NaN follow IEEE 754 without integer panic behavior. |
| NaN comparison | Covered | Covered | Covered | Covered | Visible unordered comparison behavior matches; payload bits are not compared. |
| Signed zero | Covered | Covered | Covered | Covered | Zeroes compare equal while the sign remains observable through division. |
| Float parameters, returns, and calls | Covered | Covered | Covered | Covered | F32/F64 ABI values remain in their declared widths, including recursive/general helper paths. |
| Runtime bool locals | Covered | Covered | Covered | Covered | Readonly/writable locals use canonical false/true scalar values. |
| Bool parameters, returns, and calls | Covered | Covered | Covered | Covered | Canonical I8 ABI values 0/1 cross helper boundaries. |
| Bool value short-circuit | Covered | Covered | Covered | Covered | `and`/`or` skip the right operand in value and condition position. |
| Bool eager xor | Covered | Covered | Covered | Covered | Both operands execute left-to-right and produce a canonical bool. |
| `Int::toFloat` | Covered | Covered | Covered | Covered | Canonical signed int64 converts to binary64 with IEEE rounding and no panic. |
| `Float::toInt` | Covered | Covered | Covered | Covered | Binary64 truncates toward zero after explicit finite/range checks. |
| Float-to-int panic | Covered | Covered | Covered | Covered | NaN, infinity, and positive `2^63` produce identical message, stack trace, and status 101. |
| Mixed int/float and float-width rejection | Frontend | Frontend | Frontend | Covered | Semantic diagnostics prevent implicit cross-kind or cross-width values before MIR. |
| PHP float32 boundary | Diagnostic | Diagnostic | Diagnostic | Covered | PHP never emits unknown float width names; exact float64 division uses `fdiv`. |
| Invalid process status panic | Covered | Covered | Covered | Covered | Runtime entry validates `main(): int` and exits 101 on failure. |
| Narrow `?string` seed and flow guards | Covered | Covered | Covered | Covered | `read_line` EOF is distinct from empty string; assignment invalidates or re-establishes non-null facts. |
| `read_line` and repeated buffering | Covered | Covered | Covered | Covered | Raw sidecar stdin covers LF, CRLF, empty lines, buffered subsequent lines, and final unterminated input. |
| `read_file` | Covered | Covered | Covered | Covered | Complete UTF-8 text and Unicode content agree through isolated fixture directories. |
| `write_file` and file side effects | Covered | Covered | Covered | Covered | Create/truncate output is compared byte-for-byte against expected files. |
| `write_stderr` | Covered | Covered | Covered | Covered | Exact stderr bytes with no implicit newline. |
| Checked `sprintf` | Covered | Covered | Covered | Covered | One validated MIR plan covers every accepted conversion, width, alignment, padding, and precision form. |
| Checked `printf` | Covered | Covered | Covered | Covered | Same plan as `sprintf`; exact stdout, void result, and no implicit newline. |
| Format failures | Frontend | Frontend | Frontend | Covered | Dynamic, malformed, unsupported, wrong-arity, and wrong-type formats are rejected before MIR. |
| I/O panic failures | Covered | Covered | Covered | Covered | Missing-file panic preserves exact message, Doria stack trace, and status 101. |
| Windows Unicode output | Unit + CI | Unit + CI | Unit + CI | Covered | Interactive console uses wide writes; redirected handles preserve exact UTF-8 bytes. |
| Per-stream interactivity foundation | Runtime | Runtime | Runtime | Covered | Internal stdin/stdout/stderr detection is independent and is not exposed as a public Doria API. |
| Native compile without execution preflight | Covered | Covered | Covered | Covered | Infinite-loop source compiles but is excluded from executable parity. |
| Native lowering source | MIR | MIR | MIR | Covered | `codegen_cranelift` and `codegen_llvm` consume validated MIR with no HIR or retired-smoke dependency. |
| Complete differential harness | Manifest-driven | Manifest-driven | Manifest-driven | Covered | CI requires a runtime artifact and linker; stdout, stderr, and status are exact. |

## Retirement Gate

Status: Passed through Stage 20 after this branch's full validation gates pass.

All accepted Stage <=20 scalar, string, interpolation, checked-format, text-I/O, ownership, native-class, method, static, constant, and concrete-display lowering passes through typed MIR and shared MIR validation. The interpreter, Cranelift fast profile, and LLVM release profile consume that same MIR; every finite native example is required in the executable manifest with deterministic sidecars where needed; Linux CI memory-checks the ownership-bearing native fixtures, including the static-constructor and `self` paths; and the Stage 7-10 native smoke module remains retired and deleted. Stage 21 borrowing and full definite initialization are next.
