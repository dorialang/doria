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
- Support the first accepted boolean/equality compiler slice: typed `==` / `!=`, rejection of `===` / `!==`, bool-only `!` / `not`, `&&` / `and`, `||` / `or`, and `xor` in the parser, semantic checker, Doria IR, and PHP backend.
- Stage 10 Cranelift native smoke backend is implemented for top-level free functions plus exactly one top-level `function main(): int` or `function main(): void`. `main(): int` returns an explicit process status in the accepted `0..125` portable exit-code range, while helper `int` returns are Doria `int` values, not process statuses. The supported source subset includes Stage 9 integer/control-flow/string support plus native helper functions with `int` parameters, `int`/`void` returns, calls to `int` helpers in supported integer expressions, and calls to `void` helpers in statement position. Native validation proves accepted loops terminate within the current smoke verification cap and rejects recursion, mutual recursion, unsupported native signatures, and broader runtime features before lowering to real Cranelift functions and control flow.
- Stage 7a preserves the Stage 6c native source subset while routing current native smoke validation, compile-time smoke evaluation/proof, and Cranelift lowering through a private native smoke module boundary.
- Stage 11a seeds inspectable MIR and a debug interpreter for main(): int literal returns, main(): void fallthrough or bare return, and exact string-literal echo. NativeSmokeModule still exists temporarily for broader current native smoke coverage and must not receive new language capability expansion without explicit approval.
- Keep PHP as a compatibility backend only; do not treat PHP output as the proof that Doria semantics are correct.
- Do not build PHP-to-Doria migration in the current v0.1 slice.
- Do not start desktop, game engine, raylib, or FFI implementation work in the current v0.1 slice.

## Next Compiler Work

- Follow docs/doria-end-to-end-plan.md as the master execution plan for future work. The active implementation slice is Stage 11a: seed MIR + interpreter oracle for the smallest executable subset. Full Stage <=10 MIR port, full interpreter parity, and NativeSmokeModule deletion remain future Stage 11b/11c work.
- Treat `docs/decisions/0011-native-execution-path.md` as the accepted Stage 1 native execution path.
- Follow the accepted staged Cranelift/LLVM native backend direction: Cranelift first for the smallest native smoke/backend route, LLVM later as the longer-term optimizing backend path.
- Treat `docs/decisions/0013-stage-2-native-integers.md` as the accepted Stage 2 native integer execution decision.
- Treat `docs/decisions/0015-stage-2b-native-readonly-integer-locals.md` as the accepted Stage 2b native readonly integer locals decision.
- Treat `docs/decisions/0016-fixed-width-numeric-types.md` as the accepted fixed-width numeric family and default numeric spelling decision.
- Treat `docs/decisions/0017-stage-2c-native-int-arithmetic.md` as the accepted Stage 2c native integer arithmetic decision.
- Treat `docs/decisions/0018-stage-2d-native-returned-int-expressions.md` as the accepted Stage 2d native returned integer expression decision.
- Treat `docs/decisions/0019-stage-4a-native-if-else-returns.md` as the accepted Stage 4a native terminal `if` / `else` and guard-return decision.
- Treat `docs/decisions/0020-boolean-operators-and-given-predicates.md` as the accepted typed equality, boolean operator, bitwise operator, `xor`, and `given` predicate block decision.
- Treat `docs/decisions/0021-stage-4b-native-boolean-conditions.md` as the accepted Stage 4b native boolean-condition implementation decision.
- Treat `docs/decisions/0022-stage-5a-native-writable-int-locals.md` as the accepted Stage 5a native writable integer local and direct-body assignment implementation decision.
- Treat `docs/decisions/0023-stage-5b-native-structured-if-blocks.md` as the accepted Stage 5b native structured `if` block implementation decision.
- Treat `docs/decisions/0024-stage-6a-native-bounded-while.md` as the accepted Stage 6a native bounded `while` implementation decision.
- Treat `docs/decisions/0025-stage-6b-native-if-fallthrough-merges.md` as the accepted Stage 6b native fallthrough `if` merge implementation decision.
- Treat `docs/decisions/0026-stage-6c-native-structured-while-bodies.md` as the accepted Stage 6c native structured `while` body implementation decision.
- Treat `docs/decisions/0027-stage-7a-native-smoke-ir-boundary.md` as the accepted Stage 7a native smoke IR boundary implementation decision.
- Treat `docs/decisions/0028-namespaces-use-include-and-directives.md` as the accepted namespace, `use`, `include`, `declare`, `break`, and `continue` direction.
- Treat `docs/decisions/0029-oop-declaration-vocabulary.md` as the accepted `class` / `interface` / `trait` / `extends` / `implements` OOP declaration vocabulary direction.
- Treat `docs/decisions/0030-trait-composition-uses-keyword.md` as the accepted split between namespace/file-scope `use` imports and class-body/trait-body `uses` trait composition.
- Treat `docs/decisions/0031-stage-7b-break-continue.md` as the accepted initial compiler and native smoke implementation for unlabeled `break;` and `continue;`.
- Treat `docs/decisions/0032-main-void-and-implicit-success.md` as the accepted entrypoint direction for `main(): void`, implicit successful process status, and narrow native string-literal stdout smoke support.
- Treat `docs/decisions/0033-stage-8a-native-readonly-string-locals-and-echo.md` as the accepted Stage 8a subset for literal-backed readonly string locals and supported string `echo` expressions.
- Treat `docs/decisions/0034-stage-9-mvp-iteration-syntax.md` as the accepted MVP iteration direction for traditional `for`, integer range `foreach`, standalone `++` / `--`, and bounded/proven native Stage 9 loop support.
- Treat `docs/decisions/0035-checked-throw-throws-direction.md` as the accepted checked error spelling direction. Do not implement `throw`, `throws`, `try`, or `catch` compiler behavior without a dedicated implementation slice.
- Treat `docs/decisions/0036-stage-8-mvp-string-echo-path.md` as the accepted Stage 8 MVP string and echo path, completing Stage 8a with compile-time-known `.` string concatenation in native smoke support.
- Treat `docs/decisions/0037-stage-10-native-free-functions.md` as the accepted Stage 10 native free-function implementation slice.
- Keep broader native expression, assignment, and control-flow support beyond the Stage 10 smoke subset as separate future implementation slices.
- Keep broader native strings, writable string locals, interpolation, runtime string concatenation, runtime I/O, stdout formatting, and nonzero runtime error statuses as future runtime/error-model work.
- Extend accepted operator support with integer bitwise spellings in a dedicated parser and semantic checking slice; do not import PHP loose comparison or PHP `and` / `or` precedence.
- Add compiler support for `int8`/`int16`/`int32`/`int64`, `uint8`/`uint16`/`uint32`/`uint64`, and `float32`/`float64` in a dedicated typed semantic model slice before claiming those spellings are implemented.
- Plan a broader lowered/native IR when native code generation needs a simpler representation for control flow, memory layout, runtime calls, and backend emission; do not confuse the private Stage 7a native smoke module with final Doria MIR.
- Expand native source support beyond Stage 10 only after the next accepted native slice specifies the language semantics and expected behavior.
- Keep future LLVM optimized-profile work conformant with accepted Doria integer semantics and Cranelift fast-profile behavior for the same supported programs.
- Expand return checking from the current final-statement rule into full path-sensitive control-flow analysis.
- Add full definite property initialization analysis for constructor paths.
- Plan the path toward writing more of `doriac` in Doria itself.
- Design and implement namespaces as semantic symbol ownership.
- Design and implement `use` statements for semantic imports and aliases.
- Design and implement `include` as required include-once source inclusion.
- Design and implement `declare` as structured compiler/source directives.
- Design and implement interfaces.
- Design and implement class inheritance through `extends`.
- Design and implement interface conformance through `implements`.
- Design and implement traits and class-body/trait-body `uses` trait composition.
- Keep namespace/file-scope `use` imports separate from trait-composition `uses`, and plan PHP migration rewrites from class-body `use TraitName;` to `uses TraitName;`.
- Define trait conflict-resolution rules before implementing advanced trait behavior.
- Keep OOP runtime, layout, dispatch, and ABI behavior backend-independent and native-first.
- Design labeled and numeric `break` / `continue` only with a separate accepted decision.
- Evaluate `goto` carefully before acceptance.
- Evaluate structured conditional compilation and compile-time diagnostics without adopting C/C++ textual macros.
- Expand string interpolation beyond variable/property paths after Doria has a deliberate display/string-conversion design.
- Emit precedence-aware backend expressions.
- Add parser/AST support for attributes using `#[...]`.
- Add named arguments and shared call argument representation for calls and attributes.
- Preserve property initializer expressions in AST/Doria IR and later lower non-constant initializers correctly.
- Add property hooks later for validation and computed properties without changing the default-accessible plus `internal` member model.
- Add language/design support for `writable class` and `readonly class` as mutability ergonomics before considering shorter mutation keywords.
- Keep implementation of advanced control-flow constructs as future work: `finally`, `do ... while`, `given`, value-returning `when`, `match`, and labeled or numeric loop control.

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
