# 0007 DDO database abstraction

Status: Accepted

## Decision

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
