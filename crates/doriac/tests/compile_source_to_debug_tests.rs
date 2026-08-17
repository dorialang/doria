const FIXED_WIDTH_SOURCE: &str = r#"
function answer(uint8 $base): uint8
{
    return $base + 2;
}

function main(): int
{
    return Int::from(answer(40));
}
"#;

#[test]
fn returns_rendered_interpreter_output() {
    let debug = doriac::compile_source_to_debug(
        "test.doria",
        r#"
function main(): int
{
    return 42;
}
"#,
    )
    .expect("valid source should compile for the debug target");

    assert_eq!(debug, "exit_status: 42\nstdout:\n");
}

#[test]
fn captures_stdout() {
    let debug = doriac::compile_source_to_debug(
        "test.doria",
        r#"
function main(): int throws Doria\Std\Io\IoError
{
    echo "hello";
    return 0;
}
"#,
    )
    .expect("valid source should compile for the debug target");

    assert!(
        debug.contains("stdout: hello"),
        "debug output did not carry stdout: {debug}"
    );
}

#[test]
fn reports_diagnostics_instead_of_panicking() {
    let diagnostics = doriac::compile_source_to_debug(
        "test.doria",
        r#"
function main(): int
{
    return $missing;
}
"#,
    )
    .expect_err("unresolved variable should fail to compile");

    assert!(
        !diagnostics.is_empty(),
        "failure should carry at least one diagnostic"
    );
}

#[test]
fn accepts_programs_the_php_backend_rejects() {
    let php_diagnostics = doriac::compile_source_to_php("test.doria", FIXED_WIDTH_SOURCE)
        .expect_err("fixed-width integers are unsupported by the PHP backend");
    assert_eq!(php_diagnostics[0].code, "B1301");

    let debug = doriac::compile_source_to_debug("test.doria", FIXED_WIDTH_SOURCE)
        .expect("fixed-width integers are valid Doria and must run on the debug target");

    assert_eq!(debug, "exit_status: 42\nstdout:\n");
}
