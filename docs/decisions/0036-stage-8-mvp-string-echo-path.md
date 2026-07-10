# 0036 Stage 8 MVP string and echo path

Status: Accepted

## Decision

Stage 8 completes the MVP string and echo path that began with Stage 8a. Stage 8a remains the historical literal-backed subset; this decision adds `.` string concatenation for supported string expressions while preserving the Stage 9 native iteration work as a separate slice.

Doria uses `.` as the accepted PHP-shaped string concatenation operator. This is a surface syntax choice only: Doria does not inherit PHP scalar coercion, loose typing, truthiness, interpolation behavior, or runtime string semantics.

Supported Stage 8 source shapes include:

```doria
function main(): void
{
    echo "Hello Doria!";
}
```

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

```doria
function main(): void
{
    let $name = "Doria";

    echo "Hello " . $name . "!";
}
```

```doria
function main(): void
{
    let $name = "Doria";
    let $message = "Hello " . $name . "!";

    echo $message;
}
```

The parser, AST, Doria IR, semantic checker, PHP backend, and native smoke backend all treat `.` as string concatenation. Both operands must be `string` values or recovery types. Doria does not implicitly convert `int`, `bool`, objects, resources, collections, or other values to `string` for this operator.

## Native smoke boundary

Native Stage 8 supports only compile-time-known string values. Supported native string expressions are:

- string literals
- supported readonly string locals
- supported `.` concatenation of compile-time-known string expressions
- grouped supported string expressions

Readonly inferred string locals and explicit `string` locals may be initialized from supported string expressions. The native smoke backend evaluates supported string expressions during validation and lowers `echo` by writing the resulting exact bytes to stdout. Native `echo` does not append a newline and must not use C string null termination, `strlen`, `puts`, or runtime concatenation to determine the output.

The native string representation remains private to the native smoke module. It is not public Doria IR, final MIR, final native storage layout, or a stable Doria ABI.

## PHP backend

The PHP backend supports the same MVP source shapes where applicable, including readonly string locals, direct string-literal echo, string-local echo, and `.` string concatenation. PHP output is still a compatibility/debugging backend and is not Doria's semantic oracle.

## Rejections and non-goals

Stage 8 does not implement:

- heap strings
- writable string locals in native
- string assignment in native
- runtime string concatenation
- native string interpolation
- implicit display/string conversion
- `echo` of `int`, `bool`, objects, resources, or collections
- native string return values
- native string parameters
- native string-parameter or string-returning function calls
- stdin, stderr, file I/O, or general runtime I/O
- runtime exceptions or error machinery
- public FFI string representation
- final MIR
- LLVM backend work
- Baton work

Unsupported native string shapes must produce clear semantic or unsupported-feature diagnostics instead of silently lowering through PHP behavior or treating valid Doria as invalid.

Stage 10 later adds a bounded native free-function call path for `int` parameters and `int`/`void` returns. Stage 8 remains the string/echo path and still does not define native string parameters or string returns.

## Assumption

The implementation may keep compile-time-known Stage 8 native string values as private `String` values inside native smoke validation and lower only the final exact bytes for supported `echo` expressions. This is an implementation-private shortcut, not a public ABI or runtime string model.

Stage 11h later moved this compile-time-known subset into MIR and direct MIR-to-Cranelift object lowering, then retired the native smoke representation. The original restriction still holds: these compiler-known bytes do not define Doria's future runtime string model or ABI.
