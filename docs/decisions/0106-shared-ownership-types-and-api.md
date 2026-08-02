# 0106 Shared ownership types and API

Status: Accepted

## Context

Doria's memory model is single ownership with move semantics and compile-time
borrow checking (D1/D2, records 0083 and 0089). That model is correct for the
overwhelming majority of code and costs nothing at runtime, but it cannot express
graphs whose lifetime is genuinely not a tree: caches, observer lists,
doubly-linked structures, and scene-graph back-references. §3.3 of the end-to-end
plan reserves an opt-in pressure valve for exactly those cases.

Decision 0005 accepted the construction spelling `shared new T(...)` in 2020-era
wording, before generics existed and before the type it produces had a name. The
plan's §3.3 later described the types themselves. Reconciling those two records
(and renaming the types to Doria's full-word canonical vocabulary) was done as a
documentation amendment; this record formalizes the resulting surface against the
real generics machinery delivered by Stage 25.

This record does not reopen or rename the accepted public surface. It states it
normatively, fixes the runtime model, and records the consequences.

## Decision

Stage 25a adds six compiler-known generic core types, each with exactly one type
argument, in two permanently disjoint families:

```text
Readonly-access family      Writable-access family
--------------------------  -----------------------------------
SharedReference<T>          WritableSharedReference<T>
WeakReference<T>            WritableWeakReference<T>
                            ReadonlySharedReferenceAccess<T>
                            WritableSharedReferenceAccess<T>
```

They are core/prelude types usable before Stage 31 namespace syntax. This record
assigns no qualified namespace path; Stage 31 does that. The superseded names
`Shared<T>`, `Weak<T>`, and `SharedMut<T>` are not retained as aliases, and Rust
vocabulary (`Rc`, `Arc`, `RefCell`, `borrowMut`, `downgrade`, `upgrade`) is not
exposed. User declarations that redefine a compiler-known type name are rejected,
per the compiler-known-names rule the prelude section already establishes.

## Surface syntax

Readonly family — construction is the `shared` modifier on `new`:

```doria
let $node = shared new Node("root");            // SharedReference<Node>
SharedReference<Node> $node = shared new Node("root");
```

`shared` is a construction modifier only. It is not a type, and it is not a
binding, parameter, property, method, or class modifier; the superseded
`shared Node $node = ...` form from 0005 is rejected. `shared new` never produces
a writable-family value, and `weak new`, `writable shared new`, and
`shared writable new` do not exist. `new SharedReference(...)` is not the
construction surface.

Writable family — construction is an ordinary ownership-taking constructor:

```doria
let $settings = new WritableSharedReference(new Settings("light"));
WritableSharedReference<Settings> $settings =
    new WritableSharedReference<Settings>(new Settings("light"));
```

The compiler-known constructor has the semantic shape
`function __construct(take T $value)`, so the Stage 23a named form
`new WritableSharedReference(value: new Settings("light"))` also binds. The one
type argument is inferred from the single constructor argument for this
compiler-known type; this is not a generalization of user-defined
generic-constructor inference. Weak and access types are not directly
constructible by user code.

Operations:

```doria
function share(): SharedReference<T>                        // on SharedReference<T>
function share(): WritableSharedReference<T>                // on WritableSharedReference<T>
function createWeakReference(): WeakReference<T>            // on SharedReference<T>
function createWeakReference(): WritableWeakReference<T>    // on WritableSharedReference<T>
function acquire(): ?SharedReference<T>                     // on WeakReference<T>
function acquire(): ?WritableSharedReference<T>             // on WritableWeakReference<T>
function acquireReadonlyAccess(): ReadonlySharedReferenceAccess<T>
function acquireWritableAccess(): WritableSharedReferenceAccess<T>
```

`share()` borrows its receiver readonly and creates another owner of the same
allocation. It is not `clone()`: `clone()` stays reserved for value duplication
through the future `Cloneable` contract, and using it to copy a handle is
rejected. `acquire()` does not consume the weak reference; it creates a new strong
ownership claim, and returns `null` once the final owning reference has been
released.

## Type semantics

Every handle and access object in this stage is a **move type**. Plain assignment
transfers the handle and never silently increments a reference count:

```doria
let $first = shared new Node("root");
let $second = $first;             // $first is moved-from
let $third = $first->share();     // explicit additional owner
```

Parameters borrow handles by default; `take` transfers one. A moved-from handle
performs no cleanup. None of these types is an implicit Copy type.

`SharedReference<T>` forwards readonly access to `T` directly — property reads,
indexed reads where `T` is indexable, readonly method calls, readonly iteration,
and ordinary temporary readonly borrows. It rejects property writes, indexed
writes, writable method calls, moving the underlying `T` out, consuming it, and
calling a `take` receiver on it. A `writable` local binding of a
`SharedReference<T>` does not make access to `T` writable; the family is
readonly-only, which is why its allocations need no access state at all.

`WritableSharedReference<T>` **never** forwards access to `T`. Access must be
acquired, because every access must be counted.

## Runtime ownership model

Shared allocations use a separate ownership/control structure. The native class
representation stays headerless (0082) and ordinary class payload layout and drop
glue are unchanged; no shared-reference metadata is inserted into ordinary class
payloads, and no backend may reinterpret a public class payload layout.

```text
Readonly family            Writable family
-------------------------  --------------------------
strong-owner count         strong-owner count
weak count / liveness      weak count / liveness
payload ownership          payload ownership
payload drop glue          payload drop glue
(no access state)          one per-allocation access state
```

Reference counts are non-atomic in Stage 25a; thread-safe variants are Phase H
work under `Sendable`/`Shareable`. `share()` increments the strong count. Creating
a weak reference increments weak ownership of the control structure. Final strong
release destroys the payload exactly once. Weak references may keep the control
structure alive after payload destruction; final weak release frees the remaining
control structure. `acquire()` succeeds only while a strong payload is alive. A
generic-class payload uses its concrete specialization's drop glue (0105).

Whether the control block and payload occupy one physical allocation or separate
internal allocations is an implementation choice, provided ordinary object payload
layout is unchanged, `shared new` exposes no source-visible intermediate owner,
final destruction and weak retention are correct, and no FFI-visible Doria object
layout is exposed. All runtime ABI symbols stay internal and versioned. Ownership
operations that can fail receive the current Doria stack frame and report through
Decision 0109's private source-aware V2 runtime-outcome transport; older V1-named
infallible lifecycle helpers remain private implementation details rather than a
public ABI promise.

## Weak-reference lifetime

A weak reference does not keep its payload alive. Strong cycles leak by design and
are documented rather than collected — Doria has no tracing GC and no pervasive
refcounting (identity, per AGENTS.md). `WeakReference<T>` is how a cycle is broken,
which is why the writable family needs `WritableWeakReference<T>`: without it a
weak reference into a writable graph could only re-acquire a readonly
`SharedReference<T>`, permanently losing the writable capability and making
cycle-breaking impossible for the parent-back-reference and observer graphs that
motivate shared ownership in the first place.

## Writable-access rules

For each writable-family allocation the runtime rule is:

```text
Any Number Of Readonly Accesses
OR
Exactly One Writable Access
Never Both
```

Readonly + readonly succeeds. Readonly + writable, writable + readonly, and
writable + writable all panic. All writable-family handles to one allocation
observe the same access state.

An access object:

1. holds one owning claim on the allocation, so it keeps the payload alive even if
   every ordinary `WritableSharedReference<T>` handle has been dropped;
2. registers its readonly or writable access for its entire lifetime;
3. releases the access deterministically when destroyed, releasing the access
   state before releasing its owning claim;
4. may be returned, stored, passed, and moved, and is **not** a borrow governed by
   returned-borrow elision (§3.2) — it is an owned move value;
5. cannot be copied, `share()`d, weakened, or directly constructed;
6. cannot cross threads in Stage 25a.

Moving an access object transfers both the owning claim and the responsibility to
release the registered access; the moved-from object is inert. Because access
objects are storable, one parked in a long-lived structure holds its access open
for that lifetime — deterministic and visible, and the caller's responsibility.
Decision 0107's standalone lexical block is the ordinary source-level way to end
that lifetime early without introducing a helper function.

`ReadonlySharedReferenceAccess<T>` permits property reads, indexed reads, readonly
method calls, and readonly iteration; it rejects writes, writable method calls,
moving out `T`, and consuming `T`. `WritableSharedReferenceAccess<T>` additionally
permits property writes, indexed writes, writable method calls, and in-place
mutation; it still rejects moving the complete underlying `T` out, calling a
consuming receiver that removes it, and leaving the allocation without a valid `T`.

The `WritableSharedReference<T>` binding itself need not be `writable` to acquire
access — the reference is not being reassigned, and access bookkeeping is
compiler/runtime-internal behavior of the core type, not a general relaxation of
writable-method rules for user classes. The returned
`WritableSharedReferenceAccess<T>` binding must be `writable` to mutate through it,
preserving the rule that the whole write path permits writing.

## Payload domains

The two families accept different payloads, because they expose payload access
differently.

**The readonly family accepts class payloads only in v1.0.** `SharedReference<T>`
and `WeakReference<T>` require `T` to resolve to a class, so `shared new T(...)`
requires `T` to be a class. Concrete `SharedReference<int>`,
`SharedReference<string>`, `SharedReference<List<int>>` and the corresponding
`WeakReference` forms are rejected, as is `shared new List<int>()`.

The reason is the forwarding model, not a general claim about sharing. The readonly
family forwards readonly member access to its payload *directly* and has no access
object to route indexed or collection operations through, so a collection or scalar
payload would have no coherent surface. `shared new` is Doria's readonly shared
**class-construction** surface; it must not become a special collection constructor
when `new List<T>()` is not part of Doria's collection vocabulary at all.

A symbolic generic declaration may carry an unresolved type-parameter payload
(`SharedReference<T>` inside a generic class); each concrete specialization must
satisfy the class-payload requirement where it is written.

This is a **v1.0 domain restriction, not a permanent language rule.** A later
readonly-sharing construction design may widen it — for example by giving the
readonly family its own access projection, or by admitting a narrower set of
compiler-known payloads. Nothing here forecloses sharing collections readonly in a
future Doria version; it declines to invent that surface now.

**The writable family carries no such restriction.** `WritableSharedReference<T>`
accepts supported owned collection move types through its ownership-taking
constructor, precisely because its access objects explicitly support member *and
indexed* forwarding:

```doria
let $values = [1, 2, 3];
let $sharedValues = new WritableSharedReference($values);

let writable $access = $sharedValues->acquireWritableAccess();
$access[0] = 10;
```

## Family disjointness

The two families are permanently disjoint in v1.0:

1. no implicit conversion between `SharedReference<T>` and
   `WritableSharedReference<T>`;
2. no explicit conversion either;
3. they can never refer to the same allocation;
4. the family is chosen at construction — `shared new T(...)` versus
   `new WritableSharedReference(new T(...))`;
5. `WeakReference<T>` acquires only `?SharedReference<T>`;
6. `WritableWeakReference<T>` acquires only `?WritableSharedReference<T>`;
7. no weak-reference conversion crosses families;
8. a readonly-family allocation carries no writable-access state;
9. a writable-family allocation has one access state shared by all its strong
   handles;
10. there is no "weaken capability" conversion from the writable family to the
    readonly family.

Assignments, arguments, returns, property initializers, and casts that cross the
boundary are rejected.

## Move/drop behavior

Handles and access objects participate in ordinary ownership, move, and
deterministic-destruction rules (0081/0083). Drop of a strong handle decrements the
strong count and destroys the payload on the final release; drop of a weak handle
decrements weak ownership and frees the control structure on the final release;
drop of an access object releases its registered access and then its owning claim.
A moved-from handle or access object performs no cleanup. Ordinary owned class
layout and drop glue are unchanged, and ordinary borrow checking continues to emit
zero runtime access checks.

Under the abort-only panic policy (0040) no cleanup runs on the panic path, so an
access-conflict panic does not release outstanding access; the process aborts.

## Diagnostics and panics

Diagnostics use Doria vocabulary — Owns, Gives, Readonly, Writable, Access, Shared
Reference — with Title Case headings, and never teach Rust terms (`Rc`, `Arc`,
`RefCell`, Mutable Borrow, Lifetime Parameter). Focused diagnostics with
machine-applicable fixits where possible cover: the superseded
`shared Type $value` declaration; `new SharedReference(...)`; direct construction
of weak and access types; the old names `Shared`, `Weak`, `SharedMut`; wrong
generic arity; owned-to-shared assignment; cross-family assignment or argument
binding; direct payload access through `WritableSharedReference<T>`; writing
through `SharedReference<T>`; writing through readonly access; moving or consuming
an access payload; calling access-acquisition members on the wrong type; calling
`share()` on a moved-from handle; and using a moved access object.

Access conflicts panic on the existing abort-only status-101 path with Title Case
messages:

```text
Cannot Acquire Writable Access While Readonly Access Is Active
Cannot Acquire Readonly Access While Writable Access Is Active
Cannot Acquire Writable Access While Writable Access Is Active
```

All three conditions retain P1501 as the stable shared-access-conflict family.
The exact condition is carried as the typed static-string runtime fact
`conflictReason`, and the human renderer uses that same value for `Why`. The
interpreter and native runtime validate the fact against this closed three-value
set; no backend owns a parallel message table.

## Member-name collision between wrapper and payload

`SharedReference<T>` owns `share()` and `createWeakReference()` while also
forwarding readonly member access to `T`. When `T` declares a member of the same
name, `$ref->share()` would be ambiguous.

The rule:

1. Compiler-known members of `SharedReference<T>` take precedence on the direct
   receiver.
2. Non-conflicting members continue to forward transparently to `T`.
3. A compiler-known readonly property `referencedValue` provides explicit access to
   the underlying `T` when a collision exists.
4. A collision does **not** make `SharedReference<T>` invalid.
5. No new operator or qualification syntax is introduced.

```doria
class Document
{
    function share(): void
    {
        // Domain operation.
    }
}

let $document = shared new Document();

let $anotherOwner = $document->share();   // SharedReference<Document>::share()
$document->referencedValue->share();      // Document::share()

echo $document->title;                    // ordinary transparent forwarding
$document->render();
```

`referencedValue` produces a **readonly place** referring to the underlying `T`. It
allocates nothing, neither copies nor moves `T`, does not change the strong or weak
count, transfers no ownership, cannot be assigned to, and cannot be used to move or
consume `T`. Writable methods and writable property/index operations through it stay
rejected, and it participates in ordinary temporary readonly borrowing. It is an
explicit disambiguation path, not a mandatory `.value` wrapper — the normal spelling
remains `$document->title`, not `$document->referencedValue->title`.

If `T` itself declares a member named `referencedValue`, it stays reachable after the
explicit projection: `$reference->referencedValue->referencedValue`, where the first
is the compiler-known projection and the second belongs to `T`.

`referencedValue` exists **only** on `SharedReference<T>`. `WeakReference<T>` has no
live payload access until `acquire()` succeeds. `WritableSharedReference<T>`
deliberately performs no direct payload forwarding, so callers acquire an access
object instead. The access-object types have no source-visible control members
competing with forwarded payload members, so they do not carry it for symmetry; that
is revisited only if a concrete collision appears.

The earlier provisional rule — wrapper wins and a payload collision is an error — was
rejected: it would reserve ordinary domain names such as `share()` across every type
placed inside a `SharedReference<T>`, making legitimate payload APIs unreachable.
This rule keeps both reachable, never silently misroutes a call, and spends no new
syntax.

## Alternatives considered

**Payload members win on collision.** Rejected: adding a `share()` method to a
class would silently change the meaning of `$ref->share()` at every shared-reference
use site — action at a distance of exactly the kind the namespace rules already
reject.

**A `.value` wrapper property on access objects.** Rejected by the accepted surface:
`$access->value->theme` is noise on every access, and the forwarding is
compiler-known and closed, not a general proxy protocol.

**One shared type with a writability flag.** Rejected: it would put access state on
every shared allocation including readonly ones, and would make the writable
capability invisible in signatures — the presence of `WritableSharedReference<T>`
in a signature is the advertisement that a dynamic check exists.

**Making handles Copy.** Rejected: implicit refcount traffic on assignment is
exactly the pervasive-refcounting model Doria's identity rejects, and it would hide
ownership changes that `share()` makes explicit.

**A static factory instead of a constructor for `WritableSharedReference<T>`.**
Rejected: an ordinary constructor expresses "takes ownership of this value"
directly, and Doria prefers the plain spelling where it works.

**Allowing writable→readonly capability weakening.** Rejected: it would let one
allocation be reachable from both families, which breaks the invariant that lets
readonly allocations carry no access state.

**Allowing collection payloads in the readonly family.** Rejected for v1.0: with no
access object, `SharedReference<List<int>>` would need either a bespoke indexed
forwarding surface on the readonly family or a `shared new List<int>()` spelling
that contradicts Doria's collection vocabulary. The writable family already answers
the shared-collection use case through its access objects. Deliberately left open
for a later readonly-sharing design rather than declared impossible.

## Implementation clarification

Stage 25a Slice 4 completes the four-slice implementation without changing this
decision's surface. Class, generic-class, typed-array,
`List<T>`, `Dictionary<K, V>`, `Set<T>`, and `Bytes` payloads execute through the
interpreter, Cranelift, and LLVM. Access objects retain their registration while
stored in parameters, returns, properties, and collection value slots. Every
cleanup path selects its release operation from the complete semantic type:
writable strong, writable weak, readonly access, and writable access are never
collapsed into one generic pointer cleanup. Access destruction releases the
registration before its strong claim.

Null-safe access acquisition preserves the access family in
`?ReadonlySharedReferenceAccess<T>` and `?WritableSharedReferenceAccess<T>`.
Presence-tested access objects forward the same class and indexed operations as
their non-null forms, and absent values acquire no registration and require no
release.

Scalar and string writable-shared payload spellings retain their accepted type
identity, but runtime construction remains behind a Stage 25a diagnostic until a
complete-value access surface is accepted. Shared handles through `mixed` remain
pending because preserving family identity and drop obligations requires explicit
mixed runtime tags.

The final integration matrix adds exact collision parity for `referencedValue`,
a leak-free weak back-reference cycle, all seven writable payload domains,
bounded strong/weak/access stress, P1501 reason parity, LSP/editor coverage, and
Playground coordination. Linux leak jobs cover both Cranelift fast and LLVM
release profiles. PHP compatibility continues to refuse these forms rather than
emulate them.

## Consequences

- Shared ownership exists as an opt-in escape hatch without changing the default
  model; ordinary code pays nothing and emits no runtime access checks.
- A second allocation shape (control structure plus payload) enters the runtime,
  with two variants distinguished by the presence of access state.
- Access forwarding introduces compiler-known place behavior — the first mechanism
  in the language where member/indexed access on one type resolves against another.
  It is deliberately closed: no reflection, no general proxies, no dynamic member
  lookup, no user-definable forwarding protocol.
- Strong cycles leak. This is documented behavior, not a defect, and the leak-check
  CI jobs deliberately exclude a cycle fixture.
- The PHP compatibility backend cannot express these semantics and emits a clear
  unsupported-feature diagnostic rather than lowering to PHP object references.

## Affected components

Lexer, parser, AST, HIR, semantic types and member resolution, ownership checker,
narrowing (nullable `acquire()` results), MIR and MIR validation, the MIR
interpreter, Cranelift and LLVM lowering, the PHP backend's unsupported
diagnostics, `doria-rt` control structures and drop machinery, examples, the
differential parity manifest, leak-check jobs, and the external
`dorialang/doria-language-server`.

## Invalidated elsewhere

- **Decision 0005's "likely explicit type form"** `shared AppConfig $config = ...`
  is superseded by `SharedReference<AppConfig> $config = ...`. 0005's historical
  body is preserved with a forward reconciliation section; this record supersedes
  that form normatively.
- **§3.2's returned-borrow elision** does not govern access objects: returning one
  returns an owned move value, not a borrow. Elision wording is unchanged but no
  longer describes every returnable reference-like value.
- **§8.3 `doria-rt`** gains the two control-structure shapes; D18's "`Shared<T>`
  refcount machinery" row was already renamed and now also covers the writable
  family's per-allocation access state.
- **The php-lib bridge (§10.3, Stage 41)** must keep using internal opaque bridge
  handles. Family disjointness is what invalidated the previous
  "rooted as `SharedReference<T>`" model, and no Stage 25a public type may be
  exposed through the bridge.
- **`docs/notes/php-true-async-rfc-observations.md` B3** already carries a forward
  amendment recording that its rooting premise is superseded; its interleaving
  hazard remains open and belongs to the bridge decision.
- **The prelude list (§6/§13 namespaces)** gains six compiler-known names that
  userland may never shadow, unlike ordinary prelude entries.
- **Phase H thread-safe variants** must preserve family disjointness rather than
  introducing a cross-family bridge, and must not retrofit atomics onto these
  non-atomic counts.
- **The language server** adds all six types and five members to completion and
  hover, and does not offer the superseded names as valid surface.
- **Stage 26 and later collection work** may store these handles in collections;
  collection element move classification already covers them, but any future
  Copy-element optimization must exclude them.
- **Native collection cleanup** must dispatch writable-family stored values by
  their exact semantic type; readonly-family weak release is never a valid common
  fallback for writable strong, writable weak, or either access-object type.
