# Collection method-surface audit

> Documentation role: non-authoritative review context for the collection
> method surface. It records what the compiler accepts **today** (measured, not
> inferred), what the runtime can already do, and where the surface forces a
> clumsy or semantically wrong workaround. It exists to feed one decision rather
> than another single-method patch. Decision 0113 is the authority that follows
> from it; this note is evidence, not law.

Measured on branch `develop` at `bd5a641` with a debug `doriac` built from that
tree. Every "present" and "absent" cell below was probed with a one-member
program and `doriac check`; none of the matrix is read off documentation.

Beyond checking, every one of the 51 present members was also **executed**, so
the matrix distinguishes "the checker accepts it" from "it works". All 51 run
correctly, and the MIR interpreter and Cranelift agree byte for byte — there is
**no two-clocks violation** in the current surface. LLVM is not enabled in this
build, so the release profile is unverified rather than confirmed.

## Implementation follow-up

Decision 0113 and all four slices are complete on current `develop`. Slice 3 makes
`List::indexOf`, writable `List::remove`, both map `containsValue` operations,
and readonly `Set` / `SortedSet` `first` and `last` properties executable
through the MIR interpreter, Cranelift, and LLVM. Slice 4 makes writable
`clear(): void` executable in place on every named collection with exact-once
cleanup, state reset, and reuse; no Decision 0113 member remains behind E0559. The measured
matrix below remains the historical input to the decision rather than being
rewritten as if those operations existed at audit time.

## 1. Correction to the reported symptom

The audit was prompted by the report that `Dictionary` and `SortedDictionary`
have **no key-membership predicate**, leaving `$dict->get($key) != null` as the
only option. That specific claim is **wrong**, and the correction matters
because it changes what needs building.

`Dictionary::has(K): bool` and `SortedDictionary::has(K): bool` exist, are
settled in Decision 0100, are documented in `docs/stdlib-reference.md`, and
execute end to end:

```doria
Dictionary<string, ?int> $d = ["present" => null, "other" => 7];

$d->get("present") == null   // true  — conflates absent with present-but-null
$d->get("missing") == null   // true  — same answer, different fact
$d->has("present")           // true  — correct
$d->has("missing")           // false — correct
```

That program compiles and runs natively today and prints exactly the four
answers above. So the nullable-value conflation the report describes is real,
but the language already has the correct operation for it.

**The actual defect is that `has` is undiscoverable.** Every other membership
predicate in the family is spelled `contains`. A user who knows
`List::contains` and `Set::contains` will reach for `containsKey` on a map —
which is precisely what happened — and the compiler answers:

```text
Error[E0521]: Collection Method `containsKey` Is Not Part of the Stage 23
Surface Settled by Decision 0100
```

with **zero fixes, zero help text, and zero notes** (verified against the JSON
diagnostic payload). The diagnostic names the record that forbids the spelling
without ever naming the spelling that works. A user reasonably concludes the
capability is missing and writes the broken `get() != null` workaround.

This reframes the work. The membership gap is mostly a **naming and
diagnostics** problem, not a missing-capability problem — and the naming problem
has a documented root cause, below.

## 2. Root cause: the naming charter contradicts itself

Plan §9.1 is the naming law every stdlib record cites. Two of its bullets give
incompatible answers for this exact case:

| §9.1 bullet          | What it says                                                                                     | Implied spelling           |
|----------------------|--------------------------------------------------------------------------------------------------|----------------------------|
| Predicates           | "read as questions: `is`/`has`/`can` prefixes (`isEmpty`, **`hasKey`** as members)"              | `hasKey`                   |
| One name per concept | "it is `count` everywhere (never `size` or `length` for collections), **`contains` everywhere**" | `contains` / `containsKey` |

Decision 0100 then chose a **third** spelling, and explicitly rejected the
charter's own uniformity rule to do it:

> **Uniform `has` / uniform `contains`.** Rejected — `contains(value)` for the
> sequence/set membership and `has(key)` for the map read differently on
> purpose.

So three authoritative documents specify three different names — `hasKey`,
`contains`, and `has` — for one concept, and the implementation follows the one
that appears in neither charter bullet. Successive passes "keep failing to cover
the necessary methods" partly because there is no single rule to check coverage
against. Any decision that fixes membership must fix §9.1 in the same beat, or
the contradiction regenerates the problem.

## 3. The matrix

Measured per cell. `T[]` and `Bytes` are included as context because they share
the E0521 gate, though their surfaces are owned by §4.9 and the future Bytes
record rather than by 0100.

Legend:

| Mark  | Meaning                                                                              |
|-------|--------------------------------------------------------------------------------------|
| `yes` | Present today: accepted by the checker and executed on the interpreter and Cranelift |
| `rt`  | **Absent, but the runtime already implements it** — no new `doria-rt` code needed    |
| `new` | Absent, and needs new runtime work                                                   |
| `n/a` | Not meaningful for this type, or deliberately excluded (see §4)                      |
| `S30` | Named in 0100, scheduled with closures at Stage 30                                   |

### Properties

| Member      | List | Dict | Set | SDict | SSet | PQ  | Deque | T[] |
|-------------|------|------|-----|-------|------|-----|-------|-----|
| `count`     | yes  | yes  | yes | yes   | yes  | yes | yes   | n/a |
| `length`    | n/a  | n/a  | n/a | n/a   | n/a  | n/a | n/a   | yes |
| `isEmpty`   | yes  | yes  | yes | yes   | yes  | yes | yes   | rt  |
| `first`     | yes  | n/a  | rt  | n/a   | rt   | n/a | n/a   | rt  |
| `last`      | yes  | n/a  | rt  | n/a   | rt   | n/a | n/a   | rt  |
| `peek`      | n/a  | n/a  | n/a | n/a   | n/a  | yes | n/a   | n/a |
| `peekFront` | n/a  | n/a  | n/a | n/a   | n/a  | n/a | yes   | n/a |
| `peekBack`  | n/a  | n/a  | n/a | n/a   | n/a  | n/a | yes   | n/a |
| `keys`      | n/a  | yes  | n/a | yes   | n/a  | n/a | n/a   | n/a |
| `values`    | n/a  | yes  | n/a | yes   | n/a  | n/a | n/a   | n/a |

`keys` / `values` are `foreach`-only projections; using one as a value is
E0522, which is a good, specific diagnostic.

### Methods

| Member                  | List | Dict       | Set | SDict      | SSet | PQ  | Deque | T[] |
|-------------------------|------|------------|-----|------------|------|-----|-------|-----|
| `contains(T)`           | yes  | n/a        | yes | n/a        | yes  | rt  | rt    | rt  |
| `has(K)`                | n/a  | yes        | n/a | yes        | n/a  | n/a | n/a   | n/a |
| `containsKey(K)`        | n/a  | *(=`has`)* | n/a | *(=`has`)* | n/a  | n/a | n/a   | n/a |
| `containsValue(V)`      | n/a  | rt         | n/a | rt         | n/a  | n/a | n/a   | n/a |
| `get(K)`                | n/a  | yes        | n/a | yes        | n/a  | n/a | n/a   | n/a |
| `set(K, V)`             | n/a  | yes        | n/a | yes        | n/a  | n/a | n/a   | n/a |
| `add(T)`                | yes  | n/a        | yes | n/a        | yes  | n/a | n/a   | n/a |
| `insertAt(i, T)`        | yes  | n/a        | n/a | n/a        | n/a  | n/a | n/a   | n/a |
| `remove(K or T)`        | rt   | yes        | yes | yes        | yes  | n/a | new   | n/a |
| `removeAt(i)`           | yes  | n/a        | n/a | n/a        | n/a  | n/a | n/a   | n/a |
| `indexOf(T)`            | new  | n/a        | n/a | n/a        | n/a  | n/a | new   | new |
| `pop()`                 | yes  | n/a        | n/a | n/a        | n/a  | yes | n/a   | n/a |
| `push(T)`               | n/a  | n/a        | n/a | n/a        | n/a  | yes | n/a   | n/a |
| `pushFront(T)`          | n/a  | n/a        | n/a | n/a        | n/a  | n/a | yes   | n/a |
| `pushBack(T)`           | n/a  | n/a        | n/a | n/a        | n/a  | n/a | yes   | n/a |
| `popFront()`            | n/a  | n/a        | n/a | n/a        | n/a  | n/a | yes   | n/a |
| `popBack()`             | n/a  | n/a        | n/a | n/a        | n/a  | n/a | yes   | n/a |
| `union(S)`              | n/a  | n/a        | yes | n/a        | yes  | n/a | n/a   | n/a |
| `intersect(S)`          | n/a  | n/a        | yes | n/a        | yes  | n/a | n/a   | n/a |
| `difference(S)`         | n/a  | n/a        | yes | n/a        | yes  | n/a | n/a   | n/a |
| `isSubsetOf(S)`         | n/a  | n/a        | new | n/a        | new  | n/a | n/a   | n/a |
| `isSupersetOf(S)`       | n/a  | n/a        | new | n/a        | new  | n/a | n/a   | n/a |
| `clear()`               | new  | new        | new | new        | new  | new | new   | n/a |
| `Type::from(src)`       | n/a  | n/a        | yes | yes        | yes  | yes | yes   | n/a |
| `map`/`filter`/`reduce` | yes  | —          | —   | —          | —    | —   | —     | —   |

Decision 0121 supersedes the audit's former broad Stage 30 row. Stage 30g
implements these higher-order algorithms only on `List<T>`; other collection
families need their own result-shape, order, ownership, and entry-model
authority.

Everything not listed falls to the E0521 catch-all.

### Why the `rt` cells are genuinely free

These were checked against `crates/doria-rt/src/collection.rs`, not assumed:

- **`contains` on `Deque`, `PriorityQueue`, `T[]`.** `collection::contains`
  (line 1061) scans `0..length` through `read_value`, and `value_address`
  (line 59) already applies the `Deque` ring-buffer head translation. Backend
  dispatch already selects `COLLECTION_CONTAINS` for any collection whose
  `definition.key` is `None`. Widening the `matches!` gates in `semantics.rs`
  and `mir_lowering.rs` is the entire change — no runtime, no ABI, no new MIR op.
- **`containsValue` on the map types.** The same `collection::contains` scans
  the *values* array, which for a keyed collection is exactly value membership.
  Backend dispatch currently picks `COLLECTION_KEYED_HAS` whenever a key type
  exists, so this one needs a new `CollectionMembershipOp` variant to force the
  value path — small backend work, but zero runtime work.
- **`List::remove(T)`.** `collection::remove_value` (line 1095) already exists
  and already backs `Set::remove`.
- **`first` / `last` on the set types.** `NullableCollectionAccess::First` /
  `Last` already exist in MIR and already back `List::first` / `List::last` and
  the `Deque` peeks.
- **`List::from` / `Dictionary::from`.** `collection::from_copy` (line 625)
  already backs the five `::from` constructors that work.

### One trap for the implementer

`remove_at`, `insert_at`, and `remove_value` shift elements with a single linear
`ptr::copy` between two `value_address` results. That is correct only for a
**contiguous** collection. On a wrapped `Deque` the source and destination
regions are not adjacent and the copy would overrun. These three are currently
unreachable from `Deque`, and any decision that exposes `Deque::remove`,
`Deque::removeAt`, or `Deque::insertAt` must make the shift ring-aware first.
This is the reason `Deque | remove` is marked `new` and not `rt`.

## 4. Peer comparison and gap ranking

Compared against PHP arrays/SPL, Rust `std::collections`, and C#
`System.Collections.Generic`. Parity is explicitly **not** the goal; the ranking
criterion is the one that matters: *does the absence force a clumsy or
semantically wrong workaround?* Each workaround below was compiled and run, so
the "clumsy" claims are measured, not asserted.

### Rank 1 — forces a wrong or impossible result

| Gap                              | PHP                | Rust           | C#            | Why it ranks here                                                                                                                                                                                |
|----------------------------------|--------------------|----------------|---------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Map membership is undiscoverable | `array_key_exists` | `contains_key` | `ContainsKey` | The capability exists as `has`, but every peer spells it with the `contains` root, so users write `get() != null`, which is **wrong** for `?V` values                                            |
| `PriorityQueue` membership       | —                  | —              | —             | `PriorityQueue` has no `foreach`, no indexing, and no `contains`. There is **no non-destructive way** to ask whether an element is queued — the only answer is to drain the queue and rebuild it |

The `PriorityQueue` hole is the one place in the family where the workaround is
not merely clumsy but destroys the collection. No peer has this problem because
each exposes iteration over the heap (C# `UnorderedItems`, Rust `BinaryHeap::iter`).

### Rank 2 — forces a genuinely clumsy workaround

| Gap                           | PHP                      | Rust                  | C#                   | Workaround today                                                                                                                                                                                                                  |
|-------------------------------|--------------------------|-----------------------|----------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `List::remove(T)` / `indexOf` | `array_search` + `unset` | `position` + `remove` | `Remove` / `IndexOf` | Removing a known value from a `List` needs a hand-written index loop — measured at 8 lines for a one-line intent                                                                                                                  |
| `SortedSet` min / max         | —                        | `first` / `last`      | `Min` / `Max`        | Min is `foreach` + `break`; **max requires a full O(n) walk** of a structure that stores it at `length - 1`                                                                                                                       |
| `containsValue` on maps       | `in_array`               | `values().any(..)`    | `ContainsValue`      | `foreach ($d->values ...)` plus a `writable bool` flag and a `break` — 5 lines for a predicate                                                                                                                                    |
| `clear()`                     | `$a = []`                | `clear`               | `Clear`              | `$l = []` works for a **local**, but a collection reached through a property path or a writable shared access object has no in-place empty at all — and 0100 deliberately distinguishes replacing `$this->items` from mutating it |

### Rank 3 — absent, but the workaround is fine

| Gap                | Workaround                                | Verdict                                                                                 |
|--------------------|-------------------------------------------|-----------------------------------------------------------------------------------------|
| `getOrDefault`     | `$d->get($k) ?? $default`                 | `??` already covers it exactly. Adding a method would be a second spelling for one idea |
| `isSubsetOf` etc.  | `$a->difference($b)->isEmpty`             | Correct and readable; only costs an allocation                                          |
| `addAll` / `merge` | `foreach ($src as T $v) { $dst->add($v); }` | Three clear lines; Decision 0134 settles cloning capability but does not add a bulk member |
| `sort` / `reverse` | `SortedSet` / `SortedDictionary` exist    | Ordering is what the `Sorted` family is *for*; comparator sorting belongs with closures |

### Deliberate design, not oversight

These absences are correct and should be recorded as settled so they stop being
re-proposed:

- **No `size`, `Count`, or `length` on named collections.** `count` is the one
  name; `length` is reserved for fixed-extent buffers. §9.1 and 0100 agree.
- **No fluent mutators.** 0100 settled `void` returns; userland fluency is
  Decision 0088's job.
- **No bare PHP `array` surface, no `Queue`/`Stack` types.** Identity rules in
  AGENTS.md; `Deque` subsumes both.
- **No non-panicking `$l[i]`.** Two clear idioms (assertive index, `?T` method)
  beat one blurred one.
- **First `foreach` bindings are family-specific.** Decision 0132 gives
  `List<T>` and `T[]` a readonly zero-based `int` sequence index and preserves
  actual keys for Dictionary families. Ranges, sets, deques, and Dictionary
  projections remain value-only; stable order alone does not imply an index.
- **`keys`/`values` are not storable.** They remain compiler-known readonly,
  foreach-only projections. Decision 0134's public iteration protocol does not
  silently turn them into owned lists or nameable view types; a later additive
  iterator API would require its own authored surface.
- **`List`, `Dictionary`, and `T[]` are built by literal, not by `::from`.**
  These types exist to de-conflate PHP's `array`, and the typed bracket literal
  — including the empty `[]` — is that replacement. `::from` is the constructor
  for the five types a literal cannot build. 0100 claims `::from` is "also
  available" for the literal types; that claim is unimplemented, and for `T[]`
  it is not even expressible (`int[]::from([1, 2])` does not parse). It should
  be withdrawn from 0100 rather than built.
- **No `PriorityQueue` `foreach`.** Heap order is not a meaningful iteration
  order. Note this is *distinct* from `contains`, which needs no order at all —
  which is why the Rank 1 entry above stands despite this being deliberate.

## 5. Representation: the unordered collections are unindexed

This is a separate finding from the surface question, and a larger one. It was
raised during review and is reproduced here because it was measured, not
inferred.

> **Superseded 2026-08-05.** This section described the representation as it
> stood when the audit was written. It has since been fixed in `a78dc3d`,
> `34ef666`, and `4d6e37d`: `Dictionary` and `Set` now carry a hash index, built
> on the first membership query and maintained on append, and `dictionary_lookup`
> went from 111.5x the C peer to 1.90x with per-lookup cost flat in entry count.
> The finding below is retained because it is what the surface audit was reasoning
> against, and because the naming consequence it draws still stands.

**There was no hash table.** The string `hash` did not occur anywhere in
`crates/doria-rt/src/collection.rs`. `Dictionary` and `Set` were `KIND_LEGACY` —
flat, unindexed arrays. `find` and `contains` each binary-searched **only** when
the collection was a finalized `SortedDictionary` or `SortedSet`, and otherwise
scanned linearly. `push_unique` called `contains` before every insert, so
building a `Set` was quadratic.

The consequence inverts what the names promise: **the `Sorted` variants are
asymptotically faster to look up than the defaults.** Measured on this build,
Cranelift profile, best of several runs, with process-start baseline at 0 ms:

| Benchmark                              | Unordered | Sorted | Ratio |
|----------------------------------------|-----------|--------|-------|
| 200,000 `contains` into 4,000 elements | 7,850 ms  | 40 ms  | ~196× |
| 200,000 key lookups into 4,000 keys    | 5,800 ms  | 20 ms  | ~290× |

`Set` construction scaling, confirming the quadratic build — time roughly
quadruples as N doubles:

| N     | `Set` build |
|-------|-------------|
| 2,000 | 30 ms       |
| 4,000 | 140 ms      |
| 8,000 | 580 ms      |

Three things follow:

1. **The documentation is wrong, not just imprecise.** Plan §9 and
   `docs/stdlib-reference.md` both describe the bare name as "the default (hash
   / insertion-ordered) collection". Nothing hashes. Decision 0100 additionally
   requires `Hashable` keys and elements — a constraint the runtime never uses.
2. **The performance advice a user would infer is backwards.** Someone reaching
   for `Set` over `SortedSet` for fast membership gets a 196× slowdown.
3. **This is out of scope for a method-surface record and must not be smuggled
   into one.** Changing the representation touches the runtime layout, the ABI,
   iteration-order guarantees (0092 promises insertion order, which a naive hash
   table would not preserve), and every backend. It belongs with the Stage 26b
   performance work and needs its own decision.

The surface record that follows notes the complexity of each member honestly and
otherwise leaves this alone.

## 6. Secondary defects found while auditing

These are not method gaps, but they are part of why the surface feels
incomplete, and the decision should dispose of them:

1. **E0521 carries no fixit.** Verified empty `fixes`, `help`, and `notes`. The
   catch-all names the record that forbids a spelling and never names the
   spelling that works. AGENTS.md already requires PHP→Doria spelling
   suggestions to live in shared compiler data for reuse by
   `doriac migrate php`; the peer-spelling table for collections belongs there.
2. **`$d->isEmpty()` (call form) reports E0521**, not "`isEmpty` is a property".
   The user gets "not part of the Stage 23 surface" for a member that *is* part
   of the surface, in the wrong syntactic form.
3. **`List::from` / `Dictionary::from` fail with a misleading diagnostic.** 0100
   states `::from` "is also available" for the literal-constructible types; it is
   unimplemented, and for `T[]` unimplementable. The right fix is to withdraw the
   claim, not build it — but either way the current diagnostic is wrong:
   ``E0305: Unknown Class `List` `` tells the user the type does not exist. It
   should teach the bracket literal instead.
4. **The PHP backend refuses the entire Stage 23 collection surface.**
   `is_stage23_runtime_type` in `codegen_php.rs` causes a blanket B2301 for
   *every* method call, property access, and indexed read on `List`,
   `Dictionary`, `Set`, `T[]`, and `Bytes`. The Stage 26 ordered types
   (`SortedDictionary`, `SortedSet`, `PriorityQueue`, `Deque`) are fully
   implemented there as hand-written PHP classes. The parity gap is inverted:
   the *default* collections are the unsupported ones. Any implementation plan
   must state that new members inherit this refusal rather than appearing to
   regress the backend.
5. **`check_stage23_equatable_type` hardcodes "List::contains"** in its message
   text, so widening `contains` to other types will emit a wrong type name until
   the message is parameterized.

## 7. What this feeds

Decision 0113 settles the complete surface in one pass, including the §9.1
amendment that removes the three-way naming contradiction. The representation
finding in §5 is deliberately **not** settled there; it needs its own record
alongside the Stage 26b performance work. Nothing in this note is authoritative
on its own.
