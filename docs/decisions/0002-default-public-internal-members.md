# 0002 Default-accessible internal members

Status: Accepted

## Decision

Class members are externally accessible by default. Doria does not use `public`, `protected`, or `private` member visibility modifiers.

`internal` marks implementation details and controls API surface. `writable` controls mutation.

## Notes

`internal` does not imply writable, and writable does not imply internal. Protected is permanently excluded from Doria; inheritance does not add a third access tier.
