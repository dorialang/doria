# Decision 0115: Match Expressions, Patterns, Exhaustiveness, Narrowing, And Ownership

- **Status:** Accepted
- **Accepted:** 2026-08-12 by Andrew Masiye
- **Date:** 2026-08-12
- **Owners:** Doria language, compiler, runtime, and tooling design
- **Scope:** Core `match` expressions, guard-free patterns, exhaustiveness,
  narrowing, readonly payload observation, ternary desugaring, and the Stage 28
  Slice 1/Slice 2 boundary

## Context

Doria needs one value-selection construct that is exhaustive, strictly typed,
and compatible with nullable, `mixed`, and enum data without inheriting PHP's
coercions. Stage 27 supplied nominal inline enums and central payload layout.
Stage 22 supplied shared narrowing and dataflow. This decision joins those
foundations without making any backend, including PHP, the source of language
semantics.

Stage 28 is split by implementation risk. Slice 1 executes guard-free core
patterns and full ternary. Slice 2 owns pattern guards, guard-aware
exhaustiveness, and the explicit writable/consuming payload-pattern review.

## Expression Shape

`match` is an expression. Each arm directly produces one expression:

```doria
string $label = match ($status) {
    Status::Draft => "draft",
    Status::Published => "published",
};
```

Arms are comma-separated and the final comma is optional. Statement blocks are
not arm values. Future `when` owns branches that need a small statement
workflow before producing a result.

## Scrutinee Evaluation

The scrutinee evaluates exactly once into one canonical compiler temporary.
Copy values follow their existing Copy rules. A named move value is borrowed
for the match and remains usable afterward. An owned temporary lives through
the selected arm and is destroyed only after the selected result is safely
acquired.

## Arm Order

Arms are considered in source order. Exactly one selected arm expression
executes. There is no fallthrough, and expressions in unselected arms do not
execute.

## Pattern Kinds

Slice 1 supports:

- qualified unit, backed, and payload enum cases;
- positional payload binding, or a case-only pattern that ignores all payloads;
- exact compile-time-known constants and literals;
- `null` for nullable scrutinees;
- exact `Type $binding` patterns for existing exact `is` domains;
- one final `default` catch-all;
- runtime `bool` conditions only in the deliberate `match (true)` mode.

There is no `_` wildcard, `else` alias, partial payload destructuring, nested
pattern, or-pattern, range pattern, class destructuring, or collection
destructuring in this slice.

## Enum Case Patterns

An enum case qualifier must name the scrutinee's nominal enum. A case from a
different enum is incompatible even when names or backing values coincide.
Covering one payload case covers the whole case whether its fields are bound or
ignored.

## Payload Destructuring

Parenthesized payload bindings are positional and their arity must equal the
case payload arity. Binding names need not repeat declaration field names, but
they must be unique within the pattern. Omitting parentheses matches the case
while ignoring every payload; it does not make that case constructible without
arguments.

## Literal And Constant Patterns

Ordinary value patterns must be known during compilation and have exactly the
scrutinee type. They may use the currently const-evaluable bool, fixed-width
integer, float, string, nullable `null`, enum-case, and accessible constant
forms. Runtime calls and other arbitrary expressions are rejected.

No numeric widening, string coercion, truthiness, enum/backing conversion, or
PHP loose comparison applies to patterns.

## Exact Type-Binding Patterns

`Type $binding` performs the same exact test accepted by `is`, introduces one
arm-local binding of that type, and applies the same exact-type fact to the
original lexical scrutinee where sound. It is not a cast, subtype test, or
interface test.

`?Type $binding`, `mixed $binding`, `void $binding`, and `null $binding` are not
valid type patterns. `null` is its own pattern and `default` covers an open
domain. Hierarchy and interface tests remain with Stages 34 and 35.

## `null` Patterns

`null` is valid only for a nullable scrutinee. It is distinct from every
present payload, including an enum's first tag and numeric zero. A nullable
finite domain is exhaustive only when it covers `null` and every present value,
or ends with `default`.

## `default`

At most one `default` arm is allowed. It must be last. Anything after it is
unreachable, and a `default` after already complete finite coverage is also
unreachable.

## `match (true)`

When the scrutinee is the literal `true`, every non-default pattern is a runtime
`bool` condition. Reached conditions evaluate once in source order, selection
stops at the first true condition, later conditions remain lazy, and `default`
is mandatory. No truthiness is allowed. `match (false)` does not enable this
mode.

## Exhaustiveness

Exhaustiveness is proven before MIR. There is no runtime "no arm matched"
panic.

- An enum requires every declared case or `default`.
- `bool` requires `true` and `false` or `default`.
- Nullable finite domains additionally require `null`.
- Integers, floats, and strings require `default`.
- `mixed` always requires `default`.
- `?Class` is exhaustive with `null` plus the exact class binding.
- Other class/open/future domains require `default` until their owning stages
  define a closed domain.

Missing enum-case diagnostics name the qualified cases.

## Duplicate And Unreachable Arms

The compiler rejects duplicate enum cases, literals, `null`, exact type
patterns, and `default`; incompatible patterns; arms after `default`; and
patterns fully covered by earlier arms. `match (true)` conditions are not
generally compared for semantic equivalence; only deterministic compile-time
duplicates are eligible for duplicate analysis.

## Arm Result Types

Every arm produces a non-void value. With an expected type, every arm must be
assignable to it. Otherwise the first valid arm establishes one result type and
all remaining arms must match it under existing assignment compatibility. The
compiler does not widen numerics or infer `mixed` as a fallback.

Existing `T` plus `null` nullable unification may produce `?T`. An all-null
match without an expected nullable type cannot infer a complete source type.
Only the selected arm acquires a move result.

## Pattern Binding Scope

Every arm has its own lexical scope. Payload and type bindings are readonly by
default, obey existing duplicate/shadowing rules, and disappear at the arm
boundary. Narrowing facts likewise do not leak to another arm or beyond the
match.

## Narrowing

Patterns feed the existing shared narrowing/dataflow framework. Enum case arms
know the exact present case, type-pattern arms know the exact type, and null
arms know absence. Assignments continue to kill or replace facts through the
existing rules. Backends receive checked facts; they do not infer narrowing.

## Readonly Ownership

Slice 1 is observational. Copy payload bindings receive ordinary copies;
strings retain their immutable handle. Move payload bindings are readonly
borrows tied to the scrutinee. Matching never hides a clone, share, partial
move, or whole-enum copy. A borrowed payload cannot escape its proven
provenance.

## Temporary Lifetime

A temporary scrutinee remains alive through dispatch, selected payload access,
and selected result acquisition. Only then may cleanup run. Unselected arms
create no owner or cleanup obligation.

## Equality

Constant patterns use exact Doria equality for the scrutinee type. Strings use
exact bytes, integers preserve width and signedness, enum identity is nominal,
nullable presence is separate, and floats keep existing IEEE equality including
NaN behavior. Aggregate equality never becomes raw `memcmp`.

## Ternary Desugaring

Full ternary is strict-bool, right-associative, and lazy:

```doria
$condition ? $left : $right
```

It is represented as the same two-arm bool match used by the semantic,
ownership, MIR, and backend paths. PHP's short ternary/Elvis form is rejected;
diagnostics explain `??` for null fallback and full `? :` for bool selection.

## MIR And CFG

Shared MIR evaluates one scrutinee temporary, dispatches through explicit test
blocks, enters one arm block, creates arm-local bindings, assigns one merge
local, and reaches one merge block. MIR carries enum/case identity, central
payload type/layout identity, nullable and `mixed` tests, payload Copy/borrow
mode, and result-plan identity sufficient for independent validation.

The shared validator rejects malformed dispatch, projection, ownership, and
merge paths before the interpreter or either native backend sees them.

## PHP Compatibility

PHP consumes the checked semantic plan and preserves strict Doria comparisons,
ordered conditions, exact type identity, arm-local payload projections, and
one-time scrutinee evaluation. Generated PHP may use backend-private helpers,
but PHP object identity, truthiness, reflection, and match behavior do not
define Doria semantics.

## Performance Impact

| Operation | Expected cost |
| --- | ---: |
| Unit/backed enum match | O(1) tag dispatch |
| Payload enum match | O(1) tag dispatch plus selected arm |
| bool match | O(1) branch |
| Open scalar match | O(number of tested patterns) |
| `match (true)` | O(number of reached conditions) |
| Type pattern | O(1) tag/type test |
| Payload projection | O(1), no allocation |
| Ternary | O(1), one selected branch |

Match creates no runtime object, reflection, inactive payload read, hidden
clone, whole-enum readonly move copy, runtime exhaustiveness check, or loop-body
dynamic stack allocation. The opt-in performance report adds structural match
counters during existing lowering; ordinary compilation gains no reporting
traversal.

Stage 28 controlled timing is **Pending Available Runner** and non-blocking.
This is not a performance-pass claim.

## Pattern Guard Boundary

Slice 1 does not assign or accept guard syntax. `if`, `when`, and `where`
candidate spellings are not silently selected. Slice 2 must settle the keyword,
scope, order, side effects, borrow lifetime, repeated cases, reachability, and
guard-aware exhaustiveness before guards execute.

## Explicit Exclusions

Slice 1 excludes pattern guards; writable or consuming payload patterns;
partial moves; nested, wildcard, or-pattern, and range patterns; class and
collection destructuring; `when`, `given`, and control-flow `finally`; checked
errors; closures; namespaces/autoloading; hierarchy/interface patterns; generic
enums; reflection; automatic hashing and ordering.

## Consequences

Core match and full ternary now share one typed, ownership-aware, validated CFG
across the interpreter, Cranelift, LLVM, and PHP. Stage 28 remains in progress
because guard semantics and explicit payload mutation/consumption have not yet
been reviewed or implemented.

## Affected Components

Lexer/parser, AST/HIR, constant evaluation, semantic analysis, narrowing,
ownership, MIR/lowering/validation, interpreter, Cranelift, LLVM, PHP,
diagnostics, performance reporting, language-server projections, editor
grammars, examples, website UAT, and authority guards.

## Implementation Slices

- **Stage 28 Slice 1 — Complete.** Guard-free core match, enum and exact
  type-binding patterns, exhaustiveness, narrowing, readonly payload observation,
  `match (true)`, ternary, backend parity, and tooling integration.
- **Stage 28 Slice 2 — Next.** Pattern guards, guard-aware diagnostics and
  exhaustiveness, and explicit writable/consuming payload-pattern review.

Stage 28 is **In Progress**. Stage 28a remains **Blocked Until Stage 28
Completes**.

## Invalidated Elsewhere

- Stage 27's active E0576 boundary and statements that core match is only parsed
  are historical; valid Slice 1 match now executes.
- Decision 0094's ternary direction is now executable through match rather than
  pending implementation.
- The open-questions audit no longer lists match as an unauthored subject.
- Stage 31 still requires a pre-implementation authority amendment for public
  `autoload` vocabulary, namespace-prefix-to-path mappings, main/test/generated
  autoload scopes, dependency source discovery, deterministic package graphs,
  incremental source indexing, and top-level execution across autoloaded files.
  Internal plans may use `SourceRoot`, `SourceMapping`, and
  `PackageSourceGraph`; the public manifest term remains `autoload`.
