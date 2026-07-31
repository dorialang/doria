# Decision 0108: Human-first diagnostic model and presentation

Status: Accepted

## Context

The compiler historically represented a diagnostic as one code, one message,
one byte span, and at most one help item, fix, and related span. That shape was
adequate for early compiler bring-up but not for explaining ownership, type, and
cross-file failures. The CLI, language server, and website also interpreted that
small model independently, which allowed wording, capitalization, severity,
source ranges, and fix safety to drift.

Doria diagnostics are a public language interface. They must teach the language
in Doria vocabulary, remain usable by people without compiler expertise, and
carry enough stable structure for tools without requiring tools to parse
terminal prose.

## Decision

One compiler-owned diagnostic model is authoritative for every consumer. A
diagnostic carries:

- a stable code, severity, and kind;
- a short Title Case title and separate explanatory prose;
- any number of primary and secondary labels, each with a source identity, span,
  role, and local message;
- zero or more notes and help items;
- zero or more suggested fixes, each with an applicability classification and
  one or more source edits;
- optional causal identity, documentation metadata, and developer details.

The supported kinds are language error, unsupported development surface,
backend failure, external-tool failure, internal compiler error, and runtime
panic. Fix applicability is one of machine-applicable, requires review, or
informational. Consumers may offer one-click application only for the first
classification.

User-facing headings, prefixes, and titles use Title Case: `Error`, `Warning`,
`Note`, `Help`, `Related`, `Why`, `Suggested Fix`, `Caused By`,
`Internal Compiler Error`, `Panic`, and `Call Path`. Explanatory prose remains
ordinary sentence case. Diagnostic titles use plain Doria vocabulary; ownership
diagnostics say owns, gives away, readonly, writable, and still using rather
than exposing compiler implementation terminology.

The CLI has three stable presentations:

- `human`, the default, renders source context, all labels, explanations, help,
  and fixes to stderr;
- `concise` emits one line per diagnostic plus a summary to stderr;
- `json` emits a versioned, deterministic, ANSI-free envelope to stdout.

Color is independently selected with `auto`, `always`, or `never`. `auto`
requires an interactive stderr and honors `NO_COLOR`.

The renderer suppresses exact duplicate diagnostics by code, source, span,
title, and cause identity. Related failures retain causal identity so consumers
can group them without guessing from prose. Recovery diagnostics should be
suppressed when they add no new actionable information to the root failure.

Backend and external-tool failures expose a concise public summary while
retaining complete developer details in structured output. Internal failures use
a dedicated envelope with the toolchain version, build commit, source context,
and reporting guidance. Raw Rust panic text and backtraces are not ordinary
user-facing output.

Decision 0109 extends this same model with runtime-outcome details. Runtime
panic remains a distinct abort-only status-101 outcome, represented as
`DiagnosticKind::RuntimePanic`, with a precise source label, typed facts, and a
Doria `Call Path`. It is not a parallel report, catalogue, renderer, or JSON
schema. Interpreter, Cranelift, LLVM, PHP compatibility, standalone, CLI, LSP,
and Playground paths consume the same structured facts; no consumer parses
rendered prose.

## Alternatives considered

### Let each consumer format compiler messages

Rejected. It duplicates language policy and causes severity, range, fix, and
wording drift between the CLI, editors, and playground.

### Preserve terminal text as the tooling protocol

Rejected. Parsing human output prevents presentation improvements and loses
multi-file labels, fix applicability, causal identity, and stable source edits.

### Make JSON the only representation

Rejected. Structured transport is essential for tools, but humans should not
need an editor or JSON parser to understand a compile failure.

## Consequences

- The compiler catalogue and style guard own stable codes, default metadata,
  Title Case validation, and documentation slugs.
- Existing compiler passes may temporarily retain compatibility views of their
  original message, single help, fix, and related span while the structured
  fields remain authoritative at rendering and transport boundaries.
- The language server maps structured labels to a primary diagnostic plus
  related information and exposes only machine-applicable fixes as automatic
  code actions.
- The website playground renders the structured model as escaped, accessible
  diagnostic UI and never displays ANSI terminal output.
- Snapshot coverage includes Unicode, tabs, long and narrow source lines,
  multiple labels and files, every severity and fix applicability, summaries,
  backend limitations, and the internal-error envelope.

## Invalidated elsewhere

- Lowercase diagnostic headings or prefixes on any user-facing surface.
- Bare-array or unversioned diagnostic JSON.
- Consumer-side parsing of rendered terminal diagnostics.
- One-click editor or website fixes whose applicability requires review or is
  informational.
- Raw backend, linker, Rust panic, or backtrace output as the default diagnostic
  experience.
