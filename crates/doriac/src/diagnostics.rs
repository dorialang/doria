use std::collections::HashSet;
use std::io::IsTerminal;
use std::ops::{Deref, DerefMut};

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

    fn title(self, severity: DiagnosticSeverity) -> &'static str {
        match self {
            Self::RuntimePanic => "Panic",
            _ => severity.title(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminationBehavior {
    AbortWithoutCleanup,
    PropagateWithCleanup,
}

impl TerminationBehavior {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AbortWithoutCleanup => "abortWithoutCleanup",
            Self::PropagateWithCleanup => "propagateWithCleanup",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeOutcomeOrigin {
    pub source: DiagnosticSource,
    pub span: Span,
    pub function: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeOutcomeFrame {
    pub function: String,
    pub source: DiagnosticSource,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeFactValue {
    Signed(i64),
    Unsigned(u64),
    Boolean(bool),
    StaticString(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeFact {
    pub name: String,
    pub value: RuntimeFactValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeOutcomeDetails {
    pub process_status: i32,
    pub termination_behavior: TerminationBehavior,
    pub origin: RuntimeOutcomeOrigin,
    pub path: Vec<RuntimeOutcomeFrame>,
    pub facts: Vec<RuntimeFact>,
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
    pub severity: DiagnosticSeverity,
    pub kind: DiagnosticKind,
    pub span: Span,
    details: Box<DiagnosticDetails>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticDetails {
    /// Original semantic detail retained for compatibility and developer tooling.
    pub message: String,
    /// Canonical, user-facing Title Case summary.
    pub title: String,
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
    pub runtime_outcome: Option<RuntimeOutcomeDetails>,
    // Compatibility views used by existing compiler passes and tests.
    pub help: Option<String>,
    pub fix: Option<Box<FixIt>>,
    pub related: Vec<RelatedSpan>,
}

impl Deref for Diagnostic {
    type Target = DiagnosticDetails;

    fn deref(&self) -> &Self::Target {
        &self.details
    }
}

impl DerefMut for Diagnostic {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.details
    }
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

    pub fn runtime_panic(code: &'static str, span: Span, outcome: RuntimeOutcomeDetails) -> Self {
        let entry = runtime_catalogue_entry(code)
            .unwrap_or_else(|| panic!("runtime diagnostic code `{code}` is not catalogued"));
        let mut diagnostic = Self::build(
            code,
            entry.title.to_string(),
            span,
            DiagnosticKind::RuntimePanic,
        )
        .with_title(entry.title)
        .with_primary_label(entry.primary_label)
        .with_explanation(entry.explanation);
        diagnostic.runtime_outcome = Some(outcome);
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
            severity: metadata.severity,
            kind,
            span,
            details: Box::new(DiagnosticDetails {
                title,
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
                runtime_outcome: None,
                message,
                help: None,
                fix: None,
                related: Vec::new(),
            }),
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
        let span = self.span;
        if let Some(primary) = self
            .labels
            .iter_mut()
            .find(|label| label.role == LabelRole::Primary)
        {
            primary.message = message;
        } else {
            self.labels.push(DiagnosticLabel {
                source: DiagnosticSource::Current,
                span,
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

    pub fn with_runtime_outcome(mut self, outcome: RuntimeOutcomeDetails) -> Self {
        self.kind = DiagnosticKind::RuntimePanic;
        self.runtime_outcome = Some(outcome);
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
        let prefix = format!(
            "{}[{}]",
            diagnostic.kind.title(diagnostic.severity),
            diagnostic.code
        );
        let prefix = if color {
            let ansi = match (diagnostic.kind, diagnostic.severity) {
                (DiagnosticKind::RuntimePanic, _) => "31;1",
                (_, DiagnosticSeverity::Error) => "31;1",
                (_, DiagnosticSeverity::Warning) => "33;1",
                (_, DiagnosticSeverity::Note) => "36;1",
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
        if let Some(primary) = labels.iter().find(|label| label.role == LabelRole::Primary) {
            block.push_str("\n\nWhere\n");
            render_label(
                &mut block,
                source,
                primary,
                options,
                diagnostic
                    .runtime_outcome
                    .as_ref()
                    .and_then(|outcome| outcome.origin.function.as_deref()),
            );
        }
        let related = labels
            .iter()
            .filter(|label| label.role == LabelRole::Secondary)
            .collect::<Vec<_>>();
        if !related.is_empty() {
            block.push_str("\n\nRelated");
            for label in related {
                block.push('\n');
                render_label(&mut block, source, label, options, None);
            }
        }
        if let Some(explanation) = &diagnostic.explanation {
            push_prose_section(&mut block, "Why", explanation, options.terminal_width, true);
        }
        for note in &diagnostic.notes {
            push_prose_section(&mut block, "Note", note, options.terminal_width, true);
        }
        for help in &diagnostic.helps {
            push_prose_section(&mut block, "Help", help, options.terminal_width, true);
        }
        for fix in &diagnostic.fixes {
            block.push_str(&format!(
                "\n\nSuggested Fix\n{}\n{}",
                fix.applicability.title(),
                fix.title
            ));
            for edit in &fix.edits {
                let location = display_location(source, &edit.source, edit.span.start);
                block.push_str(&format!(
                    "\n{location} · Replace With `{}`",
                    edit.replacement
                ));
            }
        }
        if let Some(cause) = &diagnostic.cause_id {
            block.push_str("\n\nCaused By\n");
            block.push_str(cause);
        }
        if matches!(
            diagnostic.kind,
            DiagnosticKind::Backend
                | DiagnosticKind::ExternalTool
                | DiagnosticKind::InternalCompiler
        ) {
            if let Some(details) = &diagnostic.developer_details {
                block.push_str("\n\nNote\nDeveloper details are available in JSON diagnostics.");
                if std::env::var_os("DORIA_DIAGNOSTIC_DEBUG").is_some() {
                    block.push_str("\n\nDeveloper Details\n");
                    block.push_str(details);
                }
            }
        }
        if let Some(outcome) = &diagnostic.runtime_outcome {
            if !outcome.path.is_empty() {
                block.push_str("\n\nCall Path");
                for frame in &outcome.path {
                    let (line, _) = display_line_col(source, frame.span.start, 4);
                    block.push_str(&format!(
                        "\n{} · {}:{}",
                        frame.function,
                        source_name(&frame.source, source),
                        line
                    ));
                }
            }
            block.push_str(&format!(
                "\n\nProcess Exited With Status {}",
                outcome.process_status
            ));
        }
        rendered.push(block);
    }
    if diagnostics
        .iter()
        .all(|diagnostic| diagnostic.runtime_outcome.is_none())
    {
        let summary = DiagnosticSummary::from_diagnostics(diagnostics);
        rendered.push(format_summary(summary));
    }
    rendered.join("\n\n")
}

fn render_label(
    rendered: &mut String,
    source: &SourceFile,
    label: &DiagnosticLabel,
    options: RenderOptions,
    context: Option<&str>,
) {
    let (line, _) = display_line_col(source, label.span.start, 4);
    let label_source = source_name(&label.source, source);
    rendered.push_str(&format!("{label_source} · line {line}"));
    if let Some(context) = context {
        rendered.push_str(" · ");
        rendered.push_str(context);
    }
    rendered.push('\n');

    if matches!(label.source, DiagnosticSource::Path(ref path) if path != &source.path) {
        if !label.message.is_empty() {
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
        if visible_line.is_empty() {
            rendered.push_str(&format!("\n{:>gutter_width$}", context_line));
        } else {
            rendered.push_str(&format!("\n{:>gutter_width$}      ", context_line));
            rendered.push_str(&visible_line);
        }
        if is_labelled {
            let marker = if label.role == LabelRole::Primary {
                '^'
            } else {
                '-'
            };
            rendered.push_str(&format!(
                "\n{:>gutter_width$}      {}{}",
                "",
                " ".repeat(visible_marker_offset.min(maximum_width)),
                marker.to_string().repeat(
                    marker_width
                        .min(maximum_width.saturating_sub(visible_marker_offset))
                        .max(1)
                )
            ));
            if context_line == line && !label.message.is_empty() {
                rendered.push_str(&format!(
                    "\n{:>gutter_width$}      {}",
                    "",
                    " ".repeat(visible_marker_offset.min(maximum_width))
                ));
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
    rendered.push('\n');
    let line_width = terminal_width.max(20);
    for paragraph in prose.lines() {
        if paragraph.is_empty() {
            rendered.push('\n');
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
                rendered.push_str(&current);
                rendered.push('\n');
                current.clear();
            }
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        }
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
                diagnostic.kind.title(diagnostic.severity),
                diagnostic.code,
                diagnostic.title
            )
        })
        .collect::<Vec<_>>();
    if diagnostics
        .iter()
        .all(|diagnostic| diagnostic.runtime_outcome.is_none())
    {
        let summary = DiagnosticSummary::from_diagnostics(diagnostics);
        lines.push(format_summary(summary));
    }
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
                "runtimeOutcome": diagnostic.runtime_outcome.as_ref().map(|outcome| {
                    serde_json::json!({
                        "processStatus": outcome.process_status,
                        "terminationBehavior": outcome.termination_behavior.as_str(),
                        "origin": runtime_origin_json(source, &outcome.origin),
                        "pathKind": "callPath",
                        "frames": outcome.path.iter().map(|frame| {
                            let (line, column) = display_line_col(source, frame.span.start, 4);
                            serde_json::json!({
                                "function": frame.function,
                                "source": source_name(&frame.source, source),
                                "span": {
                                    "start": frame.span.start,
                                    "end": frame.span.end,
                                },
                                "line": line,
                                "column": column,
                            })
                        }).collect::<Vec<_>>(),
                        "facts": outcome.facts.iter().map(runtime_fact_json).collect::<Vec<_>>(),
                    })
                }),
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&serde_json::json!({
        "schemaVersion": DIAGNOSTIC_SCHEMA_VERSION,
        "diagnostics": values,
        "summary": {
            "status": diagnostics.iter().find_map(|diagnostic| {
                diagnostic.runtime_outcome.as_ref().map(|outcome| {
                    format!("Process Exited With Status {}", outcome.process_status)
                })
            }).unwrap_or_else(|| summary.status().to_string()),
            "errors": summary.errors,
            "warnings": summary.warnings,
            "notes": summary.notes,
        },
    }))
    .unwrap_or_else(|_| {
        "{\"schemaVersion\":1,\"diagnostics\":[],\"summary\":{\"status\":\"Internal Compiler Error\",\"errors\":1,\"warnings\":0,\"notes\":0}}".to_string()
    })
}

fn runtime_origin_json(source: &SourceFile, origin: &RuntimeOutcomeOrigin) -> serde_json::Value {
    let (line, column) = display_line_col(source, origin.span.start, 4);
    let (end_line, end_column) = display_line_col(source, origin.span.end, 4);
    serde_json::json!({
        "source": source_name(&origin.source, source),
        "function": origin.function,
        "span": {
            "start": origin.span.start,
            "end": origin.span.end,
        },
        "range": {
            "start": { "line": line, "column": column },
            "end": { "line": end_line, "column": end_column },
        },
    })
}

fn runtime_fact_json(fact: &RuntimeFact) -> serde_json::Value {
    let (kind, value) = match &fact.value {
        RuntimeFactValue::Signed(value) => ("signedInteger", serde_json::json!(value)),
        RuntimeFactValue::Unsigned(value) => ("unsignedInteger", serde_json::json!(value)),
        RuntimeFactValue::Boolean(value) => ("boolean", serde_json::json!(value)),
        RuntimeFactValue::StaticString(value) => ("staticString", serde_json::json!(value)),
    };
    serde_json::json!({
        "name": fact.name,
        "kind": kind,
        "value": value,
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
        let details = *consequence.details;
        let root = &mut prepared[root_index];
        root.labels
            .extend(details.labels.into_iter().map(|mut label| {
                label.role = LabelRole::Secondary;
                label
            }));
        root.notes.push(format!(
            "{}: {}",
            details.title,
            details.explanation.as_deref().unwrap_or(&details.message)
        ));
        root.helps.extend(details.helps);
        root.fixes.extend(details.fixes);
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
            let minor = is_minor_title_word(rest.trim_end());
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
    let mut prose = String::with_capacity(title.len());
    let mut in_code = false;
    for character in title.chars() {
        match character {
            '`' => in_code = !in_code,
            _ if in_code => prose.push(' '),
            _ => prose.push(character),
        }
    }
    if in_code {
        return false;
    }
    prose
        .split_whitespace()
        .filter_map(|word| {
            let word = word.trim_matches(|character: char| !character.is_ascii_alphabetic());
            (!word.is_empty() && !is_minor_title_word(word)).then_some(word)
        })
        .next()
        .is_none_or(|word| word.as_bytes()[0].is_ascii_uppercase())
}

fn is_minor_title_word(word: &str) -> bool {
    matches!(
        word,
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
    )
}

#[derive(Debug, Clone, Copy)]
pub struct DiagnosticMetadata {
    pub severity: DiagnosticSeverity,
    pub kind: DiagnosticKind,
    pub title_family: &'static str,
    pub development_only: bool,
}

pub use doria_diagnostic_catalogue::{
    runtime_entry as runtime_catalogue_entry, RuntimeCatalogueEntry,
    DIAGNOSTIC_CODES as CATALOGUED_CODES, RUNTIME_CATALOGUE,
};
pub fn catalogue_metadata(code: &str) -> DiagnosticMetadata {
    let runtime = code.starts_with("P1");
    let kind = match code.as_bytes().first().copied() {
        Some(b'B') => DiagnosticKind::Backend,
        Some(b'I') => DiagnosticKind::InternalCompiler,
        Some(b'P') if runtime => DiagnosticKind::RuntimePanic,
        _ => DiagnosticKind::Language,
    };
    DiagnosticMetadata {
        severity: DiagnosticSeverity::Error,
        kind,
        title_family: match code.as_bytes().first().copied() {
            Some(b'L') => "Lexical Error",
            Some(b'P') if runtime => "Runtime Panic",
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
        assert!(is_title_case(
            "`when`, `given`, and Control-flow `finally` Are Accepted Syntax"
        ));
        assert!(!is_title_case("`when` is not available"));
        assert!(!is_title_case("Unclosed `syntax"));
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
        assert!(rendered.contains("unicode.doria · line 1"));
        assert!(rendered.contains("…"));
        assert!(rendered.contains("second $value"));
        assert_eq!(rendered.lines().filter(|line| *line == "Help").count(), 2);
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
    fn backend_failures_advertise_protected_developer_details() {
        let source = SourceFile::new("main.doria", "");
        let rendered = render_diagnostics(
            &source,
            &[Diagnostic::new("B0001", "linker failed", Span::default())
                .with_developer_details("complete linker output")],
            RenderOptions {
                color: ColorChoice::Never,
                ..RenderOptions::default()
            },
        );
        assert!(rendered.contains("\n\nNote\nDeveloper details are available in JSON diagnostics."));
        assert!(!rendered.contains("complete linker output"));
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
