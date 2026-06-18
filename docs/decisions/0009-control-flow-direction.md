# 0009 Control-flow direction

Status: Accepted

## Decision

Doria should support familiar control flow while also exploring a Gherkin-inspired setup/condition/action style for stateful conditional and looping code.

This note records language direction only. It does not specify final grammar, lowering, borrow-checking behavior, or implementation requirements for the current compiler slice.

Planned control-flow families:

- `foreach`
- `while`
- `do ... while ... finally`
- `given ... when ... finally`
- `given ... while ... finally`
- `if` / `else if` / `else` / `finally`
- `when`
- `match`

## Intent

### foreach

`foreach` is the standard iteration construct for walking collection values.

### while

`while` is the standard looping construct for checking a condition before each loop iteration.

### do ... while ... finally

`do ... while ... finally` is a looping form where the body runs before the condition check. The `finally` block runs after the loop completes according to the eventual `finally` semantics.

### when

`when` is a value-returning conditional block.

Non-normative sketch:

```doria
when ($condition): int {
    return 1;
}
```

If a return type is declared, all successful paths should return that type. Whether `when` is an expression, a statement, or both remains open.

### given ... when

`given` establishes a precondition/setup scope. Variables declared in the `given` block are available to the following `when` block.

Non-normative sketch:

```doria
given ($count % 2 == 0) {
    let writable $message = "say something";
    let $timeInterval = 50;
    let writable $nextTime = get_time() + $timeInterval;
} when (get_time() > $nextTime): void {
    echo $message;
} finally {
    // cleanup
}
```

### given ... while

`given` can also feed a `while` block. In that form, it becomes a looping construct with setup state.

Non-normative sketch:

```doria
given ($count % 2 == 0) {
    let writable $message = "say something";
    let $timeInterval = 50;
    let writable $nextTime = get_time() + $timeInterval;
} while (get_time() > $nextTime): void {
    echo $message;
} finally {
    // cleanup
}
```

`given ... when` and `given ... while` are separate planned alternatives.

### if / else if / else / finally

Doria should support normal `if` / `else if` / `else` control flow. Doria may also support a `finally` block attached to an `if` chain.

Use `else if` as the spelling for now.

### match

Doria should eventually support `match` as a pattern/value selection construct. The exact match grammar is open.

## Open questions

- Does `finally` run after `return`?
- Does `finally` run after `break` or `continue`?
- Does `finally` run if the `given` precondition is false?
- Does `finally` run if the `when` condition is false?
- Does `finally` run after zero `while` iterations?
- Is `when` an expression, a statement, or both?
- Does `when` require an `else` or default branch when used as an expression?
- What is the exact scope relationship between `given`, `when`/`while`, and `finally`?
- Are variables declared in `given` mutable across `while` iterations?
- Does `finally` have access to variables declared in `given`?
- Does `finally` have access to variables declared inside `when`/`while`?
- How does this interact with borrow checking and writable values?
- How does this lower into Doria IR?
- How does `match` differ from `when`?
- Do we spell else-if as `else if` or `elseif`?

## Notes

Do not implement lexer, parser, semantic, Doria IR, or backend behavior from this note alone. Each construct needs a specific grammar and semantics pass before compiler work begins.
