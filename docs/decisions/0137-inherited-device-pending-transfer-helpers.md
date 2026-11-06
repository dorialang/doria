# Decision 0137: Inherited-Device Pending-Transfer Helpers

- **Status:** Accepted
- **Accepted:** 2026-10-08
- **Implementation Status:** In Progress
- **Amends:** Decision 0110

## Context

Inherited standard devices are not necessarily completion-capable. Windows
synchronous handles cannot become overlapped handles by association with a
completion port. Unix descriptors may share open-file-description flags with
another standard channel or an external owner. Changing `O_NONBLOCK` without
exclusive mode ownership can therefore alter unrelated operations.

Andrew approved lazy, bounded blocking helpers for pending transfers through
these real devices. This removes Decision 0110's blanket exclusion of hidden
workers only for this transport; it does not change ordinary stream execution.

## Decision

Use native readiness/completion where the real handle supports it safely.
Otherwise, a pending transfer through an inherited synchronous standard device
may use a group-owned blocking helper. Do not replace the handle, change shared
blocking flags without proved authority, or simulate completion with read-ahead.

Helpers initialize only when such a transfer is submitted. Their number is
bounded by the group's configured transfer capacity; idle watches and ordinary
synchronous I/O start none. Reuse stable group-owned request records and the
original caller or adapter buffer. No per-transfer wrapper allocation, hidden
buffer copy, general executor, or worker per registered stream is introduced.

Only native operating-system work runs on a helper. Adapter preparation,
completion, Doria execution, and PHP-runtime-affined callbacks remain on the
driving thread. The same group lifecycle, cancellation state, standard-device
access claims, canonical input queue, partial-progress facts, and original
checked errors apply to all transports.

Cancellation is a request, not retirement. Request cancellation for all pending
operations before draining any; release storage, device claims, and retained
loans only after actual host-operation retirement. Where the host cannot
interrupt a synchronous operation, cancellation and drain may wait for that
operation to return. A completion-fetch timeout still stops only the wait and
leaves the transfer live and available to await again.

## Alternatives And Consequences

- Rejecting inherited synchronous devices leaves first-class standard streams
  unable to participate in the accepted portable concurrent-transfer model.
- Changing shared flags, reopening devices, or substituting handles changes
  the inherited device contract and does not establish correct ownership.
- A general per-stream worker model adds cost to idle registration and ordinary
  I/O. It remains rejected; this exception has an explicit use and capacity bound.

The requested transport pays thread startup and synchronization costs. Tests
must cover lazy startup, bounded reuse, real buffer retirement, driver-thread
callbacks, shared-descriptor flags, cancellation/progress races, and repeated
wait-only expiry on tier-1 platforms. Controlled benchmarks report helper costs
separately from native completion; unavailable timing evidence is not a pass.
Later networking, process, terminal, and async work reuse this lifecycle rather
than introducing a second cancellation, buffer, or completion model.

**Invalidated elsewhere:** Decision 0110's unqualified hidden-worker exclusion;
SPEC and the end-to-end plan's readiness contract; standard-device and group
transport, ownership/structural regressions, and the I/O audit/guards; tooling
guidance; website versioned guides and API reference; benchmark transport
provenance. Ordinary no-eager-async and no-per-transfer-buffer-copy rules remain.
