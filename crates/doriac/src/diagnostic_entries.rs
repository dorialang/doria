//! The catalogue's per-code documentation as the compiler presents it
//! (Decision 0145): `doriac explain`, and the export the website and the
//! language server read.

use doria_diagnostic_catalogue::{
    explanations::{self, DiagnosticEntry, EntryError},
    DIAGNOSTIC_CODES,
};
use serde_json::{json, Value};

/// The version of [`export`]'s document.
pub const EXPORT_SCHEMA_VERSION: u32 = 1;

/// Why `doriac explain <code>` has nothing to print.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupError {
    /// The text is not a Doria diagnostic code.
    UnknownCode(String),
    /// The code exists and no entry has been written for it.
    NoEntry(&'static str),
    /// The entry file is not in the required shape.
    Malformed(&'static str, EntryError),
}

impl std::fmt::Display for LookupError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownCode(code) => write!(
                formatter,
                "`{code}` is not a Doria diagnostic code\n\n\
                 A code is a letter and four digits, such as E0782. \
                 Run `doriac explain --all` to list the codes that have an explanation."
            ),
            Self::NoEntry(code) => write!(
                formatter,
                "no explanation has been written for {code} yet\n\n\
                 Run `doriac explain --all` to list the codes that have one."
            ),
            Self::Malformed(code, error) => write!(
                formatter,
                "the catalogue entry for {code} is malformed: {}",
                error.requirement()
            ),
        }
    }
}

/// The catalogued spelling of `code`, whatever case it was typed in.
fn catalogued(code: &str) -> Option<&'static str> {
    DIAGNOSTIC_CODES
        .iter()
        .copied()
        .find(|known| known.eq_ignore_ascii_case(code))
}

/// The entry for a code typed by a person.
pub fn lookup(code: &str) -> Result<DiagnosticEntry<'static>, LookupError> {
    let known = catalogued(code).ok_or_else(|| LookupError::UnknownCode(code.to_string()))?;
    match explanations::entry(known) {
        Some(Ok(entry)) => Ok(entry),
        Some(Err(error)) => Err(LookupError::Malformed(known, error)),
        None => Err(LookupError::NoEntry(known)),
    }
}

/// Every entry, in code order. A malformed entry is an error naming its code.
pub fn all() -> Result<Vec<DiagnosticEntry<'static>>, LookupError> {
    explanations::ENTRY_SOURCES
        .iter()
        .map(|(code, source)| {
            let known =
                catalogued(code).ok_or_else(|| LookupError::UnknownCode(code.to_string()))?;
            explanations::parse(known, source).map_err(|error| LookupError::Malformed(known, error))
        })
        .collect()
}

/// The catalogued codes that have no entry yet, in code order.
pub fn codes_without_entry() -> Vec<&'static str> {
    let mut codes: Vec<&'static str> = DIAGNOSTIC_CODES
        .iter()
        .copied()
        .filter(|code| explanations::entry(code).is_none())
        .collect();
    codes.sort_unstable();
    codes
}

fn entry_json(entry: &DiagnosticEntry<'_>) -> Value {
    json!({
        "code": entry.code,
        "title": entry.title,
        "titleIsFixed": entry.title_is_fixed,
        "explanation": entry.explanation,
        "fix": entry.fix,
        "example": entry.example,
        "url": crate::diagnostics::documentation_url(entry.code),
    })
}

/// One entry as a JSON document.
pub fn entry_document(entry: &DiagnosticEntry<'_>) -> Value {
    json!({
        "schemaVersion": EXPORT_SCHEMA_VERSION,
        "toolchainVersion": crate::TOOLCHAIN_VERSION,
        "compilerCommit": crate::BUILD_COMMIT,
        "entry": entry_json(entry),
    })
}

/// The whole catalogue as the document consumers pin: every entry, and the
/// codes that have none yet.
pub fn export() -> Result<Value, LookupError> {
    let entries: Vec<Value> = all()?.iter().map(entry_json).collect();
    Ok(json!({
        "schemaVersion": EXPORT_SCHEMA_VERSION,
        "toolchainVersion": crate::TOOLCHAIN_VERSION,
        "compilerCommit": crate::BUILD_COMMIT,
        "entries": entries,
        "codesWithoutEntry": codes_without_entry(),
    }))
}

/// One entry as a person reads it in a terminal.
pub fn render(entry: &DiagnosticEntry<'_>) -> String {
    format!(
        "{}: {}\n\nWhy\n{}\n\nFix\n{}\n\nExample\n{}\n\n{}\n",
        entry.code,
        entry.title,
        entry.explanation,
        entry.fix,
        entry.example.trim_end(),
        crate::diagnostics::documentation_url(entry.code),
    )
}

/// The codes that have an entry, one per line with the title.
pub fn render_index(entries: &[DiagnosticEntry<'_>]) -> String {
    let mut index = String::new();
    for entry in entries {
        index.push_str(&format!("{}  {}\n", entry.code, entry.title));
    }
    index
}
