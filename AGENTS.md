# AGENTS.md

## What this document is

Durable guidance for working on Doria: identity, guardrails, language laws, and working discipline that should hold for most of the codebase's lifetime, barring a deliberate philosophical or strategic change.

It is **not** a status tracker. Stage completion, sequencing, acceptance criteria, and the currently supported native surface live in `docs/doria-end-to-end-plan.md` and `docs/notes/current-pipeline.md`. Do not duplicate them here — a rule that names a stage number or a completion state goes stale, and a stale rule in the file agents read first is worse than no rule.

Where sequencing genuinely matters to a rule, express it as a **dependency**, not a stage number: "migrates to declared `throws` when checked errors land" survives renumbering and re-planning; "at Stage 29" does not.

## Project

Doria is a compiled programming language for native applications, command-line tools, services, games, and systems software. The compiler is `doriac`, and the current `doriac` is a Rust bootstrap compiler. The native backend and standalone native executables are the primary product target. PHP is only an optional compatibility, migration, debugging, and inspection backend.

A strategic early goal is **self-hosting**: as Doria matures, more of `doriac` should be written in Doria itself. Rust is the bootstrap implementation language, not the permanent identity of the compiler.

Doria is intended for native CLI tools, native desktop applications, game tooling, game engines, graphics/media work, C-library bindings, and future raylib bindings.

The accepted native backend strategy is dual-profile:

```text
Fast native profile       -> Cranelift
Optimized native profile  -> LLVM
```

The fast profile is for local development feedback and smoke builds. The optimized profile is for release/shipping builds. Both profiles must preserve the same Doria-visible semantics for supported code.

Doria may intentionally support features PHP cannot express directly, including executable instance property initializers and richer attribute/metadata expressions. PHP backend limitations must not define Doria semantics.

Doria may eventually include a PHP-to-Doria migration converter, but that converter is a migration tool, not the Doria parser and not the core compiler identity.

The accepted project-tool name is Baton. Baton is the planned user-facing project, package, build, and application orchestration tool. `doriac` remains the compiler.

## Repositories and roles

- `doria` is the compiler repository and the subject of these instructions.
- `doria-website` is a separate repository. It hosts the documentation site and a playground that invokes `doriac` against user source, plus per-stage examples used for acceptance testing. Do not modify, clone, scaffold, or make assumptions about it from compiler work.
- Playground acceptance testing has repeatedly found language-design gaps before implementation calcified them. When a compiler change alters an accepted surface, report it as an "Invalidated elsewhere" item so the website can follow. Do not schedule website work from a compiler prompt.
- Andrew is the language designer and sole developer. He reviews and approves before anything advances.

## Non-negotiable engineering guardrails

- Correctness and accuracy outrank quick demos, fast runnable output, and compatibility shortcuts.
- Design Doria as a native-first language even when a native backend slice is not yet implemented.
- Do not choose syntax, type rules, runtime behavior, standard-library shape, or IR structure because it is easier for the PHP backend.
- Use PHP-shaped syntax, vocabulary, and spelling by default when the choice is only surface familiarity. Stop and ask only when the choice affects Doria-visible semantics, safety, memory/runtime behavior, ABI/layout, Cranelift/LLVM conformance, backend independence, ownership/lifetime behavior, or a costly-to-reverse public design.
- Do not treat generated PHP as a semantic oracle. It is backend output, not the definition of Doria.
- Do not silently import behavior from PHP, Rust, JavaScript, C, C++, or any backend/runtime ecosystem.
- If implementation hits a language-design fork not explicitly settled by `SPEC.md`, `docs/decisions/`, or the current task prompt, stop and ask Andrew.
- When stopping for a design decision, report the question, viable options, tradeoffs, affected files, and a recommendation clearly marked as a recommendation.
- Do not implement a workaround that makes the current backend pass while leaving Doria semantics ambiguous.
- Prefer clear unsupported-feature diagnostics over permissive behavior that may become wrong.
- Preserve the ability to lower Doria to native code safely, even if the immediate task only touches frontend code or a compatibility backend.
- Do not silently rename or replace Baton.
- Do not claim Baton is implemented until it exists.
- Do not turn Baton into a separate compiler or semantic authority.
- Do not present `doriac check` as a mandatory public workflow stage.
- Public onboarding uses write/build/run.
- Compiler-oriented documentation may still document direct `doriac` commands.
- If Baton design encounters an unresolved product or language fork, stop and ask Andrew.

## Decision triage

Stop and ask Andrew only when a decision affects one or more of:

- Doria-visible language semantics
- safety or memory guarantees
- ABI or externally observable data layout
- Cranelift/LLVM conformance
- public APIs that would be costly to reverse
- syntax, type conversions, ownership, destruction, or runtime behavior

For reversible implementation details:

- choose the simplest correct backend-independent option
- explicitly record the assumption
- test it
- proceed without blocking

At completion, report assumptions made and critical decisions encountered. If no critical decision requires Andrew's input, say so directly.

## Blast radius

Every change reports what it invalidates elsewhere, under a field named "Invalidated elsewhere". An empty answer is a claim that can be checked; a missing field is a step that was silently skipped. This applies equally to compiler code, `SPEC.md`, decision records, the end-to-end plan, editor fixtures, examples, and agent prompts.

The procedure is deliberately mechanical, because judgment is what fails here. Global thinking that happens when a connection feels interesting is unreliable exactly when the connection is merely load-bearing.

- Before an edit, grep the fact being changed: its old value, its siblings, its dependents. The question is "what else in the system asserts this?", never "does this line need fixing?".
- After an edit, grep for what the edit just made false.
- Before accepting a new rule, name what the rule invalidates. A rule with no listed casualties has not been checked.
- When writing a guard, a lint, or a CI check, enumerate the forms the fact takes rather than the form in front of you. A pattern matched against one example is an example, not a pattern.
- Sweep the whole tree, not the file being edited: `docs/`, `docs/decisions/`, `SPEC.md`, `README.md`, `AGENTS.md`, `examples/**`, tests, fixtures, and diagnostic snapshots. For editor-visible language changes, coordinate the corresponding work in `dorialang/doria-language-server`.
- Fix what is in scope, report what is not, and never leave a falsified claim standing because it was outside the diff.

Locally-correct fixes are this project's dominant defect source. Every recorded instance was caught late — by review or by acceptance testing — at the most expensive moment available.

## Verifying claims

- Do not repeat another agent's assertion as fact. Cite it as a claim and verify it, or mark it unverified.
- Do not reason about the repository from a document that describes the repository. Read the tree.
- Early records are snapshots, not law. A record that says "writable locals" may say so because locals were all that existed when it was written, not because anything else is prohibited. Before citing an old record as a constraint, ask whether it decided something or merely described what existed at the time.
- Record numbers listed in the end-to-end plan's decision-record section are subject labels, not assignments. Assign real numbers at authoring time from the next free slot in `docs/decisions/`, and verify the slot is unused.
- Prose cites a record subject ("the Console/terminal decision") until `docs/decisions/NNNN-*.md` exists, and a number only afterwards. `scripts/check_docs_authority.php` enforces this.
- Console has an accepted direction record (0006) that predates the end-to-end plan; read it before designing Console, and later work elaborates it without silently contradicting it. DDO's early record (0007) is **superseded** by the plan's §9 DDO charter — treat §9 as authoritative and do not build on 0007.

## Language identity — permanent nevers

These are identity, not scope deferral. They do not become available later, and they are distinct from the "not yet" list in the end-to-end plan.

- Tracing garbage collection, and pervasive reference counting as the default memory model.
- Rust-spelled borrow sigils and lifetime annotations. Inference and elision only.
- Doria does not use `public`, `protected`, or `private` as member visibility modifiers. Members are externally accessible by default; `internal` is the only access-surface keyword.
- A broad PHP-style `array` type. Sequences are `T[]` typed arrays and named collections.
- `Vec` as a collection alias.
- PHP loose comparison, `===`, and `!==`.
- `instanceof`. `is` is the single type-test and narrowing operator; the fixit is `is`.
- Late static binding through `static::`. `static` is the member modifier; `self::` is the qualifier.
- Sigil-carrying static access (`Foo::$prop`). The fixit is `Foo::prop`.
- `print`. `echo` is the one output spelling.
- `__toString`. Display conversion is the `Displayable` contract.
- Catchable panics. Panic is fatal, non-catchable, and non-unwinding.
- Full dynamic reflection: instantiate-by-string, invoke-by-string, and field access by name. Compile-time introspection through attribute-driven codegen is the sanctioned mechanism for shape-driven behavior such as row mappers, serializers, dependency injection, and validation. Dynamic reflection fights the headless class representation, punches holes in the type system, defeats dead-code elimination, and is a deserialization-attack surface.
- `require`, `require_once`, and `include_once`. Doria `include` already means required include-once.
- C/C++ textual macro substitution, `#define`, and `#undef`.
- Raw escape sequences as any public standard-library surface. The terminal layer is capability-based.

## Working rules

- Treat `docs/doria-end-to-end-plan.md` as the master execution plan for future work. It answers future-work forks unless Andrew later amends it.
- Treat supporting specification, notes, and decision files as subordinate where they conflict with the end-to-end plan.
- Treat `docs/doria-end-to-end-plan.md`, `docs/decisions/`, `SPEC.md`, `README.md`, `AGENTS.md`, and `docs/information-architecture.md` according to the documentation authority model. Supporting design notes are subordinate to the end-to-end plan and accepted decisions.
- Doria has a real ownership/borrow checker model in Doria spelling: readonly is shared borrow, writable is exclusive borrow, and `take` transfers ownership.
- `use` is namespace import/alias, `uses` is trait composition, and `with` is closure capture. These three keywords are not interchangeable.
- Keep compiler work incremental and tested, but never use incremental delivery as an excuse to make unsound language decisions.
- Do not make PHP the public identity of Doria. PHP is development context, migration context, and one optional compatibility backend; Doria should be described as its own native-first language.
- Do not describe Doria as a Rust language. Rust is only the bootstrap implementation language for the current `doriac`.

### Architecture

- Preserve the public compiler pipeline: lexer -> parser -> AST -> semantic/type checking -> Doria IR -> backend.
- Treat Doria IR as the checked compiler-owned representation. MIR is the single active native-oriented IR for control flow and runtime calls; later native work must extend or deliberately evolve it rather than add a parallel lowering path.
- The debug interpreter, Cranelift fast profile, and LLVM release profile consume the same validated typed MIR. Do not recreate a parallel native IR, bypass shared MIR validation, or execute user code as a native-compilation preflight.
- The durable parity manifest at `crates/doriac/tests/fixtures/native_parity_examples.txt` drives exact stdin/stdout/stderr/status and file-side-effect differential tests, and must cover every finite `examples/native/*.doria` fixture.
- `doria-rt` owns native entry, class allocation and free, raw device I/O, line discipline, file I/O, exact output, canonical display conversion, refcounted strings, and abort-only panic behavior. Its ABI is internal and never treated as stable.
- Do not let PHP backend needs leak into the parser, AST, semantic model, Doria IR, or native-oriented IR design.
- For native work, keep the fast Cranelift profile and optimized LLVM profile semantically equivalent for supported code. Differences may be in compile time, optimization, debug information, and binary quality, not Doria behavior.
- `--release` must select LLVM explicitly and may never silently fall back to Cranelift. Compiler builds without the optional LLVM feature must fail clearly.
- Native backends must call shared MIR validation. LLVM lowering must not use fast-math flags or unchecked undefined/poison-producing operations for defined Doria behavior.
- Do not let Cranelift or LLVM semantics decide Doria semantics. Backend-specific assumptions must remain behind Doria IR or native-oriented IR lowering.
- Class instances are a headerless, data-only heap payload with static per-type drop glue. Interface dispatch, when it lands, uses fat pointers rather than per-object headers.
- The object-representation machinery must not assume every aggregate with methods is a heap-allocated move type. Compiler-known inline Copy aggregates exist and share layout machinery with, but not the heap/move classification of, classes.

### Two clocks

- The parser tracks the accepted language; the checker tracks what is implemented. Accepted-but-unimplemented syntax must parse cleanly and then produce a semantic unsupported-feature diagnostic naming its landing stage, never a parser malformed-syntax error. The LSP delegates to the compiler and the website playground runs examples against it, so a parser that rejects future syntax makes valid Doria show as errors and destroys early developer-experience feedback.
- Grammar work is assigned, never implied. When syntax is accepted, its lexer/parser work goes at that moment into a named grammar slice or the nearest preceding stage. It is never left to the semantic stage that gives the syntax meaning, because that is the deferral this rule rejects.
- Every stage that activates syntax ships a compiler-side accepted-syntax regression test asserting that accepted-but-unimplemented forms yield stage-named unsupported diagnostics and zero parser errors. Coordinate the corresponding LSP no-false-diagnostics coverage in `dorialang/doria-language-server`.
- Do not confuse unsupported native backend coverage with invalid Doria. If a construct is valid Doria but unsupported by the current native slice, call it unsupported native backend coverage.

### Surface and spelling

- Treat PHP-shaped spellings such as `function`, `class`, `interface`, `trait`, `extends`, `implements`, `namespace`, `use`, `as`, `include`, `declare`, `echo`, `return`, `if`, `else if`, `else`, `while`, `for`, `foreach`, `try`, `catch`, `throw`, `new`, `->`, `::`, `.`, and `#[...]` as the default surface direction unless contradicted by an accepted decision.
- Apply "PHP's spelling is an artifact, not a decision" before importing PHP syntax. Ask whether PHP's spelling solves a problem Doria has. `read_line`, `is`, and sigil-free static access are deliberate Doria spellings, not compatibility gaps: each replaced a PHP spelling that existed only because of PHP's dynamic parser or its lack of enforced casing.
- Do not inherit PHP runtime semantics: loose typing, truthiness, dynamic properties, variable variables, `eval`, runtime include behavior, PHP autoloading, PHP arrays as every collection model, PHP references as-is, PHP trait conflict rules, and PHP magic behavior all require deliberate Doria decisions.
- Do not treat accepted PHP-shaped OOP syntax as permission to import all PHP runtime behavior.
- Do not make PHP output the semantic oracle for Doria OOP behavior.
- Preserve Doria's naming charter by category: built-in free functions use `snake_case`; userland free functions, methods, static methods, companion/type APIs, properties, parameters, and named arguments use `camelCase`. Classes, interfaces, traits, enums, and enum cases use `PascalCase`; constants use `SCREAMING_SNAKE_CASE`; type parameters use single Pascal capitals such as `T`, `K`, and `V`. Namespace segments are `PascalCase` with acronyms folded (`Doria\Std\Io`, `Doria\Std\Http` — never `IO` or `HTTP`). Keep the inherited magic-method spellings `__construct` and `__destruct`.
- Built-in free-function names are fully worded and never fused: `str_case_compare`, not `strcasecmp`. Whitelisted abbreviations are only those more recognizable than their expansions, plus industry-universal single lexemes such as `printf`. Whitelisting is always explicit and documented in the plan's naming charter.
- Keep PHP-to-Doria spelling suggestions in shared compiler data so the future `doriac migrate php` command reuses one table rather than maintaining a second.
- Prefer nouns as properties and verbs as methods in Doria APIs and examples. Use property hooks for computed, validated, lazy, or guarded values instead of vague zero-argument noun methods such as `body()`.

### Types and values

- Preserve the accepted fixed-width numeric direction: `int` means `int64`, `float` means `float64`, and the accepted explicit numeric spellings are `int8`/`int16`/`int32`/`int64`, `uint8`/`uint16`/`uint32`/`uint64`, and `float32`/`float64`.
- No implicit conversions anywhere, including int-to-float. Conversion is explicit through companion intrinsics.
- Floating-point semantics are deterministic by default: IEEE 754, defined NaN and infinity behavior, and no fast-math-style transformations applied implicitly.
- Do not treat `array` as a Doria type. Doria has C-style typed arrays spelled `T[]`, such as `int[] $numbers`; broader collection APIs use `List<T>`, `Dictionary<K, V>`, `Set<T>`, and future named collection types such as `Queue<T>` or `Stack<T>`. `array $items` and `List<array>` are invalid Doria surface syntax. PHP backend output may still use PHP `array` internally when lowering Doria collections.
- `mixed` is the only dynamic type and is unknown-flavored, never any-flavored: every value may flow in implicitly, and nothing may be done with it until narrowed. It is a boxed move type even when the payload is Copy.
- `object` does not exist. `null` is a literal, not a type-position name; nullable values are spelled `?T`. `void` is return-position only.
- Preserve the accepted typed equality and boolean operator direction: `==` and `!=` are typed equality/inequality; Doria does not use PHP loose comparison.
- Treat `not` as an exact synonym for `!`, `and` as an exact synonym for `&&`, and `or` as an exact synonym for `||`. Do not import PHP `and` / `or` precedence.
- Treat `xor` as a bool-only, non-short-circuiting boolean exclusive OR. It is not bitwise XOR.
- Treat `&`, `|`, `^`, and `~` as integer bitwise operators. Do not make `&` or `|` boolean aliases, and do not add `^^`.
- Do not add `nand`, `nor`, `implies`, `iff`, or `unless` without a new accepted decision.
- Treat `string` as immutable UTF-8 and Copy at the source level, backed by a private refcounted runtime representation. `echo` adds no newline and uses the same canonical display conversion as `.` and interpolation; never lower through newline-adding helpers such as `puts`.
- Doria is strongly typed in every parameter position. Free functions, methods, constructors, anonymous functions, arrow functions, interface requirements, trait requirements, property hook setters, callbacks, and future function-like forms must show explicit parameter types in docs, examples, tests, fixtures, and implementation grammar. Do not infer omitted parameter types and do not publish untyped arrow-function or anonymous-function parameters.
- Preserve readonly-by-default as the language default. Use class-level ergonomics such as `writable class` / `readonly class` before adding shorter aliases for `writable`.

### Classes and members

- Keep `writable` and `internal` separate: `writable` controls mutation, while `internal` controls API surface.
- Static identity law: declarations carry `$`, while class and static access is sigil-free (`Foo::prop`, `Foo::method()`, `self::prop`, `self::method()`).
- Treat `self` as reserved compiler vocabulary denoting the declaring class, in scope and type positions. `parent::member()` is the parent-implementation spelling; its full semantics land with inheritance.
- Enforce one member namespace per class across constants, static/instance properties, and static/instance methods. Do not use punctuation or call syntax to select among conflicting declarations.
- A constructor write to a writable static is ordinary mutation, not constructor init access. Constructor init access governs `$this` and the instance under construction only.
- A readonly static with a const-evaluable initializer is itself const-evaluable and may seed another static. Static initialization ordering resolves through the constant-evaluation dependency graph; cycles are rejected with the chain shown.
- `Displayable` is a compiler-known nominal contract requiring explicit `implements Displayable` and exactly `function toString(): string`. Do not accept structural conformance or general interfaces early. Concrete classes dispatch `toString()` through ordinary method lowering; interface values and general dispatch land with interfaces.
- Lifecycle methods are compiler-invoked protocol points, not ordinary methods. Their legal shapes are an allowlist; unspecified modifier combinations on magic names are rejected by default, and they are never callable directly.
- Everything dies in reverse of construction: owned locals and temporaries drop in reverse initialization order among values still owned, and a class's `__destruct` body runs before its properties drop in reverse declaration order. This deliberately matches C++ rather than Rust, so the language is uniform.

### Control flow and errors

- Treat basic `if` / `else if` / `else`, `while`, traditional `for`, integer range `foreach`, standalone `++` / `--`, and unlabeled `break;` / `continue;` as MVP control flow. `if` is statement control flow and does not return a value; `when` is the planned value-returning conditional/control construct. `for` is the explicit counter/index loop; `foreach` is preferred for collections and ranges. `0..10` is inclusive, `0..<10` is exclusive-end, and range `foreach` bindings are readonly per iteration. `break` exits the nearest enclosing loop, and `continue` jumps to the next nearest-loop iteration.
- Read-modify-write works on any writable place, not only locals: `$this->value++`, `$counter->value += 2`, `self::next += self::STEP`, and indexed places once collections land. Each desugars to a read-modify-write over the place-borrow rule, so a writable place is required and the ordinary one-writer rule applies. Value-producing `++` / `--` expression semantics remain future work; statement position only.
- Do not treat the native `0..125` process-exit range as the range of Doria integer values.
- Do not require `main` to return `int`. `main(): void` is valid; falling through or using bare `return;` means successful process status `0`.
- Do not allow `return <expr>;` in `main(): void`; it is a void-return semantic error.
- Treat panic as fatal, non-catchable, and non-unwinding. Explicit `panic("message")`, checked-integer failures, invalid float-to-int conversion, and an invalid `main(): int` process status use the abort-only status-101 path.
- An abort-only panic runs no cleanup. RAII guards therefore do not restore state on the panic path; say so honestly wherever a guard's guarantee is described, rather than implying panics are covered.
- Treat `throw` / `throws` as the accepted checked-error spelling direction. `throw` raises checked errors, `throws` declares checked thrown error types, and callers must catch or declare thrown errors once implemented. Do not implement checked-error compiler behavior, `try` / `catch`, runtime exception machinery, or `Result<T, E>` as the default Doria error model without a dedicated accepted implementation decision.
- Errors escaping `main` are an orderly declared failure: destructors run on the propagation path and the process exits 70, distinct from a panic's 101. The split is machine-readable triage for a supervisor or host.

### Standard library and I/O

- The stdin spelling is `read_line`, never `readline`. The PHP spelling may appear only as migration input or in a fixit test that directs users to `read_line`.
- Treat `read_file` and `write_file` as UTF-8 text-file functions. `read_file` must validate before constructing a `string`; invalid bytes never enter a Doria string. `null` from `read_line` means EOF, never failure.
- File I/O is a three-tier family: UTF-8 text free functions, a binary `Bytes` tier, and a streaming `File`/stream-object tier. The text free functions panic on failure until checked errors exist, then migrate to declared `throws` signatures — a planned, recorded, announced signature change.
- Compiler-known intrinsics are language, not library: `read_line`, `sprintf`, `printf`, `write_stderr`, `read_file`, `write_file`, `panic`, and the byte-I/O family are recognized before name resolution. They have no namespace, no prelude entry, and cannot be redeclared. Their names are reserved globally and permanently.
- Format strings for `sprintf` / `printf` must be literal and are checked at compile time. Keep the literal-format analysis structured for reuse; it has more than one planned consumer.
- Root the standard library at the reserved `Doria\Std` namespace (`Doria\Std\Term`, `Doria\Std\Math`, `Doria\Std\Io`, `Doria\Std\Json`, `Doria\Std\Net`, `Doria\Std\Data`). First-party modules that are not standard library sit at `Doria\<Module>`. Never use Rust-shaped `std::term` spellings; `scripts/check_docs_authority.php` guards this.
- The standard library is batteries-included: the default answer to "where is the X library?" is "in std", added deliberately over time. This never means everything ships at once.
- Double-quoted interpolation uses the ordinary Doria expression grammar. Literal `{` uses `\{`; bare `}` is literal, `\}` is accepted, and brace doubling is rejected. Preserve left-to-right exactly-once evaluation and one canonical display conversion for interpolation, `.`, `echo`, and `%s`.

### Namespaces and resolution

- Preserve the accepted namespace/import/include/directive direction: namespaces are semantic symbol ownership, `use` is semantic import and name aliasing, `include` is required include-once compile-time source inclusion, and `declare` is a structured compiler/source directive.
- Any name containing `\` is absolute; unqualified names resolve through context. Doria has no leading-`\` form. This deliberately diverges from PHP, where a qualified-but-not-leading-`\` name is relative to the current namespace and the same spelling can mean two things depending on a distant `use` line.
- Use one resolution chain for every symbol kind: imports, then current namespace, then prelude. No per-kind special cases; PHP's global fallback for functions but not classes is a wart, not a model.
- Accept group imports (`use Doria\Std\Math\{Vector2, Vector3};`) and aliasing (`use X as Y;`). Reject wildcard imports: they reintroduce resolution-at-a-distance and make adding a library symbol a breaking change for every importer.
- The prelude is a small documented list the compiler injects, not a glob. Userland may shadow prelude names; compiler-known names may never be shadowed. Prelude additions ride the edition mechanism.
- Do not describe `include` as PHP runtime include, and do not treat `include` as the normal import mechanism.
- Do not confuse `use` with `include`, and do not confuse `use` with Baton package resolution.
- Use `use` only for namespace/file-scope semantic imports and aliases; use `uses` for class-body or trait-body trait composition.
- Do not document or implement class-body `use TraitName;` as accepted Doria; PHP migration should rewrite it to `uses TraitName;`.
- Do not implement `goto` without a separate accepted decision.
- Do not confuse source/compiler directives with runtime control flow.

### Tooling and ecosystem

- Language-server transport, syntax highlighting, shared editor fixtures, and IDE clients live in `dorialang/doria-language-server`; do not add them back to this compiler repository.
- Keep compiler frontend services reusable by `doria-language-server`, and coordinate protocol/editor updates when language behavior changes.
- Treat TextMate/editor highlighting as editor UX only, not lexer, parser, compiler, or LSP semantic-token support.
- Use `doria` fences for Doria Markdown examples. Keep `php` fences only for generated PHP, PHP interop, or PHP migration input/output.
- Planned Doria keywords may be highlighted in editor tooling to keep docs readable, but highlighting must never be described as compiler implementation.
- Do not mark rejected Doria syntax as accepted language syntax in editor tooling.
- Keep self-hosting in mind when designing compiler APIs, diagnostics, source management, Doria IR, and the standard library.
- Keep native desktop, game engine, C-library binding, and raylib goals visible when designing Doria IR, future native-oriented IR, runtime, memory representation, FFI, and performance benchmarks.
- Keep Baton architecturally outside the compiler pipeline. Baton may orchestrate projects and invoke `doriac`; it must not duplicate parsing, semantic analysis, type checking, Doria IR lowering, or code generation.
- CLI commands wrap reusable compiler services rather than owning compiler behavior, so future REPL, notebook, and incremental tooling never needs a second frontend.
- Keep executable initializers and attribute expressions represented as Doria concepts, not PHP workarounds.
- Keep PHP-to-Doria migration architecturally separate from the Doria parser. The migration tool may parse PHP, but Doria itself should parse Doria.
- Do not introduce external Rust crates unless they remove real complexity and the repository is ready to manage that dependency.
- Do not add repository utility scripts in Python, JavaScript, shell, or another scripting language out of habit. Prefer Rust for compiler/project tooling and PHP for small repository text/JSON/regex helpers unless a different tool has an explicit, documented advantage for that specific task.
- Tier-1 platforms are Linux, macOS, and Windows. Every runtime syscall-surface addition lands with its Windows implementation in the same change — never "Unix now, Windows later."

## Global planning and documentation hygiene

- The end-to-end plan is the skeleton.
- The compiler, language server, editor integrations, and website remain unreleased until the end-to-end plan is complete. Public READMEs and product-facing copy describe the completed language and toolchain; never expose interim stage completion, implementation drift, or "planned/not yet supported" caveats there. Keep those facts in the end-to-end plan, current-pipeline notes, tests, and agent guidance.
- Implementation prompts must start from the skeleton, not from local file edits.
- Before generating or executing a prompt, check whether an open PR already covers the work.
- Before adding docs, check `docs/information-architecture.md`.
- Do not create parallel roadmaps.
- Do not patch stale planning docs when deletion or redirection is the correct fix.
- Do not list deleted or superseded docs in "Read first."
- If a file duplicates the end-to-end plan, stop and classify it.
- A clear picture is required before implementation; a complete picture is not required.
- Local MVP work must not undermine the long-term architecture.
- When a design decision affects parser, AST, HIR, MIR, backend, LSP, editor grammar, docs, and tests, plan the full surface area up front, even if implementation is sliced; LSP and editor work is coordinated in `dorialang/doria-language-server`.
- The end-to-end plan states decisions; decision records hold rationale, alternatives, and consequences. Do not put the same reasoning in both. A plan entry that grows into an essay is a signal the record needs authoring, not that the entry needs expanding.
- Documentation and examples may only demonstrate behavior the plan or an accepted decision record specifies. Specified-but-unimplemented features shown in docs carry the stage in which they land.

Prompt checklist before implementation:

- What stage or decision in the end-to-end plan does this belong to?
- Is there an open PR already doing this?
- Which source-of-truth docs own this topic?
- Which files are active vs historical?
- Is this a local patch or a skeleton-aligned change?
- What future speed bumps will this remove?
- What future work must this avoid duplicating?
- What does this change invalidate elsewhere?

## Documentation conventions

- Markdown tables are whitespace-padded so every pipe aligns: cells pad to the column's maximum width, and separator dashes fill that width plus two. Editing one cell means re-padding the whole table.
- Match the surrounding section before writing into it. Measure the length and structure of the units already there; a bullet five times the length of its neighbours is wrong even when every word is true.
- The end-to-end plan's decision-record list holds one short clause per subject, separated by ` · `, with no bold and no trailing period before the separator.
- Use American spelling: behavior, labeled, favor.
- Project spellings: Fibonacci, Mojibake, Multithreaded.

## MVP non-goals

Scope and schedule live in the end-to-end plan's out-of-scope section, not here. That list is "not yet"; the permanent nevers above are "not ever". Do not restate either list in this document — check the plan.

Small native backend smoke tests are not ruled out by any non-goal. They are preferred once the required semantics are explicit enough to avoid backend-shaped shortcuts.

## Verification

Run core checks for compiler changes:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --workspace --all-targets --locked --verbose
cargo test --workspace --all-targets --locked --verbose
cargo run -p doriac -- check examples/php/person.doria
cargo run -p doriac -- hir examples/php/person.doria
cargo run -p doriac -- compile examples/native/main_return_zero.doria --target native --out build/native/main_return_zero
cargo run -p doriac -- compile examples/native/main_return_42.doria --target native --out build/native/main_return_42
cargo run -p doriac -- compile examples/native/main_void_hello.doria --target native --out build/native/main_void_hello
```

Run documentation guardrails for documentation changes:

```bash
php scripts/check_docs_authority.php
```

Run backend-specific checks only when the touched task depends on that backend. For the current PHP compatibility backend:

```bash
cargo run -p doriac -- compile examples/php/person.doria --target php --out build/person.php
```

When native backend work changes supported behavior, run linker-independent Cranelift and LLVM object tests plus the complete durable interpreter/Cranelift/LLVM differential suite across tier-1 platforms. The durable manifest must include every finite native example and compare exact stdout, stderr, status, and declared file side effects. Consult `docs/notes/current-pipeline.md` and the end-to-end plan for the currently supported surface — do not infer it from this document. Normal interpretation has no artificial block or call-depth cap, and native compilation has no interpreter preflight. Only `main(): int` crosses the `0..125` process-status boundary. Run leak checks when runtime ownership paths change, and never describe an unexecuted check as passing.
