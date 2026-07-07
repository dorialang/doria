# Doria API Design Guidelines

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

## Properties are for data

Use properties for stored values, state, identifiers, configuration values, computed values that are conceptually data, cheap derived values, and values exposed through validation or access control.

Examples:

```doria
$message->id
$message->body
$message->headers
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
