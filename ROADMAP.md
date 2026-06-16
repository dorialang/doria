# Roadmap

## Strategic Goals

- Build Doria as a compiled language with native machine code and standalone executables as the long-term target.
- Support command-line tools, services, systems software, native desktop applications, game development, game tooling, game engines, graphics/media work, C-library bindings, and future raylib bindings.
- Keep PHP as a compatibility, migration, debugging, and inspection backend only.
- Move toward **self-hosting**: `doriac` is initially implemented in Rust, but an early language-development goal is to eventually write significant parts of `doriac` in Doria itself.
- Support Doria language features that PHP cannot express directly, including executable property initializers and richer attribute/metadata expressions.
- Eventually support PHP-to-Doria migration tooling, while keeping that tooling separate from the Doria parser and core compiler semantics.
- Build a benchmark culture early: measure speed, memory, compile time, startup time, and artifact size before making performance claims.

## Current Slice

- Keep the parser and semantic checker small but tested.
- Treat the checked compiler-owned representation as Doria IR.
- Keep PHP as a compatibility backend only.
- Do not build PHP-to-Doria migration in the current v0.1 slice.
- Do not start desktop, game engine, raylib, or FFI implementation work in the current v0.1 slice.

## Next Compiler Work

- Implement real semantic type IDs and assignment compatibility.
- Add return type checking.
- Add constructor init access for readonly properties.
- Plan a lowered/native IR when native code generation needs a simpler representation for control flow, memory layout, runtime calls, and backend emission.
- Add native backend experiments behind explicit targets.
- Plan the path toward writing more of `doriac` in Doria itself.
- Add string interpolation AST nodes independent of PHP behavior.
- Emit precedence-aware backend expressions.
- Add parser/AST support for attributes using `#[...]`.
- Add shared call argument representation for positional and named arguments.
- Preserve property initializer expressions in AST/Doria IR and later lower non-constant initializers correctly.
- Add property hooks later for validation and computed properties without changing the default-public plus `internal` member model.
- Add language/design support for `writable class` and `readonly class` as mutability ergonomics before considering shorter mutation keywords.

## Performance and Native Application Path

- Add a `benchmarks/` structure before making public performance claims.
- Track runtime speed, compile time, startup time, memory, binary size, stripped binary size, compressed artifact size, and correctness output.
- Include Doria-relevant benchmarks such as lexing, parsing, type checking, object construction, string operations, collections, and eventually small game-loop/FFI smoke tests.
- Keep native desktop, game engine, and raylib goals visible when designing Doria IR, runtime, memory representation, and FFI.
- Do not begin raylib bindings until native backend, FFI model, and basic runtime are ready.

## PHP Migration Path

- Treat Doria-to-PHP as a backend and PHP-to-Doria as a migration converter. They are separate directions with separate architecture.
- Do not promise perfect conversion for all valid PHP.
- Start with simple, typed, modern PHP and produce conservative valid Doria.
- Prefer diagnostics and TODO comments over unsafe or misleading rewrites for dynamic PHP features.
- Add a Doria pretty-printer before serious PHP-to-Doria work.
- Consider a future `doria_migrate_php` crate or `doriac migrate php` command after the Doria AST, semantic types, and formatter are more stable.

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
