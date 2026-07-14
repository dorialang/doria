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
- Every stage ships with: a decision record (if it introduces semantics), integration tests in `crates/doriac/tests`, updated `SPEC.md` and `README.md` sections, updated editor token guardrails when vocabulary changes, and example programs in `examples/`.
- The "stop and ask" rule from SPEC.md §1.1 still applies, but **only for forks not answered by this document**. If this document answers it, implement it as written. If this document and SPEC.md conflict, this document wins for future-work items and SPEC.md wins for already-implemented behavior; flag the conflict in the stage's decision record either way.
- Native-first correctness policy is unchanged: Doria semantics → Doria IR → backend lowering. The PHP backend never defines semantics.
- Temporary backend limitations remain unsupported-feature diagnostics, never redefinitions of the language.
- **Documentation and website examples may only demonstrate behavior this plan or an accepted decision record specifies.** An example that presupposes unresolved semantics (an entry-point form, a feature interaction, a stdlib API no record covers) is itself a design fork: stop and ask before publishing it. Specified-but-unimplemented features shown in docs must be marked with the stage in which they land.

---

## 1. Decisions this plan makes — designer review checklist

These are the load-bearing choices. Each becomes a decision record before its first implementing stage lands. Andrew has approved this plan as the current master direction; later amendments should update this file and, where appropriate, the corresponding decision record.

| # | Decision | Accepted default in this plan |
|---|----------|-------------------------------|
| D1 | Memory model | **Full ownership + borrow checking — Rust's model in PHP spelling.** Single ownership with move semantics for classes/collections, deterministic `__destruct` at end of owning scope, Copy semantics for primitives/strings, opt-in `Shared<T>`/`Weak<T>`/`SharedMut<T>` for shared ownership. **No tracing GC, no pervasive ARC, no Rust sigils or lifetime annotations in surface syntax** |
| D2 | Borrow spelling | readonly = shared borrow, `writable` = exclusive borrow, `take` = ownership transfer into the callee. Ordinary borrow rules (many readers XOR one writer; borrows cannot outlive owners; moved values unusable) are enforced entirely at compile time by a non-lexical borrow checker over MIR, with zero runtime cost and no dynamic fallback. Explicit `SharedMut<T>` is the named dynamic-check escape hatch. |
| D3 | Copy vs move | Copy types: primitives, `bool`, `string`, ranges, enums with Copy payloads, and (from Stage 47) the built-in `std::math` value types (`Vector2/3/4`, `Quaternion`, ...) as compiler-known inline Copy aggregates. Move types: classes, `T[]` typed arrays, `List`/`Dictionary`/`Set`, `Bytes`, closures. Explicit duplication uses the future `->clone()` / `Cloneable` surface once method and interface support exist; before that, move-type duplication is deliberately unavailable and diagnostics must not suggest `->clone()` (see record 0083). No user-defined `struct` in v1.0 — classes are the owned record type (revisit inline layout post-1.0 if engine profiling demands it) |
| D4 | Integer overflow | Arithmetic overflow panics in both dev and release profiles. Explicit `Int::wrappingAdd(...)`, `Int::saturatingAdd(...)`, `Int::checkedAdd(...)` for other behavior. A `declare` key may later relax this per-module for engine hot paths |
| D5 | Nullability | `?Type` optional types (PHP spelling), `??` null coalescing, `?->` null-safe access. `null` is not assignable to non-`?` types. No implicit truthiness |
| D6 | Enums | PHP 8.1-shaped `enum` declarations extended with payload cases (tagged unions): `case Some(int $value);`. This is Doria's sum type |
| D7 | Pattern matching | `match` is expression-position, exhaustiveness-checked over enums/bools/finite domains, PHP 8 `match` spelling extended with payload destructuring. `when` is the value-returning conditional chain per decision 0009 |
| D8 | Errors | Checked `throw`/`throws` with PHP-shaped `try`/`catch`/`finally`. Errors are class instances implementing the built-in `interface Error`. `Result<T, E>` stays out of the surface model per decision 0035 |
| D9 | Generics | Monomorphized generics for functions, classes, interfaces, traits. Constraint spelling: `<T implements Comparable>`. No runtime generic reflection in v1.0 |
| D10 | Closures | PHP-shaped anonymous `function (...) with (...) { }` plus auto-capturing arrow functions `fn(int $x) => ...`. Closure and arrow-function parameters must be explicitly typed, just like free-function, method, constructor, and property-hook setter parameters; Doria never infers omitted parameter types. The capture clause is spelled `with`, not PHP's `use` — in Doria `use` belongs exclusively to namespace imports. Captures are borrows by default (`with ($x)` readonly, `with (writable $x)` exclusive) or moves via `with (take $x)`. A borrow-capturing closure is itself borrow-bound, so the borrow checker rejects escapes automatically and suggests `take` |
| D11 | Concurrency | Structured concurrency with `async function` / `await` / task groups; data-race freedom falls out of the ownership model via auto-derived `Sendable` / `Shareable` marker interfaces (Rust's Send/Sync payoff) checked at spawn boundaries. Detailed design gated behind its own decision record in Phase H |
| D12 | Unsafe & FFI | `unsafe { }` blocks gate raw pointers (`Ptr<T>`, `MutPtr<T>`), foreign calls, and manual memory. `extern` declarations bind C ABI symbols. Everything outside `unsafe` keeps full safety guarantees |
| D13 | PHP interop (the strategic pillar) | **Four architecturally separate products**: (a) existing Doria→PHP compat backend, (b) `doriac migrate php`, **(c) the PHP→native-Doria runtime bridge via `baton build --php-lib`** (Baton orchestrates doriac compilation, a versioned C-ABI bridge, and generated PHP adapters — FFI as bootstrap transport, generated Zend extension as the intended production transport over the same contract), and **(d) native-Doria→embedded-PHP host runtime** (the Lenga pattern: a native application hosting PHP as first-class scripting) — architecture-visible now, implementation-deferred until the (c) contract is stable and separately approved. Generated PHP is never Doria's semantic reference. doriac remains the compiler and exposes only narrow emission primitives to Baton. (c) gets its own phase |
| D14 | Division/modulo | `/` on `int` is truncating integer division; `%` is remainder with sign of dividend (C/PHP `intdiv`-consistent). Division/modulo by zero panics. `float` division follows IEEE 754 |
| D15 | Numeric widening | No implicit conversions anywhere, including int→float. Explicit `Int::toFloat($x)`, `Float::toInt($x)` (truncating, panics on NaN/out-of-range), and fixed-width conversions via `Int32::from($x)` (panics on overflow) / `Int32::tryFrom($x)` (nullable) |
| D16 | String encoding | `string` is immutable UTF-8. Byte-level work uses `Bytes` (a mutable move-type buffer). Indexing a `string` by integer is not allowed; iteration yields grapheme clusters via `$s->chars` is deferred, `$s->bytes` ships first. One canonical display conversion (§4.6) feeds interpolation, `.`, and `echo`: primitives convert out of the box (`bool` → `"true"`/`"false"`, never PHP's `"1"`/`""`), classes via `Displayable::toString()` — there is no `__toString` magic method |
| D17 | Inheritance model | Single class inheritance, multiple interface implementation, trait composition via `uses` with explicit conflict resolution (`insteadof`/`as` PHP spelling accepted). Methods are non-virtual by default; `open function` opts into overriding; `override function` required at override sites |
| D18 | Standard entry runtime | Every native binary links `doria-rt` (Rust-implemented runtime library): allocator, drop glue, `Shared<T>` refcount machinery, string/collection intrinsics, panic machinery, stdout/stderr. `doria-rt` is an internal ABI, not public, until v1.0 |
| D19 | Naming charter & the free-function boundary | **Built-in free functions are `snake_case`** with uniform, fully-worded names (`get_time`, `str_starts_with`, and the emblematic `str_case_compare` — never `strcasecmp`); **userland free functions are camelCase**, so free-function casing alone marks what is Doria's versus the codebase's. **All member-style APIs are camelCase everywhere** — standard-library members and companion/type APIs included (`Int::wrappingAdd`, `$s->isEmpty`, `Displayable::toString`) — because a member's receiver (`Int::`, `$s->`) already carries its provenance; only prefix-less free functions need casing to do that job. Types and enum cases `PascalCase`, constants `SCREAMING_SNAKE_CASE`; magic methods keep PHP spelling (`__construct`, `__destruct`). §9.1 is checkable API law |
| D20 | Explicit typing discipline | **Parameter types are never inferred — anywhere.** Free functions, methods, constructors (including promoted parameters), closures, arrow functions, callbacks, and property-hook setters all require written parameter types: `fn($x) => ...` is a compile error, `fn(int $x) => ...` is the language. Named functions and methods must declare return types; only an arrow function's return type may be inferred from its body. `let` locals may infer from their initializer. Nothing ever silently defaults to `mixed`. Full rules in §4.7 |
| D21 | Dynamic boundary types | `mixed` is Doria's **only** dynamic type and it is **unknown-flavored, never any-flavored**: every value may flow in (implicit boxing), and nothing may be done with it until narrowed via `is` / `match`. `mixed` is a boxed, runtime-tagged **move type** — always, even when holding a Copy value. `object` does not exist. `null` is a literal and the `?T` machinery, never a standalone type-position name. `resource` is reserved for the Phase I PHP bridge, not a core v1.0 type. `void` is return-position only. Full rules in §4.8 |
| D22 | Sequences: typed arrays + named collections, no `array` | **Doria has no broad PHP-style `array` type** — `array $items` and `List<array>` are invalid, and the identifier is not a type name. C-style **typed arrays** are spelled `T[]` (`int[]`, `string[]`, `mixed[]`, `int[][]`): contiguous, fixed-length-after-creation move types — the engine-grade buffer. **Named collections** are the growable/structured family: `List<T>`, `Dictionary<K, V>`, `Set<T>` (future: `Queue<T>`, `Stack<T>`, ...). Bracket literals are contextually typed collection literals, never evidence of an `array` type. Full rules in §4.9 |

Everything below elaborates these decisions into implementable specifications.

---

## 2. Vision, positioning, and end products

Doria is a statically checked, natively compiled systems language with PHP-shaped syntax and Rust-grade safety defaults, minus Rust's lifetime/borrow surface language. The strategic products it must eventually support, in priority order:

1. **A native systems language** producing standalone executables (already the accepted direction).
2. **The PHP power-backend story**: when a PHP application hits performance or capability limits, teams write the hot module in Doria and call it from PHP with near-zero friction, because the syntax is already familiar and the bridge is first-class (D13c). This is Doria's unique adoption wedge — no other native language offers PHP developers syntax continuity plus a generated, type-checked FFI bridge.
3. **A game engine written in Doria**, which drives requirements for: deterministic destruction (ownership/RAII — no GC pauses, no refcount traffic on hot paths), fixed-width numerics, floats and SIMD, unsafe/FFI for graphics/audio/input APIs, allocator control, and predictable value-type collections.
4. **A UI framework** integrating with PHP web backends, which drives requirements for: attributes-as-metadata, property hooks, closures, enums, pattern matching, and async.
5. **Portable terminal (TUI) applications as a first-class capability, not a happy accident.** The designer's PHP terminal game engines — Sendama (arcade-style terminal games) and Ichiloto (terminal-native 2D JRPG engine with a TUI editor) — lack native Windows support because they emit raw ANSI escape sequences. Doria fixes this at the platform level: a capability-based `std::term` layer (§9) abstracts Windows Console/VT and Unix termios+ANSI behind one API, so **user code never writes an escape sequence**, and the same TUI binary runs natively on Windows, macOS, and Linux. The intended port pattern for those engines is the D13c bridge run in reverse of the usual pitch: the engine core is rewritten in Doria (performance + portability) and compiled as a php-lib, while game projects keep their existing PHP scripting against generated stubs — or go full Doria against `std::term` directly.

**Long-range direction (informs architecture, never expands current scope): AI research, scientific computing, and data-intensive native systems.** Doria should eventually be able to grow a competitive numerical/AI ecosystem — as libraries plus compiler extension points, never as core-language AI syntax. Nothing AI-specific enters v1.0; instead, this plan threads *readiness constraints* through the stages it already has (§4.1 numerical semantics, §4.5 generics, §8.1 IR extensibility, §8.3 external memory, §8.4 compiler services, Stage 40 zero-copy FFI, §11 Baton data model), and the directional workstream lives in Appendix A (§16). The governing rule: prefer a recorded constraint or review gate over any new implementation task.

The plan sequences language work so that requirement sets 2–5 unlock in that order.

---

## 3. Memory model and safety: ownership and borrowing in PHP spelling (D1–D3, D12)

This is the foundational design. Doria adopts **Rust's ownership and borrow-checking model — the real mechanism, not an approximation** — and re-spells it entirely in vocabulary Doria already has. There is no tracing GC and no pervasive reference counting. What is deliberately absent is Rust's *surface*: no `&` / `&mut` sigils, no `'a` lifetime annotations, no `Box` / `&str` / `Rc<RefCell<T>>` vocabulary, and no borrow-checker jargon in diagnostics. The checker is Rust-grade; the spelling is PHP-grade.

The mapping in one table:

| Rust concept | Doria spelling |
|---|---|
| Ownership + move semantics | Plain assignment / plain argument passing of move types |
| Shared borrow `&T` | readonly — the existing default for bindings, parameters, `$this` |
| Exclusive borrow `&mut T` | `writable` |
| Consuming (by-value) parameter | `take` parameter modifier |
| `Drop` / RAII | `__destruct` runs at end of owning scope |
| `Copy` types | Primitives, `bool`, `string`, ranges, enums with Copy payloads |
| `Rc<T>` / `Weak<T>` | `Shared<T>` / `Weak<T>` stdlib types (opt-in shared ownership) |
| `RefCell<T>` interior mutability | `SharedMut<T>` with runtime-checked `writable` access |
| `Send` / `Sync` | Auto-derived `Sendable` / `Shareable` marker interfaces (Phase H) |
| Lifetimes | Inferred only, with fixed elision rules; never written in surface syntax |

### 3.1 Ownership and moves

- Every value has exactly one owner: a binding, a property, or a collection slot. When the owning scope ends and the value has not been moved out, the value is destroyed: `__destruct` runs immediately and deterministically, then memory is freed. This is RAII — files, GPU buffers, locks, and sockets close at scope exit with zero GC pauses and zero refcount traffic, exactly what the game engine needs.
- **Copy types** (primitives, `bool`, `string`, ranges, enums whose payloads are all Copy) duplicate on assignment and argument passing. The ownership machinery is invisible for them, which means most everyday PHP-shaped code never encounters a move at all.
- **Move types** (classes, `T[]` typed arrays, `List<T>`, `Dictionary<K, V>`, `Set<T>`, `Bytes`, closures, enums with move payloads): assignment and by-value passing transfer ownership. Using a moved-from binding is a compile error.
- Diagnostics use plain ownership vocabulary — *owns*, *gives*, *still using*, *readonly*, *writable* — never *borrow*, *lifetime*, or `'a`:

```text
error[D0203]: $user was given to store() on line 12, so it can no longer be used here
help: $user gave away ownership at line 12 and cannot be used afterward
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
- **The ordinary borrow rules** (compile-time only, zero runtime cost, checked non-lexically on MIR, excluding explicit `SharedMut<T>` dynamic access checks): at most one live writable borrow XOR any number of readonly borrows of the same value; no borrow may outlive the value's owner; a moved value cannot be borrowed. Non-lexical means a borrow ends at its last use, not at the end of a block, so idiomatic PHP-shaped code rarely fights the checker.
- **Place expressions borrow implicitly**: `$obj->prop`, `$list[0]`, and chained access borrow for the duration of the enclosing operation — no sigils at use sites.
- **`foreach` borrows elements**: `foreach ($users as $user)` takes a readonly borrow per iteration (the existing readonly-loop-binding rule, now real); `foreach ($users as writable $user)` takes exclusive borrows for in-place mutation, requiring the collection binding itself to be writable.
- **Returned borrows use fixed elision rules, never annotations.** In v1.0 a function or method may return a borrow only when it derives from `$this` or from exactly the one borrowed parameter — Rust's elision rules, which cover getters, views, and accessors. APIs needing multi-source lifetime relationships must restructure to return owned values or use `Shared<T>`. Named lifetime/region annotations are rejected for v1.0 and may only be revisited post-1.0 with concrete evidence, and even then never in Rust spelling.

### 3.3 Shared ownership is opt-in, not the default

When single ownership genuinely does not fit — caches, observer lists, doubly-linked structures, scene-graph back-references — the stdlib provides explicit shared-ownership types instead of silently changing the language model:

```doria
class Node
{
    writable List<Shared<Node>> $children = [];
    writable ?Weak<Node> $parent = null;
}
```

- `Shared<T>`: refcounted shared owner (Rust `Rc`). `->clone()` copies the handle cheaply; the value is destroyed when the last `Shared` handle drops. Access through `Shared<T>` is readonly.
- `SharedMut<T>`: adds runtime-checked writable access (Rust `RefCell` discipline): obtaining writable access while any other access is live panics with a clear message. This is the one place a dynamic check exists, and the type's presence in a signature advertises it.
- `Weak<T>`: non-owning handle; `->upgrade()` returns `?Shared<T>`. Strong `Shared` cycles leak by design (documented); `Weak` breaks them.
- Thread-safe variants arrive with Phase H under the `Sendable`/`Shareable` design record.

### 3.4 Why this fits Doria's products

The engine gets Rust's performance model: no GC, no refcount traffic on hot paths, deterministic destruction, aggressive alias-based optimization license from exclusive borrows. PHP developers get a shallow on-ramp: Copy types plus borrowed-by-default parameters mean ordinary code reads and behaves like the PHP they know, and ownership only announces itself where it earns its keep — move types, `take` signatures, and `Shared<T>` in type positions.

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

- `unsafe { }` is the only context where `Ptr<T>` / `MutPtr<T>` may be dereferenced, `extern` functions called, and ownership deliberately sidestepped (`Shared::intoRaw` / `Shared::fromRaw` style intrinsics for FFI handle passing).
- `extern "C"` blocks declare foreign symbols; parameter/return types restricted to FFI-safe types (fixed-width numerics, `Ptr<T>`).
- An `unsafe function` spelling marks a whole function as requiring an unsafe context to call.
- `declare` keys will later govern per-module unsafe policy (deny/allow), per decision 0028's directive direction.
- The safety contract is Rust's: `unsafe` code must uphold the invariants the borrow checker assumes; everything outside `unsafe` keeps full guarantees.

### 3.6 Panics

A panic is a fatal runtime error, distinct from checked `throw`/`throws` per decision 0035: arithmetic overflow, division by zero, out-of-bounds indexing, `SharedMut` access violation, failed `Float::toInt`, explicit `panic("message")`. Default behavior: print message + Doria stack trace to stderr and exit with status 101. **v1.0 panic policy is abort-only (no unwinding, no catching panics).** This keeps codegen simple and honest; checked errors are the recoverable path.

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
- Flow-sensitive narrowing: `!= null` / `== null` comparisons and `match` narrow `?T` to `T` inside the guarded region. This is the first path-sensitive analysis and lands as its own stage.
- Representation: `?T` for class types uses null pointers (zero cost); for other types a discriminant word (niche optimization is a backend improvement later).
- `mixed` remains the dynamic escape hatch for PHP-interop shapes; narrowing `mixed` requires `match` or explicit `is` checks (`$x is string`) introduced in the same stage as narrowing. `mixed` is deliberately unknown-flavored and boxed — its complete rules live in §4.8.

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

- Arms: enum case patterns with payload destructuring, literal patterns, `null` pattern, `default`. Guards (`Shape::Circle($r) if $r > 1.0 =>`) are a fast-follow stage.
- Non-exhaustive `match` over an enum or `bool` without `default` is a compile error.
- `when` (decision 0009) is the value-returning conditional chain; it lands after `match` since `match (true)` covers most needs, and its grammar gets its own decision record in Phase E.

### 4.5 Generics (D9)

```doria
function first<T>(List<T> $items): ?T
{
    // ...
}

class History<T>   // deliberately not Stack<T>: that name is reserved for a future stdlib collection (§4.9)
{
    internal writable List<T> $entries = [];

    writable function push(T $entry): void { /* ... */ }
    writable function pop(): ?T { /* ... */ }
}

function max<T implements Comparable<T>>(T $a, T $b): T
{
    return match (true) {
        $a->compareTo($b) >= 0 => $a,
        default => $b,
    };
}
```

- Monomorphization at MIR level: each concrete instantiation generates specialized code (Rust model — zero-cost, no boxing). Compile-time cost is accepted; the dev backend (Cranelift) keeps iteration fast.
- Constraint spelling `T implements Interface` keeps Doria's own vocabulary; multiple constraints with `+`? **No** — spelling is `T implements A, B` inside the angle brackets, comma-separated, matching `implements` lists.
- Generic type inference at call sites from argument types; explicit turbofish-style spelling is **not** adopted — where inference fails, bind through a typed declaration.
- Collections `List<T>`, `Dictionary<K, V>`, `Set<T>` become real generic types in the compiler (they already have checked arity) backed by runtime intrinsics, then by stdlib generic implementations as self-hosting matures.
- **Extension point (record 0050): compile-time value parameters.** Generic metadata, arity checking, and the monomorphization keying must not assume every generic argument is a type. Future numerical work may want value parameters (a `Buffer<float32, 4096>` shape of thing); v1.0 implements none of this, but the specialization machinery is designed so adding a value-parameter kind later is an extension, not a redesign.

### 4.6 Strings and Bytes (D16)

- `string`: immutable, UTF-8, and a Copy type (internally a refcounted immutable buffer, so copies are pointer-cheap). One string type deliberately avoids Rust's `String`/`&str` split — the single biggest spelling complaint this plan removes. `$s->length` is byte length; `$s->isEmpty`, `$s->bytes` accessor returning `Bytes` view (copy in v1.0).
- `Bytes`: mutable move-type byte buffer for binary work, file I/O, network buffers, engine assets; in-place mutation goes through `writable` borrows like everything else.
- **Display conversion (amends the earlier string-only `.` decision).** One canonical, locale-independent conversion feeds all three display contexts — string interpolation `{...}`, `.` concatenation, and `echo` — resolving SPEC §7's open display-conversion question:
  - `string` converts as itself; the `int`/`uint` family converts to decimal digits; the `float` family converts to the shortest round-trip decimal (deterministic, never locale- or ini-dependent, unlike PHP's `precision` behavior); `bool` converts to `"true"` / `"false"` — explicitly **not** PHP's `"1"` / `""`, whose silent empty-string `false` is a classic wart Doria refuses to inherit.
  - Classes convert only by implementing the built-in `interface Displayable { function toString(): string; }` — **this is Doria's answer to PHP's `__toString`**. There is no `__toString` magic method: the magic-name surface remains exactly `__construct` / `__destruct`, and string-conversion conformance is the nominal interface (PHP 8's own `Stringable` interface is the precedent). `doriac migrate php` rewrites a `__toString()` method into `implements Displayable` plus `toString()`. Interpolating or concatenating a non-Displayable class remains a compile error with an implement-Displayable suggestion.
  - Deliberately **not** display-convertible: `?T` (narrow or apply `??` first — no PHP-style silent empty output for `null`), `mixed` (narrow first, per D21), collections and typed arrays (no PHP `"Array"` wart), enums (deferred to record 0047), closures, and `Ptr<T>`.
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

`mixed` exists because Doria's strategic products need one place for dynamism to land: `std::json` values, PHP-bridge payloads, and `doriac migrate php` output. It is designed so that a hole in static *knowledge* is never a hole in *safety*:

- **Unknown-flavored, never any-flavored.** A `mixed` value permits no operations at all — no method or property access, no arithmetic, no concatenation or interpolation, no comparison — until it is narrowed by an `is` check or a `match`. Prove, then use. An any-flavored `mixed` (PHP's untyped reality) would punch a hole through monomorphization, Copy-vs-move classification, and the borrow checker simultaneously; Doria never permits that.
- **Implicit in, explicit out.** Assigning or passing any value into a `mixed` slot boxes it silently — acceptable because writing `mixed` in a signature is itself the deliberate opt-in (D20 guarantees `mixed` is never a silent default), and this inbound widening is exempt from D15's no-implicit-conversion rule by design. Outbound is never implicit: only `is` narrowing and `match` extract the payload; no cast spelling exists.
- **Always a move type.** `mixed` is a boxed, runtime-tagged value (a `dr_mixed` intrinsic in doria-rt) and classifies as a move type even when the payload is Copy — one uniform rule, no special cases in the checker. Narrowed access follows the binding's existing ownership: narrowing a readonly `mixed` yields readonly access to the payload; moving the payload out consumes the box (Copy payloads copy out instead). Full ownership interaction is specified in record 0069.
- **`object` does not exist.** "Any class instance" is just `mixed` plus a promise the type system cannot use: with runtime reflection out of scope, the only operation on such a value would be `is`-downcasting, which `mixed` already provides. Two dynamic boundary types where one suffices is precisely the PHP-shaped redundancy Doria eliminates elsewhere (`use`/`uses`/`with`, two-state visibility). Reintroduce post-1.0 only with concrete PHP-bridge evidence.
- **`null` is a literal, not a type-position name.** The null *type* exists internally (it is how `?T` assignment and narrowing are specified), but `null` in type position is rejected with a diagnostic suggesting `?T`. Docs list `null` under literals.
- **`resource` is reserved, not implemented.** Native Doria's resource story is RAII classes owning handles (plus `Ptr<T>` under `unsafe`). The `resource` name is reserved for the Phase I PHP bridge boundary and rejected until then with an unsupported-feature diagnostic; it does not appear in core type documentation except as reserved.
- **`void` is return-position only**; any other position is rejected with a diagnostic.

---

### 4.9 Typed arrays and named collections — there is no `array` (D22)

Doria has **no broad PHP-style `array` type**. `array $items` is invalid Doria; so is `List<array>`. The identifier `array` is not a type name and is rejected with a diagnostic pointing at this section's alternatives. The word may appear in Doria documentation and tooling only when discussing PHP backend lowering/output, PHP migration input, or explicitly rejected syntax.

What Doria has instead is a two-tier sequence model:

- **Typed arrays, spelled C-style `T[]`**: `int[] $numbers`, `string[] $names`, `mixed[] $items`, `int[][] $matrix`. A `T[]` is a contiguous, fixed-length-after-creation array — the engine-grade buffer type: length chosen at creation, elements read and written in place through ordinary readonly/writable borrows, no grow/shrink surface. `T[]` is a move type and participates in ownership and borrow checking exactly like every other move type; indexing borrows the element, `foreach` borrows elements, out-of-bounds indexing panics. (`Bytes` remains the dedicated byte buffer; whether `uint8[]` and `Bytes` interconvert, and how, is decided in record 0070.)
- **Named collections** are the growable/structured family: `List<T>` (the everyday growable sequence and default workhorse), `Dictionary<K, V>`, `Set<T>`. Future named collections — `Queue<T>`, `Stack<T>`, and similar — join this family under the same `PascalCase<T>` naming and the §9.1 charter.
- **Bracket literals are collection literals, not arrays.** `[1, 2, 3]` and `["a" => 1]` are contextually typed by the expected type: `int[] $a = [1, 2, 3];` builds a typed array, `List<int> $l = [1, 2, 3];` a list, `Dictionary<string, int> $d = ["a" => 1];` a dictionary. Without an expected type (`let $x = [1, 2, 3];`), a sequence literal defaults to `List<T>` and a keyed literal to `Dictionary<K, V>` — the growable PHP-intuitive reading — with the element/key/value types inferred from the elements (vetoable default; record 0070). `Set<T>` has no literal form in v1.0; use `Set::from([...])`.
- **Mixed-flow shapes are always valid Doria shapes**: `mixed[]`, `List<mixed>`, `Dictionary<string, mixed>` — never `array`. `std::json`, the PHP bridge, and migration output use these.

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
- **Checked propagation**: a call to a `throws`-declared function must be (a) inside a `try` whose `catch` arms cover every declared error type (covering = the arm type is the error class or a superclass/implemented interface), or (b) inside a function whose own `throws` clause covers the uncovered remainder. `main` may declare `throws Error`; errors escaping `main` are handled by the runtime as specified below.
- `catch (Error $e)` is the catch-all. Rethrow is plain `throw $e;`.
- `finally` runs on normal exit, thrown-error exit, and early `return`; it may not `return`, `throw`, `break`, or `continue` (avoids PHP/Java's swallowed-error trap; compile error).
- Lowering: `throws` functions return a hidden discriminated result in the native ABI (no unwinding — consistent with the abort-only panic policy and cheap for the engine). The PHP backend lowers to native PHP exceptions.
- `throw` is a statement in v1.0; expression-position `throw` (PHP 8 style) is a fast-follow.

**Errors escaping `main` — the caller of last resort.** `main` is called by doria-rt's entry glue (`dr_main`), so the runtime is the caller of last resort and its behavior is language-specified, not incidental:

- Because `throws` lowers to a hidden discriminated result (no unwinding), an error propagating out of `main` travels through ordinary returns, and drop elaboration runs `__destruct` at every scope boundary on the way out exactly as on the success path — files flush, sockets close, locks release. An escaping checked error is an *orderly, declared* failure; contrast panics, which abort with no cleanup and exit 101.
- `dr_main` then prints `error: <ClassName>: <message>` to stderr — the class name via a minimal type-name intrinsic (drop glue already carries per-type metadata; this is not reflection and must not grow into one) and the message via the `Error` interface's guaranteed readonly `string $message` — destroys the error value, and exits with status **70** (BSD `EX_SOFTWARE`). Never 101.
- The 70/101 split is machine-readable triage: a supervisor, orchestrator, or PHP frontend distinguishes "declared failure" (70) from "Doria bug" (101) without parsing stderr.
- Checked errors carry **no stack traces by default**: they are values and ordinary control flow, and trace capture at every `throw` would tax exactly the hot paths the result ABI keeps cheap. Panics keep traces; errors keep messages. A dev-profile opt-in (trace capture at throw sites under an environment flag) may be added later within record 0049's scope.
- `async function main` is permitted: the entry glue bootstraps the executor with `main` as the root task, and structured concurrency guarantees no orphan tasks remain when the root task completes with an error — child scopes have already awaited or cancelled their tasks before propagation continues. A synchronous `main` never starts the executor, so non-async programs pay zero async cost. Bootstrap details land with record 0063 / Stage 38.
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

- `static` properties/methods with PHP spelling `ClassName::member()`; static properties follow readonly/writable rules; writable statics are per-process globals and are rejected in `Sendable`/`Shareable`-checked concurrency contexts later.
- `const NAME = expr;` class constants and namespace-level constants; const expressions are compile-time evaluated over literals, arithmetic, and other consts (this defines the first compile-time evaluation tier, which attributes will reuse).

---

## 7. Namespaces, source organization, closures

- Implement decision 0028 as written: `namespace App\Services;`, file-scope `use ... as ...`, string-literal include-once `include`, structured `declare`. Multi-file compilation units are the enabler stage for everything package-shaped.
- Name resolution: a compilation invocation takes a root set of files (later, Baton passes it); symbols resolve by fully qualified name; unqualified names resolve via current namespace then `use` imports; duplicate symbol definitions across files are compile errors with both spans.
- First `declare` keys (each rejected until implemented): `declare(overflow: "wrapping");` (module-local, D4 relaxation for engine hot paths), `declare(unsafe: "deny");`.
- **One word, one meaning.** PHP overloads `use` with at least three jobs (namespace import, trait composition, closure capture). Doria splits them permanently: `use` is namespace import/alias only, `uses` is trait composition only, and `with` is closure capture only. The parser, diagnostics, editor grammars, and the migration converter (`use (...)` capture clauses rewrite to `with (...)`) must all treat these as three distinct keywords; none of the three is ever accepted in another's position.
- Closures (D10):

```doria
let $double = fn(int $x) => $x * 2;                  // typed parameter, inferred return
let $adder = function (int $x): int with ($base) {   // typed parameter, explicit capture
    return $x + $base;
};
```

Closure parameters are declarations, so every parameter requires an explicit type. Doria does not infer omitted parameter types for anonymous functions, arrow functions, callbacks, collection methods, property hook setters, or any other function-like form. A closure's return type may be inferred for arrows and may be declared for anonymous functions, but parameter types are never optional.

The capture clause is spelled `with`, never PHP's closure `use`. Captures are borrows by default: `with ($base)` and arrow-function auto-capture take readonly borrows; `with (writable $counter)` takes an exclusive borrow; `with (take $conn)` moves the value into the closure. A closure holding borrows is itself borrow-bound, so it cannot outlive or escape the captured variables' scope — no bespoke escape analysis is needed, the borrow checker rejects it and the diagnostic suggests `take`. Closures are move-typed values with type spelling `function(int): int` in type position; `Callable<...>` alias is not adopted.

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
- Drop-glue dispatch, `Shared<T>`/`Weak<T>` refcount and upgrade machinery.
- String/Bytes/List/Dictionary/Set intrinsic implementations (refcounted immutable string buffers, owned growable collection buffers, hashing).
- Panic machinery, stack trace capture, process entry glue (`dr_main` wrapping user `main`).
- stdout/stderr/stdin, basic clock, environment access — the syscall surface the stdlib wraps.

Record 0044's ABI review must evaluate, as named design cases before the native object representation freezes: externally owned memory (buffers doria-rt did not allocate), custom deallocation callbacks, alignment requirements, and pinned/non-moving memory for interop. The ownership model already guarantees the hard part — stable addresses, deterministic release, no movable-GC assumption — so these are representation questions, not model changes.

All symbols `dr_`-prefixed, internal ABI, versioned in lockstep with the compiler.

### 8.4 Diagnostics

Adopt error codes now (`D0001`-style) before the count explodes; every diagnostic carries code, span(s), message, and machine-applicable suggestion where possible; `doriac check --json` for tooling; LSP reuses the same diagnostics verbatim (already the architecture). Architectural goal, standing from now: **CLI commands wrap reusable compiler services** (in-memory parse/check, diagnostics, module compilation, interpreter execution) rather than owning compiler behavior directly, so future REPL, notebook, and incremental tooling never needs a second frontend. Incremental compilation itself stays deferred.

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

Two layers, both written in Doria as early as possible (self-hosting on-ramp):

- **core** (no I/O, always available): `Int`/`Int8`.../`Float`/`Bool`/`String` companion APIs (`Int::parse`, `Int::toFloat`, `Int::wrappingAdd`, ...), `Option`-free nullable helpers, `Cloneable` (Stage 35 interface contract), `Shared<T>`/`Weak<T>`/`SharedMut<T>`, `Comparable<T>`, `Equatable<T>`, `Hashable`, `Displayable`, `Error`, `Iterable<T>`/`Iterator<T>` (Stage 35 user conformance; collections use compiler-internal iteration earlier), range types, `math` basics, and a PHP-familiar free-function layer (`get_time`, `str_starts_with`, `str_case_compare`, ...) that wraps the method/companion surface — regularized names only, never PHP's fused spellings.
- **std** (hosted): `io` (files, stdin/out streams), `fs`, `env`, `process`, `time`, `random`, `json` (drives enum/match/mixed ergonomics and the PHP bridge), `net` (TCP first), later `http`, and `term` — the portable terminal layer for product 5. `std::term` is capability-based on the crossterm model: raw mode, non-blocking input decoded to payload enums (`KeyEvent::Char(string $char)`, `KeyEvent::Up`, resize events), cursor positioning, styling/color, screen size and clearing — with per-platform backends (Windows Console API / VT processing; Unix termios + escape emission) hidden behind the API. Raw ANSI is an implementation detail of the Unix backend, never the public surface. The canonical high-level API is the **`Console` class** — a static facade so the user never checks what macOS/Linux/Windows supports — covering terminal info (size, interactivity, color capability), screen (clear, title, alternate screen later), cursor (position, move, show/hide), styled output, and input (blocking `readKey`, non-blocking `pollKey`, resize events). Raw mode is entered through an ownership guard whose `__destruct` restores the terminal — RAII closing the classic wedged-terminal bug on every structured exit (the one exception is an abort-only panic while in raw mode, which runs no cleanup per record 0081; a panic-hook restoration is possible future work). `Console`'s design ancestor is the designer's TermUtil PHP library (improved upon, not copied — TermUtil is ANSI-powered by definition; `Console` is capability-based by definition); the exact method inventory is decided in record 0072 with TermUtil as the reference input (a source-derived mapping note accompanies this plan). Two constraints from that source review bind 0072: **no `std::term` public type may carry escape sequences or platform encodings in its values or API** (TermUtil's `Color` enum is backed by ANSI strings — the exact pattern that made it unportable; Doria's `Color` means colors, and each backend renders them), and the **shadow screen-buffer question** (TermUtil's `Grid`/`charAt` read-back, half of a flicker-free diffing renderer) must be decided explicitly — stateless `Console` vs a separate `ScreenBuffer` type — never bolted on silently. The `Console` name is reserved in the stdlib namespace from now.

**Formatted I/O — the v1.0 minimal set (record 0071).** Doria ships a deliberately small, PHP-familiar formatted-I/O surface in the free-function layer; the broader PHP string-function catalogue (php.net's `ref.strings`) is a post-1.0 expansion, imported case by case under the §9.1 charter with regularized names, never wholesale.

- **`sprintf(format, ...args): string` and `printf(format, ...args): void`** are compiler-known functions with **compile-time-checked format strings**: the format must be a string literal (or const) in v1.0, and the compiler verifies specifier/argument count and types — `%d` against a non-integer is a compile error, not a runtime surprise. This is Rust's checked-`format!` guarantee delivered through a PHP spelling, and it is the reason format strings are safe to have at all in a language with Doria's discipline. v1.0 specifier subset: `%s` (any display-convertible value, §4.6), `%d` (int/uint family), `%f` with `%.Nf` precision (float family), `%x`/`%X`/`%o`/`%b` (integer bases), width / `-` left-align / `0` zero-pad flags, and `%%`. Everything else (`%e`, `%g`, positional `%1$s`, dynamic format strings) is deferred with a specifier-not-supported diagnostic. `printf` returns `void` — PHP's returns-byte-count behavior is dropped as charter noise.
- **`read_line(): ?string`** reads one line from stdin, strips the trailing newline, and returns `null` at EOF (never PHP's `false`). v1.0 takes no prompt argument; print prompts with `echo`. Richer stdin APIs live on the `std::io` stream types. The Stage 17 free-function family is charter-uniform verb_noun throughout: `read_line`, `read_file`, `write_file`, `write_stderr` — **not** PHP's `readline`, which is a `strlen`-style fusion (and, fittingly, ships from ext/readline, absent on Windows PHP — the exact gap Doria closes). `sprintf`/`printf` are *not* counterexamples: they are whitelisted as industry-universal single lexemes (C, PHP, Go, Java all spell them this way), the same whitelist tier as `id` and `json`; `readline` has no such cross-language standing and does not qualify.
- **`read_file(): string` is the text tier of a deliberate three-tier I/O family — not the whole story.** Because `string` is immutable UTF-8, `read_file` is *by definition* the text-file function: it validates UTF-8 on read and has defined failure behavior — invalid bytes never enter a `string` (the type's invariant is load-bearing for the whole language). The binary tier is `read_file_bytes(): Bytes` / `write_file_bytes(...)`, landing with `Bytes` in Stage 23, for assets, saves, images, and any encoding-agnostic work. The streaming tier — `File`/stream objects with RAII close via `__destruct`, buffered readers, seek — lands after checked errors exist (post-Stage 29), because serious file APIs want `throws`, not panics. **Failure-semantics migration (record 0075):** until Stage 29, I/O failures in these free functions panic with a clear message; at Stage 29 they migrate to declared `throws` signatures — this signature change is planned, recorded, and announced, never a surprise.
- **PHP-spelling fixits**: the unknown-function diagnostic recognizes well-known PHP spellings and suggests the charter name — `readline` → "did you mean `read_line`?", `strcasecmp` → `str_case_compare` — so PHP muscle memory becomes a one-keystroke teaching moment instead of friction. The suggestion table is maintained alongside the free-function surface and reused by `doriac migrate php`.
- **`print` is not included — ever.** It is `echo` with a vestigial return value; two spellings for one construct is exactly the PHP redundancy Doria eliminates (`use`/`uses`/`with`, no `object`). `echo` is the one output spelling; the name `print` is rejected with a use-`echo` diagnostic so it can never drift into userland-looking-like-builtin.
- **`sscanf` is deferred, not spelled.** Its shape — runtime-format-determined result arity/types, or by-reference out-parameters — fundamentally fights static typing (Rust has no scanf either, and Doria has no tuples to receive one). v1.0 parsing is `Int::parse`/`Float::parse` returning `?T`, the `str_` functions, and `match`/narrowing; a compile-time-checked scan design may be revisited post-1.0 in record 0071's follow-up.

**Batteries-included game/graphics math (record 0074).** Game development is a first-class Doria application, so the common math a game or renderer needs ships in std rather than being every project's first wheel to reinvent: `Vector2`/`Vector3`/`Vector4`, `Quaternion`, `Euler`, `Matrix3x3`/`Matrix4x4` (a "competent" 3D library without matrices isn't one — transforms need them), plus lerp/clamp/easing scalar helpers alongside the existing `math` basics. Three foundational constraints make this fast rather than merely present:

- **Math types are built-in Copy value types with inline layout** — they join the `string`/ranges/Copy-enum tier, *not* the class tier. A `Vector3` as a heap-allocated move type would make vector arithmetic unusable (moves and `->clone()` on every hot-loop operation); as an inline Copy aggregate it costs what three floats cost. D3 is untouched — there is still no *user-defined* `struct` in v1.0; these are stdlib-defined, compiler-known types, and payload enums already prove the compiler supports inline Copy aggregates.
- **Arithmetic operators on math types are compiler-known**, exactly as they are for `int` and `float` — `$a + $b`, `$v * 2.0`, `==` — which does **not** open user-defined operator overloading (§14 unchanged; the general operator/numerical protocol stays parked in Appendix A). Beyond operators, the API follows the charter: `$v->length` and `$v->normalized` as properties, `$v->dot($other)` / `$v->cross($other)` as methods.
- **`float32` variants and SIMD**: record 0065 (SIMD direction) treats `std::math` as its first consumer — `Vector3`/`Vector4` over `float32` map directly onto SIMD lanes, so layout decisions in 0074 and 0065 are made together.

Implementation lands as Stage 47 in Phase J, ahead of the engine-seed flagship demo; the *constraint* on Stage 19's object-layout work applies now.

`foreach (collection as ...)` uses compiler-internal iteration machinery in Phase D for built-in collections. The public `Iterable<T>` / `Iterator<T>` protocol that makes user types iterable lands with interface conformance in Stage 35.

### 9.1 Naming charter and the built-in/userland boundary (D19)

PHP's standard library is the cautionary tale this charter exists to prevent: `strlen` vs `str_replace` vs `nl2br`, camelCase methods beside snake_case functions, and needle/haystack argument order that flips between functions. Doria's built-in surface follows one law, enforced by API review and a `doriac` lint over the stdlib:

- **Casing**: built-in **free functions** are `snake_case` (`get_time`, `str_starts_with`, `sprintf`). **All member-style APIs are camelCase** — standard-library members and companion/type APIs included (`Int::wrappingAdd`, `Int::parse`, `$s->isEmpty`, `Shared::intoRaw`) — the convention PHP's own built-in classes already use, so `DateTime::createFromFormat`-style familiarity carries straight over. Userland free functions are camelCase (see the boundary below). `__construct`/`__destruct` keep their PHP spellings as keywords-in-disguise. Classes, interfaces, traits, enums, and enum cases are `PascalCase`. Constants are `SCREAMING_SNAKE_CASE`. Type parameters are single capitals (`T`, `K`, `V`).
- **Canonical member-casing examples (normative; preserve these spellings):** `Int::wrappingAdd`, `$s->isEmpty`, `$response->retryAfter`, `$repo->findById(...)`, `$request->tenantId`. Member-style APIs are camelCase on both the built-in and userland sides; these exact spellings are normative exemplars guarded by CI (`scripts/check_docs_authority.php`) and must never be renamed, reformatted, or converted to snake_case in any future edit of this document.
- **No contractions**: `length` not `len`, `to_string` not `strval`, and — the emblematic case — **`str_case_compare`, not `strcasecmp`**. PHP's `str_` free-function family is kept as a familiar, whitelisted domain prefix, but everything after the prefix is fully worded and underscore-separated; `strlen`-style fusions never appear. Other whitelisted abbreviations are only those more recognizable than their expansions (`str`, `id`, `min`, `max`, `io`, `http`, `json`, `utf8`) plus industry-universal single lexemes that transcend any one language's spelling (`printf`, `sprintf`). Whitelisting is always explicit and documented here — the law stays exception-documented, never exception-riddled; PHP-only spellings like `readline` do not qualify.
- **Symmetric pairs**: conversions always pair as to-X / from-X / try-from-X in the casing of their context (`toFloat` / `tryFrom` as members; `str_`-family spellings in free functions); lifecycle verbs pair predictably (`open`/`close`, `push`/`pop`, `add`/`remove`, `intoRaw`/`fromRaw`).
- **Predicates** read as questions: `is`/`has`/`can` prefixes (`isEmpty`, `hasKey` as members; `is_`/`has_` in free functions) — and per SPEC §6's nouns-are-properties rule, argument-free ones are properties (`$s->isEmpty`), never `get`-prefixed methods.
- **Uniform argument order**: the subject always comes first (it is `$this` on methods); options and callbacks come last. No needle/haystack roulette.
- **One name per concept** across modules: it is `count` everywhere (never `size` or `length` for collections), `contains` everywhere.

**The free-function boundary — and why it lives only there.** A member never needs casing to declare its provenance: the receiver already does that work. In `Int::wrappingAdd(...)` or `$s->isEmpty`, the `Int::` / string-typed receiver *is* the tell that this belongs to the standard library, so members are simply camelCase on both sides — one rule, no historical camelCase/snake_case mish-mash inside any codebase. A free function has no such prefix, so for free functions casing carries the provenance instead: snake_case is Doria's built-in signature (`str_case_compare($a, $b)` is the language), and **userland free functions are camelCase by convention** (`normalizeTitle($post)` is the application). Userland remains free to choose its own style — but Doria's docs, examples, and tooling never teach snake_case for userland free functions and subtly, consistently encourage camelCase. Enforcement:

- All documentation, SPEC examples, `baton new` templates, LSP snippets, and generated code (`#[Derive(...)]` members) write userland declarations in camelCase; this plan's own examples model it (`loadUser`, `findById`), and conformances to built-in interfaces always keep the interface's member spelling (`compareTo`, `toString`).
- A default-on lint gives a gentle, silenceable hint when userland declares a snake_case **free function** ("this reads as a Doria built-in; camelCase is the userland convention") — encouragement, not an error, silenceable per-declaration and per-module. Methods are exempt: the receiver already carries provenance, so there is no member boundary to protect.
- `function_exists("name")` is a compile-time predicate usable in top-level `if` to conditionally declare a function. This is the sanctioned collision/polyfill mechanism: guarded declarations may adopt the built-in's snake_case name because they deliberately stand in for one (e.g. back-filling a newer stdlib function on an older Doria); outside such a guard, userland free functions stay camelCase. `function_exists` is const-evaluated — there is no runtime symbol table.
- The generated PHP FFI stubs mirror the exported Doria class's own casing, so a `#[PHPExport]` class written in charter-compliant userland camelCase lands in PHP looking like idiomatic PSR code — a free win for the bridge.

Every stdlib decision record cites this charter, and `baton fmt` plus the stdlib lint enforce it mechanically.

Stdlib API style follows SPEC §6's nouns-are-properties rule and the collection method surface gets its own decision record (List: `add`, `insertAt`, `removeAt`, `contains`, `count` property, `isEmpty` property, `map`/`filter`/`reduce` after closures land; Dictionary: `get` returning `?V`, `set`, `remove`, `has`, `keys`, `values`; Set: `add`, `remove`, `has`, `union`, `intersect`). The typed-array `T[]` surface (`length` property, iteration, slicing) is decided alongside in record 0070, and future named collections (`Queue<T>`, `Stack<T>`, ...) follow the same charter.

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

- Exported surface restricted to a bridgeable type set: numerics, `bool`, `string`, `Bytes`, `?T` of those, `T[]`/`List`/`Dictionary` of bridgeable types, and `#[PHPExport]` classes (marshaled as opaque handles rooted as `Shared<T>` on the Doria side; the generated PHP stub holds the handle and releases it in its `__destruct`).
- **Export is metadata, not visibility.** `#[PHPExport]` never changes Doria accessibility, adds no modifier, and is not a third visibility state — the exported surface is the class's externally accessible, bridgeable members, and `internal` members never cross the boundary. Unsupported signatures are compile-time errors. Diagnostics speak Doria's model and never suggest `public`/`protected`/`private`.
- `throws` errors surface as generated PHP exception classes. An error escaping an exported function must surface as that exception and never terminate the host PHP process — the bridge is an FFI boundary, not a process boundary. Panics are the documented exception: under v1.0's abort-only policy a Doria panic aborts the hosting PHP worker (exactly as a crashing native extension would), which is why exported surfaces should prefer `throws` APIs over panicking ones.
- Transport: **FFI is the bootstrap transport, not the product.** Stage 41 ships a versioned C-ABI bridge (sized types, opaque handles, pointer+length views, status/result returns — no cross-language unwinding, no Zend structures, no Doria object layout exposed) plus generated PHP ≥ 8.0 `FFI` stubs (zero build tooling on the PHP side). A generated Zend-extension transport (`--php-ext`) is the **intended production transport** later; both transports consume the same higher-level bridge contract so FFI never accidentally defines the permanent semantics, and that same contract must remain reusable by the deferred embedded-host product (d).
- Threading: v1 bridge is single-threaded per PHP request (matches PHP's model). Recorded now as a standing bridge invariant feeding record 0063: **a PHP runtime context and its values belong to a designated thread**; `Sendable`/`Shareable` are never permission to move PHP-runtime-affined values across threads. Boundary concurrency means validating and copying/transferring typed data in, running native work on Doria workers, and returning results to the PHP-owning thread.
- This product plus `std::json`/`net` also covers the sidecar pattern (Doria service, PHP client), but the in-process bridge is the headline.

### 10.4 Product (d): native Doria → embedded PHP host (deferred)

The reverse direction — a native application hosting PHP as a first-class scripting language (the Lenga engine pattern: PHP gameplay scripts over a native core; also the port pattern's endgame for Sendama/Ichiloto) — is part of the strategy but **implementation-deferred**: no stage exists for it in this plan and none may be added until the product-(c) bridge contract is stable and the designer separately approves it. What this plan does now is refuse to foreclose it: Stage 41's ABI and ownership design must be transport- and direction-neutral (§10.3), and the eventual Doria-facing host API surfaces typed concepts (`PhpRuntime` / `PhpValue` / `PhpObject`-shaped — exact names unsettled) rather than `zval` internals, with any low-level access behind explicit unsafe/trusted boundaries. Everything else about product (d) lives in record 0062's open-questions list, not in v1.0 scope.

---

## 11. Baton and developer experience

Baton lands mid-plan (Phase F), once multi-file compilation exists to orchestrate:

- `baton new <name>` (binary/lib/php-lib templates), `baton build [--release]`, `baton run`, `baton test`, `baton check`.
- The command boundary is permanent: **doriac = compiler and compiler-facing inspection; Baton = project, package, workspace, build, test, benchmark, and dependency tooling.** Neither absorbs the other's job. (`baton bench` and richer workflows are future Baton commands, not MVP scope.)
- Manifest `Baton.toml` (TOML, human-edited): package name, version, edition placeholder, dependency table (path dependencies first; registry protocol deferred to post-1.0 — do not build a registry server in this plan). A generated `Baton.lock` records resolution for reproducibility and is never hand-edited; its encoding (TOML vs JSON) stays open until record 0060.
- **Resolver data-model constraint (record 0060):** the manifest and resolver must not assume every dependency is target-independent pure Doria source — platform/architecture constraints, feature selection, native libraries, and binary artifacts are future realities the data model leaves room for, even though none ship in the MVP. Reproducibility (deterministic resolution, recorded compiler version and target, build profiles) is a founding Baton property, not a retrofit.
- `baton test` defines the Doria test convention: `tests/*.doria` files whose functions marked `#[Test]` run and report (first real consumer of attributes).
- **Release versioning (record 0073): calendar versioning for the toolchain, SemVer for packages.** Doria/doriac/baton/stdlib releases use the common Ubuntu-shaped CalVer `yyyy.mm.n` — year, zero-padded month (01–12), release number — so a version's age is readable at a glance (`2026.07.1`). The month is the month the release actually ships (stamped at release time, never a slipped target), which keeps the age-readability promise honest; zero-padding keeps versions sorting lexically as well as numerically. `n` starts at 1 and increments monotonically within a month across all channels: prereleases consume release numbers (`2026.07.1-canary`, then `2026.07.2-rc`), they never use dotted suffix counters, and a prerelease chain that crosses a month boundary simply picks up the new month's prefix. The suffix marks the channel — `-canary` (experimental/moving) and `-rc` (release candidate) are the fixed set — and an unsuffixed version is a stable release, with same-month patches as further `n` values. Ordering: numeric on the triple, suffixed before unsuffixed at the same triple. **Every release before the 1.0 gate carries a suffix; the first unsuffixed release ever is the 1.0-gate release.** Compatibility is *not* the version's job: language-rule changes ride the `Baton.toml` edition mechanism, and **packages in the Baton ecosystem version by SemVer** — `^`/`~` resolution semantics require it, so the record-0060 resolver treats toolchain CalVer and package SemVer as distinct schemes and never range-matches against toolchain versions.
- Baton drives `doriac`; it never owns semantics. LSP/editors gain workspace awareness from `Baton.toml`.

---

## 12. Decision records to author (numbering continues from 0037)

0043 MIR + interpreter oracle (§8.1–8.2) is already authored and accepted in `docs/decisions/0043-stage-11-mir-and-interpreter-oracle.md`.

0038 ownership and move semantics (D1, D3) · 0039 borrowing rules, elision, and the borrow checker (D2) · 0040 panics & overflow policy (D4, §3.6) · 0041 division/modulo/shifts (D14) · 0042 numeric conversions (D15) · 0044 doria-rt ABI incl. external-memory design cases (D18, §8.3) · 0045 runtime strings/Bytes & canonical display conversion incl. the amended `.` (D16, §4.6) · 0046 nullable types & narrowing (D5) · 0047 enums & payload cases (D6) · 0048 match (D7) · 0049 checked errors full semantics incl. errors escaping `main` (D8, §5) · 0050 generics & monomorphization incl. the value-parameter extension point (D9, §4.5) · 0051 collections runtime & API surface (§9) · 0052 compiler-internal iteration machinery · 0053 inheritance/open/override (D17) · 0054 interfaces, traits, Cloneable, and public Iterable conformance · 0055 property hooks · 0056 statics & const evaluation · 0057 closures (D10) · 0058 namespaces implementation notes (elaborating 0028) · 0059 attributes & compile-time evaluation policy · 0060 Baton manifest, lockfile encoding, resolver data model & test convention · 0061 unsafe/FFI incl. zero-copy numerical exchange (D12) · 0062 php-lib bridge: transport-neutral contract, export analysis, thread-affinity invariant, and the interop handoff brief's open-questions catalogue (D13c/d) · 0063 async & Shareable (D11) · 0064 when grammar · 0065 SIMD/engine intrinsics direction · 0066 `Shared<T>` / `Weak<T>` / `SharedMut<T>` · 0067 naming charter, built-in/userland boundary, and `function_exists` (D19) · 0068 explicit typing discipline (D20) · 0069 dynamic boundary types: `mixed`/`object`/`null`/`resource`/`void` (D21) · 0070 typed arrays `T[]`, collection literals and their defaults, and the rejection of `array` (D22) · 0071 formatted I/O: checked `sprintf`/`printf` (whitelisted lexemes), `read_line` + file-I/O family, PHP-spelling fixits, `print` rejected, `sscanf` deferred (§9) · 0072 `std::term` portable terminal layer & the `Console` class API (TermUtil as reference input; §9, product 5) · 0073 release versioning: toolchain CalVer `yyyy.mm.n` + channels, package SemVer (§11) · 0074 `std::math` geometry: built-in Copy value types, compiler-known operators, SIMD coordination (§9, Stage 47) · 0075 I/O family tiers & failure-semantics migration: text/binary/stream, panic→`throws` at Stage 29 (§9). · 0081 destruction order (Stage 19): reverse-declaration/initialization drop of still-owned locals, temporaries, and properties; `__destruct` body before property drops; move removes cleanup obligation; assignment acquires replacement before dropping previous; abort-only panic runs no cleanup — with the deliberate divergences and consequences recorded below · 0082 private native class representation (Stage 19): headerless data-only payload with static per-type drop glue; concrete drops inlined; interface dispatch committed to fat pointers at Stage 35; `doria-rt`-private and versioned. · 0083 Stage 19 ownership mechanics: `take`/`writable` mutually exclusive; promoted move-type params require `take`; explicit-then-promoted property order; readonly bindings may be moved but reinitialization needs `writable`; self/overlapping moves rejected and direct move-into-properties deferred; temporary native-eligibility soundness gate (lifted at Stage 21); `class allocation failed` panic.

Each record follows the existing template: context, decision, alternatives considered, consequences, affected components.

**Numbering policy.** The numbers in the list above are *subject labels, not assignments*. Actual record numbers are assigned at authoring time from the repository's next unused number, because repo reality diverges from this list's speculative numbering (e.g. Stage 17 landed as 0074/0075, Stage 18 as 0079, and the lifecycle-shapes amendment as 0080). When authoring, verify the next free number in `docs/decisions/` rather than trusting a number here.

### Record 0081 detail — destruction order (Stage 19)

Observable rules: (1) owned locals and temporaries are dropped in reverse order of initialization, among values still owned at the exit point — moved-out and never-initialized values are skipped; the Stage 19 definite-initialization analysis is what makes "still owned" statically known. (2) Owned temporaries within an expression die at the end of the enclosing statement (the `;`), reverse creation order, after the statement's result is bound. (3) On class destruction the user's `__destruct` body runs first, then owned properties drop in reverse declaration order, then the allocation is freed. (4) Moving a value removes it from the source's cleanup obligations. (5) Assignment fully evaluates and acquires the replacement before dropping the previous destination value. (6) Normal structured exits (`return`/`break`/`continue`/fallthrough) run cleanup; abort-only panic does not.

**Deliberate divergence to record:** properties drop in *reverse* declaration order — this diverges from Rust (which drops struct fields in forward order, an asymmetry with its reverse-order locals) and matches C++. Chosen so the whole language is uniform: everything dies in reverse of construction, locals and properties alike (properties initialize top-to-bottom, so reverse-declaration = reverse-initialization). A contributor arriving from Rust expects the opposite and must see this was deliberate.

**Consequence to record (not solve here):** rule (6) means a panic while a RAII guard is live does not run the guard — so the `Console::rawMode` "wedged terminal is structurally impossible" property carries one asterisk: a panic in raw mode will not restore the terminal. This is inherent to the accepted abort-only panic model (record 0040), not a Stage 19 defect. A minimal panic-hook restoration is a possible future addition, out of scope now; the Console narrative must state the asterisk honestly.

### Record 0082 detail — private native class representation (Stage 19)

An owned class instance is a headerless, data-only heap payload: an opaque pointer to fields at compiler-known offsets, with immutable per-type **static** metadata (size, alignment, drop glue) held once per type, never per object. Static ownership makes drop glue known at every cleanup site, so no per-object type tag is needed; no reflection means no runtime type soup; data-only layout is what FFI/zero-copy and the future inline-Copy math aggregates want.

**Consequences to record (they bind later stages):** (a) **Interface dispatch (Stage 35) is committed to fat pointers** — data ptr + vtable ptr — not per-object headers; recording it now prevents Stage 35 from reintroducing headers after the representation has shipped without them. (b) **Concrete owned drops are statically resolved and inlinable, zero indirection** — the static drop-glue metadata is consulted only for abstracted drops (fat-pointer/interface values, generic drop); dropping a value of known concrete type compiles to a direct/inlined call with no metadata lookup. The inline-Copy `std::math` aggregates remain a separate representation path as §9/record 0074 require; both paths share the no-per-object-header property, so the distinction between them is move-vs-Copy and heap-vs-inline, never presence of metadata.

### Record 0083 detail — Stage 19 ownership mechanics

Resolves the implementation-critical rules Stage 19 exposes that 0081/0082 don't cover:

- **`take` and `writable` are mutually exclusive parameter modes.** Both `take writable` and `writable take` are rejected — they answer different questions (ownership transfer vs exclusive borrow), and having taken ownership, exclusivity is moot.
- **Promoted move-type parameters require `take`.** A promoted move-type param transfers directly into its property; without `take` it would create two owners, which the ownership model forbids. So `function __construct(take Person $manager)` is the promoted spelling; a promoted move-type param without `take` is an error with a fixit that inserts it. Copy-type promoted params are unchanged.
- **Property order is explicit-then-promoted:** explicit properties in class-body order, then promoted properties in constructor-parameter order. Construction follows this total order; destruction (record 0081) reverses it. The total order is what makes reverse-declaration drop well-defined across both property kinds.
- **Readonly bindings may be moved from; reinitializing a moved-from binding needs `writable`.** Principle: a move is not a mutation of the binding — it is the end of the binding's ownership — so `readonly` (which governs mutation) does not forbid it. Reinitializing a moved-from binding *is* a new assignment, i.e. mutation, so it requires `writable`.
- **Self-move and overlapping source/destination moves are rejected** (`$value = $value` and taking a nested owned property from an aliasing path). Direct moves into (nested) owned properties stay unsupported until explicitly specified — they entangle with the writable-path rules and are not improvised here.
- **Temporary native-eligibility soundness gate.** Full constructor definite-initialization is Stage 21, but Stage 19 emits native code and must never hand out uninitialized class storage. So Stage 19 natively constructs only classes whose every property is provably initialized via a property initializer, promotion, or the existing narrow direct constructor-init forms; anything not provable gets a clear "unsupported until Stage 21" diagnostic, never uninitialized memory. This is a soundness gate, explicitly temporary, explicitly lifted when Stage 21's definite-initialization analysis lands.
- **Allocation failure** uses the status-101 panic path with the canonical message `class allocation failed`. Allocation failure is OOM — the one place abort-only cleanup is unambiguously correct, since no cleanup is possible mid-allocation.

---

## 13. Phased roadmap with stages and acceptance criteria

Stages continue the existing numbering. Every stage = decision record(s) + tests + docs + examples, per Section 0. "AC" = acceptance criteria.

### Phase A — Real native foundation (Stages 11–15)
Retire the smoke architecture; make the native path general.

- **Stage 11 — MIR + interpreter oracle.** Introduce MIR; port all Stage ≤10 lowering onto it; delete `NativeSmokeModule`; ship the MIR interpreter as `--target debug`; stand up the differential test harness. AC: every existing native example produces identical output under interpreter and Cranelift; no smoke-module code remains.
- **Stage 12 — General control flow + runtime foundation.** Arbitrary/nested loops, `return` anywhere, unbounded `while`, `break`/`continue` everywhere, recursion and mutual recursion; shared dataflow framework replaces the final-statement-return rule with returns-on-all-paths. Create the minimal `crates/doria-rt`, native entry glue, and abort-only panic ABI so later checked panics have a runtime target. AC: recursive fibonacci, nested-loop matrix example, early-return search all run natively; loop-verification cap removed; a minimal explicit panic smoke exits 101 through doria-rt; CI builds and passes on Linux, macOS, and Windows per §8.6.
- **Stage 13 — Full integer family + operators.** All fixed-width types in the compiler; `/`, `%`, shifts, bitwise across widths; contextual integer literals; overflow/div-zero panics with runtime messages through the Stage 12 doria-rt panic machinery. AC: differential tests over an arithmetic torture fixture; panic exit status 101 with message.
- **Stage 14 — Floats + bool runtime.** `float32/64` arithmetic/comparison codegen, bool as runtime value (not just conditions), `Float`/`Int` conversion companions. Records 0040–0042 collectively form the **numerical-semantics gate**: NaN/infinity behavior, determinism policy, and conversion rules must be stated in those records — not inherited by accident from PHP, Rust, or whichever backend lands first — before native numeric behavior is treated as stable. AC: numeric integration examples match interpreter bit-for-bit for f64 ops; NaN/inf comparison and conversion fixtures pass identically on all backends.
- **Stage 15 — LLVM release backend.** `--release` through LLVM over the same MIR; differential suite triples. AC: all examples identical across interpreter/Cranelift/LLVM; release binaries pass the suite.

### Phase B — Runtime strings and I/O (Stages 16–18)
- **Stage 16 — doria-rt strings + display conversion.** Heap `string` (immutable, refcounted), runtime concatenation, writable string locals, string equality/ordering, and the canonical display conversion of §4.6 for primitives across interpolation, the amended `.` (display-convertible operands, at-least-one-string guard), and `echo`: int/uint decimal, float shortest-round-trip, `bool` → `"true"`/`"false"`. AC: string-building loop example; concat of function results; `"I am " . 183` and `{$flag}` bool fixtures with exact expected output; two-int concatenation rejection snapshot; leak checker (Miri/valgrind CI job) clean.
- **Stage 17 — std::io v0 + formatted I/O.** stdin/stdout/stderr streams, file read/write over `string` lines; the §9 formatted-I/O minimal set: `read_line(): ?string` (newline-stripped, `null` at EOF), the `read_file`/`write_file`/`write_stderr` family, and compiler-known `sprintf`/`printf` with compile-time-checked literal format strings over the v1.0 specifier subset; `print` rejected with a use-`echo` diagnostic. **Console-enabling constraints (so Stage 46's `Console` needs no rework):** doria-rt's I/O layering separates the raw device layer (handle/fd read-write, explicit flush) from line discipline (`read_line`'s buffering lives above the device layer, never baked into it, so raw-mode byte-level input later reuses the same primitives); a TTY/interactivity query primitive ships for all three standard streams; and the Windows implementation writes correct UTF-8 to the console from day one (console output code page / wide-write handling decided here, not deferred — mojibake discovered at Stage 46 would mean re-plumbing Stage 17). `Bytes` is deferred to Stage 23: a mutable buffer is a move type, so it belongs after the ownership stages. AC: cat-clone and line-count example programs; echoing-`read_line` loop example; PHP-spelling fixit snapshot (`readline` → `read_line`); sprintf fixture matrix (`%05d`, `%.2f`, `%x`, width/align) with exact expected output; format-mismatch diagnostic snapshots (`%d` with string arg, wrong arity, non-literal format); TTY-detection primitive unit-tested (piped vs terminal); non-ASCII output renders correctly on the Windows CI runner.
- **Stage 18 — Interpolation of expressions + Displayable.** Full `{expr}` interpolation; `Displayable` interface (compiler-known) as Doria's `__toString` replacement, wired into all three display contexts; parser fuzzing job lands. AC: `echo "sum: {a() + b()}"`; interpolating a non-Displayable class is a compile error with suggestion.

### Phase C — Classes go native (Stages 19–22)
- **Stage 19 — Ownership, moves, destruction.** Native class layout, `new`, property init expressions, promoted params; classes become the first Doria move types: move analysis, use-after-move diagnostics in plain vocabulary, drop elaboration placing deterministic `__destruct` at end of owning scope, `take` parameters. Explicit user clone syntax and the `Cloneable` interface are deferred until method/interface support exists. **Layout constraint (records 0038/0074):** the object-representation machinery must not assume every aggregate-with-methods is a heap-allocated move type — compiler-known inline Copy aggregates exist (payload enums now, `std::math` value types at Stage 47) and share layout machinery with, but not the heap/move classification of, classes. **Destruction order and native class representation are settled in records 0081/0082 (see §12); combination rules for `take`/`writable`, promoted move-type transfer, property ordering, move legality, the temporary native-eligibility soundness gate, and the allocation panic are settled in record 0083.** The RAII resource-guard example is a simple class that acquires an abstract owned resource in its constructor and releases it observably in `__destruct`, shaped as the future `Console::rawMode` guard so it dry-runs the flagship RAII case and is differential-testable across backends; it defines no `File`, stream, or FFI handle. The use-after-move diagnostic does not suggest `->clone()` (clone does not exist until method/interface support); it names the give-away point and that the value can't be used after. AC: destructor-order example; use-after-move diagnostic snapshots; RAII resource-guard example; leak CI clean.
- **Stage 20 — Methods, statics, internal.** Instance/static method codegen, `internal` enforcement in native path, class constants + const evaluation tier. This stage may add compiler-recognized `->clone()` lowering for explicit built-in duplication where needed, but the public `Cloneable` interface waits for Stage 35 conformance. AC: the SPEC §6 `Parser` class runs natively.
- **Stage 21 — The borrow checker.** Non-lexical borrow checking on MIR: readonly/writable parameters and `$this` become enforced borrows, place-expression borrows, borrow-returning accessors under the §3.2 elision rule, one-writer-XOR-many-readers conflict diagnostics in owns/gives vocabulary; constructor definite-initialization on all paths (finishing SPEC §5 future-work note) lands on the same dataflow framework. `Shared<T>`/`Weak<T>`/`SharedMut<T>` ship here as the pressure valve; `SharedMut<T>` explicitly emits dynamic access checks. AC: legal/illegal borrow and ctor fixture matrix; borrow-conflict diagnostic snapshots; getter-returning-borrow example; zero runtime checks emitted for ordinary borrow checking outside explicit `SharedMut<T>` use.
- **Stage 22 — Nullable + narrowing + `is` + `mixed` statics.** D5 complete, plus D21 static semantics: `mixed` accepts every type and rejects every operation until narrowed; `is`/`match` narrowing over `mixed`; `null` rejected in type position with a `?T` suggestion; `void` restricted to return position; `object` absent from the grammar and reserved-word guardrails; `resource` rejected as reserved. The boxed runtime representation lands in Stage 23. AC: null-safe chaining example; narrowing snapshot diagnostics; mixed operation-rejection and narrowing fixture matrix; null/void/object/resource position-diagnostic snapshots.

### Phase D — Collections and generics (Stages 23–26)
- **Stage 23 — Runtime collections, typed arrays + Bytes.** Owned `T[]` typed-array, `List/Dictionary/Set`, and `Bytes` intrinsics in doria-rt as move types; contextually typed collection literals per §4.9 (sequence literals default to `List<T>`, keyed literals to `Dictionary<K, V>`); indexing (`$list[0]` borrows the element; panic OOB), insertion moves values in, `foreach` element borrows (`as $item` readonly, `as writable $item` exclusive); `array` in type position rejected with a §4.9 diagnostic. The boxed `mixed` runtime representation (`dr_mixed` tagged box) ships here alongside the other move-type intrinsics, and the binary file tier arrives with `Bytes`: `read_file_bytes(): Bytes` / `write_file_bytes(...)` per §9. AC: move/borrow fixture matrix over collections and typed arrays; `int[]`/`int[][]` fixed-length fixtures; literal context-typing fixtures (`int[]` vs `List<int>` from the same literal); in-place mutation loop example; use-after-move-into-list diagnostic snapshot; mixed box round-trip fixture (int/string/class in, narrowed out); binary file round-trip fixture (`write_file_bytes` then `read_file_bytes` over non-UTF-8 data, byte-identical).
- **Stage 24 — Generic functions.** D9 for free functions/methods, monomorphization in MIR. AC: `first<T>` works across int/string/class lists.
- **Stage 25 — Generic classes + compiler-internal iteration machinery.** The §4.5 `History<T>` example and generic collection/runtime machinery; `foreach` over built-in collections lowers through compiler-internal iteration support. Public generic interfaces, traits, and user-defined `Iterable<T>`/`Iterator<T>` conformance are deferred to Stage 35. AC: generic classes run natively; built-in collections are consumed by `foreach` without user-visible interface conformance.
- **Stage 26 — Collection API surface.** Decision 0051 methods incl. `map`/`filter` once Stage 30 closures exist (split: non-closure API here, closure API revisited in Stage 30). Before Stage 31 include/multi-file support, required stdlib fragments are compiler-bundled or prelude-style rather than source-included. AC: non-closure collection APIs compile and run from the compiler-provided stdlib surface.

### Phase E — Enums, match, errors (Stages 27–29)
- **Stage 27 — Enums + payload cases.** D6, inline tagged layout, Copy/move classification per payloads. AC: `Shape` example native.
- **Stage 28 — match.** D7, exhaustiveness, payload destructuring, narrowing integration; guards fast-follow within the stage. AC: exhaustiveness diagnostics snapshots; `match (true)` chains.
- **Stage 29 — Checked errors end-to-end.** D8: `throws` ABI, `try/catch/finally`, `Error` interface with property requirement, `main throws`. This stage also executes the record-0075 I/O failure migration: the §9 file/input free functions move from panic-on-failure to declared `throws` signatures, and `File`/stream object design is unblocked. AC: SPEC-style `loadUser` example; uncovered-error diagnostics; finally-ordering fixture; error-escaping-`main` fixture asserting stderr `error: <Class>: <message>`, exit status 70, and destructor execution on the propagation path.

### Phase F — Multi-file, namespaces, Baton (Stages 30–33)
- **Stage 30 — Closures.** D10 + D20: explicitly typed closure and arrow-function parameters (omitted parameter types are compile errors with context-derived suggestions), borrow and `take` captures, function types in type position; collection closure APIs unlock. AC: sort-with-comparator example with typed callback parameters; borrow-bound closure escape rejected with a `take` suggestion fixture.
- **Stage 31 — Namespaces/use/include/declare.** Decision 0028 implemented; multi-file compilation; first declare keys. AC: multi-file example project builds; duplicate-symbol diagnostics; Doria stdlib fragments that were compiler-provided in Stage 26 now compile through `include`.
- **Stage 32 — Attributes.** `#[...]` parsing, type-checked against attribute classes, const-evaluation-tier arguments (resolving SPEC §11's evaluation-policy question: compile-time const evaluation only, no side effects); reflection deferred — attributes are compiler/tooling metadata in v1.0. Standing separation, preserved by this stage and after: **attribute metadata, constant evaluation, and any future general compile-time execution are three distinct concepts** — attributes never grow into arbitrary compile-time code execution, and compiler transformations reject unsupported side effects rather than silently changing behavior. AC: `#[Test]`, `#[PHPExport]` representable — parsed and type-checked as attributes only; `#[PHPExport]` bridge semantics activate in Stage 41, and export is metadata, never visibility.
- **Stage 33 — Baton MVP.** §11 scope: new/build/run/test/check, path deps, `#[Test]` runner. AC: `baton new game && baton test` green out of the box.

### Phase G — OOP completion (Stages 34–36)
- **Stage 34 — Inheritance.** D17: `open`/`override`, vtables, parent construction rules, devirtualization in LLVM profile. AC: `Post extends Model` native; missing-override diagnostics.
- **Stage 35 — Interfaces + traits.** Conformance checking, interface-typed values (fat pointer or vtable-embedded — decide in 0053/0054), trait flattening + `insteadof`/`as`. `Cloneable` becomes the public explicit-duplication contract here, and user-defined `Iterable<T>`/`Iterator<T>` conformance plugs into `foreach`. AC: SPEC §8 examples native; a user-defined iterable is consumed by `foreach`; a Cloneable class can be cloned through the interface contract.
- **Stage 36 — Property hooks + when.** §6.4 hooks; `when` grammar per 0064. AC: `Temperature` example; when-chain example.

### Phase H — Concurrency (Stages 37–39)
- **Stage 37 — Concurrency design record 0063.** Paper stage: full async model (executor in doria-rt, task groups, cancellation, `Shareable` rules). The design tests must include data-pipeline and long-running compute scenarios — bounded parallelism, backpressure, prefetch, worker pools, failure propagation, deterministic cleanup — alongside the PHP thread-affinity invariant (§10.3). Designer sign-off required — this is the one deliberate design gate in the plan.
- **Stage 38 — async/await codegen.** State-machine lowering in MIR; single-threaded executor first; `async main` entry bootstrap per §5 (executor started only when `main` is async — sync programs pay zero async cost). AC: async file-read example; interpreter parity; async-`main` escaping-error example exits 70 with destructors run.
- **Stage 39 — Structured task groups + Shareable checking.** Multi-threaded executor; spawn-boundary checks via auto-derived `Sendable`/`Shareable` — with ownership in place this is Rust Send/Sync-grade freedom from data races. AC: parallel map example; data-race fixture rejected at compile time.

### Phase I — Systems and PHP bridge (Stages 40–42)
- **Stage 40 — unsafe/FFI.** D12: `unsafe`, `Ptr<T>`, `extern "C"`, linking foreign libs via Baton manifest. Record 0061 must evaluate **zero-copy numerical exchange** as a named design case: pointer+length(+stride) views over `T[]`/`Bytes`, ownership transfer of externally allocated buffers, and callbacks — FFI must not be designed to copy all buffers by default. AC: bind and call a C function (e.g., zlib) from Doria; a zero-copy `Bytes`-view round-trip fixture.
- **Stage 41 — php-lib bridge.** D13c end-to-end behind public `baton build --php-lib`: export analysis, C-ABI shim gen, PHP FFI stub gen, handle lifetime tests against real PHP 8 in CI. doriac provides compiler emission primitives only as needed by Baton. The bridge ABI and ownership/lifetime design must be transport- and direction-neutral per §10.3 — a later Zend adapter and the deferred embedded host (§10.4) consume the same contract. AC: the `ImageResizer` scenario runs from a PHP script in CI through Baton; the ABI header review confirms no Zend structures and no Doria layout leakage.
- **Stage 42 — migrate php v0.** §10.2 conservative converter. AC: converts a small idiomatic PHP 8 fixture app; dynamic features produce diagnostics not silent guesses.

### Phase J — Engine enablers and 1.0 hardening (Stages 43+)
- **Stage 43 — Engine profile.** declare-based overflow relaxation for audited modules; arena allocator hooks in doria-rt; benchmark suite (criterion-style) vs C/Rust baselines for drop-glue, `Shared<T>`, and collection hot paths.
- **Stage 44 — SIMD direction (0065)** + `std::net`/`http` maturation for the PHP sidecar pattern.
- **Stage 45 — Self-hosting start.** Port the lexer to Doria as the first self-hosted component (per docs/self-hosting.md), compiled by `doriac`, differentially tested against the Rust lexer.
- **Stage 46 — std::term v0 + `Console` (record 0072).** The §9 portable terminal layer surfaced through the `Console` class (TermUtil-informed API inventory settled in the record): raw-mode enter/leave (restored via the guard's `__destruct` on every structured exit including error escaping `main` — a wedged terminal is the classic TUI failure; per record 0081 an abort-only *panic* in raw mode does not run cleanup, so raw-mode restoration on panic, if wanted, requires the future panic-hook addition, not the guard), key/resize event decoding to enums, cursor/style/clear/size, both platform backends landing together per §8.6. May proceed in parallel with Stage 45. AC: an interactive demo (input echo + moving glyph + resize handling) runs from the same source on Linux, macOS, and Windows; Unix CI drives it under a pty harness, Windows CI under a ConPTY harness; zero escape sequences appear in the demo's source.
- **Stage 47 — std::math geometry v0 (record 0074).** `Vector2/3/4`, `Quaternion`, `Euler`, `Matrix3x3/4x4` as built-in inline Copy value types with compiler-known operators, `float32` variants, charter-named API surface; layout coordinated with record 0065's SIMD direction. AC: transform-chain example (translate–rotate–scale via `Matrix4x4`); quaternion slerp fixture matching reference values; operator fixtures differential across all backends; no heap allocation in a vector-arithmetic hot-loop benchmark.
- **1.0 gate** (ships as the first unsuffixed `yyyy.mm.n` release per §11)**:** spec freeze pass over SPEC.md, diagnostics audit, doria-rt ABI review, differential + fuzz suites green, the three flagship demos build: a portable TUI game (engine seed — the same binary source running natively on Windows, macOS, and Linux via `std::term`, zero ANSI in user code), a UI component demo, and a PHP app calling a Doria php-lib.

### Dependency notes for the implementing agent
- Nothing in Phases B–J may begin before Stage 11 lands (everything depends on MIR + oracle).
- WASM backend remains recognized-but-unscheduled; do not start it before 1.0.
- Game engine and UI framework are **separate repositories** consuming Doria; this plan only builds their enablers. Do not scaffold them inside the compiler repo.

---

## 14. What is explicitly out of scope for v1.0

Tracing GC (never), pervasive ARC as the default model (never), Rust-spelled borrow sigils and lifetime annotations (never — inference and elision only), visibility modifiers beyond the default-accessible + `internal` two-state model — `protected`, `private`, and `public` keywords (never; this is identity, not scope deferral), a broad PHP-style `array` type (never — sequences are `T[]` typed arrays and named collections per §4.9), an `object` type (cut; reintroduce only with concrete PHP-bridge evidence), `resource` as a core type (reserved to the Phase I bridge), `Result<T,E>` surface model (per 0035), unions beyond `?T`, `goto`, textual macros (per 0028), runtime reflection, package registry server, catchable panics, user-defined operator overloading, default interface methods, variadic generics, a TUI widget/framework layer in the stdlib (userland territory — the ported engines are the widget layer; `std::term` stays primitive), raw ANSI escape sequences as any public stdlib API (never — `std::term` is capability-based), a `print` construct (never — `echo` is the one output spelling), `sscanf` (deferred — see §9), dynamic (non-literal) format strings for `sprintf`/`printf`, wholesale import of PHP's string-function catalogue (post-1.0, case by case under the §9.1 charter), the embedded-PHP host implementation (product d — deferred, architecture preserved per §10.4), Zend-extension code generation (intended production transport, unscheduled until after Stage 41), PHP object proxies and the callback/reentrancy runtime, a Composer prebuilt-binary packaging matrix, framework adapter packages (Laravel/Symfony/AssegaiPHP — separate repositories), a Laravel 4 / legacy assessment profile for `doriac migrate` (modern typed PHP remains the first migration target), the entire AI/numerical stack (tensor or `NDArray` standard types, automatic differentiation, GPU/accelerator backends, graph capture, mixed-precision or distributed training, notebooks and REPL kernels, experiment tracking, model/dataset hubs, Python-interop bridges, MLIR adoption, shape-dependent typing, and any AI-specific keyword — all Appendix A, none v1.0), and bidirectional PHP compatibility guarantees.

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
