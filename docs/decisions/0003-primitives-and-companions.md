# 0003 Primitives and companions

Status: Accepted

## Decision

Doria uses lowercase primitive type names:

```text
int, float, string, bool
```

PascalCase companion/helper objects provide related functions:

```text
Int, Float, String, Bool
```

## Notes

`int::parse` is invalid. `Int::parse` is the intended form. Primitive names are types, not namespaces.

Decision 0069 supersedes the earlier idea that `object` and `resource` are primitive type names. Doria has no `object` type, and `resource` is reserved for future PHP interop rather than a usable core primitive.
