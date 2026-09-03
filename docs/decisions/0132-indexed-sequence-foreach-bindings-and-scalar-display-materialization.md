# Decision 0132: Indexed Sequence Foreach Bindings And Scalar Display Materialization

- **Status:** Accepted
- **Accepted:** 2026-09-02
- **Implementation Status:** Implemented By The Post-Stage-34 Indexed Foreach And Scalar Display Corrective Beat
- **Amends:** Decisions 0009, 0045, 0079, 0089, 0092, 0100, 0104, 0113, and 0116

## Context

Doria's PHP-shaped two-binding `foreach` syntax previously treated every first
binding as a dictionary key. Semantic checking therefore accepted sequence and
value-only sources more broadly than native MIR could execute. In particular, a
property-rooted `List<T>` reached M1101 even though its public order already
defined a natural zero-based position. The language also documented canonical
scalar display but did not state plainly that a display expression materializes
an ordinary reusable `string`.

## Decision

The syntax remains `foreach ($iterable as $first => $value)`. The AST calls the
optional source position `first_binding`; it does not assign a semantic role.
Checked semantics classify the iterable once and record one compiler-owned
iteration plan consumed by HIR, MIR, backends, and language tooling.

The supported first-binding matrix is:

| Iterable | First binding | Type | Order |
| --- | --- | --- | --- |
| `List<T>` | zero-based sequence index | canonical `int` | sequence order |
| `T[]` | zero-based sequence index | canonical `int` | sequence order |
| `Dictionary<K, V>` | actual stored key | `K` | insertion order |
| `SortedDictionary<K, V>` | actual stored key | `K` | ascending-key order |

Integer ranges, `Set<T>`, `SortedSet<T>`, `Deque<T>`, and dictionary
`keys`/`values` projections remain value-only. `PriorityQueue<T>` remains
non-iterable, `Bytes` gains no `foreach`, and user-defined
`Iterable<T>`/`Iterator<T>` remains Stage 35 work. Stable order alone does not
create a public index contract for a set or deque.

## Sequence Index Rules

A sequence index starts at zero and advances exactly once for every yielded
element whose body begins. It is a readonly, Copy, loop-local canonical `int`
with no cleanup obligation. An authored type must resolve to that exact semantic
type; a fixed-width mismatch is rejected and no conversion is introduced.
`writable` on the first binding is rejected.

The iterable expression evaluates once. The existing iteration borrow is
acquired before the index is initialized; each iteration establishes the first
binding before the value binding. Normal completion and same-loop `continue`
advance exactly once. `break`, `return`, checked Error, ambient Error, and
AssertionError expose no later index and follow existing structured cleanup.
Fatal panic remains abort-only. Empty sequences execute no body and nested loops
have independent index state.

The index is the public sequence position, never a runtime storage slot, host
array key, hash key, allocation index, or backend cursor. Producing it allocates
nothing, copies no collection, and does not mutate the collection.

## Dictionary And Value Bindings

Dictionary first bindings remain actual keys. No ordinal replaces or accompanies
the key, and no three-binding form is introduced. Existing key equality,
hashing, ordering, readonly behavior, and value borrowing remain unchanged.

Value bindings preserve their inferred or explicit element type, readonly-by-
default behavior, nullable and generic substitutions, and existing writable
element access. Writable values still require a writable iterable path and the
current exclusive-borrow rules. Sequence indexes never become writable. Move
elements are borrowed rather than moved out by ordinary `foreach`.

A property-rooted sequence is borrowed and evaluated once. An internal writable
`List<T>` may therefore be iterated through readonly `$this` without moving,
replacing, copying, or repeatedly loading the property. Ownership remains with
the containing object.

## Representation And Validation

`ForeachBinding` preserves modifier, authored-type, name, and whole-binding
spans. Semantic facts record the iterable type and family, one of `ValueOnly`,
`SequenceIndex`, or `DictionaryKey`, first/value types and spans, value access,
order, source, and package identity. These facts participate in incremental
fingerprints whenever the iterable family or binding contract changes.

HIR preserves the typed role and uses Doria-facing names. Shared MIR records a
validated `ForeachPlan`: sequence loops use one zero-initialized integer ordinal
and positional element access; dictionary loops retrieve the actual key while
values continue to use positional traversal. MIR validation rejects mismatched
families, writable or non-`int` indexes, malformed initialization/advancement,
and keyed-versus-positional access confusion. It is not weakened to admit an
unproven backend plan.

The interpreter, Cranelift, and LLVM consume that shared MIR. PHP emits a
collision-safe compiler-owned sequence ordinal and does not define Doria indexes
from PHP host keys. Dictionary PHP lowering keeps actual keys. No backend
reclassifies a loop from source spelling.

## Diagnostics

Unsupported first bindings fail during semantic analysis, before executable MIR
lowering. The diagnostic names the actual value-only family and preserves the
valid value binding. A machine-applicable edit removes the first binding and
`=>` only when it crosses no comment. Wrong sequence-index types may be replaced
with `int`; `writable` may be removed from a sequence index. PriorityQueue and
Bytes retain their non-iterable diagnostics without a second cascade.

M1101 remains a broad internal unsupported-lowering boundary for unrelated
routes. It is no longer reachable from valid indexed `List<T>` or `T[]` source
and is not replaced by a backend-specific rejection.

## Scalar Display Materialization

Interpolation, string-anchored concatenation, and `%s` use Doria's existing
canonical display conversion and produce an ordinary `string`. That result may
be assigned, returned, passed to a `string` parameter, stored in `List<string>`
or `string[]`, nested in another interpolation, and echoed later. `%d` and `%f`
remain deliberate formatting operations.

Direct scalar-to-string assignment and ordinary argument conversion remain
invalid. Doria adds no `Int::toString`, `Float::toString`, primitive instance
`toString`, `String::from` scalar overload, scalar cast, or implicit coercion.
Decision 0104's no-primitive-companion-`toString` rule remains authoritative.

Canonical display remains locale-independent base-10 for signed and unsigned
integers, `true`/`false` for booleans, and deterministic shortest-round-trip text
for binary32 and binary64. Float special spellings remain `NaN`, `Infinity`,
`-Infinity`, `0`, and `-0`. Host-language formatting is not source semantics.

## Performance And Security

Indexed sequence iteration adds one scalar local or induction value. It performs
no collection conversion, reindexing copy, dynamic key lookup, heap iterator,
per-element wrapper allocation, or loop-body stack growth. LLVM stack slots, if
needed, belong in the function entry block. Collection and object representation
remain private; metadata schemas 1, 2, and 3 and processor protocol version 1
remain exact. No runtime reflection or unchecked cast is introduced.

## Non-Goals

This decision does not add indexes for ranges, sets, deques, or projections;
PriorityQueue/Bytes iteration; generalized `enumerate` syntax; public iterator
objects; user-defined iterable execution; collection covariance; scalar casts or
conversion APIs; interfaces; traits; Stage 35 execution; or property hooks.
Decisions 0130 and 0131 remain unchanged.

## Invalidated Elsewhere

- Any AST, HIR, ownership, narrowing, or tooling path that names every first
  binding a key is stale; syntax is neutral and the compiler supplies the role.
- Any semantic path that accepts a first binding for a value-only family is
  incomplete; the source diagnostic must precede HIR/MIR execution lowering.
- Any MIR/backend path that treats a sequence first binding as a dictionary key
  is invalid; sequence access is positional and the index is compiler-owned.
- Website examples may teach indexed `List<T>` and `T[]` iteration and reusable
  display strings after their compiler/toolchain pin is refreshed. This
  repository does not coordinate that website update.
- Stage 35 remains the next numbered language stage. The next record number
  remains unallocated, and property hooks remain scheduled and unimplemented.
