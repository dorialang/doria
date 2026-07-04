# Doria Language Specification

This document describes the v0.1 direction for Doria.

## 1. What Doria is

Doria is a statically checked compiled programming language designed for native executables, tooling, services, desktop software, games, and future self-hosting.

Doria's surface syntax is intentionally familiar to developers coming from PHP-like and C-like languages, but Doria is not PHP++, PHP does not define Doria's semantics, and generated PHP is not Doria's reference behavior.

Doria source files use the `.doria` extension and do not require `<?php` tags.

The compiler is `doriac`. The current bootstrap implementation is written in Rust. Doria's primary target is native machine code and standalone executables. A strategic goal is for `doriac` to become increasingly self-hosted in Doria over time.

Baton is planned external project tooling for project, package, build, and application orchestration. Baton does not define Doria semantics and is not part of the compiler pipeline.

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

The current compiler implementation produces only Stage 6c native smoke executables for exactly one top-level `function main(): int` with supported readonly and writable integer locals, `=`, `+=`, and `-=` assignments to writable integer locals, `+`/`-`/`*` arithmetic, structured returning `if` blocks, fallthrough `if` statements with visible-local merges, and bounded structured `while` loops in the accepted `0..125` portable exit-code range. Supported native `while` bodies may contain integer local declarations, writable integer assignments, and fallthrough `if` statements. Native validation proves accepted loops terminate within the current smoke verification cap before lowering them through a private native smoke module to real Cranelift control flow. Supported native conditions include bool literals, grouped conditions, integer comparisons, and bool-only `not` / `and` / `or` / `xor`. It is not yet full native code generation, a package manager, reflection system, macro system, async runtime, PHP migration converter, or full standard library. That implementation status does not make PHP transpilation the language goal.

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
- `break` and `continue` for nearest-loop control flow.

These advanced control-flow forms are not MVP syntax. See `docs/decisions/0009-control-flow-direction.md`.

### Source organization and compiler directives

The accepted namespace, import, include, and directive direction is recorded in `docs/decisions/0028-namespaces-use-include-and-directives.md`. Current compiler support may lag this accepted direction until lexer, parser, semantic name resolution, source management, Doria IR, backends, and LSP support are updated.

Namespaces define logical symbol ownership and declaration scope. They are part of semantic name resolution, not source inclusion, package resolution, build orchestration, or runtime loading.

Accepted conceptual syntax:

```doria
namespace App\Services;

class UserService
{
}
```

Nested namespace paths such as `namespace App\Domain\Users;` are accepted as the likely/default direction. The backslash separator matches Doria's PHP-shaped readability, but exact grammar details remain future implementation work.

`use` statements import names from namespaces at namespace/file-scope only. `use` is semantic name resolution and aliasing. It is not textual inclusion, PHP runtime include, package dependency resolution, trait composition, or code execution. `use` is not valid inside class, trait, interface, function, or method bodies.

Accepted conceptual syntax:

```doria
use App\Models\User;
use App\Security\Permission;
use App\Repositories\PostRepository as Posts;
```

`use` may import fully qualified symbols and may alias symbols. Duplicate or conflicting imports should be diagnosed. Unused import warnings may be added later. `use` does not load packages by itself; package resolution belongs to Baton later.

Class-body and trait-body trait composition uses `uses`, not namespace import `use`.

`include` is compile-time source inclusion with required include-once behavior. It is lower-level source composition, not the normal import mechanism. If an included file cannot be found, compilation fails. If the same canonical file is included more than once, it is included once. Include resolution must be deterministic, include diagnostics must preserve source file and span information, and included source participates in the same compiler pipeline as normal Doria source.

Accepted conceptual syntax:

```doria
include "src/generated/routes.doria";
```

Only string-literal local source paths are accepted in the intended direction. Computed paths and remote includes are rejected:

```doria
include $path;                         // rejected direction
include getPath();                     // rejected direction
include "https://example.com/file.doria"; // rejected direction
```

Doria does not add separate PHP-style `require`, `require_once`, or `include_once` forms. Doria `include` already means required include-once source inclusion.

`break` exits the nearest enclosing loop. PHP-style numeric break levels such as `break 2;` are not accepted by the namespace/directive decision. Labeled break may be evaluated later if needed.

`continue` jumps to the next iteration of the nearest enclosing loop. PHP-style numeric continue levels such as `continue 2;` are not accepted by the namespace/directive decision. Labeled continue may be evaluated later if needed.

`declare` is a structured compiler/source directive. It is not a macro system and not textual substitution. Exact grammar and allowed declaration keys require future decisions. Unknown declare keys should be rejected when `declare` is implemented. Possible future uses include warning policy, unsafe/FFI boundary policy, backend/profile constraints, platform configuration, optimization intent, feature gates, and compile-time diagnostics.

`goto` is evaluation-only and is not accepted for implementation yet. If it is ever accepted, it should be constrained so it cannot jump into deeper scopes, bypass visible initialization, bypass cleanup or `finally` obligations, cross protected resource regions, or cross future borrow/lifetime boundaries.

Doria should not adopt a C/C++ textual macro preprocessor by default. `#define` and `#undef` textual macro substitution are not accepted. Future conditional compilation and compile-time diagnostics should use structured compiler semantics rather than arbitrary token substitution. Doria source should remain parseable, typed, and semantically checked by `doriac`.

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

Accepted type-position names include:

```text
void
int
int8
int16
int32
int64
uint8
uint16
uint32
uint64
float
float32
float64
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

Lowercase primitive names are type-position names: `int`, `int8`, `int16`, `int32`, `int64`, `uint8`, `uint16`, `uint32`, `uint64`, `float`, `float32`, `float64`, `string`, `bool`, `object`, and `resource`. `int` means `int64`; `float` means `float64`. PascalCase names such as `Int`, `Float`, `String`, `Bool`, `Object`, and `Resource` are reserved for future expression-level standard-library/helper APIs. They are not primitive type spellings, and primitive type names are not namespaces. Future code should prefer APIs such as `Int::parse(...)`, but companion semantics are not part of the current implementation.

The fixed-width numeric type family is accepted in `docs/decisions/0016-fixed-width-numeric-types.md`. Current compiler support may lag this accepted direction until the lexer, parser, semantic type model, Doria IR, and backends are updated.

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

Numeric widening is not implemented yet; for now `float` is not assignable from `int`, and `int` is not assignable from `float`. The accepted fixed-width numeric family also does not imply implicit widening, narrowing, or scalar coercion. Any future safe numeric widening should be a separate design decision. Named arguments and richer call argument representation are separate future slices.

Simple collection literals infer collection element/key/value types when all clear parts match. Clear heterogeneous collection literals, such as `[1, "two"]`, are rejected by narrow collection alias assignment checks rather than being erased to `Unknown`. The empty literal `[]` stays ambiguous so typed contexts may use it as an empty `List<T>` or `Dictionary<K, V>`. The PHP-compatible `array` annotation remains broad enough to accept list-shaped and dictionary-shaped literals for now, but `array` is not the desired long-term collection model.

### Equality and boolean operators

Doria equality is typed:

```doria
==
!=
```

`==` is typed equality. `!=` is typed inequality. Doria does not use PHP-style loose comparison, so expressions such as `1 == "1"` and `false == 0` are type errors rather than truthy comparisons. Doria does not use PHP strict-comparison operators; `===` and `!==` are not part of Doria syntax.

Accepted boolean operators are:

```doria
!
not

&&
and

||
or

xor
```

`not` is an exact synonym for `!`, `and` is an exact synonym for `&&`, and `or` is an exact synonym for `||`. Doria does not copy PHP's lower-precedence `and` / `or` behavior. Boolean operators require `bool` operands, and conditions must be `bool`; Doria does not use PHP-style truthiness.

`xor` is bool-only boolean exclusive OR. It evaluates both operands and does not short-circuit. It is not bitwise XOR. Unparenthesized chained `xor` and `xor` mixed with `and`, `or`, `&&`, or `||` should require parentheses or produce a diagnostic/lint when implemented.

Accepted bitwise operators are:

```doria
&
|
^
~
```

`&`, `|`, `^`, and `~` are integer bitwise operators. They are not boolean operators, and `&` / `|` are not aliases for boolean AND/OR. Doria does not add `^^`.

Do not add `nand`, `nor`, `implies`, `iff`, `unless`, `^^`, `===`, or `!==` as core syntax without a new accepted decision. Future helper APIs such as `Bool::all(...)`, `Bool::any(...)`, `Bool::none(...)`, or `Bool::one(...)` may be considered separately.

The accepted boolean/equality/bitwise operator direction is recorded in `docs/decisions/0020-boolean-operators-and-given-predicates.md`. Current compiler support includes typed `==` / `!=` checking, rejection of `===` / `!==`, `not` / `and` / `or` / `xor` parsing, bool-only semantic checking, Doria IR lowering, PHP backend lowering for the supported subset, and Stage 6c native lowering for supported `if` / `while` conditions and writable integer assignment in supported native blocks. Bitwise operators and broader native expression lowering remain future implementation work.

### Control-flow conditions

Basic `if` / `else if` / `else` and `while` are MVP syntax. Conditions must be `bool`; Doria does not use PHP-style truthiness for integers, strings, null, objects, resources, or collections. The checker currently allows `mixed` and the internal `Unknown` recovery type so one diagnostic does not cascade into unrelated follow-up errors.

Each `if`, `else if`, `else`, and `while` body has its own block scope. Variables declared inside those bodies are not visible after the block. Until Doria has path-sensitive definite initialization analysis, constructor readonly init access is available only for direct constructor-body assignments and not inside `if`, `else if`, `else`, or `while` bodies.

`if` is statement control flow and does not return a value. `if` without `else` is valid Doria. `else`, `else if`, `given`, and `finally` are optional. A base `if`, `while`, `foreach`, or future control construct does not require `given` or `finally`.

`when` is the planned value-returning conditional/control construct. `when`, `given`, and `finally` are accepted design direction but are not implemented in the current compiler slice.

### given predicate blocks

A `given` block attached to a control construct may contain variable declarations, void expression statements, and bool expression statements. Bool expression statements are predicates. Void expression statements are setup actions. Variable declarations introduce scoped names available to the attached control construct. Non-bool, non-void discarded expressions should be rejected.

```doria
given {
    let $user = $session->user;
    let $permission = Permission::EditPost;

    $user->isActivated;
    $this->isOrgMember($user) || $this->isAdmin($user);
} if ($user->can($permission)) {
    $post->publish();
}
```

Separate bool predicate lines are implicitly AND-ed in source order with the attached control condition. Bool predicates short-circuit the attached condition and body when false. Inside a predicate, normal boolean short-circuiting applies for `&&` / `and` and `||` / `or`; `xor` does not short-circuit.

The scoped declarations remain scoped to the whole `given` plus attached control construct. The exact lowering, borrow/lifetime interaction, cleanup behavior, and `finally` execution guarantees remain future decisions.

## 8. Class syntax

Doria's accepted OOP declaration vocabulary is recorded in `docs/decisions/0029-oop-declaration-vocabulary.md`. Current compiler support may lag this accepted direction until lexer, parser, semantic checking, Doria IR, backend, and LSP support are updated.

Accepted OOP declaration vocabulary:

```text
class
interface
trait
extends
implements
```

`class` declares an object type. Doria already has class syntax in the current compiler surface:

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

Doria will support `interface` for contracts that classes can implement:

```doria
interface Renderable
{
    function render(): string;
}
```

Interfaces may declare method requirements and may extend one or more interfaces. Interface members do not define instance storage. Default methods, static interface methods, constants, generic interfaces, variance, and interface property requirements remain future design work.

Doria will support `trait` for reusable class-body members:

```doria
trait HasSlug
{
    string $slug;
}
```

Traits may be composed into classes or other traits with `uses`. Trait conflict-resolution rules, aliasing, visibility changes through trait composition, trait property rules, trait static member rules, trait abstract method requirements, and whether PHP-style `insteadof` / `as` are accepted exactly remain future design work.

Doria will support `extends` for inheritance:

```doria
class Post extends Model
{
}

interface JsonRenderable extends Renderable
{
}
```

Likely direction: a class may extend at most one class, and an interface may extend one or more interfaces. Constructor inheritance, initialization order, override rules, virtual dispatch layout, final/sealed behavior, runtime layout, and ABI remain future design work.

Doria will support `implements` for compiler-checked interface conformance:

```doria
class Post extends Model implements Renderable, JsonSerializable
{
}
```

Likely direction: a class may implement one or more interfaces, and Doria's PHP-shaped direction points toward nominal interface conformance. Exact conformance checking details remain future implementation work.

`use` and `uses` have distinct meanings:

```text
namespace/file-scope use  -> semantic import / alias
class-body/trait-body uses -> trait composition
```

```doria
namespace App\Posts;

use App\Models\Post;
use App\Security\Permission;

class Article
{
    uses HasSlug;
}
```

The parser can distinguish namespace/file-scope import `use` from class-body or trait-body trait-composition `uses` by spelling and context. Neither form is implemented by the current compiler slice.

Doria is PHP-shaped, not PHP++. Accepting PHP-shaped OOP declaration syntax does not import PHP dynamic object semantics, magic methods as core behavior, autoloading behavior, reflection behavior, loose typing, PHP visibility rules beyond what Doria has separately accepted, PHP trait conflict-resolution rules without review, or PHP runtime initialization behavior.

OOP declaration vocabulary is accepted separately from final visibility semantics. Doria's accepted early member model remains default-public plus `internal`: class members are accessible by default, `internal` controls API surface, and `writable` controls mutation.

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

The native backend is the primary target. It should lower Doria IR, and any later native-oriented IR, toward native machine code and standalone executables. The current Cranelift-backed Stage 6c native backend is deliberately limited to exactly one top-level `function main(): int` with supported readonly and writable integer locals, `=`, `+=`, and `-=` assignments to writable integer locals, `+`/`-`/`*` arithmetic, structured returning `if` blocks, fallthrough `if` statements with visible-local merges, and bounded structured `while` loops in `0..125`. Supported native `while` bodies may contain integer local declarations, writable integer assignments, and fallthrough `if` statements. Stage 6c validates loop termination, loop-body scoping, fallthrough branch state merging, and checked integer arithmetic before native lowering, then emits real Cranelift control flow. Stage 7a keeps that source support unchanged while separating native smoke validation, compile-time smoke evaluation/proof, and Cranelift lowering behind a private `NativeSmokeModule` boundary. That module is not public Doria IR, final MIR, or a permanent local storage model. The native loop verification cap is a backend support limit, not Doria language semantics. Stage 6c conditions support bool literals, grouped conditions, integer comparisons over supported integer expressions, `!` / `not`, `&&` / `and`, `||` / `or`, and `xor`. It emits unsupported-feature diagnostics for general loops beyond the bounded Stage 6c shape, nested `while`, returns inside `while`, `return` inside fallthrough branch bodies, `break`, `continue`, non-integer locals, division/modulo, strings, classes, collections, and broader valid Doria until later native slices are designed.

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
- Class inheritance through `extends`, interface conformance through `implements`, and class-body/trait-body `uses` trait composition.
- `use` statements for semantic imports and aliases.
- `include` as required include-once compile-time source inclusion.
- `declare` as structured compiler/source directives.
- Attribute syntax and metadata representation.
- Richer instance property initializers.
- Named arguments.
- Full path-sensitive control-flow analysis for returns and constructor initialization.
- Advanced control-flow design for `do ... while ... finally`, `given ... when`, `given ... while`, `if` chains with possible `finally`, value-returning `when`, `match`, `break`, and `continue`.
- Careful evaluation of `goto`, labeled loop control, and structured conditional compilation without adopting C/C++ textual macros.
- Async/await and structured concurrency.
- Broader native backend design and implementation beyond the Stage 6c smoke target.
- Native-oriented IR implementation when native code generation needs it.
- Broader native code generation and standalone executable production beyond the Stage 6c smoke target.
- Self-hosting path for writing more of `doriac` in Doria.
- PHP-to-Doria migration tooling.
- Package management.
