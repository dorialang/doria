# Current Pipeline

Documentation role: working note. This file prevents duplicated in-flight work. It is not a roadmap; `docs/doria-end-to-end-plan.md` owns the roadmap.

## Recently merged

- PR #75: Stage 17 integration, parity, examples, editor, docs, and CI closure.
- PR #76: Stage 17 naming, I/O-tier, and migration-guidance corrections.

## Active

- Stage 18 full expression interpolation and compiler-known `Displayable` is merged.
- Stage 19 ownership, moves, destruction, and native class layout is complete on the current branch.
- Stage 20 statically resolved instance/static methods, Copy-type static properties, class/top-level constants, `internal` enforcement, and concrete native `Displayable` execution are complete on the current branch. Static access is sigil-free, `self` resolves to the declaring class, and one class-level index rejects cross-kind member-name collisions.
- Stage 20a/20b const-evaluable defaults are complete for Copy scalars and readonly strings across free functions, instance methods, static methods, and constructors through one caller-side MIR splice. Writable Copy scalars remain supported; `?string`, `writable string`, `take string`, and other move/`take` defaults retain explicit temporary diagnostics.
- Stage 21 non-lexical borrowing, returned-borrow elision, and constructor definite initialization are complete on the current branch. Returned-borrow provenance now remains tied transitively to `$this` or one borrowed parameter through property paths, collection indexing, and compiler-known `Dictionary::get`/`List::first`/`List::last` reads. Constructor paths use decision 0090's uninitialized/initialized/maybe-initialized lattice, and shared MIR validation independently enforces the normal-exit and readonly exactly-once invariants.
- Stage 22 general nullable types, `??`, `?->`, exact `is`, flow-sensitive narrowing, and `mixed` static semantics are complete on the current branch. Narrowing reuses the shared CFG/forward-dataflow framework; local and parameter guards preserve dominating nullable-presence proofs in MIR, and nullable scalar, string, and concrete-class values execute through the interpreter, Cranelift, and LLVM.
- Stage 23 Slice 1 runtime collections and typed arrays are complete on the current branch. `T[]`, `List<T>`, `Dictionary<K, V>`, and `Set<T>` are owned move types backed by shared collection MIR and `doria-rt`; contextual literals, fixed-length typed arrays, indexing and indexed read-modify-write, insertion-ordered `foreach`, move-in/removal ownership, and Decision 0100's default member surface run through the interpreter, Cranelift, and LLVM. Decision 0113 amends that surface. Slice 1 implements `containsKey` and the widened `contains`; Slice 2 implements receiver-aware structured fixes, property-call diagnostics, and withdrawn literal-family `::from` guidance; Slice 3 implements `List::indexOf`, `List::remove`, map `containsValue`, and readonly set endpoint properties. Slice 4 `clear()` is next and remains accepted pending before MIR. Dictionary `keys`/`values` are readonly, insertion-ordered, `foreach`-only projections and are not storable values.
- Stage 23 Slice 2 is complete on the current branch. The owned `Bytes` move type provides explicit copying conversion to and from `uint8[]`, length, byte indexing and indexed read-modify-write, and byte-wise equality. `read_file_bytes`/`write_file_bytes`/`append_file_bytes`, `read_stdin_bytes`/`write_stdout_bytes`/`write_stderr_bytes`, and text `append_file` use shared validated MIR and `doria-rt`, with exact non-UTF-8 bytes and interpreter/Cranelift/LLVM parity.
- Stage 23 Slice 3 is complete on the current branch. The boxed `dr_mixed` runtime representation stores a tag, class type id when needed, and owned payload; bool, fixed-width integers, floats, string, and concrete classes box into `mixed`, narrow back out through exact `is`, and execute through the interpreter, Cranelift, and LLVM. `?mixed`, `List<mixed>`, `Dictionary<K, mixed>`, and `Set<mixed>` value paths use the same shared MIR/runtime box. Collection/interface/subtype `is` and boxing collections, typed arrays, or `Bytes` into `mixed` remain deferred with stage-named diagnostics.
- Stage 25a Slices 1 through 4 are implemented and Stage 25a is complete. The readonly `SharedReference<T>` / `WeakReference<T>` family and the permanently disjoint writable family lower through validated MIR to the interpreter, Cranelift, LLVM, and separate `doria-rt` control structures. `WritableSharedReference<T>` executes class, generic-class, typed-array, `List<T>`, `Dictionary<K, V>`, `Set<T>`, and `Bytes` payloads through owned readonly/writable access objects. One access state is shared by every writable-family handle to an allocation; access objects move through returns, parameters, properties, and collection slots; nullable strong, weak, and readonly/writable access forms remain in-family and lazy; and destruction releases access before strong ownership. P1501 carries one of the three exact Decision 0106 conflict conditions as a typed runtime fact. The allocation-free `referencedValue` projection resolves wrapper/payload collisions without changing either ownership count, and durable weak-cycle and bounded-stress fixtures agree across all native paths. Scalar/string payload access and all shared handles through `mixed` remain runtime-pending rather than being given an invented value projection or misrepresented as class pointers. The PHP backend still refuses shared ownership.
- PHP Stream And I/O Completeness Audit — Implemented. The canonical human
  artifact expands `docs/notes/io-surface-audit.md`; the stored 153-row PHP
  inventory and offline guard classify every official Stream Function,
  `streamWrapper`/`php_user_filter` method, `StreamBucket`, wrapper/context/filter
  family, relevant open-stream filesystem/process entry, and Readline boundary.
  Capabilities are preserved through typed owners or explicit deferrals without
  accepting PHP resources, dynamic wrapper/filter registries, global contexts,
  mixed metadata bags, or sentinel outcomes.
- Andrew’s Stream API Completeness Review — Complete. Stream, Readiness, Standard I/O, Blocking Mode, And Performance Model — Accepted (decision 0110). The semantic and performance model is fixed: small byte
  capabilities, owned handles, typed outcomes, first-class non-owning standard
  streams, one readiness/time/cancellation/backpressure substrate, and typed
  domain adapters. Steady-state stream loops have no mandatory per-operation
  allocation or whole-chunk copy; reusable buffers expose safe byte regions;
  concrete adapters remain statically specializable; readiness storage is
  reused; and synchronous programs initialize no async executor/task machinery.
- Stage 36a — Scheduled. Stage 36a Public Spellings — Deferred. **Stage 36a — Not Implemented**: no stream interface, blocking-mode type,
  non-blocking result, readiness waiter, file handle, process API, or first-class
  standard-stream accessor is currently executable. Exact names reopen through
  decision 0110's bounded appendix before implementation, without reopening the
  accepted semantic or performance architecture. Stage 36a owns the initial
  cross-platform stream benchmark and memory-regression gate; Stage 43 later
  continues and broadens it.
- The parser accepts generalized `parent::member()` and trait-local `self::member` under the two-clocks rule; semantic checking names Stage 34 and Stage 35 respectively and stops those forms before MIR. `Foo::$prop` and `static::` are permanent errors with precise fixes.
- Native remains one target: direct compile/run uses the Cranelift fast profile, while `--release` selects LLVM 18 over the same validated typed MIR.
- Both native profiles keep loop-body stack use constant. Every LLVM scratch slot — the flags and out-parameters the dictionary, set, list, string-search, and parse lowerings use to return `?T` — is allocated in its function's entry block, because LLVM treats an allocation anywhere else as a dynamic stack allocation that moves the stack pointer when it executes and is not reclaimed until the function returns. Cranelift stack slots are function-scoped by construction and were never affected. `llvm_mir_tests` asserts the placement on the module the backend emits before optimization, and `native_mir_parity_tests` runs those loop bodies two million times on every enabled profile and asserts a clean exit with exact output.
- Ordinary expression interpolation of primitive/string values lowers through the existing ordered MIR string and display operations consumed by all three execution paths.
- Native classes now cover construction, property initialization/access, class-valued locals/arguments/returns, `take` transfer, lifecycle bodies, recursive destruction, and deterministic normal structured-exit cleanup through the interpreter, Cranelift, and LLVM.
- Concrete `Displayable` conversion lowers to an ordinary direct `toString()` method call for interpolation, `.`, `echo`, and `%s`; interface-typed values and general interface dispatch remain deferred.
- Stage 23a named arguments are complete on the current branch. Call arguments carry an optional parameter name through the AST and Doria IR, and one shared binding step maps them to parameters before type inference for free functions, instance methods, static methods, and constructors. Positional arguments may precede named ones but not follow them; named arguments reorder freely and may skip a defaulted parameter, including a middle one, whose folded default the caller-side splice fills in by parameter position. Arguments evaluate in source order regardless of the parameter they bind to: a reordered call evaluates each observable argument into an ordered temporary and then assembles the callee vector in parameter order. Scalar and string temporaries remain borrowed; class, collection, `Bytes`, `mixed`, and their nullable move forms use tracked owned temporaries that move into the call. A borrowing parameter causes the caller to drop that temporary once after the call, while a `take` parameter transfers the obligation to the callee. The shared MIR validator, interpreter, Cranelift, and LLVM paths enforce the same rule, so ownership/borrow checking, side effects, and destruction all preserve the written program. Parameter names are part of a callable's public API; language intrinsics stay positional-only.
- Stage 23b program entry arguments are complete on the current branch. `main(List<string> $args)` joins the parameterless forms for both return types; semantic checking rejects every other entry shape (a second parameter, a non-`List<string>` type, and `writable`/`take`), and shared MIR validation independently enforces that the parameter is a borrowed `List<string>`. `doria-rt` entry glue builds the owned list from the platform argument vector, lends it to `main`, and releases it afterwards — using `GetCommandLineW`/`CommandLineToArgvW` on Windows, because the ANSI `char**` handed to C `main` there cannot represent every argument. The executable path is stripped, so `$args[0]` is the first real argument and a no-argument invocation yields an empty list. Non-UTF-8 arguments panic rather than entering the program. `doriac run <source> [--release] [-- <program args>...]` forwards arguments after `--`.
- Stage 23c sequence fills are complete on the current branch. `[value; count]` constructs runtime-sized `T[]` or one-shot-filled `List<T>` values, defaulting to `List<T>` without context. The value and count each evaluate once in source order; Copy scalars are bit-copied and string handles are retained once per slot. Constant-negative counts are rejected, dynamic-negative counts panic with `fill count is negative`, and zero produces an empty sequence. Move-type fills remain gated on Stage 35 `Cloneable`; `Set` and `Dictionary` fills remain intentionally unsupported.
- Stage 24 generic free functions, instance methods, and static methods are complete. Type arguments are inferred through the existing positional/named argument binding, with a typed declaration supplying expected-result context when arguments alone cannot resolve a parameter. MIR monomorphization emits one reachable specialization per distinct concrete argument set and deduplicates repeated uses; its kinded specialization key leaves compile-time value parameters as an additive future extension. Compiler-known constraints are checked at instantiation; user-defined interface constraints remain deferred to Stage 35.
- Stage 25 generic classes are complete on the current branch under decision 0105. `Name<T>` is structurally typed and monomorphized into a distinct concrete class layout and method set per argument list, including nested instantiations. Every class instantiation remains an owned move type; substituted field types specialize its field handling and drop glue. Type parameters are invariant, while default type arguments, compile-time value arguments, and runtime generic reflection remain fenced.
- The Diagnostic Experience Foundation is implemented under decision 0108.
  Compiler diagnostics now carry stable severity/kind metadata, Title Case
  titles, multi-source labels, explanations, repeated notes/help,
  applicability-classified multi-edit fixes, cause identity, documentation, and
  protected developer details. Human/concise stderr and versioned JSON stdout
  are rendered centrally with exact-duplicate suppression. Coordinated
  language-server and website consumers use the structured model.
- The String API Decision Amendment is implemented under amended decision 0103.
  `$string->` is reserved for intrinsic measurements and views; `String::` owns
  every string-specific operation; and the former public `str_*` family is
  removed. `length` is defined in Unicode grapheme units, while `byteLength`
  reports UTF-8 bytes.
- The String API Completeness Audit Against PHP is implemented as a checked-in
  181-row inventory covering the official core string, mbstring, and current
  released grapheme capabilities.
- Andrew approved the audit's recommended path on 2026-07-31. Decision 0103's
  v1 inventory now includes the symmetric ignore-case search family,
  first-grapheme casing, and occurrence counting; every other proposed item is
  explicitly deferred under its recorded dependency or domain.
- The Minimum String Runtime Surface is implemented. Intrinsic
  `length`/`byteLength`/`isEmpty`/`bytes` and the selected trimming, casing,
  predicate, search, replacement, split/join, slice, repeat, padding, and
  `fromBytes` companion operations lower through validated MIR to the
  interpreter and the shared `doria-rt` ABI used by Cranelift and LLVM.
  Unicode 17.0 data drives locale-independent grapheme, whitespace, casing,
  and full-fold behavior. Grapheme/code-point views remain pending on the
  traversal protocol, and ordering comparisons remain pending on `Ordering`.
  The approved ignore-case search family, first-grapheme casing, and
  occurrence counting are included in this executable surface.
- The Unified Doria Diagnostic Presentation And Runtime Outcome Foundation is
  implemented under decision 0109. The compiler-owned `Diagnostic` is the sole
  public representation for compilation findings and runtime outcomes.
  Interpreter, Cranelift, LLVM, PHP compatibility, standalone executables, and
  `doriac run` preserve catalogued `P` codes, source labels, Doria call paths,
  and status-101 abort-without-cleanup semantics. Human output uses the global
  `Where`/preview/`Why` grammar and `Call Path`; concise and JSON remain
  projections of the same facts. Future unhandled checked errors are bound to
  this foundation but remain unimplemented.
- The durable manifest supports raw stdin, isolated seeded files, declared program arguments, and exact interpreter/Cranelift/LLVM stdout, stderr, status, generated-file, and class-lifetime comparison.

- The Interactive Line-Input Amendment is implemented. `read_line(string $prompt
  = ""): ?string` is one compiler-known function with an optional parameter and
  an inclusive arity range, not an overload pair. One canonical MIR operation
  owns the ordering contract — evaluate the prompt exactly once, write it
  exactly with no added newline, flush stdout, then read one line — and the
  interpreter, Cranelift, LLVM, and PHP compatibility backend all consume it.
  `read_line()` remains valid and lowers with the canonical empty-string default,
  which still performs the pre-read flush. Line discipline, EOF, and buffered
  remainders are unchanged. A closed stdout pipe during the prompt write or
  flush exits with status 0 without reading stdin; other output failures use
  P1407, read failures P1403, invalid UTF-8 P1404, and allocation failure P1206,
  all through the decision 0109 foundation. The flush substrate now reports the
  same success/broken-pipe/other-failure vocabulary as writes.

## Current collection and local-declaration milestone

- Stage 26 — Complete. `SortedDictionary`, `SortedSet`, min-first
  `PriorityQueue`, and ring-buffer `Deque` consume explicit validated MIR and a
  shared private runtime ABI across the interpreter, Cranelift, and LLVM. PHP
  compatibility uses generated semantic helpers rather than host collection
  ordering. Set iteration is readonly and non-consuming construction/algebra
  preserve their sources.
- Stage 26a — Complete. Grouped inferred/typed readonly/writable locals preserve
  one initializer in AST/HIR and canonical validated MIR. The initializer runs
  once, targets initialize left to right, scope insertion is atomic, Copy values
  remain independent, strings retain their immutable handle per binding, and
  move values are rejected except for explicitly typed nullable literal-null
  initialization. Interpreter, Cranelift, LLVM, and PHP compatibility agree;
  grouping creates no runtime object.
- Stage 26b — Performance Baseline Foundation — Complete. Decision 0112 is
  accepted and all three slices are complete: the manifest/provenance foundation,
  opt-in compiler phase and structural report, compiler scaling generators,
  compiler/generated/runtime/diagnostic matrix, process resource counters,
  separate Callgrind/DHAT adapters, candidate evidence, and exact structural
  baseline are in place. Slice 3 adds peer sources, the exact native acceptance
  policy, and controlled measurement and promotion workflows; timing thresholds
  still require separate review of eligible evidence, and Slice 2 makes no
  optimization claim.
  Slice 3 Part 1 is delivered: C, C++, Rust, and PHP peers for the comparative
  generated-program and runtime-subsystem cases, peer fairness and
  semantic-equivalence records enforced when the manifest loads, two controlled
  candidate sessions, and a timing threshold proposal. It accepts no threshold
  and promotes no baseline. Both Doria backends are measured. The proposal's
  finding is that seventy-nine of eighty case and target pairs are dominated by
  process startup rather than by their workload, so those cases cannot carry a
  timing threshold until their workloads are scaled. An earlier write-up
  reported `string_search` on Cranelift at 6.6 times its floor; that result is
  withdrawn. Deterministic runtime construction and full compiler/runtime
  identity now close the reproducibility defect. The exact `1.30` native
  acceptance rule is bound, and hot workloads require both five times their
  target startup floor and at least 25 ms. No controlled physical-host Linux runner or
  eligible physical-host session is currently configured, so no native
  acceptance matrix or Doria timing baseline can be promoted. Docker, WSL,
  containers, and virtual machines remain valid for engineering and correctness
  evidence, but not release claims. Controlled timing, verified affinity,
  Callgrind, DHAT, hardware counters, and cross-platform timing baselines are not
  stage gates. Missing eligible-runner evidence does not halt compiler stages.
- Stage 26b — Complete.
- Stage 26b Slice 1 — Complete.
- Stage 26b Slice 2 — Complete.
- Stage 26b Slice 3 — Complete.
- Measurement Status: Pending Available Runner.
- Decision 0113 Slice 1 — Complete.
- Decision 0113 Slice 2 — Complete.
- Decision 0113 Slice 3 — Complete.
- Decision 0113 Slice 4 — Next.
- Stage 27 — Sequenced After Decision 0113; No Performance-Evidence Dependency.
- Stage 35a — Optimizer Contracts, Dispatch, And Escape Audit — Scheduled.
- Stage 36a — Scheduled, Not Implemented.

## Do not duplicate

- Stage 17 I/O and formatting work from PRs #75 and #76.
- ROADMAP-style planning outside the end-to-end plan.

## Deferred

- General interface declarations and conformance until Stage 35.
- Runtime-initialized and owned statics until separately accepted lifetime/concurrency decisions.
- Parent lookup/dispatch until Stage 34 and trait composition until Stage 35; their accepted grammar is already represented.
- Growable/slice/search `Bytes` members until a future method-surface decision.
- `match` narrowing until Stage 28, hierarchy `is` until Stage 34, and interface `is` until Stage 35.
- Collection/interface/subtype `is`, plus boxing collections, typed arrays, or `Bytes` into `mixed`, until their authored stages.
- Variadic/spread user parameters stay deferred from Stage 23a as their own slice, per decisions 0095 and 0098.
