# Stage 20 Property Increment Gap

> Documentation role: working note.
> This file records a verified compiler gap and corrects a misreading of an
> early decision record. It is not an accepted language decision. The roadmap
> owner remains `docs/doria-end-to-end-plan.md`.

## Finding

`$this->value++;` (statement position, writable property place) is a **decided,
in-scope Stage 20 deliverable that is currently unimplemented**. The compiler
rejecting it is a completeness gap, not enforcement of a language rule.

Plan §3.2 and the Stage 20 stage entry both assign property read-modify-write to
Stage 20:

- §3.2: "Read-modify-write works on any writable place, not just locals:
  `$this->value++`, `$counter->value += 2` ... Decision 0034's
  writable-local-only restriction is a **Stage 9 scope artifact from before
  properties existed, not a design decision**."
- Stage 20 entry: "**Read-modify-write on property places lands here** (§3.2):
  `$this->value++`, `$obj->prop += 1` — same operation as the property
  assignment this stage already lowers, with sugar."

## Correction to the earlier assessment

An assessment cited `0034-stage-9-mvp-iteration-syntax.md` ("`++`/`--` on
properties or indexed expressions") as evidence that property increment is
unsupported by design, and concluded the website fixture and docs should switch
to `$this->value = $this->value + 1;`.

That reverses the plan. The cited line sits under 0034's **"Stage 9 does not
add:"** non-goals — a scope snapshot of what Stage 9 shipped, not a standing
prohibition. §3.2 names that exact restriction and repudiates it. The fixture
and docs are written to the language design; the compiler is the lagging party.
Correct remedy: implement property increment lowering, not downgrade the
examples.

Reading rule this exercises (see `session-handoff.md` §3): when an agent cites an
early-stage record as a constraint, ask whether it *decided* something or merely
*described what existed*. 0034 described Stage 9.

## Verified current behavior

`doriac` on the current branch (`check` = semantic, `run` = interpreter +
Cranelift + LLVM):

| Form | Result | Where it stops |
|-------------------------------------|-----------|----------------------------------------|
| `$this->value = $this->value + 1;`  | works     | —                                      |
| `$this->value += 1;`                | build err | `M1101` at `mir_lowering.rs` (compound to property) |
| `$this->value++;`                   | check err | `E0204` at `semantics.rs` (increment target)        |

Note the divergence: `+=` passes `doriac check` but fails `doriac run`. The
semantic layer accepts a form the lowering rejects, so `check` reports a false OK
for compound property assignment.

## Fix scope

No new MIR node and no new codegen are required. Property **read**
(`Operand::Property`, `mir.rs`) and property **write**
(`Statement::AssignProperty`, `mir.rs`) already exist and codegen in all three
backends — property assignment works today, and the `borrowed_counter` fixture
already reads `$counter->value` in its `while` condition. The gap is front-end
wiring only:

1. `semantics.rs::check_increment_target` — add a `PropertyAccess` arm (and a
   `StaticMember` arm for writable statics) mirroring
   `check_assignment_target`'s place validation (E0201 readonly receiver, E0202
   readonly property, E0423 non-numeric). Factor a shared writable-scalar-place
   check so the two paths cannot drift.
2. `mir_lowering.rs::lower_increment` — for a property place, use
   `lower_property_place` to get `(object, property, ty)`, build the value from
   an `Operand::Property` read ± 1, and push `AssignProperty` instead of routing
   through `lower_assignment_target` (which only accepts locals).
3. `mir_lowering.rs::lower_assignment` — replace the `M1101` rejection of
   compound property assignment with the same read-`Operand::Property` → binary
   op → `AssignProperty` lowering. §3.2 lists `$counter->value += 2` beside
   `$this->value++`; both land together.

Out of scope (unchanged): value-producing `++`/`--` expressions (§3.2 keeps these
future work, statement position only); indexed places `$items[0]++` (Stage 23).

## Invalidated elsewhere

- Website: `PlaygroundService.php` `borrowed_counter` fixture and
  `src/Docs/Content/classes/properties-and-methods.md` are **correct as written**
  and need no change; they compile once the feature lands.
- The executable-examples test only checks textual shape and would not have
  caught this drift. A test that compiles the curated examples through `doriac`
  belongs in the website repo (it would have surfaced the compiler gap, which is
  the right outcome). Tracked separately from this compiler change.

## Adjacent hygiene (not blocking)

- `E0204` is overloaded across "unsupported increment target", "unsupported
  assignment target", and "must be a writable class value". Splitting codes would
  make diagnostics and tests unambiguous.
