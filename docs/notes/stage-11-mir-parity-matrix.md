# Stage 11 MIR Parity Matrix

Documentation role: working note.

Source of truth for sequencing remains `docs/doria-end-to-end-plan.md`. This matrix tracks current MIR/debug parity with existing Stage <=10 native smoke examples so `NativeSmokeModule` retirement can be planned deliberately. It is evidence tracking, not a roadmap or a claim that Stage 11 is complete.

`Covered` means both paths have an example or focused test for the Doria-visible behavior. `Blocker` means the gap must be closed or explicitly removed from the accepted native surface before `NativeSmokeModule` can retire.

| Feature / example | Existing native smoke support | MIR/debug support | Status | Notes |
| --- | --- | --- | --- | --- |
| `main(): int` literal return | `examples/native/main_return_42.doria` | `examples/debug/main_return_42.doria` | Covered | Both produce status 42. |
| `main(): void` fallthrough | `examples/native/main_void_empty.doria` | `examples/debug/main_void_empty.doria` | Covered | Both map normal completion to status 0. |
| String-literal echo | `examples/native/main_void_hello.doria` | `examples/debug/main_void_hello.doria` | Covered | Exact bytes, no implicit newline. |
| Readonly and writable int locals | `main_readonly_local` / `main_writable_local_42` | `main_int_local_42` / writable-local debug fixtures | Covered | MIR uses typed local slots. |
| Int arithmetic `+`, `-`, `*` | Native arithmetic fixtures | Stage 11b MIR arithmetic tests | Covered | Checked `int64` behavior is tested on both paths. |
| `if` / `else if` / `else` | Native structured-if fixtures | Stage 11c debug fixtures | Covered | Includes returning and fallthrough branches. |
| `while` | `examples/native/main_structured_while_42.doria` | `examples/debug/main_while_count_42.doria` | Covered | Debug execution remains fuel bounded. |
| `break` / `continue` | `examples/native/main_break_continue_42.doria` | `main_while_break_42` / `main_while_continue_6` | Covered | MIR also covers nested loop targeting. |
| Traditional `for` | `examples/native/main_for_42.doria` | `examples/debug/main_for_count_10.doria` | Covered | MIR preserves the increment block on `continue`. |
| Integer range `foreach`, exclusive | `examples/native/main_foreach_range_45.doria` | `examples/debug/main_foreach_range_exclusive_10.doria` | Covered | End value is excluded. |
| Integer range `foreach`, inclusive | `examples/native/main_foreach_range_55.doria` | `examples/debug/main_foreach_range_inclusive_11.doria` | Covered | Terminal overflow guard is tested. |
| Top-level helper function add | `examples/native/main_function_add_42.doria` | `examples/debug/main_function_add_42.doria` | Covered | Positional int parameters and int return. |
| Top-level helper function loop | `examples/native/main_function_loop_42.doria` | `examples/debug/main_function_loop_42.doria` | Covered | Helper-local mutation and loop control. |
| Void helper echo | `examples/native/main_function_echo_hello.doria` | `examples/debug/main_function_echo_hello.doria` | Covered | Shared stdout preserves call order. |
| Readonly string-local echo | `examples/native/main_string_local_hello.doria` | `examples/debug/main_string_local_hello.doria` | Covered | Stage 11g typed readonly string slots. |
| String concat echo | `examples/native/main_string_concat_hello.doria` | `examples/debug/main_string_concat_hello.doria` | Covered | Literal/local `.` operands retain source order. |
| String concat inside void helper | Native backend integration fixture | `examples/debug/main_function_string_local_echo.doria` | Covered | Focused tests assert `Hello Doria!`. |
| String echo in branches and loops | Native string control-flow fixtures | `main_string_if_echo` / `main_string_loop_echo_xxx` | Covered | Exact stdout is preserved across blocks. |
| Process exit boundary on `main(): int` | Native status-boundary tests | MIR interpreter status-boundary tests | Covered | Portable accepted range remains `0..125`. |
| Helper int return is not process bounded | Native big-helper test | `examples/debug/main_function_big_int_helper.doria` | Covered | Helper values retain the full Doria `int` range. |
| String echo inside int-returning functions | Accepted by current native smoke fixtures | Stage 11g permits string echo only in void functions | Blocker | Decide in a later parity slice whether MIR adopts this existing surface. |
| Cranelift lowering source | `NativeSmokeModule` | No Cranelift-from-MIR lowering yet | Blocker | Stage 11h owns the bridge seed. |
| Full differential example harness | Native integration suite exists | Focused parity tests exist | Blocker | Stage 11i must run the complete accepted matrix across both paths. |

## Retirement Gate

`NativeSmokeModule` must remain until every blocker above is resolved, Cranelift consumes MIR for the accepted subset, and the differential suite proves identical exit status and stdout bytes for the complete Stage <=10 example matrix. New capability work continues through MIR/interpreter; this note does not authorize expanding the smoke module.
