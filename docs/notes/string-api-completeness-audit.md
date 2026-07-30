# String API Completeness Audit Against PHP

**Audit date:** 2026-07-30  
**PHP release current at audit:** 8.5.9  
**PHP manual copyright:** 2001-2026 The PHP Documentation Group  
**Status:** Implemented audit; recommendations require Andrew's review

## Purpose

Decision 0103 settles where Doria string vocabulary belongs, but its accepted
operation list was not produced from a complete comparative inventory. This
audit accounts for PHP's official core string, mbstring, and grapheme
catalogues without making PHP the source of Doria semantics.

The machine-readable row-by-row result is
[`php-string-capability-inventory.json`](php-string-capability-inventory.json).
Every row names its semantic unit, Doria owner or unresolved owning domain,
migration action, dependencies, and whether a designer decision remains.

This is a design checkpoint only. It accepts no proposed operation, changes no
compiler behavior, and blocks the Minimum String Runtime Surface until Andrew
reviews the proposed completeness amendment.

## Official Sources

- [PHP: String Functions - Manual](https://www.php.net/manual/en/ref.strings.php):
  105 entries.
- [PHP: Multibyte String Functions - Manual](https://www.php.net/manual/en/ref.mbstring.php):
  65 entries.
- [PHP: Grapheme Functions - Manual](https://www.php.net/manual/en/ref.intl.grapheme.php):
  10 entries in the table of contents.
- [PHP 8.5 Release Announcement](https://www.php.net/releases/8.5/en.php):
  adds `grapheme_levenshtein`, bringing the current released grapheme
  capability inventory to 11. The catalogue page had not yet listed it on the
  audit date.
- Focused boundary pages:
  [Normalizer](https://www.php.net/manual/en/class.normalizer.php),
  [Collator](https://www.php.net/manual/en/class.collator.php),
  [Transliterator](https://www.php.net/manual/en/class.transliterator.php),
  [IntlBreakIterator](https://www.php.net/manual/en/class.intlbreakiterator.php),
  [IntlChar](https://www.php.net/manual/en/class.intlchar.php), and
  [UConverter](https://www.php.net/manual/en/class.uconverter.php).

The inventory reflects the official English manual as fetched on the audit
date. User-contributed notes were not used as normative contracts. Relevant
version boundaries are recorded in the JSON manifest: PHP 8.3 added
`str_increment`/`str_decrement`; PHP 8.4 added the multibyte trim and
first-casing functions plus `grapheme_str_split`; PHP 8.5 added
`grapheme_levenshtein`.

## Coverage Result

- PHP core string rows: 105.
- PHP mbstring rows: 65.
- PHP grapheme rows: 11.
- Total function rows: 181.
- Duplicate rows: 0.
- Unclassified rows: 0.
- Deferred rows without an owner and named dependency: 0.

The offline guard at `scripts/check_string_api_completeness.php` verifies these
counts and names, exact-once coverage, required fields, the classification and
migration vocabularies, alias targets, deferred ownership, Intl boundary
reviews, and the plan's review block.

## Accepted Current Decision

Decision 0103 currently accepts this boundary:

- `$text->` owns intrinsic measurements and views: `length`, `byteLength`,
  `isEmpty`, `bytes`, `graphemes`, and `codePoints`.
- `String::` owns accepted string-specific operations: Unicode-whitespace trim,
  locale-independent lower/upper casing, case-sensitive predicates,
  `equalsIgnoreCase`, grapheme-indexed first/last search, literal replacement,
  split/join, grapheme slicing, repetition, grapheme padding, UTF-8 validation
  from `Bytes`, and lexical comparisons returning `Ordering` once that type is
  executable.
- Free functions own capabilities with no natural type owner, including I/O and
  formatting.
- Doria has no public `str_*` family, string-operation instance aliases,
  integer string indexing, or `$text->chars`.

This remains the accepted surface. It is a coherent seed, not a reviewed
exhaustive v1 inventory.

## Inventory Findings

### String-owned capability

The inventory found 35 rows that directly map to already accepted String
intrinsics/companions or existing output/formatting decisions. Thirty more rows
surface plausible additions or unresolved String-adjacent forks. The largest
gaps are case-insensitive search/replacement, first/title casing, occurrence
counting, grapheme reversal/chunking, range replacement, code-point
construction/inspection, custom trimming, normalization, and text analysis.

### Bytes and encoding

PHP uses `string` as both text and bytes. Doria does not. Byte construction,
hex conversion, byte-frequency analysis, byte cuts, and similar operations
belong to `Bytes`. Charset detection, conversion, MIME charset naming,
scrubbing, and legacy encodings belong to an explicit encoding domain operating
at the `Bytes`/UTF-8 boundary. Ordinary `String::` operations never take an
encoding parameter and never consult process-global encoding state.

### Domain ownership

- HTML entity handling and markup parsing belong to an HTML-aware domain;
  `strip_tags` is not a safe String transform.
- MIME headers, quoted-printable, and mail encoding belong to MIME/encoding.
- `parse_str`/`mb_parse_str` belong to URL/query handling, likely near
  `Doria\Std\Http`; the exact owner is unresolved.
- CSV parsing belongs to a CSV domain.
- Regex quoting, matching, splitting, and replacement belong to the future
  regex surface.
- Checksums, cryptographic hashes, password hashing, and file hashing belong to
  a hash/crypto domain.
- Random reordering belongs to `Doria\Std\Random`.
- Locale collation and locale-sensitive formatting require explicit locale
  objects; Doria does not inherit process-global locale.
- Display width, wrapping, Hebrew visual ordering, and line breaking belong to
  text layout, with terminal-specific width potentially integrating with
  `Doria\Std\Term`.
- Edit distance, similarity, phonetic keys, and word statistics belong to text
  analysis.

### Derivable and rejected convenience

Substring-returning search (`strstr`, `stristr`, `strrchr`, and grapheme/mb
variants) is derivable from first/last search plus `slice`; prefix-limited
comparison is derivable from `slice` plus comparison. Dedicated aliases add
surface without adding semantics.

`chop`, `join`, and `strchr` are explicit alias rows and map to their canonical
PHP rows. Doria rejects `print`, stateful `strtok`, alphanumeric
`str_increment`/`str_decrement`, PHP slash escaping as a generic text transform,
process-global encoding configuration, and deprecated legacy conversions.

## Focused Intl Boundary Review

### Normalization

PHP's Normalizer exposes NFC, NFD, NFKC, and NFKD plus normalized-state tests.
Decision 0103 correctly says ordinary operations do not normalize. That does
not settle explicit normalization. The owner remains a designer fork between
`String` and a future Unicode domain; this audit recommends a Unicode domain
because normalization forms and code-point metadata form one coherent service.

### Collation

PHP Collator is locale-sensitive and configurable by strength, case handling,
normalization, and numeric collation. It cannot define `String::compare`.
Doria's lexical comparison remains deterministic and locale-independent.
Locale-aware ordering belongs to a future collation/locale object.

### Transliteration

General transliteration is rule- and script-dependent. It belongs to a
specialized Unicode/text service, not the core companion.

### Break iteration

Grapheme boundaries are accepted core string semantics. Word, sentence, and
line boundaries require named text-segmentation policy and should not be hidden
inside `split`.

### Unicode metadata

Unicode category, direction, name, digit value, and property queries belong to
a future Unicode domain. `String` should not become a bag of scalar database
queries.

### Encoding conversion

UConverter confirms that charset conversion is a separate concern with explicit
source/target encodings and failure behavior. It belongs at the Bytes/text
boundary.

## Core String Gap Analysis

Each item below is a recommendation for review, not an accepted API.

### 1. Case-Insensitive Contains

- **Use cases / PHP precedent:** user-facing lookup; `stripos`, `mb_stripos`,
  and `grapheme_stripos`.
- **Unit / owner / candidate:** graphemes; `String`;
  `String::containsIgnoreCase(string $text, string $needle): bool`.
- **Alternatives:** derive from `indexOfIgnoreCase`; omit the predicate.
- **v1 / dependencies / performance:** recommend v1 only if the whole
  case-insensitive search family is accepted; Unicode full case folding and
  boundary-preserving search; avoid allocating folded copies where practical.
- **Migration / rationale / decision:** high migration value; explicit spelling
  avoids a boolean mode; designer decision required.

### 2. Case-Insensitive Prefix

- **Use cases / PHP precedent:** command and identifier prefixes; `stripos` at
  offset zero and mb/grapheme equivalents.
- **Unit / owner / candidate:** graphemes; `String`;
  `String::startsWithIgnoreCase(string $text, string $prefix): bool`.
- **Alternatives:** `indexOfIgnoreCase(...) == 0`.
- **v1 / dependencies / performance:** recommend with the family; full case
  folding; permit streaming prefix comparison.
- **Migration / rationale / decision:** useful direct rewrite with Unicode
  warning; designer decision required.

### 3. Case-Insensitive Suffix

- **Use cases / PHP precedent:** extensions and suffix conventions; no direct
  core PHP predicate, derivable from case-insensitive search.
- **Unit / owner / candidate:** graphemes; `String`;
  `String::endsWithIgnoreCase(string $text, string $suffix): bool`.
- **Alternatives:** last-index plus length.
- **v1 / dependencies / performance:** recommend with the family; full case
  folding and expansion-aware suffix matching.
- **Migration / rationale / decision:** closes family symmetry; designer
  decision required.

### 4. Case-Insensitive First Search

- **Use cases / PHP precedent:** search in human text; `stripos`,
  `mb_stripos`, `grapheme_stripos`.
- **Unit / owner / candidate:** graphemes; `String`;
  `String::indexOfIgnoreCase(string $text, string $needle): ?int`.
- **Alternatives:** a future search-options object.
- **v1 / dependencies / performance:** recommend v1; full case folding,
  grapheme-boundary result, no integer sentinel; allocation-free algorithms are
  desirable but not semantic.
- **Migration / rationale / decision:** strong migration value with byte/code
  point warning; designer decision required.

### 5. Case-Insensitive Last Search

- **Use cases / PHP precedent:** final human-text match; `strripos`,
  `mb_strripos`, `grapheme_strripos`.
- **Unit / owner / candidate:** graphemes; `String`;
  `String::lastIndexOfIgnoreCase(string $text, string $needle): ?int`.
- **Alternatives:** repeated first search.
- **v1 / dependencies / performance:** recommend with first search; same
  folding and boundary contract; avoid quadratic rescans.
- **Migration / rationale / decision:** completes search symmetry; designer
  decision required.

### 6. Case-Insensitive Replacement

- **Use cases / PHP precedent:** literal user-text rewrite; `str_ireplace`.
- **Unit / owner / candidate:** graphemes; `String`;
  `String::replaceIgnoreCase(string $text, string $search, string $replacement): string`.
- **Alternatives:** regex domain; folded search plus slice composition.
- **v1 / dependencies / performance:** recommend v1 only after expansion and
  replacement-boundary semantics are settled; Unicode folding; one-pass growth
  planning.
- **Migration / rationale / decision:** high migration value, but PHP's
  byte/array polymorphism needs human review; designer decision required.

### 7. Occurrence Counting

- **Use cases / PHP precedent:** validation and text statistics;
  `substr_count`, `mb_substr_count`.
- **Unit / owner / candidate:** graphemes; `String`;
  `String::countOccurrences(string $text, string $needle): int`.
- **Alternatives:** repeated `indexOf`; iterator matches after Stage 35.
- **v1 / dependencies / performance:** recommend v1; settle overlapping and
  empty-needle behavior; linear search preferred.
- **Migration / rationale / decision:** common and avoids error-prone loops;
  designer decision required.

### 8. First-Grapheme Casing

- **Use cases / PHP precedent:** labels and sentence starts; `lcfirst`,
  `ucfirst`, `mb_lcfirst`, `mb_ucfirst`.
- **Unit / owner / candidate:** first grapheme with Unicode casing; `String`;
  `String::lowerFirst(string $text): string` and
  `String::upperFirst(string $text): string`.
- **Alternatives:** slice plus casing plus concat.
- **v1 / dependencies / performance:** recommend v1; full casing may expand;
  empty input returns empty.
- **Migration / rationale / decision:** familiar and safely nameable; designer
  decision required.

### 9. Title Casing

- **Use cases / PHP precedent:** display labels; `ucwords`, `mb_convert_case`.
- **Unit / owner / candidate:** Unicode title casing over a word-boundary
  policy; `String` or text segmentation; `String::toTitleCase(string $text): string`.
- **Alternatives:** locale/text-layout service; caller-supplied word iteration.
- **v1 / dependencies / performance:** recommend defer until word-boundary and
  locale policy are explicit; Unicode segmentation/casing.
- **Migration / rationale / decision:** PHP precedent is not semantically
  sufficient; designer decision required.

### 10. Reverse

- **Use cases / PHP precedent:** presentation, algorithms, tests; `strrev`.
- **Unit / owner / candidate:** graphemes; `String`;
  `String::reverse(string $text): string`.
- **Alternatives:** reverse `$text->graphemes` after traversal lands.
- **v1 / dependencies / performance:** recommend v1 if convenience justifies
  allocation; grapheme segmentation; linear time and storage.
- **Migration / rationale / decision:** PHP byte reversal requires a semantic
  warning; designer decision required.

### 11. Fixed-Size Chunking

- **Use cases / PHP precedent:** pagination and batching; `str_split`,
  `mb_str_split`, `grapheme_str_split`.
- **Unit / owner / candidate:** graphemes; `String`;
  `String::chunks(string $text, int $size): List<string>`.
- **Alternatives:** `$text->graphemes` plus future iterator chunking.
- **v1 / dependencies / performance:** recommend defer until the traversal or
  iterator choice is reviewed; positive size, empty input returns empty list.
- **Migration / rationale / decision:** useful but eager allocation may be the
  wrong v1 shape; designer decision required.

### 12. Range Replacement

- **Use cases / PHP precedent:** editor-style text surgery; `substr_replace`.
- **Unit / owner / candidate:** graphemes; `String`;
  `String::replaceSlice(string $text, int $start, ?int $length, string $replacement): string`.
- **Alternatives:** `slice` plus concat; future `Range`.
- **v1 / dependencies / performance:** recommend v1 only if negative/range
  rules align with `slice`; no Range dependency required for the candidate.
- **Migration / rationale / decision:** avoids repeated segmentation in a
  common operation; designer decision required.

### 13. Multi-Pattern Replacement

- **Use cases / PHP precedent:** templating and transliteration; `strtr` map
  mode and array forms of `str_replace`.
- **Unit / owner / candidate:** graphemes/code points; `String` or a text
  transformation domain; `String::replaceMany` is only a candidate.
- **Alternatives:** ordered `List<Replacement>`; trie/rewrite object; repeated
  `replace`.
- **v1 / dependencies / performance:** recommend defer; must settle dictionary
  order, longest-match priority, overlap, recursion, empty keys, case, and
  boundary units; efficient implementation likely needs an automaton.
- **Migration / rationale / decision:** PHP behavior is substantial and cannot
  be pretended away; designer decision required.

### 14. Code-Point Construction

- **Use cases / PHP precedent:** protocol/text generation; `chr`, `mb_chr`.
- **Unit / owner / candidate:** Unicode scalar values; `String` or Unicode
  domain; `String::fromCodePoint(int $codePoint): ?string` is a candidate.
- **Alternatives:** panic on invalid scalar; `Unicode::fromCodePoint`.
- **v1 / dependencies / performance:** recommend review for v1; settle nullable
  versus panic and reject surrogate/out-of-range values.
- **Migration / rationale / decision:** core `chr` is byte-oriented while
  `mb_chr` is scalar-oriented; designer decision required.

### 15. Code-Point Inspection

- **Use cases / PHP precedent:** parsers and Unicode tooling; `ord`, `mb_ord`,
  `IntlChar`.
- **Unit / owner / candidate:** Unicode scalar values; Unicode domain preferred;
  `codePointAt(string $text, int $index): ?int` is a candidate.
- **Alternatives:** `$text->codePoints` traversal; first-code-point-only API.
- **v1 / dependencies / performance:** recommend defer to the Unicode/traversal
  design; code-point index is deliberately not the ordinary grapheme index.
- **Migration / rationale / decision:** avoids reviving ambiguous character
  vocabulary; designer decision required.

### 16. Custom Character Trimming

- **Use cases / PHP precedent:** stripping explicit delimiter sets; PHP
  `trim`/`mb_trim` character lists.
- **Unit / owner / candidate:** grapheme or code-point set; `String`;
  `trimCharacters`/`trimStartCharacters`/`trimEndCharacters` candidates.
- **Alternatives:** overload trim; predicate-based trim after closures; no core
  addition.
- **v1 / dependencies / performance:** recommend defer; requires a settled
  character-set representation and avoids ambiguous range syntax.
- **Migration / rationale / decision:** do not overload accepted
  Unicode-whitespace trim silently; designer decision required.

### 17. Natural Comparison

- **Use cases / PHP precedent:** filenames and human labels; `strnatcmp`,
  `strnatcasecmp`, numeric Collator mode.
- **Unit / owner / candidate:** locale/numeric token policy; collation or text
  analysis; canonical name unresolved.
- **Alternatives:** explicit Collator with numeric option; `NaturalOrdering`.
- **v1 / dependencies / performance:** recommend defer to collation; tokenization
  and locale policy required.
- **Migration / rationale / decision:** PHP's algorithm is not a universal
  lexical law; designer decision required.

### 18. Explicit Normalization

- **Use cases / PHP precedent:** stable storage, comparison, and protocol
  boundaries; Normalizer.
- **Unit / owner / candidate:** code points; Unicode domain preferred;
  `normalize` and `isNormalized` with a typed normalization-form enum.
- **Alternatives:** `String::normalize`; NFC-only operation.
- **v1 / dependencies / performance:** recommend v1 Unicode-domain design if
  canonical-equivalence-sensitive products need it; Unicode data tables and
  possible allocation.
- **Migration / rationale / decision:** ordinary operations remain
  non-normalizing; explicit capability is still valuable; designer decision
  required.

### 19. Substring Before And After

- **Use cases / PHP precedent:** path/header parsing; substring-returning
  `strstr`/`stristr` families.
- **Unit / owner / candidate:** graphemes; `String`; `before`/`after` names are
  possible but not recommended now.
- **Alternatives:** `indexOf` plus `slice`.
- **v1 / dependencies / performance:** recommend no new v1 API; derivable and a
  future match/range object can improve composition.
- **Migration / rationale / decision:** migration can rewrite with a semantic
  warning; no designer decision required unless convenience is desired.

### 20. Index Of Any

- **Use cases / PHP precedent:** delimiter scanning; `strpbrk`.
- **Unit / owner / candidate:** graphemes; `String` or text analysis;
  `String::indexOfAny(string $text, Set<string> $needles): ?int` is illustrative,
  not accepted.
- **Alternatives:** predicate/iterator search; code-point set.
- **v1 / dependencies / performance:** recommend defer; requires a settled
  character-set/grapheme-set parameter.
- **Migration / rationale / decision:** PHP byte masks do not map directly;
  designer decision required.

### 21. Initial Span Matching

- **Use cases / PHP precedent:** lexical scanning; `strspn`, `strcspn`.
- **Unit / owner / candidate:** graphemes or code points; String/text analysis;
  `span`/`spanNot` candidates.
- **Alternatives:** parser/lexer utility; predicate-based take-while.
- **v1 / dependencies / performance:** recommend defer until closures or a
  Unicode-set type; linear scan.
- **Migration / rationale / decision:** byte-mask PHP semantics are unsuitable;
  designer decision required.

### 22. Word Segmentation

- **Use cases / PHP precedent:** word counts, title casing, search;
  `str_word_count`, BreakIterator.
- **Unit / owner / candidate:** Unicode word boundaries; text-segmentation
  domain; exact type/module unresolved.
- **Alternatives:** locale-specific tokenizer or regex.
- **v1 / dependencies / performance:** recommend post-v1 unless required by
  title casing; Unicode break data and possibly locale tailoring.
- **Migration / rationale / decision:** PHP's byte/locale rules are not a
  Doria contract; designer decision required.

### 23. Sentence Segmentation

- **Use cases / PHP precedent:** document analysis; Intl BreakIterator.
- **Unit / owner / candidate:** Unicode sentence boundaries; text-segmentation
  domain; no candidate public spelling yet.
- **Alternatives:** specialized NLP library.
- **v1 / dependencies / performance:** recommend post-v1; Unicode break data.
- **Migration / rationale / decision:** no core PHP string equivalent, but Intl
  proves the domain boundary; designer decision required later.

### 24. Line Segmentation

- **Use cases / PHP precedent:** editors and layout; BreakIterator and
  `wordwrap`.
- **Unit / owner / candidate:** Unicode line boundaries; text-layout/
  segmentation domain; no candidate spelling yet.
- **Alternatives:** literal newline split for protocol-specific needs.
- **v1 / dependencies / performance:** recommend post-v1; Unicode line-break
  policy.
- **Migration / rationale / decision:** line opportunities are not the same as
  newline characters; designer decision required later.

### 25. Display Width

- **Use cases / PHP precedent:** terminal columns and aligned output;
  `mb_strwidth`, `mb_strimwidth`.
- **Unit / owner / candidate:** display columns; text layout, potentially
  integrated with `Doria\Std\Term`; no String member recommended.
- **Alternatives:** terminal renderer computes width under its capability model.
- **v1 / dependencies / performance:** recommend defer to terminal/text-layout
  design; Unicode width data plus environment/emoji policy.
- **Migration / rationale / decision:** ordinary String length must stay
  grapheme-based; designer decision required.

### 26. Word Wrapping

- **Use cases / PHP precedent:** terminal/doc layout; `wordwrap`.
- **Unit / owner / candidate:** display width plus line/word boundaries; text
  layout; no String member recommended.
- **Alternatives:** a layout/wrapper object with explicit width policy.
- **v1 / dependencies / performance:** recommend defer; depends on segmentation
  and display width.
- **Migration / rationale / decision:** PHP character counts are insufficient
  for Unicode display; designer decision required later.

### 27. Edit Distance

- **Use cases / PHP precedent:** suggestions and fuzzy matching; `levenshtein`,
  `grapheme_levenshtein`.
- **Unit / owner / candidate:** graphemes preferred; text analysis; canonical
  name and weighted-cost type unresolved.
- **Alternatives:** dedicated similarity package.
- **v1 / dependencies / performance:** recommend defer; quadratic time/memory
  contract and resource limits must be documented.
- **Migration / rationale / decision:** useful but not a basic String transform;
  designer decision required.

### 28. Similarity And Phonetic Keys

- **Use cases / PHP precedent:** fuzzy matching; `similar_text`, `metaphone`,
  `soundex`.
- **Unit / owner / candidate:** algorithm/language-specific; text analysis; no
  unified String spelling recommended.
- **Alternatives:** named algorithm types/functions in text analysis.
- **v1 / dependencies / performance:** recommend post-v1; specify language,
  normalization, complexity, and score type per algorithm.
- **Migration / rationale / decision:** these are not universal Unicode String
  semantics; designer decision required only when the text-analysis domain is
  designed.

## Proposed Completeness Amendment

### Recommended additions to String

Subject to Andrew's review:

- accept the symmetric case-insensitive search family:
  `containsIgnoreCase`, `startsWithIgnoreCase`, `endsWithIgnoreCase`,
  `indexOfIgnoreCase`, and `lastIndexOfIgnoreCase`;
- consider `replaceIgnoreCase`, `countOccurrences`, `lowerFirst`,
  `upperFirst`, grapheme `reverse`, and grapheme `replaceSlice` for v1;
- defer title casing, chunks, custom trimming, code-point APIs, natural
  comparison, and multi-replacement until their dependencies and units are
  settled.

### Recommended non-String owners

- `Bytes`: raw octet construction, hex, byte chunks/frequencies/cuts.
- Encoding domain, exact module unresolved: charset detection/conversion,
  scrubbing, legacy encodings.
- Unicode domain, exact module unresolved: normalization, scalar construction
  and metadata, specialized script transforms.
- Text analysis/layout/collation domains, exact modules unresolved: similarity,
  phonetics, segmentation, width, wrapping, natural and locale ordering.
- Existing/future domain modules: `Doria\Std\Random` for randomization;
  `Doria\Std\Http` or a URL/query owner for query parsing; formatting and I/O
  remain with their accepted intrinsics and `Doria\Std\Io` boundary.
- Regex, HTML, MIME, CSV, and hash/crypto require their own domain designs; no
  module names are invented by this audit.

### Recommended derivations and rejections

- Keep substring-returning search and prefix-limited comparison derivable.
- Keep aliases out of Doria.
- Reject stateful tokenization, PHP alphanumeric carry mutation, process-global
  encoding/locale state, generic slash escaping, and false-as-not-found.
- Route `strip_tags` to real HTML parsing rather than reproducing unsafe text
  stripping.

### Proposed final Decision 0103 outline

After designer review, Decision 0103 should:

1. preserve the accepted instance/companion/free-function boundary;
2. enumerate the accepted v1 String operation families;
3. name semantic units and exact empty/negative/failure behavior;
4. list deliberate derivations and permanent rejected aliases;
5. identify non-String domain owners without inventing unresolved module names;
6. leave post-v1 additions explicitly named rather than silently absent.

The accepted Decision 0103 list is not expanded by this audit.

## Migration Findings

- **Direct rewrites:** 18 rows.
- **Rewrites with semantic warnings:** 35 rows, chiefly byte/code-point offsets,
  PHP `false` results, custom trim lists, array polymorphism, and locale/global
  behavior.
- **Domain-module rewrites:** 59 rows.
- **Human-review rewrites:** 10 rows.
- **Unsupported until a named dependency lands:** 43 rows.
- **No Doria equivalent by design:** 12 rows.
- **Deprecated PHP input:** 4 rows.

Migration tooling must never rewrite a byte offset to a grapheme index without
a warning, convert PHP `false` to a Doria integer sentinel, or send users to an
unimplemented Doria method.

## Designer Review

| Decision                             | Recommendation                                              | Alternatives                            | Why It Matters                              | Runtime Impact                              | Migration Impact                         | Suggested v1 Status |
| ------------------------------------ | ----------------------------------------------------------- | --------------------------------------- | ------------------------------------------- | ------------------------------------------- | ---------------------------------------- | ------------------- |
| Case-insensitive search family       | Accept the five-name symmetric family                       | Only index methods; search options      | Common text lookup without mode flags       | Unicode fold and boundary-aware search      | High; byte/code-point warning            | Accept              |
| Case-insensitive replacement         | Review after search; accept only with explicit fold rules   | Regex; derive manually                  | Fold expansion affects replacement ranges   | Search plus allocation planning             | High; PHP arrays need review             | Review              |
| First-grapheme casing                | Accept `lowerFirst` and `upperFirst`                         | Derive from slice/case                  | Common, nameable Unicode operation           | Grapheme segmentation and full casing       | Medium                                   | Accept              |
| Title casing                         | Defer until word-boundary policy is explicit                | Locale/text service                     | "Word" is not a trivial delimiter            | Unicode word breaks and casing              | Medium                                   | Defer               |
| Custom trim characters               | Keep separate from whitespace trim; defer parameter design  | Overload; predicate-based trim; omit    | Prevents an ambiguous overload               | Character/grapheme-set matching             | Medium                                   | Defer               |
| Occurrence counting                  | Accept with non-overlap and empty-needle rules              | Repeated search; future iterator        | Common and avoids hand-written loops         | Linear boundary-aware search                | High                                     | Accept              |
| Reverse unit                         | Use graphemes                                               | Code points; expose traversal only      | Byte/code-point reversal corrupts user units | Segmentation plus one result allocation     | Medium; PHP warning                      | Review              |
| Chunking                             | Use graphemes if accepted; review eager `List` shape         | Iterator chunks; Bytes chunks           | Unit and ownership shape are public          | Segmentation and potentially many allocs    | Medium; PHP unit warning                 | Defer               |
| Range replacement                    | Align with `slice`; candidate `replaceSlice`                 | Slice plus concat; future Range         | Avoids duplicate range semantics             | Segmentation and one result allocation      | High                                     | Review              |
| Multi-replacement semantics          | Defer                                                       | Ordered rules; trie object; repeat calls | Ordering/overlap/empty-key rules are complex | Automaton/trie likely                       | High; no mechanical parity               | Defer               |
| Code-point construction/inspection   | Prefer a Unicode owner                                      | `String::fromCodePoint`/`codePointAt`   | Keeps scalar metadata out of String          | Unicode validation/traversal                | Medium; chr/ord differ                   | Review              |
| Normalization                        | Prefer an explicit Unicode owner with typed forms           | String methods; NFC-only                | Ordinary operations remain non-normalizing   | Unicode normalization data and allocation   | Medium                                   | Review              |
| Natural comparison                   | Put behind collation/text policy                             | String method; fixed algorithm          | "Natural" is policy, not lexical ordering    | Tokenization/collation                      | Medium                                   | Defer               |
| Edit distance and similarity         | Put in text analysis, grapheme-aware where applicable       | String methods; external package        | Complexity and units must be visible         | Potential quadratic resource use           | Medium                                   | Defer               |
| Word segmentation                    | Future text-segmentation domain                              | Regex; String view                      | Drives title case and analysis                | Unicode break data                          | Low                                      | Defer               |
| Display width and wrapping           | Text layout, with terminal policy integrated where needed   | String methods                          | Display cells are not grapheme length         | Width tables, environment and line breaking | High for TUI migrations                  | Defer               |

## Invalidated Elsewhere

- Decision 0103's statement that `String::` owns the complete string-specific
  vocabulary can still describe the ownership boundary, but cannot imply that
  its current list is an exhaustively reviewed v1 catalogue.
- `SPEC.md` and `docs/stdlib-reference.md` currently use wording that can be
  read as an exhaustive operation inventory; they require wording corrections.
- The master plan and current-pipeline note currently place the Minimum String
  Runtime Surface next; both must insert this audit and Andrew's completeness
  review, then mark runtime work blocked.
- The language server and website contain no proposed API only as a result of
  this audit; they must be searched for false exhaustiveness claims, but their
  pins and public surfaces must not advance.

## Next Required Action

Andrew reviews the designer-review table and either accepts, rejects, or defers
each Decision 0103 completeness recommendation. Only then may the Minimum
String Runtime Surface resume.
