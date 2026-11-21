//! The catalogue's per-code documentation (Decision 0145): every entry is in
//! the required shape, and its example is a program that raises its code.

#[path = "common/execution_directory.rs"]
mod execution_directory;

#[path = "common/native_execution_runner.rs"]
mod native_execution;

use doria_diagnostic_catalogue::explanations::ENTRY_SOURCES;
use doriac::diagnostic_entries::{self, LookupError};

/// Language codes that no single file can show as its first error. Each is
/// here for a stated reason; any other language, lexical or syntax code must
/// have an example program.
const CHECKED_CODES_WITHOUT_A_PROGRAM: &[&str] = &[
    // Only ever reported after another error at the same place.
    "E0454", "E0473", "E0633", "E0634", "E0696",
    // Reported when a program is compiled, not when it is checked.
    "E0505", "E0784", "E0804", "E0805", "E0817",
    // Raised only through compiler interfaces that no command reaches.
    "E0673", "E0674", "E0699",
    // Reported for a project's build plan or several of its files.
    "E0676", "E0677", "E0678", "E0679", "E0680", "E0682", "E0683", "E0684", "E0685", "E0816",
    // Reported only for a test function the compiler itself generates.
    "E0711",
];

/// What the compiler reports for `source` checked as a project's test
/// source: one package whose only file is development source.
fn development_source_diagnostics(source: &str) -> Vec<doriac::diagnostics::Diagnostic> {
    use doriac::build_plan::{
        BuildPlan, BuildPlanDocument, CompilerOptions, CompilerTarget, NamespaceMapping, Package,
        SelectedTarget, Source, SourceOrigin, SourceScope, TargetKind,
    };
    const PACKAGE: &str = "example/entry";
    let document = BuildPlanDocument {
        path: "build-plan.json".to_string(),
        directory: std::env::current_dir().expect("current directory"),
        text: String::new(),
        plan: BuildPlan {
            schema_version: 1,
            edition: "2026".to_string(),
            root_package: PACKAGE.to_string(),
            selected_target: SelectedTarget {
                package: PACKAGE.to_string(),
                name: "tests".to_string(),
                kind: TargetKind::Library,
                entry_source: None,
                active_scopes: vec![
                    SourceScope::Main,
                    SourceScope::Development,
                    SourceScope::Generated,
                ],
                windows_subsystem: None,
            },
            packages: vec![Package {
                identity: PACKAGE.to_string(),
                root: ".".to_string(),
                namespace_mappings: vec![NamespaceMapping {
                    prefix: String::new(),
                    path: String::new(),
                    scope: SourceScope::Development,
                    generated_for: None,
                }],
                sources: vec![Source {
                    identity: format!("{PACKAGE}:tests.doria"),
                    path: "tests.doria".to_string(),
                    scope: SourceScope::Development,
                    origin: SourceOrigin::Explicit,
                    generated_for: None,
                }],
                dependencies: Vec::new(),
                native_libraries: Vec::new(),
                unsafe_policy: None,
            }],
            compiler: CompilerOptions {
                target: CompilerTarget::Debug,
                native_profile: None,
                target_triple: None,
            },
        },
    };
    let mut provider = doriac::source_provider::InMemorySourceProvider::new();
    provider.insert(PACKAGE, "tests.doria", source);
    let graph = doriac::compilation_graph::load_compilation_graph(&document, &provider)
        .expect("an entry's development example should load as a one-file package");
    doriac::analyze_compilation_graph_for_ide(&graph).diagnostics
}

/// A diagnostic title or entry title in Title Case, as the diagnostic style
/// requires: no word begins with a lowercase letter unless it is quoted code
/// or a short connecting word.
fn begins_with_a_capital(title: &str) -> bool {
    title
        .chars()
        .next()
        .is_some_and(|first| !first.is_lowercase())
}

/// Entry text is embedded as checked out. A carriage return would make the
/// export differ between platforms and breaks the entry parser, so the files
/// are LF in every checkout (`.gitattributes`).
#[test]
fn entry_files_use_lf_line_endings_on_every_platform() {
    for (code, source) in ENTRY_SOURCES {
        assert!(
            !source.contains('\r'),
            "{code}: entries/{code}.md has CRLF line endings; .gitattributes must mark \
             crates/doria-diagnostic-catalogue/entries/** as `text eol=lf`"
        );
    }
}

/// Checks one entry. Panics with what is wrong and how to correct it.
fn check_entry(entry: &doria_diagnostic_catalogue::explanations::DiagnosticEntry<'static>) {
    let code = entry.code;
    assert!(
        begins_with_a_capital(entry.title),
        "{code}: title `{}`",
        entry.title
    );
    assert!(
        !entry.title.ends_with('.'),
        "{code}: a title has no closing period"
    );

    // Language, lexical and syntax codes are raised by checking a
    // program. Runtime codes are raised by running one. The rest
    // (backend, internal and MIR failures) are not raised by any
    // program on demand.
    let runtime = doria_diagnostic_catalogue::RUNTIME_CATALOGUE
        .iter()
        .any(|runtime| runtime.code == code);
    let checked =
        !runtime && (code.starts_with('E') || code.starts_with('L') || code.starts_with("P0"));
    let Some(example) = entry.example else {
        assert!(
            !checked || CHECKED_CODES_WITHOUT_A_PROGRAM.contains(&code),
            "{code}: a language, lexical or syntax code needs an example program; only the \
             codes listed in CHECKED_CODES_WITHOUT_A_PROGRAM may go without one"
        );
        assert!(
            entry.example_note.is_some_and(|note| note.len() > 60),
            "{code}: an entry without a program must say why no program raises it"
        );
        return;
    };
    assert!(
        checked || runtime,
        "{code}: no program raises this kind of code on demand, so its entry cannot \
         carry an example program"
    );

    if runtime {
        // The example must compile cleanly and end with this outcome.
        let program = doriac::lower_source_to_mir(format!("{code}.doria"), example.to_string())
            .unwrap_or_else(|diagnostics| {
                panic!("{code}: its example does not compile: {diagnostics:?}")
            });
        // The interpreter runs unsafe code through the native companion
        // library, which this compiler does not build yet; such an example
        // runs as a native program, whose report carries the same title.
        if !doriac::unsafe_code::companion_operations(&program).is_empty() {
            let run =
                native_execution::execute_native(&program, doriac::backend::NativeProfile::Fast);
            let stderr = String::from_utf8_lossy(&run.stderr);
            assert_eq!(run.status.code(), Some(101), "{code}: {stderr}");
            assert!(
                stderr.contains(code),
                "{code}: its example ends with {stderr}"
            );
            assert!(
                stderr.contains(entry.title) == entry.title_is_fixed,
                "{code}: the entry's `Title:` line does not match the report: {stderr}"
            );
            return;
        }
        let output = doriac::mir_interpreter::interpret(&program)
            .unwrap_or_else(|error| panic!("{code}: its example could not be run: {error}"));
        let raised = output.runtime_diagnostic.unwrap_or_else(|| {
            panic!("{code}: its example runs to the end without a runtime diagnostic")
        });
        assert_eq!(
            raised.code, code,
            "{code}: its example ends with {}",
            raised.code
        );
        if entry.title_is_fixed {
            assert_eq!(
                raised.title, entry.title,
                "{code}: the entry says `Title: fixed`, so its title must be the one the \
                 program ends with"
            );
        } else {
            assert_ne!(
                raised.title, entry.title,
                "{code}: the program ends under exactly the entry's title; if it always \
                 does, the entry should say `Title: fixed`"
            );
        }
        return;
    }

    let raised = if entry.example_is_development_source {
        development_source_diagnostics(example)
    } else {
        // A warning does not stop checking, so a warning's example checks
        // and reports it beside the program.
        match doriac::check_source_with_warnings(format!("{code}.doria"), example) {
            Ok((_, warnings)) => warnings,
            Err(diagnostics) => diagnostics,
        }
    };
    assert!(
        !raised.is_empty(),
        "{code}: its example compiles without any diagnostic"
    );
    if entry.example_is_development_source {
        // The scope is part of the claim: the same file as ordinary
        // source must not raise the code, or the header line is wrong.
        let as_main = doriac::check_source(format!("{code}.doria"), example)
            .err()
            .unwrap_or_default();
        assert!(
            as_main.iter().all(|diagnostic| diagnostic.code != code),
            "{code}: its example raises {code} as ordinary source, so the entry must not \
             say `Example source: development`"
        );
    }
    let Some(diagnostic) = raised.iter().find(|diagnostic| diagnostic.code == code) else {
        let seen: Vec<_> = raised
            .iter()
            .map(|diagnostic| format!("{} {}", diagnostic.code, diagnostic.title))
            .collect();
        panic!("{code}: its example does not raise {code}; it raises {seen:?}");
    };
    // The first diagnostic a reader sees should be the one the entry is
    // about, so an example cannot lean on an earlier, different error.
    assert_eq!(
        raised[0].code, code,
        "{code}: its example raises {} first; make {code} the first error",
        raised[0].code
    );

    if entry.title_is_fixed {
        assert_eq!(
            diagnostic.title, entry.title,
            "{code}: the entry says `Title: fixed`, so its title must be the one the \
             compiler raises"
        );
    } else {
        assert_ne!(
            diagnostic.title, entry.title,
            "{code}: the compiler raises exactly the entry's title here; if it always \
             does, the entry should say `Title: fixed`"
        );
    }
}

#[test]
fn every_entry_is_well_formed_and_its_example_raises_its_code() {
    let entries = diagnostic_entries::all().expect("every entry file should parse");
    assert_eq!(entries.len(), ENTRY_SOURCES.len());
    assert!(!entries.is_empty(), "the catalogue should have entries");

    // Every entry is checked, so one run reports every entry that fails.
    let mut failures = Vec::new();
    for entry in &entries {
        if let Err(panic) = std::panic::catch_unwind(|| check_entry(entry)) {
            let message = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(ToString::to_string))
                .unwrap_or_else(|| format!("{}: its check failed", entry.code));
            failures.push(message);
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} entries fail:\n\n{}",
        failures.len(),
        entries.len(),
        failures.join("\n\n")
    );
}

#[test]
fn every_catalogued_code_has_an_entry_or_is_listed_as_waiting() {
    let waiting: Vec<&str> = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../doria-diagnostic-catalogue/entries/WITHOUT_ENTRY.txt"
    ))
    .lines()
    .map(str::trim)
    .filter(|line| !line.is_empty() && !line.starts_with('#'))
    .collect();
    let without_entry = diagnostic_entries::codes_without_entry();

    // A new code arrives with its entry: it may not join the waiting list.
    // A code that gains an entry leaves the list in the same change.
    assert_eq!(
        without_entry, waiting,
        "entries/WITHOUT_ENTRY.txt must list exactly the catalogued codes that have no entry \
         file, in code order"
    );
}

#[test]
fn every_entry_names_a_family_the_catalogue_declares() {
    use doria_diagnostic_catalogue::explanations::{families, family_lines};

    let declared: Vec<_> = families().collect();
    assert_eq!(
        declared.len(),
        family_lines().count(),
        "every line of entries/FAMILIES.txt must be `id | Title | summary`"
    );
    let mut ids: Vec<&str> = declared.iter().map(|family| family.id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), declared.len(), "family ids must be unique");
    for family in &declared {
        assert!(
            family
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '-'),
            "family id `{}` must be a lowercase slug",
            family.id
        );
        assert!(
            begins_with_a_capital(family.title),
            "family title `{}`",
            family.title
        );
        assert!(
            family.summary.ends_with('.'),
            "the summary of `{}` is one sentence",
            family.id
        );
    }

    let entries = diagnostic_entries::all().expect("every entry file should parse");
    for entry in &entries {
        assert!(
            declared.iter().any(|family| family.id == entry.family),
            "{}: `Family: {}` is not declared in entries/FAMILIES.txt",
            entry.code,
            entry.family
        );
    }
    for family in &declared {
        assert!(
            entries.iter().any(|entry| entry.family == family.id),
            "family `{}` has no entry; remove it or assign its entries",
            family.id
        );
    }

    let document = diagnostic_entries::export().expect("the catalogue should export");
    let exported = document["families"].as_array().expect("families");
    assert_eq!(exported.len(), declared.len());
    let listed: usize = exported
        .iter()
        .map(|family| family["codes"].as_array().expect("codes").len())
        .sum();
    assert_eq!(
        listed,
        entries.len(),
        "every entry appears under exactly one family in the export"
    );
}

#[test]
fn retired_codes_are_reserved_and_have_no_entry() {
    use doria_diagnostic_catalogue::{DIAGNOSTIC_CODES, RETIRED_DIAGNOSTIC_CODES};

    assert!(!RETIRED_DIAGNOSTIC_CODES.is_empty());
    for code in RETIRED_DIAGNOSTIC_CODES {
        assert!(
            !DIAGNOSTIC_CODES.contains(code),
            "{code} is retired, so it cannot also be a code in use; a new diagnostic needs a \
             new number"
        );
        assert!(
            ENTRY_SOURCES.iter().all(|(entry, _)| entry != code),
            "{code} is retired and has no entry"
        );
        let retired = diagnostic_entries::lookup(code).unwrap_err();
        assert_eq!(retired, LookupError::Retired(code));
        assert!(retired.to_string().contains("retired"));
    }
    let mut sorted = RETIRED_DIAGNOSTIC_CODES.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted, RETIRED_DIAGNOSTIC_CODES,
        "retired codes are listed once, in order"
    );
}

#[test]
fn an_entry_for_an_unavailable_feature_says_it_is_part_of_doria() {
    use doria_diagnostic_catalogue::explanations::Availability;

    let entries = diagnostic_entries::all().expect("every entry file should parse");
    for entry in &entries {
        let text = format!("{}\n{}", entry.explanation, entry.fix);
        match entry.availability {
            Availability::Current => {}
            // The feature is in the language; only this compiler lacks it.
            Availability::Planned => assert!(
                text.contains("part of Doria") && text.contains("does not implement it yet"),
                "{}: a planned feature's entry must say the feature is part of Doria and \
                 that this compiler does not implement it yet",
                entry.code
            ),
            Availability::Later => {
                assert!(
                    text.contains("reserves") && text.contains("later"),
                    "{}: an entry for a reserved feature must say Doria reserves it for a later \
                     release",
                    entry.code
                );
                // The reservation is the rule of the release the entry documents.
                for dated in ["today", "as before"] {
                    assert!(
                        !text.contains(dated),
                        "{}: an entry for a reserved feature states the release's rule, so it \
                         must not date it (`{dated}`)",
                        entry.code
                    );
                }
            }
        }
        if entry.availability != Availability::Current {
            for rejected in ["is rejected", "not supported", "unsupported", "not allowed"] {
                assert!(
                    !text.contains(rejected),
                    "{}: an entry for a feature that is not available yet must not read as a \
                     rejection (`{rejected}`)",
                    entry.code
                );
            }
        }
    }
}

#[test]
fn lookups_name_what_is_wrong_and_what_to_do() {
    assert_eq!(
        diagnostic_entries::lookup("e0782").map(|entry| entry.code),
        Ok("E0782"),
        "a code is found whatever case it is typed in"
    );

    let unknown = diagnostic_entries::lookup("Z9999").unwrap_err();
    assert_eq!(unknown, LookupError::UnknownCode("Z9999".to_string()));
    assert!(unknown.to_string().contains("doriac explain --all"));

    let waiting = diagnostic_entries::codes_without_entry();
    if let Some(code) = waiting.first() {
        let missing = diagnostic_entries::lookup(code).unwrap_err();
        assert_eq!(missing, LookupError::NoEntry(code));
        assert!(missing.to_string().contains("doriac explain --all"));
    }
}

#[test]
fn the_export_lists_every_entry_and_every_waiting_code() {
    let document = diagnostic_entries::export().expect("the catalogue should export");
    assert_eq!(
        document["schemaVersion"],
        diagnostic_entries::EXPORT_SCHEMA_VERSION
    );
    let release = document["documentationRelease"]
        .as_str()
        .expect("documentationRelease");
    // The documentation site names a release `v` and the unpadded year,
    // quarter and number; the toolchain version pads the quarter.
    assert_eq!(
        release,
        format!(
            "v{}",
            env!("CARGO_PKG_VERSION")
                .split('-')
                .next()
                .expect("a package version")
        )
    );
    let entries = document["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), ENTRY_SOURCES.len());
    for entry in entries {
        assert!(
            entry["example"].is_string() != entry["exampleNote"].is_string(),
            "an entry has a program or a note saying why it has none: {entry}"
        );
        for field in ["code", "title", "explanation", "fix", "url"] {
            assert!(
                entry[field].as_str().is_some_and(|text| !text.is_empty()),
                "{field} of {entry}"
            );
        }
        assert!(entry["titleIsFixed"].is_boolean());
        let code = entry["code"].as_str().unwrap();
        assert_eq!(
            entry["url"],
            format!(
                "https://docs.dorialang.org/{release}/diagnostics/{}",
                code.to_ascii_lowercase()
            )
        );
    }
    let waiting = document["codesWithoutEntry"]
        .as_array()
        .expect("codesWithoutEntry");
    assert_eq!(
        entries.len() + waiting.len(),
        doria_diagnostic_catalogue::DIAGNOSTIC_CODES.len()
    );
}
