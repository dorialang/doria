# Class Inheritance

Doria implements single class inheritance with explicit openness and explicit
overriding. Decision 0130 is the normative design record; this guide summarizes
the source model developers use day to day.

## Declaring A Hierarchy

Classes are closed by default. Declare a class `open` only when it is designed
to be a parent:

```doria
open class Model
{
    string $id;

    function __construct(string $id)
    {
        $this->id = $id;
    }

    open function label(): string
    {
        return "model {$this->id}";
    }
}

class Post extends Model
{
    override function label(): string
    {
        return "post {$this->id}";
    }
}
```

When a child accepts an input corresponding to an inherited external property,
it marks that parameter `override`. The root property remains the one storage
slot and the parent phase remains responsible for initializing it:

```doria
class Article extends Document
{
    function __construct(override string $title, parameter string $source)
    {
        parent::__construct($title);
        echo $source;
    }
}
```

`parameter` opts out of promotion for constructor-only inputs. Neither marker
forwards arguments implicitly. An internal parent property is not an override
target; a child may independently declare the same spelling because the two
fields retain distinct declaring-class identities. Explicit class-body hiding
and incompatible promoted collisions remain E0727 errors.

A class has at most one direct parent. Parent names may be qualified and may
instantiate generic classes. Generic hierarchies are monomorphized and remain
invariant.

Canonical class modifier order is attributes, `internal`, `open`, then `class`.
Doria does not add `protected`, abstract classes, multiple inheritance,
unchecked casts, `final`, `sealed`, or `static::`.

## Methods And Overrides

Methods are direct and nonvirtual by default. `open function` introduces a
virtual slot. A child replacing that implementation must write `override
function`; an accidental same-name method is diagnosed rather than silently
changing dispatch.

An override preserves the root method's parameter count, names, types,
ownership modes, generic arity, and constraints. Defaults stay on the root
declaration and are omitted by overrides. Receiver access may weaken from
writable to readonly, a class return may narrow covariantly when ordinary
assignment permits it, and checked Errors may narrow to covered descendants but
may not widen.

Static members are inherited but never virtual. `self::` remains lexically
bound. `parent::member()` directly calls or accesses the immediate parent member
and bypasses virtual dispatch:

```doria
override function label(): string
{
    return parent::label() . " (published)";
}
```

An `internal` member belongs only to its declaring class. It is neither
inherited-visible nor accessible through `parent::`. A child may independently
declare the same spelling because the two members have distinct
declaring-class identities.

## Construction And Destruction

One `new` expression allocates one complete most-derived object. Arguments are
evaluated once in source order. Construction then proceeds from the root class
to the most-derived class. Each phase initializes that class's properties and
runs its constructor body before the next phase begins.

When a parent constructor requires arguments, the child constructor must place
exactly one `parent::__construct(...)` call as its first source-level statement:

```doria
class Post extends Model
{
    string $title;

    function __construct(string $id, string $title)
    {
        parent::__construct($id);
        $this->title = $title;
    }
}
```

A zero-argument parent call is inserted when the parent constructor can be
called without supplied arguments and the child does not write one. Child
arguments are never forwarded implicitly.

Destruction reverses construction: the most-derived destructor runs first,
then that class's remaining properties are dropped in reverse order, followed
by each parent phase through the root. The complete allocation is freed once.
During construction and destruction, a virtual call through `$this` dispatches
only as far as the class phase currently executing, so it cannot reach
uninitialized or already destroyed derived state.

## Upcasts And Narrowing

A child value may be used wherever its invariant parent specialization is
expected. Owned upcasts transfer ownership; borrowed upcasts preserve readonly
or writable access and provenance. Nullable upcasts preserve null and dynamic
identity. The conversion allocates nothing and does not copy the object.

For class targets, `is` and `match` accept the target class or any descendant:

```doria
function describe(Model $model): string
{
    if ($model is Post) {
        return $model->label();
    }

    return "base model";
}
```

The narrowed value keeps its ownership and borrow mode. Open hierarchies are
not exhaustive, so a hierarchy `match` still needs `default` unless another arm
proves complete coverage. Doria has no unchecked downcast spelling.

Classes extending an Error-conforming class remain Error-conforming. A parent
`throws` declaration and catch cover descendants, broader catches make later
child catches unreachable, and typed `toThrow` inspection accepts descendant
Errors while preserving their dynamic identity.

## Runtime Shape

Class payloads remain headerless and data-only. A closed exact class value is
one pointer. A statically open class value is a two-word private carrier holding
the complete-object pointer and a static hierarchy descriptor pointer. The
descriptor is immutable and shared by every value of that concrete
specialization; it is not stored in the object.

Properties use root-to-derived prefix layout, so parent offsets remain stable.
Virtual slots are deterministic: inherited slots keep their position,
overrides reuse the root slot, and newly open methods append in source order.
Exact calls remain direct when proven; genuinely base-typed calls use one
constant virtual slot. Dynamic destruction follows the same descriptor and
runs the concrete drop path exactly once.

These representation details are compiler-private. Public metadata does not
expose descriptors, virtual slots, offsets, thunks, or runtime type IDs, and
Doria does not add runtime reflection to implement inheritance.

## What Comes Next

Stage 34 completes single class inheritance. Decision 0134 now fixes the Stage 35
contract for nominal generic interfaces, two-word interface carriers,
compile-time trait flattening, core value interfaces, and public iteration. Its
five implementation slices remain separate from this completed class-hierarchy
surface.
