use doriac::diagnostics::Diagnostic;

fn lifecycle_diagnostics(source: &str) -> Vec<Diagnostic> {
    doriac::check_source("lifecycle.doria", source)
        .expect_err("fixture must produce lifecycle diagnostics")
}

fn diagnostic_snapshot(source: &str, code: &str) -> String {
    let diagnostics = lifecycle_diagnostics(source);
    let diagnostic = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == code)
        .unwrap_or_else(|| panic!("expected {code}, got {diagnostics:?}"));

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

#[test]
fn lifecycle_modifier_matrix_covers_every_current_modifier_combination() {
    let modifier_cases = [
        ("", &[][..]),
        ("internal ", &[][..]),
        ("static ", &["E0465"][..]),
        ("internal static ", &["E0465"][..]),
        ("writable ", &["E0466"][..]),
        ("internal writable ", &["E0466"][..]),
        ("static writable ", &["E0465", "E0466"][..]),
        ("internal static writable ", &["E0465", "E0466"][..]),
    ];
    let mut matrix_entries = 0;

    for name in ["__construct", "__destruct"] {
        for (modifiers, expected_codes) in modifier_cases {
            matrix_entries += 1;
            let source =
                format!("class Subject {{ {modifiers}function {name}(): void {{ return; }} }}");
            let result = doriac::check_source("matrix.doria", &source);

            if expected_codes.is_empty() {
                result.unwrap_or_else(|diagnostics| {
                    panic!("expected `{modifiers}{name}` to be legal, got {diagnostics:?}")
                });
                continue;
            }

            let diagnostics = result.expect_err("invalid lifecycle shape must be rejected");
            let actual_codes = diagnostics
                .iter()
                .filter(|diagnostic| matches!(diagnostic.code, "E0465" | "E0466"))
                .map(|diagnostic| diagnostic.code)
                .collect::<Vec<_>>();
            assert_eq!(actual_codes, expected_codes, "source: {source}");
        }
    }

    assert_eq!(matrix_entries, 16);
}

#[test]
fn writable_lifecycle_fix_removes_only_the_modifier() {
    for (name, help_fragment) in [
        ("__construct", "construction grants `__construct`"),
        ("__destruct", "destruction invokes `__destruct`"),
    ] {
        let source = format!("class Subject {{ writable function {name}() {{}} }}");
        let diagnostics = lifecycle_diagnostics(&source);
        let diagnostic = diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code == "E0466")
            .expect("writable lifecycle diagnostic");
        let fix = diagnostic.fix.as_ref().expect("machine-applicable fix");

        assert_eq!(fix.span.start, source.find("writable").unwrap());
        assert_eq!(fix.span.end, fix.span.start + "writable".len());
        assert_eq!(fix.replacement, "");
        assert!(diagnostic.help.as_deref().unwrap().contains(help_fragment));
    }
}

#[test]
fn lifecycle_declaration_diagnostics_match_snapshots() {
    for (source, code, expected) in [
        (
            "class Person { static function __construct() {} }",
            "E0465",
            include_str!("fixtures/diagnostics/lifecycle_static_construct.txt"),
        ),
        (
            "class Person { writable function __construct() {} }",
            "E0466",
            include_str!("fixtures/diagnostics/lifecycle_writable_construct.txt"),
        ),
        (
            "class Person { static function __destruct() {} }",
            "E0465",
            include_str!("fixtures/diagnostics/lifecycle_static_destruct.txt"),
        ),
        (
            "class Person { writable function __destruct() {} }",
            "E0466",
            include_str!("fixtures/diagnostics/lifecycle_writable_destruct.txt"),
        ),
        (
            "class Person { function __destruct(string $reason) {} }",
            "E0411",
            include_str!("fixtures/diagnostics/lifecycle_destruct_parameters.txt"),
        ),
    ] {
        assert_snapshot(diagnostic_snapshot(source, code), expected);
    }
}

#[test]
fn direct_lifecycle_call_diagnostics_match_snapshots() {
    for (source, expected) in [
        (
            "class Person { function __construct() {} } let writable $person = new Person(); $person->__construct();",
            include_str!("fixtures/diagnostics/lifecycle_object_construct_call.txt"),
        ),
        (
            "class Person { function __construct() {} function reset(): void { $this->__construct(); } }",
            include_str!("fixtures/diagnostics/lifecycle_this_construct_call.txt"),
        ),
        (
            "class Person { function __destruct() {} } let writable $person = new Person(); $person->__destruct();",
            include_str!("fixtures/diagnostics/lifecycle_object_destruct_call.txt"),
        ),
        (
            "class Person { function __construct() {} } Person::__construct();",
            include_str!("fixtures/diagnostics/lifecycle_static_construct_call.txt"),
        ),
    ] {
        assert_snapshot(diagnostic_snapshot(source, "E0414"), expected);
    }
}
