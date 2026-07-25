# Decision 0103: String companion surface

**Status:** Accepted (establishes the `String` companion as the home for pure
string transforms, and draws the boundary between the three ways string operations
are spelled). Seeds the minimal transform set; the fuller surface grows with the
strings work.

## Context

The website guides teach `String::trim(...)` and `String::lower(...)` as built-in
companions — five guides use them, and the statics-and-constants guide states it
outright: "Built-in companions use the same call syntax. For example,
`Int::parse($value)` … and `String::trim($value)` trims text." But no `String`
companion method was ever scheduled: the plan's string surface is `$s->` intrinsic
members (`length`/`isEmpty`/`bytes`/`chars`, §4.6, 0045) plus the PHP-familiar
`str_*` free-function family (`str_starts_with`, `str_case_compare`, §9.1/0074).
So the guides promise a surface the plan does not own — and, left unresolved, they
also risk a *third* spelling for the same operations, which Doria's one-spelling
rule (the `print`/`echo` ban) forbids.

The `Int`/`Float`/`Bool`/`String` companions already exist as a concept
(§Core primitive companions; `Int::parse`, `Int::toFloat`, `Float::parse` are
scheduled). This record puts string transforms on the `String` companion to match,
and fixes which of the three spellings owns what so there is exactly one home per
operation.

## Decision

### `String` is the companion home for pure string transforms

A **string transform** — a pure operation that takes a `string` and returns a new
`string` (or a parsed `?T`) — lives on the **`String` companion**, spelled
`String::method(...)`, PascalCase companion + camelCase method, exactly like the
`Int`/`Float` companions. Because `string` is immutable UTF-8 (0045), a transform
never mutates; it returns a new value.

The seed set, ratifying what the guides already use plus the obvious sibling:

- `String::trim(string): string` — strip leading/trailing ASCII whitespace.
- `String::lower(string): string` — ASCII/Unicode-lowercased copy.
- `String::upper(string): string` — the uppercase sibling.

This is the **minimal** set. The fuller transform surface (`replace`, `split`,
`padStart`/`padEnd`, `substring`/slicing, `repeat`, case-fold specifics, and the
Unicode-vs-ASCII contract for `lower`/`upper`) is settled with the strings work,
marked *(surface TBD)* in the stdlib reference until then. `String::parse`-shaped
companions are not introduced here (parsing to numbers is `Int::parse`/`Float::parse`).

### The three-spelling boundary — one home per operation

To keep exactly one spelling per operation:

- **`$s->` intrinsic members** own *data about the string* — `length`, `isEmpty`,
  `bytes`, `chars` (deferred). These are properties of the value, not transforms.
- **`String::` companion** owns *pure transforms* — `trim`, `lower`, `upper`, and
  the fuller transform set above.
- **The `str_*` free-function family** owns the *PHP-familiar predicate/search
  layer* — `str_starts_with`, `str_case_compare`, and the rest of that family. It
  **does not** duplicate the companion transforms: there is deliberately **no**
  `str_trim` / `str_lower` / `str_upper`. A given operation is spelled exactly one
  way.

This mirrors the numeric companions (`Int::parse` is `Int::`, not `int_parse`) and
keeps the §9.1 naming charter intact (companion/member APIs camelCase; the
remaining free functions snake_case).

### Scheduling

The `String` companion transforms are primitive-companion surface, the same tier
as the `Int`/`Float` companions, and land with the strings/companion work (they do
not depend on collections or later stages). The seed set is small and pure; the
fuller surface follows the strings work.

## Alternatives considered

- **Fix the guides to `str_trim`/`str_lower` instead (make transforms free
  functions).** Rejected — it would grow the `str_*` family into transforms and
  split string operations across two rationales (some `str_*`, some `$s->`), and it
  diverges from the `Int`/`Float` companion precedent. Companions are the
  established home for typed operations on a primitive.
- **Put transforms on `$s->` members (`$s->trim()`).** Reasonable and PHP-fluent,
  but it collides with the guides' shipped `String::` spelling and would still need
  a ruling to retire one of the two. Kept as a possible future ergonomic addition,
  not the v1.0 canonical spelling. `$s->` stays reserved for intrinsic properties.
- **Allow both `String::trim` and `str_trim`.** Rejected — that is the exact
  `print`/`echo` redundancy Doria bans.

## Consequences

- The guides' `String::trim`/`String::lower` are correct as written and now have a
  scheduled home; no guide edits are required for these.
- String operations have one home each: properties on `$s->`, transforms on
  `String::`, PHP-familiar predicates/search on `str_*`.
- The `str_*` family will never carry a transform that duplicates a `String::`
  method.

## Affected components

The stdlib reference (String entry — add the `String::` companion transforms),
plan §4.6 / §Core primitive companions and §9.1 (the boundary note), semantic
analysis and the runtime `String` companion intrinsics when implemented, and SPEC
when the methods land. No compiler behavior changes with this record; it schedules
and bounds a surface.

## Invalidated elsewhere

- The stdlib reference's `String` entry — now also lists the `String::` companion
  transforms, not only `$s->` members and the `str_*` family.
- Any assumption that string casing/trimming is a `str_*` free function — it is a
  `String::` companion transform; `str_trim`/`str_lower`/`str_upper` do not exist.
