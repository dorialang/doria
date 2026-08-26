# 0002 Default-accessible and package-internal declarations

Status: Accepted

## Decision

Class members are externally accessible by default. Doria does not use `public`, `protected`, or `private` member visibility modifiers.

`internal` marks implementation details and controls API surface. It is
accessible throughout the declaring package and inaccessible to other
packages. This applies to top-level declarations, constructors, methods,
properties, and class constants. `writable` controls mutation.

## Notes

`internal` does not imply writable, and writable does not imply internal. Protected is permanently excluded from Doria; inheritance does not add a third access tier.
