# Decision 0100: Collection method surface

**Status:** Accepted (the method surface, receiver/ownership modes, and
missing-element contract for the whole collection family are settled here).
Consumes decision 0092's type inventory; feeds the Stage 23 collections-runtime
implementation.

## Context

Decision 0092 settled the collection **family, names, and ordering semantics**
(`List`/`Dictionary`/`Set`, the `Sorted` variants, `PriorityQueue`, `Deque`, and
the `T[]`/`Bytes` sequences) and explicitly deferred the **method surface**,
literal forms, and runtime representation to "the collections-runtime decision."
This record is that decision's method-surface half. Stage 23 implements the
default hash/sequence collections, so their surface must be authored before the
implementation lands rather than invented ad hoc in the compiler.

The surface is not free-form: Doria's ownership model decides more than the names.
Every collection is a move type, so each member is defined by (1) its **receiver
mode** — `readonly` for reads, `writable` for mutations; (2) whether it **moves a
value in** (ingestion) or **hands ownership back** (removal) or **borrows** (in-place
access); and (3) whether it can **miss**, in which case it returns `?T` and narrows
through the Stage 22 flow model. Naming follows the §9.1 charter.

## Decision

### Cross-cutting rules (normative — apply to every type below)

- **Receiver modes.** Reads (`get`, `contains`, `has`, `keys`, `values`, `first`,
  `last`, `peekFront`/`peekBack`/`peek`, the set-algebra methods) take a
  **`readonly`** receiver. Mutations (`add`, `insertAt`, `removeAt`, `pop`, `set`,
  `remove`, `push`, `pushFront`/`pushBack`, `popFront`/`popBack`) take a
  **`writable`** receiver. `count`, `isEmpty`, and `length` are **readonly
  properties**, not calls.
- **Ingestion moves the value in.** `add`, `insertAt`, `set`, `push`, and the
  deque/queue push family take their value argument by **move (`take`)**. For a
  `Copy` type (`int`, `bool`, fixed-width numerics, …) the move is a copy and the
  argument stays usable; for a move type the argument is consumed and using it
  afterward is ordinary use-after-move.
- **In-place access borrows.** `$l[i]`, `first`, `last`, `get`, `keys`, `values`,
  and the `peek*` family yield **borrows** of elements — readonly by default,
  writable in a writable indexed place (`$l[i]++`, `$l[i] = x`) or through
  `foreach ... as writable`. A borrow cannot outlive its collection. Where access
  can miss, the borrow is nullable (`?T`) and narrows via Stage 22 flow.
- **Removal hands ownership back.** `removeAt`, `pop`, `remove`, and the
  `pop*`/`popFront`/`popBack` family **move the element out** of the collection
  and transfer **owned** ownership to the caller — nullable (`?T`) where the
  removal can find nothing. This is why removals return values, not `bool`; the
  one exception is `Set`, whose elements are not distinct owned payloads worth
  returning, so `Set::remove` returns `bool`.
- **The missing-element contract: assertive index panics, `?T` is the safe form.**
  Reading `$l[i]` or `$d[k]` **asserts the element is present** and **panics** if
  it is not (out-of-bounds index, absent key) — the same contract §4.9 already
  fixes for `T[]`. The nullable methods (`get`, `first`, `last`, `pop`, the
  `peek*`/`pop*` family) are the safe path: they return `?T` and never panic on
  absence. Index reads and index-assignment (`$d[k] = v`, insert-or-update) are
  the assertive idiom; `get` is the "might miss" idiom.
- **Mutators return `void`; fluency is a userland capability, not a built-in one.**
  Every built-in mutator returns `void` (or the moved-out value for removals);
  none returns a writable `self` for chaining. This is deliberate for v1.0: the
  Stage 23 idiom is statement-position mutation and in-place loops, and threading
  writable-self returns through every collection mutator is surface the language
  does not yet need. **This does not restrict fluent APIs** — decision 0088's three
  chaining conventions remain fully available for *user-defined* types, which was
  0088's actual purpose (that users *can* build fluent interfaces, not that the
  built-in collections *must* expose them). Built-in fluent mutators are deferred,
  not precluded; if a concrete need appears they can be added under 0088
  convention 2 without breaking `void` callers.
- **Key/element constraints.** `Dictionary`/`Set` keys and elements require
  `Hashable`; `SortedDictionary`/`SortedSet`/`PriorityQueue` require `Comparable`
  (decisions 0092, 0096). `float` is neither, so it cannot key a map nor be a
  sorted-set / priority-queue element (0092, 0096).
- **`count` vs `length`.** Collections expose `count`; `T[]` and `string` expose
  `length`. The split is deliberate (matching C#'s `.Count` vs `.Length`): `length`
  is the fixed extent of a buffer, `count` is the live size of a growable/keyed
  structure.

### `List<T>`

| Member | Receiver / kind | Contract |
| --- | --- | --- |
| `$l[i]` read | readonly borrow | element borrow; **panics** out of bounds |
| `$l[i] = x`, `$l[i]++` | writable place | in-place write / read-modify-write (§3.2) |
| `add(T): void` | writable, moves in | append |
| `insertAt(int $index, T): void` | writable, moves in | insert; panics on out-of-range index |
| `removeAt(int $index): T` | writable | removes and returns the **owned** element; panics out of bounds |
| `pop(): ?T` | writable | removes and returns the last element; `null` if empty |
| `contains(T): bool` | readonly | value membership |
| `first: ?T` / `last: ?T` | readonly properties | safe end borrows; `null` if empty |
| `count` / `isEmpty` | readonly properties | live size / emptiness |
| `map` / `filter` / `reduce` | — | **deferred to Stage 30** (require closures) |

### `Dictionary<K, V>`

| Member | Receiver / kind | Contract |
| --- | --- | --- |
| `get(K): ?V` | readonly borrow | safe lookup; `null` if absent |
| `$d[k]` read | readonly borrow | **panics** if the key is absent |
| `set(K, V): void`, `$d[k] = v` | writable, moves in | insert-or-update |
| `remove(K): ?V` | writable | removes and returns the **owned** value; `null` if absent |
| `has(K): bool` | readonly | key membership |
| `keys` / `values` | readonly projections | `foreach`-only, insertion order; **not storable** (see below) |
| `count` / `isEmpty` | readonly properties | |
| `foreach ($d as $k => $v)` / `as $v` | readonly / writable borrows | **insertion order** (0092) |

**`keys` / `values` are `foreach`-only projections, not storable values (v1.0).**
Each is a readonly, insertion-ordered projection of the dictionary's keys or
values, usable **only as the iterable in a `foreach` head** — `foreach ($d->keys
as $k)`, `foreach ($d->values as $v)` — where it borrows `$d` for the loop's
duration and yields element borrows. It has **no nameable, storable type in
v1.0**: binding one (`let $ks = $d->keys`) or using it anywhere but a `foreach`
iterable position is a diagnostic directing the user to iterate it, or to build an
owned copy explicitly. This deliberately avoids introducing a bespoke view type
ahead of the general iteration protocol (`Iterable<T>`/`Iterator<T>`, Stage 35):
when that protocol lands, `keys`/`values` can be **upgraded** to return a
first-class storable iterator **without breaking any `foreach` caller** (an
additive change). An owned copy is an explicit, later, copying operation
(`List::from($d->keys)` once projections are accepted `::from` sources) — deferred,
not v1.0. Writable value mutation stays on the main `foreach ($d as $k => $v)`
form; `values` is readonly.

### `Set<T>`

| Member | Receiver / kind | Contract |
| --- | --- | --- |
| `Set::from(T[] \| List<T>): Set<T>` | constructor | explicit construction (no bracket literal, per 0092) |
| `add(T): bool` | writable, moves in | `true` if newly inserted, `false` if already present |
| `remove(T): bool` | writable | `true` if it was present |
| `contains(T): bool` | readonly | membership |
| `union` / `intersect` / `difference(Set<T>): Set<T>` | readonly | return a **new owned** set; receiver and argument unchanged |
| `count` / `isEmpty` | readonly properties | |
| `foreach ($s as $e)` | readonly / writable borrow | insertion order (0092) |

`isSubsetOf` / `isSupersetOf` predicates are deferred (not required for v1.0).

### The rest of the family (designed complete; scheduled after the defaults)

The surface is fixed now so the family is coherent; only the default hash/sequence
types (`List`/`Dictionary`/`Set`/`T[]`) land at Stage 23. The remainder land with
their types, at or after Stage 23 per the collections-runtime rollout.

- **`SortedDictionary<K, V>` / `SortedSet<T>`** carry the **same member surface**
  as `Dictionary` / `Set`; only iteration order differs (ascending by `Comparable`
  key/element, 0092). Range/slice queries over the ordering are a deferred
  addition, not part of the v1.0 surface.
- **`PriorityQueue<T>`** (`Comparable T`): `push(T): void`, `pop(): ?T`,
  `peek: ?T`, `count` / `isEmpty`. **`pop`/`peek` return the smallest element**
  by `Comparable` order (min-first). This is chosen for the pathfinding/scheduling
  workloads a game runtime wants (Dijkstra/A* expand the minimum) and matches C#'s
  `PriorityQueue`; it is called out explicitly because Rust's `BinaryHeap` is
  max-first, so the polarity is a deliberate, documented choice. A max-first need
  is expressed by wrapping the element in a reverse-ordering key. `PriorityQueue`
  has **no `foreach`** in v1.0 — its internal order is heap order, not a meaningful
  iteration order; drain it with `pop`.
- **`Deque<T>`**: `pushFront(T): void`, `pushBack(T): void`, `popFront(): ?T`,
  `popBack(): ?T`, `peekFront: ?T`, `peekBack: ?T`, `count` / `isEmpty`;
  `foreach` iterates front to back. `Deque` subsumes FIFO (push back / pop front)
  and LIFO (push back / pop back), so there is no separate `Queue`/`Stack` surface
  (0092).

### Construction

Bracket literals build `List`/`Dictionary`/`T[]` by context typing (§4.9). `::from`
is the explicit constructor from an existing sequence: it is **required** for
`Set` (which has no literal form) and available as the equivalent explicit form
for the others. Capacity hints (`withCapacity`) and other performance-shaped
constructors are a profiling-driven addition, deferred with the runtime
representation (0092).

## Alternatives considered

- **Non-panicking indexed reads (`$l[i]`/`$d[k]` return `?T`).** Rejected — it
  would make every element access nullable and force narrowing on the common
  assertive path; §4.9 already fixes panic-on-out-of-bounds for `T[]`, and the
  `get`/`first`/`last`/`pop` family already gives the safe `?T` idiom. Two clear
  idioms beat one blurred one.
- **`bool`-returning removals (C#/`Dictionary.Remove` style).** Rejected for the
  move-type collections — the element is being moved out anyway, so returning it
  is free and strictly more useful than discarding it. `Set::remove` keeps `bool`
  because its elements are membership facts, not owned payloads.
- **Fluent (`writable self`) built-in mutators.** Rejected for v1.0 — statement
  mutation is the idiom the stage needs, and userland fluent APIs are already
  served by 0088. Deferred, not precluded.
- **Uniform `has` / uniform `contains`.** Rejected — `contains(value)` for the
  sequence/set membership and `has(key)` for the map read differently on purpose:
  the verb signals value-membership versus key-presence.
- **Two-parameter `PriorityQueue<TElement, TPriority>` and max-first polarity.**
  Rejected per 0092 (single `Comparable` parameter) and above (min-first for the
  target workloads).

## Consequences

- The Stage 23 collections implementation builds against an authored surface: the
  default `List`/`Dictionary`/`Set`/`T[]` members, their receiver/ownership modes,
  and the missing-element contract are fixed, not improvised.
- The whole 0092 family has a coherent surface, so the sorted variants,
  `PriorityQueue`, and `Deque` slot in without a second design pass.
- `map`/`filter`/`reduce` are named but scheduled at Stage 30 with closures; until
  then a call to them parses and yields a stage-named unsupported diagnostic under
  the two-clocks rule.
- The `void`-mutator choice and 0088's fluent-API conventions are explicitly
  reconciled: built-in collections are not fluent, user types may be.

## Sequencing

The default `List`/`Dictionary`/`Set` surface (minus `map`/`filter`/`reduce`) and
`T[]` land at Stage 23 (the collections-runtime slice). `SortedDictionary`,
`SortedSet`, `PriorityQueue`, and `Deque` land with their types, at or after Stage
23 per the collections-runtime rollout. `map`/`filter`/`reduce` land at Stage 30
with closures. Nothing here changes the type inventory or naming (0092).

## Affected components

Semantic analysis (member resolution, receiver-mode and move/borrow checking for
collection members), the collections-runtime implementation in `doria-rt`, shared
MIR validation (indexed-place and move/borrow invariants), diagnostics
(missing-element panic messages, `map`/`filter`/`reduce` stage-named
unsupported), plan §4.9 and §9, the stdlib reference, and the durable parity
fixtures. SPEC is updated when the members are implemented, not now.

## Invalidated elsewhere

- Decision 0092's deferral of the method surface to "the collections-runtime
  decision" is **discharged by this record**; 0092's type inventory, names, and
  ordering are unchanged and remain authoritative.
- The stdlib reference's collection entries — reconcile the membership verb
  (`Set` uses `contains`, not `has`), add the members fixed here (`removeAt`,
  `pop`, `first`/`last`, `insertAt`, `remove` returning `?V`, `difference`, the
  `peek*`/`pop*` families), and cite 0100 as the owning record.
- The Stage 23 collections implementation prompt/slice — its member set is now the
  authored default surface here, no longer a pre-authorization minimum.
- Any assumption that built-in collection mutators might be fluent — they return
  `void`; fluency lives in userland via 0088.
