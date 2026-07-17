# Decision 0086: Copy-scalar default arguments

**Status:** Accepted

## Context

Doria already parses parameter defaults, checks their declared types, enforces
required-before-optional ordering, and accepts calls that omit trailing optional
arguments. Native lowering previously rejected every default after semantic
checking, which made `doriac check` and native compile/run disagree.

Stage 20's bounded constant evaluator and full native callable signatures make a
narrow first executable slice possible without defining move-value construction,
presence flags, multiple native ABIs, or backend-specific behavior.

## Decision

### Supported defaults

The current native slice supports const-evaluable defaults for Copy scalar
parameters:

- `int`, `int8`, `int16`, `int32`, and `int64`;
- `uint8`, `uint16`, `uint32`, and `uint64`;
- `float`, `float32`, and `float64`;
- `bool`.

Default expressions use the Stage 20 constant-evaluation tier. They may use its
accepted literals, operators, conversions, and references to accessible class or
top-level constants. Runtime calls, object construction, mutable state, I/O, and
other non-constant operations are rejected before MIR with:

```text
a default value must be a constant expression
```

A `writable` Copy-scalar parameter remains supported. `writable` controls
mutation and does not turn a Copy scalar into an owned move value. This includes
writable constructor-promoted scalar properties:

```doria
class Counter
{
    function __construct(writable int $value = 0)
    {
    }
}
```

### Caller-side splice

Every native callable retains one full-arity MIR and backend ABI. At each call,
the compiler lowers supplied positional arguments from left to right, then
appends the const-folded values for omitted trailing parameters as ordinary
typed MIR arguments.

The same shared splice applies to free functions, instance methods, static
methods, and constructors. Promoted constructor properties read from that same
completed argument vector. The MIR interpreter, Cranelift, and LLVM therefore
receive full-arity calls and require no default-argument-specific behavior.

Only trailing positional omission is included. Named arguments remain separate
future work.

### Deferred defaults

Const-literal `string` defaults are a committed near-future addition, not a
permanent rejection. Until their per-call runtime-string materialization is
implemented, semantic checking reports:

```text
default values for string parameters are not yet supported
```

Defaults for move types and `take` parameters remain deferred to ownership work
that defines construction and per-call destruction obligations. A writable move
parameter is covered by that deferral; writable Copy scalars are not. The current
diagnostic is:

```text
default values for move-type or `take` parameters are not yet supported
```

## Alternatives considered

- **Callee-side presence flags or prologues:** rejected because they add a second
  calling convention and backend work without benefit for constant defaults.
- **Backend-specific default insertion:** rejected because call semantics belong
  in shared checked lowering, not Cranelift or LLVM.
- **Reject all writable defaults:** rejected because writable Copy scalars have
  no ownership or destruction obligation; doing so would also break writable
  constructor promotion for scalar properties.
- **Implement string and move defaults together:** deferred because strings need
  runtime value materialization and move values need ownership accounting. Neither
  should weaken or complicate the scalar slice.

## Consequences

- `doriac check`, MIR lowering, the interpreter, Cranelift, and LLVM agree on the
  currently supported and deferred default forms.
- All call kinds use one argument-completion path.
- Explicit arguments retain ordinary left-to-right evaluation and override their
  corresponding defaults.
- Default values are folded once into compiler semantic data and carried through
  callable signatures; MIR lowering does not reimplement constant evaluation.
- Adding string defaults later extends value materialization without changing
  arity handling or native callable ABI.

## Affected components

Constant evaluation, semantic analysis, MIR lowering, native parity fixtures,
tests, the language specification, status documentation, and the master plan.

## Invalidated elsewhere

- The native MIR diagnostic that rejected every parameter default.
- Exact-arity call lowering that ignored semantically optional trailing
  parameters.
- Status documentation that described Stage 20 as the highest completed native
  compiler slice.
- Tests that treated string and runtime-expression defaults as currently accepted
  by semantic checking.
