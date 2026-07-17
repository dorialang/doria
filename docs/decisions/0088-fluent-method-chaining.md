# Decision 0088: Fluent method chaining

**Status:** Accepted (design direction; readonly/writable self-return and the
owned-temporary rule belong with Stage 21, the consuming receiver is deferred —
see Sequencing). Feeds the borrowing-rules and borrow-checker decision.

## Context

Fluent interfaces — `$query->where(...)->orderBy(...)->get()`,
`$builder->add(...)->add(...)` — are pervasive in PHP, and Doria's familiarity
goal wants them. But chaining has to be defined against the ownership model, not
assumed. Three facts set the problem:

- The grammar already accepts method calls on any primary, including `new X()`
  and `(new X())`, with parentheses optional (the PHP 8.4 form). Nothing
  grammatical is missing.
- A method call binds what the method **returns**, not the receiver. So
  `let $x = $recv->m()` is only useful for chaining when `m` returns something
  chainable, and only useful for binding when it returns something bindable.
- A freshly-constructed temporary is currently a **readonly value**: calling a
  `writable` method on `new UserStore()` fails with `E0203`. There is no defined
  self-return convention, so mutation chains and construct-and-bind one-liners do
  not work today.

The returned-borrow elision rule (§3.2 / plan line 141) already permits a method
to return a borrow derived from `$this`, which is the mechanism the chaining
conventions build on. Stage 21 is implementing borrow-returning accessors now.

## Decision

Fluent chaining is supported in **three conventions, distinguished by the
receiver mode — not by a return annotation**. The return type is uniformly
`: self`; whether a call yields a readonly borrow, a writable borrow, or an owned
value is inferred from the receiver mode together with the §3.2 elision rule — a
`self` return that derives from `$this` is a borrow under a `readonly`/`writable`
receiver and owned only under a consuming receiver. This keeps borrow returns
inferred rather than annotated, exactly as §3.2 requires; there is no return-side
spelling to add.

### 1. Readonly self-return — accessor/query chains

Receiver readonly; returns a readonly borrow of `$this`. Enables read-only
chains (`$config->database()->host()`). Covered by the §3.2 elision rule; in
Stage 21's scope.

### 2. Writable self-return — in-place mutable builders

Receiver `writable`; returns a writable borrow of `$this`. Enables mutation
chains on an existing writable place: `$store->add($a)->add($b);`. The reborrow
is sequential, so the one-writer rule holds. Belongs with Stage 21.

### 3. Consuming self-return — ownership-transfer builders

Receiver consuming `$this` (owned); returns the owned `self`. Ownership flows
through the chain, so it works on temporaries and the **result is bindable**:
`let $store = new UserStore()->add($a)->add($b);`. This is the deferred third
receiver mode reserved by the Stage 20 receiver-mode note; it is also what the
DDO decision's consuming `Transaction::commit` needs. **Its declaration spelling
is deliberately not fixed here**: it must reuse the existing `take` ownership
vocabulary for consistency, but the exact surface form is decided with the
receiver-mode work, not this record — so convention 3 stays a semantic
reservation until then.

### Owned temporaries are exclusive places

A freshly-owned rvalue (`new X()`, or a call returning an owned `T`) may receive
`writable` and `take` method calls. It is provably exclusive — no other binding
aliases it — so an exclusive borrow or a move of it is sound. This narrows the
current `E0203`-on-temporary restriction and is what lets `(new X())->mutate()`
and construct-and-chain work at all. (Precedent: Rust's `String::new().push_str(...)`.)

### What binds versus what only chains

The return convention decides whether a chain's result can be stored:

- **void return:** not chainable; using it in bind position is an error.
- **borrow return (readonly or writable self):** chainable, but the result is a
  borrow — it cannot be bound to an owned `let` or outlive the receiver. So
  `let $x = new UserStore()->add(...)` fails under conventions 1–2, because the
  temporary drops at end of statement and the borrow would dangle.
- **owned return (consuming self):** moves; bindable and re-chainable.

The user-facing consequence: mutation chaining in statement position needs
convention 2; a construct-and-**bind** one-liner needs convention 3.

## Gotchas (normative — each must be diagnosed, never silent)

- A chain whose final call returns `void` or a borrow, on a temporary that is
  not otherwise bound, constructs an object, uses it, and drops it at end of
  statement — a "temporary result discarded" lint when the value is unused.
- Consuming self-return moves the receiver; using the old binding afterward is
  use-after-move, in the ordinary give-away vocabulary.
- A borrow-returning chain cannot escape the receiver's scope; the borrow
  checker rejects storing it beyond that scope.
- Within one expression, the receiver and arguments must not take conflicting
  borrows of the same place (`f($x->mutate(), $x->read())`); evaluation is
  left-to-right and the one-writer-XOR-many-readers rule applies across the whole
  expression.

## Alternatives considered

- **Readonly chaining only:** rejected — kills the mutable-builder and
  Laravel-style idiom the familiarity goal wants.
- **Keep `new` temporaries readonly, require a `let` binding first:** rejected —
  blocks `(new X())->mutate()` and construct-and-chain, which are sound and
  expected; the exclusivity that makes them safe is already provable.
- **PHP-style unrestricted `return $this` with free aliasing:** rejected —
  violates the one-writer/borrow model; Doria makes the return convention
  explicit instead of pretending every fluent method returns an aliasable self.
- **Implicit `self` return for a marked method kind:** rejected — the return
  type is always written (explicit-typing discipline); chaining reads from the
  declared type, not a hidden rule.

## Consequences

- Three explicit, greppable conventions; identity of what a chain returns is
  always in the signature.
- `(new X())->add(...)` and mutation chains become valid once owned temporaries
  are exclusive places and writable self-return lands (Stage 21).
- The construct-and-bind one-liner (`let $s = new X()->build()`) becomes valid
  when the consuming receiver lands; builder finalizers and DDO `commit` share
  that mode.
- The return-vs-receiver trap ("`$x = $x->fluent()` bound the wrong thing") is a
  diagnostic, not a silent surprise.

## Sequencing

Conventions 1 and 2 and the owned-temporary rule are **borrow-model rules that
belong with Stage 21**, which is already implementing borrow-returning accessors
and lifting the temporary native-eligibility gate (record 0083). Deciding them
now avoids building the borrow checker on a "readonly getters only, temporaries
stay readonly" assumption and reworking it later. Convention 3 (the consuming
receiver) is the deferred third receiver mode; it lands with the
builder-finalizer / DDO consuming-commit work, which also fixes its declaration
spelling, and the receiver-mode representation already reserves room for it.

## Affected components

Parser (already accepts method-on-`new`), semantic analysis (receiver modes and
temporary place-ness), the borrow checker, diagnostics, the language
specification, and the master plan.

## Invalidated elsewhere

- `E0203` applied uniformly to owned temporaries — to be narrowed so an owned,
  exclusive temporary may receive `writable` and `take` calls.
- Any assumption that fluent chaining is unsupported, or that a `new` temporary
  is permanently a readonly value.
