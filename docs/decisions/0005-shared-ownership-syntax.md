# 0005 Shared ownership syntax

Status: Accepted

## Decision

The surface syntax for shared ownership is:

```doria
shared new AppConfig(...)
```

The likely explicit type form is:

```doria
shared AppConfig $config = shared new AppConfig(...);
```

## Notes

`shared` is a Doria ownership modifier, not a Rust-style wrapper exposed as primary syntax. Shared ownership should not automatically imply shared mutation. Weak ownership remains an open question.
