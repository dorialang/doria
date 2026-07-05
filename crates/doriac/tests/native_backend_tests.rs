use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use doriac::backend::BackendTarget;

#[test]
fn compiles_and_runs_current_native_smoke_examples() {
    if !host_linker_is_available() {
        eprintln!(
            "native smoke test unavailable: host linker `{}` was not found",
            host_linker()
        );
        return;
    }

    let workspace = workspace_root();
    let cases = [
        (
            "main_return_zero",
            "examples/native/main_return_zero.doria",
            0,
        ),
        (
            "main_void_empty",
            "examples/native/main_void_empty.doria",
            0,
        ),
        ("main_void_return", "inline_main_void_return.doria", 0),
        ("main_return_42", "examples/native/main_return_42.doria", 42),
        ("main_return_125", "inline_main_return_125.doria", 125),
        (
            "main_return_arithmetic_literal",
            "inline_main_return_arithmetic_literal.doria",
            42,
        ),
        (
            "main_readonly_local",
            "examples/native/main_readonly_local.doria",
            42,
        ),
        (
            "main_typed_readonly_local",
            "inline_main_typed_readonly_local.doria",
            42,
        ),
        (
            "main_unused_large_local",
            "inline_main_unused_large_local.doria",
            0,
        ),
        (
            "main_arithmetic_42",
            "examples/native/main_arithmetic_42.doria",
            42,
        ),
        (
            "main_return_arithmetic_42",
            "examples/native/main_return_arithmetic_42.doria",
            42,
        ),
        (
            "main_return_arithmetic_locals",
            "inline_main_return_arithmetic_locals.doria",
            42,
        ),
        (
            "main_return_product_arithmetic",
            "inline_main_return_product_arithmetic.doria",
            42,
        ),
        (
            "main_return_grouped_arithmetic",
            "inline_main_return_grouped_arithmetic.doria",
            42,
        ),
        (
            "main_stage_2c_arithmetic_local",
            "inline_main_stage_2c_arithmetic_local.doria",
            42,
        ),
        (
            "main_local_to_local",
            "inline_main_local_to_local.doria",
            42,
        ),
        (
            "main_negative_unused_local",
            "inline_main_negative_unused_local.doria",
            0,
        ),
        (
            "main_unused_arithmetic_126",
            "inline_main_unused_arithmetic_126.doria",
            0,
        ),
        (
            "main_if_else_42",
            "examples/native/main_if_else_42.doria",
            42,
        ),
        ("main_if_42", "examples/native/main_if_42.doria", 42),
        ("main_if_true_42", "inline_main_if_true_42.doria", 42),
        ("main_if_false_42", "inline_main_if_false_42.doria", 42),
        (
            "main_guard_if_false_fallback_42",
            "inline_main_guard_if_false_fallback_42.doria",
            42,
        ),
        (
            "main_if_less_than_local",
            "inline_main_if_less_than_local.doria",
            42,
        ),
        (
            "main_if_large_local",
            "inline_main_if_large_local.doria",
            42,
        ),
        (
            "main_boolean_condition_42",
            "examples/native/main_boolean_condition_42.doria",
            42,
        ),
        ("main_if_not_false", "inline_main_if_not_false.doria", 42),
        ("main_if_bang_false", "inline_main_if_bang_false.doria", 42),
        (
            "main_if_true_and_true",
            "inline_main_if_true_and_true.doria",
            42,
        ),
        (
            "main_if_true_symbol_and_true",
            "inline_main_if_true_symbol_and_true.doria",
            42,
        ),
        (
            "main_if_false_or_true",
            "inline_main_if_false_or_true.doria",
            42,
        ),
        (
            "main_if_false_symbol_or_true",
            "inline_main_if_false_symbol_or_true.doria",
            42,
        ),
        (
            "main_if_true_xor_false",
            "inline_main_if_true_xor_false.doria",
            42,
        ),
        (
            "main_if_comparison_and_comparison",
            "inline_main_if_comparison_and_comparison.doria",
            42,
        ),
        (
            "main_if_comparison_or_false",
            "inline_main_if_comparison_or_false.doria",
            42,
        ),
        (
            "main_if_comparison_xor_comparison",
            "inline_main_if_comparison_xor_comparison.doria",
            42,
        ),
        (
            "main_terminal_if_else_false_or_true",
            "inline_main_terminal_if_else_false_or_true.doria",
            42,
        ),
        (
            "main_writable_local_42",
            "examples/native/main_writable_local_42.doria",
            42,
        ),
        (
            "main_typed_writable_sub_assign",
            "inline_main_typed_writable_sub_assign.doria",
            42,
        ),
        (
            "main_writable_assign",
            "inline_main_writable_assign.doria",
            42,
        ),
        (
            "main_writable_assign_from_multiply",
            "inline_main_writable_assign_from_multiply.doria",
            42,
        ),
        (
            "main_writable_if_condition",
            "inline_main_writable_if_condition.doria",
            42,
        ),
        (
            "main_large_writable_reassigned_zero",
            "inline_main_large_writable_reassigned_zero.doria",
            0,
        ),
        (
            "main_exit_boundary_reassigned_zero",
            "inline_main_exit_boundary_reassigned_zero.doria",
            0,
        ),
        (
            "main_structured_if_42",
            "examples/native/main_structured_if_42.doria",
            42,
        ),
        (
            "main_branch_local_declaration_return",
            "inline_main_branch_local_declaration_return.doria",
            42,
        ),
        (
            "main_branch_assignment_return",
            "inline_main_branch_assignment_return.doria",
            42,
        ),
        (
            "main_else_if_structured_branch",
            "inline_main_else_if_structured_branch.doria",
            42,
        ),
        (
            "main_nested_if_structured_branch",
            "inline_main_nested_if_structured_branch.doria",
            42,
        ),
        (
            "main_multiple_guard_if_returns",
            "inline_main_multiple_guard_if_returns.doria",
            42,
        ),
        (
            "main_branch_large_reassigned_42",
            "inline_main_branch_large_reassigned_42.doria",
            42,
        ),
        (
            "main_if_fallthrough_42",
            "examples/native/main_if_fallthrough_42.doria",
            42,
        ),
        (
            "main_if_fallthrough_else_42",
            "inline_main_if_fallthrough_else_42.doria",
            42,
        ),
        (
            "main_if_fallthrough_else_if_42",
            "inline_main_if_fallthrough_else_if_42.doria",
            42,
        ),
        (
            "main_nested_if_fallthrough_42",
            "inline_main_nested_if_fallthrough_42.doria",
            42,
        ),
        (
            "main_while_then_if_fallthrough_42",
            "inline_main_while_then_if_fallthrough_42.doria",
            42,
        ),
        (
            "main_if_fallthrough_large_reassigned_42",
            "inline_main_if_fallthrough_large_reassigned_42.doria",
            42,
        ),
        (
            "main_if_fallthrough_shadow_preserves_outer",
            "inline_main_if_fallthrough_shadow_preserves_outer.doria",
            1,
        ),
        (
            "main_if_fallthrough_shadow_preserves_pre_shadow_assignment",
            "inline_main_if_fallthrough_shadow_preserves_pre_shadow_assignment.doria",
            2,
        ),
        ("main_while_42", "examples/native/main_while_42.doria", 42),
        (
            "main_structured_while_42",
            "examples/native/main_structured_while_42.doria",
            42,
        ),
        (
            "main_while_decrement_42",
            "inline_main_while_decrement_42.doria",
            42,
        ),
        (
            "main_while_multiply_42",
            "inline_main_while_multiply_42.doria",
            42,
        ),
        (
            "main_while_step_local_42",
            "inline_main_while_step_local_42.doria",
            42,
        ),
        (
            "main_while_inside_if_42",
            "inline_main_while_inside_if_42.doria",
            42,
        ),
        (
            "main_while_local_above_exit_boundary_returns_zero",
            "inline_main_while_local_above_exit_boundary_returns_zero.doria",
            0,
        ),
        (
            "main_while_rhs_uses_ordered_body_state",
            "inline_main_while_rhs_uses_ordered_body_state.doria",
            42,
        ),
        (
            "main_while_body_local_42",
            "inline_main_while_body_local_42.doria",
            42,
        ),
        (
            "main_while_body_writable_local_42",
            "inline_main_while_body_writable_local_42.doria",
            42,
        ),
        (
            "main_while_body_if_42",
            "inline_main_while_body_if_42.doria",
            42,
        ),
        (
            "main_while_body_if_empty_then_branch_42",
            "inline_main_while_body_if_empty_then_branch_42.doria",
            42,
        ),
        (
            "main_while_body_if_empty_else_branch_42",
            "inline_main_while_body_if_empty_else_branch_42.doria",
            42,
        ),
        (
            "main_while_body_shadow_preserves_outer",
            "inline_main_while_body_shadow_preserves_outer.doria",
            0,
        ),
        (
            "main_while_body_shadow_preserves_pre_shadow_assignment",
            "inline_main_while_body_shadow_preserves_pre_shadow_assignment.doria",
            2,
        ),
        (
            "main_while_break_shadow_preserves_outer",
            "inline_main_while_break_shadow_preserves_outer.doria",
            0,
        ),
        (
            "main_while_continue_shadow_preserves_outer",
            "inline_main_while_continue_shadow_preserves_outer.doria",
            0,
        ),
        (
            "main_while_if_break_shadow_preserves_outer",
            "inline_main_while_if_break_shadow_preserves_outer.doria",
            0,
        ),
        (
            "main_while_if_continue_shadow_preserves_outer",
            "inline_main_while_if_continue_shadow_preserves_outer.doria",
            0,
        ),
        (
            "main_break_continue_42",
            "examples/native/main_break_continue_42.doria",
            42,
        ),
        (
            "main_while_break_42",
            "inline_main_while_break_42.doria",
            42,
        ),
        (
            "main_while_continue_skips_remaining_body_42",
            "inline_main_while_continue_skips_remaining_body_42.doria",
            42,
        ),
        (
            "main_while_if_else_break_42",
            "inline_main_while_if_else_break_42.doria",
            42,
        ),
        (
            "main_while_if_continue_42",
            "inline_main_while_if_continue_42.doria",
            42,
        ),
        (
            "main_while_true_break_42",
            "inline_main_while_true_break_42.doria",
            42,
        ),
        (
            "main_while_continue_then_unreachable_assignment_42",
            "inline_main_while_continue_then_unreachable_assignment_42.doria",
            42,
        ),
        ("main_for_42", "examples/native/main_for_42.doria", 42),
        (
            "main_for_pre_increment_42",
            "inline_main_for_pre_increment_42.doria",
            42,
        ),
        (
            "main_for_decrement_42",
            "inline_main_for_decrement_42.doria",
            42,
        ),
        (
            "main_foreach_range_45",
            "examples/native/main_foreach_range_45.doria",
            45,
        ),
        (
            "main_foreach_range_55",
            "examples/native/main_foreach_range_55.doria",
            55,
        ),
        (
            "main_foreach_range_break_42",
            "inline_main_foreach_range_break_42.doria",
            42,
        ),
        (
            "main_foreach_range_continue_42",
            "inline_main_foreach_range_continue_42.doria",
            42,
        ),
    ];

    for (stem, source, expected_code) in cases {
        let output = temp_executable_path(stem);

        if source.ends_with(".doria") && source.starts_with("examples/") {
            compile_native_file(&workspace.join(source), &output);
        } else {
            compile_native_source(native_smoke_source(stem), &output);
        }

        let run = Command::new(&output)
            .status()
            .expect("native executable should run");
        assert_eq!(run.code(), Some(expected_code), "{stem}");

        let _ = fs::remove_file(output);
    }
}

#[test]
fn compiles_and_runs_void_main_string_literal_echo() {
    if !host_linker_is_available() {
        eprintln!(
            "native stdout smoke test unavailable: host linker `{}` was not found",
            host_linker()
        );
        return;
    }

    let workspace = workspace_root();
    let hello_output = temp_executable_path("main_void_hello");
    compile_native_file(
        &workspace.join("examples/native/main_void_hello.doria"),
        &hello_output,
    );
    assert_native_run_output(&hello_output, "main_void_hello", b"Hello Doria!");
    let _ = fs::remove_file(hello_output);

    let string_local_output = temp_executable_path("main_string_local_hello");
    compile_native_file(
        &workspace.join("examples/native/main_string_local_hello.doria"),
        &string_local_output,
    );
    assert_native_run_output(
        &string_local_output,
        "main_string_local_hello",
        b"Hello Doria!",
    );
    let _ = fs::remove_file(string_local_output);

    for (stem, expected_stdout) in [
        ("main_void_multiple_echo", b"Hello Doria!".as_slice()),
        ("main_void_empty_echo", b"".as_slice()),
        (
            "main_void_typed_string_local_echo",
            b"Hello Doria!".as_slice(),
        ),
        (
            "main_void_multiple_string_locals_echo",
            b"Hello Doria!".as_slice(),
        ),
        (
            "main_void_string_local_plus_literal_echo",
            b"Hello Doria!".as_slice(),
        ),
        (
            "main_void_grouped_string_local_echo",
            b"Hello Doria!".as_slice(),
        ),
        ("main_void_string_local_guard_skip", b"".as_slice()),
        (
            "main_void_branch_string_shadowing_echo",
            b"innerouter".as_slice(),
        ),
        ("main_void_loop_body_string_local_echo", b"HiHi".as_slice()),
        ("main_void_guard_true_return", b"".as_slice()),
        ("main_void_guard_false_return", b"".as_slice()),
        ("main_void_guard_true_skips_echo", b"".as_slice()),
        (
            "main_void_guard_false_reaches_echo",
            b"Hello Doria!".as_slice(),
        ),
        ("main_void_else_if_final_true_returns", b"".as_slice()),
        (
            "main_void_else_if_final_false_falls_through",
            b"".as_slice(),
        ),
    ] {
        let output = temp_executable_path(stem);
        compile_native_source(native_smoke_source(stem), &output);
        assert_native_run_output(&output, stem, expected_stdout);
        let _ = fs::remove_file(output);
    }
}

#[test]
fn compiles_and_runs_large_void_main_string_literal_echo() {
    if !host_linker_is_available() {
        eprintln!(
            "native stdout smoke test unavailable: host linker `{}` was not found",
            host_linker()
        );
        return;
    }

    let message = "Doria".repeat(64 * 1024);
    let source = format!(
        r#"
function main(): void
{{
    echo "{message}";
}}
"#
    );
    let output = temp_executable_path("main_void_large_echo");
    compile_native_source(&source, &output);
    assert_native_run_output(&output, "main_void_large_echo", message.as_bytes());
    let _ = fs::remove_file(output);
}

fn native_smoke_source(stem: &str) -> &'static str {
    match stem {
        "main_void_return" => {
            r#"
function main(): void
{
    return;
}
"#
        }
        "main_void_multiple_echo" => {
            r#"
function main(): void
{
    echo "Hello";
    echo " Doria!";
}
"#
        }
        "main_void_empty_echo" => {
            r#"
function main(): void
{
    echo "";
}
"#
        }
        "main_void_typed_string_local_echo" => {
            r#"
function main(): void
{
    string $message = "Hello Doria!";
    echo $message;
}
"#
        }
        "main_void_multiple_string_locals_echo" => {
            r#"
function main(): void
{
    let $hello = "Hello";
    string $space = " ";
    let $name = "Doria!";
    echo $hello;
    echo $space;
    echo $name;
}
"#
        }
        "main_void_string_local_plus_literal_echo" => {
            r#"
function main(): void
{
    let $message = "Doria!";
    echo "Hello ";
    echo $message;
}
"#
        }
        "main_void_grouped_string_local_echo" => {
            r#"
function main(): void
{
    let $message = ("Hello Doria!");
    echo ($message);
}
"#
        }
        "main_void_string_local_guard_skip" => {
            r#"
function main(): void
{
    let $message = "should not print";

    if (true) {
        return;
    }

    echo $message;
}
"#
        }
        "main_void_branch_string_shadowing_echo" => {
            r#"
function main(): void
{
    let $message = "outer";

    if (true) {
        let $message = "inner";
        echo $message;
    }

    echo $message;
}
"#
        }
        "main_void_loop_body_string_local_echo" => {
            r#"
function main(): void
{
    let writable $count = 0;

    while ($count < 2) {
        let $message = "Hi";
        echo $message;
        $count += 1;
    }
}
"#
        }
        "main_void_guard_true_return" => {
            r#"
function main(): void
{
    if (true) {
        return;
    }
}
"#
        }
        "main_void_guard_false_return" => {
            r#"
function main(): void
{
    if (false) {
        return;
    }
}
"#
        }
        "main_void_guard_true_skips_echo" => {
            r#"
function main(): void
{
    if (true) {
        return;
    }

    echo "should not print";
}
"#
        }
        "main_void_guard_false_reaches_echo" => {
            r#"
function main(): void
{
    if (false) {
        return;
    }

    echo "Hello Doria!";
}
"#
        }
        "main_void_else_if_final_true_returns" => {
            r#"
function main(): void
{
    if (false) {
        return;
    } else if (true) {
        return;
    }
}
"#
        }
        "main_void_else_if_final_false_falls_through" => {
            r#"
function main(): void
{
    if (false) {
        return;
    } else if (false) {
        return;
    }
}
"#
        }
        "main_return_125" => {
            r#"
function main(): int
{
    return 125;
}
"#
        }
        "main_return_arithmetic_literal" => {
            r#"
function main(): int
{
    return 20 + 22;
}
"#
        }
        "main_typed_readonly_local" => {
            r#"
function main(): int
{
    int $code = 42;
    return $code;
}
"#
        }
        "main_unused_large_local" => {
            r#"
function main(): int
{
    let $value = 9223372036854775807;
    return 0;
}
"#
        }
        "main_local_to_local" => {
            r#"
function main(): int
{
    let $first = 42;
    let $second = $first;
    return $second;
}
"#
        }
        "main_negative_unused_local" => {
            r#"
function main(): int
{
    let $negative = 1 - 2;
    return 0;
}
"#
        }
        "main_return_arithmetic_locals" => {
            r#"
function main(): int
{
    let $left = 20;
    let $right = 22;
    return $left + $right;
}
"#
        }
        "main_return_product_arithmetic" => {
            r#"
function main(): int
{
    let $base = 6;
    let $scale = 7;
    return $base * $scale;
}
"#
        }
        "main_return_grouped_arithmetic" => {
            r#"
function main(): int
{
    let $left = 20;
    let $right = 22;
    return ($left + $right) * 1;
}
"#
        }
        "main_stage_2c_arithmetic_local" => {
            r#"
function main(): int
{
    let $base = 6;
    let $scale = 7;
    let $code = $base * $scale;
    return $code;
}
"#
        }
        "main_unused_arithmetic_126" => {
            r#"
function main(): int
{
    let $value = 100 + 26;
    return 0;
}
"#
        }
        "main_if_true_42" => {
            r#"
function main(): int
{
    if (true) {
        return 42;
    } else {
        return 0;
    }
}
"#
        }
        "main_if_false_42" => {
            r#"
function main(): int
{
    if (false) {
        return 0;
    } else {
        return 42;
    }
}
"#
        }
        "main_guard_if_false_fallback_42" => {
            r#"
function main(): int
{
    if (false) {
        return 0;
    }

    return 42;
}
"#
        }
        "main_if_less_than_local" => {
            r#"
function main(): int
{
    let $x = 10;

    if ($x < 20) {
        return $x + 32;
    } else {
        return 0;
    }
}
"#
        }
        "main_if_large_local" => {
            r#"
function main(): int
{
    let $value = 126;

    if ($value > 100) {
        return 42;
    } else {
        return 0;
    }
}
"#
        }
        "main_if_not_false" => {
            r#"
function main(): int
{
    if (not false) {
        return 42;
    }

    return 0;
}
"#
        }
        "main_if_bang_false" => {
            r#"
function main(): int
{
    if (!false) {
        return 42;
    }

    return 0;
}
"#
        }
        "main_if_true_and_true" => {
            r#"
function main(): int
{
    if (true and true) {
        return 42;
    }

    return 0;
}
"#
        }
        "main_if_true_symbol_and_true" => {
            r#"
function main(): int
{
    if (true && true) {
        return 42;
    }

    return 0;
}
"#
        }
        "main_if_false_or_true" => {
            r#"
function main(): int
{
    if (false or true) {
        return 42;
    }

    return 0;
}
"#
        }
        "main_if_false_symbol_or_true" => {
            r#"
function main(): int
{
    if (false || true) {
        return 42;
    }

    return 0;
}
"#
        }
        "main_if_true_xor_false" => {
            r#"
function main(): int
{
    if (true xor false) {
        return 42;
    }

    return 0;
}
"#
        }
        "main_if_comparison_and_comparison" => {
            r#"
function main(): int
{
    let $left = 20;
    let $right = 22;

    if (($left == 20) and ($right == 22)) {
        return 42;
    }

    return 0;
}
"#
        }
        "main_if_comparison_or_false" => {
            r#"
function main(): int
{
    let $left = 20;
    let $right = 22;

    if (($left + $right == 42) or false) {
        return 42;
    }

    return 0;
}
"#
        }
        "main_if_comparison_xor_comparison" => {
            r#"
function main(): int
{
    let $left = 20;
    let $right = 22;

    if (($left == 20) xor ($right == 20)) {
        return 42;
    }

    return 0;
}
"#
        }
        "main_terminal_if_else_false_or_true" => {
            r#"
function main(): int
{
    if (false or true) {
        return 42;
    } else {
        return 0;
    }
}
"#
        }
        "main_typed_writable_sub_assign" => {
            r#"
function main(): int
{
    writable int $code = 50;
    $code -= 8;

    return $code;
}
"#
        }
        "main_writable_assign" => {
            r#"
function main(): int
{
    let writable $code = 0;
    $code = 42;

    return $code;
}
"#
        }
        "main_writable_assign_from_multiply" => {
            r#"
function main(): int
{
    let writable $code = 1;
    $code = $code * 42;

    return $code;
}
"#
        }
        "main_writable_if_condition" => {
            r#"
function main(): int
{
    let $base = 20;
    let writable $code = $base;
    $code += 22;

    if ($code == 42) {
        return 42;
    }

    return 0;
}
"#
        }
        "main_large_writable_reassigned_zero" => {
            r#"
function main(): int
{
    let writable $large = 9223372036854775807;
    $large = 0;

    return $large;
}
"#
        }
        "main_exit_boundary_reassigned_zero" => {
            r#"
function main(): int
{
    let writable $code = 126;
    $code = 0;

    return $code;
}
"#
        }
        "main_branch_local_declaration_return" => {
            r#"
function main(): int
{
    if (true) {
        let $code = 42;

        return $code;
    }

    return 0;
}
"#
        }
        "main_branch_assignment_return" => {
            r#"
function main(): int
{
    let writable $code = 0;

    if (true) {
        $code = 42;

        return $code;
    }

    return 0;
}
"#
        }
        "main_else_if_structured_branch" => {
            r#"
function main(): int
{
    let writable $code = 0;

    if (false) {
        return 1;
    } else if (true) {
        $code = 42;

        return $code;
    } else {
        return 0;
    }
}
"#
        }
        "main_nested_if_structured_branch" => {
            r#"
function main(): int
{
    if (true) {
        if (true) {
            let $code = 42;

            return $code;
        } else {
            return 0;
        }
    } else {
        return 0;
    }
}
"#
        }
        "main_multiple_guard_if_returns" => {
            r#"
function main(): int
{
    if (false) {
        return 1;
    }

    if (true) {
        let $code = 42;

        return $code;
    }

    return 0;
}
"#
        }
        "main_branch_large_reassigned_42" => {
            r#"
function main(): int
{
    if (true) {
        let writable $code = 9223372036854775807;
        $code = 42;

        return $code;
    }

    return 0;
}
"#
        }
        "main_if_fallthrough_else_42" => {
            r#"
function main(): int
{
    let writable $code = 0;

    if (true) {
        $code = 42;
    } else {
        $code = 1;
    }

    return $code;
}
"#
        }
        "main_if_fallthrough_else_if_42" => {
            r#"
function main(): int
{
    let writable $code = 0;

    if (false) {
        $code = 1;
    } else if (true) {
        $code = 42;
    } else {
        $code = 2;
    }

    return $code;
}
"#
        }
        "main_nested_if_fallthrough_42" => {
            r#"
function main(): int
{
    let writable $code = 40;

    if ($code == 40) {
        if (true) {
            $code += 2;
        }
    }

    return $code;
}
"#
        }
        "main_while_then_if_fallthrough_42" => {
            r#"
function main(): int
{
    let writable $code = 0;

    while ($code < 40) {
        $code += 1;
    }

    if ($code == 40) {
        $code += 2;
    }

    return $code;
}
"#
        }
        "main_if_fallthrough_large_reassigned_42" => {
            r#"
function main(): int
{
    let writable $code = 126;

    if (true) {
        $code = 42;
    }

    return $code;
}
"#
        }
        "main_if_fallthrough_shadow_preserves_outer" => {
            r#"
function main(): int
{
    let $code = 1;

    if (true) {
        let $code = 42;
    }

    return $code;
}
"#
        }
        "main_if_fallthrough_shadow_preserves_pre_shadow_assignment" => {
            r#"
function main(): int
{
    let writable $code = 1;

    if (true) {
        $code = 2;
        let $code = 42;
    }

    return $code;
}
"#
        }
        "main_while_decrement_42" => {
            r#"
function main(): int
{
    let writable $code = 50;

    while ($code > 42) {
        $code -= 1;
    }

    return $code;
}
"#
        }
        "main_while_multiply_42" => {
            r#"
function main(): int
{
    let writable $code = 1;

    while ($code < 64) {
        $code = $code * 2;
    }

    return $code - 22;
}
"#
        }
        "main_while_step_local_42" => {
            r#"
function main(): int
{
    let writable $code = 0;
    let writable $step = 2;

    while ($code < 42) {
        $code += $step;
    }

    return $code;
}
"#
        }
        "main_while_inside_if_42" => {
            r#"
function main(): int
{
    let writable $code = 0;

    if (true) {
        while ($code < 42) {
            $code += 1;
        }

        return $code;
    }

    return 0;
}
"#
        }
        "main_while_local_above_exit_boundary_returns_zero" => {
            r#"
function main(): int
{
    let writable $code = 126;

    while ($code > 0) {
        $code -= 42;
    }

    return $code;
}
"#
        }
        "main_while_rhs_uses_ordered_body_state" => {
            r#"
function main(): int
{
    let writable $code = 0;
    let writable $x = 3037000500;

    while ($code == 0) {
        $x -= 1;
        $x = $x * $x;
        $x = 0;
        $code = 42;
    }

    return $code;
}
"#
        }
        "main_while_body_local_42" => {
            r#"
function main(): int
{
    let writable $code = 0;

    while ($code < 42) {
        let $step = 2;
        $code += $step;
    }

    return $code;
}
"#
        }
        "main_while_body_writable_local_42" => {
            r#"
function main(): int
{
    let writable $code = 0;

    while ($code < 42) {
        let writable $step = 1;
        $step += 1;
        $code += $step;
    }

    return $code;
}
"#
        }
        "main_while_body_if_42" => {
            r#"
function main(): int
{
    let writable $code = 0;

    while ($code < 42) {
        if ($code < 40) {
            $code += 2;
        } else {
            $code += 1;
        }
    }

    return $code;
}
"#
        }
        "main_while_body_if_empty_then_branch_42" => {
            r#"
function main(): int
{
    let writable $i = 0;

    while ($i < 1) {
        if ($i == 1) {
        } else {
            $i += 1;
        }
    }

    return $i + 41;
}
"#
        }
        "main_while_body_if_empty_else_branch_42" => {
            r#"
function main(): int
{
    let writable $i = 0;

    while ($i < 1) {
        if ($i == 0) {
            $i += 1;
        } else {
        }
    }

    return $i + 41;
}
"#
        }
        "main_while_body_shadow_preserves_outer" => {
            r#"
function main(): int
{
    let writable $code = 0;
    let writable $count = 0;

    while ($count < 1) {
        let $code = 42;
        $count += 1;
    }

    return $code;
}
"#
        }
        "main_while_body_shadow_preserves_pre_shadow_assignment" => {
            r#"
function main(): int
{
    let writable $code = 1;
    let writable $count = 0;

    while ($count < 1) {
        $code = 2;
        let $code = 42;
        $count += 1;
    }

    return $code;
}
"#
        }
        "main_while_break_shadow_preserves_outer" => {
            r#"
function main(): int
{
    let writable $code = 0;

    while (true) {
        let $code = 42;
        break;
    }

    return $code;
}
"#
        }
        "main_while_continue_shadow_preserves_outer" => {
            r#"
function main(): int
{
    let writable $code = 0;
    let writable $guard = 0;

    while ($guard < 1) {
        let $code = 42;
        $guard += 1;
        continue;
    }

    return $code;
}
"#
        }
        "main_while_if_break_shadow_preserves_outer" => {
            r#"
function main(): int
{
    let writable $code = 0;

    while (true) {
        let $code = 42;

        if (true) {
            break;
        }
    }

    return $code;
}
"#
        }
        "main_while_if_continue_shadow_preserves_outer" => {
            r#"
function main(): int
{
    let writable $code = 0;
    let writable $guard = 0;

    while ($guard < 1) {
        let $code = 42;
        $guard += 1;

        if (true) {
            continue;
        }
    }

    return $code;
}
"#
        }
        "main_while_break_42" => {
            r#"
function main(): int
{
    let writable $code = 0;

    while ($code < 100) {
        if ($code == 42) {
            break;
        }

        $code += 1;
    }

    return $code;
}
"#
        }
        "main_while_continue_skips_remaining_body_42" => {
            r#"
function main(): int
{
    let writable $code = 0;
    let writable $sum = 0;

    while ($code < 10) {
        $code += 1;

        if ($code < 10) {
            continue;
        }

        $sum = 42;
    }

    return $sum;
}
"#
        }
        "main_while_if_else_break_42" => {
            r#"
function main(): int
{
    let writable $code = 0;

    while ($code < 100) {
        if ($code == 42) {
            break;
        } else {
            $code += 1;
        }
    }

    return $code;
}
"#
        }
        "main_while_if_continue_42" => {
            r#"
function main(): int
{
    let writable $code = 0;
    let writable $sum = 0;

    while ($code < 42) {
        $code += 1;

        if ($code < 42) {
            continue;
        }

        $sum = $code;
    }

    return $sum;
}
"#
        }
        "main_while_true_break_42" => {
            r#"
function main(): int
{
    let writable $code = 0;

    while (true) {
        if ($code == 42) {
            break;
        }

        $code += 1;
    }

    return $code;
}
"#
        }
        "main_while_continue_then_unreachable_assignment_42" => {
            r#"
function main(): int
{
    let writable $code = 0;

    while ($code < 42) {
        $code += 1;
        continue;

        $code = 0;
    }

    return $code;
}
"#
        }
        "main_for_pre_increment_42" => {
            r#"
function main(): int
{
    let writable $sum = 0;

    for (let writable $i = 0; $i < 42; ++$i) {
        $sum += 1;
    }

    return $sum;
}
"#
        }
        "main_for_decrement_42" => {
            r#"
function main(): int
{
    let writable $sum = 0;

    for (let writable $i = 42; $i > 0; $i--) {
        $sum += 1;
    }

    return $sum;
}
"#
        }
        "main_foreach_range_break_42" => {
            r#"
function main(): int
{
    let writable $sum = 0;

    foreach (0..100 as $i) {
        if ($i == 42) {
            break;
        }

        $sum += 1;
    }

    return $sum;
}
"#
        }
        "main_foreach_range_continue_42" => {
            r#"
function main(): int
{
    let writable $sum = 0;

    foreach (0..10 as $i) {
        if ($i < 10) {
            continue;
        }

        $sum = 42;
    }

    return $sum;
}
"#
        }
        _ => unreachable!("unexpected inline native smoke source `{stem}`"),
    }
}

#[test]
fn rejects_unsupported_current_native_smoke_shapes() {
    let cases = [
        ("no main", "", "B0001", "no native entrypoint found"),
        (
            "main has parameter",
            r#"
function main(int $code): int
{
    return 0;
}
"#,
            "B0001",
            "must not declare parameters",
        ),
        (
            "return nonzero literal",
            r#"
function main(): int
{
    return 126;
}
"#,
            "B0001",
            "native Stage 7b exit code must be in the range 0..125",
        ),
        (
            "return 255",
            r#"
function main(): int
{
    return 255;
}
"#,
            "B0001",
            "native Stage 7b exit code must be in the range 0..125",
        ),
        (
            "return out of Doria int range",
            r#"
function main(): int
{
    return 9223372036854775808;
}
"#,
            "E0417",
            "integer literal is outside the Doria `int` range",
        ),
        (
            "return string",
            r#"
function main(): int
{
    return "0";
}
"#,
            "E0404",
            "cannot return value of type `string`",
        ),
        (
            "return bool",
            r#"
function main(): int
{
    return true;
}
"#,
            "E0404",
            "cannot return value of type `bool`",
        ),
        (
            "unproven for loop",
            r#"
function main(): int
{
    let writable $sum = 0;

    for (let writable $i = 0; true; $i++) {
        $sum += 1;
    }

    return $sum;
}
"#,
            "B0001",
            "loop could not be proven to terminate",
        ),
        (
            "foreach range exceeds smoke cap",
            r#"
function main(): int
{
    let writable $sum = 0;

    foreach (0..10001 as $i) {
        $sum += 0;
    }

    return 0;
}
"#,
            "B0001",
            "loop could not be proven to terminate",
        ),
        (
            "return undeclared variable",
            r#"
function main(): int
{
    return $code;
}
"#,
            "E0101",
            "undeclared variable `$code`",
        ),
        (
            "returned local outside exit-code range",
            r#"
function main(): int
{
    let $code = 126;
    return $code;
}
"#,
            "B0001",
            "native Stage 7b exit code must be in the range 0..125",
        ),
        (
            "return arithmetic outside exit-code range",
            r#"
function main(): int
{
    return 100 + 26;
}
"#,
            "B0001",
            "native Stage 7b exit code must be in the range 0..125",
        ),
        (
            "returned arithmetic local outside exit-code range",
            r#"
function main(): int
{
    let $value = 100 + 26;
    return $value;
}
"#,
            "B0001",
            "native Stage 7b exit code must be in the range 0..125",
        ),
        (
            "non-int local",
            r#"
function main(): int
{
    let $ok = true;
    return 0;
}
"#,
            "B0001",
            "unsupported native local for current native smoke backend",
        ),
        (
            "non-int writable local",
            r#"
function main(): int
{
    let writable $ok = true;
    return 0;
}
"#,
            "B0001",
            "unsupported native local for current native smoke backend",
        ),
        (
            "readonly assignment",
            r#"
function main(): int
{
    let $code = 40;
    $code += 2;

    return $code;
}
"#,
            "E0201",
            "cannot assign to readonly variable `$code`",
        ),
        (
            "undeclared assignment",
            r#"
function main(): int
{
    $code = 42;

    return 0;
}
"#,
            "E0101",
            "undeclared variable `$code`",
        ),
        (
            "assignment type mismatch",
            r#"
function main(): int
{
    let writable $code = 0;
    $code = true;

    return $code;
}
"#,
            "E0403",
            "cannot assign value of type `bool`",
        ),
        (
            "assignment result outside exit-code range",
            r#"
function main(): int
{
    let writable $code = 100;
    $code += 26;

    return $code;
}
"#,
            "B0001",
            "native Stage 7b exit code must be in the range 0..125",
        ),
        (
            "assignment overflow",
            r#"
function main(): int
{
    let writable $code = 9223372036854775807;
    $code += 1;

    return 0;
}
"#,
            "B0001",
            "integer arithmetic overflows the Doria `int` range",
        ),
        (
            "assignment rhs division",
            r#"
function main(): int
{
    let writable $code = 0;
    $code = 84 / 2;

    return $code;
}
"#,
            "B0001",
            "unsupported native arithmetic operator for Stage 7b",
        ),
        (
            "assignment after final return",
            r#"
function main(): int
{
    let writable $code = 42;

    return $code;

    $code = 0;

    return 0;
}
"#,
            "B0001",
            "unsupported statement after native terminator for Stage 7b",
        ),
        (
            "return division",
            r#"
function main(): int
{
    return 42 / 1;
}
"#,
            "B0001",
            "unsupported native arithmetic operator for Stage 7b",
        ),
        (
            "return modulo",
            r#"
function main(): int
{
    return 42 % 5;
}
"#,
            "B0001",
            "unsupported native arithmetic operator for Stage 7b",
        ),
        (
            "local initialized from division",
            r#"
function main(): int
{
    let $code = 84 / 2;
    return $code;
}
"#,
            "B0001",
            "unsupported native arithmetic operator for Stage 7b",
        ),
        (
            "local initialized from function call",
            r#"
function main(): int
{
    let $code = calculate();
    return $code;
}
"#,
            "E0309",
            "unknown function `calculate`",
        ),
        (
            "return function call",
            r#"
function main(): int
{
    return calculate();
}
"#,
            "E0309",
            "unknown function `calculate`",
        ),
        (
            "local outside Doria int range",
            r#"
function main(): int
{
    let $value = 9223372036854775808;
    return 0;
}
"#,
            "E0417",
            "integer literal is outside the Doria `int` range",
        ),
        (
            "return arithmetic overflow",
            r#"
function main(): int
{
    return 9223372036854775807 + 1;
}
"#,
            "E0418",
            "integer arithmetic overflows the Doria `int` range",
        ),
        (
            "return multiplication overflow",
            r#"
function main(): int
{
    return 9223372036854775807 * 2;
}
"#,
            "E0418",
            "integer arithmetic overflows the Doria `int` range",
        ),
        (
            "constant arithmetic overflow",
            r#"
function main(): int
{
    let $value = 9223372036854775807 + 1;
    return 0;
}
"#,
            "E0418",
            "integer arithmetic overflows the Doria `int` range",
        ),
        (
            "constant multiplication overflow",
            r#"
function main(): int
{
    let $value = 9223372036854775807 * 2;
    return 0;
}
"#,
            "E0418",
            "integer arithmetic overflows the Doria `int` range",
        ),
        (
            "returned negative local outside exit-code range",
            r#"
function main(): int
{
    let $code = 1 - 2;
    return $code;
}
"#,
            "B0001",
            "native Stage 7b exit code must be in the range 0..125",
        ),
        (
            "if else branch outside exit-code range",
            r#"
function main(): int
{
    if (true) {
        return 0;
    } else {
        return 126;
    }
}
"#,
            "B0001",
            "native Stage 7b exit code must be in the range 0..125",
        ),
        (
            "guard if branch outside exit-code range",
            r#"
function main(): int
{
    if (true) {
        return 126;
    }

    return 0;
}
"#,
            "B0001",
            "native Stage 7b exit code must be in the range 0..125",
        ),
        (
            "logical if branch outside exit-code range",
            r#"
function main(): int
{
    if (true and true) {
        return 126;
    }

    return 0;
}
"#,
            "B0001",
            "native Stage 7b exit code must be in the range 0..125",
        ),
        (
            "if integer condition",
            r#"
function main(): int
{
    if (1) {
        return 42;
    } else {
        return 0;
    }
}
"#,
            "E0416",
            "condition must be `bool`",
        ),
        (
            "if arithmetic integer condition",
            r#"
function main(): int
{
    if (20 + 22) {
        return 42;
    } else {
        return 0;
    }
}
"#,
            "E0416",
            "condition must be `bool`",
        ),
        (
            "if logical condition has non-bool operand",
            r#"
function main(): int
{
    if (1 and true) {
        return 42;
    }

    return 0;
}
"#,
            "E0419",
            "requires `bool` operands",
        ),
        (
            "if ambiguous xor condition",
            r#"
function main(): int
{
    if (true xor false or true) {
        return 42;
    }

    return 0;
}
"#,
            "P0001",
            "ambiguous `xor`",
        ),
        (
            "if condition division",
            r#"
function main(): int
{
    if (42 / 1 == 42) {
        return 42;
    } else {
        return 0;
    }
}
"#,
            "B0001",
            "unsupported native arithmetic operator for Stage 7b",
        ),
        (
            "if call condition",
            r#"
function main(): int
{
    if (isReady()) {
        return 42;
    }

    return 0;
}
"#,
            "E0309",
            "unknown function `isReady`",
        ),
        (
            "if without fallback return",
            r#"
function main(): int
{
    if (true) {
        return 42;
    }
}
"#,
            "E0406",
            "must return a value",
        ),
        (
            "int else if without final fallback return",
            r#"
function main(): int
{
    if (false) {
        return 0;
    } else if (true) {
        return 42;
    }
}
"#,
            "E0406",
            "must return a value",
        ),
        (
            "branch-local variable leak",
            r#"
function main(): int
{
    if (true) {
        let $code = 42;
    }

    return $code;
}
"#,
            "E0101",
            "undeclared variable `$code`",
        ),
        (
            "readonly assignment in fallthrough branch",
            r#"
function main(): int
{
    let $code = 40;

    if (true) {
        $code += 2;
    }

    return $code;
}
"#,
            "E0201",
            "cannot assign to readonly variable `$code`",
        ),
        (
            "assignment overflow inside fallthrough branch",
            r#"
function main(): int
{
    let writable $code = 9223372036854775807;

    if (true) {
        $code += 1;
    }

    return 0;
}
"#,
            "B0001",
            "integer arithmetic overflows the Doria `int` range",
        ),
        (
            "returned value outside exit-code range after fallthrough branch",
            r#"
function main(): int
{
    let writable $code = 100;

    if (true) {
        $code += 26;
    }

    return $code;
}
"#,
            "B0001",
            "native Stage 7b exit code must be in the range 0..125",
        ),
        (
            "branch return outside exit-code range",
            r#"
function main(): int
{
    if (true) {
        return 126;
    }

    return 0;
}
"#,
            "B0001",
            "native Stage 7b exit code must be in the range 0..125",
        ),
        (
            "division inside fallthrough branch",
            r#"
function main(): int
{
    let writable $code = 0;

    if (true) {
        $code = 84 / 2;
    }

    return $code;
}
"#,
            "B0001",
            "unsupported native arithmetic operator for Stage 7b",
        ),
        (
            "division inside branch",
            r#"
function main(): int
{
    if (true) {
        return 42 / 1;
    }

    return 0;
}
"#,
            "B0001",
            "unsupported native arithmetic operator for Stage 7b",
        ),
        (
            "readonly assignment in while",
            r#"
function main(): int
{
    let $code = 0;

    while ($code < 42) {
        $code += 1;
    }

    return 0;
}
"#,
            "E0201",
            "cannot assign to readonly variable `$code`",
        ),
        (
            "non-bool while condition",
            r#"
function main(): int
{
    let writable $code = 0;

    while ($code) {
        $code += 1;
    }

    return 0;
}
"#,
            "E0416",
            "condition must be `bool`",
        ),
        (
            "break outside loop",
            r#"
function main(): int
{
    break;

    return 0;
}
"#,
            "E0421",
            "`break` may only be used inside a loop",
        ),
        (
            "continue outside loop",
            r#"
function main(): int
{
    continue;

    return 0;
}
"#,
            "E0422",
            "`continue` may only be used inside a loop",
        ),
        (
            "numeric break",
            r#"
function main(): int
{
    let writable $code = 0;

    while ($code < 42) {
        break 2;
    }

    return 0;
}
"#,
            "P0001",
            "`break` does not accept a value or label in this Doria slice",
        ),
        (
            "numeric continue",
            r#"
function main(): int
{
    let writable $code = 0;

    while ($code < 42) {
        continue 2;
    }

    return 0;
}
"#,
            "P0001",
            "`continue` does not accept a value or label in this Doria slice",
        ),
        (
            "nested while",
            r#"
function main(): int
{
    let writable $code = 0;

    while ($code < 42) {
        while ($code < 10) {
            $code += 1;
        }
    }

    return $code;
}
"#,
            "B0001",
            "unsupported native while body statement for Stage 7b",
        ),
        (
            "return inside while",
            r#"
function main(): int
{
    let writable $code = 0;

    while ($code < 42) {
        return $code;
    }

    return 0;
}
"#,
            "B0001",
            "unsupported native while body statement for Stage 7b",
        ),
        (
            "nonterminating while",
            r#"
function main(): int
{
    let writable $code = 0;

    while (true) {
        $code += 1;
    }

    return 0;
}
"#,
            "B0001",
            "loop could not be proven to terminate within the current native smoke verification cap",
        ),
        (
            "unproven continue loop",
            r#"
function main(): int
{
    let writable $code = 0;

    while (true) {
        continue;
        $code += 1;
    }

    return 0;
}
"#,
            "B0001",
            "loop could not be proven to terminate within the current native smoke verification cap",
        ),
        (
            "cap-exceeding while",
            r#"
function main(): int
{
    let writable $code = 0;

    while ($code < 10001) {
        $code += 1;
    }

    return 0;
}
"#,
            "B0001",
            "loop could not be proven to terminate within the current native smoke verification cap",
        ),
        (
            "assignment overflow inside while",
            r#"
function main(): int
{
    let writable $code = 9223372036854775807;

    while ($code > 0) {
        $code += 1;
    }

    return 0;
}
"#,
            "B0001",
            "integer arithmetic overflows the Doria `int` range",
        ),
        (
            "overflow before break",
            r#"
function main(): int
{
    let writable $code = 9223372036854775807;

    while (true) {
        $code += 1;

        break;
    }

    return 0;
}
"#,
            "B0001",
            "integer arithmetic overflows the Doria `int` range",
        ),
        (
            "returned value outside process boundary after while",
            r#"
function main(): int
{
    let writable $code = 0;

    while ($code < 126) {
        $code += 1;
    }

    return $code;
}
"#,
            "B0001",
            "native Stage 7b exit code must be in the range 0..125",
        ),
        (
            "statement after terminal if else",
            r#"
function main(): int
{
    if (true) {
        return 42;
    } else {
        return 0;
    }

    return 1;
}
"#,
            "B0001",
            "unsupported statement after native terminator for Stage 7b",
        ),
        (
            "numeric echo",
            r#"
function main(): int
{
    echo 0;
    return 0;
}
"#,
            "B0001",
            "unsupported native echo expression for Stage 8a",
        ),
        (
            "int local echo",
            r#"
function main(): int
{
    let $code = 42;
    echo $code;
    return 0;
}
"#,
            "B0001",
            "unsupported native echo expression for Stage 8a",
        ),
        (
            "writable string local",
            r#"
function main(): void
{
    let writable $message = "Hello Doria!";
    echo $message;
}
"#,
            "B0001",
            "unsupported native string local for Stage 8a",
        ),
        (
            "explicit writable string local",
            r#"
function main(): void
{
    writable string $message = "Hello Doria!";
    echo $message;
}
"#,
            "B0001",
            "unsupported native string local for Stage 8a",
        ),
        (
            "string assignment",
            r#"
function main(): void
{
    string $message = "Hello";
    $message = "Doria!";
    echo $message;
}
"#,
            "E0201",
            "cannot assign to readonly variable `$message`",
        ),
        (
            "interpolated echo",
            r#"
function main(): void
{
    let $name = 1;
    echo "Hello, {$name}";
}
"#,
            "B0001",
            "unsupported native string interpolation for Stage 8a",
        ),
        (
            "top-level statement",
            r#"
echo 0;

function main(): int
{
    return 0;
}
"#,
            "B0001",
            "unsupported top-level item",
        ),
        (
            "class",
            r#"
class Person
{
}

function main(): int
{
    return 0;
}
"#,
            "B0001",
            "class `Person`",
        ),
        (
            "extra top-level function",
            r#"
function helper(): int
{
    return 0;
}

function main(): int
{
    return 0;
}
"#,
            "B0001",
            "extra top-level function `helper`",
        ),
    ];

    for (name, source, expected_code, expected_message) in cases {
        let diagnostics =
            doriac::compile_source(format!("{name}.doria"), source, BackendTarget::Native)
                .expect_err("unsupported current native smoke source should fail");

        assert_eq!(diagnostics[0].code, expected_code, "{name}");
        assert!(
            diagnostics[0].message.contains(expected_message),
            "{name}: expected message containing `{expected_message}`, got `{}`",
            diagnostics[0].message
        );
    }
}

#[test]
fn native_backend_returns_executable_output_for_literal_shape() {
    if !host_linker_is_available() {
        eprintln!(
            "native executable output test unavailable: host linker `{}` was not found",
            host_linker()
        );
        return;
    }

    let output = doriac::compile_source(
        "test.doria",
        r#"
function main(): int
{
    return 42;
}
"#,
        BackendTarget::Native,
    )
    .expect("current native smoke source should compile");

    match output {
        doriac::backend::BackendOutput::Executable { bytes, .. } => {
            assert!(!bytes.is_empty());
        }
        other => panic!("native backend should return executable output, got {other:?}"),
    }
}

#[test]
fn native_backend_returns_executable_output_for_arithmetic_shape() {
    if !host_linker_is_available() {
        eprintln!(
            "native executable output test unavailable: host linker `{}` was not found",
            host_linker()
        );
        return;
    }

    let output = doriac::compile_source(
        "test.doria",
        r#"
function main(): int
{
    let $base = 20;
    let $code = $base * 2 + 2;
    return $code;
}
"#,
        BackendTarget::Native,
    )
    .expect("current native arithmetic source should compile");

    match output {
        doriac::backend::BackendOutput::Executable { bytes, .. } => {
            assert!(!bytes.is_empty());
        }
        other => panic!("native backend should return executable output, got {other:?}"),
    }
}

fn assert_native_run_output(output: &Path, stem: &str, expected_stdout: &[u8]) {
    let run = run_native_executable(output).expect("native executable should run");
    assert_eq!(run.status.code(), Some(0), "{stem}");
    assert_eq!(run.stdout, expected_stdout, "{stem}");
    assert!(
        run.stderr.is_empty(),
        "{stem}: expected empty stderr, got {}",
        String::from_utf8_lossy(&run.stderr)
    );
}

fn run_native_executable(output: &Path) -> io::Result<Output> {
    const MAX_ATTEMPTS: usize = 20;

    for attempt in 0..MAX_ATTEMPTS {
        match Command::new(output).output() {
            Ok(output) => return Ok(output),
            Err(error) if is_transient_executable_busy(&error) && attempt + 1 < MAX_ATTEMPTS => {
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => return Err(error),
        }
    }

    unreachable!("retry loop returns on final attempt")
}

fn is_transient_executable_busy(error: &io::Error) -> bool {
    cfg!(unix) && error.raw_os_error() == Some(26)
}

fn compile_native_file(input: &Path, output: &Path) {
    let doriac = env!("CARGO_BIN_EXE_doriac");
    let compile = Command::new(doriac)
        .arg("compile")
        .arg(input)
        .arg("--target")
        .arg("native")
        .arg("--out")
        .arg(output)
        .output()
        .expect("doriac binary should run");

    assert_native_compile_succeeded(compile);
    assert!(output.exists(), "native executable should exist");
}

fn compile_native_source(source: &str, output: &Path) {
    let native = doriac::compile_source("test.doria", source, BackendTarget::Native)
        .expect("current native smoke source should compile");
    let doriac::backend::BackendOutput::Executable { bytes, .. } = native else {
        panic!("native backend should return executable output, got {native:?}");
    };
    fs::write(output, bytes).expect("native executable bytes should be writable");
    make_executable(output);
}

fn assert_native_compile_succeeded(compile: std::process::Output) {
    if !compile.status.success() {
        let stderr = String::from_utf8_lossy(&compile.stderr);
        let stdout = String::from_utf8_lossy(&compile.stdout);
        panic!(
            "native smoke compile failed\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
            compile.status, stdout, stderr
        );
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate should live under crates/doriac")
        .to_path_buf()
}

fn temp_executable_path(stem: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let extension = if cfg!(windows) { ".exe" } else { "" };
    std::env::temp_dir().join(format!(
        "doriac-{stem}-{}-{nanos}{extension}",
        std::process::id()
    ))
}

fn host_linker_is_available() -> bool {
    let linker = host_linker();
    let mut command = Command::new(&linker);
    if cfg!(windows) {
        command.arg("/?");
    } else {
        command.arg("--version");
    }
    command.output().is_ok()
}

fn host_linker() -> String {
    std::env::var("CC").unwrap_or_else(|_| default_linker().to_string())
}

fn default_linker() -> &'static str {
    if cfg!(windows) {
        "cl.exe"
    } else {
        "cc"
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .expect("native executable metadata should be readable")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("native executable should be made executable");
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}
