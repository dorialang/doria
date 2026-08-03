# Decision 0111: Grouped Local Declarations

- **Status:** Accepted
- **Date:** 2026-08-03
- **Owners:** Doria language and compiler design
- **Scope:** Local declarations that initialize two or more independent bindings from one Copy value

## Context

Doria already has inferred and explicitly typed local declarations, each readonly by default with an optional shared `writable` prefix. Repeating a side-effecting initializer merely to seed several Copy locals is noisy and can change behavior. Importing C-style declarators, chained assignment, tuple destructuring, or implicit cloning would conflict with Doria's ownership and declaration rules.

## Decision

A local declaration may name two or more bindings before its single initializer. The bindings are independent locals. Grouping is declaration syntax only: there is no runtime group object, tuple, collection, or shared owner.

## Canonical Syntax

The four forms are:

```doria
let $left, $right = 0;
let writable $left, $right = 0;
int $left, $right = 0;
writable int $left, $right = 0;
```

Every binding carries the ordinary `$` sigil. A group contains at least two bindings and has no trailing comma. Existing single-binding declarations are unchanged. Ordinary whitespace and line breaks may surround commas.

## Common Type And Mutability

One inferred or explicit type applies to the complete group. One readonly-or-`writable` mode also applies to the complete group. Per-binding types and per-binding mutability modifiers are rejected; declarations that need different shapes remain separate declarations.

## Single Evaluation

The initializer is evaluated exactly once, before any new binding enters scope. Its result is then copied into the bindings from left to right. An initializer cannot refer to any name introduced by its own group.

## Copy Eligibility

Eligibility follows the compiler's existing Copy classification. Copy scalars and immutable `string` values may initialize a group. Each binding receives an independent scalar value or string handle. A string group retains the shared immutable handle once per binding; it never duplicates the string contents.

## Move-Type Rejection

Concrete classes, collections, `Bytes`, `mixed`, shared-ownership handles and access objects, and symbolic generic values are move types and cannot initialize multiple bindings. The compiler does not hide a `clone()`, `share()`, or repeated initializer evaluation behind grouped syntax.

## Nullable Empty Initialization

An explicitly typed nullable move type may initialize a group from the literal `null`, because no owner exists to duplicate:

```doria
?Token $left, $right = null;
```

An untyped grouped `null` is rejected because it does not reveal the shared nullable payload type. A non-null nullable move value remains a move value and is rejected.

## Scope And Name Resolution

The declaration is atomic. The initializer is checked in the preceding scope. Every binding name and duplicate is checked before any group name is inserted. If any part fails, no binding from the group enters scope. Each binding retains its own source span for diagnostics, hover, references, and rename.

Grouped declarations are local-only. They do not alter property, parameter, promotion, static-property, constant, `foreach`, or closure-capture grammar.

## Initialization Order

After the single initializer evaluation, bindings initialize left to right in source order. This order is semantic even when the backend can eliminate the temporary.

## Destruction Order

Each binding has its ordinary independent lifetime. Still-live cleanup therefore follows decision 0081: reverse declaration/acquisition order on structured exits. Abort-only panic still runs no cleanup.

## Lowering Model

AST and HIR preserve the ordered binding list, each binding span, and one initializer. MIR has one canonical grouped-local initializer whose validator independently requires at least two ordered, unique, non-synthetic targets with one common type, mutability mode, and ownership mode. It permits Copy values and the exact typed nullable-move `null` exception only.

The interpreter, Cranelift, and LLVM evaluate that MIR initializer once and materialize each binding in order. PHP uses a collision-safe synthetic temporary, ordered assignments, and explicit temporary cleanup; it never emits chained assignment.

## Performance Contract

Grouped locals are zero-abstraction-cost syntax. Grouping alone creates no tuple, collection, heap allocation, dynamic dispatch, or async-runtime interaction. Copy cost matches equivalent separate local initialization from an already-evaluated value. String contents are not copied. The compiler temporary is eligible for elimination, and no runtime representation records that the locals were authored as a group.

Stage 26b owns the general benchmark foundation; this feature does not introduce a private benchmark framework.

## Diagnostics

Diagnostics use stable codes and Title Case titles. They identify the exact offending binding, modifier, type, initializer separator, trailing comma, or move-valued initializer. Move rejection explains that one owned value cannot create multiple independent owners. No diagnostic recommends unavailable hidden cloning or sharing.

## Explicit Non-Goals

- Per-binding initializers, types, or mutability.
- Trailing commas.
- Destructuring, tuple assignment, or multiple-return destructuring.
- Grouped properties, parameters, constructor promotion, static properties, constants, `foreach` bindings, or closure captures.
- Implicit cloning, implicit sharing, or a new public Copy interface.
- Stage 26b benchmark infrastructure, Stage 27 enums, Stage 35a optimizer work, or Stage 36a streams.

## Consequences

- Stage 26a completes grouped local declarations across the frontend, semantic and ownership analysis, canonical MIR, all three native execution paths, PHP compatibility, diagnostics, fixtures, and tooling coordination.
- Stage 26b becomes the next stage and establishes the repository-owned performance baseline before Stage 27.
- Stage 35a is scheduled after Stage 35 for optimizer contracts, dispatch, and escape analysis.
- Stage 43 consumes and broadens the Stage 26b benchmark system; Stage 36a retains ownership of its initial stream-performance gate.
- Later runtime-affecting stages must record a `Performance Impact` section under the master-plan rule.

## Invalidated Elsewhere

- The target-state website must document grouped locals as completed Doria syntax with fabricated examples such as `let writable $name, $color, $shape = "";` and `writable int $age, $height = 0;`. This task does not modify the website repository.
- The language server must consume the final compiler revision and expose every binding independently without duplicating grouped-declaration semantics.
