# Decision 0134: Interfaces, Traits, Core Value Contracts, And Public Iteration

- **Status:** Accepted
- **Accepted:** 2026-09-04
- **Implementation Status:** Stage 35 Authority Accepted; Slice 1 Next
- **Amends:** Decisions 0029, 0030, 0079, 0082, 0087, 0089, 0093, 0096, 0100, 0102, 0105, 0106, 0110, 0113, 0119, 0121, 0125, 0129, 0130, 0131, 0132, and 0133

## Context

Doria already commits to nominal interfaces, multiple interface inheritance,
compile-time trait composition, headerless class objects, two-word erased
interface values, monomorphized generic constraints, explicit ownership, and
checked Errors. The compiler currently implements only the compiler-known
`Displayable` and `Error` contracts and accepts incomplete interface and trait
syntax behind Stage 35 diagnostics.

This decision completes the source, ownership, ABI, iteration, and composition
contract before implementation begins. It does not accept property hooks,
default interface methods, primitive boxing, runtime traits, reflection, or
Stage 35a optimizer work.

## Interface Declarations

Interfaces are nominal, external by default, and may be `internal`:

```doria
interface Renderable
{
    function render(): string;
}

internal interface Repository<T implements Entity>
    extends Readable<T>, Writable<T>
{
    function find(string $id): ?T throws StorageError;
    writable function save(take T $entity): void throws StorageError;
    function convert<U implements Displayable>(U $value): string;
}
```

The grammar is:

```text
InterfaceDecl := Attributes? Internal? "interface" Name TypeParams?
                 ("extends" TypeRef ("," TypeRef)*)?
                 "{" InterfaceRequirement* "}"

InterfaceRequirement := Attributes? ReceiverMode? "function" Name TypeParams?
                        "(" Parameters? ")" ":" TypeRef ThrowsClause? ";"

ReceiverMode := "writable" | empty
Parameter := Attributes? ("take" | "writable")? TypeRef Variable
```

Interface names, parents, constraints, parameter and return types, and checked
Errors are source-preserving qualified `TypeRef` values. Every interface
requirement is external and terminated by a semicolon. A requirement may be
generic, may require a writable receiver, and may use the existing readonly,
`writable`, and `take` parameter modes.

Stage 35 interface requirements do not permit:

- parameter defaults;
- `internal`, `open`, or `override`;
- `static`;
- constructors or destructors;
- constants or properties;
- method bodies.

`self` is permitted only as an owned covariant return and means the exact
dynamic implementing class. It is not accepted in an interface parameter
position in Stage 35.

User interfaces remain method-only. The compiler-known `Error` contract keeps
its existing externally accessible readonly stored `string $message`
requirement as a narrow exception. Stage 35 does not decide readable properties,
setters, backing storage, accessor ownership, accessor effects, or any other
property-hook syntax. Stage 36 remains their sole owner.

## Interface Inheritance

An interface may extend one or more interface specializations. Generic
specializations are invariant and have distinct compiler identities.

Parent resolution and canonical requirement order are deterministic:

1. Traverse direct parents in authored left-to-right order.
2. Traverse each parent's canonical requirements parent-first.
3. De-duplicate the same root requirement reached through a diamond.
4. Append requirements authored by the current interface in source order.

Extending the same direct specialization twice is an error. Reaching that same
specialization through a diamond is valid and contributes one ancestor and one
copy of each root requirement. Extending different specializations of one
generic interface is rejected in Stage 35, even when their current member names
do not conflict.

Same-name inherited requirements coalesce without a child declaration only
when their fully substituted callable contracts are identical. Otherwise the
child must redeclare one contract that is substitutable for every inherited
requirement, or the inheritance is rejected. Source or hash-map order never
selects a winner.

Interface inheritance cycles are rejected with every cycle edge identified.
An external interface cannot expose an inaccessible internal parent from
another package. An interface value upcasts to any ancestor implicitly,
without allocation and without changing ownership or provenance.

## Nominal Class Conformance

A class conforms only through an authored `implements` clause or conformance
inherited from its parent class:

```doria
class Report implements Renderable
{
    function render(): string
    {
        return "report";
    }
}
```

Method-name coincidence is not conformance. A child inherits its parent's
conformances after generic substitution; redundantly repeating the same
specialization is diagnosed and may be removed. A class cannot implement two
different specializations of the same generic interface.

Inherited class methods and flattened trait methods may satisfy requirements.
A trait never grants nominal conformance: the class still authors `implements`
unless its parent already conforms. An internal method cannot satisfy an
external requirement, and static and instance methods never satisfy each
other.

Conformance is checked after generic substitution, trait flattening, and class
inheritance are known, but before HIR. Missing requirements are errors. Stage 35
does not invent abstract classes or incomplete conforming classes. Open and
closed classes follow the same conformance law; a valid child override updates
the inherited interface vtable for the child's dynamic specialization.

## Requirement Compatibility

Interface conformance reuses Decision 0130's callable substitution machinery:

- Method name, instance/static identity, parameter count, parameter names,
  parameter types, ownership modes, generic arity, and normalized generic
  constraints match exactly.
- A readonly implementation may satisfy a readonly or writable requirement. A
  writable implementation may satisfy only a writable requirement.
- A return may narrow covariantly only through the central assignment and
  ownership compatibility relation. Nullability may narrow, never widen, and a
  borrowed return preserves at least the required provenance.
- The implementation's checked Error set is a subset of the requirement's set.
  AmbientIo and TestAssertion remain automatic and cannot disappear through
  erasure.
- The satisfying implementation is externally accessible.

Interface requirements have no defaults. A concrete implementation may add
trailing defaults for concrete calls, but calls through the interface contract
still provide every interface argument. Named interface calls use the
requirement's parameter names, which is why parameter-name identity is exact.
`open` and `override` remain class-hierarchy declarations rather than interface
requirements.

## Interface Values And Ownership

An owned interface value is a Move value. Its runtime carrier is exactly:

```text
data pointer
interface vtable pointer
```

Concrete class objects retain headerless data-only payloads. Converting a class
to an interface keeps the same object allocation and constructs the two-word
carrier without allocating, copying the object, or creating another owner.

- Assignment from an owned class transfers ownership into the carrier.
- A readonly interface parameter borrows the object.
- A `writable` interface parameter creates one exclusive borrow.
- A `take` interface parameter transfers the carrier and its drop obligation.
- An unqualified interface return transfers ownership.
- A borrowed interface return is valid only when existing provenance analysis
  ties it to a live receiver or parameter.
- Interface-typed properties, supported statics, collection elements, and
  closure captures store owned carriers; borrowed carriers cannot escape into
  storage.

Upcasting an interface to an ancestor changes only the vtable word. Sibling
interface conversion and interface-to-class conversion require checked `is`
narrowing. No implicit downcast exists.

An open-class carrier converts without allocation: its data pointer is retained
and its exact class descriptor supplies the statically indexed conformance
vtable for the requested interface specialization.

Destroying an owned interface value invokes the exact dynamic class drop glue
once. Moves and nullable transport treat both carrier words as one value so no
partial move, duplicate drop, or stale vtable is possible.

## Interface Vtables And Dispatch

The compiler emits one immutable private vtable for every reachable pair of:

```text
concrete dynamic class specialization
interface specialization
```

Its canonical contents are:

1. exact dynamic class descriptor identity;
2. exact dynamic drop glue;
3. constant ancestor-interface conversion entries;
4. method entries in canonical requirement order;
5. checked ABI, receiver, or covariant-return thunks where required.

The table has no reflected method names, mutable state, public metadata, or
runtime checked-effect profile. Effects and slot ABI are compile-time facts.
Generic requirement methods are monomorphized for reachable concrete argument
tuples.

Calls preserve three distinct paths:

- an exact concrete call is direct;
- a monomorphized `T implements I` constrained call is statically selected and
  remains inlinable;
- a genuinely interface-erased call performs one constant-slot indirect call.

Runtime tests use static descriptor/conformance identities, never method-name
lookup. There is no per-object header, per-call allocation, interface wrapper
object, runtime reflection, or external implementation registry.

## Nullable, Mixed, Is, And Match

`?I` uses the two-word interface carrier with a null data pointer and canonical
null vtable word. Presence is proved from the data pointer. Both words remain
paired through control-flow joins and moves.

Concrete, open-class, interface, and `mixed` values may be tested against an
exact interface specialization. A successful test preserves the source
ownership mode and borrow provenance. Class tests on interface values and
sibling-interface tests use the same exact dynamic class descriptor. Failed
tests enter the false arm; there is no unchecked cast.

`mixed` stores one boxed interface carrier plus its exact dynamic identity and
drop facts. It does not allocate a second interface wrapper. Match type patterns
reuse the same `is` fact. Interface implementation sets are open, so listing the
currently known implementers never makes a match exhaustive.

## Core Value Interfaces

The compiler-known prelude contracts are:

```doria
interface Comparable<T>
{
    function compare(T $other): Ordering;
}

interface Equatable<T>
{
    function equals(T $other): bool;
}

interface Hashable
{
    function hash(): uint64;
}

interface Displayable
{
    function toString(): string;
}

interface Cloneable
{
    function clone(): self;
}
```

All five methods have readonly receivers and declare no checked Errors. For
Move types, `T` parameters are readonly borrows under the ordinary parameter
rule.

`Comparable<T>` supplies sorted-collection comparison and returns the core
`Ordering { Less, Equal, Greater }` enum. It does not overload ordinary class
`<`, `<=`, `>`, or `>=`, and Doria adds no general operator-overloading model.

`Equatable<T>` is explicit opt-in value equality for user classes and collection
membership. `==` and `!=` delegate to `equals` only for a statically valid
Equatable contract. Nonconforming class equality remains identity. Nullable
equality handles absence first and delegates only for two present values.

`Hashable` returns `uint64`. Equal values produce equal hashes, and a key's hash
is stable while stored. The returned value is not a persistence or wire-format
guarantee across compiler/runtime versions. Hash collections apply private
per-process keyed mixing for collision resistance. Keys are consumed and never
exposed writable, so equality/hash-participating state cannot change while the
key is stored. Shared-reference wrappers are not automatically Hashable.

`Displayable::toString` remains the only class display contract and creates no
implicit string assignment conversion, `__toString`, primitive method, or cast.
Concrete and constrained calls remain static; interface-erased display uses the
ordinary interface slot.

`Error` becomes a compiler-known nominal interface identity implemented by the
general carrier and dispatch machinery while retaining its special external
readonly stored `string $message` conformance rule. Error subinterfaces are
valid and participate in `throws`, `catch`, rethrow, and typed `toThrow`
coverage. The existing checked-error carrier is unified with the general
interface carrier; Doria has no second erased Error representation.

Primitives retain compiler-known unboxed conformance for generic constraints.
They cannot inhabit interface-typed slots. `float` remains Equatable but not
Hashable or totally Comparable.

## Cloneable

`Cloneable::clone(): self` has a readonly receiver, returns a new owned value of
the same exact dynamic class, and declares no checked Error. Implementations are
author-written; Doria does not synthesize cloning and never clones implicitly.

The language guarantees an independently owned valid result, not universal deep
copy. The implementation decides whether referenced subobjects are themselves
cloned or deliberately shared. `SharedReference::share()` creates another owner
of one object; cloning a payload duplicates the value. Shared-reference wrapper
families do not automatically implement Cloneable.

Constrained clone calls remain static. Calling through a Cloneable interface
returns an owned interface carrier retaining the exact dynamic class. Sequence
fill evaluates its source once, clones once per output slot in ascending order,
and drops the original temporary after construction. Copy values retain the
existing direct path.

Stage 35 widens the accepted preserving operations from Copy to Copy-or-
Cloneable:

- sequence fill;
- existing-source collection `::from` operations that preserve the source;
- Set and SortedSet `union`, `intersect`, and `difference`;
- `List::filter`, which preserves the source and output elements.

Consuming operations and operations whose callback creates fresh results do not
need Cloneable. Because `clone` is nonthrowing in the checked-effect system,
there is no open-ended dynamic checked Error set; allocation panic remains
abort-only under existing rules.

## Public Iteration

The public contracts are:

```doria
interface Iterable<T>
{
    function iterator(): Iterator<T>;
}

interface Iterator<T>
{
    function hasCurrent(): bool;
    function current(): T;
    writable function advance(): void;
}
```

These compiler-known interfaces carry one narrow provenance rule required by
Move values:

- `iterator()` creates an iterator carrier tied to a readonly source loan.
- The carrier is Move and may own cursor state. A carrier containing a source
  loan cannot be returned, stored, captured, or otherwise escape that loan.
- `current()` returns a readonly element borrow tied to the iterator/source
  loan, despite the ordinary `T` spelling. This is not a general hidden-borrow
  return rule.
- `hasCurrent()` distinguishes exhaustion, so nullable elements remain valid
  data. `advance()` mutates cursor state, not the source.

An iterator that owns its complete source has no borrowed-source escape
restriction. A borrowed iterator is stack-capable; the public model requires no
heap or per-iteration allocation. The compiler-known Iterator methods declare
no checked Errors, as fixed by the core signatures above. `foreach` still routes
checked exits from its body through iterator and source-loan cleanup.

`foreach` acquires one iterator, checks `hasCurrent`, borrows `current`, runs the
body, then advances. It releases the element, iterator, and source loan exactly
once on exhaustion, `break`, `continue`, `return`, or checked exit. Nested and
concurrent readonly iterators are valid; writable access to the source conflicts
with every active readonly iterator loan.

User-defined iteration is value-only in Stage 35:

```doria
foreach ($values as Value $value) {
}
```

Decision 0133's explicit binding type remains mandatory. A first binding on a
user-defined Iterable remains unsupported; Stage 35 does not invent an index or
key role. Decision 0132's sequence indexes and dictionary keys remain built-in
contracts.

The rejected `writable next(): ?T` form cannot distinguish exhaustion from a
nullable element and would make an ordinary return own a Move element, forcing
removal or cloning. Callback traversal complicates structured exits and checked
effects. Universal integer cursors impose unsuitable complexity on linked
structures. Mandatory owning or heap iterators prevent ordinary borrowed
container traversal. The accepted current/advance carrier keeps ownership and
control flow explicit in compiler facts.

## Built-In Collection Integration

Built-in arrays and collections keep their compiler-internal optimized
iteration plans and exact Decision 0132 first-binding roles. They also satisfy
the relevant compiler-known `Iterable<T>` constraint, but direct built-in
`foreach` does not allocate an adapter or invoke an erased interface call.

Deliberately erasing a built-in collection to `Iterable<T>` creates a
nonallocating compiler-known carrier backed by static vtables. Only that erased
path pays interface dispatch. Existing writable element iteration remains on
specialized built-in plans; the Stage 35 public Iterable contract is readonly.

## Shared Ownership With Interface Payloads

Stage 35 supports interface payloads across all six existing families:

```text
SharedReference<I>
WeakReference<I>
WritableSharedReference<I>
WritableWeakReference<I>
ReadonlySharedReferenceAccess<I>
WritableSharedReferenceAccess<I>
```

The shared control block retains the concrete allocation pointer, exact dynamic
class descriptor/drop, counts, and lease state. Each typed handle or access
value retains the interface vtable for its static `I`. No operation duplicates
the object, creates a heap wrapper, or creates another ownership family.

Readonly access forwards readonly requirements. Writable access invokes
writable requirements only while its exclusive lease is active. Wrapper members
continue to win over payload forwarding. Narrowing changes only the retained
interface view and preserves the same control block and lease. All families
remain invariant; no wrapper covariance is introduced. Weak acquisition returns
the matching typed owner/access carrier or null, and exact dynamic drop runs once
when the final strong owner dies.

## Trait Declarations

Traits are generic compile-time composition units, never values:

```doria
trait HasSlug
{
    string $slug = "";

    function slug(): string
    {
        return $this->slug;
    }
}

trait Serializes<T implements Displayable>
{
    function serialize(T $value): string
    {
        return "{$value}";
    }
}
```

The grammar is:

```text
TraitDecl := Attributes? Internal? "trait" Name TypeParams? "{" TraitEntry* "}"
TraitEntry := TraitUse | Property | Constant | MethodWithBody | MethodRequirement

TraitUse := "uses" TypeRef ("," TypeRef)* (";" | "{" Adaptation* "}")
MethodRequirement := MethodSignature ";"

Adaptation := TraitRef "::" Name "insteadof" TraitRef ("," TraitRef)* ";"
            | TraitRef "::" Name "as" ("internal")? Name? ";"
```

Traits may define instance and static properties, methods, static methods, and
constants. Bodyless methods are explicit composer requirements. Stage 35 has no
trait property requirements and adds no `requires` keyword.

`$this`, `self`, and `self::member` resolve to the composing class
specialization after flattening. Traits cannot declare constructors,
destructors, `open`, or `override`, and cannot use `parent::`. Those forms depend
on a concrete class lifecycle or hierarchy and are rejected in v1.

Attributes are accepted on trait declarations and ordinary authored members,
not on `uses` or adaptation statements in Stage 35.

## Trait Requirements

A bodyless trait method carries the same signature, ownership, receiver,
generic, provenance, access, and checked-effect facts as an interface
requirement:

```doria
trait Persists
{
    function connection(): Connection;

    function save(): void throws StorageError
    {
        let $connection = $this->connection();
    }
}
```

Requirements are validated at declaration where possible and resolved after
recursive trait flattening and class inheritance. Class-authored, inherited, or
other trait-provided methods may satisfy them. An unresolved requirement reports
both its trait origin and composing class. It creates no runtime slot, abstract
class, implicit interface, or dynamic lookup.

## Trait Flattening And Layout

The compiler resolves the complete acyclic trait-specialization graph before
class layout. Direct `uses` entries expand in authored order at their lexical
position. Nested traits expand depth-first in authored order. A diamond copy of
the same trait specialization/member origin is included once at its first
canonical expansion; different specializations remain distinct and may
conflict.

Instance layout is:

1. inherited parent storage as the fixed prefix;
2. class-body properties and expanded trait properties in exact lexical and
   expansion order;
3. promoted constructor properties under the existing post-body rule.

Every flattened property initializer runs once per object in forward final
layout order. Initialized values are destroyed once in reverse order. Static
trait properties become one cell per composing class specialization, never one
cell on a runtime trait object. Constants and methods enter the composing
class's one member namespace.

Flattening preserves one authored trait member identity and private origin facts
containing its specialization, source span, and use path. Backends receive only
final checked class members/layout and never flatten traits independently.

## Trait Conflicts And Adaptations

```doria
class Message
{
    uses JsonFormatting, TextFormatting {
        JsonFormatting::format insteadof TextFormatting;
        TextFormatting::format as formatText;
        TextFormatting::debug as internal;
        TextFormatting::trace as internal traceText;
    }
}
```

`insteadof` chooses one available method and excludes one or more named trait
origins for that original name. `as alias` adds an alias while preserving the
original. `as internal` tightens the original. `as internal alias` adds an
internal alias while preserving the original.

No adaptation restores external access to an internal member. Doria does not
accept `public`, `private`, or `protected`. Adaptations apply only to instance
and static methods. Properties and constants cannot be adapted. Every alias
preserves signature, receiver, generics, return provenance, checked effects,
and body. Duplicate aliases and all one-member-namespace collisions are errors.

Precedence is deterministic:

1. The same diamond origin is de-duplicated.
2. Unrelated trait members with one name conflict until explicitly adapted,
   even when their signatures match.
3. A class-authored method wins over a trait method only when it is compatible
   with every displaced trait signature. An alias is required to retain access
   to the trait body.
4. A class-authored method may satisfy a trait requirement.
5. A trait never silently overrides an inherited class method. A closed
   inherited method makes composition invalid. An open inherited method
   requires a class-authored `override` wrapper, which may call an aliased trait
   method.
6. Interface requirements are checked against the final class and do not choose
   precedence.

## Attributes And Metadata

Interface and trait declarations are authored targets. Interface requirements,
trait members, type parameters, and parameters use their existing authored
target kinds. Implementations do not inherit requirement attributes.

Flattening and generic specialization do not create duplicate public metadata
targets or duplicate attribute applications. Public metadata retains the
trait-authored target once; compiler-private origin facts connect the effective
class member for diagnostics and tooling. Metadata schemas 1, 2, and 3 and
processor protocol 1 remain exact. Offsets, slots, vtables, descriptors, and
private origin tables never enter public metadata.

## Checked Errors

Interface requirements declare exact checked Error coverage; implementations
may narrow to a subset. A constrained static call uses the selected
implementation's known set where permitted, while an erased call exposes the
interface contract's set. Interface slots use the existing checked return ABI.
AmbientIo and TestAssertion remain automatic and separate.

Trait requirements use the same subset law. Trait bodies are checked when
authored and revalidated after substitution. Error subinterfaces participate in
catch coverage, rethrow, `toThrow`, finalizers, constructor failure, and dynamic
drop without losing concrete Error identity.

## Diagnostics And Safe Fixes

Stage 35 implementation reserves stable diagnostics for unknown/wrong-kind
interfaces and traits, inheritance/composition cycles, duplicate parents and
conformance, incompatible inherited requirements, missing or mismatched methods,
primitive erasure, invalid members/bodies/construction/narrowing, trait
conflicts/requirements/adaptations, forbidden lifecycle/hierarchy forms, invalid
core contracts, invalid Cloneable results, invalid Iterator contracts, and
iterator borrow escape.

Each diagnostic has a Title Case title, focused primary span, related
declarations/origins, and package-aware qualified names. Machine-applicable
fixes are limited to proven imports/names, exact duplicate removal, and token
changes that preserve behavior. Fixes never invent method behavior, erase
checked Errors, weaken ownership, add writable access, expose an internal API,
clone, allocate, box a primitive, choose a trait winner, or add property-hook
syntax.

## Compiler-Owned Models

The source-preserving AST must represent interface type parameters, parents,
requirements, bodyless functions, generic `implements`, ordered trait entries,
uses, adaptations, and spans. Checked semantic facts own interface and trait
identities, specializations, hierarchies, requirements, conformances,
implementations, canonical slots, conversions/tests, iterator loans, trait
origins, and finalized flattened class members/layout.

HIR carries checked contracts and explicit interface operations. MIR carries
validated two-word values, conversions, tests, calls, dynamic drops, nullable
facts, shared payloads, and iterator loan begin/end. Traits do not survive as
runtime MIR objects. Physical vtable constants, symbols, thunks, and descriptor
tables are backend-private.

Backends must never reconstruct conformance, slot order, trait precedence,
ownership, or effects. MIR validation rejects mismatched carrier/vtable types,
partial moves, missing dynamic drop, invalid slots/ABIs, unproved nullable use,
borrow escape, writable aliasing, iterator use outside its loan, and duplicate
flattened origins/layout entries.

## Backend Contract

The MIR interpreter remains the semantic oracle and does not use Rust trait
objects as Doria semantics. Cranelift and LLVM use static vtables and constant
slots; LLVM may devirtualize only from proven exact facts. Both preserve the
existing checked ABI, no per-object header, no interface wrapper allocation,
and no loop-body stack growth.

The PHP backend implements Doria semantics. It may emit compiler-private native
PHP interfaces where useful, but does not use reflection or dynamic method-name
lookup. Doria traits are flattened by the compiler; generated PHP does not rely
on PHP trait precedence or visibility. `instanceof` is emitted only from checked
Doria conformance facts.

## Tooling Contract

The compiler exports immutable semantic facts for interface/trait declarations,
hierarchies, requirements, conformances, conversions, narrowing, uses,
adaptations, final members, and source origins. The language server consumes
those facts for completion, hover, hierarchy and origin navigation,
go-to-implementation, references, conservative rename, diagnostics, and code
actions across files and packages.

The language server does not implement a second interface checker, trait
flattener, iteration protocol parser, slot algorithm, or spelling table.
Official tooling preserves UTF-16 ranges, unsaved overlays, generated/dependency
source protection, and parity between VS Code and IntelliJ.

## Performance And Soundness Contract

Stage 35 must prove structurally:

```text
concrete constrained call     static when specialized
interface-erased call         one constant-slot indirect call
interface value               two machine words
interface upcast              no allocation
per-object interface header   none
runtime method-name lookup    none
per-call adapter allocation   none
trait runtime object          none
public iteration allocation   not mandatory
primitive interface boxing    none
```

Conformance carriers cannot be forged. Vtables key exact class/interface
specializations and exact compiler provenance. Erasure preserves dynamic drop,
checked effects, nullability, ownership, and writable lease state. Trait
flattening initializes and destroys each property once. Incremental keys include
every interface/trait contract and dependent specialization.

Stage 35a remains responsible for broad devirtualization, optimizer metadata,
code-size measurement, generalized escape analysis, and additional stack
promotion. It consumes this sound nonallocating base representation and does not
redesign it.

## Implementation Slices

### Slice 1: Grammar, Graphs, And Conformance

Implement complete lexer/parser/AST/source identity, interface and trait
declaration graphs, interface inheritance, method compatibility, nominal
conformance, incremental facts, diagnostics, and compiler-fact-based tooling.
Interface value execution and trait composition remain precise pre-HIR
unsupported boundaries.

### Slice 2: Interface Runtime And Ownership

Implement HIR/MIR carriers, static vtables, calls, conversions, dynamic drop,
nullable/mixed/type-pattern behavior, Error and Displayable migration, all six
shared-ownership families, and interpreter/Cranelift/LLVM/PHP parity. Core
Cloneable/Iterable execution remains a Slice 3 boundary.

### Slice 3: Core Contracts And Public Iteration

Implement Comparable, Equatable, Hashable, Cloneable, the accepted Copy-to-
Cloneable widening, Iterable/Iterator loans, user-defined value-only `foreach`,
and optimized built-in integration. Generalized first-binding and mutable public
iteration remain out of scope.

### Slice 4: Trait Composition

Implement generic recursive flattening, requirements, adaptations, precedence,
layout/init/drop/static state, inheritance/interface integration, private
attribute origins, incremental invalidation, all backends, and official tooling.

### Slice 5: Cross-Repository Closure

Complete parity, malformed-MIR negatives, structural performance checks,
installed-toolchain UAT, documentation, editor artifacts, website handoff, and
Stage 35 status closure. Stage 35a, Stage 36 property hooks, and Stage 36a stream
surface work remain separate.

At every intermediate slice, accepted-but-unimplemented behavior is rejected
before HIR with one precise slice/stage diagnostic. No backend reinterprets an
unsupported path.

## Rejected Alternatives

- Structural or inferred conformance, external implementations, primitive
  boxing, universal erasure, per-object headers, reflection, and runtime method
  lookup conflict with accepted Doria identity.
- General interface properties and accessors would pre-decide Stage 36.
- Default/static interface members and constants are outside the accepted v1
  contract.
- `Cloneable<T>`, synthesized/magic clone, checked-throwing clone, and universal
  deep-copy guarantees either duplicate `self`, hide behavior, or infect
  preserving operations with open dynamic effects.
- `next(): ?T`, callback traversal, universal integer cursors, mandatory owning
  iterators, and mandatory heap iterators do not preserve Doria's Move-element,
  control-flow, complexity, or allocation contracts.
- Runtime traits, PHP trait precedence, source-order conflict winners, trait
  lifecycle methods, `parent::`, trait `open`/`override`, all-member adaptation,
  and duplicated flattened metadata are rejected.

## Non-Goals

This decision does not implement Stage 35. It does not accept property hooks,
general interface properties, default interface methods, static interface
members, abstract classes, primitive boxing, external implementations, generic
variance/defaults/reflection, runtime traits, async, FFI, Stage 35a optimizer
work, or Stage 36a stream names.

## Invalidated Elsewhere

- `SPEC.md`, `README.md`, the end-to-end plan, standard-library inventory, API
  guidance, inheritance guide, pipeline notes, audits, testing/self-hosting,
  metadata, diagnostics, and website guidance must stop describing Stage 35 as
  semantically unresolved.
- The AST/parser assumptions that interface bodies may be skipped, implements
  are bare strings, functions always own bodies, and traits contain only
  class-shaped members are implementation gaps.
- Displayable/Error-only semantic gates, compiler-known enumerations, and the
  separate Error carrier become migration inputs rather than general models.
- Class descriptors, ownership/borrowing, collections, shared ownership, HIR,
  MIR, validation, every backend, incremental facts, diagnostics, tests, and
  tooling require the slices above.
- Website guide, API reference, and playground examples require coordinated
  updates only as the corresponding executable slices land. Planned examples
  may remain clearly marked and must not be downgraded to current compiler lag.
- Stage 35a consumes but does not replace this ABI. Stage 36 retains all
  property-hook authority. Stage 36a may later name small capability interfaces
  without reopening the interface machinery.
