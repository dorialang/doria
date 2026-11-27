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
    int $displayWidth = $text->displayWidth;
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
    let $codePoints = $text->codePoints;
    String::compare("a", "b");
}
"#,
    );
    assert!(deferred.iter().any(|diagnostic| {
        diagnostic.kind == DiagnosticKind::UnsupportedDevelopmentSurface
            && diagnostic.message.contains("codePoints")
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
        r#"function main(): void throws Doria\Std\Io\IoError { echo String::upper("Straße"); }"#,
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
function main(): void throws Doria\Std\Io\IoError
{
    Bytes $bytes = "abc"->bytes;
    echo $bytes->length;
}
"#,
    )
    .expect("String intrinsic recognition must respect the receiver's resolved type");
}

/// The Doria spellings a diagnostic's help lines name, in backticks, after
/// "write" or "use".
fn suggested_spellings(diagnostic: &Diagnostic) -> Vec<String> {
    diagnostic
        .helps
        .iter()
        .filter_map(|help| {
            let rest = help
                .strip_prefix("write `")
                .or_else(|| help.strip_prefix("use `"))?;
            Some(rest.split('`').next()?.to_string())
        })
        .collect()
}

#[test]
fn every_foreign_string_spelling_names_doria_that_compiles() {
    use doriac::builtins::{StringSpellingTarget, FOREIGN_STRING_SPELLINGS};

    for spelling in FOREIGN_STRING_SPELLINGS {
        let foreign = spelling.foreign;
        let found = diagnostics(&format!("function main(): void {{ {foreign}(\"a\"); }}"));
        let diagnostic = found
            .iter()
            .find(|diagnostic| diagnostic.code == "E0461")
            .unwrap_or_else(|| panic!("`{foreign}` should name its Doria spelling: {found:?}"));
        let expected_title = match spelling.target {
            StringSpellingTarget::Property(_) => "Use The String Property",
            _ => "Use The String Companion",
        };
        assert_eq!(diagnostic.title, expected_title, "`{foreign}`");

        let suggested = suggested_spellings(diagnostic);
        let expected_count = 1 + usize::from(spelling.alternative.is_some());
        assert_eq!(
            suggested.len(),
            expected_count,
            "`{foreign}`: {suggested:?}"
        );
        if matches!(spelling.target, StringSpellingTarget::PendingComparison(_)) {
            continue;
        }
        for spelling_text in suggested {
            let program = format!(
                r#"
function main(): void
{{
    let $text = "a";
    let $needle = "a";
    let $prefix = "a";
    let $suffix = "a";
    let $search = "a";
    let $replacement = "b";
    let $separator = ",";
    let $padding = " ";
    int $start = 0;
    int $length = 1;
    int $count = 2;
    List<string> $values = ["a", "b"];
    let $result = {spelling_text};
}}
"#
            );
            if let Err(errors) = doriac::check_source("suggested.doria", &program) {
                panic!(
                    "`{foreign}` suggests `{spelling_text}`, which does not compile: {errors:?}"
                );
            }
        }
    }
}

#[test]
fn foreign_string_spellings_offer_a_rename_only_when_the_arguments_fit() {
    let rename = |source: &str| -> Option<String> {
        diagnostics(source)
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0461")
            .flat_map(|diagnostic| diagnostic.fixes.iter())
            .filter(|fix| fix.applicability == FixApplicability::MachineApplicable)
            .flat_map(|fix| fix.edits.iter())
            .map(|edit| edit.replacement.clone())
            .next()
    };

    for (source, expected) in [
        (
            r#"function main(): void { strtolower("A"); }"#,
            Some("String::lower"),
        ),
        (
            r#"function main(): void { trim(" a "); }"#,
            Some("String::trim"),
        ),
        (
            r#"function main(): void { substr("abc", 1); }"#,
            Some("String::slice"),
        ),
        // A named argument carries PHP's parameter name, which the
        // companion operation does not declare. The count alone would fit.
        (
            r#"function main(): void { substr("abc", offset: 1); }"#,
            None,
        ),
        // PHP's second argument, the characters to strip, has no place in
        // `String::trim`.
        (r#"function main(): void { trim(" a ", " "); }"#, None),
        // The arguments are in a different order.
        (
            r#"function main(): void { str_replace("a", "b", "abc"); }"#,
            None,
        ),
        (r#"function main(): void { explode(",", "a,b"); }"#, None),
        // Two Doria spellings replace it, and the padding text is required.
        (r#"function main(): void { str_pad("a", 5); }"#, None),
        (r#"function main(): void { strlen("a"); }"#, None),
    ] {
        // A case that failed earlier, in the parser for example, would pass
        // a "no rename" expectation without testing anything.
        assert!(
            diagnostics(source)
                .iter()
                .any(|diagnostic| diagnostic.code == "E0461"),
            "{source} must reach the string spelling diagnostic"
        );
        assert_eq!(rename(source).as_deref(), expected, "{source}");
    }
}

#[test]
fn padding_with_the_php_spelling_names_both_companion_operations() {
    let found = diagnostics(
        r#"
class Window
{
    internal writable int $width = 4;

    function render(string $lineContent): void
    {
        echo str_pad($lineContent, $this->width);
    }
}

function main(): void
{
}
"#,
    );
    assert!(
        !found.iter().any(|diagnostic| diagnostic.code == "E0309"),
        "{found:?}"
    );
    let diagnostic = found
        .iter()
        .find(|diagnostic| diagnostic.code == "E0461")
        .unwrap_or_else(|| panic!("expected the string spelling diagnostic: {found:?}"));
    assert_eq!(
        suggested_spellings(diagnostic),
        [
            "String::padEnd($text, $length, $padding)",
            "String::padStart($text, $length, $padding)",
        ]
    );
}

/// The String surface table is the checker's only source: every executable
/// member type-checks with the declared result type, and every pending member
/// reports E0304. A member that needs an unsafe context is written in one.
#[test]
fn string_surface_table_matches_the_checker() {
    use doriac::string_surface::{StringMemberKind, StringValueType, STRING_MEMBERS};

    fn argument(ty: StringValueType) -> &'static str {
        match ty {
            StringValueType::String => "\"a\"",
            StringValueType::Int => "1",
            StringValueType::Bool => "true",
            StringValueType::Bytes => "\"a\"->bytes",
            StringValueType::NullableInt => "null",
            StringValueType::NullableString => "null",
            StringValueType::StringList => "[\"a\"]",
            StringValueType::CStringPointer => "$cText->pointer",
            StringValueType::Utf16Pointer => "$utf16Text->pointer",
            other => panic!("no sample argument for {other:?}"),
        }
    }

    for member in STRING_MEMBERS {
        let expression = match member.kind {
            StringMemberKind::Property => format!("$text->{}", member.name),
            StringMemberKind::Companion => {
                let arguments = member
                    .parameters
                    .iter()
                    .filter(|parameter| parameter.default.is_none())
                    .map(|parameter| argument(parameter.ty))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("String::{}({arguments})", member.name)
            }
        };
        let expression = if member.unsafe_operation.is_some() {
            format!("unsafe({expression})")
        } else {
            expression
        };
        let declaration = match member.result {
            StringValueType::ByteRegion
            | StringValueType::StringIterable
            | StringValueType::Ordering
            | StringValueType::Undecided => "let".to_string(),
            result => result.source().to_string(),
        };
        let source = format!(
            "use Doria\\Std\\Ffi\\{{CString, Utf16String}};\n\
             function main(): void\n{{\n    let $text = \"a\";\n    \
             let $cText = CString::from($text);\n    \
             let $utf16Text = Utf16String::from($text);\n    \
             {declaration} $value = {expression};\n}}\n"
        );
        let result = doriac::check_source("surface.doria", &source);
        if member.is_executable() {
            assert!(
                result.is_ok(),
                "`{}` should type-check as {declaration}: {:#?}",
                member.spelling(),
                result.err()
            );
        } else {
            let diagnostics = result.expect_err("a pending member is reported");
            assert!(
                diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.code == "E0304"),
                "pending `{}` should report E0304: {diagnostics:#?}",
                member.spelling()
            );
        }
    }
}

/// `$text->graphemes` is an `Iterable<string>`, and the classes behind it are
/// compiler-private: authored source cannot name them (Decision 0147).
#[test]
fn graphemes_is_an_iterable_view_whose_classes_cannot_be_named() {
    let source = r#"
function count(Iterable<string> $items): int
{
    writable int $total = 0;
    foreach ($items as string $item) {
        $total++;
    }
    return $total;
}

function main(): void
{
    let $text = "a👍🏾b";
    Iterable<string> $view = $text->graphemes;
    int $total = count($view);
    foreach ($text->graphemes as string $grapheme) {
        int $width = $grapheme->displayWidth;
    }
}
"#;
    doriac::lower_source_to_mir("graphemes.doria", source)
        .expect("$text->graphemes type-checks and lowers as Iterable<string>");

    for name in [
        "Doria\\Std\\Text\\GraphemeView",
        "Doria\\Std\\Text\\GraphemeCursor",
    ] {
        let source =
            format!("function main(): void\n{{\n    let $value = new {name}(\"a\");\n}}\n");
        assert!(
            doriac::check_source("private.doria", &source).is_err(),
            "authored source must not reach `{name}`"
        );
    }
}

/// A program that does no I/O can walk graphemes: the view's interface slots
/// bring the I/O identities the checker needs, although no authored name
/// refers to them.
#[test]
fn graphemes_compile_in_programs_without_io() {
    for (name, source) in [
        (
            "function",
            "function cells(string $text): int\n{\n    writable int $total = 0;\n    foreach ($text->graphemes as string $g) {\n        $total += $g->displayWidth;\n    }\n    return $total;\n}\n\nfunction main(): int\n{\n    return cells(\"a\");\n}\n",
        ),
        (
            "library method",
            "class Line\n{\n    function __construct(internal string $text) {}\n\n    function cells(): int\n    {\n        writable int $total = 0;\n        foreach ($this->text->graphemes as string $g) {\n            $total += $g->displayWidth;\n        }\n        return $total;\n    }\n}\n",
        ),
    ] {
        // A library has no `main` to lower, so it is checked; a program is
        // lowered to MIR as well.
        let result = if source.contains("function main") {
            doriac::lower_source_to_mir("no-io.doria", source).map(|_| ())
        } else {
            doriac::check_source("no-io.doria", source).map(|_| ())
        };
        result.unwrap_or_else(|diagnostics| panic!("{name}: {diagnostics:#?}"));
    }
}
