# Current Pipeline

Documentation role: working note. This file prevents duplicated in-flight work. It is not a roadmap; `docs/doria-end-to-end-plan.md` owns the roadmap.

## Scheduled application direction

The master plan adds Stage 40a network/data foundations and the separate post-40a OSINT prototype checkpoint 40b. `Doria\Tui` design starts against Stage 36a; executable acceptance follows Stage 46. Native UI checkpoints UI-1/UI-2 cover Doria components/styles and the reviewed PHP host path after Stage 40/41. These are scheduled product checkpoints, not implemented compiler surfaces. The [supporting package design](../application-package-design.md) records proposals and open decisions.

## Recently merged

- PR #75: Stage 17 integration, parity, examples, editor, docs, and CI closure.
- PR #76: Stage 17 naming, I/O-tier, and migration-guidance corrections.

## Active

- Stage 46, the terminal layer, delivered 2026-11-11 under Decision 0140.
  `Doria\Std\Term` is one module authored in Doria over two runtime
  primitives: `Console`, the `RawMode` and `AlternateScreen` guards, typed key
  and resize events, `Style` and `Color`, `TerminalError`. The runtime keeps a
  ledger of what a program changed and puts the terminal back on return,
  closed pipe, panic and signals. Windows uses console modes and input
  records behind the same VT renderer and decoder as Unix.
  Also delivered with it: `$isInteractive` on the standard views; terminal
  events in a transfer-group wait (`Watchable`, `TransferPlan::watch` and
  `andWatch`, `WatchKey`, `TransferCompletion::Ready`, `resubmitWatch`,
  Decision 0110 "Watched Sources"); PHP output for all of it through the
  native runtime; integer `/ % << >> & | ^ ~`, their compound forms and
  checked `Target::from` in PHP output, which the module itself needed; one
  decoder for native I/O failures in PHP output; the runtime's self-check
  panic (P1001) naming the check that failed; E0781 for a program class that
  declares `Watchable`; standard modules reaching each other's `internal`
  members.
  Acceptance: `examples/terminal/menu.doria`, the `Menu` / `MenuItem` program
  Andrew asked for, runs unchanged on Linux, macOS and native Windows. The
  `terminal` CI job compiles it on each platform, drives it in a pty or
  ConPTY (keys, a window resize, cancel) and holds every screen to the
  interpreter's, with the fast profile, the release profile and PHP output.
  The same harness runs a key and modifier probe, a return and panic restore
  probe, a relay that waits on a command's output and the terminal together,
  and a checked error leaving a group that reads and watches.
  Evidence: CI run 43033980886 on `a2d19dc`, all three `terminal` jobs green.
  Local validation on macOS at `bceee70`: `php scripts/validate_work_unit.php
  --llvm` ran every step from an empty cache (the repository guards, format,
  strict Clippy for the host and for the Windows and Linux targets, the
  workspace tests, the hosted runtime profile, the release runtime tests and
  the LLVM suites): 8,254 passing test executions in 169 suites, none failed
  or ignored. The runner exited 1 only after its 22.79 GiB target exceeded
  the separate 15 GiB cache cap and was reclaimed; this is not an overall
  validator pass. Since the local vet gained the Windows and Linux lint
  graphs, a run from an empty cache cannot stay under that cap; that is a
  defect in the validator and is open.
  Open, not closure conditions: the runtime's own pty tests (signals, stop
  and continue, cursor query) run on Unix only, so those paths have Windows
  coverage only through the probes; a cancelled pending read of a child's
  pipe is reported as a failure on Windows and as no outcome on Unix, which
  PHP output now decodes but nothing unifies; `codegen_php_tests` is not in
  the Windows CI job; mouse events, a line editor and scroll regions are
  later slices of Decision 0140; the build ledger was reconciled by hand once
  on 2026-11-10 after its guard was killed.
  After the completion commit `61e3145` the `rust` CI job reached its
  90-minute limit: its test step had grown from 52 to 71 minutes in a day.
  Ten of those minutes were one Stage 46 test, the PHP integer operator
  parity test, whose generated `main` has about 1,366 statements. The cause
  was the Stage 36a call-origin proof, which copied its whole per-local state
  at every block and edge; `863949d` makes copies share unchanged values and
  cut that proof's cost by about 28 times on that program. Still open from
  the same measurements: the borrowed-views validator has the same dense
  state and is now the larger cost (about 5 seconds for that function in a
  release build); a fast-profile executable for that program is 74 MB, 66 MB
  of it read-only data, and grows with the square of program size.
  With the fix the `rust` job passed in 81 minutes (run 43099429345 on
  `0514bfd`). It had been running the durable parity suite twice, once in its
  workspace test step and again in a step of its own, about 14 minutes a run;
  Andrew ruled on 2026-11-12 to remove the repeated step. The suite still
  runs once, in the `Test` step.

- Stage 36a push, 2026-11-05 (delivered 2026-11-08). Decisions and their
  alternatives are in `docs/notes/stage36a-decision-log.md` (entries 1-51).
  Landed: PHP generic payload enums (class per specialization, exact `mixed`
  identity); `FinishOutcome<S, R>` / `BorrowedFinishOutcome<R>`; ordinary-class
  `borrow` / `borrow writable` retention (E0761 now only shape errors);
  payload loan transport (closures still E0658); `take $this->field`
  extraction (E0780; skips only the receiver's own destructor); per-application
  move classification; buffered byte adapters (owned and borrowed) as authored
  Doria; template field identities, retained-field projection and
  constructor-collection fixes; nonblocking constrained-target and seeding
  fixes; symbolic conformances no longer disable PHP field transport;
  assignment destinations under Decision 0136 (MIR validation now holds only
  the destination path while the value is evaluated, matching source
  checking).
  Evidence on macOS ARM64 with LLVM: the full `doriac` +
  `doria-diagnostic-catalogue` run had 104 suites green and 10 failing
  (22 tests). After the fixes above, a recheck passed library 545/545, PHP
  codegen 142/142, stream capabilities 72/72, Stage 27 41/41, Stage 35
  contract 56/56, consuming receiver 29/29, MIR validation 125/125,
  returned-closure transport 78/78, byte regions, indexed access, transfer
  groups and standard-stream MIR except one test each in the last two suites.
  Clippy with warnings denied, fmt, diagnostic style and docs authority pass.
  After the assignment-destination fix: Stage 21 89/89 (adds an
  interpreter/Cranelift/LLVM/PHP regression and a MIR destination-reservation
  test), MIR validation 125/125, library 545/545, durable native parity 16/16,
  byte-region MIR 27/27 and execution 28/28, writable-borrow writeback 43/43,
  property hooks 91/91, returned-borrow aliases 36/36, pending selected
  storage 30/30 and 10/10, consuming receivers 29/29, constructor-owned
  properties 10/10, Stages 20, 23, 30c and 30d green; the failures below are
  unchanged.
  Resolved 2026-11-06 by decision log entry 47 (all eleven now pass): `stage35_runtime_tests::virtual_overrides_preserve_retained_input_dependencies`
  (virtual slot return contract), `standard_stream_mir_tests::public_standard_views_register_transfer_borrow_and_return_concrete_and_erased_values`
  (PHP field-argument entry), `property_hook_cleanup_effect_tests::consuming_match_drops_only_payloads_not_transferred_by_the_selected_arm`
  (extra E0478), `stage13_mir_interpreter_tests::typed_literal_binary_expression_keeps_uint16_context_in_mir`,
  `php_integer_execution_tests::checked_integer_loops_reuse_direct_places_without_sequencing_closures`,
  and two `owned_copy_cleanup_tests` `when` loop-exit cases (Codex's untracked
  tests). Four `mir_tests` defensive-check tests predate today: since the
  Nov 1 monomorphization change, unchecked HIR is rejected with I2401 before
  MIR lowering's M1101. Linux/Windows not rerun; runtime crates unchanged.
  2026-11-06: E0770/E0771 related labels no longer point into compiler-owned
  std sources and are deterministic (log entry 30;
  `pending_stream_contract_tests` 16/16, stream capabilities 72/72, property
  hooks 91/91, conditional capability grammar 23/23; property-hook cleanup
  effects fails only the E0478 test above). Lock failures gain
  `IoOperation::Lock`/`Unlock` in Decision 0138 (log entry 25). Tooling and
  Website synced to this tree, all local and uncommitted. Language server
  483/483 (library 304, CLI 3, protocol 176) against compiler build-input
  identity `dev-23d4f7e9…`, run by the compiler session (log entry 29). Website
  Pest 334/335; the remaining test is the API-surface lock, which moves with
  the compiler pin at commit (log entry 28). Website's guide-example and
  compiler-backed tests run at 36a close.
  2026-11-06: preparation gives every diagnostic location the identity of the
  source its span is in, and compiler-owned locations never reach a
  presentation (log entry 31; `diagnostic_experience_tests` 7/7, `diagnostics::`
  unit tests 15/15). Tooling notified. For the final vet: the language server
  suite and `stage31_package_graph_tests` against this change (not run; another
  session was editing the crate).
  2026-11-06: file locks (Decision 0138) implemented as authored guards over
  file-control intrinsics, with runtime owner lock state and std-wide
  `internal` (log entry 32). Interpreter, Cranelift, LLVM and PHP agree
  (`stream_capability_tests` lock tests 2/2, runtime lock-state tests 2/2,
  catalogue 6/6); macOS only, Windows waiting unrun.
  2026-11-06: text adapters (Decision 0110, Text Adapters): bounded
  `LineReader`/`BorrowedLineReader` with exact UTF-8 failure offsets and
  `Utf8InputSource::Stream` (log entry 33), on every backend. Fixed on the way:
  Cranelift aborted on `String::fromBytes` of empty Bytes (runtime null slice);
  PHP now runs `String::fromBytes` and widening `X::from`. Runtime 638/638,
  catalogue 6/6, stream capability lock and line tests 4/4.
  Regressions from the day's work, found and fixed: PHP programs without I/O
  required the native runtime (std lock outcomes counted as runtime enums);
  two tag tests pinned the old IoOperation list; stream programs checked about
  8x slower because 33 lock-guard specializations each re-ran whole-program
  retained inference (log entry 36; a trivial check went from 5.0 s to 1.7 s,
  against 0.62 s before the day's std additions). Time allowance raised from
  6 to 8 hours (log entry 35). After the fixes: library 545/545, stream
  capabilities 75/75, Stage 27 41/41, codegen PHP 142/142, PHP execution 4/4,
  Stage 16 8/8, Stage 17 24/24, string runtime 5/5, pending stream contracts
  16/16, I/O progress structure 6/6, runtime 638/638, catalogue 6/6; standard
  stream MIR and PHP integer each fail only their known test.
  Child process spelling settled in Decision 0110 (Child Process
  Declarations); the new estimate (log entry 34) is 2-3 working days, about
  10-14 hours of build-and-test time.
  2026-11-06 later (log entries 37-40): allowance raised to 22 hours for
  processes (entry 37). Bulk region copies `copyFrom`/`copyWithin` on every
  backend; the buffered and line adapters use them (entry 38). Reserved words
  are valid member names (entry 39, for `spawn`). Child processes execute on
  the interpreter (scripted children), Cranelift, LLVM and PHP (FFI to the
  same runtime): posix_spawn with close-on-exec pipes and PATH search,
  poll-based concurrent drain, kqueue/pidfd deadline waits, SIGKILL
  termination, cleanup policy at destruction after the pipes close; the
  Windows module type-checks (CreateProcessW, handle-list inheritance,
  helper-thread drain) but has not run. Spelling amendments in entry 40:
  pipe getters are writable-loan methods, `Child` is a runtime owner. New
  IoOperation `Spawn`/`Terminate` and IoTarget `Process*` cases. Runtime ABI
  16. Results: child processes 6/6, runtime 639/639, catalogue 7/7, library
  545/545, byte-region copies pass on every backend. Programs include only
  the std declarations they reach (entry 41): a trivial stream check takes
  0.03 s. Destructors have writable access and the buffered writers flush on
  drop (entry 42). The buffered adapters forward readiness and pending tiers
  (entry 44); building them exposed and fixed an execution recheck that
  joined summaries numbered for the source pass (entry 45). The incremental
  nonblocking solver is deferred (entry 46).
  The eleven known failing tests are fixed (entry 47). Runtime artifact
  selection no longer rehashes the 24 MB runtime (entry 48: 2.16 s to 8-26 ms
  per debug compile). Text is written without copying through
  `string->byteRegion` (entry 49). The runtime type-checks for Linux and
  Windows (`x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc`).
  Final vet, 2026-11-06 (macOS ARM64, LLVM): every `doriac` suite, library
  547/547, `doria-rt` 639/639, the catalogue, clippy with warnings denied,
  fmt and the repository checks pass; the native parity matrix runs in three
  `DORIA_PARITY_SHARD` shards under the 900 s cap. The vet found and fixed
  the defects in entry 50 (macOS SIGKILL on rewritten executables, the
  child-process PHP helper in every program, erased pending calls missing
  E0774). Language server 487/487 (library 308, CLI 3, protocol 176) against
  compiler identity `dev-9870c82b…`, frozen for Website's example checks in
  `.agent-bridge/frozen/`.
  Website's seven guide examples compile against the frozen compiler.
  2026-11-06: calls that select their implementation after the template is
  checked (constrained methods, bodyless trait requirements, an explicit
  `toString` on a Displayable `T`) carry the complete automatic I/O set, as
  erased calls already did. This fixes a MIR-lowering panic, B0001 and a false
  E0629. Each checked call's transport now follows its selected callee and is
  lowered once, and the buffered writer destructors catch both ambient Errors
  (log entry 51; `constrained_ambient_effect_tests` 3/3 on the interpreter,
  Cranelift, LLVM and PHP).
  Re-vet after entry 51, 2026-11-07 (macOS ARM64, LLVM): every `doriac`
  suite (118 test binaries), the three parity shards, clippy with warnings
  denied, fmt and the repository checks pass; `doria-rt` and the catalogue
  are unchanged since the final vet. Language server 487/487 against
  compiler identity `dev-7bc0fd8c…`.
  Close, 2026-11-07: local develop commits `doria` `5e72056`,
  `doria-language-server` `339c997` (doriac pin `5e72056`; canonical workspace
  checks against it), `doria-website` `adb3b5b` (compiler pin and v2026.3.1
  API lock at `5e72056`; full Pest 347 passed against the compiler built from
  it). The installed `doriac` and `doria-lsp` were refreshed from those
  commits and report compiler `5e72056`.
  2026-11-07, after the close (uncommitted): catch reachability, the
  destructor boundary and nonthrowing initializers read each body's authored
  reach. Transport, MIR and the ABI keep the narrowed profile. The closures
  callers pass and a composer's members no longer decide E0629 or E0631
  (log entry 52; Decision 0123 amended; `constrained_ambient_effect_tests`
  7/7 on the interpreter, Cranelift, LLVM and PHP).
  Close, 2026-11-08: `doria` `1f68889`, `doria-language-server` `58585d1` and
  `doria-website` `13db4e4` are on origin develop, with the pins and the API
  lock at `1f68889`. CI is green in all three (log, "Close results at the
  final pins"). The installed `doriac` and `doria-lsp` come from those commits.
  Compiler round, 2026-11-08: catch reachability, enum source locations,
  per-specialization effect contracts, child pipes in readiness sets,
  region-returning requirements and the PHP byte-range bridge. The closing vet
  on macOS ARM64 with LLVM passed 6,156 tests, parity shards, release-profile
  runtime tests, and clippy for macOS, Linux and Windows.
  Stream gate: all ten binding cases exist and verify at bounded size on macOS
  at `d176222`; the structural baseline is accepted in `benchmarks`
  (26 entries, all Pass). Timing stays Pending Available Runner.
  CI is green on Linux, macOS and Windows for `doria` at `4670b18`, the language
  server at `fbd4564`, the website at `c7c8fa8` and `benchmarks` at `00c0eb2`,
  and the toolchain is refreshed from those pins. `benchmarks` CI compiles and
  verifies the ten gate programs on all three platforms with the fast native
  profile. On Windows, their LLVM builds and their C and Rust peers have not run
  (the peers ran under Wine). Controlled timing stays Pending Available Runner.

- Scoped class borrowing contract (approved 2026-11-03; implementation open): extend Decision 0134's
  constructor promotion to ordinary classes with `borrow T $source` for
  readonly access and `borrow writable T $source` for exclusive
  access. The parameter declares a non-owning stored field bound once during
  construction. It cannot be replaced or taken; mutation through a writable
  field affects the source. The existing owner remains responsible for
  destruction. Inference ties each stored loan to the checked caller place,
  including a supported disjoint property path, and checks the combined
  dependencies when the wrapper moves, returns, or enters another aggregate.
  No return or storage operation may outlive a source or launder exclusivity
  through erasure. Conflicting access remains unavailable until the loan is
  released. A destructor may use the field until it finishes; construction
  failure and ordinary cleanup release the loan without destroying the source.
  Consuming finish releases on completion and transfers the loan on retry.
  This approval does not settle owned-field extraction or
  the remaining adapter spellings, and adds no allocation or runtime lock.
  The parser already collects `borrow` and `writable`
  independently. E0761 still gates writable and ordinary-class retained
  fields, and distinguishes pending support from invalid parameter shapes.
  The first source change carries a typed readonly/writable mode from promotion
  through semantic properties into MIR. It is uncompiled. Ownership, MIR
  validation and backend consumers must enforce the mode before activation. Removing E0761
  alone would admit declarations without an executable exclusivity contract.
  Constructor summaries now carry the checked access mode, including parent
  calls. MIR checks the input access and preserves writable Copy source roots.
  Native ABI planning now forwards caller borrow homes for writable retained
  constructor inputs. Both native backends store the address in a pointer-sized
  field and skip source destruction. The interpreter records the caller place;
  PHP binds the field to the caller cell by reference. These compiler changes
  have not been compiled. The remaining work includes verifying exact storage
  types, address forwarding through parent construction and projections, and
  normal and checked-error cleanup across all backends before lifting E0761.
  The PHP reference probe exercised the host mechanism only, not emitted code.

- Enum loan follow-up, awaiting verification: match projections preserve
  aggregate loans when the selected field can carry a source but its precise
  provenance is unknown. Last-use cleanup also prunes nested payload facts.
  MIR origin analysis uses the same field-storage mechanism for classes and
  enum payloads, preserves the byte source across relocation, and handles
  present nullable payloads. MIR loan validation now tracks known case and
  field loans through local moves, nested payloads, coalescing and branch
  joins. Case and presence tests narrow the loan set; match bindings project
  it and drops retire the carrier. Loop widening and last-use expiration
  also apply to nested facts. Direct enum returns use the shared callable
  storage summary, but that summary does not yet carry exact case paths.
  The new regressions are uncompiled. Run the enum case-edge, field-binding,
  loop-widening, nested last-use and MIR region-origin regressions first,
  then the affected callable, pending-request and enum integration suites.
  Do not lift E0572 until callable loan transport and native/PHP validation
  are covered. The prior passing source identity below excludes this work.
  The independent MIR loan check is a separate gate from call-origin tracking:
  `collect_payload_enum_class_local_accesses` emits an `Independent` check
  per constructor field, and `borrowed_views` checks it against the field's
  retained sources. Case-origin propagation alone cannot authorize storing a
  retained payload. The checker still needs to prove destination lifetimes
  through aggregate escapes and callable handoffs, and verify case-specific
  transfer and release across execution paths, before replacing this gate.
  Preserve the independent check for storage that cannot prove those obligations.

- Call-origin proof summaries, 2026-11-04: the writeback and pending-request
  proof created one context per acyclic call path, so validation time grew
  exponentially with call depth; the nested Copy-transport PHP test did not
  finish in an hour. Andrew approved per-function summaries. Calls inside one
  strongly connected component of the call graph (closure calls link every body
  of the same function type) keep path contexts and ancestor folding. A call
  across components accumulates its entry like a path context, names caller
  storage and closure creators `Foreign`, keeps only reachable fields and
  selections, and binds to the summary for that canonical entry. Storage the
  callee creates is renamed below the call's frame, so names match path
  contexts. A summary exit keeps only storage its caller can name; selections
  stay whole. Only live contexts are scheduled, covered and checked. Inside a
  large recursive component the proof is still per path. Corrections in the
  same pass: borrowed match bindings of callbacks stay nonescaping (E0657
  regression), `direct_place_local` maps a re-wrapped narrowed nullable payload
  to its local as `direct_borrow_home_place` does, and a scheduling fixture
  uses its program's own enum layout. At
  `dev-3da659c29d5dc67cc1484c1247fc4393a4a83c8cd34102e86aeb04ba5729207e` on
  macOS ARM64 with LLVM: all 545 library tests pass in 153 s, including the
  nested-transport test (27 s); constructor-owned property 10/10, consuming
  receiver 27/27, Stage 35 contract 56/56, Stage 27 38/39, MIR validation
  124/125, returned-closure transport 77/78. The remaining failures are fixed
  in the next entry.

- Stage 36a corrections, 2026-11-05: retained-source validation gives a payload
  enum only the loans its fields can carry, whatever place it is read from, so
  a Copy enum holding a managed token has no receiver loan. The call-origin
  proof does not analyze the present edge of a presence test whose value is
  only null in that context, as it already skips the success edge of a call
  that never returns. Stored-owner validation takes selectors and contents
  from an already resolved dispatch or vtable candidate instead of expanding
  it again with the caller's receiver. Coalesce lowering uses its checked
  result type, so a present `?mixed` moves out as `mixed`: validation now lets
  an owned nullable mixed local transfer as mixed, which asserts presence as
  the borrowed form already did; both native backends share one slot and leave
  it absent. PHP treats plain string literals as pure operands and assigns a
  pure value to a stored property without staging its receiver. Five clippy
  lints and two test helpers were corrected without behavior change. At
  `dev-fb4bd71fcaa7c729f9bf034fec8e0438fa72f5efb7ce2875e06a8285e41844ef` on
  macOS ARM64 with LLVM: library 545/545, PHP codegen 142/142, Stage 27 39/39,
  returned-closure transport 78/78, MIR validation 125/125, constructor-owned
  property 10/10, consuming receiver 27/27, and both durable native parity
  tests (every manifest example and I/O fixture on interpreter, Cranelift and
  LLVM). Stage 35 contract passed 56/56 one source revision earlier; the later
  changes are PHP-only. The other 14 parity tests passed before the validation
  and PHP corrections. Clippy passes with warnings denied for the library and
  every test target. A probe build with E0572 lifted ran concrete, nested and
  multi-parameter generic payload enums natively and an owned callback payload.
  E0572 stays: a borrow-capturing closure cannot enter a payload (E0658, the
  open callable loan transport), and a generic body's `match (take ...)` on
  `Optional<T>` is rejected per Copy instantiation (E0600).

- Local enum loan transport, 2026-11-03: construction records retained
  values by authored case and bound field; match guards and arms project
  known payload facts instead of treating sibling payloads as one value.
  Joins retain partitions only when both inputs have them. Existing escape
  checks remain in force. 46 focused library tests passed on macOS ARM64
  with LLVM enabled at source identity
  `dev-79c97a6be4c0f8645c0ed1a63d8e1eef6da5ba0ac959f7c01cb2268fae494d50`.
  Aggregate caches were 4,058,427,392 bytes. Dedicated case-loan execution
  regressions, callable summary transport, and backend validation remain
  open; Linux and Windows were not rerun.

- Folded enum admission, 2026-11-03: checked constant, static initializer,
  parameter default and attribute paths validate folded generic enum
  declaration identity and nominal bounds, then record the concrete
  applications. Default and attribute evaluation also check the final folded
  type against the expected type. 46 focused enum and retained-loan library
  tests passed on macOS ARM64 with LLVM enabled at source identity
  `dev-65037299c6a96bdfa7d5449d1fdddd4dfffc3d24edad2c4944d4a357c83cd117`.
  Aggregate caches were 4,058,320,896 bytes. Linux and Windows were not rerun.
  Case-specific loan transport and public execution activation remain open.

- Generic enum constant evaluation, 2026-11-03: the early evaluator resolves
  enum declarations by identity and arity, uses shared argument inference,
  and uses the expected application for nested empty cases. Constants and
  attribute metadata retain resolved payload types. Six constant and nominal
  resolution tests and 17 attribute tests passed on macOS ARM64 with LLVM
  enabled at source identity
  `dev-8148a249e3ef12248aea5a49cdb97a8de8b53419bbbdaca16878b9be87a73681`.
  Aggregate caches were 4,058,169,344 bytes. Generic constant semantic
  admission, case-specific retained loans, and public execution activation
  remain open. Linux and Windows were not rerun.

- Shared type-argument inference, 2026-11-03: structural binding is independent
  of the semantic checker so the early constant evaluator can reuse the same
  nominal and nullable rules. Existing callable and enum expression paths use
  the extracted routine; constant schema integration remains open.
  On macOS ARM64 with LLVM enabled, 2 inference unit tests, 22 generic callable
  tests and 38 public enum tests passed at source identity
  `dev-971b995c51afe4d51f77ff02c1fc29762e111154a7cd66699bab61abfdd34819`.
  Aggregate caches were 4,055,818,240 bytes. Linux and Windows were not rerun.

- Concrete enum payload ownership, 2026-11-03: ownership checking uses the
  checked application and authored case to derive constructor parameter modes.
  Match cleanup uses the same Copy classification for selected payloads.
  44 focused library tests and 38 public enum tests passed on macOS ARM64
  with LLVM enabled at source identity
  `dev-bc46ec4640079fc7dd543e21e5448f180936db8071b95df5575324c8ea9402a2`.
  Aggregate caches were 4,055,322,624 bytes. Linux and Windows were not rerun.
  Generic execution remains gated pending constant inference and case-specific
  loan transport.

- Enum constant payload validation, 2026-11-03: the constant evaluator checks
  each evaluated field against the bound payload type before recording the
  case. A regression accepts a nullable field and rejects a different nominal
  enum with the same case name, using reordered named arguments. On macOS
  ARM64, 4 constant regressions, 38 public enum tests and 17 attribute tests
  passed with LLVM enabled at source identity
  `dev-693f26fdf3eaf88cb45e14f605af820f9429b79abdac5b1a722173119c343633`.
  Aggregate caches were 4,007,616,512 bytes. Generic constant inference
  remains open; Linux and Windows were not rerun.

- Enum retained-source classification, 2026-11-03: shared ownership analysis
  follows checked payload fields by exact enum application, including nested
  class and enum storage. Authored and expanded checking and native lowering
  use the same classification. A regression distinguishes loan-bearing
  `Packet<ByteRegion>` from `Packet<bool>` and checks transitive storage.
  All 100 focused library tests passed on macOS ARM64 at source identity
  `dev-2759e8bf1442a934c86c0df9e377cfb4d69c8f5f5d40d78f974bee9aecf92fd5`.
  Build and tests used 154.63 seconds; aggregate caches were 4,054,130,688
  bytes. Linux and Windows were not rerun. Case-specific loan transport,
  generic constant inference and execution activation remain open.

- Enum constant identity, 2026-11-03: constants and their types now retain the
  complete source enum application and authored case identity. Semantic typing
  no longer selects the first declaration-ID match. Native constants, defaults,
  static initializers and match operands resolve exact checked layouts; payload
  fields use checked substituted types. PHP constants, defaults and field
  transport consume the same checked applications and emitted symbols.
  Three new regressions cover distinct type arguments, declaration/layout ID
  separation, invalid cases/payloads and specialized PHP symbols. Final-source
  verification passes 187 distinct tests: enum library 32, public enums 38,
  attributes 17, scalar defaults 4, string defaults 4, further PHP value
  transport 32 and further conditional capabilities 60. macOS ARM64 execution
  fixtures cover interpreter, Cranelift, LLVM 18.1.8 and PHP 8.5.12. A new PHP
  assertion initially expected an unencoded symbol; correcting that fixture
  produced the passing final run. Formatting and whitespace checks pass.
  Tested identity: `dev-6937f2179836387c10e82850d83d08d53aaff1770ebf4d3a42d6bf5613f56878`.
  Library executable SHA-256:
  `0fde0aaa7b79ebd548b5a4ea9638de8a4ef3647aa3e9c51ea4225f23bdafcec4`.
  This work used 580.56 execution seconds including the diagnosed retry;
  cumulative allowance use is 2,248.70 of 3,600 seconds before the documentation
  check. Peak sampled group memory was 1,192,640,512 bytes; aggregate caches
  were 4,053,540,864 bytes. All build/test processes finished. This note is a
  documentation-only change after testing. Generic constant inference, retained
  loans and full backend activation remain open; E0572 remains. Linux, Windows,
  full workspace, Clippy and installed-tool refresh were not run in this pass.
  Tooling acknowledged full constant type identity and authored navigation
  identity in read-only coordination; no Tooling edits/builds are claimed.
  Invalidated elsewhere: editor constant typing/display now consumes full enum
  applications; constant diagnostics use source type names instead of enum IDs.
  Website versioned guide/API/playground and benchmark integration remain open.

- Generic enum construction and matching, 2026-11-03: shared argument inference
  now serves callable and enum construction records separately. Payload types
  bind first; result context fills missing arguments, including empty and nested
  empty cases. Matching substitutes authored payload types for concrete and
  symbolic applications. Unmaterialized applications no longer assume Copy;
  conservative ownership preserves their return-loan eligibility. Six new
  regressions cover inference, bounds, conflicts, nested context, renamed
  parameters and nonmaterializing payload queries. E0572 remains pending
  constant identity, retained-loan transport and complete backend activation.
  Final-source verification passes 257 distinct tests on macOS ARM64: enum
  library 29, public enums 38, generic functions 22, generic classes 41,
  closure ownership 35, further conditional capabilities 60 and PHP result
  transport 32. Existing execution fixtures cover interpreter, Cranelift,
  LLVM 18.1.8 and PHP 8.5.12; generic enum source execution remains gated.
  An initial diagnostic-caller build error and three incorrect new fixture
  assumptions were corrected; the borrowed-result regression then exposed
  and verified the shared ownership correction. Formatting and whitespace
  checks pass. Tested identity:
  `dev-f5be0d4f8e454fdd3c7fdba0ee10038e17a10b08f5f13007b4e6250377abe936`.
  Library executable SHA-256:
  `4fef4e3f379bd983ab497c50895050ddb2338bd0c281420b0ce99515a465af63`.
  This pass used 812.91 execution seconds, including diagnosed retries;
  cumulative allowance use is 1,667.14 of 3,600 seconds before the documentation
  check. Peak sampled group memory was 1,168,556,032 bytes; aggregate caches
  were 3,848,396,800 bytes. No verification process remains running. This note
  changes documentation only after testing. Linux, Windows, full workspace,
  Clippy and installed-tool refresh were not verified in this pass.
  Tooling acknowledged the construction-fact and substituted-match consumer
  requirements in read-only assessment; no Tooling edits/builds are claimed.
  Invalidated elsewhere: generic enum editor signatures and match hovers;
  website versioned guide/API/playground and benchmark integration remain open.

- Generic enum source-case identity, 2026-11-03: construction and match facts
  now carry authored declaration/case identities. Ownership and ambient-effect
  payload flows retain that identity; native lowering resolves it against the
  concrete enum metadata, and PHP field transport validates the same mapping.
  Two new regressions cover differing declaration/layout IDs and one authored
  case shared by distinct concrete applications. Current-source builds and
  188 focused tests pass on macOS ARM64: enum library 28, public enums 33,
  closure ownership 35, conditional capabilities 60 and further PHP result
  transport 32. Execution fixtures cover interpreter, Cranelift, LLVM 18.1.8
  and PHP 8.5.12; formatting and whitespace checks pass.
  Tested identity: `dev-5b1efab64898d14fde0233c9ad3964add9f386de7ea62f29b5724da79a112136`.
  Library executable SHA-256:
  `39b73a5bd5c994f935820601e44076162f4abe4093ffa687a9c85451dcc33c40`.
  This correction used 397.62 execution seconds; cumulative allowance use is
  853.29 of 3,600 seconds, peak sampled process-group memory 1,210,302,464
  bytes, and aggregate caches 3,851,669,504 bytes. All verification processes
  finished. This evidence update changes documentation only after testing.
  E0572 remains while generic construction inference, constant handling,
  matching payload substitution and retained-loan transport are completed.
  Linux, Windows, full workspace/Clippy and installed tools were not verified
  in this pass. Tooling acknowledged the authored/physical case contract in its
  read-only assessment; no Tooling implementation or build is claimed.
  Invalidated elsewhere: future Tooling semantic case-fact consumers; existing
  website versioned guide/API/playground and benchmark obligations remain open.

- Current-source verification approved, 2026-11-03: Andrew's approval
  supersedes the older pending 30-minute old-binary request. Approved scope is
  compilation plus focused enum, generic-class, conditional-capability and
  ownership regressions, including diagnosed correction/retest cycles. Limits
  approved: 60 minutes total execution, offline, one job, 3 GiB process-group
  memory and 4 GB additional cache storage within the aggregate 25 GB ceiling.
  Initial allocated caches: 3,498,192,896 bytes. A separate ledger records this
  allowance; the exhausted prior ledger remains unchanged.
  Current-source verification passed 260 distinct tests (266 executions, six
  overlapping specialization checks) on macOS ARM64: 130 library tests and
  130 public integration tests. The groups were enum 27, conditional capability
  60, specialization 17 (six repeated), PHP result transport 32, generic
  functions 22, generic classes 41, public enums 32 and closure ownership 35.
  Execution fixtures exercised interpreter, Cranelift, LLVM 18.1.8 and PHP
  8.5.12. One stale PHP result-publication argument caused the initial build
  failure; correcting that caller produced passing builds and tests.
  Tested identity: `dev-6a1cca38811945354cb53cb173a45e7e6222c75d47a79cbcf2a8a5717b121d26`.
  Library executable SHA-256:
  `9545d390be8443774c2750ba8f63a8cfda613b6fd86e8577952a17a6bb0ba523`.
  Build/test execution used 454.75 seconds, peak sampled process-group memory
  1,126,678,528 bytes, and 351,326,208 bytes of cache growth; final aggregate
  caches were 3,849,519,104 bytes. No build/test remains running. Ledger:
  `/private/tmp/doria-stage36a-generic-enums-20261103-budget.json`.
  This report is a documentation-only change after the tested identity.
  Linux, Windows, full workspace/Clippy, installed-tool refresh and generic
  enum source execution are unverified here. Next: complete generic case
  identity, retained-loan transport and backend activation under the accepted
  contract. No new approval is needed for those already-authorized source
  changes; further verification must use the remaining recorded allowance.

- Generic enum type groundwork, 2026-11-03: `EnumType` now carries typed
  arguments in resolved and interned representations. Shared substitution,
  nominal resolution inside enum arguments, symbolic detection, complexity
  accounting and display preserve those arguments. The existing nested
  type-constructor regression matrix includes enums; two added unit tests cover
  interning/identity round trips and nested nominal resolution. Existing
  nongeneric construction sites use the empty-argument constructor. Source
  activation remains held behind E0572 while
  case construction/matching, ownership transport
  and backend consumers are completed. These source edits compile and their
  focused regressions pass under the current-source verification above.
  Earlier binaries prove their recorded source states only. Tooling completed its read-only
  consumer assessment; no builds, installs, pin changes or publishing occurred.
  Invalidated elsewhere: compiler type consumers and Tooling specialization
  need verification; finish guide/API/playground and benchmark coverage remain
  open.

- Generic enum checker preparation, 2026-11-03: enum type-parameter scopes now
  cover name resolution and payload checking. Type applications retain their
  arguments and use shared nominal arity/bound checks; nested nominal formation
  visits enum arguments. Checker substitution and symbolic detection preserve
  them. Layout dependencies, cycle detection, Copy/drop/equality facts and
  checked enum lookup use the full source type. Shared call inference now visits
  enum arguments only for matching nominal declarations and arity; expected
  result types fill gaps without replacing argument-inferred bindings.
  Nine source regressions cover
  substitution, distinct concrete layouts, isolated recursive applications,
  namespaced parameter scopes, nominal bounds, constant declaration identity
  and call/context inference. Five inspect checker facts; four exercise public
  analysis behind E0572. Formatting
  and whitespace checks pass; current-source compilation and all nine
  regressions pass in the verification above.
  Case execution and retained-loan transport remain
  unfinished. Invalidated elsewhere: Tooling enum type/diagnostic consumers
  need verification; website guide/API/playground and benchmark work remain open.

- Generic enum metadata groundwork, 2026-11-03: checked enum metadata now
  separates materialized IDs from typed source identity and retains authored
  enum, case and payload locations. `enum_declarations` publishes templates
  independently of concrete `enums`; uninstantiated generic declarations have
  no executable layout record. Nongeneric instances receive dense physical IDs,
  while constant typing retains declaration identity. Regressions assert both
  metadata domains and their shared origins. Native/PHP type reconstruction consumes
  the recorded source identity rather than recreating an argument-free type
  from its display name. A new public-analysis regression checks origin
  preservation for a namespaced enum. Shared nominal resolution now keys enums
  by name and resolved arguments; native lowering maps full source types to
  materialized IDs, including after substitution. PHP field shapes use those
  IDs and preserve separate emitted names. Three further regressions cover
  distinct applications, absent applications and Copy/Move field selection.
  Source activation is still pending; E0572 remains.
  Formatting and whitespace checks pass; current-source compilation and the
  metadata/backend reconstruction regressions pass. Tooling's read-only assessment
  identifies specialization-aware lookup/signatures, shared authored navigation
  and rename origins, construction/match occurrences and editor fixtures as
  integration requirements. No Tooling implementation is claimed here.
  Invalidated elsewhere: semantic metadata consumers and backend reconstruction
  require verification; the existing website and benchmark obligations remain.

- Generic enum instance discovery, 2026-11-03: the declaration-owned dependency
  graph now admits class and enum applications together, with guarded callable
  dependencies, substituted stored types and shared expansion ancestry. Enum
  instances reuse typed identities, substitute payloads, preserve origins and
  receive dense layout IDs. Unused enum templates retain their dependencies.
  Four additional source regressions cover nested/repeated applications,
  class/enum expansion cycles, proof-only lookup and guarded availability;
  the public metadata fixture now checks concrete payload substitution.
  Formatting and whitespace checks pass; current-source compilation, the four
  regressions and affected class/conformance suites pass. E0572 remains until
  case execution and ownership transport are complete.
  Invalidated elsewhere: class specialization and conformance regressions now
  need re-verification alongside enum consumers and Tooling. Website versioned
  guide/API/playground and benchmark integration remain open.

- Conditional pending setup-failure regression prepared, 2026-11-03:
  `conditional_stream_forwarders_retire_prepared_writes_after_setup_failure`
  exercises two nested forwarding layers with one selected write followed by
  a throwing preparation. Assertions cover original error/progress preservation,
  no submitted writes, completion only for successful preparation, released buffer
  loans, owner reuse and exactly-once destruction. Interpreter, Cranelift, LLVM
  and PHP execution assertions are written but unexecuted pending verification
  allowance. Production behavior is unchanged; previous passing evidence does
  not cover this new fixture. Invalidated elsewhere: no new public contract or
  Tooling behavior; existing website and benchmark obligations remain open.

- Broader compiler-library checkpoint, 2026-11-03: the LLVM-enabled macOS ARM64
  suite completed 441 of 514 tests with no failures before the shared 60-minute
  allowance stopped execution. The interrupted test is
  `semantics::conditional_capabilities::tests::conditional_source_facts_share_public_finalization_and_execution`;
  it and the following 72 tests remain uncompleted. This is not a full-suite
  pass. Tested source was stable at
  `dev-a2d38c26dfcbfb950aebd7802ae98cb4612df39789ff8e199f833d4ec08f7588`;
  retained library executable SHA-256 is
  `f33f935c9c4cc54d3ed3427da9cdbd58e1f806953b1d4eab4cd9c80829746821`.
  An initial incorrectly labelled build was stopped before tests after 28.80
  seconds; the correctly labelled run consumed 599.76 seconds, with peak
  sampled process-group RSS 1,043,628,032 bytes. Cumulative guarded time is
  3,595.10 seconds; the guard reserves the remaining five seconds. Aggregate
  caches are 3,516,809,216 bytes. All processes are terminal. The remaining
  test inventory and exact executable identity are retained in
  `/private/tmp/doria-stage36a-library-20261103-remaining.json`; further execution
  needs a new scoped allowance and must not repeat the 441 completed tests
  without a relevant change. No production source changed in this checkpoint.
  Full workspace, fresh Linux/Windows, performance and integration gates remain
  open; existing Tooling and website/benchmark obligations are unchanged.

- Coordinated compiler/Tooling verification, 2026-11-03: caller-scoped nominal
  availability and static receiver types now survive source-to-executable
  finalization, including guarded bodies without executable specializations.
  Declaration inventories remain stable; proved, unproved and method-argument
  dependent conditions stay distinct. Metadata schemas 1-3 preserve authored
  generic names and composed callable source/package identity. Conformance-only
  conditions retain checked empty admissions; missing admission facts still fail.
  The first compiler pass had 254 passes and eight failures. Shared receiver,
  source-fact and metadata-origin corrections, plus valid project/borrow fixtures,
  produced 262 passes at
  `dev-2aa883d5c42755226fe0e67f7168adab806b049221d5667a75f1552aafd06b4e`:
  181 conditional-capability, seven metadata, 12 semantic IDE and 62 stream tests.
  Execution covers interpreter, Cranelift, LLVM and PHP with FFI disabled on
  macOS ARM64. Tooling then exposed the missing static receiver facts; after
  correcting that shared finalization, all 12 IDE and seven metadata tests passed
  again at `dev-1f12646440b1678e133bb09fb90ce160dc132376e7b309573244e9f0fb690f97`.
  The other 243 compiler tests were not repeated after this editor-fact correction.
  Semantic IDE artifact SHA-256:
  `d4f2ec0f4d255090955487d2830ac291310b6338e499ef4c6efad378e8257e53`;
  metadata artifact SHA-256:
  `42e2ae25e2320e491566f9fda37e36b9e8e16056829d4bc63b612289b66d23af`.
  Follow-on public stream acceptance adds four passing regressions without
  production changes: 70 nested-forwarder capability checks across ten concrete,
  erased and lookalike source types; nested concrete/generic/erased read, write
  and resumable flush execution; and real pending-read readiness, completion,
  resubmission, exhausted observation and returned-owner/storage reuse; and
  pending writes cancelled before preparation or after storage selection,
  exactly-once completion, rejected cancelled resubmission, independent later
  groups and successful write resubmission through the same nested wrapper.
  The first two pass at
  `dev-65a51eb7e4267af641b76b6d23689d35c9b848e9e1ad359166d1eb8f3fa0cadf`;
  the added pending-read test passes at
  `dev-0a6e4966965a914eb77c97d6b160a5f76edcc671659cf9d7a55fff65a5ffa6c3`,
  unchanged before/after execution. The pending-write/cancellation test passes at
  `dev-a03f5d473fa026848de988203450f3259a317b92dc8b988e33b3c0b45b400e45`,
  also unchanged before/after execution: one passed, zero failed, 65 filtered.
  It verifies exact callback counts, no cancelled writes, final `abbc` file bytes,
  and matched interpreter file-open/close counts. All four backends run on macOS
  ARM64; hosted pending execution uses PHP FFI. The nested execution test now
  also proves seven static forwarding bodies, one dispatch per erased caller,
  and no forbidden allocation/copy-helper references through native reachable
  calls. Its first structural assertion incorrectly counted only checked calls;
  execution passed, then that assertion failed. Counting ordinary typed outcome
  calls as well preserves exactly-one-target assertions. The corrected test
  passes at `dev-e187730d3d28d7839a75096306031995476ea068bbdc7085e6fa57056a632700`,
  unchanged before/after execution: one passed, zero failed, 65 filtered.
  Follow-on runs consume 259.55, 163.39, 163.70, 146.08 and 153.33 seconds.
  Including intervening documentation checks, guarded execution reaches
  2,965.89 seconds and 3,539,693,568 allocated bytes; peak sampled process-group
  RSS remains 1,273,135,104 bytes. Final stream artifact SHA-256:
  `6acdcb5da09c42dedfe9dec31ebf38a36024718e2e99643621590be336eb2c5e`.
  The original 62 stream tests were not repeated. This does not establish
  in-flight cancellation races, borrowed adapters, fresh portable coverage or
  the full performance gate. No public adapter is added;
  existing Tooling behavior and public metadata contracts are unchanged.

- Tooling coordinated checkpoint, 2026-11-03: the existing owner consumes the
  compiler's caller-scoped facts for completion, hover, signatures and navigation,
  removes obsolete E0776 expectations, and synchronizes flush enum/signature help
  and shared editor fixtures. Its first pass had 477 passes and three failures;
  an intermediate conditional retest had four passes and one failure. The final
  compiler correction and corrected navigation/diagnostic assertions produce
  480 passes: 303 library, three CLI and 174 protocol, zero failed or ignored.
  Tested Tooling source:
  `dev-6bc94dd6d474a34ea9ed660ab45776cb78b143227a22c204256647ca575b1996`,
  against compiler `dev-1f12646440b1678e133bb09fb90ce160dc132376e7b309573244e9f0fb690f97`.
  Only verification documentation changed afterward. Committed manifests, locks,
  compiler pin `f500042e1546a7282311423c31164131536ed1e1` and installed tools remain
  unchanged. No installs, dependency upgrades, Git writes or publishing occurred.
  No fresh Linux/Windows, full-workspace, Clippy, editor-build or performance
  evidence is claimed. All build/test commands are terminal. Cumulative guarded
  execution was 2,076.84 seconds of 3,600; peak sampled process-group RSS was
  1,124,106,240 bytes under 3 GiB. Aggregate allocated caches grew from
  2,649,001,984 to 3,545,104,384 bytes, below both the additional 8 GB allowance
  and aggregate 25,000,000,000-byte ceiling. Documentation-authority and stream
  completeness checks pass after these planning updates; scoped Rust formatting
  and whitespace checks pass. These documentation-only updates postdate the
  tested identities; compiler source and dependency hashes remain unchanged.

- Resumable flush verified surface, 2026-11-03: Andrew's approved amendment
  removes writable `flush(): void` and replaces it with `flush(): FlushOutcome`
  throwing `IoError`; ordinary inline Copy cases are `Complete` and `Pending`.
  The 62 stream tests above cover synchronous completion, partial drainage,
  repeated pending results, original checked failures, retained bytes and retry
  through concrete, generic and erased calls, plus allocation-free outcome
  transport. These are contract/regression fixtures; public buffer/text adapters
  and consuming finish remain open. Private device/file flushing and durable
  synchronization are unchanged. Invalidated elsewhere: Tooling source behavior
  is synchronized and tested, but installed refresh and pin alignment remain held;
  website versioned guide/API reference/playground and compiler-keyed benchmark
  evidence remain outstanding. Full Stage 36a remains estimated at 63% with
  implementation 50%, verification 30%, integration 20% weighting; scoped borrowing,
  separate native/bridge corrections, public adapters and full portable/integration
  gates remain open. This checkpoint does not close the full acceptance matrix.

- Consuming buffer finish direction approved, 2026-11-03: Andrew selected
  "A: Return completion or owned retry (Recommended)": one
  non-waiting attempt returning an owned completion/retry result: completion
  returns an owned source, while pending returns the wrapper with its retained
  bytes and position. Borrowed forms release their source loan on completion
  and retain it on pending. Checked failures retain ordinary consuming-call
  cleanup, without promised wrapper recovery. Waiting remains explicit and no
  new buffer copy is introduced. The direction is approved but unimplemented;
  source-extraction/destruction rules still need review. The subsequent result
  declaration approval is recorded below; borrowed fields and move-out remain
  separate.

  Concrete finish result review approved, 2026-11-03: Andrew selected
  "A: General generic finish enums (Recommended)" for ordinary generic payload
  enums `FinishOutcome<S, R>` with `Complete(S $source)` / `Pending(R $writer)`,
  and `BorrowedFinishOutcome<R>` with `Complete` / `Pending(R $writer)`.
  Decision 0114 now assigns generic enum execution to Stage 36a; semantic
  checking still rejects it until implementation. The approved shared path preserves inline layout,
  substituted Copy classification, case-sensitive retained loans and cleanup.
  A separate proposed extraction rule permits direct owned fields to move from
  exclusively owned closed, non-inherited classes without user destructors,
  with partial-owner checking and remaining-field cleanup. E0472 currently
  rejects property move-out under Decision 0122. Alternatives presented are
  compiler-known finish carriers and atomic take-and-replace with nullable
  source storage. Generic enum/result implementation is approved; field
  extraction remains pending and is not authorized by either finish approval.
  Invalidated elsewhere: compiler ownership/layout/dispatch and
  Tooling diagnostics/hover/completion, website versioned guide/API/playground,
  and allocation/cleanup benchmark evidence. Borrowed-storage review remains
  separate; these proposals do not change its pending status.

- Public conditional-capability handoff checkpoint, 2026-11-03: the compiler
  candidate removes temporary E0776 rejection and routes public analysis through
  shared checked finalization, admissions and HIR specialization. Lowering rejects
  missing admission facts. Public contract publication removes source-only trait
  view predicates; incoherent conformance closures no longer produce repeated
  missing-requirement consequences. Execution fixtures use `lower_source` so the
  resolved program and compiler-generated declarations match their semantic facts.
  The first public pass had 170 passing and 11 failing tests: three fixture setup
  failures, stale empty-fact expectations, source-view leakage and consequence
  diagnostics. The corrected source
  `dev-aa54f9056ab3ca1d098f43b0d9f68a1a75fcf17d4bd6963fab1e21b7c0788b3b`
  passes all 181 focused tests: 60 conditional library, 23 grammar, 38 body,
  32 coherence, 12 composition and 16 type-formation tests; zero failed/ignored,
  with 454 unrelated library tests filtered. Execution covers interpreter,
  Cranelift, LLVM and PHP on macOS ARM64, including widened virtual overrides,
  composed checked-error cleanup and writable borrowed getters. PHP fixtures
  disable FFI; no fresh Linux/Windows, full-workspace or performance pass is claimed.
  Source identity matched before/after execution. Library artifact SHA-256:
  `4951acc118cc42240115a576aaccbc363115c2fd41f9b290257c6f56827091b9`.
  Andrew approved an additional offline ten-minute, one-job, 6 GiB, 2 GB-growth
  verification budget after the earlier growth-limited attempt. The failed and
  corrected runs used 220.30 and 306.41 seconds, with peak sampled group RSS
  1,179,287,552 and 1,090,355,200 bytes. Aggregate allocated caches rose from
  1,541,177,344 to 2,705,559,552 bytes: 1,164,382,208 bytes growth, below both
  the approved growth budget and aggregate 25,000,000,000-byte ceiling. Both runs
  are terminal. The coordinated checkpoint above supersedes this evidence for
  admission, metadata and caller-scoped editor facts.
  Tooling completed read-only impact inspection: E0776/empty-fact assertions,
  hover wording, shared getter fixture and checked editor-feature coverage need
  coordinated updates. Andrew approved conditional-capability source-only
  synchronization on 2026-11-03, followed by the coordinated build/test approval
  recorded above. Tooling's current behavioral evidence is in that checkpoint;
  installs, committed pins, dependency upgrades, Git writes and publishing remain
  excluded.
  Full Stage 36a remains estimated at 63% using implementation 50%, verification
  30% and integration 20%; this focused checkpoint does not close the full matrix.
  Invalidated elsewhere: Tooling diagnostics, completion/hover/signature/navigation
  expectations, website versioned guide/API-reference/playground and compiler-keyed
  Benchmark evidence. Existing scoped-borrowing and separate native/bridge decisions,
  remaining adapters and full integration/portable gates remain open.

- On 2026-11-03 Andrew approved coherence obligations carried through generic
  callers and resolved after substitution (Decision 0139), and general scoped
  class borrowing as the adapter ownership direction (Decision 0110). The latter
  still requires review of its concrete declaration/storage/escape contract;
  the constructor-promoted readonly/writable borrowing proposal has been sent
  for that review and is not implemented.
  M-C and R-S corrections are approved and implemented; their verification
  checkpoint below supersedes the earlier permission holds. A separate checked
  virtual-call provenance correction now awaits approval. Callable coherence
  propagation is implemented with the checkpoint below; remaining coherence
  and Stage 36a work can continue independently.

- Andrew approved adapter strategy A on 2026-10-31: "A: Conditional nominal
  capabilities (Recommended)". Decision 0139 records the general conditional
  conformance/constrained-member direction. Andrew subsequently answered
  "Approve local-clause contract (Recommended)", accepting contextual
  `where (...)` clauses, computed-property boundaries and inheritance/coherence
  rules. Compiler grammar, declaration-only condition resolution and the
  positive nominal proofs, declaration availability implication and guarded
  body/caller-condition checks, trait-predicate normalization and composed
  class/free caller checks with authored loan/effect summaries and unambiguous
  imported trait-member caller checks and caller-local conformance views with
  declaration-owned source duplicate checks and concrete target/storage closure
  are implemented. Ordinary and guarded programs now share callable-contract
  refinement for concrete return loans, retained-source graphs and authored
  callback effects. Shared source ownership checks now cover authored class and
  trait bodies, composed taking calls and retained-source storage. Ordinary and
  conditional paths also share constructor and post-ownership checks, including
  borrowed getters, pending-request escape and nonblocking phases. Nominal source
  requirement graphs are prepared by shared callable validation before ownership
  and retained-source refinement; they remain distinct from active witnesses.
  Guarded generic methods and trait aliases preserve owned Copy snapshots and
  borrowed erased Move results, rejecting escaping temporary loans. Symbolic-self ownership
  no longer treats trait/interface `self` as Copy before
  substitution, preserving composing-receiver cleanup. Guard-aware
  dependency admission now uses concrete nominal proofs and finite-root ancestry;
  declaration storage inventories remain independent of executable registration.
  Concrete ownership and returned-loan checks now use authored composition
  templates, including renamed method parameters, nested aliases, fixed trait
  arguments and composing receivers. The final checking-only pass collects
  guarded concrete roots; the public path now uses checked finalization.
  Recorded generic-call and body-local class dependency traversal now uses the
  same authored mapping, as do constrained methods/getters and core operations;
  effective roots retain admission and finite ancestry.
  Authored instance/static targets now retain generic relay dependencies. Live
  source return contracts and effective getter identities preserve composing
  receivers and imported aliases' ownership modes.
  A checked-admission inventory and shared native/PHP specialization consumer
  support the public conditional path, including nongeneric witness validation.
  Source checking now publishes the exact generic-call closure for shared
  native/PHP materialization, including virtual overrides, initializer factories
  and global calls. PHP global emission consumes specialized call-site plans and
  the selected MIR entry owner.
  Source-owned physical virtual families now reserve inactive/unmaterialized
  entries independently of body admission. Explicit MIR class views preserve
  the actual static receiver domain and its ownership/storage facts.
  Source-owned interface witness templates now fix concrete owner bindings,
  declaration identities, requirement origins and exact-self proofs for shared
  native/PHP materialization and PHP ownership/value-layout coordination.
  Full specialized provenance/ownership, composition/specialization coherence
  and member/witness execution acceptance remain pending beyond the focused
  public-path checkpoint above.
  The master plan assigns
  named grammar and execution slices before generic adapter execution.
  Decisions 0110 and 0134 link the accepted direction.
  The earlier `fe59f5ed` checking-only probes still establish the current model:
  direct StandardInput readiness is accepted, SyncForwarder<StandardInput>
  fails E0535, and an explicitly pending-capable wrapper is accepted. These
  are not adapter execution or performance evidence. The independent recursive
  PHP transport checkpoint below does not implement conditional capabilities.
  All prior work is preserved.
  Full Stage 36a is estimated at 63% under unchanged scope
  and weights, with no delivery claim. Installs, pins, staging, commits and
  publishing remain held.
  Invalidated elsewhere: the unresolved A/B question is replaced by A; the
  accepted contract identifies compiler/metadata, Tooling, SPEC/agent guidance,
  website versioned guide/API-reference/playground and Benchmark work required
  before delivery. The local-clause design review is complete; full execution
  acceptance and integration remain pending.

- Virtual-contract inspection checkpoint: the explicitly selected compiler
  `dev-550105f9171029b35e331a791ee49942c3a731db09538fc210414b511d4c94cd`
  source-checks all four new fixtures without diagnostics. MIR accepts required
  error narrowing with incompatible checked entry profiles and assigns ambient
  I/O to a pure direct parent implementation. Readonly-over-writable receiver
  weakening fails with B0001; class-to-interface return covariance fails with
  M1101. The latter fixture covers closed and open concrete returns; lowering
  stops at its first adapter. Decisions 0130/0134 already accept these contracts.
  Four enabled Rust regressions now cover the physical hidden-input/status
  contract across seven return shapes, exact-parent profiles, receiver safety
  and interface/class views. They have not been compiled or executed. The prior
  429-unit/317-integration checkpoint predates these additions; no production
  compiler code changed. Correction belongs to shared source-owned call facts,
  MIR contracts/adaptation and native consumers before conditional activation.
  No native programs, Docker sessions, installs, pin changes, Git writes or
  publishing were started. Stage 36a remains estimated at 63% under unchanged
  scope and weights. Invalidated elsewhere: compiler-keyed Tooling/Benchmark
  evidence and website versioned guide/API-reference/playground verification.
  Tooling pin, strings and hovers remain unchanged; existing holds remain.

- Receiver-contract verification: semantic checking and MIR now share one
  receiver-mode representation and substitution rule. The checked source slot
  plan retains each selected declaration's receiver contract. MIR carries both
  the canonical root bound and the selected bound independently of executable
  body admission, checks inherited bounds even through absent root/intermediate
  bodies, and requires active entries to match their selected contract.
  Removed MIR rejection of valid readonly-over-writable receiver overrides.
  Removed method-contract acceptance of mismatched static/instance receiver
  presence. Nonreceiver parameter modes/types/ownership remain strict.
  Four additional enabled Rust tests cover the nine mode pairs, inactive roots,
  inactive narrowed intermediates, consuming/strengthened forgeries and ordinary
  parameter ownership; the existing parity regression also checks the root bound,
  instance-field access and exact child/root destruction trace `77cr`.
  `dev-2986bf37258ecc48aef32405c462558fb8fe45d9ca77af2bcfc94228f78fabde`
  passes all five exact tests on macOS ARM64/PHP 8.5.12; the four execution cases
  cover interpreter, Cranelift, LLVM and PHP with FFI disabled. The full 438-test
  harness was compiled, but only these five tests ran. Source remained unchanged
  across the run. The strengthened fixture initially failed E0631 because its
  destructor output did not catch I/O errors; the fixture now catches `Error`
  locally, preserving the expected trace and the nonthrowing destructor rule.
  The successful bounded run took 91.96 seconds, sampled peak group RSS
  1,076,363,264 bytes, and grew allocated caches by 194,879,488 bytes. Earlier
  production LLVM library checking passed before this fixture-only strengthening.
  No fresh full suite, strict Clippy, Linux, native Windows or PHP 8.4 run is
  claimed. Code audit: 2,081 files, SHA-256
  `4267b72255e18b6fb4ff7465860b2821415ea9f9adfc1275d939ab805b855a7f`.
  The following checkpoint verifies required-error transport. Exact-parent
  ambient profiles and interface-return adaptation remain pending. Stage 36a remains
  estimated at 63% under unchanged scope and weights. Invalidated elsewhere:
  compiler-keyed Tooling/Benchmark and website guide/API-reference/playground
  verification; no new Doria-visible API or versioned metadata schema is added.

- Virtual checked-transport correction: canonical MIR families retain a
  root-owned transport flag independently of the selected implementation's
  logical error profile and inactive body admission. The shared native callable
  plan owns status/result/error inputs; Cranelift and LLVM use existing stack
  layouts for logically infallible calls through checked families. Direct
  implementation clones keep their own profiles. Return adapters preserve
  implementation logical effects, and MIR checks reachable entries against each
  static callable's nominal Error contract. Removed per-override selection of
  virtual checked transport, adapter widening of implementation error profiles,
  and MIR acceptance of entries that widen a static checked-error promise.
  `dev-ec23b70488587837eaf387bda6a6714d97b62f2b317b87ca84da1f16c5b8edc6`
  passed 35 exact tests before a new fixture failed source checking: the Error
  subclass incorrectly assumed inherited construction. The fixture now declares
  its parameter-only constructor and first parent call; expected output is
  unchanged. Both nominal fixtures passed a pre-existing CLI source-check
  prerequisite before rebuilding. Production sources and the 35 passing cases
  remained unchanged; only that fixture, a new Error-subinterface case and the
  six unexecuted checks ran next. All eight passed at
  `dev-ac24b23c769d3d1fe2fea9390d6a9b558f54caebbc246857cbfd47800a3b75ea`.
  The combined 43 passes comprise 42 monomorphization cases and one native
  signature-ordering unit, with no ignored tests. The current 442-test library
  harness compiled; the full suite and strict Clippy did not run. The two known
  pending exact-parent-ambient and interface-return tests remain enabled and
  outside this selection. macOS ARM64/PHP 8.5.12 execution covers interpreter,
  Cranelift, LLVM and PHP with FFI disabled, including seven result shapes,
  inactive roots, owned cleanup and nominal Error classes/subinterfaces.
  No fresh Linux/native Windows/PHP 8.4 run is claimed. The final run took
  105.26 seconds with sampled peak group RSS 1,127,596,032 bytes and allocated
  cache growth 197,005,312 bytes. Code audit: 2,083 files, SHA-256
  `e4d784b2e0691431faaa690ad1964c8e719865192b1ae3b3679d3f573cc8b04a`.
  Conditional activation remains under E0776. Stage 36a stays estimated at 63%
  under unchanged scope and weights. Invalidated elsewhere: compiler-keyed
  Tooling/Benchmark evidence and website versioned guide/API/playground
  verification; no new public API, runtime layout or versioned metadata schema.

- Exact/virtual ambient-profile correction: checked source inference now retains
  separate implementation and virtual-descendant profiles. Exact parent, closed
  receiver and direct lifecycle facts feed generic dependencies, property hooks,
  nonblocking analysis and native/PHP call selection. Native direct bodies are
  lowered under exact signatures; PHP emits distinct implementation entries
  through its existing argument/result/ownership machinery. Removed family
  effect contamination of exact bodies, cloning virtual-profile CFGs into direct
  entries, native-side lifecycle inference and PHP host-virtual dispatch for
  checked exact calls. The first compile found two stale property-hook fields;
  after correction, 38 tests passed before PHP lifecycle output diverged. The
  shared PHP correction then required one Rust emission-borrow fix.
  `dev-f803ef066d340a965fa1d33c01013a17993873173f764f3992860dff349e7101`
  passes all 46 selected tests: 45 monomorphization cases and one signature-order
  unit, with no ignored cases. The 444-test library harness compiled; its full
  suite and strict Clippy did not run. New regressions cover exact versus virtual
  scopes, catches, lifecycle calls, owner/method generics, traits and callbacks,
  including missing-profile rejection. Execution covers interpreter, Cranelift,
  LLVM and PHP 8.5.12 with FFI disabled on macOS ARM64. No fresh Linux, native
  Windows or PHP 8.4 execution is claimed. Production/test source stayed unchanged
  across the successful run: 2,085 code files, SHA-256
  `77825617fcf1728acb21cb14274d9213814f9290a4a88e3e39adeeb6ca2d5eed`.
  The run took 158.27 seconds, sampled peak group RSS 1,166,655,488 bytes and
  grew allocated caches by 195,346,432 bytes. Interface-return adaptation,
  broader regressions and portable execution remain pending; E0776 stays active.
  Stage 36a remains estimated at 63% under unchanged scope and weights.
  Invalidated elsewhere: compiler-keyed Tooling/Benchmark evidence, generated PHP
  and website versioned guide/API-reference/playground verification. No new
  public API, runtime layout or versioned metadata schema; pin/install/Git and
  publishing holds persist.

- Shared return-adaptation checkpoint: MIR root families now retain physical
  return/borrow contracts, while selected slots retain logical promises through
  inactive bodies. Lowering and validation share typed value factories, nominal
  conversions and one adapter constructor. Validation rejects inherited widening,
  changed implementation effects, swapped bodies and internal environment
  parameters. Removed class-only return conversion/projection, independent
  interface-entry erasure, primitive constraint-only return erasure and manual
  class-word checked-result scratch sizing. Native backends reuse storage
  projection; LLVM normalizes payload-enum expression/storage carriers and
  forwards borrow homes through class upcasts. Inline nominal tests preserve
  target identity and witness-based presence, including zero-valued StandardInput.
  Integration found moved-helper references, missing shared nullable-enum
  classification, LLVM aggregate/provenance boundaries and nominal-test gaps;
  these were corrected. The new fixture removed float display and instead checks
  returned float equality, preserving transport coverage without changing the
  documented PHP B1301 limitation. Collection/inline native checks moved to an
  independent enabled test; the interpreter/PHP check remains enabled.
  `dev-662da81891e5c777435d912dcbd73625550c3414fb469aac2fa4bfd9bc8a4a6e`
  ran all 59 selected tests: 58 passed, none ignored; it exposed PHP's rejection
  of Child's array return beneath Root's Iterable return. The 451-test library
  harness compiled; the full suite,
  strict Clippy, integration-target rebuild and fresh portable runs did not run.
  That batch took 179.70 seconds, peak sampled RSS 1,032,880,128 bytes, with
  unchanged code SHA-256
  `51ef7fd1485358df741cf8c94937d9df66d88b451bba86eff2bc7dd56cbf4d4b`.
  The subsequent internal-parameter guard at
  `dev-487a0d64f4aeaaafface463788a6703c0e118ec27b3fef0d5e09a983c4fc4505`
  passes four focused checks in 105.09 seconds, peak RSS 1,254,047,744 bytes;
  2,091 code files stayed unchanged, SHA-256
  `d12facffc7d9141b4ce951451b56a51c74a7adb72348af0e9ac6979ab8f2d461`.
  The documentation guard then caught unnecessary explicit main clauses in two
  new fixtures. Only those clauses were removed; required ordinary contracts and
  the existing explicit-main compatibility coverage remain. Both fixtures pass
  at `dev-00b5c8af986f4f269e3b6ab4fa16292b437f4ff7dc8bab373d6cf806d908d535`
  in 90.34 seconds with 1,056,292,864-byte peak RSS. Their unchanged-code audit is
  `a8cc296426eeb968ce2637da32c8905725ed5ac27199bd2f8e2bcb4561d23e5e`.
  Execution covers macOS ARM64 interpreter/Cranelift/LLVM and PHP 8.5.12 with FFI
  disabled; no fresh Linux/native Windows/PHP 8.4 or leak-check pass is claimed.
  Ten exact-plan obsolete-only cleanups each preserved all 30,676 retained-file
  observations; reviewed full snapshots were retired with checksums retained.
  This does not reconstruct the earlier transport preservation anomaly.
  Typed PHP caller views now consume checked families and nominal witnesses,
  retaining exact return annotations, borrowed collection storage, computed
  getters, root-owned defaults and core iteration/display entries. Removed PHP
  host-method covariance as the owner of these conversions. Generation without
  native MIR remains supported. Shared direct inline-to-interface tests now
  preserve zero payloads, nullable presence and exactly-once evaluation.
  `dev-41c38c48f1324bf7857807e3e32f6bc5c95481576997eef3ac6a66478e591fc4`
  passes all 62 selected checks, none ignored, in 187.60 seconds with peak sampled
  RSS 1,113,227,264 bytes. Its 2,094 code files stayed unchanged, SHA-256
  `797a3f7629b3b26b513dab450d522527d94e698f161dcfc00de0f481a81443eb`.
  A protocol fixture initially violated existing interface/default rules; only
  that fixture was corrected, preserving root-owned default coverage. The
  inherited-witness/default/iteration/display check then passes at
  `dev-0ec02e3f7e642ea75e9e6ae31478e75f64576084fab2540d9e69925c117cad08`
  in 88.54 seconds, peak RSS 1,123,368,960 bytes, with 2,095 unchanged code files,
  SHA-256 `a8510df70d7368ad8f20bbcc371179f5ca72e56da2250be8a9a0ac08af084684`.
  The 455-test harness compiled; the full suite, strict Clippy and fresh portable
  checks remain unrun. Four subsequent obsolete-only cleanups each preserved all
  30,676 retained-file observations. An intervening plan was safely rejected:
  six candidates and one retained runtime object changed allocation blocks only;
  complete changed records were retained, but the cause is unverified. A fresh
  exact plan succeeded with unchanged retained observations. Final allocated
  caches are 24,775,774,208 bytes: Doria 24,611,151,872 and Tooling 164,622,336,
  below the aggregate ceiling. No healthy caches, installed tools, source or user
  changes were removed. Builds are stopped.
  Stage 36a remains estimated at 63% under unchanged scope and weights; E0776 and
  pin/install/Git/publishing holds remain. Invalidated elsewhere: compiler-keyed
  Tooling/Benchmark evidence, generated PHP and website versioned guide,
  API-reference and playground verification; no new public API or versioned schema.

- Conditional ownership checkpoint: ordinary/guarded callable-contract checks
  share one phase. Function-value signatures preserve symbolic ownership and
  distinguish borrowed results. Retained-source, callback, pending and writeback
  analysis now share one function/class/trait/accessor inventory. Checked body
  templates refine composed summaries before caller diagnostics, with distinct
  receiver storage homes and recursive effective-to-authored dependencies.
  This removes the lost-loan path through nested trait aliases; unsafe mutation
  is rejected. Opaque summary facts remain separate from known-body evidence.
  The first 37 focused checks passed. The full 461-test library run at
  `dev-f934845f315b60d7b51ba271cb1d9c1aa1a99ebb4f7cb93f67144167e9a9e3db`
  passed 458, failed three, ignored none, in 493.80 seconds, peak sampled RSS
  1,154,613,248 bytes. The failures were the unchanged V-A native match merge,
  a new invalid trait-constructor fixture and an older admission fixture missing
  its required parent call. Both fixtures were corrected without changing their
  test names or narrowing the checks. The final knowledge guard and 58 focused
  checks pass at `dev-f0fc102ee622abcab0964ed0359f8e09f8156e84cdcac9d37df5c2e44846b616`,
  none ignored, in 102.44 seconds, peak RSS 1,247,903,744 bytes. These include both
  fixtures, receiver-specific property types, recursive callback writeback and
  all 18 nominal-proof units. Its 462-test harness compiled; the full suite was
  not rerun after the guard/fixture edits. Code audit: 2,095 unchanged files,
  SHA-256 `88cdebba95758ef69fd6ced4ab3ef1f35a2530e4c3cb1b4ba06a9a6a080181ea`.
  V-A remains enabled and unfixed. Strict Clippy, integration-target rebuild,
  fresh Linux/native Windows/PHP 8.4, leak and allocation gates did not run.
  Three obsolete-only cleanups each matched all 30,676 retained-file records;
  owned full observations were retired after preserving compact proofs.
  Latest allocated caches: 24,766,472,192 bytes, Doria 24,601,849,856 and Tooling
  164,622,336, below the aggregate ceiling. No healthy caches, installed tools,
  source or user work were removed. Builds are stopped. Full Stage 36a remains
  estimated at 63% under unchanged scope and weights; E0776 and existing holds
  remain. At that checkpoint, shared post-ownership property-return and
  pending/nonblocking checks were still open; the next checkpoint records their
  correction. Remaining specialized provenance, coherence and executable
  witnesses remain open. Invalidated elsewhere: compiler-keyed Tooling/Benchmark evidence and
  website versioned guide/API-reference/playground verification. No public API,
  syntax, schema, pin, install, Git ref or publication changed.

- Conditional post-ownership checkpoint: constructor initialization, canonical
  binding/accessor publication, borrowed-getter returns, pending-request escape
  and nonblocking checks now share one phase with ordinary checking. Composing
  classes own initializers and partial cleanup. Authored call contexts preserve
  effective receivers, aliases, callback environments and generic bindings.
  Known blocking work is checked without concrete callers; type-dependent
  release obligations remain checked at executable instances. Declared nominal
  paths identify pending phases by canonical requirement origin without
  publishing an unconditional witness. Lookalike phase names remain ordinary.
  All 471 library tests ran at
  `dev-a9ecfd3dd500cebe08946b595f763b2120f984f85814c89918bf6bff99aa27c3`:
  470 passed, one failed, none ignored, in 534.54 seconds, peak sampled RSS
  1,110,769,664 bytes. The sole failure is the unchanged, enabled V-A native MIR
  match-result merge; PHP matched its exact cleanup output. All 67 focused checks
  passed, including read/write preparation/completion, direct/composed guards,
  pure aliases, deferred callbacks, generic cleanup, request escape and partial
  constructor cleanup. New invalid fixtures/setup and an older uninitialized
  field fixture were corrected without renaming tests or weakening assertions.
  All 2,095 code files stayed unchanged during verification, SHA-256
  `aa7175ec7c397a7fa2cd78532711e3412b6fea324e3d7c6ddefc347d71b170d7`.
  Environment: macOS ARM64, Rust 1.98.1, backend LLVM 18.1.8, PHP 8.5.12.
  Fresh Linux/native Windows/PHP 8.4, strict Clippy, integration-target rebuild,
  leak checks and the full allocation/performance gate did not run. Three
  exact-plan obsolete-only cleanups preserved all 30,676 retained-file records;
  owned full observations were retired with compact checksums retained. Final
  allocated caches are 24,770,662,400 bytes: Doria 24,606,040,064 and Tooling
  164,622,336. Healthy caches, installed tools, source and user work were preserved.
  Builds are stopped. Tooling's existing source/hover facts remain consistent;
  no strings/hovers, pin, install, Git ref, schema or publication changed. Full
  Stage 36a remains estimated at 63% under unchanged scope and weights. E0776,
  coherence/specialized-provenance work, executable activation and integration
  remain open. Invalidated elsewhere: compiler-keyed Tooling/Benchmark evidence
  and website versioned guide/API-reference/playground verification.

- Symbolic-self cleanup checkpoint: the shared ownership classifier now retains
  trait/interface `self` as symbolic until substitution. The former Copy
  classification omitted a composing receiver's destructor obligation. The
  function-value signature regression now covers both symbolic self forms.
  Returned generic callbacks distinguish immediate cleanup from escaping owned
  captures; region copies keep their original byte-storage loan after descriptor
  rebinding. Generic request captures already received E0772 correctly and now
  have direct/composed regression coverage. Pure and rejecting controls remain.
  The initial 70-check run exposed the self-cleanup failure plus invalid capture
  and import fixtures; the fixtures were corrected without narrowing assertions.
  All 71 selected checks pass at
  `dev-821c115a2d85c9c8cec2a6d8edcc6c3d90f05f6d10833038fb660eb04d778451`,
  none ignored, in 108.29 seconds, peak sampled RSS 1,110,032,384 bytes. All 2,095
  code files stayed unchanged, SHA-256
  `c801425e960ad5a4be95cda162793c8ce7143250ca9a6b4c7cd469a738ba8640`.
  The 475-test harness compiled; the full library was not rerun after this
  correction. The preceding full 471-test result remains separate. No fresh
  portable, strict-Clippy, integration-target, leak or full performance gate ran.
  Two exact-plan obsolete-only cleanups preserved every one of 30,676 retained
  records; full observations were retired with compact proofs. Final allocated
  caches are 24,781,836,288 bytes: Doria 24,617,213,952 and Tooling 164,622,336.
  No healthy caches, installed tools, source or user work were removed. Builds are
  stopped; E0776 and all existing holds remain. Full Stage 36a stays estimated at
  63% under unchanged scope and weights. No syntax, public schema, Tooling text,
  hover, pin, install, Git ref or publication changed. Invalidated elsewhere:
  compiler-keyed Tooling/Benchmark evidence and website versioned guide,
  API-reference and playground verification.

- Inherited cleanup and lifecycle checkpoint: partial-construction cleanup now
  selects physical properties through specialized ancestor views. Direct and
  transitive generic parents preserve scalar/collection destructor obligations.
  That correction alone left a pure parent handoff incorrectly rejected E0770:
  omitted lifecycle annotations published an unknown owned result. Canonical
  lifecycle signatures now publish `void`, matching explicit annotations and
  the existing lifecycle contract. Ordinary unknown-return recovery is unchanged.
  Abstract nominal read/write forwarders also have accepting preparation/
  completion and rejecting synchronous-I/O controls. The intermediate full run
  passed 475 of 477 tests; the final run executed all 478 at
  `dev-a65bdb4cb378205f20ad4038e9abd8f97d37f1ea9d532daa56dae96165e88021`:
  477 passed, one failed, none ignored. All 74 focused checks passed. The sole
  failure remains enabled V-A: PHP matches exact cleanup output; native MIR
  reports B0001, "match arm terminates before assigning and merging its result."
  The final run took 530.11 seconds, peak sampled RSS 1,174,601,728 bytes. All
  2,095 code files stayed unchanged, SHA-256
  `dd98074882e1c74c136f90d404558e31ff1aeb7870286b4fdee0721155aa8e8c`.
  Environment: macOS ARM64, Rust 1.98.1, backend LLVM 18.1.8, PHP 8.5.12. Fresh
  Linux/native Windows/PHP 8.4, strict Clippy, integration-target rebuild, leak
  checks and the full allocation/performance gate did not run. Four obsolete-only
  cleanup applications retained 30,676 files each. The first had one retained
  object's allocated-block count change with all other recorded fields stable;
  its current bytes match both retained archives. The cause remains unverified,
  and no historical byte identity is claimed. Subsequent retained records all
  matched. A later plan was rejected before deletion after three candidates'
  block counts changed; a fresh exact plan was reviewed and applied without
  weakening checks. Owned full observations were retired with compact proofs.
  Final allocated caches are 24,783,519,744 bytes: Doria 24,618,897,408 and Tooling
  164,622,336. Builds are stopped. No healthy cache, source, user work or installed
  tool was removed. No syntax, public schema, Tooling text/hover, pin, install,
  Git ref or publication changed. Full Stage 36a stays estimated at 63% under
  unchanged scope and weights. E0776, coherence/specialized-provenance work,
  active execution and integration remain open. Invalidated elsewhere:
  compiler-keyed Tooling/Benchmark evidence and website versioned guide,
  API-reference and playground verification.

- Source interface-graph checkpoint: a new guarded generic-erasure regression
  found an erased Move return classified as owned despite its correct parameter
  return-loan signature. Checking-only conditional ownership lacked the canonical
  interface requirement graphs prepared by ordinary final analysis. Shared
  callable validation now prepares those graphs before source ownership and
  retained-source refinement, including their nominal types, requirement loans
  and pending boundaries. Removed the erroneous owned classification of these
  borrowed interface results. The regression covers direct methods and nested
  trait aliases: Copy snapshots remain independently owned, erased Move results
  remain borrowed, and nested calls/closures cannot return temporary carrier loans.
  The initial 75-check run passed 74 and exposed this failure; the diagnostic run
  confirmed that the method signature was already correct. All 479 library tests
  then ran at
  `dev-295b6a283503d1d7f4978cadbf44e2e1854ec5dd0f33c5ba5f9ec2118bdeb388`:
  478 passed, one failed, none ignored; all 75 focused checks passed. The sole
  failure remains V-A's enabled native B0001 match-result merge rejection; PHP
  matched its exact cleanup output. Duration 555.65 seconds, peak sampled RSS
  996,737,024 bytes. All 2,095 code files stayed unchanged, SHA-256
  `a195d16cd59241506c060e8388839bada1678f26fcdb3af66067a1d03841dd3f`.
  Environment: macOS ARM64, Rust 1.98.1, backend LLVM 18.1.8, PHP 8.5.12. Fresh
  Linux/native Windows/PHP 8.4, strict Clippy, integration-target rebuild, leak
  checks and the full allocation/performance gate did not run. Three exact-plan
  obsolete-only cleanups preserved all 30,676 retained-file records each; owned
  full observations were retired with compact proofs. Final allocated caches are
  24,768,450,560 bytes: Doria 24,603,828,224 and Tooling 164,622,336. Builds are
  stopped. No healthy caches, installed tools, source or user work were removed.
  No syntax, public schema/API, Tooling text/hover, pin, install, Git ref or
  publication changed. Full Stage 36a remains estimated at 63% under unchanged
  scope and weights. E0776, coherence, remaining executable ownership/provenance,
  active members/witnesses and integration remain open. Invalidated elsewhere:
  compiler-keyed Tooling/Benchmark evidence and website versioned guide,
  API-reference and playground verification.

- Erased iterator source-loan checkpoint: a new regression checks the existing
  Decision 0134 iterator facility through guarded generic methods and nested
  trait aliases. Concrete and erased inputs preserve cleanup-lifetime source
  loans through direct, helper and callback forwarding. Source mutation remains
  rejected before and after the result's last explicit use; escaping a local
  source remains rejected. All 76 selected checks pass at
  `dev-8adaee74ef2abff554665cad71a09a63f2239f2a16786ca63976ae9cbbfc240a`,
  none ignored, in 116.39 seconds, peak sampled RSS 1,187,479,552 bytes. The
  480-test harness compiled; the full current library was not rerun. Production
  code is unchanged from the preceding full 479-test checkpoint. All 2,095 audit
  files stayed unchanged during this run, SHA-256
  `deeb14974c6d23e11111e45b8472c4939f52c462aa8ac9803e80b739dccbce41`.
  An exact obsolete-only cleanup removed 16 stale test objects and preserved all
  30,676 retained-file records. The full owned observation was retired with its
  compact proof. Final allocated caches are 24,783,409,152 bytes: Doria
  24,618,786,816 and Tooling 164,622,336. No healthy caches, source, installed tools
  or user work were removed. Builds are stopped. No fresh portable, strict-Clippy,
  integration-target, leak or full performance gate ran.
  Separately, inspection confirmed the borrowed-adapter design gap: Decision
  0110 requires those forms, while E0761/Decision 0134 currently permit only
  readonly retained source promotion in Iterator constructors. The general scoped
  class-storage versus compiler-known carrier strategy is awaiting Andrew's
  ruling; no implementation permission is inferred. This regression does not
  implement borrowed stream adapters or general borrowed fields. Full Stage 36a
  stays estimated at 63% under unchanged scope and weights. E0776 and all existing
  holds remain; no public API, Tooling text/hover, pin, install, Git ref or
  publication changed. Invalidated elsewhere: compiler-keyed Tooling/Benchmark
  evidence and website versioned guide/API-reference/playground verification.

- Checked conditional handoff checkpoint: actual checked source now exercises
  ordinary final semantic assembly through a test-only entry. The first smoke
  failed because concrete conformances only cloned symbolic templates; no broader
  tests ran in that attempt. Removed template-only concrete materialization;
  symbolic and concrete instances now share nominal closure and requirement
  checking. The regression covers direct and nested-trait methods/getters,
  active/inactive witnesses, attributes, constant-error blocking and public E0776.
  All 481 library tests then ran at
  `dev-7d093994e91f482a243f03fdb507b4defab76b1d038907013db4df3b8f29504e`:
  480 passed, one known V-A failed, none ignored; all 77 focused checks passed.
  V-A still reports native B0001, "match arm terminates before assigning and
  merging its result"; PHP matches exact cleanup output. Duration 539.29 seconds,
  peak sampled RSS 1,184,481,280 bytes. All 2,095 audit files stayed unchanged,
  SHA-256 `355e19847172fd0f41cd854c4c14c59058be422a7ffc2f9b8588bf14bcb74bd1`.
  A final ownership-boundary refinement keeps authored symbolic storage views
  separate from exported concrete layouts. All 77 focused checks then passed,
  none ignored, at
  `dev-f0a5a198f0d09d5aec0ea6b7d7b14f9bc05dace61c7205a47bf0a572739501f6`
  in 120.20 seconds, peak sampled RSS 1,296,203,776 bytes. The 481-test harness
  compiled; the full library was not rerun after that refinement. All 2,095 audit
  files stayed unchanged, SHA-256
  `2779f31eb0b3fd7d41d3fab9934adaa7eed448102fd43709d1f4496801589c3b`.
  Three exact obsolete-only cleanups removed 16 stale test objects each and
  preserved all 30,676 retained-file records each. One plan was rejected before
  deletion after one candidate's allocated-block count changed; proofs and
  retained records stayed equal. The cause of that allocation change is
  unverified. A fresh exact plan was reviewed and applied. Final allocated caches
  are 24,786,321,408 bytes: Doria 24,621,699,072 and Tooling 164,622,336, below the
  aggregate 25,000,000,000-byte ceiling. No healthy cache, source, user work or
  installed tool was removed. No build remains active. Environment: macOS ARM64,
  Rust 1.98.1, backend LLVM 18.1.8, PHP 8.5.12. No fresh Linux/native Windows/PHP
  8.4, strict Clippy, integration-target, leak or full performance gate ran.
  Public E0776, symbolic coherence, borrowed-storage review, remaining ownership
  specialization, activation and integration remain open. No conditional MIR was
  executed. Full Stage 36a stays estimated at 63% under unchanged scope and
  weights. No public API/schema, Tooling text/hover, pin, install, Git ref or
  publication changed. Invalidated elsewhere: compiler-keyed Tooling/Benchmark
  evidence and website versioned guide/API-reference/playground verification.

- Actual-source specialization checkpoint: the checked conditional regression
  now uses ordinary HIR lowering and shared callable specialization through a
  test-only entry. Direct and nested-trait methods/getters preserve active and
  inactive owner selection; generic methods and their helper dependencies
  materialize with checked bindings. No consumer facts are injected. Public
  lowering still returns E0776 with complete checked facts; no conditional MIR
  is lowered or executed. The production lowering worker is shared unchanged.
  All 77 selected checks pass, none ignored, at
  `dev-368a52135e877c918d909e63759953c01b11eac4ecdde60c5f83b3a5d0110838`
  in 118.60 seconds, peak sampled RSS 1,238,958,080 bytes. The 481-test harness
  compiled; the full library was not rerun. All 2,095 audit files stayed unchanged,
  SHA-256 `3c550382c8f183c2d6e2105bee9fe8250508ff46fde16ed4a187792baa66db58`.
  Environment: macOS ARM64, Rust 1.98.1, backend LLVM 18.1.8, PHP 8.5.12. No fresh
  Linux/native Windows/PHP 8.4, strict Clippy, integration-target, leak or full
  performance gate ran. One selective plan was rejected before deletion after
  a candidate's allocated-block count changed; cause unverified. The refreshed
  exact plan removed 16 obsolete objects and preserved all 30,676 retained-file
  records. Final allocated caches are 24,787,783,680 bytes: Doria 24,623,161,344
  and Tooling 164,622,336. No active build remains; healthy caches, installed
  tools, source and user work are preserved. Full Stage 36a remains estimated
  at 63% under unchanged scope and weights. Coherence/borrowed-storage rulings,
  conditional activation and all remaining integration/performance gates stay
  open. No public API/schema, Tooling hover/text, pin, install, Git ref or
  publication changed. Invalidated elsewhere: compiler-keyed Tooling/Benchmark
  evidence and website versioned guide/API-reference/playground verification.
  A planning audit also removed the stale requirement to remove per-outcome PHP
  construction on file/group hot paths. Those paths use shared field transport;
  their constructor-count probes passed in the preceding full 481-test run.
  Five relevant emitter/probe files remain unchanged. That prior execution is
  retained under its original compiler identity; no current-identity PHP rerun
  or total host-allocation pass is claimed. Arrays, closures, FFI and internal
  allocator work remain outside that constructor probe's measurement scope.

- PHP native-address allocation checkpoint: the current `__DoriaBytes` helper
  was exercised directly with native storage, without a compiler build or I/O.
  At 0/1/1,000 calls, offset-zero returned 0/1/1 distinct CData objects while
  offset-one returned 0/1/1,000. Both offset-one views addressed the original
  byte. File/standard scalar read/write and pending preparation reach this
  helper through `__doriaNativeRegionPointer`. This reproduces an allocation
  position excluded by ordinary constructor counts. A clean PHP 8.5.12 probe
  uses instance FFI allocation and supported collection methods, treating
  warnings as errors. Two exploratory probes using deprecated calls were
  corrected. No transfer timing, total allocator count or byte copy was measured.
  The existing weak observer now has an explicit CData regression: 0/1/1,000
  fresh views count exactly, duplicate observations count once, cached views
  count once, and no observed view is retained. All eight selected observer/
  stream controls pass, none ignored, at
  `dev-b54a3872297ebf072bcfdb264eb03450e4d25fadee30b23e0f6bd54e49001f4b`
  in 84.70 seconds, peak sampled RSS 1,208,205,312 bytes. The 482-test harness
  compiled; the full library was not rerun. All 2,095 audit files stayed unchanged,
  SHA-256 `e731c107b3dfe00f95b147784a687da38da9c1ab38b003bf8059c92cc0046701`.
  A separate isolated pointer-slot control produces 0/1/1,000 observed CData
  objects. Hosted source replies read such a slot. The native action contract
  lends record/view only until the matching reply; caching across handoffs
  would exceed that contract. This additional path remains uncorrected.
  A scoped shared byte-range bridge correction has been requested, preserving
  bounds, outcomes, retained storage and drain behavior; approval is pending.
  No native bridge, MIR or production PHP behavior changed. Only observer tests
  and their measurement-scope comment changed. Existing V-A/M-C/R-S, coherence,
  borrowed-storage and integration holds remain separate. Environment: macOS
  ARM64, Rust 1.98.1, backend LLVM 18.1.8, PHP 8.5.12. No fresh Linux/native
  Windows/PHP 8.4, strict Clippy, integration-target, leak or full performance
  gate ran. One exact cleanup plan was rejected before deletion after two
  disposable objects' block counts changed; cause unverified. A refreshed exact
  plan removed 16 obsolete objects and preserved all 30,676 retained-file records.
  Final allocated caches are 24,787,881,984 bytes: Doria 24,623,259,648 and Tooling
  164,622,336. No active build remains; healthy caches, source, user work and
  installed tools are preserved. No public API/schema, Tooling text/hover, pin,
  install, Git ref or publication changed. Full Stage 36a stays estimated at 63%
  under unchanged scope and weights. Invalidated elsewhere: compiler-keyed
  Tooling/Benchmark evidence and website versioned guide/API-reference/playground
  verification. The full host-allocation/copy/performance gate remains open.

- Shared lexical allocation-guard checkpoint: a two-loop reproduction expected
  four allocation tokens but observed two because the helper returned after the
  first loop. Removed first-loop-only inspection. The shared helper now scans
  every peer loop in the selected function, counts nested ranges once, and
  includes headers and do-conditions. Controls cover neighboring functions,
  nested/peer loops, conditional and try/finally placement, missing functions
  and functions without loops. No test was renamed, disabled or narrowed.
  All 11 current guard consumers plus three observer controls pass: 14 selected,
  zero ignored, at
  `dev-b8b09c556ca339541efa867ba2be5fe68ecd91f32c708db38bb52c9f3713663c`.
  This includes stream completions/reads, field arguments, nested payloads,
  callback results and local/property storage. Duration 161.92 seconds, peak
  sampled RSS 1,231,831,040 bytes. The 482-test harness compiled; the full library
  was not rerun. All 2,095 audit files stayed unchanged, SHA-256
  `6c3e2fe5b82713890e0565d1d2fcfe8939199d26a0d0e0d76165fbe8202fee83`.
  Environment: macOS ARM64, Rust 1.98.1, backend LLVM 18.1.8, PHP 8.5.12. The
  existing LLVM libunwind linker warning recurred; no dependency changed.
  No fresh Linux/native Windows/PHP 8.4, strict Clippy, integration-target, leak
  or full allocation/performance gate ran. Lexical syntax checks establish
  neither runtime reachability nor total host allocations. The reproduced FFI
  address and borrowed handoff-slot paths remain uncorrected; byte-range bridge
  approval, other design rulings and all existing holds remain unchanged.
  An exact selective plan removed 16 obsolete test objects and preserved all
  30,676 retained-file records. Final allocated caches are 24,787,881,984 bytes:
  Doria 24,623,259,648 and Tooling 164,622,336. No active build remains; healthy
  caches, source, user work and installed tools are preserved. No production
  PHP/native/MIR behavior, public API/schema, Tooling text/hover, pin, install,
  Git ref or publication changed. Full Stage 36a stays estimated at 63% under
  unchanged scope and weights. Invalidated elsewhere: compiler-keyed Tooling/
  Benchmark evidence and website versioned guide/API-reference/playground
  verification.

- PHP inline match-guard checkpoint: field-match guards now use the existing
  statement-region predicate path inside the matched-pattern scope. Removed
  expression-only predicate emission from this path and its per-evaluation
  closure. Ordinary matches already used the shared path. The new regression
  first passed functional order/cleanup controls, then failed the closure guard
  before the fix. Afterward, 42 of 43 selected checks pass, none ignored, at
  `dev-36ab0ba982f53d33398e577c56e14c0f9e18b3839f6a7021af526eb824b74e36`.
  Coverage includes 0/1/1,000 evaluations, skipped unmatched and false guards,
  nullable patterns, and exactly-once cleanup for false and throwing predicates.
  The enabled V-A test still passes its exact PHP cleanup output and fails
  native MIR B0001: a match arm terminates before assigning/merging its result.
  Its native correction remains held. No test was renamed, disabled or narrowed.
  The 483-test harness compiled; the full current library was not rerun.
  All 2,095 audit files stayed unchanged, SHA-256
  `de33e533fa2723a3be7022863c7cdc73b8eee3c6e7ee3d0d6302126bd14e11c0`.
  The related run took 162.68 seconds, peak sampled RSS 1,203,961,856 bytes,
  under one offline build job and a 320 MiB cache-growth bound. Environment:
  macOS ARM64, Rust 1.98.1, backend LLVM 18.1.8, PHP 8.5.12. No fresh Linux,
  native Windows, PHP 8.4, strict Clippy, integration-target, leak or full
  host-allocation/performance gate ran. Expression-only match emission still
  uses closures; FFI address and borrowed handoff-slot paths remain uncorrected.
  Existing design, native/bridge and integration holds remain separate.
  Two exact selective plans removed 16 obsolete test objects each, reclaiming
  198,660,096 and 205,877,248 allocated bytes; each preserved all 30,676 retained
  records. Final allocated caches are 24,786,030,592 bytes: Doria 24,621,408,256
  and Tooling 164,622,336. No active build remains; healthy caches, source,
  user work and installed tools are preserved. No native/MIR/ABI, public API,
  metadata schema, Tooling text/hover, pin, install, Git ref or publication
  changed. Full Stage 36a stays estimated at 63% under unchanged scope/weights.
  Invalidated elsewhere: generated PHP and compiler-keyed Tooling/Benchmark
  evidence and website versioned guide/API-reference/playground verification.
  No new public content synchronization contract is introduced.

- PHP prepared-control checkpoint: the shared value preparer now stages
  supported `match`/`when` values through existing statement destinations.
  Removed direct expression rendering of do-loop predicates. A first-iteration
  flag stages each tail at the next head, preserving unconditional first entry,
  condition evaluation after `continue`, and no condition after `break`.
  Removed omission of fixed static assignments from RHS statement roots;
  computed targets retain their separate evaluation/ownership requirements.
  No accepted language behavior was removed. The new regression covers
  0/1/1,000 iterations, nested/lazy conditions, finalizers, body cleanup before
  the tail, and false/throwing condition cleanup. It checks both hot-loop and
  structured-exit functions for generated expression closures.
  An initial fixture failed E0585 for missing `default` arms; that fixture was
  corrected and source-checked before further builds. Two subsequent runs
  passed functional controls but still failed the lexical guard. A bounded,
  build-free rerun captured the exact emitted PHP: two closures came from
  finalizer arithmetic in a static assignment. The statement-root correction
  resolves that finding. No assertion was disabled or narrowed.
  Final verification passes 46 of 47 selected checks, none ignored, at
  `dev-a6633fbd291f1727ccdc2ede4f726d16790d95336ea279e4a268263abda001ab`.
  The only failure remains the enabled V-A native MIR B0001 match-result merge;
  its exact PHP cleanup output passes and its separate native hold remains.
  The 484-test harness compiled; the full library and integration targets did
  not run. All 2,095 audit files and the source identity stayed unchanged during
  verification, SHA-256
  `5ea4e99ba9f5023fe58b78ab5c02ce5c093d4c90c65c2ddae9f49a4921f7ee5f`.
  Final run: 163.09 seconds, peak sampled RSS 1,158,496,256 bytes, one offline
  build job and a 320 MiB growth bound. Environment: macOS ARM64, Rust 1.98.1,
  backend LLVM 18.1.8, PHP 8.5.12. No fresh Linux/native Windows/PHP 8.4,
  strict Clippy, leak or full host-allocation/performance gate ran. For-loop
  increment expression emission remains an inspection position; no general
  expression-allocation freedom is claimed. FFI view findings and existing
  design/native/bridge/integration holds remain open.
  Four selective plans removed only 64 obsolete test objects, reclaiming
  807,907,328 allocated bytes and preserving all 30,029 retained records each
  time. One plan was rejected before deletion after candidate block counts
  changed; the refreshed exact plan preserved all other candidate fields,
  ownership proofs and retained records. The block-count change's cause remains
  unverified. Final allocated caches: 24,787,918,848 bytes, including Doria
  24,623,296,512 and Tooling 164,622,336. No active build remains. Healthy
  caches, source, user changes and installed tools are preserved. No native
  MIR/ABI, public API/schema, Tooling strings/hovers, pins, installs, Git refs or
  publication changed. Stage 36a remains estimated at 63% under unchanged
  scope/weights. Invalidated elsewhere: generated PHP and compiler-keyed
  Tooling/Benchmark and website versioned guide/API-reference/playground
  verification. No new public-content synchronization contract is introduced.

- PHP for-increment checkpoint: removed the expression-string increment helper.
  For increments now reuse ordinary statement emission at the next iteration's
  head, preserving initial-condition checks, body cleanup, `continue` increments
  and `break` skipping increments. Stable conditional assignment destinations
  use the shared value emitter. Owning locals use their existing cells, acquiring
  complete replacements before old-owner cleanup; abandoned replacement results
  are released while the prior owner survives. Throw operands now participate
  in statement regions and owned conditional-result staging before error handoff.
  The new regression first passed functional controls and failed its closure
  guard. After increment/assignment correction, all ownership controls passed;
  a remaining closure in a throwing finalizer identified the omitted throw
  producer. The final correction passes 47 of 48 selected checks, none ignored,
  at `dev-0720200cec9a15570adcf0ddc973a07e6df12fc80f3267c3f38e60c05b537141`.
  Coverage includes 0/1/1,000 iterations, compound/plain counters, primitive
  static counters, successful owning replacement, a finalizer abandoning an
  acquired result, and conditional Error values. An exploratory owned-static
  fixture was removed after the E0486 source prerequisite rejected it; Decision
  0084's deferred owned-static boundary remains unchanged. No accepted behavior
  or existing test assertion was removed, disabled or narrowed.
  The only failure remains enabled V-A native MIR B0001 for a missing match-result
  merge; its exact PHP cleanup output passes and its separate native hold remains.
  The 485-test harness compiled; the full library and integration targets were
  not rerun. Source identity and all 2,095 audit files stayed unchanged during
  verification, SHA-256
  `1b394e682aa9412360deb410e681777a0fa8315a352b322c81ba303660fffdb8`.
  Final batch: 165.24 seconds, peak sampled RSS 1,216,364,544 bytes, one offline
  job and a 320 MiB growth bound. Environment: macOS ARM64, Rust 1.98.1,
  backend LLVM 18.1.8, PHP 8.5.12. No fresh Linux/native Windows/PHP 8.4,
  strict Clippy, leak or full allocation/performance gate ran. Computed assignment
  targets and other expression-only contexts retain separate staging requirements;
  total host-allocation freedom is not established. Native/bridge, conditional
  coherence, borrowed-storage and integration holds remain separate.
  Three exact selective plans removed only 48 obsolete test objects, reclaiming
  604,262,400 allocated bytes; all 30,029 retained records matched each time.
  One plan was rejected before deletion after two candidate block counts changed;
  the refreshed exact plan preserved other fields, proofs and retained records.
  The cause remains unverified. A post-run scan measured 24,781,582,336 allocated
  bytes: Doria 24,616,960,000 and Tooling 164,622,336. No active build remains.
  Healthy caches, source, user work and installed tools are preserved. No native
  MIR/ABI, public API/schema, Tooling strings/hovers, pins, installs, Git refs or
  publishing changed. Stage 36a stays estimated at 63% under unchanged
  scope/weights. Invalidated elsewhere: generated PHP and compiler-keyed
  Tooling/Benchmark and website versioned guide/API-reference/playground
  verification; no new public-content synchronization contract.

- PHP conditional call-value checkpoint: checked call signatures now direct
  non-cell `when`/`match` arguments through existing statement destinations.
  Operand ownership is registered before evaluation; field transport and
  cell-parameter rendering/loans retain their existing protocols. The shared
  path covers free/static/instance calls, callbacks and computed setters.
  The new regression checks 0/1/1,000 iterations, written named-argument order,
  null-safe argument skipping, setter counts and exactly-once cleanup when a
  later argument throws before invocation. Its lexical hot-loop guard finds
  no expression closures in the plain value-parameter path. Conditional cell
  arguments, receivers/callees, constructors and computed targets retain
  separate staging requirements; total allocation freedom is unestablished.
  An unsupported arrow return annotation failed the source prerequisite and
  was corrected before the build. No language behavior or test was removed,
  disabled or narrowed. No fresh pre-fix runtime red run is claimed.
  Verification passes 48 of 49 selected checks, none ignored, at
  `dev-75e25b93bbb16ccd38840738a2af4be07f60fa982d9b897b5111c1fba05b5a4e`.
  The only failure remains enabled V-A native MIR B0001 for the missing
  match-result merge; its exact PHP cleanup output passes. The 486-test
  harness compiled; the full library and integration targets were not rerun.
  Source identity and all 2,095 audit files stayed unchanged during the run,
  SHA-256 `66f7257596856feecb04d31c1fd790c09044ee69ef0c1f9571e9e5774c4bf1da`.
  Batch: 165.47 seconds, peak sampled RSS 1,091,616,768 bytes, one offline job,
  600-second/6 GiB limits and a 320 MiB growth bound. Environment: macOS ARM64,
  Rust 1.98.1, backend LLVM 18.1.8, PHP 8.5.12. No fresh Linux/native Windows/
  PHP 8.4, strict Clippy, leak or full allocation/performance gate ran.
  One reviewed selective plan reclaimed 198,590,464 allocated bytes from
  16 obsolete completed-link test objects; all 30,029 retained records matched.
  Post-run caches measured 24,777,711,616 allocated bytes: Doria 24,613,089,280
  and Tooling 164,622,336. No active build remains. Healthy caches, source,
  user changes and installed tools are preserved. Native MIR/ABI, public API/
  schema, Tooling strings/hovers, pins, installs, Git refs and publishing are
  unchanged. Stage 36a stays estimated at 63% under the same full scope and
  implementation/verification/integration weights of 50%/30%/20%. Existing
  native/bridge, conditional coherence, borrowed-storage and integration holds
  remain separate. Invalidated elsewhere: generated PHP and compiler-keyed
  Tooling/Benchmark and website versioned guide/API-reference/playground
  verification; no new public-content synchronization contract.

- PHP conditional receiver checkpoint: the call shortcut now checks the
  receiver as well as arguments. Ordinary and consuming conditional receivers
  use the existing statement-value emitter, including zero-argument calls.
  Shared payload projection is factored without changing its storage-home
  contract; cell-backed receivers retain the existing loan path. Computed
  setters reuse the same projection helper, but conditional setter targets
  were not separately executed. Callee staging also uses statement values;
  its execution remains unverified behind the shared lowering failure below.
  The first 50-check run retained the previous 48 passing focused checks.
  V-A's enabled native MIR B0001 remains unchanged. The new combined fixture
  failed M1101 before PHP generation: shared function-value lowering lacks
  conditional-result materialization for a directly invoked callback. PHP
  closures require its MIR plans. A separately requested native correction is
  unapproved; no gate, closure plan or backend fallback was changed.
  Splitting the regression exposed B2301 for the initial concrete-class shared
  payload control. It now uses the supported nominal-interface payload; the
  existing concrete-class shared PHP rejection and its tests are unchanged.
  Both split fixtures pass source checking. The callable regression remains
  enabled and fails M1101; its source hash is unchanged after the split run.
  The final receiver-only regression passes at
  `dev-232e4058fdf44c9ec789e2136ea7b0f7f134e29b6f3efb680ebadc86054b2c0b`.
  It checks 0/1/1,000 iterations, zero-argument `when`/`match` receivers,
  shared-interface projection, null gating, finalizer/argument order, and five
  exactly-once owner releases on pending, successful and abandoned handoffs.
  Its hot-loop lexical guard finds no expression closures. The 488-test
  harness compiled; the full library/integration targets were not rerun.
  Final run: 92.47 seconds, peak sampled RSS 1,160,642,560 bytes, one offline
  job, 600-second/6 GiB limits and a 320 MiB growth bound. All 2,095 audit files
  and the identity stayed unchanged during that run, SHA-256
  `a0b0047f2a25f9af429eb821208282202d480f7f771e5c984f3a589cddf81486`.
  Three exact plans reclaimed 590,659,584 allocated bytes from 48 obsolete
  completed-link objects; all 30,029 retained records matched after each
  cleanup. Post-run caches: 24,787,939,328 bytes, Doria 24,623,316,992 and Tooling
  164,622,336. Environment remains macOS ARM64/Rust 1.98.1/LLVM 18.1.8/PHP 8.5.12.
  No fresh Linux/native Windows/PHP 8.4, strict Clippy, leak or full allocation/
  copy/performance gate ran. No accepted behavior or existing assertion was
  removed or disabled. Native/bridge, coherence, borrowed-storage and integration
  holds stay separate. No Tooling strings/hovers, public API/schema, pins,
  installs, Git writes or publishing changed. Stage 36a remains estimated at
  63% under unchanged 50%/30%/20% implementation/verification/integration weights.
  Invalidated elsewhere: generated PHP and compiler-keyed Tooling/Benchmark and
  website versioned guide/API-reference/playground verification; no new public
  content synchronization contract. Total host-allocation freedom is unproven.

- PHP physical-input and receiver verification checkpoint (2026-11-02): named
  and callback argument emission share checked value/cell selection, and property
  receivers retain one evaluated producer with its existing ownership or loan.
  Grouped borrowed collection results keep their physical cell; logical reads
  remain lazy. The strengthened named-call regression passes at 0/1/1,000
  iterations, including named/default argument order, borrowed getters,
  `List.first`, temporary-owner cleanup and zero loop expression closures.
  The earlier installed-compiler B2301 was stale evidence; current source accepts
  and executes that collection property. No rejection gate was bypassed.
  At `dev-00e3afc5353163b1584a8d94137044f4a1d79d3cd2d08d3ecfc5efbdb64e8374`,
  all 84 PHP-module library tests ran once: 81 passed, three failed, zero ignored.
  The unchanged failures are native match-result merging, M1101 conditional
  callable lowering, and native when-result merging. Their tests remain enabled.
  The first 53-test batch took 167.27 seconds with peak sampled group RSS
  1,226,489,856 bytes; the remaining 31 reused its executable and passed in
  16.89 seconds with 78,856,192 bytes. Aggregate caches were 24,787,300,352
  bytes after execution. Approved selective cleanup then reclaimed 205,283,328
  bytes from 16 obsolete linker objects, retaining the executable and reusable
  caches; the remeasured aggregate was 24,582,017,024 bytes. The verified old
  8,061,816-byte temporary cache observation was also retired. Rust 2021
  formatting correction and this
  documentation update postdate execution; no subsequent semantic edit occurred.
  Full library/integration, fresh Linux/Windows/PHP 8.4, and total host-allocation
  gates were not run. Native correction/design holds and public E0776 remain.
  Tooling source/pin, installed tools, Git refs and publication are unchanged.
  Stage 36a remains estimated at 63% under unchanged scope and weights.
  Invalidated elsewhere: generated PHP and compiler-keyed Tooling/Benchmark and
  website versioned guide/API-reference/playground verification.

- PHP conditional borrowed-cell checkpoint (2026-11-02): shared argument
  destinations carry the selected existing cell through nested `when`/`match`
  branches, preserving conversions and pending temporary cleanup. The regression
  passes both branch choices at 0/1/1,000 iterations with no cell-count growth or
  loop expression closures. Generated-output inspection traced four unrelated
  per-iteration cells to static numeric counter updates. The shared numeric-place
  path now uses direct stored-static locations; the counter assertions remain.
  At `dev-6c994c8e196cf97be1cb5c1a7b676eab96efbb028758731f702719655004d47e`,
  all 86 PHP-module library tests ran: 82 passed, four failed, zero ignored.
  The three existing native result-merge/callable failures remain. The added
  single-source returned-borrow regression fails E0478 before PHP generation:
  shared source provenance does not follow conditional results back to their
  borrowed parameter. Its correction awaits scoped approval; no ownership or
  native checker was changed. The run took 180.08 seconds with peak sampled RSS
  1,169,063,936 bytes, one offline job and unchanged source identity. Selective
  cleanup first removed 192,684,032 bytes of obsolete linker objects. Aggregate
  caches measured 24,788,217,856 bytes after verification. The temporary observer
  was removed. Full host-allocation freedom, portable verification, Tooling
  integration and installed refresh remain open; no Git writes or publication.
  Invalidated elsewhere: generated PHP and compiler-keyed Tooling/Benchmark and
  website versioned guide/API-reference/playground verification.

- PHP forwarded argument-home checkpoint (2026-11-02): conditional argument
  discovery records every selected binding, and concrete call plans propagate
  borrowed Move storage requirements through callers and matching virtual and
  interface families. Copy parameters keep independent snapshots. The shared
  call-site planner also retains callback returned-borrow home requirements.
  The new regression passes both branches at 0/1/1,000 iterations through
  multi-hop named calls, generic calls, virtual calls and erased interface calls,
  with flat cell counts, exact results and two owner destructions. An initial
  source-signature assumption failed for synthesized functions; the correction
  consumes the checked parameter-binding metadata used by emission.
  At `dev-312956bf70385be1e603495bc912696ea4c33d6c5ab34515ad6bb5b0aaab199b`,
  all 87 PHP-module tests plus 11 specialization/ownership regressions ran:
  94 passed, four failed, zero ignored. The same B0001/M1101/E0478 failures above
  remain enabled and their correction approvals remain pending. The final
  one-job offline run took 227.63 seconds, with peak sampled RSS 1,007,943,680
  bytes and unchanged source identity. Reviewed selective cleanup removed only
  stale completed-link objects before each build; the final aggregate cache was
  24,787,509,248 bytes. Formatting and diff checks pass. Full host-allocation
  freedom, fresh Linux/Windows/PHP 8.4 verification, workspace-wide Clippy,
  integration and installed refresh remain open. No native/source ownership
  semantics, installed tools, Git refs or publication changed. Stage 36a remains
  estimated at 63% under unchanged scope and weights. Invalidated elsewhere:
  generated PHP and compiler-keyed Tooling/Benchmark and website versioned
  guide/API-reference/playground verification; no public spelling changed.

- PHP numeric stored-place checkpoint (2026-11-02): compound assignments and
  standalone increments share statement preparation, previous-value capture,
  checked arithmetic and commit. Stored object receivers are captured once in
  their lexical scope, without per-update borrowed cells or expression closures;
  ordinary statement cleanup retains receiver and operand owners. For-header
  increments use the same path. The new fixture checks internal/nested fields,
  integer widths, float updates, conditional operands, `continue`, temporary
  hooked receivers and checked-failure destruction order. Interpreter and PHP
  agree on exact output; 0/1/1,000-iteration constructor counts stay flat.
  Existing E0204 temporary stored-write and E0477 conflicting destination-borrow
  boundaries remain checked, without changing source ownership semantics.
  At `dev-3c3581bed3e1ed65f6505007beece459269808346739cf220f8a24fdb70015da`,
  99 focused checks produced 94 passes, the four unchanged native/provenance
  failures, and a new fixture rejection. After correcting that fixture's
  ownership forms, all five directly affected checks passed at
  `dev-d7f803109383ae66d92ff6a2d8a447f9c16bd1470f8a1d6c9cf2ded5696523aa`;
  production code was unchanged between runs. The final offline one-job run took
  104.86 seconds with peak sampled RSS 1,088,684,032 bytes. Source identity,
  formatting and diff checks passed. Reviewed cleanup reclaimed 192,684,032 and
  207,347,712 bytes of stale completed-link objects; final aggregate caches were
  24,788,512,768 bytes. No fresh Linux/Windows/PHP 8.4, full workspace validation,
  installed refresh or publication occurred. Stage 36a remains estimated at 63%.
  Invalidated elsewhere: generated PHP and compiler-keyed Tooling/Benchmark and
  website versioned guide/API-reference/playground verification. Public spelling
  and the existing native, bridge, design and publication holds are unchanged.

- PHP stored-assignment checkpoint (2026-11-02): ordinary stored-property
  assignments and numeric updates share receiver preparation in the declaring
  lexical scope. Conditional RHS values use statement staging; owning writes
  acquire the replacement, commit it, then release the previous value without
  an expression closure. Existing field-backed storage and accessor plans remain
  separate. The new fixture covers nested/internal fields, generic nullable
  fields, repeated owning replacement, failed replacement and partial constructor
  cleanup. Its initial property move-out was corrected to a borrowed parameter
  read; no source ownership rule changed. The 100-check run passed 95, retaining
  four existing failures and this new fixture failure. Three related checks
  passed after the fixture correction. At
  `dev-8fedf2b3ebc935000e978ca0a7a5ddc6fd5d5178e10487af829d260d099d196b`,
  PHP exact output, destruction order and flat wrapper counts at 0/1/1,000
  iterations pass. The same test then fails its retained native assertion:
  I2803 rejects a declared `int` conditional result assigned to a nullable field.
  At this checkpoint, shared destination conversion correction awaited scoped
  approval; interpreter and native parity were unverified. The final one-job offline
  run took 84.79 seconds, peak sampled RSS 1,090,535,424 bytes; source identity,
  formatting and diff checks passed. Two reviewed obsolete-only cleanup plans
  each reclaimed 204,414,976 bytes; final aggregate caches were 24,786,862,080
  bytes. No fresh portable/full-workspace validation, installs, Git writes or
  publication occurred. Stage 36a remains estimated at 63%. Invalidated elsewhere:
  generated PHP and compiler-keyed Tooling/Benchmark and website versioned
  guide/API-reference/playground evidence; no public spelling changed.

- Full compiler-library checkpoint (2026-11-02): reused the existing
  `dev-8fedf2b3ebc935000e978ca0a7a5ddc6fd5d5178e10487af829d260d099d196b`
  executable after documentation-only changes. All 495 library tests ran in one
  sequential process: 490 passed, five failed, none ignored or filtered out.
  The failures are conditional returned-borrow provenance (E0478), conditional
  callable lowering (M1101), conditional argument and match-result merges
  (B0001), and conditional-to-nullable destination conversion (I2803). No new
  failure appeared. Guarded source checking, nominal proofs, checked witnesses,
  virtual contracts and the other library regressions pass; this does not enable
  conditional source execution or close full-workspace/portable verification.
  The bounded run took 445.25 seconds, peak sampled RSS 808,779,776 bytes, with
  aggregate caches unchanged at 24,779,550,720 bytes. No build or cleanup ran.
  All 1,378 inventoried crate/configuration files and the executable matched
  before/after checksums; the full checkout identity also stayed unchanged during
  the run. Environment: macOS ARM64, Rust 1.98.1, PHP 8.5.12, LLVM-enabled test
  executable. Strict Clippy, integration-test targets, fresh Linux/Windows/PHP
  8.4, complete allocation/performance gates and installed refresh remain open.
  Stage 36a remains estimated at 63% under unchanged scope and weights. Tooling
  was coordinated without pin/install changes. Invalidated elsewhere: none from
  this verification-only step; existing compiler-keyed integration evidence still
  requires coordinated refresh. No Git writes or publication occurred.

- PHP 8.4 compatibility checkpoint (2026-11-02): reused the same compiler-library
  executable with installed PHP 8.4.21 first in the child PATH. All 89 PHP-module
  tests ran sequentially: 84 passed, the same five compiler failures listed above
  remained, none ignored and no execution-skip messages; 406 non-PHP tests were
  filtered out. Stream/readiness FFI and FFI-free cases passed. Numeric-match and
  stored-assignment PHP cleanup checks passed before their retained native
  assertions failed. No PHP-version-specific failure appeared. Source and
  executable checksums matched before/after; checkout identity stayed
  `dev-950109f2bc889056da96642499594f84218443a3201ebf911b27e4c509e1daaa`.
  The macOS ARM64 run took 107.16 seconds, peak sampled RSS 784,072,704 bytes;
  allocated caches stayed at 24,779,550,720 bytes. No build, cleanup, install,
  dependency change, Git write or publication ran. This closes current PHP-module
  coverage on PHP 8.4; portable/full-workspace and complete performance gates
  remain open. Tooling was coordinated without pin or installation changes.
  Stage 36a stays estimated at 63%. Invalidated elsewhere: none from this
  verification-only step; earlier integration refresh requirements remain.

- Approved conditional-result conversion checkpoint (2026-11-02): Andrew
  approved the shared correction. Branching expressions now form their checked
  result before applying the existing destination conversion, preserving result
  storage and ownership through the merge. No language rule or backend-specific
  override changed. Three new regressions cover scalar widths, nullable fields,
  arguments and returns, short-circuit evaluation, nominal upcasts, managed
  values, replacement and failed-construction cleanup. An invalid inherited
  constructor declaration in the initial test was corrected; no assertion was
  removed. At
  `dev-fbf73ea7830df8747049f1bae9fa9b161103576bdcd7502e0c4670e86ed0a959`,
  all four focused checks pass on macOS ARM64, including interpreter/Cranelift/
  LLVM parity and the original stored-assignment regression. The full library
  run passed 494 of 498 tests, none ignored or filtered: E0478, M1101 and the two
  B0001 merge failures remain unchanged. I2803 is resolved. All 1,379 source/
  configuration files and the executable matched before/after checksums.
  The sequential full run took 529.76 seconds, peak sampled RSS 795,901,952
  bytes; aggregate caches stayed at 24,789,512,192 bytes. The affected regression
  also passed on installed PHP 8.4.21 without rebuilding. The preceding one-job
  offline build/focused run took 135.57 seconds after reclaiming 206,311,424 bytes
  of verified obsolete linker objects through the approved repository workflow.
  Fresh Linux/Windows, full-workspace, complete performance and installed-refresh
  gates remain open. Tooling was coordinated; its pin, strings and hovers remain
  unchanged. No installs, dependencies, Git writes or publishing changed.
  Stage 36a stays estimated at 63% under unchanged scope and weights. Invalidated
  elsewhere: compiler-keyed Tooling/Benchmark and website versioned guide/API-
  reference/playground verification; no public spelling or metadata changed.

- Approved conditional ownership and merge checkpoint (2026-11-03): the four
  approved E0478/M1101/B0001 regressions pass. Shared callable lowering consumes
  checked conditional function types; result validation distinguishes acquired
  values from proved escaping checked errors. Canonical binding/CFG inference
  carries returned parameter borrows through branch aliases and closures.
  Pending return/yield loans share cleanup state, and variable replacement uses
  the shared borrow-conflict check. This rejects previously accepted replacement
  or consumption of a borrowed owner in a finalizer. Nested finalizer forwarding
  preserves exit kind, result identity and parent-child relationships; removing
  the original acquisition remains a malformed-MIR rejection.
  The preceding source identity passed 502 library tests. After closure and
  finalizer coverage was added, all 11 focused checks and all 504 library tests
  pass at
  `dev-eb3d866c1d2a85ab7c2e72b042a61f9369b8dad7e43b4c6d8214312d9a56195d`.
  The macOS ARM64 library run had zero failures, ignored or filtered tests and
  took 464.52 seconds (466.01 seconds bounded elapsed), with peak sampled RSS
  806,125,568 bytes. All 1,379 source/configuration files and the executable
  matched before/after: source inventory SHA-256
  `d4b3c489037fb66c8489d86627258c7b95dab786c77db7a9d67a298e1fbb3b4c`,
  executable SHA-256
  `7d227ad7311d51645972ac1ef5c19cf116589d095f16f17bc570271c1bf24bb0`.
  Tests include interpreter/Cranelift/LLVM and PHP compatibility execution;
  there was no fresh Linux/Windows or full-workspace run. Aggregate allocated
  caches ended at 24,781,774,848 bytes, below the 25,000,000,000-byte ceiling.
  The final focused build followed exact-plan reclamation of 206,299,136 bytes
  of verified obsolete linker objects; healthy caches and installed tools were
  preserved. Local-alias, branch-local escape and pattern-bound result coverage
  remain open, alongside the full Stage 36a implementation, performance and
  integration gates. E0776 and separate native/design holds remain unchanged.
  Tooling remains idle; its pin, strings and hovers are unchanged. No installs,
  dependency changes, Git writes or publishing ran. Stage 36a remains estimated
  at 63% with implementation/verification/integration weights 50/30/20.
  Invalidated elsewhere: compiler-keyed Tooling/Benchmark and website versioned
  guide/API-reference/playground verification. Public spellings and metadata
  are unchanged; follow-up documentation edits do not change the tested code.

- Pattern-bound return checkpoint (2026-11-03): shared MIR provenance follows
  borrowed payload bindings and mixed class projections. Cranelift/LLVM payload
  homes retain original nullable storage layouts; interpreter payload paths
  retain original places and reuse layout-only descriptors across owner lifetimes.
  PHP borrowed conditional returns now use the existing argument/result
  destination machinery, with identity checks for direct `match`/`when` returns.
  Local aliases, expired branch/given owners, guarded/nested/nullable/mixed
  patterns and explicit callback transfer have regression coverage. Returning
  a borrowed callback remains rejected under Decision 0121; no escape rule was
  relaxed. Removing payload-binding provenance remains a malformed-MIR rejection.
  All 18 focused checks and all 510 compiler-library tests pass at
  `dev-b52c0d9e0bca68198928a02a7ae7d9fc111585d6cb460bb2dde0760e83287f04`.
  The macOS ARM64 full run had zero failures, ignored or filtered tests:
  473.96 test seconds, 475.01 bounded seconds, peak sampled RSS 796,524,544 bytes.
  All 1,379 source/configuration files and the executable matched before/after:
  source inventory SHA-256
  `d78f4ffcc6e523c7bd5186a6584a583f560399c314160ddfed4b594f230bc0fe`,
  executable SHA-256
  `38f8bd88c2f72c2edf4e52db36a38df778947bc2a4e09456361fe837e6b60f4f`.
  Aggregate allocated caches stayed at 24,790,708,224 bytes during the full run.
  The preceding build reclaimed 206,970,880 bytes of verified obsolete linker
  objects through the exact-plan workflow; reusable caches were preserved.
  No fresh Linux/Windows, full-workspace or native leak-check run is claimed.
  Wider ownership, implementation, performance and integration gates remain open;
  the Stage 36a estimate remains 63% under the unchanged 50/30/20 weighting.
  E0776, separate design/native holds, installed refresh, Tooling pin changes and
  publishing remain held. Tooling was verified idle; no source edits, installs,
  dependencies, pins or Git writes were made in its repository.
  Invalidated elsewhere: compiler-keyed Tooling/Benchmark and website versioned
  guide/API-reference/playground evidence; public spellings remain unchanged.
  These checkpoint documentation edits do not alter the tested compiler sources.

- Managed Copy and retained callback checkpoint (2026-11-03): ordinary writable
  managed Copy parameters now acquire private owned snapshots; callback writable
  borrowing remains unchanged. Retained scalar call sources use checked storage
  roots while ordinary value-borrow validation stays strict. The shared PHP
  stored-field reader reuses its prepared receiver instead of evaluating it
  twice. Existing nullable fallback and destruction-order assertions are preserved.
  Source identity:
  `dev-fe809a6ded9f4395bc570c432d239aa7994e60100db8001c851f9ad72447643e`.
  All five focused tests pass, including the nine-fixture Copy/callback batch,
  on macOS ARM64 with interpreter, Cranelift, LLVM 18.1.8 and PHP 8.5.12.
  The build/run took 435.71 bounded seconds, peak sampled RSS 1,060,634,624 bytes.
  Codegen test executable SHA-256:
  `6d58e7de53792cec42e769d9d9719d9c0772ace7e4e69188bbb63630c22406bf`.
  The complete returned-closure target totals 77 passed and one failed, none
  ignored. Its first run completed 13 passes and one failure before a cache scan
  raced with temporary-directory deletion; only the remaining 64 tests resumed,
  all passing in 346.47 bounded seconds (342.26 test seconds), peak RSS
  217,972,736 bytes. Source identity and executable hash matched before/after.
  Returned-closure executable SHA-256:
  `4f21b44bfa74191f4d4f74a973aadd233eeb7cd12b96b6c90fe200cbf9607e7a`.
  The enabled failure is
  `checked_virtual_calls_preserve_returned_closure_receiver_places`: shared
  analysis rejects a virtual receiver domain after selecting an override.
  Inspection identifies repeated candidate expansion in checked storage-selector
  analysis; its separate correction is requested, not implemented. The bounded
  runner now requires a successful full cache measurement within three attempts;
  an unavailable volume or persistently failed measurement still stops the run.
  Native `leaks --atExit` checks report zero leaked bytes for the 1,000-iteration
  managed Copy fixture in both profiles. All checks used Rust 1.98.1. Formatting,
  whitespace and documentation authority checks pass. The earlier 510-library
  checkpoint predates these changes; no fresh full-library/workspace, Linux,
  Windows or PHP 8.4 execution is claimed.
  Aggregate allocated caches ended at 24,644,972,544 bytes. The initial exact-plan
  cleanup reclaimed 197,005,312 bytes of obsolete test objects; healthy caches,
  installed tools and all source/user work were preserved. Tooling remains idle
  with its compiler pin, strings and hovers unchanged. No installs, dependencies,
  Git writes or publication occurred. Stage 36a remains estimated at 63% under
  the unchanged implementation/verification/integration weights 50/30/20.
  Invalidated elsewhere: compiler-keyed Tooling/Benchmark evidence and website
  versioned guide/API-reference/playground verification. E0776 and the remaining
  implementation, performance and integration gates remain open.

- Callable coherence checkpoint (2026-11-03): declaration-owned formation
  obligations follow generic callers, static/instance methods, authored trait
  bindings and deferred capability checks through the existing specialization
  graph. Concrete nominal facts resolve guards without importing symbolic
  assumptions. Equal specialized interface paths remain valid; independent
  concrete conflicts cannot hide behind a symbolic canonical entry. Final
  conformance validation retains class bounds, and property availability errors
  identify their condition clauses. The grammar fixture's potentially blocking
  interface call was removed from its positive getter example and retained in
  a dedicated E0770 rejection test; the nonblocking rule is unchanged.
  Tested source identity:
  `dev-19e405d1f2d094d1701a53b0d5d45b59e8698a3f707ab95d1f04c2f0d5afdaec`.
  macOS ARM64 results: coherence 29, conditional bodies 38, composition 12,
  grammar 23, type formation 16, ordinary interface contracts 56, cross-file
  graphs 6: 180 passed, zero failed or ignored. The four conditional suites
  took 169.47 bounded seconds, peak sampled RSS 1,109,311,488 bytes; the three
  ordinary suites reused that build and took 17.87 seconds, peak 586,039,296.
  Coherence executable SHA-256:
  `bc5d4dd64b08a3a1e55a71982b39a2b25de328aca0add9ced2f0413d755ae3f8`.
  Source identity was checked before and after each run. Earlier attempts found
  one malformed new test signature and then three grammar-suite failures;
  the completed results above follow their corrections. Aggregate allocated
  caches ended at 24,653,561,856 bytes; no cleanup, installs, dependencies,
  Git writes or publication occurred. No fresh full-library/workspace,
  native-backend parity, Linux, Windows or PHP 8.4 execution is claimed.
  E0776 remains. Full symbolic dependency closure, specialized execution,
  borrowed adapters, performance and integration gates remain open. The
  separate checked virtual-call correction and concrete borrowing contract
  remain pending approval. Stage estimate stays 63%, with weights 50/30/20.
  Tooling builds, pins and installs remain held. Invalidated elsewhere:
  compiler-keyed Tooling/Benchmark evidence, conditional grammar fixture
  expectations, and website versioned guide/API-reference/playground evidence.

- Symbolic dependency checkpoint (2026-11-03): formed-type coherence now follows
  unconditional storage and proved active targets for symbolic arguments under
  the original caller's frozen bounds. Unproved target guards introduce no
  assumptions. Stable recursion and finite collapse remain valid; expanding
  symbolic paths retain their root obligation until substitution, where concrete
  expansion retains E0539. Tests cover nested, inherited and trait storage,
  target/storage mixtures, guard isolation and deferred expansion.
  Tested source identity:
  `dev-849c4427fc0563f9243192582a572e001a9727f594b32ccc5ee7ee7b93301574`.
  Seven suites completed successfully on macOS ARM64: coherence 32, conditional
  bodies 38, composition 12, grammar 23, type formation 16, ordinary contracts 56
  and cross-file graphs 6 (183 tests). The bounded run exited zero in 194.35
  seconds with peak sampled RSS 939,835,392 bytes. Source identity was checked
  before and after execution. Coherence executable SHA-256:
  `761a41def26d5850352cfd9cc5a2b5ca34a474c2f909caf91869767ec674f2fc`.
  Aggregate caches ended at 24,665,903,104 allocated bytes.
  A subsequent test-only extension sends the actual checked conditional source
  through MIR validation and interpreter execution, exercising direct and
  interface calls for direct and composed methods/getters. It is unverified:
  its library build stopped at the cache-growth guard before any tests ran.
  Attempted source identity:
  `dev-1b41e46d8392f38eba9c2d6fd7005656c9483d7034c3214360c42ea559972679`.
  That attempt exited 124 after 111.84 seconds, peak sampled RSS 905,576,448
  bytes, with final allocated caches 24,876,163,072 bytes. The selective cleanup
  dry-run found zero eligible bytes and preserved the interrupted test unit
  because it lacks completed-link proof. No deletion or retry occurred.
  Read-only inspection identified 17 matching code-generation objects totaling
  210,268,160 allocated bytes, all newer than the retained 81,965,056-byte
  executable. That executable's SHA-256 is
  `38f8bd88c2f72c2edf4e52db36a38df778947bc2a4e09456361fe837e6b60f4f`.
  Names and timestamps alone do not authorize deletion. Recovery must establish
  exact interrupted-unit ownership and inactivity through the managed workflow;
  these measurements are sizing evidence, not an approved cleanup plan.
  Approval was requested for verified interrupted-compilation recovery in the
  repository-managed cleanup workflow. The following recovery checkpoint
  records Andrew's approval and the subsequent verification attempt.
  Public E0776 remains. No fresh full-library/workspace, native parity, PHP,
  Linux or Windows execution is claimed. Stage estimate remains 63% with
  implementation/verification/integration weights 50/30/20. Tooling is idle;
  builds, pins and installs remain held. No installs, dependency changes, Git
  writes or publication occurred. Invalidated elsewhere: compiler-keyed
  Tooling/Benchmark evidence and website versioned guide/API-reference/playground
  verification. Borrowed adapters, specialized execution, performance and
  integration remain incomplete.

- Interrupted recovery checkpoint (2026-11-03): Andrew approved verified
  interrupted-build recovery and then instructed, "After this clean up the
  24GB used." The existing selective workflow now has a separate
  `--reclaim-interrupted` mode. Version-3 plans bind selection mode, exact unit
  ownership, retained executable, dependency records, empty completion marker,
  invocation record and candidate identities. Cargo locks and active-use checks
  remain mandatory. Unknown names and inconsistent evidence remain preserved.
  The artifact policy safety guard passed twice on macOS, including rejection
  of changed plans, active outputs, held locks, malformed records, linked files
  and missing executable evidence. PHP lint, documentation authority and
  whitespace checks passed. Linux/Windows cleanup execution remains untested.
  The reviewed real plan reclaimed 194,596,864 allocated bytes from 16 objects;
  the retained executable's SHA-256 remained unchanged. One unrecognized object
  was preserved. Aggregate allocation became 24,612,663,296 bytes.
  The subsequent bounded LLVM-feature library build completed. Its focused
  conditional-capability suite finished with 55 passed and 1 failed, 454
  filtered out, on macOS ARM64. Tested identity:
  `dev-529d82365deee1687611ae192e4fc05fd82edab5f285ca4a5d9f961673a6d68e`.
  Test executable SHA-256:
  `5aca93a7b5ea9af6892905b515ab6c6d41793da0cf40d06b2ee515997d6efc6f`.
  `checked_conditional_facts_share_finalization_without_opening_the_public_gate`
  failed at MIR validation: global declaration `Adapter` references a missing
  source. Interpreter execution was not reached. The run exited 1 after 128.16
  seconds, peak sampled RSS 1,090,109,440 bytes, ending at 24,824,913,920
  allocated bytes within the 300,000,000-byte growth budget and aggregate cap.
  No native parity, PHP, Linux, Windows or full-library pass is claimed.
  After confirming no active cache consumers, the explicit `--reclaim` workflow
  cleared the compiler-owned cache, leaving 12,288 allocated bytes of management
  metadata. Compiler and Tooling source identities and both installed binary
  checksums were unchanged. Tooling confirmed its generated cache snapshots
  need no retention; its historical test logs remain outside the cleanup root.
  Tooling then cleared its 164,622,336-byte cache through Cargo and reported
  unchanged installed-server and four historical-log identities. Independent
  final measurement confirms aggregate allocation fell from 24,824,913,920 to
  12,288 bytes: 24,824,901,632 bytes reclaimed by the full cleanup. Both installed
  binary checksums were independently rechecked and remain unchanged. The later
  rebuild was separately approved below.
  Follow-up inspection traced the missing record to the test setup: unlike
  production standalone lowering, it omitted `complete_standalone_hir` before
  MIR validation. The regression now calls that existing finalizer with the
  checked compilation context and original source. It also checks Cranelift,
  LLVM and PHP (with and without supplied MIR, FFI disabled) against the same
  empty stdout/stderr and status 85 for direct and trait-composed source.
  The case exercises guarded generic static calls and observable setter calls
  through concrete and erased receivers; inactive specializations retain no
  static/getter/setter executable target. Andrew approved the offline cold
  verification: one job, ten minutes, 6 GiB RAM and 3 GB cache growth under the
  unchanged aggregate ceiling. The cold build completed in 3m 16s. Fixture
  corrections removed forbidden output from a hook, made the setter update
  stored state, included the setter witness and queried generic admissions
  using the effective method parameter identity. These correct the test;
  production checking/lowering rules were not changed.
  The cold checkpoint had 55 passed, 1 failed and 454 filtered out; nested-trait
  MIR lowering failed I2201 because expanded calls lacked checked expression
  types. Andrew approved the subsequent offline warm pass: one job, ten minutes,
  6 GiB RAM and 500,000,000 additional cache bytes under the aggregate ceiling.
  Checked preparation now validates authored bodies first, retains their callable
  and ownership summaries, and selects expanded bodies from checked admissions.
  Shared contract refinement establishes callback effects at expanded call sites.
  Complete declarations remain available; inactive bodies acquire no execution
  facts. Ordinary checking still selects the complete expanded program.
  Verification exposed and corrected two handoff gaps: retained getter summaries
  were absent from the fresh ownership context, and composed callback effects
  had not undergone refinement at their expanded identities. A test setup error
  also called PHP generation without MIR for captured closures. The matrix now
  preserves captured-closure checks with MIR and adds closure-free variants with
  and without MIR, matching the production PHP backend's closure requirement.
  Latest focused result: 57 passed, 0 failed, 454 filtered out on macOS ARM64 at
  `dev-5e5bdf37517124defa2950e18e66200b16b66f3b11dc1360aad91cc12e57523c`;
  executable SHA-256
  `3690c629fc8f4b51d03544def6f009dda8f4b30b0a7c01c4cdaa049cf3fcc10c`.
  Direct classes and nested traits, with and without captured closures, pass
  interpreter/Cranelift/LLVM and PHP (FFI disabled), with empty stdout/stderr
  and status 85. Assertions preserve distinct receiver/closure identities,
  constrained calls, generic borrowed storage contracts, inactive declarations
  without executable facts, and E0537 for invalid inactive authored bodies.
  Public E0776 remains; checked execution is still test-only. The six sequential
  verification invocations used 525.92 seconds total, peaked at 1,233,928,192
  sampled group RSS bytes, and changed aggregate allocated cache from
  1,104,973,824 to 1,124,098,048 bytes (19,124,224 bytes growth). All processes
  exited; no resource limit was exceeded. No fresh Linux/Windows, full-library
  or workspace pass is claimed. Next work remains the Decision 0139 activation
  matrix and full Stage 36a integration, with existing design/publication holds.
  Three additional actual-source execution regressions are written for widened
  virtual overrides, composed checked-error cleanup and composed writable
  borrowed getters. They share checked preparation, MIR validation and exact
  interpreter/Cranelift/LLVM/PHP result checks. Source review corrected the
  getter fixture to preserve Decision 0135's readonly result while testing
  receiver mutation; cleanup assertions check success and failure separately.
  Andrew approved the separate bounded acceptance pass. All 60 focused tests
  passed, zero failed or ignored, 454 filtered out, on macOS ARM64 at
  `dev-197733eba9210e1c2b3149225d035c547f734051953974fca7da738f83f2e717`.
  The executable SHA-256 is
  `337b2f126829ab56af8c099819d4091268fd763cf2972ff8f821ec5e05798c1e`.
  All three new cases passed interpreter/Cranelift/LLVM and PHP with and without
  supplied MIR, FFI disabled, with exact exit status and empty stdout/stderr.
  Build and tests took 131.56 bounded seconds; the tests took 56.85 seconds.
  Peak sampled group RSS was 1,337,344,000 bytes. Aggregate allocated caches
  changed from 1,107,632,128 to 1,123,061,760 bytes (15,429,632 bytes growth).
  The process exited zero within the approved ten-minute, 6 GiB and 500 MB
  limits. Source identity matched before and after. No Linux/Windows or broader
  suite execution is claimed; public E0776 remains. Tooling is idle and unchanged.
  Stage estimate remains 63%; public E0776 and the remaining full-scope work
  remain open. No installs, dependencies, Git writes or publishing occurred.
  Invalidated elsewhere: compiler-keyed Tooling/Benchmark and website versioned
  guide/API-reference/playground evidence remains pending; this adds no
  public language or editor contract change.

- Aggregate cache resource gate: an earlier final post-verification scan measured
  25,062,342,656 allocated bytes against Andrew's aggregate 25,000,000,000-byte
  ceiling. The periodic verification watcher missed final growth. Andrew then
  approved a selective cleanup extension. The validator now plans and reclaims
  stale, superseded workspace test executables and stale completed-link test
  objects with versioned Cargo dependency proof, exact plan identities, profile
  locks and active-use checks. Completion markers, source freshness, recognized
  object names and singly linked regular files are required; unknown, healthy
  or unfinished outputs stay intact. Its artifact
  policy guard passes on macOS/PHP 8.5.12, including real Cargo lock
  interoperability through a package-clean dry-run; Linux/Windows execution
  remains unverified. One obsolete 68,722,688-byte test executable was removed;
  libraries, incremental data, current executables and installed tools were
  preserved. Whole-target reclamation was not run. A subsequent exact-plan
  application reclaimed 358,842,368 bytes from 107 obsolete test objects and
  preserved the identity/stat hash of all 30,676 retained files. Two fixture
  verification iterations each reclaimed only that test unit's newly obsolete
  objects, with the same retained-file preservation check. This enabled the
  production check and focused receiver tests above. The bounded wrapper now
  requires an explicit growth budget, a 64 MiB aggregate reserve and a final
  measurement on success or failure; missing/oversized budgets launch no child.
  The transport work subsequently reclaimed 186,523,648 bytes from 16 stale
  objects, with all 30,676 retained-file metadata records matching. Its fixture
  correction reclaimed 193,957,888 bytes from another 16 stale objects. That
  application's cache-wide metadata hash changed unexpectedly; the retained
  executable identity/stat and Cargo fingerprint/completion/dependency hashes
  still matched, no build consumer or recent retained-file change was found,
  and subsequent full metadata scans stabilized. The old observation retained
  only a hash, so the exact changed fields cannot be reconstructed; do not
  report this second cache-wide preservation comparison as passed. Future
  preservation observations must retain per-file fields long enough to diagnose
  a difference. The workflow's exact candidate/ownership/lock/use proofs remain
  enforced; no whole-target or healthy-cache cleanup ran.
  Post-run allocated aggregate: 24,773,853,184 bytes; Doria 24,609,230,848;
  Tooling 164,622,336. A later scan measured 24,760,459,264 bytes without another
  cleanup. Both observations left less than the wrapper's 256 MiB launch minimum.
  The ambient-profile correction subsequently reclaimed 183,595,008 and
  195,981,312 bytes from two reviewed sets of 16 obsolete linker objects. Full
  per-file observations matched all 30,676 retained paths after each application;
  compact evidence was retained and both completed observation files removed.
  This does not resolve the earlier hash-only anomaly above. The final aggregate
  is 24,774,144,000 bytes: Doria 24,609,521,664 and Tooling 164,622,336.
  Headroom is 225,856,000 bytes, below the wrapper's launch minimum; further
  builds require verified headroom.
  Healthy caches remain intact; selective cleanup approval is settled.
  No installs, pins, Git writes, publishing or Docker session were started.
  The earlier timeout executable cleanup and
  preservation of the pre-existing test output remain unchanged.

- Covariant virtual-return checkpoint:
  `dev-550105f9171029b35e331a791ee49942c3a731db09538fc210414b511d4c94cd`
  passes 429 compiler unit checks and 317 integration checks across nine complete
  suites and one focused native-receiver group on macOS ARM64/PHP 8.5.12.
  Only the known V-A numeric-result unit fails; zero tests are ignored. Strict
  all-target workspace LLVM Clippy passes. The first batch reached its ten-minute
  cap during region tests; only unfinished suites resumed at the same identity.
  All 28 region, 36 returned-borrow and 91 property-hook checks pass. Five new
  unit regressions cover emitted call signatures, nullable/checked results and
  exact cleanup, borrowed dynamic descriptors, generic owner bindings and an
  inactive root body across interpreter/Cranelift/LLVM/PHP. A structural regression
  first reproduced a one-pointer LLVM call to a two-pointer virtual entry.
  Canonical physical return types now derive from source roots independently of
  body admission; native calls adapt carriers and use success-only stack result
  conversion for checked calls. The interpreter reuses class-view projection.
  Removed prior MIR acceptance of virtual entries with mismatched canonical
  return signatures. No source behavior or test was removed or disabled.
  Code audit: 2,075 files, SHA-256
  `509df5afceaa918c87e556968f65e3f17e496cc4e20c3f3693de843f0a553a93`.
  E0776 remains; remaining provenance/coherence, conditional activation, adapters,
  performance and integration remain open. V-A/M-C/R-S and bridge holds remain
  separate; R-S and fresh Linux/native Windows/PHP 8.4 checks were not run.
  Public APIs, runtime symbols/layouts and metadata schemas are unchanged.
  No installs, pin changes, Git writes or publishing occurred. Full Stage 36a
  remains estimated at 63% under unchanged scope and weights.
  Invalidated elsewhere: compiler-keyed Tooling/Benchmark evidence and website
  versioned guide/API-reference/playground verification. Tooling strings/hovers
  are unchanged; its pin/build/install hold remains.

- Canonical interface-witness checkpoint:
  `dev-34f3ad52c2f68912249a1ad477944ebd0abe5d201e880e1f159d858f0f89f648`
  passes 424 compiler unit checks and 307 integration checks across 13 targets
  or focused groups on macOS ARM64/PHP 8.5.12. Only the known V-A numeric-result
  unit fails; zero tests are ignored. Strict all-target workspace LLVM Clippy
  passes. The ten-minute cap interrupted interface runtime tests; unfinished
  suites resumed unchanged. All 27 focused specialization checks pass, including
  inherited generic witness execution across interpreter/Cranelift/LLVM/PHP.
  Forged targets, absent preparation, missing implementations, changed origins,
  false exact-self proofs and duplicate witness records fail both emission
  boundaries consistently. Removed the prior acceptance of missing implementations
  in checked concrete conformance facts; no test was renamed or disabled.
  One source-owned template supplies exact declaration/owner/method bindings.
  Native entries no longer search member names and ancestors; PHP home, cell and
  field coordination share one checked edge graph. Method/accessor syntax is
  indexed once for virtual and interface identities, retaining owned hook views.
  All 28 region and 36 returned-borrow cases pass; 10 property-interface cases
  pass and the other 81 property-hook cases were not rerun. Conditional grammar,
  body, composition and coherence suites preserve E0776. Code audit: 2,075 files,
  SHA-256 `b7ba16b6dba2b45a4a321aef24cf1d1c5a7f86d73ddf01e8480bc63158d7fa78`.
  Public APIs, runtime ABI, dependencies and metadata schemas are unchanged.
  Conditional activation, remaining provenance/coherence, adapters, performance
  and integration remain open; V-A/M-C/R-S and bridge holds remain separate.
  R-S, Linux/native Windows and PHP 8.4 were not rerun. No installs, pin changes,
  commits or publishing occurred. Full Stage 36a remains estimated at 63% under
  unchanged scope and weights. Invalidated elsewhere: compiler-keyed Tooling and
  Benchmark evidence; website versioned guide/API-reference/playground
  verification. Tooling strings/hovers remain unchanged and its hold remains.

- Canonical virtual-slot checkpoint:
  `dev-a1d6ba104ce3339a99f9ea5498e672a453174828ffb0bc77704fa69d68be657e`
  passes 421 compiler unit checks and 384 integration checks across 11 suites
  on macOS ARM64/PHP 8.5.12. Only the enabled known V-A numeric-result unit
  fails; zero tests are ignored. All 24 focused specialization checks pass,
  including interpreter/Cranelift/LLVM/PHP dispatch, reserved inactive parents,
  unmaterialized generic siblings, direct parent calls and lifecycle calls on
  other objects. Missing checked plans and forged slot/family entries fail
  closed. Strict all-target workspace LLVM Clippy passes. The ten-minute batch
  cap interrupted writeback; only unfinished suites resumed unchanged.
  Canonical root/concrete-owner/type-argument families now originate in source
  checking. MIR keeps reserved entries and explicit class upcasts; shared
  provenance, ownership and call-target analysis retain the narrower receiver
  domain and original storage. Native tables leave reserved words empty, with
  no runtime capability predicate or fallback body. Removed the interpreter's
  caller-wide constructor/destructor dispatch cap; direct `$this` phase calls
  remain encoded by shared lowering. The approved R-B correction still passes
  all 28 byte-region execution checks. The authority guard now requires the
  canonical family fields, root-first ordering and dispatch regressions. Only
  that guard and the three status documents changed after Rust verification.
  The final code audit covers 2,074 files, SHA-256
  `cd3f08f81743167b06ad0c4a18b1cdf26923af0c022ae4ddd556192d1b89d775`.
  E0776 remains. Remaining provenance/coherence, active member/witness
  execution, adapters, performance and integration gates stay open. V-A/M-C/R-S
  and bridge holds remain separate; R-S was not rerun. No fresh Linux/native
  Windows/PHP 8.4 matrix, installs, pin changes, commits or publishing occurred.
  Full Stage 36a remains estimated at 63% under unchanged scope and weights.
  Invalidated elsewhere: compiler-keyed Tooling evidence and website versioned
  guide/API-reference/playground verification. Runtime ABI, public APIs and
  metadata schemas are unchanged; Tooling strings/hovers are unchanged and its
  pin/install/verification hold remains.

- Shared callable admission and generic-call graph checkpoint:
  `dev-1a93341c7a245e489729951ef439639e61b5b8349f4b7b1489d588e91746e921`
  ran the full 417-case compiler unit suite and nine selected integration suites:
  416 unit and 305 integration passes on macOS ARM64/PHP 8.5.12. The enabled
  numeric-result V-A unit and focused indirect scalar-capture R-S integration
  checks retain their known native failures; zero tests are ignored. The
  ten-minute batch cap stopped the contract suite; only the three unfinished
  suites resumed at the same identity. The other 77 returned-closure cases were
  not rerun. A test-only helper move then produced
  `dev-0348bec57f391a37f48736b85f00194088909c2c35628fc44cf5ccfd5bf11dbe`:
  all 19 specialization checks, 74 PHP unit checks and the new concrete-owner
  virtual override parity check pass; V-A remains failing. Strict all-target
  workspace LLVM Clippy passes. The parity case executes the interpreter,
  Cranelift, LLVM and PHP. Earlier
  `dev-bfcc8c45ee5f8b335fb455008dc6f222e31e774e0786a129f79e756cde66f77f`
  passes 153 checks across six generic, inheritance, interface and trait suites
  before the PHP global entry-owner correction.
  Source checking now owns the exact generic-call graph. Shared native/PHP
  materialization consumes canonical declaration/binding keys and erased
  requirement ABI shapes without rediscovering targets. Virtual calls retain
  override-body dependencies, concrete owner families and the actual static
  receiver; direct parent calls and unrelated siblings remain distinct.
  Initializers discovered through factories, constrained interface receivers,
  global calls and global closures have execution regressions. Source ancestry
  still rejects expanding specialization. Guard admission remains separate;
  checked nongeneric witnesses still require their exact concrete owner/ancestor.
  Missing graph preparation fails closed. At that checkpoint E0776 prevented
  conditional execution and inactive-parent slot representation remained pending.
  PHP file-scope statements now share specialized argument plans with callable
  bodies and use MIR's selected entry owner for closure invocation. The PHP test
  helper has one crate-level owner. A separate file-scope `echo identity(7);`
  probe with an empty `main` still panics in native error routing with
  `checked source has a semantic propagation target`; its correction is not
  included. The global generic-root regression uses a pure global call.
  Code audit: 2,073 files, SHA-256
  `e7c230092e0a540b103485f2fbfdd5033f0c4631fa446d49ee8196ef63549917`.
  Runtime ABI, public APIs, metadata schemas, dependencies and Git refs are
  unchanged. No fresh Linux/native Windows or PHP 8.4 matrix ran. Remaining
  provenance, coherence, member/witness execution, adapters, performance and
  integration gates stay open; separate native and Tooling holds remain.
  Full Stage 36a remains estimated at 63% under unchanged scope and weights.
  Invalidated elsewhere: compiler-keyed Tooling evidence; website versioned
  guide/API-reference/playground verification, Benchmark and installed-tool
  integration remain pending.

- Authored dependency and member-target checkpoint:
  `dev-66a2ee4f8c8db2b0832b7caa796bda87e9a25ad36812a27b7f5366ad5a87915d`
  passes 919 distinct macOS ARM64 checks: 394 unit and 525 integration passes.
  The enabled numeric-result V-A unit and indirect scalar-capture R-S integration
  failures retain their recorded native diagnostics; none are ignored. The full
  395-case compiler unit suite and 17 selected integration suites ran. Cargo
  stopped at R-S; only ten unexecuted compiled targets were resumed at the same
  identity. Strict all-target LLVM Clippy passes.
  Building on the authored dependency traversal, trait instance/static calls now
  record deferred targets, preserving concrete generic relay discovery. Live
  source contracts preserve checked generic bindings; return queries apply full
  composing-receiver substitution and select effective getter declarations without
  changing checked plans. Fixed/generic owners, nested aliases, positional/named
  arguments, borrowed/taking Copy, Move and byte-region cases pass.
  The expanded matrix exposed a false static-forwarding E0478, now corrected.
  Recording targets initially hid an imported alias's `take` contract from source
  ownership. The existing E0470 regression caught it; the shared selector now
  reads the authored method table, preserving its original expected diagnostic
  and adding static-alias controls. All 38 public body checks pass and keep
  execution/binding/closure facts empty under E0776. Code audit: 2,072 files,
  SHA-256 `da102502f8d9c1b0ab386c177f7fa75040afd9f11844b0d6c3b1e8dc892e5579`.
  Remaining specialized provenance, coherence, active execution, adapters,
  performance and integration gates remain open. No fresh Linux/native Windows
  or PHP 8.4 matrix ran. Runtime ABI, public APIs, metadata schemas and dependencies
  are unchanged. Builds are stopped; aggregate allocated caches occupy
  24,862,744,576 bytes, below 25 GB without healthy cleanup. No Docker, installs,
  pins, Git ref/index writes or publishing. Separate native and Tooling holds remain.
  Full Stage 36a remains estimated at 63% under unchanged scope and weights.
  Invalidated elsewhere: compiler-identity-keyed evidence and Tooling diagnostic
  verification; website versioned guide/API-reference/playground validation,
  Benchmark and installed-tool integration remain pending.

- Authored-specialization and approved R-B checkpoint:
  `dev-bc74d74b8bc82f2c7bcbc03f67ca7029ec71b41d3cffeb05d367f1d75b26c3cb`
  passes 583 distinct selected macOS ARM64 checks and strict all-target LLVM
  Clippy. Three enabled V-A cases remain failing: the numeric match result and
  two existing `when` result-exit cases; none are ignored. The latter two have
  the previously documented native result-path diagnostic. No result-exit
  lowering or route-validation correction is included.
  Source specialization now uses authored body spans and canonical composition
  substitutions for ownership, closures and returned loans, including fixed
  trait arguments without generic callable bindings. Guarded finite roots are
  collected for diagnostic-only analysis. Copy, Move, byte-region, taking and
  borrowed callback cases, aliases, inactive members and source order are
  covered; public binding/closure/execution facts remain empty under E0776.
  Andrew approved the shared R-B region-provenance correction for compiler
  source and local verification only. Source and MIR now preserve readonly
  descriptor snapshots and their original storage/ancestor loans separately;
  explicit descriptor captures and writable parents remain protected. The
  complete 28-case region execution suite and independent forged-MIR controls
  pass. The original PHP 0/1/1,000-iteration region/cell constructor and
  no-allocating-construct gate passes unchanged with boxed positive controls.
  A previously masked PHP argument wrapper is removed through shared named-call
  statement staging. Yield temporary ownership is selected before operand setup,
  preserving finalizer/destruction order; a regression introduced during this
  work was caught and corrected without changing expected output.
  Final coverage is 118 unit passes and 465 integration passes, with the three
  V-A failures above. Cargo stopped one command at its failing target; only its
  unexecuted compiled targets were resumed. Code audit: 2,072 files, SHA-256
  `bf0a82b9e59cefdeb84c1bd10a061b0dd7ceff536dd251389fba299edadf1857`.
  Verification used PHP 8.5.12; no fresh Linux, native Windows or PHP 8.4 matrix
  ran. Runtime ABI, dependencies, public APIs and metadata schemas are unchanged.
  Builds are stopped; aggregate allocated caches occupy 24,872,574,976 bytes,
  below 25 GB, without healthy cleanup. The owned PHP inspection output was
  removed after compact evidence was retained. No Docker, installs, pins, Git
  ref/index writes or publishing. R-B approval does not release V-A/M-C/R-S,
  native bridge or Tooling source/behavior/build/pin/install holds. Full Stage 36a remains
  estimated at 63% under unchanged scope and weights; remaining specialization,
  coherence, active execution, adapters/processes, performance and integration
  gates remain open. Invalidated elsewhere: R-B failure status and compiler-
  identity-keyed evidence; Tooling diagnostic verification, website versioned
  guide/API-reference/playground, Benchmark and installed-tool integration.

- Guarded dependency-admission checkpoint:
  `dev-0cfe3dbe56c0f3b83500079f41069532d8e84a844e4ec0db78fd623f2346ed22`
  passes 885 distinct selected macOS ARM64 checks and strict all-target LLVM
  Clippy. Two known native failures remain enabled: R-B region rebinding and
  V-A match-result merging; none are ignored. Removed eager executable
  registration from inactive or uninstantiated callable bodies. Methods,
  computed properties and conformance targets retain their declaring conditions;
  admission uses fresh shared nominal proofs. Physical fields and parents stay
  unconditional. Source-only generic declaration inventories preserve disjoint
  stored paths without registering executable instances. Class and callable
  expansion share finite ancestry; enclosing roots are admitted before dependent
  types. A broader runtime fixture exposed a self-cycle introduced during this
  work; explicit root admission and self/mutual-reference regressions correct it.
  Four new internal regressions and one public body regression cover reversed
  discovery, inactive calls, uncalled generics, recursion and class/trait storage
  paths, retaining overlap rejection. The original proof-isolation assertions
  remain, with exact callable and unconditional class positive controls.
  The corrected runtime fixture passed separately and its exact-source result
  was reused in the broader sweep. Code audit: 2,072 files, SHA-256
  `9e457b1d5a6c9f3fb8cc74b33e3ae700d09a4fa68610295c9e796fce73fe7bd3`.
  E0776 remains; full specialized provenance/ownership, active member/witness
  execution, symbolic coherence, adapters and integration remain pending. Native
  ABI, dependencies and metadata schemas are unchanged. Builds are stopped;
  aggregate allocated caches occupy 24,871,858,176 bytes, below 25 GB, with no
  healthy cleanup. No fresh Linux/Windows or PHP 8.4 execution, Docker, installs,
  pins, Git changes or publishing. Full Stage 36a remains estimated at 63% under
  unchanged scope and weights. Invalidated elsewhere: compiler-identity-keyed
  evidence, Tooling diagnostics, website versioned guide/API-reference/playground,
  Benchmark and installed-tool integration.

- Shared authored-ownership checkpoint:
  `dev-be5958c54258698d7a0a2df85a8659ba3103fdea42bc85ae578e595166ff7aac`
  passes 751 distinct selected macOS ARM64 tests and strict all-target LLVM
  Clippy. Two known native failures remain enabled: R-B region rebinding and
  V-A match-result merging; none are ignored. Source ownership uses one context
  for ordinary and guarded programs, canonical composed member contracts and
  refined retained-source types. Guarded class/trait methods and getters now
  reject repeated ownership transfer. Previously unchecked unused trait bodies
  also reject use-after-transfer; their diagnostic-only checks publish no
  executable cleanup or closure facts. Six new tests cover these boundaries,
  valid ownership and borrowed returns, aliases, static calls and source order.
  The coherence fixture now reads its property without requesting unsupported
  ownership move-out; its original conflict assertions remain. Source storage
  collection avoids execution dependency expansion and builtin admission.
  Code audit: 2,071 files, SHA-256
  `89e01a0ed11ce4a21658aa7eb441ac14ae181369474a2ec72058891dacc58eed`.
  E0776 remains; guard-aware execution dependencies, full specialized ownership,
  active witnesses, symbolic coherence and integration remain pending. Native
  ABI, dependencies and metadata schemas are unchanged. Builds are stopped;
  aggregate allocated caches are 24,843,911,168 bytes below 25 GB, with no healthy
  cleanup. No fresh Linux/Windows or PHP 8.4 execution, Docker, installs, pins,
  Git changes or publishing. Full Stage 36a remains estimated at 63% under
  unchanged scope and weights. Invalidated elsewhere: newly diagnosed trait
  ownership errors and compiler-identity-keyed evidence; Tooling diagnostics,
  website versioned guide/API-reference/playground, Benchmark and installed-tool
  integration remain pending.

- Shared callable-contract checkpoint:
  `dev-96be27cfe1449b9295df23b466cd1ada5258577e1268c36922b79975bbcb59f4`
  passes 731 distinct selected macOS ARM64 tests and strict all-target LLVM
  Clippy: 382 compiler unit tests and 349 integration tests. Three known native
  failures remain enabled: R-B region rebinding, V-A match-result merging and
  R-S indirect scalar captures; no tests are ignored. The shared refinement loop
  now covers specialized return loans, retained storage and callback effects.
  Authored contract roots include unused generic bodies; execution cleanup roots
  retain registered specialization checks. Trait provenance reads the same
  authored bodies as their checked facts. Known-pure callbacks lose spurious
  unknown-call I/O effects; unknown and I/O callback controls still pass.
  A ten-minute batch stopped with three closure tests unfinished; those tests,
  the stream suite and Clippy completed separately without repeating completed
  suites. The scalar failure was rerun only to confirm its existing MIR error.
  Code audit: 2,070 files, SHA-256
  `f2f4ea1a85c22290e478a395be2347f632290cc9a246c6311b5ff20028938160`.
  Runtime ABI, public APIs and metadata schemas are unchanged. E0776 still
  fences execution; specialized ownership checking, active witnesses and the
  symbolic coherence ruling remain pending. No fresh Linux/Windows or PHP 8.4
  execution, installs, pins, commits or publishing. Full Stage 36a remains
  estimated at 63% under unchanged scope and weights. Invalidated elsewhere:
  inferred callable facts and compiler-identity-keyed evidence; coordinated
  Tooling, website guide/API/playground, Benchmark and installed-tool integration
  remain pending.

- PHP hosted-region field checkpoint:
  `dev-8257ff9c87d561bd8b2072666b43e81231b1e0c90d7e50bac28dc16dd50a946a`
  passes 113 selected macOS ARM64 checks and strict all-target LLVM Clippy.
  The two selected native failures R-B and V-A remain enabled; none are ignored.
  Coverage includes 72/74 PHP emitter units, 35/35 transfer groups and 6/6 group
  controls. Build-free PHP 8.4.21/FFI passes 33/34 selected checks with the same
  pre-PHP R-B failure; PHP 8.5.12 is the main environment. Removed the duplicate
  current-region array and per-resubmission descriptor construction. Existing
  typed region homes now own the current window; immutable operation roots,
  stable callback homes and boxed compatibility access are preserved. Completed
  region access publishes through the same checked field-result channel.
  The strengthened 0/1/1,000-iteration probe keeps every measured constructor
  count fixed for empty transfers and real transfers with root offset 1,
  offsets 0/1/2 and lengths 3/2/1. It checks initial bytes, EOF prefixes and exact
  shifted writes, retaining boxed positive controls. Original callback-window
  parity and legacy hosted checks pass. An unsupported remainder operator in
  the new fixture was replaced by an equivalent explicit offset cycle.
  The main batch took 431.43 seconds, peak sampled RSS 1,137,246,208 bytes;
  PHP 8.4 checks took 21.03 seconds, peak RSS 117,260,288 bytes. Tested code
  audit: 2,069 files, SHA-256
  `34a160103835f48393ab81036cc63240f1c1432181a0e19154e70864e210a6ba`.
  Only module and pipeline documentation changes after verification. Native
  sources/ABI, dependencies, public APIs and metadata schemas are unchanged.
  Nonzero-offset FFI address wrappers and the full Zend/FFI allocation/copy gate
  remain open; constructor probes do not prove total host allocation freedom.
  Builds are stopped; allocated aggregate caches are 24,827,314,176 bytes below
  25 GB, with no healthy cleanup. No fresh Linux/Windows execution, Docker,
  install, pin, Git or publication changes occurred. Tooling source/behavior/
  pin/install holds persist. Full Stage 36a remains estimated at 63% under
  unchanged scope and weights. Invalidated elsewhere: generated PHP/private
  transport and compiler-identity-keyed evidence; installed refresh and Benchmark
  integration. Website versioned guide/API-reference/playground verification
  and conditional-capability integration remain pending.

- PHP region dispatch checkpoint:
  `dev-1423552af793fca055239769a1a47f3e1ce375ec4835ae9ba0f131dbcfd50219`
  passes 311 selected macOS ARM64 checks and strict all-target LLVM Clippy;
  three selected tests retain native failures: R-B readonly descriptor rebinding after
  a typed callback, V-A result merging, and R-S indirect scalar captures. None
  are ignored. Build-free PHP 8.4.21/FFI passes 74 selected checks with the same
  two native unit failures; the main environment uses PHP 8.5.12. Shared dispatch
  families now compare payload identities from the checked field inventory,
  including region records. Pending operations select the matching field
  argument/result entry. Direct and erased file-read loops keep all measured
  constructor counts fixed at 0/1/1,000 iterations, with boxed positive controls.
  Pending-entry checks cover scalar arity, tag/presence types and both directions.
  Writable callback slot replacement, return aliases, regions, transfer groups
  and closure regressions pass except the named native failures. Original callback
  windows remain correct. Native sources, ABI, dependencies, public APIs and
  metadata schemas are unchanged. R-B correction approval is pending; its test
  remains enabled. Hosted resubmission/read-region boxing and nonzero-offset FFI
  address projection still leave the full allocation/copy gate open. Constructor
  probes do not measure every Zend/FFI allocation. Four final sequential batches
  took 1,472.14 seconds; peak sampled process-group RSS was 1,166,622,720 bytes.
  Code audit: 2,069 files, SHA-256
  `16411827cacfac4136e4b10b452cb9a916505fae7d25508bef81affa8c7e813f`.
  Builds are stopped; allocated aggregate caches are 24,822,726,656 bytes below
  25 GB, with no healthy cleanup. No fresh Linux/Windows execution, Docker,
  install, pin, Git or publication change occurred. Tooling source is unchanged;
  its behavior/pin/install holds persist. Full Stage 36a remains estimated at
  63% under unchanged scope and weights. Invalidated elsewhere: generated PHP
  and compiler-identity-keyed verification/performance artifacts; installed
  refresh and Benchmark integration. Website versioned guide/API-reference/
  playground verification remains pending.

- Byte-region callback ownership guard:
  `dev-25f56cb2328cceb3a52a524b262951d5b6828160be0edc649805fa66c9924a4e`
  passes the new interpreter/Cranelift/LLVM/PHP parity regression on macOS ARM64.
  Preparation and completion each rebind their writable region parameter to an
  empty subregion. Three submissions use different offsets and lengths; completed
  prefixes and the original buffer remain correct. Existing transport already
  isolates these parameter rebindings; no production correction was needed.
  This guards the shared descriptor work against mutable-object recycling that
  would change existing aliases. Regions remain immutable snapshots with tracked
  storage provenance. Shared field/argument/result transport and the full PHP
  allocation/copy gate remain open; no allocation reduction is claimed here.
  The bounded parity run took 117.38 seconds, peak sampled process-group RSS
  1,279,082,496 bytes. Strict LLVM-enabled Clippy for the changed test target
  passes in 20.14 seconds, peak RSS 1,176,666,112 bytes. No tests were ignored.
  Only the regression and this note change; production sources, native ABI,
  dependencies, metadata and public APIs are unchanged. Earlier 403-case evidence
  belongs to the preceding checkpoint and was not rerun. Code audit: 2,069 files,
  SHA-256 `1a0083f3d9c85d49b9851538393dbc6c82a0cb76a7ee922a722618753352d4be`.
  No new Tooling diagnostic/hover/syntax impact; behavior, pins and installs remain
  held. No fresh Linux/Windows execution, Docker or publication occurred. Builds
  are stopped; aggregate allocated caches are 24,790,511,616 bytes below 25 GB.
  No healthy cache was cleaned. Full Stage 36a remains estimated at 63% under
  unchanged scope and weights. Invalidated elsewhere: compiler-identity-keyed
  verification/performance artifacts; installed refresh and Benchmark integration.
  Website versioned guide/API-reference/playground verification remains pending.

- Concrete active-target checkpoint:
  `dev-0d5378c27d635cb5f1cfc43a373831cce6b2749b283930f2417b85850bb44dce`
  passes 403 selected macOS ARM64 tests, none ignored, and strict all-target
  LLVM Clippy. The bounded sequential run took 489.72 seconds; peak sampled
  process-group RSS was 1,253,277,696 bytes.
  Shared formed-type checking follows every established active conformance-target
  argument alongside unconditional concrete storage. Nested targets, interface/
  class inheritance, compound arguments and mixed storage paths share the worklist,
  frozen caller premises and existing recursion identity. Conflicting active paths
  retain their arguments independently of canonical-entry selection. Stable
  recursion and finite collapse remain accepted; pure checks register no
  executable specialization or witness. The CLI now reports the previously missed
  nested E0752, preserves unconditional E0539 and restores that same diagnostic in
  the checking-only conditional path. E0776 remains. Symbolic dependency expansion,
  the symbolic-coherence ruling and specialized member/witness closure remain open.
  Code audit: 2,069 files, SHA-256
  `02af05ccb1bc36e2838908046d1492d230eb36f39a4b10f2fd4ceb3d31f4e03b`.
  Only Decision 0139, the master plan and this note change after verification.
  Native MIR/runtime ABI, dependencies, public APIs and metadata schemas are
  unchanged. Tooling source
  `dev-736fb2dd283221298bf19d546b3498f8ff666cb6d489864351359cb1a9cbca8b`
  adds 18 inspected target/inherited/target-to-storage diagnostic scenarios.
  Formatting and whitespace checks pass; behavior, builds, pins and installs
  remain held. No fresh Linux/Windows execution, Docker or publication occurred.
  Builds are stopped; aggregate allocated caches are 24,751,546,368 bytes below
  25,000,000,000 bytes. No healthy cache was cleaned. Full Stage 36a remains
  estimated at 63% under unchanged scope and weights; ownership/effects,
  member/witness execution, adapters and portable/performance/integration gates
  remain required. Invalidated elsewhere: Tooling diagnostics and installed tools;
  website versioned guide, API reference, diagnostic pages and playground;
  conditional metadata and compiler-identity-keyed Benchmark evidence.

- PHP trivial Copy operand snapshot checkpoint:
  `dev-46d57b6a77d185a1e3ece56c632d78ce3b0f55f8710fe3d0e5adac9dc6e6e201`
  passes 107 selected macOS ARM64 checks and strict all-target LLVM Clippy.
  The enabled V-A native result-merge test remains the sole failure (70/71 PHP
  emitter cases); six group-control, eight completion and 23 Time cases pass.
  None are ignored. The sequential run took 434.84 seconds, peak sampled
  process-group RSS 1,365,360,640 bytes. Build-free PHP 8.4.21/FFI passes all
  29 stream and three field-transport cases in 21.06 seconds; the main run uses
  PHP 8.5.12. Generated-code inspection first located two getter wrapper closures
  in the completion loop. Shared statement operand staging now uses existing
  nominal arity/lifecycle classification for all eight trivial inline families,
  including nullable values; managed tokens and ordinary classes remain excluded.
  Removed those generated getter wrapper closures. The strengthened mixed loop
  rejects function/arrow/new constructs, while the new eight-family regression
  checks ordering, lazy/null/coalesce paths and zero transport constructors.
  Resubmission still creates two region wrappers per iteration. These checks
  do not measure every host/FFI allocation; the full performance gate stays open.
  Code audit: 2,069 files, SHA-256
  `5f16adcb4dcaee4783cfc6509e5b2bb483ac261485927a4cd639dd0951b151de`.
  Only this note changes after verification. Native MIR/runtime ABI, dependencies,
  public APIs and metadata schemas are unchanged. No new Tooling source/hover
  change; existing behavior/pin/installed-refresh gates remain held. No fresh
  Linux/Windows execution, Docker, install or publication occurred. Builds are
  stopped; aggregate allocated caches are 24,807,403,520 bytes below 25 GB.
  No healthy cache was cleaned. Full Stage 36a remains estimated at 63% under
  unchanged scope and weights; conditional execution, adapters and remaining
  portable/performance/integration gates remain required.
  Invalidated elsewhere: generated PHP/private transport artifacts and compiler-
  identity-keyed performance evidence; installed tools and Benchmark integration.
  Website versioned guide/API-reference/playground verification remains pending.

- PHP completion field transport and equality checkpoint:
  `dev-2cddb5f0f2c7c433616e27e117caac4cf1e806bfdba902689ace3ac80ef67578`
  passes 108 selected macOS ARM64 checks and strict all-target LLVM Clippy;
  one enabled V-A test still fails after PHP produces its exact expected cleanup
  output because unchanged native MIR rejects the result merge. None are ignored.
  Coverage includes 69 PHP emitter cases (68 pass), six group-control cases,
  eight completion cases, 23 Time contracts and three enum compatibility cases.
  The two final sequential runs took 136.77 and 332.27 seconds, peak sampled
  process-group RSS 1,252,278,272 bytes. Build-free PHP 8.4.21/FFI passes all
  28 stream and three field-transport cases in 20.02 seconds; PHP 8.5.12 is the
  main environment. Early emitter-binding/fixture mistakes were corrected.
  Immutable directional keys belong to typed group operation records; checked
  native observation still decides availability. Foreign-set startup preserves
  the original Configure/TransferGroup error and releases loans without I/O.
  Removed the private boxed completion builder. Completion/Wait intrinsics use
  shared field argument/result transport and checked nested enum writers;
  constructors use checked case identities. Shared field equality preserves
  active payloads, nulls, evaluation order, lazy operands and floating edges.
  The 0/1/1,000-iteration mixed-direction probe constructs no result enum objects
  and keeps key/other constructor counts fixed, with boxed positive controls.
  Resubmission still constructs two region wrappers per iteration. Constructor
  probes exclude arrays, closures, FFI and implicit objects; the full host
  allocation/copy/performance gate remains open.
  Code audit: 2,069 files, SHA-256
  `aa2978ea2b9a8187885fae02f6315fef384ded1e5b9fffa6903495ec2df97ba9`.
  Only this note changes after verification. Native MIR/runtime ABI, dependencies,
  public APIs and metadata schemas are unchanged. No new Tooling source/hover
  change; its existing behavior/pin/installed-refresh gates remain held.
  No fresh Linux/Windows execution, Docker, install or publication occurred.
  Builds are stopped; allocated aggregate caches are 24,808,087,552 bytes below
  25,000,000,000 bytes. No healthy cache was cleaned. Full Stage 36a remains
  estimated at 63% under unchanged scope and weights; conditional execution,
  adapters and remaining portable/performance/integration gates remain required.
  Invalidated elsewhere: generated PHP/private transport artifacts and compiler-
  identity-keyed performance evidence; installed tools and Benchmark integration.
  Website versioned guide/API-reference/playground verification remains pending.

- PHP scalar wait-word checkpoint:
  `dev-82d217055aa76fb203308fde9023850b162423a14434a4488376e3769d5411d1`
  passes 56 selected macOS ARM64 checks, none ignored, plus strict all-target
  compiler LLVM Clippy. Coverage is 26 PHP stream tests, 23 Time contracts,
  six transfer-control tests and the existing all-four-Wait completion parity
  case. The bounded sequential run took 313.01 seconds, peak sampled
  process-group RSS 1,349,189,632 bytes. The same built library test binary
  passes all 26 stream tests with installed PHP 8.4.21/FFI in a build-free
  16.82-second pass, peak RSS 88,326,144 bytes; the main run uses PHP 8.5.12.
  Trivial inline carriers expose scalar low/high words from their existing
  eight/sixteen-byte representation. Managed Copy keeps its owning transport.
  The shared hosted Wait decoder owns the four wait-kind choices; Deadline
  startup and observation read scalar words without temporary arrays, spread,
  reference cells or shared scratch. Removed the private `__doriaWaitWords`
  array bridge and unused allocating `__doria_transfer_wait_budget` factory.
  The new regression checks all eight trivial carrier families, signed-bit
  extremes, width padding and emitted word/decoder bodies. This proves removal
  of those explicit allocation constructs, not total host-allocation freedom.
  Hosted completion keys/outcomes/result boxing still needs the full allocation
  gate. Earlier 395-case evidence is a separate checkpoint; that entire batch
  was not rerun after these three PHP codegen changes.
  Code audit: 2,069 files, SHA-256
  `fd13b16d2359dd82fee0974fac9133f499e8e253403f40870bf366f9968a2105`.
  Only this working note changes after verification. Native MIR/runtime ABI,
  dependencies, Doria APIs and public metadata schemas are unchanged. Time IDE
  coverage passes; no new editor hover or source-visible member is introduced.
  Tooling behavior/pins, installed refresh and publishing remain held. No fresh
  Linux/Windows execution, Docker session or download occurred. All builds and
  tests are stopped. Aggregate allocated caches are 24,781,426,688 bytes below
  25,000,000,000 bytes; no healthy cache was cleaned.
  Full Stage 36a remains estimated at 63% under unchanged scope and weights.
  Conditional execution, ownership/coherence, adapters and all remaining
  allocation/performance, portable and integration gates remain required.
  Invalidated elsewhere: generated PHP/private word-bridge artifacts and
  compiler-identity-keyed performance evidence; installed tools and full
  Benchmark integration. Website versioned guide/API-reference/playground
  Stage 36a verification remains outstanding; no public spelling changes.

- Concrete stored-type checkpoint:
  `dev-51281af9187712edf952d0a2d9f8b2d1b3629bb82ca137c96ac1200a7d059622`
  passes 395 selected tests, none ignored, and strict all-target LLVM Clippy on
  macOS ARM64. The final bounded sequential pass took 484.28 seconds; peak
  sampled process-group RSS was 1,258,389,504 bytes. Earlier fixture/assertion
  mistakes and a duplicate recursion finding were corrected; no assertion was
  weakened and no test was disabled.
  Shared formed-type checking now follows unconditional concrete storage through
  nested fields, parents, trait-provided state and backed properties without
  member access. Computed properties remain outside storage traversal. Pure
  substitution registers no executable class or witness. Storage and metadata
  discovery share the existing expansion rule and one source-ordered cycle
  identity: duplicate E0539 reports for a mutual component coalesce, independent
  components retain separate findings, and stable recursion/finite collapse
  remain accepted. Existing owned-static rejection E0486 remains enforced.
  A CLI probe confirms the previously missed stored-field conflict now receives
  E0752. At that checkpoint, the active-conformance-target probe still missed its
  nested concrete conflict; the later active-target checkpoint above closes that
  concrete gap. Specialized member/witness closure remains open.
  The symbolic-coherence ruling is still pending; E0776 remains.
  Code audit excludes only SPEC, AGENTS, this note, Decision 0139 and the master
  plan; all other verification inputs remain unchanged after documentation.
  Audit: 2,069 files, SHA-256
  `a6fef7c5b3a3021b26f29858f04b8518b4383ec049890cf5ce7947995a915a16`.
  Native MIR/runtime ABI, dependencies and JSON schemas are unchanged. No fresh
  Linux/Windows verification, Docker session, installation or publication occurred.
  Tooling source
  `dev-4f00c645ee8dc157a3996007c7857efa7f78d5b58a747a66763e9715f1471b71`
  adds eight inspected stored-type scenarios. Compiler-owner formatting and
  whitespace checks pass; behavioral verification, pins and installs remain held.
  All builds are stopped. Aggregate allocated caches are 24,728,875,008 bytes,
  below the 25,000,000,000-byte ceiling; no healthy cache was cleaned.
  Full Stage 36a remains estimated at 63% under unchanged scope and weights.
  Ownership/effects, member/witness execution, adapters, performance and portable/
  integration gates remain required. Invalidated elsewhere: Tooling diagnostics
  and installed tools; website versioned guide, API reference, diagnostic pages
  and playground; full conditional metadata and Benchmark integration.

- Formed-type conflict checkpoint:
  `dev-c2b02474ca248946a32f8cea36e8e7f87bf1ce856ad1cc02091fe56288d0689f`
  passes 385 selected tests, none ignored, and strict all-target LLVM Clippy on
  macOS ARM64. Two bounded sequential passes took 229.34 and 258.36 seconds;
  peak sampled process-group RSS was 1,189,904,384 bytes.
  Formed source types, registered substitutions and published inferred/contextual
  expression types retain frozen caller premises, excluding transient proof
  state. Established active conflicts receive E0752 outside iteration;
  incoherent predicate arguments lose their condition fact.
  The guard-only follow-up
  `dev-c4013e055cd779c386c7b596585bafbfd6b8e00e0124c3c659c66380f8380ccf`
  also checks receiver-substituted availability subjects/interface arguments.
  Its 123 affected conditional/type tests and strict checks pass in 232.83
  seconds, peak sampled RSS 1,378,091,008 bytes. The earlier broader evidence is
  retained for unchanged paths; those suites were not rerun after this follow-up.
  A CLI probe confirms the composed-guard conflict now receives E0752 plus E0776.
  Code audit: 2,069 files, SHA-256
  `5c2b3f904d70790f7bb3a32eb74f70e1f3d8dcfdf71db5440c32d720fca27284`.
  Native MIR/runtime ABI, dependencies and JSON schemas are unchanged. Tooling
  source `dev-52766899f07a94f0e34f209a586f29061883809cb58cdb7af73b8d45e52b7c0a`
  adds eight inspected formed-type scenarios; behavior, pins and installs remain
  held. No fresh Linux/Windows evidence, Docker session or publication action.
  Builds are stopped; aggregate caches are 24,785,141,760 bytes below 25 GB.
  Full Stage 36a remains estimated at 63% under unchanged scope and weights.
  Symbolic overlap is unresolved: `Adapter<T>` with Root<int> under Ready and
  Root<string> under Writable must reject concrete Both, but a Ready-only
  symbolic premise does not establish absence of Writable. Andrew is asked to
  choose explicit residual coherence obligations (recommended) or declaration-
  time overlap rejection. Neither is implemented as an approved rule; E0776
  remains. Full specialized provenance/ownership, member/witness execution,
  adapters, performance and portable/integration gates remain open.
  Invalidated elsewhere: previously missed formed/normalized-guard conflicts,
  Tooling and installed tools; website versioned guide, API reference, diagnostic
  pages/playground; conditional metadata and Benchmark integration.

- Conditional conformance-path checkpoint:
  `dev-d442e754d7a12e11ce68b0103b35367411d1ffdc4092fa7d796c94868362da96`.
  Checked declaration types own source validity; the cache retains declared paths
  and guards, while each caller proves availability separately. Unused/inactive
  exact source duplicates receive E0752 before lookup. Distinct authored paths
  can coalesce after equal substitution; active distinct specializations remain
  invalid and supply no iteration element. Conditional inherited overlap and
  interface diamonds coalesce. The order-dependent ancestor/direct-list
  rejection is removed; exact direct repeats and unconditional parent-class
  redundancy remain errors. Proof regressions select canonical callable
  identities instead of arbitrary expanded predicates.
  macOS ARM64 passes 376 selected tests, none ignored, and strict all-target
  compiler LLVM Clippy in 473.18 seconds, peak sampled RSS 1,226,014,720 bytes.
  The unused-declaration CLI probe now reports E0752 alongside E0776. Code audit:
  2,068 files, SHA-256 `642fcdbeff2efd0b4cb35fec41145764ab7a5dade4e579e67ec18c8c37739fcd`.
  Native MIR/runtime ABI, dependencies and public JSON schemas are unchanged.
  Tooling source preparation at
  `dev-a8fa49a12397d5ea8e7b6abe64fc7e112f55eefbbdc1d644918ff49aca2711bc`
  adds seven inspected diamond/conflict/unused-duplicate scenarios. Behavioral
  verification, pins and installed refresh remain held. No Docker or publication
  action occurred; builds are stopped. Allocated aggregate caches are
  24,735,363,072 bytes below 25 GB; no healthy caches were cleaned.
  Full Stage 36a remains estimated at 63% with unchanged scope and weights.
  Full specialization coherence, ownership/provenance, active members/witnesses,
  execution, adapters, performance and portable/integration gates remain open.
  Invalidated elsewhere: prior cache/duplicate/iteration admission assumptions,
  Tooling diagnostics and installed tools; website versioned guide, API reference,
  diagnostic pages and playground; conditional metadata and Benchmark integration.

- Conditional imported trait-member checkpoint:
  `dev-6a3a46dfbe47587a4fbc68085e6658142a71c1ec8261085a1f5e2145fe95407a`.
  The shared composer builds lexical trait-member views with checked
  substitutions, aliases, exclusions and distinct class/trait owners. Imported
  method/getter/static-generic calls prove their conditions; loan/effect
  summaries reach forwarding traits and classes. Checking-only view identities
  do not become public declaration facts. Conflicting contributions remain
  unresolved; trait-authored members gain no class-authored precedence.
  The macOS ARM64 batch passed 364 selected tests, none ignored, and strict
  all-target compiler LLVM Clippy in 488.40 seconds with 1,206,435,840-byte peak
  sampled RSS. Native MIR/runtime ABI, dependencies and JSON schemas are
  unchanged. Fresh Linux/Windows coverage and conditional execution remain
  unverified. No Docker session, installation, pin, ref or publication action
  occurred. Aggregate allocated caches are 24,700,747,776 bytes, below 25 GB;
  no healthy caches were cleaned and the build is stopped.
  Tooling preparation at
  `dev-09222c0147f835fbb83d907fb973b4a03288b1a800a0fc221a0cd595e32bd8ac`
  adds eight scenarios across four imported-call families; the compiler owner
  inspected them. Presentation assertions now compare authored source
  coordinates, excluding private expansion-ID equality; exact LSP ranges remain
  checked. Behavioral verification and the existing compiler pin remain held.
  Full Stage 36a is estimated at 63%: implementation 82% of its 50% weight,
  verification 55% of 30%, integration 25% of 20%. Scope and weights are
  unchanged. Coherence, specialized ownership/provenance, active members/
  witnesses, execution, adapters, performance and portable/integration gates
  remain open; E0776 remains. No delivery claim is made.
  Invalidated elsewhere: the prior imported-call checking gap, Tooling
  diagnostics and installed tools; website versioned guide, API reference,
  diagnostic pages and playground; conditional metadata and Benchmark work.

- Conditional composed class-caller checkpoint:
  `dev-287537c32eaf4a556f90c593f0199bced18b06c646ff4f4858db8ac7b7b8bacd`.
  Effective class declarations serve member lookup; authored bodies retain
  their lexical parameters and premises. Shared callable-template mappings
  project return loans and ambient effects, including aliases, computed
  getters, returned closures and caught-effect refinement. The macOS ARM64
  batch passed 352 selected checks, none ignored, and strict all-target compiler
  LLVM Clippy in 490.07 seconds; peak sampled RSS was 1,139,982,336 bytes.
  The diagnostic-only follow-up
  `dev-3ef16f9c7d80529cab9711fffb12927a29367acf5e77a1d5cf4882daf899b9b1`
  fixes E0531's private generic-name leak through the existing authored-name
  helper. Its 104 selected checks and strict compiler LLVM Clippy pass in
  196.01 seconds with 1,231,257,600-byte peak sampled RSS. An inverse source
  audit proves only that rendering leaf and its regression changed after the
  core batch; the two checkpoints cover 358 unique selected tests.
  Tooling preparation at
  `dev-4b4e9bca43e99de476f42e9fdc0e7061f2c7a4b19bcabfabbf44bb3dd1909ac6`
  adds one protocol matrix/ten scenarios; the compiler owner inspected it.
  Behavioral verification and the existing compiler pin remain held.
  A negative probe confirms imported calls inside authored trait bodies still
  miss caller-condition checking; the shared trait-member view is next work.
  E0776 still fences execution. Full specialized provenance/ownership,
  coherence, active witnesses, adapters, performance and portable/integration
  gates remain open. Stage 36a remains estimated at 62% with unchanged scope
  and weights. Native MIR/runtime ABI and JSON schemas are unchanged; no Linux
  or Windows rerun, installation, pin, ref or publication action occurred.
  Invalidated elsewhere: source caller diagnostics and summaries, Tooling and
  installed compiler/LSP, website versioned guide, API reference, diagnostic
  pages and playground; conditional metadata and Benchmark integration remain
  required before delivery.

- Conditional trait-predicate composition checkpoint:
  `dev-955ff49de200fa4172d74932d4f6d1aafc805f3a5cd9e1214ae79ccc72df4eae`.
  Checked trait-use arguments substitute resolved subjects and interface types
  while retaining authored parameter identities and spans. Nested uses,
  composing-class `self`, method generics, aliases, diamonds and computed
  accessors share one condition plan. False predicates remain conditional;
  unresolved clauses retain unavailable callable markers and invalidate
  composition. Generic isolation now covers nongeneric traits too, removing
  the false E0530 rejection when a composing class uses the same parameter name.
  Authored shadowing still receives E0530. The ordinary regression produces
  `box;7;plain;` on the interpreter, Cranelift, LLVM and PHP 8.5.12.
  The macOS ARM64 batch passed 310 unique selected checks, none ignored, plus
  strict all-target compiler LLVM Clippy. The focused execution case runs once
  before the broader batch. Native MIR/runtime ABI and JSON schemas are unchanged.
  The metadata-only follow-up
  `dev-73108299ed1699129261e94b4c8f3b5fa47764b4ef228a8c4b7e83d337fafc5a`
  restores authored generic names in the shared public renderer; frozen-schema
  and contract verification passed 58 checks plus strict compiler LLVM Clippy.
  The two checkpoints cover 314 unique selected checks. Source audit confirms
  no other compiler/runtime/dependency edit in the metadata-only follow-up.
  The core batch took 528.53 seconds with 1,136,623,616-byte peak sampled RSS;
  the metadata follow-up took 317.95 seconds with 1,250,476,032-byte peak RSS.
  Final allocated aggregate caches are 24,708,562,944 bytes, below Andrew's
  25,000,000,000-byte ceiling. No caches were cleaned and no build remains active.
  Performance Impact: predicates and substitutions remain compile-time state;
  no runtime dictionary, allocation, copy or dispatch mechanism is added.
  Full stream performance/allocation gates remain open.
  Tooling source preparation at
  `dev-be9e9e6396d2cfe93cdec9a559f99c9383592c51b578556ad6adfbcdb896d2e2`
  adds three protocol tests/seven scenarios; the compiler owner inspected them.
  Behavioral verification and the `f500042e1546a7282311423c31164131536ed1e1`
  pin remain held. Stage 36a stays estimated at 62% under unchanged scope/weights.
  Composed bodies/calls, refinement, coherence, active witnesses, adapters and
  portable/integration gates remain open; E0776 still fences execution.
  Installs, pins, staging, commits and publishing remain held.
  Invalidated elsewhere: trait generic admission and metadata rendering,
  Tooling diagnostics, installed compiler/LSP, website versioned guide,
  API reference, diagnostic pages and playground; conditional metadata and
  Benchmark integration remain required.

- Symbolic nominal type-formation checkpoint:
  `dev-4edea89f227993f2f43d5c6b71d5985fa9cca3ecef27952f2606c5df872c5f99`.
  Class/interface arguments, parent/trait types, guarded signatures and
  conformance targets prove substituted bounds under their own premises.
  Nested predicate types use the complete conjunction; malformed inactive
  predicates do not survive as checked conditions. Pure identity queries cannot
  register executable instantiations or templates. Class discovery ancestry
  now belongs to the shared checker across metadata/returned-borrow fixed
  points. Direct/mutual expanding recursion retains E0539; stable recursion
  and finite collapse remain accepted. This fixes the observed stack overflow
  without an arbitrary expansion-depth cap.
  D20 is enforced at named-signature collection with E0617. Legacy named-return
  type inference and its separate resolver are removed; arrow inference and
  returned-borrow provenance remain. Ownership tests now distinguish borrowed
  forwarding from ownership transfer. The coalesce rejection assertion uses
  source E0471 instead of internal B0001; PHP specialization assertions follow
  the inspected typed fields/constructors instead of host promotion syntax.
  macOS ARM64 verification passed 829 unique targeted checks, none ignored:
  759 affected regressions, 13 proof-kernel, 15 type-formation, 41 generic-class
  and one generated-error check. The factory case passes exact interpreter,
  Cranelift, LLVM and PHP 8.5.12 output/cleanup. Fixture-only follow-up
  `dev-bad61cbfd1b3a6da4848558975142a7da433e1d9d8aedfae7356f1e6f82865e4`
  passes the 61 ownership and 41 generic-class checks plus strict all-target
  LLVM Clippy. Source audit confirms no other compiler/runtime/dependency change
  between the runs. Native MIR/runtime ABI and public JSON schemas are unchanged.
  Performance Impact: the discovery/proof state is compile-time-only; no new
  runtime allocation, copy or dispatch machinery is introduced. The full stream
  allocation/performance gates remain open.
  Tooling source preparation at
  `dev-d16989648fa12c5aa35c9af0c9642976fbe6f091cb9ab1128e592bf01c1a0b69`
  includes type formation, D20 and four recursion scenarios; the compiler owner
  inspected them and the relevant hover claims. Behavioral verification and its
  `f500042e1546a7282311423c31164131536ed1e1` pin remain held.
  Full Stage 36a is estimated at 62% using the existing 50/30/20 weights for
  implementation/verification/integration (81/54/25). The increase reflects
  the completed source-type-formation gate. Conditional execution, refinement,
  composition/coherence, adapters, portable gates and integrations remain open.
  Installs, pins, staging, commits and publishing remain held.
  Invalidated elsewhere: source-bound and named-return checking, Tooling
  diagnostics, installed compiler/LSP, website versioned guide, API reference,
  diagnostic pages and playground. E0776 still fences conditional execution.

- Exact core-capability admission checkpoint:
  `dev-da76f6e8691037ce69050c7ead968715db98fd2376883329f2a6541ab7e5bd56`.
  Display conversion, generic core bounds, Error admission and concrete backend
  class facts share exact-type nominal proofs. Declaration-level capability
  flags and name-only class traversal are removed; inactive specializations
  now fail the existing conversion, bound and Error checks. Guarded Error
  message reads use scoped premises, and nullable symbolic receivers reach
  nominal member lookup without gaining unproved members. Both generated-error
  owners supply nominal edge origins through the shared synthetic-declaration
  constructor. This preserves I/O and assertion Error reporting metadata.
  The final macOS ARM64 batch passed 343/343 targeted checks, none ignored,
  plus strict all-target LLVM Clippy: 15 body/caller, 22 declaration, 12 kernel,
  one generated-error graph/MIR check and 293 affected regressions. It took
  420.00 seconds, peak sampled process-group RSS 1,095,843,840 bytes.
  The new ordinary generic/nullable Error message case passes interpreter,
  PHP 8.5.12, Cranelift and LLVM with exact output and empty stderr. Verification
  exposed missing generated edge origins and nullable symbolic admission;
  both were corrected. The generated-error fixture uses the existing
  development-source graph so AssertionError retains its scope restriction.
  Native MIR/runtime ABI and public JSON schemas are unchanged. Conditional
  executable facts remain empty behind E0776. Tooling prepared six protocol
  tests/eight scenarios at
  `dev-3a89c098efb4fee60ad85098d987f6b53ae1b8413e4ab1512809b343d0b64ab1`;
  the compiler owner inspected the source. Its pin remains
  `f500042e1546a7282311423c31164131536ed1e1`; builds/behavioral verification,
  installs, pins, staging, commits and publishing remain held. Fresh Linux and
  Windows coverage, full conditional execution and required integrations remain
  open. Overall Stage 36a remains estimated at 61% under unchanged scope/weights.
  Invalidated elsewhere: Tooling protocol/API consumers, installed compiler/LSP,
  website versioned guide, API reference, diagnostic pages and playground.

- Guarded body/caller-condition checkpoint:
  `dev-d32f7551dbe1bd6024a664aaf1e8ba00df2906e7a51fdf70fb164a50fa01fd12`.
  The conditional prepass shares the normal callable-body phase. Own clauses
  supply scoped nominal/member/core-value premises and do not leak to sibling
  or free bodies. Method/accessor calls freeze caller facts and prove the
  callee clause after argument/result inference. E0779 is a language finding
  at the call/access, with related authored clause and typed cycle origins.
  Computed read-modify-write coalesces its property's one condition; unresolved
  generic arguments retain E0531. The symbolic generic-call bound bypass is
  removed globally; unproved nominal bounds now receive E0535 in source bodies.
  The final macOS ARM64 batch passed 273/273 selected checks, no ignored tests:
  nine body/caller, 22 declaration, nine kernel and 233 affected generic/
  inheritance/contract/graph/namespace/IDE/metadata/property-hook checks.
  Strict all-target LLVM Clippy passed. The batch took 364.11 seconds, peak
  sampled process-group RSS 1,164,804,096 bytes. An initial internal-visibility
  compile error was corrected before the final batch. Six diagnostic-catalogue
  unit checks also passed (4.26 seconds; 94,715,904-byte peak sampled RSS).
  The ordinary inherited
  generic-call regression again passes interpreter/PHP/Cranelift/LLVM execution;
  this does not execute conditional members. Native MIR/runtime ABI and public
  metadata JSON schemas are unchanged. All conditional executable facts remain
  empty and E0776/lowering retain the execution boundary. Full effect/return-
  loan refinement, ownership integration, symbolic class/trait instantiation
  bounds, trait predicate composition and active member/witness selection remain
  pending. Tooling prepared 13 source-only protocol scenarios at
  `dev-de80dd0bcd42a77577ca1014fd284924d3254875497a9659b1a8b8e3477f6dbb`;
  the compiler owner inspected the cases and the revised contextual `where` hover.
  That hover now says conditional execution remains pending, replacing the
  stale shared-proof claim. Filtering/navigation and prior matrices are
  preserved. Tooling builds/behavioral tests, pins, installed refresh, Git writes
  and publishing remain held. Fresh portable
  and integration gates remain required. Overall Stage 36a stays estimated at
  61% under unchanged scope/weights.
  Invalidated elsewhere: guarded-body/member query assumptions, generic-call
  source rejection and E0779 diagnostics; Tooling protocol coverage/hover;
  installed compiler/LSP; website versioned guide, API reference, diagnostic
  pages and playground. Benchmark and versioned metadata integration remain.

- Generic method-identity checkpoint:
  `dev-04ec62081a6090ad56a6f120e957315aca86bbca900150fedd88560bd478b897`.
  Shared binding uses the original callable template with separate owner and
  method arguments for contract parameters/results/effects, inherited calls
  and member inventories. Repeated generic-parent substitutions preserve those
  identities. The earlier acceptance of mismatched generic interface and
  override contracts is removed; E0755/E0729 reject them. A CLI probe changed
  from success to exactly those two findings. Positive counterparts, swapped/
  collapsed parameters, nullable return mismatches and nested member surfaces
  have durable coverage. The inherited-call case produces `owner7`, empty
  stderr and status 0 on the interpreter, PHP 8.5.12, Cranelift and LLVM release
  on macOS ARM64. The final batch passed 264/264 selected checks, no ignored
  tests: 84 generic/contract/IDE, 31 condition/kernel and 149 affected
  inheritance/graph/namespace/metadata/property-hook cases. Strict all-target
  LLVM Clippy passed. The batch took 390.03 seconds, peak sampled process-group
  RSS 1,092,599,808 bytes. Earlier new-test setup/assertion errors were corrected;
  one owned batch was stopped to update the remaining member-inventory path.
  Native MIR/runtime ABI and public metadata JSON schemas are unchanged.
  Tooling prepared nine source-only diagnostic/call cases at
  `dev-70a96b4407fefb446bbeae605b256b192825e29c0f97bf75d02ad6b5868c9d80`;
  the compiler owner inspected them. Its existing conditional regressions and
  production behavior are preserved. Tooling builds/behavioral tests, pins,
  installed refresh, Git writes and publishing remain held. E0776 and lowering
  retain the conditional-execution boundary. Guarded bodies, trait predicates,
  active member/witness selection, full portable and integration gates remain
  pending. Overall Stage 36a stays estimated at 61% under unchanged scope/weights.
  Invalidated elsewhere: generic contract rejection, call inference and compiler
  IDE signatures; Tooling diagnostic coverage and installed compiler/LSP;
  website versioned guide, API reference and playground. Benchmark and versioned
  metadata integration remain required before delivery.

- Availability-implication checkpoint:
  `dev-d9124ccd5a4d92650a3a7c46ce9ac838722e1012e3498d185d60e446e01e1943`.
  Conformance premises and required method bounds must imply implementation
  guards; inherited guards must imply override guards under child bounds.
  Unconditional implementations widen availability. Original owner substitution
  and canonical method-parameter identities preserve inherited generic-name
  collisions. Computed getter/setter callables share the property's predicate,
  including borrowed getters. E0755/E0729 retain exact clause primaries and
  related contracts; one property's conformance cause is coalesced. No active
  callable/witness facts escape E0776 or lowering. The final macOS batch passed
  260/260 selected checks, no ignored tests: 22 grammar/declaration/availability,
  nine kernel and 229 affected generic/inheritance/contract/graph/namespace/IDE/
  metadata/property-hook cases. Strict all-target LLVM Clippy passed. The batch
  took 350.89 seconds, peak sampled process-group RSS 1,255,522,304 bytes.
  Native MIR/runtime ABI and public metadata JSON schemas are unchanged.
  Tooling prepared a source-only 16-case protocol regression at
  `dev-be54e033758a9feaf8f4e947015a6de1baedd584a89e2156e2ba2478ccd18d76`;
  the compiler owner inspected it. No production filtering/navigation changed.
  Tooling builds, behavioral tests, pins and installed refresh remain held.
  Guarded bodies, trait composition, specialized callable-signature identity,
  active member/witness selection and backend execution remain pending. Fresh
  Linux/Windows evidence and required integration remain outstanding. Overall
  Stage 36a stays estimated at 61% under the unchanged scope and weights.
  Invalidated elsewhere: shared contract availability and compiler IDE clause/
  accessor facts; Tooling diagnostic regressions; installed compiler/LSP;
  website versioned guide, API reference and playground. Versioned metadata
  review and Benchmark integration remain required before delivery.

- Nominal proof-kernel checkpoint:
  `dev-08606a8936f075d68bca6f6de26251cfcb5de97a6ee126e2cf9c3d764edcecba`.
  Typed, query-local proofs freeze caller bounds, substitute declaration edges
  before proving conditions and distinguish exact specializations. Only
  successful proofs are memoized. Cycles retain typed source chains and cannot
  establish their premise; independent evidence can still succeed. Nested
  adapters, inherited generic edges, aliases, multiple subjects and primitive/
  nullable boundaries have focused coverage. A public-entry regression checks
  true/false conditional class bounds in both source orders. A primitive
  interface-return admission regression found during integration was corrected
  through the shared interface-value restriction; E0755 remains enforced.
  One owned verification group was stopped before integration tests to make
  that correction. The final macOS batch passed 145/145 selected checks, no
  ignored tests: nine kernel, 16 grammar/declaration, 20 generic and 100 affected
  contract/graph/namespace/IDE/metadata cases. Strict all-target LLVM Clippy
  passed. The batch took 262.97 seconds, peak sampled process-group RSS
  1,120,714,752 bytes. One transient rmeta scan race recovered through the
  existing complete-root rescan; no partial/stale/zero measurement was used.
  Native MIR/runtime ABI and public metadata JSON schemas are unchanged.
  E0776 and lowering retain the execution fence. Guarded bodies, implication
  checks, active member/witness selection, composition/coherence and backend
  execution remain pending. Tooling prepared matching source-only E0535/E0755
  protocol regressions at
  `dev-cca4c1a50cedc72ce9264dda73c3dd78b8e7f802b747c32e775fc85ab5fed27a`;
  the compiler owner inspected them; builds and behavioral tests remain held.
  No fresh Linux/Windows, installed refresh or publication is claimed.
  Overall Stage 36a remains estimated at 61% under the existing scope/weights.
  Invalidated elsewhere: shared nominal-query/caller-scope assumptions, compiler
  IDE diagnostics and Tooling protocol regressions; installed compiler/LSP;
  website versioned guide, API reference and playground. Full execution,
  versioned metadata review and Benchmark integration remain required.

- Conditional declaration checkpoint:
  `dev-2e5c417adf2c52ebc0cedb3f1aef1f72e36bda027c5a944b18ac6237c93270e5`.
  The shared contract checker records lexical parameter origins and canonical
  interface arguments, including class/trait `self`, without publishing active
  members or witnesses. E0778 identifies invalid clause subjects. Existing
  declaration checks cover malformed interfaces/arguments, duplicate members
  and method-parameter shadowing; ordinary class conformance shares the owner
  context propagation. Fourteen focused grammar/declaration cases and 100
  affected contract/graph/namespace/IDE/metadata checks passed, 114/114 with no
  ignored tests. Strict all-target LLVM Clippy and diagnostic style passed.
  The final batch took 214.28 seconds with peak sampled process-group RSS
  1,128,366,080 bytes. CLI JSON retains exactly eight source-spanned E0776
  findings for the shared fixture. The first run exposed an incorrect new test
  expectation for an existing duplicate-member diagnostic; the expectation
  now names E0481, and the original rejection remains intact. Owner-context
  coverage was extended before the final batch. E0776 and lowering retain the
  pending-execution fence. Public metadata JSON schemas and native MIR/runtime
  ABI files are unchanged. Tooling prepared source-only subject navigation
  and E0778 regressions at
  `dev-665689a33b5481851df37a893869261f7c80247feae5f6ef620356d71d73dee6`;
  the compiler owner inspected the source. Navigation is same-source; complete
  generic references/rename remain unavailable. Tooling builds, behavioral
  verification, pins and installed refresh remain held. Fresh Linux/Windows and executable conditional capability
  evidence remain outstanding. Overall Stage 36a stays estimated at 61% under
  the existing scope and weights.
  Invalidated elsewhere: compiler IDE condition/parameter-origin facts and
  Tooling diagnostics/navigation; installed compiler/LSP artifacts; website
  versioned guide, API reference and playground. Shared proof/member/witness
  execution, versioned metadata review and Benchmark integration remain required.

- Conditional grammar checkpoint:
  `dev-3673bd48602f5cca54fde1852911459c3e53ee0707e4a7a0dba8433f3576aad7`.
  Explicit conformance conditions, member/property availability, exact spans,
  source transforms, namespace/alias facts and incremental signatures share
  the authored clause model. E0776 reports pending Stage 36a execution; E0777
  diagnoses forbidden conditional storage/lifecycle/requirement declarations.
  Conditional programs expose no unconditional semantic callable/witness plan
  and cannot bypass the fence through lowering. The shared generic parser now
  leaves enclosing-list commas after nested `>>` closes intact. Nine new grammar
  cases and 164 affected parser/contract/graph/namespace/debug/metadata/IDE/cache
  regressions passed; both existing Cranelift/LLVM interface structure checks
  passed, for 175/175 selected checks with no ignored tests. Strict all-target
  LLVM Clippy and diagnostic style passed. The affected batch took 138.22
  seconds, peak sampled process-group RSS 1,062,567,936 bytes; structural checks
  took 8.01 seconds, peak 492,437,504 bytes. CLI JSON reports exactly eight
  source-spanned E0776 findings and no parser errors for the shared fixture.
  One initial build stopped safely because an allocated-size scan raced with
  deletion of a linker temporary. A fresh complete scan succeeded below the
  aggregate ceiling; the orchestration guard now permits one complete rescan
  while retaining hard failure for unavailable volumes or repeated scan errors.
  No partial/stale/zero measurement is accepted; no cache was cleaned.
  Tooling prepared AST consumers, signatures, contextual editor highlighting,
  hover, unavailable-member fallback filtering and a shared fixture/regressions.
  Its builds, behavioral verification, pins and installed refresh remain held;
  this is source preparation, not verified integration. No new metadata schema
  keys or native runtime/MIR changes were made. Shared proof/member/witness
  execution and fresh portable coverage remain outstanding. Overall Stage 36a
  stays estimated at 61% under the existing scope/weights.
  Invalidated elsewhere: the compiler AST conformance/member model and typed
  consumers, incremental condition fingerprints, Tooling build compatibility,
  and installed compiler/LSP artifacts. Website versioned guide, API reference
  and playground need the accepted clauses; Benchmark and execution integration
  remain required before delivery.

- Recursive private PHP transport now derives nested components from checked
  enum payload types. Constructors, arguments/results, defaults, match bindings,
  boxed inspection and typed storage share that layout. Source-level returned
  parameter homes no longer persist into specializations that do not lend their
  parameter; capture, ownership and actual returned-loan homes remain supported.
  Native MIR and runtime ABI files are unchanged. The 18-route nested fixture
  preserves output across interpreter/PHP/Cranelift/LLVM in the nine-fixture
  matrix. Its complete instrumented constructor maps stay fixed at 0, 1 and
  1,000 iterations, with no retained measured objects or function/arrow/new
  loop tokens. Direct baseline Leaf/Envelope counts of 2,000/1,000 and the
  generic route's additional 1,000 typed cells are eliminated. Fixed setup
  objects remain. Boxed snapshots, null versus Empty, named evaluation order,
  failed replacement and generated parameter-name collisions are covered.
  Tested source: `dev-0d7604337afdaef7b650f0423b0903fc20431f19f32cc6695b77f85ca7c7c726`.
  Seven selected macOS suites completed at 409/412: PHP library 64/65,
  integration 141/142, hooks 88/88, initializers 18/18, integers 18/18,
  returned closures 77/78 and nullable defaults 3/3. V-A/M-C/R-S retain their
  original native diagnostics. No tests are ignored. The first bounded batch
  stopped at fifteen minutes after 17 returned-closure passes; the exact 61
  unfinished names were inventoried and resumed with the same binaries, without
  repeating those passes. The resumed job finished in 303.75 seconds with
  556,154,880-byte peak sampled process-group RSS. Strict all-target LLVM Clippy
  passed. Build-free PHP 8.4.21 checks passed 23/23. Both PHP 8.4.21 and 8.5.12
  passed 132 same-input probe rows with FFI disabled, fixed constructor counts
  and live0. The original nested-default probe now constructs no measured
  objects, eliminating its remaining 1,000 inner Packets. This measures explicit
  constructors, not total host array/string/clone/implicit-constructor costs.
  Final checks took 120.43 seconds, peak sampled RSS 780,894,208 bytes. Fresh
  Linux/Windows, conditional-capability implementation and required integration
  remain outstanding; no installed refresh or publication occurred.
  Invalidated elsewhere: private generated PHP component/writer/argument ABI,
  constructor baselines and installed artifacts require this compiler identity;
  public Doria semantics and native layout are unchanged. Conditional-clause
  Tooling, SPEC, website versioned guide/API reference/playground and Benchmark
  integration remain the separate accepted Decision 0139 work above.

- PHP outcome setters now use the shared argument and statement-region plans,
  including concrete, generic, virtual and interface calls. Pending ownership
  lasts through receiver/RHS evaluation and transfers at invocation. Checked
  trivial-Copy enum facts avoid move-ownership cells for `take` parameters;
  managed-Copy and Move enums retain their ownership homes. Field-backed explicit
  defaults initialize their typed cells during construction, including nullable
  generic defaults, without calling user setters. No source defaults are added.
  Raw stored-property reads, including null-safe reads, now share this storage
  plan. Receiver evaluation uses the shared statement lifetime, absent receivers
  clear the result channel, and field snapshots precede nested-region cleanup.
  The property fixture now covers fourteen routes, each with fixed instrumented
  constructor counts at zero, one and 1,000 updates, zero function/arrow-function/
  `new` loop tokens, and no retained instrumented objects. Boxed inspection,
  failing setter replacement, temporary receiver cleanup, checked receiver
  failure and lazy nullable fallback stay covered. The same-input nullable-read
  probe previously constructed 1,000 outcome objects in 1,000 reads; it now
  constructs none, with unchanged setup counts and no retained measured objects.
  Defaulted trivial-Copy enum arguments now use the same private field transport
  as required parameters. Callers complete omitted positions from checked folded
  defaults through the shared argument binder, including named gaps, nullable
  values and inherited method defaults. Private field-argument helpers no longer
  accept omitted parameters; compiler-generated callers supply their full arity.
  Boxed entry points retain omission support, including effective defaults on
  overrides whose source parameters have no repeated default expression.
  The argument fixture covers twenty-four direct, nested-call, generic, static,
  virtual, interface, callback, nullable and defaulted routes. All have fixed
  instrumented constructor counts at zero, one and 1,000 iterations, no measured
  retained objects and zero function/arrow-function/`new` loop tokens. The saved
  simple-default probe changes from one Packet construction per iteration to
  zero on PHP 8.4 and 8.5. Its nested-payload default changes from two measured
  constructions to one inner Packet per iteration. PHP's shared Shape still
  counts top-level payload slots; recursive payload transport remains required.
  These probes do not measure all host arrays, strings, clones, implicit
  constructors or FFI allocations. The full allocation gate remains open.
  A compiler sample identified repeated recursive validation of concatenation
  operands. Display-type checks now reuse the existing child traversal; a
  deterministic 32-term guard requires exactly 63 expression visits. This does
  not establish deeper-source stack coverage.
  At `dev-ee8e6c237ac89d79a4d38d98fe40c531b6ff02aeb19bbfad6906736dcbe1ca5d`,
  macOS ARM64/Rust 1.98.1/PHP 8.5.12/LLVM 18.1.8 passes 63/64 PHP-library,
  141/142 PHP integration, 88/88 hooks, 18/18 initializers and 18/18 integer
  cases, none ignored. A redundant unwrap found by strict Clippy is replaced
  with equivalent pattern matching at
  `dev-31803f7ee6dd6ccd15f67b90cd0e16f00776fde93ecded17bcde56846cc9ca94`.
  The followup passes strict all-target LLVM Clippy, 63/64 PHP-library cases,
  the eight-fixture interpreter/PHP/Cranelift/LLVM matrix, 5/5 PHP default cases,
  77/78 returned-closure cases and 3/3 nullable-default cases, none ignored.
  Unchanged broad-suite evidence remains scoped to `ee8e6c23`; it is not
  presented as a new full-suite run after the lint-only change. Enabled failures
  remain V-A, M-C and R-S with their original native diagnostics; their separate
  corrective approvals are pending. No native lowering, validation, MIR model,
  backend or runtime ABI changes are made by this PHP correction.
  Twenty-two build-free focused cases pass with PHP 8.4.21 selected and matching
  native runtimes, including the argument/property constructor probes with FFI
  disabled, backend matrix, defaults and parameter-home guard. The standalone
  fourteen-route property probe retains its earlier `fe59f5ed` evidence; it is
  not regenerated here. The current saved default probe and exact main oracle
  pass with FFI disabled on PHP 8.4.21 and 8.5.12. The broad batch takes 455.12
  seconds with sampled process-group peak RSS 1,147,207,680 bytes; lint and
  followup checks take 715.72 seconds and 1,155,252,224 bytes. PHP 8.4 checks take
  92.99 seconds and 219,529,216 bytes. Current Linux PHP and Windows coverage
  remain unverified. Docker is not started; a bounded PHP-image download awaits
  approval. The Windows packet remains the earlier cleanup checkpoint and needs
  transfer to the laptop before a run can be requested.
  Stage 36a is estimated at 61% with unchanged full scope and weights:
  implementation 80% of 50%, verification 53% of 30%, integration 25% of 20%.
  Public appendix, remaining allocation positions and host allocations,
  adapters/processes, platform/performance and integration gates remain open.
  Host refresh, pins, staging, commits and publishing remain held. Source
  outside this note is stable through final verification and documentation.
  Invalidated elsewhere: generated PHP field/argument/constructor shape and
  allocation baselines, installed artifacts, and website versioned guide/API-reference
  ownership and cleanup examples need revalidation. Public Doria names and
  editor facts are unchanged; Tooling's compiler pin remains held.

- PHP owned instance outcome properties now reuse typed cells across ordinary,
  promoted, generic, nullable, internal and shadowed fields. Storage and
  initializer ownership share most-derived lookup; parent-private and child
  fields stay separate. Shared scalar operand staging preserves lazy evaluation,
  operation diagnostics, Copy snapshots and checked-failure cleanup. Generated
  properties now use virtual boxed-snapshot hooks; direct host PHP by-reference
  access to these formerly backed properties is no longer supported. Doria
  borrows use stable compiler-owned cells.
  Seven property routes have fixed instrumented constructor counts at zero,
  one and 1,000 updates, zero function/arrow-function/`new` loop tokens and no
  retained instrumented objects. Boxed get/set inspection preserves Copy values.
  At `dev-b69604e96c4183fa9bc6e6a930b4300aab6ce3116eb25fe046346479099e6ed6`,
  macOS ARM64/PHP 8.5.12 passes 21/22 result-emitter cases, with known V-A only;
  the eight-fixture interpreter/PHP/Cranelift/LLVM matrix passes. The PHP suite
  passes 137/142 before four stale nested-closure shape assertions are updated.
  That test-only followup, including executed mixed type-test coverage, passes
  all five precedence-filter cases at
  `dev-cdd2cb6b8b5d8f38b1b9efc735806aa21a956e6dfb515f4cdc9fab7032b74ac3`.
  Unchanged passing PHP-suite evidence is reused; the full 142 are not rerun.
  At the followup, hooks pass 88/88, initializers 18/18, returned closures
  77/78 and integer execution 18/18, none ignored. M-C, R-S and V-A remain
  enabled and await their separate corrective approvals. Strict all-target LLVM
  Clippy passes. PHP 8.4.21 passes ten focused checks and the seven-route portable
  probe; unchanged PHP-only library components reuse their `b69604e9` executable,
  while native integration uses the matching `cdd2cb6b` runtime.
  The broad followup takes 631.53 seconds with sampled process-group peak RSS
  502,415,360 bytes; Clippy takes 82.54 seconds and 980,353,024 bytes. The original
  shadowing repro returns 98 on PHP 8.4 and 8.5, matching the interpreter.
  A Linux PHP-only attempt executes no tests: the cached Rust image has no PHP,
  and no PHP 8.4+ image is cached. Its owned container is removed and Docker is
  stopped; a bounded image download is requested. Earlier Linux evidence stays
  scoped to its recorded checkpoint. Current Linux PHP and native Windows
  coverage remain unverified. This followup changes no native MIR model or
  runtime ABI. The full allocation gate remains open, including static and
  accessor paths, other expression positions and unmeasured host allocations.
  Stage 36a is estimated at 59% with unchanged full scope and weights:
  implementation 78% of 50%, verification 51% of 30%, integration 25% of 20%.
  Public appendix, adapters/processes, remaining platform/performance and
  integration gates stay open. Host refresh, pins, commits and publishing remain
  held. Source is stable through verification; this note follows testing.
  Invalidated elsewhere: generated PHP shape/reference access and allocation
  baselines, installed artifacts, and website versioned guide/API-reference
  ownership and cleanup examples need revalidation. Public Doria names and
  editor facts are unchanged; Tooling's compiler pin remains held.

- PHP trivial-Copy outcome storage now reuses scalar local, private-parameter
  and captured homes. Actual capture/borrow consumers still receive stable
  cells; generated field-backed cells preserve identity through writable
  callbacks and expose boxed inspection through PHP 8.4 property hooks. Owned
  Copy captures get independent homes, and failed replacement leaves the prior
  value intact. Local, parameter, captured, writable-callback and owned-capture
  loops have fixed constructor counts at zero, one and 1,000 updates. A fixed
  boxed inspection after a loop was counted separately. Their loop conditions
  and bodies contain zero function, arrow-function and `new` tokens. Physical
  property updates at this earlier checkpoint constructed one outcome object per
  update and contained one expression closure; the full allocation gate remained
  open.
  Andrew approved C-A's shared native cleanup correction. Common indirect-call
  and throw-assertion lowering retain pending owned callees until invocation;
  MIR validation rejects missing cleanup, inconsistent acquisition ownership
  and non-consuming once calls. The original regression and seven-case matrix
  preserve reverse operand cleanup before catch and exactly-once handoff across
  interpreter/PHP/Cranelift/LLVM on macOS. All PHP callback modes now share the
  staged argument plan; nested raw calls preserve returned borrow cells before
  the outer projection. An introduced double projection was corrected with the
  original returned-borrow oracle intact.
  At `dev-91268d0893dd329d2373a4074712a59c7363dda1f482c8eca7615cdafee6fa1f`,
  macOS ARM64/Rust 1.98.1/PHP 8.5.12/LLVM 18.1.8 passes 141/142 PHP,
  123/123 MIR-validation, 77/78 returned-closure, 88/88 property-hook and 20/21
  result-emitter cases, none ignored. Enabled failures remain V-A's native
  result merge, M-C's managed-Copy replacement and R-S's scalar retained-source
  validation; their corrective approvals remain pending. Seven selected
  assertion tests pass at this checkpoint. The eighth exposed an existing
  harness conflict between private panic transport and human stderr. Its
  test-only followup separately verifies both paths, retaining the human oracle,
  at `dev-394297e0730e12d4cb34e2af78f7551419d1886b9c99a681647043c08505f481`.
  Strict all-target LLVM Clippy passes there. PHP 8.4.21 also passes the storage,
  returned-borrow and two cleanup regressions with matching native runtimes.
  At that followup, Linux ARM64/Debian 13.7/Rust 1.98.1/PHP 8.4.27 passes 136/136
  focused validation, allocation, fixture, assertion and cleanup checks using
  interpreter/PHP/Cranelift, none ignored. Linux LLVM and native Windows remain
  unverified; a focused Windows packet is prepared and its run requested.
  The macOS broad followup takes 712.01 seconds with sampled process-group peak
  RSS 1,162,821,632 bytes. Linux takes 345.61 seconds with one CPU, 2.5 GiB,
  read-only source/dependencies and disconnected networking; cgroup peak is
  2,684,354,560 bytes with 3,017 memory-limit events and zero OOM counters.
  MIR model and runtime ABI are unchanged. The owned container and duplicate
  probes are removed; Docker backend/VM and builds are stopped. Healthy caches,
  original containers, cached image, Windows packets and unfinished work are
  preserved. Aggregate allocated caches are 23,976,275,968 bytes below 25 GB.
  Source identity is stable through verification; this note follows testing.
  Stage 36a is estimated at 58% with unchanged full scope and weights:
  implementation 76% of 50%, verification 50% of 30%, integration 25% of 20%.
  Public appendix, remaining allocation, platform and integration gates stay
  open. Host refresh, pins, staging, commits and publishing remain held.
  Invalidated elsewhere: PHP emission/allocation baselines, native callback
  cleanup evidence and installed artifacts; website versioned guide and API
  reference ownership/cleanup examples need checkpoint revalidation. Public
  names and editor facts are unchanged; no hover/highlighting additions are
  required by these private transport changes. Tooling's compiler pin is held.

- PHP callback results now share scalar return storage with named producers.
  Carriers retain boxed inspection entries and expose field-result and combined
  argument/result entries over one helper body. Once-only carriers snapshot
  results before environment cleanup and publish afterward; checked failures
  clear the channel after cleanup. Shared statement regions also schedule
  result-only call operands, preserving evaluation order and pending ownership.
  Field producers enter their own region unless already rooted; numeric updates
  snapshot the place and previous value before staging every numeric RHS. This
  removes coalescing/compound-update IIFEs through shared emission. Both loop
  probes now inspect real PHP tokens in the condition and body: zero function,
  arrow-function and `new` tokens. Positive/negative controls cover each form,
  and the same guard detects two closures in the preserved pre-correction output.
  The six-route callback-result constructor probe measures zero outcome objects
  at zero, one and 1,000 iterations across direct, generic, nested, combined,
  nullable and stored-function calls. Boxed controls construct/release 2,000
  objects; a boxed once-only result survives a destructor producing another
  enum. The existing twelve-route argument probe and 1,000-object boxed control
  still pass. The full allocation gate still includes helper allocations,
  arrays, strings, implicit constructors, FFI, default/stored/captured values and
  other expression positions. Exact callback, ordering and cleanup
  fixtures agree across interpreter/PHP/Cranelift/LLVM on macOS, with additional
  generic inline-Copy callback coverage passing with FFI disabled.
  At `dev-3a16615896d99a8cd243131a9bf58a7239e712236662df8543de8bb6bd7d7ff1`,
  macOS ARM64/Rust 1.98.1/PHP 8.5.12/LLVM 18.1.8 passes 329/330 library,
  137/141 PHP and 18/18 integer-execution cases. The two extra PHP failures assert
  the prior inline float shape. The assertion-only followup
  `dev-6bdd90143466ca83a98210fc7ecdfb276a18196cbe04c791593a8828990d986b`
  passes both corrected read/evaluate/commit assertions with original IEEE and
  conversion oracles intact, plus 77/78 returned-closure and 88/88 property-hook
  cases. Reusing exact unchanged production evidence gives 139/141 PHP and
  651/655 total checks, none ignored. Strict all-target LLVM Clippy passes.
  Enabled failures remain
  V-A's native result merge, M-C's managed-Copy replacement, R-S's scalar
  retained-source validation and C-A's pending once-callee cleanup. At that
  checkpoint, all four corrective permissions remained pending; native
  MIR/model/lowering/validation and runtime ABI files were unchanged.
  The macOS broad/float/ownership batches
  take 472.51/106.83/477.33 seconds with sampled process-group RSS peaks
  1,397,473,280/1,467,449,344/522,731,520 bytes; lint takes 60.40 seconds and
  1,119,092,736 bytes. At the followup checkpoint, focused Linux ARM64/Debian
  13.7/Rust 1.98.1/PHP 8.4.27 passes 19/20 result-emitter units, 4/5 selected PHP
  and 18/18 integer-execution cases: 41/43, none ignored, only V-A/C-A failing.
  Both constructor/token probes, float oracles and FFI-disabled generic callbacks
  pass; the expanded fixture agrees
  across interpreter/PHP/Cranelift. Linux LLVM and Windows remain unverified.
  Linux takes 273.45 seconds with one CPU, 2.5 GiB, read-only source/dependencies
  and disconnected test networking. Container cgroup peak is 2,684,354,560 bytes
  with 1,393 memory-limit events and zero OOM counters. The initial toolchain
  probe accidentally installed Rust 1.99.1 inside the disposable container;
  verification explicitly uses installed 1.98.1 with automatic installation
  disabled. Subsequent container creation pins 1.98.1 and disables auto-install.
  Both owned containers, including that accidental install, are removed;
  Docker backend/VM and builds are stopped. Owned probes are removed after
  preserving compact evidence; healthy caches and the older Windows packet
  are preserved. Allocated caches total 23,619,526,656 bytes below
  25 decimal GB. Source identity is stable through execution; this note follows
  verification. Stage 36a is estimated at 57% with unchanged full scope/weights:
  implementation advances from 72% to 74% of its 50% weight; verification stays
  50% of 30%, integration 25% of 20%. Host installation, pins, staging, commits
  and publishing stay held. Invalidated elsewhere: PHP emission/allocation
  baselines and installed artifact evidence; website guide/API-reference
  ownership/cleanup examples still need checkpoint revalidation. Public names
  and editor facts are unchanged; Tooling has no private-helper dependency.
  Full platform, integration, allocation and delivery gates remain.

- Eligible PHP field-backed local arguments now use scalar parameter slots
  across source free/generic/static/virtual/erased calls and typed callbacks.
  Callback carriers preserve boxed `__invoke` inspection through the same
  scalar helper body; checked cell parameters keep their existing borrow ABI,
  and private captured parameters materialize their own homes. The seven-route
  constructor probe observes zero outcome objects for six routes at zero, one
  and 1,000 iterations. Nested producers still construct zero, one and 1,000;
  that full regression remains enabled and failing. The boxed control still
  constructs and releases exactly 1,000 objects. These counts exclude arrays,
  strings, closures, FFI wrappers and implicit constructors. Stored/captured
  values, callback results and shared nested-operand scheduling remain within
  the open allocation gate. The new callback fixture covers nullable and boxed
  inputs, writable replacement, `take`, generic dispatch, nested captures and
  once-only owned cleanup with exact interpreter/PHP/Cranelift/LLVM agreement.
  At `dev-248fa1c3b33e231e6c875c9d812e88f22568d5426e46f0a4eb4e514524f00e24`,
  macOS ARM64/Rust 1.98.1/PHP 8.5.12/LLVM 18.1.8 passes 325/327 library and
  139/140 PHP backend cases, none ignored, plus strict all-target LLVM Clippy.
  The 455.14-second suite and 76.79-second lint runs peak at sampled process-group
  RSS 1,113,096,192 and 1,054,998,528 bytes respectively. Before the fixture-only
  expansion, three owned-callback cleanup regressions pass at `dev-e1c9945d`.
  Enabled failures remain nested boxing, V-A's native result merge and M-C's
  native managed-Copy replacement; V-A/M-C/R-S approvals remain pending.
  Native MIR/model/lowering/validation and runtime ABI files are unchanged.
  Fresh Linux/Windows execution is pending. Owned preflight outputs are removed;
  build/test processes are stopped and Docker remains off. Aggregate allocated
  caches total 23,618,490,368 bytes below 25 decimal GB; healthy caches and the
  older Windows packet are preserved. Source identity is stable through testing;
  this note follows verification. Stage 36a stays estimated at 55% with unchanged
  full scope/weighting. Installation, pins, staging, commits and publishing stay
  held. Invalidated elsewhere: PHP emission/allocation baselines and installed
  artifact evidence; public names, website guide/API contracts and editor facts
  are unchanged. Tooling has no direct dependency on the private PHP helpers.

- Named Copy parameters now keep private PHP storage across free, generic,
  virtual, erased-interface, static, constructor, nullable and default calls.
  Removed unintended caller mutation from parameter replacement and local
  captures; structural writable callbacks retain their checked borrow ABI.
  Promoted properties read the checked binding place, and private managed Copy
  cells acquire and release their own claims. The interpreter shares native
  parameter-home policy and preserves returned closures' retained-source homes.
  PHP specialization and interface plans preserve the full checked return-borrow
  contract, including a checked absence; owned carriers retaining source loans
  remain distinct from borrowed return values. The original scalar-capture test
  and its E0658 rejection remain intact, with separate generic and indirect
  factory regressions. Existing caller-mutating expectations are corrected from
  78 to 69 and from `true:false:false:false:true` to
  `true:false:true:false:true`, with interpreter and native comparisons.
  At `dev-c6ad1eb4bf5843dff7d76e092f214352dce3751aaff0189088534609f6c2b580`,
  macOS ARM64/Rust 1.98.1/PHP 8.5.12/LLVM 18.1.8 passes 325/326 library,
  139/140 PHP, 45/45 checked-error and 88/88 property-hook cases: 597/599.
  Strict compiler all-target LLVM-enabled Clippy passes. Removing the new
  fixture's obsolete explicit routine I/O clause is the only source change to
  `dev-dba3a4a621d9161d0e88ce59b48ea1b6593427f8aefa0e83598148e694050284`;
  its six focused macOS parameter/capture cases pass four and fail two.
  The original and generic capture cases pass interpreter, PHP, Cranelift and
  LLVM execution. At that same checkpoint Linux ARM64/Debian 13.7/Rust 1.98.1/
  PHP 8.4.27 passes 320/321 library, 139/140 PHP, 45/45 checked-error, 88/88
  property-hook and 77/78 returned-closure cases: 669/672, none ignored.
  Linux native comparisons use Cranelift; fresh Linux LLVM and Windows execution
  remain unverified. Enabled failures are V-A's native result-exit merge, M-C's
  native CancellationToken replacement and R-S's indirect scalar retained-source
  validation. PHP exact cleanup and managed-Copy outputs pass before the first
  two native failures. R-S rejects an accepted int source as not a Move value.
  Those native corrective scopes await approval; MIR model/lowering/validation
  and runtime ABI 15 are unchanged by this step. The broad/focused macOS batches
  take 456.01/132.16 seconds; Linux batches take 294.13/336.40/391.35 seconds.
  Docker reuses the cached image/target with one CPU, 2.5 GiB, read-only source
  and dependencies and no test networking. Cgroup peak reaches 2,684,354,560
  bytes with 1,374 memory-limit events and zero OOM counters. The owned container
  and completed temporary probes are removed; Docker backend and VM are stopped.
  Aggregate caches total 23,616,503,808 allocated bytes below 25 decimal GB;
  healthy caches and the older Windows packet are preserved. Formatting, source,
  documentation-authority and whitespace checks pass. Source identity is stable
  through verification; this note follows it. Stage 36a remains estimated at 55%
  with unchanged scope/weighting and open allocation/performance, public-surface,
  platform, integration and delivery gates. Additional Tooling verification,
  installed-tool refresh, pin changes, staging, commits and publishing stay held.
  Invalidated elsewhere: PHP/signature and debug-interpreter baselines; website
  guide/API ownership examples need revalidation. Public names and editor facts
  are unchanged; installed compiler/LSP evidence remains stale until refresh.

- PHP numeric compound assignments now emit structured `when` and eligible
  Copy-enum `match` operands directly in statement position. The shared numeric
  place emitter captures the storage home and old value before RHS evaluation,
  preserves getter/RHS/setter order, and retains operands and receivers through
  commit or checked failure. Selected match arms reuse statement operation
  regions; parentheses preserve this path without discarding outer conversion
  metadata. Regression coverage includes local, float64, hooked, static, typed
  array, List and Bytes updates, RHS mutation, cleanup and a 1,000-iteration
  grouped-match loop with no emitted closure. The native file-read probe now
  compares all instrumented constructor counts at zero, one and 1,000 reads:
  fixed setup counts are identical, ReadOutcome construction stays zero, and
  the read loop contains no closure. Its explicit boxed control still constructs
  and releases 1,000 outcomes. These counts exclude arrays, strings, closures,
  FFI wrappers and implicit constructors; the full PHP allocation gate is open.
  At `dev-e0300098d0244fe4f2a3a5b6373a13ce2a035f009e0113e05e4eede1d83ee8ce`,
  macOS ARM64/PHP 8.5.12 passes 56 of 57 PHP emitter units and all 88 property-hook
  checks. Strict compiler all-target LLVM-enabled Clippy passes. Linux ARM64,
  Debian 13.7, Rust 1.98.1 and PHP 8.4.27 produces the same 56/57 unit result,
  and passes 45 checked-error, 138 PHP backend, 18 integer-execution and 88
  property-hook checks: 289 integration cases, none ignored. Linux native
  comparisons use Cranelift; fresh Linux LLVM and Windows execution are unverified.
  The enabled cleanup regression first verifies exact PHP output
  `12345;1245;15`, then fails shared native MIR validation because a caught-error
  route is followed outside the match and incorrectly required to merge a result.
  The same accepted fixture was rejected at the earlier `dev-66cc61c7` checkpoint.
  V-A permission remains pending for explicit shared `when`/`match` result-exit
  routes; this and the two earlier native `when` regressions remain enabled.
  MIR, lowering, independent validation and runtime ABI 15 are unchanged.
  Before the grouping follow-up, `dev-b36bd399` passed the 201 macOS PHP,
  checked-error and integer cases. That combined batch reached its ten-minute
  limit after 74 passing closure cases; the final two passed separately from
  the same binary. The original combined run is recorded as incomplete.
  Current macOS unit/property/lint runs take 88.84/180.80/61.47 seconds, with
  sampled process-group RSS peaks 1,280,950,272/1,203,077,120/1,264,238,592 bytes.
  Linux unit/integration batches take 85.03/373.41 seconds. Docker reused the
  cached image and owned target, one CPU, 2.5 GiB, read-only source/dependencies,
  and no test networking. Cgroup peak reaches the 2,684,354,560-byte cap with
  1,286 memory-limit events; all out-of-memory counters remain zero. The owned
  container and completed temporary probe are removed; Docker, its VM and test
  processes are verified stopped. Formatting, fixture syntax, documentation
  authority and whitespace checks pass. Source identity is unchanged across
  execution; this note follows verification. Aggregate allocated caches total
  23,483,428,864 bytes, below 25 decimal GB; healthy caches are preserved.
  The Windows packet remains the older `dev-7e0941f7` snapshot and does not
  contain this PHP optimization. Stage 36a stays estimated at 55% with unchanged
  scope/weighting and open public-surface, allocation/performance, platform,
  integration and delivery gates. Tooling's additional verification permission
  remains pending. No install, pin/lock change, staging, commit or publication
  occurred. Invalidated elsewhere: generated PHP numeric/match structure,
  allocation baselines and Windows-packet PHP coverage; public language/API
  contracts and compiler-owned editor facts are unchanged by this step.

- PHP outcome construction now has a test-only dynamic probe. `PhpToken` source
  positions identify emitted constructor bodies; strings, interpolation and
  comments do not become code patterns. WeakMap identity counts one object
  across inherited constructor phases without retaining it. Calibration covers
  direct and inherited construction, failure, global functions, later instances
  and weak lifetimes. At
  `dev-6e931bddce867aaa5bc82a3868e9a8542e359bd1a3cd913ca51eee8107945e49`,
  1,000 field-result calls/copies/matches construct zero Packet outcome objects
  and zero cells; an explicit boxed-call control constructs exactly 1,000
  outcomes and releases them. A separate loop performs 1,000 three-byte native
  file reads into one reusable region, with zero ReadOutcome objects; its
  explicit factory control constructs and releases exactly 1,000 outcomes.
  These are instrumented-constructor counts, not total PHP heap allocation:
  arrays, strings, closures, FFI wrappers and implicit constructors are excluded.
  The full argument/region/nested/managed/outer-completion allocation gate and
  controlled performance evidence remain open. An initial real-file fixture
  used an unapproved method spelling and failed source checking; it now uses
  the existing `getWritableRegion` declaration without changing language rules.
  All 55 PHP emitter units pass on macOS ARM64/PHP 8.5.12, none ignored. Strict
  compiler all-target Clippy, formatting, fixture syntax and whitespace checks
  pass. The unit batch takes 81.70 seconds and peaks at sampled process-group
  RSS 1,285,505,024 bytes; strict lint takes 62.71 seconds and peaks at
  1,165,852,672 bytes. Source identity is unchanged across final verification;
  this note follows it. Only test modules and the new fixture changed; production
  prefixes match the Windows packet, whose checksum is unchanged. Fresh Linux
  and Windows execution of these new probes is unverified. The shared PHP test
  runner remains unchanged, and owned per-test files are removed by their
  fixture owners. Aggregate caches total 23,274,696,704 allocated bytes, below
  25 decimal GB; healthy caches are preserved and Docker was not started.
  Stage 36a remains estimated at 55%; existing design, platform, integration,
  install and publishing gates are unchanged. Invalidated elsewhere: PHP
  allocation verification evidence; public APIs, MIR validation and ABI 15
  are unchanged. No install, pin/lock change, commit or publication occurred.

- Native Windows verification is prepared against
  `dev-7e0941f774bf8a868cc8ccd5e7d4a61bb9d29d6e7292cdc555fd5685792690f6`.
  Windows MSVC compiler all-target checking without LLVM/runtime bundling and
  both standalone and hosted runtime test-target checks pass offline on macOS.
  These are compile-only checks; native Windows execution remains unverified.
  One initial attempt selected the separate named Rust 1.98.1 installation,
  which lacks the Windows standard library. Using the project-configured
  stable installation fixes the selection; its compiler is also Rust 1.98.1
  and its Windows target was already installed. No tool/version change occurred.
  The successful compiler check takes 49.27 seconds with sampled peak
  process-group RSS 951,615,488 bytes; runtime checks take under six seconds each.
  A 4,141,196-byte portable packet preserves all 2,040 source files, the original
  rules, a hash manifest and the scoped Windows handover. Archive extraction,
  every file hash, the source fingerprint, all five workspace packages and the
  final ZIP contents are verified; temporary helpers and duplicate copies are
  removed. Its SHA-256 is
  `99e6f9629d96e0848cf953aab6769dcd7d8e9413cdd9219d638323b5210d2d56`.
  Andrew has been asked to run it with Codex on his configured laptop: one
  offline native Windows pass, one build job, up to 45 minutes, 6 GiB RAM and
  3 GB new artifacts, with no installs or publication. Approval/execution and
  the additional Tooling pass remain pending. All existing V-A, public-surface,
  performance, integration and delivery gates remain open; Stage 36a remains
  estimated at 55%. Aggregate allocated caches total 23,312,637,952 bytes;
  no healthy cache cleanup or Docker start occurred. This note follows the
  exact-source checks and packaging. Invalidated elsewhere: Windows
  verification provenance and handover; production code/API contracts unchanged.

- PHP pending completions now select the compiler-generated scalar field-result
  entry points for both read and write callbacks. Dispatch captures presence,
  tag and byte count in reusable member slots and clears the shared result
  channel on success and failure. This removes mandatory outcome-object boxing
  at this callback boundary; explicit boxed callers retain their existing path.
  Checked argument modes, preparation, native error transport and public
  contracts are unchanged. Regressions cover every argument mode, present zero,
  absent and terminal outcomes, stale fields, exceptions after publication,
  checked entry-point selection and 1,000 resubmissions per standard device.
  At `dev-66cc61c7ba8dce52e3872f13216f26ffdc8a63855c2813fa02b449d910bf6025`,
  macOS passes 52 PHP emitter units, 138 PHP backend, 45 checked-error,
  34 transfer-group and 24 transfer-plan checks: 293 total. Linux ARM64/Debian
  13.7/Rust 1.98.1/PHP 8.4.27 passes 52 emitter units, 45 checked-error,
  35 closure-ownership, five diagnostic, 34 group and 23 plan checks: 194 total.
  None are ignored. The Linux checks also cover the preceding diagnostic fixes.
  Native comparisons cover Cranelift/LLVM on macOS and Cranelift on Linux;
  fresh Linux LLVM and Windows execution remain untested. Strict compiler
  all-target Clippy, formatting, PHP syntax and whitespace checks pass on macOS.
  The source fingerprint is unchanged across execution; this note follows
  verification. Runtime ABI 15 and independent MIR validation are unchanged.
  Docker reused the existing image/cache with one CPU, 2.5 GiB memory,
  read-only source/dependencies and no test networking. The two Linux batches
  completed within their ten-minute bounds, with no out-of-memory event.
  The owned container is removed; Docker and test processes are verified stopped.
  Unrelated resources and healthy reusable caches are preserved. Allocated
  aggregate caches total 23,277,473,792 bytes, below the 25 decimal GB ceiling.
  Full PHP allocation/performance, remaining public surfaces and integration
  gates stay open; Stage 36a remains estimated at 55%. The additional Tooling
  verification permission remains pending. No install, pin/lock change, staging,
  commit or publication occurred. Invalidated elsewhere: generated PHP pending
  completion emission and callback allocation baselines; public language/API,
  native ABI and compiler-owned editor facts are unchanged by this change.

- Compiler diagnostic follow-up now offers the existing review-only capture
  transfer edit on local-borrow return errors. The shared edit builder reads
  checked local ownership rather than declaration-time ownership. Removed invalid
  `take` suggestions for inferred borrowed Move locals; writable captures and
  byte regions still receive no invented ownership-transfer edit. Regression
  cases cover direct/aliased arrow and block closures, preserved comments,
  successful checking after valid edits, and unsafe return/collection storage.
  All three retained-loan conflict consumers now choose the earliest source
  location with stable category/access tie-breaks instead of hash-map order.
  Repeated independent compiler/IDE analyses check closure and iterator origins.
  Both new regressions initially failed at
  `dev-757e191e292dbf00c372539acb90be1fc45798687a318f266948ddcda0107f8a`.
  A subsequent safety case exposed the stale edit-builder ownership lookup;
  the shared correction preserves the negative assertion rather than accepting
  the invalid suggestion. At
  `dev-7ae24481d7ea4326c0b5c2787074bb8ad3afc4dee5c86cb691ddc8f63715dc86`,
  all 35 closure-ownership, five diagnostic-presentation, 52 interface-contract,
  27 byte-region and 70 readiness checks pass on macOS: 189 total, none ignored.
  Strict compiler all-target Clippy, formatting, documentation authority and
  whitespace checks pass. Source identity is unchanged across verification;
  this note is a subsequent documentation edit. Runtime ABI 15 and independent
  MIR validation are unchanged. Subsequent Linux coverage for these diagnostic
  fixes is recorded above; Windows execution remains untested. Compiler/Docker
  processes were verified stopped at this checkpoint.
  Aggregate allocated caches total 23,131,058,176 bytes, below 25 decimal GB.
  Tooling's owner corrected the shared test baseline to use compiler-prepared
  findings and removed the blanket E0763 expectation: nullable closures expect
  E0654 while iterator/source carriers retain E0763. The ownership-edit and
  exact-location assertions remain. Tooling source is now
  `dev-b71678d6664e536a1f691abf300c9491cadcc3631f4e1766f63d9f672a505ec9`;
  its whitespace check passes, but no follow-up build/test has run. The last
  executed Tooling result still has five failures. Andrew has been asked to
  approve one follow-up offline macOS pass, after compiler checks: one build job,
  15 minutes, 6 GB RAM, estimated cache growth 0.2-0.7 GB within the unchanged
  aggregate ceiling. That permission remains pending; installs, pins/locks,
  staging, commits and publishing remain held. Stage 36a stays estimated at 55%;
  remaining public surfaces, performance and delivery gates are unchanged.
  Invalidated elsewhere: compiler/LSP diagnostic review edits, source locations
  and publication-test baselines; public language/ownership rules are unchanged.

- PHP hosted callbacks now reuse group-owned request/region borrow homes across
  slot replacement. Dispatch passes the checked argument modes directly; teardown
  drains before releasing homes. The fixture covers all eight argument modes,
  1,000 resubmissions per standard device, replacement, failed startup and release.
  Shared checked-error forwarding now clears function-scoped PHP catch carriers
  after transfer; source catches release their carrier before handler cleanup.
  Named/unnamed catches and nested finalizers check weak carrier lifetime and
  exact Error destruction without changing the original Error or diagnostic facts.
  Compiler method/accessor exports also preserve retained loans on Copy regions
  through the existing return-borrow predicate. Regressions cover nullable views,
  aliases and independent generic Copy snapshots; no ownership-checker, MIR,
  independent-verifier or runtime-ABI rule changed.
  At `dev-9ee7f43ad727efc5167d0a8aba9dbcbf08f2ba018358129fa17462f7e239027f`,
  macOS passes 319 compiler units, 138 PHP backend, 45 checked-error, 17 key,
  34 group, 24 plan, eight IDE, 27 byte-region and 88 property-hook checks:
  700 total. Linux ARM64/Debian 13.7/Rust 1.98.1/PHP 8.4.27 passes 51 PHP emitter,
  138 PHP backend, 45 checked-error, eight IDE and 27 byte-region checks: 269 total.
  None are ignored. Native comparisons cover Cranelift/LLVM on macOS and
  Cranelift on Linux; fresh Linux LLVM and Windows execution remain untested.
  Strict compiler all-target Clippy, formatting, fixture PHP syntax, documentation
  authority and whitespace checks pass on macOS. The fingerprint is unchanged
  across execution; this checkpoint is a subsequent documentation edit.
  One combined macOS batch hit its ten-minute limit after 519 completed checks;
  the unfinished group/plan targets subsequently pass in a separate bounded run.
  No assertion failed in that batch. Its orphan executable and the temporary
  failure reproducer are removed. Docker reused the existing image/cache with
  one CPU, 2.5 GiB memory, read-only source/dependencies and no test networking;
  no out-of-memory event occurred. A preflight accidentally requested a floating
  Rust update, which failed and rolled back; checks explicitly selected the
  unchanged installed Rust 1.98.1. The owned container is removed and Docker is
  stopped; unrelated resources and healthy caches are preserved. Allocated
  aggregate caches total 23,062,974,464 bytes, below the 25 decimal GB ceiling.
  Full allocation/performance, remaining public surfaces and integration gates
  remain open; Stage 36a stays estimated at 55%, not delivered. No install,
  commit, pin change or publication occurred. Invalidated elsewhere: generated
  PHP/callback allocation baselines and compiler-owned IDE loan metadata;
  accepted public language/API facts are unchanged.

- PHP hosted transfer groups now retain native address/cast views and key
  scratch in the group owner. Observation copies exact identity words into an
  independent typed key; region access and resubmission write into the reusable
  scratch without allocating another native key buffer. Callback order, checked
  errors, draining and ownership are unchanged. The regression checks 1,000
  resubmissions per standard device, scratch identity, signed key extremes,
  foreign-key rejection/recovery, stored-key independence and completed regions.
  At `dev-74e77a3f51e2c23e33775398212af3dbe626c1d0fe83aba00d20f569046f8ab7`,
  macOS ARM64/PHP 8.5.12 passes 51 emitter units, 137 backend, 17 registration-key,
  34 transfer-group and 24 transfer-plan tests: 263 total. Linux ARM64/Debian 13.7/
  Rust 1.98.1/PHP 8.4.27 passes the same selections with 23 non-LLVM plan tests,
  plus 70 readiness and four PHP-launch tests: 336 total. None are ignored.
  This Linux run also covers the preceding file-address change. Native
  comparisons cover Cranelift/LLVM on macOS and Cranelift on Linux; Linux LLVM
  and Windows execution remain untested for this snapshot. Strict compiler
  all-target Clippy passes on macOS. Source identity and checked hashes remain
  unchanged across execution; runtime ABI 15 and independent MIR validation are
  unchanged. Docker reused the cached image with one CPU, 2.5 GiB memory,
  read-only source/dependencies and no networking during tests. Only the owned
  container received PHP packages; it and its temporary files are removed.
  Docker and compiler test processes are verified stopped; unrelated resources
  and healthy reusable caches are preserved. Allocated caches total
  22,138,224,640 bytes, below the aggregate 25 decimal GB ceiling. Remaining
  borrow-cell/value transport, full allocation/performance, public-surface and
  integration gates keep Stage 36a estimated at 55%, not delivered. No installed
  refresh, compiler commit or publication occurred. Invalidated elsewhere:
  private PHP group/key emission and allocation baselines; public language/API/
  editor facts are unchanged by this optimization.

- PHP owned files now cache native storage/status address views in the existing
  file owner. Reads, writes, seek/inspection/control, pending preparation and
  teardown reuse those views. Teardown invalidates the cached storage address
  while retaining its backing allocation through native drop. The new regression
  checks 1,000 paired transfers, actual byte contents and progress, independent
  owner/status storage, checked seek failure and recovery, consumed-file rejection
  and idempotent drop. Method-source checks reject address/buffer construction
  inside these operations. An initial test incorrectly read after consuming
  close and hit the native invariant; it now tests a valid checked failure before
  close, with no production invariant change. Temporary diagnostics are removed.
  At `dev-fb340d9e4763edb56a177c59872387c942f680842d0c1ec2d9497291274b957a`,
  all 51 PHP emitter units, 137 PHP backend, 34 transfer-group and 24 transfer-plan
  regressions pass on macOS: 246 total, none ignored. Native comparisons retain
  Cranelift/LLVM coverage. Strict compiler all-target Clippy, formatting,
  documentation authority and whitespace checks pass. The source identity and
  checked emitter/verifier/ABI hashes match across execution. Runtime ABI 15 and
  independent MIR validation are unchanged. This private PHP change has no fresh
  Linux or Windows execution yet; earlier platform evidence keeps its own source
  identity. Full host-allocation measurements, other transport/public-surface
  work and integration gates remain open; Stage 36a remains estimated at 55%,
  not delivered. Allocated caches total 21,940,199,424 bytes, below the aggregate
  25 decimal GB ceiling; healthy caches are preserved. No Docker start, installed
  refresh, compiler commit or publication occurred. Invalidated elsewhere: PHP
  file-owner emission and allocation baselines; public language/API/editor facts
  are unchanged.

- Before the file-address change, strict Windows MSVC-target checks passed at
  `dev-dc4060c7b401f60d3ac0d049ab2a420e71eddd69f362735d8a9acc6ef01b7269`:
  standalone and hosted runtime configurations, including their test targets,
  and all compiler targets. The compiler cross-check explicitly omitted runtime
  bundling and LLVM; no production defaults changed. These are compile-time
  checks, with no Windows linking or execution claim. Installed targets and
  existing caches were reused offline with one build job.

- Andrew explicitly approved messaging `Doria - Tooling` with "Yes, message
  Doria - Tooling" for already-approved Stage 36a editor integration, then approved
  one offline macOS verification pass: one build job, 45 minutes, 6 GB RAM and
  the unchanged aggregate 25 GB cache ceiling. That pass is complete against
  compiler `dev-6a519968d4ced79aaafd582e9669941707b7ec4f76828df7f6e2a2d183a0b8f3`
  and tooling `dev-2fa377cf67fc8b255fab4b4cca54c73c6f82f4d64e2294855917f47eb76935fb`.
  The managed explicit-compiler server build, formatting, strict all-target
  Clippy and three CLI tests pass; units pass 296/297 and protocol tests 138/142.
  Static-accessor receiver signatures, hover/completion handling and ambient-I/O
  expectations were corrected. Removed the stale expectation that a writable
  getter makes its returned borrow writable; the misuse remains an E0478 negative.
  The compiler owns the missing `getReadRegion` retained-loan export, corrected
  and compiler-tested in the later snapshot above but not yet rechecked through
  the LSP. Four enabled protocol mismatches remain: E0654/E0763 classification,
  missing E0658 review edits, duplicate E0657 expectations and a diagnostic span.
  Final failure logs remain in `/private/tmp/doria-tooling-*-final-20261030.log`.
  Peak sampled process-group RSS was 901,562,368 bytes; costly-command elapsed
  time was 968.1 seconds. Aggregate cache growth was 699,879,424 bytes, ending at
  22,806,245,376 bytes. The production pin stays
  `f500042e1546a7282311423c31164131536ed1e1`; the isolated local runner did not
  change the production manifest/lock. Regular production-pinned builds remain
  unverified. No install, commit or publication occurred. Further Tooling runs,
  installs, pin changes, commits and publishing remain held. Integration is
  incomplete. Invalidated elsewhere: Tooling accessor/hover presentation and
  fixture expectations, plus the compiler export fix above; language rules are
  unchanged. The earlier attempted message was blocked before chat-specific
  approval and was not sent.

- Repaired generated-PHP test execution after Linux exposed 22 argument-length
  failures among 50 emitter units. A shared owned script/driver pair replaces
  generated source in process arguments, preserving PHP options, binary stdin,
  status, explicit entry and cleanup. Each test crate has one execution-directory
  owner. Four regressions cover large sources, exactly-once entry, stdin/options
  and cleanup after success or failed launch. The wider command-line checks also
  exposed empty ordered construction requesting an invalid intermediate hash
  callback. Empty inputs now construct their destination directly; shared MIR
  lowering recognizes grouped inputs, and author-defined `from` calls retain
  their bodies. The new regression covers all five constructor families, three
  grouping forms, non-Cloneable elements, later insertion and exact destruction.
  At `dev-86614be5cd9d452054f8614a7ac5c42b946643dee22e2e3b601ad94fdb9e9c3e`,
  macOS ARM64/PHP 8.5.12 passes 50 emitter units, 137 backend, four runner and
  four command-line regressions: 195 total. Linux ARM64/Debian 13.7/Rust 1.98.1/
  PHP 8.4.27 passes those same selections plus 70 readiness and 17 registration-key
  regressions: 282 total. None are ignored. Native comparisons cover Cranelift
  and LLVM on macOS and Cranelift on Linux; Linux LLVM and native Windows remain
  unexecuted. Strict compiler all-target Clippy passes on macOS; formatting,
  documentation authority and whitespace checks pass. The tested source identity
  is unchanged across execution; this checkpoint is a subsequent documentation
  edit. Runtime ABI and independent MIR validation are unchanged. Docker reused
  the cached Rust image with one CPU, 2.5 GiB memory, read-only source/dependencies
  and no networking during verification; PHP was installed only in the disposable
  container. The owned container and empty temporary directory are removed,
  Docker is verified stopped, and unrelated containers/images are untouched.
  Allocated caches total 21,779,410,944 bytes, below the aggregate 25 decimal GB
  ceiling; healthy caches are preserved. Overall Stage 36a remains estimated at
  55%, not delivered. Remaining public surfaces, performance evidence,
  integration and installed refresh remain open. No sibling edit, commit or
  publication occurred. Invalidated elsewhere: PHP construction/allocation
  baselines and test-launch transport; public language/API/editor facts are
  unchanged, so no language-server fact or pin change belongs to this correction.

- Andrew confirms Windows is already configured and running Doria, with PHP 8.5
  on native Windows and PHP 8.5.9 in WSL. Windows remains in every design and
  implementation change. Ask for a focused laptop run when ready; setup is
  covered, and only concrete execution failures should be reported as blockers.

- Removed per-call PHP receiver-home closures through shared temporary
  tracking. Owned temporaries retain explicit destruction; receiver aliases are
  only unset. Receiver emission precedes argument emission, preserving reverse
  cleanup order, and null-safe chains keep arguments lazy. Methods, shared
  payload methods and property getters use this common path. The new regression
  checks original storage, foreach access, nullable calls, temporary receivers
  and arguments, checked failures and exact cleanup across the interpreter,
  PHP, Cranelift and LLVM on macOS. At
  `dev-d404db7f657b31b222efc27176767d34359a14e60fc2815617e19aca9d8928e0`,
  all 318 compiler library units, 136 PHP backend, 18 property-initializer,
  70 readiness, 36 returned-borrow, 76 returned-closure and 43 writable-writeback
  regressions pass: 697 total, none ignored. Strict library/six-target Clippy,
  formatting, documentation authority and whitespace checks pass; source
  identity and seven checked hashes are unchanged across verification.
  Native ABI and independent MIR validation are unchanged. Nested diagnostic,
  consuming-call and argument/value transport, full allocation/performance and
  platform/integration gates remain open. The overall Stage 36a estimate remains
  55%, not delivered. Allocated caches total 21,150,412,800 bytes; healthy caches
  are preserved. No Docker start, installed refresh, sibling edit, commit or
  publication occurred. Invalidated elsewhere: private PHP call/cleanup output
  and allocation baselines; public language/API/editor facts are unchanged.

- PHP readiness sets now retain their status-address view, and registration
  keys prepare immutable identity bytes once in their checked factory. Repeated
  membership operations no longer construct address views or repack identities.
  Strengthened regressions check address/buffer ownership and reuse, readonly
  identity storage, exact signed-word transport, foreign/stale rejection and
  teardown. At
  `dev-ef70e01155a11dc39feb087ea4c45417c07d96398a1513c316187d74ed8e9b9d`,
  all 50 PHP emitter units, 135 PHP backend regressions, 70 readiness contract
  tests and 17 registration-key tests pass on macOS, with no selected ignored
  cases. Strict library/three-target Clippy, formatting, documentation authority
  and whitespace checks pass; source identity and emitter/verifier hashes are
  unchanged across verification. Native ABI and independent MIR validation are
  unchanged. Remaining expression/argument transport and full host-allocation
  measurements remain open, alongside public decisions, platform/integration
  gates and installed refresh. The overall Stage 36a estimate remains 55%,
  not delivered. Allocated caches total 21,136,535,552 bytes; healthy caches are
  preserved. No Docker start, sibling edit, commit or publication occurred.
  Invalidated elsewhere: generated PHP readiness/key-helper structure and
  allocation baselines; accepted language/API/editor facts are unchanged.

- PHP readiness sets now own reusable status and registration-key FFI buffers,
  initialized during construction and released during teardown. Membership
  lookups/removal write exact key words into that storage; readonly/writable
  getters return the pending member's existing borrowed home. Per-lookup owned
  FFI buffer and borrow-cell construction is removed. The regression checks
  1,000 paired accesses, independent set scratch, original-owner mutation,
  foreign/stale rejection, re-registration without retargeting stale homes and
  exact once-only cleanup. Registration-key tests also reuse caller-owned
  storage across both generic key types and signed-word extremes.
  At `dev-3f1fa3476c195f614998d6dae2549583477ad91dc400791bb997ff00ec6a8f5a`,
  all 50 PHP emitter units, 135 PHP backend regressions and 70 readiness contract
  tests pass on macOS, with no selected ignored cases. Strict library/two-target
  Clippy, formatting and whitespace checks pass; the source fingerprint and
  checked emitter/verifier hashes remain unchanged across verification. Native
  runtime ABI and independent MIR validation are unchanged. These assertions
  do not measure total host allocation: FFI address views, key-identity strings,
  remaining expression/argument transport and the full allocation gate remain
  open. Public decisions, platform/integration gates and installed refresh
  remain pending; the overall Stage 36a estimate remains 55%, not delivered.
  Allocated caches total 21,200,330,752 bytes; healthy caches are preserved.
  No Docker start, sibling edit, commit or publication occurred in this beat.
  Invalidated elsewhere: generated PHP readiness payload/key-helper structure
  and allocation baselines; accepted language/API/editor facts are unchanged.

- Overall Stage 36a planning estimate: 55%. Tracking weights are implementation
  50%, verification/performance 30%, and integration/delivery 20%; their estimated
  completion is 70%, 50%, and 25% respectively. This includes the full accepted
  scope, remaining public surfaces and delivery gates, not a test-count ratio.

- Following Andrew's Docker confirmation, fresh Linux ARM64 verification at
  `dev-afe591e029cee33ce2067a0d4645ad8c7228ece7128beb83e7ca0c96eba9c322`
  passes all 636 standalone and 637 hosted runtime tests, plus all eleven
  indexed-foreach compiler regressions, with no ignored cases. The initial
  compiler launch omitted the explicit dirty-source identity and stamped
  `unknown`; it was rebuilt and the compiler checks rerun with the canonical
  fingerprint before accepting compiler evidence. The fingerprint is unchanged
  across execution. A Linux Cranelift build of the existing readiness-set
  construction fixture matches its committed stdout byte-for-byte, exits zero
  and emits no stderr, including checked-exit and owned cleanup behavior.
  The cached Rust image remains
  `sha256:441452f8033b93f69dce3a3ff5c821c411f0a580723e6343932fc34a1b03ba5d`;
  it supplies Debian 13, Rust 1.98.1 and GCC 14.2.0. CPU is capped at one,
  memory is capped at 2.5 GiB with no additional swap, source/dependencies are read-only and
  networking is disabled. No packages or images were downloaded. PHP and LLVM
  18 are absent, so their Linux execution remains unverified. Windows execution
  remains pending; Andrew offered his laptop when the checks are ready.
  The owned smoke directory and container were removed, no other container was
  running, and Docker app/backend/VM shutdown was verified within 20 minutes.
  Allocated caches total 21,142,560,768 bytes, below the aggregate 25 decimal GB
  ceiling; healthy caches are preserved. No implementation, installed tools,
  sibling sources, pins or remote refs changed in this verification run.
  Invalidated elsewhere: Linux validation records only. Remaining public,
  allocation, integration and platform gates keep Stage 36a incomplete.

- PHP core equality, hash, ordering and clone adapters now share named functions
  keyed by checked type specialization and operation, including initializer
  scopes. Direct equality and collection callbacks reuse them; collection Copy
  callbacks name the existing copy helper. Capture-free adapter closures are
  removed, while nested diagnostic closures and host allocation measurements
  remain open. The regression preserves nullable equality, generic dispatch,
  operand order, checked-failure short-circuiting and collection initialization.
  It also exposed spelling-only dictionary projection recognition in MIR/PHP:
  ordinary collection properties named `keys` or `values` were misclassified.
  Both consumers now share the checked HIR iterable classification. Grouped
  property iteration and real nested dictionary projections agree across the
  interpreter, Cranelift, LLVM and PHP on macOS. All 317 compiler library units,
  135 PHP backend regressions and 11 indexed-foreach tests pass, with none
  ignored. Strict library/two-target Clippy, formatting and whitespace checks
  pass; seven checked source hashes remain unchanged across verification.
  The native verifier and runtime ABI are unchanged; V-A is still pending.
  Allocated caches total 19,525,763,072 bytes; no healthy cache was removed.
  Public lock decisions, remaining allocation/performance/platform and sibling
  integration gates, installed refresh and Stage 36a completion remain open.
  No Docker start, sibling edits, commits or publishing occurred.
  Invalidated elsewhere: generated PHP callback structure and backend projection
  recognition; accepted language/API facts and editor contracts are unchanged.

- Plain local PHP assignments now stage their RHS through the shared statement
  region, including grouped targets, writable Move parameters and owned-value
  replacement. The regression preserves Copy-parameter isolation, exactly-once
  production, destruction order and the previous value after a checked failure;
  its authored assignment bodies construct no diagnostic closures. Compound
  updates and projected targets retain their existing place/snapshot ordering.
  On macOS, all 133 PHP backend tests, 49 PHP emitter units and 18 numeric
  regressions pass, with no selected ignored cases. The numeric target includes
  interpreter/Cranelift/LLVM/PHP parity. Strict library/PHP-target Clippy,
  formatting and whitespace checks pass; the three changed-source hashes are
  unchanged across verification. Allocated caches total 19,463,606,272 bytes,
  below the aggregate 25 decimal GB ceiling; no healthy cache was removed.
  Nested-expression allocation, full performance/platform and integration gates
  remain open. V-A and public lock decisions are pending; no Docker start,
  installed refresh, sibling edits, commits or publishing occurred.
  Invalidated elsewhere: generated PHP structure and regression assertions;
  public language/API contracts are unchanged. Stage 36a remains incomplete.

- Parenthesized PHP root operations now reuse statement-region production,
  grouping and conversion phases without constructing diagnostic closures.
  Each phase keeps its own authored span; the underlying producer still emits
  once. All 132 PHP backend regressions and ten diagnostic-provenance units pass
  on macOS, including grouped checked calls, returned-object cleanup and exact
  interpreter/PHP panic output. Strict Clippy, formatting and whitespace checks
  pass. Nested expression closures and measured allocation/platform gates remain
  open. Allocated caches total 19,482,771,456 bytes; healthy caches are untouched.
  V-A, public lock design, installed tools and publishing remain unchanged.
  Invalidated elsewhere: PHP output structure and its regression assertions;
  public language/API contracts are unchanged. Stage 36a remains incomplete.

- The private lock substrate's 36 selected runtime tests pass sequentially on
  macOS, including native cross-process interoperability, ownership-preserving
  contention/error retry, guard cleanup and allocation probes. The selection
  also includes nonblocking-I/O tests; it is not 36 distinct lock contracts.
  Microsoft's `LockFileEx` contract confirms the existing immediate-failure
  flag avoids asynchronous lock requests. Runtime sources are unchanged;
  Windows execution and public lock declarations remain unverified/deferred.
  The proposed guard-file projection and V-A still require Andrew's decisions.
  No installed refresh or publishing occurred. Invalidated elsewhere: none.

- PHP statement regions now stage root diagnostic conversions after guarded
  value production, removing the output-conversion closure while preserving
  checked origins and statement-temporary cleanup. The PHP target records 128
  passes; three corrected fixture/output-structure cases pass on focused rerun.
  Signed-minimum and shadowed-local checks retain exact execution assertions;
  the new display regression checks returned-object destruction and error/finally
  order. All ten diagnostic-provenance units, strict library/PHP-target Clippy,
  formatting and whitespace checks pass on macOS. Nested expression closures,
  measured allocation counts and full Stage 36a validation remain open. V-A,
  native verifier sources, installed tools, siblings and remote refs are unchanged.
  Invalidated elsewhere: generated PHP output structure and matching assertions;
  public language/API contracts are unchanged.

- PHP primitive `Comparable` dispatch now calls shared signed/string and
  unsigned comparison helpers instead of constructing a closure per call.
  Three selected regressions pass on macOS: helper reuse with exact interpreter
  parity, left-to-right single evaluation and checked-error short-circuiting;
  nominal/generic ordering; and ordered-collection transport. Formatting and
  whitespace checks pass. Host allocation counts and timing are unmeasured;
  diagnostic-expression closures and the full allocation gate remain open.
  V-A, native verifier sources, installed tools, sibling repositories and remote
  refs are unchanged. Allocated caches total 19,396,538,368 bytes, below the
  aggregate 25 decimal GB ceiling. Invalidated elsewhere: PHP generated-code
  structure; public language/API contracts are unchanged. Stage 36a is incomplete.

- PHP diagnostic operation tokens are now scalar previous-site values. Authored
  call frames reuse a flat scalar table bounded by peak active depth, clearing
  retired entries without discarding capacity. Normal and checked exits retain
  existing `try`/`finally` scopes; panic paths read only active frames. The new
  storage tests exercise 5,000 fixed-depth iterations in six scalar slots, nested
  expression unwinding, and exact interpreter parity after checked recursion.
  This removes explicit token and per-call frame-record arrays. Host allocator
  calls were not measured; expression closures and the full allocation gate
  remain open. At
  `dev-4da921f239d295b955e82765a522718e198ecf1cfd8a04a80d60a7cae05dfc35`,
  373 selected checks pass: 49 PHP units, 18 numeric, 126 PHP backend, 16
  Copy-cleanup, 88 property-hook and 76 returned-closure cases. The same two
  enabled native `when` result-path failures remain; V-A is still pending.
  No selected case is ignored. Strict Clippy passes. Full source identity and
  six checked hashes match after execution; MIR model/lowering/validation and
  native ABI 15 are unchanged. The offline one-job preparation build takes
  134.20 seconds and peaks at 1,295,663,104 bytes under the 1.5 GiB cap.
  Execution is sequential on macOS; fresh Linux/native Windows remain open.
  Allocated caches total 19,389,636,608 bytes; healthy caches are untouched.
  No temporary probe, installed refresh, Docker, sibling edit, commit or
  publication was added. Public locks/adapters/processes, remaining allocation,
  benchmark, integration and full Stage 36a gates remain open.
  Invalidated elsewhere: generated PHP diagnostic artifact structure and
  allocation baselines; earlier Tooling semantic/API and Website versioned
  guide/API reference integration remain open.

- Narrowed native `mixed` numeric updates now read the existing exact payload
  and use ordinary boxed replacement, including nullable and writable borrowed
  homes. The shared local-assignment ownership check remains in force. The MIR
  model and independent validator are unchanged; missing and wrong `is` proofs
  still fail validation. PHP integer and float64 updates now share stable-place
  sequencing: evaluate the target once, snapshot before the RHS and commit only
  after success. Target owners register before RHS owners for reverse cleanup.
  This also fixes invalid PHP write-context code for boxed float64 updates.
  Direct integer locals still need no temporary cell or sequencing closure.
  PHP's existing float32 precision and floating-increment boundaries remain.
  At `dev-46f9442d02e76d267cff0bc4934c948eb29cdd7dbd4c90f02870c64e6d3428b4`,
  245 selected checks pass and two known native `when` cases fail: 18 numeric,
  122 MIR validation, 43 writable-borrow, 46 PHP units and 16 Copy-cleanup pass.
  Only two PHP output assertions then changed, adding IEEE infinity/NaN checks;
  the compiler and nine checked source hashes remain identical. At
  `dev-db9b65d18a4f6380ec0e1c5a0dd078b0943f1e05ff712f7b4761189ceb1f8d25`,
  another 290 pass: 126 PHP backend, 88 property-hook and 76 returned-closure.
  The total is 535 passes and two enabled failures, with no selected ignored
  cases. V-A remains pending for the separate `when` result-path change; neither
  failure is suppressed or changed to expect rejection. Strict Clippy passes.
  The last offline one-job preparation build takes 91.11 seconds and peaks at
  1,278,345,216 bytes under the 1.5 GiB cap. All execution is sequential on macOS;
  fresh Linux/native Windows remain unverified. Allocated caches total
  19,420,962,816 bytes; healthy caches are untouched. The owned PHP reproduction
  file was removed after regression coverage and a no-consumer check. Full
  allocation, public locks/adapters/processes, platform, benchmark, integration,
  installed refresh and Stage 36a gates remain open. No Docker, sibling edits,
  commits or publishing occurred. Invalidated elsewhere: PHP artifact/structural
  fixtures; Tooling semantic/API and Website versioned guide/API reference
  integration remain open.

- Statement-boundary PHP `when` results now share field and ordinary value
  destinations. Scalar, managed Copy and owned results avoid a conditional
  expression closure in local initializers and returns. Grouped/nested yields,
  `given`, statement temporaries and checked-error replacement retain cleanup.
  Owned result slots reset before each evaluation and register cleanup once;
  the new loop/predicate regression reproduces and prevents early destruction
  of a value already transferred to another owner.
  At `dev-8a4a66179e17523de617331da5e7bb20cffb60855a503c7512675979601415df`,
  352 selected checks pass: 46 PHP units, 16 Copy-cleanup, 126 PHP backend,
  88 property-hook and 76 returned-closure cases. Two enabled native cases fail
  with the existing malformed-MIR result-path error: enclosing-loop exits and
  a checked-error predicate in a looped `when` with a finalizer. Both PHP cases
  pass. The independent core fixture passes interpreter/Cranelift/LLVM/PHP.
  Neither native failure is ignored or changed to expect rejection. V-A remains
  pending; MIR/lowering/validation sources are unchanged. Strict Clippy passes.
  Source identity and eight hashes match across execution. The final offline
  build takes 151.93 seconds and peaks at 1,210,875,904 bytes under one job and
  the 1.5 GiB cap. All execution is sequential on macOS; fresh Linux/native
  Windows remain unverified. Allocated build caches total 19,352,883,200 bytes.
  The full allocation gate remains open for embedded/assignment expressions,
  arguments, callbacks, erased/stored and nested/managed Copy transports.
  Public locks/adapters/processes, benchmarks, integration, installed refresh
  and the full Stage 36a gate remain open. No publishing or sibling edits occurred.
  Invalidated elsewhere: PHP artifact structure/fixtures; earlier Tooling
  semantic/API and Website versioned guide/API reference integration remain open.

- Conditional PHP payload-enum field destinations now reuse the existing `when`
  branch and cleanup emitter. Nested, nullable, `given`, finalizer and replacement
  paths avoid enum boxes and `when` expression closures at these destinations.
  Generated labels share a function-scoped allocator, and yielded statement
  owners remain live through result binding under Decision 0081.
  At `dev-d8f3e577b14f0d34a622348efe6b4df8f007157a8aea8d90bc2116de25c3d996`,
  349 checks pass and one enabled native regression fails: 44 PHP units,
  15 Copy-cleanup cases, 126 PHP backend, 88 property-hook and 76 returned-closure
  cases pass. No selected case is ignored or skipped. The PHP loop-exit case
  returns 35 correctly. The same accepted loop exit fails full native MIR
  validation with `when branch terminates before assigning and merging its result`.
  Decision 0116 allows these enclosing-loop `break` and `continue` exits; this is
  an implementation gap, not invalid Doria. The active failing cross-backend
  regression is kept in `inline_copy_when_loop_exits.doria`.
  Auto-review rejected the proposed broader result-path verifier edit because
  malformed-control-flow acceptance could widen. That verifier edit was never
  applied; only the owned unverified MIR-model additions were removed, restoring
  the prior MIR/lowering/validator sources. Approval V-A is pending for a scoped
  shared MIR exit model, route validation and positive/corruption regressions.
  Strict compiler-library/four-target Clippy passes. The preparation build takes
  168.41 seconds and peaks at 1,222,737,920 bytes under one job and the 1.5 GiB cap.
  All execution is sequential on macOS; fresh Linux and native Windows remain
  unverified. The full allocation and Stage 36a gates remain open. Build caches
  total 19,334,221,824 allocated bytes, below the aggregate 25 decimal GB ceiling.
  No healthy cache cleanup, installed refresh, Docker, commits, remote writes or
  sibling edits occurred. Invalidated elsewhere: PHP artifact structure and
  fixtures; the accepted native control-flow gap needs MIR regression closure.
  Prior Tooling semantic/API and Website versioned guide/API reference integration
  remain open.

- PHP trivial Copy enum transport now includes field-return bodies for ordinary
  functions and methods and native file, standard-stream and pending outcomes.
  Typed boxed entry points preserve existing argument, stored and callback
  boundaries. Return fields publish after owned and consuming cleanup and
  finalizers. Nominal case writers preserve signed values and named-argument
  order; nullable presence remains separate from zero-valued payloads.
  Field-backed locals, copies, replacement and matches share that transport.
  Expression-level matches reuse field dispatch. Statement temporaries survive
  through result binding under Decision 0081, including compound assignments;
  interpreter and PHP destruction-order regressions agree.
  Shared MIR result-temporary lowering now copies cleanup-free enum values out
  of temporary storage. This fixes valid `when` returns previously rejected as
  malformed borrowed MIR; independent validation is unchanged. New `when`,
  match, nullable and coalescing returns pass interpreter, Cranelift, LLVM and PHP.
  **The PHP allocation gate remains open:** nested and managed Copy values,
  `when` result materialization, boxed arguments, callback, erased and stored
  transports, and diagnostic and arithmetic expression closures remain.
  At `dev-250d46db9eb47204c1601d051d1f4a945a20c86175b3c4115f5392e3d720af2f`,
  all 544 selected checks pass: 41 PHP units, 126 PHP backend, nine region
  transport, 43 writable-borrow and writeback, 26 standard-stream, 76 returned
  closure, 88 property-hook, 121 MIR-validation and 14 Copy-cleanup cases.
  No selected case is ignored or omitted. Strict compiler-library/eight-target
  Clippy passes. The full source identity and ten file hashes match across runs.
  The offline preparation build takes 261.29 seconds and peaks at 1,071,955,968
  bytes. Builds use one job; all runs are sequential with a 1.5 GiB RSS cap.
  This is macOS evidence. Runtime ABI 15 is unchanged. Fresh Linux and native
  Windows execution, public locks/adapters/processes, benchmarks, integration,
  installed refresh and the full Stage 36a gate remain pending.
  Allocated build caches total 19,371,204,608 bytes, below the aggregate
  25 decimal GB ceiling; healthy reusable caches are untouched. No sibling edits,
  Docker, installed refresh, commits or remote writes occurred. Invalidated
  elsewhere: PHP/MIR artifact structure and structural fixtures. Earlier Tooling
  semantic/API fixtures and the Website's versioned guide and API reference
  integration remain open.

- Generic region returns now share concrete source contracts across free,
  concrete, erased and inherited generic calls. Source discovery and native
  monomorphization reuse checked implementation identities. Specialization
  infers region loans even when a taking symbolic parameter had no carrier
  loan, preserving original storage, writable access and lifetime. Independent
  MIR validation is unchanged. PHP retained-source arguments use value staging;
  native byte pointers reuse wrappers while checking their weak owner on every
  access. This removes unsafe acceptance of input-bound region implementations
  against independent-owned interface requirements (E0755). Escaping storage
  and competing writable access still produce E0762 and E0763. Bounded stored
  regions remain valid under Decision 0110; the existing Holder escape fixture
  now actually returns the holder beyond its local buffer lifetime.
  At `dev-05f6b123437d6ef90b1748812db531715df831afa893728af0ebf376ed03b22d`,
  all 612 selected checks pass: 569 integration cases across 14 targets and
  43 PHP stream, ABI, interpreter and independent-conformance units. Region
  transport executes interpreter, Cranelift, LLVM and PHP parity, including
  writable inherited erased dispatch. Time-bounded batches retain completed
  results and resume only unfinished cases; no selected case is ignored or
  omitted, and interrupted attempts are excluded. Strict LLVM-configured
  Clippy passes for the library and all 14 targets. The full identity and all
  12 touched source/test hashes match across execution. One offline build job
  stays below 1.5 GiB aggregate RSS; the largest passing build peak is
  1,081,425,920 bytes. Runtime sources and ABI 15 are unchanged. This is macOS
  evidence; fresh Linux and native Windows execution remain unverified.
  Combined allocated build caches are 19,262,717,952 bytes, below the aggregate
  25 decimal GB ceiling; no healthy cache was cleaned. The PHP payload-enum
  allocation gate remains open. Public locks/adapters/processes, benchmark and
  platform gates, coordinated integration and installed refresh remain pending.
  No additional C-B ruling is needed. Invalidated elsewhere: Tooling semantic
  and API fixtures, and the Website's versioned guide and API reference.
  No sibling edits, installed refresh, Docker, commits or remote writes occurred.

- PHP standard-device functional parity now uses the shared native device
  coordinator. Checked specialized calls and erasure facts select the native
  dependency for the whole program before its first input operation. Native
  programs route `read_line`, `echo`/`printf` and standard-error output through
  the same checked transport as standard views; getter-only and userland-stream
  programs still run with FFI disabled. Standard reads/writes reuse native
  checked outcomes, preparation uses the existing hosted controller, and
  readiness-source views are cached without owning or closing a device.
  All six byte-I/O convenience intrinsics now execute through the verified
  native runtime; their previous B2301 rejection is removed. Native byte
  results transfer their buffer ownership into PHP without copying the buffer.
  Readiness-set access now follows the checked specialized return plan: Copy
  results return values, while Move results retain their borrowed storage home.
  At `dev-1ae7d6fd44b0a663612f87bd99e83a9800edb6f9974513ac4bfcf607dfc5e8ac`,
  all 172 selected tests pass: all 26 standard-stream and 126 PHP backend
  integration tests, plus all 20 PHP stream/file units. Public pending parity
  executes concrete and erased stdin/stdout/stderr registrations across
  interpreter, Cranelift, LLVM and PHP, including binary queue reuse, selected
  regions, claim conflicts, completion reuse, teardown and stale keys. Units
  cover direct/erased native dependency selection without byte intrinsics,
  closed-device facts, UTF-8 recovery, buffer adoption and FFI-disabled use.
  Disjoint batches run each selected test once; none is ignored. Compiler
  library and both affected integration targets pass strict LLVM Clippy.
  The full identity and nine touched source/test hashes match before and after
  execution. Checks use one offline job, four minutes and 1.5 GiB aggregate RSS;
  the current passing checkpoint's largest observed peak is 1,271,136,256 bytes.
  Earlier fixture errors and the readiness Copy/cell failure were corrected;
  failed runs are excluded. Formatting, whitespace, documentation authority,
  stream completeness and diagnostic style checks pass. Runtime ABI 15 and
  runtime sources are unchanged; these executions are on macOS. Native Windows
  and fresh Linux execution remain unverified.
  **Open PHP allocation acceptance failure:** shared payload-enum factories
  still construct host objects for ordinary outcomes (`codegen_php::emit_enum`).
  Native allocation-free evidence does not establish PHP host allocation
  behavior. Shared PHP value/borrowed-cell transport and structural allocation
  checks must close this gap; outcome-specific caches or a performance waiver
  do not satisfy the accepted Stage 36a contract. Functional parity does not
  complete this gate. Public locks, adapters/processes, cross-repository
  integration, other performance/platform gates and installed refresh remain.
  SPEC, the reference and plan now record functional parity and correct the
  earlier overbroad allocation-free claim. Invalidated elsewhere: Tooling
  semantic/API/backend-capability fixtures, and the Website's versioned guide
  and API reference, including native-runtime deployment requirements.
  Combined allocated caches are 19,246,039,040 bytes: 19,081,416,704 for Doria
  and 164,622,336 for the language server, below the aggregate 25 decimal GB
  ceiling. No healthy cache was cleaned, Docker started, installed tools
  refreshed, commits made or remote refs changed. No owned temporary probes or
  compiler/test jobs remain.

- Andrew approved C-B, **Own erased copy first**, on 2026-10-18. Decision 0138,
  Decision 0134, SPEC and the stdlib reference record it. Copy-to-interface
  conversion creates an independent owned snapshot. An implicit carrier lasts
  through its statement; storing a returned borrow requires an interface owner
  local first. This rejects escaping temporary borrows and preserves the
  original Copy binding's independence. Shared source ownership now consumes
  existing checked interface-conversion facts, including indirect calls.
  Replacement analysis keeps the erased temporary's physical storage type;
  independent MIR checks exact nominal conformance. PHP stages independent
  snapshot cells and keeps literal-null borrows owner-free. PHP file-payload
  classification now uses the shared declaration metadata, removing erroneous
  file ownership and destruction from standard Copy views. The interpreter's
  one-word interface rule no longer rejects ordinary wide nullable Copy values.
  Generic source-parameter erasure now returns an owned snapshot after Copy
  substitution; Move parameters retain their loans. Shared source ownership
  rechecks affected concrete generic bodies and publishes their binding,
  closure and retained-source facts to MIR and PHP. Forwarded local owners,
  nullable values, generic methods, callbacks and composed trait methods are
  covered. Composed methods are selected through their class members, since
  they retain authored source identities. Return checking now rejects loans of
  temporaries in owning and borrowed return paths, including block and arrow
  callbacks. This removes previously unsafe acceptance of temporary-borrow
  escapes through generic callbacks; ordinary Move loans remain supported.
  Copy field and collection-element erasure now stops the source return-loan
  analysis at the checked independent-copy boundary after substitution.
  Shared CFG inference publishes concrete return contracts to source callers,
  ownership replay, MIR signatures and PHP. Generic class discovery reuses
  the existing callable/class expansion, including uncalled methods in nested
  generic field types. Concrete/generic fields, collection elements, borrowed
  getters, named calls and nullable present/absent snapshots are covered.
  Copy snapshots survive parent destruction and collection clearing; Move
  aliases retain their original loans. Owned writable-result permission is
  checked in source and MIR without claiming public standard-I/O execution.
  Independent MIR validation and retained-source contracts remain intact.
  PHP now implements non-nullsafe array-backed `List::clear` and
  `Dictionary::clear`, removing their previous B2301 rejection. Both use the
  existing cell-replacement cleanup to clear the caller's storage, release
  contents once in reverse order, and permit reuse and repeated empty clears.
  At `dev-ca369f841dc27ba37358bbd7c993b640c883214f6340287abac2983d9abc271e`,
  all 444 selected tests pass: 26 standard-stream, 88 property-hook,
  18 initializer, 36 borrowed-return, 43 writable-replacement, all 76
  returned-closure, all 126 PHP backend, ten ABI, six interpreter, one
  independent conformance and 14 PHP stream/file tests. Disjoint batches run
  each selected test once; none is ignored. Snapshot and clear cleanup parity
  covers interpreter, Cranelift, LLVM and PHP. Compiler library and all seven
  affected integration targets pass strict LLVM-configured Clippy. The full
  checkout identity matches before and after execution. Jobs use one offline
  Cargo job, four minutes and 1.5 GiB aggregate RSS; this checkpoint's largest
  observed preparation/lint/test peak is 1,551,056,896 bytes. Earlier failed
  projection/PHP-clear checks led to the shared fixes above and are excluded
  from this passing checkpoint. No additional C-B language ruling is needed.
  Unchanged runtime sources pass 635 standalone and 636 hosted tests at
  `dev-a64393e08693f8e5b1af2e72db141fd12217fbb7cfbca9801fa35a1e1c0653f9`;
  both runtime configurations also pass strict production/test lint. These are
  macOS executions, including Windows model tests; native Windows execution
  and fresh Linux execution remain unverified. The subsequent PHP functional
  checkpoint is recorded above. Stage 36a remains incomplete:
  remaining public lock/adapter/process work, cross-repository integration,
  performance gates and installed refresh remain incomplete. ABI 15 is unchanged.
  Invalidated elsewhere: compiler ownership diagnostics/conversion facts and
  artifact identities, Tooling semantic/API fixtures, and the Website's
  versioned guide and API reference. No installed refresh, commits or remote
  mutations occurred. The Homebrew LLVM libunwind linker warning remains.
  Formatting, whitespace, documentation authority, stream completeness and
  diagnostic style checks passed at that checkpoint; all 13 touched source/test
  hashes matched across its verification and documentation-only record update.

- Named writable Move parameters now replace the caller's actual storage through
  the existing borrowed-home ABI, source ownership checking, independent MIR
  validation, interpreter, Cranelift, LLVM and PHP. Ordinary named Copy
  parameters retain local-value assignment. Native replacement evaluates its
  RHS before acquiring the current old owner, preserving nested replacements
  and destroying each displaced owner once. Shared checked-call proof preserves
  failure edges and conservatively retains unknown normal-return contexts.
  At `dev-1a3b1d1aa01fbbd169f897536c89115d9f3a4855b20f4ed5cd523d21319d7beb`,
  all 42 writeback tests pass in two bounded sequential batches, including six
  new named-parameter cases across interpreter/Cranelift/LLVM/PHP. No assertions
  were removed. After correcting five stale test-only interpreter frame calls,
  the same production source passes 37 call-origin/ABI/interpreter unit tests
  and all 58 selected-storage, pending-contract and standard-device MIR tests
  at `dev-9bf2ecc034e36793ecb4a1d12c66f70a0477b03280313dde9049e7673a8abce7`.
  These 137 current correction tests complement the preceding 274-case
  checkpoint below. Compiler library and writeback-test Clippy pass with
  warnings denied in default and LLVM configurations. Formatting, whitespace,
  documentation authority and stream completeness checks pass. Linux/Windows
  execution and the complete Stage 36a gate remain pending.
  Andrew approved S-A: `StandardStreams::input`, `output`, and `error` return
  `StandardInput`, `StandardOutput`, and `StandardError`. Decision 0110, SPEC,
  the stdlib index and audit now record these names; public execution is pending.
  Runtime ABI 15, installed tools and production pins are unchanged.
  Checks used one offline job with four-minute/1.5 GiB aggregate RSS bounds;
  the largest observed RSS for these passing checks was 1,007,910,912 bytes.
  Aggregate allocated caches are 18,946,875,392 bytes, below 25 decimal GB.
  No healthy cache was cleaned or Docker started; verification jobs have stopped.
  Invalidated elsewhere: compiler artifact identities, Tooling ownership and
  semantic fixtures, and the Website's versioned guide and API reference.
  No commits, installed refresh or remote mutations occurred.

- Shared ownership analysis now preserves every reachable source of a stored
  coalesced borrowed result. Nested aliases, fields, indices, captures and
  retained collection/group loans keep the actual possible owners. Conditional
  storage effects join per-selected-owner outcomes instead of clearing every
  candidate. Writable coalesces preserve checked permissions in MIR and all
  physical replacement types in source and independent MIR validation. PHP
  uses one borrowed-result-cell path, evaluating the left side once and the
  fallback only when absent. Returned captures infer their single parameter
  source through borrowed aliases; unrelated or local escape origins remain
  rejected under the existing return contract.
  Owning captures of borrowed or maybe-borrowed Move aliases are now rejected
  with E0645 under Decision 0121. This removes previously unsafe acceptance;
  taking Copy values and owning retained carriers remain supported. Existing
  E0474 borrowed-transfer diagnostics are preserved.
  Private standard-device transfer/readiness/pending MIR remains shared by the
  interpreter, Cranelift and LLVM, using ABI 15 static source homes and pending
  configurators. File outcome/error projection and canonical process devices
  remain shared, without per-view File objects. Group-lifetime claims retain
  the intrinsic cursor, plan ordering, buffered-prefix/EOF precedence,
  cancellation, reuse and teardown behavior. Absent readonly borrowed returns
  keep ownerless homes; writable nullable returns retain their storage homes.
  At source checkpoint
  `dev-9661ab3ca3b837643aec001be616318e9a6f070c1892ce2840a5256d44610e91`,
  macOS passes all 274 targeted cases: 36 borrowed-return aliases, 34 Stage 30c
  ownership, 36 writable writeback, 13 standard MIR, 30 selected-storage MIR,
  15 pending-stream contracts, 34 transfer groups and 76 returned-closure
  transport. All 76 closure names were listed and each passed once in four
  bounded batches; no assertions or platform-available cases were omitted.
  Execution helpers cover interpreter/Cranelift/LLVM, plus PHP in the applicable
  borrowed-return, closure and group cases. Standard fixtures also store and
  capture coalesced File and private standard-device readiness-source getters.
  The compiler library and three changed test targets pass Clippy with warnings
  denied in default and LLVM configurations. Source hashes match throughout
  the passing suites and after the documentation-only update.
  Runtime code and ABI 15 are unchanged from
  `dev-cdbf6165d82e218b6ab4ab0149858dde86baa78e4601e91d41bd464f51d8f29e`,
  retaining 633/634 standalone/hosted runtime tests, 14 PHP stream tests, four
  PHP group tests, source-replacement parity and Linux/Windows runtime/test
  compilation. Fresh Linux/Windows execution remains unverified.
  The stored-coalescing E0478 implementation gap is resolved. At this checkpoint,
  named writable owning-parameter replacement retained its E0505/M1101 boundary;
  the later shared named-parameter correction is recorded above.
  Public standard-view/lock declarations and remaining integration are incomplete.
  All checks used one offline build job and four-minute/1.5 GiB aggregate RSS
  bounds; peak RSS for the current checkpoint's checks was 1,075,576,832 bytes.
  Every current checkpoint batch completed within its bound. Final checks find
  no matching execution directories or compiler/test jobs. Docker was not
  started; only its pre-existing privileged network helper is present.
  Aggregate allocated caches are 18,900,475,904 bytes, below 25 decimal GB.
  No healthy cache was cleaned, installed tool refreshed or remote ref changed.
  Formatting, diff whitespace, documentation authority and stream completeness
  checks pass. The Homebrew LLVM libunwind warning remains.
  Invalidated elsewhere: shared source ownership acceptance and capture
  diagnostics, MIR permissions, PHP borrowed-cell emission and compiler artifact
  identities. Tooling/editor semantic fixtures and the Website versioned guide
  and API ownership guidance require synchronization with this correction.
  Public standard-view declarations, installed tools and cross-repository pins
  are unchanged.

- Checked standard-device transfers now share the file outcome decoder and the
  existing IoError field transport. Native ABI 14 adds the private checked entry
  point; real PHP/FFI stdin-to-stdout execution preserves binary bytes, EOF,
  empty data and canonical standard-device failure targets without a fake path.
  Ordinary intrinsic/view I/O and Unix/Windows group admission use one fallible
  canonical-device claim. Failed admission preserves the existing claim and
  queued bytes. Unnecessary exclusive admission is removed from empty output,
  read-only TTY queries and the no-op raw output flush. Windows text input and
  output query the exact captured handle; input no longer re-enters its claim
  while checking console identity.
  At unchanged source checkpoint
  `dev-962963ee6424330954e2f113cc9e35ef865789789ca8f22ee2414266685c7f9d`,
  complete macOS standalone/hosted runtime suites pass 628/629 tests, including
  nine new standard-device cases. All 60 stream-capability cases pass in bounded
  batches of 2 hosted transports, 57 regressions and the complete 40-combination
  file-creation matrix. Both runtime configurations pass production/test Clippy
  with warnings denied and Linux/Windows runtime/test compile checks. These are
  macOS execution and cross-check results; fresh Linux/Windows execution remains
  unverified. All checks use one build job, offline dependencies and four-minute/
  1.5 GiB aggregate RSS bounds. Final compiler verification peaked at
  1,187,282,944 bytes; allocated aggregate caches are 17,675,329,536 bytes, below
  the 25 decimal GB ceiling. No healthy cache was cleaned or Docker started.
  Temporary execution directories are cleaned and all verification jobs stopped.
  The Homebrew LLVM libunwind warning remains. Standard-stream declaration
  choice S-A (StandardStreams properties) was subsequently approved by Andrew
  and recorded in Decision 0110; public declaration execution remains pending.
  Invalidated elsewhere: runtime ABI consumers, native standard-device failure
  behavior, Windows text-device handling and build identities. Source-level
  standard-view integration, Tooling/Website facts, pins and installed refresh
  remain pending. No installed tools, commits or remote refs changed.

- Andrew approved `1A, 2A, 3A` on 2026-10-18 and explicitly resumed Stage 36a.
  Decision 0138 records inline Copy standard-device views, explicit borrowed and
  owning file-lock guards, and native platform locking with documented host
  differences. Decision 0110, the plan, SPEC, stdlib index, stream audit/manifest
  and authority guard are synchronized; the old advisory-only promise is removed.
  Public names remain subject to the existing bounded appendix.
  Native whole-file locks use the existing `OwnedFile`: `flock` on Unix and
  `LockFileEx` on Windows, with typed contention, preserved failure ownership,
  and deterministic borrowed/owning cleanup. The first raw-file checkpoint used
  ABI 12 and passed 607/608 standalone/hosted runtime tests.
  Canonical `FileObject` guards now retain the original stable `FileResource`,
  payload, opening path and readiness identity. They add no duplicate handle,
  registry or guard allocation. Explicit release returns the original owned
  file; destructor cleanup destroys its fields and returns backing storage to
  its original allocator. Native class cleanup, PHP FFI arrays and inline
  storage therefore share one implementation without mismatched frees.
  That file-lock checkpoint used runtime ABI 13. The shared PHP byte-pointer declaration also
  preserves explicit lengths and embedded NUL bytes instead of requesting an
  automatically converted C string.
  At source checkpoint
  `dev-57973cc74b681b428816dc84e9948f0c1be2f2abe9fc3f31da0f9566fbfb1c56`,
  complete macOS standalone/hosted runtime suites pass 619/620 tests, including
  32 lock cases. All 59 stream-capability cases pass at that same checkpoint,
  covering interpreter/Cranelift/LLVM/PHP parity and real PHP-owned lock storage.
  A bounded batch reached its time limit after 56 passes; the two interrupted
  cases and the full 40-combination creation matrix then passed separately,
  preserving every assertion. Runtime Clippy passes with warnings denied for
  standalone and hosted production/tests; Linux and Windows check both feature
  sets, including tests. Standalone archives and the shared runtime build with
  all eleven lock exports. Apple `nm` cannot read LLVM 22 bitcode from Rust's
  standard library in the static archive; the eleven native symbols are visible
  there and in the shared runtime. The Homebrew LLVM libunwind warning remains.
  New Linux/Windows execution and public lock compiler/MIR/backend integration
  remain unverified. Checks ran sequentially with one build job, four-minute
  and 1.5 GiB aggregate RSS limits; peak observed RSS was 1,270,857,728 bytes.
  Aggregate allocated caches are 17,295,802,368 bytes, below 25 decimal GB. No
  healthy cache was cleaned or Docker started; owned temporary test directories
  were removed and all verification processes stopped. The Website owner
  synchronized seven guide/API/audit documents; Tooling found no conflicting
  contract prose. Their behavioral validation, pins and installed refresh remain
  pending. Invalidated elsewhere: standard-view compiler ownership and remaining
  public declarations, native lock ABI consumers, PHP byte transport and artifact
  identity, Tooling facts and Website versioned guide/API. Existing uncommitted
  work is preserved; no installed tools, pins, commits or remote refs changed.

- Stage 36a remains the active delivery priority. Andrew's 2026-10-12 use-case
  sequencing amendment is recorded only in the master plan's
  [approved execution sequence](../doria-end-to-end-plan.md#approved-execution-sequence-after-stage-36a).
  It replaces numeric/phase scheduling after Stage 36a; no future product work
  has started and no existing acceptance or publishing gate is removed.

- A separate approved 2.5 GiB Linux ARM64 attempt now passes the complete
  Concurrency/Time suites, 12/23 tests, at source checkpoint
  `dev-c517f0336bf0f29abff3829fac80b360bb18f702693d9bc548af3fa8632b6fe5`.
  Real notification and monotonic-clock fault injection passes through Cranelift,
  LLVM 18 and PHP 8.4/FFI, preserving the original owned checked errors and
  exact result assertions. The compiler library built without an OOM kill;
  test linking exposed a missing C++ development linker library in the slim
  runner. Installing its distribution-owned `libstdc++-14-dev` package fixed
  that prerequisite, without a source workaround or host installation.
  The retry reused the built compiler and passed all 35 tests. Container CPU
  stayed capped at one, memory at 2.5 GiB, and downloads totaled 133,497,644
  bytes; validation ran with its network disconnected. Source identity was
  unchanged across execution. The owned container and temporary packages were
  removed, Docker's app/backend/VM shutdown was verified within the 20-minute
  budget, and no additional image was created. Reusable caches total 16.37
  decimal GB, below the aggregate ceiling. This verifies these selected Linux
  suites, not the full Stage 36a gate or Windows execution. Invalidated
  elsewhere: validation records only; public contracts, installed tools and
  pins remain unchanged.

- Bounded Linux ARM64 verification used a disposable Debian 13 image with cached
  Rust 1.98.1, PHP 8.4.26 with a verified native FFI call, LLVM 18.1.8 and GCC
  14.2.0. At source checkpoint
  `dev-5c75cb1dc978b4ab6fa854c6905adcc0f2052d4e8a0ca98f66fe8dfb88034f21`,
  the complete standalone/hosted runtime suites pass 588/589 tests, including
  the many-stream reuse and shared-owner cost probes. The Linux fault injector
  compiles with strict C warnings. Compiler test building and the distinct
  selected-target type-check both stopped in `doriac` library compilation at
  the approved 1.5 GiB memory limit; neither checked or executed the Time and
  Concurrency test targets. That 1.5 GiB attempt provides no compiler/LLVM/PHP
  parity result.
  Container downloads totaled 132,011,680 bytes; its CPU stayed capped at one
  and resources were not increased. Cargo package cleanup removed the failed
  compiler artifacts while preserving reusable dependencies/runtime caches.
  The owned temporary container/image were removed and Docker's app/backend/VM
  shutdown was verified, within the 30-minute budget. Aggregate allocated
  caches are 15.42 decimal GB, below the 25 GB ceiling. Invalidated elsewhere:
  runtime validation records only; public contracts, installed tools and pins
  are unchanged, and Windows execution remains unverified.

- Real macOS syscall-fault injection now exercises public clock and cancellation
  notification failures in Cranelift, LLVM and PHP/FFI. The PHP emitters had
  bypassed the shared checked-error carrier for ClockError and CancellationError,
  producing an uncatchable host fatal error; both now use the existing carrier.
  Normal controls and injected cases preserve exact messages/system codes,
  requested cancellation, notification retry, group/set reuse and owned errors.
  The complete Time/Concurrency suites pass 23/12 tests. Injection is confined
  to owned test children and generated libraries/programs are removed afterward.
  Linux execution is now verified by the separate checkpoint above; Windows
  execution remains unverified. ABI 11,
  installed tools and pins are unchanged. Invalidated elsewhere: PHP failure
  behavior and validation records; stdlib and Website guide/API contracts already
  require these checked errors, and Tooling declarations remain unchanged.
  The same scoped harness now includes Linux eventfd/LD_PRELOAD injection,
  preserving the complete native/PHP assertions rather than skipping PHP.
  The macOS failure regressions still pass. Linux compiler-test cross-checking on
  this host stopped before checking tests: its shared runtime requires an ELF
  linker, but the installed `cc` uses Darwin's linker. The 1.5 GiB Linux attempt
  verified the injector's C compilation but reached its memory cap before
  checking compiler tests; the separate 2.5 GiB checkpoint now executes the
  public failure cases. No host tool was installed or build requirement
  bypassed. Invalidated
  elsewhere: Unix regression coverage only, with no additional public or
  production behavior.

- Test-only shared-owner probes now guard synchronous file I/O isolation:
  open, bounded binary copy, position/length inspection, flush and close perform
  no runtime clock read, native readiness creation, Windows broker acquisition,
  inherited-device helper submission, cancellation-state creation or signal
  creation. Positive owner controls, nested intervals, unwind reset and thread
  isolation validate the probes themselves. The macOS standalone/hosted runtime
  suites pass 587/588 tests; the Linux suites above pass 588/589 with these
  probes included. Windows test compilation passes, not execution; the Windows
  broker positive control remains unexecuted here.
  These guards do not measure public-program cold startup, kernel resources or
  total process allocation. Production artifacts contain none of the probes.
  Invalidated elsewhere: private runtime tests and validation records only;
  public contracts, Website guide/API, Tooling, installed artifacts and pins
  are unchanged.

- Many-stream runtime reuse now has repeated native coverage on macOS:
  96 persistent watches deliver 32 exact rounds without changing watcher,
  registration-generation or event-storage identity. One transfer group performs
  3,072 reads using the same 96 buffers and request records, preserves sentinel
  bytes, retires each request's pointer, and closes no stream on group teardown.
  Both probes report zero runtime-storage allocations, not kernel-resource or
  global allocator counts. Standalone/hosted runtime suites pass 585/586 tests;
  the Linux suites above now execute these added Unix regressions. Windows
  execution remains unverified. This is
  structural runtime evidence, not the required public Doria benchmark or a
  timing pass. No public contract, installed artifact or pin changed.
  Invalidated elsewhere: runtime validation records only.

- Five existing Stage 36a benchmark scenarios now execute through the canonical
  runner on macOS: file copy, repeated small writes, writable-stream dispatch,
  adapter-chain dispatch and empty-main startup. All 14 Cranelift/LLVM target
  and feature/control combinations pass exact output/status and declared binary
  file checks at compiler source checkpoint
  `dev-9ac6d749af67641f9df7e2f6010ff1e44b4d87c5ac6bceb65a7f9a8459219cc9`.
  The explicit development compiler and its ABI-11 bundled runtimes are
  source-matched; no installed artifact or identity check was replaced.
  Version-2 reports are retained under the benchmark repository's ignored
  `reports/raw/stage36a-20261014-verify-*.json` and `*-quick-*.json` paths.
  The separate quick captures contain one warmup and two measured samples per
  target, child wall/user/system time, kernel-reported peak RSS, artifact sizes
  and available compiler metrics. All are timing-baseline-ineligible; allocation
  and syscall counts were not measured. Empty-main startup is not synchronous
  stream-startup proof, and the remaining required cases/structural gates are
  still unfinished. No peer was built and no baseline was accepted. Build and
  runtime scratch used secondary storage; the existing workload artifact root
  was reused without changing source, pins, tools or publishing. Invalidated
  elsewhere: measurement/validation records only; no public API changed.

- The separate benchmark repository's current harness passes 152 tests on
  macOS, including exact binary-fixture hashes and bounds, short-tail copy,
  repeated writes, adapter/erased-dispatch variant consistency, startup selection,
  source/runtime provenance, report schemas and unavailable-metric handling.
  Its process tests cover failed launches versus real 126/127 exits, bounded
  simultaneous pipe progress, deadlines and descendant cleanup. The run used
  owned secondary-volume test scratch; compiler/build-driver invocations were
  fixture stubs, not native Doria benchmark execution. Benchmark source and
  pins were unchanged. Missing Stage 36a cases and native structural/scenario
  verification remain pending; no timing baseline or comparative claim follows
  from this harness checkpoint. Invalidated elsewhere: validation status only.

- Current Linux ARM64 runtime execution passes 584 standalone and 585 hosted
  tests, with no failures or ignored cases, at source checkpoint
  `dev-d184d5b8df258e55b256cc837a33908cbef972d51cc4083dd2f2522650583e0f`.
  The cached Rust 1.98.1 image
  `sha256:441452f8033b93f69dce3a3ff5c821c411f0a580723e6343932fc34a1b03ba5d`
  ran locked/offline with read-only source/dependencies, one CPU and 1 GiB of
  memory. Builds and scratch used the repository-managed secondary-volume
  Linux cache. An initial launch failed before tests because the source's
  rolling stable selection requested an update in the read-only image; selecting
  its already-installed exact toolchain required no download or source change.
  Test containers were removed and Docker Desktop was stopped. This supersedes
  earlier Linux runtime compile-only evidence, not the remaining Linux compiler,
  LLVM/PHP and Windows execution gates. No installed tools or pins changed.
  Invalidated elsewhere: runtime validation status only; no public contract,
  Website guide/API or Tooling source change.

- Seek transport and readonly file inspection are under integration. Andrew
  approved required Start/Current/End origins, a canonical-int seek result and
  appended IoOperation::Seek, with Access for inspection. The shared runtime
  now separates shared inspection from exclusive cursor mutation, preserves
  opening-path error ownership and null progress, and allocates nothing on
  successful primitive control calls. Standalone/hosted runtime suites pass
  581/582 tests on macOS, both strict runtime configurations pass, and Linux
  and Windows compile checks pass (not execution).
  A focused macOS runtime ownership probe reports zero leaked bytes under
  restricted process-memory inspection; this is not generated public execution.
  Strict LLVM-feature compiler all-target checks pass; documentation authority
  and stream-inventory checks pass. Those are static checks, not public execution.
  Andrew resolved the property-hook conflict on 2026-10-13: readonly
  getPosition/getLength methods replace the proposed position/length properties.
  Fresh inspection may block; Decision 0135's global hook restriction remains
  unchanged. Declarations, examples and regressions now use methods, without
  property aliases or exemptions. Public seek/inspection execution passes on
  the interpreter, Cranelift, LLVM and PHP/FFI. The 49-case stream suite passed
  48 cases initially; the PHP SeekOrigin symbol bridge was corrected and the
  sole failing case passed on rerun. The durable native matrix passed 16 tests
  across 421 examples, comparing exact output, status and file effects before
  that PHP-only correction. Direct/transitive hook rejection and fresh queries
  after another open writes are covered. Compiler library checks passed 287
  tests; the new PHP symbol regression passed separately, not in a full 288-test
  run. Cranelift, LLVM and PHP/FFI each completed 1,000 ownership-probe loops
  with zero reported leaks under restricted macOS inspection. The private probe
  artifacts were removed; reusable caches were preserved.
  Installed artifacts, pins and publishing remain unchanged. Invalidated
  elsewhere: stdlib inventory, Website guide/API and exhaustive IoOperation
  matches, plus Tooling help/fixtures; source owners have the approved seek
  contract and the approved inspection-name correction. Their execution and
  refresh holds remain in place. This is not Stage 36a delivery.

- Andrew approved synchronizeData()/synchronizeMetadata() and distinct appended
  IoOperation::SynchronizeData/SynchronizeMetadata on 2026-10-13. The writable
  file methods use the existing shared file-control transport. This checkpoint
  passes 288 compiler-library, 53 stream, 582 standalone/583 hosted runtime and
  six catalogue tests on macOS, with strict compiler/both runtime checks.
  Linux/Windows runtime compilation passes, not execution. The complete native
  matrix was stopped after Andrew's resize approval arrived so one combined
  run can validate both additions; the stopped run is not a pass. Readable
  files expose neither method. Website guide/API and Tooling descriptions,
  fixtures and source tests are synchronized and source-checked; behavioral
  integration, installed tools, pins and publishing remain held.

- Andrew approved writable setLength(int $length): void throws IoError and
  appended IoOperation::Resize on 2026-10-13. Shared controls preserve the
  cursor, permit tail-discarding shrink/zero-readable growth, and retain
  opening-path errors with null progress. The interpreter's virtual-file
  resizing and writes share fallible storage growth, with no arbitrary cap.
  This combined checkpoint passes 289 compiler-library, 57 stream, six catalogue,
  and 583 standalone/584 hosted runtime tests on macOS. Strict LLVM-feature
  compiler and both runtime checks pass. The full native matrix passes 16 tests
  across 423 examples, comparing exact interpreter/Cranelift/LLVM output, status
  and file effects; its 1,469-second run supersedes the intentionally stopped
  synchronization-only run. Direct/transitive hook restrictions and independent
  MIR type/access checks remain enforced. Native structure uses one shared
  control call, not user-space filling, cursor restoration or eager infrastructure;
  runtime probes report zero successful-control allocations. The previously
  invalid private control tag 6 now performs resizing; unknown-tag and negative
  argument regressions remain present, with tag 7 also checked.
  Linux/Windows all-target runtime compilation passes, not execution. Website
  guide/API and Tooling descriptions, fixtures and source tests are synchronized
  and source-checked. Their behavioral validation and the whole Stage 36a gate
  remain unfinished. Source owners have the verified checkpoint; no tools, pins
  or publishing changed. Private probe and stopped-run artifacts were removed;
  healthy reusable caches were preserved.

- The bounded regular-file copy regression preserves all 1,048,833 binary bytes
  through one reusable 8,192-byte buffer, including its short final chunk. Exact
  source/output bytes, stdout and status agree on the interpreter, Cranelift,
  LLVM and PHP/FFI on macOS. Native helper-reachability guards reject allocation
  and byte-copy calls before the existing checked-file runtime boundary; prior
  runtime probes cover successful primitive transfers. The complete 58-case
  stream suite, argument-forwarding regression for the shared execution helper,
  and strict LLVM-feature compiler all-target checks pass. A separate current
  integration run passed 178 time/cancellation/readiness/plan/group tests.
  This is correctness and structural evidence, not a throughput comparison,
  portable execution gate or public copy-helper implementation. Execution helpers
  retain caller-owned isolated directories for seeded binary inputs and exact
  file inspection; existing scoped cleanup remains unchanged. Invalidated
  elsewhere: shared test-helper callers are checked; no public API, Website or
  Tooling contract changed. Installed tools, pins and publishing remain held.

- Approved CancellationSource construction, token/status properties and
  cancellation requests now execute through shared typed MIR, the interpreter,
  Cranelift, LLVM and PHP/FFI. Managed Copy claims survive nullable and enum
  storage, grouped bindings, captures, taking callbacks, exceptional returns,
  fills, filtering, preserving conversions and public/core collection iteration.
  Promoted fields acquire their own claims, including generic and inherited
  fields; taking Copy parameters remain usable after promotion. Constructor and
  destructor receiver homes are internal lifecycle views, not caller arguments.
  PHP no longer uses host property promotion: explicit initializers precede
  Doria promotions, and failed construction cleans body locals, initialized
  fields/completed ancestry, then still-owned parameters. Two durable snapshots
  were corrected from argument-first to field-first failure cleanup; tests and
  accepted promotion behavior were retained.
  The final selected macOS regression set passes 936 tests, with strict LLVM
  compiler checks. Fast native, LLVM release and PHP/FFI each complete 1,000
  ownership-stress iterations; macOS leak probes report zero leaked bytes for
  all three, with restricted process-memory inspection. Linux/Windows execution
  and full wait-control integration remain unfinished. Later macOS native/FFI
  notification-failure evidence is recorded above. Andrew approved shared-state
  token identity equality on 2026-10-13.
  The dedicated concurrency suite now passes 11 tests on macOS, including
  interpreter/Cranelift/LLVM/PHP parity, distinct PHP carriers for one state,
  nullable equality and stable surviving-token identity. Native borrowed
  comparison checks require no retain/release, clock or state allocation.
  This is separate from the earlier 936-test checkpoint, not a full stage gate.
  The optional typed `begin` parameters `deadline` and `cancellation` are also
  approved. The new private v2 native begin bridge copies the exact deadline
  and retains group-local cancellation state. Unix group teardown removes its
  wake registration before releasing the state; failed removal closes only the
  runtime-owned poller. Windows adapter and pending-broker groups receive the
  same explicit controls. Legacy private v1 set-scoped policies are preserved;
  the new controlled path does not implicitly select them. Reused owning sets
  do not inherit an earlier group's controls.
  Public typed `begin` controls now execute through shared MIR, the interpreter,
  Cranelift, LLVM and PHP/FFI. Native calls use v2 begin; hosted startup uses the
  same explicit-control runtime setup. Shared validation checks both nullable
  control types and retains only the set/plan loans. Declaration defaults now
  fold even without a source body, and nullable token defaults use ordinary
  constant absence. Project graphs inject Concurrency once with correct source
  identity, including callers with no explicit Concurrency import.
  This checkpoint passes 287 compiler-library tests and 157 selected default,
  graph, time, cancellation, readiness and transfer regressions on macOS, with
  strict LLVM-feature all-target compiler checks. Both native structure checks
  require v2 control transport without caller clock/state-allocation calls.
  Runtime suites pass 578 standalone and 579 hosted tests, strict checks of both
  configurations, and Linux/Windows compilation. Group clock-failure tests
  preserve unsubmitted and actual completed results for the next fetch; original
  preparation errors win a racing cancellation. Cranelift, LLVM and PHP/FFI each
  complete 1,000 controlled group teardown iterations with zero reported leaked
  bytes under restricted macOS inspection. Linux/Windows failure-path execution
  and the full Stage 36a gate remain unfinished; later macOS notification-failure
  evidence is recorded above.
  Stage 36a is not delivered; ABI remains 11 and installed tools, pins and
  publishing are unchanged and held.
  Performance impact: native token copies retain without boxing, signal setup or
  shared-state allocation. Scope separation changes cleanup order, not native
  storage layout. PHP retains managed field/collection claims and creates
  nonowning lifecycle homes only when checked borrowing requires them. No new
  wall-clock benchmark result is claimed. Optional native group cancellation
  adds one retained state claim and a group wake membership, not per-transfer
  storage; controlled completion-fetch regressions require zero allocations.
  Invalidated elsewhere: compiled compiler/runtime/editor artifacts need a
  coordinated authorized refresh. The stdlib inventory records development
  execution; versioned website guide/API pages retain the approved target
  lifecycle/ownership contract without development-status caveats. Tooling
  declaration/ownership source synchronization is inspected, while sibling
  build/pin/installed-tool validation remains held. Website lifecycle guide,
  tutorial and failed-construction fixture updates were inspected; PHP lint and
  source-consistency checks pass, but Website compiler/Pest execution remains
  unrun under its holds. The approved equality and group-control declarations
  are also synchronized in the versioned website guides/API and Tooling help
  and regression source; those files were inspected. Website clarified call-only
  control inputs in its byte-stream guide and both group/set API references.
  Tooling added metadata-driven default availability and removed stale
  compiler-integration wording, with source regressions retaining both defaults.
  Website test-source lint,
  Tooling formatting and sibling whitespace checks pass, but their behavioral
  validation remains held. The master plan alone owns
  the unchanged approved execution sequence.

- The private cancellation source factory now reports source-identified P1601
  on allocation failure. The checked request bridge returns owned
  `CancellationError` message/code fields, preserving absent versus present-zero
  system codes and the published request after notification failure. Success
  allocates nothing and initializes no native signal. Runtime ABI identity is
  11; public Source/Token execution was absent at this private-factory checkpoint.
  Installed tools/pins are unchanged. macOS standalone/hosted runtime suites pass
  574/575 tests; strict compiler/runtime library checks pass locally, including
  LLVM, and runtime compile checks pass for Linux and Windows. The macOS leak
  tool reports zero leaks across 24 cancellation tests, with restricted process
  memory inspection. This does not claim Linux/Windows execution or completed
  public cancellation integration.

- Private cancellation-source construction and retirement now use shared MIR
  factory/lifecycle metadata in the interpreter, Cranelift and LLVM. One runtime
  state is allocated without an ordinary class shell; source retirement
  releases its claim without cancelling, and token claims may outlive it.
  Exact declaration ownership is checked; authored fields, constructor
  arguments and destructor metadata cannot override runtime ownership.
  Stack promotion and ordinary-class parameter memory assumptions were removed
  for all opaque runtime owners; proved ordinary-class optimization remains.
  The factory/lifecycle checkpoint passed 282 compiler library tests and 176
  focused regressions on macOS, including five private factory tests and both
  native profiles. Strict LLVM compiler/library and factory-test checks passed.
  Checked cancellation requests and status now use typed MIR in the interpreter,
  Cranelift and LLVM, with Concurrency-owned checked Error transport. Shared
  validation checks runtime source identity, writable request access, exact
  effects, owned Error slots and distinct success/failure edges; optimization
  records request mutation. The interpreter's single shared state owns both
  claim counts and request status. Notification failure preserves the published
  request, nullable system code and ordinary Error cleanup; retrying never
  resets request status or claim counts. Typed token production now acquires an
  independent shared-state claim through the same managed Copy lifecycle in
  the interpreter, Cranelift and LLVM. It borrows the source only for production;
  the returned token retains no Doria source loan and can outlive source
  destruction. Native object checks require the shared retain operation and
  reject factory, cancellation, ordinary-class allocation/free and I/O calls.
  Interpreter tests exercise production and exact cleanup counts with both
  source-first and token-first destruction, plus authored P1503 on claim-count
  exhaustion without changing the live source or shared request.
  All 286 compiler library tests, 11 private cancellation MIR tests and 237
  focused checked-error, MIR-validation,
  Copy-cleanup, time, Concurrency and transfer-group regressions pass on macOS.
  Request/status success executes in both native profiles; notification-failure
  execution was interpreter-only at this checkpoint. Later macOS native/FFI
  failure-path execution is recorded above; Linux/Windows execution and full
  wait-control integration remain unfinished Stage 36a work.
  Public Source/Token declarations, frontend production and PHP managed-token
  integration were absent at this private-MIR checkpoint; their newer execution
  checkpoint is recorded above. These earlier checks were private MIR fixture
  validation, not public Source/Token execution,
  Linux/Windows execution, a complete stage gate or a new leak measurement.
  Performance impact: one source-state allocation, no per-copy boxing or
  implicit cancellation; opaque parameter optimization is conservative.
  No new wall-clock benchmark result is claimed. ABI remains 11; installed
  tools, pins and publishing are unchanged and held.
  Invalidated elsewhere: compiled compiler/runtime/editor artifacts need
  coordinated refresh when authorized. The Tooling owner synchronized the new
  inline-kind match, managed Copy ownership descriptions and regression source;
  those source changes were inspected, but Tooling execution/build/pin/refresh
  validation remains held. The stdlib inventory, versioned website guide/API
  contracts and approved execution sequence are unchanged.

- Managed inline Copy cleanup is now distinct from Move capability in shared
  type and native lifecycle metadata. Typed MIR distinguishes borrowed, copied
  and owned-handoff claims through nullable values, calls, interface adapters,
  pattern bindings, grouped declarations and replacement cleanup. Consuming
  captures retire their environment claim; repeatable captures remain borrowed.
  Native class-property cleanup uses the shared typed value cleanup helper.
  PHP uses checked Move/Copy facts and separates return ownership from payload
  cleanup, preserving generic collection returns and the caller's Copy values.
  Six MIR safety tests pass. The interpreter and native runtime now share the
  `isize::MAX` claim-count ceiling: exhaustion preserves the original claim
  and reports source-located P1503 without running queued cleanup. Typed panic
  propagation remains distinct from malformed-state interpreter failure.
  All 281 compiler library tests passed at the implementation checkpoint;
  the final 11 interpreter tests passed again after assertion cleanup.
  Standalone/hosted runtime suites pass 574/575 tests, strict compiler/runtime
  library checks pass with LLVM, and Linux/Windows runtime compilation passes.
  The earlier 269 focused
  Copy/consuming/inheritance/property-hook/PHP regressions passed on macOS, including
  interpreter, Cranelift, LLVM and PHP execution. Earlier native capture stress
  reported zero leaks over 1,000 iterations with restricted macOS inspection;
  this update does not claim a new leak measurement or Linux/Windows execution.
  Public Source/Token declarations and PHP managed-token integration were absent
  at this earlier checkpoint; their newer execution checkpoint is recorded above.
  Full cancellation integration remains unfinished. No public cancellation
  surface, pin or installed tool changed in that earlier beat; native runtime
  ABI remains 11 and
  Stage 36a is not delivered. Performance impact: existing retains check the
  count limit without new state allocation, native signals or per-copy boxing;
  the interpreter handles nested typed panics at its execution boundary.
  No new wall-clock benchmark result is claimed.
  Invalidated elsewhere: compiled compiler/runtime/editor artifacts need
  coordinated refresh when authorized; existing stdlib inventory and versioned
  website guide/API/editor language contracts are unchanged.

- Stage 36a's shared Copy storage correction preserves full inline and nullable
  carriers, exact aggregate size/alignment, and nested managed claims through
  fills and preserving collection conversions. Owning insertions require a
  copied or transferred claim in MIR. Nullable enum presence tests read the
  original nullable storage without assuming presence before the branch.
  PHP public iteration now consumes checked protocol callable plans for
  receiver homes and returned borrows; captured Copy loop values use distinct
  per-iteration storage. All 195 affected regressions, 277 compiler library
  tests, and 574/575 standalone/hosted runtime tests pass on macOS. The two
  aggregate-storage runtime tests report zero leaks with restricted macOS
  memory inspection; this is not native public-token leak validation.
  Linux/Windows checks were compile-only. Public cancellation and PHP managed-token
  execution were absent at this earlier storage checkpoint; the newer execution
  checkpoint is recorded above. Complete Stage 36a delivery remains missing.
  Performance impact: aggregate copying is proportional to slot count and
  complete carrier size; destination claims are retained per slot without
  boxing or per-copy shared-state allocation. Word/string fast paths remain;
  no new wall-clock benchmark result is claimed.
  Invalidated elsewhere: private runtime ABI identity 11 requires coordinated
  compiler/runtime/editor artifact refresh when authorized. Versioned website
  guide/API fill descriptions must match the existing checked Copy contract;
  the stdlib inventory already does. Installed tools, pins and publishing remain
  unchanged and held; the approved execution sequence is unchanged.

- On 2026-10-11 Andrew approved nullable completed-read regions. The shared
  native and adapter completed-read/resubmission bridges validate the original
  plan's directional operation key before access or mutation. Read access
  exposes only a retired read's known prefix. Pending or unknown
  progress returns null; known zero-byte completion returns a present empty
  region. Resubmission restores unavailability. Lookup allocates nothing,
  performs no I/O, and preserves observation, the original checked Error and
  group loans. Keyed resubmission preserves the operation identity while reusing
  an offset range; invalid/foreign keys or directions cannot mutate completed
  bytes, invoke adapters or take an owned Error. Existing index-based private
  bridges remain unchanged. Andrew approved readonly
  `getReadRegion(ReadTransferKey $key): ?ByteRegion throws IoError`, appended
  `IoTarget::TransferGroup`, and `resubmitRead` / `resubmitWrite`. Invalid access
  uses `Access`; invalid resubmission keys/ranges use `Configure`. Both use
  existing `IoError` / `InvalidInput`, with no path, system code or byte progress.
  Shared checked scalar bridges project both operations through the existing
  error transport and reject malformed status records. Signed resubmission
  ranges are checked before storage access or callbacks. Rejection preserves
  bytes, identities, callback counts and original owned errors. Native and
  complete-adapter controls now own the frozen wait budget inline; no group
  retains its caller's budget address, adds an allocation or restarts a deadline.
  Actual duplex-transfer fixtures outlive their setup budget, and control tests
  preserve immediate, indefinite, expired and finite budgets after the caller's
  storage changes. Andrew approved `Configure` / `TransferGroup` for `begin`'s
  own setup errors. Shared typed setup now feeds both unchanged raw bridges and
  checked scalar transport. Validation and native setup preserve failure
  details; adapter errors move unchanged with signed progress and once-only
  destruction. Failed setup consumes the plan, submits no work and restores
  the set's active gate. Canonical `TransferGroup` and `ReadinessSet->begin`
  now lower to typed MIR with exclusive set and inherited plan cleanup loans.
  Fourteen focused tests pass, including setup/cleanup and original adapter error
  preservation in the interpreter and actual macOS Cranelift/LLVM executables.
  Interpreter callbacks use ordinary MIR dispatch, fresh readiness getters and
  selected-source pins; virtual file transfers share the synchronous I/O core.
  The temporary frontend `B3603` rejection of request and pending-phase calls
  has been removed; pending source/storage safety checks remain mandatory.
  Fresh source resolution now follows setup preflight, preparation and nested
  file configuration, matching native callback boundaries. Fresh constructor
  receivers forward taken carriers' inherited cleanup loans through owning
  properties instead of requiring independent source storage. Source and
  independent MIR checking cover the same rule; ordinary, promoted, nested,
  inherited and generic iterator owners execute with exact cleanup in the
  interpreter, Cranelift, LLVM and PHP. This removes the former blanket rejection
  of fresh owning wrappers, not the lifetime checks on their retained sources.
  Generic callbacks now preserve owned source loans and storage partitions
  without inventing loans for plain specializations. Declared signatures and
  inferred bodies share concrete source-lifetime specialization. Complete source
  graphs participate in discovery/refinement, so declaration order cannot change
  callback contracts. Selector tracking augments that provenance rather than
  replacing it; independent MIR checking follows the same consumed-parameter
  contract. Focused execution and source-safety
  tests pass. Before completed-read access, the broader closure, trait and
  ownership sweep passed all 521 tests across 17 suites. PHP borrowed direct
  places now preserve their nonowning
  receiver home; virtual receiver-home transport follows the concrete checked
  override family, including property accessors in the shared hierarchy table.
  Both previously failing property-hook regressions passed in that sweep. Observed-key
  getters project the existing group identity through typed MIR, the interpreter
  and both native backends without acquiring an owner or loan. Readonly
  `getReadRegion` now lowers through typed MIR with a retained LastUse receiver
  loan. Pending-buffer protection, foreign-key errors, helper/alias loans,
  conflicting mutation and owner escape are covered. Returned intrinsic
  descriptors survive the callable boundary; callbacks retain view loans
  separately from owning-container contents. All 186 selected regressions
  across seven suites pass, including all 21 group tests and checked nullable
  writable callbacks in the interpreter, Cranelift, LLVM and PHP. Strict
  LLVM-feature compiler checks pass; these are macOS checks, not Linux/Windows
  execution. Public completed-prefix execution remains untested without
  completion fetching. Writable `resubmitRead` / `resubmitWrite` now use typed
  MIR and the existing checked native bridge. Independent checking enforces
  exact directional keys, int ranges, writable receivers and region-loan
  conflicts. Interpreter reuse retains the original buffer and checked errors;
  selected-range conflicts inspect every active operation, not just earlier
  plan positions. The extended seven-suite sweep passes 190 regressions,
  including 25 group tests covering rejected reuse without observation or
  submission. Final-source validation passes those 25 group tests, all 44
  stream-capability regressions, the original-buffer range unit and strict
  LLVM-feature compiler checks. The file-creation matrix passes; a process
  sample identifies shared borrow-origin analysis as its slow phase, not I/O.
  Both borrow-proof validators now share a reverse-postorder worklist without
  changing joins, loop widening or final validation. Three reference-worklist
  tests cover diamonds, cycles, checked branches and malformed targets. The
  pre-nullable-callback checkpoint passes 555 macOS regressions across 16 suites,
  all 255 library and 31 semantic tests, strict compiler Clippy, and all 420 durable
  interpreter/Cranelift/LLVM examples. Subsequent nullable/fresh-result regressions
  exposed exact-retention checking that rejected valid producers. Andrew approved
  owned-result retention as a permitted upper bound on 2026-10-12. Source checking
  and independent MIR now share that rule, using proved executable origins for
  actual returned loans and stored contents. All 318 focused macOS regressions
  pass, including 32 callback semantics, 118 independent MIR, 76 linked closure
  execution and 32 writeback tests. Interpreter/Cranelift/LLVM/PHP exercise nullable
  forwarding, loan-free results and exactly-once cleanup; erased loans and borrowed
  alias relabeling remain rejected. Strict LLVM-feature compiler checks pass.
  The earlier source recheck passed all 34 ownership and 22 Time tests.
  A further 65 regressions pass across pending-storage source/MIR proofs,
  nullable defaults, typed registration keys and native Error-progress metadata.
  Separate interpreter/Cranelift/LLVM/PHP
  scalar-capture tests verify lifetime-safe collection storage and reject
  escaping captures. Three older tests no longer require blanket rejection of
  lifetime-safe storage; their longer-lived-destination safety checks remain.
  All 34 public transfer-group regressions now pass on the interpreter, Cranelift
  and LLVM on macOS, covering fetching, completed-prefix access, reuse and owned
  error cleanup. The compiler library passes all 257 tests, including retained
  completion after relative-wait clock failure. Chained boolean operands release
  completed calls while preserving enclosing argument loans; all 87 Stage 21 and
  119 independent MIR regressions pass. Nullable Copy homes remain independent
  after writable clearing, and stale presence proofs are rejected.
  Andrew approved wait-infrastructure failure first on 2026-10-12, preserving
  retained completions/original transfer errors for subsequent fetches. The shared
  core and both raw transports now report a wait failure once before taking queued
  outcomes. Checked scalar observation now uses that same complete-adapter path,
  preserving original directional keys, owned Errors and every explicit outcome.
  Independent control facts remain in private transport; their public projection
  is unfinished. One call-owned wait budget leaves active transfers
  available after fetch expiry. Seven added tests cover mixed erased dispatch,
  original Error identity/cleanup, completed prefixes, absent outcomes and reuse;
  successful fetches allocate nothing. Immediately completed requests retire at
  submission: their known prefixes are available before fetching without another
  callback, and queued original Errors drop once when fetched or during teardown.
  The interpreter no longer accepts null erasing an actual native/immediate
  outcome. Pending completion facts now live inline in the stable request record,
  preserving the four-word borrowed view without retaining callback-stack output
  locations. Preparation/completion replies decode independently of C callback
  dispatch, preserving original Error carriers and actual-outcome presence. Four
  callback-independent reply regressions allocate nothing. Native startup now
  drives one shared resumable guard: source resolution precedes preparation,
  and all preparation precedes submission. Yielded preparations retain stable
  request/buffer loans but no call frame; cancellation aborts unfinished staging
  and cleans successful preparations once. Four handoff regressions allocate
  nothing. The shared pending driver now separates native retirement from
  complete-adapter dispatch and reply acceptance. Delayed replies retain caller
  and selected storage, source pins and original Error ownership; repeated
  inspection never requeries or redrains a retired request. Four added handoff
  regressions allocate nothing, including unsubmitted cleanup and retained
  selected-storage exclusivity. Observation, resubmission and stop/drain now use
  shared resumable guards: host handoffs retain request/buffer loans without a
  saved call frame, consume returned replies once, and preserve the first stop
  and cancellation failures. All cancellations precede drainage. Six additional
  handoff regressions allocate nothing, covering wait-only expiry, failure-first
  observation, startup cleanup and stable-key/range reuse. Preparation validation
  now also accepts ordinary replies independently of native callback dispatch;
  three allocation-free regressions preserve source/storage loans through
  cleanup and distinguish successful preparation from a throwing preparation.
  Hosted Error transport retains the real PHP payload behind a move-only core
  claim, with no fabricated native vtable or PHP callback through C. Four
  allocation-free runtime regressions cover return, deferred disposal, slot reuse
  and original cleanup-error precedence. PHP execution preserves payload identity,
  descriptor, authored origin and signed progress; actual Error destruction runs
  once after native return. The hosted controller now runs actual PHP groups
  through the same shared core, without native-to-PHP callbacks or a second
  execution engine. Immediate completion retires during submission/reuse;
  reply status stays distinct from every actual outcome tag. New fetches clear
  observed keys before freezing their wait, including failed wait construction.
  Six allocation-free controller regressions cover real file transfers,
  startup cleanup, original Error claims, immediate outcomes and stable reuse.
  Cancellation handoffs preserve returned preparation outcomes and retired
  native bytes, complete once, and retain state after source/token release.
  The private Copy-token retain bridge acquires owned claims without allocation
  and reports count exhaustion at the authored copy site through P1503.
  All 19 cancellation runtime tests pass macOS leak checking with zero leaked bytes.
  All 12 PHP stream unit regressions pass; the former B3603 group boundary is removed.
  Standalone/hosted macOS suites pass 567/568
  tests, with strict checks of both configurations on macOS and Linux/Windows
  cross-targets; strict LLVM-feature compiler checks pass for all Cargo targets
  on macOS. All 34 transfer-group and 15 pending-contract compiler regressions
  pass, with PHP joining interpreter/Cranelift/LLVM group parity. All 44 stream
  capability cases pass, including the affected case rerun after correcting its
  stale plain-text-only PHP output assertion. Native linking retains the libunwind re-export warning.
  All 223 runtime transfer-group tests pass macOS leak checking with zero leaked
  bytes, retaining the tool's restricted-memory-content notice. Cross-checks are
  not Linux/Windows execution; public cancellation/control integration and the
  full Stage 36a gate remain unfinished. PHP dependency selection now follows shared
  runtime-payload classification and declared storage, including library-only
  ownership cleanup and nested containers/enum payloads. All 24 transfer-plan
  regressions pass on macOS; scalar and opaque-key libraries remain runtime-free.
  Website versioned guide/API source checks also
  require observation of the previous result before reuse, preserving unread
  results, distinguishing retired bytes from a fetched result. Matching Tooling
  hover/test-source updates were inspected; Website and Tooling behavioral tests
  remain unrun, with pins, installed tools and publishing held.
  Before these additive runtime bridges, compiler checkpoint
  `dev-ee6289d9d327565f418d1cd10de586e6dcdd21bf6262807b6ccff9524062b6fb`
  passed 626 selected tests across 13 suites, including all 420 durable
  interpreter/Cranelift/LLVM examples and the nullable enum, complete
  Error/interface carrier, returned-loan and closure regressions. Its strict
  workspace-wide LLVM-feature checks pass. Runtime checkpoint
  `dev-d4426304d80f9bc2f0e3294664e7c71ae4eecbd349b38145d2ec6e1f9302d20f`
  passes 512 standalone and 513 hosted macOS tests, plus strict checks of both
  configurations on macOS and the Linux/Windows cross-targets. Those checks
  are not Linux/Windows execution. Leak probes were not rerun at that checkpoint;
  controlled timing remains unverified. The additive group target passes 136
  selected compiler tests; its error fields execute in the interpreter,
  Cranelift, LLVM and PHP. The extended group bridge passes 513 standalone and
  514 hosted macOS runtime tests. These component checks do not establish public
  group execution or a completed Stage 36a validation. The checked setup, scalar
  and inline-budget bridges pass 517 standalone and 518 hosted macOS runtime tests,
  plus strict standalone/hosted checks on macOS and the Linux/Windows cross-targets.
  The 57 focused adapter transport tests also pass under macOS leak checking,
  with zero leaked bytes. Fresh compiler regressions pass all 70 readiness
  tests and 23 transfer-plan tests, including their selected interpreter,
  Cranelift, LLVM and PHP checks. This does not establish public group execution,
  Linux/Windows execution or
  native-backend leak coverage. Website versioned guide,
  API reference and exhaustive example updates, and language-server description
  and test-source updates, were independently inspected. Their behavior tests
  remain unrun. Andrew approved ongoing local documentation/test synchronization
  for each contract he approves; the website continues to represent finished
  target state, not implementation lag. Andrew approved inline Copy
  `Doria\Std\Concurrency\CancellationToken`: owned copies retain/release the
  shared state without allocation; borrowed arguments acquire no ownership claim.
  Public token declarations and compiler/backend cleanup integration were absent
  at that approval checkpoint; their newer execution checkpoint is recorded above.
  The approved Concurrency-owned `CancellationError` now executes through ordinary
  checked-error transport in interpreter, Cranelift, LLVM and PHP. Its four
  contract/IDE regressions and 45 checked-error regressions pass on macOS.
  Pending owned Copy results now keep cleanup independently of Move capability;
  synthetic handoffs, nested nullable enum construction, branch yields and
  finalizer replacement pass 171 enum/match/MIR regressions and strict
  LLVM-feature compiler checks. Both native profiles complete 2,000 cleanup
  stress iterations with zero leaked bytes, including coalesced results.
  These checks do not establish public source/token execution or Linux/Windows
  execution. Matching Website guide/API pages and prepared Tooling fixtures were
  inspected; sibling behavior validation remains held.
  Andrew approved local UAT restoration using the installed development compiler.
  The website's explicit prebuilt selection now restores both Playground and
  Tutorial: their native Hello programs pass in the actual browser workspaces,
  and the service independently returns status 0, exact output and no diagnostics.
  Production pins and installed tools are unchanged; this is not Stage 36a closure.
  Sibling builds, production pins, installed tools, staging and publishing
  remain held.

- On 2026-10-10 Andrew approved `getStream` / `getWritableStream` and existing
  `IoError` for transfer-plan configuration with appended `IoOperation::Configure`
  and `IoTarget::TransferPlan`, then appended `IoOperation::Access` for failed
  registration lookup. Borrowed lookup now executes through the original member
  storage without transferring, copying, or dropping its owner. The
  implementation preserves approved writable borrowed locals' permission,
  nonownership, and original storage homes through nullable Copy arguments,
  nested captures, and dynamic object replacements. Source-following registration
  and selected-source retention proofs preserve the approved lifetime contract;
  public group execution remains incomplete. Shared checked read-plan construction
  and consuming append forward all region loans without I/O; descriptor/status
  layouts stay runtime-owned and failures use existing IoError fields. Removed
  unused duplicate borrow-state bookkeeping from the call-origin proof;
  independent function-local loan validation remains mandatory. Dependency-driven
  proof scheduling now
  revisits changed contexts and their callers, refreshes relocated closure and
  definition bindings, and still checks every context after convergence. Tests
  compare its facts and final checks with exhaustive scheduling; no iteration
  cap or validation exemption was added. The source checkpoint
  `dev-67aa61ff5a754d6f4d7b712dd29af65a3a34024796067bc26c31ad3bf160901c`
  passes 797 selected compiler tests across 17 suites, including all 419 durable
  interpreter/Cranelift/LLVM examples, both object suites, independent MIR,
  writable-borrow writeback, returned-loan, and selected-storage regressions.
  Strict workspace-wide LLVM-feature checks pass. The same checkpoint passes
  508 standalone and 509 hosted macOS runtime tests, plus standalone/hosted Linux
  and Windows cross-target compile checks. Cross-compilation is not execution;
  Linux/Windows execution of this checkpoint and controlled timing remain
  unverified. Earlier macOS leak probes of the shared ABI reported zero leaks
  under both native profiles; they were not rerun at this checkpoint. Tooling
  behavior validation and website example execution remain pending; guide and API
  source contracts are synchronized. Remaining public spellings and group
  execution remain Stage 36a work. Production pins, publishing holds, and
  installed tools are unchanged. Aggregate secondary caches remain below
  Andrew's 25 decimal GB ceiling.

- Andrew approved opaque inline Copy `RegistrationKey<T>` and
  `register` / `unregister`, followed by `new ReadinessSet()` on 2026-10-09.
  Empty construction retains checked setup failures and lazy native association.
  Compiler-owned constructor and generic registration declarations now preserve
  nominal readiness conformance, take ownership, exact key types and checked
  effects. Empty construction now uses shared checked-I/O lowering and the
  runtime-owned opaque payload, with one owner allocation and one deterministic
  release. At source checkpoint
  `dev-9b1a86759c2600f8096779ca168d62cd4ccf3399083fe72263611ac1aa974d7f`,
  all 211 selected compiler tests pass on macOS, including all 416 durable
  interpreter/Cranelift/LLVM examples, readiness/file/key construction and cleanup,
  forged-MIR rejection, and structured PHP FFI dependency failure. Strict compiler
  checks pass in both feature graphs. Runtime validation passes all six checked
  membership regressions, 454 standalone tests and 455 hosted tests. Strict runtime checks
  pass on macOS and the Linux/Windows cross-targets; those are source checks,
  not Linux/Windows execution. Earlier macOS construction leak checks reported
  zero leaks under both native profiles; Linux leak coverage is wired but
  unexecuted at this checkpoint. Shared runtime class owners now use the
  established fatal P1601 allocator, removing their previous catchable owner-heap
  allocation failure; table, identity and native-resource failures remain checked.
  Registration and removal now use ordinary frontend checking, shared validated
  MIR, and interpreter/Cranelift/LLVM execution. The blanket frontend `E0775`
  execution fence is removed without bypassing ownership or nominal capability
  checks. PHP membership now retains original complete PHP values while the
  shared runtime borrows their typed readiness metadata. The `B3604` boundary
  is removed. Native and external admission share identities and checked
  rejection; native metadata retires before reverse-acquisition value cleanup.
  The focused macOS checkpoint passes 100 runtime readiness tests and 15
  compiler readiness checks, including exact interpreter/Cranelift/LLVM/PHP
  round trips, full-width keys, retained cursor loans, getter failure, and
  escaping-error cleanup. MIR, independent validation, LLVM structure, and
  readiness contracts pass 148, 115, 21, and 64 tests respectively.
  Readiness-only registration is not
  restricted to pending-transfer streams.
  Exact generic identity now follows
  shared Copy, constant, nullable, capture, and validated MIR transport. MIR
  preserves structured specialization arguments, including nested collections and
  iterator conformance, without interpreting generated type names. Static
  payload-enum reads use existing places; nullable named constants use the
  ordinary checked resolver. Wide object fields and static enum strings retain
  their required alignment without extra object allocations. Shared enum layouts
  preserve nullable wide and scalar fields; MIR rejects undersized or inconsistent
  storage. Aggregate static replacement and nullable string copies use ordinary
  copy/drop paths; nullable string destinations reuse checked string lowering.
  Sparse runtime owners preserve reverse acquisition cleanup through slot reuse
  and table growth without moving live values or adding allocations.
  Private adapter-set admission now retains complete original interface carriers;
  removal returns both words, and native metadata retires before remaining values
  drop. Its checked membership bridge reuses canonical readiness errors and
  ordinary take-on-failure cleanup. Borrowed fixture admission keeps its existing
  nonowning contract.
  Waiting, public group execution, full work-unit validation, installed refresh,
  and publication remain pending.

- Readiness ownership checking now carries stored source loans through owned
  sets, enclosing objects, and take-and-return helpers. Returned sets preserve
  member partitions through nested helpers and factories; independent MIR
  checking proves returned storage from executable bodies and rejects forged
  input origins, member identities, sources, and repetition claims. Interface
  payload conversions, nullable wrappers, and mixed boxing preserve storage
  metadata and source loans. Source and independent MIR joins preserve
  unpartitioned incoming loans when a known member is removed, including branch
  replacement by helper-returned sets. Checked removal helpers preserve exact
  returned-member sources, including newly registered members and source-free
  input sets. Optional, failed, and virtual removals discharge only proved
  obligations; independent MIR rejects forged normal and exceptional effects.
  Checked callback forwarding preserves stored sources and individual member
  partitions across arrow, block, and joined choices; ordinary borrowed-value
  returns stay distinct. General expression, capture, and collection extraction
  proofs remain incomplete. Collection reads and removals retain candidate source
  dependencies through nested containers and nullable callables. Definite whole-
  collection clears retire only the cleared storage; precise selected-element
  retirement remains incomplete. Nominal iterator requirements survive pruning
  of unreachable MIR methods. Owned carriers stay distinct from borrowed values,
  while callback and region return contracts preserve their source relationships.
  Source and MIR keep nonowning Copy key identities separate from loans,
  distinguish successful and exceptional storage effects, and invalidate prior
  result-slot provenance.
  Call effects follow evaluation order across return types and conditional
  expressions; optional paths cannot claim definite removals. Empty ownership
  records are no longer emitted; independent MIR still rejects omitted required
  contracts, forged stored sources, and returned keys.
  Resource reads, transfers, storage accesses, and temporary loans now follow
  that same ordered traversal. Nested calls retire argument-only loans while
  preserving returned loans; independent ownership checks resolve stored-member
  obligations before a call can remove their source membership.
  Generic and grouped nullable returns preserve presence through shared type
  resolution; PHP hints name the actual ordered-collection carriers. Nullable
  nested-collection and mixed access preserve type, presence, and cleanup:
  removals return owned values, while peeks retain source loans. PHP uses shared
  ingestion and borrowed-element classification, including inferred nullable
  mixed bindings. Native and PHP presence/type tests evaluate general expressions
  once. Erroneous writable access during a live deque-element loan is now
  rejected with `E0477`.
  Native calls now forward original storage homes for borrowed Move parameters,
  including void-returning helpers, interface dispatch, and indirect callbacks.
  Runtime-owned entry arguments use the same transport; the private runtime ABI
  epoch is now 7. Generated interface adapters preserve the source call's stored
  owner identity, and closure bodies use ordinary functions' normal-exit storage
  summary. Independent MIR rejects erased normal/exceptional store contracts and
  collection mutation origins. Stream callback bridges transport the receiver's
  original home and invocation-scoped request/byte-region carriers without heap
  allocation.
  The latest focused macOS checkpoint passes 353 integration tests: 113 independent
  MIR-validation, 64 readiness-contract, 67 returned-value/closure, 11 entry-point,
  52 Stage 35 contract, and 46 iterator-runtime tests. This includes caller-owned
  callback storage through free, concrete, interface, and indirect calls, plus
  borrowed entry arguments under both native profiles. Both native stream-callback
  bridge unit regressions and the pending-file readiness round trip pass. The
  broader native-ABI checkpoint passes all 232 compiler-library units, 21
  byte-region execution, four byte-region structural, six byte-region transport,
  45 checked-error, and 27 consuming-receiver tests. Subsequent checks pass all
  20 borrow-return and 40 stream-capability tests.
  Closure control-flow contexts no longer inherit
  enclosing `when` returns or finalizer boundaries; focused execution agrees
  across the interpreter, Cranelift, LLVM, and PHP. The preceding focused checks pass all 234
  compiler-library units, 121 PHP tests, 21 indexed-access parity tests, and 64
  readiness-contract tests. Public membership unit tests now use ordinary source
  lowering, not a frontend-fence bypass. PHP diagnostics use compiler-owned
  authored callable identities and the canonical human presentation without a
  separate raw `Facts` section; typed V2 facts remain intact. No expected-output
  normalization is used. Authored/generated declaration provenance now fixes all
  33 synthetic standard-library source-inventory failures; the library run passes
  all 236 tests. Subsequent focused checks pass all 13 List-algorithm tests and
  all 114 independent MIR-validation tests. List algorithms use the ordinary
  indirect-call lowering, including semantically proved-impossible checked edges
  and the existing partial-result cleanup checks. Shared MIR explicitly records
  authored, forwarded-dispatch, and caller-owned collection diagnostic frames.
  Native adapters forward the original frame without allocating another one;
  interpreter execution retains adapter storage but omits its transport frame.
  Direct and indirect calls restore their exact source site after argument
  evaluation, including constructors and null-safe calls. Shared MIR
  callable-name selection uses existing closure and nominal method identities;
  authored frames retain whole return/throw sites through cleanup, and LLVM
  owner drop glue forwards the active frame. The diagnostic CLI checkpoint
  passes all 12 tests, with 237 compiler-library tests passing before the PHP
  membership bridge. The broader PHP suite passes 99 of 122 tests; Debug owns
  its emitted-shape assertions, constant-default emission regression, and one
  ownership-invalid fixture. It is not a green whole-PHP checkpoint.
  Strict LLVM-enabled compiler Clippy now passes for all targets after the native
  ABI correction. Shared native receiver-home lowering uses explicit matches;
  CLI tests reuse the existing native runner's execution-directory owner instead
  of loading its implementation twice. Diff checks and the documentation,
  runtime-diagnostic architecture, and build-artifact policy guards pass.
  The preceding native checkpoint passed all 416 durable
  interpreter/Cranelift/LLVM examples; it predates the final nullable-expression,
  ownership, and callback fixes. An intervening full native run, covering 417 durable
  examples including public membership, was stopped after a confirmed closure
  failure; it is not a passing checkpoint. The isolated failure was borrowed
  function narrowing from `mixed` losing its original storage home. Shared exact
  payload-place discovery now covers borrowed mixed functions, classes and Move
  enums, with physical class/interface descriptor identities preserved in both
  native profiles. Interpreter projections retain the same caller-owned storage.
  The focused mixed-function/class/interface/enum escaping-closure regression
  passes on the interpreter, Cranelift and LLVM. The existing native closure
  matrix also passes, as do all seven private closure-ABI unit checks, including
  exact borrowed mixed payload homes and owned-transfer classification. The fresh
  macOS native checkpoint now passes all 128 selected integration tests: 29
  Cranelift structure, 16 native parity (including all 417 durable examples), 15
  registration-key, and 68 returned-value/closure transport tests. This checkpoint
  includes the membership and borrowed mixed-payload fixes; it does not verify
  Debug's subsequent PHP constant-default and assertion corrections. A subsequent
  macOS checkpoint passes 240 compiler-library, 24 CLI, 124 PHP, 64 readiness-
  contract, and 15 registration-key tests. Registered-owner cleanup now forwards
  its current Doria caller frame through the shared runtime payload-drop ABI,
  never a retained acquisition frame. Native metadata retires before complete
  values drop in reverse acquisition order. Failed storage staging retains the
  taken value for cleanup in that same current frame. Fallthrough, explicit
  return, and escaping checked-error cleanup preserve exact diagnostics across
  interpreter, Cranelift, LLVM, and PHP. After the receiver/test-helper lint
  corrections, all 12 readiness-library, 12 diagnostic-transport, and 68 returned-
  value/closure transport tests pass.
  Runtime tests pass 462 standalone and 463 hosted checks; strict checks pass
  for both profiles on macOS and standalone Linux/Windows runtime/test sources.
  Both macOS native profiles preserve the membership fixture's exact output,
  status, and file state; `leaks --atExit` reports zero leaks for each, with the
  tool's restricted-memory-content notice retained. Linux CI now selects that
  same membership fixture for both native ownership leak checks; it has not run
  at this checkpoint. Cross-target checks do not establish Linux/Windows execution.
  Private group teardown now forwards the current cleanup frame to pending
  completion callbacks and untaken checked-error destruction through the shared
  core, with no retained acquisition frame or added allocation. Setup-failure
  cleanup uses its live caller frame; successful teardown clears that frame
  before releasing the driver. Both runtime profiles and strict macOS and
  standalone Linux/Windows source checks pass after this correction.
  Public group execution, current Linux/Windows execution,
  fresh source-matched tooling, installed
  refresh, and publication remain pending; this does not complete Stage 36a.

- Andrew's 2026-10-08 ClockError ruling now has an ordinary compiler-owned Time
  declaration, separate `StandardTime` identity, and shared standalone/IDE and
  package-graph injection. It keeps readonly message and optional system code;
  its imports, effects, ownership, and rethrow use the existing Error machinery.
  Six focused regressions pass, including interpreter/Cranelift/LLVM/PHP
  transport. The same checkpoint removes the I/O-class-only nullable integer
  default exception: ordinary nullable Copy integers, floats, bools, and enums
  share constant checking and caller-side materialization. All scalar widths
  pass interpreter/native checks; PHP passes supported widths and retains its
  explicit Float32 and canonical float-display boundaries. All 71 tests across
  five LLVM-enabled suites and strict compiler-library/selected-test Clippy pass
  on macOS at compiled source checkpoint
  `dev-14908b8a439e3e44d383bd3789eb74267eeb7138e28555398ca9146d374c7460`,
  including all 413 durable interpreter/Cranelift/LLVM examples. Compiler-matched
  local tooling passes 381 Rust and 61 VS Code tests, strict Clippy, build, and
  shared highlighting. Constructor help now uses compiler call/member facts
  for source-authored and compiler-owned constructors; production pins and
  installed tools were unchanged at that checkpoint. IntelliJ host tests were
  not run. Website guide/API checks pass four focused tests with 106 assertions. Current
  Linux/Windows compiler execution, native Time operations,
  and public group execution remain pending.

- Andrew approved distinct inline Copy Duration and Deadline values on
  2026-10-08. Eleven Time regressions pass on macOS at compiled source checkpoint
  `dev-8d394aef6a43880bf419f9ceddeaac8b1b5670f6301d7921a3622f1399b0283c`,
  covering interpreter/Cranelift/LLVM transport through ordinary and generic
  calls, nullable and static defaults, properties, collections, enums, mixed
  narrowing, and escaping captures. Three existing nullable-default regressions
  also pass. Shared MIR validation rejects all heap and shared-reference carriers
  for these inline values without narrowing writable Copy shared syntax.
  Canonical macOS validation with LLVM passes: 4,458 test executions, none ignored,
  both runtime profiles, strict Clippy, and all 413 durable examples across the
  interpreter, Cranelift, and LLVM. Pipe-termination fixtures now wait for actual
  reader closure instead of treating a parent acknowledgement as proof of closure.
  Coordinated local tooling passes 383 Rust and 61 VS Code tests and strict Clippy;
  focused website guide/API checks pass two tests with 78 assertions. Installed
  identities are verified separately by the refresh workflow. Current
  Linux/Windows compiler execution remains pending. Andrew subsequently selected
  signed `int64` nanoseconds for Duration (`B then`). Fifteen focused Time tests
  now pass on macOS, including exact factory/getter values across the interpreter,
  Cranelift, LLVM, and PHP with FFI disabled. Shared nullable comparison and PHP
  Copy-storage classification preserve equality and generic/captured copies;
  structural tests keep the scalar ABI and exclude heap lifecycle. Canonical
  macOS validation with LLVM passes 4,466 test executions, none ignored, and all
  414 durable examples at compiled source checkpoint
  `dev-4eeca2c4b6e5082a5413ef12196cf4bab41e0bb0b8ad994dd328077c64c9e6cf`.
  Coordinated local tooling passes 386 Rust and 61 VS Code tests, strict Clippy,
  build, and shared highlighting. Website content checks pass 212 tests; two
  production-pin assertions await coordinated publication. Installed identities
  are verified separately by the refresh workflow. Andrew approved negative
  intervals as already expired for timed waits and relative deadlines (`1: A`).
  The private shared wait-budget transport now distinguishes this expiry from
  immediate polling and indefinite waiting without changing the Duration value.
  This checkpoint passes 390 standalone and 391 hosted runtime tests on macOS,
  including allocation-free negative expiry and expiry-then-resume regressions.
  Strict runtime Clippy passes on macOS and Linux/Windows cross-targets; the
  latter are source checks, not platform execution. All 15 existing Time
  compiler/backend tests pass with LLVM enabled and PHP FFI disabled.
  Andrew's subsequent ordinary Copy `Wait` and exact past Deadline rulings now
  pass 18 Time tests across interpreter/Cranelift/LLVM/PHP on macOS. Concrete
  callable signatures are interned after Copy specialization; PHP indirect
  calls and parameter cells use those same MIR contracts. LLVM wide indexed
  removals use the existing aggregate-storage path. Canonical validation with
  LLVM completed 4,504 passing test executions, none failed or ignored, both
  runtime profiles (401 standalone and 402 hosted), strict Clippy, and all 414
  durable examples at compiled source checkpoint
  `dev-a5751d178cf9304972dd11d760d3567d6a094d04001266186986375d8ce03d98`.
  The runner exited 1 only after its 15.04 GiB target exceeded the separate
  15 GiB cache cap and was reclaimed; this is not an overall validator pass.
  The manual PHP initializer-callback smoke and its persisted regression passed
  with FFI disabled.
  Andrew subsequently approved `Deadline::fromNow(Duration $duration): Deadline
  throws ClockError`. It now samples one shared monotonic clock and preserves
  exact signed intervals and ordinary checked cleanup across all four backends.
  Canonical macOS validation with LLVM passed 4,526 test executions in 136
  suites, none failed or ignored, both runtime profiles (404 standalone and
  405 hosted), and all 415 durable examples at compiled source checkpoint
  `dev-40e226ebd8daecab41497b53bc35018a1471fe9d0ade6a6318624f39f372b5f0`.
  All 21 Time tests and injected clock-failure regressions pass. The managed
  target finished at 11.26 GiB. Coordinated local tooling passes 390 Rust and
  61 VS Code tests, strict Clippy, build, formatting, and shared highlighting
  against the same implementation at documentation fingerprint
  `dev-a20b4000cfba70f0fe73df86b514ff87f286b223e23c11191c85c9ac5d15d7c2`.
  The local LSP reports that exact compiler identity; production pins and
  installed tools remain unchanged. Strict all-target Clippy for `doria-time`
  and both `doria-rt` profiles passes for Linux GNU and Windows MSVC at source
  checkpoint
  `dev-7754c74779cad54872edfb3a2e7567a8c20a5758d850aafec4307a87beb10876`.
  These are source checks, not platform execution. Current Linux/Windows
  execution and IntelliJ host tests remain unverified.
  Remaining public Time declarations, further arithmetic, and core collection
  contracts remain in the bounded appendix. Intermediate publication is on hold
  (`2: Wait`); local-source tooling builds remain the testing workflow. This is
  not stage delivery.

- Private operation identity is now separate from registration ownership across
  the group, adapters, and Unix/Windows drivers. The blanket repeated-registration
  rejection is removed; duplicate request slots, foreign/stale keys, and
  conflicting writable storage remain rejected. Shared-owner duplex and arbitrary
  operation-count regressions preserve distinct results, reuse, and cleanup.
  Andrew approved plan-order access for competing operations on 2026-10-09 (`A`).
  The shared scheduler now removes unordered competing access across native and
  complete-adapter paths; independent duplex directions may overlap, while
  shared-cursor operations compete. Queued cleanup, original errors/progress,
  retained buffers, and allocation-free reuse remain covered.
  At ordering checkpoint
  `dev-a9664af8980ccf2ab4b7ee21b05f5f58ed80ea060042ad21eaced5a2a65cfa9d`,
  macOS passes 114 selected compiler/archive-metadata tests, 424 standalone and
  425 hosted runtime tests, and six shared-time tests per profile. Formatting and
  strict runtime source checks pass for both profiles on macOS, Linux GNU, and
  Windows MSVC. Linux/Windows runtime execution and public group execution remain
  unverified; this checkpoint does not claim stage delivery.

- Andrew approved wait-only completion-fetch expiry on 2026-10-08. Decision
  0110, SPEC, the stdlib inventory, and the I/O audit distinguish its typed
  timeout observation from completion, cancellation, and a group deadline.
  Transfers and retained loans survive expiry. The shared runtime now separates
  fetch budgets from group control across the native and adapter transports,
  preserving repeat waits, request reuse, and original checked errors. The
  earlier wait-only checkpoint passed 363 standalone and 364 hosted runtime
  tests and 15 LLVM-enabled pending-request compiler tests on macOS.
  Website wait-only guides/API/tutorial contracts pass 123 content tests;
  tooling hover guidance uses the same contract. Public group execution,
  compiler-matched integration, installed refresh, and publishing remain pending.
  Andrew approved Time-owned `ClockError` for standalone clock-read failures
  on 2026-10-08; readiness wait failures retain `IoError`. Remaining Time
  declarations still need the bounded appendix before public execution.

- Private standard-device projections now preserve the canonical stdin queue
  and device access claim across intrinsics, views, and pending requests without
  an owned-file close obligation. Decision 0137's lazy, group-bounded native-only
  helpers now retain real handles, buffers, and driver-thread adapter callbacks.
  The unsafe temporary mode-changing path is removed: Unix aliases may share
  blocking flags, so inherited nonblocking mode is not a safety proof. Windows
  handles with unproved asynchronous mode fail explicitly. The frozen runtime
  passes 386 standalone and 387 hosted tests on macOS; strict Clippy passes for
  both profiles on macOS and Linux/Windows cross-target checks. Linux ARM64
  execution passes 387 standalone and 388 hosted tests at source checkpoint
  `dev-b955e956183c4768cb1a52728d0e9b8ef0a36436690a26a55e5bd1791d7430c8`.
  The exact cached Rust 1.98.1 image ran offline with read-only source/dependencies,
  one CPU and 1 GiB of memory, reusing the owned Linux cache. Both suites passed
  without failures or ignored tests. In-image Linux Clippy was unavailable;
  Windows execution and public standard-stream/helper integration remain pending.
  The test containers were removed and Docker Desktop was stopped. The
  source-matched supported compiler paths pass the checks recorded above.
  Fatal panic remains non-unwinding.

- The frozen standard-device runtime passes 376 standalone and 377 hosted
  runtime tests and strict Clippy on macOS. Linux ARM64 execution passes 377
  standalone and 378 hosted runtime tests at source checkpoint
  `dev-03603c54484b0a78c791884504af4a9da678f25b8e8545d44f0712e527178a4a`.
  These locked, offline tests reused the official Rust 1.98.1 image in bounded
  disposable containers; Linux Clippy was unavailable in that image. Docker
  Desktop was stopped afterwards. Windows runtime/test sources compile without
  execution. All 91 focused LLVM-enabled
  compiler checks pass across stream capabilities, pending contracts, selected
  storage semantics/MIR, and progress structure. The canonical local validator
  and tier-1 LLVM CI now select all five suites, with a guard checking both
  selections. All 16 native parity checks pass on macOS, including all 413
  durable interpreter/Cranelift/LLVM fixtures, native closures, stack-growth,
  and pipe/prompt behavior. This checkpoint does not claim whole-stage
  validation, public standard-stream execution, or delivery.

- Canonical default work-unit validation passes on macOS at source checkpoint
  `dev-614353aec2c26260924706eceac3a868f4d43b6712f51cbc97a08ff01fe9eada`.
  The CLI version-contract regression now checks the exact built identity and
  accepts both clean Git revisions and dirty development fingerprints, matching
  the existing language-server check. Installed refresh and public delivery
  remain pending.

- The benchmark harness now removes its own test scratch after success or
  failure without following symlinks. All 152 portable harness tests pass on
  macOS, including independent file-copy seed, length, hash, and short-tail
  checks. Required structural metrics and unavailable targets no longer vanish
  from baseline capture; incomplete comparisons no longer return success.
  Total and phase compiler durations share strict nonnegative-integer ingestion:
  missing or malformed evidence stays unavailable, never a coerced measured
  zero. Genuine zero and explicit producer-unavailable reasons remain intact.
  Compiler byte-region and stream guards share native reference traversal,
  including recursive forwarding helpers and explicit copy/allocation probes.
  Checked file failure paths may construct ordinary Error objects; runtime
  probes cover their ordinary transfers separately. Four region and 40 stream
  checks pass after the guard correction; the unchanged file-creation matrix
  passed earlier in the same round. These static references are not runtime
  allocation or call counts. Peer records distinguish failure status and
  diagnostics. The runner's explicit existing `--build-root` now routes generated
  sources, native/peer outputs, plans, reports, provenance, and .NET intermediates
  together without fallback or automatic cleanup. At compiler source checkpoint
  `dev-c1a8ec4c78e267b339e43e342fcb5031af0eb06849646cff8bf7c56f82303173`,
  macOS smoke verification passes streaming file copy (1,048,589 bytes, including
  a 13-byte tail) and 1,024 repeated 64-byte writes through Cranelift, LLVM, C,
  and Rust. Both cases also pass `--no-build` reuse with unchanged artifact
  hashes and exact input/output lengths, hashes, stdout, stderr, and status.
  Runtime fixture cleanup succeeds. All generated files use secondary scratch;
  disposable binaries are removed after verification. This is correctness
  evidence, not controlled timing, baseline promotion, dynamic allocation/copy
  counts, current Linux/Windows execution, or whole-stage completion.
  At subsequent compiler checkpoint
  `dev-5a4282727f6a7068ad15ebe4de3233ca4319da01a904d6a7acdf8ad3bd0b7b85`,
  writable-stream dispatch and the two-adapter chain pass macOS smoke
  verification for both concrete and erased variants on Cranelift and LLVM.
  All eight variants preserve exact output and file hashes. Shared MIR metrics
  record one interface-call site for each erased variant and none for concrete
  variants; the concrete chain has two class and one callable specialization.
  These are pre-optimization structural counts, not retained machine-call or
  dynamic allocation counts. Build-plan reuse preserves compiler reports,
  artifact hashes, and file evidence for all four chain variants. Runtime
  fixtures and disposable outputs are removed; reports remain in secondary
  scratch. These quick, dirty-source checks do not promote a baseline.

- The 2026-10-07 readiness-error correction extends canonical `IoError` with
  `Register`, `Unregister`, and `Wait`, targeting `ReadinessSet` with null byte
  progress. Existing tags and original adapter/transfer errors remain intact.
  Cancellation without a transfer no longer fabricates a transfer error or
  known-zero progress. That readiness-error checkpoint passed 353 standalone
  and 354 hosted runtime tests on macOS, plus Windows and Linux runtime/test
  compilation without execution.
  Strict LLVM compiler-library Clippy, formatting, documentation, artifact-policy,
  and runtime-diagnostic guards pass. All 40 LLVM-enabled stream capability
  tests pass, including the new readiness-error fixture across interpreter,
  Cranelift, LLVM, and PHP. All 16 native parity checks pass at compiled checkpoint
  `dev-66b2fd363385e4efe734568bb7d72f25aa76e420eeeab08b5e5f6a2ac90b70ff`,
  including all 413 durable interpreter/Cranelift/LLVM fixtures, native closures,
  stack-growth checks, and pipe/prompt behavior. That checkpoint predates the
  wait-only runtime correction; its new-source validation remains separate.
  Matched LSP and website compiler-execution verification remain pending.
  Website guides/API, tutorial, and playground facts are updated locally, with
  123 content tests passing; its separate 1 MiB artifact limit preserves full
  source or reports rejection without changing the 64 KiB log limit. Installed
  tools, production pins, and publishing are unchanged. Stage 36a is incomplete.

- Andrew approved Decision 0136 independent owned-property borrowing on
  2026-10-07. Source and MIR checking now remove the former blanket whole-owner
  rejection of disjoint stored-field borrows. Parent lifetime, overlapping
  places, readonly access, and conservative indexed/opaque projections remain
  protected. Canonical default, hosted-runtime, and LLVM validation passed on
  macOS at the earlier source checkpoint
  `dev-f30d05da4084a333f78aa2f4bbdc7dbdf1981f87196a6d3fbcba285383eec9dd`,
  including all 412 durable interpreter/Cranelift/LLVM examples and targeted
  PHP ownership/cleanup regressions. Compiler-matched local tooling at that
  checkpoint passed 262 unit, three CLI, and 110 integration tests, strict Clippy, build,
  highlighting, and all 61 VS Code tests. The website guide, API reference,
  playground, and tutorial examples are coordinated locally. Production pins,
  installed tools, current-tree Linux/Windows execution, and authorized
  integration remain unverified or pending; this does not complete Stage 36a.

- Since that checkpoint, shared MIR lowering now materializes inline returned
  collections with their checked loan origins and guards nullable stored-field
  projections before access. PHP returned array-backed loans use existing
  stable storage cells across functions, methods, closures, getters, and indexed
  writes. The earlier focused macOS checkpoint passed 500 tests across eight suites and
  strict Clippy for those suites and the compiler library. Cleanup, evaluation
  order, lazy fallback, and forged presence/borrow controls remain covered.
  Existing PHP Set-method and shared-ownership boundaries remain explicit
  rejection checks. Subsequent PHP generic property writes now use checked,
  specialized expression types instead of reconstructing types from class names,
  preventing premature destruction of nested owned adapters. Setter replacement
  with blocking destruction now correctly receives the existing `E0770`; that
  erroneous acceptance is removed. Shared MIR collection storage now uses the
  checker's contextual construction types, including nested and generic literals,
  instead of an intrinsic-specific prewalk. Canonical default, hosted-runtime,
  and LLVM validation passes on macOS at source checkpoint
  `dev-4f933cedf87a2618d5e3b7f509e0e6c05734ba476f76772ab031fa82b62703ea`,
  including all 412 durable interpreter/Cranelift/LLVM examples. The benchmark
  repository adds a shared-workload concrete/erased adapter-chain case; all 142
  portable harness tests pass. Its smoke commands verify file identities before
  any timing; controlled timing and baseline promotion remain pending.
  Current-source Linux/Windows execution, installed refresh, and authorized
  integration remain pending.

- Stage 36a implementation is active under Decision 0110. Andrew approved
  `consuming function` and required explicit launch-time child cleanup policy
  (wait, terminate, or detach, with no default). Consuming receivers and basic
  owned-file transfers are implemented locally; the complete public stream
  runtime is not delivered. Remaining public-surface decisions, cross-platform
  execution, parity, structural benchmarks, tooling, website, and installed-tool
  refresh remain this stage's closure obligations.
  Andrew also approved shared cancellation state, allowing tokens to outlive
  their source; optional `IoError.transferredByteCount`, preserving unknown
  versus zero versus positive progress; and PHP FFI for classified Unix
  benchmark launch evidence. Decision 0110 records these contracts; their
  implementation and coordinated integration remain in progress.
  On 2026-10-07 Andrew approved next-completion events identifying the finished
  read or write without scanning transfers or mandatory per-completion
  allocation. Fetching a failed completion raises its original checked error,
  with the failed operation's identity available to its catch. Group-wide loans
  remain retained. Compiler request views and
  public group execution are being connected to the existing runtime engine;
  the complete stage is not delivered.
  Persistent readiness sets own registered streams; removal returns ownership.
  Andrew approved this rule on 2026-10-05. Temporary waits may borrow for the
  call; readiness execution remains in progress.
  On 2026-10-06 Andrew approved preserving the registered static type `T` on
  removal, including concrete file operations and checked `close()`. Typed keys
  do not own streams; native registrations borrow handle projections from the
  complete retained values. Public registration integration remains in progress.
  Andrew approved explicit borrowed readiness sources for custom adapters on
  2026-10-06 (`A`). Decision 0110 names `ReadinessStream.readinessSource` with
  `borrowed get` and opaque `ReadinessSource`. The shared wait core uses the
  projection while transfers retain the original adapter's behavior. Compiler
  provenance and lazy native file association are being integrated; this is
  not a public readiness execution or stage-completion claim.
  The static-type and borrowed-source ownership controls pass within all 37
  default-profile and 38 LLVM stream tests on macOS. They cover concrete and erased returns,
  mismatched key rejection, ordinary failed-`take` cleanup, and exact-once close.
  The generic Dictionary control retains PHP's existing `B2301` boundary; it is
  not the public readiness set. Dynamic borrowed native membership passes all
  295 macOS runtime tests; Linux/Windows cross-checks are compile evidence only.
  The new finite control is included in the durable parity manifest. Canonical
  default/LLVM work-unit validation now passes on macOS, including all 412
  durable interpreter/Cranelift/LLVM examples. Windows execution and the current
  tree's Linux execution remain unverified.
  Fresh strict Linux/Windows runtime and test cross-checks pass in standalone
  and hosted-shared configurations. The hosted configuration passes all 296
  runtime tests in debug and release on macOS. Its Windows test no longer
  references absent standalone CRT support: that support test is now restricted
  to its provider feature. Canonical local validation and tier-1 CI include the
  hosted variant; the added commands were verified separately, not by rerunning
  the complete default/LLVM validator after this test-only correction.
  The unchanged website counted-reader adapter exposed a shared PHP checked
  integer arithmetic gap. Width-aware overflow operations now cover addition,
  subtraction, multiplication, signed negation, compound updates, and
  increment/decrement. Writable collection homes follow one type-owned PHP
  representation plan across concrete, generic, and erased calls; byte regions
  retain their existing shared-storage representation. Indexed updates preserve
  bounds-before-RHS checks, and operand owners retain their statement lifetime.
  All eight arithmetic regressions pass with interpreter, Cranelift, LLVM, and
  PHP execution within canonical validation. The former checked-arithmetic
  `B1301` rejection is removed; separate division/shift/bitwise/conversion and
  canonical float-formatting boundaries are unchanged.
  The benchmark harness passes all 141 portable tests. Compiler selection now
  canonicalizes installed, cache, and PATH aliases before provenance checks;
  no controlled timing or baseline promotion was performed.
  Compiler-matched language-server validation passes on macOS: 258 unit,
  three CLI, and 100 integration tests, strict Clippy, build, formatting,
  highlighting, and all 61 VS Code checks. Concrete, erased-interface, and
  constrained-generic property completions now share compiler-owned accessor
  signatures while retaining stored-field distinctions and borrow/effect facts.
  The storage checker measures the configured cache; all 19 filesystem cases
  pass. The published compiler pin and installed binaries remain unchanged;
  this local-source validation does not claim a delivered tooling refresh.
  On 2026-10-06 Andrew approved the explicit pending-transfer capability (`A`)
  for concurrent custom adapters. Preparation and completion use group-owned
  reusable request slots; the compiler does not suspend ordinary adapter bodies.
  The shared core retains storage through cancellation drainage and preserves
  the complete adapter's behavior. Compiler/runtime integration is resumed;
  ordinary synchronous transfers remain unchanged and stage closure is pending.
  Nominal pending capabilities and request members are now recognised. Shared
  effect checking rejects blocking phase work with `E0771`; call-scoped request
  checks reject ownership/storage escapes with `E0772`, preserving borrowed
  helpers, parameter-rooted return loans, local aliases, and nonescaping
  callbacks. Shared MIR validation also rejects owning or escaping request
  storage, including `mixed` boxing. Shared closure analysis now distinguishes
  rejected authored captures from missing captures, removing duplicate
  machine-applicable insertion fixes while preserving primary mode errors and
  genuinely missing-capture fixes. The request and closure-semantic suites pass
  all 40 tests with strict compiler/test Clippy on macOS; adjacent ownership,
  return-loan, stream, and cleanup suites also pass. The request suite includes
  26 forged-MIR rejection cases. At that semantic checkpoint, public
  request/group execution retained the explicit `B3603` implementation boundary;
  the active integration above records its subsequent frontend removal.
  Andrew approved nullable pending request outcomes and completion returns:
  absence means no transfer result exists, while immediate buffered results and
  real zero-byte/would-block outcomes are retained. Ordinary synchronous outcome
  enums and returns are unchanged. Authority, declarations, and tooling/website
  references are synchronized. Nullable outcome transport passes interpreter,
  Cranelift, LLVM, and PHP execution with allocation-free native returns. All 13
  request-contract tests, 12 focused tooling tests, 104 website content checks,
  and 141 portable benchmark tests pass. Website checks exclude real-compiler
  fixtures and the unpublished inventory lock. Eight guard mutations prove
  stale pending and nullable synchronous signatures are rejected. Public
  request/group execution still retained `B3603` at that checkpoint; installed
  tools and published pins remain unchanged.
  Full default, hosted-runtime, and LLVM behavioral validation passes on macOS,
  including all 412 durable native examples. The canonical runner exits with a
  storage failure after its 15.72 GiB cache exceeds the 15 GiB cap; managed Cargo
  reclamation succeeds and leaves the combined secondary caches at 7.54 GB.
  Andrew also approved the owned file-family names: `ReadableFile`,
  `WritableFile`, and `DuplexFile`, with `File::openRead`, `openWrite`, and
  `openReadWrite`. Decision 0110 records this 2026-10-05 ruling; public file
  native writable execution is implemented; remaining control spellings are still
  pending. On
  2026-10-05 Andrew approved required `FileCreation` arguments for writable
  opens and `consuming function close(): void throws IoError`, with
  `IoOperation::Close`. These decisions unblock public file integration; they
  are not a completion claim.
  Andrew then approved required explicit `FileWriteMode::Positioned` or
  `FileWriteMode::Append` for both writable factories, independently of the
  required creation policy. Decision 0110 and the standard-library inventory
  record the signatures. The focused stream suite passes 31 tests, including
  the complete 40-case creation/mode matrix on the interpreter, Cranelift, and
  LLVM on macOS. Canonical default/LLVM work-unit validation now passes,
  including the complete 409-fixture interpreter/Cranelift/LLVM matrix. The suite
  also covers
  independent cursors, append after another handle extends the file, empty
  writes, duplex append/read cursor transitions, interface dispatch, single-close
  cleanup, known/unknown failure progress, named-argument evaluation, and forged
  MIR rejection. The bounded review corrected public Windows sharing for
  independent readers/writers without changing legacy helper sharing, and checked
  I/O validation now requires owning result slots for every owned file and byte
  result. At that checkpoint, 155 runtime and 181 compiler unit tests passed on macOS.
  Strict Windows runtime/test cross-compilation passes; it is not Windows
  execution evidence. The benchmark harness now passes 140 portable tests;
  no controlled timing or baseline promotion was performed.
  PHP readable, writable, and duplex file translation now uses the same native
  runtime through FFI, removing the former `B3601` writable-file boundary.
  Eight focused PHP tests and the 30-test default stream suite pass, including
  the complete 40-case creation/write-placement matrix, borrowed native byte
  slots, binary region reuse, checked error origins, owning cleanup, relocation,
  disabled FFI, and missing/corrupt/mismatched dependencies. Pure PHP streams
  remain FFI-independent. Compiler-owned publication deploys a verified shared
  runtime and metadata beside PHP output in `.doria-runtime`; `B3602` reports
  invalid deployment without inventing an `IoError` or using a host-file fallback.
  Canonical default/LLVM work-unit validation passes on macOS, including 174
  runtime tests, 191 compiler unit tests, 116 PHP tests, and the complete
  409-fixture interpreter/Cranelift/LLVM matrix. The original PHP I/O regression
  remains unchanged; internal always-throwing helpers retain the backend's
  established `void` declaration convention. Checkpoint delivery also requires
  the installed-tool refresh and its relocated PHP/native dependency probes.
  Stage 36a is still incomplete; that macOS checkpoint is not Linux or Windows
  runtime evidence. The later Linux execution results are recorded below.
  The private standard-device bridge now shares stdin's owning queue with line
  and legacy byte consumers and exposes one primitive byte transfer rather than
  implicit write-all or text conversion. File and device bridges share the same
  inline status layout, preserving the existing file ABI and optional failure
  facts. All 163 runtime tests, strict macOS/Windows compile checks, and the
  production runtime build pass; public standard-view names and dispatch are
  still deferred. Andrew has approved transfer-first Windows pipe completion,
  with buffer borrows retained through terminal completion and cancellation
  drainage. The private Windows completion path now owns a reusable event,
  pinned request, buffer borrow, cancellation claim, and one original wait
  budget through terminal completion. Cancellation drains before release, and
  wait-stop facts remain separate from actual transfer progress. Lifetime-model
  tests pass on macOS; six genuine Windows fixtures compile but have not executed.
  Public pending-operation names and compiler/MIR integration remain work;
  hidden buffering, polling, and per-stream worker threads remain prohibited.
  Andrew approved one exclusively borrowing concurrent transfer group on
  2026-10-06. Completed buffers may be processed and reused through it while
  other transfers remain pending; original bindings remain unavailable until
  group teardown drains pending operations and releases the retained loans.
  Decision 0110 records this ruling; its implementation remains in progress.
  That ownership ruling is not platform execution evidence.
  The private Unix readiness transport uses persistent epoll/kqueue watches,
  reusable inline event storage, generation-checked registration keys, and the
  shared cancellation and original wait-budget owners. Twelve focused tests
  cover 96 watches, allocation-free waits, advisory closure, regular-file EOF,
  stale generations, interrupted waits, deadline spending, and transactional
  registration failure. Failed recovery closes only the watcher and preserves
  registered streams. One shared transfer core now owns fixed request/buffer
  storage, fair completion delivery, bounded terminal progress, and
  cancel-all-before-drain cleanup. Its private platform-neutral group transport
  replaces the newly introduced Unix-prefixed group entry points; the shared
  inactive-set gate preserves loans and returns live stream ownership on release.
  Ten Unix transfer regressions cover mixed transfers, allocation-free reuse,
  original budgets, cancellation, regions, EOF, failed watcher retirement, and
  regrouping, and malformed-input rejection before submission. All 237 runtime
  tests and strict Clippy pass on macOS; this includes
  eleven Windows routing-model tests, not Windows syscall execution. Strict
  Linux runtime/test cross-compilation passed at that checkpoint; the later
  Linux execution results are recorded below.
  Windows retains each stream's completion-port association across set removal
  and regrouping. The association lasts until handle close
  ([Microsoft's contract](https://learn.microsoft.com/en-us/windows/win32/api/ioapiset/nf-ioapiset-createiocompletionport));
  a group-local port owner cannot satisfy that lifetime. The private lazy broker
  and fixed-set adapter now use the shared group transport. Strict Windows
  production/test cross-compilation passes without warning suppression, including
  eight broker fixtures, six preserved regressions and two private-ABI fixtures.
  Native Windows execution remains unverified. The macOS production runtime
  archive builds and exports the platform-neutral group entry points. A separate
  C executable linked against that archive passes layout, active-loan rejection,
  caller-buffer reuse, regrouping, EOF and cleanup checks on macOS; it is not
  public compiler execution or Linux/Windows evidence.
  Andrew approved consuming `TransferPlan` setup on 2026-10-06
  (`Public API choice: A`): configure all requests before ordinary
  `$set->begin($plan)`, with `take` declared on the parameter. Decision 0110 and
  the standard-library inventory record the setup and retained-loan contracts.
  The shared source/MIR carrier classification now preserves existing loans
  through consuming receivers, generic constraints and checked interface erasure.
  The durable consuming-retained-receiver fixture covers two source loans,
  success/error cleanup, original-source conflicts, owned-storage escape and
  forged loan removal. All 136 focused default-profile and 39 LLVM compiler tests
  pass; strict workspace/all-target Clippy passes for both feature graphs. LLVM
  native linking reports the existing host libunwind re-export warning. The
  compiler-matched local-server build and all four focused editor regressions
  pass, including six diagnostic edit/clearing cycles, without repinning to an
  unpublished compiler tree. Canonical default/LLVM validation passes with the
  added fixture, including all 410 durable interpreter/Cranelift/LLVM examples.
  Installed-tool verification is separate from compiler validation; editor
  packaging and the final published-pin refresh remain Stage 36a closure work.
  Private keyed-plan setup and arbitrary subset selection pass 255 runtime tests
  on macOS. Warning-free Linux/Windows cross-checks passed at that checkpoint;
  subsequent Linux execution is recorded below. Windows execution remains
  unverified.
  This fixed native configuration is not the public dynamic-registration API;
  that integration must preserve complete stream owners and stable unaffected
  registrations instead of rebuilding them or introducing a second handle owner.
  Compiler/MIR integration remains work; no public
  readiness or multi-transfer delivery is claimed by this private checkpoint.
  Linux ARM64 execution now passes 256 runtime tests, 27 consuming-receiver
  tests and 12 parity tests, including all 410 interpreter/Cranelift examples,
  at source identity `dev-8ad46e44c4e8f0ebfb55c36a0c8297904aa390a312bdf8f33e6880c23efbf5ed`.
  These locked, offline tests used the official Rust 1.98.1 container; LLVM and
  PHP execution were not enabled there. Docker Desktop was stopped afterwards.
  This is virtualized Linux correctness evidence, not Windows execution or
  controlled performance evidence. The subsequent shared test-directory
  cleanup correction passes separate macOS cleanup and prompt-timing regressions.
  The monotonic clock uses libc's target-owned time layout, clock ID, and time64
  symbol selection, not a hand-declared host-shaped timespec. A focused regression
  proves zero runtime allocations at all four successful argument-free entry
  boundaries; it does not claim zero host startup cost or complete stream coverage.
  Read-only file integration now lowers `File::openRead`, concrete and erased
  `ReadableFile.read`, and consuming `close` through typed checked MIR calls.
  Targeted interpreter/Cranelift/LLVM/PHP execution agrees on destination regions,
  independent cursors, EOF, and missing paths. Fault-host tests preserve partial
  progress and prove one close attempt even on checked close failure. MIR rejects
  ordinary allocation of opaque file storage and receiver access after close.
  The shared nonblocking analysis sees file calls and implicit file cleanup;
  compiler-owned bodies cannot bypass the property-hook contract. PHP regressions
  also cover binary region reuse, owning containers, and authored error origins.
  The preceding checkpoint passed canonical default/LLVM validation, including
  formatting, Clippy, workspace tests, and the 409-fixture
  interpreter/Cranelift/LLVM matrix.
  Coordinated tooling passes 350 server, 66 IntelliJ, and 61 VS Code tests.
  Installed CLI tools and editor packages must identify this source tree's
  fingerprint at checkpoint delivery; immutable pins require a final published
  compiler commit. PHP writable files are locally validated; remaining controls are pending;
  this is not a stage
  completion claim.
  The four byte-transfer interfaces and two inline Copy outcome enums now use
  ordinary conformance and dispatch. Targeted tests verify interpreter,
  Cranelift, LLVM, and PHP parity, reusable storage, checked failure progress,
  aliases, multi-file source provenance, and allocation-free outcome transport.
  Shared interface lowering separates automatic I/O effects from required
  function-type identity; source catch-or-declare obligations are unchanged.
  Owned-registration regressions cover transfer, removal, and reverse cleanup.
  Shared ownership classification now includes interface-typed inputs, while
  nested collection lowering preserves writable interface receivers and call
  arguments without weakening MIR validation. Public readiness execution is
  still pending.
  Owned factory results and writable property projections now retain exclusive
  receiver access through concrete, interface, and structural calls; readonly
  return loans remain readonly. Indexed writable arguments trace every property
  edge back to their owner instead of skipping readonly checks at an index.
  Nullable shared results preserve their family's access capability, and shared
  payload projections reserve their owned temporary cleanup slots in both native
  backends. The shared-access controls and durable cleanup fixture cover these
  paths. The checkpoint passed canonical default/LLVM validation and the
  404-fixture parity matrix; compiler-matched installed tools and ten installed
  native smokes were verified. This does not close the whole Stage 36a gate.
  The private file bridge owns the opaque headerless payload, opening path, and
  native close obligation. Layout is runtime-queried; explicit close clears the
  obligation before checked-error handling, while ordinary destruction releases
  the retained path before common class free. The private flush-only export was
  removed in favor of one file-control export for flush, synchronization, seek,
  position, and length; flush behavior is preserved. Its 150-test runtime checkpoint passed
  on macOS, including seven bridge regressions, and strict macOS/Windows compile
  checks passed. The shared write-all core now retains suffix offsets through
  advisory readiness and exact prefix progress when a wait is interrupted;
  unknown transfer progress is never retried. All 153 runtime tests pass on macOS.
  This private continuation does not deliver platform readiness or public wait
  spellings. Windows and Linux had not executed at that private-file checkpoint;
  the subsequent Linux evidence is recorded above. Windows execution remains
  unverified. Remaining public compiler/backend integration, readiness, and
  complete platform execution remain in progress. The read-only subset passes
  the complete native parity manifest above; installed identities must be
  verified at checkpoint delivery.
  This is not a Stage 36a completion claim.
- Stage 35 Slice 4 implements recursive generic trait composition, method
  requirements and adaptations, deterministic instance layout and per-composer
  static cells. One compiler-owned plan feeds ordinary class semantics and all
  backends; effective member identities retain their authored source origins.
  E0493 is retired and reserved. Slice 5 closes the parity, structural,
  documentation, and cross-repository integration matrix; Stage 35 is complete.
- Stage 35a implements shared optimizer proofs and bounded stack promotion for
  nonescaping closed classes in both native backends. Closure validation prevents
  stack environments from escaping; ownership and reverse cleanup are unchanged.
  Structural regressions cover dispatch, pointer metadata, and conservative heap
  fallbacks. Opt-in reports cover multi-file builds and native code size; pinned
  benchmark workloads separate specialization growth from erased dispatch.
  Tooling describes source dispatch without promising a machine-code shape.
  Stage 35a is complete; Stage 36 grammar, hook-body checks, concrete/interface/
  constrained call analysis, and override contract checks are implemented under
  Decision 0135. HIR storage and accessor bodies, MIR dispatch, and PHP accessor
  calls are enabled through the public compiler pipeline. Andrew approved
  exact setter input types, inherited backing-field reuse, separate callback
  ownership and receiver-bound lifetimes, and transitive blocking-I/O rejection.
  These rules have regression coverage. Backed override initializers run parent
  then child, replacing the inherited value after successful evaluation.
  Stage 36 is complete: durable native parity, PHP coverage, ownership leak
  checks, tooling, website acceptance, and installed-tool verification pass.
  Controlled timing
  remains **Measurement Status: Pending Available Runner**, never a pass.
- Stage 35 Slice 3 implements canonical value equality/hash/order/clone,
  Copy-or-Cloneable preserving collections, and public iteration. Promoted
  `borrow` constructor parameters retain readonly sources; `getCurrent()` lends
  an element without consuming it. Source and element loans survive forwarding,
  erasure, captures, and checked exits. Built-in loops remain direct; bounded
  nonescaping built-in cursors have fixed-frame storage. Hash-key stability remains an
  author obligation, including shared or external state; violations must not
  compromise memory safety.
- Earlier checkpoint validation exposed intermittent Native Testing failures:
  one
  Native Testing run ended with SIGBUS in
  `slice_three_throw_matchers_intercept_checked_errors_exactly`; an additional
  LLVM-enabled suite later missed an outcome file in
  `failed_string_equality_reports_a_grapheme_aware_v4_difference`. Focused and
  full default/LLVM-enabled reruns passed, but both causes remain unconfirmed.
  The shared outcome helper now reports the child status, stdout, stderr, and
  artifact paths on failure. No retry of a failing executable was introduced.
  Exact-head CI and cross-repository delivery are checked for every work unit.
- Stage 18 full expression interpolation and compiler-known `Displayable` is merged.
- Stage 19 ownership, moves, destruction, and native class layout is complete on the current branch.
- Stage 20 statically resolved instance/static methods, Copy-type static properties, class/top-level constants, `internal` enforcement, and concrete native `Displayable` execution are complete on the current branch. Static access is sigil-free, `self` resolves to the declaring class, and one class-level index rejects cross-kind member-name collisions.
- Stage 20a/20b const-evaluable defaults are complete for Copy scalars and readonly strings across free functions, instance methods, static methods, and constructors through one caller-side MIR splice. Writable Copy scalars remain supported; `?string`, `writable string`, `take string`, and other move/`take` defaults retain explicit temporary diagnostics.
- Stage 21 non-lexical borrowing, returned-borrow elision, and constructor definite initialization are complete on the current branch. Returned-borrow provenance now remains tied transitively to `$this` or one borrowed parameter through property paths, collection indexing, and compiler-known `Dictionary::get`/`List::first`/`List::last` reads. Constructor paths use decision 0090's uninitialized/initialized/maybe-initialized lattice, and shared MIR validation independently enforces the normal-exit and readonly exactly-once invariants.
- Stage 22 general nullable types, `??`, `?->`, exact `is`, flow-sensitive narrowing, and `mixed` static semantics are complete on the current branch. Narrowing reuses the shared CFG/forward-dataflow framework; local and parameter guards preserve dominating nullable-presence proofs in MIR, and nullable scalar, string, and concrete-class values execute through the interpreter, Cranelift, and LLVM.
- Stage 23 Slice 1 runtime collections and typed arrays are complete on the current branch. `T[]`, `List<T>`, `Dictionary<K, V>`, and `Set<T>` are owned move types backed by shared collection MIR and `doria-rt`; contextual literals, fixed-length typed arrays, indexing and indexed read-modify-write, insertion-ordered `foreach`, move-in/removal ownership, and Decision 0100's default member surface run through the interpreter, Cranelift, and LLVM. Decision 0113 completes that surface. Slice 1 implements `containsKey` and the widened `contains`; Slice 2 implements receiver-aware structured fixes, property-call diagnostics, and withdrawn literal-family `::from` guidance; Slice 3 implements `List::indexOf`, `List::remove`, map `containsValue`, and readonly set endpoint properties; Slice 4 implements writable in-place `clear(): void` on all seven named collections with exact-once cleanup and reuse. No Decision 0113 member remains pending. Dictionary `keys`/`values` are readonly, insertion-ordered, `foreach`-only projections and are not storable values.
- Stage 23 Slice 2 is complete on the current branch. The owned `Bytes` move type provides explicit copying conversion to and from `uint8[]`, length, byte indexing and indexed read-modify-write, and byte-wise equality. `read_file_bytes`/`write_file_bytes`/`append_file_bytes`, `read_stdin_bytes`/`write_stdout_bytes`/`write_stderr_bytes`, and text `append_file` use shared validated MIR and `doria-rt`, with exact non-UTF-8 bytes and interpreter/Cranelift/LLVM parity.
- Stage 23 Slice 3 is complete on the current branch. The boxed `dr_mixed` runtime representation stores a tag, class type id when needed, and owned payload; bool, fixed-width integers, floats, string, and concrete classes box into `mixed`, narrow back out through exact `is`, and execute through the interpreter, Cranelift, and LLVM. `?mixed`, `List<mixed>`, `Dictionary<K, mixed>`, and `Set<mixed>` value paths use the same shared MIR/runtime box. Stage 34 adds subtype tests; Stage 35 Slice 2 adds interface boxing and nominal class/interface tests over the same box. Collection tests and boxing collections, typed arrays, or `Bytes` into `mixed` remain deferred with stage-named diagnostics.
- Stage 27 is complete under decision 0114. Unit, `int`/`string`-backed, and payload enums are nominal inline values with one central layout authority. Payload construction supports positional and named arguments, recursive Copy/Move classification, active-case copy/drop/equality, aggregate function ABI, nullable and `mixed` transport, Copy constants/defaults, class and generic-class properties, permitted collection value positions, and faithful PHP compatibility lowering. The interpreter, Cranelift, and LLVM consume the same validated payload-enum MIR and durable fixtures. The subsequent generic-enum amendment belongs to Stage 36a and remains unimplemented.
- Stage 28 is complete under implemented decision 0115. Slice 1 delivered guard-free core `match`, exhaustive unit/payload enum cases, case-only payload ignore, exact constants, `null`, exact type-binding narrowing, strict arm typing, readonly payload ownership, ordered `match (true)`, and full right-associative ternary. Slice 2 adds `if` pattern guards with ordered one-time evaluation and guard-aware coverage/reachability, explicit `match (take $value)` whole-scrutinee consumption, selected Move payload ownership and exact cleanup, rejected writable patterns, and PHP exact tagged `mixed` identity. One validated MIR CFG drives the interpreter, Cranelift, and LLVM; PHP preserves the same checked behavior.
- Stage 28a is complete under implemented decision 0116. Slice 1 executes value-returning `when`, `given` prelude gates on `if`, `when`, and `while`, and base `do ... while`. Slice 2 executes `finally` on `if`, `when`, `while`, and `do ... while` through one validated backend-neutral finalizer-region CFG. Outgoing values are acquired before cleanup; branch/body locals drop before the finalizer; `given` locals drop after it; nested regions run inner-to-outer; same-loop `continue` skips the loop finalizer; and fatal panic bypasses every region. The interpreter, Cranelift, LLVM, and PHP compatibility backend preserve the same checked behavior. E0611 is retained only as an unreachable historical diagnostic identity.
- Stage 29 is complete under accepted and implemented decision 0119. Slices 1 and 2 provide checked syntax/effects, the two-word carrier, descriptors/origins, checked MIR/ABI, catch dispatch, propagation, finalizer routing, and failed-construction cleanup. The native collection property initializer corrective beat is closed; Decision 0122 now separately accepts and implements owned-property move-in and writable replacement while leaving move-out separate. Slice 3 adds the six exact compiler-known `Doria\Std\Io` identities, one authoritative built-in effect table, deterministic checked I/O across the interpreter, Cranelift, LLVM, and PHP, and R1000/status-70 reporting after cleanup for an Error escaping `main`. The inferred-main corrective beat is complete: reusable callables still declare escaping checked effects, while a clause-free selected entrypoint infers its exact uncovered source-ordered set and carries it through source calls, HIR/MIR, ABI selection, and all backends. Explicit `main throws` remains accepted. B2901 and B2902 have no valid route; P1401 through P1407 are historical for ordinary I/O. Allocation panics, fatal status 101, and ordinary closed-standard-pipe status 0 remain distinct.
- The pre-Stage-32 ambient-I/O and fallible-finalizer corrective beat is complete under Decision 0123. Exactly `Doria\Std\Io\IoError` and `Doria\Std\Io\InvalidUtf8Error` are ambient: source callables need no catch-or-declare boilerplate, while HIR, MIR, catches, checked ABI transport, cleanup, and R1000 retain the exact Errors. Required nonambient effects remain explicit and structural. Source finalizers may propagate checked Errors; same-try sibling catches exclude finalizer effects, outer and finalizer-local catches may handle them, and a finalizer Error supersedes a pending return, `when` result, loop transfer, or earlier Error with exact-once owned-payload cleanup. E0632 has no live route and remains historical and reserved. Panic, destructor, initializer, closed-pipe, and status behavior are unchanged.
- Decision 0120 accepts one explicit closure-capture model: arrows and anonymous block functions must list enclosing local dependencies with `with`; Copy and Move values have no implicit exception; no-capture closures omit `with`. The grammar foundation and Stage 30b semantic capture validation are complete.
- Decision 0121 accepts the complete Stage 30 closure authority. Stages 30a through 30h and Stage 30 are complete: structural function types have canonical semantic and MIR identity; closure bodies and explicit captures are checked by stable binding identity; captures acquire at creation in authored order; every function value is Move; readonly/writable leases end at last use; once calls consume path-sensitively; and escape, storage, callback retention, return roots, nested provenance, reverse logical release plans, and function values in `mixed` are enforced. Explicit closure/callable-call HIR, static descriptors, concrete native environment layouts, synthetic functions, indirect and checked indirect MIR, shared validation, stable places, and exact function-value cleanup execute through the debug interpreter, Cranelift fast profile, LLVM release profile, and PHP compatibility for the supported value surface. Stage 30g adds compiler-known `List<T>::map`, Copy-or-Cloneable `filter`, and writable-accumulator `reduce`; one explicit semantic/HIR plan and validated MIR traversal preserve source-order callback execution, exact effects, readonly source borrowing, Move map results, Copy-or-Cloneable filtering, owned accumulators, and checked cleanup. PHP emits explicit ordered loops and no host higher-order array function. E0641 is historical and remains reserved.
- Decision 0122 is accepted and implemented. Direct constructor `$this` is a construction root rather than a writable receiver; definitely initialized writable intermediates grant ordinary nested writable access. Owned values initialize owning properties and replace initialized writable owning properties through explicit validated MIR write kinds with acquire-before-drop behavior. E0472 is historical for move-in and remains only on the separate move-out boundary.
- Phase F namespace, autoload, package, and dependency authority is accepted under decisions 0117 and 0118. Stage 31 is complete. Slice 1 provides namespace/import/include grammar, canonical resolution, the edition-2026 prelude, and package/source identity. Slice 2 provides strict schema-1 JSON build plans, explicit multi-package source graphs, recursive include-once, direct and development dependency visibility, package-wide `internal`, strict hybrid layout, declaration-only non-entry sources, cross-file diagnostics, multi-source HIR/MIR/backend execution, and in-memory compiler fingerprints and invalidation. The public source-discovery term is `autoload`; Baton passes plans to `doriac`, while `doriac` never parses `Baton.toml` or discovers package inventories. E0671 and E0672 have no live route and remain historical and reserved.
- Stage 32 is complete under Decision 0125. Adjacent `#[...]` groups, typed attribute classes and applications, canonical package-aware name resolution, bounded constant-tier arguments, semantic/HIR metadata, deterministic `doriac metadata` output, and the strict processor request/response protocol are implemented. Attributes add no MIR/runtime representation or reflection, and Stage 32 does not discover or execute processors.
- Stage 33 is complete in the disposable PHP UX bootstrap, and Phase F is complete.
  - **Slice 1 — Complete:** Decision 0126 covers exact schema 1 compatibility, strict schema 2, local/scoped package identity, binary/library targets and selectors, autoload/autoload-dev discovery, deterministic source inventories, compiler build plans, target-scoped layouts, and receipts.
  - **Slice 2 — Complete:** Decision 0127 covers normal path/Git dependencies, strict SemVer validation, one-version and source-substitution diagnostics, deterministic strict JSON `Baton.lock`, exact locked installs, dependency commands, centralized offline policy, global immutable Git caching, live path packages, multi-package plans, and receipt lock/path identities.
  - **Slice 3 — Complete:** Decision 0128 implements canonical source descriptors, development dependencies, workspaces, graph commands, compiler-metadata-driven tests, one-pass processors, generated-source orchestration, incremental project inventory, and asynchronous language-server project integration.
  - **Native transition — Scheduled:** Decision 0124 requires a parity-gated port to the clean Doria-native `dorialang/baton` repository, production release-ownership transfer, and PHP-free archive cutover before the unsuffixed `2026.03.1` release.
- Installed-tool runtime ownership is corrected. Standalone `doriac` embeds both deterministic `doria-rt` profiles and materializes the selected verified archive by content identity; it no longer relies on a Cargo target path surviving installation. The development refresh deletes `target/toolchain-install` before a native compile-and-run smoke, while Baton `doctor` now performs a structured native compile probe so an identity-correct but unusable compiler is reported as failed.
- Native Testing Foundation Slices 1 through 3 are complete under Decision 0129. Exact compiler-known `Doria\Std\Test` declarations remain restricted to active development source and elaborate into deterministic ordinary generated callables without runtime registration. Strict metadata schema 3 remains compatible with schemas 1 and 2. The completed expectation surface includes scalar, string, collection, dictionary, and checked-Error matchers; bounded failure differences; strict DORIAO4 outcomes; exact cleanup across interpreter, Cranelift, LLVM, and PHP; classified hierarchy-aware Baton reporting; and compiler-fact-based tooling. The Native Testing Foundation is complete, and the Pre-Stage-45 Doria-native Baton transition remains scheduled.
- Stage 34 Single Class Inheritance is complete under Decision 0130. Closed-by-default classes opt into subclassing with `open`; virtual methods opt in with `open function`; replacements require `override function`. Qualified and generic parent types, inherited member lookup, direct `parent::` calls, root-first construction, derived-first destruction, implicit allocation-free upcasts, hierarchy-aware `is`/match/Error coverage, static hierarchy descriptors, lifecycle-phase dispatch, and exact/direct or constant-slot virtual calls execute through shared validated MIR on the interpreter, Cranelift, LLVM, and PHP compatibility backend. Closed exact values remain one word, open static types use a two-word data/descriptor carrier, and object payloads remain headerless. The Post-Stage-34 Constructor Parameter Roles Corrective Beat is complete under Decision 0131: default/internal promotion remain, `override` reuses one inherited external property without storage, and `parameter` creates a constructor-only input. The Post-Stage-34 Indexed Foreach And Scalar Display Corrective Beat is complete under Decision 0132. The Post-Stage-34 Explicit Foreach Binding Types Corrective Beat is complete under Decision 0133: every foreach binding carries an authored type, with compiler inference retained only for E0748 recovery and fixes. Stage 35 Interfaces And Traits authority is accepted under Decision 0134; Slice 1 Grammar, Graphs, And Conformance is complete. Slice 2 Interface Runtime And Ownership executes two-word interface carriers, checked static vtables, dynamic drop, storage, nullable/mixed narrowing, Error subinterfaces, Displayable, and all six shared families. Slice 3 executes core contracts, preserving duplication, and public iteration; Slice 4 executes trait composition. Slice 5 closes the backend, documentation, tooling, and website integration matrix.
- Stage 25a Slices 1 through 4 are implemented and Stage 25a is complete. The readonly `SharedReference<T>` / `WeakReference<T>` family and the permanently disjoint writable family lower through validated MIR to the interpreter, Cranelift, LLVM, and separate `doria-rt` control structures. `WritableSharedReference<T>` executes class, generic-class, typed-array, `List<T>`, `Dictionary<K, V>`, `Set<T>`, and `Bytes` payloads through owned readonly/writable access objects. One access state is shared by every writable-family handle to an allocation; access objects move through returns, parameters, properties, and collection slots; nullable strong, weak, and readonly/writable access forms remain in-family and lazy; and destruction releases access before strong ownership. P1501 carries one of the three exact Decision 0106 conflict conditions as a typed runtime fact. The allocation-free `referencedValue` projection resolves wrapper/payload collisions without changing either ownership count, and durable weak-cycle and bounded-stress fixtures agree across all native paths. Scalar/string payload access and all shared handles through `mixed` remain runtime-pending rather than being given an invented value projection or misrepresented as class pointers. PHP implements the same explicit owner/weak/access cleanup for supported class, collection, and interface payloads; unrelated scalar/string payload and shared-in-mixed boundaries remain.
- PHP Stream And I/O Completeness Audit — Implemented. The canonical human
  artifact expands `docs/notes/io-surface-audit.md`; the stored 153-row PHP
  inventory and offline guard classify every official Stream Function,
  `streamWrapper`/`php_user_filter` method, `StreamBucket`, wrapper/context/filter
  family, relevant open-stream filesystem/process entry, and Readline boundary.
  Capabilities are preserved through typed owners or explicit deferrals without
  accepting PHP resources, dynamic wrapper/filter registries, global contexts,
  mixed metadata bags, or sentinel outcomes.
- Andrew’s Stream API Completeness Review — Complete. Stream, Readiness, Standard I/O, Blocking Mode, And Performance Model — Accepted (decision 0110). The semantic and performance model is fixed: small byte
  capabilities, owned handles, typed outcomes, first-class non-owning standard
  streams, one readiness/time/cancellation/backpressure substrate, and typed
  domain adapters. Steady-state stream loops have no mandatory per-operation
  allocation or whole-chunk copy; reusable buffers expose safe byte regions;
  concrete adapters remain statically specializable; readiness storage is
  reused; and synchronous programs initialize no async executor/task machinery.
- Stage 36a — Complete. Stage 36a Remaining Public Spellings — Deferred.
  At this entry's date the stage was still in progress: transfer capabilities, byte regions, owned files,
  readiness registrations, reusable transfer groups, time/cancellation controls,
  and file seek/inspection/synchronization/resizing execute on macOS through the
  interpreter, native profiles and PHP compatibility. The active checkpoints
  above record the exact validation scope. Blocking-mode controls, remaining
  readiness operations, first-class standard-stream accessors, public buffering
  and text adapters, owned processes, locking and remaining file capabilities
  are unfinished. Pure PHP streams remain FFI-independent; native-backed files
  deploy verified runtime assets and report `B3602` for invalid dependencies.
  Full portable execution, performance and coordinated delivery gates remain
  pending; cross-compilation is not cross-platform execution.
  Remaining names reopen through
  decision 0110's bounded appendix before implementation, without reopening the
  accepted semantic or performance architecture. Stage 36a owns the initial
  cross-platform stream benchmark and memory-regression gate; Stage 43 later
  continues and broadens it.
- `parent::member()` is implemented as direct immediate-parent resolution, including constructor, instance, static, constant, and static-property forms permitted by Decision 0130. Decision 0134 Slice 4 binds trait-local `self::member` to its composing class before backend emission. `Foo::$prop` and `static::` are permanent errors with precise fixes.
- Native remains one target: direct compile/run uses the Cranelift fast profile, while `--release` selects LLVM 18 over the same validated typed MIR.
- Both native profiles keep loop-body stack use constant. Every LLVM scratch slot — the flags and out-parameters the dictionary, set, list, string-search, and parse lowerings use to return `?T` — is allocated in its function's entry block, because LLVM treats an allocation anywhere else as a dynamic stack allocation that moves the stack pointer when it executes and is not reclaimed until the function returns. Cranelift stack slots are function-scoped by construction and were never affected. `llvm_mir_tests` asserts the placement on the module the backend emits before optimization, and `native_mir_parity_tests` runs those loop bodies two million times on every enabled profile and asserts a clean exit with exact output.
- Ordinary expression interpolation of primitive/string values lowers through the existing ordered MIR string and display operations consumed by all three execution paths.
- Native classes now cover construction, property initialization/access, class-valued locals/arguments/returns, `take` transfer, lifecycle bodies, recursive destruction, and deterministic normal structured-exit cleanup through the interpreter, Cranelift, and LLVM.
- Concrete `Displayable` conversion lowers to an ordinary direct `toString()` method call for interpolation, `.`, `echo`, and `%s`; Stage 35 Slice 2 adds erased, inherited, narrowed, and nullable-proven Displayable calls through the general interface machinery.
- Stage 23a named arguments are complete on the current branch. Call arguments carry an optional parameter name through the AST and Doria IR, and one shared binding step maps them to parameters before type inference for free functions, instance methods, static methods, and constructors. Positional arguments may precede named ones but not follow them; named arguments reorder freely and may skip a defaulted parameter, including a middle one, whose folded default the caller-side splice fills in by parameter position. Arguments evaluate in source order regardless of the parameter they bind to: a reordered call evaluates each observable argument into an ordered temporary and then assembles the callee vector in parameter order. Scalar and string temporaries remain borrowed; class, collection, `Bytes`, `mixed`, and their nullable move forms use tracked owned temporaries that move into the call. A borrowing parameter causes the caller to drop that temporary once after the call, while a `take` parameter transfers the obligation to the callee. The shared MIR validator, interpreter, Cranelift, and LLVM paths enforce the same rule, so ownership/borrow checking, side effects, and destruction all preserve the written program. Parameter names are part of a callable's public API; language intrinsics stay positional-only.
- Stage 23b program entry arguments are complete on the current branch. `main(List<string> $args)` joins the parameterless forms for both return types; semantic checking rejects every other entry shape (a second parameter, a non-`List<string>` type, and `writable`/`take`), and shared MIR validation independently enforces that the parameter is a borrowed `List<string>`. `doria-rt` entry glue builds the owned list from the platform argument vector, lends it to `main`, and releases it afterwards — using `GetCommandLineW`/`CommandLineToArgvW` on Windows, because the ANSI `char**` handed to C `main` there cannot represent every argument. The executable path is stripped, so `$args[0]` is the first real argument and a no-argument invocation yields an empty list. Non-UTF-8 arguments panic rather than entering the program. `doriac run <source> [--release] [-- <program args>...]` forwards arguments after `--`.
- Stage 23c sequence fills are complete on the current branch. `[value; count]` constructs runtime-sized `T[]` or one-shot-filled `List<T>` values, defaulting to `List<T>` without context. The value and count each evaluate once in source order; Copy values preserve their complete representations and required managed claims, with string handles retained once per slot. Constant-negative counts are rejected, dynamic-negative counts panic with `fill count is negative`, and zero produces an empty sequence. Stage 35 Slice 3 adds nominal Cloneable Move fills; `Set` and `Dictionary` fills remain intentionally unsupported.
- Stage 24 generic free functions, instance methods, and static methods are complete. Type arguments are inferred through the existing positional/named argument binding, with a typed declaration supplying expected-result context when arguments alone cannot resolve a parameter. MIR monomorphization emits one reachable specialization per distinct concrete argument set and deduplicates repeated uses; its kinded specialization key leaves compile-time value parameters as an additive future extension. Compiler-known constraints are checked at instantiation; Stage 35 Slice 1 adds nominal user-interface constraints and statically selected concrete method calls.
- Stage 25 generic classes are complete on the current branch under decision 0105. `Name<T>` is structurally typed and monomorphized into a distinct concrete class layout and method set per argument list, including nested instantiations. Every class instantiation remains an owned move type; substituted field types specialize its field handling and drop glue. Type parameters are invariant, while default type arguments, compile-time value arguments, and runtime generic reflection remain fenced.
- The Diagnostic Experience Foundation is implemented under decision 0108.
  Compiler diagnostics now carry stable severity/kind metadata, Title Case
  titles, multi-source labels, explanations, repeated notes/help,
  applicability-classified multi-edit fixes, cause identity, documentation, and
  protected developer details. Human/concise stderr and versioned JSON stdout
  are rendered centrally with exact-duplicate suppression. Coordinated
  language-server and website consumers use the structured model.
- The String API Decision Amendment is implemented under amended decision 0103.
  `$string->` is reserved for intrinsic measurements and views; `String::` owns
  every string-specific operation; and the former public `str_*` family is
  removed. `length` is defined in Unicode grapheme units, while `byteLength`
  reports UTF-8 bytes.
- The String API Completeness Audit Against PHP is implemented as a checked-in
  181-row inventory covering the official core string, mbstring, and current
  released grapheme capabilities.
- Andrew approved the audit's recommended path on 2026-07-31. Decision 0103's
  v1 inventory now includes the symmetric ignore-case search family,
  first-grapheme casing, and occurrence counting; every other proposed item is
  explicitly deferred under its recorded dependency or domain.
- The Minimum String Runtime Surface is implemented. Intrinsic
  `length`/`byteLength`/`isEmpty`/`bytes` and the selected trimming, casing,
  predicate, search, replacement, split/join, slice, repeat, padding, and
  `fromBytes` companion operations lower through validated MIR to the
  interpreter and the shared `doria-rt` ABI used by Cranelift and LLVM.
  Unicode 17.0 data drives locale-independent grapheme, whitespace, casing,
  and full-fold behavior. Grapheme/code-point views remain pending on the
  traversal protocol, and ordering comparisons remain pending on `Ordering`.
  The approved ignore-case search family, first-grapheme casing, and
  occurrence counting are included in this executable surface.
- The Unified Doria Diagnostic Presentation And Runtime Outcome Foundation is
  implemented under decision 0109. The compiler-owned `Diagnostic` is the sole
  public representation for compilation findings and runtime outcomes.
  Interpreter, Cranelift, LLVM, PHP compatibility, standalone executables, and
  `doriac run` preserve catalogued `P` codes, source labels, Doria call paths,
  and status-101 abort-without-cleanup semantics. Human output uses the global
  `Where`/preview/`Why` grammar and `Call Path`; concise and JSON remain
  projections of the same facts. Future unhandled checked errors are bound to
  this foundation but remain unimplemented.
- The durable manifest supports raw stdin, isolated seeded files, declared program arguments, and exact interpreter/Cranelift/LLVM stdout, stderr, status, generated-file, and class-lifetime comparison.

- The Interactive Line-Input Amendment is implemented. `read_line(string $prompt
  = ""): ?string` is one compiler-known function with an optional parameter and
  an inclusive arity range, not an overload pair. One canonical MIR operation
  owns the ordering contract — evaluate the prompt exactly once, write it
  exactly with no added newline, flush stdout, then read one line — and the
  interpreter, Cranelift, LLVM, and PHP compatibility backend all consume it.
  `read_line()` remains valid and lowers with the canonical empty-string default,
  which still performs the pre-read flush. Line discipline, EOF, and buffered
  remainders are unchanged. A closed stdout pipe during the prompt write or
  flush exits with status 0 without reading stdin; other output and read
  failures use `Doria\Std\Io\IoError`, invalid UTF-8 uses
  `Doria\Std\Io\InvalidUtf8Error`, and allocation failure remains P1206. The
  flush substrate reports the same success/broken-pipe/other-failure vocabulary
  as writes.

## Current collection and local-declaration milestone

- Stage 26 — Complete. `SortedDictionary`, `SortedSet`, min-first
  `PriorityQueue`, and ring-buffer `Deque` consume explicit validated MIR and a
  shared private runtime ABI across the interpreter, Cranelift, and LLVM. PHP
  compatibility uses generated semantic helpers rather than host collection
  ordering. Set iteration is readonly and non-consuming construction/algebra
  preserve their sources.
- Stage 26a — Complete. Grouped inferred/typed readonly/writable locals preserve
  one initializer in AST/HIR and canonical validated MIR. The initializer runs
  once, targets initialize left to right, scope insertion is atomic, Copy values
  remain independent, strings retain their immutable handle per binding, and
  move values are rejected except for explicitly typed nullable literal-null
  initialization. Interpreter, Cranelift, LLVM, and PHP compatibility agree;
  grouping creates no runtime object.
- Stage 26b — Performance Baseline Foundation — Complete. Decision 0112 is
  accepted and all three slices are complete: the manifest/provenance foundation,
  opt-in compiler phase and structural report, compiler scaling generators,
  compiler/generated/runtime/diagnostic matrix, process resource counters,
  separate Callgrind/DHAT adapters, candidate evidence, and exact structural
  baseline are in place. Slice 3 adds peer sources, the exact native acceptance
  policy, and controlled measurement and promotion workflows; timing thresholds
  still require separate review of eligible evidence, and Slice 2 makes no
  optimization claim.
  Slice 3 Part 1 is delivered: C, C++, Rust, and PHP peers for the comparative
  generated-program and runtime-subsystem cases, peer fairness and
  semantic-equivalence records enforced when the manifest loads, two controlled
  candidate sessions, and a timing threshold proposal. It accepts no threshold
  and promotes no baseline. Both Doria backends are measured. The proposal's
  finding is that seventy-nine of eighty case and target pairs are dominated by
  process startup rather than by their workload, so those cases cannot carry a
  timing threshold until their workloads are scaled. An earlier write-up
  reported `string_search` on Cranelift at 6.6 times its floor; that result is
  withdrawn. Deterministic runtime construction and full compiler/runtime
  identity now close the reproducibility defect. The exact `1.30` native
  acceptance rule is bound, and hot workloads require both five times their
  target startup floor and at least 25 ms. No controlled physical-host Linux runner or
  eligible physical-host session is currently configured, so no native
  acceptance matrix or Doria timing baseline can be promoted. Docker, WSL,
  containers, and virtual machines remain valid for engineering and correctness
  evidence, but not release claims. Controlled timing, verified affinity,
  Callgrind, DHAT, hardware counters, and cross-platform timing baselines are not
  stage gates. Missing eligible-runner evidence does not halt compiler stages.
- Stage 26b — Complete.
- Stage 26b Slice 1 — Complete.
- Stage 26b Slice 2 — Complete.
- Stage 26b Slice 3 — Complete.
- Measurement Status: Pending Available Runner.
- Decision 0113 Slice 1 — Complete.
- Decision 0113 Slice 2 — Complete.
- Decision 0113 Slice 3 — Complete.
- Decision 0113 Slice 4 — Complete.
- Decision 0113 — Complete.
- Decision 0114 — Accepted.
- Stage 27 — Complete; No Performance-Evidence Dependency.
- Stage 27 Slice 1 — Complete.
- Stage 27 Slice 2 — Complete.
- Decision 0115 — Implemented.
- Stage 28 — Complete.
- Stage 28 Slice 1 — Complete.
- Stage 28 Slice 2 — Complete.
- Decision 0116 — Accepted.
- Stage 28a — Complete.
- Stage 28a Slice 1 — Complete.
- Stage 28a Slice 2 — Complete.
- Decision 0119 — Accepted.
- Decision 0120 — Accepted; Explicit Capture-List Foundation.
- Decision 0121 — Accepted; Stage 30 Closure Authority.
- Decision 0124 — Accepted; Baton Bootstrap, Doria-Native Transition, And Toolchain Release Gate.
- Decision 0125 — Accepted And Implemented; Typed Attributes, Const-Evaluated Metadata, And Processor Protocol.
- Decision 0126 — Accepted And Implemented; Baton Schema 2 Local Package, Target, And Selection Spellings.
- Decision 0127 — Accepted And Implemented; Baton Dependency Resolution, Lockfile, Cache, And Offline Semantics.
- Decision 0128 — Accepted And Implemented; Baton Workspaces, Development, Tests, Processors, And Project Inventory.
- Stage 29 — Complete.
- Stage 29 Slice 1 — Complete.
- Stage 29 Slice 2 — Complete.
- Corrective Beat: Native Collection Property Initializers — Complete.
- Stage 29 Slice 3 — Complete.
- Corrective Beat: Inferred Main Checked Effects — Complete.
- Pre-Stage-30 Grammar Slice — Complete.
- Stage 30a Callable Grammar Completion — Complete.
- Stage 30b Semantic Function Types And Captures — Complete.
- Constructor Writable-Path And Owned-Property Corrective Beat — Complete.
- Stage 30c Ownership, Lifetime, And Escape — Complete.
- Stage 30d Closure HIR/MIR And Interpreter Oracle — Complete.
- Stage 30e Native Execution — Complete.
- Stage 30f PHP Compatibility — Complete.
- Stage 30g List Algorithms — Complete.
- Stage 30h Cross-Repository Closure — Complete.
- Stage 30 — Complete.
- E0641 — Historical And Reserved.
- Phase F Namespace, Autoload, Package, And Dependency Authority — Accepted.
- Stage 31 Slice 1 — Complete.
- Stage 31 Slice 2 — Complete.
- Stage 31 — Complete.
- Ambient I/O And Fallible Finalizer Corrective Beat — Complete.
- E0632 — Historical And Reserved.
- Stage 32 — Complete.
- Stage 33 Slice 1 — Complete.
- Stage 33 Slice 2 — Complete.
- Stage 33 Slice 3 — Complete.
- Stage 33 — Complete.
- Phase F — Complete.
- Native Testing Foundation Slice 1 — Complete.
- Native Testing Foundation Slice 2 — Complete.
- Native Testing Foundation Slice 3 — Complete.
- Native Testing Foundation — Complete.
- Stage 34 Single Class Inheritance — Complete.
- Post-Stage-34 Constructor Parameter Roles Corrective Beat — Complete.
- Post-Stage-34 Indexed Foreach And Scalar Display Corrective Beat — Complete.
- Post-Stage-34 Explicit Foreach Binding Types Corrective Beat — Complete.
- Stage 35 Interfaces And Traits — Complete; All Five Slices Complete.
  - Stage 35 Authority — Accepted.
  - Stage 35 Slice 1 Grammar, Graphs, And Conformance — Complete.
  - Stage 35 Slice 2 Interface Runtime And Ownership — Complete.
  - Stage 35 Slice 3 Core Contracts And Public Iteration — Complete.
  - Stage 35 Slice 4 Trait Composition — Complete.
  - Stage 35 Slice 5 Cross-Repository Closure — Complete.
  - Stage 35 — Complete.
- Stage 35a — Optimizer Contracts, Dispatch, And Escape Audit — Complete.
- Stage 36 Property Hooks — Complete. Grammar, concrete/interface/constrained
  call analysis, and override contract checks under Decision 0135, including
  `borrowed get`. Accessor execution components have interpreter, linked-native,
  and PHP tests, including transitive blocking/async checks. The blanket E0764
  rejection is removed, and tests exercise the public compiler pipeline.
  Backed override initializers run parent then child, replacing the inherited
  value. Initializer regressions cover replacement ownership and failed
  construction phases. The 389-example native parity manifest, PHP coverage,
  Linux/macOS/Windows CI, and native ownership leak checks pass. Compiler-owned
  accessor facts drive coordinated editor support; all eleven website hook
  examples pass Check and Run using the managed installed compiler. The compiler
  and language server are refreshed together and verified after installation-cache
  reclamation. Stage 36a follows; its remaining public spellings are deferred.
- Pre-Stage-45 Doria-Native Baton Transition — Scheduled, Mandatory Before Unsuffixed `2026.03.1`.
- Stage 36a — Complete (at this entry's date it was still in progress).

## Do not duplicate

- Stage 17 I/O and formatting work from PRs #75 and #76.
- ROADMAP-style planning outside the end-to-end plan.

## Deferred

- Runtime-initialized and owned statics until separately accepted lifetime/concurrency decisions.
- Growable/slice/search `Bytes` members until a future method-surface decision.
- Writable match and writable payload patterns are rejected for Doria v1; Decision 0134 Slice 2 executes nominal interface `is` and borrow-preserving or explicitly consuming type patterns.
- Collection `is`, plus boxing collections, typed arrays, or `Bytes` into `mixed`, until their authored implementation slices. Class subtype and nominal interface tests are implemented.
- Variadic/spread user parameters stay deferred from Stage 23a as their own slice, per decisions 0095 and 0098.
