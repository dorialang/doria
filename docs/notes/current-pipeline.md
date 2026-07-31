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
- Stage 23 Slice 1 runtime collections and typed arrays are complete on the current branch. `T[]`, `List<T>`, `Dictionary<K, V>`, and `Set<T>` are owned move types backed by shared collection MIR and `doria-rt`; contextual literals, fixed-length typed arrays, indexing and indexed read-modify-write, insertion-ordered `foreach`, move-in/removal ownership, and Decision 0100's default member surface run through the interpreter, Cranelift, and LLVM. Dictionary `keys`/`values` are readonly, insertion-ordered, `foreach`-only projections and are not storable values.
- Stage 23 Slice 2 is complete on the current branch. The owned `Bytes` move type provides explicit copying conversion to and from `uint8[]`, length, byte indexing and indexed read-modify-write, and byte-wise equality. `read_file_bytes`/`write_file_bytes`/`append_file_bytes`, `read_stdin_bytes`/`write_stdout_bytes`/`write_stderr_bytes`, and text `append_file` use shared validated MIR and `doria-rt`, with exact non-UTF-8 bytes and interpreter/Cranelift/LLVM parity.
- Stage 23 Slice 3 is complete on the current branch. The boxed `dr_mixed` runtime representation stores a tag, class type id when needed, and owned payload; bool, fixed-width integers, floats, string, and concrete classes box into `mixed`, narrow back out through exact `is`, and execute through the interpreter, Cranelift, and LLVM. `?mixed`, `List<mixed>`, `Dictionary<K, mixed>`, and `Set<mixed>` value paths use the same shared MIR/runtime box. Collection/interface/subtype `is` and boxing collections, typed arrays, or `Bytes` into `mixed` remain deferred with stage-named diagnostics.
- Stage 25a Slices 1 through 3 are implemented. The readonly `SharedReference<T>` / `WeakReference<T>` family and the disjoint writable family lower through validated MIR to the interpreter, Cranelift, LLVM, and separate `doria-rt` control structures. `WritableSharedReference<T>` supports class, generic-class, typed-array, `List<T>`, `Dictionary<K, V>`, `Set<T>`, and `Bytes` payloads through owned readonly/writable access objects. One access state is shared by every writable-family handle to an allocation; access objects move through returns, parameters, properties, and collection slots; nullable strong, weak, and readonly/writable access forms remain in-family and lazy; and destruction releases access before strong ownership. Exact conflict panics use the abort-only status-101 path. The Slice 3 prerequisite corrections also support property-rooted indexed move-in, transitive returned collection borrows, and standalone lexical block statements with structured-exit cleanup. Scalar/string payload access and all shared handles through `mixed` remain runtime-pending rather than being given an invented value projection or misrepresented as class pointers. The PHP backend still refuses shared ownership. Stage 25a remains incomplete until Slice 4.
- The parser accepts generalized `parent::member()` and trait-local `self::member` under the two-clocks rule; semantic checking names Stage 34 and Stage 35 respectively and stops those forms before MIR. `Foo::$prop` and `static::` are permanent errors with precise fixes.
- Native remains one target: direct compile/run uses the Cranelift fast profile, while `--release` selects LLVM 18 over the same validated typed MIR.
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

## Next

- Implement the Interactive Line-Input Amendment.
- Stage 25a Slice 4 remains blocked until that amendment is complete.

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
