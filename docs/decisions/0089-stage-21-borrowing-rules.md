# Decision 0089: Stage 21 Borrowing Rules

> **Stage 35 amendment:** Decision 0134 applies these ownership and provenance
> rules to two-word interface values and adds one compiler-known nonescaping
> source-loan rule for public iterators and borrowed `getCurrent()` results.
> Its Slice 3 amendment accepts promoted `borrow` constructor parameters for
> iterator source retention without permitting general borrowed class fields.

Status: Accepted

> **Interface runtime implementation:** Decision 0134 Slice 2 retains these
> rules across class/interface and ancestor-interface views. A view preserves
> its ultimate root, including a shared access lease; replacement or release of
> that root invalidates every derived view. Conversion adds no owner or retain.

## Context

Decision 0083 makes classes and the first ownership-bearing values uniquely
owned, and decision 0088 settles the fluent-chaining conventions that depend on
borrow-returning self methods. Stage 21 turns Doria's existing `readonly`,
`writable`, and `take` vocabulary into the static borrow checker promised by the
memory model.

The surface must remain Doria-shaped. Users do not write borrow sigils or
lifetimes, diagnostics do not teach Rust vocabulary, and ordinary borrow checks
must not emit runtime guards. The explicit `WritableSharedReference<T>` pressure
valve is a separate shared-ownership decision delivered with `SharedReference<T>`
and `WeakReference<T>` at Stage 25a, not by Stage 21.

## Decision

### Borrow modes

A readonly parameter or method receiver is a shared borrow. Any number of
readonly uses of the same place may overlap.

A `writable` parameter or method receiver is an exclusive borrow. While that
exclusive use is live, no other readonly or writable use of an overlapping
place is allowed. Decision 0136 defines which stored-property paths overlap.

A `take` parameter receives ownership. Giving a value away cannot overlap with
any readonly or writable use of that value or its owned fields in the same
operation, and the source binding cannot be used afterward.

### Non-lexical extent

Borrow extent is non-lexical. A borrow ends at its last required use rather than
at the end of the enclosing block. For an ordinary call, receiver and argument
borrows live while that call is being assembled and invoked, then end after the
call unless the result itself is a returned borrow.

Argument evaluation remains left to right. A borrow created by an earlier
argument remains live while later arguments are evaluated for that same call, so
`observe($value, mutate($value))` is rejected when `mutate` needs `writable`
access to the same owner. Separate statements may read and then mutate the same
owner because the first call's borrow has ended.

### Place expressions

Decision 0136 amends the original whole-owner place rule: stored owned-property
paths borrow independently when disjoint. The same place and its ancestors or
descendants still overlap. A field loan protects its parent's lifetime and
prevents moving, replacing, or destroying the parent. Indexed places, byte
ranges, and opaque computed or borrowed projections remain conservative.

### Returned borrows and elision

Borrow-returning functions and methods use the fixed elision rule from the plan:
a returned borrow may derive from `$this` or from exactly one borrowed parameter.
The lifetime relationship is inferred and never written in source.

Readonly self-return methods return a readonly borrow of `$this`. Writable
self-return methods return a writable borrow of `$this`; chaining is sequential,
so `$store->add($a)->add($b);` is accepted without creating two simultaneous
writers. A borrow-returning result cannot be bound as an owned `let` when the
receiver is an owned temporary that drops at the end of the statement.

Returned-borrow provenance is transitive. A result derived from `$this` or one
borrowed parameter retains its lifetime root and proved storage path when it passes through readonly
property projection, nested property projection, collection indexing,
`Dictionary::get`, `List::first`, `List::last`, nullable wrapping, null-safe
access, or generic substitution. These operations do not turn an element borrow
into an owned value and do not acquire a cleanup obligation. Ambiguous roots and
borrows from temporaries that die before the result remain errors.

### Inferred borrowed locals

Andrew approved proved writable borrowed locals on 2026-10-10. An inferred
`let` remains readonly by default; `let writable` may retain a proved writable
returned borrow. It does not upgrade a readonly result or acquire ownership.

```doria
let writable $stream = $set->getWritableStream($key);
$stream->read($destination);
```

This rule applies to ordinary functions, methods, getters, generic and erased
calls, and transitive aliases, not only registration lookup. A writable alias
keeps its original source in exclusive use until its last required use.
Reborrowing through that alias suspends conflicting use of the parent alias;
it does not create a second writer. Readonly aliases remain shared. Neither
kind may consume its source, initialize owning storage from a borrow, or outlive
its source. No new owner, allocation, reference count, or runtime borrow guard
is introduced. The former blanket rejection of writable borrowed locals is
removed; source checking and independent MIR checking preserve the permission
and lifetime proof instead.

### Owned temporaries

An owned rvalue, including `new X()` and a call returning an owned `T`, is an
exclusive place. It may receive `writable` and `consuming` method calls. This narrows the
old `E0203` restriction on owned temporaries and enables `(new X())->mutate()`.

The consuming self-return convention remains unimplemented; Decision 0088 fixes
its `consuming function` spelling and assigns grammar and execution to Stage 36a.

## Alternatives considered

### Lexical borrow scopes

Rejected. Ending every borrow at block end would reject ordinary PHP-shaped code
after the compiler can prove the earlier use is finished.

### Runtime checks for ordinary borrowing

Rejected. Ordinary `readonly`/`writable`/`take` checking is fully static and has
zero runtime cost. Runtime access checks belong only to explicit
`WritableSharedReference<T>` when that type lands at Stage 25a.

### Treat owned temporaries as readonly

Rejected by decision 0088. A freshly-created or freshly-returned owned value is
exclusive because nothing else can alias it.

## Consequences

The ownership checker tracks live readonly and writable uses in the same
source-level vocabulary as move diagnostics. Conflicts say that a value is
already used as readonly or writable in the current call; they do not mention
lifetimes or Rust borrow terms.

MIR and native validation remain backend-independent consumers of the same
checked ownership facts. Backends must not emit dynamic guards for ordinary
borrowing; Stage 25a's `WritableSharedReference<T>` is the named dynamic-check exception.

Decision 0090's full constructor definite-initialization slice invalidated and
removed decision 0083's temporary native-eligibility gate. Decision 0088 feeds
this record and remains authoritative for the deferred consuming self-return
convention.

## Invalidated elsewhere

- Uniform `E0203` rejection of writable calls on owned temporaries.
- Any documentation claiming `(new X())->mutate()` or readonly/writable fluent
  self-return chaining is permanently unsupported.
- `SPEC.md` supported-subset descriptions that list non-lexical borrowing as
  unsupported.
- `docs/notes/current-pipeline.md` status text that lists non-lexical borrowing
  as future Stage 21 work.
- The Stage 19 temporary native-eligibility gate, now removed by decision 0090.
- Any checker or MIR path that treats a `$this`-derived collection/property
  projection as an owned return or loses its original borrow root.
- The blanket claim that inferred borrowed locals can only be readonly;
  readonly returned borrows still cannot initialize writable aliases.
- The website's versioned readonly/writable guide and API return contracts,
  and the language server's borrowed-local descriptions and diagnostic fixtures.

Decision 0130 extends these provenance rules through implicit class upcasts and
hierarchy narrowing. Readonly and writable borrows retain their access mode and
original root; inheritance never turns a borrow into ownership.

Decision 0131 likewise keeps constructor-only and inherited-property override
parameters on ordinary parameter provenance. Their compile-time role creates no
property alias, owner, field, or cleanup fact.

Decision 0132 applies the same provenance rule to indexed sequence iteration. A
property-rooted `List<T>` is evaluated once and borrowed in place; its readonly
Copy index creates no ownership or cleanup obligation, and its value binding
retains the existing readonly or writable element-borrow mode.
