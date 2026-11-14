//! Per-code documentation entries (Decision 0145).
//!
//! An entry documents a diagnostic code as a whole: what it means, how to fix
//! it, and a program that raises it. The compiler keeps its own specific
//! message at each place it raises the code. Entries are Markdown files under
//! `entries/`, one per code, in this fixed shape:
//!
//! ```text
//! # Title In Title Case
//!
//! Title: fixed
//!
//! ## Explanation
//!
//! One or more paragraphs.
//!
//! ## Fix
//!
//! One or more paragraphs.
//!
//! ## Example
//!
//! ```doria
//! a program that raises the code
//! ```
//! ```
//!
//! `Title: fixed` says the compiler raises the code under exactly this
//! title. `Title: varies` says the compiler's title depends on the program,
//! and the entry's title is the catalogue's name for the code.

/// The embedded entry files, sorted by code.
pub const ENTRY_SOURCES: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/entry_sources.rs"));

/// One parsed documentation entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticEntry<'a> {
    pub code: &'a str,
    pub title: &'a str,
    /// The compiler raises the code under exactly `title`.
    pub title_is_fixed: bool,
    pub explanation: &'a str,
    pub fix: &'a str,
    /// A Doria program that raises the code.
    pub example: &'a str,
}

/// Why an entry file is not in the required shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryError {
    MissingTitle,
    MissingTitleKind,
    MissingExplanation,
    MissingFix,
    MissingExample,
    SectionsOutOfOrder,
}

impl EntryError {
    /// What the file must contain, in words an author can act on.
    pub const fn requirement(self) -> &'static str {
        match self {
            Self::MissingTitle => "the first line must be `# ` followed by the title",
            Self::MissingTitleKind => {
                "a `Title: fixed` or `Title: varies` line must follow the title"
            }
            Self::MissingExplanation => "a `## Explanation` section with text is required",
            Self::MissingFix => "a `## Fix` section with text is required",
            Self::MissingExample => {
                "a `## Example` section with one ```doria fenced program is required"
            }
            Self::SectionsOutOfOrder => {
                "the sections must be `## Explanation`, `## Fix`, `## Example`, in that order"
            }
        }
    }
}

const EXPLANATION: &str = "\n## Explanation\n";
const FIX: &str = "\n## Fix\n";
const EXAMPLE: &str = "\n## Example\n";
const FENCE_OPEN: &str = "```doria\n";
const FENCE_CLOSE: &str = "\n```";

/// Parses one entry file.
pub fn parse<'a>(code: &'a str, source: &'a str) -> Result<DiagnosticEntry<'a>, EntryError> {
    let title_line = source.lines().next().unwrap_or("");
    let title = title_line
        .strip_prefix("# ")
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .ok_or(EntryError::MissingTitle)?;

    let explanation_at = source
        .find(EXPLANATION)
        .ok_or(EntryError::MissingExplanation)?;
    let fix_at = source.find(FIX).ok_or(EntryError::MissingFix)?;
    let example_at = source.find(EXAMPLE).ok_or(EntryError::MissingExample)?;
    if !(explanation_at < fix_at && fix_at < example_at) {
        return Err(EntryError::SectionsOutOfOrder);
    }

    let header = &source[title_line.len()..explanation_at];
    let title_is_fixed = match header.lines().map(str::trim).find(|line| !line.is_empty()) {
        Some("Title: fixed") => true,
        Some("Title: varies") => false,
        _ => return Err(EntryError::MissingTitleKind),
    };

    let explanation = source[explanation_at + EXPLANATION.len()..fix_at].trim();
    if explanation.is_empty() {
        return Err(EntryError::MissingExplanation);
    }
    let fix = source[fix_at + FIX.len()..example_at].trim();
    if fix.is_empty() {
        return Err(EntryError::MissingFix);
    }
    let example_section = &source[example_at + EXAMPLE.len()..];
    let program_at = example_section
        .find(FENCE_OPEN)
        .ok_or(EntryError::MissingExample)?;
    let program = &example_section[program_at + FENCE_OPEN.len()..];
    let program_end = program
        .find(FENCE_CLOSE)
        .ok_or(EntryError::MissingExample)?;
    let example = &program[..program_end];
    if example.trim().is_empty() {
        return Err(EntryError::MissingExample);
    }

    Ok(DiagnosticEntry {
        code,
        title,
        title_is_fixed,
        explanation,
        fix,
        example,
    })
}

/// The entry for `code`, parsed. `None` when no entry has been written.
pub fn entry(code: &str) -> Option<Result<DiagnosticEntry<'static>, EntryError>> {
    ENTRY_SOURCES
        .iter()
        .find(|(entry_code, _)| *entry_code == code)
        .map(|(entry_code, source)| parse(entry_code, source))
}
