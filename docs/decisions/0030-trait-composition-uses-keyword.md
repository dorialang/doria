# 0030 Trait composition uses keyword

Status: Accepted

## Decision

Doria uses separate spellings for namespace imports and trait composition:

```text
namespace/file-scope use  -> semantic import / alias
class-body/trait-body uses -> trait composition
```

Accepted namespace import syntax:

```doria
namespace App\Services;

use App\Models\User;
use App\Security\Permission;
use App\Repositories\PostRepository as Posts;
```

Accepted trait-composition syntax:

```doria
trait HasSlug
{
    string $slug;
}

trait TracksChanges
{
    int64 $lastChangedAt;
}

class Article extends Model
{
    uses HasSlug, TracksChanges;
}

trait Auditable
{
    uses TracksChanges;
}
```

`use` is reserved for namespace/file-scope semantic imports and aliases. `uses` is reserved for composing traits into a class body or another trait body.

This is a language-design decision only. It does not implement lexer, parser, AST, HIR, semantic checker, Doria IR, PHP backend, native backend, native smoke IR, or LSP support.

## Rationale

Using separate spellings avoids overloading `use` by context and keeps source files easy to scan:

- `use App\Models\User;` reads as semantic name import.
- `uses HasSlug;` reads as member composition inside a type body.

Doria remains PHP-shaped, but it does not need to copy PHP's class-body `use TraitName;` spelling when a clearer spelling prevents ambiguity for users, editor tooling, diagnostics, and future migration tooling.

## Migration

The PHP-to-Doria migration path should rewrite class-body PHP trait use declarations to Doria `uses` declarations:

```php
class Article
{
    use TraitName;
}
```

```doria
class Article
{
    uses TraitName;
}
```

Namespace imports keep the PHP-shaped `use` spelling:

```doria
use App\Models\User;
```

## Future Diagnostics

When compiler diagnostics are implemented for this surface, Doria should guide likely spelling mistakes:

```doria
class Post
{
    use SlugTrait;
}
```

Suggested diagnostic:

```text
Trait composition uses `uses`, not `use`. Did you mean `uses SlugTrait;`?
```

```doria
uses App\Models\User;
```

Suggested diagnostic:

```text
Namespace imports use `use`, not `uses`. Did you mean `use App\Models\User;`?
```

## Updates

This decision updates `docs/decisions/0029-oop-declaration-vocabulary.md` by replacing the accepted trait-composition spelling from class-body `use` to class-body/trait-body `uses`.

It also clarifies `docs/decisions/0028-namespaces-use-include-and-directives.md`: `use` is only the namespace/file-scope import and alias spelling, not trait composition.
