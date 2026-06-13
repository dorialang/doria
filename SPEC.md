# Doria Language Specification

This document describes the v0.1 direction for Doria.

## 1. What Doria is

Doria is a PHP-shaped, statically checked, compiled programming language. It keeps familiar PHP surface syntax where that helps migration, including `$variables`, classes, functions, visibility modifiers, constructor property promotion, arrays, and C-like blocks.

Doria source files use the `.doria` extension and do not require `<?php` tags.

The compiler is `doriac`, implemented in Rust. Doria's long-term primary target is native machine code and standalone executables.

The compiler architecture is backend-independent:

```text
Doria source
-> lexer
-> parser
-> AST
-> semantic analysis
-> type checker
-> readonly/writable checker
-> borrow/lifetime analysis later
-> HIR
-> MIR later
-> backend
```

Backends may include:

- Native backend.
- PHP backend.
- Debug/interpreter backend.
- WebAssembly backend.

The PHP backend is a compatibility, migration, debugging, and transpilation target. It must not shape the core compiler architecture.

## 2. What Doria is not

Doria is not PHP++ and is not required to parse every valid PHP program.

Doria is PHP-shaped, not PHP-compatible at the parser level.

Valid PHP should be easy to migrate to Doria, but Doria-specific syntax does not need to run directly in PHP.

The v0.1 compiler does not yet produce native executables, and it is not yet a package manager, reflection system, macro system, async runtime, or full standard library.

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
- `echo`, `return`, and `foreach`.
- Assignments.
- Function calls, method calls, property access, object construction, and literals.
- List and dictionary literals using PHP-like array syntax.

## 4. Declaration rules

Variables must be declared before use.

```php
let $name = "Andrew";
let writable $count = 0;

string $city = "Lusaka";
writable int $score = 0;
```

Bare assignment never declares a variable:

```php
$name = "Andrew"; // error
```

## 5. Readonly and writable rules

Everything is readonly unless explicitly marked `writable`.

```php
let $x = 5;
$x = 10; // error

let writable $y = 5;
$y = 10; // ok
```

Explicit typed declarations follow the same rule:

```php
int $x = 5;
$x = 10; // error

writable int $y = 5;
$y = 10; // ok
```

Properties are readonly by default:

```php
class Person
{
    public string $id;
    public writable string $name;
}
```

To assign to a property, both the object path and the property must be writable.

Function parameters are readonly by default and become writable only with `writable`.

Methods receive readonly `$this` by default. A method that mutates `$this` must be declared with `writable function`.

## 6. Basic type system

The MVP type names are:

```text
void
int
float
string
bool
null
mixed
List<T>
Dictionary<K, V>
Set<T>
ClassType
Unknown
```

`let` declarations infer simple literal and constructor types:

```php
let $x = 5;        // int
let $name = "Doria"; // string
let $person = new Person("Andrew"); // Person
```

## 7. Class syntax

```php
class Person
{
    public function __construct(
        public writable string $name,
        public int $age,
    ) {
    }
}
```

Constructor property promotion is supported in the parser:

```php
public function __construct(
    public writable string $name,
    public int $age = 10,
) {
}
```

Constructor init access for assigning readonly properties inside constructor bodies is a required language rule, but it is not implemented in the current vertical slice. The intended rule is narrower than writable `$this`: a constructor may initialize each uninitialized readonly property exactly once.

## 8. Function syntax

```php
function greet(string $name): void
{
    echo "Hello, {$name}";
}
```

Parameters are readonly unless marked `writable`:

```php
function rename(writable Person $person, string $name): void
{
    $person->name = $name;
}
```

## 9. Collection aliases

Doria uses:

```php
List<int>
Dictionary<string, int>
Set<string>
```

Do not use `Vec`.

The PHP backend lowers these aliases to PHP arrays, while the Doria type checker keeps them distinct.

## 10. HIR, MIR, and backend behavior

After semantic analysis, type checking, and readonly/writable checking, the compiler currently lowers the checked AST to HIR. HIR is still close to source structure and is not the final backend IR.

MIR is planned as the simpler, control-flow-oriented representation for borrow/lifetime analysis and native-oriented backend lowering.

The native backend is the primary long-term target. It should eventually lower MIR toward native machine code and standalone executables.

The PHP backend is currently the first implemented backend. It emits `<?php` and lowers Doria-only syntax away:

- `let` is removed.
- `writable` is removed.
- Collection aliases are emitted as `array`.
- Doria readonly/writable rules are enforced before HIR/MIR lowering and backend emission, not at PHP runtime.

## 11. Future features

Future work includes:

- Better diagnostics with suggestions.
- Nullable types.
- Full type inference for lists and dictionaries.
- Interfaces, traits, and namespaces.
- Async/await and structured concurrency.
- Native backend design and implementation.
- MIR implementation.
- Native code generation.
- Package management.
