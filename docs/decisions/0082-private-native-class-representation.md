# Decision 0082: Private Native Class Representation

Status: Accepted

## Context

Stage 19 is the first native class stage. The representation selected now must support deterministic ownership and destruction without pre-committing every future aggregate to heap allocation or placing reflection and dispatch state in each object.

Doria has no runtime reflection requirement, and static ownership keeps the concrete class type known at ordinary cleanup sites. Future compiler-known Copy aggregates, including the Stage 47 math value types, need to share layout machinery without being mistaken for heap-owned class references.

## Decision

### Headerless class payload

A concrete owned class value is an opaque pointer to a headerless, data-only heap payload. Properties occupy compiler-known offsets in the payload. Immutable per-type metadata—size, alignment, and drop glue—exists once per type rather than once per object.

Class metadata and the allocation ABI are private, versioned `doria-rt` implementation details. They are not source ABI, reflection API, or a promise that user code can inspect object headers.

### Static concrete destruction

At every cleanup site whose concrete class type is known, destruction is statically resolved and may be inlined. No metadata lookup or indirect call is required. Static drop metadata is reserved for abstracted cases such as future interface values or generic drop code.

### Future interface dispatch

Stage 35 interface values use fat pointers containing a data pointer and a vtable pointer. Interface support must not retrofit a vtable or type tag into every class allocation after headerless objects have shipped.

### Copy aggregates remain distinct

Compiler-known inline Copy aggregates use a separate representation path. They may share field-layout calculations with classes, but heap-vs-inline and move-vs-Copy are independent of whether per-type metadata exists. Neither representation uses a per-object metadata header.

## Alternatives considered

### Per-object reference count

Rejected. Classes are unique move types, not implicitly shared values. Reference counting would change their ownership semantics, add traffic to every transfer, and obscure deterministic destruction.

### Per-object type or vtable header

Rejected. Concrete drops are statically known, reflection is absent, and future interface dispatch can carry its vtable alongside the reference only where abstraction requires it.

### Inline all classes

Rejected. Classes are identity-bearing, owned heap values. Inline layout is retained for Copy aggregate categories rather than conflated with classes.

## Consequences

The compiler owns one canonical class-layout model used by MIR validation, the interpreter, Cranelift, and LLVM. The runtime supplies allocation and deallocation primitives but does not own source-level class metadata or generic reference counting.

Allocation failure follows decision 0083. Destruction order and the point at which the payload is freed follow decision 0081.
