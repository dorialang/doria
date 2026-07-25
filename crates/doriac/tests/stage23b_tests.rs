//! Stage 23b — program entry arguments (decision 0099).
//!
//! Covers the accepted `main(List<string> $args)` forms, the entry shapes that
//! stay rejected, and the argument-list contract: the executable path is not
//! element 0, and a no-argument invocation yields an empty list.

use std::collections::BTreeMap;

use doriac::mir_interpreter::{interpret_with_io, MirIo};

const ENTRY_ARGUMENTS_EXAMPLE: &str =
    include_str!("../../../examples/native/main_stage23b_entry_arguments.doria");
const ENTRY_ARGUMENTS_EMPTY_EXAMPLE: &str =
    include_str!("../../../examples/native/main_stage23b_entry_arguments_empty.doria");

fn diagnostics(source: &str) -> Vec<doriac::diagnostics::Diagnostic> {
    doriac::check_source("stage23b.doria", source).expect_err("source should be rejected")
}

fn run_with_args(source: &str, args: &[&str]) -> String {
    let program =
        doriac::lower_source_to_mir("stage23b.doria", source).expect("source should lower to MIR");
    let output = interpret_with_io(
        &program,
        MirIo {
            stdin: Vec::new(),
            files: BTreeMap::new(),
            args: args.iter().map(|argument| argument.to_string()).collect(),
        },
    )
    .expect("MIR should interpret");
    String::from_utf8(output.output.stdout).expect("stdout is UTF-8")
}

#[test]
fn the_entry_receives_its_arguments_without_the_executable_path() {
    // `$args[0]` is the first real argument, and `$args->count` is how many
    // arguments the user passed.
    let stdout = run_with_args(ENTRY_ARGUMENTS_EXAMPLE, &["alpha", "two words", "é日"]);
    assert_eq!(
        stdout,
        concat!(
            "count=3\n",
            "first=alpha\n",
            "arg=alpha\n",
            "arg=two words\n",
            "arg=é日\n",
        )
    );
}

#[test]
fn a_no_argument_invocation_yields_an_empty_list() {
    // Never a one-element list holding the executable path, and never null.
    let stdout = run_with_args(ENTRY_ARGUMENTS_EMPTY_EXAMPLE, &[]);
    assert_eq!(stdout, "count=0\nempty=true\n");
}

#[test]
fn a_single_argument_is_not_mistaken_for_the_executable_path() {
    let stdout = run_with_args(ENTRY_ARGUMENTS_EXAMPLE, &["only"]);
    assert_eq!(stdout, "count=1\nfirst=only\narg=only\n");
}

#[test]
fn both_return_types_accept_the_argument_list() {
    let void_entry = r#"
function main(List<string> $args): void
{
    printf("count=%d\n", $args->count);
}
"#;
    assert_eq!(run_with_args(void_entry, &["a", "b"]), "count=2\n");
}

#[test]
fn the_parameterless_entry_forms_keep_working() {
    // Regression: decision 0032's forms are unchanged by the optional parameter.
    let int_entry = r#"
function main(): int
{
    echo "int entry\n";
    return 0;
}
"#;
    assert_eq!(run_with_args(int_entry, &[]), "int entry\n");

    let void_entry = r#"
function main(): void
{
    echo "void entry\n";
}
"#;
    assert_eq!(run_with_args(void_entry, &[]), "void entry\n");
}

#[test]
fn a_separate_argument_count_parameter_is_rejected() {
    // Decision 0099 rejects `main(string[] $argv, int $argc)` explicitly: the
    // container carries its own length.
    let source = r#"
function main(string[] $argv, int $argc): int
{
    return 0;
}
"#;
    let found = diagnostics(source);
    assert!(
        found.iter().any(|diagnostic| diagnostic.code == "E0526"),
        "expected E0526, got {found:#?}"
    );
}

#[test]
fn an_argument_list_of_the_wrong_type_is_rejected() {
    let source = r#"
function main(string[] $args): int
{
    return 0;
}
"#;
    let found = diagnostics(source);
    let diagnostic = found
        .iter()
        .find(|diagnostic| diagnostic.code == "E0526")
        .unwrap_or_else(|| panic!("expected E0526, got {found:#?}"));
    assert!(
        diagnostic.message.contains("List<string>"),
        "the diagnostic names the expected type: {}",
        diagnostic.message
    );
}

#[test]
fn a_writable_or_consuming_argument_list_is_rejected() {
    // The entry glue owns the list and lends it to `main`, so the parameter is
    // an ordinary readonly borrow.
    for source in [
        r#"
function main(writable List<string> $args): int
{
    return 0;
}
"#,
        r#"
function main(take List<string> $args): int
{
    return 0;
}
"#,
    ] {
        let found = diagnostics(source);
        assert!(
            found.iter().any(|diagnostic| diagnostic.code == "E0526"),
            "expected E0526, got {found:#?}"
        );
    }
}
