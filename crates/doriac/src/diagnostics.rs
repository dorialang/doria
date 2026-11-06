use std::collections::HashSet;
use std::io::IsTerminal;
use std::ops::{Deref, DerefMut};

use doria_diagnostic_catalogue::source_excerpt::{self, SourceExcerpt};
use unicode_width::UnicodeWidthStr;

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
    CompilerInput,
    UnsupportedDevelopmentSurface,
    Backend,
    ExternalTool,
    InternalCompiler,
    RuntimePanic,
    RuntimeError,
    RuntimeAssertion,
}

impl DiagnosticKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Language => "language",
            Self::CompilerInput => "compilerInput",
            Self::UnsupportedDevelopmentSurface => "unsupportedDevelopmentSurface",
            Self::Backend => "backend",
            Self::ExternalTool => "externalTool",
            Self::InternalCompiler => "internalCompiler",
            Self::RuntimePanic => "runtimePanic",
            Self::RuntimeError => "runtimeError",
            Self::RuntimeAssertion => "runtimeAssertion",
        }
    }

    fn title(self, severity: DiagnosticSeverity) -> &'static str {
        match self {
            Self::RuntimePanic => "Panic",
            Self::RuntimeError => "Error",
            Self::RuntimeAssertion => "Error",
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
    pub error_type: Option<String>,
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
    Unavailable,
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
        let explanation = if code == "P1501" {
            outcome
                .facts
                .iter()
                .find(|fact| {
                    fact.name == doria_diagnostic_catalogue::SHARED_ACCESS_CONFLICT_REASON_FACT
                })
                .and_then(|fact| match &fact.value {
                    RuntimeFactValue::StaticString(value)
                        if doria_diagnostic_catalogue::is_shared_access_conflict_reason(value) =>
                    {
                        Some(value.as_str())
                    }
                    _ => None,
                })
                .unwrap_or(entry.explanation)
        } else {
            entry.explanation
        };
        let mut diagnostic = Self::build(
            code,
            entry.title.to_string(),
            span,
            DiagnosticKind::RuntimePanic,
        )
        .with_title(entry.title)
        .with_primary_label(entry.primary_label)
        .with_explanation(explanation);
        diagnostic.runtime_outcome = Some(outcome);
        diagnostic
    }

    pub fn runtime_error(
        error_type: impl Into<String>,
        message: impl Into<String>,
        span: Span,
        outcome: RuntimeOutcomeDetails,
    ) -> Self {
        let error_type = error_type.into();
        let message = message.into();
        let mut diagnostic =
            Self::build("R1000", message.clone(), span, DiagnosticKind::RuntimeError)
                .with_title(format!("Unhandled {error_type}"))
                .with_primary_label("This Error Was First Thrown Here")
                .with_explanation(message);
        diagnostic.runtime_outcome = Some(outcome);
        diagnostic
    }

    pub fn runtime_assertion(
        message: impl Into<String>,
        span: Span,
        outcome: RuntimeOutcomeDetails,
    ) -> Self {
        let message = message.into();
        let mut diagnostic = Self::build(
            "R1001",
            message.clone(),
            span,
            DiagnosticKind::RuntimeAssertion,
        )
        .with_title("Assertion Failed")
        .with_primary_label("Assertion Failed Here")
        .with_explanation(message);
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
            DiagnosticKind::RuntimeError => "Runtime Error".to_string(),
            DiagnosticKind::RuntimeAssertion => "Assertion Failed".to_string(),
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
                explanation: match (code, kind) {
                    // A runtime mismatch is not a code-generation failure, and
                    // saying so sends the reader hunting for a compiler bug
                    // instead of at the archive that was selected.
                    ("B0003", _) => Some(
                        "The program compiled, but the runtime library selected for linking was not built for this compiler. Linking it would produce an executable this compiler cannot vouch for."
                            .to_string(),
                    ),
                    (_, DiagnosticKind::InternalCompiler) => Some(
                        "The compiler reached an unexpected internal state. This is a compiler defect, not a problem with your program."
                            .to_string(),
                    ),
                    (_, DiagnosticKind::Backend | DiagnosticKind::ExternalTool) => Some(
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

    pub fn with_primary_source(mut self, source: DiagnosticSource) -> Self {
        if let Some(primary) = self
            .labels
            .iter_mut()
            .find(|label| label.role == LabelRole::Primary)
        {
            primary.source = source;
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
            terminal_width: source_excerpt::DEFAULT_TERMINAL_WIDTH,
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
    render_prepared(Presentation::Single(source), &prepared, options)
}

pub fn render_diagnostics_with_source_map(
    sources: &crate::source_map::SourceMap,
    diagnostics: &[Diagnostic],
    options: RenderOptions,
) -> String {
    let prepared = prepare_diagnostics_with_source_map(sources, diagnostics);
    render_prepared(Presentation::Map(sources), &prepared, options)
}

fn render_prepared(
    presentation: Presentation<'_>,
    prepared: &[Diagnostic],
    options: RenderOptions,
) -> String {
    let prepared = prepared.iter().collect::<Vec<_>>();
    match options.format {
        DiagnosticFormat::Human => render_human(presentation, &prepared, options),
        DiagnosticFormat::Concise => render_concise(presentation, &prepared),
        DiagnosticFormat::Json => diagnostics_json(presentation, &prepared),
    }
}

/// The sources a prepared diagnostic is presented against.
#[derive(Clone, Copy)]
enum Presentation<'a> {
    /// One source; `DiagnosticSource::Current` is this source.
    Single(&'a SourceFile),
    /// Every source of a graph; prepared locations name their source.
    Map(&'a crate::source_map::SourceMap),
}

impl<'a> Presentation<'a> {
    /// The text a location is shown from, when a reader can open its source.
    /// A location is never shown from a different source's text.
    fn file(self, identity: &DiagnosticSource, span: Span) -> Option<&'a SourceFile> {
        match (self, identity) {
            (_, DiagnosticSource::Unavailable) => None,
            (Self::Single(source), DiagnosticSource::Current) => Some(source),
            (Self::Single(source), DiagnosticSource::Path(path)) => {
                (path == &source.path).then_some(source)
            }
            (Self::Map(sources), DiagnosticSource::Current) => {
                sources.by_id(span.source).map(|record| &record.source)
            }
            (Self::Map(sources), DiagnosticSource::Path(path)) => {
                sources.by_path(path).map(|record| &record.source)
            }
        }
    }

    fn name<'b>(self, identity: &'b DiagnosticSource, span: Span) -> &'b str
    where
        'a: 'b,
    {
        match identity {
            DiagnosticSource::Path(path) => path,
            DiagnosticSource::Current => self
                .file(identity, span)
                .map_or(UNAVAILABLE_SOURCE_NAME, |source| source.path.as_str()),
            DiagnosticSource::Unavailable => UNAVAILABLE_SOURCE_NAME,
        }
    }
}

const UNAVAILABLE_SOURCE_NAME: &str = "<source unavailable>";

fn render_human(
    presentation: Presentation<'_>,
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
                (
                    DiagnosticKind::RuntimePanic
                    | DiagnosticKind::RuntimeError
                    | DiagnosticKind::RuntimeAssertion,
                    _,
                ) => "31;1",
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
                presentation.name(&label.source, label.span).to_string(),
                label.span.start,
                label.role != LabelRole::Primary,
            )
        });
        if let Some(primary) = labels.iter().find(|label| label.role == LabelRole::Primary) {
            block.push_str("\n\nWhere\n");
            render_label(
                &mut block,
                presentation.name(&primary.source, primary.span),
                presentation.file(&primary.source, primary.span),
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
                render_label(
                    &mut block,
                    presentation.name(&label.source, label.span),
                    presentation.file(&label.source, label.span),
                    label,
                    options,
                    None,
                );
            }
        }
        if diagnostic.kind == DiagnosticKind::RuntimeAssertion {
            render_assertion_values(&mut block, diagnostic);
        }
        if diagnostic.kind == DiagnosticKind::RuntimeError {
            render_io_progress(&mut block, diagnostic);
        }
        if let Some(explanation) = &diagnostic.explanation {
            if matches!(
                diagnostic.kind,
                DiagnosticKind::RuntimeError | DiagnosticKind::RuntimeAssertion
            ) {
                block.push_str("\n\nWhy\n  ");
                block.push_str(&terminal_safe_runtime_text(
                    explanation,
                    doria_diagnostic_catalogue::RuntimeTextPresentation::Human,
                ));
            } else {
                push_prose_section(&mut block, "Why", explanation, options.terminal_width, true);
            }
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
                let location = display_location(presentation, &edit.source, edit.span);
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
                    let name = presentation.name(&frame.source, frame.span);
                    match presentation.file(&frame.source, frame.span) {
                        Some(frame_source) => {
                            let (line, _) = display_line_col(
                                frame_source,
                                frame.span.start,
                                source_excerpt::TAB_WIDTH,
                            );
                            block.push_str(&format!("\n{} · {name}:{line}", frame.function));
                        }
                        None => block.push_str(&format!("\n{} · {name}", frame.function)),
                    }
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

/// Renders a label's location header and, when its source text is available,
/// the excerpt. Without the text, only the source name and message are shown.
fn render_label(
    rendered: &mut String,
    name: &str,
    source: Option<&SourceFile>,
    label: &DiagnosticLabel,
    options: RenderOptions,
    context: Option<&str>,
) {
    rendered.push_str(name);
    let Some(source) = source else {
        if let Some(context) = context {
            rendered.push_str(" · ");
            rendered.push_str(context);
        }
        rendered.push('\n');
        if !label.message.is_empty() {
            rendered.push_str(&label.message);
        }
        return;
    };
    let (line, _) = display_line_col(source, label.span.start, source_excerpt::TAB_WIDTH);
    rendered.push_str(&format!(" · line {line}"));
    if let Some(context) = context {
        rendered.push_str(" · ");
        rendered.push_str(context);
    }
    rendered.push('\n');

    let (end_line, _) = source.line_col(label.span.end.min(source.text.len()));
    let first_line = line.saturating_sub(options.context_lines).max(1);
    let last_line = (end_line + options.context_lines).min(source.line_count());
    let gutter_width = last_line
        .to_string()
        .len()
        .max(source_excerpt::MINIMUM_GUTTER_WIDTH);
    let gutter_gap = source_excerpt::GUTTER_GAP;
    for context_line in first_line..=last_line {
        let raw_line = source.line_text(context_line);
        let line_start = source.line_start(context_line);
        let line_end = line_start.saturating_add(raw_line.len());
        let is_labelled = context_line >= line
            && context_line <= end_line
            && label.span.start <= line_end
            && label.span.end >= line_start;
        let marker_start_byte = label.span.start.max(line_start).min(line_end);
        let marker_end_byte = label.span.end.min(line_end).max(marker_start_byte);
        let excerpt = SourceExcerpt::new(
            raw_line,
            is_labelled.then_some((marker_start_byte - line_start, marker_end_byte - line_start)),
            options.terminal_width,
            doria_unicode::grapheme_boundaries,
            UnicodeWidthStr::width,
        );
        if excerpt.is_empty() {
            rendered.push_str(&format!("\n{:>gutter_width$}", context_line));
        } else {
            rendered.push_str(&format!("\n{:>gutter_width$}{gutter_gap}", context_line));
            excerpt.write(|bytes| {
                rendered.push_str(std::str::from_utf8(bytes).expect("source excerpt is UTF-8"))
            });
        }
        if is_labelled {
            let marker = if label.role == LabelRole::Primary {
                '^'
            } else {
                '-'
            };
            rendered.push_str(&format!(
                "\n{:>gutter_width$}{gutter_gap}{}{}",
                "",
                " ".repeat(excerpt.marker_offset),
                marker.to_string().repeat(excerpt.marker_width)
            ));
            if context_line == line && !label.message.is_empty() {
                rendered.push_str(&format!(
                    "\n{:>gutter_width$}{gutter_gap}{}",
                    "",
                    " ".repeat(excerpt.marker_offset)
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

fn render_concise(presentation: Presentation<'_>, diagnostics: &[&Diagnostic]) -> String {
    let mut lines = diagnostics
        .iter()
        .map(|diagnostic| {
            let primary = diagnostic
                .labels
                .iter()
                .find(|label| label.role == LabelRole::Primary)
                .or_else(|| diagnostic.labels.first());
            let location = match primary {
                Some(label) => display_location(presentation, &label.source, label.span),
                None => match presentation {
                    Presentation::Single(source) => format!("{}:1:1", source.path),
                    Presentation::Map(_) => UNAVAILABLE_SOURCE_NAME.to_string(),
                },
            };
            format!(
                "{location}: {}[{}]: {}{}",
                diagnostic.kind.title(diagnostic.severity),
                diagnostic.code,
                if matches!(
                    diagnostic.kind,
                    DiagnosticKind::RuntimeError | DiagnosticKind::RuntimeAssertion
                ) {
                    terminal_safe_runtime_text(
                        &diagnostic.title,
                        doria_diagnostic_catalogue::RuntimeTextPresentation::Concise,
                    )
                } else {
                    diagnostic.title.clone()
                },
                diagnostic
                    .runtime_outcome
                    .as_ref()
                    .map_or_else(String::new, |outcome| {
                        format!(" (status {})", outcome.process_status)
                    })
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

fn render_io_progress(rendered: &mut String, diagnostic: &Diagnostic) {
    let Some(outcome) = &diagnostic.runtime_outcome else {
        return;
    };
    if outcome.error_type.as_deref() != Some(crate::compiler_known_io::IO_ERROR) {
        return;
    }
    let count = outcome.facts.iter().find_map(|fact| {
        if fact.name == doria_diagnostic_catalogue::IO_TRANSFERRED_BYTE_COUNT_FACT {
            match fact.value {
                RuntimeFactValue::Signed(value) => Some(value),
                _ => None,
            }
        } else {
            None
        }
    });
    if let Some(count) = count {
        rendered.push_str(&format!(
            "\n\n{}\n  {count}",
            doria_diagnostic_catalogue::IO_TRANSFERRED_BYTE_COUNT_HEADING
        ));
    }
}

fn render_assertion_values(rendered: &mut String, diagnostic: &Diagnostic) {
    let Some(outcome) = &diagnostic.runtime_outcome else {
        return;
    };
    let boolean = |name: &str| {
        outcome
            .facts
            .iter()
            .find(|fact| fact.name == name)
            .and_then(|fact| match fact.value {
                RuntimeFactValue::Boolean(value) => Some(value),
                _ => None,
            })
    };
    let text = |name: &str| {
        outcome
            .facts
            .iter()
            .find(|fact| fact.name == name)
            .and_then(|fact| match &fact.value {
                RuntimeFactValue::StaticString(value) => Some(value.as_str()),
                _ => None,
            })
    };
    if boolean(doria_diagnostic_catalogue::ASSERTION_EXPECTED_PRESENT_FACT) == Some(true) {
        if let Some(value) = text(doria_diagnostic_catalogue::ASSERTION_EXPECTED_PRESENTATION_FACT)
        {
            rendered.push_str("\n\nExpected\n  ");
            rendered.push_str(&terminal_safe_runtime_text(
                value,
                doria_diagnostic_catalogue::RuntimeTextPresentation::Human,
            ));
        }
    }
    if boolean(doria_diagnostic_catalogue::ASSERTION_ACTUAL_PRESENT_FACT) == Some(true) {
        if let Some(value) = text(doria_diagnostic_catalogue::ASSERTION_ACTUAL_PRESENTATION_FACT) {
            rendered.push_str("\n\nActual\n  ");
            rendered.push_str(&terminal_safe_runtime_text(
                value,
                doria_diagnostic_catalogue::RuntimeTextPresentation::Human,
            ));
        }
    }
    if boolean(doria_diagnostic_catalogue::ASSERTION_DIFFERENCE_PRESENT_FACT) == Some(true) {
        if let Some(value) = text(doria_diagnostic_catalogue::ASSERTION_DIFFERENCE_FACT) {
            rendered.push_str("\n\nDifference\n  ");
            rendered.push_str(&terminal_safe_runtime_text(
                value,
                doria_diagnostic_catalogue::RuntimeTextPresentation::Human,
            ));
        }
    }
}

fn terminal_safe_runtime_text(
    value: &str,
    presentation: doria_diagnostic_catalogue::RuntimeTextPresentation,
) -> String {
    let mut bytes = Vec::with_capacity(value.len());
    doria_diagnostic_catalogue::write_terminal_safe_runtime_text(
        value.as_bytes(),
        presentation,
        |chunk| bytes.extend_from_slice(chunk),
    );
    String::from_utf8(bytes).expect("terminal-safe runtime text remains valid UTF-8")
}

fn diagnostics_json(presentation: Presentation<'_>, diagnostics: &[&Diagnostic]) -> String {
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
                    serde_json::json!({
                        "source": presentation.name(&label.source, label.span),
                        "role": label.role.as_str(),
                        "message": label.message,
                        "span": {
                            "start": label.span.start,
                            "end": label.span.end,
                        },
                        "range": json_range(presentation.file(&label.source, label.span), label.span),
                    })
                }).collect::<Vec<_>>(),
                "notes": diagnostic.notes,
                "help": diagnostic.helps,
                "fixes": diagnostic.fixes.iter().map(|fix| serde_json::json!({
                    "title": fix.title,
                    "applicability": fix.applicability.as_str(),
                    "edits": fix.edits.iter().map(|edit| {
                        serde_json::json!({
                            "source": presentation.name(&edit.source, edit.span),
                            "span": { "start": edit.span.start, "end": edit.span.end },
                            "replacement": edit.replacement,
                        })
                    }).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
                "causeId": diagnostic.cause_id,
                "developmentOnly": diagnostic.development_only,
                "documentation": diagnostic.documentation.as_ref().map(|docs| serde_json::json!({
                    "slug": docs.slug,
                    "url": docs.url,
                })),
                "developerDetails": diagnostic.developer_details,
                "errorType": diagnostic.runtime_outcome.as_ref().and_then(|outcome| outcome.error_type.as_ref()),
                "runtimeOutcome": diagnostic.runtime_outcome.as_ref().map(|outcome| {
                    serde_json::json!({
                        "processStatus": outcome.process_status,
                        "terminationBehavior": outcome.termination_behavior.as_str(),
                        "origin": runtime_origin_json(presentation, &outcome.origin),
                        "pathKind": if outcome.path.is_empty() { "none" } else { "callPath" },
                        "frames": outcome.path.iter().map(|frame| {
                            let position = presentation.file(&frame.source, frame.span).map(|frame_source| {
                                display_line_col(frame_source, frame.span.start, source_excerpt::TAB_WIDTH)
                            });
                            serde_json::json!({
                                "function": frame.function,
                                "source": presentation.name(&frame.source, frame.span),
                                "span": {
                                    "start": frame.span.start,
                                    "end": frame.span.end,
                                },
                                "line": position.map(|(line, _)| line),
                                "column": position.map(|(_, column)| column),
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

fn runtime_origin_json(
    presentation: Presentation<'_>,
    origin: &RuntimeOutcomeOrigin,
) -> serde_json::Value {
    serde_json::json!({
        "source": presentation.name(&origin.source, origin.span),
        "function": origin.function,
        "span": {
            "start": origin.span.start,
            "end": origin.span.end,
        },
        "range": json_range(presentation.file(&origin.source, origin.span), origin.span),
    })
}

/// Line and column range of a location, or `null` when its source text is
/// unavailable. A position is never computed against another source's text.
fn json_range(source: Option<&SourceFile>, span: Span) -> serde_json::Value {
    let Some(source) = source else {
        return serde_json::Value::Null;
    };
    let (line, column) = display_line_col(source, span.start, source_excerpt::TAB_WIDTH);
    let (end_line, end_column) = display_line_col(source, span.end, source_excerpt::TAB_WIDTH);
    serde_json::json!({
        "start": { "line": line, "column": column },
        "end": { "line": end_line, "column": end_column },
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

/// Prepares diagnostics presented against one source: location resolution,
/// then presentation-independent duplicate and explicit cause grouping.
///
/// Every consumer (CLI, language server, Playground JSON) must prepare through
/// this or [`prepare_diagnostics_with_source_map`] rather than implementing its
/// own fuzzy or protocol-specific suppression. Use this one only when every
/// authored location is in the presented source; multi-source presentation
/// must prepare with its source map.
pub fn prepare_diagnostics(diagnostics: &[Diagnostic]) -> Vec<Diagnostic> {
    prepare(diagnostics, None)
}

/// Prepares diagnostics presented against a source map. Every location then
/// names the source its span is in, whichever compiler phase produced it.
pub fn prepare_diagnostics_with_source_map(
    sources: &crate::source_map::SourceMap,
    diagnostics: &[Diagnostic],
) -> Vec<Diagnostic> {
    prepare(diagnostics, Some(sources))
}

/// Resolves locations in place for a result published with its source map,
/// using the same rule as preparation.
pub(crate) fn resolve_diagnostic_sources(
    sources: &crate::source_map::SourceMap,
    diagnostics: &mut [Diagnostic],
) {
    for diagnostic in diagnostics {
        resolve_locations(diagnostic, Some(sources));
    }
}

/// Gives every location in `diagnostic` the identity of the source its span
/// is in.
///
/// Builders record `DiagnosticSource::Current` because a diagnostic is built
/// without the sources it will be presented with. With a source map, `Current`
/// becomes the path of the span's source, or `Unavailable` when the map does
/// not hold it. Without one, `Current` remains the single presented source.
///
/// A compiler-owned source is embedded in the compiler and no reader can open
/// it, so its locations never reach a presentation: a secondary label in one is
/// removed, a fix that edits one is removed, and a primary or runtime location
/// keeps its message with an unavailable source. Producers report such sites
/// at the authored location that reached them (Stage 36a decision log entry
/// 30); this is the boundary that guarantees it for every producer.
fn resolve_locations(diagnostic: &mut Diagnostic, sources: Option<&crate::source_map::SourceMap>) {
    let identity = |source: &DiagnosticSource, span: Span| {
        if span.source.is_compiler_owned() {
            return DiagnosticSource::Unavailable;
        }
        match (source, sources) {
            (DiagnosticSource::Current, Some(sources)) => sources
                .by_id(span.source)
                .map_or(DiagnosticSource::Unavailable, |record| {
                    DiagnosticSource::Path(record.display_path.clone())
                }),
            _ => source.clone(),
        }
    };
    let details = &mut *diagnostic.details;
    details
        .labels
        .retain(|label| label.role == LabelRole::Primary || !label.span.source.is_compiler_owned());
    details
        .related
        .retain(|related| !related.span.source.is_compiler_owned());
    details.fixes.retain(|fix| {
        fix.edits
            .iter()
            .all(|edit| !edit.span.source.is_compiler_owned())
    });
    if details
        .fix
        .as_ref()
        .is_some_and(|fix| fix.span.source.is_compiler_owned())
    {
        details.fix = None;
    }
    for label in &mut details.labels {
        label.source = identity(&label.source, label.span);
    }
    for edit in details.fixes.iter_mut().flat_map(|fix| &mut fix.edits) {
        edit.source = identity(&edit.source, edit.span);
    }
    if let Some(outcome) = &mut details.runtime_outcome {
        outcome.origin.source = identity(&outcome.origin.source, outcome.origin.span);
        for frame in &mut outcome.path {
            frame.source = identity(&frame.source, frame.span);
        }
    }
}

fn prepare(
    diagnostics: &[Diagnostic],
    sources: Option<&crate::source_map::SourceMap>,
) -> Vec<Diagnostic> {
    let mut seen = HashSet::new();
    let mut prepared = diagnostics
        .iter()
        .cloned()
        .map(|mut diagnostic| {
            resolve_locations(&mut diagnostic, sources);
            diagnostic
        })
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
                span.source,
                span.start,
                span.end,
                diagnostic.title.clone(),
                diagnostic.cause_id.clone(),
            ))
        })
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

fn display_location(
    presentation: Presentation<'_>,
    identity: &DiagnosticSource,
    span: Span,
) -> String {
    let name = presentation.name(identity, span);
    match (presentation.file(identity, span), identity) {
        (Some(source), _) => {
            let (line, column) = display_line_col(source, span.start, source_excerpt::TAB_WIDTH);
            format!("{name}:{line}:{column}")
        }
        (None, DiagnosticSource::Path(_)) => format!("{name} at byte {}", span.start),
        (None, _) => name.to_string(),
    }
}

fn display_line_col(source: &SourceFile, byte_index: usize, tab_width: usize) -> (usize, usize) {
    let safe_index = byte_index.min(source.text.len());
    let (line, _) = source.line_col(safe_index);
    let line_start = source.line_start(line);
    let prefix = source.text.get(line_start..safe_index).unwrap_or("");
    (
        line,
        source_excerpt::display_width_with_tabs(
            prefix,
            tab_width,
            doria_unicode::grapheme_boundaries,
            UnicodeWidthStr::width,
        ) + 1,
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
        // A few diagnostics name a specific failure the family title cannot
        // convey. A runtime artifact mismatch reported as "Backend Error" sends
        // the reader looking for a code-generation bug instead of at the
        // archive that was selected, which is the one fact that explains it.
        title_family: match code {
            "B0003" => "Runtime Artifact Is Incompatible With This Compiler",
            "B3602" => "PHP Native Runtime Dependency Is Unavailable",
            _ => match code.as_bytes().first().copied() {
                Some(b'L') => "Lexical Error",
                Some(b'P') if runtime => "Runtime Panic",
                Some(b'P') => "Syntax Error",
                Some(b'E') => "Language Error",
                Some(b'M') => "MIR Error",
                Some(b'B') => "Backend Error",
                Some(b'I') => "Internal Compiler Error",
                _ => "Compiler Diagnostic",
            },
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
        contains_development_stage_reference, display_line_col, is_title_case, render_diagnostics,
        render_label, ColorChoice, Diagnostic, DiagnosticFormat, DiagnosticKind,
        DiagnosticSeverity, DiagnosticSource, FixApplicability, FixEdit, LabelRole, RenderOptions,
        RuntimeFact, RuntimeFactValue, RuntimeOutcomeDetails, RuntimeOutcomeOrigin,
        TerminationBehavior,
    };
    use crate::source::{SourceFile, Span};
    use doria_diagnostic_catalogue::source_excerpt::{self, SourceExcerpt};
    use unicode_width::UnicodeWidthStr;

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
    fn runtime_error_messages_are_safe_in_terminals_and_exact_in_json() {
        let source = SourceFile::new(
            "main.doria",
            "function main(): void\n{\n    throw new Failure(\"unsafe\");\n}\n",
        );
        let start = source.text.find("throw").expect("throw site should exist");
        let span = Span::new(start, start + "throw new Failure(\"unsafe\");".len());
        let message = "first\nWhere\nWhy\r\u{1b}[31m\0\tlast";
        let diagnostic = Diagnostic::runtime_error(
            "Failure",
            message,
            span,
            RuntimeOutcomeDetails {
                process_status: 70,
                termination_behavior: TerminationBehavior::PropagateWithCleanup,
                origin: RuntimeOutcomeOrigin {
                    source: DiagnosticSource::Current,
                    span,
                    function: Some("main".to_string()),
                },
                path: Vec::new(),
                facts: Vec::new(),
                error_type: Some("Failure".to_string()),
            },
        );

        let human = render_diagnostics(
            &source,
            std::slice::from_ref(&diagnostic),
            RenderOptions {
                color: ColorChoice::Never,
                ..RenderOptions::default()
            },
        );
        assert!(human.starts_with("Error[R1000]: Unhandled Failure\n\nWhere\n"));
        assert!(human.contains("\n\nWhy\n  first\n  Where\n  Why\\r\\u001b[31m\\u0000\\tlast"));
        assert!(human.ends_with("\n\nProcess Exited With Status 70"));
        assert!(!human.contains("\u{1b}[31m"));
        assert!(!human.contains("Call Path"));

        let concise = render_diagnostics(
            &source,
            std::slice::from_ref(&diagnostic),
            RenderOptions {
                format: DiagnosticFormat::Concise,
                color: ColorChoice::Never,
                ..RenderOptions::default()
            },
        );
        assert_eq!(concise.lines().count(), 1);
        assert!(concise.ends_with("Error[R1000]: Unhandled Failure (status 70)"));

        let json = render_diagnostics(
            &source,
            &[diagnostic],
            RenderOptions {
                format: DiagnosticFormat::Json,
                color: ColorChoice::Never,
                ..RenderOptions::default()
            },
        );
        let json: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let value = &json["diagnostics"][0];
        assert_eq!(value["code"], "R1000");
        assert_eq!(value["kind"], "runtimeError");
        assert_eq!(value["message"], message);
        assert_eq!(value["errorType"], "Failure");
        assert_eq!(value["runtimeOutcome"]["processStatus"], 70);
        assert_eq!(
            value["runtimeOutcome"]["terminationBehavior"],
            "propagateWithCleanup"
        );
        assert_eq!(value["runtimeOutcome"]["pathKind"], "none");
        assert_eq!(value["runtimeOutcome"]["frames"], serde_json::json!([]));
    }

    #[test]
    fn io_error_progress_remains_optional_in_human_and_json_diagnostics() {
        let source = SourceFile::new("main.doria", "function main(): void {}\n");
        for count in [None, Some(0), Some(2048)] {
            let diagnostic = Diagnostic::runtime_error(
                crate::compiler_known_io::IO_ERROR,
                "failed transfer",
                Span::default(),
                RuntimeOutcomeDetails {
                    process_status: 70,
                    termination_behavior: TerminationBehavior::PropagateWithCleanup,
                    origin: RuntimeOutcomeOrigin {
                        source: DiagnosticSource::Unavailable,
                        span: Span::default(),
                        function: None,
                    },
                    path: Vec::new(),
                    facts: count
                        .map(|value| RuntimeFact {
                            name: doria_diagnostic_catalogue::IO_TRANSFERRED_BYTE_COUNT_FACT
                                .to_string(),
                            value: RuntimeFactValue::Signed(value),
                        })
                        .into_iter()
                        .collect(),
                    error_type: Some(crate::compiler_known_io::IO_ERROR.to_string()),
                },
            );
            let human = render_diagnostics(
                &source,
                std::slice::from_ref(&diagnostic),
                RenderOptions {
                    color: ColorChoice::Never,
                    ..RenderOptions::default()
                },
            );
            if let Some(value) = count {
                assert!(human.contains(&format!("Transferred Byte Count\n  {value}")));
            } else {
                assert!(!human.contains("Transferred Byte Count"));
            }
            let json = render_diagnostics(
                &source,
                &[diagnostic],
                RenderOptions {
                    format: DiagnosticFormat::Json,
                    ..RenderOptions::default()
                },
            );
            let json: serde_json::Value = serde_json::from_str(&json).unwrap();
            let facts = json["diagnostics"][0]["runtimeOutcome"]["facts"]
                .as_array()
                .unwrap();
            if let Some(value) = count {
                assert_eq!(facts.len(), 1);
                assert_eq!(facts[0]["name"], "transferredByteCount");
                assert_eq!(facts[0]["kind"], "signedInteger");
                assert_eq!(facts[0]["value"], value);
            } else {
                assert!(facts.is_empty());
            }
        }
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
    fn source_labels_share_default_windows_and_bounded_late_markers() {
        let text = "0123456789".repeat(20);
        let source = SourceFile::new("long.doria", &text);
        let diagnostic = Diagnostic::new("E0403", "test", Span::new(170, 200))
            .with_primary_label("Selected Here");
        let mut rendered = String::new();
        render_label(
            &mut rendered,
            &source.path,
            Some(&source),
            &diagnostic.labels[0],
            RenderOptions {
                context_lines: 0,
                ..RenderOptions::default()
            },
            None,
        );
        assert_eq!(
            rendered,
            format!(
                "long.doria · line 1\n\n  1      …{}\n         {}{}\n         {}Selected Here",
                &text[114..],
                " ".repeat(57),
                "^".repeat(30),
                " ".repeat(57),
            )
        );
    }

    #[test]
    fn source_labels_align_tabs_graphemes_multiline_and_empty_lines() {
        let text = "a\t👩‍💻\te\u{301}界\n\n\tend";
        let source = SourceFile::new("unicode.doria", text);
        let diagnostic = Diagnostic::new("E0403", "test", Span::new(2, text.len()))
            .with_primary_label("Selected Here");
        let mut rendered = String::new();
        render_label(
            &mut rendered,
            &source.path,
            Some(&source),
            &diagnostic.labels[0],
            RenderOptions {
                context_lines: 0,
                ..RenderOptions::default()
            },
            None,
        );
        assert_eq!(
            rendered,
            concat!(
                "unicode.doria · line 1\n\n",
                "  1      a   👩‍💻  e\u{301}界\n",
                "             ^^^^^^^\n",
                "             Selected Here\n",
                "  2\n",
                "         ^\n",
                "  3          end\n",
                "         ^^^^^^^",
            )
        );
        assert_eq!(
            display_line_col(&source, text.find('e').unwrap(), source_excerpt::TAB_WIDTH),
            (1, 9)
        );

        let narrow = format!("{}👩‍💻{}", "a".repeat(30), "界".repeat(30));
        let excerpt = SourceExcerpt::new(
            &narrow,
            Some((30, 30 + "👩‍💻".len())),
            38,
            doria_unicode::grapheme_boundaries,
            UnicodeWidthStr::width,
        );
        let mut visible = String::new();
        excerpt.write(|bytes| visible.push_str(std::str::from_utf8(bytes).unwrap()));
        assert!(visible.contains("👩‍💻"));
        assert!(visible.starts_with('…') && visible.ends_with('…'));
        assert!(visible.width() <= source_excerpt::source_width(38));
        assert_eq!(excerpt.marker_width, 2);
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
