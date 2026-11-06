# Decision 0136: Independent Owned-Property Borrowing

- **Status:** Accepted
- **Accepted:** 2026-10-07
- **Implementation Status:** In Progress
- **Amends:** Decision 0089
- **Extends:** Decisions 0110, 0121, 0122, and 0135

## Context

Decision 0110 permits pending adapters to select reusable storage owned by the
complete registered adapter. Decision 0089's whole-owner place rule prevents
borrowing its file and buffer together. Andrew approved independent borrowing
of separately owned fields across Doria, rather than an I/O-specific exception.

```doria
$this->file->prepareRead($request, $this->buffer->getWritableRegion(0, 4));
```

## Decision

Stored instance properties with distinct ownership paths may be borrowed
independently. Paths use resolved property identities, including inherited and
generic properties, not source names. This applies recursively to nested stored
properties, ordinary calls, local aliases, closures, and retained source loans.
The one-writer-XOR-many-readers rule applies to overlapping places: the same
place and its ancestors or descendants overlap; separate sibling paths do not.

A field loan retains its owning parent's lifetime. Moving, replacing, or
destroying that parent, or any containing property, conflicts with the loan.
A whole-object method receiver still borrows that whole object. Field access
does not grant writable permission through a readonly or uninitialized path.
Last-use and explicit retained-lifetime rules remain unchanged.

Computed getters and borrowed properties do not establish separate storage.
Returned loans preserve proved source paths; an opaque projection remains
conservative and cannot recover disjointness by subsequently naming a field.
Collection indices and byte ranges do not establish independent places. Two
regions of one buffer still conflict according to their source access modes.
Shared ownership retains its existing allocation and dynamic-access contracts.

Pending selection retains the physical storage path until native retirement,
even after the descriptor's last use. Group loans still protect all retained
buffers and registrations. Independent owned fields introduce neither general
borrowed class storage nor independent registration borrowing.

## Alternatives

- Whole-owner borrowing is simpler but rejects ordinary independent-field
  access and leaves the accepted adapter-owned-buffer contract incomplete.
- An adapter-only exception would duplicate ordinary borrowing rules and make
  identical ownership graphs behave differently outside I/O. It is rejected.

## Consequences

The source checker and independent MIR proof preserve physical paths separately
from lifetime owners. No runtime checks, new allocations, layout changes, or
backend-specific access rules are introduced. Recursive summaries may
conservatively widen to an ancestor; they must terminate without inventing
independent storage or an arbitrary analysis iteration limit.

**Invalidated elsewhere:** Decision 0089's whole-owner place wording; SPEC and
the end-to-end plan's place contract; retained/capture/alias/assignment checking
and MIR provenance; the standard-library pending-adapter reference; official
tooling guidance and diagnostics tests; website versioned ownership and adapter
guides, API references, and their examples. Stage 36a remains undelivered until
its complete validation and cross-repository obligations are satisfied.
