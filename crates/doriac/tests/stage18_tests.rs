use doriac::diagnostics::Diagnostic;
use std::path::{Path, PathBuf};

fn diagnostic_snapshot(source: &str, code: &str, semantic: bool) -> String {
    let diagnostics = if semantic {
        doriac::check_source("stage18.doria", source)
    } else {
        doriac::parse_source("stage18.doria", source)
    }
    .expect_err("fixture must produce diagnostics");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("expected {code}, got {diagnostics:?}"));

    render_snapshot(diagnostic)
}

fn render_snapshot(diagnostic: &Diagnostic) -> String {
    let mut snapshot = format!(
        "code: {}\nmessage: {}\nhelp: {}\nspan: {}..{}\n",
        diagnostic.code,
        diagnostic.message,
        diagnostic.help.as_deref().unwrap_or(""),
        diagnostic.span.start,
        diagnostic.span.end,
    );
    if let Some(fix) = &diagnostic.fix {
        snapshot.push_str(&format!(
            "fix: {}..{} -> {:?}\n",
            fix.span.start, fix.span.end, fix.replacement
        ));
    }
    snapshot
}

fn assert_snapshot(actual: String, expected: &str) {
    assert_eq!(actual, expected.replace("\r\n", "\n"));
}

fn doria_files_under(root: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(root).expect("example directory should be readable") {
        let path = entry.expect("example entry should be readable").path();
        if path.is_dir() {
            doria_files_under(&path, files);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("doria") {
            files.push(path);
        }
    }
}

#[test]
fn repository_doria_examples_remain_parseable_after_the_literal_brace_change() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let examples = workspace.join("examples");
    let mut files = Vec::new();
    doria_files_under(&examples, &mut files);
    files.sort();

    assert!(!files.is_empty());
    for path in files {
        let source = std::fs::read_to_string(&path).expect("Doria example should be UTF-8");
        doriac::parse_source(path.display().to_string(), source)
            .unwrap_or_else(|diagnostics| panic!("{}: {diagnostics:?}", path.display()));
    }
}

#[test]
fn full_expression_interpolation_accepts_every_stage_18_primitive_shape() {
    doriac::check_source(
        "stage18.doria",
        r#"
function intValue(): int { return 20; }
function floatValue(): float { return 1.5; }
function boolValue(): bool { return true; }
function stringValue(): string { return "Doria"; }

function main(): void
{
    int $right = 22;
    echo "{intValue() + $right}";
    echo "{floatValue() + 2.5}";
    echo "{boolValue() and true}";
    echo "{stringValue() . " language"}";
    echo "{($right == 22) or false}";
    echo "{Int::from(42)}";
}
"#,
    )
    .expect("the ordinary expression grammar should be available in interpolation");
}

#[test]
fn interpolation_preserves_nullable_narrowing_and_excluded_type_rules() {
    doriac::check_source(
        "stage18.doria",
        r#"
function main(): void
{
    let $line = read_line();
    if ($line != null) {
        echo "line={$line}";
    }
}
"#,
    )
    .expect("a proven non-null string should be displayable");

    for source in [
        "function main(): void { let $line = read_line(); echo \"{$line}\"; }",
        "function show(mixed $value): void { echo \"{$value}\"; }",
        "function show(List<int> $value): void { echo \"{$value}\"; }",
        "function show(Dictionary<string, int> $value): void { echo \"{$value}\"; }",
        "function show(Set<int> $value): void { echo \"{$value}\"; }",
        "function main(): void { let $value = null; echo \"{$value}\"; }",
    ] {
        doriac::check_source("stage18.doria", source)
            .expect_err("excluded values must not become display-convertible");
    }
}

#[test]
fn ordinary_expression_errors_are_preserved_inside_interpolation() {
    let missing = doriac::check_source(
        "stage18.doria",
        "function main(): void { echo \"{$missing}\"; }",
    )
    .expect_err("an undeclared interpolation binding must fail");
    assert!(missing.iter().any(|diagnostic| diagnostic.code == "E0101"));

    let arithmetic = doriac::check_source(
        "stage18.doria",
        "function main(): void { echo \"{1 + true}\"; }",
    )
    .expect_err("invalid arithmetic must remain an arithmetic error");
    assert!(arithmetic
        .iter()
        .any(|diagnostic| diagnostic.code == "E0441"));
}

#[test]
fn acceptance_example_executes_with_exact_output() {
    let source = include_str!("../../../examples/native/main_expression_interpolation.doria");
    let mir = doriac::lower_source_to_mir("main_expression_interpolation.doria", source)
        .expect("the acceptance example should lower to MIR");
    let output = doriac::mir_interpreter::interpret(&mir).expect("the example should interpret");

    assert_eq!(output.stdout, b"sum: 42");
    assert!(output.stderr.is_empty());
    assert_eq!(output.exit_status, 0);
}

#[test]
fn single_quoted_brace_backslashes_remain_literal_at_runtime() {
    let mir = doriac::lower_source_to_mir(
        "single_quotes.doria",
        r#"function main(): void { echo '\{\}'; }"#,
    )
    .expect("single-quoted brace text should lower");
    let output =
        doriac::mir_interpreter::interpret(&mir).expect("single-quoted brace text should execute");

    assert_eq!(output.stdout, b"\\{\\}");
}

#[test]
fn empty_interpolation_diagnostic_matches_snapshot() {
    assert_snapshot(
        diagnostic_snapshot("echo \"{}\";", "P0001", false),
        include_str!("fixtures/diagnostics/stage18_empty_interpolation.txt"),
    );
}

#[test]
fn malformed_interpolation_diagnostic_matches_snapshot() {
    assert_snapshot(
        diagnostic_snapshot("echo \"{1 + }\";", "P0001", false),
        include_str!("fixtures/diagnostics/stage18_malformed_interpolation.txt"),
    );
}

#[test]
fn unterminated_interpolation_diagnostic_matches_snapshot() {
    assert_snapshot(
        diagnostic_snapshot("echo \"{1 + 2\";", "L0001", false),
        include_str!("fixtures/diagnostics/stage18_unterminated_interpolation.txt"),
    );
}

#[test]
fn literal_open_brace_fix_matches_snapshot() {
    assert_snapshot(
        diagnostic_snapshot("echo \"literal {word}\";", "P0002", false),
        include_str!("fixtures/diagnostics/stage18_literal_open_brace_fix.txt"),
    );
}

#[test]
fn non_displayable_class_diagnostic_matches_snapshot() {
    assert_snapshot(
        diagnostic_snapshot(
            "class Token {} let $token = new Token(); echo \"{$token}\";",
            "E0462",
            true,
        ),
        include_str!("fixtures/diagnostics/stage18_non_displayable_class.txt"),
    );
}

#[test]
fn invalid_displayable_signature_diagnostic_matches_snapshot() {
    assert_snapshot(
        diagnostic_snapshot(
            "class Label implements Displayable { static function toString(): string { return \"Doria\"; } }",
            "E0463",
            true,
        ),
        include_str!("fixtures/diagnostics/stage18_invalid_displayable_signature.txt"),
    );
}

#[test]
fn php_magic_method_guidance_matches_snapshot() {
    assert_snapshot(
        diagnostic_snapshot(
            "class Label implements Displayable { function __toString(): string { return \"Doria\"; } }",
            "E0463",
            true,
        ),
        include_str!("fixtures/diagnostics/stage18_php_magic_method_guidance.txt"),
    );
}
