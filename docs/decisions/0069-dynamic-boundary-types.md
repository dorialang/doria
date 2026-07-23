# 0069 Dynamic boundary types

Status: Accepted

## Context

Doria needs an explicit way to model data that enters from dynamic boundaries: JSON-like input, PHP bridge payloads, migration output, and other host values whose precise shape is not known at the declaration site.

Earlier Doria wording treated several PHP-shaped names as ordinary type-position names: `mixed`, `object`, `null`, and `resource`. That made the surface look familiar, but it blurred Doria's stronger type model. Accepting `object` as a general top type, accepting `null` as a user-written type, or letting `mixed` behave like "anything goes" would weaken monomorphized generics, Copy-vs-move classification, and the borrow checker.

## Decision

Doria has exactly one dynamic type: `mixed`.

`mixed` has three laws:

1. `mixed` is unknown-flavored, never any-flavored. A `mixed` value permits no property access, method calls, arithmetic, concatenation, interpolation, comparison, or other typed operation until it is narrowed with `is` or `match`.
2. Values may flow into `mixed` implicitly. This is the deliberate dynamic-boundary exemption from the no-implicit-conversion rule. Values do not flow out implicitly; the program must narrow the value first. There is no cast spelling.
3. `mixed` is a boxed, runtime-tagged move type, always, even when the payload is a Copy value.

`object` does not exist in Doria. Writing `object` in type position is an ordinary unknown-type error. Diagnostics may add the targeted suggestion: "Doria has no `object` type; use `mixed` and narrow with `is` or `match`."

`null` is a literal, not a type-position name. The compiler may have an internal null type for nullable machinery and flow analysis, but user source spells nullable values as `?T`. Writing `null` in type position is rejected with a diagnostic suggesting nullable syntax.

`resource` is reserved for the Phase I PHP bridge. It is not a usable core type. Until the bridge designs and implements it, `resource` in type position is rejected with an unsupported-feature diagnostic naming it reserved for PHP interop.

`void` is return-position only. Writing `void` in local declarations, parameters, properties, collection arguments, or other value positions is rejected with a diagnostic saying `void` is only valid as a return type.

## Alternatives Considered

### Any-flavored `mixed`

Rejected. An any-flavored `mixed` would allow operations to punch through the type checker and would undermine monomorphization, Copy-vs-move classification, and the borrow checker. Doria needs dynamic boundaries, not ambient dynamic typing.

### Keep `object`

Rejected. `mixed` plus narrowing with `is` or `match` covers dynamic object-shaped input without creating a reflection-shaped top type. Reflection and dynamic object inspection are out of scope for the core language model.

### `null` as a usable type

Rejected. Nullable values are spelled `?T`. Treating `null` as a user-authored type would make APIs less precise and invite null-only declarations where absence should be modeled by a nullable value or a domain result.

### `resource` as a core type now

Rejected. Resource handles need the PHP bridge and host-lifetime design. Reserving the word keeps the surface available without pretending the core language has a complete resource model.

## Consequences

Docs and editor token lists must stop presenting `object`, `null`, or `resource` as ordinary usable type names.

The semantic checker must reject `object`, `null`, `resource`, and non-return-position `void` wherever current parsing and type resolution can see them.

Stage 22 implements static narrowing for `mixed` through `is` on the shared
dataflow framework. Stage 28 owns `match` and its narrowing integration. Stage
23 owns the boxed runtime representation. Static acceptance must not be used as
permission to invent placeholder runtime behavior before that representation
exists.

The PHP backend must not lower Doria `resource` to PHP `mixed` as a source-level Doria feature. Any eventual PHP resource bridge behavior must be introduced through the Phase I bridge design.

## Affected Components

- `SPEC.md` basic type system and backend notes
- semantic type resolution and diagnostics
- semantic diagnostic tests
- Doria editor/token guardrails
- website docs, especially `language-basics/types-and-literals`
