# Decision 0089: Stage 21 Borrowing Rules

Status: Accepted

## Context

Decision 0083 makes classes and the first ownership-bearing values uniquely
owned, and decision 0088 settles the fluent-chaining conventions that depend on
borrow-returning self methods. Stage 21 turns Doria's existing `readonly`,
`writable`, and `take` vocabulary into the static borrow checker promised by the
memory model.

The surface must remain Doria-shaped. Users do not write borrow sigils or
lifetimes, diagnostics do not teach Rust vocabulary, and ordinary borrow checks
must not emit runtime guards. The explicit `SharedMut<T>` pressure valve is a
separate shared-ownership decision delivered with `Shared<T>` and `Weak<T>` at
Stage 25a, not by Stage 21.

## Decision

### Borrow modes

A readonly parameter or method receiver is a shared borrow. Any number of
readonly uses of the same owner may overlap.

A `writable` parameter or method receiver is an exclusive borrow. While that
exclusive use is live, no other readonly or writable use of the same owner may
overlap.

A `take` parameter receives ownership. Giving a value away cannot overlap with
any readonly or writable use of that same owner in the same operation, and the
source binding cannot be used afterward.

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

Place expressions borrow their owning root for the duration of the enclosing
operation. `$obj->field` therefore participates in the same one-writer-XOR-many
readers rule as `$obj`. Indexed places follow the same rule when collection
indexing lands.

### Returned borrows and elision

Borrow-returning functions and methods use the fixed elision rule from the plan:
a returned borrow may derive from `$this` or from exactly one borrowed parameter.
The lifetime relationship is inferred and never written in source.

Readonly self-return methods return a readonly borrow of `$this`. Writable
self-return methods return a writable borrow of `$this`; chaining is sequential,
so `$store->add($a)->add($b);` is accepted without creating two simultaneous
writers. A borrow-returning result cannot be bound as an owned `let` when the
receiver is an owned temporary that drops at the end of the statement.

### Owned temporaries

An owned rvalue, including `new X()` and a call returning an owned `T`, is an
exclusive place. It may receive `writable` and `take` calls. This narrows the
old `E0203` restriction on owned temporaries and enables `(new X())->mutate()`.

The consuming self-return convention remains deferred by decision 0088.

## Alternatives considered

### Lexical borrow scopes

Rejected. Ending every borrow at block end would reject ordinary PHP-shaped code
after the compiler can prove the earlier use is finished.

### Runtime checks for ordinary borrowing

Rejected. Ordinary `readonly`/`writable`/`take` checking is fully static and has
zero runtime cost. Runtime access checks belong only to explicit `SharedMut<T>`
when that type lands at Stage 25a.

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
borrowing; Stage 25a's `SharedMut<T>` is the named dynamic-check exception.

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
