# Decision 0115: Match Expressions, Patterns, Exhaustiveness, Narrowing, And Ownership

- **Status:** Accepted
- **Accepted:** 2026-08-12 by Andrew Masiye
- **Date:** 2026-08-12
- **Owners:** Doria language, compiler, runtime, and tooling design
- **Implementation status:** Implemented
- **Scope:** Core `match` expressions, pattern guards, exhaustiveness,
  narrowing, readonly and consuming payload access, ternary desugaring, and
  complete Stage 28 behavior

## Context

Doria needs one value-selection construct that is exhaustive, strictly typed,
and compatible with nullable, `mixed`, and enum data without inheriting PHP's
coercions. Stage 27 supplied nominal inline enums and central payload layout.
Stage 22 supplied shared narrowing and dataflow. This decision joins those
foundations without making any backend, including PHP, the source of language
semantics.

Stage 28 was split by implementation risk. Slice 1 delivered guard-free core
patterns and full ternary. Slice 2 completes pattern guards, guard-aware
exhaustiveness, and explicit whole-scrutinee consumption.

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

Every arm has its own lexical scope. Payload and type bindings are readonly,
obey existing duplicate/shadowing rules, and disappear at the arm boundary.
They are visible in that arm's guard and result expression. A binding masks an
outer name consistently whether the binding is Copy or Move. Narrowing facts
likewise do not leak to another arm or beyond the match.

## Pattern Guards

The only guard spelling is `if`:

```doria
string $label = match ($state) {
    State::Ready($attempt) if $attempt > 1 => "retried",
    State::Ready($attempt) => "ready {$attempt}",
    State::Waiting => "waiting",
};
```

`when` is Doria's future value-returning conditional and `where` has no guard
role. Either token in guard position is rejected with a structured replacement
to `if`. A guard belongs to its arm, must produce `bool`, and uses no truthiness.

For each reached arm, the pattern is tested first. On a pattern hit the compiler
establishes readonly pattern views and evaluates the guard exactly once. A false
guard ends those views and continues to the next pattern. A true guard
materializes the selected arm bindings and evaluates the result. Pattern misses
do not run guards. Ordinary guard side effects remain observable in source order.

`default` is unconditional and cannot have a guard. `match (true)` arms do not
accept guards; combine their bool conditions with the ordinary short-circuit
operators instead.

## Guard-Aware Coverage And Reachability

A runtime-guarded arm does not complete coverage because its pattern may match
while its guard is false. A later unguarded copy of the pattern may complete
coverage. Once an unguarded arm covers a pattern, later guarded or unguarded
copies are unreachable. The rule applies uniformly to enum cases, literals,
`null`, and exact type patterns.

A compile-time `true` guard counts as unguarded coverage and is diagnosed as
redundant. A compile-time `false` guard is unreachable and contributes no
coverage. The compiler does not try to prove arbitrary runtime predicates.

## Consuming Match

`match (take $value)` transfers the whole scrutinee to the match. `take` here is
a scrutinee modifier, not a general unary operator. The source becomes unusable
when the match begins, and one match-owned temporary carries its active payload
and cleanup obligation.

Copy scrutinees reject unnecessary `take`. Named Move sources must satisfy the
existing movable-place rules; match does not widen nested-property moves or
bypass alias checking. Current consuming domains are Move payload enums,
nullable Move values, `mixed`, concrete class values where exact matching is
already supported, and owned temporaries of those types.

In a selected consuming arm, a Move payload becomes one owned binding and a Copy
payload follows ordinary Copy behavior. Moved fields are removed from the
match-owned cleanup obligation. Ignored active payloads remain owned by the
temporary and drop after the arm result is safely acquired, in the ordinary
reverse field order. Nullable presence and a `mixed` box's payload obligation
are cleared before final cleanup when their Move payload is extracted.

Consumption is deliberately whole-value only. Payload-level `take`,
`match (writable $value)`, and writable payload bindings are rejected. Writable
patterns are not part of Doria v1: failed guards must not mutate data observed by
later arms. Mutation begins after an owned selected result is assigned to a
writable destination.

## Guarded Consumption

Consuming guards use a two-phase binding. During the guard, each payload name is
a readonly, non-owning view. If the guard succeeds, the same source name denotes
the selected arm's final Copy or owned binding. If it fails, no ownership moves,
no retain survives, and the match temporary remains intact for later arms.
Tooling presents this as one coherent arm-local source binding even though the
compiler tracks the guard view and selected owner separately.

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
blocks, optionally enters a guard-view block, materializes final bindings only
on the guard's true edge, assigns one merge local, and reaches one merge block.
MIR carries an explicit borrowed/consumed match mode and distinct guard-view,
borrowed-arm, and consumed-arm binding modes. It also carries enum/case identity,
central payload type/layout identity, nullable and `mixed` tests, and result-plan
identity sufficient for independent validation.

The shared validator rejects malformed dispatch, projection, ownership, and
merge paths before the interpreter or either native backend sees them.

## PHP Compatibility

PHP consumes the checked semantic plan and preserves strict Doria comparisons,
ordered guards, exact type identity, arm-local payload projections, and one-time
scrutinee evaluation. Values crossing into `mixed` use one backend-private
tagged representation that preserves Doria integer width and signedness, float
width, enum identity, class identity, payload, and cleanup obligation. `is` and
type-binding match read the same tag. Ordinary non-`mixed` values stay on their
existing fast paths. Generated PHP does not use `get_debug_type`, host
truthiness, loose equality, or reflection to define Doria semantics.

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
| Guarded arm | Pattern test plus one reached bool guard |
| Guard false | One branch to the next pattern |
| Consuming enum match | One bounded move plus selected extraction |
| Exact PHP `mixed` test | O(1) backend-private tag comparison |
| Ternary | O(1), one selected branch |

Match creates no runtime match or guard object, reflection, inactive payload
read, hidden clone/share, whole-enum readonly move copy, runtime exhaustiveness
check, second `mixed` box, or loop-body dynamic stack allocation. The existing
single `mixed` box remains the only allocation at that boundary. The opt-in
performance report adds structural match counters during existing lowering;
ordinary compilation gains no reporting traversal.

Stage 28 controlled timing is **Pending Available Runner** and non-blocking.
This is not a performance-pass claim.

## Explicit Exclusions

Stage 28 excludes writable match and payload patterns; payload-level `take`;
partial moves; nested, wildcard, or-pattern, and range patterns; class and
collection destructuring; `when`, `given`, and control-flow `finally`; checked
errors; closures; namespaces/autoloading; hierarchy/interface patterns; generic
enums; reflection; automatic hashing and ordering.

## Consequences

Core match, pattern guards, explicit consuming match, and full ternary share one
typed, ownership-aware, validated CFG across the interpreter, Cranelift, LLVM,
and PHP. Stage 28 is complete. Decision 0116 now owns the separate Stage 28a
control-flow work; Stage 29 remains dependent on its finalizer slice.

## Affected Components

Lexer/parser, AST/HIR, constant evaluation, semantic analysis, narrowing,
ownership, MIR/lowering/validation, interpreter, Cranelift, LLVM, PHP,
diagnostics, performance reporting, language-server projections, editor
grammars, examples, website UAT, and authority guards.

## Implementation Slices

- **Stage 28 Slice 1 — Complete.** Guard-free core match, enum and exact
  type-binding patterns, exhaustiveness, narrowing, readonly payload observation,
  `match (true)`, ternary, backend parity, and tooling integration.
- **Stage 28 Slice 2 — Complete.** `if` pattern guards, guard-aware scope,
  evaluation, coverage and reachability; explicit `match (take $value)`;
  selected payload transfer and cleanup; rejected writable patterns; exact PHP
  `mixed` identity; backend parity; and tooling integration.

Stage 28 is **Complete**. Current sequencing is maintained by Decision 0116 and
the end-to-end plan rather than frozen in this Stage 28 record.

## PR #132 Review Closure

| Finding | Current code | Regression coverage | Compiler paths | Remaining gap | Disposition |
| --- | --- | --- | --- | --- | --- |
| Expected match-result types were not propagated from every context | One shared expected-expression-type path reaches match and ternary arms | Typed/grouped locals, returns, assignments, free/instance/static/constructor arguments, instance initializers, nullable/`mixed`, nested match, ternary, enum and Move destinations | Semantic checking and ownership | Runtime static match initializers remain outside the current static-initializer model | Closed |
| Copy pattern bindings could expose an outer moved binding | Arm scopes mask outer identities for Copy and Move bindings | Shadowed outer Move regression plus arm-scope diagnostics | Semantic, ownership, and tooling projections | None | Closed |
| PHP exact numeric type patterns after `mixed` lost width identity | Backend-private tagged `mixed` values drive both `is` and match | Executable signed/unsigned width, float width, bool, string, enum, and class identity matrix | PHP lowering and runtime helpers | PHP still rejects operations whose value semantics it cannot faithfully preserve | Closed |

## Invalidated Elsewhere

- Stage 27's active E0576 boundary and statements that core match is only parsed
  are historical; valid Slice 1 match now executes.
- Decision 0094's ternary direction is now executable through match rather than
  pending implementation.
- The open-questions audit no longer lists match as an unauthored subject.
- The former PHP B1301 boundary for valid exact numeric tests after `mixed` is
  removed; faithful backend-private tags preserve Doria identity instead.
- Stage 31 still requires a pre-implementation authority amendment for public
  `autoload` vocabulary, namespace-prefix-to-path mappings, main/test/generated
  autoload scopes, dependency source discovery, deterministic package graphs,
  incremental source indexing, and top-level execution across autoloaded files.
  Internal plans may use `SourceRoot`, `SourceMapping`, and
  `PackageSourceGraph`; the public manifest term remains `autoload`.
