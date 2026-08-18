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

Thrown errors are checked by the compiler. Decision 0119's accepted 2026-08-18
amendment adds one entrypoint exception to this early direction:

- an ordinary reusable function may only allow errors declared in its `throws` clause to escape
- a clause-free selected `main` infers its exact escaping effects instead
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
three-slice implementation contract. All three Stage 29 slices are complete.
Its inferred-main corrective amendment preserves explicit contracts on reusable
callables while letting the selected entrypoint infer its effective checked
effects without synthesizing source syntax.

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
