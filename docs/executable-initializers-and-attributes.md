# Executable Initializers and Attribute Expressions

> Documentation role: supporting design note.
> Source-of-truth hierarchy: `docs/doria-end-to-end-plan.md` owns future sequencing; accepted `docs/decisions/*.md` files own topic-level decisions. This note is subordinate to both.

Doria has syntax familiar to developers coming from PHP-like and C-like
languages, but its initializer and metadata rules are defined by Doria rather
than PHP.

Instance property initializers are executable per-object initialization.
Attributes are a separate compiler/tooling metadata surface whose Stage 32
arguments use bounded typed constant evaluation. Decision 0125 settles that
separation.

This document records the intended Doria direction.

---

## 1. Motivation

Doria should allow code like this:

```doria
class Person
{
    function __construct(
        string $name = "Unknown",
    ) {
    }
}

class Office
{
    Person $manager = new Person();
}
```

The `$manager` property should be initialized with a fresh `Person` object for each `Office` instance.

Doria may eventually grow a richer metadata-expression tier for configuration
such as:

```doria
#[Module(
    imports: [
        ORMModule::forRoot(
            type: "mysql",
            host: "localhost",
            port: 3306,
            username: "root",
            password: "root",
            database: "test",
            entities: [],
            synchronize: true,
        )
    ]
)]
class PostsModule
{
}
```

This `Module(imports: [...])` example is **future direction**, not accepted
Stage 32 source. Collections, object construction, and static factories are not
current metadata values.

---

## 2. Design principle

The rules are:

```text
Instance property initializers are executable construction-time expressions.
Attribute arguments are typed, bounded constant metadata expressions.
```

Doria should define expression contexts carefully:

```text
1. Runtime property initializer expressions.
2. Static/module initializer expressions.
3. Stage 32 constant attribute metadata expressions.
4. Future compile-time/evaluable expressions.
```

Each context has its own restrictions. One context does not widen another.

---

## 3. Instance property initializers

Doria should allow instance properties to be initialized with non-constant expressions:

```doria
class Office
{
    Person $manager = new Person();
    List<Person> $staff = [];
    Dictionary<string, string> $labels = [];
}
```

Semantics:

```text
- Instance property initializers run once per object construction.
- Each object gets its own initialized value.
- Object, list, dictionary, and set initializers must not be shared accidentally between instances.
- Initializers run before the constructor body.
- Constructor-promoted properties are initialized from constructor arguments.
- A constructor may traverse an initializer-initialized writable property and
  mutate the owned child under ordinary writable-path rules.
- An independently owned value may directly initialize an owning property.
```

Example:

```doria
let writable $a = new Office();
let writable $b = new Office();

$a->manager->name = "Dorothy";

// $b->manager is a different Person object.
```

This avoids accidental shared mutable state.

---

## 4. Readonly interaction

A property initializer counts as initialization.

```doria
class Office
{
    Person $manager = new Person();
}
```

Because properties are readonly by default, this means:

```doria
let writable $office = new Office();

$office->manager = new Person(); // Error: manager is readonly
```

If a property is marked `writable`, the property can be reassigned later:

```doria
class Office
{
    writable Person $manager = new Person();
}

let writable $office = new Office();
$office->manager = new Person("Lucy"); // ok
```

Constructor init access must account for property initializers:

```text
- A readonly property with an initializer is already initialized before the constructor body.
- The constructor must not assign it again unless Doria later adds an explicit override mechanism.
- A readonly property without an initializer may be assigned exactly once through constructor init access.
```

Direct constructor `$this` is a construction root rather than a writable
receiver. An initializer-initialized writable property may supply ordinary
writable access to its owned child, but a readonly intermediate blocks that
path and a nested readonly property receives no constructor initialization
privilege. An initialized writable owning property may be replaced after the
new owned value has been acquired; checked failure leaves the old value in
place.

---

## 5. Static property and module initializers

Static or module-level initializers should be treated separately from instance property initializers.

Possible future syntax:

```doria
class Registry
{
    internal static Dictionary<string, Handler> $handlers = [];
}
```

Semantics should eventually be:

```text
- Static initializers run once per program/module initialization.
- They may create shared objects intentionally.
- Initialization order must be specified before this feature becomes stable.
```

Do not design native code generation around PHP's initialization model.

---

## 6. Stage 32 attribute syntax

Stage 32 implements adjacent `#[...]` groups before declarations:

```doria
#[Attribute]
class Route
{
    function __construct(
        string $path,
        HttpMethod $method = HttpMethod::Get,
    ) {
    }
}

#[Authenticated, Route(path: "/posts"),]
#[Test]
function createPost(): void
{
}
```

Attributes reuse ordinary source-order positional and named arguments. Several
applications may share one group, several groups may target one declaration,
and trailing commas are accepted. Qualified names such as
`#[Acme\Metadata\Route(...)]` use the ordinary namespace/package resolver.

`#[` must be adjacent. `# comment` and `# [Route]` remain comments.

Supported targets are global type/function/constant declarations, class and
trait members, callable parameters, enum cases, and enum payload fields. Stage
32 does not place attributes on statements, locals, expressions, closures,
generic parameters, return types, or throws entries.

---

## 7. Attribute classes and values

`#[Attribute]` marks a non-generic class as a schema. Its constructor parameter
list defines names, types, order, and defaults:

```doria
#[Attribute]
class Cache
{
    function __construct(string $key, int $seconds = 60) {}
}

#[Cache(key: "posts", seconds: 30)]
function loadPosts(): void
{
}
```

Schema parameters are readonly. Applying the attribute does not instantiate the
class or execute its constructor body. User schemas follow package visibility;
an `internal` schema remains package-internal.

Accepted values are the bounded Decision 0084 constant tier: exact numerics,
`bool`, `string`, compatible nullable values, top-level and class constants,
typed constant operations and numeric conversions, and compatible unit,
backed, or payload enums.

These are rejected as Stage 32 metadata values:

```text
runtime locals and parameters
property or mutable-static reads
function, method, and static-factory calls
constructors and closures
I/O, panic, time, randomness, and environment access
objects, collections, typed arrays, Bytes, mixed, and shared handles
```

An invalid application creates no partial metadata record. Attributes
contribute no checked effects and perform no side effects.

---

## 8. Metadata and runtime boundary

The compiler resolves and type-checks attribute schemas and applications once,
then stores canonical typed facts in semantic information and HIR. Metadata
retains source/package identities, target identities, authored order, bound
parameter order, defaults, exact values, and source spans.

MIR contains no attribute operation. The interpreter, Cranelift, LLVM, and PHP
compatibility backend create no runtime attribute object, reflection table,
metadata registry, test runner, or export bridge. Attribute-bearing source
executes like the equivalent source without metadata.

`#[Test]` and `#[PHPExport]` are compiler-known metadata markers only. Baton test
orchestration begins in Stage 33; PHP bridge semantics begin in Stage 41.

---

## 9. PHP backend strategy

The PHP backend cannot simply emit every Doria initializer or attribute directly as PHP syntax.

For property initializers like:

```doria
class Office
{
    Person $manager = new Person();
}
```

PHP backend options include:

```text
1. Lower the initializer into the generated PHP constructor.
2. Generate a helper initialization method and call it from constructors.
3. Reject unsupported cases temporarily with a clear diagnostic.
```

Preferred eventual lowering:

```php
class Office
{
    public Person $manager;

    public function __construct()
    {
        $this->manager = new Person();
    }
}
```

For Stage 32 attributes, PHP emits no PHP attribute and no metadata registration
code. Metadata remains compiler/tooling data. The important rule is:

```text
Doria semantics come first. PHP output adapts to Doria, not the other way around.
```

---

## 10. Parser and AST model

The parser uses a first-class adjacent attribute-opening token. Source-preserving
groups retain qualified names, delimiters, arguments, argument names, commas,
trailing commas, group spans, and authored order. Attribute applications reuse
the same `Argument` representation and binding service as calls; there is no
second named-argument model.

The parser deliberately recovers malformed or misplaced groups to the next
declaration. It never treats an attribute as an expression statement.

---

## 11. Type checking

The checker verifies:

```text
- Property initializer expression type is assignable to property type.
- Attribute class or metadata constructor exists.
- Attribute argument names exist.
- Attribute argument expression types match expected parameter types.
- Attribute values belong to the bounded constant metadata tier.
```

Example:

```doria
class Office
{
    Person $manager = "Andrew"; // Error: string is not Person
}
```

Unsupported metadata types are rejected at the schema. Wrong argument types are
diagnosed before const-evaluation consequences. Named/default binding uses the
same causal diagnostics as ordinary calls.

---

## 12. Metadata command and processor protocol

`doriac metadata` supports standalone source and complete build plans. It emits
strict deterministic schema-version-1 JSON and no backend artifact.

Stage 32 also defines strict typed processor request and response models. They
validate compiler/graph identity, typed metadata, processor diagnostics,
content hashes, generated scopes, and normalized package-relative generated
paths. The compiler executes no processor and writes no generated source.
Decision 0118 and Stage 33 Slice 3 own explicit Baton orchestration.

---

## 13. Settled and future direction

Settled:

```text
- Doria should allow object construction in instance property initializers.
- Doria attributes are typed compiler/tooling metadata, not PHP attributes.
- Instance property initializers should run per object construction, not be shared across instances.
- Stage 32 attribute arguments use bounded constant evaluation only.
- Attribute applications execute no Doria code and add no runtime metadata.
- PHP backend limitations must not restrict Doria syntax.
- Constructor-rooted writable paths follow Decision 0122 rather than PHP's
  object model.
- Owned property initialization and writable replacement are accepted Doria;
  general property move-out remains separate.
```

Future, requiring separate authority:

```text
- Richer collection, object, class-reference, or factory metadata values.
- Any `const` function, `const` constructor, or general `comptime` facility.
- Target masks, repeatability declarations, and attribute inheritance.
- Runtime reflection.
- Processor execution and generated-source rounds through Baton.
```

Decision 0125 is authoritative for the implemented Stage 32 surface.
