# Decision 0084: Statics and constant evaluation

**Status:** Accepted

## Context

Stage 20 adds ordinary instance and static method execution, class and top-level
constants, and static properties. These features need one identity model and one
bounded compile-time evaluator. They must not introduce hidden startup code,
backend-defined folding, owned global lifetime rules, or arbitrary compile-time
execution.

This record implements the statics-and-constant-evaluation subject listed in the
master plan. Its acceptance criterion remains exactly:

> AC: the SPEC §6 Parser class runs natively.

## Decision

### Member identity and access

An instance method is identified by its declaring class and method name and has
an explicit receiver. A static method is identified by its declaring class and
method name and has no receiver. Lifecycle methods keep their dedicated
compiler-invoked protocol and cannot be called as ordinary methods.

Ordinary methods receive readonly `$this`; `writable function` methods receive
writable `$this`. The compiler represents receiver mode as an extensible enum
with readonly and writable modes plus a reserved unsupported consuming mode.
Stage 20 adds no consuming-receiver syntax or behavior.

Static access is sigil-free for every member kind:

```doria
Message::age
Message::create()
self::MAX_DEPTH
self::age
self::create()
```

Declarations still carry `$`, as in `static writable int $age = 0`, but access
does not. This follows the same law as instance properties: `string $name`
becomes `$this->name`, never `$this->$name`. `Foo::$prop` is permanently invalid
and receives a machine-applicable fix that removes only `$`.

Each class has one member namespace across constants, static properties,
instance properties, static methods, and instance methods. One name denotes
data or an action, never both. A collision is rejected independently of source
order, and its diagnostic identifies both declaration kinds and both source
locations. Parentheses and sigils do not select between conflicting declarations.
Existing case-sensitivity and constant-casing rules remain unchanged.

`self` is compiler-reserved before ordinary name resolution. In member scope it
denotes the declaring class for constants, static properties, and static methods;
in a method return or other type position it denotes that same concrete class.
Stage 20 adds no inheritance covariance or late binding to `self`.

The grammar accepts generalized `parent::member()` in Stage 20 so editor and LSP
parsing follow the accepted language clock. Its inheritance lookup and dispatch
semantics land in Stage 34, so current checking produces a Stage 34 unsupported
diagnostic before MIR. Trait methods likewise preserve `self::member`
structurally, while trait composition remains a Stage 35 semantic feature.

`static::` is permanently invalid and receives a machine-applicable fix that
replaces only `static` with `self`. Doria has no late static binding: `static` is
the declaration modifier, `self` is the declaring-class qualifier, and statics
are not virtual. Future instance virtuality remains explicit through
`open`/`override`.

These spellings apply the project's "PHP's spelling is an artifact, not a
decision" test, already used to choose `read_line` over `readline` and `is` over
`instanceof`. PHP's static-property sigil compensates for PHP's separate member
namespaces, unenforced constant casing, and dynamic member names. Doria has one
statically resolved member namespace, enforced constant casing, and no dynamic
member names, so that ambiguity does not exist. Sigil-free static access and the
rejection of `static::` are deliberate divergences from PHP.

Members are externally accessible by default. `internal` instance methods,
static methods, properties, static properties, constructors, and class constants
are accessible only from methods or constructors of their declaring class.
`internal` controls API surface; `writable` controls mutation.

### Static properties

Static properties use the ordinary property declaration spelling with `static`:

```doria
static int $initial = 0;
static writable int $next = 1;
internal static string $label = "parser";
```

They are per-process data symbols, not per-object fields. Every Stage 20 static
property requires a const-evaluable initializer and a Copy type. Readonly statics
cannot be assigned after initialization; writable statics can be assigned through
their qualified name. Move-type and owned statics are rejected pending a future
decision informed by Stage 39 `Sendable`/`Shareable` work. Stage 20 chooses
neither process-exit destruction nor immortal owned statics.

Stage 20 emits no pre-main initialization, lazy initialization, once machinery,
or other runtime static initialization. An initializer outside the accepted
constant tier is rejected and points to the future runtime-initialized-statics
decision requirement.

Writing a writable static from `__construct` is ordinary static mutation. Stage
19 constructor init access applies only to `$this` and the instance under
construction; it grants no special static permission and imposes no additional
static restriction. Readonly statics remain readonly in constructors.

### Constants

Top-level and class constants use:

```doria
const DEFAULT_LIMIT = 25;
const int HARD_LIMIT = 100;

class Parser
{
    internal const MAX_DEPTH = DEFAULT_LIMIT * 4;
}
```

Constant names use `SCREAMING_SNAKE_CASE`. A declaration without an annotation
infers its type from its initializer in the same general manner as `let`. An
explicit annotation is accepted and its initializer must be assignable to that
type. Constants are immutable and need no runtime storage when their evaluated
value can be embedded at each use.

Constant identities are distinct by scope: a top-level constant is identified by
its top-level name; a class constant is identified by declaring class and name.
Stage 20 does not add namespaces, imports, aliases, or multi-file resolution.

### Constant evaluation

The Stage 20 evaluator is a typed, deterministic compiler service. Its allowlist
is:

- supported primitive literals;
- references to top-level and class constants;
- grouped expressions;
- typed unary numeric and boolean operations already accepted by Doria;
- typed arithmetic, bitwise, comparison, boolean, and string-concatenation
  operations already accepted by Doria;
- explicit numeric companion conversions already accepted by Doria when their
  operands are constant.

The evaluator forbids function and method calls, constructors, property reads,
mutable static reads, runtime values, allocation with observable identity, I/O,
environment/time/random access, mutation, loops, and panic as a compile-time
programming mechanism. Attributes may reuse evaluated values at Stage 32, but
attribute metadata, constant evaluation, and any future general compile-time
execution remain separate concepts.

Declaration order does not affect meaning. The compiler builds one dependency
graph for constants and static initializers, permits forward references, and
evaluates in dependency order. A dependency cycle is rejected with the chain and
useful participating source spans.

Constant operations use Doria widths and type rules. Integer overflow, invalid
shifts, division by zero, invalid conversions, and other invalid constant
operations are source diagnostics. They are never host-language behavior,
backend folding behavior, or runtime panic paths.

### Backend contract

Methods lower to shared typed MIR with stable compiler-generated identities,
explicit receiver operands, and receiver modes. Static properties lower to MIR
global data operations. Constants are evaluated before MIR and appear as typed
constant operands. MIR validation rejects malformed receiver, call, ownership,
static-access, and non-folded-constant shapes before the interpreter, Cranelift,
or LLVM consumes them.

Compiler-generated method and static symbols are private implementation details,
not a stable Doria ABI. Cranelift and LLVM implement these semantics; they do not
define them.

## Alternatives considered

- **Runtime static constructors:** rejected for Stage 20 because they introduce
  ordering, failure, and lifecycle semantics not yet decided.
- **Owned statics that never drop or drop at process exit:** both rejected until
  ownership, concurrency, and destruction order are designed together.
- **Backend constant folding:** rejected because it would make overflow and
  accepted expressions backend-dependent.
- **Arbitrary compile-time execution:** rejected because it introduces effects,
  termination, capability, and reproducibility questions outside this tier.
- **Dynamic method dispatch:** deferred to inheritance and interface stages;
  Stage 20 uses statically known concrete classes.
- **PHP `Foo::$prop` access:** rejected because the sigil solves ambiguities Doria
  deliberately does not have and would make declaration and access laws
  inconsistent.
- **PHP late static binding through `static::`:** rejected because it gives
  `static` two unrelated meanings, makes static identity virtual, and works
  against explicit virtuality and devirtualization.
- **Separate constant/property/method namespaces:** rejected because member
  meaning must not depend on punctuation at a use site.

## Consequences

- Stage 20 statics are immediately available without hidden initialization code.
- Forward references are predictable and cycles are diagnosed consistently.
- Constant overflow is caught before MIR and cannot vary by execution profile.
- Copy static state is useful now without prematurely defining owned-global or
  concurrent mutation semantics.
- Qualified static access is unambiguous without PHP's access-site `$`.
- `self` resolves to a concrete declaring class before MIR and backend ABI
  lowering.
- Accepted future `parent::` and trait `self::` syntax can power correct editor
  behavior without implementing Stage 34 or Stage 35 semantics early.
- The same method machinery can execute compiler-known concrete `Displayable`
  conversion without introducing interface dispatch early.

## Affected components

Lexer, parser, AST, HIR, semantic analysis, ownership analysis, MIR and
validation, interpreter, Cranelift, LLVM, PHP compatibility lowering, LSP,
editor grammars, examples, parity fixtures, tests, and language documentation.

## Invalidated elsewhere

- Documentation that describes general native methods, statics, class constants,
  or native concrete `Displayable` execution as deferred.
- Parser diagnostics that reject static properties or qualified constant access.
- Native lowering restrictions that collect only lifecycle class methods.
- Documentation or editor fixtures that teach `Foo::$prop`, `static::`, separate
  member namespaces, or ordinary-identifier resolution for `self`.
- Parser diagnostics that treat generalized `parent::member()` or trait-local
  `self::member` as malformed syntax rather than accepted-but-unsupported
  semantics.
