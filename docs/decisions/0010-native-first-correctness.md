# 0010 Native-first correctness

Status: Accepted

## Decision

Doria is a native-first compiled language. Its primary product goal is standalone native programs.

The PHP backend is optional and non-authoritative. It may be useful for compatibility, migration, debugging, inspection, and early backend smoke tests, but it must not define Doria semantics.

Correctness and safety outrank quick runnable demos.

## Rules

Implementation work must follow these rules:

```text
1. Doria semantics come before backend output.
2. The native execution path is the primary long-term target.
3. PHP transpilation is a bonus path, not the language goal.
4. Generated PHP is not a semantic oracle.
5. Backend convenience must not decide syntax, typing, runtime behavior, memory behavior, object layout, or standard-library APIs.
6. If the specification does not answer a language-design question, stop and ask the language designer.
```

## Ask-first policy

If an implementation task reaches a fork that can affect Doria's long-term semantics or native backend path, the implementation must pause and report:

```text
- the exact design question
- the viable options
- the tradeoffs
- the files/components affected
- a recommendation, clearly marked as a recommendation
```

Do not silently select the behavior that is easiest for PHP, Rust, JavaScript, C, C++, Cranelift, LLVM, or any other implementation ecosystem.

## Examples of ask-first questions

Ask before deciding any of the following:

```text
- whether non-bool values can be used as conditions
- how strings are represented in the native runtime
- which values can be interpolated into strings
- whether object interpolation calls a conversion hook
- the native representation of List, Dictionary, and Set
- property initializer and constructor execution order
- whether constructors can raise recoverable errors
- whether destructors can raise recoverable errors
- what equality means for objects and collections
- whether an operation panics, raises, or returns a recoverable error value
- whether an API should be property-shaped or method-shaped
```

## Backend policy

Backends implement Doria. They do not define it.

A backend may:

```text
- lower Doria IR to its target representation
- reject unsupported Doria features with a clear diagnostic
- add backend-specific tests for output correctness
```

A backend may not:

```text
- change Doria type rules to match the target language
- introduce implicit conversions because the target language has them
- rely on target-language runtime behavior as Doria semantics
- force Doria syntax to match backend limitations
- make compatibility output the definition of correctness
```

## Consequences

This decision means some features may take longer to implement because the language must answer the real semantic question before a backend shortcut is accepted.

That is intentional.

The project should still work incrementally, but increments must be semantically sound. A tiny native backend smoke target is preferred over a large compatibility feature if the latter would delay or blur the native-first execution path.

## Relationship to PHP interop

PHP interop remains useful for adoption and inspection, but it is not central to correctness.

Doria may eventually have:

```text
- a PHP compatibility backend
- a PHP-to-Doria migration converter
- migration diagnostics and autofixes
```

None of those should constrain Doria's core semantics or native backend design.
