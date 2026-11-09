# Decision 0139: Conditional Nominal Capabilities And Constrained Members

**Status:** Accepted; public checked execution implemented with focused macOS evidence; full acceptance and integration pending

## Approval And Boundary

On 2026-10-31 Andrew selected **"A: Conditional nominal capabilities
(Recommended)"** in the Stage 36a adapter-strategy question. This approves
general, explicitly declared conditional nominal conformance and constrained
member bodies, checked after generic substitution. Ordinary synchronous,
readiness-capable and pending-capable sources, owned and borrowed adapter forms,
and Decision 0110's performance contract all remain required.

Andrew subsequently approved **"Approve local-clause contract (Recommended)"**
on 2026-10-31. This accepts the local contextual `where (...)` spelling and the
detailed contract below. The Stage 36a compiler grammar, condition resolution,
positive nominal proofs, availability implication, guarded body/caller-condition
checks and active conformance views are implemented. Conditional member/witness
activation and guarded execution use the shared public path; the full acceptance
matrix remains open. The master plan owns
the sequence. This approval includes no installs, pin changes, Git
changes, publishing, or new public metadata schema keys. Existing source and
the three forwarding probes remain unchanged.

## Context

Decision 0134 gives classes an authored nominal conformance list. Trait
composition supplies implementations but never grants conformance. The
AST represented `ClassDecl.implements` as a list of type references at strategy
review. The grammar slice now gives each entry an explicit condition and
preserves the same local clause model on methods and computed properties.

A synchronous forwarding adapter over a ReadinessStream does not itself
conform to ReadinessStream. The checked forwarding probes accept the direct
ready source, reject the synchronous-only wrapper with E0535, and accept an
explicitly pending-capable wrapper. Checking these probes establishes the
model limitation; it does not establish adapter execution or performance.

Separate readiness and pending protocols remain essential. ReadinessStream
requires the borrowed `readinessSource` getter. PendingReadableStream adds
preparation and completion contracts and inherits ReadableStream and
ReadinessStream. Exposing the source getter alone cannot implement pending
transfers through a buffering or text adapter.

## Approved Direction

- A generic class may author an interface conformance conditioned on declared
  nominal constraints on its type parameters.
- Constrained member bodies provide the corresponding operations only under
  their authored constraints. This is a general compiler mechanism, shared by
  buffering, text and other generic domains.
- Substitution and checked nominal facts decide availability. Method-name
  coincidence, runtime capability probing and synchronous-body suspension do
  not establish conformance.
- A source satisfying the separate pending protocol retains full adapter
  preparation, completion, retirement, cancellation and buffer semantics.
  Conditional conformance does not authorize bypassing the adapter.

## Accepted Clause Spelling

The following grammar sketches belong to the Stage 36a implementation slice:

```text
Conformance entry:
    implements I where (T implements J)

Method:
    function name(...): R throws E where (T implements J) { ... }

Computed property:
    P $name where (T implements J) { borrowed get { ... } }

Conjunction, using the existing generic-bound vocabulary:
    where (T implements A, B, U implements C)
```

`where` is contextual in these clause positions. Existing identifiers
named `where` remain identifiers elsewhere; match guards continue
to use `if`. Parentheses delimit the condition from the surrounding implements
list, method body, effects and property hooks.

The subject of each bound is an in-scope class, trait or method type parameter;
the interface side is an ordinary resolved interface type, including its type
arguments. Every bound in a clause must hold. Clauses introduce no type
parameters, value expressions, negation, disjunction, runtime tests or external
implementation declarations. Existing generic parameter constraints retain
their spelling and meaning.

Placement is after the ordinary method return/effect contract and
before its body, or after the property name and before its hooks. A property
clause applies to its whole accessor contract. Existing accessor receiver,
borrow, effect and setter-input rules remain exact.

## Availability Rules

1. An unguarded declaration keeps its existing contract. A guarded declaration
   is available when its clause follows from the current checked nominal facts.
   An unsatisfied concrete clause provides no member, conformance witness or
   callable fallback.
2. A member body is checked using its owner's ordinary bounds plus its own
   clause. An unguarded generic caller must prove a callee's clause; the fact
   that some later concrete instantiation could satisfy it is insufficient.
   Final concrete substitution validates the same proof and member selection.
3. The compiler resolves names, type arity, access and declaration well-formedness
   even for a clause that later evaluates false. False clauses cannot hide
   malformed declarations. A body's capability-dependent operations are checked
   under that body's declared assumptions.
4. A guarded conformance is valid only when its active requirements have
   compatible, available implementations. The conformance's assumptions must
   imply each selected implementation's availability clause. An implementation
   may be unconditional when its conformance is conditional.
5. Guards on class and trait methods, static methods and computed properties
   are supported by the same mechanism. Trait substitution and flattening
   preserve guard identities and original source spans. Traits still grant no
   conformance themselves; conditional trait-use selection is not introduced.
6. Stored fields, promoted properties, constructors, destructors and class
   parents remain unconditional. A guarded property is computed: it has no
   backing slot, initializer or own-backing access. Capability availability
   cannot disable initialization or destruction of owned state. Interface
   parent lists and requirement sets remain fixed.
7. Existing nominal constraints apply without nullable unwrapping, interface
   shape inference, variance changes or primitive boxing. Erasing a source to
   a synchronous-only interface loses readiness proof unless its static
   contract preserves readiness. Pending capability is never inferred from
   synchronous plus readiness capability.

## Coherence And Inheritance

There remains one member namespace and one canonical declaration identity.
Two guarded implementations of the same source member name are not overloads;
conditions cannot select among duplicate bodies by declaration order.

Conditional conformance uses the existing canonical interface identities,
generic substitution, parent graph and requirement compatibility rules. It
cannot admit two simultaneously active, distinct specializations of one
interface. Diamonds retain one root requirement and one effective witness.
Source duplicates and existing unconditional redundant conformance diagnostics
remain diagnosed. Conditional inherited edges that extend availability require
canonical coalescing of their overlap, never two witnesses or a chosen winner.

An override binds its existing canonical root even when a particular parent
specialization lacks that member's availability. Under the child's class
bounds, the parent's availability must imply the override's availability.
An override may widen availability but cannot remove a method from a valid
base-typed call. All existing signature, visibility, receiver, ownership,
provenance and checked-effect substitution requirements still apply. Properties
retain their separate getter/setter contracts and backing-storage rules.

Canonical virtual root and parent-slot identities must remain stable across
specializations and child availability. Inactive reserved static slots are
permitted for layout stability; validated calls cannot enter an inactive slot.
No runtime predicate branch or fallback implementation is added to dispatch.

Conformance proof uses positive nominal facts and interface ancestors. A cycle
does not prove its own premise; the compiler must terminate and identify the
constraint chain. An independently justified nominal fact may seed a valid
proof. Proof and cache identity must not depend on discovery order or on which
other instantiations happened to compile first.

## Ownership, Execution And Cost

The shared semantic plan owns declaration conditions, substituted conditions,
proofs, active conformance and selected implementations. HIR, monomorphization,
ownership/effect checking and validated MIR consume those facts. Backends do
not infer capabilities from emitted method names or host reflection.

Borrowed getters retain their original source and lifetime. Returned closures,
owned and borrowed adapter forms, group-wide buffer loans and checked cleanup
keep their existing rules. A guard never grants ownership, extends a temporary
lifetime or makes a blocking method nonblocking.

Concrete specializations retain direct static calls and inlining eligibility.
Deliberate interface erasure retains its existing carrier and constant-slot
dispatch. No per-object capability dictionary, predicate object, wrapper,
per-operation allocation, extra buffer copy, helper thread or async startup
infrastructure is introduced. PHP emits specialization-specific members and
nominal interface facts from the shared plan, including negative cases.

## Compiler And Integration Work

- Lexer/parser/AST and source transforms: preserve clause subjects, interface
  references, delimiters and source spans without adding a second parser.
- Shared nominal analysis and trait composition: substitute guards, prove
  implication, select active members and construct one coherent witness plan.
- Generic registration, inference and incremental caches: include predicates
  and effective conformance; never reuse a proof for another specialization.
- Ownership/effects, HIR, MIR and independent MIR validation: preserve active
  target identity, retained sources, receiver permissions and compatible
  witnesses; reject forged unavailable calls and dispatch slots.
- Interpreter, Cranelift, LLVM and PHP: consume the checked plan with equivalent
  behavior. Inactive bodies and witnesses must not become callable fallbacks.
- Compiler IDE facts and doria-language-server: structured availability,
  diagnostics, hovers, completion, navigation, conservative rename, highlighting
  and accepted-syntax/no-false-diagnostics tests. Consumers do not parse prose.
- Public metadata/processor/Baton consumers: determine and coordinate any
  versioned schema extension before delivery. Existing schemas 1 through 3
  retain their exact contracts; this decision approves no new JSON keys.
- SPEC, agent guidance, website versioned guide, API reference and playground:
  explain approved clauses and the same static capability/ownership model.
  Benchmark manifests cover each capability tier and ownership form.

## Acceptance Contract

The full implementation must cover all of the following. Compiler grammar,
source transforms, incremental signatures, resolved declaration conditions,
the nominal query kernel, guarded authored-body/caller checks and public checked
execution are implemented. Trait predicates are normalized during
composition. Class/free callers use effective composed members and projected
authored loan/effect summaries. Authored traits check unambiguous imported
methods, getters, aliases and static generic calls through shared member views.
Ordinary and guarded checking share callable-contract refinement for concrete
return loans, retained-source graphs and callback effects. Contract inference
checks unused authored generic bodies independently of execution cleanup's
registered specializations; trait provenance uses the authored body identities.
Authored class and trait bodies now share the ordinary ownership context,
canonical composed taking contracts and refined retained-source storage types.
Methods, getters and unused traits diagnose repeated ownership transfer; unused
trait checks publish no executable cleanup or closure facts. Source storage
collection does not expand execution dependencies or admit builtin witnesses.
Dependency admission now retains declaration conditions and uses shared concrete
nominal proofs. Callable bodies own their dependencies; inactive or uninstantiated
bodies no longer seed executable callees. Class expansion and callable admission
share finite ancestry, admitting enclosing roots before their dependent types.
Generic declaration storage inventories preserve source-only disjoint-field
checks independently of executable registration. Stored fields and parents
remain unconditional.
Concrete ownership and returned-loan checks now select authored callable bodies
through the composition templates. Substitution preserves method-parameter
renaming, composing receivers, nested aliases and fixed trait arguments even
when the effective callable has no generic arguments. The final checking pass
collects guarded finite roots independently of source-storage inventories.
Dependency traversal now selects those authored spans and substitutions for
recorded generic callees and body-local classes. Constrained methods/getters
and core operations share the same substitution; effective roots preserve
admission and recursive-specialization checks.
Authored instance/static method targets now preserve generic relay dependencies.
Source return inference reads live contracts and checked bindings; concrete
queries substitute the composing receiver and select effective getter identities.
Trait ownership uses the authored method table, including imported static and
instance aliases. Borrowed/taking Copy, Move and region forwarding, named
arguments and nested aliases retain the existing return-elision contract.
A private checked-admission inventory now uses the canonical callable-instance
key shared with ownership facts. Shared native/PHP specialization omits inactive
nongeneric roots and requires exact owner/method bindings for direct generic
targets and erased implementation candidates. Missing preparation fails closed;
an empty checked inventory means no guarded instance was admitted. Declaration
inventory and unconditional constructors remain unchanged. Producer and consumer
regressions cover effective aliases, static methods and getter/setter identities;
consumer fixtures inject checked facts without enabling conditional source
execution. Finite and expanding trait recursion retain the shared ancestry rule.
One source-owned witness plan now fixes the implementation declaration,
concrete owner bindings, positional method parameters, canonical requirement
origins and exact-self return proof for every checked concrete class/interface
pair. Native/PHP materialization consumes those identities instead of searching
member names or rebuilding owner sets. Missing preparation, changed origins or
targets, and incomplete checked witnesses are internal diagnostics. This removes
the prior acceptance of a checked concrete conformance with no implementation.
Methods, getters, setters and core contracts share nongeneric admission checks;
generic implementations retain concrete call-specific admission.
Native MIR and PHP emission entry-point regressions reject inconsistent injected
facts with matching structured diagnostics. Admitted controls validate and run
in the MIR interpreter and emit PHP; this is not conditional source execution.
Ordinary and guarded conformances share this checked witness inventory. PHP
parameter-home, cell and field-transport coordination reuse its exact callable
edges. These are compiler facts, not runtime capability dictionaries.
Source checking now owns the checked generic-call graph consumed by shared
native/PHP materialization. Concrete callable and initializer scopes retain exact
declaration/binding keys; erased calls retain their requirement ABI shapes.
Virtual calls select overrides and their body dependencies under the actual
static receiver, while direct parent calls keep their implementation target.
Finite specialization ancestry remains source-owned. Missing graph preparation
or exact target declarations produces an internal diagnostic instead of backend
target reconstruction.
Source checking also owns canonical physical virtual families keyed by root,
concrete root owner and method type arguments, independently of executable body
admission. MIR reserves inactive/unmaterialized entries; explicit class upcasts
retain the narrower static receiver domain and original ownership/storage facts.
Shared MIR validation rejects forged families and calls into missing entries.
Cranelift and LLVM leave reserved table words empty without runtime capability
predicates or ancestor fallback bodies. Physical return contracts derive from
the concrete source root even when its body is inactive. Native virtual calls
use that return carrier and convert to the statically promised result; checked
calls use stack storage and convert only after success. The interpreter shares
its existing typed value projection for this result boundary. MIR rejects entry
returns that differ from the canonical contract and incompatible callable
returns. Ordinary and injected-fact fixtures
execute across the interpreter, both native profiles and PHP; these do not
enable conditional source execution. Checked source facts own direct `$this`
lifecycle dispatch, while calls on other objects retain normal virtual dispatch.
Native carrier adaptation includes class/interface views, collection and inline
Copy erasure and nullable value narrowing. Root and selected return/borrow
contracts survive inactive bodies. Lowering and validation share one typed
adapter constructor; validation checks its source implementation, effects and
executable body. LLVM preserves payload-enum expression/storage boundaries and
forwards exact borrow homes through nullable and ordinary class upcasts.
Nominal tests preserve inline target identity and witness-based presence.
Canonical virtual
families own physical checked transport independently of logical required-error
narrowing and root-body admission. Return adapters preserve implementation
logical effects; shared MIR checks reachable entry effect subsets before native
calls may rely on an infallible logical contract. Cranelift and LLVM consume the
same signature plan and existing stack layouts. Focused macOS execution covers
seven result shapes, Error subclass/subinterface narrowing, inactive roots,
exact declared-error parent calls and owned-argument cleanup. Source inference
now separates exact implementation ambient profiles from virtual descendant
profiles. Native direct bodies are lowered under their exact contracts; PHP
emits distinct implementation entries from the same checked dispatch facts.
Focused regressions cover exact parents, closed receivers, lifecycle calls,
catches, owner/method generic binding, traits and callbacks. Typed PHP caller
views now consume canonical families and witnesses, preserving covariant owned
and borrowed collection returns without host-signature inference. The same plan
covers computed getters, root-owned defaults, inherited witnesses and core
iteration/display entries, with and without native MIR. Focused return
regressions pass. Direct inline-to-interface tests now use checked nominal
conformance and independent nullable presence, including zero-valued payloads.
Semantic/MIR receiver substitution shares one rule and
preserves root/selected contracts through inactive root/intermediate bodies.
Five focused receiver regressions pass on macOS; four execution cases cover
interpreter/Cranelift/LLVM/PHP, including instance-field access and child/root
destruction. These earlier results predate public conditional source activation. The remaining
accepted-contract corrections under Decisions 0130/0134, broader regression
verification and fresh portable execution remain prerequisites before delivery.
The public conditional candidate now publishes checked execution, binding and
closure facts. Borrowed/taking Copy, Move and byte-region callback cases retain their
existing ownership and single-source return-elision contracts.
Actual-source conditional tests exercise the ordinary final semantic assembly
without injecting conformance/admission facts.
Attributes and constant diagnostics survive that handoff; errors leave witness
plans unprepared. Authored ownership retains symbolic storage views independently
of exported concrete layouts. Removed template-only concrete conformance
materialization: symbolic and concrete instances now share nominal closure and
requirement checking, so an edge first proven after substitution gains its checked
witnesses while inactive edges remain absent. The approved
coherence-obligation implementation is described below.
Completed actual-source verification reaches shared HIR lowering and callable
specialization. Direct and nested-trait methods/getters, generic method bindings
and generic helper dependencies retain their checked identities; inactive owner
specializations stay absent. Public HIR lowering now requires checked admissions
and uses the ordinary lowering worker; missing preparation is an internal compiler
error. The temporary E0776 rejection is removed in the implementation candidate.
The execution regression now uses
the ordinary standalone source finalizer and covers generic static calls plus
direct and erased-interface getters/setters through interpreter, Cranelift,
LLVM and PHP. Checked preparation validates authored bodies, retains their
refined callable and ownership contracts, and selects composed bodies from
checked admissions. The shared refinement pass establishes callback effects at
expanded call sites before ordinary finalization. Declaration lookup remains
complete; unavailable bodies do not enter the executable body view. Direct and
nested-trait execution passes focused macOS verification, including distinct
receiver/closure identities and fixed-false composition without executable
guarded bodies. Closure-free PHP variants run with and without MIR; captured
closures use MIR as required by the production PHP backend. Invalid authored
bodies remain errors even when their only composing receiver is inactive.
The public entry now invokes this preparation; its verification is separate from
the earlier private-entry results. Current-pipeline notes own exact identities,
bounded verification results and remaining coverage.
Ordinary and conditional paths share constructor initialization,
binding/accessor fact publication and post-ownership checks for borrowed getter
results, call-scoped requests and nonblocking work. Shared callable validation
prepares canonical interface requirement graphs before ownership and retained-source
refinement. Graphs preserve nominal Move classification, requirement return loans
and pending boundaries without establishing conformance or executable admission.
Guarded generic methods and composed aliases preserve owned Copy erasures and
borrowed erased Move results; temporary carrier loans cannot escape. Stored initializers and
partial cleanup belong to composing classes; call contexts preserve authored
bodies, effective receivers and generic bindings through aliases and closures.
Inherited partial cleanup uses physical property declarations with specialized
ancestor type bindings. Omitted lifecycle return annotations publish the same
checked `void` contract as explicit annotations, without phantom owned results
at parent-constructor handoffs. Abstract constrained read/write forwarders retain
their nominal preparation/completion obligations before concrete activation.
Known effects are checked without concrete callers. Type-dependent releases
retain executable-instance obligations. Declared nominal paths identify pending
phase requirements by canonical origins without creating an active conformance.
The borrowed-adapter storage contract remains a separate Decision 0110 appendix
review; conditional capabilities do not widen Decision 0134's retained-source
construction rules.
The public handoff is implemented. Full composition/specialization coherence,
specialized provenance/ownership, execution and coordinated integration still
require acceptance against this matrix:

- Synchronous-only, synchronous-plus-readiness, pending-read, pending-write and
  duplex sources; unrelated lookalike methods do not grant a capability.
- Concrete, abstract constrained and deliberately erased source types; true,
  false, inherited, redundant and unsatisfied predicate cases.
- Generic interfaces, multiple subjects, transitive interface bounds, aliases,
  nested adapters, generic methods and trait-substituted guarded members.
- Borrowed readiness getters and conditional computed properties alongside
  ordinary synchronous and guarded preparation/completion bodies.
- Generic callers without proof are rejected; callers with declared proof
  succeed independently of instantiation and discovery order.
- Duplicate names, conflicting interface specializations, circular proofs,
  missing implementations, stronger override guards and inactive forged MIR
  calls are rejected with structured source-identified diagnostics.
- Parent/child dispatch, widened override availability and conditional
  conformance diamonds preserve one canonical root and stable virtual slots.
- Owned and borrowed adapters, returned views/closures, error propagation,
  cancellation, retirement and cleanup preserve exact existing contracts.
- Parser recovery and source transforms preserve clauses; the implemented
  grammar reports accepted-but-unimplemented forms through Stage 36a semantic
  diagnostics rather than parser errors, including coordinated LSP coverage.
- Interpreter/Cranelift/LLVM/PHP differential fixtures and Linux/macOS/Windows
  evidence establish behavior. Structural tests prove absent inactive targets,
  direct concrete dispatch, unchanged erased carriers and allocation/copy bounds.
- Versioned metadata, Tooling, website and Benchmark integration are verified
  before installed refresh or any separately authorized publishing.

## Alternatives Considered

**Accepted: local `where (...)` clauses.** Each conformance/member owns its
condition. Pros: exact signature and diagnostic ownership, small parse boundary,
and no new class-body scope. Cons: related readiness operations repeat a clause.

**Alternative: constrained class-body blocks.** Group related member bodies under
one condition. Pros: less repetition across preparation/completion operations.
Cons: another declaration scope, rules for forbidden conditional storage and
lifecycle members, and separate syntax for conformance conditions are required.
This alternative does not restore the rejected separate-adapter-family strategy.

## Approval And Remaining Work

Andrew's A ruling and subsequent local-clause approval settle the capability
strategy, spelling, computed-only property boundary, positive constraint model
and inheritance/coherence contract. Grammar and shared semantic implementation
may proceed within Stage 36a. Exact public buffering/text adapter names remain
Decision 0110 appendix work; the acceptance and integration matrix above remains
required before delivery.

The compiler grammar checkpoint parses contextual clauses, nested generic
bounds and multiple subjects, preserves exact spans and alias references, and
includes conditions in declaration fingerprints. E0777 rejects conditional
storage, lifecycle declarations and fixed
interface requirements. The declaration prepass uses the shared contract/type
checker to resolve clause subjects to lexical parameter declarations and
interface arguments to canonical types. E0778 identifies a non-parameter
subject at its exact token. Malformed interfaces, type/value arguments,
duplicate members and shadowing method parameters receive their existing
declaration diagnostics. Class and trait `self` retain their owner context;
ordinary class conformance resolution shares that context propagation.
The shared nominal query kernel freezes caller assumptions, resolves and
substitutes declaration edges before checking their conditions, and keys
successful proofs by exact subject and interface specialization within that
caller context. Failed cyclic paths are not memoized; independent positive
evidence remains usable. Typed cycle chains retain source origins, including
expanding malformed declaration cycles. Declaration-cycle validation still
rejects those malformed graphs. Concrete generic constraints query conditioned
class edges independently of source order. Primitive constraint evidence remains
unboxed and cannot satisfy an interface-valued result contract.
Declaration checking now proves that conformance premises imply a selected
implementation's guard and that an inherited guard implies its override's
guard under child bounds. Required generic-method bounds supply caller premises;
implementation guards never prove themselves. Authored predicates are
substituted with owner bindings and canonical method-parameter identities,
including inherited same-name class/method parameters. Computed getter/setter
callables retain one property-condition identity, including borrowed getters.
Existing E0755/E0729 findings identify the exact clause and related contracts.
These checks cover methods and computed accessors available in the declaration
prepass. Shared generic signature binding now uses the original callable
template for contract parameters/results/effects, inherited call inference and
specialized member inventories. Owner substitutions and canonical method
parameters remain distinct even when their names collide after inheritance.
Previously accepted mismatched generic interface and override contracts now
receive E0755/E0729. Active specialized member/witness selection remains part
of the shared execution work.
The conditional declaration path now shares the normal callable-body phase.
Each method/accessor body receives its own resolved clause as scoped nominal
premises; those premises participate in constrained member lookup and core
value bounds and are removed when the callable ends. Calls capture their
caller's nominal environment and prove the callee's clause after argument and
typed-result inference completes. E0779 identifies an unproved condition at
the call/access with related clause and circular-proof origins. One computed
read-modify-write shares one condition finding; unresolved generic arguments
retain E0531. The generic-call checker no longer bypasses nominal bounds for
symbolic arguments: unproved ordinary bounds receive E0535 in the source body.
Display conversion, generic core bounds, Error admission and concrete backend
class facts now query the same exact-type proof kernel. The declaration-level
`ClassInfo` capability flags and name-only class traversal are removed; an
inactive specialization no longer inherits Displayable or Error admission.
Synthetic declarations must supply their nominal edge origins, including both
generated I/O errors and the assertion error. Guarded Error message reads use
scoped premises. Nullable symbolic receivers reach ordinary nominal lookup,
which still rejects members without a supporting bound.
Class conformance discovery caches declared paths and their guard obligations;
caller-local availability is never stored in that cache. A shared declaration
phase owns checked conformance types under owner bounds and the edge's clause,
including unused/inactive declarations. Exact source duplicates compare those
authored normalized types before concrete substitution; equal specialized paths
coalesce. Typed parent/conformance edges preserve their separate roles. Active
diamonds and conditional inherited overlap share one specialization. E0752
retains authored conformance and guard locations for conflicts; an invalid view
supplies no iteration element. The former order-dependent ancestor/direct-list
rejection is removed. Exact direct repeats and unconditional parent-class
redundancy remain errors. These views do not enable executable witnesses or
complete specialization coherence.
Formed class and compound types now defer active-conflict checking under frozen
caller premises until declaration facts are ready. Snapshots omit memoized
proofs and active recursion. Source formation, registered substitutions and
published inferred/contextual expression types share the checks; normalized
availability-call subjects and interface arguments preserve the original
caller's premises too. Incoherent predicate argument types lose their condition
fact. Identity-only lookups remain separate from executable registration.
Concrete and symbolic formations follow unconditional stored types and established active
conformance-target arguments, including inherited interfaces. One worklist covers
nested and mixed target/storage paths, with every active overlap argument checked
independently of the canonical interface entry. Stored types include nested
fields, parents, trait-provided storage and backed properties; computed properties
remain outside storage traversal. Pure substitution discovers no executable class
or witness. The shared recursive-specialization check preserves stable recursion
and finite collapse; E0539 uses one source-ordered cycle identity across target/
storage checks and metadata discovery. Duplicate reports for one component
coalesce; independent components retain separate findings. Owned-static rejection
E0486 is unchanged. Symbolic traversal uses the original caller's proved bounds
without assuming an unproved target guard. Expanding symbolic paths retain their
root formation obligation until concrete substitution; concrete expansion still
reports E0539. Full specialized member/witness closure remains incomplete.
These checks reject conflicts established by static facts. Missing symbolic
guard proof is not evidence of absence. On 2026-11-03 Andrew approved carrying
explicit coherence obligations through generic callers and resolving them after
substitution. Actual conflicting concrete conformances remain errors; potential
symbolic overlap alone does not reject a declaration. Callable-owned formation
obligations now survive generic call chains and authored trait substitution;
deferred availability checks retain their original calling declaration. The
existing specialization graph resolves those obligations with concrete nominal
facts, then checks active target and stored-type dependencies. Symbolic interface
arguments may become identical after substitution; independently conflicting
concrete paths are checked even when the canonical entry remains symbolic.
Final nominal checks preserve the class's declared bounds, and accessor
availability failures identify the authored condition. Symbolic stored/target
dependency traversal is implemented. Specialized member/witness execution uses
the shared checked public path; full acceptance remains incomplete.
Nominal type formation proves symbolic class/interface arguments, parent and
trait types, guarded signatures and conformance targets under their declaration
premises. Nested bound types substitute together; predicate types use the whole
conjunction, including inactive predicates. Malformed predicates are discarded.
Identity-only proof queries do not register executable instantiations, and
shared discovery ancestry preserves E0539 across metadata/return-borrow passes.
Trait predicate composition now uses resolved checked-use arguments rather than
re-resolving copied subject names. An obligation retains its authored parameter
identity separately from its substituted subject type. Nested trait uses,
method-generic renaming and composing-class `self` share simultaneous type
substitution. Effective aliases retain distinct identities, diamond members
coalesce, and computed getter/setter pairs share their property's condition.
Method-generic isolation also applies to nongeneric traits; it no longer depends
on having owner substitutions. Genuine authored shadowing retains E0530.
Malformed conditions retain unresolved callable markers and invalidate their
composition; false concrete predicates are never made unconditional. The
conditional analysis prepares these normalized predicates before class-member
lookup. Effective declarations serve callers while authored bodies retain their
lexical parameters and premises, including inactive trait specializations.
Composed callable templates preserve authored declarations and checked owner/
method bindings. Return loans and ambient effects project into effective
signatures; source refinement removes caught effects and propagates returned
closure sources independently of declaration order. E0531 renders authored
parameter names through the shared helper. Checked admissions select conditional
executable classes, targets and witnesses after source validation.
Imported member views reuse the same checked composition engine. Class and trait
owners remain distinct; a trait view uses its lexical `TraitSelf` receiver and
preserves checked substitutions, adaptations and callable summaries. Source
callers prove imported method/getter conditions, including static method
generics. Conflicting contributions remain unresolved; a trait-authored method
gains no class-authored precedence. Source-only view predicates and accessor
identities stay out of public declaration facts. Executable consumers use checked
effective identities instead of these source-only views.
Public metadata renders authored generic names through the existing name helper;
internal binding identities remain distinct and schemas 1 through 3 gain no keys.
The ordinary and conditional checkers now use one final callable-contract phase
for override provenance, property hierarchies, defaults, nominal conformance and
parent-constructor protocols. Rejection controls cover required/late parent
calls and receiver-bound returned closures, including generic computed getters.
Function-value signatures preserve symbolic ownership, including trait and
interface `self`, and distinguish borrowed results from owned results. Symbolic
`self` does not establish Copy ownership before substitution. Retained-source,
callback, pending-transfer and
writeback analysis now share one callable inventory including authored traits
and property hooks. This removes the omission of trait bodies from summary
refinement. Composed summaries reuse checked body templates and substitutions;
caller diagnostics wait for authored/composed fixed-point refinement. Storage
homes are recomputed under each composing receiver, while authored local
identities remain isolated. Recursive summary paths include effective-to-authored
edges. Imported opaque facts do not become known-body evidence.
The nested trait-alias taking-callback rejection now passes, alongside direct,
Copy, nested-owner and collection controls. Distinct composing receivers retain
their own physical property types. Recursive generic callback writeback rejects
incompatible replacement through a widened view and permits compatible writes.
The source-only summaries enable no unconditional member or executable witness.
The conditional prepass still needs complete specialized loan/callback/
retained-source refinement, ownership integration, composition/specialization
coherence and the specialized member/witness
pipeline before execution is enabled.
Resolved predicates and checking-only active views do not enable execution. Semantic checking and
lowering do not expose an unconditional callable or witness plan for a
conditional program. The shared
generic parser also preserves enclosing-list commas after nested `>>` closes.
Tooling source updates are prepared; its builds, pins and behavioral verification
remain held. No executable conditional conformance or adapter delivery is claimed.

## Invalidated Elsewhere

- AST, parser and source-transform assumptions that an implements entry and
  class/trait member carry no availability predicate.
- Unconditional-only class-conformance/member inventories, trait composition,
  generic caching, ownership/effect and dispatch facts.
- HIR/MIR witness and callable-target validation, backend emission and native
  dispatch structure assumptions once the accepted contract is implemented.
- Compiler IDE facts, Tooling syntax/signature/availability presentation and
  coordinated compiler pins; public metadata requires explicit version review.
- SPEC and agent guidance; website versioned guide, API reference and playground;
  adapter/performance cases in the separate Benchmark repository.
- The former unresolved A/B strategy question. Separate synchronous/readiness/
  pending adapter families are not the chosen mechanism.
