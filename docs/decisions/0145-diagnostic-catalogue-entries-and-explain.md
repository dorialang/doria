# Decision 0145: Diagnostic Catalogue Entries And `doriac explain`

- **Status:** Accepted
- **Date:** 2026-11-14
- **Owners:** Doria language and compiler design
- **Scope:** Per-code documentation of diagnostics, where it lives, how it is checked, and how people and tools read it

## Context

Decision 0108 gives every diagnostic a stable code and documentation metadata. Each diagnostic carries a link of the form `https://dorialang.org/docs/diagnostics/<code>`. The catalogue owned by `crates/doria-diagnostic-catalogue` held the list of codes and the runtime text for panics, and nothing held an explanation, a fix, or an example per code. The links led nowhere, and the website, the language server, and the command line each had nothing to show.

Andrew ruled the shape on 2026-11-14: the per-code text is owned by the compiler catalogue and exported; an entry documents the code as a whole; entries land by family; and the command is `doriac explain`.

## Decision

### One entry per code, owned by the catalogue

Each diagnostic code has one documentation entry: a title, an explanation, a fix, and a Doria program that raises the code. Entries are Markdown files in `crates/doria-diagnostic-catalogue/entries/`, one per code, named `<CODE>.md`. Adding the file registers the entry.

```text
# Title In Title Case

Title: fixed

## Explanation

## Fix

## Example
```

The example is one fenced `doria` block.

### An entry documents the code

An entry describes the code as a whole. The compiler keeps the specific message, labels, and help it writes at each place it raises the code. Many codes are raised with a title built from the program, and some under several titles; one entry covers them all.

- `Title: fixed` states that the compiler raises the code under exactly the entry's title.
- `Title: varies` states that the compiler's title depends on the program, and the entry's title is the catalogue's name for the code.

### Every example is proven

A compiler test checks every entry:

- the file is in the required shape;
- its example raises the entry's code, and that code is the first diagnostic reported;
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
  "entries": [
    { "code": "", "title": "", "titleIsFixed": true, "explanation": "", "fix": "", "example": "", "url": "" }
  ],
  "codesWithoutEntry": []
}
```

A code that is not catalogued, and a catalogued code without an entry, are errors that say which it is and name `doriac explain --all`.

## Implementation Contract

- The entry text is compiled into the catalogue crate behind its `explanations` feature. The compiler enables the feature. The runtime does not, so no entry text is carried into a compiled program.
- The catalogue's build script lists the entry files; no second registry names them.
- The documentation link is built by one function, used by diagnostics and by the export.
- Consumers read the export at a pinned compiler commit. They do not restate entry text. The website builds one page per entry and checks it against the export, as it does for the API reference; the language server shows the entry for a diagnostic's code.

## Explicit Non-Goals

- Replacing the messages the compiler prints with catalogue text. Emission sites keep their own wording.
- Runtime panic examples in the first batch. A panic's example must be run, not only checked; the test gains that step with the panic family.
- A Baton command. `baton explain` would be its own decision.
- Translations of entry text.

## Consequences

- `doriac --help` lists `explain`.
- `docs/diagnostic-style.md` governs entry prose as it governs diagnostic prose.
- Until a family's batch lands, its codes link to pages that do not exist yet; `codesWithoutEntry` tells consumers which.

## Invalidated Elsewhere

- The website has no diagnostics pages. It builds them from `doriac explain --all --format json` at its pinned commit, one per entry, with a drift test. This task does not modify the website repository.
- The language server can show an entry beside a diagnostic and can offer the link only for codes that have an entry. This task does not modify the language-server repository.
