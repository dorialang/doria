//! Per-code documentation entries (Decision 0145).
//!
//! An entry documents a diagnostic code as a whole: what it means, how to fix
//! it, and a program that raises it. The compiler keeps its own specific
//! message at each place it raises the code. Entries are Markdown files under
//! `entries/`, one per code, in this fixed shape:
//!
//! ````text
//! # Title In Title Case
//!
//! Title: fixed
//! Family: types
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
//! ````
//!
//! A code that no program raises on demand, such as an internal or backend
//! failure, has no fenced program. Its `## Example` section is a sentence
//! that begins `No program raises this code on demand` and says why. A code
//! reported only for a project of several files begins `No single source file
//! raises this code`, and a code only ever reported after another error
//! begins `No program shows this code on its own`.
//!
//! A code raised only in a project's test sources carries the line
//! `Example source: development` under its `Title:` line. Its example is then
//! checked as development source.
//!
//! `Family:` names one of the families in `entries/FAMILIES.txt`.
//!
//! A code that reports a feature the compiler does not have yet carries
//! `Availability: planned` when the feature is part of Doria for the first
//! full release, and `Availability: later` when Doria reserves it for a
//! release after that. Its entry says which, and never reads as a rejection.
//!
//! `Title: fixed` says the compiler raises the code under exactly this
//! title. `Title: varies` says the compiler's title depends on the program,
//! and the entry's title is the catalogue's name for the code.

/// The families of diagnostic codes as written in `entries/FAMILIES.txt`.
const FAMILIES_SOURCE: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/entries/FAMILIES.txt"));

/// One family of diagnostic codes: how the catalogue groups them for a
/// reader. The order of [`families`] is the catalogue's order, most common
/// first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticFamily {
    pub id: &'static str,
    pub title: &'static str,
    pub summary: &'static str,
}

/// The families, in catalogue order. A line that is not `id | Title |
/// summary` is skipped; the compiler's test requires every line to parse.
pub fn families() -> impl Iterator<Item = DiagnosticFamily> {
    family_lines().filter_map(|line| {
        let mut parts = line.splitn(3, '|').map(str::trim);
        let (id, title, summary) = (parts.next()?, parts.next()?, parts.next()?);
        (!id.is_empty() && !title.is_empty() && !summary.is_empty()).then_some(DiagnosticFamily {
            id,
            title,
            summary,
        })
    })
}

/// The lines of `entries/FAMILIES.txt` that declare a family.
pub fn family_lines() -> impl Iterator<Item = &'static str> {
    FAMILIES_SOURCE
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
}

/// The embedded entry files, sorted by code.
pub const ENTRY_SOURCES: &[(&str, &str)] = include!(concat!(env!("OUT_DIR"), "/entry_sources.rs"));

/// One parsed documentation entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticEntry<'a> {
    pub code: &'a str,
    pub title: &'a str,
    /// The compiler raises the code under exactly `title`.
    pub title_is_fixed: bool,
    /// The id of the entry's family, one of [`families`].
    pub family: &'a str,
    /// Whether the code reports a rule of the language or a feature this
    /// compiler does not have yet.
    pub availability: Availability,
    pub explanation: &'a str,
    pub fix: &'a str,
    /// A Doria program that raises the code. `None` for a code that no
    /// program raises on demand, such as an internal or backend failure.
    pub example: Option<&'a str>,
    /// Why there is no example program, when there is none.
    pub example_note: Option<&'a str>,
    /// The example is development source: a project's tests. The code is
    /// raised only there, so the example is checked in that scope.
    pub example_is_development_source: bool,
}

/// What a code reports about the language.
///
/// A feature that is part of Doria and not yet in the compiler is still part
/// of the language. Its code says so, and never reads as a rejection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    /// A rule of the language.
    Current,
    /// The program uses a feature that is part of Doria for the first full
    /// release and that this compiler does not implement yet.
    Planned,
    /// The program uses a feature that Doria reserves for a release after
    /// the first full one.
    Later,
}

impl Availability {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Planned => "planned",
            Self::Later => "later",
        }
    }
}

/// Why an entry file is not in the required shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryError {
    MissingTitle,
    MissingTitleKind,
    MissingFamily,
    MissingExplanation,
    MissingFix,
    MissingExample,
    SectionsOutOfOrder,
    UnknownHeaderLine,
}

impl EntryError {
    /// What the file must contain, in words an author can act on.
    pub const fn requirement(self) -> &'static str {
        match self {
            Self::MissingTitle => "the first line must be `# ` followed by the title",
            Self::MissingTitleKind => {
                "a `Title: fixed` or `Title: varies` line must follow the title"
            }
            Self::MissingFamily => {
                "a `Family: <id>` line must follow the `Title:` line, naming a family from \
                 entries/FAMILIES.txt"
            }
            Self::MissingExplanation => "a `## Explanation` section with text is required",
            Self::MissingFix => "a `## Fix` section with text is required",
            Self::MissingExample => {
                "a `## Example` section is required, holding one ```doria fenced program, or a \
                 sentence that begins `No program raises this code on demand`, `No single \
                 source file raises this code` or `No program shows this code on its own` \
                 and says why"
            }
            Self::UnknownHeaderLine => {
                "between the title and `## Explanation` only a `Title:` line, a `Family:` line, \
                 an optional `Availability: planned` or `Availability: later` line and an \
                 optional `Example source: development` line are allowed, in that order"
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
/// The header line of an entry whose example is a project's test source.
pub const DEVELOPMENT_SOURCE: &str = "Example source: development";
/// How the `## Example` section may begin when it holds no program: the code
/// is not raised by any program on demand, is reported only for a project
/// of several files, or is only ever reported after another error.
pub const NO_EXAMPLE_OPENINGS: [&str; 3] = [
    "No program raises this code on demand",
    "No single source file raises this code",
    "No program shows this code on its own",
];

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
    let mut header_lines = header
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let title_is_fixed = match header_lines.next() {
        Some("Title: fixed") => true,
        Some("Title: varies") => false,
        _ => return Err(EntryError::MissingTitleKind),
    };
    let family = header_lines
        .next()
        .and_then(|line| line.strip_prefix("Family: "))
        .map(str::trim)
        .filter(|family| !family.is_empty())
        .ok_or(EntryError::MissingFamily)?;
    let mut next_line = header_lines.next();
    let availability = match next_line {
        Some("Availability: planned") => Availability::Planned,
        Some("Availability: later") => Availability::Later,
        _ => Availability::Current,
    };
    if availability != Availability::Current {
        next_line = header_lines.next();
    }
    let example_is_development_source = match next_line {
        None => false,
        Some(DEVELOPMENT_SOURCE) => true,
        Some(_) => return Err(EntryError::UnknownHeaderLine),
    };
    if header_lines.next().is_some() {
        return Err(EntryError::UnknownHeaderLine);
    }

    let explanation = source[explanation_at + EXPLANATION.len()..fix_at].trim();
    if explanation.is_empty() {
        return Err(EntryError::MissingExplanation);
    }
    let fix = source[fix_at + FIX.len()..example_at].trim();
    if fix.is_empty() {
        return Err(EntryError::MissingFix);
    }
    let example_section = &source[example_at + EXAMPLE.len()..];
    let (example, example_note) = match example_section.find(FENCE_OPEN) {
        Some(program_at) => {
            let program = &example_section[program_at + FENCE_OPEN.len()..];
            let program_end = program
                .find(FENCE_CLOSE)
                .ok_or(EntryError::MissingExample)?;
            let example = &program[..program_end];
            if example.trim().is_empty() {
                return Err(EntryError::MissingExample);
            }
            (Some(example), None)
        }
        None => {
            let note = example_section.trim();
            if !NO_EXAMPLE_OPENINGS
                .iter()
                .any(|opening| note.starts_with(opening))
            {
                return Err(EntryError::MissingExample);
            }
            (None, Some(note))
        }
    };

    Ok(DiagnosticEntry {
        code,
        title,
        title_is_fixed,
        family,
        availability,
        explanation,
        fix,
        example,
        example_note,
        example_is_development_source,
    })
}

/// The entry for `code`, parsed. `None` when no entry has been written.
pub fn entry(code: &str) -> Option<Result<DiagnosticEntry<'static>, EntryError>> {
    ENTRY_SOURCES
        .iter()
        .find(|(entry_code, _)| *entry_code == code)
        .map(|(entry_code, source)| parse(entry_code, source))
}
