# Roadmap

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

## Repository Work

- Keep CI green.
- Add branch rulesets in GitHub after the first CI run.
- Enable Dependabot, secret scanning, and push protection in repository settings.
