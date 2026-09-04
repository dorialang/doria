# 0029 OOP declaration vocabulary

> **Stage 35 amendment:** Decision 0134 fixes the complete nominal interface and
> compile-time trait contract. Its later rules govern Stage 35 without changing
> this record's declaration vocabulary.

Status: Accepted

Updated by: `docs/decisions/0030-trait-composition-uses-keyword.md` for trait-composition spelling.

## Decision

Doria accepts the PHP-shaped OOP declaration vocabulary where it fits Doria's native-first, statically checked goals:

- Doria will support `class`.
- Doria will support `interface`.
- Doria will support `trait`.
- Doria will support `extends`.
- Doria will support `implements`.

Doria is PHP-shaped, not PHP++. Accepting familiar OOP declaration words does not import PHP runtime behavior, PHP object semantics, or PHP's full class model.

This is a language-design decision only. It does not implement lexer, parser, AST, HIR, semantic checker, Doria IR, PHP backend, native backend, native smoke IR, or LSP support.

## class

`class` declares a class type.

Conceptual syntax:

```doria
class Post
{
}
```

Doria already has class syntax in the current compiler surface. This decision locks the OOP declaration vocabulary direction alongside `interface`, `trait`, `extends`, and `implements`.

## interface

`interface` declares a contract that classes can implement.

Conceptual syntax:

```doria
interface Renderable
{
    function render(): string;
}
```

Likely direction:

- interfaces may declare method requirements
- interfaces may extend one or more interfaces
- interface members do not define instance storage
- Doria's PHP-shaped direction points toward nominal interface conformance

This decision does not fully specify default methods, static interface methods, constants, generic interfaces, variance, or interface property requirements. Those require later design.

## trait

`trait` declares reusable class-body members.

Conceptual syntax:

```doria
trait HasSlug
{
    string $slug;
}
```

Likely direction:

- traits are composed into classes or other traits
- trait conflict-resolution rules must be defined before advanced trait behavior is implemented

This decision does not fully specify:

- method conflict resolution
- aliasing
- visibility changes through trait composition
- trait property rules
- trait static member rules
- trait abstract method requirements
- whether PHP-style `insteadof` and `as` are accepted exactly

## extends

`extends` establishes inheritance.

Conceptual class inheritance syntax:

```doria
class Post extends Model
{
}
```

Conceptual interface inheritance syntax:

```doria
interface JsonRenderable extends Renderable
{
}
```

Likely direction:

- a class may extend at most one class
- an interface may extend one or more interfaces

This decision does not fully specify constructor inheritance, initialization order, method override rules, virtual dispatch layout, final/sealed behavior, runtime layout, or ABI.

## implements

`implements` declares that a class satisfies one or more interfaces.

Conceptual syntax:

```doria
class Post extends Model implements Renderable, JsonSerializable
{
}
```

Decision 0134 accepts the complete direction:

- a class may implement one or more interfaces;
- conformance is nominal and checked by the compiler;
- method-name coincidence alone does not establish conformance;
- inherited and trait-provided methods may satisfy an authored `implements`
  contract under the exact substitution law.

Stage 35 Slice 1 owns implementation of those conformance facts and diagnostics.

## Trait Composition and uses

Doria has accepted `use` statements for namespace imports and accepted `uses` declarations for trait composition. The spellings are distinct:

```text
namespace/file-scope use  -> semantic import / alias
class-body/trait-body uses -> trait composition
```

Conceptual example:

```doria
namespace App\Posts;

use App\Models\Post;
use App\Security\Permission;

class Article
{
    uses HasSlug;
}
```

The parser can distinguish namespace/file-scope import `use` from class-body or trait-body trait-composition `uses` by spelling and context.

This decision does not implement either `use` imports or `uses` trait composition.

## Doria Guardrails

Doria is PHP-shaped, not PHP++.

Accepting PHP-shaped OOP declaration syntax does not automatically import:

- PHP dynamic object semantics
- PHP magic methods as core behavior
- PHP autoloading behavior
- PHP reflection behavior
- PHP loose typing
- PHP visibility rules beyond what Doria has separately accepted
- PHP trait conflict-resolution rules without review
- PHP runtime initialization behavior

Doria's accepted early member model remains default-accessible plus `internal`. `writable` controls mutation. `internal` controls API surface. OOP declaration vocabulary is accepted separately from final visibility, inheritance, interface dispatch, runtime layout, and ABI semantics.

## Non-goals

This decision does not implement:

- `interface`
- `trait`
- `extends`
- `implements`
- class-body or trait-body trait `uses`
- namespace/file-scope `use`
- visibility changes
- trait conflict resolution
- inheritance semantic checking
- interface conformance checking
- runtime object layout
- ABI behavior
- PHP magic methods
- PHP autoloading
- PHP reflection behavior
- PHP dynamic properties

## Stage 34 Amendment

Decision 0130 implements the class-inheritance portion of this vocabulary:
closed-by-default classes, explicit `open`/`override`, one parent class,
inherited lookup, parent construction, hierarchy narrowing, and virtual
dispatch. Decision 0134 fixes interface and trait semantics; their implementation
remains Stage 35 work.
