# Doria Language Specification

This document describes the v0.1 direction for Doria.

Documentation role: current language specification. This file records Doria language rules and current implementation status where useful; it is not a parallel roadmap. Future implementation sequencing belongs to `docs/doria-end-to-end-plan.md`, and topic-level accepted decisions belong to `docs/decisions/`.

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

The current compiler implementation lowers the accepted native subset through validated typed MIR. The debug interpreter, default Cranelift fast profile, and `--release` LLVM profile consume that same MIR, and the durable executable parity manifest compares exact stdin-driven stdout bytes, stderr bytes, process status, and declared file side effects across all three paths. The supported subset includes top-level free functions; parameterless int/void `main`; structured control flow and recursion; fixed-width numerics and bool; immutable UTF-8 strings; the narrow Stage 17 `?string` seed; checked formatting; UTF-8 line/file I/O; exact stdout/stderr; and fatal panic. Native strings are private non-atomic refcounted buffers and are Copy at the source level. `main(): int` crosses the accepted `0..125` process boundary and `main(): void` maps normal completion to status `0`. Release optimization does not change observable semantics. `doria-rt` owns process entry, runtime strings, raw standard-device I/O, line discipline, text-file I/O, exact output, panic formatting, and Doria stack traces. Collections, methods/classes, general ownership/borrow checking, arbitrary-expression interpolation/`Displayable`, general nullable types, and `Bytes` remain unsupported. The former Stage 7-10 native smoke module remains retired.

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
- `echo`, `return`, `foreach`, `for`, `if` / `else if` / `else`, and `while`.
- Assignments.
- Function calls, method calls, property access, object construction, and literals.
- Collection literals using bracket syntax.

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
- checked `throw` / `throws` error handling.

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

Traditional `for` loops are accepted for explicit counter/index iteration:

```doria
for (let writable $i = 0; $i < 10; $i++) {
    echo $i;
}
```

`foreach` is preferred for collections and ranges. Integer ranges use `..` for inclusive ranges and `..<` for exclusive-end ranges:

```doria
foreach (0..10 as $i) {
    echo $i;
}

foreach (0..<10 as $i) {
    echo $i;
}
```

`0..10` produces `0` through `10`. `0..<10` produces `0` through `9`. Range endpoints must be `int` expressions. The variable after `as` is a readonly loop-local binding for each iteration and does not leak outside the `foreach` body.

Standalone `++` and `--` mutation statements require a declared writable `int` target:

```doria
$i++;
++$i;
$i--;
--$i;
```

Value-producing `++` / `--` expression semantics are future work.

`declare` is a structured compiler/source directive. It is not a macro system and not textual substitution. Exact grammar and allowed declaration keys require future decisions. Unknown declare keys should be rejected when `declare` is implemented. Possible future uses include warning policy, unsafe/FFI boundary policy, backend/profile constraints, platform configuration, optimization intent, feature gates, and compile-time diagnostics.

`goto` is evaluation-only and is not accepted for implementation yet. If it is ever accepted, it should be constrained so it cannot jump into deeper scopes, bypass visible initialization, bypass cleanup or `finally` obligations, cross guarded resource regions, or cross future ownership/borrow-checking boundaries.

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

Every parameter in Doria source has an explicit type. This applies to all function-like parameter lists: free functions, methods, constructors, anonymous functions, arrow functions, interface requirements, trait requirements, property hook setters, and future callback-style declarations. Doria does not infer omitted parameter types in any context.

Valid:

```doria
let $double = fn(int $x) => $x * 2;

let $format = function (int $score): string {
    return "score: {$score}";
};
```

Omitting a parameter type is a syntax or semantic error even when the surrounding expression makes the intended type obvious.

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

Internal members are accessible only from methods and constructors of the declaring class. They are not accessible from top-level code, free functions, or other classes. Protected is permanently excluded from Doria; inheritance does not add a third access tier.

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

### Naming charter

Doria chooses casing by API category, not by whether an implementation is built into the language:

- Built-in free functions use `snake_case`, such as `get_time()` and `str_starts_with()`.
- Userland free functions, instance methods, static methods, companion/type APIs, properties, parameters, and named arguments use `camelCase`.
- Classes, interfaces, traits, enums, and enum cases use `PascalCase`.
- Constants use `SCREAMING_SNAKE_CASE`.
- Type parameters use single Pascal capitals such as `T`, `K`, and `V`.
- PHP-shaped magic methods retain their inherited spellings: `__construct` and `__destruct`.

Free-function casing and member/companion casing are intentionally different:

```doria
let $now = get_time();
let $matches = str_starts_with($name, "Dor");
let $wrapped = Int::wrappingAdd(1, 2);
let $empty = $s->isEmpty();
let $tenant = $message->tenantId;
$message->retryAfter(seconds: 30);
let $person = $repository->findById($id);
```

## 7. Basic type system

Accepted type-position names include:

```text
void (return position only)
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
mixed
T[]
List<T>
Dictionary<K, V>
Set<T>
ClassType
```

Reserved or rejected names:

```text
null      literal only; nullable values are spelled ?T
resource  reserved for Phase I PHP interop; rejected until specified
object    not a Doria type
array     not a Doria type; use T[] for typed arrays or collection aliases
```

The compiler keeps parsed type syntax and semantic types separate:

```text
TypeRef      parsed source spelling, such as `List<int>` or `Person`
TypeId       resolved semantic type identity
TypeKind     resolved semantic type shape
```

The semantic model also has an internal `Unknown` recovery type for diagnostics and error recovery; it is not the normal spelling for user-authored type declarations.

Lowercase primitive names are type-position names: `int`, `int8`, `int16`, `int32`, `int64`, `uint8`, `uint16`, `uint32`, `uint64`, `float`, `float32`, `float64`, `string`, and `bool`. PascalCase names are expression-level standard-library/helper or compiler-known companion APIs, not primitive type spellings or namespaces.

### Fixed-width integers

Stage 13 implements these canonical integer types through semantic analysis, typed MIR, the debug interpreter, and Cranelift:

```text
int8   int16   int32   int64
uint8  uint16  uint32  uint64
```

`int` is signed 64-bit. `int64` is an exact source alias of `int`; they have one canonical semantic and runtime type. Doria has no bare `uint`, no pointer-width integer type, and no Rust-style `i8`/`u8`/`usize`/`isize` spellings.

Stage 14 implements `float32` as IEEE 754 binary32 and canonical `float`/`float64` as IEEE 754 binary64. `float` and `float64` are one semantic/runtime type; `float32` remains distinct, with no implicit width or integer conversion. Decision 0072 defines arithmetic, comparisons, special values, literal rounding, bool runtime values, and backend behavior.

An unconstrained decimal integer literal defaults to `int`. A literal may instead adopt an expected integer type from a declaration, parameter, return, assignment, or typed binary operand when its mathematical value fits that type. Contextual literal typing is not an implicit conversion. Out-of-range literals are compile-time errors. Stage 13 adds no numeric suffixes and no hexadecimal, octal, or binary literal syntax.

Both operands of an integer binary operator must resolve to the same canonical integer type. Nonliteral values never widen or narrow implicitly, and Doria has no C-style integer promotions. The implemented integer operators are:

```text
-  ~
+  -  *  /  %
<<  >>
&  ^  |
==  !=  <  <=  >  >=
++  --
+=  -=  *=  /=  %=  <<=  >>=  &=  |=  ^=
```

`+`, `-`, `*`, and signed negation are checked. Signed overflow, unsigned overflow, and unsigned underflow panic. Signed division truncates toward zero; division by zero panics, and signed minimum divided by `-1` panics. Signed remainder uses that quotient and gives a nonzero remainder the dividend's sign; remainder by zero panics, while signed minimum remainder `-1` is zero. Unsigned division and remainder use ordinary unsigned arithmetic.

Shift operands have one canonical integer type. A negative signed shift count or a count greater than or equal to the left operand's width panics. Left shift discards bits beyond the fixed width after validating the count. Signed right shift is arithmetic; unsigned right shift is logical. `&`, `|`, `^`, and `~` operate on the fixed-width two's-complement bit pattern and do not overflow. The word `xor` remains the distinct bool-only operator.

Explicit integer conversion uses compiler-known PascalCase companion intrinsics:

```doria
Int::from($value)
Int8::from($value)
Int16::from($value)
Int32::from($value)
Int64::from($value)
UInt8::from($value)
UInt16::from($value)
UInt32::from($value)
UInt64::from($value)
```

`Int` and `Int64` target the same canonical `int64` type. Each `from` accepts exactly one integer expression. Same-type and exact widening conversions preserve the value; narrowing and signedness-changing conversions are checked and panic with `integer conversion out of range` when the value cannot be represented. Stage 13 adds no `as` cast and no wrapping, saturating, or unchecked conversion API.

The exact operator, panic, and conversion rules are authoritative in decisions 0041 and 0042. The PHP compatibility backend supports only integer shapes it can preserve exactly; it emits a backend unsupported-feature diagnostic for precise Stage 13 behavior that PHP cannot represent rather than changing Doria semantics.

### Dynamic boundary type

`mixed` is Doria's only dynamic type. It has three laws:

1. `mixed` is unknown-flavored, never any-flavored. A `mixed` value permits no property access, method calls, arithmetic, concatenation, interpolation, comparison, or other typed operation until it is narrowed with `is` or `match`.
2. Any value may flow into `mixed` implicitly. This is the deliberate dynamic-boundary exemption from the no-implicit-conversion rule. Values do not flow out of `mixed` implicitly; source must narrow first. There is no cast spelling.
3. `mixed` is a boxed, runtime-tagged move type, always, even when the payload is a Copy value.

```doria
mixed $payload = Json::decode($line);

let $label = match ($payload) {
    string $value => $value,
    int $value => Int::toString($value),
    default => "unknown",
};
```

`object` does not exist in Doria. Use `mixed` for dynamic object-shaped boundaries and narrow with `is` or `match`.

`null` is a literal, not a type-position name. The internal null type exists for nullable machinery, but source spells nullable values as `?T`.

`resource` is reserved for Phase I PHP bridge work and is rejected until the bridge specifies its exact semantics.

`void` is valid only as a function or method return type, including `main(): void`; it is not a value type.

Typed arrays use C-style suffix spelling:

```text
T[]
```

Examples:

```doria
int[] $numbers = [1, 2, 3];
string[] $names = [];
int[][] $matrix = [[1], []];
```

`array` is not a Doria type-position name.

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

Interpolated values may currently be `string`, a fixed-width integer, `float`, `bool`, or the internal `Unknown` recovery type. They use the same canonical display conversion as `echo` and `.`. Null, class values, `mixed`, `List<T>`, `Dictionary<K, V>`, and `Set<T>` are rejected. Arbitrary expression interpolation and `Displayable` remain Stage 18.

The `.` operator is runtime string concatenation. Each operand may be a display-convertible primitive, but at least one operand of that binary operation must already be statically `string`; therefore `"x=" . 1` is valid while `1 . 2` is rejected. The result is `string`, evaluation is left-to-right, and no conversion is implied outside display contexts. `echo`, `.`, and current interpolation parts use decimal integers, shortest-round-trip locale-independent binary32/binary64 floats, lowercase `true`/`false`, and strings unchanged.

There is no implicit widening, narrowing, or scalar coercion between distinct integer or float types. `float` is not assignable from an integer and an integer is not assignable from `float`. Stage 14 provides only `Int::toFloat(int): float` and checked `Float::toInt(float): int`; decision 0042 defines their exact contracts. Named arguments remain a separate future slice.

Simple collection literals infer collection element/key/value types when all clear parts match. Clear heterogeneous collection literals, such as `[1, "two"]`, are rejected by typed array and narrow collection alias assignment checks rather than being erased to `Unknown`. The empty literal `[]` stays ambiguous so typed contexts may use it as an empty `T[]`, `List<T>`, or `Dictionary<K, V>`.

### Stage 17 text I/O and checked formatting

Stage 17 provides these compiler-known built-ins:

```doria
read_line(): ?string
sprintf(string $format, ...): string
printf(string $format, ...): void
read_file(string $path): string
write_file(string $path, string $contents): void
write_stderr(string $value): void
```

`read_line` reads UTF-8 text, removes one LF ending and a preceding CR when present, preserves empty lines and final unterminated lines, and returns `null` only when EOF occurs before any bytes. Its return type is the first supported position for the nullable `?T` model specified for Stage 22, not an I/O-only replacement type. A `!= null` guard narrows `?string` to `string`; assigning `null` or another nullable result invalidates that fact, while assigning a known `string` establishes a new non-null fact. Stage 22 generalizes the same nullable model to other type positions and null-safe operations.

`read_file` and `write_file` are text-file functions. `read_file` reads an entire file and validates UTF-8 before constructing a `string`; invalid bytes never enter a Doria string. `write_file` creates or truncates a text file and writes the string's exact bytes. `write_stderr` writes exact bytes without adding a newline. Stage 17 I/O failures, including invalid UTF-8 and operating-system read/write failures, use the fatal panic path with a clear message; `null` from `read_line` means EOF and never signals an error. At Stage 29 these free functions migrate to declared `throws` signatures when checked errors are implemented.

Binary file I/O is planned for Stage 23 as `read_file_bytes(...)` and `write_file_bytes(...)`. Their parameter contracts remain a Stage 23 design decision. `File` and stream objects, including RAII close and buffered/seekable access, are planned after Stage 29. These future tiers do not change the Stage 17 text and EOF contracts.

`sprintf` and `printf` require a direct literal format in Stage 17. The compiler parses it into a validated MIR plan before any backend runs. Accepted conversions are `%s`, `%d`, `%f`, `%x`, `%X`, `%o`, `%b`, and `%%`; accepted controls are decimal field width, `-` left alignment, `0` numeric zero padding, and `.N` precision on `%f`. Width for `%s` counts UTF-8 bytes. Formatting is deterministic and locale-independent. `printf` uses the same plan, returns `void`, and adds no newline. `print` is rejected in favor of `echo`; dynamic/positional formats, `*` width, `%e`, `%g`, and `sscanf` are not accepted.

The runtime separates raw standard-device reads/writes and explicit flush from buffered line discipline. It detects stdin, stdout, and stderr interactivity independently for internal use. On Windows, interactive console text uses validated UTF-8 converted to wide console operations; redirected handles preserve exact UTF-8 bytes. This is infrastructure for the future Stage 46 `Console` API, not a public terminal API. Binary data and `Bytes` remain Stage 23, and arbitrary-expression interpolation plus `Displayable` remain Stage 18.

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

The accepted boolean/equality/bitwise operator direction is recorded in decisions 0020, 0041, and 0072. Current compiler support includes typed scalar equality, rejection of `===` / `!==`, runtime bool locals/parameters/returns/calls, value- and condition-position short-circuit `not`/`and`/`or`, eager `xor`, fixed-width integer bitwise operators, typed MIR lowering, and native execution. PHP lowers only its exact supported subset.

### Control-flow conditions

Basic `if` / `else if` / `else` and `while` are MVP syntax. Conditions must be `bool`; Doria does not use PHP-style truthiness for integers, strings, null, dynamic boundaries, or collections. The checker currently allows the internal `Unknown` recovery type so one diagnostic does not cascade into unrelated follow-up errors.

Each `if`, `else if`, `else`, and `while` body has its own block scope. Variables declared inside those bodies are not visible after the block. Until Doria has path-sensitive definite initialization analysis, constructor readonly init access is available only for direct constructor-body assignments and not inside `if`, `else if`, `else`, or `while` bodies.

`if` is statement control flow and does not return a value. `if` without `else` is valid Doria. `else`, `else if`, `given`, and `finally` are optional. A base `if`, `while`, `foreach`, or future control construct does not require `given` or `finally`.

`when` is the planned value-returning conditional/control construct. `when`, `given`, and `finally` are accepted design direction but are not implemented in the current compiler slice.

### Checked errors

The accepted checked error direction is recorded in `docs/decisions/0035-checked-throw-throws-direction.md`.

`throw` raises an error. `throws` declares possible thrown error types in function and method signatures. Thrown errors are checked by the compiler: callers must catch thrown errors or declare them in their own `throws` clause.

`Result<T, E>` is not Doria's default surface error model unless a later accepted decision explicitly adopts it. Runtime panic or fatal-error behavior is separate from checked `throw` / `throws`.

### Panic

`panic("message");` invokes a compiler-known built-in free function that terminates execution. Panic is fatal, is not catchable, does not unwind, and does not run cleanup or destructors while aborting in v1.0. User code cannot redeclare `panic`.

The current compiler accepts a string literal, readonly compile-time-known string local, or concatenation of those expressions as the panic message. Panic writes a deterministic first line and a Doria function-name stack trace to stderr, then exits with status 101:

```text
Panic: <message>
Stack Trace:
  at <currentFunction>
  at <callerFunction>
  at main
```

Checked integer addition, subtraction, multiplication, and signed negation overflow use this panic path for every integer width. Division by zero, signed division overflow, remainder by zero, an out-of-range shift count, and an out-of-range explicit conversion use the same path with the deterministic messages in decisions 0041 and 0042. Returning a process status outside `0..125` from `main(): int` also panics. Panic is a runtime outcome, not a compiler diagnostic or malformed-MIR error.

Compiler implementation for `throw`, `throws`, `try`, and `catch` is future work.

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

The scoped declarations remain scoped to the whole `given` plus attached control construct. The exact lowering, ownership/borrow-checker interaction, cleanup behavior, and `finally` execution guarantees remain future decisions.

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

`class` declares a class type. Doria already has class syntax in the current compiler surface:

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

OOP declaration vocabulary is accepted separately from final visibility semantics. Doria's accepted early member model remains default-accessible plus `internal`: class members are accessible by default, `internal` controls API surface, and `writable` controls mutation.

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

Top-level function names beginning with `__doria_` are reserved for compiler-generated helpers.
The prefix does not reserve method names or otherwise change Doria's member model.

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

`void` functions and methods may use `return;` or fall through. Lifecycle methods, currently `__construct` and `__destruct`, are void-like: they may omit a return type or explicitly declare `: void`, may use bare `return;`, and may fall through. A non-`void` lifecycle return annotation is an error, returning a value from a `void` function or lifecycle method is an error, and lifecycle methods cannot be called directly as ordinary methods. `__construct` may declare parameters and constructor calls are checked against them through `new Class(...)`. `__destruct` must not declare parameters.

For declared non-`void` return types, no reachable path may fall through the function body. `return` may occur anywhere in nested control flow. A path ending in `panic()` or a proven non-terminating `while (true)` loop without a reachable `break` is diverging and does not require a return. A loop with a reachable exit must lead to a return or another diverging path. Missing-return diagnostics are produced by path-sensitive source control-flow analysis before MIR lowering.

The program entrypoint may be `main(): int` or `main(): void`. `main(): int` returns an explicit process status. `main(): void` may fall through or use `return;` and maps normal completion to successful status `0`. Returning a value from `main(): void` is the same semantic error as returning a value from any other `void` function.

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
int[]
List<int>
Dictionary<string, int>
Set<string>
```

Do not use `Vec`.
Do not use `array` as a type spelling.

The current PHP compatibility backend may lower typed arrays and collection aliases to PHP arrays, while the Doria type checker keeps them distinct. The native backend must make deliberate representation choices for typed arrays and each collection family rather than inheriting PHP array behavior.

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

MIR is Doria's native-oriented, backend-independent control-flow representation for the executable subset. It contains typed scalar and string locals, parameters, calls and returns; runtime string literal/local/call/concatenation/display expressions; string comparison; basic blocks; checked numeric operations/conversions; and panic termination. The debug interpreter uses safe private string values, an explicit heap-backed Doria frame stack, and exact stdout/stderr buffers. It models source value behavior, not native refcount layout. Ordinary interpretation has no fixed execution-fuel or call-depth cap and does not reject repeated states.

Native is the primary target. Checked HIR lowers to typed MIR, shared MIR validation gates both native lowerers, Cranelift emits the default fast object, LLVM 18 emits the O3 `--release` object, and the host linker combines either object with `doria-rt`. Native compilation has no interpreter preflight, fallback IR, or release-to-fast fallback. `doria-rt` owns entry policy, immutable refcounted runtime strings, Stage 17 text I/O and formatting support, exact stdout/stderr writes, abort-only panic formatting, stack traversal, and status 101. Both lowerers share scalar and opaque string ABI conventions and clean up owned string locals on normal returns. Runtime failures use the shared panic path. Only canonical int/void entry results cross the process boundary. Unsupported coverage remains for collections, classes/methods, general ownership checking, Stage 18 displayable classes/arbitrary interpolation, Stage 22 general nullable types, and Stage 23 `Bytes`.

The PHP backend is currently implemented as a compatibility/debugging backend. It emits `<?php` and lowers Doria-only syntax away:

- `let` is removed.
- `writable` is removed.
- `internal` is enforced by Doria before backend emission and may lower to PHP `private` or another backend-specific representation.
- Typed arrays and collection aliases are emitted as `array` for the current PHP backend only.
- Doria readonly/writable rules are enforced before Doria IR lowering and backend emission, not at PHP runtime.
- `int`/`int64` remain the exact supported signed-integer alias subset.
- Checked arithmetic, nondefault widths, unsigned semantics, division/remainder, shifts/bitwise operations, and integer companion conversions produce a clear backend unsupported-feature diagnostic whenever PHP cannot preserve the Doria behavior exactly.

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
- Constructor definite-initialization analysis on the reusable control-flow/dataflow framework.
- Advanced control-flow design for `do ... while ... finally`, `given ... when`, `given ... while`, `if` chains with possible `finally`, value-returning `when`, `match`, and labeled or numeric loop control.
- Careful evaluation of `goto`, labeled loop control, and structured conditional compilation without adopting C/C++ textual macros.
- Async/await and structured concurrency.
- Public stream/file objects, binary I/O, and terminal APIs beyond the Stage 17 text helpers.
- Self-hosting path for writing more of `doriac` in Doria.
- PHP-to-Doria migration tooling.
- Package management.
