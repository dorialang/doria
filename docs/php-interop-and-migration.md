# PHP Interop and Migration Strategy

Doria is PHP-shaped, but it is not PHP++ and it is not primarily a PHP transpiler.

Still, PHP interoperability is important. A major adoption advantage would be letting existing PHP teams move toward Doria gradually instead of rewriting whole applications at once.

This document distinguishes three related but different ideas:

```text
1. Doria -> PHP compatibility backend
2. PHP -> Doria migration converter
3. Full bidirectional PHP/Doria compatibility
```

Doria should support the first two over time. It should avoid promising the third.

---

## 1. Strategic position

The long-term compiler direction remains:

```text
Doria source -> doriac -> HIR -> MIR -> native backend -> standalone executable
```

The PHP backend is a compatibility, debugging, migration, and transpilation backend. It is not Doria's reference implementation, and generated PHP is not the definition of Doria semantics.

A PHP-to-Doria converter may also be useful, but it should be treated as a migration tool, not as the core compiler pipeline.

Recommended framing:

```text
Doria can help PHP developers migrate codebases incrementally.
Doria should not be limited to PHP semantics.
```

Avoid framing:

```text
Doria is a two-way PHP translator.
Doria guarantees that all PHP can be converted to idiomatic Doria.
Doria and PHP are equivalent languages.
```

---

## 2. Java/Kotlin analogy

The useful analogy is Kotlin and Java, but only partially.

Kotlin succeeded partly because Java developers could adopt it gradually. Java and Kotlin can coexist in the same ecosystem, call each other, and share tooling. Doria can learn from that adoption model.

The analogy should be:

```text
Kotlin did not need to make Java and Kotlin identical.
Doria does not need to make PHP and Doria identical.
```

Doria should aim for a migration bridge, not semantic equivalence.

---

## 3. Doria -> PHP compatibility backend

This is the easier direction.

Doria source can lower into PHP for supported features. This output is useful for compatibility and inspection, but it is non-normative:

```text
Doria source -> parser -> AST -> semantic checks -> HIR -> PHP backend
```

Use cases:

```text
- early runnable compatibility backend
- debugging output
- migration aid
- ability to run some Doria code in PHP environments
- inspection target while native backend matures
```

Important rule:

```text
The PHP backend adapts to Doria. Doria must not adapt itself to PHP limitations.
```

For example, Doria may support:

```doria
class Office
{
    public Person $manager = new Person();
}
```

Even if the PHP backend has to lower this into constructor code.

---

## 4. PHP -> Doria converter

A PHP-to-Doria converter is viable and probably wise as a migration aid.

But it should be scoped honestly.

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
    echo "Hello, $name";
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

## 5. Migration modes

The converter should support modes.

### Conservative mode

Goal: preserve behavior as much as possible.

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
- PHP arrays -> List / Dictionary
- docblock generics -> Doria generics
- nullable patterns -> ?T or Option<T>
- exception-heavy code -> Result<T, E> where appropriate
- setter-heavy code -> constructor/init patterns where safe
```

This should come much later.

---

## 6. Why full PHP conversion is hard

Valid PHP includes many dynamic features that do not map cleanly to Doria's safety model.

Hard cases include:

```text
- dynamic variables: ${$name}
- variable functions and dynamic method calls
- dynamic properties
- magic methods: __get, __set, __call
- eval
- include/require with computed paths
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

## 7. PHP compatibility profile

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

## 8. Implementation approach

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

## 9. Relationship to Doria compiler frontend

The Doria compiler frontend should parse Doria, not PHP.

The PHP migration tool may produce Doria source, then pass that source into `doriac check`.

Recommended separation:

```text
doriac parser:
  parses Doria only

PHP migration parser:
  parses PHP for conversion purposes only

Doria backend_php:
  emits PHP from Doria HIR/MIR
```

Do not mix these into one parser.

---

## 10. Should PHP -> Doria be in doriac?

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
  doria_hir/
  doria_mir/
  doria_backend_php/
  doria_backend_native/
  doria_migrate_php/
```

---

## 11. Why this is wise if scoped correctly

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
Build a PHP-to-Doria migration tool, not a promise of perfect PHP compatibility.
```

---

## 12. Recommended timeline

Do not build PHP-to-Doria migration immediately.

Recommended order:

```text
1. Stabilize Doria parser and AST.
2. Add real semantic types.
3. Add assignment and return type checking.
4. Add HIR stability and pretty/debug output.
5. Add a Doria pretty-printer.
6. Build a small PHP-to-Doria prototype for simple PHP files.
7. Use the prototype to inform Doria syntax and diagnostics.
8. Add project-level migration only after single-file migration works well.
```

The converter should begin with simple, typed, modern PHP.

---

## 13. Settled direction

Settled:

```text
- Doria should support Doria -> PHP as a compatibility backend.
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
