# Doria Language Specification

This document describes the v0.1 direction for Doria.

## 1. What Doria is

Doria is a statically checked compiled programming language designed for native executables, tooling, services, desktop software, games, and future self-hosting.

Doria's surface syntax is intentionally familiar to developers coming from PHP-like and C-like languages, but Doria is not PHP++, PHP does not define Doria's semantics, and generated PHP is not Doria's reference behavior.

Doria source files use the `.doria` extension and do not require `<?php` tags.

The compiler is `doriac`. The current bootstrap implementation is written in Rust. Doria's primary target is native machine code and standalone executables. A strategic goal is for `doriac` to become increasingly self-hosted in Doria over time.

The compiler architecture is backend-independent:

```text
Doria source
-> lexer
-> parser
-> AST
-> semantic analysis
-> type checker
-> readonly/writable checker
-> Doria IR
-> backend
```

Backends may include:

- Native backend, the primary product target.
- Debug/interpreter backend, for validating Doria semantics without relying on another language runtime.
- PHP backend, as an optional compatibility, migration, debugging, and inspection target.
- WebAssembly backend.

The PHP backend must not shape the parser, AST, semantic model, Doria IR, native-oriented IR, runtime model, memory model, object model, error model, or standard library.

### 1.1 Design authority and correctness policy

Doria semantics are defined by Doria's specification, accepted design decisions, and explicit language-designer decisions. Backend output is an implementation of those semantics, not the authority for those semantics.

The project follows these rules:

```text
Correctness over speed.
Native-first over convenient transpilation.
Safety over quick demos.
Explicit design decisions over silent implementation assumptions.
```

If an implementation task reaches a design fork not answered by this specification or `docs/decisions/`, the implementation must stop and ask for a decision. It should report the question, options, tradeoffs, affected files, and a recommendation. It must not silently choose behavior because PHP, Rust, JavaScript, C, C++, or a backend library makes that behavior easy.

Temporary backend limitations may produce unsupported-feature diagnostics. They must not redefine Doria.

## 2. What Doria is not

Doria is not PHP++ and is not required to parse every valid PHP program.

Doria syntax is familiar to developers coming from PHP-like and C-like languages, but it is not PHP-compatible at the parser level.

Valid PHP should be easy to migrate to Doria, but Doria-specific syntax does not need to run directly in PHP.

Doria does not use `public`, `protected`, or `private` as member visibility modifiers. Class members are externally accessible by default, and `internal` marks implementation details.

The current compiler implementation produces only Stage 2a native smoke executables for exactly one top-level `function main(): int` returning an integer literal in the accepted `0..125` portable exit-code range. It is not yet full native code generation, a package manager, reflection system, macro system, async runtime, PHP migration converter, or full standard library. That implementation status does not make PHP transpilation the language goal.

Doria is not a Rust language. Rust is the current bootstrap implementation language for `doriac`, not the permanent identity of the compiler.

## 3. MVP syntax

The MVP supports:

- Top-level statements.
- `let` declarations.
- Explicit typed declarations.
- Functions.
- Classes.
- Properties.
- Methods.
- Constructor parameters and constructor property promotion.
- `echo`, `return`, `foreach`, `if` / `else if` / `else`, and `while`.
- Assignments.
- Function calls, method calls, property access, object construction, and literals.
- List and dictionary literals using PHP-like array syntax.

Planned near-term syntax includes:

- Attribute lists using `#[...]`.
- Named arguments using `name: expression`.
- Richer property initializer expressions, including object construction.

Planned future control-flow design includes:

- `do ... while ... finally`.
- `given ... when ... finally`.
- `given ... while ... finally`.
- `finally` attached to `if` / `else if` / `else` chains.
- `when` as a value-returning conditional form.
- `match` as a pattern/value selection construct.

These advanced control-flow forms are not MVP syntax. See `docs/decisions/0009-control-flow-direction.md`.

## 4. Declaration rules

Variables must be declared before use.

```doria
let $name = "Andrew";
let writable $count = 0;

string $city = "Lusaka";
writable int $score = 0;
```

Bare assignment never declares a variable:

```doria
$name = "Andrew"; // error
```

## 5. Readonly and writable rules

Everything is readonly unless explicitly marked `writable`.

```doria
let $x = 5;
$x = 10; // error

let writable $y = 5;
$y = 10; // ok
```

Explicit typed declarations follow the same rule:

```doria
int $x = 5;
$x = 10; // error

writable int $y = 5;
$y = 10; // ok
```

Properties are readonly by default:

```doria
class Person
{
    string $id;
    writable string $name;
}
```

To assign to a property, both the object path and the property must be writable, unless a constructor is initializing an uninitialized readonly property through constructor init access.

Constructor init access is narrower than writable `$this`. Inside `__construct`, a direct simple assignment such as `$this->id = $id;` may initialize an uninitialized readonly property of the declaring class exactly once. Property initializers and constructor-promoted parameters count as already initialized. Readonly init access does not permit compound assignments, nested writes such as `$this->child->name = "Lucy";`, calls to writable methods through `$this`, or initialization from repeatable bodies such as `foreach`. Writable properties keep normal mutation rules inside constructors, including inside repeatable bodies, subject to type checking. An explicitly declared `writable function __construct` also follows normal writable `$this` method rules. Full definite property initialization is future work; the current checker does not yet require every readonly property to be initialized by every constructor path.

Function parameters are readonly by default and become writable only with `writable`.

Methods receive readonly `$this` by default. A method that mutates `$this` must be declared with `writable function`.

## 6. Member access

Doria class members are accessible by default. Use `internal` only for implementation details that should not be accessed from outside the declaring class. Doria does not use visibility modifiers as boilerplate.

`writable` and `internal` solve different problems:

```text
writable controls mutation.
internal controls API surface.
```

Valid member declarations:

```doria
class Parser
{
    string $name;
    writable string $buffer;
    internal string $slug;
    internal writable int $position = 0;

    function parse(): Ast
    {
        return $this->parseProgram();
    }

    internal function parseProgram(): Ast
    {
        return new Ast();
    }

    internal writable function advance(): void
    {
        $this->position = $this->position + 1;
    }
}
```

Internal members are accessible only from methods and constructors of the declaring class. They are not accessible from top-level code, free functions, or other classes. No inheritance or `protected` behavior is part of early Doria.

Property hooks are planned later for validation and computed properties, but they are not part of the current implementation.

### API surface naming

Doria APIs should make intent obvious at the call site.

The preferred rule is:

```text
Nouns are properties.
Verbs are methods.
```

Use properties for values, state, identifiers, configuration, and computed data:

```doria
let $body = $message->body;
let $headers = $message->headers;
let $status = $message->status;
```

Avoid vague zero-argument noun methods when the member is conceptually data:

```doria
let $body = $message->body(); // avoid
let $headers = $message->headers(); // avoid
let $status = $message->status(); // avoid
```

A noun method such as `body()` can be misread as an action, preparation step, mutation, or builder-style operation. If the member represents data, expose it as a property.

Property hooks are the planned escape hatch when a property-shaped API needs validation, computed behavior, lazy decoding, caching, normalization, or guarded access. The public member should remain property-shaped when it is conceptually a value.

Use methods for actions, commands, mutation, I/O, async work, fallible operations, and behavior with meaningful work:

```doria
await $message->acknowledge();
await $message->retryAfter(seconds: 30);
$report->renderPdf();
```

If a data-returning operation must be a method because it performs I/O, expensive work, decoding, or another explicit operation, use an unmistakable verb such as `loadBody()`, `decodeBody()`, `findById()`, or `fetchProfile()`.

See `docs/api-design-guidelines.md` for the detailed design notes.

## 7. Basic type system

The MVP type names are:

```text
void
int
float
string
bool
null
mixed
object
resource
List<T>
Dictionary<K, V>
Set<T>
ClassType
```

The compiler keeps parsed type syntax and semantic types separate:

```text
TypeRef      parsed source spelling, such as `List<int>` or `Person`
TypeId       resolved semantic type identity
TypeKind     resolved semantic type shape
```

The semantic model also has an internal `Unknown` recovery type for diagnostics and error recovery; it is not the normal spelling for user-authored type declarations.

Lowercase primitive names are type-position names: `int`, `float`, `string`, `bool`, `object`, and `resource`. PascalCase names such as `Int`, `Float`, `String`, `Bool`, `Object`, and `Resource` are reserved for future expression-level standard-library/helper APIs. They are not primitive type spellings, and primitive type names are not namespaces. Future code should prefer APIs such as `Int::parse(...)`, but companion semantics are not part of the current implementation.

Collection aliases have fixed arity:

```text
List<T>
Dictionary<K, V>
Set<T>
```

`let` declarations infer simple literal and constructor types:

```doria
let $x = 5;        // int
let $name = "Doria"; // string
let $person = new Person("Andrew"); // Person
```

The semantic checker resolves parsed type syntax into semantic types before checking assignment, return, and positional call compatibility. Doria checks typed declarations, property initializers, property writes, parameter defaults, declared function/method return values, and call arguments for functions, methods, static calls, and constructors. It does not perform PHP-style scalar coercion: `int` is not assignable from `string`, `string` is not assignable from `int`, and `bool` is not assignable from `int`.

### String literals and interpolation

Single-quoted string literals are plain string literals. Double-quoted string literals support braced interpolation using Doria-owned syntax, not PHP backend behavior:

```doria
let $name = "Andrew";
echo "Hello, {$name}";
echo "Hello, {$this->profile->displayName}";
```

This slice supports only variable/property-path interpolation: `$name`, `$this`, `$user->name`, and repeated property access. Unbraced interpolation, method calls, function calls, arithmetic, comparisons, custom formatting, and full expressions inside interpolation are future work.

Interpolated strings are represented in the AST and Doria IR as string parts before any backend runs. The PHP backend lowers them explicitly, for example `"Hello, {$name}!"` becomes PHP equivalent to `"Hello, " . $name . "!"`.

Interpolated values may currently be `string`, `int`, `float`, `bool`, `null`, `mixed`, or the internal `Unknown` recovery type. Class values, `object`, `resource`, `List<T>`, `Dictionary<K, V>`, and `Set<T>` are rejected until Doria has a deliberate display/string-conversion design.

Numeric widening is not implemented yet; for now `float` is not assignable from `int`, and `int` is not assignable from `float`. Any future safe numeric widening should be a separate design decision. Named arguments and richer call argument representation are separate future slices.

Simple collection literals infer collection element/key/value types when all clear parts match. Clear heterogeneous collection literals, such as `[1, "two"]`, are rejected by narrow collection alias assignment checks rather than being erased to `Unknown`. The empty literal `[]` stays ambiguous so typed contexts may use it as an empty `List<T>` or `Dictionary<K, V>`. The PHP-compatible `array` annotation remains broad enough to accept list-shaped and dictionary-shaped literals for now, but `array` is not the desired long-term collection model.

### Control-flow conditions

Basic `if` / `else if` / `else` and `while` are MVP syntax. Conditions must be `bool`; Doria does not use PHP-style truthiness for integers, strings, null, objects, resources, or collections. The checker currently allows `mixed` and the internal `Unknown` recovery type so one diagnostic does not cascade into unrelated follow-up errors.

Each `if`, `else if`, `else`, and `while` body has its own block scope. Variables declared inside those bodies are not visible after the block. Until Doria has path-sensitive definite initialization analysis, constructor readonly init access is available only for direct constructor-body assignments and not inside `if`, `else if`, `else`, or `while` bodies.

## 8. Class syntax

```doria
class Person
{
    function __construct(
        writable string $name,
        int $age,
    ) {
    }
}
```

Constructor property promotion is supported in the current vertical slice. Constructor parameters are promoted to externally accessible properties by default unless marked `internal`:

```doria
function __construct(
    writable string $name,
    int $age = 10,
    internal string $cacheKey = "person",
) {
}
```

Constructor init access is supported for direct initialization of uninitialized readonly properties inside constructor bodies:

```doria
class Person
{
    string $id;

    function __construct(string $givenId)
    {
        $this->id = $givenId;
    }
}
```

This does not make `$this` writable. The constructor cannot assign the same readonly property twice, cannot reassign a readonly property that already has an initializer or is promoted from a constructor parameter, cannot use compound assignment for init access, and cannot use init access for nested object paths.

Doria should support richer instance property initializers than PHP:

```doria
class Office
{
    Person $manager = new Person();
}
```

Instance property initializer expressions run once per object construction. Each object gets its own initialized value. A property initializer counts as initialization for readonly properties.

## 9. Function syntax

```doria
function greet(string $name): void
{
    echo "Hello, {$name}";
}
```

Parameters are readonly unless marked `writable`:

```doria
function rename(writable Person $person, string $name): void
{
    $person->name = $name;
}
```

Declared return types are checked against returned expressions:

```doria
function age(): int
{
    return 37;
}
```

`void` functions and methods may use `return;` or fall through. Lifecycle methods, currently `__construct` and `__destruct`, are void-like: they may omit a return type or explicitly declare `: void`, may use bare `return;`, and may fall through. A non-`void` lifecycle return annotation is an error, returning a value from a `void` function or lifecycle method is an error, and lifecycle methods cannot be called directly as ordinary methods. `__construct` may declare parameters and constructor calls are checked against them through `new Class(...)`. `__destruct` must not declare parameters. For declared non-`void` return types, the current checker requires the final top-level statement of the function or method body to be `return expr;`. This is a deliberate early rule, not full path-sensitive control-flow analysis.

Calls are checked against declared parameter lists:

```doria
function greet(string $name, string $suffix = "!"): void
{
}

greet("Andrew");      // ok
greet("Andrew", "!"); // ok
greet();              // error
greet(123);           // error
```

Only positional arguments are supported in the current slice. Required parameters cannot follow optional parameters until named arguments exist.

## 10. Collection aliases

Doria uses:

```doria
List<int>
Dictionary<string, int>
Set<string>
```

Do not use `Vec`.

The current PHP compatibility backend may lower these aliases to PHP arrays, while the Doria type checker keeps them distinct. The native backend must make deliberate representation choices for each collection family rather than inheriting PHP array behavior.

The current type foundation resolves explicit annotations, reports unknown type names and invalid collection alias arity, and checks assignment compatibility for typed declarations, property initializers, property writes, parameter defaults, declared return values, and positional call arguments. Classes without constructors cannot be constructed with arguments.

## 11. Attributes and metadata expressions

Doria should support attribute syntax using `#[...]`.

Unlike PHP attributes, Doria attributes should eventually allow richer typed expressions, including static factory calls and named arguments.

Example:

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

The intended direction is:

```text
- Attribute expressions are parsed and type-checked by Doria.
- Attribute arguments may use named arguments.
- Attribute expressions may include literals, lists, dictionaries, class/static references, object construction, and static factory calls.
- The exact compile-time vs runtime evaluation policy is not settled yet.
- Doria should avoid blindly executing arbitrary side-effecting code at compile time.
```

See `docs/executable-initializers-and-attributes.md` for the detailed design notes.

## 12. PHP interop and migration

Doria may support two separate PHP-related directions:

```text
1. Doria -> PHP compatibility/debugging backend.
2. PHP -> Doria migration converter.
```

Both are optional adoption and tooling aids. Neither is the core correctness target for the language.

A PHP-to-Doria converter may eventually help migrate existing PHP codebases into Doria, but it must remain architecturally separate from the Doria parser and core compiler semantics.

Recommended future shape:

```bash
doriac migrate php src --out migrated
```

The converter should initially produce conservative valid Doria, not perfect idiomatic Doria. It should use diagnostics for unsupported dynamic PHP features rather than pretending every valid PHP program can be automatically converted safely.

Doria should avoid promising full bidirectional PHP/Doria compatibility.

See `docs/php-interop-and-migration.md` for the detailed design notes.

## 13. Doria IR and backend behavior

Doria IR is the checked compiler-owned representation of a Doria program. After semantic analysis, type checking, and readonly/writable checking, the compiler lowers the checked AST into Doria IR before backend output.

As native code generation matures, Doria IR may lower into a simpler native-oriented IR for control flow, memory layout, runtime calls, and backend code generation.

The native backend is the primary target. It should lower Doria IR, and any later native-oriented IR, toward native machine code and standalone executables. The current Cranelift-backed native backend is deliberately limited to the accepted Stage 2a smoke entrypoint: exactly one top-level `function main(): int` returning an integer literal in `0..125`. It emits unsupported-feature diagnostics for locals, arithmetic, strings, `if` / `while`, classes, collections, and broader valid Doria until later native slices are designed.

The PHP backend is currently implemented as a compatibility/debugging backend. It emits `<?php` and lowers Doria-only syntax away:

- `let` is removed.
- `writable` is removed.
- `internal` is enforced by Doria before backend emission and may lower to PHP `private` or another backend-specific representation.
- Collection aliases are emitted as `array` for the current PHP backend only.
- `resource` is emitted as `mixed` because PHP cannot declare `resource` type hints.
- Doria readonly/writable rules are enforced before Doria IR lowering and backend emission, not at PHP runtime.

For Doria features that PHP cannot express directly, such as object construction in property initializers or richer attribute expressions, the PHP backend should lower to equivalent generated PHP where practical or produce a clear unsupported-feature diagnostic temporarily. PHP limitations must not define Doria semantics.

Backend-specific tests are useful, but the PHP backend must not be the required proof that a language feature is correct. Correctness belongs to the parser, semantic checker, Doria IR, and eventually the native/backend-independent execution path.

## 14. Future features

Future work includes:

- Better diagnostics with suggestions.
- Nullable types.
- Full type inference for lists and dictionaries.
- Interfaces, traits, and namespaces.
- Attribute syntax and metadata representation.
- Richer instance property initializers.
- Named arguments.
- Full path-sensitive control-flow analysis for returns and constructor initialization.
- Advanced control-flow design for `do ... while ... finally`, `given ... when`, `given ... while`, `if` chains with possible `finally`, value-returning `when`, and `match`.
- Async/await and structured concurrency.
- Broader native backend design and implementation beyond the Stage 2a smoke target.
- Native-oriented IR implementation when native code generation needs it.
- Broader native code generation and standalone executable production beyond the Stage 2a smoke target.
- Self-hosting path for writing more of `doriac` in Doria.
- PHP-to-Doria migration tooling.
- Package management.
