# Roadmap

## Strategic Goals

- Build Doria as a PHP-shaped compiled language with native machine code and standalone executables as the long-term target.
- Keep PHP as a compatibility, migration, debugging, and transpilation backend only.
- Move toward **self-hosting**: `doriac` is initially implemented in Rust, but an early language-development goal is to eventually write significant parts of `doriac` in Doria itself.
- Support Doria language features that PHP cannot express directly, including executable property initializers and richer attribute/metadata expressions.

## Current Slice

- Keep the parser and semantic checker small but tested.
- Treat the current lowered form as HIR, not final IR.
- Keep PHP as a compatibility backend only.

## Next Compiler Work

- Implement real semantic type IDs and assignment compatibility.
- Add return type checking.
- Add constructor init access for readonly properties.
- Design MIR as a control-flow-oriented lowering target.
- Add native backend experiments behind explicit targets.
- Add string interpolation AST nodes independent of PHP behavior.
- Emit precedence-aware backend expressions.
- Add parser/AST support for attributes using `#[...]`.
- Add shared call argument representation for positional and named arguments.
- Preserve property initializer expressions in AST/HIR and later lower non-constant initializers correctly.

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
