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
        let message = message.into();
        debug_assert!(
            !contains_development_stage_reference(&message),
            "user-facing diagnostic `{code}` exposes a development stage: {message}"
        );
        Self {
            code,
            message,
            span,
            help: None,
            fix: None,
        }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        let help = help.into();
        debug_assert!(
            !contains_development_stage_reference(&help),
            "user-facing help for `{}` exposes a development stage: {help}",
            self.code
        );
        self.help = Some(help);
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
    use super::contains_development_stage_reference;

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
}
