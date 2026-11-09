use doriac::diagnostics::{ColorChoice, DiagnosticFormat, FixApplicability, RenderOptions};

fn fixture(name: &str) -> &'static str {
    match name {
        "readonly_property_write" => {
            include_str!("fixtures/diagnostics/readonly_property_write.doria")
        }
        "use_after_move" => include_str!("fixtures/diagnostics/use_after_move.doria"),
        "unknown_named_argument" => {
            include_str!("fixtures/diagnostics/unknown_named_argument.doria")
        }
        "type_mismatch_unicode" => {
            include_str!("fixtures/diagnostics/type_mismatch_unicode.doria")
        }
        "independent_errors" => include_str!("fixtures/diagnostics/independent_errors.doria"),
        _ => panic!("unknown diagnostic fixture {name}"),
    }
}

#[test]
fn representative_fixtures_exercise_the_human_first_contract() {
    let cases = [
        (
            "readonly_property_write",
            "E0202",
            "Cannot Write to Readonly Property `value`",
        ),
        (
            "use_after_move",
            "E0470",
            "`$guard` Cannot Be Used After Its Value Was Given Away",
        ),
        ("unknown_named_argument", "E0516", "Unknown Named Argument"),
        ("type_mismatch_unicode", "E0403", "Type Mismatch"),
    ];
    for (name, code, title) in cases {
        let source = fixture(name);
        let path = format!("{name}.doria");
        let diagnostics =
            doriac::check_source(&path, source).expect_err("fixture must be rejected");
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == code)
            .unwrap_or_else(|| panic!("{name} must emit {code}: {diagnostics:#?}"));
        assert_eq!(diagnostic.title, title);
        assert!(diagnostic.explanation.is_some());
        assert!(diagnostic
            .labels
            .iter()
            .any(|label| !label.message.is_empty()));

        let human = doriac::render_diagnostics_with_options(
            &path,
            source,
            &diagnostics,
            RenderOptions {
                color: ColorChoice::Never,
                terminal_width: 72,
                ..RenderOptions::default()
            },
        );
        assert!(human.contains(&format!("Error[{code}]: {title}")));
        assert!(human.contains("\nWhy\n"));
        assert!(human.ends_with(&format!(
            "Compilation Failed: {} Error{}",
            diagnostics.len(),
            if diagnostics.len() == 1 { "" } else { "s" }
        )));
    }
}

#[test]
fn readonly_property_human_output_matches_the_reviewed_design_artifact() {
    let source = fixture("readonly_property_write");
    let diagnostics = doriac::check_source("readonly_property_write.doria", source)
        .expect_err("readonly property write must fail");
    let actual = doriac::render_diagnostics_with_options(
        "readonly_property_write.doria",
        source,
        &diagnostics,
        RenderOptions {
            color: ColorChoice::Never,
            terminal_width: 72,
            ..RenderOptions::default()
        },
    );
    let expected =
        include_str!("fixtures/diagnostics/readonly_property_write.human.txt").trim_end();
    assert_eq!(actual, expected);
}

#[test]
fn named_argument_fixture_exposes_only_a_semantically_certain_automatic_fix() {
    let source = fixture("unknown_named_argument");
    let diagnostics = doriac::check_source("unknown_named_argument.doria", source)
        .expect_err("misspelled named argument must fail");
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "E0516")
        .unwrap();
    let fix = diagnostic.fixes.first().expect("clear typo has a fix");
    assert_eq!(fix.applicability, FixApplicability::MachineApplicable);
    assert_eq!(fix.edits.len(), 1);
    assert_eq!(fix.edits[0].replacement, "size");
}

#[test]
fn independent_errors_survive_all_presentations_without_duplicates() {
    let source = fixture("independent_errors");
    let diagnostics = doriac::check_source("independent_errors.doria", source)
        .expect_err("both undeclared assignments must fail");
    assert_eq!(
        diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "E0101")
            .count(),
        2
    );

    let concise = doriac::render_diagnostics_with_options(
        "independent_errors.doria",
        source,
        &diagnostics,
        RenderOptions {
            format: DiagnosticFormat::Concise,
            color: ColorChoice::Never,
            ..RenderOptions::default()
        },
    );
    assert_eq!(concise.matches("Error[E0101]").count(), 2);

    let json = doriac::render_diagnostics_with_options(
        "independent_errors.doria",
        source,
        &diagnostics,
        RenderOptions {
            format: DiagnosticFormat::Json,
            color: ColorChoice::Always,
            ..RenderOptions::default()
        },
    );
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["schemaVersion"], 1);
    assert_eq!(value["summary"]["errors"], 2);
    assert!(!json.contains('\u{1b}'));
}

#[test]
fn live_loan_conflicts_keep_the_same_earliest_source_across_analyses() {
    use doriac::diagnostics::{prepare_diagnostics, LabelRole};

    let cases = [
        (
            r#"
function main(): void {
    let writable $value = 1;
    let $earlier = fn() with ($value) => $value;
    let $later = fn() with ($value) => $value;
    $value = 2;
    echo $earlier(); echo $later();
}
"#,
            "E0654",
            "with ($value)",
            "$value",
        ),
        (
            r#"
class Cursor implements Iterator<int> {
    function __construct(borrow List<int> $source) {}
    function hasCurrent(): bool { return true; }
    function getCurrent(): int { return $this->source[0]; }
    writable function advance(): void {}
}
function main(): void {
    let writable $source = [7];
    let $earlier = new Cursor($source);
    let $later = new Cursor($source);
    $source->add(9);
    echo $earlier->getCurrent(); echo $later->getCurrent();
}
"#,
            "E0763",
            "new Cursor($source)",
            "$source",
        ),
    ];
    for (source, code, origin, binding) in cases {
        let origin = source.find(origin).unwrap();
        let start = origin + source[origin..].find(binding).unwrap();
        let expected = doriac::source::Span::new(start, start + binding.len());
        for _ in 0..16 {
            let compiler = doriac::check_source("live-loans.doria", source).unwrap_err();
            let (_, ide) = doriac::analyze_source_for_ide("live-loans.doria", source).unwrap();
            for diagnostics in [compiler, ide.diagnostics] {
                let prepared = prepare_diagnostics(&diagnostics);
                assert_eq!(prepared.len(), 1, "{prepared:#?}");
                assert_eq!(prepared[0].code, code);
                let related = prepared[0]
                    .labels
                    .iter()
                    .find(|label| label.role == LabelRole::Secondary)
                    .unwrap();
                assert_eq!(related.span, expected, "{prepared:#?}");
            }
        }
    }
}

/// A two-file project whose property hook reaches blocking output through a
/// helper declared in the other file.
const HOOK_SOURCE: &str = r#"class Counter {
    int $value {
        get {
            logValue();
            return 1;
        }
    }
}

function main(): int {
    let $counter = new Counter();
    return $counter->value;
}
"#;
const HELPER_SOURCE: &str = r#"// The helper lives in its own file, so its label must name that file.


function logValue(): void {
    echo "reading";
}
"#;
const HELPER_DISPLAY_PATH: &str = "acme/application:helpers.doria";

/// A user adapter whose `read` does not match the standard contract it
/// implements. The contract is declared in a compiler-owned source.
const STANDARD_CONTRACT_SOURCE: &str = r#"use Doria\Std\Io\{PendingReadableStream, ReadRequest, ReadOutcome, ReadinessSource, WritableByteRegion, IoError};

class ForwardingReader implements PendingReadableStream {
    internal writable PendingReadableStream $source;
    function __construct(parameter take PendingReadableStream $source) { $this->source = $source; }
    ReadinessSource $readinessSource { borrowed get => $this->source->readinessSource; }
    writable function read(writable WritableByteRegion $destination): int throws IoError {
        return 0;
    }
    writable function prepareRead(writable ReadRequest $request, writable WritableByteRegion $destination): void throws IoError {
    }
    writable function completeRead(ReadRequest $request, writable WritableByteRegion $destination): ?ReadOutcome throws IoError {
        return null;
    }
}

function main(): int {
    return 0;
}
"#;

fn two_file_plan(second: &str) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "schemaVersion": 1,
        "edition": "2026",
        "rootPackage": "acme/application",
        "selectedTarget": {
            "package": "acme/application",
            "name": "application",
            "kind": "binary",
            "entrySource": "acme/application:main.doria",
            "activeScopes": ["main"]
        },
        "packages": [{
            "identity": "acme/application",
            "root": ".",
            "namespaceMappings": [
                { "prefix": "", "path": "", "scope": "main", "generatedFor": null }
            ],
            "sources": [
                { "identity": "acme/application:main.doria", "path": "main.doria", "scope": "main", "origin": "entry", "generatedFor": null },
                { "identity": format!("acme/application:{second}"), "path": second, "scope": "main", "origin": "explicit", "generatedFor": null }
            ],
            "dependencies": []
        }],
        "compiler": { "target": "debug", "nativeProfile": null, "targetTriple": null }
    }))
    .unwrap()
}

/// Writes a two-file project and runs `doriac check` on its build plan with
/// the given diagnostic options.
fn check_project(
    name: &str,
    main: &str,
    second: (&str, &str),
    options: &[&str],
) -> std::process::Output {
    let directory = std::env::temp_dir().join(format!(
        "doriac-diagnostic-source-{name}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("main.doria"), main).unwrap();
    std::fs::write(directory.join(second.0), second.1).unwrap();
    std::fs::write(directory.join("plan.json"), two_file_plan(second.0)).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_doriac"))
        .current_dir(&directory)
        .arg("check")
        .args(options)
        .args(["--build-plan", "plan.json"])
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&directory);
    output
}

/// Every label names the source its span is in. A related label in another
/// file of the project renders that file's text, and its structured source is
/// that file, whichever compiler phase produced the diagnostic.
#[test]
fn multi_file_related_labels_name_the_source_their_span_is_in() {
    use doriac::diagnostics::{
        prepare_diagnostics_with_source_map, Diagnostic, DiagnosticSource, LabelRole,
    };

    let human = check_project(
        "human",
        HOOK_SOURCE,
        ("helpers.doria", HELPER_SOURCE),
        &["--diagnostic-color", "never"],
    );
    assert_eq!(human.status.code(), Some(1));
    let stderr = String::from_utf8(human.stderr).unwrap();
    assert!(
        stderr.contains(concat!(
            "Related\n",
            "acme/application:helpers.doria · line 5\n",
            "\n",
            "  4      function logValue(): void {\n",
            "  5          echo \"reading\";\n",
            "             ---------------\n",
            "             Potentially Blocking Operation\n",
        )),
        "{stderr}"
    );
    assert!(
        stderr.contains("acme/application:main.doria · line 3"),
        "{stderr}"
    );

    let json = check_project(
        "json",
        HOOK_SOURCE,
        ("helpers.doria", HELPER_SOURCE),
        &["--json"],
    );
    let json: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    let labels = json["diagnostics"][0]["labels"].as_array().unwrap();
    assert_eq!(json["diagnostics"][0]["code"], "E0770");
    assert_eq!(labels[0]["role"], "primary");
    assert_eq!(labels[0]["source"], "acme/application:main.doria");
    assert_eq!(labels[1]["role"], "secondary");
    assert_eq!(labels[1]["source"], HELPER_DISPLAY_PATH);
    assert_eq!(
        labels[1]["range"]["start"],
        serde_json::json!({ "line": 5, "column": 5 })
    );

    // The structured label the language server reads carries the same source.
    let document =
        doriac::build_plan::parse_build_plan_document("plan.json", two_file_plan("helpers.doria"))
            .unwrap();
    let mut provider = doriac::source_provider::InMemorySourceProvider::new();
    provider.insert("acme/application", "main.doria", HOOK_SOURCE);
    provider.insert("acme/application", "helpers.doria", HELPER_SOURCE);
    let graph = doriac::compilation_graph::load_compilation_graph(&document, &provider).unwrap();
    let helper = graph
        .source_map
        .by_path(HELPER_DISPLAY_PATH)
        .expect("the helper is a graph source");
    let analysis = doriac::analyze_compilation_graph_for_ide(&graph);
    let prepared = prepare_diagnostics_with_source_map(&graph.source_map, &analysis.diagnostics);
    assert_eq!(prepared.len(), 1, "{prepared:#?}");
    let related = prepared[0]
        .labels
        .iter()
        .find(|label| label.role == LabelRole::Secondary)
        .unwrap();
    assert_eq!(
        related.source,
        DiagnosticSource::Path(HELPER_DISPLAY_PATH.to_string())
    );
    assert_eq!(related.span.source, helper.source.id);
    assert_eq!(
        &HELPER_SOURCE[related.span.start..related.span.end],
        "echo \"reading\";"
    );

    // Producers after semantic analysis build labels the same way. Preparation
    // resolves them with no per-diagnostic or per-phase handling.
    let primary = prepared[0].span;
    let later = Diagnostic::new(
        prepared[0].code,
        "property hooks must not perform potentially blocking work",
        primary,
    )
    .with_related(related.span, "Potentially Blocking Operation");
    assert_eq!(later.labels[1].source, DiagnosticSource::Current);
    let later = prepare_diagnostics_with_source_map(&graph.source_map, &[later]);
    assert_eq!(
        later[0].labels[0].source,
        DiagnosticSource::Path("acme/application:main.doria".to_string())
    );
    assert_eq!(
        later[0].labels[1].source,
        DiagnosticSource::Path(HELPER_DISPLAY_PATH.to_string())
    );
}

/// No reader can open a compiler-owned source, so none of its locations reach
/// a presentation, with or without a source map.
#[test]
fn compiler_owned_locations_never_reach_a_presentation() {
    use doriac::diagnostics::{
        prepare_diagnostics, prepare_diagnostics_with_source_map,
        render_diagnostics_with_source_map, Diagnostic, DiagnosticSource, FixApplicability,
        FixEdit, LabelRole,
    };

    // Single source: the standard contract's declaration is not a label.
    let diagnostics = doriac::check_source("reader.doria", STANDARD_CONTRACT_SOURCE).unwrap_err();
    let mismatch = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "E0755")
        .unwrap_or_else(|| panic!("{diagnostics:#?}"));
    assert!(
        mismatch
            .labels
            .iter()
            .any(|label| label.span.source.is_compiler_owned()),
        "the fixture must exercise a label in a compiler-owned source: {mismatch:#?}"
    );
    let prepared = prepare_diagnostics(&diagnostics);
    assert!(prepared
        .iter()
        .flat_map(|diagnostic| &diagnostic.labels)
        .all(|label| !label.span.source.is_compiler_owned()));
    let human = doriac::render_diagnostics_with_options(
        "reader.doria",
        STANDARD_CONTRACT_SOURCE,
        &diagnostics,
        RenderOptions {
            color: ColorChoice::Never,
            ..RenderOptions::default()
        },
    );
    assert!(!human.contains("Related"), "{human}");
    assert!(!human.contains("line 20"), "{human}");

    // Project: no label falls back to another file's text.
    let output = check_project(
        "standard-contract",
        STANDARD_CONTRACT_SOURCE,
        (
            "aaa.doria",
            "// sorts before main.doria\nfunction unused(): int { return 1; }\n",
        ),
        &["--diagnostic-color", "never"],
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Error[E0755]"), "{stderr}");
    assert!(!stderr.contains("Related"), "{stderr}");
    assert!(!stderr.contains("<source unavailable>"), "{stderr}");
    assert!(!stderr.contains("unused"), "{stderr}");

    // A primary location in a compiler-owned source keeps its message without
    // a position, and a fix that would edit one is withheld.
    let stream = |start: usize| {
        doriac::source::Span::in_source(doriac::compiler_known_stream::SOURCE_ID, start, start + 4)
    };
    let mut sources = doriac::source_map::SourceMap::default();
    let main = doriac::source::SourceFile::with_id(
        doriac::source::SourceId(7),
        "acme/application:main.doria",
        "function main(): int { return 0; }\n",
    );
    sources.insert(doriac::source_map::SourceRecord {
        identity: doriac::names::SourceIdentity("acme/application:main.doria".to_string()),
        package: doriac::names::PackageIdentity::Named("acme/application".to_string()),
        display_path: main.path.clone(),
        canonical_path: None,
        content_fingerprint: String::new(),
        source: main,
    });
    let internal = Diagnostic::new("E0755", "the standard contract is malformed", stream(40))
        .with_related(stream(80), "the required contract is declared here")
        .with_related(
            doriac::source::Span::in_source(doriac::source::SourceId(7), 9, 13),
            "the implementation is declared here",
        )
        .with_structured_fix(
            "Edit The Standard Contract",
            FixApplicability::MachineApplicable,
            vec![FixEdit {
                source: DiagnosticSource::Current,
                span: stream(40),
                replacement: String::new(),
            }],
        );
    let prepared = prepare_diagnostics_with_source_map(&sources, std::slice::from_ref(&internal));
    let labels = &prepared[0].labels;
    assert_eq!(labels.len(), 2, "{labels:#?}");
    assert_eq!(labels[0].role, LabelRole::Primary);
    assert_eq!(labels[0].source, DiagnosticSource::Unavailable);
    assert_eq!(
        labels[1].source,
        DiagnosticSource::Path("acme/application:main.doria".to_string())
    );
    assert!(prepared[0].fixes.is_empty());
    let human = render_diagnostics_with_source_map(
        &sources,
        std::slice::from_ref(&internal),
        RenderOptions {
            color: ColorChoice::Never,
            ..RenderOptions::default()
        },
    );
    assert!(
        human.contains("Where\n<source unavailable>\nthe standard contract is malformed\n"),
        "{human}"
    );
    assert!(
        human.contains("acme/application:main.doria · line 1"),
        "{human}"
    );
    assert!(!human.contains("Suggested Fix"), "{human}");
    let json = render_diagnostics_with_source_map(
        &sources,
        &[internal],
        RenderOptions {
            format: DiagnosticFormat::Json,
            ..RenderOptions::default()
        },
    );
    let json: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        json["diagnostics"][0]["labels"][0]["source"],
        "<source unavailable>"
    );
    assert_eq!(
        json["diagnostics"][0]["labels"][0]["range"],
        serde_json::Value::Null
    );
}
