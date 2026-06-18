# 0001 Native-first compiler pipeline

Status: Accepted

## Decision

Doria is a native-first compiled programming language. The long-term target is native machine code and standalone executables.

The public compiler mental model is:

```text
Doria source -> lexer -> parser -> AST -> semantic/type checking -> Doria IR -> backend
```

PHP is a compatibility, migration, debugging, and inspection backend only. It is not Doria's semantic reference and must not shape the core language architecture.

## Notes

Doria IR is the checked compiler-owned representation of a Doria program. As native code generation matures, Doria IR may lower into a simpler native-oriented IR for control flow, memory layout, runtime calls, and backend code generation.
