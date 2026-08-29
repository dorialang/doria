# Decision 0125: Typed Attributes, Const-Evaluated Metadata, And Processor Protocol

- **Status:** Accepted
- **Accepted:** 2026-08-27
- **Date:** 2026-08-27
- **Implementation Status:** Implemented By Stage 32
- **Amends:** Decisions 0084, 0098, and 0118; SPEC section 11
- **Preserves:** Decisions 0117, 0119, 0121, 0122, 0123, and 0124

## Context

Doria needs declaration metadata for tooling, tests, future generated source,
and the Stage 41 PHP bridge without adopting PHP reflection or arbitrary
compile-time execution. Older supporting notes left open whether an attribute
could execute constructors, factories, I/O, or runtime registration. That
ambiguity conflicts with Doria's native-first, deterministic compiler model.

Stage 32 settles attributes as typed compiler metadata. The compiler parses and
resolves them, binds their arguments through the ordinary named-argument model,
evaluates a bounded constant tier, and exposes deterministic metadata. It emits
no runtime attribute objects or reflection tables. Stage 33 may orchestrate
explicit processors through Baton; Stage 32 defines the protocol but executes
no processor.

## Syntax And Source Model

An attribute group begins only with adjacent `#[` and ends with `]`:

```doria
#[Authenticated, Route(path: "/posts"),]
#[Tag("api")]
function createPost(): void
{
}
```

`# comment` and `# [Route]` remain line comments. Strings and comments that
contain `#[` are unchanged.

The AST preserves groups rather than flattening source spelling. It retains the
opening and closing delimiters, qualified-name segments and separators,
parentheses, source-order arguments, argument names, commas, trailing commas,
application spans, group spans, and authored group/application order. Attribute
arguments reuse the ordinary `Argument` representation from Decision 0098.

Groups precede every declaration modifier. An attribute after `internal`,
`static`, `writable`, or another modifier is rejected rather than silently
reordered.

## Targets

Stage 32 accepts attributes on:

- global classes, enums, interfaces, traits, functions, and constants;
- properties, methods, constructors, destructors, and class constants;
- free-function, method, constructor, and promoted-constructor parameters;
- enum cases and enum payload fields.

A promoted parameter is one authored target with parameter and
promoted-property roles. Its application is not duplicated.

Stage 32 does not accept attributes on namespaces, imports, includes,
statements, locals, blocks, expressions, closures or closure parameters, catch
or foreach bindings, generic parameters, return types, throws entries, or match
patterns.

Repeated applications and multiple groups are retained independently in
authored order. Stage 32 adds no target mask, repeatability declaration,
inheritance, merging, override, or automatic deduplication policy.

## Compiler-Known Attributes

Three names have deliberate compiler-known identity in attribute position:

- `#[Attribute]` marks a user class as an attribute schema. It accepts no
  arguments and is valid only on a class.
- `#[Test]` is zero-argument metadata. Stage 32 does not discover or execute a
  test; Stage 33 owns Baton test orchestration.
- `#[PHPExport]` is zero-argument metadata. It exports no symbol and creates no
  bridge; Stage 41 owns bridge semantics.

These identities are not fake runtime classes or package declarations. A user
class with the same short name does not replace them.

## User Attribute Classes

An ordinary non-generic class marked with `#[Attribute]` defines a schema:

```doria
#[Attribute]
class Route
{
    function __construct(
        string $path,
        HttpMethod $method = HttpMethod::Get,
    ) {
        // This body runs only if ordinary runtime source constructs Route.
    }
}
```

The constructor parameter list is the schema. A class without a constructor is
a zero-argument schema. Schema parameters are readonly and must use a
metadata-compatible type; `writable` and `take` are rejected. Attribute classes
cannot be generic in Stage 32.

Applying an attribute never constructs the class and never executes its
property initializers, constructor body, methods, destructor, I/O, panic, or
checked-error path. Runtime construction of the class remains ordinary Doria.

## Resolution And Visibility

User attribute classes resolve through Decision 0117's canonical resolver.
Unqualified names use explicit imports, the current namespace, and the edition
prelude after compiler-known handling. Qualified names are exact.

Schemas may be forward declared or live in another source in the complete
compilation graph. Same-package `internal` remains visible throughout the
package; dependency-internal declarations are inaccessible; transitive
dependencies are not implicitly visible. Package ownership is never inferred
from namespace spelling.

Attribute references are compiler-owned global references with the dedicated
`AttributeClass` role. A complete graph reports an unavailable class as a
language diagnostic. Partial IDE input reports it as missing compiler input.

## Binding And Defaults

Attribute applications reuse Decision 0098's argument binder. Positional
arguments may precede named arguments but may not follow them. Named arguments
may reorder parameters and skip supported defaults. Duplicate names, unknown
names, overflow, and missing required parameters use the shared causal
diagnostics.

The metadata record preserves both authored argument order and constructor
parameter order. A value inserted from a default is explicitly marked as
defaulted. Parameter names are public schema API.

A default is usable only when it is valid for the declaration, type-compatible,
metadata-compatible, and const-evaluable. Forward constant references and
constant cycles retain Decision 0084 behavior.

## Constant Evaluation

Attribute values reuse Decision 0084's bounded typed evaluator. The compiler
does not run MIR, native code, PHP, a host callback, or a general interpreter.

The Stage 32 value tier includes:

- exact implemented signed and unsigned integers and floats;
- `bool`, `string`, and `null` under a compatible nullable expectation;
- top-level and class constants;
- grouped expressions and supported typed unary, arithmetic, bitwise,
  comparison, boolean, and string operations;
- accepted explicit numeric companion conversions over constant operands;
- unit and backed enum values;
- payload-enum values whose fields recursively belong to this tier.

Doria overflow, conversion, shift, division, comparison, and nominal enum rules
remain authoritative. Host and PHP coercion are forbidden.

Stage 32 rejects runtime locals and parameters, property and mutable-static
reads, function and method calls, static factories, constructors, closures,
I/O, panic, time, randomness, environment access, mutation, loops, collections,
typed arrays, `Bytes`, `mixed`, class instances, function values, shared handles,
and access objects. A precise type error is reported before a const-evaluation
consequence. Invalid applications create no partial metadata record.

The richer `Module(imports: [...])`, object, collection, and factory examples in
older notes remain future metadata-expression direction. They are not Stage 32
source.

## Metadata-Compatible Types

Schema parameters may use exact implemented integer and float types, `bool`,
`string`, compatible nullable forms, unit or backed enums, and payload enums
whose fields are recursively compatible.

They may not use `void`, `mixed`, class types, function types, typed arrays,
named collections, `Bytes`, shared-reference families, access objects, Error
class instances, symbolic generic types, or unresolved types. Unsupported
values are never boxed or serialized as objects.

## Compiler Metadata

Semantic analysis owns typed attribute-class schemas and applications. Facts
include canonical class and target identities, package and source identities,
typed values, authored and bound argument order, defaults, spans, target roles,
and dependency references. Application identity derives deterministically from
source identity, target identity, and authored ordinals.

HIR carries the same resolved metadata table. It does not rebind arguments,
reevaluate expressions, or invoke constructors. Public dumps and JSON use
Doria-facing canonical names, never raw compiler IDs, Rust enum names, backend
symbols, runtime addresses, or PHP helper identities.

Attribute source, schema, signature, default, constant, enum, import,
visibility, source-inventory, and target-identity changes participate in the
existing compiler fingerprints and semantic dependency graph. A body-only edit
unrelated to metadata retains the existing declaration-index reuse behavior.

## Runtime Boundary

MIR contains no attribute operation, registration call, reflection table,
processor invocation, test registration, or PHP-export registration. Attributes
do not alter class, enum, closure, function, or collection layout.

The interpreter executes only the ordinary program. Cranelift and LLVM emit no
metadata section or registry. The PHP compatibility backend emits neither PHP
attributes nor registration helpers. All backends consume the same
metadata-free MIR, and metadata-only source changes add no Doria runtime cost.

## Metadata Command

The compiler provides:

```console
doriac metadata source.doria
doriac metadata --build-plan build-plan.json
```

The command runs the ordinary frontend and complete semantic graph, fails on
blocking diagnostics, writes deterministic schema-version-1 JSON to stdout,
and produces no backend artifact. It executes no processor, writes no generated
source, and mutates no input.

The document includes the edition, compiler revision, graph fingerprint,
selected target, packages, sources, attribute schemas, and ordered applications.
Typed integer and float values use canonical strings where JSON numbers would
lose type identity or precision. Enum values use canonical type and case names;
payload values retain field order, types, and values. Byte ranges are the
authoritative source location.

## Processor Protocol

Stage 32 defines strict Serde models for schema-version-1 processor requests and
responses. Unknown fields and unknown schema versions are rejected.

A request carries compiler and graph identity, processor package identity,
selected target, the exact source inventory and byte lengths, attribute schemas,
applications, typed values, and source location facts. Future Baton orchestration
supplies processor selection and package identity; `doriac` does not parse
`Baton.toml` or discover processors.

A response may contain structured processor diagnostics and proposed UTF-8
`.doria` sources for `main` or `development`. Every response must answer the
request graph fingerprint. Diagnostic labels must name a source in that request
and remain within its byte length. Generated paths must be normalized
package-relative `.doria` paths and may not be absolute, drive-qualified, URLs,
empty, dot paths, traversals, duplicates, case collisions, handwritten-source
replacements, commands, binary artifacts, environment mutations, dependency
edits, or manifest edits. Content hashes are verified.

Processor diagnostics use processor-owned codes, Title Case titles, bounded
structured fields, source-identified labels, and safe text. They cannot claim a
compiler diagnostic code or inject terminal controls. Initial protocol fixes do
not edit handwritten source.

Stage 32 validates and serializes the protocol but executes no processor and
writes no generated source. Stage 33 Slice 3 owns explicit Baton processor
orchestration and generated-source graph insertion.

## Consequences

- Attributes provide useful typed tooling metadata without runtime reflection.
- `#[Test]` and `#[PHPExport]` can be represented before their consumers exist.
- Named-argument and constant semantics remain single compiler services.
- Processor integration has a deterministic, versioned boundary without making
  the compiler a package manager or arbitrary build-script host.
- Future richer metadata expressions require separate accepted authority and
  cannot be inferred from Stage 32.

## Invalidated Elsewhere

- `SPEC.md` section 11 and `docs/executable-initializers-and-attributes.md` no
  longer describe attribute evaluation as unsettled or runtime-lowered.
- `docs/doria-end-to-end-plan.md`, `README.md`, and
  `docs/notes/current-pipeline.md` record Stage 32 and Stage 33 Slices 1 and 2
  complete and Stage 33 Slice 3 next.
- Decisions 0084 and 0098 record their implemented Stage 32 consumers.
- Decision 0118 records that Stage 32 supplies the metadata/protocol boundary
  while Stage 33 Slice 3 owns execution and generated-source orchestration.
- `dorialang/doria-language-server` must consume compiler-owned facts and pin
  the final Stage 32 compiler revision.
- The website needs a later coordinated content and playground update; Stage 32
  does not modify it.

## Verification

Mechanical checks and durable tests must prove lexer/comment compatibility,
source-preserving AST facts, complete target coverage, graph resolution and
visibility, shared argument binding, bounded constant evaluation, exact typed
metadata, deterministic JSON, strict processor validation, HIR retention,
metadata-free MIR/backends, runtime parity, and the absence of processor, test,
PHP-export, and reflection activation.
