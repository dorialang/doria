# Decision 0107: Standalone lexical block statements

Status: Accepted

## Context

Doria already uses braced blocks for functions and structured control flow.
Ordinary code also needs a direct way to shorten the lifetime of locals,
deterministic cleanup obligations, and explicit shared-access objects without
inventing a helper function. The same braces provide that boundary without new
surface vocabulary.

## Decision

A bare braced block is a statement:

```doria
{
    let $value = createValue();
    useValue($value);
}
```

It is valid wherever an ordinary statement is valid inside an executable body,
may nest, produces no value, and takes no trailing semicolon. It creates a
lexical scope. Bindings declared inside are unavailable after the closing brace.

Still-owned values created in the block are destroyed in reverse acquisition
order when control leaves the block normally, including through `return`,
`break`, or `continue`. Access objects release their registration before their
strong ownership claim, as required by Decision 0106. Fatal panic remains
abort-only and runs no cleanup.

Lexical scope and borrow extent are separate concepts. Decision 0089 remains
authoritative: ordinary borrows end at their final required use and need not
remain live until the block closes. A block is nevertheless useful when a
programmer wants an explicit, visible lifetime boundary:

```doria
{
    let writable $access = $settings->acquireWritableAccess();
    $access->theme = "dark";
}

let $readonly = $settings->acquireReadonlyAccess();
```

No `scope` keyword is added. `scope` remains reserved for the separately planned
concurrency vocabulary. A standalone block is statement control flow, not a
value-returning expression; `when` remains Doria's value-returning branch form.

## Alternatives considered

### A `scope` statement

Rejected. Braces already express the required structure, while `scope` is
reserved for concurrency and would add a second spelling for the same lifetime
boundary.

### A helper function

Rejected. A helper changes call structure and API shape merely to express local
cleanup timing.

### Treating a block as an expression

Rejected. This decision supplies statement scope and deterministic cleanup only.
It does not introduce block values or compete with `when`.

## Consequences

- Parser, AST, HIR, semantic analysis, ownership checking, control-flow
  construction, MIR lowering, and the PHP compatibility emitter carry a distinct
  block-statement form.
- MIR lowering opens one cleanup scope and closes it on every structured exit.
- The durable native matrix covers nested cleanup and the shared-access lifetime
  use through the interpreter, Cranelift, and LLVM.
- Language-server diagnostics must accept a bare block without reporting an
  expected-expression parser error.

## Invalidated elsewhere

- Any parser or editor fixture that reports a bare executable block as an
  expression error.
- Any guidance that recommends a helper function solely to shorten a local or
  access-object lifetime.
- Any future proposal that spends `scope` on ordinary lexical blocks.
