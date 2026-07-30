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
        assert!(human.contains("Why:"));
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
