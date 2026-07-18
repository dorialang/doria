# 0007 DDO database abstraction

Status: Superseded by the plan's §9 DDO charter (the DDO decision, unauthored; to be recorded under a fresh number when DDO is scheduled, post-Stage-29).

> Superseded note. This is an early sketch that predates the §9 DDO charter,
> which revises it structurally: a decomposed API (`Connection`, `Statement`,
> `Transaction`, typed rows), not a `DDO` god-object class; checked-errors
> `throws` on every fallible call; an `Sql` provenance newtype; typed connection
> configuration (§9 rejects stringly DSNs); RAII transactions with a consuming
> `commit`; capability-based drivers. `DDO` is retained as the layer/brand name,
> not a class, so no class-casing question arises. The example below is pre-SPEC:
> its `DDO` connection-class, DSN-only construction, and
> `foreach ($users as UserRow $user)` typed binding are not the current design.
> Kept for history; do not build against it.

## Decision (superseded — see §9)

Doria will have a first-class database abstraction named DDO. DDO is similar in role to PDO, but it is Doria's own native-safe database layer.

Canonical rough example:

```doria
let $db = new DDO("mysql://user:pass@localhost/app");

let $statement = $db->prepare("
SELECT id, name, email
FROM users
WHERE active = ?
");

let $users = $statement->query([true]);

foreach ($users as UserRow $user) {
    echo $user->name;
}
```

## Notes

DDO should eventually support DSN construction, prepared statements, parameter binding, transactions, streaming result sets, typed row mapping, and deterministic cleanup.
