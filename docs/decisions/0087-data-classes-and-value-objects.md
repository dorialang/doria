# Decision 0087: Data classes, value objects, and the DTO boundary

**Status:** Accepted (design direction; implementation deferred — see Sequencing)

## Context

Data-transfer objects, value objects, records, and "data classes" recur as
requests. They read as one feature but decompose into distinct needs: identity
vs value semantics, structural equality, immutability, copy-with-changes,
destructuring, and boundary serialization. Treating them as a single new type is
the trap this record closes.

Doria has already chosen its data type. The Copy-vs-move / ownership decision
records that **classes are the owned record type** and that there is **no
user-defined `struct` or value type in v1.0** (inline-layout value aggregates are
a post-1.0 revisit; the only compiler-known Copy aggregates are the `Doria\Std\Math`
value types). Classes are move types; duplication is explicit through the future
`Cloneable`/`->clone()` surface, never implicit. An immutable (all-`readonly`)
class is therefore already the idiom for a value object.

What is missing is not a type. It is **compiler-derived structural behavior** on
the class: structural equality, hashing, structural display, copy-with-changes,
and destructuring. Because Doria has no reflection and keeps a headless data
representation (record 0082), that behavior must be generated statically, which
is exactly what the attributes & compile-time codegen decision exists to provide.

## Decision

### The class is Doria's record type — no new kind

No `record`, `struct`, or `data`-type is introduced as a distinct kind. The class
remains the single owned aggregate. Data classes, value objects, and DTOs are all
*uses* of the class, not new declarations. This preserves the Copy-vs-move
decision (classes stay move types) and adds nothing to the type taxonomy.

### Structural behavior is an opt-in derive

Class equality stays **identity/opt-in by default**. Structural behavior is
requested explicitly — the Rust `#[derive(...)]` model, not Kotlin's automatic
`data class` equality — so structural semantics are always greppable and never
silent. The derivable behaviors are:

- **Equatable** — structural `==`/`!=` comparing fields.
- **Hashable** — a canonical hash making the type usable as a `Dictionary`/`Set`
  key (composite keys).
- **Displayable** — a structural `toString`.
- **Copyable / copy-with-changes** — a shallow copy overriding named fields,
  built on the `Cloneable` surface from the interfaces decision.
- **Destructurable** — binding a value's fields in `let`/`match`, extending the
  enum-payload destructuring machinery from the enums and match decisions.

These are delivered by the attributes & compile-time codegen decision's
derive-style codegen: static, no runtime metadata, DCE-safe, and preserving the
headless representation (record 0082). A **`data class` spelling is optional
sugar** over the common bundle (Equatable + Hashable-where-eligible +
Displayable + Copyable); it expands to derives and introduces no new type.

### Value objects

A value object is one of two existing things, not a third:

- **Single-field wrapper** (`Money`, `EmailAddress`, the DDO decision's `Sql`
  provenance type): the **newtype** work.
- **Multi-field immutable aggregate** (`Point`, `DateRange`): an immutable
  (`readonly`) class with the derives above.

### The DTO boundary belongs to frameworks

A DTO is a data class plus **serialization** (JSON in/out) at a boundary.
Serialization and transport are framework/library concerns, expressed through the
attributes & compile-time codegen decision — which already names JSON
serialization as a sanctioned derive consumer. The language ships the data
primitive and the derive engine; it does not ship a DTO type, a serializer, or a
transport. **DAOs remain ordinary classes** (they are behavior, not data).

### Equality, hashing, and mutability edges

- Structural `==` uses each field's own `==`. Float fields follow IEEE 754 per
  the floating-point-semantics decision (`NaN` compares unequal to itself).
- **Hashable requires every field be hashable.** Float fields are not hashable
  (`NaN`, signed zero), matching the numeric model, so a data class with a float
  field may derive Equatable but not Hashable. An explicit bit-equality opt-in for
  float keys is deferred.
- A data class **may** have `writable` fields; immutability is not required.
  Map-key safety does not depend on forcing immutability: a key is **moved** into
  the collection and the one-writer rule prevents post-insertion mutation through
  an alias. Immutable value objects remain the recommended, idiomatic form.
- Structural derives cover the declaring type's own fields only. Deriving on
  `open`/inheritable classes is restricted; the interaction is settled with the
  inheritance decision at implementation time (records typically final).

## Alternatives considered

- **A user-defined value/`struct` type with implicit copy:** rejected — collides
  with the Copy-vs-move decision's "no user-defined struct in v1.0"; a post-1.0
  revisit if engine profiling demands inline layout.
- **Kotlin-style automatic structural equality for a class kind:** rejected —
  implicit structural equality on types that may carry identity or mutable state
  is a footgun; opt-in derives keep it explicit.
- **A distinct `record`/`struct` keyword:** rejected — duplicates class
  machinery (fields, constructors, generics, traits, borrow interaction) and
  fights one-word-one-meaning for no capability the derive does not already give.
- **A language-owned DTO type with built-in serialization:** rejected —
  serialization/transport is a framework concern; with no reflection it routes
  through the attributes/codegen derive mechanism.
- **Runtime reflection-based equals/serialize:** rejected — fights the headless
  representation (record 0082), defeats DCE, and is a deserialization-attack
  surface, per the attributes/reflection stance.

## Consequences

- No new type kind; classes remain the single owned aggregate and move semantics
  are unchanged.
- Structural behavior is explicit, opt-in, and greppable; identity equality
  stays the default.
- Value objects and DTOs are expressible with no language additions beyond the
  derive engine the attributes decision already owns.
- Hashable data classes enable composite `Dictionary`/`Set` keys.
- The feature adds no runtime metadata and is a pure consumer of existing
  decisions.

## Sequencing

Design is locked; implementation is deferred behind hard dependencies:

- the attributes & compile-time codegen decision (the derive engine);
- the interfaces/`Cloneable` decision and an `Equatable`/`Hashable`/`Displayable`
  interface surface (around Stage 35);
- named arguments (decision 0098, Stage 23a) for ergonomic copy-with-changes;
- generics (Phase D) for generic data classes.

It does not compete with the Stage 21 borrow checker. The implementation stage is
assigned when the derive engine lands; a `data class` sugar, if adopted, is
sequenced after the raw derives prove out.

## Affected components

Semantic analysis, the attributes/derive engine, stdlib interface surface
(`Equatable`/`Hashable`/`Displayable`/`Cloneable`), collection key contracts, the
language specification, and the master plan. No compiler code changes land with
this record; it fixes the model.

## Invalidated elsewhere

- Any framing that Doria needs a separate `record`, `struct`, or value type to
  serve DTOs or value objects.
- Any assumption that DTO serialization is a language or standard-library
  responsibility rather than a framework consumer of the derive mechanism.
