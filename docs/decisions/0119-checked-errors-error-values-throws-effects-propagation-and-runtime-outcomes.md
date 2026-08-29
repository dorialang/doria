# Decision 0119: Checked Errors, Error Values, Throws Effects, Propagation, And Runtime Outcomes

- **Status:** Accepted
- **Accepted:** 2026-08-15
- **Entrypoint inference amendment accepted:** 2026-08-18
- **Date:** 2026-08-15
- **Implementation status:** Stage 29 Slices 1 through 3 complete; native
  collection property initializer corrective beat complete; inferred-main
  checked-effects corrective beat complete; Stage 29 complete
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
`Error` interface. Reusable callables declare every required checked error that
may escape with `throws`. Decision 0123 classifies exactly
`Doria\Std\Io\IoError` and `Doria\Std\Io\InvalidUtf8Error` as ambient: they do
not create a source catch-or-declare obligation, although explicit declarations
remain accepted and source-preserved. The selected program entrypoint infers its
exact escaping checked effects when the author omits that clause. `throw` transfers
an owned error into the propagation path. A caller must catch each possible
error or declare it; source calls to `main` see its inferred effective contract
and receive no exemption. Checked propagation performs required cleanup but
does not roll back completed mutations, output, or other side effects. Fatal
panic remains non-catchable, cleanup-free, and status 101.

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

## Runtime Representation

The erased Error carrier is two machine words: the concrete object pointer and
a static Error descriptor pointer. Each conforming object gains one private,
optional first-throw origin slot. Construction leaves it empty, the first throw
sets it, and rethrowing the same object preserves it. This slot is not a source
property, public object header, reflection facility, or second allocation.

Slice 2 implements this representation. One immutable descriptor per concrete
Error specialization carries concrete identity, type name, the validated
`message` projection, and concrete drop glue. Only Error-conforming objects gain
the hidden origin slot; ordinary class layouts remain unchanged.

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
or allow a checked error to escape. Ordinary functions, methods, constructors,
and generic specializations declare escaping effects explicitly. The accepted
top-level `main` shapes may omit `throws`; the compiler then infers the exact
uncovered effects from the body. A written `main throws` remains accepted and is
checked exactly like any other explicit contract, including E0631 for an
incomplete clause. A class or static method merely named `main` is ordinary and
does not receive entrypoint inference.

Each entry is either a concrete Error-conforming class or `Error`. Nullable
types, primitives, `mixed`, collections, typed arrays, enums, shared handles,
unknown types, non-Error classes, and general interfaces are rejected. Source
order is preserved for HIR, documentation, hovers, signatures, and diagnostics;
checking uses a normalized semantic set. Duplicate entries are rejected.
`throws Error, StorageError` is rejected because `Error` already covers the
concrete entry.

Every resolved callable has an effective semantic checked-effect set. For an
ordinary callable, that set comes from its written declaration. For a
clause-free selected entrypoint, it is the source-ordered uncovered set inferred
from the body. Source syntax and effective effects are separate facts: the AST
and HIR preserve whether `throws` was actually written and never synthesize a
clause or source span. Empty means nonthrowing. A nonthrowing or narrower
callable fits a position accepting a wider set; a wider set does not fit a
narrower one. Future overrides may preserve or narrow effects, never widen
them. Decision 0121 carries the same law into structural function and closure
types: function types write `throws`, semantic identity uses normalized effect
sets, and closure bodies infer their effects after local catch subtraction.

## Sources And Boundaries Of Effects

Effects arise from direct `throw`, resolved free/method/static/constructor calls,
instance property initializers, compiler-known built-ins, nested control flow,
catches, and nested `try`. Catches subtract only covered protected-body effects.
Catch bodies are checked independently; sibling catches do not handle errors
raised by another catch. Every uncovered error must be declared at an ordinary
callable boundary. At a clause-free selected entrypoint, the same analysis
becomes its effective escaping set instead. One diagnostic reports the complete
uncovered set where a declaration is required.

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

Decision 0123 supersedes the original restriction on throwing finalizers. A
checked Error may escape `finally`; it supersedes a pending return, `when`
result, break, continue, normal completion, or earlier checked Error after the
superseded owned payload is dropped exactly once. Same-try sibling catches do
not cover finalizer effects. A catch nested inside the finalizer or an outer
catch may handle them.

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

## Native ABI And MIR

Nonthrowing callables preserve their ABI. A throwing non-void callable uses its
ordinary arguments plus caller-owned success and Error out slots and returns a
u8 status: zero initializes success, one initializes Error. Throwing void omits
the success slot. Storage is function-entry or backend-equivalent fixed frame
storage; successful calls allocate no heap memory; propagation moves the same
Error carrier without cloning or reallocating its object.

MIR and every backend select this ABI from the effective semantic effect set,
not from source `throws` presence. Clause-free nonthrowing `main` therefore keeps
the ordinary entry ABI, while clause-free throwing `main` carries its exact
effects through HIR/MIR and uses the existing checked-result entry boundary.

Slice 2 implements the carrier, descriptors, hidden origin, explicit checked
calls, `StructuredExitKind::CheckedError`, propagation, exact/catch-all dispatch,
rethrow, failed-construction cleanup, and interpreter/Cranelift/LLVM/PHP
transport parity. B2901 remains a historical catalogue identity with no valid
source route. Slice 3 removes B2902 from valid programs: a checked Error escaping
any accepted `main` shape performs cleanup, reports R1000, and exits status 70.

## Canonical Checked I/O

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

`IoOperation` has `Open`, `Read`, `Write`, `Append`, and `Flush`. `IoTarget` has
`File(string $path)`, `StandardInput`, `StandardOutput`, and `StandardError`.
`IoErrorReason` has `NotFound`, `PermissionDenied`, `InvalidInput`,
`Interrupted`, `ResourceExhausted`, `Unsupported`, `Closed`, and `Other`.
`Utf8InputSource` has `File(string $path)` and `StandardInput`.

`IoError` exposes externally accessible readonly `message`, `operation`,
`target`, `reason`, and `?int systemCode`. `InvalidUtf8Error` exposes externally
accessible readonly `message`, `source`, `validByteCount`, and
`?int invalidByteCount`. Counts are bytes. The stable messages are Doria-owned:
`failed to <operation> <target>: <reason>` and `invalid UTF-8 in <source>`.
Localized host prose is not public API; the optional platform code remains a
typed fact.

The compiler-owned built-in table is authoritative for I/O effects. `read_line`
and `read_file` may produce both concrete I/O Error types. Text and binary
writes, `printf`, and `echo` may produce `IoError`; `sprintf` remains
nonthrowing. Decision 0123 makes exactly those two canonical Error identities
ambient at the source boundary while retaining their exact HIR/MIR effects,
checked transport, cleanup, catchability, and R1000 behavior. EOF is
successful `null`, blank input is `""`, P1206/P1302 remain allocation panics,
and P1401 through P1407 are historical identities with no ordinary valid route.

Canonical entrypoint source omits routine I/O boilerplate:

```doria
function main(): void
{
    echo "Hello, Doria!\n";
}
```

The compiler retains `Doria\Std\Io\IoError` as an effective ambient entrypoint
effect. Reusable I/O helpers do not need to write either ambient identity in a
`throws` clause. Explicit ambient declarations remain accepted and
source-preserved.

## Unhandled `main` Outcome

An Error escaping `main`, whether its contract was inferred or written, is
`R1000`, kind `runtimeError`, status 70, and
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

R1000 is catalogue kind `runtimeError`, severity `error`, status 70, and
termination `propagateWithCleanup`. It preserves the first throw or compiler-
known I/O effect origin and has no propagation path. Reporting is best effort:
an unavailable stderr still exits 70 silently after dropping the Error exactly
once. A successful `main(): int` returning 70 is not an R1000 outcome.

## PHP And Foreign Boundaries

PHP is transport, not semantic authority. Slice 2 uses a private PHP Throwable
wrapper carrying the Doria object, concrete identity, and origin metadata.
Doria Error classes do not gain source-visible PHP exception inheritance and
catch matching never uses PHP reflection. An escaping Doria main error becomes
the same status-70 outcome; panic stays status 101. A future PHP-library export
converts an escaping checked error into a generated public PHP exception rather
than terminating the host process.

## Performance And Memory

Effect sets use resolved semantic identities, not source strings. Entrypoint
inference reuses the existing operation-precise analysis and adds no runtime
effect table or general named-callable inference fixpoint. An erased
Error is exactly two machine words. Checked calls use fixed function-entry
success/Error scratch, one status test, and no heap allocation on success.
Propagation moves the carrier without cloning or reallocating the concrete
object, path capture, native unwinding, or a runtime cleanup stack. Nonthrowing
callables retain their existing ABI. Controlled timing is **Pending Available
Runner** and non-blocking; the accepted performance standard is unchanged.

## PR #137 Closure Audit

| Finding | Current Fix | Regression Test | Slice 2 Dependency | Risk | Disposition |
| --- | --- | --- | --- | --- | --- |
| Effect sites must remain operation-precise | The semantic effect-site map records the actual throwing expression, including calls nested in conditions and loop clauses. | `checked_effects_propagate_through_every_callable_form` plus the checked-error control-flow parity fixtures | Checked calls branch at the operation that can fail. | Later lowering could collapse an effect to a statement or function. | Preserved |
| Checked edges must select the semantic catch | One centralized exact/Error coverage model drives semantic CFG routing and MIR `ErrorSwitch` descriptor dispatch. | `catches_subtract_only_protected_effects_and_catch_bodies_are_independent`, exact/catch-all/nested parity fixtures | Runtime dispatch must agree with checked reachability. | Backend host-type matching could diverge. | Preserved |
| Checked edges must cross the right finalizers | Checked exits reuse Decision 0116 finalizer regions; the try-attached finalizer runs after its selected catch and before unmatched propagation. | `main_checked_error_finally.doria`, `main_checked_error_control_finalizers.doria`, malformed finalizer-plan validation | Slice 2 cleanup depends on the existing structured region graph. | A parallel cleanup path could reorder finalizers. | Preserved |
| Loop and condition sites must not disappear | Effect collection and lowering retain calls in branch, loop, `given`, and iteration positions. | semantic callable-form matrix and durable control-flow fixtures | Checked execution needs an edge at every resolved effect site. | Expression-only lowering could miss an exceptional edge. | Preserved |
| Structured fixes must preserve source | Duplicate/redundant `throws` fixes remain compiler-owned edits over precise spans and do not consume adjacent comments. | `throws_entries_are_error_types_unique_and_source_ordered` and diagnostic fix snapshots | Slice 2 consumes normalized effects after diagnostics. | Runtime work could bypass or duplicate semantic normalization. | Preserved |

## Implementation Slices

- **Slice 1 - Complete:** decision authority; grammar; AST/HIR; Error
  conformance; semantic effect sets; catch coverage/reachability;
  catch-or-declare; ownership; constructor/static/constant/finally checks;
  diagnostics; tooling; and one execution boundary.
- **Slice 2 - Complete:** runtime representation, checked MIR, ABI, propagation,
  cleanup, catch dispatch, rethrow, and backend execution parity.
- **Corrective beat - Complete:** concrete specialized instance-property and
  payload-enum storage types are interned before callable lowering. This closes
  the native collection-property initializer N1101 ordering defect without
  changing checked-error semantics or absorbing the then-pending owned-property
  transfer correction later accepted by Decision 0122.
- **Corrective beat - Complete:** a clause-free selected `main` infers its exact
  uncovered checked effects. Source omission remains visible in AST/HIR while
  the effective set drives source callers, MIR, ABI selection, all backends, and
  the existing R1000 boundary. Ordinary callables remain explicit.
- **Slice 3 - Complete:** canonical I/O errors and signature migration, R1000,
  status-70 entry handling, generalized private runtime-outcome transport, and
  interpreter/Cranelift/LLVM/PHP parity.

Stage 29 is complete. The pre-Stage-30 closure grammar slice is complete:
Decision 0120's base closure forms and function-type spelling now parse into
source-preserving AST nodes. Decision 0121 accepts complete closure effect and
function-type authority. Stages 30a through 30h are complete: valid closures
lower through closure-aware HIR and structural MIR, checked indirect calls reuse
this decision's Error model, and the debug interpreter, Cranelift, LLVM, and PHP
compatibility backend execute the supported surface. List algorithm calls carry
their callback's required structural effects and complete runtime profile,
including ambient I/O, through the same propagation and cleanup model. Stages
30 through 32 and the Decision 0123 corrective beat are complete. All three
Stage 33 slices and Phase F are complete under Decisions 0126 through 0128;
Stage 34 is next.

## Explicit Exclusions

Stage 29 does not implement general required named-callable effect inference,
expression-position throw, bare rethrow, union catches, inheritance/interface
catch matching, closure effects, namespaces, streams, PHP export conversion,
native unwinding, reflection, or a second diagnostic/cleanup model. It also
retains ownership of runtime reporting and I/O migration. Entrypoint inference
does not synthesize AST syntax, invent omitted-clause spans, or imply
`throws Error`.

## Consequences

- The compiler validates and executes handled checked-error programs while
  preserving their exact source-ordered contracts.
- Reusable callables declare required checked effects; exact canonical ambient
  I/O effects propagate automatically. The selected program entrypoint infers
  escaping checked effects when its source clause is absent.
- Backends cannot silently define or approximate checked-error semantics.
- Ownership, cleanup, and diagnostics remain one language-wide model.
- Stage 30 and Stage 35 must preserve effect-set substitution and extend the
  existing conformance/coverage abstractions rather than creating parallel ones.
- I/O migration and unhandled-error reporting use the same checked-effect,
  cleanup, descriptor, and diagnostic architecture as user errors.

## Affected Components

Compiler lexing, parsing, AST, HIR, symbols, type resolution, semantic analysis,
ownership, constructor initialization, diagnostics, class layout, executable
MIR, native/PHP backends, language server, editors, website documentation, and
playground metadata are affected. Ordinary nonthrowing function and class ABI
remain unchanged.

## Invalidated Elsewhere

- Decision 0035's direction-only implementation status and missing complete
  grammar/semantic contract are superseded by this record.
- Decision 0109's unresolved checked-error presentation is settled as R1000 in
  the shared diagnostic model.
- Decision 0116's reserved checked-error exit now has an executable owned Error
  payload and reuses its finalizer-region routing.
- Decision 0117's ordinary namespace dependency is relaxed only for the exact
  compiler-known `Doria\Std\Io` identities before Stage 31. No short alias exists.
- Decision 0121 requires every Stage 30 closure slice to preserve checked-effect
  sets and the subset law.
- Canonical examples and onboarding no longer write `throws` solely to allow
  checked work at `main`; explicit entrypoint contracts remain compatible.
- I/O documentation that says "panic until Stage 29" is obsolete: ordinary I/O
  failures are checked errors, while allocation failure and fatal panic remain
  separate.
- Decision 0122 implements owned-property move-in and writable replacement.
  E0472 remains reachable only for the separate move-out boundary.
- The pre-Stage-30 closure grammar slice is complete. Stage 30 closure semantics,
  Stage 31 namespaces, Stage 34 inheritance, Stage 35 interfaces, Stage 36a
  streams, and Stage 41 PHP-library conversion remain separate.
