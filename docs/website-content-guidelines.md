# Website Content Guidelines

> Documentation role: supporting design note.
> Source-of-truth hierarchy: `docs/doria-end-to-end-plan.md` owns future sequencing; accepted `docs/decisions/*.md` files own topic-level decisions. This note is subordinate to both.

## Homepage Toolchain Positioning

The homepage teaches Doria's public workflow as:

```text
write -> build -> run
```

Baton is the intended public project tool. `doriac` is the underlying compiler. Baton coordinates projects, packages, builds, tests, and application runs by invoking compiler functionality; it does not define Doria semantics.

Baton is planned, not currently implemented. Until Baton exists, public docs must describe it as planned product direction rather than current user functionality.

Guardrails:

- Do not present `doriac check` as a mandatory workflow stage.
- Do not imply users must manually validate a program before building it.
- `doriac check` remains valid optional tooling for editors, compiler tooling, CI, and local validation without output.
- Backend implementation details such as Cranelift, LLVM, object files, linkers, and backend profile names are not homepage onboarding content.
- The homepage must not claim full native-language support while the native backend remains intentionally incremental.
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

```text
Baton currently builds Doria projects.
```

## Constructor Examples

Website docs should teach Doria constructor property promotion as the default/simple class style.

Guardrails:

- Prefer `function __construct(string $name) { }` over declaring `string $name;` and assigning `$this->name = $name;`.
- Use promoted modifiers such as `writable`, `internal`, and `internal writable` to teach mutability and API surface.
- Use manual constructor assignment only when the stored property has a different name, or when the constructor validates, normalizes, transforms, or accepts ownership into a differently named field.
- Do not use PHP visibility modifiers such as `public`, `private`, or `protected` in Doria examples.
