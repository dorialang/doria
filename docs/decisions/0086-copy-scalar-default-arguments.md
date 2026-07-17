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

Provenance note. This record was briefly authored as 0085 before renumbering to
0086. The number 0085 was already reserved for the namespace-model decision (plan
§12) and cross-referenced by record 0074 a day before this record existed, so the
0085 assignment was a collision rather than a durable citation. 0085 belongs to
the namespace model; this record is 0086.

## Decision

### Supported defaults

The native default-argument slice supports const-evaluable defaults for Copy
scalar parameters:

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

Const-evaluable `string` defaults are also supported for readonly parameters.
The accepted value may be a string literal or a const string expression from the
Stage 20 constant-evaluation tier, including an accessible class constant:

```doria
class Labels
{
    const string GREETING = "Hello";
}

function greet(string $message = Labels::GREETING): void
{
    echo $message;
}
```

At an omitted call position, the caller materializes the folded value exactly as
it materializes an explicit string-literal argument. The callee borrows that
caller-owned value through the existing string parameter ABI. An ordinary call
releases the caller-owned temporary after the call; constructor promotion instead
transfers that value into the promoted property, which releases it with the
object. This adds no new MIR node or backend primitive.

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

Defaults on `?string`, `writable string`, and `take string` parameters
remain deferred until their representation, mutation, and ownership obligations
are settled. They use distinct temporary diagnostics:

```text
default values for nullable string parameters are not yet supported
default values for `writable string` parameters are not yet supported
default values for `take string` parameters are not yet supported
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
- **Implement writable/take string and move defaults with readonly strings:**
  rejected because readonly string defaults reuse the established borrowed
  string-argument lifetime, while the other forms still require mutation or
  ownership decisions.

## Consequences

- `doriac check`, MIR lowering, the interpreter, Cranelift, and LLVM agree on the
  currently supported and deferred default forms.
- All call kinds use one argument-completion path.
- Explicit arguments retain ordinary left-to-right evaluation and override their
  corresponding defaults.
- Default values are folded once into compiler semantic data and carried through
  callable signatures; MIR lowering does not reimplement constant evaluation.
- Const string defaults extend value materialization without changing arity
  handling or the native callable ABI.

## Affected components

Constant evaluation, semantic analysis, MIR lowering, native parity fixtures,
tests, the language specification, status documentation, and the master plan.

## Invalidated elsewhere

- The native MIR diagnostic that rejected every parameter default.
- Exact-arity call lowering that ignored semantically optional trailing
  parameters.
- Status documentation that described Stage 20 as the highest completed native
  compiler slice.
- Tests that treated readonly string defaults as unsupported by semantic
  checking.
- The temporary E0498 message that rejected every string default without
  distinguishing readonly borrowing from writable or consuming parameters.
