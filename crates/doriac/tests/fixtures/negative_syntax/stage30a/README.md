# Stage 30a Malformed Syntax Inventory

These fixtures keep malformed callable grammar separate from the accepted Stage
30a syntax fixture. Parser tests assert deliberate diagnostics and recovery for
each form.

- `invalid-invocation-mode.doria`
- `conflicting-parameter-mode.doria`
- `missing-effect.doria`
- `tuple-like-group.doria`
- `named-callable-argument.doria`
- `ambiguous-nested-effects.doria`
