# Doria ChatGPT Project Context

This document captures shared project context for ChatGPT/Codex conversations about Doria. Use it as the first reference point when starting a new assistant conversation, reviewing a branch, creating Codex prompts, or deciding whether a proposed change fits the project direction.

Last updated: 2026-06-13

---

## 1. Project identity

**Doria** is a new PHP-shaped, C-like, compiled programming language.

The name comes from the creator's two sisters' middle names:

```text
Dorothy + Lucy / Lucia -> Doria
```

Doria should feel familiar to PHP developers, but it is **not PHP++** and should not be constrained by PHP's parser, runtime, or historical design choices.

The compiler is called **`doriac`**.

`doriac` is currently a **Rust bootstrap compiler**. Rust is the initial implementation language, not the permanent identity of the compiler. A strategic early goal is for Doria to become capable enough that significant parts of `doriac` can eventually be written in Doria itself.

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

## 2. Product goal

Doria is for places where PHP developers may want a PHP-like development experience, but PHP itself does not completely make sense.

Important target areas include:

```text
- native command-line tools
- native desktop applications
- long-running services
- game tooling
- game engines
- graphics/media applications
- native bindings to C libraries, eventually including raylib
- self-hosted compiler/tooling work
```

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
- FFI/native library integration
```

The project should grow in small, tested compiler slices. Avoid rushing into a full language runtime, async runtime, borrow checker, game engine, raylib bindings, or native backend before the frontend and semantic model are solid.

A useful slogan:

> Doria keeps PHP's readability, but adds compiler-enforced safety and native compilation.

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
10. Native code generation is the primary long-term direction.
11. PHP output is only one backend.
12. Native desktop/game/FFI use cases should influence runtime and ABI design.
13. Self-hosting should influence compiler architecture.
```

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

Doria uses this rule:

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

---

### 4.4 Class-level mutability ergonomics

Users may find repeated `writable` tedious for mutable data models. The preferred solution is to preserve readonly-by-default as the language default, but add explicit class-level opt-ins.

Planned direction:

```php
writable class Person
{
    public string $name;
    public int $age;
}
```

Meaning:

```text
Properties in this class are writable by default.
```

`writable class` should affect properties only. It should not make every method a writable method. Mutating methods should still say:

```php
public writable function rename(string $name): void
{
    $this->name = $name;
}
```

Allow readonly overrides inside writable classes:

```php
writable class User
{
    public readonly int $id;
    public string $name;
    public string $email;
}
```

Also support explicit immutable value objects:

```php
readonly class Money
{
    public int $amount;
    public string $currency;
}
```

Do not add `var`, `mut`, `rw`, or other shorter aliases yet. Keep `writable` as the canonical keyword and add class-level/property-group ergonomics first.

---

### 4.5 Parameters are readonly by default

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

### 4.6 Methods receive readonly `$this` by default

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

### 4.7 References

Doria should support references/borrows, but they should be explicit, typed, and borrow-checker-aware rather than a direct copy of PHP's dynamic reference aliasing.

Possible direction:

```php
function increment(writable int &$count): void
{
    $count += 1;
}

let writable $count = 0;
increment(&$count);
```

Migration hard case wording should be:

```text
PHP-style dynamic reference aliasing with `&`, especially assignment-by-reference, foreach-by-reference, return-by-reference, and dynamic reference patterns.
```

Do not describe Doria as dropping references.

---

### 4.8 Constructor init access

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

---

### 4.9 Collection names

Use clear collection aliases:

```php
List<int>                 // ordered list of ints
Dictionary<string, int>   // map from string to int
Set<string>               // unique strings
```

Do **not** use `Vec`.

---

## 5. Executable property initializers and attributes

Doria should support object construction in instance property initializers:

```php
class Office
{
    public Person $manager = new Person();
}
```

Semantics:

```text
- Instance property initializers run once per object construction.
- Each object gets its own initialized value.
- A property initializer counts as initialization for readonly properties.
- PHP backend limitations must not define Doria semantics.
```

Doria should also support richer attribute/metadata expressions than PHP:

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

Attribute expression evaluation policy is not settled yet. Doria should avoid blindly executing arbitrary side-effecting code at compile time.

See:

```text
docs/executable-initializers-and-attributes.md
```

---

## 6. Compiler architecture

Target architecture:

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

Long-term backend model:

```text
                 -> PHP backend
Doria frontend -> HIR -> MIR -> native backend
                         -> debug/interpreter backend
                         -> WebAssembly backend
```

The PHP backend should never decide what the parser, AST, semantic model, HIR, or MIR can express.

---

## 7. Self-hosting direction

Self-hosting is an early strategic goal.

Current state:

```text
doriac is a Rust bootstrap compiler.
```

Long-term direction:

```text
Rust doriac
-> compiles Doria-written doriac
-> Doria-written doriac compiles itself
```

Important wording:

```text
Rust is the bootstrap implementation language.
Doria should eventually become capable enough to implement significant parts of doriac.
```

Avoid wording:

```text
Compiler implementation language: Rust.
Doria is implemented in Rust forever.
```

See:

```text
docs/self-hosting.md
```

---

## 8. Native desktop, games, and FFI direction

Doria should not be only a server/web language.

Important long-term targets:

```text
- native desktop apps
- native GUI tooling
- game tools
- game engines
- graphics programming
- media applications
- bindings to C libraries
- raylib bindings eventually
```

This means Doria will eventually need serious answers for:

```text
- FFI to C libraries
- stable ABI or binding conventions
- ownership rules across FFI boundaries
- native string/buffer representation
- low-level arrays/slices
- struct layout controls
- predictable performance
- explicit allocation strategies
- cross-platform builds
- package/build integration for native libraries
```

Do not implement these immediately, but keep them in mind when designing MIR, runtime, standard library, and native backend.

---

## 9. Performance and benchmarking direction

Doria's long-term performance goal is to be far closer to native compiled languages than to PHP/Python.

Honest expectation for a mature native Doria:

```text
- usually slower than mature C/C++/Rust on extreme low-level workloads
- potentially close to Rust/Go/C# NativeAOT territory for many application workloads
- much faster than PHP/Python for CPU-bound userland code
- competitive with Java/C#/JavaScript depending on workload, startup, runtime, and JIT effects
```

Benchmarking should be built into the project culture.

Track:

```text
- compile time
- cold startup time
- hot execution time
- wall/user/system time
- peak RSS memory
- binary size
- stripped binary size
- compressed artifact size
- container image size later
- correctness hash/output
```

Likely benchmark cases:

```text
hello_world
startup
fibonacci
primes
json_parse
json_encode
string_interpolation
list_dictionary_ops
object_construction
method_dispatch
lexer
parser
type_checker
small_game_loop
raylib_binding_smoke_test later
```

Do not make broad performance claims from one benchmark.

---

## 10. PHP interop and migration

Doria should have:

```text
1. Doria -> PHP backend.
2. PHP -> Doria migration tooling.
```

Doria should not promise:

```text
perfect conversion of all valid PHP into clean, idiomatic Doria.
```

Recommended framing:

```text
PHP-to-Doria migration assistant.
```

Architecture rule:

```text
Doria parser parses Doria.
PHP migration tool parses PHP.
```

Do not make the Doria parser accept all PHP.

Migration tooling should be confidence-based:

```text
Exact
Likely
Partial
Unsupported
```

See:

```text
docs/php-interop-and-migration.md
```

---

## 11. Current repo context

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

This is acceptable for early development. Keep boundaries clean enough for a future split.

---

## 12. Current implemented behavior

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

## 13. Known limitations and design gaps

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
- references/borrows
- interfaces
- traits
- namespaces
- async/await
- borrow checker across tasks
- real MIR
- native code generation
- FFI
- desktop app support
- game engine support
- raylib bindings
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
7. FFI/native runtime goals should be considered before locking memory representation.
```

---

## 14. Near-term roadmap

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
12. Add parser/AST support for attributes and named arguments.
13. Preserve property initializer expressions in AST/HIR.
14. Begin MIR design with simple functions and returns.
15. Add benchmarks structure before making performance claims.
16. Add a tiny native backend experiment only after MIR has shape.
```

---

## 15. CI expectations

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

---

## 16. Guidance for ChatGPT and Codex

When helping with Doria:

```text
- Treat Doria as the language, and `doriac` as the compiler.
- Do not call Doria a Rust language.
- Rust is only the bootstrap implementation language for the current doriac.
- Remember that self-hosting doriac in Doria is a strategic goal.
- Do not describe Doria as primarily compiling to PHP.
- Preserve native compilation as the long-term goal.
- Keep native desktop, game engine, and C-library binding use cases in mind.
- Treat PHP output as a compatibility/debugging backend.
- Treat PHP-to-Doria conversion as migration tooling, not core parsing.
- Keep compiler changes incremental and tested.
- Prefer explicit diagnostics over permissive parsing.
- Do not introduce dependencies without explaining the tradeoff.
- Update SPEC.md or design docs when language behavior changes.
- Update tests when compiler behavior changes.
```

When reviewing code:

```text
1. Check whether the change preserves backend independence.
2. Check whether PHP assumptions leaked into AST/HIR/semantics.
3. Check whether readonly/writable rules remain consistent.
4. Check whether errors have useful diagnostics.
5. Check whether new behavior has tests.
6. Check whether docs/spec need updating.
7. Check whether the change accidentally treats Rust as permanent rather than bootstrap.
8. Check whether native/FFI/desktop/game goals are being prematurely blocked.
```

---

## 17. Useful Codex task template

```text
You are working on Doria, a PHP-shaped compiled programming language.
The compiler is `doriac`; the current doriac is a Rust bootstrap compiler, and self-hosting doriac in Doria is a strategic goal.
Doria's long-term primary target is native machine code and standalone executables.
Doria should eventually be suitable for native desktop apps, game tooling/engines, and C-library bindings such as raylib.
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
- Do not let Rust-specific implementation details define Doria's language model.
- Current HIR may remain source-shaped.
- MIR is the future control-flow-oriented lowering target.

Definition of done:
- cargo fmt passes.
- cargo clippy passes.
- cargo test passes.
- Add or update tests for changed compiler behavior.
- Update SPEC.md, ROADMAP.md, or design docs if behavior or priorities change.

Non-goals:
- Do not implement unrelated language features.
- Do not start native backend, FFI, raylib, game engine, or desktop work unless this task explicitly asks for it.
- Do not add dependencies without justification.
```

---

## 18. Important design decisions already settled

Settled:

```text
- Language name: Doria.
- Compiler name: doriac.
- Current bootstrap implementation language: Rust.
- Self-hosting doriac in Doria is a strategic goal.
- Long-term goal: native machine code / standalone executables.
- Native desktop, game tooling/engines, and C-library bindings are important long-term use cases.
- PHP backend: side feature only.
- PHP-to-Doria migration: desirable, but separate from the Doria parser.
- Variable declaration: must use let or explicit type.
- Bare assignment never declares.
- Default mutability: readonly.
- Mutation keyword: writable.
- Class-level mutability direction: writable class / readonly class.
- Collection aliases: List, Dictionary, Set.
- Avoid Vec.
- Current IR naming: HIR today, MIR later.
- Branch strategy: main stable, develop integration.
```

Not settled / still open:

```text
- Exact ownership/borrow checker design.
- Exact reference/borrow syntax and lifetime rules.
- Exact async runtime model.
- Native backend choice: Cranelift, LLVM, or another path.
- Memory model: GC, reference counting, ownership-first, or hybrid.
- FFI and ABI design.
- Desktop app packaging strategy.
- Game engine architecture.
- Raylib binding design.
- String interpolation semantics.
- Nullable and union type syntax details.
- Package manager strategy.
- Module/namespacing strategy.
- Whether parameter shadowing is forbidden in all nested scopes or only same body scope.
```

---

## 19. Tone and product direction

The project should feel ambitious but practical.

Good phrasing:

```text
Doria is PHP-shaped, but not PHP++.
Doria starts with a Rust bootstrap compiler and should grow toward self-hosting.
Doria borrows safety ideas from Rust without forcing Rust vocabulary onto PHP developers.
Doria should make mutation visible without making normal code ugly.
Doria should compile to native executables eventually.
Doria should support places where PHP-like ergonomics are useful but PHP itself is the wrong runtime.
```

Avoid phrasing like:

```text
Doria is a Rust compiler.
Doria is implemented in Rust forever.
Doria is a PHP transpiler.
Doria is PHP with generics.
Doria should be understandable by a PHP interpreter.
```

Better phrasing:

```text
Doria code should be familiar to PHP developers, and existing PHP should be easy to migrate, but Doria itself has its own syntax and semantics.
```

---

## 20. Review checklist for future branches

Before merging a branch:

```text
- Does CI pass?
- Are tests added for new behavior?
- Does the change preserve readonly-by-default semantics?
- Does the change avoid PHP-specific assumptions in frontend/semantics/HIR?
- Does the change avoid treating Rust as the permanent compiler identity?
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
Bad: "Add native backend, async, generics, raylib bindings, and refactor parser"
```

---

## 21. Native backend direction

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
8. FFI foundations.
9. Desktop/game/raylib experiments.
10. Async/concurrency.
```

Do not jump directly from source-shaped HIR to a complex native backend.
