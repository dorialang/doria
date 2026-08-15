# Stage 30 closure examples

These files are accepted Stage 30 target-state documentation. They are not
registered as native parity fixtures and do not claim current execution support.
Stage 30 remains blocked until Stage 29 completes.

The inventory covers:

- `no_capture.doria`: arrow and block closures with no environment.
- `readonly_arrow_capture.doria`: explicit readonly arrow capture.
- `readonly_block_capture.doria`: the same contract with a block body.
- `writable_capture.doria`: exclusive writable capture.
- `taking_capture.doria`: ownership transfer into a returned closure.
- `collection_pipeline.doria`: accepted `List<T>` closure algorithms with both
  captured and no-capture callbacks.

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
