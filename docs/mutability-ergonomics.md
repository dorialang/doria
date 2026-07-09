# Mutability Ergonomics

> Documentation role: supporting design note.
> Source-of-truth hierarchy: `docs/doria-end-to-end-plan.md` owns future sequencing; accepted `docs/decisions/*.md` files own topic-level decisions. This note is subordinate to both.

Doria is readonly by default. This is a core safety feature and should not be abandoned simply because some users find `writable` repetitive.

However, Doria should also be pleasant to write. This document records the planned ergonomic tools for reducing repetition while keeping mutation explicit.

---

## 1. Core rule

The default rule remains:

```text
Everything is readonly unless explicitly marked writable.
```

Member access is separate from mutability. Doria class members are externally accessible by default, and `internal` marks implementation details. `writable` controls mutation; `internal` controls API surface.

Examples:

```doria
let $x = 5;
$x = 10; // error
```

```doria
let writable $x = 5;
$x = 10; // ok
```

```doria
class Person
{
    string $id;
    writable string $name;
}
```

---

## 2. The complaint

Users may say:

```text
I have to write `writable` too often.
```

This is especially likely for:

```text
- DTOs
- ORM entities
- form models
- config objects
- test fixtures
- game state objects
- ECS components
- editor/tooling data models
```

The solution is not to make Doria mutable by default. The solution is to provide explicit larger-scope mutability controls.

---

## 3. Planned `writable class`

Doria should support:

```doria
writable class Person
{
    string $name;
    int $age;
}
```

Meaning:

```text
Properties in this class are writable by default.
```

Equivalent to:

```doria
class Person
{
    writable string $name;
    writable int $age;
}
```

This helps mutable data-heavy classes without making the whole language mutable by default.

---

## 4. `writable class` affects properties only

`writable class` should not make every method a mutating method.

This should still be an error:

```doria
writable class Person
{
    string $name;

    function rename(string $name): void
    {
        $this->name = $name; // Error: method is not writable
    }
}
```

The method must still say:

```doria
writable function rename(string $name): void
{
    $this->name = $name;
}
```

Reason:

```text
Property writability answers: can this field be reassigned?
Method writability answers: can this method mutate `$this`?
```

Those should remain separate.

---

## 5. Readonly overrides inside writable classes

A writable class should allow readonly exceptions:

```doria
writable class User
{
    readonly int $id;
    string $name;
    string $email;
}
```

Meaning:

```text
$id is readonly.
$name and $email are writable by default because the class is writable.
```

---

## 6. Planned `readonly class`

Doria is already readonly by default, but `readonly class` is still useful as a stronger declaration of intent.

```doria
readonly class Money
{
    int $amount;
    string $currency;
}
```

Meaning:

```text
This class is an immutable value object.
Writable properties should be rejected inside it.
```

Example error:

```doria
readonly class Money
{
    writable int $amount; // error
}
```

---

## 7. Property groups as a later feature

Property groups may later reduce repetition inside a normal class:

```doria
class Person
{
    writable {
        string $name;
        int $age;
        string $email;
    }

    string $id;
}
```

This is useful, but it should come after simpler class-level mutability.

---

## 8. Shorter keyword alternatives

Possible alternatives were considered:

```text
mut       too Rust-flavored
var       possible local-variable sugar later, but not now
rw        too cryptic
write     awkward grammar
mutable   clear, but not shorter than writable in a meaningful way
```

Decision for now:

```text
Keep `writable` as the canonical keyword.
Do not add aliases yet.
```

The compiler may offer typo help:

```text
Error: unknown keyword `writeable`
Help: did you mean `writable`?
```

Do not accept both spellings as valid syntax.

---

## 9. Interaction with object bindings

Even if a class is writable, the variable binding still matters.

```doria
writable class Person
{
    string $name;
}

let $person = new Person();
$person->name = "Lucy"; // Error: binding is readonly
```

```doria
let writable $person = new Person();
$person->name = "Lucy"; // ok
```

This preserves the rule:

```text
To write through a path, the whole path must permit writing.
```

---

## 10. Settled direction

Settled:

```text
- Doria remains readonly by default.
- `writable` remains the canonical mutation keyword.
- Add `writable class` to make properties writable by default inside that class.
- Add `readonly class` to make immutable value-object intent explicit.
- `writable class` affects properties only, not method `$this` mutability.
- Do not add `var`, `mut`, `rw`, or other aliases yet.
```

Open:

```text
- Exact parser grammar for `writable class` and `readonly class`.
- Whether property groups are worth adding later.
- Whether `var` should ever become local-variable sugar.
```
