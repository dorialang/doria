# 0034 Stage 9 MVP iteration syntax

Status: Accepted

## Decision

Doria supports traditional PHP/C-style `for` loops for explicit counter and index iteration:

```doria
for (let writable $i = 0; $i < 10; $i++) {
    echo $i;
}
```

```doria
for (let writable $i = 0; $i < 10; ++$i) {
    echo $i;
}
```

```doria
for (let writable $i = 10; $i > 0; $i--) {
    echo $i;
}
```

Doria also supports `foreach` over integer ranges:

```doria
foreach (0..10 as $i) {
    echo $i;
}
```

```doria
foreach (0..<10 as $i) {
    echo $i;
}
```

Range rules:

- `0..10` is inclusive and produces `0` through `10`.
- `0..<10` is exclusive-end and produces `0` through `9`.
- Range endpoints are integer expressions.
- Stage 9 native support is limited to compile-time-known integer ranges.

`foreach` remains preferred for collections and ranges. `for` remains the explicit counter/index loop.

## Bindings and mutation

The variable after `as` in a range `foreach` is a loop-local binding. It does not need a prior `let` declaration and does not leak outside the `foreach` body.

Range `foreach` bindings are readonly per iteration by default:

```doria
foreach (0..10 as $i) {
    let $copy = $i;
}
```

Increment and decrement are accepted as standalone mutation statements and as `for` increment operations:

```doria
$i++;
+$i;
$i--;
--$i;
```

`++` and `--` require a declared writable `int` target. Value-producing `++`/`--` expression semantics are future work.

## Native Stage 9

Native Stage 9 extends the current native smoke backend with bounded/proven support for:

- simple traditional `for` loops
- integer range `foreach`
- `break` and `continue` inside supported range/counter loop bodies

Native validation preserves the current correctness-first model:

- initializer executes once
- condition executes before each iteration
- normal body fallthrough executes the increment, then condition
- `continue` executes the increment, then condition
- `break` exits directly after the loop
- integer increment/decrement uses checked Doria `int` arithmetic
- loops must be proven to terminate within the current native smoke cap

The native smoke cap is a backend support limit, not Doria language semantics.

## Non-goals

Stage 9 does not add:

- labeled break/continue
- numeric break/continue levels
- value-producing `++`/`--` expressions
- `++`/`--` on properties or indexed expressions
- float ranges
- string ranges
- descending range syntax
- step syntax
- runtime range objects or iterators
- heap range allocation
- user-defined iterator protocol
- general native `foreach` over collections unless separately implemented
- broader native runtime services
- LLVM backend work

Unsupported backend coverage must remain an unsupported native backend diagnostic, not a claim that otherwise valid Doria is invalid.
