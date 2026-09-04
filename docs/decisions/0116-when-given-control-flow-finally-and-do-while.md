# Decision 0116: When, Given, Control-Flow Finally, And Do-While

- **Status:** Accepted
- **Accepted:** 2026-08-13
- **Date:** 2026-08-13
- **Implementation status:** Implemented; Stage 28a Slices 1 and 2 complete
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
attached condition and bodies, and the attached `finally`.

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
its finalizer, then leave scope. Branch and loop-body locals do not become
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
locals then drop, `finally` runs, and `given` locals drop after the
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

Source `return`, `when` yield, `break`, and `continue` inside `finally` remain
forbidden when they escape the finalizer, replace the pending outcome, or cancel
it. Decision 0123 amends the checked-error case: a checked Error may escape a
finalizer and supersedes the pending structured exit after its owned payload is
dropped exactly once. Nested control flow wholly contained inside the finalizer
remains legal.

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

## Checked Errors

Stage 29 checked errors reuse the same structured-exit and finalizer regions.
They do not invent a second cleanup model. Decision 0119 Slices 1 and 2 now
implement the grammar, semantic obligations, and executable checked-error
routing through those regions.

A checked-error exit identifies crossed regions exactly as return and loop
exits do. Its pending payload belongs in a typed synthetic local acquired
before ordinary cleanup; crossed regions run before propagation continues. A
matching catch consumes that pending route and stops propagation. Decision 0123
supersedes this record's former prohibition on replacement: an Error escaping a
finalizer replaces the pending normal completion, return, `when` result, loop
transfer, or earlier Error. Same-try sibling catches do not catch that
replacement; finalizer-local and outer catches may. Fatal panic remains a
separate abort-only edge and bypasses every region. Checked errors are neither
panic nor ordinary return values.

## MIR And Structured Exit Regions

`given`, `when`, `do ... while`, and `finally` lower once into explicit validated
CFG. Each source finalizer owns one backend-neutral region with one body, a
stack-local exit discriminator, and explicit continuations. A structured exit
first acquires any outgoing value, drops branch/body locals, enters each crossed
region exactly once from inner to outer, drops `given` locals, and resumes its
original destination. Same-loop `continue` does not cross the loop's region.

Validation plans preserve source control-flow identity and prove entry,
discriminator selection, value acquisition, continuation routing, and lexical
nesting. Backends execute ordinary blocks and branches; there is no runtime
finalizer object, heap cleanup stack, per-iteration registration, or unwind path.
`StructuredExitKind::CheckedError` is Stage 29's implemented extension point for
executable checked-error MIR.

## PHP Compatibility

The PHP backend preserves setup frequency, source-order short-circuiting,
condition skipping, result typing established by Doria, and post-tested loop
behavior. Backend-private closures may materialize a `when` expression, but they
do not define Doria return or ownership semantics. PHP truthiness is not used.
PHP emits a host `try`/`finally` only after Doria has established the legal
control-flow and ownership model. PHP `exit(101)` bypasses host `finally`, which
preserves Doria's fatal-panic rule; checked errors will use orderly structured
routing rather than panic or ordinary returns.

## Performance Impact

These constructs lower to direct CFG. There is no runtime `given`, `when`, loop,
or finalizer object and no required heap allocation. Loop scratch and finalizer
discriminators remain function-entry storage. Same-loop `continue` pays no
loop-finalizer cost; other exits cost one direct region traversal per crossed
finalizer, so nesting is O(crossed regions). Opt-in structural reports add
finalizer, structured-exit, finalized-return/break/continue, and maximum nesting
facts while MIR is already being materialized. Controlled timing remains
**Pending Available Runner** and does not block development.

## Implementation Slices

Stage 28a Slice 1 implements this decision's authority, executable `when`,
`given` on `if`/`when`/`while`, base `do ... while`, finalizer grammar and
preservation, backend parity, and tooling synchronization.

Slice 2 implements shared finalizer regions and routes normal completion,
`return`, `break`, `continue`, and `when` yields through them. The same region
model now also executes checked-error crossings under Decision 0119. Stage 28a
and all Stage 29 slices are complete; the pre-Stage-30 closure grammar slice is
next.

## PR #134 Closure Audit

| Finding | Current Fix | Regression Test | Semantic Analysis | Ownership Analysis | PHP | Native Paths | Remaining Risk |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `given ... while` backedges | Initial entry, body fallthrough, and `continue` target the shared predicate header; predicate failure exits before the condition. | `shared_validator_rejects_malformed_stage28a_control_flow_plans` rejects a `continue` that skips predicate reevaluation. | `stage28a_control_flow_analysis_honors_given_gates_and_enclosing_loops` preserves the predicate-false exit. | Loop backedges merge through the same shared CFG and flow state. | `php_backend_executes_stage28a_control_flow` observes the predicate before and after the body. | Durable `main_given_while` and `main_given_while_finally` fixtures run through the interpreter, Cranelift, and LLVM. | Future checked-error edges must enter the same predicate/finalizer regions rather than adding a second loop model. |
| PHP `given ... when` gate caching | PHP materializes the gate once before selecting the head, `else when`, or `else` branch. | `php_backend_executes_stage28a_control_flow` observes one side-effecting gate evaluation. | Branch selection consumes one classified `given` predicate sequence. | Only the selected branch contributes ownership state. | The cached gate is reused across the complete chain. | The native paths consume the shared validated `given` and `when` CFG plans. | None for current exits; Stage 29 must not reevaluate the gate while routing errors. |
| Loop depth inside `when` | A `when` branch retains the nearest enclosing loop target while return-to-yield remains local to the `when`. | `stage28a_control_flow_analysis_honors_given_gates_and_enclosing_loops` accepts `break` and `continue` from branches. | Loop depth is captured before checking the `when` and restored afterward. | Branch flows preserve backedge and break states. | Generated branch control flow targets the enclosing host loop without redefining Doria semantics. | Shared MIR target identity drives interpreter, Cranelift, and LLVM branches. | Nested future error handlers must preserve the same lexical target identity. |
| `do ... while` exit state | The body is the only entry; only post-body false-condition and `break` exits reach code after the loop. | `do_while_ownership_exit_excludes_the_unexecuted_pre_body_state` and the malformed continue-target test reject a synthetic zero-iteration route. | Return and definite-state analysis use the post-tested CFG. | Move-state merging excludes a pre-body exit. | Generated PHP executes the body before testing the condition. | The validated MIR plan and durable `do ... while` fixtures are shared by all native paths. | None for current exits; future checked errors remain separate abortible paths. |

## Explicit Exclusions

This decision does not add `given` on `do`, finalizers on excluded constructs,
truthiness, block expressions, general `yield`, checked errors, closures,
namespaces, autoloading, or reflection.

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
- Stage 28a Slice 2 implements this exact finalizer model.
- Stage 29 Slice 2 reuses the same structured finalizer regions.
- Stage 31 namespace and compile-time autoload authority, Stage 33 Baton work,
  self-hosting, and the performance workstream gain no new semantics here.
- Decision 0132 uses these same structured loop exits for indexed sequence
  iteration: same-loop `continue` advances the ordinal exactly once, while
  `break`, return, and checked exits expose no later index and preserve cleanup.
