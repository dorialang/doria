//! Stage 23b — program entry arguments (decision 0099).
//!
//! Covers the accepted `main(List<string> $args)` forms, the entry shapes that
//! stay rejected, and the argument-list contract: the executable path is not
//! element 0, and a no-argument invocation yields an empty list.

use std::collections::BTreeMap;

use doriac::mir::{LocalId, ScalarType, Type};
use doriac::mir_interpreter::{interpret_with_io, MirIo};
use doriac::numeric::IntegerType;

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
            ..MirIo::default()
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
function main(List<string> $args): void throws Doria\Std\Io\IoError
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
function main(): int throws Doria\Std\Io\IoError
{
    echo "int entry\n";
    return 0;
}
"#;
    assert_eq!(run_with_args(int_entry, &[]), "int entry\n");

    let void_entry = r#"
function main(): void throws Doria\Std\Io\IoError
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

#[test]
fn shared_validation_rejects_a_writable_entry_argument_list() {
    let mut program = doriac::lower_source_to_mir(
        "stage23b-writable-mir.doria",
        "function main(List<string> $args): void {}",
    )
    .expect("source should lower to MIR");
    let entry = program.entry;
    let parameter = program.functions[entry.0].params[0];
    program.functions[entry.0].locals[parameter.0].writable = true;

    let error = doriac::mir_validation::validate_program(&program)
        .expect_err("entry glue lends a readonly argument list");
    assert!(error
        .message
        .contains("readonly borrow from the entry glue"));
}

#[test]
fn the_interpreter_rejects_malformed_entry_argument_mir() {
    let source = "function main(List<string> $args): void {}";

    let mut wrong_element = doriac::lower_source_to_mir("stage23b-wrong-element.doria", source)
        .expect("source should lower to MIR");
    let entry = wrong_element.entry;
    let parameter = wrong_element.functions[entry.0].params[0];
    let Type::Collection(collection) = wrong_element.functions[entry.0].locals[parameter.0].ty
    else {
        panic!("entry parameter should be a collection");
    };
    wrong_element.collection_types[collection.0].value =
        Type::Scalar(ScalarType::Integer(IntegerType::Int64));
    let error = interpret_with_io(&wrong_element, MirIo::default())
        .expect_err("the interpreter must reject a non-string entry list");
    assert!(error.message.contains("List<string>"));

    let mut missing_local = doriac::lower_source_to_mir("stage23b-missing-local.doria", source)
        .expect("source should lower to MIR");
    let entry = missing_local.entry;
    missing_local.functions[entry.0].params[0] = LocalId(usize::MAX);
    let error = interpret_with_io(&missing_local, MirIo::default())
        .expect_err("the interpreter must reject an out-of-range entry parameter");
    assert!(error
        .message
        .contains("entry parameter local does not exist"));
}
