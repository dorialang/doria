use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::source::{SourceFile, Span};

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StringQuoteKind {
    Single,
    Double,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Class,
    Function,
    Internal,
    Static,
    Let,
    Writable,
    Readonly,
    Return,
    Echo,
    New,
    Foreach,
    As,
    If,
    Else,
    While,
    For,
    Break,
    Continue,
    Throw,
    Throws,
    True,
    False,
    Null,
    Void,
    IntType,
    Int8Type,
    Int16Type,
    Int32Type,
    Int64Type,
    UInt8Type,
    UInt16Type,
    UInt32Type,
    UInt64Type,
    FloatType,
    Float32Type,
    Float64Type,
    StringType,
    BoolType,
    Reserved(String),
    Identifier(String),
    Variable(String),
    IntLiteral(String),
    FloatLiteral(String),
    StringLiteral {
        value: String,
        quote: StringQuoteKind,
    },
    Equals,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Dot,
    DotDot,
    DotDotLess,
    PlusPlus,
    MinusMinus,
    PlusEquals,
    MinusEquals,
    StarEquals,
    SlashEquals,
    PercentEquals,
    ShiftLeftEquals,
    ShiftRightEquals,
    AmpersandEquals,
    PipeEquals,
    CaretEquals,
    EqualEqual,
    EqualEqualEqual,
    BangEqual,
    BangEqualEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    ShiftLeft,
    ShiftRight,
    Ampersand,
    Pipe,
    Caret,
    Tilde,
    AndAnd,
    OrOr,
    Bang,
    Not,
    And,
    Or,
    Xor,
    Question,
    QuestionQuestion,
    FatArrow,
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    LeftBracket,
    RightBracket,
    Semicolon,
    Colon,
    Comma,
    Arrow,
    DoubleColon,
    Eof,
}

pub struct Lexer<'source> {
    source: &'source SourceFile,
    bytes: &'source [u8],
    index: usize,
    diagnostics: Vec<Diagnostic>,
}

impl<'source> Lexer<'source> {
    pub fn new(source: &'source SourceFile) -> Self {
        Self {
            source,
            bytes: source.text.as_bytes(),
            index: 0,
            diagnostics: Vec::new(),
        }
    }

    pub fn lex(mut self) -> DiagnosticResult<Vec<Token>> {
        let mut tokens = Vec::new();

        while !self.is_at_end() {
            self.skip_whitespace_and_comments();
            if self.is_at_end() {
                break;
            }

            let start = self.index;
            let token = match self.advance() {
                b'$' => self.lex_variable(start),
                b'a'..=b'z' | b'A'..=b'Z' | b'_' => self.lex_identifier(start),
                b'0'..=b'9' => self.lex_number(start),
                b'"' | b'\'' => self.lex_string(start),
                b'=' => {
                    if self.match_byte(b'=') {
                        if self.match_byte(b'=') {
                            self.error(
                                "Doria uses typed `==`; `===` is not supported",
                                start,
                                self.index,
                            );
                            continue;
                        } else {
                            self.token(TokenKind::EqualEqual, start)
                        }
                    } else if self.match_byte(b'>') {
                        self.token(TokenKind::FatArrow, start)
                    } else {
                        self.token(TokenKind::Equals, start)
                    }
                }
                b'+' => {
                    if self.match_byte(b'+') {
                        self.token(TokenKind::PlusPlus, start)
                    } else if self.match_byte(b'=') {
                        self.token(TokenKind::PlusEquals, start)
                    } else {
                        self.token(TokenKind::Plus, start)
                    }
                }
                b'-' => {
                    if self.match_byte(b'-') {
                        self.token(TokenKind::MinusMinus, start)
                    } else if self.match_byte(b'>') {
                        self.token(TokenKind::Arrow, start)
                    } else if self.match_byte(b'=') {
                        self.token(TokenKind::MinusEquals, start)
                    } else {
                        self.token(TokenKind::Minus, start)
                    }
                }
                b'*' => {
                    if self.match_byte(b'=') {
                        self.token(TokenKind::StarEquals, start)
                    } else {
                        self.token(TokenKind::Star, start)
                    }
                }
                b'/' => {
                    if self.match_byte(b'=') {
                        self.token(TokenKind::SlashEquals, start)
                    } else {
                        self.token(TokenKind::Slash, start)
                    }
                }
                b'%' => {
                    if self.match_byte(b'=') {
                        self.token(TokenKind::PercentEquals, start)
                    } else {
                        self.token(TokenKind::Percent, start)
                    }
                }
                b'.' => {
                    if self.match_byte(b'.') {
                        if self.match_byte(b'<') {
                            self.token(TokenKind::DotDotLess, start)
                        } else {
                            self.token(TokenKind::DotDot, start)
                        }
                    } else {
                        self.token(TokenKind::Dot, start)
                    }
                }
                b'!' => {
                    if self.match_byte(b'=') {
                        if self.match_byte(b'=') {
                            self.error(
                                "Doria uses typed `!=`; `!==` is not supported",
                                start,
                                self.index,
                            );
                            continue;
                        } else {
                            self.token(TokenKind::BangEqual, start)
                        }
                    } else {
                        self.token(TokenKind::Bang, start)
                    }
                }
                b'<' => {
                    if self.match_byte(b'<') {
                        if self.match_byte(b'=') {
                            self.token(TokenKind::ShiftLeftEquals, start)
                        } else {
                            self.token(TokenKind::ShiftLeft, start)
                        }
                    } else if self.match_byte(b'=') {
                        self.token(TokenKind::LessEqual, start)
                    } else {
                        self.token(TokenKind::Less, start)
                    }
                }
                b'>' => {
                    if self.match_byte(b'>') {
                        if self.match_byte(b'=') {
                            self.token(TokenKind::ShiftRightEquals, start)
                        } else {
                            self.token(TokenKind::ShiftRight, start)
                        }
                    } else if self.match_byte(b'=') {
                        self.token(TokenKind::GreaterEqual, start)
                    } else {
                        self.token(TokenKind::Greater, start)
                    }
                }
                b'&' => {
                    if self.match_byte(b'&') {
                        self.token(TokenKind::AndAnd, start)
                    } else if self.match_byte(b'=') {
                        self.token(TokenKind::AmpersandEquals, start)
                    } else {
                        self.token(TokenKind::Ampersand, start)
                    }
                }
                b'|' => {
                    if self.match_byte(b'|') {
                        self.token(TokenKind::OrOr, start)
                    } else if self.match_byte(b'=') {
                        self.token(TokenKind::PipeEquals, start)
                    } else {
                        self.token(TokenKind::Pipe, start)
                    }
                }
                b'^' => {
                    if self.match_byte(b'=') {
                        self.token(TokenKind::CaretEquals, start)
                    } else {
                        self.token(TokenKind::Caret, start)
                    }
                }
                b'~' => self.token(TokenKind::Tilde, start),
                b'?' => {
                    if self.match_byte(b'?') {
                        self.token(TokenKind::QuestionQuestion, start)
                    } else {
                        self.token(TokenKind::Question, start)
                    }
                }
                b'(' => self.token(TokenKind::LeftParen, start),
                b')' => self.token(TokenKind::RightParen, start),
                b'{' => self.token(TokenKind::LeftBrace, start),
                b'}' => self.token(TokenKind::RightBrace, start),
                b'[' => self.token(TokenKind::LeftBracket, start),
                b']' => self.token(TokenKind::RightBracket, start),
                b';' => self.token(TokenKind::Semicolon, start),
                b':' => {
                    if self.match_byte(b':') {
                        self.token(TokenKind::DoubleColon, start)
                    } else {
                        self.token(TokenKind::Colon, start)
                    }
                }
                b',' => self.token(TokenKind::Comma, start),
                byte => {
                    self.error(
                        format!("unexpected character `{}`", byte as char),
                        start,
                        self.index,
                    );
                    continue;
                }
            };

            tokens.push(token);
        }

        tokens.push(Token {
            kind: TokenKind::Eof,
            span: Span::new(self.index, self.index),
        });

        if self.diagnostics.is_empty() {
            Ok(tokens)
        } else {
            Err(self.diagnostics)
        }
    }

    fn lex_variable(&mut self, start: usize) -> Token {
        while matches!(
            self.peek(),
            Some(b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_')
        ) {
            self.advance();
        }

        let name = &self.source.text[start + 1..self.index];
        if name.is_empty() {
            self.error("expected variable name after `$`", start, self.index);
        }

        Token {
            kind: TokenKind::Variable(name.to_string()),
            span: Span::new(start, self.index),
        }
    }

    fn lex_identifier(&mut self, start: usize) -> Token {
        while matches!(
            self.peek(),
            Some(b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_')
        ) {
            self.advance();
        }

        let text = &self.source.text[start..self.index];
        let kind = match text {
            "class" => TokenKind::Class,
            "function" => TokenKind::Function,
            "internal" => TokenKind::Internal,
            "static" => TokenKind::Static,
            "let" => TokenKind::Let,
            "writable" => TokenKind::Writable,
            "readonly" => TokenKind::Readonly,
            "return" => TokenKind::Return,
            "echo" => TokenKind::Echo,
            "new" => TokenKind::New,
            "foreach" => TokenKind::Foreach,
            "as" => TokenKind::As,
            "if" => TokenKind::If,
            "else" => TokenKind::Else,
            "while" => TokenKind::While,
            "for" => TokenKind::For,
            "break" => TokenKind::Break,
            "continue" => TokenKind::Continue,
            "throw" => TokenKind::Throw,
            "throws" => TokenKind::Throws,
            "true" => TokenKind::True,
            "false" => TokenKind::False,
            "null" => TokenKind::Null,
            "void" => TokenKind::Void,
            "int" => TokenKind::IntType,
            "int8" => TokenKind::Int8Type,
            "int16" => TokenKind::Int16Type,
            "int32" => TokenKind::Int32Type,
            "int64" => TokenKind::Int64Type,
            "uint8" => TokenKind::UInt8Type,
            "uint16" => TokenKind::UInt16Type,
            "uint32" => TokenKind::UInt32Type,
            "uint64" => TokenKind::UInt64Type,
            "float" => TokenKind::FloatType,
            "float32" => TokenKind::Float32Type,
            "float64" => TokenKind::Float64Type,
            "string" => TokenKind::StringType,
            "bool" => TokenKind::BoolType,
            "not" => TokenKind::Not,
            "and" => TokenKind::And,
            "or" => TokenKind::Or,
            "xor" => TokenKind::Xor,
            "async" | "await" | "spawn" | "scope" | "interface" | "trait" | "enum" | "match"
            | "try" | "catch" => TokenKind::Reserved(text.to_string()),
            _ => TokenKind::Identifier(text.to_string()),
        };

        Token {
            kind,
            span: Span::new(start, self.index),
        }
    }

    fn lex_number(&mut self, start: usize) -> Token {
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.advance();
        }

        let mut is_float = false;
        if self.peek() == Some(b'.') && matches!(self.peek_next(), Some(b'0'..=b'9')) {
            is_float = true;
            self.advance();
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.advance();
            }
        }

        let value = self.source.text[start..self.index].to_string();
        Token {
            kind: if is_float {
                TokenKind::FloatLiteral(value)
            } else {
                TokenKind::IntLiteral(value)
            },
            span: Span::new(start, self.index),
        }
    }

    fn lex_string(&mut self, start: usize) -> Token {
        let quote = self.bytes[start];
        let quote_kind = if quote == 34 {
            StringQuoteKind::Double
        } else {
            StringQuoteKind::Single
        };
        let mut value = String::new();

        while let Some(byte) = self.peek() {
            if byte == quote {
                self.advance();
                return Token {
                    kind: TokenKind::StringLiteral {
                        value,
                        quote: quote_kind,
                    },
                    span: Span::new(start, self.index),
                };
            }

            if byte == b'\\' {
                self.advance();
                match self.advance() {
                    b'n' => value.push('\n'),
                    b'r' => value.push('\r'),
                    b't' => value.push('\t'),
                    b'\\' => value.push('\\'),
                    b'\'' => value.push('\''),
                    b'"' => value.push('"'),
                    other => {
                        value.push('\\');
                        value.push(other as char);
                    }
                }
            } else {
                value.push(self.advance() as char);
            }
        }

        self.error("unterminated string literal", start, self.index);
        Token {
            kind: TokenKind::StringLiteral {
                value,
                quote: quote_kind,
            },
            span: Span::new(start, self.index),
        }
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
                self.advance();
            }

            if self.peek() == Some(b'/') && self.peek_next() == Some(b'/') {
                while !matches!(self.peek(), None | Some(b'\n')) {
                    self.advance();
                }
                continue;
            }

            if self.peek() == Some(b'#') {
                while !matches!(self.peek(), None | Some(b'\n')) {
                    self.advance();
                }
                continue;
            }

            if self.peek() == Some(b'/') && self.peek_next() == Some(b'*') {
                self.advance();
                self.advance();
                while !(self.peek() == Some(b'*') && self.peek_next() == Some(b'/')) {
                    if self.is_at_end() {
                        return;
                    }
                    self.advance();
                }
                self.advance();
                self.advance();
                continue;
            }

            break;
        }
    }

    fn token(&self, kind: TokenKind, start: usize) -> Token {
        Token {
            kind,
            span: Span::new(start, self.index),
        }
    }

    fn error(&mut self, message: impl Into<String>, start: usize, end: usize) {
        self.diagnostics
            .push(Diagnostic::new("L0001", message, Span::new(start, end)));
    }

    fn advance(&mut self) -> u8 {
        let byte = self.bytes[self.index];
        self.index += 1;
        byte
    }

    fn match_byte(&mut self, expected: u8) -> bool {
        if self.peek() == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.index).copied()
    }

    fn peek_next(&self) -> Option<u8> {
        self.bytes.get(self.index + 1).copied()
    }

    fn is_at_end(&self) -> bool {
        self.index >= self.bytes.len()
    }
}
