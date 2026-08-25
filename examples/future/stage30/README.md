# Stage 30 closure examples

These snippets are accepted Stage 30 target-state documentation. Their closure
grammar, semantic types, captures, ownership, lifetime, and escape behavior are
checked. Stage 30f-supported closure forms lower through closure-aware HIR/MIR
and execute through debug, both native profiles, and the PHP compatibility
backend where the surrounding value surface is supported. This directory also
documents later Stage 30 surfaces and is not an executable manifest.

[Decision 0121](../../../docs/decisions/0121-closure-function-types-capture-semantics-and-execution-model.md)
accepts the complete closure model and elaborates Decision 0120's explicit
capture lists. **Stages 30a through 30h and Stage 30 are complete**: structural function
types have semantic identity; captures, closure bodies, inferred modes/effects,
callable-value calls, capture acquisition, Move state, leases, and escape are
checked; explicit closure/callable-call HIR, structural function MIR, indirect
calls, environment cleanup, the debug-interpreter oracle, and Cranelift/LLVM
native execution and explicit PHP compatibility lowering are implemented.
Stage 30g List algorithms are implemented across the debug interpreter, both
native profiles, and PHP compatibility for supported values. Stage 30h closes
the accepted storage and backend routes, including function values in `mixed`.
E0641 is historical and reserved. Stage 31 is next.

The inventory covers:

- No-capture arrow and block closures.
- Explicit readonly arrow capture.
- The same readonly capture contract with a block body.
- Exclusive writable capture.
- Ownership transfer into a returned closure.
- Accepted `List<T>` closure algorithms with captured and no-capture callbacks.

Keeping these examples in this document is deliberate. The accepted syntax has
checked source fixtures; Stage 30d has a durable debug-interpreter manifest and
Stages 30e through 30g have separate native and PHP parity manifests. Complete
target-state examples must not be registered as runnable programs before their
owning slices land.

## No capture

```doria
function main(): void
{
    let $double = fn(int $value) => $value * 2;

    let $positive = function (int $value): bool {
        return $value > 0;
    };
}
```

## Readonly arrow capture

```doria
function main(): void
{
    let $minimum = 70;

    let $passes = fn(int $score) with ($minimum) =>
        $score >= $minimum;
}
```

## Readonly block capture

```doria
function main(): void
{
    let $minimum = 70;

    let $passes = function (int $score): bool with ($minimum) {
        return $score >= $minimum;
    };
}
```

## Writable capture

```doria
function main(): void
{
    let writable $count = 0;

    let $next = function (): int with (writable $count) {
        $count += 1;
        return $count;
    };
}
```

## Taking capture

```doria
class Payload
{
    function __construct(string $value)
    {
    }
}

function makeReader(
    take Payload $payload,
): function(): string
{
    return function (): string with (take $payload) {
        return $payload->value;
    };
}
```

## Collection pipeline

```doria
function main(): void
{
    List<int> $scores = [65, 72, 88];

    let $minimum = 70;
    let $bonus = 5;

    let $passing = $scores->filter(
        fn(int $score) with ($minimum) =>
            $score >= $minimum
    );

    let $adjusted = $passing->map(
        fn(int $score) with ($bonus) =>
            $score + $bonus
    );

    foreach ($adjusted as int $score) {
        echo "{$score}\n";
    }

    List<string> $labels = ["atlas", "birch", "cedar"];

    let $filteredLabels = $labels->filter(
        fn(string $label) => $label != "birch"
    );

    foreach ($filteredLabels as string $label) {
        echo "{$label}\n";
    }
}
```

## Future diagnostic acceptance

An arrow using an omitted outer binding:

```doria
let $minimum = 70;

let $passes = fn(int $score) =>
    $score >= $minimum;
```

and an anonymous function doing the same:

```doria
let $minimum = 70;

let $passes = function (int $score): bool {
    return $score >= $minimum;
};
```

must report **Closure Must Capture `$minimum`** and direct the author to add
`with ($minimum)`. A structured edit inserts the clause between an arrow's
parameter list and `=>`, or before an anonymous function's block. An existing
list is extended only when the required mode is unambiguous, formatting and
source order are preserved, and no duplicate is introduced.

A taking capture uses the ordinary moved-value diagnostic afterward:

```doria
let $payload = new Payload("ready");

let $reader = function (): string with (take $payload) {
    return $payload->value;
};

echo $payload->value;
```

A readonly capture is borrow-bound and may not escape its owner. The future
diagnostic should recommend a taking capture when ownership transfer is the
correct solution. Decision 0121 also requires explicit `with ($this)` or
`with (writable $this)`, rejects taking `$this`, infers closure effects from the
body, and adds no closure-expression effect annotation. Stages 30b and 30c check
those semantics plus capture acquisition, lifetime, and escape. Stage 30d
executes the supported closure surface through the debug interpreter; Stage 30e
executes it through Cranelift and LLVM; Stage 30f executes the PHP-compatible
surface through explicit generated carriers and environments. Stage 30g List
algorithms remain next.
