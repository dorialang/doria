# Doria ChatGPT Project Context

This document captures shared project context for ChatGPT/Codex conversations about Doria. Use it as the first reference point when starting a new assistant conversation, reviewing a branch, creating Codex prompts, or deciding whether a proposed change fits the project direction.

Last updated: 2026-06-13

---

## 1. Project identity

**Doria** is a new PHP-shaped, C-like, compiled programming language.

The name comes from the creator's two sisters' middle names:

- Dorothy
- Lucy / Lucia

Doria should feel familiar to PHP developers, but it is **not PHP++** and should not be constrained by PHP's parser, runtime, or historical design choices.

The compiler is called **`doriac`** and is implemented in Rust.

The long-term target is:

```text
Doria source -> doriac -> native machine code -> standalone executables
```

A PHP backend is allowed, but only as a secondary feature:

```text
- migration aid
- compatibility bridge
- debugging/transpilation backend
- convenient early execution target
```

The PHP backend must not shape the core compiler architecture.

---

## 2. Project goal

Doria should become the language PHP developers might choose if PHP were designed today for:

```text
- strong static typing
- generics
- readonly-by-default code
- explicit mutation
- safe borrowing / lifetime analysis
- async and concurrency
- native compilation
- standalone deployment
```

The first implementation should grow in small, tested compiler slices. Avoid rushing into a full language runtime, async runtime, borrow checker, or native backend before the frontend and semantic model are solid.

---

## 3. Language design principles

Doria should follow these principles:

```text
1. PHP-shaped, not PHP-compatible at the parser level.
2. Valid PHP should be easy to migrate to Doria, but Doria-specific syntax does not need to run in PHP.
3. Declarations must be explicit.
4. Bare assignment never declares a variable.
5. Everything is readonly by default.
6. Mutation must be intentional via `writable`.
7. Types are real compiler-checked types, not comments.
8. Generics are first-class.
9. Async/concurrency should be built into the language eventually.
10. The compiler architecture must be backend-independent.
11. Native code generation is the primary long-term direction.
12. PHP output is only one backend.
```

A useful slogan:

> Doria keeps PHP's readability, but adds compiler-enforced safety and native compilation.

---

## 4. Core syntax decisions

### 4.1 Variables must be declared

Doria does not allow PHP-style implicit variable creation.

Valid declarations:

```php
let $name = "Andrew";
let writable $count = 0;

string $city = "Lusaka";
writable int $score = 0;
```

Invalid:

```php
$name = "Andrew"; // error: undeclared variable
```

Bare assignment is only assignment to an existing writable variable.

---

### 4.2 Readonly by default

Doria uses **Option B** from earlier design discussion:

```text
Everything is readonly unless explicitly marked writable.
```

Readonly inferred variable:

```php
let $x = 5;
$x = 10; // error
```

Writable inferred variable:

```php
let writable $x = 5;
$x = 10; // ok
```

Readonly explicit type:

```php
int $x = 5;
$x = 10; // error
```

Writable explicit type:

```php
writable int $x = 5;
$x = 10; // ok
```

Preferred local declaration forms:

```php
let $x = 5;              // inferred readonly
let writable $x = 5;     // inferred writable
int $x = 5;              // explicit readonly
writable int $x = 5;     // explicit writable
```

Do not use Rust-style `mut` in Doria. The chosen keyword is `writable`, because PHP developers already understand `readonly`, and `writable` is the plain-English counterpart.

---

### 4.3 Properties are readonly by default

```php
class Person
{
    public string $id;
    public writable string $name;
}
```

Assignment requires two permissions:

```text
1. The object path must be writable.
2. The property itself must be writable, unless being initialized through constructor init access.
```

Example:

```php
let writable $person = new Person("p1", "Andrew");

$person->name = "Lucy"; // ok if name is writable
$person->id = "p2";     // error if id is readonly
```

---

### 4.4 Parameters are readonly by default

Readonly parameter:

```php
function greet(Person $person): void
{
    echo $person->name;
    $person->name = "Lucy"; // error
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

### 4.5 Methods receive readonly `$this` by default

A normal method cannot mutate `$this`:

```php
class Person
{
    public writable string $name;

    public function rename(string $name): void
    {
        $this->name = $name; // error
    }
}
```

A method that mutates the object must be marked `writable`:

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

Calling a writable method requires a writable receiver:

```php
let $person = new Person("Andrew");
$person->rename("Lucy"); // error
```

```php
let writable $person = new Person("Andrew");
$person->rename("Lucy"); // ok
```

---

### 4.6 Constructor init access

This is an important pending language rule.

Constructors should get **init access**, not full writable `$this` by default.

Intended rule:

```text
A constructor may initialize each uninitialized readonly property exactly once.
```

This should be allowed:

```php
class Person
{
    public string $id;
    public writable string $name;

    public function __construct(string $id, string $name)
    {
        $this->id = $id;       // ok: init readonly property
        $this->name = $name;   // ok: writable property
    }
}
```

This should fail:

```php
public function __construct(string $id)
{
    $this->id = $id;
    $this->id = "other"; // error: readonly property already initialized
}
```

Outside construction, assigning readonly properties remains invalid:

```php
$person->id = "other"; // error
```

This rule is documented but not fully implemented yet.

---

### 4.7 Collection names

Use clear collection aliases:

```php
List<int>                 // ordered list of ints
Dictionary<string, int>   // map from string to int
Set<string>               // unique strings
```

Do **not** use `Vec`.

For early PHP output, these may lower to PHP arrays, but the Doria type checker should keep them distinct.

---

## 5. Example Doria code

```php
class Person
{
    protected Dictionary<string, int> $items = [
        "apples" => 5,
        "oranges" => 10,
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

## 6. Compiler architecture

The target architecture is:

```text
Doria source
-> lexer
-> parser
-> AST
-> semantic analysis
-> type checker
-> readonly/writable checker
-> borrow/lifetime analysis later
-> HIR today
-> MIR later
-> backend
```

Important distinction:

```text
AST = syntax-shaped representation.
HIR = checked, backend-neutral, but still relatively source-shaped.
MIR = future control-flow-oriented representation for borrow/lifetime analysis and native lowering.
```

Current HIR is intentionally close to AST. Do not pretend it is the final backend IR.

Long-term backend model:

```text
                 -> PHP backend
Doria frontend -> HIR -> MIR -> native backend
                         -> debug/interpreter backend
                         -> WebAssembly backend
```

The PHP backend should never decide what the parser, AST, semantic model, HIR, or MIR can express.

---

## 7. Repository context

Repository:

```text
https://github.com/amasiye/doria
```

Important branches:

```text
main     = stable protected branch
develop  = integration branch for active work
```

Preferred flow:

```text
feature/codex-some-task -> PR into develop
develop -> PR into main
```

Recommended branch protection:

```text
main:
  - require pull request before merging
  - require status checks
  - require branch up to date
  - require linear history
  - block force pushes
  - restrict deletions


develop:
  - require pull request before merging
  - require status checks
  - block force pushes
  - restrict deletions
```

`develop` can be slightly less strict than `main` while the project is moving quickly.

---

## 8. Current repo structure

Current structure is still mostly one compiler crate:

```text
crates/
  doriac/
    src/
      ast.rs
      backend.rs
      codegen_php.rs
      diagnostics.rs
      hir.rs
      lexer.rs
      lowering.rs
      main.rs
      mir.rs
      parser.rs
      semantics.rs
      source.rs
      symbols.rs
      types.rs
    tests/
```

This is acceptable for early development.

A future split may look like:

```text
crates/
  doriac/                 # CLI driver
  doria_frontend/         # lexer, parser, AST, source, diagnostics
  doria_semantics/        # symbols, type checker, mutability checker
  doria_hir/              # high-level checked representation
  doria_mir/              # control-flow-oriented representation
  doria_backend_php/      # PHP backend
  doria_backend_native/   # native backend experiments
```

Do not force this split too early, but keep code boundaries clean enough that the split remains easy.

---

## 9. Current implemented behavior

The current compiler vertical slice should support:

```text
- lexing a useful Doria token set
- parsing a small subset of declarations, classes, functions, statements, and expressions
- building AST
- checking undeclared assignment
- checking readonly/writable local mutation
- checking readonly/writable property mutation
- checking writable methods and readonly `$this`
- detecting duplicate local declarations
- detecting duplicate class declarations
- detecting duplicate property declarations
- detecting duplicate method declarations
- detecting unknown class construction
- detecting unknown property read/write
- detecting unknown method calls
- lowering checked AST to HIR
- emitting PHP for supported syntax
- CLI commands: check, ast, hir, compile, run
```

The compiler is intentionally incomplete.

---

## 10. Known limitations and design gaps

Do not assume these are implemented yet:

```text
- real semantic TypeId / TypeKind system
- assignment type compatibility
- return type checking
- function call argument checking
- constructor argument checking
- constructor init access for readonly properties
- string interpolation as Doria-owned AST nodes
- quote-kind preservation for strings
- precedence-aware PHP expression emission
- nullable types
- union types
- interfaces
- traits
- namespaces
- async/await
- borrow checker across tasks
- real MIR
- native code generation
- package manager
```

Important known technical concerns:

```text
1. Current HIR is still source-shaped.
2. TypeRef is still stringly and should eventually resolve into TypeId/TypeKind.
3. PHP codegen needs precedence-aware expression emission.
4. String interpolation currently risks being PHP-backend-dependent.
5. Constructor init access needs a proper semantic model.
6. Parameter shadowing policy needs to be decided and tested.
```

---

## 11. Near-term roadmap

Prioritize these tasks before adding large language features:

```text
1. Keep CI green on main and develop.
2. Fix semantic checking for property-assignment base expressions.
3. Decide and enforce parameter shadowing policy.
4. Add spans to foreach bindings.
5. Implement TypeId / TypeKind separate from parsed TypeRef.
6. Add assignment compatibility checking.
7. Add return type checking.
8. Add constructor argument count/type checking.
9. Design and implement constructor init access.
10. Add string interpolation AST independent of PHP behavior.
11. Add precedence-aware backend expression emission.
12. Begin MIR design with simple functions and returns.
13. Add a tiny native backend experiment only after MIR has shape.
```

---

## 12. CI expectations

Expected CI checks:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --workspace --all-targets --locked --verbose
cargo test --workspace --all-targets --locked --verbose
cargo run -p doriac -- check examples/person.doria
cargo run -p doriac -- hir examples/person.doria
cargo run -p doriac -- compile examples/person.doria --target php --out build/person.php
```

The workflow should run on:

```text
push to main
push to develop
pull requests targeting main
pull requests targeting develop
manual workflow_dispatch
```

Use read-only GitHub Actions permissions unless a workflow genuinely needs more.

---

## 13. Repository governance files

Recommended/expected files:

```text
LICENSE
README.md
SPEC.md
ROADMAP.md
CONTRIBUTING.md
SECURITY.md
AGENTS.md
.github/CODEOWNERS
.github/dependabot.yml
.github/workflows/ci.yml
rust-toolchain.toml
```

The project currently uses the MIT license.

---

## 14. Guidance for ChatGPT and Codex

When helping with Doria:

```text
- Treat Doria as the language, and `doriac` as the compiler.
- Do not call Doria a Rust language.
- Rust is only the implementation language of the compiler.
- Do not describe Doria as primarily compiling to PHP.
- Preserve native compilation as the long-term goal.
- Treat PHP output as a compatibility/debugging backend.
- Keep compiler changes incremental and tested.
- Prefer explicit diagnostics over permissive parsing.
- Do not introduce dependencies without explaining the tradeoff.
- Update SPEC.md when language behavior changes.
- Update tests when compiler behavior changes.
- Keep examples small and meaningful.
```

When reviewing code:

```text
1. Check whether the change preserves backend independence.
2. Check whether PHP assumptions leaked into AST/HIR/semantics.
3. Check whether readonly/writable rules remain consistent.
4. Check whether errors have useful diagnostics.
5. Check whether new behavior has tests.
6. Check whether docs/spec need updating.
```

When creating Codex prompts:

```text
- Give one focused task at a time.
- State the branch target.
- State non-goals explicitly.
- Ask for tests.
- Ask Codex not to add unrelated features.
- Ask Codex to keep architecture terms precise: AST, HIR, MIR, backend.
```

---

## 15. Useful Codex task template

```text
You are working on Doria, a PHP-shaped compiled programming language.
The compiler is `doriac`, implemented in Rust.
Doria's long-term primary target is native machine code and standalone executables.
The PHP backend is only a compatibility/debugging backend and must not shape core architecture.

Branch target: develop

Task:
<describe one focused task>

Language rules to preserve:
- Variables must be declared with `let` or an explicit type.
- Bare assignment never declares a variable.
- Everything is readonly by default.
- Use `writable` for intentional mutation.
- Properties, parameters, and `$this` are readonly by default.
- Methods that mutate `$this` must be declared `writable function`.
- Collection aliases are `List<T>`, `Dictionary<K, V>`, and `Set<T>`.
- Do not use `Vec`.

Architecture rules:
- Keep parser/AST/HIR/semantics backend-independent.
- Do not let PHP backend needs leak into the core model.
- Current HIR may remain source-shaped.
- MIR is the future control-flow-oriented lowering target.

Definition of done:
- cargo fmt passes.
- cargo clippy passes.
- cargo test passes.
- Add or update tests for changed compiler behavior.
- Update SPEC.md or ROADMAP.md if behavior or priorities change.

Non-goals:
- Do not implement unrelated language features.
- Do not start native backend work unless this task explicitly asks for it.
- Do not add dependencies without justification.
```

---

## 16. Important design decisions already settled

Settled:

```text
- Language name: Doria.
- Compiler name: doriac.
- Compiler implementation language: Rust.
- Long-term goal: native machine code / standalone executables.
- PHP backend: side feature only.
- Variable declaration: must use let or explicit type.
- Bare assignment never declares.
- Default mutability: readonly.
- Mutation keyword: writable.
- Collection aliases: List, Dictionary, Set.
- Avoid Vec.
- Current IR naming: HIR today, MIR later.
- Branch strategy: main stable, develop integration.
```

Not settled / still open:

```text
- Exact ownership/borrow checker design.
- Exact async runtime model.
- Native backend choice: Cranelift, LLVM, or another path.
- Memory model: GC, reference counting, ownership-first, or hybrid.
- String interpolation semantics.
- Nullable and union type syntax details.
- Package manager strategy.
- Module/namespacing strategy.
- Whether parameter shadowing is forbidden in all nested scopes or only same body scope.
```

---

## 17. Tone and product direction

The project should feel ambitious but practical.

Good phrasing:

```text
Doria is PHP-shaped, but not PHP++.
Doria borrows safety ideas from Rust without forcing Rust vocabulary onto PHP developers.
Doria should make mutation visible without making normal code ugly.
Doria should compile to native executables eventually.
```

Avoid phrasing like:

```text
Doria is a Rust compiler.
Doria is a PHP transpiler.
Doria is PHP with generics.
Doria should be understandable by a PHP interpreter.
```

Better phrasing:

```text
Doria code should be familiar to PHP developers, and existing PHP should be easy to migrate, but Doria itself has its own syntax and semantics.
```

---

## 18. Review checklist for future branches

Before merging a branch:

```text
- Does CI pass?
- Are tests added for new behavior?
- Does the change preserve readonly-by-default semantics?
- Does the change avoid PHP-specific assumptions in frontend/semantics/HIR?
- Are diagnostics clear and stable?
- Does SPEC.md need updating?
- Does ROADMAP.md need updating?
- Does README.md need updating?
- Is the PR focused?
- Are unrelated refactors avoided?
```

For compiler changes, prefer smaller PRs:

```text
Good: "Add unknown method diagnostics"
Good: "Add return type checking"
Bad: "Add native backend, async, generics, and refactor parser"
```

---

## 19. Current mental model for borrow checking

The borrow checker is a future feature. The current readonly/writable model is the foundation.

Conceptual model:

```text
readonly = safe to read/share
writable = allowed to change, but must not be shared unsafely
```

Future async example:

```php
let writable $person = new Person("Andrew", 37);

let $task = spawn sendWelcomeEmail($person);

$person->name = "Lucy"; // future error until task is awaited

await $task;
```

The compiler should eventually prevent writing to values that are borrowed or in use elsewhere, especially across async/concurrent tasks.

Do not implement this yet unless explicitly requested.

---

## 20. Native backend direction

Native code generation is the primary long-term target, but it should wait until MIR and semantic types are better defined.

Possible future backend paths:

```text
- Cranelift: Rust-native, practical early native backend candidate.
- LLVM: mature, powerful, broader optimization and target support.
- MLIR: possible future research-grade multi-stage lowering, not an early need.
```

Recommended order:

```text
1. Frontend correctness.
2. Real semantic type system.
3. HIR cleanup.
4. MIR design.
5. Tiny native function experiment.
6. Native printing/runtime model.
7. Objects, strings, collections.
8. Async/concurrency.
```

Do not jump directly from source-shaped HIR to a complex native backend.
