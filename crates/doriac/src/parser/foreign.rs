//! Grammar for unsafe code and the foreign function interface (Decision
//! 0148): `declare` directives, documentation comments, `unsafe` contexts and
//! functions, `extern "C"` blocks, C-callable functions, foreign structs and
//! unions, and C function pointer types.

use super::{FunctionModifiers, Parser};
use crate::ast::*;
use crate::diagnostics::Diagnostic;
use crate::lexer::{StringQuoteKind, TokenKind};
use crate::source::Span;
use crate::string_literal::interpolation_close;
use crate::types::{CAbiTypeRef, ForeignAbi, TypeRef};

/// The text of one `/** ... */` comment in the source the tokens came from.
#[derive(Debug, Clone)]
pub(crate) struct DocCommentText {
    pub(crate) span: Span,
    pub(crate) text: String,
}

/// The documentation comments among the comment ranges a lexer reported: a
/// block comment that opens with `/**` and is not the empty `/**/`.
pub(crate) fn documentation_comments(
    source: &crate::source::SourceFile,
    comments: &[std::ops::Range<usize>],
) -> Vec<DocCommentText> {
    comments
        .iter()
        .filter_map(|range| {
            let text = source.text.get(range.clone())?;
            (text.starts_with("/**") && text != "/**/" && text.ends_with("*/")).then(|| {
                DocCommentText {
                    span: Span::in_source(source.id, range.start, range.end),
                    text: text.to_string(),
                }
            })
        })
        .collect()
}

/// Reads the sections of a documentation comment. A tag such as `@safety`
/// begins a line; its section runs to the next tag or the end of the comment.
fn documentation(comment: &DocCommentText) -> DocComment {
    let inner_start = 3;
    let inner_end = comment.text.len() - 2;
    let mut sections: Vec<(String, usize, usize, Vec<String>)> = Vec::new();
    let mut offset = inner_start;
    for line in comment.text[inner_start..inner_end].split_inclusive('\n') {
        let line_start = offset;
        offset += line.len();
        let trimmed_start = line.len() - line.trim_start().len();
        let mut content = &line[trimmed_start..];
        let mut content_start = line_start + trimmed_start;
        if let Some(rest) = content.strip_prefix('*') {
            content_start += 1;
            let spaces = rest.len() - rest.trim_start_matches([' ', '\t']).len();
            content_start += spaces;
            content = &rest[spaces..];
        }
        let content = content.trim_end();
        if let Some(tagged) = content.strip_prefix('@') {
            let tag_length = tagged
                .find(|character: char| character.is_whitespace())
                .unwrap_or(tagged.len());
            let tag = tagged[..tag_length].to_string();
            let text = tagged[tag_length..].trim().to_string();
            let end = content_start + content.len();
            sections.push((tag, content_start, end, vec![text]));
        } else if let Some((_, _, end, lines)) = sections.last_mut() {
            if !content.is_empty() {
                *end = content_start + content.len();
            }
            lines.push(content.to_string());
        }
    }
    let safety =
        sections
            .into_iter()
            .find(|(tag, ..)| tag == "safety")
            .map(|(_, start, end, lines)| DocSection {
                span: Span::in_source(
                    comment.span.source,
                    comment.span.start + start,
                    comment.span.start + end,
                ),
                text: lines
                    .iter()
                    .map(|line| line.trim())
                    .collect::<Vec<_>>()
                    .join("\n")
                    .trim()
                    .to_string(),
            });
    DocComment {
        span: comment.span,
        safety,
    }
}

impl Parser {
    pub(crate) fn with_documentation(mut self, comments: Vec<DocCommentText>) -> Self {
        self.doc_comments = comments;
        self
    }

    /// The documentation comment immediately before the token at `index`:
    /// the last comment between that token and the one before it.
    pub(super) fn documentation_before(&self, index: usize) -> Option<DocComment> {
        let token = self.tokens.get(index)?;
        let previous_end = index
            .checked_sub(1)
            .and_then(|previous| self.tokens.get(previous))
            .map_or(0, |previous| previous.span.end);
        self.doc_comments
            .iter()
            .rev()
            .find(|comment| {
                comment.span.end <= token.span.start && comment.span.start >= previous_end
            })
            .map(documentation)
    }

    /// `declare(<key>: "<value>");`, after the keyword.
    pub(super) fn parse_declare_directive(&mut self) -> Option<DeclareDirective> {
        let keyword_span = self.previous().span;
        self.expect(TokenKind::LeftParen, "expected `(` after `declare`")?;
        let key_token = self.advance().clone();
        let key_text = match &key_token.kind {
            TokenKind::Identifier(text) => Some(text.clone()),
            // A key may be spelled like a keyword, as `unsafe` is.
            kind if crate::lexer::keyword_kind(super::token_name(kind)).is_some() => {
                Some(super::token_name(kind).to_string())
            }
            _ => None,
        };
        let key = match key_text {
            Some(text) => NameRef {
                text,
                span: key_token.span,
            },
            None => {
                self.diagnostics.push(
                    Diagnostic::new(
                        "P0001",
                        "a `declare` directive names one key, as in `declare(unsafe: \"deny\");`",
                        key_token.span,
                    )
                    .with_title("Declare Key Is Missing"),
                );
                return None;
            }
        };
        if !self.check(&TokenKind::Colon) {
            let at = self.peek().span;
            let mut diagnostic = Diagnostic::new(
                "P0001",
                "a `declare` key is followed by `:` and its value",
                at,
            )
            .with_title("Declare Directive Expects A Colon");
            if matches!(self.peek().kind, TokenKind::Equals) {
                diagnostic = diagnostic.with_fix(at, ":");
            }
            self.diagnostics.push(diagnostic);
            return None;
        }
        self.advance();
        let literal = self.advance().clone();
        let TokenKind::StringLiteral { value, raw, quote } = literal.kind else {
            self.diagnostics.push(
                Diagnostic::new(
                    "P0001",
                    "a `declare` value is one string literal, as in `\"deny\"`",
                    literal.span,
                )
                .with_title("Declare Value Must Be A String Literal"),
            );
            return None;
        };
        if is_interpolated(&raw, quote) {
            self.diagnostics.push(
                Diagnostic::new(
                    "P0001",
                    "a `declare` value cannot contain string interpolation",
                    literal.span,
                )
                .with_title("Declare Value Must Be Constant"),
            );
        }
        if self.check(&TokenKind::Comma) {
            self.diagnostics.push(
                Diagnostic::new(
                    "P0001",
                    "each `declare` directive names one key; write one directive per key",
                    self.peek().span,
                )
                .with_title("Declare Directive Names One Key"),
            );
            return None;
        }
        self.expect(
            TokenKind::RightParen,
            "expected `)` after the declare value",
        )?;
        let semicolon_span = self.expect_directive_semicolon("declare directive")?;
        Some(DeclareDirective {
            keyword_span,
            key,
            value,
            value_span: literal.span,
            semicolon_span,
            span: keyword_span.merge(semicolon_span),
        })
    }

    /// The ABI string after `extern`.
    pub(super) fn parse_foreign_abi(&mut self) -> Option<ForeignAbi> {
        let token = self.peek().clone();
        let TokenKind::StringLiteral { value, raw, quote } = token.kind else {
            let insertion = self.span(token.span.start, token.span.start);
            self.diagnostics.push(
                Diagnostic::new(
                    "P0001",
                    "`extern` is followed by its ABI string, `\"C\"`",
                    insertion,
                )
                .with_title("Extern ABI String Is Missing")
                .with_fix(insertion, "\"C\" "),
            );
            return None;
        };
        self.advance();
        if is_interpolated(&raw, quote) {
            self.diagnostics.push(
                Diagnostic::new(
                    "P0001",
                    "an ABI string cannot contain string interpolation",
                    token.span,
                )
                .with_title("Extern ABI String Must Be Constant"),
            );
        }
        Some(ForeignAbi {
            value,
            span: token.span,
        })
    }

    /// Everything after `extern` at namespace level: a block of foreign
    /// declarations, a C-callable function, or a foreign struct or union.
    pub(super) fn parse_extern_items(
        &mut self,
        access: MemberAccess,
        access_span: Option<Span>,
        documentation: Option<DocComment>,
        attributes: Vec<AttributeGroup>,
    ) -> Option<Vec<Item>> {
        let extern_span = self.previous().span;
        let start = access_span.unwrap_or(extern_span).start;
        let abi = self.parse_foreign_abi()?;
        if self.check(&TokenKind::LeftBrace) {
            return self.parse_extern_block(
                access,
                access_span,
                extern_span,
                abi,
                documentation,
                attributes,
            );
        }
        if matches!(&self.peek().kind, TokenKind::Identifier(word) if word == "struct" || word == "union")
        {
            let aggregate =
                self.parse_foreign_aggregate(access, access_span, extern_span, abi, documentation)?;
            self.attach_attributes(
                attributes,
                AttributeTargetKind::ForeignAggregate,
                aggregate.span,
                aggregate.name_span,
                vec![AttributeTargetRole::Declaration],
            );
            return Some(vec![Item::ForeignAggregate(aggregate)]);
        }
        let unsafe_span = self
            .match_kind(&TokenKind::Unsafe)
            .then(|| self.previous().span);
        if self.check(&TokenKind::Function)
            && self
                .tokens
                .get(self.current + 1)
                .is_some_and(Self::is_callable_name_token)
        {
            self.advance();
            let mut function = self.parse_function_with_modifiers(FunctionModifiers {
                access,
                open_span: None,
                override_span: None,
                writable_span: None,
                consuming_span: None,
                static_span: None,
                unsafe_span,
                start_span: self.span(start, self.previous().span.end),
                is_member: false,
                allow_requirement: false,
                foreign_declaration: false,
            })?;
            function.linkage = FunctionLinkage::CAbi {
                extern_span,
                abi: Box::new(abi),
            };
            function.documentation = documentation.map(Box::new);
            let item = Item::Function(function);
            self.attach_item_attributes(attributes, &item);
            return Some(vec![item]);
        }
        self.diagnostics.push(
            Diagnostic::new(
                "P0001",
                "`extern \"C\"` is followed by `{` and declarations, by `function` and a C-callable function, or by `struct` or `union`",
                self.peek().span,
            )
            .with_title("Extern Declaration Is Incomplete"),
        );
        None
    }

    fn parse_extern_block(
        &mut self,
        access: MemberAccess,
        access_span: Option<Span>,
        extern_span: Span,
        abi: ForeignAbi,
        documentation: Option<DocComment>,
        attributes: Vec<AttributeGroup>,
    ) -> Option<Vec<Item>> {
        let start = access_span.unwrap_or(extern_span).start;
        let open_brace_span = self.advance().span;
        let mut functions = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            let declaration_index = self.current;
            if self.recovery_stalled_at == Some(declaration_index) {
                self.advance();
                continue;
            }
            self.recovery_stalled_at = Some(declaration_index);
            let documentation = self.documentation_before(declaration_index);
            let Some(member_attributes) = self.parse_attribute_groups() else {
                self.synchronize_member();
                continue;
            };
            let declaration_start = self.peek().span.start;
            let unsafe_span = self
                .match_kind(&TokenKind::Unsafe)
                .then(|| self.previous().span);
            if let Some(span) = unsafe_span {
                self.diagnostics.push(
                    Diagnostic::new(
                        "P0001",
                        "every declaration in an `extern` block is called only in an unsafe context, so it takes no `unsafe` modifier",
                        span,
                    )
                    .with_title("Extern Declaration Does Not Take Unsafe")
                    .with_fix(self.span(span.start, self.peek().span.start), ""),
                );
            }
            if !self.check(&TokenKind::Function) {
                self.diagnostics.push(
                    Diagnostic::new(
                        "P0001",
                        "an `extern` block contains function declarations only",
                        self.peek().span,
                    )
                    .with_title("Extern Block Member Is Not A Function"),
                );
                self.synchronize_member();
                if self.check(&TokenKind::Semicolon) {
                    self.advance();
                }
                continue;
            }
            self.advance();
            let Some(mut function) = self.parse_function_with_modifiers(FunctionModifiers {
                access,
                open_span: None,
                override_span: None,
                writable_span: None,
                consuming_span: None,
                static_span: None,
                unsafe_span: None,
                start_span: self.span(declaration_start, self.previous().span.end),
                is_member: false,
                allow_requirement: false,
                foreign_declaration: true,
            }) else {
                self.synchronize_member();
                continue;
            };
            function.documentation = documentation.map(Box::new);
            self.attach_attributes(
                member_attributes,
                AttributeTargetKind::ForeignFunction,
                function.span,
                function.name_span,
                vec![AttributeTargetRole::Declaration],
            );
            functions.push(function);
        }
        let close_brace_span = self
            .expect(TokenKind::RightBrace, "expected `}` after the extern block")?
            .span;
        let span = self.span(start, close_brace_span.end);
        let block = ExternBlock {
            access,
            access_span,
            extern_span,
            abi,
            open_brace_span,
            close_brace_span,
            documentation,
            declarations: functions.iter().map(|function| function.span).collect(),
            span,
        };
        self.attach_attributes(
            attributes,
            AttributeTargetKind::ExternBlock,
            span,
            extern_span,
            vec![AttributeTargetRole::Declaration],
        );
        let mut items = vec![Item::ExternBlock(block)];
        items.extend(functions.into_iter().map(|mut function| {
            function.linkage = FunctionLinkage::Foreign { block: span };
            Item::Function(function)
        }));
        Some(items)
    }

    fn parse_foreign_aggregate(
        &mut self,
        access: MemberAccess,
        access_span: Option<Span>,
        extern_span: Span,
        abi: ForeignAbi,
        documentation: Option<DocComment>,
    ) -> Option<ForeignAggregateDecl> {
        let start = access_span.unwrap_or(extern_span).start;
        let keyword = self.advance().clone();
        let kind = match &keyword.kind {
            TokenKind::Identifier(word) if word == "union" => ForeignAggregateKind::Union,
            _ => ForeignAggregateKind::Struct,
        };
        self.mark_cursor(crate::syntax::CompletionContext::DeclarationName);
        let name = self.expect_type_declaration_name(match kind {
            ForeignAggregateKind::Struct => "expected struct name",
            ForeignAggregateKind::Union => "expected union name",
        })?;
        let name_span = self.previous().span;
        if self.check(&TokenKind::Semicolon) {
            let end = self.advance().span.end;
            return Some(ForeignAggregateDecl {
                access,
                access_span,
                extern_span,
                abi,
                kind,
                keyword_span: keyword.span,
                name,
                name_span,
                fields: None,
                documentation,
                span: self.span(start, end),
            });
        }
        self.expect(
            TokenKind::LeftBrace,
            match kind {
                ForeignAggregateKind::Struct => "expected `{` or `;` after the struct name",
                ForeignAggregateKind::Union => "expected `{` or `;` after the union name",
            },
        )?;
        let mut fields = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            let field_index = self.current;
            if self.recovery_stalled_at == Some(field_index) {
                self.advance();
                continue;
            }
            self.recovery_stalled_at = Some(field_index);
            if let Some(field) = self.parse_foreign_field(kind) {
                fields.push(field);
            } else {
                self.synchronize_member();
                if self.check(&TokenKind::Semicolon) {
                    self.advance();
                }
            }
        }
        let close = self
            .expect(
                TokenKind::RightBrace,
                match kind {
                    ForeignAggregateKind::Struct => "expected `}` after the struct fields",
                    ForeignAggregateKind::Union => "expected `}` after the union members",
                },
            )?
            .span;
        Some(ForeignAggregateDecl {
            access,
            access_span,
            extern_span,
            abi,
            kind,
            keyword_span: keyword.span,
            name,
            name_span,
            fields: Some(fields),
            documentation,
            span: self.span(start, close.end),
        })
    }

    fn parse_foreign_field(&mut self, kind: ForeignAggregateKind) -> Option<ForeignFieldDecl> {
        if self.check(&TokenKind::AttributeOpen) {
            let groups = self.parse_attribute_groups()?;
            self.reject_attribute_target(&groups, "foreign struct and union fields");
        }
        if matches!(
            self.peek().kind,
            TokenKind::Writable
                | TokenKind::Readonly
                | TokenKind::Static
                | TokenKind::Internal
                | TokenKind::Function
                | TokenKind::Const
        ) {
            let token = self.peek().clone();
            self.diagnostics.push(
                Diagnostic::new(
                    "P0001",
                    match kind {
                        ForeignAggregateKind::Struct => {
                            "a foreign struct contains fields only: a type, a name and an optional constant default"
                        }
                        ForeignAggregateKind::Union => {
                            "a foreign union contains members only: a type and a name"
                        }
                    },
                    token.span,
                )
                .with_title("Foreign Aggregate Member Is Not A Field"),
            );
            return None;
        }
        let type_start = self.peek().span.start;
        self.foreign_field_type = true;
        let ty = self.parse_type_ref();
        self.foreign_field_type = false;
        let ty = ty?;
        let type_span = self.span(type_start, self.previous().span.end);
        self.mark_cursor(crate::syntax::CompletionContext::DeclarationName);
        let (name, name_span) = self.expect_variable("expected field variable name")?;
        let default = if self.match_kind(&TokenKind::Equals) {
            Some(self.parse_expression()?)
        } else {
            None
        };
        let end = self
            .expect(TokenKind::Semicolon, "expected `;` after the field")?
            .span
            .end;
        Some(ForeignFieldDecl {
            ty,
            type_span,
            name,
            name_span,
            default,
            span: self.span(type_start, end),
        })
    }

    /// `[N]` after a foreign field's element type.
    pub(super) fn parse_fixed_array_suffix(&mut self, element: TypeRef) -> Option<TypeRef> {
        let token = self.advance().clone();
        let TokenKind::IntLiteral(count) = token.kind else {
            self.error(
                "expected the element count of a fixed-length array field",
                token.span,
            );
            return None;
        };
        self.expect(
            TokenKind::RightBracket,
            "expected `]` after the fixed-length array count",
        )?;
        Some(TypeRef::fixed_array_of(element, count))
    }

    /// `extern "C" function(...): R` in a type position, after `extern`.
    pub(super) fn parse_c_function_type_ref(&mut self, extern_span: Span) -> Option<TypeRef> {
        let abi = self.parse_foreign_abi()?;
        let keyword = self
            .expect(
                TokenKind::Function,
                "expected `function` after `extern \"C\"` in a type",
            )?
            .span;
        let mut ty = self.parse_function_type_ref(keyword)?;
        if let Some(function) = ty.function.as_mut() {
            function.c_abi = Some(CAbiTypeRef { extern_span, abi });
            function.span = extern_span.merge(function.span);
        }
        Some(ty)
    }

    /// `unsafe(<expression>)` yields a value: it is never assigned to.
    pub(super) fn reject_unsafe_place(&mut self, target: &Expr) {
        if let Expr::Unsafe { span, .. } = target {
            self.diagnostics.push(
                Diagnostic::new(
                    "P0001",
                    "`unsafe(<expression>)` yields a value and cannot be assigned to or incremented",
                    *span,
                )
                .with_title("Unsafe Expression Is Not A Place")
                .with_help("write the assignment inside an `unsafe { ... }` block"),
            );
        }
    }

    /// `unsafe { ... }` in statement position, after the keyword.
    pub(super) fn parse_unsafe_block(&mut self) -> Option<Stmt> {
        let keyword_span = self.previous().span;
        let block = self.parse_block()?;
        Some(Stmt::Unsafe(UnsafeBlock {
            keyword_span,
            span: keyword_span.merge(block.span),
            block,
        }))
    }

    /// `unsafe(<expression>)`, after the keyword.
    pub(super) fn parse_unsafe_expression(&mut self, keyword_span: Span) -> Option<Expr> {
        if !self.check(&TokenKind::LeftParen) {
            self.diagnostics.push(
                Diagnostic::new(
                    "P0001",
                    "an unsafe expression is `unsafe(<expression>)`; a statement block is `unsafe { ... }`",
                    self.peek().span,
                )
                .with_title("Unsafe Expression Expects Parentheses"),
            );
            return None;
        }
        self.advance();
        let expr = self.parse_expression()?;
        let close = self
            .expect(
                TokenKind::RightParen,
                "expected `)` after the unsafe expression",
            )?
            .span;
        Some(Expr::Unsafe {
            keyword_span,
            expr: Box::new(expr),
            span: keyword_span.merge(close),
        })
    }
}

fn is_interpolated(raw: &str, quote: StringQuoteKind) -> bool {
    matches!(quote, StringQuoteKind::Double)
        && raw.char_indices().any(|(index, character)| {
            character == '{'
                && (index == 0 || !raw[..index].ends_with('\\'))
                && interpolation_close(raw, index).is_some()
        })
}
