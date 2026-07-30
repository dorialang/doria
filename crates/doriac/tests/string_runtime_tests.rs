use doriac::backend::{BackendTarget, CompileOptions};
use doriac::diagnostics::{Diagnostic, DiagnosticKind, FixApplicability};

fn diagnostics(source: &str) -> Vec<Diagnostic> {
    doriac::check_source("string-runtime.doria", source)
        .expect_err("source should produce diagnostics")
}

#[test]
fn minimum_string_surface_has_canonical_types_and_named_arguments() {
    let source = r#"
function main(): void
{
    let $text = " Dória 👍🏾 ";
    int $length = $text->length;
    int $byteLength = $text->byteLength;
    bool $empty = $text->isEmpty;
    Bytes $bytes = $text->bytes;

    string $trimmed = String::trim(text: $text);
    string $trimStart = String::trimStart(text: $text);
    string $trimEnd = String::trimEnd(text: $text);
    string $lower = String::lower(text: $text);
    string $upper = String::upper(text: $text);
    string $lowerFirst = String::lowerFirst(text: $text);
    string $upperFirst = String::upperFirst(text: $text);
    bool $contains = String::contains(needle: "Dória", text: $text);
    bool $starts = String::startsWith(prefix: " ", text: $text);
    bool $ends = String::endsWith(suffix: " ", text: $text);
    bool $containsFolded = String::containsIgnoreCase(needle: "DÓRIA", text: $text);
    bool $startsFolded = String::startsWithIgnoreCase(prefix: " ", text: $text);
    bool $endsFolded = String::endsWithIgnoreCase(suffix: " ", text: $text);
    bool $folded = String::equalsIgnoreCase(right: "STRASSE", left: "Straße");
    ?int $first = String::indexOf(needle: "👍🏾", text: $text);
    ?int $last = String::lastIndexOf(needle: " ", text: $text);
    ?int $firstFolded = String::indexOfIgnoreCase(needle: "DÓRIA", text: $text);
    ?int $lastFolded = String::lastIndexOfIgnoreCase(needle: " ", text: $text);
    int $occurrences = String::countOccurrences(needle: " ", text: $text);
    string $replaced = String::replace(
        replacement: "Doria",
        search: "Dória",
        text: $text,
    );
    List<string> $parts = String::split(separator: " ", text: $text);
    string $joined = String::join(values: $parts, separator: "|");
    string $slice = String::slice(length: 2, start: -2, text: $text);
    string $tail = String::slice(start: 1, text: $text);
    string $repeated = String::repeat(count: 2, text: $text);
    string $leftPad = String::padStart(padding: ".", length: 20, text: $text);
    string $rightPad = String::padEnd(padding: ".", length: 20, text: $text);
    ?string $decoded = String::fromBytes(bytes: $bytes);
}
"#;

    doriac::lower_source_to_mir("string-runtime.doria", source)
        .expect("the complete minimum String surface should type-check and lower");
}

#[test]
fn minimum_string_surface_rejects_wrong_calls_and_reserves_the_companion() {
    let cases = [
        ("wrong arity", "function main(): void { String::trim(); }"),
        (
            "wrong type",
            "function main(): void { String::repeat(\"x\", \"2\"); }",
        ),
        (
            "wrong name",
            "function main(): void { String::contains(text: \"x\", value: \"x\"); }",
        ),
        (
            "companion redeclaration",
            "class String { function value(): int { return 1; } }",
        ),
    ];

    for (name, source) in cases {
        let diagnostics = diagnostics(source);
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.kind == DiagnosticKind::Language),
            "{name} should be rejected as a language error: {diagnostics:#?}"
        );
    }
}

#[test]
fn deferred_and_removed_string_surfaces_have_structured_guidance() {
    let deferred = diagnostics(
        r#"
function main(): void
{
    let $text = "Doria";
    let $graphemes = $text->graphemes;
    String::compare("a", "b");
}
"#,
    );
    assert!(deferred.iter().any(|diagnostic| {
        diagnostic.kind == DiagnosticKind::UnsupportedDevelopmentSurface
            && diagnostic.message.contains("grapheme")
    }));
    assert!(deferred.iter().any(|diagnostic| {
        diagnostic.kind == DiagnosticKind::UnsupportedDevelopmentSurface
            && diagnostic.message.contains("Ordering")
    }));
    assert!(!deferred
        .iter()
        .any(|diagnostic| diagnostic.message.contains("unknown property")));

    for (source, canonical) in [
        (
            r#"function main(): void { str_starts_with("Doria", "Dor"); }"#,
            "String::startsWith",
        ),
        (
            r#"function main(): void { let $text = " Doria "; $text->trim(); }"#,
            "String::trim",
        ),
        (
            r#"function main(): void { explode(",", "one,two"); }"#,
            "String::split",
        ),
    ] {
        let diagnostics = diagnostics(source);
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic.title == "Use The String Companion"
                && diagnostic.helps.iter().any(|help| help.contains(canonical))
        }));
    }

    let direct = diagnostics(r#"function main(): void { str_starts_with("Doria", "Dor"); }"#);
    assert!(direct.iter().any(|diagnostic| {
        diagnostic.fixes.iter().any(|fix| {
            fix.applicability == FixApplicability::MachineApplicable
                && fix
                    .edits
                    .iter()
                    .any(|edit| edit.replacement == "String::startsWith")
        })
    }));
}

#[test]
fn php_backend_refuses_string_intrinsics_instead_of_changing_semantics() {
    let error = doriac::compile_source_with_options(
        "string-runtime.doria",
        r#"function main(): void { echo String::upper("Straße"); }"#,
        CompileOptions::new(BackendTarget::Php),
    )
    .expect_err("PHP must refuse Unicode String intrinsics it cannot preserve");

    assert!(error.iter().any(|diagnostic| {
        diagnostic.code == "B2501" && diagnostic.kind == DiagnosticKind::Backend
    }));
}

#[test]
fn string_property_recognition_does_not_capture_other_length_properties() {
    doriac::lower_source_to_mir(
        "bytes-length.doria",
        r#"
function main(): void
{
    Bytes $bytes = "abc"->bytes;
    echo $bytes->length;
}
"#,
    )
    .expect("String intrinsic recognition must respect the receiver's resolved type");
}
