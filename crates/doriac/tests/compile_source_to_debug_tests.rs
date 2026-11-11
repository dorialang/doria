// PHP output cannot prove a float-to-integer conversion exact, so it rejects
// this valid program.
const FLOAT_CONVERSION_SOURCE: &str = r#"
function main(): int
{
    return Float::toInt(42.0);
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
    let php_diagnostics = doriac::compile_source_to_php("test.doria", FLOAT_CONVERSION_SOURCE)
        .expect_err("float-to-integer conversion remains unsupported by the PHP backend");
    assert_eq!(php_diagnostics[0].code, "B1301");

    let debug = doriac::compile_source_to_debug("test.doria", FLOAT_CONVERSION_SOURCE)
        .expect("the conversion is valid Doria and must run on the debug target");

    assert_eq!(debug, "exit_status: 42\nstdout:\n");
}
