# Stage 25a final integration audit

> Documentation role: mechanical closure evidence for Decision 0106. This note
> records implementation coverage; the decision remains the semantic authority.

The audit uses only these status values: **Implemented And Verified**,
**Implementation Gap**, **Documentation Gap**, **Tooling Gap**, and
**Intentionally Deferred By Accepted Authority**. A row is verified only when
its cited executable or diagnostic coverage reaches every applicable native
backend.

Abbreviations: `S25a` means `crates/doriac/tests/stage25a_tests.rs`; `parity`
means the durable manifest plus the matching `native_io` sidecar; `RT` means
`crates/doria-rt/src/lib.rs` tests; `NA` means the column is not applicable to
that requirement.

| Decision 0106 requirement | Implementation location | Frontend coverage | MIR coverage | Interpreter coverage | Cranelift coverage | LLVM coverage | Runtime coverage | Diagnostic coverage | LSP coverage | Editor coverage | Website coverage | Documentation coverage | Leak or lifetime coverage | Status |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Exactly six canonical move types, one type argument each | `types.rs::SharedHandleKind::ALL` | S25a arity/redeclaration tests | Typed handle variants | S25a | S25a | S25a with feature | Separate handle values | E0546/E0547 | Six type hovers exist | VS Code/IntelliJ type lists | API reference exists | Decision 0106 | Move/drop fixtures | Implemented And Verified |
| `shared new T()` creates readonly family; plain `new` stays owned | parser, semantics, MIR lowering | S25a construction tests | `SharedReferenceExpression::New` | readonly parity | parity | parity | readonly control | E0540-E0544 | No-false diagnostics | `shared` keyword | shared example | SPEC/0106 | readonly leak job | Implemented And Verified |
| Writable constructor takes one owned payload, including named `value:` | semantics and named-call binding | S25a constructor tests | writable shared new | writable fixtures | parity | parity | writable control | E0544/use-after-move | compiler projection | type highlighting | writable example | SPEC/0106 | writable leak jobs | Implemented And Verified |
| Weak and access objects are not directly constructible | semantics | S25a rejection tests | no constructor MIR | NA | NA | NA | no constructors exported | E0543 | compiler diagnostics | NA | NA | 0106 | NA | Implemented And Verified |
| Readonly family accepts class payloads only | type resolver/specialization | S25a concrete and symbolic tests | class payload only | readonly fixture | parity | parity | readonly class drop glue | E0545 | compiler diagnostics | type highlighting | docs example uses class | SPEC/stdlib/0106 | readonly lifecycle | Implemented And Verified |
| Writable family executes class, generic class, `T[]`, `List`, `Dictionary`, `Set`, and `Bytes` | specialization, access forwarding | S25a and domain fixture | typed writable payload variants | domain parity | domain parity | domain parity | collection/bytes/class drop glue | payload diagnostics | No-false diagnostics | type highlighting | writable examples | SPEC/stdlib/0106 | both profile leak lists | Implemented And Verified |
| Scalar/string writable payload execution remains deferred | MIR capability boundary | accepted type spelling | lowering refusal | NA | NA | NA | no invented representation | stage-named M1102 | compiler projection | type highlighting | not advertised executable | 0106/current pipeline | NA | Intentionally Deferred By Accepted Authority |
| Shared handles through `mixed` remain deferred | MIR capability boundary | S25a mixed test | lowering refusal | NA | NA | NA | no mixed tag | stage-named M1102 | compiler projection | NA | not advertised executable | 0106/current pipeline | NA | Intentionally Deferred By Accepted Authority |
| Readonly property/method forwarding preserves ownership modes | semantics/ownership/MIR lowering | S25a forwarding/take tests | payload borrow operands | readonly parity | parity | parity | payload pointer lookup | write/take rejection | compiler projection | property/method scopes | readonly example | 0106/stdlib | readonly leak job | Implemented And Verified |
| `referencedValue` resolves wrapper collisions as a readonly place | semantics and MIR lowering | S25a collision/rejection tests | shared payload projection | collision parity | collision parity | collision parity | refcount-neutral payload lookup test | E0201/E0472/E0548 | receiver completion and concrete hover | VS Code/IntelliJ property fixtures | executable collision example | 0106/stdlib | collision leak job | Implemented And Verified |
| Weak acquisition preserves family and expires after final owner | handle typing and MIR | S25a family tests | nullable family-specific acquire | lifecycle parity | parity | parity | strong/weak control tests | family-crossing diagnostics | compiler projection | canonical types | weak examples | 0106/stdlib | lifecycle leak jobs | Implemented And Verified |
| Weak back-reference breaks a cycle without collection | weak-cycle fixture | accepted source | writable weak/strong graph | weak-cycle parity | weak-cycle parity | weak-cycle parity | final strong then weak release | NA | compiler-projected no-false diagnostics | canonical types | executable weak-cycle example | 0106/stdlib | weak-cycle leak jobs | Implemented And Verified |
| All writable handles observe one per-allocation access state | writable control pointer | S25a multi-handle tests | access acquire/release | writable parity | parity | parity | RT per-allocation assertions | P1501 | compiler projection | NA | conflict example exists | 0106 | access lifecycle jobs | Implemented And Verified |
| Many readonly accesses coexist; access can switch after drop | access-object semantics | S25a lexical/access tests | ordered acquire/drop | stress parity | stress parity | stress parity | RT 128-access stress | conflict absence | compiler projection | NA | lexical example | 0106 | stress leak job | Implemented And Verified |
| Three access conflicts remain distinguishable | central diagnostic catalogue | structured conflict test | source-aware panic | exact P1501 facts | exact P1501 facts | exact P1501 facts | V2 typed fact transport | three exact reasons and source spans | compiler ranges remain UTF-16-safe | NA | P1501 structured-fact rendering test | 0106/0109 | abort fixtures excluded from leaks | Implemented And Verified |
| Access objects own one claim and release access before strong ownership | ownership/drop lowering | S25a movement/storage tests | distinct access cleanup | access fixtures | parity | parity | RT final-owner test | moved-from diagnostics | compiler projection | canonical types | writable example | AGENTS/0106 | access leak jobs | Implemented And Verified |
| Access objects return, pass, store, move, and keep payload alive | semantics/ownership/MIR | S25a access fixtures | value/property/collection slots | access parity | parity | parity | access claim | E0470/use-after-move | No-false diagnostics | NA | access example | 0106 | access lifetime leak jobs | Implemented And Verified |
| Readonly/writable access forwarding follows receiver capability | semantics member/index resolution | S25a forwarding tests | direct payload place | writable/readonly fixtures | parity | parity | shared access payload APIs | E0201/E0203 | receiver-kind completion filters | property/method scopes | writable example | 0106 | writable leak jobs | Implemented And Verified |
| Families are disjoint in all conversions and weak acquisition | type assignability/specialization | S25a assignment/argument/weak tests | distinct MIR variants | family fixtures | parity | parity | separate controls/release APIs | canonical family diagnostics | compiler projection | canonical types | docs only | AGENTS/0106 | separate lifecycle tests | Implemented And Verified |
| Generic functions/classes preserve family and payload specialization | monomorphization | S25a generic tests | concrete specialized payloads | generic parity | parity | parity | concrete drop glue | full canonical names | generic no-false diagnostics | canonical types | generic example | plan/0106 | generic domain leak job | Implemented And Verified |
| Handles/access objects compose through accepted storage positions | ownership and collection lowering | S25a property/array/dictionary/list tests | typed owned slots | storage parity | parity | parity | one claim per stored value | move/replacement diagnostics | compiler diagnostics | canonical types | examples | 0106 | stored-access leak job | Implemented And Verified |
| Ordinary borrows remain compile-time-only beside shared ownership | borrow checker and MIR | S25a coexistence tests | no ordinary runtime access operations | mixed fixtures | parity | parity | access counters only in writable control | Doria borrow vocabulary | compiler projection | NA | docs | AGENTS/0106 | control-layout assertion | Implemented And Verified |
| Readonly and writable controls remain separate, non-atomic shapes | `doria-rt` controls/native ABI | NA | distinct runtime calls | RT/model | ABI parity | ABI parity | layout/count/stress tests | P1502-P1505 | NA | NA | NA | 0106 | direct lifetime assertions | Implemented And Verified |
| Count arithmetic is checked; payload drops once; final weak frees control | `doria-rt` retain/release | NA | catalogued calls | RT/model | native fixtures | native fixtures | overflow/lifetime tests | P1502-P1505 | NA | NA | NA | 0106 | Valgrind lists | Implemented And Verified |
| PHP refuses rather than emulates shared ownership | backend capability validation | supported source parses/checks | no PHP lowering | NA | NA | NA | NA | structured backend limitation | compiler projection | NA | compatibility guidance | SPEC/0106 | NA | Implemented And Verified |
| Bounded stress preserves strong, weak, and access counts | stress fixture and RT test | stress source checks | collection handle/access slots | stress parity | stress parity | stress parity | exact 64/128 bookkeeping | NA | compiler-projected diagnostics | canonical highlighting | not needed publicly | this audit | stress leak jobs | Implemented And Verified |
| LSP offers receiver-aware members and exact shared-ownership hovers | language-server analysis | compiler owns diagnostics | NA | NA | NA | NA | NA | ranges/facts projected | semantic receiver contexts, wrapper precedence, and incomplete-accessor tests | NA | NA | 0106 | NA | Implemented And Verified |
| Editors highlight `shared`, six types, and `referencedValue` consistently | TextMate/IntelliJ lexers | NA | NA | NA | NA | NA | NA | NA | NA | shared fixture, guard, and lexer tests | NA | 0106 | NA | Implemented And Verified |
| Website/Playground runs final examples and structured P1501 outcomes | website Playground service | exact compiler revision pin | shared compiler MIR | debug target | native target | release target | compiler runtime facts | structured outcome | NA | syntax grammar | Slice 4 metadata, weak-cycle/collision examples, P1501 fact test | shared ownership and prompted-input guides | NA | Implemented And Verified |
| Installed compiler and LSP report the final Slice 4 compiler commit | refresh script/version JSON | final binaries | NA | installed compiler | installed compiler | installed compiler | installed runtime | installed diagnostics | installed LSP | editor uses installed LSP | Playground independent | toolchain docs | installed fixture runs | Implemented And Verified |
| Stage 25a authorities state complete and Stage 26 next without widening deferrals | plan/pipeline/decision/spec | NA | NA | NA | NA | NA | NA | NA | NA | NA | closure edits complete | plan, pipeline, SPEC, README, stdlib, and 0106 | NA | Implemented And Verified |

## Runtime measurements

Final sizes are recorded after the release build in the closure report. The
layout test fixes the architectural relationship mechanically: on a 64-bit
target the readonly control is 32 bytes, the writable control is 48 bytes, and
an access value is one 8-byte control pointer. Ordinary class payloads remain
headerless.

## Intentional deferrals

- writable shared scalar and string payload execution;
- shared handles through `mixed`;
- atomic/thread-safe shared ownership and cross-thread transfer;
- Stage 31 qualified core names;
- PHP compatibility execution of shared ownership;
- tracing cycle collection (strong cycles intentionally leak).

These are accepted boundaries, not unfinished Slice 4 work.
