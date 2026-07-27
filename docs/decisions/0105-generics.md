# Decision 0105: Generics (monomorphized functions and classes)

**Status:** Accepted. Formalizes D9 and plan §4.5, which set the direction but
left "the generics decision" explicitly unauthored. It **ratifies** the generic
functions/methods already shipped at Stage 24 and **settles the class-specific
rules** Stage 25 (generic classes) needs, so generic classes are designed against
a written contract rather than extending the compiler ad hoc.

## Context

§4.5 settled the generics *direction* (monomorphization, `T implements …`
constraints, inference without turbofish, a reserved value-parameter extension
point), and Stage 24 implemented generic functions and methods on it. The formal
record was overdue: Stage 25 adds user-defined generic *classes*, which raise
questions §4.5 did not answer — chiefly how an instantiation is classified
Copy-vs-move. This record writes generics down whole so those questions are
decided, not discovered.

## Decision

### 1. Compilation model: monomorphization (ratified)

Generics are **monomorphized at the MIR level**: each concrete instantiation
(`first<int>`, `Box<Token>`) generates specialized code, keyed by its type
arguments. This is the Rust model — zero-cost, no boxing, no per-call type
dispatch. Compile-time cost is accepted; the Cranelift dev profile keeps iteration
fast. There is **no runtime generic reflection** in v1.0.

### 2. Declaration, constraints, and inference (ratified)

- Type parameters are declared in angle brackets on functions, methods, and
  classes: `function first<T>(…)`, `class History<T>`.
- **Constraints** are spelled `T implements Interface`; multiple constraints are
  comma-separated inside the brackets (`<T implements A, B>`), matching
  `implements` lists — **not** `A + B`. A constraint may itself be generic
  (`<T implements Comparable<T>>`). Primitives satisfy constraints by the
  compiler-known conformance of decision 0096, with no boxing.
- Type arguments are **inferred at call/use sites from argument types**. There is
  **no turbofish** (`f::<int>()` is not Doria syntax); where inference cannot
  determine a parameter, bind through a typed declaration
  (`List<int> $xs = …`) so the annotation supplies it.
- Constraints are checked at the point of instantiation/use; an unsatisfied
  constraint is a compile error naming the missing interface.

### 3. Generic classes (the part Stage 25 needs)

- A class may declare type parameters (`class Box<T>`), use them in fields,
  parameters, returns, and method bodies, and is instantiated by supplying
  arguments (`Box<int>`, `Box<Token>`). Each instantiation is a **distinct
  monomorphized type** with its own specialized layout and code.
- **Copy-vs-move classification is compositional, computed per instantiation.** An
  instantiation is classified by the ordinary per-type rule with the type
  arguments substituted in: it is a **move type** if it has a destructor or any
  field that is a move type after substitution, and a **Copy type** only when
  every field is Copy after substitution. So a value/`Copy`-eligible generic class
  `Pair<A, B>` is Copy as `Pair<int, bool>` and a move type as `Pair<int, Token>`,
  while a class that owns a resource or declares `__destruct` is a move type at
  every instantiation regardless of `T`. Classification never depends on the
  *name* `T`, only on the concrete arguments — the same composition rule Doria
  already applies to non-generic types, so no new classification concept is
  introduced.
- Type-parameter **constraints on classes** (`class Sorted<T implements
  Comparable<T>>`) are checked at instantiation exactly as for functions; the body
  may rely only on the constrained surface of `T`.
- Generic **methods on generic classes** and **nested instantiations**
  (`List<Pair<int, string>>`) are permitted; each distinct instantiation
  monomorphizes independently.

### 4. Built-in collections share the machinery

`List<T>`, `Dictionary<K, V>`, and `Set<T>` (and `T[]`) are **real generic types**
using this same monomorphization machinery — not a separate compiler-internal
path. They are backed by runtime intrinsics today and move to Doria-source generic
implementations as self-hosting matures, without changing their surface.

### 5. Fences (v1.0 scope)

- **No variance.** Type parameters are **invariant**: `Box<Cat>` is not a
  `Box<Animal>`. Variance is not a v1.0 feature and is not reserved room that
  affects the surface.
- **No default type arguments** in v1.0 (`class Box<T = int>` is not accepted).
- **Value (non-type) parameters remain a reserved extension point, not
  implemented.** Per §4.5, generic metadata, arity checking, and monomorphization
  keying must not assume every argument is a *type*, so a future
  `Buffer<float32, 4096>`-shaped feature is an additive extension, not a redesign.
  v1.0 accepts type arguments only.
- **No runtime reflection** over generic type arguments.

## Alternatives considered

- **Boxed/type-erased generics (Java/`dyn` model).** Rejected — punches through
  Copy-vs-move classification and the borrow checker, forces heap boxing, and
  contradicts the zero-cost goal. Monomorphization is the committed model.
- **`T: A + B` constraint spelling (Rust).** Rejected in §4.5 — Doria reuses its
  own `implements` vocabulary and comma-separated lists so one spelling serves
  both `implements` clauses and constraints.
- **Turbofish `f::<int>()`.** Rejected — inference plus typed-declaration fallback
  covers the cases; a second explicit-argument syntax is surface without payoff.
- **Classification of a generic class fixed by declaration (always move).**
  Rejected — it would make `Pair<int, bool>` a move type even though it holds only
  Copy data, breaking the composition rule that makes small value aggregates cheap.
  Classification follows the substituted fields.
- **Variance / default type args in v1.0.** Rejected as scope — invariance is
  sound and simple; both can be added later without breaking existing code.

## Consequences

- Generic functions/methods (Stage 24, shipped) now rest on a written contract;
  Stage 25 implements generic classes against §3 here.
- The Copy-vs-move rule for instantiations is settled compositionally, so the
  borrow checker and native layout treat `Pair<int, bool>` and `Pair<int, Token>`
  correctly without a special generic-classification pass.
- Built-in collections and user generic classes are one mechanism, easing the
  self-hosting migration.
- Interfaces/traits as *generic constraints that users define and implement*, and
  user `Iterable<T>`/`Iterator<T>` conformance, are consumed here but their
  declaration/conformance machinery lands with Stage 35; this record only relies
  on constraints existing, not on user-defined interfaces being complete.

## Sequencing

Generic functions/methods are complete (Stage 24). Generic classes are Stage 25,
built on §3. Public generic **interfaces/traits** and user `Iterable`/`Iterator`
conformance are Stage 35. Thread-safe shared generics stay in Phase H. The
value-parameter extension point reopens only when a concrete need is scheduled.

## Affected components

Parser (type-parameter and constraint grammar — already accepted for functions;
extended to classes), semantic analysis (type-parameter scoping, constraint
checking, inference, per-instantiation Copy-vs-move classification), HIR/MIR and
monomorphization keying, shared MIR validation, native layout in the
interpreter/Cranelift/LLVM, diagnostics, the LSP (`dorialang/doria-language-server`),
SPEC's generics section, and plan §4.5 / the D9 row (the "unauthored" marker is
discharged here).

## Invalidated elsewhere

- Plan §4.5's "the generics decision, unauthored" marker and the D9 row's implicit
  deferral — the decision is authored here (0105).
- The current-pipeline note that "the generics decision remains unauthored and is a
  merge prerequisite for this generics work" — discharged; Stage 25 may proceed
  against this record.
- Any assumption that a generic class instantiation is classified move-or-Copy by
  declaration rather than by its substituted fields.
