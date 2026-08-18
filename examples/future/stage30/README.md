# Stage 30 closure examples

These snippets are accepted Stage 30 target-state documentation. Their closure
grammar is parseable and mirrored in
`crates/doriac/tests/fixtures/accepted_syntax/closures.doria`. They are parser
fixtures, not executable examples or native parity fixtures: semantic checking
stops at the catalogued E0641 boundary until Stage 30 implements closure
semantics.

The inventory covers:

- No-capture arrow and block closures.
- Explicit readonly arrow capture.
- The same readonly capture contract with a block body.
- Exclusive writable capture.
- Ownership transfer into a returned closure.
- Accepted `List<T>` closure algorithms with captured and no-capture callbacks.

Keeping these examples in this document is deliberate. The accepted syntax now
has checked source fixtures, but these complete Stage 30 examples must not be
registered as runnable programs until their capture, typing, ownership, and
execution semantics land.

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
function main(): void throws Doria\Std\Io\IoError
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
correct solution. No concrete escape fixture is fixed here because the exact
callable-effect annotation and `$this` capture rules remain bounded Stage 30
questions.
