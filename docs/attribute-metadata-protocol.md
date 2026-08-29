# Attribute Metadata Protocol

This document describes the compiler-owned Stage 32 attribute metadata and
processor protocol. Decision 0125 is authoritative.

## Source Syntax

```doria
#[Attribute]
class Route
{
    function __construct(
        string $path,
        HttpMethod $method = HttpMethod::Get,
    ) {
    }
}

#[Route(path: "/posts", method: HttpMethod::Post)]
#[Test]
function createPost(): void
{
}
```

`#[` must be adjacent. Ordinary `#` comments remain comments. Attribute groups
precede declaration modifiers and retain authored grouping, order, commas,
parentheses, qualified names, arguments, and source spans.

User attribute classes are marked with `#[Attribute]`. Their constructor
parameters define the schema. Parameters are readonly and use
metadata-compatible types. `#[Test]` and `#[PHPExport]` are compiler-known
zero-argument metadata markers; neither activates runtime behavior.

## Constant Metadata Values

The compiler type-checks each argument against its bound schema parameter and
then evaluates it through the Decision 0084 constant evaluator. Accepted values
are exact implemented numerics, `bool`, `string`, compatible nullable values,
constants, and compatible unit, backed, or payload enums.

Runtime calls, I/O, constructors, static factories, closures, objects,
collections, typed arrays, `Bytes`, `mixed`, and shared-reference values are not
Stage 32 metadata. Attribute applications execute no code.

## Metadata Command

```console
doriac metadata source.doria
doriac metadata --build-plan build-plan.json
doriac metadata source.doria --schema-version 2
doriac metadata --build-plan build-plan.json --schema-version 2
```

The command emits one deterministic JSON document to stdout. Schema version 1 remains the default,
and explicit `--schema-version 1` is byte-identical to that
default. Decision 0128 adds strict schema version 2 for Baton test discovery;
unknown versions are rejected.
It produces no backend artifact, invokes no processor, writes no generated
source, and does not modify input files. Blocking diagnostics follow the
ordinary CLI diagnostic rules.

The document contains:

| Field | Meaning |
| --- | --- |
| `schemaVersion` | Exact metadata schema version; `1` by default, or explicit `2` |
| `edition` | Doria source edition |
| `compilerRevision` | Compiler revision that produced the document |
| `graphFingerprint` | Deterministic identity of the complete compiler input |
| `selectedTarget` | Package, binary/library kind, and optional entry source |
| `packages` | Ordered package identities |
| `sources` | Ordered public source identities and display paths |
| `attributeClasses` | Compiler-known and user schema declarations |
| `applications` | Ordered, resolved, typed attribute applications |

Schema version 2 contains every schema-version-1 field plus `callables`. Each
callable records one canonical attribute-target-compatible identity, name, kind,
package, source, access, generic arity, ordered parameters, return type, required
effects, ambient effects, and location. Parameters preserve index, name, exact
semantic type, and `readonly`, `writable`, or `take` ownership. Functions,
methods, constructors, and destructors remain distinct.

Callable metadata is static compiler output. It exposes no compiler numeric ID,
Rust type, MIR or backend symbol, runtime address, or reflection API.

Source locations carry a public source identity, display path, and authoritative
byte range. Private absolute host paths, compiler numeric IDs, Rust type names,
backend symbols, and runtime addresses are not protocol data.

## Typed Values

Values carry an explicit `kind` and Doria semantic `type`. Integers and floats
use canonical strings so width, signedness, formatting, and precision do not
depend on a JSON implementation.

```json
{
  "kind": "integer",
  "type": "int32",
  "value": "42"
}
```

```json
{
  "kind": "float",
  "type": "float",
  "value": "3.5"
}
```

Strings and booleans carry JSON string and boolean values. Null carries its
expected nullable type. Enum values carry canonical type and case names.
Payload enums additionally carry ordered recursively typed field values. These
records describe Doria metadata, not native or PHP memory layout.

## Processor Request Version 1

`AttributeProcessorRequestV1` is a strict, typed subset suitable for one future
explicitly selected processor. It contains:

```text
schemaVersion
edition
compilerRevision
graphFingerprint
processorPackage
selectedTarget
attributeClasses
applications
```

The processor package is supplied by future Baton orchestration. `doriac` does
not parse `Baton.toml`, discover processor packages, or execute a request.
Metadata schema version 2 does not change this protocol: processor requests and responses remain strict schema version 1,
and Baton constructs requests from the
metadata fields common to both document versions.

The parser rejects unknown fields, missing fields, unsupported schema versions,
invalid package identities, inconsistent bound-value types, malformed or
oversized numerics, and unsupported metadata kinds.

## Processor Response Version 1

`AttributeProcessorResponseV1` contains:

```text
schemaVersion
graphFingerprint
diagnostics
generatedSources
```

The response fingerprint must match its request. Validation does not execute a
processor or write any output.

### Diagnostics

Processor diagnostics have processor-owned codes, severity, a Title Case title,
message, source-identified labels, and optional explanation/help. A processor
cannot claim a compiler-owned code, use an unknown source, inject terminal
control sequences, or supply an unstructured terminal transcript. Stage 32
does not accept processor fixes that edit handwritten source.

### Generated sources

A generated source proposal contains:

```text
relativePath
generatedFor
UTF-8 contents
contentHash
```

`generatedFor` is `main` or `development`. The relative path must be a normalized
package-relative path. Validation rejects:

- absolute, drive-qualified, URL, empty, dot, NUL-containing, or traversing
  paths;
- backslash-based or repeated-separator aliases;
- duplicate normalized paths and case collisions;
- a path that would overwrite handwritten source;
- an incorrect content hash;
- command, environment, binary-artifact, dependency, or manifest mutation
  fields.

Stage 32 only validates proposed output. Stage 33 Slice 3 owns explicit
processor execution, output writes, generated-source inventory, and graph
insertion.

## Runtime And Reflection Boundary

Typed attribute metadata is retained in semantic information and HIR. MIR and
all runtime backends are metadata-free. The interpreter creates no attribute
value; Cranelift and LLVM emit no metadata table; PHP emits no PHP attribute or
registration helper.

Doria v1.0 has no runtime attribute lookup or dynamic reflection. `#[Test]`
execution belongs to Stage 33 Baton orchestration. `#[PHPExport]` bridge
semantics belong to Stage 41.
