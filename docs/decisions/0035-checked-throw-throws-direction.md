# 0035 Checked throw/throws direction

Status: Accepted

## Decision

Doria uses checked thrown errors as the default surface error model.

`throw` raises an error:

```doria
throw new UserNotFound($id);
```

`throws` declares possible thrown error types in a function or method signature:

```doria
function loadUser(int $id): User throws UserNotFound, DatabaseError
{
    if ($id < 1) {
        throw new UserNotFound($id);
    }

    return $repository->findUser($id);
}
```

Thrown errors are checked by the compiler:

- a function may only throw errors declared in its `throws` clause unless they are caught internally
- a caller must catch the error or include it in its own `throws` clause
- runtime panic or fatal-error behavior is separate from checked `throw`/`throws`

`Result<T, E>` is not Doria's default surface error model unless a later decision explicitly adopts it.

## Conceptual examples

```doria
function renderProfile(int $id): string throws UserNotFound, DatabaseError
{
    let $user = loadUser($id);

    return $user->name;
}
```

```doria
function renderProfile(int $id): string
{
    try {
        let $user = loadUser($id);

        return $user->name;
    } catch (UserNotFound $error) {
        return "Unknown user";
    } catch (DatabaseError $error) {
        return "Service unavailable";
    }
}
```

## Implementation status

This decision records the language direction only. It does not implement compiler behavior for `throw`, `throws`, `try`, or `catch`.

## Non-goals

This decision does not add:

- parser support for `throws` clauses
- semantic checking for thrown error declarations
- `throw` statement lowering
- `try`/`catch` compiler behavior
- runtime error objects
- exception unwinding
- panic/fatal-error taxonomy
- interaction with `finally`
- standard-library error hierarchies
- native runtime exception machinery
