# Decision 0104: Primitive companion completeness

**Status:** Accepted (establishes the completeness invariant across every
primitive companion, defines the uniform v1.0 baseline, and fills the one
remaining hole — the `Bool` companion). Consolidates the companion surface owned
in detail by decisions 0013/0016/0042/0095/0096 (numerics) and 0103 (string);
adds no numeric or string members those records did not already settle.

## Context

Doria has a full, closed list of primitive types — the integer family (`int`,
`int8`…`int64`, `uint8`…`uint64`), the float family (`float`, `float32`,
`float64`), `bool`, and `string` — each with a PascalCase companion
(`Int`/`Int32`/`UInt64`/…, `Float`/`Float32`/`Float64`, `Bool`, `String`) that is
a reserved, unshadowable name. But the *surface* on those companions grew
piecemeal and asymmetric:

- `Int`/`Float` carry a real, documented surface (`parse`, cross-kind conversion,
  `pow`, and for integers wrapping arithmetic and per-width `from`/`tryFrom`).
- `String` was settled by 0103 (pure transforms: `trim`/`lower`/`upper` seed).
- **`Bool` had a reserved name and *zero* members** — the stdlib reference
  literally read "companion helpers" with not one method named. `Bool::parse`,
  the direct analogue of `Int::parse`/`Float::parse`, did not exist.

Nothing in the plan or records asserted that the companion set is *complete* or
that companions share a *consistent* surface. So a real primitive (`bool`) had no
usable companion while its siblings did, and the gap read as an oversight rather
than a decision. This record removes the asymmetry and states the invariant that
prevents it recurring: the companion surface is designed as a complete matrix,
simple in v1.0 and furnished in v1.0+, with no silent holes.

## Decision

### The completeness invariant

**Every primitive type has a companion, and every companion carries the uniform
baseline below.** A baseline member that is *meaningful* for a primitive is
present and spelled *identically* across every primitive where it applies; a
member that is *not* meaningful for a primitive is an **explicit, documented N/A
with a reason**, never a silent absence. A primitive companion is never shipped
"empty pending later work" — if a member is not in v1.0 it is named as a v1.0+
furnishing, so the surface is always complete as stated, just deliberately small.

### The uniform v1.0 baseline

- **`parse(string): ?T` — the universal text→value constructor.** Present on every
  **scalar value** companion: the integer family (`?intN`), the float family
  (`?floatN`), and `bool` (`?bool`). It returns the parsed value or `null` when
  the text is not a valid literal of `T` (or is out of range). This is the one
  member every scalar companion shares; it is why `Int::parse`, `Float::parse`,
  and now `Bool::parse` all read the same.
- **Display is uniform and lives off the companion.** Every primitive is
  display-convertible through the ordinary display path (interpolation / `.` /
  `echo` / `%s`), so no companion carries a `toString` — the uniform display
  spelling is the value itself, not a companion call. This is a *consistent*
  treatment, not a gap.
- **`String` is the one principled exception to `parse`.** `string` is the parse
  *domain*, not a parse *target*: `String::parse` would be identity and is
  therefore a documented N/A. In its place `String` carries the pure-transform
  baseline settled in 0103 (`trim`/`lower`/`upper` seed). So `String`'s baseline
  is complete and consistent under the same "companion owns the pure operation"
  rule — the member that differs is a deliberate, categorical distinction, not an
  omission.

### The `Bool` companion (the hole this record fills)

`Bool` gains the baseline scalar member and nothing more in v1.0:

- **`Bool::parse(string): ?bool`** — returns `true` for exactly `"true"`, `false`
  for exactly `"false"`, and `null` for anything else. **Case-sensitive, exact
  match, no surrounding whitespace tolerated** — consistent with Doria's rejection
  of loose/coercive comparison, and trivially extensible. Callers that want looser
  input normalize first (`String::trim`/`String::lower`, 0103) and then parse — one
  operation per home, no coercion baked into the parser.

`Bool` carries no `MIN`/`MAX`, no `pow`, no wrapping arithmetic — all N/A for a
two-valued type, and documented as such.

### v1.0+ furnishing (named now so the matrix is complete, not open-ended)

These are the agreed extension points; they are **not** in v1.0 but are recorded
so the surface is closed-with-a-roadmap rather than half-specified:

- Numeric companions: `MIN`/`MAX` (and float `EPSILON`, `isNan`/`isInfinite`)
  bound/classification constants; a broader math surface.
- `Bool::toInt(bool): int` (`false`→0, `true`→1) — deferred because bool↔int is a
  coercion Doria should introduce deliberately, not by default.
- `String`: decision 0103 now owns the complete canonical string-specific
  vocabulary and Unicode contracts; implementation still lands incrementally,
  beginning with the Minimum String Runtime Surface.

## Alternatives considered

- **Leave `Bool` reserved-name-only for v1.0.** Rejected — it is a real primitive;
  a companion with no members while every sibling has `parse` is exactly the
  asymmetry this record exists to remove.
- **Lenient `Bool::parse` (case-insensitive, accept `"1"`/`"0"`, trim
  whitespace).** Rejected for v1.0 — that is coercion, which Doria spells out
  explicitly elsewhere; strict now, looser later is a safe widening, the reverse
  is a breaking change.
- **Add `toString` to every companion for symmetry.** Rejected — display is
  already uniform through the display path; a companion `toString` would be a
  second spelling of one operation, the `print`/`echo` redundancy Doria bans.
- **A single generic `Parse<T>` facility instead of per-companion `parse`.**
  Rejected for v1.0 — the per-companion static is what the guides teach and what
  `Int`/`Float` already ship; a generic parse facility is a post-generics
  consideration, not a v1.0 unification.

## Consequences

- The primitive companion surface is a **complete, symmetric matrix**: every
  scalar companion has `parse`, `String` has its transform baseline, display is
  uniform, and every absence is a named N/A or a named v1.0+ furnishing.
- `Bool::parse` is a small, scheduled implementation task (the parse machinery
  already exists for `Int`/`Float`; `Bool` reuses it). It lands as a compiler slice
  with interpreter/Cranelift/LLVM parity and is then documentable by the language
  server.
- No numeric or string companion member changes; this record cites their owning
  decisions and adds only the invariant and the `Bool` baseline.
- The stdlib reference's "companion helpers" placeholder for `Bool` is replaced by
  a real, named surface; the whole companion block reads as complete.

## Sequencing

`Bool::parse` implements with the primitive-companion surface (it reuses the
existing `parse` lowering used by `Int`/`Float`); it is a small slice schedulable
independently of the collection/mixed work. The v1.0+ furnishings land when their
motivating work does (math surface, generics, the strings pass). This record is
the completeness contract; it blocks nothing already scheduled.

## Affected components

Semantic analysis and MIR lowering for `Bool::parse` (reusing the scalar `parse`
path), the interpreter and both native backends (parity), shared MIR validation,
plan §9 / §12, the stdlib reference (the `Bool` entry and the companion-block
framing), the `doria-language-server` hover surface (companion coverage must be
uniform across `Int`/`Float`/`Bool`/`String`, per the AGENTS Language-server
sweep), and SPEC when `Bool::parse` implements.

## Invalidated elsewhere

- The stdlib reference's "`Bool` — companion helpers" line — replaced by the named
  `Bool::parse` surface and the N/A statements for `MIN`/`MAX`/`pow`/wrapping.
- Any statement or hover implying the primitive companions are a per-type ad-hoc
  collection rather than a uniform, complete matrix.
- The language-server companion-hover asymmetry (`Int` companions carry identifier
  hovers while `Float`/`Bool`/`String` do not) — the sweep must make companion
  hover coverage uniform, over surface that actually exists (so `Bool::parse` once
  it lands, not before).

Decision 0132 confirms that canonical display expressions materialize ordinary
strings without changing this matrix. Primitive companion `toString`, primitive
instance `toString`, `String::from` scalar overloads, and scalar casts remain
absent.
