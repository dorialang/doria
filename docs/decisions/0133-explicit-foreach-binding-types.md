# Decision 0133: Explicit Foreach Binding Types

> **Stage 35 amendment:** Decision 0134 applies this explicit-type requirement
> unchanged to user-defined Iterable values; compiler inference remains recovery
> and fix data, never permission to omit the type.

- **Status:** Accepted
- **Accepted:** 2026-09-04
- **Implementation Status:** Implemented By The Post-Stage-34 Explicit Foreach Binding Types Corrective Beat
- **Amends:** Decisions 0034, 0092, 0100, 0113, and 0132

## Context

Decision 0132 added the missing zero-based index role for `List<T>` and `T[]`,
but retained an older allowance for inferred foreach binding types. That made a
foreach binding an exceptional implicit local declaration even though Doria
requires source-visible types for parameters, properties, and ordinary typed
locals, and makes local inference explicit through `let`.

Adding a sequence index expands what foreach exposes. It does not weaken the
language's declaration discipline.

## Decision

Every authored foreach binding has an explicit type:

```doria
foreach ($names as string $name) {
    echo $name;
}

foreach ($names as int $index => string $name) {
    echo "{$index}: {$name}\n";
}

foreach (0..<10 as int $index) {
    echo $index;
}
```

This rule applies uniformly to integer ranges, `List<T>`, `T[]`, Dictionary
families and projections, sets, sorted sets, and deques. It also applies to both
bindings in the two-binding form. `writable` remains an access modifier and does
not stand in for a type:

```doria
foreach ($items as int $index => writable Item $item) {
    $item->refresh();
}
```

Decision 0132's iterable-role matrix remains unchanged. A `List<T>` or `T[]`
first binding is canonical `int`; a Dictionary-family first binding is its key
type; value-only families still reject a first binding. Explicit types do not
introduce conversion, alter borrowing, or make sequence indexes writable.

## Checking And Diagnostics

The parser retains an optional authored type in `ForeachBinding` so incomplete
source can be represented and diagnosed precisely. Semantic analysis resolves
the iterable and may derive the required binding type only for recovery and a
local machine-applicable fix. It never treats that derived type as permission to
accept the source.

An omitted known type reports E0748, `Foreach Binding Type Is Required`, at the
binding name and offers insertion of the exact checked type. Each omitted
binding receives its own fix. An unknown or invalid iterable does not receive a
speculative type diagnostic, and a forbidden first binding retains the more
fundamental value-only-family diagnostic. Source with E0748 does not enter HIR
or MIR lowering.

An explicit but incompatible type continues to use the existing assignment or
sequence-index diagnostic. Writable-source, readonly-key, set-element, and
borrow rules remain unchanged.

## Tooling And Execution

Compiler semantic facts continue to carry resolved binding types for hover,
rename, and backend-neutral lowering. Tooling presents E0748 and its compiler-
owned fix; it does not reinterpret omission as valid inference. Once types are
authored, the interpreter, Cranelift, LLVM, and PHP compatibility backend consume
the same foreach plan established by Decision 0132. No MIR or runtime change is
required.

Official examples and generated guidance use explicit types in every foreach
binding. Historical records may retain superseded source when labelled, but
they must point forward to this decision before being used as current guidance.

## Non-Goals

This decision does not add destructuring, tuple bindings, a `let` foreach form,
three-binding iteration, inferred declaration syntax, user-defined iterables,
or new collection/index roles. Decision 0134 assigns public `Iterable<T>` and
`Iterator<T>` conformance to Stage 35 Slice 3.

## Invalidated Elsewhere

- Decision 0132's statement that value bindings may preserve an inferred type
  is superseded; resolved compiler facts remain, but authored types are required.
- Current docs, examples, fixtures, or tooling that present `as $value`,
  `as writable $value`, or `as $index => $value` as valid Doria are stale.
- Website and external UAT examples must use `as T $value` and
  `as int $index => T $value` after their compiler/tooling integration updates.
- Decision 0134 accepts Stage 35 authority; Slice 1 is the next implementation
  unit.
