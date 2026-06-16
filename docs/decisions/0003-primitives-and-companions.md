# 0003 Primitives and companions

Status: Accepted

## Decision

Doria uses lowercase primitive type names:

```text
int, float, string, bool, object, resource
```

PascalCase companion/helper objects provide related functions:

```text
Int, Float, String, Bool, Object, Resource
```

## Notes

`int::parse` is invalid. `Int::parse` is the intended form. Primitive names are types, not namespaces.
