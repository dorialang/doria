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
- **F4 — integer literals:** RESOLVED → add `0x`/`0o`/`0b` literals and `_` digit separators (`1_000_000`); **no** typed suffixes. Recorded in SPEC; a lexer slice still needs a stage assignment.
- **F5 — `uint8[]`↔`Bytes`:** RESOLVED → **explicit, non-implicit** conversion, copy in v1.0; method surface finalized with the collections decision (Stage 23).
- **F6 — property-hook I/O policy:** RESOLVED → a hook **may `throws`**, **may not block/async** in v1.0, and is **not guaranteed side-effect-free** ("looks like data" is a readability convention, not a purity guarantee). Recorded on the §12 property-hooks subject for the future record.
- **F7 — `Baton.lock` encoding:** RESOLVED → **JSON**.
- **F8 — `Console` vs `ScreenBuffer`:** RESOLVED → **stateless `Console`, no `ScreenBuffer` std type** (back-buffer renderers are userland).

## Read (sources consulted)

- `AGENTS.md` — blast-radius, two-clocks, verifying-claims, documentation-authority rules.
- `docs/doria-end-to-end-plan.md` — §0 process, the D1–D22 decision table, §3 ownership, §4 types, §5 errors, §6 OOP, §7 namespaces/closures, §8 architecture, §9 stdlib (incl. the "(… decision, unauthored)" markers, DDO, `Console`), §10 interop, §11 Baton, §12 decision-record catalogue, §13 stage roadmap.
- `SPEC.md` — literals (§ integer literals), control flow, arguments/defaults, panic, class syntax.
- `docs/decisions/` — spot-checked 0032 (`main` forms), 0082/§8.3 (native representation), 0086 (default args), 0095/0096 (operator/primitive surface), 0092–0097.

**Method:** skipped anything already settled in a record (cited where relevant) and anything explicitly deferred to a later stage *with a recorded reason* (that is a made decision — see "Recommended deferrals"). Focus is the residue: genuine forks left open, unrecorded, and decidable today.

## Already settled / correctly scheduled (not open — do not re-decide)

Most of the plan's "(… decision, unauthored)" markers are large features whose **design is sketched and stage is assigned**; they need a record authored, not a decision made: enums (Stage 27), `match` (28), checked errors (29), closures (30), namespaces (31), inheritance (34), interfaces/traits (35), FFI/unsafe (40), geometry-math (47), DDO (post-29), concurrency/async (Phase H). The versioning scheme (§11) is fully specified in-plan. The reflection stance (attributes decision) is decided in principle (compile-time derive = yes; dynamic reflection = no). These are **authoring tasks, not open questions**, and are out of scope for this audit.

## Open questions (answerable now)

Format per item: **Status · Options · Tradeoffs · Recommendation (marked) · Blast radius.**

### F2 — Interface-dispatch representation: the plan contradicts itself [OPEN · internal inconsistency]
- **Status.** §8.3 / record 0082 (line 780) states interface dispatch is **"committed to fat pointers"** — "recording it now prevents Stage 35 from reintroducing headers." But the Stage 35 roadmap entry (line 858) says interface-typed values are **"fat pointer or vtable-embedded — settle this in the interfaces/traits decision."** These disagree: one says decided, the other says open. (Same class of self-contradiction as the `when` Phase-E-vs-Stage-36 one already fixed.)
- **Options.** (a) Honor 0082: fat pointers, and correct the Stage 35 entry. (b) Reopen the representation at Stage 35.
- **Recommendation → (a).** 0082 deliberately committed to fat pointers to stop exactly this reopening; the Stage 35 "settle this" wording is stale. Reconcile the Stage 35 entry to cite 0082's commitment. Pure doc-consistency, no code impact now.
- **Blast radius.** Plan Stage 35 entry (line 858); the interfaces/traits decision when authored; cross-ref to 0082/§8.3.

### F3 — Named arguments: undesigned, unscheduled, yet load-bearing [OPEN]
- **Status.** SPEC defers named arguments to "a separate future slice" (three places: lines 716, 1039, 1043); 0095 defers spread/variadic "to the named-arguments slice." But named arguments are a **prerequisite** for: DDO's "named placeholders map onto named arguments" binding story; skipping *middle* optional arguments (0086 notes "required cannot follow optional until named arguments exist"); and attribute named arguments. Despite that, named arguments have **no stage and no design record** — the §0 "grammar/feature work is assigned, never implied" rule is being violated for a load-bearing feature.
- **Options.** (a) Author the named-arguments design (PHP 8 `f(name: value)` spelling; rules for mixing with positional; no silent reordering of evaluation) and assign it a stage before DDO. (b) Leave it implicit until DDO forces it.
- **Recommendation → (a).** Design it now (spelling is uncontroversial — PHP 8 `name:`), and schedule it explicitly ahead of its consumers rather than letting it surface late under DDO. This also unblocks the 0086 "omit a middle default" case.
- **Blast radius.** A new decision record + stage assignment; 0086 (default-arg ordering); 0095 (spread deferral target); the DDO prerequisites; SPEC argument sections.

### F4 — Non-decimal integer literals, digit separators, typed suffixes [OPEN]
- **Status.** SPEC (line 532) says only *"Stage 13 adds no numeric suffixes and no hexadecimal, octal, or binary literal syntax."* That is a *not-yet*, not a decision. Whether Doria will ever have `0xFF` / `0o755` / `0b1010`, digit separators (`1_000_000`), or typed suffixes (`100u8`) is unrecorded. For a systems language doing bitmask, hardware, engine, and byte work — and one that already ships `%x`/`%o`/`%b` *formatting* — decimal-only *input* literals are a real ergonomic hole.
- **Options.** (a) Add `0x`/`0o`/`0b` literals and `_` digit separators; no typed suffixes. (b) Also add typed suffixes (`100u8`). (c) Decimal-only, permanently.
- **Recommendation → (a).** Add hex/octal/binary literals and `_` separators; **skip typed suffixes** — Doria's contextual literal typing already assigns a literal's width from its expected type, so `100u8` is redundant with inference (and a suffix would be a second, competing typing channel). Decidable now; implements with lexer/numeric work.
- **Blast radius.** Lexer; the fixed-width-numerics decision (0016); SPEC literals section; the `fixed-width-integers` example (bit ops would read far better in hex/binary).

### F5 — `uint8[]` ↔ `Bytes` interconversion [OPEN · lands Stage 23]
- **Status.** Line 343: *"whether `uint8[]` and `Bytes` interconvert, and how, is decided in the collections decision."* Both are byte buffers; the relationship is undecided.
- **Recommendation → explicit, non-implicit, copy in v1.0.** An explicit `Bytes::fromArray(uint8[])` / `$bytes->toArray()` (copy), never an implicit coercion; zero-copy views over either belong to the FFI/unsafe tier (Stage 40), not here. Decide the surface with the collections decision (Stage 23).
- **Blast radius.** Collections decision; 0045 (`Bytes`); Stage 23.

### F6 — Property-hook capability policy: may a hook do I/O, `throws`, or block? [OPEN · lands Stage 36]
- **Status.** The property-hooks decision *"must decide"* this (line 744), designed against the ORM-shaped lazy-loaded-relation case — the one place a property-shaped API might need to hit the database on access.
- **Options.** (a) Hooks are pure/total (no I/O, no `throws`, no blocking) — keeps "a property is data"; lazy relations must be methods. (b) Hooks may `throws` and do I/O — enables property-shaped lazy relations (the ORM case) but a `$post->author` access can now fail or block, which fights the §6 "looks like data" charter. (c) Allow `throws` but not blocking/async in v1.0.
- **Recommendation → decide against the ORM case explicitly; lean (c).** Permit a hook to `throws` (so a lazy relation can surface a load failure through the checked-error path) while keeping the "looks like data" contract honest by documenting that a hooked property is not guaranteed side-effect-free. This is a genuine hard fork — flag it for a real decision, don't let Stage 36 default it silently.
- **Blast radius.** Property-hooks decision; §6 charter wording; Stage 36; DDO/ORM design cases.

### F7 — `Baton.lock` encoding: TOML vs JSON [OPEN · small · lands Stage 33]
- **Status.** Line 695: the lockfile's encoding *"stays open until the Baton manifest/resolver decision."* `Baton.toml` (human-edited) is TOML; the lock is machine-generated and never hand-edited.
- **Recommendation → JSON for the lock.** The human/machine split argues it: TOML earns its keep for the hand-edited manifest; the never-hand-edited lock wants the most ubiquitous, unambiguous machine format (JSON), avoiding TOML's edge cases in generated output. Decide with the Baton decision (Stage 33).
- **Blast radius.** Baton manifest/resolver decision; §11.

### F8 — `Console` statelessness vs a `ScreenBuffer` type [OPEN · lands Stage 46]
- **Status.** Line 588 flags this as something that *"must be decided explicitly, never bolted on silently"*: is `Console` stateless, or is there a separate `ScreenBuffer` (the TermUtil `Grid`/`charAt` read-back, half of a flicker-free diffing renderer)?
- **Recommendation → separate `ScreenBuffer` type; keep `Console` stateless.** A diffing renderer needs a readable back-buffer; bolting read-back onto a stateless capability facade muddies both. Best decided *with* the terminal API (Stage 46) rather than in the abstract, so lower priority — but the plan already commits to deciding it deliberately.
- **Blast radius.** Console/terminal decision; Stage 46; `Doria\Std\Term`.

## Minor / spec-tightening (lower priority)
- **`given` + chained `if`.** 0020 AND-s `given` predicates with *"the attached control condition"* (singular); 0097 generalized this to each `when`/`else when` and noted the `if`/`else if` mirror. 0020/SPEC's `if`-chain wording should be tightened to say the same (predicates AND with each `else if`), so the two constructs match on paper.
- **Collection method surface.** Line 632 sketches List/Dictionary/Set methods but says the surface "gets its own decision record." The names look settled (inventory is 0092); this is closer to an authoring task than an open fork — noted for completeness.

## Recommended deferrals (reason · reopen trigger)
- **F5** (`uint8[]`↔`Bytes`) → decide with the **collections decision (Stage 23)**; the recommendation above is the direction.
- **F6** (hook I/O policy) → decide with the **property-hooks decision (Stage 36)**; needs a real ruling, not a default.
- **F7** (lock encoding) → decide with the **Baton decision (Stage 33)**.
- **F8** (ScreenBuffer) → decide with the **terminal decision (Stage 46)**.
- Genuinely blocked / correctly parked (not audited): async/concurrency (Phase H), FFI zero-copy (Stage 40), generics value-parameters (kept-room extension point), `sscanf` (post-1.0), registry server (post-1.0), labeled break/continue, `goto`, `declare` keys.

## Invalidated elsewhere (if recommendations are adopted)
- **F2**: the Stage 35 plan entry (line 858) — reword to cite 0082's fat-pointer commitment; no code.
- **F3**: a new named-arguments record + stage; 0086 and 0095 cross-refs; DDO prerequisites.
- **F4**: lexer + 0016 + SPEC literals; the `fixed-width-integers` example.
- Nothing in this note edits the plan/SPEC/records — it is findings only. On approval, each item becomes a plan/SPEC amendment and/or a decision record (next free number, subject-cited until authored, `scripts/check_docs_authority.php` green).

## Proposed deliverable path
`docs/notes/plan-open-questions-audit.md` (this file), under "supporting context" per `docs/information-architecture.md`. Not a decision record — every item is a stop-and-ask for Andrew.
