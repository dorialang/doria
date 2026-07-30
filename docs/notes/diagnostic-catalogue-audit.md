# Diagnostic catalogue audit

Documentation role: implementation audit for Decision 0108.

The Diagnostic Experience Foundation mechanically inspected every compiler
construction and escape path with searches for `Diagnostic::`,
`unsupported_stage`, `with_help`, `with_related`, `backend_failure`, `eprintln!`,
`panic!`, and `unwrap(` across `crates/doriac/src` and its integration tests.
The source tree currently emits the catalogued `L`, `P`, `E`, `M`, `B`, and `I`
families. Their stable codes are enumerated by `CATALOGUED_CODES` in
`diagnostics.rs`; construction rejects an uncatalogued code in debug and test
builds.

## Audit totals

| Measure | Count |
| --- | ---: |
| Total codes audited | 173 |
| Codes with representative human-language upgrades in this beat | 7 |
| Codes intentionally retaining their established semantic detail through the compatibility projection | 166 |
| Codes currently constructed through the development-only path | 15 |
| Backend codes | 7 |
| Internal compiler codes | 10 |

The 15 development-only codes are `E0493`, `E0496`, `E0509`, `E0510`,
`E0513`, `E0521`, `E0523`, `E0524`, `E0525`, `E0528`, `E0533`, `E0534`,
`E0536`, `M1101`, and `M1102`. Development-only is a property of the emitted
diagnostic, not permanently of the code: a stable code can cease to use the
unsupported-development-surface constructor when its implementation lands.

## Closed foundation gaps

- The one-message/one-span model now projects into severity, kind, Title Case
  title, multi-source labels, explanation, repeated notes/help, multi-edit
  applicability-classified fixes, cause identity, documentation metadata, and
  developer details.
- Human, concise, and schema-version-1 JSON are produced by one compiler-owned
  renderer. Exact duplicates are suppressed before every presentation.
- Backend and internal families no longer render raw developer detail by
  default. JSON preserves it; internal diagnostics add toolchain/build identity.
- Unexpected characters, type mismatches, readonly binding/property writes, use after ownership
  transfer, unknown named arguments, and conflicting access now exercise the
  richer explanatory and secondary-label paths.
- Runtime `Panic` and `Stack Trace` headings are already Title Case in the
  interpreter, native runtime, and PHP compatibility emitter. Canonical panic
  messages remain unchanged because they are runtime behavior, not titles.

## Compatibility bridge

Compiler passes and existing semantic regression tests still read the historical
`message`, optional `help`, optional `fix`, and `related` views. Constructors
populate those views and the authoritative structured fields together. This
keeps the migration reviewable and prevents a mass call-site rewrite from
changing language behavior. New diagnostics should use explanations, labels,
and structured fixes directly.

## Follow-up audit rule

Every new code must be added to the central catalogue. Any new raw
backend/external/internal path must become a structured diagnostic before it
crosses a public boundary. `scripts/check_diagnostic_style.php` and Rust renderer
tests enforce the capitalization, schema, and envelope invariants.

## Known follow-up gaps

- The compatibility projection deliberately avoids rewriting all 167 unchanged
  messages in one review. Each future semantic change must migrate the touched
  diagnostic to explicit labels and explanations.
- Multi-file source text is not available until Stage 31. The model and JSON
  preserve cross-file identities now; terminal rendering shows the related path
  and byte offset when that source text is not loaded.
- The explicit consequence marker provides deterministic cause grouping without
  fuzzy text heuristics. Existing recovery paths continue to rely on the
  `Unknown` type to suppress secondary failures and should opt into cause
  identities as individual passes are migrated.
