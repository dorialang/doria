# Doria compiler plan for Codex

## Project name

**Doria**

Doria is a PHP-shaped, C-like, compiled language inspired by PHP syntax but with stronger safety guarantees:

```text
- Strong static typing
- Type inference through let
- Variables declared only with let or explicit types
- Everything readonly by default
- writable keyword for intentional mutation
- Classes, functions, methods, visibility, constructor promotion
- Collection aliases: List<T>, Dictionary<K, V>, Set<T>
- Future: generics, borrow checker, async/await, native backend
```

The first target should be a **Doria-to-PHP compiler**, not a native compiler. The compiler should reject invalid Doria code before outputting PHP.

---

# Core language decisions

## 1. File extension

Use:

```text
.doria
```

Do not require `<?php` tags in Doria source files.

---

## 2. Variable declarations

Doria must not allow implicit declarations.

Valid declarations:

```php
let $name = "Andrew";
let writable $count = 0;

string $city = "Lusaka";
writable int $score = 0;
```

Invalid:

```php
$name = "Andrew"; // Error: undeclared variable
```

Bare assignment is only assignment to an existing variable.

---

## 3. Readonly by default

Everything is readonly unless explicitly marked `writable`.

```php
let $x = 5;

$x = 10; // Error
```

Writable variable:

```php
let writable $x = 5;

$x = 10; // Okay
```

Explicit type version:

```php
int $x = 5;

$x = 10; // Error
```

```php
writable int $x = 5;

$x = 10; // Okay
```

---

## 4. Properties are readonly by default

```php
class Person
{
    public string $id;
    public writable string $name;
    public writable int $age;
}
```

This means:

```php
$person->id = "new-id"; // Error
$person->name = "Lucy"; // Okay, if $person itself is writable
```

To assign to a property, both the object path and the property must be writable.

---

## 5. Function parameters are readonly by default

```php
function greet(Person $person): void
{
    echo $person->name;

    $person->name = "Lucy"; // Error
}
```

Writable parameter:

```php
function rename(writable Person $person, string $name): void
{
    $person->name = $name;
}
```

---

## 6. Methods receive readonly `$this` by default

```php
class Person
{
    public writable string $name;

    public function getName(): string
    {
        return $this->name;
    }

    public function rename(string $name): void
    {
        $this->name = $name; // Error
    }
}
```

To mutate `$this`, the method must be marked `writable`:

```php
class Person
{
    public writable string $name;

    public writable function rename(string $name): void
    {
        $this->name = $name;
    }
}
```

Usage:

```php
let $person = new Person("Andrew");

$person->rename("Lucy"); // Error: $person is readonly
```

```php
let writable $person = new Person("Andrew");

$person->rename("Lucy"); // Okay
```

---

## 7. Collection aliases

Use these names:

```php
List<int>
Dictionary<string, int>
Set<string>
```

Do **not** use `Vec`.

For the first PHP-emitting backend, these can compile to PHP arrays internally, but the Doria type checker should distinguish them.

Examples:

```php
List<int> $numbers = [1, 2, 3];

Dictionary<string, int> $items = [
    'apples' => 5,
    'oranges' => 10,
];

Set<string> $names = Set::from(["Dorothy", "Lucy"]);
```

MVP may support only `List<T>` and `Dictionary<K, V>`. `Set<T>` can be added after arrays and dictionaries work.

---

# Recommended implementation approach

Use **Rust** for the compiler.

The first compiler should be called:

```bash
doriac
```

Initial commands:

```bash
doriac check examples/person.doria
doriac compile examples/person.doria --target php --out build/person.php
doriac run examples/person.doria
```

For MVP, `doriac run` can compile to temporary PHP and execute it using the local `php` binary.

Codex should work in small, testable increments. OpenAI’s Codex docs describe Codex as an AI agent for writing, reviewing, and shipping code, so this plan is written as a sequence of scoped engineering tasks rather than one giant “build a language” task. ([OpenAI Help Center][4])

---

# Repository structure

Create this structure:

```text
doria/
├── AGENTS.md
├── README.md
├── SPEC.md
├── Cargo.toml
├── crates/
│   └── doriac/
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs
│           ├── lib.rs
│           ├── lexer.rs
│           ├── parser.rs
│           ├── ast.rs
│           ├── types.rs
│           ├── symbols.rs
│           ├── semantics.rs
│           ├── diagnostics.rs
│           ├── codegen_php.rs
│           └── source.rs
├── examples/
│   ├── hello.doria
│   ├── variables.doria
│   ├── person.doria
│   └── errors/
│       ├── undeclared_variable.doria
│       ├── readonly_assignment.doria
│       ├── readonly_property.doria
│       └── non_writable_method.doria
└── tests/
    ├── lexer_tests.rs
    ├── parser_tests.rs
    ├── semantic_tests.rs
    └── codegen_php_tests.rs
```

Create `AGENTS.md` with project-specific instructions for Codex. Codex supports generating an `AGENTS.md` scaffold through `/init`, but for this project, it should be explicit from the start. ([OpenAI Help Center][4])

---

# Phase 0: Write the initial spec

Create `SPEC.md`.

It should define:

```text
1. What Doria is
2. What Doria is not
3. MVP syntax
4. Declaration rules
5. Readonly/writable rules
6. Basic type system
7. Class syntax
8. Function syntax
9. Collection aliases
10. PHP backend behavior
11. Future features
```

Important wording:

```text
Doria is PHP-shaped, not PHP-compatible at the parser level.

Valid PHP should be easy to migrate to Doria, but Doria-specific syntax does not need to run directly in PHP.
```

---

# Phase 1: Lexer

Implement a lexer that recognizes:

```text
Keywords:
class
function
public
protected
private
static
let
writable
readonly
return
echo
new
foreach
as
if
else
while
for
true
false
null
void
int
float
string
bool
array

Future reserved:
async
await
spawn
scope
interface
trait
enum
match
try
catch
throw
Result
Option
```

Recognize tokens for:

```text
Identifiers:
Person
Dictionary
List
Set

Variables:
$name
$age
$this

Literals:
123
10.5
"hello"
'hello'
true
false
null

Operators:
=
+
-
*
/
%
.
+=
-=
==
===
!=
!==
<
<=
>
>=
&&
||
!
?
??
=>

Punctuation:
(
)
{
}
[
]
;
:
,
->
::
<
>
```

Important: generic type syntax uses `<` and `>`, so the parser must distinguish type context from comparison expressions.

Acceptance tests:

```php
let $x = 5;
let writable $name = "Doria";
Dictionary<string, int> $items = ['apples' => 5];
```

---

# Phase 2: Parser and AST

Implement a recursive-descent parser first. Do not use Tree-sitter or ANTLR for the compiler parser in the MVP.

Reason: handwritten parser gives tighter control over Doria-specific diagnostics. Tree-sitter is still useful later for editor support because its grammar DSL is designed for creating parsers and syntax trees, and ANTLR is also a proven parser generator, but neither is necessary for the first compiler milestone. ([tree-sitter.github.io][5]) ([antlr.org][6])

AST should support:

```text
Program
FunctionDecl
ClassDecl
PropertyDecl
MethodDecl
ConstructorDecl
Parameter
Block
Statement
Expression
TypeRef
```

Minimum statements:

```text
Variable declaration
Assignment
Expression statement
Return
Echo
Foreach
```

Minimum expressions:

```text
Variable
$this
Property access
Method call
Function call
New object
String literal
Int literal
Float literal
Bool literal
Null literal
Array/list literal
Dictionary literal
Binary expression
```

---

# Phase 3: Minimal type system

Implement these types first:

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

Support nullable types later:

```php
?string
?Person
```

Do not implement full union types in v0.1.

Type inference rules:

```php
let $x = 5;        // int
let $y = 10.0;     // float
let $z = "hello";  // string
let $ok = true;    // bool
```

Dictionary inference:

```php
let $items = [
    'apples' => 5,
    'oranges' => 10,
];
```

Should infer:

```php
Dictionary<string, int>
```

List inference:

```php
let $numbers = [1, 2, 3];
```

Should infer:

```php
List<int>
```

For mixed list values in MVP, either reject or infer `List<mixed>`. Prefer rejecting until the type system is stable.

---

# Phase 4: Symbol table

Implement lexical scopes.

The compiler must track:

```text
- Global functions
- Classes
- Class properties
- Methods
- Constructor parameters
- Local variables
- Function parameters
- Whether each binding is writable
- Whether each property is writable
- Whether each method mutates this
```

Rules:

```php
$x = 5;
```

Error:

```text
Cannot assign to undeclared variable `$x`.
Use `let $x = ...` or an explicit type declaration.
```

```php
let $x = 5;
$x = 6;
```

Error:

```text
Cannot assign to readonly variable `$x`.
Declare it as `let writable $x = ...` if mutation is intended.
```

---

# Phase 5: Readonly/writable checker

This is one of Doria’s most important differentiators.

Implement these checks before worrying about generics or async.

## Local variable reassignment

```php
let $x = 5;
$x = 6;
```

Error.

```php
let writable $x = 5;
$x = 6;
```

Okay.

## Explicit typed variable reassignment

```php
int $x = 5;
$x = 6;
```

Error.

```php
writable int $x = 5;
$x = 6;
```

Okay.

## Property assignment

```php
class Person
{
    public string $name;
}

let writable $person = new Person("Andrew");
$person->name = "Lucy";
```

Error because `name` is readonly.

```php
class Person
{
    public writable string $name;
}

let $person = new Person("Andrew");
$person->name = "Lucy";
```

Error because `$person` is readonly.

```php
class Person
{
    public writable string $name;
}

let writable $person = new Person("Andrew");
$person->name = "Lucy";
```

Okay.

## Method mutation

```php
class Person
{
    public writable string $name;

    public function rename(string $name): void
    {
        $this->name = $name;
    }
}
```

Error because `rename` is not a writable method.

Correct:

```php
class Person
{
    public writable string $name;

    public writable function rename(string $name): void
    {
        $this->name = $name;
    }
}
```

---

# Phase 6: PHP code generator

The first backend should emit PHP.

Doria:

```php
let $name = "Andrew";
echo $name;
```

PHP output:

```php
<?php

$name = "Andrew";
echo $name;
```

Doria:

```php
let writable $count = 0;
$count += 1;
```

PHP output:

```php
<?php

$count = 0;
$count += 1;
```

Doria class:

```php
class Person
{
    public writable string $name;
    public int $age;

    public function __construct(string $name, int $age)
    {
        $this->name = $name;
        $this->age = $age;
    }
}
```

PHP output:

```php
<?php

class Person
{
    public string $name;
    public int $age;

    public function __construct(string $name, int $age)
    {
        $this->name = $name;
        $this->age = $age;
    }
}
```

The PHP output does not need to preserve `readonly` or `writable` semantics at runtime for v0.1. The Doria compiler enforces those rules before output.

---

# Phase 7: MVP example to compile

This file should compile and run:

```php
class Person
{
    protected Dictionary<string, int> $items = [
        'apples' => 5,
        'oranges' => 10,
    ];

    public function __construct(
        public writable string $name,
        public int $age = 10,
    ) {
    }

    public function greet(): void
    {
        echo $this->getGreetingMessage();
    }

    public function displayInventory(): void
    {
        foreach ($this->items as string $name => int $quantity) {
            echo sprintf("%-20s %d\n", "{$name}:", $quantity);
        }
    }

    public writable function rename(string $name): void
    {
        $this->name = $name;
    }

    private function getGreetingMessage(): string
    {
        return "Hello, my name is {$this->name} and I am {$this->age} years old!";
    }
}

let writable $person = new Person("Andrew Masiye", 37);

$person->greet();
echo "\n---\n";
$person->displayInventory();

$person->rename("Lucy");
echo "\n---\n";
$person->greet();
```

Expected output:

```text
Hello, my name is Andrew Masiye and I am 37 years old!
---
apples:              5
oranges:             10

---
Hello, my name is Lucy and I am 37 years old!
```

---

# Phase 8: Diagnostics quality

Doria should have friendly errors.

Bad code:

```php
let $person = new Person("Andrew", 37);
$person->name = "Lucy";
```

Possible diagnostic:

```text
error[E0201]: cannot write through readonly variable `$person`

  examples/person.doria:2:1
  |
2 | $person->name = "Lucy";
  | ^^^^^^^ `$person` was declared readonly here
  |
help: declare it as writable:
  |
1 | let writable $person = new Person("Andrew", 37);
  |     +++++++++
```

Bad code:

```php
class Person
{
    public string $name;
}

let writable $person = new Person("Andrew");
$person->name = "Lucy";
```

Diagnostic:

```text
error[E0202]: cannot assign to readonly property `Person::$name`

help: mark the property writable:
  public writable string $name;
```

---

# Phase 9: Borrow checker v0.1

Do **not** build a full Rust-like borrow checker immediately.

Start with a simpler rule:

```text
A writable value cannot be captured by an async/spawned task while it is still writable in the parent scope.
```

Since async is not in the MVP, this phase can wait.

For now, the readonly/writable checker gives most of the beginner-facing benefit.

Later, when async exists:

```php
let writable $person = new Person("Andrew", 37);

let $task = spawn sendWelcomeEmail($person);

$person->name = "Lucy"; // Error until task is awaited

await $task;
```

After await:

```php
$person->name = "Lucy"; // Okay
```

---

# Phase 10: Async later

Do not implement async in v0.1.

When ready, add syntax:

```php
async function fetchUser(int $id): User
{
    let $response = await Http::get("https://example.com/users/{$id}");
    return User::fromJson($response->body);
}
```

Task type:

```php
Task<User>
```

Spawn:

```php
let $task = spawn fetchUser(1);
let $user = await $task;
```

Structured concurrency later:

```php
async scope {
    let $userTask = spawn fetchUser(1);
    let $postsTask = spawn fetchPosts(1);

    let $user = await $userTask;
    let $posts = await $postsTask;
}
```

For a PHP backend, this could eventually lower to a Doria runtime built on Fibers. For a native backend, lower async to Doria IR, then to LLVM or another backend later. PHP’s Fiber API gives low-level suspension/resumption, but Doria should offer a cleaner language-level abstraction. ([PHP][3])

---

# Phase 11: Native backend later

After the PHP backend works, design a Doria IR.

Possible future pipeline:

```text
Doria source
→ Lexer
→ Parser
→ AST
→ Semantic analysis
→ Doria HIR
→ Doria MIR
→ LLVM IR or MLIR
→ Native executable
```

LLVM is the obvious long-term backend candidate because its tutorial path covers implementing a language frontend, generating LLVM IR, JIT support, object-code compilation, and debug info. MLIR is another possible future option if Doria needs multiple levels of IR and more advanced lowering, because MLIR is built around a textual/in-memory/serialized IR suitable for transformations and compiler development. ([LLVM][1]) ([mlir.llvm.org][7])

Do not begin here.

---

# First Codex prompt

Copy this into Codex:

```text
You are helping build a new programming language called Doria.

Doria is PHP-shaped but not PHP++. It uses PHP-like syntax, `$variables`, classes, functions, visibility modifiers, constructor property promotion, and C-like blocks. However, it is strongly typed, compiled, readonly by default, and uses `writable` for intentional mutation.

Build the first MVP compiler in Rust.

Goal for v0.1:
- Parse a small Doria subset.
- Build an AST.
- Perform semantic checks.
- Enforce declaration and readonly/writable rules.
- Emit PHP.
- Run tests.

Do not implement async, generics beyond parsing simple collection types, borrow checking across tasks, native code generation, or a full standard library yet.

Create this repository structure:

doria/
├── AGENTS.md
├── README.md
├── SPEC.md
├── Cargo.toml
├── crates/
│   └── doriac/
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs
│           ├── lib.rs
│           ├── lexer.rs
│           ├── parser.rs
│           ├── ast.rs
│           ├── types.rs
│           ├── symbols.rs
│           ├── semantics.rs
│           ├── diagnostics.rs
│           ├── codegen_php.rs
│           └── source.rs
├── examples/
│   ├── hello.doria
│   ├── variables.doria
│   ├── person.doria
│   └── errors/
│       ├── undeclared_variable.doria
│       ├── readonly_assignment.doria
│       ├── readonly_property.doria
│       └── non_writable_method.doria
└── tests/
    ├── lexer_tests.rs
    ├── parser_tests.rs
    ├── semantic_tests.rs
    └── codegen_php_tests.rs

Language rules:
1. Variables must be declared with `let` or an explicit type.
2. Bare assignment never declares a variable.
3. `let $x = value;` creates a readonly inferred variable.
4. `let writable $x = value;` creates a writable inferred variable.
5. `int $x = value;` creates a readonly explicitly typed variable.
6. `writable int $x = value;` creates a writable explicitly typed variable.
7. Properties are readonly by default.
8. Properties are writable only when declared with `writable`.
9. Function parameters are readonly by default.
10. Function parameters are writable only when declared with `writable`.
11. Methods receive readonly `$this` by default.
12. Methods must be declared `writable function` to mutate `$this`.
13. Collection type aliases are `List<T>`, `Dictionary<K, V>`, and `Set<T>`.
14. Do not use `Vec`.

Initial CLI:
- `doriac check <file>`
- `doriac compile <file> --target php --out <file>`
- `doriac run <file>`

Start by implementing:
1. SPEC.md
2. Lexer
3. Parser for variable declarations, functions, classes, properties, methods, constructor params, echo, return, foreach, assignments, function calls, method calls, property access, literals, arrays/dictionaries.
4. AST structs/enums.
5. Semantic checker for symbol declarations and readonly/writable rules.
6. PHP code generator.
7. Tests for success and failure cases.

Definition of done:
- `cargo test` passes.
- `doriac check examples/person.doria` succeeds.
- `doriac compile examples/person.doria --target php --out build/person.php` emits runnable PHP.
- Running the emitted PHP produces the expected output.
- Invalid examples produce clear compiler errors.
```

---

# Suggested first 10 issues for Codex

## Issue 1: Initialize Rust workspace

Create the workspace, CLI crate, module files, README, SPEC, and AGENTS.

Acceptance:

```bash
cargo test
cargo run -p doriac -- --help
```

Both should work.

---

## Issue 2: Implement source locations and diagnostics

Every token and AST node should carry a span.

Acceptance:

```text
Diagnostics must show filename, line, column, and a useful message.
```

---

## Issue 3: Implement lexer

Tokenize the MVP language.

Acceptance:

```text
Lexer tests cover keywords, variables, strings, numbers, operators, comments, and generics punctuation.
```

---

## Issue 4: Implement parser for variables and expressions

Support:

```php
let $x = 5;
let writable $x = 5;
int $x = 5;
writable int $x = 5;
$x = $x + 1;
echo $x;
```

Acceptance:

```text
Parser tests produce the expected AST.
```

---

## Issue 5: Implement type references

Support:

```php
int
float
string
bool
void
Person
List<int>
Dictionary<string, int>
Set<string>
```

Acceptance:

```text
Type parser tests pass.
```

---

## Issue 6: Implement functions and blocks

Support:

```php
function main(): void
{
    echo "Hello";
}
```

Acceptance:

```text
Functions parse and semantic checker creates function symbols.
```

---

## Issue 7: Implement classes, properties, and methods

Support:

```php
class Person
{
    public writable string $name;

    public function greet(): void
    {
        echo $this->name;
    }
}
```

Acceptance:

```text
Class symbols, properties, and methods are registered.
```

---

## Issue 8: Implement readonly/writable semantic checks

Catch:

```php
let $x = 5;
$x = 6;
```

Catch:

```php
class Person
{
    public string $name;
}

let writable $person = new Person("Andrew");
$person->name = "Lucy";
```

Catch:

```php
class Person
{
    public writable string $name;

    public function rename(string $name): void
    {
        $this->name = $name;
    }
}
```

Acceptance:

```text
Semantic tests verify all expected errors.
```

---

## Issue 9: Implement PHP code generation

Emit valid PHP for the supported subset.

Acceptance:

```text
Generated PHP passes `php -l`.
```

---

## Issue 10: Compile and run the Person example

Acceptance:

```bash
doriac run examples/person.doria
```

Expected output:

```text
Hello, my name is Andrew Masiye and I am 37 years old!
---
apples:              5
oranges:             10

---
Hello, my name is Lucy and I am 37 years old!
```

---

# MVP non-goals

Do not implement these yet:

```text
- Full PHP compatibility
- Native code generation
- LLVM
- MLIR
- Full borrow checker
- Async/await
- Interfaces
- Traits
- Namespaces
- Composer integration
- Reflection
- Attributes
- Union types
- Pattern matching
- Macros
- Package manager
```

This keeps the first version focused and achievable.

---

# The main principle

The first version of Doria should prove this idea:

> **Doria feels familiar to PHP developers, but catches undeclared variables, accidental mutation, and unsafe object writes before runtime.**

That is enough for v0.1. Generics, async, borrow checking, and native compilation can grow from that foundation.

[1]: https://llvm.org/docs/tutorial/index.html "LLVM Tutorial: Table of Contents — LLVM 23.0.0git documentation"
[2]: https://www.php.net/manual/en/language.oop5.properties.php "PHP: Properties - Manual"
[3]: https://www.php.net/manual/en/language.fibers.php "PHP: Fibers - Manual"
[4]: https://help.openai.com/en/articles/11369540-getting-started-with-codex "Using Codex with your ChatGPT plan | OpenAI Help Center"
[5]: https://tree-sitter.github.io/tree-sitter/creating-parsers/2-the-grammar-dsl.html "The Grammar DSL - Tree-sitter"
[6]: https://www.antlr.org/about.html "About The ANTLR Parser Generator"
[7]: https://mlir.llvm.org/docs/LangRef/ "MLIR Language Reference - MLIR"
