# Decision 0113: Collection Surface Completion

- **Status:** Complete
- **Accepted:** 2026-08-06 by Andrew Masiye
- **Date:** 2026-08-05
- **Owners:** Doria language and compiler design
- **Scope:** The complete member surface of the seven named collections, the one
  membership-naming law that governs them, and the surface's diagnostics
- **Relationship to 0100:** **Amends** Decision 0100. 0100's cross-cutting rules
  (receiver modes, move-in ingestion, borrow-returning reads, removal handing
  ownership back, the assertive-index / `?T` missing-element contract, `void`
  mutators, `count` vs `length`) are **unchanged and remain authoritative**.
  This record replaces 0100's membership naming, adds the members 0100 left out,
  and converts 0100's open deferrals into settled exclusions.

## Context

The collection surface has been built in successive passes, each adding the
members one slice needed. The passes keep missing members that the family as a
whole ought to have, because there has been no complete picture to check
coverage against and no single naming law that resolves cleanly.

`docs/notes/collection-surface-audit.md` is that picture, measured against the
compiler rather than the documentation. Two findings drive this record.

**First, the naming law contradicts itself.** Plan §9.1 names `hasKey` as its
predicate exemplar in one bullet and mandates "`contains` everywhere" in the
next. Decision 0100 then chose `has` — a third spelling appearing in neither
bullet — and explicitly rejected the uniformity rule to do it. Three
authoritative documents, three names, one concept.

**Second, that contradiction produces wrong programs.**
`Dictionary::has(K): bool` exists and works, but users who know
`List::contains` and `Set::contains` reach for `containsKey`, receive a bare
E0521 with no fixit, conclude the capability is missing, and write
`$dict->get($key) != null`. For a `Dictionary<K, ?V>` that workaround is
**semantically wrong**: it reports a key bound to `null` as absent. The language
had the right operation and the naming hid it.

The remaining gaps are ranked in the audit by one criterion — does the absence
force a clumsy or semantically wrong workaround — and this record settles all of
them, including the ones it declines, so the next pass does not re-litigate
them.

## Decision

### 1. The membership law (normative)

**`contains` is the root verb for every membership question in the collection
family.** Where a type has exactly one membership axis, the spelling is bare
`contains`. Where a type has more than one, a suffix names the axis. The root
never changes.

| Type                                                                       | Membership axes     | Spelling                                         |
|----------------------------------------------------------------------------|---------------------|--------------------------------------------------|
| `List<T>`, `Set<T>`, `SortedSet<T>`, `Deque<T>`, `PriorityQueue<T>`, `T[]` | one — the element   | `contains(T): bool`                              |
| `Dictionary<K, V>`, `SortedDictionary<K, V>`                               | two — key and value | `containsKey(K): bool`, `containsValue(V): bool` |

`Dictionary::has` and `SortedDictionary::has` are **renamed** to `containsKey`.

**This is a rename, not an addition.** The member count on both map types is
unchanged; `containsKey` *is* `has` under the spelling the naming law requires.
A reviewer who reads this as "add `containsKey` alongside the existing `has`"
would correctly object that it duplicates a working method — that objection has
already been raised once, and it is an objection to a proposal this record does
not make.

There is no `has` alias and no deprecation window: the compiler, language
server, and website are unreleased, so a clean rename costs nothing and a
lingering alias would preserve exactly the two-spellings-for-one-concept
ambiguity this record exists to remove. `has` becomes an unknown member that the
fixit table in §6 redirects.

The alternative of **leaving the split as it is and documenting it** is a real
option and is weighed in the alternatives below.

This resolves the contradiction in favor of §9.1's "one name per concept"
bullet, because that bullet is the one *about* this question and names
`contains` explicitly. The predicate bullet's `is`/`has`/`can` guidance is
retained for argument-free property predicates (`isEmpty`), which is the form it
actually describes.

### 2. Members added

Every member below is `readonly` unless marked, follows 0100's unchanged
cross-cutting rules, and returns the types shown.

| Member                         | On                                    | Contract                                                                                                                                                       |
|--------------------------------|---------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `contains(T): bool`            | `Deque<T>`, `PriorityQueue<T>`, `T[]` | Element membership. O(n)                                                                                                                                       |
| `containsKey(K): bool`         | `Dictionary`, `SortedDictionary`      | Renamed from `has`. O(log n) on the finalized sorted form, O(n) otherwise                                                                                      |
| `containsValue(V): bool`       | `Dictionary`, `SortedDictionary`      | Value membership. **O(n)** on both map types — documented, not hidden                                                                                          |
| `indexOf(T): ?int`             | `List<T>`                             | Position of the first equal element; `null` if absent. **Not** `-1` — 0100's `?T` contract governs                                                             |
| `remove(T): bool` *(writable)* | `List<T>`                             | Removes the first equal element; `true` if one was removed                                                                                                     |
| `first: ?T` / `last: ?T`       | `Set<T>`, `SortedSet<T>`              | Readonly properties. Iteration-order endpoints: insertion order for `Set`, ascending for `SortedSet`, so `SortedSet::first`/`last` are its minimum and maximum |
| `clear(): void` *(writable)*   | all seven                             | Empties in place, releasing every element exactly once. Length becomes 0; capacity is unspecified                                                              |

Three of these need justification beyond the audit's ranking:

**`List::remove(T)` returns `bool`, not the owned element.** This is the one
place this record extends 0100's `Set` exception rather than its general rule.
Searching by value requires `Equatable`, so the removed element is by
construction equal to the argument the caller already holds; returning it adds
nothing a caller does not have. `removeAt(int)` remains the positional form and
keeps returning the owned element, because there the caller has no equal value.

**`clear()` is not redundant with reassignment.** `$l = []` empties a *local*,
but a collection reached through a property path or a writable shared access
object has no in-place empty at all — and 0100 deliberately distinguishes
mutating `$this->items` from replacing it. `clear()` is the only spelling of
in-place emptying, which is why it ranks above its apparent workaround.

**`PriorityQueue::contains` is consistent with having no `foreach`.** 0100
withholds iteration because heap order is not a meaningful *order*. Membership
asks no question about order, and without it `PriorityQueue` is the only type in
the family with no non-destructive way to inspect its contents at all.

### 3. Naming conflicts this record settles

Stated explicitly so they are not reopened:

- **`Set` and `SortedSet` keep bare `contains`.** One axis, no suffix. Adding
  `containsValue` for symmetry with maps would imply a second axis that does not
  exist.
- **`Deque` does not get `first`/`last`.** It already has `peekFront`/`peekBack`,
  which pair with `pushFront`/`pushBack`/`popFront`/`popBack`. Two spellings for
  one concept is exactly what §9.1 forbids; queue vocabulary wins on the queue
  type.
- **`PriorityQueue` does not get `first`/`last`.** It has `peek`, and "last" has
  no meaning in heap order.
- **The map types do not get `first`/`last`.** A map endpoint is a key-value
  pair, and Doria has no pair value in v1.0. Revisit with the Stage 35 iteration
  protocol, not before.
- **`T[]` gets `contains` and nothing else here.** The uniformity law requires
  it; the rest of the typed-array surface stays owned by §4.9.
- **`count` remains the one size name** and `length` remains buffer-only.
  Unchanged from 0100 and §9.1.

### 4. Deliberately excluded (settled, not deferred)

Each of these was measured against the "forces a clumsy or wrong workaround"
criterion and **failed it**. They are closed, with the reason recorded, so a
future pass does not re-propose them as oversights.

| Excluded                                              | Why                                                                                                                                                                                              |
|-------------------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `getOrDefault`, `tryGetValue`, entry APIs             | `$d->get($k) ?? $default` already expresses this exactly. A method would be a second spelling for one idea                                                                                       |
| `isSubsetOf`, `isSupersetOf`, `overlaps`, `setEquals` | `$a->difference($b)->isEmpty` is correct and readable. It costs one allocation, which is a performance argument, not an ergonomics one — reopen it with profiling evidence, not by analogy to C# |
| `addAll`, `extend`, `merge`                           | `foreach ($src as $v) { $dst->add($v); }` is three clear lines, and bulk-move ownership is genuinely unsettled until `Cloneable` at Stage 35                                                     |
| `sort`, `sortBy`, `reverse`                           | Ordering is what `SortedSet`/`SortedDictionary` are for; comparator-driven sorting belongs with closures at Stage 30                                                                             |
| `Deque::remove`, `removeAt`, `insertAt`               | Middle mutation is not what a deque is for, and the runtime's shift path is not ring-aware (see §7)                                                                                              |
| Range and slice queries over sorted types             | 0100 deferred these; they remain deferred, and they need a range/slice value type that v1.0 does not have                                                                                        |
| `capacity`, `withCapacity`, `shrink`                  | Performance-shaped surface, deferred with the runtime representation per 0092. Profiling-driven, not now                                                                                         |
| Fluent mutators                                       | Unchanged from 0100: mutators return `void`; userland fluency is Decision 0088's capability                                                                                                      |
| `List::from`, `Dictionary::from`, `T[]::from`         | **Withdrawn from 0100, not implemented** — see below                                                                                                                                             |

`map` / `filter` / `reduce` are unchanged — named, and scheduled at Stage 30
with closures.

**`::from` on the literal-constructible types is withdrawn.** 0100 states that
`::from` is "also available as the equivalent explicit form" for `List`,
`Dictionary`, and `T[]`. That claim should be **removed from 0100 rather than
implemented**, for three reasons.

`List` and `Dictionary` exist to de-conflate PHP's single `array` type, and the
bracket literal is precisely the safe, typed replacement for it. Construction is
already covered end to end, including the empty case:

```doria
writable Dictionary<string, int> $myDict = [];
writable List<int> $items = [];
```

Adding `Dictionary::from(...)` would put a second spelling on the one concept
that already has a first-class one — the same duplication this record rejects
for `has` / `containsKey`. The rule has to apply to construction too, or it is
not a rule.

`::from` remains **required and unchanged** for `Set`, `SortedSet`,
`SortedDictionary`, `PriorityQueue`, and `Deque`, where it is the only
construction path because those types have no literal form. That asymmetry is
the point rather than an inconsistency: `::from` is the constructor for types a
literal cannot build.

For `T[]` the claim is not merely unnecessary but **syntactically impossible** —
`int[]::from([1, 2])` does not parse, and there is no spelling for a static call
on an array type. 0100 asserts a form the grammar cannot express.

The cross-type conversion case (a `List` from a `Set`, say) is real, but it is
served by `foreach ($source as $v) { $target->add($v); }` — three clear lines,
and the same standard that excludes `addAll` above. When the Stage 35 iteration
protocol makes `keys` / `values` storable, conversion is worth revisiting as
**one general mechanism**, not as a per-type `::from` overload added now.

**Representation is explicitly out of scope for this record.** The audit's §5
establishes by measurement that `Dictionary` and `Set` are unindexed flat arrays
with no hash table anywhere in the runtime, that their lookups are therefore
O(n) and roughly 200–290× slower than the `Sorted` variants at 4,000 elements,
and that building a `Set` is quadratic. That is a more consequential finding
than anything in this record, and fixing it touches runtime layout, the internal
ABI, 0092's insertion-order guarantee, and every backend. Folding it into a
method-surface decision would be exactly the kind of scope smuggling that makes
a record impossible to review. It needs its own decision alongside the Stage 26b
performance work.

This record therefore states each member's complexity honestly at today's
representation and does not promise any of them will get faster. Note that the
added members are not the slow ones: `containsKey` has the same cost as the
`has` it renames, and the O(n) cost of `containsValue`, `contains`, and
`indexOf` is the cost every unordered lookup in the family already pays.

### 5. The complete surface after this record

This table is the coverage checklist. A future pass that believes the surface is
incomplete checks against this, not against a peer language.

| Member                               | List | Dict | Set | SDict | SSet | PQ  | Deque |
|--------------------------------------|------|------|-----|-------|------|-----|-------|
| `count`                              | yes  | yes  | yes | yes   | yes  | yes | yes   |
| `isEmpty`                            | yes  | yes  | yes | yes   | yes  | yes | yes   |
| `first` / `last`                     | yes  | —    | yes | —     | yes  | —   | —     |
| `peek`                               | —    | —    | —   | —     | —    | yes | —     |
| `peekFront` / `peekBack`             | —    | —    | —   | —     | —    | —   | yes   |
| `keys` / `values`                    | —    | yes  | —   | yes   | —    | —   | —     |
| `contains`                           | yes  | —    | yes | —     | yes  | yes | yes   |
| `containsKey`                        | —    | yes  | —   | yes   | —    | —   | —     |
| `containsValue`                      | —    | yes  | —   | yes   | —    | —   | —     |
| `indexOf`                            | yes  | —    | —   | —     | —    | —   | —     |
| `get` / `set`                        | —    | yes  | —   | yes   | —    | —   | —     |
| `add`                                | yes  | —    | yes | —     | yes  | —   | —     |
| `insertAt` / `removeAt`              | yes  | —    | —   | —     | —    | —   | —     |
| `remove`                             | yes  | yes  | yes | yes   | yes  | —   | —     |
| `pop`                                | yes  | —    | —   | —     | —    | yes | —     |
| `push`                               | —    | —    | —   | —     | —    | yes | —     |
| `pushFront` / `pushBack`             | —    | —    | —   | —     | —    | —   | yes   |
| `popFront` / `popBack`               | —    | —    | —   | —     | —    | —   | yes   |
| `union` / `intersect` / `difference` | —    | —    | yes | —     | yes  | —   | —     |
| `clear`                              | yes  | yes  | yes | yes   | yes  | yes | yes   |
| `::from`                             | —    | —    | yes | yes   | yes  | yes | yes   |
| `map` / `filter` / `reduce`          | S30  | S30  | S30 | S30   | S30  | S30 | S30   |

`T[]` carries `length`, indexing, `foreach`, and `contains`; it is built by
literal only. `List` and `Dictionary` are likewise literal-only. `Bytes`
is unchanged and stays owned by the future Bytes method-surface record.

### 6. Diagnostics (normative)

The surface is only as discoverable as its diagnostics, which is the whole
lesson of the reported symptom. Three requirements:

- **E0521 must carry a did-you-mean fixit.** A collection member miss is
  resolved against a peer-spelling table covering at minimum: `has`,
  `hasKey`, `array_key_exists`, `contains_key`, `ContainsKey` → `containsKey`;
  `in_array`, `includes` → `contains`; `size`, `Count`, `len`, `length` →
  `count`; `push`, `append` → `add`; `array_search`, `position`, `find` →
  `indexOf`; `unset`, `delete` → `remove`; `Min`/`Max` → `first`/`last`;
  `Enqueue`/`Dequeue` → the `Deque` push/pop family. Per AGENTS.md this table
  lives in shared compiler data so `doriac migrate php` reuses it rather than
  maintaining a second copy.
- **A property invoked as a method gets its own diagnostic**, not E0521.
  `$d->isEmpty()` must say `isEmpty` is a property and offer removing the
  parentheses as a machine-applicable fix.
- **`check_stage23_equatable_type` must be parameterized.** Its message
  hardcodes "List::contains" and will name the wrong type once `contains`
  widens.
- **`List::from(...)` and `Dictionary::from(...)` must teach the literal.**
  Today they report ``E0305: Unknown Class `List` `` — the compiler denies the
  type exists at all, which is actively misleading. Because §4 withdraws these
  constructors rather than implementing them, this diagnostic is the *only*
  thing a user meets, so it must carry a machine-applicable fix rewriting
  `List::from([1, 2])` to a typed literal. A withdrawn form with a good
  diagnostic is a better outcome than an implemented duplicate; a withdrawn form
  with today's diagnostic is worse than either.

### 7. Implementation notes that bind

Recorded here because getting them wrong is silent corruption, not a test
failure.

- **`Deque` is a ring buffer.** `remove_at`, `insert_at`, and `remove_value` in
  `crates/doria-rt/src/collection.rs` shift with one linear `ptr::copy` between
  two `value_address` results, which is correct only for contiguous storage. On
  a wrapped deque the regions are not adjacent and the copy overruns. This is
  why §4 excludes `Deque` middle mutation. `contains` is safe because
  `value_address` applies the head translation per element.
- **Most of this is not runtime work.** `contains` on `Deque`/`PriorityQueue`/
  `T[]` needs only widened gates in `semantics.rs` and `mir_lowering.rs` — the
  backends already dispatch to `COLLECTION_CONTAINS` for any collection with no
  key type. `containsValue` needs a new `CollectionMembershipOp` variant to
  force the value path past the existing key-presence dispatch, but no runtime
  function. `List::remove` reuses `remove_value`. `Set`/`SortedSet` `first`/
  `last` reuse `NullableCollectionAccess::First`/`Last`.
- **`clear()` is the one member with real per-backend cost.** The runtime cannot
  release elements — `collection::free` deallocates buffers only, and element
  release is compiler-emitted drop glue. `clear()` therefore needs an
  element-release loop in each of the four backends plus the interpreter, and
  must release exactly once.
- **The PHP backend inherits its current behavior; this record does not change
  it.** `is_stage23_runtime_type` in `codegen_php.rs` blanket-refuses every
  member on `List`, `Dictionary`, `Set`, `T[]`, and `Bytes` with B2301, while
  the Stage 26 ordered types are fully implemented as hand-written PHP classes.
  New members on the ordered types must be implemented there; new members on the
  default types inherit the existing refusal. An implementer must not read the
  B2301s as a regression they introduced.

## Alternatives considered

- **Leave the `contains` / `has` split exactly as it is and document it.** The
  null option, and the cheapest — it costs no compiler change at all, and the
  split is arguably meaningful: `contains(value)` and `has(key)` do ask
  different questions. Rejected on evidence rather than principle. The split has
  already produced one wrong program in the field: a user who knew
  `List::contains` reached for `containsKey`, got a bare E0521, concluded the
  capability was absent, and wrote a `get() != null` test that is incorrect for
  nullable value types. Documentation does not reach a user who does not know
  there is something to look up. If this option is chosen anyway, the §6 fixit
  work becomes **mandatory rather than supporting**, because the fixit is then
  the only thing standing between a user and the same wrong program.
- **Keep `has` and add `containsKey` as an alias.** Rejected — two spellings for
  one concept is the defect, not the fix, and §9.1 forbids it. Nothing is
  released, so there is no compatibility argument for the alias.
- **`hasKey` / `hasValue`, following §9.1's predicate exemplar literally.**
  Rejected, but it is the closest runner-up and the one genuine fork in this
  record. It satisfies the `is`/`has`/`can` bullet but breaks "one name per
  concept": membership would read as `contains` on five types and `hasKey` on
  two, which is the same discoverability trap in a new spelling. `containsKey`
  additionally matches all three peers. **Flipping this choice means editing §1,
  §5, and the §6 fixit table and nothing else.**
- **Uniform bare `contains(K)` on maps.** Rejected — a map has two membership
  axes and a bare verb cannot say which one it means. PHP, Rust, and C# each
  independently refused this.
- **`indexOf` returning `-1` when absent.** Rejected — 0100's missing-element
  contract makes `?T` the safe form, and a sentinel integer that is also a valid
  arithmetic value is exactly the trap `?int` exists to remove.
- **Leaving `PriorityQueue` without `contains` for consistency with having no
  `foreach`.** Rejected — the `foreach` exclusion is about order, and membership
  needs none. Consistency of *reasoning* beats consistency of *shape*.
- **Adding the set-relation predicates for C# parity.** Rejected — parity is not
  a criterion. The workaround is correct and readable, and the only cost is an
  allocation, which is a profiling question.

## Consequences

- One law now covers the whole family, so the next pass has a checklist (§5)
  rather than a peer language to guess against.
- The reported symptom is fixed twice over: `containsKey` becomes the spelling
  users already reach for, and E0521 gains the fixit that would have prevented
  the wrong workaround even under the old name.
- `has` disappears. Every occurrence in the compiler, tests, fixtures, examples,
  documentation, and the language server must move in one beat.
- The exclusions in §4 are decisions with recorded reasons, so re-proposing one
  requires new evidence rather than a fresh opinion.

## Sequencing

Implementation begins only after Andrew accepts this record, and follows the
two-clocks rule: accepted syntax parses, unimplemented members yield
stage-named unsupported diagnostics with zero parser errors.

Suggested slices, cheapest and highest-value first:

1. **Rename and widen.** `has` → `containsKey`; `contains` onto `Deque`,
   `PriorityQueue`, `T[]`. Gate changes only, no new runtime.
2. **Diagnostics.** The E0521 fixit table, the property-as-method diagnostic,
   the parameterized equatable message. This slice is what makes the surface
   discoverable and should not trail the surface itself.
3. **`List::indexOf` / `List::remove`, `containsValue`, set `first`/`last`.**
   New MIR op variant and new runtime export for `indexOf`; the rest reuse
   existing paths.
4. **`clear()`** across all seven — the only slice with per-backend release-loop
   work.

Each slice needs `semantics.rs`, MIR lowering, all four backends
(`codegen_llvm.rs`, `codegen_cranelift.rs`, `codegen_native.rs`,
`codegen_php.rs` subject to §7's refusal note), `mir_interpreter.rs`, fixtures
including durable native parity entries, `docs/stdlib-reference.md`, plan §9.1,
and the language-server sweep in `dorialang/doria-language-server`.

### Implementation status

- **Slice 1 — Complete.** `has` was removed, `containsKey` is executable on both
  map families, and element `contains` is executable on every receiver settled
  by §1.
- **Slice 2 — Complete.** E0521 now uses one compiler-owned, receiver-aware
  suggestion table with structured source edits. Property calls have dedicated
  E0557 diagnostics, equality requirements name the actual receiver operation,
  and withdrawn `List::from` / `Dictionary::from` calls teach bracket literals.
  This slice introduced E0559 as the pre-MIR boundary for accepted later work.
- **Slice 3 — Complete.** `List::indexOf` returns the first equal position as
  `?int`; writable `List::remove` removes the first equal element;
  `Dictionary` and `SortedDictionary` execute O(n) value membership; and `Set`
  and `SortedSet` expose borrowed O(1) `first` / `last` properties. Shared MIR,
  its validator, the interpreter, Cranelift, LLVM, and the ordered-family PHP
  helpers implement the same contracts. E0559 no longer applies to these
  members.
- **Slice 4 — Complete.** Writable `clear(): void` executes on all seven named
  collections through shared validated MIR. It releases owned keys and values
  through the same type-aware, backend-independent order as final collection
  destruction, resets the existing allocation in place, invalidates membership
  indexes, and leaves the collection reusable. No Decision 0113 member remains
  routed to E0559.
- **Decision 0113 — Complete.** All four implementation slices are complete.
- **Stage 27 — In Progress.** Slice 1 is complete, Slice 2 is next, and pending
  controlled performance measurement is not a dependency.

### Slice 2 performance impact

| Dimension | Impact |
| --- | --- |
| Generated runtime | No change |
| Runtime representation or ABI | No change |
| Generated allocations, copies, or dispatch | No change |
| Backend lowering | No change |
| Compiler work | One bounded receiver-aware table lookup after failed collection-member resolution |
| Successful compilation fast path | No meaningful change expected |
| Benchmark | Not required |

The suggestion data is compiler-owned so a future `doriac migrate php` can
reuse it directly. The language server consumes structured compiler fixes and
does not maintain a second spelling table.

### Slice 3 performance impact

| Operation | Required complexity | Allocation |
| --- | ---: | ---: |
| `List::indexOf` | O(n) | None |
| `List::remove` | O(n), including one tail shift | None |
| `Dictionary::containsValue` | O(n) | None |
| `SortedDictionary::containsValue` | O(n) | None |
| `Set::first` / `last` | O(1) | None |
| `SortedSet::first` / `last` | O(1) | None |

Each search performs one scan and evaluates its probe once. `List::remove`
finds and removes in one runtime operation rather than searching twice. Set
endpoints use known storage positions and neither scan nor sort. Slice 3 does
not change the collection representation. Its private runtime ABI adds the
position-returning `indexOf` operation and advances membership/removal exports
to carry nullable-value presence explicitly; the compiler and runtime ship that
private ABI together, and no legacy entry points are retained.

### Slice 4 performance impact

| Case | Required complexity | Allocation |
| --- | ---: | ---: |
| Scalar-only collection | O(1) where the drop loop is elided | None |
| Owned-element collection | O(n), one release pass | None |
| Dictionary with owned keys or values | O(n), one release pass | None |
| Membership-index invalidation | O(1) reset/deallocation | No replacement collection |
| Refill within retained primary capacity | Existing insertion cost | No primary-storage growth |

Capacity remains unspecified publicly. The current implementation retains the
primary buffers, resets length and deque head, and discards the auxiliary
membership index. Compiler-emitted drop glue owns value cleanup; the narrow
runtime reset operation never interprets type IDs or releases Doria values.

## Affected components

Semantic analysis (member resolution and the membership gates), MIR and its
`CollectionMembershipOp`, shared MIR validation, all four backends, the MIR
interpreter, `doria-rt` collection exports (`indexOf` position export; `clear`
if any part moves into the runtime), the diagnostic catalogue and the shared
PHP→Doria spelling table, `docs/stdlib-reference.md`, plan §9.1 and §4.9,
durable parity fixtures, examples, and `dorialang/doria-language-server`.

## Invalidated elsewhere

- **Decision 0100** — its membership naming is superseded: `Dictionary::has` /
  `SortedDictionary::has` become `containsKey`, and its "Uniform `has` / uniform
  `contains`" alternative is reversed. Its deferral of `isSubsetOf` /
  `isSupersetOf` is converted from "deferred" to settled exclusion. Its
  Construction paragraph's claim that `::from` is "also available" for
  `List`/`Dictionary`/`T[]` is **withdrawn** — delete the clause; `::from` stays
  required for the five types with no literal form. Everything else in 0100
  stands.
- **Plan §9.1** — the predicate bullet's `hasKey` exemplar is replaced by
  `containsKey`, and the "one name per concept" bullet gains the axis-suffix
  clause from §1. §9.1's closing paragraph restates 0100's member lists inline
  and must be updated with the added members.
- **Plan §4.9** — the typed-array surface gains `contains`.
- **`docs/stdlib-reference.md`** — the `Dictionary`/`SortedDictionary` entry says
  "`has` (key membership)"; the `List`, `Set`/`SortedSet`, `PriorityQueue`, and
  `Deque` entries all need the added members.
- **`docs/notes/current-pipeline.md`** — its Stage 23 Slice 1 paragraph
  describes "Decision 0100's default member surface" as complete; that surface
  is now 0113's.
- **Any test, fixture, example, or diagnostic snapshot naming `has`** — including
  the `native_parity_examples.txt` manifest entries that exercise it.
- **`dorialang/doria-language-server`** — hover text, completion, and
  no-false-diagnostics coverage for every renamed and added member, plus the
  `doriac` pin bump. Per AGENTS.md this is same-beat work, not a follow-up.
- **`doria-website`** — the versioned guide and API reference restate the
  collection surface. Reported here; not scheduled from this record.
- **Any claim that a Doria map has no key-membership predicate** — it has one
  today under the name `has`, and will have one under the name `containsKey`.
- **Descriptions of `Dictionary` and `Set` as "hash" collections** — these were
  false when this record was drafted and are true as of `a78dc3d`, which gave
  both a hash index. Plan §9 and `docs/stdlib-reference.md` may keep the wording.
  0100's `Hashable` requirement is now met by the runtime rather than unused.
  The audit's §5 carries the superseding note.
