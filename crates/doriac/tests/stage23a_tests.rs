//! Stage 23a — named arguments (decision 0098).
//!
//! Covers binding across all four callable forms, the middle-default skip that
//! positional calls could not express, source-order evaluation, the
//! duplicate/unknown/missing diagnostic set, and the parser's
//! positional-after-named rule.

use doriac::mir;

const NAMED_ARGUMENTS_EXAMPLE: &str =
    include_str!("../../../examples/native/main_stage23a_named_arguments.doria");
const NAMED_ARGUMENT_ORDER_EXAMPLE: &str =
    include_str!("../../../examples/native/main_stage23a_named_argument_order.doria");
const NAMED_OWNED_ARGUMENTS_EXAMPLE: &str =
    include_str!("../../../examples/native/main_stage23a_named_owned_arguments.doria");
const NAMED_OWNED_ARGUMENTS_STDOUT: &str =
    include_str!("fixtures/native_io/main_stage23a_named_owned_arguments/expected_stdout");

fn diagnostics(source: &str) -> Vec<doriac::diagnostics::Diagnostic> {
    doriac::check_source("stage23a.doria", source).expect_err("source should be rejected")
}

fn diagnostic_snapshot(source: &str, code: &str) -> String {
    let found = diagnostics(source);
    let diagnostic = found
        .iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("expected {code}, got {found:#?}"));
    let mut snapshot = format!(
        "code: {}\nmessage: {}\nhelp: {}\nspan: {}..{}\n",
        diagnostic.code,
        diagnostic.message,
        diagnostic.help.as_deref().unwrap_or(""),
        diagnostic.span.start,
        diagnostic.span.end,
    );
    for related in &diagnostic.related {
        snapshot.push_str(&format!(
            "related: {}..{}: {}\n",
            related.span.start, related.span.end, related.message
        ));
    }
    snapshot
}

fn lower(source: &str) -> mir::Program {
    doriac::lower_source_to_mir("stage23a.doria", source).expect("source should lower to MIR")
}

fn interpret(source: &str) -> doriac::mir_interpreter::InterpreterOutput {
    doriac::mir_interpreter::interpret(&lower(source)).expect("MIR should interpret")
}

#[test]
fn named_arguments_bind_across_every_callable_form() {
    let output = interpret(NAMED_ARGUMENTS_EXAMPLE);
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        concat!(
            "free|Ada|36|London\n",
            "free|Grace|42|unknown\n",
            "free|Alan|41|Cambridge\n",
            "method|3|shop|5\n",
            "3\n",
            "default|7|shop|5\n",
            "7\n",
            "static|static|2|true\n",
            "2\n",
            "55:55:64\n",
        )
    );
}

#[test]
fn named_arguments_evaluate_in_source_order() {
    // Decision 0098: `pair(b: g(), a: h())` runs `g()` then `h()` — the written
    // order — and then binds those results to `b` and `a` by name.
    let output = interpret(NAMED_ARGUMENT_ORDER_EXAMPLE);
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        concat!("gh|a=2 b=1\n", "gkh|a=1 b=2 c=3\n", "hg|a=2 b=1\n")
    );
}

#[test]
fn reordered_repeat_literals_panic_before_later_arguments_run() {
    let source = r#"
function marker(): int
{
    echo "marker\n";
    return 1;
}

function sink(int $first, List<bool> $second): void {}

function route(int $count): void
{
    sink(second: [true; $count], first: marker());
}

function main(): void
{
    route(-1);
}
"#;
    let output = interpret(source);
    assert_eq!(output.exit_status, 101);
    assert!(
        output.stdout.is_empty(),
        "the later marker must not run before the source-earlier fill panic"
    );
    let diagnostic = output
        .runtime_diagnostic
        .expect("fill panic should retain a structured diagnostic");
    assert_eq!(diagnostic.code, "P1311");
    assert_eq!(diagnostic.title, "Collection Fill Count Cannot Be Negative");
}

#[test]
fn reordered_owned_arguments_move_through_tracked_temporaries() {
    let output = interpret(NAMED_OWNED_ARGUMENTS_EXAMPLE);
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout is UTF-8"),
        NAMED_OWNED_ARGUMENTS_STDOUT
    );
}

#[test]
fn a_named_argument_may_skip_a_defaulted_middle_parameter() {
    // The case decision 0086 could not express positionally: `second` and
    // `third` keep their defaults while `fourth` is supplied by name.
    let source = r#"
function middle(int $first, int $second = 20, int $third = 30, int $fourth = 40): int
{
    return $first + $second + $third + $fourth;
}

function main(): void
{
    echo middle(1, fourth: 4);
    echo ":";
    echo middle(1, third: 3);
    echo "\n";
}
"#;
    assert_eq!(
        String::from_utf8(interpret(source).stdout).expect("stdout is UTF-8"),
        "55:64\n"
    );
}

#[test]
fn supplying_one_parameter_twice_is_rejected() {
    let named_twice = r#"
function save(string $name, int $size): void {}

function main(): void
{
    save(name: "a", name: "b");
}
"#;
    insta_like_snapshot(
        "named_argument_duplicate_named",
        &diagnostic_snapshot(named_twice, "E0517"),
    );

    let positional_and_named = r#"
function save(string $name, int $size): void {}

function main(): void
{
    save("a", name: "b");
}
"#;
    insta_like_snapshot(
        "named_argument_duplicate_positional",
        &diagnostic_snapshot(positional_and_named, "E0517"),
    );
}

#[test]
fn an_unknown_parameter_name_is_rejected() {
    let source = r#"
function save(string $name, int $size): void {}

function main(): void
{
    save(name: "a", nmae: 1);
}
"#;
    insta_like_snapshot(
        "named_argument_unknown",
        &diagnostic_snapshot(source, "E0516"),
    );
}

#[test]
fn a_missing_required_parameter_is_rejected() {
    let source = r#"
function save(string $name, int $size): void {}

function main(): void
{
    save(name: "a");
}
"#;
    insta_like_snapshot(
        "named_argument_missing",
        &diagnostic_snapshot(source, "E0518"),
    );
}

#[test]
fn a_positional_argument_may_not_follow_a_named_argument() {
    // Parser-level: the ordering rule is grammar, so it never reaches semantics.
    let source = r#"
function save(string $name, int $size): void {}

function main(): void
{
    save(name: "a", 1);
}
"#;
    let found = doriac::parse_source("stage23a.doria", source)
        .expect_err("a positional argument may not follow a named argument");
    let diagnostic = found
        .iter()
        .find(|diagnostic| diagnostic.code == "E0515")
        .unwrap_or_else(|| panic!("expected E0515, got {found:#?}"));
    assert_eq!(
        diagnostic.message,
        "a positional argument cannot follow a named argument"
    );
    assert_eq!(
        diagnostic.related.len(),
        1,
        "the diagnostic points back at the named argument"
    );
}

#[test]
fn borrow_conflicts_in_a_named_call_follow_source_order() {
    // Decision 0098: the one-writer-XOR-many-readers rule is checked over the
    // written order, so the *second written* argument is the conflicting one
    // even though it binds the first parameter.
    let source = r#"
class Counter
{
    writable int $value = 0;
}

function tally(writable Counter $sink, Counter $source): void {}

function main(): void
{
    let writable $c = new Counter();
    tally(source: $c, sink: $c);
}
"#;
    let found = diagnostics(source);
    let conflict = found
        .iter()
        .find(|diagnostic| diagnostic.code == "E0477")
        .unwrap_or_else(|| panic!("expected E0477, got {found:#?}"));
    let sink_argument = source.rfind("$c").expect("the second `$c` is written last");
    assert_eq!(
        conflict.span.start, sink_argument,
        "the conflict is reported at the later-written argument"
    );
}

#[test]
fn intrinsics_do_not_accept_named_arguments() {
    // Parameter names are public API for user callables only; language
    // intrinsics keep positional-only binding.
    let source = r#"
function main(): void
{
    write_stderr(value: "x");
}
"#;
    let found = diagnostics(source);
    assert!(
        found.iter().any(|diagnostic| diagnostic.code == "E0519"),
        "expected E0519, got {found:#?}"
    );
}

/// Compare against a checked-in snapshot file, writing it on first run when
/// `UPDATE_DIAGNOSTIC_SNAPSHOTS` is set.
fn insta_like_snapshot(name: &str, actual: &str) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/diagnostics")
        .join(format!("{name}.txt"));
    if std::env::var_os("UPDATE_DIAGNOSTIC_SNAPSHOTS").is_some() {
        std::fs::write(&path, actual).expect("snapshot should be writable");
        return;
    }
    let expected = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("missing snapshot {}: {error}", path.display()));
    assert_eq!(actual, expected, "snapshot mismatch for {name}");
}
