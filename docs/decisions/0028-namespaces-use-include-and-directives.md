# 0028 Namespaces, use, include, and directives

Status: Accepted

Implementation note: `docs/decisions/0031-stage-7b-break-continue.md` implements the first compiler and native smoke slice for `break;` and `continue;`. Namespace, `use`, `include`, and `declare` compiler support still remain future work.

## Decision

Doria accepts the following language directions:

- Doria will support namespaces.
- Doria will support `use` statements for semantic imports and name aliases.
- Doria will support `break`.
- Doria will support `continue`.
- Doria will support `include` as compile-time source inclusion with required include-once behavior.
- Doria will support `declare` as a structured compiler/source directive.

The following remain evaluation-only and are not accepted for implementation by this decision:

- `goto`
- C/C++-style textual preprocessor directives copied as syntax
- textual macro substitution such as `#define` and `#undef`

This is a language-design decision only. It does not implement lexer, parser, AST, HIR, semantic checker, Doria IR, PHP backend, native backend, or LSP support.

## Categories

These concepts solve different problems:

- `namespace`: logical symbol ownership and declaration scope
- `use`: semantic import and name aliasing
- `include`: compile-time required include-once source inclusion
- `declare`: structured compiler/source directive
- `break` and `continue`: runtime statement control flow
- `goto`: evaluation-only
- C/C++ textual preprocessor: evaluation-only

Do not collapse these categories into one mechanism.

## Namespaces

Namespaces define logical symbol ownership and declaration scope. They are part of semantic name resolution.

Namespaces are not source inclusion, package resolution, build orchestration, or runtime loading.

Accepted conceptual syntax:

```doria
namespace App\Services;

class UserService
{
}
```

Nested namespace paths are part of the intended direction:

```doria
namespace App\Domain\Users;
```

The backslash separator is the likely/default direction because it matches Doria's PHP-shaped readability. This decision does not implement or permanently lock grammar details.

Namespaces are required for serious multi-file programs, libraries, package ecosystems, and eventual self-hosting.

## use Statements

`use` imports names from namespaces at namespace/file-scope only. `use` is semantic name resolution.

`use` is not valid inside class, trait, interface, function, or method bodies.

`use` is not textual inclusion, PHP runtime include, package dependency resolution, or code execution.

Trait composition does not use this spelling. Class-body and trait-body trait composition uses `uses`, documented in `docs/decisions/0030-trait-composition-uses-keyword.md`.

Accepted conceptual syntax:

```doria
use App\Models\User;
use App\Security\Permission;
use App\Repositories\PostRepository as Posts;
```

Likely behavior:

- A `use` statement may import a fully qualified symbol.
- A `use` statement may alias a symbol.
- Duplicate or conflicting imports should be diagnosed.
- Unused import warnings may be added later.
- `use` does not load packages by itself; package resolution belongs to Baton later.

## include

`include` is compile-time source inclusion. It is lower-level source composition, not the normal import mechanism.

Accepted conceptual syntax:

```doria
include "src/generated/routes.doria";
```

Accepted intended semantics:

- `include` uses required include-once behavior.
- If the included file cannot be found, compilation fails.
- If the same canonical file is included more than once, it is included once.
- Include resolution must be deterministic.
- Include diagnostics must preserve source file and span information.
- Included source participates in the same compiler pipeline as normal Doria source.

Only string-literal local source paths are accepted in the intended direction.

Rejected direction:

```doria
include $path;
include getPath();
include "https://example.com/file.doria";
```

Doria does not add separate PHP-style forms:

```text
require
require_once
include_once
```

Reason: Doria `include` already means required include-once source inclusion.

## break

`break` exits the nearest enclosing loop.

Initial accepted direction:

```doria
while ($running) {
    if ($done) {
        break;
    }
}
```

PHP-style numeric break levels are not accepted by this decision:

```doria
break 2;
```

Labeled break may be evaluated later if needed, but it is not accepted here.

## continue

`continue` jumps to the next iteration of the nearest enclosing loop.

Initial accepted direction:

```doria
while ($i < 10) {
    $i += 1;

    if ($i == 5) {
        continue;
    }

    $sum += $i;
}
```

PHP-style numeric continue levels are not accepted by this decision:

```doria
continue 2;
```

Labeled continue may be evaluated later if needed, but it is not accepted here.

## declare

`declare` is a structured compiler/source directive. It is not a macro system and not textual substitution.

The exact grammar and allowed declaration keys require future decisions. Unknown declare keys should be rejected when `declare` is implemented.

Possible future purposes include:

- warning policy
- unsafe/FFI boundary policy
- backend/profile constraints
- platform configuration
- optimization intent
- feature gates
- compile-time diagnostics

## goto Evaluation

`goto` is not accepted for implementation yet.

Possible benefits:

- generated code
- finite-state machines
- parsers
- low-level cleanup paths
- escaping deeply nested control flow

Risks:

- bypassing variable initialization
- jumping into scopes
- breaking readonly/writable analysis
- complicating ownership/borrow checking over MIR later
- bypassing future `given` / `finally` cleanup obligations
- making definite-return analysis harder
- making CFG lowering harder

Doria should prefer structured control flow, labeled `break` / `continue` if accepted later, `when` / `match`-style constructs, and explicit state machines before accepting `goto`.

If `goto` is ever accepted later, likely restrictions include:

- same function only
- cannot jump into a deeper scope
- cannot jump past initialization that would be visible at the target
- cannot bypass cleanup or `finally` obligations
- cannot jump into or out of guarded resource regions
- cannot cross future ownership/borrow-checking boundaries

## C/C++ Textual Preprocessor Evaluation

Doria should not adopt a C/C++ textual macro preprocessor by default.

Comparison set:

```text
#include
#define
#undef
#if
#ifdef
#ifndef
#elif
#else
#endif
#warning
#error
```

Accepted conceptual mapping:

- `#include` maps to Doria `include`, with required include-once source inclusion.
- `#error` may map later to a structured compile-time error directive.
- `#warning` may map later to a structured compile-time warning directive.
- The `#if` family may map later to structured conditional compilation.
- `#define` is not accepted as textual macro substitution.
- `#undef` is not accepted as textual macro mutation.

Textual macros are dangerous for Doria because:

- token substitution bypasses normal parsing and semantic checking
- macro-expanded syntax can hide errors from source-level reasoning
- macro systems can undermine type safety
- macro conditionals can create unparseable inactive code
- macros complicate diagnostics, source maps, tooling, and future ownership/borrow checking over MIR

Doria source should remain parseable, typed, and semantically checked by `doriac`. If a directive changes compilation, it should do so through structured compiler semantics, not arbitrary token substitution.

## Non-goals

This decision itself did not implement:

- namespaces
- `use`
- `include`
- `declare`
- `goto`
- preprocessor directives
- conditional compilation
- macro expansion
- `#define`
- `#undef`
- `#if`
- `#ifdef`
- `#ifndef`
- `#elif`
- `#else`
- `#endif`
- `#warning`
- `#error`
