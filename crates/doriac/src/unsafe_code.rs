//! Unsafe code and the foreign function interface (Decision 0148): the forms
//! this compiler accepts before it can check or run all of them.
//!
//! The parser accepts every form Decision 0148 specifies. Two gates name with
//! E0784 what is not implemented yet, so a valid program reads as not yet
//! supported, never as invalid:
//!
//! - [`unchecked_forms`] runs at name resolution, for forms checking does not
//!   cover yet.
//! - [`unexecutable_forms`] runs before lowering, for checked forms no
//!   execution target runs yet.

use std::collections::HashSet;

use crate::ast::transform::{Transform, Transformable};
use crate::ast::*;
use crate::diagnostics::Diagnostic;
use crate::source::Span;
use crate::types::TypeRef;

/// The diagnostic for an accepted Decision 0148 form this compiler does not
/// implement yet.
pub const NOT_AVAILABLE_CODE: &str = "E0784";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gate {
    Checking,
    Execution,
}

/// One E0784 diagnostic per form checking does not cover yet, in source
/// order. Empty for a program without them.
pub fn unchecked_forms(program: &Program) -> Vec<Diagnostic> {
    let mut finder = Finder::new(Gate::Checking);
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
        for entry in &import.entries {
            let target = match &import.prefix {
                Some(prefix) => format!("{}\\{}", prefix.canonical(), entry.target.canonical()),
                None => entry.target.canonical(),
            };
            finder.pending_name(&target, entry.span);
        }
    }
    for name in &program.qualified_names {
        finder.pending_name(&name.canonical(), name.span);
    }
    for item in &program.items {
        finder.item(item);
    }
    finder.finish()
}

/// The E0784 diagnostic for a checked form the native targets build and the
/// interpreter or PHP output does not: those run unsafe code, and take the
/// target's C data model, through the native companion library (Decision
/// 0148 sections 22 and 23).
pub fn companion_form(span: Span, title: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(NOT_AVAILABLE_CODE, message, span)
        .with_title(title)
        .with_explanation(
            "Unsafe code and the foreign function interface are part of Doria (Decision 0148). The program checks and builds for the native target. The interpreter and PHP output run unsafe code, and PHP output takes the target's C data model, through the native companion library, which this compiler does not build yet.",
        )
}

/// One E0784 diagnostic per MIR operation the native targets build and the
/// interpreter runs through the native companion library, in program order.
pub fn companion_operations(program: &crate::mir::Program) -> Vec<Diagnostic> {
    let mut seen = HashSet::new();
    let mut diagnostics = Vec::new();
    for function in &program.functions {
        for block in &function.blocks {
            for statement in &block.statements {
                if let crate::mir::Statement::Pointer { span, .. } = statement {
                    if seen.insert(*span) {
                        diagnostics.push(companion_form(
                            *span,
                            "Pointer Operations Cannot Run In The Interpreter Yet",
                            "pointer operations are part of Doria; the interpreter does not run them yet",
                        ));
                    }
                }
            }
        }
    }
    diagnostics
}

/// One E0784 diagnostic per checked form that no execution target runs yet.
pub fn unexecutable_forms(program: &Program) -> Vec<Diagnostic> {
    let mut finder = Finder::new(Gate::Execution);
    for item in &program.items {
        finder.item(item);
    }
    finder.finish()
}

struct Finder {
    gate: Gate,
    seen: HashSet<Span>,
    diagnostics: Vec<Diagnostic>,
}

impl Finder {
    fn new(gate: Gate) -> Self {
        Self {
            gate,
            seen: HashSet::new(),
            diagnostics: Vec::new(),
        }
    }

    fn finish(mut self) -> Vec<Diagnostic> {
        self.diagnostics
            .sort_by_key(|diagnostic| (diagnostic.span.source.0, diagnostic.span.start));
        self.diagnostics
    }

    fn report(&mut self, span: Span, title: &str, message: impl Into<String>) {
        if span.source.is_compiler_owned() || !self.seen.insert(span) {
            return;
        }
        let explanation = match self.gate {
            Gate::Checking => {
                "Unsafe code and the foreign function interface are part of Doria (Decision 0148). The program is not rejected; this compiler cannot check this form yet."
            }
            Gate::Execution => {
                "Unsafe code and the foreign function interface are part of Doria (Decision 0148). The program checks; this compiler cannot build or run this form yet."
            }
        };
        self.diagnostics.push(
            Diagnostic::new(NOT_AVAILABLE_CODE, message, span)
                .with_title(title)
                .with_explanation(explanation),
        );
    }

    /// `Doria\Std\Ffi` members whose surface lands later, and typed array
    /// regions.
    fn pending_name(&mut self, canonical: &str, span: Span) {
        let pending = crate::compiler_known_ffi::PENDING_TYPES.contains(&canonical)
            || matches!(canonical, "ArrayRegion" | "WritableArrayRegion");
        if pending {
            let short = canonical.rsplit('\\').next().unwrap_or(canonical);
            self.report(
                span,
                &format!("{short} Is Not Available Yet"),
                format!("`{short}` is part of Doria; this compiler does not implement it yet"),
            );
        }
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
                        self.type_refs(&mut field.ty.clone(), field.span);
                    }
                    if let Some(value) = &case.backing_value {
                        self.expression(value);
                    }
                }
            }
            Item::Function(function) => self.function(function),
            Item::Constant(constant) => {
                if let Some(ty) = &constant.ty {
                    self.type_refs(&mut ty.clone(), constant.span);
                }
                self.expression(&constant.initializer);
            }
            Item::ExternBlock(block) => {
                if self.gate == Gate::Execution {
                    self.report(
                        block.span,
                        "Extern Blocks Cannot Be Built Yet",
                        "`extern \"C\" { ... }` is part of Doria; this compiler does not build foreign calls yet",
                    );
                }
            }
            Item::ForeignAggregate(aggregate) => {
                if self.gate == Gate::Execution {
                    self.report(
                        aggregate.span,
                        match aggregate.kind {
                            ForeignAggregateKind::Struct => "Foreign Structs Cannot Be Built Yet",
                            ForeignAggregateKind::Union => "Foreign Unions Cannot Be Built Yet",
                        },
                        format!(
                            "`extern \"C\" {}` is part of Doria; this compiler does not build foreign layouts yet",
                            aggregate.kind.keyword()
                        ),
                    );
                }
            }
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
                        self.type_refs(&mut ty.clone(), constant.span);
                    }
                    self.expression(&constant.initializer);
                }
                ClassMember::Uses(_) => {}
            }
        }
    }

    fn property(&mut self, property: &PropertyDecl) {
        self.type_refs(&mut property.ty.clone(), property.span);
        if let Some(initializer) = &property.initializer {
            self.expression(initializer);
        }
        for hook in &property.hooks {
            if let Some(parameter) = &hook.parameter {
                self.type_refs(&mut parameter.ty.clone(), parameter.span);
            }
            if let Some(body) = hook.body.as_block() {
                self.block(body);
            }
        }
    }

    fn function(&mut self, function: &FunctionDecl) {
        if self.gate == Gate::Execution {
            match &function.linkage {
                FunctionLinkage::CAbi { .. } => self.report(
                    function.span,
                    "C-ABI Functions Cannot Be Built Yet",
                    format!(
                        "`extern \"C\" function {}` is part of Doria; this compiler does not build C-ABI functions yet",
                        function.name.rsplit('\\').next().unwrap_or(&function.name)
                    ),
                ),
                FunctionLinkage::Foreign { .. } => return,
                FunctionLinkage::Doria => {}
            }
        }
        let mut signature = function.clone();
        signature.body = FunctionBody::Requirement {
            semicolon_span: function.span,
        };
        self.type_refs(&mut signature, function.span);
        if let Some(body) = function.body.as_block() {
            self.block(body);
        }
    }

    fn block(&mut self, block: &Block) {
        for statement in &block.statements {
            self.statement(statement);
        }
    }

    fn statement(&mut self, statement: &Stmt) {
        let span = crate::return_analysis::statement_span(statement);
        self.type_refs(&mut statement.clone(), span);
        let mut nested = Vec::new();
        crate::ast::visit::stmt(statement, &mut |expression| nested.push(expression.clone()));
        for expression in &nested {
            self.expression_node(expression);
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
    fn expression_node(&mut self, expression: &Expr) {
        match expression {
            Expr::StaticCall {
                qualifier: StaticQualifier::Class(class),
                method,
                member_span,
                ..
            } if self.gate == Gate::Checking
                && class == "String"
                && matches!(method.as_str(), "fromCString" | "fromUtf16") =>
            {
                self.report(
                    *member_span,
                    "Foreign String Conversions Are Not Available Yet",
                    format!(
                        "`String::{method}` is part of Doria; this compiler does not implement it yet"
                    ),
                )
            }
            // Every pointer and boundary value comes from a gated
            // declaration or type, or from one of these calls.
            Expr::StaticCall {
                qualifier: StaticQualifier::Class(class),
                method,
                args,
                span,
                ..
            } if self.gate == Gate::Execution => {
                match crate::pointer_intrinsics::ForeignCompanion::from_class_name(class) {
                    // The native targets build every other pointer
                    // operation; the interpreter and PHP output report them.
                    // A pointer to a local builds; one to a field or
                    // property does not yet.
                    Some(crate::pointer_intrinsics::ForeignCompanion::Pointer(access))
                        if crate::pointer_intrinsics::formation(
                            &StaticQualifier::Class(class.clone()),
                            method,
                        )
                        .is_some()
                            && !args.first().is_some_and(|argument| {
                                matches!(ungrouped(&argument.value), Expr::Variable { .. })
                            }) =>
                    {
                        self.report(
                            *span,
                            "Pointers To Fields And Properties Cannot Be Built Yet",
                            format!(
                                "`{}::{method}` of a field or property is part of Doria; this compiler does not build it yet",
                                access.short_name()
                            ),
                        )
                    }
                    Some(crate::pointer_intrinsics::ForeignCompanion::Pointer(_)) => {}
                    // Every target builds boundary conversions; PHP output
                    // reports them itself.
                    Some(crate::pointer_intrinsics::ForeignCompanion::Boundary(_)) | None => {}
                }
            }
            Expr::Closure(closure) => {
                if let ClosureBody::Block(body) = &closure.body {
                    self.block(body);
                }
            }
            _ => {}
        }
    }

    fn type_refs(&mut self, node: &mut impl Transformable, span: Span) {
        if self.gate != Gate::Execution {
            return;
        }
        let mut types = TypeFinder::default();
        node.transform(&mut types);
        if types.foreign {
            self.report(
                span,
                "C Function Pointer Types Cannot Be Built Yet",
                "C function pointer types are part of Doria; this compiler does not build code that uses them yet",
            );
        }
    }
}

#[derive(Default)]
struct TypeFinder {
    foreign: bool,
}

impl Transform for TypeFinder {
    fn type_ref(&mut self, ty: &mut TypeRef) {
        if ty
            .function
            .as_ref()
            .is_some_and(|function| function.c_abi.is_some())
            || ty.fixed_array_parts().is_some()
        {
            self.foreign = true;
        }
        ty.walk(self);
    }

    fn function(&mut self, function: &mut FunctionDecl) {
        function.walk(self);
    }
}

/// `expr` without the parentheses and `unsafe(...)` around it.
fn ungrouped(expr: &Expr) -> &Expr {
    match expr {
        Expr::Grouped { expr, .. } | Expr::Unsafe { expr, .. } => ungrouped(expr),
        _ => expr,
    }
}
