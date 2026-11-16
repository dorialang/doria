# Decision 0148: Unsafe Code And The Foreign Function Interface

- **Status:** Accepted. Andrew ruled every fork on cards on 2026-11-16. Nothing here is implemented; delivery is tracked in the end-to-end plan (Stage 40) and `docs/notes/current-pipeline.md`.
- **Date:** 2026-11-16
- **Owners:** Doria language, compiler, runtime and Baton design
- **Scope:** Stage 40 (plan D12): `unsafe` contexts, raw pointers, `extern "C"` declarations, the types that cross the C boundary, zero-copy exchange of Doria buffers, foreign-owned memory, callbacks from C, platform selection, linking foreign libraries through Baton and `doriac`, and the native companion library through which the interpreter and PHP output run unsafe code

## Approval And Boundary

Andrew answered these questions on cards on 2026-11-16. Each line gives the
question's subject and his selection, quoted.

1. Where the FFI vocabulary lives: "Module Doria\Std\Ffi (Recommended)".
2. Unsafe contexts: "Block plus expression form (Recommended)".
3. Pointer nullability and the untyped pointee: "Non-null Ptr, ?Ptr, Ptr<Opaque> (Recommended)".
4. Pointer operations: "Named verbs, checked alignment (Recommended)".
5. C types whose width varies by platform: "Distinct boundary types (Recommended)".
6. Foreign aggregates: "extern \"C\" struct and union (Recommended)".
7. How long pointers into Doria storage stay valid: "For the enclosing unsafe context (Recommended)".
8. Typed views over `T[]`: "ArrayRegion<T> for every T, no strides (Recommended)".
9. Strings across the boundary: "Explicit conversions (Recommended)".
10. Memory a foreign library allocates: "Owner-bound regions plus RAII (Recommended)".
11. Raw handles: "Owned and shared values (Recommended)".
12. Callbacks: "extern \"C\" functions (Recommended)".
13. Callback threads and the Call Path: "Checked thread, chained Call Path (Recommended)".
14. Calling conventions and variadic functions: "\"C\" only, typed per-arity (Recommended)".
15. Symbol names: "Name is the symbol, #[LinkName] when needed (Recommended)".
16. `errno` and `GetLastError`: "Opt-in capture per declaration (Recommended)".
17. Platform-specific bindings: "Constant Platform branches (Recommended)".
18. Who owns link facts: "Source names, Baton maps (Recommended)".
19. The Baton description of a native library: "Typed per-platform table (Recommended)".
20. The Windows C runtime: "UCRT only with foreign code (Recommended)".
21. The Windows GUI subsystem: "Baton target field, defined streams (Recommended)".
22. The interpreter: first "Native-only diagnostic, own manifest (Recommended)",
    then, the same day, "Use the companion too (Recommended)", which
    supersedes the first answer.
23. PHP output: Andrew rejected the recommended unsupported-feature
    diagnostic with "Im not looking for the quick answer but the correct
    answer... what makes Doria stand out and will save developers time?" and
    selected "Native companion library (Recommended)".
24. Standard-library runtime bindings: "Stay intrinsics, dr_ reserved (Recommended)".
25. Unsafe policy: "File, package and dependency audit (Recommended)".
26. Foreign calls inside property hooks: "Only if declared non-blocking (Recommended)".
27. Safety documentation: "Safety section, shown at call sites (Recommended)".

These rulings amend plan D3 for foreign layouts, replace the plan's
`Ptr<void>` with `Ptr<Opaque>`, move native-library declarations from plan
Appendix A's deferral into Stage 40, and give the interpreter and PHP output
native execution of unsafe code. The amendments are listed under Invalidated
Elsewhere.

## Context

Stage 40 is the second item of the approved execution sequence (plan §13).
Its consumers are concrete:

- **The Stage 40 acceptance criteria**: bind and call zlib, and a zero-copy
  `Bytes`-view round trip.
- **The native-window checkpoint** that follows immediately. Win32 needs C
  structs (`WNDCLASSEXW`, `MSG`), UTF-16 strings, a window procedure that
  receives no user-data argument, and an executable with no console. macOS
  needs framework linking and `objc_msgSend` called through several typed
  signatures. Linux needs X11 or Wayland client libraries found through
  pkg-config and an event union. One application source must select among the
  three.
- **raylib and C-library bindings** (AGENTS.md, Project): raylib passes small
  structs by value throughout its API.
- **Stage 41**: the php-lib bridge needs C-callable Doria entry points and a
  native shared-library output, transport- and direction-neutral (plan §10.3).
- **The Doria-native Baton transition** (Decision 0124) consumes "Stage 40 FFI
  and native-library integration where a portable library needs it".
- **AI-0 readiness** (plan Appendix A): "transport-neutral FFI with zero-copy
  views".

### Facts confirmed in the tree on 2026-11-16

- `unsafe`, `extern` and `declare` are not keywords
  (`crates/doriac/src/lexer.rs:182-259`). Doria has no documentation-comment
  form.
- `doriac run` compiles a native executable and runs it
  (`crates/doriac/src/main.rs:1230-1300`); the MIR interpreter runs for
  `--target debug` (`main.rs:2029`).
- On Unix the compiler links with `cc <object> <runtime archive> -o <exe>`,
  honoring `CC` (`crates/doriac/src/codegen_native.rs:177-180`, `472-518`).
- On Windows MSVC the compiler links with `/ENTRY:main`, `/SUBSYSTEM:CONSOLE`,
  `kernel32.lib`, `shell32.lib` and `bcrypt.lib` and no C runtime
  (`codegen_native.rs:478-503`). The freestanding runtime supplies `memcpy`,
  `memmove`, `memcmp`, `memset`, `strlen`, `wcslen`, `_fltused`, `__chkstk`
  and a `__CxxFrameHandler3` stub (`crates/doria-rt/src/lib.rs:4680-4880`,
  feature `standalone-windows-support`).
- The runtime allocator is `malloc`/`free` on Unix and `HeapAlloc`/`HeapFree`
  on the process heap on Windows, with two-word natural alignment
  (`lib.rs:4569-4598`).
- Panic and `exit` end in `_exit` on Unix and `ExitProcess` on Windows
  (`lib.rs:4646-4660`).
- `Bytes` is a runtime header `{ length, data }` over a separate fixed-length
  allocation (`crates/doria-rt/src/bytes.rs:6-10`); `bytes::from_owned` adopts
  only runtime-allocated storage (`bytes.rs:28-30`).
- `T[]` storage is contiguous at natural width for scalars: `bool` 1 byte,
  integers their width, `float32` 4, `float64` 8, enum tags 4
  (`crates/doria-rt/src/collection.rs:30-40`;
  `crates/doriac/src/native_abi.rs:549-559`).
- Every Doria callable receives hidden inputs, always including the current
  diagnostic frame (`crates/doriac/src/native_closure_abi.rs:521-529`;
  `codegen_cranelift.rs:1740-1743`). A Doria function is never a C function.
- Native code generation targets the host only (`codegen_llvm.rs:125`).
- The build plan has no link inputs; every struct is `deny_unknown_fields`
  (`crates/doriac/src/build_plan.rs:10-19`, `112-117`).
- Standard-library modules reach the runtime through `internal static
  function` bodies bound to `CompilerIntrinsic` variants
  (`crates/doriac/src/compiler_known_term.rs:1-8`, `311-312`;
  `crates/doriac/src/ast.rs:498-508`).
- PHP output already loads a native library: the hosted shared runtime, through
  `FFI::cdef` with a path relative to the generated file and a validated
  metadata sidecar (`crates/doriac/src/codegen_php/native_runtime.rs:172-195`;
  `runtime_artifact.rs:252-255` for the `hosted-shared-runtime` feature set).
- The parity manifest lists 428 example paths
  (`crates/doriac/tests/fixtures/native_parity_examples.txt`).

### Inconsistencies in the sources, and how this record resolves them

1. Plan §3.5 used `Ptr<void>`, which Decision 0069 (line 27), plan §4.8 and SPEC
   §7 forbid. Resolved: `Ptr<Opaque>` (ruling 3); plan §3.5 amended.
2. Plan §4.9 placed zero-copy views over `uint8[]`/`Bytes` in the unsafe tier;
   Decision 0110 had already delivered safe byte regions. Resolved: views are
   safe regions and only their pointer accessors need `unsafe` (rulings 7 and
   8); plan §4.9 amended.
3. Plan §8.3 required Decision 0044 to evaluate external memory, custom
   deallocation, alignment and pinning; 0044 recorded none. Resolved: this
   record settles them (rulings 4, 6, 7, 10); plan §8.3 amended with what
   remains.
4. Plan D3 forbade user structs while the plan's goals need C-layout
   aggregates. Resolved: foreign layouts only (ruling 6); D3 amended.
5. Plan Appendix A deferred the "Baton native-dependency" model while Stage 40
   links through the Baton manifest. Resolved: native-library declarations are
   Stage 40 (rulings 18 and 19); Appendix A amended to keep only build scripts
   and native feeds deferred.
6. Plan D18 says the runtime is internal "until v1.0"; AGENTS.md says "never
   treated as stable". Ruling 24 applies the AGENTS.md reading to FFI: `dr_`
   symbols are reserved. D18's wording is unchanged and still differs.
7. Plan §3.5 did not say whether an `unsafe function` body is an unsafe
   context. Resolved: it is an ordinary context (ruling 2); §3.5 amended.
8. Decision 0142 (Proposed, untracked) leaves passing math values in vector
   registers across FFI to Stage 44; the C ABI fixes those registers for a
   by-value C-layout aggregate. Recorded here only; 0142 belongs to another
   session.
9. Decision 0141 (Proposed, untracked) places the `declare` grammar in Stage 47.
   Stage 40 runs first and now implements it (ruling 25). Recorded here only;
   0141 belongs to another session.

## Decision

### Fixed by the plan and accepted records

- `unsafe` gates pointer access, foreign calls and deliberate ownership
  sidesteps; everything outside keeps full guarantees (plan D12, §3.5).
- `extern "C"` blocks declare foreign symbols as bodyless functions (plan §3.5).
- The safety contract: unsafe code upholds the invariants the ownership checker
  assumes (plan D12).
- FFI must not copy all buffers by default (plan Stage 40 entry).
- `doriac` never parses `Baton.toml`; packages have no build scripts
  (Decisions 0117, 0118).
- `Ptr<T>` is not display-convertible (plan §4.6); `resource` stays reserved
  for the PHP bridge (plan §4.8).
- No implicit conversions (plan D15); checked arithmetic in both profiles (plan
  D4); abort-only panics that never unwind (plan §3.6, Decision 0040).
- Class payloads are private (Decision 0082); shared ownership exposes no
  FFI-visible layout (Decision 0106); bridge handles are separate from both
  shared families (plan §10.3).
- Strings carry an explicit length and may contain NUL (Decision 0045); invalid
  UTF-8 never enters a `string`.
- Platform polling and handle types never enter the Doria surface (SPEC §7).
- Tier-1 platforms are Linux, macOS and Windows on x86_64, and aarch64 on Linux
  and macOS (plan §8.6); runtime additions land with Windows in the same change.
- Two clocks: accepted syntax parses before its semantics land.

### 1. The `Doria\Std\Ffi` module

`Ptr`, `MutPtr`, `Opaque`, the C boundary types, `CString`, `Utf16String`,
`Handle`, `ForeignCall`, `Platform`, `OperatingSystem`, `Architecture`,
`SystemCodeSource` and the FFI attributes (`Library`, `LinkName`, `Variadic`,
`CaptureSystemCode`, `NonBlocking`) are compiler-known members of
`Doria\Std\Ffi`. A file imports what it uses:
`use Doria\Std\Ffi\{Ptr, MutPtr, Opaque};`. The keywords `unsafe` and `extern`
are language-level. `ArrayRegion<T>` and `WritableArrayRegion<T>` are core
types beside `T[]` (section 8). `String::fromCString` and `String::fromUtf16`
are `String` companion members (Decision 0103).

Consequence: an FFI import line marks every file that touches foreign memory,
and the names stay available to programs that do not import them.

### 2. Unsafe contexts

An unsafe context is either a statement block or a parenthesized expression:

```doria
unsafe {
    $status = deflate(MutPtr::to($stream), Z_FINISH);
}
let $bound = unsafe(compressBound($sourceLength));
```

- `unsafe { }` is a statement with Decision 0107's lexical scope and cleanup.
  `return`, `break` and `continue` inside it keep their ordinary meaning.
- `unsafe(<expression>)` covers exactly that expression and yields its value.
- An `unsafe function` (free function, instance method or static method)
  requires an unsafe context at every call. Its body is an ordinary context;
  its unsafe operations need their own blocks or expressions.
- An unsafe context does not extend into a closure body or nested declaration
  written inside it.
- An `unsafe function` cannot be converted to a function value.
- An override may omit `unsafe` from an inherited `unsafe function`; an override
  may not add `unsafe` to a safe parent method. An interface may require an
  `unsafe function`; calls through the interface need an unsafe context.
- A nested unsafe context inside another is redundant and draws the unused
  `unsafe` warning.

Consequences: every unsafe region is as small as its author writes it, and it is
the unit sections 7, 22 and 23 build on. Plan §3.5 records the body rule.

### 3. Pointer types

- `Ptr<T>` (read access) and `MutPtr<T>` (read and write access) are Copy, one
  machine word, and never null.
- `?Ptr<T>` and `?MutPtr<T>` are C nullable pointers: one word, null is address
  zero, narrowed with `is`, `??` and `?->` (Decision 0093).
- `Opaque` is a compiler-known pointee-only type with no values and no size.
  `Ptr<Opaque>` and `MutPtr<Opaque>` spell C's `const void *` and `void *`.
- A bodyless foreign struct declares a named incomplete C type:
  `extern "C" struct ZStream;` gives `Ptr<ZStream>`.
- `void` remains return-position only (Decision 0069).

Consequence: the checker proves which pointers may be null at compile time, and
`?Ptr<T>` passes to C unchanged.

### 4. Pointer operations

| Operation | Spelling | Context |
| --- | --- | --- |
| Aligned read | `$pointer->read(): T` | unsafe |
| Unaligned read | `$pointer->readUnaligned(): T` | unsafe |
| Aligned write | `$mutable->write(T $value): void` | unsafe |
| Unaligned write | `$mutable->writeUnaligned(T $value): void` | unsafe |
| Element offset | `$pointer->offset(int $count)` scaled by `T`'s size | unsafe |
| Byte offset | `$pointer->byteOffset(int $bytes)` | unsafe |
| Cast | `Ptr::cast($pointer)`, `MutPtr::cast($pointer)`, target from the expected type | unsafe |
| From address | `Ptr::fromAddress(uint64 $address): ?Ptr<T>` | unsafe |
| Address | `$pointer->address: uint64` | safe |
| Drop write access | `Ptr::from(MutPtr<T> $pointer): Ptr<T>` | safe |
| Copy, `==`, `!=`, null narrowing | ordinary | safe |

- `read` and `write` panic with a catalogued code on a misaligned address in
  both profiles; the unaligned forms carry no alignment requirement.
- Address arithmetic that overflows panics with a catalogued code.
- `offset`, `read` and `write` on `Ptr<Opaque>` are compile errors.
- `T` in a read or write is an FFI-safe type (section 6).

Consequence: every unchecked memory access is a named verb a search finds, and
behavior is identical in the fast and release profiles.

### 5. C boundary types

`CChar`, `CLong`, `CUnsignedLong`, `CSize`, `CPointerDifference` and
`CWideChar` are inline Copy values with the target C type's width and
signedness: `long` and `unsigned long` are 32-bit on Windows and 64-bit on
Linux and macOS; `char` is signed on x86_64 and Apple arm64 and unsigned on
Linux aarch64; `wchar_t` is 16-bit on Windows and 32-bit elsewhere; `size_t`
and `ptrdiff_t` are pointer-width.

- `CLong::from($value)` accepts any integer and panics with a catalogued code
  when the value does not fit on this target; `CLong::tryFrom($value)` returns
  `?CLong`. Each boundary type has both.
- The fixed-width companions accept boundary values: `Int64::from($length)`,
  checked as usual.
- Boundary values support `==` and `!=`; they have no arithmetic, ordering or
  display conversion.
- C types fixed on every tier-1 target bind as Doria types: `int` as `int32`,
  `unsigned int` as `uint32`, `short` as `int16`, `long long` as `int64`,
  `float` as `float32`, `double` as `float64`, `_Bool` as `bool`,
  `unsigned char` as `uint8`, and the `<stdint.h>` types by width.

Consequence: a binding type-checks identically on every platform, and the only
platform difference is a checked range at an explicit conversion. SPEC §7's
"no pointer-width integer type" holds for ordinary Doria arithmetic.

### 6. Foreign structs and unions

```doria
extern "C" struct Color
{
    uint8 $red;
    uint8 $green;
    uint8 $blue;
    uint8 $alpha = 255;
}

extern "C" union EventStorage
{
    KeyEvent $key;
    ButtonEvent $button;
    CLong[24] $padding;
}
```

- A foreign struct is an inline Copy value laid out by the target C rules in
  declaration order. A foreign union overlays its members at offset zero with
  the size and alignment of its largest member.
- Field types are FFI-safe. A field may declare a constant default; every field
  without one must be supplied at construction: `new Color(red: 255, green: 0,
  blue: 0)`.
- A fixed-length array field spells its element type and count
  (`uint16[128] $tip`) and is accessed as an `ArrayRegion<T>` view borrowing the
  containing value; a writable containing place gives `WritableArrayRegion<T>`.
- The compiler provides constants `Color::SIZE` and `Color::ALIGNMENT`.
- A foreign aggregate has no methods, hooks, constructors, destructor,
  conformances or type parameters.
- Reading a union member is unsafe; writing one is safe.
- Structs and unions pass to and return from foreign functions by value and by
  pointer. One shared aggregate classification in the native ABI layer covers
  System V x86_64, Windows x64, AAPCS64 and Apple arm64, and both Cranelift
  and LLVM consume it; neither backend classifies aggregates itself.
- Bit fields and explicit packing are deferred to a later decision with a
  consumer; a C struct that needs them cannot be declared in Stage 40.
- Foreign aggregates are externally accessible types under Decision 0117: one
  per file unless `internal`, with generated roots as the bundled form.
- `Doria\Std\Math` values become FFI-safe by value through the same
  classification once Decisions 0141 and 0142 are accepted.

Consequence: plan D3 gains one exception, foreign C layouts declared with
`extern "C"`. Classes stay heap move types with private payloads.

FFI-safe types are: fixed-width integers and floats, `bool`, `Ptr<T>`,
`MutPtr<T>` and their nullable forms, the boundary types, C function pointer
types (section 12), and foreign structs and unions.

### 7. Pointers into Doria storage

Formation:

- `$region->pointer: Ptr<uint8>` on `ByteRegion` and `WritableByteRegion`;
  `$writableRegion->writablePointer: MutPtr<uint8>` requires writable access to
  the region. Typed regions have the same members with `Ptr<T>` and `MutPtr<T>`
  when `T` is FFI-safe.
- `$text->byteRegion->pointer` addresses a string's UTF-8 bytes, without a NUL
  terminator.
- `Ptr::to($place)` and `MutPtr::to($place)` address an FFI-safe Copy place: a
  local, a foreign struct field, or a property of FFI-safe type. `MutPtr::to`
  takes a `writable` parameter, so the place must be writable.
- A class payload has no pointer form; classes reach C only as raw handles
  (section 11).
- An empty region's pointer is a non-null address aligned for `T` that must not
  be dereferenced.

Validity: forming a pointer extends the source's loan to the end of the
innermost enclosing unsafe context. Within it the source cannot be moved,
destroyed or accessed in a conflicting way. The pointer is valid only within
that context on every target; using it after the context ends is undefined
behavior. Pointers to foreign memory (from C) follow the foreign library's own
rules and are unaffected.

Consequence: the checker pins storage exactly for the audited region, which
covers multi-call protocols such as zlib's streaming `deflate`. The same
boundary makes copy-in and copy-out at a companion region's edge unobservable
(section 23).

### 8. Typed array regions

`ArrayRegion<T>` (readonly, borrowed Copy carrier) and `WritableArrayRegion<T>`
(borrowed move carrier) are core views over `T[]` for every element type,
mirroring Decision 0110's byte regions:

- `$values->getRegion(int $offset, int $length): ArrayRegion<T>`;
  writable `$values->getWritableRegion(int $offset, int $length): WritableArrayRegion<T>`.
- Both have readonly `int $length`, checked indexing, and `getRegion`; the
  writable form has `getWritableRegion`, indexed writes through a writable
  binding, and `copyFrom(ArrayRegion<T> $source): int`.
- `pointer` and `writablePointer` exist when `T` is FFI-safe (section 7).
- Formation allocates nothing and copies nothing; bounds panic as `T[]`
  indexing does; borrow rules match the byte regions.
- No strided view exists in core. Shapes, ranks and strides belong to the
  `NDArray` storage library of plan Appendix A; a binding passes a stride to C
  as an integer.

Consequence: this delivers the `T[]` slicing that plan §9.1 left as a future
addition, and zero-copy numerical exchange reaches C through a safe value.

### 9. Strings

- `CString::from(string $text): CString` copies the UTF-8 bytes and appends NUL;
  it panics with a catalogued code on an interior NUL.
  `CString::tryFrom(string $text): ?CString` returns null instead.
  `$cString->pointer: Ptr<CChar>` borrows it.
- `Utf16String::from` and `Utf16String::tryFrom` produce NUL-terminated UTF-16
  for the Windows `W` APIs, with `$utf16->pointer: Ptr<uint16>`.
- `String::fromCString(Ptr<CChar> $text): ?string` (unsafe) copies up to the
  NUL and returns null for invalid UTF-8; `String::fromUtf16(Ptr<uint16> $text):
  ?string` (unsafe) returns null for unpaired surrogates. Both follow
  `String::fromBytes` (Decision 0103).
- APIs taking pointer and length use `$text->byteRegion`.
- An extern signature never converts a `string` implicitly; `string` is not
  FFI-safe.

Consequence: every allocation and copy at the boundary is visible in source,
and a returned C string's ownership stays with the binding author.

### 10. Memory a foreign library allocates

- `ByteRegion::fromRaw($owner, Ptr<uint8> $data, int $length)` and
  `WritableByteRegion::fromRaw(writable $owner, MutPtr<uint8> $data, int $length)`,
  and the typed `ArrayRegion::fromRaw` / `WritableArrayRegion::fromRaw`, are
  unsafe constructors whose region borrows `$owner`.
- The owner is an ordinary class whose `__destruct` calls the library's own
  release function in an unsafe context. A method returning the region follows
  Decision 0089's elision to `$this`.

```doria
class DecodedImage
{
    function __construct(internal MutPtr<uint8> $pixels, internal int $length) {}

    function getPixels(): ByteRegion
    {
        return unsafe(ByteRegion::fromRaw($this, Ptr::from($this->pixels), $this->length));
    }

    function __destruct()
    {
        unsafe { freeImage($this->pixels); }
    }
}
```

- Doria never passes its own allocations to foreign code for release; the
  runtime allocator is private (Decision 0044) and differs from the CRT heap on
  Windows.

Consequence: every release protocol is stated by its owner class, and the
region works with every region-taking API, including the Decision 0110 streams.

### 11. Raw handles

- Owned class values: `Handle::intoRaw(take T $value): MutPtr<Opaque>`;
  `Handle::fromRaw(MutPtr<Opaque> $handle): T` (unsafe) reclaims ownership.
- `SharedReference<T>`: `$reference->intoRaw(): MutPtr<Opaque>` consumes one
  strong claim; `SharedReference::fromRaw($handle)` (unsafe) reclaims it.
- `WritableSharedReference<T>`: the same pair, preserving its family and its
  runtime access checks.
- Borrowing forms lend without reclaiming, for the enclosing unsafe context:
  `Handle::borrowRaw($handle)` (readonly access to `T`),
  `SharedReference::borrowRaw($handle)`, `WritableSharedReference::borrowRaw($handle)`.
- `T` comes from the expected type. A handle reclaimed or borrowed as the wrong
  type or family is undefined behavior.
- Raw handles are not Stage 41 bridge handles, and no conversion exists between
  them.

Consequence: a callback re-entered while it runs (Win32 `SendMessage` inside a
handler) reaches application state through `WritableSharedReference<T>`, whose
second writable access panics where an unchecked pointer would alias.

### 12. Callbacks and C function pointers

```doria
extern "C" function windowProcedure(Ptr<Opaque> $window, uint32 $message,
    uint64 $wParam, int64 $lParam): int64
{
    ...
}
```

- A C-ABI Doria function is a namespace-level free function declared
  `extern "C" function` with a body. It has no captures, type parameters or
  `throws`, and its signature is FFI-safe. Its body is an ordinary context.
- Its name in expression position is a value of the structural type
  `extern "C" function(Ptr<Opaque>, uint32, uint64, int64): int64`; the nullable
  form is `?extern "C" function(...)`. These values are Copy and one word.
- Calling a C function pointer value is unsafe.
- `Ptr::toFunction(Ptr<Opaque> $address)` (unsafe) converts an address from
  `dlsym` or `GetProcAddress` to the C function pointer type of the expected
  type.
- A checked error must be handled inside the body. A panic aborts.
- Stage 40 exports no symbol for a C-ABI function; C reaches it only through its
  address. Stage 41 reuses the same C entry machinery for exports.
- Closures do not convert to C function pointers. State travels through a
  context pointer and section 11's handles.

### 13. Callback threads and the Call Path

- Every C entry compares the current thread with the thread running Doria code
  and panics with a catalogued code on a mismatch.
- Each outbound foreign call records the current Doria frame in a per-thread
  runtime slot. A C entry links its frame to that record, so a panic shows
  `main → render → [foreign call] → windowProcedure`. An entry with no active
  outbound call starts a new chain.
- Foreign-thread callbacks become possible only through the concurrency
  decision (Stage 39).

Consequence: non-atomic reference counts (Decisions 0045, 0106) cannot be
corrupted from a foreign thread unnoticed, and Decision 0109's Call Path stays
continuous through foreign frames.

### 14. Calling conventions and variadic functions

- `extern "C"` is the only ABI string. It means the target's C convention:
  System V x86_64 on Linux and macOS x86_64, Windows x64, AAPCS64 on Linux
  aarch64, Apple arm64 on macOS. Any other string is a compile error naming
  `"C"`.
- A variadic C function is bound once per used arity, with the count of fixed
  parameters:

```doria
extern "C"
{
    #[LinkName("printf")]
    #[Variadic(fixedParameters: 1)]
    function printInteger(Ptr<CChar> $format, int32 $value): int32;
}
```

- Parameters after the fixed ones must already have their promoted C types: no
  `float32`, no integer narrower than 32 bits, no `bool`.

Consequence: every call stays fully typed, and Apple arm64's stack placement of
variadic arguments is known at the declaration.

### 15. Symbol names and declaration rules

- A declaration's Doria name is its symbol. `#[LinkName("...")]` names a
  different symbol and is required when the symbol is a reserved compiler
  intrinsic name (`printf`, `read_line`), is not a valid Doria identifier, or is
  bound more than once with different signatures (`objc_msgSend`).
- Two declarations of one symbol with different signatures in one program are a
  compile error unless every one carries `LinkName`.
- Extern declarations are exempt from the naming charter's snake_case
  free-function lint.
- Extern blocks appear at namespace level. Their functions resolve as free
  functions of the enclosing namespace (Decision 0117). `internal extern "C" { }`
  keeps raw bindings package-private.
- Declarations accept named arguments (Decision 0098) and constant defaults
  (Decision 0086). Parameters take no `writable` or `take` mode.
- An extern block without `#[Library]` binds the platform C runtime (libc on
  Linux, libSystem on macOS, the UCRT on Windows).

### 16. `errno` and `GetLastError`

- `#[CaptureSystemCode(SystemCodeSource::Errno)]` or
  `#[CaptureSystemCode(SystemCodeSource::WindowsLastError)]` on a declaration
  makes the compiler read that code immediately after the call, before any other
  code runs, into a per-thread runtime slot.
- `ForeignCall::lastSystemCode` (readonly static `int`) returns the value the
  last capturing call stored on this thread.
- `WindowsLastError` on another platform is a compile error in emitted code.

Consequence: the code survives the allocation and drop glue between a call and
its read, and only declarations that request it pay for it.

### 17. Platform constants

```doria
if (Platform::operatingSystem == OperatingSystem::Windows) {
    Win32Window::open($title);
} else if (Platform::operatingSystem == OperatingSystem::MacOs) {
    CocoaWindow::open($title);
} else {
    X11Window::open($title);
}
```

- `Platform::operatingSystem` (`OperatingSystem::Linux`, `MacOs`, `Windows`) and
  `Platform::architecture` (`Architecture::X8664`, `Aarch64`) are compile-time
  constants of the compilation target.
- Every branch is parsed and type-checked on every host. A branch whose
  condition is constant false for the target is neither emitted nor linked.
- A foreign library is required only when emitted code references it.

Consequence: Doria gains structured conditional compilation of the kind
Decision 0028 anticipated, and platform code is checked on every developer's
machine.

### 18. Link facts

```doria
#[Library("zlib")]
internal extern "C"
{
    function compressBound(CUnsignedLong $sourceLength): CUnsignedLong;
}
```

- Source names a logical library with `#[Library("name")]` on the extern block.
- The package's `Baton.toml` maps it per platform (section 19). Baton resolves
  the mapping, running pkg-config where named, and writes resolved link inputs
  into build-plan schema 2.
- `doriac compile` and `doriac run` accept the same inputs through repeatable
  flags, so direct compiler use links identically.
- Emitted code referencing a logical library with no resolved input is a
  compile-time diagnostic naming the manifest table and the flag.
- Link inputs of dependency packages reach the final link.

### 19. `[native-libraries]`

```toml
[native-libraries.zlib]
link = "dynamic"
linux = { pkg-config = "zlib" }
macos = { system = "z" }
windows = { library = "zlib", search-paths = ["native/windows/lib"], runtime-files = ["native/windows/bin/zlib1.dll"] }

[native-libraries.appkit]
macos = { framework = "AppKit" }
```

- Kinds: `pkg-config`, `system` (link by name through the default search),
  `library` with project-relative `search-paths`, and `framework` (macOS).
- `link` is `"static"` or `"dynamic"`.
- Baton copies `runtime-files` beside the built executable or companion and
  gives project-local Unix libraries an rpath relative to the loading artifact.
  `doriac run` and `baton run` make the same files reachable.
- Paths follow Decision 0118's containment rules. Resolved facts, including
  pkg-config output, are recorded in the build receipt.
- A platform without an entry has no such library, which is valid until emitted
  code references it.
- Native package feeds, prebuilt downloads and compiling C sources stay deferred
  (Decision 0118).

### 20. The Windows C runtime

- A program that links any foreign code, including an extern block without
  `#[Library]`, links the dynamic Universal CRT with the static `vcruntime`,
  uses a CRT-hosted runtime archive (the freestanding exports disabled, as the
  runtime's hosted builds already do), and enters through the CRT's startup. A
  normal return from `main` leaves through the CRT's exit, as on Unix; panic and
  `exit` keep `ExitProcess`.
- A program without foreign code keeps today's CRT-free entry.

Consequence: C libraries get an initialized C runtime, clean machines need no
redistributable, and existing Windows programs are unchanged.

### 21. The Windows GUI subsystem

- A Baton binary target may set `windows-subsystem = "gui"`; the default is
  `"console"`. The value travels through build-plan schema 2 and a `doriac`
  flag.
- When a standard stream is absent, writing to it discards the bytes and the
  program continues; its view's `isInteractive` is `false`; reading absent
  standard input reports end of input. Panic and unhandled-error statuses are
  unchanged, and their reports go to standard error when it exists.
- The rule applies to every platform where a stream can be absent.

Consequence: Decision 0091's closed-pipe rule (status 0 on a closed pipe) does
not apply to an absent stream, so a GUI program keeps running after `echo`.

### 22 and 23. The native companion library

Every target runs unsafe regions as native code. Cranelift and LLVM compile
them inline. For the interpreter and PHP output, `doriac` compiles the
program's unsafe regions (the extern bindings and the code inside each unsafe
context) into a native companion library, and the host (the interpreter or the
PHP output) calls each region as one function. FFI fixtures join the existing
parity manifest and run under the interpreter, Cranelift, LLVM and PHP output.

**Regions.** A region is one `unsafe { }` block or one `unsafe(<expression>)`,
per monomorphized specialization of its enclosing function. Its identity is a
hash of package identity, source identity, span and type arguments. An unsafe
function's body is ordinary host code; its inner regions are regions.

**Entry ABI.** Each region is exported from the companion as

```text
int32 doria_region_v1_<regionId>(DrRegionHostV1 *host,
                                 const void *callerFrame,
                                 void *inputs,
                                 void *outputs)
```

`inputs` is a C-layout record with one slot per place the region reads or
writes from its surroundings, in order of first use. `outputs` holds the final
value of each writable place, the expression result, a return value, a
break/continue target index, and an error token. The status is 0 for
fallthrough, 1 for `return`, 2 for `break`, 3 for `continue`, and 4 for a
checked error.

**Marshalling at the edge.**

| Value kind | Crossing |
| --- | --- |
| Integers, floats, `bool`, boundary types, pointers, C function pointers, foreign aggregates, Copy inline values (Copy enums, inline records, Time values) | By value in their native layout |
| `string` | UTF-8 bytes; the companion materializes a native string for the region; a string produced in the region returns as bytes and the host builds its own `string` |
| `Bytes`, `T[]` of Copy edge kinds, byte and array regions over host storage | Readonly: copied into a native buffer for the region. Writable: copied in, and copied back at every region exit |
| Regions over foreign memory (section 10) | Address and length, no copy |
| `CString`, `Utf16String` | Content re-materialized natively per region |
| Raw handles | Host handle tokens, by value |
| Every other value (class instances, interfaces, other collections, closures, `mixed`, shared references, nullable move values) | Stays on the host; the companion holds an opaque host token |

The borrow rules of section 7 make the copies unobservable: a writable pin
excludes every other access for the region, a readonly pin excludes mutation,
and a pointer into Doria storage is valid only inside its region.

**Host operations.** Inside a region, a call to a Doria function, method,
closure or hook, construction of a class, an operation on a host-owned value,
a static property access, and a Doria I/O or runtime intrinsic each run on the
host through a reverse edge: `host->call(host, operationId, arguments, results)`,
returning the same status set. Arguments and results cross by the table above.

**Callbacks.** The companion contains the C entry for each C-ABI Doria function.
The entry performs section 13's thread check and forwards to the host through a
reverse edge; the body runs on the host. C-ABI signatures are FFI-safe, so every
argument crosses by value.

**Ownership and destruction.** Host-owned values are created, moved and
destroyed on the host in ordinary Doria order. Values created in the companion
(native strings, `CString` buffers, copied buffers) are destroyed in the
companion in reverse order at region exit. Writable places are written back
before the host resumes, on every exit status. A value moved into a host
operation transfers its token.

**Raw handles in hosted mode.** `intoRaw`, `fromRaw` and `borrowRaw` are host
operations; the handle is a nonzero address-sized token from a host handle
table. C treats it as opaque, as section 11 requires.

**Foreign-backed regions on the host.** A region over foreign memory that leaves
an unsafe context is represented on the host as address and length; its reads,
writes and copies go through the shared runtime's byte accessors.

**Panics.** A panic inside the companion goes through the runtime-outcome
transport of Decision 0109 to a panic sink the host registered at load. The host
renders the report exactly as a native panic and ends the process with status
101 without returning into the companion. No cleanup runs on either side.
Harnesses that embed the interpreter run companion fixtures in a child process.

**Checked errors.** An Error raised inside a region is a host value (its
construction is a host operation). It propagates through region code with
Decision 0119's status transport and leaves with status 4; the host resumes
propagation with the original Error object.

**Call Path.** The host passes its current frame as `callerFrame`; companion
frames link to it, and reverse edges pass the companion frame back, so the
rendered Call Path matches the native one.

**Thread rule.** Regions are entered only on the host's Doria thread. C entries
in the companion check that thread.

**One runtime per process.** The companion links the hosted shared runtime
dynamically, the same library PHP output already loads, so host and companion
share one runtime state.

**Locating and loading.**

- PHP output writes `<stem>.native.<so|dylib|dll>` and `<stem>.native.json`
  beside the generated PHP. The sidecar records its schema version, compiler
  identity, runtime ABI version, region identities and signatures, foreign
  libraries and the library's SHA-256. The generated PHP validates the sidecar
  and loads the library with `FFI::cdef(<generated declarations>, __DIR__ . ...)`,
  as it loads the runtime. Missing PHP FFI, a missing file or a mismatch fails
  explicitly with no fallback (Decision 0110's precedent).
- The interpreter builds the companion into the compiler cache at
  `doria-companion-v1/<hash>/` under the same root as the embedded runtime
  cache, and loads it with the platform loader, searching the companion's
  directory first for its dependent libraries.
- `runtime-files` (section 19) are placed beside the companion.

**Caching.** The cache key hashes compiler identity, runtime digest, the
validated MIR of every region and host operation signature, resolved link
inputs and the target triple.

**Build plans and receipts.** A `php` or `debug` build of a program with regions
requires the native toolchain and the resolved link inputs. Baton's receipt
records the companion's relative path, its SHA-256, the region inventory hash
and the resolved native link facts.

**Native shared-library output.** Stage 40 defines the native shared-library
artifact kind the companion uses: a platform shared library exporting versioned
entries, linked against the hosted shared runtime, with a metadata sidecar.
Stage 41 reuses it for bridge entries.

**Programs without regions** produce no companion; their PHP output stays
platform-neutral.

Consequences: a binding written once runs identically under every target,
including the PHP compatibility backend, and calls inside a region stay
zero-copy. PHP output of a program with unsafe code becomes host-specific and
needs PHP FFI. The interpreter's oracle role covers Doria semantics; foreign
behavior inside a region is defined by the native code it calls.

### 24. Standard-library runtime bindings

- Runtime-owned behavior keeps its compiler intrinsics and per-backend
  implementations.
- An extern declaration binding a `dr_`-prefixed symbol is a compile error.
- A standard-library module may bind a third-party C library only through its
  own decision.

### 25. Unsafe policy and audit

- Stage 40 implements the general `declare` grammar: `declare(<key>: "<value>");`
  directives come first in a file, before `namespace` and `use`; each names one
  key; a key appears at most once; an unknown key or value is a compile error.
- `declare(unsafe: "deny");` or `declare(unsafe: "allow");` sets the file
  policy; the default is `"allow"`. Under `"deny"` the file may contain no unsafe
  context, `unsafe function`, extern block, foreign aggregate or C-ABI function.
  Calling a safe wrapper defined elsewhere stays allowed.
- `unsafe = "deny"` in a package's `Baton.toml` applies `"deny"` to every source
  of that package; a file directive may not relax it. The policy travels to
  `doriac` in the build plan per package. Dependencies keep their own policy.
- `baton audit unsafe` lists every unsafe context, `unsafe function`, extern
  declaration, foreign aggregate and C-ABI function across the dependency tree
  with package, file and line. `doriac` produces that inventory as compiler
  metadata; Baton does not parse Doria.

### 26. Foreign calls in property hooks

A foreign call counts as potentially blocking under Decision 0135, directly and
through helpers, unless its declaration carries `#[NonBlocking]`. The attribute
is part of the declaration's unchecked contract.

### 27. Safety sections

- A documentation comment is a `/** ... */` block immediately before a
  declaration.
- Every `unsafe function` and extern declaration carries a Safety section, the
  `@safety` tag of its documentation comment, stating the caller's obligations.
  The compiler warns when one is missing. Inside an extern block, the block's
  comment may state obligations shared by its declarations.
- The language server shows the Safety section in hover and completion at call
  sites.
- The diagnostics, each with a catalogue entry (Decision 0145): unsafe operation
  outside an unsafe context; non-FFI-safe type in an extern signature or foreign
  aggregate, naming the type to use; `throws` on a C-ABI function; conflicting
  declarations of one symbol; a logical library with no link input; a missing
  Safety section (warning); `declare(unsafe: "deny")` and package policy
  violations; an unused unsafe context (warning); a missing or mismatched
  companion; and the runtime panics for misaligned access, address overflow,
  interior NUL, out-of-range boundary conversion, and a callback on a foreign
  thread.

## Specified Without A Card

The rulings did not name these points; this record specifies them so the stage
has one precise contract.

- A pointer into Doria storage is invalid after its unsafe context on every
  target (section 7). Ruling 23's statement that the pinning rule bounds edge
  crossing requires it.
- An extern block without `#[Library]` binds the platform C runtime (section
  15), and counts as foreign code for section 20.
- Documentation comments are `/** */` blocks and the Safety section is the
  `@safety` tag, the PHP docblock shape (section 27). Doria had no
  documentation-comment form.
- The companion's entry ABI, status codes, marshalling table, host operations,
  hosted raw-handle tokens, foreign-backed host regions, panic sink, file
  naming, cache path and key (sections 22 and 23).
- The unsafe inventory for `baton audit unsafe` is compiler metadata (section 25).
- `Platform::architecture` beside `Platform::operatingSystem` (section 17), and
  the placement of `Platform`, `OperatingSystem` and `Architecture` in
  `Doria\Std\Ffi` (section 1).
- Section 21's absent-stream rule applies on every platform.

## Unsettled

- **Browser playground.** The playground runs the interpreter in WebAssembly and
  cannot load a native companion. Which diagnostic or presentation it gives for
  FFI examples is not decided.
- **Foreign writes to standard streams.** The interpreter and PHP output keep
  their own Doria output paths, while foreign code writes to the process's
  streams. Whether the host flushes buffered Doria output at each region entry,
  so ordering matches native, is not decided.
- **`Sendable` and `Shareable` for pointers and handles** belong to Stage 39.
- **Math values by value** wait for Decisions 0141 and 0142.
- **Bit fields and packing** wait for a consumer.
- **Plan D18's "until v1.0"** wording still differs from AGENTS.md.

## Acceptance

1. **Bind and call zlib.** A Baton package declares `zlib` in
   `[native-libraries]` and binds `compressBound`, `compress2`, `uncompress` and
   streaming `deflateInit_` / `deflate` / `deflateEnd` through a foreign
   `z_stream`; `deflateInit_` receives `ZLIB_VERSION` as a `CString` and
   `ZStream::SIZE`.
2. **Zero-copy round trip.** A `Bytes` value is compressed and decompressed
   through region pointers. Natively, the region `address` equals the address C
   reports, and a Linux allocation count in `dorialang/benchmarks` shows no
   payload copy.
3. **Every target.** Fixtures 1 and 2 and every fixture below are in
   `native_parity_examples.txt` and produce identical stdout, stderr, status and
   file effects under the interpreter, Cranelift, LLVM and PHP output on Linux
   x86_64 and aarch64, macOS aarch64 and x86_64, and Windows x86_64 (MSVC). CI
   provisions zlib on every host: pkg-config on Linux, the system `libz` on
   macOS, and on Windows both an import library with its DLL and a static
   library.
4. **Platform-dependent layout.** The `z_stream` fixture passes with 32-bit
   `uLong` on Windows and 64-bit `uLong` elsewhere from one source.
5. **Callbacks.** zlib's `zalloc` / `zfree` are C-ABI Doria functions reaching a
   Doria allocation counter through `opaque` and a raw handle; every allocation
   is released once, including under the interpreter and PHP output through
   reverse edges.
6. **Foreign-owned memory.** An RAII owner exposes a foreign-backed region,
   read by safe Doria code on every target, and releases the buffer once.
7. **Region edges.** Fixtures cover `return`, `break` and `continue` out of an
   unsafe block, a checked error crossing a region edge, a Doria helper called
   from inside a region, and writable writeback on every exit status.
8. **Failure paths.** Each runtime panic in section 27, including a callback on a
   thread the program creates through FFI and a panic inside a companion
   region, exits 101 with the same report on every target.
9. **Companion loading.** PHP output fails explicitly when PHP FFI is disabled,
   the companion is missing, or its sidecar does not match.
10. **Native-window needs.** One source with `Platform::operatingSystem` branches
    builds and links on each platform while every host type-checks every branch;
    a `windows-subsystem = "gui"` binary opens with no console, survives
    `echo`, and reports `isInteractive` false.
11. **Direct compiler use.** `doriac compile` with link flags builds the zlib
    fixture on every host without Baton.
12. **Policy and audit.** `declare(unsafe: "deny")`, package `unsafe = "deny"`,
    and `baton audit unsafe` across a dependency tree with file and line.
13. **Two clocks.** An accepted-syntax regression test shows every Stage 40 form
    parses with zero parser errors before its semantics land; the language
    server's no-false-diagnostics coverage matches.

## Performance Impact

- A native foreign call is a direct call in the target C convention with no
  implicit marshalling.
- Region and pointer formation allocate nothing and copy nothing. `CString`,
  `Utf16String` and `String::fromCString` each make one visible allocation and
  copy.
- A C entry costs one thread comparison and a frame link; an outbound call costs
  two stores for the Call Path record; aligned `read` and `write` cost one mask
  test; system-code capture costs one thread-local read on opted-in
  declarations.
- Programs without foreign code keep today's Windows entry and binary size.
- Under the interpreter and PHP output, each region entry costs one native call
  plus edge copies of the buffers it pins; calls inside the region are native
  and zero-copy; each host operation inside a region costs a reverse edge.
- `dorialang/benchmarks` gains a native call-overhead case (Doria to C, C to
  Doria), the zero-copy allocation case, and a companion region-entry case.
  Timing evidence carries `Measurement Status: Pending Available Runner` until a
  controlled runner exists (Decision 0112).

## Explicit Non-Goals

- C++, Objective-C and Swift language interop; Objective-C messaging goes through
  `objc_msgSend` bindings.
- Generating declarations from C headers.
- Foreign data symbols.
- Exported symbols for C-ABI functions, and a public native library format; the
  companion's shared-library kind is internal and Stage 41 builds on it.
- A standard dynamic-loading API.
- Inline assembly, volatile access and atomics.
- Bit fields and packed layout.
- Operating-system handles of Doria files and streams.
- ABIs other than `"C"`; Windows on aarch64 and 32-bit targets.
- Callbacks on foreign threads before Stage 39; foreign exceptions or `longjmp`
  across Doria frames.
- Native package feeds, prebuilt downloads and compiling C sources in packages.

## Alternatives Considered

The options not selected on each card, in brief: core FFI names (1); statement
blocks only, or `when`-style value blocks (2); nullable `Ptr<T>`, `Ptr<void>`,
non-generic `RawPtr` (3); pointer indexing and unchecked alignment (4); fixed
widths only, per-target aliases (5); hand-computed offsets, `#[CLayout]`
classes, waiting for general inline records (6); call-only and last-use pointer
validity (7); pointer accessors on `T[]` only, regions limited to FFI-safe
elements, strided regions (8); implicit string marshalling, pointer and length
only (9); a standard `ForeignBuffer`, `Bytes` adopting foreign memory (10);
`SharedReference<T>` only, owned values only (11); closure trampolines, nominal
callback types (12); unchecked or unrestricted callback threads, root Call Paths
(13); `"system"` and named ABIs, rejected or untyped variadics (14); mandatory
camelCase names (15); capture after every call, no capture (16); Baton
per-platform source roots, a `#[Platform]` attribute (17); all link facts in
source or all in Baton (18); raw linker arguments, pkg-config only (19); a
CRT-free Windows with foreign code, the CRT for every program (20); console
only, deciding with the GUI framework (21); a native-only interpreter diagnostic
with its own manifest, an interpreter calling dynamic libraries through a
generic call bridge, Doria stand-ins (22); an unsupported PHP diagnostic, PHP
FFI lowering with per-call copies (23); migrating standard modules to public
`extern` (24); file-only or deferred policy (25); rejecting or allowing every
foreign call in hooks (26); no documentation enforcement, a required
`#[Safety]` attribute (27).

## Consequences

- Plan D3, D12, §3.5, §4.9, §8.2, §8.3, §9.1, the Stage 40 and Stage 41 entries,
  Appendix A and the decision-record list are amended (Invalidated Elsewhere).
- SPEC gains the keywords, the `Doria\Std\Ffi` vocabulary, foreign aggregates,
  platform constants, the `declare` grammar, documentation comments and the
  absent-stream rule.
- Decision 0110's regions gain pointer accessors and owner-bound `fromRaw`;
  typed array regions join them.
- Decision 0125's attribute targets grow to extern blocks and declarations.
- Decision 0121's function types gain the structural C function pointer form.
- The build plan moves to schema 2 (link inputs, Windows subsystem, package
  unsafe policy); Baton gains `[native-libraries]`, `windows-subsystem`,
  `unsafe = "deny"` and `baton audit unsafe`.
- The runtime gains a CRT-hosted Windows archive, the C entry thread check, the
  Call Path and system-code slots, the companion host interface, and the panic
  sink, each with its Windows implementation in the same change.
- The parity manifest's fixtures require zlib on every test host.
- `--target php` and `--target debug` need the native toolchain for programs
  with unsafe code.
- Raw pointers are neither `Sendable` nor `Shareable` until Stage 39 decides.

## Invalidated Elsewhere

**Amended with this record (2026-11-16):**

- `docs/doria-end-to-end-plan.md`: D3 (foreign layouts), D12 (`Ptr<Opaque>`,
  companion), §3.5 (example, body rule, policy, companion), §4.9 (safe regions),
  §8.2 (interpreter and companion), §8.3 (the ABI review: settled and remaining),
  §9.1 (`ArrayRegion<T>` slicing), the Stage 40 entry (companion and
  native-window needs in the acceptance criteria), the Stage 41 entry (reuses the
  shared-library output), Appendix A (native-library declarations moved to Stage
  40), and the decision-record list (a line for this record, and the
  unnumbered subject removed).

**Not edited here, owned by Stage 40 delivery:**

- **Compiler (`doria`).** Lexer and parser (`unsafe`, `extern`, `declare`,
  foreign aggregates, C function pointer types, documentation comments); AST,
  HIR, semantics, MIR, MIR validation; Cranelift and LLVM with one shared
  aggregate classification; C entry thunks; the companion emitter and its
  shared-library output; `codegen_native.rs` link arguments and subsystem;
  `build_plan.rs` schema 2; `main.rs` flags; the interpreter's companion host;
  PHP output's companion loader and reverse-edge dispatcher; the diagnostic
  catalogue; FFI fixtures in `native_parity_examples.txt`; `SPEC.md`;
  `docs/stdlib-reference.md` (`Doria\Std\Ffi`, typed regions,
  `String::fromCString`, `String::fromUtf16`); `docs/build-plan-schema.md`;
  compiler metadata for the unsafe inventory.
- **Runtime (`doria-rt`).** CRT-hosted Windows archive, companion host
  interface, panic sink, thread check, Call Path and system-code slots, new panic
  codes, empty-region addresses, foreign-backed region accessors.
- **Language server (`dorialang/doria-language-server`).** The `doriac` pin;
  hovers for the new keywords, types and attributes; Safety sections in hover
  and completion at call sites; no-false-diagnostics coverage; highlighting.
- **Website (`doria-website`).** A versioned unsafe and FFI guide, API reference
  for `Doria\Std\Ffi` and typed regions, diagnostic pages, and playground
  behavior for FFI examples. This record does not modify the website repository.
- **Baton (`dorialang/baton-php`, later `dorialang/baton`).**
  `docs/project-manifest.md` (`[native-libraries]`, `unsafe = "deny"`),
  `docs/targets.md` (`windows-subsystem`), `docs/build-plan.md` (schema 2),
  `docs/dependencies.md` (transitive link inputs), `docs/toolchain.md`
  (pkg-config and native toolchain needs for `php` and `debug` builds), receipts
  (companion and link facts), run-time file deployment, and `baton audit unsafe`.
- **Benchmarks (`dorialang/benchmarks`).** Call-overhead, zero-copy and
  companion region-entry cases.
- **Draft records owned by another session, not edited.** Decision 0141 places
  the `declare` grammar in Stage 47; Stage 40 now implements it (section 25).
  Decision 0142 leaves FFI vector-register passing to Stage 44; section 6's
  shared C classification fixes it for by-value math values.
