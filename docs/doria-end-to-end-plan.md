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
| D3 | Copy vs move | Copy types: primitives, `bool`, `string`, ranges, enums with Copy payloads. Move types: classes, `List`/`Dictionary`/`Set`, `Bytes`, closures. Explicit duplication uses the future `->clone()` / `Cloneable` surface once method and interface support exist; before that, move-type duplication is deliberately unavailable. No user-defined `struct` in v1.0 — classes are the owned record type (revisit inline layout post-1.0 if engine profiling demands it) |
| D4 | Integer overflow | Arithmetic overflow panics in both dev and release profiles. Explicit `Int::wrapping_add(...)`, `Int::saturating_add(...)`, `Int::checked_add(...)` for other behavior. A `declare` key may later relax this per-module for engine hot paths |
| D5 | Nullability | `?Type` optional types (PHP spelling), `??` null coalescing, `?->` null-safe access. `null` is not assignable to non-`?` types. No implicit truthiness |
| D6 | Enums | PHP 8.1-shaped `enum` declarations extended with payload cases (tagged unions): `case Some(int $value);`. This is Doria's sum type |
| D7 | Pattern matching | `match` is expression-position, exhaustiveness-checked over enums/bools/finite domains, PHP 8 `match` spelling extended with payload destructuring. `when` is the value-returning conditional chain per decision 0009 |
| D8 | Errors | Checked `throw`/`throws` with PHP-shaped `try`/`catch`/`finally`. Errors are class instances implementing the built-in `interface Error`. `Result<T, E>` stays out of the surface model per decision 0035 |
| D9 | Generics | Monomorphized generics for functions, classes, interfaces, traits. Constraint spelling: `<T implements Comparable>`. No runtime generic reflection in v1.0 |
| D10 | Closures | PHP-shaped anonymous `function (...) with (...) { }` plus auto-capturing arrow functions `fn(int $x) => ...`. Closure and arrow-function parameters must be explicitly typed, just like free-function, method, constructor, and property-hook setter parameters; Doria never infers omitted parameter types. The capture clause is spelled `with`, not PHP's `use` — in Doria `use` belongs exclusively to namespace imports. Captures are borrows by default (`with ($x)` readonly, `with (writable $x)` exclusive) or moves via `with (take $x)`. A borrow-capturing closure is itself borrow-bound, so the borrow checker rejects escapes automatically and suggests `take` |
| D11 | Concurrency | Structured concurrency with `async function` / `await` / task groups; data-race freedom falls out of the ownership model via auto-derived `Sendable` / `Shareable` marker interfaces (Rust's Send/Sync payoff) checked at spawn boundaries. Detailed design gated behind its own decision record in Phase H |
| D12 | Unsafe & FFI | `unsafe { }` blocks gate raw pointers (`Ptr<T>`, `MutPtr<T>`), foreign calls, and manual memory. `extern` declarations bind C ABI symbols. Everything outside `unsafe` keeps full safety guarantees |
| D13 | PHP bridge (the strategic pillar) | Three interop products: (a) existing Doria→PHP compat backend, (b) `doriac migrate php`, and **(c) `baton build --php-lib` workflow: Baton orchestrates doriac native compilation, C-ABI shared-library emission, and generated PHP FFI stub classes, so PHP applications call native Doria directly.** doriac remains the compiler and exposes only narrow emission primitives needed by Baton. (c) is the powerful backends for PHP product and gets its own phase |
| D14 | Division/modulo | `/` on `int` is truncating integer division; `%` is remainder with sign of dividend (C/PHP `intdiv`-consistent). Division/modulo by zero panics. `float` division follows IEEE 754 |
| D15 | Numeric widening | No implicit conversions anywhere, including int→float. Explicit `Int::to_float($x)`, `Float::to_int($x)` (truncating, panics on NaN/out-of-range), and fixed-width conversions via `Int32::from($x)` (panics on overflow) / `Int32::try_from($x)` (nullable) |
| D16 | String encoding | `string` is immutable UTF-8. Byte-level work uses `Bytes` (a mutable move-type buffer). Indexing a `string` by integer is not allowed; iteration yields grapheme clusters via `$s->chars` is deferred, `$s->bytes` ships first |
| D17 | Inheritance model | Single class inheritance, multiple interface implementation, trait composition via `uses` with explicit conflict resolution (`insteadof`/`as` PHP spelling accepted). Methods are non-virtual by default; `open function` opts into overriding; `override function` required at override sites |
| D18 | Standard entry runtime | Every native binary links `doria-rt` (Rust-implemented runtime library): allocator, drop glue, `Shared<T>` refcount machinery, string/collection intrinsics, panic machinery, stdout/stderr. `doria-rt` is an internal ABI, not public, until v1.0 |
| D19 | Naming charter & the built-in/userland boundary | The **entire built-in surface** (language functions, stdlib functions/methods/properties — e.g. `get_time`, `str_starts_with`, `$s->is_empty`) is `snake_case` with uniform, fully-worded names: `str_case_compare`, never `strcasecmp`. **Userland functions and methods are camelCase by convention**, so casing itself marks the boundary: snake_case reads as "Doria built-in", camelCase as "this codebase". Docs, examples, `baton new` templates, and lints all model and enforce the boundary. Types `PascalCase`, constants `SCREAMING_SNAKE_CASE`. §9.1 is checkable API law |
| D20 | Explicit typing discipline | **Parameter types are never inferred — anywhere.** Free functions, methods, constructors (including promoted parameters), closures, arrow functions, callbacks, and property-hook setters all require written parameter types: `fn($x) => ...` is a compile error, `fn(int $x) => ...` is the language. Named functions and methods must declare return types; only an arrow function's return type may be inferred from its body. `let` locals may infer from their initializer. Nothing ever silently defaults to `mixed`. Full rules in §4.7 |
| D21 | Dynamic boundary types | `mixed` is Doria's **only** dynamic type and it is **unknown-flavored, never any-flavored**: every value may flow in (implicit boxing), and nothing may be done with it until narrowed via `is` / `match`. `mixed` is a boxed, runtime-tagged **move type** — always, even when holding a Copy value. `object` does not exist. `null` is a literal and the `?T` machinery, never a standalone type-position name. `resource` is reserved for the Phase I PHP bridge, not a core v1.0 type. `void` is return-position only. Full rules in §4.8 |

Everything below elaborates these decisions into implementable specifications.

---

## 2. Vision, positioning, and end products

Doria is a statically checked, natively compiled systems language with PHP-shaped syntax and Rust-grade safety defaults, minus Rust's lifetime/borrow surface language. The strategic products it must eventually support, in priority order:

1. **A native systems language** producing standalone executables (already the accepted direction).
2. **The PHP power-backend story**: when a PHP application hits performance or capability limits, teams write the hot module in Doria and call it from PHP with near-zero friction, because the syntax is already familiar and the bridge is first-class (D13c). This is Doria's unique adoption wedge — no other native language offers PHP developers syntax continuity plus a generated, type-checked FFI bridge.
3. **A game engine written in Doria**, which drives requirements for: deterministic destruction (ownership/RAII — no GC pauses, no refcount traffic on hot paths), fixed-width numerics, floats and SIMD, unsafe/FFI for graphics/audio/input APIs, allocator control, and predictable value-type collections.
4. **A UI framework** integrating with PHP web backends, which drives requirements for: attributes-as-metadata, property hooks, closures, enums, pattern matching, and async.

The plan sequences language work so that requirement sets 2–4 unlock in that order.

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
- **Move types** (classes, `List<T>`, `Dictionary<K, V>`, `Set<T>`, `Bytes`, closures, enums with move payloads): assignment and by-value passing transfer ownership. Using a moved-from binding is a compile error.
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

- `unsafe { }` is the only context where `Ptr<T>` / `MutPtr<T>` may be dereferenced, `extern` functions called, and ownership deliberately sidestepped (`Shared::into_raw` / `Shared::from_raw` style intrinsics for FFI handle passing).
- `extern "C"` blocks declare foreign symbols; parameter/return types restricted to FFI-safe types (fixed-width numerics, `Ptr<T>`).
- An `unsafe function` spelling marks a whole function as requiring an unsafe context to call.
- `declare` keys will later govern per-module unsafe policy (deny/allow), per decision 0028's directive direction.
- The safety contract is Rust's: `unsafe` code must uphold the invariants the borrow checker assumes; everything outside `unsafe` keeps full guarantees.

### 3.6 Panics

A panic is a fatal runtime error, distinct from checked `throw`/`throws` per decision 0035: arithmetic overflow, division by zero, out-of-bounds indexing, `SharedMut` access violation, failed `Float::to_int`, explicit `panic("message")`. Default behavior: print message + Doria stack trace to stderr and exit with status 101. **v1.0 panic policy is abort-only (no unwinding, no catching panics).** This keeps codegen simple and honest; checked errors are the recoverable path.

---

## 4. Type system completion (D4–D9, D14–D16)

### 4.1 Numerics

- Full fixed-width family per decision 0016 becomes real compiler types: `int8/16/32/64`, `uint8/16/32/64`, `float32/64`; `int` = `int64`, `float` = `float64`.
- Literals: `42` is `int` unless the expected type in context is another integer type and the literal fits (contextual typing, checked at compile time; `int8 $x = 200;` is a compile error). `4.2` is `float` with the same contextual rule for `float32`. Suffixed literal spellings are **not** added; contextual typing plus `Int32::from(...)` covers the need.
- Operators complete: `/`, `%` (D14), bit shifts `<<` `>>` (arithmetic right shift on signed; shifting by ≥ bit-width panics), bitwise `& | ^ ~` on all integer types.
- No implicit widening (D15). Mixed-type arithmetic (`int + int32`) is a compile error; convert explicitly.

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

class Stack<T>
{
    internal writable List<T> $items = [];

    writable function push(T $item): void { /* ... */ }
    writable function pop(): ?T { /* ... */ }
}

function max<T implements Comparable<T>>(T $a, T $b): T
{
    return match (true) {
        $a->compare_to($b) >= 0 => $a,
        default => $b,
    };
}
```

- Monomorphization at MIR level: each concrete instantiation generates specialized code (Rust model — zero-cost, no boxing). Compile-time cost is accepted; the dev backend (Cranelift) keeps iteration fast.
- Constraint spelling `T implements Interface` keeps Doria's own vocabulary; multiple constraints with `+`? **No** — spelling is `T implements A, B` inside the angle brackets, comma-separated, matching `implements` lists.
- Generic type inference at call sites from argument types; explicit turbofish-style spelling is **not** adopted — where inference fails, bind through a typed declaration.
- Collections `List<T>`, `Dictionary<K, V>`, `Set<T>` become real generic types in the compiler (they already have checked arity) backed by runtime intrinsics, then by stdlib generic implementations as self-hosting matures.

### 4.6 Strings and Bytes (D16)

- `string`: immutable, UTF-8, and a Copy type (internally a refcounted immutable buffer, so copies are pointer-cheap). One string type deliberately avoids Rust's `String`/`&str` split — the single biggest spelling complaint this plan removes. `$s->length` is byte length; `$s->is_empty`, `$s->bytes` accessor returning `Bytes` view (copy in v1.0).
- `Bytes`: mutable move-type byte buffer for binary work, file I/O, network buffers, engine assets; in-place mutation goes through `writable` borrows like everything else.
- Concatenation `.` stays string-only (already accepted). Interpolation grows to full expressions in braces `{...}` in its own stage; display conversion is governed by a built-in `interface Displayable { function to_string(): string; }` — interpolating a non-Displayable class remains a compile error, resolving SPEC §7's open display-conversion question.
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

All symbols `dr_`-prefixed, internal ABI, versioned in lockstep with the compiler.

### 8.4 Diagnostics

Adopt error codes now (`D0001`-style) before the count explodes; every diagnostic carries code, span(s), message, and machine-applicable suggestion where possible; `doriac check --json` for tooling; LSP reuses the same diagnostics verbatim (already the architecture).

### 8.5 Testing strategy (all phases)

- Unit tests per compiler pass; integration tests per stage in `crates/doriac/tests` (current pattern).
- Differential suite: interpreter vs Cranelift vs LLVM on every executable example.
- UI-style diagnostic snapshot tests (expected diagnostics per fixture file) so error messages are versioned.
- The PHP backend keeps its own snapshot tests but is never the proof of semantics (unchanged policy).
- Fuzzing the lexer/parser with `cargo-fuzz` starts in Phase B (cheap, catches panics early).

---

## 9. Standard library plan

Two layers, both written in Doria as early as possible (self-hosting on-ramp):

- **core** (no I/O, always available): `Int`/`Int8`.../`Float`/`Bool`/`String` companion APIs (`Int::parse`, `Int::to_float`, `Int::wrapping_add`, ...), `Option`-free nullable helpers, `Cloneable` (Stage 35 interface contract), `Shared<T>`/`Weak<T>`/`SharedMut<T>`, `Comparable<T>`, `Equatable<T>`, `Hashable`, `Displayable`, `Error`, `Iterable<T>`/`Iterator<T>` (Stage 35 user conformance; collections use compiler-internal iteration earlier), range types, `math` basics, and a PHP-familiar free-function layer (`get_time`, `str_starts_with`, `str_case_compare`, ...) that wraps the method/companion surface — regularized names only, never PHP's fused spellings.
- **std** (hosted): `io` (files, stdin/out streams), `fs`, `env`, `process`, `time`, `random`, `json` (drives enum/match/mixed ergonomics and the PHP bridge), `net` (TCP first), later `http`.

`foreach (collection as ...)` uses compiler-internal iteration machinery in Phase D for built-in collections. The public `Iterable<T>` / `Iterator<T>` protocol that makes user types iterable lands with interface conformance in Stage 35.

### 9.1 Naming charter and the built-in/userland boundary (D19)

PHP's standard library is the cautionary tale this charter exists to prevent: `strlen` vs `str_replace` vs `nl2br`, camelCase methods beside snake_case functions, and needle/haystack argument order that flips between functions. Doria's built-in surface follows one law, enforced by API review and a `doriac` lint over the stdlib:

- **Casing**: every built-in function, method, property, and parameter is `snake_case` — free functions (`get_time`, `str_starts_with`) and companion/type APIs (`Int::wrapping_add`, `$s->is_empty`) alike, with `__construct`/`__destruct` kept as PHP-inherited keywords-in-disguise. Classes, interfaces, traits, enums, and enum cases are `PascalCase`. Constants are `SCREAMING_SNAKE_CASE`. Type parameters are single capitals (`T`, `K`, `V`).
- **No contractions**: `length` not `len`, `to_string` not `strval`, and — the emblematic case — **`str_case_compare`, not `strcasecmp`**. PHP's `str_` free-function family is kept as a familiar, whitelisted domain prefix, but everything after the prefix is fully worded and underscore-separated; `strlen`-style fusions never appear. Other whitelisted abbreviations are only those more recognizable than their expansions (`str`, `id`, `min`, `max`, `io`, `http`, `json`, `utf8`).
- **Symmetric pairs**: conversions are always `to_x` / `from_x` / `try_from_x`; lifecycle verbs pair predictably (`open`/`close`, `push`/`pop`, `add`/`remove`, `into_raw`/`from_raw`).
- **Predicates** read as questions: `is_`, `has_`, `can_` — and per SPEC §6's nouns-are-properties rule, argument-free ones are properties (`$s->is_empty`), never `get`-prefixed methods.
- **Uniform argument order**: the subject always comes first (it is `$this` on methods); options and callbacks come last. No needle/haystack roulette.
- **One name per concept** across modules: it is `count` everywhere (never `size` or `length` for collections), `contains` everywhere.

**The built-in/userland boundary.** snake_case is Doria's built-in signature; **userland functions and methods are camelCase by convention**. The payoff is that casing alone tells a reader — and tooling — whether a call is Doria's or the codebase's own: `str_case_compare($a, $b)` is the language, `normalizeTitle($post)` is the application. Enforcement of the boundary:

- All documentation, SPEC examples, `baton new` templates, LSP snippets, and generated code (`#[Derive(...)]` members) write userland declarations in camelCase; this plan's own examples model it (`loadUser`, `findById` are userland; `compare_to`, `to_string` appear in userland classes only when implementing a built-in interface, whose member names always keep the interface's spelling).
- A default-on lint hints when userland declares a snake_case free function or method, framed as a boundary warning ("this reads as a Doria built-in"), silenceable per-declaration and per-module.
- `function_exists("name")` is a compile-time predicate usable in top-level `if` to conditionally declare a function. This is the sanctioned collision/polyfill mechanism: guarded declarations may adopt the built-in's snake_case name because they deliberately stand in for one (e.g. back-filling a newer stdlib function on an older Doria); outside such a guard, userland free functions stay camelCase. `function_exists` is const-evaluated — there is no runtime symbol table.
- The generated PHP FFI stubs mirror the exported Doria class's own casing, so a `#[PhpExport]` class written in charter-compliant userland camelCase lands in PHP looking like idiomatic PSR code — a free win for the bridge.

Every stdlib decision record cites this charter, and `baton fmt` plus the stdlib lint enforce it mechanically.

Stdlib API style follows SPEC §6's nouns-are-properties rule and the collection method surface gets its own decision record (List: `add`, `insert_at`, `remove_at`, `contains`, `count` property, `is_empty` property, `map`/`filter`/`reduce` after closures land; Dictionary: `get` returning `?V`, `set`, `remove`, `has`, `keys`, `values`; Set: `add`, `remove`, `has`, `union`, `intersect`).

---

## 10. PHP interop: the three products (D13)

### 10.1 Doria → PHP compat backend (exists)
Keeps growing opportunistically for migration/debugging; never gates a language feature. Features PHP cannot express lower where practical or emit unsupported-feature diagnostics (unchanged policy).

### 10.2 PHP → Doria migration (`doriac migrate php`)
Phase I product, per SPEC §12: conservative output, diagnostics for dynamic PHP (variable variables, `eval`, magic methods, loose comparisons become explicit conversions or `mixed` + TODO diagnostics). Architecturally separate crate `crates/doria-migrate` with its own PHP parser (use `mago`/`php-parser-rs` class of dependency; do not touch the Doria parser).

### 10.3 The strategic product: `baton build --php-lib`

Baton builds a Doria library into something a running PHP application calls natively, using doriac as the compiler underneath:

```doria
namespace App\Native;

#[PhpExport]
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

- Exported surface restricted to a bridgeable type set: numerics, `bool`, `string`, `Bytes`, `?T` of those, `List`/`Dictionary` of bridgeable types, and `#[PhpExport]` classes (marshaled as opaque handles rooted as `Shared<T>` on the Doria side; the generated PHP stub holds the handle and releases it in its `__destruct`).
- `throws` errors surface as generated PHP exception classes. An error escaping an exported function must surface as that exception and never terminate the host PHP process — the bridge is an FFI boundary, not a process boundary. Panics are the documented exception: under v1.0's abort-only policy a Doria panic aborts the hosting PHP worker (exactly as a crashing native extension would), which is why exported surfaces should prefer `throws` APIs over panicking ones.
- Transport: C ABI shim generated by `doriac` under Baton orchestration + PHP ≥ 8.0 `FFI` stubs first (zero build tooling required on the PHP side); a Zend-extension emission mode (`--php-ext`) is a later optimization stage for call-overhead-sensitive users.
- Threading: v1 bridge is single-threaded per PHP request (matches PHP's model); `Shareable` interactions revisited with Phase H.
- This product plus `std::json`/`net` also covers the sidecar pattern (Doria service, PHP client), but the in-process bridge is the headline.

---

## 11. Baton and developer experience

Baton lands mid-plan (Phase F), once multi-file compilation exists to orchestrate:

- `baton new <name>` (binary/lib/php-lib templates), `baton build [--release]`, `baton run`, `baton test`, `baton check`.
- Manifest `Baton.toml`: package name, version, edition placeholder, dependency table (path dependencies first; registry protocol deferred to post-1.0 — do not build a registry server in this plan).
- `baton test` defines the Doria test convention: `tests/*.doria` files whose functions marked `#[Test]` run and report (first real consumer of attributes).
- Baton drives `doriac`; it never owns semantics. LSP/editors gain workspace awareness from `Baton.toml`.

---

## 12. Decision records to author (numbering continues from 0037)

0043 MIR + interpreter oracle (§8.1–8.2) is already authored and accepted in `docs/decisions/0043-stage-11-mir-and-interpreter-oracle.md`.

0038 ownership and move semantics (D1, D3) · 0039 borrowing rules, elision, and the borrow checker (D2) · 0040 panics & overflow policy (D4, §3.6) · 0041 division/modulo/shifts (D14) · 0042 numeric conversions (D15) · 0044 doria-rt ABI (D18) · 0045 runtime strings/Bytes (D16) · 0046 nullable types & narrowing (D5) · 0047 enums & payload cases (D6) · 0048 match (D7) · 0049 checked errors full semantics incl. errors escaping `main` (D8, §5) · 0050 generics & monomorphization (D9) · 0051 collections runtime & API surface (§9) · 0052 compiler-internal iteration machinery · 0053 inheritance/open/override (D17) · 0054 interfaces, traits, Cloneable, and public Iterable conformance · 0055 property hooks · 0056 statics & const evaluation · 0057 closures (D10) · 0058 namespaces implementation notes (elaborating 0028) · 0059 attributes & compile-time evaluation policy · 0060 Baton manifest & test convention · 0061 unsafe/FFI (D12) · 0062 php-lib bridge (D13c) · 0063 async & Shareable (D11) · 0064 when grammar · 0065 SIMD/engine intrinsics direction · 0066 `Shared<T>` / `Weak<T>` / `SharedMut<T>` · 0067 naming charter, built-in/userland boundary, and `function_exists` (D19) · 0068 explicit typing discipline (D20) · 0069 dynamic boundary types: `mixed`/`object`/`null`/`resource`/`void` (D21).

Each record follows the existing template: context, decision, alternatives considered, consequences, affected components.

---

## 13. Phased roadmap with stages and acceptance criteria

Stages continue the existing numbering. Every stage = decision record(s) + tests + docs + examples, per Section 0. "AC" = acceptance criteria.

### Phase A — Real native foundation (Stages 11–15)
Retire the smoke architecture; make the native path general.

- **Stage 11 — MIR + interpreter oracle.** Introduce MIR; port all Stage ≤10 lowering onto it; delete `NativeSmokeModule`; ship the MIR interpreter as `--target debug`; stand up the differential test harness. AC: every existing native example produces identical output under interpreter and Cranelift; no smoke-module code remains.
- **Stage 12 — General control flow + runtime foundation.** Arbitrary/nested loops, `return` anywhere, unbounded `while`, `break`/`continue` everywhere, recursion and mutual recursion; shared dataflow framework replaces the final-statement-return rule with returns-on-all-paths. Create the minimal `crates/doria-rt`, native entry glue, and abort-only panic ABI so later checked panics have a runtime target. AC: recursive fibonacci, nested-loop matrix example, early-return search all run natively; loop-verification cap removed; a minimal explicit panic smoke exits 101 through doria-rt.
- **Stage 13 — Full integer family + operators.** All fixed-width types in the compiler; `/`, `%`, shifts, bitwise across widths; contextual integer literals; overflow/div-zero panics with runtime messages through the Stage 12 doria-rt panic machinery. AC: differential tests over an arithmetic torture fixture; panic exit status 101 with message.
- **Stage 14 — Floats + bool runtime.** `float32/64` arithmetic/comparison codegen, bool as runtime value (not just conditions), `Float`/`Int` conversion companions. AC: numeric integration examples match interpreter bit-for-bit for f64 ops.
- **Stage 15 — LLVM release backend.** `--release` through LLVM over the same MIR; differential suite triples. AC: all examples identical across interpreter/Cranelift/LLVM; release binaries pass the suite.

### Phase B — Runtime strings and I/O (Stages 16–18)
- **Stage 16 — doria-rt strings.** Heap `string` (immutable, refcounted), runtime concatenation, writable string locals, string equality/ordering, full interpolation of currently-interpolable types at runtime. AC: string-building loop example; concat of function results; leak checker (Miri/valgrind CI job) clean.
- **Stage 17 — std::io v0.** stdin/stdout/stderr streams, file read/write over `string` lines. `Bytes` is deferred to Stage 23: a mutable buffer is a move type, so it belongs after the ownership stages. AC: cat-clone and line-count example programs.
- **Stage 18 — Interpolation of expressions + Displayable.** Full `{expr}` interpolation; `Displayable` interface (compiler-known); parser fuzzing job lands. AC: `echo "sum: {a() + b()}"`; interpolating a non-Displayable class is a compile error with suggestion.

### Phase C — Classes go native (Stages 19–22)
- **Stage 19 — Ownership, moves, destruction.** Native class layout, `new`, property init expressions, promoted params; classes become the first Doria move types: move analysis, use-after-move diagnostics in plain vocabulary, drop elaboration placing deterministic `__destruct` at end of owning scope, `take` parameters. Explicit user clone syntax and the `Cloneable` interface are deferred until method/interface support exists. AC: destructor-order example; use-after-move diagnostic snapshots; RAII file-handle example; leak CI clean.
- **Stage 20 — Methods, statics, internal.** Instance/static method codegen, `internal` enforcement in native path, class constants + const evaluation tier. This stage may add compiler-recognized `->clone()` lowering for explicit built-in duplication where needed, but the public `Cloneable` interface waits for Stage 35 conformance. AC: the SPEC §6 `Parser` class runs natively.
- **Stage 21 — The borrow checker.** Non-lexical borrow checking on MIR: readonly/writable parameters and `$this` become enforced borrows, place-expression borrows, borrow-returning accessors under the §3.2 elision rule, one-writer-XOR-many-readers conflict diagnostics in owns/gives vocabulary; constructor definite-initialization on all paths (finishing SPEC §5 future-work note) lands on the same dataflow framework. `Shared<T>`/`Weak<T>`/`SharedMut<T>` ship here as the pressure valve; `SharedMut<T>` explicitly emits dynamic access checks. AC: legal/illegal borrow and ctor fixture matrix; borrow-conflict diagnostic snapshots; getter-returning-borrow example; zero runtime checks emitted for ordinary borrow checking outside explicit `SharedMut<T>` use.
- **Stage 22 — Nullable + narrowing + `is` + `mixed` statics.** D5 complete, plus D21 static semantics: `mixed` accepts every type and rejects every operation until narrowed; `is`/`match` narrowing over `mixed`; `null` rejected in type position with a `?T` suggestion; `void` restricted to return position; `object` absent from the grammar and reserved-word guardrails; `resource` rejected as reserved. The boxed runtime representation lands in Stage 23. AC: null-safe chaining example; narrowing snapshot diagnostics; mixed operation-rejection and narrowing fixture matrix; null/void/object/resource position-diagnostic snapshots.

### Phase D — Collections and generics (Stages 23–26)
- **Stage 23 — Runtime collections + Bytes.** Owned `List/Dictionary/Set` and `Bytes` intrinsics in doria-rt as move types; literals, indexing (`$list[0]` borrows the element; panic OOB), insertion moves values in, `foreach` element borrows (`as $item` readonly, `as writable $item` exclusive). The boxed `mixed` runtime representation (`dr_mixed` tagged box) ships here alongside the other move-type intrinsics. AC: move/borrow fixture matrix over collections; in-place mutation loop example; use-after-move-into-list diagnostic snapshot; mixed box round-trip fixture (int/string/class in, narrowed out).
- **Stage 24 — Generic functions.** D9 for free functions/methods, monomorphization in MIR. AC: `first<T>` works across int/string/class lists.
- **Stage 25 — Generic classes + compiler-internal iteration machinery.** `Stack<T>` and generic collection/runtime machinery; `foreach` over built-in collections lowers through compiler-internal iteration support. Public generic interfaces, traits, and user-defined `Iterable<T>`/`Iterator<T>` conformance are deferred to Stage 35. AC: generic classes run natively; built-in collections are consumed by `foreach` without user-visible interface conformance.
- **Stage 26 — Collection API surface.** Decision 0051 methods incl. `map`/`filter` once Stage 30 closures exist (split: non-closure API here, closure API revisited in Stage 30). Before Stage 31 include/multi-file support, required stdlib fragments are compiler-bundled or prelude-style rather than source-included. AC: non-closure collection APIs compile and run from the compiler-provided stdlib surface.

### Phase E — Enums, match, errors (Stages 27–29)
- **Stage 27 — Enums + payload cases.** D6, inline tagged layout, Copy/move classification per payloads. AC: `Shape` example native.
- **Stage 28 — match.** D7, exhaustiveness, payload destructuring, narrowing integration; guards fast-follow within the stage. AC: exhaustiveness diagnostics snapshots; `match (true)` chains.
- **Stage 29 — Checked errors end-to-end.** D8: `throws` ABI, `try/catch/finally`, `Error` interface with property requirement, `main throws`. AC: SPEC-style `loadUser` example; uncovered-error diagnostics; finally-ordering fixture; error-escaping-`main` fixture asserting stderr `error: <Class>: <message>`, exit status 70, and destructor execution on the propagation path.

### Phase F — Multi-file, namespaces, Baton (Stages 30–33)
- **Stage 30 — Closures.** D10 + D20: explicitly typed closure and arrow-function parameters (omitted parameter types are compile errors with context-derived suggestions), borrow and `take` captures, function types in type position; collection closure APIs unlock. AC: sort-with-comparator example with typed callback parameters; borrow-bound closure escape rejected with a `take` suggestion fixture.
- **Stage 31 — Namespaces/use/include/declare.** Decision 0028 implemented; multi-file compilation; first declare keys. AC: multi-file example project builds; duplicate-symbol diagnostics; Doria stdlib fragments that were compiler-provided in Stage 26 now compile through `include`.
- **Stage 32 — Attributes.** `#[...]` parsing, type-checked against attribute classes, const-evaluation-tier arguments (resolving SPEC §11's evaluation-policy question: compile-time const evaluation only, no side effects); reflection deferred — attributes are compiler/tooling metadata in v1.0. AC: `#[Test]`, `#[PhpExport]` representable.
- **Stage 33 — Baton MVP.** §11 scope: new/build/run/test/check, path deps, `#[Test]` runner. AC: `baton new game && baton test` green out of the box.

### Phase G — OOP completion (Stages 34–36)
- **Stage 34 — Inheritance.** D17: `open`/`override`, vtables, parent construction rules, devirtualization in LLVM profile. AC: `Post extends Model` native; missing-override diagnostics.
- **Stage 35 — Interfaces + traits.** Conformance checking, interface-typed values (fat pointer or vtable-embedded — decide in 0053/0054), trait flattening + `insteadof`/`as`. `Cloneable` becomes the public explicit-duplication contract here, and user-defined `Iterable<T>`/`Iterator<T>` conformance plugs into `foreach`. AC: SPEC §8 examples native; a user-defined iterable is consumed by `foreach`; a Cloneable class can be cloned through the interface contract.
- **Stage 36 — Property hooks + when.** §6.4 hooks; `when` grammar per 0064. AC: `Temperature` example; when-chain example.

### Phase H — Concurrency (Stages 37–39)
- **Stage 37 — Concurrency design record 0063.** Paper stage: full async model (executor in doria-rt, task groups, cancellation, `Shareable` rules). Designer sign-off required — this is the one deliberate design gate in the plan.
- **Stage 38 — async/await codegen.** State-machine lowering in MIR; single-threaded executor first; `async main` entry bootstrap per §5 (executor started only when `main` is async — sync programs pay zero async cost). AC: async file-read example; interpreter parity; async-`main` escaping-error example exits 70 with destructors run.
- **Stage 39 — Structured task groups + Shareable checking.** Multi-threaded executor; spawn-boundary checks via auto-derived `Sendable`/`Shareable` — with ownership in place this is Rust Send/Sync-grade freedom from data races. AC: parallel map example; data-race fixture rejected at compile time.

### Phase I — Systems and PHP bridge (Stages 40–42)
- **Stage 40 — unsafe/FFI.** D12: `unsafe`, `Ptr<T>`, `extern "C"`, linking foreign libs via Baton manifest. AC: bind and call a C function (e.g., zlib) from Doria.
- **Stage 41 — php-lib bridge.** D13c end-to-end behind public `baton build --php-lib`: export analysis, C-ABI shim gen, PHP FFI stub gen, handle lifetime tests against real PHP 8 in CI. doriac provides compiler emission primitives only as needed by Baton. AC: the `ImageResizer` scenario runs from a PHP script in CI through Baton.
- **Stage 42 — migrate php v0.** §10.2 conservative converter. AC: converts a small idiomatic PHP 8 fixture app; dynamic features produce diagnostics not silent guesses.

### Phase J — Engine enablers and 1.0 hardening (Stages 43+)
- **Stage 43 — Engine profile.** declare-based overflow relaxation for audited modules; arena allocator hooks in doria-rt; benchmark suite (criterion-style) vs C/Rust baselines for drop-glue, `Shared<T>`, and collection hot paths.
- **Stage 44 — SIMD direction (0065)** + `std::net`/`http` maturation for the PHP sidecar pattern.
- **Stage 45 — Self-hosting start.** Port the lexer to Doria as the first self-hosted component (per docs/self-hosting.md), compiled by `doriac`, differentially tested against the Rust lexer.
- **1.0 gate:** spec freeze pass over SPEC.md, diagnostics audit, doria-rt ABI review, differential + fuzz suites green, the three flagship demos build: a small game (engine seed), a UI component demo, and a PHP app calling a Doria php-lib.

### Dependency notes for the implementing agent
- Nothing in Phases B–J may begin before Stage 11 lands (everything depends on MIR + oracle).
- WASM backend remains recognized-but-unscheduled; do not start it before 1.0.
- Game engine and UI framework are **separate repositories** consuming Doria; this plan only builds their enablers. Do not scaffold them inside the compiler repo.

---

## 14. What is explicitly out of scope for v1.0

Tracing GC (never), pervasive ARC as the default model (never), Rust-spelled borrow sigils and lifetime annotations (never — inference and elision only), visibility modifiers beyond the default-accessible + `internal` two-state model — `protected`, `private`, and `public` keywords (never; this is identity, not scope deferral), an `object` type (cut; reintroduce only with concrete PHP-bridge evidence), `resource` as a core type (reserved to the Phase I bridge), `Result<T,E>` surface model (per 0035), unions beyond `?T`, `goto`, textual macros (per 0028), runtime reflection, package registry server, catchable panics, user-defined operator overloading, default interface methods, variadic generics, and bidirectional PHP compatibility guarantees.

---

## 15. Summary for the designer

This plan turns Doria's accepted principles into a complete, ordered build-out: a genuine ownership and borrow-checking model re-spelled into Doria's existing readonly/writable vocabulary plus `take` — Rust's machinery with none of its sigils; a finished type system (fixed-width numerics, nullables, payload enums, exhaustive match, monomorphized generics); checked errors as the recoverable path and abort panics as the fatal one; closed-by-default OOP with traits and hooks; a real MIR with an interpreter oracle and dual Cranelift/LLVM backends over one semantics; a Doria-authored stdlib; Baton; and — as the strategic differentiator — a first-class native bridge that lets any PHP application call compiled Doria as if it were a normal PHP class. Approve or amend the Section 1 table, and the rest executes stage by stage without further design stalls.