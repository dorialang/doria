# Decision 0147: The String Graphemes View

- **Status:** Accepted
- **Date:** 2026-11-15
- **Owners:** Doria language and standard library design
- **Scope:** What `$text->graphemes` is, how it is represented, and how the compiler keeps its implementation out of reach of programs

## Context

Decision 0103 accepted `$text->graphemes`, a traversal of a string's Unicode
extended grapheme clusters, and left its nameable type to be settled with the
traversal surface. The compiler rejected it with E0304 until the public
iteration protocol existed. Decision 0134 has since made `Iterable<T>` and
`Iterator<T>` executable.

The `Doria\Tui` library must walk text one grapheme cluster at a time to place
it in terminal cells. Without the view the only walk was `String::slice` once
per position, which segments the string again for every cluster.

## Decision

Andrew ruled on 2026-11-15 ("A view that is an Iterable<string>
(Recommended)"):

```doria
foreach ($text->graphemes as string $grapheme) {
    echo $grapheme;
}
```

- `$text->graphemes` is a readonly property whose type is `Iterable<string>`.
  No new type name is added.
- It yields each extended grapheme cluster of the string, in order, as a
  `string`. The clusters are exactly the ones `$text->length` counts and
  `$text->displayWidth` measures, under the same Unicode data.
- Nothing is segmented or copied when the view is made. The cursor finds the
  next cluster boundary as it advances, and each cluster's text is made when
  the cursor reaches it.
- PHP output rejects it with B2501, as it rejects the other String
  properties.

## Representation

The view is written in Doria in a compiler-known module
(`crates/doriac/src/compiler_known_text.rs`): a view class that implements
`Iterable<string>` and a cursor class that implements `Iterator<string>`.
The cursor keeps two byte offsets into the string. Two compiler-private
primitives do the Unicode work: the end of the cluster that starts at a byte
offset, and the text between two cluster boundaries. Both are MIR String
intrinsics backed by `doria_unicode::grapheme_boundary_after` and
`doria_unicode::text_between`; the interpreter calls those functions and the
native runtime exports them as `dr_v1_string_grapheme_boundary_after` and
`dr_v2_string_text_between` (runtime ABI 19).

The view and the cursor hold the string itself. A string is Copy and
immutable, so holding it shares its buffer, copies no text and needs no
borrow: the view stays valid after the variable it came from is reassigned.

`$text->graphemes` lowers to a call of the view's compiler-known factory with
the string as its argument.

### Compiler-private declarations

A compiler-known type is reachable from authored source only through the
public canonical-type list of its module. The view and the cursor are in no
such list, so no authored name resolves to them; a program that writes
`Doria\Std\Text\GraphemeView` gets an unresolved-name error. This is the
general rule for compiler-known declarations that are implementation detail.

## Decided Without A Card

- **The view holds the string by value.** The ruling's option said the view
  "borrows the string, like `$text->byteRegion`". Because strings are Copy and
  immutable, holding the string gives the same guarantees with no borrow, and
  the view can outlive the expression that made it.
- **`$text?->graphemes` is not added.** No String property accepts a null-safe
  receiver today (E0507); graphemes follows the other properties.
- **`codePoints` stays pending.** Its element type is not settled; E0304 now
  says it is part of Doria and not implemented by this compiler yet.

## Explicit Non-Goals

- Indexing, counting or reversing through the view. `$text->length` counts.
- The `codePoints` view.

## Consequences

- `docs/stdlib-reference.md`, `SPEC.md` and Decision 0103 describe the view as
  executable.
- The String surface table (`doriac::string_surface`) lists `graphemes` as an
  executable property of type `Iterable<string>`.
- Interface slots use the same automatic-effect transport as structural calls,
  so they need the `Doria\Std\Io` identities even when nothing performs I/O.
  The compiler decides those identities once more after it adds the
  compiler-known modules, over the program it will check, so a program that
  walks graphemes and does no I/O compiles. Every compilation path adds the
  compiler-known modules through one function, `augment_compiler_known`.

## Invalidated Elsewhere

- The website's String API reference and strings guide gain the executable
  view. This task does not modify the website repository.
- The language server's String hover and completion read the compiler's
  String surface table after its pin moves.
