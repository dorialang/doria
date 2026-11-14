# Decision 0143: For Initializer Local Declarations

- **Status:** Accepted
- **Date:** 2026-11-13
- **Owners:** Doria language and compiler design
- **Scope:** The declaration forms a traditional `for` loop accepts in its initializer

## Context

A local declaration statement has four forms: `let $name = value;`, `let writable $name = value;`, `Type $name = value;`, and `writable Type $name = value;`, each with Decision 0111's grouped spelling. The `for` initializer, added in Stage 9 under Decision 0034, accepted only the two `let` forms and an assignment. Decision 0034 showed `for (let writable $i = 0; ...)` and said nothing about typed declarations, and no record rejected them. The parser reported `for (writable int $row = 0; ...)` as "expected expression".

Andrew reported the gap on 2026-11-13 from an editor session: the typed form is valid Doria and must be accepted.

## Decision

A `for` initializer accepts every local declaration form that a statement accepts, with the same grammar, the same rules, and the same diagnostics. It also still accepts an assignment, or nothing.

```doria
for (writable int $row = 0; $row < $height; $row++) {
}

for (let writable $index = 0; $index < 10; $index++) {
}

for (writable int $low, $high = 0; $low < 10; $low++) {
}
```

The declared bindings are loop-local, as before: they are visible in the condition, the increment, and the body, and not after the loop. A binding declared without `writable` is readonly, so an increment of it is the ordinary readonly-write error.

Neither spelling is preferred. `let` infers the type; the typed form states it.

## Implementation Contract

One parser routine owns each local declaration form, and the statement parser and the `for` initializer both call it. A form added to local declarations later reaches the `for` initializer without a second edit. The syntax tree already carried an optional type on the initializer's declaration, so semantic analysis, ownership, lowering, MIR, the native backends, and PHP output are unchanged.

## Explicit Non-Goals

- Several declarations or several assignments in one initializer, as with C's comma operator.
- A declaration in the condition or the increment.
- Any change to `foreach` bindings, which Decision 0133 owns.

## Consequences

- `SPEC.md` shows the typed form beside the `let` form.
- Decision 0034's examples stay as written; this record extends its initializer grammar.
- The parser message for an initializer that is neither a declaration nor an assignment now reads "expected a local declaration or an assignment in for initializer".

## Invalidated Elsewhere

- The language server must move its compiler pin to a revision containing this change. Until it does, editors report the typed form as three parser errors.
- The website guide's `for` loop page shows only the `let` form and should show both. This task does not modify the website repository.
