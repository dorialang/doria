# 0031 Stage 7b break and continue

Status: Accepted

## Decision

Doria supports unlabeled loop-control statements:

```doria
break;
continue;
```

Initial semantics:

- `break` exits the nearest enclosing loop immediately.
- `continue` skips the remaining body of the nearest enclosing loop and continues with the next iteration.

Numeric break/continue levels are not accepted:

```doria
break 2;
continue 2;
```

Labeled break/continue remain future design work.

## Implementation Slice

Stage 7b implements frontend support for `break;` and `continue;` in the lexer, parser, AST, HIR, semantic checker, Doria IR lowering, and PHP backend.

The semantic checker rejects loop control outside loops:

```doria
function main(): int
{
    break;

    return 0;
}
```

`break` and `continue` are designed around nearest-loop semantics. The checker tracks loop depth rather than a one-off boolean.

## Native Stage 7b

Native Stage 7b extends the private native smoke backend with `break` and `continue` inside supported bounded/proven `while` loops.

Supported Stage 7b loop bodies may contain:

- supported `int` local declarations
- supported assignments to visible writable `int` locals
- supported assignments to loop-body writable `int` locals
- supported fallthrough `if` statements
- `break`
- `continue`

`break` and `continue` may appear directly in the loop body or inside supported fallthrough `if` statements within the loop body.

Stage 7b preserves the current compile-time loop proof model. Native validation still proves:

- the loop condition is supported
- the loop body contains only supported Stage 7b while-body statements
- every assignment target is writable and visible in the correct scope
- branch-local and loop-body-local declarations do not leak
- every executed arithmetic operation remains within signed 64-bit Doria `int` range
- the loop terminates within the current native smoke verification cap, either by condition becoming false or by `break`

The native smoke verification cap remains:

```text
10_000 iterations
```

This cap is a backend support limit, not Doria language semantics.

Accepted native `break` and `continue` lower to real Cranelift control flow:

- `break` jumps to the nearest loop-after block with current loop-carried visible locals.
- `continue` jumps to the nearest loop-header block with current loop-carried visible locals.

The current native process-exit boundary remains `0..125`. That is only the portable process status range for native smoke `main()` return values; it is not the Doria `int` range.

## Non-goals

Stage 7b does not add:

- labeled break
- labeled continue
- numeric break levels
- numeric continue levels
- nested `while`
- `foreach` native lowering
- `return` inside `while` bodies
- `return` inside fallthrough branch bodies
- `goto`
- `given`
- `finally`
- value-returning `when`
- namespace/use/include/declare compiler support
- trait `uses` compiler support
- bitwise operators
- division or modulo native lowering
- unary minus syntax
- bool locals or native bool return values
- strings, stdout, objects, classes, collections, FFI, runtime services, LLVM backend, or Baton

Unsupported backend coverage must remain an unsupported native backend diagnostic, not a claim that otherwise valid Doria is invalid.
