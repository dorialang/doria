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
not produce a partially initialized object. A constructor may traverse a
definitely initialized writable property and mutate the owned child normally;
it does not thereby gain a generally writable `$this`. Independently owned
values may initialize owning properties or replace initialized writable owning
properties, with the replacement acquired before the previous value is
destroyed.

Bare `{ ... }` blocks provide an explicit shorter lifetime boundary when a value
or access guard should be cleaned up before the surrounding function continues.

Closures use explicit `with` capture lists and structural function types.
Captures are acquired in written order, function values move as one owned value,
and owned captures are released in reverse order when the closure dies.
Valid closures execute through the debug interpreter, both native profiles, and
the PHP compatibility backend where the surrounding value surface is supported.
No-capture closures allocate no environment. Native nonescaping captures use
stack storage and escaping environments use one allocation without reference
counting; PHP uses explicit compiler-generated carriers, environments, and
stable places rather than PHP automatic capture or references as language
semantics.

`List<T>` provides `map`, Copy-preserving `filter`, and writable-accumulator
`reduce`. These methods borrow the source readonly, visit elements in insertion
order, accept readonly- or writable-repeatable callbacks, and propagate the
callback's exact checked Errors. `map` may produce owned Move values, while
`filter` remains limited to Copy elements until an explicit cloning capability
exists. Other collection families do not expose these algorithms.

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
- **Honest defaults.** Booleans print as `true` and `false`. Integer overflow is an error, not a wraparound. Format strings are checked at compile time. Reusable callables declare required checked errors with `throws`; canonical I/O failures propagate ambiently and remain catchable. The selected program entrypoint infers what escapes it, and `try`/`catch` handling remains statically checked.
- **Small language, sharp edges filed off.** Where a familiar construct is a known footgun, Doria deliberately does the safer thing instead.
- **Purpose-shaped collections.** Insertion-ordered dictionaries and sets sit
  beside ascending sorted variants, a min-first priority queue, and one deque
  that handles both FIFO and LIFO workflows. Collection ordering and ownership
  remain identical across the debug, native, and compatibility backends.
- **Concise initialization without hidden work.** Grouped locals such as
  `let writable $red, $green, $blue = 0;` evaluate one Copy initializer once,
  create independent bindings in source order, and never hide cloning, sharing, or a runtime tuple.
- **Nominal choices without object overhead.** Unit, backed, and payload enums
  are inline values with exact case identity. Payloads are Copy only when every
  field is Copy; otherwise ownership moves with the enum. Backed values are
  explicit through readonly `value`, and enums never silently become integers,
  strings, objects, or display text.

## What people build with it

- **Native services and CLI tools** — single-binary deployment, measured cold startup, predictable memory.
- **Portable terminal applications** — first-class, cross-platform TUI support (Windows, macOS, Linux) with no hand-written escape sequences: terminal games and tools that just run everywhere.
- **Game engines and performance-critical systems** — deterministic destruction, fixed-width numerics, statically specialized abstractions, and a safe interop story with native libraries.
- **Native power for PHP applications** — Doria libraries compile to packages that PHP code calls like ordinary classes, with generated, type-checked bindings.

## Lineage

Doria was created by a PHP developer who wanted compile-time safety and native performance without giving up readable syntax, and it doesn't hide that. If you know PHP, you'll feel at home in minutes; the `$variables`, the class shapes, the pragmatism all carry over. But familiarity is a doorway, not the destination: Doria is its own language, with its own type system, its own memory model, and its own opinions about what a language owes the people who read code as often as they write it.

`match` is an exhaustive value expression. It handles enum cases, payload
destructuring, exact constants, nullable values, and exact type narrowing while
evaluating its input once and executing one arm:

```doria
string $message = match ($result) {
    Delivery::Queued($attempt) if $attempt > 1 => "retried",
    Delivery::Queued($attempt) => "queued",
    Delivery::Sent($reference) => "sent {$reference}",
    Delivery::Failed($code, $reason) => "failed {$code}: {$reason}",
};
```

Open domains use a final `default`. `match (true)` provides ordered strict-bool
conditions, and full ternary uses the same typing and ownership rules. A guard
uses `if`, runs only after its pattern matches, and may fall through to a later
arm. `match (take $value)` explicitly gives the whole value to the match so a
selected Move payload can become an owned arm binding.

`when` handles branch-local work that must produce one value, while `given`
prepares shared state and strict-bool predicates for `if`, `when`, or `while`:

```doria
string $message = given {
    let $ready = serviceIsReady();
    $ready;
} when ($hasWork): string {
    return "ready";
} else {
    return "waiting";
};
```

Base `do ... while` and control-flow `finally` are executable. `finally` attaches
to `if`, `when`, `while`, and `do ... while`, runs once for each normal or
structured exit, and may propagate a checked Error. A failing finalizer replaces
the pending nonfatal outcome and destroys any superseded owned payload exactly
once; an outer catch may recover, while a sibling catch on the same `try` cannot.
Fatal panic remains abort-only and runs no cleanup.

Doria parses, checks, and executes explicit `Error` conformance, `throw`,
source-ordered `throws`, and `try`/`catch`/`finally`. Handled errors use one
backend-independent MIR and deterministic cleanup model across the interpreter,
Cranelift, LLVM, and the PHP compatibility backend. Text, file, and standard-
device I/O expose exactly the ambient checked errors
`Doria\Std\Io\IoError` and `Doria\Std\Io\InvalidUtf8Error`. They do not
require source `throws`, but remain exact and explicitly catchable. An Error
that escapes `main` is reported as `R1000` after cleanup and exits with status 70.
Fatal panic remains a distinct cleanup-free status-101 outcome.

## Tooling

Official language-server and editor integrations are developed separately in [`dorialang/doria-language-server`](https://github.com/dorialang/doria-language-server), which consumes reusable `doriac` frontend services without duplicating compiler semantics.

Doria source may declare one file-wide namespace and use individual, aliased,
or grouped imports. Qualified names are exact and absolute whenever they contain
`\`; unqualified names resolve through explicit imports, the current namespace,
and the documented edition prelude. Package source discovery and `include`
operate at compile time; Doria source loading is never lowered to runtime PHP.
`doriac` accepts a strict versioned JSON build plan containing the selected
target, explicit source inventory, package graph, source scopes, namespace
mappings, and compiler profile. Every active source is checked as one package
compilation graph, while `internal` remains visible only inside its package.
The compiler-facing format and CLI forms are documented in
[`docs/build-plan-schema.md`](docs/build-plan-schema.md).

Doria attributes are typed compiler metadata with no runtime reflection:

```doria
#[Attribute]
class Route
{
    function __construct(string $path) {}
}

#[Route(path: "/posts")]
#[Test]
function routeCanBeMatched(): void
{
}
```

Attribute arguments reuse ordinary named/default argument binding and the
bounded constant evaluator. Applying an attribute executes no constructor or
other Doria code. `#[Test]` is metadata for future Baton orchestration, while
`#[PHPExport]` remains metadata for the later PHP bridge. Use `doriac metadata`
or `doriac metadata --build-plan` to inspect deterministic typed metadata.
The compiler validates the versioned processor protocol but executes no processor
and writes no generated source. See
[`docs/attribute-metadata-protocol.md`](docs/attribute-metadata-protocol.md).

Baton is the accepted project and package tool for Doria. It is the Doria-native project and package tool
maintained in the `dorialang/baton` repository. The current bootstrap reads manifest schema 1 only; the
accepted target adds schema 2 with package identity, targets, compile-time `autoload`
mappings, dependencies, development dependencies, and explicit processors. Baton
produces a versioned build plan for `doriac`. Compiled executables will never search for or load source files at runtime.
Package versions use SemVer, deterministic resolution is recorded in JSON
`Baton.lock`, and each workspace has one root lockfile without merging package
`internal` boundaries. Production toolchains ship the native Baton executable
and require no Baton PHP runtime or Composer payload.

The CLI supports human, concise, and versioned JSON diagnostics. Human and
concise output go to stderr; JSON goes to stdout for tools:

```console
doriac check main.doria --diagnostic-format human
doriac check main.doria --diagnostic-format concise
doriac check main.doria --diagnostic-format json
```

Color is controlled independently with
`--diagnostic-color auto|always|never`; automatic color honors `NO_COLOR`.

Performance evidence lives in the separate
[`dorialang/benchmarks`](https://github.com/dorialang/benchmarks) repository.
Its manifest checks committed exact output before timing, keeps Cranelift and
LLVM results distinct, retains the raw samples behind its statistics, and
records the compiler, build driver, toolchain, commands, and host used for each
report. Public comparisons are workload-specific and reproducible rather than
broad language rankings.

The benchmark matrix includes deterministic compiler scaling, runtime-subsystem
cases, process resources, artifact sizes, and optional profiler evidence. Shared CI
compares deterministic structure; controlled runners alone own timing thresholds.

Runtime panics use the same structured diagnostic model and format selection.
Built-in panics have stable `P` codes, a precise Doria source label, an
explanation, a Doria-only `Call Path`, and status 101. `doriac run` receives
native runtime facts over a private structured channel; neither it, the
language server, nor the Playground parses rendered terminal prose.

Unhandled checked errors use that same structured model without becoming
panics. Human and concise output safely escape untrusted Error messages, JSON
preserves the exact logical message, and the first Doria throw or built-in I/O
effect site remains the origin. Ordinary closed stdout/stderr pipes retain their
separate clean status-0 behavior.

---

<div align="center">

*Readable code. Checked contracts. Native binaries.*

</div>
