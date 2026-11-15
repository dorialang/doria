# Decision 0146: String Display Width

- **Status:** Accepted
- **Date:** 2026-11-15
- **Owners:** Doria language and standard library design
- **Scope:** The number of terminal cells a string occupies, and where that measurement lives

## Context

A terminal draws text in a grid of cells. A Latin letter takes one cell, a
Chinese character or an emoji takes two, and a combining accent takes none
because it is drawn over the letter before it. Grapheme count (`$text->length`)
does not give the number of cells, so a program that lays text out in a
terminal needs a separate measurement.

Decision 0140 left this measurement to the String API: "Console does not
measure text. Display width is a string question owned by the String API."
The String API had no such member, and the 2026-07-31 review of the PHP string
inventory had listed display width as domain-owned with no owner named. The
`Doria\Tui` library, next on the plan's approved sequence after Stage 46,
cannot lay out wide characters, emoji or combining marks without it.

## Decision

Andrew ruled on 2026-11-15 ("$text->displayWidth (Recommended)"):

```doria
$text->displayWidth
```

`displayWidth` is a readonly `int` property beside `length` and `byteLength`.
It is a measurement of the string, so it is a property under Decision 0103's
instance boundary.

### How it is measured

The width of a string is the sum of the widths of its Unicode extended
grapheme clusters, the same clusters `length` counts. One cluster's width is
decided by the first character in it that is drawn:

- an East Asian Wide or Fullwidth character takes **2** cells;
- an emoji takes **2** cells when it is shown as an emoji, either by default
  (`Emoji_Presentation`) or because the cluster contains U+FE0F;
- every other drawn character takes **1** cell, East Asian Ambiguous and
  Halfwidth characters included;
- a cluster with no drawn character takes **0** cells.

Control characters, line and paragraph separators, format characters,
nonspacing and enclosing marks, and default ignorable characters are not
drawn. A cluster made only of them, such as a newline, a tab, a zero-width
space or a lone combining accent, takes no cells.

The Unicode data is the version the String implementation already uses
(`UNICODE_VERSION`, 17.0.0), so `displayWidth` and `length` always segment a
string the same way.

### One implementation

`doria_unicode::display_width` is the only implementation. The MIR
interpreter calls it directly, and the native runtime exports it as
`dr_v1_string_display_width`, which Cranelift and LLVM call. PHP output
rejects `displayWidth` with the other String intrinsic properties.

## Decided Without A Card

These follow from the ruling and from how terminals draw text; Andrew may
correct them.

- **Control characters take no cells.** Tab and newline are layout, and a
  terminal does not draw them as a glyph. A layout that expands tabs to the
  next stop does so itself.
- **Ambiguous characters take one cell.** This matches Windows Terminal,
  macOS Terminal and most Linux terminals in their default settings, which the
  ruling's option stated. A terminal configured to draw them wide shows those
  characters a cell wider than measured.
- **The base character decides.** A whole emoji sequence joined with U+200D,
  a flag, and a Hangul syllable written as separate jamo each take the width
  of their first drawn character. Terminals that draw a joined family as
  several emoji show it wider than measured.

## Explicit Non-Goals

- Wrapping, truncation to a width (PHP `mb_strimwidth`), and padding to a
  width. These are text-layout work and stay with the libraries that lay text
  out.
- A per-call ambiguous-width policy. It needs its own decision if a program
  for East Asian terminals requires it.
- Measuring for a font. Display width counts terminal cells only.

## Consequences

- `docs/stdlib-reference.md`, `SPEC.md` and Decision 0103's list of instance
  members name `displayWidth`.
- The compiler teaches `$text->displayWidth` for PHP's `mb_strwidth`, and the
  PHP string inventory row for `mb_strwidth` is accepted-current.
- The runtime ABI version moves from 17 to 18 because compiled programs call a
  new runtime function.
- The plan's approved sequence records display width as a dependency of the
  TUI library.

## Invalidated Elsewhere

- The website's String API reference and its guide on strings name the String
  properties; both gain `displayWidth`. This task does not modify the website
  repository.
- The language server's String hover lists the properties; it gains
  `displayWidth` with the compiler pin move.
