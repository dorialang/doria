# Temporary Language Restrictions Audit

Documentation role: current implementation-boundary audit. Accepted decisions
and `SPEC.md` remain normative.

This audit applies the Decision 0122 rule that an implementation fence is not a
permanent language prohibition. It records the current owner and diagnostic
posture without implementing unrelated work.

| Source or diagnostic | Former/current wording or boundary | Original reason | Classification | Accepted authority | Implementation owner and follow-up | Diagnostic kind accurate? |
| --- | --- | --- | --- | --- | --- | --- |
| `writable function __construct` | Constructors cannot be declared writable | Construction is a lifecycle protocol over an incomplete object, not an ordinary mutable receiver borrow | Permanent Language Rejection | Decisions 0080 and 0122 | No implementation follow-up | Yes, `Language` with a removal fix |
| E0472 move-in route | Direct moves into owned properties were unsupported | Stage 19 lacked property-path ownership and replacement analysis | Historical Diagnostic | Decisions 0083 and 0122 | Move-in route removed; retain the catalogue identity | Yes only as historical; no valid move-in may emit it |
| E0472 move-out route | Direct moves out require a separate take-and-replace operation | Moving out can leave a required property uninitialized | Temporary Soundness Fence | Decision 0122 preserves object invariants and leaves move-out separate | Future owned-property extraction authority | The current language diagnostic is accurate for the unavailable operation, but its message must say move-out only |
| Constructor direct-path wording | Constructor access was described as direct-only for every write | Early stages had only direct definite-initialization facts | Stale Restriction Now Provably Safe | Decisions 0090 and 0122 | Corrected by the construction-root capability | No blanket rejection remains valid |
| Nested constructor writable paths | A path beginning at readonly `$this` emitted E0201 | Receiver mutability was represented as one boolean | Stale Restriction Now Provably Safe | Decision 0122 | One semantic path-capability query now preserves construction-root provenance | E0201 remains correct only at the first readonly path segment |
| Owned property initialization | Move values could not initialize explicit owning properties | Ownership transfer was implemented only for promotion and locals | Stale Restriction Now Provably Safe | Decision 0122 | Property-write ownership transfer is implemented | E0472 is inaccurate and unreachable here |
| Owned writable-property replacement | Writable Move properties could not be reassigned | Backends could not distinguish initialization from replacement | Stale Restriction Now Provably Safe | Decisions 0081 and 0122 | Typed MIR carries acquire-before-drop replacement | E0472 is inaccurate and unreachable here |
| Stage 19 native-eligibility gate | Construction required narrow statically preinitialized shapes | Full constructor CFG/dataflow had not landed | Historical Diagnostic | Decisions 0083 and 0090 | Removed at Stage 21 | No active user-facing diagnostic route |
| PHP ownership limitations | Some accepted ownership/lifecycle behavior cannot be represented faithfully in PHP | Host runtime has different deterministic ownership semantics | Accepted But Not Implemented | Native-first decisions and backend capability diagnostics | PHP compatibility backend; reject before emission where parity is impossible | Yes when `UnsupportedDevelopmentSurface` and development-only |
| E0641 closure target boundary | Supported plain closures formerly stopped before complete backend routing | Explicit Doria capture, ownership, storage, and backend lowering had to be complete | Historical and reserved after Stage 30h; no active emitter or generic fallback remains | Decision 0121 | Complete | No valid accepted closure route emits it; precise capability diagnostics remain independent |
| `List<T>` higher-order methods | Compiler-known callback contracts formerly had no executable collection traversal | Closure ownership, exact effects, indirect calls, and partial-result cleanup had to exist first | Implemented for `map`, Copy-only `filter`, and writable-accumulator `reduce`; other collection families remain outside this contract | Decision 0121 | Complete | No valid supported List algorithm emits E0641; contract errors use compiler-owned E0664-E0668 identities |
| Shared-ownership pending payloads and PHP support | Some shared payloads and all PHP shared execution remain unavailable | Required access surface or host parity is not implemented | Accepted But Not Implemented | Decision 0106 | Scheduled shared/runtime or PHP compatibility work | Yes when stage-named and development-only |
| `mixed` collection/aggregate boxing boundaries | Several values cannot yet enter the boxed runtime carrier | Box layout/drop paths are not complete for those families | Accepted But Not Implemented | Stage 23 mixed authority | Future mixed-runtime extension | Yes when stage-named and development-only |
| Collection `Cloneable` boundaries | Move-value sequence fills and duplication are unavailable | No public explicit-duplication contract exists yet | Accepted But Not Implemented | Decisions 0100/0113 and planned Stage 35 `Cloneable` | Stage 35 | Yes when the diagnostic names the missing capability rather than invalid syntax |
| General property move-out | No source operation leaves a property hole or atomically replaces it | Writable path, alias, and object-invariant semantics are unsettled | Open Design Question | Decision 0122 | Separate take-and-replace/swap authority | Current rejection is required; do not describe move-in as equally unresolved |
| `Doria\Std\Test` expectation members | `expect`, `fail`, `AssertionError`, and `TestAssertion` remain unavailable while `describe`/`it`/`test` execute | Assertion ownership, structured outcomes, cleanup, and backend parity belong to the next bounded slice | Accepted But Not Implemented | Decision 0129 | Native Testing Foundation Slice 2 | Yes when one stage-named E0710 boundary is emitted; do not reject delivered behavioral declarations |

## Standing Review Rule

A permanent language rejection must cite accepted language authority. An
accepted-but-unimplemented feature must be described as an implementation
boundary, not as invalid Doria. Historical diagnostics remain catalogued but
must have no valid source route.
