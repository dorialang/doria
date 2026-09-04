# Website Content Guidelines

> Documentation role: supporting design note.
> Source-of-truth hierarchy: `docs/doria-end-to-end-plan.md` owns future sequencing; accepted `docs/decisions/*.md` files own topic-level decisions. This note is subordinate to both.

## Target-State Authority

The public website represents the completed language and toolchain target, not a
status page for the incremental compiler implementation. It is Andrew's BDD/UAT
surface: examples may intentionally describe accepted target behavior before the
compiler can execute it, so implementation gaps remain visible during delivery.

Guardrails:

- Never downgrade target-state documentation or playground examples to match the
  current compiler stage.
- Change website semantics only when a later accepted decision changes the
  completed target.
- Do not expose stage numbers, current backend coverage, temporary deferral
  diagnostics, or implementation-status caveats as product documentation.
- Do not invent exact API spellings that an accepted decision explicitly defers.
  Describe the accepted capability contract until the owning decision settles
  its public names.
- A current compiler failure against a valid target-state example is UAT evidence,
  not permission to remove or weaken the example.
- Performance copy must reflect an accepted target-state contract and remain
  workload-specific. Describe bounded allocation/copy behavior or benchmarked
  cases when authority supports them; never turn a design goal or one benchmark
  into an unqualified claim that Doria is faster, instant, zero-cost, or has no
  hidden costs.

## Homepage Toolchain Positioning

The homepage teaches Doria's public workflow as:

```text
write -> build -> run
```

Baton is the intended public project tool. `doriac` is the underlying compiler. Baton coordinates projects, packages, builds, tests, and application runs by invoking compiler functionality; it does not define Doria semantics.

The website presents Baton as the completed public project tool. Compiler-side
working notes may track its implementation status, but public product copy must
remain written from the completed-release perspective.

Guardrails:

- Do not present `doriac check` as a mandatory workflow stage.
- Do not imply users must manually validate a program before building it.
- `doriac check` remains valid optional tooling for editors, compiler tooling, CI, and local validation without output.
- Backend implementation details such as Cranelift, LLVM, object files, linkers, and backend profile names are not homepage onboarding content.
- Compiler-oriented documentation may still document direct `doriac` commands.

Acceptable:

```text
Doria source -> Baton build -> Native executable -> Run
```

```text
Write Doria, build with Baton, run native.
```

```text
For fast validation without output, doriac check is available to editors, tooling, and CI.
```

Unacceptable:

```text
Doria source -> doriac check -> doriac compile -> Executable
```

```text
Check your source, compile it, then run it.
```

## Package And Autoload Positioning

Public package documentation uses `autoload` for Baton-managed source discovery.
Baton finds matching `.doria` files while building and gives a deterministic
source inventory to `doriac`; compiled programs do not search for or load Doria source files at runtime.

Keep the package concepts separate:

- `autoload` finds a package's source files automatically during the build.
- `use` gives a shorter name to a declaration in source.
- `include` explicitly adds one same-package source file at compile time.
- dependencies add other packages to the resolved build graph.

Target-state project guides may teach the accepted manifest schema, hybrid
source layout, dependency workflow, lockfile, workspace, cache, and offline
behavior before those facilities execute in the bootstrap toolchain. Do not add stage numbers,
temporary implementation caveats, compiler backend details, or
runtime-autoloader language to public package guidance. Do not rename the public
manifest action to `sources`, `discover`, or another implementation term.

## API Naming

Website examples must follow the naming charter:

- Use `snake_case` only for built-in free functions, such as `read_line()` and `get_time()`.
- Use `camelCase` for userland free functions, methods, static/companion APIs, properties, parameters, and named arguments.
- Use `PascalCase` for types and enum cases, `SCREAMING_SNAKE_CASE` for constants, and single Pascal capitals for type parameters.
- Keep `__construct` and `__destruct` in their inherited PHP-shaped spelling.

Member examples should look like `Int::wrappingAdd()`, `$s->isEmpty()`, `$message->tenantId`, `$message->retryAfter(seconds: 30)`, and `$repository->findById($id)`.

String examples follow Decision 0103's one-spelling rule. Use `$text->` only
for intrinsic data and views such as `length`, `byteLength`, `isEmpty`, `bytes`,
`graphemes`, and `codePoints`. Use `String::` for every string-specific
operation, including `String::trim()` and `String::startsWith()`. Do not publish
canonical Doria examples using a `str_*` free function or a string-operation
instance method. PHP migration examples may show PHP spellings when they are
clearly identified as PHP input.

## Constructor Examples

Website docs should teach Doria constructor property promotion as the default/simple class style.

Guardrails:

- Prefer `function __construct(string $name) { }` over declaring `string $name;` and assigning `$this->name = $name;`.
- Use promoted modifiers such as `writable`, `internal`, and `internal writable` to teach mutability and API surface.
- Teach `override string $title` when a derived constructor reuses an inherited external property; keep the explicit first `parent::__construct($title)` call.
- Teach `parameter string $raw` when an input exists only during construction, including manual validation or transformation into an explicit property.
- Use manual constructor assignment only with `parameter` when the stored property has a different name, or when the constructor validates, normalizes, transforms, or accepts ownership into a differently named field.
- Do not rename a child parameter merely to evade an inherited-property collision; role markers express the semantic intent without hidden storage.
- Do not use PHP visibility modifiers such as `public`, `private`, or `protected` in Doria examples.

## Foreach And Display Examples

For two-binding `foreach`, teach a zero-based readonly `int` first binding on
`List<T>` and `T[]`, and an actual readonly key on Dictionary families. Keep
ranges, sets, deques, and Dictionary projections value-only. Property-rooted
sequence examples are valid and should not be rewritten into manual counters.

Teach interpolation, string-anchored concatenation, and `%s` as expressions that
produce ordinary reusable strings. Do not compensate with primitive `toString`,
`String::from` scalar aliases, casts, or implicit scalar assignment conversion.
