#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SourceId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Span {
    pub source: SourceId,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NameSegmentRef {
    pub text: String,
    pub span: Span,
}

/// A source-preserving Doria name. Separators are retained independently so
/// diagnostics and editor tooling never need to reconstruct authored syntax
/// from a flattened semantic name.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct QualifiedNameRef {
    pub segments: Vec<NameSegmentRef>,
    pub separator_spans: Vec<Span>,
    pub span: Span,
}

impl QualifiedNameRef {
    pub fn unqualified(text: impl Into<String>, span: Span) -> Self {
        Self {
            segments: vec![NameSegmentRef {
                text: text.into(),
                span,
            }],
            separator_spans: Vec::new(),
            span,
        }
    }

    pub fn canonical(&self) -> String {
        self.segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>()
            .join("\\")
    }

    pub fn is_qualified(&self) -> bool {
        !self.separator_spans.is_empty()
    }

    pub fn final_segment(&self) -> &NameSegmentRef {
        self.segments
            .last()
            .expect("a parsed qualified name has at least one segment")
    }
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self::in_source(SourceId::default(), start, end)
    }

    pub fn in_source(source: SourceId, start: usize, end: usize) -> Self {
        Self { source, start, end }
    }

    pub fn at(self, start: usize, end: usize) -> Self {
        Self::in_source(self.source, start, end)
    }

    pub fn merge(self, other: Span) -> Span {
        debug_assert_eq!(self.source, other.source);
        Span {
            source: self.source,
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    pub id: SourceId,
    pub path: String,
    pub text: String,
    line_starts: Vec<usize>,
}

impl SourceFile {
    pub fn new(path: impl Into<String>, text: impl Into<String>) -> Self {
        Self::with_id(SourceId::default(), path, text)
    }

    pub fn with_id(id: SourceId, path: impl Into<String>, text: impl Into<String>) -> Self {
        let text = text.into();
        let mut line_starts = vec![0];
        for (index, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(index + 1);
            }
        }

        Self {
            id,
            path: path.into(),
            text,
            line_starts,
        }
    }

    pub fn line_col(&self, byte_index: usize) -> (usize, usize) {
        let line_index = match self.line_starts.binary_search(&byte_index) {
            Ok(index) => index,
            Err(index) => index.saturating_sub(1),
        };
        let line_start = self.line_starts[line_index];
        (line_index + 1, byte_index.saturating_sub(line_start) + 1)
    }

    pub fn line_text(&self, line: usize) -> &str {
        if line == 0 || line > self.line_starts.len() {
            return "";
        }

        let start = self.line_starts[line - 1];
        let end = self
            .line_starts
            .get(line)
            .copied()
            .unwrap_or(self.text.len());
        self.text[start..end].trim_end_matches(['\n', '\r'])
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    pub fn line_start(&self, line: usize) -> usize {
        self.line_starts
            .get(line.saturating_sub(1))
            .copied()
            .unwrap_or(self.text.len())
    }
}
