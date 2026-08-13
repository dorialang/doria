# 0009 Control-flow direction

Status: Accepted

## Decision

Doria should support familiar control flow while also exploring a Gherkin-inspired setup/condition/action style for stateful conditional and looping code.

This note records broad control-flow direction only. Decision 0020 later accepted
the `given { ... }` predicate-block direction and the `if` / `when` distinction.
Decision 0116 now settles and implements the complete Stage 28a grammar,
semantics, and finalizer model, and is authoritative where this early direction
note was open or illustrative.

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

`when` is an expression, requires a total `else`, and uses branch-local
`return expression;` to yield from the nearest `when`, as settled by decisions
0097 and 0116.

### given ... when

`given` establishes a precondition/setup scope. Variables declared in the `given` block are available to the attached control construct.

Non-normative sketch:

```doria
given {
    let writable $message = "say something";
    let $timeInterval = 50;
    let writable $nextTime = get_time() + $timeInterval;

    $count % 2 == 0;
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
given {
    let writable $message = "say something";
    let $timeInterval = 50;
    let writable $nextTime = get_time() + $timeInterval;

    $count % 2 == 0;
} while (get_time() > $nextTime): void {
    echo $message;
} finally {
    // cleanup
}
```

`given ... when` and `given ... while` are separate planned alternatives.

### if / else if / else / finally

Doria should support normal `if` / `else if` / `else` statement control flow. `if` does not return a value. Doria may also support a `finally` block attached to an `if` chain.

Use `else if` as the spelling for now.

### match

Doria should eventually support `match` as a pattern/value selection construct. The exact match grammar is open.

## Questions Settled By Later Decisions

Decision 0116 settles the finalizer trigger paths, activation, scope, transfer
rules, ownership order, nested order, and `do ... while` punctuation that this
record left open. It also settles `given` execution and MIR lowering. Decisions
0097 and 0116 settle `when` as an exhaustive expression with mandatory `else`.
Decision 0115 settles `match`: match selects one expression by pattern, while
`when` runs statement branches that yield a value. The spelling is `else if`;
`elseif` is not Doria syntax.

## Notes

Use Decision 0116, not this direction note alone, for current implementation.
Stage 28a Slice 2 implements executable `finally` through shared finalizer
regions and structured-exit routing.
