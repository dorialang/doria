# 0014 Baton project tool

Status: Accepted

## Decision

The Doria project tool is named Baton.

Accepted decisions:

- `doriac` is the Doria compiler.
- Baton is the planned user-facing project, package, build, and application orchestration tool for Doria.
- Baton coordinates projects and invokes compiler functionality; it does not duplicate or redefine Doria parsing, semantic analysis, type checking, readonly/writable checking, Doria IR lowering, or code generation.
- Doria semantics remain owned by the language specification, accepted design decisions, and `doriac`.
- Baton must not become a second semantic implementation of Doria.
- Baton is planned. It is not currently implemented.

Baton plays the ecosystem role that Cargo plays for Rust projects and Composer plays for PHP dependency management, while remaining distinctly designed for Doria and its native-first goals.

## Architectural boundary

Baton sits outside the compiler pipeline.

The intended orchestration shape is:

```text
Doria source/project
-> Baton project orchestration
-> doriac compiler pipeline
-> selected native backend
-> standalone executable
```

The compiler pipeline remains:

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

Baton must not be documented or implemented as a stage inside that compiler pipeline.

## Planned responsibilities

Baton is expected to own project and ecosystem orchestration concerns such as:

- creating and initializing Doria projects
- managing project metadata
- dependency declaration and resolution
- lockfile management
- coordinating builds
- invoking `doriac`
- running applications
- running tests
- managing build profiles
- preparing packages for publication
- interacting with a future Doria package registry

This list records product direction. It is not an implementation claim and does not mean these features exist today.

## Deferred decisions

This decision does not settle:

- manifest filename
- lockfile filename
- registry protocol
- package namespace syntax
- dependency version grammar
- workspace format
- build-script language
- package signing model
- exact publication workflow
- complete CLI command surface

Those require separate design decisions.

## Public workflow positioning

The canonical newcomer-facing workflow is:

```text
write -> build -> run
```

When Baton is described, the intended public shape is:

```text
Doria source
-> baton build
-> standalone native executable
-> baton run
```

A shorter presentation may use:

```text
Source -> Build -> Run
```

Guardrails:

- Do not put `doriac check` in the main homepage or newcomer workflow.
- Do not write "check, compile, run" as the primary user journey.
- Do not imply that users must manually validate a program before building it.
- `doriac check` remains valid optional tooling for editors, CI, compiler development, and validation without output.
- `doriac compile` and future `baton build` must validate source before emitting output.
- Do not globally ban references to `doriac check`; it belongs in compiler and CLI documentation.
- Do not expose Cranelift or LLVM in the main newcomer workflow. Those belong in architecture, performance, and roadmap documentation.

## Native profile relationship

Baton must eventually support the accepted native strategy:

```text
Fast native profile       -> Cranelift
Optimized native profile  -> LLVM
```

Baton may select and orchestrate profiles, but it must not change Doria-visible semantics between them. Fast and optimized builds are backend profiles for the same Doria language, not separate semantic modes.

## Non-goals

This decision does not:

- implement a Baton binary or crate
- add Baton commands
- create a manifest format
- create a lockfile format
- design the package registry
- implement dependency solving
- change `doriac` CLI behavior
- change compiler code
- change native backend behavior
- change PHP backend behavior
- add package-manager dependencies
