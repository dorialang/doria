# Decision 0144: Never-Null Coalesce And Sequence `get`

- **Status:** Accepted
- **Date:** 2026-11-13
- **Owners:** Doria language and compiler design
- **Scope:** What `??` accepts as its left operand, and the safe indexed read of `List<T>` and `T[]`

## Context

Decision 0100 fixes the missing-element contract: `$l[i]` and `$d[k]` assert that the element is present and panic when it is not, and the nullable members are the safe path. Decision 0093 defines `??` as using its right operand only when the left operand is null.

Two gaps followed from those rules:

- The checker accepted `??` after any left operand, including one that can never be null. `$lines[$row] ?? ""` passed `check`, then failed native code generation with the internal error I2201, because lowering has no path for a fallback that cannot run. An editor showed no error while a build did.
- Decision 0100 names `get` as the safe read, but only the dictionaries had it. `List<T>` and `T[]` had no way to read an element by index without asserting it is there.

Andrew ruled both on 2026-11-13.

## Decision

### `??` requires a left operand that can be null

A `??` whose left operand can never be null is a compile-time error, E0782, "Null-Coalescing Operand Is Never Null".

- A binding counts by its declared type. A `?T` or `mixed` binding that flow analysis has narrowed to `T` may still be coalesced; the check is redundant and valid, as Decision 0093's narrowing already allows for `== null`.
- Any other operand counts by the type of the expression. An index read, a call, or a property whose type is a non-nullable `T` is never null. An index read of a `List<?T>` has type `?T` and may be coalesced.
- `null ?? $fallback` stays valid. A bare `mixed` operand keeps its existing mixed-operation error.

The error names the fix. After an index read it says to read the element with `get`; otherwise it says to remove `??` and its fallback or make the operand nullable.

```doria
let $line = $lines[$row] ?? "";        // E0782
let $line = $lines->get($row) ?? "";   // the element, or "" when $row is out of range
```

The index read itself is unchanged: `$lines[$row]` still panics when the element is missing.

### `get(int $index): ?T` on `List<T>` and `T[]`

```doria
$list->get($index)    // ?T
$array->get($index)   // ?T
```

`get` returns the element at `$index`, or `null` when `$index` is negative or not less than the element count. It never panics. It is a readonly borrow of the element, with the same borrow rules as the dictionaries' `get`. When the element type is already nullable the result keeps one nullable layer, so a `null` result does not distinguish a missing element from a stored `null`.

## Implementation Contract

- `get` lowers to the existing optional read at an offset, the access `foreach` already uses for positional element reads. The interpreter, Cranelift, LLVM, and the runtime share that one operation; no runtime entry point is added. PHP output uses the optional array read the dictionaries use.
- The interpreter's optional read treats a negative offset as absent, as the native runtime does.
- MIR lowering accepts a `??` whose left operand has a non-nullable type equal to the result type and lowers it as the left operand alone. This covers the narrowed-binding case; it is not a second way to accept what E0782 rejects.

## Explicit Non-Goals

- A non-panicking index read. Decision 0100 rejected `$l[i]` returning `?T`, and that stands.
- `get` on `Bytes` and `ByteRegion`. Their `[]` belongs to the byte surface, which has its own member table and storage; a safe byte read is a separate decision.
- `get` on `Deque<T>`, `Set<T>`, `SortedSet<T>`, and `PriorityQueue<T>`, which have no index read.
- A lint level. E0782 is an error.

## Consequences

- Programs that coalesced a never-null value stop compiling and are told why. Inside this repository none did.
- `docs/stdlib-reference.md` lists `get` for `List<T>` and `T[]`; Decision 0100's `List<T>` table gains the row.

## Invalidated Elsewhere

- The language server must move its compiler pin, list `get` in its `List<T>` and `T[]` member data, and show no error for `$list->get($index) ?? $fallback`.
- The website API pages for `List<T>` and typed arrays need `get`, and the nullable-types guide should state that `??` requires a left operand that can be null. This task does not modify the website repository.
