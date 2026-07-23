# Standard library reference — the planned surface

> Documentation role: the at-a-glance catalogue of Doria's **core** and **standard-library** surface — every companion, interface, collection, free function, and `Doria\Std\*` module, with its purpose and known member surface. This is the *inventory*; the end-to-end plan §9 owns the *direction and rationale*, and each decision record owns the *precise contract*. Both are linked from every entry. This is the **planned** surface: some members are settled in a record, others are marked *(surface TBD in …)* until their decision is authored. It grows as decisions land — keep it in sync when a stdlib record is authored or amended.

Two layers (plan §9): **core** (no I/O, always available) and **std** (hosted under the reserved `Doria\Std` namespace, per the namespace-model decision). Both are written in Doria as early as self-hosting allows.

---

## Core layer (no I/O, always available)

### Primitive companions
Fixed-width numeric, bool, and string companion APIs — the member surface each primitive carries. Details: decisions 0013/0016 (numeric types), 0042 (conversions), 0095 (`pow`), 0096 (interface conformance); §4.6 (strings).

- **`Int`, `Int8`/`Int16`/`Int32`/`Int64`, `UInt8`/`UInt16`/`UInt32`/`UInt64`** — `Int::parse(string): ?int`, `Int::toFloat(int): float`, `Int::pow(...)`, wrapping arithmetic (`wrappingAdd`/`wrappingSub`/`wrappingMul`), and per-width `Int32::from($x)` (checked, panics on overflow) / `Int32::tryFrom($x): ?int32`.
- **`Float`, `Float32`/`Float64`** — `Float::parse(string): ?float`, `Float::toInt(float): int` (checked, panics on NaN/out-of-range), `Float::pow(...)`. `float` is neither `Hashable` nor totally `Comparable` (0096).
- **`Bool`** — companion helpers.
- **`String`** — `$s->length` (byte length), `$s->isEmpty`, `$s->bytes` (a `Bytes` view, copy in v1.0); `$s->chars` (grapheme iteration) is deferred. Plus the `str_*` free-function family (below).

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
The escape hatch when single ownership does not fit (caches, graphs, back-references). Details: §3.3, the shared-ownership decision (ships with the borrow checker).

- **`Shared<T>`** — refcounted shared owner (Rust `Rc`); `->clone()` copies the handle; access is readonly.
- **`Weak<T>`** — non-owning weak handle (breaks cycles).
- **`SharedMut<T>`** — shared mutable owner; emits dynamic access checks (the one place runtime borrow checks appear).

### Iteration
- **`Iterable<T>` / `Iterator<T>`** — the public iteration protocol that makes user types work with `foreach`; user conformance lands at Stage 35 (built-in collections use compiler-internal iteration earlier, Phase D).

### Ranges and math basics
- **Range types** — `a..b` (inclusive) / `a..<b` (exclusive-end); `int` endpoints; used with `foreach` (SPEC control flow).
- **`math` basics** — scalar math functions (the geometry/vector types live in `Doria\Std\Math`, below).

### PHP-familiar free-function layer
Regularized `snake_case` free functions wrapping the member/companion surface — never PHP's fused spellings. Details: decision 0074 (formatted I/O), §9.1 charter.

- **Formatting:** `sprintf(format, ...args): string`, `printf(format, ...args): void` — compile-time-checked literal format strings; specifiers `%s %d %f %.Nf %x %X %o %b %%`, width / `-` / `0` flags.
- **Text I/O:** `read_line(): ?string`, `read_file(string): string`, `write_file(string, string): void` (truncate), `append_file(string, string): void` (Stage 23), `write_stderr(string): void`.
- **Output statement:** `echo` (the single output spelling — `print` is rejected).
- **String/utility:** `get_time`, `str_starts_with`, `str_case_compare`, and the `str_*` family (fully worded after the `str_` prefix).
- **Meta:** `function_exists("name")` — const-evaluated compile-time predicate for guarded/polyfill declarations.

Binary (`read_file_bytes`/`write_file_bytes`/`append_file_bytes`, `read_stdin_bytes`, `write_stdout_bytes`/`write_stderr_bytes`) arrives with `Bytes` at Stage 23; the stream tier moves to `Doria\Std\Io` post-Stage-29. See the I/O audit (`docs/notes/io-surface-audit.md`) for the byte/stream surface still being finalized.

---

## Collections (core-language move types)
Owned move types with a growable/fixed distinction. The complete family and naming are settled in **decision 0092**; the method surface and `T[]` surface are settled in the collections decision (Stage 23–26). A bare name is the default (hash / insertion-ordered) collection; the `Sorted` prefix is the comparison-ordered variant.

- **`T[]`** (typed arrays) — contiguous, fixed-length-after-creation; `length` property, indexing, slicing, `foreach`; the engine-grade buffer.
- **`Bytes`** — mutable byte buffer for binary work; `uint8[]`↔`Bytes` interconvert only through explicit `Bytes::fromArray`/`->toArray` (copy in v1.0).
- **`List<T>`** — the everyday growable sequence and default workhorse: `add`, `insertAt`, `removeAt`, `contains`, `count` (property), `isEmpty` (property), and `map`/`filter`/`reduce` once closures land (Stage 30).
- **`Dictionary<K, V>` / `SortedDictionary<K, V>`** — `get` returning `?V`, `set`, `remove`, `has`, `keys`, `values`; `Dictionary` iterates in insertion order, `SortedDictionary` by `Comparable` key.
- **`Set<T>` / `SortedSet<T>`** — `add`, `remove`, `has`, `union`, `intersect`; `Set` insertion-ordered, `SortedSet` by `Comparable` element.
- **`PriorityQueue<T>`**, **`Deque<T>`** — `Deque` subsumes FIFO/LIFO, so there are no separate `Queue`/`Stack` types.

---

## Standard library modules (`Doria\Std\*`)
Hosted modules under the reserved `Doria\Std` namespace. Most are direction-only in the plan today; each links to its owning section/record and is marked *(surface TBD)* where its decision is unauthored.

- **`Doria\Std\Io`** — the post-Stage-29 `File`/stream objects: `read`/`write`/`seek`/`tell`/`flush`/`close` (RAII), buffered readers. The Stage 17 text free functions are *language intrinsics*, not this module. *(Stream surface: io-surface-audit; lands post-Stage-29.)*
- **`Doria\Std\Fs`** — filesystem/namespace operations without an open handle: existence, size/metadata, permissions, timestamps, rename, delete, `mkdir`, directory listing, and path manipulation (join/normalize/split; a `Path` type is a live design case). *(Surface TBD; the `Io`/`Fs` line is drawn in the I/O audit, D8.)*
- **`Doria\Std\Env`** — environment variables. *(Surface TBD.)*
- **`Doria\Std\Process`** — process facts: exit code, process id, executable path. Command-line arguments arrive through `main(List<string> $args)` (decision 0099), not here.
- **`Doria\Std\Time`** — clock and time. *(Surface TBD.)*
- **`Doria\Std\Random`** — random-number generation. *(Surface TBD.)*
- **`Doria\Std\Json`** — JSON encode/decode; drives enum/match/mixed ergonomics and the PHP bridge. *(Surface TBD.)*
- **`Doria\Std\Net`** — networking, TCP first. *(Surface TBD.)*
- **`Doria\Std\Http`** — HTTP, later than `Net`. *(Surface TBD.)*
- **`Doria\Std\Data`** — **DDO**, the batteries-included database layer: decomposed `Connection` / `Statement` / `Transaction` / typed rows, the `Sql` provenance newtype, RAII transactions with consuming `commit`, capability-based drivers, typed fetches. Direction: plan §9 (DDO); prerequisites checked errors (Stage 29) + `Net`. *(The authoritative DDO record is unauthored; 0007 is a superseded sketch.)*
- **`Doria\Std\Term`** — the portable terminal layer for product 5, surfaced through the `Console` static facade. See "The terminal layer" in plan §9 and the Console/terminal decision. Surface (planned):
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
- No compiler behavior or implemented standard-library surface changes; this file catalogues planned APIs.
