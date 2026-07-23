# Decision 0096: Primitives and interface conformance

**Status:** Accepted (settles how primitives satisfy interfaces; largely forced by
the fat-pointer representation in 0082 and monomorphization in §4.5, and required
by decision 0092).

## Context

Decision 0092 requires `Hashable` keys for `Dictionary`/`Set` and `Comparable`
keys/elements for `SortedDictionary`, `SortedSet`, and `PriorityQueue`. The
overwhelmingly common keys are `int` and `string` — primitives. But nothing
specified whether a primitive can satisfy an interface, so 0092 shipped with an
**unstated dependency**: without an answer here, `Dictionary<string, V>` is not
expressible.

The surrounding facts constrain the answer more than they leave it open:

- Conformance is **nominal** via an `implements` clause on the declaration (§6.2),
  and SPEC requires an explicit `implements Displayable` declaration — while
  primitives instead convert "out of the box" through compiler-known behavior
  (D16). So "built-in behavior for primitives, nominal conformance for classes"
  is already the established pattern.
- **Interface values are fat pointers** — a data pointer plus a vtable pointer
  (record 0082). A primitive has no data pointer.
- **Generics monomorphize with no boxing** (§4.5, the Rust model).

## Decision

The question splits into three, and they do not share an answer.

### 1. Primitives conform to the core value interfaces (compiler-known)

The primitive types conform to the core value interfaces without a source
`implements` clause; the compiler knows the conformance:

- `int` and the fixed-width integer family, `bool`, `string`, ranges, and Copy
  enums conform to **`Equatable`**, **`Comparable`**, and **`Hashable`**.
- The **`float` family conforms to `Equatable` only** — not `Hashable`, and not
  total `Comparable`. `NaN` compares unequal to itself and signed zero breaks
  hash/order agreement, exactly the rule decision 0087 already applied to float
  fields in derived data classes. Float ordering remains available as ordinary
  `<`/`>` comparison; what it does not provide is the *total* order a sorted
  container or a hash key requires.

This is what satisfies generic constraints, and it costs nothing:
`<T implements Comparable>` with `T = int` monomorphizes to a direct comparison —
no vtable, no allocation.

### 2. Interface-typed slots stay class-only

A primitive may **not** inhabit an interface-typed slot — `Comparable $x = 5;`
and `List<Comparable>` are compile errors. An interface value is a fat pointer,
so holding a primitive there would require boxing it to obtain an address: a
second boxing mechanism beside `mixed`, and the classic autoboxing trap.

A primitive that must be held dynamically goes through **`mixed`** — the one
sanctioned box — and is recovered with `is` narrowing. The diagnostic for an
interface-typed slot points at a generic type parameter or `mixed`.

So the rule is: **primitives satisfy constraints, they do not inhabit interface
types.**

### 3. No retroactive user-interface conformance for primitives (v1.0)

A user-declared interface cannot be implemented for a primitive. Conformance is
nominal and written at the declaration site, and a built-in has none; Doria has
no external-impl mechanism (Rust's `impl Trait for Type`). Express it instead by
constraining generically over a core interface, or by wrapping the primitive in a
class.

External impls plus the orphan/coherence rules they require are out of scope for
v1.0 and reopen only with their own decision.

### Spelling

Compiler-known conformance is never written in source: `int implements Comparable`
is a compiler fact and a documentation statement, not a declaration a user writes
or reads. No `implements` clause is ever attached to a built-in type.

## Alternatives considered

- **Box primitives into interface slots (Java's `int`/`Integer`):** rejected — a
  second box beside `mixed`, allocation on hot paths, and the autoboxing
  performance trap Doria's Copy-type model exists to avoid.
- **Require an explicit `implements` on primitives:** not expressible — built-ins
  have no declaration site to carry the clause.
- **Allow external impls now (Rust-style):** rejected for v1.0 — it needs orphan
  and coherence rules, which is a large feature, not a small one.
- **Make `float` `Hashable` / totally `Comparable` via bit-equality:** rejected
  here for consistency with 0087, which already deferred exactly that opt-in.

## Consequences

- Decision 0092's collections are expressible with zero boxing:
  `Dictionary<string, V>`, `SortedDictionary<int, V>`, and `PriorityQueue<int>`
  all monomorphize to direct hashing/comparison.
- **`float` cannot key a `Dictionary` or `SortedDictionary`, nor be a `SortedSet`
  element or `PriorityQueue` element.** This is a real limitation users will meet;
  it is stated here rather than discovered. The workaround is an integer or string
  key, or a wrapper type — matching Rust, where `f64` is neither `Ord` nor `Hash`.
- Generic constraints are the mechanism for "accepts several types"; interface-
  typed parameters remain class-only.
- The `Displayable` precedent generalizes: built-in behavior for primitives,
  nominal conformance for classes.

## Affected components

Semantic analysis (constraint satisfaction for primitive type arguments;
rejecting primitives in interface-typed slots with a fixit), the core interface
definitions (`Equatable`/`Comparable`/`Hashable`), generics and monomorphization,
diagnostics, decision 0092 (its dependency becomes explicit), plan §4.5/§6.2/§9
core, and SPEC when generics and interface values are implemented.

## Invalidated elsewhere

- Decision 0092's unstated assumption that `Comparable`/`Hashable` keys are
  available for primitive key types — now explicit, **with `float` excluded**.
- Any assumption that a primitive can be assigned to an interface-typed variable
  or stored in a collection of an interface type.
