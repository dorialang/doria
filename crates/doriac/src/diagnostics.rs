use std::collections::HashSet;
use std::io::IsTerminal;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::source::{SourceFile, Span};

pub const DIAGNOSTIC_SCHEMA_VERSION: u32 = 1;

pub type DiagnosticResult<T> = Result<T, Vec<Diagnostic>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Note,
}

impl DiagnosticSeverity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Note => "note",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Error => "Error",
            Self::Warning => "Warning",
            Self::Note => "Note",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticKind {
    Language,
    UnsupportedDevelopmentSurface,
    Backend,
    ExternalTool,
    InternalCompiler,
    RuntimePanic,
}

impl DiagnosticKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Language => "language",
            Self::UnsupportedDevelopmentSurface => "unsupportedDevelopmentSurface",
            Self::Backend => "backend",
            Self::ExternalTool => "externalTool",
            Self::InternalCompiler => "internalCompiler",
            Self::RuntimePanic => "runtimePanic",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelRole {
    Primary,
    Secondary,
}

impl LabelRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Secondary => "secondary",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DiagnosticSource {
    Current,
    Path(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticLabel {
    pub source: DiagnosticSource,
    pub span: Span,
    pub role: LabelRole,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixApplicability {
    MachineApplicable,
    RequiresReview,
    Informational,
}

impl FixApplicability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MachineApplicable => "machineApplicable",
            Self::RequiresReview => "requiresReview",
            Self::Informational => "informational",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::MachineApplicable => "Machine Applicable",
            Self::RequiresReview => "Requires Review",
            Self::Informational => "Informational",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixEdit {
    pub source: DiagnosticSource,
    pub span: Span,
    pub replacement: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticFix {
    pub title: String,
    pub applicability: FixApplicability,
    pub edits: Vec<FixEdit>,
}

// Kept as a compatibility view while call sites migrate to `DiagnosticFix`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixIt {
    pub span: Span,
    pub replacement: String,
}

// Kept as a compatibility view while call sites migrate to `DiagnosticLabel`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelatedSpan {
    pub span: Span,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticDocumentation {
    pub slug: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: &'static str,
    /// Original semantic detail retained for compatibility and developer tooling.
    pub message: String,
    /// Canonical, user-facing Title Case summary.
    pub title: String,
    pub severity: DiagnosticSeverity,
    pub kind: DiagnosticKind,
    pub span: Span,
    pub labels: Vec<DiagnosticLabel>,
    pub explanation: Option<String>,
    pub notes: Vec<String>,
    pub helps: Vec<String>,
    pub fixes: Vec<DiagnosticFix>,
    pub cause_id: Option<String>,
    /// Marks a finding as a consequence of another diagnostic with the same
    /// explicit cause identity. Consequences are attached to that root rather
    /// than counted as independent failures.
    pub is_consequence: bool,
    pub development_only: bool,
    pub documentation: Option<DiagnosticDocumentation>,
    pub developer_details: Option<String>,
    // Compatibility views used by existing compiler passes and tests.
    pub help: Option<String>,
    pub fix: Option<Box<FixIt>>,
    pub related: Vec<RelatedSpan>,
}

impl Diagnostic {
    pub fn new(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        Self::build(code, message.into(), span, DiagnosticKind::Language)
    }

    pub fn unsupported_stage(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        let mut diagnostic = Self::build(
            code,
            message.into(),
            span,
            DiagnosticKind::UnsupportedDevelopmentSurface,
        );
        diagnostic.development_only = true;
        diagnostic
    }

    fn build(code: &'static str, message: String, span: Span, kind: DiagnosticKind) -> Self {
        debug_assert!(
            is_catalogued_code(code),
            "diagnostic code `{code}` is missing from the central catalogue"
        );
        if kind != DiagnosticKind::UnsupportedDevelopmentSurface {
            debug_assert!(
                !contains_development_stage_reference(&message),
                "user-facing diagnostic `{code}` exposes a development stage: {message}"
            );
        }
        let metadata = catalogue_metadata(code);
        let kind = if kind == DiagnosticKind::Language {
            metadata.kind
        } else {
            kind
        };
        let title = match kind {
            DiagnosticKind::InternalCompiler => "Internal Compiler Error".to_string(),
            DiagnosticKind::Backend | DiagnosticKind::ExternalTool => {
                metadata.title_family.to_string()
            }
            _ => to_title_case(&message),
        };
        debug_assert!(
            is_title_case(&title),
            "diagnostic title for `{code}` is not Title Case: {title}"
        );
        let mut diagnostic = Self {
            code,
            title,
            severity: metadata.severity,
            kind,
            span,
            labels: vec![DiagnosticLabel {
                source: DiagnosticSource::Current,
                span,
                role: LabelRole::Primary,
                message: if matches!(
                    kind,
                    DiagnosticKind::Backend
                        | DiagnosticKind::ExternalTool
                        | DiagnosticKind::InternalCompiler
                ) {
                    String::new()
                } else {
                    message.clone()
                },
            }],
            explanation: match kind {
                DiagnosticKind::InternalCompiler => Some(
                    "The compiler reached an unexpected internal state. This is a compiler defect, not a problem with your program."
                        .to_string(),
                ),
                DiagnosticKind::Backend | DiagnosticKind::ExternalTool => Some(
                    "The program passed language checking, but a code-generation or external tool step could not complete."
                        .to_string(),
                ),
                _ => None,
            },
            notes: Vec::new(),
            helps: Vec::new(),
            fixes: Vec::new(),
            cause_id: None,
            is_consequence: false,
            development_only: metadata.development_only,
            documentation: Some(DiagnosticDocumentation {
                slug: code.to_ascii_lowercase(),
                url: Some(format!(
                    "https://dorialang.org/docs/diagnostics/{}",
                    code.to_ascii_lowercase()
                )),
            }),
            developer_details: matches!(
                kind,
                DiagnosticKind::InternalCompiler
                    | DiagnosticKind::Backend
                    | DiagnosticKind::ExternalTool
            )
            .then(|| message.clone()),
            message,
            help: None,
            fix: None,
            related: Vec::new(),
        };
        if kind == DiagnosticKind::InternalCompiler {
            diagnostic.notes.push(format!(
                "Toolchain {} ({})",
                crate::TOOLCHAIN_VERSION,
                crate::BUILD_COMMIT
            ));
            diagnostic.helps.push(
                "Please report this diagnostic with the source that triggered it.".to_string(),
            );
        }
        diagnostic
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        let title = title.into();
        debug_assert!(
            is_title_case(&title),
            "diagnostic title for `{}` is not Title Case: {title}",
            self.code
        );
        self.title = title;
        self
    }

    pub fn with_severity(mut self, severity: DiagnosticSeverity) -> Self {
        self.severity = severity;
        self
    }

    pub fn with_kind(mut self, kind: DiagnosticKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn with_explanation(mut self, explanation: impl Into<String>) -> Self {
        self.explanation = Some(explanation.into());
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        let help = help.into();
        debug_assert!(
            !contains_development_stage_reference(&help),
            "user-facing help for `{}` exposes a development stage: {help}",
            self.code
        );
        self.help = Some(help.clone());
        self.helps.push(help);
        self
    }

    pub fn with_fix(mut self, span: Span, replacement: impl Into<String>) -> Self {
        let replacement = replacement.into();
        let title = self
            .help
            .as_deref()
            .map(to_title_case)
            .unwrap_or_else(|| "Apply Suggested Fix".to_string());
        self.fix = Some(Box::new(FixIt {
            span,
            replacement: replacement.clone(),
        }));
        self.fixes.push(DiagnosticFix {
            title,
            applicability: FixApplicability::MachineApplicable,
            edits: vec![FixEdit {
                source: DiagnosticSource::Current,
                span,
                replacement,
            }],
        });
        self
    }

    pub fn with_structured_fix(
        mut self,
        title: impl Into<String>,
        applicability: FixApplicability,
        edits: Vec<FixEdit>,
    ) -> Self {
        self.fixes.push(DiagnosticFix {
            title: title.into(),
            applicability,
            edits,
        });
        self
    }

    pub fn with_related(mut self, span: Span, message: impl Into<String>) -> Self {
        let message = message.into();
        self.related.push(RelatedSpan {
            span,
            message: message.clone(),
        });
        self.labels.push(DiagnosticLabel {
            source: DiagnosticSource::Current,
            span,
            role: LabelRole::Secondary,
            message,
        });
        self
    }

    pub fn with_label(
        mut self,
        source: DiagnosticSource,
        span: Span,
        role: LabelRole,
        message: impl Into<String>,
    ) -> Self {
        self.labels.push(DiagnosticLabel {
            source,
            span,
            role,
            message: message.into(),
        });
        self
    }

    pub fn with_primary_label(mut self, message: impl Into<String>) -> Self {
        let message = message.into();
        if let Some(primary) = self
            .labels
            .iter_mut()
            .find(|label| label.role == LabelRole::Primary)
        {
            primary.message = message;
        } else {
            self.labels.push(DiagnosticLabel {
                source: DiagnosticSource::Current,
                span: self.span,
                role: LabelRole::Primary,
                message,
            });
        }
        self
    }

    pub fn with_cause(mut self, cause_id: impl Into<String>) -> Self {
        self.cause_id = Some(cause_id.into());
        self
    }

    pub fn as_consequence(mut self) -> Self {
        debug_assert!(
            self.cause_id.is_some(),
            "a diagnostic consequence requires an explicit cause identity"
        );
        self.is_consequence = true;
        self
    }

    pub fn with_developer_details(mut self, details: impl Into<String>) -> Self {
        self.developer_details = Some(details.into());
        self
    }

    pub fn internal_compiler_error(
        context: impl Into<String>,
        details: impl Into<String>,
        span: Span,
    ) -> Self {
        Self::build(
            "I0001",
            context.into(),
            span,
            DiagnosticKind::InternalCompiler,
        )
        .with_title("Internal Compiler Error")
        .with_developer_details(details)
    }

    pub fn render(&self, source: &SourceFile) -> String {
        render_diagnostics(source, std::slice::from_ref(self), RenderOptions::default())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticFormat {
    Human,
    Concise,
    Json,
}

impl DiagnosticFormat {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "human" => Some(Self::Human),
            "concise" => Some(Self::Concise),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

impl ColorChoice {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "always" => Some(Self::Always),
            "never" => Some(Self::Never),
            _ => None,
        }
    }

    fn enabled(self) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderOptions {
    pub format: DiagnosticFormat,
    pub color: ColorChoice,
    pub context_lines: usize,
    pub terminal_width: usize,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            format: DiagnosticFormat::Human,
            color: ColorChoice::Auto,
            context_lines: 1,
            terminal_width: 100,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiagnosticSummary {
    pub errors: usize,
    pub warnings: usize,
    pub notes: usize,
}

impl DiagnosticSummary {
    pub fn from_diagnostics(diagnostics: &[&Diagnostic]) -> Self {
        let mut summary = Self {
            errors: 0,
            warnings: 0,
            notes: 0,
        };
        for diagnostic in diagnostics {
            match diagnostic.severity {
                DiagnosticSeverity::Error => summary.errors += 1,
                DiagnosticSeverity::Warning => summary.warnings += 1,
                DiagnosticSeverity::Note => summary.notes += 1,
            }
        }
        summary
    }

    pub fn status(self) -> &'static str {
        if self.errors > 0 {
            "Compilation Failed"
        } else if self.warnings > 0 {
            "Compilation Completed With Warnings"
        } else {
            "Compilation Completed"
        }
    }
}

pub fn render_diagnostics(
    source: &SourceFile,
    diagnostics: &[Diagnostic],
    options: RenderOptions,
) -> String {
    let prepared = prepare_diagnostics(diagnostics);
    let prepared = prepared.iter().collect::<Vec<_>>();
    match options.format {
        DiagnosticFormat::Human => render_human(source, &prepared, options),
        DiagnosticFormat::Concise => render_concise(source, &prepared),
        DiagnosticFormat::Json => diagnostics_json(source, &prepared),
    }
}

fn render_human(
    source: &SourceFile,
    diagnostics: &[&Diagnostic],
    options: RenderOptions,
) -> String {
    let color = options.color.enabled();
    let mut rendered = Vec::with_capacity(diagnostics.len() + 1);
    for diagnostic in diagnostics {
        let prefix = format!("{}[{}]", diagnostic.severity.title(), diagnostic.code);
        let prefix = if color {
            let ansi = match diagnostic.severity {
                DiagnosticSeverity::Error => "31;1",
                DiagnosticSeverity::Warning => "33;1",
                DiagnosticSeverity::Note => "36;1",
            };
            format!("\x1b[{ansi}m{prefix}\x1b[0m")
        } else {
            prefix
        };
        let mut block = format!("{prefix}: {}", diagnostic.title);
        let mut labels = diagnostic.labels.clone();
        labels.sort_by_key(|label| {
            (
                source_name(&label.source, source).to_string(),
                label.span.start,
                label.role != LabelRole::Primary,
            )
        });
        for label in &labels {
            render_label(&mut block, source, label, options);
        }
        if let Some(explanation) = &diagnostic.explanation {
            push_prose_section(&mut block, "Why", explanation, options.terminal_width, true);
        }
        for note in &diagnostic.notes {
            push_prose_section(&mut block, "Note", note, options.terminal_width, false);
        }
        for help in &diagnostic.helps {
            push_prose_section(&mut block, "Help", help, options.terminal_width, false);
        }
        for fix in &diagnostic.fixes {
            block.push_str(&format!(
                "\nSuggested Fix ({}): {}",
                fix.applicability.title(),
                fix.title
            ));
            for edit in &fix.edits {
                let location = display_location(source, &edit.source, edit.span.start);
                block.push_str(&format!(
                    "\n  {location} replace with `{}`",
                    edit.replacement
                ));
            }
        }
        if let Some(cause) = &diagnostic.cause_id {
            block.push_str("\nCaused By: ");
            block.push_str(cause);
        }
        if diagnostic.kind == DiagnosticKind::InternalCompiler {
            if let Some(details) = &diagnostic.developer_details {
                block.push_str("\nNote: developer details are available in JSON diagnostics.");
                if std::env::var_os("DORIA_DIAGNOSTIC_DEBUG").is_some() {
                    block.push_str("\nDeveloper Details: ");
                    block.push_str(details);
                }
            }
        }
        rendered.push(block);
    }
    let summary = DiagnosticSummary::from_diagnostics(diagnostics);
    rendered.push(format_summary(summary));
    rendered.join("\n\n")
}

fn render_label(
    rendered: &mut String,
    source: &SourceFile,
    label: &DiagnosticLabel,
    options: RenderOptions,
) {
    let (line, col) = display_line_col(source, label.span.start, 4);
    let label_source = source_name(&label.source, source);
    rendered.push_str(&format!("\n  --> {label_source}:{line}:{col}\n   |"));

    if matches!(label.source, DiagnosticSource::Path(ref path) if path != &source.path) {
        if !label.message.is_empty() {
            rendered.push_str("\nRelated: ");
            rendered.push_str(&label.message);
        }
        return;
    }

    let (end_line, _) = source.line_col(label.span.end.min(source.text.len()));
    let first_line = line.saturating_sub(options.context_lines).max(1);
    let last_line = (end_line + options.context_lines).min(source.line_count());
    let gutter_width = last_line.to_string().len().max(3);
    for context_line in first_line..=last_line {
        let raw_line = source.line_text(context_line);
        let display_line = expand_tabs(raw_line, 4);
        let maximum_width = options.terminal_width.saturating_sub(12).max(8);
        let line_start = source.line_start(context_line);
        let line_end = line_start.saturating_add(raw_line.len());
        let is_labelled = context_line >= line
            && context_line <= end_line
            && label.span.start <= line_end
            && label.span.end >= line_start;
        let marker_start_byte = label.span.start.max(line_start).min(line_end);
        let marker_end_byte = label.span.end.min(line_end).max(marker_start_byte);
        let before = source.text.get(line_start..marker_start_byte).unwrap_or("");
        let selected = source
            .text
            .get(marker_start_byte..marker_end_byte)
            .unwrap_or("");
        let marker_offset = display_width_with_tabs(before, 4);
        let marker_width = display_width_with_tabs(selected, 4).max(1);
        let (visible_line, visible_marker_offset) = if is_labelled {
            visible_window(&display_line, marker_offset, marker_width, maximum_width)
        } else {
            (
                truncate_display(&display_line, maximum_width),
                marker_offset,
            )
        };
        rendered.push_str(&format!(
            "\n{:>gutter_width$} | {}",
            context_line, visible_line
        ));
        if is_labelled {
            let marker = if label.role == LabelRole::Primary {
                '^'
            } else {
                '-'
            };
            rendered.push_str(&format!(
                "\n{:>gutter_width$} | {}{}",
                "",
                " ".repeat(visible_marker_offset.min(maximum_width)),
                marker.to_string().repeat(
                    marker_width
                        .min(maximum_width.saturating_sub(visible_marker_offset))
                        .max(1)
                )
            ));
            if context_line == line && !label.message.is_empty() {
                rendered.push(' ');
                rendered.push_str(&label.message);
            }
        }
    }
}

fn push_prose_section(
    rendered: &mut String,
    heading: &str,
    prose: &str,
    terminal_width: usize,
    separate: bool,
) {
    if separate {
        rendered.push('\n');
    }
    rendered.push('\n');
    rendered.push_str(heading);
    rendered.push_str(":\n");
    let line_width = terminal_width.saturating_sub(2).max(20);
    for paragraph in prose.lines() {
        if paragraph.is_empty() {
            rendered.push_str("  \n");
            continue;
        }
        let mut current = String::new();
        for word in paragraph.split_whitespace() {
            let next_width = if current.is_empty() {
                word.width()
            } else {
                current.width() + 1 + word.width()
            };
            if !current.is_empty() && next_width > line_width {
                rendered.push_str("  ");
                rendered.push_str(&current);
                rendered.push('\n');
                current.clear();
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
        rendered.push_str("  ");
        rendered.push_str(&current);
        rendered.push('\n');
    }
    rendered.pop();
}

fn render_concise(source: &SourceFile, diagnostics: &[&Diagnostic]) -> String {
    let mut lines = diagnostics
        .iter()
        .map(|diagnostic| {
            let primary = diagnostic
                .labels
                .iter()
                .find(|label| label.role == LabelRole::Primary)
                .or_else(|| diagnostic.labels.first());
            let (path, line, col) = primary.map_or((source.path.as_str(), 1, 1), |label| {
                let (line, col) = display_line_col(source, label.span.start, 4);
                (source_name(&label.source, source), line, col)
            });
            format!(
                "{path}:{line}:{col}: {}[{}]: {}",
                diagnostic.severity.title(),
                diagnostic.code,
                diagnostic.title
            )
        })
        .collect::<Vec<_>>();
    let summary = DiagnosticSummary::from_diagnostics(diagnostics);
    lines.push(format_summary(summary));
    lines.join("\n")
}

fn diagnostics_json(source: &SourceFile, diagnostics: &[&Diagnostic]) -> String {
    let summary = DiagnosticSummary::from_diagnostics(diagnostics);
    let values = diagnostics
        .iter()
        .map(|diagnostic| {
            serde_json::json!({
                "code": diagnostic.code,
                "severity": diagnostic.severity.as_str(),
                "kind": diagnostic.kind.as_str(),
                "title": diagnostic.title,
                "message": diagnostic.message,
                "explanation": diagnostic.explanation,
                "labels": diagnostic.labels.iter().map(|label| {
                    let (line, column) = display_line_col(source, label.span.start, 4);
                    let (end_line, end_column) = display_line_col(source, label.span.end, 4);
                    serde_json::json!({
                        "source": source_name(&label.source, source),
                        "role": label.role.as_str(),
                        "message": label.message,
                        "span": {
                            "start": label.span.start,
                            "end": label.span.end,
                        },
                        "range": {
                            "start": { "line": line, "column": column },
                            "end": { "line": end_line, "column": end_column },
                        },
                    })
                }).collect::<Vec<_>>(),
                "notes": diagnostic.notes,
                "help": diagnostic.helps,
                "fixes": diagnostic.fixes.iter().map(|fix| serde_json::json!({
                    "title": fix.title,
                    "applicability": fix.applicability.as_str(),
                    "edits": fix.edits.iter().map(|edit| serde_json::json!({
                        "source": source_name(&edit.source, source),
                        "span": { "start": edit.span.start, "end": edit.span.end },
                        "replacement": edit.replacement,
                    })).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
                "causeId": diagnostic.cause_id,
                "developmentOnly": diagnostic.development_only,
                "documentation": diagnostic.documentation.as_ref().map(|docs| serde_json::json!({
                    "slug": docs.slug,
                    "url": docs.url,
                })),
                "developerDetails": diagnostic.developer_details,
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&serde_json::json!({
        "schemaVersion": DIAGNOSTIC_SCHEMA_VERSION,
        "diagnostics": values,
        "summary": {
            "status": summary.status(),
            "errors": summary.errors,
            "warnings": summary.warnings,
            "notes": summary.notes,
        },
    }))
    .unwrap_or_else(|_| {
        "{\"schemaVersion\":1,\"diagnostics\":[],\"summary\":{\"status\":\"Internal Compiler Error\",\"errors\":1,\"warnings\":0,\"notes\":0}}".to_string()
    })
}

/// Applies presentation-independent duplicate and explicit cause grouping.
///
/// Every consumer (CLI, language server, Playground JSON) must use this rather
/// than implementing its own fuzzy or protocol-specific suppression.
pub fn prepare_diagnostics(diagnostics: &[Diagnostic]) -> Vec<Diagnostic> {
    let mut seen = HashSet::new();
    let mut prepared = diagnostics
        .iter()
        .filter(|diagnostic| {
            let primary = diagnostic
                .labels
                .iter()
                .find(|label| label.role == LabelRole::Primary)
                .or_else(|| diagnostic.labels.first());
            let source = primary
                .map(|label| format!("{:?}", label.source))
                .unwrap_or_default();
            let span = primary.map(|label| label.span).unwrap_or(diagnostic.span);
            seen.insert((
                diagnostic.code,
                source,
                span.start,
                span.end,
                diagnostic.title.as_str(),
                diagnostic.cause_id.as_deref(),
            ))
        })
        .cloned()
        .collect::<Vec<_>>();

    let roots = prepared
        .iter()
        .enumerate()
        .filter_map(|(index, diagnostic)| {
            (!diagnostic.is_consequence)
                .then(|| {
                    diagnostic
                        .cause_id
                        .as_ref()
                        .map(|cause| (cause.clone(), index))
                })
                .flatten()
        })
        .collect::<std::collections::HashMap<_, _>>();
    let consequences = prepared
        .iter()
        .filter(|diagnostic| diagnostic.is_consequence)
        .cloned()
        .collect::<Vec<_>>();
    for consequence in consequences {
        let Some(cause) = consequence.cause_id.as_ref() else {
            continue;
        };
        let Some(&root_index) = roots.get(cause) else {
            continue;
        };
        let root = &mut prepared[root_index];
        root.labels
            .extend(consequence.labels.into_iter().map(|mut label| {
                label.role = LabelRole::Secondary;
                label
            }));
        root.notes.push(format!(
            "{}: {}",
            consequence.title,
            consequence
                .explanation
                .as_deref()
                .unwrap_or(&consequence.message)
        ));
        root.helps.extend(consequence.helps);
        root.fixes.extend(consequence.fixes);
    }
    prepared.retain(|diagnostic| {
        !diagnostic.is_consequence
            || diagnostic
                .cause_id
                .as_ref()
                .is_none_or(|cause| !roots.contains_key(cause))
    });
    prepared
}

fn source_name<'a>(source: &'a DiagnosticSource, current: &'a SourceFile) -> &'a str {
    match source {
        DiagnosticSource::Current => &current.path,
        DiagnosticSource::Path(path) => path,
    }
}

fn display_location(source: &SourceFile, identity: &DiagnosticSource, offset: usize) -> String {
    if matches!(identity, DiagnosticSource::Current)
        || matches!(identity, DiagnosticSource::Path(path) if path == &source.path)
    {
        let (line, column) = display_line_col(source, offset, 4);
        format!("{}:{line}:{column}", source_name(identity, source))
    } else {
        format!("{} at byte {offset}", source_name(identity, source))
    }
}

fn display_line_col(source: &SourceFile, byte_index: usize, tab_width: usize) -> (usize, usize) {
    let safe_index = byte_index.min(source.text.len());
    let (line, _) = source.line_col(safe_index);
    let line_start = source.line_start(line);
    let prefix = source.text.get(line_start..safe_index).unwrap_or("");
    (line, display_width_with_tabs(prefix, tab_width) + 1)
}

fn expand_tabs(text: &str, tab_width: usize) -> String {
    let mut expanded = String::new();
    let mut width = 0;
    for character in text.chars() {
        if character == '\t' {
            let spaces = tab_width - (width % tab_width);
            expanded.push_str(&" ".repeat(spaces));
            width += spaces;
        } else {
            expanded.push(character);
            width += character.width().unwrap_or(0);
        }
    }
    expanded
}

fn display_width_with_tabs(text: &str, tab_width: usize) -> usize {
    expand_tabs(text, tab_width).width()
}

fn truncate_display(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    if width <= 1 {
        return "…".to_string();
    }
    let mut output = String::new();
    let mut used = 0;
    for character in text.chars() {
        let character_width = character.width().unwrap_or(0);
        if used + character_width >= width {
            break;
        }
        output.push(character);
        used += character_width;
    }
    output.push('…');
    output
}

fn visible_window(
    text: &str,
    marker_start: usize,
    marker_width: usize,
    maximum_width: usize,
) -> (String, usize) {
    if text.width() <= maximum_width {
        return (text.to_string(), marker_start);
    }
    let content_width = maximum_width.saturating_sub(2).max(1);
    let desired_start = marker_start
        .saturating_sub(content_width / 3)
        .min(text.width().saturating_sub(content_width));
    let desired_end = desired_start + content_width;
    let mut output = String::new();
    let mut display_column = 0;
    let mut actual_start = desired_start;
    for character in text.chars() {
        let width = character.width().unwrap_or(0);
        let next = display_column + width;
        if next > desired_start && display_column < desired_end {
            if output.is_empty() {
                actual_start = display_column;
            }
            output.push(character);
        }
        display_column = next;
        if display_column >= desired_end {
            break;
        }
    }
    let has_left = actual_start > 0;
    let has_right = display_column < text.width();
    if has_left {
        output.insert(0, '…');
    }
    if has_right {
        output.push('…');
    }
    let adjusted = marker_start.saturating_sub(actual_start) + usize::from(has_left);
    let marker_capacity = maximum_width.saturating_sub(adjusted).max(1);
    (
        output,
        adjusted.min(maximum_width.saturating_sub(marker_width.min(marker_capacity))),
    )
}

fn format_summary(summary: DiagnosticSummary) -> String {
    let mut counts = Vec::new();
    if summary.errors > 0 {
        counts.push(format!(
            "{} Error{}",
            summary.errors,
            if summary.errors == 1 { "" } else { "s" }
        ));
    }
    if summary.warnings > 0 {
        counts.push(format!(
            "{} Warning{}",
            summary.warnings,
            if summary.warnings == 1 { "" } else { "s" }
        ));
    }
    if counts.is_empty() && summary.notes > 0 {
        counts.push(format!(
            "{} Note{}",
            summary.notes,
            if summary.notes == 1 { "" } else { "s" }
        ));
    }
    format!("{}: {}", summary.status(), counts.join(" And "))
}

fn to_title_case(message: &str) -> String {
    let mut in_code = false;
    message
        .split_inclusive(char::is_whitespace)
        .enumerate()
        .map(|(index, word)| {
            for character in word.chars() {
                if character == '`' {
                    in_code = !in_code;
                }
            }
            if in_code || word.starts_with('`') {
                return word.to_string();
            }
            let trimmed = word.trim_start_matches(|character: char| !character.is_alphabetic());
            let prefix_len = word.len().saturating_sub(trimmed.len());
            let (prefix, rest) = word.split_at(prefix_len);
            let mut characters = rest.chars();
            let Some(first) = characters.next() else {
                return word.to_string();
            };
            let minor = matches!(
                rest.trim_end(),
                "a" | "an"
                    | "and"
                    | "as"
                    | "at"
                    | "but"
                    | "by"
                    | "for"
                    | "from"
                    | "in"
                    | "into"
                    | "nor"
                    | "of"
                    | "on"
                    | "or"
                    | "per"
                    | "the"
                    | "to"
                    | "via"
                    | "with"
                    | "without"
            );
            if index > 0 && minor {
                word.to_string()
            } else {
                format!("{prefix}{}{}", first.to_uppercase(), characters.as_str())
            }
        })
        .collect()
}

pub fn is_title_case(title: &str) -> bool {
    if title.is_empty() || title.contains(['\n', '\r']) || title.ends_with('.') {
        return false;
    }
    let mut in_code = false;
    for character in title.chars() {
        if character == '`' {
            in_code = !in_code;
            continue;
        }
        if !in_code && character.is_ascii_alphabetic() {
            return character.is_ascii_uppercase();
        }
    }
    true
}

#[derive(Debug, Clone, Copy)]
pub struct DiagnosticMetadata {
    pub severity: DiagnosticSeverity,
    pub kind: DiagnosticKind,
    pub title_family: &'static str,
    pub development_only: bool,
}

pub fn catalogue_metadata(code: &str) -> DiagnosticMetadata {
    let kind = match code.as_bytes().first().copied() {
        Some(b'B') => DiagnosticKind::Backend,
        Some(b'I') => DiagnosticKind::InternalCompiler,
        _ => DiagnosticKind::Language,
    };
    DiagnosticMetadata {
        severity: DiagnosticSeverity::Error,
        kind,
        title_family: match code.as_bytes().first().copied() {
            Some(b'L') => "Lexical Error",
            Some(b'P') => "Syntax Error",
            Some(b'E') => "Language Error",
            Some(b'M') => "MIR Error",
            Some(b'B') => "Backend Error",
            Some(b'I') => "Internal Compiler Error",
            _ => "Compiler Diagnostic",
        },
        development_only: false,
    }
}

fn is_catalogued_code(code: &str) -> bool {
    CATALOGUED_CODES.contains(&code)
}

pub const CATALOGUED_CODES: &[&str] = &[
    "B0001", "B0002", "B1301", "B1901", "B2001", "B2301", "B2401", "E0101", "E0102", "E0103",
    "E0201", "E0202", "E0203", "E0204", "E0300", "E0303", "E0304", "E0305", "E0306", "E0307",
    "E0308", "E0309", "E0310", "E0401", "E0402", "E0403", "E0404", "E0405", "E0406", "E0407",
    "E0408", "E0409", "E0410", "E0411", "E0412", "E0413", "E0414", "E0415", "E0416", "E0417",
    "E0419", "E0420", "E0421", "E0422", "E0423", "E0424", "E0425", "E0426", "E0430", "E0431",
    "E0432", "E0433", "E0434", "E0435", "E0436", "E0440", "E0441", "E0442", "E0443", "E0444",
    "E0445", "E0450", "E0451", "E0452", "E0453", "E0454", "E0455", "E0456", "E0457", "E0461",
    "E0462", "E0463", "E0464", "E0465", "E0466", "E0467", "E0468", "E0470", "E0471", "E0472",
    "E0473", "E0474", "E0475", "E0476", "E0477", "E0478", "E0479", "E0480", "E0481", "E0482",
    "E0483", "E0484", "E0485", "E0486", "E0487", "E0488", "E0489", "E0490", "E0491", "E0492",
    "E0493", "E0494", "E0495", "E0496", "E0497", "E0498", "E0500", "E0501", "E0502", "E0503",
    "E0504", "E0505", "E0506", "E0507", "E0508", "E0509", "E0510", "E0511", "E0512", "E0513",
    "E0515", "E0516", "E0517", "E0518", "E0519", "E0520", "E0521", "E0522", "E0523", "E0524",
    "E0525", "E0526", "E0527", "E0528", "E0529", "E0530", "E0531", "E0532", "E0533", "E0534",
    "E0535", "E0536", "E0537", "E0538", "E0539", "E0540", "E0541", "E0542", "E0543", "E0544",
    "E0545", "E0546", "E0547", "E0548", "E0549", "E0550", "I0001", "I1101", "I1301", "I1302",
    "I1401", "I2001", "I2002", "I2003", "I2201", "I2401", "L0001", "L0002", "M1101", "M1102",
    "P0001", "P0002", "P0017",
];

fn contains_development_stage_reference(text: &str) -> bool {
    let mut words = text.split_whitespace();
    while let Some(word) = words.next() {
        if word.trim_matches(|character: char| !character.is_ascii_alphanumeric()) == "Stage"
            && words.next().is_some_and(|next| {
                next.trim_matches(|character: char| !character.is_ascii_alphanumeric())
                    .starts_with(|character: char| character.is_ascii_digit())
            })
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{
        contains_development_stage_reference, is_title_case, render_diagnostics, ColorChoice,
        Diagnostic, DiagnosticFormat, DiagnosticKind, DiagnosticSeverity, DiagnosticSource,
        FixApplicability, FixEdit, LabelRole, RenderOptions,
    };
    use crate::source::{SourceFile, Span};

    #[test]
    fn user_facing_diagnostics_reject_numbered_development_stages() {
        assert!(contains_development_stage_reference(
            "unsupported MIR Stage 11 coverage"
        ));
        assert!(contains_development_stage_reference(
            "planned for Stage 35."
        ));
        assert!(!contains_development_stage_reference(
            "class property access is not supported by native compilation"
        ));
    }

    #[test]
    fn titles_and_prefixes_use_title_case() {
        let source = SourceFile::new("main.doria", "let $x = unknown;\n");
        let diagnostic = Diagnostic::new("E0201", "unknown variable `unknown`", Span::new(9, 16));
        let rendered = render_diagnostics(
            &source,
            &[diagnostic],
            RenderOptions {
                color: ColorChoice::Never,
                ..RenderOptions::default()
            },
        );
        assert!(rendered.starts_with("Error[E0201]: Unknown Variable `unknown`"));
        assert!(rendered.ends_with("Compilation Failed: 1 Error"));
        assert!(is_title_case("Internal Compiler Error"));
    }

    #[test]
    fn json_is_versioned_structured_and_never_contains_ansi() {
        let source = SourceFile::new("main.doria", "readonly int $x = 1;\n$x = 2;\n");
        let diagnostic = Diagnostic::new(
            "E0406",
            "cannot assign to readonly variable",
            Span::new(21, 23),
        )
        .with_kind(DiagnosticKind::Language)
        .with_explanation("Readonly bindings cannot be changed after initialization.")
        .with_related(Span::new(13, 14), "binding declared readonly here")
        .with_structured_fix(
            "Remove Readonly",
            FixApplicability::RequiresReview,
            vec![FixEdit {
                source: DiagnosticSource::Current,
                span: Span::new(0, 9),
                replacement: String::new(),
            }],
        )
        .with_label(
            DiagnosticSource::Path("other.doria".to_string()),
            Span::new(0, 1),
            LabelRole::Secondary,
            "related declaration",
        );
        let json = render_diagnostics(
            &source,
            &[diagnostic],
            RenderOptions {
                format: DiagnosticFormat::Json,
                color: ColorChoice::Always,
                ..RenderOptions::default()
            },
        );
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["diagnostics"][0]["severity"], "error");
        assert_eq!(
            value["diagnostics"][0]["labels"].as_array().unwrap().len(),
            3
        );
        assert!(!json.contains("\u{1b}["));
    }

    #[test]
    fn renderer_aligns_tabs_unicode_multiline_spans_and_narrow_windows() {
        let text = "\t🙂 prefix with a deliberately long description\n\tsecond $value line\n";
        let source = SourceFile::new("unicode.doria", text);
        let start = text.find("prefix").unwrap();
        let end = text.find("$value").unwrap() + "$value".len();
        let rendered = render_diagnostics(
            &source,
            &[Diagnostic::new(
                "E0403",
                "value does not match the expected type",
                Span::new(start, end),
            )
            .with_help("Use a value with the declared type.")
            .with_help("Check the declaration that establishes the expected type.")],
            RenderOptions {
                color: ColorChoice::Never,
                terminal_width: 38,
                context_lines: 0,
                ..RenderOptions::default()
            },
        );
        assert!(rendered.contains("unicode.doria:1:8"));
        assert!(rendered.contains("…"));
        assert!(rendered.contains("second $value"));
        assert_eq!(rendered.matches("Help:").count(), 2);
        assert!(rendered.lines().filter(|line| line.contains('^')).count() >= 2);
    }

    #[test]
    fn ansi_and_plain_renderers_preserve_the_same_information() {
        let source = SourceFile::new("main.doria", "unknown;\n");
        let diagnostic = Diagnostic::new("E0201", "unknown identifier", Span::new(0, 7));
        let plain = render_diagnostics(
            &source,
            std::slice::from_ref(&diagnostic),
            RenderOptions {
                color: ColorChoice::Never,
                ..RenderOptions::default()
            },
        );
        let ansi = render_diagnostics(
            &source,
            &[diagnostic],
            RenderOptions {
                color: ColorChoice::Always,
                ..RenderOptions::default()
            },
        );
        assert_eq!(
            ansi.replace("\u{1b}[31;1m", "").replace("\u{1b}[0m", ""),
            plain
        );
    }

    #[test]
    fn duplicate_and_causal_consequence_control_preserve_independent_errors() {
        let source = SourceFile::new("main.doria", "$missing;\n$other;\n");
        let root = Diagnostic::new("E0201", "unknown identifier `$missing`", Span::new(0, 8))
            .with_cause("unknown-missing");
        let duplicate = root.clone();
        let consequence =
            Diagnostic::new("E0403", "cannot determine the value type", Span::new(0, 8))
                .with_explanation(
                    "The type is unknown because the identifier could not be resolved.",
                )
                .with_cause("unknown-missing")
                .as_consequence();
        let independent =
            Diagnostic::new("E0201", "unknown identifier `$other`", Span::new(10, 16))
                .with_cause("unknown-other");
        let json = render_diagnostics(
            &source,
            &[root, duplicate, consequence, independent],
            RenderOptions {
                format: DiagnosticFormat::Json,
                ..RenderOptions::default()
            },
        );
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["summary"]["errors"], 2);
        assert_eq!(value["diagnostics"].as_array().unwrap().len(), 2);
        assert_eq!(
            value["diagnostics"][0]["labels"].as_array().unwrap().len(),
            2
        );
        assert!(value["diagnostics"][0]["notes"][0]
            .as_str()
            .unwrap()
            .contains("Cannot Determine"));
    }

    #[test]
    fn severities_summaries_and_fix_applicability_are_structured() {
        let source = SourceFile::new("main.doria", "value;\n");
        let base = Diagnostic::new("E0403", "value has the wrong type", Span::new(0, 5))
            .with_structured_fix(
                "Apply Exact Replacement",
                FixApplicability::MachineApplicable,
                vec![FixEdit {
                    source: DiagnosticSource::Current,
                    span: Span::new(0, 5),
                    replacement: "other".to_string(),
                }],
            )
            .with_structured_fix(
                "Review The Intended Type",
                FixApplicability::RequiresReview,
                Vec::new(),
            )
            .with_structured_fix(
                "Consider An Explicit Conversion",
                FixApplicability::Informational,
                Vec::new(),
            );
        let warning = Diagnostic::new("E0404", "value may be surprising", Span::new(0, 5))
            .with_severity(DiagnosticSeverity::Warning);
        let note = Diagnostic::new("E0405", "additional type information", Span::new(0, 5))
            .with_severity(DiagnosticSeverity::Note);
        let json = render_diagnostics(
            &source,
            &[base, warning, note],
            RenderOptions {
                format: DiagnosticFormat::Json,
                ..RenderOptions::default()
            },
        );
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["summary"]["errors"], 1);
        assert_eq!(value["summary"]["warnings"], 1);
        assert_eq!(value["summary"]["notes"], 1);
        let fixes = value["diagnostics"][0]["fixes"].as_array().unwrap();
        assert_eq!(fixes[0]["applicability"], "machineApplicable");
        assert_eq!(fixes[1]["applicability"], "requiresReview");
        assert_eq!(fixes[2]["applicability"], "informational");
    }

    #[test]
    fn development_backend_and_internal_envelopes_keep_boundaries_clear() {
        let source = SourceFile::new("main.doria", "value;\n");
        let development =
            Diagnostic::unsupported_stage("M1101", "planned for Stage 31", Span::new(0, 5));
        let backend = Diagnostic::new(
            "B0001",
            "ld: raw linker implementation detail",
            Span::new(0, 5),
        );
        let internal = Diagnostic::internal_compiler_error(
            "indexed assignment lowering",
            "RawRustType { field: 1 }",
            Span::new(0, 5),
        );
        let human = render_diagnostics(
            &source,
            &[development.clone(), backend.clone(), internal.clone()],
            RenderOptions {
                color: ColorChoice::Never,
                ..RenderOptions::default()
            },
        );
        assert!(!human.contains("RawRustType"));
        assert!(!human.contains("ld: raw"));
        let json = render_diagnostics(
            &source,
            &[development, backend, internal],
            RenderOptions {
                format: DiagnosticFormat::Json,
                ..RenderOptions::default()
            },
        );
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["diagnostics"][0]["developmentOnly"], true);
        assert_eq!(value["diagnostics"][1]["kind"], "backend");
        assert!(value["diagnostics"][1]["developerDetails"]
            .as_str()
            .unwrap()
            .contains("linker"));
        assert!(value["diagnostics"][2]["developerDetails"]
            .as_str()
            .unwrap()
            .contains("RawRustType"));
    }

    #[test]
    fn concise_renderer_uses_one_line_per_diagnostic_and_a_grammar_aware_summary() {
        let source = SourceFile::new("main.doria", "first;\nsecond;\n");
        let diagnostics = [
            Diagnostic::new("E0201", "unknown identifier `first`", Span::new(0, 5)),
            Diagnostic::new("E0201", "unknown identifier `second`", Span::new(7, 13)),
        ];
        let concise = render_diagnostics(
            &source,
            &diagnostics,
            RenderOptions {
                format: DiagnosticFormat::Concise,
                ..RenderOptions::default()
            },
        );
        assert_eq!(concise.lines().count(), 3);
        assert!(concise.ends_with("Compilation Failed: 2 Errors"));
    }

    #[test]
    fn warning_only_summary_does_not_report_compilation_failure() {
        let source = SourceFile::new("main.doria", "value;\n");
        let rendered = render_diagnostics(
            &source,
            &[
                Diagnostic::new("E0404", "value may be surprising", Span::new(0, 5))
                    .with_severity(DiagnosticSeverity::Warning),
            ],
            RenderOptions {
                color: ColorChoice::Never,
                ..RenderOptions::default()
            },
        );
        assert!(rendered.ends_with("Compilation Completed With Warnings: 1 Warning"));
    }
}
