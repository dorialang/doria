# Doria API Design Guidelines

> Documentation role: supporting design note.
> Source-of-truth hierarchy: `docs/doria-end-to-end-plan.md` owns future sequencing; accepted `docs/decisions/*.md` files own topic-level decisions. This note is subordinate to both.

Doria APIs should make intent obvious at the call site.

## Core rule

Use nouns for values and verbs for actions.

```text
Nouns are properties.
Verbs are methods.
```

Prefer property access for data:

```doria
let $body = $message->body;
let $headers = $message->headers;
let $status = $message->status;
```

Avoid vague zero-argument noun methods:

```doria
let $body = $message->body();
let $headers = $message->headers();
let $status = $message->status();
```

A bare noun method such as `body()` can be misread as an action, preparation step, mutation, or builder-style method. If the member represents data, make it a property.

## Naming and casing

The naming charter is category-based:

- Built-in free functions use `snake_case`, such as `read_line()` and `get_time()`.
- Userland free functions use `camelCase`.
- Instance methods, static methods, and companion/type APIs use `camelCase`, whether they are built in or user-authored.
- Properties, parameters, and named arguments use `camelCase`.
- Classes, interfaces, traits, enums, and enum cases use `PascalCase`.
- Constants use `SCREAMING_SNAKE_CASE`.
- Type parameters use single Pascal capitals such as `T`, `K`, and `V`.
- The inherited magic methods keep the PHP-shaped spellings `__construct` and `__destruct`.

Free-function casing and member casing are intentionally different:

```doria
let $now = get_time();
let $startsWithDor = String::startsWith($name, "Dor");
let $wrapped = Int::wrappingAdd(1, 2);
let $empty = $s->isEmpty();
let $tenant = $message->tenantId;
$message->retryAfter(seconds: 30);
let $person = $repository->findById($id);
```

Avoid snake_case methods, static methods, properties, parameters, and named arguments unless a later accepted decision introduces a specific exception.

Type-coupled vocabulary belongs to the type companion. Cross-domain and
environment capabilities may remain free functions. In particular,
`$string->` is reserved for intrinsic data, measurements, and views, while
every string-specific operation is a `String::` companion call:

```doria
let $length = $text->length;
let $bytes = $text->byteLength;
let $trimmed = String::trim($text);
let $matches = String::startsWith($text, "Dor");
```

Doria has no public `str_*` family and does not duplicate companion operations
as instance methods such as `$text->trim()`.

Shared ownership follows the same explicit-vocabulary rule. `share()` means
"create another owner" and `createWeakReference()` means "observe without
keeping alive"; neither is shortened to ecosystem-specific terms. The readonly
and writable families remain visible in signatures as `SharedReference<T>` and
`WritableSharedReference<T>` rather than being hidden behind one ambiguous
wrapper. `referencedValue` is the one compiler-known collision projection on
`SharedReference<T>`, not a general `.value` proxy convention.

## Capability and resource APIs

Decision 0110 supplies reusable rules for hosted resource APIs:

- Prefer small capability interfaces over universal god objects. A value should
  advertise only the operations it can actually perform.
- Make ownership visible. Resource handles are move values; ownership transfer
  uses `take`; explicit close/finish consumes the owner; borrowed adapters are
  explicitly distinguished from the default owning form.
- Use typed outcomes for ordinary state such as EOF, would-block, timeout,
  partial progress, lock contention, readiness, and closure. Do not encode those
  states as booleans, negative numbers, empty values, or other sentinels.
- Prefer named modes over boolean control flags, and expose the current mode when
  it is meaningful.
- Expose partial progress whenever the operating system or domain can make it;
  then provide readable high-level conveniences, such as complete writes, over
  that honest primitive.
- Use typed request/configuration values instead of string modes, boolean flag
  clusters, string-keyed option bags, global contexts, or dictionaries of
  `mixed`.
- Keep timing operation-local. Durations, deadlines, and cancellation compose
  across synchronous, asynchronous, network, process, and terminal work; they
  are not mutable process-global settings.
- Keep resource use bounded by default. Line/delimiter reads, capture, buffering,
  copying, and progress reporting must expose the relevant limit or policy.
- Make steady-state reuse possible. Chunk APIs accept reusable storage and expose
  safe readable/writable byte regions so progress does not require allocating or
  copying an entire chunk or unread suffix on every operation.
- Keep abstraction costs proportional to the chosen abstraction. Concrete
  generic adapters remain statically specializable; deliberate interface
  erasure may dispatch dynamically but does not allocate a heap object per call
  or per adapter layer.
- Reuse readiness registrations and event storage. Ordinary waiting neither
  busy-polls nor creates one thread per stream, and timeout bookkeeping is
  incurred only when timing is requested.
- Keep asynchronous machinery isolated. A synchronous program must not start an
  executor or allocate task/scheduler state merely because the standard library
  also supports asynchronous I/O.
- Separate buffer flushing from durable synchronization, and explicit
  failure-reporting cleanup from best-effort nonthrowing destruction.
- Keep platform mechanisms behind portable contracts. File descriptors, OS
  handles, polling APIs, shell parsing, and terminal encodings are backend facts.
- Prefer typed adapters and domain ownership over registries and wrappers named
  by strings. Each domain extends the common ownership/read/write/readiness/time/
  cancellation/backpressure foundation instead of creating a parallel one.
- Expose typed metadata through properties, capability interfaces, outcomes, or
  domain values rather than `Dictionary<string, mixed>`.
- Prefer clear, fully worded Doria vocabulary over abbreviated systems jargon
  unless an accepted naming rule establishes the abbreviation.

The semantic and performance patterns are settled. Exact Stage 36a public names
remain deferred under decision 0110 and must not be inferred from illustrative
audit vocabulary.

## Properties are for data

Use properties for stored values, state, identifiers, configuration values, computed values that are conceptually data, cheap derived values, and values exposed through validation or access control.

Examples:

```doria
$message->id
$message->body
$message->headers
$message->tenantId
$message->receivedAt
$alert->severity
$user->email
```

## Property hooks are the escape hatch

An externally accessible member can remain property-shaped even when access needs implementation logic. Property hooks should support validation, computed values, lazy decoding, caching, normalization, or guarded access without forcing data-shaped members to become vague noun methods.

Possible future shape:

```doria
class Message<T>
{
    internal string $rawBody;
    internal MessageDecoder<T> $decoder;

    T $body {
        get {
            return $this->decoder->decode($this->rawBody);
        }
    }
}
```

The exact property-hook syntax is not settled, but the API design principle is settled: property hooks should preserve clear property-style access for members that are conceptually values.

## Methods are for actions

Use methods for commands, mutations, operations with meaningful work, I/O, async operations, fallible operations, operations with required arguments, and behavior that is not simply exposing a value.

Examples:

```doria
await $message->acknowledge();
await $message->retryAfter(seconds: 30);
$report->renderPdf();
```

If a method primarily returns data but must remain a method because it performs I/O, expensive work, decoding, or another explicit operation, name it with a clear verb:

```doria
await $message->loadBody();
$message->decodeBody();
$repository->findById($id);
$client->fetchProfile($handle);
```

Prefer explicit verbs such as `load`, `read`, `decode`, `resolve`, `find`, `fetch`, `render`, `publish`, `acknowledge`, and `retry` over bare nouns.

## Avoid Rust-flavored API vocabulary

Doria may borrow safety ideas from Rust, but it should not inherit Rust surface vocabulary by default.

Avoid making examples and standard APIs feel Rust-shaped:

```doria
Ack::ok();
Result<T, E>;
Option<T>;
Dictionary::new();
```

Prefer Doria/PHP-shaped APIs:

```doria
$message->acknowledge();
return new AcknowledgeMessage();
return MessageDecision::Acknowledge;
```

Static calls are allowed where they make sense, especially for framework metadata or named constructors, but they should not become a default replacement for clear properties, constructors, or action methods.

## Settled direction

Settled:

```text
- Nouns should be properties.
- Verbs should be methods.
- Data-shaped members should not become vague zero-argument noun methods.
- Property hooks should preserve property-style access when values need validation, computation, lazy decoding, or guarded behavior.
- Methods should clearly communicate action, mutation, I/O, or meaningful work.
- Doria examples should avoid Rust-flavored API vocabulary unless that vocabulary has been intentionally adopted.
```

Open:

```text
- Exact property-hook grammar.
- Whether heavy computed properties should require an annotation or lint.
- Whether async property access should be disallowed entirely or represented through explicit methods only.
```
