# PHP Interop and Migration Strategy

Doria is its own compiled programming language. PHP interop is an adoption strategy, not Doria's identity and not Doria's correctness target.

Doria's primary goal is native programs: native command-line tools, services, desktop applications, game tooling, systems software, and eventually self-hosted compiler components. PHP interoperability can be valuable, but it is optional and non-authoritative.

This document distinguishes three related but different ideas:

```text
1. Doria -> PHP compatibility/debugging backend
2. PHP -> Doria migration converter
3. Full bidirectional PHP/Doria compatibility
```

Doria may support the first two over time. It should avoid promising the third.

---

## 1. Strategic position

The long-term compiler direction remains:

```text
Doria source -> lexer -> parser -> AST -> semantic/type checking -> Doria IR -> backend
```

The native backend is the primary product target. Native executables are the reason the compiler exists.

The PHP backend is a compatibility, debugging, migration, and inspection backend. It is not Doria's reference implementation, and generated PHP is not the definition of Doria semantics. As native code generation matures, Doria IR may lower into a simpler native-oriented IR for control flow, memory layout, runtime calls, and backend code generation.

A PHP-to-Doria converter may also be useful, but it should be treated as a migration tool, not as the core compiler pipeline.

Recommended framing:

```text
Doria can help PHP developers migrate codebases incrementally.
Doria should not be limited to PHP semantics.
Doria's primary target is native standalone programs.
```

Avoid framing:

```text
Doria is PHP++.
Doria is a two-way PHP translator.
Doria guarantees that all PHP can be converted to idiomatic Doria.
Doria and PHP are equivalent languages.
Generated PHP defines what Doria means.
```

---

## 2. Correctness boundary

The PHP backend adapts to Doria. Doria must not adapt itself to PHP limitations.

Implementation work must not choose Doria behavior because it is easy to emit as PHP. If a feature exposes an unresolved language-design question, the implementation should stop and ask for a decision before proceeding.

Examples of questions that must not be answered silently:

```text
- How are Doria strings represented in a native runtime?
- What types may be interpolated into strings?
- Does object interpolation call a conversion hook?
- What is the native representation of List, Dictionary, and Set?
- What is the order of property initializers, constructor promotion, and constructor body execution?
- What errors are recoverable, and how are they represented?
```

Truthiness is no longer an open question: Doria conditions must be `bool`, and Doria does not use PHP-style truthiness.

Temporary PHP-backend limitations may produce diagnostics. They must not change the language.

---

## 3. Java/Kotlin analogy

The useful analogy is Kotlin and Java, but only partially.

Kotlin succeeded partly because Java developers could adopt it gradually. Java and Kotlin can coexist in the same ecosystem, call each other, and share tooling. Doria can learn from that adoption model.

The analogy should be:

```text
Kotlin did not need to make Java and Kotlin identical.
Doria does not need to make PHP and Doria identical.
```

Doria should aim for a migration bridge, not semantic equivalence.

---

## 4. Doria -> PHP compatibility backend

Doria source can lower into PHP for supported features. This output is useful for compatibility and inspection, but it is non-normative:

```text
Doria source -> parser -> AST -> semantic checks -> Doria IR -> PHP backend
```

Use cases:

```text
- debugging output
- migration aid
- ability to run some Doria code in PHP environments
- inspection target while native backend matures
- regression comparison for supported backend behavior
```

Important rule:

```text
The PHP backend adapts to Doria. Doria must not adapt itself to PHP limitations.
```

For example, Doria may support:

```doria
class Office
{
    Person $manager = new Person();
}
```

Even if the PHP backend has to lower this into constructor code or temporarily report an unsupported-feature diagnostic.

---

## 5. PHP -> Doria converter

A PHP-to-Doria converter is viable and may be useful as a migration aid.

But it should be scoped honestly and should not be prioritized ahead of the native execution path.

Recommended command shape:

```bash
doriac migrate php path/to/source.php --out path/to/source.doria
```

or:

```bash
doria-migrate php path/to/project --out migrated-project
```

The converter should initially produce **valid conservative Doria**, not perfect idiomatic Doria.

Example PHP input:

```php
<?php

function greet(string $name): void
{
    echo "Hello, $name";
}
```

Possible Doria output:

```doria
function greet(string $name): void
{
    echo "Hello, {$name}";
}
```

PHP input:

```php
<?php

$count = 0;
$count += 1;
```

Possible Doria output:

```doria
let writable $count = 0;
$count += 1;
```

The converter can infer `writable` when it sees later assignments.

---

## 6. Migration modes

The converter should support modes.

### Conservative mode

Goal: preserve behavior as much as possible while producing valid Doria.

```bash
doriac migrate php src --mode conservative
```

Characteristics:

```text
- Prefer `mixed` when types are unclear.
- Mark variables writable if reassigned.
- Avoid aggressive rewrites.
- Preserve structure and naming.
- Add comments or diagnostics for uncertain translations.
```

Example:

```doria
mixed $value = getValue();
```

or:

```doria
let writable $value = getValue(); // type unknown
```

### Strict mode

Goal: produce more Doria-like code and require stronger typing.

```bash
doriac migrate php src --mode strict
```

Characteristics:

```text
- Require explicit types or inferred safe types.
- Flag dynamic features.
- Prefer readonly declarations when safe.
- Emit TODO diagnostics where human decisions are needed.
```

### Idiomatic mode later

Goal: improve style after correctness is proven.

```bash
doriac migrate php src --mode idiomatic
```

Possible rewrites:

```text
- PHP arrays -> List / Dictionary after safe shape analysis
- docblock generics -> Doria generics once Doria generics exist
- nullable patterns -> the chosen Doria nullable/error model once designed
- exception-heavy code -> the chosen Doria recoverable error model once designed
- setter-heavy code -> constructor/init/property-hook patterns where safe
```

This should come much later.

---

## 7. Why full PHP conversion is hard

Valid PHP includes many dynamic features that do not map cleanly to Doria's safety model.

Hard cases include:

```text
- dynamic variables: ${$name}
- variable functions and dynamic method calls
- dynamic properties
- magic methods: __get, __set, __call
- eval
- PHP include/require with computed paths
- runtime symbol table tricks
- weak typing and coercion
- by-reference behavior
- global state
- arrays used as both lists and maps
- reflection-heavy frameworks
- runtime code generation
```

A converter can support many common PHP programs, but it should not promise automatic perfect conversion for every valid PHP program.

Recommended policy:

```text
Convert what can be converted safely.
Emit clear migration diagnostics for the rest.
```

Example diagnostic style:

```text
warning[MIG020]: dynamic property access cannot be safely converted
  source.php:12:5
help: replace dynamic property access with an explicit property or Dictionary lookup
```

---

## 8. PHP compatibility profile

Doria can define migration profiles:

```text
PHP-simple:
  functions, classes, typed params, typed returns, simple expressions

PHP-modern:
  constructor promotion, enums later, attributes, readonly properties, match later

PHP-dynamic:
  magic methods, dynamic calls, variable variables, eval, reflection-heavy code
```

Doria should target `PHP-simple` and much of `PHP-modern` first.

`PHP-dynamic` should be warning-heavy and may require manual migration.

---

## 9. Implementation approach

Do not write a PHP parser from scratch first.

The migration converter can initially be a separate tool that uses a mature PHP parser to produce a PHP AST, then lowers that AST into Doria AST or Doria source.

Possible architecture:

```text
PHP source
-> PHP parser
-> PHP AST
-> migration analysis
-> Doria migration AST
-> Doria pretty-printer
-> doriac check
```

The migration tool should also output diagnostics:

```text
- unsupported dynamic feature
- inferred writable variable
- inferred mixed type
- array could be List<T>
- array could be Dictionary<K, V>
- human review needed
```

Over time, migration could become integrated into `doriac`, but it should stay architecturally separate from the Doria parser.

---

## 10. Relationship to Doria compiler frontend

The Doria compiler frontend should parse Doria, not PHP.

The PHP migration tool may produce Doria source, then pass that source into `doriac check`.

Recommended separation:

```text
doriac parser:
  parses Doria only

PHP migration parser:
  parses PHP for conversion purposes only

Doria backend_php:
  emits PHP from Doria IR
```

Do not mix these into one parser.

---

## 11. Should PHP -> Doria be in doriac?

It can be exposed through `doriac`, but internally it should be a separate pipeline.

Acceptable CLI:

```bash
doriac migrate php src --out migrated
```

Internal architecture:

```text
doriac CLI
  -> doria_migrate_php crate
       -> PHP parser adapter
       -> migration analyzer
       -> Doria source emitter
  -> doriac check generated Doria
```

A future workspace shape could be:

```text
crates/
  doriac/
  doria_frontend/
  doria_semantics/
  doria_ir/
  doria_backend_native/
  doria_backend_php/
  doria_migrate_php/
```

---

## 12. Why this is wise if scoped correctly

PHP-to-Doria conversion is wise because it supports adoption.

Benefits:

```text
- gives existing PHP developers a migration path
- turns Doria from a greenfield-only language into an incremental option
- helps discover real-world language requirements
- encourages tools, diagnostics, and autofixes early
- creates examples for documentation and tests
```

Risks:

```text
- it can distract from native compilation
- it can pressure Doria to preserve PHP's worst dynamic behavior
- automatic conversion may create ugly Doria
- users may expect impossible perfect migration
- the converter may become larger than the compiler frontend
```

Conclusion:

```text
Build a PHP-to-Doria migration tool only after the native-first compiler path remains protected.
Do not build a promise of perfect PHP compatibility.
```

---

## 13. Recommended timeline

Do not build PHP-to-Doria migration immediately.

Recommended order:

```text
1. Stabilize Doria parser and AST.
2. Add real semantic types.
3. Stabilize assignment compatibility, return checking, argument checking, control flow, and constructor init rules.
4. Stabilize Doria IR as a backend-independent checked representation.
5. Define the native execution path and first native smoke target.
6. Add a tiny native backend slice for `main(): int` returning an exit code.
7. Add a Doria pretty-printer.
8. Build a small PHP-to-Doria prototype for simple PHP files only after native direction is active.
9. Use the prototype to inform diagnostics and migration tooling, not core Doria semantics.
10. Add project-level migration only after single-file migration works well.
```

The converter should begin with simple, typed, modern PHP.

---

## 14. Settled direction

Settled:

```text
- Doria is native-first.
- Doria should support Doria -> PHP only as an optional compatibility/debugging backend.
- Doria may support PHP -> Doria as a migration converter.
- PHP -> Doria should not be part of the Doria parser.
- PHP -> Doria should not define Doria semantics.
- Generated PHP should not be treated as Doria's semantic reference output.
- The converter should be honest and diagnostic-heavy.
- The converter should produce conservative valid Doria first, idiomatic Doria later.
```

Open:

```text
- Exact CLI command name.
- Which PHP parser to use.
- How much PHPDoc/Psalm/PHPStan information to consume.
- How to infer List vs Dictionary from PHP arrays.
- How to represent unsupported dynamic PHP features.
- Whether migration output should preserve comments and formatting.
```
