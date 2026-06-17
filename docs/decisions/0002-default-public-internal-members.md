# 0002 Default-public internal members

Status: Accepted

## Decision

Class members are externally accessible by default. Doria does not use `public`, `protected`, or `private` member visibility modifiers in the early language.

`internal` marks implementation details and controls API surface. `writable` controls mutation.

## Notes

`internal` does not imply writable, and writable does not imply internal. No inheritance-oriented `protected` behavior is part of early Doria.
