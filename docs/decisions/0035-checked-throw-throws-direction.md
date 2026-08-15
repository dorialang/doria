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

Decision 0119 supplies the complete grammar, semantic, representation, and
three-slice implementation contract. Stage 29 Slice 1 implements checking,
AST/HIR, and the shared pre-MIR execution boundary. Checked-error execution,
runtime transport, and I/O migration remain in Slices 2 and 3.

## Unified diagnostic amendment

Decision 0109 binds checked errors to Doria's compiler-owned diagnostic and
runtime-outcome foundation without implementing them. Compile-time violations
are ordinary language diagnostics. Caught checked errors are ordinary control
flow and produce no automatic diagnostic. An error escaping `main` remains an
orderly status-70 outcome after required propagation cleanup and destructors;
it is not a panic and must not use panic's abort-without-cleanup semantics.

The future unhandled outcome reuses the same source identity, byte-span labels,
runtime-outcome extension, human/concise/JSON presentations, path
normalization, tooling component, and native transport family. The old
conceptual lowercase `error: <Class>: <message>` line is not a separate
renderer contract. Its final human presentation remains pending designer
review.

## Non-goals

This decision does not add:

- runtime error objects
- exception unwinding
- panic/fatal-error taxonomy
- interaction with `finally`
- standard-library error hierarchies
- native runtime exception machinery

These historical non-goals described this direction-only record before Decision
0119. They do not override the implemented Slice 1 grammar and semantic model.
