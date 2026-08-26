# Doria End-to-End Development Plan

**Document ID:** docs/doria-end-to-end-plan.md
**Status:** Accepted master execution plan for Doria v0.1 → v1.0
**Audience:** The implementing agent (Codex) and the language designer
**Supersedes:** This plan is the authoritative future-work execution plan. It supersedes older roadmap, SPEC, and decision wording only where it explicitly resolves a future-work fork or scheduling question. Already-implemented behavior remains governed by current compiler behavior and accepted decisions until a later stage migrates it.

---

## 0. How to use this document

This is the single authoritative execution plan. It exists so that implementation can proceed **without design back-and-forth**. It does three things:

1. **Resolves every open language-design fork** in SPEC.md with a concrete accepted default, each traceable to a numbered decision record (Section 12 lists the records to author).
2. **Defines the full compiler, runtime, standard library, tooling, and PHP-interop architecture** from the current Stage 10 slice to v1.0.
3. **Sequences the work into phases and stages** with explicit scope, out-of-scope lists, and acceptance criteria, in the same incremental style as Stages 1–10.

**Rules of engagement for the implementing agent:**

- Implement stages strictly in order within a phase. Phases may not be reordered without designer approval.
- Every stage ships with: a decision record (if it introduces semantics), integration tests in `crates/doriac/tests`, updated `SPEC.md` and `README.md` sections, and example programs in `examples/`.
- Repository ownership is explicit: this compiler repository owns frontend services, compiler diagnostics, and compiler-side accepted-syntax tests. Language-server transport, editor token fixtures and guardrails, and IDE integration tests live in `dorialang/doria-language-server`. Any stage with editor-visible vocabulary or diagnostics coordinates matching coverage there; it does not add those assets back here.
- The "stop and ask" rule from SPEC.md §1.1 still applies, but **only for forks not answered by this document**. If this document answers it, implement it as written. If this document and SPEC.md conflict, this document wins for future-work items and SPEC.md wins for already-implemented behavior; flag the conflict in the stage's decision record either way.
- Native-first correctness policy is unchanged: Doria semantics → Doria IR → backend lowering. The PHP backend never defines semantics.
- Temporary backend limitations remain unsupported-feature diagnostics, never redefinitions of the language.
- **Documentation and website examples may only demonstrate behavior this plan or an accepted decision record specifies.** An example that presupposes unresolved semantics (an entry-point form, a feature interaction, a stdlib API no record covers) is itself a design fork: stop and ask before publishing it. Specified-but-unimplemented features shown in docs must be marked with the stage in which they land.
- **Blast radius is a required output, not a judgment call.** Every change — to this plan, to SPEC, to a decision record, to code, to an agent prompt — reports what it invalidates elsewhere under a field named **"Invalidated elsewhere"**. An empty answer is a *claim* that can be checked; a missing field is a step that was silently skipped. The procedure is deliberately mechanical, because judgment is precisely what fails here — global thinking that happens when a connection *feels* salient is unreliable exactly when the connection is merely load-bearing:
  - **Before** an edit, grep the fact being changed — its old value, its siblings, its dependents. The question is "what else in the system asserts this?", never "does this line need fixing?".
  - **After** an edit, grep for what the edit just made false.
  - **Before** accepting a new rule, name what the rule invalidates. A rule with no listed casualties has not been checked.

  This rule exists because locally-correct fixes are this project's dominant defect source, and every instance was caught late, by review, at the most expensive moment: `std::term` was a Rust-shaped spelling that felt locally sensible and leaked into records, prompts, and agent output for months; this plan simultaneously claimed record 0074 for formatted I/O *and* geometry math because one citation was corrected to satisfy one CI check without grepping the same file for the number nine lines away; §9's RAII wording was fixed while Stage 46 still asserted the opposite; a use-after-move diagnostic was specified to suggest `->clone()` before clone existed; the two-clocks rule below was adopted without asking what it invalidated, which is why the IntelliJ class action immediately began generating syntax the parser rejects. Move the check to the edit.
- **Two clocks: the parser tracks the *accepted* language; the checker tracks what is *implemented*.** Accepted-but-unimplemented syntax must parse cleanly and then produce a **semantic unsupported-feature diagnostic naming its landing stage** — never a parser malformed-syntax error. The rule already existed narrowly (Stage 18, on interface names: "do not claim the syntax is invalid Doria merely because Stage 35 has not landed"); it is now general. Rationale: the external LSP delegates to the compiler, and the designer's BDD/playground UAT runs per-stage examples against it, so a parser that rejects future syntax makes valid Doria show as errors and makes early developer-experience feedback impossible — which is how the namespace-model gap was found. Consequence, recorded honestly: **parser and lexer work moves earlier than the corresponding semantic stage** (the `\` separator, `extends`/`implements`, qualified names in type positions, and later traits, generics, closures, and `match` are parsed well before Stages 31/34/35 give them meaning). This is deliberate. Parsing is the cheap half, and the only alternative — a second grammar maintained inside the LSP — is guaranteed drift. Every stage that activates syntax ships a **compiler-side accepted-syntax regression test** proving stage-named unsupported diagnostics and zero parser errors, plus coordinated **LSP no-false-diagnostics coverage** in `dorialang/doria-language-server`. **Grammar work is assigned, never implied:** when syntax is accepted, its lexer/parser work is placed *at that moment* into a named standalone grammar slice or the nearest preceding stage — it is never left to the semantic stage that gives the syntax meaning, because that is the very deferral this rule rejects. Unassigned grammar work is how the namespace gap reached UAT: `\` lexing, qualified names in type positions, and `extends`/`implements` parsing belonged to no stage at all.

---

## 1. Decisions this plan makes — designer review checklist

These are the load-bearing choices. Each becomes a decision record before its first implementing stage lands. Andrew has approved this plan as the current master direction; later amendments should update this file and, where appropriate, the corresponding decision record.

| #   | Decision                                                | Accepted default in this plan                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
|-----|---------------------------------------------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| D1  | Memory model                                            | **Full ownership + borrow checking — Rust's model in PHP spelling.** Single ownership with move semantics for classes/collections, deterministic `__destruct` at end of owning scope, Copy semantics for primitives/strings, opt-in `SharedReference<T>`/`WeakReference<T>`/`WritableSharedReference<T>` for shared ownership. **No tracing GC, no pervasive ARC, no Rust sigils or lifetime annotations in surface syntax**                                                                                                                                                                                                                                                                                                                                                                                                                       |
| D2  | Borrow spelling                                         | readonly = shared borrow, `writable` = exclusive borrow, `take` = ownership transfer into the callee. Ordinary borrow rules (many readers XOR one writer; borrows cannot outlive owners; moved values unusable) are enforced entirely at compile time by a non-lexical borrow checker over MIR, with zero runtime cost and no dynamic fallback. Explicit `WritableSharedReference<T>` is the named dynamic-check escape hatch.                                                                                                                                                                                                                                                                                                                                                                                                   |
| D3  | Copy vs move                                            | Copy types: primitives, `bool`, `string`, ranges, enums with Copy payloads, and (from Stage 47) the built-in `Doria\Std\Math` value types (`Vector2/3/4`, `Quaternion`, ...) as compiler-known inline Copy aggregates. Move types: classes, `T[]` typed arrays, `List`/`Dictionary`/`Set`, `Bytes`, closures. Explicit duplication uses the future `->clone()` / `Cloneable` surface once method and interface support exist; before that, move-type duplication is deliberately unavailable and diagnostics must not suggest `->clone()` (see record 0083). No user-defined `struct` in v1.0 — classes are the owned record type (revisit inline layout post-1.0 if engine profiling demands it)                                                                                                                  |
| D4  | Integer overflow                                        | Arithmetic overflow panics in both dev and release profiles. Explicit `Int::wrappingAdd(...)`, `Int::saturatingAdd(...)`, `Int::checkedAdd(...)` for other behavior. A `declare` key may later relax this per-module for engine hot paths                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| D5  | Nullability                                             | `?Type` optional types (PHP spelling), `??` null coalescing, `?->` null-safe access. `null` is not assignable to non-`?` types. No implicit truthiness                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| D6  | Enums                                                   | PHP 8.1-shaped `enum` declarations extended with payload cases (tagged unions): `case Some(int $value);`. This is Doria's sum type                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| D7  | Pattern matching                                        | `match` is expression-position, exhaustiveness-checked over enums/bools/finite domains, PHP 8 `match` spelling extended with payload destructuring. `when` is the value-returning conditional chain per decision 0009                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| D8  | Errors                                                  | Checked `throw`/`throws` with PHP-shaped `try`/`catch`/`finally`. Errors are class instances implementing the built-in `interface Error`. `Result<T, E>` stays out of the surface model per decision 0035                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| D9  | Generics                                                | Monomorphized generics for functions, classes, interfaces, traits. Constraint spelling: `<T implements Comparable>`. No runtime generic reflection in v1.0                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| D10 | Closures                                                | PHP-shaped anonymous functions and concise arrow functions share one explicit capture model. Both forms require a `with` list when they reference enclosing local bindings, including Copy and Move values; a no-capture closure omits `with`. Closure and arrow-function parameters are explicitly typed, while only arrow return types may be inferred. `with`, never PHP closure `use`, records a readonly borrow (`with ($x)`), exclusive writable borrow (`with (writable $x)`), or ownership transfer (`with (take $x)`). Borrow-capturing closures remain borrow-bound, so escape checking rejects lifetimes beyond their captured owners and can suggest `take`                                                                                                                              |
| D11 | Concurrency                                             | Structured concurrency with `async function` / `await` / task groups; data-race freedom falls out of the ownership model via auto-derived `Sendable` / `Shareable` marker interfaces (Rust's Send/Sync payoff) checked at spawn boundaries. Detailed design gated behind its own decision record in Phase H                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| D12 | Unsafe & FFI                                            | `unsafe { }` blocks gate raw pointers (`Ptr<T>`, `MutPtr<T>`), foreign calls, and manual memory. `extern` declarations bind C ABI symbols. Everything outside `unsafe` keeps full safety guarantees                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| D13 | PHP interop (the strategic pillar)                      | **Four architecturally separate products**: (a) existing Doria→PHP compat backend, (b) `doriac migrate php`, **(c) the PHP→native-Doria runtime bridge via `baton build --php-lib`** (Baton orchestrates doriac compilation, a versioned C-ABI bridge, and generated PHP adapters — FFI as bootstrap transport, generated Zend extension as the intended production transport over the same contract), and **(d) native-Doria→embedded-PHP host runtime** (the Lenga pattern: a native application hosting PHP as first-class scripting) — architecture-visible now, implementation-deferred until the (c) contract is stable and separately approved. Generated PHP is never Doria's semantic reference. doriac remains the compiler and exposes only narrow emission primitives to Baton. (c) gets its own phase |
| D14 | Division/modulo                                         | `/` on `int` is truncating integer division; `%` is remainder with sign of dividend (C/PHP `intdiv`-consistent). Division/modulo by zero panics. `float` division follows IEEE 754                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| D15 | Numeric widening                                        | No implicit conversions anywhere, including int→float. Explicit `Int::toFloat($x)`, `Float::toInt($x)` (truncating, panics on NaN/out-of-range), and fixed-width conversions via `Int32::from($x)` (panics on overflow) / `Int32::tryFrom($x)` (nullable)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| D16 | String encoding                                         | `string` is immutable, valid UTF-8. Byte-level work uses `Bytes` (a mutable move-type buffer), and integer indexing on `string` is not allowed. Intrinsic properties/views distinguish grapheme clusters (`length`, `graphemes`), Unicode scalar values (`codePoints`), and UTF-8 bytes (`byteLength`, `bytes`); `chars` is not a Doria spelling. One canonical display conversion (§4.6) feeds interpolation, `.`, and `echo`: primitives convert out of the box (`bool` → `"true"`/`"false"`, never PHP's `"1"`/`""`), classes via `Displayable::toString()` — there is no `__toString` magic method                                                                                                                                                                                                                                                                                                                               |
| D17 | Inheritance model                                       | Single class inheritance, multiple interface implementation, trait composition via `uses` with explicit conflict resolution (`insteadof`/`as` PHP spelling accepted). Methods are non-virtual by default; `open function` opts into overriding; `override function` required at override sites                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| D18 | Standard entry runtime                                  | Every native binary links `doria-rt` (Rust-implemented runtime library): allocator, drop glue, `SharedReference<T>` refcount machinery, string/collection intrinsics, panic machinery, stdout/stderr. `doria-rt` is an internal ABI, not public, until v1.0                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| D19 | Naming charter & the free-function boundary             | **Built-in free functions are `snake_case`** with uniform, fully-worded names (`read_line`, `get_time`, `function_exists`); **userland free functions are camelCase**, so free-function casing alone marks what is Doria's versus the codebase's. **All member-style APIs are camelCase everywhere** — standard-library members and companion/type APIs included (`Int::wrappingAdd`, `String::startsWith`, `$s->isEmpty`, `Displayable::toString`) — because a member's receiver already carries its provenance. Type-coupled vocabulary belongs to its companion; cross-domain and environment capabilities may remain free functions. Types and enum cases `PascalCase`, constants `SCREAMING_SNAKE_CASE`; magic methods keep PHP spelling (`__construct`, `__destruct`). §9.1 is checkable API law                             |
| D20 | Explicit typing discipline                              | **Parameter types are never inferred — anywhere.** Free functions, methods, constructors (including promoted parameters), closures, arrow functions, callbacks, and property-hook setters all require written parameter types: `fn($x) => ...` is a compile error, `fn(int $x) => ...` is the language. Named functions and methods must declare return types; only an arrow function's return type may be inferred from its body. `let` locals may infer from their initializer. Nothing ever silently defaults to `mixed`. Full rules in §4.7                                                                                                                                                                                                                                                                    |
| D21 | Dynamic boundary types                                  | `mixed` is Doria's **only** dynamic type and it is **unknown-flavored, never any-flavored**: every value may flow in (implicit boxing), and nothing may be done with it until narrowed via `is` / `match`. `mixed` is a boxed, runtime-tagged **move type** — always, even when holding a Copy value. `object` does not exist. `null` is a literal and the `?T` machinery, never a standalone type-position name. `resource` is reserved for the Phase I PHP bridge, not a core v1.0 type. `void` is return-position only. Full rules in §4.8                                                                                                                                                                                                                                                                      |
| D22 | Sequences: typed arrays + named collections, no `array` | **Doria has no broad PHP-style `array` type** — `array $items` and `List<array>` are invalid, and the identifier is not a type name. C-style **typed arrays** are spelled `T[]` (`int[]`, `string[]`, `mixed[]`, `int[][]`): contiguous, fixed-length-after-creation move types — the engine-grade buffer. **Named collections** are the growable/structured family: `List<T>`, `Dictionary<K, V>`/`SortedDictionary<K, V>`, `Set<T>`/`SortedSet<T>`, `PriorityQueue<T>`, `Deque<T>` (bare = default/insertion-ordered, `Sorted` prefix = comparison-ordered; `Deque` subsumes queue/stack). Full inventory and naming in decision 0092. Bracket literals are contextually typed collection literals, never evidence of an `array` type. Full rules in §4.9                                                        |

Everything below elaborates these decisions into implementable specifications.

---

## 2. Vision, positioning, and end products

Doria is a statically checked, natively compiled systems language with PHP-shaped syntax and Rust-grade safety defaults, minus Rust's lifetime/borrow surface language. The strategic products it must eventually support, in priority order:

1. **A native systems language** producing standalone executables (already the accepted direction).
2. **The PHP power-backend story**: when a PHP application hits performance or capability limits, teams write the hot module in Doria and call it from PHP with near-zero friction, because the syntax is already familiar and the bridge is first-class (D13c). This is Doria's unique adoption wedge — no other native language offers PHP developers syntax continuity plus a generated, type-checked FFI bridge.
3. **A game engine written in Doria**, which drives requirements for: deterministic destruction (ownership/RAII — no GC pauses, no refcount traffic on hot paths), fixed-width numerics, floats and SIMD, unsafe/FFI for graphics/audio/input APIs, allocator control, and predictable value-type collections.
4. **A UI framework** integrating with PHP web backends, which drives requirements for: attributes-as-metadata, property hooks, closures, enums, pattern matching, and async.
5. **Portable terminal (TUI) applications as a first-class capability, not a happy accident.** The designer's PHP terminal game engines — Sendama (arcade-style terminal games) and Ichiloto (terminal-native 2D JRPG engine with a TUI editor) — lack native Windows support because they emit raw ANSI escape sequences. Doria fixes this at the platform level: a capability-based `Doria\Std\Term` layer (§9) abstracts Windows Console/VT and Unix termios+ANSI behind one API, so **user code never writes an escape sequence**, and the same TUI binary runs natively on Windows, macOS, and Linux. The intended port pattern for those engines is the D13c bridge run in reverse of the usual pitch: the engine core is rewritten in Doria (performance + portability) and compiled as a php-lib, while game projects keep their existing PHP scripting against generated stubs — or go full Doria against `Doria\Std\Term` directly.

**Long-range direction (informs architecture, never expands current scope): AI research, scientific computing, and data-intensive native systems.** Doria should eventually be able to grow a competitive numerical/AI ecosystem — as libraries plus compiler extension points, never as core-language AI syntax. Nothing AI-specific enters v1.0; instead, this plan threads *readiness constraints* through the stages it already has (§4.1 numerical semantics, §4.5 generics, §8.1 IR extensibility, §8.3 external memory, §8.4 compiler services, Stage 40 zero-copy FFI, §11 Baton data model), and the directional workstream lives in Appendix A (§16). The governing rule: prefer a recorded constraint or review gate over any new implementation task.

The plan sequences language work so that requirement sets 2–5 unlock in that order.

---

## 3. Memory model and safety: ownership and borrowing in PHP spelling (D1–D3, D12)

This is the foundational design. Doria adopts **Rust's ownership and borrow-checking model — the real mechanism, not an approximation** — and re-spells it entirely in vocabulary Doria already has. There is no tracing GC and no pervasive reference counting. What is deliberately absent is Rust's *surface*: no `&` / `&mut` sigils, no `'a` lifetime annotations, no `Box` / `&str` / `Rc<RefCell<T>>` vocabulary, and no borrow-checker jargon in diagnostics. The checker is Rust-grade; the spelling is PHP-grade.

The mapping in one table:

| Rust concept                     | Doria spelling                                                           |
|----------------------------------|--------------------------------------------------------------------------|
| Ownership + move semantics       | Plain assignment / plain argument passing of move types                  |
| Shared borrow `&T`               | readonly — the existing default for bindings, parameters, `$this`        |
| Exclusive borrow `&mut T`        | `writable`                                                               |
| Consuming (by-value) parameter   | `take` parameter modifier                                                |
| `Drop` / RAII                    | `__destruct` runs at end of owning scope                                 |
| `Copy` types                     | Primitives, `bool`, `string`, ranges, enums with Copy payloads           |
| `Rc<T>` / `Weak<T>`              | `SharedReference<T>` / `WeakReference<T>` stdlib types (opt-in shared ownership) |
| `RefCell<T>` interior mutability | `WritableSharedReference<T>` with runtime-checked `writable` access      |
| `Send` / `Sync`                  | Auto-derived `Sendable` / `Shareable` marker interfaces (Phase H)        |
| Lifetimes                        | Inferred only, with fixed elision rules; never written in surface syntax |

### 3.1 Ownership and moves

- Every value has exactly one owner: a binding, a property, or a collection slot. When the owning scope ends and the value has not been moved out, the value is destroyed: `__destruct` runs immediately and deterministically, then memory is freed. This is RAII — files, GPU buffers, locks, and sockets close at scope exit with zero GC pauses and zero refcount traffic, exactly what the game engine needs.
- **Copy types** (primitives, `bool`, `string`, ranges, enums whose payloads are all Copy) duplicate on assignment and argument passing. The ownership machinery is invisible for them, which means most everyday PHP-shaped code never encounters a move at all.
- **Move types** (classes, `T[]` typed arrays, `List<T>`, `Dictionary<K, V>`, `Set<T>`, `Bytes`, closures, enums with move payloads): assignment and by-value passing transfer ownership. Using a moved-from binding is a compile error.
- Diagnostics use plain ownership vocabulary — *owns*, *gives*, *still using*, *readonly*, *writable* — never *borrow*, *lifetime*, or `'a`:

```text
error[D0203]: $user was given to store() on line 12, so it can no longer be used here
help: call $user->clone() before line 12 if you need to keep a copy
```

- Explicit duplication is the future `->clone()` surface, backed by `Cloneable` once method and interface support exist. Until then, move-type duplication is intentionally unavailable except for compiler-internal lowering needs.

### 3.2 Borrowing is readonly/writable

Doria's existing readonly/writable rules are the borrow system — this plan makes them enforced borrows rather than surface conventions:

- **Parameters borrow by default.** A readonly parameter is a shared borrow: the callee may read, the caller keeps ownership, and any number of readonly borrows may coexist. A `writable` parameter is an exclusive borrow: the callee may mutate, and while it lives no other access to that value exists. SPEC §9's `function rename(writable Person $person, string $name)` already *is* an exclusive borrow in PHP clothing — no syntax changes.
- **`take` transfers ownership into the callee** for sinks, builders, and consuming APIs:

```doria
function store(take User $user): void
{
    $this->users->add($user);   // $user moves into the collection
}

store($person);                 // fine; $person is moved-from afterward
```

Call sites are unmarked, as in Rust; the signature is the contract and the checker enforces it.
- **Method receivers**: a normal method takes a readonly borrow of `$this`; a `writable function` takes an exclusive borrow — exactly the existing SPEC §5 semantics, now checked as true borrows.
- **The ordinary borrow rules** (compile-time only, zero runtime cost, checked non-lexically on MIR, excluding explicit `WritableSharedReference<T>` dynamic access checks): at most one live writable borrow XOR any number of readonly borrows of the same value; no borrow may outlive the value's owner; a moved value cannot be borrowed. Non-lexical means a borrow ends at its last use, not at the end of a block, so idiomatic PHP-shaped code rarely fights the checker.
- **Place expressions borrow implicitly**: `$obj->prop`, `$list[0]`, and chained access borrow for the duration of the enclosing operation — no sigils at use sites.
- **Read-modify-write works on any writable place, not just locals**: `$this->value++`, `$counter->value += 2`, and (from Stage 23) `$items[0]++`. This is not new semantics — each desugars to a read-modify-write over the place-borrow rule above, so a writable place is required and the ordinary one-writer rule applies. Decision 0034's writable-local-only restriction is a **Stage 9 scope artifact from before properties existed**, not a design decision: property places land with Stage 20's property mutation, indexed places with Stage 23's collections. Value-producing `++`/`--` expression semantics remain future work; statement position only. The compound-assignment operators are the arithmetic/bitwise set plus `.=` (string concatenation) and `??=` (null-coalescing), each a read-modify-write over this same place rule (decision 0094).
- **`foreach` borrows elements**: `foreach ($users as $user)` takes a readonly borrow per iteration (the existing readonly-loop-binding rule, now real); `foreach ($users as writable $user)` takes exclusive borrows for in-place mutation, requiring the collection binding itself to be writable.
- **Returned borrows use fixed elision rules, never annotations.** In v1.0 a function or method may return a borrow only when it derives from `$this` or from exactly the one borrowed parameter — Rust's elision rules, which cover getters, views, and accessors. APIs needing multi-source lifetime relationships must restructure to return owned values or use `SharedReference<T>`. Named lifetime/region annotations are rejected for v1.0 and may only be revisited post-1.0 with concrete evidence, and even then never in Rust spelling.

### 3.3 Shared ownership is opt-in, not the default

When single ownership genuinely does not fit — caches, observer lists, doubly-linked structures, scene-graph back-references — the stdlib provides explicit shared-ownership types instead of silently changing the language model:

```doria
class Node
{
    writable List<SharedReference<Node>> $children = [];
    writable ?WeakReference<Node> $parent = null;
}
```

- `SharedReference<T>`: an owning reference that may share responsibility for keeping one value alive with other `SharedReference<T>` values. Constructed with `shared new T(...)` (decision 0005): the value is created directly under shared ownership and destroyed exactly once when the final owning reference is released. `$ref->share(): SharedReference<T>` creates another owning reference to the same value — this *shares ownership*; it is not the `Cloneable` `->clone()`, which duplicates the underlying value. `$ref->createWeakReference(): WeakReference<T>` derives a non-owning reference. A `SharedReference<T>` gives direct readonly access to `T` — no acquisition step, because the readonly-only path needs no runtime bookkeeping. Reference counting is the implementation mechanism, but the source-level model is ownership and lifetime responsibility.
- `WeakReference<T>`: a non-owning reference to a shared value; it does not keep the value alive. `$weak->acquire(): ?SharedReference<T>` attempts to obtain a live owning reference, yielding `null` once the final owning reference has been released. Strong `SharedReference` cycles leak by design (documented); `WeakReference` breaks them.
- `WritableSharedReference<T>`: a shared owning reference that permits runtime-checked writable access. It is built with an ordinary constructor that takes ownership of the value — `new WritableSharedReference(new Settings())`, never `shared new` or a modifier chain. `$settings->share(): WritableSharedReference<T>` adds another owner; `$settings->createWeakReference(): WritableWeakReference<T>` derives the weak form. **It never forwards direct access to `T`** — unlike `SharedReference<T>`, controlled access must first be acquired, because every access must be counted:
    - `$settings->acquireReadonlyAccess(): ReadonlySharedReferenceAccess<T>`
    - `$settings->acquireWritableAccess(): WritableSharedReferenceAccess<T>`

    At runtime a `WritableSharedReference<T>` permits any number of readonly accesses XOR exactly one writable access, never both; incompatible access panics with a clear message in Doria's plain ownership vocabulary in Title Case (e.g. `Cannot Acquire Writable Access While Readonly Access Is Active`), never Rust borrow-checker terms (`Mutably Borrowed`, `RefCell`, `Already Borrowed`). The reference binding need not be declared `writable` to request access — the reference is not being reassigned — but a `WritableSharedReferenceAccess<T>` binding must be `writable` to mutate through it, preserving Doria's rule that the whole write path permits writing.
- `WritableWeakReference<T>`: the non-owning form of `WritableSharedReference<T>`; `$weak->acquire(): ?WritableSharedReference<T>`. It exists so a weak reference into a writable shared graph re-acquires the writable capability rather than degrading to a readonly `SharedReference<T>` — without it, cycle-breaking would be impossible for exactly the parent-back-reference and observer graphs that motivate shared ownership.
- `ReadonlySharedReferenceAccess<T>` / `WritableSharedReferenceAccess<T>`: the temporary access objects. Member and indexed access forward to the underlying `T` (`$access->theme`, never `$access->value->theme`) as compiler-known behavior specific to these types, not general proxy or reflection. The readonly form permits only readonly operations; the writable form permits readonly and writable operations; **neither may move out of or consume the underlying `T`**. Each access object retains an owning reference to the underlying allocation, so holding access keeps the value alive. They are ordinary move types — returnable, storable, and passable, participating in normal ownership, move, and deterministic-destruction rules — but they **cannot be constructed directly by user code**; the acquire methods are the only source. Access stays live for the access object's lifetime and is released deterministically when it is destroyed; moving one transfers the release obligation, and a moved-from access object neither holds nor releases access. Because they are storable, an access object parked in a long-lived structure holds its access open for that lifetime — visible and deterministic, but the caller's responsibility.
- **The two families are disjoint.** A value joins one family at construction — `shared new T(...)` for the readonly-access family, `new WritableSharedReference(new T())` for the writable-access family — and never crosses: there is **no conversion, implicit or explicit, between `SharedReference<T>` and `WritableSharedReference<T>`**. `createWeakReference()` preserves the family (`WeakReference<T>` / `WritableWeakReference<T>`) and `acquire()` returns it intact, so a readonly-family handle and a writable-family handle can never refer to the same allocation. All `WritableSharedReference<T>` handles to one allocation share a single runtime access state, so access counting is per-allocation, not per-handle — and readonly-family allocations carry no access state at all, which is why `SharedReference<T>` costs nothing beyond its reference count.
- These types are **not thread-safe** and do not automatically cross thread boundaries; thread-safe variants arrive with Phase H under the `Sendable`/`Shareable` design record.

These types land at Stage 25a (thread-safe variants at Phase H); they depend on nullable types and narrowing (Stage 22) and generic classes (Stage 25). The type and operation names above are the approved canonical surface; the Stage 25a shared-ownership decision formalizes them together with the compiler-known access forwarding and access-lifetime rules.

### 3.4 Why this fits Doria's products

The engine gets Rust's performance model: no GC, no refcount traffic on hot paths, deterministic destruction, aggressive alias-based optimization license from exclusive borrows. PHP developers get a shallow on-ramp: Copy types plus borrowed-by-default parameters mean ordinary code reads and behaves like the PHP they know, and ownership only announces itself where it earns its keep — move types, `take` signatures, and `SharedReference<T>` in type positions.

### 3.5 Unsafe and FFI (D12)

For engine internals and C interop:

```doria
extern "C" {
    function malloc(uint64 $size): Ptr<void>;
    function free(Ptr<void> $ptr): void;
}

function fastCopy(writable Bytes $dst, Bytes $src): void
{
    unsafe {
        // raw pointer work permitted only here
    }
}
```

- `unsafe { }` is the only context where `Ptr<T>` / `MutPtr<T>` may be dereferenced, `extern` functions called, and ownership deliberately sidestepped (raw-handle intrinsics that convert a `SharedReference<T>` to and from a raw FFI handle; their exact member spelling is owned by the later unsafe/FFI decision, not settled here).
- `extern "C"` blocks declare foreign symbols; parameter/return types restricted to FFI-safe types (fixed-width numerics, `Ptr<T>`).
- An `unsafe function` spelling marks a whole function as requiring an unsafe context to call.
- `declare` keys will later govern per-module unsafe policy (deny/allow), per decision 0028's directive direction.
- The safety contract is Rust's: `unsafe` code must uphold the invariants the borrow checker assumes; everything outside `unsafe` keeps full guarantees.

### 3.6 Panics

A panic is a fatal runtime error, distinct from checked `throw`/`throws` per decision 0035: arithmetic overflow, division by zero, out-of-bounds indexing, `WritableSharedReference` access violation, failed `Float::toInt`, explicit `panic("message")`. Decision 0109 represents it through the compiler-owned diagnostic model with a stable code, precise source label, explanation, Doria `Call Path`, and status 101. **v1.0 panic policy is abort-only (no unwinding, no catching panics).** This keeps codegen simple and honest; checked errors are the recoverable path.

---

## 4. Type system completion (D4–D9, D14–D16)

### 4.1 Numerics

- Full fixed-width family per decision 0016 becomes real compiler types: `int8/16/32/64`, `uint8/16/32/64`, `float32/64`; `int` = `int64`, `float` = `float64`.
- Literals: `42` is `int` unless the expected type in context is another integer type and the literal fits (contextual typing, checked at compile time; `int8 $x = 200;` is a compile error). `4.2` is `float` with the same contextual rule for `float32`. Suffixed literal spellings are **not** added; contextual typing plus `Int32::from(...)` covers the need.
- Operators complete: `/`, `%` (D14), bit shifts `<<` `>>` (arithmetic right shift on signed; shifting by ≥ bit-width panics), bitwise `& | ^ ~` on all integer types.
- No implicit widening (D15). Mixed-type arithmetic (`int + int32`) is a compile error; convert explicitly.
- **Floating-point semantics are deterministic by default**: IEEE 754, defined NaN/infinity behavior (NaN compares unequal to everything including itself; no signaling-NaN surface), no fast-math-style transformations ever applied implicitly — value-changing FP optimization requires a future explicit `declare` profile, never a compiler default. Additional numeric types (`float16`/`bfloat16`, SIMD vector types) are **reserved future extensions** the semantic type model must be able to represent (§8.1), not v1.0 surface.

### 4.2 Nullable types (D5)

```doria
?Person $found = $repo->findById($id);

let $name = $found?->name ?? "anonymous";

if ($found != null) {
    echo $found->name;   // flow-narrowed to Person in this block
}
```

- `?T` is `T` or `null`. `null` literal has type `null` and is assignable only to `?T` and `mixed`.
- Flow-sensitive narrowing: `!= null` / `== null` comparisons and exact `is` tests establish non-null, null, or exact-type facts inside the guarded region. Stage 22 implements these facts on the shared forward-dataflow framework; Stage 28 Slice 1 adds guard-free `match` patterns as another fact source.
- Representation: `?T` for class types uses null pointers (zero cost); for other types a discriminant word (niche optimization is a backend improvement later).
- `mixed` remains the dynamic escape hatch for PHP-interop shapes; Stage 22 narrows it through explicit `is` checks (`$x is string`), while Stage 28 Slice 1 adds exact type-binding match patterns. `mixed` is deliberately unknown-flavored and boxed — Stage 22 implements its static rules, Stage 23 Slice 3 implements its runtime representation, and its complete language rules live in §4.8.

### 4.3 Enums and payload enums (D6)

```doria
enum Status
{
    case Draft;
    case Published;
    case Archived;
}

enum Shape
{
    case Circle(float $radius);
    case Rect(float $width, float $height);
}
```

- Backed enums (`enum Level: int { case Low = 1; ... }`) supported with PHP spelling.
- Payload cases make `enum` Doria's tagged union: inline tagged layout, a Copy type when every payload is Copy and a move type otherwise, monomorphized with generics later (`enum Option<T> { case None; case Some(T $value); }` ships as a stdlib type once generic enums land).
- Enum values compare with `==` by case + payload equality.
- Decision 0114 owns the complete enum semantics. Stage 27 executes unit,
  `int`/`string`-backed, and payload enums with nominal identity, central inline
  layout, recursive Copy/Move classification, active-case equality and cleanup,
  aggregate calls and returns, nullability, `mixed`, Copy constants/defaults,
  class/generic storage, and permitted collection value positions. Generic
  enums remain deferred; Stage 28 owns payload observation through `match`.

### 4.4 match and when (D7)

`match` is a value-returning expression with mandatory exhaustiveness over closed domains:

```doria
let $area = match ($shape) {
    Shape::Circle($r) => 3.14159265 * $r * $r,
    Shape::Rect($w, $h) => $w * $h,
};

let $label = match (true) {
    $n < 0 => "negative",
    $n == 0 => "zero",
    default => "positive",
};
```

- Arms: enum case patterns with payload destructuring, literal/constant patterns, exact type-binding patterns, `null`, and `default`. `Pattern if condition => value` is the sole guard form; guards require strict `bool`, run once after pattern success, and do not complete coverage unless compile-time true. `match (take $value)` explicitly consumes the whole Move scrutinee so a selected Move payload becomes owned. Writable patterns and payload-level `take` are rejected for v1.
- Non-exhaustive `match` over an enum or `bool` without `default` is a compile error.
- The ternary `cond ? a : b` is sugar for a two-arm `match` over a strict `bool` (`match ($cond) { true => a, false => b }`); the condition is never truthy, and PHP's short ternary `?:` is rejected in favor of `??` (decision 0094).
- `when` is the value-returning form of `if` — the same `given` / `else when` / `else` / `finally` structure (`when`/`else when` in place of `if`/`else if`), always yielding a value. One result type comes from the head annotation, then surrounding expected context, then head-branch inference; it is never written on `else when`. Every other branch is checked against it, `else` is mandatory, and each branch uses `return expression;` to yield from the nearest `when`. Decisions 0097 and 0116 are authoritative. Stage 28a executes both `when` and its shared finalizer regions.

### 4.5 Generics (D9)

```doria
function first<T>(List<T> $items): ?T
{
    // ...
}

class History<T>   // a domain type, distinct from the general Deque<T> collection (§4.9)
{
    internal writable List<T> $entries = [];

    writable function push(T $entry): void { /* ... */ }
    writable function pop(): ?T { /* ... */ }
}

function max<T implements Comparable<T>>(T $a, T $b): T
{
    return match ($a->compare($b)) {
        Ordering::Less => $b,
        default => $a,
    };
}
```

- Monomorphization at MIR level: each concrete instantiation generates specialized code (Rust model — zero-cost, no boxing). Compile-time cost is accepted; the dev backend (Cranelift) keeps iteration fast.
- Constraint spelling `T implements Interface` keeps Doria's own vocabulary; multiple constraints with `+`? **No** — spelling is `T implements A, B` inside the angle brackets, comma-separated, matching `implements` lists.
- Generic type inference at call sites from argument types; explicit turbofish-style spelling is **not** adopted — where inference fails, bind through a typed declaration.
- Collections `List<T>`, `Dictionary<K, V>`, `Set<T>` become real generic types in the compiler (they already have checked arity) backed by runtime intrinsics, then by stdlib generic implementations as self-hosting matures.
- **Extension point (decision 0105): compile-time value parameters.** Generic metadata, arity checking, and the monomorphization keying must not assume every generic argument is a type. Future numerical work may want value parameters (a `Buffer<float32, 4096>` shape of thing); v1.0 implements none of this, but the specialization machinery is designed so adding a value-parameter kind later is an extension, not a redesign.

### 4.6 Strings and Bytes (D16)

- `string`: immutable, valid UTF-8, and a Copy type (internally a refcounted immutable buffer, so copies are pointer-cheap). One string type deliberately avoids Rust's `String`/`&str` split. `$s->length` counts Unicode extended grapheme clusters; `$s->byteLength` reports exact UTF-8 bytes; `$s->isEmpty` is a constant-time emptiness fact; `$s->bytes` copies into `Bytes` in v1.0; `$s->graphemes` and `$s->codePoints` expose the two explicit text traversal units. Integer indexing and `$s->chars` are not Doria. Every string-specific operation belongs to the `String::` companion under decision 0103.
- `Bytes`: mutable move-type byte buffer for binary work, file I/O, network buffers, engine assets; in-place mutation goes through `writable` borrows like everything else.
- **Display conversion (amends the earlier string-only `.` decision).** One canonical, locale-independent conversion feeds all three display contexts — string interpolation `{...}`, `.` concatenation, and `echo` — resolving SPEC §7's open display-conversion question:
  - `string` converts as itself; the `int`/`uint` family converts to decimal digits; the `float` family converts to the shortest round-trip decimal (deterministic, never locale- or ini-dependent, unlike PHP's `precision` behavior); `bool` converts to `"true"` / `"false"` — explicitly **not** PHP's `"1"` / `""`, whose silent empty-string `false` is a classic wart Doria refuses to inherit.
  - Classes convert only by implementing the built-in `interface Displayable { function toString(): string; }` — **this is Doria's answer to PHP's `__toString`**. There is no `__toString` magic method: the magic-name surface remains exactly `__construct` / `__destruct`, and string-conversion conformance is the nominal interface (PHP 8's own `Stringable` interface is the precedent). `doriac migrate php` rewrites a `__toString()` method into `implements Displayable` plus `toString()`. Interpolating or concatenating a non-Displayable class remains a compile error with an implement-Displayable suggestion.
  - Deliberately **not** display-convertible: `?T` (narrow or apply `??` first — no PHP-style silent empty output for `null`), `mixed` (narrow first, per D21), collections and typed arrays (no PHP `"Array"` wart), enums (deferred to the enums decision), closures, and `Ptr<T>`.
  - `.` is hereby amended from string-only: it accepts display-convertible operands, with the guard that **at least one operand must already be a `string`** — so `"I am " . 183` is valid while `$a . $b` on two ints stays a compile error suggesting `+` or interpolation (vetoable guard; record 0045). `echo` accepts any display-convertible expression.
- Interpolation grows to full expressions in braces `{...}` in its own stage (Stage 18). **Literal braces in double-quoted strings (accepted with Stage 18):** `\{` is required for a literal `{`, joining the existing backslash escape set — one escape mechanism, never a second (`{{` doubling is rejected). A bare `}` is literal (it is special only as the terminator inside an open interpolation); `\}` is accepted but never required, so symmetric escaping never errors. A bare `{` that does not begin a valid interpolation is a compile error with a machine-applicable fixit ("write `\{` for a literal brace"). Single-quoted strings remain the escape-free home for brace-heavy text. `doriac migrate php` escapes literal `{` when converting double-quoted PHP strings.
- Ordered comparison of strings (`<`, `<=`, ...) is byte-lexicographic; locale-aware collation is stdlib territory, not operators.

---

### 4.7 Explicit typing discipline (D20)

Doria rejects PHP's gradual-typing looseness outright: a signature is a contract, and contracts are written down. The uniform rule is that **no parameter type is ever inferred**, in any function-like form:

- Free functions, methods, constructors (including promoted parameters), closures, arrow functions, callbacks passed to collection methods, and property-hook setters all require explicit parameter types. `fn($x) => $x * 2` is a compile error whose diagnostic suggests the expected type when the surrounding context (e.g. the function type of a `map` parameter) makes it computable — the compiler may *check* against context, but it never *silently fills* the type in.
- Named functions and methods must declare return types, `: void` included. The single inference allowance: an arrow function's return type may be inferred from its body expression, since the one-expression body is the entire contract.
- `let` locals may infer their type from the initializer — the right-hand side's type is already fully known and checked, so this is inference of convenience, not of contract. Parameters have no initializer to infer from; their type *is* the API.
- Omission never means `mixed`. PHP's costliest default — an untyped parameter silently accepting anything — does not exist in Doria; `mixed` must always be written deliberately.
- This is load-bearing, not stylistic: monomorphized generics, the borrow checker's readonly/writable/`take` analysis, and Copy-vs-move classification all key off precise parameter types at the declaration site. Inferring parameter types from call sites would couple checking to usage order and degrade diagnostics. Callback-heavy code stays ergonomic because parameter types are short to write and the LSP autofills them from the expected function type.

---

### 4.8 Dynamic boundary types: `mixed`, and the types Doria does not have (D21)

`mixed` exists because Doria's strategic products need one place for dynamism to land: `Doria\Std\Json` values, PHP-bridge payloads, and `doriac migrate php` output. It is designed so that a hole in static *knowledge* is never a hole in *safety*:

- **Unknown-flavored, never any-flavored.** A `mixed` value permits no operations at all — no method or property access, no arithmetic, no concatenation or interpolation, no comparison — until it is narrowed by an `is` check or a `match`. Prove, then use. An any-flavored `mixed` (PHP's untyped reality) would punch a hole through monomorphization, Copy-vs-move classification, and the borrow checker simultaneously; Doria never permits that.
- **Implicit in, explicit out.** Assigning or passing any value into a `mixed` slot boxes it silently — acceptable because writing `mixed` in a signature is itself the deliberate opt-in (D20 guarantees `mixed` is never a silent default), and this inbound widening is exempt from D15's no-implicit-conversion rule by design. Outbound is never implicit: only `is` narrowing and `match` extract the payload; no cast spelling exists.
- **Always a move type.** `mixed` is a boxed, runtime-tagged value (a `dr_mixed` intrinsic in doria-rt) and classifies as a move type even when the payload is Copy — one uniform rule, no special cases in the checker. Narrowed access follows the binding's existing ownership: narrowing a readonly `mixed` yields readonly access to the payload; moving the payload out consumes the box (Copy payloads copy out instead). Full ownership interaction is specified in the dynamic-boundary decision.
- **`object` does not exist.** "Any class instance" is just `mixed` plus a promise the type system cannot use: with runtime reflection out of scope, the only operation on such a value would be `is`-downcasting, which `mixed` already provides. Two dynamic boundary types where one suffices is precisely the PHP-shaped redundancy Doria eliminates elsewhere (`use`/`uses`/`with`, two-state visibility). Reintroduce post-1.0 only with concrete PHP-bridge evidence.
- **`null` is a literal, not a type-position name.** The null *type* exists internally (it is how `?T` assignment and narrowing are specified), but `null` in type position is rejected with a diagnostic suggesting `?T`. Docs list `null` under literals.
- **`resource` is reserved, not implemented.** Native Doria's resource story is RAII classes owning handles (plus `Ptr<T>` under `unsafe`). The `resource` name is reserved for the Phase I PHP bridge boundary and rejected until then with an unsupported-feature diagnostic; it does not appear in core type documentation except as reserved.
- **`void` is return-position only**; any other position is rejected with a diagnostic.

---

### 4.9 Typed arrays and named collections — there is no `array` (D22)

Doria has **no broad PHP-style `array` type**. `array $items` is invalid Doria; so is `List<array>`. The identifier `array` is not a type name and is rejected with a diagnostic pointing at this section's alternatives. The word may appear in Doria documentation and tooling only when discussing PHP backend lowering/output, PHP migration input, or explicitly rejected syntax.

What Doria has instead is a two-tier sequence model:

- **Typed arrays, spelled C-style `T[]`**: `int[] $numbers`, `string[] $names`, `mixed[] $items`, `int[][] $matrix`. A `T[]` is a contiguous, fixed-length-after-creation array — the engine-grade buffer type: length chosen at creation, elements read and written in place through ordinary readonly/writable borrows, no grow/shrink surface. `T[]` is a move type and participates in ownership and borrow checking exactly like every other move type; indexing borrows the element, `foreach` borrows elements, out-of-bounds indexing panics. (`Bytes` remains the dedicated byte buffer; `uint8[]` and `Bytes` interconvert only through **explicit, non-implicit** conversions (`Bytes::fromArray` / `->toArray`), copying in v1.0 — zero-copy views over either belong to the FFI/unsafe tier at Stage 40; the exact method surface is settled in the collections decision.)
- **Named collections** are the growable/structured family (the full inventory and naming are settled in decision 0092): `List<T>` (the everyday growable sequence and default workhorse), `Dictionary<K, V>` and `SortedDictionary<K, V>`, `Set<T>` and `SortedSet<T>`, `PriorityQueue<T>`, and `Deque<T>`. The naming rule is one axis — a bare name is the default (hash / insertion-ordered) collection, the `Sorted` prefix is the comparison-ordered variant — so no `HashMap`/`HashSet` alias exists. `Dictionary` and `Set` iterate in **insertion order** (PHP-familiar); the `Sorted` variants iterate by `Comparable` key/element. `Deque<T>` subsumes FIFO and LIFO, so `Queue<T>`/`Stack<T>` are not separate types. All join under the same `PascalCase<T>` naming and the §9.1 charter.
- **Bracket literals are collection literals, not arrays.** `[1, 2, 3]` and `["a" => 1]` are contextually typed by the expected type: `int[] $a = [1, 2, 3];` builds a typed array, `List<int> $l = [1, 2, 3];` a list, `Dictionary<string, int> $d = ["a" => 1];` a dictionary. Without an expected type (`let $x = [1, 2, 3];`), a sequence literal defaults to `List<T>` and a keyed literal to `Dictionary<K, V>` — the growable PHP-intuitive reading — with the element/key/value types inferred from the elements (vetoable default; the collections decision). `Set<T>` has no literal form in v1.0; use `Set::from([...])`. A bracket literal also has a **repeat form `[value; count]`** (decision 0102) — a runtime-`count`-length sequence of copies of `value`, contextually typed to `T[]` or `List<T>` (defaulting to `List<T>`) exactly like the element-list form. It is the only runtime-sized `T[]` constructor; `value` must be a Copy scalar or `string` in v1.0 (move-type fills await `Cloneable`), it is sequence-only (no `Set`/`Dictionary`), and a negative `count` panics.
- **Mixed-flow shapes are always valid Doria shapes**: `mixed[]`, `List<mixed>`, `Dictionary<string, mixed>` — never `array`. Stage 23 Slice 1 establishes those types, and Stage 23 Slice 3 implements the `dr_mixed` box needed to construct and use runtime mixed values in those shapes. `Doria\Std\Json`, the PHP bridge, and migration output use these.

---

## 5. Error handling: checked throw/throws (D8)

Full semantics for decision 0035's accepted direction:

```doria
class NotFoundError implements Error
{
    function __construct(string $message)
    {
    }
}

function loadUser(string $id): User throws NotFoundError, StorageError
{
    let $row = $db->find($id);      // $db->find declares `throws StorageError`
    if ($row == null) {
        throw new NotFoundError("no user {$id}");
    }
    return User::fromRow($row);
}

function handler(): Response
{
    try {
        let $user = loadUser("42");
        return Response::ok($user);
    } catch (NotFoundError $e) {
        return Response::notFound($e->message);
    } catch (StorageError $e) {
        return Response::serverError($e->message);
    } finally {
        $metrics->record();
    }
}
```

Rules:

- `interface Error` is built-in with a required readonly `string $message` property requirement (property requirements on interfaces land in the same stage, scoped to this need first).
- Only class types implementing `Error` may be thrown or listed in `throws`.
- **Checked propagation**: a call to a `throws`-declared function must be (a) inside a `try` whose `catch` arms cover every declared error type (covering = the arm type is the error class or a superclass/implemented interface), or (b) inside an ordinary callable whose own `throws` clause covers the uncovered remainder. The selected top-level `main` is the one exception to written contracts: when it omits `throws`, the compiler infers its exact uncovered escaping set. A written `main throws` remains accepted and checked. Errors escaping `main` are handled by the runtime as specified below.
- `catch (Error $e)` is the catch-all. Rethrow is plain `throw $e;`.
- `finally` runs on normal exit, thrown-error exit, and early `return`; it may not `return`, `throw`, `break`, or `continue` (avoids PHP/Java's swallowed-error trap; compile error).
- Lowering: `throws` functions return a hidden discriminated result in the native ABI (no unwinding — consistent with the abort-only panic policy and cheap for the engine). The PHP backend lowers to native PHP exceptions.
- `throw` is a statement in v1.0; expression-position `throw` (PHP 8 style) is a fast-follow.

**Errors escaping `main` — the caller of last resort.** `main` is called by doria-rt's entry glue (`dr_main`), so the runtime is the caller of last resort and its behavior is language-specified, not incidental:

- Because `throws` lowers to a hidden discriminated result (no unwinding), an error propagating out of `main` travels through ordinary returns, and drop elaboration runs `__destruct` at every scope boundary on the way out exactly as on the success path — files flush, sockets close, locks release. An escaping checked error is an *orderly, declared* failure; contrast panics, which abort with no cleanup and exit 101.
- `dr_main` then prints `error: <ClassName>: <message>` to stderr — the class name via a minimal type-name intrinsic (drop glue already carries per-type metadata; this is not reflection and must not grow into one) and the message via the `Error` interface's guaranteed readonly `string $message` — destroys the error value, and exits with status **70** (BSD `EX_SOFTWARE`). Never 101.
- The 70/101 split is machine-readable triage: a supervisor, orchestrator, or PHP frontend distinguishes "declared failure" (70) from "Doria bug" (101) without parsing stderr.
- Checked errors carry **no captured propagation path by default**: they are values and ordinary control flow, and path capture at every `throw` would tax exactly the hot paths the result ABI keeps cheap. Panics keep their source-aware `Call Path`; errors keep structured identities and messages. A dev-profile opt-in (path capture at throw sites under an environment flag) may be added later within the checked-errors decision's scope.
- `async function main` is permitted: the entry glue bootstraps the executor with `main` as the root task, and structured concurrency guarantees no orphan tasks remain when the root task completes with an error — child scopes have already awaited or cancelled their tasks before propagation continues. A synchronous `main` never starts the executor, so non-async programs pay zero async cost. Bootstrap details land with the async decision / Stage 38.
- `main`'s handler is the *process* boundary. The php-lib bridge is the *FFI* boundary with its own contract (§10.3): escaping checked errors become generated PHP exceptions and never terminate the host.

Panics (Section 3.6) remain entirely separate: not declarable, not catchable.

---

## 6. OOP completion (D17)

### 6.1 Inheritance and dispatch

```doria
open class Model
{
    open function save(): void throws StorageError { /* ... */ }
    function id(): string { /* ... */ }        // not overridable
}

class Post extends Model
{
    override function save(): void throws StorageError { /* ... */ }
}
```

- Classes are **closed by default**; `open class` permits subclassing. This is the Rust/Kotlin idea (inheritance as a deliberate API) in plain spelling, and it lets the compiler devirtualize aggressively — important for engine performance.
- Methods are non-virtual by default; `open function` creates a vtable slot; `override function` is mandatory at override sites (typo-proof).
- Single inheritance; construction order is parent-first. Allocation creates storage for the whole object, then the parent initializer/constructor chain completes before subclass property initializers run and before the remaining subclass constructor body executes. If the parent declares a constructor with required parameters, the subclass constructor must contain `parent::__construct(...)` as its first source-level action; lowering treats subclass property initializers as running after that parent call and before the rest of the subclass body.
- `internal` members are never inherited-visible — not even to subclasses. **Doria's member model is permanently two states: externally accessible by default, or `internal` to the declaring class.** `protected` is not deferred, not under evaluation, and never becomes Doria syntax; inheritance does not add a third visibility tier. If a subclass needs access to a parent's `internal` member, the parent must expose a deliberate accessible API instead.
- Upcasts implicit; downcasts via `$x is Post` narrowing and `match`; no unchecked cast spelling exists.

### 6.2 Interfaces

- Method requirements plus (from the Error work) readonly property requirements.
- Interfaces may extend multiple interfaces. Conformance is nominal via `implements`, checked at compile time.
- Default method bodies in interfaces: deferred to v1.x (traits cover reuse).

### 6.3 Traits

```doria
trait HasSlug
{
    writable string $slug = "";

    writable function refreshSlug(string $from): void
    {
        $this->slug = Slug::from($from);
    }
}

class Article
{
    uses HasSlug;
    uses Timestamps { touchedAt as internal; }
}
```

- Traits contribute properties and methods textually-by-semantics (flattened at class composition, monomorphized like generics — no runtime trait objects).
- Conflicts (two traits provide the same member) are a compile error resolved with PHP-spelled `insteadof` / `as` clauses inside the `uses` block; `as internal` may tighten surface.
- Traits may declare abstract requirements (`function render(): string;` with no body) the composing class must satisfy.

### 6.4 Property hooks

The planned escape hatch from SPEC §6, landing after classes are fully native:

```doria
class Temperature
{
    internal writable float $celsius = 0.0;

    float $fahrenheit {
        get => $this->celsius * 9.0 / 5.0 + 32.0;
        set (float $value) => $this->celsius = ($value - 32.0) * 5.0 / 9.0;
    }
}
```

PHP 8.4 hook spelling, adjusted to Doria's always-typed parameter rule; `get`-only hooks make computed readonly properties; `set` hooks require the property (or hook) to be writable-consistent.

### 6.5 Statics and constants

- **Static access is sigil-free for both properties and methods: `Message::age`, `Message::create()`.** Deliberate divergence from PHP's `Foo::$prop`. Doria's own rule is that declarations carry `$` and member access does not (`string $name;` → `$this->name`), and PHP's static sigil is an artifact: its parser cannot tell a constant from a static property without one, because PHP enforces no constant casing and permits dynamic member names. Doria resolves statically, enforces casing (§9.1), and has no dynamic member names, so the ambiguity the sigil solves does not exist. `Foo::$prop` is rejected with a remove-the-sigil fixit.
- **One member namespace per class.** A member name is unique across constants, static properties, instance properties, and methods; PHP's three separate namespaces become one. This is what makes sigil-free `Foo::x` resolve, and it matches nouns-are-properties: a name is data or an action, never both.
- **`self` is reserved**, in scope position (`self::MAX_DEPTH`, `self::age`, `self::create()`) and type position (`function withName(string $n): self`). Traits require it — a trait method referencing the composing class's constant or static has no other spelling. Recognized before name resolution like the intrinsics, so `class self` is rejected.
- **`parent::` generalizes** beyond §6.1's constructor chain to any parent implementation (`parent::save()`), which `override function` needs.
- **`static::` (late static binding) is rejected permanently.** `static` is already the member modifier, so `static::` would give one word two meanings — the `use`/`uses`/`with` sin. Doria needs no LSB: statics are not virtual, `open`/`override` makes instance virtuality explicit, and devirtualization is a goal. Fixit: `static::` → `self::`.
- Static properties follow readonly/writable rules; writable statics are per-process globals and are rejected in `Sendable`/`Shareable`-checked concurrency contexts later. Writing a writable static from a constructor is ordinary mutation, never constructor init-access — the Stage 19 ownership-mechanics rules govern `$this` only.
- `const NAME = expr;` class constants and namespace-level constants; const expressions are compile-time evaluated over literals, arithmetic, and other consts (this defines the first compile-time evaluation tier, which attributes will reuse).

### 6.6 Data classes and value objects

- Doria adds **no** `record`/`struct`/value type: the class is the owned record type (the Copy-vs-move decision), and an immutable class already *is* a value object.
- Structural behavior — equality, hashing, `toString`, copy-with-changes, destructuring — is **opt-in derives** through the attributes and compile-time codegen decision, optionally sugared as `data class`; identity equality stays the default.
- A **DTO** is a data class plus a serialization derive: a framework concern, not a language type. **DAOs** are ordinary classes. Single-field value objects (`Money`, `Sql`) are the newtype work.
- Design is settled in decision 0087; implementation is deferred behind the attributes/codegen and interfaces/`Cloneable` decisions and does not compete with the Stage 21 borrow checker.

---

## 7. Namespaces, source organization, closures

- Implement decisions 0028 and 0117: `namespace App\Services;`, file-scope `use ... as ...`, string-literal same-package include-once `include`, structured `declare`, and compile-time package source discovery through Baton's public `autoload` mappings. Multi-file compilation units are the enabler stage for everything package-shaped; Doria has no runtime source autoloader.
- Name resolution: a compilation invocation takes an explicit source inventory or versioned JSON build plan from Baton; symbols resolve by fully qualified name; unqualified names resolve through explicit imports, the current namespace, then the edition prelude, after lexical and intrinsic handling. Duplicate fully qualified symbols across files or packages are compile errors identifying both packages and sources. Baton discovers and resolves package inputs, while `doriac` owns every declaration and semantic decision.
- Decision 0117 fixes hybrid strict layout: namespace directories and externally accessible type filenames match exactly, one primary externally accessible type occupies a file, related `internal` helpers and function/constant bundles remain legal, and every active source file is checked. Main, development, and generated source scopes stay distinct; only a selected binary entry file may contain top-level executable statements.
- First `declare` keys (each rejected until implemented): `declare(overflow: "wrapping");` (module-local, D4 relaxation for engine hot paths), `declare(unsafe: "deny");`.
- **One word, one meaning.** PHP overloads `use` with at least three jobs (namespace import, trait composition, closure capture). Doria splits them permanently: `use` is namespace import/alias only, `uses` is trait composition only, and `with` is closure capture only. The parser, diagnostics, editor grammars, and the migration converter (`use (...)` capture clauses rewrite to `with (...)`) must all treat these as three distinct keywords; none of the three is ever accepted in another's position.
- Closures (D10):

```doria
let $double = fn(int $x) => $x * 2;                  // typed parameter, inferred return
let $base = 10;
let $adder = fn(int $x) with ($base) => $x + $base;  // arrow, explicit capture
let $block = function (int $x): int with ($base) {   // block, same capture
    return $x + $base;
};
```

Closure parameters are declarations, so every parameter requires an explicit type. Doria does not infer omitted parameter types for anonymous functions, arrow functions, callbacks, collection methods, property hook setters, or any other function-like form. A closure's return type may be inferred for arrows and may be declared for anonymous functions, but parameter types are never optional.

The capture clause is spelled `with`, never PHP's closure `use`. Both arrows and block closures list every referenced binding from an enclosing lexical scope. `with ($base)` takes a readonly borrow; `with (writable $counter)` takes an exclusive writable borrow; `with (take $connection)` moves the value into the closure. Copy, readonly, writable, and Move bindings have no implicit-capture exception. A closure with no surrounding-local dependency omits `with`; Doria does not require or recommend `with ()`. Changing an arrow into a block closure preserves its capture list and ownership modes.

A closure holding borrows is itself borrow-bound, so it cannot outlive or escape the captured variables' scope; the borrow checker rejects invalid escape and the diagnostic suggests `take` when ownership transfer is appropriate. Closures are Move values with structural `function(int): int` types; `Callable<...>` is not adopted. Decision 0120 owns explicit capture lists, and Decision 0121 completes the model: invocation is readonly, writable, or `once`; `$this` uses explicit readonly/writable capture; function types preserve parameter ownership and `throws`; named and bound callable references remain deferred; and the logical two-word carrier uses a lean descriptor.

---

## 8. Compiler and runtime architecture plan

### 8.1 Pipeline evolution

```text
source → lexer → parser → AST
      → name resolution (namespaces, use, include)
      → semantic analysis + type checking (HIR)
      → readonly/writable surface checking
      → definite-initialization & flow analysis (narrowing, returns, ctor init)
      → Doria IR (checked, typed, desugared)
      → MIR (SSA-ish control-flow graph: ownership/move analysis, non-lexical
             borrow checking, drop elaboration placing `__destruct` calls,
             monomorphization, exhaustiveness lowering, panic edges)
      → backend (Cranelift dev | LLVM release | PHP compat | wasm later)
```

- The private `NativeSmokeModule` is retired in Phase A, replaced by the real MIR layer. MIR is the permanent native-oriented IR SPEC §13 anticipated. Until v1.0, MIR is not a stable format.
- Full path-sensitive control-flow analysis (returns on all paths, definite readonly-property initialization on all constructor paths, null narrowing) is one shared dataflow framework built once in Phase A and reused everywhere — it replaces the "final statement must be return" early rule.
- **Semantic type-model extensibility (acceptance criterion for the TypeId/TypeKind work underway in Phase A):** the internal representation must remain able to grow into fixed-width numerics (present), callable/function types, generic instantiations, opaque foreign types, buffers/slices/views, and possible future value parameters — never an internal model that collapses all integers or floats into one host representation.
- **MIR extension points:** MIR's boundaries must admit typed intrinsics, vector operations, calls to optimized native kernels, and an *optional* future domain-specific lowering stage (numerical/accelerator work is a named future consumer) — without any of those entering current MIR scope, the source AST, or the PHP backend. Public communication calls all of this simply the **Doria IR**; layer names are internal.

### 8.2 Dual backend (decision 0012, made concrete)

- **Dev compiler profile** (direct `doriac compile` / `doriac run` while Baton is unavailable; later Baton default `baton build` / `baton run` selects the same profile): Cranelift, fast compile, overflow checks on, debug info.
- **Release compiler profile** (direct `doriac compile --release` while Baton is unavailable; later `baton build --release` selects the same profile): LLVM (via `inkwell`), optimizations, overflow checks still on per D4. Exclusive borrows give both backends `noalias`-grade optimization license, the same performance story as Rust.
- Identical Doria-visible semantics across profiles is a tested invariant: the differential test suite runs every `examples/native` program under both backends plus the interpreter and asserts identical stdout/exit status.
- **Debug/interpreter backend** (SPEC §1's listed backend) is implemented in Phase A as a direct MIR interpreter. It is the semantic oracle for differential testing and makes the test suite backend-independent — this is the single highest-leverage correctness investment in the plan.

### 8.3 doria-rt (D18)

A Rust `crates/doria-rt` static library, introduced as a minimal runtime/panic foundation in Stage 12 and expanded by later runtime stages, is linked into every native binary:

- Allocator (system malloc initially; pluggable arena hooks reserved for the engine later).
- Drop-glue dispatch, `SharedReference<T>`/`WeakReference<T>` refcount and weak-resolution machinery, and the writable family's per-allocation access state (readonly-family allocations carry none — §3.3).
- String/Bytes/List/Dictionary/Set intrinsic implementations (refcounted immutable string buffers, owned growable collection buffers, hashing).
- Runtime-outcome transport, source-aware Doria call-path capture, and process entry glue (`dr_main` wrapping user `main`).
- stdout/stderr/stdin, basic clock, environment access — the syscall surface the stdlib wraps.

Record 0044's ABI review must evaluate, as named design cases before the native object representation freezes: externally owned memory (buffers doria-rt did not allocate), custom deallocation callbacks, alignment requirements, and pinned/non-moving memory for interop. The ownership model already guarantees the hard part — stable addresses, deterministic release, no movable-GC assumption — so these are representation questions, not model changes.

All symbols `dr_`-prefixed, internal ABI, versioned in lockstep with the compiler.

### 8.4 Diagnostics

Decision 0108 is authoritative. Every diagnostic carries a stable code, severity,
kind, Title Case title, source-identified primary/secondary labels, and optional
explanation, repeated notes/help, applicability-classified multi-edit fixes,
cause identity, documentation metadata, and developer details. Human and concise
CLI output goes to stderr; schema-version-1 JSON goes to stdout without ANSI.
The LSP and website consume the structure rather than parsing terminal prose.
Backend, external-tool, and internal failures retain full developer detail
without exposing raw output by default. Decision 0109 extends the same
compiler-owned model to runtime outcomes: built-in panics have stable `P`
codes, source-aware labels, `Where`, `Why`, a Doria-only `Call Path`, and
status-101 abort-without-cleanup semantics.

Architectural goal, standing from now: **CLI commands wrap reusable compiler
services** (in-memory parse/check, diagnostics, module compilation, interpreter
execution) rather than owning compiler behavior directly, so future REPL,
notebook, and incremental tooling never needs a second frontend. Incremental
compilation itself stays deferred.

### 8.5 Testing strategy (all phases)

- Unit tests per compiler pass; integration tests per stage in `crates/doriac/tests` (current pattern).
- Differential suite: interpreter vs Cranelift vs LLVM on every executable example.
- UI-style diagnostic snapshot tests (expected diagnostics per fixture file) so error messages are versioned.
- The PHP backend keeps its own snapshot tests but is never the proof of semantics (unchanged policy).
- Fuzzing the lexer/parser with `cargo-fuzz` starts in Phase B (cheap, catches panics early).

### 8.6 Platform tiers

Tier-1 targets from Phase A onward: **Linux, macOS, and Windows** (x86_64; aarch64 on Linux/macOS). This is a consequence of product 5, not an afterthought: "portable TUI" is only true if Windows is exercised continuously, so the CI matrix builds doria-rt and runs the differential suite on all three operating systems starting at Stage 12, and every doria-rt syscall-surface addition (I/O, clock, env, term) lands with its Windows implementation in the same stage — never "Unix now, Windows later." Cranelift and LLVM both support these targets; the PHP compat backend is platform-neutral.

---

## 9. Standard library plan

**Stdlib philosophy: batteries included (the Odin instinct).** Common needs are covered out of the box — curated, charter-named, and cohesive — so developers don't go hunting for a third-party library to do ordinary work. The language stays thin; the standard library carries the weight. This never means everything ships in the first release; it means the *default answer* to "where's the X library?" is "in std," added deliberately over time.

Two layers, both written in Doria as early as possible (self-hosting on-ramp): **core** (no I/O, always available) and **std** (hosted). The comprehensive surface — every core companion, interface, collection, and free function, and every `Doria\Std\*` module with its members — is catalogued in the [standard-library reference](stdlib-reference.md). This section states shape and direction; the reference is the inventory.

### Core (no I/O, always available)

- **Primitive companions** — `Int`/`Int8`.../`Float`/`Bool`/`String` APIs (`Int::parse`, `Int::toFloat`, `Int::wrappingAdd`, ...) plus `Option`-free nullable helpers (there is no `Option` type; `?T` and helpers do the work).
- **Value interfaces** — `Comparable<T>` (over the core `Ordering` enum), `Equatable<T>`, `Hashable`, `Displayable`, `Cloneable` (public from Stage 35), `Error`.
- **Shared ownership** — `SharedReference<T>` / `WeakReference<T>` / `WritableSharedReference<T>` / `WritableWeakReference<T>`, with the `ReadonlySharedReferenceAccess<T>` / `WritableSharedReferenceAccess<T>` access objects (§3.3).
- **Iteration** — `Iterable<T>` / `Iterator<T>`; user conformance lands at Stage 35 (built-in collections use compiler-internal iteration earlier).
- **Ranges and `math` basics.**
- **The built-in free-function layer** — regularized `snake_case` capabilities that do not naturally belong to one type (`read_line`, `get_time`, `function_exists`, ...). Type-coupled vocabulary belongs to the type companion; Doria has no public string-specific free-function family.

### Standard library modules (`Doria\Std\*`)

Rooted at the reserved `Doria\Std` namespace (Decision 0117); per-module surface in the [reference](stdlib-reference.md).

- **`Doria\Std\Io`** — the post-Stage-29 `File`/stream objects (the Stage 17 text free functions are *language* intrinsics, not this module).
- **`Doria\Std\Fs`** — filesystem and path operations.
- **`Doria\Std\Env`** — environment variables. **`Doria\Std\Process`** — exit code, pid, executable path (command-line arguments arrive via `main(List<string> $args)`, decision 0099).
- **`Doria\Std\Time`**, **`Doria\Std\Random`**.
- **`Doria\Std\Json`** — drives enum/match/mixed ergonomics and the PHP bridge.
- **`Doria\Std\Net`** (TCP first), later **`Doria\Std\Http`**.
- **`Doria\Std\Data`** — DDO (see below).
- **`Doria\Std\Term`** — the portable terminal layer for product 5 (see "The terminal layer" below).
- **`Doria\Std\Math`** — game/graphics geometry math (see below).

### The terminal layer — `Doria\Std\Term` and `Console`

`Doria\Std\Term` is **capability-based** on the crossterm model — raw mode, non-blocking input decoded to payload enums (`KeyEvent::Char(string $char)`, `KeyEvent::Up`, resize events), cursor positioning, styling/colour, screen size and clearing — with per-platform backends (Windows Console API / VT processing; Unix termios + escape emission) hidden behind the API. Raw ANSI is a Unix-backend implementation detail, never the public surface. The canonical high-level API is the **`Console` class** — a static facade so the user never checks what macOS/Linux/Windows supports — covering terminal info (size, interactivity, colour capability), screen (clear, title; alternate screen later), cursor (position, move, show/hide), styled output, and input (blocking `readKey`, non-blocking `pollKey`, resize events); the full method inventory is in the [reference](stdlib-reference.md) and settled in the Console/terminal decision (TermUtil as reference input; a source-derived mapping note accompanies this plan). Raw mode is entered through an ownership guard whose `__destruct` restores the terminal on every structured exit — RAII closing the classic wedged-terminal bug (an abort-only panic in raw mode runs no cleanup per record 0081; a panic-hook restoration is possible future work). `Console`'s design ancestor is the designer's TermUtil PHP library, improved upon, not copied (TermUtil is ANSI-powered by definition; `Console` is capability-based by definition). Two binding constraints from that source review: **no `Doria\Std\Term` public type may carry escape sequences or platform encodings** in its values or API (TermUtil's `Color` enum was ANSI-backed — the exact pattern that made it unportable; Doria's `Color` means colours, and each backend renders them), and the terminal is **stateless** — there is no separate `ScreenBuffer` std type; a diffing/back-buffer renderer is userland (the ported engines are the widget layer, per §14), not a std primitive. The `Console` name is reserved in the stdlib namespace from now.

**Formatted I/O — the v1.0 minimal set (record 0074).** Doria ships a deliberately small, PHP-familiar formatted-I/O surface in the free-function layer. PHP's broader text-processing catalogues are comparison inventories, not a surface to import wholesale: the String API completeness audit classifies every core string, mbstring, and grapheme entry by its Doria owner, while Andrew's Decision 0103 completeness review decides which genuinely core omissions belong in v1.

- **`sprintf` and `printf`** are compiler-known functions whose first parameter is a literal `string $format`; the remaining operands are typed and checked against that format rather than declared through an untyped userland variadic parameter. `sprintf` returns `string` and `printf` returns `void`. The compiler verifies specifier/argument count and types — `%d` against a non-integer is a compile error, not a runtime surprise. This is Rust's checked-`format!` guarantee delivered through a PHP spelling, and it is the reason format strings are safe to have at all in a language with Doria's discipline. v1.0 specifier subset: `%s` (any display-convertible value, §4.6), `%d` (int/uint family), `%f` with `%.Nf` precision (float family), `%x`/`%X`/`%o`/`%b` (integer bases), width / `-` left-align / `0` zero-pad flags, and `%%`. Everything else (`%e`, `%g`, positional `%1$s`, dynamic format strings) is deferred with a specifier-not-supported diagnostic. `printf` returns `void` — PHP's returns-byte-count behavior is dropped as charter noise.
- **`read_line(string $prompt = ""): ?string`** writes the prompt to stdout exactly as supplied (adding no newline), flushes stdout, then reads one line from stdin, strips the trailing newline, and returns `null` at EOF (never PHP's `false`). It is one function with an optional parameter, not an overload pair: `read_line()` is exactly `read_line("")`, and the pre-read flush happens even for the empty prompt, so an earlier `echo "Name: ";` is visible before the program blocks. The prompt is emitted under redirection, is never conditional on terminal interactivity, and is not line editing, history, or completion. Richer stdin APIs live on the post-Stage-29 `Doria\Std\Io` stream types. The free-function family is charter-uniform verb_noun throughout: `read_line`, `read_file`, `write_file`, `append_file`, `write_stderr` — **not** PHP's `readline`, which is a `strlen`-style fusion (and, fittingly, ships from ext/readline, absent on Windows PHP — the exact gap Doria closes). `append_file` is specified now and implemented in Stage 23 Slice 2; it does not change truncate-only `write_file`. `sprintf`/`printf` are *not* counterexamples: they are whitelisted as industry-universal single lexemes (C, PHP, Go, Java all spell them this way), the same whitelist tier as `id` and `json`; `readline` has no such cross-language standing and does not qualify.
- **`read_file(string $path): string` is the text tier of a deliberate three-tier I/O family — not the whole story.** Because `string` is immutable UTF-8, `read_file` is *by definition* the text-file function: it validates UTF-8 on read and has defined failure behavior — invalid bytes never enter a `string` (the type's invariant is load-bearing for the whole language). `write_file` creates or truncates; the additive `append_file` spelling landed in Stage 23 Slice 2 without changing that contract. The binary tier is `read_file_bytes(string $path): Bytes`, `write_file_bytes(string $path, Bytes $contents): void`, and `append_file_bytes(string $path, Bytes $contents): void`; these whole-file operations borrow `$contents` and preserve exact bytes. The streaming tier — `File`/stream objects with RAII close via `__destruct`, buffered readers, seek — lands after checked errors exist (post-Stage 29), because serious file APIs want `throws`, not panics. **Failure-semantics migration (records 0075 and 0091):** until Stage 29, I/O failures in these free functions panic with a clear message; at Stage 29 they migrate to declared `throws` signatures. A closed stdout or stderr pipe during ordinary program output is the permanent exception: it exits immediately with status 0, emits no panic or trace, and is never thrown. Panic reporting remains fatal with status 101 even when stderr is unavailable. This signature change and carve-out are planned, recorded, and announced, never a surprise.
- **PHP-spelling fixits and migration mappings**: the unknown-function/unknown-operator diagnostic recognizes implemented Doria replacements such as `readline` → "did you mean `read_line`?", while the migration catalogue maps PHP's string functions to decision 0103's canonical companion calls (`strcasecmp` → `String::compareIgnoreCase`, `str_starts_with` → `String::startsWith`, and so on). A fixit activates only when its Doria target is executable; documentation does not send users from one unknown call to another. Other examples include `static::` → `self::` (§6.5), `Foo::$prop` → `Foo::prop`, and `instanceof` → `is`. The mapping is shared with `doriac migrate php`.
- **`print` is not included — ever.** It is `echo` with a vestigial return value; two spellings for one construct is exactly the PHP redundancy Doria eliminates (`use`/`uses`/`with`, no `object`). `echo` is the one output spelling; the name `print` is rejected with a use-`echo` diagnostic so it can never drift into userland-looking-like-builtin.
- **`sscanf` is deferred, not spelled.** Its shape — runtime-format-determined result arity/types, or by-reference out-parameters — fundamentally fights static typing (Rust has no scanf either, and Doria has no tuples to receive one). v1.0 parsing is `Int::parse`/`Float::parse` returning `?T`, the `str_` functions, and `match`/narrowing; a compile-time-checked scan design may be revisited post-1.0 in record 0074's follow-up.

**Batteries-included game/graphics math (the geometry-math decision, unauthored).** Game development is a first-class Doria application, so the common math a game or renderer needs ships in std rather than being every project's first wheel to reinvent: `Vector2`/`Vector3`/`Vector4`, `Quaternion`, `Euler`, `Matrix3x3`/`Matrix4x4` (a "competent" 3D library without matrices isn't one — transforms need them), plus lerp/clamp/easing scalar helpers alongside the existing `math` basics. Three foundational constraints make this fast rather than merely present:

- **Math types are built-in Copy value types with inline layout** — they join the `string`/ranges/Copy-enum tier, *not* the class tier. A `Vector3` as a heap-allocated move type would make vector arithmetic unusable (moves and `->clone()` on every hot-loop operation); as an inline Copy aggregate it costs what three floats cost. D3 is untouched — there is still no *user-defined* `struct` in v1.0; these are stdlib-defined, compiler-known types, and payload enums already prove the compiler supports inline Copy aggregates.
- **Arithmetic operators on math types are compiler-known**, exactly as they are for `int` and `float` — `$a + $b`, `$v * 2.0`, `==` — which does **not** open user-defined operator overloading (§14 unchanged; the general operator/numerical protocol stays parked in Appendix A). Beyond operators, the API follows the charter: `$v->length` and `$v->normalized` as properties, `$v->dot($other)` / `$v->cross($other)` as methods.
- **`float32` variants and SIMD**: the SIMD-direction decision treats `Doria\Std\Math` as its first consumer — `Vector3`/`Vector4` over `float32` map directly onto SIMD lanes, so the geometry-math and SIMD layout decisions are made together.

Implementation lands as Stage 47 in Phase J, ahead of the engine-seed flagship demo; the *constraint* on Stage 19's object-layout work applies now.

**DDO — Doria Data Objects (the DDO decision, unauthored; supersedes the 0007 sketch).** The batteries-included database layer: PDO's good idea (one API, many databases, a driver model) with PDO's mistakes fixed by Doria's existing decisions rather than by options and modes. The improvement charter:

- **checked errors always** — there is no silent mode, no error-mode setting at all; every fallible operation declares `throws` (PDO needed until PHP 8.0 to make exceptions the default).
- **Native prepared statements, parameterized-only by default** — no client-side emulation (PDO's emulated prepares enabled charset-based injection edge cases).
- Injection safety is layered, strongest guarantee at the bottom: (1) **runtime floor** — the driver parameterizes bound values always, so even a dynamic query is injection-safe when values are bound; (2) **compile-time convenience** — for literal SQL, placeholder arity/names are checked at compile time against bound arguments, reusing the literal-format machinery `sprintf` proved (this checks *placeholders*, not SQL — it is not a SQL parser, knows no schema, and makes no query-correctness promise; the framing must never overclaim); (3) **provenance layer** — an `Sql` newtype over `string` (a trusted-string wrapper, the sqlx/trusted-types pattern) is what the connection API accepts, so a bare `string` of user input is a *type error* at the call site, not a runtime hope. `Sql` is constructible from a compile-time-literal (placeholder-checked) or through explicit, greppable, review-visible escape hatches (`Sql::fragment(...)`); `Sql` composes with `Sql` to stay `Sql`, while a `string` can enter only through a bound placeholder — so `$base . $userInput` does not typecheck and the dynamic-query hole shrinks to named, audited joins. Honest limit recorded: the newtype tracks *provenance* (trusted source), not *correctness* (valid against a schema) — schema-aware checking, if ever wanted, is a `baton`-level tool reading migrations, never a std feature depending on a live database. Doc-comment-carried SQL types (Psalm `literal-string` style) are rejected — provenance lives in the type, not in unchecked comments.
- **Typed fetches** — an `int` column comes back as `int`, never `"42"`; nullable columns are `?T`, non-null columns are `T`; fetch shapes are a small set (row into a typed class, `Dictionary<string, mixed>`, scalar, column list, and a lazily-streamed cursor of typed rows for large results that must not buffer the whole set) — no `FETCH_BOTH` duplication zoo, no stringify option.
- **One binding story** — named placeholders map onto named arguments; there is no by-reference `bindParam` (nothing in Doria to express it with, happily).
- **Typed connection configuration** with named arguments, not stringly DSNs.
- **Capability-based drivers** on the `Console` philosophy: driver quirks never leak through the public surface; capabilities are queryable.
- **RAII transactions** — `begin` returns an owned `Transaction`; an uncommitted transaction rolls back in `__destruct` (and on panic the server rolls back the dropped connection anyway, so the abort-only model is safe here); `commit` consumes the transaction so post-commit use is a compile error.
- **Decomposed API** (`Connection`, `Statement`, `Transaction`, typed rows), not PDO's god object.
- Prerequisites: checked errors (Stage 29), `Doria\Std\Net`, and newtype/generics support for the `Sql` provenance type (post-Stage-25, comfortably before the Stage-29 floor); stage assigned when scheduled; not 1.0-gate-blocking (the gate's flagship is the TUI demo) but a headline of the batteries-included story.
- Row↔class mapping needs compile-time, attribute-driven codegen — Doria has no reflection — so the attributes/codegen decision must treat derive-style codegen as a design case; the async decision treats DDO connections as a design case for async and thread-affinity (connections are not `Sendable`); the checked-errors decision treats the driver-agnostic error taxonomy (SQLSTATE + driver payload) as a design case.

`foreach (collection as ...)` uses compiler-internal iteration machinery in Phase D for built-in collections. The public `Iterable<T>` / `Iterator<T>` protocol that makes user types iterable lands with interface conformance in Stage 35.

### 9.1 Naming charter and the built-in/userland boundary (D19)

PHP's standard library is the cautionary tale this charter exists to prevent: `strlen` vs `str_replace` vs `nl2br`, camelCase methods beside snake_case functions, and needle/haystack argument order that flips between functions. Doria's built-in surface follows one law, enforced by API review and a `doriac` lint over the stdlib:

- **Casing**: built-in **free functions** are `snake_case` (`read_line`, `get_time`, `sprintf`). **All member-style APIs are camelCase** — standard-library members and companion/type APIs included (`Int::wrappingAdd`, `Int::parse`, `String::startsWith`, `$s->isEmpty`). Userland free functions are camelCase (see the boundary below). `__construct`/`__destruct` keep their PHP spellings as keywords-in-disguise. Classes, interfaces, traits, enums, and enum cases are `PascalCase`. Constants are `SCREAMING_SNAKE_CASE`. Type parameters are single capitals (`T`, `K`, `V`). **Namespace segments are `PascalCase` with acronyms folded** — `Doria\Std\Io`, `Doria\Std\Http`, `Doria\Std\Json`, and a hypothetical `Doria\Orm` — never `IO`/`HTTP`/`ORM`.
- **Canonical member-casing examples (normative; preserve these spellings):** `Int::wrappingAdd`, `$s->isEmpty`, `$response->retryAfter`, `$repo->findById(...)`, `$request->tenantId`. Member-style APIs are camelCase on both the built-in and userland sides; these exact spellings are normative exemplars guarded by CI (`scripts/check_docs_authority.php`) and must never be renamed, reformatted, or converted to snake_case in any future edit of this document.
- **No contractions**: `length` not `len`, `read_line` not `readline`, and `String::compareIgnoreCase` rather than PHP's `strcasecmp`. Whitelisted abbreviations are only those more recognizable than their expansions (`id`, `min`, `max`, `io`, `http`, `json`, `utf8`) plus industry-universal single lexemes that transcend any one language's spelling (`printf`, `sprintf`). Whitelisting is explicit and documented; PHP-only historical spellings do not qualify.
- **Symmetric pairs**: conversions always pair as to-X / from-X / try-from-X in the casing of their context (`toFloat` / `tryFrom` as members; `str_`-family spellings in free functions); lifecycle verbs pair predictably (`open`/`close`, `push`/`pop`, `add`/`remove`, `intoRaw`/`fromRaw`).
- **Predicates** read as questions: `is`/`has`/`can` prefixes (`isEmpty` as a member; membership follows decision 0113's `contains` law, so a map spells it `containsKey`; `is_`/`has_` in free functions) — and per SPEC §6's nouns-are-properties rule, argument-free ones are properties (`$s->isEmpty`), never `get`-prefixed methods.
- **Uniform argument order**: the subject always comes first (it is `$this` on methods); options and callbacks come last. No needle/haystack roulette.
- **One name per concept** across modules: it is `count` everywhere (never `size` or `length` for collections), `contains` everywhere.

**The free-function boundary — and why it lives only there.** A member never needs casing to declare its provenance: the receiver already does that work. In `Int::wrappingAdd(...)`, `String::startsWith(...)`, or `$s->isEmpty`, the receiver identifies the owning type, so member-style APIs are camelCase. Type-coupled vocabulary belongs to that companion. A free function is reserved for a capability without one natural owning type; built-ins use snake_case (`read_line`, `get_time`, `function_exists`) while **userland free functions are camelCase by convention** (`normalizeTitle($post)`). Doria's docs, examples, and tooling do not teach snake_case for userland free functions. Enforcement:

- All documentation, SPEC examples, `baton new` templates, LSP snippets, and generated code (`#[Derive(...)]` members) write userland declarations in camelCase; this plan's own examples model it (`loadUser`, `findById`), and conformances to built-in interfaces always keep the interface's member spelling (`compare`, `toString`).
- A default-on lint gives a gentle, silenceable hint when userland declares a snake_case **free function** ("this reads as a Doria built-in; camelCase is the userland convention") — encouragement, not an error, silenceable per-declaration and per-module. Methods are exempt: the receiver already carries provenance, so there is no member boundary to protect.
- `function_exists("name")` is a compile-time predicate usable in top-level `if` to conditionally declare a function. This is the sanctioned collision/polyfill mechanism: guarded declarations may adopt the built-in's snake_case name because they deliberately stand in for one (e.g. back-filling a newer stdlib function on an older Doria); outside such a guard, userland free functions stay camelCase. `function_exists` is const-evaluated — there is no runtime symbol table.
- The generated PHP FFI stubs mirror the exported Doria class's own casing, so a `#[PHPExport]` class written in charter-compliant userland camelCase lands in PHP looking like idiomatic PSR code — a free win for the bridge.

Every stdlib decision record cites this charter, and `baton fmt` plus the stdlib lint enforce it mechanically.

Stdlib API style follows SPEC §6's nouns-are-properties rule and the collection method surface is settled in decision 0100 (List: `add`, `insertAt`, `removeAt` returning the owned element, `pop`, `contains`, `first`/`last`, `count` property, `isEmpty` property, and Decision 0121's Stage 30g-only `map`/Copy-only `filter`/writable-accumulator `reduce`; Dictionary: `get` returning `?V`, `set`, `remove` returning `?V`, `containsKey`, `keys`, `values`; Set: `add`, `remove`, `contains`, `union`, `intersect`, `difference`). No other collection receives the Stage 30 algorithms. Reads on `$l[i]`/`$d[k]` assert presence and panic on absence; the `?T` methods are the safe path; mutators return `void`, and userland fluent APIs stay a 0088 capability. The Stage 23 typed-array `T[]` surface is `length`, indexed read/write, and iteration; slicing remains a separate future addition. The rest of the named-collection family (`SortedDictionary`, `SortedSet`, `PriorityQueue`, `Deque` — the inventory settled in decision 0092, the surface in 0100) follows the same charter.

---

## 10. PHP interop: the four products (D13)

### 10.1 Doria → PHP compat backend (exists)
Keeps growing opportunistically for migration/debugging; never gates a language feature. Features PHP cannot express lower where practical or emit unsupported-feature diagnostics (unchanged policy).

### 10.2 PHP → Doria migration (`doriac migrate php`)
Phase I product, per SPEC §12: conservative output, diagnostics for dynamic PHP (variable variables, `eval`, magic methods, loose comparisons become explicit conversions or `mixed` + TODO diagnostics). PHP arrays convert to valid Doria shapes only: `List<mixed>` for list-shaped arrays, `Dictionary<string, mixed>` for associative ones, tightening to precise `T[]` / `List<T>` / `Dictionary<K, V>` where docblocks or inference allow; a PHP `array` type hint is never emitted as Doria — it converts with a diagnostic explaining the §4.9 model. Architecturally separate crate `crates/doria-migrate` with its own PHP parser (use `mago`/`php-parser-rs` class of dependency; do not touch the Doria parser).

### 10.3 The strategic product: `baton build --php-lib`

Baton builds a Doria library into something a running PHP application calls natively, using doriac as the compiler underneath:

```doria
namespace App\Native;

#[PHPExport]
class ImageResizer
{
    function resize(Bytes $input, int $width, int $height): Bytes throws ResizeError
    {
        // hot-path native code
    }
}
```

```bash
baton build src/native --php-lib --out build/app_native
# emits: build/app_native/libapp_native.so
#        build/app_native/php/ImageResizer.php   (generated FFI stubs)
```

```php
<?php // in the existing PHP app
use App\Native\ImageResizer;              // generated stub, feels like a normal class
$resizer = new ImageResizer();
$out = $resizer->resize($bytes, 800, 600); // dispatches through FFI into native Doria
```

Design:

- Exported surface restricted to a bridgeable type set: numerics, `bool`, `string`, `Bytes`, `?T` of those, `T[]`/`List`/`Dictionary` of bridgeable types, and `#[PHPExport]` classes.
- **Bridge handles are internal ABI, not a Doria ownership family.** `#[PHPExport]` class instances cross the boundary as transport-neutral **opaque bridge handles**: the bridge owns or retains the underlying Doria instance per the bridge ABI, and the generated PHP stub holds its handle and releases it from its PHP `__destruct`. A bridge handle is an internal ABI value, never a Doria source-level value — it is not a `SharedReference<T>`, `WeakReference<T>`, `WritableSharedReference<T>`, or `WritableWeakReference<T>`, and **no conversion exists in either direction between bridge handles and either Stage 25a ownership family** (§3.3's families are disjoint, and the bridge is outside both). The handle contract may use internal reference counting where several foreign wrappers must retain the same native object; that mechanism does not make the handle a public `SharedReference<T>`. The bridge invokes exported methods according to their declared receiver modes — readonly methods with readonly access to the bridge-owned instance, writable methods with exclusive writable access — so **exported `writable function`s remain legal and `#[PHPExport]` does not make a class readonly**. Exact control-block layout, reentrancy, wrapper duplication, refcount operations, callback behavior, and writable-call mechanics belong to the future php-lib bridge decision and Stage 41.
- **Export is metadata, not visibility.** `#[PHPExport]` never changes Doria accessibility, adds no modifier, and is not a third visibility state — the exported surface is the class's externally accessible, bridgeable members, and `internal` members never cross the boundary. Unsupported signatures are compile-time errors. Diagnostics speak Doria's model and never suggest `public`/`protected`/`private`.
- `throws` errors surface as generated PHP exception classes. An error escaping an exported function must surface as that exception and never terminate the host PHP process — the bridge is an FFI boundary, not a process boundary. Panics are the documented exception: under v1.0's abort-only policy a Doria panic aborts the hosting PHP worker (exactly as a crashing native extension would), which is why exported surfaces should prefer `throws` APIs over panicking ones.
- Transport: **FFI is the bootstrap transport, not the product.** Stage 41 ships a versioned C-ABI bridge (sized types, opaque handles, pointer+length views, status/result returns — no cross-language unwinding, no Zend structures, no Doria object layout exposed) plus generated PHP ≥ 8.0 `FFI` stubs (zero build tooling on the PHP side). A generated Zend-extension transport (`--php-ext`) is the **intended production transport** later; both transports consume the same higher-level bridge contract so FFI never accidentally defines the permanent semantics, and that same contract must remain reusable by the deferred embedded-host product (d).
- Threading: v1 bridge is single-threaded per PHP request (matches PHP's model). Recorded now as a standing bridge invariant feeding the async decision: **a PHP runtime context and its values belong to a designated thread**; `Sendable`/`Shareable` are never permission to move PHP-runtime-affined values across threads. Boundary concurrency means validating and copying/transferring typed data in, running native work on Doria workers, and returning results to the PHP-owning thread.
- This product plus `Doria\Std\Json`/`Net` also covers the sidecar pattern (Doria service, PHP client), but the in-process bridge is the headline.

### 10.4 Product (d): native Doria → embedded PHP host (deferred)

The reverse direction — a native application hosting PHP as a first-class scripting language (the Lenga engine pattern: PHP gameplay scripts over a native core; also the port pattern's endgame for Sendama/Ichiloto) — is part of the strategy but **implementation-deferred**: no stage exists for it in this plan and none may be added until the product-(c) bridge contract is stable and the designer separately approves it. What this plan does now is refuse to foreclose it: Stage 41's ABI and ownership design must be transport- and direction-neutral (§10.3), and the eventual Doria-facing host API surfaces typed concepts (`PhpRuntime` / `PhpValue` / `PhpObject`-shaped — exact names unsettled) rather than `zval` internals, with any low-level access behind explicit unsafe/trusted boundaries. Everything else about product (d) lives in the embedded-PHP-host decision's open-questions list, not in v1.0 scope.

---

## 11. Baton and developer experience

Baton lands mid-plan (Phase F), once multi-file compilation exists to orchestrate:

- `baton new <name>` (binary/lib/php-lib templates), `baton build [--release]`, `baton run`, `baton test`, `baton check`.
- The command boundary is permanent: **doriac = compiler and compiler-facing inspection; Baton = project, package, workspace, build, test, benchmark, and dependency tooling.** Neither absorbs the other's job. (`baton bench` and richer workflows are future Baton commands, not MVP scope.)
- **Manifest and source inventory (decisions 0117 and 0118):** human-edited schema 2 `Baton.toml` records `vendor/package` identity, SemVer version, edition, targets, `[autoload.namespaces]`, `[autoload-dev.namespaces]`, dependencies, development dependencies, and processors. The PHP bootstrap's schema 1 remains one explicit-entry binary with no autoload, dependency, lockfile, or workspace semantics until Stage 33 implements schema 2.
- Baton resolves project and workspace structure into a versioned JSON build plan; `doriac` never parses `Baton.toml`, and Baton never parses Doria declarations. The boundary carries explicit package, source, scope, namespace-mapping, entry, generated-source, dependency, target, profile, and compiler-option facts.
- **Resolver and reproducibility (decision 0118):** direct-only dependency visibility; one version per package identity; rejected package cycles; path and Git sources first; exact Git commit locks; SemVer package constraints; deterministic JSON `Baton.lock`; source-neutral descriptors; one workspace lock; a global content-addressed cache; explicit processors; and network-free offline behavior. Host/compiler/profile facts belong in a versioned build receipt rather than the lockfile. Registry, archive, native-feed, and publishing systems remain deferred.
- **Resolver data-model constraint:** the manifest and resolver must not assume every dependency is target-independent pure Doria source — platform/architecture constraints, feature selection, native libraries, processors, binary tools, and prebuilt artifacts are typed future dimensions even where public spellings remain deferred. Reproducibility is a founding Baton property, not a retrofit.
- `baton test` defines the Doria test convention: `tests/*.doria` files whose functions marked `#[Test]` run and report (first real consumer of attributes).
- **Release versioning (the versioning decision, unauthored): calendar versioning for the toolchain, SemVer for packages.** Doria/doriac/baton/stdlib releases use the common Ubuntu-shaped CalVer `yyyy.mm.n` — year, zero-padded month (01–12), release number — so a version's age is readable at a glance (`2026.07.1`). The month is the month the release actually ships (stamped at release time, never a slipped target), which keeps the age-readability promise honest; zero-padding keeps versions sorting lexically as well as numerically. `n` starts at 1 and increments monotonically within a month across all channels: prereleases consume release numbers (`2026.07.1-canary`, then `2026.07.2-rc`), they never use dotted suffix counters, and a prerelease chain that crosses a month boundary simply picks up the new month's prefix. The suffix marks the channel — `-canary` (experimental/moving) and `-rc` (release candidate) are the fixed set — and an unsuffixed version is a stable release, with same-month patches as further `n` values. Ordering: numeric on the triple, suffixed before unsuffixed at the same triple. **Every release before the 1.0 gate carries a suffix; the first unsuffixed release ever is the 1.0-gate release.** Compatibility is *not* the version's job: language-rule changes ride the `Baton.toml` edition mechanism, and **packages in the Baton ecosystem version by SemVer** — `^`/`~` resolution semantics require it, so Decision 0118 treats toolchain CalVer and package SemVer as distinct schemes and never range-matches against toolchain versions.
- Baton drives `doriac`; it never owns semantics. LSP/editors gain workspace awareness from `Baton.toml`.

---

## 12. Decision record catalogue

Authored subjects cite their actual record numbers:

- Decision 0040: panics and overflow policy (D4, §3.6).
- Decision 0041: division, modulo, and shifts (D14).
- Decision 0042: numeric conversions (D15).
- Decision 0043: MIR and interpreter oracle (§8.1–8.2).
- Decision 0044: `doria-rt` ABI, including external-memory design cases (D18, §8.3).
- Decision 0045: runtime strings/`Bytes` and canonical display conversion, including the amended `.` (D16, §4.6).
- Decision 0069: dynamic boundary types — `mixed`/`object`/`null`/`resource`/`void` (D21).
- Decision 0074: formatted I/O — checked `sprintf`/`printf`, `read_line`, the file-I/O family, PHP-spelling fixits, rejected `print`, and deferred `sscanf` (§9).
- Decision 0075: I/O family tiers and failure-semantics migration — text/binary/stream and panic-to-`throws` migration at Stage 29 (§9).
- Decision 0081: destruction order (Stage 19) — reverse-declaration/initialization drop of still-owned locals, temporaries, and properties; the `__destruct` body runs before property drops; moves remove cleanup obligations; assignment acquires the replacement before dropping the previous value; abort-only panic runs no cleanup. The deliberate divergences and consequences are recorded below.
- Decision 0082: private native class representation (Stage 19) — headerless data-only payloads with static per-type drop glue, inlined concrete drops, fat-pointer interface dispatch at Stage 35, and a versioned `doria-rt`-private layout.
- Decision 0083: Stage 19 ownership mechanics — `take`/`writable` are mutually exclusive; promoted move-type parameters require `take`; properties are ordered explicit-then-promoted; readonly bindings may be moved but reinitialization needs `writable`; self and overlapping moves are rejected; direct move-into-properties is deferred; the temporary native-eligibility soundness gate lifts at Stage 21; allocation failure panics with `class allocation failed`.
- Decision 0084: statics and constant evaluation — sigil-free `ClassName::member`, one member namespace, `self`, `parent::`, and rejected `static::`.
- Decision 0086: copy-scalar default arguments — const-evaluable Copy-scalar and readonly-string defaults via caller-side splice (Stage 20a).
- Decision 0087: data classes, value objects, and the DTO boundary — class remains the record type, structural behavior uses opt-in derives, and DTOs remain a framework concern.
- Decision 0088: fluent method chaining — three self-return conventions, owned temporaries become exclusive places, and consuming builders remain deferred.
- Decision 0089: Stage 21 borrowing rules — readonly/writable/`take` become non-lexical borrows and ownership transfer, one-writer-XOR-many-readers is enforced, returned-borrow elision is inferred, and owned temporaries become exclusive places.
- Decision 0090: constructor definite initialization — a three-state per-property lattice merges reachable paths, requires initialization at every normal constructor exit, preserves abort-only panic, and defensively revalidates the same invariants in MIR.
- Decision 0091: I/O surface corrections — ordinary output to closed stdout/stderr pipes exits cleanly with status 0 and never migrates to `throws`, while panic reporting remains fatal if stderr is unavailable; `append_file` is the additive text spelling, `write_file` remains truncate-only, and append implementation lands in Stage 23 Slice 2.
- Decision 0092: collection type family and naming — the complete named family, default insertion ordering, sorted variants, priority queue, and deque names.
- Decision 0093: nullable types and narrowing — general `?T`, `??`, `?->`, exact `is`, shared forward dataflow facts, nullable runtime representation, and payload-derived ownership classification.
- Decision 0094: ternary and compound-assignment operators — full `? :` (strict-`bool` condition, no Elvis, desugars to a two-arm `match`), plus `.=` (string concat-assign) and `??=` (null-coalescing assign) completing the compound-assignment family.
- Decision 0095: operator surface completeness — `**`/`**=` and `<=>` are rejected as operators in favor of `Int::pow`/`Float::pow` and `Comparable<T>::compare(T $other): Ordering` (a core `Ordering { Less, Equal, Greater }` enum); `@`, backtick execution, and PHP `&`-references are rejected with targeted migration guidance, while context-aware rewrites belong to `doriac migrate php`; spread/variadic user parameters defer to the named-arguments slice.
- Decision 0096: primitives and interface conformance — primitives conform to the core value interfaces (`Equatable`/`Comparable`/`Hashable`) by compiler-known conformance and satisfy generic constraints with no boxing, `float` is neither `Hashable` nor totally `Comparable`, interface-typed slots stay class-only (a dynamically held primitive goes through `mixed`), and retroactive user-interface conformance for primitives is out of scope for v1.0.
- Decision 0097: the `when` value-returning control construct — `when` is the value-returning form of `if` (same `given`/`else when`/`else`/`finally` structure), differing only in that it always yields a value: one result type on the head only (or inferred from the first block, all branches checked against it), mandatory total `else`, block-scoped `return`-to-yield per branch, expression position, and `given` predicates AND-ed with each `when`/`else when` condition (falling to `else` when no conjunction holds).
- Decision 0098: named arguments — `name: expression` at the call site for all four callable forms, scheduled at Stage 23a; positional may precede named but not follow, named may reorder and skip defaulted parameters, arguments evaluate in source order though binding is by name, parameter names become public API, attributes (Stage 32) reuse the syntax, and variadics remain a separately deferred slice.
- Decision 0099: program entry arguments — arguments arrive as an optional `main(List<string> $args)` parameter (no `argc`; `$args->count` gives the count), populated by entry glue and depending on `List` (Stage 23); `Doria\Std\Process` owns the other process facts (exit code, pid, executable path); amends record 0032's entry-point forms.
- Decision 0101: binary standard-stream I/O — ratifies io-audit Q1(b)/Q2(a): `write_stdout_bytes(Bytes)`, `write_stderr_bytes(Bytes)`, and `read_stdin_bytes(): Bytes` (whole-stdin slurp, empty on EOF) join the Stage 23 byte tier as unshadowable intrinsics; no `write_stdout(string)` (the stdout/stderr text asymmetry is intentional — `echo` is the sole stdout text writer); output inherits the closed-pipe clean-exit carve-out, `read_stdin_bytes` panics on OS read failure with no UTF-8 validation; chunked/seekable byte I/O stays with the post-Stage-29 stream tier.
- Decision 0100: collection method surface — settles the members, receiver/ownership modes, and missing-element contract for the 0092 family. Reads are readonly, mutators writable; ingestion moves values in, removal hands the owned element back (`removeAt: T`, `pop: ?T`, `remove: ?V`; `Set::remove: bool`); `$l[i]`/`$d[k]` assert presence and panic, while `get`/`first`/`last`/`pop`/`peek*` are the `?T` safe path; mutators return `void` with userland fluency left to 0088; `contains` for membership throughout, suffixed by axis where a type has two, so Dictionary keys read `containsKey` (amended by 0113); `PriorityQueue` is min-first. Defaults land Stage 23, the sorted/priority/deque surface lands with those types, and Decision 0121 finalizes `List<T>`-only `map`/`filter`/`reduce` for Stage 30g.
- Decision 0102: sequence fill literal — the `[value; count]` repeat literal builds a runtime-`count`-length `T[]` or `List<T>` of copies of `value` (contextually typed, defaulting to `List<T>`), the only runtime-sized `T[]` constructor; `value` is a Copy scalar or `string` in v1.0 (move-type fills await `Cloneable`, Stage 35), it is sequence-only (no `Set`/`Dictionary`), `count` is evaluated once with a negative count panicking. Un-parks 0100's fill deferral (capacity `withCapacity` stays parked); scheduled as Stage 23c.
- Decision 0103: Canonical String API and companion boundary — `$s->` is limited to intrinsic measurements/views (`length`, `byteLength`, `isEmpty`, `bytes`, `graphemes`, `codePoints`), and `String::` owns every string-specific operation: transforms, predicates, search, comparison, replacement, splitting/joining, slicing, repetition/padding, and explicit byte validation. There is no public string-specific free-function family, `chars`, integer string indexing, companion alias, or instance-method alias. Unicode grapheme units govern `length`, search indices, slicing, and padding; byte-sensitive work uses `byteLength`/`Bytes`.
- Decision 0104: primitive companion completeness — the primitive companions are a complete, symmetric matrix, not per-type ad-hoc surfaces. Uniform v1.0 baseline: `parse(string): ?T` on every scalar companion (int/float/`bool`); `String` carries transforms instead of `parse` (it is the parse domain, 0103); display is uniform through the display path so no companion has `toString`. Fills the one hole — `Bool::parse(string): ?bool` (exact `"true"`/`"false"`, case-sensitive, no coercion) — with `MIN`/`MAX`, `Bool::toInt`, and the fuller string transforms named as v1.0+ furnishings. Absences are documented N/As, never silent gaps. Cites 0013/0016/0042/0095/0096/0103 for the surface they own; adds no numeric or string member. `Bool::parse` is a small scheduled slice reusing the existing scalar `parse` lowering.
- Decision 0105: generics — monomorphized (Rust model, zero-cost, no boxing) generic functions/methods (shipped at Stage 24) and classes (Stage 25); constraints spelled `<T implements A, B>` (comma-separated, constraints may be generic), inference from argument types with no turbofish. A generic class instantiation is a move type like every class (D3/0082/0087 — no user Copy aggregates); monomorphization specializes each instantiation's field and drop glue from its substituted field types (`Box<int>` drops nothing for its `int` field, `Box<Token>` drops its `Token`). Built-in `List`/`Dictionary`/`Set`/`T[]` share the same monomorphization machinery. v1.0 fences: invariant type parameters (no variance), no default type arguments, no runtime generic reflection, and value (non-type) parameters reserved as an additive extension point only. Discharges §4.5's unauthored-generics marker.
- Decision 0106: shared ownership types and API — the six compiler-known core types in two permanently disjoint families (`SharedReference<T>`/`WeakReference<T>`; `WritableSharedReference<T>`/`WritableWeakReference<T>` plus the `ReadonlySharedReferenceAccess<T>`/`WritableSharedReferenceAccess<T>` access objects), formalizing §3.3's surface against the real generics machinery. `shared new T(...)` constructs the readonly family directly under shared ownership; `new WritableSharedReference(new T(...))` takes ownership into the writable family; the families never convert or share an allocation, so readonly allocations carry no access state. All handles and access objects are move types — plain assignment transfers, `share()` adds an owner, and `clone()` stays value duplication. `WritableSharedReference<T>` never forwards to `T`: access is acquired, forwards member/indexed access without a `.value` wrapper, releases deterministically, and enforces many-readonly-XOR-one-writable with Title Case panics on the abort-only path. Compiler-known members win on the direct receiver, non-conflicting members forward transparently to `T`, and a compiler-known readonly `referencedValue` projection on `SharedReference<T>` keeps a colliding payload member reachable without new syntax. Reconciles 0005's construction modifier; strong cycles leak by design and weak references break them.
- Decision 0107: standalone lexical block statements — a bare `{ ... }` is a non-value statement and lexical scope with no trailing semicolon; bindings do not escape, still-owned values clean up in reverse acquisition order on structured exits, ordinary borrow extent remains non-lexical, and `scope` stays reserved for concurrency vocabulary.
- Decision 0110: stream, readiness, standard I/O, blocking-mode, and performance model — small byte-stream capability interfaces; owned handles and consuming explicit close/finish; first-class non-owning standard-stream views; typed read/write outcomes; capability-gated blocking modes; one portable readiness, time, cancellation, and backpressure substrate; typed buffering/text/file/process adapters; reusable buffers and safe byte regions; allocation/copy bounds; static specialization for concrete adapters; readiness reuse; and zero async scheduler cost for synchronous programs. Stage 36a implements the accepted semantic and performance contracts and owns their initial cross-platform regression gate; exact public spellings remain deferred to the record's bounded appendix process.
- Decision 0111: grouped local declarations — two or more inferred or explicitly typed locals may share one Copy initializer and one mutability mode; the initializer evaluates once before atomic name insertion, bindings initialize left to right and clean up in reverse order, strings retain one immutable handle per binding, move values are rejected without hidden clone/share, and explicitly typed nullable move groups alone may begin as literal `null`. AST/HIR preserve the group, validated MIR owns one canonical grouped initializer, and no runtime group object exists.
- Decision 0112: performance baseline, provenance, and regression measurement — one strict manifest-driven sibling benchmark engine owns comparative and matched diagnostic tiers across compiler, generated-program, and runtime-subsystem tracks; committed exact output is authoritative before timing; reports preserve raw interleaved samples, explicit availability, complete toolchain/host/command provenance, and baseline eligibility; opt-in compiler phase reports impose no ordinary compile-path work; shared CI gates deterministic structure while controlled runners own timing thresholds.
- Decision 0114: enums, backed cases, payload cases, and inline tagged layout — enums are nominal top-level types; unit/backed cases are inline Copy values with private declaration-order tags, exact equality, explicit readonly backing `value`, nullable and `mixed` identity, and no implicit display/conversion; payload cases use finite inline storage and conditional Copy/move classification. Decision 0115 now owns their Stage 28 match observation.
- Decision 0115: match expressions, patterns, exhaustiveness, narrowing, and ownership — complete core `match` evaluates one scrutinee, proves guard-aware coverage, narrows through shared dataflow, evaluates `if` guards once after pattern success, observes Move payloads by readonly borrow, explicitly consumes whole Move scrutinees with `match (take $value)`, executes ordered `match (true)`, and implements full ternary through the same validated MIR CFG. Writable patterns and payload-level `take` are rejected for v1.
- Decision 0116: `when`, `given`, control-flow `finally`, and `do ... while` — `when` is an exhaustive value expression with return-to-yield; `given` has one setup phase and attachment-specific predicate frequency; base `do ... while` is post-tested; and the bounded finalizer attachment, trigger, scope, transfer, and cleanup model is implemented across both Stage 28a slices.
- Decision 0119: checked Errors, Error values, `throws` effects, propagation, and runtime outcomes — `Error` is a compiler-known explicit-conformance Move interface; `throw` transfers ownership; `try`/`catch` coverage is checked; callable signatures carry semantic effect sets; reusable callables declare those effects while clause-free selected `main` infers its exact uncovered set; checked propagation reuses structured finalizer regions; Slices 1 and 2 implement checking and handled execution, Slice 3 owns I/O migration plus R1000, and the inferred-main corrective beat closes entrypoint boilerplate without general callable inference.
- Decision 0120: explicit closure capture lists — arrows and anonymous block functions use the same explicit `with` list for every enclosing local dependency; readonly, writable, and taking modes are written; Copy and Move values receive no implicit exception; no-capture closures omit `with`; the pre-Stage-30 grammar slice preserves that syntax before implementation.
- Decision 0121: closure function types, capture semantics, and execution model — Move-only structural function values separate value ownership, readonly/writable/once invocation, and argument ownership; function types carry checked effects; arbitrary callable expressions are invoked positionally; `$this` is captured explicitly; binding identities, creation-time acquisition, lifetime/escape, a lean two-word carrier, private physical field reordering, E0641 retirement, and the completed Stage 30a through Stage 30h sequence are fixed. Stage 30g adds `map`, Copy-only `filter`, and writable-accumulator `reduce` to `List<T>` only; readonly callbacks are borrowed readonly, while writable callbacks are borrowed exclusively. Stage 30h closes every accepted compiler/backend route and makes E0641 historical and reserved. Stage 31 Slice 1 is complete and Slice 2 is next.
- Decision 0117: namespaces, compile-time autoloading, hybrid source layout, and package compilation graphs — public `autoload`, strict namespace directories and external type filenames, source scopes, same-package include-once, package-wide `internal`, source-complete checking, and the versioned Baton-to-compiler build plan.
- Decision 0118: Baton manifests, dependencies, resolution, lockfiles, workspaces, and caches — schema 2, schema 1 compatibility, direct dependencies, path/Git resolution, SemVer, deterministic JSON `Baton.lock`, workspaces, processors, content-addressed caching, commands, and offline behavior.

Subjects awaiting decision records are deliberately unnumbered:

- Ownership and move semantics (D1, D3).
- Checked-error semantics, including errors escaping `main` (D8, §5).
- Generics and monomorphization, including the value-parameter extension point (D9, §4.5).
- Collections runtime and API surface (§9).
- Compiler-internal iteration machinery.
- Inheritance, `open`, and `override` (D17).
- Interfaces, traits, `Cloneable`, and public `Iterable` conformance.
- Property hooks. The demanding design case is **ORM-shaped lazy-loaded relations**. PHP/TypeScript ORMs in the AssegaiPHP/TypeORM lineage use proxies to intercept property access; Doria has no reflection and no proxies, so hooks are the only mechanism a property-shaped lazy relation could use. Accepted direction (audit finding F6): a hook **may `throws`** — so a lazy relation can surface a load failure through the checked-error path — but **may not block or perform async work** in v1.0, and a hooked property is therefore **not guaranteed side-effect-free**; the §6 "looks like data" contract is a readability convention, not a purity guarantee, and the record documents that honestly. Design against the ORM lazy-relation case, not only simple validation and computed properties.
- Attributes and compile-time evaluation policy, including the reflection stance: (a) compile-time introspection and derive-style attribute-driven code generation are the sanctioned mechanism for shape-driven behavior such as row mappers, serializers, validators, DI, and ORM entity mapping; they preserve static checking, require no runtime metadata, preserve decision 0082's headless representation, and permit DCE; (b) lightweight read-only runtime type identity through the `mixed`/`is`/`Displayable` tag may later grow bounded `typeName`/`implements?` queries; (c) full dynamic reflection — instantiate-by-string, invoke-by-string, and field access by name — remains out of scope pending a decision clearing a high bar because it conflicts with the headless representation, creates `mixed` holes, defeats DCE, and expands the deserialization attack surface. DDO and Assegai-shaped frameworks are served by compile-time generation, not dynamic reflection. The hardest design case is ORM-shaped cross-type metadata for entity mapping, relation wiring, and migration diffing, not a per-type derive. The record must enumerate DDO row mapping, JSON serialization, DI, validation, and ORM entity mapping as consumers.
- Unsafe/FFI, including zero-copy numerical exchange (D12).
- The `php-lib` bridge: transport-neutral contract, export analysis, thread-affinity invariant, and interop handoff questions (D13c/d).
- Async and `Shareable` (D11).
- `when` grammar.
- SIMD and engine intrinsics.
- Naming charter, built-in/userland boundary, and `function_exists` (D19).
- Explicit typing discipline (D20).
- Typed arrays `T[]`, collection literals and defaults, and rejection of `array` (D22).
- `Doria\Std\Term` and `Console`: portable terminal layer and class API, building on decision 0006 and using TermUtil as reference input (§9, product 5).
- Release versioning: toolchain CalVer `yyyy.mm.n` with channels and package SemVer (§11).
- `Doria\Std\Math` geometry: built-in Copy value types, compiler-known operators, and SIMD coordination (§9, Stage 47).
- DDO — Doria Data Objects: a replacement for the superseded decision 0007 sketch, with checked errors, native prepares, compile-checked placeholders, typed fetches, RAII transactions with consuming commit, and capability-based drivers (§9; post-Stage-29).

Each record follows the existing template: context, decision, alternatives considered, consequences, and affected components.

**Numbering policy.** Unauthored subjects above intentionally have no number. A record receives the repository's next unused number only when its file is authored. This governs all prose: cite a subject ("the Console/terminal decision") until `docs/decisions/NNNN-*.md` exists, and cite the actual number only afterwards. `scripts/check_docs_authority.php` scans this plan, `AGENTS.md`, and every decision record outside code fences; explicit `record`/`decision` citations and bare decision-shaped tokens must resolve to exactly one authored file.

### Record 0081 detail — destruction order (Stage 19)

Observable rules: (1) owned locals and temporaries are dropped in reverse order of initialization, among values still owned at the exit point — moved-out and never-initialized values are skipped; the Stage 19 definite-initialization analysis is what makes "still owned" statically known. (2) Owned temporaries within an expression die at the end of the enclosing statement (the `;`), reverse creation order, after the statement's result is bound. (3) On class destruction the user's `__destruct` body runs first, then owned properties drop in reverse declaration order, then the allocation is freed. (4) Moving a value removes it from the source's cleanup obligations. (5) Assignment fully evaluates and acquires the replacement before dropping the previous destination value. (6) Normal structured exits (`return`/`break`/`continue`/fallthrough) run cleanup; abort-only panic does not.

**Deliberate divergence to record:** properties drop in *reverse* declaration order — this diverges from Rust (which drops struct fields in forward order, an asymmetry with its reverse-order locals) and matches C++. Chosen so the whole language is uniform: everything dies in reverse of construction, locals and properties alike (properties initialize top-to-bottom, so reverse-declaration = reverse-initialization). A contributor arriving from Rust expects the opposite and must see this was deliberate.

**Consequence to record (not solve here):** rule (6) means a panic while a RAII guard is live does not run the guard — so the `Console::rawMode` "wedged terminal is structurally impossible" property carries one asterisk: a panic in raw mode will not restore the terminal. This is inherent to the accepted abort-only panic model (record 0040), not a Stage 19 defect. A minimal panic-hook restoration is a possible future addition, out of scope now; the Console narrative must state the asterisk honestly.

### Record 0082 detail — private native class representation (Stage 19)

An owned class instance is a headerless, data-only heap payload: an opaque pointer to fields at compiler-known offsets, with immutable per-type **static** metadata (size, alignment, drop glue) held once per type, never per object. Static ownership makes drop glue known at every cleanup site, so no per-object type tag is needed; no reflection means no runtime type soup; data-only layout is what FFI/zero-copy and the future inline-Copy math aggregates want.

**Consequences to record (they bind later stages):** (a) **Interface dispatch (Stage 35) is committed to fat pointers** — data ptr + vtable ptr — not per-object headers; recording it now prevents Stage 35 from reintroducing headers after the representation has shipped without them. (b) **Concrete owned drops are statically resolved and inlinable, zero indirection** — the static drop-glue metadata is consulted only for abstracted drops (fat-pointer/interface values, generic drop); dropping a value of known concrete type compiles to a direct/inlined call with no metadata lookup. The inline-Copy `Doria\Std\Math` aggregates remain a separate representation path as §9 and the geometry-math decision require; both paths share the no-per-object-header property, so the distinction between them is move-vs-Copy and heap-vs-inline, never presence of metadata.

### Record 0083 detail — Stage 19 ownership mechanics

Resolves the implementation-critical rules Stage 19 exposes that 0081/0082 don't cover:

- **`take` and `writable` are mutually exclusive parameter modes.** Both `take writable` and `writable take` are rejected — they answer different questions (ownership transfer vs exclusive borrow), and having taken ownership, exclusivity is moot.
- **Promoted move-type parameters require `take`.** A promoted move-type param transfers directly into its property; without `take` it would create two owners, which the ownership model forbids. So `function __construct(take Person $manager)` is the promoted spelling; a promoted move-type param without `take` is an error with a fixit that inserts it. Copy-type promoted params are unchanged.
- **Property order is explicit-then-promoted:** explicit properties in class-body order, then promoted properties in constructor-parameter order. Construction follows this total order; destruction (record 0081) reverses it. The total order is what makes reverse-declaration drop well-defined across both property kinds.
- **Readonly bindings may be moved from; reinitializing a moved-from binding needs `writable`.** Principle: a move is not a mutation of the binding — it is the end of the binding's ownership — so `readonly` (which governs mutation) does not forbid it. Reinitializing a moved-from binding *is* a new assignment, i.e. mutation, so it requires `writable`.
- **Self-move and overlapping source/destination moves are rejected** (`$value = $value` and taking a nested owned property from an aliasing path). Decision 0122 accepts move-in to an owning property and replacement of an initialized writable owning property. General property move-out remains separate because no property-hole or atomic take-and-replace operation is accepted.
- **Temporary native-eligibility soundness gate (historical).** Before decision 0090 landed, Stage 19 emitted native code only for classes whose every property was provably initialized via a property initializer, promotion, or the narrow direct constructor-init form; it never fell back to uninitialized memory. Decision 0090's Stage 21 reachable-path analysis lifted this gate and removed its unsupported diagnostic exactly as planned.
- **Allocation failure** uses the status-101 panic path with the canonical message `class allocation failed`. Allocation failure is OOM — the one place abort-only cleanup is unambiguously correct, since no cleanup is possible mid-allocation.

### Decision 0117 namespace-resolution authority

Decision 0117 owns the complete namespace syntax and resolution contract:
absolute qualified names, one resolution chain, grouped and aliased `use`,
wildcard rejection, the `Doria\Std` root, the edition-scoped prelude, and
unshadowable language intrinsics. This plan schedules that authority at Stage
31 and does not maintain a second copy of its rules.

### Related language naming notes

- **`is`, never `instanceof`.** `is` is the single type-test and narrowing operator across `mixed`, `?T`, and (from Stages 34/35) class hierarchies: `if ($object is Person)`. PHP splits one question three ways (`instanceof`, `is_int()`, `gettype()`); Doria answers it once. `instanceof` is rejected with a fixit, alongside `readline`→`read_line` and `print`→`echo`. Note the scope growth this implies: `is` as specified in D21 narrowed `mixed`; it now also spans class hierarchies, and SPEC must say so. A statically-decidable `is` (concrete class against its own type) should lint as always-true rather than silently pass.
- **`\` is also the string escape.** `"App\name"` is a hazard exactly as in PHP; single-quoted strings are the answer and should be documented as the home for namespace paths and Windows paths. Open sub-question for authoring: SPEC calls single-quoted strings "plain string literals" without saying whether they process *any* escapes — if zero, there is no way to write a literal `'`, which needs a ruling.

---

## 13. Phased roadmap with stages and acceptance criteria

Stages continue the existing numbering. Every stage = decision record(s) + tests + docs + examples, per Section 0. "AC" = acceptance criteria.

**Continuous performance rule:** every later stage that changes runtime
representation, allocation, ownership, dispatch, code generation, control flow,
I/O, concurrency, or FFI includes a `Performance Impact` section. It records the
expected cost; allocation, copying, dispatch, memory, and code-size changes;
benchmark cases added or updated; and measured evidence where material. A stage
may state that no measurable impact is expected, but that remains a checkable
claim. Deterministic structural checks belong in ordinary shared CI; brittle
wall-clock thresholds do not.

### Phase A — Real native foundation (Stages 11–15)
Retire the smoke architecture; make the native path general.

- **Stage 11 — MIR + interpreter oracle.** Introduce MIR; port all Stage ≤10 lowering onto it; delete `NativeSmokeModule`; ship the MIR interpreter as `--target debug`; stand up the differential test harness. AC: every existing native example produces identical output under interpreter and Cranelift; no smoke-module code remains.
- **Stage 12 — General control flow + runtime foundation.** Arbitrary/nested loops, `return` anywhere, unbounded `while`, `break`/`continue` everywhere, recursion and mutual recursion; shared dataflow framework replaces the final-statement-return rule with returns-on-all-paths. Create the minimal `crates/doria-rt`, native entry glue, and abort-only panic ABI so later checked panics have a runtime target. AC: recursive Fibonacci, nested-loop matrix example, early-return search all run natively; loop-verification cap removed; a minimal explicit panic smoke exits 101 through doria-rt; CI builds and passes on Linux, macOS, and Windows per §8.6.
- **Stage 13 — Full integer family + operators.** All fixed-width types in the compiler; `/`, `%`, shifts, bitwise across widths; contextual integer literals; overflow/div-zero panics with runtime messages through the Stage 12 doria-rt panic machinery. AC: differential tests over an arithmetic torture fixture; panic exit status 101 with message.
- **Stage 14 — Floats + bool runtime.** `float32/64` arithmetic/comparison codegen, bool as runtime value (not just conditions), `Float`/`Int` conversion companions. Records 0040–0042 collectively form the **numerical-semantics gate**: NaN/infinity behavior, determinism policy, and conversion rules must be stated in those records — not inherited by accident from PHP, Rust, or whichever backend lands first — before native numeric behavior is treated as stable. AC: numeric integration examples match interpreter bit-for-bit for f64 ops; NaN/inf comparison and conversion fixtures pass identically on all backends.
- **Stage 15 — LLVM release backend.** `--release` through LLVM over the same MIR; differential suite triples. AC: all examples identical across interpreter/Cranelift/LLVM; release binaries pass the suite.

### Phase B — Runtime strings and I/O (Stages 16–18)
- **Stage 16 — doria-rt strings + display conversion.** Heap `string` (immutable, refcounted), runtime concatenation, writable string locals, string equality/ordering, and the canonical display conversion of §4.6 for primitives across interpolation, the amended `.` (display-convertible operands, at-least-one-string guard), and `echo`: int/uint decimal, float shortest-round-trip, `bool` → `"true"`/`"false"`. AC: string-building loop example; concat of function results; `"I am " . 183` and `{$flag}` bool fixtures with exact expected output; two-int concatenation rejection snapshot; leak checker (Miri/valgrind CI job) clean.
- **Stage 17 — text I/O intrinsics + formatted I/O.** stdin/stdout/stderr streams, file read/write over `string` lines; the §9 formatted-I/O minimal set: `read_line` (newline-stripped, `null` at EOF; the prompt parameter arrives with the later Interactive Line-Input Amendment), the `read_file`/`write_file`/`write_stderr` family, and compiler-known `sprintf`/`printf` with compile-time-checked literal format strings over the v1.0 specifier subset; `print` rejected with a use-`echo` diagnostic. The literal-format analysis should stay structured for reuse — DDO's compile-checked SQL placeholders (the DDO decision) are a planned second consumer of literal-DSL validation.
  - **Console-enabling constraints (so Stage 46's `Console` needs no rework):** doria-rt's I/O layering separates the raw device layer (handle/fd read-write, explicit flush) from line discipline (`read_line`'s buffering lives above the device layer, never baked into it, so raw-mode byte-level input later reuses the same primitives); a TTY/interactivity query primitive ships for all three standard streams; and the Windows implementation writes correct UTF-8 to the console from day one (console output code page / wide-write handling decided here, not deferred — Mojibake discovered at Stage 46 would mean re-plumbing Stage 17).
  - **Continuation:** `Bytes` is deferred to Stage 23 Slice 2: a mutable buffer is a move type, so it belongs after the ownership stages.
  - **Acceptance criteria:** cat-clone and line-count example programs; echoing-`read_line` loop example; PHP-spelling fixit snapshot (`readline` → `read_line`); sprintf fixture matrix (`%05d`, `%.2f`, `%x`, width/align) with exact expected output; format-mismatch diagnostic snapshots (`%d` with string arg, wrong arity, non-literal format); TTY-detection primitive unit-tested (piped vs terminal); non-ASCII output renders correctly on the Windows CI runner.
- **Stage 18 — Interpolation of expressions + Displayable.** Full `{expr}` interpolation; `Displayable` interface (compiler-known) as Doria's `__toString` replacement, wired into all three display contexts; parser fuzzing job lands. AC: `echo "sum: {a() + b()}"`; interpolating a non-Displayable class is a compile error with suggestion.

### Phase C — Classes go native (Stages 19–22)
- **Stage 19 — Ownership, moves, destruction.** Native class layout, `new`, property init expressions, promoted params; classes become the first Doria move types: move analysis, use-after-move diagnostics in plain vocabulary, drop elaboration placing deterministic `__destruct` at end of owning scope, `take` parameters. Explicit user clone syntax and the `Cloneable` interface are deferred until method/interface support exists.
  - **Layout constraint (the ownership/move-semantics decision + the geometry-math decision):** the object-representation machinery must not assume every aggregate-with-methods is a heap-allocated move type — compiler-known inline Copy aggregates exist (payload enums now, `Doria\Std\Math` value types at Stage 47) and share layout machinery with, but not the heap/move classification of, classes.
  - **Settled authority:** Destruction order and native class representation are settled in records 0081/0082 (see §12); combination rules for `take`/`writable`, promoted move-type transfer, property ordering, move legality, the temporary native-eligibility soundness gate, and the allocation panic are settled in record 0083.
  - **RAII resource guard:** The example is a simple class that acquires an abstract owned resource in its constructor and releases it observably in `__destruct`, shaped as the future `Console::rawMode` guard so it dry-runs the flagship RAII case and is differential-testable across backends; it defines no `File`, stream, or FFI handle.
  - **Use-after-move diagnostic:** It does not suggest `->clone()` (clone does not exist until method/interface support); it names the give-away point and that the value can't be used after.
  - **Acceptance criteria:** destructor-order example; use-after-move diagnostic snapshots; RAII resource-guard example; leak CI clean.
- **Stage 20 — Methods, statics, internal.** Instance/static method codegen, `internal` enforcement in native path, class constants + const evaluation tier.
  - **Receiver-mode constraint (the ownership/move-semantics decision + the DDO decision):** the receiver-mode representation must not hard-code exactly two modes (readonly/writable `$this`) — a consuming (`take`) receiver is a known future need (DDO's `Transaction::commit` consuming the transaction; builder finalizers), so the machinery leaves room for a third mode even though Stage 20 implements only two.
  - **Clone boundary:** This stage may add compiler-recognized `->clone()` lowering for explicit built-in duplication where needed, but the public `Cloneable` interface waits for Stage 35 conformance.
  - **Read-modify-write on property places lands here** (§3.2): `$this->value++`, `$obj->prop += 1` — same operation as the property assignment this stage already lowers, with sugar.
  - **Static-access spelling is settled in §6.5:** sigil-free `ClassName::member`, one member namespace, `self` reserved, `parent::` generalized, `static::` rejected — and per §0 the `::`-form grammar is assigned here: `self`/`parent::` parse and `static::` is rejected with its fixit in this stage, while `parent::save()` semantics wait for Stage 34.
  - **Acceptance criteria:** the SPEC §6 `Parser` class runs natively.
- **Stage 20a — Copy-scalar default arguments.** Decision 0086 establishes one caller-side splice for native calls that omit trailing const-evaluable fixed-width integer, float, or bool arguments. The splice serves free functions, instance methods, static methods, and constructors; writable Copy-scalar parameters remain valid. Move-type and `take` defaults remain ownership work. AC: omitted and explicit defaults match across the interpreter, Cranelift, and LLVM, including promoted constructor properties and class-constant defaults.
- **Stage 20b — Const-literal string default arguments.** Decision 0086 extends the shared caller-side splice to const-evaluable defaults on readonly `string` parameters. Omitted values use the ordinary string-literal argument materialization path; ordinary call temporaries are released after the call, while constructor-promoted values are retained by their properties. `?string`, `writable string`, `take string`, and other ownership-bearing defaults remain deferred. AC: omitted and explicit string arguments match across the interpreter, Cranelift, and LLVM for free functions, instance methods, static methods, and constructors; class-constant defaults and mixed scalar/string trailing omission are covered; native leak checks remain clean.
- **Stage 21 — The borrow checker.** Non-lexical borrow checking on MIR: readonly/writable parameters and `$this` become enforced borrows, place-expression borrows, borrow-returning accessors under the §3.2 elision rule, one-writer-XOR-many-readers conflict diagnostics in owns/gives vocabulary; **fluent chaining conventions 1–2 and the owned-temporary rule (decision 0088): readonly and writable self-return chaining, and owned rvalues become exclusive places so `E0203` no longer fires on them — `(new X())->mutate()` becomes legal** (the consuming self-return, convention 3, stays deferred); constructor definite-initialization on all paths (decision 0090, finishing SPEC §5 future-work note) lands on the same dataflow framework. The shared-ownership pressure valve (`SharedReference<T>`/`WeakReference<T>`/`WritableSharedReference<T>`) is rescheduled to Stage 25a, where the nullable and generics machinery it depends on exists (§3.3).
AC: legal/illegal borrow and ctor fixture matrix; borrow-conflict diagnostic snapshots; getter-returning-borrow example; writable self-return chain and owned-temporary (`(new X())->mutate()`) fixtures, with the borrow-can't-bind-to-owned-`let` diagnostic; zero runtime checks emitted for ordinary borrow checking.
- **Stage 22 — Nullable + narrowing + `is` + `mixed` statics.** Decision 0093 completes D5: general `?T`, `??`, `?->`, exact `is`, shared flow-sensitive narrowing, and nullable runtime values across the interpreter, Cranelift, and LLVM. D21 static semantics are complete: `mixed` accepts every type and rejects every operation until narrowed with `is`; `null` is rejected in type position with a `?T` suggestion; `void` is return-only; `object` does not exist; and `resource` remains reserved. The boxed `mixed` runtime representation lands in Stage 23 Slice 3; `match` narrowing lands in Stage 28, hierarchy `is` in Stage 34, and interface `is` in Stage 35. AC: null-safe chaining and scalar/string/class nullable parity; narrowing snapshots; mixed operation-rejection and narrowing matrix; null/void/object/resource position diagnostics.

### Phase D — Collections, generics, and performance foundation (Stages 23–26b)
- **Stage 23 — Runtime collections, typed arrays, Bytes, and mixed runtime (three slices).** Each deferred slice keeps a slice-named semantic diagnostic; no backend placeholder stands in for it.
  - **Slice 1:** Implements owned `T[]`, `List`/`Dictionary`/`Set`, contextual collection literals, indexing and indexed read-modify-write, insertion-ordered `foreach` borrows, move-in/removal ownership, and Decision 0100's default member surface in shared MIR and `doria-rt`. Dictionary `keys`/`values` are readonly insertion-ordered `foreach`-only projections, not storable copies or bespoke view values. AC: move/borrow fixture matrix over collections and typed arrays; `int[]`/`int[][]` fixed-length fixtures; literal context typing (`int[]` vs `List<int>` from the same literal, uncontextual sequence/keyed defaults); in-place `foreach` and indexed mutation; use-after-move and owned-removal lifetime snapshots; exact interpreter/Cranelift/LLVM parity.
  - **Slice 2:** Implements the `Bytes` move type; explicit copying `uint8[]` conversion; byte indexing, mutation, length, and equality; text `append_file`; whole-file `read_file_bytes`/`write_file_bytes`/`append_file_bytes`; and whole-stream `read_stdin_bytes`/`write_stdout_bytes`/`write_stderr_bytes` per §9 and records 0091/0101. AC: non-UTF-8 byte-identical file and standard-stream round trips across all three backends, append/truncate parity, conversion-copy behavior, indexed RMW, equality, and OOB panic.
  - **Slice 3:** Adds the boxed `dr_mixed` representation and collection element paths that require runtime mixed boxes. AC: mixed box round trip (int/string/class in, narrowed out), `?mixed`, and heterogeneous mixed collection values across interpreter/Cranelift/LLVM.
- **Stage 23a — Named arguments.** Decision 0098: `name: expression` at the call site for all four callable forms (free functions, instance/static methods, constructors), on the caller-side binding machinery default arguments (Stages 20a/20b) established. Positional args may precede named but not follow them; named args may appear in any order and may skip defaulted parameters; arguments evaluate in source order though binding is by name; parameter names become part of the callable's public API. Lands here because Stage 23 does not depend on it and Stage 24's generic call resolution should build on a settled named-binding model; Stage 32 attributes reuse this syntax (adding only const-eval rules), and DDO inherits the binding model. Variadics remain a separate deferred slice (0095). AC: named-arg fixtures across all four callable forms; middle-default-skip fixture; source-order-evaluation (side-effect) fixture; duplicate/unknown/missing diagnostics; borrow-conflict-in-a-named-call fixture; a compiler-side accepted-syntax fixture with zero parser errors and stage-named unsupported diagnostics before this stage.
- **Stage 23b — Program entry arguments.** Decision 0099's optional `main(List<string> $args): int` form: entry glue in `doria-rt` populates an owned `List<string>` from the platform argument vector and passes it to `main`; `$args->count` gives the count and there is no `argc`. Per 0099's post-acceptance amendment, the **executable path is stripped** — `$args[0]` is the first real argument and a no-argument invocation yields an empty list — with the executable path reachable through `Doria\Std\Process` instead. The existing parameterless `main(): void` / `main(): int` forms keep working unchanged, and `Doria\Std\Process` still owns the other process facts (exit code, pid, executable path). Scheduled explicitly because 0099 tied this to "the collections tier (Stage 23)" for its `List<string>` dependency, which Slice 1 satisfied, but none of Stage 23's three slices owned the work. AC: an argument-echoing example program run with and without arguments across the interpreter, Cranelift, and LLVM; `$args->count` and indexed/`foreach` access fixtures; empty-argument-vector fixture; parameterless-`main` regression fixtures; and a stage-named (never permanent-sounding) semantic diagnostic for the form before this stage.
- **Stage 23c — Sequence fill literal.** Decision 0102's `[value; count]` repeat literal: a runtime-`count`-length `T[]` or `List<T>` of copies of `value`, contextually typed (defaulting to `List<T>`) exactly like the element-list literal (§4.9), and the only runtime-sized `T[]` constructor. `value` is evaluated once and replicated (a Copy scalar bit-copy, or a `string` retained per slot); `count` is a runtime `int` evaluated once (value before count), with a negative `count` panicking, a const-negative `count` a compile error, and `count == 0` yielding an empty sequence. Element eligibility in v1.0 is Copy scalars and `string`; move-type element fills (concrete class, collection, `Bytes`, `mixed`, nullable) are deferred with a diagnostic naming `Cloneable` (Stage 35). The repeat form is sequence-only — rejected for `Set`/`Dictionary`. Un-parks 0100's fill deferral; the `withCapacity` capacity hint stays parked. AC: `bool[]`/`int[]`/`List<bool>`/`string` fill parity across the interpreter/Cranelift/LLVM; the uncontextual `List<T>` default; a negative-count panic fixture and a const-negative-count compile-error snapshot; move-type-element and `Set`/`Dictionary` rejection snapshots; a runtime-sized `bool[]` sieve example.
- **Stage 24 — Generic functions.** D9 for free functions/methods, monomorphization in MIR. AC: `first<T>` works across int/string/class lists.
- **Stage 25 — Generic classes.** The §4.5 `History<T>` example and user-defined generic class machinery build on Stage 23's compiler-internal built-in collection iteration. Public generic interfaces, traits, and user-defined `Iterable<T>`/`Iterator<T>` conformance are deferred to Stage 35. AC: generic classes run natively without changing the already-shipped built-in `foreach` behavior.
- **Stage 25a prerequisite corrections (not a new language stage).** Before Slice 3 acceptance, the compiler closes three foundational gaps exposed by natural Doria: indexed writes through writable property paths move into the collection slot without replacing the containing property; returned-borrow provenance remains tied transitively to `$this` or one borrowed parameter through property and compiler-known collection projections; and Decision 0107 standalone lexical blocks provide an explicit cleanup/access boundary. AC: the generic `Repository<T>` indexed-save/borrowed-find example and the writable-shared access-block example run through the interpreter, Cranelift, and LLVM with exact output and cleanup parity.
- **Stage 25a — Opt-in shared ownership — Complete.** One language stage landed through four separately reviewed slices. Every slice coordinates its compiler revision, diagnostics, hover/completion, highlighting, and fixtures with `dorialang/doria-language-server`; Stage 25a is complete only when Slice 4 merges and every acceptance criterion passes.
  - **Slice 1 — Grammar and type model — Implemented.**
  - **Slice 2 — Readonly shared-ownership family — Implemented.**
  - **Slice 3 — Writable family and access guards — Implemented.**
  - **Slice 4 — Final integration and LSP/editor sweep — Implemented.**
  - **Readonly ownership:** The §3.3 pressure valve builds on decision 0005's `shared new` surface and on Stage 22 nullable/narrowing plus Stage 25 generic classes. `shared new T(...)` creates a `SharedReference<T>` — the opt-in reference-counted owning handle — directly under shared ownership (plain `new T(...)` remains an owned `T`, and there is no implicit owned-to-shared conversion). `$ref->share()` creates another owning reference; `$ref->createWeakReference()` derives a `WeakReference<T>` whose `$weak->acquire()` returns `?SharedReference<T>` (`null` after the final owner is released).
  - **Writable ownership:** `WritableSharedReference<T>` is the explicit runtime-checked writable form, built with an ordinary ownership-taking constructor (`new WritableSharedReference(new T())`), with its own `share(): WritableSharedReference<T>` and `createWeakReference(): WritableWeakReference<T>`; it never forwards direct access to `T`, so access must be acquired through `acquireReadonlyAccess()`/`acquireWritableAccess()`, returning `ReadonlySharedReferenceAccess<T>`/`WritableSharedReferenceAccess<T>` access objects that forward member/indexed access to `T`, release deterministically, and enforce many-readonly-XOR-one-writable at runtime.
  - **Payload domains:** The two families accept different payloads: the readonly family takes **class payloads only in v1.0** (so `shared new List<int>()` and `SharedReference<int>` are rejected), because it forwards readonly access directly and has no access object for indexed or collection operations; the writable family executes class, generic-class, typed-array, `List<T>`, `Dictionary<K, V>`, `Set<T>`, and `Bytes` payloads through access objects that forward member and indexed operations. Scalar/string writable-shared payload spellings retain their type identity but remain behind a stage-named runtime diagnostic until a complete-value access surface is accepted; shared handles through `mixed` remain pending.
  - **Authority and future boundary:** These type and operation names are approved canonical inputs; the Stage 25a shared-ownership decision formalizes them and the compiler-known access-forwarding and access-lifetime rules. Stage 25a's reference and access types are not thread-safe and do not automatically cross thread boundaries; thread-safe variants remain deferred to Phase H under the `Sendable`/`Shareable` record. At this stage the six types are compiler-known core/prelude types usable without namespace syntax; Stage 31 assigns their canonical qualified core names and enables `use ... as ...` local aliases, while canonical docs, diagnostics, and generated code keep the full names.
  - **Acceptance — ownership:** `shared new T(...)` has static type `SharedReference<T>` and `new T(...)` remains owned `T`, with no implicit owned-to-shared conversion. `share()` creates another owning reference on both shared forms and last-owner destruction runs exactly once. `createWeakReference()` yields a `WeakReference<T>` / `WritableWeakReference<T>` that does not keep the value alive, `acquire()` returns `?SharedReference<T>` / `?WritableSharedReference<T>`, strong cycles leak by design, and a weak reference breaks them.
  - **Acceptance — access:** `WritableSharedReference<T>` takes ownership through its constructor and never forwards direct access to `T`. Multiple readonly accesses coexist, writable access is exclusive, readonly and writable never overlap, and incompatible access uses P1501 with one of the three exact typed conflict reasons. Access objects are move types that may be returned, stored, and passed but cannot be constructed directly by user code, retain an owning reference to the allocation, permit only their declared operations, and may not move out or consume `T`.
  - **Acceptance — family separation and cleanup:** The readonly and writable families are disjoint — no conversion either way between `SharedReference<T>` and `WritableSharedReference<T>`, weak forms preserve their family, and no readonly-family and writable-family handle ever refer to the same allocation, with all `WritableSharedReference<T>` handles to one allocation sharing a single runtime access state. Access objects release deterministically and moving one transfers the release obligation. Access objects forward member/indexed access to `T` without a `.value` wrapper. `SharedReference<T>::referencedValue` resolves wrapper/payload collisions as a readonly allocation-free projection. Ordinary borrow checking still emits zero runtime checks even alongside `WritableSharedReference<T>`.
  - **Acceptance — integration:** Coordinated LSP completion/hover and editor coverage; interpreter/Cranelift/LLVM parity; leak-free weak-cycle and bounded stress coverage.
- **Diagnostic Experience Foundation — Implemented.** Decision 0108 replaces the
  early one-message/one-span presentation boundary with one compiler-owned,
  multi-label diagnostic model and human, concise, schema-version-1 JSON
  renderers. It establishes Title Case titles and prefixes, fix applicability,
  causal grouping, duplicate suppression, backend/external wrapping, and the
  internal-error envelope; the language server and website consume the same
  structure. This checkpoint did not complete Stage 25a Slice 4 when it landed;
  the later final integration slice now does.
- **String API Decision Amendment — Implemented.** Decision 0103 now establishes
  one canonical boundary: `$string->` owns intrinsic measurements and
  views, `String::` owns all string-specific operations, and capabilities
  without a natural type owner may remain free functions. The public `str_*`
  family and `$string->chars` are removed; `length` is grapheme-counted and
  `byteLength` is explicit. This is an authority change only and does not claim
  runtime support.
- **String API Completeness Audit Against PHP — Implemented.** The checked-in
  offline inventory classifies every official PHP core string, mbstring, and
  grapheme capability as String, Bytes, another domain, derivable, deferred,
  rejected, not applicable, or an unresolved design fork. It proposes no API
  silently and changes no compiler behavior.
- **Decision 0103 Completeness Review — Implemented.** Andrew approved the
  audit's recommended path on 2026-07-31. The symmetric ignore-case search
  family, first-grapheme casing, and occurrence counting join the v1 inventory;
  review/defer rows remain named later furnishings with their dependencies.
- **Minimum String Runtime Surface — Implemented.** The selected Decision 0103
  subset uses one shared Unicode implementation across the interpreter and
  `doria-rt`; Cranelift and LLVM lower the same validated String MIR intrinsics
  to that runtime ABI. The approved ignore-case search family, first-grapheme
  casing, and occurrence counting are included. Grapheme and code-point views
  remain traversal work, and ordering comparisons remain blocked on executable
  `Ordering`.
- **Unified Doria Diagnostic Presentation And Runtime Outcome Foundation —
  Implemented.** Decision 0109 extends the compiler-owned `Diagnostic` as the
  sole public representation for compile-time findings and runtime outcomes.
  All implemented built-in panics have stable central-catalogue `P` codes,
  source-aware labels, Doria call paths, and status-101
  abort-without-cleanup semantics across the interpreter, Cranelift, LLVM, PHP
  compatibility, standalone executables, and `doriac run`. Human output uses
  the global Doria `Where`/preview/`Why` grammar and `Call Path`; concise, JSON,
  LSP, and Playground projections consume the same structured facts. The
  source model and runtime-outcome extension are also mandatory for checked
  errors. Decision 0119 settles their semantic model and future R1000
  presentation; Stage 29 Slices 1 and 2 complete checking and handled
  propagation, while process-level runtime rendering remains in Slice 3.
- **Interactive Line-Input Amendment — Implemented.** Amends decision 0074's
  line-input surface to `read_line(string $prompt = ""): ?string` — one function
  with an optional parameter, not an overload pair. Every call evaluates the
  prompt exactly once, writes it to stdout exactly as supplied with no added
  newline, flushes stdout, and only then reads one line; the flush also happens
  for the default empty prompt, so `read_line()` keeps working while making
  earlier `echo` output visible before the program blocks. Prompts are emitted
  under redirection, repeated calls emit their own prompts even when the next
  line is already buffered, and the existing line discipline and EOF contract are
  unchanged. A closed stdout pipe during the prompt write or flush takes decision
  0091's permanent status-0 carve-out and never reads stdin; other output
  failures stay status-101 panics through decision 0109. Interpreter, Cranelift,
  LLVM, and the PHP compatibility backend share one lowering, and PHP does not
  depend on the optional readline extension. This follows the completed Unified
  Diagnostic Presentation And Runtime Outcome Foundation and precedes Stage 25a
  Slice 4, which is now implemented. Stage 25a is complete. The PHP Stream And
  I/O Completeness Audit is implemented, Andrew's Stream API Completeness Review
  is complete, and decision 0110 accepts the stream architecture and performance contract. Stage 26, Stage 26a, Stage 26b, all four Decision 0113 slices, both Stage 27 slices, both Stage 28 slices, both Stage 28a slices, all three Stage 29 slices, the pre-Stage-30 closure grammar slice, Stages 30a through 30h, and the Constructor Writable-Path And Owned-Property Corrective Beat are complete. Decision 0121 accepts and implements the complete Stage 30 closure authority, and Decision 0122 accepts the constructor/property correction. The performance `Measurement Status: Pending Available Runner` remains non-blocking and is not a performance pass. Stage 30 and Stage 31 Slice 1 are complete, E0641 is historical and reserved, Stage 31 Slice 2 is next, and Stage 31 remains in progress.

  **Current planning checkpoint:** Stage 25a — Complete; PHP Stream And I/O Completeness Audit — Implemented; Andrew’s Stream API Completeness Review — Complete; Stream Architecture And Performance Decision — Accepted (decision 0110); Stage 26 — Complete; Stage 26a — Complete; Stage 26b — Complete, All Three Slices Complete (decision 0112); Measurement Status: Pending Available Runner; Decision 0113 — Complete; Stage 27 — Complete; Decision 0115 — Implemented; Stage 28 — Complete; Decision 0116 — Implemented; Stage 28a — Complete; Decision 0119 — Accepted And Implemented; Stage 29 — Complete; Stage 29 Slice 1 — Complete; Stage 29 Slice 2 — Complete; Corrective Beat: Native Collection Property Initializers — Complete; Stage 29 Slice 3 — Complete; Corrective Beat: Inferred Main Checked Effects — Complete; Pre-Stage-30 Grammar Slice — Complete; Stage 30 Closure Authority — Accepted And Implemented; Stage 30a Callable Grammar Completion — Complete; Stage 30b Semantic Function Types And Captures — Complete; Constructor Writable-Path And Owned-Property Corrective Beat — Complete; Stage 30c Ownership, Lifetime, And Escape — Complete; Stage 30d Closure HIR/MIR And Interpreter Oracle — Complete; Stage 30e Native Execution — Complete; Stage 30f PHP Compatibility — Complete; Stage 30g List Algorithms — Complete; Stage 30h Cross-Repository Closure — Complete; Stage 30 — Complete; E0641 — Historical And Reserved; Stage 31 Slice 1 — Complete; Stage 31 Slice 2 — Next; Stage 31 — In Progress, Not Complete; Stage 35a — Scheduled; Stage 36a — Scheduled; Stage 36a Public Spellings — Deferred; Stage 36a — Not Implemented.

  **Completed slice checkpoints:** Decision 0113 Slice 2 — Complete; Decision 0113 Slice 3 — Complete; Decision 0113 Slice 4 — Complete; Stage 27 Slice 1 — Complete; Stage 27 Slice 2 — Complete; Stage 28 Slice 1 — Complete; Stage 28 Slice 2 — Complete; Stage 28a Slice 1 — Complete; Stage 28a Slice 2 — Complete. Stage 29 is complete.
- **Stage 26 — Remaining collection family — Complete.** Stage 23 ships Decision 0100's default `List`/`Dictionary`/`Set`/`T[]` surface. Stage 26 adds the authored non-closure surface with ascending `SortedDictionary`, ascending `SortedSet`, min-first `PriorityQueue`, and ring-buffer `Deque`; Decision 0121 reserves `map`/`filter`/`reduce` for `List<T>` alone at Stage 30g. Existing-source `::from` and set algebra preserve their inputs and support `Copy` values here, with `Cloneable` widening retained by Stage 35. Set iteration is readonly, dictionary keys remain readonly, sorted-dictionary values and deque elements may be writable where their receiver and binding permit it, and `PriorityQueue` has no iteration order. Before Stage 31 include/multi-file support, required stdlib fragments are compiler-bundled or prelude-style rather than source-included. The completed stream review does not add streams to Stage 26. AC: the remaining non-closure collection family compiles and runs from the compiler-provided stdlib surface.
- **Stage 26a — Grouped local declarations — Complete.** Decision 0111 adds the four local-only forms `let $a, $b = value;`, `let writable $a, $b = value;`, `T $a, $b = value;`, and `writable T $a, $b = value;`. One common Copy initializer evaluates once before atomic name insertion; bindings initialize left to right, receive independent locals with one type and mutability mode, and clean up in reverse order. Strings retain the immutable handle per binding without copying contents. Move values are rejected without implicit clone/share, except explicitly typed nullable move groups initialized by literal `null`; untyped grouped `null` is rejected. AST/HIR and one validated MIR form preserve the group, all native paths and PHP agree, and no runtime grouping abstraction exists. AC: parser/semantic/ownership diagnostics, PHP temporary collision coverage, and exact interpreter/Cranelift/LLVM fixture parity.
- **Stage 26b — Performance Baseline Foundation — Complete; All Three Slices Complete.** Decision 0112 accepts one repository-owned benchmark system that later stages extend; this is measurement infrastructure and accepted baselines, not an unlimited optimization campaign.
  - **Slice 1 — Complete:** Supplies the strict manifest/report and compiler-report foundation.
  - **Slice 2 — Complete:** Supplies deterministic scaling, the compiler/generated/runtime/diagnostic matrix, process-resource and profiler adapters, candidate evidence, and an exact structural baseline without timing thresholds.
  - **Slice 3 — Complete:** Supplies comparative peers, enforced fairness records, controlled-runner and promotion workflows, deterministic compiler/runtime artifact identity, shared workload profiles, and the exact native acceptance policy.
  - **Performance model:** Compiler performance, generated-program performance, runtime-subsystem performance, cold startup, memory, and artifact size remain separate dimensions. The interpreter is a semantic oracle rather than a native competitor. Native runtime status compares each Doria backend with the fastest valid C, C++, or Rust peer and passes only at `ratio <= 1.30`; PHP remains adoption evidence. Correctness and adequacy precede timing, Inconclusive is never Pass, and Doria self-regression thresholds do not weaken cross-language status. A future `baton bench` orchestrates the same engine.
  - **Portable obligations:** Complete. These cover correctness and interpreter/Cranelift/LLVM parity; exact compiler/runtime provenance and deterministic runtime selection; workload scaling and peer-equivalence metadata; the binding `1.30` rule; structural baselines and portable smoke validation; the failure/scheduled-work register; and controlled-runner commands/report schemas.
  - **Controlled measurement:** `Measurement Status: Pending Available Runner`. No eligible controlled physical-host Linux sessions exist, so no timing baseline or acceptance matrix is promoted. That evidence is required before release/public performance claims, but it is not a language or compiler stage gate. Docker, WSL, containers, and virtual machines may provide engineering, correctness, workflow-rehearsal, optimization-guidance, and local-regression evidence; their native timing status remains Inconclusive. Controlled Linux timing, verified affinity, Callgrind, DHAT, hardware counters, and cross-platform timing baselines cannot become Stage 26b or Stage 27 closure conditions.
  - **Acceptance criteria:** Reproducible manifest-driven runner and report schema; exact-output/hash validation; deterministic CI structural checks; exact structural baseline; executable controlled-runner and acceptance workflows; exact native acceptance policy. Decision 0113 and all four slices are complete; both Stage 27 slices are complete with no performance-evidence dependency.
- **Decision 0113 — Collection Surface Completion — Complete.**
  - **Slice 2 — Complete:** Adds one compiler-owned receiver-aware suggestion table, structured E0521 fixes, a dedicated property-invoked-as-method diagnostic, operation-specific equality diagnostics, and bracket-literal guidance for withdrawn `List::from` / `Dictionary::from`.
  - **Slice 3 — Complete:** Executes first-position `List::indexOf`, first-match writable `List::remove`, O(n) map `containsValue`, and borrowed O(1) set endpoint properties.
  - **Slice 4 — Complete:** Executes writable, zero-argument `clear(): void` in place on all seven named collections through shared validated MIR and exact-once type-aware cleanup; no Decision 0113 member remains routed to E0559.

### Phase E — Enums, match, errors (Stages 27–29)
- **Stage 27 — Enums + payload cases — Complete; No Performance-Evidence Dependency.** Decision 0114.
  - **Slice 1 — Complete:** Authority, enum/case grammar, unit and `int`/`string`-backed execution, nominal identity, inline tags, equality, readonly `value`, nullable/`mixed`, constants/statics/defaults, supported collection placement, PHP native enums, and grammar-only payload/`match` preparation.
  - **Slice 2 — Complete:** Payload construction with shared positional/named binding, finite central inline layout, recursive Copy/Move classification, case-aware copy/drop/equality glue, aggregate ABI, class/generic/collection storage, nullable and `mixed` transport, Copy constants/defaults, faithful PHP lowering, and exact interpreter/Cranelift/LLVM parity.
  - **Deferred:** Generic enums remain deferred and ordinary enum containers allocate no heap object.
- **Stage 28 — match — Complete.** Implemented decision 0115 preserves D7 and delivers both risk-separated slices.
  - **Slice 1 — Complete:** Guard-free exhaustive match expressions, unit/payload enum cases, positional payload destructuring and case-only ignore, exact compile-time constants, `null`, exact type-binding narrowing for nullable/`mixed`, strict arm typing, readonly payload ownership, ordered `match (true)`, and full right-associative ternary.
  - **Slice 2 — Complete:** `if` pattern guards with readonly guard views, one-time ordered evaluation and guard-aware coverage/reachability; explicit `match (take $value)` whole-scrutinee consumption; owned selected Move payloads, nullable/`mixed` extraction and exact cleanup; rejected writable patterns; and PHP backend-private exact `mixed` tags.
  - **Acceptance criteria:** One validated MIR CFG executes across the interpreter, Cranelift, and LLVM; PHP preserves the same checked semantics. Durable guard/consumption/destruction parity, malformed-MIR rejection, exact PHP type identity, and coordinated tooling.
- **Stage 28a — Control-flow completion — Complete.** Decision 0116 settles and implements the family.
  - **Slice 1 — Complete:** Executable `when` expressions with mandatory `else`, explicit/expected/inferred result typing and nearest-`when` return-to-yield; executable `given` on `if`, `when`, and `while`; and executable base `do ... while`.
  - **Slice 2 — Complete:** One backend-neutral finalizer-region model executes `finally` on `if`, `when`, `while`, and `do ... while` and routes every structured exit while preserving panic's abort-only behavior.
  - **Continuation:** Stage 29 extends these same regions for checked errors rather than creating a second cleanup mechanism. Controlled timing is Pending Available Runner and non-blocking.
- **Stage 29 — Checked errors end-to-end — Complete.** Decision 0119 settles and implements the complete model.
  - **Slice 1 — Complete:** Dedicated `try`/`catch`/`throw`/`throws` grammar; backend-independent AST/HIR; compiler-known explicit `Error` conformance; source-ordered semantic effect sets; catch coverage; catch-or-declare checking; ownership; construction/finally checks; diagnostics; and tooling.
  - **Slice 2 — Complete:** The two-word carrier, static descriptors, hidden first-origin storage, checked-result ABI, explicit checked MIR, propagation, exact and catch-all dispatch, rethrow, Stage 28a finalizer reuse, failed construction, Error values in supported aggregates and `mixed`, shared validation, and backend parity. B2901 is historical and unreachable for valid programs.
  - **Corrective Beat — Native Collection Property Initializers — Complete:** All concrete specialized property and payload-enum storage types are interned before callable lowering, preserving contextual collection initializers, independent per-instance storage, reverse destruction, and semantic capability diagnostics; property move-in and replacement are governed separately by Decision 0122.
  - **Slice 3 — Complete:** The record-0075 migration supplies the six exact compiler-known `Doria\Std\Io` identities, authoritative built-in effects, deterministic platform failure facts, checked text/binary/device I/O, and first-origin construction across all backends. An Error escaping any accepted `main` shape cleans up, reports R1000 as `runtimeError`/`propagateWithCleanup`, and exits 70 through generalized private transport or standalone rendering. B2902 has no valid route. P1401 through P1407 are historical for ordinary I/O; allocation panic and the closed-standard-pipe status-0 carve-out remain separate.
  - **Corrective Beat — Inferred Main Checked Effects — Complete:** Clause-free selected `main` infers the same exact uncovered source-ordered set used by HIR, MIR, source callers, ABI selection, and every backend; explicit `main throws` stays compatible, ordinary callables remain explicit, and nonthrowing entrypoints pay no checked-ABI cost.
  - **Continuation:** The pre-Stage-30 closure grammar slice, Decision 0121 authority beat, Stages 30a through 30h, and the Decision 0122 corrective beat are complete; Stage 30 is complete. Stage 31 Slice 1 is complete, Slice 2 is next, and Stage 31 remains in progress.

### Phase F — Multi-file, namespaces, Baton (Stages 30–33)
- **Pre-Stage-30 Grammar Slice — Closure accepted syntax — Complete.** Implemented after Stage 29 and before Stage 30 under decision 0120 and the §0 two-clocks rule. `fn` and anonymous-function expressions, explicitly typed parameters, arrow return inference, authoritative `function(T): R` type syntax, explicit `with` lists, readonly/writable/taking capture modes, precise source-preserving AST nodes, deliberate recovery, accepted and malformed syntax fixtures, and the catalogued `E0641` Stage 30 boundary are complete. No free-variable discovery, capture validation, HIR/MIR lowering, ownership analysis, environment construction, collection algorithm, or backend execution was added. The accepted snippets are parser fixtures, not runnable examples.
- **Stage 30 Closure Authority — Accepted And Implemented; Stage 30 — Complete.** Decision 0121 elaborates Decision 0120 and applies Decision 0119. Function values are Move-only structural values with independent value ownership, readonly/writable/once invocation, and per-argument ownership. Function types preserve checked effects; arbitrary callable expressions invoke positionally after one callee evaluation; named and bound callable references remain deferred. The subsection order below is the accepted dependency order.
  - **Stage 30a — Callable Grammar Completion — Complete.**
  - **Stage 30b — Semantic Function Types And Captures — Complete:** Checks explicit `$this` and local captures by stable binding identity and validates structural callable use.
  - **Corrective Beat — Constructor Writable-Path And Owned-Property — Complete.**
  - **Stage 30c — Ownership, Lifetime, And Escape — Complete:** Acquires captures at creation in authored order, tracks readonly/writable leases to last use, moves function carriers and taking captures, consumes once calls path-sensitively, enforces callback escape and storage, validates one-root returned borrows and nested provenance, and records reverse logical release plans.
  - **Stage 30d — Closure HIR/MIR And Interpreter Oracle — Complete:** Adds explicit closure/callable-call HIR, structural function MIR, the logical two-word carrier, static descriptors, owned environment layouts, synthetic closure functions, indirect and checked indirect calls, shared validation, stable interpreter places, and debug-target execution.
  - **Stage 30e — Native Execution — Complete:** Implements the shared concrete native ABI, stable capture places, stack storage for nonescaping environments, one heap allocation for escaping environments, generated reverse-order drop glue, and Cranelift/LLVM execution.
  - **Stage 30f — PHP Compatibility — Complete:** Consumes the same semantic and validated MIR closure plans for PHP compatibility, using explicit compiler-owned carriers, environments, stable places, checked paths, and cleanup rather than host capture semantics.
  - **Stage 30g — List Algorithms — Complete:** Adds explicit semantic/HIR plans and one validated traversal CFG for `List<T>::map`, Copy-only `filter`, and writable-accumulator `reduce`; all supported backends preserve exact callback effects, readonly source traversal, and checked partial-state cleanup.
  - **Stage 30h — Cross-Repository Closure — Complete:** Completes the accepted storage and backend cross-product, including exact function-value transport through `mixed`, and proves all accepted routes through the shared parity manifests.
  - **Runtime and performance:** No-capture closures allocate no environment and ordinary closures use no reference counting. E0641 is historical and reserved. Controlled timing remains non-blocking with `Measurement Status: Pending Available Runner`; pending evidence is not a performance pass.
  - **Continuation:** Stage 31 Slice 1 is complete and Slice 2 is next. The debug interpreter remains the semantic oracle; Cranelift, LLVM, and PHP compatibility consume the same validated closure authority for their supported surfaces.

  Stage 30 acceptance criteria:

  1. Both closure body forms use the same explicit capture-list law; no-capture closures omit `with`, and Copy/Move values have no implicit exception.
  2. Function types distinguish readonly repeatable, writable repeatable, and `once` consuming invocation; `function take()` is rejected.
  3. Function-type parameters preserve readonly, `writable`, and `take`, and structural types preserve normalized `throws` effects.
  4. Closure bodies infer invocation mode and checked effects; closure expressions gain no effect annotation.
  5. Any callable-valued expression can be invoked after evaluating the callee once and arguments left to right; structural calls are positional.
  6. Named-function, static-method, bound-method, and constructor references remain deferred; wrapper closures adapt them.
  7. `$this` uses `with ($this)` or `with (writable $this)`; taking the borrowed receiver is rejected.
  8. Capture discovery uses stable binding identities through blocks, control-flow bindings, interpolation, guards, and every intermediate nested closure.
  9. Captures acquire at creation in written order and destroy in reverse logical order; borrowing closures obey owner lifetime and escape constraints.
  10. Missing, duplicate, wrong-mode, unused, moved, and insufficient-lifetime captures receive precise diagnostics and conservative fixes.
  11. The logical carrier is two words; no-capture closures allocate no environment; descriptors are lean and non-reflective.
  12. Physical environment fields may reorder privately while preserving logical source order, offsets, ownership events, and destruction.
  13. Validated MIR and all backends preserve one indirect/checked-call, cleanup, ownership, and diagnostic model; PHP host behavior is private.
  14. `List<T>` alone receives `map`, Copy-only preserving `filter`, and writable-accumulator `reduce` in Stage 30g.
  15. List callbacks are nonescaping, reject once invocation, traverse in insertion order, and propagate their exact checked effects.
  16. E0641 has retired by completed route and is historical and reserved.
  17. Portable allocation, cleanup, compile-time, memory, and artifact checks land with implementation; unavailable controlled hardware never blocks a slice.
- **Stage 31 — Namespaces and package compilation graph — In Progress, Not Complete.**
  - **Slice 1 — Complete:** decisions 0028 and 0117 now execute source-preserving namespace/import/include grammar, absolute qualified names, one central imports/current-namespace/edition-prelude chain after intrinsic handling, grouped `use`, the explicit edition-2026 prelude, package/edition/source compiler context, canonical package-owned global identities, same-file namespace execution across all backends, and open-document LSP symbol identity. `include` and external graph-dependent names stop at precise Slice 2 development boundaries and never lower.
  - **Slice 2 — Next:** versioned JSON build plans, multi-file indexing, direct-dependency visibility, package-wide `internal`, hybrid strict layout, include-once, declaration-only non-entry files, duplicate symbols across packages, cross-file diagnostics, and compiler incremental inputs. Qualified names resolve before the existing `extends` Stage 34 and `implements` Stage 35 boundaries. AC remaining for Stage 31 completion: multi-package project build; every active file checked; complete duplicate-symbol provenance; include/autoload deduplication; package visibility; coordinated package-graph LSP coverage.
- **Stage 32 — Attributes.** `#[...]` parsing, type-checked against attribute classes, const-evaluation-tier arguments (resolving SPEC §11's evaluation-policy question: compile-time const evaluation only, no side effects); reflection deferred — attributes are compiler/tooling metadata in v1.0. Standing separation, preserved by this stage and after: **attribute metadata, constant evaluation, and any future general compile-time execution are three distinct concepts** — attributes never grow into arbitrary compile-time code execution, and compiler transformations reject unsupported side effects rather than silently changing behavior. AC: `#[Test]`, `#[PHPExport]` representable — parsed and type-checked as attributes only; `#[PHPExport]` bridge semantics activate in Stage 41, and export is metadata, never visibility.
- **Stage 33 — Baton package and dependency workflow.** Decision 0118.
  - **Slice 1:** schema 2 parsing with exact schema 1 compatibility, autoload/autoload-dev discovery, main/development/generated scopes, binary/library targets, deterministic inventory, and single-package build plans.
  - **Slice 2:** path and Git dependencies, SemVer validation, one-version conflicts, deterministic JSON `Baton.lock`, install/update/fetch/tree/why/add/remove, global content-addressed cache, and offline resolution.
  - **Slice 3:** workspaces, development dependencies, graph commands, incremental inventory, `baton test`, explicit processor registration, generated-source orchestration, and Phase F closure. Stage 32 supplies typed metadata and the processor protocol; it does not own automatic package orchestration. AC: `baton new game && baton test` green; locked offline workspace build; direct-dependency visibility and conflict diagnostics; no implicit build scripts.

### Phase G — OOP completion and hosted I/O foundation (Stages 34–36a)
- **Stage 34 — Inheritance.** Qualified `extends` names (parsed since the §0 two-clocks rule, resolved since Stage 31) become fully semantic here. D17: `open`/`override`, vtables, parent construction rules, devirtualization in LLVM profile. AC: `Post extends Model` native; missing-override diagnostics.
- **Stage 35 — Interfaces + traits.** Qualified `implements` names become fully semantic here; `Displayable` stays compiler-known and unqualified per 0079. Conformance checking, interface-typed values (**fat pointers** — data ptr + vtable ptr — as committed in §8.3 and record 0082, not per-object headers), trait flattening + `insteadof`/`as`. `Cloneable` becomes the public explicit-duplication contract here, and user-defined `Iterable<T>`/`Iterator<T>` conformance plugs into `foreach`. The public interface machinery must support decision 0110's small readable, writable, duplex, seekable, flushable, blocking-configurable, and readiness-aware capabilities without hard-coding one god-object stream hierarchy. Concrete generic adapters remain monomorphizable and inlinable; deliberate interface erasure may dispatch dynamically but must not allocate a heap object per call or adapter layer. Exact interface names remain deferred to the decision-0110 appendix before Stage 36a implementation. AC: SPEC §8 examples native; a user-defined iterable is consumed by `foreach`; a Cloneable class can be cloned through the interface contract.
- **Stage 35a — Optimizer Contracts, Dispatch, And Escape Audit — Scheduled.** Audit and encode sound optimization facts for direct methods, open virtual calls, devirtualization, interface fat-pointer dispatch, generic static dispatch, trait-flattened calls, primitive constraint specialization, monomorphization code-size growth, ownership-derived alias information, sound readonly/nonnull/dereferenceability/alignment/`nocapture` metadata, escape analysis, stack promotion, and non-escaping closure/class allocations. Specialized and deliberately erased paths are measured separately. Completion requires at least one real stack-promotion or equivalent escape-analysis path, not merely an optimization plan; broader future escape analysis remains incremental. AC: dispatch-shape IR checks, metadata soundness negatives, code-size evidence, separate specialized/erased measurements, and one proven non-escaping allocation promotion.
- **Stage 36 — Property hooks.** §6.4 hooks. AC: `Temperature` example. (`when` moved to Stage 28a — it is basic control flow, not OOP completion.)
- **Stage 36a — Stream, readiness, and standard I/O foundation.** **Scheduled, not implemented.** Decision 0110 accepts both the semantic architecture and its binding performance/memory contract: small byte-stream capabilities; owned handles with consuming explicit close/finish and nonthrowing best-effort destruction; first-class non-owning standard streams over the intrinsic device substrate; data/would-block/EOF/timed-out reads; partial-progress writes; capability-gated blocking modes; one multi-stream readiness, duration/deadline, cancellation, and backpressure model; typed buffering and incremental UTF-8 text adapters; typed file requests and locking; bounded streaming copy; and owned child processes with concurrently drained pipes. The steady-state data plane has no mandatory allocation per operation or loop iteration, exposes reusable caller/adapter buffers and safe readable/writable byte regions, avoids hidden whole-chunk and unread-suffix copies, keeps common outcomes and standard-stream views allocation-free, reuses readiness registrations/event storage, forbids ordinary busy polling and one-thread-per-stream designs, and initializes no executor/task/scheduler infrastructure in synchronous programs. Concrete adapters remain eligible for static specialization and inlining; deliberate interface erasure may dispatch dynamically without a heap object per call or layer. Exact public interface, member, result-case, readiness, byte-region, reusable-buffer, standard-stream, file, adapter, and process spellings are deferred to a decision-0110 appendix before implementation begins; this is a naming deferral, not a semantic or performance review gate. Prerequisites: Stage 29 checked errors, Stage 31 namespaces/multi-file support, Stage 35 interfaces/capability contracts, existing ownership/RAII, the existing standard-device runtime substrate, and decision 0109's unified diagnostics/runtime outcomes. Property hooks are not intrinsically required, but Stage 36a remains after Stage 36 to preserve the accepted linear sequence. Non-goals: async state-machine lowering, a multithreaded executor, TCP/UDP product APIs, HTTP, TLS implementation, terminal raw mode/key/resize/cursor/screen/color/styling, PHP-compatible dynamic wrapper or string-filter registries, global stream contexts, and mixed metadata bags. Stage 36a owns the initial Linux/macOS/Windows stream benchmark and memory-regression gate: cold startup; throughput and latency; wall/user/system time; peak RSS; allocation count where available; syscall count where available; development/release/stripped binary size; and correctness hashes or exact output. Its required cases are large streaming file copy, repeated small writes, non-blocking pipe transfer, child stdout/stderr drainage, incremental UTF-8 line processing, first-class versus intrinsic standard-output writes, many stable readiness registrations, synchronous startup, a concrete adapter chain, and an erased interface stream. Equivalent direct OS/C/Rust implementations are comparison baselines, never unsupported superiority claims. Structural allocation/copy/readiness assertions run in ordinary CI; curated timing regressions run on controlled runners. Stage 43 continues and broadens this suite instead of postponing the initial gate. AC: the semantic fixtures above agree across interpreter/Cranelift/LLVM; the benchmark cases produce identical correctness hashes; reusable-buffer loops show no mandatory steady-state allocation or hidden whole-chunk copy; standard-stream/common-outcome and readiness-reuse structural checks pass; synchronous startup initializes no async infrastructure; all three native OS backends meet the accepted regression thresholds recorded with the benchmark harness. No speculative source spelling is accepted syntax here.

### Phase H — Concurrency (Stages 37–39)
- **Stage 37 — Concurrency design the async decision.** Paper stage: full async model (executor in doria-rt, task groups, cancellation, `Shareable` rules). **Stage 37 must consume Stage 36a readiness**, stream ownership, typed read/write outcomes, backpressure semantics, duration/deadline model, one cancellation model, process pipes, and the accepted readiness-reuse/allocation contract from decision 0110. It must not invent a second stream model, readiness model, event-loop abstraction, I/O-specific cancellation model, per-stream worker thread, or per-event heap-allocation convention. The design tests must include data-pipeline and long-running compute scenarios — bounded parallelism, backpressure, prefetch, worker pools, failure propagation, deterministic cleanup — alongside the PHP thread-affinity invariant (§10.3). Designer sign-off required — this is the one deliberate design gate in the plan.
- **Stage 38 — async/await codegen.** State-machine lowering in MIR; single-threaded executor first; `async main` entry bootstrap per §5 (executor started only when `main` is async — sync programs initialize no executor, task, or scheduler infrastructure). Async stream operations lower over decision 0110's same outcomes, readiness, deadlines, cancellation, backpressure, reusable buffers, and partial-progress state; they do not gain async-only contracts or require per-operation allocation. AC: an async file-read example expressed through Stage 36a's stream contracts rather than a one-off file intrinsic; interpreter parity; async-`main` escaping-error example exits 70 with destructors run; the synchronous-startup structural benchmark still reports zero async infrastructure.
- **Stage 39 — Structured task groups + Shareable checking.** Multithreaded executor; spawn-boundary checks via auto-derived `Sendable`/`Shareable` — with ownership in place this is Rust Send/Sync-grade freedom from data races. Task-group cancellation/failure releases or closes pending stream operations deterministically under decision 0110's ownership and single cancellation model. The executor preserves bounded backpressure and readiness reuse instead of replacing them with unbounded task queues or one thread per stream. AC: parallel map example; data-race fixture rejected at compile time; cancelled stream-operation cleanup fixture; bounded stream-pipeline allocation/queue regression.

### Phase I — Systems and PHP bridge (Stages 40–42)
- **Stage 40 — unsafe/FFI.** D12: `unsafe`, `Ptr<T>`, `extern "C"`, linking foreign libs via Baton manifest. The unsafe/FFI decision must evaluate **zero-copy numerical exchange** as a named design case: pointer+length(+stride) views over `T[]`/`Bytes`, ownership transfer of externally allocated buffers, and callbacks — FFI must not be designed to copy all buffers by default. AC: bind and call a C function (e.g., zlib) from Doria; a zero-copy `Bytes`-view round-trip fixture.
- **Stage 41 — php-lib bridge.** D13c end-to-end behind public `baton build --php-lib`: export analysis, C-ABI shim gen, PHP FFI stub gen, handle lifetime tests against real PHP 8 in CI. doriac provides compiler emission primitives only as needed by Baton. The bridge ABI and ownership/lifetime design must be transport- and direction-neutral per §10.3 — a later Zend adapter and the deferred embedded host (§10.4) consume the same contract. AC: the `ImageResizer` scenario runs from a PHP script in CI through Baton; a generated PHP stub retains an opaque bridge handle and releases it in `__destruct`; a readonly exported method can be invoked, and a writable exported method can be invoked and observably mutates the underlying Doria instance; the bridge handle is represented as neither public Stage 25a shared-reference family and no ownership-family conversion is exposed; multiple PHP references/wrappers to one native instance cannot double-free it, and final handle release destroys the Doria instance exactly once; checked errors become generated PHP exceptions while a panic retains the §10.3 host-process behavior; the ABI remains transport-neutral and the header review confirms no Zend structures and no Doria object layout leakage; thread-affinity tests confirm PHP runtime values stay on their designated thread.
- **Stage 42 — migrate php v0.** §10.2 conservative converter. AC: converts a small idiomatic PHP 8 fixture app; dynamic features produce diagnostics not silent guesses.

### Phase J — Engine enablers and 1.0 hardening (Stages 43+)
- **Stage 43 — Engine Performance And Optimization Hardening.** Consume the Stage 26b benchmark system to broaden, profile, tune, harden, and compare Doria's engine workloads. Scope includes declare-based overflow relaxation for audited modules, arena allocator hooks, profile-guided optimization and layout, whole-program optimization, link-time optimization policy, broader escape analysis, stack/arena promotion, collection layout tuning, drop-glue optimization, shared-ownership hot paths, cache locality, game/engine workloads, and C/C++/Rust comparative reports. It does not introduce Doria's first benchmark infrastructure. It continues and broadens Stage 36a's already-enforced stream performance/memory gate; it does not defer the initial stream benchmarks or their regression thresholds until this stage.
- **Stage 44 — SIMD direction (the SIMD/engine-intrinsics decision)** + `Doria\Std\Net`/`Http` maturation for the PHP sidecar pattern. **Stage 44 builds on Stage 36a duplex streams, readiness, timeouts, partial writes, cancellation, backpressure, reusable buffers/byte regions, readiness reuse, and async-cost isolation** under decision 0110; it does not create unrelated socket read/write contracts or regress to hidden per-packet allocation/copy. Network connection and TLS configuration remain typed `Doria\Std\Net` concerns over that shared foundation.
- **Stage 45 — Self-hosting start.** Port the lexer to Doria as the first self-hosted component (per docs/self-hosting.md), compiled by `doriac`, differentially tested against the Rust lexer.
- **Stage 46 — `Doria\Std\Term` v0 + `Console` (the Console/terminal decision).** **Stage 46 reuses Stage 36a standard-stream views, readiness, blocking substrate, timeout/deadline integration, cancellation, and platform-device abstraction** under decision 0110 while remaining the exclusive owner of raw mode, key/resize events, cursor, screen, color, and styling. Mode changes require exclusive control and a restoring guard. The §9 portable terminal layer is surfaced through the `Console` class (TermUtil-informed API inventory settled in the record): raw-mode enter/leave (restored via the guard's `__destruct` on every structured exit including error escaping `main` — a wedged terminal is the classic TUI failure; per record 0081 an abort-only *panic* in raw mode does not run cleanup, so raw-mode restoration on panic, if wanted, requires the future panic-hook addition, not the guard), key/resize event decoding to enums, cursor/style/clear/size, both platform backends landing together per §8.6. It does not reimplement generic readiness or pipe polling. May proceed in parallel with Stage 45. AC: an interactive demo (input echo + moving glyph + resize handling) runs from the same source on Linux, macOS, and Windows; Unix CI drives it under a pty harness, Windows CI under a ConPTY harness; zero escape sequences appear in the demo's source.
- **Stage 47 — `Doria\Std\Math` geometry v0 (the geometry-math decision).** `Vector2/3/4`, `Quaternion`, `Euler`, `Matrix3x3/4x4` as built-in inline Copy value types with compiler-known operators, `float32` variants, charter-named API surface; layout coordinated with the SIMD-direction decision's SIMD direction. AC: transform-chain example (translate–rotate–scale via `Matrix4x4`); quaternion slerp fixture matching reference values; operator fixtures differential across all backends; no heap allocation in a vector-arithmetic hot-loop benchmark.
- **1.0 gate** (ships as the first unsuffixed `yyyy.mm.n` release per §11)**:** spec freeze pass over SPEC.md, diagnostics audit, doria-rt ABI review, differential + fuzz suites green, the three flagship demos build: a portable TUI game (engine seed — the same binary source running natively on Windows, macOS, and Linux via `Doria\Std\Term`, zero ANSI in user code), a UI component demo, and a PHP app calling a Doria php-lib. At least one reviewed stream-focused demonstration — streaming file copy, a non-blocking child-process pipeline, or a readiness-driven multi-stream program — must also pass; the exact source is chosen after the decision-0110 public spellings are finalized.

### Dependency notes for the implementing agent
- Nothing in Phases B–J may begin before Stage 11 lands (everything depends on MIR + oracle).
- WASM backend remains recognized-but-unscheduled; do not start it before 1.0.
- Game engine and UI framework are **separate repositories** consuming Doria; this plan only builds their enablers. Do not scaffold them inside the compiler repo.

---

## 14. What is explicitly out of scope for v1.0

The following are out of scope for v1.0 — each a deliberate identity or scope choice, not a gap:

- Tracing GC (never)
- pervasive ARC as the default model (never)
- Rust-spelled borrow sigils and lifetime annotations (never — inference and elision only)
- visibility modifiers beyond the default-accessible + `internal` two-state model — `protected`, `private`, and `public` keywords (never; this is identity, not scope deferral)
- a broad PHP-style `array` type (never — sequences are `T[]` typed arrays and named collections per §4.9)
- an `object` type (cut; reintroduce only with concrete PHP-bridge evidence)
- `resource` as a core type (reserved to the Phase I bridge)
- `Result<T,E>` surface model (per 0035)
- unions beyond `?T`
- `goto`
- textual macros (per 0028)
- runtime reflection
- package registry server
- catchable panics
- user-defined operator overloading
- late static binding (`static::` — never; `static` is the member modifier and `self::` is the qualifier, §6.5)
- sigil-carrying static access (`Foo::$prop` — never)
- default interface methods
- variadic generics
- a TUI widget/framework layer in the stdlib (userland territory — the ported engines are the widget layer; `Doria\Std\Term` stays primitive)
- raw ANSI escape sequences as any public stdlib API (never — `Doria\Std\Term` is capability-based)
- a `print` construct (never — `echo` is the one output spelling)
- `sscanf` (deferred — see §9)
- dynamic (non-literal) format strings for `sprintf`/`printf`
- wholesale import of PHP's string-function catalogue (post-1.0, case by case under the §9.1 charter)
- the embedded-PHP host implementation (product d — deferred, architecture preserved per §10.4)
- Zend-extension code generation (intended production transport, unscheduled until after Stage 41)
- PHP object proxies and the callback/reentrancy runtime
- a Composer prebuilt-binary packaging matrix
- framework adapter packages (Laravel/Symfony/AssegaiPHP — separate repositories)
- a Laravel 4 / legacy assessment profile for `doriac migrate` (modern typed PHP remains the first migration target)
- the entire AI/numerical stack (tensor or `NDArray` standard types, automatic differentiation, GPU/accelerator backends, graph capture, mixed-precision or distributed training, notebooks and REPL kernels, experiment tracking, model/dataset hubs, Python-interop bridges, MLIR adoption, shape-dependent typing, and any AI-specific keyword — all Appendix A, none v1.0)
- bidirectional PHP compatibility guarantees

---

## 15. Summary for the designer

This plan turns Doria's accepted principles into a complete, ordered build-out: a genuine ownership and borrow-checking model re-spelled into Doria's existing readonly/writable vocabulary plus `take` — Rust's machinery with none of its sigils; a finished type system (fixed-width numerics, nullables, payload enums, exhaustive match, monomorphized generics); checked errors as the recoverable path and abort panics as the fatal one; closed-by-default OOP with traits and hooks; a real MIR with an interpreter oracle and dual Cranelift/LLVM backends over one semantics; a Doria-authored stdlib; Baton; and — as the strategic differentiator — a first-class native bridge that lets any PHP application call compiled Doria as if it were a normal PHP class. Approve or amend the Section 1 table, and the rest executes stage by stage without further design stalls.

---

## 16. Appendix A — long-range AI & scientific-computing workstream (directional, unscheduled)

Doria's long-range positioning: *a compiled, statically checked AI and data-systems language with PHP-shaped readability, explicit mutation, native deployment, and deep interoperability with established numerical ecosystems.* This appendix is direction, not schedule — no phase here has dates, stages, or v1.0 standing, and nothing in it may be pulled forward except through the readiness constraints already threaded into Sections 2–11. AI facilities arrive as **libraries plus compiler extension points** (intrinsics, transformations, optional lowering behind stable APIs), never as core-language AI syntax; interoperability with existing native/Python-adjacent infrastructure precedes any attempt at ecosystem replacement.

- **AI-0 — Readiness (this plan):** semantic type extensibility, numerical-semantics gate, closures/callables, ownership-native buffers, transport-neutral FFI with zero-copy views, compiler services, reproducible Baton. Complete when v1.0 ships as specified.
- **AI-1 — Numerical foundation:** fixed-width dtype completion (`float16`/`bfloat16`), an `NDArray`/tensor storage library over `T[]`-grade buffers (dtype/rank/shape/strides/views), CPU kernels, serialization/memory-mapping, baseline benchmarks (the first legitimate AI benchmark category — none exist before this).
- **AI-2 — Ecosystem interop:** native numerical-library bindings, low-copy tensor exchange, model/data interchange formats, Python extension or embedding bridges where practical (architecture decided then, not now).
- **AI-3 — Differentiation & model development:** automatic differentiation as a compiler transformation behind library APIs (purity/effect metadata question resolved here), parameters/modules/losses/optimizers, data loaders, checkpointing.
- **AI-4 — Research experience:** REPL and notebook kernel over the §8.4 compiler services, incremental compilation, rich values, experiment manifests on Baton's reproducibility base.
- **AI-5 — Accelerators & distribution:** device abstraction, accelerator lowering through the §8.1 optional stage, fusion, mixed precision, multi-device and distributed execution.

Open decisions deliberately *not* settled by this plan and reserved for these phases: operator overloading / numerical protocol design (post-interfaces, consistent with nouns-are-properties and the §9.1 charter), value-parameter surface syntax, purity/effect metadata, accelerator IR, Python interop architecture, notebook execution model, Baton native-dependency and build-script security model.
