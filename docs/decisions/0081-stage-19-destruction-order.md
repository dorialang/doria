# Decision 0081: Stage 19 Destruction Order

Status: Accepted

## Context

Stage 19 makes classes native move types and therefore makes destruction observable. A value may leave its original binding, a scope may have several structured exits, and a class may own other values. Without one source-level ordering rule, the interpreter and native backends could all be memory-safe while still disagreeing about visible destructor effects.

Doria's panic model is already abort-only under decision 0040. Destruction therefore needs to distinguish normal structured control flow from panic rather than accidentally introducing unwinding in one backend.

## Decision

### Owned locals, temporaries, and moves

Still-owned locals are destroyed in reverse order of initialization at every normal scope exit. A value that was never initialized or has been moved out is skipped. Moving a value transfers its cleanup obligation to the destination; it does not schedule a second destruction at the source.

Owned temporaries created within an expression live until the end of the enclosing statement. After the statement result has been bound, those still-owned temporaries are destroyed in reverse order of creation.

Assignment evaluates and acquires the replacement completely before destroying the destination's previous value. Self-move and other overlapping moves are rejected under decision 0083, so this order cannot invalidate the replacement while it is being acquired.

### Class destruction

When a class instance is destroyed, its user-defined `__destruct` body runs first. Still-owned properties are then destroyed in reverse of the class's total property order, and finally the class allocation is freed.

The total property order is defined by decision 0083: explicit properties in class-body order followed by promoted properties in constructor-parameter order. This supplies the complete declaration order that reverse property destruction requires.

Reverse property order is a deliberate divergence from Rust, which drops struct fields in forward order. It matches C++ and gives Doria one uniform rule: values are destroyed in reverse of construction, whether they are locals or properties.

### Control flow and panic

Fallthrough, `return`, `break`, and `continue` run the cleanup obligations for every scope they leave. Abort-only panic runs no cleanup and does not unwind. The compiler must not synthesize destructor calls or resource restoration on a panic edge.

## Alternatives considered

### Forward property destruction

Rejected. It would reproduce Rust's field/local asymmetry and make property destruction differ from the reverse-construction rule used everywhere else in Doria.

### Panic unwinding

Rejected. It conflicts with decision 0040, adds a second control-flow regime to every backend, and would turn Stage 19 into an exception-unwinding stage.

### Backend-local cleanup insertion

Rejected. Cleanup is observable language behavior. It must be elaborated once in typed MIR and consumed identically by the interpreter, Cranelift, and LLVM.

## Consequences

Drop elaboration must be path-sensitive and backend-independent. It tracks initialization and ownership state, emits explicit cleanup for normal structured exits, and emits none for panic.

An RAII guard restores its resource on every structured exit, including an error escaping `main` once checked errors exist, but not on an abort-only panic. In particular, the future `Console::rawMode` guard cannot promise terminal restoration on panic. A minimal panic hook could provide that behavior later, but Stage 19 neither implements nor designs toward it.

Decision 0082 defines how concrete class cleanup is represented natively. Decision 0083 defines the total property order and the move restrictions on which this destruction order depends.
