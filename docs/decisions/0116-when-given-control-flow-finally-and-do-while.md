# Decision 0116: When, Given, Control-Flow Finally, And Do-While

- **Status:** Accepted
- **Accepted:** 2026-08-13
- **Date:** 2026-08-13
- **Implementation status:** Stage 28a Slice 1 implemented; executable `finally` is Slice 2
- **Scope:** Value-returning conditional control flow, prepared predicates,
  post-tested loops, and the bounded control-flow finalizer model

## Context

Doria already has statement `if`, pre-tested loops, structured exits, ownership,
and match expressions. It also needs a conditional expression whose branches may
perform work before producing a value, a concise way to prepare shared condition
state, and one cleanup model that can later carry checked errors without becoming
backend-specific. Earlier records left important parts of that family open.

This decision settles the Doria semantics first. MIR, Cranelift, LLVM, and the
PHP compatibility backend implement these rules; none of them defines the rules.

## Plain-Language Model

`given` prepares values and checks shared conditions before another control-flow
construct runs. `when` is the value-returning form of `if`. `finally` performs
cleanup when its attached construct is leaving. `do ... while` runs its body once
before checking whether it should repeat.

Setup work in `given` happens once. A `given` predicate on a `while` loop is
checked again whenever the loop condition is checked. `finally` belongs to the
complete control-flow construct, not to each loop iteration.

## Control-Flow Family

`if` remains statement control flow and may omit `else`. `when` is an expression
and requires `else`. `given` attaches only to `if`, `when`, or `while` in v1.
`finally` attaches only to `if`, `when`, `while`, or `do ... while` in v1.

## `given` Grammar

```doria
given {
    let $ready = serviceIsReady();
    $ready;
} if ($requestIsValid) {
    handleRequest();
}
```

The attached construct may instead be `when` or `while`. `given` does not attach
to `do`, `for`, `foreach`, `match`, or a bare block. No alias is accepted.

## `given` Setup Phase

Declarations and expression statements returning `void` are setup. They execute
once in source order. A declaration is visible to later setup, predicates, the
attached condition and bodies, and a future attached `finally`.

## `given` Predicate Phase

The first standalone `bool` expression begins the predicate phase. Only further
standalone `bool` expressions may follow. Setup after that boundary is rejected.
A discarded expression returning neither `void` nor `bool` is rejected.

## `given` Evaluation Frequency

For `given ... if` and `given ... when`, setup and predicates run once before
branch selection. For `given ... while`, setup runs once, while predicates run
before every condition check: initially, after body completion, and after
`continue`.

## `given` Short-Circuiting

Predicates evaluate in source order. The first false predicate skips later
predicates and the attached condition. Doria does not evaluate an `if`,
`else if`, `when`, `else when`, or `while` condition after the shared gate fails.

## `given` Scope

Given declarations are visible throughout the complete attached construct and
future finalizer, then leave scope. Branch and loop-body locals do not become
visible in `finally`. Ordinary lexical shadowing applies.

## `given` On `if`

Predicates run once before every condition in the chain. A failed gate selects
the final `else` when present; otherwise no branch runs.

## `given` On `when`

Predicates run once before every condition in the chain. A failed gate skips all
conditional branches and selects the mandatory `else` result.

## `given` On `while`

The predicate header precedes the loop condition. Body completion and
`continue` return to that header. A failed predicate exits without evaluating
the loop condition. Predicate values are not cached automatically.

## False Predicate Behavior

Gate failure never falls through to an `else if` or `else when` condition. It
skips the complete conditional chain and uses only its unconditional fallback,
when one exists.

## `when` Grammar

```doria
let $label = when ($ready): string {
    return "ready";
} else when ($waiting) {
    return "waiting";
} else {
    return "unavailable";
};
```

The result annotation appears only on the head. Conditions require `bool`.
Branches are statement blocks. `else` is mandatory. There is no trailing-value
syntax, `yield` keyword, truthiness, or general block-expression semantics.

## `when` Result Typing

Result typing uses this priority:

1. the explicit head annotation;
2. the surrounding expected type;
3. the first reachable yield in the head branch.

Every other yield uses existing assignment compatibility. Doria does not infer
`mixed` to hide disagreement and does not widen numerics. An unannotated all-null
`when` requires an expected nullable type. `void` is never a `when` result.

Expected types flow from declarations, assignments, returns, callable arguments,
property or static initializers where already permitted, nested `when`, match
arms, ternary branches, nullable contexts, and `mixed` contexts.

## `when` Return-To-Yield

`return expression;` in a branch yields from the nearest enclosing `when`; it
does not return from the function. Nested `when` expressions therefore have
independent yield targets. Bare `return;` is invalid.

## `when` Exhaustiveness

Every normally completing path in every branch must yield one value. Fatal panic
may diverge without a value. The existing completion lattice supplies this
analysis; `when` does not create a second return analysis.

## `when` Ownership

Copy results copy. Move results are acquired exactly once into the merge result
before selected-branch cleanup. A borrowed result must not outlive its owner.
Unselected branches neither move nor drop their values. A value borrowed from a
`given` local cannot escape the complete construct.

## `when` Cleanup

The selected result is acquired before branch locals are destroyed. Branch
locals then drop, future `finally` runs, and `given` locals drop after the
finalizer. Unselected branch locals never exist.

## `do ... while` Grammar

```doria
do {
    advance();
} while ($ready);
```

The ordinary form requires its terminating semicolon. With a finalizer there is
no semicolon between the condition and `finally`, and none after the finalizer:

```doria
do {
    advance();
} while ($ready) finally {
    close();
}
```

`given` does not attach to `do`.

## `do ... while` Execution

The body runs before the first strict-`bool` condition check. `continue` reaches
the condition. A true condition returns to the body; false exits. `break` exits.

## `finally` Attachment Set

The v1 set is `if`, `when`, `while`, and `do ... while`, including accepted
`given` forms. `for`, `foreach`, `match`, and bare blocks reject `finally`.

## `finally` Activation

A finalizer becomes active before the first `given` setup statement executes.
It therefore covers setup, predicate, condition, and selected-body exits.

## `finally` Trigger Paths

The finalizer runs exactly once when its construct exits normally or through a
structured transfer. A loop finalizer does not run once per iteration or on
`continue`. Fatal panic remains abort-without-cleanup and does not run it.

## `finally` Scope

Given declarations remain visible and alive through `finally`. Branch and body
locals leave scope before it. The finalizer's own locals are scoped to its block.

## `finally` Transfer Restrictions

A transfer inside `finally` is forbidden when it escapes the finalizer, replaces
the pending outcome, or cancels it. Nested control flow wholly contained inside
the finalizer remains legal.

## `finally` Cleanup Order

An outgoing `when` result or function return is acquired first. Branch locals
drop next. The finalizer runs. Given locals drop afterward. This preserves both
ownership and lexical lifetime.

## Nested `finally`

Nested finalizers run from innermost to outermost.

## Borrowing And Moves

The existing ownership and borrow analyses apply across the full construct.
Finalizers do not revive moved values, extend branch-local lifetimes, or permit
a borrow to escape its owner. Cleanup acquisition order is part of correctness,
not a backend optimization.

## Fatal Panic

Fatal panic does not unwind and runs no finalizer or destructor. PHP compatibility
must preserve that Doria rule instead of inheriting PHP exception unwinding.

## Future Checked Errors

Stage 29 checked errors reuse the same structured-exit and finalizer regions.
They must not invent a second cleanup model. This decision does not implement
`try`, `catch`, `throw`, or `throws`.

## MIR And Structured Exit Regions

Slice 1 lowers `given`, `when`, and `do ... while` once into explicit validated
CFG. Validation-only plans preserve their source control-flow identity so shared
validation can prove predicate routing, `continue` targets, and one result write
per completing `when` path. Backends execute the ordinary blocks and branches;
there is no runtime control-flow object.

Executable `finally` is Slice 2. Slice 1 preserves its AST/HIR identity but emits
one stage-named diagnostic before MIR. Any finalizer marker reaching MIR is a
malformed-IR error.

## PHP Compatibility

The PHP backend preserves setup frequency, source-order short-circuiting,
condition skipping, result typing established by Doria, and post-tested loop
behavior. Backend-private closures may materialize a `when` expression, but they
do not define Doria return or ownership semantics. PHP truthiness is not used.
Executable `finally` remains unavailable until Slice 2.

## Performance Impact

These constructs lower to direct CFG. There is no runtime `given`, `when`, or
loop object and no required heap allocation. Loop scratch remains function-entry
storage. Opt-in structural reports count `when`, `else when`, `given` predicates,
and `do ... while` while walking MIR already being materialized. Controlled
timing remains **Pending Available Runner** and does not block development.

## Implementation Slices

Stage 28a Slice 1 implements this decision's authority, executable `when`,
`given` on `if`/`when`/`while`, base `do ... while`, finalizer grammar and
preservation, backend parity, and tooling synchronization.

Slice 2 implements shared finalizer regions and routes normal completion,
`return`, `break`, `continue`, and future checked-error crossings through them.
Stage 29 remains blocked until Slice 2 completes.

## Explicit Exclusions

This decision does not add executable finalizers in Slice 1, `given` on `do`,
finalizers on excluded constructs, truthiness, block expressions, general
`yield`, checked errors, closures, namespaces, autoloading, or reflection.

## Consequences

The control-flow family now has one settled division: `if` controls statements,
`when` produces values, `given` prepares a shared gate, and `finally` owns exit
cleanup. The parser retains exact source identities, semantic analysis owns the
shared plans, MIR owns executable CFG validation, and backends remain consumers.

## Affected Components

Compiler lexer/parser, AST/HIR, contextual typing, completion analysis,
narrowing, ownership, MIR lowering and validation, interpreter, native backends,
PHP compatibility, diagnostics, performance reporting, language server, editor
grammars, website examples, and authority guards are affected.

## Invalidated Elsewhere

- Decision 0009's open questions about `when`, finalizer paths/scope/order, and
  do-while punctuation are settled here.
- Decision 0020's one-time wording is narrowed: `given ... while` predicates
  reevaluate before every condition check.
- Decision 0097's result inference is amended: a surrounding expected type may
  type an unannotated `when` before head-branch inference.
- Stage 28a Slice 2 must implement this exact finalizer model.
- Stage 29 must reuse the same structured finalizer regions.
- Stage 31 namespace and compile-time autoload authority, Stage 33 Baton work,
  self-hosting, and the performance workstream gain no new semantics here.
