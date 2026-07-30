# Doria diagnostic style

This guide is normative for compiler diagnostics and for every consumer of the
compiler's structured diagnostic model.

## Write for the person fixing the program

A diagnostic answers three questions in this order:

1. What went wrong?
2. Where did it happen, including the other location when two places conflict?
3. Why is it invalid and what can the programmer do next?

The title is a short, stable summary in Title Case. It is not a complete
sentence and has no trailing full stop. Put dynamic detail and reasoning in a
label or explanation, not in an ever-growing title.

Use Doria vocabulary. Say that a value was given away and is still being used;
say readonly, writable, owns, and ownership transfer. Do not teach users Rust
terms such as borrow checker, lifetime, move semantics, or mutable reference.
Development stages are planning vocabulary, not product vocabulary. A
temporarily unsupported accepted feature should say that it is not available
yet and may link to its landing documentation.

## Capitalization

All user-facing headings, prefixes, and titles use Title Case:

- `Error`, `Warning`, `Note`, `Help`, `Related`, and `Why`
- `Suggested Fix` and `Caused By`
- `Internal Compiler Error`
- runtime `Panic` and `Stack Trace`
- `Compilation Failed` and `Compilation Completed With Warnings`

Short source labels use Title Case (`Value Given Here`, `This Operation Needs
Writable Access`). Explanations, notes, and help use ordinary sentence-case
prose. Code, identifiers, and type spellings use backticks and retain their
exact case.

## Labels and source context

Give the actual failure a primary label. Add secondary labels for declarations,
earlier ownership transfers, previous access sites, conflicting types, and
causal locations. Every label carries its source identity; never encode a file
name or a byte range only in prose.

Prefer a small amount of relevant context. Rendering must remain correct for
UTF-8, combining/wide characters, tabs, empty lines, long lines, narrow
terminals, multi-line spans, and labels in another file. Byte offsets are the
compiler interchange coordinate; renderers and LSP adapters derive their own
display/UTF-16 positions from source text.

## Explanations, notes, and help

`Why` explains the language rule. `Note` adds relevant facts or provenance.
`Help` gives a next action. Do not repeat the title in all three. A diagnostic
may carry several notes and help items.

Unsupported-development diagnostics must distinguish valid-but-not-yet-covered
Doria from invalid language. Backend limitations explain that language checking
succeeded and that one output target could not preserve or emit the program.

## Suggested fixes

Every structured fix declares one applicability:

- `Machine Applicable`: exact edits are safe to apply without judgment.
- `Requires Review`: edits are plausible but may alter intent.
- `Informational`: an example or direction, not an automatic edit.

A fix may contain edits in several files. Editors and the website may expose an
automatic action only for `Machine Applicable`; the other classes remain visible
guidance.

## Output contracts

`--diagnostic-format human` and `concise` write to stderr. Human output provides
labels and explanations; concise output is one line per diagnostic. Both end
with a Title Case compilation summary.

`--diagnostic-format json` writes one schema-version-1 envelope to stdout. It is
deterministic, structured, and contains no ANSI. Consumers must use fields, not
parse rendered prose. Schema changes that remove or reinterpret a field require
a version increment.

`--diagnostic-color auto` colors only an interactive stderr and honors
`NO_COLOR`. `always` and `never` are explicit and testable.

## Failures outside ordinary language checking

Backend and external-tool errors have a concise public title and explanation;
complete tool output is retained as developer details in JSON and is shown in
human output only when diagnostic debugging is requested.

An internal compiler error uses the dedicated `Internal Compiler Error`
envelope, includes toolchain version and build commit, and asks for a report.
Raw Rust panic messages and backtraces are not the default interface.

Runtime panic is separate from compilation. Preserve each canonical panic
message and the exact Title Case `Panic` / `Stack Trace` envelope across all
backends.

## Review checklist

- The code is catalogued and its default severity/kind are correct.
- The title and every heading are Title Case.
- The primary span identifies the actionable token.
- Related source locations are structured secondary labels.
- The explanation teaches one Doria rule in plain language.
- Help is actionable and does not suggest unavailable syntax.
- Fix applicability is honest and all edits carry source identities.
- Recovery does not emit exact duplicates or cascades without new information.
- Human, concise, JSON, LSP, and playground consumers preserve the same facts.
