//! The catalogue's per-code documentation (Decision 0145): every entry is in
//! the required shape, and its example is a program that raises its code.

use doria_diagnostic_catalogue::explanations::ENTRY_SOURCES;
use doriac::diagnostic_entries::{self, LookupError};

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

#[test]
fn every_entry_is_well_formed_and_its_example_raises_its_code() {
    let entries = diagnostic_entries::all().expect("every entry file should parse");
    assert_eq!(entries.len(), ENTRY_SOURCES.len());
    assert!(!entries.is_empty(), "the catalogue should have entries");

    for entry in &entries {
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

        let raised = match doriac::check_source(format!("{code}.doria"), entry.example) {
            Ok(_) => panic!("{code}: its example compiles without any diagnostic"),
            Err(diagnostics) => diagnostics,
        };
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
    let entries = document["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), ENTRY_SOURCES.len());
    for entry in entries {
        for field in ["code", "title", "explanation", "fix", "example", "url"] {
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
                "https://dorialang.org/docs/diagnostics/{}",
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
