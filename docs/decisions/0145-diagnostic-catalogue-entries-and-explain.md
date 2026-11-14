# Decision 0145: Diagnostic Catalogue Entries And `doriac explain`

- **Status:** Accepted
- **Date:** 2026-11-14
- **Owners:** Doria language and compiler design
- **Scope:** Per-code documentation of diagnostics, where it lives, how it is checked, and how people and tools read it

## Context

Decision 0108 gives every diagnostic a stable code and documentation metadata. Each diagnostic carried a link of the form `https://dorialang.org/docs/diagnostics/<code>`. The catalogue owned by `crates/doria-diagnostic-catalogue` held the list of codes and the runtime text for panics, and nothing held an explanation, a fix, or an example per code. The links led nowhere, and the website, the language server, and the command line each had nothing to show.

Andrew ruled the shape on 2026-11-14: the per-code text is owned by the compiler catalogue and exported; an entry documents the code as a whole; entries land by family; and the command is `doriac explain`.

## Decision

### One entry per code, owned by the catalogue

Each diagnostic code has one documentation entry: a title, an explanation, a fix, and a Doria program that raises the code. Entries are Markdown files in `crates/doria-diagnostic-catalogue/entries/`, one per code, named `<CODE>.md`. Adding the file registers the entry.

```text
# Title In Title Case

Title: fixed
Family: types

## Explanation

## Fix

## Example
```

The example is one fenced `doria` block. A code that no program raises on demand, such as an internal compiler error or a backend failure, has no program: its `## Example` section is a sentence that begins `No program raises this code on demand` and says why.

A code raised only in a project's test sources carries the header line `Example source: development`; its example is one test file and is checked as test source. A code reported only for a project of several files begins its `## Example` section `No single source file raises this code`, and a code only ever reported after another error begins it `No program shows this code on its own`. The compiler test lists the language codes allowed to go without a program, each with its reason; every other language, lexical, or syntax code has one.

### Families

Every entry names its family on a `Family:` line. The families, their titles, their one-sentence summaries, and their order are declared once, in `entries/FAMILIES.txt`, most common first. A consumer groups codes by the family the catalogue gives it and never derives a grouping from code numbers.

### A feature the compiler does not have yet

A code raised where the compiler does not yet have a feature must not read as a rejection of the feature. Andrew ruled on 2026-11-14: "If it is in our plan and just not implemented yet then it is a part of the full spec and should be treated as such."

- `Availability: planned` marks a code for a feature that is part of Doria for the first full release and that the compiler does not implement yet. The entry says the feature is part of Doria and that this compiler does not implement it yet.
- `Availability: later` marks a code for a feature that Doria reserves for a release after the first full one. The entry says so.

An entry without the line describes a rule of the language. The compiler test enforces the wording of the other two.

### Retired codes

A code that no compiler raises any more is retired. It moves from the list of codes in use to `RETIRED_DIAGNOSTIC_CODES`. A retired code is never raised and its number is never given a new meaning, so an old message, note, or test that names it cannot come to mean something else. It has no entry. `doriac explain` says that the code is retired, and the export lists retired codes separately. Forty-two codes were retired with this decision; each was confirmed to have no place in the compiler that raises it.

### An entry documents the code

An entry describes the code as a whole. The compiler keeps the specific message, labels, and help it writes at each place it raises the code. Many codes are raised with a title built from the program, and some under several titles; one entry covers them all.

- `Title: fixed` states that the compiler raises the code under exactly the entry's title.
- `Title: varies` states that the compiler's title depends on the program, and the entry's title is the catalogue's name for the code.

### Every example is proven

A compiler test checks every entry:

- the file is in the required shape;
- for a language or syntax code, its example raises the entry's code, and that code is the first diagnostic reported;
- for a runtime panic, its example compiles cleanly and, run in the interpreter, ends with that panic under the entry's title;
- a language or syntax code always has an example program; only a code that no source program raises may carry the sentence in its place;
- a `fixed` title equals the title the compiler raises, and a `varies` title differs from it.

### Coverage is tracked, and new codes arrive with an entry

`entries/WITHOUT_ENTRY.txt` lists the catalogued codes that have no entry yet. The same test requires that list to equal the codes without an entry file. A code leaves the list in the change that adds its entry. A code added to the catalogue after this decision is not on the list, so it must arrive with its entry.

Entries land by family, most common first: names, types, nullability, and collections, then ownership, then runtime panics, then the rest.

### `doriac explain`

```text
doriac explain <code> [--format human|json]
doriac explain --all [--format human|json]
```

- `doriac explain E0782` prints the entry: code and title, `Why`, `Fix`, `Example`, and the documentation link. The code may be typed in either case.
- `doriac explain --all` lists the codes that have an entry, with their titles.
- `--format json` writes a versioned document. For `--all` it is the export consumers pin:

```json
{
  "schemaVersion": 1,
  "toolchainVersion": "...",
  "compilerCommit": "...",
  "documentationRelease": "v2026.3.1",
  "families": [
    { "id": "", "title": "", "summary": "", "codes": [] }
  ],
  "entries": [
    { "code": "", "title": "", "titleIsFixed": true, "family": "", "availability": "current", "explanation": "", "fix": "", "example": "", "exampleNote": null, "exampleSource": "main", "url": "" }
  ],
  "codesWithoutEntry": [],
  "retiredCodes": []
}
```

Exactly one of `example` and `exampleNote` is a string; the other is `null`. `families` is in catalogue order and each family's `codes` are in code order; every entry appears under exactly one family. `availability` is `current`, `planned`, or `later`. `exampleSource` is `main` or `development`.

A code that is not catalogued, and a catalogued code without an entry, are errors that say which it is and name `doriac explain --all`.

### The documentation address is versioned

A diagnostic's link, and an entry's `url`, is

```text
https://docs.dorialang.org/<release>/diagnostics/<code>
```

`<release>` is the documentation release of the compiler that printed it: `v`, then the toolchain's year, quarter, and release number without zero padding and without a channel suffix. Toolchain `2026.03.1-canary` links to `v2026.3.1`. `<code>` is the code in lowercase. The site keeps one set of diagnostic pages per release, as it does for the guides and the API reference, so each compiler links to the explanations for its own release. The export carries the release as `documentationRelease`.

Andrew ruled this on 2026-11-14 ("Versioned on docs host"). It replaces the unversioned `dorialang.org/docs/diagnostics/<code>` form that Decision 0108's metadata first used.

## Implementation Contract

- The entry text is compiled into the catalogue crate behind its `explanations` feature. The compiler enables the feature. The runtime does not, so no entry text is carried into a compiled program.
- The catalogue's build script lists the entry files; no second registry names them.
- The documentation link is built by one function, used by diagnostics and by the export.
- Consumers read the export at a pinned compiler commit. They do not restate entry text. The website builds one page per entry and checks it against the export, as it does for the API reference; the language server shows the entry for a diagnostic's code.

## Explicit Non-Goals

- Replacing the messages the compiler prints with catalogue text. Emission sites keep their own wording.
- A Baton command. `baton explain` would be its own decision.
- Translations of entry text.

## Consequences

- `doriac --help` lists `explain`.
- `docs/diagnostic-style.md` governs entry prose as it governs diagnostic prose.
- Until a family's batch lands, its codes link to pages that do not exist yet; `codesWithoutEntry` tells consumers which.

## Invalidated Elsewhere

- The website has no diagnostics pages. It builds them from `doriac explain --all --format json` at its pinned commit, one per entry, with a drift test. This task does not modify the website repository.
- The language server can show an entry beside a diagnostic and can offer the link only for codes that have an entry. This task does not modify the language-server repository.
