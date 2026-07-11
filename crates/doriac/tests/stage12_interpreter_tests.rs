use doriac::mir_interpreter::{interpret, interpret_with_limits, InterpreterLimits};

fn interpret_source(source: &str) -> doriac::mir_interpreter::InterpreterOutput {
    let mir = doriac::lower_source_to_mir("test.doria", source)
        .expect("source should lower through checked MIR");
    interpret(&mir).expect("MIR should execute")
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
    assert_eq!(output.exit_status, 101);
    assert_eq!(
        output.stderr,
        b"Panic: explicit panic\nStack Trace:\n  at main\n"
    );
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
    assert_eq!(output.exit_status, 101);
    assert!(output.stderr.starts_with(b"Panic: runtime boom\n"));
}

#[test]
fn nested_panic_uses_source_function_names() {
    let output = interpret_source(include_str!(
        "../../../examples/native/main_nested_panic_stack.doria"
    ));
    assert_eq!(output.exit_status, 101);
    assert_eq!(
        output.stderr,
        b"Panic: boom\nStack Trace:\n  at explode\n  at middle\n  at main\n"
    );
}

#[test]
fn recursive_panic_trace_retains_recursive_frames() {
    let output = interpret_source(include_str!(
        "../../../examples/native/main_recursive_panic_stack.doria"
    ));
    assert_eq!(output.exit_status, 101);
    assert_eq!(
        output.stderr,
        b"Panic: bottom\nStack Trace:\n  at descend\n  at descend\n  at descend\n  at main\n"
    );
}

#[test]
fn checked_addition_overflow_panics() {
    let output = interpret_source(include_str!(
        "../../../examples/native/main_add_overflow_panic.doria"
    ));
    assert_eq!(output.exit_status, 101);
    assert!(output
        .stderr
        .starts_with(b"Panic: integer overflow during addition\n"));
}

#[test]
fn checked_subtraction_overflow_panics() {
    let output = interpret_source(include_str!(
        "../../../examples/native/main_subtract_overflow_panic.doria"
    ));
    assert_eq!(output.exit_status, 101);
    assert!(output
        .stderr
        .starts_with(b"Panic: integer overflow during subtraction\n"));
}

#[test]
fn checked_multiplication_overflow_panics() {
    let output = interpret_source(include_str!(
        "../../../examples/native/main_multiply_overflow_panic.doria"
    ));
    assert_eq!(output.exit_status, 101);
    assert!(output
        .stderr
        .starts_with(b"Panic: integer overflow during multiplication\n"));
}

#[test]
fn invalid_main_status_panics() {
    let output = interpret_source(include_str!(
        "../../../examples/native/main_invalid_status_panic.doria"
    ));
    assert_eq!(output.exit_status, 101);
    assert_eq!(
        output.stderr,
        b"Panic: main returned process status outside 0..125\nStack Trace:\n  at main\n"
    );
}
