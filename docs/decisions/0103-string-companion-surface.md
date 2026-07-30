# Decision 0103: Canonical String API And Companion Boundary

**Status:** Accepted (amended by the Minimum String Runtime Surface)

## Context

The original record divided string operations among three public spellings:
intrinsic `$string->` members, a small `String::` transform seed, and a
PHP-familiar `str_*` predicate/search family. That division gave one primitive
type three API homes and made PHP's historical function catalogue, rather than
Doria's companion model, determine where operations lived.

Doria's primitive companions already provide the natural owner for
type-specific operations. Free functions remain useful where no single type is
the natural owner, but string-specific vocabulary does have one natural owner.

This amendment supersedes the earlier three-spelling boundary. Doria has not
been publicly released, so the old `str_*` spelling receives no alias,
compatibility shim, or deprecation period.

## Decision

The canonical boundary is:

```text
$string->   intrinsic data, measurements, and views
String::    every string-specific operation
free function
             capabilities that do not naturally belong to one type
```

There is one public spelling for each string operation.

### Intrinsic data and views

The instance surface contains facts and compiler-known views intrinsic to one
string value:

```doria
$text->length
$text->byteLength
$text->isEmpty
$text->bytes
$text->graphemes
$text->codePoints
```

These are properties or views, not action methods. The canonical instance
surface does not include `$text->trim()`, `$text->contains(...)`,
`$text->startsWith(...)`, or `$text->split(...)`.

A Doria `string` always contains valid UTF-8. Invalid UTF-8 remains in `Bytes`
until explicitly validated and converted.

- `length` is the number of Unicode extended grapheme clusters.
- `byteLength` is the exact number of bytes in the UTF-8 representation.
- `isEmpty` is equivalent to `byteLength == 0` and does not require traversal.
- `bytes` exposes the UTF-8 bytes under the accepted `Bytes` contract, which is
  a copy in v1.0 unless a later decision explicitly changes it.
- `graphemes` traverses Unicode extended grapheme clusters.
- `codePoints` traverses Unicode scalar values.

`chars` is not a canonical Doria member. The exact nameable types behind the two
text views remain scheduled with the traversal surface. Integer indexing on
`string` is not permitted.

### The `String` companion

Every accepted operation whose meaning is specifically about strings belongs
to the `String` companion. The families listed below establish the reviewed v1
inventory.

The [String API completeness audit](../notes/string-api-completeness-audit.md)
accounts for PHP's core string, mbstring, and grapheme capabilities, identifies
non-String owners, and presents the recommendations Andrew approved on
2026-07-31. This amendment accepts only the recommendations marked for v1 and
keeps every review/defer item named below rather than silently importing PHP's
catalogue.

#### Trimming and casing

```doria
String::trim($text)
String::trimStart($text)
String::trimEnd($text)
String::lower($text)
String::upper($text)
String::lowerFirst($text)
String::upperFirst($text)
```

Trimming uses Unicode whitespace. Casing is Unicode-aware,
locale-independent, and returns a new string without changing the original.
Runtime sharing optimizations may occur only when unobservable.
`lowerFirst` and `upperFirst` apply default Unicode casing to the first extended
grapheme cluster and leave the remainder unchanged. They do not normalize.

#### Predicates

```doria
String::contains($text, $needle)
String::startsWith($text, $prefix)
String::endsWith($text, $suffix)
String::containsIgnoreCase($text, $needle)
String::startsWithIgnoreCase($text, $prefix)
String::endsWithIgnoreCase($text, $suffix)
String::equalsIgnoreCase($left, $right)
```

`contains`, `startsWith`, and `endsWith` are case-sensitive.
`equalsIgnoreCase` uses Unicode case folding rather than ASCII-only lowercasing.
Case-insensitive behavior is explicit rather than selected by a boolean flag.
Matches must begin and end on extended grapheme-cluster boundaries. No operation
normalizes either operand. The empty needle is contained by every string, starts
and ends every string, has first index `0`, and has last index equal to the
string's grapheme count.

#### Search

```doria
String::indexOf($text, $needle)
String::lastIndexOf($text, $needle)
String::indexOfIgnoreCase($text, $needle)
String::lastIndexOfIgnoreCase($text, $needle)
String::countOccurrences($text, $needle)
```

The four index operations return `?int`; absence is `null`.
`countOccurrences` returns the number of non-overlapping left-to-right matches.
For an empty needle it returns the number of grapheme boundaries, which is
`$text->length + 1`. Indices use grapheme units, not UTF-8 byte offsets. A match
beginning or ending inside a grapheme is not a match. Ignore-case operations use
full default Unicode case folding without normalization and report positions in
the original text. Byte-level searching belongs to `Bytes`.

#### Replacement

```doria
String::replace($text, $search, $replacement)
```

Replacement is literal, not regular-expression based. It replaces all
non-overlapping occurrences from left to right. Regular expressions require a
separately designed API. Matches are grapheme-boundary aligned. An empty search
inserts the replacement at every grapheme boundary, including the beginning and
end; replacing an empty search in an empty string therefore returns one
replacement.

#### Splitting and joining

```doria
String::split(string $text, string $separator): List<string>
String::join(string $separator, List<string> $values): string
```

Splitting is literal, not regular-expression based. `join` may generalize to
`Iterable<string>` after the public iteration protocol lands without breaking
existing `List<string>` callers. A non-empty separator matches only at grapheme
boundaries and preserves empty fields. An empty separator splits into extended
grapheme clusters; splitting an empty string by an empty separator yields an
empty list.

#### Slicing

The single canonical name is `String::slice`; `substring` is not an alias.
Slicing operates in grapheme units. The planned callable shape is:

```doria
String::slice(
    string $text,
    int $start,
    ?int $length = null,
): string
```

A negative start counts backward from the grapheme count and clamps to zero; a
start beyond the end returns an empty string. An absent length returns the
remainder, zero returns an empty string, and a positive length returns at most
that many graphemes. A negative length panics with
`String Slice Length Cannot Be Negative`. This record does not invent a
nameable `Range` parameter.

#### Repetition and padding

```doria
String::repeat($text, $count)
String::padStart($text, $length, $padding)
String::padEnd($text, $length, $padding)
```

Padding targets use grapheme units. `repeat` returns empty for zero and panics
on a negative count with `String Repetition Count Cannot Be Negative`. Padding
returns the source when the target is no larger than its grapheme count.
Otherwise it repeats and grapheme-truncates the padding text to the exact
target. Negative targets panic with `String Padding Length Cannot Be Negative`;
empty padding panics with `String Padding Text Cannot Be Empty` only when
padding is required.

#### Explicit construction from bytes

```doria
String::fromBytes(Bytes $bytes): ?string
```

This returns a Doria string for valid UTF-8 and `null` for invalid UTF-8. It is
not lossy and never substitutes replacement characters. Any future lossy
conversion needs a distinct name and decision.

#### Ordering comparison

```doria
String::compare($left, $right)
String::compareIgnoreCase($left, $right)
```

These are reserved with planned return type `Ordering` and activate only when
that accepted surface is executable. Doria does not introduce an
integer-returning `str_case_compare` replacement.

### Free functions

Built-in free functions remain valid for capabilities without one natural
owning type, including I/O, formatting, environment, process, time,
cross-domain capabilities, and compiler meta operations:

```doria
read_line()
read_file($path)
write_file($path, $contents)
get_time()
function_exists("name")
```

This decision removes string-specific free functions. It does not remove free
functions as a language or standard-library category.

### Removed public spellings

The public Doria API has no `str_*` or `string_*` family and no instance-method
aliases:

```doria
str_starts_with($text, $prefix)    // not Doria
string_starts_with($text, $prefix) // not Doria
$text->startsWith($prefix)         // not Doria
$text->trim()                      // not Doria
```

The canonical forms are:

```doria
String::startsWith($text, $prefix)
String::trim($text)
String::split($text, ",")
String::join(",", $values)
String::slice($text, 1, 4)
```

The one-spelling rule applies to compiler diagnostics, documentation, examples,
generated code, migration guidance, language-server hovers, playground
examples, books, and website copy.

### PHP migration mapping

PHP source maps to Doria's companion surface:

| PHP input | Doria |
| --- | --- |
| `trim($text)` | `String::trim($text)` |
| `ltrim($text)` | `String::trimStart($text)` |
| `rtrim($text)` | `String::trimEnd($text)` |
| `strtolower($text)` | `String::lower($text)` |
| `strtoupper($text)` | `String::upper($text)` |
| `str_contains($text, $needle)` | `String::contains($text, $needle)` |
| `str_starts_with($text, $prefix)` | `String::startsWith($text, $prefix)` |
| `str_ends_with($text, $suffix)` | `String::endsWith($text, $suffix)` |
| `strpos($text, $needle)` | `String::indexOf($text, $needle)` |
| `strrpos($text, $needle)` | `String::lastIndexOf($text, $needle)` |
| `str_replace($a, $b, $text)` | `String::replace($text, $a, $b)` |
| `explode($separator, $text)` | `String::split($text, $separator)` |
| `implode($separator, $values)` | `String::join($separator, $values)` |
| `substr($text, $start, $length)` | `String::slice($text, $start, $length)` |
| `str_repeat($text, $count)` | `String::repeat($text, $count)` |
| `str_pad(..., STR_PAD_LEFT)` | `String::padStart(...)` |
| `str_pad(..., STR_PAD_RIGHT)` | `String::padEnd(...)` |
| `strcasecmp($left, $right)` | `String::compareIgnoreCase($left, $right)` |

This is migration guidance, not semantic inheritance. Tooling must identify
differences where Doria uses Unicode grapheme units and PHP uses byte-oriented
behavior.

## Alternatives considered

### Keep predicates and search as `str_*` free functions

Rejected. It preserves a PHP-derived split for one type and makes one-spelling
discipline harder to teach and enforce.

### Put all operations on string instances

Rejected. Doria's companion model already owns primitive-specific operations,
while the instance surface remains a compact set of intrinsic facts and views.

### Keep aliases during migration

Rejected. Doria has no released compatibility obligation, and aliases would
make the rejected split permanent.

## Consequences

- The prior three-spelling division in this record is superseded.
- `$string->` owns only intrinsic measurements and views.
- `String::` is the exclusive home for accepted string-specific vocabulary.
- The operation list in this record is the reviewed v1 inventory. The approved
  completeness additions are the symmetric ignore-case search family,
  `lowerFirst`, `upperFirst`, and `countOccurrences`.
- Case-insensitive replacement, grapheme reversal, range replacement,
  code-point construction/inspection, explicit normalization, title casing,
  custom trim characters, chunking, multi-replacement, natural comparison,
  edit-distance/similarity, segmentation, and display-width/wrapping remain
  named later furnishings under the owners and dependencies recorded by the
  audit.
- String-specific free functions and instance-method aliases are not Doria.
- PHP migration tooling rewrites PHP spellings to companion calls while warning
  when grapheme-based Doria semantics differ from byte-based PHP behavior.
- The executable subset includes the four intrinsic properties, every
  pre-audit companion operation above except the traversal views and
  `compare`/`compareIgnoreCase`, and the approved completeness additions.
- Interpreter and native execution share one pinned Unicode implementation;
  backends do not define String semantics.

## Invalidated elsewhere

- D19 examples that use string-specific free functions to illustrate built-in
  `snake_case`.
- The stdlib catalogue's `str_*` family.
- `$string->chars` and byte-length descriptions of `$string->length`.
- API guidance that assigns string predicates or search to free functions.
- Compiler, language-server, website, book, example, or migration guidance that
  publishes the old canonical spellings.
- Future runtime or tooling work planned around the former three-way split.
