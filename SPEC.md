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

The current compiler implementation lowers the accepted native subset through validated typed MIR. The debug interpreter, default Cranelift fast profile, and `--release` LLVM profile consume that same MIR, and the durable executable parity manifest compares exact stdin-driven stdout bytes, stderr bytes, process status, declared file side effects, and class lifetime behavior across all three paths. The supported subset includes top-level free functions; int/void `main` with either no parameters or one readonly borrowed `List<string>` argument; structured control flow and recursion; fixed-width numerics and bool; const-evaluable defaults for Copy scalars and readonly strings; immutable UTF-8 strings; expression interpolation; general nullable scalar, string, concrete-class, and `mixed` values with flow narrowing, `??`, `?->`, and exact `is`; checked formatting; UTF-8 line/file I/O; exact stdout/stderr; fatal panic; native classes, methods, statics, constants, complete `internal` checking, deterministic ownership, and compile-time borrowing; Stage 23 collections, typed arrays, `Bytes`, binary I/O, and boxed `mixed`; Stage 24 generic functions and methods; Stage 25 generic classes; and both Stage 25a shared-ownership families. The readonly `SharedReference<T>` / `WeakReference<T>` family supports class allocations, including interface views. The writable family supports class, generic-class, typed-array, `List<T>`, `Dictionary<K, V>`, `Set<T>`, and `Bytes` payloads through `WritableSharedReference<T>` / `WritableWeakReference<T>` and owned readonly/writable access objects. It includes nullable weak acquisition, lazy nullable operations, generic/property/collection storage, deterministic strong/weak/access release, and one per-allocation many-readonly-XOR-one-writable access state. `SharedReference<T>::referencedValue` is a readonly, allocation-free, refcount-neutral collision projection available only on that wrapper. Native strings are private non-atomic refcounted buffers and are Copy at the source level. Concrete closed-class, collection, `Bytes`, and `mixed` handles occupy one word; open-class, interface, and shared-interface carriers occupy two. Their payload layouts are compiler-known. Shared-reference control blocks are separate from unchanged payload layouts; the final strong release destroys the payload once, while weak references may keep only the control block alive. Shared-access conflicts use P1501 with a typed `conflictReason` that distinguishes all three incompatible access states. `main(): int` crosses the accepted `0..125` process boundary and `main(): void` maps normal completion to status `0`. Release optimization does not change observable semantics. `doria-rt` owns process entry, class/collection/byte-buffer/mixed-box/shared-control allocation and free, runtime strings, raw standard-device I/O, line discipline, text/binary file I/O, exact output, and the private source-aware runtime-outcome transport and standalone `Call Path` projection. Scalar/string writable-shared payload access and shared handles through `mixed` remain unsupported. Stage 35 Slice 2 executes general interface values, Error subinterfaces, Displayable erasure, and interface payloads across all six shared families. The former Stage 7-10 native smoke module remains retired.

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

The accepted namespace, import, include, and directive direction is recorded in `docs/decisions/0028-namespaces-use-include-and-directives.md`. Decision 0117 defines compile-time autoloading, hybrid strict source layout, package compilation graphs, and the Baton-to-compiler build-plan boundary. Decision 0118 defines the package manifest, dependencies, lockfile, workspace, processor, cache, and offline model. Decision 0126 fixes schema-2 local/scoped package identity, binary/library targets, target selection, deterministic source discovery, and target-scoped plans and receipts. Decision 0127 fixes the implemented normal path/Git dependency resolver, SemVer validation, one-version graph, strict deterministic lockfile, dependency commands, global Git cache, offline policy, multi-package plans, and receipt identities. Decision 0128 fixes and implements canonical dependency source descriptors, workspaces, development graphs, tests, processors, generated-source ownership, graph inspection, and project inventory. Decision 0124 fixes implementation ownership without changing those semantics: Stage 33 validates the Baton product contract in the disposable PHP UX bootstrap, and a mandatory Pre-Stage-45 transition parity-ports it to the clean Doria-native `dorialang/baton` repository before the unsuffixed `2026.03.1` release. Stage 31 implements namespace and import syntax, the edition-2026 prelude, canonical package-owned global identities, compiler-facing edition/package/source context, versioned build plans, complete multi-file indexing, compile-time include resolution, package visibility, and strict source layout. All three Stage 33 slices and Phase F are complete. Native Testing Foundation Slices 1 through 3 are complete, the foundation is complete, and Stage 34 single class inheritance is complete. The Pre-Stage-45 Doria-native Baton transition remains scheduled.

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
foreach (0..10 as int $i) {
    echo $i;
}

foreach (0..<10 as int $i) {
    echo $i;
}
```

`0..10` produces `0` through `10`. `0..<10` produces `0` through `9`. Range endpoints must be `int` expressions. Every `foreach` binding requires an explicit type. The variable after `as` is a readonly loop-local binding for each iteration and does not leak outside the `foreach` body.

The optional first binding in the PHP-shaped two-binding form has a checked role:

```doria
foreach ($items as int $index => Item $item) {
    echo "{$index}: {$item}\n";
}
```

For `List<T>` and `T[]`, it is a readonly canonical-`int` sequence index starting
at zero and advancing once per yielded element. For `Dictionary<K, V>` and
`SortedDictionary<K, V>`, it is the actual readonly key of type `K`; dictionary
ordering remains insertion or ascending-key order respectively. Integer ranges,
sets, sorted sets, deques, and dictionary key/value projections are value-only.
`PriorityQueue<T>` and `Bytes` are not iterable. A property-rooted sequence is
evaluated and borrowed once, not moved or copied. Writable value iteration
retains its existing exclusive-access rules; a first binding is never writable.
Decisions 0132 and 0133 are authoritative. Decision 0132 defines iterable roles;
Decision 0133 requires an explicit authored type on every foreach binding.

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

Decision 0136 permits independent borrowing of disjoint stored owned-property
paths, including nested paths and aliases. The same field and its ancestors or
descendants overlap; moving, replacing, or destroying a parent conflicts with
any live field loan. Whole-object receivers still borrow the whole object.
Computed or borrowed projections, collection indices, and byte ranges do not
establish independent storage. Writable-path permission and loan extent remain
unchanged; returned and retained loans preserve their physical source paths
separately from their owning lifetime roots.

Readonly controls mutation, not ownership transfer: a readonly class binding may be moved from. Assigning a new owner to that moved-from binding is mutation and therefore requires `writable`. An independently owned value may initialize an owning instance property or replace an initialized writable owning property. A taken source-carrying owner may also initialize the fresh constructor receiver's owning property when the new owner forwards all inherited cleanup loans; this does not permit borrowed fields or storage in an independently surviving receiver. Replacement evaluates and acquires the new value before destroying the old value; checked failure leaves the old property unchanged. Borrowed values, self-moves, and overlapping transfers are rejected. Moving a value out of a property remains separate because Doria has no accepted property-hole or take-and-replace operation.

Every parameter in Doria source has an explicit type. This applies to all function-like parameter lists: free functions, methods, constructors, anonymous functions, arrow functions, interface requirements, trait requirements, property hook setters, and future callback-style declarations. Doria does not infer omitted parameter types in any context.

Decision 0089 permits inferred locals to retain returned borrows. Plain `let`
is readonly; `let writable` requires a proved writable borrow and retains its
exclusive source access until last use. A readonly result cannot be upgraded.
Functions, methods, getters, generic/erased calls and alias chains follow the
same rule. Reborrowing suspends conflicting parent-alias access without copying
ownership. Borrowed locals cannot consume their source, initialize owning
storage from the borrow, or outlive it; ordinary borrowing adds no allocation
or runtime access guard.

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

Stage 30g adds `map`, preserving `filter`, and writable-accumulator
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
accumulator exactly once and leaves the source list unchanged. Stage 35 Slice 3
widens preserving `filter` to Copy-or-Cloneable elements: selected Move elements
are cloned once, in traversal order, through the checked nominal contract.

### Current compiler support

The compiler resolves readonly/writable/once structural function types into
canonical semantic identities, including parameter ownership, return-borrow
provenance, and normalized checked effects. It checks closure bodies, explicit
capture lists, `$this` capture, inferred invocation modes and effects, nullable
callable narrowing, callable-value calls, and callable properties. Source type
grouping remains transparent and does not create a tuple type.

Owned-result retention in a callback annotation is a permitted upper bound.
For example, `function(take Cursor): ?Cursor` accepts both forwarding its
consumed input and consuming it before returning loan-free `null`. Known body
proofs preserve actual loans without inventing them for loan-free results;
opaque calls preserve the declared bound. Returned borrowed aliases remain
distinct and retain their exact provenance and ownership contract.

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

Decision 0088 accepts `consuming function` for a method that takes ownership of
its instance; subsequent use of the old owning binding is use-after-move. The
modifier describes the receiver, not the return type or explicit parameters.
`take` parameters and `function once(...)` closure types keep their existing
spellings. Stage 36a implements consuming-method grammar, ownership, dispatch,
and cleanup; the earlier spelling amendment did not itself provide execution.

A consuming receiver is owned but readonly, matching an explicit `take`
parameter. It does not grant writable access; `writable` and `consuming` are
mutually exclusive receiver modifiers. Ordinary ownership transfer to an
explicitly writable local remains available. Borrowed receivers cannot be
consumed, and moving an owner with a live borrow remains invalid.

Consumption forwards an owned carrier's existing retained source loans into
its returned carrier; it never permits lending storage owned by the consumed
receiver. Generic calls and interface erasure preserve those source loans until
the resulting carrier releases them.

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

`parent::member()` directly names the immediate parent implementation or
accessible static member. Instance calls use the current complete-object
pointer and bypass virtual dispatch. `parent::__construct(...)` is the one
lifecycle protocol form; `parent::__destruct()` is invalid. Trait-local
`self::member` resolves to the composing class during compile-time trait
composition under Decision 0134. `static::` is permanently rejected with a fix to `self::`;
Doria has no late static binding.

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

Decision 0135 accepts property hooks with automatic backing storage, computed
values, explicitly writable setters and getters, and interface/trait/override
integration. Direct same-property `$this` access inside a hook uses its backing
field; outside access invokes the accessor. Declaration initializers fill
backing storage without calling the setter; purely computed hooks have no slot.
An overriding backed hook reuses its inherited field. Parent-then-child
declaration initializers each run once per object. The child replaces the
inherited value after successfully evaluating its initializer, then destroys
the old owned value; a failed initializer leaves the old value for ordinary
failed-construction cleanup. An override without an initializer keeps the
inherited value. A setter's explicitly
declared input type must exactly match the property's resolved type.
`borrowed get` declares a borrowed result; plain interface `get` requires an
owned result for a Move type. Concrete bodies retain returned-borrow inference
and must satisfy declared contracts. `writable borrowed get` combines writable
receiver access with a borrowed result; these are separate permissions.
A plain interface getter may return a newly owned callback borrowing `$this`;
the callback owns its environment, while its receiver loan follows ordinary
non-lexical last-use rules. If cleanup of an owned capture can observe a borrowed
source, that source must instead remain valid through that cleanup. Ending a
loan does not move environment destruction earlier or change reverse cleanup
order. Returning an existing stored callback through `borrowed get` lends that
environment instead.

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

Property hooks are the escape hatch when a property-shaped API needs validation, computed behavior, lazy decoding, caching, normalization, or guarded access. The public member should remain property-shaped when it is conceptually a value.

Use methods for actions, commands, mutation, I/O, async work, and behavior with meaningful work:

```doria
await $message->acknowledge();
await $message->retryAfter(seconds: 30);
$report->renderPdf();
```

If a data-returning operation must be a method because it performs I/O, expensive work, decoding, or another explicit operation, use an unmistakable verb such as `loadBody()`, `decodeBody()`, `findById()`, or `fetchProfile()`.

Fallibility alone does not require a method. The accepted Stage 36 property-hook
direction permits checked `throws` and side effects, but not blocking or async
work in v1.0. Property-shaped access does not promise purity; complete accessor
contracts remain owned by Stage 36.
Potentially blocking output is included, and the restriction applies through
helpers and callable dispatch as well as direct operations.

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

Generic classes are instantiated in type positions and construction expressions with concrete type arguments, including nested forms such as `List<Box<int>>`. Every instantiation is a distinct monomorphized type with specialized field layout, methods, and drop glue. A user-defined class remains an owned move type regardless of its arguments; substituting only Copy fields does not turn it into a Copy aggregate. Compiler-known inline Copy types, including `Doria\Std\Io\RegistrationKey<T>`, retain their specified value classification and exact generic identity.

Constraint declarations use `<T implements A, B>`, and constraints may themselves be generic. Compiler-known `Comparable`, `Hashable`, `Equatable`, and `Displayable` constraints are checked at instantiation without boxing primitives. Stage 35 Slice 1 implements nominal user-interface constraints and statically specializes their concrete method calls. Type parameters are invariant. Doria v1.0 has no default type arguments, compile-time value arguments, call-site turbofish, or runtime generic reflection.

Owner and method type parameters retain distinct identities through inherited
contract comparison, call inference and member specialization. A descendant's
parameter may reuse an inherited method parameter's spelling without merging
their types. A method still cannot shadow its own declaring class's parameter.
Generic method signatures bind the authored template with owner and method
arguments together; owner replacements are not rebound as method parameters.

Nullable types do not inherit `Hashable` or `Comparable` conformance from their
payload. Hash collections, sorted collections, and priority queues require the
key or element type itself to satisfy the applicable contract. Nullable equality
and preserving duplication have explicit absence handling, not a null hash or
null ordering.

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

### Inline Time values

`Doria\Std\Time\Duration` and `Doria\Std\Time\Deadline` are distinct
compiler-known inline Copy values (Decision 0110). Assignment and calls preserve
the original without heap-class allocation, ownership transfer, or destruction.
Nullable forms remain Copy. This does not introduce user-defined Copy classes,
integer conversions, a public clock epoch, or an implicit conversion between
the two Time types. `Duration` represents signed `int64` nanoseconds, including
negative intervals. `Duration::fromNanoseconds(int64 $nanoseconds): Duration`
and the readonly `int64 $nanoseconds` getter preserve that exact value. A negative
interval used for a timed wait or relative deadline is already expired, not
invalid, an immediate poll, or an indefinite wait. It does not change the
Duration value or cancel transfers when only a completion-fetch wait expires.
Deadline preserves its exact instant after expiry, including distinct past
instants. Expiry does not replace its value with a marker. Static
`Deadline::fromNow(Duration $duration): Deadline throws ClockError` samples
the monotonic clock once and adds the exact signed interval. Clock-read failure
preserves its optional system code through the ordinary checked Error contract.
Success requires no timer or readiness infrastructure. Remaining Time operations
and arithmetic declarations are owned by the bounded Time appendix.
`Doria\Std\Io\Wait` is an ordinary inline Copy enum with `Immediate`,
`For(Duration $duration)`, `Until(Deadline $deadline)`, and `Indefinite` cases.
The reusable transfer owner is `Doria\Std\Io\TransferGroup`. Its readonly
`observedReadKey: ?ReadTransferKey` and `observedWriteKey: ?WriteTransferKey`
properties expose only the fetched operation's direction, before propagating
its original checked error. Both are null before the first fetch and after a
timeout or exhaustion observation. These are the original consumed plan's
operation identities, preserved across resubmission; copies retain no loans.
Readonly `getReadRegion(ReadTransferKey $key): ?ByteRegion throws IoError`
returns null for an active transfer or unknown completed byte range;
a present empty region for known zero-byte completion, including EOF.
Invalid or foreign keys throw `IoError`, not null, using `IoOperation::Access`,
`IoTarget::TransferGroup`, and `IoErrorReason::InvalidInput`, with no path,
system code or byte progress. The readonly view exposes
only the completed prefix, preserves the group loan, and prevents conflicting
mutation or access after the owner dies. Lookup performs no I/O or allocation;
resubmission makes the previous prefix unavailable.
The approved reuse methods are `resubmitRead` and `resubmitWrite`. They select
the original directional operation key and a checked range of its retained
buffer, preserving the key and the existing retirement and exclusivity rules.
They create no new operation; `begin` submits the initial transfers.
Invalid resubmission keys or ranges throw existing `IoError` with
`IoOperation::Configure`, `IoTarget::TransferGroup`, and
`IoErrorReason::InvalidInput`, without a path, system code or byte progress.
Failed validation submits no work and consumes no stored transfer Error;
actual transfer failures still propagate unchanged at `getNextCompletion`.
Completion observation uses `getNextCompletion(Wait $wait): TransferWaitResult`. A relative wait
freezes one deadline at observation start; retries do not restart it. Expiry
stops only the wait, preserving transfers for subsequent observation.
`Doria\Std\Io\TransferWaitResult` separates `Completed(TransferCompletion
$completion)`, `TimedOut`, and `Exhausted`. `TransferCompletion` contains only
`Read(ReadTransferKey $key, ?ReadOutcome $outcome)` and
`Write(WriteTransferKey $key, ?WriteOutcome $outcome)`. Both are ordinary inline
Copy enums with no retained resources. An actual completion with a null outcome
is distinct from no completion; zero-byte success and would-block remain actual
outcomes. Failed fetches still throw the original owned checked Error. Neither
enum grants buffer access or retires work; timeout and exhaustion observations
do not permanently prevent resubmission. Public group execution remains
in-progress Stage 36a work.

Writable `ReadinessSet::begin` takes an owned `TransferPlan` plus optional
`?Doria\\Std\\Time\\Deadline $deadline = null` and
`?Doria\\Std\\Concurrency\\CancellationToken $cancellation = null` parameters.
Null omits that group-lifetime control; no `TransferControl` type is introduced.
The group retains shared cancellation state without borrowing its source and
preserves the supplied exact deadline. A later group using the same readiness
set does not inherit these controls. Cancellation or group-deadline expiry
cancels and drains outstanding transfers before releasing storage, distinct
from the completion-fetch budget whose expiry ends only the wait. Executable
control integration remains Stage 36a work.

### Dynamic boundary type

`mixed` is Doria's only dynamic type. It has three laws:

1. `mixed` is unknown-flavored, never any-flavored. A `mixed` value permits no property access, method calls, arithmetic, concatenation, interpolation, comparison, or other typed operation until it is narrowed. Exact `is` and exact type-binding `match` patterns provide narrowing.
2. Any value may flow into `mixed` implicitly. This is the deliberate dynamic-boundary exemption from the no-implicit-conversion rule. Values do not flow out of `mixed` implicitly; source must narrow first. There is no cast spelling.
3. `mixed` is a boxed, runtime-tagged move type, always, even when the payload is a Copy value.

An owning binding such as `mixed $item = $items[$index]` acquires an owned mixed
box and payload claim without removing the collection element. This accepted
mixed-box materialization does not make the type Copy; ordinary borrowed indexed
uses still acquire no ownership (Decision 0100).

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

Const-evaluable parameter defaults for nullable Copy scalars, Copy enums, and
compiler-known inline Copy values
use ordinary caller-side default materialization. `null` preserves absence;
a present constant preserves its exact declared type. Authored and
compiler-known callables follow the same rule, including writable Copy
parameters and named omissions. Nullable string and move/take defaults remain
separate implementation boundaries under Decision 0086.
An absent managed Copy token holds no ownership claim.

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

For class targets, `is` and match type patterns test whether the dynamic class
is the target or a descendant. The narrowed value preserves ownership, borrow
mode, provenance, and the existing non-null proof. Interface conformance tests
use the checked nominal interface graph and static metadata from Decision 0134.
Primitive, string, enum, and closed non-hierarchy
tests retain their exact Stage 22 behavior.

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
patterns. Decision 0114's approved generic-enum amendment assigns general
monomorphized payload enums to Stage 36a; execution is still pending. It applies
the same layout, ownership and cleanup rules after substitution, preserving
retained source loans through case selection. Decision 0114 defines the enum
model; decision 0115 defines match semantics and ownership.

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
produces an empty sequence. Copy values duplicate their complete representation
and retain any required managed claims; immutable strings share their data
through retained handles. Move elements require the nominal `Cloneable` contract
and are cloned once per output slot in ascending order. Nullable absence is copied
without calling `clone`; a present value uses its underlying duplication plan.
The repeat form is rejected for `Set` and `Dictionary`.

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

Interpolated values may be `string`, a fixed-width integer, `float`, `bool`, or an explicitly conforming `Displayable` class. They use the same canonical display conversion as `echo`, `.`, and Stage 17 `%s`. Interpolation, string-anchored concatenation, and `%s` materialize an ordinary `string` that may be assigned, returned, passed to a `string` parameter, stored in `List<string>` or `string[]`, nested, and echoed later. Direct scalar-to-string assignment or ordinary argument conversion remains invalid. Null, nullable values without a non-null proof, `mixed`, typed arrays, `List<T>`, `Dictionary<K, V>`, and `Set<T>` are rejected.

`Displayable` uses the general interface runtime from Decision 0134 Slice 2,
while concrete and constrained calls retain their direct paths:

```doria
class Label implements Displayable
{
    function toString(): string
    {
        return "Doria";
    }
}
```

Conformance requires the explicit `implements Displayable` declaration and exactly an externally accessible readonly instance `function toString(): string` with no parameters. Method-name coincidence does not conform, and Doria has no `__toString` magic method. Display conversion is limited to interpolation, `echo`, `.`, and `%s`; it does not permit implicit class-to-string assignment. For a statically known concrete class, the interpreter, Cranelift, LLVM, and PHP compatibility backend execute conversion through the ordinary `toString()` method machinery exactly once and left-to-right. Interface-typed values use the checked vtable entry through the general interface runtime, preserving the same evaluation and ownership rules.

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

The owned file family is `Doria\Std\Io\ReadableFile`, `WritableFile`, and
`DuplexFile`. The `File` companion factories `openRead`, `openWrite`, and
`openReadWrite` return those respective concrete handles. This is not a
`File<Capability>` generic family. Writable opens require an explicit
`FileCreation` (`OpenExisting`, `OpenOrCreate`, `CreateNew`, `TruncateExisting`,
or `CreateOrTruncate`), with no default; read opens neither create nor truncate.
Append uses typed `FileWriteMode::Append`, separately from creation. Each owned
file's `consuming function close(): void throws IoError` consumes the receiver
on success and checked failure. `IoOperation::Close` follows existing cases;
close failure does not introduce a separate Error or cause destruction to retry
close. The approved seek capability is `SeekableStream::seek(int $offset,
SeekOrigin $origin): int throws IoError`, with writable access and required
Start/Current/End origin. All three owned file types implement it. Seek errors
use appended `IoOperation::Seek`; readonly inspection uses `Access`, preserving
opening-path targets and null byte progress. Readonly `getPosition(): int throws
IoError` and `getLength(): int throws IoError` methods replace the proposed
position/length properties. Fresh inspection may block; the global nonblocking
hook rule remains unchanged and applies through helpers. There are no property
aliases. WritableFile and DuplexFile expose writable
`synchronizeData(): void throws IoError` and
`synchronizeMetadata(): void throws IoError`, requesting durable data and full
data-plus-metadata respectively. Neither is buffer flushing or alters the file
cursor/length/bytes. Failures use appended `IoOperation::SynchronizeData` and
`IoOperation::SynchronizeMetadata`, retaining opening-path targets, optional
system codes and null progress. Platforms may strengthen, never weaken these
guarantees. Both calls may block and remain forbidden in property hooks through
direct or helper calls; ReadableFile exposes neither method.
WritableFile and DuplexFile also expose writable
`setLength(int $length): void throws IoError`: required nonnegative byte length,
tail-discarding shrink or zero-readable growth, unchanged cursor and retained
ownership. Sparse-file support and current-EOF append behavior are preserved.
Errors use appended `IoOperation::Resize`, opening-path File targets, optional
system codes and null progress; negative lengths are InvalidInput. The operation
may block and remains forbidden in property hooks through helpers. ReadableFile
has no resizing method, and no truncate alias or interface expansion is added.
Remaining controls and public execution belong to the in-progress
Decision 0110 appendix and Stage 36a work.

Decision 0110's reusable-storage appendix accepts `Bytes::allocate(int $length)`
and bounded `Doria\Std\Io\ByteRegion` / `WritableByteRegion` views. Allocation
creates one zero-initialized owned buffer; region formation copies no bytes,
allocates no object, and does not extend storage lifetime. Readonly regions are
Copy carriers; writable regions are move carriers with exclusive source access.
Both expose readonly `length` and bounds-checked `uint8` indexing; only writable
regions permit mutation through a writable binding. Returning, copying, or
capturing a region preserves its source loan, including through callable values.
Nullable byte containers and regions support ordinary narrowing and null-safe
`?->` member access, which preserves absence and the same borrowing restrictions.
Decision 0089's single-source return rule applies to named region helpers;
Decision 0121's single-argument closure rule and deferred captured-environment
returns remain unchanged. These additions belong to the in-progress
Stage 36a implementation, not the completed whole-file intrinsic tier.

Primitive reads distinguish data, would-block, EOF, and timeout; empty data is
not EOF. Primitive writes preserve partial progress and distinguish would-block.
Decision 0110 names the `Doria\Std\Io` capabilities `ReadableStream`,
`WritableStream`, `DuplexStream`, and `FlushableStream`. `read(writable
WritableByteRegion $destination): ReadOutcome` and `write(ByteRegion $source):
WriteOutcome` require writable receiver access and throw `IoError`; `flush():
FlushOutcome` is separately writable and throws `IoError`. Regions are borrowed only
for the call. `ReadOutcome` cases are `Data(int $byteCount)`, `WouldBlock`,
`EndOfStream`, and `TimedOut`; `WriteOutcome` cases are `Accepted(int
$byteCount)` and `WouldBlock`. These ordinary nominal Copy enums have inline
layout and never own the supplied storage. Successful counts are bounded by
the region length; an empty request reports zero progress, not EOF. Capability
interfaces do not expose close or imply mode control or durable storage.
`FlushOutcome` is an inline Copy enum with `Complete` and `Pending` cases.
`Pending` may follow partial drainage; the adapter retains the remainder and a
retry resumes it without repeating accepted bytes. Flush inserts no automatic
readiness wait or busy polling. Checked failures retain `IoError` transport.
Decision 0110 adds readonly `?int IoError.transferredByteCount`, defaulting to
`null` in existing construction. It distinguishes unavailable progress from
known zero or positive byte progress in the failing operation; it promises
neither rollback nor durable storage. Existing catches retain their identities.
One multi-stream readiness substrate, durations and absolute deadlines, and one
cancellation/backpressure model serve synchronous I/O, future async lowering,
networking, child-process pipes, and terminal input. Platform polling and handle
types never enter the Doria surface.
Persistent readiness sets own registered streams: registration transfers
ownership, access goes through the registration, and removal returns ownership.
Registration preserves the registered static type `T` and the complete value;
a typed key identifies the registration without owning its stream. Removal
returns owned `T`, without automatic capability erasure or runtime type discovery.
Intentionally registering an erased interface preserves that interface type,
not a recoverable hidden concrete type. Failed registration follows ordinary
`take` cleanup and does not restore the caller's moved binding.
Destroying a set cleans up streams that remain registered. A temporary
one-stream wait may instead borrow for that call; explicit borrowed adapters
keep their separate source-lifetime rules.
Custom adapters expose an opaque borrowed readiness source through the nominal
`Doria\Std\Io\ReadinessStream` capability and its
`ReadinessSource $readinessSource { borrowed get; }` property. Source loans obey
ordinary receiver provenance and elision; they neither own nor extend a handle's
lifetime. Waiting uses the shared native core, while transfers dispatch through
the complete registered adapter. A transfer interface alone does not imply
readiness, and erased values must retain that capability statically. Source
formation does no I/O and requires no allocation or new close claim; native
association is lazy. No public operating-system handle or runtime capability
discovery is introduced.
`Doria\Std\Io\RegistrationKey<T>` is an opaque inline Copy value, including
nullable keys. Assignment, passing, and return leave the original usable;
copying allocates nothing and acquires no ownership or stream loan. The exact
registered static type `T` remains part of its nominal identity through shared
MIR checking. Keys do not implicitly convert between registered types or to
integers. Stale and foreign keys are rejected before registration access;
copies neither extend set/stream lifetimes nor bypass ordinary borrowing.
`ReadTransferKey` and `WriteTransferKey` are separate opaque inline Copy
identifiers for transfers, not registrations or integer indexes. They have
no public constructor, allocate nothing when copied, and do not convert to
each other or to registration keys.
`register` transfers the complete stream into the set and returns its typed
key; `unregister` returns the same owned `T`, making other key copies stale.
Both require writable set access and retain the checked `IoError` contract.
`getStream` lends the same registered `T` readonly; `getWritableStream`
requires writable set access and lends it writable. The set remains the owner
and loan source. Neither method removes, copies, or erases the stream, and live
loans prevent incompatible set mutation, group creation, or teardown.
Registration follows the complete adapter's current readiness source. Replacing
its underlying file preserves the registration and key; future waits and
transfers use the new source. An already-started operation retains its selected
source and native resources through retirement, rather than being retargeted
by a later replacement. Unchanged sources reuse native association. Ordinary
borrowing and group cleanup remain unchanged; the opaque readiness view gains
no independent ownership.
Concurrent pending transfers use one exclusively borrowing group, acquiring
the set and participating storage loans together. The original bindings stay
unavailable for the group's lifetime. Completed bytes may be processed and
their storage reused through the group while other operations remain pending.
Completion does not independently release an original binding, and registration
keys do not create disjoint ownership roots. Teardown cancels and drains pending
operations before releasing retained loans. This narrow carrier does not enable
general borrowed class fields; synchronous transfers remain call-scoped.
`TransferPlan::read` and consuming `andReadFrom` retain writable destination
loans; `TransferPlan::writeTo` and consuming `andWriteTo` retain readonly source
loans. Setup performs no I/O or byte copying. Mixed plans preserve both kinds
of loan, permitting shared write sources but rejecting overlapping reads.
Readonly `getReadKey(int $position)` and `getWriteKey(int $position)` retrieve
the directional Copy operation key at a zero-based mixed-plan position. Invalid
or wrong-direction positions throw `IoError` with operation `Access`, target
`TransferPlan`, reason `InvalidInput`, and no path, system code, or byte progress.
Key identities survive consuming
appends, distinguish repeated operations and separate plans, and retain no loan.
Competing operations on one stream follow plan order. Independent duplex
read/write directions may overlap; operations sharing a file cursor compete.
This orders access to stream bytes, not completion observation across
independent streams. Partial-transfer outcomes remain unchanged.
Completion-fetch budgets are separate from group deadlines. Expiry stops only
the wait; transfers remain active. A typed timeout observation is distinct from
a completion or exhausted group, without fabricating a transfer outcome or
checked error. Loans, identities, and actual progress remain available through
the group for another wait or resubmission of completed storage.
Cancellation or an explicit group deadline cancels and drains outstanding transfers.
Untimed waits require no clock sampling or per-wait allocation. Standalone
clock-read failures belong to `Doria\Std\Time\ClockError`; clock failures inside
readiness waits retain the accepted `IoError` contract. Both reuse the same
private platform clock and preserve its failure facts.
`ClockError` is an ordinary owned `Error` with readonly stored `string $message`
and `?int $systemCode`; its constructor defaults `systemCode` to `null`, not
zero. It requires qualification or an explicit import and has no I/O operation,
target, or transferred-byte field. Ordinary checked propagation and rethrow
preserve the original object and optional platform code.
Concurrent adapters require an explicit pending-transfer capability, checked
statically even after interface erasure. Readiness metadata alone is insufficient.
Preparation and completion run through the complete adapter using group-owned
stable request slots; ordinary synchronous method bodies are not suspended.
Completion runs exactly once after retirement, preserving adapter behavior and
actual progress that wins cancellation. The shared core cancels all pending
operations before draining any, without releasing storage early or allocating
per transfer. Ordinary synchronous adapters retain their existing contract.
Cancellation state is explicitly shared: a token may outlive its source, and
releasing the source does not implicitly request cancellation. One source may
allocate one shared state; token creation, checks, and chunk transfers require
no further allocation. `Doria\Std\Concurrency\CancellationToken` is inline Copy:
assignment preserves both bindings, and each independently owned copy retains
and later releases
one shared-state claim. Borrowed arguments do not acquire an ownership claim;
copying and cleanup do not request cancellation. Native waits observe persistent
wake notifications rather than polling. These approved contracts remain part
of the in-progress stage.
`==` compares shared cancellation-state identity and `!=` is its inverse.
Tokens from one source remain equal through copying, cancellation and source
destruction; distinct live states differ. Ordinary nullable equality applies.
Borrowed comparison acquires no claim and performs no allocation, signal or
clock work. This introduces no address, integer-conversion, ordering or hashing
contract.

Standard input, output, and error become first-class non-owning, nonclosable
inline Copy views over the same devices used by the intrinsics above.
`Doria\Std\Io\StandardStreams` exposes readonly static `input`, `output`,
and `error` properties returning `StandardInput`, `StandardOutput`, and
`StandardError` respectively in the same namespace. Access uses
`StandardStreams::output` after import. Decision 0110 records Andrew's S-A
approval. Public getters, transfers and readiness/pending groups pass targeted
macOS interpreter/Cranelift/LLVM/PHP functional validation. Native PHP I/O uses
the compiler-published shared runtime through FFI from the first input operation;
getter-only and userland-stream programs remain FFI-free. Linux/native Windows
execution and the complete allocation/performance gate remain Stage 36a work. Assignment
preserves both bindings without new handles or allocations; shared device access
and exclusive mode control remain coordinated. Copy-to-interface erasure creates
an independent owned snapshot. A borrowed return from an implicit erased
temporary lasts through its statement; storing the borrow requires declaring
the interface owner first. The original Copy binding remains independent
(Decision 0138, C-B). File locks support explicit
borrowed and owning guards over native platform locks with documented host
differences (Decision 0138). A borrowed guard does not close the caller's file.
Typed per-value
buffering and UTF-8 text adapters layer over byte streams, own their underlying
handle by default, and use explicit borrowing when they do not. Line and
delimiter operations are bounded. Typed file requests replace mode strings;
buffer flush, durable data synchronization, and full synchronization are
distinct. Owned child processes expose typed pipes and require concurrent,
bounded stdout/stderr drainage. Launch requires an explicit wait, terminate,
or detach cleanup policy with no default. The policy travels with ownership
and governs still-active children on normal and checked-error exits; destructor
cleanup remains best-effort and nonthrowing. Explicit operations observe checked
failures, and a timed-out wait retains ownership rather than terminating the
child. Abort-only panic runs no cleanup (Decision 0110).

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
Decision 0137 permits lazy, group-bounded native-only helpers for pending
transfers through inherited synchronous standard devices when native completion
or safe mode control is unavailable. Helpers preserve real handles and buffers;
adapter callbacks stay on the driving thread. Cancellation/drain may wait for
actual host-operation retirement before releasing loans. Ordinary synchronous
operations and idle watches do not start helpers.
Synchronous programs do not initialize async executors, task objects, or async
scheduler infrastructure. Stage 36a owns the initial cross-platform stream
performance and memory regression gate defined by decision 0110; Stage 43
continues and broadens the benchmark suite rather than postponing that gate.

Stage 36a is complete. The semantic and performance
contracts are accepted. The byte-transfer interfaces and outcome cases are
named above, together with standard-stream accessors and view types.
Exact remaining public interface, member, readiness, file, adapter, and process spellings
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

Named functions and methods require explicit return types; omission receives
E0617 during declaration checking. Their result types are not inferred from
bodies. Arrow result inference and returned-borrow provenance inference remain
separate supported analyses. Constructors
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
readonly, and catch-scoped. Concrete catches cover the named Error class and its
descendants (Decision 0130); `catch (Error)` catches every checked error.
Source order applies: a parent catch makes a later descendant catch unreachable.
Duplicate catches, catches after `Error`, and catches proven unable to match a protected effect are
unreachable. That proof uses the protected body's authored reach (Decision
0123). A call through a function value, an erased interface, a constrained
method, or a bodyless trait requirement contributes its complete contract, so
no caller, closure, or composer elsewhere decides a catch's reachability.
Catch bodies are independent: sibling catches do not handle an
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
failed construction phase runs no `__destruct` for that incomplete class;
initialized owned fields drop in reverse initialization order, followed by
completed ancestor phases in reverse construction order. Uninitialized fields
are ignored, the one allocation is freed once, and the error continues. Fatal
panic remains separate, non-catchable, cleanup-free, and status 101.

Stage 29 implements grammar, semantic checking, AST/HIR,
ownership, the two-word erased carrier, hidden first-throw origin storage,
checked MIR, the status/out-slot ABI, propagation, cleanup, exact catches,
rethrow, canonical `Doria\Std\Io` errors, checked I/O effects, and backend
transport. Handled checked errors execute through the interpreter, Cranelift,
LLVM, and the PHP compatibility backend. An Error escaping `main` performs
required cleanup, is reported as `Error[R1000]`, and exits with status 70. A
successful `main(): int` may independently return 70 without producing R1000.

The compiler-known I/O family contains `IoOperation` (`Open`, `Read`, `Write`,
`Append`, `Flush`, `Close`, `Register`, `Unregister`, `Wait`, `Configure`, `Access`, `Seek`), `IoTarget`
(`File(string $path)`, `StandardInput`, `StandardOutput`, `StandardError`,
`ReadinessSet`, `TransferPlan`, `TransferGroup`), `IoErrorReason` (`NotFound`,
`PermissionDenied`, `InvalidInput`, `Interrupted`, `ResourceExhausted`,
`Unsupported`, `Closed`, `Other`), and `Utf8InputSource` (`File(string $path)`,
`StandardInput`). `IoError` exposes readonly `message`, `operation`, `target`,
`reason`, `?int systemCode`, and `?int transferredByteCount`;
`transferredByteCount` is an optional trailing constructor argument defaulting
to `null`. Unknown progress stays `null`; zero and positive values are known
progress within the failing operation, not rollback or durability guarantees.
`InvalidUtf8Error` exposes readonly `message`,
`source`, `validByteCount`, and `?int invalidByteCount`. Counts are bytes.
Stable messages are Doria-owned and host-localized error prose is not exposed.
Readiness infrastructure uses `IoError` with `IoTarget::ReadinessSet` and the
matching `IoOperation::Register`, `IoOperation::Unregister`, or `IoOperation::Wait`
operation. Its byte progress is
`null`; original transfer errors and their progress are preserved, not rebuilt
as infrastructure failures. New cases append without changing existing tags.
Transfer-plan configuration failures use the same `IoError` with
`IoOperation::Configure`, `IoTarget::TransferPlan`, and `null` byte progress.
Configuration performs no transfer; native system-code presence is preserved.
Failed registered-stream lookup uses `IoOperation::Access`, `IoTarget::ReadinessSet`,
and `null` byte progress. Stale or foreign keys are rejected before stream access.
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

Decision 0134 defines generic nominal interfaces, multiple interface inheritance,
exact requirement compatibility, and two-word erased values. Requirements are
external bodyless instance methods terminated by `;`; they may be generic, may
use readonly or writable receivers, preserve parameter ownership and names, and
declare checked Errors. Parameter defaults, `internal`, `open`, `override`,
static members, constructors, destructors, constants, and method bodies are not
interface requirements. Stored properties are not user-interface requirements.
Decision 0135 extends the requirement graph with bodyless getter and setter
contracts, preserving each accessor's receiver mode, result ownership, exact
setter input type, and checked Errors. `Error` retains its narrow compiler-known
readonly stored `string $message` exception. A bodyless interface method or
accessor, or bodyless trait method, whose substituted return type is a region
or nullable region elides to a retained borrow of its readonly or writable
receiver (Decision 0134). An implementation may return a region its receiver
holds or one independent of it, and a region from a parameter does not conform;
callers keep the receiver borrowed for as long as the result lives.

Classes conform only through authored or inherited `implements`. Interface
values are `(data pointer, interface vtable pointer)` carriers over the original
headerless class allocation. Conversion and ancestor upcast do not allocate;
deliberate erasure uses one constant-slot indirect call while monomorphized
generic constraints remain static. `is` and match perform checked exact
specialization narrowing, and an open interface domain is never exhaustive.

Decision 0139 accepts local contextual `where (...)` clauses on class
conformance entries, class/trait methods (including static methods), and
computed properties. Conditions contain positive nominal bounds on in-scope
type parameters, for example `implements Ready where (T implements Ready)` or
`function inspect(): int where (T implements Ready) { ... }`. Multiple bounds
are conjunctive: `where (T implements Ready, Other, U implements Pending)`.
Method clauses follow the return/effect contract; property clauses follow the
property name and precede its hooks. `where` remains an ordinary identifier
elsewhere, and match guards continue to use `if`.

Stored state, initialization/destruction and interface requirements remain
unconditional. A guarded property has no backing storage or initializer. There
is one member identity, with no condition-selected overload bodies; an override
may widen availability but cannot strengthen the base condition. A false
condition provides no member or conformance witness. Existing ownership,
borrowed getters, static specialization and erased carriers remain exact.

The Stage 36a compiler grammar, source-preserving transforms, condition
resolution and guarded body/caller-condition checks are implemented. Conditions
retain lexical type-parameter
identities and canonical interface arguments, including their enclosing `self`
context. E0778 diagnoses subjects that are not in-scope type parameters; existing
declaration checks diagnose malformed interface arguments and member shapes.
The shared nominal proof kernel checks conditioned class edges under fixed
caller assumptions, with exact generic substitution and cycle handling. Concrete
generic constraints use these proofs independently of declaration order.
Unboxed primitive constraint evidence does not admit interface-valued transport.
`Displayable` conversion and generic bounds, `Error` admission and concrete
backend class facts use the same exact-type proofs. Declaration-name flags
cannot grant a capability to an inactive specialization. Compiler-generated
Error declarations carry their nominal edge origins through the shared
synthetic-declaration constructor. Nullable symbolic receivers proceed to
nominal member lookup; an unconstrained parameter gains no member. An `Error`
premise supplies the readonly `message` property in a guarded generic body.
Conformance discovery caches declaration-checked paths and predicates, while
availability uses a fresh caller environment. Exact authored duplicates are
diagnosed even in unused or inactive declarations; equal concrete substitution
can coalesce distinct source paths. Active inherited and interface diamonds
share one specialization, and conflicting active specializations receive E0752.
Iteration obtains no element type from an invalid view. Ancestor/direct
interface lists coalesce independently of source order; exact direct repeats
and unconditional parent-class redundancy remain errors. These checking-only
views expose no executable witness.
Formed source types, registered substitutions and published inferred/contextual
expression types check that view under frozen caller premises. Normalized
callable guards use the same checks for subjects and interface arguments after
receiver substitution. Concrete formations also follow unconditional stored
types through nested fields, inheritance, trait storage and backed properties,
without requiring member access. Computed properties remain outside storage
traversal. Pure substitution registers no executable specialization. Shared
E0539 cycle identity coalesces duplicate reports for one expanding component
while preserving separate findings for independent components.
An established conflict in a predicate argument discards its predicate.
These checks reject established conflicts; they do not prove
that an unresolved symbolic capability is absent. Callable-owned coherence
obligations follow generic callers and authored trait substitution, then resolve
against concrete nominal facts. Potential symbolic overlap alone is permitted;
conflicting concrete conformances remain errors under Decision 0139.
Declaration checks also prove conformance-to-implementation and inherited-to-
override guard implication under owner bounds. Generic method bounds retain
their distinct parameter identities; computed accessors share their property's
condition. E0755/E0729 identify stronger guards at their exact clauses with
related contract locations. These checks expose no active callable or witness.
Guarded callable bodies receive their own scoped nominal assumptions. Calls
prove their target's clause against frozen caller facts after argument/result
inference; E0779 identifies an unproved clause at the call/access with related
declaration locations. Unresolved generic arguments retain E0531. Ordinary
generic calls must prove nominal bounds for symbolic arguments too; unproved
bounds receive E0535 rather than waiting for a later concrete instantiation.
Nominal type formation applies these proofs to class/interface arguments,
parent types, trait uses and guarded signatures. Bound types use simultaneous
substitution. Predicate types are checked under their complete conjunction,
including inactive predicates. Identity-only proof queries do not discover
executable specializations. Class discovery retains ancestry across metadata
and returned-borrow fixed points; expanding recursion receives E0539.
Trait predicate composition consumes checked trait-use arguments, preserving
authored parameter identities and spans while substituting resolved subjects
and interface arguments. Nested traits, method-parameter renaming, aliases,
diamonds and computed accessors retain their canonical member/condition
identities. Trait `self` becomes the composing class with its owner arguments;
simultaneous substitution cannot capture those arguments with method parameters.
Method scopes remain distinct even when the trait has no owner parameters.
False predicates remain predicates; unresolved clauses retain an unavailable
callable marker and invalidate composition. These predicates do not establish
active members. Conditional source analysis uses effective class members for
lookup while checking authored bodies under their lexical parameters and own
premises. Composed callable templates project authored return loans and ambient
effects into effective signatures; refinement removes caught effects and keeps
class/free callers independent of source order. E0531 renders authored generic
names while retaining distinct internal identities.
Authored trait bodies also use imported member views built by the same
composition engine. Checked substitutions, aliases and exclusions retain the
lexical trait receiver; imported methods, getters and static generic calls
prove their clauses against the caller's bounds. Source-only checking identities
do not add public declarations or class witnesses. Conflicting contributions
remain unresolved and acquire no implicit trait-authored precedence.
Concrete source-body return and ownership checks use those authored templates
with effective instance substitutions, including renamed method parameters,
nested aliases, fixed trait arguments and composing-class `self`. Diagnostic-only
specialization collects finite concrete roots separately from source-storage
inventories. Its source binding and closure identities remain checking-only;
admitted expanded bodies acquire their own executable facts.
Admitted composed bodies discover recorded generic callees and body-local class
dependencies through the same authored spans and substitutions. Constrained
methods/getters and core operations share that substitution, including composing
`self`; effective callable/class roots retain admission and finite ancestry.
Authored trait instance/static calls retain their checked targets. Source return
inference reads live authored contracts and checked generic bindings; concrete
return queries substitute the composing receiver and select effective getters.
Ownership reads the authored method table for trait targets, preserving imported
aliases' taking contracts. Positional/named forwarding retains Copy, Move and
byte-region loans without changing checked call/accessor plans.
A checked-admission inventory and shared native/PHP specialization gate
govern guarded instances through exact concrete owner/method bindings;
missing preparation is rejected separately from checked unavailability.
Declaration slots and unconditional constructors remain intact. Public
conditional analysis uses this inventory after source checks succeed.
One source-owned witness plan fixes each checked concrete class/interface pair's
implementation declaration, owner bindings, canonical requirement origins and
exact-self return proof. Native and PHP materialization reject changed or
incomplete witness facts. Nongeneric witnesses require an admitted implementation
in their concrete declaring class or ancestor. Generic implementations retain
call-specific admission with their concrete method arguments. PHP ownership and
value-layout coordination consume the same exact callable identities.
The public analysis path passes checked conditional source through the
ordinary final semantic assembly, including attributes, constant diagnostics,
admissions and witness plans, then shared HIR lowering and callable specialization.
Direct and nested-trait members retain active/inactive selection and generic
dependencies without injected consumer facts.
Authored ownership consumes symbolic storage views;
exported layouts retain concrete views. Concrete conformance facts use the shared
nominal closure and requirement checker, including edges first proven after
substitution. The temporary E0776 rejection is removed in the implementation
candidate; current-pipeline notes record verification and remaining acceptance.
Lowering without checked admissions reports a structured internal compiler error.
Ordinary and conditional programs share final callable-contract
validation, including override return provenance, property hierarchies, defaults,
nominal conformances and parent-constructor protocols. Function-value ownership
signatures preserve symbolic parameters and trait/interface `self` ownership
until substitution. Symbolic `self` never implies a Copy carrier. Retained-source,
callback, pending-transfer and writeback analysis share one inventory of function,
class, trait and accessor bodies. Checked composing receivers refine authored
summaries before caller diagnostics, preserving storage types and recursive
dependencies. Taking callbacks through nested trait aliases retain their source
loans; unsafe mutation remains rejected. Opaque summary facts do not establish
that a body has been analyzed.
Shared callable validation prepares canonical interface requirement graphs before
ownership and retained-source refinement. Guarded generic methods and composed
aliases preserve owned Copy interface snapshots and borrowed erased Move results;
escaping temporary carrier loans remain rejected. A requirement graph establishes
neither conformance nor executable admission.
Constructor initialization and post-ownership validation also share one phase.
Composing classes own stored initializers and partial-construction cleanup;
authored callable identities own borrowed-getter and nonblocking body checks.
Inherited cleanup selects each physical property declaration through its
specialized declaring-owner view, preserving renamed and transitive type bindings.
Known blocking work is checked without concrete callers, while type-dependent
release obligations remain checked at executable specializations. Conditional
pending phases retain canonical nominal requirement origins without publishing
an unconditional witness. Pending-request storage and escape checks remain active.
Full ownership, composition/specialization, portable execution and integration
acceptance remain open. E0777 diagnoses forbidden clause placement. No runtime capability test,
fallback member or unconditional conformance is introduced by these declaration
facts.

Doria accepts `trait` declaration grammar for reusable class-body members:

```doria
trait HasSlug
{
    string $slug;
}
```

Decision 0134 defines generic traits composed into classes or traits with
`uses`. Traits may provide instance/static properties and methods plus constants;
bodyless methods are composer requirements. They flatten recursively in authored
order before class layout and MIR. The same trait origin reached through a
diamond is included once; unrelated collisions require explicit `insteadof` or
`as`, and `as internal` may only tighten access. Adaptations apply to methods,
not properties or constants. Traits cannot declare lifecycle methods, `open`, or
`override`, and cannot use `parent::`. User trait properties follow one
deterministic layout, initialization, and reverse-destruction order. There is no runtime trait object or PHP-defined trait precedence.

Doria implements single class inheritance. Classes are closed by default;
subclassing requires an `open class`, and a class may have at most one direct
parent:

```doria
open class Model
{
}

class Post extends Model
{
}
```

Methods are nonvirtual by default. `open function` creates a virtual slot and
`override function` is mandatory when replacing it. Override parameter names,
types, ownership modes, generic constraints, and generic arity match exactly;
the root declaration owns defaults; receiver access may weaken from writable to
readonly but not strengthen; class returns may be covariant through ordinary
assignment compatibility; and checked Errors may narrow but not widen.

```doria
open class Model
{
    open function save(): void throws StorageError
    {
    }
}

class Post extends Model
{
    override function save(): void throws StorageError
    {
    }
}
```

Qualified and generic parent types are accepted and monomorphized. Generic
classes remain invariant. Externally accessible members participate in
inherited lookup; declaring-class `internal` members do not. Doria has no
`protected`, abstract classes, multiple class inheritance, unchecked cast,
`final`, `sealed`, or late-static-binding surface.

Construction allocates one complete object, evaluates `new` arguments once,
and executes class phases root to derived. A required parent constructor call
must be the derived constructor's first source-level statement. Destruction
runs derived to root, dropping each class phase's remaining properties in
reverse order and freeing the allocation once. During construction or
destruction, `$this` virtual calls stop at the currently active class phase.

The payload remains headerless and data-only, including when a proven
nonescaping allocation uses stack storage instead of the heap. Placement does
not change ownership or destruction order. Closed exact class values
remain one-word payload pointers. A statically open class value carries the
payload pointer and a static hierarchy-descriptor pointer; an upcast constructs
that carrier without allocating or copying the object. Parent-prefix property
layout keeps inherited offsets stable. Decision 0130 defines hierarchy
resolution, parent calls, construction, destruction, dispatch, narrowing,
checked Error coverage, and backend obligations in full.

Decision 0134 accepts `implements` for compiler-checked nominal interface
conformance; Stage 35 Slice 1 implements its declaration graph and diagnostics:

```doria
class Post extends Model implements Renderable, JsonSerializable
{
}
```

A class may implement one or more interfaces. Conformance is authored rather
than structural, inherited through the class hierarchy, and checked against the
fully specialized interface requirement graph. Method-name coincidence alone
never establishes conformance.

Stage 35 Slice 1 checks generic interface inheritance, method-only requirements,
nominal class conformance, and declared trait graphs. Requirements end with `;`
and are not empty executable bodies. Trait-free concrete calls, including
interface-constrained generic specializations, use the existing class backend.
Stage 35 Slice 2 executes interface values, storage, conversions, and type tests
through one two-word data/vtable carrier. Class and ancestor conversions retain
the original allocation, ownership, and borrow root; vtables are immutable and
calls use checked constant slots. Error and Displayable use this same machinery,
including Error subinterfaces and all six shared/weak/access interface families.
Stage 35 Slice 3 executes core contracts, Copy-or-Cloneable preserving operations,
and public iteration with retained source and current-element provenance.
Stage 35 Slice 4 expands generic traits before final class checking and layout.
Requirements, adaptations, and inherited-member compatibility are resolved once
by the compiler; executable backends receive ordinary final class members.
E0493, E0758, and E0759 are retired and remain reserved.
Invalid declarations and primitive interface erasure receive language
diagnostics, not a future-feature excuse. No conversion adds an object header,
wrapper allocation, or owner; borrowed views cannot escape their root or lease.

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

Namespace/file-scope import `use` is implemented and resolved at compile time. Class-body or trait-body trait-composition `uses` preserves ordered generic references and `insteadof`/`as` adaptations. Slice 4 applies them to the composer before checking final class bodies, layout, and conformance.

Doria is PHP-shaped, not PHP++. Accepting PHP-shaped OOP declaration syntax does not import PHP dynamic object semantics, magic methods as core behavior, autoloading behavior, reflection behavior, loose typing, PHP visibility rules beyond what Doria has separately accepted, PHP trait conflict-resolution rules without review, or PHP runtime initialization behavior.

OOP declaration vocabulary is accepted separately from final visibility semantics. Doria's accepted early member model remains default-accessible plus `internal`: class members are accessible by default, `internal` controls API surface, and `writable` controls mutation.

Constructor property promotion is supported. Constructor parameters are promoted to externally accessible properties by default; `internal` creates an internal promoted property. In a derived constructor, `override` declares a parameter relationship to one exact inherited external instance property without adding storage or a second automatic write. `parameter` declares a constructor-only input and no property. Roles precede `writable` or `take`; those modes describe the input and do not change an inherited property's contract:

```doria
function __construct(
    writable string $name,
    int $age = 10,
    internal string $cacheKey = "person",
) {
}
```

```doria
open class Document
{
    function __construct(string $title) {}
}

class Article extends Document
{
    function __construct(override string $title, parameter string $source)
    {
        parent::__construct($title);
        echo $source;
    }
}
```

`override` requires an exact inherited external instance-property target after
generic substitution. The root property remains the one logical member, layout
slot, initializer, and cleanup obligation. `parameter` remains in constructor
arity, named arguments, defaults, attributes, ownership analysis, and body scope,
but never enters member lookup, layout, initialization, or drop order. A
constructor-only Move value follows ordinary parameter borrowing and does not
require `take` unless ownership is actually transferred. Actual Move-value
promotion retains its `take` requirement. `parameter` is the reserved spelling;
`param` is not an alias. Decision 0131 is authoritative.

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

Omitted and explicit lifecycle return annotations publish the same checked
`void` signature. Parent-constructor chaining produces no owned return value.

Lifecycle declaration shapes are a fixed allowlist. A constructor is declared as `function __construct(parameters)` or `internal function __construct(parameters)`. A destructor is declared as `function __destruct()` or `internal function __destruct()`. Either may explicitly declare `: void`. `static` and `writable` are rejected on both lifecycle names, and `__destruct` must declare exactly zero parameters. Other current or future method modifiers are rejected unless this specification explicitly adds them to the lifecycle allowlist.

Lifecycle methods cannot be invoked directly as ordinary instance or static methods. Construction uses `new Class(...)`, whose arguments are checked against `__construct`; destruction is compiler/runtime-invoked. The inheritance protocol reserves `parent::__construct(...)` for parent-constructor chaining; other direct lifecycle calls remain illegal.

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
    foreach ($args as string $argument) {
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

Native is the primary target. Checked HIR lowers to typed MIR, shared MIR validation gates both native lowerers, Cranelift emits the default fast object, LLVM 18 emits the O3 `--release` object, and the host linker combines either object with `doria-rt`. Native compilation has no interpreter preflight, fallback IR, or release-to-fast fallback. `doria-rt` owns entry policy, headerless class payload allocation/free, typed-array/collection/byte-buffer/mixed-box/shared-control storage, immutable refcounted runtime strings, text and binary I/O, formatting support, exact stdout/stderr writes, abort-only panic formatting, stack traversal, and status 101. Both lowerers share scalar/string conventions, one-word closed-class/collection/`Bytes`/`mixed` handles, two-word open-class/interface/shared-interface carriers, and their nullable ABI conventions. Normal cleanup drops still-owned class, collection, mixed, strong-reference, weak-reference, access-object, and nullable-strong locals and statement temporaries on fallthrough, `return`, `break`, and `continue`; invokes `__destruct` before reverse-order owned-property/element cleanup; releases writable-family access registrations before their strong claims; and frees the relevant payload or control block last. Ordinary instance/static calls must preserve those obligations. Owning class, collection, mixed, shared-handle, access-object, nullable-class, nullable-mixed, and nullable-strong returns transfer ownership, while Decision 0089 returned-borrow elision preserves an inferred readonly or writable alias to `$this` or exactly one borrowed parameter; only explicit `take` parameters and collection ingestion consume move arguments. Copy-type statics are private compiler-generated data symbols; compile-time string statics use an immortal private runtime representation and remain Copy at the Doria surface. Ownership transfer suppresses source cleanup, assignment acquires the replacement before dropping the old value, and abort-only panic runs no cleanup. Constructor definite initialization follows Decision 0090: semantic dataflow checks every reachable normal path, and MIR validation independently rejects incomplete or multiply initialized readonly property state before either native backend runs. Runtime failures use the shared panic path, except that an ordinary program write to a closed stdout or stderr pipe exits cleanly with status 0 under Decision 0091; panic reporting remains fatal even when its stderr sink is unavailable. Only canonical int/void entry results cross the process boundary. Stage 35 Slice 2 adds general interface carriers, checked erased dispatch, and dynamic drop across those paths. Unsupported coverage remains for scalar/string writable-shared payload access, shared handles through `mixed`. Trait composition executes through final checked class members without a trait runtime.

Shared native return adaptation now preserves checked nominal interface views,
class ancestry and nullable narrowing under Decisions 0130/0134. Root families
retain physical return and borrow contracts; selected slots retain their logical
promises even when bodies are inactive. One typed MIR adapter constructor serves
lowering and validation, which rejects widened inherited promises, mismatched
implementation effects and forged adapter bodies. Existing value factories and
storage projections preserve owned and borrowed class, interface, collection,
inline Copy, scalar, string, payload-enum and function results. Nullable payload
classification is shared, and LLVM class upcasts forward exact borrow homes.
Nominal tests preserve inline target identity and use interface witnesses for
presence, including zero-valued payloads. Constraint-only primitive capabilities
do not authorize erased return covariance.

PHP consumes checked virtual families and nominal witnesses through typed caller
views and distinct implementation entries. Covariant collection returns retain
their source annotations without relying on PHP method covariance. Borrowed
collection views retain the original storage cell; ordinary and erased calls,
computed getters, root-owned defaults and core iteration/display entries use the
same plan. PHP generation remains available without native MIR. Direct inline
nominal interface tests preserve zero payloads, nullable presence and exactly-once
evaluation through shared lowering. Receiver
substitution now shares one semantic/MIR rule and explicit root/selected
contracts independent of body admission. Canonical virtual families also own
physical checked transport independently of logical error narrowing; return
adapters preserve implementation error contracts. Shared MIR validates reachable
entry error subsets, and both native backends use the same signature plan and
stack-only result transport. Focused macOS execution covers field access,
child/root destruction, seven result shapes, inactive roots, owned-argument
cleanup, nominal Error subclasses and Error subinterfaces across the interpreter,
Cranelift, LLVM and PHP. Checked source facts now distinguish exact implementation
ambient effects from the descendants reachable through a virtual call. Exact
parent, closed-receiver and direct lifecycle calls preserve that distinction
through generic binding, property hooks, MIR bodies and PHP implementation
entries. Focused regressions cover catches, lifecycle calls, traits and callbacks.
This preserves the accepted source contracts above;
fresh Linux/Windows execution and broader regression verification remain open.

The PHP backend is currently implemented as a compatibility/debugging backend. It emits `<?php` and lowers Doria-only syntax away:

- `let` is removed.
- `writable` is removed.
- `internal` is enforced by Doria before backend emission and may lower to PHP `private` or another backend-specific representation.
- Typed arrays and collection aliases are emitted as `array` for the current PHP backend only.
- Doria readonly/writable rules are enforced before Doria IR lowering and backend emission, not at PHP runtime.
- `int`/`int64` remain the exact supported signed-integer alias subset.
- `uint64` transport, constants/defaults, display/formatting, comparison, and collection ordering preserve all 64 bits on 64-bit PHP. The backend uses a private signed bit-pattern representation, not a floating-point conversion; this does not enable otherwise unsupported arithmetic or conversions.
- Checked arithmetic, nondefault widths, unsigned semantics, division/remainder, shifts/bitwise operations, and integer companion conversions produce a clear backend unsupported-feature diagnostic whenever PHP cannot preserve the Doria behavior exactly.

For Doria features that PHP cannot express directly, such as object construction in property initializers or richer attribute expressions, the PHP backend should lower to equivalent generated PHP where practical or produce a clear unsupported-feature diagnostic temporarily. PHP limitations must not define Doria semantics.

Backend-specific tests are useful, but the PHP backend must not be the required proof that a language feature is correct. Correctness belongs to the parser, semantic checker, Doria IR, and eventually the native/backend-independent execution path.

## 14. Future features

Future work includes:

- Better diagnostics with suggestions.
- Full type inference for lists and dictionaries.
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
