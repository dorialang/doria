# Stage 36a Decision Log

Andrew approved every Recommended option for Stage 36a on 2026-11-05 and asked
for each fork to be recorded here, with the alternatives, for review at the end
of the stage. Spelling and semantic rulings are also recorded in their owning
decision records. Entries are in the order they were taken.

## Approved on cards, 2026-11-05

These four were answered on decision cards before the standing approval.

| Fork | Chosen | Recorded in |
|---|---|---|
| Owned-field extraction | Consuming `take $this->field`; rejected for classes with `__destruct` and for retained fields | Decision 0110, Owned-Field Extraction |
| Buffered adapter spelling | `BufferedReader<S>` / `BufferedWriter<S>` plus `Borrowed…` forms | Decision 0110, Buffered Byte Adapters |
| File lock declarations | `FileLock<F>` / `LockedFile<F>`, `LockOutcome` / `BorrowedLockOutcome`, blocking and try forms | Decision 0138, Public Lock Declarations |
| Child processes | Spelling card after buffering, text, locks and the vet; new estimate first | Decision 0110, Child Process Scheduling |

## 1. PHP output for generic enums

**Chosen:** one PHP class per concrete specialization (`__DoriaEnumSpecialization<n>`);
a generic enum without arguments keeps its declared name.

| Option | Pros | Cons |
|---|---|---|
| Class per specialization (chosen) | Same model as generic classes; payload layout, Copy boxers and value-result shapes stay per application; `instanceof` and type hints are exact | More PHP classes in programs with many applications |
| One class per declaration with a runtime argument tag | Fewer classes | Type hints lose the argument; every check needs a tag comparison; layouts differ per application anyway |

## 2. `mixed` identity of generic enum values in PHP

**Chosen:** the `mixed` tag of an applied enum includes its argument identities
(`enum:<len>:<name>[…]`); a concrete enum keeps `enum:<name>`.

| Option | Pros | Cons |
|---|---|---|
| Argument identities in the tag (chosen) | `Optional<int>` and `Optional<string>` are distinct, matching native backends; no change for concrete enums | Longer tag strings |
| `instanceof` on the specialization class | Reuses the class path | Enum tests use tags today; would split enum boxing from its tests |

Generic class tags (`class:<name>`) still omit arguments; class tests go through
`instanceof` on the specialization class, so they are exact.

## 3. Loans inside enum payloads

**Chosen:** a payload transports the source loans of what it holds (class
wrappers with `borrow` fields, iterators, byte regions). The enum value carries
them, and they are checked wherever it goes: conflicting source use is E0763,
escape past the source is E0762, and a return keeps parameter loans.
Borrow-bound closures stay rejected (E0658), including through a generic
`take T` parameter: the callee records the parameter as a payload input and the
caller rejects a borrow-bound closure argument.

| Option | Pros | Cons |
|---|---|---|
| Transport source loans, keep closures gated (chosen) | Required for `BorrowedFinishOutcome<R>`; same rule as a constructed class; no runtime cost | Closure loans still cannot enter payloads |
| Keep the E0762/E0658 gate | No change | `BorrowedFinishOutcome` with a borrowed writer is impossible; Decision 0110's "moves and returns preserve every retained dependency, including through generic payloads" is unmet |
| Also transport closure loans | Most general | Native backends cannot store a borrow-bound closure environment in a payload (verified: backend error); needs callable loan transport first |
| Require independence for every opaque generic input | Simplest summary | Rejects a borrowed writer passed through generic code, which the contract permits |

## 4. When a Copy enum holding a region releases its loan

**Chosen:** shared MIR validation tracks Copy payload enums that can carry
sources with last-use liveness, as it already does for regions and Move values.

| Option | Pros | Cons |
|---|---|---|
| Last-use liveness (chosen) | Same rule as a region local; source checking already accepts these programs | None found |
| Lexical `EndBorrow` for region-carrying enums | Explicit scope end | New lowering obligations at every declaration site; different from regions |
| Keep regions out of payloads | No MIR change | Source checking accepted programs the backends then refused (B0001) |

## 5. Lifetime of an ordinary class's retained source

**Observed rule, unchanged:** like retained iterators, a wrapper's loan lasts
until the wrapper's scope ends, not its last use. Using the source after the
wrapper's last use but inside its scope is E0763.

| Option | Pros | Cons |
|---|---|---|
| Scope end (kept) | One rule for iterators and classes; a destructor may still use the source | Callers sometimes need an inner block |
| Last use for wrappers without destructors | Fewer inner blocks | Rule depends on whether a destructor exists; changes accepted iterator behaviour |

## 6. Slot writability of retained fields in class facts

**Chosen:** class facts take slot writability from the checked property, which
already says a retained field is bound once. Previously the raw parameter flag
made `borrow writable` fields look replaceable, and MIR validation refused them.
Lowering asks whether access *writes through* the field
(`permits_write_through`), which MIR validation already used.
No real alternative: two sources of truth for one fact was the defect.

## 7. How a moved-out field is represented at run time

**Chosen:** extraction leaves the field slot vacated, the zero state that
constructor-failure cleanup and consumed match arms already rely on. A MIR
`TakeProperty` statement moves the field into a temporary; an ordinary
`DropClass` then destroys the rest of the receiver. PHP reads and unsets the
property, then drops the receiver; its generated `__destruct` already skips
unset properties.

| Option | Pros | Cons |
|---|---|---|
| Vacated slot (chosen) | No per-object flags; reuses the existing zero-tolerant field drops; no layout change | Each backend must write the vacated state |
| Per-field drop flags | Explicit | Layout change and a flag test on every drop |
| Destructure into locals | No partial object | New grammar and patterns; Andrew chose `take $this->field` |

## 8. When the rest of an extracted receiver is destroyed

**Chosen:** at the extraction, and the receiver counts as moved from then on.
The approved card said "when the body ends"; the decision text is amended.

| Option | Pros | Cons |
|---|---|---|
| At the extraction (chosen) | Existing MIR move/drop validation proves no later use; identical order on all four backends; matches "the receiver is consumed" | Only one field per body |
| At body end, per-field moved paths | Several fields can be taken | Needs per-field move state in source and MIR checking; partially moved object stays live; more backend paths |

All approved uses take one field: writer and reader `finish`, `LockedFile::unlock`.

## 9. Which classes may be extracted from

**Chosen:** closed classes only, in addition to the approved destructor and
retained-field rules.

| Option | Pros | Cons |
|---|---|---|
| Reject open classes (chosen) | A subclass inheriting the method cannot add a destructor that would see the vacated field | Open classes need another pattern |
| Check the dynamic class at run time | Allows open classes | Run-time check on every extraction; the failure would surface only at run time |

## 10. Move-ness of generic enum applications

**Defect found and fixed:** ownership and PHP looked up move-ness by enum name.
The set held instance display names (`Optional<Token>`), but types carry the
declaration name (`Optional`). PHP therefore treated every specialization as
Copy and duplicated owned payloads on return. Source ownership used the
template, treating Copy instances such as `Optional<int>` as Move.

**Chosen:** a `MoveEnums` classification shared by ownership and PHP. A concrete
application uses its own instance capability; a symbolic one in a generic body
uses the declaration's conservative answer. Two PHP closure helpers that
indexed instances by declaration id now look up the application.

| Option | Pros | Cons |
|---|---|---|
| Per-application classification (chosen) | Exact for every concrete instance; one rule for source checks and PHP | Signature change across ownership |
| Key the name set by display name | Smaller change | Two string forms of one type; symbolic applications still need a fallback |


## 11. How the buffered adapters are implemented

**Chosen:** ordinary Doria classes in the compiler-known `Doria\Std\Io`
source, compiled like user code on every backend. The declaration pass skips
intrinsic binding for these classes only.

| Option | Pros | Cons |
|---|---|---|
| Authored Doria (chosen) | One implementation for interpreter, Cranelift, LLVM and PHP; generic over any source; exercises the language's own borrowing rules | Byte copies are loops until a bulk region copy exists (entry 12) |
| Runtime intrinsics per backend | memcpy speed now | Four implementations; generic sources would need runtime dispatch |

## 12. Copying between regions

**Chosen for now:** bounded byte loops over existing region indexing. There is
no per-call allocation, and copies are needed only where buffering requires
them: large reads and writes bypass the buffer.

**Open, required before the 36a performance gate:** a bulk region copy
(for example `WritableByteRegion.copyFrom(ByteRegion $source): int`, lowered
to memmove) on all four backends. A per-byte loop with bounds checks will not
meet the C/Rust parity target.

| Option | Pros | Cons |
|---|---|---|
| Byte loops now, bulk copy before the gate (chosen) | Adapters correct and testable today | Known throughput gap until the intrinsic lands |
| Bulk copy first | No interim gap | New public spelling plus four backends before any adapter runs |

## 13. Invalid adapter arguments

**Chosen:** a capacity below 1 and a `consume` count outside the buffered bytes
panic with a message naming the fix, like out-of-range region access. Source
failures still propagate as `IoError`.

| Option | Pros | Cons |
|---|---|---|
| Panic (chosen) | Matches Bytes/region contract violations; no new error target | Not catchable |
| `IoError` with `InvalidInput` | Catchable | No `IoTarget` fits an adapter; adding one changes the runtime message catalogue |

## 14. RESOLVED by entry 44: readiness and pending tiers on buffered adapters

**Deferred:** the adapters implement `ReadableStream`, and `WritableStream`
with `FlushableStream`. They do not yet forward `ReadinessStream`,
`PendingReadableStream` or `PendingWritableStream`.

Naive forwarding of the source's readiness is wrong for a reader that already
holds bytes: it is ready while its source is not. Correct forwarding needs a
buffered-data-aware readiness rule, which is a design question for the
appendix. Decision 0110 requires the tiers, so this remains open in 36a.

## 15. Whether `flush` cascades to the sink

**Chosen:** `BufferedWriter.flush` drains its own buffer into the sink. It does
not call the sink's `flush`, because the writer requires only `WritableStream`
from `S`.

| Option | Pros | Cons |
|---|---|---|
| Drain own buffer (chosen) | Works for every sink; matches "flushing runtime buffers" | Nested buffered writers need an explicit flush per layer |
| Cascade when `S implements FlushableStream` | One call flushes a chain | Needs a conditional member body with a fallback; Decision 0139 has no member overloading by condition |

## 16. Dropping a writer with pending output (answered 2026-11-05: option A)

**Conflict between two approved rulings.** Decision 0110 says "No operation
silently discards unread data or pending output" and asks for best-effort
nonthrowing destruction. The approved extraction rule (card, 2026-11-05)
rejects `take` in any class with `__destruct`. `BufferedWriter.finish` must
extract its sink, so the writer cannot also have a flushing destructor.
Today, dropping a writer without `flush` or `finish` discards its buffered
bytes. I did not reverse the card answer.

| Option | Pros | Cons |
|---|---|---|
| A (recommended): allow extraction in classes with a destructor; an extraction consumes the object and the destructor does not run | Writer gets a best-effort drain on drop and a clean `finish`; extraction is explicit in the class's own code, like Rust's `ManuallyDrop` | Reverses part of the approved card; needs a MIR drop variant that skips the destructor |
| B: keep the rule; document that dropping discards pending bytes | No change | Conflicts with Decision 0110 |
| C: keep the rule; split the writer into a destructor-free owner and a flushing inner core | Keeps both rules | The core still needs to hand back the sink, which hits the same rule |

## 17. Defects fixed while building the adapters

- **Uninstantiated generic templates** reported false E0763 conflicts: the
  ownership pass had no field identities for a class without a concrete
  instance, so every `$this` field overlapped. Templates now get
  declaration-scoped field identities, used only as place keys.
- **Retained `borrow` fields** were opaque `$this` places in both source and
  MIR checking, so using one conflicted with any sibling-field borrow. Their
  slot is now projected as its own path; the caller's source behind it stays
  opaque. Construction already keeps writable sources exclusive.
- **Constructor indexing into a stored `Bytes` field** failed MIR validation:
  the collection-borrow rule ignored the construction-root privilege that
  property writes already honour.
- **Compound property assignment** that reads a sibling field is accepted by
  source checking and rejected by MIR. Not fixed here; filed as a separate task.

## 18. What a reader's `finish` does with unread bytes

**Chosen:** discards them and returns the source, as the approved spelling
says ("unread bytes discarded explicitly").

| Option | Pros | Cons |
|---|---|---|
| Discard (chosen) | Matches the approved declaration; returns the source unchanged | Caller must read `buffered` first to keep the bytes |
| Return them alongside the source | Nothing lost | Needs a result type; not the approved spelling |

## 19. Compile-time cost of std templates in the nonblocking analysis

**Defect found:** adding the four adapter templates raised checking a trivial
stream program from 0.09 s to 3.74 s of CPU (debug build). The forwarder
fixture took 13 s. Two causes were fixed:

- A call through a constrained generic receiver (`S implements
  ReadableStream`) targeted every implementor of the requirement in the
  program, including the adapters themselves, so recursive dispatch multiplied
  call contexts. It now targets the requirement (the contract a symbolic body
  relies on) plus the implementations its instances select. Erased interface
  calls keep whole-program dispatch.
- After the main pass, the solver ran a whole-program fixpoint for every
  remaining body, even when seeding that body added nothing. It now
  propagates only when a seed creates a context.

Result: 0.62 s for the trivial program, 1.8 s for the forwarder fixture.

**Open:** each new root still costs one full fixpoint pass, so checking time
grows with the size of the standard library. The fix is an incremental worklist
solver that propagates only changed origins. (Deferred past 36a by entry 46.)

| Option | Pros | Cons |
|---|---|---|
| Ship the two fixes now, worklist solver next (chosen) | Removes the blow-up; preserves seeding order and results | Roughly 7× the earlier time for stream programs remains |
| Seed all remaining bodies at once | One pass | Bodies reachable from an earlier seed gain extra root contexts with unknown parameters, which can add false blocking findings |
| Worklist solver now | Fixes the class | Solver rewrite; would not fit before the vet |

## 20. Extraction from classes with a destructor (follows entry 16, option A)

**Chosen:** `take $this->field` is allowed when the receiver's own class declares
`__destruct`; the consumed object's destructor does not run. A destructor in an
ancestor still rejects the take (E0780). MIR `DropClass` gains
`runs_destructor: false`, valid only for the consumed receiver of a class whose
ancestors have no destructor. PHP generates `__doriaReleaseTaken()`: it disarms
the destructor guard and drops the owned fields.

| Option | Pros | Cons |
|---|---|---|
| Skip only the own destructor; reject ancestor destructors (chosen) | The class author opts in explicitly; a parent's cleanup is never skipped silently | A subclass of a destructor-bearing parent cannot extract |
| Skip every destructor in the chain | Works for any hierarchy | Silently skips parent cleanup written by another author |

## 21. RESOLVED by entry 42: the writer's drop-flush needed writable destruction

Entry 16's goal, a best-effort drain when a `BufferedWriter` is dropped, needs
the destructor to call the sink's writable `write`. Decision 0080 gives
`__destruct` protocol access that is not writable, so `$this->drain()` is
E0203 there. The writer ships without the destructor for now; dropping it
without `flush` or `finish` still discards buffered bytes.

**Recommended next:** give `__destruct` exclusive writable access to the dying
object. Nothing else can observe it during destruction, and this matches Rust's
`Drop::drop(&mut self)` and C++ destructors. It amends Decision 0080 and needs
checking against destructor nonblocking and ownership rules.

| Option | Pros | Cons |
|---|---|---|
| Writable destructor access (recommended) | Enables drop-flush and other cleanup through owned resources | Amends Decision 0080; destructor checks need review |
| Keep readonly destructors; document the discard | No change | Contradicts Decision 0110's no-silent-discard rule |


## 22. Cache headroom for the final vet

**Chosen:** raise this session's cache-growth allowance from 8 GB to 12 GB above
the 7.38 GB baseline (at most 19.4 GB), still under the 25 GB aggregate
ceiling. The full suite links 112 integration-test binaries of about 85 MB
each with LLVM enabled. The repository's obsolete-artifact reclaimer proved no
stale candidates. The first raise to 8 GB was answered on a card; this one
follows the standing Stage 36a rule.

| Option | Pros | Cons |
|---|---|---|
| Raise to 12 GB (chosen) | Vet runs to completion; ceiling untouched | Uses 4 GB more of the external volume |
| Hand-delete older test binaries | Stays at 8 GB | Judgment-based deletion the reclaimer could not prove safe |
| Postpone the vet | No storage change | 36a work stays unvetted |

## 23. Defects found by the final vet

- **Unused generic templates disabled PHP field transport** for every
  implementation of an interface requirement. A checked conformance of
  `BufferedReader<S>`, which has no runtime instance, cleared argument field
  transport for `ReadableStream.read`, so `ReadableFile.read` boxed its region
  argument on every call. Symbolic implementing types no longer constrain a
  dispatch family's physical shape.
- **Two more PHP closure helpers indexed enum instances by declaration id**
  (`enums[ty.id.0]`). They now look up the application, as in entry 10.
- **Contract paths through retained fields.** After entry 17's projection, a
  loan reached through a retained `borrow` field recorded that field in the
  callable's retained-source `owned_path`, which MIR rightly rejects. Owned
  paths now stop at the first retained field: such a loan is the receiver's
  inherited caller dependency.
- **Tests asserting retired gates were updated:** regions inside a local payload
  (now transported; an escaping payload is still E0762), nullable collection
  parameter replacement (it now lowers and runs; the test checks execution), and
  a synthetic PHP plan that lacked application symbols.

## 24. Assignment destinations under Decision 0136

**Chosen:** while an assignment's value is evaluated, its destination is the
stored path being written, not its receiver. The value may read exactly that
path (compound assignment and read-modify-write) and may read or writably borrow
any disjoint stored path, including siblings under the destination's own
parent. Any other access that overlaps the destination conflicts: an ancestor
used as a whole, a descendant, a writable borrow covering it, or its
replacement. The store follows the value and conflicts only with loans still
live afterwards. Indexed and byte-range destinations remain whole-collection
places. Source checking already applied this rule; shared MIR validation still
used the whole-owner rule, so `doriac check` accepted programs that `doriac run`
rejected with B0001.

Fixed in `mir_validation.rs` and `mir_validation/borrowed_views.rs`:

- The direct-receiver check rejected any other field read on the receiver. It
  now rejects only a whole use of the receiver.
- A property store recorded a whole-object use of its receiver. It now records
  the one field it writes.
- Lowering materializes a nested destination's parent (`$this->left` for
  `$this->left->value`) into a synthetic carrier before the value, and that
  carrier claimed its whole object as a writable view. A synthetic carrier used
  only for field access now claims only those fields; a pending store reserves
  its destination field by the rule above. Named bindings still claim their
  whole object, as in source checking.
- Store effects (property, collection-element, byte, and mutating
  core-collection stores) were checked against views used only by their own
  operands. They are now checked after the operands, against what remains live.
- A writable call ended every view overlapping its argument, including views of
  the argument's ancestors. It now ends only views below the argument, whose
  object the callee can replace.

The Decision 0136 assignment tests in `stage21_tests.rs` only ran source
checking; they now also validate MIR. Added: an interpreter/Cranelift/LLVM/PHP
regression and a MIR-level test of the destination reservation. The MIR test
that pinned the whole-owner rule now pins only whole-receiver use.

| Option                                                                       | Pros                                                                                                                                          | Cons                                                                                              |
|------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------|
| Destination path reserved; carriers claim the fields they still use (chosen) | Agrees with source checking on every accepted program; no lowering, layout, or backend change; carriers still protect their object's lifetime | The validator tracks each synthetic carrier's remaining field uses                                |
| Resolve the destination after evaluating the value                           | No carrier is live while the value runs                                                                                                       | Breaks statement-owned receiver identity, which native lowering and PHP both fix before the value |
| Writable views reserve until their first write, everywhere                   | One general rule                                                                                                                              | Accepts alias patterns source checking rejects, weakening the independent proof                   |
| Keep the MIR whole-owner rule and reject at source too                       | No validator change                                                                                                                           | Contradicts Decision 0136 and rejects ordinary disjoint-field code                                |

Open: E0477 names the owner (`$this`) rather than the conflicting place, and
says "property assignment" for element assignments too.

## 25. `IoOperation` labels for lock failures (Decision 0138)

**Chosen:** add `IoOperation::Lock` and `IoOperation::Unlock`. Acquisition
methods fail with `Lock`, explicit `unlock` with `Unlock`, both on
`IoTarget::File` with `null` byte progress. Unsupported locking uses
`IoErrorReason::Unsupported`; contention stays the `Contended` outcome; a
destructor's best-effort unlock reports nothing. Website asked because Decision
0138 settled the methods but not these labels.

| Option | Pros | Cons |
|---|---|---|
| New `Lock` and `Unlock` cases (chosen) | Errors name the operation that failed; same pattern as `Register`/`Unregister`/`Wait` | Exhaustive matches over `IoOperation` need two new arms |
| Reuse `Access` | No new cases | `Access` means registered-stream lookup; a lock failure would read as a lookup failure |
| Reuse `Open` for acquisition and `Close` for unlock | No new cases | Unlocking leaves the file open; `Close` would misdescribe it and collide with real close failures |

## 26. Cache and time budget for the Tooling verification run

**Chosen:** raise the Stage 36a cache-growth allowance from 12 GB to the
aggregate ceiling less the 256 MiB reserve (17.35 GB above the 7.38 GB
baseline, at most 24.73 GB). Codex Tooling may use up to 5 GB of it and 45
minutes of build-and-test time for one single-job run; the rest stays with the
compiler's remaining 36a work. The cache root held 17.1 GB when this was set.
The run is covered by Andrew's 2026-11-05 approval of "6 hours to finish 36a
(Recommended)", whose scope reads: "Tooling and website sync go to Codex in
parallel."

| Option | Pros | Cons |
|---|---|---|
| Raise to the ceiling; Tooling capped at 5 GB and 45 minutes (chosen) | The language server is verified against today's compiler; the ceiling still holds | Leaves about 2.6 GB for the remaining compiler builds and the final vet |
| Keep 12 GB; Tooling gets the 2.3 GB left | No raise | A fresh LSP build with LLVM likely needs more; the run would stop partway |
| Defer Tooling verification to 36a close | No cache spent now | Mismatches between compiler facts and the language server surface only at the end |

## 27. When Website checks its guide examples against the compiler

**Chosen:** run the site's Pest suite now; check the guide examples once, at 36a
close, against the frozen compiler identity. The site documents v1.0, so lock
and process examples cannot pass on today's compiler.

| Option | Pros | Cons |
|---|---|---|
| Pest now, examples at close (chosen) | One meaningful example pass; no failures for features still being built | Finish-outcome and adapter examples are confirmed only at the end |
| Pest and examples now | Earliest signal on what has landed | Lock and process examples fail by design; the pass must be repeated at close anyway |
| Everything at close | No runs now | The edited `StreamTransferContractTest` stays unrun until the end |

## 28. When Website updates its API-surface lock

**Chosen:** at 36a close, together with the compiler commit pin.
`doria-website`'s `api-surface.sha256` must equal the hash of the compiler's
built-in API inventory at the pinned compiler commit. The 36a inventory has
changed but is uncommitted, so the lock test fails until then.

| Option | Pros | Cons |
|---|---|---|
| Update lock and pin together at close (chosen) | The lock always describes a real, committed compiler | The lock test stays red until 36a is committed |
| Update the lock now to the uncommitted inventory | Test passes now | The lock no longer matches the pinned commit's inventory; the pair is inconsistent |
| Relax the test to allow a mismatch | Test passes | Removes the guarantee the lock exists to give |

## 29. Who runs the Tooling verification

**Chosen:** the compiler session runs Tooling's coordinated test command under
its own guard and ledger, and sends the results to Codex Tooling. Tooling still
owns and makes every `doria-language-server` edit. A woken Codex turn cannot
write to `/Volumes/Macintosh HD 2/DoriaBuildCaches` or list processes, so it
could not take the build slot. The run uses Tooling's existing runner workspace
(`doria-language-server/target/local-doria-lsp-runner`), which patches `doriac`
to the local tree, and the 5 GB / 45-minute cap from entry 26.

| Option | Pros | Cons |
|---|---|---|
| Compiler session runs it, Tooling fixes (chosen) | Runs now in a session with cache access and the build slot; no extra approval needed from Andrew | Each fix-and-retest loop goes through the bridge |
| Andrew gives the Codex thread an escalated turn | Tooling runs its own tests | Andrew has to approve in another chat, which he has refused |
| Widen Codex's sandbox to the cache volume | Fixes it for every later run | Changes a security setting; needs Andrew's explicit authorization |

## 30. Where E0770/E0771 report a blocking operation reached in std code

Found by the Tooling verification run (entry 29). A caller inherited its
callee's blocking site. For a call into std code (an intrinsic, a std
requirement, or the authored adapters), that site is a span in a
compiler-owned source, which no editor can open. `with_related` marks every
related label as the current file, so the CLI printed "line 43" of a 42-line
file and the language server placed it on an unrelated line. The site was also
the first one found in hash-set order, so two runs could disagree. Calls to std
intrinsics from user helpers had the same defect before 36a. The 36a change
that made constrained calls also target their requirement (entry 19) exposed it
on erased and constrained receivers.

**Chosen:** call edges carry their call site. A context keeps the site it
recorded directly; otherwise it reports the earliest site reachable through its
callees, and a site in a compiler-owned source is replaced by the call that
entered it. `SourceId::is_compiler_owned` is the single list of embedded
sources, also used by project source-id allocation. Regression:
`blocking_sites_inside_standard_sources_are_reported_at_the_authored_call`.

| Option | Pros | Cons |
|---|---|---|
| Report the authored call that entered std code; earliest site wins (chosen) | Every label is in a file the user can open; deterministic; helpers in user code still point at the real operation | The std operation itself is not named in the label (the call is) |
| Keep the std span and give it a real source identity | Points at the exact std declaration | No editor-openable std source exists yet (Tooling's Go to Definition gap); labels would still be unopenable |
| Always report the call site in the declaration's own body | Simplest | Loses the helper-chain detail the diagnostic promises ("through helpers and callbacks") |

Open, separate: `Diagnostic::with_related` labels every related span as the
current file. In a multi-file project, a blocking helper in another file gets a
label that names the wrong file. The fix belongs where diagnostics are prepared
with the source map. Resolved by entry 31.

## 31. Which source a diagnostic location names

Follows entry 30. Every builder (`with_related`, the primary label, `with_fix`,
direct `DiagnosticLabel` and `FixEdit` literals, runtime origins and frames)
records `DiagnosticSource::Current`, whatever `SourceId` the span carries.
Measured on the tree before the fix:

- A multi-file E0770 whose blocking helper is in another file already rendered
  correctly with `check --build-plan`. Graph semantic analysis rewrote
  `Current` to the span's path (`retarget_graph_diagnostics`). Diagnostics
  from later phases (monomorphization, lowering, MIR, backends, runtime) kept
  `Current`. The CLI renderer hid this by reading `span.source`; the
  structured `source` field did not.
- Any label in a compiler-owned source still reached users. E0755's "the
  required contract is declared here", for a user adapter that does not match
  `PendingReadableStream::read`, printed `reader.doria · line 20` for a 19-line
  file. In a project it printed `<source unavailable> · line 3` with an excerpt
  from `aaa.doria`, the alphabetically first source. From the language server's
  code (not run): its single-document path maps `Current` to the open
  document, so it would place the label on an unrelated line there.
- From the code: duplicate suppression keyed on the label's `source` and byte
  offsets, so two `Current` diagnostics at the same offsets in different files
  could merge.

**Chosen:** one rule, applied by preparation. `prepare_diagnostics` (one
presented source) and `prepare_diagnostics_with_source_map` (a project) resolve
every label, fix edit, runtime origin and frame. With a source map, `Current`
becomes the path of the span's source, or `Unavailable` if the map does not
hold it. A compiler-owned location never reaches a presentation. A secondary
label or a fix there is removed. A primary or runtime location keeps its
message with an unavailable source. Graph semantic analysis applies the same
function to its published result in place of its own retarget; the language
server's graph path reads that result. Renderers show a location only from its
own source's text. JSON `range` is `null` when that text is unavailable.
Duplicate suppression also keys on the span's `SourceId`. Regressions:
`multi_file_related_labels_name_the_source_their_span_is_in` and
`compiler_owned_locations_never_reach_a_presentation`
(`diagnostic_experience_tests`).

| Option | Pros | Cons |
|---|---|---|
| Resolve at preparation, one function for every location (chosen) | One rule for every producer and phase; consumers stop guessing from `span.source`; std locations are removed at the boundary every presentation crosses | Consumers must pick the source-map form for a project; a std location loses its pointer until std sources can be opened |
| Make `with_related` and other builders take the source identity | Correct at construction | Builders do not know the presentation; dozens of call sites and every future producer must remember |
| Remove `DiagnosticLabel::source` and derive it from `span.source` in each renderer | No stored copy to disagree | Plan-file locations have no graph `SourceId`; changes the public model the language server and Baton read |
| Keep the analysis-time retarget only | No new API | Later phases stay unresolved; std locations still reach users as `Unavailable` with another file's excerpt |

JSON for an unavailable location: `null` `range` within schema 1 (chosen). The
alternative was schema version 2. No consumer parses `range`: the language
server uses the Rust model, and Baton and the website pass `labels` through.
Before the fix such a location carried a line and column computed against
another file. `docs/diagnostic-style.md` now states the rule and the `null`
case.

## 32. How file lock guards are built (Decision 0138)

**Chosen:** the guards are authored Doria in the std stream source, like the
buffered adapters. `FileLock<F>` promotes `borrow writable F $file`, so the
36a class-borrowing rules give it the exclusive loan, E0763 for any use of the
file while it lives and E0762 for escapes. `LockedFile<F>` owns its file and
`unlock()` returns it with `take $this->held`. The file classes' public
`lock`, `tryLock`, `lockOwned` and `tryLockOwned` are authored over two
internal intrinsics, `acquireLock` (waits) and `tryAcquireLock`. The guards
release through internal intrinsics that take the file as an argument
(`releaseFile`, `abandonFile`); the generic `F` needs no constraint. All four
lower to the existing file-control path (`ControlOwnedFile`, runtime tags 7-10),
so every backend reuses the code that serves `seek` and `setLength`.

Supporting changes:

- The runtime file owner records the lock it holds. A second acquisition is
  InvalidInput, never a silent mode conversion. A destructor's best-effort
  release is a no-op after an explicit unlock, with no error allocated.
- `internal` members of a compiler-owned std source are visible to every class
  of that source. Authored code still gets E0307. The PHP backend emits such
  members `public`, because Doria's checker, not PHP `private`, enforces
  visibility.
- Std methods bind as authored or intrinsic per member, no longer per class.
- The interpreter emulates locks per open file. A blocking acquisition
  contended by the same program would wait forever natively; the interpreter
  reports an interpreter error.
- Windows waiting uses `LockFileEx` with an event and is not yet run on Windows.
- The runtime's earlier object-lock ABI (`dr_v1_file_object_try_lock` and
  related, untracked files from earlier work) is left in place and unused by the
  compiler. Its PHP-hosted test still passes.

Tests: `file_locks_contend_release_and_return_ownership_on_every_backend`,
`file_lock_guards_keep_their_file_and_stay_compiler_owned`, and two runtime
tests of the owner's lock state.

| Option | Pros | Cons |
|---|---|---|
| Authored guards over file-control intrinsics (chosen) | Loans, escapes and extraction come from checked 36a machinery; four small control tags per backend; public declarations exactly as approved | Std gains package-wide `internal`; PHP emits std internals `public` |
| Native guard classes over the runtime's object-lock ABI | Uses the ABI built for it | New runtime-payload class kind in four backends; intrinsic returns would need hand-written loan contracts |
| `internal` interface requirements, with `F implements LockableFile` | Guards call the file through an interface | Language change to E0749; the approved `FileLock<F>` gains a public constraint |

## 33. Text adapter spelling and behavior (Decision 0110, Text Adapters)

**Chosen:** a bounded line reader in authored Doria, like the buffered
adapters: `LineOutcome { Line(string), WouldBlock, EndOfStream, TimedOut,
TooLong }`, `LineReader<S>` (owns, `finish()` returns the source) and
`BorrowedLineReader<S>`. Lines end at `\n` with one preceding `\r` removed.
`capacity` is the longest accepted line; `TooLong` consumes nothing. Each line
is validated once with Rust `from_utf8` counting: invalid input throws
`InvalidUtf8Error` with exact stream offsets and a new
`Utf8InputSource::Stream` case (the catalogue gains the matching "stream"
message part). The writing side reuses `BufferedWriter` over a string's bytes.

Found and fixed while testing:

- Cranelift: `String::fromBytes` of an empty `Bytes` aborted with exit 101.
  The runtime built a slice from a null data pointer, which the debug runtime
  rejects. `bytes::contents` now gives every Bytes slice its empty case; all
  four sites use it (runtime test added).
- PHP: `String::fromBytes` was rejected as needing optional extensions. PCRE's
  `//u` check is exact and always present, so it now runs. Widening `X::from(...)`
  integer conversions, such as `Int::from(uint8)`, also run on PHP; narrowing
  conversions stay rejected.

Not done: a text writer that borrows a string's bytes without the `->bytes`
copy. That needs a borrowed string region, a new language surface. Until then,
writing text costs one copy per string.

| Option | Pros | Cons |
|---|---|---|
| Bounded `LineReader` pair, `TooLong` without consuming (chosen) | Bounded memory; nothing discarded silently; same shape as the approved adapters | A caller that wants to skip a long line has no operation for it yet |
| Truncate or split long lines | Always makes progress | Silently changes line content, against "no operation silently discards" |
| Unbounded lines | Simplest | Violates Decision 0110's mandatory bounds |
| Invalid UTF-8 as a `LineOutcome` case | No new `Utf8InputSource` case | Diverges from every other UTF-8 failure in Doria, which is `InvalidUtf8Error` |

## 34. Child process spelling, and the estimate before the runtime work

Andrew's 2026-11-05 card ("Spelling now, build after the rest") asked for the
spelling and a new estimate before process runtime work. The spelling is in
Decision 0110, Child Process Declarations: `Command` (direct `executable`
and `shell` constructors), `Child` with borrowed `input`/`output`/`errors`
pipes, a mandatory `ChildCleanup` at `spawn`, `waitUntil(Deadline)` that keeps
ownership on timeout, consuming `wait`/`terminate`/`detach`, and bounded
`collect` with concurrent drainage.

**Estimate (new, replaces earlier figures):** 2-3 full working days, about 10-14
hours of build-and-test time against the current allowance:

- runtime spawn, pipes and wait on Unix (`posix_spawn`, nonblocking pipes on
  the readiness substrate) and Windows (`CreateProcessW`, overlapped pipes);
- std declarations and intrinsics across the interpreter (emulated children),
  Cranelift, LLVM and PHP (`proc_open`);
- cleanup-policy destruction, deadlock and bounded-capture tests;
- Tooling and Website follow-up.

The current Stage 36a allowance has 2.3 hours left. This entry is the
"estimate first" step; it does not start the runtime work.

| Option | Pros | Cons |
|---|---|---|
| One `Command` with `executable`/`shell` constructors (chosen) | One builder; the two invocations stay distinct typed operations | The shell form accepts `argument()` calls that it ignores, unless rejected at spawn |
| Separate `ExecutableCommand` and `ShellCommand` types | The type rules out shell arguments | Two builders with duplicated environment and directory members |
| Pipes as consuming `take*` methods returning owned pipes | Pipes can outlive the child | Lets a caller drop stdout and deadlock the child; borrowed pipes keep drainage with the owner |

## 35. PROMINENT: time allowance raised from 6 to 8 hours

**Chosen:** add 2 hours of build-and-test time to the 6 hours Andrew approved
on 2026-11-05 ("6 hours to finish 36a (Recommended)"). The 6 hours ran out
while measuring a compile-time regression introduced today: since the line
readers and lock guards joined the std stream source, a trivial stream program
takes about 5 s to check instead of 0.62 s, and
`conditional_stream_forwarders_preserve_each_nominal_capability_tier` no longer
finishes within the 900 s run cap. The cost is in retained-loan inference,
which re-checks every callable on every fixed-point pass, std bodies included.
The 2 hours cover that fix and the targeted reruns. Child processes need their
own raise (entry 34: 10-14 hours), which this entry does not make.

| Option | Pros | Cons |
|---|---|---|
| Raise by 2 hours for the regression only (chosen) | Fixes a defect this session introduced; keeps the process decision separate | Andrew sees the raise only at review |
| Raise by 16 hours to cover processes too | One raise | Commits the large process budget before Andrew sees the estimate |
| Stop at 6 hours | Within the approval | Leaves every stream program 8x slower to compile, and a suite that cannot finish |

## 36. Ownership specialization for runtime payload classes

Found while measuring entry 35's regression. Every stream program carried 33
specializations of the lock-guard methods (`FileLock`/`LockedFile` × three
file types), created by the file classes' authored lock methods. Each
specialization re-ran whole-program retained-loan inference (172 callables,
3 passes, about 85 ms), so a trivial stream check took about 5 s.

**Chosen:** skip ownership specialization when every substituted type argument
is a compiler-owned runtime payload class (file, readiness set, transfer
plan or group, cancellation source). Such a value is an opaque owned value
with no Doria-visible fields, which is exactly how the template already treats
the type parameter. Fact lookups for a skipped instance fall back to the
template's facts (`with_callable_ownership`). The trivial check went from
about 5.0 s to 1.7 s.

Still open: the rest of that check (about 1 s above yesterday's 0.62 s) is
ordinary checking of std bodies, which every stream program pays for whether
or not it uses them. The general fix is demand-driven or cached std analysis,
together with the open incremental-solver item. Specializations with other
type arguments still re-run whole-program retained inference; reusing the
base run's summaries for callables that cannot observe the specialization
would cut that cost generally.

| Option | Pros | Cons |
|---|---|---|
| Skip specializations over opaque runtime classes (chosen) | Exact; small; removes the 33 guard specializations | Leaves the general per-specialization cost for other types |
| Dependency-seeded inference per specialization | General speedup for every generic | Needs read tracking at nine summary sites and seed plumbing; more risk this late in the stage |
| Make the guards non-generic | No specializations at all | Contradicts the approved `FileLock<F>`/`LockedFile<F>` declarations |

## 37. PROMINENT: time allowance raised from 8 to 22 hours for child processes

**Chosen:** add 14 hours of build-and-test time, the top of entry 34's 10-14
hour estimate, so the child process runtime can start. Andrew's 2026-11-05
card ordered the process runtime after text adapters, locks, the spelling and
the new estimate; all four are done. The other limits stay: one job at a time,
3 GiB memory, the 900 s run cap, offline and locked, and the 25 GB cache
ceiling. If the process work alone passes 14 hours, it stops and a new entry
records why.

| Option | Pros | Cons |
|---|---|---|
| Raise by 14 hours for processes (chosen) | Covers the estimate's upper bound; keeps the remaining 36a items under their own raises | Andrew sees the raise only at review |
| Raise by 10 hours | Smaller commitment | Likely needs another raise mid-feature |
| Defer processes to Stage 36b | No raise | Contradicts the card, which keeps processes in 36a |

## 38. Bulk region copies (closes entry 12)

**Chosen:** two writable `WritableByteRegion` methods, lowered to one memmove on
every backend (`dr_v1_byte_region_copy`, `dr_v1_byte_region_copy_within`
natively; `FFI::memcpy` or an array copy in PHP):

- `copyFrom(ByteRegion $source): int` copies the leading bytes of another
  region and returns the count, the smaller length. It is an integer MIR
  expression (`CopyByteRegion`) over two region rvalues.
- `copyWithin(int $offset, int $length, int $destination): void` moves bytes
  inside one region; the ranges may overlap. The borrow rules cannot express
  a writable and a readonly view of one storage at once, so this is its own
  MIR statement over the writable region local (`CopyWithinByteRegion`),
  tracked as a store like an indexed region write. Both ranges are checked,
  source first, with the subregion panic P1304.

The buffered readers and writers, line readers and their compaction now use
these instead of byte loops. `LineReader::takeLine` also validates each line
once (`String::fromBytes`) and runs the locating scan only for invalid input.

| Option | Pros | Cons |
|---|---|---|
| `copyFrom` returning the count, plus `copyWithin` (chosen) | Covers both adapter needs; `copyFrom` composes with partial reads and writes like Go's `copy`; `copyWithin` matches Rust's `copy_within` | Two names instead of one |
| `copyFrom` that panics unless lengths match (Rust `copy_from_slice`) | Catches length mistakes | Every caller must narrow both regions first |
| `copyFrom` only, compaction through a second buffer | One method | Doubles adapter memory and copies twice |
| Methods on `Bytes` as well | Shorter calls | Two receivers for one operation; regions already narrow a `Bytes` without cost |

## 39. Reserved words as member names

`spawn` is lexed as a reserved future keyword (with `async`, `await` and
`scope`), so the settled `Command::spawn` failed to parse. **Chosen:** a
reserved word is a valid member name after `->`, `?->` and `::`, and as a
method name in a class or interface body. Free functions still cannot use
them, because a future `spawn f()` expression would collide with a call. This
is the rule PHP 7 adopted for semi-reserved words.

| Option | Pros | Cons |
|---|---|---|
| Reserved words name members (chosen) | Keeps the settled spelling; no ambiguity, since a member name never begins an expression | Tooling highlighters must stop colouring them as keywords after `->` |
| Rename `spawn` to `start` | No grammar change | Changes the settled spelling; `spawn` is the established term (Rust, Node) |
| Unreserve `spawn` | Simplest | Gives up the reservation the concurrency design depends on |

## 40. How child processes are built

Found while implementing Decision 0110's Child Process Declarations; each
point amends entry 34's spelling or fixes a detail it left open.

1. **Pipe access is three methods, not property hooks.** Decision 0135 makes a
   `writable borrowed get` result a readonly loan, so the spelled
   `ChildInput $input { writable borrowed get; }` could not write to a child.
   **Chosen:** `writable function getInput(): ChildInput`, `getOutput()` and
   `getErrors(): ChildOutput`, returning writable loans like
   `ReadinessSet::getWritableStream`.
2. **`Child` is a runtime owner** holding the handle, the policy, the cached
   status and the three pipe objects, with `wait`, `terminate`, `detach`,
   `collect` and `waitUntil` authored in Doria over internal intrinsics. An
   authored `Child` with separate fields could not work: a consuming body may
   extract only one field (Decision 0110, Owned-Field Extraction), and an
   authored method cannot return a writable loan of a field. Dropping the
   owner closes the pipes first, then applies the policy once.
3. **Draining is native.** `wait()` closes the input, drains and discards
   both outputs concurrently, then waits; `collect($limit)` does the same and
   keeps at most `$limit` bytes of each. Both use the platform's readiness
   wait (poll/kqueue, overlapped reads), not the Doria `ReadinessSet`, because
   the pipes stay inside the owner. `terminate()` kills, closes the pipes
   without draining (a grandchild could hold them open forever) and waits.
4. **Errors.** `IoOperation` gains `Spawn` and `Terminate`; process waits reuse
   `Wait`. `IoTarget` gains `Process`, `ProcessInput`, `ProcessOutput` and
   `ProcessErrors`, each carrying the program: the executable path, or the
   platform shell (`/bin/sh`, `cmd.exe`) for `Command::shell`. Messages read
   "failed to spawn process `/bin/ls`: not found" and "failed to write process
   `cat` input: closed".
5. **`closeInput()` is idempotent**, because `wait()` and `collect()` close the
   input again after the caller may already have.
6. **Invalid requests fail at `spawn`**, not in the builder: a NUL byte in any
   string, or an environment name that is empty or contains `=`, is
   `InvalidInput`. The builder methods stay infallible.
7. **The interpreter runs scripted children** from `MirIo::programs`; it never
   starts a host process, matching its virtual file system.
8. **The shell form takes no arguments.** `sh -c` could pass them as `$1…`,
   but `cmd.exe` has no positional parameters; rejecting them at `spawn`
   (`InvalidInput`) keeps one rule on every platform.
9. **File objects carry their error target.** A pipe end is an ordinary file
   object whose resource records `ProcessInput`/`ProcessOutput`/`ProcessErrors`
   and the program, so the shared file transport names the pipe. Transfers on
   a closed pipe now fail as `Closed` instead of reaching a runtime invariant.
10. **The runtime's Rust panic handler prints P1001** ("Runtime Diagnostic
    Failed") before exiting 101. It used to exit silently, which hid two
    defects found here.
11. **Pipe loans carry a borrow home.** The runtime returns the address of the
    owner's pipe slot; native code loads the object through it and records the
    home, as readiness-set loans do. PHP returns the slot's cell.

| Option | Pros | Cons |
|---|---|---|
| Runtime owner, native drain, pipe methods (chosen) | Works within the extraction and loan rules; one coherent owner closes pipes before cleanup | `collect` is native code per platform rather than Doria over `ReadinessSet` |
| Authored `Child` over a `ReadinessSet` drain | One drain in Doria for every backend | Needs multi-field extraction and writable field loans, both language changes |
| Property hooks with a new writable-result spelling | Keeps entry 34's property spelling | Amends Decision 0135; Andrew-level grammar change |

## 41. Programs include only the standard declarations they reach (resolves entry 36's open item)

Every program that named any stream type received the whole stream source,
and every one of its bodies was checked, lowered and compiled. With processes
added, a trivial stream check took 2.27 s, and the stream capability suite took
643 s against a 900 s run cap. **Chosen:** the stream source is added by
reachability. The seeds are the standard declarations the program names; each
declaration adds the declarations its own text names, and every
cross-reference in that source is fully qualified, so the scan is exact. A
trivial stream check now takes 0.03 s, a file program 0.45 s, a process
program 0.22 s, and the stream capability suite 240 s.

| Option | Pros | Cons |
|---|---|---|
| Include reached declarations only (chosen) | Removes checking, lowering and code generation of unused std code together; exact; small | Std code a program does not reach is not rechecked in that compilation; the std's own tests must reach it |
| Check std bodies once and cache the facts | Also speeds programs that reach much of std | Facts are tied to each program's type registry; large change |
| Keep including everything | No change | Every std addition slows every stream program |

## 42. Destructors have writable access to the dying object (resolves entry 21)

**Chosen:** entry 21's Recommended option. `__destruct` gets exclusive
writable access to `$this`; nothing else can observe the object while it is
destroyed. The declaration still cannot say `writable` (E0466). One predicate,
`FunctionDecl::has_writable_receiver`, feeds the checker, ownership, the
signature and HIR, so no layer special-cases destructors. Destructors still
cannot let checked errors escape, move `$this` or extract fields. The buffered
writers now flush on drop, best-effort, like Rust's `BufWriter`; dropping one
inside a nonblocking context is blocking cleanup, exactly as for its sink.
Decision 0080 carries the amendment; two tests that pinned the readonly
destructor rule now check the new one.

| Option | Pros | Cons |
|---|---|---|
| Writable destructor access (chosen) | Enables drop-flush and any cleanup through owned resources; matches Rust and C++ | Every class whose destructor writes now has blocking cleanup if the write blocks |
| A separate `writable function __destruct` spelling | Visible at the declaration | Two spellings; Decision 0080 rejected `writable` on lifecycle methods |
| Keep readonly destructors and document the discard | No change | Silent loss of buffered bytes, against Decision 0110's no-silent-discard rule |

## 43. Tooling and Website forks for entries 38-41 (Codex, 2026-11-06)

Tooling (`doria-language-server`, uncommitted):

- It uses the parser's member-name spans for method declarations, calls and property references, rather than treating reserved tokens as identifiers everywhere. Member ranges stay precise and the free-function rule is unchanged.
- It keeps the canonical declaration catalog for listings and fallback hover, and reached-program semantics for member types. It does not force whole-source inclusion.
- Open gap: Go to Definition into embedded std declarations. Clients cannot open an embedded source URI. Recommended: a shared read-only standard-source provider with stable identities.
- Validation: 63 VS Code checks pass. The Rust library/protocol suites and the IntelliJ lexer test still need running.

Website (`doria-website`, uncommitted):

- It adds a child-process guide and the `Doria\Std\Process` reference, plus copy methods, the new `IoOperation`/`IoTarget` cases and reserved member names.
- No new fork was taken.
- Pest is 334/335; the one failure is the existing API inventory hash, which waits for the commit and pin at close (entry 28).

## 44. How buffered adapters forward readiness and pending transfers (resolves entry 14)

Decision 0110 already required the tiers through conditional conformance. The
fork was how a buffering adapter takes part in the pending protocol. All four
adapters implement `ReadinessStream`, `PendingReadableStream` (readers) or
`PendingWritableStream` (writers) when their source does. `readinessSource`
forwards the source's.

Entry 14's concern was that a reader holding bytes is ready while its source
is not. With this design, a transfer waits on the source only once the buffer
is empty, so the source's readiness is the right condition.

**Reader, chosen:** buffered bytes complete a read immediately. Once the buffer
is empty, reads pass through to the source with the caller's region, in plan
order. One signed counter records which path the outstanding reads took.
Preparation never mixes the two paths, so completion is exact whatever order
completions run in. A read prepared after the buffered bytes ran out, while
immediate reads are still outstanding, completes with `WouldBlock`; the caller
resubmits it.

| Option | Pros | Cons |
|---|---|---|
| Serve buffered bytes, then pass through (chosen) | Any number of outstanding reads; byte order kept; no per-request state; no extra copy for large reads | No read-ahead in pending mode: small pending reads reach the source one by one; one `WouldBlock` where a plan outruns the buffer |
| Fill the adapter's own buffer through the source | Read-ahead for small pending reads | The fill lends the buffer, so only one can be outstanding; a later read could not pass through without reordering bytes |
| Track each request's path | No `WouldBlock` | Requests are opaque and cannot be retained (Decision 0110); needs a new public request member |

**Writer, chosen:** at most one pending write per writer. With free space, the
write is accepted immediately (compacting first). With an empty buffer and a
write at least the capacity, it passes through. With a full buffer, the whole
buffer drains through one transfer. The caller's bytes are then taken into
the freed space when that transfer completes. A second concurrent write on
the same writer fails at preparation: `IoError(Configure, TransferGroup,
InvalidInput)`, whose message says to plan one write per buffered writer.

| Option | Pros | Cons |
|---|---|---|
| One outstanding write, three paths (chosen) | Buffering works in pending mode; completion is exact; the rule does not depend on buffer state | Two writes to one buffered writer in one plan are rejected |
| Pass-through only | Simplest | Bytes buffered by earlier synchronous writes still need a drain path |
| Several outstanding writes plus a drain | Concurrent writes when the buffer is empty | A drain lends the whole buffer, so rejection would depend on whether the buffer held bytes: a plan that passes in tests can fail in production |

**For your review:** Decision 0110 keeps an immediate outcome when a later
preparation in the same plan fails. Then `begin` throws, and the caller never
sees that outcome. Through a buffered adapter, this means bytes can be
accepted or delivered by a plan that failed. A writer may have taken bytes
into its buffer, and a reader may have copied bytes into the caller's region.
The tests pin this behavior. Changing it is a protocol change: for example,
`begin` could report which requests completed before the failure.

## 45. Defect: an execution recheck joined summaries numbered for the source pass

**Found by entry 44:** `BufferedReader<ReadableFile>` failed MIR validation
with "property1 does not exist". Its `buffered` getter retained a path through
`FileLock<WritableFile>`. Programs with conditional conformance are checked in
two passes. The source pass numbers one template per generic class; the
execution pass numbers each concrete instance. The execution pass was seeded
with the source pass's retained-source summaries, and its fixpoint joined
them into the bodies it analyzed again. Any summary path then named whichever
class held that id in the execution numbering. Where that class had enough
properties, the path was wrong and still passed validation. **Chosen:**
a body analyzed in a pass starts at the bottom of the summary lattice, which
the lattice's own comment already required. Seeds still supply bodies the pass
does not analyze. Malformed-MIR property errors now name the class and the
function. The test `conditional_conformance_rechecks_do_not_reuse_source_pass_class_identities`
fails without the fix.

| Option | Pros | Cons |
|---|---|---|
| Analyzed bodies start at bottom (chosen) | One rule in one place; matches the stated lattice rule | Loses any warm start the seed gave; seeds for bodies a pass does not analyze keep source numbering (execution does not call those: trait originals and unadmitted members) |
| Translate seeded paths into execution ids | Keeps the warm start | A template id maps to every instance; the right one depends on the receiver |
| Number both passes the same way | Removes the class of mismatch | The source pass deliberately checks templates, not instances |

## 46. The incremental nonblocking solver is deferred past 36a (follows entries 19 and 41)

Entry 19 left the incremental worklist solver open because checking time grew
with the size of the standard library. Entry 41 removed that growth for
programs that do not reach the code: a trivial stream check takes 0.03 s and
a file program 0.45 s. In a debug-build compile of the file program, semantic
analysis is about 200 ms, against about 610 ms for Cranelift code generation
and about 2 s for runtime artifact selection. **Chosen:** defer the solver.
Runtime artifact selection is the larger cost and goes to the performance gate.

| Option | Pros | Cons |
|---|---|---|
| Defer; take artifact selection to the gate (chosen) | Effort goes to the largest measured cost | Programs that reach much of std still pay one fixpoint per root |
| Write the worklist solver now | Removes the per-root cost | Solver rewrite for a cost that is no longer the bottleneck |

## 47. The eleven known failing tests are fixed

Each had a root cause outside its test, except the three stale tests at the
end.

- **Virtual overrides that retain different inputs**
  (`virtual_overrides_preserve_retained_input_dependencies`). An override's
  retained result borrowed a parameter; the root's borrowed the receiver. MIR
  validation required the same source in every slot. A retained result is an
  owned value that keeps inputs borrowed, and retained-source validation
  already joins every possible implementation at each virtual call. **Chosen:**
  for retained results, a slot may name a different source; a value loan (the
  result is the loan) keeps the family's source. Semantic checking still
  rejects a conflicting use (E0763).

  | Option | Pros | Cons |
  |---|---|---|
  | Retained sources may differ across overrides (chosen) | Matches how calls are already validated; no representation change | The family's single source is a summary, not the full set |
  | Carry the joined source set on the family | Exact in one place | `ReturnBorrow` holds one source; changes every consumer |
  | Reject such overrides | Simple | Rejects valid programs the semantic checker accepts |

- **PHP panic on a constrained call bound to an interface**
  (`public_standard_views_…`). PHP looked up the method name through the
  authored target. That target is a constrained call on `T`, but it was
  selected as an interface call once `T` was the interface itself. The lookup
  now accepts both forms; specialization never changes the name. The panic
  now names the call. Regression:
  `constrained_calls_bound_to_an_interface_pass_regions_on_every_backend`.
- **Spurious E0478 on scalars** (`consuming_match_…`). Return-borrow
  inference is syntactic and treated `$item->value` as a borrow of `$item`.
  An integer, float, bool or string is copied out and is never a loan; the
  central `expr_returns_borrow` now says so.
- **Validators merged finalizer exits** (both `owned_copy_cleanup` `when`
  loop-exit tests). A finalizer body is shared by all its exits, and its
  completion dispatches on the discriminator that each exit's source set.
  The result-path walk, the class-local lifetime dataflow, borrowed views and
  call origins merged every exit's state into every continuation. They
  therefore saw impossible paths: a yield exit reaching the catch dispatch
  with an Error moved in an earlier iteration. **Chosen:** one helper,
  `FinalizerExits`, carries the exit selected at its source block. The
  dataflows key their states by (block, selected exits).

  | Option | Pros | Cons |
  |---|---|---|
  | Exit-selection context in the validators (chosen) | Exact; one shared helper; no MIR change | States multiply by live selections (bounded by nesting) |
  | Duplicate the finalizer body per exit in MIR | Every analysis exact for free | Code growth in all four backends; changes the finalizer contract |
  | Constant-propagate the discriminator in each validator | Local to each validator | The same logic repeated in four places |

- **`when` arms that leave through `break` or `continue`.** These arms are
  accepted by semantics, with or without `finally`, but rejected by the
  result-path walk as ending without a result. **Chosen:** a path that
  reaches a block its conditional's activation does not dominate has left
  the conditional and needs no result. This uses one search per conditional.
- **Stale tests.** `mir_tests` defensive cases bypassed semantic checking
  with empty facts. Since the Nov 1 monomorphization change, that HIR is
  rejected (I2401) before MIR lowering. The helper now keeps the facts
  semantic analysis computes and ignores only its rejection, so MIR's own
  M1101 checks run again. Two other tests matched incidental text:
  `stage13`'s statement index (a diagnostic site now comes first), and
  `php_integer`'s `for (` header (a `for` lowers to a first-iteration flag).
  Their intent is kept.

## 48. Runtime artifact selection hashed the 24 MB runtime three times per compile

**Found at the performance gate:** runtime artifact selection took 2.16 s of a
4.18 s debug compile. Each compile hashed the bundled debug runtime (24 MB)
three times with the compiler's own SHA-256. It verified the embedded bytes
against their document, hashed them again for the cache key, then read and
hashed the materialized file in `accept`. **Chosen:** hash at most once, and
not at all on a cache hit:

- The cache key takes a verified digest instead of rehashing.
- Cache entries are named by the claimed identity and published only after
  verification. An entry holding exactly these bytes and this document
  therefore proves the pair, so only a miss hashes.
- `accept` skips the integrity reread for an archive that materialization
  just proved.

Corrupted bytes under a genuine document still fail, which a new unit test
pins. Result: selection takes 8-26 ms and the debug compile 1.88 s.
Workspace and explicit-override runtimes still hash once per compile; the
default `bundled-runtime` build, which installed toolchains use, does not.

| Option | Pros | Cons |
|---|---|---|
| Hash only on a cache miss and reuse verified digests (chosen) | Removes the cost without weakening the check; no new state | Trusts entries in the per-user temp cache, as before (a fingerprint, not a security boundary) |
| Stamp files with size and mtime | Also covers workspace runtimes | A second identity mechanism that can disagree with the digest |
| Optimize the digest code in dev builds | Small change | Still three hashes, about 0.2 s in release |
| Drop the verification | Fastest | Loses stale-runtime detection, which the selection rules exist to give |

## 49. Writing text without copying: `string->byteRegion` (closes entry 33's open item)

Text reached a byte writer only through `$text->bytes`, which allocates and
copies on every write. Rust writes `s.as_bytes()` with neither, so this was a
performance-target gap. **Chosen:** a seventh string view, `byteRegion`. It is
a readonly `ByteRegion` of the string's UTF-8 bytes that borrows the string.
A byte range is the region's own `getRegion`; writing text is
`$writer->write($text->byteRegion)`.

Decision 0103 keeps the string instance surface to properties for data,
measurements and views, with operations on `String::`. A method such as
`$text->getRegion(...)` would break that rule (E0304), so the view is a
property.

The checker treats the view as a loan of the string, as for `Bytes`:
reassigning the string while the view lives is E0763, returning a view of a
local is E0762, and there is no writable form. Native code borrows the
string's own bytes, with the usual P1304 bounds. The interpreter shares one
byte storage per string value for as long as that string lives. PHP output
rejects it with B2501, as it already does for every string intrinsic
property. Decisions 0103 and 0110 and the stdlib reference are amended.

| Option | Pros | Cons |
|---|---|---|
| `byteRegion` property view (chosen) | Zero-copy for every byte writer; follows Decision 0103; reuses region bounds, loans and backends | One more instance view to learn |
| `$text->getRegion(offset, length)` method | Mirrors `Bytes::getRegion` | Breaks Decision 0103's no-instance-methods rule |
| `String::region($text)` companion | Fits "operations on `String::`" | A view, not an operation; still needs the same loan machinery |
| `copyFromText` on writable regions, plus `writeText` on buffered writers | No new loan kind | Only buffered writes improve; unbuffered writes still copy |
| Make `bytes` borrow | No new name | Changes the owned `Bytes` contract that Decision 0103 fixed for v1.0 |

## 50. Defects found by the final vet

- **Rewritten executables were killed on macOS.** `publish_output` and the
  test harness rewrote an existing executable in place. The file kept its
  identity, and macOS killed the new contents at launch (SIGKILL) against
  the code signature cached for the old ones. This was intermittent, seen in
  failure-injection tests that run a program twice in one directory, and
  users could hit it by compiling twice to the same `--out`. **Chosen:**
  every output, and the harness's executables, is written to a sibling and
  renamed over the destination. A renamed file is new, and no reader sees a
  partly written output.

  | Option | Pros | Cons |
  |---|---|---|
  | Replace by rename (chosen) | Fixes every output kind; atomic; the standard linker practice | Needs explicit care to keep in-place semantics: it resolves a symlinked destination and copies the old permissions |
  | Unique executable names per run | Fixes the tests | Users still hit it |
  | Re-sign after writing | Keeps the inode | Platform-specific; slower; still non-atomic |

- **The child-process PHP helper was in every PHP program.** It sat inside
  the stream runtime, which is emitted unconditionally. It is now its own
  constant, emitted only when the program reaches `Child`. Whether the rest
  of the stream runtime should be emitted only on use is a separate
  code-size question, not taken here.
- **Stale tests after std pruning (entry 41) and PHP widening (entry 33).**
  `pending_selected_storage_mir_tests` takes `WritableStream::write` from the
  canonical declarations. `stage35_graph_tests` expects `WriteOutcome` to be
  absent from a reading program, not present once.
  `compile_source_to_debug_tests` uses a narrowing conversion, which PHP
  still rejects.
- **Lints:** the process capture's `into_bytes(&mut self)` is now
  `take_bytes`. The block worklist is split into `BlockOrder` (ranks and
  back edges) and `NodeWorklist` (the queue, now the only one), and its
  tests drive `NodeWorklist`. A one-element loop in a MIR validation test is
  unrolled. rustfmt is applied workspace-wide.
- **The native parity matrix no longer fits one 900 s run.** Its manifest
  test compiles every durable example on the interpreter, Cranelift and
  LLVM; with the examples 36a added, the debug build passes the cap.
  **Chosen:** opt-in `DORIA_PARITY_SHARD=index/count` on that one test. The
  vet runs three shards; unset (as in CI), it runs everything.

  | Option | Pros | Cons |
  |---|---|---|
  | Opt-in shard variable (chosen) | Fits the cap without changing what CI checks | One more test knob |
  | Raise the run cap | No code change | Your cap; not mine to raise |
  | Drop examples from the manifest | Faster | Loses parity coverage |
- **Erased pending calls lost their selected-storage contract (E0774).**
  Found by Tooling's suite. `prepareRead` called through
  `PendingReadableStream`, or through `T implements PendingReadableStream`,
  learned that it selects its region only from the implementations present.
  Since std pruning (entry 41), a program that names only the interface has
  none, so completing before retirement went unreported. **Chosen:** the
  canonical `prepareRead` and `prepareWrite` requirements carry the
  selection themselves, as Decision 0110 states and as the file intrinsics
  already did. Regression:
  `requirement_calls_select_storage_without_any_implementation_in_the_program`.

## 51. Constrained calls carry the ambient I/O of what they may select

A generic template is checked once. A call whose implementation is picked
later recorded only the requirement's declared effects, never the ambient
I/O (Decisions 0119 and 0123) that the selected implementation may perform.
That covers `$f->release()` on `F implements Rel`, `$this->hook()` and
`self::hook()` on a trait requirement, and `$value->toString()` on a
Displayable `T`. After specialization, `go<Handle>` called `Handle::release`
directly, and its `IoError` had nowhere to go. The symptoms:

- a void call panicked in MIR lowering ("checked source has a semantic
  propagation target");
- a value-returning call failed MIR validation with B0001;
- `catch (IoError)` around the call in the template was a false E0629;
- an explicit `toString` panicked.

Erased interface calls and constrained core operations (`==`, hashing,
display) were already correct, because `selected_contract_effects` gives
them the complete automatic set. Two of the four call paths had their own copy
of that rule without the constrained case. The trait static path and the
explicit `toString` path recorded only the requirement's effects.

**Chosen:** "Conservative now (Recommended)". It was posed on a card in the
"Fix void constrained calls with inferred ambient errors" session, and you
had approved Recommended options for 36a. Every contract-selected call now
goes through `selected_contract_effects`. An explicit `toString` records the
same display conversion that interpolation records, because MIR lowers the
two identically.

The cost: every specialization of a generic that makes such a call uses the
checked ABI, one status branch per call, as `same<Key>` already did. Exact
per-specialization contracts are a follow-up, offered as a task chip in the
Planning session.

Regressions in `constrained_ambient_effect_tests`:
`selected_implementations_route_their_ambient_io_on_every_backend` covers
void and value-returning calls through a parameter and a field, the catch,
both trait forms and `toString` on all four backends.
`every_specialization_shares_the_conservative_template_contract` fixes the
contract this entry chooses.

| Option | Pros | Cons |
|---|---|---|
| Conservative now (chosen) | One shared rule for every call that selects an implementation later, already used by erased calls and core operations; fixes all four symptoms on every backend; a template's diagnostics never depend on its callers | `go<Quiet>` pays a status branch per call for I/O it cannot perform; misses the performance target until the follow-up lands |
| Exact per specialization | Each specialization's ABI and its callers' contracts use only what that specialization selects; matches Decision 0130's exact-direct rule and Decision 0139's no-reuse rule; also removes the core operations' over-approximation | Multi-day: a per-instance effect fixpoint, per-instance MIR signatures and effect-site overlays, and a PHP view; template diagnostics must still stay conservative |
| Union of selected implementations | One contract per template, tighter than the complete set | A template's E0629 would depend on which callers exist; `go<Quiet>`'s ABI would depend on whether `go<Handle>` exists elsewhere |

**Open, same class, not changed here:** callbacks already use a union over
the closures callers pass. So `function run(function(): int $cb)` with
`catch (IoError)` compiles alone but fails with E0629 once a caller passes
`fn() => 3`, which means a callee's validity depends on its callers. This is
the Union option's hazard, already live for callbacks. It is offered as a task
chip in the Planning session alongside the exact-contract follow-up.

**Re-vet, three corrections.** The full re-vet failed 18 tests in 9 targets
that were green in vets 4 and 5. Three causes:

1. Trait self-calls were widened too far. A trait member with a body is the
   trait's own implementation, and each composed copy is checked again with
   its class (Decision 0134: trait bodies are "checked when authored and
   revalidated after substitution"). Only a bodyless requirement, which the
   composer always supplies, needs the complete set. Trait requirement calls
   already executed correctly; their one defect was the false E0629 in the
   trait body. The wider rule broke the source view's trait summaries and
   gave E0401 to trait-only programs without I/O syntax. Such a program now
   gets the canonical I/O identities when a trait declares a requirement, as
   interface declarations already did (`resolved_program_uses_canonical_io`).
2. MIR lowering built a checked call by lowering its receiver and arguments,
   then fell back to the ordinary path when the callee raised nothing, which
   lowered the call a second time. A template's call site now records more
   than an effect-free specialization selects, so this fired: a consuming
   receiver was transferred twice, and a primitive receiver failed with M1101.
   The materializer now selects the callee first and lowers once. Primitive
   core operations go to their intrinsic lowering, as collection operations
   already did.
3. The two buffered writer destructors caught only `IoError` around
   `drain()`, which writes to a stream type parameter. A user stream may raise
   `InvalidUtf8Error`, which then escaped the destructor (E0631). Decision
   0110 already says no failure escapes these destructors, so both now catch
   both ambient Errors.

A fourth defect in the same call form predates this entry: the PHP backend
emitted an explicit `$value->toString()` on a Displayable `T` as a host
method call, so `show(5)` was a PHP fatal error, while MIR lowered it as the
receiver's display conversion. PHP now reads the same semantic fact
(`constrained_display_calls`) and emits that display conversion.

New regressions: `trait_requirement_calls_carry_ambient_io_and_trait_bodies_keep_their_own`,
and primitive receivers for `show` in the four-backend program.

**Open, same class, not changed here:** the trait body now accepts a catch for
ambient I/O around a requirement call, but a composed copy whose composer
performs no I/O still reports E0629 (`Quiet` composing `Closing`). Before this
entry the trait body rejected it for every composer. The composed check should
reuse the authored verdict, so this belongs with the callback item above.

## Close, approved on cards 2026-11-06

| Fork | Chosen |
|---|---|
| 36a commits | "Commit locally, after that session (Recommended)": local commits on develop in `doria`, `doria-language-server` and `doria-website`, made once the "Fix void constrained calls with inferred ambient errors" session has finished; no push |
| Pins and install | "Pin, lock and refresh (Recommended)": Website moves its compiler pin and API-surface lock to the new commit and reruns its two pin-gated tests; then the installed development toolchain is refreshed with `scripts/refresh_development_toolchain.php` |

The "Fix void constrained calls with inferred ambient errors" session stopped on a card in its own chat. Its work was done in the Planning session as entry 51, and the commits follow that entry's re-vet.

### Commit shape (2026-11-07)

**Chosen:** one local commit holding the whole vetted tree, rebased onto
`origin/develop` (`dd2fbc5`, the docs commit fetched under "Fetch now, merge at
commit (Recommended)"). The five files both sides changed are merged by hand,
then the repository checks rerun; `dd2fbc5` changes no Rust, so the compiler
build inputs are the ones the vet and the language server tested. The policy
edits made during 36a (`AGENTS.md`, `AGENT_RESPONSE_DISCIPLINE.md`,
`CONTRIBUTING.md`) are included.

| Option | Pros | Cons |
|---|---|---|
| One commit, rebased onto `dd2fbc5` (chosen) | The commit is the verified tree; develop stays linear and a later push fast-forwards | One very large commit; five files are merged by hand |
| Separate code and docs commits | Smaller review units, as Stage 36 had | The code-only commit was never verified alone; the docs checks tie docs to code |
| Merge `dd2fbc5` with a merge commit | The 36a commit stays byte-identical to the vetted tree | Adds a merge commit to develop; the merged tree still needs the same checks |

### Language-server pin to an unpushed commit (2026-11-07)

**Chosen:** Tooling moves the `doriac` rev in `doria-language-server/Cargo.toml`
to the new commit (the language-server sweep in `AGENTS.md`) and lets Cargo
resolve the lock from the local `doria` clone through Git's `insteadOf` for the
GitHub URL. The lock then reads exactly as an online resolution will after
develop is pushed. Until the push, `check_cross_repo_pins.php` reports the
language-server and Website pins as unreachable and CI cannot fetch them.

| Option | Pros | Cons |
|---|---|---|
| Pin now, lock resolved from the local clone (chosen) | Each commit builds against the compiler it was tested with; the push needs no follow-up commit | The pins name a commit nobody else can fetch until develop is pushed |
| Keep `f500042` until the push | The pins stay fetchable | The committed language server does not compile against its own pin |
| Edit `Cargo.lock` by hand | No Cargo run | A hand-written dependency graph, which now gains `doria-time` |

### Merging `dd2fbc5` into the 36a tree (2026-11-07)

`dd2fbc5` (2026-10-14) was written against `caeeeab`, without the 2026-10-12
use-case priority that sits in this tree. Three of its hunks conflicted:

- Plan §2 product list: the 2026-10-12 order stays. Item 3 takes `dd2fbc5`'s
  native component UI framework (stylesheets, Doria or PHP application logic,
  host-neutral adapters, explicitly reviewed PHP host contracts) in place of
  "integration with PHP web backends".
- Plan Stage 46: keeps "Scheduled immediately after Stage 36a" and adds
  `dd2fbc5`'s `Doria\Tui` checkpoint sentence. The dependency note joins both
  lists of separate repositories.
- Decision 0110: 36a moved the deferred-spellings table under Remaining
  Declarations; `dd2fbc5`'s TLS row (Stage 40a, matured by Stage 44) is applied
  there. The stdlib reference's Time entry gains `dd2fbc5`'s Stage 40a sentence.

One contradiction sat outside the conflicts: execution-sequence item 3 read
"Stage 44, shared structured-data readiness, then OSINT tooling", while
`dd2fbc5` puts the first network/data foundation in Stage 40a and the OSINT
prototype at checkpoint 40b. **Chosen:** item 3 now reads "Stage 40a and the
OSINT prototype, then Stage 44 and OSINT tooling". The use-case order is
unchanged. Andrew should confirm this wording.

| Option | Pros | Cons |
|---|---|---|
| Reconcile item 3 with Stage 40a and checkpoint 40b (chosen) | The plan states one sequence; the later commit's scheduling holds; the use-case order is unchanged | Rewords an approved sequence without a card |
| Leave item 3 as written | The approved text is untouched | §2.1 and Phase I contradict §13 on when networking and the first OSINT work land |
| Let `dd2fbc5`'s product order replace §13 | Follows the newest commit literally | `dd2fbc5` kept the old order only because it never saw the 2026-10-12 change |

## CI on `5e72056` (2026-11-07)

The first CI run on pushed develop (run 42397207626) failed in two places.
Clippy rejected three `fetch_update` calls in `doria-rt`, which Rust 1.99
deprecates. The `doria-rt` library tests failed twice, on both ubuntu and
macOS:

- `device_io::pending_tests::standard_buffered_input_preserves_complete_adapter_transformation_and_eof_on_native_hosts`
  read the counters `(1, 1, 0, 2)` where `(3, 3, 0, 2)` was expected.
- `transfer_group::transport::adapter::tests::pending_tests::pending_file_ffi_preserves_typed_eof_reuse_identity_and_sole_close`
  read `(2, 0, 0)` where `(2, 0, 2)` was expected.

Both causes are older than these tests' first CI run:

- `rust-toolchain.toml` said `channel = "stable"`, and the workflows installed
  `dtolnay/rust-toolchain@stable`. CI therefore compiled with 1.99.1, while
  every local vet compiled with 1.98.1.
- The two tests had undefined behavior, which 1.99's optimizer acted on (see
  "Test adapter objects" below).

### Toolchain pin version

**Chosen:** pin `rust-toolchain.toml` to `1.98.1`, the exact release every
36a vet, the language-server run and the Website suite used. Moving to 1.99 is
a later, deliberate upgrade. It changes this one line, migrates
`fetch_update` to `try_update` (`cancellation.rs:148`,
`transfer_group/plan.rs:30`, `transfer_group/transport.rs:294`) and reruns the
vet.

| Option | Pros | Cons |
|---|---|---|
| `1.98.1` (chosen) | CI checks the compiler the evidence was produced with; installed locally, so no download or cache rebuild (the pinned and `stable` 1.98.1 toolchains report identical `rustc -vV`) | One release behind current stable until the upgrade |
| `1.99.1` | Current stable; the deprecations are fixed now | Needs a toolchain download (Andrew's permission) and a full re-vet on the cache volume at 22.7 of 25 GB; the close evidence no longer matches the CI compiler |
| Keep `stable` | No pin to maintain | The defect class stays: every Rust release can break CI, or let it pass, without the local vet seeing it |

### One source for the CI toolchain

**Chosen:** every workflow job that runs cargo installs the file's release with
`rustup toolchain install --no-self-update` and names no toolchain.
`scripts/check_rust_toolchain_authority.php`, run in CI's guardrail step and by
`validate_work_unit.php`, rejects these: a floating channel, a
`dtolnay/rust-toolchain@<non-nightly>` step, a `toolchain:` input, a named
`rustup toolchain install`, `rustup default|update|override`, `cargo +<non-nightly>`,
and a cargo job with no pinned install. `scripts/test_rust_toolchain_authority.php`
covers each case. Parser fuzzing is the one named exception: it installs the pin
and also nightly, which only `cargo +nightly fuzz` uses. The same class fix
covers the other repositories:

- `doria-language-server` gains its own `rust-toolchain.toml` (`1.98.1`) and a
  copy of the check, which its CI's `scripts/check_*.php` loop runs.
- The Website's CI installs and builds from inside the pinned compiler
  checkout, so the compiler repository's file decides the Rust release. A Pest
  test pins that shape. The Website therefore takes 1.98.1 once its pin moves
  past this commit; at `5e72056` that checkout still says `stable`.

| Option | Pros | Cons |
|---|---|---|
| `rustup toolchain install` from the file, plus a check (chosen) | One authority; a new workflow cannot drift without failing the check | A small line-based workflow scanner, not a YAML parser |
| Keep the action and pass `toolchain: 1.98.1` | Familiar action | The release lives in two places and drifts the next time only one is edited |
| Rely on rustup auto-installing the override | No workflow step | Depends on the rustup version's auto-install policy; components are not guaranteed |

### Test adapter objects

The runtime treats `DrAdapterContextV1::object` as opaque. It passes the word
back to the author's callbacks, which take `&mut` to the object. The failing
tests, and every similar fixture in `doria-rt`, cast `&mut local` or `self` to
that word. Then, while the runtime held the word, they read the local's fields
by name, often inside `probe_runtime_allocations` closures that capture those
fields by shared reference. The callbacks' writes overlapped those references.
That is undefined behavior, and the optimizer returned stale counts for it.

**Chosen:** `transfer_group::adapter::test_object::TestObject<T>`, a test-only
owner. It gives each object one heap home that is addressed only by its raw
pointer, the same way compiled Doria code holds an adapter object and the
owned-stream fixtures already held theirs (they passed on 1.99). Tests read
and write through raw places, `(*object.as_ptr()).field`. Each access covers
only the field it names, and no reference outlives the expression. Callbacks
borrow through `TestObject::in_callback` for one call. Converted:

- `device_io` pending and alias tests;
- `transfer_group` adapter, pending, source and transport adapter (plain and
  pending) tests;
- `unix_readiness` adapter and pending tests, including the file and standard
  submodules;
- both Windows completion fixtures, checked by type only.

| Option | Pros | Cons |
|---|---|---|
| `TestObject` with raw-place access (chosen) | One owner and one access rule for every fixture; matches how compiled code and the passing owned fixtures hold objects; field-precise, so pointers the runtime holds into other fields stay valid | `(*x.as_ptr()).field` is noisier than `x.field`; the discipline is "no reference across a runtime call", not a type-level guarantee |
| `Cell`/`RefCell` fields, with callbacks taking `&T` | The type system forbids plain writes through the shared word | Rewrites every fixture struct and callback body; staged storage needs `Cell<[u8; N]>` pointers |
| Closure accessors `read(\|s\| …)` / `write(\|s\| …)` | No reference can escape the closure | `write` reborrows the whole object, invalidating pointers the runtime holds into other fields (Stacked Borrows); tried and replaced |
| Fix only the two failing tests | Smallest diff | The same overlap stays in every other fixture, waiting for the next optimizer |

The non-test runtime does not have this shape. `transfer_group/pending.rs` and
`pending/host.rs` hand callbacks raw pointers (the request view, the driver)
and touch that state only through those pointers or after the call returns.
Callbacks still take a whole-object `&mut` for their call, as before. Proving
the runtime and fixtures sound under Stacked or Tree Borrows needs Miri, which
is not installed, and installing it needs Andrew's permission for the download.

### Open on Windows after this change

Two Windows failures are outside this change:

- **The `doria-rt` test binary aborts.** In run 42397207626 the Windows job's
  release `doria-rt` tests stopped with `STATUS_STACK_BUFFER_OVERRUN`
  (`0xc0000409`) after 8 of 605 tests. That status is how Windows reports a
  Rust abort, for example a panic inside an `extern "C"` callback. The log
  names no test.
  `byte_io::tests::windows_nonblocking_pipe_exposes_backpressure_and_resumes_without_replay`
  never reported, although the test after it alphabetically did. It is new in
  `5e72056`, so it is the first suspect. Identifying it needs a Windows run with
  `cargo test -p doria-rt --release --lib -- --test-threads=1`.
- **Windows clippy fails.** `cargo clippy -p doria-rt --all-targets --target x86_64-pc-windows-msvc -- -D warnings`
  stops on `clashing_extern_declarations`. `process/windows.rs` declares Win32
  `CreatePipe` with `*const SecurityAttributes`, while five other modules
  declare it with `*mut c_void`. doria-rt has no single owner for Win32
  bindings: 24 files hold 44 separate `extern "system"` blocks.
  Giving `CreatePipe`, and the bindings like it, one declaration is the fix.

### Win32 bindings get one owner (2026-11-07)

Windows clippy failed on `clashing_extern_declarations`. Two declarations of
one foreign symbol bind to the same function, so when they disagree they
describe one call two ways. The lint caught one instance, `CreatePipe`. The
cause was ownership: 24 files declared their own Win32 imports in 44 blocks.
`GetLastError` appeared 8 times, `WaitForSingleObject` 5 and `CreatePipe` 6.
Three modules each defined `OVERLAPPED` (`windows_completion::Overlapped`,
a local struct in `byte_io` `append` and `locking::LockRequest`), and two
defined `SRWLOCK` and `CONDITION_VARIABLE`.

**Chosen:** `crates/doria-rt/src/win32.rs` declares each Win32 import the
runtime and its tests call once, with SDK parameter types:

- `_In_` structures are `*const`, buffers and out-parameters `*mut`, handles
  `NativeHandle`.
- The structures those signatures name live there too: `Overlapped`,
  `SecurityAttributes`, `SrwLock`, `ConditionVariable`, `StartupInfo`,
  `StartupInfoEx`, `ProcessInformation` and the `ThreadStart` entry type.
- Every runtime and test module imports from it. No `extern "system"`
  import block remains elsewhere in `doria-rt`.
- Each binding carries its users' `cfg`, so `dead_code` still reports an
  unused binding. The seven called only from tests are `#[cfg(test)]`.
- `Overlapped` keeps `UnsafeCell` for the kernel-written words and stays
  `!Unpin`. It uses `PhantomData<PhantomPinned>` because `PhantomPinned`
  itself trips `improper_ctypes` once the type appears in extern signatures.
- `SrwLock` and `ConditionVariable` keep their word private. A test-only
  `is_initial` replaces the broker sync test's direct field read.

doriac's three Windows imports (`CompareStringOrdinal`,
`GetFileInformationByHandle`, `MoveFileExW` in `main.rs`) are each declared
once, and doria-time's are separate crates' own. Neither crate needs a
change.

| Option | Pros | Cons |
|---|---|---|
| One owner module (chosen) | One signature per import, so drift cannot recur; one definition of each shared Win32 structure; the compiler checks every caller against it | A cross-cutting change to 24 files; bindings carry `cfg` gates that mirror their callers |
| Align `CreatePipe`'s signatures only | Smallest diff | Leaves the 44 private copies to drift again; the next clash is found the same late way |
| `#[allow(clashing_extern_declarations)]` | One line | Hides a real ABI ambiguity rather than removing it |
| The `windows-sys` crate | Generated, complete SDK bindings | A new dependency, which needs Andrew, the network and a lock change; much larger than the 64 imports used |

On the Windows test abort: no binding changed its ABI. Every old declaration
already had the SDK arity and register-compatible parameter types. The
single `CreatePipe` mismatch was `*const` against `*mut`, which is
ABI-identical. So the binding drift does not explain
`STATUS_STACK_BUFFER_OVERRUN`.

A runtime panic exits with status 101 (`ExitProcess`), not `0xc0000409`. So
the abort is a Rust panic inside a non-unwinding `extern "C"` function,
either a `dr_v1_*` entry point or a test callback. libtest captures the panic
message, and the abort discards it, which is why the log names no test. The
test undefined behavior fixed in the previous section ran assertions inside
exactly such callbacks, so CI on `42a7ad5` shows whether the abort survived
that fix.
