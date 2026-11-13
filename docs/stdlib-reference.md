# Standard library reference — the planned surface

Constructor parameter roles are core-language syntax, not standard-library
members. `parameter` is a keyword and `param` is not an alias. Attribute-class
schema binding continues to include promoted, override, and constructor-only
callable parameters without executing their constructors.

> Documentation role: the at-a-glance catalogue of Doria's **core** and **standard-library** surface — every companion, interface, collection, free function, and `Doria\Std\*` module, with its purpose and known member surface. This is the *inventory*; the end-to-end plan §9 owns the *direction and rationale*, and each decision record owns the *precise contract*. Both are linked from every entry. This is the **planned** surface: some members are settled in a record, others are marked *(surface TBD in …)* until their decision is authored. It grows as decisions land — keep it in sync when a stdlib record is authored or amended.

Two layers (plan §9): **core** (no I/O, always available) and **std** (hosted under the reserved `Doria\Std` namespace, per the namespace-model decision). Both are written in Doria as early as self-hosting allows.

## Edition 2026 Prelude

The prelude is an explicit compiler-owned table, not a wildcard import. Its
complete edition-2026 inventory is:

```text
Displayable Error Comparable Hashable Equatable
Int Int8 Int16 Int32 Int64 UInt8 UInt16 UInt32 UInt64
Float Float32 Float64 Bool String Bytes
List Dictionary SortedDictionary Set SortedSet PriorityQueue Deque
SharedReference WeakReference WritableSharedReference WritableWeakReference
ReadonlySharedReferenceAccess WritableSharedReferenceAccess
```

`Comparable`, `Hashable`, and `Equatable` are ordinary conveniences that a
current-namespace declaration or explicit import may shadow. The other listed
compiler-owned identities retain their existing reserved-name rules. Prelude
changes are edition changes.

Lowercase primitive spellings and `T[]` are language type syntax rather than
prelude aliases. Built-in free functions such as `read_line`, formatting and
byte I/O, and `panic` are language intrinsics; they resolve before namespace
lookup and are not prelude entries. Domain APIs under `Doria\Std`, including
the six exact `Doria\Std\Io` checked-error identities, require their canonical
qualified name or an explicit import. The compiler does not implicitly import
`Doria\Std\Console`, `Doria\Std\Math`, or any future standard-library module.

---

## Core layer (no I/O, always available)

### Primitive companions
Every primitive has a companion, and the set is a **complete, symmetric matrix** — not a per-type ad-hoc collection (decision 0104). The uniform v1.0 baseline: `parse(string): ?T` on every scalar companion (int family, float family, `bool`); `String` is the exclusive owner of accepted string-specific operation vocabulary instead of `parse` because `string` is the parse *domain* (0103); display is uniform through the display path, so no companion has `toString`. Absences are deliberate — a member not meaningful for a primitive is a documented N/A, one not in v1.0 is a named v1.0+ furnishing (`MIN`/`MAX`, `Bool::toInt`). Decision 0103's reviewed inventory records both executable operations and accepted follow-up furnishings. Details: decisions 0013/0016 (numeric types), 0042 (conversions), 0095 (`pow`), 0096 (interface conformance), 0103 (canonical String API), 0104 (completeness); §4.6 (strings).

- **`Int`, `Int8`/`Int16`/`Int32`/`Int64`, `UInt8`/`UInt16`/`UInt32`/`UInt64`** — `Int::parse(string): ?int`, `Int::toFloat(int): float`, `Int::pow(...)`, wrapping arithmetic (`wrappingAdd`/`wrappingSub`/`wrappingMul`), and per-width checked conversion families such as `Int32::from` (panics on overflow) / `Int32::tryFrom` (returns `?int32`). Each accepts one fixed-width integer expression; their callable declaration surface is TBD with conversion-overload work rather than published as an untyped Doria parameter.
- **`Float`, `Float32`/`Float64`** — `Float::parse(string): ?float`, `Float::toInt(float): int` (checked, panics on NaN/out-of-range), `Float::pow(...)`. `float` is neither `Hashable` nor totally `Comparable` (0096).
- **`Bool`** — `Bool::parse(string): ?bool` (returns `true`/`false` for exactly `"true"`/`"false"`, case-sensitive, no whitespace tolerance; `null` otherwise). No `MIN`/`MAX`/`pow`/wrapping (N/A for a two-valued type); `Bool::toInt` is a named v1.0+ furnishing (0104).
- **`String`** — executable intrinsic properties are `$s->length` (Unicode grapheme clusters), `$s->byteLength` (UTF-8 bytes), `$s->isEmpty`, `$s->bytes` (copy in v1.0), and `$s->byteRegion`, a readonly `ByteRegion` view of the UTF-8 bytes that borrows the string without copying (native backends; PHP output rejects string intrinsic properties). Pass it to any byte writer to write text; use the region's `getRegion` for a byte range. Executable companion operations are trimming/casing (`trim`, `trimStart`, `trimEnd`, `lower`, `upper`, `lowerFirst`, `upperFirst`); predicates (`contains`, `startsWith`, `endsWith`, `equalsIgnoreCase`, `containsIgnoreCase`, `startsWithIgnoreCase`, `endsWithIgnoreCase`); grapheme-indexed search (`indexOf`, `lastIndexOf`, `indexOfIgnoreCase`, `lastIndexOfIgnoreCase`); `countOccurrences`; `replace`; `split`/`join`; `slice`; `repeat`; `padStart`/`padEnd` (the padding text is optional and defaults to one space); and UTF-8-validating `fromBytes` (decision 0103). The `$s->graphemes` / `$s->codePoints` views await the public traversal protocol, and `compare` / `compareIgnoreCase` await their String ordering implementation. There is no public `str_*` family, `$s->chars`, integer string indexing, or duplicate instance-method spelling.

### Value interfaces (core contracts)
Details: decisions 0096 (primitive conformance), 0134 (interfaces, traits, value
contracts, and public iteration), and 0079 (`Displayable`).

- **`Comparable<T>`** — `compare(T $other): Ordering`, over the core enum **`Ordering { Less, Equal, Greater }`** (decision 0095). There is no `<=>` operator.
- **`Equatable<T>`** — readonly `equals(T $other): bool`; explicit value-equality
  conformance for `==`/`!=`. Nonconforming class equality remains identity.
- **`Hashable`** — readonly `hash(): uint64` for Dictionary/Set keys. Equal values
  must yield equal hashes. Authors must preserve equality/hash-participating
  state while a key is stored, including shared or external state; owned-key
  storage is not a compiler guarantee of transitive immutability. Violations
  must not compromise memory safety. Hashes are not a persistent cross-version
  format. Hash tables add private per-process keyed mixing.
- **`Displayable`** — `toString(): string`; Doria's answer to `__toString`, drives interpolation / `.` / `echo` (§4.6, 0079).
- **`Cloneable`** — readonly nonthrowing `clone(): self`; author-provided explicit
  duplication returning an independently owned value of the same dynamic class.
  Doria does not synthesize, implicitly invoke, or promise universal deep cloning.
- **`Error`** — the compiler-known Move interface for checked errors. Conformance
  is explicit through `implements Error` and requires an externally accessible
  readonly stored `string $message`; a promoted constructor property satisfies
  the contract. The erased value exposes readonly `message` and uses identity
  equality. A class extending an Error-conforming class remains conforming: a
  parent `throws` contract, catch, and typed `toThrow` inspector cover
  descendants while preserving the concrete Error identity. Stage 35 Slice 2
  executes Error subinterfaces and all six shared/weak/access families with
  interface payloads. Decisions 0119 and 0130. Decision 0134 unifies its erased carrier with general interface
  values, permits Error subinterfaces, and retains the stored-message rule as a
  narrow compiler-known exception rather than accepting general interface
  properties.

Stage 35 Slice 1 checks these canonical declarations, inherited requirements,
and trait-free nominal conformance, including generic concrete specialization.
Slice 2 executes interface values and erased calls, preserving readonly,
writable, and take contracts through nullable, mixed, stored, and shared values.
Error uses the general interface carrier; Displayable invokes the selected
requirement once in every display context. Slice 3 executes `Ordering`, core
value operations, Copy-or-Cloneable preserving collection operations, and public
iteration. Concrete and constrained operations retain static selection;
deliberate erasure uses the existing interface slots. Generated calls preserve
the full automatic-effect profile and clean partial owned results on failure.
Iterator carriers can retain a readonly source through a promoted `borrow`
constructor parameter; neither the carrier nor an element borrow can outlive
that source or its access lease. Slice 4 supplies these same contracts through
checked trait composition, including selected aliases, composer-bound `self`,
and retained iterator-source provenance. Primitive interface erasure remains
rejected (E0760); E0493, E0758, and E0759 are reserved.

Primitives conform to `Equatable`/`Comparable`/`Hashable` by compiler-known conformance and satisfy generic constraints with no boxing (0096).

### Shared ownership
The escape hatch when single ownership does not fit (caches, graphs, back-references). Details: §3.3, decision 0106. Stage 25a Slices 1 through 4 implement the grammar/type model, both disjoint families, runtime-checked access guards, collision projection, complete payload matrix, backend parity, and tooling integration.

- **`SharedReference<T>`** — an owning reference that may share responsibility for keeping one value alive with other `SharedReference<T>` values. Constructed with `shared new T(...)`. `share()` creates another owning reference (distinct from `Cloneable`'s `clone()`, which duplicates the value); `createWeakReference()` derives a `WeakReference<T>`; gives direct readonly access to `T`. Reference counting is the implementation mechanism; the source-level model is ownership and lifetime responsibility.
- **`WeakReference<T>`** — a non-owning reference to a shared value; it does not keep the value alive. `acquire()` returns `?SharedReference<T>` — a live owner, or `null` once the last owner is released (breaks cycles).
- **`WritableSharedReference<T>`** — a shared owning reference permitting runtime-checked writable access, built with an ordinary ownership-taking constructor (`new WritableSharedReference(new T())`). `share()` returns another `WritableSharedReference<T>`; `createWeakReference()` returns a `WritableWeakReference<T>`. It never forwards direct access to `T`: `acquireReadonlyAccess()` / `acquireWritableAccess()` return the access objects below, and overlapping incompatible access causes a clear panic (the one place runtime borrow checks appear).
- **`WritableWeakReference<T>`** — the non-owning form of `WritableSharedReference<T>`; `acquire()` returns `?WritableSharedReference<T>`, so breaking a cycle in a writable shared graph retains the writable capability.
`SharedReference<T>` also carries a compiler-known readonly `referencedValue` projection: compiler-known members win on the direct receiver, everything else forwards transparently to `T`, and `referencedValue` keeps a colliding payload member reachable (`$document->referencedValue->share()`). It never copies, moves, or consumes the value.

Writable access conflicts retain the stable P1501 identity and identify the exact
failed transition through a structured `conflictReason`: readonly-to-writable,
writable-to-readonly, or writable-to-writable. Access objects release that access
registration before releasing their owning claim.

The readonly family accepts class allocations, including an interface view of
the concrete class: `SharedReference<I> $value = shared new Implementation();`.
All six families retain the same allocation and invariant interface view.
`SharedReference<int>` and `shared new List<int>()` remain rejected. The writable
family also accepts supported owned collection move types, because its access
objects forward member and indexed operations (`$access[0] = 10`). Interface
support adds no container or wrapper covariance and no cross-family conversion.

The readonly and writable families are disjoint: no conversion exists either way between `SharedReference<T>` and `WritableSharedReference<T>`, the weak forms preserve their family, and no readonly-family and writable-family handle ever refer to the same allocation. All `WritableSharedReference<T>` handles to one allocation share a single runtime access state.

- **`ReadonlySharedReferenceAccess<T>` / `WritableSharedReferenceAccess<T>`** — temporary access objects returned by `WritableSharedReference<T>`; member and indexed access forward to the underlying `T`. The readonly form permits only readonly operations, the writable form both; neither may move out or consume `T`. Each retains an owning reference to the allocation. They are move types — returnable, storable, passable — but never directly constructible by user code, and the access releases deterministically when the object is destroyed (moving it transfers that obligation).

A standalone `{ ... }` block (decision 0107) is the direct way to end an access
object's lifetime before the surrounding function continues.

### Iteration
- **`Iterable<T>`** — readonly `iterator(): Iterator<T>`.
- **`Iterator<T>`** — readonly `hasCurrent(): bool`, readonly `getCurrent(): T`, and
  writable `advance(): void`. A borrowed iterator carries a compiler-owned
  nonescaping readonly source loan; `getCurrent()` borrows one element and
  `hasCurrent()` keeps nullable elements distinct from exhaustion. User-defined
  Stage 35 iteration is value-only and every binding remains explicitly typed.
  Built-in collections retain their optimized compiler-internal plans and exact
  sequence-index/dictionary-key roles; deliberate erasure alone uses the public
  protocol carrier.

Decision 0134's `borrow` receiving mode on an iterator's promoted constructor
parameter retains readonly source access instead of ownership. Cursor state
remains independently owned; releasing the cursor does not destroy its source.
This is an iterator-carrier facility, not general borrowed class fields.
Taking such a carrier into a newly constructed owner's owning property forwards
its existing source loans through that owner until cleanup. It does not turn
the property into a borrowed field or allow the owner to outlive those sources.

### Ranges and math basics
- **Range types** — `a..b` (inclusive) / `a..<b` (exclusive-end); `int` endpoints; used with `foreach` (SPEC control flow).
- **`math` basics** — scalar math functions (the geometry/vector types live in `Doria\Std\Math`, below).

### Enums
- **Unit enums** — nominal inline Copy values selected as `Type::Case`; equality compares exact case identity and no enum container allocation occurs.
- **Backed enums** — exactly `int` or `string` backed; the readonly `value` property exposes the associated value without converting the enum itself.
- **Payload enums** — inline nominal values constructed with positional or named case arguments. They are Copy only when every case payload is Copy, otherwise move; equality compares case plus active fields, and payload observation belongs to `match`.

Enums are not implicitly displayable and do not receive `name`, `cases`,
`from`, `tryFrom`, automatic hashing, or automatic ordering APIs.

### Built-in free-function layer
Regularized `snake_case` functions for capabilities without one natural owning type. Type-coupled vocabulary belongs to the corresponding companion. Details: decision 0074 (formatted I/O), §9.1 charter, and decision 0103 (String boundary).

- **Formatting:** compiler-known `sprintf` returns `string` and `printf` returns `void`. Each takes a literal `string $format` first, followed by the typed operands required by that format; this intrinsic-only tail is not an untyped userland parameter declaration. Specifiers: `%s %d %f %.Nf %x %X %o %b %%`, width / `-` / `0` flags.
- **Text I/O:** `read_line(string $prompt = ""): ?string` writes the prompt exactly (no newline added), flushes stdout, then reads one line; the flush happens even for the default empty prompt, `null` is EOF and `""` is a blank line. `read_file(string): string` reads validated UTF-8; `write_file`, `append_file`, and `write_stderr` return `void`.
- **Ambient I/O:** `read_line` and `read_file` carry exact ambient `Doria\Std\Io\IoError` and `Doria\Std\Io\InvalidUtf8Error`; text writes carry ambient `Doria\Std\Io\IoError`. Ambient failures propagate automatically and may be caught explicitly where the program has a recovery strategy. They are not swallowed or converted to panic, and an unhandled Error reports R1000/status 70.
- **Output statement:** `echo` (the single output spelling — `print` is rejected) carries ambient `Doria\Std\Io\IoError` without adding it to the source `throws` obligation.
- **Time:** `get_time`.
- **Meta:** `function_exists("name")` — const-evaluated compile-time predicate for guarded/polyfill declarations.

Binary I/O uses `read_file_bytes(string): Bytes`, `write_file_bytes(string, Bytes): void`, and `append_file_bytes(string, Bytes): void` per record 0091, plus `read_stdin_bytes(): Bytes` and `write_stdout_bytes(Bytes)`/`write_stderr_bytes(Bytes)` per record 0101. Each carries ambient `Doria\Std\Io\IoError`; byte arguments are readonly borrows and no binary read performs UTF-8 validation. The stream-object tier moves to `Doria\Std\Io` after Stage 29. There is deliberately no `write_stdout(string)` — `echo` is the sole stdout text writer (0101). `sprintf` remains non-I/O and nonthrowing; `printf` carries ambient `IoError`.

---

## Collections (core-language move types)
Owned move types with a growable/fixed distinction. The complete family and naming are settled in **decision 0092**; the method surface originates in **decision 0100** and is completed by **decision 0113**. All four Decision 0113 slices are implemented: the renamed/widened membership surface, receiver-aware diagnostics, value search/removal, set endpoints, and writable in-place `clear(): void` execute. Stage 30g adds the accepted higher-order surface to `List<T>` alone. A bare name is the default (hash / insertion-ordered) collection; the `Sorted` prefix is the comparison-ordered variant. Cross-cutting rules (0100): reads are `readonly`, mutators `writable`; ingestion moves values in; `$l[i]`/`$d[k]` reads **assert presence and panic** on absence while the `?T` methods are the safe path; removal hands the owned element back; mutators return `void` (userland fluent APIs stay a decision-0088 capability).

Sequence literals also have the repeat form **`[value; count]`** (decision 0102). It contextually constructs either a runtime-sized `T[]` or a one-shot-filled `List<T>` and defaults to `List<T>` without an expected type. `value` is evaluated once before the runtime-`int` `count`; zero produces an empty sequence, a constant-negative count is rejected, and a runtime-negative count panics with `fill count is negative`. Copy values use the direct fill path; a Move value must implement Decision 0134's `Cloneable`, is evaluated once, and is cloned once per output slot in ascending order. The form is sequence-only: it does not construct `Set` or `Dictionary`.

- **`T[]`** (typed arrays) — contiguous, fixed-length-after-creation; `length` property, indexing (panics OOB), `get(int $index): ?T` (`null` when the index is out of range), `foreach`, and `contains`; the engine-grade buffer. Its optional first `foreach` binding is a readonly zero-based canonical-`int` sequence index. `[value; count]` is its only runtime-sized constructor. Slicing is a future addition.
- **`Bytes`** — owned mutable byte buffer for binary work; `uint8[]`↔`Bytes` interconvert only through explicit `Bytes::fromArray`/`->toArray` (copy in v1.0). Executable members are `length`, indexed `uint8` reads/writes and RMW, and byte-wise `==`/`!=`. Decision 0110's Stage 36a appendix adds executable `Bytes::allocate(int $length): Bytes` (zero-initialized), readonly `getRegion(int $offset, int $length): Doria\Std\Io\ByteRegion`, and writable `getWritableRegion(int $offset, int $length): Doria\Std\Io\WritableByteRegion`. Growable/search members remain outside that appendix; the complete stream foundation is still in progress.
- **`Doria\Std\Io\ByteRegion` / `Doria\Std\Io\WritableByteRegion`** — Stage 36a's executable bounded borrowed byte views, not owned copies. Both expose readonly `int length`, indexed `uint8` reads, and readonly `getRegion(int $offset, int $length): ByteRegion`. The writable region also permits indexed mutation through a writable binding and writable `getWritableRegion(int $offset, int $length): WritableByteRegion`, plus two bulk copies: writable `copyFrom(ByteRegion $source): int` copies the leading bytes of another region and returns the count, the smaller of the two lengths; writable `copyWithin(int $offset, int $length, int $destination): void` moves bytes inside the region, the ranges may overlap, and invalid bounds panic (P1304, source range first). Both lower to one memmove on every backend. Readonly views are borrowed Copy carriers; writable views are borrowed move carriers. Nullable byte containers and regions use narrowing or null-safe `?->` member access without changing source loans or writable authority. Bounds, source lifetime, and conflicting access are checked. No region operation allocates, copies bytes, or retains a runtime reference count. The complete stream foundation is not delivered yet.
- **`List<T>`** — the everyday growable sequence and default workhorse: `add`, `insertAt`, `removeAt` (returns the owned element), `pop` (`?T`), `get(int $index): ?T` (the element, or `null` when the index is out of range; `$list[$index]` panics instead), `contains`, `indexOf(T): ?int` (first equal position, `null` when absent), writable `remove(T): bool` (first equal element only), writable `clear(): void` (in-place emptying), `first`/`last` (`?T` properties), `count`, and `isEmpty` (properties). Stage 30g implements `map<U>(function(T): U $transform): List<U>`, preserving `filter(function(T): bool $predicate): List<T>`, and writable-accumulator `reduce<A>(take A $initial, function(writable A, T): void $reducer): A`. Each also accepts the corresponding `writable function writable(...)` callback through an exclusive function-value borrow; readonly callbacks use readonly borrows, callbacks never escape, and once callbacks are rejected. The source is visited in insertion order and remains unchanged. `map` owns its new results and supports Move results; Decision 0134 widens `filter` from Copy to Copy-or-Cloneable elements, invoking author-provided clone for retained Move values; `reduce` owns its initial accumulator and supports Copy or Move accumulators. Each method throws exactly the checked Errors declared by its callback's structural function type. Checked failure destroys the partial result or accumulator before propagating. No other collection family receives these algorithms in Stage 30.
- **`Dictionary<K, V>` / `SortedDictionary<K, V>`** — `get` returning `?V`, `set`, `remove` returning `?V`, `containsKey` (key membership), executable O(n) `containsValue(V): bool` (value membership), writable `clear(): void` (in-place emptying), `keys`/`values` (`foreach`-only projections, not storable in v1.0), and `count`/`isEmpty` (properties). `Dictionary` iterates in insertion order, `SortedDictionary` by ascending `Comparable` key. The optional first main-loop binding is the actual readonly key, never an ordinal; values may be writable through the main iteration form. Projections remain value-only.
- **`Set<T>` / `SortedSet<T>`** — `::from`, `add` (`bool`), `remove` (`bool`), `contains`, `union`, `intersect`, `difference`, writable `clear(): void` (in-place emptying), executable readonly `first: ?T` / `last: ?T` properties, and `count`/`isEmpty` (properties). `Set` endpoints follow insertion order; `SortedSet` endpoints are the ascending minimum and maximum. All endpoints are O(1), return `null` when empty, and borrow the retained element. Iteration is readonly: replacement is remove plus add. Existing-source construction and algebra preserve every input and require Copy-or-Cloneable elements.
- **`PriorityQueue<T>`** — `push`, `pop` (`?T`), `peek` (`?T`), `contains`, writable `clear(): void`, and `count`/`isEmpty` (properties), min-first by `Comparable`. Duplicates are allowed, equal elements have no stable tie order, and there is no `foreach` (drain via `pop`). **`Deque<T>`** — `pushFront`/`pushBack`/`popFront`/`popBack`/`peekFront`/`peekBack`/`contains`, writable `clear(): void`, `count`/`isEmpty` (properties), and readonly or writable `foreach` front-to-back. It subsumes FIFO/LIFO, so there are no separate `Queue`/`Stack` types.

Collection-member failures are compiler-owned structured diagnostics. E0521
uses receiver-aware suggestions (for example `has` to `containsKey`), properties
called with parentheses receive E0557, and withdrawn `List::from` /
`Dictionary::from` calls receive bracket-literal guidance rather than an unknown
class error. The same suggestion data is reserved for future migration tooling;
editors consume compiler fixes rather than maintaining another spelling table.

The four no-literal forms use positional-only `Type::from(source)`. Direct bracket assignment is not their construction surface. Existing sources remain unchanged; Copy values (including retained immutable strings) are copied, and Move values must be Cloneable and are cloned. A consuming conversion remains deferred.

Nullable keys and elements do not inherit `Hashable` or `Comparable` from their
payload. `Set` and `Dictionary` require a Hashable element or key;
`SortedSet`, `SortedDictionary`, and `PriorityQueue` require a Comparable element
or key. Nullable equality and duplication remain available where required by
unconstrained element or value positions, but supply no null hash or ordering.

---

## Standard library modules (`Doria\Std\*`)
Hosted modules under the reserved `Doria\Std` namespace. Most are direction-only in the plan today; each links to its owning section/record and is marked *(surface TBD)* where its decision is unauthored.

- **`Doria\Std\Io`** — Stage 29 implements exactly six compiler-known public identities, and Stage 31 Slice 1 integrates them into general namespace resolution: `IoOperation`, `IoTarget`, `IoErrorReason`, `Utf8InputSource`, `IoError`, and `InvalidUtf8Error`. They have no temporary unqualified aliases and cannot be redeclared.

`IoOperation` cases are `Open`, `Read`, `Write`, `Append`, `Flush`, `Close`,
`Register`, `Unregister`, `Wait`, `Configure`, `Access`, `Seek`, `SynchronizeData`,
`SynchronizeMetadata`, `Resize`, `Lock`, `Unlock`, `Spawn`, `Terminate`; `IoTarget` cases are
`File(string $path)`, `StandardInput`, `StandardOutput`, `StandardError`, `ReadinessSet`,
`TransferPlan`, `TransferGroup`, `Process(string $program)`, `ProcessInput(string $program)`,
`ProcessOutput(string $program)` and `ProcessErrors(string $program)`. The process targets name
the program actually launched: the executable path, or the platform shell (`/bin/sh`,
`cmd.exe`) for `Command::shell`; for example "failed to spawn process `/bin/ls`: not found" or
"failed to write process `cat` input: closed". Process waits reuse `Wait`.
`IoErrorReason` cases are `NotFound`, `PermissionDenied`, `InvalidInput`,
`Interrupted`, `ResourceExhausted`, `Unsupported`, `Closed`, `Other`;
`Utf8InputSource` cases are `File(string $path)`, `StandardInput`, `Stream`
(a text adapter over a byte stream with no path).

`IoError` implements `Error` and exposes readonly `message`, `operation`,
`target`, `reason`, `?int systemCode`, and `?int transferredByteCount`.
The trailing `transferredByteCount` constructor parameter defaults to `null`,
preserving existing construction and catches. It records known byte progress
in the failing operation: `null` is unavailable or inapplicable, `0` is known
zero progress, and a positive count is partial or completed transfer progress
before failure; none implies rollback or disk durability.
`InvalidUtf8Error` implements `Error` and exposes readonly `message`, `source`,
`int validByteCount`, and `?int invalidByteCount`; these are byte counts.

Consuming file close uses `IoOperation::Close`, including a checked close
failure; it does not introduce another Error type. Readiness infrastructure
uses `IoOperation::Register`, `IoOperation::Unregister`, or `IoOperation::Wait`
with `IoTarget::ReadinessSet` and `null` byte progress. The added cases preserve
existing tags but require new exhaustive-match arms. Original transfer Errors
retain their identity and optional progress, not a reconstructed readiness
error. Wait expiry and cancellation are typed observations, not infrastructure
failures: expiry ends only the wait, whereas cancellation retires the transfers.
Transfer-plan configuration uses `IoOperation::Configure` with
`IoTarget::TransferPlan` and `null` byte progress, preserving existing `IoError`
catches and optional system codes without creating another Error type.
Registered-stream lookup failures use `IoOperation::Access`, `IoTarget::ReadinessSet`,
and `null` byte progress. Stale or foreign keys fail before stream access.
Stable messages are Doria-owned, while raw platform prose is never
exposed. Ordinary closed stdout/stderr pipes still exit cleanly with status 0;
allocation remains panic. An unhandled Error is R1000/status 70 after cleanup.

**Stage 36a complete; semantics and performance contract accepted, remaining
public spellings deferred.** It uses small
readable/writable/duplex/seekable/flushable/blocking/readiness capabilities with
owned handles, reusable buffers, and first-class standard streams. The bounded
appendix names byte-transfer interfaces, inline outcomes, and owned file
factories. Readable, writable, and duplex files pass native and PHP compatibility
validation on macOS; PHP file owners use the approved shared native runtime
through FFI. Readiness and standard views have targeted functional parity;
buffering, adapters, remaining file controls, processes and the complete
allocation/performance gate remain work. The current free functions are intrinsics, not proof
that the Stage 36a layer exists.

Decision 0138 fixes standard-device views as non-owning, nonclosable inline Copy
values sharing intrinsic device coordination, with no allocation or new handle
on access or copy. Andrew's S-A ruling in Decision 0110 names
`Doria\Std\Io\StandardStreams` with readonly static `input`, `output`, and
`error` properties returning `StandardInput`, `StandardOutput`, and
`StandardError` respectively in `Doria\Std\Io`.
Use `StandardStreams::output` after import. Public getters, transfers and
readiness/pending groups pass targeted macOS interpreter/Cranelift/LLVM/PHP
functional validation. Native PHP standard operations and all six byte-I/O
intrinsics require enabled FFI and the compiler-published matching shared
runtime. A native program shares one line/byte input queue from its first
operation; getter-only and userland-stream programs remain FFI-free. Native
Windows/Linux execution and the full allocation/performance gate remain open.
Under Decision 0138's C-B ruling, Copy-to-interface erasure owns an
independent snapshot. Immediate borrowed calls are valid; a stored returned
borrow requires an explicitly declared interface owner, such as
`ReadinessStream $owner = $input; let $view = borrowInput($owner);`.
The original Copy binding stays independent.
Each view has a readonly `bool $isInteractive`: true when that standard
stream is attached to a terminal (Decision 0140). It never throws, answers
while a pending transfer holds the device, and in PHP output is asked of the
native runtime like every other standard-view member.

Line reading (Decision 0110, Text Adapters): `LineReader<S implements
ReadableStream>` owns its source, and `BorrowedLineReader<S>` borrows it.
`new LineReader<S>($source, $capacity)` bounds a line at `$capacity` bytes;
`readLine()` returns `LineOutcome::Line($text)` (without `\n` or a preceding
`\r`), `WouldBlock`, `EndOfStream`, `TimedOut` or `TooLong`, and throws
`IoError` or `InvalidUtf8Error` (source `Utf8InputSource::Stream`, with exact
stream offsets). `finish()` returns the owned source and discards buffered
bytes.

File locks (Decision 0138, Public Lock Declarations) are whole-file native locks:
`flock` on Linux/macOS, `LockFileEx` on Windows. `ReadableFile`, `WritableFile`
and `DuplexFile` each provide `lock(FileLockMode)` (waits) and
`tryLock(FileLockMode)` returning a borrowed `FileLock<F>`, plus consuming
`lockOwned` and `tryLockOwned` returning a `LockedFile<F>` that owns the file.
`FileLockMode` is `Shared` or `Exclusive`. Contention is a typed outcome:
`BorrowedLockOutcome::Contended`, or `LockOutcome::Contended($file)`, which
returns the file. A `FileLock` holds the caller's file under an exclusive loan,
so the file is reachable only through `$guard->file` until the guard ends; it
never closes the file. `unlock()` reports failures with `IoOperation::Unlock`;
destroying a guard releases best-effort and reports nothing. `LockedFile::unlock()`
returns the file. Acquisition failures use `IoOperation::Lock`, including a
second acquisition through a file that already holds a lock (InvalidInput):
a held mode is never silently converted. Locks belong to each open file, so two
opens of one path in one process contend. Unsupported platforms report
`IoErrorReason::Unsupported`. Locks are advisory on Unix; network filesystems
follow their own semantics.

Decision 0110 specifies inline Copy `Doria\Std\Concurrency\CancellationToken`:
assignment leaves both bindings usable with the same explicitly shared
cancellation state. Each owned
copy retains and releases one claim without allocating; borrowed arguments do
not acquire a claim. Tokens may outlive their source, and neither copying nor
releasing a source or token requests cancellation.
`==` compares shared cancellation-state identity and `!=` is its inverse:
tokens from the same source compare equal even after cancellation or source
destruction.
Equality does not acquire a claim or initialize native resources; nullable
tokens retain ordinary nullable equality. No public address, ordering, integer
conversion, or hashing contract is provided.

- **`Doria\Std\Concurrency\CancellationSource`** is a Move owner, constructed
  with `new CancellationSource()`. Its readonly `token` property returns an
  independently owned Copy token and readonly `cancellationRequested` reports
  the shared state. Writable `cancel(): void throws CancellationError` requests
  cancellation; destruction does not. This lifecycle executes in the development
  compiler through interpreter, both native profiles and PHP/FFI; complete
  cancellation/wait-control integration remains Stage 36a work.
- **`Doria\Std\Concurrency\CancellationError`** implements `Error`, with
  `__construct(string $message, ?int $systemCode = null)` and readonly stored
  `message` and `systemCode`. Cancellation remains requested after notification
  failure. Missing system code is null, not known zero; no I/O operation, target,
  or byte progress is attached.

- **`Doria\Std\Test`** — development-source-only compiler declarations. Slice 1 implements exact compiler-known `describe`, `it`, and `test`, const-evaluable descriptions, nested suites, deterministic generated test callables, and one schema-version-3 table shared with `#[Test]`; there is no runtime registry. Baton consumes that table and never parses Doria. Slice 2 implements `expect`, `fail`, `AssertionError`, the separate automatic `TestAssertion` effect, and core scalar/null/bool/order/string matchers. Slice 3 completes collection/dictionary expectations, `toThrow` and typed Error inspectors, bounded failure differences, strict outcome classification and hierarchy reporting, and compiler-fact-based tooling.
- **`Doria\Std\Fs`** — filesystem/namespace operations without an open handle: existence, size/metadata, permissions, timestamps, rename, delete, `mkdir`, directory listing, and path manipulation. Decision 0110 fixes the `Io`/`Fs` boundary while deferring the exact `Path` representation to the filesystem design; Stage 36a must preserve typed-path evolution. *(Surface TBD.)*
- **`Doria\Std\Env`** — environment variables. *(Surface TBD.)*
- **`Doria\Std\Process`** — Decision 0110 child processes, executable on every backend (the MIR interpreter runs scripted children from its sandbox, never a host process). `Command::executable(string $path)` runs a program directly, found on the child's own `PATH` when the name has no separator; `Command::shell(string $script)` runs the platform shell on one script and takes no arguments. Writable `argument`, `environment`, `removeEnvironment` (the last edit of a variable wins) and `directory` build the request; invalid strings (a NUL byte, or a variable name that is empty or contains `=`) fail at `spawn` as `InvalidInput`. `spawn(ChildCleanup $cleanup): Child throws IoError` requires `Wait`, `Terminate` or `Detach` with no default. `Child` is a runtime-owned value: writable `getInput(): ChildInput`, `getOutput(): ChildOutput` and `getErrors(): ChildOutput` lend its pipes, and different pipes of one child may be lent at the same time; `getId(): int`; writable `waitUntil(Deadline $deadline): WaitOutcome` keeps the child on `TimedOut`; consuming `wait(): ExitStatus` closes the input, drains and discards both outputs concurrently and waits; consuming `terminate(): ExitStatus` kills (Unix `SIGKILL`, Windows exit code 1) without draining; consuming `detach(): void` stops the policy; consuming `collect(int $limit): CollectedOutput` drains both outputs concurrently, keeping at most `$limit` bytes of each (`truncated`), then waits. `ChildInput::closeInput()` half-closes the input and is idempotent; transfers on a closed pipe fail as `Closed`. Pipes are `PendingReadableStream`/`PendingWritableStream`, so they join a `ReadinessSet` and `TransferPlan` through `BorrowedBufferedReader`/`BorrowedBufferedWriter`; a plain `read` or `write` waits for the child as any blocking pipe does and never reports `WouldBlock`. `ExitStatus` is `Exited(int $code)` or `Signaled(int $signal)` (Unix only). Destruction closes the pipes first, then applies the policy once unless the child was resolved, so a `Wait` policy cannot deadlock on a full pipe. Command-line arguments of the current program arrive through `main(List<string> $args)` (decision 0099), not here.
- **`Doria\Std\Time`** — clock and time. Decision 0110 requires duration and absolute-deadline concepts, explicit immediate/indefinite waiting, and no mutable process-global stream timeout. Standalone clock-read failures use Time-owned `ClockError`; readiness-wait failures remain `IoError`. Exact Time members remain owned by the bounded appendix no later than Stage 36a surface finalization. Stage 40a adds calendar/timestamp parsing without replacing that duration/deadline model. *(Remaining surface TBD.)*
  - **`ClockError`** implements `Error`, with `__construct(string $message, ?int $systemCode = null)` and readonly stored properties `message` and `systemCode`. Missing platform code stays `null`, distinct from known zero. Ordinary checked propagation preserves this owned error; it carries no I/O target or byte progress. Use its qualified name or an explicit import.
  - **`Duration` and `Deadline`** are distinct compiler-known inline Copy values. Assignment and argument passing preserve the original without a heap object or reference count. Nullable forms remain Copy; neither type implicitly converts to integers or to the other. Use qualified names or explicit imports, not prelude aliases.
  - **`Duration`** represents signed `int64` nanoseconds across the full range, including negative intervals. Static `fromNanoseconds(int64 $nanoseconds): Duration` constructs an interval; readonly `int64 $nanoseconds` returns it exactly. A negative interval used for a timed wait or relative deadline is already expired, not invalid, an immediate poll, or an indefinite wait. Completion-fetch expiry does not cancel transfers. Remaining Time arithmetic members are owned by the bounded appendix.
  - **`Deadline`** preserves its exact instant after expiry. Static `fromNow(Duration $duration): Deadline throws ClockError` samples the monotonic clock once and adds the exact signed interval, without timer/readiness initialization. Different past instants remain distinguishable; expiry never replaces the value with a marker. Native carrier bits and the monotonic epoch are private ABI, not public integers.
  - **`Doria\Std\Io\Wait`** has `Immediate`, `For(Duration $duration)`, `Until(Deadline $deadline)`, and `Indefinite` cases. This ordinary inline Copy enum requires no heap container. Completion observation uses `getNextCompletion(Wait $wait)`; relative waits freeze one deadline at observation start, while expiry stops only observation and preserves transfers. Import or qualify it; there is no prelude alias.
- **`Doria\Std\Random`** — random-number generation. *(Surface TBD.)*
- **`Doria\Std\Json`** — JSON encode/decode; drives enum/match/mixed ergonomics and the PHP bridge. Stage 40a owns its first practical data-pipeline contract. *(Surface TBD.)*
- **`Doria\Std\Net`** — networking, TCP first. Stage 40a owns the first typed TCP/TLS and HTTP-client foundation over decision 0110 streams, readiness, duration/deadline timing, cancellation, partial writes, backpressure, and reusable buffers. Stage 44 matures and hardens it; socket streams do not define a second I/O contract. Exact TLS configuration belongs to the Stage 40a network design. *(Surface TBD.)*
- **`Doria\Std\Http`** — HTTP client foundation alongside `Net` in Stage 40a; further maturation in Stage 44. *(Surface TBD.)*
- **`Doria\Std\Data`** — **DDO**, the batteries-included database layer: decomposed `Connection` / `Statement` / `Transaction` / typed rows, the `Sql` provenance newtype, RAII transactions with consuming `commit`, capability-based drivers, typed fetches. Direction: plan §9 (DDO); prerequisites checked errors (Stage 29) + `Net`. *(The authoritative DDO record is unauthored; 0007 is a superseded sketch.)*
- **`Doria\Std\Term`** — the portable terminal layer for the TUI product in plan §2, specified by [Decision 0140](decisions/0140-terminal-layer-console-and-raw-mode.md). One program behaves the same in a Linux terminal, a macOS terminal and a native Windows console. It reuses decision 0110's standard-stream views, waits, duration/deadline timing, cancellation, and platform-device layer; generic stream readiness is not reimplemented here. No public value carries an escape sequence or a platform encoding, and there is no screen-buffer type. Columns and rows are zero-based, column first. Every `Console` method below is static and throws `TerminalError` unless noted. Import or qualify the names; there is no prelude alias.
  - **`Console`** (no instances)
    - **terminal** — `getSize(): Size`; readonly static `ColorSupport $colorSupport` (never throws). Whether a standard stream is a terminal is `$isInteractive` on the `Doria\Std\Io` standard views.
    - **output** — `write(string $text, ?Style $style = null)`, `writeAt(int $column, int $row, string $text, ?Style $style = null)`, `flush()`. A style belongs to one write: the style, the text, then the default attributes again. A color the terminal cannot show maps to the nearest one it can. When standard output is not a terminal, or it has no color, `write` emits the text alone. Output collects and reaches the device as one write on `flush()`, before an event wait, before any other use of a standard stream, and at exit, in order with `echo`.
    - **screen** — `clear()`, `clearLine()`, `clearLineFromCursor()`, `eraseAt(int $column, int $row)`, `setTitle(string $title)`, `setLineWrap(LineWrap $wrap)`, `reset()`.
    - **cursor** — `moveTo(int $column, int $row)`, `moveUp(int $count = 1)`, `moveDown`, `moveLeft`, `moveRight`, `showCursor()`, `hideCursor()`. A negative position or count panics.
    - **modes** — `enterRawMode(): RawMode` and `enterAlternateScreen(): AlternateScreen`. Each returns a Move guard; a second one while the first is alive throws `TerminalError` (in use).
  - **`RawMode`** — the owner of terminal input. `getNextEvent(Wait $wait, ?CancellationToken $cancellation = null): TerminalWaitResult throws TerminalError` waits for one key or resize; `getCursorPosition(): Position throws TerminalError`; consuming `leave(): void throws TerminalError`. While it is alive, `read_line`, the byte intrinsics and `StandardInput` reads are refused, and Control+C is a key.
  - **`AlternateScreen`** — consuming `leave(): void throws TerminalError`; the first screen and its scrollback return when it ends. Independent of raw mode.
  - **restoring** — a guard's destructor puts back what it changed. So does the runtime on every way a program ends (return from `main`, `exit`, a closed output pipe, a panic before its message is written) and on termination and stop signals; a forced kill cannot be restored.
  - **`TerminalWaitResult`** — `Event(TerminalEvent $event)`, `TimedOut`, `Cancelled`, `Closed`. **`TerminalEvent`** — `Key(Key $key, KeyModifiers $modifiers)`, `Resized(Size $size)`; several resizes between two waits are one event with the latest size. **`Key`** — `Character(string $text)` (one Unicode scalar), `Enter`, `Escape`, `Backspace`, `Tab`, `Up`, `Down`, `Left`, `Right`, `Home`, `End`, `PageUp`, `PageDown`, `Insert`, `Delete`, `Function(int $number)` (1 to 12).
  - **values** — inline Copy `Size` (`$columns`, `$rows`), `Position` (`$column`, `$row`), `KeyModifiers` (`$control`, `$alt`, `$shift`) and `Style`. A `Style` starts from static `Style::plain` and each of `withForeground(Color $color)`, `withBackground(Color $color)`, `withBold()`, `withDim()`, `withItalic()`, `withUnderline()`, `withReverse()`, `withStrikethrough()` returns a changed copy; `$foreground`, `$background`, `$isBold`, `$isDim`, `$isItalic`, `$isUnderlined`, `$isReversed`, `$isStruckThrough` read it back.
  - **enums** — `Color`: `Default`, `Black`, `Red`, `Green`, `Yellow`, `Blue`, `Magenta`, `Cyan`, `White`, the eight `Bright…` forms, `Indexed(int $index)`, `Rgb(int $red, int $green, int $blue)` (each 0 to 255; outside it panics when the color is used). `ColorSupport`: `None`, `Basic`, `Indexed`, `TrueColor`. `LineWrap`: `Enabled`, `Disabled`.
  - **`TerminalError`** implements `Error` with readonly `message`, `TerminalErrorReason $reason` (`NotATerminal`, `InUse`, `Unsupported`, `DeviceFailure`) and `?int $systemCode`.
  Executable in the interpreter, which runs against a scripted terminal, and in both native profiles. PHP output drives the terminal through the native runtime (FFI), so it needs the compiler-published shared runtime beside the generated program. A `RawMode` implements `Doria\Std\Io\Watchable` (readonly `WatchSource $watchSource`), so a transfer plan can watch it and one group wait covers a program's transfers and its terminal events; see "Watched Sources" under `Doria\Std\Io`.
- **`Doria\Std\Math`** — batteries-included game/graphics math as built-in Copy value types: `Vector2`/`Vector3`/`Vector4`, `Quaternion`, `Euler`, `Matrix3x3`/`Matrix4x4`, plus `lerp`/`clamp`/easing helpers. Compiler-known arithmetic operators; `$v->length` / `$v->normalized` properties, `$v->dot`/`$v->cross` methods. Direction: plan §9 (math); lands Stage 47. *(Geometry-math record unauthored.)*

### Byte Transfer Contracts

Decision 0110's Stage 36a appendix specifies these nominal `Doria\Std\Io`
contracts; the full native stream foundation is still in progress.

- **`ReadOutcome`**: `Data(int $byteCount)`, `WouldBlock`, `EndOfStream`, and
  `TimedOut`. An ordinary inline Copy enum; `Data(0)` is distinct from EOF.
- **`WriteOutcome`**: `Accepted(int $byteCount)` and `WouldBlock`. An ordinary
  inline Copy enum; partial progress does not copy the unwritten suffix.
- **`FlushOutcome`**: `Complete` and `Pending`. An ordinary inline Copy enum;
  it owns no buffer storage and requires no result allocation.
- **`FinishOutcome<S, R>`**: `Complete(S $source)` and `Pending(R $writer)`.
  A generic payload enum, specialized per application. A consuming owned writer
  returns its source on completion or the same writer retaining pending bytes
  for retry. General generic enum layout and payload ownership apply; no extra
  result allocation is introduced.
- **`BorrowedFinishOutcome<R>`**: `Complete` and `Pending(R $writer)`.
  A pending writer carries its source loan through the payload. Completion releases the
  source loan without transferring its owner; pending retains it in the writer.
  Finish performs one non-waiting attempt. Checked I/O failure uses ordinary
  consuming cleanup without promising recovery of the consumed writer.
- **`ReadableStream`**: `writable function read(writable WritableByteRegion
  $destination): ReadOutcome throws IoError`.
- **`WritableStream`**: `writable function write(ByteRegion $source):
  WriteOutcome throws IoError`.
- **`DuplexStream`**: extends `ReadableStream` and `WritableStream`, adding no
  unrelated capability.
- **`FlushableStream`**: `writable function flush(): FlushOutcome throws IoError`.
  `Complete` reports completed drainage. `Pending` may follow partial drainage;
  the adapter retains the remainder for retry without repeating accepted bytes.
  No automatic readiness wait or busy polling is inserted. Buffer flushing is
  not durable synchronization; checked failures still throw `IoError`.
- **`BufferedReader<S implements ReadableStream>`**: `new BufferedReader($source,
  $capacity)` takes the source and a required capacity. `read`, `fill():
  ReadOutcome`, readonly `ByteRegion $buffered { borrowed get; }` (the unread
  window, not a copy), `consume(int $byteCount)` and `consuming finish(): S`,
  which discards unread bytes. A request at least as large as the buffer reads
  straight into the caller's region. `BorrowedBufferedReader<S>` retains
  `borrow writable S $source` instead and has no `finish`.
- **`BufferedWriter<S implements WritableStream>`**: implements `WritableStream`
  and `FlushableStream`. `write` copies into the bounded buffer or, when the
  buffer is empty, passes a large write straight through; a full buffer that
  cannot drain reports `WouldBlock`. `flush` drains the buffer into the source
  without waiting. `consuming finish(): FinishOutcome<S, BufferedWriter<S>>`
  makes one drainage attempt. `BorrowedBufferedWriter<S>` retains
  `borrow writable S $source`, and its `finish` returns
  `BorrowedFinishOutcome<BorrowedBufferedWriter<S>>`. Dropping either writer
  flushes what it can; failures cannot escape a destructor, so call `flush` or
  `finish` to observe them. Capacity below 1, and a `consume` beyond the
  buffered bytes, panic.
- **Buffered adapter tiers**: each adapter also implements `ReadinessStream`
  and the pending tier of its direction (`PendingReadableStream` or
  `PendingWritableStream`) when its source does, forwarding the source's
  `readinessSource`. A pending read completes immediately from buffered bytes
  and otherwise passes through to the source in plan order; a read prepared
  after the buffered bytes run out in the same plan completes with
  `WouldBlock`. A writer carries one pending write at a time: free space
  accepts it immediately, a full buffer drains through one transfer first, and
  a second concurrent write on the same writer fails at preparation with
  `IoError` (`Configure`, `TransferGroup`, `InvalidInput`).

Transfers borrow regions only for the call. Successful byte counts lie between
zero and region length; empty requests succeed with zero progress. The writable
receiver permits position and buffer-state changes without consuming ownership.
These interfaces grant neither close nor mode control; nonclosable standard
views can implement them. Concrete generic paths and deliberate erasure use
ordinary interface machinery, not another stream object model.

### Typed Registration Keys

`Doria\Std\Io\RegistrationKey<T>` is an opaque inline Copy value identifying
a registration, not owning its stream. Assignment, argument passing, and return
leave the original key usable; nullable keys are Copy too. The set retains the
complete registered value and its static type `T`. Copying a key allocates
nothing and acquires no handle, reference count, or independent stream loan.
Keys do not implicitly convert between registered types or to integers.
Stale and foreign keys are rejected before registration access. Key copies do
not bypass ordinary set/group borrowing or keep a destroyed set alive. Public
registration/group member declarations remain in Decision 0110's appendix.
`register` transfers the complete stream into the set and returns its typed
key; `unregister` returns the same owned `T`, making other key copies stale.
Both require writable set access and retain the checked `IoError` contract.
`getStream` borrows the same registered `T` readonly; `getWritableStream`
requires writable set access and returns a writable loan. The set keeps the
complete owner, and either loan prevents incompatible set mutation or teardown.
Under Decision 0089, `let writable $stream = $set->getWritableStream($key);`
retains that proved writable loan as a nonowning exclusive local. Plain `let`
remains readonly; a readonly lookup cannot be upgraded by adding `writable`.
Registration follows that adapter's current readiness source. Replacing its
underlying file keeps the same key and complete adapter; future waits and
transfers use the new source. An already-started operation retains its selected
source and native resources until retirement. Unchanged sources reuse their
native association. The public readiness view remains borrowed, not owning.
These member names are approved; their executable integration remains Stage 36a work.
`new ReadinessSet()` constructs an empty owning set, with checked `IoError`
setup failures. Construction registers no stream, performs no transfer, and
does not eagerly associate a native handle.

```doria
class ReadinessSet {
    function __construct() throws IoError;
    writable function register<T implements ReadinessStream>(take T $stream): RegistrationKey<T> throws IoError;
    writable function unregister<T>(RegistrationKey<T> $key): T throws IoError;
    function getStream<T>(RegistrationKey<T> $key): T throws IoError;
    writable function getWritableStream<T>(RegistrationKey<T> $key): T throws IoError;
    writable function begin(
        take TransferPlan $plan,
        ?Doria\Std\Time\Deadline $deadline = null,
        ?Doria\Std\Concurrency\CancellationToken $cancellation = null
    ): TransferGroup throws IoError;
}
```

Registration requires the explicit readiness capability, not a byte-transfer
or pending-transfer capability. Concurrent plan operations separately require
their applicable pending-transfer contract.
The optional `deadline` and `cancellation` parameters control the group's
lifetime, not an individual completion wait. Null omits the control; the group
retains shared cancellation state without a source loan. Cancellation or a
group deadline cancels and drains before storage release. A reused set does
not inherit an earlier group's controls. No `TransferControl` type is added.
The development signature executes through the interpreter, Cranelift, LLVM,
and PHP/FFI. macOS validation covers default and named controls, exact deadline
copying, detached-token retention, control replacement, stopped groups, and set
reuse; Linux/Windows execution remains unverified.

### Retained Transfer Plans

Decision 0110 names `Doria\Std\Io\TransferPlan` as the consuming configuration
carrier for concurrent transfers. `TransferPlan::read` starts a read plan;
consuming `andReadFrom` adds a read without losing earlier destination loans.
`writeTo` starts a write plan and consuming `andWriteTo` adds a write while
retaining shared readonly source loans. Either append follows either factory
and preserves all earlier loans. Each operation requires its exact registered
pending capability and declares checked `IoError` setup failures:

```doria
class TransferPlan {
    static function read<T implements PendingReadableStream>(RegistrationKey<T> $key, writable WritableByteRegion $destination): self throws IoError;
    consuming function andReadFrom<T implements PendingReadableStream>(RegistrationKey<T> $key, writable WritableByteRegion $destination): self throws IoError;
    static function writeTo<T implements PendingWritableStream>(RegistrationKey<T> $key, ByteRegion $source): self throws IoError;
    consuming function andWriteTo<T implements PendingWritableStream>(RegistrationKey<T> $key, ByteRegion $source): self throws IoError;
    function getReadKey(int $position): ReadTransferKey throws IoError;
    function getWriteKey(int $position): WriteTransferKey throws IoError;
}
```

The readonly key lookups use zero-based positions across the entire mixed plan.
Negative, out-of-range, or wrong-direction positions throw `IoError` without I/O.
Those failures use `IoOperation::Access`, `IoTarget::TransferPlan`, and
`IoErrorReason::InvalidInput`, with a payload-free `TransferPlan` target and null
`systemCode` and `transferredByteCount`.
Keys remain stable through consuming appends; repeated operations on one stream
and operations in separate plans have distinct identities. Key copies retain no
plan, buffer, or stream loan.

Configuration does no I/O. `begin` consumes the plan through a `take` parameter,
borrows the set exclusively, and forwards all plan loans into the transfer group.
Its own checked setup failures use `IoOperation::Configure` and
`IoTarget::TransferGroup`, preserving reason and system-code details. An
adapter's original owned checked error propagates unchanged. Failed setup
consumes the plan, releases its loans, and leaves no active group or submitted work.
The canonical group retains the set's exclusive borrow and all inherited plan
loans until cleanup, including through helpers that return the group. Failed
preparation retires each earlier successful preparation exactly once before
releasing its selected source; it never submits those prepared operations.
Generic calls and interface erasure preserve an owned carrier's earlier loans;
consumption cannot lend the carrier's own storage beyond its destruction.
Owned-result callback retention is a permitted upper bound: nullable forwarding
and loan-free null/empty results share the same annotation. Known body proofs
preserve actual loans without inventing loans for fresh results; opaque calls
retain the bound. Returned borrowed aliases retain their distinct contract.
The source bindings stay unavailable until teardown cancels and drains pending
operations. Completed storage is processed and reused through the group.
Dropping an unstarted plan releases its loans without submitting operations.
Competing operations on one stream follow plan order. Independent duplex
read/write directions may overlap; operations sharing a file cursor compete.
This orders access to stream bytes, not completion observation across
independent streams. Partial-transfer outcomes remain unchanged.
The group returns the next completion identifying the finished read or write,
without requiring callers to scan transfers or allocate for each observation.
No available completion and a completed operation with no transfer outcome are
distinct. Observation retains the group-wide loans; completed storage remains
accessible through the group, not the original bindings.
Fetching a failed completion raises the original checked error, rather than
deferring failure to a completion outcome getter. Its catch can identify the
failed read or write without scanning; the owned error transfers exactly once
with optional byte progress preserved. Catching it neither releases the group
loans nor cancels the other operations.
If wait infrastructure also fails, the fetch reports that failure first without
taking a queued transfer result. A later fetch still observes the completed
bytes or original owned transfer Error exactly once; both observed keys are null
for the infrastructure failure, which has no transfer identity or byte progress.
The reusable owner is `Doria\Std\Io\TransferGroup`. Its readonly properties
`observedReadKey` (`?ReadTransferKey`) and `observedWriteKey`
(`?WriteTransferKey`) publish the fetched operation's original plan identity
before raising its error. Only the matching direction is present; both are
null before the first fetch and after timeout or exhaustion. Resubmission keeps
the same key, and copying an observed key retains no owner or loan.
Readonly `getReadRegion(ReadTransferKey $key): ?ByteRegion throws IoError`
returns null for active work or an unknown completed byte range, but
a present empty readonly region for known zero-byte completion, including EOF.
Invalid or foreign keys throw `IoError` with `IoOperation::Access`, appended
`IoTarget::TransferGroup`, and `IoErrorReason::InvalidInput`, without a path,
system code or byte progress. The view covers only the completed
prefix, retains the group loan, and prevents conflicting mutation or access
after group destruction. Lookup performs no I/O or allocation; resubmission
makes the previous prefix unavailable. Public group execution is not yet delivered.
An immediately completed read retires at submission: its known prefix can be
inspected with the saved original key before fetching. This does not repeat the
completion callback, take the queued original outcome or Error, publish an
observed key, or permit reuse before the previous result has been fetched.
`resubmitRead` / `resubmitWrite` are the approved reuse names: they retain the
original typed operation key and reuse a checked range of its retained buffer,
after conflicting region loans end and the previous result has been observed.
Both require writable group access:

```doria
writable function resubmitRead(ReadTransferKey $key, int $offset, int $length): void throws IoError;
writable function resubmitWrite(WriteTransferKey $key, int $offset, int $length): void throws IoError;
```

They do not create new operations or keys; `begin` submits the initial transfers.
Invalid resubmission keys or ranges, active operations and unobserved results
use existing `IoError` with
`IoOperation::Configure`, `IoTarget::TransferGroup`, and
`IoErrorReason::InvalidInput`, without a path, system code or byte progress.

### Watched Sources

A plan can also watch a source that is not a byte stream, so one group wait
covers transfers and, for example, the terminal's key and resize events
(Decisions 0110 and 0140):

```doria
interface Watchable {
    WatchSource $watchSource { get; }
}
class TransferPlan {
    static function watch<T implements Watchable>(T $source): self throws IoError;
    consuming function andWatch<T implements Watchable>(T $source): self throws IoError;
    function getWatchKey(int $position): WatchKey throws IoError;
}
enum TransferCompletion {
    case Read(ReadTransferKey $key, ?ReadOutcome $outcome);
    case Write(WriteTransferKey $key, ?WriteOutcome $outcome);
    case Ready(WatchKey $key);
}
class TransferGroup {
    ?WatchKey $observedWatchKey { get; }
    writable function resubmitWatch(WatchKey $key): void throws IoError;
}
```

- `Watchable` is implemented by the standard library only;
  `Doria\Std\Term\RawMode` is the first implementation. A class in a program
  that declares it is rejected (E0781). `WatchSource` is an opaque inline Copy
  value with no public constructor or member: it is how a source tells the
  runtime what to watch.
- A watched source is borrowed, not registered. The plan and then the group
  keep a shared loan of it: its readonly members stay callable while the group
  runs, and consuming or moving it while it is watched is rejected (E0763).
  A plan that only watches can begin on a `ReadinessSet` with no registration.
- `WatchKey` is an inline Copy key. `getWatchKey` takes the zero-based plan
  position; a position that is not a watch throws the same `IoError` as a
  wrong-direction `getReadKey` (`Access`, `TransferPlan`, `InvalidInput`), and
  `getReadKey` / `getWriteKey` throw it for a watch position.
  `observedWatchKey` follows the two directional properties: present only
  after a fetch that reported that watch.
- A watch is armed when the group begins. It completes once, as
  `Completed(Ready($key))`, when its source has something the program can take
  without waiting. For a `RawMode` that is when
  `getNextEvent(Wait::Immediate)` would not return `TimedOut`. The program
  takes what is there and calls `resubmitWatch($key)` to watch again. A watch
  that is not resubmitted is retired, and a group whose every read, write and
  watch is retired reports `Exhausted`.
- At each fetch a ready watch is reported before a queued transfer, in plan
  order. If the program has already taken everything itself, no `Ready` is
  reported.
- When cancellation or the group deadline stops the group, a watch that was
  armed is reported once as `Ready`, as a pending read is reported once as
  `Read($key, null)`. Destroying the group retires its watches silently and
  leaves the source as it was.
- `resubmitWatch` throws `IoError` (`Configure`, `TransferGroup`,
  `InvalidInput`) for a key that is not a watch of this group, a watch that
  is still armed, a `Ready` that has not been fetched, and a stopped group.
  `begin` throws `IoError` (`Configure`, `TransferGroup`, `Unsupported`) when
  the platform's wait cannot include the source; on Windows one group can
  watch one source.
Validation submits no work and consumes no stored transfer Error; actual
transfer failures still propagate unchanged at `getNextCompletion`.
`getNextCompletion(Wait $wait)` returns `TransferWaitResult`. Both result types
are ordinary nominal inline Copy enums in `Doria\Std\Io`:

```doria
enum TransferCompletion {
    case Read(ReadTransferKey $key, ?ReadOutcome $outcome);
    case Write(WriteTransferKey $key, ?WriteOutcome $outcome);
}
enum TransferWaitResult {
    case Completed(TransferCompletion $completion);
    case TimedOut;
    case Exhausted;
}
```

`Completed(Read($key, null))` identifies a retired read with no transfer outcome,
not a timeout or exhaustion observation. Actual zero-byte success, would-block,
EOF, and timed-out read outcomes retain their existing meaning. Copying either
enum retains no group, stream, or buffer and grants no buffer access. Failed
fetches still throw the original owned checked Error, not an error-bearing case.
Timeout and exhaustion observations do not permanently prevent resubmission.

Completion-fetch budgets are separate from group deadlines. Expiry stops only
the wait; transfers remain active. A typed timeout observation is distinct from
a completion or exhausted group. Expiry retains loans, identities, and progress,
allowing another wait and resubmission through the same group without fabricating
a checked error or transfer outcome.
Cancellation or an explicit group deadline cancels and drains outstanding transfers.
Untimed waits require no clock sampling or per-wait allocation.
`ReadTransferKey` and `WriteTransferKey` are distinct opaque inline Copy
identifiers for read and write operations, not stream registrations or integer
indexes. Neither has a public constructor, converts to an integer, or substitutes
for the other key family. Their transport adds no separate allocation or owner.
Public group execution and the remaining readiness-member spellings are still
in-progress Stage 36a work, not a delivered API.

### Owned File Family

Decision 0110 fixes the `Doria\\Std\\Io` names below. `File` is a factory
companion; its results are owned Move handles, not capability-parameterized
`File<Capability>` values. Public execution remains in-progress Stage 36a work.

| Factory               | Owned Result   | Transfer Capability |
|-----------------------|----------------|---------------------|
| `File::openRead`      | `ReadableFile` | `ReadableStream`    |
| `File::openWrite`     | `WritableFile` | `WritableStream`    |
| `File::openReadWrite` | `DuplexFile`   | `DuplexStream`      |

`FileCreation` has cases `OpenExisting`, `OpenOrCreate`, `CreateNew`,
`TruncateExisting`, and `CreateOrTruncate`. Writable opens require an explicit
creation choice, with no default. Readable opens use existing files without
creation/truncation. `FileWriteMode` has exactly `Positioned` and `Append`.
Both writable factories require an explicit mode with no default, independently
of creation. Positioned writes use and advance the handle's cursor; every
`FileWriteMode::Append` write goes to the then current end of the file. Each open has its own
cursor initially at zero. Each owned file exposes `consuming function close(): void throws
IoError`; success and checked failure both consume it. Close uses
`IoOperation::Close`, retains the opening path for errors, and is not retried by
ordinary nonthrowing destruction. Remaining metadata/option controls reopen through Decision
0110 before implementation.
Public owners permit simultaneous readers and writers, with independent cursors.
Windows shares read and write access but not deletion access; the existing free
file helpers retain their established sharing contracts.

All three file owners also implement `SeekableStream`, whose writable
`seek(int $offset, SeekOrigin $origin): int throws IoError` returns the new byte
position. `SeekOrigin` has exactly `Start`, `Current`, and `End`; origin has no
default. Each file exposes readonly `getPosition(): int throws IoError` and
`getLength(): int throws IoError` methods. Seeking requires writable access; inspection requires only
readonly access and changes neither cursor nor contents. A seek beyond EOF
does not itself extend the file, and append writes remain placed at the then
current EOF. Seek uses appended `IoOperation::Seek`; inspection uses `Access`.
Both preserve the opening-path File target, null byte progress and optional
system code. Unrepresentable canonical-int results throw `InvalidInput` rather
than truncating. These declarations are approved Stage 36a contracts; execution
validation is tracked in the pipeline note, not a claim of complete stage delivery.
These methods replace the proposed position/length properties. Fresh inspection
may block; Decision 0135's nonblocking hooks remain unchanged and cannot call
these methods, including through helpers. There are no property aliases.

WritableFile and DuplexFile expose writable
`synchronizeData(): void throws IoError` and
`synchronizeMetadata(): void throws IoError`. The first requests durable data;
the second full data plus metadata, not runtime-buffer flushing. Neither changes
position, length or contents. Both may block and are forbidden in property hooks
directly or through helpers. Failures use appended
`IoOperation::SynchronizeData` / `IoOperation::SynchronizeMetadata`,
opening-path File targets, optional system codes and null byte progress.
Platforms may strengthen data-only synchronization, never weaken either
guarantee. ReadableFile exposes neither method. Successful controls allocate
nothing or initialize readiness/async infrastructure.

WritableFile and DuplexFile also expose writable
`setLength(int $length): void throws IoError`, with a required nonnegative byte
length. Shrinking discards the tail; growth reads as zero and preserves supported
sparse behavior. The cursor is unchanged, even beyond EOF; other opens retain
their cursors and observe fresh length. Append still writes at current EOF.
Errors use appended `IoOperation::Resize`, the opening-path File target,
optional system code and null progress. Negative lengths are InvalidInput.
The call neither consumes the file nor flushes/synchronizes it. It may block and
is forbidden in property hooks, including through helpers. ReadableFile has
no resizing method; there is no truncate alias or new interface.

Read-only signatures are `File::openRead(string $path): ReadableFile throws
IoError`, writable `ReadableFile.read(writable WritableByteRegion
$destination): ReadOutcome throws IoError`, and consuming
`ReadableFile.close(): void throws IoError`. Users do not construct either
`File` or `ReadableFile` directly. Each open has its own position. Reads borrow
the destination only for the call without copying or allocating another buffer;
an empty region returns `Data(0)` even at EOF, while a nonempty read at EOF
returns `EndOfStream`. Read-only integration passes validation; the complete
file family is not delivered.

Writable signatures are `File::openWrite(string $path, FileCreation $creation,
FileWriteMode $mode): WritableFile throws IoError` and
`File::openReadWrite(string $path, FileCreation $creation, FileWriteMode $mode):
DuplexFile throws IoError`. `WritableFile` implements `WritableStream`;
`DuplexFile` implements `DuplexStream`. Both expose writable
`write(ByteRegion $source): WriteOutcome throws IoError`, and the duplex owner
also exposes the same writable `read` as `ReadableFile`. Neither transfer
retains its region or consumes its owner. Each owned file has consuming checked
`close`; direct construction of any file owner remains unavailable.
Writable native integration passes interpreter/Cranelift/LLVM validation on
macOS. PHP readable, writable, and duplex files use the same native runtime through
FFI, including the complete creation/write-placement matrix. Generated native-backed
PHP must be deployed with its compiler-published `.doria-runtime` directory on a
matching platform with FFI enabled. Missing, disabled, or mismatched dependencies
report `B3602`; there is no PHP host-file fallback. Pure PHP stream implementations
do not require FFI. These local results are not Linux/Windows execution evidence.

`ReadableFile` also implements `PendingReadableStream`, `WritableFile`
implements `PendingWritableStream`, and `DuplexFile` implements both. Their
preparation and completion signatures are the exact pending contracts below.
Preparation selects the supplied region without submitting I/O; completion
observes the retired request's nullable outcome without another transfer.
These typed implementation operations are not yet public group execution.

### Readiness Ownership

Decision 0110 requires persistent readiness sets to own registered streams.
Registration transfers ownership, access goes through the registration, and
removal returns ownership. Set destruction cleans up remaining streams.
Registration preserves the registered static type `T`; a typed key identifies
the registration without owning its stream. Removal returns owned `T`, without
automatic capability erasure or runtime type discovery. A registered
`ReadableFile` retains its checked `close()`; an intentionally erased
`ReadableStream` remains that interface type. Failed registration follows
ordinary `take` cleanup, not restoration of the caller's moved binding.
Temporary one-stream waits may borrow for the call. Public readiness spellings
and execution remain part of the in-progress Stage 36a surface.
`ReadinessStream` exposes `ReadinessSource $readinessSource { borrowed get; }`.
Custom adapters expose an opaque borrowed readiness source explicitly, forwarding
the receiver's loan without acquiring ownership or a public platform handle.
Waiting uses the shared native core, while transfers dispatch through the
complete registered adapter. A transfer interface alone does not imply readiness;
erased values must retain the capability statically. `ReadinessSource` has no
public constructor or operations. Formation performs no I/O or allocation, and
native association is lazy rather than part of synchronous file opening.
Concurrent transfers use one exclusively borrowing group. Original set and
buffer bindings remain unavailable until that group releases its loans;
completed buffers may be processed and reused through it while other operations
remain pending. Teardown drains pending operations before releasing storage.
This does not add independent registration borrowing or general borrowed fields.
Decision 0136 allows an adapter's separately owned stored file and buffer paths
to be borrowed independently. Their lifetime owner and containing properties
cannot be moved, replaced, or destroyed during either loan. Selected storage
remains protected until retirement; different indices or ranges of one buffer
do not establish independent ownership.
Concurrent adapters require an explicit pending-transfer capability rather than
only a readiness source. Preparation and completion dispatch through the complete
adapter using group-owned stable request slots; ordinary synchronous method
bodies are not suspended. Completion runs exactly once after retirement,
including actual progress that wins cancellation. Ordinary synchronous adapters
retain their existing contract. Public execution remains in-progress Stage 36a
work.

### Pending Adapter Capabilities

Decision 0110 fixes these `Doria\Std\Io` members:

- **`PendingReadableStream`** extends `ReadableStream` and `ReadinessStream`:
  writable `prepareRead(writable ReadRequest $request, writable
  WritableByteRegion $destination): void throws IoError` and writable
  `completeRead(ReadRequest $request, writable WritableByteRegion
  $destination): ?ReadOutcome throws IoError`.
- **`PendingWritableStream`** extends `WritableStream` and `ReadinessStream`:
  writable `prepareWrite(writable WriteRequest $request, ByteRegion
  $source): void throws IoError` and writable `completeWrite(WriteRequest
  $request, ByteRegion $source): ?WriteOutcome throws IoError`.
- **`ReadRequest`**: opaque call-scoped view of a reusable group slot;
  readonly `?ReadOutcome $outcome { get throws IoError; }` and writable
  `complete(ReadOutcome $outcome): void` for immediate preparation.
- **`WriteRequest`**: corresponding write slot with readonly
  `?WriteOutcome $outcome { get throws IoError; }` and writable
  `complete(WriteOutcome $outcome): void`.

Neither request is constructible, consumable, or independently retained.
Preparation selects group/registered-owner storage without submitting I/O;
completion receives the original group region after retirement. `outcome`
preserves checked failures, and `complete` cannot retire an active native request.
Both adapter operations are nonblocking, including helpers and cleanup. See
Decision 0110 for exact-once completion and cancellation progress.

Pending getters and completion returns are nullable only when no transfer
outcome exists. Completion still releases prepared storage if cancellation or
later setup failure prevents submission. Buffered/in-memory immediate outcomes
are retained even without native submission; zero-byte success and `WouldBlock`
are actual outcomes, not absence. Ordinary transfer enums and synchronous
`read`/`write` returns remain unchanged.

## Invalidated elsewhere

- The plan's Decision 0095 catalogue entry and Decision 0095's `Comparable<T>` consequence now include the typed comparison operand.
- The integer conversion inventory no longer presents an untyped parameter as a Doria signature; decision 0042 remains authoritative for the accepted integer-expression operand contract.
- The collection inventory now documents decision 0102's `[value; count]` sequence constructor and its v1.0 restrictions.
- Stage 23c implements the core-language sequence constructor described above;
  the `Doria\Std\*` module inventory remains a catalogue of planned APIs.
- Decision 0114 replaces the unauthored enum subject. Unit, backed, and payload
  enums are executable core-language values, and implemented Decision 0115
  completes core `match`, `if` pattern guards, and explicit whole-scrutinee
  consumption. The approved general generic-enum extension belongs to Stage 36a;
  its implementation remains pending. Website versioned guide/API reference and
  playground synchronization for the finish result declarations remains open.
