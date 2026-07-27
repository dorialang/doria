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
- **A generic class instantiation is a move type — always.** Classes are
  identity-bearing, owned heap move types (plan D3, decisions 0082 and 0087), and
  Doria has **no user-defined Copy aggregates** (the only Copy aggregates are the
  compiler-known `Doria\Std\Math` value types, Stage 47). Substituting type
  arguments never makes an instantiation Copy: `Box<int>` and `Box<Token>` are both
  move types, exactly as a non-generic class is. What monomorphization specializes
  per instantiation is not the *class's* classification but its **field handling
  and drop glue**, computed from the substituted field types — a field of type `T`
  is copied or moved-and-destructed according to `T`'s own Copy-vs-move
  classification, so `Box<int>`'s destructor drops nothing for its Copy `int` field
  while `Box<Token>`'s destructor drops its move `Token` field. That is §1's
  monomorphization applied to class fields, not a Copy-vs-move classification of the
  class. (A future user value/`struct` type, if ever introduced, is separate design
  work — 0087 prohibits it in v1.0.)
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
- **Compositional Copy classification (a generic class is Copy when its
  substituted fields are all Copy).** Rejected — it would create user-defined Copy
  aggregates, which plan D3, decision 0082, and decision 0087 prohibit: classes are
  identity-bearing move types and the only Copy aggregates are the compiler-known
  `Doria\Std\Math` value types. Making a generic class the sole exception would be
  internally inconsistent (generic vs non-generic classes classified by different
  rules). An instantiation is therefore always a move type; monomorphization still
  specializes each instantiation's field/drop glue from the substituted field types.
- **A user value/`struct` type so `Pair<int, bool>` could be Copy.** Rejected as
  out of scope — introducing user Copy aggregates reverses 0087 and is its own
  design effort, not part of Stage 25 generics.
- **Variance / default type args in v1.0.** Rejected as scope — invariance is
  sound and simple; both can be added later without breaking existing code.

## Consequences

- Generic functions/methods (Stage 24, shipped) now rest on a written contract;
  Stage 25 implements generic classes against §3 here.
- Generic class instantiations are move types like all classes (D3/0082/0087), so
  the borrow checker and native layout need no special generic-classification pass;
  monomorphization alone produces the correct per-instantiation field/drop glue
  (`Box<int>` drops nothing for its `int` field, `Box<Token>` drops its `Token`).
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
checking, inference, per-instantiation field/drop-glue specialization), HIR/MIR and
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
- Nothing in plan D3, decision 0082, or decision 0087 changes: generic class
  instantiations are move types like all classes. This record aligns with them; an
  earlier draft's claim that an instantiation could be Copy is corrected — there
  are no user-defined Copy aggregates in v1.0.
