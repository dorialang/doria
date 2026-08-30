# Doria Language Specification

This document describes the v0.1 direction for Doria.

Documentation role: current language specification. This file records Doria language rules and current implementation status where useful; it is not a parallel roadmap. Future implementation sequencing belongs to `docs/doria-end-to-end-plan.md`, and topic-level accepted decisions belong to `docs/decisions/`.

## 1. What Doria is

Doria is a statically checked compiled programming language designed for native executables, tooling, services, desktop software, games, and future self-hosting.

Doria's surface syntax is intentionally familiar to developers coming from PHP-like and C-like languages, but Doria is not PHP++, PHP does not define Doria's semantics, and generated PHP is not Doria's reference behavior.

Doria source files use the `.doria` extension and do not require `<?php` tags.

The compiler is `doriac`. The current bootstrap implementation is written in Rust. Doria's primary target is native machine code and standalone executables. A strategic goal is for `doriac` to become increasingly self-hosted in Doria over time.

Baton is Doria's external project, package, build, and application orchestration
tool. Its current executable implementation is a disposable PHP bootstrap that
ships with its own private runtime; the production implementation is scheduled
to be written in Doria. Baton does not define Doria semantics and is not part of
the compiler pipeline.

The compiler architecture is backend-independent:

```text
Doria source
-> lexer
-> parser
-> AST
-> semantic analysis
-> type checker
-> readonly/writable checker
-> Doria IR
-> backend
```

Backends may include:

- Native backend, the primary product target.
- Debug/interpreter backend, for validating Doria semantics without relying on another language runtime.
- PHP backend, as an optional compatibility, migration, debugging, and inspection target.
- WebAssembly backend.

The PHP backend must not shape the parser, AST, semantic model, Doria IR, native-oriented IR, runtime model, memory model, object model, error model, or standard library.

### 1.1 Design authority and correctness policy

Doria semantics are defined by Doria's specification, accepted design decisions, and explicit language-designer decisions. Backend output is an implementation of those semantics, not the authority for those semantics.

The project follows these rules:

```text
Correctness over speed.
Native-first over convenient transpilation.
Safety over quick demos.
Explicit design decisions over silent implementation assumptions.
```

If an implementation task reaches a design fork not answered by this specification or `docs/decisions/`, the implementation must stop and ask for a decision. It should report the question, options, tradeoffs, affected files, and a recommendation. It must not silently choose behavior because PHP, Rust, JavaScript, C, C++, or a backend library makes that behavior easy.

Temporary backend limitations may produce unsupported-feature diagnostics. They must not redefine Doria.

Compiler diagnostics use the shared human-first model defined by the compiler
diagnostic authority. Every diagnostic has a stable code, severity, kind, Title
Case title, one or more source-identified labels, and optional explanation,
notes, help, applicability-classified multi-edit fixes, causal identity, and
documentation metadata. Human and concise CLI presentations write to stderr;
schema-version-1 structured JSON writes to stdout and contains no ANSI.
Language-server and website consumers use that structure rather than parsing
terminal prose. Backend/external/internal failures preserve full developer
details but do not expose raw tool or Rust panic output by default. Runtime
panic is a `RuntimePanic` outcome in the same compiler-owned model. Built-in
panics use stable `P` codes, source-identified labels, `Where`, `Why`, and a
Doria-only `Call Path`; panic remains abort-only, performs no cleanup or
destruction, and exits with status 101.

## 2. What Doria is not

Doria is not PHP++ and is not required to parse every valid PHP program.

Doria syntax is familiar to developers coming from PHP-like and C-like languages, but it is not PHP-compatible at the parser level.

Valid PHP should be easy to migrate to Doria, but Doria-specific syntax does not need to run directly in PHP.

Doria does not use `public`, `protected`, or `private` as member visibility modifiers. Class members are externally accessible by default, and `internal` marks implementation details.

The current compiler implementation lowers the accepted native subset through validated typed MIR. The debug interpreter, default Cranelift fast profile, and `--release` LLVM profile consume that same MIR, and the durable executable parity manifest compares exact stdin-driven stdout bytes, stderr bytes, process status, declared file side effects, and class lifetime behavior across all three paths. The supported subset includes top-level free functions; int/void `main` with either no parameters or one readonly borrowed `List<string>` argument; structured control flow and recursion; fixed-width numerics and bool; const-evaluable defaults for Copy scalars and readonly strings; immutable UTF-8 strings; expression interpolation; general nullable scalar, string, concrete-class, and `mixed` values with flow narrowing, `??`, `?->`, and exact `is`; checked formatting; UTF-8 line/file I/O; exact stdout/stderr; fatal panic; native classes, methods, statics, constants, complete `internal` checking, deterministic ownership, and compile-time borrowing; Stage 23 collections, typed arrays, `Bytes`, binary I/O, and boxed `mixed`; Stage 24 generic functions and methods; Stage 25 generic classes; and both Stage 25a shared-ownership families. The readonly `SharedReference<T>` / `WeakReference<T>` family supports class payloads. The writable family supports class, generic-class, typed-array, `List<T>`, `Dictionary<K, V>`, `Set<T>`, and `Bytes` payloads through `WritableSharedReference<T>` / `WritableWeakReference<T>` and owned readonly/writable access objects. It includes nullable weak acquisition, lazy nullable operations, generic/property/collection storage, deterministic strong/weak/access release, and one per-allocation many-readonly-XOR-one-writable access state. `SharedReference<T>::referencedValue` is a readonly, allocation-free, refcount-neutral collision projection available only on that wrapper. Native strings are private non-atomic refcounted buffers and are Copy at the source level. Native classes, collections, `Bytes`, `mixed` boxes, and shared-reference handles are pointer-sized move values whose payload layout is compiler-known. Shared-reference control blocks are separate from unchanged payload layouts; the final strong release destroys the payload once, while weak references may keep only the control block alive. Shared-access conflicts use P1501 with a typed `conflictReason` that distinguishes all three incompatible access states. `main(): int` crosses the accepted `0..125` process boundary and `main(): void` maps normal completion to status `0`. Release optimization does not change observable semantics. `doria-rt` owns process entry, class/collection/byte-buffer/mixed-box/shared-control allocation and free, runtime strings, raw standard-device I/O, line discipline, text/binary file I/O, exact output, and the private source-aware runtime-outcome transport and standalone `Call Path` projection. Scalar/string writable-shared payload access, shared handles through `mixed`, and interface-typed values remain unsupported. The former Stage 7-10 native smoke module remains retired.

Stage 26 completes the compiler-known non-closure collection family with
ascending `SortedDictionary`/`SortedSet`, a min-first `PriorityQueue`, and a
front/back `Deque`. These types share the validated MIR and native runtime
contract used by the existing collections.

Doria is not a Rust language. Rust is the current bootstrap implementation language for `doriac`, not the permanent identity of the compiler.

## 3. MVP syntax

The MVP supports:

- Top-level statements.
- `let` declarations.
- Explicit typed declarations.
- Functions.
- Classes.
- Properties.
- Methods.
- Constructor parameters and constructor property promotion.
- `echo`, `return`, `foreach`, `for`, `if` / `else if` / `else`, and `while`.
- Assignments.
- Function calls, method calls, property access, object construction, and literals.
- Collection literals using bracket syntax.

The MVP also supports named arguments using `name: expression`, per decision 0098; §9 records their binding, ordering, and evaluation rules.

Planned near-term syntax includes:

- Attribute lists using `#[...]`, which reuse the named-argument syntax above.
- Richer property initializer expressions, including object construction.

Implemented control-flow also includes:

- base `do ... while`;
- `given` attached to `if`, `when`, and `while`;
- `when` as a value-returning conditional form.
- `match` as a pattern/value selection construct.

Control-flow `finally` executes on `if`, `when`, `while`, and `do ... while`,
including the corresponding `given` forms. It runs once on normal or structured
exit, from inner to outer when nested. Same-loop `continue` does not run the
loop finalizer, and fatal panic runs no finalizer. `finally` on `for`, `foreach`,
`match`, or a bare block is not Doria syntax.

See Decision 0116 for the current control-flow authority.

### Source organization and compiler directives

The accepted namespace, import, include, and directive direction is recorded in `docs/decisions/0028-namespaces-use-include-and-directives.md`. Decision 0117 defines compile-time autoloading, hybrid strict source layout, package compilation graphs, and the Baton-to-compiler build-plan boundary. Decision 0118 defines the package manifest, dependencies, lockfile, workspace, processor, cache, and offline model. Decision 0126 fixes schema-2 local/scoped package identity, binary/library targets, target selection, deterministic source discovery, and target-scoped plans and receipts. Decision 0127 fixes the implemented normal path/Git dependency resolver, SemVer validation, one-version graph, strict deterministic lockfile, dependency commands, global Git cache, offline policy, multi-package plans, and receipt identities. Decision 0128 fixes and implements canonical dependency source descriptors, workspaces, development graphs, tests, processors, generated-source ownership, graph inspection, and project inventory. Decision 0124 fixes implementation ownership without changing those semantics: Stage 33 validates the Baton product contract in the disposable PHP UX bootstrap, and a mandatory Pre-Stage-45 transition parity-ports it to the clean Doria-native `dorialang/baton` repository before the unsuffixed `2026.03.1` release. Stage 31 implements namespace and import syntax, the edition-2026 prelude, canonical package-owned global identities, compiler-facing edition/package/source context, versioned build plans, complete multi-file indexing, compile-time include resolution, package visibility, and strict source layout. All three Stage 33 slices and Phase F are complete. Native Testing Foundation Slice 1 is complete, Slice 2 is next, and Stage 34 single class inheritance remains blocked until the foundation completes.

Namespaces define logical symbol ownership and declaration scope. They are part of semantic name resolution, not source inclusion, package resolution, build orchestration, or runtime loading.

Accepted conceptual syntax:

```doria
namespace App\Services;

class UserService
{
}
```

Nested namespace paths such as `namespace App\Domain\Users;` use the backslash separator. Any name containing `\` is absolute; unqualified names resolve through imports, the current namespace, and the edition prelude.

`use` statements import names from namespaces at namespace/file-scope only. `use` is semantic name resolution and aliasing. It is not textual inclusion, PHP runtime include, package dependency resolution, trait composition, or code execution. `use` is not valid inside class, trait, interface, function, or method bodies.

Accepted conceptual syntax:

```doria
use App\Models\User;
use App\Security\Permission;
use App\Repositories\PostRepository as Posts;
```

`use` may import fully qualified symbols and may alias symbols. Duplicate or conflicting imports are diagnosed. `use` does not load files or packages; Baton resolves packages and discovers source through compile-time `autoload` mappings.

Class-body and trait-body trait composition uses `uses`, not namespace import `use`.

`include` is compile-time source inclusion with required include-once behavior. It is lower-level source composition, not the normal import mechanism. If an included file cannot be found, compilation fails. If the same canonical file is included more than once, it is included once. Include resolution must be deterministic, include diagnostics must preserve source file and span information, and included source participates in the same compiler pipeline as normal Doria source.

Accepted conceptual syntax:

```doria
include "src/generated/routes.doria";
```

Only string-literal same-package source paths are accepted. Paths resolve relative to the including file, use include-once semantics, and cannot escape the package root. A file discovered by both autoload and include enters the compilation once. Computed paths, remote includes, and cross-package traversal are rejected:

```doria
include $path;                         // rejected direction
include getPath();                     // rejected direction
include "https://example.com/file.doria"; // rejected direction
```

Doria does not add separate PHP-style `require`, `require_once`, or `include_once` forms. Doria `include` already means required include-once source inclusion.

### Package source organization

The public manifest term `autoload` maps namespace prefixes to package-relative
source directories. Baton discovers matching `.doria` files during the build
and gives a deterministic source inventory to `doriac`; a Doria executable does
not load source files at runtime. Every active main, development, generated,
dependency, and explicitly included file is checked.

Source layout uses hybrid strictness: namespace directory segments match
exactly, an externally accessible type matches its filename, and a file has one
primary externally accessible type. Related `internal` helpers may share that
file, while free functions and constants may use descriptive bundle files.
Generated bundles and selected binary entry files have the bounded exceptions
defined by Decision 0117.

Publishable package identities use lowercase `vendor/package` and remain
independent of Doria namespace identity. `internal` is accessible throughout
one package, including package-owned development sources, but not from another
package or merely another member of the same workspace. Only direct declared
dependencies are source-visible.

Manifest schema 2 accepts scoped packages, which are publishable by default,
and unscoped local packages only with explicit `publishable = false`. Baton maps
a local manifest name to the reserved compiler identity `local/<name>` without
deriving it from a namespace or path. The single-binary shorthand remains
package-level `kind = "binary"` plus `entry`; explicit targets use
`[targets.library]` and `[[targets.binary]]`. Ambiguous commands select with
`--library` or `--binary <name>`; Doria has no generic Baton `--target` option or
manifest `default-target`. A library build checks the complete plan and records
`artifact: null` until a public native library artifact is separately defined.

Normal dependencies use `[dependencies]` with exactly one path or Git source.
The authored dependency key must match the dependency manifest's package name.
Path dependencies remain live and may point to sibling packages. Git
dependencies use exactly one `rev`, `tag`, or `branch` selector, while
`Baton.lock` records the resolved exact commit. Optional SemVer constraints
validate the selected manifest version; Baton does not search Git tags as a
package registry.

One compiler package identity resolves to one source and version in a complete
graph. Source substitution, incompatible constraints, and package cycles are
errors with all contributing dependency chains. Existing valid locks are used
exactly, including when a branch or tag has moved. Offline mode permits live path
packages and cached exact Git content but performs no network operation. Baton
emits the resolved package graph through the compiler-owned build-plan schema;
`doriac` does not parse `Baton.toml` or `Baton.lock`.

In package mode, only a selected binary entry file may contain top-level
executable statements. Library, autoloaded non-entry, development, generated,
dependency, and included files are declaration-only. Source discovery order has
no runtime initialization meaning.

`break` exits the nearest enclosing loop. PHP-style numeric break levels such as `break 2;` are not accepted by the namespace/directive decision. Labeled break may be evaluated later if needed.

`continue` jumps to the next iteration of the nearest enclosing loop. PHP-style numeric continue levels such as `continue 2;` are not accepted by the namespace/directive decision. Labeled continue may be evaluated later if needed.

A bare braced block is a statement and creates a lexical scope:

```doria
{
    let $value = createValue();
    useValue($value);
}
```

It may nest, produces no value, and requires no trailing semicolon. Bindings
declared inside are unavailable after the closing brace. Still-owned values drop
in reverse acquisition order on fallthrough, `return`, `break`, and `continue`;
fatal panic remains abort-only and runs no cleanup. Borrow extent remains
non-lexical under Decision 0089, so an ordinary borrow may end at its final use
before the block closes. Doria does not use a `scope` keyword for lexical blocks.

Traditional `for` loops are accepted for explicit counter/index iteration:

```doria
for (let writable $i = 0; $i < 10; $i++) {
    echo $i;
}
```

`foreach` is preferred for collections and ranges. Integer ranges use `..` for inclusive ranges and `..<` for exclusive-end ranges:

```doria
foreach (0..10 as $i) {
    echo $i;
}

foreach (0..<10 as $i) {
    echo $i;
}
```

`0..10` produces `0` through `10`. `0..<10` produces `0` through `9`. Range endpoints must be `int` expressions. The variable after `as` is a readonly loop-local binding for each iteration and does not leak outside the `foreach` body.

Standalone `++` and `--` mutation statements require a declared writable `int` target:

```doria
$i++;
++$i;
$i--;
--$i;
```

Value-producing `++` / `--` expression semantics are future work.

`declare` is a structured compiler/source directive. It is not a macro system and not textual substitution. Exact grammar and allowed declaration keys require future decisions. Unknown declare keys should be rejected when `declare` is implemented. Possible future uses include warning policy, unsafe/FFI boundary policy, backend/profile constraints, platform configuration, optimization intent, feature gates, and compile-time diagnostics.

`goto` is evaluation-only and is not accepted for implementation yet. If it is ever accepted, it should be constrained so it cannot jump into deeper scopes, bypass visible initialization, bypass cleanup or `finally` obligations, cross guarded resource regions, or cross future ownership/borrow-checking boundaries.

Doria should not adopt a C/C++ textual macro preprocessor by default. `#define` and `#undef` textual macro substitution are not accepted. Future conditional compilation and compile-time diagnostics should use structured compiler semantics rather than arbitrary token substitution. Doria source should remain parseable, typed, and semantically checked by `doriac`.

## 4. Declaration rules

Variables must be declared before use.

```doria
let $name = "Andrew";
let writable $count = 0;

string $city = "Lusaka";
writable int $score = 0;
```

Two or more local bindings may share one declaration initializer:

```doria
let $left, $right = 0;
let writable $red, $green, $blue = 0;
int $minimum, $maximum = 0;
writable int $x, $y = 0;
```

The initializer is evaluated exactly once before any group name enters scope,
then the independent bindings initialize from left to right. One inferred or
explicit type and one mutability mode apply to the complete group. Copy scalars
and immutable strings are eligible; string handles are retained without copying
their contents. Move values are rejected rather than implicitly cloned or
shared. An explicitly typed nullable move group may begin as literal `null`, for
example `?Token $left, $right = null;`; an untyped grouped `null` is rejected.
Still-live locals clean up in reverse order. A group has no runtime object and
does not permit a trailing comma or per-binding types, mutability, or
initializers. Grouping is local-only; it is not destructuring and does not apply
to properties, parameters, promotions, statics, constants, `foreach` bindings,
or closure captures. Decision 0111 records the complete contract.

Bare assignment never declares a variable:

```doria
$name = "Andrew"; // error
```

## 5. Readonly and writable rules

Everything is readonly unless explicitly marked `writable`.

```doria
let $x = 5;
$x = 10; // error

let writable $y = 5;
$y = 10; // ok
```

Explicit typed declarations follow the same rule:

```doria
int $x = 5;
$x = 10; // error

writable int $y = 5;
$y = 10; // ok
```

Properties are readonly by default:

```doria
class Person
{
    string $id;
    writable string $name;
}
```

To assign to a property, both the object path and the property must be writable, unless a constructor is directly initializing an uninitialized property through constructor init access.

Constructor init access is narrower than writable `$this`. Inside `__construct`, a direct simple assignment such as `$this->id = $id;` may initialize an uninitialized property of the declaring class exactly once on each reachable path. Property initializers and constructor-promoted parameters count as already initialized. Direct readonly init access does not permit compound assignment, nested readonly initialization, calls to writable methods through direct `$this`, or initialization from repeatable bodies such as `foreach`.

Direct constructor `$this` is a construction root. It may traverse through a definitely initialized, non-null, writable property and then perform ordinary mutation on the owned child. Every further intermediate must satisfy the same ordinary initialization and writable-path rules. Thus `$this->window->title = "Lucy";`, `$this->state->counter += 1;`, nested writable method calls, collection mutation, and indexed mutation are valid when the path grants writable access; a readonly or maybe-uninitialized intermediate remains an error. This traversal does not make `$this` generally writable and does not give constructor initialization privilege to a nested readonly property.

Writable properties must be initialized before observation or normal constructor completion; later ordinary writable mutation remains legal. Branches merge only normally continuing paths, panic-terminated paths produce no object, and every property must be definitely initialized at each fallthrough or explicit-return completion. An incomplete `$this` cannot be exposed to another call or ordinary instance method.

The access `__construct` has to the instance under construction is granted by the construction protocol itself and is never declared. Explicit `writable` on `__construct` or `__destruct` is a compile error with a machine-applicable fix that removes `writable`. This removes a spelling, not an access rule: it does not make `$this` writable and does not widen constructor init access beyond the narrow rules above plus normal mutation of writable properties. Lifecycle methods are compiler-invoked protocol points, not ordinary methods. Stages 19 and 21 formalize construction natively through drop elaboration and definite initialization without changing these source-level rules.

Function parameters are readonly by default and become writable only with `writable`. A `take` parameter gives the callee ownership of a class move value; the call site remains unmarked, and the caller cannot use that value afterward. `take` and `writable` are mutually exclusive. Copy-type arguments retain their ordinary Copy behavior.

Readonly controls mutation, not ownership transfer: a readonly class binding may be moved from. Assigning a new owner to that moved-from binding is mutation and therefore requires `writable`. An independently owned value may initialize an owning instance property or replace an initialized writable owning property. Replacement evaluates and acquires the new value before destroying the old value; checked failure leaves the old property unchanged. Borrowed values, self-moves, and overlapping transfers are rejected. Moving a value out of a property remains separate because Doria has no accepted property-hole or take-and-replace operation.

Every parameter in Doria source has an explicit type. This applies to all function-like parameter lists: free functions, methods, constructors, anonymous functions, arrow functions, interface requirements, trait requirements, property hook setters, and future callback-style declarations. Doria does not infer omitted parameter types in any context.

Valid:

```doria
let $double = fn(int $x) => $x * 2;

let $format = function (int $score): string {
    return "score: {$score}";
};
```

Omitting a parameter type is a syntax or semantic error even when the surrounding expression makes the intended type obvious.

Closure captures are explicit for both arrow functions and anonymous block
functions. When a closure references a local binding from an enclosing lexical
scope, the binding must appear in a `with` clause. This applies to Copy and Move
bindings, readonly and writable bindings, outer parameters, and enclosing
pattern, catch, and `given` bindings. There is no automatic arrow capture.

```doria
let $minimum = 70;

let $arrow = fn(int $score) with ($minimum) =>
    $score >= $minimum;

let $block = function (int $score): bool with ($minimum) {
    return $score >= $minimum;
};
```

`with ($value)` is a readonly borrow, `with (writable $value)` is an exclusive
writable borrow, and `with (take $value)` transfers ownership into the closure.
`use` is not a closure-capture alias. A closure that uses no enclosing local
omits `with`; an empty `with ()` is neither required nor recommended. Changing
between arrow and block bodies preserves the capture list and its ownership
modes. Own parameters and locals, top-level functions, constants, statics, type
names, and enum cases do not require capture.

### Accepted Stage 30 closure semantics

Decision 0121 accepts structural Move-only function values. Their structural
types preserve parameter ownership, invocation mode, return type, nullability,
monomorphized identities, and checked effects:

```doria
function(int): int
function(writable Counter): void
function(take Payload): string
function writable(int): int
function once(): Payload
function(string): Record throws ParseError, StorageError
```

Default invocation is readonly and repeatable. `writable` invocation requires
exclusive access to the function value. `once` invocation consumes it.
`function take()` is not Doria: `take` remains the ownership-transfer mode for a
value, parameter, or capture. Closure bodies infer invocation mode and checked
effects; closure expressions do not write their own `throws` clause.

Any expression with a callable semantic type may be invoked in postfix position.
The callee evaluates once, arguments evaluate left to right, and structural
calls are positional with no named or default arguments. Named-function,
static-method, bound-method, and constructor references are deferred; a wrapper
closure adapts an existing callable.

Lexical capture is validated by stable binding identity. A binding used by a
nested closure must pass explicitly through every intermediate closure.
Captures acquire at closure creation in written order and owned captures are
destroyed in reverse logical order. Borrow-capturing closures cannot escape
beyond their owners; no-capture and take-only environments may escape.

Method-local closures capture the receiver explicitly. `with ($this)` borrows a
readonly receiver and `with (writable $this)` borrows it exclusively. Doria
rejects `with (take $this)`. PHP's implicit receiver capture does not define this
behavior.

The runtime model is a two-word descriptor/environment carrier. No-capture
closures have a null environment and allocate no environment. Descriptors are
lean compiler-private records, not reflective function signatures. Logical
capture order remains source order; physical environment fields may reorder
privately while preserving logical acquisition and destruction.

Stage 30g adds `map`, Copy-only preserving `filter`, and writable-accumulator
`reduce` to `List<T>` only. Their callbacks are nonescaping, process elements in
insertion order, and propagate the callback's checked effects. A
readonly-repeatable callback is borrowed readonly. A writable-repeatable callback
is borrowed exclusively and requires writable access to the function value;
invocation mode never silently widens a readonly function-value borrow. Other
collection families receive no Stage 30 higher-order algorithms. These three
methods lower through explicit algorithm HIR and one validated MIR traversal CFG
for the debug interpreter, Cranelift, and LLVM. PHP compatibility emits the same
ordered behavior through compiler-generated loops rather than host array
higher-order functions. Checked failure destroys a partial result or reduce
accumulator exactly once and leaves the source list unchanged.

### Current compiler support

The compiler resolves readonly/writable/once structural function types into
canonical semantic identities, including parameter ownership, return-borrow
provenance, and normalized checked effects. It checks closure bodies, explicit
capture lists, `$this` capture, inferred invocation modes and effects, nullable
callable narrowing, callable-value calls, and callable properties. Source type
grouping remains transparent and does not create a tuple type.

Valid closures now lower through explicit HIR and MIR closure nodes and execute
through the debug interpreter, Cranelift fast profile, LLVM release profile,
and the PHP compatibility backend for its supported value surface.
MIR structural function types preserve parameter
ownership, invocation mode, checked effects, return type, and return-borrow
provenance. Function values use the logical two-word descriptor/environment
carrier: descriptors are static, no-capture closures have no environment, and a
capturing closure acquires fields in source order and releases owned fields in
reverse logical order. Checked indirect calls reuse Decision 0119's ordinary
checked-error and cleanup model.

Native function values use a two-word descriptor/environment carrier. Static
descriptors identify generated entry and drop functions. No-capture closures
allocate no environment, proven nonescaping environments use stack storage, and
escaping environments use one heap allocation without reference counting.
Capture acquisition follows source order and remaining owned captures drop in
reverse logical order. Checked indirect calls reuse the ordinary checked-error
ABI and cleanup model.

PHP compatibility uses compiler-generated function carriers, explicit capture
environments, and stable BindingId-backed places derived from semantic and
validated MIR authority. PHP automatic capture, host `callable`, capture
`use (...)`, and PHP references do not define Doria closure or borrowing
semantics. No-capture closures create no PHP environment. `E0641` is historical
and remains reserved; no accepted Stage 30 route emits it. Unrelated PHP
compatibility boundaries remain independent.

```text
Stage 30a Callable Grammar Completion - Complete
Stage 30b Semantic Function Types And Captures - Complete
Stage 30c Ownership, Lifetime, And Escape - Complete
Stage 30d Closure HIR/MIR And Interpreter Oracle - Complete
Stage 30e Native Execution - Complete
Stage 30f PHP Compatibility - Complete
Stage 30g List Algorithms - Complete
Stage 30h Cross-Repository Closure - Complete
Stage 30 - Complete
E0641 - Historical And Reserved
```

Methods receive readonly `$this` by default. A method that mutates `$this` must be declared with `writable function`.

## 6. Member access

Doria class members are accessible by default. Use `internal` only for implementation details that should not be accessed from outside the declaring class. Doria does not use visibility modifiers as boilerplate.

`writable` and `internal` solve different problems:

```text
writable controls mutation.
internal controls API surface.
```

Valid member declarations:

```doria
class Parser
{
    const DEFAULT_SLUG = "parser";

    string $name;
    internal string $slug;
    internal writable int $position = 0;

    function __construct(internal string $givenName, internal string $givenSlug): void
    {
        $this->name = $givenName;
        $this->slug = $givenSlug;
    }

    static function create(string $name): Parser
    {
        return new Parser($name, Parser::DEFAULT_SLUG);
    }

    writable function parse(): Ast
    {
        $this->advance();
        return $this->parseProgram();
    }

    internal function parseProgram(): Ast
    {
        return new Ast();
    }

    internal writable function advance(): void
    {
        $this->position = $this->position + 1;
    }
}
```

Internal members are accessible only from methods and constructors of the declaring class. They are not accessible from top-level code, free functions, or other classes. Protected is permanently excluded from Doria; inheritance does not add a third access tier.

Instance and static methods have distinct identities. An ordinary method has a readonly `$this`; `writable function` has a writable `$this` and requires a writable receiver path. A static method has no `$this` and is called with `ClassName::method()`. `__construct` and `__destruct` remain compiler-invoked lifecycle methods and cannot be called as ordinary instance or static methods.

Static properties are per-process state and use qualified access:

```doria
class Counter
{
    static int $initial = 0;
    static writable int $value = Counter::initial;
}

Counter::value = 42;
```

Static properties are readonly unless marked `writable`. Their initializers must be accepted by the bounded constant evaluator, and the current implementation admits Copy types only. There is no runtime, lazy, or once static initialization. Owned statics and their lifetime, destruction, and concurrency rules require future accepted design work.

Static member access is always sigil-free:

```doria
Message::age
Message::create()
self::MAX_DEPTH
self::age
self::create()
```

Declarations carry `$`; accesses do not. This is the same law used by instance
properties: `string $name` is accessed as `$this->name`, not `$this->$name`.
`Foo::$prop` is permanently rejected with a fix that removes `$`. PHP needs that
sigil to distinguish across separate member namespaces and dynamic names; Doria
has neither ambiguity.

Each class has one member namespace across constants, static properties,
instance properties, static methods, and instance methods. A name represents
data or an action, never both, and collisions are errors regardless of source
order.

`self` is reserved and denotes the declaring class. It is valid as a static
qualifier and as a type, including a return type such as:

```doria
function withName(string $name): self
{
    return new Message($name);
}
```

`parent::member()` is accepted grammar, but parent lookup and dispatch are Stage
34 semantics and are currently diagnosed as unsupported before Doria IR.
Trait-local `self::member` also parses under the accepted-language clock, while
trait composition remains Stage 35. `static::` is permanently rejected with a
fix to `self::`; Doria has no late static binding.

Writing a writable static inside `__construct` is ordinary static mutation.
Constructor init access applies only to `$this` and the instance under
construction; it neither grants nor removes permission for class statics.

Top-level and class constants use `SCREAMING_SNAKE_CASE` and may infer their type or declare it explicitly:

```doria
const DEFAULT_LIMIT = 25;
const int HARD_LIMIT = 100;

class ParserLimits
{
    const MAX_DEPTH = DEFAULT_LIMIT * 4;
}
```

Constants are immutable and evaluated before MIR. Declaration order does not affect meaning: forward references are resolved through a dependency graph, while cycles report the dependency chain. The bounded evaluator accepts supported primitive literals, other constants, grouping, typed arithmetic/bitwise/comparison/boolean/string operations, and accepted explicit numeric conversions. Overflow and invalid constant operations are compile-time errors. Function or method calls, constructors, runtime/static-property reads from constants, mutation, I/O, environment access, allocation with observable identity, loops, and arbitrary compile-time execution are rejected.

Property hooks are planned later for validation and computed properties, but they are not part of the current implementation.

### API surface naming

Doria APIs should make intent obvious at the call site.

The preferred rule is:

```text
Nouns are properties.
Verbs are methods.
```

Use properties for values, state, identifiers, configuration, and computed data:

```doria
let $body = $message->body;
let $headers = $message->headers;
let $status = $message->status;
```

Avoid vague zero-argument noun methods when the member is conceptually data:

```doria
let $body = $message->body(); // avoid
let $headers = $message->headers(); // avoid
let $status = $message->status(); // avoid
```

A noun method such as `body()` can be misread as an action, preparation step, mutation, or builder-style operation. If the member represents data, expose it as a property.

Property hooks are the planned escape hatch when a property-shaped API needs validation, computed behavior, lazy decoding, caching, normalization, or guarded access. The public member should remain property-shaped when it is conceptually a value.

Use methods for actions, commands, mutation, I/O, async work, fallible operations, and behavior with meaningful work:

```doria
await $message->acknowledge();
await $message->retryAfter(seconds: 30);
$report->renderPdf();
```

If a data-returning operation must be a method because it performs I/O, expensive work, decoding, or another explicit operation, use an unmistakable verb such as `loadBody()`, `decodeBody()`, `findById()`, or `fetchProfile()`.

See `docs/api-design-guidelines.md` for the detailed design notes.

### Naming charter

Doria chooses casing by API category, not by whether an implementation is built into the language:

- Built-in free functions use `snake_case`, such as `read_line()` and `get_time()`.
- Userland free functions, instance methods, static methods, companion/type APIs, properties, parameters, and named arguments use `camelCase`.
- Classes, interfaces, traits, enums, and enum cases use `PascalCase`.
- Constants use `SCREAMING_SNAKE_CASE`.
- Type parameters use single Pascal capitals such as `T`, `K`, and `V`.
- PHP-shaped magic methods retain their inherited spellings: `__construct` and `__destruct`.

Free-function casing and member/companion casing are intentionally different:

```doria
let $now = get_time();
let $matches = String::startsWith($name, "Dor");
let $wrapped = Int::wrappingAdd(1, 2);
let $empty = $s->isEmpty();
let $tenant = $message->tenantId;
$message->retryAfter(seconds: 30);
let $person = $repository->findById($id);
```

Type-coupled vocabulary belongs to the type companion. For `string`,
`$text->length`, `$text->byteLength`, `$text->isEmpty`, `$text->bytes`,
`$text->graphemes`, and `$text->codePoints` are intrinsic properties or views;
all string-specific operations use the `String::` companion. Doria has no
public `str_*` family and no instance-method aliases such as `$text->trim()`.

A Doria `string` always contains valid UTF-8. `$text->length` counts Unicode
extended grapheme clusters, while `$text->byteLength` reports exact UTF-8
bytes. Search indices, slicing, and padding lengths use grapheme units.
`$text->graphemes` traverses extended grapheme clusters and
`$text->codePoints` traverses Unicode scalar values. Integer indexing on a
`string` is not permitted. The canonical string-operation families are
`String::trim`/`trimStart`/`trimEnd`, `lower`/`upper`, predicates and search,
replacement, `split`/`join`, `slice`, `repeat`, padding, `fromBytes`, and
comparison. Decision 0103 defines the reviewed v1 inventory. The executable
surface includes `length`, `byteLength`, `isEmpty`, `bytes`, trimming, default
Unicode casing and case folding, predicates, grapheme-indexed search,
replacement, `split`/`join`, `slice`, `repeat`, padding, and UTF-8-validating
`fromBytes`. It also includes the reviewed ignore-case search family,
first-grapheme casing, and occurrence counting. The `graphemes`/`codePoints`
views await the public traversal protocol, and ordering comparisons await the
executable `Ordering` type.

## 7. Basic type system

Accepted type-position names include:

```text
void (return position only)
int
int8
int16
int32
int64
uint8
uint16
uint32
uint64
float
float32
float64
string
bool
mixed
?T
T[]
List<T>
Dictionary<K, V>
Set<T>
SortedDictionary<K, V>
SortedSet<T>
PriorityQueue<T>
Deque<T>
ClassType
```

Reserved or rejected names:

```text
null      literal only; nullable values are spelled ?T
resource  reserved for Phase I PHP interop; rejected until specified
object    not a Doria type
array     not a Doria type; use T[] for typed arrays or collection aliases
```

The compiler keeps parsed type syntax and semantic types separate:

```text
TypeRef      parsed source spelling, such as `List<int>` or `Person`
TypeId       resolved semantic type identity
TypeKind     resolved semantic type shape
```

The semantic model also has an internal `Unknown` recovery type for diagnostics and error recovery; it is not the normal spelling for user-authored type declarations.

Lowercase primitive names are type-position names: `int`, `int8`, `int16`, `int32`, `int64`, `uint8`, `uint16`, `uint32`, `uint64`, `float`, `float32`, `float64`, `string`, and `bool`. PascalCase names are expression-level standard-library/helper or compiler-known companion APIs, not primitive type spellings or namespaces.

### Generics

Free functions, methods, and classes may declare type parameters after their name:

```doria
function first<T>(List<T> $items): ?T
{
    return $items->first;
}

function pair<T, U>(T $left, U $right): U
{
    return $right;
}

class Box<T>
{
    function __construct(take T $value) {}

    function get(): T
    {
        return $this->value;
    }
}
```

Calls infer type arguments from the arguments after ordinary positional/named binding. If arguments do not determine every parameter, a typed declaration may supply the expected result type. Doria has no explicit call-site turbofish syntax. Native compilation monomorphizes reachable calls and deduplicates identical concrete specializations.

Generic classes are instantiated in type positions and construction expressions with concrete type arguments, including nested forms such as `List<Box<int>>`. Every instantiation is a distinct monomorphized type with specialized field layout, methods, and drop glue. A class remains an owned move type regardless of its arguments; substituting only Copy fields does not turn it into a user-defined Copy aggregate.

Constraint declarations use `<T implements A, B>`, and constraints may themselves be generic. Compiler-known `Comparable`, `Hashable`, `Equatable`, and `Displayable` constraints are checked at instantiation without boxing primitives. User-defined interface constraints use the same surface once general interfaces land. Type parameters are invariant. Doria v1.0 has no default type arguments, compile-time value arguments, call-site turbofish, or runtime generic reflection.

### Fixed-width integers

Stage 13 implements these canonical integer types through semantic analysis, typed MIR, the debug interpreter, and Cranelift:

```text
int8   int16   int32   int64
uint8  uint16  uint32  uint64
```

`int` is signed 64-bit. `int64` is an exact source alias of `int`; they have one canonical semantic and runtime type. Doria has no bare `uint`, no pointer-width integer type, and no Rust-style `i8`/`u8`/`usize`/`isize` spellings.

Stage 14 implements `float32` as IEEE 754 binary32 and canonical `float`/`float64` as IEEE 754 binary64. `float` and `float64` are one semantic/runtime type; `float32` remains distinct, with no implicit width or integer conversion. Decision 0072 defines arithmetic, comparisons, special values, literal rounding, bool runtime values, and backend behavior.

An unconstrained decimal integer literal defaults to `int`. A literal may instead adopt an expected integer type from a declaration, parameter, return, assignment, or typed binary operand when its mathematical value fits that type. Contextual literal typing is not an implicit conversion. Out-of-range literals are compile-time errors.

The accepted future direction adds hexadecimal (`0x`), octal (`0o`), and binary (`0b`) integer literals plus `_` digit separators for readability (`1_000_000`, `0xFF_FF`). There will be **no typed numeric suffixes** (`100u8`): contextual literal typing already supplies a literal's width from its expected type, so a suffix would be a redundant second typing channel. These forms are not yet accepted source syntax; the current grammar remains decimal-only until a dedicated numeric-literals slice defines separator placement and malformed-form diagnostics and lands lexer, parser, semantic, and regression coverage together.

Both operands of an integer binary operator must resolve to the same canonical integer type. Nonliteral values never widen or narrow implicitly, and Doria has no C-style integer promotions. The implemented integer operators are:

```text
-  ~
+  -  *  /  %
<<  >>
&  ^  |
==  !=  <  <=  >  >=
++  --
+=  -=  *=  /=  %=  <<=  >>=  &=  |=  ^=
```

`+`, `-`, `*`, and signed negation are checked. Signed overflow, unsigned overflow, and unsigned underflow panic. Signed division truncates toward zero; division by zero panics, and signed minimum divided by `-1` panics. Signed remainder uses that quotient and gives a nonzero remainder the dividend's sign; remainder by zero panics, while signed minimum remainder `-1` is zero. Unsigned division and remainder use ordinary unsigned arithmetic.

Shift operands have one canonical integer type. A negative signed shift count or a count greater than or equal to the left operand's width panics. Left shift discards bits beyond the fixed width after validating the count. Signed right shift is arithmetic; unsigned right shift is logical. `&`, `|`, `^`, and `~` operate on the fixed-width two's-complement bit pattern and do not overflow. The word `xor` remains the distinct bool-only operator.

Explicit integer conversion uses compiler-known PascalCase companion intrinsics:

```doria
Int::from($value)
Int8::from($value)
Int16::from($value)
Int32::from($value)
Int64::from($value)
UInt8::from($value)
UInt16::from($value)
UInt32::from($value)
UInt64::from($value)
```

`Int` and `Int64` target the same canonical `int64` type. Each `from` accepts exactly one integer expression. Same-type and exact widening conversions preserve the value; narrowing and signedness-changing conversions are checked and panic with `integer conversion out of range` when the value cannot be represented. Stage 13 adds no `as` cast and no wrapping, saturating, or unchecked conversion API.

The exact operator, panic, and conversion rules are authoritative in decisions 0041 and 0042. The PHP compatibility backend supports only integer shapes it can preserve exactly; it emits a backend unsupported-feature diagnostic for precise Stage 13 behavior that PHP cannot represent rather than changing Doria semantics.

### Dynamic boundary type

`mixed` is Doria's only dynamic type. It has three laws:

1. `mixed` is unknown-flavored, never any-flavored. A `mixed` value permits no property access, method calls, arithmetic, concatenation, interpolation, comparison, or other typed operation until it is narrowed. Exact `is` and exact type-binding `match` patterns provide narrowing.
2. Any value may flow into `mixed` implicitly. This is the deliberate dynamic-boundary exemption from the no-implicit-conversion rule. Values do not flow out of `mixed` implicitly; source must narrow first. There is no cast spelling.
3. `mixed` is a boxed, runtime-tagged move type, always, even when the payload is a Copy value.

```doria
function describe(mixed $payload): string
{
    if ($payload is string) {
        return $payload;
    }

    return "unknown";
}
```

Stage 22 implements these static rules and classifies `mixed` as a move type.
Stage 23 Slice 3 implements the boxed, runtime-tagged `dr_mixed` representation for
bool, fixed-width numeric, string, and concrete-class payloads, plus `?mixed` and
runtime mixed collection values. Boxing collections, typed arrays, `Bytes`, and
other values outside the exact Stage 22 `is` set remains deferred with a
stage-named unsupported-feature diagnostic; those values are never represented by
a placeholder.

`object` does not exist in Doria. Use `mixed` for dynamic object-shaped boundaries and narrow with exact `is` or `match` type patterns.

`null` is a literal, not a type-position name. The internal null type exists for nullable machinery, but source spells nullable values as `?T`.

`resource` is reserved for Phase I PHP bridge work and is rejected until the bridge specifies its exact semantics.

`void` is valid only as a function or method return type, including `main(): void`; it is not a value type.

### Nullable values and narrowing

`?T` contains `T` and `null`. The null literal is assignable to `?T` and
`mixed`, but not to non-nullable `T`. Nullable values have no implicit
truthiness.

```doria
class Label
{
    function text(): string
    {
        return "Doria";
    }
}

function display(?Label $label): string
{
    if ($label != null) {
        return $label->text();
    }

    return $label?->text() ?? "none";
}
```

`$left ?? $fallback` evaluates the left operand once and evaluates the fallback
only when the left value is null. `$receiver?->member` evaluates the receiver
once and skips the access, method body, and arguments when it is null. A
value-producing null-safe access produces a nullable result; an already-nullable
member result is not wrapped in another nullable layer.

`== null` and `!= null` establish path-sensitive null facts. `$value is T`
tests and narrows `mixed` or `?T` against an exact fixed-width numeric, `bool`,
`string`, declared enum, or declared concrete class type. Facts follow lexical bindings,
short-circuit control flow, assignments, loops, and branch joins; a fact is
available after a join only when every incoming path proves it. Ordinary member
access on a possibly-null class value is an error until the value is narrowed.

Stage 22 exact class tests do not perform subtype or interface conformance.
Hierarchy `is` is deferred to Stage 34 and interface `is` to Stage 35; those
forms parse and receive stage-named diagnostics. Core exact type-binding match
patterns use the same Stage 22 exact-type narrowing facts.

Nullable concrete classes use a null pointer for absence. Other nullable values
use an explicit presence word and payload. `?T` keeps `T`'s ownership class:
nullable Copy payloads remain Copy, while `?Class` remains a move value and drops
the class only when present. Decision 0093 defines the complete Stage 22 model.

### Enums

Enums are nominal top-level types. Enum and case names use PascalCase, every
case ends with `;`, and cases are selected with static-member syntax:

```doria
enum Status
{
    case Draft;
    case Published;
}

Status $status = Status::Draft;
```

A backed enum uses exactly `int` or `string`. Every case has one unique,
exactly typed constant backing value. The public backing value is available
through the readonly `value` property; it is not the enum's identity and there
is no implicit enum/backing conversion.

```doria
enum Priority: int
{
    case Low = 1;
    case High = 10;
}

echo Priority::High->value;
```

Unit and backed enums are inline Copy values. Equality requires the same enum
type and compares case identity. Enums are not implicitly displayable and do
not gain automatic hashing or ordering. Nullable enums keep presence separate
from the private case tag, and enum values boxed into `mixed` retain exact enum
type identity.

Payload cases store explicitly typed readonly owned fields inline with their
case tag. Construction accepts positional or named arguments; expressions run
once in source order and fields initialize in declaration order. An enum is
Copy only when every payload in every case is Copy, and only its active case is
dropped, with fields destroyed in reverse declaration order. Equality first
compares nominal type and case, then compares active fields left to right.
Payload fields are observed through Stage 28 pattern matching, not as ordinary
properties.

Payload enums work in nullable values, `mixed`, class and generic-class
properties, function calls and returns, Copy constants/defaults, and collection
value positions whose existing equality/order constraints permit them. They do
not gain automatic hashing, ordering, display, reflection, or heap identity.
Core `match` observes payloads through readonly positional bindings or case-only
patterns. Generic enums remain deferred. Decision 0114 defines the enum model;
decision 0115 defines match semantics and ownership.

### Match expressions

`match` is an exhaustive expression. Its scrutinee evaluates once, arms are
tested in source order, exactly one arm expression executes, and there is no
fallthrough:

```doria
string $label = match ($status) {
    Status::Draft => "draft",
    Status::Published => "published",
};
```

Core patterns are qualified enum cases, exact compile-time-known literals and
constants, `null`, exact `Type $binding` patterns, and one final `default`.
Payload cases bind every field positionally when parentheses are present; a
case-only pattern ignores all payloads. Bindings are readonly and local to one
arm.

Enum and bool matches must cover their finite domain. Nullable finite matches
also cover `null`. Integer, float, string, class, and `mixed` open domains use a
final `default`, except the exact nullable pair `null` plus `Type $binding`.
Duplicate and unreachable arms are errors. Every arm produces one non-void
value, and arm types unify without numeric widening or an implicit `mixed`
fallback.

```doria
string $description = match ($value) {
    int $number => "integer {$number}",
    string $text => "text {$text}",
    default => "other",
};
```

`match (true)` is the ordered-condition form. Its reached conditions are strict
`bool`, evaluate once in source order, and stop at the first true arm; it
requires `default`. Full right-associative ternary is the same two-arm bool
match semantically. Short ternary/Elvis `?:` is not Doria.

Named move scrutinees are borrowed rather than consumed. Temporary move
scrutinees live through the selected arm. Copy payload bindings copy, including
the normal immutable string retain; move payload bindings are readonly borrows
and may not escape their provenance. An arm guard is written `Pattern if
condition => value`, requires `bool`, runs once only after the pattern succeeds,
and falls through to the next arm when false. A guarded arm does not complete
coverage unless the guard is compile-time `true`; a later unguarded copy may
complete it. `default` and `match (true)` arms do not take guards.

`match (take $value)` explicitly consumes the complete Move scrutinee. During a
guard, payload bindings are readonly views; after a successful guard, selected
Move payloads become owned arm bindings. Failed guards transfer nothing.
Payload-level `take`, writable match scrutinees, and writable payload patterns
are rejected in Doria v1. Decision 0115 is authoritative.

Typed arrays use C-style suffix spelling:

```text
T[]
```

Examples:

```doria
int[] $numbers = [1, 2, 3];
string[] $names = [];
int[][] $matrix = [[1], []];
```

`array` is not a Doria type-position name.

Collection aliases have fixed arity:

```text
List<T>
Dictionary<K, V>
Set<T>
```

Bracket sequence literals have an element-list form and a repeat form:

```doria
int[] $values = [1, 2, 3];
bool[] $flags = [false; $count];
List<string> $labels = ["pending"; $count];
let $zeros = [0; $count]; // List<int>
```

`[value; count]` is contextually typed as `T[]` or `List<T>` and defaults to
`List<T>` when there is no expected type. `value` is evaluated once, then the
runtime `int` count is evaluated once. A constant-negative count is a compile
error, a runtime-negative count panics with `fill count is negative`, and zero
produces an empty sequence. Copy scalars are bit-copied; immutable string
handles are shared. The repeat form is rejected for `Set` and `Dictionary`, and
move-type elements remain unavailable until the `Cloneable` contract can define
their replication.

`let` declarations infer simple literal and constructor types:

```doria
let $x = 5;        // int
let $name = "Doria"; // string
let $person = new Person("Andrew"); // Person
```

The semantic checker resolves parsed type syntax into semantic types before checking assignment, return, and positional call compatibility. Doria checks typed declarations, property initializers, property writes, parameter defaults, declared function/method return values, and call arguments for functions, methods, static calls, and constructors. It does not perform PHP-style scalar coercion: `int` is not assignable from `string`, `string` is not assignable from `int`, and `bool` is not assignable from `int`.

### String literals and interpolation

Single-quoted string literals are plain string literals. Double-quoted string literals support braced interpolation using Doria-owned syntax, not PHP backend behavior:

```doria
let $name = "Andrew";
echo "Hello, {$name}";
echo "Hello, {$this->profile->displayName}";
```

The contents of each interpolation brace use the ordinary Doria expression grammar. Variables, property paths, grouping, arithmetic, comparisons, function calls, static calls, string calls, and nested expression structure retain their normal parsing and semantic rules. Parts evaluate left-to-right and exactly once; interpolation adds no newline.

Interpolated strings are represented in the AST and Doria IR as string parts before any backend runs. The PHP backend lowers them explicitly, for example `"Hello, {$name}!"` becomes PHP equivalent to `"Hello, " . $name . "!"`.

Literal opening braces in double-quoted strings are written `\{`. A bare `}` is literal outside an open interpolation, and `\}` is accepted but not required. Brace doubling is not an escape. A bare `{` that does not begin a valid expression is an error with a machine-applicable `\{` fix. Single-quoted strings remain non-interpolating and are the simple choice for brace-heavy text.

Interpolated values may be `string`, a fixed-width integer, `float`, `bool`, or an explicitly conforming `Displayable` class. They use the same canonical display conversion as `echo`, `.`, and Stage 17 `%s`. Null, nullable values without a non-null proof, `mixed`, typed arrays, `List<T>`, `Dictionary<K, V>`, and `Set<T>` are rejected.

`Displayable` is a narrow compiler-known nominal interface contract, not general interface support:

```doria
class Label implements Displayable
{
    function toString(): string
    {
        return "Doria";
    }
}
```

Conformance requires the explicit `implements Displayable` declaration and exactly an externally accessible readonly instance `function toString(): string` with no parameters. Method-name coincidence does not conform, and Doria has no `__toString` magic method. Display conversion is limited to interpolation, `echo`, `.`, and `%s`; it does not permit implicit class-to-string assignment. For a statically known concrete class, the interpreter, Cranelift, LLVM, and PHP compatibility backend execute conversion through the ordinary `toString()` method machinery exactly once and left-to-right. Interface-typed values, vtables, and general interface dispatch remain Stage 35.

The `.` operator is runtime string concatenation. Each operand may be a display-convertible primitive, but at least one operand of that binary operation must already be statically `string`; therefore `"x=" . 1` is valid while `1 . 2` is rejected. The result is `string`, evaluation is left-to-right, and no conversion is implied outside display contexts. `echo`, `.`, and current interpolation parts use decimal integers, shortest-round-trip locale-independent binary32/binary64 floats, lowercase `true`/`false`, and strings unchanged.

There is no implicit widening, narrowing, or scalar coercion between distinct integer or float types. `float` is not assignable from an integer and an integer is not assignable from `float`. Stage 14 provides only `Int::toFloat(int): float` and checked `Float::toInt(float): int`; decision 0042 defines their exact contracts. Named arguments are designed by decision 0098 and scheduled for Stage 23a; they do not alter numeric conversion rules.

Simple collection literals infer collection element/key/value types when all clear parts match. Clear heterogeneous collection literals, such as `[1, "two"]`, are rejected by typed array and narrow collection alias assignment checks rather than being erased to `Unknown`. The empty literal `[]` stays ambiguous so typed contexts may use it as an empty `T[]`, `List<T>`, or `Dictionary<K, V>`.

### Text I/O and ambient checked failures

Stage 17 provides these compiler-known built-ins:

```doria
read_line(string $prompt = ""): ?string
read_file(string $path): string
write_file(string $path, string $contents): void
write_stderr(string $value): void
```

The compiler-known `sprintf` returns `string` and remains nonthrowing. `printf`
returns `void`; every `echo` statement carries the ambient
`Doria\Std\Io\IoError` runtime effect. Each formatting intrinsic
takes a literal `string $format` first, followed by the typed operands required
by that format. The intrinsic-only operand tail is not an untyped Doria
parameter declaration.

Exactly `Doria\Std\Io\IoError` and
`Doria\Std\Io\InvalidUtf8Error` are ambient checked effects. Authors do not
need to list them in `throws` or catch them locally, but they remain exact,
catchable runtime Errors. Explicit ambient `throws` entries remain accepted.
Unhandled ambient I/O still performs checked cleanup and reports R1000 with
status 70. Ambient-only differences do not change structural function identity;
required nonambient Errors still do. Decision 0123 defines the complete effect
and finalizer model.

The additive text-file spelling is
`append_file(string $path, string $contents): void`. It is implemented by Stage 23 Slice 2 alongside
the binary file tier; `write_file` remains truncate-only.

`read_line` reads UTF-8 text, removes one LF ending and a preceding CR when present, preserves empty lines and final unterminated lines, and returns `null` only when EOF occurs before any bytes. Its return type was the first supported position for the nullable `?T` model now generalized by Decision 0093. A `!= null` guard narrows `?string` to `string`; assigning `null` or another nullable result invalidates that fact, while assigning a known `string` establishes a new non-null fact.

`read_file` and `write_file` are text-file functions. `read_file` reads an entire file and validates UTF-8 before constructing a `string`; invalid bytes never enter a Doria string. `write_file` creates or truncates a text file and writes the string's exact bytes. `write_stderr` writes exact bytes without adding a newline. An ordinary program write to stdout or stderr that reports a closed pipe exits immediately with status 0 and emits no panic diagnostic or `Call Path`. This exception does not apply to panic diagnostics: a panic remains fatal with status 101 when stderr is unavailable, although its best-effort diagnostic output may be absent. Other I/O failures are checked errors: invalid UTF-8 is `Doria\Std\Io\InvalidUtf8Error`, while file, input, flush, and non-broken-pipe device failures are `Doria\Std\Io\IoError`. `null` from `read_line` means EOF and never signals an error.

Stage 23 Slice 2 binary file I/O is whole-file:
`read_file_bytes(string $path): Bytes`,
`write_file_bytes(string $path, Bytes $contents): void`, and
`append_file_bytes(string $path, Bytes $contents): void`. The write functions borrow `$contents`;
they do not consume it. Reads and writes preserve exact bytes without UTF-8 validation or newline
translation. `File` and stream objects, including RAII close and buffered/seekable access, are
planned after Stage 29. These future tiers do not change the text and EOF contracts.

Binary standard-stream I/O is `read_stdin_bytes(): Bytes`,
`write_stdout_bytes(Bytes $contents): void`, and `write_stderr_bytes(Bytes $contents): void`;
each carries ambient `Doria\Std\Io\IoError` at runtime.
`read_stdin_bytes` slurps to EOF and returns an empty `Bytes` at immediate EOF. Byte output borrows
its buffer and follows the ordinary closed-pipe clean-exit rule. All Stage 23 I/O intrinsics are
unshadowable.

`sprintf` and `printf` require a direct literal format in Stage 17. The compiler parses it into a validated MIR plan before any backend runs. Accepted conversions are `%s`, `%d`, `%f`, `%x`, `%X`, `%o`, `%b`, and `%%`; accepted controls are decimal field width, `-` left alignment, `0` numeric zero padding, and `.N` precision on `%f`. Width for `%s` counts UTF-8 bytes. Formatting is deterministic and locale-independent. `printf` uses the same plan, returns `void`, and adds no newline. `print` is rejected in favor of `echo`; dynamic/positional formats, `*` width, `%e`, `%g`, and `sscanf` are not accepted.

The runtime separates raw standard-device reads/writes and explicit flush from buffered line discipline. It detects stdin, stdout, and stderr interactivity independently for internal use. On Windows, interactive console text uses validated UTF-8 converted to wide console operations; redirected handles preserve exact UTF-8 bytes. Binary standard-stream operations bypass text-console conversion and preserve exact bytes. This is infrastructure for the future Stage 46 `Console` API, not a public terminal API.

### Hosted stream architecture (accepted future target)

Decision 0110 defines the hosted I/O architecture that Stage 36a will implement.
Generic streams are byte-oriented owned move values exposed through small
capability interfaces rather than one universal stream class. A handle may be
readable, writable, duplex, seekable, flushable, blocking-configurable, or
readiness-aware only when it actually provides that authority. Explicit
close/finish consumes an owned handle and reports checked failure; destructor
cleanup is best-effort and nonthrowing. Structured checked-error exits clean up,
while abort-only panic does not.

Primitive reads distinguish data, would-block, EOF, and timeout; empty data is
not EOF. Primitive writes preserve partial progress and distinguish would-block.
One multi-stream readiness substrate, durations and absolute deadlines, and one
cancellation/backpressure model serve synchronous I/O, future async lowering,
networking, child-process pipes, and terminal input. Platform polling and handle
types never enter the Doria surface.

Standard input, output, and error become first-class non-owning, nonclosable
views over the same devices used by the intrinsics above. Typed per-value
buffering and UTF-8 text adapters layer over byte streams, own their underlying
handle by default, and use explicit borrowing when they do not. Line and
delimiter operations are bounded. Typed file requests replace mode strings;
buffer flush, durable data synchronization, and full synchronization are
distinct. Owned child processes expose typed pipes and require concurrent,
bounded stdout/stderr drainage.

The steady-state stream data plane does not require an allocation per read,
write, readiness event, adapter hop, or loop iteration. Chunk operations expose
reusable caller- or adapter-owned storage and safe readable/writable byte
regions; successful progress reports the initialized or consumed extent without
copying a whole chunk or unread suffix. Text decoding is incremental, buffering
is bounded, and backpressure prevents an unbounded producer queue. Static
generic adapters remain eligible for specialization and inlining; deliberate
interface erasure may use dynamic dispatch but does not require a heap object per
call or layer.

Readiness implementations reuse registration and platform event storage rather
than rebuilding them on every wait, and ordinary operation does not busy-poll or
allocate one thread per stream. Timing support is opt-in per operation or wait.
Synchronous programs do not initialize async executors, task objects, or async
scheduler infrastructure. Stage 36a owns the initial cross-platform stream
performance and memory regression gate defined by decision 0110; Stage 43
continues and broadens the benchmark suite rather than postponing that gate.

Stage 36a is scheduled and not implemented. The semantic and performance
contracts are accepted. Exact public interface, member,
outcome-case, readiness, standard-stream, file, adapter, and process spellings
remain deferred under decision 0110; the accepted semantics above do not.

### Equality and boolean operators

Doria equality is typed:

```doria
==
!=
```

`==` is typed equality. `!=` is typed inequality. Doria does not use PHP-style loose comparison, so expressions such as `1 == "1"` and `false == 0` are type errors rather than truthy comparisons. Doria does not use PHP strict-comparison operators; `===` and `!==` are not part of Doria syntax.

Accepted boolean operators are:

```doria
!
not

&&
and

||
or

xor
```

`not` is an exact synonym for `!`, `and` is an exact synonym for `&&`, and `or` is an exact synonym for `||`. Doria does not copy PHP's lower-precedence `and` / `or` behavior. Boolean operators require `bool` operands, and conditions must be `bool`; Doria does not use PHP-style truthiness.

`xor` is bool-only boolean exclusive OR. It evaluates both operands and does not short-circuit. It is not bitwise XOR. Unparenthesized chained `xor` and `xor` mixed with `and`, `or`, `&&`, or `||` should require parentheses or produce a diagnostic/lint when implemented.

Accepted bitwise operators are:

```doria
&
|
^
~
```

`&`, `|`, `^`, and `~` are integer bitwise operators. They are not boolean operators, and `&` / `|` are not aliases for boolean AND/OR. Doria does not add `^^`.

Do not add `nand`, `nor`, `implies`, `iff`, `unless`, `^^`, `===`, or `!==` as core syntax without a new accepted decision. Future helper APIs such as `Bool::all(...)`, `Bool::any(...)`, `Bool::none(...)`, or `Bool::one(...)` may be considered separately.

The accepted boolean/equality/bitwise operator direction is recorded in decisions 0020, 0041, and 0072. Current compiler support includes typed scalar equality, rejection of `===` / `!==`, runtime bool locals/parameters/returns/calls, value- and condition-position short-circuit `not`/`and`/`or`, eager `xor`, fixed-width integer bitwise operators, typed MIR lowering, and native execution. PHP lowers only its exact supported subset.

### Control-flow conditions

Basic `if` / `else if` / `else` and `while` are MVP syntax. Conditions must be `bool`; Doria does not use PHP-style truthiness for integers, strings, null, dynamic boundaries, or collections. The checker currently allows the internal `Unknown` recovery type so one diagnostic does not cascade into unrelated follow-up errors.

Each `if`, `else if`, `else`, and `while` body has its own block scope. Variables declared inside those bodies are not visible after the block. Constructor readonly init access is path-sensitive inside conditional branches, but remains unavailable inside repeatable `while`, `for`, and `foreach` bodies.

`if` is statement control flow and does not return a value. `if` without `else` is valid Doria. `else`, `else if`, `given`, and `finally` are optional. A base `if`, `while`, `foreach`, or future control construct does not require `given` or `finally`.

`when` is the value-returning form of `if`: the same `given` / `else when` /
`else` / future `finally` structure (`when` / `else when` in place of `if` /
`else if`), differing only in that it always yields a value. Its one result type
is written on the head only (`when (cond): T`), supplied by a surrounding
expected type when omitted, or inferred from the first reachable head-branch
yield when neither exists. It requires a total `else`, and each branch produces
the value with `return expression;`, which completes the nearest enclosing
`when` rather than returning from the function. Bare `return;` and `void` result
types are invalid. Every normally completing branch path must yield.

Conditions are strict `bool` and run in source order until one branch is
selected. An unannotated all-null `when` is valid only in an expected nullable
context. Copy results copy; Move results are acquired exactly once into the
merge result before selected-branch cleanup. Decision 0116 defines the complete
Stage 28a model.

### Checked errors

Decision 0119 defines Doria's checked-error model. `Error` is a compiler-known
core interface. A class conforms only by explicitly declaring `implements Error`
and providing an externally accessible readonly stored `string $message`
property. A promoted readonly constructor parameter named `message` satisfies
the contract. Error classes are ordinary Move classes; there is no mandatory
base class, synthesized property, automatic cause chain, hashing, or ordering.

`throws` follows the explicit return type and preserves source order:

```doria
class StorageError implements Error
{
    function __construct(string $message, string $operation)
    {
    }
}

function loadRecord(string $id): Record throws StorageError
{
    throw new StorageError("record unavailable", "load");
}
```

Named functions and methods still require explicit return types. Constructors
may omit a return annotation and declare `throws`; destructors cannot declare or
allow checked errors to escape. Each throws entry is `Error` or a concrete
Error-conforming class. Duplicate entries and concrete entries after the
catch-all `Error` are rejected. Nullable, primitive, collection, `mixed`,
unknown, and non-Error entries are invalid.

`throw expression;` is a statement. Its operand evaluates exactly once, must be
an owned Error value, and transfers ownership. Rethrow uses `throw $error;`.
Bare throw and expression-position throw are not Doria syntax. Ordinary move
checking rejects use of a named Error after it is thrown.

```doria
function renderRecord(string $id): string
{
    try {
        let $record = loadRecord($id);
        return $record->title;
    } catch (StorageError $error) {
        return $error->message;
    } finally {
        recordAttempt();
    }
}
```

A `try` statement requires at least one `catch` or `finally`; catches precede
the optional finalizer. Catch bindings are optional. A present binding is owned,
readonly, and catch-scoped. Concrete catches match exact concrete Error identity
in Stage 29; `catch (Error)` catches every checked error. Duplicate catches,
catches after `Error`, and catches proven unable to match a protected effect are
unreachable. Catch bodies are independent: sibling catches do not handle an
error raised by another catch. A checked Error may escape `finally`; a catch on
that same `try` does not cover it, while a finalizer-local or outer catch may.
The finalizer Error supersedes any pending nonfatal outcome and destroys a
superseded owned payload exactly once.

Every callable carries required, ambient, and complete semantic checked-effect
profiles. Direct throws
and resolved function, method, static, constructor, property-initializer, and
compiler-known built-in calls contribute effects. Catches remove only effects
they cover. Ordinary reusable functions, methods, constructors, and generic
specializations must declare every remaining required error. Exactly
`Doria\Std\Io\IoError` and `Doria\Std\Io\InvalidUtf8Error` are ambient and do
not impose that source obligation, but remain in the complete executable
profile. When the selected top-level
`main` omits `throws`, its exact remaining set is inferred instead; source calls
to `main` observe that effective contract. Source syntax remains separate, so
the AST does not gain a synthetic clause or invented span. A written `main
throws` remains accepted and incomplete explicit clauses still produce E0631.
Nonthrowing and narrower required-effect sets may be used where a wider set is
accepted, never the reverse. Ambient-only differences do not change structural
function identity, while executable function values retain ambient-capable
transport.

Checked propagation performs deterministic cleanup through the existing
structured finalizer regions, but never rolls back completed side effects. A
failed construction runs no class `__destruct`; initialized owned fields drop in
reverse order, uninitialized fields are ignored, allocation is freed, and the
error continues. Fatal panic remains separate, non-catchable, cleanup-free, and
status 101.

Stage 29 implements grammar, semantic checking, AST/HIR,
ownership, the two-word erased carrier, hidden first-throw origin storage,
checked MIR, the status/out-slot ABI, propagation, cleanup, exact catches,
rethrow, canonical `Doria\Std\Io` errors, checked I/O effects, and backend
transport. Handled checked errors execute through the interpreter, Cranelift,
LLVM, and the PHP compatibility backend. An Error escaping `main` performs
required cleanup, is reported as `Error[R1000]`, and exits with status 70. A
successful `main(): int` may independently return 70 without producing R1000.

The compiler-known I/O family contains `IoOperation` (`Open`, `Read`, `Write`,
`Append`, `Flush`), `IoTarget` (`File(string $path)`, `StandardInput`,
`StandardOutput`, `StandardError`), `IoErrorReason` (`NotFound`,
`PermissionDenied`, `InvalidInput`, `Interrupted`, `ResourceExhausted`,
`Unsupported`, `Closed`, `Other`), and `Utf8InputSource` (`File(string $path)`,
`StandardInput`). `IoError` exposes readonly `message`, `operation`, `target`,
`reason`, and `?int systemCode`; `InvalidUtf8Error` exposes readonly `message`,
`source`, `validByteCount`, and `?int invalidByteCount`. Counts are bytes.
Stable messages are Doria-owned and host-localized error prose is not exposed.
P1401 through P1407 remain historical catalogue identities with no ordinary
valid source route; string and Bytes allocation failures remain P1206 and P1302.

`Result<T, E>` is not Doria's default error model. Runtime panic is separate
from checked `throw` / `throws`.

### Panic

`panic("message");` invokes a compiler-known built-in free function that terminates execution. Panic is fatal, is not catchable, does not unwind, and does not run cleanup or destructors while aborting in v1.0. User code cannot redeclare `panic`.

The current compiler accepts a string literal, readonly compile-time-known string local, or concatenation of those expressions as the panic message. Panic produces a source-aware runtime diagnostic through the compiler-owned diagnostic model and exits with status 101:

```text
Panic[P1000]: User Panic

Where
<path> · line <line> · <function>

<source preview and marker>

Note
<message>

Call Path
<currentFunction> · <path>:<line>
<callerFunction>  · <path>:<line>
main              · <path>:<line>

Process Exited With Status 101
```

Checked integer addition, subtraction, multiplication, and signed negation overflow use this runtime-outcome path for every integer width. Division by zero, signed division overflow, remainder by zero, an out-of-range shift count, and an out-of-range explicit conversion use the same catalogue infrastructure with distinct codes and titles. Returning a process status outside `0..125` from `main(): int` also panics. A panic is not a compilation failure: it is a runtime outcome represented by the compiler-owned `Diagnostic`, with abort-without-cleanup termination semantics.

Checked `throw`, `throws`, `try`, and `catch` are implemented through Stage 29.

### given predicate blocks

A `given` block attached to a control construct may contain variable declarations, void expression statements, and bool expression statements. Bool expression statements are predicates. Void expression statements are setup actions. Variable declarations introduce scoped names available to the attached control construct. Non-bool, non-void discarded expressions should be rejected.

```doria
given {
    let $user = $session->user;
    let $permission = Permission::EditPost;

    $user->isActivated;
    $this->isOrgMember($user) || $this->isAdmin($user);
} if ($user->can($permission)) {
    $post->publish();
}
```

Separate bool predicate lines are implicitly AND-ed in source order with the attached control condition. Bool predicates short-circuit the attached condition and body when false. Inside a predicate, normal boolean short-circuiting applies for `&&` / `and` and `||` / `or`; `xor` does not short-circuit.

Setup declarations and `void` actions must precede the first predicate. Setup
runs once. Predicates run once for `if` and `when`; for `while` they reevaluate
before every condition check, including after body completion and `continue`.
A failed gate skips every attached conditional condition and selects only the
unconditional `else` when present.

The scoped declarations remain visible through the complete attached construct
and finalizer, then leave scope. Ownership and borrow checking use the ordinary
lexical rules. Outgoing values are acquired before branch cleanup, the finalizer
runs next, and scoped `given` declarations are released afterward. A transfer
inside `finally` may target only control flow wholly contained in that finalizer.

### do while

```doria
do {
    advance();
} while ($ready);
```

The body executes before the first strict-`bool` condition. `continue` reaches
the condition and `break` exits. The ordinary form requires its semicolon. In
the finalizer form, `while ($ready) finally { ... }` has no
intervening or trailing semicolon. `given` does not attach to `do`.

## 8. Class syntax

Doria's accepted OOP declaration vocabulary is recorded in `docs/decisions/0029-oop-declaration-vocabulary.md`. Current compiler support may lag this accepted direction until lexer, parser, semantic checking, Doria IR, backend, and LSP support are updated.

Accepted OOP declaration vocabulary:

```text
class
interface
trait
extends
implements
```

`class` declares a class type. Doria already has class syntax in the current compiler surface:

```doria
class Person
{
    function __construct(
        writable string $name,
        int $age,
    ) {
    }
}
```

Doria will support `interface` for contracts that classes can implement:

```doria
interface Renderable
{
    function render(): string;
}
```

Interfaces may declare method requirements and may extend one or more interfaces. Interface members do not define instance storage. Default methods, static interface methods, constants, generic interfaces, variance, and interface property requirements remain future design work.

Doria accepts `trait` declaration grammar for reusable class-body members:

```doria
trait HasSlug
{
    string $slug;
}
```

Current semantic checking reports trait declarations as unsupported until Stage 35. The accepted grammar preserves member bodies such as `self::MAX_DEPTH` without false parser errors. Traits may eventually be composed into classes or other traits with `uses`. Trait conflict-resolution rules, aliasing, access changes through trait composition, trait property rules, trait static member rules, trait abstract method requirements, and whether PHP-style `insteadof` / `as` are accepted exactly remain future design work.

Doria will support `extends` for inheritance:

```doria
class Post extends Model
{
}

interface JsonRenderable extends Renderable
{
}
```

Likely direction: a class may extend at most one class, and an interface may extend one or more interfaces. Constructor inheritance, initialization order, override rules, virtual dispatch layout, final/sealed behavior, runtime layout, and ABI remain future design work.

Doria will support `implements` for compiler-checked interface conformance:

```doria
class Post extends Model implements Renderable, JsonSerializable
{
}
```

Likely direction: a class may implement one or more interfaces, and Doria's PHP-shaped direction points toward nominal interface conformance. Exact conformance checking details remain future implementation work.

`use` and `uses` have distinct meanings:

```text
namespace/file-scope use  -> semantic import / alias
class-body/trait-body uses -> trait composition
```

```doria
namespace App\Posts;

use App\Models\Post;
use App\Security\Permission;

class Article
{
    uses HasSlug;
}
```

Namespace/file-scope import `use` is implemented and resolved at compile time. Class-body or trait-body trait-composition `uses` remains a distinct spelling under its own later implementation stage.

Doria is PHP-shaped, not PHP++. Accepting PHP-shaped OOP declaration syntax does not import PHP dynamic object semantics, magic methods as core behavior, autoloading behavior, reflection behavior, loose typing, PHP visibility rules beyond what Doria has separately accepted, PHP trait conflict-resolution rules without review, or PHP runtime initialization behavior.

OOP declaration vocabulary is accepted separately from final visibility semantics. Doria's accepted early member model remains default-accessible plus `internal`: class members are accessible by default, `internal` controls API surface, and `writable` controls mutation.

Constructor property promotion is supported in the current vertical slice. Constructor parameters are promoted to externally accessible properties by default unless marked `internal`:

```doria
function __construct(
    writable string $name,
    int $age = 10,
    internal string $cacheKey = "person",
) {
}
```

Constructor init access supports direct initialization of uninitialized properties inside constructor bodies:

```doria
class Person
{
    string $id;

    function __construct(string $givenId)
    {
        $this->id = $givenId;
    }
}
```

This does not make `$this` writable. The constructor cannot assign the same readonly property twice on one reachable path, cannot reassign a readonly property that already has an initializer or is promoted from a constructor parameter, cannot use compound assignment for direct readonly init access, and cannot use constructor privilege to initialize a nested readonly property. Conditional readonly initialization is valid when every normally continuing branch initializes the property exactly once. A readonly property initialized on only some incoming paths cannot be repaired after the merge with an unconditional assignment.

A definitely initialized writable intermediate supplies ordinary writable access
to its owned child:

```doria
class Window
{
    writable string $title = "";
}

class Application
{
    internal writable Window $window = new Window();

    function __construct(string $initialTitle)
    {
        $this->window->title = $initialTitle;
    }
}
```

An owned value may also directly initialize an owning property:

```doria
class Application
{
    internal writable Window $window;

    function __construct(string $initialTitle)
    {
        $this->window = new Window($initialTitle);
    }
}
```

An initialized writable owning property may later be replaced from a fresh or
otherwise independently owned value. The new value is acquired before the old
value is destroyed. General move-out from a property remains unavailable.

Doria should support richer instance property initializers than PHP:

```doria
class Office
{
    Person $manager = new Person();
}
```

Instance property initializer expressions run once per object construction. Each object gets its own initialized value. A property initializer counts as initialization for readonly properties.

## 9. Function syntax

```doria
function greet(string $name): void
{
    echo "Hello, {$name}";
}
```

Top-level function names beginning with `__doria_` are reserved for compiler-generated helpers.
The prefix does not reserve method names or otherwise change Doria's member model.
Type declaration names beginning with `__Doria` are reserved case-insensitively for
compiler-generated compatibility types. This type prefix does not reserve local,
property, method, or function names.

Parameters are readonly unless marked `writable`:

```doria
function rename(writable Person $person, string $name): void
{
    $person->name = $name;
}
```

Declared return types are checked against returned expressions:

```doria
function age(): int
{
    return 37;
}
```

`void` functions and methods may use `return;` or fall through. Lifecycle methods, currently `__construct` and `__destruct`, are void-like: they may omit a return type or explicitly declare `: void`, may use bare `return;`, and may fall through. A non-`void` lifecycle return annotation is an error, and returning a value from a `void` function or lifecycle method is an error.

Lifecycle declaration shapes are a fixed allowlist. A constructor is declared as `function __construct(parameters)` or `internal function __construct(parameters)`. A destructor is declared as `function __destruct()` or `internal function __destruct()`. Either may explicitly declare `: void`. `static` and `writable` are rejected on both lifecycle names, and `__destruct` must declare exactly zero parameters. Other current or future method modifiers are rejected unless this specification explicitly adds them to the lifecycle allowlist.

Lifecycle methods cannot be invoked directly as ordinary instance or static methods. Construction uses `new Class(...)`, whose arguments are checked against `__construct`; destruction is compiler/runtime-invoked. The planned inheritance protocol reserves `parent::__construct(...)` for parent-constructor chaining once inheritance is implemented; this does not make other direct lifecycle calls legal.

For declared non-`void` return types, no reachable path may fall through the function body. `return` may occur anywhere in nested control flow. A path ending in `panic()` or a proven non-terminating `while (true)` loop without a reachable `break` is diverging and does not require a return. A loop with a reachable exit must lead to a return or another diverging path. Missing-return diagnostics are produced by path-sensitive source control-flow analysis before MIR lowering.

The program entrypoint may be `main(): int` or `main(): void`. `main(): int` returns an explicit process status. `main(): void` may fall through or use `return;` and maps normal completion to successful status `0`. Returning a value from `main(): void` is the same semantic error as returning a value from any other `void` function.

The selected top-level entrypoint may omit `throws`. In that form the compiler
infers the exact checked effects that escape its body after local catch
subtraction. An empty set uses the ordinary nonthrowing entry ABI; a nonempty set
uses the existing checked-result ABI and R1000/status-70 process boundary. Class
and static methods merely named `main` are ordinary methods and still require
explicit contracts. Explicit `main throws ...` remains accepted and checked.

```doria
function main(): void
{
    echo "Hello, Doria!\n";
}
```

`main` may also take command-line arguments through an optional parameter: `main(List<string> $args): int` (and the `: void` variant), per decision 0099. `$args` is populated by the entry glue at process start; `$args->count` is the argument count and there is no separate `argc`, so `main(string[] $argv, int $argc)` is rejected.

```doria
function main(List<string> $args): int
{
    printf("count=%d\n", $args->count);
    foreach ($args as $argument) {
        echo $argument;
        echo "\n";
    }
    return 0;
}
```

`$args` holds the program's arguments only: **the executable path is not element 0**, so `$args[0]` is the first real argument and `$args->count` is how many arguments the user passed. A program invoked with no arguments receives an empty list — never a one-element list, and never null. The executable path is a process fact reached through `Doria\Std\Process` instead.

The argument list is owned by the entry glue and borrowed by `main` for the duration of the call, so the parameter is an ordinary readonly borrow; declaring it `writable` or `take` is an error. An argument that is not valid UTF-8 panics rather than entering the program, because `string` is defined as immutable UTF-8 and invalid bytes never enter a Doria string.

This contract is identical on Linux, macOS, and Windows, and across the interpreter, Cranelift, and LLVM.

Calls are checked against declared parameter lists:

```doria
function greet(string $name, string $suffix = "!"): void
{
}

greet("Andrew");      // ok
greet("Andrew", "!"); // ok
greet();              // error
greet(123);           // error
```

Arguments may also be passed by parameter name, per decision 0098:

```doria
function describe(string $name, int $age, string $city = "unknown"): void
{
}

describe(name: "Ada", age: 36);        // named
describe(age: 36, name: "Ada");        // any order
describe("Ada", city: "London", age: 36); // positional first, then named
describe(name: "Ada", 36);             // error: positional after named
```

Positional arguments may precede named arguments but may not follow them. Named
arguments may appear in any order, and may skip a parameter that has a default —
including a *middle* one, which positional calls cannot express. A parameter
supplied twice (named twice, or once positionally and once by name), an unknown
parameter name, and a missing required parameter are each errors.

Arguments evaluate in source (written) order regardless of the parameter each
one binds to: `f(b: g(), a: h())` runs `g()` then `h()`, then binds those results
to `b` and `a`. Ownership and borrowing are checked over that same written order.

Because a callable can be invoked by parameter name, its parameter names are part
of its public interface: renaming a parameter is a breaking change for
named-argument callers. Language intrinsics keep positional-only binding.

Native execution currently supports omitted trailing defaults when the parameter is a fixed-width integer, float, bool, or readonly string and the default is accepted by the Stage 20 constant-evaluation tier. This applies uniformly to free functions, instance methods, static methods, and constructors. A writable Copy-scalar parameter may use such a default because writability does not change its ownership classification. For a readonly string parameter, the caller materializes the folded value as an ordinary string-literal argument. Ordinary call temporaries are released after the call; a constructor-promoted value is retained by the property and released with the object. The compiler inserts each folded value at its omitted call position before MIR execution.

Defaults for `?string`, `writable string`, `take string`, other move types, and `take` parameters remain deferred until their representation, mutation, construction, and destruction obligations are implemented. Non-constant defaults are rejected before MIR. A named argument may skip any parameter whose default is supported above, including a middle one; the compiler splices the folded value at that parameter's position.

## 10. Collection aliases

Doria uses:

```doria
int[]
List<int>
Dictionary<string, int>
Set<string>
```

Do not use `Vec`.
Do not use `array` as a type spelling.

The current PHP compatibility backend may lower typed arrays and collection aliases to PHP arrays, while the Doria type checker keeps them distinct. The native backend must make deliberate representation choices for typed arrays and each collection family rather than inheriting PHP array behavior.

The current type foundation resolves explicit annotations, reports unknown type names and invalid collection alias arity, and checks assignment compatibility for typed declarations, property initializers, property writes, parameter defaults, declared return values, and positional call arguments. Classes without constructors cannot be constructed with arguments.

## 11. Attributes and metadata expressions

Doria implements typed declaration attributes using adjacent `#[...]` syntax:

```doria
#[Attribute]
class Route
{
    function __construct(
        string $path,
        HttpMethod $method = HttpMethod::Get,
    ) {
    }
}

#[Route(path: "/posts", method: HttpMethod::Post)]
#[Test]
function createPost(): void
{
}
```

Ordinary `# comment` and `# [Route]` text remain line comments. An attribute
group may contain several comma-separated applications and may use a trailing
comma. Several groups may precede one target. Groups must appear before all
declaration modifiers.

Attributes are accepted on global type/function/constant declarations, class
and trait members, callable parameters, enum cases, and enum payload fields.
They are not accepted on statements, locals, expressions, closures, generic
parameters, return types, or throws entries. A promoted constructor parameter
is one authored target with parameter and promoted-property roles.

`#[Attribute]` marks a non-generic class as an attribute schema. Its constructor
parameters define the schema and must be readonly metadata-compatible values.
`#[Test]` and `#[PHPExport]` are compiler-known zero-argument metadata markers.
Stage 32 runs no tests and exports no PHP bridge.

Attribute applications resolve through the ordinary package graph and reuse
the named/default argument binder. Arguments are type-checked and evaluated by
the bounded constant evaluator. Supported metadata includes exact numeric
types, `bool`, `string`, compatible nullable values, constants, and compatible
unit, backed, or payload enums. Runtime calls, I/O, constructors, static
factories, closures, objects, collections, typed arrays, `Bytes`, `mixed`, and
shared handles are not metadata values.

Applying an attribute executes no constructor or other Doria code. Typed
metadata is retained in semantic information and HIR, while MIR and every
runtime backend remain metadata-free. Doria has no runtime attribute reflection.

`doriac metadata` and `doriac metadata --build-plan` emit deterministic strict
schema-version-1 JSON. Schema version 2 additively carries callable facts, and
schema version 3 additively unifies compiler-owned `#[Test]` and behavioral test
suites/tests. Schemas 1 and 2 remain exact. The compiler also defines a strict versioned processor
request/response protocol, but Stage 32 executes no processor and writes no
generated source. Baton orchestration begins in Stage 33; PHP bridge semantics
remain Stage 41 work.

### Native behavioral test declarations

In development and generated-development source, exact compiler-known imports
from `Doria\Std\Test` expose `describe`, `it`, and `test`. These call-shaped
forms are declarations, not runtime functions: `describe` contains nested test
declarations, while `it` and `test` produce deterministic compiler-generated
callables. Descriptions are const-evaluable strings, the compiler applies
ordinary source/package scope, and no runtime registration or source parsing in
Baton exists. `#[Test]` remains the lower-level function-oriented form, and both
forms enter one schema-version-3 test table consumed by Baton.

The fluent `expect`/`fail` surface, `AssertionError`, and the `TestAssertion`
effect are not implemented in Slice 1. They remain the accepted Slice 2
boundary and produce the single stage-named diagnostic until that slice lands.
Decision 0129 is authoritative.

Decision 0125 is authoritative. See
`docs/attribute-metadata-protocol.md` for the protocol reference and
`docs/executable-initializers-and-attributes.md` for the separate property
initializer model and future richer metadata-expression direction.

## 12. PHP interop and migration

Doria may support two separate PHP-related directions:

```text
1. Doria -> PHP compatibility/debugging backend.
2. PHP -> Doria migration converter.
```

Both are optional adoption and tooling aids. Neither is the core correctness target for the language.

A PHP-to-Doria converter may eventually help migrate existing PHP codebases into Doria, but it must remain architecturally separate from the Doria parser and core compiler semantics.

Recommended future shape:

```bash
doriac migrate php src --out migrated
```

The converter should initially produce conservative valid Doria, not perfect idiomatic Doria. It should use diagnostics for unsupported dynamic PHP features rather than pretending every valid PHP program can be automatically converted safely.

Doria should avoid promising full bidirectional PHP/Doria compatibility.

See `docs/php-interop-and-migration.md` for the detailed design notes.

## 13. Doria IR and backend behavior

Doria IR is the checked compiler-owned representation of a Doria program. After semantic analysis, type checking, and readonly/writable checking, the compiler lowers the checked AST into Doria IR before backend output.

As native code generation matures, Doria IR may lower into a simpler native-oriented IR for control flow, memory layout, runtime calls, and backend code generation.

MIR is Doria's native-oriented, backend-independent control-flow representation for the executable subset. It contains typed scalar, string, nullable-string, and class locals, parameters, calls and returns; class allocation, compiler-known property initialization/load/store, explicit ownership transfer and drops; method identities with explicit receiver operands and receiver modes; static data operations; runtime string literal/local/call/concatenation/display expressions; string comparison; basic blocks; checked numeric operations/conversions; and panic termination. Constants are typed and evaluated before MIR, so consumers receive folded values rather than a second evaluator. The debug interpreter uses safe private string and class values, an explicit heap-backed Doria frame stack, per-program static storage, and exact stdout/stderr buffers. It models source value and lifetime behavior, not native pointer/refcount layout. Ordinary interpretation has no fixed execution-fuel or call-depth cap and does not reject repeated states.

Native is the primary target. Checked HIR lowers to typed MIR, shared MIR validation gates both native lowerers, Cranelift emits the default fast object, LLVM 18 emits the O3 `--release` object, and the host linker combines either object with `doria-rt`. Native compilation has no interpreter preflight, fallback IR, or release-to-fast fallback. `doria-rt` owns entry policy, headerless class payload allocation/free, typed-array/collection/byte-buffer/mixed-box/shared-control storage, immutable refcounted runtime strings, text and binary I/O, formatting support, exact stdout/stderr writes, abort-only panic formatting, stack traversal, and status 101. Both lowerers share scalar, opaque string, pointer-sized class/collection/`Bytes`/`mixed`/shared-handle, and Decision 0093 nullable ABI conventions. Normal cleanup drops still-owned class, collection, mixed, strong-reference, weak-reference, access-object, and nullable-strong locals and statement temporaries on fallthrough, `return`, `break`, and `continue`; invokes `__destruct` before reverse-order owned-property/element cleanup; releases writable-family access registrations before their strong claims; and frees the relevant payload or control block last. Ordinary instance/static calls preserve those obligations. Owning class, collection, mixed, shared-handle, access-object, nullable-class, nullable-mixed, and nullable-strong returns transfer ownership, while Decision 0089 returned-borrow elision preserves an inferred readonly or writable alias to `$this` or exactly one borrowed parameter; only explicit `take` parameters and collection ingestion consume move arguments. Copy-type statics are private compiler-generated data symbols; compile-time string statics use an immortal private runtime representation and remain Copy at the Doria surface. Ownership transfer suppresses source cleanup, assignment acquires the replacement before dropping the old value, and abort-only panic runs no cleanup. Constructor definite initialization follows Decision 0090: semantic dataflow checks every reachable normal path, and MIR validation independently rejects incomplete or multiply initialized readonly property state before either native backend runs. Runtime failures use the shared panic path, except that an ordinary program write to a closed stdout or stderr pipe exits cleanly with status 0 under Decision 0091; panic reporting remains fatal even when its stderr sink is unavailable. Only canonical int/void entry results cross the process boundary. Unsupported coverage remains for scalar/string writable-shared payload access, shared handles through `mixed`, dynamic dispatch, and general interfaces.

The PHP backend is currently implemented as a compatibility/debugging backend. It emits `<?php` and lowers Doria-only syntax away:

- `let` is removed.
- `writable` is removed.
- `internal` is enforced by Doria before backend emission and may lower to PHP `private` or another backend-specific representation.
- Typed arrays and collection aliases are emitted as `array` for the current PHP backend only.
- Doria readonly/writable rules are enforced before Doria IR lowering and backend emission, not at PHP runtime.
- `int`/`int64` remain the exact supported signed-integer alias subset.
- Checked arithmetic, nondefault widths, unsigned semantics, division/remainder, shifts/bitwise operations, and integer companion conversions produce a clear backend unsupported-feature diagnostic whenever PHP cannot preserve the Doria behavior exactly.

For Doria features that PHP cannot express directly, such as object construction in property initializers or richer attribute expressions, the PHP backend should lower to equivalent generated PHP where practical or produce a clear unsupported-feature diagnostic temporarily. PHP limitations must not define Doria semantics.

Backend-specific tests are useful, but the PHP backend must not be the required proof that a language feature is correct. Correctness belongs to the parser, semantic checker, Doria IR, and eventually the native/backend-independent execution path.

## 14. Future features

Future work includes:

- Better diagnostics with suggestions.
- Full type inference for lists and dictionaries.
- Interface and trait execution semantics.
- Class inheritance through `extends`, interface conformance through `implements`, and class-body/trait-body `uses` trait composition.
- Multi-file package-graph resolution for imports and qualified names.
- Resolution of accepted `include` syntax as required include-once compile-time source inclusion.
- `declare` as structured compiler/source directives.
- Attribute syntax and metadata representation.
- Richer instance property initializers.
- Named arguments.
- Any future labeled or numeric loop-control surface. Base `do ... while`,
  `given`, `when`, control-flow `finally`, and `match` are implemented.
- Careful evaluation of `goto`, labeled loop control, and structured conditional compilation without adopting C/C++ textual macros.
- Async/await and structured concurrency.
- The decision-0110 hosted stream/file foundation and the later terminal APIs beyond the existing text and binary intrinsics.
- Self-hosting path for writing more of `doriac` in Doria.
- PHP-to-Doria migration tooling.
- Package management.
