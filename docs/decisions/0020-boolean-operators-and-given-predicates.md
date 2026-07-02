# 0020 Boolean operators and given predicates

Status: Accepted

## Decision

Doria accepts typed equality, bool-only boolean operators, integer bitwise operators, and `given` predicate block rules as language design direction. The compiler now has an initial implementation slice for typed equality, bool-only boolean operators, Doria IR lowering, PHP backend lowering, and Stage 4b native lowering for supported `if` conditions. This decision still does not implement integer bitwise operators, broad native expression lowering, `Bool` helper APIs, `given`, `finally`, or `when`.

Backend support may lag this decision. Unsupported backend coverage must be described as unsupported backend coverage, not invalid Doria syntax.

## Typed Equality

Doria equality operators are:

```doria
==
!=
```

`==` is typed equality. `!=` is typed inequality. Doria does not use PHP-style loose comparison and does not use PHP strict-comparison operators to compensate for loose comparison semantics.

Doria does not introduce these operators:

```doria
===
!==
```

Examples:

```doria
1 == "1"       // type error, not true
false == 0     // type error, not true
$user != null  // valid only where null comparison is type-valid
```

This decision does not fully design nullable types. Nullable type syntax and nullability rules remain separate work unless already accepted elsewhere.

## Boolean Operators

Doria boolean operators are:

```doria
!
not

&&
and

||
or

xor
```

Rules:

- `not` is an exact synonym for `!`.
- `and` is an exact synonym for `&&`.
- `or` is an exact synonym for `||`.
- Doria has no PHP precedence split for `and` / `or`.
- Boolean operators require `bool` operands.
- Conditions must be `bool`.
- Doria does not use PHP-style truthiness.

These pairs are equivalent:

```doria
!$ready
not $ready

$user->isActivated && $user->can($permission)
$user->isActivated and $user->can($permission)

$this->isOrgMember($user) || $this->isAdmin($user)
$this->isOrgMember($user) or $this->isAdmin($user)
```

## xor

`xor` is boolean exclusive OR.

Truth table:

```text
true xor false  -> true
false xor true  -> true
true xor true   -> false
false xor false -> false
```

Rules:

- `xor` is bool-only.
- `xor` evaluates both operands.
- `xor` does not short-circuit.
- `xor` is not bitwise XOR.

Examples:

```doria
$request->hasPassword xor $request->hasSsoToken
$config->usesLocalStorage xor $config->usesCloudStorage
$input->leftPressed xor $input->rightPressed
```

`xor` is useful for mutually exclusive states, such as exactly one login method, exactly one config mode, or mutually exclusive game inputs.

## xor Ambiguity Guardrails

Chained or mixed `xor` expressions should require parentheses or produce a future diagnostic or lint.

Preferred strict future rule:

- Unparenthesized chained `xor` is not allowed.
- `xor` mixed with `and`, `or`, `&&`, or `||` requires parentheses.

Examples to reject or diagnose later:

```doria
$a xor $b xor $c
$a && $b xor $c
$a xor $b || $c
```

Examples to allow:

```doria
($a xor $b) xor $c
($a && $b) xor $c
$a xor ($b || $c)
```

Reason: developers often read `a xor b xor c` as "exactly one of these is true", but chained XOR normally means odd parity. Doria should avoid this ambiguity.

A future standard-library helper is better for exactly-one-of-many:

```doria
Bool::one($a, $b, $c)
```

This decision does not implement `Bool::one`.

## Bitwise Operators

Doria bitwise operators are:

```doria
&
|
^
~
```

Meaning:

- `&` is bitwise AND.
- `|` is bitwise OR.
- `^` is bitwise XOR.
- `~` is bitwise NOT.

Rules:

- Bitwise operators operate on integer types.
- Bitwise operators are not boolean operators.
- Boolean operators are not bitwise operators.
- `&` and `|` are not aliases for boolean AND/OR.
- Doria does not add `^^`.

## Operators Not Accepted As Syntax

Do not add these as core syntax unless Andrew explicitly accepts them later:

```text
nand
nor
implies
iff
unless
^^
===
!==
```

Notes:

- `nand` and `nor` are clearer as `!(a && b)` and `!(a || b)`.
- `implies` has surprising vacuous truth behavior and may belong later in contracts/assertions, not normal boolean syntax.
- `iff` / XNOR is usually expressible as typed `bool` equality.
- `unless` tends to make `else` branches harder to read.

Future helper APIs may be considered separately:

```doria
Bool::all(...)
Bool::any(...)
Bool::none(...)
Bool::one(...)
```

This decision does not implement those helpers.

## given Predicate Blocks

A `given` block attached to a control construct may contain:

- variable declarations
- void expression statements
- bool expression statements

Bool expression statements are predicates. Void expression statements are setup actions. Variable declarations introduce scoped names available to the attached control construct. Non-bool, non-void discarded expressions should be rejected.

Example:

```doria
given {
    let $user = $session->user;
    let $permission = Permission::EditPost;

    $user->isActivated;
    $this->isOrgMember($user) || $this->isAdmin($user);
} if ($user->can($permission)) {
    $post->publish();
}
```

Separate bool predicate lines are implicitly AND-ed in order.

This:

```doria
given {
    $user->isActivated;
    $this->isOrgMember($user) || $this->isAdmin($user);
    $user->can($permission);
} if ($post->isDraft) {
    $post->publish();
}
```

is conceptually like:

```doria
if (
    $user->isActivated
    && ($this->isOrgMember($user) || $this->isAdmin($user))
    && $user->can($permission)
    && $post->isDraft
) {
    $post->publish();
}
```

The scoped declarations remain scoped to the whole `given` plus attached control construct.

## given Execution Order

`given` statements execute in source order.

- Variable declarations bind names.
- Void expressions run setup actions.
- Bool expressions act as predicates.
- Bool predicates short-circuit the attached control condition and body when false.
- Inside a bool predicate, normal boolean short-circuiting applies for `&&` / `and` and `||` / `or`.
- `xor` does not short-circuit.

Example:

```doria
given {
    let $console = Console::current();

    $console->enterRawMode(); // void setup action
    $console->hideCursor();   // void setup action
    $console->supportsAnsi;   // bool predicate
} while ($game->running) {
    $game->draw($console);
} finally {
    $console->showCursor();
    $console->leaveRawMode();
}
```

This decision does not decide destructor scheduling, cleanup guarantees, borrow/lifetime behavior, or whether `finally` runs after every possible exit path. Those remain separate control-flow/runtime decisions.

## if, when, given, and finally

`if` is statement control flow. `if` does not return a value.

Rules:

- `if` without `else` is valid Doria.
- `else` is optional.
- `else if` is optional.
- `given` is optional.
- `finally` is optional.
- A base `if`, `while`, `foreach`, or future control construct does not require `given` or `finally`.

`when` is the value-returning conditional/control construct. This decision does not implement or fully specify `when`.

## Valid Doria vs Backend Coverage

Do not confuse "unsupported by the current native backend slice" with "invalid Doria".

If a construct is valid Doria but unsupported by native output, diagnostics, docs, and review comments must call it unsupported native backend coverage, not invalid language syntax.

This is especially important for:

- `if` without `else`
- `else if`
- `given`
- `finally`
- `when`
- wider boolean expressions
- broader control-flow shapes

## Non-goals

This decision does not:

- implement `and`, `or`, `not`, or `xor`
- implement bitwise operators
- implement `given`
- implement `finally`
- implement `when`
- implement `Bool` helper APIs
- design nullable type syntax
- design cleanup/destruction semantics
- change parser, lexer, semantic checker, Doria IR, PHP backend, or native backend behavior
