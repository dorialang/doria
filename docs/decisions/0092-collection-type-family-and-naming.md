# Decision 0092: Collection type family and naming

**Status:** Accepted (the type inventory, names, and ordering semantics are
settled here; method surfaces, literal forms, and runtime representation remain
with the collections-runtime decision).

## Context

Doria's named-collection family was `List<T>` / `Dictionary<K, V>` / `Set<T>`
with `Queue<T>` and `Stack<T>` reserved (§4.9, the D22 row). A completeness
review against Rust's `std::collections` and C#'s collection types found the
hash-and-sequence half well covered but three real gaps: no **sorted (ordered)**
map or set, no **priority** structure, and no **double-ended queue**. Sorted
iteration and range queries, priority scheduling and pathfinding (game tooling is
a first-class product), and work-queue/ring-buffer patterns had no home.

This record settles the type **inventory, names, and ordering semantics** so the
family is designed complete rather than patched later. It does **not** design the
method API or runtime representation — those stay with the collections-runtime
decision.

## Decision

### The family

- **Sequences:** `List<T>` (the growable workhorse), `T[]` (fixed-length buffer),
  `Bytes` (byte buffer) — unchanged.
- **Maps:** `Dictionary<K, V>` (default) and `SortedDictionary<K, V>`.
- **Sets:** `Set<T>` (default) and `SortedSet<T>`.
- **`PriorityQueue<T>`** — a binary-heap priority queue.
- **`Deque<T>`** — a double-ended queue.

The reserved `Queue<T>` and `Stack<T>` names are **retired**: `Deque<T>` serves
both FIFO and LIFO from one buffer (Rust's `VecDeque` precedent), so three narrow
types would be surface without capability.

### Naming rule

One scheme across maps and sets: **the bare name is the default (hash /
insertion-ordered) collection; the `Sorted` prefix is the comparison-ordered
variant** — `Dictionary`/`SortedDictionary`, `Set`/`SortedSet`. This matches C#
and keeps a single naming axis. No `HashMap`/`HashSet` spelling is introduced:
the bare name already *is* the hash collection, so a hash-prefixed alias would be
a redundant second spelling — the `print`/`echo` redundancy Doria bans.

### Ordering semantics

- `Dictionary` and `Set` iterate in **insertion order** — PHP/Python/modern-JS
  familiarity, so `foreach` is predictable. Keys/elements require `Hashable`.
- `SortedDictionary` iterates by ascending key; `SortedSet` by ascending element.
  Keys/elements require `Comparable`. These are the home for ordered iteration and
  range queries.
- `PriorityQueue<T>` orders by `Comparable` `T` — a **single** type parameter,
  not C#'s two-parameter element/priority split. A distinct priority is expressed
  through the element's own ordering (wrap the payload with a comparable key).
- `Deque<T>` preserves insertion order at both ends.

**Primitive key/element conformance (added post-acceptance).** This record
depends on primitives satisfying `Hashable`/`Comparable` — `int` and `string`
are the common keys — which decision 0096 settles: primitives conform to the core
value interfaces by compiler-known conformance and satisfy generic constraints
with no boxing. One consequence binds here: **`float` is neither `Hashable` nor
totally `Comparable`** (`NaN`, signed zero, per 0087), so a float cannot key a
`Dictionary` or `SortedDictionary`, nor be a `SortedSet` or `PriorityQueue`
element.

### Deferred to the collections-runtime decision (not decided here)

- Method surfaces, iteration machinery, and literal forms for the new types
  (`Set`-style `::from` construction is expected; the new named collections have
  no bracket-literal form).
- Runtime representation and performance (e.g. whether insertion-ordered
  `Dictionary` is an index-vector-plus-hash or a linked hash map).
- Whether a separate truly-unordered `HashMap`/`HashSet` for raw throughput is
  ever added — **deferred**; the insertion-ordered default suffices for v1.0, and
  an unordered variant is a profiling-driven addition, not a launch type.
- `T[]` / `Bytes` interconversion.

## Alternatives considered

- **`HashMap` (unsorted) + `Dictionary` (sorted):** rejected — inverts C#, where
  `Dictionary` is the hash map; silently redefines the existing
  `Dictionary<string, mixed>` usages (JSON, DDO, mixed flows) as key-sorted; and
  pairs a different-root scheme with the adjective-prefix `Set`/`SortedSet`.
- **Three types `Queue` / `Stack` / `Deque`:** rejected — `Deque<T>` serves LIFO
  and FIFO from one buffer.
- **Unordered-hash `Dictionary`/`Set` default** (Rust `HashMap` iteration order):
  rejected as the default — PHP developers rely on insertion order; a surprising
  iteration order is a familiarity tax. Unordered stays available later as an
  explicit variant if profiling demands.
- **C#-style two-parameter `PriorityQueue<TElement, TPriority>`:** rejected for
  v1.0 — a single `Comparable` type parameter is simpler.

## Consequences

- The collection family is named as a complete set now (sorted maps/sets, priority
  queue, deque), so the collections-runtime decision designs a coherent whole.
- `Dictionary` and `Set` keep their current meaning and every existing usage; no
  plan-wide rename.
- Insertion order for `Dictionary`/`Set` is a stated guarantee the runtime upholds.
- Sorted collections and `PriorityQueue` depend on the `Comparable` interface; the
  hash collections on `Hashable`.

## Sequencing

The default hash/sequence types land at the collections stage (Stage 23). The
sorted family, `PriorityQueue`, and `Deque` join the collections family with their
stage assigned by the collections-runtime decision (at or after Stage 23). Nothing
here is implementation; this fixes the family and its names.

Stage 26 has now discharged the implementation dependency for
`SortedDictionary`, `SortedSet`, `PriorityQueue`, and `Deque`. Their accepted
names, order, and single-parameter priority-queue shape are unchanged; decision
0100 owns the implemented member and ownership surface.

## Affected components

Plan §4.9 and the D22 row (the named-collection list); the §9.1 collection-method
note; the `Stack<T>` reservation aside; the collections-runtime decision (consumes
this family). SPEC is updated when the types are implemented, not now — it tracks
the current language, and these are future types. No compiler code lands with this
record.

## Invalidated elsewhere

- The plan's "future: `Queue<T>`, `Stack<T>`" reservation — replaced by the full
  family; `Queue`/`Stack` retired in favor of `Deque<T>`.
- The `class History<T> // deliberately not Stack<T>` aside — retargeted to
  `Deque<T>`, the now-reserved name.
- Any assumption that `Dictionary`/`Set` iteration order is unspecified — it is
  insertion order.
- Any pipeline note that still describes the sorted/priority/deque family as
  unimplemented — Stage 26 completes that family without adding aliases.
