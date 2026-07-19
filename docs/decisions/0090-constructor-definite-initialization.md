# Decision 0090: Constructor Definite Initialization

Status: Accepted

## Context

Decision 0083 temporarily limited native construction to property initializers,
promotion, and a narrow unconditional constructor assignment. That soundness
gate prevented uninitialized class storage from escaping before the compiler had
path-sensitive construction analysis, but it could not accept valid conditional
construction. Stage 21 supplies the missing shared semantic dataflow and must
remove the temporary restriction without making zero-filled allocation a source
language default.

Constructor initialization also interacts with decision 0081's cleanup rules
and decision 0089's enforced receiver borrows. A constructor operates on an
incomplete allocation, not an ordinary writable `$this`, and an abort-only panic
does not return an object or run partial-construction cleanup.

## Decision

### Initialization lattice and entry state

Every instance property is tracked independently as definitely uninitialized,
definitely initialized, or initialized on only some incoming paths. Instance
property initializers and promoted constructor parameters enter the ordinary
constructor body initialized, in decision 0083's explicit-then-promoted property
order. Other properties enter uninitialized.

A class with no declared constructor has an implicit no-argument construction
path. Every property on that path must already be initialized by a property
initializer or promotion-equivalent accepted mechanism.

### Control-flow merging and exits

Only reachable predecessors participate in a merge. A property is definitely
initialized after a conditional only when every normally continuing branch
initializes it. An omitted `else` preserves the path that skipped the body.
Nested and `else if` conditionals apply the same rule.

Every normal completion—fallthrough and explicit `return`—requires every
property to be definitely initialized. A branch terminated by `panic` produces
no object and contributes no normal predecessor. Statements after a terminating
panic or return are unreachable and cannot establish initialization.

### Readonly and writable properties

A readonly property may be initialized exactly once on every reachable path by
a direct simple `$this->property = value` assignment in its declaring
`__construct`; harmless grouping of that direct target is equivalent. An
initialized-on-some-paths state cannot be repaired with an unconditional
readonly assignment after the merge, because that assignment would initialize
the property twice on paths where it was already initialized.

A writable property must be initialized before observation or normal constructor
completion. Its first simple assignment establishes initialization; subsequent
ordinary writable assignment, compound assignment, and increment/decrement are
mutation and remain legal after initialization. An unconditional writable
assignment after a partial merge safely establishes definite initialization.

### Narrow construction access and repeatable bodies

Construction access applies only to the direct `$this` of the declaring
`__construct`. It does not make `$this` writable and does not permit compound
readonly initialization, nested paths, aliases, helper-mediated initialization,
or writable method calls. Static writes remain ordinary mutation.

Readonly initialization is rejected inside `while`, `for`, and `foreach`
bodies because those bodies may repeat. A writable assignment in a
possibly-zero-iteration loop does not establish post-loop initialization. Reads
inside loop bodies are still checked from the state that reaches the first
iteration; `break` and `continue` preserve sound reachable-flow facts.

### Incomplete `$this`

An uninitialized or maybe-initialized property cannot be read. Until every
property is initialized, `$this` cannot be passed to another callable, returned,
or used as the receiver of an ordinary instance method that could observe the
object. Initializing one property never establishes another.

### MIR and backends

Source diagnostics come from the compiler's shared control-flow graph and
forward-dataflow solver with source spans. Typed MIR represents every property
not preinitialized at allocation as a constructor-body obligation. Shared MIR validation independently recomputes the
initialization lattice over reachable MIR blocks and rejects early observation,
normal incomplete return, and same-path readonly duplication. It also retains
construction-order and property-table invariants.

The interpreter, Cranelift, and LLVM consume only that validated MIR. Allocation
may use raw storage internally, but zero initialization is not Doria semantics
and can never substitute for a proven property write. Abort-only panic retains
decision 0081's no-unwind, no-cleanup rule.

### Stage 19 gate

Decision 0083's temporary native-eligibility gate is removed by this analysis.
Conditional and nested constructor paths are accepted when the shared dataflow
proves them safe. Genuinely invalid construction is rejected semantically rather
than preserved under a renamed backend limitation.

## Alternatives considered

### Zero-fill every allocation

Rejected. It would expose backend representation as language semantics, invent
default values for types that have none, and could hide missing ownership
initialization.

### Require one top-level assignment per property

Rejected. It was the temporary decision-0083 gate and rejects valid conditional,
panic-terminated, and nested construction paths.

### Treat constructors as writable methods

Rejected by decision 0080. Initialization protocol access is narrower than an
exclusive borrow of an already-constructed object.

### Validate only during MIR lowering or in each backend

Rejected. User-facing errors need source paths and spans, while backend-local
analyses could disagree. MIR validation is a defensive invariant check, not a
second user semantics implementation.

### Unwind partially constructed objects on panic

Rejected. It conflicts with decisions 0040 and 0081's abort-only panic model.

## Consequences

Construction is path-sensitive and native-safe without runtime initialization
checks. Decision 0083's temporary gate is historical. Decision 0089's ordinary
borrow rules remain unchanged: construction access neither widens `$this` nor
emits dynamic guards.

The executable conditional-construction fixture is compared through the MIR
interpreter, Cranelift fast profile, and LLVM release profile, including exact
destructor output. New diagnostics require coordinated no-false-diagnostics
coverage in the separate `dorialang/doria-language-server` repository.

This decision does not settle or implement `Shared<T>`, `Weak<T>`, or
`SharedMut<T>`. Their public API remains a separately unauthored decision subject,
so the complete Stage 21 acceptance criterion cannot yet be claimed.

## Invalidated elsewhere

- `SPEC.md` text saying full constructor definite initialization is future or
  unsupported.
- The working-pipeline note saying constructor initialization is the only
  unfinished Stage 21 slice.
- Native parity prose describing only narrow direct constructor initialization.
- Decision 0083 wording that presents its temporary soundness gate as active.
- Decision 0089 consequences that still await the constructor slice before the
  gate can lift.
- MIR lowering assumptions that only one unconditional top-level assignment can
  satisfy a constructor-body property obligation.
