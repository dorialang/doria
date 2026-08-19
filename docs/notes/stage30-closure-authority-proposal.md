# Stage 30 Closure Authority Proposal

## Status

**In Review.** This is a supporting design proposal, not accepted language
authority. Decision 0120 remains the accepted capture authority. Stage 30 is not
implemented, and the compiler continues to report E0641 after validating the
currently accepted closure and function-type syntax.

## Scope

This proposal audits the complete Stage 30 closure design surface: function
types, capture validation, invocation, ownership and escape rules, checked
effects, environment representation, backend behavior, diagnostics, and the
three `List<T>` higher-order algorithms already assigned to Stage 30.

It proposes no compiler, runtime, backend, language-server, editor, website, or
benchmark implementation. A numbered accepted decision must follow Andrew's
rulings before implementation begins.

## Andrew Decision Checklist

Each recommendation below is independent unless its consequences say otherwise.

| ID | Question | Recommended answer | Alternatives and invalidating consequences | Andrew |
| --- | --- | --- | --- | --- |
| D1 | Callable invocation kinds | Three semantic kinds: readonly repeatable, writable repeatable, and consuming one-shot. Infer the minimum kind from the body. | One repeatable kind would hide mutation; two kinds cannot represent moving a capture out. | Approve / Amend / Reject |
| D2 | Function-type parameter ownership | Preserve default readonly, `writable`, and `take` inside the function-type parameter list. | Exact types alone lose the caller/callee ownership contract. | Approve / Amend / Reject |
| D3 | Function-type invocation-mode spelling | Default readonly is `function(...)`; propose `function writable(...)` and `function take(...)` for the other invocation kinds. | Prefix forms collide with the ownership of the function value; new words such as `once` add vocabulary. | Approve / Amend / Reject |
| D4 | Function-type checked-effect spelling | Mirror named callables: `function(...): R throws E1, E2`. | Omitting effects makes safe substitution impossible; a separate effect syntax duplicates `throws`. | Approve / Amend / Reject |
| D5 | Closure effects | Infer effects from closure bodies; do not add closure-expression `throws` syntax in Stage 30. Expected function types constrain the inferred set. | Optional or required annotations add grammar and can drift from the body. | Approve / Amend / Reject |
| D6 | Invocation grammar | Accept postfix invocation of every expression with a callable type: `$callback(...)`, `($factory())(...)`, `$factory()(...)`, callable properties, and indexed callables. | Variable-only invocation is smaller but creates an arbitrary permanent restriction. | Approve / Amend / Reject |
| D7 | `$this` capture | Require `with ($this)` or `with (writable $this)`; reject `take $this` in v1. | Implicit capture breaks explicit dependency rules; banning `$this` is unnecessarily restrictive. | Approve / Amend / Reject |
| D8 | Nested free variables | Resolve binding IDs lexically and require every intermediate closure to carry each ancestor binding used below it. | Direct grandparent pointers complicate lifetime proofs and environment ownership. | Approve / Amend / Reject |
| D9 | Unused and duplicate captures | Duplicate capture is an error. Unused capture is a warning with a safe removal fix when comments are unaffected. | Accepting duplicates makes acquisition order ambiguous; making unused capture an error is needlessly brittle. | Approve / Amend / Reject |
| D10 | Missing-capture fixes | Auto-add readonly or writable only when use and source capability make the mode unambiguous; `take` always requires review. | Guessing `take` changes ownership and API lifetime merely to make code compile. | Approve / Amend / Reject |
| D11 | Callback escape contract | Existing parameter ownership is sufficient: readonly/writable callback parameters are nonescaping borrows; `take` transfers ownership and may escape. | A separate `nonescaping` marker duplicates the ownership contract. | Approve / Amend / Reject |
| D12 | Closure storage | Support owned/no-capture closures in ordinary Move-capable value positions; reject capability-constrained key/sorted positions; enforce borrow lifetimes separately. | Local-only closures avoid runtime work but make function types incomplete. | Approve / Amend / Reject |
| D13 | Environment ABI | Two-word closure carrier: static descriptor pointer plus environment pointer. Descriptors own entries, type identity, layout, and drop glue. | Code-plus-environment needs separate metadata elsewhere; a larger carrier taxes every value. | Approve / Amend / Reject |
| D14 | Nullable and `mixed` | Support nullable closure values with a null descriptor; support `mixed` only in the final representation slice with an exact function-type tag and drop glue. | Permanent deferral makes `mixed` less complete; untyped invocation would violate narrowing. | Approve / Amend / Reject |
| D15 | PHP representation | Generate backend-private PHP closures/wrappers from Doria's explicit modes; never rely on PHP implicit arrow capture or expose host references. | Direct host capture is shorter but lets PHP define Doria behavior. | Approve / Amend / Reject |
| D16 | `List::map` | Preserve the list, borrow each element readonly, accept a nonescaping repeatable callback, move each owned result into a new ordered list, and propagate callback effects. | Copying Move elements or consuming the source is unnecessary. | Approve / Amend / Reject |
| D17 | `List::filter` | Preserve the source and support Copy elements in Stage 30; defer preserving Move-element filtering until `Cloneable`. | Consuming filter changes the receiver contract; a view depends on later iteration protocols. | Approve / Amend / Reject |
| D18 | `List::reduce` | Move an initial accumulator through a nonescaping repeatable callback, borrow elements readonly, preserve source order, and return the final owned accumulator. | Copy-only accumulation needlessly excludes classes and collections. | Approve / Amend / Reject |
| D19 | Implementation sequence | Use bounded grammar, semantic typing, ownership, MIR/interpreter, native, PHP, algorithms, and cross-repository closure slices. | One large Stage 30 change obscures semantic and backend regressions. | Approve / Amend / Reject |
| D20 | Grammar completion | Add function-type ownership/mode/effects and general callable postfix invocation before semantic implementation. `$this` capture already parses. | Hiding grammar changes inside semantic slices violates the two-clocks rule. | Approve / Amend / Reject |

## Existing Authority

- Decision 0120 accepts explicit `with` lists for arrows and block closures,
  fixes readonly/writable/taking capture spellings, prohibits implicit local
  capture, and assigns the remaining closure model to Stage 30.
- Decision 0119 requires every callable to carry an effective checked-effect set
  and fixes subset-effect substitution, checked status/out-slot transport,
  cleanup, and non-unwinding panic separation. Reusable callables declare that
  set explicitly; a clause-free selected program entrypoint infers its exact
  uncovered set. This entrypoint exception does not grant general named-callable
  or closure effect inference beyond the Stage 30 questions below.
- Decisions 0081, 0083, 0089, and 0090 fix Move/Copy transfer, reverse
  destruction, non-lexical borrows, returned-borrow provenance, and definite
  construction.
- Decisions 0082 and 0044 keep object and runtime ABI metadata private,
  compiler-owned, and backend-independent.
- Decisions 0069 and 0093 fix `mixed` and nullable narrowing and representation
  laws.
- Decision 0105 fixes monomorphized generics and invariant type arguments.
- Decisions 0100 and 0113 assign `map`, `filter`, and `reduce` to closure work;
  Decision 0120 narrows the accepted Stage 30 algorithm grant to `List<T>`.
- Decision 0106 keeps ordinary ownership separate from explicit shared ownership
  and prohibits hidden sharing or pervasive reference counting.
- Decision 0109 owns diagnostic facts and stable Doria call-path presentation.

## Current Implementation Inventory

- The lexer and parser accept `fn`, anonymous `function` expressions, explicitly
  typed closure parameters, block return annotations, explicit `with` captures,
  and `function(T): R` syntax.
- `ClosureExpression`, capture modes, source spans, and `FunctionTypeRef` preserve
  parsed syntax in the AST.
- Function-type parameters currently preserve only a type. There is no semantic
  `TypeKind` or `ResolvedType` for callables.
- Semantic checking validates component type positions, reports one E0641
  boundary, and intentionally does not inspect closure bodies for Stage 30
  semantics.
- The postfix parser recognizes named free-function calls, methods, and indexing,
  but an arbitrary expression followed by `(...)` is rejected as "only named
  function calls are supported in this position." Closure-variable invocation
  therefore needs a grammar/AST addition.
- `with ($this)`, `with (writable $this)`, and `with (take $this)` already parse
  because `$this` is lexed as a variable in a capture entry. Their semantics are
  deliberately unset.
- HIR, MIR, MIR validation, the interpreter, Cranelift, LLVM, and PHP contain no
  Doria closure value or closure invocation representation.
- Ownership and narrowing walkers currently treat `Expr::Closure` as an opaque
  boundary.
- The language server and both editor grammars understand the accepted syntax,
  preserve compiler-owned E0641, and do not implement Stage 30 semantics.
- There is no top-level `benchmarks/` directory. Existing benchmark authority and
  runners live under the repository's Stage 26b performance machinery; Stage 30
  benchmark cases remain unimplemented.

## Executive Recommendation

Adopt structural, move-only function types with three independent axes:

1. the ownership mode used to pass the closure value;
2. the access required to invoke its environment;
3. each closure argument's ownership mode.

Infer the minimum invocation capability and checked-effect set from the closure
body. Require explicit capture of `$this` just like any other lexical dependency,
but reject taking the borrowed method receiver in v1. Use existing parameter
ownership as the escape contract: borrowed callback parameters cannot retain the
closure, while `take` parameters may.

Represent every closure value as a two-word descriptor/environment carrier. Keep
capture storage concrete and monomorphized, allow stack allocation or elimination
for nonescaping environments, and heap-allocate only when an owned environment
must escape. Use existing checked-call and cleanup machinery with a hidden
environment argument.

Implement `List<T>` algorithms only. `map` borrows input elements and owns its
results; `filter` preserves its source and therefore supports only Copy elements
until `Cloneable`; `reduce` moves one accumulator through ordered calls. All three
are nonescaping and effect-polymorphic over their callback.

## Detailed Decision Areas

### 1. Three distinct ownership axes

**Current authority.** Decisions 0083 and 0089 distinguish readonly borrow,
writable borrow, and `take`; Decision 0120 applies those modes to captures but
does not define invocation capability.

**Question.** How are closure-value transfer, environment access during a call,
and closure-argument passing represented without conflation?

**Viable options.** Use one callable kind; use readonly/writable only; or use
readonly repeatable, writable repeatable, and consuming one-shot kinds.

**Tradeoffs.** One kind hides mutation or rejects useful closures. Two kinds
cannot safely move an owned capture out. Three kinds add type identity but map
directly onto existing Doria capabilities.

**Recommendation.** Use all three axes in v1. The outer declaration modifier
governs the closure value (`function(...)` borrow, `writable function(...)`
exclusive borrow, `take function(...)` transfer). Function-type parameter modes
govern arguments. A distinct invocation mode is inferred from body use:

- readonly repeatable when captured state is only read;
- writable repeatable when captured state is mutated but remains initialized;
- consuming when a captured owned value or the environment is moved out.

A writable call requires an exclusive borrow of the closure value for the call.
A consuming call takes the closure value and makes future calls impossible.
Moving an owned capture out therefore necessarily makes the closure one-shot.

**Consequences.** The compiler can enforce many-readers-XOR-one-writer at calls,
and capability substitution can safely allow a less-demanding callable where a
more-capable caller contract is available.

**Invalidated elsewhere.** Function-type AST/types, callable hovers, ownership
analysis, collection callback binding, MIR call validation, and editor grammar
all need the three axes after acceptance.

**Andrew decision.** Approve / Amend / Reject.

### 2. Function-type identity and syntax

**Current authority.** `function(T): R` is accepted; names are absent; nullable
types and generic arguments already compose structurally. Decision 0120 says
closure values are Move types.

**Question.** Which callable facts are type identity, and how are they spelled?

**Viable options.** Keep only parameter/return types; add modes but omit effects;
or make modes, effects, and semantic return provenance part of the complete type.

**Tradeoffs.** A partial identity cannot safely invoke or substitute callables.
Adding Doria's existing words is more verbose but avoids a second capability
system.

**Recommendation.** Function types are structural and include:

- parameter types and readonly/writable/taking ownership modes;
- readonly, writable, or taking invocation mode;
- return type;
- normalized checked-effect set, while preserving source order for display;
- nullability;
- monomorphized enclosing generic arguments;
- compiler-resolved return-borrow provenance where existing elision can prove it.

Parameter names remain absent. Proposed syntax:

```doria
function(int): int
function(writable Counter): void
function(take Payload): string
function writable(int): int
function take(): Payload
function(string): Record throws ParseError, StorageError
?function(int): int
```

The position after `function` denotes invocation mode. A modifier before the
type denotes how the closure value parameter is passed, so this is unambiguous:

```doria
function apply(
    writable function writable(int): int $callback,
): void
{
}
```

All function values are Move types, including no-capture values. They may be
generic arguments and nullable values. They have no implicit equality, hashing,
ordering, display, or cloning. Stage 30 does not support a closure returning a
borrow from its captured environment; return-borrow elision from exactly one
borrowed argument remains available when the existing provenance law can express
it. Captured-environment borrow returns are deferred pending explicit authority.

**Consequences.** Function types can be interned and monomorphized without
nominal wrapper declarations. Dictionary keys, sets, sorted collections, and
priority queues cannot use them because no capability is inferred.

**Invalidated elsewhere.** `FunctionTypeParameterRef`, semantic type registries,
type display, specialization keys, diagnostics, hovers, and editor scopes need
the added fields after approval.

**Andrew decision.** Approve / Amend / Reject.

### 3. Callable compatibility

**Current authority.** Decision 0119 fixes effect-subset substitution. Generics
are invariant and Stage 30 precedes inheritance execution.

**Question.** When may one structural function type satisfy another?

**Viable options.** Exact identity only; conventional variance immediately; or
exact Stage 30 value types with narrow capability/effect/nullability substitution.

**Tradeoffs.** Full variance before inheritance and interface conformance creates
rules with no executable consumers. Exact capability matching rejects harmless
less-demanding closures.

**Recommendation.** Require equal arity, exact parameter and return value types,
exact parameter ownership modes, and equal monomorphized generic identities in
Stage 30. Add only these safe substitutions:

- effect set: actual must be a subset of expected;
- invocation capability: an actual requiring less access may fit a position that
  provides more (`readonly` fits writable or taking; writable fits taking; the
  reverse directions fail);
- nullability: non-null fits nullable; nullable never fits non-null.

Accepted effect example:

```doria
function(string): Record $load = $nonthrowingLoader;
```

when the expected type allows `throws StorageError`. Rejected: assigning a
loader that throws `StorageError` to a nonthrowing function type.

Accepted invocation example: a readonly closure may satisfy a writable-invoked
callback contract. Rejected: a writable closure cannot satisfy a readonly-only
contract, because the caller may provide only shared access.

Accepted nullability example: `function(int): int` may flow into
`?function(int): int`. Rejected: the reverse without narrowing.

Parameter ownership remains exact in Stage 30. For example,
`function(Payload): void` does not substitute for
`function(take Payload): void`, in either direction. Future inheritance work may
reopen value-type variance without changing these capability laws.

**Consequences.** Compatibility stays small, backend-independent, and auditable.

**Invalidated elsewhere.** Assignment/call compatibility, generic inference,
diagnostic rendering, and language-server signature help need these rules.

**Andrew decision.** Approve / Amend / Reject.

### 4. Closure-expression typing

**Current authority.** Parameters are explicitly typed; arrows infer returns;
anonymous block closures require a return annotation; defaults and generic
closure declarations are not accepted.

**Question.** How does a closure expression acquire complete type identity?

**Viable options.** Infer everything; require effect annotations; or combine
written parameter/block-return syntax with inferred return/effect/invocation.

**Tradeoffs.** Inferring parameter types violates a permanent rule. Requiring
effects on every anonymous expression adds noise where a contextual type already
states the contract.

**Recommendation.** Keep explicit parameter types mandatory. Infer arrow return
types from all reachable expression paths. Keep the block return annotation
mandatory. Infer checked effects after local catches and infer invocation mode
from capture use. An expected function type constrains, but never supplies,
parameter types. It may validate the inferred return, mode, and effect subset.

Do not add closure-expression `throws` syntax. Do not add parameter defaults,
variadics, closure-local generic declarations, or implicit `mixed`. Enclosing
generic parameters are in scope and specialize normally. Nested closures are
typed inside-out after binding resolution. A closure cannot refer to its own
binding in its initializer; recursive closure values are deferred, while named
function recursion remains available.

**Consequences.** Function types are complete without making expression syntax
duplicate inferred facts.

**Invalidated elsewhere.** Semantic inference, return analysis, checked-effects
collection, and hover display need closure-aware routes.

**Andrew decision.** Approve / Amend / Reject.

### 5. Closure invocation grammar

**Current authority.** No accepted rule limits callable values to variable
positions. The parser currently rejects non-identifier postfix calls.

**Question.** Which callable expressions may be invoked?

**Viable options.** Variable calls only; variables and grouped calls; or uniform
postfix invocation for every callable-typed expression.

**Tradeoffs.** Narrow forms reduce the first parser change but create arbitrary
restrictions and later AST churn. Uniform postfix invocation matches existing
member/index postfix composition.

**Recommendation.** Accept all of these in Stage 30:

```doria
$callback($value);
($factory())($value);
$getCallback()($value);
$object->callbackProperty($value);
$list[0]($value);
```

Represent callable-value invocation separately from a named free-function call.
Evaluate the callee once, then arguments left to right. Function-value calls are
positional because structural function types have no parameter names. They have
no defaults. The call contributes the function type's effects and enforces its
invocation mode against the callee place. A nullable callable must be narrowed
before invocation; Stage 30 adds no null-safe callable-call spelling.

For `$object->callbackProperty(...)`, semantic member resolution distinguishes a
callable property from a method using the existing single member namespace. The
diagnostic labels the callee and complete argument span.

**Consequences.** One postfix AST form supports factories, properties, and
collections without special cases.

**Invalidated elsewhere.** Parser postfix handling, AST calls, argument binding,
semantic resolution, HIR/MIR, editor scopes, and signature help need updates in
the grammar and implementation slices.

**Andrew decision.** Approve / Amend / Reject.

### 6. `$this` capture

**Current authority.** Decision 0120 leaves receiver capture open. Method
receivers are readonly by default and writable only on writable methods.

**Question.** Is `$this` implicit, explicit, or unavailable inside closures?

**Viable options.** Implicit method receiver; explicit `with`; or prohibit it in
v1.

**Tradeoffs.** Implicit behavior is concise but contradicts visible environment
dependencies and makes PHP host behavior tempting. Prohibition prevents ordinary
method-local callbacks. Explicit capture is consistent and checkable.

**Recommendation.** Require `with ($this)` for readonly receiver access and
`with (writable $this)` for mutation or writable method calls. Reject
`with (take $this)` in v1 because a method receives a borrow, not ownership of
its receiver. The current capture grammar already accepts all three spellings;
this is semantic validation, not new grammar.

Readonly methods can create readonly-receiver closures. Writable methods can
create readonly or writable-receiver closures. Static methods have no `$this`.
Each nested closure must list and receive `$this` through every intermediate
environment. Readonly property access keeps readonly invocation; writable
property or method use requires writable capture and writable invocation.

A borrowed receiver closure cannot outlive the receiver. Returning it, storing
it in a longer-lived aggregate, or storing it on the same receiver is rejected.
The latter would create a self-borrow cycle. Receiver ownership transfer and
owned self-capturing closures are deferred. PHP lowering must use a backend
temporary in a static host closure rather than inheriting PHP's implicit `$this`.

**Consequences.** `$this` follows the same visible dependency and borrow rules as
locals without reopening ordinary explicit capture.

**Invalidated elsewhere.** Existing parser fixtures need semantic cases for the
already-parseable forms; LSP fixes and PHP lowering need explicit receiver facts.

**Andrew decision.** Approve / Amend / Reject.

### 7. Free-variable discovery

**Current authority.** Decision 0120 lists lexical bindings that require capture
and static symbols that do not. Decision 0093 already uses binding identities for
flow facts.

**Question.** How are free variables found under shadowing and nesting?

**Viable options.** Name scanning; resolved binding IDs; or runtime environment
discovery.

**Tradeoffs.** Names fail under shadowing. Runtime discovery violates static
layout and reflection rules. Binding IDs reuse established compiler structure.

**Recommendation.** Resolve lexical names to stable semantic binding IDs before
capture validation. Walk parameters, locals, nested blocks, loop bindings,
pattern bindings, catches, `given`, interpolation expressions, and match guards.
Constants, statics, top-level functions, type names, and enum cases resolve as
static symbols and are not captures.

Nested closure bodies are separate lexical roots. An inner reference to an
ancestor creates an explicit dependency on the inner closure and every
intermediate closure. In:

```doria
let $base = 10;

let $outer = fn(int $left) with ($base) =>
    fn(int $right) with ($left, $base) =>
        $left + $right + $base;
```

the outer closure captures `$base`; the inner captures the outer parameter
`$left` and the carried `$base`. The inner never points directly into a
grandparent stack frame.

**Consequences.** Shadowing is correct, environments are explicit, and lifetime
checking can follow one parent at a time.

**Invalidated elsewhere.** Symbol resolution must expose binding IDs and nested
environment provenance to ownership, diagnostics, HIR, and LSP semantic tokens.

**Andrew decision.** Approve / Amend / Reject.

### 8. Capture-list validation

**Current authority.** Three capture modes and explicit local capture are
accepted; the complete validation algorithm is deferred.

**Question.** Which capture-list mistakes are errors, warnings, or accepted?

**Viable options.** Treat the list as advisory; require exact correspondence; or
allow extras while rejecting unsafe modes.

**Tradeoffs.** Advisory capture defeats the feature. Exact errors for unused
entries make refactoring noisy. Warning on unused retains clarity without
blocking builds.

**Recommendation.** Apply these rules:

- missing capture: error;
- duplicate capture: error even when modes match;
- undeclared binding: error;
- own parameter, own local, constant, static, type, or enum case: error because
  it is not capturable;
- wrong mode: error with source/use labels;
- writable from readonly source: error;
- take from a borrowed Move source: error;
- take of Copy: accepted, copies into an independently owned environment and
  leaves the source usable;
- unused capture: warning, with removal fix when source trivia is preserved;
- parameter shadowing a capture: error on the capture, because the body resolves
  to the parameter;
- capture after source move or capture whose borrow cannot live long enough:
  ordinary ownership/lifetime error with capture context.

Capture entries acquire in source order. Source order affects acquisition,
diagnostic ordering, and reverse destruction, but not name lookup or type
identity.

**Consequences.** The written list is complete and deterministic without making
harmless stale entries fatal.

**Invalidated elsewhere.** Diagnostic catalogue, structured fixes, ownership
events, and source-preserving AST consumers need dedicated capture facts.

**Andrew decision.** Approve / Amend / Reject.

### 9. Missing-capture diagnostics and fixes

**Current authority.** Decision 0120 fixes the title direction "Closure Must
Capture `$minimum`" and safe insertion principles; no code is allocated yet.

**Question.** How are multiple omissions grouped and repaired safely?

**Viable options.** One grouped diagnostic; one diagnostic per use; or one per
binding/cause.

**Tradeoffs.** Per-use floods. One group obscures ownership mode and edit spans.
One per binding gives stable cause grouping.

**Recommendation.** Allocate the code only in the implementation decision/beat.
Emit one diagnostic per missing binding, ordered by first use. Group repeated
uses by binding ID. Use the closure as primary label and the declaration/use as
secondary labels. Explain that parameters come from the caller while captures
come from the surrounding scope.

Insert `with ($name)` when there is no list and only readonly use exists. Extend
an existing list token-aware, preserving comments, multiline layout, trailing
commas, and source ordering. Insert `writable` only when mutation and source
writability make it unambiguous. Never machine-insert `take`; ownership transfer
requires review even when it would make an escape compile. Prevent duplicates.
Nested closures diagnose the nearest missing environment link. `$this` uses the
same policy.

**Consequences.** Fixes are useful without silently changing ownership.

**Invalidated elsewhere.** LSP code actions must consume compiler edits rather
than reconstruct lists; editor-only diagnostics remain prohibited.

**Andrew decision.** Approve / Amend / Reject.

### 10. Capture acquisition and lifetime

**Current authority.** Decisions 0081, 0083, 0089, and 0120 fix creation-time
borrow/move intent, NLL, and reverse cleanup.

**Question.** When do captures take effect and when do they end?

**Viable options.** First invocation; closure creation; or backend-dependent
lazy capture.

**Tradeoffs.** Lazy capture changes use-after-move and alias behavior based on
whether a closure is called. Creation-time acquisition is visible and stable.

**Recommendation.** Allocate storage first, then acquire captures left to right
at closure creation. Readonly and writable borrows begin then. Taking capture
moves then. No capture is delayed to invocation. Captures drop in reverse
acquisition order when the closure environment is destroyed. Nonescaping borrow
extent ends at the closure value's final use/drop, not necessarily block end.

Moving a closure transfers one cleanup obligation. A never-invoked closure still
drops owned captures. Repeatable calls do not reacquire captures. A consuming
call owns the environment and destroys any remaining captures exactly once after
normal return or checked propagation. Existing structured cleanup applies to
checked errors. Fatal panic performs no cleanup. Creation has no checked effect;
allocation failure is the existing fatal allocation panic.

**Consequences.** Source use-after-move and borrow conflicts do not depend on
backend or call count.

**Invalidated elsewhere.** Ownership event ordering, drop elaboration, MIR
validation, interpreter cleanup, and both native backends need closure-specific
coverage.

**Andrew decision.** Approve / Amend / Reject.

### 11. Escape checking

**Current authority.** Borrowed values cannot outlive owners; `take` transfers
ownership. Parameters already encode readonly, writable, or taking behavior.

**Question.** Is a separate `nonescaping` marker necessary?

**Viable options.** Assume all callbacks escape; assume none escape; add a marker;
or derive escape permission from parameter ownership.

**Tradeoffs.** The first two are unsound or unusable. A marker duplicates
ownership. Existing modes already answer whether the callee owns the value.

**Recommendation.** Do not add an escape keyword. A normal function-type
parameter is a nonescaping readonly borrow. A `writable` function-type parameter
is a nonescaping exclusive borrow. A `take` function-type parameter transfers
the closure and permits retention, return, or aggregate storage subject to its
captured lifetimes.

Returning, assigning to a property/static, inserting into a collection, storing
in an enum payload, or boxing into `mixed` is an escape. Borrow-capturing closures
may do so only when the destination lifetime is proven within every captured
owner; function returns never satisfy a local owner's lifetime. No-capture and
take-only closures are owned and may escape. Nullable wrappers preserve the same
provenance. A closure returned by another closure follows the same rule.
`map`/`filter`/`reduce` parameters are explicitly nonescaping.

**Consequences.** Ordinary callbacks remain ergonomic and retention remains
explicit through `take`.

**Invalidated elsewhere.** Call argument ownership, return checking, aggregate
stores, `mixed` boxing, and diagnostics need closure provenance.

**Andrew decision.** Approve / Amend / Reject.

### 12. Closure checked effects

**Current authority.** Decision 0119 owns effect collection, catches, subset
substitution, checked ABI, cleanup, and source origin.

**Question.** How do closure bodies and higher-order calls participate?

**Viable options.** A second closure effect mechanism; explicit annotations only;
or infer into the existing effect set and spell contracts on function types.

**Tradeoffs.** A second mechanism duplicates cleanup. Annotation-only blocks
concise callbacks. Shared inference preserves one law.

**Recommendation.** Infer closure effects exactly like named callables after
local catches. Store the normalized set in the semantic function type while
preserving source order when an expected type spells it. Calls contribute that
set to the caller. Function types use Decision 0119 subset compatibility.

Compiler-known higher-order methods carry an effect variable bound to the actual
callback set; their specialization identity includes the normalized set. MIR
uses the existing checked status/out-slot path with a hidden environment
argument. Checked cleanup crosses closure frames through existing finalizer
regions. Creation does not allocate `Error`. Panic remains fatal and separate.
Anonymous frames use stable Doria labels from source identity, never host names.

**Consequences.** No finite Error list is hardcoded into collection algorithms,
and PHP/native/interpreter behavior remains one model.

**Invalidated elsewhere.** Checked-effect inference, function-type display,
specialization keys, HIR/MIR calls, hovers, and diagnostics need closure support.

**Andrew decision.** Approve / Amend / Reject.

### 13. Closure support matrix

**Current authority.** Closure values are Move. Existing aggregate capabilities
are explicit; no Hashable, Comparable, Cloneable, or Displayable conformance is
implicit.

**Question.** Where may function values be stored in final Stage 30?

**Viable options.** Locals only; all Move-capable positions; or a capability and
lifetime-filtered matrix.

**Tradeoffs.** Locals-only makes returns and APIs unusable. Unfiltered storage is
unsound for borrowed environments and invalid for key/sorted capabilities.

**Recommendation.** Use this normative target matrix:

| Position | Status | Reason |
| --- | --- | --- |
| Local variables | Supported | Move value; borrow provenance checked. |
| Parameters | Supported | readonly/writable are nonescaping; `take` may retain. |
| Return values | Supported | only owned/no-capture environments or otherwise proven lifetime-safe. |
| Instance properties | Supported | owned/no-capture environments; borrowed captures must outlive the owner, usually rejected. |
| Static properties | Deferred | owned Move static storage remains a separate capability gap. |
| Constants | Rejected | closure construction is not constant evaluation. |
| Parameter defaults | Rejected | closure parameters/default capture and caller-side materialization are not accepted. |
| Instance property initializers | Supported for no-capture closures | `$this` and local capture are unavailable during initializer construction. |
| Typed arrays | Supported | owned function values; exact type and cleanup. |
| `List` values | Supported | owned function values; exact type and cleanup. |
| `Dictionary` values | Supported | values need no Hashable capability. |
| `Dictionary` keys | Rejected | function values are not Hashable. |
| `Set` elements | Rejected | function values are not Hashable. |
| `SortedDictionary` values | Supported | values need no Comparable capability. |
| Sorted keys/elements | Rejected | function values are not Comparable. |
| `PriorityQueue` | Rejected | function values are not Comparable. |
| `Deque` | Supported | owned Move values with cleanup. |
| Enum payloads | Supported | recursive Copy/Move/drop classification already handles Move payloads. |
| Nullable values | Supported | null descriptor is absence. |
| `mixed` | Supported in the final representation slice | exact function-type tag, ownership, and drop glue required before use. |
| Generic arguments | Supported | invariant monomorphized types. |
| `SharedReference` payloads | Deferred | readonly family currently accepts class payloads only. |
| `WritableSharedReference` payloads | Deferred | dynamic access and borrowed environment lifetime need separate authority. |

Representation support never overrides escape checking. A supported aggregate
still rejects a closure whose borrowed environment cannot live long enough.

**Consequences.** Storage follows existing capabilities rather than special
closure exceptions.

**Invalidated elsewhere.** Type capability tables, aggregate cleanup, `mixed`
tags, collection diagnostics, and future stdlib docs need this matrix after
approval.

**Andrew decision.** Approve / Amend / Reject.

### 14. Environment representation

**Current authority.** Runtime representations are private, classes are
headerless, and concrete generic types are monomorphized. Decision 0120 requires
a zero-environment no-capture path and forbids hidden sharing.

**Question.** What fixed carrier lets many concrete environments satisfy one
structural type?

**Viable options.** Code pointer plus environment pointer; descriptor pointer plus
environment pointer; or a larger inline carrier.

**Tradeoffs.** A raw code pointer requires type/drop/debug metadata elsewhere.
Inline storage makes every closure large and still needs an overflow path.

**Recommendation.** Use two machine words: a static descriptor pointer and an
environment pointer. The descriptor contains concrete closure identity, full
structural function-type identity, invocation mode, normalized effects,
nonthrowing and/or checked invocation entries as applicable, environment
size/alignment, drop glue, and stable debug identity.

The environment layout is monomorphized from captures in source order. Owned
captures are stored by value; readonly/writable captures store compiler-private
borrow carriers retaining source provenance. Layout/alignment are private. A
no-capture closure uses a static descriptor and null environment pointer with no
allocation. Nullable absence uses a null descriptor and null environment.

Different closure expressions have different descriptors but satisfy one
structural function type through ABI-compatible entries. Interpreter values use
the same logical descriptor/environment split. Cranelift and LLVM lower the two
words directly. PHP uses a private wrapper/host closure while preserving the
same Doria identity and ownership facts. ABI validation checks descriptor/type,
entry kind, environment requirements, and drop obligations.

**Consequences.** Concrete layout stays optimized while the public type remains
structural and fixed-size.

**Invalidated elsewhere.** Semantic/MIR type systems, runtime ABI, codegen,
interpreter values, `mixed`, dumps, and validation need a closure carrier.

**Andrew decision.** Approve / Amend / Reject.

### 15. Allocation and escape optimization

**Current authority.** Correctness precedes backend convenience; no tracing GC
or pervasive ARC; allocation failure is fatal panic.

**Question.** Which allocation properties are semantic guarantees?

**Viable options.** Heap every environment; promise stack allocation; or keep
placement private while guaranteeing no-capture allocation freedom.

**Tradeoffs.** Heap-everything adds avoidable cost. Stack guarantees conflict
with escaping closures. Private placement enables optimization without changing
behavior.

**Recommendation.** Guarantee only that no-capture closures allocate no
environment. Permit stack environments for proven nonescaping closures and heap
environments for escaping owned closures. Allow scalar replacement, closure
elimination, direct-call devirtualization, and callback inlining when observable
acquisition, effects, panic, and destruction remain unchanged. Do not promise
all closures are allocation-free and do not require heap allocation merely
because a closure exists.

Allocation failure uses the existing fatal allocation panic. Benchmark compile
time, peak memory, allocation count, artifact size, and runtime separately.

**Consequences.** Interpreter, Cranelift, and LLVM may optimize differently but
must preserve one semantic result.

**Invalidated elsewhere.** Escape analysis, runtime allocation APIs, optimizer
reports, and performance manifests need explicit closure categories.

**Andrew decision.** Approve / Amend / Reject.

### 16. Invocation ABI

**Current authority.** Named calls use shared typed MIR; checked calls use
status/out slots; native unwinding is prohibited.

**Question.** How does indirect closure invocation reuse those ABIs?

**Viable options.** A separate boxed-result convention; host exceptions; or the
existing callable ABI plus a hidden environment argument.

**Tradeoffs.** Separate transport duplicates validation and cleanup. A hidden
environment argument is conventional and backend-neutral.

**Recommendation.** Invoke the descriptor entry with environment pointer first,
then ordinary source arguments. Nonthrowing calls use existing return handling
for `void`, Copy scalars, nullable values, Move pointers, and aggregates. Checked
calls use the existing status and out slots plus the hidden environment. Generic
specializations receive one compatible entry per concrete function type.

Writable calls receive exclusive environment access. Consuming calls take the
carrier and leave it moved; remaining captures drop exactly once on all normal
and checked exits. Interpreter, Cranelift, LLVM, and PHP must pass the same MIR
validation. Malformed MIR rejects mode, effect, signature, environment, or cleanup
mismatches before backend emission.

**Consequences.** Closure calls reuse checked cleanup and aggregate return law.

**Invalidated elsewhere.** MIR call operands, validator invariants, backend
indirect-call code, descriptor generation, and object tests need extensions.

**Andrew decision.** Approve / Amend / Reject.

### 17. PHP compatibility

**Current authority.** PHP output implements Doria semantics and may use private
helpers; PHP references do not define Doria borrowing.

**Question.** How can host closures preserve explicit capture and ownership?

**Viable options.** PHP arrow functions; direct `use` lowering; or generated
private wrappers/cells selected from Doria capture facts.

**Tradeoffs.** Arrow functions auto-capture by value. Raw PHP references leak
host alias behavior. Private wrappers can enforce the checked source model.

**Recommendation.** Emit static PHP closures or backend-private callable wrappers
with an explicit generated capture list. Readonly Copy captures use values;
readonly Move captures use private non-mutable handles under static Doria checks;
writable captures use a backend-private cell/reference that cannot escape into
user surface; taking captures move into a private temporary and invalidate the
source in generated ownership bookkeeping. No-capture closures are static.

Closure values may be passed, returned, nested, and invoked through the same
checked function type. Checked effects use the existing PHP checked-error
transport. Taking-capture source reuse remains a compile error. PHP destruction
must release owned captures once in reverse acquisition order. PHP helper names
and `Closure` internals are unobservable.

**Consequences.** PHP remains a compatibility backend and cannot broaden aliasing
or capture behavior.

**Invalidated elsewhere.** PHP helper namespace, codegen tests, ownership
bookkeeping, checked wrappers, and generated fixture snapshots need closure work.

**Andrew decision.** Approve / Amend / Reject.

### 18. `List<T>::map`

**Current authority.** Decision 0100 assigns `map` to Stage 30; list reads borrow
elements and ingestion moves results.

**Question.** Does mapping preserve the source and support Move elements/results?

**Viable options.** Copy elements; consume source; or borrow elements and own
results.

**Tradeoffs.** Copying Move elements is invalid. Consuming source is unnecessary.
Borrowing inputs preserves the list and supports every T.

**Recommendation.** Proposed compiler-known contract notation:

```doria
function map<U>(
    function I(T): U $transform,
): List<U> throws effects($transform)
```

`I` and `effects(...)` describe compiler-known semantic variables, not new user
syntax. `I` is readonly or writable repeatable; consuming callbacks are rejected.
The callback parameter is borrowed for the call and cannot escape. A readonly
actual callback is borrowed readonly; a writable actual callback is borrowed
writable and therefore requires a writable callback place or owned temporary.

The receiver is readonly. Each source element is lent readonly in insertion
order. The callback returns an owned or Copy `U`, moved into a newly allocated
`List<U>`. Source count and order are preserved in the result. Move T and Move U
are supported without cloning. On checked failure, already-produced U values and
the result list clean up in reverse order; the source remains unchanged. Panic
uses existing abort behavior.

**Consequences.** Writable captures are usable through writable callback
invocation without making the list writable or consuming it.

**Invalidated elsewhere.** Collection member signatures, generic/effect
specialization, MIR loops, cleanup, runtime growth, and all backend fixtures need
the contract.

**Andrew decision.** Approve / Amend / Reject.

### 19. `List<T>::filter`

**Current authority.** Filtering is assigned to Stage 30, but preserving a Move
element requires cloning or consuming ownership that no accepted contract grants.

**Question.** What does Stage 30 do for Move elements?

**Viable options.** Copy-only preserving filter; consume the list; return a view;
or defer all Move filtering while supporting Copy.

**Tradeoffs.** Consuming changes ordinary readonly collection use. A view depends
on Stage 35 iteration. Hidden clone is forbidden. Copy-only is honest and useful.

**Recommendation.** Proposed contract notation:

```doria
function filter(
    function I(T): bool $predicate,
): List<T> throws effects($predicate)
```

The receiver and elements are readonly; callback borrowing, invocation modes,
escape, order, and effects match `map`. Stage 30 accepts this only when T is Copy.
It preserves the source, evaluates in insertion order, and copies each selected
value into a new ordered list. Empty input returns an empty list. On checked
failure the partial result cleans up; source remains unchanged.

Move-element preserving filter remains deferred until Stage 35 `Cloneable` can
widen the same contract. A separately named consuming filter may be proposed
later, but Stage 30 does not invent it or a borrowed view.

**Consequences.** No Move value is silently cloned and no receiver changes
ownership unexpectedly.

**Invalidated elsewhere.** Diagnostics must name the Copy/Cloneable boundary;
future Stage 35 work may widen, not replace, this contract.

**Andrew decision.** Approve / Amend / Reject.

### 20. `List<T>::reduce`

**Current authority.** Collection elements can be borrowed and Move values can be
transferred explicitly.

**Question.** How does one accumulator work for Copy and Move types?

**Viable options.** Copy accumulator; writable borrowed accumulator; or move the
accumulator through each callback result.

**Tradeoffs.** Copy-only is weak. Writable in-place mutation requires a separate
result convention. Move-through-result is uniform and explicit.

**Recommendation.** Proposed contract notation:

```doria
function reduce<A>(
    take A $initial,
    function I(take A, T): A $reducer,
): A throws effects($reducer)
```

The list is preserved. The initial accumulator moves into the operation. Each
iteration, in insertion order, moves the current accumulator into the callback,
lends the element readonly, and receives the next owned accumulator. Empty input
returns the initial value. The callback is nonescaping and repeatable; I may be
readonly or writable. On checked failure, ownership already transferred to the
callback is cleaned by its propagation path, and any returned/current accumulator
is dropped exactly once.

Copy example:

```doria
int $total = $numbers->reduce(
    0,
    fn(take int $sum, int $value) => $sum + $value,
);
```

Move example:

```doria
List<string> $joined = $parts->reduce(
    [],
    function (take List<string> $result, string $part): List<string> {
        $result->add($part);
        return $result;
    },
);
```

**Consequences.** One rule handles Copy and Move accumulators with no clone or
special empty-list behavior.

**Invalidated elsewhere.** Generic inference, ownership transfer, callback ABI,
cleanup, and collection diagnostics need the accumulator contract.

**Andrew decision.** Approve / Amend / Reject.

### 21. Higher-order effect propagation

**Current authority.** Effects are source-ordered semantic sets and substitution
uses subset law.

**Question.** How do generic built-ins declare effects determined by callbacks?

**Viable options.** Hardcode Error types; erase callback effects; or bind a
semantic effect variable from the function type.

**Tradeoffs.** Hardcoding is incomplete. Erasure is unsound. Effect variables are
new internally but require no user syntax.

**Recommendation.** Give each compiler-known algorithm an internal effect
parameter E unified with the callback's normalized effect set. Method effect is
exactly E. E participates in specialization identity, call diagnostics, MIR
checked signatures, PHP wrappers, and backend entries. Hovers render the concrete
specialized `throws` list; generic documentation says "throws the callback's
checked effects." No finite Error family is embedded in collection code.

**Consequences.** A nonthrowing callback yields a nonthrowing algorithm call; a
throwing callback is caught or declared normally.

**Invalidated elsewhere.** Generic specialization keys and built-in signature
metadata must carry effect variables.

**Andrew decision.** Approve / Amend / Reject.

### 22. Scope boundary for collection algorithms

**Current authority.** Decision 0120 explicitly found no accepted Stage 30 grant
beyond `List<T>`.

**Question.** Should apparent Decision 0113 matrix entries widen the stage?

**Viable options.** Implement algorithms across the collection family; or treat
the matrix as stale against later, explicit Decision 0120 scope.

**Tradeoffs.** Broadening requires distinct key/value, ordering, heap, and view
contracts not designed here.

**Recommendation.** Stage 30 adds `map`, `filter`, and `reduce` only to `List<T>`.
Do not add them to Dictionary, SortedDictionary, Set, SortedSet, PriorityQueue,
Deque, typed arrays, Iterable, or Iterator. Reconcile Decision 0113's broad matrix
in the later accepted Stage 30 decision rather than silently treating it as a
grant.

**Consequences.** The first closure algorithms have one ordered sequence and one
element ownership model.

**Invalidated elsewhere.** Decision 0113's support matrix row, stdlib reference,
and any completion tables implying all seven named collections receive Stage 30
algorithms require an authority amendment after approval.

**Andrew decision.** Approve / Amend / Reject.

### 23. Runtime diagnostics and call paths

**Current authority.** Decision 0109 requires compiler-owned facts and Doria-only
call paths; host symbols and addresses are forbidden.

**Question.** How are anonymous frames identified?

**Viable options.** Generated numeric symbols; raw addresses; or source-owned
labels.

**Tradeoffs.** Host identities leak implementation. Pure ordinals become unstable
when unrelated closures are inserted.

**Recommendation.** Use an internal stable ClosureId derived from package/source
identity and expression span. Human call paths display `closure at
path:line:column`, optionally preceded by the containing Doria callable. MIR dumps
may show the compiler ID plus source label. Panic, checked-origin, validation, and
debug output use the same catalogue facts. Never print Rust, LLVM, PHP-helper
symbols, or addresses.

**Consequences.** Anonymous frames remain actionable and backend-identical.

**Invalidated elsewhere.** Diagnostic catalogue generation, frame descriptors,
runtime outcome transport, PHP wrappers, and snapshots need closure labels.

**Andrew decision.** Approve / Amend / Reject.

### 24. E0641 retirement

**Current authority.** E0641 is the one development boundary for every accepted
closure expression and function type.

**Question.** How is it removed without hiding partially implemented semantics?

**Viable options.** Remove it at semantic start; keep it until all Stage 30 work;
or narrow it slice by slice after real routes exist.

**Tradeoffs.** Early removal sends unsupported forms to lowering failures. Late
global retention hides useful semantic diagnostics and type-only support.

**Recommendation.** Retire by route:

1. Grammar completion keeps E0641 unchanged.
2. Semantic function types stop E0641 in type-only contexts once complete;
   closure expressions first report capture/type/effect errors, then one narrowed
   execution boundary.
3. Ownership/escape checking replaces generic boundaries for invalid closure
   values; valid values retain the execution boundary.
4. MIR/interpreter support removes E0641 from supported expressions and calls;
   target-specific unsupported storage forms use precise diagnostics.
5. Native/PHP/algorithm slices remove their bounded target diagnostics.
6. Stage 30 closure marks E0641 historical/unreachable only after every accepted
   grammar form has an intentional semantic route.

Language-server tests track each transition from one generic boundary to
structured semantic diagnostics and finally to no false diagnostic.

**Consequences.** No accepted form falls through to an internal lowering error.

**Invalidated elsewhere.** Compiler/LSP snapshots, diagnostic catalogue status,
current-pipeline notes, and editor README wording change incrementally.

**Andrew decision.** Approve / Amend / Reject.

### 25. Grammar-completion requirements

**Current authority.** The pre-Stage-30 slice accepts base closure/function-type
syntax. The two-clocks rule requires all additional syntax before semantics.

**Question.** Which recommended forms are not parsed today?

**Viable options.** Hide grammar in semantic slices; reduce the model to existing
syntax; or run one bounded completion slice.

**Tradeoffs.** Hidden grammar repeats the deferral Decision 0120 corrected.

**Recommendation.** Schedule **Stage 30a: Callable Grammar Completion** for:

- function-type parameter ownership modes;
- function-type invocation modes after `function`;
- function-type checked effects after the return type;
- a distinct postfix callable-expression invocation AST covering variable and
  arbitrary callable expressions.

No closure-expression effect annotation is recommended. `$this` capture already
parses and needs semantic fixtures, not grammar. Existing closure parameters
already carry ownership modes. Grammar recovery must distinguish outer parameter
ownership from function invocation mode and preserve nested function types.

**Consequences.** Every accepted spelling exists before semantic implementation.

**Invalidated elsewhere.** Parser fixtures, shared editor fixtures, VS Code,
IntelliJ, and language-server no-false-diagnostic tests need same-beat updates
after authority acceptance.

**Andrew decision.** Approve / Amend / Reject.

### 26. Implementation slicing

**Current authority.** Stage 30 owns semantics through cross-repository activation,
but no implementation slices are accepted.

**Question.** What sequence minimizes semantic reversal and backend drift?

**Viable options.** One change; backend-first prototypes; or dependency-ordered
slices around shared semantic/MIR boundaries.

**Tradeoffs.** One change is hard to review. Backend-first work lets one backend
define semantics. Shared boundaries keep correction local.

**Recommendation.** Use these slices after an accepted decision:

1. **Authority Acceptance.** Numbered record, normative amendments, diagnostics
   allocation plan. No execution.
2. **Stage 30a - Callable Grammar Completion.** The syntax in area 25, AST spans,
   recovery, compiler/LSP/editor accepted-syntax tests. E0641 remains.
3. **Stage 30b - Semantic Function Types And Captures.** Semantic TypeKind,
   structural compatibility, free-variable binding IDs, capture validation,
   `$this`, missing-capture diagnostics/fixes, effects and invocation inference.
   No HIR/MIR.
4. **Stage 30c - Ownership, Lifetime, And Escape.** Creation-time acquisition,
   NLL, parameter escape contracts, aggregate matrix, one-shot moves, drop plans.
5. **Stage 30d - Closure HIR/MIR And Interpreter Oracle.** Descriptor/environment
   MIR, indirect/checked calls, validation, cleanup, interpreter fixtures, E0641
   removed for oracle-supported forms.
6. **Stage 30e - Native Execution.** Runtime substrate plus Cranelift and LLVM in
   one parity beat, malformed-MIR tests, leak/ownership tests, portable benchmark
   structure.
7. **Stage 30f - PHP Compatibility.** Explicit host lowering and parity without
   changing Doria semantics.
8. **Stage 30g - List Algorithms.** Effect/capability-polymorphic map/filter/reduce,
   Copy filter fence, failure cleanup, all-backend parity.
9. **Stage 30h - Cross-Repository Closure.** Language-server semantic features,
   installed toolchain refresh, website/playground activation, benchmark manifest,
   E0641 historical status, Stage 30 closure audit.

Each slice has focused accepted/negative tests, shared MIR validation where
applicable, interpreter/Cranelift/LLVM/PHP parity once its backend exists, and an
explicit non-goal against later slices. A slice that discovers a semantic fork
stops at its shared boundary; backend-specific patches do not amend authority.

**Consequences.** Rollback/correction remains bounded and no temporary semantic
model needs reversal.

**Invalidated elsewhere.** The end-to-end plan and current pipeline receive these
slice names only after Andrew accepts them.

**Andrew decision.** Approve / Amend / Reject.

### 27. Performance and benchmarking

**Current authority.** Decision 0112 requires portable structure/provenance and
keeps controlled timing non-blocking with `Measurement Status: Pending Available
Runner`.

**Question.** What evidence does Stage 30 add without making hardware a gate?

**Viable options.** No benchmark work; timing-only gates; or deterministic
portable cases plus later controlled timing.

**Tradeoffs.** No evidence risks allocation regressions. Timing-only gates repeat
the unavailable-runner blocker. Portable structure catches most architectural
mistakes.

**Recommendation.** Add manifest cases for no-capture, readonly, writable, and
taking creation; direct and indirect calls; escaping/nonescaping environments;
map/filter/reduce; checked callbacks; artifact size; compile time; peak memory;
and allocation count where available. Reports record exact compiler/runtime
provenance and deterministic workload scale. Structural assertions verify no
environment allocation for no-capture closures and no mandatory heap allocation
for proven nonescaping closures.

Controlled comparative timing remains `Measurement Status: Pending Available
Runner`, is not a Stage 30 gate, and is required before public comparative claims.

**Consequences.** Performance expectations are testable without weakening the
accepted evidence policy.

**Invalidated elsewhere.** Stage 26b manifests/reports and future website claims
need closure cases after implementation; no benchmark code changes in this beat.

**Andrew decision.** Approve / Amend / Reject.

## Proposed Grammar Consequences

Subject to approval, the grammar-completion slice adds:

```doria
function(writable Counter, take Payload): string
function writable(int): int
function take(): Payload
function(string): Record throws ParseError

$callback($value)
$factory()($value)
($factory())($value)
$object->callbackProperty($value)
$callbacks[0]($value)
```

It adds no closure-expression `throws`, default parameters, variadics, generic
closure declarations, recursive self syntax, null-safe call operator, or implicit
capture. `with ($this)` already parses. The grammar must preserve this distinction:

```doria
writable function writable(int): int $callback
```

The first `writable` is a writable borrow of the closure value. The second is the
closure's environment invocation mode.

## Proposed Semantic Model

- Function types are structural, invariant in Stage 30 value types, and Move.
- Captures are resolved by binding ID and acquired at closure creation.
- Invocation mode and checked effects are inferred from the body.
- Readonly/writable captures are borrow-bound; taking captures own their fields.
- Existing parameter ownership defines nonescape versus ownership transfer.
- Every intermediate nested closure carries ancestor dependencies explicitly.
- Function values have no implicit Hashable, Comparable, Cloneable, Displayable,
  or equality capability.
- Borrowed environment returns are deferred; owned returns and existing
  argument-derived return-borrow provenance remain supported.

## Proposed ABI And Runtime Model

The closure value is `(descriptor_pointer, environment_pointer)`. A static
descriptor names the concrete closure and structural type, provides normal and
checked invocation entries, and owns environment layout/drop metadata. Captures
use one concrete monomorphized layout. No-capture values use a null environment
without allocation. Nullable absence uses a null descriptor. The compiler may
stack-allocate, heap-allocate, scalar-replace, or eliminate environments as long
as creation-time acquisition, call behavior, effects, and reverse destruction
remain identical.

Invocation prepends the hidden environment argument to existing Doria callable
ABIs. Checked calls retain Decision 0119 status/out slots and cleanup. Panic never
unwinds. The interpreter, Cranelift, LLVM, and PHP consume one validated MIR
contract.

## Proposed Collection Algorithm Surface

The following is semantic contract notation, not proposed user-declaration
syntax for effect or invocation variables:

```text
map<U>(function I(T): U callback) -> List<U> effects(callback)
filter(function I(T): bool predicate) -> List<T> effects(predicate), T: Copy
reduce<A>(take A initial, function I(take A, T): A reducer) -> A effects(reducer)
```

I is a repeatable readonly or writable invocation mode. The callback parameter
does not escape. All three preserve the source list and insertion order. Map
supports Move input and output elements through input borrowing and output moves.
Filter is Copy-only until Stage 35 `Cloneable`. Reduce moves one accumulator.

## Proposed Diagnostics

The accepted implementation decision should allocate stable identities for:

- missing, duplicate, unused, undeclared, and non-capturable entries;
- wrong capture mode and source capability;
- capture after move and borrow-bound escape;
- callable arity/type/ownership/invocation/effect mismatches;
- nullable callable invocation without narrowing;
- consuming closure reuse;
- unsupported function-value capability positions;
- Move-element preserving filter before `Cloneable`;
- precise bounded E0641 successors during implementation.

All diagnostics use Title Case, binding-ID cause grouping, source declaration and
use labels, Doria ownership words, and compiler-provided structured edits.

## Proposed Implementation Slices

The recommended order is: authority acceptance, Stage 30a grammar, Stage 30b
semantics, Stage 30c ownership/escape, Stage 30d MIR/interpreter, Stage 30e native,
Stage 30f PHP, Stage 30g List algorithms, and Stage 30h cross-repository closure.
No backend precedes shared semantic and MIR validation. Language-server/editor
changes accompany accepted grammar and semantic slices. Website/playground
activation waits until executable parity. Installed binaries refresh at every
delivered work unit.

## Performance And Memory Contract

- No-capture closure values allocate no environment.
- Nonescaping does not semantically require heap allocation.
- Escaping owned environments may allocate once; no pervasive ARC or tracing GC
  is introduced.
- Taking captures are moved, never cloned or shared implicitly.
- Captures and partial algorithm results drop exactly once in reverse order.
- Collection callbacks add no per-element wrapper allocation requirement.
- Portable structure, correctness, provenance, and deterministic reports are
  required; controlled timing remains non-blocking and pending an eligible
  runner.

## Compatibility And Tooling Consequences

After acceptance, `doria-language-server` must add structural function types,
capture binding occurrences, `$this` capture semantics, callable-call signature
help, effect/mode hovers, missing-capture code actions, and phased E0641 tests.
VS Code and IntelliJ need scopes for any newly accepted type modifiers/effects
and callable-value calls. They must not reimplement diagnostics.

The PHP backend must lower written capture modes explicitly. Website guide/API
content and playground examples activate only with the executable slice that
supports them. None of those repositories changes in this proposal beat.

## Explicit Deferrals

- Async, spawned, cross-task, Sendable, and Shareable closures.
- Capturing `$this` by ownership transfer.
- Recursive/self-referential closure values.
- Generic closure declaration syntax and variadic/default closure parameters.
- Closure-expression effect annotations.
- Returned borrows rooted in a captured environment.
- Null-safe callable invocation syntax.
- Implicit equality, hashing, ordering, display, or cloning.
- Function values in shared-reference payloads or owned static properties.
- Move-element preserving `filter` until `Cloneable`; consuming filters and views.
- Higher-order algorithms on non-List collections, typed arrays, Iterable, or
  Iterator.
- Concurrent invocation and thread-safe closure environments.

## Coherent Recommended Model

Every spelling in this section is proposed, not accepted.

```doria
class ParseError implements Error
{
    function __construct(string $message)
    {
    }
}

class Payload
{
    function __construct(string $value)
    {
    }
}

function apply(
    function(int): int $callback,
    int $value,
): int
{
    return $callback($value);
}

function retain(
    take function take(): Payload $factory,
): function take(): Payload
{
    return $factory;
}

function applyText(
    function(string): int throws ParseError $callback,
    string $text,
): int throws ParseError
{
    return $callback($text);
}

function main(): void
{
    let $minimum = 10;
    let writable $calls = 0;
    let $payload = new Payload("owned");

    let $readonly = fn(int $value) with ($minimum) =>
        $value + $minimum;

    let writable $counted = function (int $value): int
        with (writable $calls) {
        $calls += 1;
        return $value;
    };

    let $ownedFactory = function (): Payload with (take $payload) {
        return $payload;
    };

    function(int): int $typed = $readonly;
    int $answer = apply($typed, 32);

    let $throwing = function (string $text): int {
        if ($text == "") {
            throw new ParseError("empty");
        }
        return $text->length;
    };
    int $length = applyText($throwing, "ready");

    List<int> $values = [1, 2, 3];
    List<int> $mapped = $values->map($readonly);
    List<int> $filtered = $mapped->filter(
        fn(int $value) with ($minimum) => $value >= $minimum,
    );
    int $sum = $filtered->reduce(
        0,
        fn(take int $total, int $value) => $total + $value,
    );

    function take(): Payload $escaping = retain($ownedFactory);
    Payload $escapedPayload = $escaping();
}
```

The readonly capture may be called repeatedly. The writable capture makes
`$counted` writable-invoked and requires exclusive access at each call. Moving
`$payload` from the environment makes `$ownedFactory` consuming and one-shot.
`$throwing` infers `throws ParseError`; assigning it to a nonthrowing function
type is rejected, while assigning it to a type that declares `throws ParseError`
is accepted.

This attempted escape is rejected because the closure borrows a local:

```doria
function invalid(): function(): int
{
    let $value = 42;
    return fn() with ($value) => $value;
}
```

The ownership-preserving form is proposed as:

```doria
function valid(): function(): int
{
    let $value = 42;
    return fn() with (take $value) => $value;
}
```

Because `int` is Copy, taking capture places an independent owned copy in the
environment and leaves the source binding usable.

## Invalidated elsewhere

The audit found these future synchronization points. They are not changed now
because this proposal has no accepted semantic force:

- `docs/doria-end-to-end-plan.md` owns only a one-entry Stage 30 description and
  has no accepted slice sequence. It must be amended after Andrew's rulings.
- `SPEC.md` correctly states E0641 and that `$this` remains open. It must not copy
  this proposal until a numbered decision is accepted.
- Decision 0113's collection matrix labels `map`/`filter`/`reduce` as Stage 30 on
  all seven named collections, while later Decision 0120 explicitly records no
  accepted grant beyond `List<T>`. The accepted Stage 30 record must reconcile
  that stale matrix row.
- `examples/future/stage30/README.md` contains target examples but cannot yet
  state invocation capability, effects, `$this`, or exact algorithm ownership.
  It now links to this in-review proposal and remains nonexecutable.
- `crates/doriac/tests/fixtures/accepted_syntax/closures.doria` intentionally
  proves syntax, including duplicates and a nested ancestor reference without
  semantic captures. Semantic fixtures must be separate so grammar acceptance is
  not mistaken for valid Stage 30 ownership.
- `TypeRef` preserves function syntax, but `TypeKind`, `ResolvedType`, HIR, MIR,
  validation, interpreter, Cranelift, LLVM, PHP, ownership, narrowing, and checked
  effects have no closure route.
- Parser postfix calls reject closure variables and arbitrary callable
  expressions. This is a required grammar-completion item, not a semantic error.
- The language-server pin and behavior remain correct for this documentation-only
  beat. Once authority is accepted, its AST analysis, hovers, signature help,
  semantic tokens, code actions, E0641 expectations, VS Code grammar, IntelliJ
  lexer, and shared editor guard all require coordinated updates.
- The website's versioned guide, API reference, and playground closure examples
  remain future activation work after executable parity; no website work is
  scheduled from this repository proposal.
- Stage 26b benchmark infrastructure has no Stage 30 workload inventory yet, and
  there is no top-level `benchmarks/` directory. The implementation closure slice
  must add portable cases through the existing performance system.
- Self-hosted compiler planning eventually needs explicit environment and
  function-type inspection, but Rust bootstrap representation does not define it.
