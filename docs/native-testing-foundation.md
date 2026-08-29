# Native Testing Foundation

> **Documentation role:** implementation-facing reference for Decision 0129.
> Decision 0129 owns the accepted semantics. The end-to-end plan owns the global
> roadmap once mechanically synchronized. Until that synchronization lands, this
> accepted amendment inserts the Native Testing Foundation before Stage 34.

## Status

```text
Decision 0129 - Accepted
Pre-Stage-34 Native Testing Foundation - Next
Stage 34 Single Class Inheritance - Blocked Until This Foundation Completes
```

Stage 33 and Phase F remain complete. Baton already owns test discovery,
development graph activation, dispatcher compilation, fresh-process isolation,
and suite orchestration. This foundation adds the missing first-party test
language and assertion system.

## Canonical Authoring Experience

```doria
use Doria\Std\Test\{
    describe,
    it,
    expect,
};

describe("Shopping Cart", function (): void {
    describe("when it is empty", function (): void {
        it("has a total of zero", function (): void {
            let $cart = new ShoppingCart();

            expect($cart->total)->toEqual(0);
            expect($cart->items)->toBeEmpty();
        });
    });

    it("contains an added item", function (): void {
        let writable $cart = new ShoppingCart();
        let $item = new Product("Book", 20);

        $cart->add($item);

        expect($cart->items)->toContain($item);
        expect($cart->total)->toEqual(20);
        expect($cart->items)->not->toContain(new Product("Pen", 2));
    });
});
```

The lower-level form remains available:

```doria
#[Test]
function cartStartsEmpty(): void
{
    let $cart = new ShoppingCart();
    expect($cart->items)->toBeEmpty();
}
```

The behavioral DSL and `#[Test]` produce one compiler-owned test metadata model.
Baton never parses Doria source.

## Public Module

```doria
Doria\Std\Test
```

Initial public identities:

```text
describe
test
it
expect
fail
AssertionError
```

The module is available only in active development and generated-development
source. It does not enter normal package release graphs.

## Behavioral DSL

### `describe`

```doria
describe(string $description, function(): void $body);
```

The displayed signature is conceptual. The compiler recognizes the canonical
identity as a test declaration form.

The description is const-evaluable. The body may contain nested `describe`,
`it`, and `test` declarations only. It is not executed at runtime and does not
create a suite object or registration call.

### `it` And `test`

```doria
it(string $description, function(): void $body);
test(string $description, function(): void $body);
```

The body compiles into a deterministic test callable. `it` and `test` are
semantic aliases; the authored spelling remains available to tooling.

Full display name:

```text
<describe path> > <test description>
```

Example:

```text
Shopping Cart > when it is empty > has a total of zero
```

Full names must be unique within one package.

## Expectation Surface

### General

```doria
expect($actual)->toEqual($expected);
expect($actual)->not->toEqual($unexpected);
expect($value)->toBeNull();
expect($value)->not->toBeNull();
expect($value)->toBeTrue();
expect($value)->toBeFalse();
fail("message");
```

### Ordered Values

```doria
expect($value)->toBeGreaterThan($minimum);
expect($value)->toBeGreaterThanOrEqual($minimum);
expect($value)->toBeLessThan($maximum);
expect($value)->toBeLessThanOrEqual($maximum);
```

### Strings

```doria
expect($value)->toContain("fragment");
expect($value)->toStartWith("prefix");
expect($value)->toEndWith("suffix");
expect($value)->toBeEmpty();
```

### Collections

```doria
expect($items)->toBeEmpty();
expect($items)->toHaveCount(3);
expect($items)->toContain($expected);
expect($dictionary)->toHaveKey($key);
expect($dictionary)->toHaveValue($value);
```

Only capabilities already implemented by the concrete type are available.

### Checked Errors

```doria
expect(fn() => performOperation())->toThrow();
expect(fn() => performOperation())->not->toThrow();
```

Typed inspection:

```doria
expect(fn() => parseConfiguration("invalid"))
    ->toThrow(
        function (ParseError $error): void {
            expect($error->message)->toContain("configuration");
        },
    );
```

The inspector parameter supplies the expected Error type. No general runtime
type token or reflection API is introduced.

## Expectation Ownership

`expect($actual)` evaluates `$actual` once and borrows it readonly for the chain.
It does not move a class, collection, `Bytes`, function value, or Move payload
enum merely to inspect it.

The expectation value is ephemeral and non-escaping. It cannot be stored,
returned, captured, boxed into `mixed`, or inserted into a collection.

`not` is a property because it represents expectation state. It is not a
zero-argument noun method.

Successful assertions have no mandatory heap allocation. Expected/actual
formatting is deferred until failure.

## Assertion Outcome

A failed matcher raises:

```doria
Doria\Std\Test\AssertionError
```

It uses the test-only `TestAssertion` effect. Test source and helpers do not
write `throws AssertionError`, but the Error remains present in HIR, MIR, checked
transport, cleanup, and runtime outcomes.

Assertion failure is not panic.

Baton reports these categories separately:

```text
Passed
Assertion Failed
Unexpected Checked Error
Fatal Panic
Abnormal Process Failure
```

The structured assertion outcome carries matcher, expected/actual presentation,
bounded difference, optional message, and source origin. Baton must not classify
assertions by parsing stderr prose.

## Compiler Metadata

Metadata schema versions 1 and 2 remain exact.

Schema version 3 adds:

```text
test suites
behavioral path segments
authored it/test spelling
display names
canonical test identities
compiler-generated callable identities
source/package locations
#[Test] versus behavioral origin
```

Processor request and response protocols remain version 1.

## Implementation Sequence

### Slice 1 — Behavioral Test DSL And Unified Compiler Metadata

Deliver:

```text
compiler-known Doria\Std\Test identities
describe / it / test declaration semantics
nested suite validation
const descriptions
compiler-generated test callables
unified #[Test] metadata
metadata schema 3
Baton discovery migration to schema 3
LSP compiler facts
```

Acceptance:

```text
no runtime suite registry
no source parsing in Baton
no parser errors for accepted DSL
stable full test identity
#[Test] remains green
```

### Slice 2 — Fluent Expectation Kernel And Assertion Semantics

Deliver:

```text
expect / fail
ephemeral Expectation<T>
not property
equality/null/bool assertions
ordered/numeric assertions
string assertions
AssertionError
TestAssertion effect
structured failure outcomes
interpreter/Cranelift/LLVM/PHP parity
```

Acceptance:

```text
actual evaluated once
Move actual remains owned
no required throws boilerplate
ordinary cleanup on assertion failure
panic remains separate
no mandatory success-path allocation
```

### Slice 3 — Collections, Errors, Baton Reporting, And Tooling Closure

Deliver:

```text
collection and dictionary expectations
toThrow / not->toThrow
typed Error inspectors
bounded differences
hierarchical Baton reporting
assertion outcome classification
behavioral-name filtering
LSP completion/hover/navigation
docs/examples/installed-toolchain closure
```

Acceptance:

```text
Stage 33 remains green
all native/PHP profiles agree
Baton never parses source
editor tooling consumes compiler metadata
Native Testing Foundation marked complete
Stage 34 becomes next
```

## Deliberate Follow-Up Surface

These remain deferred:

```text
beforeEach / afterEach
datasets
custom matchers
snapshots
property-based testing
mocks and test doubles
parallel tests
retries
tags and ignored tests
random ordering
editor test code lenses
```

Hooks and fixture sharing require an ownership-aware test context. Custom
matchers become substantially cleaner after Stage 35 lands user interfaces,
traits, `Equatable`, `Displayable`, and `Cloneable` conformance.

They are deferred, not permanently rejected.

## Invalidated Elsewhere

- `docs/doria-end-to-end-plan.md` and `docs/notes/current-pipeline.md` must place
  this foundation before Stage 34 when the first implementation slice begins.
- `docs/stdlib-reference.md` must gain the `Doria\Std\Test` surface.
- `docs/self-hosting.md` must distinguish the existing runner from the complete
  native testing story.
- Decision 0128 remains the Baton orchestration authority but is no longer the
  complete testing API authority.
- Baton, language-server, editor, and website documentation require staged
  synchronization as the three slices land.
