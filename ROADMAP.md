# Roadmap

## Current Slice

- Keep the parser and semantic checker small but tested.
- Treat the current lowered form as HIR, not final IR.
- Keep PHP as a compatibility backend only.
- Keep Rust framed as the bootstrap implementation language while Doria self-hosting grows.

## Product Direction

- Make Doria useful where PHP developers want a PHP-like experience but PHP itself is unsuitable.
- Prioritize native desktop applications, CLI tools, game development, game engines, graphics/multimedia tooling, native library bindings, and future raylib bindings.
- Preserve focus on native compilation, standalone executables, low-overhead runtime design, C-compatible FFI, predictable performance, binary size, and game/graphics-friendly APIs.

## Next Compiler Work

- Implement real semantic type IDs and assignment compatibility.
- Add return type checking.
- Add constructor init access for readonly properties.
- Design MIR as a control-flow-oriented lowering target.
- Add native backend experiments behind explicit targets.
- Plan the path toward writing more of `doriac` in Doria itself.
- Add string interpolation AST nodes independent of PHP behavior.
- Emit precedence-aware backend expressions.

## Repository Work

- Keep CI green.
- Add branch rulesets in GitHub after the first CI run.
- Enable Dependabot, secret scanning, and push protection in repository settings.
