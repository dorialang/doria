# Decision 0095: Operator surface — exponentiation, three-way comparison, and rejected PHP operators

**Status:** Accepted (completes the operator-surface sweep; the two open
operators are resolved as typed methods, and the remaining PHP operators are
explicitly ruled in or out).

## Context

A completeness sweep of Doria's operator surface against PHP's found it already
~95% settled. SPEC's "Equality and boolean operators" section and decisions 0020
/ 0041 / 0042 / 0072 / 0093 / 0094 fix: typed `==`/`!=` (no loose comparison,
`===`/`!==`/`<>`/`^^` rejected); the boolean set `!`/`&&`/`||` with `not`/`and`/
`or` as **exact synonyms** (not PHP's low-precedence forms) and eager `xor`;
bitwise/shift; casts as explicit conversions (`Int::toFloat`, not `(int)`); `is`
not `instanceof`; nullable `??`/`?->`; and ternary/`.=`/`??=` with the short
ternary `?:` rejected.

Two operators remained genuinely undecided — `**`/`**=` (exponentiation) and
`<=>` (three-way comparison) — and a few PHP operators were unaddressed (`@`,
backtick, references, spread). This record closes the surface.

## Decision

### Exponentiation `**` / `**=` — rejected as operators; use `Int::pow` / `Float::pow`

No `**` or `**=` operator. Exponentiation is `Int::pow(int): int` (checked;
overflow panics; a negative exponent panics, since integer exponentiation is
non-negative) and `Float::pow(float): float`, on the numeric companion APIs.

Rationale: every systems language uses a method, not an operator (Rust `.pow`,
C# `Math.Pow`, Go/Java the same); it routes through the companion-API surface
Doria already uses (`Int::wrappingAdd`, `Int::toFloat`); and it avoids `**`'s
precedence footgun (`-2 ** 2` binds as `-(2 ** 2)` in operator languages) and the
integer/float/negative-exponent ambiguity, which a typed method contract states
explicitly. migrate-php: `$a ** $b` → `Int::pow`/`Float::pow` by operand type.

### Three-way comparison `<=>` — rejected as an operator; use `Comparable::compare` returning `Ordering`

No `<=>` operator. Three-way comparison is `Comparable::compare(other): Ordering`,
where `Ordering` is a core enum `{ Less, Equal, Greater }`.

Rationale: PHP/Ruby's `<=>` returns a magic `-1`/`0`/`1` int — un-Doria (magic
numbers, and Doria has no truthiness to lean on). A typed `Ordering` matched with
`match` is the idiom, matches Rust (`cmp` → `Ordering`) and C# (`CompareTo`), and
gives sorting a typed contract. This supplies what decision 0092 needs:
`SortedDictionary`/`SortedSet`/`PriorityQueue` order by `Comparable::compare`. The
full `Comparable`/`Ordering` surface is finalized with the interfaces and
collections decisions; the **no-operator / typed-`Ordering` shape** is settled
here. migrate-php: `$a <=> $b` → `$a->compare($b)`.

### Explicitly rejected PHP operators (so diagnostics and migrate-php have an authority)

- **`@` error suppression** — rejected; Doria has checked errors and no
  silent-failure surface. Fixit: handle the `throws`/nullable result.
- **Backtick shell execution** `` `...` `` — rejected; process execution is the
  `Doria\Std\Process` module, never implicit shell.
- **PHP reference `&$var`** (reference parameters/assignment) — rejected; aliasing
  is the ownership/borrow model (`writable`/`take`). `&` is the bitwise-AND
  operator only.

Already settled elsewhere, cited not re-decided: `===`/`!==`/`<>`/`^^`/`nand`/
`nor`/… rejected (SPEC §equality-and-boolean, 0020); C-style casts → explicit
conversions (0042); `instanceof` → `is`; short ternary `?:` → `??` (0094).

### Deferred

`...` spread and variadic **user-function** parameters — the general spread/
variadic surface is the named-arguments future slice (today's `...args` is limited
to the compiler-known `sprintf`/`printf`). Not an operator gap to close now;
reopen with the named-arguments/variadic work.

## Alternatives considered

- **Add `**`/`**=` as operators (PHP/Python parity):** rejected — the precedence
  footgun, the integer/float/negative-exponent ambiguity, and inconsistency with
  every systems language outweigh the ergonomic win; `Int::pow`/`Float::pow` are
  clear and typed.
- **`<=>` returning `int` (-1/0/1):** rejected — magic-number return; `Ordering`
  is the typed, match-friendly form.
- **`<=>` returning `Ordering` (operator, but typed):** rejected — a spaceship
  operator with an enum result is idiosyncratic; `compare` on `Comparable` is
  where ordering belongs and needs no new operator token.

## Consequences

- The operator surface is complete and internally consistent: every PHP operator
  is accepted, rejected, or replaced by a named typed method, each with an
  authority to cite.
- Exponentiation and three-way comparison are typed method contracts, not
  operators — consistent with the companion-API / interface surface.
- `Ordering` (core enum) and `Comparable::compare` are the ordering foundation the
  sorted collections (0092) build on.
- No new operator tokens are added; `**`/`<=>`/`@`/backtick/`&$` are
  recognized-and-rejected with fixits.

## Affected components

Semantic analysis / diagnostics (recognize-and-reject `**`/`<=>`/`@`/backtick/`&$`
with fixits), the numeric companion APIs (`Int::pow`, `Float::pow`), core stdlib
(`Ordering` enum, `Comparable::compare`), `doriac migrate php` (the fixit table),
plan §9 core layer and the §12 catalogue, and SPEC's operator/rejection notes when
the diagnostics and methods land (SPEC tracks the implemented surface).

## Invalidated elsewhere

- Any assumption that exponentiation or three-way comparison would be operators.
- The migrate-php / unknown-operator fixit table gains `**` → `Int::pow`/
  `Float::pow`, `<=>` → `compare`, `@` → handle-the-result, `` ` `` →
  `Doria\Std\Process`, `&$` → ownership.
- Decision 0092's reliance on `Comparable` ordering now has a concrete shape
  (`compare(): Ordering`).
