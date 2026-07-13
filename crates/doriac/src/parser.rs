use crate::ast::*;
use crate::diagnostics::{Diagnostic, DiagnosticResult};
use crate::lexer::{Lexer, StringQuoteKind, Token, TokenKind};
use crate::source::{SourceFile, Span};
use crate::string_literal::{decode_escape, interpolation_close};
use crate::types::TypeRef;

pub struct Parser {
    tokens: Vec<Token>,
    current: usize,
    pending_type_argument_close: Option<Span>,
    diagnostics: Vec<Diagnostic>,
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
        let mut items = Vec::new();

        while !self.is_at_end() {
            match self.parse_item() {
                Some(item) => items.push(item),
                None => self.synchronize(),
            }
        }

        if self.diagnostics.is_empty() {
            Ok(Program { items })
        } else {
            Err(self.diagnostics)
        }
    }

    fn parse_item(&mut self) -> Option<Item> {
        if self.match_kind(&TokenKind::Class) {
            self.parse_class().map(Item::Class)
        } else if self.match_kind(&TokenKind::Interface) {
            self.parse_unsupported_interface();
            None
        } else if self.match_kind(&TokenKind::Function) {
            self.parse_function(MemberAccess::External, false, false, self.previous().span)
                .map(Item::Function)
        } else {
            self.parse_statement().map(Item::Statement)
        }
    }

    fn parse_class(&mut self) -> Option<ClassDecl> {
        let start = self.previous().span.start;
        let name = self.expect_identifier("expected class name")?;
        let mut implements = Vec::new();
        if self.match_kind(&TokenKind::Implements) {
            loop {
                implements
                    .push(self.expect_identifier("expected interface name after `implements`")?);
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
            implements,
            members,
            span: Span::new(start, end),
        })
    }

    fn parse_class_member(&mut self) -> Option<ClassMember> {
        let access = self.parse_member_access();
        let is_static = self.match_kind(&TokenKind::Static);

        let writable = self.match_kind(&TokenKind::Writable);
        if self.match_kind(&TokenKind::Function) {
            let start = self.previous().span.start;
            return self
                .parse_function(access, writable, is_static, Span::new(start, start))
                .map(ClassMember::Method);
        }

        if is_static {
            self.error(
                "static properties are not implemented yet",
                self.previous().span,
            );
            return None;
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
            writable,
            ty,
            name,
            initializer,
            span: Span::new(start.min(name_span.start), end),
        }))
    }

    fn parse_function(
        &mut self,
        access: MemberAccess,
        writable_this: bool,
        is_static: bool,
        start_span: Span,
    ) -> Option<FunctionDecl> {
        let start = start_span.start;
        let name = self.expect_identifier("expected function name")?;
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

        let body = self.parse_block()?;
        let span = Span::new(start, body.span.end);
        Some(FunctionDecl {
            access,
            writable_this,
            is_static,
            name,
            params,
            return_type,
            body,
            span,
        })
    }

    fn parse_unsupported_interface(&mut self) {
        let start = self.previous().span.start;
        let name = self.expect_identifier("expected interface name");
        let message = if matches!(name.as_deref(), Some("Displayable")) {
            "`Displayable` is a compiler-known interface and cannot be redeclared"
        } else {
            "general interface declarations are planned for Stage 35 and are not implemented yet"
        };

        let mut end = self.previous().span.end;
        if self.match_kind(&TokenKind::LeftBrace) {
            let mut depth = 1_usize;
            while depth > 0 && !self.is_at_end() {
                let token = self.advance();
                end = token.span.end;
                match token.kind {
                    TokenKind::LeftBrace => depth += 1,
                    TokenKind::RightBrace => depth -= 1,
                    _ => {}
                }
            }
        }
        self.diagnostics
            .push(Diagnostic::new("P0003", message, Span::new(start, end)));
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
        let writable = self.match_kind(&TokenKind::Writable);
        let ty = self.parse_type_ref()?;
        let (name, name_span) = self.expect_variable("expected parameter variable name")?;
        let default = if self.match_kind(&TokenKind::Equals) {
            Some(self.parse_expression()?)
        } else {
            None
        };

        let end = default.as_ref().map(Expr::span).unwrap_or(name_span).end;

        Some(Param {
            promoted_access: is_constructor.then_some(access),
            writable,
            ty,
            name,
            default,
            span: Span::new(start, end),
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
            let checkpoint = self.current;
            let diagnostics_checkpoint = self.diagnostics.len();
            let pending_type_argument_close_checkpoint = self.pending_type_argument_close;
            let start = self.peek().span.start;
            let writable = self.match_kind(&TokenKind::Writable);
            if let Some(ty) = self.parse_type_ref() {
                if let Some((name, _name_span)) = self.consume_variable() {
                    self.expect(TokenKind::Equals, "expected `=` in variable declaration")?;
                    let initializer = self.parse_expression()?;
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
                        name,
                        initializer,
                        span: Span::new(start, end),
                    }));
                }
            }
            self.current = checkpoint;
            self.pending_type_argument_close = pending_type_argument_close_checkpoint;
            self.diagnostics.truncate(diagnostics_checkpoint);
        }

        let expr = self.parse_expression()?;
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
        let (name, _span) = self.expect_variable("expected variable name after `let`")?;
        self.expect(TokenKind::Equals, "expected `=` in let declaration")?;
        let initializer = self.parse_expression()?;
        let end = self
            .expect(TokenKind::Semicolon, semicolon_message)?
            .span
            .end;

        Some(VarDecl {
            writable,
            ty: None,
            name,
            initializer,
            span: Span::new(start, end),
        })
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
        self.expect(TokenKind::LeftParen, "expected `(` after if")?;
        let condition = self.parse_expression()?;
        self.expect(TokenKind::RightParen, "expected `)` after if condition")?;
        let then_block = self.parse_block()?;
        let else_branch = if self.match_kind(&TokenKind::Else) {
            if self.match_kind(&TokenKind::If) {
                Some(ElseBranch::If(Box::new(self.parse_if_statement()?)))
            } else {
                Some(ElseBranch::Block(self.parse_block()?))
            }
        } else {
            None
        };
        let end = else_branch
            .as_ref()
            .map(ElseBranch::span)
            .unwrap_or(then_block.span)
            .end;

        Some(IfStmt {
            condition,
            then_block,
            else_branch,
            span: Span::new(start, end),
        })
    }

    fn parse_while(&mut self) -> Option<WhileStmt> {
        let start = self.previous().span.start;
        self.expect(TokenKind::LeftParen, "expected `(` after while")?;
        let condition = self.parse_expression()?;
        self.expect(TokenKind::RightParen, "expected `)` after while condition")?;
        let body = self.parse_block()?;
        let span = Span::new(start, body.span.end);
        Some(WhileStmt {
            condition,
            body,
            span,
        })
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
        if let Some((name, _span)) = self.consume_variable() {
            return Some(ForeachBinding { ty: None, name });
        }

        let ty = self.parse_type_ref()?;
        let (name, _span) = self.expect_variable("expected foreach binding variable")?;
        Some(ForeachBinding { ty: Some(ty), name })
    }

    fn parse_expression(&mut self) -> Option<Expr> {
        self.parse_range()
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

        while let Some((op, prec)) = self.current_binary_op() {
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
            if self.match_kind(&TokenKind::Arrow) {
                let property =
                    self.expect_identifier("expected property or method name after `->`")?;
                if self.match_kind(&TokenKind::LeftParen) {
                    let args = self.parse_argument_list_after_open()?;
                    let span = expr.span().merge(self.previous().span);
                    expr = Expr::MethodCall {
                        object: Box::new(expr),
                        method: property,
                        args,
                        span,
                    };
                } else {
                    let span = expr.span().merge(self.previous().span);
                    expr = Expr::PropertyAccess {
                        object: Box::new(expr),
                        property,
                        span,
                    };
                }
                continue;
            }

            if self.match_kind(&TokenKind::LeftParen) {
                let args = self.parse_argument_list_after_open()?;
                match expr {
                    Expr::Identifier { name, span } => {
                        let end = self.previous().span.end;
                        expr = Expr::FunctionCall {
                            name,
                            args,
                            span: Span::new(span.start, end),
                        };
                    }
                    _ => {
                        self.error(
                            "only named function calls are supported in this position",
                            expr.span(),
                        );
                    }
                }
                continue;
            }

            break;
        }

        Some(expr)
    }

    fn parse_primary(&mut self) -> Option<Expr> {
        let token = self.advance().clone();
        match token.kind {
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
                if self.match_kind(&TokenKind::DoubleColon) {
                    let method = self.expect_identifier("expected method name after `::`")?;
                    self.expect(
                        TokenKind::LeftParen,
                        "expected `(` after static method name",
                    )?;
                    let args = self.parse_argument_list_after_open()?;
                    let span = Span::new(token.span.start, self.previous().span.end);
                    Some(Expr::StaticCall {
                        class_name: name,
                        method,
                        args,
                        span,
                    })
                } else {
                    Some(Expr::Identifier {
                        name,
                        span: token.span,
                    })
                }
            }
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
            TokenKind::New => self.parse_new(token.span.start),
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
        if matches!(expr, Expr::Identifier { .. }) {
            self.report_literal_open_brace(opening_brace_span);
            return None;
        }

        Some(expr)
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

    fn parse_new(&mut self, start: usize) -> Option<Expr> {
        let class_name = self.expect_identifier("expected class name after `new`")?;
        self.expect(TokenKind::LeftParen, "expected `(` after class name")?;
        let args = self.parse_argument_list_after_open()?;
        let span = Span::new(start, self.previous().span.end);
        Some(Expr::New {
            class_name,
            args,
            span,
        })
    }

    fn parse_array(&mut self, start: usize) -> Option<Expr> {
        let mut elements = Vec::new();
        if !self.check(&TokenKind::RightBracket) {
            loop {
                let first = self.parse_expression()?;
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

    fn parse_argument_list_after_open(&mut self) -> Option<Vec<Expr>> {
        let mut args = Vec::new();
        if !self.check(&TokenKind::RightParen) {
            loop {
                args.push(self.parse_expression()?);
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

    fn parse_type_ref_inner(&mut self) -> Option<TypeRef> {
        let nullable = self.match_kind(&TokenKind::Question);
        let name = match self.advance().kind.clone() {
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
            TokenKind::Identifier(name) => name,
            other => {
                self.error(
                    format!("expected type name, found `{}`", token_name(&other)),
                    self.previous().span,
                );
                return None;
            }
        };

        let mut args = Vec::new();
        if self.match_kind(&TokenKind::Less) {
            loop {
                args.push(self.parse_type_ref_inner()?);
                if !self.match_kind(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect_type_argument_close()?;
        }

        let mut ty = if args.is_empty() {
            TypeRef::named(name)
        } else {
            TypeRef::generic(name, args)
        };

        while self.pending_type_argument_close.is_none() && self.match_kind(&TokenKind::LeftBracket)
        {
            self.expect(
                TokenKind::RightBracket,
                "expected `]` after typed array suffix",
            )?;
            ty = TypeRef::array_of(ty);
        }

        Some(if nullable { ty.nullable() } else { ty })
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
        matches!(
            self.peek().kind,
            TokenKind::Writable
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
        if !self.is_at_end() {
            self.current += 1;
        }
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
                | TokenKind::Function
                | TokenKind::Let
                | TokenKind::Echo
                | TokenKind::Return
                | TokenKind::Break
                | TokenKind::Continue
                | TokenKind::If
                | TokenKind::While
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
        TokenKind::Interface => "interface",
        TokenKind::Implements => "implements",
        TokenKind::Function => "function",
        TokenKind::Internal => "internal",
        TokenKind::Static => "static",
        TokenKind::Let => "let",
        TokenKind::Writable => "writable",
        TokenKind::Readonly => "readonly",
        TokenKind::Return => "return",
        TokenKind::Echo => "echo",
        TokenKind::New => "new",
        TokenKind::Foreach => "foreach",
        TokenKind::As => "as",
        TokenKind::If => "if",
        TokenKind::Else => "else",
        TokenKind::While => "while",
        TokenKind::For => "for",
        TokenKind::Break => "break",
        TokenKind::Continue => "continue",
        TokenKind::Throw => "throw",
        TokenKind::Throws => "throws",
        TokenKind::True => "true",
        TokenKind::False => "false",
        TokenKind::Null => "null",
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
