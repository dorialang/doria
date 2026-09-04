# Decision 0129: Native Testing Foundation, Behavioral DSL, Fluent Expectations, And Assertion Outcomes

> **Stage 35 amendment:** Decision 0134 supplies the public Equatable,
> Displayable, Cloneable, Error-subinterface, and interface-erasure contracts
> consumed by later testing fixtures without changing schemas 1 through 3.

- **Status:** Accepted
- **Accepted:** 2026-08-29
- **Date:** 2026-08-29
- **Implementation Status:** Implemented By Native Testing Foundation Slices 1 Through 3
- **Amends:** Decisions 0096, 0113, 0121, 0123, 0125, and 0128
- **Preserves:** Stage 33 and Phase F completion; compiler-owned `#[Test]` metadata; Baton process isolation and development-graph orchestration; Doria ownership, checked-error, namespace, package, and backend laws; and Stage 34 inheritance authority

## Context

Stage 33 delivered a practical test runner. Baton discovers compiler-owned
`#[Test]` metadata, activates development graphs, builds a dispatcher, isolates
tests in fresh processes, and reports checked Errors and panics. That is a test
execution convention, not yet a native testing suite.

Doria still lacks a first-party assertion API, assertion-failure semantics,
typed Error expectations, bounded value differences, collection assertions, and
a behavioral declaration style comparable to PestPHP and Jest. A fluent
`expect(...)` call alone would not make the experience Pest-like; the defining
shape also requires `describe(...)`, `it(...)`, and `test(...)` as
compiler-understood behavioral declarations.

The gap is load-bearing for ordinary users, the Doria-native Baton transition,
and eventual compiler self-hosting. A language cannot claim a usable native
testing story when users must hand-roll equality, null, collection, and Error
checks for every package.

## Scheduling Decision

Insert one mandatory foundation before Stage 34:

```text
Stage 33 - Complete
Phase F - Complete
Native Testing Foundation Slice 1 - Complete
Native Testing Foundation Slice 2 - Complete
Native Testing Foundation Slice 3 - Complete
Native Testing Foundation - Complete
Stage 34 Single Class Inheritance - Complete
Stage 35 Interfaces And Traits - Authority Accepted; Slice 1 Next
Pre-Stage-45 Doria-Native Baton Transition - Scheduled
```

This foundation has three slices:

```text
Slice 1 - Behavioral Test DSL And Unified Compiler Metadata
Slice 2 - Fluent Expectation Kernel And Assertion Semantics
Slice 3 - Collection/Error Expectations, Baton Reporting, And Tooling Closure
```

Stage 33 remains complete. This decision does not reopen Phase F. It extends the
language and standard testing surface consumed by the already implemented Baton
runner.

## Product Identity

The first-party suite ships with the toolchain under:

```doria
Doria\Std\Test
```

No package installation is required.

The canonical import is:

```doria
use Doria\Std\Test\{
    describe,
    it,
    test,
    expect,
    fail,
};
```

`Doria\Std\Test` is available only to active development and
generated-development source. Main and release source cannot silently acquire a
test-only runtime or assertion effect.

The primary authored experience is a behavioral DSL plus fluent expectations.
All `expect(...)` examples in this record show delivered Slice 2 surface. Slice
1 executes the surrounding `describe`/`it`/`test` declarations and Slice 2
executes the expectation calls:

```doria
describe("User", function (): void {
    it("can be created", function (): void {
        let $user = new User("Andrew");

        expect($user->name)->toEqual("Andrew");
        expect($user->email)->toBeNull();
    });
});
```

The lower-level metadata form remains valid:

```doria
#[Test]
function userCanBeCreated(): void
{
}
```

Neither style is deprecated. `#[Test]` is the low-level function-oriented form;
`describe`/`it`/`test` is the recommended behavioral authoring form.

## Behavioral Declaration DSL

### Call-Shaped, Compiler-Owned Declarations

`describe`, `it`, and `test` use ordinary Doria call-shaped syntax, but resolve
to compiler-known test declarations in development source. They are not runtime
registration functions and do not become top-level executable statements.

The compiler recognizes them only after canonical name resolution to the exact
`Doria\Std\Test` identities. A user function with the same short name is not a
test declaration.

No runtime registry, reflection scan, static initializer, or source parser is
introduced.

### `describe`

Accepted shape:

```doria
describe(const-evaluable-string, function (): void {
    // nested describe / it / test declarations only
});
```

Rules:

- the description is a compile-time string;
- the body has zero parameters and `void` return;
- the body is a declaration container, not an executable suite callback;
- it may contain nested `describe`, `it`, and `test` declarations;
- it introduces no hidden runtime object or mutable suite context;
- it is absent from MIR and every runtime backend;
- nesting depth is bounded by ordinary compiler recursion limits and guarded
  against pathological input.

Ordinary runtime statements directly inside a `describe` body are rejected with
a precise diagnostic. Test code belongs inside `it` or `test`.

### `it` And `test`

Accepted shapes:

```doria
it("can be created", function (): void {
    // test body
});
```

```doria
test("user creation", function (): void {
    // test body
});
```

```doria
it(
    "adds two numbers",
    fn() => expect(add(20, 22))->toEqual(42),
);
```

Rules:

- `it` and `test` are semantic aliases;
- the authored spelling is preserved for tooling and source presentation;
- the description is a compile-time string;
- the body has zero parameters and returns `void`;
- the body is compiled as a deterministic compiler-generated test callable;
- the body may use ordinary Doria code, checked Errors, ambient I/O, closures,
  classes, collections, and package-internal declarations under existing laws;
- the declaration itself performs no runtime registration;
- duplicate full behavioral test names in one package are rejected.

A behavioral test's display name is the authored description path joined by:

```text
 > 
```

Example:

```text
Shopping Cart > when it is empty > has a total of zero
```

Its semantic identity also includes package and source identity so separate
packages cannot collide.

### `#[Test]` Interoperability

Compiler-owned `#[Test]` functions and behavioral declarations lower into one
unified test metadata model. Baton never merges two independently inferred test
systems.

A `#[Test]` function uses its canonical function name as its display name unless
a future accepted metadata extension says otherwise.

## Compiler Metadata

Metadata schemas 1 and 2 remain exact.

Add strict metadata schema version 3. It contains every schema-2 field plus
compiler-owned test suite and test declaration facts.

At minimum, schema 3 records:

```text
test suites
behavioral path segments
authored it/test spelling
display name
canonical test identity
package identity
source identity
source location
compiler-generated callable identity
low-level #[Test] versus behavioral origin
```

Baton consumes schema 3 for test discovery. It does not parse Doria source,
closure bodies, imports, or descriptions.

Unknown metadata schema versions remain rejected. Processor protocol version 1
is unchanged.

## Fluent Expectation Model

The canonical entry is:

```doria
expect($actual)
```

Conceptually it produces:

```text
Expectation<T>
```

`Expectation<T>` is a compiler-known, ephemeral, non-escaping expectation view.
It is not a user-constructible runtime class and is not runtime reflection.

Rules:

- the actual expression is evaluated exactly once;
- the actual value is borrowed readonly;
- Move values are not consumed merely to assert on them;
- the borrow ends at the end of the expectation chain;
- an expectation cannot be stored, returned, captured, boxed into `mixed`, or
  inserted into a collection;
- successful assertions require no mandatory heap allocation;
- expected operands are evaluated exactly once in source order;
- expected values are borrowed readonly where ownership permits;
- formatting and differences are computed only on failure.

Negation is a property, not a vague zero-argument method:

```doria
expect($roles)->not->toContain("guest");
```

The `not` projection returns the same ephemeral expectation with one inverted
matcher polarity. Repeated `not` is rejected rather than creating ambiguous
chains.

## Initial Expectation Surface

### General Values

```doria
expect($actual)->toEqual($expected);
expect($actual)->not->toEqual($unexpected);

expect($value)->toBeNull();
expect($value)->not->toBeNull();

expect($value)->toBeTrue();
expect($value)->toBeFalse();

fail("This code path should not be reached.");
```

`toEqual` uses Doria's existing typed equality law. It does not introduce test-only
coercion, structural reflection, hidden cloning, or a fallback string comparison.

### Ordered And Numeric Values

```doria
expect($value)->toBeGreaterThan($minimum);
expect($value)->toBeGreaterThanOrEqual($minimum);
expect($value)->toBeLessThan($maximum);
expect($value)->toBeLessThanOrEqual($maximum);
```

These use the existing supported ordering semantics. Tests do not weaken Doria's
numeric conversion rules.

### Strings

```doria
expect($message)->toContain("failed");
expect($message)->toStartWith("Error:");
expect($message)->toEndWith(".");
expect($message)->toBeEmpty();
```

String matching is exact and case-sensitive in the initial surface.

### Collections

```doria
expect($items)->toBeEmpty();
expect($items)->toHaveCount(3);
expect($items)->toContain($expected);

expect($dictionary)->toHaveKey("user");
expect($dictionary)->toHaveValue($user);
```

The compiler exposes only assertions supported by the concrete collection and
element capabilities already implemented. It does not invent equality, hashing,
ordering, cloning, or iteration for a type that lacks it.

The exact initial matrix is:

| Actual | `toBeEmpty` | `toHaveCount` | `toContain` | `toHaveKey` | `toHaveValue` |
| --- | --- | --- | --- | --- | --- |
| `T[]` | yes | yes | yes | no | no |
| `Bytes` | yes | yes | no | no | no |
| `List<T>` | yes | yes | yes | no | no |
| `Dictionary<K, V>` / `SortedDictionary<K, V>` | yes | yes | no | yes | yes |
| `Set<T>` / `SortedSet<T>` | yes | yes | yes | no | no |
| `PriorityQueue<T>` | yes | yes | yes | no | no |
| `Deque<T>` | yes | yes | yes | no | no |

String `toContain` and `toBeEmpty` retain the `StringContains` and
`StringEmpty` runtime fact names. Collection overloads use the distinct
`CollectionContains` and `CollectionEmpty` facts. Receiver-domain selection is
owned by one compiler matcher table; backends and tooling consume the selected
fact rather than repeating this matrix. `Bytes` gains no test-only membership
operation, dictionary `toContain` remains an ambiguous-use diagnostic, and a
`PriorityQueue` presentation never exposes private heap order.

## Checked Error Expectations

A function-value expectation may inspect checked Error behavior:

```doria
expect(fn() => performOperation())->toThrow();
expect(fn() => performOperation())->not->toThrow();
```

A typed inspector supplies the expected concrete Error type without adding type
tokens or runtime reflection:

```doria
expect(fn() => parseConfiguration("invalid"))
    ->toThrow(
        function (ParseError $error): void {
            expect($error->message)->toContain("configuration");
        },
    );
```

Rules:

- the subject function value is invoked exactly once;
- a readonly subject uses an ordinary readonly invocation and remains usable;
- a writable subject requires ordinary writable access and remains usable;
- a once subject uses ordinary once-call consumption and cannot be used again;
- it may have any concrete return type and checked-effect set accepted by the
  compiler-known matcher;
- a normal result is destroyed normally when `toThrow` expected an Error;
- a checked Error satisfies untyped `toThrow`;
- a typed inspector runs only for the exact compatible Error type;
- the wrong Error type is an assertion failure;
- `not->toThrow` fails on any checked Error;
- fatal panic is never treated as a thrown checked Error;
- panic remains a panic and Baton reports it separately;
- inspector assertions use ordinary assertion propagation.

The optional inspector is evaluated once before subject invocation. It is a
function value taking exactly one readonly `Error` or exact concrete
Error-conforming class and returning `void`. Its complete required, ambient,
and `TestAssertion` effects propagate normally. A concrete inspector compares
the runtime Error descriptor by exact identity; an erased `Error` inspector
accepts every checked Error. Negated `toThrow` accepts no inspector. Fatal panic
is never routed through the checked-Error branch.

Normal Move results and caught Errors are each destroyed exactly once on their
ordinary structured paths. Presentations and differences are constructed only
after matcher failure, are bounded to 4 KiB, and collection previews show at
most eight public-order entries. No user `Displayable`, reflection registry,
runtime suite registry, type token, metadata schema 4, or DORIAO5 is introduced.

No `Type::class`, `Type::type`, string class name, or general runtime type-token
model is introduced.

## Assertion Failure Semantics

Introduce the exact compiler-known Error identity:

```doria
Doria\Std\Test\AssertionError
```

An assertion failure is not a panic.

It uses ordinary checked-error ownership, cleanup, source origin, and runtime
transport. Fatal panic remains status 101 and cleanup-free under existing law.

### Test Assertion Effect

Add one compiler-owned effect class:

```text
TestAssertion
```

The complete effect classes become conceptually:

```text
Required
AmbientIo
TestAssertion
```

`TestAssertion` is available only through `Doria\Std\Test` in active development
or generated-development source.

Rules:

- authors do not write `throws AssertionError` for expectations or test helpers;
- test assertion effects propagate automatically through test call graphs;
- the effect remains catchable explicitly;
- runtime transport retains the exact `AssertionError` identity;
- assertion effects do not disappear from HIR, MIR, indirect calls, cleanup, or
  backend ABI planning;
- assertion-only differences do not create a separate structural function-type
  identity within the test-only context;
- required nonassertion Errors remain governed by ordinary checked-error rules;
- ambient I/O remains a separate effect class;
- production source cannot import the test module and therefore cannot silently
  acquire this exemption.

An explicit `throws AssertionError` remains accepted for source compatibility but
is not required in test source.

### Structured Failure Information

The compiler/runtime assertion outcome preserves at least:

```text
matcher identity
negated state
optional user message
expected presentation where applicable
actual presentation where applicable
bounded difference where applicable
assertion source origin
```

Baton consumes structured assertion identity. It must not infer an assertion
failure by scraping human stderr text.

## Failure Presentation

Baton classifies test outcomes as:

```text
Passed
Assertion Failed
Unexpected Checked Error
Fatal Panic
Abnormal Process Failure
```

A failed expectation reports:

```text
behavioral test name
assertion source location
matcher
expected value when applicable
actual value when applicable
bounded useful difference when supported
optional user message
```

Value rendering is bounded. Strings and collections show useful local context
without dumping unbounded application state.

Formatting uses compiler/runtime-owned value knowledge and existing display laws.
It does not call arbitrary user code merely to produce a failure message before
the later extensibility decision permits that behavior.

## Backend And Runtime Boundary

One validated HIR/MIR model drives:

```text
debug interpreter
Cranelift fast profile
LLVM release profile
PHP compatibility
```

Behavioral declarations and suite metadata do not become runtime registration
operations.

Expectation operations lower through compiler-known test intrinsics or an
equivalent typed standard-library lowering that preserves:

```text
readonly borrowing
single evaluation
exact assertion effect
structured failure data
ordinary cleanup
no mandatory success-path allocation
```

The PHP backend adapts to Doria testing semantics. PHP exceptions, PHPUnit, or
Pest do not define the language behavior.

## Implementation Slices

### Slice 1: Behavioral Test DSL And Unified Compiler Metadata — Implemented

Exact compiler-known resolution, development-source enforcement,
const-evaluated descriptions, nested suite extraction, direct generated
callables, unified `#[Test]` and behavioral facts, strict metadata schema 3,
ordinary HIR/MIR/backend execution, Baton schema-3 orchestration, and
compiler-fact-based language-server presentation are implemented. All three
slices are complete, and the broader foundation is implemented.

Owns:

```text
Doria\Std\Test declaration identities
describe / it / test parsing through existing call syntax
compiler-known declaration semantics
nested suite identity
compile-time descriptions
behavioral closure validation
unified #[Test] and behavioral metadata
metadata schema 3
Baton schema-3 discovery foundation
LSP syntax/semantic facts
```

Acceptance requires no runtime registration and no Baton source parsing.

### Slice 2: Fluent Expectation Kernel And Assertion Semantics — Implemented

The compiler now owns `expect`, `fail`, ephemeral expectation-chain semantics,
the `not` property, core scalar/null/bool/order/string matchers,
`AssertionError`, the separate `TestAssertion` checked-effect partition, and
strict DORIAO4 assertion outcomes. The same validated HIR/MIR executes through
the debug interpreter, Cranelift, LLVM, and PHP compatibility backends. Baton
preserves generic process isolation and `FAIL` reporting while replaying the
runtime-rendered assertion failure, and the official tooling consumes compiler
facts instead of parsing Doria source. Slice 2 is complete.

Owns:

```text
expect
fail
Expectation<T>
not property
equality
null
boolean
ordered/numeric expectations
string expectations
AssertionError
TestAssertion effect
structured assertion outcomes
readonly expectation borrowing
interpreter / Cranelift / LLVM / PHP parity
```

### Slice 3: Collections, Errors, Baton Reporting, And Tooling Closure — Implemented

Owns:

```text
collection expectations
dictionary expectations
toThrow / not->toThrow
typed Error inspectors
bounded differences
hierarchy-aware Baton output
outcome classification
filtering by behavioral name
LSP completion/hover/navigation
standard-library reference
installed-toolchain validation
cross-repository closure
```

Completed status:

```text
Native Testing Foundation - Complete
Stage 34 Single Class Inheritance - Complete
Stage 35 Interfaces And Traits - Authority Accepted; Slice 1 Next
Pre-Stage-45 Doria-Native Baton Transition - Scheduled
```

## Deliberate Deferrals

The initial foundation does not add:

```text
beforeEach / afterEach hooks
datasets
custom user matchers
snapshots
property-based testing
mocks or test doubles
parallel test execution
retries
ignored tests
tags
random ordering
editor-triggered test execution
runtime reflection
general type tokens
assertion macros
```

Scoped hooks and fixture sharing require a dedicated ownership model. Custom
matchers and user-defined display/equality extension are best revisited after
Stage 35 lands interfaces, traits, `Equatable`, `Displayable`, and `Cloneable`
conformance for user types.

This is a deferral, not a permanent rejection.

## Performance And Memory

Required direction:

```text
runtime suite registry:
    none

runtime reflection:
    none

success-path expectation allocation:
    none mandatory

actual evaluation:
    once

expected evaluation:
    once

failure formatting:
    only on failure

behavioral metadata:
    compile time / tooling only
```

No test assertion may consume a Move value merely to inspect it.

## Consequences

- Doria gains a native testing suite rather than only a test runner.
- The recommended authoring style is genuinely Pest/Jest-like through
  `describe`, `it`, `test`, and fluent `expect` chains.
- `#[Test]` remains a stable low-level metadata form.
- Assertion failures are recoverable structured test outcomes, not fatal panics.
- Test helper functions avoid checked-error boilerplate without broadening
  ambient I/O or arbitrary user Errors.
- Baton remains the runner and process orchestrator.
- The compiler remains the semantic authority.
- The language server remains a presentation client over compiler/Baton facts.
- Stage 34 is complete and Stage 35 authority is accepted and Slice 1 is next because this foundation is closed.

## Invalidated Elsewhere

- Decision 0128 remains authoritative for Baton test discovery, development
  graphs, dispatchers, process isolation, and suite execution, but no longer
  represents the complete native testing story by itself.
- Decision 0125's metadata surface gains a future additive schema version 3;
  schemas 1 and 2 remain exact.
- Decision 0123's effect architecture gains the separate test-only
  `TestAssertion` class without broadening ambient I/O.
- The end-to-end plan, current-pipeline note, standard-library reference,
  self-hosting note, README, and SPEC must schedule and describe this mandatory
  foundation before Stage 34.
- Baton strict V2/V3/V4 decoding, classified hierarchy reporting, and final
  behavioral-name filtering now consume the completed compiler outcome model.
- Language-server collection/Error completion, hovers, symbols, and navigation
  consume compiler-owned facts from the final foundation closure revision.
- Website testing documentation remains a later synchronization task.

Decision 0130 makes `toThrow` inspectors hierarchy-aware and preserves dynamic
Error identity in assertion facts and Baton presentation. Virtual test methods
retain the separate TestAssertion effect and strict DORIAO4 outcome contract;
the compiler still emits no runtime suite registry.
