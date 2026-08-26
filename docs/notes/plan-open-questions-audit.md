# End-to-end plan — open-questions audit

> Documentation role: working note / findings for Andrew's decision. A
> design-completeness sweep of `docs/doria-end-to-end-plan.md` (plus SPEC and the
> decision records it cites) for open questions that are **answerable now** — not
> blocked on a future stage's implementation. Same stop-and-ask style as
> `docs/notes/io-surface-audit.md`: nothing here is decided; each open item gets
> options, tradeoffs, a marked recommendation, and blast radius. First-pass sweep
> — thorough but not claiming exhaustiveness.

## Resolutions (Andrew's decisions, 2026-07-22)

- **F1 — CLI args:** RESOLVED → optional **`main(List<string> $args)`** (no `argc`; `$args->count`); `Doria\Std\Process` owns the other process facts; `Console` rejected as a home. **Decision 0099** authored; depends on `List` (Stage 23).
- **F2 — interface dispatch:** RESOLVED → **fat pointers** (per §8.3 / 0082). Stage 35 plan entry reconciled.
- **F3 — named arguments:** RESOLVED → scheduled **Stage 23a** (after collections, before generic functions); **decision 0098** authored with the full binding/ordering/evaluation ruleset; variadics stay deferred.
- **F4 — integer literals:** RESOLVED direction → add `0x`/`0o`/`0b` literals and `_` digit separators (`1_000_000`); **no** typed suffixes. SPEC records this as future direction, not current syntax. A dedicated numeric-literals slice must settle separator placement and malformed-form diagnostics before promoting the forms into the accepted grammar.
- **F5 — `uint8[]`↔`Bytes`:** RESOLVED → **explicit, non-implicit** conversion, copy in v1.0; method surface finalized with the collections decision (Stage 23).
- **F6 — property-hook I/O policy:** RESOLVED → a hook **may `throws`**, **may not block/async** in v1.0, and is **not guaranteed side-effect-free** ("looks like data" is a readability convention, not a purity guarantee). Recorded on the §12 property-hooks subject for the future record.
- **F7 — `Baton.lock` encoding:** RESOLVED → **deterministic JSON**, now
  formalized by Decision 0118 together with the complete source-neutral lock,
  resolver, workspace, cache, and offline contract.
- **F8 — `Console` vs `ScreenBuffer`:** RESOLVED → **stateless `Console`, no `ScreenBuffer` std type** (back-buffer renderers are userland).

## Read (sources consulted)

- `AGENTS.md` — blast-radius, two-clocks, verifying-claims, documentation-authority rules.
- `docs/doria-end-to-end-plan.md` — §0 process, the D1–D22 decision table, §3 ownership, §4 types, §5 errors, §6 OOP, §7 namespaces/closures, §8 architecture, §9 stdlib (incl. the "(… decision, unauthored)" markers, DDO, `Console`), §10 interop, §11 Baton, §12 decision-record catalogue, §13 stage roadmap.
- `SPEC.md` — literals (§ integer literals), control flow, arguments/defaults, panic, class syntax.
- `docs/decisions/` — spot-checked 0032 (`main` forms), 0082/§8.3 (native representation), 0086 (default args), 0095/0096 (operator/primitive surface), 0092–0097.

**Method:** skipped anything already settled in a record (cited where relevant) and anything explicitly deferred to a later stage *with a recorded reason* (that is a made decision — see "Recommended deferrals"). Focus is the residue: genuine forks left open, unrecorded, and decidable today.

## Already settled / correctly scheduled (not open — do not re-decide)

Most of the plan's "(… decision, unauthored)" markers are large features whose **design is sketched and stage is assigned**; they need a record authored, not a decision made: inheritance (34), interfaces/traits (35), FFI/unsafe (40), geometry-math (47), DDO (post-29), concurrency/async (Phase H). Closures are no longer in that list: decision 0120 requires explicit `with` capture lists for arrows and anonymous block functions; the pre-Stage-30 grammar slice is complete and owns accepted lexer/parser/AST syntax under the two-clocks rule, while Stage 30 still owns `$this`, mode validation, callable-effect inference, environment ABI, HIR/MIR, and execution. Checked errors are complete under decision 0119 across all three Stage 29 slices. Namespace/package-graph authority and Baton's manifest/resolver authority are settled by decisions 0117 and 0118. Enums and complete core match are settled by decisions 0114 and 0115. The versioning scheme (§11) is fully specified in-plan. The reflection stance (attributes decision) is decided in principle (compile-time derive = yes; dynamic reflection = no). These are **authoring tasks, not open questions**, and are out of scope for this audit.

## Open questions (answerable now)

No unresolved items remain from this audit. F1-F8 are archived in the resolutions
above; their accepted decisions and scheduled work are the authority.

## Minor / spec-tightening (lower priority)
- **`given` + chained `if`: RESOLVED.** Decision 0116 defines one gate for the complete chain. A failed predicate skips every `if`/`else if` condition and selects only the unconditional `else`, when present. Decision 0020 and SPEC now use the same rule.
- **Collection method surface.** Line 632 sketches List/Dictionary/Set methods but says the surface "gets its own decision record." The names look settled (inventory is 0092); this is closer to an authoring task than an open fork — noted for completeness.
- **Closure-based collection algorithms.** Decision 0100 grants `map`, `filter`,
  and `reduce` to `List<T>` at Stage 30. No accepted authority grants those
  methods to `Dictionary`, `SortedDictionary`, `Set`, `SortedSet`, `Deque`,
  `T[]`, `Iterable`, or a shared algorithm interface. Their result shapes,
  traversal contracts, callback ownership, and any dictionary-entry shape remain
  unresolved; decision 0120 deliberately adds no public method to those types.

## Recommended deferrals (reason · reopen trigger)
- **F5** (`uint8[]`↔`Bytes`) → decide with the **collections decision (Stage 23)**; the recommendation above is the direction.
- **F6** (hook I/O policy) → decide with the **property-hooks decision (Stage 36)**; needs a real ruling, not a default.
- **F7** (lock encoding) → resolved by **Decision 0118**; no longer deferred.
- **F8** (ScreenBuffer) → decide with the **terminal decision (Stage 46)**.
- Genuinely blocked / correctly parked (not audited): async/concurrency (Phase H), FFI zero-copy (Stage 40), generics value-parameters (kept-room extension point), `sscanf` (post-1.0), registry server (post-1.0), labeled break/continue, `goto`, `declare` keys.

## Invalidated elsewhere (if recommendations are adopted)
- **F2**: the Stage 35 plan entry (line 858) — reword to cite 0082's fat-pointer commitment; no code.
- **F3**: a new named-arguments record + stage; 0086 and 0095 cross-refs; DDO prerequisites.
- **F4**: lexer + 0016 + SPEC literals; the `fixed-width-integers` example.
- Nothing in this note edits the plan/SPEC/records — it is findings only. On approval, each item becomes a plan/SPEC amendment and/or a decision record (next free number, subject-cited until authored, `scripts/check_docs_authority.php` green).

## Stage 31 Authority Prerequisite

Resolved. Decision 0117 accepts the public `autoload` vocabulary, compile-time
discovery, namespace-prefix mappings, main/development/generated scopes, hybrid
strict layout, deterministic package compilation graphs, incremental ownership,
and top-level entry rules. Decision 0118 accepts schema 2 and the dependency,
lockfile, workspace, cache, and offline model. This authority does not begin
implementation by itself. Both Stage 31 slices are complete: namespace/import
syntax, canonical resolution, the edition prelude, compiler context, build-plan
package graphs, include resolution, package visibility, and multi-source
lowering are implemented. Stage 32 is next; Stage 33 remains scheduled in three
slices. Stage 30 is complete and E0641 remains historical and reserved.

## Proposed deliverable path
`docs/notes/plan-open-questions-audit.md` (this file), under "supporting context" per `docs/information-architecture.md`. Not a decision record — every item is a stop-and-ask for Andrew.
