# Decision 0083: Stage 19 Ownership Mechanics

Status: Accepted

## Context

Decisions 0081 and 0082 settle destruction order and native representation, but Stage 19 also needs precise rules for parameter transfer, promoted properties, readonly bindings, assignment, and safe construction before Stage 21's full definite-initialization analysis exists.

These rules must prevent duplicate ownership and uninitialized native storage while staying within Stage 19's deliberately small ownership surface. Method/interface-based cloning does not exist yet, direct nested-property transfers are not specified, and panic remains abort-only.

## Decision

### `take` and `writable` are exclusive

`take` transfers ownership into a parameter. `writable` grants an exclusive mutable borrow. They answer different questions, and once ownership is taken exclusivity of a borrow is moot, so a parameter cannot use both. The compiler rejects both `take writable` and `writable take`.

### Promoted move values require transfer

A promoted constructor parameter whose type is a move type must be declared with `take`, for example:

```doria
class Team {
    function __construct(take Person $manager) {}
}
```

Promotion stores the parameter directly in a new owning property. Omitting `take` would leave both the caller and the property as owners, so it is an error with a machine-applicable fix that inserts `take`. Promoted Copy-type parameters are unchanged. This extends the promotion grammar settled in Section 8 without introducing a call-site marker.

### Total property order

A class's total property order is all explicit properties in class-body order, followed by promoted properties in constructor-parameter order. Construction follows that order; destruction reverses it under decision 0081.

This total order is required because "reverse declaration order" is otherwise undefined for a class containing both explicit and promoted properties.

### Moving readonly bindings

A readonly binding may be moved from. A move ends the binding's ownership; it does not mutate the value or reassign the binding, so readonly does not prohibit it.

Reinitializing a moved-from binding is a new assignment and therefore a mutation. It requires `writable`, just like any other reassignment.

### Invalid and deferred moves

Self-moves such as `$value = $value` and overlapping source/destination moves are rejected. Direct moves into or out of nested owned properties remain unsupported until their interaction with writable paths and aliasing is explicitly specified. Stage 19 diagnoses these forms rather than improvising semantics.

### Temporary native-eligibility soundness gate

**This gate is temporary and is lifted when Stage 21 definite initialization lands.**

Stage 19 may natively construct only a class for which every property is provably initialized by a property initializer, promotion, or the existing narrow direct constructor-initialization form. If initialization cannot be proven, compilation emits a clear "unsupported until Stage 21" diagnostic.

This is a soundness gate, not a permanent language limitation. Stage 19 emits native code and must never return a class pointer containing uninitialized property storage. Uninitialized memory is never the fallback.

### Allocation failure

Class allocation failure takes the status-101 panic path with the exact message `class allocation failed`. Allocation failure is OOM: no class allocation exists to clean up, making this the case where abort-only, no-cleanup behavior is unambiguously correct.

## Alternatives considered

### Implicitly take promoted move parameters

Rejected. Ownership transfer must be visible at the parameter declaration, and silently changing a parameter from borrowed to owned would make ordinary and promoted parameters inconsistent.

### Forbid moving readonly bindings

Rejected. That would conflate mutation with ownership transfer and make readonly bindings accidental permanent owners.

### Allocate first and trust constructors

Rejected. Until Stage 21 proves full constructor initialization, accepting an unproven class could expose uninitialized native memory. The temporary gate preserves soundness without pulling Stage 21 forward.

### Reference-count classes

Rejected by decision 0082. It would avoid some move errors by changing the language's class model rather than implementing the approved unique-ownership model.

## Consequences

The parser accepts `take` only in parameter position. Semantic analysis classifies Copy and move types, applies the mutual-exclusion and promotion rules, and performs path-sensitive ownership checking. Typed MIR carries ownership transfer and explicit cleanup so every backend observes the same behavior.

Use-after-move diagnostics identify the give-away point and explain that the value cannot be used afterward. They must not suggest `->clone()`: cloning is unavailable until the later method/interface surface exists.

Decision 0081 remains authoritative for panic: panic performs no cleanup. Stage 19 does not restore terminal raw mode or any other RAII-managed state on the panic path; a future panic hook is the only contemplated exception.

The Stage 19 acceptance criterion remains exactly: `AC: destructor-order example; use-after-move diagnostic snapshots; RAII resource-guard example; leak CI clean.` Explanatory resource-guard and diagnostic details belong in roadmap prose, not inside that criterion.

The temporary native-eligibility gate must be removed, rather than normalized into permanent behavior, when Stage 21 definite initialization is implemented.
