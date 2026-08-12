# Decision 0114: Enums, Backed Cases, Payload Cases, And Inline Tagged Layout

- **Status:** Accepted
- **Accepted:** 2026-08-11 by Andrew Masiye
- **Date:** 2026-08-11
- **Owners:** Doria language, compiler, runtime, and tooling design
- **Scope:** Enum syntax and identity, unit and backed cases, payload cases,
  inline layout, ownership, equality, nullability, `mixed`, constants, and the
  boundary between Stage 27 enums and Stage 28 `match`

## Context

The end-to-end plan already accepts enums as nominal tagged values with
PHP-shaped declarations and Doria-owned semantics. Stage 27 is the first stage
that needs a durable representation for a finite set of cases and, later, for
case-specific owned payloads. The representation must remain native-first: PHP
syntax is familiar input and a compatibility lowering target, but PHP objects
or PHP enum reflection do not define Doria enum behavior.

Stage 27 lands in two implementation slices. Slice 1 establishes the complete
authority and executes unit and backed enums. It also parses payload enums so
the syntax clock stays ahead of the semantic clock. Slice 2 adds payload
execution, aggregate ABI handling, and case-aware copy, move, and destruction.

## Syntax

Unit enums declare semicolon-terminated cases:

```doria
enum Status
{
    case Draft;
    case Published;
}
```

Backed enums declare exactly one `int` or `string` backing type and one unique,
const-evaluable value for every case:

```doria
enum Priority: int
{
    case Low = 1;
    case High = 2;
}
```

```doria
enum Transport: string
{
    case Road = "road";
    case Rail = "rail";
}
```

Payload enums declare explicitly typed, readonly owned slots:

```doria
enum Shape
{
    case Circle(float $radius);
    case Rect(float $width, float $height);
}
```

Every case ends with `;`. Enum and case names use PascalCase. Payload names use
camelCase and are unique within their case.

## Enum Names And Case Names

Enums are top-level nominal type declarations. They share one type namespace
with classes, interfaces, traits, and future type declarations. Duplicate enum
names and cross-kind type-name collisions are errors. Cases occupy a namespace
local to their enum, so two different enums may use the same case name.

An enum must contain at least one case. Methods, properties, constants, and
nested type declarations are not valid enum members.

## Unit Cases

A unit case is constructed with static-member syntax:

```doria
Status $status = Status::Draft;
```

The value has type `Status`. It is an enum case, not a class constant, static
property, or static method. Calling a unit case with empty parentheses is an
error with a machine-applicable fix that removes the parentheses.

## Backed Enums

Only `int` and `string` may back an enum. A backed enum cannot contain payload
cases. Every case must provide one exactly typed compile-time value, and backing
values must be unique. There is no numeric widening, string coercion, aliasing,
or implicit conversion between an enum and its backing type.

The declaration-order case tag is private identity. A backing value is public
associated data and is never the sole internal discriminant.

## Backing Values And Readonly Properties

A backed enum exposes one intrinsic readonly property named `value`:

```doria
int $level = Priority::High->value;
```

The property has the declared backing type. Integer access is O(1) and
allocation-free. String access follows Doria's immutable string Copy/retain
contract. Calling `value()` uses the existing property-as-method diagnostic.
Unit and payload enums do not gain `value`.

No `name`, `cases`, `from`, `tryFrom`, `parse`, `rawValue`, reflection API, or
implicit display conversion is added by this decision.

## Payload Cases

Payload entries are owned enum slots, not borrowed function parameters. Their
types are explicit and their declarations do not accept `writable`, `take`,
defaults, promotion, or access modifiers. Construction may use positional or
named arguments and follows Decision 0098 binding while preserving source-order
evaluation.

Payloads initialize left to right. Copy payloads copy; move payloads move into
the enum without a hidden clone or share. A moved source becomes unusable. Only
the active case owns payloads, and active payloads are destroyed in reverse
declaration order.

During Stage 27 Slice 1, the compiler parsed and preserved payload schemas but
stopped payload case construction before MIR with one Slice 2 diagnostic.
Slice 2 removed that temporary boundary and delivered payload execution.

## Generic Enum Deferral

Generic enum syntax is reserved for future monomorphized enums:

```doria
enum Optional<T>
{
    case None;
    case Some(T $value);
}
```

The syntax parses, but semantic analysis reports that generic enums are not yet
implemented. This decision does not assign a fictional landing stage.

## Case Identity And Type Identity

Every enum and case has stable compiler-internal identities. Values preserve
both enum type identity and case identity through semantic analysis, MIR,
native lowering, PHP lowering, constants, nullable values, collections, and
`mixed`. Enum cases are not erased to arbitrary integers or strings.

## Inline Tagged Layout

An enum has one private discriminant and, for payload enums, one inline payload
area sized and aligned for the largest case. Payload fields have case-specific
offsets. The enum container is never heap-allocated merely because it is an
enum.

Unit and backed enums use the smallest practical unsigned inline tag width.
Tag assignment follows declaration order and is unobservable. Backed string
data lives in compile-time metadata/static storage rather than being duplicated
inside every value.

The exact private layout may change before 1.0. Cranelift and LLVM implement
this model; neither backend defines it.

## Finite Layout Requirement

Direct or indirect by-value payload recursion is rejected because it has no
finite inline layout. Recursion through an existing pointer-shaped owner such
as a class or collection may remain finite under that owner's existing rules.

Slice 1 records payload schemas and the future layout boundary. Slice 2 computes
payload offsets and aggregate layout.

## Copy And Move Classification

An enum is Copy only when every payload type in every case is Copy. It is Move
when any payload type in any case is Move. Classification is one type-level fact
and never changes based on the active runtime case.

Unit and backed enums are Copy. Payload-only scalar, string, and Copy-enum
cases are Copy; class, collection, `mixed`, shared-handle, and Move-enum
payloads make the whole enum Move.

## Equality

Enum equality requires the same nominal enum type. Unit and backed enum values
compare by case identity. Payload enums additionally compare active payloads
left to right when every payload type supports equality.

Different enum types cannot compare, even when backing values match. An enum
cannot compare directly with its backing value. Enums have no ordered
comparison and do not implicitly become numeric or string types.

## Nullability

`?Enum` uses the existing non-class nullable representation: a presence value
plus the enum payload. No discriminant niche is assumed. A present first case
whose tag is zero is distinct from `null`. Null comparison, coalescing, flow
narrowing, parameters, returns, properties, and locals follow Decision 0093.

## `mixed` Integration

Unit and backed enum values box into `mixed` with an explicit enum type ID and
case tag. A backed enum does not become an integer or string in the box. Exact
`is EnumName` narrowing recovers the enum type while preserving the mixed box's
existing ownership rules. Payload-enum boxing lands in Slice 2.

## Collection Placement

Unit and backed enums work in scalar-like Copy value positions including typed
arrays, lists, dictionary values, sorted-dictionary values, properties, and
generic arguments. Equality-based membership compares case identity.

This decision does not invent automatic `Hashable` or `Comparable`
conformance. Enum keys, sets, sorted sets, and priority queues remain governed
by their existing constraints. Payload-enum storage lands in Slice 2.

## Constants, Statics, And Defaults

Unit and backed cases are const-evaluable nominal values. They may appear in
every existing Copy-constant position: top-level and class constants, eligible
static properties, default arguments, property and local initializers, and
grouped Copy declarations. The constant model stores enum and case identity;
it does not erase cases to their tags or backing values.

Payload const evaluation lands in Slice 2.

## Display Conversion

Enums are not implicitly display-convertible. `echo` and interpolation reject
an enum value. A backed enum's `value` may be displayed explicitly. Plain and
payload enums do not gain a magic case-name conversion or a nonexistent `name`
property.

## `match` Boundary

`match` is the value-returning pattern construct owned by Stage 28. Stage 27
Slice 1 promotes `match` and `default` to real tokens and parses expression
syntax while preserving the scrutinee, arm order, patterns, bindings, arm
expressions, and spans. Semantic analysis emits one Stage 28 diagnostic and
does not inspect arm expressions in a way that produces cascades.

This decision does not implement exhaustiveness, pattern binding, narrowing,
arm type unification, MIR, backend execution, or guards.

## PHP Compatibility

The PHP compatibility backend emits native PHP unit and backed enums when the
semantics match and uses PHP's readonly `value` property for backed cases.
Generated comparisons remain strict. Payload enums stop before backend
lowering in Slice 1; Slice 2 owns faithful payload lowering rather than a class
approximation.

Implementation status: Slice 2 emits an immutable generated representation for
the complete payload enum, including nominal helper identity and explicit
case-aware equality. This backend representation does not define Doria
ownership, layout, or equality semantics.

PHP output does not define Doria enum semantics.

## Performance Impact

| Operation | Expected cost |
|---|---:|
| Unit/backed construction | constant tag, O(1) |
| Unit/backed copy | one inline tag copy |
| Equality | static type check, one tag comparison |
| `int` backing `value` | O(1), no allocation |
| `string` backing `value` | O(1), immutable string retain/copy |
| Nullable enum | presence plus inline tag |
| `mixed` boxing | existing mixed-box allocation |
| Enum container allocation | none |

For payload enums, construction is proportional to active static payload size;
trivial Copy uses one bounded aggregate copy, string-bearing Copy additionally
retains its handles, move performs bounded relocation without cloning, drop
visits active owned fields only, and equality checks the tag then active fields
with short-circuiting. Nullable values add presence beside the inline aggregate;
boxing uses the one existing `mixed` allocation. Ordinary enum containers and
collection elements add no enum allocation.

No reflection registry, virtual dispatch, runtime case-name lookup, reference
count, metadata pointer, or enum allocation is introduced. Controlled timing
remains pending an available runner and does not block this work.

## Explicit Exclusions

This decision does not add enum methods, user properties, reflection, implicit
display, automatic hashing/ordering, generic execution, property hooks, or
Stage 28 semantics. Slice 1 specifically excluded payload construction,
aggregate ABI, payload storage, payload copy/move/drop glue, payload `mixed`
boxing, and payload PHP execution.

## Consequences

- Semantic and MIR types gain nominal enum identity.
- MIR carries enum and case metadata and validates all enum operations before a
  backend sees them.
- The interpreter stores explicit enum values rather than untyped integers.
- Native backends use central inline layouts for scalar-tag and aggregate enum
  values.
- Constant evaluation, nullability, `mixed`, collections, PHP lowering, editor
  tooling, diagnostics, and structural reporting all share the same enum model.

## Affected Components

Lexer, parser, AST, semantic symbols and types, constant evaluation, ownership,
HIR/MIR, MIR validation and interpreter, Cranelift, LLVM, PHP compatibility,
native ABI metadata, runtime mixed transport, diagnostics, performance reports,
language-server/editor tooling, tests, specification, roadmap, and pipeline
authority are affected.

## Implementation Slices

### Stage 27 Slice 1

Authority, grammar, unit enums, backed enums, nominal identity, equality,
nullable and `mixed` integration, constant evaluation, supported collection
placement, PHP native lowering, editor integration, and Stage 28 `match` grammar.

### Stage 27 Slice 2

Payload execution, finite inline payload layout, Copy/Move classification,
case-aware copy/drop glue, aggregate ABI, all storage positions, payload
equality, payload `mixed` boxing, payload PHP lowering, and Stage 27 closure.

**Implementation status: Complete.** Both Stage 27 slices are complete. Payload
construction, layout, ownership, destruction, equality, nullable and `mixed`
transport, Copy constants/defaults, aggregate ABI, class/generic/collection
storage, PHP lowering, and durable interpreter/Cranelift/LLVM parity are
implemented. Generic enums remain deferred and `match` remains Stage 28.

Controlled payload-enum timing is `Pending Available Runner` and non-blocking.
No unmeasured result is recorded as a performance pass.

## Invalidated Elsewhere

This record invalidates any source that treats enum cases as class constants,
uses backing values as case identity, permits implicit enum display or
enum/backing conversion, heap-boxes enum containers by default, conflates tag
zero with `null`, grants automatic hashing/ordering, or describes payload/match
syntax as lexer/parser errors. It also invalidates pipeline notes that call
Stage 27 wholly next once Slice 1 is delivered.
