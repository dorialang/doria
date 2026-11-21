//! Unsafe code and the foreign function interface (Decision 0148): the forms
//! this compiler accepts before it can check or run all of them.
//!
//! The parser accepts every form Decision 0148 specifies, and checking
//! covers each one. The gates here name with E0784 what no execution target
//! builds yet, so a valid program reads as not yet supported, never as
//! invalid: [`unexecutable_forms`] runs before lowering, and the other
//! functions run where the facts they read are known.

use std::collections::HashSet;

use crate::ast::transform::{Transform, Transformable};
use crate::ast::*;
use crate::diagnostics::Diagnostic;
use crate::source::Span;
use crate::types::{TypeArgumentRef, TypeRef};

/// The diagnostic for an accepted Decision 0148 form this compiler does not
/// implement yet.
pub const NOT_AVAILABLE_CODE: &str = "E0784";

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

/// The E0784 diagnostic for a foreign struct or union in `position`, a
/// place no execution target builds one yet, found after checking: a value
/// boxed as `mixed`, or a closure capture.
pub fn misplaced_aggregate(span: Span, position: &str) -> Diagnostic {
    let (title, message) = misplaced_aggregate_text(position);
    unbuilt_form(span, title, message)
}

fn misplaced_aggregate_text(position: &str) -> (&'static str, String) {
    (
        "Foreign Aggregates Cannot Be Built Here Yet",
        format!(
            "a foreign struct or union in {position} is part of Doria; this compiler builds foreign structs and unions and their nullable forms as locals, parameters, results, pointer targets, properties, constants and enum payloads, and not here yet"
        ),
    )
}

/// The E0784 diagnostic for a typed array region (Decision 0148 section 8)
/// over elements of `element`, a type no execution target builds a region
/// over yet.
pub fn unbuilt_region_element(span: Span, region: &str, element: &str) -> Diagnostic {
    unbuilt_form(
        span,
        "Array Regions Of This Element Type Cannot Be Built Yet",
        format!(
            "`{region}<{element}>` is part of Doria; this compiler builds array regions over numbers, `bool`, enum cases, pointers and other plain Copy values, and not over `{element}` yet"
        ),
    )
}

/// The E0784 diagnostic for a checked form no execution target builds yet,
/// for a gate that runs after checking, where the form's types are known.
pub fn unbuilt_form(span: Span, title: &str, message: impl Into<String>) -> Diagnostic {
    let mut finder = Finder::new();
    finder.report(span, title, message);
    finder
        .diagnostics
        .pop()
        .expect("a reported form has its diagnostic")
}

/// One E0784 diagnostic per MIR operation the native targets build and the
/// interpreter runs through the native companion library, in program order.
pub fn companion_operations(program: &crate::mir::Program) -> Vec<Diagnostic> {
    let mut seen = HashSet::new();
    let mut diagnostics = Vec::new();
    for function in &program.functions {
        for block in &function.blocks {
            for statement in &block.statements {
                let (span, title, message) = match statement {
                    crate::mir::Statement::Pointer { span, .. } => (
                        span,
                        "Pointer Operations Cannot Run In The Interpreter Yet",
                        "pointer operations are part of Doria; the interpreter does not run them yet",
                    ),
                    crate::mir::Statement::ForeignCall { span, .. } => (
                        span,
                        "Foreign Calls Cannot Run In The Interpreter Yet",
                        "foreign calls are part of Doria; the interpreter does not run them yet",
                    ),
                    crate::mir::Statement::ForeignField { span, .. }
                    | crate::mir::Statement::AssignLocal {
                        value:
                            crate::mir::Rvalue::ForeignAggregate(
                                crate::mir::ForeignAggregateExpression::Construct { span, .. },
                            ),
                        ..
                    }
                    | crate::mir::Statement::AssignProperty {
                        value:
                            crate::mir::Rvalue::ForeignAggregate(
                                crate::mir::ForeignAggregateExpression::Construct { span, .. },
                            ),
                        ..
                    } => (span, AGGREGATE_TITLE, AGGREGATE_MESSAGE),
                    _ => continue,
                };
                if seen.insert(*span) {
                    diagnostics.push(companion_form(*span, title, message));
                }
            }
        }
        // Every foreign aggregate value comes from a construction, a
        // pointer read or a foreign call; a function that only passes one
        // on is reported where it is declared.
        let holds_aggregate = function
            .locals
            .iter()
            .any(|local| holds_foreign_aggregate(program, local.ty, &mut HashSet::new()));
        if holds_aggregate && diagnostics.is_empty() && seen.insert(function.source_span) {
            diagnostics.push(companion_form(
                function.source_span,
                AGGREGATE_TITLE,
                AGGREGATE_MESSAGE,
            ));
        }
    }
    // A static property's foreign struct or union lives in static data the
    // program reads and writes in place.
    for property in &program.statics {
        if holds_foreign_aggregate(program, property.ty, &mut HashSet::new())
            && seen.insert(property.source_span)
        {
            diagnostics.push(companion_form(
                property.source_span,
                AGGREGATE_TITLE,
                AGGREGATE_MESSAGE,
            ));
        }
    }
    diagnostics
}

/// Whether a value of `ty` holds a foreign struct or union: the aggregate
/// itself or its nullable form, or a payload enum with one in a case's
/// payload.
fn holds_foreign_aggregate(
    program: &crate::mir::Program,
    ty: crate::mir::Type,
    visited: &mut HashSet<crate::enums::EnumId>,
) -> bool {
    match ty {
        crate::mir::Type::ForeignAggregate(_) | crate::mir::Type::NullableForeignAggregate(_) => {
            true
        }
        crate::mir::Type::PayloadEnum(payload) | crate::mir::Type::NullablePayloadEnum(payload) => {
            visited.insert(payload.id)
                && program.enums.get(payload.id.0).is_some_and(|definition| {
                    definition.cases.iter().any(|case| {
                        case.payload
                            .iter()
                            .any(|field| holds_foreign_aggregate(program, field.ty, visited))
                    })
                })
        }
        _ => false,
    }
}

const AGGREGATE_TITLE: &str = "Foreign Structs And Unions Cannot Run In The Interpreter Yet";
const AGGREGATE_MESSAGE: &str =
    "foreign structs and unions are part of Doria; the interpreter does not run them yet";

/// Section 16's `WindowsLastError` capture exists only on Windows, so a
/// call that emitted code makes to a binding capturing it on another target
/// is a compile error. A call in a branch whose condition is constant false
/// for the target is not emitted (section 17), so a binding used only under
/// `Platform::operatingSystem == OperatingSystem::Windows` builds everywhere.
pub const WINDOWS_LAST_ERROR_CODE: &str = "E0817";

/// One E0817 diagnostic per emitted call to a binding capturing
/// `WindowsLastError` when `target` is not Windows, in program order.
pub fn windows_last_error_off_windows(
    program: &crate::mir::Program,
    target: crate::c_layout::CTarget,
) -> Vec<Diagnostic> {
    if target.operating_system == crate::c_layout::TargetOperatingSystem::Windows {
        return Vec::new();
    }
    let mut seen = HashSet::new();
    let mut diagnostics = Vec::new();
    for function in &program.functions {
        for block in &function.blocks {
            for statement in &block.statements {
                let crate::mir::Statement::ForeignCall {
                    function: foreign,
                    span,
                    ..
                } = statement
                else {
                    continue;
                };
                if foreign.system_code != Some(crate::foreign::SystemCodeSource::WindowsLastError)
                    || !seen.insert(*span)
                {
                    continue;
                }
                let symbol = match &foreign.callee {
                    crate::mir::ForeignCallee::Symbol { symbol, .. } => symbol.as_str(),
                    crate::mir::ForeignCallee::Pointer(_) => "the function",
                };
                diagnostics.push(
                    Diagnostic::new(
                        WINDOWS_LAST_ERROR_CODE,
                        format!(
                            "the binding of `{symbol}` captures `SystemCodeSource::WindowsLastError`, which only Windows has, and this program builds for {}",
                            target.operating_system.display_name()
                        ),
                        *span,
                    )
                    .with_title("Windows Error Code Captured Off Windows")
                    .with_primary_label("Called Here")
                    .with_explanation(
                        "`#[CaptureSystemCode(SystemCodeSource::WindowsLastError)]` reads `GetLastError` after the call, which exists only on Windows. A call in a branch whose condition is false for the target is not built, so Windows bindings belong under `Platform::operatingSystem == OperatingSystem::Windows`.",
                    )
                    .with_help("call it inside `if (Platform::operatingSystem == OperatingSystem::Windows) { ... }`, or capture `SystemCodeSource::Errno` in the binding this platform uses"),
                );
            }
        }
    }
    diagnostics
}

/// One E0784 diagnostic per checked form that no execution target runs yet.
pub fn unexecutable_forms(program: &Program) -> Vec<Diagnostic> {
    let mut finder = Finder::new();
    finder.aggregates = program
        .items
        .iter()
        .filter_map(|item| match item {
            Item::ForeignAggregate(aggregate) => Some(aggregate.name.clone()),
            _ => None,
        })
        .collect();
    for item in &program.items {
        finder.item(item);
    }
    finder.finish()
}

struct Finder {
    seen: HashSet<Span>,
    diagnostics: Vec<Diagnostic>,
    /// The canonical names of the program's foreign structs and unions.
    aggregates: HashSet<String>,
}

impl Finder {
    fn new() -> Self {
        Self {
            seen: HashSet::new(),
            diagnostics: Vec::new(),
            aggregates: HashSet::new(),
        }
    }

    /// Reports a foreign struct or union `ty` holds where no execution
    /// target builds one yet: anywhere in a closure's signature or
    /// captures (`anywhere`), otherwise as a type argument other than a
    /// pointer's pointee, or in a function type. Locals, parameters,
    /// results, properties, constants, enum payloads and pointees build,
    /// each also as `?Name`.
    fn misplaced_aggregate(&mut self, ty: &TypeRef, span: Span, anywhere: Option<&str>) {
        if self.aggregates.is_empty() {
            return;
        }
        let misplaced = match anywhere {
            Some(_) => mentions_aggregate(ty, &self.aggregates),
            None => aggregate_outside_value_position(ty, &self.aggregates, true),
        };
        if misplaced {
            let position = anywhere
                .unwrap_or("a type argument other than a pointer's pointee, or a function type");
            let (title, message) = misplaced_aggregate_text(position);
            self.report(span, title, message);
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
        let explanation = "Unsafe code and the foreign function interface are part of Doria (Decision 0148). The program checks; this compiler cannot build or run this form yet.";
        self.diagnostics.push(
            Diagnostic::new(NOT_AVAILABLE_CODE, message, span)
                .with_title(title)
                .with_explanation(explanation),
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
                    // A payload field holds a foreign struct or union by
                    // value in the enum's storage (Decision 0148 section 6).
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
            // Each declaration is an `Item::Function`; the bindings that
            // cannot be built yet are reported from the checked facts
            // ([`unbuildable_bindings`]).
            Item::ExternBlock(_) => {}
            // A foreign layout builds; fixed-length array fields report
            // where a program reads or writes them, at checking.
            Item::ForeignAggregate(_) => {}
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
        // An instance property holds a foreign struct or union by value in
        // the object's storage, and a static property in its static data
        // (Decision 0148 section 6).
        self.misplaced_aggregate(&property.ty, property.span, None);
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
        if let Expr::Closure(closure) = expression {
            for parameter in &closure.parameters {
                self.misplaced_aggregate(
                    &parameter.ty,
                    parameter.span,
                    Some("a closure parameter"),
                );
            }
            if let Some(result) = &closure.return_type {
                self.misplaced_aggregate(&result.ty, closure.span, Some("a closure result"));
            }
            if let ClosureBody::Block(body) = &closure.body {
                self.block(body);
            }
        }
    }

    fn type_refs(&mut self, node: &mut impl Transformable, span: Span) {
        let mut types = TypeFinder::default();
        node.transform(&mut types);
        for ty in std::mem::take(&mut types.named) {
            self.misplaced_aggregate(&ty, span, None);
        }
    }
}

#[derive(Default)]
struct TypeFinder {
    /// Every outermost type the node names, for the aggregate placement
    /// check, which reads nested types itself.
    named: Vec<TypeRef>,
}

impl Transform for TypeFinder {
    fn type_ref(&mut self, ty: &mut TypeRef) {
        self.named.push(ty.clone());
        ty.walk(self);
    }

    fn function(&mut self, function: &mut FunctionDecl) {
        function.walk(self);
    }
}

/// Whether `ty` names a foreign struct or union anywhere.
fn mentions_aggregate(ty: &TypeRef, aggregates: &HashSet<String>) -> bool {
    aggregates.contains(&ty.name)
        || ty.arguments.iter().any(|argument| match argument {
            TypeArgumentRef::Type(inner) => mentions_aggregate(inner, aggregates),
            TypeArgumentRef::Value(_) => false,
        })
        || doria_function_mentions_aggregate(ty, aggregates)
}

/// Whether `ty` holds a foreign struct or union other than by value, or
/// as `?Name`, in a value position or as a pointer's pointee.
fn aggregate_outside_value_position(
    ty: &TypeRef,
    aggregates: &HashSet<String>,
    value_position: bool,
) -> bool {
    if aggregates.contains(&ty.name) && !value_position {
        return true;
    }
    let pointer = crate::foreign::PointerAccess::from_source_name(&ty.name).is_some();
    ty.arguments.iter().any(|argument| match argument {
        TypeArgumentRef::Type(inner) => {
            aggregate_outside_value_position(inner, aggregates, pointer)
        }
        TypeArgumentRef::Value(_) => false,
    }) || doria_function_mentions_aggregate(ty, aggregates)
}

/// Whether `ty` is a Doria function type whose signature names a foreign
/// struct or union. A C function pointer type passes them by value, as an
/// extern declaration does, and holds only an address (section 12).
fn doria_function_mentions_aggregate(ty: &TypeRef, aggregates: &HashSet<String>) -> bool {
    ty.function.as_ref().is_some_and(|function| {
        function.c_abi.is_none()
            && (function
                .parameters
                .iter()
                .any(|parameter| mentions_aggregate(&parameter.ty, aggregates))
                || mentions_aggregate(&function.return_type, aggregates))
    })
}
