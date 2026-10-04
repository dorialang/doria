# Decision 0135: Property Hooks

- **Status:** Accepted
- **Accepted:** 2026-10-02
- **Implementation Status:** Stage 36 Complete
- **Extends:** Decisions 0089, 0119, 0122, 0130, 0131, and 0134

## Context

The end-to-end plan's property-hook surface exposes computed, validated, and
cached values as properties. Stage 35 deliberately left hooks to Stage 36.
Andrew approved automatic backing storage, explicit writable accessor rules,
and full interface, trait, and inheritance integration on 2026-10-02. The
explicit-separate-backing-field-only recommendation was rejected.

Andrew approved the backing-access rules and the `borrowed get` result contract
below on 2026-10-03. Grammar recognition does not claim executable hook support.
On that date he also approved exact setter input types, inherited backing-field
reuse, transitive rejection of blocking I/O, and receiver-bound owned callbacks.
On 2026-10-04 he approved parent-then-child initializers for backed overrides.

## Accepted Contract

### Storage And Access

Hooked properties support automatic backing storage. A user must not need to
declare a second property merely to hold the hooked property's value. Computed
properties remain supported; the storage/access distinction must be explicit
in the compiler rather than reconstructed by each backend.

An ordinary `get` borrows its receiver readonly. A getter that mutates the
receiver, including filling a cache, must say `writable get`; calling it
requires writable receiver access. A getter-only property remains unassignable
even when its getter is writable.

A property with a setter must be declared `writable`. Calling its setter
requires writable receiver access. Setter parameters always have an explicit
type and use the existing parameter ownership vocabulary. Hooks do not grant
permission to store a borrowed value as an owned property or to bypass the
one-writer rule.
The setter parameter's resolved type must exactly equal the property's type;
setters do not introduce a separate assignment-conversion type.

```doria
class Temperature
{
    internal writable float $celsius = 0.0;

    writable float $fahrenheit {
        get => $this->celsius * 9.0 / 5.0 + 32.0;
        set (float $value) =>
            $this->celsius = ($value - 32.0) * 5.0 / 9.0;
    }
}
```

### Object Contracts

Interfaces may require a readable property, a writable property, or both
accessors. Traits may contribute hook implementations. Inheritance replaces
hooks only through deliberate `open` and `override` declarations, preserving
type, ownership, receiver access, and checked-error compatibility using the
same rules as methods. Ordinary stored fields do not become virtual or
hideable.
An overriding backed hook reuses the inherited backing field. Parent and child
accessors observe one stored value, not separate same-named fields.
Declaration initializers run in parent-to-child construction order, each once
per object without calling a setter. A child initializer replaces the inherited
value only after its new value is successfully evaluated; replacement then
destroys the old owned value. A failed initializer preserves the prior value
for ordinary failed-construction cleanup. An override without an initializer
keeps the inherited value.

### Effects And Evaluation

Hooks may declare checked `throws` and have side effects. They may not block or
perform async work in v1.0. This preserves the plan's accepted ORM-shaped
lazy-relation case; a property-shaped API is not a promise of purity.
The restriction includes potentially blocking output, such as `echo`, and
applies transitively through helpers, accessor calls, and callable dispatch.
Output is not exempt merely because it is synchronous. Nonblocking state may
record observations for output after the hook returns.

Property operations retain ordinary left-to-right, exactly-once evaluation.
Read-modify-write must not evaluate a receiver twice or move the getter after
the right-hand side. Checked failures and ownership use the existing call,
cleanup, and returned-borrow machinery, not a second runtime property system.

## Backing Access

The accepted backing-access rules are:

- Direct `$this->property` in that property's own hook accesses its backing
  field; access outside the hook invokes the accessor.
- A declaration initializer fills the backing field once per object without
  invoking the setter.
- A computed hook that does not refer to its own backing field has no field.

These rules govern recursion, construction, layout, and destruction. The
semantic implementation must also make accessor return provenance and transitive
nonblocking checks explicit, applying the accepted ownership/effect contract
without silently adding an exception.

### Getter Result Ownership

An interface must distinguish a getter that lends an existing Move value from
one that returns an independently owned value. Method conformance already
requires those contracts to agree; an accessor cannot erase that distinction.
`borrowed get` declares a borrowed result. An ordinary interface `get` requires
an owned result for a Move type. Concrete getter bodies retain ordinary returned-
borrow inference and must satisfy explicit result declarations and interface
contracts. Copy results retain their ordinary Copy behavior.

```doria
interface BookShelf
{
    Book $featured { borrowed get; }
}

interface BookFactory
{
    Book $created { get; }
}
```

Result ownership and receiver access are separate: `writable borrowed get` may
mutate its receiver and returns a borrowed value. `borrowed` describes the
result, not permission to mutate the returned object. Existing returned-borrow
provenance, readonly defaults, and conformance rules remain binding. The earlier
`borrow get` proposal is not accepted syntax; neither are `borrowable get` or
`lendable get`.

An ordinary getter, including an interface getter, may return a newly created
callback that borrows `$this`. The caller owns that callback's environment but
may not let it outlive the receiver. Environment ownership and retained-source
lifetime are separate facts and must survive interface and override dispatch.
By contrast, `borrowed get` returning a stored callback lends the existing
environment and transfers no cleanup obligation. Capturing `$this` in a new
callback does not by itself make that callback a borrowed getter result.

## Implementation And Closure

Stage 36 owns grammar, semantic accessor/storage facts, native and PHP
lowering, interface dispatch, trait composition, tooling, and coordinated
documentation. Recognition of hook declarations must not allow the compiler
to discard their bodies and execute them as plain stored properties.

Declaration checks require a writable property for a setter and use shared
backing-storage facts in constructor initialization analysis. Computed properties
do not require field initialization; backed properties retain those checks.
Hook bodies reuse method type, receiver, checked-effect, ownership, generic
instantiation, closure, and retained-source analysis. Explicit borrowed-result
contracts are validated after ownership analysis. Concrete accessor calls use
shared receiver, argument ownership, returned-borrow, and checked-effect checks.
Read-modify-write analysis preserves getter-before-RHS failure ownership. The
canonical interface graph retains distinct getter and setter identities and
checks their substituted callable contracts. Generic Copy results carry no
borrow, while Move results preserve provenance, including collection returns.
The shared return path distinguishes borrowed collection, enum, and shared-handle
results from owned temporaries. Newly created returned closure carriers own their
environments even when a capture borrows its source. Concrete return elision uses
resolved ownership facts rather than a class-name or collection-name shortcut;
symbolic generic parameters retain deferred Copy/Move classification.
Interface and constrained property expressions use those same callable contracts,
including setter ownership, checked errors, callable-valued getters, and inferred
return provenance. Return inference enters both owner and callable generic scopes.
Hook overrides are checked against their inherited accessor contracts after
return inference, preserving separate getter/setter roots across generic ancestors
and source declaration order. Ordinary stored fields remain non-virtual.
HIR retains accessor bodies and backing-storage identity. MIR getter, setter,
and scalar read-modify-write lowering reuse ordinary callable dispatch,
argument/result ownership, and checked-error cleanup. Component tests exercise
concrete, interface, generic, inherited, nullable, and temporary receivers through
the interpreter and linked Cranelift/LLVM executables. Compiler-owned member surfaces expose
each accessor's specialized signature, receiver access, effects, and result
provenance separately from property writability. Checked member receiver facts
also distinguish readonly, writable, and constructor-only access for tooling;
an unanalysed receiver is not implicitly writable. Constrained receiver surfaces
preserve lexical bounds and use the same compatible intersection selection as
calls; tooling does not reconstruct generic constraints. PHP emits collision-safe ordinary
accessor methods and shares call-argument and full-expression cleanup machinery;
its existing numeric and shared-ownership capability boundaries remain in force.
The approved setter input-type and inherited-backing rules, separate
stored-callback ownership and captured-source lifetimes, and transitive
blocking/async checks have regression coverage. The blanket E0764 execution
rejection is removed; hook execution tests use the public checked compiler
pipeline. Parent-then-child initializer regressions cover independent instances,
generic and multilevel overrides, replacement ownership, and failed construction
phases. Canonical workspace and LLVM validation, the 389-example durable native
parity matrix, and Linux/macOS/Windows CI pass. Native ownership leak checks pass.
Coordinated tooling preserves accessor contracts, source navigation, and receiver
facts; the website's eleven hook examples pass Check and Run through its managed
installed compiler. Both installed tools report the same compiler revision, and
native execution succeeds after reclaiming the installation build cache.

Closure requires the computed Temperature case, backed validation, writable
caching, checked failures, ownership/cleanup, interface and override dispatch,
trait composition, and exactly-once read-modify-write to agree across the MIR
interpreter, Cranelift, and LLVM. PHP executes the same hook contracts within
its existing numeric compatibility boundaries; canonical float formatting and
checked integer arithmetic still produce B1301, not host-semantic substitutes.
Negative cases cover readonly access,
missing accessors, incompatible contracts, and forbidden blocking/async work.

## Invalidated Elsewhere

- The plan's Temperature declaration needs `writable` when it has a setter.
- SPEC and API guidance must distinguish accepted hooks from implementation
  status and must not prohibit all throwing properties.
- Stage 35's method-only interface boundary is historical once Stage 36's
  accessor conformance lands; its stored `Error::message` exception remains
  separately defined.
- Compiler AST transforms and visitors, language-server traversal, contextual
  highlighting, hover, navigation, and diagnostics must preserve accessor
  bodies, parameter types, receiver modes, borrowed-result declarations, and
  checked-error spans.
- Website versioned guide, API reference, tutorials, and playground hook
  examples require coordinated verification against these contracts. They
  must not be downgraded to conceal compiler implementation gaps.
- Guards that freeze Stage 36 as future work must track its actual completion
  rather than retaining an unrelated stage's historical absence assertion.
