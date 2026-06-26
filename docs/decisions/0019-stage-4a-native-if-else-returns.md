# 0019 Stage 4a native if/else returns

Status: Accepted

Accepted by the Stage 4a implementation task. Keep this slice narrow; later native control-flow stages must still be accepted separately.

## Decision

Stage 4a adds native support for a terminal `if` / `else` statement inside the accepted native entrypoint:

```doria
function main(): int
{
    let $left = 20;
    let $right = 22;

    if ($left + $right == 42) {
        return 42;
    } else {
        return 0;
    }
}
```

The accepted body shape is:

```text
zero or more supported readonly integer local declarations
then exactly one terminal return or terminal if/else statement
```

The terminal `if` / `else` shape is:

```doria
if (<supported bool condition>) {
    return <supported integer expression>;
} else {
    return <supported integer expression>;
}
```

Each branch body must contain exactly one supported return statement.

## Supported conditions

Stage 4a conditions support:

```text
- bool literals: true, false
- integer comparisons over supported integer expressions
```

Supported comparison operators are:

```text
== != < <= > >=
```

Integer comparisons compare signed 64-bit Doria `int` values. Doria does not use PHP-style truthiness; conditions must be `bool`.

## Range and overflow

Doria `int` remains signed 64-bit. The current native process-exit boundary remains:

```text
0..125
```

That range applies only to values returned from `main()` as the current portable native smoke-test process exit code. It is not the range of Doria `int`.

Stage 4a validates both branch return expressions against the current process-exit boundary even when the condition is a literal `true` or `false`. This is a narrow backend support rule, not a Doria language rule. Stage 4a does not add constant branch elimination or path-sensitive reachability.

Compile-time overflow in supported integer arithmetic remains a diagnostic before native lowering.

## Backend implementation

The native backend keeps Stage 3a expression lowering and extends the implementation-private native model with a terminal `if` / `else` form. Supported integer expressions lower as Cranelift `i64` values. Supported conditions lower to Cranelift branch conditions. Each branch converts its validated Doria `int` return expression to the platform-compatible `main` return type only after process-exit range validation.

This is not a public Doria IR, full native-oriented IR, or full native control-flow implementation.

## Non-goals

This decision does not support:

- `else if`
- nested `if`
- branch-local declarations
- `if` without `else`
- statements after the terminal `if` / `else`
- `while` or `foreach`
- division or modulo
- unary minus syntax
- bool locals
- comparisons outside `if` conditions
- logical operators
- writable locals or assignments
- function calls, methods, static calls, strings, interpolation, classes, objects, or collections
- top-level native script mode
- FFI, recoverable errors, panic/runtime machinery, LLVM backend support, Baton, or explicit fixed-width numeric type implementation
