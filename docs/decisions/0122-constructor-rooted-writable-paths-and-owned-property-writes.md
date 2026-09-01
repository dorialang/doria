# Decision 0122: Constructor-Rooted Writable Paths And Owned Property Writes

- **Status:** Accepted
- **Accepted:** 2026-08-21
- **Date:** 2026-08-21
- **Implementation Status:** Implemented By The Pre-Stage-30c Corrective Beat
- **Amends:** Decisions 0080, 0083, and 0090
- **Preserves:** Decisions 0081, 0089, and 0119

## Context

Earlier implementation stages described constructor access as direct-only and
rejected moves both into and out of owned properties. Those restrictions kept
incomplete native analysis sound, but they were broader than Doria's ownership
and writable-path model requires. A fresh object under construction can safely
traverse a definitely initialized writable property and use the ordinary
writable capability supplied by that property. Likewise, an independently owned
value can safely become an instance property's owner.

Doria rejects an operation because the compiler proves it unsafe or because
accepted authority excludes it. A temporary compiler limitation must not be
promoted into a permanent language prohibition merely because an earlier stage
lacked the analysis required to prove the operation safe.

## Decision

### Construction-root access

The direct `$this` of the declaring `__construct` has `ConstructionRoot`
access. This is distinct from readonly and writable receiver access:

- it permits direct initialization of the new object's uninitialized property;
- it permits traversal through a definitely initialized writable property;
- after that traversal, ordinary writable-path rules govern the child value;
- it does not permit an ordinary writable method call on direct `$this`;
- it does not bypass nullable narrowing or shared-access rules; and
- it never makes `writable function __construct` valid.

Every intermediate property in a nested path must be definitely initialized,
non-null at that program point, and writable. The final operation follows its
ordinary rule. A nested readonly property does not receive constructor-only
initialization privilege.

```doria
class Window
{
    writable string $title = "";
}

class Application
{
    internal writable Window $window = new Window();

    function __construct(string $initialTitle)
    {
        $this->window->title = $initialTitle;
    }
}
```

The same capability applies uniformly to nested property assignment, compound
assignment, increment and decrement, writable method calls, collection
mutation, indexed mutation, and deeper paths. An uninitialized or
maybe-initialized intermediate is rejected by the shared constructor dataflow.
Initialization in one branch is usable in that branch; post-merge use requires
initialization on every normally continuing predecessor.

### Direct initialization remains narrow

A direct simple assignment to `$this->property` may initialize an uninitialized
property exactly once on each reachable constructor path. This applies to
readonly and writable, Copy and Move, and nullable properties. Initializers and
promoted values enter the constructor body initialized.

Direct readonly initialization does not extend to nested properties, compound
assignment, repeatable bodies, aliases, or helper-mediated writes. A first
assignment to an uninitialized writable property initializes it. A later write
to that initialized writable property is replacement.

### Owned property writes

An independently owned Move value may initialize an uninitialized owning
instance property or replace an initialized writable owning property. Accepted
sources include a fresh construction, an owned local, a `take` parameter, an
owned call result, and an executable owned collection or aggregate value. A
readonly binding may be moved because ownership transfer is not reassignment.

A borrowed value cannot become a property owner. Self-moves and overlapping
source/destination transfers are rejected. General move-out from an instance
property remains separate because it would leave an object invariant hole
without an accepted take-and-replace operation.

Replacement has one observable order:

1. evaluate the destination path once;
2. evaluate and acquire the RHS once;
3. if the RHS fails with a checked Error, leave the old property unchanged;
4. after successful acquisition, install the new owner and destroy the old
   value exactly once; and
5. destroy the new property value later under Decision 0081's ordinary reverse
   destruction order.

Fatal panic remains abort-only. It does not introduce unwinding.

### Shared semantic and MIR model

Semantic analysis records whether a property write is `Initialize`, `Replace`,
or `InitializeOrReplace`. Constructor control-flow analysis derives this from
the existing uninitialized/initialized/maybe-initialized lattice. Lowering
carries the result into typed MIR; backends do not infer it again.

Shared MIR validation independently verifies target identity and type,
constructor receiver provenance, write-state transitions, writable access,
owned RHS transfer, and receiver overlap. Initialization never drops prior
storage. Replacement acquires before dropping. `InitializeOrReplace` is valid
only for a writable direct-constructor property whose incoming state is
maybe-initialized.

Compiler-private zeroed allocation remains an implementation carrier and is not
a Doria default value, a nullable property, or a source-visible initialized
state.

## Alternatives Considered

### Make constructors writable methods

Rejected. It would expose incomplete `$this`, permit direct writable-method
calls, and erase the distinction between lifecycle initialization and borrowing
an existing object.

### Keep all nested constructor paths forbidden

Rejected. It discards proven writable capability from initialized owned
properties and makes valid initialization patterns depend on helper-free
workarounds.

### Clone or share borrowed RHS values implicitly

Rejected. Doria keeps ownership transfer explicit and does not invent copying,
reference counting, or sharing to satisfy an assignment.

### Define initialization in each backend

Rejected. Source semantics and ownership must be backend-independent. Shared
semantic information and MIR validation are the authority consumed by the
interpreter, Cranelift, LLVM, and PHP compatibility lowering.

## Consequences

Constructors retain narrow lifecycle authority while gaining safe ordinary
mutation through proven writable children. Writable owned properties now uphold
their reassignment contract. Move-in and replacement execute without cloning,
leaks, double drops, or backend-specific rules. Move-out remains a separate
language-design problem.

Decision 0080's lifecycle shapes, Decision 0081's destruction order, Decision
0089's borrow authority, Decision 0090's definite-initialization lattice, and
Decision 0119's checked-failure cleanup continue to govern their respective
parts of the operation.

## Invalidated Elsewhere

- Active wording that says every nested constructor write is invalid.
- Active wording that combines move-in, replacement, and move-out as one
  unsupported property-transfer category.
- E0472 guidance that describes valid property initialization or replacement.
- Pipeline notes that list all owned-property transfer as pending.
- Tooling diagnostics that reject either motivating program.

Decision 0130 preserves these write capabilities independently in every
root-to-derived constructor phase. Inherited writable paths retain their
declaring property identity, and replacement still acquires the new value
before dropping the old one.
