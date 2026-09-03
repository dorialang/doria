# Decision 0131: Constructor Property Overrides And Constructor-Only Parameters

- **Status:** Accepted
- **Accepted:** 2026-09-02
- **Implementation Status:** Implemented By Post-Stage-34 Constructor Parameter Roles Corrective Beat
- **Amends:** Decisions 0080, 0081, 0083, 0086, 0089, 0090, 0098, 0122, 0125, and 0130

## Context

Doria promotes constructor parameters by default. After Stage 34, an unmarked
child constructor parameter with the same name as an inherited external property
therefore attempted to declare hidden duplicate storage and correctly reached
E0727. The language lacked syntax for the two intentions that do not declare a
new property: reusing the inherited property relationship and accepting an input
only for construction.

## Decision

Constructor parameters have one explicit compiler-owned role:

```text
unmarked     new externally accessible promoted property
internal     new internal promoted property
override     inherited-property override parameter
parameter    constructor-only parameter
```

`parameter` is a reserved keyword. `param` remains an identifier and is not an
alias. The canonical order is attributes, role, `writable` or `take`, type,
variable, and default. `internal`, `override`, and `parameter` are mutually
exclusive. `parameter` is valid only in a concrete class constructor; parameter-
position `override` is valid only in a derived concrete class constructor.

Default and internal promotion retain their existing property, initialization,
layout, attribute-role, ownership, and cleanup behavior. This remains Doria's
simple-class style.

## Inherited Property Override

An `override` constructor parameter must resolve to exactly one inherited,
externally visible instance property of the same name. After generic
substitution, its type, nullability, and generic or collection shape must match
exactly. Internal and static properties, methods, constants, and cross-kind
members are not valid targets. The root declaration retains accessibility,
writability, authored attributes, physical ownership, layout, and cleanup.

The override declaration is a callable parameter and a compile-time property-
family relation. It creates no field, promoted assignment, initializer,
destructor obligation, vtable entry, runtime tag, or reflection data. Parent and
child property access resolve to the root field. Intermediate override
declarations preserve that same family. `writable` and `take` describe only the
incoming parameter mode; they do not change the root property contract.

The parent constructor remains explicit and first where Decision 0130 requires
it. Neither role performs implicit forwarding. The override declaration itself
does not write the inherited property; the parent construction phase initializes
it. A later child write is an ordinary explicit write and must satisfy the root
property's existing mutability rules.

## Constructor-Only Parameter

A `parameter` constructor parameter participates in arity, named and positional
binding, source-order evaluation, defaults, generic inference, body scope,
borrowing, ownership, narrowing, checked effects, parameter attributes, and
tooling signatures. It participates in no member lookup, property layout,
property initialization, class size, destruction order, or property metadata.

Readonly and writable constructor-only values follow ordinary parameter borrow
rules. A `take` constructor-only value follows ordinary ownership and cleanup
rules until moved. A Move value does not require `take` merely because the
callable is a constructor; only actual property promotion requires ownership
transfer. A same-named explicit property remains a separate symbol and remains
an E0500 definite-initialization obligation until explicitly initialized.

## Property Families And Representation

Compiler-owned property-family facts record the root declaring class, root
property identity and source span, exact type, access and writability contract,
and descendant override parameter identities. Layout continues to contain only
physical root and newly promoted or explicit properties. These facts are static
semantic/HIR authority and never enter object headers or public metadata.

AST and HIR preserve ordinary, promoted, inherited-override, and constructor-
only roles separately, including role, mode, type, variable, default, and whole
parameter spans. Parameter role participates in incremental fingerprints. MIR
uses ordinary callable inputs for `override` and `parameter`; it emits no new
runtime operation for either role. All backends consume the checked role and
layout facts rather than inferring promotion from constructor position.

## Attributes And Metadata

An actual promoted parameter has `Parameter` and `PromotedProperty` target roles.
An override or constructor-only parameter has `Parameter` only. Attributes are
authored-only and are not copied to the root property. Attribute constructor
binding still uses every callable parameter name, type, and default. Metadata
schemas 1, 2, and 3 and processor protocol version 1 remain exact; layout and
property-family internals remain private.

## Diagnostics

E0727 remains `Inherited Member Cannot Be Hidden` for actual hiding, incompatible
storage, static/constant/cross-kind collisions, and explicit class-body property
redeclaration. A compatible unmarked child promotion receives a precise missing-
`override` diagnostic with a machine-applicable `override` insertion and a
reviewed `parameter` alternative. Explicit no-target and contract-mismatch
overrides, invalid placement, conflicting or duplicated roles, and modifier order
receive causal role diagnostics. E0500 remains authoritative for explicit
property initialization and may explain that a same-named constructor-only input
does not initialize the property.

## Parent Construction, Failure, And Cleanup

Decision 0130's one allocation, root-to-derived construction, explicit required
parent call, no implicit forwarding, partial-object cleanup, derived-to-root
destruction, and lifecycle-phase dispatch remain unchanged. Only actual new
promoted properties enter the child initialization and property-drop order.
Override and constructor-only values use ordinary frame cleanup, so ownership
transfer cannot produce a duplicate drop. Panic remains abort-only.

## Performance And Security

Each override and constructor-only parameter adds zero object fields, zero
automatic property stores, zero property cleanup obligations, and zero runtime
role tags. Object allocation and the callable ABI are unchanged. Property access
uses the existing compile-time offset. The compiler adds no hidden storage,
unchecked cast, runtime lookup, reflection, or host-language property behavior.

## Non-Goals

This decision does not add class-body property overrides, open or virtual
properties, property hooks, `protected`, property covariance, generic variance,
implicit parent forwarding, constructor overloading, the `param` alias, runtime
reflection, metadata schema 4, processor protocol 2, indexed-foreach behavior,
interfaces, traits, or Stage 35 execution.

## Invalidated Elsewhere

- Decision 0130's blanket no-property-redeclaration wording is narrowed: hidden
  duplicate storage remains forbidden, while a compatible constructor parameter
  marked `override` records one inherited property relationship.
- Constructor initialization, ownership, cleanup, attributes, HIR, PHP lowering,
  and incremental logic may no longer infer promotion merely from constructor
  position.
- Website examples that manually rename child constructor parameters to avoid
  E0727 should use `override`; examples that manually initialize a differently
  named or transformed explicit property should use `parameter`.
- Official language tooling must consume compiler-owned role and property-family
  facts and keep same-named parameter and property symbols distinct.
- The indexed-foreach corrective work does not consume Decision 0131. Decision 0132 owns that separate subject; the next record number remains unallocated.
