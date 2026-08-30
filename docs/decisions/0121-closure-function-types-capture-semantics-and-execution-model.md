# Decision 0121: Closure Function Types, Capture Semantics, And Execution Model

- **Status:** Accepted
- **Accepted:** 2026-08-19
- **Date:** 2026-08-19
- **Implementation Status:** Authority Accepted; Stages 30a Through 30h Implemented; Stage 30 Complete
- **Scope:** Function types, closure invocation, capture validation, ownership,
  lifetime, representation, backend behavior, diagnostics, and Stage 30
  `List<T>` algorithms
- **Elaborates:** Decision 0120's explicit capture-list foundation
- **Applies:** Decisions 0081, 0083, 0089, 0090, 0106, 0112, and 0119
- **Finalizes:** Decision 0100's Stage 30 `List<T>` algorithm surface
- **Corrects:** Decision 0113's former all-collection higher-order-method row
- **Supersedes:** The in-review Stage 30 closure authority proposal as normative
  authority

## Context

Decision 0120 fixed one explicit capture syntax for arrow and block closures and
the pre-Stage-30 grammar slice preserved that syntax in the AST. It deliberately
left function-type capabilities, `$this`, free-variable validation, escape,
runtime representation, execution, and collection callback contracts for Stage
30. Decision 0119 independently fixed checked-effect sets and checked-call ABI.

This record closes those authority questions before implementation. Stage 30a
implements the accepted callable grammar. Stage 30b implements canonical
semantic function types, stable binding and closure identities, capture and
`$this` validation, inferred closure modes/effects, and semantic callable-value
calls. Stage 30c implements creation-time capture acquisition, Move-only
function values, non-lexical capture leases, path-sensitive invocation
consumption, escape and storage enforcement, returned-borrow roots, nested
environment provenance, and reverse logical release plans. Valid execution
routes now lower through explicit closure-aware HIR and MIR and execute through
the debug interpreter. Stage 30d implements structural MIR function types, the
logical descriptor/environment carrier, static closure descriptors, source-order
capture acquisition, reverse logical release, synthetic closure functions,
indirect and checked indirect calls, shared validation, stable interpreter
places, and exact environment cleanup. Stage 30e implements the shared native
two-word carrier and descriptor ABI, stack/heap environment placement, generated
drop glue, and indirect execution through Cranelift and LLVM. Stage 30f emits
PHP compatibility closures from the same semantic and validated MIR authority
through explicit carriers, environments, and stable places. Stage 30g adds
explicit semantic and HIR List algorithm plans plus validated MIR traversal CFG
for `map`, Copy-only `filter`, and writable-accumulator `reduce`. The debug
interpreter, Cranelift, LLVM, and PHP compatibility execute the supported
surface with exact callback effects, readonly source traversal, and checked
partial-state cleanup. Stage 30h completes the accepted mixed function-value
representation, storage-route audit, tooling integration, UAT, and portable
benchmark closure. Stage 30 is complete.

```text
Stage 30a Callable Grammar Completion — Complete
Stage 30b Semantic Function Types And Captures — Complete
Stage 30c Ownership, Lifetime, And Escape — Complete
Stage 30d Closure HIR/MIR And Interpreter Oracle — Complete
Stage 30e Native Execution — Complete
Stage 30f PHP Compatibility — Complete
Stage 30g List Algorithms — Complete
Stage 30h Cross-Repository Closure — Complete
Stage 30 — Complete
E0641 — Historical And Reserved
Stage 31 Slice 1 — Complete
Stage 31 Slice 2 — Complete
Stage 31 — Complete
Stage 32 — Complete
Stage 33 Slice 1 — Complete; Stage 33 Slice 2 — Complete; Stage 33 Slice 3 — Complete
Stage 33 — Complete; Phase F — Complete
Native Testing Foundation Slice 1 — Complete; Slice 2 — Next
Stage 34 — Blocked Until The Native Testing Foundation Completes
```

## Accepted Amendment: Parenthesized Type Grouping

**Accepted: 2026-08-20**

Doria accepts `(Type)` as source-preserving type grouping. Grouping makes nested
function-type boundaries and ownership of following checked-effect clauses
explicit:

```doria
function((function(): int throws FirstError), string): void
function(): (function(): int) throws Failure
function(): (function(): int throws InnerError) throws OuterError
```

The authored opening parenthesis, inner type, closing parenthesis, and whole
group span remain in parsed type syntax. Grouping has the semantics of its inner
type. It is not a tuple, product, union, runtime wrapper, or nominal type, and
`(int, string)` is rejected. Stage 30a implements this grammar clarification;
semantic function types and checked-effect compatibility remain Stage 30b work.

## Decision

### 1. Three Independent Ownership Axes

A function value has three independent ownership dimensions:

1. how the function value itself is passed or stored;
2. what access invocation requires to its environment; and
3. how each source argument is passed.

Existing leading declaration modes govern the function value:

```doria
function(int): int $callback
writable function writable(int): int $callback
take function once(): Payload $factory
```

The leading `writable` or `take` is not part of structural function-type
identity. Every function value is a Move value, including no-capture closures.
Function values have no implicit Copy, Cloneable, Hashable, Comparable,
Displayable, equality, or ordering capability. Doria introduces no automatic
sharing or pervasive reference counting for closures.

### 2. Invocation Modes

Function types have exactly three invocation modes:

```doria
function(int): int
function writable(int): int
function once(): Payload
```

- `function(...)` is readonly repeatable invocation.
- `function writable(...)` is writable repeatable invocation.
- `function once(...)` is consuming one-shot invocation.

`function take()` is rejected. `take` means value transfer and does not also
mean one-shot invocation.

A closure's minimum invocation mode is inferred from its body. Readonly
environment access produces a readonly closure; mutation of captured state
produces a writable closure; moving an owned capture out or otherwise consuming
the environment produces a once closure. Calling a writable closure requires
exclusive access to the closure value. Calling a once closure consumes it.

### 3. Function-Type Parameters

Structural function-type parameters preserve Doria's ordinary parameter modes:

```doria
function(Counter): void
function(writable Counter): void
function(take Payload): string
```

Bare parameters borrow readonly, `writable` borrows exclusively, and `take`
transfers ownership. Parameter names are absent. `take` and `writable` remain
mutually exclusive on one parameter.

### 4. Checked Effects

Function types use Decision 0119's `throws` vocabulary:

```doria
function(string): Record throws ParseError, StorageError
```

The normalized required checked-effect set is part of structural identity.
Decision 0123 excludes the two exact ambient canonical I/O identities from that
identity while retaining them in the callable's complete runtime effect profile
and checked ABI transport. Source order is preserved for display,
documentation, hovers, signatures, and diagnostics; semantic comparison uses
the normalized required set. Closure bodies infer checked effects after local
catch subtraction. There is no closure-expression `throws` annotation. The
selected entrypoint's existing inference rule does not grant general
named-callable inference.

### 5. Structural Identity And Compatibility

A semantic function type contains:

- parameter count, value types, and ownership modes;
- invocation mode;
- return type;
- normalized required checked-effect set, plus a non-structural ambient runtime
  effect profile;
- nullability;
- monomorphized enclosing generic identities; and
- compiler-proven return-borrow provenance where existing elision can express
  it.

Parameter names are absent. Borrows returned from captured environments are
deferred. A closure may return a borrow derived from exactly one borrowed
argument when existing provenance rules prove it.

Compatibility requires equal arity, exact parameter value types and ownership
modes, exact return value types, and equal monomorphized identities. The only
Stage 30 substitutions are:

- actual checked effects may be a subset of expected effects;
- readonly invocation may satisfy readonly, writable, or once;
- writable invocation may satisfy writable or once;
- once invocation may satisfy once only; and
- non-null may satisfy nullable, while nullable requires narrowing before a
  non-null use.

General parameter contravariance and return covariance remain deferred until
inheritance and interface work provides executable consumers.

### 6. Closure-Expression Typing

Closure parameters remain explicitly typed. Arrow closures infer their return
type from reachable expression paths. Anonymous block closures keep their
mandatory written return type. Invocation mode and checked effects are inferred
from the body. An expected function type may constrain return type, invocation
mode, and checked-effect set, but never supplies omitted parameter types.

Stage 30 adds no implicit `mixed` parameters, default or variadic closure
parameters, closure-local generic declarations, recursive closure-self syntax,
or closure-expression effect annotations. Enclosing generic parameters remain
in scope and specialize normally. A closure cannot recursively refer to the
binding being initialized.

### 7. Callable-Value Invocation

Stage 30a accepts postfix invocation of any expression with a callable semantic
type:

```doria
$callback($value);
($factory())($value);
$factory()($value);
$object->callbackProperty($value);
$callbacks[0]($value);
```

Callable-value invocation is represented separately from a named free-function
call. The callee is evaluated exactly once, followed by arguments from left to
right, then invocation. Structural calls are positional and have no named or
default arguments because structural function types contain neither parameter
names nor defaults. Nullable callables require narrowing. Stage 30 adds no
null-safe callable operator. Existing member resolution distinguishes callable
properties from methods.

### 8. Callable References

Stage 30 supports closure expressions as function values. It does not add source
syntax for named top-level function, static-method, bound-instance-method,
constructor, PHP callable-array, Rust function-item, or C address-of references.
Users adapt existing callables through closures:

```doria
let $callback = fn(int $value) => transform($value);
```

The compiler may eliminate the wrapper when behavior is unchanged. Direct named
calls remain supported. Callable references may be added later without changing
this closure model.

### 9. Explicit `$this` Capture

Method-local closures capture their receiver explicitly:

```doria
with ($this)
with (writable $this)
```

Readonly receiver access uses the first form. Writable property access and
writable method calls require the second. `with (take $this)` is rejected because
a method borrows its receiver rather than owning it. Readonly methods may create
readonly-receiver closures; writable methods may create readonly or writable
receiver closures; static methods have no `$this`.

Every intermediate nested closure carries `$this` explicitly when a deeper
closure uses it. A receiver-borrowing closure cannot outlive the receiver or be
stored on that receiver. Owned receiver capture and self-borrow cycles remain
deferred. PHP lowering must not inherit PHP's implicit `$this` behavior.

### 10. Binding Identity And Capture Validation

Lexical variables resolve to stable semantic binding identities before capture
validation. Name scanning is insufficient. Discovery includes parameters,
locals, nested blocks, loops, patterns, catches, `given` bindings,
interpolations, match guards, and nested closures. Constants, statics, top-level
functions, type names, enum cases, and a closure's own parameters and locals do
not require capture.

Each closure is a distinct lexical root. A nested closure using an ancestor
binding requires that identity through every intermediate environment; it never
points directly into an arbitrary grandparent stack frame.

Validation rejects missing, duplicate, undeclared, own-local, own-parameter,
non-binding, and wrong-mode captures. Writable capture from a readonly source and
taking capture from a borrowed Move source are errors. Taking a Copy value copies
an independent value into the environment and leaves the source usable. Unused
captures warn. Parameter shadowing of a listed capture is an error on the capture
entry. Capture after move and insufficient lifetime use ordinary ownership and
lifetime diagnostics with capture context.

Capture source order governs acquisition, diagnostics, logical destruction, and
source display. It does not govern lookup or structural function-type identity.

### 11. Diagnostics And Fixes

Emit one missing-capture diagnostic per missing binding identity, ordered by
first use; repeated uses share one cause. The Title Case direction is:

```text
Closure Must Capture `$minimum`
```

Labels identify the closure, declaration, first relevant use, and existing
capture clause. Diagnostics explain that callers provide parameters while the
surrounding lexical scope provides captures.

Machine fixes may insert or append a readonly capture when unambiguous, insert
`writable` only when mutation and source capability make it unambiguous, and
remove an unused capture only while preserving trivia and comments. They never
insert `take`, never duplicate captures, and diagnose the nearest missing nested
environment link. `$this` follows the same policy.

### 12. Acquisition, Lifetime, And Escape

Closure creation selects or allocates environment storage, acquires captures
left to right in written source order, then produces the closure value. Readonly
and writable borrows begin at creation. Taking capture transfers ownership at
creation. Nothing is delayed until first invocation.

A never-invoked closure still drops owned captures. Captures are destroyed in
reverse logical acquisition order. Moving a closure transfers one cleanup
obligation. Repeatable calls do not reacquire captures. A once call consumes the
environment and, after normal completion or checked propagation, destroys every
remaining capture exactly once. Fatal panic remains non-unwinding; allocation
failure uses the existing fatal allocation panic.

Doria adds no `nonescaping` keyword. Existing function-value parameter ownership
controls escape:

```text
function(...) parameter           nonescaping readonly borrow
writable function(...) parameter  nonescaping exclusive borrow
take function(...) parameter      ownership transfer; retention may be permitted
```

Return, property/static storage, collection or typed-array insertion, enum
payload storage, `mixed` boxing, and aggregate retention are escapes.
Borrow-capturing closures may escape only when every owner outlives the
destination. Returning a closure that borrows a local is rejected. No-capture
and take-only environments are owned and may escape. Nullable wrappers preserve
provenance. `List<T>` algorithm callbacks are nonescaping.

### 13. Function-Value Support Matrix

| Position | Stage 30 status |
| --- | --- |
| Local variables | Supported |
| Parameters | Supported |
| Return values | Supported when owned or lifetime-safe |
| Instance properties | Supported for owned/no-capture environments |
| Static properties | Deferred |
| Constants | Rejected |
| Parameter defaults | Rejected |
| Instance property initializers | No-capture closures only |
| Typed arrays | Supported for owned function values |
| `List` values | Supported for owned function values |
| `Dictionary` values | Supported |
| `Dictionary` keys | Rejected |
| `Set` elements | Rejected |
| `SortedDictionary` values | Supported |
| Sorted keys/elements | Rejected |
| `PriorityQueue` elements | Rejected |
| `Deque` elements | Supported |
| Enum payloads | Supported |
| Nullable values | Supported |
| `mixed` | Supported |
| Generic arguments | Supported invariantly |
| Shared-reference payloads | Deferred |

Representation support never overrides escape checking.

### 14. Carrier, Descriptor, And Environment

The logical closure carrier is two words:

```text
descriptor pointer
environment pointer
```

No-capture closures use a static descriptor and null environment and allocate no
environment. Nullable absence uses null descriptor and null environment.
Different expressions may have different descriptors while satisfying one
structural type.

Descriptors are lean, compiler-private runtime records, not reflective copies of
parameter types, return types, effect sets, generic identities, or source type
spellings. They carry only validated indirect-call entry points, environment
size/alignment, drop glue, required cleanup metadata, stable Doria debug
identity, and compact runtime type identity where exact narrowing requires it.
Checked effects need not survive validated lowering as runtime metadata.

Logical capture order remains source order for acquisition, diagnostics,
ownership events, reverse destruction, and debug presentation. Physical fields
may reorder privately to reduce padding if metadata maps logical captures to
physical offsets and logical destruction order. Layout is unobservable. Taking
captures move; readonly and writable captures use private borrow carriers with
provenance.

No-capture closures allocate no environment. Nonescaping environments may be
stack allocated, scalar replaced, eliminated, or direct-called. Escaping owned
environments may use one heap allocation. Doria requires no heap allocation
merely because a closure exists and no tracing GC, pervasive reference counting,
per-call allocation, or per-element callback wrapper.

### 15. Invocation ABI And Backends

Indirect invocation prepends the hidden environment pointer to the existing
callable ABI. Nonthrowing calls reuse current return conventions; throwing calls
reuse Decision 0119's checked status/out-slot conventions. There is no boxed
result or host exception ABI. Writable invocation requires exclusive environment
access; once invocation consumes the carrier.

Validated MIR rejects function-type, invocation-mode, effect, environment,
cleanup, argument-ownership, and return mismatches. The interpreter, Cranelift,
LLVM, and PHP ultimately consume one validated MIR model. ABI-compatible code
may be deduplicated after exact semantic effect validation.

PHP compatibility follows Doria's explicit captures and inferred modes. It does
not use PHP automatic arrow capture or expose PHP references as borrowing. The
backend may use static host closures, generated wrappers/cells, and ownership
temporaries. No-capture closures are static. Owned captures drop exactly once in
reverse logical order. Host helper names, PHP `Closure` internals, stack frames,
and reference identity are unobservable.

### 16. `List<T>` Algorithms

Stage 30g adds higher-order algorithms only to `List<T>`. The following
`effects(...)` notation describes compiler-internal contracts and is not Doria
source syntax. Each algorithm has two compiler-known callback-mode
specializations. These are not user-declared overloads:

- a readonly-repeatable callback is passed as a readonly function-value borrow;
- a writable-repeatable callback is passed as an exclusive function-value
  borrow and therefore requires writable access to the callback value.

The compiler selects the least-capable specialization that satisfies the
callback's inferred minimum invocation mode. A readonly callback never requires
an exclusive borrow merely because readonly invocation can substitute for a
writable expectation. A writable callback can never pass through the readonly
specialization. Once callbacks remain rejected.

#### `map`

```text
map<U>(function(T): U transform): List<U> effects(transform)
map<U>(writable function writable(T): U transform): List<U> effects(transform)
```

The callback is readonly- or writable-repeatable and nonescaping. The leading
parameter mode and invocation mode remain coupled as defined above.
The readonly receiver remains unchanged. Elements are lent readonly in insertion
order, including Move elements. Each owned or Copy result moves into a new list.
On checked failure, produced results and the partial list are destroyed, the
source remains unchanged, and the callback error propagates.

#### `filter`

```text
filter(function(T): bool predicate): List<T> effects(predicate) where T: Copy
filter(writable function writable(T): bool predicate): List<T> effects(predicate) where T: Copy
```

The callback is readonly- or writable-repeatable and nonescaping, using the
matching readonly or exclusive function-value borrow.
The readonly receiver remains unchanged. Elements are tested in insertion order
and selected Copy values enter a new ordered list. Move-element preserving
filter waits for `Cloneable`; Stage 30 adds no consuming filter or borrowed view.

#### `reduce`

```text
reduce<A>(take A initial, function(writable A, T): void reducer): A effects(reducer)
reduce<A>(take A initial, writable function writable(writable A, T): void reducer): A effects(reducer)
```

`reduce` owns the initial accumulator. For each element in insertion order it
lends the accumulator writably and the element readonly, then ends the writable
borrow before the next iteration. The callback may be readonly- or
writable-repeatable with respect to its own environment and is nonescaping.
Writable-repeatable reducers are borrowed exclusively; once reducers are
rejected. Empty input returns the initial accumulator unchanged. Copy and Move
accumulators are supported. On checked failure, the borrow ends, the owned
accumulator is destroyed exactly once, the source remains unchanged, and the
error propagates.

The algorithm call's effective set is exactly the callback's normalized set and
participates in semantic specialization and checked MIR signatures. Public docs
state that the algorithm throws its callback's checked effects without exposing
the internal notation.

No Stage 30 higher-order algorithm is added to Dictionary, SortedDictionary,
Set, SortedSet, PriorityQueue, Deque, typed arrays, Iterable, or Iterator. Those
families require separate result-shape, order, ownership, and entry-model
authority.

### 17. Stable Closure Identity

Each closure expression receives a stable internal ClosureId from stable source
identity and expression span. Human call paths use Doria labels such as `closure
at path:line:column`, optionally naming the containing Doria callable. Panic,
checked-origin, MIR, interpreter, native, and PHP diagnostics share those facts
and never expose bootstrap/compiler/backend symbols, host stack frames, native
addresses, or helper names.

### 18. E0641 Retirement

E0641 retires by route:

1. Stage 30a keeps E0641.
2. Stage 30b removes it from completed semantic function-type contexts.
3. Valid closures retain a narrower execution boundary after semantic checking.
4. Stage 30c replaces ownership boundaries with precise diagnostics.
5. Stage 30d removes it from target-neutral checking, HIR/MIR lowering,
   interpreter-supported expressions, and debug execution.
6. Native, PHP, algorithm, and storage slices retire their bounded routes.
7. Stage 30h makes E0641 historical only when every accepted form has an
   intentional route.

Implementation beginning is not sufficient reason to remove E0641.

## Implementation Sequence

The accepted dependency order is:

```text
Authority Acceptance
Stage 30a - Callable Grammar Completion
Stage 30b - Semantic Function Types And Captures
Stage 30c - Ownership, Lifetime, And Escape
Stage 30d - Closure HIR/MIR And Interpreter Oracle
Stage 30e - Native Execution
Stage 30f - PHP Compatibility
Stage 30g - List Algorithms
Stage 30h - Cross-Repository Closure
```

- **Authority Acceptance:** this record, synchronized authority, and guardrails;
  no compiler execution change.
- **Stage 30a — Complete:** parameter ownership in function types, writable/once modes,
  function-type effects, arbitrary postfix invocation, AST/recovery/fixtures,
  coordinated editor tooling; E0641 remains.
- **Stage 30b — Complete:** semantic function types, compatibility, binding
  identities, capture discovery/validation, `$this`, fixes, inferred
  modes/effects, and semantic callable calls; no HIR/MIR.
- **Stage 30c — Complete:** acquisition at closure creation in authored order,
  non-lexical readonly/writable leases, Move behavior, once consumption,
  nonescaping callback enforcement, storage/escape and one-root return checks,
  nested provenance, and reverse logical drop plans; no HIR/MIR or execution.
- **Stage 30d — Complete:** closure-aware HIR, structural function MIR,
  carrier/descriptors/environments, indirect and checked calls, shared MIR
  validation and cleanup, stable-place interpreter execution, and the bounded
  executable-backend handoff.
- **Stage 30e — Complete:** shared native carrier/descriptor/environment ABI,
  stack and heap placement, stable native capture places, generated drop glue,
  Cranelift and LLVM execution, malformed-MIR validation, and native parity.
- **Stage 30f — Complete:** explicit PHP lowering consumes semantic and validated
  MIR closure plans; compiler-owned carriers, environments, stable places,
  source identities, checked effects, moves, replacement, and cleanup preserve
  Doria behavior without PHP automatic capture or host callable semantics.
- **Stage 30g — Complete:** the three `List<T>` algorithms, internal effect
  specialization, partial-result and accumulator cleanup, shared MIR validation,
  and interpreter/Cranelift/LLVM/PHP parity.
- **Stage 30h — Complete:** mixed function-value carriers, final storage-route
  and E0641 audits, language-server/editor alignment, installed-toolchain
  refresh, website/playground activation, and portable benchmark closure.

Every slice remains independently testable and does not hide future-slice work
inside backend conveniences.

## Performance And Memory

Implementation adds portable structural and benchmark cases for no-capture,
readonly/writable/taking capture, direct/indirect and once calls, nonescaping and
escaping environments, checked callbacks, all three algorithms, compile time,
peak memory, allocation counts where available, and artifact size.

Required structural guarantees are zero environment allocation for no-capture
closures, no semantic heap requirement for proven nonescaping environments, no
mandatory per-call or per-element-wrapper allocation, and exact-once capture and
partial-result destruction.

Decision 0112 remains controlling. Controlled physical-host timing retains
`Measurement Status: Pending Available Runner`; unavailable hardware is not a
Stage 30 gate and does not convert missing evidence into a pass.

## Explicit Deferrals

- async, spawned, cross-task, and concurrently writable closures;
- `Sendable`, `Shareable`, and thread-safe closure environments;
- taking `$this`, owned receiver capture, and self-borrow cycles;
- recursive/self-referential closures and closure-local generic declarations;
- variadic or default closure parameters and closure-expression effects;
- borrows returned from captured environments;
- null-safe callable invocation;
- named-function, static-method, bound-method, and constructor references;
- implicit equality, hashing, ordering, display, cloning, or dynamic reflection;
- PHP callable arrays;
- function values in shared-reference payloads or owned static properties;
- Move-element preserving filter before `Cloneable`, consuming filters, and
  filter views;
- higher-order algorithms on non-List collections; and
- general Iterable or Iterator algorithms.

## Alternatives Considered

- **Implicit arrow or Copy capture.** Rejected by Decision 0120 because hidden
  dependencies and ownership differ under refactoring.
- **`function take()` for one-shot invocation.** Rejected because `take` already
  transfers the function value; `once` names invocation behavior.
- **Full runtime-reflective descriptors.** Rejected as unnecessary metadata and
  contrary to Doria's static, headless representations.
- **Source-order physical layout.** Rejected as an observable padding burden;
  logical source order remains authoritative while layout stays private.
- **Move-through-every-call reduce.** Rejected in favor of one owned accumulator
  lent writably for each callback.
- **Algorithms on every collection.** Rejected because result shape, ordering,
  ownership, and entry contracts differ by family.
- **Hidden sharing or pervasive reference counting.** Rejected by Doria's
  ordinary ownership model and Decision 0106's explicit shared families.

## Consequences

- Function types carry enough static information for safe ownership, effects,
  invocation, generic specialization, and checked ABI selection.
- Explicit captures remain refactoring-stable and now include receivers.
- Runtime representation is compact without becoming reflective.
- `List<T>` algorithms have exact callback, ownership, order, and failure
  contracts rather than a nominal method list.
- Stage 30 implementation is risk-separated. E0641 is historical and reserved
  now that every accepted route has an intentional outcome.

## Affected Components

Stage 30 implementation affected parser/AST, semantic types and binding identities,
ownership/lifetime/escape checking, HIR/MIR and validation, interpreter,
Cranelift, LLVM, PHP compatibility, diagnostics, `List<T>` algorithms,
language-server/editor tooling, website/playground material, installed tooling,
and benchmark infrastructure. Stage 30h closes those routes without changing
the accepted semantics or deferrals.

## Invalidated elsewhere

- The supporting Stage 30 proposal is historical and no longer requests
  Approve/Amend/Reject rulings.
- Decision 0120's open `$this`, effect, ABI, and slicing questions are settled by
  this record without weakening explicit captures.
- Decision 0119's closure-effect TODO is settled as inferred closure effects and
  structural function-type `throws` sets.
- Decision 0113's broad all-collection Stage 30 row is superseded; only `List<T>`
  receives these algorithms.
- Stage 30a coordinates the compiler pin, `once`, function-type ownership and
  effects, grouping, callable invocation fixtures, parser-backed LSP tests, VS
  Code and IntelliJ grammar changes, and shared editor fixtures. It adds no
  closure semantics or execution route.
- The website's versioned closure guides, `List<T>` API contract, target-state
  collection matrix, writable-callback fixture, tests, and release lock are
  synchronized with executing compiler support. Stage 30h changes no accepted
  public contract.
