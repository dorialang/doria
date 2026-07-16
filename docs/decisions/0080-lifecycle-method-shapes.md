# Decision 0080: Lifecycle Method Shapes

Status: Accepted

## Context

Doria's `__construct` and `__destruct` spellings are compiler-invoked lifecycle protocol points, not ordinary methods that happen to have magic names. The early method grammar nevertheless allowed `static` and `writable` to be attached to them. The PHP compatibility backend then emitted those modifiers directly, even though PHP rejects static constructors and destructors while loading the generated program.

The specification also previously said that an explicitly writable constructor followed ordinary writable-`$this` method rules. That wording answered how readonly-by-default properties are initialized with the wrong model: it treated construction as borrowing an existing instance instead of a protocol operating on a new instance.

This decision is an accepted amendment to previously specified pre-1.0 behavior. It is a breaking source change, but every rejected `writable` lifecycle declaration has a machine-applicable fix.

## Decision

### Fixed declaration allowlist

The complete legal source shapes are:

```doria
function __construct(/* typed parameters */)
internal function __construct(/* typed parameters */)

function __destruct()
internal function __destruct()
```

Either lifecycle method may omit a return annotation or declare `: void`. Constructors may declare typed parameters, including promoted parameters. Destructors declare exactly zero parameters.

`static` and `writable` are rejected on both lifecycle names. Each rejected modifier receives its own declaration-site diagnostic. Any other current or future method modifier is rejected unless a later accepted decision explicitly adds it to this allowlist.

### Protocol-granted construction access

The access `__construct` has to the instance under construction is granted by the construction protocol itself and is never declared. Rejecting `writable function __construct` removes a spelling, not an access rule.

Constructor access remains exactly the existing narrow model:

- direct simple assignment may initialize an uninitialized readonly property of the declaring class exactly once;
- property initializers and promoted parameters count as already initialized;
- compound assignment does not receive readonly init access;
- nested object paths do not receive readonly init access;
- writable methods cannot be called through `$this` merely because execution is in a constructor;
- readonly init access is unavailable inside repeatable bodies;
- writable properties retain their normal mutation rules.

This protocol access does not classify `$this` as writable. The compiler represents construction as its own checking context rather than using the ordinary writable-method flag. Stage 19 drop elaboration and Stage 21 definite initialization formalize construction and destruction further without changing these source-level rules.

### Diagnostics and fixes

A static constructor reports that `__construct` is invoked by `new` and cannot be `static`. A static destructor reports that `__destruct` is invoked automatically when an instance is destroyed and cannot be `static`.

Explicit `writable` on either lifecycle name is a declaration-site error. The fix removes only the `writable` token. Constructor guidance states that construction grants `__construct` its access to the new instance; destructor guidance states that destruction invokes `__destruct` through the lifecycle protocol.

### Invocation

Ordinary instance and static calls to either lifecycle name are rejected. Construction is expressed with `new Class(...)`, and destruction is compiler/runtime-invoked. The parent-first construction rule in the end-to-end plan reserves `parent::__construct(...)` as the parent-chain protocol form when inheritance is implemented. Stage 20 accepts generalized `parent::member()` grammar under the two-clocks rule, but parent lookup, constructor chaining, and direct parent-call semantics remain unsupported until Stage 34.

The PHP backend treats a lifecycle declaration with either rejected modifier reaching emission as a compiler-invariant violation. User source is rejected during semantic analysis before backend emission.

## Alternatives considered

### Option A: redundant but legal

Allow `writable` on lifecycle methods but give it no additional meaning.

Rejected. This creates two spellings for one meaning and still miscasts construction as an ordinary method borrowing an existing instance.

### Option C: require writable when mutating

Require `writable function __construct` whenever a constructor assigns properties.

Rejected. This adds mandatory ceremony to nearly every constructor and makes the same conceptual error: initialization of a new instance is not ordinary writable borrowing.

## Consequences

Lifecycle source shapes are finite and exhaustively testable. Invalid declarations fail before PHP emission, and editor/LSP clients can heal the pre-1.0 writable-spelling break in one edit. Constructor readonly initialization remains available without widening `$this`, while future ownership, destruction, inheritance, and definite-initialization work inherit an explicit protocol boundary rather than a backend-shaped magic-method exception.
