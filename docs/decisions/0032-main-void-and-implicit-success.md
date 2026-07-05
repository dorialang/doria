# 0032 Main void and implicit success

Status: Accepted

## Decision

Doria entrypoints may use either:

```doria
function main(): int
{
    return 0;
}
```

or:

```doria
function main(): void
{
}
```

`main(): int` is the explicit process-status form. The current native smoke backend accepts only returned status values in the portable `0..125` range. That range is a process-status boundary for this native slice, not the range of Doria `int`.

`main(): void` is the implicit-success form:

- falling through the end of `main(): void` exits successfully with process status `0`
- `return;` in `main(): void` exits successfully with process status `0`
- `return <expr>;` in `main(): void` is a semantic error

Non-entrypoint `void` functions and methods may also use `return;` or fall through, but they do not define process status. Only the program entrypoint maps normal `void` completion to process success.

## Native Stage 7b

Native Stage 7b accepts exactly one top-level entrypoint named `main` with no parameters and one of these signatures:

```doria
function main(): int
function main(): void
```

Stage 7b also adds a narrow native stdout smoke path for string-literal `echo`:

```doria
function main(): void
{
    echo "Hello Doria!";
}
```

This writes the literal bytes exactly as spelled by the string literal contents. It does not append a newline, and it must not be lowered through helper behavior such as `puts`.

The current native stdout path is intentionally narrow. It supports string literals only. It does not add general native strings, string locals, interpolation, formatting, `Console`, runtime I/O, or a Doria standard-library output API.

Unsupported examples remain unsupported native backend coverage, not invalid Doria:

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
    echo "Hello, {$name}";
}
```

## Runtime Errors

This decision does not define runtime error behavior. Nonzero process statuses for runtime failures, panic/error taxonomy, unwinding, `finally` interaction, and diagnostic/runtime reporting are future runtime and error-model work.

## Non-goals

This decision does not add:

- parser changes beyond already accepted `void`, `return;`, and `echo` syntax
- general native string values
- string variables in the native backend
- string interpolation in the native backend
- nonliteral native `echo`
- stdout formatting
- implicit newline behavior
- `Console`
- DDO
- namespaces, imports, or source inclusion
- broader native runtime services
- LLVM backend work
- Baton work

Unsupported backend coverage must continue to produce clear unsupported-feature diagnostics rather than pretending valid Doria is invalid.

Stage 8a later extends this Stage 7b boundary with readonly string locals initialized from string literals and supported string `echo` expressions. See `docs/decisions/0033-stage-8a-native-readonly-string-locals-and-echo.md` for the current native string-local slice.
