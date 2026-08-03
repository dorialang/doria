<div align="center">
  <img src="res/images/doria-app-icon-warm.svg" alt="Doria Logo" width="200" height="200">

# Doria

**A statically checked, natively compiled language for building software you can read and binaries you can trust.**

</div>

---

## What is Doria?

Doria is a general-purpose systems language built around three commitments: **code that reads plainly, safety that is checked at compile time, and performance that is deterministic.** It compiles to standalone native executables with deterministic ownership and no tracing garbage collector. Runtime and allocation costs are specified explicitly and verified with workload-specific measurements, while the syntax stays approachable enough to be someone's first language.

```doria
function greet(string $name, int $year): string
{
    return "Hello, {$name}! Welcome to {$year}.";
}

function main(): void
{
    let $message = greet("newcomer", 2026);
    echo "{$message}\n";
}
```

## Memory safety, in plain words

Doria's memory model is built on ownership: every value has exactly one owner, and when the owner's scope ends, the value is cleaned up — immediately, deterministically, every time. Sharing is governed by two words:

- Everything is **readonly by default**. Passing a value grants the right to look, not to touch.
- **`writable`** grants exclusive access: mutation with a compile-time guarantee that nobody else is watching.
- **`take`** hands ownership over entirely — the signature says so, and the compiler holds everyone to it.

There are no annotations to sprinkle, no sigils to memorize, and no jargon in
the diagnostics. When something is wrong, the compiler names the problem,
labels every relevant source location, explains the Doria rule, and offers only
fixes whose safety is explicit:

> `Error[E0470]: Value Used After Ownership Transfer`
>
> `$user` was given away here and cannot be used afterward.

Use-after-free, data races, double-frees, null surprises: these are compile errors in Doria, not production incidents.

Construction follows the same rule: every property must be initialized on every
normal constructor path before the new object can be observed. Readonly
properties are initialized exactly once, writable properties may be changed
after their first initialization, and a branch that ends in a fatal panic does
not produce a partially initialized object.

Bare `{ ... }` blocks provide an explicit shorter lifetime boundary when a value
or access guard should be cleaned up before the surrounding function continues.

When a graph genuinely needs several owners, Doria provides an explicit escape
hatch rather than changing the default model. `shared new Node()` creates a
readonly `SharedReference<Node>`; writable shared graphs use
`WritableSharedReference<T>` and scoped readonly/writable access objects. The two
families are deliberately disjoint, additional ownership is always requested
with `share()`, and weak references break back-reference cycles without keeping
the payload alive. Ordinary owned values still pay no reference-counting or
runtime access-check cost.

## Design principles

- **Contracts are written down.** Every parameter is explicitly typed — always. Nothing silently defaults to a dynamic type, and nullability is spelled `?T` and enforced.
- **One word, one meaning.** `use` imports. `uses` composes traits. `with` captures in closures. No keyword in Doria ever has two jobs.
- **A standard library with one voice.** Type-specific operations live on companions such as `String::startsWith`, while cross-domain capabilities use fully worded free functions such as `read_line`. Each operation has one canonical spelling.
- **Unicode text with explicit units.** `$text->length` counts what a reader sees as graphemes, `$text->byteLength` reports UTF-8 storage, and String search, slicing, casing, splitting, and padding use deterministic Unicode rules on every native backend.
- **Honest defaults.** Booleans print as `true` and `false`. Integer overflow is an error, not a wraparound. Format strings are checked at compile time. Errors are declared with `throws` and handled with `try`/`catch` — the compiler makes sure of it.
- **Small language, sharp edges filed off.** Where a familiar construct is a known footgun, Doria deliberately does the safer thing instead.
- **Purpose-shaped collections.** Insertion-ordered dictionaries and sets sit
  beside ascending sorted variants, a min-first priority queue, and one deque
  that handles both FIFO and LIFO workflows. Collection ordering and ownership
  remain identical across the debug, native, and compatibility backends.

## What people build with it

- **Native services and CLI tools** — single-binary deployment, measured cold startup, predictable memory.
- **Portable terminal applications** — first-class, cross-platform TUI support (Windows, macOS, Linux) with no hand-written escape sequences: terminal games and tools that just run everywhere.
- **Game engines and performance-critical systems** — deterministic destruction, fixed-width numerics, statically specialized abstractions, and a safe interop story with native libraries.
- **Native power for PHP applications** — Doria libraries compile to packages that PHP code calls like ordinary classes, with generated, type-checked bindings.

## Lineage

Doria was created by a PHP developer who wanted compile-time safety and native performance without giving up readable syntax, and it doesn't hide that. If you know PHP, you'll feel at home in minutes; the `$variables`, the class shapes, the pragmatism all carry over. But familiarity is a doorway, not the destination: Doria is its own language, with its own type system, its own memory model, and its own opinions about what a language owes the people who read code as often as they write it.

## Tooling

Official language-server and editor integrations are developed separately in [`dorialang/doria-language-server`](https://github.com/dorialang/doria-language-server), which consumes reusable `doriac` frontend services without duplicating compiler semantics.

The CLI supports human, concise, and versioned JSON diagnostics. Human and
concise output go to stderr; JSON goes to stdout for tools:

```console
doriac check main.doria --diagnostic-format human
doriac check main.doria --diagnostic-format concise
doriac check main.doria --diagnostic-format json
```

Color is controlled independently with
`--diagnostic-color auto|always|never`; automatic color honors `NO_COLOR`.

Runtime panics use the same structured diagnostic model and format selection.
Built-in panics have stable `P` codes, a precise Doria source label, an
explanation, a Doria-only `Call Path`, and status 101. `doriac run` receives
native runtime facts over a private structured channel; neither it, the
language server, nor the Playground parses rendered terminal prose.

---

<div align="center">

*Readable code. Checked contracts. Native binaries.*

</div>
