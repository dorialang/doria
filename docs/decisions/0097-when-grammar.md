# Decision 0097: The `when` value-returning control construct

Status: Accepted

## Context

`when` was accepted as design direction in records 0009 (control-flow direction) and 0020 (boolean operators and `given` predicates), but its grammar was deferred to "a later decision record" and 0009 left open questions — whether `when` is an expression or a statement, and how a branch produces its value. In the absence of that record the website control-flow guide and the `value-returning-when` playground example demonstrated a concrete syntax, and the end-to-end plan contradicted itself on where `when` lands (§4.4 said "Phase E"; the §13 Stage 36 entry scheduled it). This record closes the grammar.

The ruling: **`when` is the value-returning form of `if`.** It has exactly the structure of `given / if / else if / else / finally`; the single difference is that it always produces a value. This is non-negotiable and settles the open questions below.

## Decision

### `when` mirrors the `if` family exactly

A `when` construct has the same shape as the `if` family:

- an optional `given { ... }` prelude — scoped declarations, void setup statements, and `bool` predicates, per record 0020;
- `when (cond): T { ... }`;
- zero or more `else if (cond) { ... }`;
- an `else { ... }`;
- an optional `finally { ... }`.

Conditions are `bool`. Doria applies no truthiness, exactly as for `if`.

### The one difference: `when` always yields a value

- **A result type is declared on the head** — `when (cond): T`. `T` applies to the whole construct; every branch yields a `T`.
- **`else` is mandatory.** Because every evaluation must produce a value, a `when` with no total `else` is a compile error. (Contrast `if`, which permits no `else`.) This is `when`'s exhaustiveness rule and is what makes "always returns a value" true.
- **Branches yield with `return`.** Inside a `when` branch, `return <expr>;` yields that branch's value and completes the `when` — it does **not** return from the enclosing function. This is a block-scoped return: a `when` branch is an `if` block that produces a value. Every reachable path within a branch must yield.

### Position: `when` is an expression

`when` produces a value and appears wherever a value of its result type is expected — assignment right-hand side (`let $x = when ...`), `return` operand (`return when ...`), and call arguments. This resolves 0009's open "expression or statement" question: **`when` is an expression.** A `when` whose value is discarded in pure statement position is a "result discarded" lint, the same as any unused value — use `if` when no value is needed.

### `given ... when` routes a false predicate to `else`

The `given` prelude behaves exactly as with `if` (record 0020): scoped declarations, void setup actions, and `bool` predicates implicitly AND-ed in source order with the head condition. The single adaptation forced by "always yields a value": when a `given` predicate — or the head condition — is false, control falls to the next `else if` / `else`, never skipping the construct. A `when` can never be short-circuited away, because a value is owed. So `given ... if` skips its body on a false predicate; `given ... when` selects its `else`.

### `finally`

`finally` runs on completion for cleanup only, per plan §5: it may not `return`, `throw`, `break`, or `continue`. It therefore cannot supply or alter the `when`'s value — the value is fixed by the selected branch's `return` before `finally` runs.

## Alternatives considered

- **Last-expression-yields (Rust/Kotlin arm style).** Rejected: the ruling is that `when` works exactly like an `if` block, whose bodies are statement sequences, not trailing expressions. `return`-to-yield keeps the `if` and `when` forms structurally identical.
- **Optional `else` / `when` as a plain statement.** Rejected: a value-returning form with an unfilled path has no value to produce. Mandatory `else` is precisely what makes the construct total.
- **A distinct yield keyword (`yield` / `give`).** Rejected: a second exit spelling for what is structurally a return from a block. `return` already means "produce this and leave"; reusing it keeps one exit vocabulary and one mental model shared with `if`.

## Consequences

- `when` and `if` share one parser and analysis shape; the checker adds only `when`'s value obligations — the declared result type, the mandatory `else`, and every-path-yields — reusing the same forward-dataflow framework that definite-initialization and narrowing use.
- The website control-flow guide and the `value-returning-when` playground example are now anchored to this record rather than inventing syntax. Both already use the decided form (typed `when`, `return` inside branches, expression position); reconcile any divergence to this record.
- `given` / `finally` / `do ... while ... finally` from 0009's control-flow family share this prelude-and-cleanup machinery; their exact lowering remains their own implementation work, but the `when` surface is fixed here.

## Affected components

Lexer and parser (accept `when` / `given` / `finally` ahead of semantics, per §0 two-clocks), semantic analysis (value-obligation checking on the shared dataflow framework), MIR lowering, the interpreter / Cranelift / LLVM backends, `SPEC.md` control-flow section, the end-to-end plan (§4.4 and the §13 stage roadmap), and the website control-flow guide and playground example.

## Invalidated elsewhere

- SPEC's "`when` grammar is undecided / not specified" wording — the grammar is specified here.
- Record 0009's "whether `when` is an expression, a statement, or both remains open" — resolved: `when` is an expression.
- Plan §4.4's assertion that `when`'s grammar decision is authored "in Phase E" **and** the §13 Stage 36 roadmap entry — these disagreed on `when`'s stage. Reconciled: `when` is basic control flow, so it is re-slotted from Stage 36 (property hooks) to **Stage 28a — control-flow completion**, right after `match`, together with `given`, control-flow `finally`, and `do … while … finally` (the rest of record 0009's accepted-but-unimplemented control-flow family). Stage 36 keeps only property hooks.
- Any assertion that `when` / `given` / `finally` grammar remains undecided.
