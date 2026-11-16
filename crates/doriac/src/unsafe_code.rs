//! Unsafe code and the foreign function interface (Decision 0148).
//!
//! The parser accepts every form Decision 0148 specifies. Until checking and
//! execution land, [`unavailable_forms`] names each form in a program with
//! E0784, so a valid program reads as not yet supported, never as invalid.

use std::collections::HashSet;

use crate::ast::transform::{Transform, Transformable};
use crate::ast::*;
use crate::diagnostics::Diagnostic;
use crate::source::Span;
use crate::types::TypeRef;

pub const FFI_NAMESPACE: &str = "Doria\\Std\\Ffi";

/// The diagnostic for an accepted Decision 0148 form this compiler does not
/// implement yet.
pub const NOT_AVAILABLE_CODE: &str = "E0784";

/// One E0784 diagnostic per unsafe or foreign form in `program`, in source
/// order. Empty for a program without them.
pub fn unavailable_forms(program: &Program) -> Vec<Diagnostic> {
    let mut finder = Finder::default();
    for directive in &program.directives {
        finder.report(
            directive.span,
            "Declare Directives Are Not Available Yet",
            format!(
                "`declare({}: ...)` is part of Doria; this compiler does not implement `declare` directives yet",
                directive.key.text
            ),
        );
    }
    for import in &program.imports {
        let names_ffi = import
            .prefix
            .iter()
            .map(|prefix| prefix.canonical())
            .chain(import.entries.iter().map(|entry| entry.target.canonical()))
            .any(|name| name == FFI_NAMESPACE || name.starts_with("Doria\\Std\\Ffi\\"));
        if names_ffi {
            finder.report(
                import.span,
                "The Doria\\Std\\Ffi Module Is Not Available Yet",
                "`Doria\\Std\\Ffi` is part of Doria; this compiler does not implement it yet",
            );
        }
    }
    for item in &program.items {
        finder.item(item);
    }
    for name in &program.qualified_names {
        let canonical = name.canonical();
        if matches!(canonical.as_str(), "ArrayRegion" | "WritableArrayRegion") {
            finder.report(
                name.span,
                "Typed Array Regions Are Not Available Yet",
                format!("`{canonical}` is part of Doria; this compiler does not implement it yet"),
            );
        }
    }
    let mut diagnostics = finder.diagnostics;
    diagnostics.sort_by_key(|diagnostic| (diagnostic.span.source.0, diagnostic.span.start));
    diagnostics
}

#[derive(Default)]
struct Finder {
    seen: HashSet<Span>,
    diagnostics: Vec<Diagnostic>,
}

impl Finder {
    fn report(&mut self, span: Span, title: &str, message: impl Into<String>) {
        if !self.seen.insert(span) {
            return;
        }
        self.diagnostics.push(
            Diagnostic::new(NOT_AVAILABLE_CODE, message, span)
                .with_title(title)
                .with_explanation(
                    "Unsafe code and the foreign function interface are part of Doria (Decision 0148). The program is not rejected; this compiler cannot check or run these forms yet.",
                ),
        );
    }

    fn item(&mut self, item: &Item) {
        match item {
            Item::Class(class) => self.members(&class.members),
            Item::Trait(declaration) => self.members(&declaration.members),
            Item::Interface(declaration) => {
                for requirement in &declaration.requirements {
                    self.function(requirement);
                }
                for property in &declaration.properties {
                    self.property(property);
                }
            }
            Item::Enum(declaration) => {
                for case in &declaration.cases {
                    for field in &case.payload {
                        self.type_refs(field.ty.clone(), field.span);
                    }
                    if let Some(value) = &case.backing_value {
                        self.expression(value);
                    }
                }
            }
            Item::Function(function) => self.function(function),
            Item::Constant(constant) => {
                if let Some(ty) = &constant.ty {
                    self.type_refs(ty.clone(), constant.span);
                }
                self.expression(&constant.initializer);
            }
            Item::ExternBlock(block) => self.report(
                block.span,
                "Extern Blocks Are Not Available Yet",
                "`extern \"C\" { ... }` is part of Doria; this compiler does not implement foreign declarations yet",
            ),
            Item::ForeignAggregate(aggregate) => self.report(
                aggregate.span,
                match aggregate.kind {
                    ForeignAggregateKind::Struct => "Foreign Structs Are Not Available Yet",
                    ForeignAggregateKind::Union => "Foreign Unions Are Not Available Yet",
                },
                format!(
                    "`extern \"C\" {}` is part of Doria; this compiler does not implement foreign layouts yet",
                    aggregate.kind.keyword()
                ),
            ),
            Item::Statement(statement) => self.statement(statement),
        }
    }

    fn members(&mut self, members: &[ClassMember]) {
        for member in members {
            match member {
                ClassMember::Method(method) => self.function(method),
                ClassMember::Property(property) => self.property(property),
                ClassMember::Constant(constant) => {
                    if let Some(ty) = &constant.ty {
                        self.type_refs(ty.clone(), constant.span);
                    }
                    self.expression(&constant.initializer);
                }
                ClassMember::Uses(_) => {}
            }
        }
    }

    fn property(&mut self, property: &PropertyDecl) {
        self.type_refs(property.ty.clone(), property.span);
        if let Some(initializer) = &property.initializer {
            self.expression(initializer);
        }
        for hook in &property.hooks {
            if let Some(parameter) = &hook.parameter {
                self.type_refs(parameter.ty.clone(), parameter.span);
            }
            if let Some(body) = hook.body.as_block() {
                self.block(body);
            }
        }
    }

    fn function(&mut self, function: &FunctionDecl) {
        match &function.linkage {
            FunctionLinkage::CAbi { .. } => self.report(
                function.span,
                "C-ABI Functions Are Not Available Yet",
                format!(
                    "`extern \"C\" function {}` is part of Doria; this compiler does not implement C-ABI functions yet",
                    function.name
                ),
            ),
            FunctionLinkage::Foreign { .. } | FunctionLinkage::Doria => {}
        }
        if let Some(span) = function.unsafe_span {
            self.report(
                span.merge(function.name_span),
                "Unsafe Functions Are Not Available Yet",
                format!(
                    "`unsafe function {}` is part of Doria; this compiler does not implement unsafe functions yet",
                    function.name
                ),
            );
        }
        let mut signature = function.clone();
        signature.body = FunctionBody::Requirement {
            semicolon_span: function.span,
        };
        self.type_refs_in(&mut signature);
        if let Some(body) = function.body.as_block() {
            self.block(body);
        }
        for parameter in &function.params {
            if let Some(default) = &parameter.default {
                self.expression(default);
            }
        }
    }

    fn block(&mut self, block: &Block) {
        for statement in &block.statements {
            self.statement(statement);
        }
    }

    fn statement(&mut self, statement: &Stmt) {
        if let Stmt::Unsafe(unsafe_block) = statement {
            self.report(
                unsafe_block.keyword_span,
                "Unsafe Blocks Are Not Available Yet",
                "`unsafe { ... }` is part of Doria; this compiler does not implement unsafe contexts yet",
            );
        }
        let mut types = statement.clone();
        self.type_refs_in(&mut types);
        let mut nested = Vec::new();
        crate::ast::visit::stmt(statement, &mut |expression| {
            nested.push(expression.clone());
        });
        for expression in &nested {
            self.expression_node(expression);
        }
        match statement {
            Stmt::Block(block) | Stmt::Unsafe(UnsafeBlock { block, .. }) => self.block(block),
            Stmt::Try(statement) => {
                self.block(&statement.body);
                for catch in &statement.catches {
                    self.block(&catch.body);
                }
                if let Some(finalizer) = &statement.finally {
                    self.block(&finalizer.body);
                }
            }
            Stmt::If(statement) => self.conditional(statement),
            Stmt::While(statement) => {
                self.given(&statement.given);
                self.block(&statement.body);
                self.finally(&statement.finally);
            }
            Stmt::DoWhile(statement) => {
                self.block(&statement.body);
                self.finally(&statement.finally);
            }
            Stmt::For(statement) => self.block(&statement.body),
            Stmt::Foreach(statement) => self.block(&statement.body),
            Stmt::VarDecl(_)
            | Stmt::Assignment(_)
            | Stmt::Echo { .. }
            | Stmt::Return { .. }
            | Stmt::Throw(_)
            | Stmt::Break { .. }
            | Stmt::Continue { .. }
            | Stmt::Increment(_)
            | Stmt::Expr { .. } => {}
        }
    }

    fn conditional(&mut self, statement: &IfStmt) {
        self.given(&statement.given);
        self.block(&statement.then_block);
        match &statement.else_branch {
            Some(ElseBranch::If(nested)) => self.conditional(nested),
            Some(ElseBranch::Block(block)) => self.block(block),
            None => {}
        }
        self.finally(&statement.finally);
    }

    fn given(&mut self, prelude: &Option<GivenPrelude>) {
        if let Some(prelude) = prelude {
            self.block(&prelude.block);
        }
    }

    fn finally(&mut self, finalizer: &Option<ControlFlowFinally>) {
        if let Some(finalizer) = finalizer {
            self.block(&finalizer.block);
        }
    }

    fn expression(&mut self, expression: &Expr) {
        let mut nested = Vec::new();
        crate::ast::visit::expr(expression, &mut |inner| nested.push(inner.clone()));
        for inner in &nested {
            self.expression_node(inner);
        }
    }

    /// One expression, without its children: the visitors above reach those.
    /// Statements inside closure and `when` bodies are walked here.
    fn expression_node(&mut self, expression: &Expr) {
        match expression {
            Expr::Unsafe { keyword_span, .. } => self.report(
                *keyword_span,
                "Unsafe Expressions Are Not Available Yet",
                "`unsafe(<expression>)` is part of Doria; this compiler does not implement unsafe contexts yet",
            ),
            Expr::StaticCall {
                qualifier: StaticQualifier::Class(class),
                method,
                member_span,
                ..
            } if class == "String" && matches!(method.as_str(), "fromCString" | "fromUtf16") => {
                self.report(
                    *member_span,
                    "Foreign String Conversions Are Not Available Yet",
                    format!(
                        "`String::{method}` is part of Doria; this compiler does not implement it yet"
                    ),
                )
            }
            Expr::Closure(closure) => {
                if let ClosureBody::Block(body) = &closure.body {
                    for statement in &body.statements {
                        if let Stmt::Unsafe(unsafe_block) = statement {
                            self.statement(&Stmt::Unsafe(unsafe_block.clone()));
                        } else {
                            self.statement_blocks(statement);
                        }
                    }
                }
            }
            Expr::When(when) => {
                for branch in &when.branches {
                    for statement in &branch.block.statements {
                        self.statement_blocks(statement);
                    }
                }
            }
            _ => {}
        }
        if let Expr::New { class_type, .. } | Expr::IsType { ty: class_type, .. } = expression {
            self.type_refs(class_type.clone(), expression.span());
        }
    }

    /// Unsafe blocks among a statement's nested blocks.
    fn statement_blocks(&mut self, statement: &Stmt) {
        if let Stmt::Unsafe(unsafe_block) = statement {
            self.report(
                unsafe_block.keyword_span,
                "Unsafe Blocks Are Not Available Yet",
                "`unsafe { ... }` is part of Doria; this compiler does not implement unsafe contexts yet",
            );
        }
        match statement {
            Stmt::Block(block) | Stmt::Unsafe(UnsafeBlock { block, .. }) => {
                for nested in &block.statements {
                    self.statement_blocks(nested);
                }
            }
            Stmt::If(statement) => self.conditional_blocks(statement),
            Stmt::While(statement) => self.blocks_in(&statement.body),
            Stmt::DoWhile(statement) => self.blocks_in(&statement.body),
            Stmt::For(statement) => self.blocks_in(&statement.body),
            Stmt::Foreach(statement) => self.blocks_in(&statement.body),
            Stmt::Try(statement) => {
                self.blocks_in(&statement.body);
                for catch in &statement.catches {
                    self.blocks_in(&catch.body);
                }
            }
            _ => {}
        }
    }

    fn conditional_blocks(&mut self, statement: &IfStmt) {
        self.blocks_in(&statement.then_block);
        match &statement.else_branch {
            Some(ElseBranch::If(nested)) => self.conditional_blocks(nested),
            Some(ElseBranch::Block(block)) => self.blocks_in(block),
            None => {}
        }
    }

    fn blocks_in(&mut self, block: &Block) {
        for statement in &block.statements {
            self.statement_blocks(statement);
        }
    }

    fn type_refs(&mut self, mut ty: TypeRef, span: Span) {
        let mut types = TypeFinder::default();
        ty.transform(&mut types);
        self.report_types(types, span);
    }

    fn type_refs_in(&mut self, node: &mut impl Transformable) {
        let mut types = TypeFinder::default();
        node.transform(&mut types);
        self.report_types(types, Span::default());
    }

    fn report_types(&mut self, types: TypeFinder, _fallback: Span) {
        for span in types.c_function_types {
            self.report(
                span,
                "C Function Pointer Types Are Not Available Yet",
                "`extern \"C\" function(...)` types are part of Doria; this compiler does not implement them yet",
            );
        }
    }
}

#[derive(Default)]
struct TypeFinder {
    c_function_types: Vec<Span>,
}

impl Transform for TypeFinder {
    fn type_ref(&mut self, ty: &mut TypeRef) {
        if let Some(function) = &ty.function {
            if function.c_abi.is_some() {
                self.c_function_types.push(function.span);
            }
        }
        ty.walk(self);
    }

    fn function(&mut self, function: &mut FunctionDecl) {
        function.walk(self);
    }
}
