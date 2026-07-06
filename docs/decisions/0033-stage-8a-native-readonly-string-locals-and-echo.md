# 0033 Stage 8a native readonly string locals and echo

Status: Accepted

Note: `docs/decisions/0036-stage-8-mvp-string-echo-path.md` completes the Stage 8 MVP string and echo path by adding compile-time-known `.` concatenation. This decision remains the historical Stage 8a subset record.

## Decision

Native Stage 8a extends the current native smoke backend with readonly string locals initialized directly from string literals and with `echo` support for the supported string-expression subset.

The accepted local forms are:

```doria
function main(): void
{
    let $message = "Hello Doria!";
    echo $message;
}
```

```doria
function main(): void
{
    string $message = "Hello Doria!";
    echo $message;
}
```

Multiple readonly string locals may be declared and echoed in sequence:

```doria
function main(): void
{
    let $hello = "Hello";
    string $space = " ";
    let $name = "Doria!";
    echo $hello;
    echo $space;
    echo $name;
}
```

Supported native `echo` string expressions are:

- a string literal
- a supported readonly string local
- a grouped supported string expression

Native `echo` continues to write exact bytes to stdout with no implicit newline.

## Native representation boundary

Stage 8a does not introduce a public Doria string ABI or a general native string runtime. The current implementation may treat supported string locals as immutable literal-backed bindings and lower supported `echo` expressions directly to the existing stdout byte-write path using pointer and length information known at compile time.

This representation is private to the current native smoke module. It must not be documented as final Doria MIR, final native storage layout, or a stable ABI.

## Rejections

The native backend must continue to reject unsupported string shapes with clear unsupported-feature diagnostics instead of silently lowering them through another backend's semantics.

Rejected Stage 8a native shapes include:

- writable string locals
- string assignment
- string interpolation
- string concatenation
- string comparison
- string returns
- method calls or property access on strings
- display conversion or formatting
- heap strings or runtime-managed string values
- broader runtime I/O abstractions

For example, unbraced or braced interpolation remains future native string work:

```doria
function main(): void
{
    let $name = "Doria";
    echo "Hello, {$name}";
}
```

Writable string locals are also future work:

```doria
function main(): void
{
    let writable $message = "Hello";
    echo $message;
}
```

## Non-goals

This decision does not add:

- heap allocation for strings
- `strlen`-style runtime length discovery for supported literals
- writable string locals
- string assignment or mutation
- native string interpolation
- string concatenation
- string equality/comparison lowering
- string return values
- string parameters
- standard-library output APIs
- runtime exceptions or panic behavior
- FFI string representation
- LLVM backend work

Stage 8a is intentionally a small native-first usability slice for literal-backed stdout smoke programs. Broader native strings require a later accepted decision.
