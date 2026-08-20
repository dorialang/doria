use crate::ast::*;
use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::lexer::{Lexer, StringQuoteKind, Token, TokenKind};
use crate::source::{SourceFile, Span};
use crate::string_literal::{decode_escape, interpolation_close};
use crate::types::{
    FunctionInvocationMode, FunctionTypeEffectRef, FunctionTypeParameterMode,
    FunctionTypeParameterRef, FunctionTypeRef, FunctionTypeThrowsRef, TypeRef,
};

pub struct Parser {
    tokens: Vec<Token>,
    current: usize,
    pending_type_argument_close: Option<Span>,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Clone, Copy)]
struct ParserCheckpoint {
    current: usize,
    pending_type_argument_close: Option<Span>,
    diagnostics_len: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            current: 0,
            pending_type_argument_close: None,
            diagnostics: Vec::new(),
        }
    }

    pub fn parse_program(mut self) -> DiagnosticResult<Program> {
        let namespace = if self.match_kind(&TokenKind::Namespace) {
            self.parse_namespace()
        } else {
            None
        };
        let mut items = Vec::new();

        while !self.is_at_end() {
            match self.parse_item() {
                Some(item) => items.push(item),
                None => self.synchronize(),
            }
        }

        if self.diagnostics.is_empty() {
            Ok(Program { namespace, items })
        } else {
            Err(self.diagnostics)
        }
    }

    fn checkpoint(&self) -> ParserCheckpoint {
        ParserCheckpoint {
            current: self.current,
            pending_type_argument_close: self.pending_type_argument_close,
            diagnostics_len: self.diagnostics.len(),
        }
    }

    fn restore_checkpoint(&mut self, checkpoint: ParserCheckpoint) {
        self.current = checkpoint.current;
        self.pending_type_argument_close = checkpoint.pending_type_argument_close;
        self.diagnostics.truncate(checkpoint.diagnostics_len);
    }

    fn parse_namespace(&mut self) -> Option<NamespaceDecl> {
        let start = self.previous().span.start;
        let name = self.expect_qualified_name("expected namespace name")?;
        let end = self
            .expect(
                TokenKind::Semicolon,
                "expected `;` after namespace declaration",
            )?
            .span
            .end;
        Some(NamespaceDecl {
            name,
            span: Span::new(start, end),
        })
    }

    fn parse_item(&mut self) -> Option<Item> {
        if self.match_kind(&TokenKind::Class) {
            self.parse_class().map(Item::Class)
        } else if self.match_kind(&TokenKind::Enum) {
            self.parse_enum().map(Item::Enum)
        } else if self.match_kind(&TokenKind::Interface) {
            self.parse_interface().map(Item::Interface)
        } else if self.match_kind(&TokenKind::Trait) {
            self.parse_trait().map(Item::Trait)
        } else if self.check(&TokenKind::Function)
            && self
                .tokens
                .get(self.current + 1)
                .is_some_and(|token| matches!(token.kind, TokenKind::Identifier(_)))
        {
            self.advance();
            self.parse_function(MemberAccess::External, None, None, self.previous().span)
                .map(Item::Function)
        } else if self.match_kind(&TokenKind::Const) {
            let start = self.previous().span.start;
            self.parse_const_decl(MemberAccess::External, start)
                .map(Item::Constant)
        } else {
            self.parse_statement().map(Item::Statement)
        }
    }

    fn parse_enum(&mut self) -> Option<EnumDecl> {
        let start = self.previous().span.start;
        let name = self.expect_type_declaration_name("expected enum name")?;
        let name_span = self.previous().span;
        let type_params = self.parse_type_params()?;
        let backing_type = if self.match_kind(&TokenKind::Colon) {
            Some(self.parse_type_ref()?)
        } else {
            None
        };
        self.expect(TokenKind::LeftBrace, "expected `{` after enum declaration")?;

        let mut cases = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            let case_start = self.peek().span.start;
            self.expect(TokenKind::Case, "expected `case` in enum body")?;
            let name = self.expect_identifier("expected enum case name")?;
            let name_span = self.previous().span;
            let payload = if self.match_kind(&TokenKind::LeftParen) {
                self.parse_enum_payload_fields()?
            } else {
                Vec::new()
            };
            let backing_value = if self.match_kind(&TokenKind::Equals) {
                Some(self.parse_expression()?)
            } else {
                None
            };
            let end = self
                .expect(TokenKind::Semicolon, "expected `;` after enum case")?
                .span
                .end;
            cases.push(EnumCaseDecl {
                name,
                name_span,
                payload,
                backing_value,
                span: Span::new(case_start, end),
            });
        }

        let end = self
            .expect(TokenKind::RightBrace, "expected `}` after enum body")?
            .span
            .end;
        Some(EnumDecl {
            name,
            name_span,
            type_params,
            backing_type,
            cases,
            span: Span::new(start, end),
        })
    }

    fn parse_enum_payload_fields(&mut self) -> Option<Vec<EnumPayloadField>> {
        let mut fields = Vec::new();
        if self.match_kind(&TokenKind::RightParen) {
            return Some(fields);
        }
        loop {
            let start = self.peek().span.start;
            let ty = self.parse_type_ref()?;
            let (name, name_span) = self.expect_variable("expected enum payload variable")?;
            fields.push(EnumPayloadField {
                ty,
                name,
                span: Span::new(start, name_span.end),
            });
            if !self.match_kind(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(TokenKind::RightParen, "expected `)` after enum payload")?;
        Some(fields)
    }

    fn parse_class(&mut self) -> Option<ClassDecl> {
        let start = self.previous().span.start;
        let name = self.expect_type_declaration_name("expected class name")?;
        let type_params = self.parse_type_params()?;
        let (parent, parent_span) = if self.match_kind(&TokenKind::Extends) {
            let parent_start = self.previous().span.start;
            let parent = self.expect_qualified_name("expected parent class after `extends`")?;
            (
                Some(parent),
                Some(Span::new(parent_start, self.previous().span.end)),
            )
        } else {
            (None, None)
        };
        let mut implements = Vec::new();
        if self.match_kind(&TokenKind::Implements) {
            loop {
                implements.push(
                    self.expect_qualified_name("expected interface name after `implements`")?,
                );
                if !self.match_kind(&TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(TokenKind::LeftBrace, "expected `{` after class name")?;

        let mut members = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            if let Some(member) = self.parse_class_member() {
                members.push(member);
            } else {
                self.synchronize();
            }
        }

        let end = self
            .expect(TokenKind::RightBrace, "expected `}` after class body")?
            .span
            .end;

        Some(ClassDecl {
            name,
            type_params,
            parent,
            parent_span,
            implements,
            members,
            span: Span::new(start, end),
        })
    }

    fn parse_trait(&mut self) -> Option<TraitDecl> {
        let start = self.previous().span.start;
        let name = self.expect_type_declaration_name("expected trait name")?;
        self.expect(TokenKind::LeftBrace, "expected `{` after trait name")?;

        let mut members = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            if let Some(member) = self.parse_class_member() {
                members.push(member);
            } else {
                self.synchronize();
            }
        }

        let end = self
            .expect(TokenKind::RightBrace, "expected `}` after trait body")?
            .span
            .end;
        Some(TraitDecl {
            name,
            members,
            span: Span::new(start, end),
        })
    }

    fn parse_class_member(&mut self) -> Option<ClassMember> {
        let access = self.parse_member_access();
        if self.match_kind(&TokenKind::Const) {
            let start = self.previous().span.start;
            return self
                .parse_const_decl(access, start)
                .map(ClassMember::Constant);
        }
        let static_span = self
            .match_kind(&TokenKind::Static)
            .then(|| self.previous().span);

        let writable_span = self
            .match_kind(&TokenKind::Writable)
            .then(|| self.previous().span);
        if self.check(&TokenKind::Function)
            && self
                .tokens
                .get(self.current + 1)
                .is_some_and(|token| matches!(token.kind, TokenKind::Identifier(_)))
        {
            self.advance();
            let start = self.previous().span.start;
            return self
                .parse_function(access, writable_span, static_span, Span::new(start, start))
                .map(ClassMember::Method);
        }

        let start = self.peek().span.start;
        let ty = self.parse_type_ref()?;
        let (name, name_span) = self.expect_variable("expected property variable name")?;
        let initializer = if self.match_kind(&TokenKind::Equals) {
            Some(self.parse_expression()?)
        } else {
            None
        };
        let end = self
            .expect(
                TokenKind::Semicolon,
                "expected `;` after property declaration",
            )?
            .span
            .end;

        Some(ClassMember::Property(PropertyDecl {
            access,
            is_static: static_span.is_some(),
            writable: writable_span.is_some(),
            ty,
            name,
            initializer,
            span: Span::new(start.min(name_span.start), end),
        }))
    }

    fn parse_const_decl(&mut self, access: MemberAccess, start: usize) -> Option<ConstDecl> {
        let inferred = matches!(self.peek().kind, TokenKind::Identifier(_))
            && self
                .tokens
                .get(self.current + 1)
                .is_some_and(|token| matches!(token.kind, TokenKind::Equals));
        let ty = if inferred {
            None
        } else {
            Some(self.parse_type_ref()?)
        };
        let name = self.expect_identifier("expected constant name")?;
        self.expect(TokenKind::Equals, "expected `=` after constant name")?;
        let initializer = self.parse_expression()?;
        let end = self
            .expect(
                TokenKind::Semicolon,
                "expected `;` after constant declaration",
            )?
            .span
            .end;
        Some(ConstDecl {
            access,
            ty,
            name,
            initializer,
            span: Span::new(start, end),
        })
    }

    fn parse_function(
        &mut self,
        access: MemberAccess,
        writable_span: Option<Span>,
        static_span: Option<Span>,
        start_span: Span,
    ) -> Option<FunctionDecl> {
        let start = start_span.start;
        let name = self.expect_identifier("expected function name")?;
        let type_params = self.parse_type_params()?;
        self.expect(TokenKind::LeftParen, "expected `(` after function name")?;

        let mut params = Vec::new();
        if !self.check(&TokenKind::RightParen) {
            loop {
                params.push(self.parse_param(name == "__construct")?);
                if !self.match_kind(&TokenKind::Comma) {
                    break;
                }
                if self.check(&TokenKind::RightParen) {
                    break;
                }
            }
        }

        self.expect(TokenKind::RightParen, "expected `)` after parameters")?;

        let return_type = if self.match_kind(&TokenKind::Colon) {
            Some(self.parse_type_ref()?)
        } else {
            None
        };

        let throws = if self.match_kind(&TokenKind::Throws) {
            Some(self.parse_throws_clause()?)
        } else {
            None
        };

        let body = self.parse_block()?;
        let span = Span::new(start, body.span.end);
        Some(FunctionDecl {
            access,
            writable_this: writable_span.is_some(),
            writable_span,
            is_static: static_span.is_some(),
            static_span,
            name,
            type_params,
            params,
            return_type,
            throws,
            body,
            span,
        })
    }

    fn parse_throws_clause(&mut self) -> Option<ThrowsClause> {
        let keyword_span = self.previous().span;
        let mut entries = Vec::new();
        loop {
            let start = self.peek().span.start;
            let ty = self.parse_type_ref()?;
            let span = Span::new(start, self.previous().span.end);
            entries.push(ThrowsEntry { ty, span });
            if !self.match_kind(&TokenKind::Comma) {
                break;
            }
        }
        let end = entries.last()?.span.end;
        Some(ThrowsClause {
            keyword_span,
            entries,
            span: Span::new(keyword_span.start, end),
        })
    }

    fn parse_type_params(&mut self) -> Option<Vec<TypeParamDecl>> {
        if !self.match_kind(&TokenKind::Less) {
            return Some(Vec::new());
        }

        let mut params = Vec::new();
        loop {
            let start = self.peek().span.start;
            let name = self.expect_identifier("expected type-parameter name")?;
            let mut constraints = Vec::new();
            if self.match_kind(&TokenKind::Implements) {
                loop {
                    constraints.push(self.parse_type_ref_inner()?);
                    // A comma followed by `Name implements` starts the next
                    // parameter. Without that second `implements`,
                    // `<T implements A, U>` remains the documented
                    // comma-separated constraint list for `T`.
                    if !self.check(&TokenKind::Comma)
                        || self
                            .tokens
                            .get(self.current + 1)
                            .zip(self.tokens.get(self.current + 2))
                            .is_some_and(|(name, implements)| {
                                matches!(name.kind, TokenKind::Identifier(_))
                                    && matches!(implements.kind, TokenKind::Implements)
                            })
                    {
                        break;
                    }
                    self.advance();
                }
            }
            let default_type = if self.match_kind(&TokenKind::Equals) {
                Some(self.parse_type_ref_inner()?)
            } else {
                None
            };
            let end = self.previous().span.end;
            params.push(TypeParamDecl {
                name,
                constraints,
                default_type,
                span: Span::new(start, end),
            });

            if self.pending_type_argument_close.take().is_some() {
                break;
            }
            if self.check(&TokenKind::Greater) {
                self.advance();
                break;
            }
            self.expect(TokenKind::Comma, "expected `,` or `>` after type parameter")?;
        }

        Some(params)
    }

    fn parse_interface(&mut self) -> Option<InterfaceDecl> {
        let start = self.previous().span.start;
        let name = self.expect_identifier("expected interface name")?;
        self.expect(TokenKind::LeftBrace, "expected `{` after interface name")?;

        let mut depth = 1_usize;
        let mut end = self.previous().span.end;
        while depth > 0 {
            if self.is_at_end() {
                self.error("expected `}` after interface body", self.peek().span);
                return None;
            }
            let token = self.advance();
            end = token.span.end;
            match token.kind {
                TokenKind::LeftBrace => depth += 1,
                TokenKind::RightBrace => depth -= 1,
                _ => {}
            }
        }

        Some(InterfaceDecl {
            name,
            span: Span::new(start, end),
        })
    }

    fn parse_param(&mut self, is_constructor: bool) -> Option<Param> {
        let start = self.peek().span.start;
        if !is_constructor && self.check(&TokenKind::Internal) {
            let span = self.advance().span;
            self.error(
                "`internal` is only valid on class members and constructor-promoted properties",
                span,
            );
            return None;
        }

        let access = self.parse_member_access();
        let ownership_modifier_insert = Span::new(self.peek().span.start, self.peek().span.start);
        let mut take_span = None;
        let mut writable_span = None;
        while self.check(&TokenKind::Take) || self.check(&TokenKind::Writable) {
            let token = self.advance().clone();
            match token.kind {
                TokenKind::Take if take_span.is_none() => take_span = Some(token.span),
                TokenKind::Writable if writable_span.is_none() => writable_span = Some(token.span),
                TokenKind::Take => self.error("duplicate `take` parameter modifier", token.span),
                TokenKind::Writable => {
                    self.error("duplicate `writable` parameter modifier", token.span)
                }
                _ => unreachable!("modifier loop accepts only take/writable"),
            }
        }
        let ty = self.parse_type_ref()?;
        if self.match_kind(&TokenKind::Ampersand) {
            self.error(
                "Doria does not support PHP-style parameter references; use `writable` for an exclusive borrow or `take` for ownership transfer",
                self.previous().span,
            );
            return None;
        }
        let (name, name_span) = self.expect_variable("expected parameter variable name")?;
        let default = if self.match_kind(&TokenKind::Equals) {
            Some(self.parse_expression()?)
        } else {
            None
        };

        let end = default.as_ref().map(Expr::span).unwrap_or(name_span).end;

        Some(Param {
            promoted_access: is_constructor.then_some(access),
            take: take_span.is_some(),
            take_span,
            writable: writable_span.is_some(),
            writable_span,
            ownership_modifier_insert,
            ty,
            name,
            default,
            span: Span::new(start, end),
        })
    }

    fn parse_arrow_closure(&mut self, keyword_span: Span) -> Option<Expr> {
        let (parameters, parameter_list_span) = self.parse_closure_parameters()?;
        if self.check(&TokenKind::Colon) {
            let colon = self.advance().span;
            self.diagnostics.push(
                Diagnostic::new(
                    "P0001",
                    "an explicit arrow return annotation has not been accepted; omit it and let the arrow body determine the return type",
                    colon,
                )
                .with_title("Arrow Return Annotation Is Not Accepted"),
            );
            let _ = self.parse_type_ref();
        }
        let captures = self.parse_optional_closure_captures()?;
        let arrow_span = self
            .expect(
                TokenKind::FatArrow,
                "expected `=>` before arrow closure body",
            )?
            .span;
        let expression = self.parse_expression()?;
        let span = Span::new(keyword_span.start, expression.span().end);
        Some(Expr::Closure(Box::new(ClosureExpression {
            form: ClosureForm::Arrow,
            keyword_span,
            parameter_list_span,
            parameters,
            return_type: None,
            captures,
            body: ClosureBody::Expression {
                arrow_span,
                expression: Box::new(expression),
            },
            span,
        })))
    }

    fn parse_anonymous_block_closure(&mut self, keyword_span: Span) -> Option<Expr> {
        let (parameters, parameter_list_span) = self.parse_closure_parameters()?;
        let return_type = if self.match_kind(&TokenKind::Colon) {
            let colon_span = self.previous().span;
            let return_start = self.peek().span.start;
            let return_type = self.parse_type_ref()?;
            let return_span = Span::new(return_start, self.previous().span.end);
            Some(ClosureReturnType {
                colon_span,
                ty: return_type,
                type_span: return_span,
                span: Span::new(colon_span.start, return_span.end),
            })
        } else {
            self.diagnostics.push(
                Diagnostic::new(
                    "P0001",
                    "an anonymous block closure requires a written return type after its parameter list",
                    self.peek().span,
                )
                .with_title("Anonymous Closure Return Type Is Required")
                .with_help("write `: ReturnType` before `with` or the block body"),
            );
            None
        };
        let captures = self.parse_optional_closure_captures()?;
        if self.check(&TokenKind::FatArrow) {
            let arrow = self.advance().span;
            self.diagnostics.push(
                Diagnostic::new(
                    "P0001",
                    "anonymous `function` closures use a block body; use `fn` for an arrow body",
                    arrow,
                )
                .with_title("Anonymous Closure Requires A Block"),
            );
            return None;
        }
        let block = self.parse_block()?;
        let span = Span::new(keyword_span.start, block.span.end);
        Some(Expr::Closure(Box::new(ClosureExpression {
            form: ClosureForm::AnonymousBlock,
            keyword_span,
            parameter_list_span,
            parameters,
            return_type,
            captures,
            body: ClosureBody::Block(block),
            span,
        })))
    }

    fn parse_closure_parameters(&mut self) -> Option<(Vec<ClosureParameter>, Span)> {
        let open_span = self
            .expect(TokenKind::LeftParen, "expected `(` after closure keyword")?
            .span;
        let mut parameters = Vec::new();
        if !self.check(&TokenKind::RightParen) {
            loop {
                parameters.push(self.parse_closure_parameter()?);
                if !self.match_kind(&TokenKind::Comma) {
                    break;
                }
                if self.check(&TokenKind::RightParen) {
                    break;
                }
            }
        }
        let close_span = self
            .expect(
                TokenKind::RightParen,
                "expected `)` after closure parameters",
            )?
            .span;
        Some((parameters, open_span.merge(close_span)))
    }

    fn parse_closure_parameter(&mut self) -> Option<ClosureParameter> {
        let start = self.peek().span.start;
        let ownership_modifier_insert = Span::new(start, start);
        let mut take_span = None;
        let mut writable_span = None;
        while self.check(&TokenKind::Take) || self.check(&TokenKind::Writable) {
            let token = self.advance().clone();
            match token.kind {
                TokenKind::Take if take_span.is_none() => take_span = Some(token.span),
                TokenKind::Writable if writable_span.is_none() => writable_span = Some(token.span),
                TokenKind::Take => {
                    self.error("duplicate `take` closure parameter modifier", token.span)
                }
                TokenKind::Writable => self.error(
                    "duplicate `writable` closure parameter modifier",
                    token.span,
                ),
                _ => unreachable!("closure parameter modifier loop is exact"),
            }
        }

        if let Some((name, name_span)) = self.consume_variable() {
            self.diagnostics.push(
                Diagnostic::new(
                    "P0001",
                    format!("closure parameter `${name}` requires a written type"),
                    name_span,
                )
                .with_title("Closure Parameter Type Is Required")
                .with_help("write the parameter type before the variable name"),
            );
            return Some(ClosureParameter {
                take: take_span.is_some(),
                take_span,
                writable: writable_span.is_some(),
                writable_span,
                ty: TypeRef::unknown(),
                type_span: ownership_modifier_insert,
                name,
                name_span,
                span: Span::new(start, name_span.end),
            });
        }

        let type_start = self.peek().span.start;
        let ty = self.parse_type_ref()?;
        let type_span = Span::new(type_start, self.previous().span.end);
        let (name, name_span) = self.expect_variable("expected closure parameter variable name")?;
        if self.match_kind(&TokenKind::Equals) {
            let equals = self.previous().span;
            self.diagnostics.push(
                Diagnostic::new(
                    "P0001",
                    "closure parameter defaults are not part of the accepted closure grammar",
                    equals,
                )
                .with_title("Closure Parameter Default Is Not Accepted"),
            );
            let _ = self.parse_expression();
        }
        Some(ClosureParameter {
            take: take_span.is_some(),
            take_span,
            writable: writable_span.is_some(),
            writable_span,
            ty,
            type_span,
            name,
            name_span,
            span: Span::new(start, name_span.end),
        })
    }

    fn parse_optional_closure_captures(&mut self) -> Option<Option<ClosureCaptureClause>> {
        if self.match_kind(&TokenKind::With) {
            let keyword_span = self.previous().span;
            return self.parse_closure_capture_clause(keyword_span).map(Some);
        }
        if matches!(&self.peek().kind, TokenKind::Identifier(name) if name == "use")
            && self
                .tokens
                .get(self.current + 1)
                .is_some_and(|token| matches!(token.kind, TokenKind::LeftParen))
        {
            let keyword_span = self.advance().span;
            self.diagnostics.push(
                Diagnostic::new(
                    "P0001",
                    "Doria closure captures use `with`, not PHP closure `use`",
                    keyword_span,
                )
                .with_title("Closure Capture Uses With")
                .with_help("replace `use` with `with`"),
            );
            return self.parse_closure_capture_clause(keyword_span).map(Some);
        }
        Some(None)
    }

    fn parse_closure_capture_clause(&mut self, keyword_span: Span) -> Option<ClosureCaptureClause> {
        let open_span = self
            .expect(
                TokenKind::LeftParen,
                "expected `(` after closure capture keyword",
            )?
            .span;
        let mut captures = Vec::new();
        if self.check(&TokenKind::RightParen) {
            let close_span = self.advance().span;
            self.diagnostics.push(
                Diagnostic::new(
                    "P0001",
                    "a closure without captures omits the `with` clause",
                    keyword_span.merge(close_span),
                )
                .with_title("Empty Closure Capture List Is Not Accepted")
                .with_help("remove `with ()`"),
            );
            return Some(ClosureCaptureClause {
                keyword_span,
                open_span,
                close_span,
                captures,
                span: keyword_span.merge(close_span),
            });
        }

        loop {
            captures.push(self.parse_closure_capture()?);
            if self.match_kind(&TokenKind::Comma) {
                if self.check(&TokenKind::RightParen) {
                    self.error(
                        "expected a captured variable after `,`",
                        self.previous().span,
                    );
                    break;
                }
                continue;
            }
            if self.check(&TokenKind::RightParen) {
                break;
            }
            if matches!(
                self.peek().kind,
                TokenKind::Variable(_)
                    | TokenKind::Writable
                    | TokenKind::Take
                    | TokenKind::Readonly
                    | TokenKind::Ampersand
            ) {
                self.error("expected `,` between closure captures", self.peek().span);
                continue;
            }
            break;
        }

        let close_span = self
            .expect(TokenKind::RightParen, "expected `)` after closure captures")?
            .span;
        Some(ClosureCaptureClause {
            keyword_span,
            open_span,
            close_span,
            captures,
            span: keyword_span.merge(close_span),
        })
    }

    fn parse_closure_capture(&mut self) -> Option<ClosureCapture> {
        let start = self.peek().span.start;
        let (mode, modifier_span) = if self.match_kind(&TokenKind::Writable) {
            (ClosureCaptureMode::Writable, Some(self.previous().span))
        } else if self.match_kind(&TokenKind::Take) {
            (ClosureCaptureMode::Take, Some(self.previous().span))
        } else if self.match_kind(&TokenKind::Readonly) {
            let span = self.previous().span;
            self.diagnostics.push(
                Diagnostic::new(
                    "P0001",
                    "readonly closure capture is written as a bare variable",
                    span,
                )
                .with_title("Readonly Capture Does Not Use A Modifier")
                .with_help("remove `readonly` and keep the captured variable"),
            );
            (ClosureCaptureMode::Readonly, Some(span))
        } else {
            (ClosureCaptureMode::Readonly, None)
        };

        while self.check(&TokenKind::Writable)
            || self.check(&TokenKind::Take)
            || self.check(&TokenKind::Readonly)
        {
            let span = self.advance().span;
            self.diagnostics.push(
                Diagnostic::new(
                    "P0001",
                    "a closure capture has exactly one ownership mode",
                    span,
                )
                .with_title("Closure Capture Has Multiple Modifiers"),
            );
        }
        if self.match_kind(&TokenKind::Ampersand) {
            let span = self.previous().span;
            self.diagnostics.push(
                Diagnostic::new(
                    "P0001",
                    "Doria closure captures do not use PHP reference `&` syntax",
                    span,
                )
                .with_title("Closure Capture Does Not Use Ampersand")
                .with_help("use a bare variable for readonly capture or `writable $value` for an exclusive borrow"),
            );
        }
        let (name, name_span) = self.expect_variable("expected captured variable")?;
        Some(ClosureCapture {
            mode,
            modifier_span,
            name,
            name_span,
            span: Span::new(start, name_span.end),
        })
    }

    fn parse_block(&mut self) -> Option<Block> {
        let start = self
            .expect(TokenKind::LeftBrace, "expected `{` before block")?
            .span
            .start;
        let mut statements = Vec::new();

        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            if let Some(statement) = self.parse_statement() {
                statements.push(statement);
            } else {
                self.synchronize();
            }
        }

        let end = self
            .expect(TokenKind::RightBrace, "expected `}` after block")?
            .span
            .end;

        Some(Block {
            statements,
            span: Span::new(start, end),
        })
    }

    fn parse_statement(&mut self) -> Option<Stmt> {
        if self.check(&TokenKind::LeftBrace) {
            return self.parse_block().map(Stmt::Block);
        }
        if self.check(&TokenKind::Finally) {
            self.reject_disallowed_finally();
            return None;
        }
        if matches!(&self.peek().kind, TokenKind::Identifier(name) if name == "print") {
            let span = self.advance().span;
            self.diagnostics.push(
                Diagnostic::new("P0017", "Doria does not support `print`; use `echo`", span)
                    .with_help("echo writes output and does not return a value"),
            );
            while !self.check(&TokenKind::Semicolon) && !self.is_at_end() {
                self.advance();
            }
            self.match_kind(&TokenKind::Semicolon);
            return None;
        }
        if self.match_kind(&TokenKind::Let) {
            return self.parse_let_decl();
        }

        if self.match_kind(&TokenKind::Echo) {
            let start = self.previous().span.start;
            let expr = self.parse_expression()?;
            let end = self
                .expect(TokenKind::Semicolon, "expected `;` after echo statement")?
                .span
                .end;
            return Some(Stmt::Echo {
                expr,
                span: Span::new(start, end),
            });
        }

        if self.match_kind(&TokenKind::Return) {
            let start = self.previous().span.start;
            let expr = if self.check(&TokenKind::Semicolon) {
                None
            } else {
                Some(self.parse_expression()?)
            };
            let end = self
                .expect(TokenKind::Semicolon, "expected `;` after return statement")?
                .span
                .end;
            return Some(Stmt::Return {
                expr,
                span: Span::new(start, end),
            });
        }

        if self.match_kind(&TokenKind::Throw) {
            let keyword_span = self.previous().span;
            if self.check(&TokenKind::Semicolon) {
                self.advance();
                self.diagnostics.push(
                    Diagnostic::new("E0624", "bare `throw` is not supported", keyword_span)
                        .with_title("Throw Requires An Error Value")
                        .with_help("rethrow a caught Error with `throw $error;`"),
                );
                return None;
            }
            let expr = self.parse_expression()?;
            let semicolon_span = self
                .expect(TokenKind::Semicolon, "expected `;` after throw statement")?
                .span;
            return Some(Stmt::Throw(ThrowStmt {
                keyword_span,
                span: Span::new(keyword_span.start, semicolon_span.end),
                expr,
                semicolon_span,
            }));
        }

        if self.match_kind(&TokenKind::Try) {
            return self.parse_try_statement().map(Stmt::Try);
        }

        if self.match_kind(&TokenKind::Break) {
            return self.parse_loop_control_statement(
                TokenKind::Break,
                "`break` does not accept a value or label in this Doria slice",
                "expected `;` after break statement",
            );
        }

        if self.match_kind(&TokenKind::Continue) {
            return self.parse_loop_control_statement(
                TokenKind::Continue,
                "`continue` does not accept a value or label in this Doria slice",
                "expected `;` after continue statement",
            );
        }

        if self.match_kind(&TokenKind::If) {
            return self.parse_if_statement().map(Stmt::If);
        }

        if self.match_kind(&TokenKind::While) {
            return self.parse_while().map(Stmt::While);
        }

        if self.match_kind(&TokenKind::Do) {
            return self.parse_do_while().map(Stmt::DoWhile);
        }

        if self.match_kind(&TokenKind::Given) {
            return self.parse_given_statement();
        }

        if self.match_kind(&TokenKind::For) {
            return self
                .parse_for()
                .map(|for_stmt| Stmt::For(Box::new(for_stmt)));
        }

        if self.match_kind(&TokenKind::Foreach) {
            return self.parse_foreach();
        }

        if self.check(&TokenKind::PlusPlus) || self.check(&TokenKind::MinusMinus) {
            return self.parse_pre_increment_statement();
        }

        if self.can_start_typed_decl() {
            let checkpoint = self.checkpoint();
            let start = self.peek().span.start;
            let writable = self.match_kind(&TokenKind::Writable);
            if let Some(ty) = self.parse_type_ref() {
                if let Some((name, name_span)) = self.consume_variable() {
                    let bindings = self.parse_local_bindings(name, name_span)?;
                    self.expect(TokenKind::Equals, "expected `=` in variable declaration")?;
                    let initializer = self.parse_expression()?;
                    if self.reject_disallowed_finally() {
                        return None;
                    }
                    if self.reject_additional_group_initializer() {
                        return None;
                    }
                    let end = self
                        .expect(
                            TokenKind::Semicolon,
                            "expected `;` after variable declaration",
                        )?
                        .span
                        .end;
                    return Some(Stmt::VarDecl(VarDecl {
                        writable,
                        ty: Some(ty),
                        bindings,
                        initializer,
                        span: Span::new(start, end),
                    }));
                }
            }
            self.restore_checkpoint(checkpoint);
        }

        let expr = self.parse_expression()?;
        if self.reject_disallowed_finally() {
            return None;
        }
        if self.check(&TokenKind::PlusPlus) || self.check(&TokenKind::MinusMinus) {
            return self.parse_post_increment_statement(expr);
        }

        if let Some(op) = self.parse_assignment_op() {
            let start = expr.span().start;
            let value = self.parse_expression()?;
            let end = self
                .expect(TokenKind::Semicolon, "expected `;` after assignment")?
                .span
                .end;
            return Some(Stmt::Assignment(Assignment {
                target: expr,
                op,
                value,
                span: Span::new(start, end),
            }));
        }

        let end = self
            .expect(
                TokenKind::Semicolon,
                "expected `;` after expression statement",
            )?
            .span
            .end;
        Some(Stmt::Expr {
            span: Span::new(expr.span().start, end),
            expr,
        })
    }

    fn parse_try_statement(&mut self) -> Option<TryStmt> {
        let keyword_span = self.previous().span;
        let body = self.parse_block()?;
        let mut catches = Vec::new();
        while self.match_kind(&TokenKind::Catch) {
            let catch_keyword = self.previous().span;
            self.expect(TokenKind::LeftParen, "expected `(` after `catch`")?;
            let type_start = self.peek().span.start;
            let ty = self.parse_type_ref()?;
            let ty_span = Span::new(type_start, self.previous().span.end);
            let binding = if let Some((name, span)) = self.consume_variable() {
                Some(CatchBinding { name, span })
            } else {
                None
            };
            self.expect(TokenKind::RightParen, "expected `)` after catch type")?;
            let catch_body = self.parse_block()?;
            let span = Span::new(catch_keyword.start, catch_body.span.end);
            catches.push(CatchClause {
                keyword_span: catch_keyword,
                ty,
                ty_span,
                binding,
                body: catch_body,
                span,
            });
        }
        let finally = if self.match_kind(&TokenKind::Finally) {
            let finally_keyword = self.previous().span;
            let finally_body = self.parse_block()?;
            Some(TryFinally {
                keyword_span: finally_keyword,
                span: Span::new(finally_keyword.start, finally_body.span.end),
                body: finally_body,
            })
        } else {
            None
        };
        if finally.is_some() && self.check(&TokenKind::Catch) {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0637",
                    "a `catch` clause cannot follow `finally`",
                    self.peek().span,
                )
                .with_title("Catch Cannot Follow Finally")
                .with_help("place every `catch` before the single final `finally` clause"),
            );
            return None;
        }
        if catches.is_empty() && finally.is_none() {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0625",
                    "`try` requires at least one `catch` or `finally`",
                    body.span,
                )
                .with_title("Try Requires Catch Or Finally"),
            );
            return None;
        }
        let end = finally.as_ref().map_or_else(
            || catches.last().map_or(body.span.end, |catch| catch.span.end),
            |clause| clause.span.end,
        );
        Some(TryStmt {
            keyword_span,
            body,
            catches,
            finally,
            span: Span::new(keyword_span.start, end),
        })
    }

    fn parse_let_decl(&mut self) -> Option<Stmt> {
        let start = self.previous().span.start;
        self.parse_let_var_decl_after_let(start, "expected `;` after let declaration")
            .map(Stmt::VarDecl)
    }

    fn parse_let_var_decl_after_let(
        &mut self,
        start: usize,
        semicolon_message: &str,
    ) -> Option<VarDecl> {
        let writable = self.match_kind(&TokenKind::Writable);
        let (name, name_span) = self.expect_variable("expected variable name after `let`")?;
        let bindings = self.parse_local_bindings(name, name_span)?;
        self.expect(TokenKind::Equals, "expected `=` in let declaration")?;
        let initializer = self.parse_expression()?;
        if self.reject_disallowed_finally() {
            return None;
        }
        if self.reject_additional_group_initializer() {
            return None;
        }
        let end = self
            .expect(TokenKind::Semicolon, semicolon_message)?
            .span
            .end;

        Some(VarDecl {
            writable,
            ty: None,
            bindings,
            initializer,
            span: Span::new(start, end),
        })
    }

    fn parse_local_bindings(&mut self, name: String, span: Span) -> Option<Vec<VarBinding>> {
        let mut bindings = vec![VarBinding { name, span }];
        while self.match_kind(&TokenKind::Comma) {
            let comma = self.previous().span;
            if self.check(&TokenKind::Equals) {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0556",
                        "a grouped local declaration cannot end its binding list with a comma",
                        comma,
                    )
                    .with_title("Grouped Declaration Cannot Have A Trailing Comma")
                    .with_explanation(
                        "The first grouped-declaration form requires a variable after every comma.",
                    )
                    .with_help("remove the trailing comma before `=`"),
                );
                return None;
            }

            if self.check(&TokenKind::Writable) || self.check(&TokenKind::Readonly) {
                let modifier = self.advance().clone();
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0554",
                        "mutability is declared once for the complete local group",
                        modifier.span,
                    )
                    .with_title("Grouped Bindings Share One Mutability Mode")
                    .with_explanation(
                        "Every binding in a grouped declaration receives the mutability mode written before the first binding.",
                    )
                    .with_help("move the mutability modifier to the shared declaration prefix"),
                );
                return None;
            }

            if let Some(type_span) = self.probe_per_binding_type() {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0555",
                        "a declared type applies to the complete local group",
                        type_span,
                    )
                    .with_title("Grouped Bindings Share One Declared Type")
                    .with_explanation(
                        "Doria grouped declarations do not use C-style per-binding declarators.",
                    )
                    .with_help("write separate declarations when bindings need different types"),
                );
                return None;
            }

            let (name, span) =
                self.expect_variable("expected variable name after `,` in grouped declaration")?;
            bindings.push(VarBinding { name, span });
        }
        Some(bindings)
    }

    /// Recognize the complete ordinary type grammar without committing parser
    /// state. Grouped declarations reject a type after a comma, but the
    /// diagnostic must understand every type shape accepted elsewhere rather
    /// than maintaining a second, shallower lookahead grammar.
    fn probe_per_binding_type(&mut self) -> Option<Span> {
        if matches!(self.peek().kind, TokenKind::Variable(_)) {
            return None;
        }

        let checkpoint = self.checkpoint();
        let start = self.peek().span.start;
        let parsed = self.parse_type_ref();
        let end = self.previous().span.end;
        let followed_by_binding = matches!(self.peek().kind, TokenKind::Variable(_));

        self.restore_checkpoint(checkpoint);

        (parsed.is_some() && followed_by_binding).then(|| Span::new(start, end))
    }

    fn reject_additional_group_initializer(&mut self) -> bool {
        if !self.check(&TokenKind::Comma) {
            return false;
        }

        let span = self.advance().span;
        self.diagnostics.push(
            Diagnostic::new(
                "E0553",
                "a grouped local declaration has one initializer shared by every binding",
                span,
            )
            .with_title("Grouped Declarations Use One Shared Initializer")
            .with_explanation("Every binding in a grouped declaration begins with the same value.")
            .with_help("write separate declarations when the initial values differ"),
        );
        while !self.check(&TokenKind::Semicolon) && !self.is_at_end() {
            self.advance();
        }
        if self.check(&TokenKind::Semicolon) {
            self.advance();
        }
        true
    }

    fn reject_disallowed_finally(&mut self) -> bool {
        if !self.match_kind(&TokenKind::Finally) {
            return false;
        }
        let keyword_span = self.previous().span;
        self.diagnostics.push(
            Diagnostic::new(
                "P0021",
                "`finally` attaches only to `if`, `when`, `while`, or `do ... while`",
                keyword_span,
            )
            .with_title("Finally Is Not Available On This Construct")
            .with_help("attach cleanup to one of the supported control-flow constructs"),
        );
        let _ = self.parse_block();
        self.match_kind(&TokenKind::Semicolon);
        true
    }

    fn parse_loop_control_statement(
        &mut self,
        kind: TokenKind,
        unsupported_message: &'static str,
        missing_semicolon_message: &'static str,
    ) -> Option<Stmt> {
        let start = self.previous().span.start;
        if !self.check(&TokenKind::Semicolon) {
            self.error(unsupported_message, self.peek().span);
            return None;
        }

        let end = self
            .expect(TokenKind::Semicolon, missing_semicolon_message)?
            .span
            .end;
        match kind {
            TokenKind::Break => Some(Stmt::Break {
                span: Span::new(start, end),
            }),
            TokenKind::Continue => Some(Stmt::Continue {
                span: Span::new(start, end),
            }),
            _ => unreachable!("loop-control parser called for non-loop-control token"),
        }
    }

    fn parse_if_statement(&mut self) -> Option<IfStmt> {
        let start = self.previous().span.start;
        self.parse_if_statement_after_keyword(start, None, true)
    }

    fn parse_if_statement_after_keyword(
        &mut self,
        start: usize,
        given: Option<GivenPrelude>,
        allow_finally: bool,
    ) -> Option<IfStmt> {
        self.expect(TokenKind::LeftParen, "expected `(` after if")?;
        let condition = self.parse_expression()?;
        self.expect(TokenKind::RightParen, "expected `)` after if condition")?;
        let then_block = self.parse_block()?;
        let else_branch = if self.match_kind(&TokenKind::Else) {
            if self.match_kind(&TokenKind::If) {
                let nested_start = self.previous().span.start;
                Some(ElseBranch::If(Box::new(
                    self.parse_if_statement_after_keyword(nested_start, None, false)?,
                )))
            } else {
                Some(ElseBranch::Block(self.parse_block()?))
            }
        } else {
            None
        };
        let branch_end = else_branch
            .as_ref()
            .map(ElseBranch::span)
            .unwrap_or(then_block.span)
            .end;
        let finally = if allow_finally {
            self.parse_optional_finally()?
        } else {
            None
        };
        let end = finally
            .as_ref()
            .map_or(branch_end, |clause| clause.span.end);

        Some(IfStmt {
            given,
            condition,
            then_block,
            else_branch,
            finally,
            span: Span::new(start, end),
        })
    }

    fn parse_while(&mut self) -> Option<WhileStmt> {
        let start = self.previous().span.start;
        self.parse_while_after_keyword(start, None)
    }

    fn parse_while_after_keyword(
        &mut self,
        start: usize,
        given: Option<GivenPrelude>,
    ) -> Option<WhileStmt> {
        self.expect(TokenKind::LeftParen, "expected `(` after while")?;
        let condition = self.parse_expression()?;
        self.expect(TokenKind::RightParen, "expected `)` after while condition")?;
        let body = self.parse_block()?;
        let finally = self.parse_optional_finally()?;
        let end = finally
            .as_ref()
            .map_or(body.span.end, |clause| clause.span.end);
        Some(WhileStmt {
            given,
            condition,
            body,
            finally,
            span: Span::new(start, end),
        })
    }

    fn parse_do_while(&mut self) -> Option<DoWhileStmt> {
        let start = self.previous().span.start;
        let body = self.parse_block()?;
        self.expect(TokenKind::While, "expected `while` after `do` block")?;
        self.expect(TokenKind::LeftParen, "expected `(` after while")?;
        let condition = self.parse_expression()?;
        let condition_end = self
            .expect(
                TokenKind::RightParen,
                "expected `)` after do-while condition",
            )?
            .span
            .end;

        let (semicolon_span, finally, end) = if self.check(&TokenKind::Finally) {
            let finally = self.parse_optional_finally()?.expect("checked finally");
            let end = finally.span.end;
            (None, Some(finally), end)
        } else if self.match_kind(&TokenKind::Semicolon) {
            let semicolon = self.previous().span;
            (Some(semicolon), None, semicolon.end)
        } else {
            let insertion = Span::new(condition_end, condition_end);
            self.diagnostics.push(
                Diagnostic::new("P0018", "expected `;` after do-while condition", insertion)
                    .with_help("terminate `do ... while` with `;` when no `finally` clause follows")
                    .with_fix(insertion, ";"),
            );
            (None, None, condition_end)
        };

        Some(DoWhileStmt {
            body,
            condition,
            semicolon_span,
            finally,
            span: Span::new(start, end),
        })
    }

    fn parse_given_statement(&mut self) -> Option<Stmt> {
        let start = self.previous().span.start;
        let given = self.parse_given_prelude(start)?;
        if self.match_kind(&TokenKind::If) {
            return self
                .parse_if_statement_after_keyword(start, Some(given), true)
                .map(Stmt::If);
        }
        if self.match_kind(&TokenKind::While) {
            return self
                .parse_while_after_keyword(start, Some(given))
                .map(Stmt::While);
        }
        if self.match_kind(&TokenKind::When) {
            let expr = self.parse_when_expression(start, Some(given))?;
            let end = self
                .expect(
                    TokenKind::Semicolon,
                    "expected `;` after `given ... when` expression statement",
                )?
                .span
                .end;
            return Some(Stmt::Expr {
                span: Span::new(start, end),
                expr,
            });
        }
        if self.check(&TokenKind::Do) {
            self.diagnostics.push(
                Diagnostic::new(
                    "P0019",
                    "`given` does not attach to `do ... while`",
                    self.peek().span,
                )
                .with_help("put setup before the `do` statement"),
            );
            return None;
        }

        self.diagnostics.push(
            Diagnostic::new(
                "P0020",
                "`given` must attach to `if`, `when`, or `while`",
                given.span,
            )
            .with_help("place the governed control-flow construct immediately after the block"),
        );
        None
    }

    fn parse_given_prelude(&mut self, start: usize) -> Option<GivenPrelude> {
        let block = self.parse_block()?;
        Some(GivenPrelude {
            span: Span::new(start, block.span.end),
            block,
        })
    }

    fn parse_optional_finally(&mut self) -> Option<Option<ControlFlowFinally>> {
        if !self.match_kind(&TokenKind::Finally) {
            return Some(None);
        }
        let keyword_span = self.previous().span;
        let block = self.parse_block()?;
        Some(Some(ControlFlowFinally {
            keyword_span,
            span: keyword_span.merge(block.span),
            block,
        }))
    }

    fn parse_for(&mut self) -> Option<ForStmt> {
        let start = self.previous().span.start;
        self.expect(TokenKind::LeftParen, "expected `(` after for")?;

        let initializer = if self.match_kind(&TokenKind::Semicolon) {
            None
        } else if self.match_kind(&TokenKind::Let) {
            let start = self.previous().span.start;
            Some(ForInitializer::VarDecl(self.parse_let_var_decl_after_let(
                start,
                "expected `;` after for initializer",
            )?))
        } else {
            let target = self.parse_expression()?;
            let Some(op) = self.parse_assignment_op() else {
                self.error(
                    "expected assignment or `let` declaration in for initializer",
                    target.span(),
                );
                return None;
            };
            let value = self.parse_expression()?;
            let end = self
                .expect(TokenKind::Semicolon, "expected `;` after for initializer")?
                .span
                .end;
            Some(ForInitializer::Assignment(Assignment {
                span: Span::new(target.span().start, end),
                target,
                op,
                value,
            }))
        };

        let condition = if self.match_kind(&TokenKind::Semicolon) {
            None
        } else {
            let condition = self.parse_expression()?;
            self.expect(TokenKind::Semicolon, "expected `;` after for condition")?;
            Some(condition)
        };

        let increment = if self.check(&TokenKind::RightParen) {
            None
        } else {
            Some(self.parse_for_increment()?)
        };

        self.expect(TokenKind::RightParen, "expected `)` after for clauses")?;
        let body = self.parse_block()?;
        let span = Span::new(start, body.span.end);
        Some(ForStmt {
            initializer,
            condition,
            increment,
            body,
            span,
        })
    }

    fn parse_for_increment(&mut self) -> Option<ForIncrement> {
        if self.check(&TokenKind::PlusPlus) || self.check(&TokenKind::MinusMinus) {
            return self.parse_pre_increment(false).map(ForIncrement::Increment);
        }

        let target = self.parse_expression()?;
        if self.check(&TokenKind::PlusPlus) || self.check(&TokenKind::MinusMinus) {
            return self
                .parse_post_increment(target, false)
                .map(ForIncrement::Increment);
        }

        if let Some(op) = self.parse_assignment_op() {
            let start = target.span().start;
            let value = self.parse_expression()?;
            let span = Span::new(start, value.span().end);
            return Some(ForIncrement::Assignment(Assignment {
                target,
                op,
                value,
                span,
            }));
        }

        self.error(
            "expected increment, decrement, or assignment in for increment",
            target.span(),
        );
        None
    }

    fn parse_pre_increment_statement(&mut self) -> Option<Stmt> {
        self.parse_pre_increment(true).map(Stmt::Increment)
    }

    fn parse_post_increment_statement(&mut self, target: Expr) -> Option<Stmt> {
        self.parse_post_increment(target, true).map(Stmt::Increment)
    }

    fn parse_pre_increment(&mut self, expect_semicolon: bool) -> Option<IncrementStmt> {
        let token = self.advance().clone();
        let (op, op_name) = match token.kind {
            TokenKind::PlusPlus => (IncrementOp::Increment, "++"),
            TokenKind::MinusMinus => (IncrementOp::Decrement, "--"),
            _ => unreachable!("pre-increment parser called without increment token"),
        };
        let target = self.parse_postfix()?;
        let end = if expect_semicolon {
            self.expect(
                TokenKind::Semicolon,
                &format!("expected `;` after `{op_name}` statement"),
            )?
            .span
            .end
        } else {
            target.span().end
        };
        Some(IncrementStmt {
            target,
            op,
            position: IncrementPosition::Pre,
            span: Span::new(token.span.start, end),
        })
    }

    fn parse_post_increment(
        &mut self,
        target: Expr,
        expect_semicolon: bool,
    ) -> Option<IncrementStmt> {
        let token = self.advance().clone();
        let (op, op_name) = match token.kind {
            TokenKind::PlusPlus => (IncrementOp::Increment, "++"),
            TokenKind::MinusMinus => (IncrementOp::Decrement, "--"),
            _ => unreachable!("post-increment parser called without increment token"),
        };
        let end = if expect_semicolon {
            self.expect(
                TokenKind::Semicolon,
                &format!("expected `;` after `{op_name}` statement"),
            )?
            .span
            .end
        } else {
            token.span.end
        };
        Some(IncrementStmt {
            span: Span::new(target.span().start, end),
            target,
            op,
            position: IncrementPosition::Post,
        })
    }

    fn parse_foreach(&mut self) -> Option<Stmt> {
        let start = self.previous().span.start;
        self.expect(TokenKind::LeftParen, "expected `(` after foreach")?;
        let iterable = self.parse_expression()?;
        self.expect(TokenKind::As, "expected `as` in foreach")?;
        let first = self.parse_foreach_binding()?;
        let (key, value) = if self.match_kind(&TokenKind::FatArrow) {
            let value = self.parse_foreach_binding()?;
            (Some(first), value)
        } else {
            (None, first)
        };
        self.expect(TokenKind::RightParen, "expected `)` after foreach bindings")?;
        let body = self.parse_block()?;
        let span = Span::new(start, body.span.end);
        Some(Stmt::Foreach(ForeachStmt {
            iterable,
            key,
            value,
            body,
            span,
        }))
    }

    fn parse_foreach_binding(&mut self) -> Option<ForeachBinding> {
        let writable = self.match_kind(&TokenKind::Writable);
        if let Some((name, span)) = self.consume_variable() {
            return Some(ForeachBinding {
                writable,
                ty: None,
                name,
                span,
            });
        }

        let ty = self.parse_type_ref()?;
        let (name, span) = self.expect_variable("expected foreach binding variable")?;
        Some(ForeachBinding {
            writable,
            ty: Some(ty),
            name,
            span,
        })
    }

    fn parse_expression(&mut self) -> Option<Expr> {
        self.parse_ternary()
    }

    fn parse_ternary(&mut self) -> Option<Expr> {
        let condition = self.parse_range()?;
        if !self.match_kind(&TokenKind::Question) {
            return Some(condition);
        }

        let question = self.previous().span;
        if self.check(&TokenKind::Colon) {
            self.error(
                "Doria does not support the short ternary `?:`; use `??` for null fallback or the full `? :` form for a bool condition",
                question.merge(self.peek().span),
            );
            self.advance();
            return self.parse_ternary();
        }

        let when_true = self.parse_expression()?;
        let colon = self
            .expect(TokenKind::Colon, "expected `:` in ternary expression")?
            .span;
        let when_false = self.parse_ternary()?;
        let span = condition.span().merge(when_false.span());
        Some(Expr::Match {
            scrutinee: Box::new(condition),
            mode: MatchMode::Borrowed,
            arms: vec![
                MatchArm {
                    pattern: MatchPattern::Expression(Expr::Bool {
                        value: true,
                        span: question,
                    }),
                    guard: None,
                    span: question.merge(when_true.span()),
                    value: when_true,
                },
                MatchArm {
                    pattern: MatchPattern::Expression(Expr::Bool {
                        value: false,
                        span: colon,
                    }),
                    guard: None,
                    span: colon.merge(when_false.span()),
                    value: when_false,
                },
            ],
            origin: MatchOrigin::Ternary,
            span,
        })
    }

    fn parse_range(&mut self) -> Option<Expr> {
        let start = self.parse_binary(1)?;
        let inclusive = if self.match_kind(&TokenKind::DotDot) {
            true
        } else if self.match_kind(&TokenKind::DotDotLess) {
            false
        } else {
            return Some(start);
        };

        let end = self.parse_binary(1)?;
        let span = start.span().merge(end.span());
        Some(Expr::Range {
            start: Box::new(start),
            end: Box::new(end),
            inclusive,
            span,
        })
    }

    fn parse_binary(&mut self, min_prec: u8) -> Option<Expr> {
        let mut left = self.parse_unary()?;

        loop {
            if self.check(&TokenKind::Is) {
                const IS_PRECEDENCE: u8 = 8;
                if IS_PRECEDENCE < min_prec {
                    break;
                }
                self.advance();
                let ty = self.parse_type_ref()?;
                let span = left.span().merge(self.previous().span);
                left = Expr::IsType {
                    expr: Box::new(left),
                    ty,
                    span,
                };
                continue;
            }

            let Some((op, prec)) = self.current_binary_op() else {
                break;
            };
            if prec < min_prec {
                break;
            }
            self.advance();
            let right = self.parse_binary(prec + 1)?;
            let span = left.span().merge(right.span());
            if Self::xor_mix_is_ambiguous(&op, &left, &right) {
                self.error(
                    "ambiguous `xor` expression; keep `xor` separate from other logical operators in this compiler slice",
                    span,
                );
            }
            left = Expr::Binary {
                left: Box::new(left),
                op,
                right: Box::new(right),
                span,
            };
        }

        Some(left)
    }

    fn parse_unary(&mut self) -> Option<Expr> {
        let op = if self.match_kind(&TokenKind::Bang) || self.match_kind(&TokenKind::Not) {
            Some(UnaryOp::Not)
        } else if self.match_kind(&TokenKind::Minus) {
            Some(UnaryOp::Negate)
        } else if self.match_kind(&TokenKind::Tilde) {
            Some(UnaryOp::BitwiseNot)
        } else {
            None
        };

        if let Some(op) = op {
            let op_span = self.previous().span;
            let expr = self.parse_unary()?;
            let span = op_span.merge(expr.span());
            return Some(Expr::Unary {
                op,
                expr: Box::new(expr),
                span,
            });
        }

        self.parse_postfix()
    }

    fn xor_mix_is_ambiguous(op: &BinaryOp, left: &Expr, right: &Expr) -> bool {
        match op {
            BinaryOp::Xor => {
                Self::has_unparenthesized_logical_binary(left)
                    || Self::has_unparenthesized_logical_binary(right)
            }
            BinaryOp::And | BinaryOp::Or => {
                Self::has_unparenthesized_xor_binary(left)
                    || Self::has_unparenthesized_xor_binary(right)
            }
            _ => false,
        }
    }

    fn has_unparenthesized_logical_binary(expr: &Expr) -> bool {
        match expr {
            Expr::Binary {
                op, left, right, ..
            } => {
                matches!(op, BinaryOp::And | BinaryOp::Or | BinaryOp::Xor)
                    || Self::has_unparenthesized_logical_binary(left)
                    || Self::has_unparenthesized_logical_binary(right)
            }
            Expr::Grouped { .. } => false,
            Expr::Unary { expr, .. } => Self::has_unparenthesized_logical_binary(expr),
            _ => false,
        }
    }

    fn has_unparenthesized_xor_binary(expr: &Expr) -> bool {
        match expr {
            Expr::Binary {
                op, left, right, ..
            } => {
                matches!(op, BinaryOp::Xor)
                    || Self::has_unparenthesized_xor_binary(left)
                    || Self::has_unparenthesized_xor_binary(right)
            }
            Expr::Grouped { .. } => false,
            Expr::Unary { expr, .. } => Self::has_unparenthesized_xor_binary(expr),
            _ => false,
        }
    }

    fn parse_postfix(&mut self) -> Option<Expr> {
        let mut expr = self.parse_primary()?;

        loop {
            let null_safe = if self.match_kind(&TokenKind::Arrow) {
                Some(false)
            } else if self.match_kind(&TokenKind::QuestionArrow) {
                Some(true)
            } else {
                None
            };
            if let Some(null_safe) = null_safe {
                let property =
                    self.expect_identifier("expected property or method name after member access")?;
                let member_span = self.previous().span;
                if self.match_kind(&TokenKind::LeftParen) {
                    let argument_list_start = self.previous().span.start;
                    let args = self.parse_argument_list_after_open()?;
                    let argument_list_span =
                        Span::new(argument_list_start, self.previous().span.end);
                    let span = expr.span().merge(self.previous().span);
                    expr = Expr::MethodCall {
                        object: Box::new(expr),
                        method: property,
                        member_span,
                        args,
                        argument_list_span,
                        null_safe,
                        span,
                    };
                } else {
                    let span = expr.span().merge(self.previous().span);
                    expr = Expr::PropertyAccess {
                        object: Box::new(expr),
                        property,
                        member_span,
                        null_safe,
                        span,
                    };
                }
                continue;
            }

            if self.match_kind(&TokenKind::LeftBracket) {
                let index = self.parse_expression()?;
                let end = self
                    .expect(
                        TokenKind::RightBracket,
                        "expected `]` after collection index",
                    )?
                    .span
                    .end;
                let start = expr.span().start;
                expr = Expr::Index {
                    collection: Box::new(expr),
                    index: Box::new(index),
                    span: Span::new(start, end),
                };
                continue;
            }

            if self.match_kind(&TokenKind::LeftParen) {
                let open_span = self.previous().span;
                let callee_span = expr.span();
                let args = self.parse_argument_list_after_open()?;
                let close_span = self.previous().span;
                match expr {
                    Expr::Identifier { name, span } => {
                        expr = Expr::FunctionCall {
                            name,
                            args,
                            span: Span::new(span.start, close_span.end),
                        };
                    }
                    callee => {
                        if let Some(named) = args.iter().find_map(|argument| argument.name.as_ref())
                        {
                            self.diagnostics.push(
                                Diagnostic::new(
                                    "P0001",
                                    "callable-value invocation uses positional arguments because structural function types contain no parameter names",
                                    named.span,
                                )
                                .with_title("Callable Value Argument Cannot Be Named")
                                .with_help("remove the argument name and pass the value positionally"),
                            );
                        }
                        expr = Expr::CallableCall {
                            callee: Box::new(callee),
                            open_span,
                            args,
                            close_span,
                            argument_list_span: open_span.merge(close_span),
                            span: callee_span.merge(close_span),
                        };
                    }
                }
                continue;
            }

            break;
        }

        Some(expr)
    }

    fn parse_primary(&mut self) -> Option<Expr> {
        if self.is_at_end() {
            let span = if self.current == 0 {
                self.peek().span
            } else {
                self.previous().span
            };
            self.error("expected expression", span);
            return None;
        }
        let token = self.advance().clone();
        match token.kind {
            TokenKind::Throw => {
                self.diagnostics.push(
                    Diagnostic::new(
                        "E0636",
                        "`throw` is a statement and cannot be used as a value",
                        token.span,
                    )
                    .with_title("Throw Is A Statement")
                    .with_help("write `throw $error;` as its own statement"),
                );
                None
            }
            TokenKind::Variable(name) => {
                if name == "this" {
                    Some(Expr::This { span: token.span })
                } else {
                    Some(Expr::Variable {
                        name,
                        span: token.span,
                    })
                }
            }
            TokenKind::Identifier(name) => {
                let name = self.finish_qualified_name(
                    name,
                    "expected name segment after namespace separator",
                )?;
                let name_span = Span::new(token.span.start, self.previous().span.end);
                if self.match_kind(&TokenKind::DoubleColon) {
                    self.parse_scoped_access(
                        StaticQualifier::Class(name),
                        name_span,
                        token.span.start,
                    )
                } else {
                    Some(Expr::Identifier {
                        name,
                        span: name_span,
                    })
                }
            }
            TokenKind::SelfType if self.match_kind(&TokenKind::DoubleColon) => {
                self.parse_scoped_access(StaticQualifier::SelfType, token.span, token.span.start)
            }
            TokenKind::Parent if self.match_kind(&TokenKind::DoubleColon) => {
                self.parse_scoped_access(StaticQualifier::Parent, token.span, token.span.start)
            }
            TokenKind::Static if self.match_kind(&TokenKind::DoubleColon) => self
                .parse_scoped_access(StaticQualifier::InvalidStatic, token.span, token.span.start),
            TokenKind::StringLiteral { value, raw, quote } => {
                self.parse_string_literal(value, raw, quote, token.span)
            }
            TokenKind::IntLiteral(value) => Some(Expr::Int {
                value,
                span: token.span,
            }),
            TokenKind::FloatLiteral(value) => Some(Expr::Float {
                value,
                span: token.span,
            }),
            TokenKind::True => Some(Expr::Bool {
                value: true,
                span: token.span,
            }),
            TokenKind::False => Some(Expr::Bool {
                value: false,
                span: token.span,
            }),
            TokenKind::Null => Some(Expr::Null { span: token.span }),
            TokenKind::Fn => self.parse_arrow_closure(token.span),
            TokenKind::Function => self.parse_anonymous_block_closure(token.span),
            TokenKind::With => {
                self.diagnostics.push(
                    Diagnostic::new(
                        "P0001",
                        "a closure capture clause must appear after its parameter list and before its body",
                        token.span,
                    )
                    .with_title("Closure Capture Clause Is In The Wrong Position"),
                );
                None
            }
            TokenKind::When => self.parse_when_expression(token.span.start, None),
            TokenKind::Given => {
                let given = self.parse_given_prelude(token.span.start)?;
                self.expect(TokenKind::When, "expected `when` after `given` block")?;
                self.parse_when_expression(token.span.start, Some(given))
            }
            TokenKind::Match => self.parse_match_expression(token.span.start),
            TokenKind::New => self.parse_new(token.span.start, false),
            TokenKind::Shared => self.parse_shared_new(token.span.start),
            TokenKind::LeftBracket => self.parse_array(token.span.start),
            TokenKind::LeftParen => {
                let start = token.span.start;
                let expr = self.parse_expression()?;
                let end = self
                    .expect(TokenKind::RightParen, "expected `)` after expression")?
                    .span
                    .end;
                Some(Expr::Grouped {
                    expr: Box::new(expr),
                    span: Span::new(start, end),
                })
            }
            _ => {
                self.error("expected expression", token.span);
                None
            }
        }
    }

    fn parse_match_expression(&mut self, start: usize) -> Option<Expr> {
        self.expect(TokenKind::LeftParen, "expected `(` after `match`")?;
        let mode = if self.match_kind(&TokenKind::Take) {
            MatchMode::Consumed {
                take_span: self.previous().span,
            }
        } else {
            MatchMode::Borrowed
        };
        if self.match_kind(&TokenKind::Writable) {
            let span = self.previous().span;
            self.diagnostics.push(
                Diagnostic::new(
                    "E0602",
                    "Doria v1 does not support writable match scrutinees",
                    span,
                )
                .with_title("Writable Match Is Not Supported")
                .with_help(
                    "consume the whole value with `match (take $value)` and assign the result to a writable destination",
                )
                .with_fix(span, ""),
            );
        }
        let scrutinee = self.parse_expression()?;
        self.expect(TokenKind::RightParen, "expected `)` after match value")?;
        self.expect(TokenKind::LeftBrace, "expected `{` before match arms")?;

        let mut arms = Vec::new();
        while !self.check(&TokenKind::RightBrace) && !self.is_at_end() {
            let arm_start = self.peek().span.start;
            let pattern = self.parse_match_pattern()?;
            let guard = self.parse_match_guard()?;
            self.expect(TokenKind::FatArrow, "expected `=>` after match pattern")?;
            let value = self.parse_expression()?;
            let arm_end = value.span().end;
            arms.push(MatchArm {
                pattern,
                guard,
                value,
                span: Span::new(arm_start, arm_end),
            });
            if !self.match_kind(&TokenKind::Comma) {
                break;
            }
        }
        let end = self
            .expect(TokenKind::RightBrace, "expected `}` after match arms")?
            .span
            .end;
        Some(Expr::Match {
            scrutinee: Box::new(scrutinee),
            mode,
            arms,
            origin: MatchOrigin::Match,
            span: Span::new(start, end),
        })
    }

    fn parse_match_guard(&mut self) -> Option<Option<MatchGuard>> {
        let keyword = if self.match_kind(&TokenKind::If) {
            Some(self.previous().clone())
        } else if self.match_kind(&TokenKind::When) {
            let token = self.previous().clone();
            self.diagnostics.push(
                Diagnostic::new("E0596", "`when` is not a match guard; use `if`", token.span)
                    .with_title("Match Guard Uses The Wrong Keyword")
                    .with_fix(token.span, "if"),
            );
            Some(token)
        } else if matches!(&self.peek().kind, TokenKind::Identifier(name) if name == "where") {
            let token = self.advance().clone();
            self.diagnostics.push(
                Diagnostic::new(
                    "E0596",
                    "`where` is not a match guard; use `if`",
                    token.span,
                )
                .with_title("Match Guard Uses The Wrong Keyword")
                .with_fix(token.span, "if"),
            );
            Some(token)
        } else {
            None
        };
        let Some(keyword) = keyword else {
            return Some(None);
        };
        let condition = self.parse_expression()?;
        Some(Some(MatchGuard {
            span: keyword.span.merge(condition.span()),
            keyword_span: keyword.span,
            condition,
        }))
    }

    fn parse_match_pattern(&mut self) -> Option<MatchPattern> {
        if self.match_kind(&TokenKind::Writable) {
            let span = self.previous().span;
            self.diagnostics.push(
                Diagnostic::new(
                    "E0604",
                    "Doria v1 does not support writable match patterns",
                    span,
                )
                .with_title("Writable Payload Pattern Is Not Supported")
                .with_help(
                    "consume the matched value into a writable destination when mutation is required",
                )
                .with_fix(span, ""),
            );
        }
        if self.match_kind(&TokenKind::Default) {
            return Some(MatchPattern::Default {
                span: self.previous().span,
            });
        }

        let enum_case_checkpoint = self.current;
        if let TokenKind::Identifier(name) = self.peek().kind.clone() {
            let qualifier_token = self.advance().clone();
            let name = self.finish_qualified_name(
                name,
                "expected enum-name segment after namespace separator",
            )?;
            let qualifier_span = Span::new(qualifier_token.span.start, self.previous().span.end);
            if self.match_kind(&TokenKind::DoubleColon) {
                let case_token = self.advance().clone();
                let TokenKind::Identifier(case) = case_token.kind else {
                    self.error("expected enum case after `::`", case_token.span);
                    return None;
                };
                let mut bindings = None;
                let mut end = case_token.span.end;
                if self.match_kind(&TokenKind::LeftParen) {
                    let mut parsed = Vec::new();
                    if !self.check(&TokenKind::RightParen) {
                        loop {
                            if self.match_kind(&TokenKind::Take) {
                                let span = self.previous().span;
                                self.diagnostics.push(
                                    Diagnostic::new(
                                        "E0603",
                                        "payload-level `take` is not allowed in match patterns",
                                        span,
                                    )
                                    .with_title("Payload-Level Take Is Not Allowed")
                                    .with_help(
                                        "consume the complete value with `match (take $value)` instead",
                                    )
                                    .with_fix(span, ""),
                                );
                            } else if self.match_kind(&TokenKind::Writable) {
                                let span = self.previous().span;
                                self.diagnostics.push(
                                    Diagnostic::new(
                                        "E0604",
                                        "Doria v1 does not support writable payload patterns",
                                        span,
                                    )
                                    .with_title("Writable Payload Pattern Is Not Supported")
                                    .with_help(
                                        "consume the enum into a writable destination when mutation is required",
                                    )
                                    .with_fix(span, ""),
                                );
                            }
                            let (name, span) =
                                self.expect_variable("expected payload binding variable")?;
                            parsed.push(MatchBinding { name, span });
                            if !self.match_kind(&TokenKind::Comma) {
                                break;
                            }
                        }
                    }
                    end = self
                        .expect(TokenKind::RightParen, "expected `)` after payload bindings")?
                        .span
                        .end;
                    bindings = Some(parsed);
                }
                return Some(MatchPattern::EnumCase {
                    qualifier: name,
                    qualifier_span,
                    case,
                    case_span: case_token.span,
                    bindings,
                    span: Span::new(qualifier_token.span.start, end),
                });
            }
            self.current = enum_case_checkpoint;
        }

        let checkpoint = self.checkpoint();
        if self.can_start_match_type_binding() {
            if let Some(ty) = self.parse_type_ref() {
                if let Some((name, binding_span)) = self.consume_variable() {
                    return Some(MatchPattern::TypeBinding {
                        ty,
                        binding: MatchBinding {
                            name,
                            span: binding_span,
                        },
                        span: Span::new(
                            self.tokens[checkpoint.current].span.start,
                            binding_span.end,
                        ),
                    });
                }
            }
            self.restore_checkpoint(checkpoint);
        }

        self.parse_expression().map(MatchPattern::Expression)
    }

    fn can_start_match_type_binding(&self) -> bool {
        self.can_start_type_ref()
    }

    fn parse_scoped_access(
        &mut self,
        qualifier: StaticQualifier,
        qualifier_span: Span,
        start: usize,
    ) -> Option<Expr> {
        let token = self.advance().clone();
        let (member, member_sigil_span) = match token.kind {
            TokenKind::Identifier(name) => (name, None),
            TokenKind::Variable(name) => (
                name,
                Some(Span::new(token.span.start, token.span.start + 1)),
            ),
            _ => {
                self.error("expected member name after `::`", token.span);
                return None;
            }
        };

        if self.match_kind(&TokenKind::LeftParen) {
            let argument_list_start = self.previous().span.start;
            let args = self.parse_argument_list_after_open()?;
            let argument_list_span = Span::new(argument_list_start, self.previous().span.end);
            Some(Expr::StaticCall {
                qualifier,
                qualifier_span,
                method: member,
                member_span: token.span,
                member_sigil_span,
                args,
                argument_list_span,
                span: Span::new(start, self.previous().span.end),
            })
        } else {
            Some(Expr::StaticMember {
                qualifier,
                qualifier_span,
                member,
                member_span: token.span,
                member_sigil_span,
                span: Span::new(start, token.span.end),
            })
        }
    }

    fn parse_string_literal(
        &mut self,
        value: String,
        raw: String,
        quote: StringQuoteKind,
        span: Span,
    ) -> Option<Expr> {
        if matches!(quote, StringQuoteKind::Single) {
            return Some(Expr::String { value, span });
        }

        let mut parts = Vec::new();
        let mut text = String::new();
        let mut cursor = 0;
        let mut text_start = 0;
        let mut has_interpolation = false;

        while cursor < raw.len() {
            let character = raw[cursor..]
                .chars()
                .next()
                .expect("cursor is on a UTF-8 boundary");
            if character == '\\' {
                cursor += 1;
                let Some(escaped) = raw[cursor..].chars().next() else {
                    text.push('\\');
                    break;
                };
                cursor += escaped.len_utf8();
                if let Some(decoded) = decode_escape(escaped) {
                    text.push(decoded);
                } else {
                    text.push('\\');
                    text.push(escaped);
                }
                continue;
            }

            if character != '{' {
                text.push(character);
                cursor += character.len_utf8();
                continue;
            }

            let open = cursor;
            let Some(close) = interpolation_close(&raw, open) else {
                self.error(
                    "unterminated string interpolation",
                    Span::new(span.start + 1 + open, span.end.saturating_sub(1)),
                );
                return None;
            };
            if !text.is_empty() {
                parts.push(InterpolatedStringPart::Text {
                    value: std::mem::take(&mut text),
                    span: Span::new(span.start + 1 + text_start, span.start + 1 + open),
                });
            }

            let inner_start = open + 1;
            let inner = &raw[inner_start..close];
            let inner_span = Span::new(span.start + 1 + inner_start, span.start + 1 + close);
            if inner.trim().is_empty() {
                self.error("empty string interpolation", inner_span);
                return None;
            }

            let expr = self.parse_interpolation_expr(inner, inner_span, open, span)?;
            parts.push(InterpolatedStringPart::Expr(expr));
            has_interpolation = true;
            cursor = close + 1;
            text_start = cursor;
        }

        if !has_interpolation {
            return Some(Expr::String { value: text, span });
        }
        if !text.is_empty() {
            parts.push(InterpolatedStringPart::Text {
                value: text,
                span: Span::new(span.start + 1 + text_start, span.start + 1 + raw.len()),
            });
        }
        Some(Expr::InterpolatedString { parts, span })
    }

    fn parse_interpolation_expr(
        &mut self,
        inner: &str,
        inner_span: Span,
        open_offset: usize,
        string_span: Span,
    ) -> Option<Expr> {
        let opening_brace_span = Span::new(
            string_span.start + 1 + open_offset,
            string_span.start + 2 + open_offset,
        );
        if inner.trim_start().starts_with('{') {
            self.report_literal_open_brace(opening_brace_span);
            return None;
        }

        let fragment = SourceFile::new("<interpolation>", inner);
        let mut tokens = match Lexer::new(&fragment).lex() {
            Ok(tokens) => tokens,
            Err(mut diagnostics) => {
                for diagnostic in &mut diagnostics {
                    diagnostic.span.start += inner_span.start;
                    diagnostic.span.end += inner_span.start;
                }
                self.diagnostics.extend(diagnostics);
                return None;
            }
        };
        for token in &mut tokens {
            token.span.start += inner_span.start;
            token.span.end += inner_span.start;
        }
        if tokens
            .first()
            .is_some_and(|token| matches!(token.kind, TokenKind::Eof))
        {
            self.error("expected expression", tokens[0].span);
            return None;
        }

        let mut nested = Parser::new(tokens);
        let expr = nested.parse_expression();
        if expr.is_some() && !nested.is_at_end() {
            let unexpected = nested.peek().clone();
            nested.error(
                format!(
                    "unexpected {} after interpolation expression",
                    token_name(&unexpected.kind)
                ),
                unexpected.span,
            );
        }
        if !nested.diagnostics.is_empty() {
            self.diagnostics.extend(nested.diagnostics);
            return None;
        }

        let expr = expr?;
        if Self::contains_bare_identifier(&expr) {
            self.report_literal_open_brace(opening_brace_span);
            return None;
        }

        Some(expr)
    }

    fn contains_bare_identifier(expr: &Expr) -> bool {
        match expr {
            Expr::Identifier { name, .. } => !name.contains('\\'),
            Expr::InterpolatedString { parts, .. } => parts.iter().any(|part| match part {
                InterpolatedStringPart::Text { .. } => false,
                InterpolatedStringPart::Expr(expr) => Self::contains_bare_identifier(expr),
            }),
            Expr::Array { elements, .. } => elements.iter().any(|element| {
                element
                    .key
                    .as_ref()
                    .is_some_and(Self::contains_bare_identifier)
                    || Self::contains_bare_identifier(&element.value)
            }),
            Expr::ArrayRepeat { value, count, .. } => {
                Self::contains_bare_identifier(value) || Self::contains_bare_identifier(count)
            }
            Expr::Index {
                collection, index, ..
            } => {
                Self::contains_bare_identifier(collection) || Self::contains_bare_identifier(index)
            }
            Expr::PropertyAccess { object, .. } => Self::contains_bare_identifier(object),
            Expr::MethodCall { object, args, .. } => {
                Self::contains_bare_identifier(object)
                    || args
                        .iter()
                        .any(|arg| Self::contains_bare_identifier(&arg.value))
            }
            Expr::FunctionCall { args, .. }
            | Expr::StaticCall { args, .. }
            | Expr::New { args, .. } => args
                .iter()
                .any(|arg| Self::contains_bare_identifier(&arg.value)),
            Expr::CallableCall { callee, args, .. } => {
                Self::contains_bare_identifier(callee)
                    || args
                        .iter()
                        .any(|arg| Self::contains_bare_identifier(&arg.value))
            }
            Expr::Grouped { expr, .. } | Expr::Unary { expr, .. } => {
                Self::contains_bare_identifier(expr)
            }
            Expr::IsType { expr, .. } => Self::contains_bare_identifier(expr),
            Expr::Binary { left, right, .. } => {
                Self::contains_bare_identifier(left) || Self::contains_bare_identifier(right)
            }
            Expr::Range { start, end, .. } => {
                Self::contains_bare_identifier(start) || Self::contains_bare_identifier(end)
            }
            Expr::Closure(_) | Expr::Match { .. } | Expr::When(_) => false,
            Expr::Variable { .. }
            | Expr::This { .. }
            | Expr::StaticMember { .. }
            | Expr::String { .. }
            | Expr::Int { .. }
            | Expr::Float { .. }
            | Expr::Bool { .. }
            | Expr::Null { .. } => false,
        }
    }

    fn report_literal_open_brace(&mut self, span: Span) {
        self.diagnostics.push(
            Diagnostic::new(
                "P0002",
                "unescaped `{` does not begin a valid interpolation expression",
                span,
            )
            .with_help("write `\\{` for a literal brace")
            .with_fix(span, "\\{"),
        );
    }

    /// `shared` is a construction modifier only (records 0005 and 0106): the sole
    /// accepted form is `shared new T(...)`. Every other continuation is rejected
    /// here with a migration diagnostic rather than a bare unexpected-token error.
    fn parse_shared_new(&mut self, start: usize) -> Option<Expr> {
        if self.match_kind(&TokenKind::New) {
            return self.parse_new(start, true);
        }

        let next = self.peek().clone();
        let span = Span::new(start, next.span.end);
        if matches!(next.kind, TokenKind::Writable) {
            self.diagnostics.push(
                Diagnostic::new(
                    "E0540",
                    "`shared writable new` Is Not Doria Syntax",
                    span,
                )
                .with_help(
                    "shared ownership picks its family at construction: use `shared new T(...)` for `SharedReference<T>`, or `new WritableSharedReference(new T(...))` for the writable family",
                ),
            );
            return None;
        }

        self.diagnostics.push(
            Diagnostic::new(
                "E0541",
                "`shared` Is A Construction Modifier, Not A Declaration Modifier",
                span,
            )
            .with_help(
                "write the type as `SharedReference<T>` and construct with `shared new T(...)`; `shared T $value` is superseded",
            ),
        );
        None
    }

    fn parse_new(&mut self, start: usize, shared: bool) -> Option<Expr> {
        let type_start = self.peek().span.start;
        let class_type = self.parse_type_ref()?;
        let type_span = Span::new(type_start, self.previous().span.end);
        if class_type.nullable || class_type.name == "[]" {
            self.error("`new` requires a non-nullable class type", type_span);
        }
        self.expect(TokenKind::LeftParen, "expected `(` after class name")?;
        let args = self.parse_argument_list_after_open()?;
        let span = Span::new(start, self.previous().span.end);
        Some(Expr::New {
            class_type,
            args,
            shared,
            span,
        })
    }

    fn parse_array(&mut self, start: usize) -> Option<Expr> {
        let mut elements = Vec::new();
        if !self.check(&TokenKind::RightBracket) {
            let first = self.parse_expression()?;
            if self.match_kind(&TokenKind::Semicolon) {
                let count = self.parse_expression()?;
                let end = self
                    .expect(
                        TokenKind::RightBracket,
                        "expected `]` after collection repeat literal",
                    )?
                    .span
                    .end;
                return Some(Expr::ArrayRepeat {
                    value: Box::new(first),
                    count: Box::new(count),
                    span: Span::new(start, end),
                });
            }

            let mut first = Some(first);
            loop {
                let first = match first.take() {
                    Some(first) => first,
                    None => self.parse_expression()?,
                };
                if self.match_kind(&TokenKind::FatArrow) {
                    let value = self.parse_expression()?;
                    elements.push(ArrayElement {
                        key: Some(first),
                        value,
                    });
                } else {
                    elements.push(ArrayElement {
                        key: None,
                        value: first,
                    });
                }

                if !self.match_kind(&TokenKind::Comma) {
                    break;
                }
                if self.check(&TokenKind::RightBracket) {
                    break;
                }
            }
        }

        let end = self
            .expect(
                TokenKind::RightBracket,
                "expected `]` after collection literal",
            )?
            .span
            .end;
        Some(Expr::Array {
            elements,
            span: Span::new(start, end),
        })
    }

    fn parse_argument_list_after_open(&mut self) -> Option<Vec<Argument>> {
        let mut args = Vec::new();
        let mut last_named_span: Option<Span> = None;
        if !self.check(&TokenKind::RightParen) {
            loop {
                let is_named = matches!(self.peek().kind, TokenKind::Identifier(_))
                    && self
                        .tokens
                        .get(self.current + 1)
                        .is_some_and(|token| matches!(token.kind, TokenKind::Colon));
                if is_named {
                    let name_token = self.advance().clone();
                    let TokenKind::Identifier(name) = name_token.kind.clone() else {
                        unreachable!("named-argument lookahead guarantees an identifier");
                    };
                    self.advance();
                    let value = self.parse_expression()?;
                    let span = name_token.span.merge(value.span());
                    last_named_span = Some(name_token.span);
                    args.push(Argument {
                        name: Some(ArgumentName {
                            text: name,
                            span: name_token.span,
                        }),
                        value,
                        span,
                    });
                } else {
                    let value = self.parse_expression()?;
                    let span = value.span();
                    if let Some(named_span) = last_named_span {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "E0515",
                                "a positional argument cannot follow a named argument",
                                span,
                            )
                            .with_help(
                                "once a call uses a named argument, every following argument must also be named",
                            )
                            .with_related(named_span, "named argument appears here"),
                        );
                    }
                    args.push(Argument {
                        name: None,
                        value,
                        span,
                    });
                }
                if !self.match_kind(&TokenKind::Comma) {
                    break;
                }
                if self.check(&TokenKind::RightParen) {
                    break;
                }
            }
        }
        self.expect(TokenKind::RightParen, "expected `)` after arguments")?;
        Some(args)
    }

    fn parse_when_expression(&mut self, start: usize, given: Option<GivenPrelude>) -> Option<Expr> {
        self.expect(TokenKind::LeftParen, "expected `(` after when")?;
        let condition = self.parse_expression()?;
        self.expect(TokenKind::RightParen, "expected `)` after when condition")?;
        let result_type = if self.match_kind(&TokenKind::Colon) {
            Some(self.parse_type_ref()?)
        } else {
            None
        };
        let block = self.parse_block()?;
        let mut end = block.span.end;
        let mut branches = vec![WhenBranch {
            span: condition.span().merge(block.span),
            condition: Some(condition),
            block,
        }];

        let mut has_else = false;
        while self.match_kind(&TokenKind::Else) {
            if self.match_kind(&TokenKind::When) {
                self.expect(TokenKind::LeftParen, "expected `(` after `else when`")?;
                let condition = self.parse_expression()?;
                self.expect(TokenKind::RightParen, "expected `)` after when condition")?;
                let block = self.parse_block()?;
                end = block.span.end;
                branches.push(WhenBranch {
                    span: condition.span().merge(block.span),
                    condition: Some(condition),
                    block,
                });
            } else {
                has_else = true;
                let block = self.parse_block()?;
                end = block.span.end;
                branches.push(WhenBranch {
                    span: block.span,
                    condition: None,
                    block,
                });
                break;
            }
        }

        if !has_else {
            self.error(
                "value-returning `when` requires an `else` block",
                self.peek().span,
            );
        }
        let finally = self.parse_optional_finally()?;
        if let Some(clause) = &finally {
            end = clause.span.end;
        }

        Some(Expr::When(Box::new(WhenExpression {
            given,
            result_type,
            branches,
            finally,
            span: Span::new(start, end),
        })))
    }

    fn parse_type_ref(&mut self) -> Option<TypeRef> {
        let ty = self.parse_type_ref_inner();
        match (ty, self.pending_type_argument_close.take()) {
            (Some(ty), None) => Some(ty),
            (Some(_), Some(span)) => {
                self.error("unexpected `>` after type", span);
                None
            }
            (None, _) => None,
        }
    }

    fn parse_function_type_ref(&mut self, keyword_span: Span) -> Option<TypeRef> {
        let mut invocation_mode = FunctionInvocationMode::Readonly;
        let mut invocation_modifier_span = None;
        if matches!(
            self.peek().kind,
            TokenKind::Writable | TokenKind::Once | TokenKind::Take | TokenKind::Readonly
        ) {
            let modifier = self.advance().clone();
            invocation_modifier_span = Some(modifier.span);
            invocation_mode = match modifier.kind {
                TokenKind::Writable => FunctionInvocationMode::Writable,
                TokenKind::Once => FunctionInvocationMode::Once,
                TokenKind::Take => {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "P0001",
                            "`take` transfers a function value; `once` describes consuming invocation",
                            modifier.span,
                        )
                        .with_title("Function Invocation Mode Uses `Once`")
                        .with_help("replace `take` with `once`")
                        .with_fix(modifier.span, "once"),
                    );
                    FunctionInvocationMode::Once
                }
                TokenKind::Readonly => {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "P0001",
                            "readonly invocation is the unmodified `function(...)` form",
                            modifier.span,
                        )
                        .with_title("Readonly Function Mode Is Implicit")
                        .with_help("omit `readonly`")
                        .with_fix(modifier.span, ""),
                    );
                    FunctionInvocationMode::Readonly
                }
                _ => unreachable!("function invocation modifier lookahead is exact"),
            };
        }
        while matches!(
            self.peek().kind,
            TokenKind::Writable | TokenKind::Once | TokenKind::Take | TokenKind::Readonly
        ) {
            let modifier = self.advance().clone();
            self.diagnostics.push(
                Diagnostic::new(
                    "P0001",
                    "a function type accepts at most one invocation modifier",
                    modifier.span,
                )
                .with_title("Function Invocation Mode Is Duplicated")
                .with_help("keep exactly one of the default readonly form, `writable`, or `once`"),
            );
        }
        let open_span = self
            .expect(TokenKind::LeftParen, "expected `(` after `function` type")?
            .span;
        let mut parameters = Vec::new();
        if !self.check(&TokenKind::RightParen) {
            loop {
                let start = self.peek().span.start;
                let mut ownership_mode = FunctionTypeParameterMode::Readonly;
                let mut ownership_modifier_span = None;
                if matches!(
                    self.peek().kind,
                    TokenKind::Writable | TokenKind::Take | TokenKind::Readonly
                ) {
                    let modifier = self.advance().clone();
                    ownership_modifier_span = Some(modifier.span);
                    ownership_mode = match modifier.kind {
                        TokenKind::Writable => FunctionTypeParameterMode::Writable,
                        TokenKind::Take => FunctionTypeParameterMode::Take,
                        TokenKind::Readonly => {
                            self.diagnostics.push(
                                Diagnostic::new(
                                    "P0001",
                                    "readonly function-type parameters use a bare type",
                                    modifier.span,
                                )
                                .with_title("Readonly Parameter Mode Is Implicit")
                                .with_help("omit `readonly`")
                                .with_fix(modifier.span, ""),
                            );
                            FunctionTypeParameterMode::Readonly
                        }
                        _ => unreachable!("parameter ownership modifier lookahead is exact"),
                    };
                }
                while matches!(
                    self.peek().kind,
                    TokenKind::Writable | TokenKind::Take | TokenKind::Readonly
                ) {
                    let modifier = self.advance().clone();
                    let duplicate = matches!(
                        (ownership_mode, &modifier.kind),
                        (FunctionTypeParameterMode::Writable, TokenKind::Writable)
                            | (FunctionTypeParameterMode::Take, TokenKind::Take)
                            | (FunctionTypeParameterMode::Readonly, TokenKind::Readonly)
                    );
                    let (title, message) = if duplicate {
                        (
                            "Function Type Parameter Mode Is Duplicated",
                            "a function-type parameter cannot repeat its ownership modifier",
                        )
                    } else {
                        (
                            "Function Type Parameter Modes Conflict",
                            "`take` and `writable` cannot describe the same function-type parameter",
                        )
                    };
                    self.diagnostics.push(
                        Diagnostic::new("P0001", message, modifier.span)
                            .with_title(title)
                            .with_help("keep one ownership mode on the parameter"),
                    );
                }
                if self.check(&TokenKind::RightParen) || self.check(&TokenKind::Comma) {
                    self.diagnostics.push(
                        Diagnostic::new(
                            "P0001",
                            "a function-type parameter ownership modifier must be followed by a type",
                            ownership_modifier_span.unwrap_or(self.peek().span),
                        )
                        .with_title("Function Type Parameter Type Is Missing"),
                    );
                    parameters.push(FunctionTypeParameterRef {
                        ownership_mode,
                        ownership_modifier_span,
                        ty: TypeRef::unknown(),
                        type_span: Span::new(self.peek().span.start, self.peek().span.start),
                        span: Span::new(start, self.peek().span.start),
                    });
                } else {
                    let type_start = self.peek().span.start;
                    let ty = self.parse_type_ref_inner()?;
                    let type_span = Span::new(type_start, self.previous().span.end);
                    if ty
                        .function
                        .as_ref()
                        .is_some_and(|nested| nested.throws_clause.is_some())
                        && ty.grouped.is_none()
                    {
                        self.report_ambiguous_nested_function_effects(
                            ty.function.as_ref().unwrap().span,
                        );
                    }
                    let span = Span::new(start, type_span.end);
                    if let Some((_name, name_span)) = self.consume_variable() {
                        self.diagnostics.push(
                            Diagnostic::new(
                                "P0001",
                                "function-type parameters contain types, not parameter names",
                                name_span,
                            )
                            .with_title("Function Type Parameter Has A Name")
                            .with_help("remove the variable name from the function type"),
                        );
                    }
                    parameters.push(FunctionTypeParameterRef {
                        ownership_mode,
                        ownership_modifier_span,
                        ty,
                        type_span,
                        span,
                    });
                }
                if !self.match_kind(&TokenKind::Comma) {
                    break;
                }
                if self.check(&TokenKind::RightParen) {
                    break;
                }
            }
        }
        let close_span = self
            .expect(
                TokenKind::RightParen,
                "expected `)` after function-type parameters",
            )?
            .span;
        self.expect(
            TokenKind::Colon,
            "expected `:` before function-type return type",
        )?;
        let colon_span = self.previous().span;
        let return_start = self.peek().span.start;
        let return_type = self.parse_type_ref_inner()?;
        let return_type_span = Span::new(return_start, self.previous().span.end);
        if return_type
            .function
            .as_ref()
            .is_some_and(|nested| nested.throws_clause.is_some())
            && return_type.grouped.is_none()
        {
            self.report_ambiguous_nested_function_effects(
                return_type.function.as_ref().unwrap().span,
            );
        }
        let throws_clause = self.parse_function_type_throws_clause()?;
        let end = throws_clause
            .as_ref()
            .map(|clause| clause.span.end)
            .unwrap_or(return_type_span.end);
        let span = Span::new(keyword_span.start, end);
        Some(TypeRef::function(FunctionTypeRef {
            keyword_span,
            invocation_mode,
            invocation_modifier_span,
            parameter_list_open_span: open_span,
            parameter_list_close_span: close_span,
            parameter_list_span: open_span.merge(close_span),
            parameters,
            colon_span,
            return_type: Box::new(return_type),
            return_type_span,
            throws_clause,
            span,
        }))
    }

    fn parse_function_type_throws_clause(&mut self) -> Option<Option<FunctionTypeThrowsRef>> {
        if !self.match_kind(&TokenKind::Throws) {
            return Some(None);
        }
        let keyword_span = self.previous().span;
        let mut entries = Vec::new();
        if !self.can_start_type_ref() {
            self.diagnostics.push(
                Diagnostic::new(
                    "P0001",
                    "a function-type `throws` clause requires at least one effect type",
                    self.peek().span,
                )
                .with_title("Function Type Effect Is Missing"),
            );
            return Some(Some(FunctionTypeThrowsRef {
                keyword_span,
                entries,
                span: keyword_span,
            }));
        }
        loop {
            let start = self.peek().span.start;
            let ty = self.parse_type_ref_inner()?;
            let type_span = Span::new(start, self.previous().span.end);
            entries.push(FunctionTypeEffectRef {
                ty,
                type_span,
                span: type_span,
            });
            if !self.match_kind(&TokenKind::Comma) {
                break;
            }
            if !self.can_start_type_ref() {
                self.diagnostics.push(
                    Diagnostic::new(
                        "P0001",
                        "a trailing comma in a function-type `throws` clause must be followed by an effect type",
                        self.previous().span,
                    )
                    .with_title("Function Type Effect Is Missing"),
                );
                break;
            }
        }
        let end = entries
            .last()
            .map(|entry| entry.span.end)
            .unwrap_or(keyword_span.end);
        Some(Some(FunctionTypeThrowsRef {
            keyword_span,
            entries,
            span: Span::new(keyword_span.start, end),
        }))
    }

    fn report_ambiguous_nested_function_effects(&mut self, span: Span) {
        self.diagnostics.push(
            Diagnostic::new(
                "P0001",
                "a nested throwing function type must be grouped so its effect list has an explicit boundary",
                span,
            )
            .with_title("Nested Function Type Effects Need Grouping")
            .with_help("wrap the nested function type in parentheses"),
        );
    }

    fn parse_type_ref_inner(&mut self) -> Option<TypeRef> {
        let nullable = self.match_kind(&TokenKind::Question);
        let token = self.advance().clone();
        let mut ty = if matches!(token.kind, TokenKind::Function) {
            self.parse_function_type_ref(token.span)?
        } else if matches!(token.kind, TokenKind::LeftParen) {
            if self.check(&TokenKind::RightParen) {
                let close_span = self.advance().span;
                self.diagnostics.push(
                    Diagnostic::new(
                        "P0001",
                        "a type grouping must contain exactly one type",
                        token.span.merge(close_span),
                    )
                    .with_title("Type Group Is Empty")
                    .with_help("write one type between the parentheses"),
                );
                TypeRef::grouped(TypeRef::unknown(), token.span, close_span)
            } else {
                let inner = self.parse_type_ref_inner()?;
                if self.match_kind(&TokenKind::Comma) {
                    let comma_span = self.previous().span;
                    self.diagnostics.push(
                        Diagnostic::new(
                            "P0001",
                            "parenthesized type grouping contains one type; Doria does not define tuple types here",
                            comma_span,
                        )
                        .with_title("Tuple Type Is Not Supported")
                        .with_help("use a named class or another explicit aggregate type"),
                    );
                    while !self.check(&TokenKind::RightParen) && !self.is_at_end() {
                        if self.parse_type_ref_inner().is_none() {
                            break;
                        }
                        if !self.match_kind(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                let close_span = self
                    .expect(TokenKind::RightParen, "expected `)` after grouped type")?
                    .span;
                TypeRef::grouped(inner, token.span, close_span)
            }
        } else {
            let name = match token.kind {
                TokenKind::Void => "void".to_string(),
                TokenKind::IntType => "int".to_string(),
                TokenKind::Int8Type => "int8".to_string(),
                TokenKind::Int16Type => "int16".to_string(),
                TokenKind::Int32Type => "int32".to_string(),
                TokenKind::Int64Type => "int64".to_string(),
                TokenKind::UInt8Type => "uint8".to_string(),
                TokenKind::UInt16Type => "uint16".to_string(),
                TokenKind::UInt32Type => "uint32".to_string(),
                TokenKind::UInt64Type => "uint64".to_string(),
                TokenKind::FloatType => "float".to_string(),
                TokenKind::Float32Type => "float32".to_string(),
                TokenKind::Float64Type => "float64".to_string(),
                TokenKind::StringType => "string".to_string(),
                TokenKind::BoolType => "bool".to_string(),
                TokenKind::Null => "null".to_string(),
                TokenKind::Object => "object".to_string(),
                TokenKind::Resource => "resource".to_string(),
                TokenKind::SelfType => "self".to_string(),
                TokenKind::Identifier(name) => self.finish_qualified_name(
                    name,
                    "expected type-name segment after namespace separator",
                )?,
                other => {
                    self.error(
                        format!("expected type name, found `{}`", token_name(&other)),
                        self.previous().span,
                    );
                    return None;
                }
            };

            let mut arguments = Vec::new();
            if self.match_kind(&TokenKind::Less) {
                loop {
                    let negative = self.match_kind(&TokenKind::Minus);
                    if let TokenKind::IntLiteral(value) = self.peek().kind.clone() {
                        self.advance();
                        arguments.push(crate::types::TypeArgumentRef::Value(if negative {
                            format!("-{value}")
                        } else {
                            value
                        }));
                    } else {
                        if negative {
                            self.error(
                                "expected integer compile-time value after `-`",
                                self.peek().span,
                            );
                            return None;
                        }
                        arguments.push(crate::types::TypeArgumentRef::Type(
                            self.parse_type_ref_inner()?,
                        ));
                    }
                    if !self.match_kind(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect_type_argument_close()?;
            }

            if arguments.is_empty() {
                TypeRef::named(name)
            } else {
                TypeRef::generic_with_arguments(name, arguments)
            }
        };

        while self.pending_type_argument_close.is_none() && self.match_kind(&TokenKind::LeftBracket)
        {
            self.expect(
                TokenKind::RightBracket,
                "expected `]` after typed array suffix",
            )?;
            ty = TypeRef::array_of(ty);
        }

        if nullable {
            ty = ty.nullable();
        }
        Some(ty)
    }

    fn expect_type_argument_close(&mut self) -> Option<()> {
        if self.pending_type_argument_close.take().is_some() {
            return Some(());
        }

        if self.check(&TokenKind::Greater) {
            self.advance();
            return Some(());
        }

        if self.check(&TokenKind::ShiftRight) {
            let span = self.advance().span;
            let split = span.start + 1;
            self.pending_type_argument_close = Some(Span::new(split, span.end));
            return Some(());
        }

        self.error(
            "expected `>` after generic type arguments",
            self.peek().span,
        );
        None
    }

    fn parse_member_access(&mut self) -> MemberAccess {
        if self.match_kind(&TokenKind::Internal) {
            MemberAccess::Internal
        } else {
            MemberAccess::External
        }
    }

    fn parse_assignment_op(&mut self) -> Option<AssignOp> {
        if self.match_kind(&TokenKind::Equals) {
            Some(AssignOp::Assign)
        } else if self.match_kind(&TokenKind::PlusEquals) {
            Some(AssignOp::AddAssign)
        } else if self.match_kind(&TokenKind::MinusEquals) {
            Some(AssignOp::SubAssign)
        } else if self.match_kind(&TokenKind::StarEquals) {
            Some(AssignOp::MulAssign)
        } else if self.match_kind(&TokenKind::SlashEquals) {
            Some(AssignOp::DivAssign)
        } else if self.match_kind(&TokenKind::PercentEquals) {
            Some(AssignOp::ModAssign)
        } else if self.match_kind(&TokenKind::ShiftLeftEquals) {
            Some(AssignOp::ShiftLeftAssign)
        } else if self.match_kind(&TokenKind::ShiftRightEquals) {
            Some(AssignOp::ShiftRightAssign)
        } else if self.match_kind(&TokenKind::AmpersandEquals) {
            Some(AssignOp::BitwiseAndAssign)
        } else if self.match_kind(&TokenKind::PipeEquals) {
            Some(AssignOp::BitwiseOrAssign)
        } else if self.match_kind(&TokenKind::CaretEquals) {
            Some(AssignOp::BitwiseXorAssign)
        } else {
            None
        }
    }

    fn current_binary_op(&self) -> Option<(BinaryOp, u8)> {
        match self.peek().kind {
            TokenKind::OrOr | TokenKind::Or => Some((BinaryOp::Or, 1)),
            TokenKind::Xor => Some((BinaryOp::Xor, 1)),
            TokenKind::AndAnd | TokenKind::And => Some((BinaryOp::And, 2)),
            TokenKind::QuestionQuestion => Some((BinaryOp::Coalesce, 3)),
            TokenKind::Pipe => Some((BinaryOp::BitwiseOr, 4)),
            TokenKind::Caret => Some((BinaryOp::BitwiseXor, 5)),
            TokenKind::Ampersand => Some((BinaryOp::BitwiseAnd, 6)),
            TokenKind::EqualEqual => Some((BinaryOp::Equal, 7)),
            TokenKind::BangEqual => Some((BinaryOp::NotEqual, 7)),
            TokenKind::Less => Some((BinaryOp::Less, 8)),
            TokenKind::LessEqual => Some((BinaryOp::LessEqual, 8)),
            TokenKind::Greater => Some((BinaryOp::Greater, 8)),
            TokenKind::GreaterEqual => Some((BinaryOp::GreaterEqual, 8)),
            TokenKind::ShiftLeft => Some((BinaryOp::ShiftLeft, 9)),
            TokenKind::ShiftRight => Some((BinaryOp::ShiftRight, 9)),
            TokenKind::Plus => Some((BinaryOp::Add, 10)),
            TokenKind::Minus => Some((BinaryOp::Sub, 10)),
            TokenKind::Dot => Some((BinaryOp::Concat, 10)),
            TokenKind::Star => Some((BinaryOp::Mul, 11)),
            TokenKind::Slash => Some((BinaryOp::Div, 11)),
            TokenKind::Percent => Some((BinaryOp::Mod, 11)),
            _ => None,
        }
    }

    fn can_start_typed_decl(&self) -> bool {
        self.check(&TokenKind::Writable) || self.can_start_type_ref()
    }

    fn can_start_type_ref(&self) -> bool {
        matches!(
            self.peek().kind,
            TokenKind::Question
                | TokenKind::LeftParen
                | TokenKind::Void
                | TokenKind::IntType
                | TokenKind::Int8Type
                | TokenKind::Int16Type
                | TokenKind::Int32Type
                | TokenKind::Int64Type
                | TokenKind::UInt8Type
                | TokenKind::UInt16Type
                | TokenKind::UInt32Type
                | TokenKind::UInt64Type
                | TokenKind::FloatType
                | TokenKind::Float32Type
                | TokenKind::Float64Type
                | TokenKind::StringType
                | TokenKind::BoolType
                | TokenKind::Null
                | TokenKind::Object
                | TokenKind::Resource
                | TokenKind::SelfType
                | TokenKind::Function
                | TokenKind::Identifier(_)
        )
    }

    fn expect_identifier(&mut self, message: &str) -> Option<String> {
        let token = self.advance().clone();
        match token.kind {
            TokenKind::Identifier(name) => Some(name),
            _ => {
                self.error(message, token.span);
                None
            }
        }
    }

    fn expect_type_declaration_name(&mut self, message: &str) -> Option<String> {
        let token = self.advance().clone();
        match token.kind {
            TokenKind::Identifier(name) => Some(name),
            TokenKind::SelfType => Some("self".to_string()),
            _ => {
                self.error(message, token.span);
                None
            }
        }
    }

    fn expect_qualified_name(&mut self, message: &str) -> Option<String> {
        let first = self.expect_identifier(message)?;
        self.finish_qualified_name(first, "expected name segment after namespace separator")
    }

    fn finish_qualified_name(&mut self, mut name: String, message: &str) -> Option<String> {
        while self.match_kind(&TokenKind::Backslash) {
            name.push('\\');
            name.push_str(&self.expect_identifier(message)?);
        }
        Some(name)
    }

    fn expect_variable(&mut self, message: &str) -> Option<(String, Span)> {
        self.consume_variable().or_else(|| {
            self.error(message, self.peek().span);
            None
        })
    }

    fn consume_variable(&mut self) -> Option<(String, Span)> {
        let token = self.peek().clone();
        if let TokenKind::Variable(name) = token.kind {
            self.advance();
            Some((name, token.span))
        } else {
            None
        }
    }

    fn expect(&mut self, kind: TokenKind, message: &str) -> Option<Token> {
        if self.check(&kind) {
            Some(self.advance().clone())
        } else {
            self.error(message, self.peek().span);
            None
        }
    }

    fn match_kind(&mut self, kind: &TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn check(&self, kind: &TokenKind) -> bool {
        if self.is_at_end() {
            return matches!(kind, TokenKind::Eof);
        }
        std::mem::discriminant(&self.peek().kind) == std::mem::discriminant(kind)
    }

    fn advance(&mut self) -> &Token {
        if self.is_at_end() {
            return self.peek();
        }
        self.current += 1;
        self.previous()
    }

    fn is_at_end(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.current]
    }

    fn previous(&self) -> &Token {
        &self.tokens[self.current - 1]
    }

    fn error(&mut self, message: impl Into<String>, span: Span) {
        self.diagnostics
            .push(Diagnostic::new("P0001", message, span));
    }

    fn synchronize(&mut self) {
        while !self.is_at_end() {
            if matches!(
                self.previous().kind,
                TokenKind::Semicolon | TokenKind::RightBrace
            ) {
                return;
            }
            match self.peek().kind {
                TokenKind::Class
                | TokenKind::Interface
                | TokenKind::Trait
                | TokenKind::Namespace
                | TokenKind::Function
                | TokenKind::Const
                | TokenKind::Let
                | TokenKind::Echo
                | TokenKind::Return
                | TokenKind::Throw
                | TokenKind::Try
                | TokenKind::Break
                | TokenKind::Continue
                | TokenKind::If
                | TokenKind::While
                | TokenKind::Do
                | TokenKind::Given
                | TokenKind::For
                | TokenKind::Foreach
                | TokenKind::Internal => return,
                _ => {
                    self.advance();
                }
            }
        }
    }
}

fn token_name(kind: &TokenKind) -> &'static str {
    match kind {
        TokenKind::Class => "class",
        TokenKind::Enum => "enum",
        TokenKind::Case => "case",
        TokenKind::Interface => "interface",
        TokenKind::Trait => "trait",
        TokenKind::Implements => "implements",
        TokenKind::Namespace => "namespace",
        TokenKind::Extends => "extends",
        TokenKind::Function => "function",
        TokenKind::Fn => "fn",
        TokenKind::Const => "const",
        TokenKind::Internal => "internal",
        TokenKind::Static => "static",
        TokenKind::SelfType => "self",
        TokenKind::Parent => "parent",
        TokenKind::Let => "let",
        TokenKind::With => "with",
        TokenKind::Take => "take",
        TokenKind::Once => "once",
        TokenKind::Writable => "writable",
        TokenKind::Readonly => "readonly",
        TokenKind::Return => "return",
        TokenKind::Echo => "echo",
        TokenKind::New => "new",
        TokenKind::Shared => "shared",
        TokenKind::Foreach => "foreach",
        TokenKind::As => "as",
        TokenKind::If => "if",
        TokenKind::Else => "else",
        TokenKind::Match => "match",
        TokenKind::Default => "default",
        TokenKind::When => "when",
        TokenKind::Given => "given",
        TokenKind::Try => "try",
        TokenKind::Catch => "catch",
        TokenKind::Finally => "finally",
        TokenKind::While => "while",
        TokenKind::Do => "do",
        TokenKind::For => "for",
        TokenKind::Break => "break",
        TokenKind::Continue => "continue",
        TokenKind::Throw => "throw",
        TokenKind::Throws => "throws",
        TokenKind::True => "true",
        TokenKind::False => "false",
        TokenKind::Null => "null",
        TokenKind::Is => "is",
        TokenKind::Object => "object",
        TokenKind::Resource => "resource",
        TokenKind::Void => "void",
        TokenKind::IntType => "int",
        TokenKind::Int8Type => "int8",
        TokenKind::Int16Type => "int16",
        TokenKind::Int32Type => "int32",
        TokenKind::Int64Type => "int64",
        TokenKind::UInt8Type => "uint8",
        TokenKind::UInt16Type => "uint16",
        TokenKind::UInt32Type => "uint32",
        TokenKind::UInt64Type => "uint64",
        TokenKind::FloatType => "float",
        TokenKind::Float32Type => "float32",
        TokenKind::Float64Type => "float64",
        TokenKind::StringType => "string",
        TokenKind::BoolType => "bool",
        TokenKind::Reserved(_) => "reserved keyword",
        TokenKind::Identifier(_) => "identifier",
        TokenKind::Variable(_) => "variable",
        TokenKind::IntLiteral(_) => "integer",
        TokenKind::FloatLiteral(_) => "float",
        TokenKind::StringLiteral { .. } => "string",
        TokenKind::Equals => "=",
        TokenKind::Plus => "+",
        TokenKind::Minus => "-",
        TokenKind::Star => "*",
        TokenKind::Slash => "/",
        TokenKind::Backslash => "\\",
        TokenKind::Percent => "%",
        TokenKind::Dot => ".",
        TokenKind::DotDot => "..",
        TokenKind::DotDotLess => "..<",
        TokenKind::PlusPlus => "++",
        TokenKind::MinusMinus => "--",
        TokenKind::PlusEquals => "+=",
        TokenKind::MinusEquals => "-=",
        TokenKind::StarEquals => "*=",
        TokenKind::SlashEquals => "/=",
        TokenKind::PercentEquals => "%=",
        TokenKind::ShiftLeftEquals => "<<=",
        TokenKind::ShiftRightEquals => ">>=",
        TokenKind::AmpersandEquals => "&=",
        TokenKind::PipeEquals => "|=",
        TokenKind::CaretEquals => "^=",
        TokenKind::EqualEqual => "==",
        TokenKind::EqualEqualEqual => "===",
        TokenKind::BangEqual => "!=",
        TokenKind::BangEqualEqual => "!==",
        TokenKind::Less => "<",
        TokenKind::LessEqual => "<=",
        TokenKind::Greater => ">",
        TokenKind::GreaterEqual => ">=",
        TokenKind::ShiftLeft => "<<",
        TokenKind::ShiftRight => ">>",
        TokenKind::Ampersand => "&",
        TokenKind::Pipe => "|",
        TokenKind::Caret => "^",
        TokenKind::Tilde => "~",
        TokenKind::AndAnd => "&&",
        TokenKind::OrOr => "||",
        TokenKind::Bang => "!",
        TokenKind::Not => "not",
        TokenKind::And => "and",
        TokenKind::Or => "or",
        TokenKind::Xor => "xor",
        TokenKind::Question => "?",
        TokenKind::QuestionQuestion => "??",
        TokenKind::QuestionArrow => "?->",
        TokenKind::FatArrow => "=>",
        TokenKind::LeftParen => "(",
        TokenKind::RightParen => ")",
        TokenKind::LeftBrace => "{",
        TokenKind::RightBrace => "}",
        TokenKind::LeftBracket => "[",
        TokenKind::RightBracket => "]",
        TokenKind::Semicolon => ";",
        TokenKind::Colon => ":",
        TokenKind::Comma => ",",
        TokenKind::Arrow => "->",
        TokenKind::DoubleColon => "::",
        TokenKind::Eof => "end of file",
    }
}
