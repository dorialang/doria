# Decision 0093: Nullable types and narrowing

Status: Accepted

## Context

Doria already used `?string` for `read_line` EOF, but that narrow seed did not
provide the general `?T` model promised by D5. Other scalar types and classes
needed one language-level absence model, flow-sensitive proof before ordinary
member access, null-safe access, and behavior shared by the interpreter,
Cranelift, LLVM, and compatibility backends.

Decision 0069 separately defines `mixed` as Doria's only dynamic boundary type.
This record consumes that decision's static narrowing rules; it does not alter
them or pull the Stage 23 runtime box forward.

## Decision

### Nullable values

`?T` contains the values of `T` and `null`. `null` is a literal with an internal
semantic type, not a source type-position name. It is assignable to `?T` and
`mixed`, but not to non-nullable `T`. Doria adds no nullable truthiness.

The `??` operator evaluates its left operand once. It produces that value when
present and evaluates its right operand only when the left operand is null. The
result is the compatible non-null type when the fallback supplies it.

The `?->` operator evaluates a nullable class receiver once. It skips the
property access, method call, and call arguments when the receiver is null. A
value-producing access returns the member result made nullable; an already
nullable result remains one nullable layer. Ordinary `->` on a possibly-null
receiver is rejected until control flow proves the receiver non-null.

### Flow narrowing

Nullable and exact-type facts are a forward analysis over the shared control-flow
graph and `dataflow.rs` solver. Facts attach to lexical binding identities, not
names. Branch assumptions establish facts for `== null`, `!= null`, and `is`;
assignments kill or replace facts; joins keep only facts true on every incoming
path. Short-circuit `&&` and `||` propagate only facts guaranteed by the path
that evaluates their right operand and by the path entering a guarded block.

This analysis is backend-independent and is designed to accept more fact sources
later. Stage 28 `match` lowering will feed the same analysis rather than adding
another flow engine.

### Exact `is` tests

`$value is T` tests and narrows `mixed` or `?T` against a concrete exact type:
fixed-width integers, floats, `bool`, `string`, or a declared concrete class. For
`?T is T`, the runtime test is presence. A statically incompatible exact test is
false. Tests over already concrete values are valid; an always-true or
always-false lint is a diagnostics follow-up and does not change behavior.

Stage 22 does not perform subtype or interface conformance tests. Source using
those forms parses, then receives a Stage 34 or Stage 35 unsupported-feature
diagnostic. `instanceof` and its migration fix are namespace-stage work, not part
of this record.

### Runtime representation and ownership

A nullable concrete class uses the null pointer for absence, so `?Class` remains
one pointer wide. Nullable non-class values use an explicit presence word plus
their payload. This is a semantic layout requirement, not permission for a
backend to define Doria; Cranelift and LLVM may represent the pair differently
inside their private IR while preserving the same Doria ABI and behavior.

`?T` has the ownership classification of `T`. Adding `null` adds an inhabitant,
not an allocation: `?int`, `?bool`, floats, and `?string` are Copy when their
payload is Copy, while `?Class` is a move value and retains the class's cleanup
obligations when present.

Decision 0069 remains authoritative for `mixed`: every value may flow in, no
typed operation may occur before `is` narrowing, and `mixed` is always a move
type. Stage 22 implements those static rules only. A live `mixed` value reaching
native MIR lowering receives a diagnostic naming Stage 23; no placeholder
runtime representation exists.

## Alternatives considered

### Treat every nullable as a pointer

Rejected. Integers, floats, and bools are values rather than heap allocations.
Boxing them merely to represent absence would impose allocation and ownership
costs that are not part of Doria semantics.

### Use backend-specific niches immediately

Rejected for this stage. A presence word is explicit, portable, and easy to
compare across the interpreter and both native backends. Niche optimization may
be added later only if it preserves the accepted ABI and observable semantics.

### Keep ad hoc narrowing in statement checking

Rejected. It would fail at joins, loops, short-circuit expressions, and lexical
shadowing, and would duplicate the control-flow machinery already established
for constructor initialization.

### Implement hierarchy, interfaces, or `match` now

Rejected. Exact tests require no dynamic dispatch metadata. Hierarchy tests need
Stage 34, interface tests need Stage 35, and `match` syntax and exhaustiveness
belong to Stage 28. Pulling them forward would blur their owning decisions.

## Consequences

General nullable scalar, string, and concrete-class values now cross locals,
properties, parameters, calls, and returns through typed MIR. Copy nullable
scalar and string values also cross static properties. Nullable concrete-class
statics remain deferred with other owned static properties because `?Class`
retains its payload's move ownership. The debug interpreter, Cranelift fast
profile, and LLVM release profile execute one durable fixture with exact output
parity.

The semantic checker now records resolved expression types for backend-independent
lowering and consults shared dataflow facts at each variable use. Backends consume
typed MIR and never perform source narrowing themselves.

`mixed` programs can be fully checked at their static boundary, including
ownership classification and prove-then-use rules, but cannot execute until
Stage 23 supplies `dr_mixed`. `match` narrowing, subtype tests, interface tests,
and nullable layout optimizations remain deferred to their owning stages.

## Affected components

Lexer and parser tokens for `is` and `?->`; AST and HIR expressions; semantic
type resolution and diagnostics; the shared control-flow graph and forward
dataflow analysis; typed MIR, validation, interpreter, Cranelift, LLVM, and PHP
compatibility lowering; ownership classification; native parity fixtures;
`SPEC.md`; the end-to-end plan; and pipeline/parity notes.

The separate `dorialang/doria-language-server` repository needs coordinated
accepted-syntax, diagnostic, and token coverage for `?T`, `??`, `?->`, and `is`.
It is intentionally not modified by this compiler-repository decision.

## Invalidated elsewhere

- `SPEC.md` statements that only the Stage 17 `?string` seed has native runtime
  support or that general nullable types remain unsupported.
- The Stage 22 plan entry that grouped `match` narrowing into this stage; Stage
  28 owns that integration.
- Decision 0069 wording that described all `mixed` narrowing as future Stage 22
  work; `is` narrowing is now implemented statically, while `match` waits for
  Stage 28 and the runtime box waits for Stage 23.
- Decision 0074 wording that described general nullable values as deferred and
  its seed-only null-pointer representation for `?string`; its EOF contract and
  historical account of the first nullable position remain.
- Pipeline and parity notes that placed Stage 22 wholly in the future or listed
  only the narrow `?string` runtime path.
