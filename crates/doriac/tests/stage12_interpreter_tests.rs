use doriac::mir_interpreter::{interpret, interpret_with_limits, InterpreterLimits};

fn interpret_source(source: &str) -> doriac::mir_interpreter::InterpreterOutput {
    let mir = doriac::lower_source_to_mir("test.doria", source)
        .expect("source should lower through checked MIR");
    interpret(&mir).expect("MIR should execute")
}

fn assert_runtime_panic(
    output: &doriac::mir_interpreter::InterpreterOutput,
    code: &str,
    title: &str,
    frames: &[&str],
) {
    assert_eq!(output.exit_status, 101);
    let diagnostic = output
        .runtime_diagnostic
        .as_ref()
        .expect("panic should retain a structured diagnostic");
    assert_eq!(diagnostic.code, code);
    assert_eq!(diagnostic.title, title);
    let stderr = String::from_utf8(output.stderr.clone()).expect("panic output should be UTF-8");
    assert!(stderr.starts_with(&format!("Panic[{code}]: {title}\n\nWhere\n")));
    assert!(stderr.contains("\n\nWhy\n"));
    assert!(stderr.contains("\n\nCall Path\n"));
    let mut previous = 0;
    for frame in frames {
        let marker = format!("\n{frame} · ");
        let index = stderr[previous..]
            .find(&marker)
            .map(|index| previous + index)
            .unwrap_or_else(|| panic!("missing `{frame}` in call path:\n{stderr}"));
        previous = index + marker.len();
    }
    assert!(stderr.ends_with("\n\nProcess Exited With Status 101\n"));
}

#[test]
fn recursive_fibonacci_executes() {
    let output = interpret_source(include_str!(
        "../../../examples/native/main_recursive_fibonacci_55.doria"
    ));
    assert_eq!(output.exit_status, 55);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
}

#[test]
fn mutual_recursion_executes() {
    let output = interpret_source(include_str!(
        "../../../examples/native/main_mutual_recursion_42.doria"
    ));
    assert_eq!(output.exit_status, 42);
}

#[test]
fn recursion_depth_exceeds_the_old_256_frame_cap() {
    let output = interpret_source(include_str!(
        "../../../examples/native/main_recursive_depth_512_42.doria"
    ));
    assert_eq!(output.exit_status, 42);
}

#[test]
fn long_finite_loop_exceeds_the_old_block_budget() {
    let output = interpret_source(include_str!(
        "../../../examples/native/main_long_while_42.doria"
    ));
    assert_eq!(output.exit_status, 42);
}

#[test]
fn explicitly_limited_interpretation_stops_an_infinite_program() {
    let mir = doriac::lower_source_to_mir(
        "test.doria",
        include_str!("../../../examples/compile-only/main_infinite_while.doria"),
    )
    .expect("infinite loop should lower normally");
    let error = interpret_with_limits(
        &mir,
        InterpreterLimits {
            max_executed_blocks: Some(100),
            max_call_frames: None,
        },
    )
    .expect_err("explicit test limit should stop execution");
    assert!(error.message.contains("explicit test execution limit"));
}

#[test]
fn explicit_panic_is_a_runtime_outcome() {
    let output = interpret_source(include_str!(
        "../../../examples/native/main_explicit_panic.doria"
    ));
    assert_runtime_panic(&output, "P1000", "Program Panicked", &["main"]);
}

#[test]
fn panic_accepts_readonly_compile_time_string_concatenation() {
    let output = interpret_source(
        r#"function main(): void
{
    let $message = "boom";
    panic("runtime " . $message);
}
"#,
    );
    assert_runtime_panic(&output, "P1000", "Program Panicked", &["main"]);
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("\n\nNote\nruntime boom\n"), "{stderr}");
}

#[test]
fn nested_panic_uses_source_function_names() {
    let output = interpret_source(include_str!(
        "../../../examples/native/main_nested_panic_stack.doria"
    ));
    assert_runtime_panic(
        &output,
        "P1000",
        "Program Panicked",
        &["explode", "middle", "main"],
    );
}

#[test]
fn recursive_panic_trace_retains_recursive_frames() {
    let output = interpret_source(include_str!(
        "../../../examples/native/main_recursive_panic_stack.doria"
    ));
    assert_runtime_panic(
        &output,
        "P1000",
        "Program Panicked",
        &["descend", "descend", "descend", "main"],
    );
}

#[test]
fn checked_addition_overflow_panics() {
    let output = interpret_source(include_str!(
        "../../../examples/native/main_add_overflow_panic.doria"
    ));
    assert_runtime_panic(&output, "P1101", "Integer Addition Overflowed", &["main"]);
}

#[test]
fn checked_subtraction_overflow_panics() {
    let output = interpret_source(include_str!(
        "../../../examples/native/main_subtract_overflow_panic.doria"
    ));
    assert_runtime_panic(
        &output,
        "P1102",
        "Integer Subtraction Overflowed",
        &["main"],
    );
}

#[test]
fn checked_multiplication_overflow_panics() {
    let output = interpret_source(include_str!(
        "../../../examples/native/main_multiply_overflow_panic.doria"
    ));
    assert_runtime_panic(
        &output,
        "P1103",
        "Integer Multiplication Overflowed",
        &["main"],
    );
}

#[test]
fn invalid_main_status_panics() {
    let output = interpret_source(include_str!(
        "../../../examples/native/main_invalid_status_panic.doria"
    ));
    assert_runtime_panic(
        &output,
        "P1111",
        "Main Returned An Invalid Process Status",
        &["main"],
    );
}
