# Standard library reference — the planned surface

> Documentation role: the at-a-glance catalogue of Doria's **core** and **standard-library** surface — every companion, interface, collection, free function, and `Doria\Std\*` module, with its purpose and known member surface. This is the *inventory*; the end-to-end plan §9 owns the *direction and rationale*, and each decision record owns the *precise contract*. Both are linked from every entry. This is the **planned** surface: some members are settled in a record, others are marked *(surface TBD in …)* until their decision is authored. It grows as decisions land — keep it in sync when a stdlib record is authored or amended.

Two layers (plan §9): **core** (no I/O, always available) and **std** (hosted under the reserved `Doria\Std` namespace, per the namespace-model decision). Both are written in Doria as early as self-hosting allows.

---

## Core layer (no I/O, always available)

### Primitive companions
Every primitive has a companion, and the set is a **complete, symmetric matrix** — not a per-type ad-hoc collection (decision 0104). The uniform v1.0 baseline: `parse(string): ?T` on every scalar companion (int family, float family, `bool`); `String` is the exclusive owner of accepted string-specific operation vocabulary instead of `parse` because `string` is the parse *domain* (0103); display is uniform through the display path, so no companion has `toString`. Absences are deliberate — a member not meaningful for a primitive is a documented N/A, one not in v1.0 is a named v1.0+ furnishing (`MIN`/`MAX`, `Bool::toInt`). Decision 0103's reviewed inventory records both executable operations and accepted follow-up furnishings. Details: decisions 0013/0016 (numeric types), 0042 (conversions), 0095 (`pow`), 0096 (interface conformance), 0103 (canonical String API), 0104 (completeness); §4.6 (strings).

- **`Int`, `Int8`/`Int16`/`Int32`/`Int64`, `UInt8`/`UInt16`/`UInt32`/`UInt64`** — `Int::parse(string): ?int`, `Int::toFloat(int): float`, `Int::pow(...)`, wrapping arithmetic (`wrappingAdd`/`wrappingSub`/`wrappingMul`), and per-width checked conversion families such as `Int32::from` (panics on overflow) / `Int32::tryFrom` (returns `?int32`). Each accepts one fixed-width integer expression; their callable declaration surface is TBD with conversion-overload work rather than published as an untyped Doria parameter.
- **`Float`, `Float32`/`Float64`** — `Float::parse(string): ?float`, `Float::toInt(float): int` (checked, panics on NaN/out-of-range), `Float::pow(...)`. `float` is neither `Hashable` nor totally `Comparable` (0096).
- **`Bool`** — `Bool::parse(string): ?bool` (returns `true`/`false` for exactly `"true"`/`"false"`, case-sensitive, no whitespace tolerance; `null` otherwise). No `MIN`/`MAX`/`pow`/wrapping (N/A for a two-valued type); `Bool::toInt` is a named v1.0+ furnishing (0104).
- **`String`** — executable intrinsic properties are `$s->length` (Unicode grapheme clusters), `$s->byteLength` (UTF-8 bytes), `$s->isEmpty`, and `$s->bytes` (copy in v1.0). Executable companion operations are trimming/casing (`trim`, `trimStart`, `trimEnd`, `lower`, `upper`, `lowerFirst`, `upperFirst`); predicates (`contains`, `startsWith`, `endsWith`, `equalsIgnoreCase`, `containsIgnoreCase`, `startsWithIgnoreCase`, `endsWithIgnoreCase`); grapheme-indexed search (`indexOf`, `lastIndexOf`, `indexOfIgnoreCase`, `lastIndexOfIgnoreCase`); `countOccurrences`; `replace`; `split`/`join`; `slice`; `repeat`; `padStart`/`padEnd`; and UTF-8-validating `fromBytes` (decision 0103). The `$s->graphemes` / `$s->codePoints` views await the public traversal protocol, and `compare` / `compareIgnoreCase` await executable `Ordering`. There is no public `str_*` family, `$s->chars`, integer string indexing, or duplicate instance-method spelling.

### Value interfaces (core contracts)
Details: decision 0096 (primitive conformance), the interfaces/traits decision (Stage 35), 0079 (`Displayable`).

- **`Comparable<T>`** — `compare(T $other): Ordering`, over the core enum **`Ordering { Less, Equal, Greater }`** (decision 0095). There is no `<=>` operator.
- **`Equatable<T>`** — structural/value equality contract (`==`/`!=`).
- **`Hashable`** — a canonical hash for `Dictionary`/`Set` keys.
- **`Displayable`** — `toString(): string`; Doria's answer to `__toString`, drives interpolation / `.` / `echo` (§4.6, 0079).
- **`Cloneable`** — the explicit-duplication contract (`->clone()`), public from Stage 35.
- **`Error`** — the built-in error interface all thrown errors implement (checked-errors decision, Stage 29).

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

In v1.0 the readonly family accepts **class payloads only** — `SharedReference<int>` and `shared new List<int>()` are rejected, since it forwards readonly access directly with no access object for indexed operations. The writable family accepts supported owned collection move types, because its access objects forward member and indexed operations (`$access[0] = 10`). This is a v1.0 domain rule that a later readonly-sharing design may widen.

The readonly and writable families are disjoint: no conversion exists either way between `SharedReference<T>` and `WritableSharedReference<T>`, the weak forms preserve their family, and no readonly-family and writable-family handle ever refer to the same allocation. All `WritableSharedReference<T>` handles to one allocation share a single runtime access state.

- **`ReadonlySharedReferenceAccess<T>` / `WritableSharedReferenceAccess<T>`** — temporary access objects returned by `WritableSharedReference<T>`; member and indexed access forward to the underlying `T`. The readonly form permits only readonly operations, the writable form both; neither may move out or consume `T`. Each retains an owning reference to the allocation. They are move types — returnable, storable, passable — but never directly constructible by user code, and the access releases deterministically when the object is destroyed (moving it transfers that obligation).

A standalone `{ ... }` block (decision 0107) is the direct way to end an access
object's lifetime before the surrounding function continues.

### Iteration
- **`Iterable<T>` / `Iterator<T>`** — the public iteration protocol that makes user types work with `foreach`; user conformance lands at Stage 35 (built-in collections use compiler-internal iteration earlier, Phase D).

### Ranges and math basics
- **Range types** — `a..b` (inclusive) / `a..<b` (exclusive-end); `int` endpoints; used with `foreach` (SPEC control flow).
- **`math` basics** — scalar math functions (the geometry/vector types live in `Doria\Std\Math`, below).

### Built-in free-function layer
Regularized `snake_case` functions for capabilities without one natural owning type. Type-coupled vocabulary belongs to the corresponding companion. Details: decision 0074 (formatted I/O), §9.1 charter, and decision 0103 (String boundary).

- **Formatting:** compiler-known `sprintf` returns `string` and `printf` returns `void`. Each takes a literal `string $format` first, followed by the typed operands required by that format; this intrinsic-only tail is not an untyped userland parameter declaration. Specifiers: `%s %d %f %.Nf %x %X %o %b %%`, width / `-` / `0` flags.
- **Text I/O:** `read_line(string $prompt = ""): ?string` — writes the prompt exactly (no newline added), flushes stdout, then reads one line; the flush happens even for the default empty prompt, `null` is EOF and `""` is a blank line — `read_file(string): string`, `write_file(string, string): void` (truncate), `append_file(string, string): void`, `write_stderr(string): void`.
- **Output statement:** `echo` (the single output spelling — `print` is rejected).
- **Time:** `get_time`.
- **Meta:** `function_exists("name")` — const-evaluated compile-time predicate for guarded/polyfill declarations.

Binary I/O uses `read_file_bytes(string): Bytes`, `write_file_bytes(string, Bytes): void`, and `append_file_bytes(string, Bytes): void` per record 0091, plus `read_stdin_bytes(): Bytes` and `write_stdout_bytes(Bytes)`/`write_stderr_bytes(Bytes)` per record 0101. The byte arguments are readonly borrows. The stream-object tier moves to `Doria\Std\Io` post-Stage-29. There is deliberately no `write_stdout(string)` — `echo` is the sole stdout text writer (0101).

---

## Collections (core-language move types)
Owned move types with a growable/fixed distinction. The complete family and naming are settled in **decision 0092**; the **method surface** is settled in **decision 0100**. The complete non-closure family is implemented and closure methods wait for Stage 30. A bare name is the default (hash / insertion-ordered) collection; the `Sorted` prefix is the comparison-ordered variant. Cross-cutting rules (0100): reads are `readonly`, mutators `writable`; ingestion moves values in; `$l[i]`/`$d[k]` reads **assert presence and panic** on absence while the `?T` methods are the safe path; removal hands the owned element back; mutators return `void` (userland fluent APIs stay a decision-0088 capability).

Sequence literals also have the repeat form **`[value; count]`** (decision 0102). It contextually constructs either a runtime-sized `T[]` or a one-shot-filled `List<T>` and defaults to `List<T>` without an expected type. `value` is evaluated once before the runtime-`int` `count`; zero produces an empty sequence, a constant-negative count is rejected, and a runtime-negative count panics with `fill count is negative`. In v1.0, `value` must be a Copy scalar or `string`; move-type and nullable fills await `Cloneable` (Stage 35). The form is sequence-only: it does not construct `Set` or `Dictionary`.

- **`T[]`** (typed arrays) — contiguous, fixed-length-after-creation; `length` property, indexing (panics OOB), and `foreach`; the engine-grade buffer. `[value; count]` is its only runtime-sized constructor. Slicing is a future addition.
- **`Bytes`** — owned mutable byte buffer for binary work; `uint8[]`↔`Bytes` interconvert only through explicit `Bytes::fromArray`/`->toArray` (copy in v1.0). Slice 2 provides `length`, indexed `uint8` reads/writes and RMW, and byte-wise `==`/`!=`; growable/slice/search members await a future method-surface record.
- **`List<T>`** — the everyday growable sequence and default workhorse: `add`, `insertAt`, `removeAt` (returns the owned element), `pop` (`?T`), `contains`, `first`/`last` (`?T` properties), `count` (property), `isEmpty` (property), and `map`/`filter`/`reduce` once closures land (Stage 30).
- **`Dictionary<K, V>` / `SortedDictionary<K, V>`** — `get` returning `?V`, `set`, `remove` returning `?V`, `containsKey` (key membership, decision 0113), `keys`/`values` (`foreach`-only projections, not storable in v1.0); `Dictionary` iterates in insertion order, `SortedDictionary` by ascending `Comparable` key. Keys are readonly; values may be writable through the main iteration form.
- **`Set<T>` / `SortedSet<T>`** — `::from`, `add` (`bool`), `remove` (`bool`), `contains`, `union`, `intersect`, `difference`; `Set` is insertion-ordered and `SortedSet` is ascending by `Comparable` element. Iteration is readonly: replacement is remove plus add. Existing-source construction and algebra preserve every input and require `Copy` or, once Stage 35 lands, `Cloneable`.
- **`PriorityQueue<T>`** — `push`, `pop` (`?T`), `peek` (`?T`), min-first by `Comparable`; duplicates are allowed, equal elements have no stable tie order, and there is no `foreach` (drain via `pop`). **`Deque<T>`** — `pushFront`/`pushBack`/`popFront`/`popBack`/`peekFront`/`peekBack`, readonly or writable `foreach` front-to-back; subsumes FIFO/LIFO, so there are no separate `Queue`/`Stack` types.

The four no-literal forms use positional-only `Type::from(source)`. Direct bracket assignment is not their construction surface. Existing sources remain unchanged; Stage 26 copies `Copy` values (including retained immutable strings), while move values are inserted individually into an empty destination. A consuming conversion remains deferred.

---

## Standard library modules (`Doria\Std\*`)
Hosted modules under the reserved `Doria\Std` namespace. Most are direction-only in the plan today; each links to its owning section/record and is marked *(surface TBD)* where its decision is unauthored.

- **`Doria\Std\Io`** — the decision-0110 Stage 36a byte-stream and standard-I/O foundation: small readable/writable/duplex/seekable/flushable/blocking/readiness capabilities rather than one god object; owned move handles with consuming checked close/finish and nonthrowing best-effort destruction; data/would-block/EOF/timed-out reads; partial-progress writes; first-class non-owning standard streams over the existing intrinsic substrate; one portable readiness, duration/deadline, cancellation, and backpressure model; typed file requests; per-value buffering, UTF-8 text/line adapters, bounded streaming copy, and process-pipe integration. Its accepted performance contract requires reusable caller/adapter buffers, safe byte regions, allocation-free common outcomes and standard-stream views, no mandatory allocation or whole-chunk copy per steady-state operation, reusable readiness storage, static specialization for concrete adapters, and zero async scheduler cost in synchronous programs. Stage 29 supplies checked errors but does not implement this layer. Exact interface, member, result-case, mode, byte-region, reusable-buffer, adapter, file, readiness, process, and standard-stream spellings remain deferred to the bounded decision-0110 appendix before Stage 36a implementation. The current text/binary free functions are *language intrinsics*, not proof that this module exists. *(Capability inventory: `docs/notes/io-surface-audit.md`; Stage 36a scheduled and not implemented; semantics and performance contract accepted, public spellings deferred.)*
- **`Doria\Std\Fs`** — filesystem/namespace operations without an open handle: existence, size/metadata, permissions, timestamps, rename, delete, `mkdir`, directory listing, and path manipulation. Decision 0110 fixes the `Io`/`Fs` boundary while deferring the exact `Path` representation to the filesystem design; Stage 36a must preserve typed-path evolution. *(Surface TBD.)*
- **`Doria\Std\Env`** — environment variables. *(Surface TBD.)*
- **`Doria\Std\Process`** — process facts (exit code, process id, executable path) and decision-0110 owned child processes with typed stdin/stdout/stderr pipes, stdin half-close, explicit wait/detach/terminate lifetime resolution, concurrent bounded output drainage, and shared timeout/readiness/cancellation integration. Destruction does not silently wait, detach, or terminate. Exact process and type-state spellings remain deferred to the process owner. The pipe capability depends on Stage 36a and is not currently executable. Command-line arguments arrive through `main(List<string> $args)` (decision 0099), not here.
- **`Doria\Std\Time`** — clock and time. Decision 0110 requires duration and absolute-deadline concepts, explicit immediate/indefinite waiting, and no mutable process-global stream timeout; exact public types remain owned by the Time design no later than Stage 36a surface finalization. *(Surface TBD.)*
- **`Doria\Std\Random`** — random-number generation. *(Surface TBD.)*
- **`Doria\Std\Json`** — JSON encode/decode; drives enum/match/mixed ergonomics and the PHP bridge. *(Surface TBD.)*
- **`Doria\Std\Net`** — networking, TCP first. Stage 44 builds its client/listener/socket and HTTP maturation on decision 0110's duplex byte streams, readiness, duration/deadline timing, cancellation, partial writes, backpressure, and async integration; socket streams do not define a second I/O contract. TLS is a typed network-adapter boundary over that shared foundation whose exact configuration remains Stage 44 work. *(Surface TBD.)*
- **`Doria\Std\Http`** — HTTP, later than `Net`. *(Surface TBD.)*
- **`Doria\Std\Data`** — **DDO**, the batteries-included database layer: decomposed `Connection` / `Statement` / `Transaction` / typed rows, the `Sql` provenance newtype, RAII transactions with consuming `commit`, capability-based drivers, typed fetches. Direction: plan §9 (DDO); prerequisites checked errors (Stage 29) + `Net`. *(The authoritative DDO record is unauthored; 0007 is a superseded sketch.)*
- **`Doria\Std\Term`** — the portable terminal layer for product 5, surfaced through the `Console` static facade. It reuses decision 0110's standard-stream views, readiness, blocking substrate, duration/deadline timing, cancellation, and platform-device abstraction; generic stream readiness is not reimplemented here. Raw mode and terminal events remain Term-owned, and mode changes require exclusive control plus a restoring guard. See "The terminal layer" in plan §9 and the Console/terminal decision. Surface (planned):
  - **terminal info** — size, interactivity, colour capability;
  - **screen** — clear, title (alternate-screen later);
  - **cursor** — position, move, show/hide;
  - **styled output**;
  - **input** — blocking `readKey`, non-blocking `pollKey`, resize events, decoded to payload enums (`KeyEvent::Char(string $char)`, `KeyEvent::Up`, …);
  - **raw mode** — entered through an ownership guard whose `__destruct` restores the terminal on every structured exit.
  Capability-based (no escape sequences or platform encodings in any public value); stateless (no `ScreenBuffer` std type). *(Method inventory settled in the Console/terminal decision, TermUtil-informed; lands Stage 46.)*
- **`Doria\Std\Math`** — batteries-included game/graphics math as built-in Copy value types: `Vector2`/`Vector3`/`Vector4`, `Quaternion`, `Euler`, `Matrix3x3`/`Matrix4x4`, plus `lerp`/`clamp`/easing helpers. Compiler-known arithmetic operators; `$v->length` / `$v->normalized` properties, `$v->dot`/`$v->cross` methods. Direction: plan §9 (math); lands Stage 47. *(Geometry-math record unauthored.)*

## Invalidated elsewhere

- The plan's Decision 0095 catalogue entry and Decision 0095's `Comparable<T>` consequence now include the typed comparison operand.
- The integer conversion inventory no longer presents an untyped parameter as a Doria signature; decision 0042 remains authoritative for the accepted integer-expression operand contract.
- The collection inventory now documents decision 0102's `[value; count]` sequence constructor and its v1.0 restrictions.
- Stage 23c implements the core-language sequence constructor described above;
  the `Doria\Std\*` module inventory remains a catalogue of planned APIs.
