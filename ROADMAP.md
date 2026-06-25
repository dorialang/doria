# Roadmap

## Strategic Goals

- Build Doria as a compiled language with native machine code and standalone executables as the primary product target.
- Support command-line tools, services, systems software, native desktop applications, game development, game tooling, game engines, graphics/media work, C-library bindings, and future raylib bindings.
- Prioritize correctness, safety, and explicit language semantics over quick runnable demos or backend-specific shortcuts.
- Pursue a dual native backend strategy: Cranelift for fast local development/profile builds and LLVM for optimized release/profile builds.
- Keep PHP as an optional compatibility, migration, debugging, and inspection backend only.
- Move toward **self-hosting**: `doriac` is initially implemented in Rust, but an early language-development goal is to eventually write significant parts of `doriac` in Doria itself.
- Support Doria language features that PHP cannot express directly, including executable property initializers and richer attribute/metadata expressions.
- Eventually support PHP-to-Doria migration tooling, while keeping that tooling separate from the Doria parser and core compiler semantics.
- Establish Baton as Doria's planned project, package, build, and application orchestration tool while keeping `doriac` as the compiler.
- Build a benchmark culture early: measure speed, memory, compile time, startup time, and artifact size before making performance claims.

## Current Slice

- Keep the parser and semantic checker small but tested.
- Treat the checked compiler-owned representation as Doria IR.
- Check assignment compatibility, declared function/method return values, and positional call arguments in the current semantic slice.
- Allow constructors to initialize uninitialized readonly properties through narrow direct init access.
- Support MVP `if` / `else if` / `else` and `while` in the AST, semantic checker, Doria IR, and PHP backend.
- Represent braced string interpolation in the Doria AST and Doria IR, with PHP lowering emitted as explicit concatenation.
- Stage 2a Cranelift native smoke backend is implemented for exactly one top-level `function main(): int` returning an integer literal in the portable `0..125` exit-code range.
- Keep PHP as a compatibility backend only; do not treat PHP output as the proof that Doria semantics are correct.
- Do not build PHP-to-Doria migration in the current v0.1 slice.
- Do not start desktop, game engine, raylib, or FFI implementation work in the current v0.1 slice.

## Next Compiler Work

- Treat `docs/decisions/0011-native-execution-path.md` as the accepted Stage 1 native execution path.
- Follow the accepted staged Cranelift/LLVM native backend direction: Cranelift first for the smallest native smoke/backend route, LLVM later as the longer-term optimizing backend path.
- Treat `docs/decisions/0013-stage-2-native-integers.md` as the accepted Stage 2 native integer execution decision.
- Treat `docs/decisions/0016-fixed-width-numeric-types.md` as the accepted fixed-width numeric family and default numeric spelling decision.
- Keep Stage 2b readonly integer locals, Stage 2c simple integer arithmetic, and Stage 2d returned integer expressions as separate future implementation slices after Stage 2a.
- Add compiler support for `int8`/`int16`/`int32`/`int64`, `uint8`/`uint16`/`uint32`/`uint64`, and `float32`/`float64` in a dedicated typed semantic model slice before claiming those spellings are implemented.
- Plan a lowered/native IR when native code generation needs a simpler representation for control flow, memory layout, runtime calls, and backend emission.
- Expand native support beyond Stage 2a only after the next accepted native slice specifies the language semantics and expected behavior.
- Keep future LLVM optimized-profile work conformant with accepted Doria integer semantics and Cranelift fast-profile behavior for the same supported programs.
- Expand return checking from the current final-statement rule into full path-sensitive control-flow analysis.
- Add full definite property initialization analysis for constructor paths.
- Plan the path toward writing more of `doriac` in Doria itself.
- Expand string interpolation beyond variable/property paths after Doria has a deliberate display/string-conversion design.
- Emit precedence-aware backend expressions.
- Add parser/AST support for attributes using `#[...]`.
- Add named arguments and shared call argument representation for calls and attributes.
- Preserve property initializer expressions in AST/Doria IR and later lower non-constant initializers correctly.
- Add property hooks later for validation and computed properties without changing the default-public plus `internal` member model.
- Add language/design support for `writable class` and `readonly class` as mutability ergonomics before considering shorter mutation keywords.
- Keep advanced control-flow constructs as future design work: `finally`, `do ... while`, `given`, value-returning `when`, `match`, `break`, and `continue`.

## Performance and Native Application Path

- Add a `benchmarks/` structure before making public performance claims.
- Track runtime speed, compile time, startup time, memory, binary size, stripped binary size, compressed artifact size, and correctness output.
- Include Doria-relevant benchmarks such as lexing, parsing, type checking, object construction, string operations, collections, and eventually small game-loop/FFI smoke tests.
- Keep native desktop, game engine, and raylib goals visible when designing Doria IR, runtime, memory representation, and FFI.
- Require conformance tests once Cranelift and LLVM both support the same native feature: same Doria source, same semantic checks, same Doria-visible behavior.
- Do not begin raylib bindings until native backend, FFI model, and basic runtime are ready.

## Baton Project Tool Path

Baton is planned project tooling. It should not move ahead of current compiler correctness work unless explicitly directed later.

Future Baton work should proceed in stages:

1. Accepted product identity and responsibility boundary: Baton is the project/package/build tool; `doriac` is the compiler; Doria semantics remain owned by the language specification and compiler.
2. Manifest and lockfile design: decide file names, package metadata, dependency syntax, lockfile guarantees, and reproducibility rules in separate decisions.
3. Project creation: design project initialization, default layout, examples, and starter application shape.
4. Build orchestration: define the public write/build/run path without exposing compiler internals as the primary workflow.
5. Compiler invocation: decide how Baton invokes `doriac`, passes profiles/options, receives diagnostics, and preserves compiler-owned semantics.
6. Dependency resolution: design version constraints, source kinds, conflict resolution, and diagnostics.
7. Local/package cache: define where packages and build artifacts live and how cache invalidation works.
8. Workspaces: design multi-package repositories and shared dependency resolution.
9. Testing integration: define how Baton discovers and runs tests without becoming a separate language runtime.
10. Package registry and publication: design registry interaction, publication checks, package metadata, and yanking/deprecation policy.
11. Security, integrity, and reproducibility: design checksums, signing or trust model, supply-chain protections, offline behavior, and deterministic build inputs.

Throughout this path, Baton may orchestrate the accepted native profiles:

```text
Fast native profile       -> Cranelift
Optimized native profile  -> LLVM
```

Baton must not change Doria-visible semantics between profiles.

## PHP Migration Path

- Treat Doria-to-PHP as an optional backend and PHP-to-Doria as a migration converter. They are separate directions with separate architecture.
- Do not promise perfect conversion for all valid PHP.
- Do not prioritize PHP migration ahead of the native execution path.
- Start with simple, typed, modern PHP and produce conservative valid Doria.
- Prefer diagnostics and TODO comments over unsafe or misleading rewrites for dynamic PHP features.
- Add a Doria pretty-printer before serious PHP-to-Doria work.
- Consider a future `doria_migrate_php` crate or `doriac migrate php` command after the Doria AST, semantic types, formatter, and native execution path are more stable.

## Self-Hosting Path

- Keep the Rust implementation small, readable, and modular enough to port gradually.
- Define a Doria subset capable of expressing compiler code: enums or tagged unions, collections, pattern-like control flow, error handling, modules/namespaces, file I/O, and tests.
- Begin with small compiler-adjacent Doria libraries before rewriting compiler stages.
- Port leaf components first, such as diagnostics formatting, source spans, token definitions, or small utilities.
- Use the Rust `doriac` as the bootstrap compiler until the Doria implementation can compile itself.
- Eventually verify self-hosting through a repeatable bootstrap chain: Rust doriac builds Doria doriac, then Doria doriac builds itself and produces equivalent behavior.

## Repository Work

- Keep CI green.
- Add branch rulesets in GitHub after the first CI run.
- Enable Dependabot, secret scanning, and push protection in repository settings.
