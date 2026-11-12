// PHP output cannot measure a string in grapheme clusters without optional
// extensions, so it rejects this valid program.
const STRING_LENGTH_SOURCE: &str = r#"
function main(): int
{
    let $text = "forty-two characters fill this literal....";
    return $text->length;
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
    let php_diagnostics = doriac::compile_source_to_php("test.doria", STRING_LENGTH_SOURCE)
        .expect_err("string measurement remains unsupported by the PHP backend");
    assert_eq!(php_diagnostics[0].code, "B2501");

    let debug = doriac::compile_source_to_debug("test.doria", STRING_LENGTH_SOURCE)
        .expect("the measurement is valid Doria and must run on the debug target");

    assert_eq!(debug, "exit_status: 42\nstdout:\n");
}
