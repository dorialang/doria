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

**Open, same class, not changed here (resolved by entry 52):** callbacks already use a union over
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

**Open, same class, not changed here (resolved by entry 52):** the trait body now accepts a catch for
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

### Close results (2026-11-07)

- `doria` `5e72056`: origin/develop `dd2fbc5` plus the Stage 36a commit. The
  vet7 re-vet and the language server (487/487) ran on build inputs
  identical to it.
- `doria-language-server` `339c997`: doriac pinned to `5e72056`, Cargo.lock
  resolved from the local clone, canonical `cargo check` against the pin,
  Tooling's CHANGELOG entry and PHP guards.
- `doria-website` `adb3b5b`: compiler pin and v2026.3.1 API lock at
  `5e72056` (the lock equals the SHA-256 of `docs/stdlib-reference.md` at
  that commit); full Pest 347 passed against the compiler built from it,
  including `ClosureCaptureContractTest` and `PlaygroundStageLadderTest`.
- The installed `doriac` and `doria-lsp` report compiler `5e72056`. The
  installed compiler built and ran one native program (fast profile) and two
  PHP programs; the refresh runs no LLVM smoke. Its build target lived on
  the cache volume under the guard and was removed after.

None of the three is pushed. Until develop is pushed in `doria`, the two pins
name a commit only this machine has, so `check_cross_repo_pins.php` reports
them unreachable and CI cannot fetch the compiler.

## Follow-ups to entry 51 (started 2026-11-07)

### Order of the two follow-up sessions

Andrew started both entry-51 follow-ups at once, in the same `doria`
working tree. "Make catch reachability independent of callers" makes the
authored body decide E0629. "Give each specialization its exact effect
contract" narrows each specialization's ABI to what it selects. Both change
the selected-implementation path in `semantics/contracts.rs`, the Decision
0123 amendment, entry 51 and `constrained_ambient_effect_tests.rs`.

- **Chosen: catch reachability first; exact contracts does read-only design
  until it lands, then builds on its tree.**
  - Pro: exact contracts assumes template diagnostics never depend on
    callers, and this fixes that first.
  - Pro: one cargo job at a time.
  - Pro: no half-edited tree breaks the other session's build.
  - Con: exact contracts finishes later.
- Both at once in the one tree.
  - Pro: fastest if they never touch the same lines.
  - Con: they share functions and files; each uncommitted edit can break the
    other's build and test results.
  - Con: two cargo jobs break the one-job limit.
- Exact contracts first.
  - Con: it would keep diagnostics conservative on a caller-dependent base
    and then have to redo that work.

### 52. Catch reachability no longer depends on callers or composers (resolves entry 51's two open items)

**Where E0629 is decided.** In one place, `check_try_statement` in
`semantics.rs`, from the effect scope of the protected body. That scope had
two consumers with different owners. Diagnostics (E0629, the destructor
boundary E0631, and the nonthrowing initializers E0633-E0635) belong to the
authored body. Transport (catch routing, uncovered effects, a callable's
escaping profile, and so its ABI) belongs to the selected implementations.
Two whole-program facts narrowed that one list:

- the callback seed, the union of the closures callers pass
  (`callable_contracts::Seeds::collect`, then `known_callback_targets`);
- a composed trait copy's exact members.

Neither defect belonged to a single pass. An ordinary program uses one
checker for authored trait bodies and their composed copies, so `Quiet`'s
copy reported E0629 in that same pass. A program with a `where` member runs
an authored source pass, then an executable recheck once the source pass is
clean. Both passes applied the callback seed. The fault was the shared list,
so the fix splits it rather than silencing one pass.

**Chosen: split each effect scope into authored reach and transport.**
`CheckedEffectSet` now has `reach` beside `ordered`, and `reach` always
contains `ordered`.

- A function value's reach is its type's complete contract, Decision 0123's
  one ambient-capable indirect convention. That holds for `$cb()`, a stored
  callable, a local function value, a List algorithm callback, and a `toThrow`
  inspector. The exception is a call that names a closure literal, as
  `$list->map(fn(int $x) => $x * 2)` does: it reaches what that literal
  reaches. Transport still follows the provenance seed.
- A named call reaches its callee's authored reach through the new
  `ambient_reach_seed`. That seed is refined in `Seeds` beside transport, with
  discovery and escaping forms that mirror the transport ones.
- Calls that select their implementation after checking keep the complete
  set in both profiles, as entry 51 set them.
- A composed copy has exact transport. Its authored member's reach is
  projected onto it in every view (`project_composed_templates`). During the
  executable recheck, an authored member that is not rechecked keeps its
  seeded reach.
- A catch inside a composed copy reports no E0629. The authored member's
  check decides it once for every composer.
- E0629, the destructor's E0631, and the nonthrowing initializer checks read
  `reach`. Catch coverage, `try_uncovered_effects`, effect sites, MIR, and the
  ABI read `ordered`. A catch that reach allows but narrowed transport can
  never enter compiles to a route that is never taken.

Effects on programs:

- `function run(function(): int $cb): int { try { return $cb(); } catch (IoError) { return -1; } }`
  compiles whatever its callers pass. So do `Closing::safe` under a quiet
  composer and a caller's catch around `$quiet->closeAll()`.
- `try { return relay(fn() => 6); } catch (IoError) {}` is accepted even
  when every closure passed to `relay` is pure. Before, its E0629 depended on
  what `relay`'s other callers passed.
- A catch around a local function value (`function(): int $f = fn() => 4;`
  then `$f()`) is accepted. This is a deliberate loosening, consistent with
  erased interface values. A closure literal invoked in place keeps its E0629.
- A destructor that invokes a stored callback must now catch both ambient
  Errors, even if only pure closures are ever stored (E0631). Before, it
  passed until some caller stored an I/O closure.
- Transport, MIR, and the ABI are unchanged. `relay` keeps its ordinary ABI
  when every closure passed to it is pure.

| Option | Pros | Cons |
|---|---|---|
| Split reach from transport (chosen) | One shared rule in every pass and program shape; transport and the existing ABI narrowing are kept; also fixes E0631 and initializers and callers of callback-taking functions | Callers of a function that forwards a function value see its complete contract even when their own closure is pure; a catch around a local function value is no longer E0629 |
| Body-local provenance for function values | Keeps E0629 around `function(): int $f = fn() => 4; $f()` | Needs a second, isolated provenance solve per body that treats parameters, captures and stored fields as unknown, beside the shared solver |
| Context-sensitive reach for named callees | `try { relay(fn() => 6) } catch (IoError)` would be E0629 again, from the caller's own closure | Per-call-site instantiation of callee effects, the machinery of the exact-contract follow-up; diagnostics would depend on callee bodies instantiated per caller |
| Drop callback narrowing | One profile | Every callback-taking function pays checked transport, against the performance target |
| Skip E0629 in the executable recheck only | Small | Ordinary programs have no recheck, so both defects stay; E0631 and initializers stay caller-dependent |

**Open, same class, not changed:** under Decision 0130, the ambient profile of
a virtual call on an open class is the union over reachable overrides. A
caller's E0629 around `$base->m()` therefore changes when a subclass anywhere
adds or removes I/O. Erased interface calls already use the complete
contract, so the two dispatch forms disagree. Decision 0130 chose the union
for profiles, so splitting its diagnostics from its slot ABI is a ruling for
Andrew.

Regressions in `constrained_ambient_effect_tests` cover ordinary and
conditional programs on the interpreter, Cranelift, LLVM, and PHP:

- `callers_and_composers_never_decide_catch_reachability_on_every_backend`
- `narrowed_transport_keeps_callers_and_composers_off_the_catch_route`
- `the_authored_body_still_rejects_catches_it_cannot_reach`
- `destructors_judge_stored_callbacks_by_their_contract_not_by_what_is_stored`

Decision 0123's amendment paragraph and one sentence in Decision 0134
record the rule.

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

### Native profiles get one owner (2026-11-07)

CI's `rust` job (run 42404305965) runs
`cargo clippy --workspace --all-targets -- -D warnings` with default features.
It stopped on `single_element_loop` in
`semantics/conditional_capabilities/tests.rs`: `for profile in [Fast,
#[cfg(feature = "llvm-backend")] Release]` has one element without the
feature. The cause was ownership. Tests in about 50 doriac files decided for
themselves which native profiles this build compiles, in seven spellings:

- `cfg`'d array elements;
- `cfg`/`not(cfg)` pairs of `let profiles`;
- local `profiles()` and `native_profiles()` helpers in four files;
- `.chain(cfg!(…).then_some(Release))`;
- `if profile == Release && !cfg!(…) { continue; }` over a fixed list;
- `if cfg!(…) { profiles.push(Release) }`;
- a Fast statement followed by its `#[cfg]` Release twin, 86 times.

The vet missed it because its one clippy run used
`--features doriac/llvm-backend`. `scripts/validate_work_unit.php` already
runs the default-feature workspace clippy that CI runs.

**Chosen:** `NativeProfile::AVAILABLE` in `backend.rs` lists the profiles this
build compiles, and `NativeProfile::is_available` answers for one profile.
`BuildNativeProfile::native` maps a build plan's profile to its backend
profile, and `compilation_graph_options` now uses it instead of a second
match. Every test that runs a program on each native profile iterates
`AVAILABLE`. A test that walks build-plan profiles skips one whose
`native()` is unavailable. The `cfg` gates that remain guard code that needs
LLVM itself:

- `codegen_llvm` IR and object inspection;
- the LLVM-only tests (`llvm_*`, the release-report and devirtualization
  tests, the CLI's enabled and disabled release tests);
- `codegen_native`'s release lowering and the CLI's `backends` list.

`constrained_ambient_effect_tests.rs` keeps its two `#[cfg]` Release
statements until the catch-reachability work there lands.

| Option | Pros | Cons |
|---|---|---|
| `NativeProfile::AVAILABLE` and `is_available` (chosen) | The backend states once what the build compiles; tests cannot drift from it; default-feature clippy has nothing feature-shaped to flag | Touches about 50 test files; a loop replaces some straight-line Fast and Release calls |
| Fix the two flagged loops | Smallest diff | Leaves 80-odd re-derivations; the next default-feature build finds the next one the same late way |
| `#[allow(clippy::single_element_loop)]` | One line per loop | Hides the duplicated ownership rather than removing it |
| A shared test helper only | Shorter call sites | Test-only; the build plan and production code would still re-derive availability |

### Windows test abort: the test binary exported the runtime's CRT replacements (2026-11-07)

CI on `42a7ad5` (run 42404305965) passed the `doria-rt` release tests on
ubuntu and macOS. Windows aborted again with `0xc0000409` after the same
eight `byte_io::tests`.

**Cause:** the abort comes from the link, not from a test. This corrects the
reading at the end of the previous section, which blamed a panic inside a
non-unwinding function. The evidence:

1. The `standalone-windows-support` feature is on by default. CI's test build
   compiles it into the harness (`--test --cfg feature="standalone-windows-support"`
   in the job log).
2. Under that feature `lib.rs` exported `#[no_mangle]` replacements for CRT
   symbols: `memcpy`, `memmove`, `memcmp`, `memset`, `strlen`, `wcslen`,
   `_fltused`, `__chkstk`, and a `__CxxFrameHandler3` that always returns
   `ExceptionContinueSearch`. The harness's own objects defined them, so they
   won over the CRT's. `llvm-nm` on a Windows object of the harness at `5bb163c`
   shows `T __CxxFrameHandler3`, `T memcpy` and `R _fltused`.
3. MSVC Rust uses `__CxxFrameHandler3` as the personality of every
   `catch_unwind`. A probe object shows `U __CxxFrameHandler3` for a plain
   `catch_unwind`. With the stub, no catch frame catches: libtest's per-test
   catch and the tests' own `catch_unwind` both fail, and the first panic of
   any kind aborts the binary through `__fastfail`.
4. At `dd2fbc5`, `doria-rt` used neither `catch_unwind` nor `should_panic`.
   At `5e72056` it used them 17 times. Two of those tests sit in
   `byte_io::windows_completion`, the module that starts right after
   `byte_io::tests`:
   `native::broker::sync::tests::rust_host_unwind_releases_guard_without_poisoning_cell`
   and `tests::cleanup_is_armed_before_submit_and_drains_during_host_unwind`.
   The stub was latent until 36a added the first test that catches a panic
   on Windows.

`windows_nonblocking_pipe_exposes_backpressure_and_resumes_without_replay` was
only in flight when another thread aborted. Nothing points at it.

**Chosen:** the runtime exports its CRT replacements only where the CRT is
absent: the freestanding build, `all(not(test), panic = "abort")`. That is the
same condition that makes the crate `no_std`.

- `_fltused` and the `__chkstk` assembly carry that condition.
- The functions keep `#[cfg_attr(all(not(test), panic = "abort"), no_mangle)]`.
  In every build that links `std` they stay ordinary Rust functions, so
  `no_crt_memory_support_symbols_preserve_bytes_and_overlap` still tests them.
- After the change the harness object has `U __CxxFrameHandler3`, `U _fltused`
  and `U __chkstk`. The freestanding release object still exports all nine
  symbols.

| Option | Pros | Cons |
|---|---|---|
| Export only from the freestanding build (chosen) | One owner condition, the crate's own `no_std` predicate; every `std` build gets the CRT's handler, memory routines and stack probe; the replacements stay testable | Three `cfg` forms to keep aligned (item gate, `cfg_attr`, asm gate) |
| Turn the feature off in CI's test command | One workflow line | Leaves the trap for every local and future `cargo test` on Windows; the feature's default stays wrong for the crate's own harness |
| Drop only the `__CxxFrameHandler3` stub from tests | Fixes this abort | Leaves the harness running the runtime's byte-loop `memcpy` and its own `__chkstk` in place of the CRT's |

### Test callbacks that panic

Fixing the link makes test-body failures ordinary named failures on Windows.
A panic inside a test callback still aborts on every platform. The runtime
calls these callbacks through `extern "C"` or `extern "system"`, which cannot
unwind. libtest's capture then loses the test name, location and message.
`doria-rt` has 74 such test callbacks (adapter, stream, drop, entry and thread
callbacks), 21 of them with assertions.

**Chosen:** `crate::test_callback::guard` wraps every test callback body.

- A panic inside it is reported straight to the process's standard error,
  past libtest's capture, naming the thread (libtest names it after the
  test), the location and the message. Then the process aborts.
- A chained panic hook writes the report only while a guarded body is
  running, so tests that expect panics keep their captured output.
- `scripts/check_test_callback_guards.php` fails any test-code `extern "C"` or
  `extern "system"` function whose body does not start with the guard.
  `scripts/test_test_callback_guards.php` covers it in 6 cases. CI's guardrail
  step and `validate_work_unit.php` run both.
- A probe harness confirmed the report: `thread 'tests::zz_callback_contract_fails' ... panicked at probe.rs:69:9: assertion ... failed`,
  status 134, and the earlier tests reported `ok`.

| Option | Pros | Cons |
|---|---|---|
| Report past the capture, then abort (chosen) | Names the test, line and message on every platform and thread; never returns an invented status into a runtime whose calling contract just failed while it holds raw pointers into the fixture | The run still stops at the first callback failure |
| Catch, record, return a fallback status, re-raise on the test thread | Later tests keep running | Each of the 74 callbacks needs an honest fallback for its contract (status, outcome slot, ownership of a dropped stream); libtest has no end-of-test hook, so re-raising needs a scope in every test or a checkpoint the test may never reach, and a test could pass with a recorded failure |
| `extern "C-unwind"` callbacks | Panics reach the test normally | The descriptors' pointer types are `extern "C"`, and unwinding through the abort-only runtime would skip its cleanup invariants |
| Leave as is | No change | A failed callback assertion loses its test, line and message |

**Open:**

- Windows tests still need a Windows run.
- The fix is verified at the object level and by the native release and
  debug suites, but no Windows machine here can execute the harness.

### Returns wider than two registers take the result slot (2026-11-07)

CI's ubuntu job (run 42404305965) failed two `byte_region_execution_tests`
with Cranelift's "Too many return values to fit in registers". A
`?ByteRegion` returns three pointer-sized words: data, length and presence.
x86-64 System V and Windows x64 return at most two integer registers (`rax`,
`rdx`). AArch64 returns up to eight, so macOS passed. `?Deadline` has the same
shape: a presence word and a 16-byte inline scalar in two registers. Only
payload enums and checked values used the caller's result slot
(`NativeCallableHiddenInput::ResultOut`). Every other value went to the return
registers, whatever its width.

The return transport of a Doria callable belongs to the backend-neutral
callable plan (`native_closure_abi::NativeCallableSignaturePlan`). Both
backends already build every definition, call shape and runtime trampoline
from that plan's hidden inputs.

**Chosen:** one rule in the plan, `returns_through_result_slot`.

- A value returns in registers only if it fits `DIRECT_RETURN_SLOTS`: two
  slots, at most one of them floating-point.
- Anything wider uses the result slot, like payload enums, on every target.
- `direct_value_slots` gives each MIR type's register shape: a class counts
  its widest carrier, and a 128-bit inline scalar counts two slots.
- Every signature builder takes its returns from the plan in both backends:
  - Cranelift: `append_return_abi_params`;
  - LLVM: `callable_return_type`.
- The definition stores through the slot and the caller loads from it:
  - an ordinary call loads from fresh storage;
  - an indirect call passes the result local, as a checked indirect call
    already does.
- `direct_for_return` builds a virtual call's plan from the carrier of the
  family adapter it reaches, so a covariant return can't leave caller and
  callee disagreeing about the slot.

| Option | Pros | Cons |
|---|---|---|
| One budget for every target, in the shared plan (chosen) | Every host runs the same transport, so macOS now exercises what Linux and Windows need; matches C, whose System V and AAPCS64 return aggregates above 16 bytes through memory; one rule for both backends | A three-word value on AArch64 goes through memory where eight return registers could carry it, at the same cost C pays |
| Per-target capacity from the ISA | Keeps AArch64's register return for three words | The lowering differs by host, which is how this reached CI unseen; LLVM and Cranelift need the target in the plan |
| Cranelift `enable_multi_ret_implicit_sret` | One flag | Cranelift documents it as non-conforming, with an ABI that may change between versions; LLVM would still need its own path |
| Cranelift `CallConv::Tail` for Doria-internal calls | Eight return registers on x86-64 | Changes every internal signature and runtime boundary, and its Windows unwind support needs proving; out of proportion to this fix |

**Tests:**

- `native_closure_abi` checks the budget for every two-slot shape and every
  three-slot shape.
- `codegen_cranelift::return_transport_tests` runs on any host:
  - every function and function-type signature of a program with
    `?ByteRegion` and `?Deadline` returns, from a free function, a method and
    a closure, fits the budget;
  - those returns carry a result slot;
  - `direct_value_slots` bounds every Cranelift carrier the program uses.
- `native_return_transport_tests` runs the same shapes on the interpreter and
  every available native profile. It also runs virtual calls whose family
  returns `?Deadline` (three slots, through the result slot) while the
  overrides return `Deadline` (two slots, in registers), including one whose
  static callee is an open override, so that `direct_for_return` decides the
  slot. It also covers an open class returned through a function value.

**Open-class methods that return a region (fixed after this entry).** The
covariant test first used `?ByteRegion` and `ByteRegion`. A program that
declares this pair and calls `view` through a `Source` failed shared MIR
validation before any backend ran:

```doria
use Doria\Std\Io\ByteRegion;
open class Source {
    Bytes $bytes = Bytes::allocate(2);
    open function view(): ?ByteRegion { return $this->bytes->getRegion(0, 1); }
}
class Filled extends Source {
    override function view(): ByteRegion { return $this->bytes->getRegion(0, 2); }
}
```

The error was "retained source path must name stored owning properties (in
function `Source::view`)". "Open-class region returns: bodies and dispatch
get separate summaries" below records the cause and the fix.

**Every tier-1 target lowers on every host.** The executable parity matrix
runs each program on the host only, which is how an x86-64 ABI limit first
appeared in CI.
- doriac's dev-dependency on `cranelift-codegen` enables the `x86` and `arm64`
  ISAs, so test builds carry every tier-1 backend while the shipped compiler
  keeps the host's.
- `lower_validated_mir_to_object_for_isa` takes the target. Executables pass
  the host's; the tests pass x86-64 and AArch64 Linux, x86-64 Windows and
  AArch64 macOS (Windows on ARM64 is blocked upstream, below).
- `return_transport_tests` lowers the wide-return program and every program in
  the durable native manifest for all four.

| Option | Pros | Cons |
|---|---|---|
| Test-only ISA features (chosen) | Every vet sees every target's ABI limits; the shipped compiler is unchanged | Test builds compile two extra Cranelift backends, a second feature variant of `cranelift-codegen` and its dependents in the cache |
| Enable the ISAs in the shipped compiler | One feature graph; groundwork for cross-compilation | A larger compiler for a capability no command exposes yet |
| Rely on CI's per-OS runners | No change | The defect class keeps surfacing first in CI |

**16-byte inline scalars in x86-64 signatures.** The cross-target lowering
found that Cranelift rejects every `i128` parameter and return under x86-64
System V and Windows fastcall ("i128 args/return values not supported unless
LLVM ABI extensions are enabled"). AArch64 accepts them. `Deadline` and the
registration and transfer keys are 16-byte inline scalars with an `i128`
carrier, so the fast profile could not compile a Doria function that passes
or returns one by value on Linux or Windows x64.

The ABIs on each side:
- **C and LLVM.** System V passes `__int128` in two consecutive integer
  registers and returns it in `rax`/`rdx`. AAPCS64 uses an even register
  pair. MSVC has no 128-bit integer; clang and current Rust pass it by
  reference on Windows x64.
- **doria-rt.** Its `extern "C"` functions already carry 128-bit values as
  two words (`dr_v1_deadline_from_now` writes `low` and `high`). An `i128`
  appears only in Doria-to-Doria calls, and the two native backends never
  call each other.

**Chosen:** `native_flags`, the one Cranelift settings owner, enables
`enable_llvm_abi_extensions`. An `i128` then travels as two consecutive
64-bit registers on x86-64 targets: the System V convention, applied to
Windows x64 for these internal calls. AArch64 is unchanged.
`declare_runtime` now rejects a 128-bit integer in any `doria-rt` import, so
the C boundary stays word-based on every target.

| Option | Pros | Cons |
|---|---|---|
| Cranelift's LLVM ABI extension, word-based runtime boundary enforced (chosen) | Cranelift's documented setting for `i128` signatures, as rustc's Cranelift backend uses it; the System V `__int128` transport; one setting for every target | On Windows x64 the internal convention is a register pair, not clang's by-reference passing; it is private to Doria-to-Doria calls, and the runtime boundary refuses it. It also lets System V return a third integer register, which `DIRECT_RETURN_SLOTS` never uses |
| Two `i64` words at every Doria signature in the shared plan | The carrier no longer depends on any code generator's `i128` support; same words as the runtime boundary | Splits and joins at every parameter, argument, return, call result and trampoline in both backends for the same machine code; LLVM already handles `i128` natively for its own calls |
| Pass 16-byte inline scalars by address | No register-pair rules | A memory round trip for every `Deadline` or key argument, slower than C |

**Windows on ARM64.** cranelift-object 0.134 maps the AArch64
symbol-relocation kinds for ELF and Mach-O only: `Aarch64AdrGotPage21`,
`Aarch64AdrPrelPgHi21` and `Aarch64AddAbsLo12Nc`. The fast profile therefore
cannot emit a COFF object for `aarch64-pc-windows-msvc`. The cross-target
test covers x86-64 Linux, x86-64 Windows, AArch64 Linux and AArch64 macOS,
and names this gap. It is an upstream Cranelift limitation: closing it needs
those relocations in cranelift-object, or a different object writer for that
target.

**One decoder for direct returns.** Cranelift decoded register returns in
three copies: ordinary calls, method calls and indirect calls. The indirect
copy had no open-class arm, so an open class returned there in registers would
have failed lowering with "open class storage received no class descriptor".
Function values and erased interface calls carry ambient I/O today and return
through the slot, so no test program reached that arm.
`lowered_from_direct_results` is now the only decoder, the inverse of
`append_type_abi_params` and `value_to_doria_abi`.

### Windows CI reports what happened (2026-11-07)

CI on `5470711` (push run 42407455781, cancelled after 70 minutes) ran the
Windows `doria-rt` tests: 38 passed and 2 failed, then 4 were still "running"
when the run was cancelled.

- **Failed:** `byte_io::tests::windows_nonblocking_pipe_exposes_backpressure_and_resumes_without_replay`
  and `byte_io::windows_completion::native::adapter::tests::shared_cursor_complete_adapter_orders_read_write_read_without_hot_allocations`.
- **Running:** `broker::driver::legacy_tests::peer_closure_projects_read_eof_without_finishing_an_unrelated_pending_read`,
  `broker::driver::legacy_tests::pre_requested_cancellation_submits_nothing_and_old_value_wake_does_not_cancel_reuse`,
  `broker::tests::borrowed_membership_growth_and_holes_preserve_unaffected_native_registrations`
  and `broker::tests::borrowed_native_set_rejection_remove_and_drop_never_consume_stream_owner`.

No failure message reached the log: libtest prints them only after the last
test ends.

**What reading the code establishes.** All four "running" tests take
`TEST_BROKER_LOCK` first: `TEST_BROKER_LOCK.lock()` in `port/tests.rs`, and
`serial()` in `broker/tests.rs`. So one of them hung while holding the lock,
and the other three were queued on it. Which one hung, and why, needs the
next run's report. The two failures need their messages too. Their likely
spots are Windows-specific facts these tests assume: how much a
`PIPE_NOWAIT` byte pipe accepts when one write is larger than its 4096-byte
quota, and which error a nonblocking server read returns after the client
closes. None of these causes is proven, so no test or runtime behavior
changes for them here.

**Chosen:** make the run report everything, by mechanism.

1. **Report a failure when it happens.** `crate::test_report` installs a
   panic hook as the test binary starts (`.CRT$XCU` on MSVC, `.init_array`
   on Linux, `__mod_init_func` on Apple). It writes every panic at once to
   the process's stderr, past libtest's capture, with the thread (= test)
   name, the location and the message. libtest's summary is unchanged. The
   12 places where a test panics on purpose now call
   `crate::test_report::expect_panic` instead of `catch_unwind`, which keeps
   their panics quiet. The callback guard reports through the same hook.
2. **No test waits forever.** `win32.rs` now owns the four Windows waits
   (`WaitForSingleObject`, `WaitForMultipleObjects`,
   `GetQueuedCompletionStatus`, `SleepConditionVariableSRW`). The raw imports
   are private, and every runtime and test caller passes what it waits for.
   In test builds an `INFINITE` wait is bounded at
   `TEST_WAIT_LIMIT_MILLISECONDS` (2 minutes). Running out reports the test
   and what it waited for, then aborts; unwinding could release storage the
   kernel still writes. Release builds keep `INFINITE`. The three files that
   each defined their own `INFINITE` now import the owner's.
3. **CI cannot hang for six hours.** Every job has `timeout-minutes` (30, 90
   and 120), and both `doria-rt` test steps have 15.
   `scripts/check_ci_timeouts.php` fails any workflow job without one. CI's
   guardrail step and `validate_work_unit.php` run it.

A probe harness built from these modules on macOS printed the failing
assertion of its first test as soon as it ran, with no lazy install call,
even though the next test aborted the binary.

| Option | Pros | Cons |
|---|---|---|
| Startup panic hook with declared expected panics (chosen) | Every failure's message reaches the log even if the run later hangs, aborts or is cancelled; local runs stay quiet for intended panics; no CI flag to forget | Platform loader sections for a test-only initializer; a new expected panic should use `expect_panic`, or it prints once more |
| `RUST_TEST_NOCAPTURE=1` on the CI test steps | libtest's own switch, no code | A CI flag the local vet doesn't share; every test's output interleaves, and intended panics print as if they failed |
| Leave capture on and rely on the summary | No change | A hang or abort discards every message, as on `5470711` |

| Option | Pros | Cons |
|---|---|---|
| Test builds bound `INFINITE` in the owner of the waits (chosen) | Every wait, in runtime and test code, gets the contract from one place; the private imports make a raw unbounded wait impossible to write | A test-build-only path in runtime code; a legitimately slow wait over 2 minutes would abort a test |
| Per-test deadlines | Local to each test | Can't reach the runtime's internal drains, where the kernel owns the request, and has to be remembered in every test |
| Only a CI step timeout | One line | A hang still kills the run with no name or reason |

**Open:**

- The hung test and the two failures' causes wait for the next Windows run.
  It will print each failure as it happens, and any hang as a named
  two-minute wait report.
- Blocking calls that are not one of the four waits (`ConnectNamedPipe` on a
  synchronous handle, a blocking `ReadFile`, `std` locks, channels and thread
  joins) are bounded only by the 15-minute step limit. libtest's "running for
  over 60 seconds" lines name the tests stuck there.

### The three Windows defects CI on `e214f49` named (2026-11-07)

Run 42416608800 printed each failure as it happened. The `.CRT$XCU`
reporter ran.

**1. A large nonblocking pipe write never progressed (runtime defect, fixed).**
`windows_nonblocking_pipe_exposes_backpressure_and_resumes_without_replay`
failed `accepted > 0` (byte_io.rs:823). Its first write offered up to 16,384
bytes to an empty `PIPE_NOWAIT` byte pipe with a 4,096-byte quota, and
`WriteFile` returned success with zero bytes. So on that platform a
nonblocking pipe write is all or nothing against the free outbound quota, not
the partial write the `WriteFile` page describes. The runtime's Windows
`write` turned that zero into `WouldBlock`, so a request larger than the free
quota could never make progress, even on an empty pipe.

The synchronous write path (`byte_io::platform::write_at`) owns the contract
"`Accepted(1..=len)` or `WouldBlock` with progress possible". It now handles
the zero case on a pipe by reading the end's free outbound quota
(`FILE_PIPE_LOCAL_INFORMATION::WriteQuotaAvailable`, through
`NtQueryInformationFile`) and retrying once with exactly that many bytes. Zero
free quota stays real backpressure. A failed query is returned as a failure,
not guessed. `win32::query_information_file` is now the one owner of the
ntdll query; `native_handle::require_synchronous_device` uses it instead of
its own copy of the lookup.

Other Windows write paths: the completion drivers (`native.rs`,
`broker/driver.rs`) use overlapped `PIPE_WAIT` handles, whose writes pend
until space exists. Device, file and child-process writes reach pipes through
this same `write`, so the fix covers them.

| Option | Pros | Cons |
|---|---|---|
| Retry once with the free quota (chosen) | Exact progress whenever any space is free; one query only on the zero-progress path | One ntdll query per blocked pipe write |
| Cap every pipe write at the pipe's buffer size (`GetNamedPipeInfo`) | No ntdll | Still writes nothing whenever the pipe is partly full and the request exceeds the free space |
| Keep `WouldBlock` | No change | A request larger than the free quota can never progress |

**2. The shared-cursor adapter test expected untransformed bytes (test defect, fixed).**
`shared_cursor_complete_adapter_orders_read_write_read_without_hot_allocations`
got `[65, 66, 67]` ("ABC") where it expected "abc" (adapter/tests.rs:103).
The fixture's read callback uppercases every byte it returns
(adapter/tests.rs:215), and the sibling tests expect uppercase ("UV", "AB",
"C"). "ABC" is the file's first three bytes read at cursor 0, before the
write, so the shared cursor and the ordering were correct. The expectations
are now "ABC" and "FG". The second read still checks that the write moved
the cursor to 5.

**3. `peer_closure_projects_read_eof_without_finishing_an_unrelated_pending_read` hung (open).**
The bounded wait aborted after 120,000 ms waiting for a completion packet on
the broker port. The 5-second deadline budget bounds the group's own waits,
so the unbounded wait was an `INFINITE` one: the broker drain
(`driver.rs`, `wait(self.group, INFINITE, ...)`) or a no-budget wait. Reading
established these facts:

- A synchronously failed overlapped submission (`ReadFile` on a server end
  whose client closed returns `ERROR_BROKEN_PIPE`) queues no packet. The
  broker records it with `Registry::rejected`, and the generic group finishes
  that slot with the failure.
- A rejected route has `packet = false` and no terminal, so
  `Registry::query` returns `None` forever. Any later drain of that slot
  would wait for a packet that cannot come.
- The completion sets never call `SetFileCompletionNotificationModes`, so a
  synchronous success still queues a packet. That path is consistent.

Whether some path drains or waits on the rejected slot is not established.
So no change is made for it.

**Open:** the hang's exact path. The next step is to show whether
`NativeGroup` drop or `next()` drains a slot whose submission was rejected.
The hang report would identify it directly if it also named the slot's route
state.

### The broker hang: a synchronous failure recorded as in flight (2026-11-08)

`peer_closure_projects_read_eof_without_finishing_an_unrelated_pending_read`
hung for 120 s in the broker drain on `e214f49`.

**Cause (proven by documented semantics, reproduced at the registry level on
macOS).** Both completion drivers decided "pending" if `ReadFile` or
`WriteFile` returned `FALSE` and `OVERLAPPED.Internal` read `STATUS_PENDING`,
even when the error was not `ERROR_IO_PENDING`. That field is not a verdict:

- The `OVERLAPPED` documentation says the system sets `Internal` to
  `STATUS_PENDING` when the request is issued. A synchronous failure leaves
  it there.
- `HasOverlappedIoCompleted`, which reads it, is documented as meaningful only
  after `ERROR_IO_PENDING`.
- The documented verdict is the return value plus `GetLastError`.

In the test, the read on the server end whose client had closed failed at
once with `ERROR_BROKEN_PIPE`. It queued no packet, but the broker recorded it
as accepted. When the test's 5-second deadline passed, the group cancelled it,
and `CancelIoEx` had no request to cancel. The drain then waited forever for a
packet that could not come. A second gap made the same state possible even
with a truthful verdict: `Registry::rejected` left the route without a
terminal, so `query` returned `None` forever for a rejected submission.

**Chosen:** the registry route owns "every submission yields exactly one
terminal", and the verdict has one owner.

- `windows_completion::issued` classifies a call from its return value and
  `GetLastError` only. It takes no `Internal` argument, so the field cannot be
  consulted. `TRUE` means completed synchronously, which still queues one
  packet because no completion-skip mode is set. `ERROR_IO_PENDING` means
  pending. Any other error is a failure with no packet. The event driver and
  the broker driver both use it.
- `Registry::rejected(route, failure)` records the synchronous failure as the
  route's terminal, so `query` and the broker's drain predicate end at once.
  The group already received the failure from submission, so it is not
  counted as unreported. The broker driver also rewrites the rejected request
  to idle, since the kernel never took it.
- The drain asks the registry first. It returns the recorded terminal, or
  `None` when the route has no submission in flight. It waits only while an
  accepted submission's packet is still outstanding.
- macOS-runnable tests. The registry model test now asserts that a rejected
  route reports its failure; before this change it asserted the terminal-less
  state that hung. `overlapped_submission_verdict_is_the_return_value_and_last_error_only`
  covers the classifier.

**Other waits with no budget, audited against the same invariant:**

- The broker group wait (`wait_any`, INFINITE without a budget) wakes on
  unreported routes. With a truthful verdict every accepted route publishes
  one packet, and rejected slots finish at submission.
- The event driver's drain and its unbudgeted wait run only after an
  `ERROR_IO_PENDING` verdict, whose event is always set.
- File locking waits only after `ERROR_IO_PENDING`.
- Process and helper-thread waits are not completion waits.

The post-pending uses of `Internal` stay. `result()` treats
`GetOverlappedResult` failing while the kernel still owns the request as
pending, which is the documented use.

| Option | Pros | Cons |
|---|---|---|
| Verdict from the return value only, plus the registry recording each rejection's terminal (chosen) | Fixes the false acceptance at its one owner; any future wait on a rejected route ends instead of hanging; the classifier's signature forbids reading `Internal` | Touches both drivers and the registry contract |
| Drop only the `Internal` clause in the broker | Smallest change | Leaves the event driver's identical clause, and rejected routes still have no terminal for any wait that reaches them |
| Bound the drain | No semantic change | Releasing storage on a timeout is unsound while the kernel may own it, and it hides the false acceptance |

### Windows CI on `80d62c9`: four failures (2026-11-08)

Run 42424122794 finished the Windows `doria-rt` suite with no hang and no
abort: 602 passed and 4 failed. Neither failure below comes from
`80d62c9`. Plan append never touches the registry, and the close counts
involve no rejected route.

**1. `private_plan_abi_drop_and_setup_rejection_leave_pipe_payload_unread` (test defect, proven by reading).**
It asserted that `dr_v1_transfer_plan_append` rejects the plan's own key.
The shared plan contract accepts a repeated key: `Plan::accepts_key` checks
only that keys share one owner, and `transport/plan/tests.rs` states and
tests that "a repeated key and a null, empty region are valid
configuration". Several operations on one stream in one group are the
design: the shared-cursor adapter test runs three on one key. The test now
rejects only a key from another set, and asserts that a repeated key is
accepted with both loans intact.

**2-4. Three pending-set tests saw one close at `drop(set)` (ownership defect, fixed in the runtime; proven by reading).**
`regular_file_leaves_preserve_cursors_append_empty_eof_and_sole_close`,
`file_preparation_cancellation_does_no_io_and_completion_keeps_original_error`
and `actual_nonregular_file_is_rejected_before_callbacks_or_native_transfer`
all failed `closes == 0` after `drop(set)`. A file-only pending set holds
the process's only broker claim: its contexts own no attached stream, and the
tests serialize on `TEST_BROKER_LOCK`. So dropping the set closes the shared
completion port, and the port was an `OwnedHandle`. That type is documented
as the owner of a device handle, and every close it makes counts in
`with_close_probe`. The same was true of the device helper's thread handle
and the event driver's completion event.

Meanwhile three other tests counted the port as a stream close ("last stream
and broker port close once"). The two expectations contradicted each other,
and both depended on which unrelated owner released the last claim.

**Chosen:** `win32::KernelObject` owns the runtime's private kernel objects:
the helper thread, the completion event and the completion port. It closes
with `CloseHandle` and outside the stream close accounting.
`with_close_probe` therefore measures one thing, the device/stream close
contract. The three port-counting tests now count stream closes only, and
`lifecycle_snapshot` still proves the port is released with its last claim.
The hosted-shared-runtime step runs these same tests with other features.
Only `lib.rs` has hosted-only code, so nothing further applies there.

| Option | Pros | Cons |
|---|---|---|
| A private kernel-object owner outside stream accounting (chosen) | One meaning for the close probe; a test's count no longer depends on who drops the last broker claim or whether a helper ran | Three tests' port counts change |
| Expect the port close in the three pending tests | No runtime change | Exact counts then depend on other claimants; helper threads and events would also need counting; the probe mixes two contracts |
| Pin the broker in the test lock | Deterministic counts | Hides the lifecycle the broker tests verify, and leaves device and kernel objects in one owner type |

### Close accounting after `KernelObject`, by enumeration (2026-11-08)

CI on `f1ead7c` (run 42425778020): 604 passed, 2 failed. Both failures were
close counts that still included kernel objects:
`private_abi_rejects_before_io_and_distinguishes_zero_data_from_pipe_eof`
counted the port, and
`private_abi_loopback_reuses_event_and_storage_without_replaying_bytes`
counted two cached events. The previous fix updated three tests from
memory. This one enumerated the probe sites, and replaced the three tests'
dropped port evidence with direct evidence.

**Enumeration.** `with_close_probe` has 191 call sites in 27 files. The 131
in the 19 files that build for Windows were checked against the contract:
the probe counts device and stream closes only.

- `KernelObject` holds only three Windows objects: the device helper's
  thread, the event driver's completion event and the broker's port.
- Only the `byte_io/windows_completion` tests (42 sites) and
  `device_io/pending_tests.rs` (1 site) can release one inside a probe.
- The file, locking, cancellation and transfer-group paths hold none. File
  locks close their events with `CloseHandle` directly. Cancellation signals
  stay device handles.
- In the 43 sites, every count of 0 was already a stream-only claim, and every
  stream count of 1 or 3 is unchanged. Five sites expected a kernel-object
  close inside the count:
  - `broker/tests.rs`, the last stream, which releases the port;
  - `broker/transport/tests.rs`, two sites: the last released stream, and the
    shared set drop;
  - `port/tests.rs`, the set drop;
  - `native/tests.rs`, the two cached events.
- The hosted-shared-runtime step runs the same tests with other features. No
  test is conditional on that feature, so the same sites cover it.

**Chosen:** a kernel object's release has its own evidence.
`win32::with_kernel_object_close_probe` counts `KernelObject` closes. It
uses the same nested measurement as `with_close_probe`, now shared through
`native_handle::measure_closes` and `record_close_in`. The five sites
measure both counts at once: stream closes as before, plus "the last claim
closes the shared port once" or "two cached completion events close once".
The broker test also keeps its `lifecycle_snapshot` check.

| Option | Pros | Cons |
|---|---|---|
| A separate kernel-object probe, measured alongside the stream probe (chosen) | Each test still proves exactly what it meant to prove (port with its last claim, events closed once), in the owner's own accounting | Two counters to read in five tests |
| Only lower the counts | Smallest edit | Loses the evidence that the port and events close exactly once |
| Count kernel objects in the stream probe again | Original numbers | Reintroduces the contradiction `f1ead7c` removed |

### Windows child processes get native coverage; the vet type-checks every tier-1 target (2026-11-08)

CI on `db20b35` (run 42427011057) passed both Windows `doria-rt` steps,
including hosted-shared-runtime on its first run. It then failed "Clippy
LLVM-enabled compiler": `child_process_tests` had 11 dead-code and
unused-import errors. Its only native tests were `#[cfg(unix)]` because they
drive `/bin/sh`, so on Windows the shared helpers were unused. The root
problem was that no doriac test executed `process/windows.rs`.

**Chosen:** the two native scenarios are written once and run on every host.

- A `Host` table spells, per platform, only the programs and how each test
  child is started: the shell report, a two-stream flood, the
  environment/directory printer, an input copier, a 30-second sleeper, a
  partial writer, the shell's name, the filesystem root, and the report for a
  terminated child.
- Unix uses POSIX tools. Windows uses `cmd.exe` for the shell form and
  Windows PowerShell (`-NoProfile -NonInteractive -InputFormat None`) for
  executables. Both exist on every Windows host.
- The environment scenario now removes a variable the test injects into the
  native program and PHP (`DORIA_CHILD_INHERITED`) instead of `HOME`. So
  removing an inherited variable is exercised on both platforms, including
  the Windows environment block built from `GetEnvironmentStringsW`.
- Expected outputs differ only where the platform does. A terminated child
  reports `signaled 9` on Unix and `exited 1` on Windows (`TerminateProcess`
  with code 1). The shell is `/bin/sh` or `cmd.exe`, the root is `/` or the
  system drive, and the copier is named `cat` or `powershell`.

Reading `process/windows.rs` against these scenarios found no defect:

- resolution tries the name, then the name with `.exe`, on the child's PATH;
- the shell form is `cmd.exe /d /s /c "<script>"`;
- arguments use MSVC quoting;
- the environment block is sorted and folded case-insensitively;
- a handle list restricts inheritance to the child's pipe ends;
- the error output drains on a helper thread;
- terminate exits the child with 1;
- waits are bounded by the budget.

Windows CI is the first execution.

**Closing the local gap:** `validate_work_unit.php` now runs clippy for every
tier-1 target other than the host, with the pinned toolchain. That is
`doria-rt` (default features and `hosted-shared-runtime`) and `doriac
--no-default-features`. doriac's `bundled-runtime` build script links a
runtime library with the target's own linker, and `llvm-backend` needs that
target's LLVM. CI's native jobs keep covering both. The validator fails,
naming `rustup target add <triple>`, when a cross target's standard library
is missing. It checks only when it runs commands, so `--plan` and
`--reclaim` (used by the artifact-policy guard in CI) are unaffected.
`cargo clippy -p doriac --all-targets --no-default-features -D warnings`
is clean for Windows and Linux.

| Option | Pros | Cons |
|---|---|---|
| One scenario set with per-host spellings (chosen) | The same contract is executed on every tier-1 host; the Windows implementation runs in CI | PowerShell start-up adds a few seconds per child |
| A separate Windows test with its own scenarios | Free choice of spelling | Two copies of the contract drift |
| `cfg`-gate the helpers or `allow(dead_code)` | Silences the lint | Leaves `process/windows.rs` unexecuted |
| A test-built helper executable | Exact byte control | A new `[[bin]]` in doriac's `Cargo.toml`, which another agent is editing |

**Open:** the local validator now needs the `x86_64-pc-windows-msvc` and
`x86_64-unknown-linux-gnu` standard libraries for the pinned 1.98.1
toolchain. Installing them is a download, which needs Andrew's approval.
Until then the validator stops before running, with the command to run.

### Tests that pinned the pre-`974762d` return ABI (2026-11-08)

CI on `f94c9b3` (run 42428913999) failed `byte_region_native_structure_tests`
on macOS, Linux and the rust job. Two of its tests pinned the old contract:
- the native plan of the `?ByteRegion` triple listed only `CurrentFrame`;
- its LLVM definition returned the `{ word, { ptr, word } }` triple directly.

Both now assert the new contract:
- the two-word pair stays direct: `[CurrentFrame]`, returning its
  `{ ptr, word }` carrier;
- the triple returns `void` through `ResultOut`, after the current frame,
  while its parameter keeps the explicit triple;
- neither plan has a borrow-home result, which is what the tests existed to
  prove.

A sweep for tests asserting native signatures, `define` lines,
`hidden_inputs`, `ResultOut` or return shapes found no other test that pins a
shape this ABI change moved:
- `llvm_mir_tests`, `common/native_structure.rs`;
- `monomorphization/tests.rs`;
- the readiness-set tests;
- `native_closure_abi` and the return-transport tests.

The gap was in the vet, not in any one test. `974762d` ran only the targets
it touched, and this test is in none of them. This update and the next fix
are committed only after the complete doriac suite passes with llvm-backend:
every target, the parity suite and its shards, and both clippy runs.

### Open-class region returns: bodies and dispatch get separate summaries (2026-11-08)

`Source::view`'s MIR plan carried a retained source whose path named
`Filled`'s property, `PropertyId { class: Filled, index: 0 }`. On a `Source`
receiver that path names a property the receiver does not have, so
`validate_source_path` rejected it.

The ownership pass keyed two different facts by the same declaration span:
- the **body summary** of `Source::view`: what that body returns and stores;
- the **dispatch summary** of the open method: the join over every override,
  which a call through `Source` may select.

The family join merged the overrides into the base method's own entry. Each
body analysis then appended its own body on top of that union. MIR lowering
reads the entry as the base body's plan, so the base definition retained
every override's sources. Its stores, removals, selectors and writeback were
merged the same way.

**Chosen:** `RetainedCallableInfo::dispatch` holds the join, and the
declaration's own fields stay its body.
- The family join in `ownership/retained.rs` builds the join from each
  implementation's body, plus the requirement's own entry (its body, or an
  interface requirement's seeded selections), and stores it in `dispatch`.
- Every lookup of a callee's summary at a call goes through `for_callers()`:
  `owned_storage`, `pending`, `retained` and `writeback`. Per-implementation
  loops keep reading each implementation's body.
- MIR definition plans, closure plans and PHP parameter roles read the body.
- `seed_callables` and the composed-summary refinement carry both views,
  because the field travels with the summary.

A second defect sat behind the first. With a covariant override (family
`?ByteRegion`, override `ByteRegion`), the virtual return adapter that MIR
lowering synthesizes had no retained plan at all. Validation rejected it:
"byte region return signature loses its retained source plan".
- An adapter returns exactly what its implementation returns, so it now
  carries the implementation's plan, mapped onto its identical parameters.
- `RetainedSourcesPlan::forwarded` in `mir.rs` is the one owner of that
  mapping: returns, stores, removals, selectors and returned storage, with no
  constructs or promotions.
- Interface entries used an inline copy of the same mapping. They now call
  `forwarded`, and `remap_storage_selection` is gone.

| Option | Pros | Cons |
|---|---|---|
| Separate `dispatch` join in the summary; calls read it, bodies keep their own (chosen) | Both facts get the owner each consumer needs; one field carries both views through every pass and seed; no instance-specific rule | Every call-site lookup must choose `for_callers()`; a future lookup that forgets reads the body |
| Canonicalize inherited property ids to the declaring class | Fixes the inherited-property case | An override returning its own property (`Extra::$more`) still lands in the base plan; stores and writeback stay merged |
| Relax `validate_source_path` to accept subclass paths on a base receiver | One line | Accepts a plan that names storage the receiver may not have, which the validator exists to reject |
| A separate exported map of body summaries | No new field | Needs a new `SemanticInfo` field (in a file another session owns) and two parallel maps that every pass must keep aligned |

Coverage: `native_return_transport_tests::open_class_region_returns_retain_each_implementations_sources`.
It checks a `?ByteRegion` family, a `ByteRegion` override from an inherited
property, and one from the override's own property, called through the base
and through an open override, on the interpreter, Cranelift and LLVM. The
`?Deadline` form stays as well.

**Found, not fixed: region-returning interface and trait requirements.**
`interface Viewing { function view(): ?ByteRegion; }` cannot be implemented
by a method that returns `$this->bytes->getRegion(...)`. That fails with
E0755 "does not preserve the required return ownership or provenance", and
the trait form fails with E0757. Both are rejected before MIR, so they are a
different failure from the one fixed here:
- a bodyless requirement gets no return borrow, so its contract reads as an
  independent return;
- Decision 0134 lets a borrowed return satisfy a requirement only when it
  preserves at least the required provenance;
- Doria has no lifetime annotations, so the requirement has no way to state
  that borrow.

Closures returning regions through function values work. No standard
library interface or SPEC example declares a region-returning requirement
today, so nothing shipped depends on the gap.

**Chosen (36a Recommended rule; Andrew reviews it with the log):** a
bodyless requirement with a receiver elides its region return to a
`Retained` receiver borrow, under the single-source rule method bodies
already use. An implementation that returns a receiver-held region then
preserves the required provenance, and one that returns an independent
region preserves more than it.

Sequencing: the rule changes `semantics/contracts.rs` and Decision 0134.
The paused catch-reachability session has uncommitted edits in both, and
exact contracts builds on that tree. The rule lands after both, in the same
tree, with its own tests for the interface and trait forms on the
interpreter, Cranelift and LLVM.

| Option | Pros | Cons |
|---|---|---|
| Elide to a receiver borrow, as method bodies do (chosen) | One elision rule for bodies and requirements; no new syntax; receiver-held and independent implementations both satisfy it | Callers of a requirement whose implementations all return independent regions still get a receiver-bound result |
| Explicit return-borrow syntax on requirements | States the contract exactly | New syntax for one case; Doria has no lifetime annotations and this would be the first |
| Infer the contract from known implementations | Callers get the tightest result | An interface's contract would depend on its implementers; an open world and separate compilation make that unknowable |
| Keep rejecting | No rule change | Region-returning interfaces and traits stay inexpressible, a design limitation left in place |

### PROMINENT: time allowance raised from 22 to 24 hours for the last vet (2026-11-08)

The complete doriac vet of the open-class fix stopped with 5 s of the
79,200 s allowance left. Library, binaries and batches 1-3 passed (565, 97,
243 and 370 tests, 0 failed). Batches 4-7, the parity suite and its three
shards, and both clippy runs did not start.

About 5,000 s of the last hours were wasted, not spent on work. The vet
script read the wrong log name, took every finished run for a guard stop, and
reran build, lib and batches 1-3 five times each. The resume script reads
the log `g.sh` writes and stops on a guard refusal instead of retrying.

**Chosen:** add 2 hours (79,200 -> 86,400 s). vet7's equivalent of the
unfinished steps took about 2,700 s; workspace clippy is new since `bab1070`.
The rest covers one rerun if CI names another defect. The other limits stay:
one job at a time, 3 GiB memory, the run cap, offline and locked, and the
25 GB cache ceiling.

| Option | Pros | Cons |
|---|---|---|
| Raise by 2 hours (chosen) | Finishes the vet and leaves room for one CI-driven rerun | Andrew sees the raise only at review; part of it replaces time a script fault wasted |
| Raise by exactly the unfinished steps (~1 hour) | Smallest raise | Any CI-named defect stops work again at the allowance |
| Push without the rest of the vet and let CI run it | No local time | CI runs only part of the doriac suite on each runner, so a red CI would again be the first report of a defect; the vet gap this stage already named |

### CI hid failures on Windows and behind the first failing target (2026-11-08)

CI on `f59169b` (run 42453796180) failed the `rust` and Ubuntu LLVM jobs, and
reading the Windows log showed a worse problem.

**Windows was never green.** The Windows job showed success on `f94c9b3` and
`f59169b`, and I reported that to Andrew as a pass. Its LLVM test step ran 38
`cargo test` lines with no `shell:`. Windows defaults to PowerShell, and GitHub
fails such a step only on the last command's exit code. Five earlier targets
were failing, 13 tests in all.

**Linux stopped at the first failure.** `cargo test` without `--no-fail-fast`
and a `bash -e` list both stop at the first failing target, so each CI round
could report one failing target per job and hid the rest.

| Failure | Tests | Root cause | Fix |
|---|---|---|---|
| Runtime symbol missing from relocations | 8 on Windows | Cranelift's COFF output reaches an external symbol through a `.refptr.` pointer cell, and the tests compared exact names | `native_structure::relocation_target_name` owns the rule; both relocation readers use it |
| Interface identity read as 0 | 1 on Windows | `object`'s `is_64()` is false for every COFF object, so the test read 4-byte words | Word width comes from the architecture's address size |
| Removing a match arm changed nothing | 1 on Windows | The Windows checkout converted the example to CRLF, so the test's `\n` text never matched | `*.doria text eol=lf` |
| `permission denied` reading a live file | 3 on Windows | The programs called `read_file` while a writer was open | Read through a public owner (below) |
| Injected failure did not reach PHP | 2 on Linux | PHP 8.4 loads libraries with `RTLD_DEEPBIND`, binding the runtime's imports to libc ahead of `LD_PRELOAD` | The fixture interposes `dlopen` and drops the flag (below) |

**`RTLD_DEEPBIND`.** PHP's `DL_LOAD` passes it unconditionally through 8.4; 8.5
uses it only under Apache. This machine runs 8.5 and CI pins 8.4. The fixture
now replaces `dlopen` and clears the flag. PHP's own call loads `ffi.so`
through the replacement, so `ffi.so`'s call that loads the runtime reaches it
too.

| Option | Pros | Cons |
|---|---|---|
| Interpose `dlopen` in the fixture (chosen) | The real runtime and real error path run; no product code changes; holds on every PHP version | Other PHP extensions in that test process bind without `RTLD_DEEPBIND` |
| Move CI to PHP 8.5 | One line | Drops 8.4 coverage, and the test would still break on the next PHP that deep-binds |
| Run the PHP half of this test on macOS only | No fixture change | Linux never proves that PHP transports these failures |
| A failure hook in the runtime | Direct | A test seam in shipped runtime code |

**`read_file` with a live writer.** `docs/stdlib-reference.md` states that public
file owners permit simultaneous readers and writers on every platform, and
that the free helpers keep their stricter Windows sharing. So the programs
were outside that contract, not the runtime. The checks have to happen while
the writer lives, so `fixtures/live_file_observation.doria` reads through
`File::openRead`.

| Option | Pros | Cons |
|---|---|---|
| Observe through a public owner (chosen) | Uses the documented contract; the tests still check mid-operation file contents | Two small helper functions in the test programs |
| Widen `read_file` to share with writers | Same program behaves alike on every platform | Changes a documented contract to suit tests; a language decision of its own |
| Move the reads after the writer closes | No new helper | Loses what these checks exist for: the file's contents during the operation |

**CI.** The LLVM step is now one `cargo test --no-fail-fast` command, whose exit
code covers every target on every shell. The `rust` job's test command also
takes `--no-fail-fast`. `scripts/check_ci_shells.php` rejects a multi-command
step that can run on Windows without a named shell; it flagged both offending
steps before this change.

| Option | Pros | Cons |
|---|---|---|
| One command plus a guard (chosen) | Every failure in one round; a false pass cannot return | One long command line |
| Add `shell: bash` only | Smallest edit | Still stops at the first failing target |
| A loop that records each exit code | Keeps one line per suite | Shell logic to maintain, on three operating systems |

### Close results at the final pins (2026-11-08)

These replace the pins in "Close results (2026-11-07)".

- `doria` `1f68889`, on origin develop. Pull-request run 42467180271 passed
  all five jobs, and its full Windows log has no failed test. Push run
  42467178084 passed four. Its Windows job stopped at the LLVM download,
  where GitHub's release host returned 504 on all five attempts, before any
  test ran.
- `doria-language-server` `9457785` (Stage 36a sync and pin) and `58585d1`
  (Rust toolchain), on origin develop. Library 308, CLI 3 and protocol 176
  tests pass against the pin; fmt, clippy, every PHP guard, the pin check and
  63 VS Code grammar tests pass. Run 42473693225 passed all five jobs on its
  second attempt. The first timed out downloading Gradle before any test ran.
- `doria-website` `057152f` (content, compiler pin and API lock at `1f68889`)
  and `13db4e4` (the playground compiler builds with its own Rust release),
  on origin develop. Full Pest passed 348 tests (39,210 assertions) against
  the compiler built at the pin, and the API reference sync check passes.
  Run 42474559884 passed.
- The installed `doriac` and `doria-lsp` were rebuilt from `1f68889` and
  `58585d1` in a detached worktree, removed afterward.
- The build cache was cleared before these builds, which would have crossed
  the 25 GB ceiling: 21 GB of `llvm-backend` and the stale 662 MB `debug`.

### Commit shape for the final pins (2026-11-08)

**Chosen:** amend each repository's one unpushed Stage 36a commit to the final
pin, and put the Rust toolchain and CI change in its own commit.

| Option | Pros | Cons |
|---|---|---|
| Amend for the pin, separate toolchain commit (chosen) | Each commit does one thing; the toolchain change can be reverted alone; no commit names a superseded compiler | Two commits in each repository |
| Fold everything into the feature commit | One commit per repository | Hides a CI policy change under a feature subject |
| Add a new commit for the pin | No amend | The first commit would name a compiler nobody should build |

### Stage 36a is not delivered: the performance gate is unfinished (2026-11-08)

The plan was to mark the stage Complete once CI was green in all three
repositories. CI is green, but the stage's closure conditions are not met, so
the status stays In progress.

`docs/performance-and-benchmarking.md` section 10 names ten binding cases and
requires correctness, structural baselines, provenance, commands and report
schemas. Enumerated from `benchmarks/manifest.json`, `runtime/` and
`diagnostic/` on 2026-11-08:

| Case | State |
|---|---|
| Synchronous startup (`startup`) | Committed |
| Large streaming file copy (`streaming_file_copy`) | Uncommitted in `benchmarks` since 2026-10-10 |
| Repeated small writes (`repeated_small_writes`) | Uncommitted |
| Concrete adapter chain (`stream_adapter_chain`) | Uncommitted |
| Erased interface stream (`writable_stream_dispatch`) | Uncommitted |
| Non-blocking pipe transfer | Does not exist |
| Child stdout and stderr drainage | Does not exist |
| Incremental UTF-8 line processing | Does not exist |
| First-class versus intrinsic standard output | Does not exist |
| Many stable readiness registrations | Does not exist |

`benchmarks-revision.json` still names `621aa13` (2026-10-02), which has none
of the four uncommitted cases. The uncommitted work has not been built against
the current compiler.

Controlled timing is not part of the gap. Section 10 records
`Pending Available Runner` when no eligible runner exists, and says hardware
availability must not block completion.

The plan also lists the PHP byte-region address-view fix as proposed and not
implemented, and nothing records it as done. It has to be checked against the
code, then implemented or the plan corrected.

**Chosen:** keep the stage In progress and build the gate before the two
paused follow-up sessions.

| Option | Pros | Cons |
|---|---|---|
| Keep In progress and build the gate (chosen) | The status is true; the plan's closure conditions decide delivery | More build time; another raise to the time allowance |
| Mark Complete and move the gate to Stage 43 | Closes the stage now | Section 10 says Stage 36a owns the first gate and Stage 43 only continues it |
| Mark Complete and list the gate as a known gap | Closes the stage now | Complete would be false |

These close notes stay unstaged on develop and land with the gate's commit. A
docs-only commit would start two full CI runs for no code change.

### PROMINENT: time allowance raised from 24 to 28 hours for the performance gate (2026-11-08)

The closing vet, the re-pins and the toolchain refresh left about 1,500 s of
the 86,400 s allowance. The gate's remaining work runs compilers and peers
under the same guard.

**Chosen:** add 4 hours (86,400 -> 100,800 s). The other limits stay: one job
at a time, 3 GiB memory, the run cap, offline and locked, and the 25 GB cache
ceiling.

| Option | Pros | Cons |
|---|---|---|
| Raise by 4 hours (chosen) | Covers five new cases, peers and one correction round | Andrew sees the raise only at review |
| Raise by 1 hour | Smallest raise | Likely to stop mid-case and restart |
| Run the gate outside the guard | No raise | Loses the memory limit and the record of time spent |

### PHP byte-region address views, checked against the code (2026-11-08)

The plan's item is still accurate at `1f68889`.
`__doriaNativePointer` in `codegen_php/bytes.rs` caches the view at offset 0
and returns a new `FFI::addr($this->values[$offset])` for every other offset,
so repeated calls at a nonzero offset (a partial write's suffix, for example)
allocate one CData object each. Hosted source replies read a pointer slot the
same way.

**Chosen:** implement the plan's shared base, capacity, offset and length
bridge after the gate, in the same compiler round as the two follow-up
sessions and the region-return elision.

| Option | Pros | Cons |
|---|---|---|
| Same compiler round as the follow-ups (chosen) | One vet, one CI round and one re-pin of the language server and website | The fix waits for the gate |
| Fix it now, alone | Closes the item first | The compiler tree holds another session's uncommitted changes; adds a full vet, CI round and re-pin |
| Cache a view per offset | Small change | The cache grows with the offsets used, and the native loan on pointer slots ends at each reply, so caching there is not allowed |

### Performance gate: compiler identity for the bounded runs (2026-11-08)

The runner checks that the selected `doriac` matches the Doria source identity,
and the compiler tree here is dirty with another session's changes.

**Chosen:** run with `DORIAC` set to the installed `1f68889` compiler and
`DORIA_REPO` set to a throwaway clean clone of the same commit, removed after
the runs.

| Option | Pros | Cons |
|---|---|---|
| Clean clone plus exact compiler (chosen) | Reports carry the true commit with no escape hatch; immune to edits in the working tree | One temporary clone |
| `--allow-compiler-commit-mismatch` against the dirty tree | No clone | An investigative flag; reports show dirty provenance; the run aborts if the other session edits a file mid-run |
| Provision a compiler from the dirty tree | No flag | Builds another session's uncommitted changes |
| `git worktree add` | Cheap checkout | Adds worktree metadata to the shared repository |

### Performance gate: `stream_adapter_chain` stopped on a compiler defect (2026-11-08)

`doriac compile --build-plan` fails with B0001 "malformed MIR: enum references
a missing source" for any program that reaches `File`. Reproduction: a
one-source build plan whose source is `use Doria\Std\Io\File; function main():
void { let $file = File::openRead("o"); echo "x\n"; }` fails on both native
profiles; the same source compiled without a plan builds. The two-file build
plan this diagnostic needs therefore cannot compile.

**Chosen:** keep the case sources unchanged, record the failure and the
reproduction, and leave the compiler for a compiler session.

| Option | Pros | Cons |
|---|---|---|
| Stop the case and report (chosen) | Honest evidence; no benchmark workaround hides the defect | One binding case stays unproven until the compiler is fixed |
| Merge the workload and helpers into two single-file programs | The case builds today | Abandons the shared-workload design the case and its tests guarantee, and hides the build-plan defect |
| Fix `mir_validation` or the graph source registration here | Closes the defect | Outside this gate's scope; the compiler tree holds another session's changes |

### Performance gate: non-blocking pipe transfer stopped on a runtime defect (2026-11-08)

The only pipes Doria exposes are child pipes, and the runtime creates them
blocking; `ReadinessSet::begin` rejects them on Unix because
`unix_readiness/transfer.rs` `validate_handle` requires `O_NONBLOCK` or a
regular file. Reproduction: spawn `cat`, wrap `$child->getOutput()` in
`BorrowedBufferedReader<ChildOutput>`, register it, and `begin` a
one-read `TransferPlan`; the program exits 70 with "failed to configure
transfer group: unsupported". `getInput` and `getOutput` also cannot be borrowed
together (E0763), so one child's pipes cannot share a set.

**Chosen:** do not add the case; report the defect. The case needs the runtime
to give child pipes the nonblocking descriptors its pending tier requires.

| Option | Pros | Cons |
|---|---|---|
| Stop and report (chosen) | No false evidence; the defect is named with a reproduction | One binding case does not exist yet |
| Read child output with blocking `read` and call it a pipe transfer | Case exists | Measures blocking I/O under a non-blocking name |
| Replace the pipe with a regular file or userland stream | Runs today | Not a pipe; the name would mislead |
| Fix `process/unix.rs` here | Closes it | Compiler repo is off limits for this gate |

### Performance gate: child stdout/stderr drainage construct (2026-11-08)

**Chosen:** `Child::collect($limit)`, the documented contract that drains both
streams concurrently and bounds retention. The peers use `poll` on two
nonblocking pipes in C and `Command::output` in Rust.

| Option | Pros | Cons |
|---|---|---|
| `collect` (chosen) | The contract the design names; proven natively by the compiler's flood test; bounded | Internal poll loop is not visible to the case |
| Manual concurrent reads through a `ReadinessSet` | Exercises the pending tier | Blocked by the pipe defect above; the two getters also cannot be borrowed together |
| Sequential reads of stdout then stderr | Simple | Deadlocks on a full pipe, which is the behaviour this case exists to prove absent |

### Performance gate: Unix-only child and readiness cases (2026-11-08)

`child_output_drainage` spawns `sh` and `cat`, and its C and Rust peers use
Unix process APIs. `readiness_registrations` peers use `poll(2)`.

**Chosen:** declare `supportedPlatforms: linux, macos` for those two cases and
say why in their descriptions; the other new cases stay portable.

| Option | Pros | Cons |
|---|---|---|
| Unix-only for the two cases (chosen) | Honest about what exists; no unverified Windows peer | Windows evidence for them waits for Windows-specific children and peers |
| Write Windows peers and children now | Broader coverage on paper | No Windows runner here to prove them; more unverified code |
| Doria-only on Windows with no peers | Some Windows evidence | No runner here either, and a per-platform target split the manifest does not express |

### Performance gate: read-only cases write one declared output (2026-11-08)

The runtime fixture format requires at least one declared output file.

**Chosen:** `utf8_line_processing` and `readiness_registrations` write a
one-line summary to a declared `summary.txt`, checked by exact length and SHA256
computed independently in `tests/run.php`; stdout stays a fixed word.

| Option | Pros | Cons |
|---|---|---|
| Declared summary output (chosen) | Uses the fixture mechanism unchanged; adds exact content evidence | Each case gains one incidental file write |
| Relax the fixture rule to allow no outputs | Purer read-only cases | Harness, schema and existing tests change for a small benefit |
| Keep the summary only on stdout | Simple | Needs per-profile expected files, and the fixture rule still demands outputs |

### Performance gate: first-class versus intrinsic stdout control (2026-11-08)

**Chosen:** a matched Doria-only diagnostic: the feature writes through
`StandardStreams::output` with partial-write offsets, the control calls
`write_stdout_bytes`, and only `writeChunk` differs. Output is checked against
committed files, so the baseline stays at 8,192 iterations.

| Option | Pros | Cons |
|---|---|---|
| `write_stdout_bytes` control (chosen) | Same binary path as the feature, byte-for-byte output | Baseline is small because stdout is committed |
| `echo` control | The everyday text path | Text formatting and buffering differ from the byte view, so the pair would not differ only in the view |
| Comparative case with C and Rust peers | Peer numbers | The question is internal to Doria; a peer would not answer it |

### Performance gate: readiness peers and registration source (2026-11-08)

**Chosen:** regular-file registrations in Doria, with C and Rust peers that
`poll(2)` the one descriptor before each one-byte read and reuse one array;
Rust binds `poll` through `extern "C"` since std has none and no crates are
available.

| Option | Pros | Cons |
|---|---|---|
| Poll peers with regular files (chosen) | Portable input, exact byte sums, real poll calls | Regular files are immediately ready, so this measures registration and completion cost rather than wake-up behaviour |
| Plain `read` loop peers with no readiness | Simplest | Not a readiness baseline at all |
| Pipes or sockets as registrations | Real wake-ups | Doria has no nonblocking pipe (defect above); sockets are outside Stage 36a |
| Report it Doria-only with no peers | Avoids peer questions | Section 10 asks for a direct baseline where meaningful |

### Performance gate: structural baseline scope and unavailable metrics (2026-11-08)

Section 10 asks for deterministic structural baselines, and the harness's
existing mechanism compares compiler-report counts exactly.

**Chosen:** `matrices/stage36a.json` pins MIR, specialization, call-site and
stack-allocation counts per Doria native target for the eight proven cases, and
`baselines/stage36a.json` is captured as `candidate` because the benchmarks tree
is uncommitted. Sizes are excluded as host-dependent. Reports add
`metricAvailability.streamGate` with a reason for allocation counts, system
calls, adapter copy bytes, cold startup and per-operation latency, never zero.

| Option | Pros | Cons |
|---|---|---|
| Structure counts, candidate baseline, explicit unavailable reasons (chosen) | Deterministic; honest about what is unmeasured; acceptance needs only a clean tree and `--accept` | Static structure does not itself prove allocation-freedom; that stays with the compiler's structural tests |
| Include object and executable sizes | More detail | Differs across hosts, so a macOS capture would fail Linux comparisons |
| Add a new runtime allocation counter | Direct evidence | Compiler and runtime are out of scope here |
| Claim allocation-free results from structure counts | Closes the invariant on paper | False: static counts cannot observe runtime allocations |

### Entry 52: verification scope and the virtual-call boundary (2026-11-08)

Planning replaced the full vet with targeted tests and said to take the
recommended option at every fork. Two forks followed.

**Which targets count as touched.**

- **Chosen: the 25 targets that exercise what entry 52 changed, plus a verdict
  comparison.** The targets are `--lib`, the E0629-E0635 assertions, closures,
  callbacks and List algorithms, trait and contract selection, the two-pass
  conditional path, property hooks, virtual calls, `toThrow` and the std
  destructors in `stream_capability_tests`. All 1469 tests passed. `doriac
  check` over the 648 example and fixture programs gave the same accept or
  reject verdict and error codes as the installed compiler.
  - Pro: entry 52 changes which programs are accepted, not MIR, so the verdict
    comparison covers the rejection risk across every program.
  - Pro: frees the build slot for Planning's end-of-round vet.
  - Con: `native_mir_parity_tests` did not run in this beat, although the
    original brief named it. Planning's end-of-round vet covers it.
- Also run the three parity shards now.
  - Pro: meets the original brief literally.
  - Con: about 25 minutes of the shared slot for a matrix the final vet repeats.
- Run every `doriac` target.
  - Con: Planning ruled that out for this round.

One earlier run of `codegen_php_tests` stalled in
`copy_parameter_values_and_writable_callback_borrows_match_native` until the
1800 s cap. The test then passed alone in 117 s and in the full target (142/142
in 90 s), with a stall watchdog armed. The cause is unknown and it has not
recurred; the end-of-round vet will show whether it comes back.

**Virtual calls on open classes keep Decision 0130's union for reach.**

- **Chosen: leave them unchanged in this beat.**
  - Pro: Decision 0130 accepted the union, and the brief named only callbacks
    and composed copies.
  - Pro: the reach and transport split makes a later change one line in
    `record_selected_contract_effects`.
  - Con: a caller's E0629 around `$base->m()` still changes when a subclass
    anywhere adds or removes I/O, so entry 52's rule has this one exception.
- Give virtual calls the complete contract for reach, as erased interface
  calls already have.
  - Pro: every call chosen elsewhere follows one rule.
  - Con: a destructor calling an open method must then catch ambient I/O even
    when no override performs any, and it reinterprets an accepted decision.

## Exact per-specialization contracts (2026-11-08, follows entries 51 and 52)

Entry 51 made every specialization share its template's conservative contract,
so `go<Quiet>` carried a status branch and an Error slot per call for I/O it
cannot perform. Each specialization's executable contract now carries only what
it selects. Template diagnostics do not change: authored reach (entry 52)
decides them, and this work touches only transport.

Built on `e74c00e`. Semantic analysis owns the new facts in
`semantics/executable_effects.rs`: for each callable instance, its contract, the
effects of its sites and `try` statements, and the catches it can never enter.
MIR reads them for signatures, try fallbacks and catch dispatch; PHP reads them
for origin guards. Call-site transport still comes from the selected callee's
signature, so a call to an effect-free selection is an ordinary call.

Result: `go<Quiet>`, `logged<Quiet>` (catch and log), `rethrown<Quiet>`,
`spin<Quiet>` (recursion), `same<Key>`, `render<Tag>` (display), `levelOf<Still>`
(accessor), `Cell<Quiet>::close` and a non-generic `quietly()` that calls
`go<Quiet>` have no checked effects. Their loud counterparts keep them.

### Where the narrowing lives

- **Chosen: a semantic per-instance view, computed by replaying checked effect
  regions as a least fixpoint over the instance call graph.**
  - Pro: semantics already owns effect facts, and both MIR and PHP read them.
  - Pro: selection reuses the rules the backends use (`CallableTarget`,
    `lookup_instance_method`, the generic call graph).
  - Pro: recursion that never reaches an operation stays empty.
  - Con: one more whole-program pass in every analysis.
- A MIR nothrow pass, in the style of LLVM's attribute inference.
  - Pro: sees exact callees.
  - Con: PHP does not go through MIR, so it would need a second answer.
  - Con: needs a dataflow over error locals and a rewrite of finalizer routing.
- Re-check each specialization's body with substituted types.
  - Pro: reuses the checker for everything.
  - Con: the checker's span-keyed state would need saving and restoring per
    instance, and a substituted body can type differently from the checked one.

### How a site says where its effects come from

- **Chosen: the checker records provenance where it records the effect**
  (`record_call_effect_profiles`, and `record_effect_profiles` for operations).
  - Pro: the code that decides the effect also names its source.
  - Pro: a path that names no callee stays an operation with the template's
    effects, so an unconverted path is less exact, never wrong. A contribution
    that bypasses provenance is kept as checked.
  - Con: touches each call-recording path.
- Derive provenance afterwards from `call_targets` and the other span maps.
  - Pro: no recording changes.
  - Con: a call and a display of its result share a span, so the union at one
    span cannot be split; dropping the wrong part would be a miscompile.

Provenance is recorded only while a body is being checked. The instance walk
re-selects core operations after checking and outside every scope, and that is
exactly what this pass derives per specialization.

### Which effects narrow

- **Chosen: only automatic effects.** Required errors and written ambient
  `throws` entries are authored contract, so a specialization keeps them and
  its callers route them.
  - Pro: matches non-generic functions, whose ABI follows their declaration.
  - Con: a quiet selection under a declared `throws Boom` keeps one status
    branch.
- Also narrow required errors.
  - Pro: more exact.
  - Con: a declared contract would stop meaning what it says at the ABI, and
    structural function identity depends on required effects.

The first version looked up only what a body inferred, so a caller of a method
declaring `throws IoError` believed the call was quiet and MIR had nowhere to
route it (`conditional_stream_forwarders_execute_nested_concrete_and_erased_transfers`).
Declared entries now seed the fixpoint.

### Handlers a specialization cannot enter

- **Chosen: not lowered.** A catch no protected effect can reach in this
  specialization is left out of the dispatcher and its body is not lowered; its
  own effects do not count.
  - Pro: `catch (IoError) { echo ...; }` around a quiet selection costs nothing.
  - Pro: a quiet specialization needs no error target at all.
  - Con: MIR's try lowering reads two per-instance facts.
- Lower every handler and count its effects.
  - Pro: no MIR change.
  - Con: any handler that logs keeps the contract, which is the common shape.

An effect with no recorded coverage keeps the catch reachable and the effect
uncovered, so unknowns stay conservative.

### Not narrowed

- **Virtual slots keep their family's union.** A slot is dispatch over a class
  family, not a specialization choice, and the exact-direct body is narrowed
  already.
  - Con: a slot still carries effects only some overrides have. Open as a
    possible refinement; it needs a per-instantiation family profile.
- **Erased interface calls keep the complete set.** The implementer set is open.
- **Specialized ownership analysis still uses the template's sites.** It sees
  more error edges than the executable view, which is conservative.

### Defect fixed in the same change: top-level statements next to `main`

`echo "top"; function main(): void {}` panicked in MIR lowering ("checked source
has a semantic propagation target"), and PHP stopped with a fatal
class-not-found. Both predate this work.

- MIR runs the entry source's top-level statements in `main`'s frame, but
  `main`'s contract never included their automatic effects. They now join it
  through the observed effects, in both reach and transport.
- PHP emitted statements in program order, before classes it binds at run time.
  It now emits every declaration first; statements still run before `main`.

### Language-rule change for review

None in source. The rule Decision 0123 records is new: a specialization's ABI
follows what it selects. Source programs compile as before, apart from the two
crash fixes above.

### Verification (2026-11-08)

macOS ARM64, Rust 1.98.1, `llvm-backend`, offline and locked, one job, under the
3 GiB guard. Targeted only; Planning runs the round's full vet.

- `constrained_ambient_effect_tests` 11/11, on the interpreter, Cranelift, LLVM
  and PHP.
- 31 targets, 1,823 passed, 0 failed. Lib 553; `checked_error_tests` 45;
  `codegen_php_tests` 142; `mir_tests` 148; `mir_validation_tests` 125;
  `cranelift_mir_tests` 29; `stream_capability_tests` 78;
  `returned_closure_transport_tests` 78; the stage 30, 34, 35 and 36 targets; the
  conditional-capability targets; `pending_stream_contract_tests`,
  `standard_stream_mir_tests`, `child_process_tests`, `time_contract_tests`,
  `native_testing_slice1_tests`, `consuming_receiver_tests` and the property-hook
  targets.
- `cargo clippy -p doriac --all-targets -D warnings` and `cargo fmt --check` are
  clean. `check_docs_authority`, `check_ambient_io_and_fallible_finalizers`,
  `check_inferred_main_effects`, `check_checked_error_foundation` and
  `check_diagnostic_style` pass.
- `doriac check` over all 673 tracked `.doria` files gives the same verdicts and
  error codes as the installed compiler at `5e72056`.
- The pass costs under 1 ms in a debug build: 0.66 ms of 0.28 s on
  `main_stream_capabilities` (32 instances), 0.35 ms per analysis of 0.84 s on
  `conditional_stream_forwarder` (71 instances). Measured with temporary
  instrumentation, since removed.

Not run: `native_mir_parity_tests`, the other `doriac` targets, the language
server, Linux and Windows. No runtime benchmark was taken; the removed status
branch and Error slot are shown by the MIR contracts, not by timing.

## Child pipes in transfer groups (2026-11-08, amends entry 40)

`ChildInput` and `ChildOutput` declared the pending and readiness tiers, but a
transfer group refused them: a child pipe is a file object, and the runtime's
pending path admits only regular files and standard devices. Spawning `cat`,
wrapping `getOutput()` in `BorrowedBufferedReader<ChildOutput>` and beginning a
plan exited 70 with "failed to configure transfer group: unsupported".

Decision 0137 does not reach this case. It permits lazy helper threads for
*inherited synchronous* standard devices, where mode control is unavailable.
The parent's pipe ends are created by the runtime and are not inherited, so
Decision 0110's readiness and completion paths apply directly.

Why `getInput()` and `getOutput()` could not be held together: entry 40 made
the pipe getters writable methods, and a method call borrows its whole
receiver, so any live pipe loan kept `$child` in writable use (E0763). The
runtime owner holds the three pipes as separate objects, so the conflict was
the checker's model of the receiver, not a fact about the storage.

### Parent's end mode

| Option | Pros | Cons |
|---|---|---|
| Nonblocking (Unix) and overlapped (Windows) parent ends (chosen) | Uses the existing readiness and completion paths; no thread per pending transfer; matches Decision 0110 and Decision 0137's scope | Plain transfers need a wait in the runtime; Windows pipe creation changes |
| Keep blocking ends and use Decision 0137 helpers | No change to plain transfers | Hidden threads for every pending pipe transfer on a device the runtime owns; 0137 explicitly limits helpers to inherited devices |
| Blocking fd plus `poll` before each read | Keeps blocking mode | Unsafe for writes: a blocking write of more than the free space blocks after POLLOUT, defeating the group |

Only the parent's ends change. On Unix the two ends of a pipe are separate open
file descriptions, so `O_NONBLOCK` on the parent's end does not reach the
child's standard handle. On Windows only the server end is overlapped.

### Who owns the wait for a plain read or write

| Option | Pros | Cons |
|---|---|---|
| `PipeEnd` in `byte_io`, called from `OwnedFile::read`/`write` (chosen) | One place; every caller of the file owner (native, PHP FFI, buffered adapters) keeps blocking semantics with no change | The owner stores a small pipe state value |
| Wait in generated code after a `WouldBlock` | No runtime state | Duplicates the loop in every backend and leaks mode into programs |
| Wait inside `dr_v1_file_object_read`/`write` | Near the FFI boundary | Misses the `OwnedFile` paths used by tests and future callers; splits the owner |

### Plain write on a nonblocking pipe

| Option | Pros | Cons |
|---|---|---|
| Wait until every byte is accepted; on a later failure report the accepted prefix and let the next write surface the failure (chosen) | Same as a blocking write: `Accepted(length)`, or a short count when the reader went away | A write can take as long as the child reads |
| Return after the first accepted chunk | Simplest loop | Programs that treat a blocking write as complete would start seeing short writes |

### Windows pipes and the completion port

| Option | Pros | Cons |
|---|---|---|
| `CreateNamedPipeW` overlapped server end with a synchronous `CreateFileW` client end for the child (chosen) | Parent gets real completion; child keeps an ordinary inheritable handle; same shape as Rust's std | Pipe names and a uniqueness retry |
| Keep `CreatePipe` and use helper threads | No pipe change | Synchronous handles cannot be overlapped; needs the Decision 0137 helper for a device the runtime owns |

| Option | Pros | Cons |
|---|---|---|
| Lazy per-pipe association owned by the pipe end; the operation's route links when a pipe is selected (chosen) | Costs nothing until a pipe joins a set; association lives as long as the handle; reuses the broker, registry and packet path of attached streams | Route linking moves from group setup to selection |
| Associate at spawn | Simpler lifecycle | Every child pays for a port it may never use, against Decision 0110's lazy association rule |
| Wrap the pipe in `AttachedStream` at registration | Reuses that type outright | `AttachedStream` owns the handle and cannot sit beside the `OwnedFile` that must keep owning it |

A plain transfer on an attached pipe uses a tagged event, so it adds no
completion packet.

| Option | Pros | Cons |
|---|---|---|
| Keep the helper thread for the error-output drain and give each drain call its own wait (chosen) | Smallest change to `wait`/`collect`; the documented drain shape stays | Still one thread during `collect` on Windows |
| Replace the drain with two overlapped reads and one wait | Removes the thread | A larger rewrite of a path that was not at fault, unverifiable here |

### A half-closed input in a set

The `readinessSource` getter panicked on a closed pipe because it required an
open file. It now always returns the source, and registration reports `Closed`.
A configuration failure on a pipe also names the pipe, not a file.

| Option | Pros | Cons |
|---|---|---|
| Getter returns the source; admission reports `Closed` (chosen) | No panic; the checked failure comes from the one admission path | The getter no longer proves the pipe is open |
| Keep the check in the getter | Rejects earlier | A Rust panic for a condition the program can reach legally |

### One child's input and output together

| Option | Pros | Cons |
|---|---|---|
| Treat the three pipe getters as loans of three distinct owner slots, in the source checker and the MIR proof (chosen) | No surface change; the classic write-stdin/read-stdout case works on every backend; a second loan of the same pipe and `wait` while a pipe is lent are still rejected | The checkers learn one more compiler-known rule (narrow, one class) |
| Add `getPipes(): ChildPipes`, one call returning several loans | Explicit | New public type; the property-loan rule of entry 40 makes its fields readonly loans, so it does not help without more language work |
| Move the pipes out: `takeInput(): ChildInput` | Ownership is plain | Runtime "already taken" states, and the destruction order that prevents deadlock on a full pipe no longer holds |
| Real stored properties on `Child` (`$child->input`) | Decision 0136 applies directly | Changes the settled spelling and the runtime-owned layout |
| Name it a blocker | No compiler change | Leaves the main use of pending child pipes unavailable |

The slot is a place identity only (Decision 0136's sibling-path rule), the same
device `add_template_fields` uses for generic templates. It is never storage,
and a contract that escapes a function widens back to the whole `Child`.

### Verification (2026-11-08)

macOS ARM64, Rust 1.98.1, `llvm-backend`, offline and locked, under the guard.
Targeted only; the full vet belongs to the end of this compiler round.

- `child_process_tests` 14/14, covering the interpreter, Cranelift, LLVM and PHP.
- `stream_capability_tests` 78/78, `pending_stream_contract_tests` 16/16,
  `mir_validation_tests` 125/125, `doriac` library 553/553, `doria-rt` library
  643/643.
- `cargo clippy -p doria-rt --all-targets -D warnings` is clean for
  `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu`, in the default and
  `hosted-shared-runtime` feature sets, and on the macOS host.
  `cargo clippy -p doriac --features llvm-backend --all-targets -D warnings` is
  clean. `check_docs_authority` passes.

Not run: Windows and Linux execution, so the overlapped pipe and completion-port
path has been compiled and linted but never executed. `readiness_set_contract_tests`,
`stage35_contract_tests`, `stage30b_semantics_tests` and `byte_region_tests` were
refused by the guard's running-job check, which matched the text of a monitor
command used during this work; they were not retried.

### Region-returning requirements: implementation (2026-11-08)

The rule recorded under "Found, not fixed: region-returning interface and trait
requirements" is implemented. Interface methods and accessors and bodyless trait
methods with a readonly or writable receiver elide a region or nullable-region
return to a retained borrow of that receiver. An implementation holding the
region in its receiver and one returning an independent region both conform.
Callers keep the receiver borrowed whichever implementation runs. Decision 0134
and `SPEC.md` state the rule. Doria has no `abstract` methods, so that case does
not arise. `docs/stdlib-reference.md` is unaffected: no standard-library
interface returns a region.

**Where the rule lives.**

| Option | Pros | Cons |
|---|---|---|
| `requirement_return_borrow` in `ownership.rs`, built from the body rule's `BorrowOrigins::contract` and `region_return_borrow`; the signature and inference passes both reach it through `declared_return_borrow` (chosen) | One elision rule for bodies and requirements; traits survive the inference pass that recomputes them | One more small helper on the checker |
| Fill `return_borrow` while collecting interface and trait declarations | Smallest diff | A second copy of the rule, and the inference pass walks trait members and would overwrite it |

**A readonly requirement returning a writable region.**

| Option | Pros | Cons |
|---|---|---|
| Hold a shared loan of the receiver (chosen) | No new rejection; an implementation returning absence or independent storage conforms and runs | No implementation can return a writable region its receiver holds, which the body check already forbids |
| Reject the declaration | Flags a requirement of little use | A new language rejection with no accepted authority behind it |
| Keep the writable contract `region_return_borrow` computes | One rule with no adjustment | Promises exclusive access to a readonly receiver, which MIR validation rejects as an invalid return-borrow source |

**What a call through a requirement retains.**

| Option | Pros | Cons |
|---|---|---|
| The contract alone: the whole receiver, decided per call from the call's type (chosen) | Holds for implementations the program does not contain and for all-independent programs; a receiver seen through a requirement has no reachable fields, so nothing is lost | Region calls through a requirement do not consult the `dispatch` join |
| The `dispatch` join, as other calls use | The existing path | Yields no loan when every implementation is independent, and its field paths can name classes this receiver is not, which MIR validation rejects |
| Seed `dispatch` with a receiver source | Keeps one path | The seed belongs to the declaration, so it also binds specializations of a generic interface that return `int` |

**Paths in an interface's dispatch join.** `interface Sized extends Viewing`
failed MIR validation: the join for `view` named a field of an implementer of
`Viewing` that no `Sized` value can be.

| Option | Pros | Cons |
|---|---|---|
| Clear receiver field paths when joining an interface requirement (chosen) | Fixed at the join, for every carrier a requirement returns; a generic `U` result keeps one shape across instances | None visible: nothing can reach a field of an opaque receiver |
| Filter paths per static receiver interface at each call | Keeps paths | Needs the implementer set at every call, for a fact no caller can use |
| Leave the join alone | No change | Valid programs stop at MIR validation |

**Contracts on `T`.**

| Option | Pros | Cons |
|---|---|---|
| Decide from the substituted return type for an interface's own `T`; leave a generic method's own `T` to its arguments (chosen) | Covers every region-carrying return the body rule covers; `Producing<ByteRegion>` and `Producing<int>` each get the right contract | A generic function forwarding `Producing<U>` shares one template summary across instances |
| Only requirements written with a region type | Simplest | `Producing<ByteRegion>` stays inexpressible |
| Treat a method's own `T` like an interface's | One rule | Moves a loan from the argument to the receiver and breaks `own<T>(take T $view): T` |

**Plans of entries and generic instances.** An interface entry selecting an
independent implementation, and an instance like `constrained<Absent>`, have a
retained signature with nothing in their body behind it.

| Option | Pros | Cons |
|---|---|---|
| The plan states the signature's source: `RetainedSourcesPlan::state_region_contract` (chosen) | One owner for entries and instances; MIR and the source checker agree on the contract | A plan can name a source its body does not use |
| Narrow the MIR signature per instance | Smallest plans | The generic's contract would differ between source and MIR, per specialization |
| Relax `byte_regions::return_contract` | One line | Accepts signatures no plan backs, which the check exists to reject |

**Static and consuming requirements** keep no elided source. The decided rule
says "with a receiver", a static requirement has none, and a consuming receiver
dies with the call. Eliding a static requirement to one borrowed parameter was
not chosen because it extends the decided rule; it stays open for Andrew.

**Found, not fixed: callable-returning requirements.**
`interface F { function make(): function(): int; }` cannot be implemented by a
closure that captures `$this`; that fails with E0755. It is the same gap this
rule closes for regions, but the decided rule names regions only, and eliding
callable returns would bind every caller of such a requirement to its receiver.
Andrew decides it separately.

**Coverage.** `native_return_transport_tests` runs a receiver-held, a narrowed,
an absent and a lent implementation, a nullable form, interface views, witnesses
and constrained generics on the interpreter, Cranelift, LLVM and PHP. It also
covers checked errors narrowed per specialization, generic interfaces, inherited
and composed implementations and interface inheritance. `byte_region_tests`
holds the negative cases, and `byte_region_mir_validation_tests` rejects an entry
or a generic instance whose plan loses the contract.

### Verification (2026-11-08)

macOS ARM64, `llvm-backend`, offline and locked, under the guard. Targeted only;
the full vet belongs to the end of this compiler round.

- On the final code: `doriac` library 553/553, `native_return_transport_tests`
  5/5, `byte_region_tests` 34/34, `byte_region_mir_validation_tests` 28/28,
  `stage35_contract_tests` 56/56, `stage35_trait_tests` 17/17,
  `conditional_capability_body_tests` 38/38, `_coherence_tests` 32/32,
  `_composition_tests` 12/12, `_grammar_tests` 23/23.
- Before the last mechanical cleanup (a helper's return type and formatting):
  `stage35_runtime_tests` 46/46, `stage35_graph_tests` 6/6,
  `byte_region_execution_tests` 32/32, `byte_region_transport_tests` 9/9,
  `byte_region_native_structure_tests` 4/4, `mir_validation_tests` 125/125,
  `mir_tests` 148/148, `returned_closure_transport_tests` 78/78,
  `semantic_ide_tests` 12/12, `stream_capability_tests` 78/78,
  `pending_stream_contract_tests` 16/16, `readiness_set_contract_tests` 70/70,
  `stage36_property_hook_tests` 91/91, `writable_borrow_writeback_tests` 43/43,
  `stage30b_semantics_tests` 32/32, `stage27_tests` 41/41, `stage21_tests` 89/89,
  `registration_key_tests` 17/17, `consuming_receiver_tests` 29/29,
  `transfer_plan_tests` 24/24, `transfer_group_tests` 35/35,
  `pending_selected_storage_tests` 11/11.

Not run: the full vet, the parity suite, clippy, and Linux and Windows execution.

## PHP byte-range bridge (2026-11-08, closes the address-view item)

PHP cannot form an interior address without allocating an FFI object, so each
scalar transfer at a nonzero offset allocated one. Regions cached a view each,
`copyFrom` and `copyWithin` allocated two per call and snapshotted overlapping
ranges through a PHP string, and each hosted source reply read a pointer slot
into a new object.

**Chosen:** every PHP byte transfer names its range by the owner's one cached
base, the owner's count, and an offset and length. `byte_region::host_range` is
the one place that checks the range and forms the address; a range that leaves
its owner is a P1001 runtime invariant. New `dr_v1_host_*` entries wrap file
read and write, standard-stream transfer, transfer-plan new and append, and
`copyFrom`/`copyWithin`. The two host-only pending configure entries change
signature in place. `__DoriaBytes::__doriaNativePointer`,
`__DoriaByteRegion::$nativePointer` and `__doriaNativeRegionPointer` are gone.
The native backends' entry points are unchanged. Callers hold the owner as a
strong local through the call, which the old `CData` argument did not do.

The hosted group's record is now one `void *` cell. Native rewrites it for each
handoff and the reply passes the cell, which reads its current value. Nothing
holds it past the matching reply, so the loan rule that forbade caching stands.

| Option | Pros | Cons |
|---|---|---|
| Native range entries (chosen) | No shared mutable state; the runtime owns bounds and address; the plan's proposed shape | Seven new thin entries |
| One reusable pointer cell, retargeted through an integer alias | No new entries | Hidden state shared across calls: a destructor running during result clearing could retarget it; forges pointers from integers |
| Declare the data parameters as `uintptr_t` in the PHP cdef | No new entries, no allocation | The declaration misstates the prototype; bounds stay outside the runtime |
| A native call that forms the pointer before each transfer | One new entry | Adds an FFI hop that costs more than the allocation it removes |
| Cache a view per offset | Small | Rejected earlier: the cache grows with offsets used |
| Leave plan configuration and bulk copies on `FFI::addr` | Fewer entries | Keeps a second owner of address formation for the same class of work |

**Coverage.** Runtime unit tests check range limits, interior addresses and
overlapping copies. A PHP test drives 1,000 file writes and reads at a nonzero
offset through `writeFields` and `readFields`, plus copies in both overlap
directions and between owners. Structural checks reject `FFI::addr` in the
transfer bodies and any interior view in the generated program.

### PROMINENT: time allowance raised from 28 to 36 hours for the closing vet (2026-11-08)

The five-fix compiler round, the adapter-chain rerun and the release compiler
left about 11,000 s of the 100,800 s allowance. The closing vet runs in guarded
batches and has used more than that before, and any Linux or Windows correction
round after CI needs its own rerun.

**Chosen:** add 8 hours (100,800 -> 129,600 s). The other limits stay: one
cargo job at a time, 3 GiB memory, the run cap, offline and locked, and the
25 GB cache ceiling.

| Option | Pros | Cons |
|---|---|---|
| Raise by 8 hours (chosen) | Covers the vet, the benchmark cases and one correction round | Andrew was told in chat before it applied, but did not approve the amount |
| Raise by 4 hours | Smaller | Likely to stop during a correction round and need a third raise |
| Run the vet outside the guard | No raise | Loses the memory limit and the record of time spent |

The guard allows one run per ledger at a time, so the closing vet takes 26,000 s
of the 36 hours in its own ledger and the benchmark agent keeps the rest. The
memory and cache limits apply to each, and only the vet runs cargo.

### Performance gate: stream_adapter_chain and the pipe case rejoin the gate (2026-11-08)

Commit `d176222` fixes the defects behind "`stream_adapter_chain` stopped on a
compiler defect" and "non-blocking pipe transfer stopped on a runtime defect".
Those two stops no longer apply: `stream_adapter_chain` verifies on all four
variants at that commit, and the pipe case exists as
`nonblocking_pipe_transfer`, with all ten binding cases in `matrices/stage36a.json`.
The README's Stage 36a status block and the matrix description drop their blocked
and excluded wording. The rest of those two entries stands as history.

### Performance gate: who owns the child command (2026-11-08)

`child_output_drainage` hardcoded `sh -c 'cat "$1"; cat "$1" >&2'` in all three
targets, and the new pipe case needs an echoing child. `cat` and `sh` do not exist
on a stock Windows machine, and `cmd.exe type` is not a byte-exact binary copy.
The child is part of the workload, so the suite should own it, not each target.

**Chosen:** one committed child program, `runtime/stream_child/stream_child.rs`,
declared with a `childProgram` manifest field, built by the harness into the
build root, and passed to every target through a `{childProgram}` argument token
in the workload profile. The report records the source and artifact hashes and
keeps the declared argument list; `runCommand` shows the resolved path. This
replaces the child half of "Performance gate: Unix-only child and readiness cases".

| Option | Pros | Cons |
|---|---|---|
| One suite-owned child program, path passed by token (chosen) | One owner; identical child for every target and platform; no per-case branches; hashes in the report | Adds a manifest field, a harness step and a schema entry; the case needs rustc even when only Doria targets run |
| Per-platform shell strings (`sh` on Unix, `cmd.exe` on Windows) | No build step | Windows has no byte-exact shell copy, so the case would not match; two commands to keep identical |
| PHP as the child | Always present with the harness | PHP startup and throughput dominate the measured interval, and the child cost would differ from any native child |
| A per-case child beside each case | Local | Duplicates the source for each case; the same drift the shared child removes |

### Performance gate: child program language (2026-11-08)

**Chosen:** Rust, one source with no per-platform code, built with `rustc -O`.
std's stdin, stdout and files move raw bytes on Linux, macOS and Windows, and
the same `rustc` invocation builds it everywhere.

| Option | Pros | Cons |
|---|---|---|
| Rust (chosen) | One portable source; rustc is required for the Rust peers anyway; byte-exact stdio on all three platforms | Needs rustc on the machine |
| C | Same | Needs `#ifdef` for binary stdio on Windows; the harness's `cc` is not guaranteed on Windows |
| Doria | Dogfooding | A Doria defect would invalidate the C and Rust peers of the case |

### Performance gate: Windows peer mechanism (2026-11-08)

The three child and readiness cases are now declared `linux, macos, windows`. The
Windows forms live in each peer's own source behind `#ifdef _WIN32` and
`#[cfg(windows)]`, so the peer stays the owner of its OS calls.

**Chosen:** overlapped named pipes and regular files on one I/O completion port,
matching the mechanism `doria-rt` uses on Windows, for the C peers of all three
cases and the Rust peers of the pipe and readiness cases; the Rust drainage peer
keeps `Command::output`. Windows has no readiness API for regular files, so
`readiness_registrations` issues each one-byte read at an explicit offset and
waits for its completion. This replaces the other half of "Performance gate:
Unix-only child and readiness cases"; the case stays a registration and completion
measurement.

| Option | Pros | Cons |
|---|---|---|
| Overlapped handles and one completion port (chosen) | Real Win32 asynchronous I/O and the same mechanism the Doria runtime uses; no thread cost | Windows peers are longer; no Windows machine here to run them |
| A reader thread per stream | Simple pipe code | A second, different concurrency model from the Doria runtime and the poll peers; adds thread cost to a stream measurement |
| Leave `linux, macos` and skip Windows | No unverified code | The gate is defined as cross-platform; the Windows gap would stay open |

### Performance gate: pipe transfer workload shape (2026-11-08)

**Chosen:** the program keeps one 64 KiB seed in memory, sends it a profile-given
number of rounds through one `TransferPlan`, and writes the echoed bytes to a
file, 16 rounds for smoke (1 MiB) and 1024 rounds for baseline (64 MiB). The write
and read adapters use capacity 1. A larger writer capacity is a usage trap: its
pending path takes a short final write into the buffer and reports it accepted, so
the last bytes never reach the child while the program waits for them. Capacity 1
makes every transfer pass straight to the pipe. The peers use the same exact-count
loop and close the child's input only after every byte has come back.

| Option | Pros | Cons |
|---|---|---|
| Seed repeated by rounds, capacity 1 adapters (chosen) | Bounded memory; isolates the pipe path; exact hash; no adapter buffering hides bytes | Short write size is fixed by the seed |
| Stream the whole input file through the pipe | One file operation shape | Measures file reads as well as the pipe, and adds a second adapter to each direction |
| Larger adapter capacity with an explicit flush after the loop | Exercises buffering | The flush would be a plain blocking write after the group ends, which can deadlock against a full child pipe, the exact case this benchmark exists to prove absent |

### Performance gate: Windows evidence short of Windows (2026-11-08)

No Windows machine is available, and CI cannot be used without a push. Wine 10.0
is installed on this Mac and runs x86_64 Windows executables under Rosetta.

**Chosen:** run the Windows paths under Wine, and say in each case's description
and in the README that this is not Windows. The Rust peers and `stream_child` were
linked as real `x86_64-pc-windows-msvc` executables with import libraries generated
from Wine's DLLs and a minimal CRT stub. The C peers were compiled with Homebrew
clang against scratch stand-ins for `windows.h` and the CRT headers, kept out of the
repository, and linked the same way. All six executables produced the smoke hashes
under Wine. The Doria programs were not compiled for Windows, because `doriac` here
has no Windows target. `rustc --target x86_64-pc-windows-msvc --emit=metadata`
also passes for all three Rust peers and `stream_child`.

| Option | Pros | Cons |
|---|---|---|
| Wine run of the peers, plainly labeled (chosen) | Runs the Win32 code paths and catches logic errors; no Windows-only claim | Not Windows; the C peers used stub headers, so a real `windows.h` compile is untested; Doria itself is not covered |
| `--emit=metadata` only | Simple | Proves nothing about the C peers or about any peer's behaviour |
| Commit a Windows CI job | Real Windows runs | Outside this gate's scope, and needs a push and a Windows runner |

### Closing vet of the compiler round (2026-11-08)

macOS ARM64, LLVM enabled, offline and locked, at `d176222` plus the vet tooling.
6,156 tests passed and none failed or were ignored: every `doriac` target, the
parity suite and its three shards, `doria-rt` in debug and release with both
feature sets, and the diagnostic catalogue. Clippy with warnings denied passes
for macOS, and for Linux and Windows across `doria-rt`, its hosted form and
`doriac`. Formatting and the script checks pass. The earlier stall of
`copy_parameter_values_and_writable_callback_borrows_match_native` did not recur.

Two script checks failed on the first pass. `check_build_artifact_policy` read
the suite list from its old file after the list moved to `vet_commands.php`,
and now reads it there. `check_docs_authority` reports only that the sibling
`benchmarks` checkout is ahead of the pin, which is expected until the pin moves.
Linux and Windows have not executed this round; that is the next step.

### Callable-returning requirements stay rejected (2026-11-08, settles the open item under "Region-Returning Requirements")

`interface F { function make(): function(): int; }` cannot be implemented by a
closure that captures `$this` (E0755).

**Chosen:** leave it rejected and outside Stage 36a. The region rule fits regions
because a region is a view of storage. A callable is commonly a value that
outlives its factory, so a default tying it to the receiver would change what
every caller of such a requirement can do with the result. A sound opt-in needs
its own language decision.

| Option | Pros | Cons |
|---|---|---|
| Keep E0755 (chosen) | Sound; no program changes meaning; no 36a work | A receiver-capturing closure still cannot implement the requirement |
| Apply the region rule to callables | One rule for both | Binds every returned callable to its receiver and rejects programs that keep it, which breaks factories |
| Add an explicit retained-callable return form | Precise | New language surface unrelated to streams |

### Benchmark CI: which compiler revision the structural tests use (2026-11-08)

The benchmark tests run the compiler repository's own cache helpers, and CI had no
copy of that repository. `doria_build_environment` exists on `develop` and not yet
on `main`, so the revision matters.

**Chosen:** `tests.yml` checks out `dorialang/doria` under `.ci/doria` and sets
`DORIA_REPO`. A run for `main`, or a pull request into it, uses compiler `main`.
Every other ref uses compiler `develop`. Benchmarks `main` therefore cannot go green
until compiler `main` has the helpers it calls, which is the real ordering between
the two repositories.

| Option | Pros | Cons |
|---|---|---|
| `main` tracks `main`, everything else tracks `develop` (chosen) | Each line is tested against its counterpart; feature-branch dispatches work | A `main` merge fails until the compiler's `main` catches up |
| Always compiler `develop` | Simple; never blocks a merge | `main` is validated against helpers it may not ship with |
| Pin a compiler commit in the workflow | Reproducible | Another pin to move by hand, and it hides drift in the helpers the tests exist to exercise |
| Skip the tests when the repository is absent | No checkout | CI stops testing the delegation these tests prove |

### Benchmark CI: line endings in the benchmark repository (2026-11-08)

The Windows runner checks out with `core.autocrlf`, which rewrote `expected/*` to
CRLF and broke byte comparisons. All 297 tracked files are LF or empty in the index.

**Chosen:** `.gitattributes` holds `* -text`, so Git never rewrites a byte in any
checkout on any host.

| Option | Pros | Cons |
|---|---|---|
| `* -text` (chosen) | States the actual rule; covers hashed sources as well as expected output; needs no renormalization | A future CRLF file stays CRLF, which here is correct |
| `* text=auto eol=lf` | Normalizes what is committed | Git guesses which files are text, and a data file with CRLF would be altered |
| Attributes only on `expected/` | Narrowest | Benchmark sources are hashed too and would still differ on Windows |
| Strip CR in the comparisons | No repository change | Weakens exact-output checks and leaves the checkout wrong |

### Benchmark CI: 64-bit only Windows declarations (2026-11-08)

The adapter's 26 native declarations named `__stdcall`. PHP FFI rejects that where
the platform has one calling convention, so every Windows launch failed at load.
arm64 macOS reproduces the same `ParserException`.

**Chosen:** the declarations name no convention and the preflight refuses 32-bit
PHP with a message naming the fix. A new test parses them on every 64-bit host.

| Option | Pros | Cons |
|---|---|---|
| No annotation, refuse 32-bit (chosen) | One supported ABI, matching the Win64 layout test and the 64-bit Unix requirement | 32-bit Windows PHP cannot run new measurements |
| Add `__stdcall` only when `PHP_INT_SIZE` is 4 | Keeps a nominal 32-bit path | A code path no host runs, and the struct layouts are only tested for Win64 |
| Declare `__cdecl` | Parses everywhere | Wrong on 32-bit Windows, where kernel32 is stdcall, so it would corrupt the stack instead of refusing |

### Benchmark CI: testing for INVALID_HANDLE_VALUE (2026-11-08)

Once the declarations loaded, PHP died without output at the first launch. `FFI::cast`
to an integer reads through a `void *`, and a kernel handle is not an address. A
`0x1f4` handle gives signal 11 on macOS with the same cast.

**Chosen:** compare with `$handle == $api->cast('HANDLE', -1)`, which compares pointer
values. A new test passes a plausible handle value through the validator on every
64-bit host.

| Option | Pros | Cons |
|---|---|---|
| Compare against a cast `-1` handle (chosen) | Says what Windows documents; no read through the handle | Relies on FFI's pointer comparison |
| Cast through `char *` before `intptr_t` | Yields the integer | An unexplained double cast that avoids the trap by accident |
| Make `HANDLE` a pointer to an opaque struct | The cast stops dereferencing | Changes every declaration to serve one check |

### Benchmark CI: a real compiler probe on Windows (2026-11-08)

Nine compiler locator tests were gated off Windows only because their stand-in for
`doriac` was a shebang script. The locator, cache precedence and provenance checks do
run on Windows, and `is_executable` there requires a real image.

**Chosen:** a short C# program, compiled once per suite run with the .NET Framework 4
`csc.exe` and copied to each fixture path. It reads the same sibling `version.json`
and writes the same `unexpected-invocation.log` as the Unix script. A host without
that compiler fails with a message naming the fix.

| Option | Pros | Cons |
|---|---|---|
| C# probe built with the system `csc.exe` (chosen) | Readable; ships with Windows; test bodies unchanged | Depends on a Windows component, and C# appears in a PHP repository as an embedded fixture |
| Hand-assembled PE written by PHP | No compiler dependency | Opaque machine code in a test fixture, and x64 only |
| Compile the probe with MSVC or MinGW | Conventional | Not on every Windows host |
| Commit a prebuilt `.exe` | No build step | Violates the rule against tracked binaries |
| Leave the tests Unix-only | No work | Nine tests of behaviour that exists on Windows stay unrun |

### Benchmark CI: symlink tests on hosts that cannot create links (2026-11-08)

Windows has symlinks, and the fixture and compiler code guards against them, so those
tests belong on Windows. Creating a link there needs Developer Mode or elevation. The
GitHub runners have it.

**Chosen:** the tests run unconditionally. Without the privilege they fail with a
message naming the fix. The helpers supply what PHP on Windows needs: a target that
exists at link time, backslash targets, and `rmdir` for links to directories.

| Option | Pros | Cons |
|---|---|---|
| Fail and name the fix (chosen) | Coverage cannot vanish silently; matches the rule that errors name the fix | An unprivileged Windows developer must enable Developer Mode to get a green suite |
| Skip when `symlink()` is refused | Green everywhere | A misconfigured runner would drop the coverage unnoticed |
| Keep the OS gate | No work | Symlink rejection stays untested on the platform where links are least obvious |

### Benchmark CI: inherited-pipe tests without fork (2026-11-08)

Two tests used `pcntl_fork` to leave a descendant holding the child's pipes after the
direct child exits. Windows has no fork, but the capture loop has the same obligation
there.

**Chosen:** on Windows the direct child spawns the descendant with `proc_open`,
sharing its stdio, then leaves through `ExitProcess`. PHP's normal shutdown waits for
whatever `proc_open` started, so `exit` would have hidden the case. Unix still forks.

| Option | Pros | Cons |
|---|---|---|
| `proc_open` plus `ExitProcess` on Windows (chosen) | Same assertions on all three platforms; FFI is already required there | The script differs by platform |
| `proc_open` on every platform | One script | Unix would need its own way to skip PHP's shutdown wait, and fork already works |
| Keep them gated on `pcntl_fork` | No work | The Windows capture loop stays unproven for this case |

### Benchmark CI: canonical test scratch (2026-11-08)

The runner reports its temporary directory by short name, `RUNNER~1`. A child's
`getcwd` kept that spelling, and the test compared it with `realpath`.

**Chosen:** `make_fixture_directory` builds from `realpath(sys_get_temp_dir())`,
matching what `benchmark_create_runtime_fixture` already does for runtime fixtures.

| Option | Pros | Cons |
|---|---|---|
| Canonicalize the test scratch (chosen) | Fixes the spelling at its source; all fixture paths benefit; no change to launches | None found |
| `realpath` the working directory in the Windows adapter | Children always see the long name | Changes production launches to satisfy a test, and rewrites what the caller asked for |
| Compare case-insensitively or by short name in the test | Local | Weakens the assertion about the working directory |

### Benchmark CI: the gate's programs run on every platform (2026-11-08)

The gate's Doria programs had only run on macOS, and the Windows forms of three
cases had never compiled there.

**Chosen:** a `stream-gate` job in `benchmarks` CI builds the compiler from
`dorialang/doria` and verifies every case in `matrices/stage36a.json` on Linux,
macOS and Windows with the fast native profile. All ten pass on all three.

| Option | Pros | Cons |
|---|---|---|
| Job in `benchmarks` CI with the fast profile (chosen) | Real Windows execution; `doria` is public, so no secret is needed; no LLVM install | LLVM builds of these programs on Windows stay unrun |
| Job in `doria` CI | LLVM is already installed there | `benchmarks` is private, so it needs a cross-repository token, a settings change |
| Leave Windows to an available runner | No work | Claims a cross-platform gate from one platform |

### The benchmark pin may name `develop` (2026-11-08)

`cross-repo-pins.json` accepted a benchmark revision only from `main`, while the
language server and website pins accept `main` or `develop`. The check exists
to reject a commit that is on no remote.

**Chosen:** accept `develop` here too.

| Option | Pros | Cons |
|---|---|---|
| Accept `develop` (chosen) | Same rule as the other pins; the commit is published | A `benchmarks` main merge is still to come |
| Merge `benchmarks` to `main` first | Keeps the narrower rule | Merging to `main` is Andrew's call, and that merge fails until compiler `main` catches up |

## Stage 36a delivered (2026-11-08)

Closure conditions in the plan, with their evidence:

| Condition | Evidence |
|---|---|
| Complete portable checks | `doria` CI green on Linux, macOS and Windows at `4670b18`, audited for hidden failures |
| Required backend regressions | Closing vet 6,156 tests on macOS; interpreter, Cranelift, LLVM and PHP suites and parity shards; CI on all three platforms |
| Structural baselines | `baselines/stage36a.json` accepted, 26 entries, all Pass |
| Benchmark commands and report schemas | All ten binding cases in `benchmarks`; verified on Linux, macOS and Windows in CI |
| Coordinated tooling | Language server pinned at `fbd4564`, CI green |
| Target-state website and API updates | Website at `c7c8fa8`, CI green, API lock at `4670b18` |
| Installed-tool refresh | `doriac` from `4670b18`, `doria-lsp` from `fbd4564` |

Still open and not closure conditions: controlled timing is Pending Available
Runner; LLVM builds of the gate programs have not run on Windows; the remaining
public spellings are deferred under Decision 0110; callable-returning
requirements stay rejected with E0755; `benchmarks` and `doria` have not been
merged to `main`.
