# Decision 0094: Ternary conditional and the `.=` / `??=` assignment operators

**Status:** Accepted (semantics and desugaring settled; implementation folds into
the relevant stages, and SPEC's grammar is amended as each lands).

## Context

A surface-completeness review found three PHP-familiar operators missing from
Doria's grammar: the ternary conditional `cond ? a : b`, string concat-assign
`.=`, and null-coalescing assign `??=`. None is in SPEC's operator inventory; the
lexer tokenizes `.` and `??` but not `.=`/`??=`, and there is no ternary
production. Doria has `match` as its value-conditional and `if` as a *statement*,
so before this record there was **no concise inline conditional at all** — the
ubiquitous `$x = $cond ? a : b` had only the three-line `match ($cond) { true =>
a, false => b }` spelling.

## Decision

### Ternary — `cond ? a : b`

Doria gains the full ternary conditional expression, under two disciplines:

- The condition is a **strict `bool`** — no PHP truthiness, the same rule as
  `match (bool)`. A non-`bool` condition is a compile error.
- It is **sugar for a two-arm `match`**: `cond ? a : b` is exactly
  `match ($cond) { true => a, false => b }`. Identical lowering, branch-type
  unification, and move/ownership rules; only the selected branch is evaluated.
  No new semantic machinery.

**PHP's short ternary `?:` (Elvis) is rejected.** Its truthiness-fallback fights
strict `bool`, and `??` already owns null-fallback, so `$a ?: $b` would be the
redundant footgun spelling Doria bans. `$a ?: $b` is a compile error suggesting
`??` (null-fallback) or the full `? :`. A nested ternary parses right-associatively
as in C/PHP; `match` remains the tool for multi-way selection.

### `.=` — string concat-assign

`$place .= $rhs` is `$place = $place . $rhs` — a read-modify-write over any
writable place (§3.2), exactly like `+=`. Because `string` is immutable it
produces a new string, as `+=` produces a new int. It joins the
compound-assignment family, and the current MIR "string compound assignment is
invalid" rejection is narrowed to admit it.

### `??=` — null-coalescing assign

`$place ??= $rhs` is `$place = $place ?? $rhs` — assign `$rhs` only when `$place`
is currently null. It builds on the nullable model and `??` (decision 0093): the
place is `?T`, `$rhs` is assignable to the result, and it is a read-conditional-
write over the same place rule.

## Alternatives considered

- **An `if`-expression instead of ternary** (Rust/Kotlin `let $x = if (c) a else
  b`): rejected as the primary spelling — PHP has ternary, not if-expressions,
  and Doria is PHP-shaped; making `if` expression-position is a larger grammar
  change for a less familiar surface. Ternary-as-`match`-sugar is smaller and
  more familiar. An if-expression is not foreclosed for later, but ternary is the
  v1.0 answer.
- **`match`-only, no ternary:** rejected — forces the ubiquitous binary
  conditional into a three-line `match`, a constant papercut and a visible
  verbosity mark against PHP.
- **Loose-bool (truthy) ternary condition:** rejected — Doria has no truthiness.
- **Elvis `?:`:** rejected — redundant with `??`, fights strict `bool`.

## Consequences

- The common inline conditional has a concise spelling; `match` stays for
  multi-way and exhaustive selection.
- Ternary introduces no new lowering (two-arm `match`); type unification, the
  borrow checker, and move rules apply unchanged.
- `.=` and `??=` complete the compound-assignment family, so every binary
  operator Doria has — including `.` and `??` — now has its compound form.
- No truthiness is introduced anywhere.

## Sequencing

- **Ternary** lands with or after Stage 28 `match`, reusing its branch typing,
  ownership, and lowering once that grammar and implementation exist. Until
  then, this record settles semantics but does not expose ternary syntax.
- **`.=`** lands with string read-modify-write (string concat exists; the
  compound-assign place machinery is §3.2 / the property-RMW work).
- **`??=`** follows Stage 22 in a compound-assignment implementation slice once
  its lexer, parser, semantic, and lowering support can land together. Stage 22
  provides the prerequisite `??` semantics but does not expose `??=` syntax.
- SPEC's grammar and the lexer (`.=` and `??=` tokens, a ternary production) are
  amended as each lands; SPEC tracks the implemented surface, so this record does
  not amend it now. This record does not open a broader operator sweep (`<=>` and
  any others are a separate completeness pass).

## Affected components

Lexer (new tokens), parser (ternary production; `.=`/`??=` compound-assign
parse), semantic analysis (strict-`bool` ternary condition; `??=` nullable
typing), HIR/MIR lowering (ternary → two-arm `match`; `.=`/`??=` read-modify-
write), diagnostics (Elvis and truthiness rejection), plan §3.2/§4.4 pointers,
`doriac migrate php` (Elvis → `??`), and SPEC at implementation.

## Invalidated elsewhere

- The MIR "string compound assignment is invalid" rejection — narrowed to admit
  `.=`.
- Any assumption that Doria has no inline conditional, or that the
  compound-assignment family is closed at the arithmetic/bitwise set.
- PHP-migration guidance gains: `?:` (Elvis) → suggest `??`; a truthy ternary
  condition → strict `bool`.
