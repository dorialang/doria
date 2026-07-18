# Session handoff — Doria plan work (claude.ai → Claude Code)

**Purpose.** This is the state a chat session held that the repository does not. It exists to be read by the next agent session (Claude Code, Codex) and by Andrew. Nothing here is a decision unless it says so; several items are explicitly flagged as *unverified assertions* the next session must check rather than inherit.

**Scope note.** The previous session could not read the repository. Every failure it recorded traces to that: it reasoned about record numbers from a list it had written itself, never saw `docs/decisions/`, could not run `scripts/check_docs_authority.php`, and could not see the website repo where two of the last three design gaps actually lived. A session with filesystem access should verify, not trust, anything below marked ⚠.

---

## 1. Open decisions awaiting Andrew

### 1.1 DDO vs record 0007 — RESOLVED (PR #85)

Superseded, not reconciled. The plan's §9 DDO charter is authoritative; `docs/decisions/0007-ddo-database-abstraction.md` is marked **Superseded**; the modern DDO decision takes a fresh record number when DDO is scheduled (post-Stage-29). Outcomes of the three former conflicts and two open questions:

- **DSN vs typed config:** §9's typed connection configuration stands; 0007's `new DDO("mysql://...")` DSN is dropped. A DSN-as-additional-path is deferred to DDO authoring.
- **God object vs decomposed API:** §9's decomposed API (`Connection`/`Statement`/`Transaction`) stands.
- **Streaming result sets:** carried into the §9 charter (a lazily-streamed typed-row cursor for large results).
- **`DDO` vs `Ddo`:** moot — `DDO` is the layer/brand name, not a class.
- **`foreach ($users as UserRow $user)`:** a pre-SPEC sketch in 0007, not adopted.

### 1.2 Record 0006 (Console) — two tightenings to record as amendments

`docs/decisions/0006-console-and-terminal-applications.md` is a faithful ancestor of the plan's §9 Console design (it independently says "termutil is useful inspiration, but Doria should not copy PHP implementation details blindly"). Two deltas the plan introduced without recording them as amendments:

- 0006 says *"bridge Windows console behavior **where practical**"*. The plan says Windows is tier-1, both backends land together, no escape sequences in any public type. Deliberately stronger — record it.
- 0006 names **exclusive terminal sessions**; the plan defers alternate-screen to "later". Confirm intended.

### 1.3 Two refinements found from the playground fixtures, not yet in the plan

Both came from reading `PlaygroundService.php::getExamples()` as specifications. Both are **live for Stage 20**:

- **Readonly statics must be const-evaluable, so they can seed other statics.** The `statics-and-constants` fixture has `static int $first = BASE_NUMBER;` then `internal static writable int $next = RequestSequence::first;`. The Stage 20 recommendation ("statics require const-evaluable initializers") never said whether a readonly static *is* const-evaluable. By its letter, Codex would reject this valid program. Fix: readonly statics with const-evaluable initializers are themselves const-evaluable; ordering still collapses into the const dependency graph.
- **Static places belong in §3.2's read-modify-write rule.** The same fixture has `self::next += self::STEP;`. §3.2 says "any writable place" but then enumerates only property places (Stage 20) and indexed places (Stage 23). Name static places explicitly, or the enumeration reads as exhaustive — which is exactly how decision 0034's "writable locals" got read as a prohibition.

### 1.4 §12 numbering: four labels collide with real records

§12's list numbers are subject labels per its own policy, but four collide with *assigned* records:

| §12 label | Real record with that number |
|---|---|
| 0071 formatted I/O | `0071-stage-12-general-control-flow` |
| 0072 `Doria\Std\Term` + Console | `0072-stage-14-floats-and-bool-runtime` |
| 0073 release versioning | `0073-stage-15-llvm-release-backend` |
| 0074 `Doria\Std\Math` geometry | `0074-stage-17-stdio-and-formatted-io` |

Only 0075 matches. **Recommendation:** §12 carries **no numbers** for unauthored records — subjects only; numbers appear when the file does. This also removes the `0071 [assigned 0074 in-repo]` wart. Console's entry points at 0006; DDO's 0007 is now superseded (§1.1), so its §12 entry is a subject label.

---

## 2. Unverified assertions — check, do not inherit

- ⚠ **"The website's executable-examples test only checks textual shape."** This is **Codex's claim, relayed without verification**. `PlaygroundService.php` proves the playground genuinely invokes `doriac` (`resolveCompilerCommand` → `createWorkspace` → `commandForAction` → `proc_open`); it does **not** contain the test in question. `getExamples()` returns static heredocs with no compile at definition time, so a shape-only test is *possible* — inference, not knowledge. A §0 rule ("website examples are compiled in CI, never shape-checked") was written on this premise and has been **reverted**. If the claim is true and the rule is wanted, it is Andrew's call and belongs in the website repo, not the compiler's plan (§14 lists website work as out of scope).
- ⚠ **Records 0038, 0040, 0044, 0045, 0074, 0075, 0081, 0082, 0083 cited by number in the plan.** Verified against an `ls` of `docs/decisions/` on 16 Jul. Re-verify; `scripts/check_docs_authority.php` now enforces this.
- ⚠ **The 21 playground fixtures are unverified against the compiler.** They encode intent (and have twice exposed plan gaps), but nobody has confirmed which currently compile.

---

## 3. Working conventions (belong in AGENTS.md / CLAUDE.md, not in an agent's head)

These were repeatedly violated and repeatedly re-taught. Make them mechanical.

**Markdown**
- **Tables are whitespace-padded so every pipe aligns.** Cells `ljust` to the column's max width; separator dashes fill `width + 2`. Touching one cell means re-padding the whole table. (The plan's two tables were left ragged by edits and flagged by the IDE.)
- §12 record entries: **one clause, ≤17 words, no bold**, separator ` · ` — never `. · `. A longer entry means the record needs authoring, not the entry expanding.
- §0 rules: one flat paragraph, ≤60 words. No nested bullets, no second paragraph.
- §9 blocks: lead-in + bullets (see "Formatted I/O"), never a prose wall.
- Headings: `### N.M Title` or don't exist.
- **Measure the neighbours before writing into a section.** Chat-session edits ran 5–30× the surrounding norm — median 44 words against 7 in §12, one 554-word paragraph in §9.

**Spelling**
- American: `behavior`, `labeled`, `favor`.
- **Fibonacci**, **Mojibake**, **Multithreaded** (project convention; keeps IDE dictionaries quiet).

**Citations**
- Plan prose cites a record **subject** ("the Console/terminal decision") until `docs/decisions/NNNN-*.md` exists; a number only after. Enforced by `check_docs_authority.php`.
- When writing a guard, **enumerate the forms a fact takes**, not the form in front of you. The first citation regex matched only `record 0085` and silently passed `record-0085`, `records 0081/0082`, and `decision 0051`.

**Reading records**
- **Early stage records are snapshots, not law.** 0034 says "writable locals" because Stage 9 had nothing else — it is not a prohibition on property increment. `0002-default-public-internal-members.md` still carries pre-two-state wording in its filename. When an agent cites an old record as a constraint, ask whether it *decided* something or merely *described what existed*.

**Blast radius** (now plan §0)
- Every change reports **"Invalidated elsewhere"**. Empty is a claim; missing is a skipped step. Before an edit, grep the fact — old value, siblings, dependents. After, grep what the edit falsified. Before accepting a rule, name its casualties.

---

## 4. Outstanding work

| Item | State |
|---|---|
| Stage 20 rectification prompt (static access + property increment) | Written, **not sent** to ChatGPT — `chatgpt-stage20-statics-rectification.md`. Needs §1.3's two refinements folded in first. |
| Blast-radius additions for ChatGPT + Codex prompts | Written, **not distributed** — `blast-radius-requirement.md`. |
| Namespace grammar slice prompt | Written, **not sent** — `codex-grammar-slice-namespaces.md`. Should land before Stage 20 completes; PR #80's IntelliJ action generates `namespace`/`extends` the parser rejects. |
| Formatting pass on the plan | **Not started.** §12 compaction + drop speculative numbers, DDO/Console → lead-in + bullets, two §0 monsters trimmed, headings renumbered. Formatting only, no decision changes. |
| DDO rewired to elaborate 0007 | **Resolved (PR #85):** 0007 superseded by the §9 charter, not elaborated; the modern DDO record is authored fresh when scheduled. |
| `check_docs_authority.php` | Updated with `std::` guard, acronym-casing guard, strict `.doria` code checks, citation-integrity check. ⚠ Never linted with `php -l` or run against the tree — regexes verified only by Python simulation. |
| ChatGPT standing context | **Stale.** Predates the §12 numbering policy (its Stage 20 prompt got it backwards), the `Doria\Std\*` sweep, the two-clocks rule, and blast radius. Needs a full refresh. |

---

## 5. What a Claude Code session should do first

1. `ls docs/decisions/` and reconcile §12 against reality — the chat session never had this and guessed for weeks.
2. Run `php scripts/check_docs_authority.php` and `php -l scripts/check_docs_authority.php`. Both are unverified.
3. `grep -rn 'std::' docs/ examples/ editors/` — the sweep covered the plan; decision records and prompts were fixed by hand and `docs/notes/` is exempt by design.
4. Open `doria-language/` (both repos), not `doria/`. Two of the last three gaps lived in the website.
5. Read `docs/decisions/0006` (Console) and the §9 DDO charter before touching Console or DDO; `0007` is superseded history.
