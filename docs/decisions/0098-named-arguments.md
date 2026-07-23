# Decision 0098: Named arguments

Status: Accepted

## Context

Named arguments were left as "a separate future slice" across SPEC and deferred to by other records (0095 sends spread/variadic parameters here), but they were never designed or scheduled — while being load-bearing for consumers already in the plan: DDO's "named placeholders map onto named arguments" binding story, skipping a *middle* defaulted parameter (0086 notes a required parameter cannot follow an optional one until named arguments exist), and Stage 32 attribute arguments. This record designs named arguments and schedules them at **Stage 23a**, immediately after Stage 23 (collections/`Bytes`, which does not depend on them) and before Stage 24 (generic functions, after which call resolution grows more complex). They complete the caller-side binding machinery that default arguments (Stages 20a/20b) began.

## Decision

### Syntax and ordering
- Call-site spelling is `name: expression`, the PHP 8 form: `createUser(name: "Andrew", sendEmail: false)`.
- **Positional arguments may precede named arguments; they may not follow them.** Once a call goes named, every remaining argument is named. `f($a, name: $b)` is legal; `f(name: $b, $a)` is a compile error.
- **Named arguments may appear in any order.** Ordering by name is the whole point; the compiler binds by name, not position.
- **Named arguments may skip parameters that have defaults.** `f(a: 1, c: 3)` binding a defaulted middle `b` is legal and is the case 0086 could not express positionally.

### Binding and diagnostics
- A named argument binds to the parameter of that name. Diagnostics: **duplicate** (a parameter supplied twice — named twice, or once positionally and once by name) is an error; an **unknown** parameter name is an error; a **missing** required parameter (no default, not supplied) is an error. These reuse the existing argument-checking path.

### Evaluation order
- **Arguments evaluate in source (left-to-right) order, regardless of the parameter order they bind to.** `f(b: g(), a: h())` runs `g()` then `h()`, then binds the results to `b` and `a`. Binding is by name; evaluation is by writing order. This keeps side-effect order predictable and matches the one-expression evaluation rule the borrow model already uses.
- **Ownership and borrowing** are checked over that same source evaluation order: the one-writer-XOR-many-readers rule (record 0089, decision 0088's "within one expression" clause) applies across the whole call expression as written. Re-mapping arguments to parameter positions does not change the borrow-conflict analysis, which is by evaluation order.

### Parameter names are part of the callable's public API
- Once a callable can be invoked with named arguments, **its parameter names are part of its public interface**: renaming a parameter is a breaking change for named-argument callers, on the same footing as reordering or retyping parameters. This binds free functions, methods, and constructors. (A future edition/lint may surface parameter renames the way other breaking API changes are surfaced.)

### Parity across callable forms
- Named arguments work uniformly for **free functions, instance methods, static methods, and constructors** — the same four forms the default-argument caller-side splice already covers.

### Generic inference
- Named-argument binding is a **name-resolution step that runs before type inference**: the compiler maps each named argument to its parameter, then generic inference proceeds on the resulting parameter→argument assignment exactly as for a positional call. Inference sees the same mapping regardless of call syntax, so named arguments introduce no new inference rules.

### Override / interface compatibility (reserved for Stages 34–35)
- Because parameter names are API, an `override` method and an interface implementation **should** keep parameter names compatible with the parent/interface for named-argument callers to remain sound. Whether this is enforced or linted is settled with the inheritance (Stage 34) and interfaces/traits (Stage 35) decisions; this record reserves the requirement so those stages design against it rather than discovering it.

### Attributes reuse this syntax (Stage 32)
- Stage 32 attributes **do not invent a second named-argument system.** They reuse this call-site syntax and binding, adding only attribute-specific constant-evaluation restrictions (attribute arguments are const-evaluated, no runtime work). `#[Route(path: "/x", method: "GET")]` is the same `name: value` grammar.

### Variadics remain deferred
- 0095 deferred spread/variadic user parameters "to the named-arguments slice." **This record keeps variadics deferred as a separate slice, not part of Stage 23a.** Variadic collection (`...$rest`) is a distinct feature whose interaction with named arguments is intricate (a named argument must not be swept into a positional rest), and named arguments deliver their full value — middle-default skipping, self-documenting calls, DDO binding — without it. Variadics reopen when a concrete need is scheduled; they are not v1.0-blocking.

## Alternatives considered
- **Allow positional after named.** Rejected — it makes binding ambiguous and evaluation order confusing; PHP 8 forbids it too.
- **Bind by name means evaluate by name (parameter order).** Rejected — evaluating in parameter order rather than written order would make side-effect and borrow-conflict order depend on the callee's parameter list, which the caller cannot see; source order is the only order the caller can reason about.
- **Fold variadics into this slice.** Rejected — see "Variadics remain deferred"; bundling delays named arguments for a feature with harder open questions.
- **Do nothing until Stage 32 attributes force it.** Rejected — that is the "grammar/feature work is assigned, never implied" anti-pattern §0 warns against; attributes, DDO, and 0086 all already depend on it.

## Consequences
- Named arguments land at Stage 23a on the existing caller-side binding machinery, before generic call resolution (Stage 24) and well before attributes (Stage 32) and DDO need them.
- 0086's "a required parameter cannot follow an optional one until named arguments exist" is resolved: a middle default is now skippable by name.
- Parameter names join the callable API surface — a new breaking-change axis to track for overrides/interfaces (Stages 34–35) and library evolution.
- Attributes and DDO inherit a settled call-binding model instead of each inventing one.

## Affected components
Lexer/parser (accept `name:` at call sites, per §0 two-clocks), semantic analysis (name→parameter binding, duplicate/unknown/missing diagnostics), the shared argument-lowering/splice path (default insertion + named reordering), MIR argument ordering, the interpreter/Cranelift/LLVM backends, diagnostics, LSP coordination (`dorialang/doria-language-server`), SPEC argument sections, and the end-to-end plan (new Stage 23a; 0086/0095/DDO/Stage 32 cross-references).

## Invalidated elsewhere
- SPEC's "named arguments remain a separate future slice / separate future work" (multiple places) — now designed here and scheduled at Stage 23a.
- 0086's note that required-after-optional waits on named arguments — the enabling feature now exists.
- 0095's deferral target "the named-arguments slice" — is Stage 23a; variadics remain deferred *from* it.
- Any assumption that Stage 32 attributes will define their own named-argument syntax.
