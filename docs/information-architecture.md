# Documentation Information Architecture

Doria has one documentation hierarchy. Documents may add context, rationale, examples, or current implementation details, but they must not compete with the active source of truth.

If a document duplicates the end-to-end plan's job, delete or redirect it. Do not keep patching stale roadmap fragments.

## Active authority

### docs/doria-end-to-end-plan.md

Role:

- Master future execution plan.
- Owns the project skeleton.
- Owns phase and stage ordering.
- Owns implementation sequencing from the current stage to v1.0.

The end-to-end plan should not contain every detail forever. When a detailed decision record exists, the plan may link to it instead of duplicating it.

### docs/decisions/*.md

Role:

- Accepted design decisions.
- Precise authority for a topic once authored and accepted.
- Can override older notes.
- Should be referenced from the end-to-end plan where relevant.

### SPEC.md

Role:

- Current language specification.
- Current implementation status where needed.
- Not a parallel roadmap.
- Not a planning document.

### README.md

Role:

- Repository entrypoint.
- Public product overview and quickstart.
- Links to the plan, specification, decisions, historical notes, and this information architecture.
- Written from the completed-release perspective; interim stage completion and implementation-status caveats stay in internal planning and pipeline documents.
- Not a roadmap replacement.

### AGENTS.md

Role:

- Working rules for Codex, agents, and contributors.
- Source-of-truth rules.
- Branch and validation expectations.
- Prompt-generation guardrails.

## Supporting context

### docs/stdlib-reference.md

Role:

- The at-a-glance catalogue of the core and standard-library surface (companions, interfaces, collections, free functions, and `Doria\Std\*` modules with their members).
- A derived **index**, not an authority: the end-to-end plan §9 owns direction and rationale, and the decision records own the precise contract. Every entry links to them.
- Kept in sync when a stdlib decision record is authored or amended; marks members `(surface TBD in …)` until their decision exists.

### docs/notes/*

Role:

- Historical notes.
- Migration notes.
- Branch review notes.
- Non-authoritative context.

Historical notes are preserved for memory, not instruction. They must not be treated as current roadmap or source-of-truth material.

### Supporting design notes

Role:

- Rationale and focused design exploration.
- Subordinate to the end-to-end plan and accepted decision records.
- Should not contain active roadmap instructions unless explicitly still active.

Examples include focused notes on brand positioning, API style, mutability ergonomics, self-hosting, PHP interop, performance, executable initializers, and website content.

## Superseded planning docs

Role:

- Deleted, redirected, or archived under `docs/notes/`.
- Never active in parallel with the end-to-end plan.

Superseded planning documents should be short redirects when their old paths are still useful to preserve. They should not keep large stale bodies below the redirect text.
