# Decision 0130: Single Class Inheritance, Open And Override, Parent Construction, And Hierarchy Dispatch

- **Status:** Accepted
- **Accepted:** 2026-09-01
- **Implementation Status:** Implemented By Stage 34
- **Amends:** Decisions 0029, 0080, 0081, 0082, 0083, 0084, 0089, 0090, 0093, 0105, 0117, 0119, 0121, 0122, 0123, 0125, and 0129

## Context

Doria already has nominal classes, deterministic ownership, constructor definite
initialization, checked Errors, generics, package-wide name resolution, and one
validated typed MIR consumed by every execution backend. Earlier records reserved
`extends` and `parent::` syntax but deliberately did not settle hierarchy
openness, overriding, construction, destruction, runtime representation, or
dispatch.

This decision completes single class inheritance without changing Doria's
headerless object payload, readonly-by-default ownership model, package model,
checked-effect model, or prohibition on runtime reflection.

## Declaration Surface

Classes are closed by default. A class permits subclassing only when declared
`open`:

```doria
open class Model
{
}

class Post extends Model
{
}
```

`open` and `override` are reserved keywords. Canonical class modifier order is
attributes, `internal`, `open`, `class`. A class has at most one direct parent.
Its parent clause is a source-preserving nominal type reference and may be
qualified or generic.

Methods are nonvirtual by default. An externally accessible instance method in
an open class introduces a virtual slot only when declared `open function`.
Replacing an inherited virtual implementation requires `override function`.
`override` reuses the inherited root slot and remains overrideable while the
containing class remains open. `open override` is redundant and invalid.

Static methods, constructors, destructors, internal methods, and methods in a
closed class cannot be open. An overriding method is externally accessible.
Doria adds no `protected`, `final`, `sealed`, abstract class, abstract method,
multiple class inheritance, or `static::` surface.

## Hierarchy And Generic Parents

The compiler builds one deterministic hierarchy graph after canonical Stage 31
name resolution. A parent must be a visible open class. Package identity,
direct-dependency visibility, generated-source scope, and package-wide global
`internal` rules remain unchanged. A class cannot extend itself and every cycle
is rejected with the complete deterministically ordered declaration chain.

A parent is a nominal class type rather than a name. Parent arity and constraints
are checked, child type parameters may appear in parent arguments, and every
concrete hierarchy is monomorphized. Generic classes remain invariant. A
`CachedNode<Cat>` may be a subtype of `Node<Cat>` but not `Node<Animal>`.
Inheritance-cycle detection operates on declarations and cannot be evaded by a
type-argument transformation.

The compiler-owned hierarchy authority records each declaration's direct
parent, ancestor chain, depth, root, generic substitution at every edge,
open/closed state, inherited source members, virtual roots and implementations,
layout facts, dynamic drop glue, and subtype identity. Semantic checking, HIR,
MIR, backends, metadata, incremental analysis, and official tooling consume
these facts rather than rebuilding a hierarchy.

## Inherited Members And Access

Externally accessible instance properties, instance methods, static properties,
static methods, and class constants participate in inherited lookup.
Constructors and destructors use the lifecycle protocol instead of ordinary
inheritance.

An inherited external property cannot be hidden and cannot gain duplicate
storage. Decision 0131 permits a compatible constructor parameter marked
`override` to redeclare the inherited-property relationship while the root
property remains the one logical member and physical storage slot. This is
compile-time property-family metadata, not virtual property dispatch. Constants,
static members, nonopen instance methods, incompatible properties, and explicit
class-body property redeclarations still cannot be hidden or redeclared. An
inherited open method can be replaced only by a valid method override. Doria's
single member namespace remains in force across member kinds.

An `internal` member is visible only to its declaring class. It is not
inherited-visible, cannot be accessed or overridden by a child, and cannot be
named through `parent::`. A child may author a same-spelled member because the
parent member is absent from its inherited source surface. The two members keep
declaring-class-qualified identities and distinct storage. A parent method
continues to resolve its own internal member lexically on a derived object.

Inherited static properties retain storage owned by their declaring class;
lookup through a child aliases that storage. Statics are never virtual.
`self::` remains lexically bound, `parent::` directly selects the immediate
parent member, and `static::` remains invalid.

## Override Contract

After generic substitution, one virtual slot has one substitutable source
contract:

- parameter count, names, types, ownership modes, generic arity, and generic
  constraints match exactly;
- the root open declaration owns parameter defaults and every override omits
  them;
- readonly may override readonly or writable, writable may override writable,
  and writable may not override readonly;
- a return may be covariant only when central class assignment compatibility
  proves it assignable to the root return while preserving ownership, borrow
  provenance, nullability, and generic specialization;
- required checked Errors may narrow to a covered subset or descendant but may
  not widen or introduce an unrelated Error.

Ambient I/O and TestAssertion remain automatic nonstructural effects. Their
profiles are unioned across reachable implementations of a virtual slot. The
slot uses checked transport whenever any reachable implementation needs it,
while a proven exact direct call keeps its exact ABI. They never become authored
`throws` entries and never disappear through virtual dispatch.

Generic open methods have one logical root slot. Reachable concrete method
specializations are monomorphized and deterministic physical entries are keyed
by logical slot plus canonical concrete type arguments. There is no boxed
generic dispatch or runtime type-argument lookup.

## Parent Calls And Construction

Inside a derived class, `parent::member()` directly names the immediate parent
implementation and bypasses virtual dispatch. In an instance method it uses the
current complete-object pointer. In static or instance context it may also name
accessible parent constants, static properties, and static methods. Ordinary
argument binding, receiver access, and checked effects apply. Internal parent
members remain inaccessible. `parent::__destruct()` is invalid.

The complete most-derived object is allocated once. Parent construction runs
exactly once and before the child's own initialization phase. If a parent has no
constructor, the compiler inserts an empty phase. If its constructor is callable
with no supplied arguments, the compiler inserts an implicit zero-argument
call unless the child writes one explicitly.

If the direct parent constructor has required parameters, the child must declare
a constructor whose first source-level statement is exactly one direct
`parent::__construct(...)` call. It cannot be conditional, nested, repeated, or
preceded by another action, and child constructor parameters are never forwarded
implicitly. Parent constructors used by children must be externally accessible;
there is no protected-like lifecycle exception.

Construction order is:

1. evaluate the `new` arguments once in source order;
2. allocate the complete most-derived payload;
3. execute root-to-parent property, promotion, and constructor phases;
4. execute the most-derived property and promotion phase;
5. execute the remaining most-derived constructor body.

Each class phase preserves Decisions 0090 and 0122. A failing phase cleans only
state already initialized, then completed parent state in reverse order, and
frees the one allocation once. An incomplete child phase has no child
destructor. Required and automatic parent-constructor effects flow into child
construction.

During a constructor or destructor phase, a direct `$this->openMethod()` call
dispatches to the implementation for the class phase currently executing. It
never reaches uninitialized or already destroyed more-derived state. Calls on
other fully constructed objects remain normally virtual. `parent::` is always
direct.

## Destruction

Fully constructed objects die in reverse construction order. The most-derived
destructor body runs first, followed by that class's still-owned properties in
reverse total property order, then the direct parent phase recursively through
the root. A class without a destructor still drops its own properties before
its parent phase. Moved-out and never-initialized fields remain skipped. The
complete allocation is freed once. Panic remains abort-only and runs no cleanup.

## Representation And Dispatch

The heap payload remains headerless and data-only. It contains no vtable pointer,
type tag, reference count, reflection header, or parent-object pointer.

A closed exact class value keeps the existing one-word opaque payload pointer.
A value whose static type is open uses a private two-word hierarchy carrier:
the complete-object data pointer plus an immutable static hierarchy descriptor
pointer. A closed child at its exact static type remains one word; upcasting it
to an open parent constructs the carrier without allocation or payload copying.
An open child's own static type is a carrier because descendants may inhabit it.
A nullable carrier uses null data and descriptor pointers and presence tests the
data pointer.

The one complete payload uses root-to-derived prefix layout. Every class adds
its explicit properties in class-body order followed by promoted properties in
constructor-parameter order. Parent offsets remain stable in descendants.
Internal parent fields remain physically present under declaring-class
identity. Generic hierarchies specialize complete size and alignment centrally.

One immutable descriptor exists per concrete monomorphized dynamic class. It
contains exact type identity, root and ordered ancestors, depth, private layout
facts, dynamic drop glue, virtual entries, checked slot ABI facts, and the
private facts needed for narrowing and mixed transport. Ancestor testing uses
stable descriptor/depth identity, never names or runtime hash lookup.

Virtual slot order inherits parent slots, reuses a slot for overrides, appends
new open methods in source order, and orders generic specializations by logical
slot and canonical type-argument key. A dynamic call uses one constant slot.
Compiler-private thunks may adapt receiver view, receiver access weakening,
covariant returns, and exact implementation effect profiles to the uniform slot
ABI. They are not source callables or metadata entries.

Destroying an owned open carrier invokes the descriptor's exact dynamic drop
once. Direct exact closed cleanup remains statically resolved. Stage 35 may
reuse private descriptor infrastructure for interface fat pointers but cannot
retrofit per-object headers.

## Upcasts, Narrowing, And Equality

Class assignment compatibility accepts an exact specialization or a source
whose ancestor is the invariant target specialization. This one relation powers
locals, assignment, arguments, returns, properties, constructors, closure
captures, collection elements, nullable values, mixed boxing, and checked Error
erasure.

Owned upcasts transfer ownership without cloning or allocation. Readonly and
writable borrowed upcasts preserve their access and provenance. `?Child`
upcasts to `?Parent`, preserving null and dynamic identity. Generic wrappers and
collections remain invariant; an element may be upcast before insertion, but a
`List<Child>` is not a `List<Parent>`.

For class targets, `is` and match type patterns test whether the dynamic class is
the target or a descendant. Narrowing preserves current ownership, borrow mode,
and null proof. Mixed class boxes retain the exact descriptor, data pointer, and
drop glue, and reconstruct the correct parent carrier or exact closed-child
view after a successful test. Open hierarchies are not assumed exhaustive.
There is no unchecked cast syntax or runtime reflection.

Class identity equality continues comparing object/data identity under the
existing compatible-type rules. The descriptor is not a second identity
component, and no structural or virtual equality is introduced.

## Shared Ownership, Collections, And Errors

Shared and weak wrappers remain invariant. A payload declared as an open class
stores the hierarchy carrier and retains the concrete dynamic drop. Acquired
readonly or writable access preserves the descriptor and ordinary access law.
Collections likewise store the representation of their static element type;
open-parent elements retain carriers, exact closed elements remain one word,
and insertion performs ordinary element upcast and ownership transfer.

A class extending an Error-conforming class remains Error-conforming through
the inherited external readonly `message`. A parent Error `throws` contract and
catch cover descendants. Catch ordering remains source order and a broader
parent catch makes a later child catch unreachable. Typed `toThrow` inspectors
accept the declared Error class or descendants, use a readonly parent view, and
preserve the dynamic concrete Error identity in presentation. Ambient I/O and
TestAssertion classification remains tied to compiler-known authority rather
than automatically spreading to arbitrary descendants.

## Attributes, Metadata, Incrementality, And Tooling

Attributes are authored-only in Stage 34: parent class and method attributes are
not copied or merged into descendants or overrides. Metadata schemas 1, 2, and
3 and processor protocol version 1 remain exact. Descriptors, slots, offsets,
thunks, and private type IDs never enter public metadata or runtime reflection.

Hierarchy, member, slot, construction, destruction, and specialization facts
participate in compiler fingerprints. Parent, openness, override, signature,
default, effect, receiver, property-order, lifecycle, import, package-visibility,
source-graph, and type-pattern changes invalidate affected descendants and
callers deterministically.

HIR and MIR carry explicit hierarchy identity, dispatch kind, parent chain,
upcast, type test, dynamic-drop, lifecycle phase, and checked-slot facts. Shared
MIR validation enforces the complete hierarchy contract. The interpreter,
Cranelift, LLVM, and PHP consume that same validated MIR and never reconstruct
inheritance from names. LLVM devirtualizes proven exact targets; true base-typed
calls remain constant-slot indirect calls. PHP adapts validated Doria behavior
without reflection or host destructor timing.

Official language-server tooling consumes compiler-owned hierarchy identities
for completion, hover, navigation, references, conservative family-wide rename,
diagnostics, and semantic tokens. Editor grammars highlight reserved syntax but
do not implement inheritance semantics.

## Performance And Security Consequences

Closed values remain one word, open static values are two words, upcasts allocate
nothing, each object has one allocation, virtual lookup is a constant slot,
descriptors are static per specialization, and exact calls are direct when
proven. Stage 34 adds no runtime name lookup, reflection, registry, or per-object
metadata header. Stage 35a owns the later broad optimizer and escape audit;
controlled timing evidence remains nonblocking.

## Non-Goals

This decision does not implement interfaces, traits, `uses`, abstract members,
multiple inheritance, `protected`, `final`, `sealed`, `static::`, unchecked
casts, class-body or virtual property overriding, property hooks, attribute inheritance, runtime
reflection, generic variance, default generic arguments, stable separately
compiled class ABI, or Stage 35/35a behavior.

## Invalidated Elsewhere

- Stage 34 unsupported diagnostics for `extends`, `parent::`, and hierarchy
  `is` are retired from active checking and retained only as historical codes.
- Exact-class-only member lookup, assignment, checked-Error coverage, match,
  `toThrow`, mixed transport, shared payloads, and collection layouts must use
  hierarchy facts.
- Exact one-word assumptions must become static-type-aware without changing
  closed exact class ABI or adding an object header.
- Constructor and destructor lowering must represent root-to-derived phases and
  lifecycle-phase dispatch rather than one isolated class body.
- Incremental fingerprints, compiler metadata clients, official editor tooling,
  active docs, parity matrices, examples, and mechanical guards must describe
  Stage 34 as complete and Stage 35 as next.
- The separate website later needs its class, override, parent construction,
  narrowing, Error hierarchy, performance, editor, and Stage-34 UAT surfaces
  synchronized; this compiler work does not modify that repository.
