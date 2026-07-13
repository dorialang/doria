use crate::source::{SourceFile, Span};

pub type DiagnosticResult<T> = Result<T, Vec<Diagnostic>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixIt {
    pub span: Span,
    pub replacement: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: &'static str,
    pub message: String,
    pub span: Span,
    pub help: Option<String>,
    pub fix: Option<FixIt>,
}

impl Diagnostic {
    pub fn new(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        Self {
            code,
            message: message.into(),
            span,
            help: None,
            fix: None,
        }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn with_fix(mut self, span: Span, replacement: impl Into<String>) -> Self {
        self.fix = Some(FixIt {
            span,
            replacement: replacement.into(),
        });
        self
    }

    pub fn render(&self, source: &SourceFile) -> String {
        let (line, col) = source.line_col(self.span.start);
        let line_text = source.line_text(line);
        let width = self.span.end.saturating_sub(self.span.start).max(1);
        let marker = format!(
            "{}{}",
            " ".repeat(col.saturating_sub(1)),
            "^".repeat(width.min(80))
        );

        let mut rendered = format!(
            "Error[{}]: {}\n  --> {}:{}:{}\n   |\n{:>3} | {}\n   | {}",
            self.code, self.message, source.path, line, col, line, line_text, marker
        );

        if let Some(help) = &self.help {
            rendered.push_str("\nHelp: ");
            rendered.push_str(help);
        }

        rendered
    }
}
