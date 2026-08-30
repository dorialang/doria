# Stage 30 Closure Authority Proposal

## Status

**Superseded By Accepted Decision 0121.** This supporting note is retained as a
historical record of the review that produced
[Decision 0121](../decisions/0121-closure-function-types-capture-semantics-and-execution-model.md).
It is not normative authority and no longer requests Approve/Amend/Reject
rulings.

## Outcome

Andrew accepted the proposal's overall structural-function-type, explicit
capture, ownership, lifetime, two-word carrier, checked-call, backend-parity,
diagnostic, and performance direction on 2026-08-19, with these amendments:

- consuming one-shot invocation is spelled `function once(...)`, not
  `function take(...)`;
- `take` remains solely the value-transfer mode for bindings, parameters, and
  captures;
- `$this` follows the explicit capture model through `with ($this)` and
  `with (writable $this)`, while taking `$this` is rejected;
- runtime descriptors are lean and non-reflective;
- logical capture order remains source order, while private physical environment
  fields may reorder to reduce padding;
- named and bound callable references remain deferred; wrapper closures are the
  Stage 30 adaptation mechanism;
- higher-order algorithms land only on `List<T>`;
- preserving `filter` is limited to Copy elements until `Cloneable`; and
- `reduce` owns one accumulator and lends it writably to a `void` callback rather
  than moving it into and out of every call.

The accepted record also fixes the dependency-ordered Stage 30a through Stage
30h implementation slices. Stages 30a through 30h and Stage 30 are complete.
E0641 is historical and reserved. Stages 31 through 33 and Phase F are complete;
Native Testing Foundation Slices 1 and 2 are complete, Slice 3 is next, and
Stage 34 waits for the foundation.

## Historical Scope

The proposal audited function-value ownership, invocation modes, function-type
parameter ownership and checked effects, callable invocation, explicit receiver
capture, lexical binding identity, capture diagnostics, lifetime and escape,
runtime representation, ABI, PHP compatibility, `List<T>` algorithms, closure
identity, E0641 retirement, implementation slicing, and performance structure.

All normative details now live only in Decision 0121. This note must not be used
to infer syntax, semantics, implementation status, or collection APIs.

## Invalidated elsewhere

- Active references that described this proposal as In Review now cite Decision
  0121 and record Stage 30a as next.
- The former `function take(...)` recommendation is rejected.
- The former move-in/move-out `reduce` callback is rejected.
- Any broad all-collection interpretation of `map`, `filter`, and `reduce` is
  rejected; Decision 0121 grants them only to `List<T>`.
