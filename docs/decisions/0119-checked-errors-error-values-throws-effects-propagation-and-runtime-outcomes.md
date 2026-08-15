# Decision 0119: Checked Errors, Error Values, Throws Effects, Propagation, And Runtime Outcomes

- **Status:** Accepted
- **Accepted:** 2026-08-15
- **Date:** 2026-08-15
- **Implementation status:** Stage 29 Slice 1 complete; Slice 2 next; Slice 3 pending
- **Scope:** Checked-error syntax, static effects, ownership, future execution,
  runtime outcomes, and I/O failure migration
- **Amends:** Decisions 0035, 0109, 0116, and 0117

## Context

Decision 0035 selected checked `throw`/`throws` as Doria's error model, but left
the complete grammar, semantic contract, runtime representation, cleanup path,
and public outcome unresolved. Later decisions established ownership, structured
finalizer regions, compiler-owned diagnostics, package identity, and the future
I/O migration. Stage 29 now binds those pieces without allowing execution work
to define the language retrospectively.

## Plain-Language Model

Recoverable failures are ordinary owned values implementing the compiler-known
`Error` interface. A callable declares every checked error that may escape with
`throws`. `throw` transfers an owned error into the propagation path. A caller
must catch each possible error or declare it. Checked propagation performs
required cleanup but does not roll back completed mutations, output, or other
side effects. Fatal panic remains non-catchable, cleanup-free, and status 101.

## Built-In `Error`

`Error` is a compiler-known core interface, not a base class or magic class-name
test. A class conforms only when it explicitly declares `implements Error` and
provides an externally accessible, readonly, stored `string $message` property.
A promoted readonly constructor parameter named `message` satisfies the
contract. Missing, `internal`, `writable`, or non-string properties do not.
Nothing is synthesized.

```doria
class StorageError implements Error
{
    function __construct(
        string $message,
        string $operation,
    ) {
    }
}
```

Error classes are ordinary owned Move classes and may have additional typed
fields. There is no automatic cause or previous-error chain. Wrapping uses an
ordinary explicitly typed field. `Error` values expose only readonly `message`
through the erased interface. Equality is object identity; hashing, ordering,
and structural field comparison are not implied.

Concrete Error classes may use existing class sharing. `SharedReference<Error>`
is deferred until Stage 35 general erased-interface payload support.

## Runtime Representation Reserved For Slice 2

The erased Error carrier is two machine words: the concrete object pointer and
a static Error descriptor pointer. Each conforming object gains one private,
optional first-throw origin slot. Construction leaves it empty, the first throw
sets it, and rethrowing the same object preserves it. This slot is not a source
property, public object header, reflection facility, or second allocation.

Slice 1 records semantic metadata for explicit conformance, the resolved
message property, concrete identity, future descriptor identity, and the future
origin requirement. It does not change executable class layout or create
runtime descriptors.

## `throws` Grammar And Effects

`throws` follows an explicit return type:

```doria
function loadRecord(string $id): Record
    throws RecordNotFoundError, StorageError
{
}
```

Named functions and methods still require an explicit return type. Constructors
may omit a return annotation and declare `throws`; destructors may not declare
or allow a checked error to escape. `main` may declare concrete errors or
`Error` while preserving its accepted parameter and return shapes.

Each entry is either a concrete Error-conforming class or `Error`. Nullable
types, primitives, `mixed`, collections, typed arrays, enums, shared handles,
unknown types, non-Error classes, and general interfaces are rejected. Source
order is preserved for HIR, documentation, hovers, signatures, and diagnostics;
checking uses a normalized semantic set. Duplicate entries are rejected.
`throws Error, StorageError` is rejected because `Error` already covers the
concrete entry.

Every resolved callable has an explicit checked-effect set, including free
functions, instance/static methods, constructors, `main`, and generic
specializations. Empty means nonthrowing. A nonthrowing or narrower callable
fits a position accepting a wider set; a wider set does not fit a narrower one.
Future overrides may preserve or narrow effects, never widen them. Stage 30
must carry the same law into callable and closure types, but owns closure effect
syntax and inference policy.

## Sources And Boundaries Of Effects

Effects arise from direct `throw`, resolved free/method/static/constructor calls,
instance property initializers, nested control flow, catches, and nested `try`.
Catches subtract only covered protected-body effects. Catch bodies are checked
independently; sibling catches do not handle errors raised by another catch.
Every uncovered error must be declared at the enclosing callable boundary. One
diagnostic reports the complete uncovered set.

Constructor effects include property initializers and the constructor body. An
explicit constructor must cover both. An implicit constructor cannot hide a
throwing initializer and must be replaced by an explicit contract. Constant and
static initializers may not throw. Destructor bodies must handle every checked
error locally.

## `throw`

`throw expression;` is a statement. The expression is evaluated once, must
produce an owned Error-conforming value, transfers that ownership, and adds its
concrete type to the current effect set. Throwing a named Move binding uses the
ordinary moved-value analysis. A borrowed Error cannot be thrown or cloned
implicitly. Rethrow is `throw $error;`; bare and expression-position throw are
not accepted in v1.

## `try`, `catch`, And `finally`

```doria
try {
    loadRecord("R-17");
} catch (RecordNotFoundError $error) {
    echo $error->message;
} catch (StorageError) {
    echo "storage unavailable";
} finally {
    recordAttempt();
}
```

A `try` statement requires at least one `catch` or `finally`. Catches precede
the optional finalizer. Each catch names one Error type; union catches are not
part of Stage 29. A binding is optional. When present it is an owned readonly
binding scoped to its catch. An omitted binding creates no source symbol; the
compiler still owns and destroys the caught value unless it is moved.

Try-body locals are unavailable in catches and finally. Catch locals are
unavailable in sibling catches and finally. Finally sees the enclosing lexical
scope. Existing shadowing rules apply.

One centralized `covers(catch_type, thrown_type)` operation defines matching.
In Stage 29 a concrete catch matches exact concrete identity and `catch (Error)`
matches every checked error. Stage 34 may add superclass coverage and Stage 35
may add general interface coverage by extending that operation. Duplicate exact
catches, every catch after `Error`, and catches proven unable to match any
protected effect are unreachable. An open `Error` effect keeps concrete catches
potentially reachable.

An attached finally may contain throwing work only when nested handling absorbs
every error. No checked error may escape finally and replace a pending return,
yield, break, continue, normal completion, or checked error.

## Cleanup, Side Effects, And Failed Construction

Checked errors reuse Decision 0116's structured finalizer regions. They do not
use native unwinding, a runtime cleanup stack, `setjmp`, or `longjmp`. The Error
carrier is acquired before cleanup; locals drop in reverse order; crossed
finalizers run inner-to-outer; and a caught Error drops exactly once unless moved
or rethrown. Fatal panic remains a separate edge that performs no cleanup.

Cleanup is not transactional. Mutations, output, filesystem changes, database
changes, and other side effects completed before the throw remain completed.

If construction fails, the class is never successfully constructed and its
ordinary `__destruct` does not run. Successfully initialized owned properties
drop once in reverse initialization order; uninitialized properties are
ignored; constructor parameters and temporaries clean normally; allocation is
freed; then the checked error propagates.

## Native ABI And MIR Reserved For Slice 2

Nonthrowing callables preserve their ABI. A throwing non-void callable uses its
ordinary arguments plus caller-owned success and Error out slots and returns a
u8 status: zero initializes success, one initializes Error. Throwing void omits
the success slot. Storage is function-entry or backend-equivalent fixed frame
storage; successful calls allocate no heap memory; propagation moves the same
Error carrier without cloning or reallocating its object.

Slice 2 introduces the carrier, descriptors, hidden origin, checked-error MIR,
status/out-slot ABI, propagation, catch dispatch, rethrow, failed-construction
cleanup, and interpreter/Cranelift/LLVM/PHP transport parity. Slice 1 does not
activate `StructuredExitKind::CheckedError` or emit partial backend artifacts.

For valid source requiring checked-error execution, `check`, `ast`, and `hir`
succeed. MIR generation and every execution or emission path stop once with the
structured Stage 29 Slice 2 boundary diagnostic. Semantic errors take priority.
Nonthrowing programs and unused conforming declarations remain executable.

## I/O Migration Reserved For Slice 3

Slice 3 migrates the accepted text and binary free-function I/O failures to
declared checked errors and supplies these canonical identities without short
aliases:

- `Doria\Std\Io\IoError`
- `Doria\Std\Io\InvalidUtf8Error`
- `Doria\Std\Io\IoOperation`
- `Doria\Std\Io\IoTarget`
- `Doria\Std\Io\IoErrorReason`
- `Doria\Std\Io\Utf8InputSource`

Before general namespaces execute, the compiler may recognize these exact
qualified identities. `IoError` carries message, operation, target, reason, and
optional system code. `InvalidUtf8Error` carries message, source, valid byte
count, and optional invalid byte count. The permanent ordinary stdout/stderr
closed-pipe status-0 rule remains neither panic nor throw.

## Unhandled `main` Outcome Reserved For Slice 3

An Error escaping `main` is `R1000`, kind `runtimeError`, status 70, and
termination `propagateWithCleanup`. It uses Decision 0109's one structured
diagnostic model and preserves concrete `errorType`, exact logical `message`,
first-throw `origin`, and no default propagation path. The human header is
`Error[R1000]: Unhandled <ConcreteType>` with `Where`, source preview, `Why`, and
`Process Exited With Status 70`. It is not panic wording, a stack trace, a Rustc
arrow, or a separate renderer.

Human and concise renderers indent multiline messages and escape control,
ANSI, and carriage-return sequences so message text cannot impersonate
diagnostic headings. JSON preserves the exact logical string. This is rendering
safety, not secret detection or automatic redaction; Error authors must not put
secrets in reportable messages. There is no automatic Help or cause chain.

## PHP And Foreign Boundaries

PHP is transport, not semantic authority. Slice 2 uses a private PHP Throwable
wrapper carrying the Doria object, concrete identity, and origin metadata.
Doria Error classes do not gain source-visible PHP exception inheritance and
catch matching never uses PHP reflection. An escaping Doria main error becomes
the same status-70 outcome; panic stays status 101. A future PHP-library export
converts an escaping checked error into a generated public PHP exception rather
than terminating the host process.

## Performance And Memory

Slice 1 adds semantic facts during existing declaration, resolution, ownership,
and control-flow traversal. It adds no runtime object, allocation, class-layout
change, ABI change, or separate whole-program error pass. Effect sets use
resolved semantic identities, not source strings. Controlled timing is
**Pending Available Runner** and non-blocking; the accepted performance standard
is unchanged.

## Implementation Slices

- **Slice 1 - Complete:** decision authority; grammar; AST/HIR; Error
  conformance; semantic effect sets; catch coverage/reachability;
  catch-or-declare; ownership; constructor/static/constant/finally checks;
  diagnostics; tooling; and one execution boundary.
- **Slice 2 - Next:** runtime representation, checked MIR, ABI, propagation,
  cleanup, catch dispatch, rethrow, and backend execution parity.
- **Slice 3 - Pending:** canonical I/O errors and signature migration, R1000,
  status-70 entry handling, installed-tooling, and website closure.

Stage 29 is in progress. Stage 30 is blocked until Stage 29 completes.

## Explicit Exclusions

This slice does not implement checked-error MIR, runtime descriptors, origin
storage, ABI changes, backend transport, runtime reporting, I/O migration,
expression-position throw, bare rethrow, union catches, inheritance/interface
catch matching, closure effects, namespaces, streams, PHP export conversion,
native unwinding, reflection, or a second diagnostic/cleanup model.

## Consequences

- The compiler can validate checked-error programs and expose their exact
  source-ordered contracts before execution exists.
- Backends cannot silently define or approximate checked-error semantics.
- Ownership, cleanup, and diagnostics remain one language-wide model.
- Stage 30 and Stage 35 must preserve effect-set substitution and extend the
  existing conformance/coverage abstractions rather than creating parallel ones.
- I/O migration and unhandled-error reporting remain visible, bounded work
  rather than accidental consequences of Slice 1.

## Affected Components

Compiler lexing, parsing, AST, HIR, symbols, type resolution, semantic analysis,
ownership, constructor initialization, diagnostics, backend boundary, language
server, editors, website documentation, playground metadata, and future runtime
work are affected. `doria-rt`, executable MIR, and existing class ABI are not
changed by Slice 1.

## Invalidated Elsewhere

- Decision 0035's direction-only implementation status and missing complete
  grammar/semantic contract are superseded by this record.
- Decision 0109's unresolved checked-error presentation is settled as R1000 in
  the shared diagnostic model.
- Decision 0116's reserved checked-error exit now has a bound payload, cleanup,
  and staged implementation contract; executable routing remains Slice 2.
- Decision 0117's ordinary namespace dependency is relaxed only for the exact
  compiler-known `Doria\Std\Io` identities before Stage 31. No short alias exists.
- Stage 30 closure work must carry checked-effect sets and the subset law.
- I/O documentation that says only "panic until Stage 29" must identify the
  Slice 3 migration rather than implying Slice 1 changes runtime behavior.
