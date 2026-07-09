# Executable Initializers and Attribute Expressions

> Documentation role: supporting design note.
> Source-of-truth hierarchy: `docs/doria-end-to-end-plan.md` owns future sequencing; accepted `docs/decisions/*.md` files own topic-level decisions. This note is subordinate to both.

Doria has syntax familiar to developers coming from PHP-like and C-like languages, but it should not inherit PHP's restrictions around property default values or attribute arguments.

In PHP, property initializers and attribute arguments are limited to constant values or constant expressions. Doria should allow richer, typed, compiler-checked expressions in both places.

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

The `Office::$manager` property should be initialized with a fresh `Person` object for each `Office` instance.

Doria should also allow expressive metadata/decorator-style configuration:

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

This is deliberately more capable than PHP attributes.

---

## 2. Design principle

The rule should be:

```text
Doria initializers and attributes may contain Doria expressions, not only constant literals.
```

However, this does not mean every arbitrary expression should be allowed everywhere immediately.

Doria should define expression contexts carefully:

```text
1. Runtime property initializer expressions.
2. Static/module initializer expressions.
3. Attribute metadata expressions.
4. Future compile-time/evaluable expressions.
```

Each context may have different restrictions.

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

## 6. Attribute expression syntax

Doria attributes should support named arguments, arrays/lists, dictionaries, class references, object construction, and static factory calls.

Examples:

```doria
#[Route(method: HttpMethod::Post, path: "/posts")]
function createPost(): Response
{
    // ...
}
```

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

This means the parser needs attribute syntax:

```text
#[AttributeName(...)]
```

and named arguments:

```text
name: expression
```

inside calls and attribute argument lists.

---

## 7. Attribute evaluation model

Doria should not blindly run arbitrary code at compile time by default.

Recommended model:

```text
- Attribute expressions are parsed, type-checked, and stored as structured metadata.
- Some attribute expressions may be evaluated at compile time only if they are explicitly allowed to be compile-time safe.
- Other attribute expressions may lower to module initialization or framework metadata registration code.
```

This keeps Doria expressive without making compilation depend on arbitrary side effects.

Future options:

```text
1. Pure metadata expressions only.
2. `const` constructors and `const` static methods allowed in attributes.
3. `comptime` expressions allowed in attributes.
4. Runtime metadata factories lowered by the backend.
```

The exact policy is not settled yet.

---

## 8. Attribute restrictions to decide later

Open questions:

```text
- Should attribute expressions be pure?
- Should they be allowed to perform I/O?
- Should they be evaluated by the compiler, by generated module initialization code, or by reflection at runtime?
- Should functions used in attributes require a marker such as `const`, `pure`, or `comptime`?
- Should attribute objects be required to be immutable?
- How should attribute metadata be represented in native binaries?
- How should PHP backend output represent Doria attributes that PHP cannot express directly?
```

Until these questions are settled, implement parsing and AST/Doria IR representation first, not full evaluation.

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

For attributes, the PHP backend may need to generate metadata registration code instead of PHP attributes.

Example direction:

```php
DoriaMetadata::register(PostsModule::class, [
    new Module(imports: [
        ORMModule::forRoot(...)
    ]),
]);
```

The exact PHP lowering can wait. The important rule is:

```text
Doria semantics come first. PHP output adapts to Doria, not the other way around.
```

---

## 10. Parser and AST implications

Needed parser additions:

```text
- Attribute lists before classes, functions, methods, properties, parameters, and possibly modules.
- Named arguments using `name: expr`.
- Object construction and static calls inside property initializers.
- Object construction and static calls inside attribute arguments.
```

Needed AST additions:

```text
Attribute
AttributeArg
CallArg
PropertyInitializer
```

A possible shape:

```rust
pub struct Attribute {
    pub name: Path,
    pub args: Vec<CallArg>,
    pub span: Span,
}

pub enum CallArg {
    Positional(Expr),
    Named { name: String, value: Expr, span: Span },
}
```

Function calls, static calls, constructor calls, and attributes should share the same argument representation so named arguments are not duplicated in the AST design.

---

## 11. Type-checking implications

The checker must eventually verify:

```text
- Property initializer expression type is assignable to property type.
- Attribute class or metadata constructor exists.
- Attribute argument names exist.
- Attribute argument expression types match expected parameter types.
- Attribute expressions obey the chosen evaluation policy.
```

Example:

```doria
class Office
{
    Person $manager = "Andrew"; // Error: string is not Person
}
```

Example:

```doria
#[Module(imports: "not a list")]
class PostsModule
{
}
```

should fail if `imports` expects a `List<ModuleImport>` or equivalent.

---

## 12. Near-term implementation plan

Implement in this order:

```text
1. Add parser support for attributes without semantic evaluation.
2. Add AST support for attributes and named arguments.
3. Add parser support for named call arguments generally.
4. Keep property initializers in AST/Doria IR as expressions.
5. Add semantic checks that property initializer type matches property type once TypeId/TypeKind exists.
6. Add PHP backend lowering for simple `new` property initializers into constructors.
7. Add a metadata representation for attributes in Doria IR.
8. Decide attribute evaluation policy before executing attribute expressions.
```

Do not let the PHP backend block the language design.

---

## 13. Settled direction

Settled:

```text
- Doria should allow object construction in instance property initializers.
- Doria should allow richer attribute expressions than PHP.
- Instance property initializers should run per object construction, not be shared across instances.
- PHP backend limitations must not restrict Doria syntax.
```

Open:

```text
- Exact compile-time vs runtime evaluation policy for attributes.
- Whether attribute expressions require `const`, `pure`, or `comptime` functions.
- Exact PHP backend lowering for attributes.
- Exact native metadata representation.
```
