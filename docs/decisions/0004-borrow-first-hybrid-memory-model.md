# 0004 Borrow-first hybrid memory model

Status: Accepted

## Decision

Borrow checking is Doria's safety foundation. Ownership is the default, parameters borrow by default, and `writable` requires unique write access.

Shared ownership is explicit. Reference counting may be used where shared ownership is the right fit, and arenas or regions may be used for temporary or high-performance allocation.

## Notes

Resources require deterministic cleanup. Mandatory tracing garbage collection is not the foundation of the memory model.
