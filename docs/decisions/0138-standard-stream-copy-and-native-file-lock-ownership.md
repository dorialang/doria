# Decision 0138: Standard-Stream Copy And Native File-Lock Ownership

- **Status:** Accepted
- **Date:** 2026-10-18
- **Owners:** Doria language and standard-library design
- **Scope:** Stage 36a standard-device views and file-lock ownership
- **Implementation status:** file locks implemented 2026-11-06 on the
  interpreter, Cranelift, LLVM and PHP (Stage 36a decision log entry 32);
  verified on macOS only. Windows `LockFileEx` waiting is written but unrun.

## Approval

Andrew approved `1A, 2A, 3A` and instructed: "Record the decisions and resume
Stage 36a." This record amends Decision 0110's standard-view classification and
advisory-only file-lock wording. Remaining public declarations still belong to
Decision 0110's bounded appendix; this approval does not invent their names.

## Standard-Device Views

Standard input, output, and error views are inline Copy values. Assignment such
as `$second = $input` leaves both bindings usable. The process owns the device;
neither view acquires a close obligation. Accessing or copying a view creates no
new native handle, control block, buffer, or heap object.

All copies and existing intrinsics use the same device owner, redirection,
injection, access coordination, and mode-restoration mechanism. Copy does not
grant independent exclusive access. Mode mutation still requires exclusive
control and a restoring guard, as required by Decision 0110.

Andrew subsequently approved S-A: `Doria\Std\Io\StandardStreams` has
readonly static `input`, `output`, and `error` properties returning
`StandardInput`, `StandardOutput`, and `StandardError` respectively.
Decision 0110's Standard-Stream Accessors appendix records this public spelling
ruling; the Copy and device-ownership rules here remain unchanged.

### Copy-To-Interface Borrowed Returns

On 2026-10-18 Andrew approved **C-B: Own erased copy first**. Converting an
inline Copy value to an owning interface creates an independent erased Copy
snapshot. A call may borrow that implicit temporary during its statement. A
returned borrow cannot survive the statement that owns the temporary.

To store the returned borrow, first declare the interface owner:

```doria
ReadinessStream $owner = $input;
let $view = borrowInput($owner);
```

The borrow follows `$owner` and its existing lifetime and access rules. The
original Copy binding remains independent and may be reassigned while `$view`
is live. Direct `let $view = borrowInput($input);` is rejected when `$input`
requires Copy-to-interface erasure. No implicit lifetime extension or loan of
the original Copy storage is introduced. This applies to nullable erasure and
all callable forms through the shared conversion and borrowing rules.

This chooses convenient value transport for a process-owned device. Move-only
views would restrict assignment without eliminating aliases obtained through
separate accesses to the same standard device.

## Borrowed And Owning File-Lock Guards

File locking supports both explicit borrowed and explicit owning constructions.
A borrowed guard retains a checked loan of the caller's file and releases the
lock without closing that file. An owning guard carries the file's ownership
and its lock obligation together, allowing a newly opened locked file to be
returned independently. Neither form duplicates the underlying file owner.

Both forms release deterministically on ordinary and checked-error exits.
Destructor cleanup is best-effort and nonthrowing; explicit operations report
checked failures. Nonblocking acquisition distinguishes contention through a
typed outcome and preserves actual operating-system failures. Public factory,
member, range, wait, and failure declarations remain subject to the appendix.

Supporting both constructions preserves temporary locking and independently
returned locked files without ownership workarounds. Borrowed-only and
owning-only APIs each exclude one of these uses.

## Native Platform Locking

File locks use native platform mechanisms with documented platform/filesystem
differences. Doria does not promise a uniform cooperation-only protocol or that
programs ignoring locking can always continue ordinary file access. Native
interoperability and operating-system lock cleanup motivate this amendment.

In particular, Windows exclusive byte-range locks can deny other processes
ordinary reads and writes; shared locks deny writes, including by the locking
process. Memory-mapped views have different restrictions. Closing the handle
or process termination releases its locks, with no promise of instantaneous
release. These facts follow the
[native Windows contract](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-lockfileex).
The private whole-file substrate uses `flock` on Linux/macOS and `LockFileEx`
on Windows. Local Unix locks are advisory; Linux NFS/SMB behavior depends on the
filesystem, server and mount configuration. Linux local `flock` locks generally
do not interact with `fcntl` record locks. These are explicit native limits,
documented by the [Linux manual](https://man7.org/linux/man-pages/man2/flock.2.html)
and [Apple manual](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/flock.2.html).
This internal whole-file implementation does not settle the deferred public
range API or promise equivalent Windows enforcement.

The same file owner and lock implementation serve borrowed and owning guards.
No global sidecar protocol, private lock registry, duplicate file handle, or
PHP-only locking path is introduced. Abort-only panic still runs no Doria
destructors; native process cleanup supplies only the host's guarantees.

## Public Lock Declarations

Andrew approved "Guard classes + typed outcomes (Recommended)" on 2026-11-05,
settling these `Doria\Std\Io` declarations:

```doria
enum FileLockMode { case Shared; case Exclusive; }
enum LockOutcome<G, F> { case Acquired(G $guard); case Contended(F $file); }
enum BorrowedLockOutcome<G> { case Acquired(G $guard); case Contended; }
class FileLock<F> {
    F $file { writable borrowed get; }
    consuming function unlock(): void throws IoError;
}
class LockedFile<F> {
    F $file { writable borrowed get; }
    consuming function unlock(): F throws IoError;
}
```

`ReadableFile`, `WritableFile` and `DuplexFile` each add, with `F` naming that
class:

```doria
writable function lock(FileLockMode $mode): FileLock<F> throws IoError;
writable function tryLock(FileLockMode $mode): BorrowedLockOutcome<FileLock<F>> throws IoError;
consuming function lockOwned(FileLockMode $mode): LockedFile<F> throws IoError;
consuming function tryLockOwned(FileLockMode $mode): LockOutcome<LockedFile<F>, F> throws IoError;
```

`FileLock` borrows the caller's file; `LockedFile` owns the file and its lock.
A contended owning attempt returns the file. Destruction unlocks
best-effort; explicit unlock reports failures. Unsupported locking is an
`IoError`. Blocking acquisition waits through the native lock call.

`IoOperation` gains `Lock` and `Unlock`. Every acquisition method fails with
`IoOperation::Lock`, and explicit `unlock` fails with `IoOperation::Unlock`.
Both use `IoTarget::File` with the file's path and `null` byte progress.
Unsupported locking uses `IoErrorReason::Unsupported`. Contention never raises
an error; it is the `Contended` outcome. A best-effort unlock during destruction
reports nothing. The added cases preserve existing tags but require new
exhaustive-match arms, as the readiness cases did.

## Invalidated Elsewhere

- Decision 0110 and its audit/manifest/guards must replace advisory-only file
  locking with native platform locking and preserve typed contention/failures.
- The plan, specification, stdlib index, compiler ownership/inline-value facts,
  runtime/backend transports and regression cases must classify standard views
  as non-owning, nonclosable inline Copy values over shared device state.
- File-lock lifetimes, cleanup, runtime implementations and tests must cover
  both borrowed and owning constructions through one native owner.
- Coordinated Tooling descriptions and the Website's versioned guide and API
  reference must reflect these contracts. Standard-stream names are recorded
  in Decision 0110; lock names are recorded under Public Lock Declarations
  above. This record makes no
  implementation, installed-tool, pin or publishing claim.
