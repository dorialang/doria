# Native MIR Parity Matrix

Documentation role: working note.

Source of truth for sequencing remains `docs/doria-end-to-end-plan.md`. The durable executable manifest is `crates/doriac/tests/fixtures/native_parity_examples.txt`. The differential test reads that manifest, executes each finite source through the MIR interpreter, Cranelift fast profile, and LLVM release profile, and compares exact stdout bytes, stderr bytes, and process status.

`Covered` means the interpreter, Cranelift, and LLVM consume the same validated MIR and the behavior has focused or manifest-driven triple differential coverage.

This matrix is semantic and correctness authority, not a performance table. The
separate `dorialang/benchmarks` manifest owns timed evidence, treats the
interpreter as an oracle rather than a native competitor, and records Cranelift
and LLVM as distinct targets built by one explicitly selected compiler commit.

Stage 26b Slice 2 extends that separate evidence with compiler scaling and
runtime-subsystem cases. Exact output remains the bridge between the systems;
this parity matrix does not acquire timing, RSS, profiler, or baseline policy.

Decision 0113 Slice 3 adds explicit shared MIR for value-axis membership and
nullable first-position search, reuses the existing first-match removal and
nullable endpoint paths, and registers
`main_stage26_collection_slice3.doria` for exact triple-backend parity. Slice 4
adds backend-neutral in-place collection clearing and registers
`main_collection_clear.doria`; no Decision 0113 member remains behind E0559.

Stage 27 Slice 1 adds nominal unit and backed enum MIR and registers
`main_unit_enums.doria`, `main_backed_enums.doria`, `main_nullable_enums.doria`,
`main_enum_mixed.doria`, and `main_enum_constants_and_defaults.doria`. The
three execution paths compare exact output and preserve enum identity through
nullable values, `mixed`, constants, defaults, properties, and collection values.

Stage 27 Slice 2 adds aggregate payload-enum MIR and registers the seven
`main_payload_enums_*` fixtures. They cover copy and move construction,
nullable presence, exact `mixed` identity, permitted collection value storage,
aggregate calls and returns, and active-case reverse-order cleanup across all
three execution paths.

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
| Standalone lexical blocks | Covered | Covered | Covered | Covered | Nested scopes and shared-access boundaries clean up on fallthrough and every structured exit; panic remains abort-only. |
| Traditional `for` | Covered | Covered | Covered | Covered | `continue` reaches the increment block. |
| Grouped local declarations | Covered | Covered | Covered | Covered | One canonical MIR initializer evaluates once; ordered independent Copy bindings, string retains, and typed nullable-move `null` agree exactly. |
| Stage 27 unit enums | Covered | Covered | Covered | Covered | Nominal inline case tags construct, copy, compare, return, pass, and occupy supported value positions without an enum allocation API. |
| Stage 27 backed enums | Covered | Covered | Covered | Covered | `int` and static-string backing projections agree exactly while runtime equality remains case identity. |
| Stage 27 nullable enums | Covered | Covered | Covered | Covered | Presence is separate from tag zero across locals, parameters, returns, coalescing, and narrowing. |
| Stage 27 enum `mixed` identity | Covered | Covered | Covered | Covered | Boxes retain both the enum mixed tag and exact enum type ID; a different enum never narrows merely because its case tag matches. |
| Stage 27 payload enum construction | Covered | Covered | Covered | Covered | Positional and named arguments evaluate once in source order and initialize central inline layouts in field order. |
| Stage 27 payload enum ownership and cleanup | Covered | Covered | Covered | Covered | Copy and move classification is enum-wide; only active fields drop, in reverse declaration order. |
| Stage 27 payload enum aggregate ABI | Covered | Covered | Covered | Covered | Parameters and returns use the same backend-neutral address-based aggregate contract with function-scoped scratch storage. |
| Stage 27 payload enum collections | Covered | Covered | Covered | Covered | Inline aggregate slots preserve equality, nullability, growth, removal, and final ownership without per-element enum allocation. |
| Stage 28 exhaustive enum match | Covered | Covered | Covered | Covered | Unit and payload cases dispatch by nominal tag through one semantic coverage proof and validated MIR result plan; no runtime exhaustiveness path exists. |
| Stage 28 payload destructuring and ownership | Covered | Covered | Covered | Covered | Copy fields copy/retain, move fields remain readonly borrows, case-only patterns skip projection, and temporary scrutinees live through the selected result. |
| Stage 28 nullable and `mixed` match narrowing | Covered | Covered | Covered | Covered | Presence and exact runtime type identity dominate selected-arm projections without leaking facts across arms. |
| Stage 28 `match (true)` and ternary | Covered | Covered | Covered | Covered | Ordered strict-bool conditions and full right-associative ternary execute one selected branch through shared match lowering. |
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
| Constructor definite initialization | Covered | Covered | Covered | Covered | Decision 0090 merges reachable per-property states, excludes panic-only paths, and rejects incomplete normal exits before MIR execution. |
| Conditional constructor execution | Covered | Covered | Covered | Covered | `main_stage21_conditional_constructor.doria` initializes multiple readonly properties on both branches and compares exact method/destructor output. |
| Class-valued locals, calls, and returns | Covered | Covered | Covered | Covered | Pointer-sized values preserve owning transfers and inferred returned borrows through free-function ABI boundaries. |
| Transitive returned collection borrows | Covered | Covered | Covered | Covered | Generic repository results preserve `$this` provenance through property projection and `Dictionary::get` without acquiring ownership. |
| Readonly shared ownership | Covered | Covered | Covered | Covered | `SharedReference<T>`, `WeakReference<T>`, nullable acquisition, forwarding, generic/property/collection storage, exact `referencedValue` collision projection, weak expiry, and final-strong destruction use a separate non-atomic control block without changing class payload layout. |
| Writable shared ownership and access | Covered | Covered | Covered | Covered | Concrete/generic class, `T[]`, `List`, `Dictionary`, `Set`, and `Bytes` payloads; disjoint strong/weak families; weak-cycle breaking; owned access objects; three exact P1501 conflict reasons; forwarding; bounded stress; and deterministic access-before-strong release share one validated MIR/runtime contract. |
| Decision 0113 Slice 3 collection members | Covered | Covered | Covered | Covered | First-match nullable `List::indexOf`, first-match writable `List::remove`, nullable map value membership, insertion-ordered `Set` endpoints, and ascending `SortedSet` endpoints preserve exact stdout, stderr, and status through one validated MIR contract. |
| Decision 0113 Slice 4 collection clear | Covered | Covered | Covered | Covered | All seven named collections clear in place, invalidate old membership state, survive double clear, and preserve reuse order through one validated MIR statement and runtime reset contract. |
| Runtime panic diagnostic outcome | Covered | Covered | Covered | Covered | Catalogued `P` code, Title Case title, precise Doria source label, `Why`, `Call Path`, stderr bytes, and status 101 remain exact across the durable manifest. |
| `take` ownership transfer | Covered | Covered | Covered | Covered | Transfer invalidates the caller slot and cleanup becomes the callee's obligation. |
| Property loads and Stage 19 assignments | Covered | Covered | Covered | Covered | Shared class metadata supplies checked types and compiler-known offsets. |
| Property-rooted indexed move-in | Covered | Covered | Covered | Covered | Writable paths mutate a contained collection slot, move the element once, and drop a replaced value without replacing the collection property. |
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
| Generalized `parent::` and trait-local `self::` grammar | Frontend | Frontend | Frontend | Covered | Accepted syntax produces Stage 34/35 semantic diagnostics without compiler parser errors; companion LSP coverage is coordinated in `dorialang/doria-language-server`. |
| `internal` member enforcement | Frontend | Frontend | Frontend | Covered | Rejected access never reaches MIR; same-class access covers instance, static, constant, and lifecycle members. |
| `Displayable` native execution | Covered | Covered | Covered | Covered | Statically known conforming classes call ordinary `toString()` MIR exactly once and left-to-right. |
| PHP `Displayable` subset | N/A | N/A | N/A | Covered | Generated private interface invokes Doria `toString` exactly once and never relies on `__toString`. |
| Parser fuzzing | Frontend | Frontend | Frontend | Covered | Bounded CI fuzzing seeds nested strings, braces, malformed expressions, and UTF-8 offsets. |
| String equality and ordering | Covered | Covered | Covered | Covered | Equality is exact-byte and ordering is unsigned byte-lexicographic. |
| String intrinsic measurements and bytes | Covered | Covered | Covered | Covered | Grapheme length, UTF-8 byte length, empty test, copied `Bytes`, and UTF-8-validating `fromBytes` share one validated MIR/runtime contract. |
| Unicode String transforms and predicates | Covered | Covered | Covered | Covered | Unicode whitespace, default and first-grapheme casing, full folding, and boundary-aligned case-sensitive/case-insensitive contains/starts/ends behavior use the shared Unicode implementation. |
| Grapheme-indexed String search and transforms | Covered | Covered | Covered | Covered | Case-sensitive/case-insensitive search, non-overlapping occurrence counting, replacement, split/join, slice, repeat, and padding preserve exact stdout and returned values across all three paths. |
| String intrinsic panic contracts | Covered | Covered | Covered | Covered | Negative slice/repeat/padding inputs and required empty padding preserve exact Title Case panic text and status 101. |
| String echo in int-returning functions | Covered | Covered | Covered | Covered | Statement validity is independent of function return type. |
| Short-circuit conditions with helper calls | Covered | Covered | Covered | Covered | `and`/`or` short-circuit; `xor` evaluates both in order. |
| Process exit boundary | Covered | Covered | Covered | Covered | Only `main(): int` is restricted to `0..125`. |
| Recursion and mutual recursion | Covered | Covered | Covered | Covered | Explicit interpreter frames remove the former 256-frame semantic cap. |
| Return from nested control flow | Covered | Covered | Covered | Covered | Source CFG reachability permits return anywhere and rejects reachable fallthrough. |
| Explicit panic | Covered | Covered | Covered | Covered | Exact structured runtime diagnostic, Doria call path, and status 101 agree. |
| Checked overflow panic | Covered | Covered | Covered | Covered | Addition, subtraction, and multiplication messages agree exactly. |
| Signed negation overflow panic | Covered | Covered | Covered | Covered | Exact catalogue identity, Doria call-path frames, and status 101 agree. |
| Divide-by-zero and signed-division-overflow panic | Covered | Covered | Covered | Covered | Both failure classes keep their distinct deterministic messages. |
| Remainder-by-zero panic | Covered | Covered | Covered | Covered | Exact catalogue identity, Doria call-path frames, and status 101 agree. |
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
| Float-to-int panic | Covered | Covered | Covered | Covered | NaN, infinity, and positive `2^63` produce identical structured diagnostics, call paths, and status 101. |
| Mixed int/float and float-width rejection | Frontend | Frontend | Frontend | Covered | Semantic diagnostics prevent implicit cross-kind or cross-width values before MIR. |
| PHP float32 boundary | Diagnostic | Diagnostic | Diagnostic | Covered | PHP never emits unknown float width names; exact float64 division uses `fdiv`. |
| Invalid process status panic | Covered | Covered | Covered | Covered | Runtime entry validates `main(): int` and exits 101 on failure. |
| Narrow `?string` seed and flow guards | Covered | Covered | Covered | Covered | `read_line` EOF is distinct from empty string; assignment invalidates or re-establishes non-null facts. |
| General nullable scalars and strings | Covered | Covered | Covered | Covered | An explicit presence word and payload cross locals, calls, returns, properties, and statics without backend-specific semantics. |
| Nullable concrete classes | Covered | Covered | Covered | Covered | Null-pointer absence preserves class move ownership and drops only present payloads. |
| `??`, `?->`, null guards, and exact `is` | Covered | Covered | Covered | Covered | `main_stage22_nullable.doria` covers lazy defaults, null-safe calls, path narrowing, matching and incompatible exact tests, and byte-identical output. |
| `mixed` runtime box | Covered | Covered | Covered | Covered | Bare operations are rejected until exact `is` narrowing; bool, numeric, string, concrete-class, nullable, and mixed collection-value paths round trip through the boxed runtime representation. |
| `read_line` and repeated buffering | Covered | Covered | Covered | Covered | Raw sidecar stdin covers LF, CRLF, empty lines, buffered subsequent lines, and final unterminated input. The prompted form writes the prompt exactly, flushes stdout, then reads; the flush also occurs for the default empty prompt, and each call emits its own prompt even when the next line is already buffered. |
| `read_file` | Covered | Covered | Covered | Covered | Complete UTF-8 text and Unicode content agree through isolated fixture directories. |
| `write_file` and file side effects | Covered | Covered | Covered | Covered | Create/truncate output is compared byte-for-byte against expected files. |
| `write_stderr` | Covered | Covered | Covered | Covered | Exact stderr bytes with no implicit newline. |
| Checked `sprintf` | Covered | Covered | Covered | Covered | One validated MIR plan covers every accepted conversion, width, alignment, padding, and precision form. |
| Checked `printf` | Covered | Covered | Covered | Covered | Same plan as `sprintf`; exact stdout, void result, and no implicit newline. |
| Format failures | Frontend | Frontend | Frontend | Covered | Dynamic, malformed, unsupported, wrong-arity, and wrong-type formats are rejected before MIR. |
| I/O panic failures | Covered | Covered | Covered | Covered | Missing-file panic preserves its exact catalogue entry, path-argument label, Doria call path, and status 101. |
| Windows Unicode output | Unit + CI | Unit + CI | Unit + CI | Covered | Interactive console uses wide writes; redirected handles preserve exact UTF-8 bytes. |
| Per-stream interactivity foundation | Runtime | Runtime | Runtime | Covered | Internal stdin/stdout/stderr detection is independent and is not exposed as a public Doria API. |
| Native compile without execution preflight | Covered | Covered | Covered | Covered | Infinite-loop source compiles but is excluded from executable parity. |
| Native lowering source | MIR | MIR | MIR | Covered | `codegen_cranelift` and `codegen_llvm` consume validated MIR with no HIR or retired-smoke dependency. |
| Complete differential harness | Manifest-driven | Manifest-driven | Manifest-driven | Covered | CI requires a runtime artifact and linker; stdout, stderr, and status are exact. |

## Retirement Gate

The durable Stage 26 fixture covers unsorted map/set construction, ascending
iteration, min-first priority removal, deque front/back mutation, and
front-to-back iteration across the interpreter, Cranelift, and LLVM. The Stage
26a fixture covers all four grouped-local prefixes, one-time side effects,
independent writable bindings, shared immutable strings, and nullable empty
move bindings across the same three execution paths.

Status: Passed through Stage 28 Slice 1.

All accepted native scalar, string, interpolation, checked-format, text-I/O, ownership, native-class, method, static, constant, concrete-display, nullable, collection, `Bytes`, boxed-`mixed`, monomorphized generic, and Stage 25a shared-ownership lowering passes through typed MIR and shared MIR validation. The interpreter, Cranelift fast profile, and LLVM release profile consume that same MIR; every finite native example is required in the executable manifest with deterministic sidecars where needed; Linux CI memory-checks the ownership-bearing native fixtures, including readonly collision projection, writable shared class, all writable payload domains, weak-cycle breaking, bounded stress, access lifetime, and stored-access paths; and the Stage 7-10 native smoke module remains retired and deleted. Stage 21 ordinary borrowing and constructor definite initialization and Stage 22 narrowing use the same backend-independent control-flow/dataflow foundation. Stage 24 specializes reachable free functions and instance/static methods once per concrete generic-argument set before any backend consumes the program. Stage 25 specializes generic classes. Stage 25a Slices 1 through 4 provide the two distinct non-atomic control models, per-allocation writable access state, owned access objects, collision projection, exact conflict reasons, and complete parity/tooling closure.
